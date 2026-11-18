//! The power batch: one action for up to 32 VMs of one pool, reviewed and
//! submitted as one `ResourceCommand::PowerBatch`.
//!
//! Carried over from the Rune `bulk_power.rs`. The selection is the
//! command's target (the grid selection, or several objects of the tree)
//! instead of the visible selected rows of the Guests table; the pool is
//! the target's pool instead of the scope session's.

use xenadmin_core::{
    BatchVmPower, Error, ObservedResource, OperationId, PermissionDecision, PoolId, PowerAction,
    ResourceCommand, ResourceTarget, Vm, VmPowerBatch,
};

use super::{DEMO, synthetic};
use crate::prelude::*;

/// The refusal of a selection of no VM or of more than 32. Nothing is ever
/// cut down to the limit silently.
pub const LIMIT: &str = "Select between 1 and 32 visible VMs for a power batch";
/// A selection across pools: the batch is one command to one pool. (New
/// text: the Rune table showed one pool at a time.)
pub const ONE_POOL: &str = "Select VMs of one pool for a power batch";
/// What the dialog says while its pool observation is not the current one.
pub const STALE: &str =
    "Connection changed or inventory is stale. Reopen the selection after reconnecting.";
/// A VM that is not what was captured.
pub const CHANGED: &str = "VM identity or power state changed; reopen the selection";
/// Why "Run batch" waits on a force action. (New text: Rune disabled the
/// button without saying why.)
pub const ACCEPT_FIRST: &str = "Accept the risk for every listed VM before running the batch.";
pub const SEQUENCE: &str = "VMs are processed one at a time. A failed or uncertain result stops the remaining batch. Completed actions are kept; there is no automatic rollback. Other changes in this pool are blocked until this workflow is resolved.";
pub const STOPPING: &str = "Stopping the batch waits for the current server task and stops remaining VMs. Recovery inspects recorded outcomes and never repeats power actions.";
pub const FORCE_RISK: &str = "Force actions can lose unsaved guest data on every listed VM.";

pub struct Editor {
    pub pool: PoolId,
    pub pool_name: String,
    /// The pool as observed when the selection was captured: the token the
    /// batch is submitted with.
    pub observed: Option<ObservedResource>,
    /// The VMs as they were when captured, in the order of the selection.
    pub vms: Vec<Vm>,
    pub action: PowerAction,
    pub review: Option<VmPowerBatch>,
    pub force_ack: bool,
    pub error: Option<String>,
}

fn edit(m: &mut Model) -> Option<&mut Editor> {
    m.areas.power.batch.as_mut()
}

/// The guests of a target in its pool's inventory, in the target's order:
/// templates and control domains are left out, as the Rune selection left
/// them out. `Err` when the target is not VMs of one pool.
fn selection(m: &Model, target: &Target) -> Result<(PoolId, Vec<Vm>), Eligibility> {
    if target.is_empty() || !target.all_of(ObjectKind::Vm) {
        return Err(Eligibility::Hidden);
    }
    let pool = target
        .pool()
        .ok_or_else(|| Eligibility::disabled(ONE_POOL))?;
    let inventory = m
        .inventory_for(pool)
        .ok_or_else(|| Eligibility::disabled(LIMIT))?;
    let vms = target
        .vms()
        .into_iter()
        .filter_map(|id| inventory.vms.get(&id))
        .filter(|vm| !vm.is_template && !vm.is_control_domain)
        .cloned()
        .collect();
    Ok((pool, vms))
}

/// The editor a target would open: the Rune `open` without its side
/// effects. `Err` is what the command is instead (hidden, or disabled with
/// [`LIMIT`] or [`ONE_POOL`]). The default action is the first every VM is
/// available for, else Start.
pub(crate) fn capture(m: &Model, target: &Target) -> Result<Editor, Eligibility> {
    let (pool, vms) = selection(m, target)?;
    let inventory = m
        .inventory_for(pool)
        .ok_or_else(|| Eligibility::disabled(LIMIT))?;
    if vms.is_empty() || vms.len() > 32 {
        return Err(Eligibility::disabled(LIMIT));
    }
    let action = PowerAction::ALL
        .into_iter()
        .find(|action| {
            vms.iter().all(|v| {
                inventory
                    .details
                    .vms
                    .get(&v.id)
                    .is_some_and(|d| d.power_availability(*action, true).is_available())
            })
        })
        .unwrap_or(PowerAction::Start);
    Ok(Editor {
        pool,
        pool_name: inventory.pool_name.clone(),
        observed: m
            .snapshot_for(pool)
            .and_then(|snapshot| snapshot.observe_resource(ResourceTarget::Pool(pool)).ok()),
        vms,
        action,
        review: None,
        force_ack: false,
        error: None,
    })
}

/// Whether a command on several VMs can run: with an action, whether the
/// batch could be reviewed for it now.
pub(crate) fn eligibility(m: &Model, target: &Target, action: Option<PowerAction>) -> Eligibility {
    let editor = match capture(m, target) {
        Ok(editor) => editor,
        Err(eligibility) => return eligibility,
    };
    Eligibility::enabled_unless(action.and_then(|action| summary(&unavailable(m, &editor, action))))
}

