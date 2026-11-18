//! "Explain outcome…": what a recorded outcome means and, where the host
//! named an exact error code, what the core's guidance says about it.
//!
//! The Rune `failure_help.rs`, with its texts word for word. The explanation
//! is captured when it is asked for: it does not follow the selection or the
//! inventory afterwards, and it submits nothing.
//!
//! What has no successor: `return_focus`, the key of the button the Rune
//! dialog gave the focus back to. Returning the focus to the opener is the
//! dialog host's business in the new front end.

use xenadmin_core::{
    FailureGuidance, OperationRecord, OperationState, OperationStep, PoolId,
    ResourceOperationRecord, ServerTask, StepState,
};

#[derive(Clone)]
pub struct FailureHelp {
    pub(super) source: String,
    pub(super) outcome: &'static str,
    pub(super) code: Option<String>,
    pub(super) guide: Option<FailureGuidance>,
}

impl FailureHelp {
    pub fn vm(record: &OperationRecord) -> Option<Self> {
        Self::operation(
            format!(
                "{} · operation {}\nPool {} · VM {}",
                record.action.label(),
                record.id,
                record.pool,
                record.vm
            ),
            &record.state,
            &record.detail,
            &record.steps,
        )
    }

    pub fn resource(record: &ResourceOperationRecord) -> Option<Self> {
        Self::operation(
            format!(
                "{} · operation {}\nPool {} · target {}",
                record.action.label(),
                record.id,
                record.pool,
                record.target.uuid()
            ),
            &record.state,
            &record.detail,
            &record.steps,
        )
    }

    pub(super) fn operation(
        source: String,
        state: &OperationState,
        detail: &str,
        steps: &[OperationStep],
    ) -> Option<Self> {
        let outcome = match state {
            OperationState::OutcomeUnknown => {
                "The result is unresolved. The host may have accepted work even though its reply was lost. Inspect the existing task, steps and retained resources, then use Recheck outcome. Do not submit the whole operation again while its outcome is uncertain."
            }
            OperationState::AcknowledgedUnknown => {
                "The uncertainty was acknowledged. This released a local guard; it did not establish host success, failure or rollback. Inspect the host and retained resources before reviewing any new change."
            }
            OperationState::PartiallyCompleted => {
                "Some work completed and later work stopped. Inspect each step and retained resource UUID. Repeating the entire workflow could duplicate completed work; any remaining change or cleanup needs a fresh review."
            }
            OperationState::Failed | OperationState::Rejected => {
                "The recorded request failed or was refused. Inspect its steps and any created resources before reviewing another change; a failure does not imply that earlier work was rolled back."
            }
            OperationState::Cancelled => {
                "The operation stopped. Inspect its steps, host tasks and retained resources; stopping work does not undo completed changes."
            }
            _ => return None,
        };
        let code = detail_code(detail).or_else(|| {
            steps
                .iter()
                .rev()
                .filter(|step| matches!(step.state, StepState::Failed | StepState::OutcomeUnknown))
                .find_map(|step| detail_code(&step.detail))
        });
        Some(Self::new(source, outcome, code))
    }

    pub fn task(pool: PoolId, task: &ServerTask) -> Option<Self> {
        if !matches!(task.status.as_str(), "failure" | "cancelled") {
            return None;
        }
        Some(Self::new(
            format!("{} · task {}\nPool {}", task.name, task.id, pool),
            "This is the host task's reported result. Inspect any related client operation and its steps: a failed or cancelled task does not establish rollback of the surrounding workflow.",
            task.error_code.as_deref(),
        ))
    }

    pub(super) fn new(source: String, outcome: &'static str, code: Option<&str>) -> Self {
        let guide = code.and_then(FailureGuidance::for_api_code);
        Self {
            source,
            outcome,
            code: guide.and(code.map(str::to_owned)),
            guide,
        }
    }
}

// Only application-owned exact error envelopes are parsed. Arbitrary messages and
// administrative block text are never classified by searching for a familiar code.
pub(super) fn detail_code(detail: &str) -> Option<&str> {
    let code = detail
        .strip_prefix("XAPI refused the request: ")
        .or_else(|| detail.strip_prefix("Host task failed: "))?;
    FailureGuidance::for_api_code(code)?;
    Some(code)
}

/// The text shown when no exact host error code is known. It names the Rune
/// "Activity" entry (AREAS.md decision 10: renamed in one sweep at the end).
pub const NO_CODE: &str = "No exact host error code is available here. Read the original detail and per-step messages in the operation or its entry on the Events tab of the Notifications view. Private diagnostics can provide additional method and receipt evidence.";

/// The line under the source: the explanation is a capture.
pub const CAPTURED: &str = "Captured explanation · inspect current state before acting";

#[cfg(test)]
mod tests {
    use super::*;

    /// The Rune test of the same name, as it was.
    #[test]
    fn exact_envelopes_only_and_unknown_outcome_takes_precedence_over_host_advice() {
        assert_eq!(
            detail_code("XAPI refused the request: SR_FULL"),
            Some("SR_FULL")
        );
        for value in [
            "A note mentioning SR_FULL",
            "XAPI refused the request: SR_FULL; detail",
            "Host task failed: SR_FULL\nprivate",
            "Administratively blocked: SR_FULL",
        ] {
            assert!(detail_code(value).is_none());
        }
        let steps = vec![OperationStep {
            label: "Second step".into(),
            state: StepState::Failed,
            detail: "Host task failed: SR_FULL".into(),
        }];
        let help = FailureHelp::operation(
            "Captured operation".into(),
            &OperationState::OutcomeUnknown,
            "Reply lost",
            &steps,
        )
        .unwrap();
        assert_eq!(help.code.as_deref(), Some("SR_FULL"));
        assert!(help.outcome.contains("Do not submit"));
        assert!(
            FailureHelp::operation(
                "x".into(),
                &OperationState::Succeeded,
                "Host task failed: SR_FULL",
                &steps
            )
            .is_none()
        );
        let ack = FailureHelp::operation(
            "x".into(),
            &OperationState::AcknowledgedUnknown,
            "Acknowledged",
            &[],
        )
        .unwrap();
        assert!(ack.outcome.contains("did not establish"));
    }
}
