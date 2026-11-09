use super::*;
use std::collections::BTreeMap;
use xenadmin_core::{
    ApplianceId, ApplianceMember, ApplianceOperation as Op, ApplianceOrder, AppliancePower,
    OperationControl, OperationId, OperationState, PoolId, ResourceCommand, ResourceOperation,
    ResourceOperationRecord, ResourceTarget, VmId,
};
const POOL: &str = "11111111-1111-1111-1111-111111111111";
const VM: &str = "33333333-3333-3333-3333-333333333333";
const APP: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const NEW: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
#[derive(Clone, Copy, PartialEq)]
enum Fault {
    None,
    DropReply,
    TaskFailure,
    TaskCancelled,
    SecondField,
    VerifyOnce,
    UnchangedChild,
}
type Records = Arc<Mutex<BTreeMap<String, Value>>>;
fn id() -> ApplianceId {
    ApplianceId::parse(APP).unwrap()
}
fn vmid() -> VmId {
    VmId::parse(VM).unwrap()
}
fn order() -> ApplianceOrder {
    ApplianceOrder {
        order: 1,
        start_delay: 2,
        shutdown_delay: 3,
    }
}
fn group(uuid: &str) -> Value {
    json!({"uuid":uuid,"name_label":"Application","name_description":"Example","VMs":[],"current_operations":{},"allowed_operations":["start","clean_shutdown","hard_shutdown"]})
}
async fn fixture(fault: Fault) -> (Mock, Records) {
    let mut vm = batch("initial", "Halted")["events"][2]["snapshot"].clone();
    for (k,v) in json!({"is_a_template":false,"is_a_snapshot":false,"is_control_domain":false,"appliance":"OpaqueRef:NULL","current_operations":{},"order":"1","start_delay":"2","shutdown_delay":"3"}).as_object().unwrap(){vm[k]=v.clone();}
    let records: Records = Arc::new(Mutex::new(BTreeMap::from([
        ("vm".into(), vm),
        ("app".into(), group(APP)),
    ])));
    let map = records.clone();
    let writes = Arc::new(AtomicUsize::new(0));
    let verified = Arc::new(AtomicUsize::new(0));
    let server=mock_overridden(override_rpc(move|request|{let map=map.clone();let writes=writes.clone();let verified=verified.clone();async move{
        let method=request["method"].as_str().unwrap();let p=&request["params"];
        if method=="event.from" && p[2]==""{let mut b=batch("initial","Halted");b["events"][2]["snapshot"]=map.lock().unwrap()["vm"].clone();b["events"].as_array_mut().unwrap().push(json!({"class":"VM_appliance","operation":"add","ref":"OpaqueRef:app","snapshot":map.lock().unwrap()["app"]}));return Some(RpcReply::Json(json!({"result":b})));}
        if method=="VM_appliance.get_all_records"{let records:serde_json::Map<_,_>=map.lock().unwrap().iter().filter(|(k,_)|k.starts_with("app")).map(|(k,v)|(format!("OpaqueRef:{k}"),v.clone())).collect();return Some(RpcReply::Json(json!({"result":records})));}
        if ["VM.get_by_uuid","VM_appliance.get_by_uuid"].contains(&method){let records=map.lock().unwrap();let found=records.iter().find(|(_,r)|r["uuid"]==p[1]);return Some(RpcReply::Json(match found{Some((k,_))=>json!({"result":format!("OpaqueRef:{k}")}),None=>json!({"error":{"message":"UUID_INVALID","data":[]}})}));}
        if ["VM.get_record","VM_appliance.get_record"].contains(&method){if method=="VM_appliance.get_record" && fault==Fault::VerifyOnce && writes.load(Ordering::SeqCst)>0 && verified.fetch_add(1,Ordering::SeqCst)==0 {return Some(RpcReply::Json(json!({"error":{"message":"RBAC_PERMISSION_DENIED","data":[]}})));}let key=p[1].as_str().unwrap().strip_prefix("OpaqueRef:").unwrap();return Some(RpcReply::Json(json!({"result":map.lock().unwrap().get(key).cloned().unwrap_or(Value::Null)})));}
        if method=="VM.assert_operation_valid"{return Some(RpcReply::Json(json!({"result":null})));}
        if method.starts_with("Async.VM_appliance.")||method.starts_with("VM_appliance.set_")||["VM.set_appliance","VM.set_order","VM.set_start_delay","VM.set_shutdown_delay"].contains(&method){
            let count=writes.fetch_add(1,Ordering::SeqCst);
            if fault==Fault::SecondField && count==1{return Some(RpcReply::Json(json!({"error":{"message":"RBAC_PERMISSION_DENIED","data":[]}})));}
            let mut records=map.lock().unwrap();
            match method{
                "Async.VM_appliance.create"=>{let mut g=group(NEW);g["name_label"]=p[1]["name_label"].clone();g["name_description"]=p[1]["name_description"].clone();records.insert("app-new".into(),g);}
                "Async.VM_appliance.destroy"=>{assert_eq!(records["app"]["VMs"],json!([]));records.remove("app");}
                "Async.VM_appliance.start"=>{assert_eq!(p[2],false);if fault!=Fault::UnchangedChild {records.get_mut("vm").unwrap()["power_state"]=json!("Running");}}
                "Async.VM_appliance.clean_shutdown"|"Async.VM_appliance.hard_shutdown"=>{records.get_mut("vm").unwrap()["power_state"]=json!("Halted");}
                "VM.set_appliance"=>{let join=p[2]!="OpaqueRef:NULL";records.get_mut("vm").unwrap()["appliance"]=p[2].clone();records.get_mut("app").unwrap()["VMs"]=if join{json!(["OpaqueRef:vm"])}else{json!([])};}
                _=>{let (class,field)=method.split_once(".set_").unwrap();let key=if class=="VM"{"vm"}else{"app"};records.get_mut(key).unwrap()[field]=p[2].clone();}
            }
            return Some(if fault==Fault::DropReply {RpcReply::Drop}else{RpcReply::Json(json!({"result":if method.starts_with("Async."){json!("OpaqueRef:appliance-task")}else{Value::Null}}))});
        }
        if method=="task.get_record"{return Some(RpcReply::Json(json!({"result":{"status":match fault{Fault::TaskFailure=>"failure",Fault::TaskCancelled=>"cancelled",_=>"success"},"allowed_operations":[],"progress":"1","error_info":["OPERATION_PARTIALLY_FAILED","private-value"],"result":"<value><string>OpaqueRef:app-new</string></value>"}})));}
        None
    }})).await;
    (server, records)
}
async fn connect(mock: &Mock, backend: &Backend, readonly: bool) -> xenadmin_core::Connection {
    let mut opts = ConnectionOptions::new(mock.endpoint.clone());
    opts.read_only = readonly;
    let c = backend.connect(
        opts,
        Credentials::new("fixture".into(), "fixture-password".into()),
    );
    c.wait_ready(deadline(3)).await.unwrap();
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
fn submit(c: &xenadmin_core::Connection, operation: Op) -> ResourceOperation {
    c.submit_resource(
        OperationId::new(),
        ResourceCommand::Appliance {
            pool: c
                .latest()
                .observe_resource(ResourceTarget::Pool(PoolId::parse(POOL).unwrap()))
                .unwrap(),
            operation,
        },
    )
    .unwrap()
}
fn power(action: AppliancePower, state: PowerState) -> Op {
    Op::Power {
        appliance: id(),
        members: vec![ApplianceMember {
            vm: vmid(),
            state,
            order: order(),
        }],
        action,
    }
}
fn add_member(records: &Records) {
    let mut r = records.lock().unwrap();
    r.get_mut("app").unwrap()["VMs"] = json!(["OpaqueRef:vm"]);
    r.get_mut("vm").unwrap()["appliance"] = json!("OpaqueRef:app");
}
fn mutations(m: &Mock) -> usize {
    m.methods
        .lock()
        .unwrap()
        .iter()
        .filter(|s| s.starts_with("Async.VM_appliance.") || s.contains(".set_"))
        .count()
}
#[tokio::test]
async fn appliance_create_proves_new_identity_and_public_history_has_no_references() {
    let (m, _) = fixture(Fault::None).await;
    let dir = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&m, &b, false).await;
    let r = finished(&submit(
        &c,
        Op::Create {
            name: "New group".into(),
            description: "Empty".into(),
        },
    ))
    .await;
    assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
    assert_eq!(
        r.created,
        Some(ResourceTarget::Appliance(ApplianceId::parse(NEW).unwrap()))
    );
    assert!(!serde_json::to_string(&*r).unwrap().contains("OpaqueRef"));
}
#[tokio::test]
async fn appliance_membership_order_and_empty_delete_never_delete_guests() {
    let (m, records) = fixture(Fault::None).await;
    let dir = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&m, &b, false).await;
    for op in [
        Op::Membership {
            appliance: id(),
            expected_members: vec![],
            vm: vmid(),
            join: true,
        },
        Op::SetOrder {
            appliance: id(),
            vm: vmid(),
            previous: order(),
            value: ApplianceOrder {
                order: 2,
                start_delay: 4,
                shutdown_delay: 6,
            },
        },
        Op::Membership {
            appliance: id(),
            expected_members: vec![vmid()],
            vm: vmid(),
            join: false,
        },
        Op::DeleteEmpty {
            appliance: id(),
            expected_name: "Application".into(),
        },
    ] {
        let r = finished(&submit(&c, op)).await;
        assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
    }
    assert!(records.lock().unwrap().contains_key("vm"));
    assert!(!m.methods.lock().unwrap().iter().any(|m| m == "VM.destroy"));
}
#[tokio::test]
async fn appliance_power_runs_reviewed_server_group_and_verifies_members() {
    let (m, records) = fixture(Fault::None).await;
    add_member(&records);
    let dir = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&m, &b, false).await;
    for (action, state) in [
        (AppliancePower::Start, PowerState::Halted),
        (AppliancePower::CleanShutdown, PowerState::Running),
    ] {
        let r = finished(&submit(&c, power(action, state))).await;
        assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
    }
    assert!(
        !m.methods
            .lock()
            .unwrap()
            .iter()
            .any(|m| m == "Async.VM_appliance.shutdown" || m == "Async.VM_appliance.hard_shutdown")
    );
}
#[tokio::test]
async fn appliance_refuses_changed_order_membership_and_nonempty_deletion() {
    for case in 0..4 {
        let (m, records) = fixture(Fault::None).await;
        add_member(&records);
        let op = match case {
            0 => {
                records.lock().unwrap().get_mut("vm").unwrap()["order"] = json!("9");
                power(AppliancePower::Start, PowerState::Halted)
            }
            1 => Op::Membership {
                appliance: id(),
                expected_members: vec![],
                vm: vmid(),
                join: true,
            },
            2 => Op::DeleteEmpty {
                appliance: id(),
                expected_name: "Application".into(),
            },
            _ => {
                records.lock().unwrap().get_mut("vm").unwrap()["is_a_snapshot"] = json!(true);
                power(AppliancePower::Start, PowerState::Halted)
            }
        };
        let dir = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&m, &b, false).await;
        let r = finished(&submit(&c, op)).await;
        assert_eq!(
            r.state,
            OperationState::Rejected,
            "case {case}: {}",
            r.detail
        );
        assert_eq!(mutations(&m), 0);
    }
}
#[tokio::test]
async fn appliance_failed_or_cancelled_group_retains_pool_guard_and_no_force_fallback() {
    for fault in [Fault::TaskFailure, Fault::TaskCancelled] {
        let (m, records) = fixture(fault).await;
        add_member(&records);
        let dir = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&m, &b, false).await;
        let r = finished(&submit(
            &c,
            power(AppliancePower::Start, PowerState::Halted),
        ))
        .await;
        assert_eq!(r.state, OperationState::OutcomeUnknown);
        let other = finished(&submit(
            &c,
            Op::Create {
                name: "Other".into(),
                description: "".into(),
            },
        ))
        .await;
        assert_eq!(other.state, OperationState::Rejected);
        assert_eq!(mutations(&m), 1);
        assert!(
            !serde_json::to_string(&*r)
                .unwrap()
                .contains("private-value")
        );
    }
}
#[tokio::test]
async fn appliance_lost_task_reply_cannot_adopt_matching_created_group() {
    let (m, _) = fixture(Fault::DropReply).await;
    let dir = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&m, &b, false).await;
    let r = finished(&submit(
        &c,
        Op::Create {
            name: "Application".into(),
            description: "Example".into(),
        },
    ))
    .await;
    assert_eq!(r.state, OperationState::OutcomeUnknown);
    c.control_resource(r.id, OperationControl::Recheck).unwrap();
    tokio::time::sleep(Duration::from_millis(120)).await;
    assert_eq!(mutations(&m), 1);
    assert!(
        !m.methods
            .lock()
            .unwrap()
            .iter()
            .any(|m| m == "task.destroy")
    );
}
#[tokio::test]
async fn appliance_order_partial_failure_keeps_first_field_and_never_replays() {
    let (m, records) = fixture(Fault::SecondField).await;
    add_member(&records);
    let dir = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&m, &b, false).await;
    let r = finished(&submit(
        &c,
        Op::SetOrder {
            appliance: id(),
            vm: vmid(),
            previous: order(),
            value: ApplianceOrder {
                order: 2,
                start_delay: 4,
                shutdown_delay: 6,
            },
        },
    ))
    .await;
    assert_eq!(r.state, OperationState::OutcomeUnknown);
    assert_eq!(records.lock().unwrap()["vm"]["order"], "2");
    assert_eq!(records.lock().unwrap()["vm"]["shutdown_delay"], "3");
    assert_eq!(mutations(&m), 2);
}
#[tokio::test]
async fn appliance_readonly_refuses_mutation_and_missing_membership_remains_unknown() {
    let (m, records) = fixture(Fault::None).await;
    records.lock().unwrap().get_mut("app").unwrap()["VMs"] = json!(["OpaqueRef:missing"]);
    let dir = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&m, &b, true).await;
    assert!(!c.latest().inventory().unwrap().details.appliances[&id()].vms_known);
    assert!(
        c.submit_resource(
            OperationId::new(),
            ResourceCommand::Appliance {
                pool: c
                    .latest()
                    .observe_resource(ResourceTarget::Pool(PoolId::parse(POOL).unwrap()))
                    .unwrap(),
                operation: Op::Create {
                    name: "Example".into(),
                    description: "".into()
                }
            }
        )
        .is_err()
    );
    assert_eq!(mutations(&m), 0);
}

