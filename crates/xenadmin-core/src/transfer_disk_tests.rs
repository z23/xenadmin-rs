use super::*;
use crate::DiskImageFormat;
pub(super) async fn execute_disk(
    f: &Fixture,
    dir: &Path,
    format: DiskImageFormat,
    export: bool,
) -> crate::ResourceOperationRecord {
    let file = if export {
        TransferFile::destination(dir.join(format!("output.{}", format.extension()))).unwrap()
    } else {
        let path = dir.join(format!("input.{}", format.extension()));
        std::fs::write(
            &path,
            if format == DiskImageFormat::Vhd && f.fault == Fault::DynamicDisk {
                crate::disk_image::tests::dynamic(true)
            } else if format == DiskImageFormat::Vhd {
                crate::disk_image::tests::fixed()
            } else {
                vec![7; 4096]
            },
        )
        .unwrap();
        if f.fault == Fault::ChangedInput {
            f.records
                .lock()
                .unwrap()
                .insert("fixture:sourcepath".into(), json!(path));
        }
        TransferFile::disk_source(path, format).unwrap()
    };
    let action = if export {
        TransferAction::ExportDisk {
            vdi: uuid(10).into(),
            expected_sr: uuid(4).into(),
            expected_size: 4096,
            format,
        }
    } else {
        TransferAction::ImportDisk {
            sr: uuid(4).into(),
            name: "New disk".into(),
            format,
        }
    };
    let (observed, pending) = resources::transfer_channel(
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
        std::time::Duration::from_secs(5),
        pending.run(
            Arc::new(f.session().await),
            dir.into(),
            CancellationToken::new(),
            tx,
        ),
    )
    .await
    .unwrap();
    (*observed.latest()).clone()
}
#[tokio::test]
async fn raw_and_vhd_import_create_only_one_new_unattached_disk() {
    for format in [DiskImageFormat::Raw, DiskImageFormat::Vhd] {
        let f = Fixture::new(Fault::None).await;
        f.disk();
        let original = f.records.lock().unwrap()["OpaqueRef:VDI:source"].clone();
        let d = tempfile::tempdir().unwrap();
        let r = execute_disk(&f, d.path(), format, false).await;
        assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
        assert_eq!(r.transfer_created.disks, vec![uuid(22).into()]);
        assert_eq!(f.count("VDI.create"), 1);
        assert_eq!(f.count("PUT /import_raw_vdi"), 1);
        assert_eq!(f.count("task.destroy"), 1);
        assert_eq!(f.records.lock().unwrap()["OpaqueRef:VDI:source"], original);
        assert!(
            f.records.lock().unwrap()["OpaqueRef:VDI:new"]["VBDs"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        let public = serde_json::to_string(&r).unwrap();
        assert!(!public.contains("transfer_plan"));
        assert!(!public.contains("OpaqueRef:"));
        assert!(!public.contains(d.path().to_str().unwrap()));
    }
}
#[tokio::test]
async fn raw_and_vhd_export_are_validated_and_private() {
    use std::os::unix::fs::PermissionsExt;
    for format in [DiskImageFormat::Raw, DiskImageFormat::Vhd] {
        let f = Fixture::new(Fault::None).await;
        f.disk();
        let d = tempfile::tempdir().unwrap();
        let r = execute_disk(&f, d.path(), format, true).await;
        assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
        let path = d.path().join(format!("output.{}", format.extension()));
        assert_eq!(path.metadata().unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(
            TransferFile::disk_source(path, format)
                .unwrap()
                .disk_image()
                .unwrap()
                .virtual_size,
            4096
        );
        assert_eq!(f.count("GET /export_raw_vdi"), 1);
        assert_eq!(f.count("VDI.create"), 0);
    }
}
#[tokio::test]
async fn lost_disk_creation_reply_recovers_identity_without_upload_or_replay() {
    let f = Fixture::new(Fault::LostDiskCreate).await;
    f.disk();
    let d = tempfile::tempdir().unwrap();
    let r = execute_disk(&f, d.path(), DiskImageFormat::Raw, false).await;
    assert_eq!(r.state, OperationState::OutcomeUnknown, "{}", r.detail);
    let r = recheck(&f, d.path(), r).await;
    assert_eq!(r.state, OperationState::PartiallyCompleted, "{}", r.detail);
    assert_eq!(r.transfer_created.disks, vec![uuid(22).into()]);
    assert_eq!(f.count("VDI.create"), 1);
    assert_eq!(f.count("PUT /import_raw_vdi"), 0);
}
#[tokio::test]
async fn lost_upload_reply_uses_owned_task_and_never_reuploads() {
    let f = Fixture::new(Fault::LostImport).await;
    f.disk();
    let d = tempfile::tempdir().unwrap();
    let r = execute_disk(&f, d.path(), DiskImageFormat::Raw, false).await;
    assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
    let _ = recheck(&f, d.path(), r).await;
    assert_eq!(f.count("VDI.create"), 1);
    assert_eq!(f.count("PUT /import_raw_vdi"), 1);
}
#[tokio::test]
async fn existing_uuid_and_misbound_result_never_claim_import_success() {
    for fault in [Fault::ExistingDisk, Fault::WrongDiskResult] {
        let f = Fixture::new(fault).await;
        f.disk();
        let d = tempfile::tempdir().unwrap();
        let r = execute_disk(&f, d.path(), DiskImageFormat::Raw, false).await;
        assert_eq!(r.state, OperationState::OutcomeUnknown, "{}", r.detail);
        let r = recheck(&f, d.path(), r).await;
        assert_ne!(r.state, OperationState::Succeeded);
        assert_eq!(f.count("VDI.create"), 1);
        assert_eq!(
            f.count("PUT /import_raw_vdi"),
            usize::from(fault == Fault::WrongDiskResult)
        );
    }
}
#[tokio::test]
async fn failed_disk_upload_retains_identified_partial_disk() {
    let f = Fixture::new(Fault::DiskFailure).await;
    f.disk();
    let d = tempfile::tempdir().unwrap();
    let r = execute_disk(&f, d.path(), DiskImageFormat::Vhd, false).await;
    assert_eq!(r.state, OperationState::PartiallyCompleted, "{}", r.detail);
    assert_eq!(r.transfer_created.disks, vec![uuid(22).into()]);
    assert_eq!(f.count("VDI.destroy"), 0);
    assert!(f.records.lock().unwrap().contains_key("OpaqueRef:VDI:new"));
}
#[tokio::test]
async fn export_refuses_live_or_changed_disks_and_never_publishes() {
    for mode in 0..3 {
        let f = Fixture::new(if mode == 2 {
            Fault::ChangedDisk
        } else {
            Fault::None
        })
        .await;
        f.disk();
        if mode < 2 {
            let mut records = f.records.lock().unwrap();
            records.get_mut("OpaqueRef:VDI:source").unwrap()["VBDs"] =
                json!(["OpaqueRef:VBD:live"]);
            records.insert("OpaqueRef:VBD:live".into(),json!({"VDI":"OpaqueRef:VDI:source","VM":"OpaqueRef:VM:source","currently_attached":mode==0,"current_operations":{}}));
            if mode == 1 {
                records.get_mut("OpaqueRef:VM:source").unwrap()["power_state"] = json!("Running");
            }
        }
        let d = tempfile::tempdir().unwrap();
        let r = execute_disk(&f, d.path(), DiskImageFormat::Raw, true).await;
        assert_ne!(r.state, OperationState::Succeeded);
        assert!(!d.path().join("output.raw").exists());
        assert_eq!(f.count("GET /export_raw_vdi"), usize::from(mode == 2));
    }
}
#[tokio::test]
async fn readonly_session_exports_but_never_creates_import_disk() {
    let mut f = Fixture::new(Fault::None).await;
    f.disk();
    f.options.read_only = true;
    let d = tempfile::tempdir().unwrap();
    let r = execute_disk(&f, d.path(), DiskImageFormat::Raw, true).await;
    assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
    let d = tempfile::tempdir().unwrap();
    let r = execute_disk(&f, d.path(), DiskImageFormat::Raw, false).await;
    assert_eq!(r.state, OperationState::Rejected, "{}", r.detail);
    assert_eq!(f.count("VDI.create"), 0);
}

#[tokio::test]
async fn dynamic_vhd_uses_virtual_capacity_instead_of_file_length() {
    let f = Fixture::new(Fault::DynamicDisk).await;
    f.disk();
    let d = tempfile::tempdir().unwrap();
    let r = execute_disk(&f, d.path(), DiskImageFormat::Vhd, false).await;
    assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
    assert_eq!(
        f.records.lock().unwrap()["OpaqueRef:VDI:new"]["virtual_size"],
        "8192"
    );
    assert_eq!(f.count("PUT /import_raw_vdi"), 1);
}
#[tokio::test]
async fn cancelled_disk_task_retains_partial_identity_and_never_replays() {
    let f = Fixture::new(Fault::PendingDisk).await;
    f.disk();
    let d = tempfile::tempdir().unwrap();
    let r = execute_disk(&f, d.path(), DiskImageFormat::Raw, false).await;
    assert_eq!(r.state, OperationState::OutcomeUnknown, "{}", r.detail);
    let r = Journal::open(d.path())
        .unwrap()
        .resource_control(
            uuid(1).into(),
            r.id,
            crate::OperationControl::RequestCancellation,
        )
        .unwrap();
    let r = recheck(&f, d.path(), r).await;
    let r = recheck(&f, d.path(), r).await;
    assert_eq!(r.state, OperationState::PartiallyCompleted, "{}", r.detail);
    assert_eq!(r.transfer_created.disks, vec![uuid(22).into()]);
    assert_eq!(f.count("task.cancel"), 1);
    assert_eq!(f.count("PUT /import_raw_vdi"), 1);
    assert_eq!(f.count("VDI.destroy"), 0);
}
#[tokio::test]
async fn disk_completion_proof_survives_restart_and_task_removal() {
    let f = Fixture::new(Fault::PendingDisk).await;
    f.disk();
    let d = tempfile::tempdir().unwrap();
    let mut r = execute_disk(&f, d.path(), DiskImageFormat::Raw, false).await;
    assert_eq!(r.state, OperationState::OutcomeUnknown, "{}", r.detail);
    let plan = r.transfer_plan.as_mut().unwrap();
    plan.task_success = true;
    plan.result = Some(vec!["OpaqueRef:VDI:new".into()]);
    Journal::open(d.path())
        .unwrap()
        .resource_save(&mut r)
        .unwrap();
    f.records.lock().unwrap().remove("OpaqueRef:task:t");
    let r = recheck(&f, d.path(), r).await;
    assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
    assert_eq!(f.count("PUT /import_raw_vdi"), 1);
    assert_eq!(r.transfer_created.disks, vec![uuid(22).into()]);
}

#[tokio::test]
async fn source_change_with_lost_http_reply_cannot_claim_verified_import() {
    let f = Fixture::new(Fault::ChangedInput).await;
    f.disk();
    let d = tempfile::tempdir().unwrap();
    let r = execute_disk(&f, d.path(), DiskImageFormat::Raw, false).await;
    assert_eq!(r.state, OperationState::PartiallyCompleted, "{}", r.detail);
    assert!(r.detail.contains("integrity changed"));
    assert_eq!(r.transfer_created.disks, vec![uuid(22).into()]);
    assert_eq!(f.count("PUT /import_raw_vdi"), 1);
}

#[tokio::test]
async fn recovered_or_acknowledged_creation_keeps_late_identity_without_starting_upload() {
    for acknowledge_outcome in [false, true] {
        let f = Fixture::new(Fault::DelayedDiskCreate).await;
        f.disk();
        let d = tempfile::tempdir().unwrap();
        let run = execute_disk(&f, d.path(), DiskImageFormat::Raw, false);
        let acknowledge = async {
            for _ in 0..200 {
                if f.count("VDI.create") > 0 {
                    let j = Journal::open(d.path()).unwrap();
                    let rows = j.resource_recover(uuid(1).into()).unwrap();
                    assert_eq!(rows.len(), 1);
                    let id = rows[0].id;
                    assert_eq!(rows[0].state, OperationState::OutcomeUnknown);
                    if acknowledge_outcome {
                        j.resource_control(
                            uuid(1).into(),
                            id,
                            crate::OperationControl::AcknowledgeUnknown,
                        )
                        .unwrap();
                    }
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(2)).await;
            }
            panic!("no create dispatch")
        };
        let (r, _) = tokio::join!(run, acknowledge);
        assert_eq!(
            r.state,
            if acknowledge_outcome {
                OperationState::AcknowledgedUnknown
            } else {
                OperationState::PartiallyCompleted
            },
            "{}",
            r.detail
        );
        assert_eq!(r.transfer_created.disks, vec![uuid(22).into()]);
        assert_eq!(
            r.transfer_plan
                .unwrap()
                .disk_creation
                .unwrap()
                .reference
                .as_deref(),
            Some("OpaqueRef:VDI:new")
        );
        assert_eq!(f.count("VDI.create"), 1);
        assert_eq!(f.count("PUT /import_raw_vdi"), 0);
        assert_eq!(f.count("task.create"), 0);
    }
}
