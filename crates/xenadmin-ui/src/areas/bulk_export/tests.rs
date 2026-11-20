//! The tests of `bulk_export_tests.rs` and `bulk_export_snapshots_tests.rs`,
//! through the harness, under their Rune names (the first says "compact and
//! wide": the two window sizes are the renderer's now), and the tests of
//! what the port added: the commands and the steps of the wizard.

use std::path::PathBuf;

use serde_json::json;
use uuid::Uuid;
use xenadmin_core::{
    BatchVmWorkflow, Error, ExportSource, Inventory, OperationId, OvfPackageFormat, PoolId,
    PowerState, ResourceAction, ResourceOperationRecord, ReviewedCheckpoint, SnapshotId,
    TransferAction, VdiId, VmExportFormat, VmId, VmWorkflow, VmWorkflowBatch, XvaCompression,
};

use super::commands::{EXPORT_SNAPSHOTS, EXPORT_TEMPLATES, EXPORT_VMS};
use super::export::{self, Editor, Item, SnapshotTarget, Step};
use super::scenes::{
    self, DEMO_CHECKPOINT, DEMO_PARENT, DEMO_SNAPSHOT, demo_checkpoint, demo_editor, demo_outputs,
};
use crate::areas::events::{self, Family, OperationKey};
use crate::areas::snapshots;
use crate::intent::{FileMode, Intent};
use crate::testing::{Live, TempDir, poll_until};
use crate::{Harness, prelude::*};

const SURFACE: &str = "batch-export";
const TABLE: &str = "snapshots-table";

fn editor(h: &Harness) -> &Editor {
    h.model
        .areas
        .bulk_export
        .editor
        .as_ref()
        .expect("the export editor")
}

fn edit(h: &mut Harness) -> &mut Editor {
    h.model
        .areas
        .bulk_export
        .editor
        .as_mut()
        .expect("the export editor")
}

fn snapshot(e: &Editor, index: usize) -> &SnapshotTarget {
    match &e.targets[index] {
        Item::Snapshot(target) => target,
        Item::Vm(_) => panic!("a VM in a snapshot editor"),
    }
}

/// The reviewed memory of each item of the editor's review, None for a disk snapshot.
fn reviewed(e: &Editor) -> Vec<Option<ReviewedCheckpoint>> {
    e.review
        .as_ref()
        .expect("a review")
        .batch
        .items
        .iter()
        .map(|item| match &item.workflow {
            VmWorkflow::ExportSnapshot { checkpoint, .. } => checkpoint.clone(),
            _ => panic!("not a snapshot export"),
        })
        .collect()
}

fn amend(h: &mut Harness, change: impl FnOnce(&mut Inventory)) {
    scenes::amend(&mut h.model, change);
}

fn refusal(h: &Harness) -> Option<String> {
    export::proposal(&h.model, editor(h)).err()
}

fn pool(h: &Harness) -> PoolId {
    h.model.inventory.as_ref().expect("inventory").pool_id
}

fn id(value: u128) -> SnapshotId {
    SnapshotId::from(Uuid::from_u128(value))
}

fn parent() -> VmId {
    VmId::from(Uuid::from_u128(DEMO_PARENT))
}

fn ova() -> VmExportFormat {
    VmExportFormat::Ovf {
        format: OvfPackageFormat::Ova,
    }
}

/// How many visible strings start with `prefix`.
fn count(h: &Harness, prefix: &str) -> usize {
    h.texts()
        .iter()
        .filter(|text| text.starts_with(prefix))
        .count()
}

/// The submit button refuses, and the handler behind it submits nothing
/// and leaves the editor open.
fn assert_inert(h: &mut Harness) {
    assert!(h.try_click("batch-export-submit").is_err());
    export::submit(&mut h.model);
    assert!(h.model.resource_operations.is_empty());
    assert!(h.model.areas.bulk_export.editor.is_some());
}

fn resource_key(h: &Harness, id: OperationId) -> OperationKey {
    OperationKey {
        pool: pool(h),
        id,
        family: Family::Resource,
    }
}

// -- bulk_export_tests.rs -------------------------------------------------------------

