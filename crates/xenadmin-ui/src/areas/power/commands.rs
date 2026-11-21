//! The commands of the area: what the power buttons of the Rune details
//! pane, "Start on host…", "Recovery controls…" and "Power selection…"
//! became.
//!
//! The eligibility of the nine power commands is one rule:
//!
//! - on several objects: `Hidden` unless all are VMs; `Disabled` when the
//!   batch could not be reviewed for the action (the reason is the batch's:
//!   the limit of 32, or "Unavailable for n VM(s): …"); the command opens
//!   the batch on its review;
//! - on one object: `Hidden` unless it is an ordinary VM; `Disabled` with
//!   the reason Rune showed under the power button
//!   ([`single::Facts::refusal`]); otherwise it asks for the action.
//!
//! Force Shut Down and Force Reboot have a second way for one VM: the
//! task-aware review of the recovery panel ([`route`]).

use xenadmin_core::{OperationRecord, OperationState, PowerAction, PowerState, VmRecoveryOperation};

use super::recovery::Review;
use super::{Guest, NOT_CURRENT, batch, placement, recovery, single, synthetic};
use crate::prelude::*;
use crate::session;

const AREA: &str = "power";

pub const START: CommandId = CommandId("vm.start");
pub const SHUT_DOWN: CommandId = CommandId("vm.shutdown");
pub const REBOOT: CommandId = CommandId("vm.reboot");
pub const SUSPEND: CommandId = CommandId("vm.suspend");
pub const RESUME: CommandId = CommandId("vm.resume");
pub const PAUSE: CommandId = CommandId("vm.pause");
pub const UNPAUSE: CommandId = CommandId("vm.unpause");
pub const FORCE_SHUT_DOWN: CommandId = CommandId("vm.force-shutdown");
pub const FORCE_REBOOT: CommandId = CommandId("vm.force-reboot");
pub const RECOVERY_START: CommandId = CommandId("vm.recovery-start");
pub const RECOVERY_RESTORE_BOOT: CommandId = CommandId("vm.recovery-restore-boot");
pub const START_ON_SERVER: CommandId = CommandId("vm.start-on-server");
pub const RESUME_ON_SERVER: CommandId = CommandId("vm.resume-on-server");
pub const POWER_SELECTION: CommandId = CommandId("vm.power-selection");
pub const CHECK_AVAILABILITY: CommandId = CommandId("vm.check-availability");
pub const REQUEST_CANCEL: CommandId = CommandId("operation.request-cancel");
pub const RECHECK: CommandId = CommandId("operation.recheck");
pub const REVIEW_UNKNOWN: CommandId = CommandId("operation.review-unknown");
pub const REVIEW_CANCELLATION: CommandId = CommandId("operation.review-cancellation");
pub const EXPLAIN: CommandId = CommandId("operation.explain");

/// The power command of each action, in the order of `PowerAction::ALL`.
pub const POWER: [(PowerAction, CommandId); 9] = [
    (PowerAction::Start, START),
    (PowerAction::Resume, RESUME),
    (PowerAction::Unpause, UNPAUSE),
    (PowerAction::Pause, PAUSE),
    (PowerAction::Suspend, SUSPEND),
    (PowerAction::CleanShutdown, SHUT_DOWN),
    (PowerAction::Reboot, REBOOT),
    (PowerAction::HardShutdown, FORCE_SHUT_DOWN),
    (PowerAction::HardReboot, FORCE_REBOOT),
];

const VM: &[ObjectKind] = &[ObjectKind::Vm];

/// One of the nine: the eligibility and the invocation are function
/// pointers, so each command names its action in its own closures.
macro_rules! power {
    ($id:expr, $label:expr, $place:expr, $action:expr) => {
        CommandSpec::new($id.0, $label, AREA)
            .menu($place)
            .context(VM)
            .multi()
            .eligibility(|m, target| eligibility(m, target, $action))
            .invoke(|m, target| invoke(m, target, $action))
    };
}

