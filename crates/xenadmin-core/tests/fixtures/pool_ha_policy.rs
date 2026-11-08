use super::*;
use xenadmin_core::VmStartupField;
fn startup(field: VmStartupField) -> PoolOperation {
    PoolOperation::SetVmStartup {
        vm: VmId::parse(VM).unwrap(),
        appliance: None,
        field,
        expected: 0,
        value: 2,
    }
}
#[tokio::test]
async fn ha_startup_and_pool_policies_preserve_unrelated_settings() {
    let (mock, records) = fixture(Fault::None).await;
    let d = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(d.path().into()).unwrap();
    let c = connect(&mock, &b, false).await;
    let inventory = c.latest();
    let a = &inventory
        .inventory()
        .unwrap()
        .details
        .pool
        .as_ref()
        .unwrap()
        .administration;
    assert!(a.ha_timeout_known);
    assert!(a.ha_timeout_override.is_none());
    for op in [
        startup(VmStartupField::Order),
        startup(VmStartupField::StartDelay),
        startup(VmStartupField::ShutdownDelay),
        PoolOperation::SetHaAllowOvercommit {
            expected: false,
            enabled: true,
        },
        PoolOperation::SetHaTimeout {
            expected: None,
            seconds: Some(90),
        },
    ] {
        let r = finished(&submit(&c, op)).await;
        assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
    }
    assert_eq!(
        records.lock().unwrap()["pool"]["other_config"],
        json!({"keep":"preserve","default_ha_timeout":"90"})
    );
    let r = finished(&submit(
        &c,
        PoolOperation::SetHaTimeout {
            expected: Some("90".into()),
            seconds: None,
        },
    ))
    .await;
    assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
    assert_eq!(
        records.lock().unwrap()["pool"]["other_config"],
        json!({"keep":"preserve"})
    );
    assert_eq!(mutations(&mock), 6);
    let records = records.lock().unwrap();
    for field in ["order", "start_delay", "shutdown_delay"] {
        assert_eq!(records["vm"][field], "2");
    }
    assert_eq!(records["vm"]["power_state"], "Halted");
}
#[tokio::test]
async fn ha_policy_refuses_active_timeout_changed_fields_and_vapp_membership() {
    for case in 0..8 {
        let (mock, records) = fixture(Fault::None).await;
        let op = if case < 4 {
            PoolOperation::SetHaTimeout {
                expected: None,
                seconds: Some(90),
            }
        } else {
            startup(VmStartupField::Order)
        };
        {
            let mut r = records.lock().unwrap();
            match case {
                0 => r.get_mut("pool").unwrap()["ha_enabled"] = json!(true),
                1 => r.get_mut("pool").unwrap()["ha_configuration"] = json!({"timeout":"60"}),
                2 => {
                    r.get_mut("pool").unwrap()["other_config"] = json!({"default_ha_timeout":"120"})
                }
                3 => {
                    r.get_mut("pool")
                        .unwrap()
                        .as_object_mut()
                        .unwrap()
                        .remove("ha_configuration");
                }
                4 => r.get_mut("vm").unwrap()["appliance"] = json!("OpaqueRef:another-vapp"),
                5 => r.get_mut("vm").unwrap()["order"] = json!("4"),
                6 => {
                    r.get_mut("vm").unwrap()["current_operations"] =
                        json!({"OpaqueRef:task":"start"})
                }
                _ => r.get_mut("vm").unwrap()["is_a_template"] = json!(true),
            }
        }
        let d = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(d.path().into()).unwrap();
        let c = connect(&mock, &b, false).await;
        let r = finished(&submit(&c, op)).await;
        assert_eq!(
            r.state,
            OperationState::Rejected,
            "case {case}: {}",
            r.detail
        );
        assert_eq!(mutations(&mock), 0);
    }
}
#[tokio::test]
async fn ha_policy_rechecks_positive_receipts_without_replaying_lost_replies() {
    for fault in [Fault::DropReply, Fault::VerifyFailure] {
        let (mock, _) = fixture(fault).await;
        let d = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(d.path().into()).unwrap();
        let c = connect(&mock, &b, false).await;
        let op = submit(&c, startup(VmStartupField::Order));
        let r = finished(&op).await;
        assert_eq!(r.state, OperationState::OutcomeUnknown, "{}", r.detail);
        c.control_resource(r.id, OperationControl::Recheck).unwrap();
        let checked = tokio::time::timeout(Duration::from_secs(4), async {
            loop {
                let row = c
                    .resource_history()
                    .iter()
                    .find(|v| v.id == r.id)
                    .cloned()
                    .unwrap();
                if (fault == Fault::DropReply
                    && row.state == OperationState::OutcomeUnknown
                    && row.detail != r.detail)
                    || (fault == Fault::VerifyFailure && row.state == OperationState::Succeeded)
                {
                    return row;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            checked.state,
            if fault == Fault::DropReply {
                OperationState::OutcomeUnknown
            } else {
                OperationState::Succeeded
            },
            "{}",
            checked.detail
        );
        assert_eq!(mutations(&mock), 1);
    }
}
#[tokio::test]
async fn ha_policy_readonly_and_invalid_ranges_are_rejected_without_writes() {
    let (mock, _) = fixture(Fault::None).await;
    let d = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(d.path().into()).unwrap();
    let c = connect(&mock, &b, true).await;
    assert!(
        c.submit_resource(
            OperationId::new(),
            ResourceCommand::Pool {
                target: c.latest().observe_resource(pool()).unwrap(),
                operation: startup(VmStartupField::Order)
            }
        )
        .is_err()
    );
    let writable = tempfile::tempdir().unwrap();
    let writable_backend = Backend::with_state_directory(writable.path().into()).unwrap();
    let c = connect(&mock, &writable_backend, false).await;
    for operation in [
        PoolOperation::SetHaTimeout {
            expected: None,
            seconds: Some(9),
        },
        PoolOperation::SetVmStartup {
            vm: VmId::parse(VM).unwrap(),
            appliance: None,
            field: VmStartupField::StartDelay,
            expected: 0,
            value: 86_401,
        },
    ] {
        assert!(
            c.submit_resource(
                OperationId::new(),
                ResourceCommand::Pool {
                    target: c.latest().observe_resource(pool()).unwrap(),
                    operation
                }
            )
            .is_err()
        );
    }
    assert_eq!(mutations(&mock), 0);
}
