use super::*;
use xenadmin_core::{
    Ipv6Configuration, Ipv6Mode, ManagementAddressFamily, ManagementInterface,
    ManagementNetworkOperation, ResourceAction,
};
pub(super) struct Editor {
    source: ManagementInterface,
    moving: bool,
    destination: Option<ManagementInterface>,
    address: TextEdit,
    netmask: TextEdit,
    gateway: TextEdit,
    dns: TextEdit,
}
fn editor(s: &mut State) -> &mut Editor {
    let Form::Management(editor) = &mut edit(s).form else {
        unreachable!()
    };
    editor
}
fn open(s: &mut State, id: PifId, moving: bool) {
    let Some(source) = s
        .inventory
        .as_ref()
        .and_then(|i| i.details.pifs.get(&id))
        .and_then(ManagementInterface::from_inventory)
    else {
        edit(s).error = Some("Management interface identity or IP settings are incomplete".into());
        return;
    };
    let ipv6 = source.primary == ManagementAddressFamily::Ipv6;
    let value = Editor {
        moving,
        destination: None,
        address: TextEdit::new(if ipv6 {
            source.ipv6.addresses.first().cloned().unwrap_or_default()
        } else {
            source.ipv4.address.clone()
        }),
        netmask: TextEdit::new(source.ipv4.netmask.clone()),
        gateway: TextEdit::new(if ipv6 {
            source.ipv6.gateway.clone()
        } else {
            source.ipv4.gateway.clone()
        }),
        dns: TextEdit::new(source.ipv4.dns.clone()),
        source,
    };
    set_form(s, Form::Management(Box::new(value)));
}
pub(super) fn buttons(id: PifId, enabled: bool, k: &Kit) -> El {
    row()
        .gap(8.0)
        .flex_wrap()
        .child(
            Button::new("Change management IP…")
                .disabled(!enabled)
                .on_press(move |s: &mut State| open(s, id, false))
                .build(k)
                .key("management-ip"),
        )
        .child(
            Button::new("Move management…")
                .disabled(!enabled)
                .on_press(move |s: &mut State| open(s, id, true))
                .build(k)
                .key("management-move"),
        )
}
fn operation(v: &Editor) -> Result<ManagementNetworkOperation, String> {
    if v.moving {
        return Ok(ManagementNetworkOperation::Move {
            source: v.source.clone(),
            destination: v
                .destination
                .clone()
                .ok_or("Choose a configured destination interface")?,
        });
    }
    Ok(match v.source.primary {
        ManagementAddressFamily::Ipv4 => ManagementNetworkOperation::ConfigureIpv4 {
            interface: v.source.clone(),
            configuration: Ipv4Configuration {
                mode: Ipv4Mode::Static,
                address: v.address.text.trim().into(),
                netmask: v.netmask.text.trim().into(),
                gateway: v.gateway.text.trim().into(),
                dns: v.dns.text.trim().into(),
            },
        },
        ManagementAddressFamily::Ipv6 => ManagementNetworkOperation::ConfigureIpv6 {
            interface: v.source.clone(),
            configuration: Ipv6Configuration {
                mode: Ipv6Mode::Static,
                addresses: vec![v.address.text.trim().into()],
                gateway: v.gateway.text.trim().into(),
                dns: v.dns.text.trim().into(),
            },
        },
    })
}
fn summary(p: &ManagementInterface) -> String {
    match p.primary {
        ManagementAddressFamily::Ipv4 => format!(
            "{} / {} · gateway {} · DNS {}",
            p.ipv4.address, p.ipv4.netmask, p.ipv4.gateway, p.ipv4.dns
        ),
        ManagementAddressFamily::Ipv6 => format!(
            "{} · gateway {} · DNS {}",
            p.ipv6.addresses.join(", "),
            p.ipv6.gateway,
            p.ipv6.dns
        ),
    }
}
fn review(v: &Editor, i: &Inventory) -> Result<(ManagementNetworkOperation, Vec<String>), String> {
    let op = operation(v)?;
    let available = op.availability(i);
    if !available.is_available() {
        return Err(available.explanation());
    }
    let mut lines = vec![
        format!(
            "Host: {} ({})",
            i.hosts
                .get(&v.source.identity.host)
                .map(|h| h.name.as_str())
                .unwrap_or("Unknown"),
            v.source.identity.host
        ),
        format!(
            "Current interface: {} ({})",
            pif_name(i, v.source.pif),
            v.source.pif
        ),
        format!(
            "Current {}: {}",
            v.source.primary.as_str(),
            summary(&v.source)
        ),
    ];
    match &op {
        ManagementNetworkOperation::Move { destination, .. } => {
            lines.push(format!(
                "Destination: {} ({}) · network {}",
                pif_name(i, destination.pif),
                destination.pif,
                name(i, destination.identity.network)
            ));
            lines.push(format!("Destination settings: {}", summary(destination)));
            lines.push("The old interface stays plugged and retains its IP settings. Clear its address separately after verifying access.".into());
        }
        ManagementNetworkOperation::ConfigureIpv4 {
            configuration: c, ..
        } => lines.push(format!(
            "New IPv4: {} / {} · gateway {} · DNS {}",
            c.address, c.netmask, c.gateway, c.dns
        )),
        ManagementNetworkOperation::ConfigureIpv6 {
            configuration: c, ..
        } => lines.push(format!(
            "New IPv6: {} · gateway {} · DNS {}",
            c.addresses.join(", "),
            c.gateway,
            c.dns
        )),
    }
    lines.extend([format!("Recovery address: {} · HTTPS on the current connection port",op.recovery_address().ok_or("Enter a usable recovery address")?),"Management and storage traffic may be interrupted. Have an independent path to the host available. The client checks the new configuration and authenticates the reviewed host at the recovery address.".into(),"Certificate validation follows this connection's existing settings. A new address may need its own trusted certificate; no trust is copied automatically.".into(),"Lost replies remain uncertain. Recheck inspects existing evidence and never repeats the change or rolls it back. Other workspaces are not deliberately disconnected.".into()]);
    Ok((op, lines))
}
pub(super) fn prepare(s: &mut State) {
    let result = (|| {
        let w = s
            .network_workspace
            .as_ref()
            .ok_or("Close and reopen networking")?;
        let i = s
            .inventory
            .as_ref()
            .filter(|i| i.pool_id == w.pool)
            .ok_or("The pool changed")?;
        let Form::Management(v) = &w.form else {
            return Err("Choose a management change".into());
        };
        review(v, i)
    })();
    match result {
        Ok((op, lines)) => prepare_action(
            s,
            ResourceTarget::Host(op.source().identity.host),
            ResourceAction::ManagementNetwork(Box::new(op)),
            lines,
        ),
        Err(error) => edit(s).error = Some(error),
    }
}
pub(super) fn review_unavailable(s: &State, w: &Workspace) -> Option<String> {
    let ResourceAction::ManagementNetwork(op) = &w.review.as_ref()?.operation else {
        return None;
    };
    let Some(i) = s.inventory.as_ref().filter(|i| i.pool_id == w.pool) else {
        return Some("The pool changed. Close and reopen networking.".into());
    };
    let available = op.availability(i);
    (!available.is_available()).then(|| available.explanation())
}
pub(super) fn form(v: &Editor, i: &Inventory, k: &Kit) -> El {
    let mut body=col().gap(8.0).child(heading(if v.moving {"Move host management"}else{"Management IP settings"},k))
        .child(caption(format!("{} · {}",pif_name(i,v.source.pif),v.source.pif),k).wrap_anywhere())
        .child(caption("Static addressing in the installed address family, ordinary physical interfaces, MTU 1500 and HA disabled are required. Bond/VLAN handovers and DHCP changes need a separate workflow.",k).wrap());
    if v.moving {
        body=body.child(caption("Choose an already plugged, statically configured interface on this host, with a different network and address.",k).wrap());
        for p in i
            .details
            .pifs
            .values()
            .filter(|p| p.host == Some(v.source.identity.host) && p.id != v.source.pif)
        {
            let Some(candidate) = ManagementInterface::from_inventory(p) else {
                continue;
            };
            let available = ManagementNetworkOperation::Move {
                source: v.source.clone(),
                destination: candidate.clone(),
            }
            .availability(i);
            let chosen = v.destination.as_ref() == Some(&candidate);
            body = body.child(
                Button::new(format!(
                    "{}{} · {}",
                    if chosen { "✓ " } else { "" },
                    p.device,
                    candidate
                        .address()
                        .map(|ip| ip.to_string())
                        .unwrap_or_else(|| "No static address".into())
                ))
                .disabled(!available.is_available())
                .on_press(move |s: &mut State| editor(s).destination = Some(candidate.clone()))
                .build(k),
            );
            if !available.is_available() {
                body = body.child(caption(available.explanation(), k).wrap());
            }
        }
    } else {
        let ipv6 = v.source.primary == ManagementAddressFamily::Ipv6;
        body = body
            .child(caption(
                if ipv6 {
                    "IPv6 address/prefix"
                } else {
                    "IPv4 address"
                },
                k,
            ))
            .child(
                TextInput::new(&v.address)
                    .label(if ipv6 {
                        "IPv6 address/prefix"
                    } else {
                        "IPv4 address"
                    })
                    .on_edit(|s: &mut State| &mut editor(s).address)
                    .build(k),
            );
        if !ipv6 {
            body = body.child(caption("Netmask", k)).child(
                TextInput::new(&v.netmask)
                    .label("Netmask")
                    .on_edit(|s: &mut State| &mut editor(s).netmask)
                    .build(k),
            );
        }
        body = body
            .child(caption("Gateway (optional)", k))
            .child(
                TextInput::new(&v.gateway)
                    .label("Gateway")
                    .on_edit(|s: &mut State| &mut editor(s).gateway)
                    .build(k),
            )
            .child(caption("DNS servers, comma separated (shared by IPv4 and IPv6)", k).wrap())
            .child(
                TextInput::new(&v.dns)
                    .label("DNS servers")
                    .on_edit(|s: &mut State| &mut editor(s).dns)
                    .build(k),
            );
    }
    if let Err(error) = review(v, i) {
        body = body.child(caption(error, k).wrap());
    }
    body
}
pub(super) fn scene(s: &mut State) {
    super::scene(s);
    let i = Arc::make_mut(s.inventory.as_mut().unwrap());
    let source_id = *i.details.pifs.keys().next().unwrap();
    let p = i.details.pifs.get_mut(&source_id).unwrap();
    p.management = Some(true);
    p.physical = Some(true);
    p.vlan = Some(-1);
    p.managed = Some(true);
    p.mtu = Some(1500);
    p.currently_attached = Some(true);
    p.disallow_unplug = Some(false);
    p.management_purpose = None;
    p.primary_address_type = Some("IPv4".into());
    p.ip_configuration_mode = "Static".into();
    p.ip = "192.0.2.10".into();
    p.netmask = "255.255.255.0".into();
    p.gateway = "192.0.2.1".into();
    p.dns = "192.0.2.53".into();
    p.ipv6_configuration_mode = "None".into();
    p.ipv6.clear();
    p.ipv6_gateway.clear();
    if let Some(pool) = &mut i.details.pool {
        pool.ha_enabled = Some(false);
    }
    open(s, source_id, false);
    editor(s).address = TextEdit::new("192.0.2.20");
    prepare(s);
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn management_review_captures_recovery_settings_at_both_widths_without_demo_dispatch() {
        for size in [(513.0, 652.0), (1440.0, 900.0)] {
            let mut s = crate::init();
            scene(&mut s);
            let w = s.network_workspace.as_ref().unwrap();
            let review = w.review.as_ref().unwrap();
            assert!(matches!(
                review.operation,
                ResourceAction::ManagementNetwork(_)
            ));
            assert!(review.lines.iter().any(|s| s.contains("192.0.2.20")));
            assert!(
                review
                    .lines
                    .iter()
                    .any(|s| s.contains("no trust is copied"))
            );
            let mut runtime = rune::Runtime::new(
                s,
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
            let (x, y) = runtime.find("network-confirm").unwrap();
            assert!(x > 0.0 && x < size.0 && y > 0.0 && y < size.1);
            runtime.pointer_down(x, y);
            runtime.pointer_up(x, y);
            runtime.frame(0.0);
            assert!(runtime.state.resource_operations.is_empty());
        }
    }
    #[test]
    fn management_review_refuses_changed_source_pool_ha_and_nonstandard_mtu() {
        for changed in ["ip", "pool", "ha", "mtu"] {
            let mut s = crate::init();
            scene(&mut s);
            let i = Arc::make_mut(s.inventory.as_mut().unwrap());
            let p = *i.details.pifs.keys().next().unwrap();
            match changed {
                "ip" => i.details.pifs.get_mut(&p).unwrap().ip = "192.0.2.99".into(),
                "pool" => i.pool_id = uuid::Uuid::new_v4().into(),
                "ha" => i.details.pool.as_mut().unwrap().ha_enabled = Some(true),
                "mtu" => i.details.pifs.get_mut(&p).unwrap().mtu = Some(9000),
                _ => unreachable!(),
            };
            assert!(
                review_unavailable(&s, s.network_workspace.as_ref().unwrap()).is_some(),
                "{changed}"
            );
        }
    }
    #[test]
    fn management_move_review_captures_the_destination_and_retains_old_address() {
        let mut s = crate::init();
        scene(&mut s);
        let source = editor(&mut s).source.clone();
        let i = Arc::make_mut(s.inventory.as_mut().unwrap());
        let mut destination = i.details.pifs[&source.pif].clone();
        destination.id = uuid::Uuid::new_v4().into();
        destination.network = Some(uuid::Uuid::new_v4().into());
        destination.management = Some(false);
        destination.ip = "192.0.2.30".into();
        let capture = ManagementInterface::from_inventory(&destination).unwrap();
        i.details.pifs.insert(destination.id, destination);
        open(&mut s, source.pif, true);
        editor(&mut s).destination = Some(capture.clone());
        prepare(&mut s);
        let review = s
            .network_workspace
            .as_ref()
            .unwrap()
            .review
            .as_ref()
            .unwrap();
        let ResourceAction::ManagementNetwork(op) = &review.operation else {
            panic!()
        };
        assert!(
            matches!(&**op,ManagementNetworkOperation::Move{destination,..} if destination==&capture)
        );
        assert!(review.lines.iter().any(|v| v.contains("retains its IP")));
        Arc::make_mut(s.inventory.as_mut().unwrap())
            .details
            .pifs
            .get_mut(&capture.pif)
            .unwrap()
            .disallow_unplug = Some(true);
        assert!(review_unavailable(&s, s.network_workspace.as_ref().unwrap()).is_some());
    }
    #[test]
    fn management_ipv6_review_keeps_installed_family_and_rejects_invalid_recovery_address() {
        let mut s = crate::init();
        scene(&mut s);
        let id = editor(&mut s).source.pif;
        let p = Arc::make_mut(s.inventory.as_mut().unwrap())
            .details
            .pifs
            .get_mut(&id)
            .unwrap();
        p.primary_address_type = Some("IPv6".into());
        p.ipv6_configuration_mode = "Static".into();
        p.ipv6 = vec!["2001:db8::10/64".into()];
        p.ip_configuration_mode = "None".into();
        p.ip.clear();
        p.netmask.clear();
        p.gateway.clear();
        open(&mut s, id, false);
        editor(&mut s).address = TextEdit::new("2001:db8::20/64");
        prepare(&mut s);
        assert!(s.network_workspace.as_ref().unwrap().review.is_some());
        edit(&mut s).review = None;
        editor(&mut s).address = TextEdit::new("fe80::20/64");
        prepare(&mut s);
        assert!(s.network_workspace.as_ref().unwrap().review.is_none());
        assert!(s.network_workspace.as_ref().unwrap().error.is_some());
    }
}