/// Rune: `compact_and_wide_export_review_keeps_exact_targets_and_safe_footer`.
#[test]
fn compact_and_wide_export_review_keeps_exact_targets_and_safe_footer() {
    let mut h = Harness::scene("batch-export");
    let review = editor(&h).review.clone().unwrap();
    assert_eq!(review.batch.items.len(), 2);
    assert!(!review.batch.is_snapshot_export());
    let surface = h.surface();
    assert_eq!(surface.id, SURFACE);
    assert_eq!(surface.title, "Review selected VM exports");
    assert!(surface.experimental);
    assert!(h.model.experimental_notice());
    h.assert_unique_keys();
    for expected in [
        "2 captured targets",
        "snapshot ancestry",
        "Read-only connections",
        "Format: XVA",
        export::PREVIEW,
    ] {
        assert!(h.has_text(expected), "{expected}");
    }
    for key in [
        permissions::REVIEW_KEY,
        "batch-export-close",
        "batch-export-back",
        "batch-export-submit",
    ] {
        assert!(h.find(key).is_some(), "{key}");
    }
    assert_eq!(
        h.node("batch-export-submit").label.as_deref(),
        Some("Export reviewed VMs")
    );
    // Another selection behind the wizard changes nothing of what was reviewed.
    h.model.select_in_grid(Target::none());
    assert_eq!(
        h.try_click("batch-export-submit"),
        Err(export::PREVIEW.into())
    );
    export::submit(&mut h.model);
    assert!(h.model.resource_operations.is_empty());
    assert!(editor(&h).review.as_ref() == Some(&review));
    assert_eq!(editor(&h).error, Some(Error::Stale.to_string()));
    assert!(review.paths.iter().all(|p| !PathBuf::from(p).exists()));
}

/// Rune: `native_output_selection_is_bound_to_editor_identity_and_format`.
#[test]
fn native_output_selection_is_bound_to_editor_identity_and_format() {
    let dir = TempDir::new("bulk-export");
    let mut h = Harness::scene("batch-export");
    h.click("batch-export-back");
    assert!(editor(&h).review.is_none() && editor(&h).step == Step::Destinations);
    h.click("batch-export-choose-0");
    let first = editor(&h).targets[0].id();
    match h.model.intents.last() {
        Some(Intent::ChooseFile(request)) => {
            assert!(matches!(request.mode, FileMode::Save));
            assert_eq!(request.title, export::PROMPT);
            assert_eq!(request.suggested_name, Some(format!("{first}.xva")));
        }
        _ => panic!("no save prompt"),
    }
    // One destination at a time.
    assert_eq!(
        h.try_click("batch-export-choose-1"),
        Err(export::WAITING.into())
    );
    let missing = format!(
        "Choose the output for {} ({first})",
        editor(&h).targets[0].name()
    );
    assert_eq!(h.try_click("batch-export-review"), Err(missing));
    assert!(h.has_text(export::WAITING));
    let path = dir.path().join("selected.xva");
    h.answer_file(Some(path.clone()));
    poll_until(&mut h, "the destination was inspected", |h| {
        editor(h).pending.is_none()
    });
    assert_eq!(editor(&h).files[0].as_ref().unwrap().0, path);
    assert!(h.has_text(&format!("Output: {}", path.display())));
    assert_eq!(
        h.node("batch-export-choose-0").label.as_deref(),
        Some("Choose another destination")
    );
    assert!(!path.exists());
    // An answer that arrives after a format change belongs to an editor that is gone.
    h.click("batch-export-choose-1");
    export::change_format(&mut h.model, ova());
    h.answer_file(Some(dir.path().join("old.xva")));
    h.tick(0.1);
    assert!(editor(&h).files.iter().all(Option::is_none));
    assert!(editor(&h).pending.is_none() && editor(&h).result.is_none());
    assert!(!dir.path().join("old.xva").exists());
    // The new format names its own extension; a cancel and a prompt that
    // could not be shown both end the wait.
    h.click("batch-export-choose-1");
    let second = editor(&h).targets[1].id();
    assert!(matches!(
        h.model.intents.last(),
        Some(Intent::ChooseFile(request))
            if request.suggested_name == Some(format!("{second}.ova"))
    ));
    h.answer_file(None);
    assert!(editor(&h).pending.is_none() && editor(&h).error.is_none());
    h.click("batch-export-choose-1");
    h.fail_file("no portal");
    assert!(editor(&h).pending.is_none());
    assert_eq!(editor(&h).error.as_deref(), Some("no portal"));
    assert_eq!(
        h.node("batch-export-error").text.as_deref(),
        Some("no portal")
    );
}

