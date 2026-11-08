use super::*;
use crate::xapi::inventory_tests::Fixture;
use crate::{
    BatchVmWorkflow, OperationControl, PowerState, ResourceAction, ResourceTarget, VmWorkflow,
    VmWorkflowBatch,
};
use std::{collections::BTreeMap, sync::Mutex};
#[path = "vm_workflow_move_tests.rs"]
mod moves;

fn pool() -> PoolId {
    Uuid::from_u128(1).into()
}
fn vm(n: u128) -> VmId {
    Uuid::from_u128(n).into()
}
fn selection() -> VmWorkflowBatch {
    VmWorkflowBatch {
        items: (2..4)
            .map(|n| BatchVmWorkflow {
                vm: vm(n),
                expected_state: PowerState::Halted,
                workflow: VmWorkflow::Delete {
                    expected_template: false,
                    disks: vec![],
                    snapshots: vec![],
                },
            })
            .collect(),
    }
}
fn parent() -> (crate::ResourceOperation, crate::resources::ResourceRequest) {
    crate::resources::channel(
        OperationId::new(),
        crate::ObservedResource {
            pool: pool(),
            generation: 4,
            target: ResourceTarget::Pool(pool()),
        },
        ResourceAction::VmWorkflowBatch(selection()),
        tokio::runtime::Handle::current(),
    )
}
fn child(p: &crate::ResourceOperationRecord, index: usize) -> MutationRequest {
    let plan = p.workflow_batch_plan.as_ref().unwrap();
    let item = &plan.selection.items[index];
    let (_, mut r) = channel(
        plan.children[index],
        p.pool,
        item.vm,
        4,
        item.workflow.action(),
        tokio::runtime::Handle::current(),
    );
    r.record.batch_parent = Some(p.id);
    r
}
fn guest(n: u128) -> serde_json::Value {
    json!({"uuid":vm(n),"name_label":"Fixture guest","name_description":"", "power_state":"Halted",
        "is_a_template":false,"is_a_snapshot":false,"is_control_domain":false,"is_default_template":false,
        "current_operations":{},"blocked_operations":{},"allowed_operations":["destroy"],"other_config":{},"VBDs":[],"VIFs":[]})
}
#[tokio::test]
async fn batch_dispatch_cannot_wait_for_a_full_progress_queue_after_its_fence() {
    let dir = tempfile::tempdir().unwrap();
    let mut j = Journal::open(dir.path()).unwrap();
    let (_, mut p) = parent();
    j.resource_admit(&mut p.record).unwrap();
    let mut c = child(&p.record, 0);
    let permit = j.workflow_permit(p.record.id, 0).unwrap();
    j.admit_owned(&mut c.record, Some(&permit)).unwrap();
    c.record.steps = vec![crate::OperationStep {
        label: "Delete selected VM".into(),
        state: crate::StepState::Pending,
        detail: String::new(),
    }];
    let (tx, _rx) = mpsc::channel(1);
    tx.try_send(OperationUpdate::Record(Box::new(c.record.clone())))
        .unwrap();
    c.updates = Some(tx);
    assert!(
        tokio::time::timeout(Duration::from_secs(1), c.begin_step(&mut j, 0))
            .await
            .expect("dispatch must not yield to UI backpressure")
            .unwrap()
    );
    assert_eq!(
        j.load(c.record.id).unwrap().unwrap().state,
        OperationState::Dispatching
    );
    j.workflow_interrupt(pool(), p.record.id, false).unwrap();
    assert!(matches!(
        j.save_vm_dispatch(&mut c.record),
        Err(Error::JournalChanged)
    ));
}
async fn run(p: crate::resources::ResourceRequest, f: &Fixture, dir: &Path) {
    let (tx, mut rx) = mpsc::channel(128);
    let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
    tokio::time::timeout(
        Duration::from_secs(10),
        p.run(
            Arc::new(f.session().await),
            dir.into(),
            CancellationToken::new(),
            tx,
        ),
    )
    .await
    .unwrap();
    drain.abort();
}
async fn fixture(fault: &str, dir: &Path, id: OperationId) -> (Fixture, Arc<Mutex<Vec<String>>>) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let records = Arc::new(Mutex::new(BTreeMap::from([
        ("OpaqueRef:pool:p".to_owned(), json!({"uuid":pool()})),
        ("OpaqueRef:VM:2".to_owned(), guest(2)),
        ("OpaqueRef:VM:3".to_owned(), guest(3)),
    ])));
    if fault == "preflight" {
        records.lock().unwrap().get_mut("OpaqueRef:VM:3").unwrap()["power_state"] =
            json!("Running");
    }
    let fault = fault.to_owned();
    let dir = dir.to_owned();
    let calls2 = calls.clone();
    let f=Fixture::new(move |request| {
        let method=request["method"].as_str().unwrap(); let p=&request["params"];
        calls2.lock().unwrap().push(method.to_owned());
        let mut r=records.lock().unwrap();
        if let Some(class)=method.strip_suffix(".get_by_uuid") {
            return r.iter().find(|(k,v)|k.starts_with(&format!("OpaqueRef:{class}:"))&&v["uuid"]==p[1]).map_or(json!({"error":{"message":"UUID_INVALID"}}),|(k,_)|json!({"result":k}));
        }
        if method.ends_with(".get_record") {return r.get(p[1].as_str().unwrap()).map_or(json!({"error":{"message":"HANDLE_INVALID"}}),|v|json!({"result":v}));}
        if let Some(class)=method.strip_suffix(".get_all_records") {return json!({"result":r.iter().filter(|(k,_)|k.starts_with(&format!("OpaqueRef:{class}:"))).collect::<BTreeMap<_,_>>()});}
        if method=="VM.destroy" {
            let first=p[1]=="OpaqueRef:VM:2";
            r.remove(p[1].as_str().unwrap());
            if first {
                let j=Journal::open(&dir).unwrap();
                match fault.as_str() {
                    "stop"=>{j.resource_control(pool(),id,OperationControl::RequestCancellation).unwrap();},
                    "recover"|"ack"|"verifying"=>{
                        j.resource_recover(pool()).unwrap();
                        if fault=="ack" {j.resource_control(pool(),id,OperationControl::AcknowledgeUnknown).unwrap();}
                        if fault=="verifying" {let mut p=j.resource_load(id).unwrap().unwrap(); p.state=OperationState::Verifying; j.resource_save(&mut p).unwrap();}
                    },
                    "changed"=>{r.get_mut("OpaqueRef:VM:3").unwrap()["power_state"]=json!("Running");},
                    _=>{},
                }
            }
            if fault=="lost" {return json!({"_fixture_close_without_reply":true});}
            return json!({"result":null});
        }
        panic!("unexpected fixture RPC: {method}")
    }).await;
    (f, calls)
}
#[tokio::test]
async fn batch_deletion_uses_existing_children_and_stops_after_known_outcome() {
    for (fault, state, count) in [
        ("none", OperationState::Succeeded, 2),
        ("preflight", OperationState::Rejected, 0),
        ("stop", OperationState::PartiallyCompleted, 1),
        ("changed", OperationState::PartiallyCompleted, 1),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let (view, p) = parent();
        let (f, calls) = fixture(fault, dir.path(), p.record.id).await;
        run(p, &f, dir.path()).await;
        let r = view.latest();
        assert_eq!(r.state, state, "{fault}: {}", r.detail);
        assert_eq!(
            calls
                .lock()
                .unwrap()
                .iter()
                .filter(|m| m.as_str() == "VM.destroy")
                .count(),
            count,
            "{fault}"
        );
        let j = Journal::open(dir.path()).unwrap();
        j.workflow_validate_links(&r).unwrap();
        for result in &r.workflow_batch {
            if result.state.is_some() {
                assert_eq!(
                    j.load(result.child).unwrap().unwrap().batch_parent,
                    Some(r.id)
                );
            }
        }
    }
}
#[tokio::test]
async fn recovered_or_acknowledged_late_reply_never_starts_the_next_vm() {
    for fault in ["lost", "recover", "ack"] {
        let dir = tempfile::tempdir().unwrap();
        let (view, p) = parent();
        let id = p.record.id;
        let (f, calls) = fixture(fault, dir.path(), id).await;
        run(p, &f, dir.path()).await;
        let j = Journal::open(dir.path()).unwrap();
        let row = j.resource_load(id).unwrap().unwrap();
        assert_eq!(
            row.state,
            if fault == "ack" {
                OperationState::AcknowledgedUnknown
            } else {
                OperationState::OutcomeUnknown
            },
            "{}",
            view.latest().detail
        );
        let p = row.workflow_batch_plan.as_ref().unwrap();
        assert_eq!(p.admitted, 1);
        let child = j.load(p.children[0]).unwrap().unwrap();
        if fault != "lost" {
            assert!(child.lifecycle_plan.as_ref().unwrap().evidence[0].completed);
        }
        assert!(j.load(p.children[1]).unwrap().is_none());
        if fault != "ack" {
            run(
                crate::resources::ResourceRequest::tracking(row),
                &f,
                dir.path(),
            )
            .await;
            let rechecked = j.resource_load(id).unwrap().unwrap();
            assert_eq!(
                rechecked.state,
                OperationState::PartiallyCompleted,
                "{}",
                rechecked.detail
            );
        }
        assert_eq!(
            calls
                .lock()
                .unwrap()
                .iter()
                .filter(|m| m.as_str() == "VM.destroy")
                .count(),
            1
        );
    }
}
#[tokio::test]
async fn exact_child_grants_exclude_only_the_parent_and_public_controls_refuse() {
    let dir = tempfile::tempdir().unwrap();
    let mut j = Journal::open(dir.path()).unwrap();
    let (_, mut p) = parent();
    j.resource_admit(&mut p.record).unwrap();
    let mut c = child(&p.record, 0);
    assert!(j.admit(&mut c.record).is_err());
    let permit = j.workflow_permit(p.record.id, 0).unwrap();
    let mut wrong = c.record.clone();
    wrong.vm = vm(90);
    assert!(j.admit_owned(&mut wrong, Some(&permit)).is_err());
    j.admit_owned(&mut c.record, Some(&permit)).unwrap();
    assert!(j.control(pool(), c.record.id, false).is_err());
    assert!(
        j.resource_control(pool(), c.record.id, OperationControl::Recheck)
            .is_err()
    );
    let mut next = child(&p.record, 1);
    let second = j.workflow_permit(p.record.id, 1).unwrap();
    assert!(j.admit_owned(&mut next.record, Some(&second)).is_err());
    let (_, mut ordinary) = channel(
        OperationId::new(),
        pool(),
        vm(90),
        4,
        OperationAction::Power(PowerAction::Start),
        tokio::runtime::Handle::current(),
    );
    assert!(matches!(
        j.admit(&mut ordinary.record),
        Err(Error::ConflictingPoolOperation)
    ));
    let (_, mut other) = parent();
    assert!(matches!(
        j.resource_admit(&mut other.record),
        Err(Error::ConflictingPoolOperation)
    ));
    j.workflow_interrupt(pool(), p.record.id, false).unwrap();
    let ack = j
        .resource_control(pool(), p.record.id, OperationControl::AcknowledgeUnknown)
        .unwrap();
    assert_eq!(ack.state, OperationState::AcknowledgedUnknown);
    assert_eq!(
        j.load(c.record.id).unwrap().unwrap().state,
        OperationState::AcknowledgedUnknown
    );
    assert!(j.workflow_child_stopped(c.record.id).unwrap());
    assert!(j.save_vm_dispatch(&mut c.record).is_err());
    assert!(j.admit(&mut ordinary.record).unwrap());
}
#[tokio::test]
async fn late_original_child_receipt_does_not_revive_an_explicit_recheck() {
    let dir = tempfile::tempdir().unwrap();
    let (_, p) = parent();
    let id = p.record.id;
    let (f, calls) = fixture("verifying", dir.path(), id).await;
    run(p, &f, dir.path()).await;
    let j = Journal::open(dir.path()).unwrap();
    let p = j.resource_load(id).unwrap().unwrap();
    assert_eq!(p.state, OperationState::Verifying);
    assert!(p.interrupted);
    let plan = p.workflow_batch_plan.as_ref().unwrap();
    assert_eq!(plan.admitted, 1);
    let child = j.load(plan.children[0]).unwrap().unwrap();
    assert_eq!(child.state, OperationState::OutcomeUnknown);
    assert!(child.lifecycle_plan.unwrap().evidence[0].completed);
    assert_eq!(
        calls
            .lock()
            .unwrap()
            .iter()
            .filter(|m| m.as_str() == "VM.destroy")
            .count(),
        1
    );
}
#[tokio::test]
async fn cancellation_before_admission_and_recovery_fence_pending_children() {
    for recover in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut j = Journal::open(dir.path()).unwrap();
        let (_, mut p) = parent();
        j.resource_admit(&mut p.record).unwrap();
        let mut c = child(&p.record, 0);
        let permit = j.workflow_permit(p.record.id, 0).unwrap();
        if recover {
            j.workflow_interrupt(pool(), p.record.id, false).unwrap();
        } else {
            j.resource_control(pool(), p.record.id, OperationControl::RequestCancellation)
                .unwrap();
        }
        assert!(j.admit_owned(&mut c.record, Some(&permit)).is_err());
        assert!(j.load(c.record.id).unwrap().is_none());
        assert_eq!(
            j.resource_load(p.record.id)
                .unwrap()
                .unwrap()
                .workflow_batch_plan
                .unwrap()
                .admitted,
            0
        );
    }
}
#[tokio::test]
async fn malformed_links_and_immutable_child_intents_fail_closed() {
    let dir = tempfile::tempdir().unwrap();
    let mut j = Journal::open(dir.path()).unwrap();
    let (_, mut p) = parent();
    j.resource_admit(&mut p.record).unwrap();
    let mut changed = j.resource_load(p.record.id).unwrap().unwrap();
    changed
        .workflow_batch_plan
        .as_mut()
        .unwrap()
        .children
        .reverse();
    assert!(j.resource_save(&mut changed).is_err());
    let c = child(&p.record, 0);
    j.0.execute(
        "UPDATE workflow_batch_children SET kind='resource' WHERE child=?1",
        [c.record.id.to_string()],
    )
    .unwrap();
    assert!(j.workflow_owner(c.record.id).is_err());
    let (_, mut ordinary) = channel(
        OperationId::new(),
        pool(),
        vm(91),
        4,
        OperationAction::Power(PowerAction::Start),
        tokio::runtime::Handle::current(),
    );
    assert!(j.admit(&mut ordinary.record).is_err());
    assert!(j.workflow_interrupt(pool(), p.record.id, false).is_err());
}
#[tokio::test]
async fn pruning_keeps_unresolved_batch_evidence_and_removes_terminal_groups_atomically() {
    let dir = tempfile::tempdir().unwrap();
    let mut j = Journal::open(dir.path()).unwrap();
    let (_, mut p) = parent();
    j.resource_admit(&mut p.record).unwrap();
    let mut c = child(&p.record, 0);
    let permit = j.workflow_permit(p.record.id, 0).unwrap();
    j.admit_owned(&mut c.record, Some(&permit)).unwrap();
    j.workflow_interrupt(pool(), p.record.id, false).unwrap();
    // Fill the retention window in another pool without bypassing active reservations.
    let other: PoolId = Uuid::from_u128(90).into();
    for _ in 0..501 {
        let (_, mut r) = crate::resources::channel(
            OperationId::new(),
            crate::ObservedResource {
                pool: other,
                generation: 1,
                target: ResourceTarget::Pool(other),
            },
            ResourceAction::RenameMetadata {
                expected_name: "old".into(),
                expected_description: "".into(),
                name: "new".into(),
                description: "".into(),
            },
            tokio::runtime::Handle::current(),
        );
        j.resource_admit(&mut r.record).unwrap();
        r.record.state = OperationState::Cancelled;
        j.resource_save(&mut r.record).unwrap();
    }
    assert!(j.resource_load(p.record.id).unwrap().is_some());
    assert!(j.load(c.record.id).unwrap().is_some());
    j.resource_control(pool(), p.record.id, OperationControl::AcknowledgeUnknown)
        .unwrap();
    j.0.execute_batch("BEGIN IMMEDIATE").unwrap();
    j.workflow_prune().unwrap();
    j.0.execute_batch("COMMIT").unwrap();
    assert!(j.resource_load(p.record.id).unwrap().is_none());
    assert!(j.load(c.record.id).unwrap().is_none());
    assert_eq!(
        j.0.query_row(
            "SELECT COUNT(*) FROM workflow_batch_children WHERE parent=?1",
            [p.record.id.to_string()],
            |r| r.get::<_, usize>(0)
        )
        .unwrap(),
        0
    );
}
