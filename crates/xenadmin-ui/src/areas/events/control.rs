//! What is done to one operation record, whichever family it is of: recheck
//! its outcome, acknowledge an unknown outcome, ask for cancellation.
//!
//! Three Rune places did this, each for a part: the Activity dialog
//! (`activity::recheck`, `can_recheck`), the details pane and its
//! "Acknowledge unknown outcome?" dialog (`main.rs`, VM operations), and the
//! pool history with `Kind::Acknowledge` (`resource_admin.rs`, resource
//! operations). Here an operation is named by its [`HiddenKey`] (pool, id,
//! family) and every caller gets the same rules:
//!
//! - A recheck only reads: `control_operation(id, Recheck)`,
//!   `control_resource(id, Recheck)` or the cross-pool recheck. It never
//!   replays a write.
//! - A batch child is controlled through its batch.
//! - A resource operation is controlled over the connection
//!   `session::resource_control_connection` picks: the record's recovery
//!   pool for a recheck, any pool it applies to otherwise. A VM operation is
//!   controlled over the connection of its pool.
//! - Acknowledging is reviewed first. The review is captured when it opens
//!   and cannot be confirmed once the record or the connection changed.
//!
//! Differences from Rune:
//!
//! - Rune acknowledged a resource operation on the selected connection while
//!   its button followed `resource_control_connection`; both use the picked
//!   connection here (map hazard 47).
//! - A failed VM acknowledgement keeps its dialog open with the error. Rune
//!   closed the dialog and left the error in the shared error line.
//! - The resource acknowledgement opens on its review: the Rune form before
//!   it was empty.

use xenadmin_core::{
    Connection, Error, ObservedResource, OperationAction, OperationControl, OperationId,
    OperationRecord, OperationState, PoolId, ResourceAction, ResourceOperationRecord,
    ResourceTarget, SnapshotOperation,
};

use super::activity::{DEMO, synthetic};
use super::failure_help::FailureHelp;
use super::hidden::{Family, HiddenKey};
use crate::command::Eligibility;
use crate::model::Model;
use crate::session;

/// The words of the Rune transfer dialogs for a pool without a writable
/// current connection. Rune disabled these buttons without saying why.
pub const NOT_WRITABLE: &str = "This connection is read-only, disconnected or stale.";
pub const RECHECK_TRANSFER: &str =
    "Reconnect both source and destination pools to recheck this transfer.";
pub const RECHECKING: &str = "Checking existing evidence; no mutation is replayed.";
pub const RECHECKING_BOTH: &str = "Checking both pools; no transfer is replayed.";
pub const ACK_VM: &str = "The earlier operation may still be running. Inspect the host and stop other client instances handling it before continuing. Acknowledging releases this client's VM block; it does not cancel the host task or establish its result.";
pub const ACK_BATCH: &str = "Acknowledgement releases the batch and its active unknown child together. It does not prove completion, cancel host work or execute remaining targets.";
pub const ACK_POOL: &str = "This releases the pool's write guard without establishing the server outcome. Inspect the affected resources and any host task before continuing.";
pub const ACK_GONE: &str = "The operation is no longer available for this pool";
pub const ACK_STALE: &str =
    "The connection or resource changed. Close and reopen this review before applying it.";

/// The reason a batch child gives for every control. The Rune text
/// said "in pool history" (AREAS.md decision 10).
pub fn batch_reason(parent: OperationId) -> String {
    format!("Managed by batch {parent}. Use the batch controls on the Events tab of the Notifications view.")
}

pub fn vm_record(m: &Model, key: HiddenKey) -> Option<&OperationRecord> {
    m.operation_rows
        .iter()
        .find(|r| r.id == key.id && r.pool == key.pool)
}

/// Resource records are merged by id alone.
pub fn resource_record(m: &Model, key: HiddenKey) -> Option<&ResourceOperationRecord> {
    m.resource_rows.iter().find(|r| r.id == key.id)
}

/// The explanation of a record's outcome, when it has one.
pub fn help(m: &Model, key: HiddenKey) -> Option<FailureHelp> {
    match key.family {
        Family::Vm => FailureHelp::vm(vm_record(m, key)?),
        Family::Resource => FailureHelp::resource(resource_record(m, key)?),
    }
}

/// Why a pool cannot be acted on: its inventory is synthetic, or no
/// writable current connection reaches it.
fn unreachable(m: &Model, pool: PoolId) -> String {
    if synthetic(m, pool) { DEMO } else { NOT_WRITABLE }.into()
}

// -- Recheck ------------------------------------------------------------------

