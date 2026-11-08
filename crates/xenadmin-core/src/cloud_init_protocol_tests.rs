use super::*;
use crate::{CloudInitConfiguration, CloudInitInputs, CloudInitMedia, ResourceAction};
fn cloud_setup(f: &Fixture) {
    f.disk();
    let mut r = f.records.lock().unwrap();
    r.insert("fixture:cloud".into(), json!(true));
    r.get_mut("OpaqueRef:VM:source").unwrap()["VBDs"] = json!([]);
}
fn pending() -> (
    crate::ResourceOperation,
    ResourceRequest,
    std::path::PathBuf,
) {
    let vm = uuid(2).into();
    let media = CloudInitMedia::prepare(
        vm,
        CloudInitInputs::new(
            "cloud-fixture".into(),
            "#cloud-config\npassword: synthetic-sensitive-user-data\n".into(),
            None,
        )
        .unwrap(),
    )
    .unwrap();
    let path = media.file.path.clone();
    let config = CloudInitConfiguration {
        sr: uuid(4).into(),
        device: "1".into(),
        configuration_id: media.id,
    };
    let (observed, mut request) = resources::channel(
        OperationId::new(),
        ObservedResource {
            pool: uuid(1).into(),
            generation: 1,
            target: ResourceTarget::Vm(vm),
        },
        ResourceAction::CloudInit(config.clone()),
        tokio::runtime::Handle::current(),
    );
    request.record.transfer = Some(Default::default());
    request.record.transfer_plan = Some(Box::new(Plan::new(media.file.clone())));
    request.record.cloud_init_plan = Some(crate::cloud_init::Plan {
        operation: request.record.id,
        pool: request.record.pool,
        vm,
        configuration: config,
        sha256: media.sha256,
        interrupted: false,
        upload_verified: false,
        attachment: None,
    });
    request.cloud_media = Some(media);
    (observed, request, path)
}
async fn run(f: &Fixture, dir: &Path, request: ResourceRequest) {
    let (tx, _rx) = mpsc::channel(512);
    tokio::time::timeout(
        std::time::Duration::from_secs(30),
        request.run(
            Arc::new(f.session().await),
            dir.into(),
            CancellationToken::new(),
            tx,
        ),
    )
    .await
    .unwrap();
}
fn cleanup(record: &crate::ResourceOperationRecord) {
    crate::cloud_init::remove_artifact(&record.transfer_plan.as_ref().unwrap().file);
}
#[tokio::test]
async fn cloud_init_upload_attaches_exact_new_read_only_disk_and_keeps_payload_private() {
    let f = Fixture::new(Fault::None).await;
    cloud_setup(&f);
    let d = tempfile::tempdir().unwrap();
    let (observed, request, path) = pending();
    run(&f, d.path(), request).await;
    let r = observed.latest();
    assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
    assert_eq!(r.transfer_created.disks, vec![uuid(22).into()]);
    assert_eq!(r.transfer_created.vbds, vec![uuid(31).into()]);
    assert_eq!(f.count("VDI.create"), 1);
    assert_eq!(f.count("VBD.create"), 1);
    assert_eq!(f.count("PUT /import_raw_vdi"), 1);
    let records = f.records.lock().unwrap();
    assert_eq!(records["OpaqueRef:VM:source"]["power_state"], "Halted");
    assert_eq!(records["OpaqueRef:VBD:cloud"]["mode"], "RO");
    assert_eq!(records["OpaqueRef:VBD:cloud"]["bootable"], false);
    assert!(!path.exists());
    let public = serde_json::to_string(&*r).unwrap();
    for secret in [
        "synthetic-sensitive-user-data",
        "cloud-fixture",
        "OpaqueRef:",
        "seed.img",
        "cloud_init_plan",
        "sha256",
    ] {
        assert!(!public.contains(secret), "{secret}");
    }
    let journal = Journal::open(d.path()).unwrap();
    let payload: String = journal
        .0
        .query_row("SELECT payload FROM resource_operations", [], |r| r.get(0))
        .unwrap();
    assert!(!payload.contains("synthetic-sensitive-user-data"));
    assert!(payload.contains("cloud_init_plan"));
}
#[tokio::test]
async fn cloud_init_lost_attachment_reply_rechecks_without_reupload_or_recreation() {
    let f = Fixture::new(Fault::CloudLostAttach).await;
    cloud_setup(&f);
    let d = tempfile::tempdir().unwrap();
    let (observed, request, _) = pending();
    run(&f, d.path(), request).await;
    let r = (*observed.latest()).clone();
    assert_eq!(r.state, OperationState::OutcomeUnknown, "{}", r.detail);
    let r = recheck(&f, d.path(), r).await;
    assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
    assert_eq!(f.count("VDI.create"), 1);
    assert_eq!(f.count("PUT /import_raw_vdi"), 1);
    assert_eq!(f.count("VBD.create"), 1);
}
#[tokio::test]
async fn cloud_init_rejects_changed_vm_read_only_and_occupied_slot_before_writes() {
    for case in ["running", "template", "read-only", "slot"] {
        let mut f = Fixture::new(Fault::None).await;
        cloud_setup(&f);
        let d = tempfile::tempdir().unwrap();
        if case == "read-only" {
            f.options.read_only = true;
        }
        if case == "running" {
            f.records
                .lock()
                .unwrap()
                .get_mut("OpaqueRef:VM:source")
                .unwrap()["power_state"] = json!("Running");
        }
        if case == "template" {
            f.records
                .lock()
                .unwrap()
                .get_mut("OpaqueRef:VM:source")
                .unwrap()["is_a_template"] = json!(true);
        }
        let (observed, mut request, _) = pending();
        if case == "slot"
            && let ResourceAction::CloudInit(c) = &mut request.record.action
        {
            c.device = "0".into();
        }
        run(&f, d.path(), request).await;
        let r = observed.latest();
        assert_eq!(r.state, OperationState::Rejected, "{case}: {}", r.detail);
        assert_eq!(f.count("VDI.create"), 0);
        assert_eq!(f.count("VBD.create"), 0);
        cleanup(&r);
    }
}
#[tokio::test]
async fn cloud_init_changed_vm_or_wrong_attachment_retains_unknown_without_more_writes() {
    for fault in [
        Fault::CloudChangedVmAfterUpload,
        Fault::CloudWrongAttach,
        Fault::WrongDiskResult,
        Fault::LostImport,
    ] {
        let f = Fixture::new(fault).await;
        cloud_setup(&f);
        let d = tempfile::tempdir().unwrap();
        let (observed, request, _) = pending();
        run(&f, d.path(), request).await;
        let r = (*observed.latest()).clone();
        assert_eq!(r.state, OperationState::OutcomeUnknown, "{}", r.detail);
        let attaches = f.count("VBD.create");
        let r = recheck(&f, d.path(), r).await;
        assert_ne!(r.state, OperationState::Succeeded, "{}", r.detail);
        assert_eq!(f.count("VBD.create"), attaches);
        assert_eq!(f.count("PUT /import_raw_vdi"), 1);
        cleanup(&r);
    }
}
#[tokio::test]
async fn cloud_init_late_attachment_evidence_preserves_recovered_and_acknowledged_state() {
    for acknowledge in [false, true] {
        let f = Fixture::new(Fault::CloudDelayedAttach).await;
        cloud_setup(&f);
        let d = tempfile::tempdir().unwrap();
        let (observed, request, _) = pending();
        let execute = run(&f, d.path(), request);
        let recover = async {
            f.response_gate.wait().await;
            let j = Journal::open(d.path()).unwrap();
            let rows = j.resource_recover(uuid(1).into()).unwrap();
            assert_eq!(rows.len(), 1);
            if acknowledge {
                j.resource_control(
                    uuid(1).into(),
                    rows[0].id,
                    crate::OperationControl::AcknowledgeUnknown,
                )
                .unwrap();
            }
            f.response_gate.resume();
        };
        tokio::join!(execute, recover);
        let r = observed.latest();
        assert_eq!(
            r.state,
            if acknowledge {
                OperationState::AcknowledgedUnknown
            } else {
                OperationState::OutcomeUnknown
            },
            "{}",
            r.detail
        );
        assert_eq!(r.transfer_created.vbds, vec![uuid(31).into()]);
        assert!(
            r.cloud_init_plan
                .as_ref()
                .unwrap()
                .attachment
                .as_ref()
                .unwrap()
                .verified
        );
        assert_eq!(f.count("VBD.create"), 1);
        assert_eq!(f.count("task.destroy"), 0);
        cleanup(&r);
    }
}
#[tokio::test]
async fn cloud_init_artifact_changes_fail_before_creating_remote_disk() {
    let f = Fixture::new(Fault::None).await;
    cloud_setup(&f);
    let d = tempfile::tempdir().unwrap();
    let (observed, request, path) = pending();
    std::fs::write(&path, vec![0_u8; crate::cloud_init::MEDIA_BYTES]).unwrap();
    run(&f, d.path(), request).await;
    let r = observed.latest();
    assert_eq!(r.state, OperationState::Rejected, "{}", r.detail);
    assert_eq!(f.count("VDI.create"), 0);
    std::fs::remove_file(&path).unwrap();
    std::fs::remove_dir(path.parent().unwrap()).unwrap();
}

