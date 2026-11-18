//! The batch deletion: up to 32 VMs or custom templates of one pool,
//! reviewed and submitted as one `ResourceCommand::VmWorkflowBatch`.
//!
//! Carried over from the Rune `bulk_delete.rs`. The selection is the
//! command's target (the grid selection, or several objects of the tree)
//! instead of the selected rows of the Guests or Templates table; the pool
//! is the target's pool instead of the scope session's.

use std::collections::BTreeSet;
use std::sync::Arc;

use xenadmin_core::{
    BatchVmWorkflow, Error, Inventory, ObservedResource, OperationId, PermissionDecision, PoolId,
    PowerState, ResourceCommand, ResourceTarget, Snapshot, SnapshotAction,
    SnapshotDeletionSelection, SnapshotId, VdiId, Vm, VmLifecycleOperation, VmWorkflow,
    VmWorkflowBatch,
};

use crate::areas::vm_lifecycle;
use crate::prelude::*;

/// The refusal of a selection of more than 32. Nothing is ever cut down to
/// the limit silently.
pub const LIMIT: &str = "Select between 1 and 32 visible VMs or custom templates";
pub const NO_INVENTORY: &str = "Current inventory is unavailable";
pub const MISSING: &str = "A selected row is no longer a VM or custom template";
/// A selection across pools: the batch is one command to one pool. (New
/// text: the Rune table showed one pool at a time.)
pub const ONE_POOL: &str = "Select VMs or custom templates of one pool to delete them together";
pub const NOT_THE_POOL: &str = "The captured pool is no longer current";
/// What the dialog says while its pool observation is not the current one.
pub const STALE: &str = "The connection changed or inventory is stale. Reopen this review.";
pub const PREVIEW: &str = "Synthetic preview; submission is disabled.";
/// Why "Delete reviewed targets" waits when the inventory moved on under
/// the review without refusing it. (New text: Rune disabled the button
/// without saying why.)
pub const CHANGED: &str =
    "The deletions that would be reviewed now differ from this review. Go back and review again.";
pub const RECORD_ONLY: &str = "By default, delete only each VM/template record and its device attachments. Disks and snapshots are retained unless individually selected below.";
pub const SEQUENCE: &str = "Every selected target is checked before the first deletion. Targets run serially. Stop lets the current VM workflow finish, then skips the rest. An interruption never starts an unsubmitted target; partial deletions cannot be rolled back.";
pub const DISK_RETAINED: &str = "Retained: exclusive ownership is not established.";
pub const SNAPSHOT_RETAINED: &str = "Retained: checkpoints require their separate deletion workflow; ordinary snapshots must allow deletion.";

pub struct Editor {
    pub pool: PoolId,
    pub pool_name: String,
    /// The pool as observed when the selection was captured: the token the
    /// batch is submitted with.
    pub observed: Option<ObservedResource>,
    pub initial: Arc<Inventory>,
    pub choices: Vec<Choice>,
    pub review: Option<Review>,
    /// The typed confirmation: exactly `DELETE <n>`.
    pub confirm: String,
    pub error: Option<String>,
}

/// One target as captured, with what of it is to be deleted besides its
/// record: nothing unless chosen.
pub struct Choice {
    pub vm: Vm,
    pub disks: BTreeSet<VdiId>,
    pub snapshots: BTreeSet<SnapshotId>,
    pub snapshot_disks: BTreeSet<VdiId>,
}

#[derive(Clone, PartialEq)]
pub struct Review {
    pub batch: VmWorkflowBatch,
    /// The review lines of each target, in the order of the batch.
    pub rows: Vec<Vec<String>>,
}

fn edit(m: &mut Model) -> Option<&mut Editor> {
    m.areas.bulk_edit.delete.as_mut()
}

/// The VMs or templates of a target in its pool's inventory, in the
/// target's order (the Rune `selected`), or why the selection is refused.
pub(crate) fn selected(m: &Model, target: &Target) -> Result<(PoolId, Vec<Vm>), String> {
    let pool = target.pool().ok_or(ONE_POOL)?;
    let i = m.inventory_for(pool).ok_or(NO_INVENTORY)?;
    if !(1..=32).contains(&target.len()) {
        return Err(LIMIT.into());
    }
    let vms = target
        .objects
        .iter()
        .map(|object| {
            object
                .vm_id()
                .and_then(|id| i.vms.get(&id))
                .filter(|vm| !vm.is_control_domain)
                .cloned()
                .ok_or_else(|| MISSING.to_owned())
        })
        .collect::<Result<_, _>>()?;
    Ok((pool, vms))
}

