use super::*;
use xenadmin_core::{
    HostOperation, HostPowerOnMode, MemoryLimits, ObservedResource, OperationId, ResourceCommand,
    ResourceTarget,
};
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Browse,
    Maintenance,
    Evacuate,
    Exit,
    Reboot,
    Shutdown,
    Restart,
    PowerOn,
    WakeOnLan,
    DisablePowerOn,
    Memory,
    Logging,
    Multipath,
    Autostart,
    Forget,
}
impl Kind {
    fn title(self) -> &'static str {
        match self {
            Self::Browse => "Host administration",
            Self::Maintenance => "Enter maintenance",
            Self::Evacuate => "Evacuate guests",
            Self::Exit => "Exit maintenance",
            Self::Reboot => "Reboot host",
            Self::Shutdown => "Shut down host",
            Self::Restart => "Restart toolstack",
            Self::PowerOn => "Power on host",
            Self::WakeOnLan => "Configure Wake-on-LAN",
            Self::DisablePowerOn => "Disable remote power-on",
            Self::Memory => "Change dom0 memory",
            Self::Logging => "Remote logging",
            Self::Multipath => "Change multipathing",
            Self::Autostart => "Pool VM autostart",
            Self::Forget => "Forget dead host",
        }
    }
}
pub(super) struct Workspace {
    initial: Arc<Inventory>,
    target: ResourceTarget,
    observed: Option<ObservedResource>,
    kind: Kind,
    value: TextEdit,
    acknowledged: bool,
    review: Option<HostOperation>,
    error: Option<String>,
}
pub(super) fn button(s: &State, k: &Kit) -> El {
    if s.page != Page::Hosts && console_view::host(s).is_none() {
        return el();
    }
    Button::new("Host administration…")
        .on_press(open)
        .build(k)
        .key("host-admin-open")
}
pub(super) fn open(s: &mut State) {
    let Some(target @ ResourceTarget::Host(_)) = resource_admin::selected_target(s) else {
        return;
    };
    open_target(s, target, Kind::Browse);
}
pub(super) fn pool(s: &mut State) {
    let Some(i) = &s.inventory else {
        return;
    };
    open_target(s, ResourceTarget::Pool(i.pool_id), Kind::Autostart);
}
fn open_target(s: &mut State, target: ResourceTarget, kind: Kind) {
    let Some(initial) = s.inventory.clone() else {
        return;
    };
    let observed = s
        .snapshot
        .as_ref()
        .and_then(|snap| snap.observe_resource(target).ok());
    s.resource_editor = None;
    s.host_workspace = Some(Workspace {
        initial,
        target,
        observed,
        kind,
        value: TextEdit::new(""),
        acknowledged: false,
        review: None,
        error: None,
    });
}
fn select(s: &mut State, kind: Kind) {
    if let Some(w) = &mut s.host_workspace {
        w.kind = kind;
        w.review = None;
        w.error = None;
        w.acknowledged = false;
        let value = if let ResourceTarget::Host(id) = w.target {
            let host = w.initial.details.hosts.get(&id);
            match kind {
                Kind::Logging => host
                    .and_then(|h| h.syslog_destination.clone())
                    .unwrap_or_default(),
                Kind::Memory => host
                    .and_then(|h| h.control_domain)
                    .and_then(|id| w.initial.details.vms.get(&id))
                    .and_then(|v| v.memory_static_max)
                    .map(|n| (n / (1024 * 1024)).to_string())
                    .unwrap_or_default(),
                _ => String::new(),
            }
        } else {
            String::new()
        };
        w.value = TextEdit::new(value);
    }
}
fn operation(w: &Workspace) -> Result<HostOperation, String> {
    let missing =
        || "Required current host data is unavailable; refresh before editing".to_string();
    if w.kind == Kind::Autostart {
        let expected = w
            .initial
            .details
            .pool
            .as_ref()
            .and_then(|p| p.auto_poweron)
            .ok_or_else(missing)?;
        return Ok(HostOperation::SetPoolAutostart {
            expected,
            enabled: !expected,
        });
    }
    let ResourceTarget::Host(id) = w.target else {
        return Err(missing());
    };
    let host = w.initial.hosts.get(&id).ok_or_else(missing)?;
    let d = w.initial.details.hosts.get(&id).ok_or_else(missing)?;
    Ok(match w.kind {
        Kind::Maintenance => HostOperation::EnterMaintenance {
            expected_enabled: host.enabled,
            resident_vms: w.initial.host_resident_guests(id),
        },
        Kind::Evacuate => HostOperation::Evacuate {
            resident_vms: w.initial.host_resident_guests(id),
        },
        Kind::Exit => HostOperation::ExitMaintenance,
        Kind::Reboot => HostOperation::Reboot {
            expected_boot_time: d.boot_time.clone().ok_or_else(missing)?,
        },
        Kind::Shutdown => HostOperation::Shutdown {
            expected_boot_time: d.boot_time.clone().ok_or_else(missing)?,
        },
        Kind::Restart => HostOperation::RestartToolstack {
            expected_agent_start_time: d.agent_start_time.clone().ok_or_else(missing)?,
        },
        Kind::PowerOn => HostOperation::PowerOn,
        Kind::WakeOnLan | Kind::DisablePowerOn => HostOperation::ConfigurePowerOn {
            expected_mode: d.power_on_mode.clone().ok_or_else(missing)?,
            mode: if w.kind == Kind::WakeOnLan {
                HostPowerOnMode::WakeOnLan
            } else {
                HostPowerOnMode::Disabled
            },
        },
        Kind::Memory => {
            let control_domain = d.control_domain.ok_or_else(missing)?;
            let vm = w
                .initial
                .details
                .vms
                .get(&control_domain)
                .ok_or_else(missing)?;
            let memory = w
                .value
                .text
                .parse::<u64>()
                .ok()
                .and_then(|n| n.checked_mul(1024 * 1024))
                .filter(|n| *n > 0 && *n <= i64::MAX as u64)
                .ok_or("Enter a positive whole MiB memory value")?;
            let total = d
                .metrics
                .and_then(|id| w.initial.details.host_metrics.get(&id))
                .and_then(|m| m.memory_total)
                .ok_or_else(missing)?;
            if memory > total {
                return Err("Memory exceeds the host's physical memory".into());
            }
            HostOperation::SetDom0Memory {
                control_domain,
                expected: MemoryLimits {
                    static_min: vm.memory_static_min.ok_or_else(missing)?,
                    dynamic_min: vm.memory_dynamic_min.ok_or_else(missing)?,
                    dynamic_max: vm.memory_dynamic_max.ok_or_else(missing)?,
                    static_max: vm.memory_static_max.ok_or_else(missing)?,
                },
                memory,
            }
        }
        Kind::Logging => {
            if !d.syslog_known {
                return Err(missing());
            }
            HostOperation::SetSyslog {
                expected_destination: d.syslog_destination.clone(),
                destination: (!w.value.text.trim().is_empty()).then(|| w.value.text.trim().into()),
            }
        }
        Kind::Multipath => {
            let expected = d.multipathing.ok_or_else(missing)?;
            HostOperation::SetMultipath {
                expected,
                enabled: !expected,
            }
        }
        Kind::Forget => HostOperation::ForgetDeadHost {
            verified_power_off: w.acknowledged,
        },
        Kind::Browse | Kind::Autostart => return Err("Choose a host operation".into()),
    })
}
fn review(s: &mut State) {
    let Some(w) = &mut s.host_workspace else {
        return;
    };
    let result = operation(w).and_then(|op| {
        let a = op.availability(&w.initial, w.target);
        if a.is_available() {
            Ok(op)
        } else {
            Err(a.explanation())
        }
    });
    match result {
        Ok(op) => {
            w.review = Some(op);
            w.error = None;
        }
        Err(e) => w.error = Some(e),
    }
}
fn current(s: &State, w: &Workspace) -> bool {
    s.can_mutate()
        && w.observed.is_some()
        && w.observed
            == s.snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.observe_resource(w.target).ok())
        && w.review.as_ref().is_some_and(|op| {
            s.inventory
                .as_ref()
                .is_some_and(|i| op.availability(i, w.target).is_available())
        })
}
fn submit(s: &mut State) {
    let result = (|| {
        let w = s
            .host_workspace
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
                ResourceCommand::Host {
                    target: w.observed.ok_or(xenadmin_core::Error::Stale)?,
                    operation: w.review.clone().ok_or(xenadmin_core::Error::Stale)?,
                },
            )
    })();
    match result {
        Ok(operation) => {
            if let Some(waker) = &s.waker {
                operation.notify_on_change(waker.clone());
            }
            s.resource_operations.push(operation);
            s.host_workspace = None;
        }
        Err(e) => {
            if let Some(w) = &mut s.host_workspace {
                w.error = Some(e.to_string());
            }
        }
    }
}
fn lines(w: &Workspace, op: &HostOperation) -> Vec<String> {
    let mut lines = vec![
        format!("Pool {}", w.initial.pool_id),
        format!("Target {}", w.target.uuid()),
    ];
    match op{
 HostOperation::EnterMaintenance{resident_vms,..}|HostOperation::Evacuate{resident_vms}=>{lines.push("Scheduling will be disabled for maintenance. The host selects compatible migration destinations after checking every reviewed guest. No guest shutdown, HA reduction or coordinator handover is automatic. A failure may leave scheduling disabled or some guests already moved.".into());for id in resident_vms{lines.push(format!("Move guest: {} ({id})",w.initial.vms.get(id).map(|v|v.name.as_str()).unwrap_or("Unknown guest")));}},
 HostOperation::ExitMaintenance=>lines.push("Enable scheduling, then clear the maintenance marker. No evacuated guest is automatically moved back.".into()),
 HostOperation::Reboot{..}|HostOperation::Shutdown{..}=>lines.push("The host must be disabled and empty of running or paused guests. Multi-host pool coordinators require separate handover. Connection loss leaves an uncertain outcome until independent task and host evidence can establish it.".into()),
 HostOperation::RestartToolstack{..}=>lines.push("Restart XAPI only with pool HA disabled and after checking that pool tasks have finished. Management and console connections may disconnect; guests keep running. A newer toolstack start marker is required to verify completion.".into()),
 HostOperation::PowerOn=>lines.push("Use the host's configured remote power-on method. Success requires a fresh report that the host is live.".into()),
 HostOperation::ConfigurePowerOn{expected_mode,mode}=>lines.push(format!("Power-on mode: {expected_mode:?} → {:?}. This editor accepts only disabled or Wake-on-LAN modes with no stored BMC configuration.",mode)),
 HostOperation::SetDom0Memory{control_domain,expected,memory}=>lines.push(format!("Control domain {control_domain}: static maximum {} MiB → {} MiB; dynamic minimum and maximum also become {} MiB. Static minimum remains {} MiB. The host decides whether the requested allocation is possible.",expected.static_max/1048576,memory/1048576,memory/1048576,expected.static_min/1048576)),
 HostOperation::SetSyslog{expected_destination,destination}=>lines.push(format!("Remote logging: {} → {}. Save the destination, then request logging reconfiguration; failure of the second step leaves the saved setting visible as a partial outcome.",expected_destination.as_deref().unwrap_or("Local only"),destination.as_deref().unwrap_or("Local only"))),
 HostOperation::SetMultipath{expected,enabled}=>lines.push(format!("Multipathing: {expected} → {enabled}. Requires an evacuated maintenance host with every storage connection detached. Apply the standard dmp handler. Storage is not automatically detached or reconnected.")),
 HostOperation::SetPoolAutostart{expected,enabled}=>lines.push(format!("Pool-wide VM autostart: {expected} → {enabled}. This setting applies to every host in the pool, together with each VM's own auto-start flag. It does not start VMs now or alter per-VM flags.")),
 HostOperation::ForgetDeadHost{..}=>lines.push("Remove this dead host's pool record only. You have independently verified it is powered off. It must be offline, disabled, empty, not the coordinator and HA must be disabled. Storage repository records and data are retained; rejoining requires separate recovery.".into()),}
    lines.push("Changes are journaled individually. Stop remaining steps does not cancel in-flight host work or undo completed changes. Use Activity for retained history and Pool administration for unknown-outcome recovery.".into());
    lines
}
pub(super) fn dialog(s: &State, k: &Kit) -> El {
    let Some(w) = &s.host_workspace else {
        return el();
    };
    let label = match w.target {
        ResourceTarget::Host(id) => w
            .initial
            .hosts
            .get(&id)
            .map(|h| h.name.as_str())
            .unwrap_or("Unknown host"),
        _ => &w.initial.pool_name,
    };
    let mut body = col()
        .gap(12.0)
        .child(heading(w.kind.title(), k))
        .child(caption(format!("{label}\n{}", w.target.uuid()), k).wrap_anywhere());
    if w.kind == Kind::Browse {
        for kind in [
            Kind::Maintenance,
            Kind::Evacuate,
            Kind::Exit,
            Kind::Reboot,
            Kind::Shutdown,
            Kind::Restart,
            Kind::PowerOn,
            Kind::WakeOnLan,
            Kind::DisablePowerOn,
            Kind::Memory,
            Kind::Logging,
            Kind::Multipath,
            Kind::Forget,
        ] {
            body = body.child(
                Button::new(kind.title())
                    .on_press(move |s: &mut State| select(s, kind))
                    .build(k)
                    .key(format!("host-choice-{}", kind.title())),
            );
        }
        let target = w.target;
        body = body.child(
            Button::new("BMC / power-on settings…")
                .on_press(move |s: &mut State| host_power_view::open(s, target))
                .build(k)
                .key("host-bmc-settings"),
        );
        body=body.child(caption("Host passwords, backup/restore and support-bundle transfers require their dedicated workflows.",k).wrap());
    } else if let Some(op) = &w.review {
        for line in lines(w, op) {
            body = body.child(caption(line, k).wrap_anywhere());
        }
    } else {
        if matches!(w.kind, Kind::Memory | Kind::Logging) {
            body = body.child(
                TextInput::new(&w.value)
                    .label(if w.kind == Kind::Memory {
                        "dom0 memory (MiB)"
                    } else {
                        "Remote syslog hostname or IP (blank for local only)"
                    })
                    .on_edit(|s: &mut State| &mut s.host_workspace.as_mut().unwrap().value)
                    .build(k)
                    .key("host-setting-value"),
            );
        }
        if w.kind == Kind::Forget {
            body = body.child(
                Checkbox::new(w.acknowledged)
                    .label("I independently verified that this host is powered off")
                    .on_toggle(|s: &mut State| {
                        if let Some(w) = &mut s.host_workspace {
                            w.acknowledged = !w.acknowledged;
                        }
                    })
                    .build(k)
                    .key("host-forget-off"),
            );
        }
        match operation(w) {
            Ok(op) => {
                for line in lines(w, &op) {
                    body = body.child(caption(line, k).wrap_anywhere());
                }
                let a = op.availability(&w.initial, w.target);
                if !a.is_available() {
                    body = body.child(caption(a.explanation(), k).wrap());
                }
            }
            Err(e) => body = body.child(caption(e, k).wrap()),
        }
    }
    if s.demo {
        body =
            body.child(caption("Synthetic preview · no host changes can be submitted", k).wrap());
    }
    if let Some(error) = &w.error {
        body = body.child(
            caption(error, k)
                .wrap()
                .color(k.c.danger.icon)
                .role("alert"),
        );
    }
    if w.review.is_some() && !s.demo && !current(s, w) {
        body=body.child(caption("The connection, host or prerequisites changed. Close and reopen this review before applying it.",k).wrap());
    }
    let mut controls = row().gap(8.0).flex_wrap().justify_end().child(
        Button::new("Close")
            .variant(ButtonVariant::Ghost)
            .on_press(|s: &mut State| s.host_workspace = None)
            .build(k),
    );
    if w.kind != Kind::Browse {
        if w.kind != Kind::Autostart || w.review.is_some() {
            controls = controls.child(
                Button::new("Back")
                    .variant(ButtonVariant::Ghost)
                    .on_press(|s: &mut State| {
                        if let Some(w) = &mut s.host_workspace {
                            if w.review.take().is_none() {
                                w.kind = Kind::Browse;
                            }
                            w.error = None;
                        }
                    })
                    .build(k),
            );
        }
        controls = if w.review.is_some() {
            controls.child(
                Button::new("Confirm host change")
                    .variant(ButtonVariant::Danger)
                    .disabled(!current(s, w))
                    .on_press(submit)
                    .build(k)
                    .key("host-confirm"),
            )
        } else {
            controls.child(
                Button::new("Review")
                    .on_press(review)
                    .build(k)
                    .key("host-review"),
            )
        };
    }
    el().w_full()
        .h_full()
        .absolute(0.0, 0.0)
        .center()
        .fill(rune::Color::BLACK.alpha(0.5))
        .role("dialog")
        .label(w.kind.title())
        .modal(false, |s: &mut State| s.host_workspace = None)
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
                .child(controls),
        )
}

