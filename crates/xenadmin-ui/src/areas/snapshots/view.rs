//! The Snapshots tab and the open dialog, described. Nothing here changes
//! the model.
//!
//! Rune keys kept: `snapshots-table` (rows keyed by the snapshot UUID),
//! `snapshot-tree-toggle`, `snapshot-selection-count`,
//! `snapshot-name`, `snapshot-description`,
//! `snapshot-clone-name`, `snapshot-capture-option`, `restore-shutdown`,
//! `restore-start`, `snapshot-delete-disks`, `cancel-snapshot`,
//! `confirm-snapshot`, `snapshot-preview-load`, `snapshot-preview-image`.
//! The Rune buttons of the page are commands now and are found by their
//! command ids; `snapshot-batch-export-open` is the command
//! `snapshot.export` of `bulk_export`.
//!
//! New keys, for elements Rune left without one: `snapshots-help`,
//! `snapshots-status`, `snapshots-restore-note`, `snapshots-unavailable`,
//! `snapshot-capture-status` (the capture message, in the tab and in the
//! Create dialog), `snapshot-memory` (the checkbox "Include guest memory"),
//! `snapshot-review` (and so `snapshot-review-stale`), `snapshot-stale`,
//! `snapshot-problem-<n>`, `snapshot-demo`, `snapshot-error`,
//! `snapshot-information` and what [`super::inspection`] lists,
//! `snapshot-operation-<id>` with `snapshot-operation-cancel-<id>`,
//! `snapshot-operation-recheck-<id>` and `snapshot-show-created-<id>`.

use xenadmin_core::{
    Inventory, OperationRecord, OperationState, PoolId, PowerState, Snapshot, SnapshotAncestryRow,
    VmId,
};

use super::edit::{self, Dialog, ReviewedDisk, SnapshotReview};
use super::{capture, commands, inspection};
use crate::prelude::*;

pub const TABLE: &str = "snapshots-table";
pub const REVIEW: &str = "snapshot-review";

const HELP: &str = "Disk snapshots capture VM configuration and virtual disks. Choose Include guest memory for a memory checkpoint when supported. Application quiescing is not available on XCP-ng 8.1 and later. Shift/Ctrl-click selects several snapshots to export together.";
const SYNTHETIC: &str = "DEMO • Synthetic snapshots. Confirming host actions is disabled.";
const QUIESCE: &str = "Application quiescing is not available on XCP-ng 8.1 and later.";
const MEMORY_UNAVAILABLE: &str = "The host does not currently allow a memory checkpoint of this VM.";
const CAPTURE_UNAVAILABLE: &str = "A current running VM is required to capture a preview";

// -- The tab ----------------------------------------------------------------------

pub fn tab(m: &Model, tab: TabId, scope: &Target) -> Option<TabBody> {
    if tab != TabId::Snapshots {
        return None;
    }
    let object = scope.single_of(ObjectKind::Vm)?;
    let (pool, vm) = (object.pool?, object.vm_id()?);
    let unavailable =
        || TabBody::Blocks(blocks![caption("VM unavailable").key("snapshots-unavailable")]);
    let Some(inventory) = m.inventory_for(pool) else {
        return Some(unavailable());
    };
    let Some(guest) = inventory.vms.get(&vm) else {
        return Some(unavailable());
    };
    let a = &m.areas.snapshots;
    let rows = edit::rows(&inventory, vm, a.tree);
    let selected = edit::selected_ids(m);
    let chosen: Vec<&Snapshot> = rows
        .snapshots
        .iter()
        .filter(|snapshot| selected.contains(&snapshot.id))
        .collect();
    let list = table(
        TABLE,
        [
            column("name", "Name"),
            column("created", "Created (host time)"),
        ],
        rows.snapshots.iter().map(|snapshot| {
            row(
                snapshot.id,
                [
                    name_cell(snapshot, rows.ancestry.get(&snapshot.id), a.tree),
                    snapshot.created_at.clone().unwrap_or("Unknown".into()),
                ],
            )
        }),
    )
    .multi(chosen.iter().map(|snapshot| snapshot.id.to_string()), edit::select)
    .on_clear(|m| edit::select(m, &[]))
    .row_menu(commands::ROW_MENU)
    .empty("No snapshots for this VM.");
    let selection: Vec<Block> = match chosen.as_slice() {
        [] => Vec::new(),
        [one] => blocks![inspection::section(m, &inventory, pool, vm, one)],
        many => blocks![caption(commands::several(many.len())).key("snapshot-selection-count")],
    };
    Some(TabBody::Blocks(blocks![
        caption(HELP).key("snapshots-help"),
        a.capture
            .message
            .clone()
            .map(|message| text(message).key("snapshot-capture-status")),
        status(m, pool),
        (guest.power_state != PowerState::Halted)
            .then(|| caption(commands::RESTORE_STATE).key("snapshots-restore-note")),
        actions(blocks_of_buttons(a.tree)),
        list,
        selection,
        outcomes(m, &inventory, pool, vm),
    ]))
}

