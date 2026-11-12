use super::*;
use crate::{
    BatchVmWorkflow, OperationControl, PowerState, VmExportFormat, VmWorkflow, VmWorkflowBatch,
};

fn parent(dir: &Path) -> (crate::ResourceOperation, ResourceRequest) {
    let selection = VmWorkflowBatch {
        items: [2, 7]
            .into_iter()
            .map(|n| BatchVmWorkflow {
                vm: uuid(n).into(),
                expected_state: PowerState::Halted,
                workflow: VmWorkflow::Export {
                    expected_template: false,
                    format: VmExportFormat::Xva {
                        compression: crate::XvaCompression::Uncompressed,
                    },
                },
            })
            .collect(),
    };
    let outputs = [2, 7]
        .into_iter()
        .map(|n| {
            (
                uuid(n),
                TransferFile::destination(dir.join(format!("vm-{n}.xva"))).unwrap(),
            )
        })
        .collect();
    let files = crate::vm_workflow_export::bind(&selection, outputs).unwrap();
    let (handle, mut r) = resources::channel(
        OperationId::new(),
        ObservedResource {
            pool: uuid(1).into(),
            target: ResourceTarget::Pool(uuid(1).into()),
            generation: 1,
        },
        crate::ResourceAction::VmWorkflowBatch(selection),
        tokio::runtime::Handle::current(),
    );
    r.record.workflow_batch_plan.as_mut().unwrap().outputs = files;
    (handle, r)
}
async fn fixture() -> Fixture {
    let mut f = Fixture::new(Fault::None).await;
    f.options.read_only = true;
    let mut r = f.records.lock().unwrap();
    let mut second = r["OpaqueRef:VM:source"].clone();
    second["uuid"] = json!(uuid(7));
    r.insert("OpaqueRef:VM:second".into(), second);
    r.insert("fixture:batch_export".into(), json!(true));
    drop(r);
    f
}
async fn run(f: &Fixture, dir: &Path, r: ResourceRequest) {
    let (tx, mut rx) = mpsc::channel(128);
    let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
    tokio::time::timeout(
        std::time::Duration::from_secs(20),
        r.run(
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
#[tokio::test]
async fn resource_child_grants_bind_kind_target_action_and_private_output() {
    let dir = tempfile::tempdir().unwrap();
    let (_, mut p) = parent(dir.path());
    let j = Journal::open(dir.path()).unwrap();
    j.resource_admit(&mut p.record).unwrap();
    let plan = p.record.workflow_batch_plan.as_ref().unwrap();
    let item = &plan.selection.items[0];
    let (_, mut child) = resources::transfer_channel(
        plan.children[0],
        ObservedResource {
            pool: p.record.pool,
            target: p.record.target,
            generation: 1,
        },
        TransferRequest {
            action: item.workflow.transfer(item.vm).unwrap(),
            file: plan.outputs[0].clone(),
        },
        tokio::runtime::Handle::current(),
    );
    child.record.batch_parent = Some(p.record.id);
    let permit = j.workflow_permit(p.record.id, 0).unwrap();
    assert!(j.resource_admit(&mut child.record).is_err());
    for corruption in 0..4 {
        let mut bad = child.record.clone();
        match corruption {
            0 => bad.pool = uuid(77).into(),
            1 => bad.target = ResourceTarget::Vm(uuid(2).into()),
            2 => bad.action = crate::ResourceAction::ScanStorage,
            _ => {
                bad.transfer_plan.as_mut().unwrap().file =
                    TransferFile::destination(dir.path().join("wrong.xva")).unwrap()
            }
        };
        assert!(j.resource_admit_permitted(&mut bad, Some(&permit)).is_err());
    }
    j.resource_admit_permitted(&mut child.record, Some(&permit))
        .unwrap();
    assert!(
        j.resource_control(
            p.record.pool,
            child.record.id,
            OperationControl::RequestCancellation
        )
        .is_err()
    );
    let mut forged = child.record.clone();
    forged.batch_parent = None;
    assert!(j.resource_save(&mut forged).is_err());
    j.workflow_interrupt(p.record.pool, p.record.id, false)
        .unwrap();
    let mut c = j.resource_load(child.record.id).unwrap().unwrap();
    assert!(c.interrupted);
    assert!(j.resource_dispatch(&mut c).is_err());
    j.workflow_interrupt(p.record.pool, p.record.id, true)
        .unwrap();
    assert_eq!(
        j.resource_load(child.record.id).unwrap().unwrap().state,
        OperationState::AcknowledgedUnknown
    );
    assert!(j.resource_dispatch(&mut c).is_err());
    j.0.execute(
        "UPDATE workflow_batch_children SET kind='vm' WHERE parent=?1",
        [p.record.id.to_string()],
    )
    .unwrap();
    assert!(j.resource_load(p.record.id).is_err());
}
#[test]
fn destination_aliases_overlap_and_replaced_parent_inodes_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let parent = dir.path().join("parent");
    std::fs::create_dir(&parent).unwrap();
    let one = TransferFile::destination(parent.join("out.xva")).unwrap();
    let mut nested = one.clone();
    nested.path = nested.path.join("inside");
    assert!(crate::vm_workflow_export::validate(&[one.clone(), nested]).is_err());
    std::os::unix::fs::symlink(&parent, dir.path().join("alias")).unwrap();
    let alias = TransferFile::destination(dir.path().join("alias/out.xva")).unwrap();
    assert!(crate::vm_workflow_export::validate(&[one.clone(), alias]).is_err());
    std::fs::rename(&parent, dir.path().join("old")).unwrap();
    std::fs::create_dir(&parent).unwrap();
    assert!(crate::vm_workflow_export::preflight(&[one]).is_err());
}
#[tokio::test]
async fn batch_exports_use_bound_resource_children_on_read_only_session() {
    let f = fixture().await;
    let dir = tempfile::tempdir().unwrap();
    let (_, r) = parent(dir.path());
    let id = r.record.id;
    assert!(r.record.action.read_only_export());
    run(&f, dir.path(), r).await;
    let j = Journal::open(dir.path()).unwrap();
    let p = j.resource_load(id).unwrap().unwrap();
    assert_eq!(p.state, OperationState::Succeeded, "{}", p.detail);
    assert_eq!(f.count("GET /export"), 2);
    for n in [2, 7] {
        let file = TransferFile::source(dir.path().join(format!("vm-{n}.xva"))).unwrap();
        assert_eq!(file.archive().unwrap().guests[0].uuid, uuid(n));
    }
    for item in &p.workflow_batch {
        let c = j.resource_load(item.child).unwrap().unwrap();
        assert_eq!(c.batch_parent, Some(id));
        assert_eq!(c.state, OperationState::Succeeded);
        assert!(
            j.resource_control(p.pool, c.id, OperationControl::Recheck)
                .is_err()
        );
    }
    assert!(!serde_json::to_string(&p).unwrap().contains("vm-2.xva"));
}
#[tokio::test]
async fn all_export_destinations_and_sources_are_checked_before_first_task() {
    for local in [false, true] {
        let f = fixture().await;
        let dir = tempfile::tempdir().unwrap();
        let (_, r) = parent(dir.path());
        let id = r.record.id;
        if local {
            std::fs::write(dir.path().join("vm-7.xva"), b"keep").unwrap()
        } else {
            f.records
                .lock()
                .unwrap()
                .get_mut("OpaqueRef:VM:second")
                .unwrap()["power_state"] = json!("Running")
        }
        run(&f, dir.path(), r).await;
        let p = Journal::open(dir.path())
            .unwrap()
            .resource_load(id)
            .unwrap()
            .unwrap();
        assert!(p.state.terminal());
        assert_eq!(f.count("task.create"), 0);
        assert_eq!(f.count("GET /export"), 0);
        assert!(!dir.path().join("vm-2.xva").exists());
        if local {
            assert_eq!(std::fs::read(dir.path().join("vm-7.xva")).unwrap(), b"keep")
        }
    }
}
#[tokio::test]
async fn batch_export_stop_finishes_current_and_recovery_never_publishes_or_starts_next() {
    for mode in ["stop", "recover", "ack"] {
        let f = fixture().await;
        f.records
            .lock()
            .unwrap()
            .insert("fixture:batch_task_gate".into(), json!(true));
        let dir = tempfile::tempdir().unwrap();
        let (_, r) = parent(dir.path());
        let id = r.record.id;
        let (tx, mut rx) = mpsc::channel(128);
        let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
        let job = tokio::spawn(r.run(
            Arc::new(f.session().await),
            dir.path().into(),
            CancellationToken::new(),
            tx,
        ));
        f.response_gate.wait().await;
        let j = Journal::open(dir.path()).unwrap();
        let p = j.resource_load(id).unwrap().unwrap();
        let child = p.workflow_batch[0].child;
        assert!(
            j.resource_load(child)
                .unwrap()
                .unwrap()
                .transfer_plan
                .as_ref()
                .unwrap()
                .stream_receipt
                .is_some()
        );
        if mode == "stop" {
            j.resource_control(p.pool, id, OperationControl::RequestCancellation)
                .unwrap();
        } else {
            j.workflow_interrupt(p.pool, id, false).unwrap();
            if mode == "ack" {
                j.workflow_interrupt(p.pool, id, true).unwrap();
            }
        }
        f.response_gate.resume();
        tokio::time::timeout(std::time::Duration::from_secs(20), job)
            .await
            .unwrap()
            .unwrap();
        drain.abort();
        let p = j.resource_load(id).unwrap().unwrap();
        assert_eq!(f.count("GET /export"), 1);
        assert!(!dir.path().join("vm-7.xva").exists());
        if mode == "stop" {
            assert_eq!(p.state, OperationState::PartiallyCompleted, "{}", p.detail);
            assert!(dir.path().join("vm-2.xva").exists());
        } else {
            assert!(!dir.path().join("vm-2.xva").exists());
            assert!(
                j.resource_load(child)
                    .unwrap()
                    .unwrap()
                    .transfer_plan
                    .as_ref()
                    .unwrap()
                    .task_success
            );
            if mode == "recover" {
                let before = f.calls.lock().unwrap().len();
                run(&f, dir.path(), ResourceRequest::tracking(p)).await;
                assert!(f.calls.lock().unwrap()[before..].iter().all(
                    |m| m.starts_with("session.")
                        || m.ends_with("get_record")
                        || m.ends_with("get_by_uuid")
                ));
                assert!(!dir.path().join("vm-2.xva").exists());
            }
        }
    }
}
