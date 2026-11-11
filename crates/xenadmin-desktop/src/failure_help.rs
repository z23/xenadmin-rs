use super::*;
use xenadmin_core::{
    FailureGuidance, OperationRecord, OperationState, OperationStep, ResourceOperationRecord,
    StepState,
};

#[derive(Clone)]
pub(super) struct FailureHelp {
    source: String,
    outcome: &'static str,
    code: Option<String>,
    guide: Option<FailureGuidance>,
    return_focus: String,
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
    fn operation(
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
    pub fn task(pool: PoolId, task: &xenadmin_core::ServerTask) -> Option<Self> {
        if !matches!(task.status.as_str(), "failure" | "cancelled") {
            return None;
        }
        Some(Self::new(
            format!("{} · task {}\nPool {}", task.name, task.id, pool),
            "This is the host task's reported result. Inspect any related client operation and its steps: a failed or cancelled task does not establish rollback of the surrounding workflow.",
            task.error_code.as_deref(),
        ))
    }
    fn new(source: String, outcome: &'static str, code: Option<&str>) -> Self {
        let guide = code.and_then(FailureGuidance::for_api_code);
        Self {
            source,
            outcome,
            code: guide.and(code.map(str::to_owned)),
            guide,
            return_focus: String::new(),
        }
    }
}

// Only application-owned exact error envelopes are parsed. Arbitrary messages and
// administrative block text are never classified by searching for a familiar code.
fn detail_code(detail: &str) -> Option<&str> {
    let code = detail
        .strip_prefix("XAPI refused the request: ")
        .or_else(|| detail.strip_prefix("Host task failed: "))?;
    FailureGuidance::for_api_code(code)?;
    Some(code)
}

pub(super) fn button(help: Option<FailureHelp>, key: impl Into<String>, k: &Kit) -> El {
    let Some(mut help) = help else {
        return el();
    };
    let key = key.into();
    help.return_focus = key.clone();
    Button::new("Explain outcome…")
        .variant(ButtonVariant::Outline)
        .on_press(move |s: &mut State| {
            console_view::release(s);
            s.failure_help = Some(help.clone());
        })
        .build(k)
        .key(key)
}

pub(super) fn dialog(s: &State, k: &Kit) -> El {
    let Some(help) = &s.failure_help else {
        return el();
    };
    let mut body = col()
        .gap(12.0)
        .child(caption(&help.source, k).wrap_anywhere())
        .child(
            caption(
                "Captured explanation · inspect current state before acting",
                k,
            )
            .wrap(),
        )
        .child(heading("Recorded outcome", k))
        .child(caption(help.outcome, k).wrap().key("failure-help-outcome"));
    if let Some(code) = &help.code {
        body = body.child(
            text(format!("Host error: {code}"))
                .font(k.ty.text_small)
                .color(k.c.text_base)
                .wrap_anywhere()
                .selectable(true)
                .key("failure-help-code"),
        );
    }
    if let Some(guide) = help.guide {
        body = body
            .child(heading(guide.title, k))
            .child(caption(guide.explanation, k).wrap());
        for (i, check) in guide.checks.iter().enumerate() {
            body = body.child(caption(format!("{}. {check}", i + 1), k).wrap());
        }
    } else {
        body = body.child(caption("No exact host error code is available here. Read the original detail and per-step messages in the operation or Activity entry. Private diagnostics can provide additional method and receipt evidence.", k).wrap());
    }
    let card = col()
        .w((s.window_width - 32.0).clamp(280.0, 700.0))
        .h((s.window_height - 48.0).clamp(280.0, 720.0))
        .padding(20.0)
        .gap(12.0)
        .radius(10.0)
        .fill(k.c.bg_base)
        .child(heading("Outcome explanation", k))
        .child(body.grow(1.0).min_h(0.0).scroll(rune::ScrollSpec {
            stick_bottom: false,
        }))
        .child(
            row().justify_end().child(
                Button::new("Close")
                    .on_press(|s: &mut State| s.failure_help = None)
                    .build(k)
                    .key("failure-help-close"),
            ),
        );
    el().w_full()
        .h_full()
        .absolute(0.0, 0.0)
        .center()
        .fill(rune::Color::BLACK.alpha(0.5))
        .role("dialog")
        .label("Outcome explanation")
        .modal(false, |s: &mut State| s.failure_help = None)
        .focus_return(help.return_focus.clone())
        .child(card)
        .key("failure-help-dialog")
}

pub(super) fn scene(s: &mut State) {
    demo(s);
    s.failure_help = Some(FailureHelp::new(
        "Create snapshot · synthetic operation\nPool 00000000-0000-0000-0000-000000000001\nVM 00000000-0000-0000-0000-000000000002".into(),
        "Some work completed and later work stopped. Inspect each step and retained resource UUID. Repeating the entire workflow could duplicate completed work; any remaining change or cleanup needs a fresh review.",
        Some("SR_BACKEND_FAILURE_44"),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
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
    #[test]
    fn captured_help_remains_read_only_and_close_is_visible_at_both_sizes() {
        for size in [(513.0, 652.0), (1440.0, 900.0)] {
            let mut state = init();
            scene(&mut state);
            state.window_width = size.0;
            state.window_height = size.1;
            let captured = state.failure_help.as_ref().unwrap().source.clone();
            let mut rt = rune::Runtime::new(
                state,
                view,
                size,
                rune::Env {
                    system_dark: true,
                    scale: 1.0,
                },
            );
            for _ in 0..30 {
                rt.frame(0.016);
            }
            rt.update(|s, _| {
                s.inventory = None;
                s.table.select("different");
            });
            rt.frame(0.016);
            assert_eq!(rt.state.failure_help.as_ref().unwrap().source, captured);
            let (x, y) = rt.find("failure-help-close").unwrap();
            assert!(x > 0.0 && x < size.0 && y > 0.0 && y < size.1);
            rt.pointer_down(x, y);
            rt.pointer_up(x, y);
            rt.frame(0.016);
            assert!(rt.state.failure_help.is_none());
            assert!(rt.state.operations.is_empty());
            assert!(rt.state.resource_operations.is_empty());
        }
    }
}
