use super::*;
use xenadmin_core::{OperationId, ReviewedCheckpoint};

fn settle(r: &mut rune::Runtime<State>) {
    for _ in 0..30 {
        r.frame(1.0 / 60.0);
    }
}
fn runtime_of(mut s: State, size: (f32, f32)) -> rune::Runtime<State> {
    s.window_width = size.0;
    s.window_height = size.1;
    let mut r = rune::Runtime::new(
        s,
        view,
        size,
        rune::Env {
            system_dark: true,
            scale: 1.0,
        },
    );
    settle(&mut r);
    r
}
/// The synthetic review: a disk snapshot and a checkpoint whose memory was chosen.
fn runtime(size: (f32, f32)) -> rune::Runtime<State> {
    let mut s = init();
    scene(&mut s);
    runtime_of(s, size)
}
fn click(r: &mut rune::Runtime<State>, key: &str) {
    r.update(|_, tx| tx.scroll_into_view(key));
    settle(r);
    let (x, y) = r.find(key).unwrap_or_else(|| panic!("missing {key}"));
    r.pointer_down(x, y);
    r.pointer_up(x, y);
    settle(r);
}
fn editor(r: &rune::Runtime<State>) -> &Editor {
    r.state.bulk_export.as_ref().unwrap()
}
fn snapshot(e: &Editor, index: usize) -> &SnapshotTarget {
    match &e.targets[index] {
        Target::Snapshot(target) => target,
        Target::Vm(_) => panic!("a VM in a snapshot editor"),
    }
}
/// The reviewed memory of each item of the editor's review, None for a disk snapshot.
fn reviewed(e: &Editor) -> Vec<Option<ReviewedCheckpoint>> {
    e.review
        .as_ref()
        .unwrap()
        .batch
        .items
        .iter()
        .map(|item| match &item.workflow {
            VmWorkflow::ExportSnapshot { checkpoint, .. } => checkpoint.clone(),
            _ => panic!("not a snapshot export"),
        })
        .collect()
}
fn inventory(r: &mut rune::Runtime<State>) -> &mut Inventory {
    Arc::make_mut(r.state.inventory.as_mut().unwrap())
}

