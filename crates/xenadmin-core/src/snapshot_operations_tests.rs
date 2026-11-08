use super::*;
use crate::xapi::inventory_tests::Fixture;
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
fn snapshot() -> crate::SnapshotId {
    crate::SnapshotId::parse(&uuid(3)).unwrap()
}
fn disk() -> crate::VdiId {
    crate::VdiId::parse(&uuid(4)).unwrap()
}
#[derive(Clone, Copy, PartialEq)]
enum Fault {
    None,
    ConversionLost,
    ConversionDenied,
    DiskLost,
    DiskDenied,
    MemoryRetained,
    ExistingTemplate,
    ExistingVm,
}

async fn fixture(checkpoint: bool, fault: Fault) -> (Fixture, Records, Calls) {
    fixture_with_permissions(checkpoint, fault, json!({"is_local_superuser":true})).await
}
async fn fixture_with_permissions(
    checkpoint: bool,
    fault: Fault,
    permissions: Value,
) -> (Fixture, Records, Calls) {
    let mut values = BTreeMap::from([
        (
            "OpaqueRef:VM:parent".into(),
            json!({"uuid":uuid(2),"power_state":"Running","is_a_snapshot":false,"is_a_template":false,"is_control_domain":false}),
        ),
        (
            "OpaqueRef:VM:snapshot".into(),
            json!({"uuid":uuid(3),"power_state":if checkpoint {"Suspended"}else{"Halted"},"is_a_snapshot":true,"is_a_template":true,"is_control_domain":false,"snapshot_of":"OpaqueRef:VM:parent","VBDs":["OpaqueRef:VBD:disk"],"allowed_operations":["destroy","hard_shutdown","clone"],"suspend_VDI":if checkpoint {"OpaqueRef:VDI:memory"} else {"OpaqueRef:NULL"}}),
        ),
        (
            "OpaqueRef:VDI:disk".into(),
            json!({"uuid":uuid(4),"type":"user","is_a_snapshot":true,"sharable":false,"managed":true,"missing":false,"VBDs":["OpaqueRef:VBD:disk"],"allowed_operations":["destroy"]}),
        ),
        (
            "OpaqueRef:VBD:disk".into(),
            json!({"uuid":uuid(5),"VM":"OpaqueRef:VM:snapshot","VDI":"OpaqueRef:VDI:disk","type":"Disk","other_config":{"owner":"true"}}),
        ),
    ]);
    if checkpoint {
        values.insert(
            "OpaqueRef:VDI:memory".into(),
            json!({"uuid":uuid(6),"type":"suspend","sharable":false,"VBDs":[]}),
        );
    }
    if matches!(fault, Fault::ExistingTemplate | Fault::ExistingVm) {
        values.insert("OpaqueRef:VM:clone".into(),json!({"uuid":uuid(10),"name_label":"Existing unrelated object","power_state":"Halted","is_a_snapshot":false,"is_a_template":fault==Fault::ExistingTemplate,"is_control_domain":false,"snapshot_of":"OpaqueRef:NULL"}));
    }
    let records: Records = Arc::new(Mutex::new(values));
    let calls: Calls = Arc::new(Mutex::new(Vec::new()));
    let fixture=Fixture::with_permissions({let records=records.clone();let calls=calls.clone();move |request|{
        let method=request["method"].as_str().unwrap();
        calls.lock().unwrap().push(request.clone());
        let mut records=records.lock().unwrap();
        if let Some(class)=method.strip_suffix(".get_by_uuid"){
            return match records.iter().find(|(reference,record)|reference.starts_with(&format!("OpaqueRef:{class}:"))&&record["uuid"]==request["params"][1]){
                Some((reference,_))=>json!({"result":reference}),None=>json!({"error":{"message":"UUID_INVALID"}}),
            };
        }
        if method=="VM.get_all_records"{return json!({"result":records.iter().filter(|(reference,_)|reference.starts_with("OpaqueRef:VM:")).collect::<BTreeMap<_,_>>()});}
        if method=="task.get_record" {return json!({"result":{"status":"success","progress":1.0,"result":"<value>OpaqueRef:VM:clone</value>"}});}
        if method.ends_with(".get_record"){
            return records.get(request["params"][1].as_str().unwrap()).map_or_else(||json!({"error":{"message":"HANDLE_INVALID"}}),|record|json!({"result":record}));
        }
        match method{
            "Async.VM.clone"=>{
                assert_eq!(request["params"][1],"OpaqueRef:VM:snapshot");
                if matches!(fault,Fault::ExistingTemplate|Fault::ExistingVm) {return json!({"result":"OpaqueRef:task:clone"});}
                records.insert("OpaqueRef:VM:clone".into(),json!({"uuid":uuid(10),"name_label":request["params"][2],"power_state":"Halted","is_a_snapshot":false,"is_a_template":true,"is_control_domain":false,"snapshot_of":"OpaqueRef:NULL"}));
                json!({"result":"OpaqueRef:task:clone"})
            },
            "VM.set_is_a_template"=>{
                assert_eq!(request["params"][1],"OpaqueRef:VM:clone");assert_eq!(request["params"][2],false);
                if fault==Fault::ConversionDenied {return json!({"error":{"message":"RBAC_PERMISSION_DENIED"}});}
                records.get_mut("OpaqueRef:VM:clone").unwrap()["is_a_template"]=json!(false);
                if fault==Fault::ConversionLost {json!({"_fixture_close_without_reply":true})}else{json!({"result":null})}
            },
            "VM.hard_shutdown"=>{
                assert_eq!(request["params"][1],"OpaqueRef:VM:snapshot");
                records.get_mut("OpaqueRef:VM:snapshot").unwrap()["power_state"]=json!("Halted");
                if fault!=Fault::MemoryRetained {records.remove("OpaqueRef:VDI:memory");}
                json!({"result":null})
            },
            "VM.destroy"=>{
                assert_eq!(request["params"][1],"OpaqueRef:VM:snapshot");
                records.remove("OpaqueRef:VM:snapshot");
                records.remove("OpaqueRef:VBD:disk");
                records.get_mut("OpaqueRef:VDI:disk").unwrap()["VBDs"]=json!([]);
                json!({"result":null})
            },
            "VDI.destroy"=>{
                assert_eq!(request["params"][1],"OpaqueRef:VDI:disk");
                if fault==Fault::DiskDenied {return json!({"error":{"message":"SR_BACKEND_FAILURE"}});}
                records.remove("OpaqueRef:VDI:disk");
                if fault==Fault::DiskLost {json!({"_fixture_close_without_reply":true})}else{json!({"result":null})}
            },
            "task.destroy"=>json!({"result":null}),
            _=>panic!("unexpected fixture request {method}"),
        }
    }}, permissions).await;
    (fixture, records, calls)
}