/// In menu order. Labels are the plan's (Menus table); the recovery boot is
/// its "recovery start", named as the reference client names it.
pub fn commands() -> Vec<CommandSpec> {
    use PowerAction as A;
    vec![
        power!(START, "Start", place::VM_START, A::Start).toolbar(ToolbarSlot::START),
        power!(
            SHUT_DOWN,
            "Shut Down",
            place::VM_SHUT_DOWN,
            A::CleanShutdown
        )
        .toolbar(ToolbarSlot::SHUT_DOWN),
        power!(REBOOT, "Reboot", place::VM_REBOOT, A::Reboot).toolbar(ToolbarSlot::REBOOT),
        power!(SUSPEND, "Suspend", place::VM_SUSPEND, A::Suspend).toolbar(ToolbarSlot::SUSPEND),
        power!(RESUME, "Resume", place::VM_RESUME, A::Resume).toolbar(ToolbarSlot::RESUME),
        power!(PAUSE, "Pause", place::VM_PAUSE, A::Pause),
        power!(UNPAUSE, "Unpause", place::VM_UNPAUSE, A::Unpause),
        power!(
            FORCE_SHUT_DOWN,
            "Force Shut Down",
            place::VM_FORCE_SHUT_DOWN,
            A::HardShutdown
        )
        .toolbar(ToolbarSlot::FORCE_SHUT_DOWN),
        power!(
            FORCE_REBOOT,
            "Force Reboot",
            place::VM_FORCE_REBOOT,
            A::HardReboot
        )
        .toolbar(ToolbarSlot::FORCE_REBOOT),
        CommandSpec::new(RECOVERY_START.0, "Start in Recovery Mode", AREA)
            .menu(place::VM_RECOVERY_START)
            .context(VM)
            .eligibility(can_boot_recovery)
            .invoke(boot_recovery),
        // Present only for a VM whose finished recovery boot left a boot
        // order to restore.
        CommandSpec::new(RECOVERY_RESTORE_BOOT.0, "Restore Saved Boot Order", AREA)
            .menu(place::VM_RECOVERY_RESTORE_BOOT)
            .context(VM)
            .eligibility(can_restore_boot)
            .invoke(restore_boot),
        CommandSpec::new(START_ON_SERVER.0, "Start on Server", AREA)
            .menu(place::VM_START_ON_SERVER)
            .context(VM)
            .eligibility(|m, target| can_place(m, target, &PowerState::Halted))
            .invoke(place_on_server),
        CommandSpec::new(RESUME_ON_SERVER.0, "Resume on Server", AREA)
            .menu(place::VM_RESUME_ON_SERVER)
            .context(VM)
            .eligibility(|m, target| can_place(m, target, &PowerState::Suspended))
            .invoke(place_on_server),
        CommandSpec::new(POWER_SELECTION.0, "Power actions for selection…", AREA)
            .context(VM)
            .multi()
            .eligibility(can_power_selection)
            .invoke(|m, target| batch::open(m, target, None)),
        CommandSpec::new(CHECK_AVAILABILITY.0, "Check action availability", AREA)
            .context(VM)
            .local(TabId::General, 10)
            .local_scopes(&[ObjectKind::Vm])
            .eligibility(can_check)
            .invoke(check),
        CommandSpec::new(REQUEST_CANCEL.0, "Request cancellation", AREA)
            .local(TabId::General, 20)
            .local_scopes(&[ObjectKind::Vm])
            .eligibility(|m, target| on_record(m, target, offers_cancel))
            .dynamic_label(|m, target| {
                record(m, target, offers_cancel).map(|record| {
                    if single::stops_steps(record) {
                        "Stop remaining steps".into()
                    } else {
                        "Request cancellation".into()
                    }
                })
            })
            .invoke(|m, target| {
                if let Some(record) = record(m, target, offers_cancel) {
                    single::request_cancellation(m, record.id);
                }
            }),
        CommandSpec::new(RECHECK.0, "Recheck outcome", AREA)
            .local(TabId::General, 30)
            .local_scopes(&[ObjectKind::Vm])
            .eligibility(|m, target| on_record(m, target, offers_recheck))
            .invoke(|m, target| {
                if let Some(record) = record(m, target, offers_recheck) {
                    single::recheck(m, record.id);
                }
            }),
        CommandSpec::new(REVIEW_UNKNOWN.0, "Review unknown outcome", AREA)
            .local(TabId::General, 40)
            .local_scopes(&[ObjectKind::Vm])
            .eligibility(|m, target| on_record(m, target, offers_unknown))
            .invoke(|m, target| {
                if let Some(record) = record(m, target, offers_unknown) {
                    single::review_unknown(m, record.id);
                }
            }),
        CommandSpec::new(REVIEW_CANCELLATION.0, "Review cancellation request…", AREA)
            .local(TabId::General, 50)
            .local_scopes(&[ObjectKind::Vm])
            .eligibility(can_review_cancellation)
            .invoke(|m, target| {
                if let Some(record) = record(m, target, offers_review_cancellation) {
                    recovery::review_cancellation(m, record.id);
                }
            }),
        CommandSpec::new(EXPLAIN.0, "Explain outcome…", AREA)
            .local(TabId::General, 60)
            .local_scopes(&[ObjectKind::Vm])
            .eligibility(|m, target| on_record(m, target, offers_explain))
            .invoke(|m, target| {
                if let Some(record) = record(m, target, offers_explain) {
                    let key = crate::areas::events::OperationKey {
                        pool: record.pool,
                        id: record.id,
                        family: crate::areas::events::Family::Vm,
                    };
                    crate::areas::events::explain(m, key);
                }
            }),
    ]
}

