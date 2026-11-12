use super::*;
use crate::{
    ForceRecoveryAction, RecoveryBootConfiguration, RecoveryTask, TaskId,
    xapi::inventory_tests::Fixture,
};
use serde_json::Value;
use std::{collections::BTreeMap, sync::Mutex};
type Records = Arc<Mutex<BTreeMap<String, Value>>>;
type Calls = Arc<Mutex<Vec<Value>>>;
fn uuid(n: u128) -> String {
    Uuid::from_u128(n).to_string()
}
fn vm() -> VmId {
    VmId::parse(&uuid(2)).unwrap()
}
fn pool() -> PoolId {
    PoolId::parse(&uuid(1)).unwrap()
}
fn config(order: Option<&str>) -> RecoveryBootConfiguration {
    RecoveryBootConfiguration {
        order: order.map(str::to_owned),
        firmware: "uefi".into(),
        secure_boot: Some(true),
    }
}
fn boot(order: Option<&str>) -> VmRecoveryOperation {
    VmRecoveryOperation::Boot {
        configuration: config(order),
    }
}
fn force(action: ForceRecoveryAction) -> VmRecoveryOperation {
    VmRecoveryOperation::Force {
        action,
        tasks: vec![RecoveryTask {
            task: TaskId::parse(&uuid(40)).unwrap(),
            operation: "export".into(),
        }],
        interrupts_migration: false,
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Fault {
    None,
    LostOverride,
    LostStart,
    LostRestore,
    RefusedStart,
    FailedTask,
    ExistingTask,
    ReboundTask,
    ChangedOrder,
    ChangedTasks,
    HaltedDuringRead,
    Blocked,
    RecoverStart,
    AckStart,
    RecoverSetter,
    AckSetter,
    RecoverVerification,
}
async fn fixture(
    running: bool,
    order: Option<&str>,
    fault: Fault,
    directory: Option<PathBuf>,
) -> (Fixture, Records, Calls) {
    let mut params = json!({"firmware":"uefi","unrelated":"retained"});
    if let Some(order) = order {
        params["order"] = json!(order)
    }
    let records: Records = Arc::new(Mutex::new(BTreeMap::from([
        (
            "OpaqueRef:VM:guest".into(),
            json!({"uuid":uuid(2),"name_label":"Synthetic guest","is_a_template":false,"is_a_snapshot":false,"is_control_domain":false,"power_state":if running{"Running"}else{"Halted"},"domain_type":"hvm","HVM_boot_policy":"BIOS order","HVM_boot_params":params,"platform":{"secureboot":"true"},"current_operations":if running{json!({"OpaqueRef:task:existing":"export"})}else{json!({})},"blocked_operations":if fault==Fault::Blocked{json!({"hard_shutdown":"Deliberate administrative restriction"})}else{json!({})},"allowed_operations":if running{json!([])}else{json!(["start"])} }),
        ),
        (
            "OpaqueRef:task:existing".into(),
            json!({"uuid":uuid(40),"status":"pending"}),
        ),
    ])));
    let calls: Calls = Arc::new(Mutex::new(vec![]));
    let f=Fixture::new({let records=records.clone();let calls=calls.clone();move|request|{
        calls.lock().unwrap().push(request.clone());let method=request["method"].as_str().unwrap();let mut records=records.lock().unwrap();
        let recover=|ack:bool|{let path=directory.as_ref().unwrap();let rows=super::super::recover(path,pool()).unwrap();if ack{Journal::open(path).unwrap().control(pool(),rows[0].id,true).unwrap();}};
        match method{
            "VM.get_by_uuid"=>json!({"result":"OpaqueRef:VM:guest"}),
            "VM.get_record"=>{let count=calls.lock().unwrap().iter().filter(|c|c["method"]=="VM.get_record").count();if fault==Fault::HaltedDuringRead&&count>=3{records.get_mut("OpaqueRef:VM:guest").unwrap()["power_state"]=json!("Halted");}
        if fault==Fault::RecoverVerification&&records["OpaqueRef:VM:guest"]["power_state"]=="Running"{recover(false);}json!({"result":records["OpaqueRef:VM:guest"]})},
            "task.get_all_records"=>json!({"result":records.iter().filter(|(k,_)|k.starts_with("OpaqueRef:task:")).collect::<BTreeMap<_,_>>()}),
            "task.get_record"=>{let reference=request["params"][1].as_str().unwrap();if reference=="OpaqueRef:task:existing"&&fault==Fault::ChangedTasks{return json!({"result":{"uuid":uuid(41),"status":"pending"}})}
                if reference=="OpaqueRef:task:new"&&fault==Fault::ReboundTask{let count=calls.lock().unwrap().iter().filter(|c|c["method"]=="task.get_record"&&c["params"][1]==reference).count();return match count{1=>json!({"result":{"uuid":uuid(50),"status":"pending"}}),2=>json!({"_fixture_close_without_reply":true}),_=>json!({"result":{"uuid":uuid(51),"status":"success"}})}}
                records.get(reference).map_or_else(||json!({"error":{"message":"HANDLE_INVALID"}}),|r|json!({"result":r}))},
            "VM.assert_operation_valid"=>json!({"result":null}),
            "VM.set_HVM_boot_params"=>{let is_override=request["params"][2]["order"]=="dn";records.get_mut("OpaqueRef:VM:guest").unwrap()["HVM_boot_params"]=request["params"][2].clone();if is_override&&matches!(fault,Fault::RecoverSetter|Fault::AckSetter){recover(fault==Fault::AckSetter)}
        if (is_override&&fault==Fault::LostOverride)||(!is_override&&fault==Fault::LostRestore){json!({"_fixture_close_without_reply":true})}else{json!({"result":null})}},
            "Async.VM.start"|"Async.VM.hard_shutdown"|"Async.VM.hard_reboot"=>{
                assert_eq!(request["params"][1],"OpaqueRef:VM:guest");if method=="Async.VM.start"{assert_eq!(request["params"][2],false);assert_eq!(request["params"][3],false);if fault==Fault::RefusedStart{return json!({"error":{"message":"VM_BAD_POWER_STATE"}})}}
                let r=records.get_mut("OpaqueRef:VM:guest").unwrap();r["power_state"]=json!(if method.ends_with("hard_shutdown"){"Halted"}else{"Running"});r["current_operations"]=json!({});
                if fault==Fault::ChangedOrder{r["HVM_boot_params"]["order"]=json!("nc");}
                records.insert("OpaqueRef:task:new".into(),json!({"uuid":uuid(if fault==Fault::ExistingTask{40}else{50}),"status":if fault==Fault::FailedTask{"failure"}else{"success"}}));
                if matches!(fault,Fault::RecoverStart|Fault::AckStart){recover(fault==Fault::AckStart)}
                if fault==Fault::LostStart{json!({"_fixture_close_without_reply":true})}else{json!({"result":"OpaqueRef:task:new"})}
            },
            _=>panic!("Unexpected synthetic RPC {method}"),
        }
    }}).await;
    (f, records, calls)
}
async fn execute(f: &Fixture, path: &Path, operation: VmRecoveryOperation) -> OperationRecord {
    let (handle, request) = channel(
        OperationId::new(),
        pool(),
        vm(),
        1,
        OperationAction::Recovery(operation),
        tokio::runtime::Handle::current(),
    );
    let (tx, _rx) = mpsc::channel(128);
    tokio::time::timeout(
        Duration::from_secs(5),
        request.run(
            Arc::new(f.session().await),
            Some("OpaqueRef:VM:guest".into()),
            path.into(),
            CancellationToken::new(),
            tx,
        ),
    )
    .await
    .unwrap();
    (*handle.latest()).clone()
}
async fn recheck(f: &Fixture, path: &Path, r: OperationRecord) -> OperationRecord {
    let id = r.id;
    let (tx, _rx) = mpsc::channel(128);
    MutationRequest::tracking(r)
        .run(
            Arc::new(f.session().await),
            None,
            path.into(),
            CancellationToken::new(),
            tx,
        )
        .await;
    Journal::open(path).unwrap().load(id).unwrap().unwrap()
}
fn count(c: &Calls, method: &str) -> usize {
    c.lock()
        .unwrap()
        .iter()
        .filter(|c| c["method"] == method)
        .count()
}
#[tokio::test]
async fn force_with_other_client_tasks_uses_fresh_assert_without_cancelling_or_resetting() {
    for action in [ForceRecoveryAction::Shutdown, ForceRecoveryAction::Reboot] {
        let (f, _, calls) = fixture(true, Some("cdn"), Fault::None, None).await;
        let d = tempfile::tempdir().unwrap();
        let r = execute(&f, d.path(), force(action)).await;
        assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
        assert_eq!(count(&calls, "VM.assert_operation_valid"), 1);
        assert_eq!(count(&calls, &format!("Async.VM.{}", action.wire())), 1);
        assert_eq!(count(&calls, "task.cancel"), 0);
        assert_eq!(count(&calls, "VM.power_state_reset"), 0);
    }
}
#[tokio::test]
async fn force_refuses_admin_blocks_task_rebinding_and_halted_reboot() {
    for fault in [Fault::Blocked, Fault::ChangedTasks, Fault::HaltedDuringRead] {
        let (f, _, calls) = fixture(true, Some("cdn"), fault, None).await;
        let d = tempfile::tempdir().unwrap();
        let r = execute(&f, d.path(), force(ForceRecoveryAction::Shutdown)).await;
        assert!(
            matches!(r.state, OperationState::Failed | OperationState::Rejected),
            "{:?}: {}",
            r.state,
            r.detail
        );
        assert_eq!(count(&calls, "Async.VM.hard_shutdown"), 0);
    }
}
/// A forced action whose captured task is of the given kind, accepted or not.
fn force_over(operation: &str, accepted: bool) -> VmRecoveryOperation {
    VmRecoveryOperation::Force {
        action: ForceRecoveryAction::Shutdown,
        tasks: vec![RecoveryTask {
            task: TaskId::parse(&uuid(40)).unwrap(),
            operation: operation.into(),
        }],
        interrupts_migration: accepted,
    }
}
#[tokio::test]
async fn a_forced_action_during_a_migration_is_refused_until_the_operator_accepts_it() {
    for kind in ["migrate_send", "pool_migrate"] {
        // Not accepted: refused in its own words, and nothing is asked of the host.
        let (f, records, calls) = fixture(true, Some("cdn"), Fault::None, None).await;
        records
            .lock()
            .unwrap()
            .get_mut("OpaqueRef:VM:guest")
            .unwrap()["current_operations"] = json!({"OpaqueRef:task:existing": kind});
        let d = tempfile::tempdir().unwrap();
        let r = execute(&f, d.path(), force_over(kind, false)).await;
        assert!(
            matches!(r.state, OperationState::Failed | OperationState::Rejected),
            "{kind}: {:?}: {}",
            r.state,
            r.detail
        );
        assert!(
            r.detail.contains("A migration of this VM is running"),
            "{}",
            r.detail
        );
        assert_eq!(count(&calls, "Async.VM.hard_shutdown"), 0, "{kind}");
        assert_eq!(count(&calls, "VM.assert_operation_valid"), 0, "{kind}");
        assert_eq!(count(&calls, "VM.get_record"), 0, "{kind}");
        // Accepted: the forced action goes ahead as for any other captured task, and
        // still cancels nobody's task.
        let (f, records, calls) = fixture(true, Some("cdn"), Fault::None, None).await;
        records
            .lock()
            .unwrap()
            .get_mut("OpaqueRef:VM:guest")
            .unwrap()["current_operations"] = json!({"OpaqueRef:task:existing": kind});
        let d = tempfile::tempdir().unwrap();
        let r = execute(&f, d.path(), force_over(kind, true)).await;
        assert_eq!(r.state, OperationState::Succeeded, "{kind}: {}", r.detail);
        assert_eq!(count(&calls, "Async.VM.hard_shutdown"), 1, "{kind}");
        assert_eq!(count(&calls, "task.cancel"), 0, "{kind}");
    }
}
#[tokio::test]
async fn an_acceptance_is_recorded_only_where_a_migration_is_captured() {
    // An acceptance with no migration among the tasks is not a reviewed operation.
    let (f, _, calls) = fixture(true, Some("cdn"), Fault::None, None).await;
    let d = tempfile::tempdir().unwrap();
    let r = execute(&f, d.path(), force_over("export", true)).await;
    assert!(
        matches!(r.state, OperationState::Failed | OperationState::Rejected),
        "{:?}: {}",
        r.state,
        r.detail
    );
    assert_eq!(count(&calls, "Async.VM.hard_shutdown"), 0);
    assert!(force_over("migrate_send", false).migration_unaccepted());
    assert!(!force_over("migrate_send", true).migration_unaccepted());
    assert!(!force_over("export", false).migration_unaccepted());
    // The stored form of an operation without the acceptance is what it was before the
    // acceptance existed, and a row written then reads as not accepted.
    let plain = serde_json::to_value(force_over("export", false)).unwrap();
    assert!(plain.get("interrupts_migration").is_none(), "{plain}");
    assert_eq!(
        serde_json::from_value::<VmRecoveryOperation>(plain).unwrap(),
        force_over("export", false)
    );
    let accepted = serde_json::to_value(force_over("migrate_send", true)).unwrap();
    assert_eq!(accepted["interrupts_migration"], true);
}
#[tokio::test]
async fn force_keeps_unresolved_local_reservations() {
    let (f, _, c) = fixture(true, Some("cdn"), Fault::None, None).await;
    let d = tempfile::tempdir().unwrap();
    let (_, mut request) = channel(
        OperationId::new(),
        pool(),
        vm(),
        1,
        OperationAction::Power(PowerAction::Reboot),
        tokio::runtime::Handle::current(),
    );
    Journal::open(d.path())
        .unwrap()
        .admit(&mut request.record)
        .unwrap();
    let r = execute(&f, d.path(), force(ForceRecoveryAction::Shutdown)).await;
    assert_eq!(r.state, OperationState::Rejected);
    assert_eq!(count(&c, "Async.VM.hard_shutdown"), 0);
}
#[tokio::test]
async fn recovery_boot_restores_exact_order_or_absence_and_preserves_firmware() {
    for order in [None, Some("cdn"), Some("dn")] {
        let (f, records, c) = fixture(false, order, Fault::None, None).await;
        let d = tempfile::tempdir().unwrap();
        let r = execute(&f, d.path(), boot(order)).await;
        assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
        let records = records.lock().unwrap();
        let vm = &records["OpaqueRef:VM:guest"];
        assert_eq!(
            vm["HVM_boot_params"].get("order").and_then(Value::as_str),
            order
        );
        assert_eq!(vm["HVM_boot_params"]["firmware"], "uefi");
        assert_eq!(vm["HVM_boot_params"]["unrelated"], "retained");
        assert_eq!(vm["platform"]["secureboot"], "true");
        assert_eq!(count(&c, "Async.VM.start"), 1);
        assert_eq!(
            count(&c, "VM.set_HVM_boot_params"),
            if order == Some("dn") { 0 } else { 2 }
        );
        assert_eq!(count(&c, "VM.set_HVM_boot_policy"), 0);
    }
}
#[tokio::test]
async fn recovery_boot_refuses_pv_or_changed_firmware() {
    for (field, value) in [
        ("domain_type", json!("pv")),
        ("HVM_boot_policy", json!("")),
        ("power_state", json!("Running")),
    ] {
        let (f, records, c) = fixture(false, Some("cdn"), Fault::None, None).await;
        records
            .lock()
            .unwrap()
            .get_mut("OpaqueRef:VM:guest")
            .unwrap()[field] = value;
        let d = tempfile::tempdir().unwrap();
        let r = execute(&f, d.path(), boot(Some("cdn"))).await;
        assert_eq!(r.state, OperationState::Rejected);
        assert_eq!(count(&c, "VM.set_HVM_boot_params"), 0);
        assert_eq!(count(&c, "Async.VM.start"), 0);
    }
}
#[tokio::test]
async fn lost_boot_write_is_unknown_and_recheck_never_replays() {
    for fault in [
        Fault::LostOverride,
        Fault::LostStart,
        Fault::LostRestore,
        Fault::FailedTask,
        Fault::ExistingTask,
        Fault::ReboundTask,
    ] {
        let (f, _, calls) = fixture(false, Some("cdn"), fault, None).await;
        let d = tempfile::tempdir().unwrap();
        let r = execute(&f, d.path(), boot(Some("cdn"))).await;
        assert_eq!(r.state, OperationState::OutcomeUnknown, "{}", r.detail);
        let writes = count(&calls, "VM.set_HVM_boot_params");
        let starts = count(&calls, "Async.VM.start");
        let checked = recheck(&f, d.path(), r).await;
        assert_eq!(
            checked.state,
            OperationState::OutcomeUnknown,
            "{}",
            checked.detail
        );
        assert_eq!(count(&calls, "VM.set_HVM_boot_params"), writes);
        assert_eq!(count(&calls, "Async.VM.start"), starts);
    }
}
#[tokio::test]
async fn concurrent_order_change_is_not_overwritten() {
    let (f, records, c) = fixture(false, Some("cdn"), Fault::ChangedOrder, None).await;
    let d = tempfile::tempdir().unwrap();
    let r = execute(&f, d.path(), boot(Some("cdn"))).await;
    assert_eq!(r.state, OperationState::OutcomeUnknown);
    assert_eq!(count(&c, "VM.set_HVM_boot_params"), 1);
    assert_eq!(
        records.lock().unwrap()["OpaqueRef:VM:guest"]["HVM_boot_params"]["order"],
        "nc"
    );
}
#[tokio::test]
async fn late_receipts_preserve_recovered_or_acknowledged_outcome_and_stop_later_steps() {
    for fault in [
        Fault::RecoverStart,
        Fault::AckStart,
        Fault::RecoverSetter,
        Fault::AckSetter,
        Fault::RecoverVerification,
    ] {
        let d = tempfile::tempdir().unwrap();
        let (f, _, c) = fixture(false, Some("cdn"), fault, Some(d.path().into())).await;
        let r = execute(&f, d.path(), boot(Some("cdn"))).await;
        assert_eq!(
            r.state,
            if matches!(fault, Fault::AckStart | Fault::AckSetter) {
                OperationState::AcknowledgedUnknown
            } else {
                OperationState::OutcomeUnknown
            },
            "{}",
            r.detail
        );
        assert_eq!(count(&c, "VM.set_HVM_boot_params"), 1);
        assert_eq!(
            count(&c, "Async.VM.start"),
            if matches!(fault, Fault::RecoverSetter | Fault::AckSetter) {
                0
            } else {
                1
            }
        );
        let p = r.recovery_plan.unwrap();
        if matches!(fault, Fault::RecoverSetter | Fault::AckSetter) {
            assert!(p.evidence[0].completed)
        } else {
            assert!(p.evidence[1].task.is_some())
        }
    }
}
#[tokio::test]
async fn separate_restoration_requires_terminal_source_and_proven_override() {
    let d = tempfile::tempdir().unwrap();
    let (f, records, c) = fixture(false, None, Fault::LostStart, None).await;
    let original = execute(&f, d.path(), boot(None)).await;
    let restore = VmRecoveryOperation::RestoreBoot {
        source: original.id,
        configuration: config(None),
    };
    let blocked = execute(&f, d.path(), restore.clone()).await;
    assert_eq!(blocked.state, OperationState::Rejected);
    Journal::open(d.path())
        .unwrap()
        .control(pool(), original.id, true)
        .unwrap();
    let r = execute(&f, d.path(), restore).await;
    assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
    assert!(
        records.lock().unwrap()["OpaqueRef:VM:guest"]["HVM_boot_params"]
            .get("order")
            .is_none()
    );
    assert_eq!(count(&c, "Async.VM.start"), 1);
}
#[tokio::test]
async fn unreceipted_override_cannot_be_claimed_for_restoration() {
    let d = tempfile::tempdir().unwrap();
    let (f, _, c) = fixture(false, Some("cdn"), Fault::LostOverride, None).await;
    let original = execute(&f, d.path(), boot(Some("cdn"))).await;
    Journal::open(d.path())
        .unwrap()
        .control(pool(), original.id, true)
        .unwrap();
    let r = execute(
        &f,
        d.path(),
        VmRecoveryOperation::RestoreBoot {
            source: original.id,
            configuration: config(Some("cdn")),
        },
    )
    .await;
    assert_eq!(r.state, OperationState::Rejected);
    assert_eq!(count(&c, "VM.set_HVM_boot_params"), 1);
}
#[tokio::test]
async fn known_start_refusal_retains_reviewable_override_as_partial() {
    let d = tempfile::tempdir().unwrap();
    let (f, _, c) = fixture(false, Some("cdn"), Fault::RefusedStart, None).await;
    let r = execute(&f, d.path(), boot(Some("cdn"))).await;
    assert_eq!(r.state, OperationState::PartiallyCompleted);
    assert_eq!(count(&c, "VM.set_HVM_boot_params"), 1);
    let restored = execute(
        &f,
        d.path(),
        VmRecoveryOperation::RestoreBoot {
            source: r.id,
            configuration: config(Some("cdn")),
        },
    )
    .await;
    assert_eq!(restored.state, OperationState::Succeeded);
}
#[tokio::test]
async fn recovery_plan_is_private_immutable_and_bound_to_public_action() {
    let d = tempfile::tempdir().unwrap();
    let (f, _, _) = fixture(false, Some("cdn"), Fault::None, None).await;
    let r = execute(&f, d.path(), boot(Some("cdn"))).await;
    let public = serde_json::to_string(&r).unwrap();
    assert!(!public.contains("OpaqueRef:"));
    assert!(!public.contains("recovery_plan"));
    let private = Journal::encode_record(&r).unwrap();
    let loaded = Journal::decode_record(&private).unwrap();
    assert_eq!(loaded.recovery_plan, r.recovery_plan);
    let mut bad = r.clone();
    bad.recovery_plan
        .as_mut()
        .unwrap()
        .baseline
        .push(TaskId::parse(&uuid(60)).unwrap());
    assert!(Journal::open(d.path()).unwrap().save(&mut bad).is_err());
    let mut value: Value = serde_json::from_str(&private).unwrap();
    value["recovery_plan"]["operation"] =
        serde_json::to_value(force(ForceRecoveryAction::Shutdown)).unwrap();
    assert!(Journal::decode_record(&value.to_string()).is_err());
}
#[tokio::test]
async fn corrupt_public_steps_or_impossible_private_receipts_fail_closed() {
    let d = tempfile::tempdir().unwrap();
    let (f, _, _) = fixture(false, Some("cdn"), Fault::None, None).await;
    let record = execute(&f, d.path(), boot(Some("cdn"))).await;
    let original: Value = serde_json::from_str(&Journal::encode_record(&record).unwrap()).unwrap();
    for mutation in 0..5 {
        let mut v = original.clone();
        match mutation {
            0 => v["steps"] = json!([]),
            1 => v["steps"][0]["label"] = json!("Wrong target step"),
            2 => v["recovery_plan"]["evidence"][0]["completed"] = json!(false),
            3 => {
                v["steps"][2]["state"] = json!("Pending");
                v["recovery_plan"]["evidence"][2]["completed"] = json!(false)
            }
            _ => v["recovery_plan"]["evidence"][1]["task_id"] = Value::Null,
        }
        assert!(
            Journal::decode_record(&v.to_string()).is_err(),
            "mutation {mutation}"
        );
    }
}
#[test]
fn recovery_requirements_include_start_and_both_order_writes_before_first_change() {
    let perms = crate::SessionPermissions::decode(
        &json!({"is_local_superuser":false,"rbac_permissions":["vm.start"]}),
    );
    assert!(
        boot(Some("cdn"))
            .permission_requirements()
            .assess(&perms)
            .decision()
            .is_denied()
    );
    assert!(
        !boot(Some("dn"))
            .permission_requirements()
            .assess(&perms)
            .decision()
            .is_denied()
    );
    assert!(
        force(ForceRecoveryAction::Shutdown)
            .permission_requirements()
            .assess(&perms)
            .decision()
            .is_denied()
    );
}
#[tokio::test]
async fn already_restored_source_cannot_claim_a_later_matching_order() {
    let d = tempfile::tempdir().unwrap();
    let (f, records, c) = fixture(false, Some("cdn"), Fault::None, None).await;
    let source = execute(&f, d.path(), boot(Some("cdn"))).await;
    assert_eq!(source.state, OperationState::Succeeded);
    assert!(source.recovery_boot_restoration().is_none());
    records
        .lock()
        .unwrap()
        .get_mut("OpaqueRef:VM:guest")
        .unwrap()["HVM_boot_params"]["order"] = json!("dn");
    let denied = execute(
        &f,
        d.path(),
        VmRecoveryOperation::RestoreBoot {
            source: source.id,
            configuration: config(Some("cdn")),
        },
    )
    .await;
    assert_eq!(denied.state, OperationState::Rejected);
    assert_eq!(count(&c, "VM.set_HVM_boot_params"), 2);
}
#[tokio::test]
async fn separate_restoration_can_only_claim_source_once() {
    let d = tempfile::tempdir().unwrap();
    let (f, records, c) = fixture(false, Some("cdn"), Fault::RefusedStart, None).await;
    let source = execute(&f, d.path(), boot(Some("cdn"))).await;
    let op = source.recovery_boot_restoration().unwrap();
    let restored = execute(&f, d.path(), op.clone()).await;
    assert_eq!(restored.state, OperationState::Succeeded);
    records
        .lock()
        .unwrap()
        .get_mut("OpaqueRef:VM:guest")
        .unwrap()["HVM_boot_params"]["order"] = json!("dn");
    let denied = execute(&f, d.path(), op).await;
    assert_eq!(denied.state, OperationState::Rejected);
    assert_eq!(count(&c, "VM.set_HVM_boot_params"), 2);
}