async fn execute(
    fixture: &Fixture,
    directory: &Path,
    action: SnapshotOperation,
) -> OperationRecord {
    let (operation, request) = channel(
        OperationId::new(),
        PoolId::parse(&uuid(1)).unwrap(),
        vm(),
        1,
        OperationAction::Snapshot(action),
        tokio::runtime::Handle::current(),
    );
    let (tx, _rx) = mpsc::channel(128);
    tokio::time::timeout(
        Duration::from_secs(5),
        request.run(
            Arc::new(fixture.session().await),
            Some("OpaqueRef:VM:parent".into()),
            directory.into(),
            CancellationToken::new(),
            tx,
        ),
    )
    .await
    .unwrap();
    (*operation.latest()).clone()
}
async fn recheck(fixture: &Fixture, directory: &Path, record: OperationRecord) -> OperationRecord {
    let id = record.id;
    let (tx, _rx) = mpsc::channel(128);
    tokio::time::timeout(
        Duration::from_secs(5),
        MutationRequest::tracking(record).run(
            Arc::new(fixture.session().await),
            None,
            directory.into(),
            CancellationToken::new(),
            tx,
        ),
    )
    .await
    .unwrap();
    Journal::open(directory).unwrap().load(id).unwrap().unwrap()
}
fn count(calls: &Calls, method: &str) -> usize {
    calls
        .lock()
        .unwrap()
        .iter()
        .filter(|call| call["method"] == method)
        .count()
}
fn clone_action(as_template: bool) -> SnapshotOperation {
    SnapshotOperation::Clone {
        snapshot: snapshot(),
        name: "Reviewed clone".into(),
        as_template,
    }
}

#[tokio::test]
async fn snapshot_clone_returns_a_verified_halted_vm_or_template() {
    for as_template in [false, true] {
        let (fixture, records, calls) = fixture(false, Fault::None).await;
        let directory = tempfile::tempdir().unwrap();
        let record = execute(&fixture, directory.path(), clone_action(as_template)).await;
        assert_eq!(record.state, OperationState::Succeeded, "{}", record.detail);
        assert_eq!(record.created_vm, Some(VmId::parse(&uuid(10)).unwrap()));
        assert_eq!(
            records.lock().unwrap()["OpaqueRef:VM:clone"]["is_a_template"],
            as_template
        );
        assert_eq!(count(&calls, "Async.VM.clone"), 1);
        assert_eq!(
            count(&calls, "VM.set_is_a_template"),
            usize::from(!as_template)
        );
        assert_eq!(
            records.lock().unwrap()["OpaqueRef:VM:parent"]["power_state"],
            "Running"
        );
    }
}