#[tokio::test]
async fn cloud_init_recovered_upload_never_attaches_and_recheck_reports_retained_disk() {
    for acknowledge in [false, true] {
        let f = Fixture::new(Fault::CloudDelayedUpload).await;
        cloud_setup(&f);
        let d = tempfile::tempdir().unwrap();
        let (observed, request, _) = pending();
        let execute = run(&f, d.path(), request);
        let recover = async {
            f.response_gate.wait().await;
            let journal = Journal::open(d.path()).unwrap();
            let before = journal
                .resource_load(observed.latest().id)
                .unwrap()
                .unwrap();
            assert!(
                before
                    .transfer_plan
                    .as_ref()
                    .unwrap()
                    .stream_receipt
                    .is_some()
            );
            let rows = journal.resource_recover(uuid(1).into()).unwrap();
            if acknowledge {
                journal
                    .resource_control(
                        uuid(1).into(),
                        rows[0].id,
                        crate::OperationControl::AcknowledgeUnknown,
                    )
                    .unwrap();
            }
            f.response_gate.resume();
        };
        tokio::join!(execute, recover);
        let r = (*observed.latest()).clone();
        assert_eq!(
            r.state,
            if acknowledge {
                OperationState::AcknowledgedUnknown
            } else {
                OperationState::OutcomeUnknown
            },
            "{}",
            r.detail
        );
        assert!(r.transfer_plan.as_ref().unwrap().stream_receipt.is_some());
        assert!(r.cloud_init_plan.as_ref().unwrap().upload_verified);
        assert_eq!(f.count("VBD.create"), 0);
        let r = recheck(&f, d.path(), r).await;
        assert_eq!(
            r.state,
            if acknowledge {
                OperationState::AcknowledgedUnknown
            } else {
                OperationState::PartiallyCompleted
            },
            "{}",
            r.detail
        );
        assert_eq!(f.count("PUT /import_raw_vdi"), 1);
        assert_eq!(f.count("VBD.create"), 0);
        cleanup(&r);
    }
}

