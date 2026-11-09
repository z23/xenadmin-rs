use super::*;
use xenadmin_core::{
    MemoryLimits, OperationId, OperationState, VmCommand, VmSettingEdit, VmSettingsPatch,
};
fn record(value: &mut Value, running: bool) {
    value["power_state"] = json!(if running { "Running" } else { "Halted" });
    value["allowed_operations"] = json!([
        "changing_VCPUs_live",
        "changing_dynamic_range",
        "changing_shadow_memory",
        "changing_shadow_memory_live"
    ]);
    value["VCPUs_max"] = json!("8");
    value["VCPUs_at_startup"] = json!("2");
    value["memory_static_min"] = json!("1024");
    value["memory_static_max"] = json!("8192");
    value["memory_dynamic_min"] = json!("2048");
    value["memory_dynamic_max"] = json!("4096");
    value["platform"] = json!({"firmware":"uefi","unrelated":"keep"});
    value["VCPUs_params"] = json!({"cap":"0"});
    value["HVM_boot_policy"] = json!("BIOS order");
    value["HVM_shadow_multiplier"] = json!(1.0);
}
async fn connection(server: &Mock, backend: &Backend) -> xenadmin_core::Connection {
    let c = backend.connect(
        ConnectionOptions::new(server.endpoint.clone()),
        Credentials::new("root".into(), "fixture".into()),
    );
    c.wait_ready(deadline(3)).await.unwrap();
    c
}
fn submit(c: &xenadmin_core::Connection, edits: Vec<VmSettingEdit>) -> xenadmin_core::Operation {
    let snap = c.latest();
    let vm = snap.inventory().unwrap().guests().next().unwrap().id;
    c.submit(
        OperationId::new(),
        VmCommand::Configure {
            vm: snap.observe_vm(vm).unwrap(),
            patch: VmSettingsPatch::new(edits).unwrap(),
        },
    )
    .unwrap()
}
fn limits() -> MemoryLimits {
    MemoryLimits {
        static_min: 1024,
        static_max: 8192,
        dynamic_min: 2048,
        dynamic_max: 4096,
    }
}
#[tokio::test]
async fn live_cpu_memory_and_weight_apply_reviewed_values_preserving_unrelated_settings() {
    let (server, data) = settings_mock(false, false, None).await;
    record(&mut data.lock().unwrap(), true);
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connection(&server, &backend).await;
    let operation = submit(
        &c,
        vec![
            VmSettingEdit::LiveCpus {
                previous: 2,
                max: 8,
                value: 4,
            },
            VmSettingEdit::DynamicMemory {
                previous: limits(),
                min: 3072,
                max: 6144,
            },
            VmSettingEdit::CpuWeight {
                previous: None,
                weight: 512,
            },
        ],
    );
    assert_eq!(
        finish(&operation).await,
        OperationState::Succeeded,
        "{}",
        operation.latest().detail
    );
    let v = data.lock().unwrap();
    assert_eq!(v["VCPUs_at_startup"], "4");
    assert_eq!(v["VCPUs_max"], "8");
    assert_eq!(v["memory_static_max"], "8192");
    assert_eq!(v["VCPUs_params"], json!({"cap":"0","weight":"512"}));
}
#[tokio::test]
async fn topology_and_shadow_apply_without_overwriting_other_platform_keys() {
    let (server, data) = settings_mock(false, false, None).await;
    record(&mut data.lock().unwrap(), false);
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connection(&server, &backend).await;
    let operation = submit(
        &c,
        vec![
            VmSettingEdit::CpuTopology {
                previous: None,
                cores_per_socket: 4,
            },
            VmSettingEdit::ShadowMultiplier {
                previous: "1".into(),
                value: "2.50".into(),
                live: false,
            },
        ],
    );
    assert_eq!(
        finish(&operation).await,
        OperationState::Succeeded,
        "{}",
        operation.latest().detail
    );
    assert_eq!(
        data.lock().unwrap()["platform"],
        json!({"firmware":"uefi","unrelated":"keep","cores-per-socket":"4"})
    );
    assert_eq!(data.lock().unwrap()["HVM_shadow_multiplier"], 2.5);
}
#[tokio::test]
async fn fresh_static_limits_or_live_capability_changes_refuse_cpu_memory_writes() {
    for change in ["cpu", "memory"] {
        let (server, data) = settings_mock(false, false, None).await;
        record(&mut data.lock().unwrap(), true);
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connection(&server, &backend).await;
        let edit = if change == "cpu" {
            data.lock().unwrap()["allowed_operations"] = json!([]);
            VmSettingEdit::LiveCpus {
                previous: 2,
                max: 8,
                value: 4,
            }
        } else {
            data.lock().unwrap()["memory_static_max"] = json!("4096");
            VmSettingEdit::DynamicMemory {
                previous: limits(),
                min: 3072,
                max: 6144,
            }
        };
        assert_eq!(
            finish(&submit(&c, vec![edit])).await,
            OperationState::Rejected
        );
        assert!(
            !server
                .methods
                .lock()
                .unwrap()
                .iter()
                .any(|m| m == "VM.set_VCPUs_number_live" || m == "VM.set_memory_dynamic_range")
        );
    }
}
#[tokio::test]
async fn lost_live_cpu_reply_rechecks_saved_value_without_repeating_hotplug() {
    let (server, data) = settings_mock(false, true, None).await;
    record(&mut data.lock().unwrap(), true);
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connection(&server, &backend).await;
    let operation = submit(
        &c,
        vec![VmSettingEdit::LiveCpus {
            previous: 2,
            max: 8,
            value: 4,
        }],
    );
    assert_eq!(finish(&operation).await, OperationState::OutcomeUnknown);
    let id = operation.latest().id;
    c.control_operation(id, xenadmin_core::OperationControl::Recheck)
        .unwrap();
    tokio::time::timeout(deadline(3), async {
        loop {
            if c.operation_history()
                .iter()
                .any(|r| r.id == id && r.state == OperationState::Succeeded)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        server
            .methods
            .lock()
            .unwrap()
            .iter()
            .filter(|m| *m == "VM.set_VCPUs_number_live")
            .count(),
        1
    );
}
#[test]
fn live_cpu_reduction_and_conflicting_memory_intents_are_invalid() {
    assert!(
        VmSettingsPatch::new(vec![VmSettingEdit::LiveCpus {
            previous: 4,
            max: 8,
            value: 2
        }])
        .is_err()
    );
    assert!(
        VmSettingsPatch::new(vec![
            VmSettingEdit::DynamicMemory {
                previous: limits(),
                min: 2048,
                max: 4096
            },
            VmSettingEdit::Memory {
                previous: limits(),
                value: limits()
            }
        ])
        .is_err()
    );
    assert!(
        VmSettingsPatch::new(vec![VmSettingEdit::ShadowMultiplier {
            previous: "1".into(),
            value: "NaN".into(),
            live: true
        }])
        .is_err()
    );
}

#[tokio::test]
async fn modern_hvm_shadow_does_not_require_the_deprecated_boot_policy() {
    let (server, data) = settings_mock(false, false, None).await;
    record(&mut data.lock().unwrap(), true);
    data.lock().unwrap()["domain_type"] = json!("hvm");
    data.lock().unwrap()["HVM_boot_policy"] = json!("");
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connection(&server, &backend).await;
    let op = submit(
        &c,
        vec![VmSettingEdit::ShadowMultiplier {
            previous: "1".into(),
            value: "2".into(),
            live: true,
        }],
    );
    assert_eq!(
        finish(&op).await,
        OperationState::Succeeded,
        "{}",
        op.latest().detail
    );
    assert!(
        VmSettingsPatch::new(vec![VmSettingEdit::ShadowMultiplier {
            previous: "1".into(),
            value: format!("1.{}", "0".repeat(4096)),
            live: true
        }])
        .is_err()
    );
}

fn home_host() -> xenadmin_core::HostId {
    xenadmin_core::HostId::parse("22222222-2222-2222-2222-222222222222").unwrap()
}
#[tokio::test]
async fn home_host_set_and_clear_preserves_guest_state_and_uses_uuid_identity() {
    let (server, data) = settings_mock(false, false, None).await;
    record(&mut data.lock().unwrap(), true);
    data.lock().unwrap()["affinity"] = json!("OpaqueRef:NULL");
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connection(&server, &backend).await;
    let vm = c.latest().inventory().unwrap().guests().next().unwrap().id;
    assert!(c.latest().inventory().unwrap().details.vms[&vm].affinity_known);
    for (previous, value) in [(None, Some(home_host())), (Some(home_host()), None)] {
        let op = submit(&c, vec![VmSettingEdit::HomeHost { previous, value }]);
        assert_eq!(
            finish(&op).await,
            OperationState::Succeeded,
            "{}",
            op.latest().detail
        );
        assert!(
            !serde_json::to_string(&*op.latest())
                .unwrap()
                .contains("OpaqueRef")
        );
    }
    assert_eq!(data.lock().unwrap()["affinity"], "OpaqueRef:NULL");
    assert_eq!(data.lock().unwrap()["power_state"], "Running");
    let calls = server.methods.lock().unwrap();
    assert_eq!(calls.iter().filter(|m| *m == "VM.set_affinity").count(), 2);
    assert!(
        !calls
            .iter()
            .any(|m| m.contains("migrate") || m.starts_with("Async.VM"))
    );
}
#[tokio::test]
async fn stale_unknown_missing_or_suspended_home_host_changes_do_not_write() {
    for case in 0..4 {
        let (server, data) = settings_mock(false, false, None).await;
        record(&mut data.lock().unwrap(), false);
        data.lock().unwrap()["affinity"] = json!("OpaqueRef:NULL");
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connection(&server, &backend).await;
        match case {
            0 => data.lock().unwrap()["affinity"] = json!("OpaqueRef:host"),
            1 => data.lock().unwrap()["affinity"] = Value::Null,
            2 => data.lock().unwrap()["fixture_missing_host"] = json!(true),
            _ => data.lock().unwrap()["power_state"] = json!("Suspended"),
        }
        let op = submit(
            &c,
            vec![VmSettingEdit::HomeHost {
                previous: None,
                value: Some(home_host()),
            }],
        );
        let state = finish(&op).await;
        assert!(
            matches!(state, OperationState::Rejected | OperationState::Failed),
            "case {case}: {}",
            op.latest().detail
        );
        assert!(
            !server
                .methods
                .lock()
                .unwrap()
                .iter()
                .any(|m| m == "VM.set_affinity")
        );
    }
}
#[tokio::test]
async fn lost_home_host_reply_reconciles_uuid_without_repeating_setter() {
    let (server, data) = settings_mock(false, true, None).await;
    record(&mut data.lock().unwrap(), false);
    data.lock().unwrap()["affinity"] = json!("OpaqueRef:NULL");
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connection(&server, &backend).await;
    let op = submit(
        &c,
        vec![VmSettingEdit::HomeHost {
            previous: None,
            value: Some(home_host()),
        }],
    );
    assert_eq!(finish(&op).await, OperationState::OutcomeUnknown);
    let id = op.latest().id;
    c.control_operation(id, xenadmin_core::OperationControl::Recheck)
        .unwrap();
    tokio::time::timeout(deadline(3), async {
        loop {
            if c.operation_history()
                .iter()
                .any(|r| r.id == id && r.state == OperationState::Succeeded)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        server
            .methods
            .lock()
            .unwrap()
            .iter()
            .filter(|m| *m == "VM.set_affinity")
            .count(),
        1
    );
}
