use super::*;
use xenadmin_core::{
    Ipv6Configuration, Ipv6Mode, ManagementAddressFamily, ManagementDynamicMode,
    ManagementInterface, ManagementNetworkOperation, ResourceAction,
};
pub(super) struct Editor {
    source: ManagementInterface,
    moving: bool,
    destination: Option<ManagementInterface>,
    address: TextEdit,
    netmask: TextEdit,
    gateway: TextEdit,
    dns: TextEdit,
    automatic: Option<ManagementDynamicMode>,
    recovery: TextEdit,
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
        .and_then(|i| ManagementInterface::capture(i, id))
    else {
        edit(s).error = Some("Management interface identity, IP settings or VLAN/bond topology are incomplete. Bond members cannot be management endpoints.".into());
        return;
    };
    let ipv6 = source.primary == ManagementAddressFamily::Ipv6;
    let value = Editor {
        moving,
        automatic: None,
        recovery: TextEdit::new(source.address().map(|v| v.to_string()).unwrap_or_default()),
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
    if let Some(mode) = v.automatic {
        return Ok(ManagementNetworkOperation::ConfigureDynamic {
            interface: v.source.clone(),
            mode,
            recovery: v.recovery.text.trim().parse().map_err(
                |_| "Enter the expected recovery IP address, without a prefix or hostname",
            )?,
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
            "{} · {} / {} · gateway {} · DNS {}",
            p.ipv4.mode.as_str(),
            p.ipv4.address,
            p.ipv4.netmask,
            p.ipv4.gateway,
            p.ipv4.dns
        ),
        ManagementAddressFamily::Ipv6 => format!(
            "{} · {} · gateway {} · DNS {}",
            p.ipv6.mode.as_str(),
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
    let mut components = std::collections::BTreeSet::new();
    let endpoints = match &op {
        ManagementNetworkOperation::Move {
            source,
            destination,
        } => vec![source, destination],
        _ => vec![op.source()],
    };
    for endpoint in endpoints {
        if let Some(topology) = &endpoint.topology {
            let ids = topology.pifs.keys().copied().collect::<Vec<_>>();
            if !components.insert(ids) {
                continue;
            }
            lines.push(format!("Reviewed topology: {} interfaces, {} bonds, {} VLANs. Membership, configuration and protection are rechecked before the change.",topology.pifs.len(),topology.bonds.len(),topology.vlans.len()));
            for bond in topology.bonds.values() {
                let label = |id| format!("{} ({id})", pif_name(i, id));
                lines.push(format!(
                    "Bond {} · {} · master {} · members {}",
                    bond.id,
                    bond.mode,
                    label(bond.master.unwrap()),
                    bond.slaves
                        .iter()
                        .map(|id| label(*id))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            for vlan in topology.vlans.values() {
                lines.push(format!(
                    "VLAN {} · tag {} · parent {} · interface {}",
                    vlan.id,
                    vlan.tag.unwrap(),
                    vlan.tagged_pif.unwrap(),
                    vlan.untagged_pif.unwrap()
                ));
            }
            lines.push("Existing bond and VLAN topology is retained. This operation does not create, destroy or retag interfaces, or move the same address between interfaces.".into());
        }
    }
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
        ManagementNetworkOperation::ConfigureDynamic { mode, recovery, .. } => {
            lines.push(format!(
                "New addressing: {}. Expected assigned address: {recovery}.",
                mode.label()
            ));
            if *mode == ManagementDynamicMode::Ipv6Dhcp {
                lines.push("XCP-ng 8.3 documents incomplete DHCPv6 support. This workflow requires the actual assignment and recovery connection to succeed before reporting completion.".into());
            } else if *mode == ManagementDynamicMode::Ipv6Autoconf {
                lines.push("XCP-ng 8.3 autoconfiguration can replace manually configured DNS. The host supplies the new IPv6 network settings; the other address family's settings must remain unchanged.".into());
            }
            lines.push("The host obtains its address and primary-family network settings automatically. This address must match a DHCP reservation or a known stable autoconfiguration address; entering it does not create a reservation.".into());
            lines.push("If the host obtains a different address, this operation remains unresolved. The client does not scan the network or update DNS aliases. Use an independently verified host address to reconnect and inspect it.".into());
        }
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
        .child(caption("Use the installed address family, MTU 1500 and HA disabled. Existing physical interfaces, bond masters and VLANs over physical interfaces or bonds are supported when their complete topology can be reviewed.",k).wrap());
    if v.moving {
        body=body.child(caption("Choose an already plugged, statically configured interface on this host, with a different network and address.",k).wrap());
        for p in i
            .details
            .pifs
            .values()
            .filter(|p| p.host == Some(v.source.identity.host) && p.id != v.source.pif)
        {
            let Some(candidate) = ManagementInterface::capture(i, p.id) else {
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
        let mut modes = row().gap(8.0).flex_wrap().child(
            Button::new(if v.automatic.is_none() {
                "✓ Static"
            } else {
                "Static"
            })
            .on_press(|s: &mut State| editor(s).automatic = None)
            .build(k),
        );
        for mode in if ipv6 {
            vec![
                ManagementDynamicMode::Ipv6Dhcp,
                ManagementDynamicMode::Ipv6Autoconf,
            ]
        } else {
            vec![ManagementDynamicMode::Ipv4Dhcp]
        } {
            modes = modes.child(
                Button::new(format!(
                    "{}{}",
                    if v.automatic == Some(mode) {
                        "✓ "
                    } else {
                        ""
                    },
                    mode.label()
                ))
                .on_press(move |s: &mut State| editor(s).automatic = Some(mode))
                .build(k),
            );
        }
        body = body.child(modes);
        if v.automatic.is_some() {
            body=body.child(caption("Expected recovery IP",k)).child(TextInput::new(&v.recovery).label("Expected recovery IP").on_edit(|s:&mut State|&mut editor(s).recovery).build(k))
                .child(caption("Enter the address reserved by DHCP or expected from stable IPv6 autoconfiguration. The client verifies the actual assignment and host certificate. It does not create reservations, scan for hosts or update DNS aliases.",k).wrap());
        } else {
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
                .child(
                    caption(
                        "DNS servers, comma separated (only this address family's entries change)",
                        k,
                    )
                    .wrap(),
                )
                .child(
                    TextInput::new(&v.dns)
                        .label("DNS servers")
                        .on_edit(|s: &mut State| &mut editor(s).dns)
                        .build(k),
                );
        }
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
pub(super) fn dynamic_scene(s: &mut State) {
    scene(s);
    edit(s).review = None;
    editor(s).automatic = Some(ManagementDynamicMode::Ipv4Dhcp);
    editor(s).recovery = TextEdit::new("192.0.2.30");
    prepare(s);
}
pub(super) fn topology_scene(s: &mut State) {
    scene(s);
    let source = editor(s).source.pif;
    let i = Arc::make_mut(s.inventory.as_mut().unwrap());
    let original = i.details.pifs[&source].clone();
    for n in [1_u128, 2, 3] {
        let mut p = original.clone();
        p.id = uuid::Uuid::from_u128(0xf000 + n).into();
        p.network = Some(uuid::Uuid::from_u128(0xf100 + n).into());
        p.device = if n == 1 {
            "bond0".into()
        } else {
            format!("eth{}", n - 1)
        };
        p.mac = format!("02:00:00:00:00:{n:02x}");
        p.physical = Some(n != 1);
        p.management = Some(false);
        p.ip_configuration_mode = "None".into();
        p.ip.clear();
        p.netmask.clear();
        p.gateway.clear();
        p.dns.clear();
        i.networks.push(xenadmin_core::Network {
            uuid: p.network.unwrap().uuid(),
            name: format!("Uplink {n}"),
            bridge: format!("xenbr{n}"),
        });
        i.details.pifs.insert(p.id, p);
    }
    let parent = uuid::Uuid::from_u128(0xf001).into();
    let member = uuid::Uuid::from_u128(0xf002).into();
    let second = uuid::Uuid::from_u128(0xf003).into();
    let bond = uuid::Uuid::from_u128(0xf200).into();
    let vlan = uuid::Uuid::from_u128(0xf201).into();
    i.details.bonds.insert(
        bond,
        xenadmin_core::NetworkBond {
            id: bond,
            master: Some(parent),
            slaves: vec![member, second],
            primary_slave: Some(member),
            mode: "active-backup".into(),
            hashing_algorithm: None,
            hashing_algorithm_known: true,
            links_up: Some(2),
        },
    );
    i.details.vlans.insert(
        vlan,
        xenadmin_core::Vlan {
            id: vlan,
            tagged_pif: Some(parent),
            untagged_pif: Some(source),
            tag: Some(17),
        },
    );
    let p = i.details.pifs.get_mut(&source).unwrap();
    p.physical = Some(false);
    p.vlan = Some(17);
    for class in ["pif", "bond", "vlan"] {
        i.details
            .classes
            .insert(class.into(), xenadmin_core::ClassAvailability::Available);
    }
    open(s, source, false);
    editor(s).address = TextEdit::new("192.0.2.30");
    prepare(s);
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn vlan_over_bond_review_captures_full_topology_at_both_widths() {
        for size in [(513.0, 652.0), (1440.0, 900.0)] {
            let mut s = crate::init();
            topology_scene(&mut s);
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
            let topology = op.source().topology.as_ref().unwrap();
            assert_eq!(topology.pifs.len(), 4);
            assert_eq!(topology.bonds.len(), 1);
            assert_eq!(topology.vlans.len(), 1);
            assert!(review.lines.iter().any(|v| v.contains("bond0")));
            assert!(review.lines.iter().any(|v| v.contains("tag 17")));
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
    fn topology_review_tracks_membership_protection_and_metadata_but_not_link_counters() {
        let mut s = crate::init();
        topology_scene(&mut s);
        let before = s.inventory.clone().unwrap();
        {
            let i = Arc::make_mut(s.inventory.as_mut().unwrap());
            i.details.bonds.values_mut().next().unwrap().links_up = Some(0);
            for p in i.details.pifs.values_mut() {
                p.capabilities = Some(vec!["runtime-capability".into()]);
            }
        }
        assert!(review_unavailable(&s, s.network_workspace.as_ref().unwrap()).is_none());
        for change in ["primary", "mac", "protected", "tag", "class"] {
            s.inventory = Some(before.clone());
            let i = Arc::make_mut(s.inventory.as_mut().unwrap());
            match change {
                "primary" => {
                    let b = i.details.bonds.values_mut().next().unwrap();
                    b.primary_slave = Some(b.slaves[1]);
                }
                "mac" => {
                    i.details
                        .pifs
                        .get_mut(&uuid::Uuid::from_u128(0xf002).into())
                        .unwrap()
                        .mac = "02:00:00:00:ff:ff".into()
                }
                "protected" => {
                    i.details
                        .pifs
                        .get_mut(&uuid::Uuid::from_u128(0xf002).into())
                        .unwrap()
                        .disallow_unplug = Some(true)
                }
                "tag" => i.details.vlans.values_mut().next().unwrap().tag = Some(18),
                "class" => {
                    i.details.classes.insert(
                        "bond".into(),
                        xenadmin_core::ClassAvailability::PermissionDenied,
                    );
                }
                _ => unreachable!(),
            }
            assert!(
                review_unavailable(&s, s.network_workspace.as_ref().unwrap()).is_some(),
                "{change}"
            );
        }
    }
    #[test]
    fn bond_members_and_nested_vlan_targets_do_not_form_a_management_review() {
        let mut s = crate::init();
        topology_scene(&mut s);
        let i = s.inventory.as_ref().unwrap();
        assert!(ManagementInterface::capture(i, uuid::Uuid::from_u128(0xf002).into()).is_none());
        let source = editor(&mut s).source.pif;
        let i = Arc::make_mut(s.inventory.as_mut().unwrap());
        let parent = uuid::Uuid::from_u128(0xf001).into();
        let grandparent = uuid::Uuid::from_u128(0xf002).into();
        i.details.bonds.clear();
        i.details.pifs.get_mut(&parent).unwrap().vlan = Some(18);
        let id = uuid::Uuid::from_u128(0xf202).into();
        i.details.vlans.insert(
            id,
            xenadmin_core::Vlan {
                id,
                tagged_pif: Some(grandparent),
                untagged_pif: Some(parent),
                tag: Some(18),
            },
        );
        assert!(ManagementInterface::capture(i, source).is_none());
    }
    #[test]
    fn automatic_address_review_captures_mode_expected_ip_and_does_not_dispatch_demo() {
        for size in [(513.0, 652.0), (1440.0, 900.0)] {
            let mut s = crate::init();
            dynamic_scene(&mut s);
            let review = s
                .network_workspace
                .as_ref()
                .unwrap()
                .review
                .as_ref()
                .unwrap();
            assert!(
                matches!(&review.operation,ResourceAction::ManagementNetwork(op) if matches!(&**op,ManagementNetworkOperation::ConfigureDynamic { mode:ManagementDynamicMode::Ipv4Dhcp,recovery,.. } if recovery.to_string()=="192.0.2.30"))
            );
            assert!(
                review
                    .lines
                    .iter()
                    .any(|v| v.contains("does not create a reservation"))
            );
            assert!(
                review
                    .lines
                    .iter()
                    .any(|v| v.contains("remains unresolved"))
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
    fn automatic_review_rejects_wrong_family_hostnames_noop_and_changed_lease() {
        let mut s = crate::init();
        dynamic_scene(&mut s);
        for value in [
            "2001:db8::30",
            "recovery.example.test",
            "192.0.2.30/24",
            "0.0.0.0",
        ] {
            edit(&mut s).review = None;
            editor(&mut s).recovery = TextEdit::new(value);
            prepare(&mut s);
            assert!(
                s.network_workspace.as_ref().unwrap().review.is_none(),
                "{value}"
            );
        }
        editor(&mut s).recovery = TextEdit::new("192.0.2.30");
        prepare(&mut s);
        let source = editor(&mut s).source.pif;
        Arc::make_mut(s.inventory.as_mut().unwrap())
            .details
            .pifs
            .get_mut(&source)
            .unwrap()
            .ip = "192.0.2.99".into();
        assert!(review_unavailable(&s, s.network_workspace.as_ref().unwrap()).is_some());
        let p = Arc::make_mut(s.inventory.as_mut().unwrap())
            .details
            .pifs
            .get_mut(&source)
            .unwrap();
        p.ip_configuration_mode = "DHCP".into();
        open(&mut s, source, false);
        editor(&mut s).automatic = Some(ManagementDynamicMode::Ipv4Dhcp);
        prepare(&mut s);
        assert!(s.network_workspace.as_ref().unwrap().review.is_none());
        let mut s = crate::init();
        scene(&mut s);
        let source = editor(&mut s).source.pif;
        open(&mut s, source, false);
        editor(&mut s).dns = TextEdit::new("192.0.2.53,2001:db8::99");
        prepare(&mut s);
        assert!(
            s.network_workspace.as_ref().unwrap().review.is_none(),
            "IPv4 editing must not count ignored IPv6 DNS as a change"
        );
    }
    #[test]
    fn ipv6_dhcp_and_autoconf_review_require_same_family_and_capture_stable_recovery() {
        for mode in [
            ManagementDynamicMode::Ipv6Dhcp,
            ManagementDynamicMode::Ipv6Autoconf,
        ] {
            let mut s = crate::init();
            scene(&mut s);
            let source = editor(&mut s).source.pif;
            let p = Arc::make_mut(s.inventory.as_mut().unwrap())
                .details
                .pifs
                .get_mut(&source)
                .unwrap();
            p.primary_address_type = Some("IPv6".into());
            p.ipv6_configuration_mode = "Static".into();
            p.ipv6 = vec!["2001:db8::10/64".into()];
            open(&mut s, source, false);
            editor(&mut s).automatic = Some(mode);
            editor(&mut s).recovery = TextEdit::new("2001:db8::30");
            prepare(&mut s);
            let review = s
                .network_workspace
                .as_ref()
                .unwrap()
                .review
                .as_ref()
                .unwrap();
            assert!(
                matches!(&review.operation,ResourceAction::ManagementNetwork(op) if matches!(&**op,ManagementNetworkOperation::ConfigureDynamic { mode:m,.. } if *m==mode))
            );
            assert!(review.lines.iter().any(|v| v.contains(
                if mode == ManagementDynamicMode::Ipv6Dhcp {
                    "incomplete DHCPv6"
                } else {
                    "replace manually configured DNS"
                }
            )));
            edit(&mut s).review = None;
            editor(&mut s).recovery = TextEdit::new("fe80::30");
            prepare(&mut s);
            assert!(s.network_workspace.as_ref().unwrap().review.is_none());
        }
    }
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
