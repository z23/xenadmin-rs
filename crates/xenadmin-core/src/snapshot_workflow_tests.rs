use super::*;
use crate::{PowerState, SnapshotId, SnapshotWorkflow, xapi::inventory_tests::Fixture};
use serde_json::Value;
use std::{collections::BTreeMap, sync::Mutex};

/// Fixture waits only poll for an outcome; `XENADMIN_TEST_TIME_SCALE` stretches them on
/// slow or loaded runners the same way the live-lab fixtures stretch theirs.
fn scaled_deadline(seconds: u64) -> Duration {
    let scale = std::env::var("XENADMIN_TEST_TIME_SCALE")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| (1..=64).contains(v))
        .unwrap_or(1);
    Duration::from_secs(seconds * scale)
}
type Records = Arc<Mutex<BTreeMap<String, Value>>>;
type Calls = Arc<Mutex<Vec<Value>>>;
fn uuid(n: u128) -> String {
    Uuid::from_u128(n).to_string()
}
fn vm() -> VmId {
    VmId::parse(&uuid(2)).unwrap()
}
fn snapshot() -> SnapshotId {
    SnapshotId::parse(&uuid(3)).unwrap()
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Fault {
    None,
    AckCreate,
    AckShutdown,
    CancelShutdown,
    RecoverShutdown,
    LostShutdown,
    LostShutdownRead,
    LostRevertRead,
    LostRevert,
    FailedRevert,
    CancelledRevert,
    LostDescription,
    DeniedDescription,
    ExistingResult,
    ChangedParent,
    WrongRevertParent,
    SameMemory,
    NoStartup,
    LostStart,
    MetadataConflict,
}
async fn fixture(running: bool, checkpoint: bool, fault: Fault) -> (Fixture, Records, Calls) {
    fixture_with_journal(running, checkpoint, fault, None).await
}
async fn fixture_with_journal(
    running: bool,
    checkpoint: bool,
    fault: Fault,
    journal: Option<PathBuf>,
) -> (Fixture, Records, Calls) {
    let values = BTreeMap::from([
        (
            "OpaqueRef:VM:parent".into(),
            json!({"uuid":uuid(2),"name_label":"Guest","name_description":"Guest description","power_state":if running {"Running"}else{"Halted"},"is_a_snapshot":false,"is_a_template":false,"is_control_domain":false,"current_operations":{},"allowed_operations":["snapshot","checkpoint","clean_shutdown","start","resume"],"blocked_operations":{},"parent":"OpaqueRef:NULL"}),
        ),
        (
            "OpaqueRef:VM:snapshot".into(),
            json!({"uuid":uuid(3),"name_label":"Before update","name_description":"Original description","power_state":if checkpoint {"Suspended"}else{"Halted"},"is_a_snapshot":true,"is_a_template":true,"is_control_domain":false,"current_operations":{},"allowed_operations":["revert"],"blocked_operations":{},"snapshot_of":"OpaqueRef:VM:parent","suspend_VDI":if checkpoint {"OpaqueRef:VDI:memory"}else{"OpaqueRef:NULL"}}),
        ),
        (
            "OpaqueRef:VDI:memory".into(),
            json!({"uuid":uuid(4),"type":"suspend"}),
        ),
        (
            "OpaqueRef:VDI:copy".into(),
            json!({"uuid":uuid(5),"type":"suspend"}),
        ),
    ]);
    let records: Records = Arc::new(Mutex::new(values));
    let calls: Calls = Arc::new(Mutex::new(Vec::new()));
    let f=Fixture::new({let records=records.clone();let calls=calls.clone();move|request|{
        let method=request["method"].as_str().unwrap();calls.lock().unwrap().push(request.clone());let mut records=records.lock().unwrap();
        if method=="VM.get_by_uuid" {return records.iter().find(|(r,v)|r.starts_with("OpaqueRef:VM:")&&v["uuid"]==request["params"][1]).map_or_else(||json!({"error":{"message":"UUID_INVALID"}}),|(r,_)|json!({"result":r}));}
        if method=="VM.get_all_records" {return json!({"result":records.iter().filter(|(r,_)|r.starts_with("OpaqueRef:VM:")).collect::<BTreeMap<_,_>>()});}
        if method=="task.get_record" {
            let task=request["params"][1].as_str().unwrap();
            if ((task=="OpaqueRef:task:shutdown" && fault==Fault::LostShutdownRead) || (task=="OpaqueRef:task:revert" && fault==Fault::LostRevertRead)) && calls.lock().unwrap().iter().filter(|c|c["method"]=="task.get_record" && c["params"][1]==task).count()==1 {return json!({"_fixture_close_without_reply":true});}
            if task=="OpaqueRef:task:revert" && matches!(fault,Fault::FailedRevert|Fault::CancelledRevert){return json!({"result":{"status":if fault==Fault::FailedRevert {"failure"}else{"cancelled"}}});}
            return json!({"result":{"status":"success","result":format!("<value>{}</value>",if fault==Fault::ExistingResult {"OpaqueRef:VM:snapshot"}else{"OpaqueRef:VM:new"})}});
        }
        if method.ends_with(".get_record") {return records.get(request["params"][1].as_str().unwrap()).map_or_else(||json!({"error":{"message":"HANDLE_INVALID"}}),|v|json!({"result":v}));}
        match method {
            "VM.assert_operation_valid"=>json!({"result":null}),
            "Async.VM.clean_shutdown"=>{
                assert_eq!(request["params"][1],"OpaqueRef:VM:parent");records.get_mut("OpaqueRef:VM:parent").unwrap()["power_state"]=json!("Halted");
                if fault==Fault::ChangedParent {records.get_mut("OpaqueRef:VM:snapshot").unwrap()["snapshot_of"]=json!("OpaqueRef:VM:unrelated");}
                if fault==Fault::CancelShutdown {let path=journal.as_ref().unwrap();let j=Journal::open(path).unwrap();let record=j.history().unwrap().remove(0);j.control(record.pool,record.id,false).unwrap();}
                if matches!(fault,Fault::AckShutdown|Fault::RecoverShutdown){recover_during_dispatch(journal.as_ref().unwrap(),fault==Fault::AckShutdown);}
                if fault==Fault::LostShutdown {json!({"_fixture_close_without_reply":true})}else{json!({"result":"OpaqueRef:task:shutdown"})}
            }
            "Async.VM.revert"=>{
                assert_eq!(request["params"][1],"OpaqueRef:VM:snapshot");let guest=records.get_mut("OpaqueRef:VM:parent").unwrap();guest["power_state"]=json!(if checkpoint {"Suspended"}else{"Halted"});guest["parent"]=json!(if fault==Fault::WrongRevertParent {"OpaqueRef:VM:other"}else{"OpaqueRef:VM:snapshot"});guest["suspend_VDI"]=json!(if fault==Fault::SameMemory {"OpaqueRef:VDI:memory"}else{"OpaqueRef:VDI:copy"});
                if fault==Fault::NoStartup {guest["allowed_operations"]=json!([]);}
                if fault==Fault::LostRevert {json!({"_fixture_close_without_reply":true})}else{json!({"result":"OpaqueRef:task:revert"})}
            }
            "Async.VM.start"|"Async.VM.resume"=>{
                assert_eq!(request["params"][1],"OpaqueRef:VM:parent");assert_eq!(request["params"][2],false);assert_eq!(request["params"][3],false);records.get_mut("OpaqueRef:VM:parent").unwrap()["power_state"]=json!("Running");
                if fault==Fault::LostStart {json!({"_fixture_close_without_reply":true})}else{json!({"result":"OpaqueRef:task:start"})}
            }
            "Async.VM.snapshot"|"Async.VM.checkpoint"=>{
                let mut created=records["OpaqueRef:VM:snapshot"].clone();created["uuid"]=json!(uuid(10));created["name_label"]=request["params"][2].clone();created["power_state"]=json!(if method.ends_with("checkpoint") {"Suspended"}else{"Halted"});records.insert("OpaqueRef:VM:new".into(),created);if fault==Fault::AckCreate{recover_during_dispatch(journal.as_ref().unwrap(),true);}json!({"result":"OpaqueRef:task:create"})
            }
            "VM.set_name_label"|"VM.set_name_description"=>{
                if method=="VM.set_name_description" && fault==Fault::DeniedDescription {return json!({"error":{"message":"RBAC_PERMISSION_DENIED"}});}
                let rec=records.get_mut(request["params"][1].as_str().unwrap()).unwrap();rec[if method.ends_with("label") {"name_label"}else{"name_description"}]=request["params"][2].clone();
                if method=="VM.set_name_label" && fault==Fault::MetadataConflict {rec["name_description"]=json!("Changed elsewhere");}
                if method=="VM.set_name_description" && fault==Fault::LostDescription {json!({"_fixture_close_without_reply":true})}else{json!({"result":null})}
            }
            _=>panic!("unexpected request {method}"),
        }
    }}).await;
    (f, records, calls)
}
fn restore(running: bool, checkpoint: bool, start_after: bool) -> SnapshotWorkflow {
    SnapshotWorkflow::Restore {
        snapshot: snapshot(),
        expected_state: if running {
            PowerState::Running
        } else {
            PowerState::Halted
        },
        checkpoint,
        shutdown: running,
        start_after,
    }
}
fn create(memory: bool) -> SnapshotWorkflow {
    SnapshotWorkflow::Create {
        name: "Named recovery point".into(),
        description: "Upgrade rollback\nTicket 42".into(),
        memory,
    }
}
fn edit() -> SnapshotWorkflow {
    SnapshotWorkflow::Edit {
        snapshot: snapshot(),
        expected_name: "Before update".into(),
        expected_description: "Original description".into(),
        name: "Reviewed point".into(),
        description: "Edited description".into(),
    }
}
async fn execute(f: &Fixture, path: &Path, op: SnapshotWorkflow) -> OperationRecord {
    let (handle, request) = channel(
        OperationId::new(),
        PoolId::parse(&uuid(1)).unwrap(),
        vm(),
        1,
        OperationAction::Snapshot(SnapshotOperation::Workflow { operation: op }),
        tokio::runtime::Handle::current(),
    );
    let (tx, _rx) = mpsc::channel(128);
    tokio::time::timeout(
        scaled_deadline(5),
        request.run(
            Arc::new(f.session().await),
            Some("OpaqueRef:VM:parent".into()),
            path.into(),
            CancellationToken::new(),
            tx,
        ),
    )
    .await
    .unwrap();
    (*handle.latest()).clone()
}
async fn recheck(f: &Fixture, path: &Path, record: OperationRecord) -> OperationRecord {
    let id = record.id;
    let (tx, _rx) = mpsc::channel(128);
    tokio::time::timeout(
        scaled_deadline(5),
        MutationRequest::tracking(record).run(
            Arc::new(f.session().await),
            None,
            path.into(),
            CancellationToken::new(),
            tx,
        ),
    )
    .await
    .unwrap();
    Journal::open(path).unwrap().load(id).unwrap().unwrap()
}
fn count(calls: &Calls, method: &str) -> usize {
    calls
        .lock()
        .unwrap()
        .iter()
        .filter(|c| c["method"] == method)
        .count()
}
#[tokio::test]
async fn restore_running_disk_snapshot_orders_shutdown_revert_start() {
    let (f, r, c) = fixture(true, false, Fault::None).await;
    let d = tempfile::tempdir().unwrap();
    let result = execute(&f, d.path(), restore(true, false, true)).await;
    assert_eq!(result.state, OperationState::Succeeded, "{}", result.detail);
    assert_eq!(
        r.lock().unwrap()["OpaqueRef:VM:parent"]["power_state"],
        "Running"
    );
    let calls = c.lock().unwrap();
    let writes: Vec<_> = calls
        .iter()
        .filter_map(|c| c["method"].as_str().filter(|m| m.starts_with("Async.")))
        .collect();
    assert_eq!(
        writes,
        vec![
            "Async.VM.clean_shutdown",
            "Async.VM.revert",
            "Async.VM.start"
        ]
    );
}
#[tokio::test]
async fn checkpoint_restore_resumes_copy_and_retains_original_memory() {
    let (f, r, c) = fixture(false, true, Fault::None).await;
    let d = tempfile::tempdir().unwrap();
    let result = execute(&f, d.path(), restore(false, true, true)).await;
    assert_eq!(result.state, OperationState::Succeeded, "{}", result.detail);
    assert_eq!(count(&c, "Async.VM.resume"), 1);
    assert_eq!(
        r.lock().unwrap()["OpaqueRef:VM:snapshot"]["suspend_VDI"],
        "OpaqueRef:VDI:memory"
    );
    assert_eq!(count(&c, "VM.hard_shutdown"), 0);
}
#[tokio::test]
async fn restore_without_start_remains_stopped_or_suspended() {
    for memory in [false, true] {
        let (f, r, c) = fixture(false, memory, Fault::None).await;
        let d = tempfile::tempdir().unwrap();
        let result = execute(&f, d.path(), restore(false, memory, false)).await;
        assert_eq!(result.state, OperationState::Succeeded, "{}", result.detail);
        assert_eq!(
            r.lock().unwrap()["OpaqueRef:VM:parent"]["power_state"],
            if memory { "Suspended" } else { "Halted" }
        );
        assert_eq!(
            count(&c, "Async.VM.start") + count(&c, "Async.VM.resume"),
            0
        );
    }
}
#[tokio::test]
async fn lost_shutdown_or_revert_receipt_never_progresses_or_replays() {
    for fault in [Fault::LostShutdown, Fault::LostRevert, Fault::LostStart] {
        let (f, _, c) = fixture(true, false, fault).await;
        let d = tempfile::tempdir().unwrap();
        let result = execute(&f, d.path(), restore(true, false, true)).await;
        assert_eq!(
            result.state,
            OperationState::OutcomeUnknown,
            "{}",
            result.detail
        );
        let writes = c.lock().unwrap().len();
        let fresh = Journal::open(d.path())
            .unwrap()
            .load(result.id)
            .unwrap()
            .unwrap();
        let result = recheck(&f, d.path(), fresh).await;
        assert_eq!(result.state, OperationState::OutcomeUnknown);
        assert_eq!(count(&c, "Async.VM.clean_shutdown"), 1);
        assert_eq!(
            count(&c, "Async.VM.revert"),
            usize::from(fault != Fault::LostShutdown)
        );
        assert_eq!(
            count(&c, "Async.VM.start"),
            usize::from(fault == Fault::LostStart)
        );
        assert!(c.lock().unwrap().len() >= writes);
    }
}
#[tokio::test]
async fn failed_or_cancelled_revert_keeps_unknown_reservation_and_never_starts() {
    for fault in [Fault::FailedRevert, Fault::CancelledRevert] {
        let (f, _, c) = fixture(false, false, fault).await;
        let d = tempfile::tempdir().unwrap();
        let result = execute(&f, d.path(), restore(false, false, true)).await;
        assert_eq!(result.state, OperationState::OutcomeUnknown);
        assert_eq!(
            recheck(&f, d.path(), result).await.state,
            OperationState::OutcomeUnknown
        );
        assert_eq!(count(&c, "Async.VM.revert"), 1);
        assert_eq!(count(&c, "Async.VM.start"), 0);
    }
}
#[tokio::test]
async fn changed_snapshot_parent_stops_before_revert() {
    let (f, _, c) = fixture(true, false, Fault::ChangedParent).await;
    let d = tempfile::tempdir().unwrap();
    let result = execute(&f, d.path(), restore(true, false, true)).await;
    assert_eq!(result.state, OperationState::OutcomeUnknown);
    assert_eq!(count(&c, "Async.VM.revert"), 0);
}
#[tokio::test]
async fn contradictory_or_changed_restore_result_never_starts() {
    for (fault, checkpoint) in [(Fault::WrongRevertParent, false), (Fault::SameMemory, true)] {
        let (f, _, c) = fixture(false, checkpoint, fault).await;
        let d = tempfile::tempdir().unwrap();
        let result = execute(&f, d.path(), restore(false, checkpoint, true)).await;
        assert_eq!(result.state, OperationState::OutcomeUnknown);
        assert_eq!(
            count(&c, "Async.VM.start") + count(&c, "Async.VM.resume"),
            0
        );
    }
}
#[tokio::test]
async fn startup_refusal_retains_verified_restore_as_partial() {
    let (f, _, c) = fixture(false, false, Fault::NoStartup).await;
    let d = tempfile::tempdir().unwrap();
    let result = execute(&f, d.path(), restore(false, false, true)).await;
    assert_eq!(
        result.state,
        OperationState::PartiallyCompleted,
        "{}",
        result.detail
    );
    assert_eq!(count(&c, "Async.VM.start"), 0);
}
#[tokio::test]
async fn create_disk_and_memory_snapshots_save_description_on_new_identity() {
    for memory in [false, true] {
        let (f, r, _) = fixture(true, memory, Fault::None).await;
        let d = tempfile::tempdir().unwrap();
        let result = execute(&f, d.path(), create(memory)).await;
        assert_eq!(result.state, OperationState::Succeeded, "{}", result.detail);
        assert_eq!(
            result.created_snapshot,
            Some(SnapshotId::parse(&uuid(10)).unwrap())
        );
        assert!(result.verified_snapshot_creation(result.created_snapshot.unwrap()));
        assert_eq!(
            r.lock().unwrap()["OpaqueRef:VM:new"]["name_description"],
            "Upgrade rollback\nTicket 42"
        );
    }
}
#[tokio::test]
async fn create_metadata_failure_preserves_created_snapshot_for_review() {
    let (f, _, c) = fixture(true, false, Fault::DeniedDescription).await;
    let d = tempfile::tempdir().unwrap();
    let result = execute(&f, d.path(), create(false)).await;
    assert_eq!(result.state, OperationState::PartiallyCompleted);
    assert!(result.created_snapshot.is_some());
    assert_eq!(count(&c, "Async.VM.snapshot"), 1);
}
#[tokio::test]
async fn preexisting_snapshot_task_result_cannot_be_renamed_or_described() {
    let (f, _, c) = fixture(true, false, Fault::ExistingResult).await;
    let d = tempfile::tempdir().unwrap();
    let result = execute(&f, d.path(), create(false)).await;
    assert_eq!(result.state, OperationState::OutcomeUnknown);
    assert!(result.created_snapshot.is_none());
    assert_eq!(count(&c, "VM.set_name_description"), 0);
}
#[tokio::test]
async fn metadata_edit_lost_reply_reconciles_without_replay() {
    let (f, r, c) = fixture(false, false, Fault::LostDescription).await;
    let d = tempfile::tempdir().unwrap();
    let result = execute(&f, d.path(), edit()).await;
    assert_eq!(result.state, OperationState::OutcomeUnknown);
    assert_eq!(
        recheck(&f, d.path(), result).await.state,
        OperationState::Succeeded
    );
    assert_eq!(count(&c, "VM.set_name_label"), 1);
    assert_eq!(count(&c, "VM.set_name_description"), 1);
    assert_eq!(
        r.lock().unwrap()["OpaqueRef:VM:snapshot"]["name_label"],
        "Reviewed point"
    );
}
#[tokio::test]
async fn concurrent_metadata_change_stops_later_setter() {
    let (f, r, c) = fixture(false, false, Fault::MetadataConflict).await;
    let d = tempfile::tempdir().unwrap();
    let result = execute(&f, d.path(), edit()).await;
    assert_eq!(result.state, OperationState::PartiallyCompleted);
    assert_eq!(count(&c, "VM.set_name_description"), 0);
    assert_eq!(
        r.lock().unwrap()["OpaqueRef:VM:snapshot"]["name_description"],
        "Changed elsewhere"
    );
}
#[tokio::test]
async fn fresh_blocked_operation_or_changed_metadata_refuses_dispatch() {
    for edit_mode in [false, true] {
        let (f, r, c) = fixture(false, false, Fault::None).await;
        if edit_mode {
            r.lock().unwrap().get_mut("OpaqueRef:VM:snapshot").unwrap()["name_description"] =
                json!("Another operator");
        } else {
            r.lock().unwrap().get_mut("OpaqueRef:VM:snapshot").unwrap()["blocked_operations"] =
                json!({"revert":"Policy"});
        }
        let d = tempfile::tempdir().unwrap();
        let result = execute(
            &f,
            d.path(),
            if edit_mode {
                edit()
            } else {
                restore(false, false, false)
            },
        )
        .await;
        assert_eq!(result.state, OperationState::Rejected);
        assert_eq!(
            count(&c, "Async.VM.revert") + count(&c, "VM.set_name_label"),
            0
        );
    }
}
#[tokio::test]
async fn workflow_evidence_survives_journal_but_is_never_publicly_serialized() {
    let (f, _, _) = fixture(true, false, Fault::DeniedDescription).await;
    let d = tempfile::tempdir().unwrap();
    let result = execute(&f, d.path(), create(false)).await;
    let public = serde_json::to_string(&result).unwrap();
    assert!(!public.contains("OpaqueRef:"));
    assert!(!public.contains("snapshot_workflow"));
    let loaded = Journal::open(d.path())
        .unwrap()
        .load(result.id)
        .unwrap()
        .unwrap();
    assert_eq!(loaded.snapshot_workflow, result.snapshot_workflow);
    let mut changed = loaded.clone();
    changed.snapshot_workflow.as_mut().unwrap().baseline.clear();
    assert!(matches!(
        Journal::open(d.path()).unwrap().save(&mut changed),
        Err(Error::JournalChanged)
    ));
}
#[test]
fn metadata_bounds_and_explicit_shutdown_are_required() {
    assert!(create(false).validate().is_ok());
    let mut command = restore(true, false, false);
    if let SnapshotWorkflow::Restore { shutdown, .. } = &mut command {
        *shutdown = false
    };
    assert!(command.validate().is_err());
    assert!(
        SnapshotWorkflow::Create {
            name: "name".into(),
            description: "x".repeat(4097),
            memory: false
        }
        .validate()
        .is_err()
    );
}

#[tokio::test]
async fn restart_reconciles_received_task_but_never_runs_pending_steps() {
    for fault in [Fault::LostShutdownRead, Fault::LostRevertRead] {
        let (f, _, calls) = fixture(true, false, fault).await;
        let directory = tempfile::tempdir().unwrap();
        let record = execute(&f, directory.path(), restore(true, false, true)).await;
        assert_eq!(record.state, OperationState::OutcomeUnknown);
        let record = Journal::open(directory.path())
            .unwrap()
            .load(record.id)
            .unwrap()
            .unwrap();
        let record = recheck(&f, directory.path(), record).await;
        assert_eq!(
            record.state,
            OperationState::PartiallyCompleted,
            "{}",
            record.detail
        );
        assert_eq!(count(&calls, "Async.VM.clean_shutdown"), 1);
        assert_eq!(
            count(&calls, "Async.VM.revert"),
            usize::from(fault == Fault::LostRevertRead)
        );
        assert_eq!(count(&calls, "Async.VM.start"), 0);
    }
}
#[tokio::test]
async fn running_restore_without_explicit_shutdown_is_rejected_before_mutation() {
    let (f, _, calls) = fixture(true, false, Fault::None).await;
    let directory = tempfile::tempdir().unwrap();
    let mut operation = restore(true, false, true);
    if let SnapshotWorkflow::Restore { shutdown, .. } = &mut operation {
        *shutdown = false
    }
    let record = execute(&f, directory.path(), operation).await;
    assert_eq!(record.state, OperationState::Rejected);
    assert_eq!(
        count(&calls, "Async.VM.clean_shutdown") + count(&calls, "Async.VM.revert"),
        0
    );
}

fn recover_during_dispatch(path: &Path, acknowledge: bool) {
    let pool = PoolId::parse(&uuid(1)).unwrap();
    let records = recover(path, pool).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].state, OperationState::OutcomeUnknown);
    if acknowledge {
        Journal::open(path)
            .unwrap()
            .control(pool, records[0].id, true)
            .unwrap();
    }
}
#[tokio::test]
async fn late_create_receipt_after_ack_keeps_new_identity_but_never_sets_metadata() {
    let directory = tempfile::tempdir().unwrap();
    let (f, _, calls) =
        fixture_with_journal(true, false, Fault::AckCreate, Some(directory.path().into())).await;
    let record = execute(&f, directory.path(), create(false)).await;
    assert_eq!(
        record.state,
        OperationState::AcknowledgedUnknown,
        "{}",
        record.detail
    );
    assert_eq!(
        record.created_snapshot,
        Some(SnapshotId::parse(&uuid(10)).unwrap())
    );
    let plan = record.snapshot_workflow.as_ref().unwrap();
    assert!(plan.evidence[0].task.is_some());
    assert!(plan.evidence[0].completed);
    assert_eq!(count(&calls, "VM.set_name_description"), 0);
    let stored = Journal::open(directory.path())
        .unwrap()
        .load(record.id)
        .unwrap()
        .unwrap();
    assert_eq!(stored.created_snapshot, record.created_snapshot);
}
#[tokio::test]
async fn late_shutdown_receipt_after_recovery_or_ack_never_continues_restore() {
    for fault in [Fault::AckShutdown, Fault::RecoverShutdown] {
        let directory = tempfile::tempdir().unwrap();
        let (f, _, calls) =
            fixture_with_journal(true, false, fault, Some(directory.path().into())).await;
        let record = execute(&f, directory.path(), restore(true, false, true)).await;
        assert_eq!(
            record.state,
            if fault == Fault::AckShutdown {
                OperationState::AcknowledgedUnknown
            } else {
                OperationState::OutcomeUnknown
            }
        );
        assert!(record.snapshot_workflow.as_ref().unwrap().evidence[0].completed);
        assert_eq!(count(&calls, "Async.VM.clean_shutdown"), 1);
        assert_eq!(count(&calls, "Async.VM.revert"), 0);
        assert_eq!(count(&calls, "Async.VM.start"), 0);
        if fault == Fault::RecoverShutdown {
            assert_eq!(
                recheck(&f, directory.path(), record).await.state,
                OperationState::PartiallyCompleted
            );
            assert_eq!(count(&calls, "Async.VM.revert"), 0);
        }
    }
}

#[tokio::test]
async fn stop_remaining_steps_waits_for_shutdown_receipt_then_keeps_vm_halted() {
    let directory = tempfile::tempdir().unwrap();
    let (f, records, calls) = fixture_with_journal(
        true,
        false,
        Fault::CancelShutdown,
        Some(directory.path().into()),
    )
    .await;
    let record = execute(&f, directory.path(), restore(true, false, true)).await;
    assert_eq!(
        record.state,
        OperationState::PartiallyCompleted,
        "{}",
        record.detail
    );
    assert!(record.cancel_requested);
    assert_eq!(record.steps[0].state, StepState::Verified);
    assert_eq!(
        records.lock().unwrap()["OpaqueRef:VM:parent"]["power_state"],
        "Halted"
    );
    assert_eq!(count(&calls, "Async.VM.revert"), 0);
    assert_eq!(count(&calls, "task.cancel"), 0);
}
