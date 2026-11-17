//! The "Operations" section of a VM's General tab: the recent operation
//! records of the VM, as the Rune details pane listed them.
//!
//! The section is information only. Rune put "Request cancellation",
//! "Recheck outcome", "Review unknown outcome" and "Explain outcome…" under
//! each record; those are commands of the `power` and `events` areas, which
//! act on the same records on the Events tab of the Notifications view.

use xenadmin_core::{OperationRecord, OperationState, PoolId, VmId};

use crate::prelude::*;

/// How many records the section shows, as in Rune.
const SHOWN: usize = 10;

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

/// The records of one VM in its pool: those that act on it and those that
/// created it; the unfinished ones first, then the newest; ten at most.
pub(super) fn vm_operations(m: &Model, pool: PoolId, vm: VmId) -> Group {
    let mut recent: Vec<&OperationRecord> = m
        .operation_rows
        .iter()
        .filter(|record| (record.vm == vm || record.created_vm == Some(vm)) && record.pool == pool)
        .collect();
    recent.sort_by_key(|record| {
        (
            record.state.terminal(),
            std::cmp::Reverse(record.created_at_ms),
        )
    });
    let records: Vec<Outcome> = recent
        .into_iter()
        .take(SHOWN)
        .map(|record| {
            let mut lines = vec![record.detail.clone()];
            lines.extend(
                record
                    .steps
                    .iter()
                    .map(|step| format!("{} • {:?}: {}", step.label, step.state, step.detail)),
            );
            lines.extend(record.local_error.clone());
            if let Some(parent) = record.batch_parent {
                lines.push(format!(
                    "Managed by batch {parent}. Use the batch controls on the Events tab of the \
                     Notifications view."
                ));
            }
            outcome(
                format!("vm-operation-{}", record.id),
                outcome_state(record),
                format!("{} • {:?}", record.action.label(), record.state),
            )
            .lines(lines)
        })
        .collect();
    group(
        "Operations",
        blocks![
            records,
            caption(if m.demo {
                "Demo mode cannot change infrastructure."
            } else {
                "Actions and property changes are tracked in the operation history, including \
                 partial and uncertain outcomes."
            })
            .key("general-operations-note"),
        ],
    )
    .key("general-operations")
}