#[test]
fn several_snapshots_selected_in_the_view_open_one_review_and_single_actions_take_one() {
    for size in [(1440.0, 900.0), (513.0, 652.0)] {
        let mut s = init();
        demo(&mut s);
        let parent = VmId::parse(DEMO_PARENT).unwrap();
        s.table.select(parent.to_string());
        vm_snapshots::open(&mut s);
        let mut r = runtime_of(s, size);
        // The single export's button is on the page only while one snapshot is selected.
        let single_exports = |r: &rune::Runtime<State>| {
            r.elements()
                .into_iter()
                .filter(|e| e.key.as_deref() == Some("export-xva"))
                .count()
        };
        let without_selection = single_exports(&r);
        // Nothing is selected: there is nothing to export together.
        click(&mut r, "snapshot-batch-export-open");
        assert!(r.state.bulk_export.is_none(), "{size:?}");
        click(&mut r, DEMO_SNAPSHOT);
        assert_eq!(
            vm_snapshots::selected_snapshot(&r.state).map(|s| s.id.to_string()),
            Some(DEMO_SNAPSHOT.to_owned())
        );
        assert_eq!(single_exports(&r), without_selection + 1);
        // Shift with an arrow extends the selection to the neighbouring row.
        let rows = r
            .state
            .snapshot_rows
            .iter()
            .map(|s| s.id)
            .collect::<Vec<_>>();
        let at = rows
            .iter()
            .position(|id| id.to_string() == DEMO_SNAPSHOT)
            .unwrap();
        let (key, expected) = if at + 1 < rows.len() {
            (rune::Key::Down, rows[at..=at + 1].to_vec())
        } else {
            (rune::Key::Up, rows[at - 1..=at].to_vec())
        };
        r.key(rune::KeyEvent {
            key,
            mods: rune::Mods {
                shift: true,
                ..Default::default()
            },
        });
        settle(&mut r);
        let picked = vm_snapshots::selected_snapshots(&r.state)
            .map(|s| s.id)
            .collect::<Vec<_>>();
        assert_eq!(picked, expected, "{size:?}");
        // With several selected, the actions that take one snapshot have no target.
        assert!(vm_snapshots::selected_snapshot(&r.state).is_none());
        assert_eq!(single_exports(&r), without_selection, "{size:?}");
        assert!(r.find("snapshot-selection-count").is_some());
        for key in ["restore-snapshot", "edit-snapshot", "delete-snapshot"] {
            click(&mut r, key);
            assert!(r.state.snapshot_dialog.is_none(), "{key} at {size:?}");
        }
        click(&mut r, "inspect-snapshot");
        assert!(r.state.snapshot_inspection.is_none());
        r.update(|s, _| vm_snapshots::refresh(s));
        assert_eq!(vm_snapshots::selected_snapshots(&r.state).count(), 2);
        click(&mut r, "snapshot-batch-export-open");
        let e = editor(&r);
        assert_eq!(
            e.targets.iter().map(Target::id).collect::<Vec<_>>(),
            picked.iter().map(|id| id.uuid()).collect::<Vec<_>>()
        );
        for index in 0..2 {
            let target = snapshot(e, index);
            assert_eq!(target.parent, parent);
            // The synthetic snapshots are disk snapshots; none has memory to choose.
            assert!(target.memory.is_none() && !target.preserve_memory);
        }
        assert!(e.review.is_none());
        assert!(r.find("experimental-notice").is_some());
        let text = r.accessibility_json().to_string();
        assert!(text.contains("Export selected snapshots"));
        let name = &r.state.inventory.as_ref().unwrap().vms[&parent].name;
        assert!(text.contains(&format!("Snapshots of {name} · {parent}")));
        assert!(text.contains("2 captured snapshots"));
        // The editor keeps what it captured when the selection behind it changes.
        r.update(|s, _| s.snapshot_table.select(DEMO_CHECKPOINT));
        assert_eq!(editor(&r).targets.len(), 2);
    }
}

#[test]
fn review_names_each_snapshot_its_memory_choice_output_and_format_with_a_reachable_footer() {
    for size in [(513.0, 652.0), (1440.0, 900.0)] {
        let mut r = runtime(size);
        let review = editor(&r).review.clone().unwrap();
        assert!(review.batch.is_snapshot_export());
        assert_eq!(review.batch.items.len(), 2);
        assert_eq!(review.outputs.len(), 2);
        let memory = snapshot(editor(&r), 1).memory.clone().unwrap();
        assert_eq!(
            reviewed(editor(&r)),
            [
                None,
                Some(ReviewedCheckpoint {
                    memory: memory.clone(),
                    preserve_memory: true,
                })
            ]
        );
        assert_eq!(
            review
                .batch
                .items
                .iter()
                .map(|i| i.expected_state.clone())
                .collect::<Vec<_>>(),
            [PowerState::Halted, PowerState::Suspended]
        );
        let text = r.accessibility_json().to_string();
        for expected in [
            "Review selected snapshot exports",
            "2 captured snapshots",
            "1 · Before upgrade",
            "2 · Known good",
            &format!("{DEMO_SNAPSHOT} · Disk snapshot"),
            &format!("{DEMO_CHECKPOINT} · Memory checkpoint"),
            &format!(
                "Checkpoint memory image: {} · {} bytes · storage {}",
                memory.vdi, memory.virtual_size, memory.sr
            ),
            "Memory: included. The archive holds guest RAM",
            "Format: XVA",
            "snapshot ancestry is not included",
            "Read-only connections may export",
            "Synthetic preview; transfers are disabled.",
        ] {
            assert!(text.contains(expected), "{expected} at {size:?}");
        }
        for path in &review.paths {
            assert!(text.contains(&format!("Output: {path}")), "{path}");
            assert!(!PathBuf::from(path).exists());
        }
        // A review shows the choice; it is changed on the page before it.
        assert!(r.find("batch-export-memory-1-include").is_none());
        assert!(r.find("batch-export-choose-0").is_none());
        assert!(r.find("experimental-notice").is_some());
        r.update(|_, tx| tx.scroll_into_view("review-permission-explanation"));
        settle(&mut r);
        let (_, y) = r.find("review-permission-explanation").unwrap();
        assert!((0.0..size.1).contains(&y));
        for key in [
            "batch-export-close",
            "batch-export-back",
            "batch-export-submit",
        ] {
            let (x, y) = r.find(key).unwrap();
            assert!(
                (0.0..size.0).contains(&x) && (0.0..size.1).contains(&y),
                "{key}"
            );
        }
        // Without a connection nothing is submitted and the review stays as it was.
        r.state.snapshot_table.select("different selection");
        click(&mut r, "batch-export-submit");
        assert!(r.state.resource_operations.is_empty());
        assert!(editor(&r).review.as_ref() == Some(&review));
    }
}

