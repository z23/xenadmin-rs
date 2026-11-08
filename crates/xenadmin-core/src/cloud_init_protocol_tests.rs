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
        ResourceAction::CloudInit(config),
        tokio::runtime::Handle::current(),
    );
    request.record.transfer = Some(Default::default());
    request.record.transfer_plan = Some(Box::new(Plan::new(media.file.clone())));
    request.record.cloud_init_plan = Some(crate::cloud_init::Plan {
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
        std::time::Duration::from_secs(10),
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
            for _ in 0..500 {
                if f.count("VBD.create") > 0 {
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
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(2)).await;
            }
            panic!("attachment not dispatched");
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
            for _ in 0..500 {
                if f.count("PUT /import_raw_vdi") > 0 {
                    let journal = Journal::open(d.path()).unwrap();
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
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(2)).await;
            }
            panic!("upload not dispatched");
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
    r.cloud_init_plan.as_mut().unwrap().upload_verified = true;
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
        Err(Error::JournalChanged)
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