/// The editor a target would open: the Rune `open` without its side
/// effects. Every target starts as a record-only deletion.
pub(crate) fn capture(m: &Model, target: &Target) -> Result<Editor, String> {
    let (pool, vms) = selected(m, target)?;
    let i = m.inventory_for(pool).ok_or(NO_INVENTORY)?;
    Ok(Editor {
        pool,
        pool_name: i.pool_name.clone(),
        observed: m
            .snapshot_for(pool)
            .and_then(|x| x.observe_resource(ResourceTarget::Pool(pool)).ok()),
        choices: vms
            .into_iter()
            .map(|vm| Choice {
                vm,
                disks: BTreeSet::new(),
                snapshots: BTreeSet::new(),
                snapshot_disks: BTreeSet::new(),
            })
            .collect(),
        initial: i,
        review: None,
        confirm: String::new(),
        error: None,
    })
}

/// What no choice in the dialog could fix: a pool that cannot be changed,
/// or a target that cannot be deleted even as a record only (choosing
/// disks or snapshots only adds conditions). The reason is the review's:
/// its first line and how many more there are.
pub(crate) fn hopeless(m: &Model, e: &Editor) -> Option<String> {
    if !m.is_synthetic_pool(e.pool) && !current(m, e) {
        return Some(STALE.into());
    }
    let refusal = build_review(m, e).err()?;
    let mut lines = refusal.lines();
    let first = lines.next().unwrap_or_default();
    Some(match lines.count() {
        0 => first.to_owned(),
        more => format!("{first} (and {more} more of the selection)"),
    })
}

/// Opens the batch for a target, on its draft. A refusal of the selection
/// is in `m.error` and nothing opens (the Rune `open`).
pub(crate) fn open(m: &mut Model, target: &Target) {
    match capture(m, target) {
        Ok(editor) => m.areas.bulk_edit.delete = Some(editor),
        Err(reason) => m.error = Some(reason),
    }
}

/// The pool is still the one observed and can be changed. Never in a
/// synthetic pool.
pub(crate) fn current(m: &Model, e: &Editor) -> bool {
    !m.is_synthetic_pool(e.pool)
        && m.can_mutate_pool(e.pool)
        && e.observed.is_some()
        && m.snapshot_for(e.pool)
            .and_then(|x| x.observe_resource(ResourceTarget::Pool(e.pool)).ok())
            == e.observed
}

/// The data disks of a target as captured: what the draft lists.
pub(crate) fn data_disks(e: &Editor, c: &Choice) -> BTreeSet<VdiId> {
    e.initial
        .details
        .vbds
        .values()
        .filter(|b| b.vm == Some(c.vm.id) && b.kind == "Disk")
        .filter_map(|b| b.vdi)
        .collect()
}

/// Whether a snapshot can be chosen for deletion with its VM.
pub(crate) fn snapshot_eligible(snapshot: &Snapshot) -> bool {
    snapshot.power_state == PowerState::Halted
        && snapshot.allowed_actions.contains(&SnapshotAction::Delete)
}

fn single(i: &Inventory, c: &Choice) -> VmLifecycleOperation {
    VmLifecycleOperation::Delete {
        expected_template: c.vm.is_template,
        disks: c.disks.iter().copied().collect(),
        snapshots: c
            .snapshots
            .iter()
            .map(|id| SnapshotDeletionSelection {
                snapshot: *id,
                disks: i
                    .snapshot_deletion_disks(*id)
                    .into_iter()
                    .filter(|id| c.snapshot_disks.contains(id))
                    .collect(),
            })
            .collect(),
    }
}

/// The batch that would be reviewed now. One target that changed, cannot
/// be deleted or whose snapshot disks changed refuses the whole batch: none
/// is dropped. A known permission denial refuses it too.
pub(crate) fn build_review(m: &Model, e: &Editor) -> Result<Review, String> {
    let i = m.inventory_for(e.pool).ok_or(NOT_THE_POOL)?;
    let i = i.as_ref();
    let mut items = Vec::new();
    let mut rows = Vec::new();
    let mut errors = Vec::new();
    for c in &e.choices {
        let Some(vm) = i.vms.get(&c.vm.id).filter(|vm| {
            !vm.is_control_domain
                && vm.is_template == c.vm.is_template
                && vm.power_state == c.vm.power_state
        }) else {
            errors.push(format!(
                "{} ({}): identity or power state changed",
                c.vm.name, c.vm.id
            ));
            continue;
        };
        let op = single(i, c);
        let initial = single(&e.initial, c);
        let availability = op.availability(i, vm.id);
        if !availability.is_available() {
            errors.push(format!(
                "{} ({}): {}",
                vm.name,
                vm.id,
                availability.explanation()
            ));
            continue;
        }
        if initial != op {
            errors.push(format!("{}: snapshot disk selection changed", vm.name));
            continue;
        }
        let VmLifecycleOperation::Delete {
            expected_template,
            disks,
            snapshots,
        } = op
        else {
            unreachable!()
        };
        items.push(BatchVmWorkflow {
            vm: vm.id,
            expected_state: vm.power_state.clone(),
            workflow: VmWorkflow::Delete {
                expected_template,
                disks,
                snapshots,
            },
        });
        rows.push(vm_lifecycle::deletion_review(
            i,
            vm,
            &c.disks,
            &c.snapshots,
            &c.snapshot_disks,
        ));
    }
    if !errors.is_empty() {
        return Err(errors.join("\n"));
    }
    let batch = VmWorkflowBatch { items };
    let permission = permissions::assess_for(m, e.pool, &batch.permission_requirements());
    if permission.decision().is_denied() {
        return Err(PermissionDecision::Denied.explanation().into());
    }
    Ok(Review { batch, rows })
}

