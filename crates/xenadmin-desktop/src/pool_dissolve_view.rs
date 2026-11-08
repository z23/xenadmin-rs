use super::*;
use xenadmin_core::{ObservedResource, OperationId, PoolDissolve, ResourceCommand, ResourceTarget};
pub(super) struct Editor {
    pool: PoolId,
    observed: Option<ObservedResource>,
    operation: PoolDissolve,
    enabled_members: usize,
    confirmation: TextEdit,
    error: Option<String>,
}
pub(super) fn open(s: &mut State) {
    let Some(inventory) = s.inventory.as_ref() else {
        return;
    };
    let operation = match PoolDissolve::capture(inventory) {
        Ok(op) => op,
        Err(e) => {
            s.error = Some(format!(
                "Cannot review dissolution: {e}. The pool must be named, HA must be disabled, and membership must be available."
            ));
            return;
        }
    };
    let enabled_members = operation
        .members
        .iter()
        .filter(|m| inventory.hosts.get(&m.host).is_some_and(|h| h.enabled))
        .count();
    s.pool_dissolve_editor = Some(Editor {
        pool: inventory.pool_id,
        observed: s.snapshot.as_ref().and_then(|v| {
            v.observe_resource(ResourceTarget::Pool(inventory.pool_id))
                .ok()
        }),
        operation,
        enabled_members,
        confirmation: TextEdit::new(""),
        error: None,
    });
    s.pool_editor = None;
}
fn current(s: &State, e: &Editor) -> bool {
    !s.demo
        && s.can_mutate()
        && e.observed.is_some()
        && s.snapshot
            .as_ref()
            .and_then(|v| v.observe_resource(ResourceTarget::Pool(e.pool)).ok())
            == e.observed
}
fn apply(s: &mut State) {
    let result = (|| {
        let e = s
            .pool_dissolve_editor
            .as_ref()
            .ok_or("Pool review closed")?;
        if !current(s, e) {
            return Err("Connection changed; reopen pool dissolution".into());
        }
        if e.confirmation.text != e.pool.to_string() || e.enabled_members > 0 {
            return Err(
                "Disable and evacuate the reviewed members first, then enter the exact pool UUID"
                    .into(),
            );
        }
        let permission =
            session_permissions_view::assess(s, &e.operation.permission_requirements());
        if permission.decision().is_denied() {
            return Err(permission.explanation());
        }
        s.connection
            .as_ref()
            .ok_or("Connect first")?
            .submit_resource(
                OperationId::new(),
                ResourceCommand::DissolvePool {
                    pool: e.observed.unwrap(),
                    operation: e.operation.clone(),
                },
            )
            .map_err(|e| e.to_string())
    })();
    match result {
        Ok(op) => {
            if let Some(w) = &s.waker {
                op.notify_on_change(w.clone());
            }
            s.resource_operations.push(op);
            s.pool_dissolve_editor = None;
            activity::open(s);
        }
        Err(error) => {
            if let Some(e) = &mut s.pool_dissolve_editor {
                e.error = Some(error);
            }
        }
    }
}
pub(super) fn dialog(s: &State, k: &Kit) -> El {
    let Some(e) = &s.pool_dissolve_editor else {
        return el();
    };
    let permission = session_permissions_view::assess(s, &e.operation.permission_requirements());
    let mut body=col().gap(12.0).child(heading("Dissolve pool",k))
        .child(caption(format!("{}\nPool {}\nRemaining coordinator {}",e.operation.expected_name,e.pool,e.operation.coordinator),k).wrap_anywhere())
        .child(caption(if e.operation.members.is_empty(){"This pool already has one host. Clear its pool name and description; keep the host, VMs and disks."}else{"Eject exactly these members in order, then clear the pool name and description on the remaining coordinator. Each ejected host reboots and its pool configuration is reset."},k).wrap())
        .child(caption("Every member must already be disabled, evacuated and free of local guest-disk data. The client checks all members before the first ejection and checks remaining membership before every step. HA must stay disabled.",k).wrap());
    for (n, host) in e.operation.members.iter().enumerate() {
        body = body
            .child(caption(format!("{}. {}\n{}", n + 1, host.name, host.host), k).wrap_anywhere());
    }
    body=body.child(caption("Stop lets an already submitted ejection finish and skips later steps. Interrupted work is never resumed automatically. Ejected-host reboot and independent connectivity require separate verification.",k).wrap())
        .child(caption(format!("Enter the pool UUID to confirm: {}",e.pool),k).wrap_anywhere())
        .child(TextInput::new(&e.confirmation).label("Confirmation pool UUID").on_edit(|s:&mut State|&mut s.pool_dissolve_editor.as_mut().unwrap().confirmation).build(k).key("pool-dissolve-identity"));
    if e.enabled_members > 0 {
        body=body.child(caption(format!("{} reviewed members are still enabled. Close this review, disable and evacuate them, then review again.",e.enabled_members),k).wrap());
    }
    if s.demo {
        body = body.child(caption("Synthetic preview; submission is disabled.", k));
    } else if !current(s, e) {
        body = body.child(
            caption(
                "Connection changed or is read-only; reopen this review before applying.",
                k,
            )
            .wrap(),
        );
    }
    if let Some(error) = &e.error {
        body = body.child(caption(error, k).wrap());
    }
    let controls = row()
        .gap(8.0)
        .flex_wrap()
        .justify_end()
        .child(
            Button::new("Close")
                .on_press(|s: &mut State| s.pool_dissolve_editor = None)
                .build(k),
        )
        .child(
            Button::new("Dissolve reviewed pool")
                .variant(ButtonVariant::Danger)
                .disabled(
                    !current(s, e)
                        || e.enabled_members > 0
                        || e.confirmation.text != e.pool.to_string()
                        || permission.decision().is_denied(),
                )
                .on_press(apply)
                .build(k)
                .key("pool-dissolve-confirm"),
        );
    let width = (s.window_width - 32.0).clamp(280.0, 800.0);
    el().w_full()
        .h_full()
        .absolute(0.0, 0.0)
        .center()
        .fill(rune::Color::BLACK.alpha(0.5))
        .modal(false, |s: &mut State| s.pool_dissolve_editor = None)
        .role("dialog")
        .label("Dissolve pool")
        .child(
            col()
                .w(width)
                .h((s.window_height - 40.0).max(280.0))
                .padding(18.0)
                .gap(12.0)
                .radius(10.0)
                .fill(k.c.bg_base)
                .child(body.grow(1.0).min_h(0.0).scroll(rune::ScrollSpec {
                    stick_bottom: false,
                }))
                .child(session_permissions_view::footer(
                    controls,
                    Some(&permission),
                    width - 36.0,
                    k,
                )),
        )
}
pub(super) fn scene(s: &mut State) {
    demo(s);
    let inventory = Arc::make_mut(s.inventory.as_mut().unwrap());
    let coordinator = *inventory.hosts.keys().next().unwrap();
    let pool = inventory.details.pool.as_mut().unwrap();
    pool.administration.coordinator = Some(coordinator);
    pool.administration.ha_enabled = Some(false);
    for host in inventory.hosts.values_mut() {
        if host.id != coordinator {
            host.enabled = false;
        }
    }
    open(s);
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dissolve_review_keeps_exact_members_and_confirmation_reachable_at_both_widths() {
        for size in [(513.0, 652.0), (1440.0, 900.0)] {
            let mut state = init();
            scene(&mut state);
            state.window_width = size.0;
            state.window_height = size.1;
            let expected = state
                .pool_dissolve_editor
                .as_ref()
                .unwrap()
                .operation
                .clone();
            let pool = state.pool_dissolve_editor.as_ref().unwrap().pool;
            state.pool_dissolve_editor.as_mut().unwrap().confirmation =
                TextEdit::new(pool.to_string());
            let mut runtime = rune::Runtime::new(
                state,
                crate::view,
                size,
                rune::Env {
                    system_dark: true,
                    scale: 1.0,
                },
            );
            runtime.frame(0.0);
            for key in ["review-permission-explanation", "pool-dissolve-confirm"] {
                let (x, y) = runtime.find(key).unwrap();
                assert!(
                    (0.0..size.0).contains(&x) && (0.0..size.1).contains(&y),
                    "{key}: {x},{y}"
                );
            }
            let (x, y) = runtime.find("pool-dissolve-confirm").unwrap();
            runtime.pointer_down(x, y);
            runtime.pointer_up(x, y);
            runtime.frame(0.0);
            assert!(runtime.state.resource_operations.is_empty());
            assert_eq!(
                runtime
                    .state
                    .pool_dissolve_editor
                    .as_ref()
                    .unwrap()
                    .operation,
                expected
            );
            runtime.state.inventory = None;
            apply(&mut runtime.state);
            assert!(runtime.state.resource_operations.is_empty());
            assert!(
                runtime
                    .state
                    .pool_dissolve_editor
                    .as_ref()
                    .unwrap()
                    .error
                    .is_some()
            );
        }
    }
    #[test]
    fn dissolve_capture_refuses_unknown_coordinator_ha_and_enabled_members_cannot_submit() {
        let mut state = init();
        scene(&mut state);
        let inventory = state.inventory.as_ref().unwrap().as_ref();
        let mut bad = inventory.clone();
        bad.details
            .pool
            .as_mut()
            .unwrap()
            .administration
            .coordinator = None;
        assert!(PoolDissolve::capture(&bad).is_err());
        bad = inventory.clone();
        bad.details.pool.as_mut().unwrap().administration.ha_enabled = Some(true);
        assert!(PoolDissolve::capture(&bad).is_err());
        let member = state
            .pool_dissolve_editor
            .as_ref()
            .unwrap()
            .operation
            .members[0]
            .host;
        Arc::make_mut(state.inventory.as_mut().unwrap())
            .hosts
            .get_mut(&member)
            .unwrap()
            .enabled = true;
        open(&mut state);
        assert_eq!(
            state.pool_dissolve_editor.as_ref().unwrap().enabled_members,
            1
        );
    }
}