#[tokio::test]
async fn snapshot_clone_conversion_refusal_retains_created_identity_as_partial() {
    let (fixture, _, calls) = fixture(false, Fault::ConversionDenied).await;
    let directory = tempfile::tempdir().unwrap();
    let record = execute(&fixture, directory.path(), clone_action(false)).await;
    assert_eq!(record.state, OperationState::PartiallyCompleted);
    assert_eq!(record.created_vm, Some(VmId::parse(&uuid(10)).unwrap()));
    assert_eq!(count(&calls, "Async.VM.clone"), 1);
    assert_eq!(record.steps[0].state, StepState::Failed);
}

#[tokio::test]
async fn snapshot_clone_lost_conversion_reply_reconciles_without_repeating_writes() {
    let (fixture, _, calls) = fixture(false, Fault::ConversionLost).await;
    let directory = tempfile::tempdir().unwrap();
    let record = execute(&fixture, directory.path(), clone_action(false)).await;
    assert_eq!(record.state, OperationState::OutcomeUnknown);
    assert!(record.created_vm.is_some());
    let record = recheck(&fixture, directory.path(), record).await;
    assert_eq!(record.state, OperationState::Succeeded, "{}", record.detail);
    assert_eq!(count(&calls, "Async.VM.clone"), 1);
    assert_eq!(count(&calls, "VM.set_is_a_template"), 1);
}

#[tokio::test]
async fn snapshot_deletion_only_removes_explicitly_reviewed_exclusive_disks() {
    for delete_disk in [false, true] {
        let (fixture, records, calls) = fixture(false, Fault::None).await;
        let directory = tempfile::tempdir().unwrap();
        let record = execute(
            &fixture,
            directory.path(),
            SnapshotOperation::Delete {
                snapshot: snapshot(),
                disks: if delete_disk { vec![disk()] } else { vec![] },
            },
        )
        .await;
        assert_eq!(record.state, OperationState::Succeeded, "{}", record.detail);
        assert!(
            !records
                .lock()
                .unwrap()
                .contains_key("OpaqueRef:VM:snapshot")
        );
        assert_eq!(
            records.lock().unwrap().contains_key("OpaqueRef:VDI:disk"),
            !delete_disk
        );
        assert_eq!(count(&calls, "VDI.destroy"), usize::from(delete_disk));
        assert_eq!(
            records.lock().unwrap()["OpaqueRef:VM:parent"]["power_state"],
            "Running"
        );
        assert!(record.steps.iter().all(|s| s.state == StepState::Verified));
    }
}

#[tokio::test]
async fn snapshot_deletion_rejects_changed_disk_ownership_before_writes() {
    for field in ["sharable", "is_a_snapshot", "owner", "outside_consumer"] {
        let (fixture, records, calls) = fixture(false, Fault::None).await;
        {
            let mut records = records.lock().unwrap();
            match field {
                "sharable" => records.get_mut("OpaqueRef:VDI:disk").unwrap()[field] = json!(true),
                "is_a_snapshot" => {
                    records.get_mut("OpaqueRef:VDI:disk").unwrap()[field] = json!(false)
                }
                "owner" => {
                    records.get_mut("OpaqueRef:VBD:disk").unwrap()["other_config"] = json!({})
                }
                _ => {
                    records.get_mut("OpaqueRef:VDI:disk").unwrap()["VBDs"] =
                        json!(["OpaqueRef:VBD:disk", "OpaqueRef:VBD:outside"])
                }
            }
        }
        let directory = tempfile::tempdir().unwrap();
        let record = execute(
            &fixture,
            directory.path(),
            SnapshotOperation::Delete {
                snapshot: snapshot(),
                disks: vec![disk()],
            },
        )
        .await;
        assert_eq!(
            record.state,
            OperationState::Rejected,
            "{field}: {}",
            record.detail
        );
        assert_eq!(count(&calls, "VM.destroy"), 0);
        assert_eq!(count(&calls, "VDI.destroy"), 0);
    }
}

