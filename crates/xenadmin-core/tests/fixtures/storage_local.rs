use super::*;
use xenadmin_core::{LocalDeviceDeclaration, LocalStorageKind};
const DEVICE: &str = "/dev/disk/by-id/ata-FIXTURE";
fn local_create(kind: LocalStorageKind) -> StorageOperation {
    StorageOperation::Create {
        host: HostId::parse(HOST).unwrap(),
        name: "Fixture local storage".into(),
        description: "Synthetic".into(),
        location: StorageLocation::Local {
            kind,
            device: DEVICE.into(),
        },
        hba_device: None,
        local_device: Some(LocalDeviceDeclaration {
            identity_statement: "Operator declares lab disk Fixture for VM storage".into(),
            acknowledge_data_loss: true,
        }),
    }
}
fn local_drivers(records: &Records) {
    let mut m = records.lock().unwrap();
    for kind in [LocalStorageKind::Ext, LocalStorageKind::Lvm] {
        m.insert(format!("SM-{}",kind.kind()),json!({"type":kind.kind(),"name_label":kind.label(),"capabilities":["SR_PROBE"],"configuration":{"device":"local device path"}}));
    }
}
#[tokio::test]
async fn storage_local_creation_probes_both_types_and_verifies_nonshared_exact_attachment() {
    for kind in [LocalStorageKind::Ext, LocalStorageKind::Lvm] {
        let (mock, data) = fixture(Fault::None).await;
        local_drivers(&data);
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connection(&mock, &backend, false).await;
        let r = finished(&submit(&c, pool(), local_create(kind))).await;
        assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
        assert_eq!(data.lock().unwrap()["SR-created"]["shared"], false);
        assert_eq!(data.lock().unwrap()["SR-created"]["type"], kind.kind());
        assert_eq!(
            data.lock().unwrap()["PBD-created"]["device_config"],
            json!({"device":DEVICE})
        );
        assert_eq!(
            mock.methods
                .lock()
                .unwrap()
                .iter()
                .filter(|m| m.as_str() == "SR.probe")
                .count(),
            4
        );
        assert_eq!(
            mock.methods
                .lock()
                .unwrap()
                .iter()
                .filter(|m| m.starts_with("Async."))
                .count(),
            1
        );
        c.close().await;
    }
}
#[tokio::test]
async fn storage_local_refuses_either_existing_backend_changed_probe_and_malformed_response() {
    for fault in [
        Fault::LocalExisting,
        Fault::LocalChanged,
        Fault::LocalMalformed,
    ] {
        let (mock, data) = fixture(fault).await;
        local_drivers(&data);
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connection(&mock, &backend, false).await;
        let r = finished(&submit(&c, pool(), local_create(LocalStorageKind::Ext))).await;
        assert_eq!(r.state, OperationState::Rejected, "{}", r.detail);
        assert!(
            !mock
                .methods
                .lock()
                .unwrap()
                .iter()
                .any(|m| m.starts_with("Async."))
        );
        c.close().await;
    }
}
#[tokio::test]
async fn storage_local_refuses_known_device_family_serial_and_incomplete_driver_evidence() {
    for case in 0..4 {
        let (mock, data) = fixture(Fault::None).await;
        local_drivers(&data);
        {
            let mut data = data.lock().unwrap();
            match case {
                0 => {
                    data.get_mut("PBD").unwrap()["device_config"]["device"] =
                        json!(format!("{DEVICE}-part1"))
                }
                1 => data.get_mut("SR").unwrap()["sm_config"]["devserial"] = json!("ata-FIXTURE"),
                2 => {
                    data.remove("SM-lvm");
                }
                _ => data.get_mut("SM-ext").unwrap()["configuration"] = json!({}),
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connection(&mock, &backend, false).await;
        let r = finished(&submit(&c, pool(), local_create(LocalStorageKind::Ext))).await;
        assert_eq!(
            r.state,
            OperationState::Rejected,
            "case {case}: {}",
            r.detail
        );
        assert!(
            !mock
                .methods
                .lock()
                .unwrap()
                .iter()
                .any(|m| m.starts_with("Async."))
        );
        c.close().await;
    }
}
#[tokio::test]
async fn storage_local_discovery_is_nonformatting_and_retains_existing_repository_kind() {
    let (mock, data) = fixture(Fault::LocalExisting).await;
    local_drivers(&data);
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connection(&mock, &backend, true).await;
    let probe = c
        .discover_storage(
            c.latest().observe_resource(pool()).unwrap(),
            StorageQuery::LocalDevice {
                host: HostId::parse(HOST).unwrap(),
                device: DEVICE.into(),
            },
        )
        .unwrap();
    let result = tokio::time::timeout(deadline(3), async {
        loop {
            if let Some(result) = probe.latest() {
                break result.unwrap();
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let StorageDiscoveryResult::LocalRepositories(rows) = result else {
        panic!("wrong query result")
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].kind, LocalStorageKind::Lvm);
    assert_eq!(rows[0].repository.uuid, SrId::parse(CREATED).unwrap());
    assert!(
        c.submit_resource(
            OperationId::new(),
            ResourceCommand::Storage {
                target: c.latest().observe_resource(pool()).unwrap(),
                operation: local_create(LocalStorageKind::Ext)
            }
        )
        .is_err()
    );
    assert!(
        !mock
            .methods
            .lock()
            .unwrap()
            .iter()
            .any(|m| m.starts_with("Async."))
    );
    c.close().await;
}
#[tokio::test]
async fn storage_local_lost_failed_or_wrong_shared_result_stays_unknown_without_replay_or_cleanup()
{
    for fault in [
        Fault::DropReply,
        Fault::TaskFailure,
        Fault::LocalWrongShared,
    ] {
        let (mock, data) = fixture(fault).await;
        local_drivers(&data);
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connection(&mock, &backend, false).await;
        let r = finished(&submit(&c, pool(), local_create(LocalStorageKind::Lvm))).await;
        assert_eq!(r.state, OperationState::OutcomeUnknown, "{}", r.detail);
        recheck_unknown(&c, &r).await;
        assert_eq!(
            mock.methods
                .lock()
                .unwrap()
                .iter()
                .filter(|m| m.as_str() == "Async.SR.create")
                .count(),
            1
        );
        assert!(
            !mock
                .methods
                .lock()
                .unwrap()
                .iter()
                .any(|m| m == "Async.SR.destroy" || m == "Async.SR.forget")
        );
        c.close().await;
    }
}
#[tokio::test]
async fn storage_local_existing_attachment_cannot_be_added_to_another_host_or_multiplugged() {
    let (mock, data) = fixture(Fault::None).await;
    local_drivers(&data);
    {
        let mut m = data.lock().unwrap();
        m.get_mut("SR").unwrap()["type"] = json!("ext");
        m.get_mut("SR").unwrap()["shared"] = json!(false);
        m.get_mut("PBD").unwrap()["device_config"] = json!({"device":DEVICE});
    }
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connection(&mock, &backend, false).await;
    let r = finished(&submit(
        &c,
        sr(),
        StorageOperation::CreateAttachment {
            host: HostId::from(uuid::Uuid::from_u128(900)),
            location: StorageLocation::Local {
                kind: LocalStorageKind::Ext,
                device: DEVICE.into(),
            },
        },
    ))
    .await;
    assert_eq!(r.state, OperationState::Rejected);
    assert!(
        !mock
            .methods
            .lock()
            .unwrap()
            .iter()
            .any(|m| m == "Async.PBD.create")
    );
    data.lock().unwrap().get_mut("SR").unwrap()["PBDs"] =
        json!(["OpaqueRef:PBD", "OpaqueRef:foreign"]);
    let r = finished(&submit(
        &c,
        pbd(),
        StorageOperation::SetAttached {
            sr: SrId::parse(SR).unwrap(),
            host: HostId::parse(HOST).unwrap(),
            expected_attached: false,
            attached: true,
        },
    ))
    .await;
    assert_eq!(r.state, OperationState::Rejected);
    assert!(
        !mock
            .methods
            .lock()
            .unwrap()
            .iter()
            .any(|m| m == "Async.PBD.plug")
    );
    c.close().await;
}

#[tokio::test]
async fn storage_local_nonformatting_introduction_and_single_host_attachment_are_separate() {
    for kind in [LocalStorageKind::Ext, LocalStorageKind::Lvm] {
        let (mock, data) = fixture(Fault::None).await;
        local_drivers(&data);
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connection(&mock, &backend, false).await;
        let location = StorageLocation::Local {
            kind,
            device: DEVICE.into(),
        };
        let r = finished(&submit(
            &c,
            pool(),
            StorageOperation::Introduce {
                uuid: SrId::parse(CREATED).unwrap(),
                name: "Existing local".into(),
                description: String::new(),
                location: location.clone(),
            },
        ))
        .await;
        assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
        assert_eq!(data.lock().unwrap()["SR-created"]["shared"], false);
        assert_eq!(data.lock().unwrap()["SR-created"]["PBDs"], json!([]));
        // Exercise the existing observed SR separately: introducing does not format or attach.
        {
            let mut m = data.lock().unwrap();
            m.remove("PBD");
            let sr = m.get_mut("SR").unwrap();
            sr["type"] = json!(kind.kind());
            sr["shared"] = json!(false);
            sr["PBDs"] = json!([]);
        }
        let r = finished(&submit(
            &c,
            sr(),
            StorageOperation::CreateAttachment {
                host: HostId::parse(HOST).unwrap(),
                location,
            },
        ))
        .await;
        assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
        assert_eq!(
            data.lock().unwrap()["PBD-created"]["currently_attached"],
            false
        );
        let methods = mock.methods.lock().unwrap().clone();
        assert_eq!(
            methods
                .iter()
                .filter(|m| m.as_str() == "Async.SR.introduce")
                .count(),
            1
        );
        assert_eq!(
            methods
                .iter()
                .filter(|m| m.as_str() == "Async.PBD.create")
                .count(),
            1
        );
        assert!(
            !methods
                .iter()
                .any(|m| m == "Async.SR.create" || m == "Async.PBD.plug")
        );
        c.close().await;
    }
}