/// The Rune `activity::can_recheck` with its reasons: never for a finished
/// record; a resource record needs its control connection, a VM record a
/// writable current connection to its pool.
pub fn recheck_eligibility(m: &Model, key: HiddenKey) -> Eligibility {
    match key.family {
        Family::Vm => {
            let Some(record) = vm_record(m, key) else {
                return Eligibility::Hidden;
            };
            if record.state.terminal() {
                Eligibility::Hidden
            } else if let Some(parent) = record.batch_parent {
                Eligibility::disabled(batch_reason(parent))
            } else if !m.can_mutate_pool(key.pool) {
                Eligibility::disabled(unreachable(m, key.pool))
            } else {
                Eligibility::Enabled
            }
        }
        Family::Resource => {
            let Some(record) = resource_record(m, key) else {
                return Eligibility::Hidden;
            };
            if record.state.terminal() {
                Eligibility::Hidden
            } else if let Some(parent) = record.batch_parent {
                Eligibility::disabled(batch_reason(parent))
            } else if session::resource_control_connection(m, key.id, OperationControl::Recheck)
                .is_some()
            {
                Eligibility::Enabled
            } else if record.action.spans_pools() {
                Eligibility::disabled(RECHECK_TRANSFER)
            } else if record.recovery_pool() != record.pool {
                Eligibility::disabled(format!(
                    "Reconnect destination pool {} to recheck this join.",
                    record.recovery_pool()
                ))
            } else {
                Eligibility::disabled(unreachable(m, record.pool))
            }
        }
    }
}

/// The writable connection that observes a pool: the Rune
/// `cross_pool_vm::connection_for_pool`.
fn connection_observing(m: &Model, pool: PoolId) -> Option<Connection> {
    m.workspace
        .active
        .values()
        .chain(m.connection.iter())
        .find(|c| {
            !c.is_read_only()
                && c.latest()
                    .observe_resource(ResourceTarget::Pool(pool))
                    .is_ok()
        })
        .cloned()
}

/// The Rune `cross_pool_vm::recheck`: both pools are read again and the
/// handle of that check is followed like any resource operation. It belongs
/// to the `cross_pool` area, which is not ported yet; when it is, this is
/// the one copy to remove.
fn recheck_spanning(m: &mut Model, id: OperationId) -> Result<(), Error> {
    let r = m
        .resource_rows
        .iter()
        .find(|r| r.id == id)
        .ok_or(Error::OperationConflict)?;
    let peer = r
        .peer_pool()
        .filter(|_| r.action.spans_pools())
        .ok_or(Error::InvalidMigration)?;
    let batch = matches!(r.action, ResourceAction::VmWorkflowBatch(_));
    let source = connection_observing(m, r.pool).ok_or(Error::Closed)?;
    let dest = connection_observing(m, peer).ok_or(Error::Closed)?;
    let handle = if batch {
        source.recheck_cross_pool_batch(id, &dest)?
    } else {
        source.recheck_cross_pool_vm(id, &dest)?
    };
    m.track_resource(handle);
    Ok(())
}

/// Reads the evidence of an unresolved operation again. `Ok` is what to
/// tell the user; nothing is replayed either way.
pub fn recheck(m: &mut Model, key: HiddenKey) -> Result<&'static str, String> {
    match recheck_eligibility(m, key) {
        Eligibility::Enabled => {}
        Eligibility::Disabled(reason) => return Err(reason),
        Eligibility::Hidden => return Err(Error::Stale.to_string()),
    }
    if key.family == Family::Resource
        && m.resource_rows
            .iter()
            .any(|r| r.id == key.id && r.action.spans_pools())
    {
        return recheck_spanning(m, key.id)
            .map(|()| RECHECKING_BOTH)
            .map_err(|e| e.to_string());
    }
    let control = OperationControl::Recheck;
    match key.family {
        Family::Vm => m.try_control_operation_for(key.pool, key.id, control),
        Family::Resource => session::resource_control_connection(m, key.id, control)
            .ok_or(Error::Closed)
            .and_then(|c| c.control_resource(key.id, control)),
    }
    .map(|()| RECHECKING)
    .map_err(|e| e.to_string())
}

// -- Request cancellation -------------------------------------------------------