#[test]
fn including_memory_is_chosen_for_each_checkpoint_by_itself_and_ends_a_review() {
    let mut s = init();
    demo(&mut s);
    let first = SnapshotId::parse(DEMO_SNAPSHOT).unwrap();
    let second = SnapshotId::parse(DEMO_CHECKPOINT).unwrap();
    let third = SnapshotId::parse("60000000-0000-0000-0000-000000000002").unwrap();
    demo_checkpoint(&mut s, first, VdiId::from(uuid::Uuid::from_u128(0x9901)));
    demo_checkpoint(&mut s, second, VdiId::from(uuid::Uuid::from_u128(0x9902)));
    demo_editor(&mut s, &[first, second, third]);
    demo_outputs(&mut s);
    let mut r = runtime_of(s, (513.0, 652.0));
    // Captured from the view, no checkpoint has its memory chosen.
    assert!(
        (0..3).all(|index| !snapshot(editor(&r), index).preserve_memory),
        "memory is left out until chosen"
    );
    assert!(r.find("batch-export-memory-0-include").is_some());
    assert!(r.find("batch-export-memory-1-include").is_some());
    // A disk snapshot has no memory to choose.
    assert!(r.find("batch-export-memory-2-include").is_none());
    assert!(r.find("batch-export-memory-2").is_none());
    click(&mut r, "batch-export-review");
    assert_eq!(
        reviewed(editor(&r))
            .iter()
            .map(|c| c.as_ref().map(|c| c.preserve_memory))
            .collect::<Vec<_>>(),
        [Some(false), Some(false), None]
    );
    let text = r.accessibility_json().to_string();
    assert_eq!(text.matches("Memory: left out.").count(), 2);
    assert!(!text.contains("Memory: included."));
    // Choosing it for the second checkpoint changes that item alone.
    click(&mut r, "batch-export-back");
    click(&mut r, "batch-export-memory-1-include");
    assert!(editor(&r).review.is_none());
    assert!(!snapshot(editor(&r), 0).preserve_memory);
    assert!(snapshot(editor(&r), 1).preserve_memory);
    // Destinations stay: the choice does not change what file is written where.
    assert!(editor(&r).files.iter().all(Option::is_some));
    click(&mut r, "batch-export-review");
    let checkpoints = reviewed(editor(&r));
    assert_eq!(
        checkpoints
            .iter()
            .map(|c| c.as_ref().map(|c| c.preserve_memory))
            .collect::<Vec<_>>(),
        [Some(false), Some(true), None]
    );
    assert_ne!(
        checkpoints[0].as_ref().unwrap().memory.vdi,
        checkpoints[1].as_ref().unwrap().memory.vdi
    );
    let text = r.accessibility_json().to_string();
    assert_eq!(text.matches("Memory: left out.").count(), 1);
    assert_eq!(text.matches("Memory: included.").count(), 1);
    // Taking the choice back ends that review as well.
    let stale = editor(&r).review.clone();
    click(&mut r, "batch-export-back");
    click(&mut r, "batch-export-memory-1-omit");
    assert!(editor(&r).review.is_none());
    edit(&mut r.state).review = stale;
    submit(&mut r.state);
    assert!(r.state.resource_operations.is_empty());
    // Setting memory on a disk snapshot does nothing.
    set_memory(&mut r.state, 2, true);
    assert!(!snapshot(editor(&r), 2).preserve_memory);
}