#[tokio::test]
async fn cloud_init_existing_seed_is_not_replaced_or_deleted() {
    let f = Fixture::new(Fault::None).await;
    cloud_setup(&f);
    let d = tempfile::tempdir().unwrap();
    {
        let mut r = f.records.lock().unwrap();
        r.get_mut("OpaqueRef:VM:source").unwrap()["VBDs"] = json!(["OpaqueRef:VBD:existing"]);
        r.insert("OpaqueRef:VBD:existing".into(),json!({"uuid":uuid(60),"VM":"OpaqueRef:VM:source","other_config":{"xenadmin-rs-cloud-init":uuid(61).to_string()}}));
    }
    let (observed, request, _) = pending();
    run(&f, d.path(), request).await;
    let r = observed.latest();
    assert_eq!(r.state, OperationState::Rejected, "{}", r.detail);
    assert_eq!(f.count("VDI.create"), 0);
    assert_eq!(f.count("VBD.create"), 0);
    cleanup(&r);
}
#[tokio::test]
async fn cloud_init_journal_keeps_hash_baseline_and_evidence_monotonic() {
    let d = tempfile::tempdir().unwrap();
    let j = Journal::open(d.path()).unwrap();
    let (_, request, _) = pending();
    let mut r = request.record.clone();
    j.resource_admit(&mut r).unwrap();
    let mut bad = r.clone();
    bad.cloud_init_plan.as_mut().unwrap().sha256 = [1; 32];
    assert!(matches!(
        j.resource_save(&mut bad),
        Err(Error::JournalChanged)
    ));
    completed_upload(&mut r);
    r.cloud_init_plan.as_mut().unwrap().attachment = Some(crate::cloud_init::Attachment {
        baseline: vec![uuid(50)],
        reference: Some("OpaqueRef:private-attachment".into()),
        uuid: None,
        verified: false,
    });
    r.state = OperationState::OutcomeUnknown;
    j.resource_save(&mut r).unwrap();
    let mut bad = r.clone();
    bad.cloud_init_plan.as_mut().unwrap().upload_verified = false;
    assert!(matches!(
        j.resource_save(&mut bad),
        Err(Error::JournalChanged | Error::Journal)
    ));
    let mut bad = r.clone();
    bad.cloud_init_plan
        .as_mut()
        .unwrap()
        .attachment
        .as_mut()
        .unwrap()
        .baseline
        .clear();
    assert!(matches!(
        j.resource_save(&mut bad),
        Err(Error::JournalChanged)
    ));
    let mut bad = r.clone();
    bad.state = OperationState::Preparing;
    bad.cloud_init_plan.as_mut().unwrap().interrupted = false;
    assert!(matches!(
        j.resource_save(&mut bad),
        Err(Error::JournalChanged)
    ));
    assert!(
        !serde_json::to_string(&r)
            .unwrap()
            .contains("private-attachment")
    );
    assert_eq!(
        j.resource_load(r.id).unwrap().unwrap().cloud_init_plan,
        r.cloud_init_plan
    );
}