#[tokio::test]
async fn snapshot_deletion_disk_failure_is_partial_and_lost_reply_is_reconciled() {
    for fault in [Fault::DiskDenied, Fault::DiskLost] {
        let (fixture, records, calls) = fixture(false, fault).await;
        let directory = tempfile::tempdir().unwrap();
        let record = execute(
            &fixture,
            directory.path(),
            SnapshotOperation::Delete {
                snapshot: snapshot(),
                disks: vec![disk()],
            },
        )
        .await;
        assert!(
            !records
                .lock()
                .unwrap()
                .contains_key("OpaqueRef:VM:snapshot")
        );
        if fault == Fault::DiskDenied {
            assert_eq!(
                record.state,
                OperationState::PartiallyCompleted,
                "{}",
                record.detail
            );
            assert!(records.lock().unwrap().contains_key("OpaqueRef:VDI:disk"));
        } else {
            assert_eq!(
                record.state,
                OperationState::OutcomeUnknown,
                "{}",
                record.detail
            );
            let record = recheck(&fixture, directory.path(), record).await;
            assert_eq!(record.state, OperationState::Succeeded, "{}", record.detail);
        }
        assert_eq!(count(&calls, "VM.destroy"), 1);
        assert_eq!(count(&calls, "VDI.destroy"), 1);
    }
}

#[tokio::test]
async fn checkpoint_deletion_discards_only_snapshot_memory_and_verifies_its_cleanup() {
    for fault in [Fault::None, Fault::MemoryRetained] {
        let (fixture, records, calls) = fixture(true, fault).await;
        let directory = tempfile::tempdir().unwrap();
        let record = execute(
            &fixture,
            directory.path(),
            SnapshotOperation::Delete {
                snapshot: snapshot(),
                disks: vec![],
            },
        )
        .await;
        if fault == Fault::None {
            assert_eq!(record.state, OperationState::Succeeded, "{}", record.detail);
            assert!(!records.lock().unwrap().contains_key("OpaqueRef:VDI:memory"));
        } else {
            assert_eq!(
                record.state,
                OperationState::OutcomeUnknown,
                "{}",
                record.detail
            );
            assert_eq!(count(&calls, "VM.destroy"), 0);
        }
        assert_eq!(count(&calls, "VM.hard_shutdown"), 1);
        assert_eq!(
            records.lock().unwrap()["OpaqueRef:VM:parent"]["power_state"],
            "Running"
        );
    }
}

#[tokio::test]
async fn checkpoint_memory_replacement_is_rejected_before_discard() {
    let (fixture, records, calls) = fixture(true, Fault::None).await;
    let session = fixture.session().await;
    let plan = snapshots::deletion_plan(&session, vm(), snapshot(), &[])
        .await
        .unwrap();
    records
        .lock()
        .unwrap()
        .get_mut("OpaqueRef:VDI:memory")
        .unwrap()["uuid"] = json!(uuid(99));
    assert!(matches!(
        DeleteStep::DiscardMemory.send(&session, vm(), &plan).await,
        Err(Error::SnapshotConflict)
    ));
    assert_eq!(count(&calls, "VM.hard_shutdown"), 0);
    assert_eq!(count(&calls, "VM.destroy"), 0);
}

#[tokio::test]
async fn clone_reconciliation_never_starts_an_unsubmitted_conversion() {
    for pending_step in [false, true] {
        let (fixture, records, calls) = fixture(false, Fault::None).await;
        records.lock().unwrap().insert("OpaqueRef:VM:clone".into(),json!({"uuid":uuid(10),"power_state":"Halted","is_a_snapshot":false,"is_a_template":true,"is_control_domain":false,"snapshot_of":"OpaqueRef:NULL"}));
        let directory = tempfile::tempdir().unwrap();
        let (_, mut request) = channel(
            OperationId::new(),
            PoolId::parse(&uuid(1)).unwrap(),
            vm(),
            1,
            OperationAction::Snapshot(clone_action(false)),
            tokio::runtime::Handle::current(),
        );
        let mut journal = Journal::open(directory.path()).unwrap();
        journal.admit(&mut request.record).unwrap();
        request.record.task = Some("OpaqueRef:task:clone".into());
        request.record.result_reference = Some("OpaqueRef:VM:clone".into());
        request.record.state = OperationState::Tracking;
        request.record.snapshot_clone_baseline = Some(vec![vm(), VmId::parse(&uuid(3)).unwrap()]);
        if pending_step {
            request.record.steps.push(OperationStep {
                label: "Convert cloned template into a VM".into(),
                state: StepState::Pending,
                detail: String::new(),
            });
        }
        journal.save(&mut request.record).unwrap();
        let record = recheck(&fixture, directory.path(), request.record).await;
        assert_eq!(
            record.state,
            OperationState::PartiallyCompleted,
            "{}",
            record.detail
        );
        assert_eq!(record.created_vm, Some(VmId::parse(&uuid(10)).unwrap()));
        assert_eq!(count(&calls, "Async.VM.clone"), 0);
        assert_eq!(count(&calls, "VM.set_is_a_template"), 0);
    }
}

