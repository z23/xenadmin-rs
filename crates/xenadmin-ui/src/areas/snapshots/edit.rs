//! `vm_snapshots.rs` without its view: the rows of the tab, the selection,
//! the five dialogs with what each captured when it opened, their
//! validation, their permission requirements and the submission.
//!
//! Carried over from the Rune module. What changed:
//!
//! - `s.snapshot_view` is the tree scope ([`super::shown_vm`]); every
//!   dialog keeps the pool of its VM and reads that pool's inventory,
//!   snapshot and connection (`inventory_for`, `snapshot_for`,
//!   `can_mutate_pool`), not the scope session's.
//! - The Rune confirm button was disabled by a condition and `submit`
//!   trusted it. Here [`blocked`] is that condition with its reason, and
//!   [`submit`] asks it again.
//! - Rune's `Target<T>` is [`Token`]: the prelude has a `Target`.

use std::collections::{BTreeMap, BTreeSet};

use xenadmin_core::{
    ClassAvailability, Error, Inventory, ObservedSnapshot, ObservedVm, PermissionRequirements,
    PoolId, PowerAction, PowerState, Snapshot, SnapshotAction, SnapshotAncestryRow, SnapshotId,
    SnapshotName, SnapshotOperation, SnapshotWorkflow, VdiId, VmCommand, VmId,
};

use super::{capture, inspection, shown_vm};
use crate::prelude::*;

/// The reason a synthetic pool gives a disabled confirm button (the text
/// every area uses).
pub const DEMO: &str = "Demo mode cannot change infrastructure.";
pub const READ_ONLY: &str = "Read-only connection: snapshot changes are disabled.";
pub const NOT_CURRENT: &str = "Inventory is stale. Snapshot actions require a current connection.";
pub const MODE_UNAVAILABLE: &str = "The host does not currently allow the selected snapshot mode.";
pub const RESTORE_INVALID: &str = "Review the current VM state and explicitly select a permitted clean shutdown before restoring a running VM.";
pub const CLONE_NAME: &str = "Enter a name of 1–255 characters without control characters.";
pub const CLONE_CHANGED: &str = "The snapshot changed or the host no longer permits cloning it. Close this review and select the snapshot again.";
pub const DELETE_CHANGED: &str = "The snapshot changed or the host no longer permits deleting it. Close this review and select the snapshot again.";
pub const OWNERSHIP_CHANGED: &str = "Disk ownership changed after this review. Close it and review the snapshot again before deleting disks.";
pub const CAPTURE_PENDING: &str = "A snapshot preview is already pending";

/// What a dialog captured of its target: the observation of a live pool,
/// or nothing in a synthetic one, where nothing can be submitted.
#[derive(Clone, Copy)]
pub enum Token<T> {
    Live(T),
    Demo,
}

pub enum Dialog {
    Create {
        pool: PoolId,
        target: Token<ObservedVm>,
        vm: String,
        id: VmId,
    },
    Restore {
        pool: PoolId,
        target: Token<ObservedSnapshot>,
        vm: String,
        id: VmId,
        snapshot: String,
        snapshot_id: SnapshotId,
        vm_target: Token<ObservedVm>,
        expected_state: PowerState,
        checkpoint: bool,
        shutdown: bool,
        start_after: bool,
    },
    Edit {
        source: SnapshotReview,
    },
    Clone {
        source: SnapshotReview,
        as_template: bool,
    },
    Delete {
        source: SnapshotReview,
        disks: Vec<ReviewedDisk>,
        retained: Vec<ReviewedDisk>,
        disk_inventory_available: bool,
        delete_disks: bool,
        memory_checkpoint: bool,
    },
}

impl Dialog {
    /// The pool of the VM the dialog was opened for.
    pub fn pool(&self) -> PoolId {
        match self {
            Self::Create { pool, .. } | Self::Restore { pool, .. } => *pool,
            Self::Edit { source } | Self::Clone { source, .. } | Self::Delete { source, .. } => {
                source.pool
            }
        }
    }
}

pub struct SnapshotReview {
    pub target: Token<ObservedSnapshot>,
    pub pool: PoolId,
    pub vm: String,
    pub id: VmId,
    pub snapshot: String,
    pub snapshot_id: SnapshotId,
    pub power_state: PowerState,
    pub description: String,
    pub vm_target: Token<ObservedVm>,
}

