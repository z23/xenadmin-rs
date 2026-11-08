use super::*;
use xenadmin_core::{
    AlarmEmailSettings, AlarmMetric, AlarmOperation, AlarmPolicy, AlarmPolicyChange, AlarmRule,
    ObservedResource, OperationId, ResourceAction, ResourceCommand, ResourceTarget,
};
#[derive(Clone, Copy, PartialEq)]
enum Page {
    Policies,
    Email,
    Refresh,
}
pub(super) struct Workspace {
    initial: Arc<Inventory>,
    pool: Option<ObservedResource>,
    page: Page,
    policy: Option<AlarmPolicy>,
    target: Option<ObservedResource>,
    metric: AlarmMetric,
    filter: TextEdit,
    threshold: TextEdit,
    duration: TextEdit,
    repeat: TextEdit,
    enabled: bool,
    destination: TextEdit,
    relay: TextEdit,
    language: String,
    review: Option<AlarmOperation>,
    error: Option<String>,
}
pub(super) fn open(s: &mut State) {
    let Some(initial) = s.inventory.clone() else {
        return;
    };
    let email = initial
        .details
        .alarm_email
        .as_ref()
        .and_then(|v| v.settings.clone())
        .unwrap_or(AlarmEmailSettings {
            destination: String::new(),
            relay: String::new(),
            language: "en-US".into(),
        });
    let pool = s.snapshot.as_ref().and_then(|v| {
        v.observe_resource(ResourceTarget::Pool(initial.pool_id))
            .ok()
    });
    s.alarms_workspace = Some(Workspace {
        initial,
        pool,
        page: Page::Policies,
        policy: None,
        target: None,
        metric: AlarmMetric::Cpu,
        filter: TextEdit::new(""),
        threshold: TextEdit::new("80"),
        duration: TextEdit::new("60"),
        repeat: TextEdit::new("3600"),
        enabled: true,
        destination: TextEdit::new(email.destination),
        relay: TextEdit::new(email.relay),
        language: email.language,
        review: None,
        error: None,
    });
}
fn label(inv: &Inventory, policy: &AlarmPolicy) -> String {
    let name = match policy.target {
        ResourceTarget::Vm(id) => inv
            .vms
            .get(&id)
            .map(|v| v.name.clone())
            .unwrap_or_else(|| "Control domain".into()),
        ResourceTarget::Host(id) => inv
            .hosts
            .get(&id)
            .map(|v| v.name.clone())
            .unwrap_or_else(|| "Host".into()),
        ResourceTarget::Storage(id) => inv
            .storage
            .iter()
            .find(|v| v.uuid == id.uuid())
            .map(|v| v.name.clone())
            .unwrap_or_else(|| "Storage".into()),
        _ => "Resource".into(),
    };
    if let Some(host) = policy.control_domain_host {
        format!(
            "{} · control-domain memory",
            inv.hosts
                .get(&host)
                .map(|v| v.name.as_str())
                .unwrap_or("Host")
        )
    } else {
        format!(
            "{name} · {}",
            match policy.target {
                ResourceTarget::Vm(_) => "VM",
                ResourceTarget::Host(_) => "Host",
                ResourceTarget::Storage(_) => "Storage",
                _ => "Resource",
            }
        )
    }
}
fn units(metric: AlarmMetric) -> &'static str {
    match metric {
        AlarmMetric::Cpu | AlarmMetric::ControlDomainMemory => "percent (0–100)",
        _ => metric.unit(),
    }
}
fn load_metric(w: &mut Workspace, metric: AlarmMetric) {
    w.metric = metric;
    w.review = None;
    w.error = None;
    let rule = w
        .policy
        .as_ref()
        .and_then(|p| p.rules.iter().find(|r| r.metric == metric));
    w.enabled = rule.is_some();
    let native = rule
        .map(|r| r.threshold.parse::<f64>().unwrap_or(0.0))
        .unwrap_or(
            if matches!(metric, AlarmMetric::Cpu | AlarmMetric::ControlDomainMemory) {
                0.8
            } else {
                1024.0
            },
        );
    w.threshold = TextEdit::new(
        (if matches!(metric, AlarmMetric::Cpu | AlarmMetric::ControlDomainMemory) {
            native * 100.0
        } else {
            native
        })
        .to_string(),
    );
    w.duration = TextEdit::new(rule.map(|r| r.duration_seconds).unwrap_or(60).to_string());
    w.repeat = TextEdit::new(rule.map(|r| r.repeat_seconds).unwrap_or(3600).to_string());
}
fn select(s: &mut State, index: usize) {
    let Some(w) = &mut s.alarms_workspace else {
        return;
    };
    let Some(policy) = w.initial.details.alarm_policies.get(index).cloned() else {
        return;
    };
    w.target = s
        .snapshot
        .as_ref()
        .and_then(|v| v.observe_resource(policy.target).ok());
    let metric = AlarmMetric::ALL
        .into_iter()
        .find(|m| m.supported(policy.target, policy.control_domain_host))
        .unwrap_or(AlarmMetric::Cpu);
    w.policy = Some(policy);
    load_metric(w, metric);
}
fn intent(w: &Workspace) -> Result<AlarmOperation, String> {
    match w.page {
        Page::Policies => {
            let p = w.policy.as_ref().ok_or("Choose a resource")?;
            if let Some(error) = &p.error {
                return Err(error.clone());
            }
            let rule = if w.enabled {
                if p.customized.contains(&w.metric) {
                    return Err("This alarm has a custom source or direction. Keep its definition or explicitly disable it; this editor cannot replace it.".into());
                }
                let mut value = w
                    .threshold
                    .text
                    .parse::<f64>()
                    .map_err(|_| "Enter a numeric threshold")?;
                if matches!(
                    w.metric,
                    AlarmMetric::Cpu | AlarmMetric::ControlDomainMemory
                ) {
                    value /= 100.0;
                }
                let rule = AlarmRule {
                    metric: w.metric,
                    threshold: value.to_string(),
                    duration_seconds: w
                        .duration
                        .text
                        .parse()
                        .map_err(|_| "Duration must be a whole number of seconds")?,
                    repeat_seconds: w
                        .repeat
                        .text
                        .parse()
                        .map_err(|_| "Repeat interval must be a whole number of seconds")?,
                };
                rule.validate().map_err(|_|"Check the threshold and use duration 1–604800 seconds and repeat interval 1–2592000 seconds")?;
                Some(rule)
            } else {
                if !p.rules.iter().any(|r| r.metric == w.metric) {
                    return Err("This alarm is already disabled".into());
                }
                None
            };
            Ok(AlarmOperation::SetPolicy(AlarmPolicyChange {
                expected_revision: p.revision.clone(),
                control_domain_host: p.control_domain_host,
                metric: w.metric,
                rule,
            }))
        }
        Page::Email => {
            let current = w
                .initial
                .details
                .alarm_email
                .as_ref()
                .ok_or("Pool email settings unavailable")?;
            if let Some(error) = &current.error {
                return Err(error.clone());
            }
            let settings = AlarmEmailSettings {
                destination: w.destination.text.clone(),
                relay: w.relay.text.clone(),
                language: w.language.clone(),
            };
            settings.validate().map_err(|_|"Use one email address (or leave it empty to disable), a relay host[:port], and a supported language")?;
            Ok(AlarmOperation::SetEmail {
                expected_revision: current.revision.clone(),
                settings,
            })
        }
        Page::Refresh => {
            let hosts = w.initial.hosts.keys().copied().collect::<Vec<_>>();
            if hosts.is_empty() || hosts.len() > 64 {
                return Err("Refresh supports 1–64 captured hosts".into());
            }
            Ok(AlarmOperation::Refresh { hosts })
        }
    }
}
fn review(s: &mut State) {
    if let Some(w) = &mut s.alarms_workspace {
        match intent(w) {
            Ok(op) => {
                w.review = Some(op);
                w.error = None;
            }
            Err(e) => w.error = Some(e),
        }
    }
}
fn observed(w: &Workspace) -> Option<ObservedResource> {
    if w.page == Page::Policies {
        w.target
    } else {
        w.pool
    }
}
fn current(s: &State, w: &Workspace) -> bool {
    !s.demo
        && s.can_mutate()
        && s.resource_operations
            .iter()
            .filter(|h| !h.latest().state.terminal())
            .count()
            < 64
        && observed(w).is_some_and(|o| {
            s.snapshot
                .as_ref()
                .and_then(|v| v.observe_resource(o.target()).ok())
                == Some(o)
        })
}
fn submit(s: &mut State) {
    let result = (|| {
        let w = s
            .alarms_workspace
            .as_ref()
            .ok_or(xenadmin_core::Error::Stale)?;
        if !current(s, w) {
            return Err(xenadmin_core::Error::Stale);
        }
        s.connection
            .as_ref()
            .ok_or(xenadmin_core::Error::Closed)?
            .submit_resource(
                OperationId::new(),
                ResourceCommand::Alarm {
                    target: observed(w).ok_or(xenadmin_core::Error::Stale)?,
                    operation: w.review.clone().ok_or(xenadmin_core::Error::Stale)?,
                },
            )
    })();
    match result {
        Ok(op) => {
            if let Some(waker) = &s.waker {
                op.notify_on_change(waker.clone());
            }
            s.resource_operations.push(op);
            s.alarms_workspace = None;
        }
        Err(e) => {
            if let Some(w) = &mut s.alarms_workspace {
                w.error = Some(e.to_string());
            }
        }
    }
}
fn review_lines(w: &Workspace, op: &AlarmOperation) -> Vec<String> {
    let mut lines = vec![format!("Pool {}", w.initial.pool_id)];
    match op {
        AlarmOperation::SetPolicy(change) => {
            if let Some(p) = &w.policy {
                lines.push(format!("{} ({})", label(&w.initial, p), p.target.uuid()));
            }
            lines.push(match &change.rule {
                Some(rule) => format!(
                    "{}: {} {} {}, sustained for {} seconds; repeat no sooner than {} seconds",
                    rule.metric.label(),
                    if rule.metric.low_threshold() {
                        "below"
                    } else {
                        "above"
                    },
                    w.threshold.text,
                    units(rule.metric),
                    rule.duration_seconds,
                    rule.repeat_seconds
                ),
                None => format!("Disable {}", change.metric.label()),
            });
            lines.push("Only this alarm is changed. Other alarm definitions and metadata are retained. Saving does not verify the host monitoring service. Request a service refresh separately, or wait for its periodic reload (up to 30 minutes).".into());
        }
        AlarmOperation::SetEmail { settings, .. } => {
            lines.push(if settings.destination.is_empty() {
                "Disable pool alarm email delivery".into()
            } else {
                format!(
                    "Destination: {} · relay: {} · language: {}",
                    settings.destination,
                    if settings.relay.is_empty() {
                        "host default"
                    } else {
                        &settings.relay
                    },
                    settings.language
                )
            });
            lines.push("This saves pool email configuration. It does not send a test message or verify SMTP delivery. Existing sender, credentials and other SMTP settings are retained.".into());
        }
        AlarmOperation::Refresh { hosts } => {
            for host in hosts {
                lines.push(format!(
                    "{} ({host})",
                    w.initial
                        .hosts
                        .get(host)
                        .map(|h| h.name.as_str())
                        .unwrap_or("Host")
                ));
            }
            lines.push("Request the installed perfmon service to reload configuration on these hosts. Acceptance confirms only that a refresh notification was sent; it does not confirm alarm evaluation or email delivery. Failed or uncertain requests are not replayed.".into());
        }
    }
    lines
}
pub(super) fn dialog(s: &State, k: &Kit) -> El {
    let Some(w) = &s.alarms_workspace else {
        return el();
    };
    let mut body = col()
        .gap(10.0)
        .child(heading("Performance alarms and email", k));
    let mut tabs = row().gap(6.0).flex_wrap();
    for (page, name) in [
        (Page::Policies, "Thresholds"),
        (Page::Email, "Pool email"),
        (Page::Refresh, "Refresh service"),
    ] {
        tabs = tabs.child(
            Button::new(name)
                .disabled(w.page == page || w.review.is_some())
                .on_press(move |s: &mut State| {
                    let w = s.alarms_workspace.as_mut().unwrap();
                    w.page = page;
                    w.error = None;
                })
                .build(k),
        );
    }
    body = body.child(tabs);
    if let Some(op) = &w.review {
        for line in review_lines(w, op) {
            body = body.child(caption(line, k).wrap_anywhere());
        }
    } else {
        match w.page {
            Page::Policies => {
                if let Some(p) = &w.policy {
                    body = body
                        .child(heading(label(&w.initial, p), k))
                        .child(caption(p.target.uuid().to_string(), k).wrap_anywhere());
                    if let Some(error) = &p.error {
                        body = body.child(caption(error, k).wrap());
                    }
                    let mut metrics = row().gap(6.0).flex_wrap();
                    for metric in AlarmMetric::ALL
                        .into_iter()
                        .filter(|m| m.supported(p.target, p.control_domain_host))
                    {
                        metrics = metrics.child(
                            Button::new(metric.label())
                                .disabled(w.metric == metric)
                                .on_press(move |s: &mut State| {
                                    load_metric(s.alarms_workspace.as_mut().unwrap(), metric)
                                })
                                .build(k),
                        );
                    }
                    body = body.child(metrics).child(
                        Checkbox::new(w.enabled)
                            .label("Enable this alarm")
                            .on_toggle(|s: &mut State| {
                                let w = s.alarms_workspace.as_mut().unwrap();
                                w.enabled = !w.enabled;
                            })
                            .build(k)
                            .key("alarm-enabled"),
                    );
                    if p.customized.contains(&w.metric) {
                        body=body.child(caption("Custom source/direction: editing is unavailable; explicit disabling is supported.",k).wrap());
                    }
                    body=body.child(caption(format!("Trigger when {} is {} the threshold ({})",w.metric.label(),if w.metric.low_threshold(){"below"}else{"above"},units(w.metric)),k).wrap())
                .child(TextInput::new(&w.threshold).label("Alarm threshold").on_edit(|s:&mut State|&mut s.alarms_workspace.as_mut().unwrap().threshold).build(k).key("alarm-threshold"))
                .child(caption("Duration in seconds",k)).child(TextInput::new(&w.duration).label("Duration in seconds").on_edit(|s:&mut State|&mut s.alarms_workspace.as_mut().unwrap().duration).build(k).key("alarm-duration"))
                .child(caption("Minimum repeat interval in seconds",k)).child(TextInput::new(&w.repeat).label("Minimum repeat interval in seconds").on_edit(|s:&mut State|&mut s.alarms_workspace.as_mut().unwrap().repeat).build(k).key("alarm-repeat"))
                .child(caption(format!("{} other/custom variables are retained. Host monitoring runs independently of this desktop.",p.other_variables),k).wrap());
                } else {
                    body = body.child(
                        TextInput::new(&w.filter)
                            .label("Find a VM, host or storage repository")
                            .placeholder("Find a VM, host or storage repository")
                            .on_edit(|s: &mut State| {
                                &mut s.alarms_workspace.as_mut().unwrap().filter
                            })
                            .build(k),
                    );
                    let query = w.filter.text.to_lowercase();
                    let mut count = 0;
                    for (index, p) in w
                        .initial
                        .details
                        .alarm_policies
                        .iter()
                        .enumerate()
                        .filter(|(_, p)| {
                            label(&w.initial, p).to_lowercase().contains(&query)
                                || p.target.uuid().to_string().contains(&query)
                        })
                        .take(100)
                    {
                        count += 1;
                        body = body.child(caption(label(&w.initial, p), k).wrap()).child(
                            Button::new("Configure alarms")
                                .on_press(move |s: &mut State| select(s, index))
                                .build(k)
                                .key(format!("alarm-resource-{index}")),
                        );
                    }
                    if count == 0 {
                        body = body.child(
                            caption("No inspectable alarm resources match this search", k).wrap(),
                        );
                    }
                    if count == 100 {
                        body = body.child(
                            caption("Showing the first 100 matches; refine the search", k).wrap(),
                        );
                    }
                }
            }
            Page::Email => {
                if let Some(error) = w
                    .initial
                    .details
                    .alarm_email
                    .as_ref()
                    .and_then(|e| e.error.as_ref())
                {
                    body = body.child(caption(error, k).wrap());
                }
                body = body
                    .child(caption("Destination email (empty disables delivery)", k).wrap())
                    .child(
                        TextInput::new(&w.destination)
                            .label("Destination email")
                            .on_edit(|s: &mut State| {
                                &mut s.alarms_workspace.as_mut().unwrap().destination
                            })
                            .build(k)
                            .key("alarm-email"),
                    )
                    .child(caption("SMTP relay host[:port] (empty uses host default)", k).wrap())
                    .child(
                        TextInput::new(&w.relay)
                            .label("SMTP relay")
                            .on_edit(|s: &mut State| {
                                &mut s.alarms_workspace.as_mut().unwrap().relay
                            })
                            .build(k)
                            .key("alarm-relay"),
                    );
                for (code, name) in [
                    ("en-US", "English"),
                    ("ja-JP", "Japanese"),
                    ("zh-CN", "Simplified Chinese"),
                ] {
                    body = body.child(
                        Button::new(name)
                            .disabled(w.language == code)
                            .on_press(move |s: &mut State| {
                                s.alarms_workspace.as_mut().unwrap().language = code.into()
                            })
                            .build(k),
                    );
                }
                body=body.child(caption("Email language applies to host-generated alarm messages. Existing SMTP credentials and sender settings are preserved.",k).wrap());
            }
            Page::Refresh => {
                body=body.child(caption("Request the existing host perfmon service to refresh its alarm settings. This does not install or restart a service.",k).wrap());
                for host in w.initial.hosts.values() {
                    body = body
                        .child(caption(format!("{} ({})", host.name, host.id), k).wrap_anywhere());
                }
            }
        }
    }
    if let Some(error) = &w.error {
        body = body.child(
            caption(error, k)
                .wrap()
                .color(k.c.danger.icon)
                .role("alert"),
        );
    }
    let target = observed(w).map(|o| o.target());
    let denial = w.review.as_ref().zip(target).and_then(|(op, target)| {
        let p = crate::session_permissions_view::assess(
            s,
            &ResourceAction::Alarm(op.clone()).permission_requirements(target),
        );
        p.decision().is_denied().then(|| p.explanation())
    });
    if let Some(reason) = &denial {
        body = body.child(caption(reason, k).wrap());
    }
    if s.demo {
        body =
            body.child(caption("Synthetic preview · no host changes can be submitted", k).wrap());
    }
    let mut controls = row().gap(6.0).flex_wrap().child(
        Button::new("Close")
            .on_press(|s: &mut State| s.alarms_workspace = None)
            .build(k),
    );
    if w.review.is_some() || w.policy.is_some() {
        controls = controls.child(
            Button::new("Back")
                .on_press(|s: &mut State| {
                    let w = s.alarms_workspace.as_mut().unwrap();
                    if w.review.take().is_none() {
                        w.policy = None;
                        w.target = None;
                    }
                    w.error = None;
                })
                .build(k),
        );
    }
    if w.review.is_some() {
        controls = controls.child(
            Button::new("Confirm change")
                .disabled(!current(s, w) || denial.is_some())
                .on_press(submit)
                .build(k)
                .key("alarm-confirm"),
        );
    } else if w.page != Page::Policies || w.policy.is_some() {
        controls = controls.child(
            Button::new("Review")
                .on_press(review)
                .build(k)
                .key("alarm-review"),
        );
    }
    controls = controls.child(Button::new("Reload settings").on_press(open).build(k));
    el().w_full()
        .h_full()
        .absolute(0.0, 0.0)
        .center()
        .fill(rune::Color::BLACK.alpha(0.5))
        .role("dialog")
        .label("Performance alarms and email")
        .modal(false, |s: &mut State| s.alarms_workspace = None)
        .child(
            col()
                .w((s.window_width - 32.0).clamp(280.0, 780.0))
                .h((s.window_height - 56.0).clamp(280.0, 820.0))
                .padding(18.0)
                .gap(12.0)
                .radius(10.0)
                .fill(k.c.bg_base)
                .child(body.grow(1.0).min_h(0.0).scroll(rune::ScrollSpec {
                    stick_bottom: false,
                }))
                .child(controls),
        )
}
pub(super) fn scene(s: &mut State) {
    demo(s);
    let mut inventory = (**s.inventory.as_ref().unwrap()).clone();
    let host = *inventory.hosts.keys().next().unwrap();
    inventory.details.alarm_policies.push(AlarmPolicy {
        target: ResourceTarget::Host(host),
        control_domain_host: None,
        revision: "0".repeat(64),
        rules: vec![AlarmRule {
            metric: AlarmMetric::Cpu,
            threshold: "0.8".into(),
            duration_seconds: 60,
            repeat_seconds: 3600,
        }],
        customized: vec![],
        other_variables: 1,
        error: None,
    });
    s.inventory = Some(Arc::new(inventory));
    open(s);
    select(s, 0);
}

