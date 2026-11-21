//! The recovery reviews of a VM: a task-aware force shutdown or reboot, a
//! boot from recovery media, the restoration of the boot order such a boot
//! saved, and the reviewed cancellation of one of this client's operations.
//!
//! Carried over from the Rune `vm_recovery_view.rs`. Its panel opened on an
//! overview and offered the reviews as buttons; here each review is opened
//! by a command or by a control under an operation record, so a panel
//! always holds a review. What is unchanged: the inventory and the
//! observation are captured when the review opens, the offered operation is
//! built from that captured inventory, and it is checked against the live
//! inventory until it is confirmed.

use std::sync::Arc;

use xenadmin_core::{
    ForceRecoveryAction, Inventory, ObservedVm, OperationAction, OperationControl, OperationId,
    OperationRecord, OperationState, PoolId, PowerAction, RecoveryBootConfiguration, StepState,
    VmCommand, VmId, VmRecoveryOperation,
};

use super::{DEMO, Guest, NOT_CURRENT, not_writable, permitted, submit_vm, synthetic};
use crate::prelude::*;

/// Shown in a forced-action review that would interrupt a migration (lab finding 63).
pub const MIGRATION_CAUTION: &str = "A migration of this VM is running. The host admits a forced action during one and does not always end the migration afterwards: on the project's test lab its task then stayed pending and left partial disk copies that took a host restart to clear. If the client that started the migration can still cancel it, do that first and wait for it to end. This action is refused unless you accept below what it can leave behind.";
/// Why the confirm button waits on a migration: the caution's last sentence.
pub const ACCEPT_FIRST: &str =
    "This action is refused unless you accept below what it can leave behind.";
pub const ACCEPT_MIGRATION: &str =
    "Force anyway: I accept that the migration may be left unfinished on the hosts";
pub const STALE: &str =
    "The captured connection changed. Close this dialog and reopen it from current inventory.";
pub const CANCEL_CHANGED: &str = "The operation or connection changed. Close this dialog and review its current state before requesting cancellation.";
pub const CANCEL: &str = "Request cancellation through this client's operation journal. Some workflows only stop their remaining steps; an in-flight host task may still finish normally. This does not cancel another client's tasks, reset VM state, or submit a force action. Wait for the recorded outcome, then refresh and review any force action separately.";
pub const FORCE: &str = "This can interrupt guest writes and lose data. The host will freshly validate the force action even when it is missing from the host’s usual action list. Administrative blocks and session permissions still apply. No task listed here will be cancelled by this action.";
pub const BOOT: &str = "The VM starts once using CD/network boot. Its original boot order is restored only after verified startup and only while the temporary value is unchanged. Firmware, Secure Boot policy and mounted media are preserved. This does not verify that a guest recovery environment actually started. A lost reply stops later writes and can leave the temporary order in place.";
pub const RESTORE: &str = "Restore only the exact temporary order set by this completed or acknowledged recovery operation. The backend requires a saved setter completion receipt and unchanged firmware/settings. This changes next-boot configuration only; it does not start, stop or cancel anything.";
pub const TASKS_NOTE: &str = "These are the task identities captured when this dialog opened. They may belong to any client. Close and reopen it before reviewing a changed VM.";
/// Why a force action is not offered as a recovery action.
pub const FORCE_UNAVAILABLE: &str = "Task-aware force recovery requires a running VM with known current task identities. For an idle VM, use its normal power actions.";
/// Why a boot from recovery media is not offered.
pub const BOOT_UNAVAILABLE: &str = "Recovery boot requires a halted, idle HVM guest. PV guests and changing installed BIOS/UEFI firmware are unsupported.";
/// The note of the Rune overview on unresolved operations, for whoever
/// lists them.
pub const RESERVATIONS: &str = "Unresolved local operations retain their reservations. Inspect unknown outcomes on the Events tab of the Notifications view before acknowledging them; cancellation requests do not release reservations or prove host cancellation.";

#[derive(Clone)]
pub enum Review {
    /// The cancellation of one of this client's operations, bound to the
    /// revision of its record when the review opened.
    Cancel {
        id: OperationId,
        revision: u64,
        label: String,
    },
    Action(VmRecoveryOperation),
}

pub struct Panel {
    pub target: Option<ObservedVm>,
    pub pool: PoolId,
    pub vm: VmId,
    pub name: String,
    /// The inventory when the review opened: the offered operation was
    /// built from it and is never rebuilt.
    pub initial: Arc<Inventory>,
    pub review: Review,
}