pub struct ReviewedDisk {
    pub id: VdiId,
    pub name: String,
    pub size: Option<u64>,
}

impl SnapshotReview {
    pub fn capture(m: &Model) -> Result<Self, Error> {
        let (pool, id) = shown_vm(m).ok_or(Error::Stale)?;
        let inventory = m.inventory_for(pool).ok_or(Error::Stale)?;
        let vm = inventory.vms.get(&id).ok_or(Error::Stale)?;
        let snapshot = selected_snapshot(m).ok_or(Error::Stale)?;
        if snapshot.vm != Some(id) {
            return Err(Error::Stale);
        }
        let demo = m.is_synthetic_pool(pool);
        let target = if demo {
            Token::Demo
        } else {
            Token::Live(
                m.snapshot_for(pool)
                    .ok_or(Error::Stale)?
                    .observe_snapshot(id, snapshot.id)?,
            )
        };
        Ok(Self {
            target,
            pool,
            vm: vm.name.clone(),
            id,
            snapshot: snapshot.name.clone(),
            snapshot_id: snapshot.id,
            power_state: snapshot.power_state.clone(),
            description: snapshot.description.clone(),
            vm_target: if demo {
                Token::Demo
            } else {
                Token::Live(m.snapshot_for(pool).ok_or(Error::Stale)?.observe_vm(id)?)
            },
        })
    }

    pub fn live(&self, m: &Model) -> bool {
        matches!(self.target, Token::Live(observed) if m
            .snapshot_for(self.pool)
            .and_then(|snapshot| snapshot.observe_snapshot(self.id, self.snapshot_id).ok())
            == Some(observed))
    }

    pub fn permits(&self, m: &Model, action: SnapshotAction) -> bool {
        m.inventory_for(self.pool)
            .as_ref()
            .filter(|inventory| inventory.pool_id == self.pool)
            .and_then(|inventory| inventory.snapshots.get(&self.snapshot_id))
            .is_some_and(|snapshot| {
                snapshot.vm == Some(self.id)
                    && snapshot.power_state == self.power_state
                    && snapshot.allowed_actions.contains(&action)
            })
    }
}

// -- Rows and selection ---------------------------------------------------------

/// The rows of the tab: the snapshots of a VM, newest first, or in ancestry
/// order with the ancestry row of each (the Rune `refresh`).
pub struct Rows {
    pub snapshots: Vec<Snapshot>,
    pub ancestry: BTreeMap<SnapshotId, SnapshotAncestryRow>,
}

pub fn rows(inventory: &Inventory, vm: VmId, tree: bool) -> Rows {
    let mut snapshots: Vec<Snapshot> = inventory.snapshots.for_vm(vm).cloned().collect();
    snapshots.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(a.id.cmp(&b.id)));
    let ancestry = inventory.snapshot_ancestry(vm);
    if tree {
        let positions: BTreeMap<_, _> = ancestry
            .iter()
            .enumerate()
            .map(|(index, row)| (row.id, index))
            .collect();
        snapshots.sort_by_key(|row| positions.get(&row.id).copied().unwrap_or(usize::MAX));
    }
    Rows {
        snapshots,
        ancestry: ancestry.into_iter().map(|row| (row.id, row)).collect(),
    }
}

/// The rows of the shown VM, in the order the tab shows them.
pub fn shown_rows(m: &Model) -> Vec<Snapshot> {
    shown_vm(m)
        .and_then(|(pool, vm)| {
            m.inventory_for(pool)
                .map(|inventory| rows(&inventory, vm, m.areas.snapshots.tree).snapshots)
        })
        .unwrap_or_default()
}

/// The ids of the selected rows. A snapshot handed over as a grid selection
/// (a search result that was opened) is the selection until a row is
/// chosen in the tab.
pub fn selected_ids(m: &Model) -> Vec<SnapshotId> {
    let pool = shown_vm(m).map(|(pool, _)| pool);
    let handed: Vec<SnapshotId> = m
        .selection
        .grid
        .objects
        .iter()
        .filter(|object| object.kind == ObjectKind::Snapshot && object.pool == pool)
        .filter_map(|object| object.uuid().map(SnapshotId::from))
        .collect();
    if handed.is_empty() {
        m.areas.snapshots.selected.clone()
    } else {
        handed
    }
}