#[tokio::test]
async fn misbound_clone_result_never_converts_or_claims_a_preexisting_object() {
    for fault in [Fault::ExistingTemplate, Fault::ExistingVm] {
        for as_template in [false, true] {
            let (fixture, records, calls) = fixture(false, fault).await;
            let directory = tempfile::tempdir().unwrap();
            let record = execute(&fixture, directory.path(), clone_action(as_template)).await;
            assert_eq!(
                record.state,
                OperationState::OutcomeUnknown,
                "{}",
                record.detail
            );
            assert!(record.created_vm.is_none());
            assert_eq!(count(&calls, "VM.set_is_a_template"), 0);
            assert_eq!(
                records.lock().unwrap()["OpaqueRef:VM:clone"]["is_a_template"],
                fault == Fault::ExistingTemplate
            );
        }
    }
}

#[tokio::test]
async fn clone_reconciliation_without_identity_baseline_fails_closed() {
    let (fixture, records, calls) = fixture(false, Fault::ExistingTemplate).await;
    let directory = tempfile::tempdir().unwrap();
    let (_, mut request) = channel(
        OperationId::new(),
        PoolId::parse(&uuid(1)).unwrap(),
        vm(),
        1,
        OperationAction::Snapshot(clone_action(false)),
        tokio::runtime::Handle::current(),
    );
    let mut journal = Journal::open(directory.path()).unwrap();
    journal.admit(&mut request.record).unwrap();
    request.record.task = Some("OpaqueRef:task:clone".into());
    request.record.result_reference = Some("OpaqueRef:VM:clone".into());
    request.record.state = OperationState::Tracking;
    journal.save(&mut request.record).unwrap();
    let record = recheck(&fixture, directory.path(), request.record).await;
    assert_eq!(
        record.state,
        OperationState::OutcomeUnknown,
        "{}",
        record.detail
    );
    assert!(record.created_vm.is_none());
    assert_eq!(count(&calls, "VM.set_is_a_template"), 0);
    assert_eq!(
        records.lock().unwrap()["OpaqueRef:VM:clone"]["is_a_template"],
        true
    );
}

#[tokio::test]
async fn oversized_clone_identity_baseline_refuses_before_dispatch() {
    let (fixture, records, calls) = fixture(false, Fault::None).await;
    {
        let mut records = records.lock().unwrap();
        for n in 100..10_101 {
            records.insert(format!("OpaqueRef:VM:extra-{n}"), json!({"uuid":uuid(n)}));
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let record = execute(&fixture, directory.path(), clone_action(false)).await;
    assert_eq!(record.state, OperationState::Rejected, "{}", record.detail);
    assert_eq!(count(&calls, "Async.VM.clone"), 0);
}

#[tokio::test]
async fn checkpoint_discard_denial_is_resolved_before_deletion_and_plain_snapshots_do_not_require_it()
 {
    let action = || SnapshotOperation::Delete {
        snapshot: snapshot(),
        disks: vec![disk()],
    };
    let (f, _, calls) = fixture(true, Fault::None).await;
    let d = tempfile::tempdir().unwrap();
    assert_eq!(
        execute(&f, d.path(), action()).await.state,
        OperationState::Succeeded
    );
    let methods = calls
        .lock()
        .unwrap()
        .iter()
        .map(|r| {
            r["method"]
                .as_str()
                .unwrap()
                .trim_start_matches("Async.")
                .to_ascii_lowercase()
        })
        .filter(|m| m != "vm.hard_shutdown")
        .collect::<Vec<_>>();
    for checkpoint in [false, true] {
        let (f, records, calls) = fixture_with_permissions(
            checkpoint,
            Fault::None,
            json!({"is_local_superuser":false,"rbac_permissions":methods}),
        )
        .await;
        let d = tempfile::tempdir().unwrap();
        let result = execute(&f, d.path(), action()).await;
        if checkpoint {
            assert!(
                result.detail.contains("RBAC_PERMISSION_DENIED"),
                "{}",
                result.detail
            );
            assert_eq!(
                records.lock().unwrap()["OpaqueRef:VM:snapshot"]["power_state"],
                "Suspended"
            );
            assert!(!calls.lock().unwrap().iter().any(|r| matches!(
                r["method"].as_str(),
                Some("VM.destroy" | "VDI.destroy" | "VM.hard_shutdown")
            )));
        } else {
            assert_eq!(result.state, OperationState::Succeeded, "{}", result.detail);
        }
    }
}