/// Opens the batch for a target: on the choice of the action, or, with the
/// action a power command stands for, on its review. A refusal is in
/// `m.error` and nothing opens (the Rune `open`).
pub(crate) fn open(m: &mut Model, target: &Target, action: Option<PowerAction>) {
    match capture(m, target) {
        Ok(mut editor) => {
            if let Some(action) = action {
                editor.action = action;
            }
            m.areas.power.batch = Some(editor);
            if action.is_some() {
                review(m);
            }
        }
        Err(Eligibility::Disabled(reason)) => m.error = Some(reason),
        Err(_) => {}
    }
}

/// The pool is still the one observed and can be changed.
pub(crate) fn current(m: &Model, e: &Editor) -> bool {
    !synthetic(m, e.pool)
        && m.can_mutate_pool(e.pool)
        && e.observed.is_some()
        && m.snapshot_for(e.pool)
            .and_then(|snapshot| snapshot.observe_resource(ResourceTarget::Pool(e.pool)).ok())
            == e.observed
}

/// For each captured VM that `action` cannot be run on, "name: reason". The
/// first that applies: a known permission denial; the VM gone, turned into
/// a template or control domain, or in another power state than captured;
/// what its details say of the action; no details. One such VM blocks the
/// action for the batch: none is dropped.
pub(crate) fn unavailable(m: &Model, e: &Editor, action: PowerAction) -> Vec<String> {
    let inventory = m.inventory_for(e.pool);
    let inventory = inventory.as_deref();
    let denied = m
        .snapshot_for(e.pool)
        .is_some_and(|snapshot| snapshot.power_permission(action).is_denied());
    let live = synthetic(m, e.pool) || current(m, e);
    e.vms
        .iter()
        .filter_map(|vm| {
            let now = inventory.and_then(|i| i.vms.get(&vm.id));
            let explanation = if denied {
                Some(PermissionDecision::Denied.explanation().into())
            } else if now.is_none_or(|v| {
                v.is_template || v.is_control_domain || v.power_state != vm.power_state
            }) {
                Some(CHANGED.into())
            } else {
                inventory
                    .and_then(|i| i.details.vms.get(&vm.id))
                    .map(|d| d.power_availability(action, live))
                    .filter(|a| !a.is_available())
                    .map(|a| a.explanation())
                    .or_else(|| {
                        inventory
                            .filter(|i| !i.details.vms.contains_key(&vm.id))
                            .map(|_| "Action availability is unknown".into())
                    })
            };
            explanation.map(|reason| format!("{}: {reason}", vm.name))
        })
        .collect()
}

/// The line under an action that some VM is unavailable for.
pub(crate) fn summary(reasons: &[String]) -> Option<String> {
    reasons
        .first()
        .map(|first| format!("Unavailable for {} VM(s): {first}", reasons.len()))
}

pub(crate) fn choose(m: &mut Model, action: PowerAction) {
    if let Some(e) = edit(m) {
        e.action = action;
        e.error = None;
    }
}

pub(crate) fn review(m: &mut Model) {
    let Some(e) = m.areas.power.batch.as_ref() else {
        return;
    };
    let problems = unavailable(m, e, e.action);
    let Some(e) = edit(m) else { return };
    if !problems.is_empty() {
        e.error = Some(problems.join("\n"));
        return;
    }
    e.review = Some(VmPowerBatch {
        action: e.action,
        vms: e
            .vms
            .iter()
            .map(|vm| BatchVmPower {
                vm: vm.id,
                expected_state: vm.power_state.clone(),
            })
            .collect(),
    });
    e.force_ack = false;
}

pub(crate) fn back(m: &mut Model) {
    if let Some(e) = edit(m) {
        e.review = None;
    }
}

pub(crate) fn toggle_force_ack(m: &mut Model) {
    if let Some(e) = edit(m) {
        e.force_ack = !e.force_ack;
    }
}

pub(crate) fn close(m: &mut Model) {
    m.areas.power.batch = None;
}

/// Why "Run batch" is disabled, the first that applies; `None` when the
/// reviewed batch can be submitted. The demo first: there the pool has no
/// observation at all.
pub(crate) fn blocked(m: &Model, e: &Editor) -> Option<String> {
    if synthetic(m, e.pool) {
        return Some(DEMO.into());
    }
    if !current(m, e) {
        return Some(STALE.into());
    }
    if let Some(reason) = summary(&unavailable(m, e, e.action)) {
        return Some(reason);
    }
    (e.action.is_force() && !e.force_ack).then(|| ACCEPT_FIRST.into())
}

/// Every refusal is the same `Stale` in the editor's error line, as in
/// Rune: the review is not current, a VM became unavailable, or a force
/// action was not accepted.
pub(crate) fn submit(m: &mut Model) {
    let result = (|| {
        let e = m.areas.power.batch.as_ref().ok_or(Error::Stale)?;
        let selection = e.review.clone().ok_or(Error::Stale)?;
        if !current(m, e)
            || !unavailable(m, e, selection.action).is_empty()
            || selection.action.is_force() && !e.force_ack
        {
            return Err(Error::Stale);
        }
        m.connection_for(e.pool)
            .ok_or(Error::Closed)?
            .submit_resource(
                OperationId::new(),
                ResourceCommand::PowerBatch {
                    pool: e.observed.ok_or(Error::Stale)?,
                    selection,
                },
            )
    })();
    match result {
        Ok(operation) => {
            m.track_resource(operation);
            m.areas.power.batch = None;
        }
        Err(error) => {
            if let Some(e) = edit(m) {
                e.error = Some(error.to_string());
            }
        }
    }
}