fn completed_upload(record: &mut crate::ResourceOperationRecord) {
    let plan = record.transfer_plan.as_mut().unwrap();
    let disk = crate::VdiId::from(uuid(77));
    plan.disk_creation = Some(crate::transfer::disk::Creation {
        baseline: vec![uuid(76)],
        dispatched: true,
        reference: Some("OpaqueRef:private-disk".into()),
        uuid: Some(disk),
    });
    record.transfer_created.disks = vec![disk];
    record.task = Some("OpaqueRef:private-upload".into());
    plan.task_uuid = Some(uuid(78));
    plan.dispatched = true;
    plan.task_success = true;
    plan.result = Some(vec!["OpaqueRef:private-disk".into()]);
    plan.stream_receipt = Some(crate::transfer::files::Receipt {
        bytes: crate::cloud_init::MEDIA_BYTES as u64,
        sha256: [0; 32],
        identity: plan.file.identity.clone().unwrap(),
    });
    record.cloud_init_plan.as_mut().unwrap().upload_verified = true;
    record.state = OperationState::Tracking;
}

#[tokio::test]
async fn cloud_init_journal_rejects_mismatched_context_and_impossible_evidence() {
    let d = tempfile::tempdir().unwrap();
    let j = Journal::open(d.path()).unwrap();
    let (_, request, _) = pending();
    let mut valid = request.record.clone();
    completed_upload(&mut valid);
    j.resource_admit(&mut valid).unwrap();
    let original: String =
        j.0.query_row("SELECT payload FROM resource_operations", [], |r| r.get(0))
            .unwrap();
    for case in [
        "operation",
        "pool",
        "vm",
        "sr",
        "slot",
        "configuration",
        "missing-plan",
        "wrong-action",
        "source-size",
        "image-size",
        "image-format",
        "path",
        "upload-receipt",
        "disk-baseline",
        "disk-baseline-limit",
        "disk-identity",
        "attachment-before-upload",
        "attachment-identity",
        "task-result",
        "task-outcomes",
        "success",
    ] {
        let mut bad = valid.clone();
        match case {
            "operation" => bad.cloud_init_plan.as_mut().unwrap().operation = OperationId::new(),
            "pool" => bad.cloud_init_plan.as_mut().unwrap().pool = uuid(90).into(),
            "vm" => bad.target = ResourceTarget::Vm(uuid(90).into()),
            "sr" => bad.cloud_init_plan.as_mut().unwrap().configuration.sr = uuid(90).into(),
            "slot" => bad.cloud_init_plan.as_mut().unwrap().configuration.device = "2".into(),
            "configuration" => {
                bad.cloud_init_plan
                    .as_mut()
                    .unwrap()
                    .configuration
                    .configuration_id = uuid(90)
            }
            "missing-plan" => bad.cloud_init_plan = None,
            "wrong-action" => bad.action = ResourceAction::CancelTask,
            "source-size" => {
                bad.transfer_plan
                    .as_mut()
                    .unwrap()
                    .file
                    .identity
                    .as_mut()
                    .unwrap()
                    .size = 1
            }
            "image-size" => {
                bad.transfer_plan
                    .as_mut()
                    .unwrap()
                    .file
                    .disk
                    .as_mut()
                    .unwrap()
                    .virtual_size = 1
            }
            "image-format" => {
                bad.transfer_plan
                    .as_mut()
                    .unwrap()
                    .file
                    .disk
                    .as_mut()
                    .unwrap()
                    .format = crate::DiskImageFormat::Vhd
            }
            "path" => bad.transfer_plan.as_mut().unwrap().file.path = "/tmp/unrelated-file".into(),
            "upload-receipt" => bad.transfer_plan.as_mut().unwrap().stream_receipt = None,
            "disk-baseline" => {
                bad.transfer_plan
                    .as_mut()
                    .unwrap()
                    .disk_creation
                    .as_mut()
                    .unwrap()
                    .baseline = vec![uuid(76), uuid(76)]
            }
            "disk-baseline-limit" => {
                bad.transfer_plan
                    .as_mut()
                    .unwrap()
                    .disk_creation
                    .as_mut()
                    .unwrap()
                    .baseline = vec![uuid(76); 10_001]
            }
            "disk-identity" => bad.transfer_created.disks = vec![uuid(90).into()],
            "attachment-before-upload" | "attachment-identity" => {
                bad.cloud_init_plan.as_mut().unwrap().attachment =
                    Some(crate::cloud_init::Attachment {
                        baseline: vec![uuid(90)],
                        reference: Some("OpaqueRef:private-attachment".into()),
                        uuid: Some(uuid(90).into()),
                        verified: true,
                    });
                bad.transfer_created.vbds = vec![uuid(90).into()];
                if case == "attachment-before-upload" {
                    bad.cloud_init_plan.as_mut().unwrap().upload_verified = false;
                }
            }
            "task-result" => {
                bad.transfer_plan.as_mut().unwrap().result =
                    Some(vec!["OpaqueRef:wrong-disk".into()])
            }
            "task-outcomes" => bad.transfer_plan.as_mut().unwrap().task_failed = true,
            "success" => bad.state = OperationState::Succeeded,
            _ => unreachable!(),
        }
        // Admission must reject malformed duplicates too, rather than accepting the old row.
        assert!(
            matches!(j.resource_admit(&mut bad), Err(Error::Journal)),
            "admit {case}"
        );
        assert!(
            matches!(j.resource_save(&mut bad), Err(Error::Journal)),
            "save {case}"
        );
        let mut payload = serde_json::to_value(&bad).unwrap();
        payload["cloud_init_plan"] = serde_json::to_value(&bad.cloud_init_plan).unwrap();
        payload["transfer_plan"] = serde_json::to_value(&bad.transfer_plan).unwrap();
        j.0.execute(
            "UPDATE resource_operations SET payload=?1",
            [payload.to_string()],
        )
        .unwrap();
        assert!(
            matches!(j.resource_load(valid.id), Err(Error::Journal)),
            "load {case}"
        );
        j.0.execute("UPDATE resource_operations SET payload=?1", [&original])
            .unwrap();
    }
    assert_eq!(
        j.resource_load(valid.id).unwrap().unwrap().cloud_init_plan,
        valid.cloud_init_plan
    );
}

