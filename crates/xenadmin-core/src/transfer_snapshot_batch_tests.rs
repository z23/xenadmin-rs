use super::*;
use crate::{
    BatchVmWorkflow, OperationControl, PowerState, ResourceAction, ReviewedCheckpoint,
    VmExportFormat, VmWorkflow, VmWorkflowBatch,
};

const PARENT: u128 = 9;
const SNAPSHOT: u128 = 2;
const CHECKPOINT: u128 = 7;
const MEMORY: u128 = 10;

fn xva() -> VmExportFormat {
    VmExportFormat::Xva {
        compression: crate::XvaCompression::Uncompressed,
    }
}
fn memory() -> CheckpointMemory {
    CheckpointMemory {
        vdi: uuid(MEMORY).into(),
        sr: uuid(4).into(),
        virtual_size: 2048,
    }
}
fn snapshot_source(snapshot: u128) -> ExportSource {
    ExportSource::Snapshot {
        snapshot: uuid(snapshot).into(),
        parent: uuid(PARENT).into(),
    }
}
fn checkpoint_source(preserve_memory: bool) -> ExportSource {
    ExportSource::Checkpoint {
        snapshot: uuid(CHECKPOINT).into(),
        parent: uuid(PARENT).into(),
        memory: memory(),
        preserve_memory,
    }
}
fn item(source: &ExportSource) -> BatchVmWorkflow {
    BatchVmWorkflow::snapshot_export(source, xva()).unwrap()
}
fn output(dir: &Path, item: &BatchVmWorkflow) -> std::path::PathBuf {
    dir.join(format!("snapshot-{}.xva", item.subject().as_u128()))
}
fn parent(dir: &Path, items: Vec<BatchVmWorkflow>) -> ResourceRequest {
    let selection = VmWorkflowBatch { items };
    let outputs = selection
        .items
        .iter()
        .map(|i| {
            (
                i.subject(),
                TransferFile::destination(output(dir, i)).unwrap(),
            )
        })
        .collect();
    let files = crate::vm_workflow_export::bind(&selection, outputs).unwrap();
    let (_, mut r) = resources::channel(
        OperationId::new(),
        ObservedResource {
            pool: uuid(1).into(),
            target: ResourceTarget::Pool(uuid(1).into()),
            generation: 1,
        },
        ResourceAction::VmWorkflowBatch(selection),
        tokio::runtime::Handle::current(),
    );
    r.record.workflow_batch_plan.as_mut().unwrap().outputs = files;
    r
}
/// One VM with a disk snapshot and a memory checkpoint, seen through a read-only session.
async fn fixture() -> Fixture {
    let mut f = Fixture::new(Fault::None).await;
    f.options.read_only = true;
    let mut r = f.records.lock().unwrap();
    let mut snapshot = r["OpaqueRef:VM:source"].clone();
    snapshot["is_a_snapshot"] = json!(true);
    snapshot["snapshot_of"] = json!("OpaqueRef:VM:parent");
    snapshot["suspend_VDI"] = json!("OpaqueRef:NULL");
    let mut checkpoint = snapshot.clone();
    checkpoint["uuid"] = json!(uuid(CHECKPOINT));
    checkpoint["power_state"] = json!("Suspended");
    checkpoint["suspend_VDI"] = json!("OpaqueRef:VDI:memory");
    r.insert("OpaqueRef:VM:source".into(), snapshot);
    r.insert("OpaqueRef:VM:checkpoint".into(), checkpoint);
    r.insert(
        "OpaqueRef:VM:parent".into(),
        json!({"uuid":uuid(PARENT),"suspend_VDI":"OpaqueRef:NULL"}),
    );
    r.insert("OpaqueRef:VDI:memory".into(),json!({"uuid":uuid(MEMORY),"SR":"OpaqueRef:SR:s","virtual_size":"2048","type":"suspend","managed":true,"missing":false,"current_operations":{},"VBDs":[]}));
    r.insert("fixture:batch_snapshot_export".into(), json!(true));
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
/// Starts the batch and returns once the fixture holds the reply named by `gate`.
async fn run_to_gate(
    f: &Fixture,
    dir: &Path,
    r: ResourceRequest,
    gate: &str,
) -> (
    tokio::task::JoinHandle<OperationId>,
    tokio::task::JoinHandle<()>,
) {
    f.records.lock().unwrap().insert(gate.into(), json!(true));
    let (tx, mut rx) = mpsc::channel(128);
    let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let job = tokio::spawn(r.run(
        Arc::new(f.session().await),
        dir.into(),
        CancellationToken::new(),
        tx,
    ));
    f.response_gate.wait().await;
    (job, drain)
}
async fn finish(
    f: &Fixture,
    job: tokio::task::JoinHandle<OperationId>,
    drain: tokio::task::JoinHandle<()>,
) {
    f.response_gate.resume();
    tokio::time::timeout(std::time::Duration::from_secs(20), job)
        .await
        .unwrap()
        .unwrap();
    drain.abort();
}
fn only_reads(calls: &[String]) -> bool {
    calls.iter().all(|m| {
        m.starts_with("session.") || m.ends_with("get_record") || m.ends_with("get_by_uuid")
    })
}

#[test]
fn snapshot_items_are_closed_over_identity_state_memory_and_format() {
    let pool = ResourceTarget::Pool(uuid(1).into());
    let valid = |items: Vec<BatchVmWorkflow>| VmWorkflowBatch { items }.validate(pool).is_ok();
    let plain = item(&snapshot_source(SNAPSHOT));
    let checkpoint = item(&checkpoint_source(true));
    assert_eq!(plain.vm, uuid(PARENT).into());
    assert_eq!(plain.expected_state, PowerState::Halted);
    assert_eq!(checkpoint.expected_state, PowerState::Suspended);
    assert_eq!(
        checkpoint.workflow.transfer(checkpoint.vm),
        Some(TransferAction::ExportXva {
            source: checkpoint_source(true),
            compression: crate::XvaCompression::Uncompressed,
        })
    );
    assert_eq!(
        plain.workflow.transfer(plain.vm),
        Some(TransferAction::ExportXva {
            source: snapshot_source(SNAPSHOT),
            compression: crate::XvaCompression::Uncompressed,
        })
    );
    assert!(
        BatchVmWorkflow::snapshot_export(
            &ExportSource::Vm {
                vm: uuid(PARENT).into(),
                expected_template: false,
            },
            xva(),
        )
        .is_none()
    );
    // Several snapshots of one VM are one valid selection, in either order.
    assert!(valid(vec![plain.clone(), checkpoint.clone()]));
    assert!(valid(vec![checkpoint.clone(), plain.clone()]));
    let batch = VmWorkflowBatch {
        items: vec![plain.clone(), checkpoint.clone()],
    };
    assert!(batch.is_export() && batch.is_snapshot_export());
    assert_eq!(
        ResourceAction::VmWorkflowBatch(batch.clone()).label(),
        "Export selected snapshots"
    );
    assert!(
        batch
            .validate(ResourceTarget::Vm(uuid(PARENT).into()))
            .is_err()
    );
    assert!(!valid(vec![plain.clone(), plain.clone()]));
    assert!(!valid(vec![plain.clone(); 33]));
    // A checkpoint is only ever an item with its memory image reviewed, and the reverse.
    let mut unreviewed = checkpoint.clone();
    unreviewed.expected_state = PowerState::Halted;
    assert!(!valid(vec![unreviewed]));
    let mut suspended = plain.clone();
    suspended.expected_state = PowerState::Suspended;
    assert!(!valid(vec![suspended]));
    let edit = |change: &dyn Fn(&mut ReviewedCheckpoint, &mut VmExportFormat)| {
        let mut changed = checkpoint.clone();
        let VmWorkflow::ExportSnapshot {
            checkpoint: Some(reviewed),
            format,
            ..
        } = &mut changed.workflow
        else {
            unreachable!()
        };
        change(reviewed, format);
        changed
    };
    assert!(!valid(vec![edit(&|m, _| {
        m.memory.vdi = uuid::Uuid::nil().into()
    })]));
    assert!(!valid(vec![edit(&|m, _| {
        m.memory.sr = uuid::Uuid::nil().into()
    })]));
    assert!(!valid(vec![edit(&|m, _| m.memory.virtual_size = 0)]));
    assert!(!valid(vec![edit(&|m, _| {
        m.memory.virtual_size = MAX_TRANSFER_BYTES + 1
    })]));
    // The memory choice does not change what is valid; a package format does.
    assert!(valid(vec![edit(&|m, _| m.preserve_memory = false)]));
    for format in [crate::OvfPackageFormat::Ovf, crate::OvfPackageFormat::Ova] {
        for preserve in [false, true] {
            assert!(!valid(vec![edit(&|m, f| {
                m.preserve_memory = preserve;
                *f = VmExportFormat::Ovf { format };
            })]));
        }
        let mut package = plain.clone();
        let VmWorkflow::ExportSnapshot { format: f, .. } = &mut package.workflow else {
            unreachable!()
        };
        *f = VmExportFormat::Ovf { format };
        assert!(valid(vec![package]));
    }
    // Two checkpoints cannot claim one memory image, and a snapshot is not its own parent.
    let mut second = checkpoint.clone();
    let VmWorkflow::ExportSnapshot { snapshot, .. } = &mut second.workflow else {
        unreachable!()
    };
    *snapshot = uuid(8).into();
    assert!(!valid(vec![checkpoint.clone(), second]));
    let mut own_parent = plain.clone();
    own_parent.vm = uuid(SNAPSHOT).into();
    assert!(!valid(vec![own_parent]));
    let mut nil = plain.clone();
    let VmWorkflow::ExportSnapshot { snapshot, .. } = &mut nil.workflow else {
        unreachable!()
    };
    *snapshot = uuid::Uuid::nil().into();
    assert!(!valid(vec![nil]));
    // A batch is of VMs or of snapshots, never both.
    let vm = BatchVmWorkflow {
        vm: uuid(PARENT).into(),
        expected_state: PowerState::Halted,
        workflow: VmWorkflow::Export {
            expected_template: false,
            format: xva(),
        },
    };
    assert!(!valid(vec![plain.clone(), vm.clone()]));
    assert!(!valid(vec![vm, plain]));
}

#[test]
fn each_output_is_bound_to_its_snapshot_and_never_to_the_vm_the_snapshots_share() {
    let dir = tempfile::tempdir().unwrap();
    let selection = VmWorkflowBatch {
        items: vec![
            item(&snapshot_source(SNAPSHOT)),
            item(&checkpoint_source(true)),
        ],
    };
    let file =
        |name: &str| TransferFile::destination(dir.path().join(name)).expect("a new destination");
    let bind = |outputs| crate::vm_workflow_export::bind(&selection, outputs);
    // Given in the other order, each file still goes with the snapshot it was chosen for.
    let bound = bind(vec![
        (uuid(CHECKPOINT), file("checkpoint.xva")),
        (uuid(SNAPSHOT), file("snapshot.xva")),
    ])
    .unwrap();
    assert_eq!(
        bound.iter().map(TransferFile::name).collect::<Vec<_>>(),
        ["snapshot.xva", "checkpoint.xva"]
    );
    for refused in [
        // Named by the parent, as a batch of VMs names its outputs.
        vec![
            (uuid(PARENT), file("a.xva")),
            (uuid(CHECKPOINT), file("b.xva")),
        ],
        vec![(uuid(PARENT), file("a.xva")), (uuid(PARENT), file("b.xva"))],
        vec![(uuid(SNAPSHOT), file("a.xva"))],
        vec![
            (uuid(SNAPSHOT), file("a.xva")),
            (uuid(CHECKPOINT), file("b.xva")),
            (uuid(8), file("c.xva")),
        ],
        // One destination for two snapshots.
        vec![
            (uuid(SNAPSHOT), file("a.xva")),
            (uuid(CHECKPOINT), file("a.xva")),
        ],
    ] {
        assert!(bind(refused).is_err());
    }
}

#[tokio::test]
async fn two_snapshot_batch_exports_a_disk_snapshot_and_a_checkpoint_with_reviewed_memory() {
    let f = fixture().await;
    let dir = tempfile::tempdir().unwrap();
    let items = vec![
        item(&snapshot_source(SNAPSHOT)),
        item(&checkpoint_source(true)),
    ];
    let r = parent(dir.path(), items.clone());
    let id = r.record.id;
    assert!(r.record.action.read_only_export());
    assert_eq!(
        r.record
            .steps
            .iter()
            .map(|s| s.label.clone())
            .collect::<Vec<_>>(),
        [SNAPSHOT, CHECKPOINT].map(|n| format!("Export snapshot {}", uuid(n)))
    );
    run(&f, dir.path(), r).await;
    let j = Journal::open(dir.path()).unwrap();
    let p = j.resource_load(id).unwrap().unwrap();
    assert_eq!(p.state, OperationState::Succeeded, "{}", p.detail);
    assert!(
        p.detail.starts_with("2 of 2 snapshot workflows verified"),
        "{}",
        p.detail
    );
    assert_eq!(f.count("task.create"), 2);
    assert_eq!(f.count("GET /export"), 2);
    let plain = TransferFile::source(output(dir.path(), &items[0])).unwrap();
    let plain = plain.archive().unwrap();
    assert_eq!(plain.guests.len(), 1);
    assert_eq!(plain.guests[0].uuid, uuid(SNAPSHOT));
    assert_eq!(plain.memory_images(), 0);
    let checkpoint = TransferFile::source(output(dir.path(), &items[1])).unwrap();
    let checkpoint = checkpoint.archive().unwrap();
    assert_eq!(checkpoint.guests.len(), 1);
    assert_eq!(checkpoint.guests[0].uuid, uuid(CHECKPOINT));
    assert_eq!(
        checkpoint.guests[0].memory,
        Some(ArchiveMemory {
            uuid: uuid(MEMORY),
            virtual_size: 2048,
        })
    );
    for (result, item) in p.workflow_batch.iter().zip(&items) {
        assert_eq!(result.vm, uuid(PARENT).into());
        let c = j.resource_load(result.child).unwrap().unwrap();
        assert_eq!(c.batch_parent, Some(id));
        assert_eq!(c.state, OperationState::Succeeded);
        assert_eq!(
            c.action,
            ResourceAction::Transfer(item.workflow.transfer(item.vm).unwrap())
        );
        assert!(
            j.resource_control(p.pool, c.id, OperationControl::Recheck)
                .is_err()
        );
    }
    assert!(
        !serde_json::to_string(&p)
            .unwrap()
            .contains("snapshot-2.xva")
    );
}

#[tokio::test]
async fn one_snapshot_failing_preflight_refuses_the_whole_selection_before_any_transfer() {
    for fault in [
        "memory-size",
        "memory-busy",
        "reparented",
        "gone",
        "destination",
    ] {
        let f = fixture().await;
        let dir = tempfile::tempdir().unwrap();
        // The disk snapshot comes first and would pass by itself.
        let items = vec![
            item(&snapshot_source(SNAPSHOT)),
            item(&checkpoint_source(true)),
        ];
        let r = parent(dir.path(), items.clone());
        let id = r.record.id;
        {
            let mut records = f.records.lock().unwrap();
            match fault {
                "memory-size" => {
                    records.get_mut("OpaqueRef:VDI:memory").unwrap()["virtual_size"] = json!("4096")
                }
                "memory-busy" => {
                    records.get_mut("OpaqueRef:VDI:memory").unwrap()["current_operations"] =
                        json!({"OpaqueRef:t":"resize"})
                }
                "reparented" => {
                    records.get_mut("OpaqueRef:VM:checkpoint").unwrap()["snapshot_of"] =
                        json!("OpaqueRef:VM:other")
                }
                "gone" => {
                    records.remove("OpaqueRef:VM:checkpoint");
                }
                _ => std::fs::write(output(dir.path(), &items[1]), b"keep").unwrap(),
            }
        }
        run(&f, dir.path(), r).await;
        let p = Journal::open(dir.path())
            .unwrap()
            .resource_load(id)
            .unwrap()
            .unwrap();
        assert!(p.state.terminal(), "{fault}: {:?}", p.state);
        assert_ne!(p.state, OperationState::Succeeded, "{fault}");
        assert_eq!(p.workflow_batch_plan.as_ref().unwrap().admitted, 0);
        assert!(p.workflow_batch.iter().all(|r| r.state.is_none()));
        assert_eq!(f.count("task.create"), 0, "{fault}");
        assert_eq!(f.count("GET /export"), 0, "{fault}");
        assert!(!output(dir.path(), &items[0]).exists(), "{fault}");
        if fault == "destination" {
            assert_eq!(
                std::fs::read(output(dir.path(), &items[1])).unwrap(),
                b"keep"
            )
        } else {
            assert!(!output(dir.path(), &items[1]).exists(), "{fault}");
        }
    }
}

#[tokio::test]
async fn a_checkpoint_archive_carries_memory_only_when_its_item_chose_it() {
    // Reviewed, memory not chosen: the host is asked for disks only and the archive that
    // is published holds a halted guest without a memory image.
    let f = fixture().await;
    let dir = tempfile::tempdir().unwrap();
    let items = vec![
        item(&checkpoint_source(false)),
        item(&snapshot_source(SNAPSHOT)),
    ];
    let r = parent(dir.path(), items.clone());
    let id = r.record.id;
    run(&f, dir.path(), r).await;
    let j = Journal::open(dir.path()).unwrap();
    let p = j.resource_load(id).unwrap().unwrap();
    assert_eq!(p.state, OperationState::Succeeded, "{}", p.detail);
    assert_eq!(
        std::fs::read(output(dir.path(), &items[0])).unwrap(),
        checkpoint_tests::archive(CHECKPOINT, MEMORY, false)
    );
    let archive = TransferFile::source(output(dir.path(), &items[0])).unwrap();
    assert_eq!(archive.archive().unwrap().memory_images(), 0);
    let child = j.resource_load(p.workflow_batch[0].child).unwrap().unwrap();
    assert_eq!(
        child.action,
        ResourceAction::Transfer(TransferAction::ExportXva {
            source: checkpoint_source(false),
            compression: crate::XvaCompression::Uncompressed,
        })
    );

    // A host that returns the memory image although the item left it out is not believed.
    let f = fixture().await;
    f.records
        .lock()
        .unwrap()
        .insert("fixture:batch_snapshot_memory_always".into(), json!(true));
    let dir = tempfile::tempdir().unwrap();
    let r = parent(dir.path(), items.clone());
    let id = r.record.id;
    run(&f, dir.path(), r).await;
    let p = Journal::open(dir.path())
        .unwrap()
        .resource_load(id)
        .unwrap()
        .unwrap();
    assert_eq!(p.state, OperationState::OutcomeUnknown, "{}", p.detail);
    assert_eq!(
        p.workflow_batch[0].state,
        Some(OperationState::OutcomeUnknown)
    );
    assert!(p.workflow_batch[1].state.is_none());
    assert_eq!(f.count("GET /export"), 1);
    assert!(items.iter().all(|i| !output(dir.path(), i).exists()));

    // Not reviewed at all: a checkpoint named as an ordinary snapshot has no memory image
    // on record, so nothing of the selection starts.
    let f = fixture().await;
    let dir = tempfile::tempdir().unwrap();
    let items = vec![
        item(&snapshot_source(SNAPSHOT)),
        item(&snapshot_source(CHECKPOINT)),
    ];
    let r = parent(dir.path(), items.clone());
    let id = r.record.id;
    run(&f, dir.path(), r).await;
    let p = Journal::open(dir.path())
        .unwrap()
        .resource_load(id)
        .unwrap()
        .unwrap();
    assert!(p.state.terminal());
    assert_ne!(p.state, OperationState::Succeeded);
    assert_eq!(f.count("task.create"), 0);
    assert_eq!(f.count("GET /export"), 0);
    assert!(items.iter().all(|i| !output(dir.path(), i).exists()));
}

#[tokio::test]
async fn stop_finishes_the_current_snapshot_and_recovery_never_publishes_or_starts_the_next() {
    for mode in ["stop", "recover", "ack"] {
        let f = fixture().await;
        let dir = tempfile::tempdir().unwrap();
        // The checkpoint with its memory is the child that is under way.
        let items = vec![
            item(&checkpoint_source(true)),
            item(&snapshot_source(SNAPSHOT)),
        ];
        let r = parent(dir.path(), items.clone());
        let id = r.record.id;
        let (job, drain) = run_to_gate(&f, dir.path(), r, "fixture:batch_task_gate").await;
        let j = Journal::open(dir.path()).unwrap();
        let p = j.resource_load(id).unwrap().unwrap();
        let child = p.workflow_batch[0].child;
        let receipt = |j: &Journal| {
            let plan = j.resource_load(child).unwrap().unwrap().transfer_plan;
            let plan = plan.unwrap();
            (plan.stream_receipt.is_some(), plan.task_success)
        };
        assert!(receipt(&j).0);
        if mode == "stop" {
            j.resource_control(p.pool, id, OperationControl::RequestCancellation)
                .unwrap();
        } else {
            j.workflow_interrupt(p.pool, id, false).unwrap();
            if mode == "ack" {
                j.workflow_interrupt(p.pool, id, true).unwrap();
            }
        }
        finish(&f, job, drain).await;
        let p = j.resource_load(id).unwrap().unwrap();
        assert_eq!(f.count("GET /export"), 1, "{mode}");
        assert_eq!(p.workflow_batch_plan.as_ref().unwrap().admitted, 1);
        assert!(p.workflow_batch[1].state.is_none());
        assert!(!output(dir.path(), &items[1]).exists(), "{mode}");
        if mode == "stop" {
            assert_eq!(p.state, OperationState::PartiallyCompleted, "{}", p.detail);
            assert!(
                p.detail
                    .starts_with("1 of 2 snapshot workflows verified; 1 not started"),
                "{}",
                p.detail
            );
            let archive = TransferFile::source(output(dir.path(), &items[0])).unwrap();
            assert_eq!(archive.archive().unwrap().memory_images(), 1);
            continue;
        }
        // The late task receipt is kept, and the interrupted child publishes nothing.
        assert!(receipt(&j).1, "{mode}");
        assert!(!output(dir.path(), &items[0]).exists(), "{mode}");
        if mode == "ack" {
            assert_eq!(p.state, OperationState::AcknowledgedUnknown);
            assert_eq!(
                j.resource_load(child).unwrap().unwrap().state,
                OperationState::AcknowledgedUnknown
            );
            continue;
        }
        assert_eq!(p.state, OperationState::OutcomeUnknown);
        assert_eq!(
            p.detail,
            "Batch interrupted; recheck reads admitted children and never starts remaining snapshots"
        );
        // Recheck reads what exists. It does not download, rename, cancel or start a child.
        let before = f.calls.lock().unwrap().len();
        run(&f, dir.path(), ResourceRequest::tracking(p)).await;
        assert!(only_reads(&f.calls.lock().unwrap()[before..]));
        assert_eq!(f.count("GET /export"), 1);
        assert_eq!(f.count("task.create"), 1);
        assert!(items.iter().all(|i| !output(dir.path(), i).exists()));
        let p = j.resource_load(id).unwrap().unwrap();
        assert_eq!(p.workflow_batch_plan.as_ref().unwrap().admitted, 1);
        assert!(
            j.resource_load(p.workflow_batch[1].child)
                .unwrap()
                .is_none()
        );
    }
}

#[tokio::test]
async fn recheck_after_an_interruption_between_snapshots_downloads_nothing_and_settles_the_unsent_child()
 {
    let f = fixture().await;
    let dir = tempfile::tempdir().unwrap();
    let items = vec![
        item(&snapshot_source(SNAPSHOT)),
        item(&checkpoint_source(true)),
    ];
    let r = parent(dir.path(), items.clone());
    let id = r.record.id;
    // The first archive is published; the second child has asked for its task and has
    // sent nothing else when the batch is interrupted.
    let (job, drain) = run_to_gate(&f, dir.path(), r, "fixture:batch_next_task_gate").await;
    let j = Journal::open(dir.path()).unwrap();
    let p = j.resource_load(id).unwrap().unwrap();
    assert_eq!(p.workflow_batch[0].state, Some(OperationState::Succeeded));
    let published = std::fs::read(output(dir.path(), &items[0])).unwrap();
    j.workflow_interrupt(p.pool, id, false).unwrap();
    finish(&f, job, drain).await;
    let p = j.resource_load(id).unwrap().unwrap();
    assert_eq!(p.state, OperationState::OutcomeUnknown);
    assert_eq!(p.workflow_batch_plan.as_ref().unwrap().admitted, 2);
    let second = j.resource_load(p.workflow_batch[1].child).unwrap().unwrap();
    assert!(second.interrupted);
    assert!(!second.transfer_plan.as_ref().unwrap().dispatched);
    assert_eq!(f.count("GET /export"), 1);
    for _ in 0..2 {
        let before = f.calls.lock().unwrap().len();
        let p = j.resource_load(id).unwrap().unwrap();
        run(&f, dir.path(), ResourceRequest::tracking(p)).await;
        assert!(only_reads(&f.calls.lock().unwrap()[before..]));
        assert_eq!(f.count("GET /export"), 1);
        assert_eq!(f.count("task.create"), 2);
        assert_eq!(
            std::fs::read(output(dir.path(), &items[0])).unwrap(),
            published
        );
        assert!(!output(dir.path(), &items[1]).exists());
        let p = j.resource_load(id).unwrap().unwrap();
        // The second child's task was created and nothing was streamed. The journal shows
        // that exactly, because a transfer is marked as sent by the save that precedes
        // its request: the child ends as not sent and the batch as partly completed,
        // with nothing left for the operator to vouch for. A second recheck changes
        // nothing.
        assert_eq!(p.state, OperationState::PartiallyCompleted, "{}", p.detail);
        assert_eq!(p.workflow_batch[0].state, Some(OperationState::Succeeded));
        assert_eq!(p.workflow_batch[1].state, Some(OperationState::Cancelled));
        let second = j.resource_load(p.workflow_batch[1].child).unwrap().unwrap();
        assert_eq!(second.state, OperationState::Cancelled);
        assert!(
            second
                .detail
                .contains("Interrupted before the transfer was sent"),
            "{}",
            second.detail
        );
    }
    assert_eq!(f.count("GET /export"), 1);
    assert_eq!(
        std::fs::read(output(dir.path(), &items[0])).unwrap(),
        published
    );
}

fn journal_format(j: &Journal) -> u64 {
    j.0.query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap()
}
/// The batch workflows a build that stops at journal format 4 knows.
#[derive(Debug, serde::Deserialize)]
enum Format4Workflow {
    Export(#[allow(dead_code)] Value),
    Move(#[allow(dead_code)] Value),
    Delete(#[allow(dead_code)] Value),
}
fn stored_workflow(j: &Journal, id: OperationId) -> Value {
    let payload: String =
        j.0.query_row(
            "SELECT payload FROM resource_operations WHERE id=?1",
            [id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    let payload: Value = serde_json::from_str(&payload).unwrap();
    let workflow = &payload["action"]["VmWorkflowBatch"]["items"][0]["workflow"];
    assert_eq!(
        *workflow,
        payload["workflow_batch_plan"]["selection"]["items"][0]["workflow"]
    );
    workflow.clone()
}

#[tokio::test]
async fn format_4_journal_migrates_whole_to_format_5_which_alone_may_hold_a_snapshot_batch() {
    let state = tempfile::tempdir().unwrap();
    let outputs = tempfile::tempdir().unwrap();
    let pool = uuid(1).into();
    // What the release before writes: a VM export batch with its first child admitted.
    let mut old = parent(
        outputs.path(),
        [20, 21]
            .into_iter()
            .map(|n| BatchVmWorkflow {
                vm: uuid(n).into(),
                expected_state: PowerState::Halted,
                workflow: VmWorkflow::Export {
                    expected_template: false,
                    format: xva(),
                },
            })
            .collect(),
    );
    let (parent_row, child_row) = {
        let j = Journal::open(state.path()).unwrap();
        assert_eq!(journal_format(&j), 5);
        j.resource_admit(&mut old.record).unwrap();
        let plan = old.record.workflow_batch_plan.as_ref().unwrap();
        let first = &plan.selection.items[0];
        let (_, mut child) = resources::transfer_channel(
            plan.children[0],
            ObservedResource {
                pool,
                target: ResourceTarget::Pool(pool),
                generation: 1,
            },
            TransferRequest {
                action: first.workflow.transfer(first.vm).unwrap(),
                file: plan.outputs[0].clone(),
            },
            tokio::runtime::Handle::current(),
        );
        child.record.batch_parent = Some(old.record.id);
        let permit = j.workflow_permit(old.record.id, 0).unwrap();
        j.resource_admit_permitted(&mut child.record, Some(&permit))
            .unwrap();
        assert!(matches!(
            serde_json::from_value(stored_workflow(&j, old.record.id)),
            Ok(Format4Workflow::Export(_))
        ));
        j.0.execute_batch("PRAGMA user_version=4;").unwrap();
        (
            j.resource_load(old.record.id).unwrap().unwrap(),
            j.resource_load(child.record.id).unwrap().unwrap(),
        )
    };
    // Opening migrates once, changes no row and keeps the parent and child bound.
    for _ in 0..2 {
        let j = Journal::open(state.path()).unwrap();
        assert_eq!(journal_format(&j), 5);
        assert_eq!(j.resource_load(parent_row.id).unwrap().unwrap(), parent_row);
        assert_eq!(j.resource_load(child_row.id).unwrap().unwrap(), child_row);
        j.workflow_validate_persisted(&parent_row).unwrap();
        assert_eq!(j.workflow_owner(child_row.id).unwrap(), Some(parent_row.id));
    }
    // The migrated batch is still fenced and released as one.
    let j = Journal::open(state.path()).unwrap();
    let mut next = parent(
        outputs.path(),
        vec![
            item(&snapshot_source(SNAPSHOT)),
            item(&checkpoint_source(true)),
        ],
    );
    assert!(matches!(
        j.resource_admit(&mut next.record),
        Err(Error::ConflictingPoolOperation)
    ));
    j.workflow_interrupt(pool, parent_row.id, false).unwrap();
    j.resource_control(pool, parent_row.id, OperationControl::AcknowledgeUnknown)
        .unwrap();
    assert_eq!(
        j.resource_load(child_row.id).unwrap().unwrap().state,
        OperationState::AcknowledgedUnknown
    );
    // A snapshot batch is a row the earlier build cannot parse. It refuses any journal
    // above format 4, so it never meets one.
    assert!(j.resource_admit(&mut next.record).unwrap());
    let stored = stored_workflow(&j, next.record.id);
    assert!(stored.get("ExportSnapshot").is_some());
    assert!(serde_json::from_value::<Format4Workflow>(stored).is_err());
    assert!(journal_format(&j) > 4);
    let reloaded = j.resource_load(next.record.id).unwrap().unwrap();
    assert_eq!(reloaded.action, next.record.action);
    j.workflow_validate_persisted(&reloaded).unwrap();
}
