use super::*;
use xenadmin_core::{
    BondHash, BondMode, Ipv4Configuration, Ipv4Mode, NetworkId, NetworkOperation, ObservedResource,
    OperationId, PifId, PifIdentity, ResourceCommand, ResourceTarget,
};

struct Review {
    target: ResourceTarget,
    observed: Option<ObservedResource>,
    operation: NetworkOperation,
    lines: Vec<String>,
}
enum Form {
    Browse,
    Vlan {
        parent: Option<PifId>,
        tag: TextEdit,
    },
    Bond {
        members: Vec<PifId>,
        mode: BondMode,
        hash: BondHash,
    },
    Mtu {
        value: TextEdit,
    },
    Ipv4 {
        id: PifId,
        identity: PifIdentity,
        before: Ipv4Configuration,
        mode: Ipv4Mode,
        address: Box<TextEdit>,
        netmask: Box<TextEdit>,
        gateway: Box<TextEdit>,
        dns: Box<TextEdit>,
    },
}
pub(super) struct Workspace {
    pool: PoolId,
    network: Option<NetworkId>,
    host: Option<HostId>,
    form: Form,
    review: Option<Review>,
    error: Option<String>,
}
pub(super) fn button(s: &State, k: &Kit) -> El {
    if !matches!(s.page, Page::Hosts | Page::Networks) {
        return el();
    }
    Button::new("Host networking…")
        .on_press(open)
        .build(k)
        .key("network-administration-open")
}
pub(super) fn open(s: &mut State) {
    let Some(inventory) = &s.inventory else {
        return;
    };
    let target = resource_admin::selected_target(s);
    let network = match target {
        Some(ResourceTarget::Network(id)) => Some(id),
        _ => None,
    };
    let host = match target {
        Some(ResourceTarget::Host(id)) => Some(id),
        _ => None,
    };
    s.resource_editor = None;
    s.network_workspace = Some(Workspace {
        pool: inventory.pool_id,
        network,
        host,
        form: Form::Browse,
        review: None,
        error: None,
    });
}
fn name(inventory: &Inventory, id: NetworkId) -> String {
    inventory
        .networks
        .iter()
        .find(|n| n.uuid == id.uuid())
        .map(|n| n.name.clone())
        .unwrap_or_else(|| id.to_string())
}
fn pif_name(inventory: &Inventory, id: PifId) -> String {
    inventory
        .details
        .pifs
        .get(&id)
        .map(|p| {
            format!(
                "{} · {}",
                p.host
                    .and_then(|id| inventory.hosts.get(&id))
                    .map(|h| h.name.as_str())
                    .unwrap_or("Unknown host"),
                p.device
            )
        })
        .unwrap_or_else(|| id.to_string())
}
fn edit(s: &mut State) -> &mut Workspace {
    s.network_workspace
        .as_mut()
        .expect("network workspace open")
}
fn set_form(s: &mut State, form: Form) {
    let w = edit(s);
    w.form = form;
    w.review = None;
    w.error = None;
}
fn prepare(s: &mut State, target: ResourceTarget, operation: NetworkOperation, lines: Vec<String>) {
    let observed = if s.demo {
        None
    } else {
        match s
            .snapshot
            .as_ref()
            .ok_or(xenadmin_core::Error::Stale)
            .and_then(|snap| snap.observe_resource(target))
        {
            Ok(observed) => Some(observed),
            Err(error) => {
                edit(s).error = Some(error.to_string());
                return;
            }
        }
    };
    let w = edit(s);
    w.review = Some(Review {
        target,
        observed,
        operation,
        lines,
    });
    w.error = None;
}
fn review_button(
    label: impl Into<String>,
    target: ResourceTarget,
    op: NetworkOperation,
    lines: Vec<String>,
    enabled: bool,
    k: &Kit,
) -> El {
    Button::new(label.into())
        .disabled(!enabled)
        .on_press(move |s: &mut State| prepare(s, target, op.clone(), lines.clone()))
        .build(k)
}
fn ipv4(pif: &xenadmin_core::PhysicalInterface) -> Option<Ipv4Configuration> {
    Some(Ipv4Configuration {
        mode: match pif.ip_configuration_mode.as_str() {
            "None" => Ipv4Mode::None,
            "DHCP" => Ipv4Mode::Dhcp,
            "Static" => Ipv4Mode::Static,
            _ => return None,
        },
        address: pif.ip.clone(),
        netmask: pif.netmask.clone(),
        gateway: pif.gateway.clone(),
        dns: pif.dns.clone(),
    })
}
fn open_ipv4(s: &mut State, id: PifId) {
    let Some(pif) = s.inventory.as_ref().and_then(|i| i.details.pifs.get(&id)) else {
        return;
    };
    let (Some(identity), Some(before)) = (PifIdentity::from_inventory(pif), ipv4(pif)) else {
        return;
    };
    set_form(
        s,
        Form::Ipv4 {
            id,
            identity,
            mode: before.mode,
            address: Box::new(TextEdit::new(before.address.clone())),
            netmask: Box::new(TextEdit::new(before.netmask.clone())),
            gateway: Box::new(TextEdit::new(before.gateway.clone())),
            dns: Box::new(TextEdit::new(before.dns.clone())),
            before,
        },
    );
}
fn form_review(s: &mut State) {
    let result = (|| -> Result<_, String> {
        let w = s
            .network_workspace
            .as_ref()
            .ok_or("Close and reopen networking")?;
        let i = s
            .inventory
            .as_ref()
            .filter(|i| i.pool_id == w.pool)
            .ok_or("The pool changed")?;
        Ok(match &w.form {
            Form::Vlan { parent, tag } => {
                let network = w.network.ok_or("Choose the destination network")?;
                let parent = parent.ok_or("Select a physical interface")?;
                let expected = PifIdentity::from_inventory(
                    i.details.pifs.get(&parent).ok_or("Interface disappeared")?,
                )
                .ok_or("Interface identity is incomplete")?;
                let tag = tag
                    .text
                    .parse::<u16>()
                    .ok()
                    .filter(|v| (1..=4094).contains(v))
                    .ok_or("Enter a VLAN tag from 1 to 4094")?;
                (ResourceTarget::Network(network),NetworkOperation::CreateVlan{parent,expected,tag},vec![format!("Destination: {} ({network})",name(i,network)),format!("Parent: {} ({parent})",pif_name(i,parent)),format!("VLAN tag: {tag}"),"Creates an interface on this host. Repeat on other hosts that need this network.".into()])
            }
            Form::Bond {
                members,
                mode,
                hash,
            } => {
                if !(2..=4).contains(&members.len()) {
                    return Err("Select two to four physical interfaces on one host".into());
                }
                let network = w.network.ok_or("Choose the destination network")?;
                let members = members
                    .iter()
                    .map(|id| {
                        PifIdentity::from_inventory(
                            i.details.pifs.get(id).ok_or("Interface disappeared")?,
                        )
                        .map(|identity| (*id, identity))
                        .ok_or("Interface identity is incomplete")
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                if members.iter().any(|(_, p)| p.host != members[0].1.host) {
                    return Err("All bond members must be on the same host".into());
                }
                let mut lines=vec![format!("Destination: {} ({network})",name(i,network)),format!("Mode: {} · hash: {}",mode.as_str(),hash.as_str()),"The host will inherit the primary member's MAC. Active IP, VLAN and VM attachments must be moved separately before creating this bond.".into()];
                lines.extend(
                    members
                        .iter()
                        .map(|(id, _)| format!("Member: {} ({id})", pif_name(i, *id))),
                );
                (
                    ResourceTarget::Network(network),
                    NetworkOperation::CreateBond {
                        members,
                        mode: *mode,
                        hash: *hash,
                    },
                    lines,
                )
            }
            Form::Mtu { value } => {
                let network = w.network.ok_or("Choose a network")?;
                let expected = i
                    .details
                    .networks
                    .get(&network)
                    .and_then(|n| n.mtu)
                    .ok_or("Current MTU is unavailable")?;
                let mtu = value
                    .text
                    .parse::<u64>()
                    .ok()
                    .filter(|v| (576..=65_535).contains(v))
                    .ok_or("Enter an MTU from 576 to 65535")?;
                if expected == mtu {
                    return Err("Choose a different MTU".into());
                }
                let pifs = i
                    .details
                    .pifs
                    .values()
                    .filter(|p| p.network == Some(network))
                    .map(|p| p.id)
                    .collect();
                (ResourceTarget::Network(network),NetworkOperation::SetMtu{expected,mtu,pifs},vec![format!("Network: {} ({network})",name(i,network)),format!("MTU: {expected} → {mtu}"),"Traffic may be interrupted. Confirm that the physical path supports the new MTU. Management interfaces are not changed by this workflow.".into()])
            }
            Form::Ipv4 {
                id,
                identity,
                before,
                mode,
                address,
                netmask,
                gateway,
                dns,
            } => {
                let configuration = Ipv4Configuration {
                    mode: *mode,
                    address: if *mode == Ipv4Mode::Static {
                        address.text.trim().into()
                    } else {
                        String::new()
                    },
                    netmask: if *mode == Ipv4Mode::Static {
                        netmask.text.trim().into()
                    } else {
                        String::new()
                    },
                    gateway: if *mode == Ipv4Mode::Static {
                        gateway.text.trim().into()
                    } else {
                        String::new()
                    },
                    dns: dns.text.trim().into(),
                };
                if !configuration.valid() || before == &configuration {
                    return Err("Enter valid changed IP settings, a contiguous netmask and comma-separated DNS addresses".into());
                }
                let lines=vec![format!("Interface: {} ({id})",pif_name(i,*id)),format!("Previous: {} {} / {} gateway {} DNS {}",before.mode.as_str(),before.address,before.netmask,before.gateway,before.dns),format!("New: {} {} / {} gateway {} DNS {}",mode.as_str(),configuration.address,configuration.netmask,configuration.gateway,configuration.dns),"Storage or other traffic using this interface may be interrupted. This does not change the host management interface.".into()];
                (
                    ResourceTarget::Pif(*id),
                    NetworkOperation::ConfigureIpv4 {
                        expected: identity.clone(),
                        before: before.clone(),
                        configuration,
                    },
                    lines,
                )
            }
            Form::Browse => return Err("Choose a change to review".into()),
        })
    })();
    match result {
        Ok((target, op, lines)) => prepare(s, target, op, lines),
        Err(error) => edit(s).error = Some(error),
    }
}
fn submit(s: &mut State) {
    let result = (|| {
        if !s.can_mutate() {
            return Err(xenadmin_core::Error::Stale);
        }
        let review = s
            .network_workspace
            .as_ref()
            .and_then(|w| w.review.as_ref())
            .ok_or(xenadmin_core::Error::Stale)?;
        let target = review.observed.ok_or(xenadmin_core::Error::Stale)?;
        s.connection
            .as_ref()
            .ok_or(xenadmin_core::Error::Closed)?
            .submit_resource(
                OperationId::new(),
                ResourceCommand::Network {
                    target,
                    operation: review.operation.clone(),
                },
            )
    })();
    match result {
        Ok(op) => {
            if let Some(w) = &s.waker {
                op.notify_on_change(w.clone());
            }
            s.resource_operations.push(op);
            set_form(s, Form::Browse);
        }
        Err(error) => edit(s).error = Some(error.to_string()),
    }
}
fn browse(s: &State, w: &Workspace, i: &Inventory, k: &Kit) -> El {
    let enabled = s.demo || s.can_mutate();
    let mut body = col().gap(12.0);
    if let Some(id) = w.network {
        body = body
            .child(heading(name(i, id), k))
            .child(caption(format!("Network {id}"), k).wrap_anywhere());
        body = body.child(
            row()
                .gap(8.0)
                .flex_wrap()
                .child(
                    Button::new("Add VLAN interface…")
                        .disabled(!enabled)
                        .on_press(|s: &mut State| {
                            set_form(
                                s,
                                Form::Vlan {
                                    parent: None,
                                    tag: TextEdit::new("100"),
                                },
                            )
                        })
                        .build(k),
                )
                .child(
                    Button::new("Create bond…")
                        .disabled(!enabled)
                        .on_press(|s: &mut State| {
                            set_form(
                                s,
                                Form::Bond {
                                    members: vec![],
                                    mode: BondMode::ActiveBackup,
                                    hash: BondHash::SourceMac,
                                },
                            )
                        })
                        .build(k),
                )
                .child(
                    Button::new("Change MTU…")
                        .disabled(!enabled)
                        .on_press(|s: &mut State| {
                            let value = s
                                .inventory
                                .as_ref()
                                .and_then(|i| i.details.networks.get(&edit_network(s)?))
                                .and_then(|n| n.mtu)
                                .unwrap_or(1500);
                            set_form(
                                s,
                                Form::Mtu {
                                    value: TextEdit::new(value.to_string()),
                                },
                            )
                        })
                        .build(k),
                ),
        );
        if let Some(n) = i.details.networks.get(&id).filter(|n| n.automatic_known) {
            let automatic = n.automatic.as_deref() != Some("false");
            body = body
                .child(caption(
                    format!(
                        "Automatic attachment to new VMs: {}",
                        if automatic { "enabled" } else { "disabled" }
                    ),
                    k,
                ))
                .child(review_button(
                    if automatic {
                        "Disable automatic attachment"
                    } else {
                        "Enable automatic attachment"
                    },
                    ResourceTarget::Network(id),
                    NetworkOperation::SetAutomaticAttachment {
                        expected: n.automatic.clone(),
                        enabled: !automatic,
                    },
                    vec![
                        format!("Network: {} ({id})", name(i, id)),
                        format!(
                            "Automatically attach this network to new VMs: {}",
                            !automatic
                        ),
                    ],
                    enabled,
                    k,
                ));
        }
    } else {
        body=body.child(caption("Choose a network to add VLAN interfaces, create bonds or edit attachment settings.",k).wrap());
        for n in &i.networks {
            let id = NetworkId::from(n.uuid);
            body = body.child(
                Button::new(n.name.clone())
                    .on_press(move |s: &mut State| {
                        edit(s).network = Some(id);
                    })
                    .build(k),
            );
        }
    }
    let hosts = i
        .hosts
        .values()
        .filter(|h| w.host.is_none_or(|id| id == h.id));
    for host in hosts {
        body = body.child(review_button(
            format!("Scan interfaces · {}", host.name),
            ResourceTarget::Host(host.id),
            NetworkOperation::ScanInterfaces,
            vec![
                format!("Host: {} ({})", host.name, host.id),
                "Refresh physical interface discovery on this host.".into(),
            ],
            enabled,
            k,
        ));
    }
    if let Some(igmp) = i
        .details
        .pool
        .as_ref()
        .and_then(|p| p.igmp_snooping_enabled)
    {
        body = body.child(review_button(
            if igmp {
                "Disable pool IGMP snooping"
            } else {
                "Enable pool IGMP snooping"
            },
            ResourceTarget::Pool(i.pool_id),
            NetworkOperation::SetIgmpSnooping {
                expected: igmp,
                enabled: !igmp,
            },
            vec![
                format!("Pool: {} ({})", i.pool_name, i.pool_id),
                format!("IGMP snooping: {} → {}", igmp, !igmp),
                "This changes multicast forwarding behavior across the pool.".into(),
            ],
            enabled,
            k,
        ));
    }
    body = body.child(heading("Interfaces", k));
    for p in i.details.pifs.values().filter(|p| {
        w.network.is_none_or(|id| p.network == Some(id))
            && w.host.is_none_or(|id| p.host == Some(id))
    }) {
        let mut card = col()
            .gap(6.0)
            .padding(10.0)
            .child(heading(pif_name(i, p.id), k))
            .child(
                caption(
                    format!(
                        "{}\nMAC {} · MTU {} · VLAN {}\n{} {} / {} · gateway {} · DNS {}",
                        p.id,
                        p.mac,
                        p.mtu
                            .map(|v| v.to_string())
                            .unwrap_or_else(|| "unknown".into()),
                        p.vlan
                            .map(|v| v.to_string())
                            .unwrap_or_else(|| "unknown".into()),
                        p.ip_configuration_mode,
                        p.ip,
                        p.netmask,
                        p.gateway,
                        p.dns
                    ),
                    k,
                )
                .wrap_anywhere(),
            );
        if p.management == Some(true) {
            card=card.child(caption("Host management interface. Address and bond changes require a recovery workflow that is still in development.",k).wrap());
        }
        if let (Some(expected), Some(attached)) =
            (PifIdentity::from_inventory(p), p.currently_attached)
        {
            card = card.child(review_button(
                if attached {
                    "Unplug interface"
                } else {
                    "Plug interface"
                },
                ResourceTarget::Pif(p.id),
                NetworkOperation::SetAttached {
                    expected,
                    expected_attached: attached,
                    attached: !attached,
                },
                vec![
                    format!("Interface: {} ({})", pif_name(i, p.id), p.id),
                    format!("Attached: {attached} → {}", !attached),
                    "Unplugging interrupts VM and storage traffic on this interface.".into(),
                ],
                enabled && (!attached || p.management == Some(false)),
                k,
            ));
        }
        let id = p.id;
        card = card.child(
            Button::new("IPv4 configuration…")
                .disabled(!enabled || p.management != Some(false) || ipv4(p).is_none())
                .on_press(move |s: &mut State| open_ipv4(s, id))
                .build(k),
        );
        for vlan in i
            .details
            .vlans
            .values()
            .filter(|v| v.untagged_pif == Some(p.id))
        {
            if let (Some(parent), Some(tag), Some(network)) = (
                vlan.tagged_pif,
                vlan.tag.and_then(|t| u16::try_from(t).ok()),
                p.network,
            ) {
                card=card.child(review_button("Delete VLAN interface…",ResourceTarget::Vlan(vlan.id),NetworkOperation::DeleteVlan{parent,child:p.id,network,tag},vec![format!("VLAN {} · tag {tag}",vlan.id),format!("Child interface: {}",p.id),"Deletes this host's VLAN interface. The network record and other hosts' interfaces remain. Attached VMs must be moved first.".into()],enabled && p.management==Some(false),k));
            }
        }
        for bond in i.details.bonds.values().filter(|b| b.master == Some(p.id)) {
            card = card.child(
                caption(
                    format!(
                        "Bond {} · {} · hash {}",
                        bond.id,
                        bond.mode,
                        bond.hashing_algorithm.as_deref().unwrap_or("default")
                    ),
                    k,
                )
                .wrap_anywhere(),
            );
            for mode in [BondMode::BalanceSlb, BondMode::ActiveBackup, BondMode::Lacp] {
                if bond.mode == mode.as_str() {
                    continue;
                }
                card=card.child(review_button(format!("Change to {}…",mode.as_str()),ResourceTarget::Bond(bond.id),NetworkOperation::SetBondMode{expected_mode:bond.mode.clone(),mode},vec![format!("Bond {}: {} → {}",bond.id,bond.mode,mode.as_str()),"Coordinate switch configuration first. Network traffic may be interrupted.".into()],enabled && p.management==Some(false),k));
            }
            if bond.mode == "lacp" && bond.hashing_algorithm_known {
                for hash in [BondHash::SourceMac, BondHash::TcpUdpPorts] {
                    if bond.hashing_algorithm.as_deref() == Some(hash.as_str()) {
                        continue;
                    }
                    card = card.child(review_button(
                        format!("Use {} hash…", hash.as_str()),
                        ResourceTarget::Bond(bond.id),
                        NetworkOperation::SetBondHash {
                            expected: bond.hashing_algorithm.clone(),
                            hash,
                        },
                        vec![
                            format!("Bond {} hash → {}", bond.id, hash.as_str()),
                            "Traffic distribution changes immediately.".into(),
                        ],
                        enabled && p.management == Some(false),
                        k,
                    ));
                }
            }
            if let Some(primary) = bond.primary_slave {
                card=card.child(review_button("Destroy bond…",ResourceTarget::Bond(bond.id),NetworkOperation::DeleteBond{master:p.id,members:bond.slaves.clone(),primary},vec![format!("Bond {}",bond.id),format!("Primary member: {} ({primary})",pif_name(i,primary)),"The host returns the bond configuration to its primary member. This workflow requires VM, VLAN and IP users to be moved first.".into()],enabled && p.management==Some(false),k));
            }
        }
        body = body.child(card);
    }
    body
}
fn edit_network(s: &State) -> Option<NetworkId> {
    s.network_workspace.as_ref()?.network
}
fn form(s: &State, w: &Workspace, i: &Inventory, k: &Kit) -> El {
    let mut body = col().gap(10.0);
    if let Some(network) = w.network {
        body = body.child(
            caption(format!("Network: {} ({network})", name(i, network)), k).wrap_anywhere(),
        );
    }
    match &w.form {
        Form::Browse => return browse(s, w, i, k),
        Form::Vlan { parent, tag } => {
            body = body
                .child(heading("Add VLAN interface", k))
                .child(caption("VLAN tag (1–4094)", k))
                .child(
                    TextInput::new(tag)
                        .label("VLAN tag (1–4094)")
                        .on_edit(|s: &mut State| {
                            let Form::Vlan { tag, .. } = &mut edit(s).form else {
                                unreachable!()
                            };
                            tag
                        })
                        .build(k),
                );
            for p in i.details.pifs.values().filter(|p| {
                p.vlan == Some(-1)
                    && w.host.is_none_or(|id| p.host == Some(id))
                    && p.network != w.network
            }) {
                let id = p.id;
                body = body.child(
                    Button::new(format!(
                        "{}{} · {}",
                        if *parent == Some(id) { "✓ " } else { "" },
                        pif_name(i, id),
                        p.mac
                    ))
                    .on_press(move |s: &mut State| {
                        if let Form::Vlan { parent, .. } = &mut edit(s).form {
                            *parent = Some(id);
                        }
                    })
                    .build(k),
                );
            }
        }
        Form::Bond {
            members,
            mode,
            hash,
        } => {
            body=body.child(heading("Create bond",k)).child(caption("Select two to four unused physical NICs on the same host. The switch must be configured for the chosen mode.",k).wrap());
            for selected in [BondMode::ActiveBackup, BondMode::BalanceSlb, BondMode::Lacp] {
                body = body.child(
                    Button::new(format!(
                        "{}{}",
                        if mode == &selected { "✓ " } else { "" },
                        selected.as_str()
                    ))
                    .on_press(move |s: &mut State| {
                        if let Form::Bond { mode, .. } = &mut edit(s).form {
                            *mode = selected;
                        }
                    })
                    .build(k),
                );
            }
            if *mode == BondMode::Lacp {
                for selected in [BondHash::SourceMac, BondHash::TcpUdpPorts] {
                    body = body.child(
                        Button::new(format!(
                            "{}Hash: {}",
                            if hash == &selected { "✓ " } else { "" },
                            selected.as_str()
                        ))
                        .on_press(move |s: &mut State| {
                            if let Form::Bond { hash, .. } = &mut edit(s).form {
                                *hash = selected;
                            }
                        })
                        .build(k),
                    );
                }
            }
            for p in i.details.pifs.values().filter(|p| {
                p.physical == Some(true)
                    && p.management == Some(false)
                    && p.vlan == Some(-1)
                    && p.network != w.network
                    && w.host.is_none_or(|id| p.host == Some(id))
            }) {
                let id = p.id;
                body = body.child(
                    Button::new(format!(
                        "{}{} · {}",
                        if members.contains(&id) { "✓ " } else { "" },
                        pif_name(i, id),
                        p.mac
                    ))
                    .on_press(move |s: &mut State| {
                        if let Form::Bond { members, .. } = &mut edit(s).form {
                            if members.contains(&id) {
                                members.retain(|m| *m != id);
                            } else if members.len() < 4 {
                                members.push(id);
                            }
                        }
                    })
                    .build(k),
                );
            }
        }
        Form::Mtu { value } => {
            body = body
                .child(heading("Network MTU", k))
                .child(caption("MTU in bytes", k))
                .child(
                    TextInput::new(value)
                        .label("MTU in bytes")
                        .on_edit(|s: &mut State| {
                            let Form::Mtu { value } = &mut edit(s).form else {
                                unreachable!()
                            };
                            value
                        })
                        .build(k),
                );
        }
        Form::Ipv4 {
            mode,
            address,
            netmask,
            gateway,
            dns,
            ..
        } => {
            body = body.child(heading("IPv4 configuration", k));
            for selected in [Ipv4Mode::None, Ipv4Mode::Dhcp, Ipv4Mode::Static] {
                body = body.child(
                    Button::new(format!(
                        "{}{}",
                        if mode == &selected { "✓ " } else { "" },
                        selected.as_str()
                    ))
                    .on_press(move |s: &mut State| {
                        if let Form::Ipv4 { mode, .. } = &mut edit(s).form {
                            *mode = selected;
                        }
                    })
                    .build(k),
                );
            }
            if *mode == Ipv4Mode::Static {
                body = body
                    .child(caption("IP address", k))
                    .child(
                        TextInput::new(address)
                            .label("IP address")
                            .on_edit(|s: &mut State| {
                                let Form::Ipv4 { address, .. } = &mut edit(s).form else {
                                    unreachable!()
                                };
                                address
                            })
                            .build(k),
                    )
                    .child(caption("Netmask", k))
                    .child(
                        TextInput::new(netmask)
                            .label("Netmask")
                            .on_edit(|s: &mut State| {
                                let Form::Ipv4 { netmask, .. } = &mut edit(s).form else {
                                    unreachable!()
                                };
                                netmask
                            })
                            .build(k),
                    )
                    .child(caption("Gateway (optional)", k))
                    .child(
                        TextInput::new(gateway)
                            .label("Gateway (optional)")
                            .on_edit(|s: &mut State| {
                                let Form::Ipv4 { gateway, .. } = &mut edit(s).form else {
                                    unreachable!()
                                };
                                gateway
                            })
                            .build(k),
                    );
            }
            body = body
                .child(caption("DNS servers, comma separated", k))
                .child(
                    TextInput::new(dns)
                        .label("DNS servers, comma separated")
                        .on_edit(|s: &mut State| {
                            let Form::Ipv4 { dns, .. } = &mut edit(s).form else {
                                unreachable!()
                            };
                            dns
                        })
                        .build(k),
                );
        }
    }
    body
}
pub(super) fn dialog(s: &State, k: &Kit) -> El {
    let Some(w) = &s.network_workspace else {
        return el();
    };
    let current = s.inventory.as_ref().filter(|i| i.pool_id == w.pool);
    let mut body = col().gap(12.0).child(heading("Host networking", k));
    if let Some(review) = &w.review {
        body = body.child(heading(review.operation.label(), k)).child(
            caption(
                format!("Target {} · pool {}", review.target.uuid(), w.pool),
                k,
            )
            .wrap_anywhere(),
        );
        for line in &review.lines {
            body = body.child(caption(line, k).wrap_anywhere());
        }
    } else if let Some(i) = current {
        body = body.child(form(s, w, i, k));
    } else {
        body =
            body.child(caption("The connection changed. Close and reopen networking.", k).wrap());
    }
    if let Some(error) = &w.error {
        body = body.child(
            caption(error, k)
                .wrap()
                .role("alert")
                .color(k.c.danger.icon),
        );
    }
    if let Some(record) = s.resource_rows.iter().rev().find(|r| r.pool == w.pool) {
        body = body.child(
            caption(
                format!(
                    "Latest operation: {} · {:?} · {}",
                    record.action.label(),
                    record.state,
                    record.detail
                ),
                k,
            )
            .wrap(),
        );
    }
    let mut footer = row().gap(8.0).justify_end().child(
        Button::new("Close")
            .variant(ButtonVariant::Ghost)
            .on_press(|s: &mut State| s.network_workspace = None)
            .build(k),
    );
    if w.review.is_none() && matches!(w.form, Form::Browse) {
        footer = footer.child(
            Button::new("Operation history")
                .on_press(|s: &mut State| {
                    s.network_workspace = None;
                    resource_admin::open_pool(s, resource_admin::Kind::History);
                })
                .build(k),
        );
    }
    if w.review.is_some() {
        footer = footer
            .child(
                Button::new("Back")
                    .on_press(|s: &mut State| edit(s).review = None)
                    .build(k),
            )
            .child(
                Button::new("Confirm network change")
                    .disabled(
                        !s.can_mutate()
                            || w.review
                                .as_ref()
                                .and_then(|r| r.observed)
                                .is_none_or(|observed| {
                                    s.snapshot.as_ref().and_then(|snap| {
                                        snap.observe_resource(observed.target()).ok()
                                    }) != Some(observed)
                                }),
                    )
                    .on_press(submit)
                    .build(k)
                    .key("network-confirm"),
            );
    } else if !matches!(w.form, Form::Browse) {
        footer = footer
            .child(
                Button::new("Back")
                    .on_press(|s: &mut State| set_form(s, Form::Browse))
                    .build(k),
            )
            .child(
                Button::new("Review network change")
                    .on_press(form_review)
                    .build(k)
                    .key("network-review"),
            );
    }
    el().w_full()
        .h_full()
        .absolute(0.0, 0.0)
        .center()
        .fill(rune::Color::BLACK.alpha(0.5))
        .role("dialog")
        .label("Host networking")
        .modal(false, |s: &mut State| s.network_workspace = None)
        .child(
            col()
                .w((s.window_width - 32.0).clamp(280.0, 760.0))
                .h((s.window_height - 56.0).clamp(280.0, 850.0))
                .padding(20.0)
                .gap(16.0)
                .radius(10.0)
                .fill(k.c.bg_base)
                .child(body.grow(1.0).min_h(0.0).scroll(rune::ScrollSpec {
                    stick_bottom: false,
                }))
                .child(footer),
        )
}
pub(super) fn scene(s: &mut State) {
    crate::demo(s);
    s.page = Page::Networks;
    let inventory = Arc::make_mut(s.inventory.as_mut().unwrap());
    let underlay = uuid::Uuid::from_u128(0x51000000000000000000000000000001);
    inventory.networks.push(xenadmin_core::Network {
        uuid: underlay,
        name: "Physical uplinks".into(),
        bridge: "xenbr1".into(),
    });
    let host = *inventory.hosts.keys().next().unwrap();
    for index in 0..2 {
        let id = PifId::from(uuid::Uuid::from_u128(
            0x52000000000000000000000000000001 + index,
        ));
        inventory.details.pifs.insert(
            id,
            xenadmin_core::PhysicalInterface {
                id,
                host: Some(host),
                network: Some(NetworkId::from(underlay)),
                device: format!("eth{index}"),
                mac: format!("02:00:00:00:00:{:02x}", index + 1),
                mtu: Some(1500),
                vlan: Some(-1),
                management: Some(false),
                physical: Some(true),
                currently_attached: Some(true),
                ip_configuration_mode: "None".into(),
                ip: String::new(),
                netmask: String::new(),
                gateway: String::new(),
                dns: String::new(),
                ipv6: vec![],
                ipv6_configuration_mode: "None".into(),
                ipv6_gateway: String::new(),
            },
        );
    }
    s.rebuild();
    s.network_workspace = Some(Workspace {
        pool: s.inventory.as_ref().unwrap().pool_id,
        network: s
            .inventory
            .as_ref()
            .unwrap()
            .networks
            .first()
            .map(|n| NetworkId::from(n.uuid)),
        host: None,
        form: Form::Vlan {
            parent: None,
            tag: TextEdit::new("100"),
        },
        review: None,
        error: None,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compact_and_wide_network_reviews_remain_visible_and_preview_cannot_submit() {
        for size in [(513.0, 652.0), (1440.0, 900.0)] {
            let mut s = crate::init();
            scene(&mut s);
            set_form(
                &mut s,
                Form::Mtu {
                    value: TextEdit::new("9000"),
                },
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
            let (x, y) = runtime.find("network-review").unwrap();
            assert!(x > 0.0 && x < size.0 && y > 0.0 && y < size.1);
            runtime.pointer_down(x, y);
            runtime.pointer_up(x, y);
            runtime.frame(0.0);
            let w = runtime.state.network_workspace.as_ref().unwrap();
            assert!(matches!(
                w.review.as_ref().unwrap().operation,
                NetworkOperation::SetMtu { mtu: 9000, .. }
            ));
            let (x, y) = runtime.find("network-confirm").unwrap();
            assert!(x > 0.0 && x < size.0 && y > 0.0 && y < size.1);
            runtime.pointer_down(x, y);
            runtime.pointer_up(x, y);
            runtime.frame(0.0);
            assert!(runtime.state.resource_operations.is_empty());
            assert!(
                runtime
                    .state
                    .network_workspace
                    .as_ref()
                    .unwrap()
                    .review
                    .is_some()
            );
        }
    }
    #[test]
    fn mtu_review_captures_original_value_instead_of_chasing_inventory() {
        let mut s = crate::init();
        scene(&mut s);
        set_form(
            &mut s,
            Form::Mtu {
                value: TextEdit::new("9000"),
            },
        );
        form_review(&mut s);
        let w = s.network_workspace.as_ref().unwrap();
        let review = w.review.as_ref().unwrap();
        assert!(matches!(
            review.operation,
            NetworkOperation::SetMtu {
                expected: 1500,
                mtu: 9000,
                ..
            }
        ));
        let network = w.network.unwrap();
        Arc::make_mut(s.inventory.as_mut().unwrap())
            .details
            .networks
            .get_mut(&network)
            .unwrap()
            .mtu = Some(8000);
        assert!(matches!(
            s.network_workspace
                .as_ref()
                .unwrap()
                .review
                .as_ref()
                .unwrap()
                .operation,
            NetworkOperation::SetMtu { expected: 1500, .. }
        ));
    }
}