/// A panel for a guest, as the Rune `open` captured it, with its review.
pub(crate) fn capture(m: &Model, g: &Guest, review: Review) -> Panel {
    Panel {
        target: m.snapshot_for(g.pool).and_then(|s| s.observe_vm(g.id).ok()),
        pool: g.pool,
        vm: g.id,
        name: g.vm().name.clone(),
        initial: g.inventory.clone(),
        review,
    }
}

pub(crate) fn open(m: &mut Model, panel: Panel) {
    m.areas.power.recovery = Some(Box::new(panel));
}

pub(crate) fn close(m: &mut Model) {
    m.areas.power.recovery = None;
}

/// The VM is still in its pool's inventory and, outside the demo, still
/// the one observed when the review opened.
pub(crate) fn current(m: &Model, p: &Panel) -> bool {
    m.inventory_for(p.pool)
        .is_some_and(|i| i.vms.contains_key(&p.vm))
        && (synthetic(m, p.pool)
            || p.target.is_some_and(|o| {
                m.snapshot_for(p.pool).and_then(|s| s.observe_vm(p.vm).ok()) == Some(o)
            }))
}

/// The recovery action a power action stands for.
pub(crate) fn force_action(action: PowerAction) -> Option<ForceRecoveryAction> {
    match action {
        PowerAction::HardShutdown => Some(ForceRecoveryAction::Shutdown),
        PowerAction::HardReboot => Some(ForceRecoveryAction::Reboot),
        _ => None,
    }
}

/// The task-aware force action for a VM, with the tasks the inventory
/// lists for it now. `Err` is why it is not offered.
pub(crate) fn force_operation(
    inventory: &Inventory,
    vm: VmId,
    action: ForceRecoveryAction,
) -> Result<VmRecoveryOperation, &'static str> {
    VmRecoveryOperation::tasks(inventory, vm)
        .map(|tasks| VmRecoveryOperation::Force {
            action,
            tasks,
            interrupts_migration: false,
        })
        .map_err(|_| FORCE_UNAVAILABLE)
}

/// Whether a forced action would interrupt a migration among its tasks.
pub(crate) fn over_migration(operation: &VmRecoveryOperation) -> bool {
    matches!(operation, VmRecoveryOperation::Force { tasks, .. } if tasks.iter().any(|t| t.is_migration()))
}

/// The boot from recovery media for a VM. `Err` is why it is not offered.
pub(crate) fn boot_operation(
    inventory: &Inventory,
    vm: VmId,
) -> Result<VmRecoveryOperation, &'static str> {
    RecoveryBootConfiguration::from_inventory(inventory, vm)
        .map(|configuration| VmRecoveryOperation::Boot { configuration })
        .map_err(|_| BOOT_UNAVAILABLE)
}

/// The restoration a finished record offers for `vm`: the record is
/// terminal, it saved a boot order, and no other record of the VM has
/// begun restoring from it.
fn restoration_of(m: &Model, r: &OperationRecord, vm: VmId) -> Option<VmRecoveryOperation> {
    if !r.state.terminal() {
        return None;
    }
    let operation = r.recovery_boot_restoration()?;
    let attempted = m.operation_rows.iter().any(|other| {
        other.id != r.id
            && other.pool == r.pool
            && other.vm == vm
            && matches!(&other.action, OperationAction::Recovery(VmRecoveryOperation::RestoreBoot { source, .. }) if *source == r.id)
            && other
                .steps
                .iter()
                .any(|step| !matches!(step.state, StepState::Pending))
    });
    (!attempted).then_some(operation)
}

/// The boot orders that can be restored for a guest, in the order of the
/// operation records.
pub(crate) fn restorations(m: &Model, g: &Guest) -> Vec<VmRecoveryOperation> {
    m.operation_rows
        .iter()
        .filter(|r| r.pool == g.pool && (r.vm == g.id || r.created_vm == Some(g.id)))
        .filter_map(|r| restoration_of(m, r, g.id))
        .collect()
}