/// The view toggle. The export of the selection is `bulk_export`'s command
/// `snapshot.export`, in the tab's row of local actions.
fn blocks_of_buttons(tree: bool) -> Vec<Button> {
    vec![button(
        "snapshot-tree-toggle",
        if tree { "Show list" } else { "Show ancestry" },
        edit::toggle_tree,
    )]
}

/// The one line above the list that says why nothing can be changed.
fn status(m: &Model, pool: PoolId) -> Vec<Block> {
    if m.is_synthetic_pool(pool) {
        blocks![notice(NoticeKind::Preview, SYNTHETIC).key("snapshots-status")]
    } else if !edit::current(m, pool) {
        blocks![caption(edit::not_writable(m, pool)).key("snapshots-status")]
    } else {
        Vec::new()
    }
}

/// The name cell. In ancestry order: two spaces a level (twelve levels at
/// most), a corner under a parent, and what the ancestry says of the row.
fn name_cell(snapshot: &Snapshot, ancestry: Option<&SnapshotAncestryRow>, tree: bool) -> String {
    if !tree {
        return snapshot.name.clone();
    }
    format!(
        "{}{}{}{}",
        "  ".repeat(ancestry.map_or(0, |row| row.depth.min(12))),
        if ancestry.is_some_and(|row| row.depth > 0) {
            "└ "
        } else {
            ""
        },
        snapshot.name,
        if ancestry.is_some_and(|row| row.current_parent) {
            " · current base"
        } else if ancestry.is_some_and(|row| row.issue.is_some()) {
            " · ancestry incomplete"
        } else {
            ""
        }
    )
}

fn outcome_state(record: &OperationRecord) -> OutcomeState {
    match record.state {
        OperationState::Succeeded => OutcomeState::Succeeded,
        OperationState::PartiallyCompleted => OutcomeState::Partial,
        OperationState::Failed | OperationState::Rejected => OutcomeState::Failed,
        OperationState::Cancelled => OutcomeState::Cancelled,
        OperationState::OutcomeUnknown | OperationState::AcknowledgedUnknown => {
            OutcomeState::Unknown
        }
        _ => OutcomeState::Running(
            record
                .progress
                .map(|progress| progress.clamp(0.0, 1.0) as f32),
        ),
    }
}

/// The two newest operations of this VM, as the Rune page listed them under
/// the table. Stopping or rechecking a snapshot workflow is a command of
/// the tab's row; it acts on the newest shown workflow that offers it.
pub(crate) fn shown(m: &Model, pool: PoolId, vm: VmId) -> Vec<&OperationRecord> {
    m.operation_rows
        .iter()
        .rev()
        .filter(|record| record.vm == vm && record.pool == pool)
        .take(2)
        .collect()
}