#[test]
fn a_checkpoint_in_the_selection_leaves_only_the_xva_formats() {
    let mut r = runtime((513.0, 652.0));
    click(&mut r, "batch-export-back");
    for index in 0..3 {
        assert!(r.find(&format!("batch-export-format-{index}")).is_some());
    }
    for index in 3..5 {
        assert!(r.find(&format!("batch-export-format-{index}")).is_none());
    }
    assert!(r.find("batch-export-xva-only").is_some());
    let ova = VmExportFormat::Ovf {
        format: OvfPackageFormat::Ova,
    };
    let before = editor(&r).format.clone();
    change_format(&mut r.state, ova.clone());
    assert!(editor(&r).format == before);
    assert!(editor(&r).files.iter().all(Option::is_some));
    // The memory choice does not bring the package formats back.
    click(&mut r, "batch-export-memory-1-omit");
    assert!(r.find("batch-export-format-4").is_none());
    // A package format that reached the editor some other way is refused for the
    // checkpoint in the words of its single export.
    edit(&mut r.state).format = ova.clone();
    let refusal = proposal(&r.state, editor(&r)).err().unwrap();
    assert!(
        refusal.contains("Known good") && refusal.contains("not supported"),
        "{refusal}"
    );
    edit(&mut r.state).format = before;
    // Disk snapshots alone are offered every format.
    r.update(|s, _| {
        let e = edit(s);
        e.targets.truncate(1);
        e.files.truncate(1);
    });
    settle(&mut r);
    for index in 0..5 {
        assert!(r.find(&format!("batch-export-format-{index}")).is_some());
    }
    assert!(r.find("batch-export-xva-only").is_none());
    click(&mut r, "batch-export-format-4");
    assert!(editor(&r).format == ova);
    assert!(editor(&r).files.iter().all(Option::is_none));
    demo_outputs(&mut r.state);
    let review = proposal(&r.state, editor(&r)).unwrap();
    assert!(matches!(
        &review.batch.items[0].workflow,
        VmWorkflow::ExportSnapshot { checkpoint: None, format, .. } if *format == ova
    ));
}

#[test]
fn a_changed_snapshot_memory_image_or_destination_blocks_the_whole_review() {
    let checkpoint = SnapshotId::parse(DEMO_CHECKPOINT).unwrap();
    let plain = SnapshotId::parse(DEMO_SNAPSHOT).unwrap();
    let refusal = |r: &rune::Runtime<State>| proposal(&r.state, editor(r)).err();
    let mut r = runtime((513.0, 652.0));
    assert!(refusal(&r).is_none());
    // Two items may not write to one place.
    let saved = editor(&r).files[1].clone();
    edit(&mut r.state).files[1] = editor(&r).files[0].clone();
    assert!(refusal(&r).unwrap().contains("overlap"));
    edit(&mut r.state).files[1] = None;
    assert!(
        refusal(&r)
            .unwrap()
            .contains("Choose the output for Known good")
    );
    edit(&mut r.state).files[1] = saved;
    assert!(refusal(&r).is_none());
    // The reviewed memory image is another size now.
    let memory = snapshot(editor(&r), 1).memory.clone().unwrap();
    inventory(&mut r)
        .details
        .vdis
        .get_mut(&memory.vdi)
        .unwrap()
        .virtual_size = Some(memory.virtual_size + 4096);
    assert!(refusal(&r).unwrap().contains("memory image changed"));
    inventory(&mut r)
        .details
        .vdis
        .get_mut(&memory.vdi)
        .unwrap()
        .virtual_size = Some(memory.virtual_size);
    assert!(refusal(&r).is_none());
    // The host no longer offers the checkpoint for export.
    inventory(&mut r)
        .details
        .snapshots
        .get_mut(&checkpoint)
        .unwrap()
        .can_export = Some(false);
    assert!(refusal(&r).unwrap().contains("Known good"));
    inventory(&mut r)
        .details
        .snapshots
        .get_mut(&checkpoint)
        .unwrap()
        .can_export = Some(true);
    // The disk snapshot belongs to another VM now: nothing of the selection goes.
    let mut moved = inventory(&mut r).snapshots.get(&plain).unwrap().clone();
    moved.vm = Some(VmId::parse("30000000-0000-0000-0000-000000000001").unwrap());
    inventory(&mut r).snapshots.insert(moved);
    assert!(refusal(&r).unwrap().contains(
        "Before upgrade (60000000-0000-0000-0000-000000000000): identity or state changed"
    ));
    settle(&mut r);
    click(&mut r, "batch-export-submit");
    submit(&mut r.state);
    assert!(r.state.resource_operations.is_empty());
    assert_eq!(editor(&r).targets.len(), 2);
}

