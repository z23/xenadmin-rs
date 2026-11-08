use super::*;
use std::collections::BTreeMap;
use xenadmin_core::{
    HostId, OperationControl, OperationId, OperationState, PoolId, ResourceCommand,
    ResourceOperation, ResourceOperationRecord, ResourceTarget, ScheduledSnapshotKind,
    SnapshotPolicyClock, SnapshotPolicyConfig, SnapshotPolicyId, SnapshotPolicyOperation as Op,
    SnapshotPolicyState, SnapshotSchedule, VmId,
};
const POOL: &str = "11111111-1111-1111-1111-111111111111";
const HOST: &str = "22222222-2222-2222-2222-222222222222";
const VM: &str = "33333333-3333-3333-3333-333333333333";
const POLICY: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const NEW: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
const SNAP: &str = "cccccccc-cccc-4ccc-8ccc-cccccccccccc";
#[derive(Clone, Copy, PartialEq)]
enum Fault {
    None,
    Drop,
    Failure,
    Cancelled,
    PluginFailure,
    NoSnapshot,
    Preexisting,
    Restricted,
    Version,
    Clock,
    SecondField,
    VerifyOnce,
}
type Records = Arc<Mutex<BTreeMap<String, Value>>>;
fn id() -> SnapshotPolicyId {
    SnapshotPolicyId::parse(POLICY).unwrap()
}
fn vmid() -> VmId {
    VmId::parse(VM).unwrap()
}
fn clock() -> SnapshotPolicyClock {
    SnapshotPolicyClock {
        coordinator: HostId::parse(HOST).unwrap(),
        utc_offset_minutes: 600,
    }
}
fn config() -> SnapshotPolicyConfig {
    SnapshotPolicyConfig {
        name: "Daily recovery".into(),
        description: "Example".into(),
        kind: ScheduledSnapshotKind::Disk,
        retained: 3,
        schedule: SnapshotSchedule::Daily {
            hour: 2,
            minute: 15,
        },
    }
}
fn expected(enabled: bool, members: bool) -> SnapshotPolicyState {
    SnapshotPolicyState {
        id: id(),
        config: config(),
        enabled,
        members: if members { vec![vmid()] } else { vec![] },
    }
}
fn policy(uuid: &str) -> Value {
    json!({"uuid":uuid,"name_label":"Daily recovery","name_description":"Example","enabled":false,"type":"snapshot","retained_snapshots":"3","frequency":"daily","schedule":{"hour":"2","min":"15"},"VMs":[],"last_run_time":"19700101T00:00:00Z"})
}
async fn fixture(fault: Fault) -> (Mock, Records) {
    let mut v = batch("initial", "Halted")["events"][2]["snapshot"].clone();
    for(k,x)in json!({"is_a_template":false,"is_a_snapshot":false,"is_control_domain":false,"current_operations":{},"snapshot_schedule":"OpaqueRef:NULL","snapshots":[]}).as_object().unwrap(){v[k]=x.clone();}
    let pool = json!({"uuid":POOL,"master":"OpaqueRef:host","restrictions":{"restrict_vmss":if fault==Fault::Restricted{"true"}else{"false"}}});
    let host = json!({"uuid":HOST,"name_label":"Coordinator","software_version":{"product_brand":"XCP-ng","product_version":if fault==Fault::Version{"9.0"}else{"8.3.0"}}});
    let records: Records = Arc::new(Mutex::new(BTreeMap::from([
        ("vm".into(), v),
        ("policy".into(), policy(POLICY)),
        ("pool".into(), pool),
        ("host".into(), host),
    ])));
    let map = records.clone();
    let writes = Arc::new(AtomicUsize::new(0));
    let verified = Arc::new(AtomicUsize::new(0));
    let last = Arc::new(Mutex::new(String::new()));
    let server=mock_overridden(override_rpc(move|request|{let map=map.clone();let writes=writes.clone();let verified=verified.clone();let last=last.clone();async move{let method=request["method"].as_str().unwrap();let p=&request["params"];
 if method=="event.from"&&p[2]==""{let mut b=batch("initial","Halted");b["events"][2]["snapshot"]=map.lock().unwrap()["vm"].clone();return Some(RpcReply::Json(json!({"result":b})));}
 if method=="VMSS.get_all_records"{let v:serde_json::Map<_,_>=map.lock().unwrap().iter().filter(|(k,_)|k.starts_with("policy")).map(|(k,v)|(format!("OpaqueRef:{k}"),v.clone())).collect();return Some(RpcReply::Json(json!({"result":v})));}
 if ["VM.get_by_uuid","VMSS.get_by_uuid","pool.get_by_uuid"].contains(&method){let m=map.lock().unwrap();let v=m.iter().find(|(_,v)|v["uuid"]==p[1]);return Some(RpcReply::Json(match v{Some((k,_))=>json!({"result":format!("OpaqueRef:{k}")}),None=>json!({"error":{"message":"UUID_INVALID","data":[]}})}));}
 if ["VM.get_record","VMSS.get_record","pool.get_record","host.get_record"].contains(&method){if method=="VMSS.get_record"&&fault==Fault::VerifyOnce&&writes.load(Ordering::SeqCst)>0&&verified.fetch_add(1,Ordering::SeqCst)==0{return Some(RpcReply::Json(json!({"error":{"message":"RBAC_PERMISSION_DENIED","data":[]}})));}let key=p[1].as_str().unwrap().strip_prefix("OpaqueRef:").unwrap();return Some(RpcReply::Json(json!({"result":map.lock().unwrap().get(key).cloned().unwrap_or(Value::Null)})));}
 if method=="host.get_servertime"{return Some(RpcReply::Json(json!({"result":"20261108T00:00:00Z"})));}
 if method=="host.get_server_localtime"{return Some(RpcReply::Json(json!({"result":if fault==Fault::Clock{"unavailable"}else{"20261108T10:00:00Z"}})));}
 if method=="VM.assert_operation_valid"{return Some(RpcReply::Json(json!({"result":null})));}
 if method.starts_with("Async.VMSS.")||method.starts_with("VMSS.set_")||method=="VM.set_snapshot_schedule"{let n=writes.fetch_add(1,Ordering::SeqCst);if fault==Fault::SecondField&&n==1{return Some(RpcReply::Json(json!({"error":{"message":"RBAC_PERMISSION_DENIED","data":[]}})));}*last.lock().unwrap()=method.into();let mut m=map.lock().unwrap();match method{
 "Async.VMSS.create"=>{assert_eq!(p[1]["enabled"],false);let mut v=policy(NEW);for(k,x)in p[1].as_object().unwrap(){v[k]=x.clone();}m.insert("policy-new".into(),v);},
 "Async.VMSS.destroy"=>{assert_eq!(m["policy"]["VMs"],json!([]));m.remove("policy");},
 "Async.VMSS.snapshot_now"=>{if fault!=Fault::NoSnapshot{m.insert("snapshot".into(),json!({"uuid":SNAP,"is_a_snapshot":true,"is_vmss_snapshot":true,"snapshot_of":"OpaqueRef:vm"}));m.get_mut("vm").unwrap()["snapshots"]=json!(["OpaqueRef:snapshot"]);}m.get_mut("policy").unwrap()["last_run_time"]=json!("20261108T00:00:00Z");},
 "VM.set_snapshot_schedule"=>{m.get_mut("vm").unwrap()["snapshot_schedule"]=p[2].clone();m.get_mut("policy").unwrap()["VMs"]=if p[2]=="OpaqueRef:NULL"{json!([])}else{json!(["OpaqueRef:vm"])};},
 _=>{let field=method.strip_prefix("VMSS.set_").unwrap();let v=m.get_mut("policy").unwrap();v[field]=p[2].clone();if field=="frequency"{let minute=v["schedule"]["min"].clone();let hour=v["schedule"].get("hour").cloned().unwrap_or(json!("0"));v["schedule"]=match p[2].as_str().unwrap(){"hourly"=>json!({"min":minute}),"daily"=>json!({"hour":hour,"min":minute}),_=>json!({"hour":hour,"min":minute,"days":"Monday"})};}}}
 return Some(if fault==Fault::Drop{RpcReply::Drop}else{RpcReply::Json(json!({"result":if method.starts_with("Async."){json!("OpaqueRef:policy-task")}else{Value::Null}}))});}
 if method=="task.get_record"{let result=if last.lock().unwrap().ends_with("create"){if fault==Fault::Preexisting{"<value><string>OpaqueRef:policy</string></value>"}else{"<value><string>OpaqueRef:policy-new</string></value>"}}else if fault==Fault::PluginFailure{"<value><string>private plugin failure</string></value>"}else{"<value><string>True</string></value>"};return Some(RpcReply::Json(json!({"result":{"status":if fault==Fault::Failure{"failure"}else if fault==Fault::Cancelled{"cancelled"}else{"success"},"progress":"1","allowed_operations":[],"error_info":["OPERATION_PARTIALLY_FAILED","private payload"],"result":result}})));}None}})).await;
    (server, records)
}
async fn connect(m: &Mock, b: &Backend, ro: bool) -> xenadmin_core::Connection {
    let mut opts = ConnectionOptions::new(m.endpoint.clone());
    opts.read_only = ro;
    let c = b.connect(
        opts,
        Credentials::new("fixture".into(), "fixture-password".into()),
    );
    c.wait_ready(Duration::from_secs(3)).await.unwrap();
    c
}
async fn done(o: &ResourceOperation) -> Arc<ResourceOperationRecord> {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let r = o.latest();
            if r.state.terminal() || r.state == OperationState::OutcomeUnknown {
                return r;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}
fn submit(c: &xenadmin_core::Connection, operation: Op) -> ResourceOperation {
    c.submit_resource(
        OperationId::new(),
        ResourceCommand::SnapshotPolicy {
            pool: c
                .latest()
                .observe_resource(ResourceTarget::Pool(PoolId::parse(POOL).unwrap()))
                .unwrap(),
            operation,
        },
    )
    .unwrap()
}
fn assign(m: &Records, enabled: bool) {
    let mut m = m.lock().unwrap();
    m.get_mut("policy").unwrap()["VMs"] = json!(["OpaqueRef:vm"]);
    m.get_mut("policy").unwrap()["enabled"] = json!(enabled);
    m.get_mut("vm").unwrap()["snapshot_schedule"] = json!("OpaqueRef:policy");
}
fn writes(m: &Mock) -> usize {
    m.methods
        .lock()
        .unwrap()
        .iter()
        .filter(|s| {
            s.starts_with("Async.VMSS.")
                || s.starts_with("VMSS.set_")
                || *s == "VM.set_snapshot_schedule"
        })
        .count()
}
#[tokio::test]
async fn policy_create_is_disabled_new_identity_and_private_evidence_survives() {
    let (m, _) = fixture(Fault::None).await;
    let d = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(d.path().into()).unwrap();
    let c = connect(&m, &b, false).await;
    let r = done(&submit(
        &c,
        Op::Create {
            config: config(),
            clock: clock(),
        },
    ))
    .await;
    assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
    assert_eq!(
        r.created,
        Some(ResourceTarget::SnapshotPolicy(
            SnapshotPolicyId::parse(NEW).unwrap()
        ))
    );
    let public = serde_json::to_string(&*r).unwrap();
    assert!(!public.contains("OpaqueRef:"));
    assert!(!public.contains("snapshot_policy_evidence"));
}
#[tokio::test]
async fn policy_support_reports_host_local_clock_and_refuses_unknown_runtime() {
    for fault in [Fault::None, Fault::Restricted, Fault::Version, Fault::Clock] {
        let (m, _) = fixture(fault).await;
        let d = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(d.path().into()).unwrap();
        let c = connect(&m, &b, false).await;
        let h = c
            .snapshot_policy_support(
                c.latest()
                    .observe_resource(ResourceTarget::Pool(PoolId::parse(POOL).unwrap()))
                    .unwrap(),
            )
            .unwrap();
        let value = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let Some(v) = h.latest() {
                    break v;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(value.is_ok(), fault == Fault::None);
        if let Ok(s) = value {
            assert_eq!(s.clock, clock());
        }
        assert_eq!(writes(&m), 0);
    }
}
#[tokio::test]
async fn policy_configuration_changes_frequency_and_map_in_reviewed_order() {
    let (m, records) = fixture(Fault::None).await;
    let d = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(d.path().into()).unwrap();
    let c = connect(&m, &b, false).await;
    let mut next = config();
    next.name = "Weekly".into();
    next.schedule = SnapshotSchedule::Weekly {
        hour: 5,
        minute: 30,
        days: vec![1, 4],
    };
    let r = done(&submit(
        &c,
        Op::Configure {
            expected: expected(false, false),
            config: next,
            clock: clock(),
        },
    ))
    .await;
    assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
    assert_eq!(
        records.lock().unwrap()["policy"]["schedule"],
        json!({"hour":"5","min":"30","days":"Tuesday,Friday"})
    );
}
#[tokio::test]
async fn policy_membership_enable_run_disable_detach_delete_preserve_guest() {
    let (m, records) = fixture(Fault::None).await;
    let d = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(d.path().into()).unwrap();
    let c = connect(&m, &b, false).await;
    for op in [
        Op::Membership {
            expected: expected(false, false),
            vm: vmid(),
            join: true,
        },
        Op::SetEnabled {
            expected: expected(false, true),
            enabled: true,
            clock: Some(clock()),
        },
        Op::RunNow {
            expected: expected(true, true),
        },
        Op::SetEnabled {
            expected: expected(true, true),
            enabled: false,
            clock: None,
        },
        Op::Membership {
            expected: expected(false, true),
            vm: vmid(),
            join: false,
        },
        Op::DeleteEmpty {
            expected: expected(false, false),
        },
    ] {
        let r = done(&submit(&c, op)).await;
        assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
    }
    let m = records.lock().unwrap();
    assert!(m.contains_key("vm"));
    assert!(m.contains_key("snapshot"));
}
#[tokio::test]
async fn policy_run_partial_or_missing_snapshot_never_claims_success_or_replays() {
    for fault in [
        Fault::Failure,
        Fault::Cancelled,
        Fault::PluginFailure,
        Fault::NoSnapshot,
        Fault::Drop,
    ] {
        let (m, records) = fixture(fault).await;
        assign(&records, true);
        let d = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(d.path().into()).unwrap();
        let c = connect(&m, &b, false).await;
        let r = done(&submit(
            &c,
            Op::RunNow {
                expected: expected(true, true),
            },
        ))
        .await;
        assert_eq!(r.state, OperationState::OutcomeUnknown, "{}", r.detail);
        assert!(!r.detail.contains("private"));
        c.control_resource(r.id, OperationControl::Recheck).unwrap();
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert_eq!(writes(&m), 1);
        let other = done(&submit(
            &c,
            Op::Create {
                config: config(),
                clock: clock(),
            },
        ))
        .await;
        assert_eq!(other.state, OperationState::Rejected);
    }
}
#[tokio::test]
async fn policy_create_lost_reply_or_preexisting_result_remains_unknown() {
    for fault in [Fault::Drop, Fault::Preexisting] {
        let (m, _) = fixture(fault).await;
        let d = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(d.path().into()).unwrap();
        let c = connect(&m, &b, false).await;
        let r = done(&submit(
            &c,
            Op::Create {
                config: config(),
                clock: clock(),
            },
        ))
        .await;
        assert_eq!(r.state, OperationState::OutcomeUnknown);
        assert!(r.created.is_none());
        assert_eq!(writes(&m), 1);
    }
}
#[tokio::test]
async fn policy_edit_partial_keeps_first_field_and_empty_enable_refuses() {
    let (m, records) = fixture(Fault::SecondField).await;
    let d = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(d.path().into()).unwrap();
    let c = connect(&m, &b, false).await;
    assert!(
        c.submit_resource(
            OperationId::new(),
            ResourceCommand::SnapshotPolicy {
                pool: c
                    .latest()
                    .observe_resource(ResourceTarget::Pool(PoolId::parse(POOL).unwrap()))
                    .unwrap(),
                operation: Op::SetEnabled {
                    expected: expected(false, false),
                    enabled: true,
                    clock: Some(clock())
                }
            }
        )
        .is_err()
    );
    let mut next = config();
    next.name = "Changed".into();
    next.description = "Changed description".into();
    let r = done(&submit(
        &c,
        Op::Configure {
            expected: expected(false, false),
            config: next,
            clock: clock(),
        },
    ))
    .await;
    assert_eq!(r.state, OperationState::OutcomeUnknown);
    assert_eq!(records.lock().unwrap()["policy"]["name_label"], "Changed");
    assert_eq!(writes(&m), 2);
}
#[tokio::test]
async fn policy_receipt_survives_recheck_without_task_replay() {
    let (m, _) = fixture(Fault::VerifyOnce).await;
    let d = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(d.path().into()).unwrap();
    let c = connect(&m, &b, false).await;
    let r = done(&submit(
        &c,
        Op::Create {
            config: config(),
            clock: clock(),
        },
    ))
    .await;
    assert_eq!(r.state, OperationState::OutcomeUnknown);
    c.control_resource(r.id, OperationControl::Recheck).unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if c.resource_history()
                .iter()
                .any(|v| v.id == r.id && v.state == OperationState::Succeeded)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(writes(&m), 1);
    assert_eq!(
        m.methods
            .lock()
            .unwrap()
            .iter()
            .filter(|s| *s == "task.get_record")
            .count(),
        1
    );
}

#[tokio::test]
async fn policy_preflight_refuses_changed_membership_clock_and_readonly_before_writes() {
    for case in 0..3 {
        let (m, records) = fixture(Fault::None).await;
        let d = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(d.path().into()).unwrap();
        let c = connect(&m, &b, case == 2).await;
        let operation = if case == 0 {
            assign(&records, true);
            let mut changed = expected(true, true);
            changed.members.push(VmId::from(uuid::Uuid::new_v4()));
            Op::RunNow { expected: changed }
        } else {
            let mut clock = clock();
            clock.utc_offset_minutes = 0;
            Op::Create {
                config: config(),
                clock,
            }
        };
        let result = c.submit_resource(
            OperationId::new(),
            ResourceCommand::SnapshotPolicy {
                pool: c
                    .latest()
                    .observe_resource(ResourceTarget::Pool(PoolId::parse(POOL).unwrap()))
                    .unwrap(),
                operation,
            },
        );
        if case == 2 {
            assert!(result.is_err());
        } else {
            let r = done(&result.unwrap()).await;
            assert_eq!(r.state, OperationState::Rejected, "{}", r.detail);
        }
        assert_eq!(writes(&m), 0);
    }
}

#[tokio::test]
async fn policy_restart_restores_private_completion_and_baseline_without_replay() {
    let (m, _) = fixture(Fault::VerifyOnce).await;
    let d = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(d.path().into()).unwrap();
    let c = connect(&m, &b, false).await;
    let r = done(&submit(
        &c,
        Op::Create {
            config: config(),
            clock: clock(),
        },
    ))
    .await;
    assert_eq!(r.state, OperationState::OutcomeUnknown);
    let id = r.id;
    c.close().await;
    drop(c);
    drop(b);
    let b = Backend::with_state_directory(d.path().into()).unwrap();
    let c = connect(&m, &b, false).await;
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if c.resource_history()
                .iter()
                .any(|r| r.id == id && r.state == OperationState::Succeeded)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(writes(&m), 1);
    assert_eq!(
        m.methods
            .lock()
            .unwrap()
            .iter()
            .filter(|m| *m == "task.get_record")
            .count(),
        1
    );
    c.close().await;
}