/// The selected snapshots of the shown VM, in the table's order.
pub fn selected_snapshots(m: &Model) -> Vec<Snapshot> {
    let selected = selected_ids(m);
    shown_rows(m)
        .into_iter()
        .filter(|snapshot| selected.contains(&snapshot.id))
        .collect()
}

/// The one selected snapshot: `None` with none or several selected.
pub fn selected_snapshot(m: &Model) -> Option<Snapshot> {
    let mut selected = selected_snapshots(m).into_iter();
    let first = selected.next()?;
    selected.next().is_none().then_some(first)
}

/// The table's selection handler: all selected row ids after the change.
/// A handed-over grid selection gives way to it, and a saved preview that
/// belongs to another snapshot is dropped.
pub fn select(m: &mut Model, ids: &[String]) {
    m.areas.snapshots.selected = ids
        .iter()
        .filter_map(|id| uuid::Uuid::parse_str(id).ok().map(SnapshotId::from))
        .collect();
    let handed = m
        .selection
        .grid
        .objects
        .iter()
        .any(|object| object.kind == ObjectKind::Snapshot);
    if handed {
        m.select_in_grid(Target::none());
    }
    inspection::follow(m);
}

/// "Show list" / "Show ancestry". The selection is by id and stays.
pub fn toggle_tree(m: &mut Model) {
    m.areas.snapshots.tree = !m.areas.snapshots.tree;
}

// -- Opening the dialogs --------------------------------------------------------

pub fn current(m: &Model, pool: PoolId) -> bool {
    m.can_mutate_pool(pool)
}

/// Why nothing can be changed in a pool that is neither synthetic nor
/// writable and current: the Rune page's read-only caption or its stale one.
pub fn not_writable(m: &Model, pool: PoolId) -> &'static str {
    if m.connection_for(pool).is_some_and(|c| c.is_read_only()) {
        READ_ONLY
    } else {
        NOT_CURRENT
    }
}

pub fn memory_available(inventory: &Inventory, id: VmId) -> bool {
    inventory.details.vms.get(&id).is_some_and(|vm| {
        !vm.blocked_operations.contains_key("checkpoint")
            && vm
                .allowed_operations
                .as_ref()
                .is_some_and(|operations| operations.iter().any(|operation| operation == "checkpoint"))
    })
}

pub fn begin_create(m: &mut Model, pool: PoolId, vm: VmId) {
    if m.areas.snapshots.capture.busy() {
        return;
    }
    m.areas.snapshots.capture.opt_in = false;
    m.areas.snapshots.capture.message = None;
    let result = (|| {
        let inventory = m.inventory_for(pool).ok_or(Error::Stale)?;
        let guest = inventory.vms.get(&vm).ok_or(Error::Stale)?;
        let target = if m.is_synthetic_pool(pool) {
            Token::Demo
        } else {
            Token::Live(m.snapshot_for(pool).ok_or(Error::Stale)?.observe_vm(vm)?)
        };
        let a = &mut m.areas.snapshots;
        a.name.clear();
        a.description.clear();
        a.memory = !guest.can_snapshot && memory_available(&inventory, vm);
        a.dialog = Some(Dialog::Create {
            pool,
            target,
            vm: guest.name.clone(),
            id: vm,
        });
        Ok::<_, Error>(())
    })();
    m.error = result.err().map(|error| error.to_string());
}

pub fn begin_restore(m: &mut Model) {
    let result = (|| {
        let (pool, vm) = shown_vm(m).ok_or(Error::Stale)?;
        let inventory = m.inventory_for(pool).ok_or(Error::Stale)?;
        let guest = inventory.vms.get(&vm).ok_or(Error::Stale)?;
        let selected = selected_snapshot(m).ok_or(Error::Stale)?;
        let demo = m.is_synthetic_pool(pool);
        let target = if demo {
            Token::Demo
        } else {
            Token::Live(
                m.snapshot_for(pool)
                    .ok_or(Error::Stale)?
                    .observe_snapshot(vm, selected.id)?,
            )
        };
        let vm_target = if demo {
            Token::Demo
        } else {
            Token::Live(m.snapshot_for(pool).ok_or(Error::Stale)?.observe_vm(vm)?)
        };
        m.areas.snapshots.dialog = Some(Dialog::Restore {
            pool,
            target,
            vm: guest.name.clone(),
            id: vm,
            snapshot: selected.name.clone(),
            snapshot_id: selected.id,
            vm_target,
            expected_state: guest.power_state.clone(),
            checkpoint: selected.power_state == PowerState::Suspended,
            shutdown: false,
            start_after: false,
        });
        Ok::<_, Error>(())
    })();
    m.error = result.err().map(|error| error.to_string());
}

