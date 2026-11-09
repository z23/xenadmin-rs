use super::*;
use std::collections::BTreeMap;
use xenadmin_core::{
    HaRestartPriority, HostId, OperationControl, OperationId, OperationState, PoolId,
    PoolOperation, ResourceCommand, ResourceOperation, ResourceOperationRecord, ResourceTarget,
    SrId, VmId,
};
const POOL: &str = "11111111-1111-1111-1111-111111111111";
const HOST: &str = "22222222-2222-2222-2222-222222222222";
const OTHER: &str = "44444444-4444-4444-4444-444444444444";
const VM: &str = "33333333-3333-3333-3333-333333333333";
const SR: &str = "66666666-6666-6666-6666-666666666666";
#[derive(Clone, Copy, PartialEq)]
enum Fault {
    None,
    DropReply,
    TaskFailure,
    TaskCancelled,
    VerifyFailure,
}
type Records = Arc<Mutex<BTreeMap<String, Value>>>;
async fn fixture(fault: Fault) -> (Mock, Records) {
    let records: Records = Arc::new(Mutex::new(BTreeMap::from([
        (
            "pool".into(),
            json!({"uuid":POOL,"name_label":"Fixture pool","master":"OpaqueRef:host","current_operations":{},"allowed_operations":["ha_enable","ha_disable","designate_new_master","tls_verification_enable"],"migration_compression":false,"https_only":false,"ha_enabled":false,"ha_host_failures_to_tolerate":"0","ha_plan_exists_for":"0","ha_overcommitted":false,"ha_allow_overcommit":false,"ha_statefiles":[],"ha_configuration":{},"other_config":{"keep":"preserve"},"tls_verification_enabled":false,"is_psr_pending":false}),
        ),
        (
            "host".into(),
            json!({"uuid":HOST,"name_label":"Coordinator","enabled":true,"address":"192.0.2.10","metrics":"OpaqueRef:host_metrics","current_operations":{}}),
        ),
        (
            "host-other".into(),
            json!({"uuid":OTHER,"name_label":"Member","enabled":false,"address":"192.0.2.11","metrics":"OpaqueRef:host_metrics","current_operations":{}}),
        ),
        ("host_metrics".into(), json!({"live":true})),
        (
            "vm".into(),
            json!({"uuid":VM,"name_label":"Fixture guest","is_control_domain":false,"is_a_template":false,"is_a_snapshot":false,"power_state":"Halted","resident_on":"OpaqueRef:NULL","scheduled_to_be_resident_on":"OpaqueRef:NULL","ha_restart_priority":"","appliance":"OpaqueRef:NULL","order":"0","start_delay":"0","shutdown_delay":"0","current_operations":{},"VCPUs_at_startup":"2","memory_static_max":"2147483648","allowed_operations":["start"]}),
        ),
        (
            "SR".into(),
            json!({"uuid":SR,"name_label":"Heartbeat","type":"nfs","shared":true,"content_type":"user","current_operations":{},"PBDs":["OpaqueRef:PBD-one","OpaqueRef:PBD-two"],"VDIs":[],"physical_size":"4294967296","physical_utilisation":"0"}),
        ),
        (
            "PBD-one".into(),
            json!({"uuid":"77777777-7777-7777-7777-777777777777","host":"OpaqueRef:host","SR":"OpaqueRef:SR","currently_attached":true}),
        ),
        (
            "PBD-two".into(),
            json!({"uuid":"88888888-8888-8888-8888-888888888888","host":"OpaqueRef:host-other","SR":"OpaqueRef:SR","currently_attached":true}),
        ),
        (
            "VDI".into(),
            json!({"uuid":"99999999-9999-9999-9999-999999999999","SR":"OpaqueRef:SR"}),
        ),
    ])));
    let map = records.clone();
    let writes = Arc::new(AtomicUsize::new(0));
    let verify = Arc::new(AtomicUsize::new(0));
    let mock=mock_overridden(override_rpc(move|request|{let map=map.clone();let writes=writes.clone();let verify=verify.clone();async move {
        let method=request["method"].as_str().unwrap();let params=&request["params"];
        if method=="event.from"&&params[2]=="" {
            let records=map.lock().unwrap();let mut initial=batch("initial","Halted");
            for event in initial["events"].as_array_mut().unwrap() {let key=if event["class"]=="VM"{"vm"}else{event["class"].as_str().unwrap()};event["snapshot"]=records[key].clone();}
            initial["events"].as_array_mut().unwrap().push(json!({"class":"host","operation":"add","ref":"OpaqueRef:host-other","snapshot":records["host-other"]}));
            initial["events"].as_array_mut().unwrap().push(json!({"class":"SR","operation":"add","ref":"OpaqueRef:SR","snapshot":records["SR"]}));
            return Some(RpcReply::Json(json!({"result":initial})));
        }
        if let Some(class)=method.strip_suffix(".get_by_uuid") && ["pool","host","VM","SR","VDI"].contains(&class) {
                let records=map.lock().unwrap();let found=records.iter().find(|(_,r)|r["uuid"]==params[1]);
                return Some(RpcReply::Json(match found {Some((key,_))=>json!({"result":format!("OpaqueRef:{key}")}),None=>json!({"error":{"message":"UUID_INVALID","data":[]}})}));
        }
        if let Some(class)=method.strip_suffix(".get_record") && ["pool","host","VM","SR","PBD","VDI","host_metrics","VM_appliance"].contains(&class) {
            if fault==Fault::VerifyFailure&&writes.load(Ordering::SeqCst)>0&&class=="pool"&&verify.fetch_add(1,Ordering::SeqCst)==0 {return Some(RpcReply::Json(json!({"error":{"message":"RBAC_PERMISSION_DENIED","data":["private-secret"]}})));}
            let key=params[1].as_str().unwrap().strip_prefix("OpaqueRef:").unwrap();
            return Some(RpcReply::Json(json!({"result":map.lock().unwrap().get(key).cloned().unwrap_or(Value::Null)})));
        }
        if let Some(class)=method.strip_suffix(".get_all_records") && ["host","VM","PBD"].contains(&class) {
            let prefix=if class=="VM"{"vm"}else{class};
            let records:serde_json::Map<_,_>=map.lock().unwrap().iter().filter(|(key,_)|key.starts_with(prefix)&&*key!="host_metrics").map(|(key,v)|(format!("OpaqueRef:{key}"),v.clone())).collect();
            return Some(RpcReply::Json(json!({"result":records})));
        }
        if ["SR.assert_can_host_ha_statefile","VM.assert_agile"].contains(&method) {return Some(RpcReply::Json(json!({"result":null})));}
        if method=="pool.ha_failover_plan_exists" {return Some(RpcReply::Json(json!({"result":true})));}
        if method.starts_with("Async.pool.")||["pool.set_migration_compression","VM.set_ha_restart_priority","pool.enable_tls_verification","pool.set_ha_allow_overcommit","pool.set_other_config","VM.set_order","VM.set_start_delay","VM.set_shutdown_delay"].contains(&method) {
            writes.fetch_add(1,Ordering::SeqCst);let mut records=map.lock().unwrap();
            match method {
                "pool.set_migration_compression"=>records.get_mut("pool").unwrap()["migration_compression"]=params[2].clone(),
                "Async.pool.set_https_only"=>records.get_mut("pool").unwrap()["https_only"]=params[2].clone(),
                "Async.pool.designate_new_master"=>records.get_mut("pool").unwrap()["master"]=params[1].clone(),
                "Async.pool.set_ha_host_failures_to_tolerate"=>records.get_mut("pool").unwrap()["ha_host_failures_to_tolerate"]=params[2].clone(),
                "Async.pool.enable_ha"=>{assert_eq!(params[1],json!(["OpaqueRef:SR"]));assert_eq!(params[2],json!({}));let p=records.get_mut("pool").unwrap();p["ha_enabled"]=json!(true);p["ha_statefiles"]=json!(["99999999-9999-9999-9999-999999999999"]);},
                "Async.pool.disable_ha"=>records.get_mut("pool").unwrap()["ha_enabled"]=json!(false),
                "VM.set_ha_restart_priority"=>records.get_mut("vm").unwrap()["ha_restart_priority"]=params[2].clone(),
                "pool.set_ha_allow_overcommit"=>records.get_mut("pool").unwrap()["ha_allow_overcommit"]=params[2].clone(),
                "pool.set_other_config"=>records.get_mut("pool").unwrap()["other_config"]=params[2].clone(),
                "VM.set_order"=>records.get_mut("vm").unwrap()["order"]=params[2].clone(),
                "VM.set_start_delay"=>records.get_mut("vm").unwrap()["start_delay"]=params[2].clone(),
                "VM.set_shutdown_delay"=>records.get_mut("vm").unwrap()["shutdown_delay"]=params[2].clone(),
                "Async.pool.eject"=>{assert_eq!(params[1],"OpaqueRef:host-other");records.remove("host-other");},
                "pool.enable_tls_verification"=>records.get_mut("pool").unwrap()["tls_verification_enabled"]=json!(true),
                "Async.pool.rotate_secret"=>{},
                _=>panic!("unexpected mutation {method}"),
            }
            return Some(if fault==Fault::DropReply {RpcReply::Drop}else{RpcReply::Json(json!({"result":if method.starts_with("Async."){json!("OpaqueRef:pool-task")}else{Value::Null}}))});
        }
        if method=="task.get_record" {return Some(RpcReply::Json(json!({"result":{"status":match fault{Fault::TaskFailure=>"failure",Fault::TaskCancelled=>"cancelled",_=>"success"},"allowed_operations":[],"progress":"1","error_info":["HOST_OFFLINE","private-secret"],"result":"<value/>"}})));}
        None
    }})).await;
    (mock, records)
}
async fn connect(mock: &Mock, backend: &Backend, readonly: bool) -> xenadmin_core::Connection {
    let mut options = ConnectionOptions::new(mock.endpoint.clone());
    options.read_only = readonly;
    let c = backend.connect(
        options,
        Credentials::new("fixture".into(), "fixture-password".into()),
    );
    let mut w = c.watch();
    tokio::time::timeout(deadline(3), async {
        while !w.borrow().is_current() {
            w.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    c
}
async fn finished(op: &ResourceOperation) -> Arc<ResourceOperationRecord> {
    tokio::time::timeout(deadline(4), async {
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
fn pool() -> ResourceTarget {
    ResourceTarget::Pool(PoolId::parse(POOL).unwrap())
}
fn submit(c: &xenadmin_core::Connection, operation: PoolOperation) -> ResourceOperation {
    c.submit_resource(
        OperationId::new(),
        ResourceCommand::Pool {
            target: c.latest().observe_resource(pool()).unwrap(),
            operation,
        },
    )
    .unwrap()
}
fn enable() -> PoolOperation {
    PoolOperation::EnableHa {
        heartbeat_srs: vec![SrId::parse(SR).unwrap()],
        expected_failures: 0,
    }
}
fn mutations(mock: &Mock) -> usize {
    mock.methods
        .lock()
        .unwrap()
        .iter()
        .filter(|m| {
            m.starts_with("Async.pool.")
                || [
                    "pool.set_migration_compression",
                    "VM.set_ha_restart_priority",
                    "pool.enable_tls_verification",
                    "pool.set_ha_allow_overcommit",
                    "pool.set_other_config",
                    "VM.set_order",
                    "VM.set_start_delay",
                    "VM.set_shutdown_delay",
                ]
                .contains(&m.as_str())
        })
        .count()
}

#[tokio::test]
async fn pool_policy_mutations_verify_current_state_and_keep_secrets_out_of_history() {
    let (mock, records) = fixture(Fault::None).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&mock, &backend, false).await;
    let ops = [
        PoolOperation::SetMigrationCompression {
            expected: false,
            enabled: true,
        },
        PoolOperation::SetHttpsOnly {
            expected: false,
            enabled: true,
        },
        PoolOperation::SetHaFailuresToTolerate {
            expected: 0,
            failures: 1,
        },
        PoolOperation::SetVmRestartPriority {
            vm: VmId::parse(VM).unwrap(),
            expected: HaRestartPriority::Disabled,
            priority: HaRestartPriority::Restart,
        },
        PoolOperation::EnableTlsVerification,
        PoolOperation::RotateSecret,
    ];
    for op in ops {
        let r = finished(&submit(&c, op)).await;
        assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
        let json = serde_json::to_string(&*r).unwrap();
        assert!(!json.contains("OpaqueRef") && !json.contains("private-secret"));
    }
    assert_eq!(mutations(&mock), 6);
    assert_eq!(
        records.lock().unwrap()["vm"]["ha_restart_priority"],
        "restart"
    );
}
#[tokio::test]
async fn pool_ha_enable_checks_storage_then_verifies_heartbeat_and_explicit_disable() {
    let (mock, _) = fixture(Fault::None).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&mock, &backend, false).await;
    let r = finished(&submit(&c, enable())).await;
    assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
    let r = finished(&submit(&c, PoolOperation::DisableHa)).await;
    assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
    let methods = mock.methods.lock().unwrap();
    assert!(
        methods
            .iter()
            .any(|m| m == "SR.assert_can_host_ha_statefile")
    );
    assert!(methods.iter().any(|m| m == "pool.ha_failover_plan_exists"));
}
#[tokio::test]
async fn pool_handover_and_ejection_are_explicit_and_guarded() {
    for eject in [false, true] {
        let (mock, records) = fixture(Fault::None).await;
        if !eject {
            records.lock().unwrap().get_mut("host-other").unwrap()["enabled"] = json!(true);
        }
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&mock, &backend, false).await;
        let op = if eject {
            PoolOperation::EjectHost {
                host: HostId::parse(OTHER).unwrap(),
                expected_name: "Member".into(),
            }
        } else {
            PoolOperation::DesignateCoordinator {
                expected: HostId::parse(HOST).unwrap(),
                host: HostId::parse(OTHER).unwrap(),
            }
        };
        let r = finished(&submit(&c, op)).await;
        assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
        assert_eq!(mutations(&mock), 1);
    }
}
#[tokio::test]
async fn pool_preflight_refuses_changed_policy_detached_heartbeat_and_occupied_host() {
    for case in 0..5 {
        let (mock, records) = fixture(Fault::None).await;
        let op = match case {
            0 => PoolOperation::SetMigrationCompression {
                expected: true,
                enabled: false,
            },
            1 => {
                records.lock().unwrap().get_mut("PBD-two").unwrap()["currently_attached"] =
                    json!(false);
                enable()
            }
            2 => {
                records.lock().unwrap().get_mut("vm").unwrap()["resident_on"] =
                    json!("OpaqueRef:host-other");
                PoolOperation::EjectHost {
                    host: HostId::parse(OTHER).unwrap(),
                    expected_name: "Member".into(),
                }
            }
            3 => {
                records.lock().unwrap().get_mut("pool").unwrap()["ha_enabled"] = json!(true);
                PoolOperation::RotateSecret
            }
            _ => {
                records.lock().unwrap().get_mut("SR").unwrap()["shared"] = json!(false);
                records.lock().unwrap().get_mut("SR").unwrap()["VDIs"] = json!(["OpaqueRef:VDI"]);
                PoolOperation::EjectHost {
                    host: HostId::parse(OTHER).unwrap(),
                    expected_name: "Member".into(),
                }
            }
        };
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&mock, &backend, false).await;
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
async fn pool_lost_dispatch_never_uses_matching_state_as_async_completion() {
    let (mock, _) = fixture(Fault::DropReply).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&mock, &backend, false).await;
    let op = submit(&c, enable());
    let r = finished(&op).await;
    assert_eq!(r.state, OperationState::OutcomeUnknown);
    c.control_resource(r.id, OperationControl::Recheck).unwrap();
    tokio::time::sleep(Duration::from_millis(120)).await;
    assert_eq!(op.latest().state, OperationState::OutcomeUnknown);
    assert_eq!(mutations(&mock), 1);
}
#[tokio::test]
async fn pool_failed_or_cancelled_task_keeps_pool_reserved_and_task_evidence() {
    for fault in [Fault::TaskFailure, Fault::TaskCancelled] {
        let (mock, _) = fixture(fault).await;
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&mock, &backend, false).await;
        let r = finished(&submit(&c, enable())).await;
        assert_eq!(r.state, OperationState::OutcomeUnknown);
        assert!(!r.detail.contains("private-secret"));
        let next = finished(&submit(
            &c,
            PoolOperation::SetMigrationCompression {
                expected: false,
                enabled: true,
            },
        ))
        .await;
        assert_eq!(next.state, OperationState::Rejected);
        assert_eq!(mutations(&mock), 1);
        assert!(
            !mock
                .methods
                .lock()
                .unwrap()
                .iter()
                .any(|m| m == "task.destroy")
        );
    }
}
#[tokio::test]
async fn pool_completed_receipt_survives_verification_failure_and_rechecks_without_replay() {
    let (mock, _) = fixture(Fault::VerifyFailure).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&mock, &backend, false).await;
    let op = submit(
        &c,
        PoolOperation::SetMigrationCompression {
            expected: false,
            enabled: true,
        },
    );
    let r = finished(&op).await;
    assert_eq!(r.state, OperationState::OutcomeUnknown);
    c.control_resource(r.id, OperationControl::Recheck).unwrap();
    tokio::time::timeout(deadline(3), async {
        loop {
            if c.resource_history()
                .iter()
                .any(|v| v.id == r.id && v.state == OperationState::Succeeded)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(mutations(&mock), 1);
}
#[tokio::test]
async fn pool_readonly_rejects_mutation_and_empty_ha_priority_remains_known() {
    let (mock, _) = fixture(Fault::None).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&mock, &backend, true).await;
    let snapshot = c.latest();
    let inventory = snapshot.inventory().unwrap();
    assert_eq!(
        inventory.details.vms[&VmId::parse(VM).unwrap()]
            .ha_restart_priority
            .as_deref(),
        Some("")
    );
    assert!(
        c.submit_resource(
            OperationId::new(),
            ResourceCommand::Pool {
                target: snapshot.observe_resource(pool()).unwrap(),
                operation: enable()
            }
        )
        .is_err()
    );
    assert_eq!(mutations(&mock), 0);
}

#[path = "pool_ha_policy.rs"]
mod ha_policy;