/// The Rune rules of the two "Request cancellation" buttons: a VM operation
/// that can be cancelled, was not asked to yet and is unfinished; a
/// resource operation that can be cancelled, is unfinished and not unknown
/// (an unknown one is rechecked or acknowledged), over its control
/// connection.
pub fn cancel_eligibility(m: &Model, key: HiddenKey) -> Eligibility {
    match key.family {
        Family::Vm => {
            let Some(record) = vm_record(m, key) else {
                return Eligibility::Hidden;
            };
            if !record.can_cancel || record.cancel_requested || record.state.terminal() {
                Eligibility::Hidden
            } else if let Some(parent) = record.batch_parent {
                Eligibility::disabled(batch_reason(parent))
            } else {
                Eligibility::enabled_if(!synthetic(m, key.pool), DEMO)
            }
        }
        Family::Resource => {
            let Some(record) = resource_record(m, key) else {
                return Eligibility::Hidden;
            };
            if !record.can_cancel
                || record.state.terminal()
                || record.state == OperationState::OutcomeUnknown
            {
                Eligibility::Hidden
            } else if let Some(parent) = record.batch_parent {
                Eligibility::disabled(batch_reason(parent))
            } else if session::resource_control_connection(
                m,
                key.id,
                OperationControl::RequestCancellation,
            )
            .is_some()
            {
                Eligibility::Enabled
            } else {
                Eligibility::disabled(unreachable(m, record.pool))
            }
        }
    }
}

/// What the cancellation of a record is called: it differs by what stops.
pub fn cancel_label(m: &Model, key: HiddenKey) -> &'static str {
    match key.family {
        Family::Vm => match vm_record(m, key).map(|r| &r.action) {
            Some(
                OperationAction::Lifecycle(_)
                | OperationAction::Recovery(_)
                | OperationAction::Snapshot(SnapshotOperation::Workflow { .. }),
            ) => "Stop remaining steps",
            _ => "Request cancellation",
        },
        Family::Resource => match resource_record(m, key).map(|r| &r.action) {
            Some(ResourceAction::VmWorkflowBatch(batch)) if batch.is_snapshot_export() => {
                "Finish current snapshot and stop"
            }
            Some(ResourceAction::VmWorkflowBatch(_)) => "Finish current VM and stop",
            Some(ResourceAction::Host(_) | ResourceAction::Disk(_)) => "Stop remaining steps",
            _ => "Request cancellation",
        },
    }
}

/// Asks the host to stop an operation. Local cancellation does not
/// establish server cancellation: the record says what happened.
pub fn cancel(m: &mut Model, key: HiddenKey) -> Result<(), String> {
    match cancel_eligibility(m, key) {
        Eligibility::Enabled => {}
        Eligibility::Disabled(reason) => return Err(reason),
        Eligibility::Hidden => return Err(Error::Stale.to_string()),
    }
    let control = OperationControl::RequestCancellation;
    match key.family {
        Family::Vm => m.try_control_operation_for(key.pool, key.id, control),
        Family::Resource => session::resource_control_connection(m, key.id, control)
            .ok_or(Error::Closed)
            .and_then(|c| c.control_resource(key.id, control)),
    }
    .map_err(|e| e.to_string())
}

// -- Acknowledge an unknown outcome ---------------------------------------------

/// The open acknowledgement review.
pub struct Acknowledge {
    pub key: HiddenKey,
    /// The reviewed lines, captured when the review opened.
    pub lines: Vec<String>,
    /// Resource family: the pool of the control connection and what it
    /// observed of that pool when the review opened.
    control_pool: Option<PoolId>,
    observed: Option<ObservedResource>,
    pub error: Option<String>,
}

/// A VM operation with an unknown outcome that is not a batch child (the
/// Rune "Review unknown outcome"); a resource operation with an unknown
/// outcome over its control connection (the Rune "Acknowledge unknown
/// outcome…").
pub fn acknowledge_eligibility(m: &Model, key: HiddenKey) -> Eligibility {
    match key.family {
        Family::Vm => {
            let Some(record) = vm_record(m, key) else {
                return Eligibility::Hidden;
            };
            if record.state != OperationState::OutcomeUnknown {
                Eligibility::Hidden
            } else if let Some(parent) = record.batch_parent {
                Eligibility::disabled(batch_reason(parent))
            } else {
                Eligibility::Enabled
            }
        }
        Family::Resource => {
            let Some(record) = resource_record(m, key) else {
                return Eligibility::Hidden;
            };
            if record.state != OperationState::OutcomeUnknown {
                Eligibility::Hidden
            } else if let Some(parent) = record.batch_parent {
                Eligibility::disabled(batch_reason(parent))
            } else if control_connection(m, key.id).is_some() {
                Eligibility::Enabled
            } else {
                Eligibility::disabled(unreachable(m, record.pool))
            }
        }
    }
}

fn control_connection(m: &Model, id: OperationId) -> Option<Connection> {
    session::resource_control_connection(m, id, OperationControl::AcknowledgeUnknown)
}