pub fn begin_edit(m: &mut Model) {
    match SnapshotReview::capture(m) {
        Ok(source) => {
            let a = &mut m.areas.snapshots;
            a.name = source.snapshot.clone();
            a.description = source.description.clone();
            a.dialog = Some(Dialog::Edit { source });
            m.error = None;
        }
        Err(error) => m.error = Some(error.to_string()),
    }
}

pub fn begin_clone(m: &mut Model, as_template: bool) {
    match SnapshotReview::capture(m) {
        Ok(source) => {
            let a = &mut m.areas.snapshots;
            a.name.clear();
            a.dialog = Some(Dialog::Clone {
                source,
                as_template,
            });
            m.error = None;
        }
        Err(error) => m.error = Some(error.to_string()),
    }
}

pub fn begin_delete(m: &mut Model) {
    let result = (|| {
        let source = SnapshotReview::capture(m)?;
        let inventory = m.inventory_for(source.pool).ok_or(Error::Stale)?;
        let disk_inventory_available = ["vbd", "vdi"].iter().all(|class| {
            inventory.details.classes.get(*class) == Some(&ClassAvailability::Available)
        });
        let candidates = inventory.snapshot_deletion_disks(source.snapshot_id);
        let mut disks = Vec::new();
        let mut retained = Vec::new();
        let attached: BTreeSet<_> = inventory
            .details
            .vbds
            .values()
            .filter(|vbd| vbd.snapshot == Some(source.snapshot_id))
            .filter_map(|vbd| vbd.vdi)
            .collect();
        for id in attached {
            let record = inventory.details.vdis.get(&id);
            let disk = ReviewedDisk {
                id,
                name: record
                    .map_or("Disk details unavailable", |disk| disk.name.as_str())
                    .into(),
                size: record.and_then(|disk| disk.virtual_size),
            };
            if disk_inventory_available && candidates.contains(&id) {
                disks.push(disk);
            } else {
                retained.push(disk);
            }
        }
        let memory_checkpoint = inventory
            .snapshots
            .get(&source.snapshot_id)
            .is_some_and(|snapshot| snapshot.power_state == PowerState::Suspended);
        m.areas.snapshots.dialog = Some(Dialog::Delete {
            source,
            disks,
            retained,
            disk_inventory_available,
            delete_disks: false,
            memory_checkpoint,
        });
        Ok::<_, Error>(())
    })();
    m.error = result.err().map(|error| error.to_string());
}

// -- The checks of the open dialog -------------------------------------------

/// Something that keeps the open dialog from being confirmed, in the words
/// the Rune dialog used. `shown` is false where Rune disabled the button
/// without a line (an empty name, a snapshot renamed under the editor).
pub struct Problem {
    pub text: String,
    pub alert: bool,
    pub shown: bool,
}

fn status(text: &str) -> Problem {
    Problem {
        text: text.into(),
        alert: false,
        shown: true,
    }
}

fn alert_if(text: impl Into<String>, shown: bool) -> Problem {
    Problem {
        text: text.into(),
        alert: true,
        shown,
    }
}

