use super::*;
use xenadmin_core::{Ipv6Configuration, Ipv6Mode, PhysicalInterface};

pub(super) enum Editor {
    Ipv6 {
        id: PifId,
        expected: PifIdentity,
        before: Ipv6Configuration,
        mode: Ipv6Mode,
        address: Box<TextEdit>,
        gateway: Box<TextEdit>,
        dns: Box<TextEdit>,
    },
    Purpose {
        id: PifId,
        expected: PifIdentity,
        before: Option<String>,
        value: Box<TextEdit>,
    },
}
fn ipv6(p: &PhysicalInterface) -> Option<Ipv6Configuration> {
    Some(Ipv6Configuration {
        mode: match p.ipv6_configuration_mode.as_str() {
            "None" => Ipv6Mode::None,
            "DHCP" => Ipv6Mode::Dhcp,
            "Autoconf" => Ipv6Mode::Autoconf,
            "Static" => Ipv6Mode::Static,
            _ => return None,
        },
        addresses: p.ipv6.clone(),
        gateway: p.ipv6_gateway.clone(),
        dns: p.dns.clone(),
    })
}
fn host_ipv6(i: &Inventory, p: &PhysicalInterface) -> bool {
    i.details.pifs.values().any(|m| {
        m.host == p.host
            && m.management == Some(true)
            && matches!(
                m.ipv6_configuration_mode.as_str(),
                "Static" | "DHCP" | "Autoconf"
            )
    })
}
fn open(s: &mut State, id: PifId, purpose: bool) {
    let Some(p) = s.inventory.as_ref().and_then(|i| i.details.pifs.get(&id)) else {
        return;
    };
    let Some(expected) = PifIdentity::from_inventory(p) else {
        return;
    };
    let editor = if purpose {
        Editor::Purpose {
            id,
            expected,
            before: p.management_purpose.clone(),
            value: Box::new(TextEdit::new(
                p.management_purpose.clone().unwrap_or_default(),
            )),
        }
    } else {
        let Some(before) = ipv6(p) else {
            return;
        };
        Editor::Ipv6 {
            id,
            expected,
            mode: before.mode,
            address: Box::new(TextEdit::new(
                before.addresses.first().cloned().unwrap_or_default(),
            )),
            gateway: Box::new(TextEdit::new(before.gateway.clone())),
            dns: Box::new(TextEdit::new(before.dns.clone())),
            before,
        }
    };
    set_form(s, Form::Interface(Box::new(editor)));
}
pub(super) fn controls(i: &Inventory, p: &PhysicalInterface, enabled: bool, k: &Kit) -> El {
    let id = p.id;
    let allowed = enabled && p.managed == Some(true) && p.management == Some(false);
    let mut body = col()
        .gap(6.0)
        .child(
            caption(
                format!(
                    "IPv6 {} · {} · gateway {}\nPurpose: {} · unplug protection: {}",
                    p.ipv6_configuration_mode,
                    p.ipv6.join(", "),
                    p.ipv6_gateway,
                    p.management_purpose.as_deref().unwrap_or("Not set"),
                    p.disallow_unplug
                        .map(|v| if v { "enabled" } else { "disabled" })
                        .unwrap_or("unknown")
                ),
                k,
            )
            .wrap_anywhere(),
        )
        .child(
            Button::new("IPv6 configuration…")
                .disabled(!allowed || ipv6(p).is_none() || !host_ipv6(i, p))
                .on_press(move |s: &mut State| open(s, id, false))
                .build(k),
        )
        .child(
            Button::new("Interface purpose…")
                .disabled(!allowed)
                .on_press(move |s: &mut State| open(s, id, true))
                .build(k),
        );
    if !host_ipv6(i, p) {
        body = body.child(caption("IPv6 configuration requires IPv6 to be enabled on the host management interface at installation.", k).wrap());
    }
    if let (Some(expected), Some(before)) = (PifIdentity::from_inventory(p), p.disallow_unplug) {
        body = body.child(review_button(if before { "Allow unplugging…" } else { "Protect from unplugging…" },
            ResourceTarget::Pif(id), NetworkOperation::SetUnplugProtection { expected, before, enabled: !before },
            vec![format!("Interface: {} ({id})", pif_name(i,id)), format!("Unplug protection: {before} → {}", !before),
                "Protection prevents XAPI from unplugging this interface. Disabling it permits later unplug operations; it does not unplug now.".into()], allowed, k));
    }
    body
}
pub(super) fn review(
    editor: &Editor,
    i: &Inventory,
) -> Result<(ResourceTarget, NetworkOperation, Vec<String>), String> {
    let (id, expected) = match editor {
        Editor::Ipv6 { id, expected, .. } | Editor::Purpose { id, expected, .. } => (*id, expected),
    };
    let p = i.details.pifs.get(&id).ok_or("The interface disappeared")?;
    if PifIdentity::from_inventory(p).as_ref() != Some(expected)
        || p.management != Some(false)
        || p.managed != Some(true)
    {
        return Err("The interface identity or management role changed; reopen the editor".into());
    }
    let mut lines = vec![format!("Interface: {} ({id})", pif_name(i, id))];
    let operation = match editor {
        Editor::Ipv6 {
            before,
            mode,
            address,
            gateway,
            dns,
            ..
        } => {
            if ipv6(p).as_ref() != Some(before) || !host_ipv6(i, p) {
                return Err(
                    "The IPv6 configuration changed or is unavailable; reopen the editor".into(),
                );
            }
            let configuration = Ipv6Configuration {
                mode: *mode,
                addresses: if *mode == Ipv6Mode::Static {
                    vec![address.text.trim().into()]
                } else {
                    vec![]
                },
                gateway: if *mode == Ipv6Mode::Static {
                    gateway.text.trim().into()
                } else {
                    String::new()
                },
                dns: dns.text.trim().into(),
            };
            if !configuration.valid() || &configuration == before {
                return Err("Enter changed valid IPv6 settings: one address/prefix for Static and comma-separated DNS addresses".into());
            }
            lines.extend([format!("Previous: {} {} · gateway {} · DNS {}", before.mode.as_str(), before.addresses.join(", "), before.gateway, before.dns),
                format!("New: {} {} · gateway {} · DNS {}", mode.as_str(), configuration.addresses.join(", "), configuration.gateway, configuration.dns),
                "Traffic using this interface may be interrupted. DNS is shared with the interface's IPv4 configuration. This change retains the primary management interface and installed address family.".into()]);
            NetworkOperation::ConfigureIpv6 {
                expected: expected.clone(),
                before: before.clone(),
                configuration,
            }
        }
        Editor::Purpose { before, value, .. } => {
            if &p.management_purpose != before {
                return Err("The interface purpose changed; reopen the editor".into());
            }
            let purpose = (!value.text.trim().is_empty()).then(|| value.text.trim().to_owned());
            if &purpose == before
                || purpose
                    .as_ref()
                    .is_some_and(|v| v.len() > 256 || v.chars().any(char::is_control))
            {
                return Err("Enter a changed purpose of at most 256 bytes, or clear it".into());
            }
            lines.extend([format!("Purpose: {} → {}", before.as_deref().unwrap_or("Not set"), purpose.as_deref().unwrap_or("Not set")),
                "Purpose labels describe secondary management or storage interfaces. Changing the label does not change routing, IP configuration or unplug protection.".into()]);
            NetworkOperation::SetInterfacePurpose {
                expected: expected.clone(),
                before: before.clone(),
                purpose,
            }
        }
    };
    Ok((ResourceTarget::Pif(id), operation, lines))
}
fn editor(s: &mut State) -> &mut Editor {
    let Form::Interface(editor) = &mut edit(s).form else {
        unreachable!()
    };
    editor
}
pub(super) fn form(value: &Editor, i: &Inventory, k: &Kit) -> El {
    let id = match value {
        Editor::Ipv6 { id, .. } | Editor::Purpose { id, .. } => *id,
    };
    let fields = match value {
        Editor::Purpose { value, .. } => col().gap(8.0).child(heading("Interface purpose", k))
            .child(caption("A label for secondary management or storage traffic; leave empty to remove it.", k).wrap())
            .child(TextInput::new(value).label("Purpose").on_edit(|s: &mut State| {
                let Editor::Purpose { value, .. } = editor(s) else { unreachable!() }; value
            }).build(k)),
        Editor::Ipv6 { mode, address, gateway, dns, .. } => {
            let mut body = col().gap(8.0).child(heading("IPv6 configuration", k));
            for selected in [Ipv6Mode::None, Ipv6Mode::Dhcp, Ipv6Mode::Autoconf, Ipv6Mode::Static] {
                body = body.child(Button::new(format!("{}{}", if *mode == selected { "✓ " } else { "" }, selected.as_str()))
                    .on_press(move |s: &mut State| { if let Editor::Ipv6 { mode, .. } = editor(s) { *mode = selected; } }).build(k));
            }
            if *mode == Ipv6Mode::Static {
                body = body.child(caption("IPv6 address/prefix (for example 2001:db8::20/64)", k).wrap())
                    .child(TextInput::new(address).label("IPv6 address/prefix").on_edit(|s: &mut State| {
                        let Editor::Ipv6 { address, .. } = editor(s) else { unreachable!() }; address
                    }).build(k))
                    .child(caption("IPv6 gateway (optional)", k))
                    .child(TextInput::new(gateway).label("IPv6 gateway").on_edit(|s: &mut State| {
                        let Editor::Ipv6 { gateway, .. } = editor(s) else { unreachable!() }; gateway
                    }).build(k));
            }
            body.child(caption("DNS servers, comma separated (shared with IPv4)", k).wrap())
                .child(TextInput::new(dns).label("DNS servers").on_edit(|s: &mut State| {
                    let Editor::Ipv6 { dns, .. } = editor(s) else { unreachable!() }; dns
                }).build(k))
        }
    };
    col()
        .gap(10.0)
        .child(caption(format!("Interface: {} ({id})", pif_name(i, id)), k).wrap_anywhere())
        .child(fields)
}

