use super::*;
use std::collections::BTreeMap;
use xenadmin_core::{
    HostId, HostOperation, HostPowerOnMode, MemoryLimits, OperationControl, OperationId,
    OperationState, PoolId, ResourceCommand, ResourceOperation, ResourceOperationRecord,
    ResourceTarget, VmId,
};
const POOL: &str = "11111111-1111-1111-1111-111111111111";
const HOST: &str = "22222222-2222-2222-2222-222222222222";
const GUEST: &str = "33333333-3333-3333-3333-333333333333";
const DOM0: &str = "44444444-4444-4444-4444-444444444444";
const OTHER: &str = "55555555-5555-5555-5555-555555555555";
const METRICS: &str = "66666666-6666-6666-6666-666666666666";
type Records = Arc<Mutex<BTreeMap<String, Value>>>;
fn host() -> ResourceTarget {
    ResourceTarget::Host(HostId::parse(HOST).unwrap())
}
fn memory() -> MemoryLimits {
    MemoryLimits {
        static_min: 1048576,
        dynamic_min: 2147483648,
        dynamic_max: 2147483648,
        static_max: 2147483648,
    }
}
fn op_memory() -> HostOperation {
    HostOperation::SetDom0Memory {
        control_domain: VmId::parse(DOM0).unwrap(),
        expected: memory(),
        memory: 3221225472,
    }
}
async fn fixture(drop_method: &'static str, fail_method: &'static str) -> (Mock, Records) {
    let records: Records = Arc::new(Mutex::new(BTreeMap::from([
        (
            "pool".into(),
            json!({"uuid":POOL,"name_label":"Host fixture pool","master":"OpaqueRef:host-other","ha_enabled":false,"other_config":{"preserve":"yes"}}),
        ),
        (
            "host".into(),
            json!({"uuid":HOST,"name_label":"Target host","address":"192.0.2.10","enabled":true,"metrics":"OpaqueRef:host_metrics","control_domain":"OpaqueRef:VM-dom0","resident_VMs":["OpaqueRef:VM-guest","OpaqueRef:VM-dom0"],"PBDs":[],"current_operations":{},"allowed_operations":["enable","evacuate","reboot","shutdown","power_on"],"other_config":{"boot_time":"100","agent_start_time":"110","preserve":"yes"},"logging":{"preserve":"yes"},"power_on_mode":"","power_on_config":{},"multipathing":false}),
        ),
        (
            "host-other".into(),
            json!({"uuid":OTHER,"name_label":"Destination host","enabled":true,"metrics":"OpaqueRef:host_metrics-other"}),
        ),
        (
            "host_metrics".into(),
            json!({"uuid":METRICS,"live":true,"memory_total":"17179869184","memory_free":"8589934592"}),
        ),
        (
            "host_metrics-other".into(),
            json!({"uuid":"77777777-7777-7777-7777-777777777777","live":true}),
        ),
        (
            "VM-guest".into(),
            json!({"uuid":GUEST,"name_label":"Movable fixture VM","is_control_domain":false,"is_a_template":false,"is_a_snapshot":false,"power_state":"Running","resident_on":"OpaqueRef:host","current_operations":{},"allowed_operations":["clean_shutdown"],"VCPUs_at_startup":"2","memory_static_max":"2147483648"}),
        ),
        (
            "VM-dom0".into(),
            json!({"uuid":DOM0,"name_label":"Control domain","is_control_domain":true,"is_a_template":false,"is_a_snapshot":false,"power_state":"Running","resident_on":"OpaqueRef:host","current_operations":{},"VCPUs_at_startup":"4","memory_static_min":"1048576","memory_static_max":"2147483648","memory_dynamic_min":"2147483648","memory_dynamic_max":"2147483648"}),
        ),
    ])));
    let source = records.clone();
    let callback = move |request: Value| {
        let source = source.clone();
        async move {
            let method = request["method"].as_str().unwrap();
            let mut records = source.lock().unwrap();
            if method == "event.from" && request["params"][2] == "" {
                let events=records.iter().filter(|(k,_)|!k.starts_with('_')).map(|(key,record)|json!({"class":key.split('-').next().unwrap(),"operation":"add","ref":format!("OpaqueRef:{key}"),"snapshot":record})).collect::<Vec<_>>();
                return Some(RpcReply::Json(
                    json!({"result":{"token":"initial","events":events}}),
                ));
            }
            if let Some(class) = method.strip_suffix(".get_all_records")
                && ["host", "pool", "host_metrics", "VM", "task", "PBD"].contains(&class)
            {
                let values = records
                    .iter()
                    .filter(|(k, _)| k.split('-').next() == Some(class))
                    .map(|(k, v)| (format!("OpaqueRef:{k}"), v.clone()))
                    .collect::<serde_json::Map<_, _>>();
                return Some(RpcReply::Json(json!({"result":values})));
            }
            if let Some(class) = method.strip_suffix(".get_by_uuid")
                && ["host", "pool", "VM"].contains(&class)
            {
                let result = records.iter().find(|(k, v)| {
                    k.split('-').next() == Some(class) && v["uuid"] == request["params"][1]
                });
                return Some(RpcReply::Json(match result {
                    Some((k, _)) => json!({"result":format!("OpaqueRef:{k}")}),
                    None => json!({"error":{"code":1,"message":"UUID_INVALID","data":[]}}),
                }));
            }
            if method == "task.get_record" {
                if records.get("_pending") == Some(&json!(true)) {
                    return Some(RpcReply::Json(
                        json!({"result":{"status":"pending","progress":0.5}}),
                    ));
                }
                return Some(RpcReply::Json(records.get("_task_error").map_or_else(
                    || json!({"result":{"status":"success","progress":1.0}}),
                    |code| json!({"error":{"code":1,"message":code,"data":[]}}),
                )));
            }
            if let Some(class) = method.strip_suffix(".get_record")
                && ["host", "pool", "host_metrics", "VM", "PBD"].contains(&class)
            {
                let key = request["params"][1]
                    .as_str()
                    .unwrap()
                    .trim_start_matches("OpaqueRef:");
                return Some(RpcReply::Json(records.get(key).map_or_else(
                    || json!({"error":{"code":1,"message":"HANDLE_INVALID","data":[]}}),
                    |v| json!({"result":v}),
                )));
            }
            if method == "host.get_all" {
                return Some(RpcReply::Json(
                    json!({"result":["OpaqueRef:host","OpaqueRef:host-other"]}),
                ));
            }
            if method == "VM.get_possible_hosts" {
                return Some(RpcReply::Json(
                    json!({"result":records.get("_destinations").cloned().unwrap_or(json!(["OpaqueRef:host-other"]))}),
                ));
            }
            if method == "host.get_vms_which_prevent_evacuation" {
                return Some(RpcReply::Json(
                    json!({"result":records.get("_blockers").cloned().unwrap_or(json!({}))}),
                ));
            }
            if method == "host.assert_can_evacuate" {
                return Some(RpcReply::Json(json!({"result":null})));
            }
            if method == fail_method {
                return Some(RpcReply::Json(
                    json!({"error":{"code":1,"message":"OPERATION_NOT_ALLOWED","data":["private fixture detail"]}}),
                ));
            }
            let mutation = match method {
                "Async.host.disable" => {
                    records.get_mut("host").unwrap()["enabled"] = json!(false);
                    true
                }
                "Async.host.enable" => {
                    records.get_mut("host").unwrap()["enabled"] = json!(true);
                    true
                }
                "Async.host.evacuate" => {
                    records.get_mut("host").unwrap()["resident_VMs"] = json!(["OpaqueRef:VM-dom0"]);
                    records.get_mut("VM-guest").unwrap()["resident_on"] =
                        json!("OpaqueRef:host-other");
                    true
                }
                "Async.host.reboot" => {
                    records.get_mut("host").unwrap()["other_config"]["boot_time"] = json!("200");
                    true
                }
                "Async.host.restart_agent" => {
                    records.get_mut("host").unwrap()["other_config"]["agent_start_time"] =
                        json!("210");
                    true
                }
                "Async.host.shutdown" => {
                    records.get_mut("host_metrics").unwrap()["live"] = json!(false);
                    true
                }
                "Async.host.power_on" => {
                    records.get_mut("host_metrics").unwrap()["live"] = json!(true);
                    true
                }
                "Async.host.destroy" => {
                    records.remove("host");
                    true
                }
                "host.set_other_config" => {
                    records.get_mut("host").unwrap()["other_config"] = request["params"][2].clone();
                    true
                }
                "pool.set_other_config" => {
                    records.get_mut("pool").unwrap()["other_config"] = request["params"][2].clone();
                    true
                }
                "host.set_power_on_mode" => {
                    records.get_mut("host").unwrap()["power_on_mode"] =
                        request["params"][2].clone();
                    records.get_mut("host").unwrap()["power_on_config"] =
                        request["params"][3].clone();
                    true
                }
                "VM.set_memory" => {
                    assert_eq!(request["params"][1], "OpaqueRef:VM-dom0");
                    for key in [
                        "memory_static_max",
                        "memory_dynamic_min",
                        "memory_dynamic_max",
                    ] {
                        records.get_mut("VM-dom0").unwrap()[key] = request["params"][2].clone();
                    }
                    true
                }
                "host.set_logging" => {
                    records.get_mut("host").unwrap()["logging"] = request["params"][2].clone();
                    true
                }
                "host.syslog_reconfigure" => true,
                "host.set_multipathing" => {
                    records.get_mut("host").unwrap()["multipathing"] = request["params"][2].clone();
                    true
                }
                _ => false,
            };
            if mutation {
                return Some(if method == drop_method {
                    RpcReply::Drop
                } else {
                    RpcReply::Json(
                        json!({"result":if method.starts_with("Async."){json!(format!("OpaqueRef:task-{}",method))}else{Value::Null}}),
                    )
                });
            }
            None
        }
    };
    (mock_overridden(override_rpc(callback)).await, records)
}
async fn connect(mock: &Mock, backend: &Backend, read_only: bool) -> xenadmin_core::Connection {
    let mut options = ConnectionOptions::new(mock.endpoint.clone());
    options.read_only = read_only;
    let c = backend.connect(options, Credentials::new("root".into(), "fixture".into()));
    c.wait_ready(Duration::from_secs(3)).await.unwrap();
    c
}
async fn done(op: &ResourceOperation) -> Arc<ResourceOperationRecord> {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let r = op.latest();
            if r.state.terminal() || r.state == OperationState::OutcomeUnknown {
                return r;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}
fn command(c: &xenadmin_core::Connection, operation: HostOperation) -> ResourceCommand {
    let target = if matches!(operation, HostOperation::SetPoolAutostart { .. }) {
        ResourceTarget::Pool(PoolId::parse(POOL).unwrap())
    } else {
        host()
    };
    ResourceCommand::Host {
        target: c.latest().observe_resource(target).unwrap(),
        operation,
    }
}
fn empty(records: &Records) {
    let mut r = records.lock().unwrap();
    r.get_mut("host").unwrap()["resident_VMs"] = json!(["OpaqueRef:VM-dom0"]);
    r.get_mut("VM-guest").unwrap()["resident_on"] = json!("OpaqueRef:host-other");
    r.get_mut("host").unwrap()["enabled"] = json!(false);
}
async fn recheck(
    c: &xenadmin_core::Connection,
    id: OperationId,
    original: &ResourceOperation,
) -> ResourceOperationRecord {
    // The handle can settle before its final record reaches connection history.
    let old = original.latest().revision();
    c.control_resource(id, OperationControl::Recheck).unwrap();
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            if let Some(r) = c.resource_history().iter().find(|r| {
                r.id == id
                    && r.revision() > old
                    && (r.state.terminal() || r.state == OperationState::OutcomeUnknown)
            }) {
                return r.clone();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}
#[tokio::test]
async fn maintenance_evacuates_exact_guests_and_preserves_configuration() {
    let (mock, r) = fixture("", "").await;
    let d = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(d.path().into()).unwrap();
    let c = connect(&mock, &b, false).await;
    let op = c
        .submit_resource(
            OperationId::new(),
            command(
                &c,
                HostOperation::EnterMaintenance {
                    expected_enabled: true,
                    resident_vms: vec![VmId::parse(GUEST).unwrap()],
                },
            ),
        )
        .unwrap();
    let out = done(&op).await;
    assert_eq!(out.state, OperationState::Succeeded, "{}", out.detail);
    assert_eq!(out.steps.len(), 3);
    assert_eq!(
        r.lock().unwrap()["host"]["other_config"]["MAINTENANCE_MODE"],
        "true"
    );
    assert_eq!(r.lock().unwrap()["host"]["other_config"]["preserve"], "yes");
    assert!(
        !mock
            .methods
            .lock()
            .unwrap()
            .iter()
            .any(|m| m.contains("ha_set") || m.contains("VM.hard_shutdown"))
    );
    c.close().await;
}
#[tokio::test]
async fn evacuation_failure_retains_disabled_host_and_reports_partial() {
    let (mock, r) = fixture("", "Async.host.evacuate").await;
    let d = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(d.path().into()).unwrap();
    let c = connect(&mock, &b, false).await;
    let op = c
        .submit_resource(
            OperationId::new(),
            command(
                &c,
                HostOperation::EnterMaintenance {
                    expected_enabled: true,
                    resident_vms: vec![VmId::parse(GUEST).unwrap()],
                },
            ),
        )
        .unwrap();
    let out = done(&op).await;
    assert_eq!(out.state, OperationState::PartiallyCompleted);
    assert_eq!(r.lock().unwrap()["host"]["enabled"], false);
    assert!(!out.detail.contains("private fixture"));
    assert!(
        !mock
            .methods
            .lock()
            .unwrap()
            .iter()
            .any(|m| m == "Async.host.enable")
    );
    c.close().await;
}
#[tokio::test]
async fn evacuation_refuses_changed_membership_missing_destinations_and_blockers_before_disable() {
    for kind in 0..3 {
        let (mock, r) = fixture("", "").await;
        let d = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(d.path().into()).unwrap();
        let c = connect(&mock, &b, false).await;
        match kind {
            0 => {
                r.lock().unwrap().get_mut("host").unwrap()["resident_VMs"] =
                    json!(["OpaqueRef:VM-dom0"])
            }
            1 => {
                r.lock().unwrap().insert("_destinations".into(), json!([]));
            }
            _ => {
                r.lock().unwrap().insert(
                    "_blockers".into(),
                    json!({"OpaqueRef:VM-guest":["VM_HAS_PCI_ATTACHED"]}),
                );
            }
        }
        let op = c
            .submit_resource(
                OperationId::new(),
                command(
                    &c,
                    HostOperation::EnterMaintenance {
                        expected_enabled: true,
                        resident_vms: vec![VmId::parse(GUEST).unwrap()],
                    },
                ),
            )
            .unwrap();
        assert_eq!(done(&op).await.state, OperationState::Rejected);
        assert!(
            !mock
                .methods
                .lock()
                .unwrap()
                .iter()
                .any(|m| m == "Async.host.disable")
        );
        c.close().await;
    }
}
#[tokio::test]
async fn lost_disable_response_remains_unknown_without_evacuating_or_replay() {
    let (mock, r) = fixture("Async.host.disable", "").await;
    let d = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(d.path().into()).unwrap();
    let c = connect(&mock, &b, false).await;
    let id = OperationId::new();
    let op = c
        .submit_resource(
            id,
            command(
                &c,
                HostOperation::EnterMaintenance {
                    expected_enabled: true,
                    resident_vms: vec![VmId::parse(GUEST).unwrap()],
                },
            ),
        )
        .unwrap();
    assert_eq!(done(&op).await.state, OperationState::OutcomeUnknown);
    assert_eq!(r.lock().unwrap()["host"]["enabled"], false);
    let out = recheck(&c, id, &op).await;
    assert_eq!(out.state, OperationState::OutcomeUnknown, "{}", out.detail);
    assert_eq!(
        mock.methods
            .lock()
            .unwrap()
            .iter()
            .filter(|m| m.as_str() == "Async.host.disable")
            .count(),
        1
    );
    assert!(
        !mock
            .methods
            .lock()
            .unwrap()
            .iter()
            .any(|m| m == "Async.host.evacuate")
    );
    c.close().await;
}
#[tokio::test]
async fn exit_maintenance_verifies_both_steps() {
    let (mock, r) = fixture("", "").await;
    empty(&r);
    r.lock().unwrap().get_mut("host").unwrap()["other_config"]["MAINTENANCE_MODE"] = json!("true");
    let d = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(d.path().into()).unwrap();
    let c = connect(&mock, &b, false).await;
    let op = c
        .submit_resource(
            OperationId::new(),
            command(&c, HostOperation::ExitMaintenance),
        )
        .unwrap();
    assert_eq!(done(&op).await.state, OperationState::Succeeded);
    assert_eq!(r.lock().unwrap()["host"]["enabled"], true);
    assert!(
        r.lock().unwrap()["host"]["other_config"]
            .get("MAINTENANCE_MODE")
            .is_none()
    );
    c.close().await;
}
#[tokio::test]
async fn restart_and_shutdown_require_evidence_not_disconnection() {
    for method in [
        "Async.host.reboot",
        "Async.host.restart_agent",
        "Async.host.shutdown",
    ] {
        for drop_reply in [false, true] {
            let (mock, r) = fixture(if drop_reply { method } else { "" }, "").await;
            empty(&r);
            let d = tempfile::tempdir().unwrap();
            let b = Backend::with_state_directory(d.path().into()).unwrap();
            let c = connect(&mock, &b, false).await;
            let operation = match method {
                "Async.host.reboot" => HostOperation::Reboot {
                    expected_boot_time: "100".into(),
                },
                "Async.host.shutdown" => HostOperation::Shutdown {
                    expected_boot_time: "100".into(),
                },
                _ => HostOperation::RestartToolstack {
                    expected_agent_start_time: "110".into(),
                },
            };
            let id = OperationId::new();
            let op = c.submit_resource(id, command(&c, operation)).unwrap();
            let out = done(&op).await;
            assert_eq!(
                out.state,
                if drop_reply {
                    OperationState::OutcomeUnknown
                } else {
                    OperationState::Succeeded
                },
                "{method}: {}",
                out.detail
            );
            if drop_reply {
                let out = recheck(&c, id, &op).await;
                assert_eq!(
                    out.state,
                    if method == "Async.host.shutdown" {
                        OperationState::OutcomeUnknown
                    } else {
                        OperationState::Succeeded
                    },
                    "{}",
                    out.detail
                );
            }
            assert_eq!(
                mock.methods
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|m| m.as_str() == method)
                    .count(),
                1
            );
            c.close().await;
        }
    }
}
#[tokio::test]
async fn reboot_refuses_coordinator_and_changed_start_marker() {
    for coordinator in [false, true] {
        let (mock, r) = fixture("", "").await;
        empty(&r);
        let d = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(d.path().into()).unwrap();
        let c = connect(&mock, &b, false).await;
        if coordinator {
            r.lock().unwrap().get_mut("pool").unwrap()["master"] = json!("OpaqueRef:host");
        } else {
            r.lock().unwrap().get_mut("host").unwrap()["other_config"]["boot_time"] = json!("200");
        }
        let op = c
            .submit_resource(
                OperationId::new(),
                command(
                    &c,
                    HostOperation::Reboot {
                        expected_boot_time: "100".into(),
                    },
                ),
            )
            .unwrap();
        assert_eq!(done(&op).await.state, OperationState::Rejected);
        assert!(
            !mock
                .methods
                .lock()
                .unwrap()
                .iter()
                .any(|m| m == "Async.host.reboot")
        );
        c.close().await;
    }
}
#[tokio::test]
async fn dom0_memory_updates_only_verified_control_domain() {
    let (mock, r) = fixture("", "").await;
    let d = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(d.path().into()).unwrap();
    let c = connect(&mock, &b, false).await;
    let op = c
        .submit_resource(OperationId::new(), command(&c, op_memory()))
        .unwrap();
    let out = done(&op).await;
    assert_eq!(out.state, OperationState::Succeeded, "{}", out.detail);
    assert_eq!(r.lock().unwrap()["VM-dom0"]["memory_static_min"], "1048576");
    assert_eq!(
        r.lock().unwrap()["VM-guest"]["memory_static_max"],
        "2147483648"
    );
    c.close().await;
}
#[tokio::test]
async fn dom0_memory_rejects_changed_identity() {
    let (mock, r) = fixture("", "").await;
    let d = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(d.path().into()).unwrap();
    let c = connect(&mock, &b, false).await;
    r.lock().unwrap().get_mut("host").unwrap()["control_domain"] = json!("OpaqueRef:VM-guest");
    let op = c
        .submit_resource(OperationId::new(), command(&c, op_memory()))
        .unwrap();
    assert_eq!(done(&op).await.state, OperationState::Rejected);
    assert!(
        !mock
            .methods
            .lock()
            .unwrap()
            .iter()
            .any(|m| m == "VM.set_memory")
    );
    c.close().await;
}
#[tokio::test]
async fn logging_applies_or_reports_saved_but_uncertain_reconfigure() {
    for dropped in [false, true] {
        let (mock, r) = fixture(
            if dropped {
                "host.syslog_reconfigure"
            } else {
                ""
            },
            "",
        )
        .await;
        let d = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(d.path().into()).unwrap();
        let c = connect(&mock, &b, false).await;
        let id = OperationId::new();
        let op = c
            .submit_resource(
                id,
                command(
                    &c,
                    HostOperation::SetSyslog {
                        expected_destination: None,
                        destination: Some("logs.example.test".into()),
                    },
                ),
            )
            .unwrap();
        let out = done(&op).await;
        assert_eq!(
            out.state,
            if dropped {
                OperationState::OutcomeUnknown
            } else {
                OperationState::Succeeded
            }
        );
        assert_eq!(r.lock().unwrap()["host"]["logging"]["preserve"], "yes");
        if dropped {
            assert_eq!(
                recheck(&c, id, &op).await.state,
                OperationState::OutcomeUnknown
            );
        }
        assert_eq!(
            mock.methods
                .lock()
                .unwrap()
                .iter()
                .filter(|m| m.as_str() == "host.syslog_reconfigure")
                .count(),
            1
        );
        c.close().await;
    }
}
#[tokio::test]
async fn multipathing_requires_detached_storage_and_preserves_other_settings() {
    for attached in [false, true] {
        let (mock, r) = fixture("", "").await;
        empty(&r);
        {
            let mut r = r.lock().unwrap();
            r.get_mut("host").unwrap()["other_config"]["MAINTENANCE_MODE"] = json!("true");
            if attached {
                r.get_mut("host").unwrap()["PBDs"] = json!(["OpaqueRef:PBD"]);
                r.insert("PBD".into(),json!({"uuid":"88888888-8888-8888-8888-888888888888","host":"OpaqueRef:host","currently_attached":true}));
            }
        }
        let d = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(d.path().into()).unwrap();
        let c = connect(&mock, &b, false).await;
        let op = c
            .submit_resource(
                OperationId::new(),
                command(
                    &c,
                    HostOperation::SetMultipath {
                        expected: false,
                        enabled: true,
                    },
                ),
            )
            .unwrap();
        let out = done(&op).await;
        assert_eq!(
            out.state,
            if attached {
                OperationState::Rejected
            } else {
                OperationState::Succeeded
            },
            "{}",
            out.detail
        );
        assert_eq!(r.lock().unwrap()["host"]["other_config"]["preserve"], "yes");
        if !attached {
            assert_eq!(
                r.lock().unwrap()["host"]["other_config"]["multipathhandle"],
                "dmp"
            );
        }
        assert!(
            !mock
                .methods
                .lock()
                .unwrap()
                .iter()
                .any(|m| m.contains("PBD.unplug"))
        );
        c.close().await;
    }
}
#[tokio::test]
async fn pool_autostart_and_wol_are_typed_and_verified() {
    for pool in [false, true] {
        let (mock, r) = fixture("", "").await;
        let d = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(d.path().into()).unwrap();
        let c = connect(&mock, &b, false).await;
        let operation = if pool {
            HostOperation::SetPoolAutostart {
                expected: false,
                enabled: true,
            }
        } else {
            HostOperation::ConfigurePowerOn {
                expected_mode: "".into(),
                mode: HostPowerOnMode::WakeOnLan,
            }
        };
        let op = c
            .submit_resource(OperationId::new(), command(&c, operation))
            .unwrap();
        let out = done(&op).await;
        assert_eq!(out.state, OperationState::Succeeded, "{}", out.detail);
        if pool {
            assert_eq!(
                r.lock().unwrap()["pool"]["other_config"]["auto_poweron"],
                "true"
            );
            assert_eq!(r.lock().unwrap()["pool"]["other_config"]["preserve"], "yes");
        } else {
            assert_eq!(r.lock().unwrap()["host"]["power_on_mode"], "wake-on-lan");
        }
        c.close().await;
    }
}
#[tokio::test]
async fn power_on_and_record_only_forget_require_offline_evidence() {
    for forget in [false, true] {
        let (mock, r) = fixture("", "").await;
        empty(&r);
        r.lock().unwrap().get_mut("host_metrics").unwrap()["live"] = json!(false);
        r.lock().unwrap().get_mut("host").unwrap()["power_on_mode"] = json!("wake-on-lan");
        let d = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(d.path().into()).unwrap();
        let c = connect(&mock, &b, false).await;
        let op = c
            .submit_resource(
                OperationId::new(),
                command(
                    &c,
                    if forget {
                        HostOperation::ForgetDeadHost {
                            verified_power_off: true,
                        }
                    } else {
                        HostOperation::PowerOn
                    },
                ),
            )
            .unwrap();
        let out = done(&op).await;
        assert_eq!(out.state, OperationState::Succeeded, "{}", out.detail);
        assert!(
            !mock
                .methods
                .lock()
                .unwrap()
                .iter()
                .any(|m| m.contains("SR.forget") || m.contains("SR.destroy"))
        );
        c.close().await;
    }
}
#[tokio::test]
async fn host_workflows_respect_read_only_and_generation() {
    let (mock, _) = fixture("", "").await;
    let d = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(d.path().into()).unwrap();
    let c = connect(&mock, &b, true).await;
    assert!(
        c.submit_resource(OperationId::new(), command(&c, op_memory()))
            .is_err()
    );
    let captured = command(&c, op_memory());
    c.close().await;
    let next = connect(&mock, &b, false).await;
    assert!(next.submit_resource(OperationId::new(), captured).is_err());
    assert!(
        !mock
            .methods
            .lock()
            .unwrap()
            .iter()
            .any(|m| m == "VM.set_memory")
    );
    next.close().await;
}

#[tokio::test]
async fn stop_remaining_steps_waits_for_inflight_task_without_undo_or_replay() {
    let (mock, r) = fixture("", "").await;
    r.lock().unwrap().insert("_pending".into(), json!(true));
    let d = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(d.path().into()).unwrap();
    let c = connect(&mock, &b, false).await;
    let id = OperationId::new();
    let operation = c
        .submit_resource(
            id,
            command(
                &c,
                HostOperation::EnterMaintenance {
                    expected_enabled: true,
                    resident_vms: vec![VmId::parse(GUEST).unwrap()],
                },
            ),
        )
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while operation.latest().state != OperationState::Tracking {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    c.control_resource(id, OperationControl::RequestCancellation)
        .unwrap();
    r.lock().unwrap().insert("_pending".into(), json!(false));
    let out = done(&operation).await;
    assert_eq!(
        out.state,
        OperationState::PartiallyCompleted,
        "{}",
        out.detail
    );
    assert_eq!(r.lock().unwrap()["host"]["enabled"], false);
    assert!(
        !mock
            .methods
            .lock()
            .unwrap()
            .iter()
            .any(|m| m == "Async.host.evacuate" || m == "Async.host.enable" || m == "task.cancel")
    );
    c.close().await;
}
#[tokio::test]
async fn interrupted_maintenance_recovers_after_restart_without_dispatching_remaining_steps() {
    let (mock, _) = fixture("Async.host.disable", "").await;
    let d = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(d.path().into()).unwrap();
    let c = connect(&mock, &b, false).await;
    let id = OperationId::new();
    let operation = c
        .submit_resource(
            id,
            command(
                &c,
                HostOperation::EnterMaintenance {
                    expected_enabled: true,
                    resident_vms: vec![VmId::parse(GUEST).unwrap()],
                },
            ),
        )
        .unwrap();
    assert_eq!(done(&operation).await.state, OperationState::OutcomeUnknown);
    c.close().await;
    drop(c);
    drop(b);
    let b = Backend::with_state_directory(d.path().into()).unwrap();
    let c = connect(&mock, &b, false).await;
    let out = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(out) = c
                .resource_history()
                .iter()
                .find(|r| r.id == id && r.state == OperationState::OutcomeUnknown)
            {
                return out.clone();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(out.state, OperationState::OutcomeUnknown);
    assert_eq!(
        mock.methods
            .lock()
            .unwrap()
            .iter()
            .filter(|m| m.as_str() == "Async.host.disable")
            .count(),
        1
    );
    assert!(
        !mock
            .methods
            .lock()
            .unwrap()
            .iter()
            .any(|m| m == "Async.host.evacuate")
    );
    c.close().await;
}

#[tokio::test]
async fn missing_async_receipts_keep_evacuated_or_deleted_targets_unresolved() {
    for forget in [false, true] {
        let method = if forget {
            "Async.host.destroy"
        } else {
            "Async.host.evacuate"
        };
        let (mock, r) = fixture(method, "").await;
        if forget {
            empty(&r);
            r.lock().unwrap().get_mut("host_metrics").unwrap()["live"] = json!(false);
        } else {
            r.lock().unwrap().get_mut("host").unwrap()["enabled"] = json!(false);
        }
        let d = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(d.path().into()).unwrap();
        let c = connect(&mock, &b, false).await;
        let id = OperationId::new();
        let op = c
            .submit_resource(
                id,
                command(
                    &c,
                    if forget {
                        HostOperation::ForgetDeadHost {
                            verified_power_off: true,
                        }
                    } else {
                        HostOperation::Evacuate {
                            resident_vms: vec![VmId::parse(GUEST).unwrap()],
                        }
                    },
                ),
            )
            .unwrap();
        assert_eq!(done(&op).await.state, OperationState::OutcomeUnknown);
        let out = recheck(&c, id, &op).await;
        assert_eq!(out.state, OperationState::OutcomeUnknown, "{}", out.detail);
        assert_eq!(
            mock.methods
                .lock()
                .unwrap()
                .iter()
                .filter(|m| m.as_str() == method)
                .count(),
            1
        );
        c.close().await;
    }
}
#[tokio::test]
async fn toolstack_restart_refuses_enabled_or_unknown_ha_before_dispatch() {
    for enabled in [json!(true), Value::Null] {
        let (mock, r) = fixture("", "").await;
        let d = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(d.path().into()).unwrap();
        let c = connect(&mock, &b, false).await;
        r.lock().unwrap().get_mut("pool").unwrap()["ha_enabled"] = enabled;
        let op = c
            .submit_resource(
                OperationId::new(),
                command(
                    &c,
                    HostOperation::RestartToolstack {
                        expected_agent_start_time: "110".into(),
                    },
                ),
            )
            .unwrap();
        let out = done(&op).await;
        assert_eq!(out.state, OperationState::Rejected);
        assert!(out.detail.contains("HA"));
        assert!(
            !mock
                .methods
                .lock()
                .unwrap()
                .iter()
                .any(|m| m == "Async.host.restart_agent")
        );
        c.close().await;
    }
}