/// The Rune `valid` of each dialog, as the list of what is wrong, in the
/// order the dialog shows it.
pub fn problems(m: &Model, dialog: &Dialog) -> Vec<Problem> {
    let a = &m.areas.snapshots;
    let demo = m.is_synthetic_pool(dialog.pool());
    let inventory = m.inventory_for(dialog.pool());
    let mut found = Vec::new();
    match dialog {
        Dialog::Create { id, .. } => {
            let selected_mode_available = inventory.as_ref().is_some_and(|i| {
                if a.memory {
                    memory_available(i, *id)
                } else {
                    i.vms.get(id).is_some_and(|vm| vm.can_snapshot)
                }
            });
            if !demo && !selected_mode_available {
                found.push(status(MODE_UNAVAILABLE));
            }
            let valid = SnapshotWorkflow::Create {
                name: a.name.trim().into(),
                description: a.description.clone(),
                memory: a.memory,
            }
            .validate()
            .is_ok();
            if !valid {
                found.push(alert_if(
                    Error::InvalidSnapshotMetadata.to_string(),
                    !a.name.is_empty(),
                ));
            }
        }
        Dialog::Restore {
            id,
            snapshot_id,
            expected_state,
            checkpoint,
            shutdown,
            start_after,
            ..
        } => {
            let operation = SnapshotWorkflow::Restore {
                snapshot: *snapshot_id,
                expected_state: expected_state.clone(),
                checkpoint: *checkpoint,
                shutdown: *shutdown,
                start_after: *start_after,
            };
            let valid = operation.validate().is_ok()
                && inventory.as_ref().is_some_and(|i| {
                    i.vms.get(id).is_some_and(|vm| {
                        vm.power_state == *expected_state
                            && (!shutdown
                                || vm.allowed_actions.contains(&PowerAction::CleanShutdown))
                    }) && i.snapshots.get(snapshot_id).is_some_and(|snap| {
                        snap.vm == Some(*id)
                            && snap.power_state
                                == if *checkpoint {
                                    PowerState::Suspended
                                } else {
                                    PowerState::Halted
                                }
                            && snap.allowed_actions.contains(&SnapshotAction::Revert)
                    })
                });
            if !valid {
                found.push(status(RESTORE_INVALID));
            }
        }
        Dialog::Edit { source } => {
            let valid = SnapshotWorkflow::Edit {
                snapshot: source.snapshot_id,
                expected_name: source.snapshot.clone(),
                expected_description: source.description.clone(),
                name: a.name.trim().into(),
                description: a.description.clone(),
            }
            .validate()
            .is_ok();
            if !valid {
                found.push(alert_if(Error::InvalidSnapshotMetadata.to_string(), false));
            }
            let unchanged = inventory.as_ref().is_some_and(|i| {
                i.pool_id == source.pool
                    && i.snapshots.get(&source.snapshot_id).is_some_and(|snap| {
                        snap.vm == Some(source.id)
                            && snap.name == source.snapshot
                            && snap.description == source.description
                    })
            });
            if !unchanged {
                found.push(alert_if(Error::Stale.to_string(), false));
            }
        }
        Dialog::Clone { source, .. } => {
            if SnapshotName::new(&a.name).is_err() {
                found.push(alert_if(CLONE_NAME, !a.name.is_empty()));
            }
            if !demo && !source.permits(m, SnapshotAction::Clone) {
                found.push(status(CLONE_CHANGED));
            }
        }
        Dialog::Delete {
            source,
            disks,
            delete_disks,
            ..
        } => {
            let ownership_current = !delete_disks
                || inventory.as_ref().is_some_and(|inventory| {
                    inventory.pool_id == source.pool && {
                        let eligible = inventory.snapshot_deletion_disks(source.snapshot_id);
                        disks.iter().all(|disk| eligible.contains(&disk.id))
                    }
                });
            if !ownership_current {
                found.push(alert_if(OWNERSHIP_CHANGED, true));
            }
            let permitted = source.permits(m, SnapshotAction::Delete)
                && matches!(
                    source.power_state,
                    PowerState::Halted | PowerState::Suspended
                );
            if !demo && !permitted {
                found.push(status(DELETE_CHANGED));
            }
        }
    }
    found
}

/// The Rune `live` of each dialog: the captured observation is still what
/// the pool's snapshot gives. Never in a synthetic pool.
pub fn live(m: &Model, dialog: &Dialog) -> bool {
    match dialog {
        Dialog::Create {
            pool, target, id, ..
        } => {
            matches!(target, Token::Live(observed) if m
                .snapshot_for(*pool)
                .and_then(|s| s.observe_vm(*id).ok())
                == Some(*observed))
        }
        Dialog::Restore {
            pool,
            target,
            id,
            snapshot_id,
            ..
        } => {
            matches!(target, Token::Live(observed) if m
                .snapshot_for(*pool)
                .and_then(|s| s.observe_snapshot(*id, *snapshot_id).ok())
                == Some(*observed))
        }
        Dialog::Edit { source } | Dialog::Clone { source, .. } | Dialog::Delete { source, .. } => {
            source.live(m)
        }
    }
}