pub(super) fn scene(s: &mut State) {
    crate::demo(s);
    s.page = Page::Hosts;
    let i = Arc::make_mut(s.inventory.as_mut().unwrap());
    let id = *i.hosts.keys().next().unwrap();
    let metrics = xenadmin_core::HostMetricsId::from(uuid::Uuid::from_u128(0x9901));
    let control = VmId::from(uuid::Uuid::from_u128(0x9902));
    i.details.host_metrics.insert(
        metrics,
        xenadmin_core::HostMetrics {
            id: metrics,
            live: Some(true),
            memory_total: Some(64 * 1024 * 1024 * 1024),
            memory_free: Some(32 * 1024 * 1024 * 1024),
            last_updated: None,
        },
    );
    i.details.hosts.insert(
        id,
        xenadmin_core::HostDetails {
            allowed_operations: Some(vec![
                "enable".into(),
                "evacuate".into(),
                "shutdown".into(),
                "reboot".into(),
                "power_on".into(),
            ]),
            current_operations: Some(vec![]),
            maintenance_mode: Some(false),
            boot_time: Some("100".into()),
            agent_start_time: Some("110".into()),
            metrics: Some(metrics),
            control_domain: Some(control),
            power_on_mode: Some(String::new()),
            power_on_config_empty: Some(true),
            syslog_known: true,
            multipathing: Some(false),
            ..Default::default()
        },
    );
    i.details.vms.insert(
        control,
        xenadmin_core::VmDetails {
            memory_static_min: Some(1024 * 1024),
            memory_static_max: Some(2 * 1024 * 1024 * 1024),
            memory_dynamic_min: Some(2 * 1024 * 1024 * 1024),
            memory_dynamic_max: Some(2 * 1024 * 1024 * 1024),
            ..Default::default()
        },
    );
    i.details
        .classes
        .insert("task".into(), xenadmin_core::ClassAvailability::Available);
    i.details
        .classes
        .insert("pbd".into(), xenadmin_core::ClassAvailability::Available);
    s.rebuild();
    open_target(s, ResourceTarget::Host(id), Kind::Maintenance);
    review(s);
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
        let (x, y) = r.find(key).unwrap_or_else(|| panic!("missing {key}"));
        assert!(x >= 0.0 && x <= r.state.window_width && y >= 0.0 && y <= r.state.window_height);
        r.pointer_down(x, y);
        r.pointer_up(x, y);
        settle(r);
    }
    #[test]
    fn maintenance_review_keeps_host_and_guest_identity_at_both_widths() {
        for size in [(1440.0, 900.0), (513.0, 652.0)] {
            let mut r = runtime(state(), size);
            let w = r.state.host_workspace.as_ref().unwrap();
            let target = w.target;
            let op = w.review.clone().expect("maintenance review");
            assert!(matches!(op, HostOperation::EnterMaintenance { .. }));
            Arc::make_mut(r.state.inventory.as_mut().unwrap()).pool_id =
                PoolId::from(uuid::Uuid::new_v4());
            click(&mut r, "host-confirm");
            assert_eq!(r.state.host_workspace.as_ref().unwrap().target, target);
            assert_eq!(r.state.host_workspace.as_ref().unwrap().review, Some(op));
            assert!(r.state.resource_operations.is_empty());
        }
    }
    #[test]
    fn memory_editor_rejects_zero_overflow_and_physical_overcommit() {
        let mut s = state();
        select(&mut s, Kind::Memory);
        for invalid in ["0", "-1", "18446744073709551615", "999999"] {
            s.host_workspace.as_mut().unwrap().value = TextEdit::new(invalid);
            review(&mut s);
            assert!(s.host_workspace.as_ref().unwrap().review.is_none());
        }
        s.host_workspace.as_mut().unwrap().value = TextEdit::new("3072");
        review(&mut s);
        assert!(matches!(
            s.host_workspace.as_ref().unwrap().review,
            Some(HostOperation::SetDom0Memory {
                memory: 3221225472,
                ..
            })
        ));
    }
    #[test]
    fn logging_review_keeps_exact_old_and_new_destination() {
        let mut s = state();
        select(&mut s, Kind::Logging);
        s.host_workspace.as_mut().unwrap().value = TextEdit::new("logs.example.test");
        review(&mut s);
        let op = s.host_workspace.as_ref().unwrap().review.clone();
        select(&mut s, Kind::Logging);
        s.host_workspace.as_mut().unwrap().value = TextEdit::new("bad name");
        review(&mut s);
        assert!(s.host_workspace.as_ref().unwrap().review.is_none());
        assert_eq!(
            op,
            Some(HostOperation::SetSyslog {
                expected_destination: None,
                destination: Some("logs.example.test".into())
            })
        );
    }
    #[test]
    fn autostart_is_explicitly_pool_scoped_and_unknown_is_not_false() {
        let mut s = state();
        pool(&mut s);
        review(&mut s);
        let w = s.host_workspace.as_ref().unwrap();
        assert_eq!(w.target, ResourceTarget::Pool(w.initial.pool_id));
        assert_eq!(
            w.review,
            Some(HostOperation::SetPoolAutostart {
                expected: false,
                enabled: true
            })
        );
        Arc::make_mut(s.inventory.as_mut().unwrap())
            .details
            .pool
            .as_mut()
            .unwrap()
            .auto_poweron = None;
        pool(&mut s);
        review(&mut s);
        assert!(s.host_workspace.as_ref().unwrap().review.is_none());
    }
    #[test]
    fn forget_requires_offline_disabled_noncoordinator_and_explicit_power_off_ack() {
        let mut s = state();
        select(&mut s, Kind::Forget);
        review(&mut s);
        assert!(s.host_workspace.as_ref().unwrap().review.is_none());
        let w = s.host_workspace.as_mut().unwrap();
        w.acknowledged = true;
        assert!(
            !operation(w)
                .unwrap()
                .availability(&w.initial, w.target)
                .is_available()
        );
    }
}
