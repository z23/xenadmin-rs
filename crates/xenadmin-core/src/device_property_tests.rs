use super::*;
use crate::{DeviceProperty as Change, DeviceQos, DiskAccess};
fn vbd() -> VbdId {
    uuid(5).into()
}
fn vdi() -> VdiId {
    uuid(4).into()
}
fn vif() -> VifId {
    uuid(9).into()
}
fn config() -> DeviceQos {
    DeviceQos {
        algorithm: "".into(),
        parameters: BTreeMap::from([("vendor-extension".into(), "preserved".into())]),
    }
}
fn enrich(records: &Records) {
    let mut r = records.lock().unwrap();
    let disk = r.get_mut("OpaqueRef:VDI:disk").unwrap();
    disk["name_description"] = json!("Before");
    disk["cbt_enabled"] = json!(true);
    disk["allowed_operations"] = json!(["destroy", "disable_cbt"]);
    for id in ["OpaqueRef:VBD:disk", "OpaqueRef:VIF:nic"] {
        let device = r.get_mut(id).unwrap();
        device["bootable"] = json!(false);
        device["qos_algorithm_type"] = json!("");
        device["qos_algorithm_params"] = json!(config().parameters);
        device["qos_supported_algorithms"] = json!(["ionice", "ratelimit"]);
    }
}
fn command(change: Change) -> DeviceOperation {
    DeviceOperation::Property { change }
}
fn metadata() -> Change {
    Change::DiskMetadata {
        vbd: vbd(),
        vdi: vdi(),
        previous_name: "Existing disk".into(),
        previous_description: "Before".into(),
        name: "Data".into(),
        description: "After".into(),
    }
}
fn writes(calls: &Calls) -> usize {
    calls
        .lock()
        .unwrap()
        .iter()
        .filter(|r| {
            r["method"].as_str().unwrap().contains(".set_") || r["method"] == "VDI.disable_cbt"
        })
        .count()
}
#[tokio::test]
async fn disk_properties_keep_identity_and_preserve_unrelated_qos_parameters() {
    let (fixture, records, calls) = fixture(Fault::None).await;
    enrich(&records);
    let dir = tempfile::tempdir().unwrap();
    for change in [
        metadata(),
        Change::DiskAccess {
            vbd: vbd(),
            vdi: vdi(),
            previous: DiskAccess::ReadWrite,
            value: DiskAccess::ReadOnly,
        },
        Change::DiskPosition {
            vbd: vbd(),
            vdi: vdi(),
            previous: "0".into(),
            value: "3".into(),
        },
        Change::Bootable {
            vbd: vbd(),
            vdi: vdi(),
            previous: false,
            value: true,
        },
        Change::DiskPriority {
            vbd: vbd(),
            vdi: vdi(),
            previous: config(),
            priority: Some(7),
        },
        Change::DisableCbt {
            vbd: vbd(),
            vdi: vdi(),
        },
    ] {
        let r = execute(&fixture, dir.path(), command(change)).await;
        assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
        assert!(!serde_json::to_string(&r).unwrap().contains("OpaqueRef"));
    }
    let r = records.lock().unwrap();
    assert_eq!(
        r["OpaqueRef:VBD:disk"]["qos_algorithm_params"]["vendor-extension"],
        "preserved"
    );
    assert_eq!(r["OpaqueRef:VBD:disk"]["qos_algorithm_type"], "ionice");
    assert_eq!(r["OpaqueRef:VDI:disk"]["cbt_enabled"], false);
    assert_eq!(writes(&calls), 8);
}
#[tokio::test]
async fn nic_rate_limits_use_kilobytes_preserve_maps_and_disable_without_replacing_nic() {
    let (fixture, records, calls) = fixture(Fault::None).await;
    enrich(&records);
    let dir = tempfile::tempdir().unwrap();
    let r = execute(
        &fixture,
        dir.path(),
        command(Change::NicRate {
            vif: vif(),
            previous: config(),
            kilobytes_per_second: Some(12500),
        }),
    )
    .await;
    assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
    let previous =
        crate::DeviceQos::from_record(&records.lock().unwrap()["OpaqueRef:VIF:nic"]).unwrap();
    assert_eq!(previous.parameters["kbps"], "12500");
    assert_eq!(previous.parameters["vendor-extension"], "preserved");
    let r = execute(
        &fixture,
        dir.path(),
        command(Change::NicRate {
            vif: vif(),
            previous,
            kilobytes_per_second: None,
        }),
    )
    .await;
    assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
    assert_eq!(writes(&calls), 3);
    assert!(records.lock().unwrap().contains_key("OpaqueRef:VIF:nic"));
}
#[tokio::test]
async fn metadata_can_change_while_running_but_device_configuration_requires_halted() {
    let (fixture, records, calls) = fixture(Fault::None).await;
    enrich(&records);
    records
        .lock()
        .unwrap()
        .get_mut("OpaqueRef:VM:guest")
        .unwrap()["power_state"] = json!("Running");
    let dir = tempfile::tempdir().unwrap();
    let r = execute(&fixture, dir.path(), command(metadata())).await;
    assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
    let r = execute(
        &fixture,
        dir.path(),
        command(Change::Bootable {
            vbd: vbd(),
            vdi: vdi(),
            previous: false,
            value: true,
        }),
    )
    .await;
    assert_eq!(r.state, OperationState::Rejected, "{}", r.detail);
    assert_eq!(writes(&calls), 2);
}
#[tokio::test]
async fn stale_shared_attached_unsupported_and_occupied_devices_are_refused_before_write() {
    for case in 0..5 {
        let (fixture, records, calls) = fixture(Fault::None).await;
        enrich(&records);
        let mut change = Change::DiskPosition {
            vbd: vbd(),
            vdi: vdi(),
            previous: "0".into(),
            value: "3".into(),
        };
        {
            let mut r = records.lock().unwrap();
            match case {
                0 => r.get_mut("OpaqueRef:VBD:disk").unwrap()["userdevice"] = json!("9"),
                1 => r.get_mut("OpaqueRef:VDI:disk").unwrap()["sharable"] = json!(true),
                2 => r.get_mut("OpaqueRef:VBD:disk").unwrap()["currently_attached"] = json!(true),
                3 => {
                    r.get_mut("OpaqueRef:VBD:disk").unwrap()["qos_supported_algorithms"] =
                        json!([]);
                    change = Change::DiskPriority {
                        vbd: vbd(),
                        vdi: vdi(),
                        previous: config(),
                        priority: Some(4),
                    }
                }
                _ => {
                    let mut other = r["OpaqueRef:VBD:disk"].clone();
                    other["uuid"] = json!(uuid(42));
                    other["userdevice"] = json!("3");
                    r.insert("OpaqueRef:VBD:other".into(), other);
                }
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let r = execute(&fixture, dir.path(), command(change)).await;
        assert_eq!(
            r.state,
            OperationState::Rejected,
            "case {case}: {}",
            r.detail
        );
        assert_eq!(writes(&calls), 0);
    }
}
#[tokio::test]
async fn lost_metadata_reply_rechecks_only_dispatched_field_and_never_replays_description() {
    let (fixture, records, calls) = fixture(Fault::LostProperty).await;
    enrich(&records);
    let dir = tempfile::tempdir().unwrap();
    let r = execute(&fixture, dir.path(), command(metadata())).await;
    assert_eq!(r.state, OperationState::OutcomeUnknown);
    let r = recheck(&fixture, dir.path(), r).await;
    assert_eq!(r.state, OperationState::PartiallyCompleted, "{}", r.detail);
    assert_eq!(writes(&calls), 1);
    assert_eq!(
        records.lock().unwrap()["OpaqueRef:VDI:disk"]["name_description"],
        "Before"
    );
}
#[tokio::test]
async fn failed_second_qos_setter_retains_first_change_as_unknown_without_enabling_algorithm() {
    let (fixture, records, calls) = fixture(Fault::RefuseSecondProperty).await;
    enrich(&records);
    let dir = tempfile::tempdir().unwrap();
    let r = execute(
        &fixture,
        dir.path(),
        command(Change::NicRate {
            vif: vif(),
            previous: config(),
            kilobytes_per_second: Some(10),
        }),
    )
    .await;
    assert_eq!(r.state, OperationState::OutcomeUnknown);
    assert_eq!(
        records.lock().unwrap()["OpaqueRef:VIF:nic"]["qos_algorithm_type"],
        ""
    );
    assert_eq!(
        records.lock().unwrap()["OpaqueRef:VIF:nic"]["qos_algorithm_params"]["kbps"],
        "10"
    );
    assert_eq!(writes(&calls), 2);
}
#[tokio::test]
async fn lost_cbt_reply_reconciles_the_same_disk_without_repeating_disable() {
    let (fixture, records, calls) = fixture(Fault::LostProperty).await;
    enrich(&records);
    let dir = tempfile::tempdir().unwrap();
    let r = execute(
        &fixture,
        dir.path(),
        command(Change::DisableCbt {
            vbd: vbd(),
            vdi: vdi(),
        }),
    )
    .await;
    assert_eq!(r.state, OperationState::OutcomeUnknown);
    let r = recheck(&fixture, dir.path(), r).await;
    assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
    assert_eq!(writes(&calls), 1);
}

#[tokio::test]
async fn a_rate_parameter_change_cannot_succeed_when_the_algorithm_was_disabled_concurrently() {
    let (fixture, records, calls) = fixture(Fault::ChangePropertyAlgorithm).await;
    enrich(&records);
    let previous = DeviceQos {
        algorithm: "ratelimit".into(),
        parameters: BTreeMap::from([("kbps".into(), "100".into())]),
    };
    {
        let mut r = records.lock().unwrap();
        r.get_mut("OpaqueRef:VIF:nic").unwrap()["qos_algorithm_type"] = json!(previous.algorithm);
        r.get_mut("OpaqueRef:VIF:nic").unwrap()["qos_algorithm_params"] =
            json!(previous.parameters);
    }
    let dir = tempfile::tempdir().unwrap();
    let r = execute(
        &fixture,
        dir.path(),
        command(Change::NicRate {
            vif: vif(),
            previous,
            kilobytes_per_second: Some(200),
        }),
    )
    .await;
    assert_eq!(r.state, OperationState::OutcomeUnknown, "{}", r.detail);
    let r = recheck(&fixture, dir.path(), r).await;
    assert_eq!(r.state, OperationState::OutcomeUnknown, "{}", r.detail);
    assert_eq!(writes(&calls), 1);
}
