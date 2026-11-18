//! The "Operations" section of a VM's General tab: the recent operation
//! records of the VM, as the Rune details pane listed them, each with what
//! Rune showed under it.
//!
//! The controls are not this area's. Under a record, in the Rune order:
//!
//! - "Explain outcome…" (`vm-failure-help-<id>`), from
//!   [`crate::areas::events::outcome_actions`];
//! - "Request cancellation" or "Stop remaining steps", "Recheck outcome",
//!   "Review unknown outcome" and, under a finished recovery boot, "Restore
//!   saved boot order", from [`crate::areas::power::operation_controls`]
//!   with the Rune conditions;
//! - "Review cancellation request…", the reviewed cancellation of the Rune
//!   recovery panel, from [`crate::areas::power::cancellation_review_button`].
//!
//! `outcome_actions` also offers the cancellation, the recheck and the
//! acknowledgement under the rules of the Events tab. Under a record of
//! this tab they would be the same three controls twice, so only its
//! explanation is taken: the buttons here are the ones the Rune details
//! pane had, and "Review unknown outcome" opens the one acknowledgement
//! dialog either way.

use xenadmin_core::{OperationRecord, OperationState, PoolId, VmId};

use crate::areas::events::{self, Family, OperationKey};
use crate::areas::power;
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

/// What Rune showed under one record, in its order: the explanation, the
/// record's controls, the reviewed cancellation. No key is twice in it.
fn controls(m: &Model, record: &OperationRecord) -> Vec<Button> {
    let key = OperationKey {
        pool: record.pool,
        id: record.id,
        family: Family::Vm,
    };
    let explain = format!("vm-failure-help-{}", record.id);
    let mut buttons: Vec<Button> = events::outcome_actions(m, key)
        .into_iter()
        .filter(|button| button.key == explain)
        .collect();
    buttons.extend(power::operation_controls(m, record));
    buttons.extend(power::cancellation_review_button(m, record));
    buttons
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
            .actions(controls(m, record))
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
