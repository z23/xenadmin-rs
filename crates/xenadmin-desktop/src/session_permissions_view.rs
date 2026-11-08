use super::*;
use xenadmin_core::{PermissionDecision, PowerAction};

pub(super) fn power_button(
    action: PowerAction,
    eligible: bool,
    permission: Option<PermissionDecision>,
    k: &Kit,
) -> El {
    Button::new(action.label())
        .variant(if action.is_force() {
            ButtonVariant::Ghost
        } else {
            ButtonVariant::Outline
        })
        .disabled(!eligible || permission.is_some_and(PermissionDecision::is_denied))
        .on_press(move |s: &mut State| s.request_power(action))
        .build(k)
        .key(format!("power-{action:?}"))
}
pub(super) fn notice(permission: PermissionDecision, k: &Kit) -> El {
    caption(permission.explanation(), k)
        .wrap()
        .role("status")
        .key("session-permission-explanation")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn runtime(size: (f32, f32), view: fn(&State, &mut Cx<State>) -> El) -> rune::Runtime<State> {
        let mut s = init();
        demo(&mut s);
        s.window_width = size.0;
        s.window_height = size.1;
        let first = s.rows.first().unwrap().id.clone();
        s.table.select(first);
        rune::Runtime::new(
            s,
            view,
            size,
            rune::Env {
                system_dark: true,
                scale: 1.0,
            },
        )
    }
    fn render(permission: PermissionDecision, eligible: bool, cx: &mut Cx<State>) -> El {
        let k = Kit::neutral(cx.system_dark());
        col()
            .padding(20.0)
            .w_full()
            .child(power_button(
                PowerAction::Start,
                eligible,
                Some(permission),
                &k,
            ))
            .child(notice(permission, &k))
    }
    fn denied(_: &State, cx: &mut Cx<State>) -> El {
        render(PermissionDecision::Denied, true, cx)
    }
    fn unknown(_: &State, cx: &mut Cx<State>) -> El {
        render(
            PermissionDecision::Unknown(xenadmin_core::PermissionUnknown::Unsupported),
            true,
            cx,
        )
    }
    fn object_ineligible(_: &State, cx: &mut Cx<State>) -> El {
        render(PermissionDecision::Granted, false, cx)
    }
    #[test]
    fn known_denial_disables_power_without_hiding_its_reason_at_both_widths() {
        for size in [(1440.0, 900.0), (513.0, 652.0)] {
            let mut r = runtime(size, denied);
            r.frame(0.0);
            assert!(r.find("session-permission-explanation").is_some());
            let (x, y) = r.find("power-Start").unwrap();
            r.pointer_down(x, y);
            r.pointer_up(x, y);
            r.frame(0.0);
            assert!(r.state.error.is_none());
            assert!(r.state.operations.is_empty());
            assert!(r.state.force_action.is_none());
        }
    }
    #[test]
    fn unknown_permissions_allow_review_but_grants_never_override_object_ineligibility() {
        let mut r = runtime((513.0, 652.0), unknown);
        r.frame(0.0);
        let (x, y) = r.find("power-Start").unwrap();
        r.pointer_down(x, y);
        r.pointer_up(x, y);
        r.frame(0.0);
        assert!(r.state.error.is_some()); // Reached the ordinary stale/demo guard, not a guessed RBAC denial.
        let mut r = runtime((513.0, 652.0), object_ineligible);
        r.frame(0.0);
        let (x, y) = r.find("power-Start").unwrap();
        r.pointer_down(x, y);
        r.pointer_up(x, y);
        r.frame(0.0);
        assert!(r.state.error.is_none());
    }
}