fn outcomes(m: &Model, inventory: &Inventory, pool: PoolId, vm: VmId) -> Vec<Block> {
    shown(m, pool, vm)
        .into_iter()
        .map(|record| {
            let id = record.id;
            let mut lines = vec![record.detail.clone()];
            let mut controls = Vec::new();
            if let Some(id) = record.created_snapshot {
                lines.push(format!("Created snapshot UUID: {id}"));
            }
            if let Some(created) = record.created_vm {
                lines.push(format!("Created object UUID: {created}"));
                if let Some(object) = inventory.vms.get(&created) {
                    let is_template = object.is_template;
                    controls.push(button(
                        format!("snapshot-show-created-{id}"),
                        if is_template {
                            "Show created template"
                        } else {
                            "Show created VM"
                        },
                        move |m| {
                            m.select(Target::one(if is_template {
                                ObjectRef::template(pool, created)
                            } else {
                                ObjectRef::vm(pool, created)
                            }));
                        },
                    ));
                }
            }
            Block::from(
                outcome(
                    format!("snapshot-operation-{id}"),
                    outcome_state(record),
                    format!("{} • {:?}", record.action.label(), record.state),
                )
                .lines(lines)
                .actions(controls),
            )
        })
        .collect()
}

// -- The dialogs ------------------------------------------------------------------

pub fn surfaces(m: &Model) -> Vec<Surface> {
    let Some(dialog) = &m.areas.snapshots.dialog else {
        return Vec::new();
    };
    let pool = dialog.pool();
    let synthetic = m.is_synthetic_pool(pool);
    let (id, title, subtitle, confirm, danger) = match dialog {
        Dialog::Create { vm, id, .. } => (
            "snapshot-create",
            "Create snapshot",
            format!("{vm} ({id})"),
            "Create snapshot",
            false,
        ),
        Dialog::Restore {
            snapshot,
            snapshot_id,
            ..
        } => (
            "snapshot-restore",
            "Restore this snapshot?",
            format!("{snapshot} ({snapshot_id})"),
            "Restore snapshot",
            true,
        ),
        Dialog::Edit { source } => (
            "snapshot-edit",
            "Edit snapshot properties",
            subtitle(source),
            "Save properties",
            false,
        ),
        Dialog::Clone {
            source,
            as_template,
        } => (
            "snapshot-clone",
            if *as_template {
                "Create template from snapshot"
            } else {
                "Create VM from snapshot"
            },
            subtitle(source),
            if *as_template {
                "Create template"
            } else {
                "Create VM"
            },
            false,
        ),
        Dialog::Delete { source, .. } => (
            "snapshot-delete",
            "Delete this snapshot?",
            subtitle(source),
            "Delete snapshot",
            true,
        ),
    };
    let mut body = match dialog {
        Dialog::Create { .. } => create(m, dialog),
        Dialog::Restore { .. } => restore(m, dialog),
        Dialog::Edit { source } => edit_body(m, source),
        Dialog::Clone {
            source,
            as_template,
        } => clone_body(m, source, *as_template),
        Dialog::Delete { .. } => delete(m, dialog),
    };
    // What is wrong, in the Rune words; then the tail every variant shared.
    let problems = edit::problems(m, dialog)
        .into_iter()
        .filter(|problem| problem.shown)
        .enumerate()
        .map(|(index, problem)| {
            let key = format!("snapshot-problem-{index}");
            if problem.alert {
                alert(problem.text).key(key)
            } else {
                text(problem.text).tone(Tone::Warning).key(key)
            }
        })
        .collect::<Vec<_>>();
    // A review shows its own stale line; a form gets the notice.
    let reviewed = matches!(dialog, Dialog::Restore { .. } | Dialog::Delete { .. });
    body.extend(blocks![
        problems,
        edit::stale(m, dialog)
            .filter(|_| !reviewed)
            .map(|reason| notice(NoticeKind::Warning, reason).key("snapshot-stale")),
        synthetic.then(|| notice(
            NoticeKind::Preview,
            "Demo preview • no host changes can be submitted."
        )
        .key("snapshot-demo")),
        m.error.clone().map(|error| alert(error).key("snapshot-error")),
    ]);
    let confirm = button("confirm-snapshot", confirm, edit::submit).disabled_opt(edit::blocked(m));
    let surface = Surface::dialog(id, title)
        .subtitle(subtitle)
        .body(body)
        .footer([
            button("cancel-snapshot", "Cancel", capture::cancel_dialog),
            if danger {
                confirm.danger()
            } else {
                confirm.primary()
            },
        ])
        .on_close(capture::cancel_dialog);
    let permission = edit::review_requirements(m)
        .map(|requirements| permissions::assess_for(m, pool, &requirements));
    vec![match &permission {
        Some(permission) => surface.footer_note(permissions::footer_note(permission)),
        None => surface,
    }]
}

