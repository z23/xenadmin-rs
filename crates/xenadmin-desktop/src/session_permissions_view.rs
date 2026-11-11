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
            ButtonVariant::Danger
        } else {
            ButtonVariant::Neutral
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

pub(super) fn assess(
    s: &State,
    requirements: &xenadmin_core::PermissionRequirements,
) -> xenadmin_core::PermissionAssessment {
    requirements.assess(
        &s.snapshot
            .as_ref()
            .map(|s| s.session_permissions())
            .unwrap_or_default(),
    )
}
pub(super) fn review_notice(assessment: &xenadmin_core::PermissionAssessment, k: &Kit) -> El {
    caption(assessment.explanation(), k)
        .wrap()
        .role("status")
        .key("review-permission-explanation")
}
pub(super) fn footer(
    controls: El,
    permission: Option<&xenadmin_core::PermissionAssessment>,
    width: f32,
    k: &Kit,
) -> El {
    if let Some(permission) = permission {
        col()
            .w(width)
            .gap(8.0)
            .child(review_notice(permission, k).w(width))
            .child(controls.w(width).flex_wrap())
    } else {
        controls
    }
}
pub(super) fn check(s: &mut State, requirements: &xenadmin_core::PermissionRequirements) -> bool {
    let assessment = assess(s, requirements);
    if assessment.decision().is_denied() {
        s.error = Some(assessment.explanation());
        false
    } else {
        true
    }
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
        let k = crate::theme::kit(cx.system_dark());
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

#[cfg(test)]
mod review_tests {
    use super::*;
    fn storage(s: &mut State) {
        crate::storage_view::scene(s, true);
    }
    fn pool(s: &mut State) {
        crate::pool_view::scene(s, true);
    }
    fn resource(s: &mut State) {
        crate::resource_admin::scene(s, false);
    }
    #[test]
    fn review_permission_notice_is_reachable_without_bypassing_synthetic_target_guard() {
        for scene in [
            storage as fn(&mut State),
            pool,
            resource,
            crate::host_admin::scene,
            crate::vm_lifecycle::review_scene,
            crate::vm_snapshots::restore_scene,
            crate::vm_snapshots::edit_scene,
            crate::appliance_view::review_scene,
            crate::snapshot_policy_view::review_scene,
            crate::organization_view::review_scene,
            crate::host_power_view::scene,
            crate::cloud_init_view::review_scene,
            crate::profiles_view::root_password_scene,
        ] {
            for size in [(1440.0, 900.0), (513.0, 652.0)] {
                let mut s = init();
                scene(&mut s);
                s.window_width = size.0;
                s.window_height = size.1;
                let mut runtime = rune::Runtime::new(
                    s,
                    crate::view,
                    size,
                    rune::Env {
                        system_dark: true,
                        scale: 1.0,
                    },
                );
                runtime.frame(0.0);
                runtime.update(|_, tx| tx.scroll_into_view("review-permission-explanation"));
                for _ in 0..30 {
                    runtime.frame(1.0 / 60.0);
                }
                let (x, y) = runtime
                    .find("review-permission-explanation")
                    .expect("captured review must explain permission discovery");
                assert!(
                    (0.0..=size.0).contains(&x) && (0.0..=size.1).contains(&y),
                    "notice is outside {size:?}: {x},{y}"
                );
                let key = [
                    "storage-confirm",
                    "pool-confirm",
                    "resource-confirm",
                    "host-confirm",
                    "apply-vm-lifecycle",
                    "confirm-snapshot",
                    "appliance-apply",
                    "policy-confirm",
                    "organization-apply",
                    "host-power-confirm",
                    "cloud-init-apply",
                    "root-password-submit",
                ]
                .into_iter()
                .find(|key| runtime.find(key).is_some())
                .unwrap();
                runtime.update(|_, tx| tx.scroll_into_view(key));
                for _ in 0..30 {
                    runtime.frame(1.0 / 60.0);
                }
                let (x, y) = runtime.find(key).unwrap();
                runtime.pointer_down(x, y);
                runtime.pointer_up(x, y);
                runtime.frame(0.0);
                assert!(runtime.state.resource_operations.is_empty());
                assert!(runtime.state.operations.is_empty());
                assert!(runtime.state.error.is_none());
            }
        }
    }
}