#[tokio::test]
async fn cloud_init_journal_accepts_rejection_before_media_preparation_only() {
    let d = tempfile::tempdir().unwrap();
    let j = Journal::open(d.path()).unwrap();
    let (_, request, _) = pending();
    let mut rejected = request.record.clone();
    rejected.state = OperationState::Rejected;
    rejected.cloud_init_plan = None;
    rejected.transfer_plan = None;
    j.resource_save(&mut rejected).unwrap();
    assert_eq!(
        j.resource_load(rejected.id).unwrap().unwrap().state,
        OperationState::Rejected
    );
    let mut unsupported = rejected.clone();
    unsupported.id = OperationId::new();
    unsupported.state = OperationState::OutcomeUnknown;
    assert!(matches!(
        j.resource_admit(&mut unsupported),
        Err(Error::Journal)
    ));
}

#[tokio::test]
async fn cloud_init_acknowledgement_before_http_receipt_never_claims_upload_or_attaches() {
    let f = Fixture::new(Fault::CloudInterruptedUpload).await;
    cloud_setup(&f);
    let d = tempfile::tempdir().unwrap();
    let (observed, request, _) = pending();
    let execute = run(&f, d.path(), request);
    let recover = async {
        f.response_gate.wait().await;
        let journal = Journal::open(d.path()).unwrap();
        let rows = journal.resource_recover(uuid(1).into()).unwrap();
        assert_eq!(rows.len(), 1);
        journal
            .resource_control(
                uuid(1).into(),
                rows[0].id,
                crate::OperationControl::AcknowledgeUnknown,
            )
            .unwrap();
        // Keep the response blocked until the HTTP driver's progress callback observes recovery.
    };
    tokio::join!(execute, recover);
    let r = (*observed.latest()).clone();
    assert_eq!(r.state, OperationState::AcknowledgedUnknown, "{}", r.detail);
    assert!(r.transfer_plan.as_ref().unwrap().stream_receipt.is_none());
    assert!(!r.cloud_init_plan.as_ref().unwrap().upload_verified);
    assert_eq!(f.count("VBD.create"), 0);
    f.response_gate.resume();
    let r = recheck(&f, d.path(), r).await;
    assert_eq!(r.state, OperationState::AcknowledgedUnknown, "{}", r.detail);
    assert_eq!(f.count("PUT /import_raw_vdi"), 1);
    assert_eq!(f.count("VBD.create"), 0);
    cleanup(&r);
}
