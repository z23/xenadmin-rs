use super::*;
use xenadmin_core::{
    HostPowerConfiguration, HostPowerOperation, HostPowerPassword, HostPowerSettings,
    ObservedResource, OperationId, ResourceAction, ResourceCommand, ResourceTarget,
};
use zeroize::Zeroize;
pub(super) struct Editor {
    initial: Arc<Inventory>,
    target: ResourceTarget,
    observed: Option<ObservedResource>,
    expected: Option<HostPowerConfiguration>,
    mode: u8,
    address: TextEdit,
    username: TextEdit,
    password: TextEdit,
    epoch: u64,
    review_epoch: u64,
    review: Option<HostPowerOperation>,
    error: Option<String>,
}
impl Drop for Editor {
    fn drop(&mut self) {
        self.password.text.zeroize();
    }
}
pub(super) fn open(s: &mut State, target: ResourceTarget) {
    let Some(initial) = s.inventory.clone() else {
        return;
    };
    let ResourceTarget::Host(host) = target else {
        return;
    };
    let expected = initial
        .details
        .hosts
        .get(&host)
        .and_then(|h| h.power_configuration.clone());
    let (mode, address, username) = match expected.as_ref().map(|c| &c.settings) {
        Some(HostPowerSettings::Ipmi { address, username }) => {
            (2, address.to_string(), username.clone())
        }
        Some(HostPowerSettings::WakeOnLan) => (1, String::new(), String::new()),
        _ => (0, String::new(), String::new()),
    };
    let observed = s
        .snapshot
        .as_ref()
        .and_then(|s| s.observe_resource(target).ok());
    s.host_workspace = None;
    s.host_power = Some(Editor {
        initial,
        target,
        observed,
        expected,
        mode,
        address: TextEdit::new(address),
        username: TextEdit::new(username),
        password: secure_field(),
        epoch: 0,
        review_epoch: 0,
        review: None,
        error: None,
    });
}
fn change_mode(s: &mut State, mode: u8) {
    if let Some(e) = &mut s.host_power {
        e.mode = mode;
        e.epoch += 1;
        e.review = None;
        e.error = None;
        e.password.text.zeroize();
        e.password = secure_field();
    }
}
fn settings(e: &Editor) -> Result<HostPowerSettings, String> {
    Ok(match e.mode {
        0 => HostPowerSettings::Disabled,
        1 => HostPowerSettings::WakeOnLan,
        _ => {
            let address = e
                .address
                .text
                .trim()
                .parse::<std::net::IpAddr>()
                .map_err(|_| "Enter a valid BMC IP address")?;
            if address.is_unspecified() || address.is_multicast() {
                return Err("Enter a unicast BMC IP address".into());
            }
            if e.username.text.is_empty()
                || e.username.text.len() > 255
                || e.username.text.chars().any(char::is_control)
            {
                return Err("Enter a valid BMC username".into());
            }
            HostPowerPassword::new(e.password.text.clone())
                .map_err(|_| "Enter a new BMC password")?;
            HostPowerSettings::Ipmi {
                address,
                username: e.username.text.clone(),
            }
        }
    })
}
fn review(s: &mut State) {
    let Some(e) = &mut s.host_power else { return };
    let result = (|| {
        let expected = e.expected.clone().ok_or(
            "Current power-on settings are unavailable or use an unsupported custom plugin",
        )?;
        let settings = settings(e)?;
        if !settings.needs_password() && expected.settings == settings {
            return Err("Choose a different power-on mode".into());
        }
        Ok(HostPowerOperation::Configure { expected, settings })
    })();
    match result {
        Ok(op) => {
            e.review = Some(op);
            e.review_epoch = e.epoch;
            e.error = None
        }
        Err(error) => e.error = Some(error),
    }
}
fn cleanup(s: &mut State, id: OperationId) {
    let Some(e) = &mut s.host_power else { return };
    let Some(r) = s.resource_rows.iter().find(|r| {
        r.id == id && r.target == e.target && r.pool == e.initial.pool_id && r.state.terminal()
    }) else {
        return;
    };
    let Some(secret) = r.host_power_secret.as_ref().filter(|v| !v.cleaned) else {
        return;
    };
    e.review = Some(HostPowerOperation::CleanupSecret {
        operation: id,
        secret: secret.secret,
    });
    e.review_epoch = e.epoch;
    e.password.text.zeroize();
    e.password = secure_field();
    e.error = None;
}
fn current(s: &State, e: &Editor) -> bool {
    s.can_mutate()
        && e.epoch == e.review_epoch
        && e.observed.is_some()
        && e.observed
            == s.snapshot
                .as_ref()
                .and_then(|s| s.observe_resource(e.target).ok())
        && match &e.review {
            Some(HostPowerOperation::Configure { expected, .. }) => {
                let ResourceTarget::Host(id) = e.target else {
                    return false;
                };
                s.inventory
                    .as_ref()
                    .and_then(|i| i.details.hosts.get(&id))
                    .and_then(|d| d.power_configuration.as_ref())
                    == Some(expected)
            }
            Some(HostPowerOperation::CleanupSecret { .. }) => true,
            None => false,
        }
}
fn submit(s: &mut State) {
    let result = (|| {
        let e = s.host_power.as_ref().ok_or(xenadmin_core::Error::Stale)?;
        if !current(s, e) {
            return Err(xenadmin_core::Error::Stale);
        }
        let permission = session_permissions_view::assess(
            s,
            &e.review
                .as_ref()
                .ok_or(xenadmin_core::Error::Stale)?
                .permission_requirements(),
        );
        if permission.decision().is_denied() {
            return Err(xenadmin_core::Error::Api("RBAC_PERMISSION_DENIED".into()));
        }
        let connection = s.connection.as_ref().ok_or(xenadmin_core::Error::Closed)?;
        let host = e.observed.ok_or(xenadmin_core::Error::Stale)?;
        match e.review.clone().ok_or(xenadmin_core::Error::Stale)? {
            HostPowerOperation::Configure { expected, settings } => {
                let password = if settings.needs_password() {
                    Some(HostPowerPassword::new(e.password.text.clone())?)
                } else {
                    None
                };
                connection.configure_host_power(
                    OperationId::new(),
                    host,
                    expected,
                    settings,
                    password,
                )
            }
            operation @ HostPowerOperation::CleanupSecret { .. } => connection.submit_resource(
                OperationId::new(),
                ResourceCommand::HostPower { host, operation },
            ),
        }
    })();
    if let Some(e) = &mut s.host_power {
        e.password.text.zeroize();
        e.password = secure_field();
        e.review = None;
        e.epoch += 1;
    }
    match result {
        Ok(op) => {
            if let Some(wake) = &s.waker {
                op.notify_on_change(wake.clone());
            }
            s.resource_operations.push(op);
            s.host_power = None;
        }
        Err(error) => {
            if let Some(e) = &mut s.host_power {
                e.error = Some(format!("{error}. Enter the password and review again."));
            }
        }
    }
}
fn summary(c: &HostPowerConfiguration) -> String {
    match &c.settings {
        HostPowerSettings::Disabled => "Disabled".into(),
        HostPowerSettings::WakeOnLan => "Wake-on-LAN".into(),
        HostPowerSettings::Ipmi { address, username } => format!(
            "IPMI at {address}, user {username}, Secret {}",
            c.secret
                .map(|v| v.to_string())
                .unwrap_or_else(|| "unknown".into())
        ),
    }
}
fn requested(settings: &HostPowerSettings) -> String {
    match settings {
        HostPowerSettings::Disabled => "Disabled".into(),
        HostPowerSettings::WakeOnLan => "Wake-on-LAN".into(),
        HostPowerSettings::Ipmi { address, username } => {
            format!("IPMI at {address}, user {username}, with a new password")
        }
    }
}
pub(super) fn dialog(s: &State, k: &Kit) -> El {
    let Some(e) = &s.host_power else { return el() };
    let permission = e
        .review
        .as_ref()
        .map(|op| session_permissions_view::assess(s, &op.permission_requirements()));
    let denied = permission
        .as_ref()
        .is_some_and(|p| p.decision().is_denied());
    let label = if let ResourceTarget::Host(id) = e.target {
        e.initial
            .hosts
            .get(&id)
            .map(|h| h.name.as_str())
            .unwrap_or("Unknown host")
    } else {
        "Unknown host"
    };
    let mut body = col()
        .gap(12.0)
        .child(heading("Host power-on settings", k))
        .child(
            caption(
                format!(
                    "{label}\nHost {}\nPool {}",
                    e.target.uuid(),
                    e.initial.pool_id
                ),
                k,
            )
            .wrap_anywhere(),
        );
    if let Some(op) = &e.review {
        match op {
            HostPowerOperation::Configure { expected, settings } => {
                body=body.child(caption(format!("Current: {}\nRequested: {}",summary(expected),requested(settings)),k).wrap_anywhere()).child(caption("The host stores the new configuration and removes its previous password Secret. Shared Secret consumers are checked first. IPMI creates a new server Secret; its clear password is never saved in application history. This does not power on the host or verify BMC reachability.",k).wrap()).child(caption("Lost replies remain uncertain. No password or configuration write is automatically repeated. Use Activity to recheck evidence and review retained Secret cleanup separately.",k).wrap());
            }
            HostPowerOperation::CleanupSecret { operation, secret } => {
                body=body.child(caption(format!("Remove retained Secret {secret}\nOriginal operation {operation}"),k).wrap_anywhere()).child(caption("Cleanup requires the original write to be finished or never dispatched, verified ownership, and no host or storage consumers. It never changes power-on settings. Missing evidence prevents deletion.",k).wrap());
            }
        }
    } else {
        body = body.child(
            caption(
                format!(
                    "Current: {}",
                    e.expected.as_ref().map(summary).unwrap_or_else(|| {
                        "Unavailable or unsupported custom configuration".into()
                    })
                ),
                k,
            )
            .wrap_anywhere(),
        );
        let mut modes = row().gap(8.0).flex_wrap();
        for (mode, label) in [(0, "Disabled"), (1, "Wake-on-LAN"), (2, "IPMI")] {
            modes = modes.child(
                Button::new(label)
                    .disabled(e.mode == mode)
                    .on_press(move |s: &mut State| change_mode(s, mode))
                    .build(k)
                    .key(format!("host-power-mode-{mode}")),
            );
        }
        body = body.child(modes);
        if e.mode == 2 {
            body=body.child(TextInput::new(&e.address).label("BMC IP address").on_edit(|s:&mut State|{let e=s.host_power.as_mut().unwrap();e.epoch+=1;&mut e.address}).build(k).key("host-power-address")).child(TextInput::new(&e.username).label("BMC username").on_edit(|s:&mut State|{let e=s.host_power.as_mut().unwrap();e.epoch+=1;&mut e.username}).build(k).key("host-power-username")).child(TextInput::new(&e.password).secure().label("New BMC password").on_edit(|s:&mut State|{let e=s.host_power.as_mut().unwrap();e.epoch+=1;&mut e.password}).build(k).key("host-power-password")).child(caption("Uses the XCP-ng IPMI plugin with LAN-plus. IPMI plugin and ipmitool availability are supplied by the host package; saving settings is not a power-on test. Legacy iLO/DRAC and custom plugin maps are not edited here.",k).wrap());
        }
        for record in s
            .resource_rows
            .iter()
            .rev()
            .filter(|r| {
                r.pool == e.initial.pool_id
                    && r.target == e.target
                    && matches!(r.action, ResourceAction::HostPower(HostPowerOperation::Configure { .. }))
                    && r.state != xenadmin_core::OperationState::Succeeded
                    && !s.resource_rows.iter().any(|cleanup| matches!(cleanup.action, ResourceAction::HostPower(HostPowerOperation::CleanupSecret { operation, .. }) if operation == r.id) && cleanup.state == xenadmin_core::OperationState::Succeeded)
                    && r.host_power_secret.as_ref().is_some_and(|v| !v.cleaned)
            })
            .take(20)
        {
            let id = record.id;
            body = body.child(
                caption(
                    format!(
                        "{} · {:?}\n{}",
                        record.action.label(),
                        record.state,
                        record.detail
                    ),
                    k,
                )
                .wrap_anywhere(),
            );
            if record.state.terminal() {
                body = body.child(
                    Button::new("Review unused Secret cleanup")
                        .on_press(move |s: &mut State| cleanup(s, id))
                        .build(k)
                        .key(format!("host-power-cleanup-{id}")),
                );
            }
        }
    }
    if s.demo {
        body =
            body.child(caption("Synthetic preview · no host changes can be submitted", k).wrap());
    }
    if let Some(error) = &e.error {
        body = body.child(
            caption(error, k)
                .color(k.c.danger.icon)
                .wrap()
                .role("alert"),
        );
    }
    if e.review.is_some() && !s.demo && !current(s, e) {
        body = body.child(
            caption(
                "Connection or reviewed host settings changed. Reopen this editor before applying.",
                k,
            )
            .wrap(),
        );
    }
    let mut footer = row().gap(8.0).flex_wrap().justify_end().child(
        Button::new("Close")
            .variant(ButtonVariant::Outline)
            .on_press(|s: &mut State| s.host_power = None)
            .build(k),
    );
    footer = if e.review.is_some() {
        footer
            .child(
                Button::new("Back")
                    .on_press(|s: &mut State| {
                        if let Some(e) = &mut s.host_power {
                            e.review = None;
                            e.error = None;
                        }
                    })
                    .build(k)
                    .key("host-power-back"),
            )
            .child(
                Button::new("Confirm host change")
                    .variant(ButtonVariant::Danger)
                    .disabled(!current(s, e) || denied)
                    .on_press(submit)
                    .build(k)
                    .key("host-power-confirm"),
            )
    } else {
        footer.child(
            Button::new("Review")
                .on_press(review)
                .build(k)
                .key("host-power-review"),
        )
    };
    el().w_full()
        .h_full()
        .absolute(0.0, 0.0)
        .center()
        .fill(rune::Color::BLACK.alpha(0.5))
        .role("dialog")
        .label("Host power-on settings")
        .modal(false, |s: &mut State| s.host_power = None)
        .child(
            col()
                .w((s.window_width - 32.0).clamp(280.0, 680.0))
                .h((s.window_height - 56.0).clamp(280.0, 780.0))
                .padding(20.0)
                .gap(16.0)
                .radius(10.0)
                .fill(k.c.bg_base)
                .child(body.grow(1.0).min_h(0.0).scroll(rune::ScrollSpec {
                    stick_bottom: false,
                }))
                .child(session_permissions_view::footer(
                    footer,
                    permission.as_ref(),
                    (s.window_width - 32.0).clamp(280.0, 680.0) - 40.0,
                    k,
                )),
        )
}
pub(super) fn scene(s: &mut State) {
    demo(s);
    s.page = Page::Hosts;
    let i = Arc::make_mut(s.inventory.as_mut().unwrap());
    let id = *i.hosts.keys().next().unwrap();
    i.details.hosts.entry(id).or_default().power_configuration = Some(HostPowerConfiguration {
        settings: HostPowerSettings::Disabled,
        secret: None,
    });
    s.rebuild();
    open(s, ResourceTarget::Host(id));
    let e = s.host_power.as_mut().unwrap();
    e.mode = 2;
    e.address = TextEdit::new("192.0.2.60");
    e.username = TextEdit::new("fixture-admin");
    e.password = secure_field();
    e.password.text = "synthetic-password-not-shown".into();
    review(s);
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn review_has_private_password_and_captured_target_at_both_widths() {
        for size in [(1440.0, 900.0), (513.0, 652.0)] {
            let mut s = init();
            scene(&mut s);
            s.window_width = size.0;
            s.window_height = size.1;
            let target = s.host_power.as_ref().unwrap().target;
            let mut r = rune::Runtime::new(
                s,
                view,
                size,
                rune::Env {
                    system_dark: true,
                    scale: 1.0,
                },
            );
            for _ in 0..30 {
                r.frame(1.0 / 60.0);
            }
            let (x, y) = r.find("host-power-confirm").unwrap();
            assert!(x > 0.0 && x < size.0 && y > 0.0 && y < size.1);
            r.pointer_down(x, y);
            r.pointer_up(x, y);
            r.frame(1.0 / 60.0);
            assert_eq!(r.state.host_power.as_ref().unwrap().target, target);
            assert!(r.state.resource_operations.is_empty());
            assert!(r.state.host_power.as_ref().unwrap().review.is_some());
        }
    }
    #[test]
    fn configuration_review_refuses_invalid_inputs_and_unknown_configuration() {
        let mut s = init();
        scene(&mut s);
        s.host_power.as_mut().unwrap().review = None;
        for address in ["not an address", "0.0.0.0", "239.0.0.1"] {
            s.host_power.as_mut().unwrap().address = TextEdit::new(address);
            review(&mut s);
            assert!(s.host_power.as_ref().unwrap().review.is_none());
        }
        s.host_power.as_mut().unwrap().address = TextEdit::new("192.0.2.60");
        s.host_power.as_mut().unwrap().expected = None;
        review(&mut s);
        assert!(s.host_power.as_ref().unwrap().review.is_none());
    }
    #[test]
    fn switching_mode_discards_pending_password_and_review() {
        let mut s = init();
        scene(&mut s);
        change_mode(&mut s, 1);
        let e = s.host_power.as_ref().unwrap();
        assert!(e.password.text.is_empty());
        assert!(e.review.is_none());
        assert_eq!(settings(e).unwrap(), HostPowerSettings::WakeOnLan);
    }
}