fn subtitle(source: &SnapshotReview) -> String {
    format!("{} ({})", source.snapshot, source.snapshot_id)
}

/// The Rune `SnapshotReview::summary`: whose snapshot, and which.
fn summary(vm: &str, id: VmId, snapshot: &str, snapshot_id: impl ToString) -> Vec<Property> {
    vec![
        property("VM", vm),
        property("VM UUID", id.to_string()).copyable(),
        property("Snapshot", snapshot),
        property("Snapshot UUID", snapshot_id.to_string()).copyable(),
    ]
}

fn source_summary(source: &SnapshotReview) -> Vec<Property> {
    summary(
        &source.vm,
        source.id,
        &source.snapshot,
        source.snapshot_id,
    )
}

fn create(m: &Model, dialog: &Dialog) -> Vec<Block> {
    let Dialog::Create { pool, vm, id, .. } = dialog else {
        return Vec::new();
    };
    let a = &m.areas.snapshots;
    let synthetic = m.is_synthetic_pool(*pool);
    let memory_available = m
        .inventory_for(*pool)
        .is_some_and(|inventory| edit::memory_available(&inventory, *id));
    blocks![
        properties([
            property("VM", vm.clone()),
            property("UUID", id.to_string()).copyable(),
        ]),
        checkbox(
            "snapshot-memory",
            "Include guest memory",
            a.memory,
            edit::toggle_memory
        )
        .disabled_if(!synthetic && !memory_available, MEMORY_UNAVAILABLE),
        caption(if a.memory {
            "Captures disks and guest memory. The VM may pause while memory is saved."
        } else {
            "Guest memory is not included. For application-consistent data, quiesce applications or shut down the guest first."
        }),
        input("snapshot-name", "Snapshot name", &a.name, edit::set_name)
            .placeholder("Snapshot name"),
        caption(QUIESCE),
        checkbox(
            "snapshot-capture-option",
            "Save a console preview with the snapshot",
            a.capture.opt_in,
            edit::toggle_capture
        )
        .disabled_if(a.capture.busy(), edit::CAPTURE_PENDING)
        .disabled_if(
            !synthetic && !capture::available(m, *pool, *id),
            CAPTURE_UNAVAILABLE
        ),
        caption("Optional: captures the console before creation and stores the image on the pool. It may contain sensitive guest information. Snapshot and preview outcomes are reported separately."),
        a.capture
            .message
            .clone()
            .filter(|_| a.capture.opt_in)
            .map(|message| text(message).key("snapshot-capture-status")),
        input(
            "snapshot-description",
            "Snapshot description",
            &a.description,
            edit::set_description
        )
        .placeholder("Optional description"),
    ]
}

fn restore(m: &Model, dialog: &Dialog) -> Vec<Block> {
    let Dialog::Restore {
        vm,
        id,
        snapshot,
        snapshot_id,
        expected_state,
        checkpoint,
        shutdown,
        start_after,
        ..
    } = dialog
    else {
        return Vec::new();
    };
    blocks![
        review(
            REVIEW,
            ["This replaces the VM's configuration and disk contents. Changes made since the snapshot will be lost."]
        )
        .affected(summary(vm, *id, snapshot, snapshot_id))
        .stale(edit::stale(m, dialog)),
        (*expected_state == PowerState::Running).then(|| checkbox(
            "restore-shutdown",
            "Cleanly shut down this running VM before restore",
            *shutdown,
            edit::toggle_shutdown
        )),
        checkbox(
            "restore-start",
            if *checkpoint {
                "Resume restored memory after restore"
            } else {
                "Start VM after restore"
            },
            *start_after,
            edit::toggle_start
        ),
        caption(if *checkpoint {
            "Restores the saved memory as a separate VM memory image. Without Resume, the VM remains suspended."
        } else {
            "Without Start, the restored VM remains halted."
        }),
        caption("Shutdown, restore and startup are tracked separately. A failure stops later steps; interrupted writes are never automatically repeated. Stop remaining steps does not interrupt a running host task."),
    ]
}