fn vm_of(target: &Target) -> Option<(xenadmin_core::PoolId, xenadmin_core::VmId)> {
    let object = target.single_of(ObjectKind::Vm)?;
    Some((object.pool?, object.vm_id()?))
}

fn record<'a>(
    m: &'a Model,
    target: &Target,
    offers: impl Fn(&Model, &OperationRecord) -> bool,
) -> Option<&'a OperationRecord> {
    let (pool, vm) = vm_of(target)?;
    crate::areas::general::operations::recent(m, pool, vm)
        .into_iter()
        .find(|record| offers(m, record))
}

fn on_record(
    m: &Model,
    target: &Target,
    offers: impl Fn(&Model, &OperationRecord) -> bool,
) -> Eligibility {
    if vm_of(target).is_none() {
        return Eligibility::Hidden;
    }
    if record(m, target, offers).is_some() {
        Eligibility::Enabled
    } else {
        Eligibility::Hidden
    }
}

fn offers_cancel(m: &Model, record: &OperationRecord) -> bool {
    let _ = m;
    record.batch_parent.is_none()
        && record.can_cancel
        && !record.cancel_requested
        && !record.state.terminal()
}

fn offers_recheck(m: &Model, record: &OperationRecord) -> bool {
    let _ = m;
    record.batch_parent.is_none()
        && record.state == OperationState::OutcomeUnknown
        && record.can_recheck()
}

fn offers_unknown(m: &Model, record: &OperationRecord) -> bool {
    let _ = m;
    record.batch_parent.is_none() && record.state == OperationState::OutcomeUnknown
}

fn offers_review_cancellation(_: &Model, record: &OperationRecord) -> bool {
    recovery::offers_cancellation(record)
}

fn offers_explain(m: &Model, record: &OperationRecord) -> bool {
    let key = crate::areas::events::OperationKey {
        pool: record.pool,
        id: record.id,
        family: crate::areas::events::Family::Vm,
    };
    crate::areas::events::can_explain(m, key)
}

fn can_review_cancellation(m: &Model, target: &Target) -> Eligibility {
    let Some(pool) = record(m, target, offers_review_cancellation).map(|record| record.pool)
    else {
        return Eligibility::Hidden;
    };
    if synthetic(m, pool) || m.can_mutate_pool(pool) {
        Eligibility::Enabled
    } else {
        Eligibility::disabled(super::not_writable(m, pool))
    }
}

/// The two ways a power action reaches one VM.
enum Route {
    /// The Rune power button: the request, and the confirmation when the
    /// action is reviewed. With the reason it is refused, if it is.
    Ordinary(Option<String>),
    /// The task-aware force review of the recovery panel.
    TaskAware(VmRecoveryOperation),
}

/// A force action goes to the task-aware review when the ordinary action is
/// refused and the VM's current tasks are known, and whenever one of those
/// tasks is a migration (the review that holds the action back until its
/// consequence is accepted). Everything else is the ordinary way.
fn route(m: &Model, g: &Guest, action: PowerAction) -> Route {
    let refusal = single::refusal(m, g, action);
    let task_aware = recovery::force_action(action)
        .and_then(|force| recovery::force_operation(&g.inventory, g.id, force).ok());
    match task_aware {
        Some(operation) if refusal.is_some() || recovery::over_migration(&operation) => {
            Route::TaskAware(operation)
        }
        _ => Route::Ordinary(refusal),
    }
}