/// Why a dialog of a live pool can no longer be confirmed whatever is
/// entered: what it captured is not current any more.
pub fn stale(m: &Model, dialog: &Dialog) -> Option<String> {
    let pool = dialog.pool();
    (!m.is_synthetic_pool(pool) && (!live(m, dialog) || !current(m, pool)))
        .then(|| Error::Stale.to_string())
}

pub fn review_requirements(m: &Model) -> Option<PermissionRequirements> {
    use SnapshotOperation as Op;
    let a = &m.areas.snapshots;
    let operation = match a.dialog.as_ref()? {
        Dialog::Create { .. } => Op::Workflow {
            operation: SnapshotWorkflow::Create {
                name: a.name.trim().into(),
                description: a.description.clone(),
                memory: a.memory,
            },
        },
        Dialog::Restore {
            snapshot_id,
            expected_state,
            checkpoint,
            shutdown,
            start_after,
            ..
        } => Op::Workflow {
            operation: SnapshotWorkflow::Restore {
                snapshot: *snapshot_id,
                expected_state: expected_state.clone(),
                checkpoint: *checkpoint,
                shutdown: *shutdown,
                start_after: *start_after,
            },
        },
        Dialog::Edit { source } => Op::Workflow {
            operation: SnapshotWorkflow::Edit {
                snapshot: source.snapshot_id,
                expected_name: source.snapshot.clone(),
                expected_description: source.description.clone(),
                name: a.name.trim().into(),
                description: a.description.clone(),
            },
        },
        Dialog::Clone {
            source,
            as_template,
        } => Op::Clone {
            snapshot: source.snapshot_id,
            name: a.name.trim().into(),
            as_template: *as_template,
        },
        Dialog::Delete {
            source,
            disks,
            delete_disks,
            ..
        } => Op::Delete {
            snapshot: source.snapshot_id,
            disks: if *delete_disks {
                disks.iter().map(|d| d.id).collect()
            } else {
                Vec::new()
            },
        },
    };
    Some(
        if let (
            Some(Dialog::Delete {
                memory_checkpoint, ..
            }),
            Op::Delete { disks, .. },
        ) = (a.dialog.as_ref(), &operation)
        {
            Op::deletion_permission_requirements(disks, Some(*memory_checkpoint))
        } else {
            operation.permission_requirements()
        },
    )
}

/// Why the confirm button of the open dialog is disabled, the first reason
/// that applies: what is wrong with the draft or the target, a stale
/// capture, a pending preview capture, a known permission denial, and a
/// synthetic pool, which never submits.
pub fn blocked(m: &Model) -> Option<String> {
    let a = &m.areas.snapshots;
    let dialog = a.dialog.as_ref()?;
    let pool = dialog.pool();
    if let Some(problem) = problems(m, dialog).into_iter().next() {
        return Some(problem.text);
    }
    if let Some(reason) = stale(m, dialog) {
        return Some(reason);
    }
    if a.capture.busy() {
        return Some(CAPTURE_PENDING.into());
    }
    if let Some(reason) = review_requirements(m)
        .map(|requirements| permissions::assess_for(m, pool, &requirements))
        .as_ref()
        .and_then(permissions::denial)
    {
        return Some(reason);
    }
    m.is_synthetic_pool(pool).then(|| DEMO.into())
}

// -- Submission ------------------------------------------------------------------