#[test]
fn history_names_each_snapshot_of_a_batch_and_stops_after_the_current_snapshot() {
    let mut r = runtime((1440.0, 900.0));
    let batch = editor(&r).review.as_ref().unwrap().batch.clone();
    let pool = r.state.inventory.as_ref().unwrap().pool_id;
    let parent = VmId::parse(DEMO_PARENT).unwrap();
    let children = [OperationId::new(), OperationId::new()];
    let mut record: xenadmin_core::ResourceOperationRecord =
        serde_json::from_value(serde_json::json!({
            "id":OperationId::new(),"pool":pool,"target":{"Pool":pool},
            "action":xenadmin_core::ResourceAction::VmWorkflowBatch(batch),
            "batch_parent":null,"state":"Tracking","progress":null,
            "detail":"Captured snapshot batch","created_at_ms":0,"can_cancel":true,
            "cancel_requested":false,"steps":[],"created":null,
            "workflow_batch":[
                {"vm":parent,"child":children[0],"state":"Succeeded","detail":"Export verified"},
                {"vm":parent,"child":children[1],"state":null,"detail":"Not started"}
            ]
        }))
        .unwrap();
    assert_eq!(record.action.label(), "Export selected snapshots");
    r.state.resource_rows.push(record.clone());
    r.update(|s, _| {
        s.bulk_export = None;
        resource_admin::open_pool(s, resource_admin::Kind::History);
    });
    settle(&mut r);
    let text = r.accessibility_json().to_string();
    assert!(text.contains("Export selected snapshots"));
    assert!(text.contains(&format!(
        "Snapshot {DEMO_SNAPSHOT} of VM {parent} · child {} · Succeeded",
        children[0]
    )));
    assert!(text.contains(&format!(
        "Snapshot {DEMO_CHECKPOINT} of VM {parent} · child {} · Not started",
        children[1]
    )));
    assert!(text.contains("Finish current snapshot and stop"));
    assert!(!text.contains("Finish current VM and stop"));
    // A batch of VMs keeps its own words.
    record.action = xenadmin_core::ResourceAction::VmWorkflowBatch(VmWorkflowBatch {
        items: vec![BatchVmWorkflow {
            vm: parent,
            expected_state: PowerState::Halted,
            workflow: VmWorkflow::Export {
                expected_template: false,
                format: VmExportFormat::Xva {
                    compression: XvaCompression::Uncompressed,
                },
            },
        }],
    });
    record.workflow_batch.truncate(1);
    r.update(|s, _| s.resource_rows = vec![record]);
    settle(&mut r);
    let text = r.accessibility_json().to_string();
    assert!(text.contains(&format!("VM {parent} · child {}", children[0])));
    assert!(!text.contains(&format!(" of VM {parent}")));
    assert!(text.contains("Finish current VM and stop"));
}
