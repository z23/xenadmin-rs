use super::*;
use std::io::Write;

fn metadata(vm: u128, memory: u128, preserve: bool) -> Value {
    let mut objects = vec![
        json!({"class":"VM","id":"Ref:1","snapshot":{"uuid":uuid(vm),"name_label":"Archive guest","is_control_domain":false,"is_a_template":false,"is_a_snapshot":false,"power_state":if preserve{"Suspended"}else{"Halted"},"suspend_VDI":if preserve{"Ref:4"}else{"OpaqueRef:NULL"},"other_config":{}}}),
        json!({"class":"VDI","id":"Ref:2","snapshot":{"uuid":uuid(11),"SR":"Ref:5","type":"user","virtual_size":"4096"}}),
        json!({"class":"network","id":"Ref:3","snapshot":{}}),
        json!({"class":"SR","id":"Ref:5","snapshot":{"uuid":uuid(4),"content_type":"user"}}),
    ];
    if preserve {
        objects.push(json!({"class":"VDI","id":"Ref:4","snapshot":{"uuid":uuid(memory),"SR":"Ref:5","type":"suspend","virtual_size":"2048"}}));
    }
    json!({"version":{},"objects":objects})
}
fn packed(value: &Value) -> Vec<u8> {
    let mut output = tar::Builder::new(Vec::new());
    let mut append = |name: &str, bytes: &[u8]| {
        let mut h = tar::Header::new_ustar();
        h.set_size(bytes.len() as u64);
        h.set_mode(0o600);
        h.set_cksum();
        output.append_data(&mut h, name, bytes).unwrap();
    };
    append("ova.xml", xml(value).as_bytes());
    for object in value["objects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|v| v["class"] == "VDI")
    {
        let size = object["snapshot"]["virtual_size"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        let bytes = vec![0; size];
        let name = format!("{}/00000000", object["id"].as_str().unwrap());
        append(&name, &bytes);
        append(
            &format!("{name}.xxhash"),
            format!("{:016X}", xxhash_rust::xxh64::xxh64(&bytes, 0)).as_bytes(),
        );
    }
    output.into_inner().unwrap()
}
pub(super) fn archive(vm: u128, memory: u128, preserve: bool) -> Vec<u8> {
    packed(&metadata(vm, memory, preserve))
}
pub(super) fn imported(records: &mut BTreeMap<String, Value>) {
    let fault = records
        .get("fixture:checkpoint_fault")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let vm = records.get_mut("OpaqueRef:VM:new").unwrap();
    vm["power_state"] = json!("Suspended");
    vm["suspend_VDI"] = json!("OpaqueRef:VDI:memory-new");
    let mut memory = json!({"uuid":uuid(24),"SR":"OpaqueRef:SR:s","type":"suspend","managed":true,"missing":false,"virtual_size":"2048","current_operations":{},"sm_config":{"import_task":"OpaqueRef:task:t"},"VBDs":[]});
    if fault == "wrong_size" {
        memory["virtual_size"] = json!(4096);
    }
    if fault == "wrong_type" {
        memory["type"] = json!("user");
    }
    if fault != "missing" {
        records.insert("OpaqueRef:VDI:memory-new".into(), memory);
    }
    if fault == "wrong_owner" {
        records.insert(
            "OpaqueRef:VM:other".into(),
            json!({"uuid":uuid(55),"suspend_VDI":"OpaqueRef:VDI:memory-new"}),
        );
    }
}
fn setup(f: &Fixture) {
    let mut r = f.records.lock().unwrap();
    r.insert("fixture:checkpoint".into(), json!(true));
    let v = r.get_mut("OpaqueRef:VM:source").unwrap();
    v["is_a_snapshot"] = json!(true);
    v["snapshot_of"] = json!("OpaqueRef:VM:parent");
    v["power_state"] = json!("Suspended");
    v["suspend_VDI"] = json!("OpaqueRef:VDI:memory");
    r.insert(
        "OpaqueRef:VM:parent".into(),
        json!({"uuid":uuid(9),"suspend_VDI":"OpaqueRef:NULL"}),
    );
    r.insert("OpaqueRef:VDI:memory".into(),json!({"uuid":uuid(10),"SR":"OpaqueRef:SR:s","virtual_size":"2048","type":"suspend","managed":true,"missing":false,"current_operations":{},"VBDs":[]}));
}
fn action(preserve: bool) -> TransferAction {
    TransferAction::ExportXva {
        source: ExportSource::Checkpoint {
            snapshot: uuid(2).into(),
            parent: uuid(9).into(),
            memory: CheckpointMemory {
                vdi: uuid(10).into(),
                sr: uuid(4).into(),
                virtual_size: 2048,
            },
            preserve_memory: preserve,
        },
    }
}
async fn run(
    f: &Fixture,
    dir: &Path,
    action: TransferAction,
    file: TransferFile,
) -> crate::ResourceOperationRecord {
    let (handle, request) = resources::transfer_channel(
        OperationId::new(),
        ObservedResource {
            pool: uuid(1).into(),
            generation: 1,
            target: ResourceTarget::Pool(uuid(1).into()),
        },
        TransferRequest { action, file },
        tokio::runtime::Handle::current(),
    );
    let (tx, _rx) = mpsc::channel(512);
    tokio::time::timeout(
        std::time::Duration::from_secs(20),
        request.run(
            Arc::new(f.session().await),
            dir.into(),
            CancellationToken::new(),
            tx,
        ),
    )
    .await
    .unwrap();
    (*handle.latest()).clone()
}
#[test]
fn checkpoint_archive_requires_unique_memory_graph_and_complete_plain_or_gzip_stream() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("checkpoint.xva");
    for gzip in [false, true] {
        let bytes = archive(2, 10, true);
        if gzip {
            let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            gz.write_all(&bytes).unwrap();
            std::fs::write(&path, gz.finish().unwrap()).unwrap();
        } else {
            std::fs::write(&path, bytes).unwrap();
        }
        let file = TransferFile::source(&path).unwrap();
        let a = file.archive().unwrap();
        assert_eq!(a.memory_images(), 1);
        assert_eq!(a.data_disks, 1);
        assert_eq!(a.virtual_bytes, 6144);
        assert_eq!(a.guests[0].memory.as_ref().unwrap().uuid, uuid(10));
    }
    for fault in [
        "missing",
        "wrong_type",
        "shared",
        "orphan",
        "vbd",
        "iso",
        "truncated",
        "checksum",
    ] {
        let mut v = metadata(2, 10, true);
        match fault {
            "missing" => v["objects"][0]["snapshot"]["suspend_VDI"] = json!("Ref:99"),
            "wrong_type" => v["objects"][4]["snapshot"]["type"] = json!("user"),
            "shared" => {
                let mut vm = v["objects"][0].clone();
                vm["id"] = json!("Ref:6");
                v["objects"].as_array_mut().unwrap().push(vm);
            }
            "orphan" => {
                v["objects"][0]["snapshot"]["power_state"] = json!("Halted");
                v["objects"][0]["snapshot"]["suspend_VDI"] = json!("OpaqueRef:NULL");
            }
            "vbd" => v["objects"]
                .as_array_mut()
                .unwrap()
                .push(json!({"class":"VBD","id":"Ref:6","snapshot":{"VDI":"Ref:4"}})),
            "iso" => v["objects"][3]["snapshot"]["content_type"] = json!("iso"),
            _ => {}
        }
        let mut bytes = packed(&v);
        if fault == "truncated" {
            bytes.truncate(bytes.len() - 4096);
        }
        if fault == "checksum" {
            let mut ar = tar::Archive::new(bytes.as_slice());
            let offset = ar
                .entries()
                .unwrap()
                .nth(3)
                .unwrap()
                .unwrap()
                .raw_file_position() as usize;
            bytes[offset] ^= 1;
        }
        std::fs::write(&path, bytes).unwrap();
        assert!(TransferFile::source(&path).is_err(), "{fault}");
    }
    std::fs::write(&path, archive(2, 10, true)).unwrap();
    assert!(
        TransferFile::source_with_cancel(&path, &std::sync::atomic::AtomicBool::new(true)).is_err()
    );
}
#[tokio::test]
async fn checkpoint_exports_reviewed_memory_or_disks_only_and_refuses_changed_memory() {
    for preserve in [false, true] {
        let f = Fixture::new(Fault::None).await;
        setup(&f);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("output.xva");
        let r = run(
            &f,
            dir.path(),
            action(preserve),
            TransferFile::destination(&path).unwrap(),
        )
        .await;
        assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
        assert_eq!(std::fs::read(path).unwrap(), archive(2, 10, preserve));
        assert_eq!(f.count("GET /export"), 1);
    }
    for fault in ["size", "type", "busy", "owner", "wrong_export_memory"] {
        let f = Fixture::new(Fault::None).await;
        setup(&f);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("output.xva");
        {
            let mut r = f.records.lock().unwrap();
            match fault {
                "size" => r.get_mut("OpaqueRef:VDI:memory").unwrap()["virtual_size"] = json!(4096),
                "type" => r.get_mut("OpaqueRef:VDI:memory").unwrap()["type"] = json!("user"),
                "busy" => {
                    r.get_mut("OpaqueRef:VDI:memory").unwrap()["current_operations"] =
                        json!({"OpaqueRef:t":"resize"})
                }
                "owner" => {
                    r.insert(
                        "OpaqueRef:VM:other".into(),
                        json!({"uuid":uuid(33),"suspend_VDI":"OpaqueRef:VDI:memory"}),
                    );
                }
                _ => {
                    r.insert("fixture:checkpoint_fault".into(), json!(fault));
                }
            }
        }
        let r = run(
            &f,
            dir.path(),
            action(true),
            TransferFile::destination(&path).unwrap(),
        )
        .await;
        assert_ne!(r.state, OperationState::Succeeded, "{fault}");
        assert!(!path.exists());
        assert_eq!(
            f.count("GET /export"),
            usize::from(fault == "wrong_export_memory")
        );
    }
}
#[tokio::test]
async fn checkpoint_import_retains_new_memory_identity_without_resuming_and_rejects_wrong_graph() {
    for fault in [
        "",
        "missing",
        "wrong_size",
        "wrong_type",
        "wrong_owner",
        "existing_memory",
    ] {
        let f = Fixture::new(Fault::None).await;
        setup(&f);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("input.xva");
        std::fs::write(&path, archive(2, 10, true)).unwrap();
        {
            let mut r = f.records.lock().unwrap();
            r.insert("fixture:checkpoint_fault".into(), json!(fault));
            if fault == "existing_memory" {
                r.insert("OpaqueRef:VDI:already".into(), json!({"uuid":uuid(24)}));
            }
        }
        let r = run(
            &f,
            dir.path(),
            TransferAction::ImportCheckpointXva { sr: uuid(4).into() },
            TransferFile::source(&path).unwrap(),
        )
        .await;
        if fault.is_empty() {
            assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
            assert_eq!(r.transfer_created.suspend_images, vec![uuid(24).into()]);
            assert_eq!(r.transfer_created.vms, vec![uuid(20).into()]);
            assert!(r.detail.contains("suspended"));
        } else {
            assert_ne!(r.state, OperationState::Succeeded, "{fault}");
        }
        assert_eq!(f.count("PUT /import"), 1);
        assert_eq!(f.count("VM.resume"), 0);
        assert_eq!(f.count("VM.start"), 0);
    }
}
#[tokio::test]
async fn checkpoint_import_rechecks_without_replay_and_revalidates_file_before_task_creation() {
    let f = Fixture::new(Fault::LostImport).await;
    setup(&f);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("input.xva");
    std::fs::write(&path, archive(2, 10, true)).unwrap();
    let r = run(
        &f,
        dir.path(),
        TransferAction::ImportCheckpointXva { sr: uuid(4).into() },
        TransferFile::source(&path).unwrap(),
    )
    .await;
    assert_eq!(r.state, OperationState::PartiallyCompleted, "{}", r.detail);
    assert_eq!(r.transfer_created.suspend_images, vec![uuid(24).into()]);
    let _ = recheck(&f, dir.path(), r).await;
    assert_eq!(f.count("PUT /import"), 1);
    for normal in [false, true] {
        let f = Fixture::new(Fault::None).await;
        setup(&f);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("input.xva");
        std::fs::write(&path, archive(2, 10, true)).unwrap();
        let file = TransferFile::source(&path).unwrap();
        if !normal {
            std::fs::write(&path, archive(2, 99, true)).unwrap();
        }
        let a = if normal {
            TransferAction::ImportXva { sr: uuid(4).into() }
        } else {
            TransferAction::ImportCheckpointXva { sr: uuid(4).into() }
        };
        let r = run(&f, dir.path(), a, file).await;
        assert_ne!(r.state, OperationState::Succeeded);
        assert_eq!(f.count("PUT /import"), 0);
        assert_eq!(f.count("task.create"), 0);
    }
}

#[tokio::test]
async fn checkpoint_recovery_and_acknowledgement_fence_late_task_response() {
    for acknowledge in [false, true] {
        let f = Fixture::new(Fault::None).await;
        setup(&f);
        f.records
            .lock()
            .unwrap()
            .insert("fixture:checkpoint_task_gate".into(), json!(true));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("input.xva");
        std::fs::write(&path, archive(2, 10, true)).unwrap();
        let work = run(
            &f,
            dir.path(),
            TransferAction::ImportCheckpointXva { sr: uuid(4).into() },
            TransferFile::source(&path).unwrap(),
        );
        let interrupt = async {
            f.response_gate.wait().await;
            let j = Journal::open(dir.path()).unwrap();
            let rows = j.resource_recover(uuid(1).into()).unwrap();
            assert_eq!(rows.len(), 1);
            assert!(
                rows[0]
                    .transfer_plan
                    .as_ref()
                    .unwrap()
                    .stream_receipt
                    .is_some()
            );
            if acknowledge {
                j.resource_control(
                    rows[0].pool,
                    rows[0].id,
                    crate::OperationControl::AcknowledgeUnknown,
                )
                .unwrap();
            }
            f.response_gate.resume();
        };
        let (r, ()) = tokio::join!(work, interrupt);
        let saved = Journal::open(dir.path())
            .unwrap()
            .resource_load(r.id)
            .unwrap()
            .unwrap();
        assert_eq!(
            saved.state,
            if acknowledge {
                OperationState::AcknowledgedUnknown
            } else {
                OperationState::OutcomeUnknown
            }
        );
        assert!(saved.interrupted);
        assert_eq!(f.count("PUT /import"), 1);
        let checked = recheck(&f, dir.path(), saved).await;
        assert_eq!(
            checked.state,
            if acknowledge {
                OperationState::AcknowledgedUnknown
            } else {
                OperationState::Succeeded
            },
            "{}",
            checked.detail
        );
        if !acknowledge {
            assert_eq!(
                checked.transfer_created.suspend_images,
                vec![uuid(24).into()]
            );
        }
        assert_eq!(f.count("PUT /import"), 1);
        assert_eq!(f.count("VM.resume"), 0);
    }
}

#[tokio::test]
async fn checkpoint_journal_rejects_false_success_and_memory_identity_removal() {
    let f = Fixture::new(Fault::LostImport).await;
    setup(&f);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("input.xva");
    std::fs::write(&path, archive(2, 10, true)).unwrap();
    let r = run(
        &f,
        dir.path(),
        TransferAction::ImportCheckpointXva { sr: uuid(4).into() },
        TransferFile::source(&path).unwrap(),
    )
    .await;
    let j = Journal::open(dir.path()).unwrap();
    let saved = j.resource_load(r.id).unwrap().unwrap();
    let mut fake = saved.clone();
    fake.state = OperationState::Succeeded;
    assert!(j.resource_save(&mut fake).is_err());
    let mut fake = saved.clone();
    fake.transfer_created.suspend_images.clear();
    assert!(j.resource_save(&mut fake).is_err());
    let mut fake = saved.clone();
    fake.transfer_plan
        .as_mut()
        .unwrap()
        .file
        .manifest
        .as_mut()
        .unwrap()
        .guests[0]
        .memory = None;
    assert!(j.resource_save(&mut fake).is_err());
    let mut fake = saved.clone();
    fake.transfer_plan = None;
    assert!(j.resource_save(&mut fake).is_err());
    assert_eq!(
        j.resource_load(r.id)
            .unwrap()
            .unwrap()
            .transfer_created
            .suspend_images,
        saved.transfer_created.suspend_images
    );
}

#[tokio::test]
async fn checkpoint_late_created_identities_survive_recovery_and_acknowledgement() {
    for acknowledge in [false, true] {
        let f = Fixture::new(Fault::None).await;
        setup(&f);
        f.records
            .lock()
            .unwrap()
            .insert("fixture:checkpoint_identity_gate".into(), json!(true));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("input.xva");
        std::fs::write(&path, archive(2, 10, true)).unwrap();
        let work = run(
            &f,
            dir.path(),
            TransferAction::ImportCheckpointXva { sr: uuid(4).into() },
            TransferFile::source(&path).unwrap(),
        );
        let interrupt = async {
            f.response_gate.wait().await;
            let j = Journal::open(dir.path()).unwrap();
            let r = j.resource_recover(uuid(1).into()).unwrap().remove(0);
            if acknowledge {
                j.resource_control(r.pool, r.id, crate::OperationControl::AcknowledgeUnknown)
                    .unwrap();
            }
            f.response_gate.resume();
        };
        let (r, ()) = tokio::join!(work, interrupt);
        let saved = Journal::open(dir.path())
            .unwrap()
            .resource_load(r.id)
            .unwrap()
            .unwrap();
        assert_eq!(
            saved.state,
            if acknowledge {
                OperationState::AcknowledgedUnknown
            } else {
                OperationState::OutcomeUnknown
            }
        );
        assert_eq!(saved.transfer_created.vms, vec![uuid(20).into()]);
        assert_eq!(saved.transfer_created.suspend_images, vec![uuid(24).into()]);
        assert!(saved.transfer_plan.as_ref().unwrap().task_success);
        assert_eq!(f.count("PUT /import"), 1);
    }
}

#[tokio::test]
async fn checkpoint_pending_recheck_never_sends_a_previously_unsent_cancellation() {
    let f = Fixture::new(Fault::Pending).await;
    setup(&f);
    f.records
        .lock()
        .unwrap()
        .insert("fixture:checkpoint_task_gate".into(), json!(true));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("input.xva");
    std::fs::write(&path, archive(2, 10, true)).unwrap();
    let work = run(
        &f,
        dir.path(),
        TransferAction::ImportCheckpointXva { sr: uuid(4).into() },
        TransferFile::source(&path).unwrap(),
    );
    let interrupt = async {
        f.response_gate.wait().await;
        let j = Journal::open(dir.path()).unwrap();
        j.resource_recover(uuid(1).into()).unwrap();
        f.response_gate.resume();
    };
    let (r, ()) = tokio::join!(work, interrupt);
    let j = Journal::open(dir.path()).unwrap();
    let requested = j
        .resource_control(r.pool, r.id, crate::OperationControl::RequestCancellation)
        .unwrap();
    let checked = recheck(&f, dir.path(), requested).await;
    assert_eq!(checked.state, OperationState::OutcomeUnknown);
    assert!(checked.cancel_requested);
    assert!(!checked.cancel_dispatched);
    assert_eq!(f.count("task.cancel"), 0);
    assert_eq!(f.count("PUT /import"), 1);
}
