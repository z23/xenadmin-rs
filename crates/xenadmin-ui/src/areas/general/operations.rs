//! The "Operations" section of a VM's General tab: the recent operation
//! records of the VM, as the Rune details pane listed them.
//!
//! What Rune put under a record is a command of the tab's row (`power`).
//! Each command acts on the first record shown here that offers it:
//! unfinished records first, then the newest. "Restore saved boot order"
//! stays a command of the VM menu (`vm.recovery-restore-boot`).

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
/// The tab's commands use the same order.
pub(crate) fn recent(m: &Model, pool: PoolId, vm: VmId) -> Vec<&OperationRecord> {
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
    recent.truncate(SHOWN);
    recent
}

/// The records of one VM in its pool, as the section shows them.
pub(super) fn vm_operations(m: &Model, pool: PoolId, vm: VmId) -> Group {
    let records: Vec<Outcome> = recent(m, pool, vm)
        .into_iter()
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
                "Actions and property changes are tracked on the Events tab of the Notifications view, including \
                 partial and uncertain outcomes."
            })
            .key("general-operations-note"),
        ],
    )
    .key("general-operations")
}