/// The confirm button. The order is Rune's: nothing while a preview
/// capture is pending; a known permission denial goes to `m.error`; the
/// command is built only from a live capture (a synthetic pool ends here);
/// then every check of the button again, which Rune left to the button;
/// with the preview opted in the capture starts and the dialog stays;
/// otherwise the command is submitted and, when it was accepted, the dialog
/// closes (the description draft is kept, as in Rune).
pub fn submit(m: &mut Model) {
    if m.areas.snapshots.capture.busy() {
        return;
    }
    let Some(pool) = m.areas.snapshots.dialog.as_ref().map(Dialog::pool) else {
        return;
    };
    if let Some(requirements) = review_requirements(m) {
        let assessment = permissions::assess_for(m, pool, &requirements);
        if let Some(reason) = permissions::denial(&assessment) {
            m.error = Some(reason);
            return;
        }
    }
    let a = &m.areas.snapshots;
    let command = match &a.dialog {
        Some(Dialog::Create {
            target: Token::Live(vm),
            ..
        }) => VmCommand::SnapshotWorkflow {
            vm: *vm,
            operation: SnapshotWorkflow::Create {
                name: a.name.trim().into(),
                description: a.description.clone(),
                memory: a.memory,
            },
        },
        Some(Dialog::Restore {
            vm_target: Token::Live(vm),
            snapshot_id,
            expected_state,
            checkpoint,
            shutdown,
            start_after,
            ..
        }) => VmCommand::SnapshotWorkflow {
            vm: *vm,
            operation: SnapshotWorkflow::Restore {
                snapshot: *snapshot_id,
                expected_state: expected_state.clone(),
                checkpoint: *checkpoint,
                shutdown: *shutdown,
                start_after: *start_after,
            },
        },
        Some(Dialog::Edit {
            source:
                SnapshotReview {
                    vm_target: Token::Live(vm),
                    snapshot_id,
                    snapshot,
                    description,
                    ..
                },
        }) => VmCommand::SnapshotWorkflow {
            vm: *vm,
            operation: SnapshotWorkflow::Edit {
                snapshot: *snapshot_id,
                expected_name: snapshot.clone(),
                expected_description: description.clone(),
                name: a.name.trim().into(),
                description: a.description.clone(),
            },
        },
        Some(Dialog::Clone {
            source:
                SnapshotReview {
                    target: Token::Live(snapshot),
                    ..
                },
            as_template,
        }) => match SnapshotName::new(&a.name) {
            Ok(name) => VmCommand::CloneSnapshot {
                snapshot: *snapshot,
                name,
                as_template: *as_template,
            },
            Err(error) => {
                m.error = Some(error.to_string());
                return;
            }
        },
        Some(Dialog::Delete {
            source:
                SnapshotReview {
                    target: Token::Live(snapshot),
                    ..
                },
            disks,
            delete_disks,
            ..
        }) => VmCommand::DeleteSnapshot {
            snapshot: *snapshot,
            disks: if *delete_disks {
                disks.iter().map(|disk| disk.id).collect()
            } else {
                Vec::new()
            },
        },
        _ => return,
    };
    if let Some(reason) = blocked(m) {
        m.error = Some(reason);
        return;
    }
    if m.areas.snapshots.capture.opt_in
        && let Some(Dialog::Create { id, .. }) = m.areas.snapshots.dialog
    {
        capture::begin(m, pool, command, id);
        return;
    }
    m.error = crate::areas::power::track_vm(m, pool, command)
        .err()
        .map(|error| error.to_string());
    if m.error.is_none() {
        let a = &mut m.areas.snapshots;
        a.dialog = None;
        a.name.clear();
        a.memory = false;
    }
}

// -- The handlers of the drafts ------------------------------------------------

pub fn set_name(m: &mut Model, text: &str) {
    m.areas.snapshots.name = text.to_owned();
}

pub fn set_description(m: &mut Model, text: &str) {
    m.areas.snapshots.description = text.to_owned();
}

pub fn toggle_memory(m: &mut Model) {
    m.areas.snapshots.memory = !m.areas.snapshots.memory;
}

pub fn toggle_capture(m: &mut Model) {
    let capture = &mut m.areas.snapshots.capture;
    capture.opt_in = !capture.opt_in;
}

pub fn toggle_shutdown(m: &mut Model) {
    if let Some(Dialog::Restore { shutdown, .. }) = &mut m.areas.snapshots.dialog {
        *shutdown = !*shutdown;
    }
}

pub fn toggle_start(m: &mut Model) {
    if let Some(Dialog::Restore { start_after, .. }) = &mut m.areas.snapshots.dialog {
        *start_after = !*start_after;
    }
}

pub fn toggle_delete_disks(m: &mut Model) {
    if let Some(Dialog::Delete { delete_disks, .. }) = &mut m.areas.snapshots.dialog {
        *delete_disks = !*delete_disks;
    }
}