/// Rune: `overlapping_destinations_and_changed_sources_block_entire_review`.
#[test]
fn overlapping_destinations_and_changed_sources_block_entire_review() {
    let mut h = Harness::scene("batch-export");
    let first = editor(&h).files[0].clone();
    edit(&mut h).files[1] = first;
    assert!(refusal(&h).unwrap().contains("overlap"));
    assert_eq!(
        h.node("batch-export-refusal").text.as_deref(),
        Some(export::OVERLAP)
    );
    let vm = VmId::from(editor(&h).targets[0].id());
    amend(&mut h, |i| {
        i.vms.get_mut(&vm).unwrap().power_state = PowerState::Running;
    });
    assert!(
        refusal(&h)
            .unwrap()
            .contains("identity or power state changed")
    );
    assert!(h.has_text("identity or power state changed"));
    assert_inert(&mut h);
    assert_eq!(editor(&h).targets.len(), 2);
    // Back on the destinations nothing can be reviewed either.
    h.click("batch-export-back");
    assert!(h.try_click("batch-export-review").is_err());
    export::review(&mut h.model);
    assert!(editor(&h).review.is_none());
    assert!(
        editor(&h)
            .error
            .as_deref()
            .is_some_and(|e| e.contains("identity or power state changed"))
    );
}

/// Rune: `batch_history_keeps_parent_reachable_and_children_have_no_independent_controls`.
/// The pool history is the History list of the Events tab, which shows the
/// children too (the `events` area's decision); they still have no controls
/// of their own.
#[test]
fn batch_history_keeps_parent_reachable_and_children_have_no_independent_controls() {
    let mut h = Harness::scene("batch-export");
    let selection = editor(&h).review.as_ref().unwrap().batch.clone();
    let pool = pool(&h);
    let parent = OperationId::new();
    let row = |id: OperationId,
               action: ResourceAction,
               batch_parent: Option<OperationId>,
               detail: &str|
     -> ResourceOperationRecord {
        serde_json::from_value(json!({
            "id":id,"pool":pool,"target":{"Pool":pool},"action":action,
            "batch_parent":batch_parent,"state":"OutcomeUnknown","progress":null,"detail":detail,
            "created_at_ms":0,"can_cancel":false,"cancel_requested":false,"steps":[],"created":null
        }))
        .unwrap()
    };
    h.model.resource_rows.push(row(
        parent,
        ResourceAction::VmWorkflowBatch(selection.clone()),
        None,
        "Captured parent export history",
    ));
    let mut children = Vec::new();
    for _ in 0..32 {
        let item = &selection.items[0];
        let child = OperationId::new();
        children.push(child);
        h.model.resource_rows.push(row(
            child,
            ResourceAction::Transfer(TransferAction::ExportXva {
                source: ExportSource::Vm {
                    vm: item.vm,
                    expected_template: false,
                },
                compression: XvaCompression::Uncompressed,
            }),
            Some(parent),
            "Owned transfer detail",
        ));
    }
    export::close(&mut h.model);
    let key = resource_key(&h, parent);
    events::show_operation(&mut h.model, key);
    assert!(h.surfaces().is_empty());
    assert!(h.has_text("Captured parent export history"));
    assert!(h.has_text("Export selected VMs"));
    let reason = format!(
        "Managed by batch {parent}. Use the batch controls on the Events tab of the Notifications view."
    );
    for child in children {
        let key = resource_key(&h, child);
        assert_eq!(
            events::recheck_eligibility(&h.model, key),
            Eligibility::disabled(reason.clone())
        );
        assert_eq!(
            events::acknowledge_eligibility(&h.model, key),
            Eligibility::disabled(reason.clone())
        );
        assert_eq!(
            events::cancel_eligibility(&h.model, key),
            Eligibility::Hidden
        );
        assert!(
            events::outcome_actions(&h.model, key)
                .iter()
                // "Explain outcome…" reads; it controls nothing.
                .filter(|button| !button.key.contains("failure-help"))
                .all(|button| button.disabled.is_some())
        );
    }
}

// -- bulk_export_snapshots_tests.rs ---------------------------------------------------

