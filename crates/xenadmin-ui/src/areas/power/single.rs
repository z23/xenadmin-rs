//! Power for one VM: why an action is or is not available, the request,
//! the confirmation, and the controls of an operation record.
//!
//! Carried over from the Rune `main.rs`: `State::request_power`,
//! `State::submit_power`, the reason under each power button of `details`,
//! the controls under each recent operation, and what the `force_action`
//! dialog did. Read against the pool of the VM instead of the scope
//! session. The Rune `unknown_review` dialog is the `events` area's
//! acknowledgement (`unknown-outcome`), the one dialog for it:
//! [`review_unknown`] opens it.

use xenadmin_core::{
    ActionAvailability, Error, ObservedVm, OperationAction, OperationControl, OperationId,
    OperationRecord, OperationState, PermissionDecision, PoolId, PowerAction, SnapshotOperation,
    VmCommand, VmId,
};

use super::{DEMO, Guest, NOT_CURRENT, READ_ONLY, not_writable, submit_vm, synthetic};
use crate::prelude::*;

/// A host check that passed for an action the inventory does not allow.
pub const ADVISORY: &str = "The host check passed, but the inventory does not currently allow this action. The check is advisory; the action stays disabled until availability updates.";
/// A host check that has not answered yet.
pub const CHECKING: &str = "Checking with the host…";
/// A VM without a details record (the text of the Rune power batch).
pub const UNKNOWN: &str = "Action availability is unknown";
/// The caption `power-actions-unavailable` of the Rune details pane.
pub const NO_ACTIONS: &str = "No power actions are available in the received VM data. Check action availability below for the host's current reason. This check does not remove restrictions.";
/// The consequence line of a force confirmation.
pub const FORCE: &str = "This interrupts the guest immediately and can lose unsaved data. A clean shutdown is safer when the guest responds.";
/// The consequence line of a routine confirmation.
pub const ROUTINE: &str = "The selected power action will be sent to this VM. Current host permissions and power state are checked again before submission.";

/// The pending confirmation of one power action: the observation captured
/// when it was asked for, which is what is submitted (Rune `force_action`).
pub struct Confirm {
    pub pool: PoolId,
    pub vm: VmId,
    pub target: ObservedVm,
    pub action: PowerAction,
    pub name: String,
}

/// What decides whether one action can be asked of one VM, and what is said
/// when it cannot. Separate from the model so the rule can be tested with a
/// permission the fixtures cannot produce.
pub(crate) struct Facts {
    pub permission: Option<PermissionDecision>,
    pub synthetic: bool,
    pub read_only: bool,
    pub can_mutate: bool,
    /// The inventory lists the action among the VM's allowed ones.
    pub allowed: bool,
    pub current: bool,
    /// The host's answer to "Check action availability" for this action:
    /// `None` without a current check of this VM, `Some(None)` while the
    /// answer is pending.
    pub diagnostic: Option<Option<Result<ActionAvailability, String>>>,
    /// What the inventory's details say; `None` without a details record.
    pub availability: Option<ActionAvailability>,
}

impl Facts {
    pub fn denied(&self) -> bool {
        self.permission.is_some_and(|p| p.is_denied())
    }

    /// The rule of the Rune power button: a writable current connection,
    /// an action the inventory allows, and no known denial. An unknown
    /// permission never disables; a host check never enables.
    pub fn enabled(&self) -> bool {
        self.can_mutate && self.allowed && !self.denied()
    }

    /// The caption Rune put under the power button, in its precedence: a
    /// known denial, then the host check, then (disabled and not the demo)
    /// what the inventory says.
    pub fn caption(&self) -> Option<String> {
        if self.denied() {
            return Some(PermissionDecision::Denied.explanation().into());
        }
        match &self.diagnostic {
            Some(Some(Ok(ActionAvailability::Available))) if !self.allowed => Some(ADVISORY.into()),
            Some(Some(Ok(availability))) => Some(availability.explanation()),
            Some(Some(Err(error))) => Some(error.clone()),
            Some(None) => Some(CHECKING.into()),
            None if !self.enabled() && !self.synthetic => {
                self.availability.as_ref().map(|a| a.explanation())
            }
            None => None,
        }
    }

    /// Why the command is disabled; `None` when it is enabled. The caption
    /// where Rune had one; where it had only the captions above the buttons
    /// (the demo, a read-only connection, a stale one) that caption.
    pub fn refusal(&self) -> Option<String> {
        if self.enabled() {
            return None;
        }
        if self.denied() {
            return self.caption();
        }
        if self.synthetic {
            return Some(DEMO.into());
        }
        if self.read_only {
            return Some(READ_ONLY.into());
        }
        Some(self.caption().unwrap_or_else(|| {
            if self.current {
                UNKNOWN.into()
            } else {
                NOT_CURRENT.into()
            }
        }))
    }
}