#[tokio::test]
async fn appliance_saved_completion_receipt_rechecks_without_task_or_mutation_replay() {
    let (mock, _) = fixture(Fault::VerifyOnce).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let connection = connect(&mock, &backend, false).await;
    let handle = submit(
        &connection,
        Op::Create {
            name: "New group".into(),
            description: "".into(),
        },
    );
    let first = finished(&handle).await;
    assert_eq!(
        first.state,
        OperationState::OutcomeUnknown,
        "{}",
        first.detail
    );
    connection
        .control_resource(first.id, OperationControl::Recheck)
        .unwrap();
    tokio::time::timeout(deadline(3), async {
        loop {
            if connection
                .resource_history()
                .iter()
                .any(|r| r.id == first.id && r.state == OperationState::Succeeded)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(mutations(&mock), 1);
    assert_eq!(
        mock.methods
            .lock()
            .unwrap()
            .iter()
            .filter(|m| *m == "task.get_record")
            .count(),
        1
    );
}

#[tokio::test]
async fn appliance_successful_parent_with_unchanged_child_retains_reservation() {
    let (mock, records) = fixture(Fault::UnchangedChild).await;
    add_member(&records);
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let connection = connect(&mock, &backend, false).await;
    let result = finished(&submit(
        &connection,
        power(AppliancePower::Start, PowerState::Halted),
    ))
    .await;
    assert_eq!(result.state, OperationState::OutcomeUnknown);
    let other = finished(&submit(
        &connection,
        Op::Create {
            name: "Other".into(),
            description: "".into(),
        },
    ))
    .await;
    assert_eq!(other.state, OperationState::Rejected);
    assert_eq!(mutations(&mock), 1);
    assert!(
        !mock
            .methods
            .lock()
            .unwrap()
            .iter()
            .any(|m| m == "task.destroy" || m == "Async.VM_appliance.hard_shutdown")
    );
}