/// Rune: `several_snapshots_selected_in_the_view_open_one_review_and_single_actions_take_one`.
#[test]
fn several_snapshots_selected_in_the_view_open_one_review_and_single_actions_take_one() {
    let mut h = Harness::scene("snapshots");
    // Nothing is selected: there is nothing to export together.
    h.select_rows(TABLE, &[]);
    assert_eq!(
        h.try_click(EXPORT_SNAPSHOTS.0),
        Err(export::SNAPSHOT_LIMIT.into())
    );
    assert!(!super::open_snapshots(&mut h.model));
    assert_eq!(h.model.error.as_deref(), Some(export::SNAPSHOT_LIMIT));
    assert!(h.model.areas.bulk_export.editor.is_none());
    h.select_rows(TABLE, &[&id(DEMO_SNAPSHOT).to_string()]);
    assert_eq!(
        snapshots::selected_snapshot(&h.model).map(|s| s.id),
        Some(id(DEMO_SNAPSHOT))
    );
    // One snapshot is the single export (the Rune "Export…" of the page).
    assert_eq!(h.eligibility(EXPORT_SNAPSHOTS.0), Eligibility::Enabled);
    // The neighbouring row joins the selection.
    let rows = snapshots::edit::shown_rows(&h.model)
        .iter()
        .map(|s| s.id)
        .collect::<Vec<_>>();
    let at = rows
        .iter()
        .position(|row| *row == id(DEMO_SNAPSHOT))
        .unwrap();
    let expected = if at + 1 < rows.len() {
        rows[at..=at + 1].to_vec()
    } else {
        rows[at - 1..=at].to_vec()
    };
    let keys = expected.iter().map(ToString::to_string).collect::<Vec<_>>();
    h.select_rows(TABLE, &keys.iter().map(String::as_str).collect::<Vec<_>>());
    let picked = snapshots::selected_snapshots(&h.model)
        .iter()
        .map(|s| s.id)
        .collect::<Vec<_>>();
    assert_eq!(picked, expected);
    // With several selected, the actions that take one snapshot have no target.
    assert!(snapshots::selected_snapshot(&h.model).is_none());
    for command in ["snapshot.revert", "snapshot.properties", "snapshot.delete"] {
        assert!(
            matches!(h.eligibility(command), Eligibility::Disabled(_)),
            "{command}"
        );
    }
    h.click(EXPORT_SNAPSHOTS.0);
    let e = editor(&h);
    assert_eq!(
        e.targets.iter().map(Item::id).collect::<Vec<_>>(),
        picked.iter().map(|id| id.uuid()).collect::<Vec<_>>()
    );
    for index in 0..2 {
        let target = snapshot(e, index);
        assert_eq!(target.parent, parent());
        // The synthetic snapshots are disk snapshots; none has memory to choose.
        assert!(target.memory.is_none() && !target.preserve_memory);
    }
    assert!(e.review.is_none() && e.step == Step::Format);
    let surface = h.surface();
    assert_eq!(surface.id, SURFACE);
    assert_eq!(surface.title, "Export selected snapshots");
    assert!(surface.experimental && h.model.experimental_notice());
    let name = h.model.inventory.as_ref().unwrap().vms[&parent()]
        .name
        .clone();
    assert!(h.has_text(&format!("Snapshots of {name} · {}", parent())));
    assert!(h.has_text("2 captured snapshots"));
    // The editor keeps what it captured when the selection behind it changes.
    snapshots::edit::select(&mut h.model, &[id(DEMO_CHECKPOINT).to_string()]);
    assert_eq!(editor(&h).targets.len(), 2);
}