fn edit_body(m: &Model, source: &SnapshotReview) -> Vec<Block> {
    let a = &m.areas.snapshots;
    blocks![
        properties(source_summary(source)),
        input("snapshot-name", "Snapshot name", &a.name, edit::set_name),
        input(
            "snapshot-description",
            "Snapshot description",
            &a.description,
            edit::set_description
        ),
        caption("Name and description are saved in separate verified steps. Concurrent metadata changes stop the update."),
    ]
}

fn clone_body(m: &Model, source: &SnapshotReview, as_template: bool) -> Vec<Block> {
    let label = if as_template {
        "Template name"
    } else {
        "VM name"
    };
    blocks![
        properties(source_summary(source)),
        caption(if as_template {
            "Creates a separate template from this snapshot's configuration and disks in the existing storage. The source VM and snapshot are retained."
        } else {
            "Creates a separate VM from this snapshot's configuration and disks in the existing storage. The source VM and snapshot are retained. The new VM will not start automatically."
        }),
        input(
            "snapshot-clone-name",
            label,
            &m.areas.snapshots.name,
            edit::set_name
        )
        .placeholder(label),
    ]
}

fn disk_lines(disks: &[ReviewedDisk]) -> Block {
    properties(disks.iter().map(|disk| {
        property(
            format!("{} • {}", disk.name, storage_bytes(disk.size)),
            disk.id.to_string(),
        )
        .copyable()
    }))
}

fn delete(m: &Model, dialog: &Dialog) -> Vec<Block> {
    let Dialog::Delete {
        source,
        disks,
        retained,
        disk_inventory_available,
        delete_disks,
        memory_checkpoint,
    } = dialog
    else {
        return Vec::new();
    };
    let mut lines = vec![
        "This permanently removes this recovery point. The source VM is not deleted or shut down. Deleting a snapshot cannot be undone.",
    ];
    if *memory_checkpoint {
        lines.push("The saved memory checkpoint is stopped before its snapshot record is deleted. Its saved memory state is discarded.");
    }
    let cleanup: Vec<Block> = if !disk_inventory_available {
        blocks![text("Disk inventory is unavailable or incomplete. Optional data-disk cleanup is disabled. Saved checkpoint memory, if present, is still discarded.").tone(Tone::Warning)]
    } else if disks.is_empty() {
        blocks![caption(
            "No exclusively owned snapshot disks are eligible for cleanup. Associated virtual disks will be retained."
        )]
    } else {
        blocks![
            checkbox(
                "snapshot-delete-disks",
                format!(
                    "Also delete {} exclusively owned snapshot disk(s)",
                    disks.len()
                ),
                *delete_disks,
                edit::toggle_delete_disks
            ),
            caption(if *delete_disks {
                "These reviewed disks will be deleted after the snapshot record. Ownership is checked again before deletion."
            } else {
                "These disks will be retained unless disk cleanup is selected:"
            }),
            disk_lines(disks),
        ]
    };
    blocks![
        review(REVIEW, lines)
            .affected(source_summary(source))
            .stale(edit::stale(m, dialog)),
        cleanup,
        (!retained.is_empty()).then(|| blocks![
            heading("Disks retained"),
            caption("Shared disks, CD/DVD media, disks without snapshot ownership, and disks with incomplete ownership data are retained."),
            disk_lines(retained),
        ]),
        caption("Deletion runs in tracked steps. If a later step fails, the Events tab of the Notifications view identifies completed work and remaining disks. An interrupted response may leave the outcome unknown; recheck it before retrying."),
    ]
}