pub(crate) fn facts(m: &Model, g: &Guest, action: PowerAction) -> Facts {
    let snapshot = m.snapshot_for(g.pool);
    let current = snapshot.as_ref().is_some_and(|s| s.is_current());
    // The checks belong to the scope session (`Model::diagnose_actions`).
    let diagnostic = m
        .diagnostics
        .as_ref()
        .filter(|d| {
            d.context.pool == g.pool
                && d.context.vm == g.id
                && d.is_current(m.snapshot.as_deref())
        })
        .and_then(|d| d.handles.iter().find(|(a, _)| *a == action))
        .map(|(_, handle)| handle.latest());
    Facts {
        permission: snapshot.as_ref().map(|s| s.power_permission(action)),
        synthetic: synthetic(m, g.pool),
        read_only: m.connection_for(g.pool).is_some_and(|c| c.is_read_only()),
        can_mutate: m.can_mutate_pool(g.pool),
        allowed: g.vm().allowed_actions.contains(&action),
        current,
        diagnostic,
        availability: g
            .inventory
            .details
            .vms
            .get(&g.id)
            .map(|d| d.power_availability(action, current)),
    }
}

/// Why `action` cannot be asked of the guest now; `None` when it can.
pub(crate) fn refusal(m: &Model, g: &Guest, action: PowerAction) -> Option<String> {
    facts(m, g, action).refusal()
}

/// The Rune `State::request_power`, for the VM a command was invoked on.
/// In its order: a known denial refuses with the fixed text before anything
/// else; the observation is captured now (failing as `Stale`); a reviewed
/// action waits for its confirmation, any other is submitted.
pub(crate) fn request(m: &mut Model, object: &ObjectRef, action: PowerAction) {
    let (Some(pool), Some(vm)) = (object.pool, object.vm_id()) else {
        return;
    };
    let snapshot = m.snapshot_for(pool);
    if snapshot
        .as_ref()
        .is_some_and(|s| s.power_permission(action).is_denied())
    {
        m.error = Some(PermissionDecision::Denied.explanation().into());
        return;
    }
    let Some(target) = snapshot.and_then(|s| s.observe_vm(vm).ok()) else {
        m.error = Some(Error::Stale.to_string());
        return;
    };
    if m.preferences.options.reviews(action) {
        m.areas.power.confirm = Some(Confirm {
            pool,
            vm,
            target,
            action,
            name: m.name_of(object).unwrap_or_default(),
        });
    } else {
        submit(m, pool, target, action);
    }
}

/// The Rune `State::submit_power`: the confirmation closes whether or not
/// the submission was accepted; a refusal is in `m.error`.
fn submit(m: &mut Model, pool: PoolId, vm: ObservedVm, action: PowerAction) {
    submit_vm(m, pool, VmCommand::Power { vm, action });
    m.areas.power.confirm = None;
}

/// Why the open confirmation cannot be confirmed: its observation is no
/// longer the current one, or the permission is now known to be denied.
pub(crate) fn confirm_blocked(m: &Model, c: &Confirm) -> Option<String> {
    let snapshot = m.snapshot_for(c.pool);
    if snapshot.as_ref().and_then(|s| s.observe_vm(c.vm).ok()) != Some(c.target) {
        return Some(Error::Stale.to_string());
    }
    snapshot
        .filter(|s| s.power_permission(c.action).is_denied())
        .map(|_| PermissionDecision::Denied.explanation().into())
}

/// The confirm button: submits the observation captured at the request.
pub(crate) fn confirm(m: &mut Model) {
    let Some(c) = &m.areas.power.confirm else {
        return;
    };
    if confirm_blocked(m, c).is_some() {
        return;
    }
    let (pool, target, action) = (c.pool, c.target, c.action);
    submit(m, pool, target, action);
}

pub(crate) fn cancel_confirm(m: &mut Model) {
    m.areas.power.confirm = None;
}

// -- Operation records ---------------------------------------------------------

/// `Model::control_operation` through the connection of the record's pool.
/// The result is in `m.error`, which a success clears. A recheck is a
/// control message on the existing operation: it never replays a write.
pub fn control(m: &mut Model, id: OperationId, control: OperationControl) {
    let pool = m.operation_rows.iter().find(|r| r.id == id).map(|r| r.pool);
    let connection = match pool {
        Some(pool) => m.connection_for(pool),
        None => m.connection.clone(),
    };
    m.error = connection
        .ok_or(Error::Closed)
        .and_then(|connection| connection.control_operation(id, control))
        .err()
        .map(|error| error.to_string());
}

/// "Request cancellation" or "Stop remaining steps": asks this client's
/// operation to stop. It does not cancel a host task by itself, and nothing
/// is marked cancelled here: the record says what happened.
pub fn request_cancellation(m: &mut Model, id: OperationId) {
    control(m, id, OperationControl::RequestCancellation);
}