/// Rune: `review_names_each_snapshot_its_memory_choice_output_and_format_with_a_reachable_footer`.
#[test]
fn review_names_each_snapshot_its_memory_choice_output_and_format_with_a_reachable_footer() {
    let mut h = Harness::scene("snapshot-batch-export");
    let review = editor(&h).review.clone().unwrap();
    assert!(review.batch.is_snapshot_export());
    assert_eq!(review.batch.items.len(), 2);
    assert_eq!(review.outputs.len(), 2);
    let memory = snapshot(editor(&h), 1).memory.clone().unwrap();
    assert_eq!(
        reviewed(editor(&h)),
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
    assert_eq!(h.surface().title, "Review selected snapshot exports");
    h.assert_unique_keys();
    for expected in [
        "2 captured snapshots",
        "1 · Before upgrade",
        "2 · Known good",
        &format!("{} · Disk snapshot", id(DEMO_SNAPSHOT)),
        &format!("{} · Memory checkpoint", id(DEMO_CHECKPOINT)),
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
        assert!(h.has_text(expected), "{expected}");
    }
    for path in &review.paths {
        assert!(h.has_text(&format!("Output: {path}")), "{path}");
        assert!(!PathBuf::from(path).exists());
    }
    // A review shows the choice; it is changed on the step before it.
    assert!(h.find("batch-export-memory-1-include").is_none());
    assert!(h.find("batch-export-memory-1").is_some());
    assert!(h.find("batch-export-choose-0").is_none());
    assert!(h.surface().experimental && h.model.experimental_notice());
    for key in [
        permissions::REVIEW_KEY,
        "batch-export-close",
        "batch-export-back",
        "batch-export-submit",
    ] {
        assert!(h.find(key).is_some(), "{key}");
    }
    assert_eq!(
        h.node("batch-export-submit").label.as_deref(),
        Some("Export reviewed snapshots")
    );
    // Without a connection nothing is submitted and the review stays as it was.
    snapshots::edit::select(&mut h.model, &[]);
    assert_eq!(
        h.try_click("batch-export-submit"),
        Err(export::PREVIEW.into())
    );
    export::submit(&mut h.model);
    assert!(h.model.resource_operations.is_empty());
    assert!(editor(&h).review.as_ref() == Some(&review));
}

/// Rune: `including_memory_is_chosen_for_each_checkpoint_by_itself_and_ends_a_review`.
#[test]
fn including_memory_is_chosen_for_each_checkpoint_by_itself_and_ends_a_review() {
    let mut m = Model::fixture();
    m.load_demo(3, 48);
    let first = id(DEMO_SNAPSHOT);
    let second = id(DEMO_CHECKPOINT);
    let third = id(DEMO_SNAPSHOT + 2);
    demo_checkpoint(&mut m, first, VdiId::from(Uuid::from_u128(0x9901)));
    demo_checkpoint(&mut m, second, VdiId::from(Uuid::from_u128(0x9902)));
    demo_editor(&mut m, &[first, second, third]);
    demo_outputs(&mut m);
    let mut h = Harness::with(m);
    assert_eq!(editor(&h).targets.len(), 3);
    h.click("batch-export-next");
    // Captured from the view, no checkpoint has its memory chosen.
    assert!(
        (0..3).all(|index| !snapshot(editor(&h), index).preserve_memory),
        "memory is left out until chosen"
    );
    assert!(h.find("batch-export-memory-0-include").is_some());
    assert!(h.find("batch-export-memory-1-include").is_some());
    // A disk snapshot has no memory to choose.
    assert!(h.find("batch-export-memory-2-include").is_none());
    assert!(h.find("batch-export-memory-2").is_none());
    h.click("batch-export-review");
    assert_eq!(
        reviewed(editor(&h))
            .iter()
            .map(|c| c.as_ref().map(|c| c.preserve_memory))
            .collect::<Vec<_>>(),
        [Some(false), Some(false), None]
    );
    assert_eq!(count(&h, "Memory: left out."), 2);
    assert_eq!(count(&h, "Memory: included."), 0);
    // Choosing it for the second checkpoint changes that item alone.
    h.click("batch-export-back");
    h.click("batch-export-memory-1-include");
    assert!(editor(&h).review.is_none());
    assert!(!snapshot(editor(&h), 0).preserve_memory);
    assert!(snapshot(editor(&h), 1).preserve_memory);
    // Destinations stay: the choice does not change what file is written where.
    assert!(editor(&h).files.iter().all(Option::is_some));
    h.click("batch-export-review");
    let checkpoints = reviewed(editor(&h));
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
    assert_eq!(count(&h, "Memory: left out."), 1);
    assert_eq!(count(&h, "Memory: included."), 1);
    // Taking the choice back ends that review as well.
    let stale = editor(&h).review.clone();
    h.click("batch-export-back");
    h.click("batch-export-memory-1-omit");
    assert!(editor(&h).review.is_none());
    edit(&mut h).review = stale;
    export::submit(&mut h.model);
    assert!(h.model.resource_operations.is_empty());
    // Setting memory on a disk snapshot does nothing.
    export::set_memory(&mut h.model, 2, true);
    assert!(!snapshot(editor(&h), 2).preserve_memory);
}

/// Rune: `a_checkpoint_in_the_selection_leaves_only_the_xva_formats`.
#[test]
fn a_checkpoint_in_the_selection_leaves_only_the_xva_formats() {
    let mut h = Harness::scene("snapshot-batch-export");
    h.click("batch-export-back");
    h.click("batch-export-back");
    assert_eq!(editor(&h).step, Step::Format);
    h.assert_unique_keys();
    for index in 0..3 {
        assert!(h.find(&format!("batch-export-format-{index}")).is_some());
    }
    for index in 3..5 {
        assert!(h.find(&format!("batch-export-format-{index}")).is_none());
    }
    assert!(h.find("batch-export-xva-only").is_some());
    let before = editor(&h).format.clone();
    export::change_format(&mut h.model, ova());
    assert!(editor(&h).format == before);
    assert!(editor(&h).files.iter().all(Option::is_some));
    // The memory choice does not bring the package formats back.
    h.click("batch-export-next");
    h.click("batch-export-memory-1-omit");
    assert!(!snapshot(editor(&h), 1).preserve_memory);
    h.click("batch-export-back");
    assert!(h.find("batch-export-format-4").is_none());
    // A package format that reached the editor some other way is refused for the
    // checkpoint in the words of its single export.
    edit(&mut h).format = ova();
    let refused = refusal(&h).unwrap();
    assert!(
        refused.contains("Known good") && refused.contains("not supported"),
        "{refused}"
    );
    edit(&mut h).format = before;
    // Disk snapshots alone are offered every format.
    let e = edit(&mut h);
    e.targets.truncate(1);
    e.files.truncate(1);
    for index in 0..5 {
        assert!(h.find(&format!("batch-export-format-{index}")).is_some());
    }
    assert!(h.find("batch-export-xva-only").is_none());
    h.click("batch-export-format-4");
    assert!(editor(&h).format == ova());
    assert!(editor(&h).files.iter().all(Option::is_none));
    demo_outputs(&mut h.model);
    let review = export::proposal(&h.model, editor(&h))
        .unwrap_or_else(|reason| panic!("{reason}"));
    assert!(matches!(
        &review.batch.items[0].workflow,
        VmWorkflow::ExportSnapshot { checkpoint: None, format, .. } if *format == ova()
    ));
}

/// Rune: `a_changed_snapshot_memory_image_or_destination_blocks_the_whole_review`.
#[test]
fn a_changed_snapshot_memory_image_or_destination_blocks_the_whole_review() {
    let checkpoint = id(DEMO_CHECKPOINT);
    let plain = id(DEMO_SNAPSHOT);
    let mut h = Harness::scene("snapshot-batch-export");
    assert!(refusal(&h).is_none());
    // Two items may not write to one place.
    let saved = editor(&h).files[1].clone();
    edit(&mut h).files[1] = editor(&h).files[0].clone();
    assert!(refusal(&h).unwrap().contains("overlap"));
    edit(&mut h).files[1] = None;
    assert!(
        refusal(&h)
            .unwrap()
            .contains("Choose the output for Known good")
    );
    edit(&mut h).files[1] = saved;
    assert!(refusal(&h).is_none());
    // The reviewed memory image is another size now.
    let memory = snapshot(editor(&h), 1).memory.clone().unwrap();
    amend(&mut h, |i| {
        i.details.vdis.get_mut(&memory.vdi).unwrap().virtual_size =
            Some(memory.virtual_size + 4096);
    });
    assert!(refusal(&h).unwrap().contains("memory image changed"));
    amend(&mut h, |i| {
        i.details.vdis.get_mut(&memory.vdi).unwrap().virtual_size = Some(memory.virtual_size);
    });
    assert!(refusal(&h).is_none());
    // The host no longer offers the checkpoint for export.
    amend(&mut h, |i| {
        i.details.snapshots.get_mut(&checkpoint).unwrap().can_export = Some(false);
    });
    assert!(refusal(&h).unwrap().contains("Known good"));
    amend(&mut h, |i| {
        i.details.snapshots.get_mut(&checkpoint).unwrap().can_export = Some(true);
    });
    // The disk snapshot belongs to another VM now: nothing of the selection goes.
    amend(&mut h, |i| {
        let mut moved = i.snapshots.get(&plain).unwrap().clone();
        moved.vm = Some(VmId::from(Uuid::from_u128(DEMO_PARENT + 1)));
        i.snapshots.insert(moved);
    });
    assert!(refusal(&h).unwrap().contains(
        "Before upgrade (60000000-0000-0000-0000-000000000000): identity or state changed"
    ));
    assert!(h.has_text("identity or state changed"));
    assert_inert(&mut h);
    assert_eq!(editor(&h).targets.len(), 2);
}

/// Rune: `history_names_each_snapshot_of_a_batch_and_stops_after_the_current_snapshot`.
/// The wording lives in the `events` area now; this pins it for the batch
/// this area submits.
#[test]
fn history_names_each_snapshot_of_a_batch_and_stops_after_the_current_snapshot() {
    let mut h = Harness::scene("snapshot-batch-export");
    let batch = editor(&h).review.as_ref().unwrap().batch.clone();
    let pool = pool(&h);
    let parent = parent();
    let children = [OperationId::new(), OperationId::new()];
    let mut record: ResourceOperationRecord = serde_json::from_value(json!({
        "id":OperationId::new(),"pool":pool,"target":{"Pool":pool},
        "action":ResourceAction::VmWorkflowBatch(batch),
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
    let key = resource_key(&h, record.id);
    h.model.resource_rows.push(record.clone());
    export::close(&mut h.model);
    events::show_operation(&mut h.model, key);
    assert!(h.has_text("Export selected snapshots"));
    assert!(h.has_text(&format!(
        "Snapshot {} of VM {parent} · child {} · Succeeded",
        id(DEMO_SNAPSHOT),
        children[0]
    )));
    assert!(h.has_text(&format!(
        "Snapshot {} of VM {parent} · child {} · Not started",
        id(DEMO_CHECKPOINT),
        children[1]
    )));
    assert_eq!(
        events::cancel_label(&h.model, key),
        "Finish current snapshot and stop"
    );
    // A batch of VMs keeps its own words.
    record.action = ResourceAction::VmWorkflowBatch(VmWorkflowBatch {
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
    h.model.resource_rows = vec![record];
    assert!(h.has_text(&format!("VM {parent} · child {}", children[0])));
    assert!(!h.has_text(&format!(" of VM {parent}")));
    assert_eq!(
        events::cancel_label(&h.model, key),
        "Finish current VM and stop"
    );
}

// -- what the port added --------------------------------------------------------------

/// The Rune "Export selection…" of the Guests and Templates pages: one
/// "Export" in the menu whatever the selection, this area's for several
/// objects and the `transfers` area's for one.
#[test]
fn several_vms_or_templates_have_the_export_command_of_this_area() {
    let mut h = Harness::scene("batch-export");
    export::close(&mut h.model);
    let pool = pool(&h);
    let inventory = h.model.inventory.clone().unwrap();
    let two = h.model.scope().clone();
    assert!(two.is_multi() && two.all_of(ObjectKind::Vm));
    // The menu and the context menu of the two selected VMs.
    let item = h.menu_item(EXPORT_VMS.0).expect("VM > Export");
    assert_eq!(item.label, "Export");
    assert!(item.disabled.is_none());
    assert!(h.menu_item("vm.export").is_none());
    assert!(h.menu_item(EXPORT_TEMPLATES.0).is_none());
    assert!(h.context_menu(&two).iter().any(|i| i.id == EXPORT_VMS));
    // One VM: the single export.
    let one = Target::one(two.objects[0].clone());
    assert_eq!(h.model.eligibility(EXPORT_VMS, &one), Eligibility::Hidden);
    assert_ne!(
        h.model.eligibility(CommandId("vm.export"), &one),
        Eligibility::Hidden
    );
    // Another kind, and VMs with a template among them.
    let host = inventory.hosts.keys().next().copied().unwrap();
    let hosts = Target::many([ObjectRef::host(pool, host), ObjectRef::host(pool, host)]);
    assert_eq!(h.model.eligibility(EXPORT_VMS, &hosts), Eligibility::Hidden);
    let ordinary = inventory
        .vms
        .values()
        .filter(|v| !v.is_control_domain)
        .map(|v| v.id)
        .collect::<Vec<_>>();
    let mixed = Target::many([
        ObjectRef::vm(pool, ordinary[0]),
        ObjectRef::template(pool, ordinary[1]),
    ]);
    assert_eq!(h.model.eligibility(EXPORT_VMS, &mixed), Eligibility::Hidden);
    assert_eq!(
        h.model.eligibility(EXPORT_TEMPLATES, &mixed),
        Eligibility::Hidden
    );
    // More than 32, a control domain, two pools: the refusals of the selection.
    assert!(ordinary.len() >= 33, "the demo pool has 48 guests");
    let many = Target::many(ordinary.iter().take(33).map(|id| ObjectRef::vm(pool, *id)));
    assert_eq!(
        h.model.eligibility(EXPORT_VMS, &many),
        Eligibility::disabled(export::LIMIT)
    );
    assert!(!super::open(&mut h.model, &many));
    assert_eq!(h.model.error.as_deref(), Some(export::LIMIT));
    assert!(h.model.areas.bulk_export.editor.is_none());
    if let Some(control) = inventory.vms.values().find(|v| v.is_control_domain) {
        let with_control = Target::many([
            ObjectRef::vm(pool, ordinary[0]),
            ObjectRef::vm(pool, control.id),
        ]);
        assert_eq!(
            h.model.eligibility(EXPORT_VMS, &with_control),
            Eligibility::disabled(export::NOT_ORDINARY)
        );
    }
    let elsewhere = PoolId::from(Uuid::new_v4());
    let two_pools = Target::many([
        ObjectRef::vm(pool, ordinary[0]),
        ObjectRef::vm(elsewhere, ordinary[1]),
    ]);
    assert_eq!(
        h.model.eligibility(EXPORT_VMS, &two_pools),
        Eligibility::disabled(export::ONE_POOL)
    );
    // Templates: the same wizard under the label of the Templates menu.
    let templates = Target::many([
        ObjectRef::template(pool, ordinary[2]),
        ObjectRef::template(pool, ordinary[3]),
    ]);
    assert_eq!(
        h.model.eligibility(EXPORT_VMS, &templates),
        Eligibility::Hidden
    );
    assert_eq!(
        h.model.eligibility(EXPORT_TEMPLATES, &templates),
        Eligibility::Enabled
    );
    assert_eq!(
        h.model
            .eligibility(CommandId("templates.export"), &templates),
        Eligibility::Hidden
    );
    h.invoke_on(EXPORT_TEMPLATES.0, &templates);
    assert_eq!(h.surface().id, SURFACE);
    assert_eq!(
        editor(&h).targets.iter().map(Item::id).collect::<Vec<_>>(),
        [ordinary[2].uuid(), ordinary[3].uuid()]
    );
    h.dismiss();
    assert!(h.model.areas.bulk_export.editor.is_none());
    // The command opens the wizard on its first step, for exactly the target.
    h.invoke_on(EXPORT_VMS.0, &two);
    let surface = h.surface();
    assert_eq!(surface.id, SURFACE);
    assert_eq!(surface.title, "Export selected VMs");
    assert!(surface.experimental);
    assert!(editor(&h).step == Step::Format && editor(&h).review.is_none());
    assert_eq!(
        editor(&h).targets.iter().map(Item::id).collect::<Vec<_>>(),
        two.vms().iter().map(|id| id.uuid()).collect::<Vec<_>>()
    );
    assert!(h.has_text("2 captured targets"));
}

/// The steps of the wizard: the Rune form in two steps, and its review.
#[test]
fn the_wizard_walks_format_destinations_and_review() {
    let mut h = Harness::scene("batch-export");
    h.click("batch-export-back");
    h.click("batch-export-back");
    assert_eq!(editor(&h).step, Step::Format);
    h.assert_unique_keys();
    // The format step: every format, no destination, no Back.
    for index in 0..5 {
        assert!(h.find(&format!("batch-export-format-{index}")).is_some());
    }
    assert!(h.find("batch-export-xva-only").is_none());
    assert!(h.find("batch-export-back").is_none());
    assert!(h.find("batch-export-choose-0").is_none());
    // The destinations chosen for XVA survive a walk through the steps and
    // picking the format that is already chosen.
    h.click("batch-export-format-0");
    assert!(editor(&h).files.iter().all(Option::is_some));
    let identity = editor(&h).id;
    // Another format is another export: a new identity and no destination.
    h.click("batch-export-format-2");
    assert!(
        editor(&h).format
            == VmExportFormat::Xva {
                compression: XvaCompression::Zstd
            }
    );
    assert_ne!(editor(&h).id, identity);
    assert!(editor(&h).files.iter().all(Option::is_none));
    h.click("batch-export-next");
    assert_eq!(editor(&h).step, Step::Destinations);
    h.assert_unique_keys();
    assert!(h.find("batch-export-format-0").is_none());
    assert_eq!(
        h.node("batch-export-choose-0").label.as_deref(),
        Some("Choose destination")
    );
    let first = &editor(&h).targets[0];
    let missing = format!("Choose the output for {} ({})", first.name(), first.id());
    assert_eq!(h.try_click("batch-export-review"), Err(missing.clone()));
    assert_eq!(
        h.node("batch-export-refusal").text.as_deref(),
        Some(missing.as_str())
    );
    demo_outputs(&mut h.model);
    h.click("batch-export-review");
    assert!(editor(&h).review.is_some());
    assert!(h.has_text("Format: XVA zstd"));
    // Back from the review is the destinations; Escape closes.
    h.click("batch-export-back");
    assert!(editor(&h).review.is_none() && editor(&h).step == Step::Destinations);
    h.dismiss();
    assert!(h.model.areas.bulk_export.editor.is_none());
}

/// The editor belongs to the objects it was opened on: another scope closes it.
#[test]
fn another_scope_closes_the_wizard() {
    let mut h = Harness::scene("batch-export");
    let pool = pool(&h);
    assert!(h.model.areas.bulk_export.editor.is_some());
    h.select(Target::one(ObjectRef::pool(pool)));
    assert!(h.model.areas.bulk_export.editor.is_none());
    let mut h = Harness::scene("snapshot-batch-export");
    h.model.load_demo(1, 4);
    assert!(h.model.areas.bulk_export.editor.is_none());
}

/// Through a live connection the editor is current and nothing says
/// "preview"; what the fixture server cannot serve (the batch itself) is
/// not submitted here.
#[test]
fn a_connected_pool_is_current_and_not_a_preview() {
    let mut live = Live::with_vm();
    let vm = Target::one(live.vm());
    live.h.select(vm.clone());
    // One VM is the single export's: this area's command hides.
    assert_eq!(
        live.h.model.eligibility(EXPORT_VMS, &vm),
        Eligibility::Hidden
    );
    assert!(super::open(&mut live.h.model, &vm));
    let e = editor(&live.h);
    assert_eq!(e.pool, live.pool);
    assert!(e.observed.is_some());
    assert!(export::current(&live.h.model, e));
    assert!(live.h.find("batch-export-preview").is_none());
    assert!(live.h.find("batch-export-stale").is_none());
    // A review that was never made cannot be submitted.
    export::submit(&mut live.h.model);
    assert_eq!(editor(&live.h).error, Some(Error::Stale.to_string()));
    assert!(live.h.model.resource_operations.is_empty());
}