/// What the control connection of a resource record observes of its own
/// pool now.
fn control_observation(m: &Model, id: OperationId) -> Option<(PoolId, ObservedResource)> {
    let snapshot = control_connection(m, id)?.latest();
    let pool = snapshot.inventory()?.pool_id;
    let observed = snapshot.observe_resource(ResourceTarget::Pool(pool)).ok()?;
    Some((pool, observed))
}

/// The lines of the resource acknowledgement review (the Rune
/// `resource_admin::review` for `Kind::Acknowledge`).
pub fn resource_lines(record: &ResourceOperationRecord) -> Vec<String> {
    let mut lines = vec![format!("{} · {}", record.action.label(), record.id)];
    if !record.workflow_batch.is_empty() {
        lines.push(ACK_BATCH.into());
    }
    lines.push(if let Some(peer) = record.peer_pool() {
        format!(
            "This releases write reservations for BOTH pools: {} and {}. The server outcome remains unverified. Inspect both pools before continuing.",
            record.pool, peer
        )
    } else {
        ACK_POOL.into()
    });
    lines
}

/// The lines of the VM acknowledgement dialog (`main.rs`): the VM by name,
/// then what acknowledging does and does not do.
fn vm_lines(m: &Model, record: &OperationRecord) -> Vec<String> {
    let name = m
        .inventory_for(record.pool)
        .and_then(|inventory| inventory.vms.get(&record.vm).map(|vm| vm.name.clone()))
        .unwrap_or_else(|| record.vm.to_string());
    vec![format!("VM: {name}"), ACK_VM.into()]
}

/// Opens the review without asking whether the command would: for the
/// command, after its eligibility, and for tests of a synthetic pool, where
/// the command is disabled as it was in Rune.
pub fn open_acknowledge(m: &mut Model, key: HiddenKey) -> bool {
    let (lines, control) = match key.family {
        Family::Vm => match vm_record(m, key) {
            Some(record) => (vm_lines(m, record), None),
            None => return false,
        },
        Family::Resource => match resource_record(m, key) {
            Some(record) => (resource_lines(record), control_observation(m, key.id)),
            None => return false,
        },
    };
    m.areas.events.acknowledge = Some(Acknowledge {
        key,
        lines,
        control_pool: control.map(|(pool, _)| pool),
        observed: control.map(|(_, observed)| observed),
        error: None,
    });
    true
}

/// Whether the acknowledgement would be of a synthetic pool's record.
pub fn acknowledge_synthetic(m: &Model, a: &Acknowledge) -> bool {
    synthetic(m, a.key.pool)
}

/// Why an open acknowledgement cannot be confirmed: the record is gone or
/// resolved, or the connection it was reviewed over changed.
pub fn acknowledge_stale(m: &Model, a: &Acknowledge) -> Option<&'static str> {
    if acknowledge_synthetic(m, a) {
        return None;
    }
    match a.key.family {
        Family::Vm => match vm_record(m, a.key) {
            None => Some(ACK_GONE),
            Some(record) if record.state != OperationState::OutcomeUnknown => Some(ACK_STALE),
            Some(_) => None,
        },
        Family::Resource => match resource_record(m, a.key) {
            None => Some(ACK_GONE),
            Some(record) if record.state != OperationState::OutcomeUnknown => Some(ACK_STALE),
            Some(_) => {
                let now = control_observation(m, a.key.id);
                let same = a.observed.is_some()
                    && now.map(|(pool, _)| pool) == a.control_pool
                    && now.map(|(_, observed)| observed) == a.observed;
                (!same).then_some(ACK_STALE)
            }
        },
    }
}

/// Confirms the open acknowledgement. Every check is made again.
pub fn confirm_acknowledge(m: &mut Model) {
    let Some(a) = &m.areas.events.acknowledge else {
        return;
    };
    let key = a.key;
    let control = OperationControl::AcknowledgeUnknown;
    let result = if acknowledge_synthetic(m, a) || acknowledge_stale(m, a).is_some() {
        Err(Error::Stale)
    } else {
        match key.family {
            Family::Vm => m.try_control_operation_for(key.pool, key.id, control),
            Family::Resource => control_connection(m, key.id)
                .ok_or(Error::Closed)
                .and_then(|c| c.control_resource(key.id, control)),
        }
    };
    match result {
        Ok(()) => m.areas.events.acknowledge = None,
        Err(error) => {
            if let Some(a) = &mut m.areas.events.acknowledge {
                a.error = Some(error.to_string());
            }
        }
    }
}