pub(super) fn review_scene(s: &mut State) {
    scene(s);
    review(s);
}
pub(super) fn email_scene(s: &mut State) {
    scene(s);
    let w = s.alarms_workspace.as_mut().unwrap();
    w.page = Page::Email;
    Arc::make_mut(&mut w.initial).details.alarm_email =
        Some(xenadmin_core::AlarmEmailConfiguration {
            revision: "0".repeat(64),
            settings: Some(AlarmEmailSettings {
                destination: "operator@example.invalid".into(),
                relay: "smtp.example.invalid:25".into(),
                language: "en-US".into(),
            }),
            error: None,
        });
    w.destination = TextEdit::new("operator@example.invalid");
    w.relay = TextEdit::new("smtp.example.invalid:25");
}

#[cfg(test)]
mod tests {
    use super::*;
    fn state() -> State {
        let mut s = init();
        scene(&mut s);
        s
    }
    fn runtime(mut s: State, size: (f32, f32)) -> rune::Runtime<State> {
        s.window_width = size.0;
        s.window_height = size.1;
        let mut r = rune::Runtime::new(
            s,
            view,
            size,
            rune::Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        settle(&mut r);
        r
    }
    fn settle(r: &mut rune::Runtime<State>) {
        for _ in 0..30 {
            r.frame(1.0 / 60.0);
        }
    }
    fn click(r: &mut rune::Runtime<State>, key: &str) {
        r.update(|_, tx| tx.scroll_into_view(key));
        settle(r);
        let (x, y) = r.find(key).unwrap();
        assert!(x >= 0.0 && x <= r.state.window_width && y >= 0.0 && y <= r.state.window_height);
        r.pointer_down(x, y);
        r.pointer_up(x, y);
        settle(r);
    }
    #[test]
    fn alarm_review_converts_percent_and_retains_original_pool_at_both_widths() {
        for size in [(1440.0, 900.0), (513.0, 652.0)] {
            let mut r = runtime(state(), size);
            click(&mut r, "alarm-review");
            let w = r.state.alarms_workspace.as_ref().unwrap();
            let pool = w.initial.pool_id;
            let review = w.review.clone().unwrap();
            assert!(
                matches!(&review,AlarmOperation::SetPolicy(change) if change.rule.as_ref().unwrap().threshold=="0.8")
            );
            Arc::make_mut(r.state.inventory.as_mut().unwrap()).pool_id =
                PoolId::from(uuid::Uuid::new_v4());
            click(&mut r, "alarm-confirm");
            assert!(r.state.resource_operations.is_empty());
            assert_eq!(
                r.state.alarms_workspace.as_ref().unwrap().initial.pool_id,
                pool
            );
            assert_eq!(
                r.state.alarms_workspace.as_ref().unwrap().review,
                Some(review)
            );
        }
    }
    #[test]
    fn invalid_threshold_and_custom_source_refuse_review_but_explicit_disable_is_captured() {
        let mut s = state();
        s.alarms_workspace.as_mut().unwrap().threshold = TextEdit::new("101");
        review(&mut s);
        assert!(s.alarms_workspace.as_ref().unwrap().review.is_none());
        let w = s.alarms_workspace.as_mut().unwrap();
        w.threshold = TextEdit::new("70");
        w.policy.as_mut().unwrap().customized.push(AlarmMetric::Cpu);
        review(&mut s);
        assert!(s.alarms_workspace.as_ref().unwrap().review.is_none());
        s.alarms_workspace.as_mut().unwrap().enabled = false;
        review(&mut s);
        assert!(
            matches!(&s.alarms_workspace.as_ref().unwrap().review,Some(AlarmOperation::SetPolicy(change))if change.rule.is_none())
        );
    }
    #[test]
    fn alarm_email_and_refresh_reviews_explain_configuration_and_delivery_separately() {
        let mut s = state();
        let w = s.alarms_workspace.as_mut().unwrap();
        w.page = Page::Refresh;
        let op = intent(w).unwrap();
        let lines = review_lines(w, &op).join(" ");
        assert!(lines.contains("does not confirm alarm evaluation or email delivery"));
        let op = AlarmOperation::SetEmail {
            expected_revision: "0".repeat(64),
            settings: AlarmEmailSettings {
                destination: String::new(),
                relay: String::new(),
                language: "en-US".into(),
            },
        };
        let lines = review_lines(w, &op).join(" ");
        assert!(lines.contains("Disable pool alarm email"));
        assert!(lines.contains("does not send a test message"));
    }
}