/// Why a recovery action is not offered for review now; `None` when it is.
/// The rule of the Rune button: the panel current, the operation available
/// in the live inventory, no known permission denial, and, outside the
/// demo, a pool that can be changed. The demo may open the review.
pub(crate) fn offer_blocked(m: &Model, p: &Panel) -> Option<String> {
    let Review::Action(operation) = &p.review else {
        return None;
    };
    if let Some(why) = m
        .inventory_for(p.pool)
        .and_then(|i| operation.unavailable_reason(&i, p.vm))
    {
        return Some(why);
    }
    let permission = permissions::assess_for(m, p.pool, &operation.permission_requirements());
    if let Some(denial) = permissions::denial(&permission) {
        return Some(denial);
    }
    if synthetic(m, p.pool) {
        return None;
    }
    if !m.can_mutate_pool(p.pool) {
        return Some(not_writable(m, p.pool).into());
    }
    (!current(m, p)).then(|| NOT_CURRENT.into())
}

/// The eligibility of a command that opens the review of `operation` for a
/// guest.
pub(crate) fn offer(m: &Model, g: &Guest, operation: VmRecoveryOperation) -> Eligibility {
    Eligibility::enabled_unless(offer_blocked(m, &capture(m, g, Review::Action(operation))))
}

/// Whether the record a cancellation was reviewed for is still that record:
/// same operation of this VM in this pool, the same revision, still
/// cancellable, not yet asked to cancel, unfinished and not unknown.
pub(crate) fn cancellable(m: &Model, p: &Panel, id: OperationId, revision: u64) -> bool {
    m.operation_rows.iter().any(|r| {
        r.id == id
            && r.pool == p.pool
            && (r.vm == p.vm || r.created_vm == Some(p.vm))
            && r.revision() == revision
            && r.can_cancel
            && !r.cancel_requested
            && !r.state.terminal()
            && r.state != OperationState::OutcomeUnknown
    })
}

/// Why the reviewed action cannot be confirmed, the first that applies;
/// `None` when it can. The demo first: it has no observation at all.
pub(crate) fn confirm_blocked(m: &Model, p: &Panel) -> Option<String> {
    if synthetic(m, p.pool) {
        return Some(DEMO.into());
    }
    if !current(m, p) {
        return Some(STALE.into());
    }
    if !m.can_mutate_pool(p.pool) {
        return Some(not_writable(m, p.pool).into());
    }
    match &p.review {
        Review::Cancel { id, revision, .. } => {
            (!cancellable(m, p, *id, *revision)).then(|| CANCEL_CHANGED.into())
        }
        Review::Action(operation) => {
            if let Some(why) = m
                .inventory_for(p.pool)
                .and_then(|i| operation.unavailable_reason(&i, p.vm))
            {
                return Some(why);
            }
            let permission =
                permissions::assess_for(m, p.pool, &operation.permission_requirements());
            if let Some(denial) = permissions::denial(&permission) {
                return Some(denial);
            }
            operation
                .migration_unaccepted()
                .then(|| ACCEPT_FIRST.into())
        }
    }
}

/// The Rune `apply`: every condition of the confirm button again, then the
/// cancellation request or the recovery command. The review closes when
/// the request was made or the command accepted; a refused command leaves
/// it open with the reason in `m.error`.
pub(crate) fn apply(m: &mut Model) {
    let Some(p) = m.areas.power.recovery.as_ref() else {
        return;
    };
    if !current(m, p) || !m.can_mutate_pool(p.pool) {
        return;
    }
    let Some(target) = p.target else { return };
    let (pool, vm) = (p.pool, p.vm);
    match p.review.clone() {
        Review::Cancel { id, revision, .. } => {
            if !cancellable(m, p, id, revision) {
                return;
            }
            super::control(m, id, OperationControl::RequestCancellation);
            close(m);
        }
        Review::Action(operation) => {
            if !m
                .inventory_for(pool)
                .is_some_and(|i| operation.unavailable_reason(&i, vm).is_none())
                || operation.migration_unaccepted()
                || !permitted(m, pool, &operation.permission_requirements())
            {
                return;
            }
            submit_vm(
                m,
                pool,
                VmCommand::Recovery {
                    vm: target,
                    operation,
                },
            );
            if m.error.is_none() {
                close(m);
            }
        }
    }
}

/// Gives or withdraws, in the open review of a forced action, the operator's acceptance
/// that a captured migration may be left unfinished.
pub(crate) fn accept_migration(m: &mut Model) {
    if let Some(Review::Action(VmRecoveryOperation::Force {
        tasks,
        interrupts_migration,
        ..
    })) = m.areas.power.recovery.as_mut().map(|p| &mut p.review)
        && tasks.iter().any(|t| t.is_migration())
    {
        *interrupts_migration = !*interrupts_migration;
    }
}