/// "Recheck outcome": inspects what the host recorded for the operation.
pub fn recheck(m: &mut Model, id: OperationId) {
    control(m, id, OperationControl::Recheck);
}

/// "Review unknown outcome": opens the acknowledgement for the operation,
/// the dialog `unknown-outcome` of the `events` area (the Rune
/// `unknown_review`). It names the VM as the inventory does, or by UUID
/// when it is gone. A record that cannot be acknowledged (a child of a
/// batch, an outcome that is known by now) leaves the reason in `m.error`.
pub fn review_unknown(m: &mut Model, id: OperationId) {
    let Some(pool) = m.operation_rows.iter().find(|r| r.id == id).map(|r| r.pool) else {
        return;
    };
    crate::areas::events::acknowledge(
        m,
        crate::areas::events::OperationKey {
            pool,
            id,
            family: crate::areas::events::Family::Vm,
        },
    );
}

/// Whether cancelling stops the remaining steps of a workflow, which is how
/// the button is worded for it.
fn stops_steps(record: &OperationRecord) -> bool {
    matches!(
        record.action,
        OperationAction::Lifecycle(_)
            | OperationAction::Recovery(_)
            | OperationAction::Snapshot(SnapshotOperation::Workflow { .. })
    )
}

/// The controls of one VM operation record, as the Rune details pane had
/// them under the record, in its order. None for a child of a batch (its
/// parent is controlled in the pool's history). Keys:
/// `operation-cancel-<id>`, `operation-recheck-<id>`,
/// `operation-unknown-<id>`, and `recovery-restore-<id>` (the Rune key)
/// under a finished recovery boot whose saved boot order can be restored.
pub fn operation_controls(m: &Model, record: &OperationRecord) -> Vec<Button> {
    let id = record.id;
    let mut controls = Vec::new();
    if record.batch_parent.is_none()
        && record.can_cancel
        && !record.cancel_requested
        && !record.state.terminal()
    {
        controls.push(button(
            format!("operation-cancel-{id}"),
            if stops_steps(record) {
                "Stop remaining steps"
            } else {
                "Request cancellation"
            },
            move |m| request_cancellation(m, id),
        ));
    }
    if record.batch_parent.is_none() && record.state == OperationState::OutcomeUnknown {
        if record.can_recheck() {
            controls.push(button(
                format!("operation-recheck-{id}"),
                "Recheck outcome",
                move |m| recheck(m, id),
            ));
        }
        controls.push(
            button(
                format!("operation-unknown-{id}"),
                "Review unknown outcome",
                move |m| review_unknown(m, id),
            )
            .warning(),
        );
    }
    controls.extend(super::recovery::restore_button(m, record));
    controls
}

// -- The power section of a VM's General tab ----------------------------------

/// The group "Power" for the General tab of a VM: what the Rune details
/// pane said above and under its power buttons. `None` for anything but a
/// VM (a template and a control domain had no power block).
///
/// Keys: `power-availability` (the group), `power-read-only`,
/// `power-actions-unavailable`, `power-actions-stale`,
/// `session-permission-explanation`, `power-reason-<Action>` for each of
/// the nine actions, `power-vm-gone`.
pub fn availability_section(m: &Model, object: &ObjectRef) -> Option<Group> {
    let section = |blocks| Some(group("Power", blocks).key("power-availability"));
    let g = match Guest::of_object(m, object) {
        Ok(g) => g,
        Err(Eligibility::Disabled(reason)) => {
            return section(blocks![caption(reason).key("power-vm-gone")]);
        }
        Err(_) => return None,
    };
    let demo = synthetic(m, g.pool);
    let snapshot = m.snapshot_for(g.pool);
    let current = snapshot.as_ref().is_some_and(|s| s.is_current());
    let unknown = snapshot
        .as_ref()
        .map(|s| s.power_permission(PowerAction::Start))
        .filter(|p| matches!(p, PermissionDecision::Unknown(_)));
    let reasons: Vec<Property> = PowerAction::ALL
        .into_iter()
        .map(|action| {
            let facts = facts(m, &g, action);
            let value = facts
                .caption()
                .or_else(|| facts.refusal())
                .unwrap_or_else(|| ActionAvailability::Available.explanation());
            property(action.label(), value).key(format!("power-reason-{action:?}"))
        })
        .collect();
    section(blocks![
        (not_writable(m, g.pool) == READ_ONLY).then(|| caption(READ_ONLY).key("power-read-only")),
        (!demo && g.vm().allowed_actions.is_empty())
            .then(|| caption(NO_ACTIONS).key("power-actions-unavailable")),
        (!demo && !current).then(|| caption(NOT_CURRENT).key("power-actions-stale")),
        unknown.map(permissions::notice),
        properties(reasons),
    ])
}
