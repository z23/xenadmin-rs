use super::*;
use xenadmin_core::{PoolAdministration, VmStartupField};
const FIELDS: [VmStartupField; 3] = [
    VmStartupField::Order,
    VmStartupField::StartDelay,
    VmStartupField::ShutdownDelay,
];
fn values(order: xenadmin_core::ApplianceOrder) -> Option<[u64; 3]> {
    Some([
        order.order.try_into().ok()?,
        order.start_delay,
        order.shutdown_delay,
    ])
}
pub(super) fn select_vm(s: &mut State, id: VmId) {
    let Some(e) = s.pool_editor.as_mut() else {
        return;
    };
    e.vm = Some(id);
    let fields = e
        .inventory
        .details
        .vms
        .get(&id)
        .and_then(|v| v.appliance_order)
        .and_then(values);
    e.startup = std::array::from_fn(|i| {
        TextEdit::new(fields.map(|v| v[i].to_string()).unwrap_or_default())
    });
}
fn startup(s: &mut State, index: usize) {
    let result = (|| {
        let e = s
            .pool_editor
            .as_ref()
            .ok_or("Pool administration is closed")?;
        let vm = e.vm.ok_or("Select a VM")?;
        let d = e
            .inventory
            .details
            .vms
            .get(&vm)
            .ok_or("VM details are unavailable")?;
        if !d.appliance_known {
            return Err("vApp membership is unknown");
        }
        let old = d
            .appliance_order
            .and_then(values)
            .ok_or("Startup settings are unknown")?;
        let value = e.startup[index]
            .text
            .parse::<u64>()
            .map_err(|_| "Enter a nonnegative whole number")?;
        if value > if index == 0 { i64::MAX as u64 } else { 86_400 } {
            return Err(
                "Order must fit a signed 64-bit number; delays must be at most 86400 seconds",
            );
        }
        if value == old[index] {
            return Err("This setting has not changed");
        }
        Ok(PoolOperation::SetVmStartup {
            vm,
            appliance: d.appliance,
            field: FIELDS[index],
            expected: old[index],
            value,
        })
    })();
    match result {
        Ok(op) => stage(s, op),
        Err(error) => {
            if let Some(e) = s.pool_editor.as_mut() {
                e.error = Some(error.into());
            }
        }
    }
}
fn timeout(s: &mut State, reset: bool) {
    let result = (|| {
        let e = s
            .pool_editor
            .as_ref()
            .ok_or("Pool administration is closed")?;
        let a = &e
            .inventory
            .details
            .pool
            .as_ref()
            .ok_or("Pool settings unavailable")?
            .administration;
        if !a.ha_timeout_known
            || a.ha_enabled != Some(false)
            || a.ha_configuration_timeout.is_some()
        {
            return Err(
                "Disable HA first; a separate HA configuration timeout cannot be overwritten here",
            );
        }
        let seconds = if reset {
            None
        } else {
            Some(
                e.timeout
                    .text
                    .parse::<u32>()
                    .ok()
                    .filter(|v| (10..=86_400).contains(v))
                    .ok_or("Enter 10 to 86400 seconds")?,
            )
        };
        if a.ha_timeout_override == seconds.map(|v| v.to_string()) {
            return Err("The timeout default has not changed");
        }
        Ok(PoolOperation::SetHaTimeout {
            expected: a.ha_timeout_override.clone(),
            seconds,
        })
    })();
    match result {
        Ok(op) => stage(s, op),
        Err(error) => {
            if let Some(e) = s.pool_editor.as_mut() {
                e.error = Some(error.into());
            }
        }
    }
}
pub(super) fn pool_controls(e: &Editor, a: &PoolAdministration, k: &Kit) -> El {
    let mut body = col().gap(8.0).child(caption(
        format!("Allow HA overcommit: {}", known(a.ha_allow_overcommit)),
        k,
    ));
    if let Some(expected) = a.ha_allow_overcommit {
        body = body.child(action(
            k,
            if expected {
                "Disallow HA overcommit…"
            } else {
                "Allow HA overcommit…"
            },
            PoolOperation::SetHaAllowOvercommit {
                expected,
                enabled: !expected,
            },
            false,
            "ha-overcommit",
        ));
    }
    body = body.child(
        caption(
            format!(
                "Persistent timeout default: {}",
                if a.ha_timeout_known {
                    a.ha_timeout_override.as_deref().unwrap_or("host default")
                } else {
                    "unknown"
                }
            ),
            k,
        )
        .wrap(),
    );
    if let Some(value) = &a.ha_configuration_timeout {
        body=body.child(caption(format!("HA configuration timeout takes precedence: {value}. This view does not replace it."),k).wrap());
    }
    if a.ha_timeout_known && a.ha_enabled == Some(false) && a.ha_configuration_timeout.is_none() {
        body = body
            .child(
                TextInput::new(&e.timeout)
                    .label("HA timeout default (seconds)")
                    .placeholder("10–86400 seconds")
                    .on_edit(|s: &mut State| &mut s.pool_editor.as_mut().unwrap().timeout)
                    .build(k)
                    .key("ha-timeout-input"),
            )
            .child(
                Button::new("Review timeout default…")
                    .on_press(|s: &mut State| timeout(s, false))
                    .build(k)
                    .key("ha-timeout-review"),
            )
            .child(
                Button::new("Restore host timeout default…")
                    .disabled(a.ha_timeout_override.is_none())
                    .on_press(|s: &mut State| timeout(s, true))
                    .build(k)
                    .key("ha-timeout-reset"),
            );
    } else {
        body=body.child(caption("Timeout defaults are editable with HA disabled and a fully readable configuration.",k).wrap());
    }
    body
}
pub(super) fn vm_controls(e: &Editor, vm: VmId, k: &Kit) -> El {
    let mut body = col().gap(8.0).child(heading("Startup order and delays", k));
    let d = e.inventory.details.vms.get(&vm);
    let old = d.and_then(|d| d.appliance_order).and_then(values);
    let available = d.is_some_and(|d| d.appliance_known) && old.is_some();
    if !available {
        return body.child(
            caption(
                "Startup settings or vApp membership are unknown; editing is unavailable.",
                k,
            )
            .wrap(),
        );
    }
    body=body.child(caption("These values are also used by vApps. Lower order values start first; delay values are seconds. Changing them does not start or stop the VM.",k).wrap());
    for (i, field) in FIELDS.iter().enumerate() {
        body = body
            .child(caption(
                format!("{} · current {}", field.label(), old.unwrap()[i]),
                k,
            ))
            .child(
                TextInput::new(&e.startup[i])
                    .label(field.label())
                    .on_edit(move |s: &mut State| &mut s.pool_editor.as_mut().unwrap().startup[i])
                    .build(k)
                    .key(format!("ha-startup-{i}")),
            )
            .child(
                Button::new(format!("Review {}…", field.label().to_lowercase()))
                    .on_press(move |s: &mut State| startup(s, i))
                    .build(k)
                    .key(format!("ha-startup-review-{i}")),
            );
    }
    body
}
pub(super) fn scene(s: &mut State) {
    demo(s);
    let i = std::sync::Arc::make_mut(s.inventory.as_mut().unwrap());
    let a = &mut i.details.pool.as_mut().unwrap().administration;
    a.ha_timeout_known = true;
    a.ha_timeout_override = Some("60".into());
    a.ha_allow_overcommit = Some(false);
    let vm = i.guests().next().unwrap().id;
    let d = i.details.vms.get_mut(&vm).unwrap();
    d.appliance_known = true;
    d.appliance = None;
    d.appliance_order = Some(xenadmin_core::ApplianceOrder {
        order: 0,
        start_delay: 0,
        shutdown_delay: 0,
    });
    open(s);
    select_vm(s, vm);
    s.pool_editor.as_mut().unwrap().startup[0] = TextEdit::new("2");
    startup(s, 0);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ha_policy_review_binds_vm_and_shared_vapp_settings_at_both_sizes() {
        for size in [(513.0, 652.0), (1440.0, 900.0)] {
            let mut s = crate::init();
            scene(&mut s);
            let op = s.pool_editor.as_ref().unwrap().review.clone().unwrap();
            let mut rt = rune::Runtime::new(
                s,
                crate::view,
                size,
                rune::Env {
                    system_dark: true,
                    scale: 1.0,
                },
            );
            for _ in 0..30 {
                rt.frame(1.0 / 60.0);
            }
            let (x, y) = rt.find("pool-confirm").unwrap();
            assert!(x > 0.0 && x < size.0 && y > 0.0 && y < size.1);
            rt.pointer_down(x, y);
            rt.pointer_up(x, y);
            rt.frame(0.0);
            assert!(rt.state.resource_operations.is_empty());
            rt.state.pool_editor.as_mut().unwrap().vm = None;
            assert_eq!(
                rt.state.pool_editor.as_ref().unwrap().review.as_ref(),
                Some(&op)
            );
            let description = describe(rt.state.pool_editor.as_ref().unwrap(), &op).join(" ");
            assert!(description.contains("vApp"));
            assert!(description.contains("0 → 2"));
        }
    }
    #[test]
    fn ha_policy_edits_refuse_invalid_inputs_and_keep_timeout_precedence_explicit() {
        let mut s = crate::init();
        scene(&mut s);
        s.pool_editor.as_mut().unwrap().review = None;
        for value in ["-1", "not-a-number", "86401"] {
            s.pool_editor.as_mut().unwrap().startup[1] = TextEdit::new(value);
            startup(&mut s, 1);
            assert!(s.pool_editor.as_ref().unwrap().review.is_none());
        }
        s.pool_editor.as_mut().unwrap().timeout = TextEdit::new("120");
        timeout(&mut s, false);
        assert!(
            matches!(&s.pool_editor.as_ref().unwrap().review,Some(PoolOperation::SetHaTimeout{expected:Some(v),seconds:Some(120)})if v=="60")
        );
        let e = s.pool_editor.as_mut().unwrap();
        e.review = None;
        std::sync::Arc::make_mut(&mut e.inventory)
            .details
            .pool
            .as_mut()
            .unwrap()
            .administration
            .ha_configuration_timeout = Some("90".into());
        timeout(&mut s, false);
        assert!(s.pool_editor.as_ref().unwrap().review.is_none());
        assert!(
            s.pool_editor
                .as_ref()
                .unwrap()
                .error
                .as_ref()
                .unwrap()
                .contains("separate HA configuration")
        );
    }
    #[test]
    fn ha_policy_missing_membership_refuses_review_and_selecting_vm_resets_draft() {
        let mut s = crate::init();
        scene(&mut s);
        let vm = s.pool_editor.as_ref().unwrap().vm.unwrap();
        select_vm(&mut s, vm);
        assert_eq!(s.pool_editor.as_ref().unwrap().startup[0].text, "0");
        let e = s.pool_editor.as_mut().unwrap();
        e.review = None;
        e.startup[0] = TextEdit::new("4");
        std::sync::Arc::make_mut(&mut e.inventory)
            .details
            .vms
            .get_mut(&vm)
            .unwrap()
            .appliance_known = false;
        startup(&mut s, 0);
        assert!(s.pool_editor.as_ref().unwrap().review.is_none());
    }
}