fn eligibility(m: &Model, target: &Target, action: PowerAction) -> Eligibility {
    if target.is_multi() {
        return batch::eligibility(m, target, Some(action));
    }
    let g = match Guest::of(m, target) {
        Ok(g) => g,
        Err(eligibility) => return eligibility,
    };
    match route(m, &g, action) {
        Route::Ordinary(refusal) => Eligibility::enabled_unless(refusal),
        Route::TaskAware(operation) => recovery::offer(m, &g, operation),
    }
}

fn invoke(m: &mut Model, target: &Target, action: PowerAction) {
    if target.is_multi() {
        batch::open(m, target, Some(action));
        return;
    }
    let Ok(g) = Guest::of(m, target) else { return };
    match route(m, &g, action) {
        Route::Ordinary(_) => single::request(m, &g.object(), action),
        Route::TaskAware(operation) => open_recovery(m, &g, operation),
    }
}

fn open_recovery(m: &mut Model, g: &Guest, operation: VmRecoveryOperation) {
    let panel = recovery::capture(m, g, Review::Action(operation));
    recovery::open(m, panel);
}

fn can_boot_recovery(m: &Model, target: &Target) -> Eligibility {
    let g = match Guest::of(m, target) {
        Ok(g) => g,
        Err(eligibility) => return eligibility,
    };
    match recovery::boot_operation(&g.inventory, g.id) {
        Ok(operation) => recovery::offer(m, &g, operation),
        Err(reason) => Eligibility::disabled(reason),
    }
}

fn boot_recovery(m: &mut Model, target: &Target) {
    let Ok(g) = Guest::of(m, target) else { return };
    if let Ok(operation) = recovery::boot_operation(&g.inventory, g.id) {
        open_recovery(m, &g, operation);
    }
}

/// The first boot order that can be restored for the VM. Each one is also
/// offered under its own operation record ([`super::operation_controls`]).
fn can_restore_boot(m: &Model, target: &Target) -> Eligibility {
    let g = match Guest::of(m, target) {
        Ok(g) => g,
        Err(eligibility) => return eligibility,
    };
    match recovery::restorations(m, &g).into_iter().next() {
        Some(operation) => recovery::offer(m, &g, operation),
        None => Eligibility::Hidden,
    }
}

fn restore_boot(m: &mut Model, target: &Target) {
    let Ok(g) = Guest::of(m, target) else { return };
    if let Some(operation) = recovery::restorations(m, &g).into_iter().next() {
        open_recovery(m, &g, operation);
    }
}

/// Rune showed "Start on host…" for a halted VM and "Resume on host…" for
/// a suspended one, never in the demo, and disabled it unless the snapshot
/// was current.
fn can_place(m: &Model, target: &Target, state: &PowerState) -> Eligibility {
    let g = match Guest::of(m, target) {
        Ok(g) => g,
        Err(eligibility) => return eligibility,
    };
    if synthetic(m, g.pool) || g.vm().power_state != *state {
        return Eligibility::Hidden;
    }
    Eligibility::enabled_if(
        m.snapshot_for(g.pool).is_some_and(|s| s.is_current()),
        NOT_CURRENT,
    )
}

fn place_on_server(m: &mut Model, target: &Target) {
    if let Some(object) = target.single_of(ObjectKind::Vm) {
        placement::open(m, object);
    }
}

/// The chooser of the batch is for several VMs: one VM has the nine
/// commands themselves.
fn can_power_selection(m: &Model, target: &Target) -> Eligibility {
    if !target.is_multi() {
        return Eligibility::Hidden;
    }
    batch::eligibility(m, target, None)
}

/// The host checks are kept by the kernel for the scope session
/// (`Model::diagnose_actions`), so the command is for a VM of that session.
/// Rune hid the button in the demo and disabled it unless the snapshot was
/// current.
fn can_check(m: &Model, target: &Target) -> Eligibility {
    let g = match Guest::of(m, target) {
        Ok(g) => g,
        Err(eligibility) => return eligibility,
    };
    let scope = m.workspace.selected.is_some()
        && session::session_for_pool(m, g.pool) == m.workspace.selected;
    if synthetic(m, g.pool) || !scope {
        return Eligibility::Hidden;
    }
    Eligibility::enabled_if(
        m.snapshot.as_ref().is_some_and(|s| s.is_current()),
        NOT_CURRENT,
    )
}

fn check(m: &mut Model, target: &Target) {
    if let Ok(g) = Guest::of(m, target) {
        m.diagnose_actions(g.id);
    }
}