pub(crate) fn review(m: &mut Model) {
    let Some(e) = m.areas.bulk_edit.delete.as_ref() else {
        return;
    };
    let built = build_review(m, e);
    let Some(e) = edit(m) else { return };
    match built {
        Ok(r) => {
            e.review = Some(r);
            e.confirm.clear();
            e.error = None;
        }
        Err(error) => e.error = Some(error),
    }
}

/// Back to the draft: the review and the typed confirmation go.
pub(crate) fn back(m: &mut Model) {
    if let Some(e) = edit(m) {
        e.review = None;
        e.confirm.clear();
    }
}

pub(crate) fn close(m: &mut Model) {
    m.areas.bulk_edit.delete = None;
}

pub(crate) fn set_confirm(m: &mut Model, text: &str) {
    if let Some(e) = edit(m) {
        e.confirm = text.to_owned();
    }
}

fn toggle<T: Ord>(set: &mut BTreeSet<T>, value: T) {
    if !set.remove(&value) {
        set.insert(value);
    }
}

pub(crate) fn toggle_disk(m: &mut Model, index: usize, id: VdiId) {
    if let Some(c) = edit(m).and_then(|e| e.choices.get_mut(index)) {
        toggle(&mut c.disks, id);
    }
}

pub(crate) fn toggle_snapshot(m: &mut Model, index: usize, id: SnapshotId) {
    if let Some(c) = edit(m).and_then(|e| e.choices.get_mut(index)) {
        toggle(&mut c.snapshots, id);
    }
}

pub(crate) fn toggle_snapshot_disk(m: &mut Model, index: usize, id: VdiId) {
    if let Some(c) = edit(m).and_then(|e| e.choices.get_mut(index)) {
        toggle(&mut c.snapshot_disks, id);
    }
}

/// The words the confirmation field must hold.
pub(crate) fn confirmation(e: &Editor) -> String {
    format!("DELETE {}", e.choices.len())
}

/// Why "Delete reviewed targets" is disabled, the first that applies;
/// `None` when the reviewed batch can be submitted. The Rune button gave no
/// reason: each is the text the dialog shows for that state.
pub(crate) fn blocked(m: &Model, e: &Editor, proposal: &Result<Review, String>) -> Option<String> {
    if m.is_synthetic_pool(e.pool) {
        return Some(PREVIEW.into());
    }
    if !current(m, e) {
        return Some(STALE.into());
    }
    match proposal {
        Err(reason) => return Some(reason.clone()),
        Ok(now) if Some(now) != e.review.as_ref() => return Some(CHANGED.into()),
        Ok(_) => {}
    }
    if e.confirm != confirmation(e) {
        return Some(format!(
            "Type {} to confirm irreversible deletion.",
            confirmation(e)
        ));
    }
    (m.resource_operations.len() >= 64).then(|| Error::Busy.to_string())
}

/// Submits the reviewed batch. Every refusal before the connection is the
/// same `Stale` in the editor's error line, as in Rune: the pool is not the
/// one observed, the confirmation is not typed exactly, the review is not
/// what would be reviewed now, or 64 resource operations are already held.
/// Accepted, the batch is tracked, the editor closes and the operations are
/// shown (Rune opened the pool history).
pub(crate) fn submit(m: &mut Model) {
    let result = (|| {
        let e = m.areas.bulk_edit.delete.as_ref().ok_or(Error::Stale)?;
        let reviewed = e.review.as_ref().ok_or(Error::Stale)?;
        if !current(m, e)
            || e.confirm != confirmation(e)
            || build_review(m, e).ok().as_ref() != Some(reviewed)
            || m.resource_operations.len() >= 64
        {
            return Err(Error::Stale);
        }
        m.connection_for(e.pool)
            .ok_or(Error::Closed)?
            .submit_resource(
                OperationId::new(),
                ResourceCommand::VmWorkflowBatch {
                    pool: e.observed.ok_or(Error::Stale)?,
                    selection: reviewed.batch.clone(),
                },
            )
    })();
    match result {
        Ok(operation) => {
            m.track_resource(operation);
            close(m);
            m.show_operations();
        }
        Err(error) => {
            if let Some(e) = edit(m) {
                e.error = Some(error.to_string());
            }
        }
    }
}