pub(super) fn scene(state: &mut State) {
    let inventory = Arc::make_mut(state.inventory.as_mut().unwrap());
    let id = *inventory.details.pifs.keys().next().unwrap();
    let mut management = inventory.details.pifs[&id].clone();
    management.id = PifId::from(uuid::Uuid::from_u128(991));
    management.management = Some(true);
    management.ipv6_configuration_mode = "Static".into();
    management.ipv6 = vec!["2001:db8::10/64".into()];
    inventory.details.pifs.insert(management.id, management);
    let network = inventory.details.pifs[&id].network;
    edit(state).network = network;
    open(state, id, false);
    let Editor::Ipv6 {
        mode,
        address,
        gateway,
        dns,
        ..
    } = editor(state)
    else {
        unreachable!()
    };
    *mode = Ipv6Mode::Static;
    **address = TextEdit::new("2001:db8::20/64");
    **gateway = TextEdit::new("2001:db8::1");
    **dns = TextEdit::new("2001:db8::53");
}

#[cfg(test)]
mod tests {
    use super::*;
    fn setup() -> (State, PifId) {
        let mut state = crate::init();
        super::super::scene(&mut state);
        let inventory = Arc::make_mut(state.inventory.as_mut().unwrap());
        let id = *inventory.details.pifs.keys().next().unwrap();
        let mut management = inventory.details.pifs[&id].clone();
        management.id = PifId::from(uuid::Uuid::from_u128(991));
        management.management = Some(true);
        management.ipv6_configuration_mode = "Static".into();
        management.ipv6 = vec!["2001:db8::10/64".into()];
        inventory.details.pifs.insert(management.id, management);
        (state, id)
    }
    #[test]
    fn ipv6_review_is_visible_at_both_widths_and_keeps_captured_identity() {
        for size in [(513.0, 652.0), (1440.0, 900.0)] {
            let (mut state, id) = setup();
            open(&mut state, id, false);
            let Editor::Ipv6 {
                mode,
                address,
                gateway,
                dns,
                ..
            } = editor(&mut state)
            else {
                panic!()
            };
            *mode = Ipv6Mode::Static;
            **address = TextEdit::new("2001:db8::20/64");
            **gateway = TextEdit::new("2001:db8::1");
            **dns = TextEdit::new("2001:db8::53");
            let mut runtime = rune::Runtime::new(
                state,
                crate::view,
                size,
                rune::Env {
                    system_dark: true,
                    scale: 1.0,
                },
            );
            for _ in 0..30 {
                runtime.frame(1.0 / 60.0);
            }
            let (x, y) = runtime.find("network-review").unwrap();
            assert!(x > 0.0 && x < size.0 && y > 0.0 && y < size.1);
            runtime.pointer_down(x, y);
            runtime.pointer_up(x, y);
            runtime.frame(0.0);
            let review = runtime
                .state
                .network_workspace
                .as_ref()
                .unwrap()
                .review
                .as_ref()
                .unwrap();
            assert_eq!(review.target, ResourceTarget::Pif(id));
            assert!(
                matches!(&review.operation, xenadmin_core::ResourceAction::Network(NetworkOperation::ConfigureIpv6 { configuration, .. }) if configuration.addresses == ["2001:db8::20/64"])
            );
            assert!(review.lines.iter().any(|s| s.contains("DNS is shared")));
            let (x, y) = runtime.find("network-confirm").unwrap();
            assert!(x > 0.0 && x < size.0 && y > 0.0 && y < size.1);
            runtime.pointer_down(x, y);
            runtime.pointer_up(x, y);
            runtime.frame(0.0);
            assert!(runtime.state.resource_operations.is_empty());
        }
    }
    #[test]
    fn interface_review_refuses_changed_identity_configuration_and_pool() {
        for change in 0..3 {
            let (mut state, id) = setup();
            open(&mut state, id, false);
            let Editor::Ipv6 { mode, address, .. } = editor(&mut state) else {
                panic!()
            };
            *mode = Ipv6Mode::Static;
            **address = TextEdit::new("2001:db8::20/64");
            let inventory = Arc::make_mut(state.inventory.as_mut().unwrap());
            match change {
                0 => inventory.details.pifs.get_mut(&id).unwrap().management = Some(true),
                1 => {
                    inventory.details.pifs.get_mut(&id).unwrap().ipv6 =
                        vec!["2001:db8::99/64".into()]
                }
                _ => inventory.pool_id = PoolId::from(uuid::Uuid::from_u128(992)),
            }
            form_review(&mut state);
            assert!(state.network_workspace.as_ref().unwrap().review.is_none());
            assert!(state.network_workspace.as_ref().unwrap().error.is_some());
        }
    }
    #[test]
    fn purpose_label_is_separate_from_unplug_protection_and_invalid_ipv6_is_refused() {
        let (mut state, id) = setup();
        open(&mut state, id, true);
        let Editor::Purpose { value, .. } = editor(&mut state) else {
            panic!()
        };
        **value = TextEdit::new("Storage fabric");
        form_review(&mut state);
        assert!(
            matches!(&state.network_workspace.as_ref().unwrap().review.as_ref().unwrap().operation,
            xenadmin_core::ResourceAction::Network(NetworkOperation::SetInterfacePurpose { before: None, purpose: Some(v), .. }) if v == "Storage fabric")
        );
        assert_eq!(
            state.inventory.as_ref().unwrap().details.pifs[&id].disallow_unplug,
            Some(false)
        );
        open(&mut state, id, false);
        let Editor::Ipv6 { mode, address, .. } = editor(&mut state) else {
            panic!()
        };
        *mode = Ipv6Mode::Static;
        **address = TextEdit::new("2001:db8::20/129");
        form_review(&mut state);
        assert!(state.network_workspace.as_ref().unwrap().review.is_none());
    }
}