pub(crate) fn order(c: &RecoveryBootConfiguration) -> String {
    c.order
        .clone()
        .unwrap_or_else(|| "host default (order key absent)".into())
}

/// The lines of "Current host tasks" for a VM: what the host is doing with
/// it, for whichever client. Empty when the inventory has no details of
/// the VM.
pub fn host_tasks(inventory: &Inventory, vm: VmId) -> Vec<String> {
    let Some(d) = inventory.details.vms.get(&vm) else {
        return Vec::new();
    };
    if d.current_operations.is_empty() {
        return vec!["No current host tasks were reported.".into()];
    }
    d.current_operations
        .iter()
        .map(|t| {
            let name = t
                .task
                .and_then(|id| inventory.details.tasks.get(&id))
                .map(|t| t.name.as_str())
                .unwrap_or("Task details unavailable");
            format!(
                "{} · {name}\n{}",
                t.operation,
                t.task
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| "Task identity unavailable".into())
            )
        })
        .collect()
}

// -- Entry points on an operation record ---------------------------------------

/// The guest an operation record is about: the VM it acts on or, when that
/// is not a guest of the inventory, the VM it created.
fn guest_of(m: &Model, record: &OperationRecord) -> Option<Guest> {
    std::iter::once(record.vm)
        .chain(record.created_vm)
        .find_map(|vm| Guest::of_object(m, &ObjectRef::vm(record.pool, vm)).ok())
}

/// Whether Rune offered "Review cancellation request…" for a record.
pub(crate) fn offers_cancellation(record: &OperationRecord) -> bool {
    !record.state.terminal()
        && record.can_cancel
        && !record.cancel_requested
        && record.state != OperationState::OutcomeUnknown
}

/// Opens the review of a cancellation request for one of this client's
/// unfinished operations. It is bound to the record as it is now: when the
/// record has moved on by the time of the confirmation, nothing is asked.
pub fn review_cancellation(m: &mut Model, id: OperationId) {
    let Some(record) = m.operation_rows.iter().find(|r| r.id == id) else {
        return;
    };
    if !offers_cancellation(record) {
        return;
    }
    let Some(g) = guest_of(m, record) else { return };
    let review = Review::Cancel {
        id,
        revision: record.revision(),
        label: record.action.label().into(),
    };
    let panel = capture(m, &g, review);
    open(m, panel);
}

/// "Review cancellation request…" for a record that can be asked to cancel
/// (key `recovery-cancel-<id>`, the Rune key); `None` for any other record.
pub fn cancellation_review_button(m: &Model, record: &OperationRecord) -> Option<Button> {
    if !offers_cancellation(record) {
        return None;
    }
    let g = guest_of(m, record)?;
    let id = record.id;
    let refused =
        (!synthetic(m, g.pool) && !m.can_mutate_pool(g.pool)).then(|| not_writable(m, g.pool));
    Some(
        button(
            format!("recovery-cancel-{id}"),
            "Review cancellation request…",
            move |m| review_cancellation(m, id),
        )
        .disabled_opt(refused),
    )
}

/// The panel that would review the restoration a record offers.
fn restoration_panel(m: &Model, record: &OperationRecord) -> Option<Panel> {
    let g = guest_of(m, record)?;
    let operation = restoration_of(m, record, g.id)?;
    Some(capture(m, &g, Review::Action(operation)))
}

/// Opens the review of restoring the boot order that a finished recovery
/// boot saved. Nothing opens when the record offers none or the
/// restoration cannot be reviewed now.
pub fn review_restore_boot(m: &mut Model, id: OperationId) {
    let Some(record) = m.operation_rows.iter().find(|r| r.id == id) else {
        return;
    };
    let Some(panel) = restoration_panel(m, record) else {
        return;
    };
    if let Some(reason) = offer_blocked(m, &panel) {
        m.error = Some(reason);
        return;
    }
    open(m, panel);
}

/// "Restore saved boot order" under the record of a finished recovery boot
/// (key `recovery-restore-<id>`, the Rune key); `None` for any other
/// record.
pub(crate) fn restore_button(m: &Model, record: &OperationRecord) -> Option<Button> {
    let panel = restoration_panel(m, record)?;
    let Review::Action(operation) = &panel.review else {
        return None;
    };
    let id = record.id;
    Some(
        button(
            format!("recovery-restore-{id}"),
            operation.label(),
            move |m| review_restore_boot(m, id),
        )
        .disabled_opt(offer_blocked(m, &panel)),
    )
}
