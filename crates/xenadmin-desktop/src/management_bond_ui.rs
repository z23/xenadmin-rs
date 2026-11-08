use super::*;
use xenadmin_core::{
    ManagementBondOperation as Bond, ManagementInterface, ManagementNetworkOperation as Operation,
};
pub(super) struct Editor {
    source: ManagementInterface,
    members: Vec<ManagementInterface>,
    network: Option<NetworkId>,
    mode: BondMode,
    hash: BondHash,
}
fn editor(s: &mut State) -> &mut Editor {
    let Form::ManagementBond(v) = &mut edit(s).form else {
        unreachable!()
    };
    v
}
fn open(s: &mut State, id: PifId) {
    let Some(source) = s
        .inventory
        .as_ref()
        .and_then(|i| ManagementInterface::capture(i, id))
    else {
        edit(s).error = Some("Management interface information is incomplete".into());
        return;
    };
    set_form(
        s,
        Form::ManagementBond(Box::new(Editor {
            members: vec![source.clone()],
            source,
            network: None,
            mode: BondMode::ActiveBackup,
            hash: BondHash::SourceMac,
        })),
    );
}
fn operation(v: &Editor) -> Result<Operation, String> {
    Ok(Operation::Bond {
        source: v.source.clone(),
        operation: Bond::Create {
            members: v.members.clone(),
            network: v.network.ok_or("Choose an empty destination network")?,
            mode: v.mode,
            hash: (v.mode == BondMode::Lacp).then_some(v.hash),
        },
    })
}
fn lines(op: &Operation, i: &Inventory) -> Vec<String> {
    let mut lines = vec![
        format!("Host: {}", op.source().identity.host),
        format!(
            "Current management: {} ({})",
            pif_name(i, op.source().pif),
            op.source().pif
        ),
        format!(
            "Static IPv4 {} / {} · gateway {} · DNS {}",
            op.source().ipv4.address,
            op.source().ipv4.netmask,
            op.source().ipv4.gateway,
            op.source().ipv4.dns
        ),
    ];
    if let Operation::Bond { operation, .. } = op {
        match operation {
            Bond::Create {
                members,
                network,
                mode,
                hash,
            } => {
                lines.push(format!(
                    "New bond network: {} ({network}) · mode {}{}",
                    name(i, *network),
                    mode.as_str(),
                    hash.map(|h| format!(" · hash {}", h.as_str()))
                        .unwrap_or_default()
                ));
                for m in members {
                    lines.push(format!(
                        "Member: {} ({}) · {} · network {}",
                        pif_name(i, m.pif),
                        m.pif,
                        m.identity.mac,
                        name(i, m.identity.network)
                    ));
                }
                lines.push("The host creates a new bond and moves the current static IPv4 settings to it. The original management member supplies the bond MAC. Member IP settings are cleared by the host.".into());
            }
            Bond::Destroy { bond, primary } => {
                lines.push(format!("Remove bond {bond}; return management and its existing IPv4 settings to {} ({primary})",pif_name(i,*primary)));
                lines.push("The host removes the bond master and returns its members to their original networks. The network objects are retained.".into());
            }
        }
    }
    lines.extend(["Traffic and the management connection can be interrupted. Configure the connected switch for the reviewed bond mode first and keep an independent recovery path available.".into(),format!("Recovery endpoint: {} using HTTPS on the current connection port. Success requires the same host certificate, exact resulting topology and address; existing trust settings still apply.",op.recovery_address().map(|v|v.to_string()).unwrap_or_default()),"Lost replies remain unresolved. Recheck reads existing task and topology evidence; no write, rollback or duplicate creation is sent. New bond/master UUIDs are retained when positively identified.".into(),"This workflow supports static IPv4 management without VLAN, guest-interface, clustering, storage-purpose, tunnel or SR-IOV dependencies. Those migrations require a separate reviewed workflow.".into()]);
    lines
}
fn review(s: &mut State, op: Operation) {
    let Some(i) = s.inventory.as_ref().filter(|i| {
        s.network_workspace
            .as_ref()
            .is_some_and(|w| w.pool == i.pool_id)
    }) else {
        edit(s).error = Some("The pool changed".into());
        return;
    };
    let available = op.availability(i);
    if !available.is_available() {
        edit(s).error = Some(available.explanation());
        return;
    }
    let text = lines(&op, i);
    prepare_action(
        s,
        ResourceTarget::Host(op.source().identity.host),
        xenadmin_core::ResourceAction::ManagementNetwork(Box::new(op)),
        text,
    );
}
pub(super) fn prepare(s: &mut State) {
    let result = operation(editor(s));
    match result {
        Ok(op) => review(s, op),
        Err(e) => edit(s).error = Some(e),
    }
}
fn remove(s: &mut State, id: PifId) {
    let Some(source) = s
        .inventory
        .as_ref()
        .and_then(|i| ManagementInterface::capture(i, id))
    else {
        return;
    };
    let Some(b) = source
        .topology
        .as_ref()
        .and_then(|t| t.bonds.values().find(|b| b.master == Some(id)))
    else {
        return;
    };
    let Some(primary) = b.primary_slave else {
        return;
    };
    let bond = b.id;
    review(
        s,
        Operation::Bond {
            source,
            operation: Bond::Destroy { bond, primary },
        },
    );
}
pub(super) fn buttons(
    id: PifId,
    p: &xenadmin_core::PhysicalInterface,
    enabled: bool,
    k: &Kit,
) -> El {
    if p.vlan != Some(-1) {
        return el();
    }
    row()
        .gap(8.0)
        .flex_wrap()
        .child(if p.physical == Some(true) {
            Button::new("Create management bond…")
                .disabled(!enabled)
                .on_press(move |s: &mut State| open(s, id))
                .build(k)
                .key("management-bond-create")
        } else {
            Button::new("Remove management bond…")
                .disabled(!enabled || p.vlan != Some(-1))
                .on_press(move |s: &mut State| remove(s, id))
                .build(k)
                .key("management-bond-remove")
        })
}
pub(super) fn form(v: &Editor, i: &Inventory, k: &Kit) -> El {
    let mut body = col()
        .gap(10.0)
        .child(heading("Create management bond", k))
        .child(
            caption(
                format!(
                    "Management member: {} · {}",
                    pif_name(i, v.source.pif),
                    v.source.ipv4.address
                ),
                k,
            )
            .wrap(),
        );
    body=body.child(caption("Choose 2–4 physical members on this host. Only the current management member may have IP settings.",k).wrap());
    for p in i.details.pifs.values().filter(|p| {
        p.host == Some(v.source.identity.host) && p.physical == Some(true) && p.vlan == Some(-1)
    }) {
        let Some(captured) = ManagementInterface::capture(i, p.id) else {
            continue;
        };
        let id = p.id;
        let selected = v.members.iter().any(|m| m.pif == id);
        body = body.child(
            Button::new(format!(
                "{} {} · {} ({id})",
                if selected { "✓" } else { "+" },
                p.device,
                p.mac
            ))
            .disabled(id == v.source.pif)
            .on_press(move |s: &mut State| {
                let v = editor(s);
                if let Some(index) = v.members.iter().position(|m| m.pif == id) {
                    v.members.remove(index);
                } else {
                    v.members.push(captured.clone());
                }
            })
            .build(k)
            .key(format!("management-bond-member-{id}")),
        );
    }
    body = body.child(caption(
        "Destination network (create an empty internal network first if needed)",
        k,
    ));
    for n in &i.networks {
        let id = NetworkId::from(n.uuid);
        if i.details.pifs.values().any(|p| p.network == Some(id))
            || i.details.vifs.values().any(|v| v.network == Some(id))
        {
            continue;
        }
        body = body.child(
            Button::new(format!(
                "{} {} ({id})",
                if v.network == Some(id) { "✓" } else { "+" },
                n.name
            ))
            .on_press(move |s: &mut State| editor(s).network = Some(id))
            .build(k)
            .key(format!("management-bond-network-{id}")),
        );
    }
    for mode in [BondMode::ActiveBackup, BondMode::BalanceSlb, BondMode::Lacp] {
        body = body.child(
            Button::new(format!(
                "{} {}",
                if v.mode == mode { "✓" } else { "" },
                mode.as_str()
            ))
            .on_press(move |s: &mut State| editor(s).mode = mode)
            .build(k)
            .key(format!("management-bond-mode-{}", mode.as_str())),
        );
    }
    if v.mode == BondMode::Lacp {
        for hash in [BondHash::SourceMac, BondHash::TcpUdpPorts] {
            body = body.child(
                Button::new(format!(
                    "{} {}",
                    if v.hash == hash { "✓" } else { "" },
                    hash.as_str()
                ))
                .on_press(move |s: &mut State| editor(s).hash = hash)
                .build(k),
            );
        }
    }
    if let Ok(op) = operation(v) {
        let a = op.availability(i);
        if !a.is_available() {
            body = body.child(caption(a.explanation(), k).wrap());
        }
    }
    body.child(
        caption(
            "Review the exact members, switch mode and recovery endpoint before applying.",
            k,
        )
        .wrap(),
    )
}
pub(super) fn scene(s: &mut State) {
    management::scene(s);
    let source = {
        let Form::Management(v) = &edit(s).form else {
            unreachable!()
        };
        v.source_id()
    };
    let i = Arc::make_mut(s.inventory.as_mut().unwrap());
    i.details.vifs.clear();
    let second = uuid::Uuid::from_u128(0xb001).into();
    let target = NetworkId::from(uuid::Uuid::from_u128(0xb002));
    let mut member = i.details.pifs[&source].clone();
    member.id = second;
    member.network = Some(uuid::Uuid::from_u128(0xb003).into());
    i.networks.push(xenadmin_core::Network {
        uuid: uuid::Uuid::from_u128(0xb003),
        name: "Second physical uplink".into(),
        bridge: "xenbr-secondary".into(),
    });
    member.device = "eth1".into();
    member.mac = "02:00:00:00:00:02".into();
    member.management = Some(false);
    member.ip_configuration_mode = "None".into();
    member.ip.clear();
    member.netmask.clear();
    member.gateway.clear();
    member.dns.clear();
    i.details.pifs.insert(second, member);
    i.networks.push(xenadmin_core::Network {
        uuid: target.uuid(),
        name: "Management bond network".into(),
        bridge: "xenbr-mgmt".into(),
    });
    for class in ["pif", "bond", "vlan", "vif"] {
        i.details
            .classes
            .insert(class.into(), xenadmin_core::ClassAvailability::Available);
    }
    let member = ManagementInterface::capture(i, second).unwrap();
    open(s, source);
    editor(s).members.push(member);
    editor(s).network = Some(target);
    prepare(s);
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn management_bond_review_has_exact_members_and_recovery_at_both_widths() {
        for size in [(513.0, 652.0), (1440.0, 900.0)] {
            let mut s = crate::init();
            scene(&mut s);
            let w = s.network_workspace.as_ref().unwrap();
            let r = w.review.as_ref().unwrap();
            let xenadmin_core::ResourceAction::ManagementNetwork(op) = &r.operation else {
                panic!()
            };
            let Operation::Bond {
                operation: Bond::Create { members, .. },
                ..
            } = op.as_ref()
            else {
                panic!()
            };
            assert_eq!(members.len(), 2);
            assert!(r.lines.iter().any(|v| v.contains("192.0.2.10")));
            assert!(r.lines.iter().any(|v| v.contains("Member:")));
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
    fn removal_review_captures_the_existing_primary_member_and_refuses_changed_bond() {
        let mut s = crate::init();
        scene(&mut s);
        let source = editor(&mut s).source.clone();
        let network = editor(&mut s).network.unwrap();
        let members = editor(&mut s).members.clone();
        let master = uuid::Uuid::from_u128(0xb004).into();
        let bond = uuid::Uuid::from_u128(0xb005).into();
        let i = Arc::make_mut(s.inventory.as_mut().unwrap());
        let mut p = i.details.pifs[&source.pif].clone();
        p.id = master;
        p.network = Some(network);
        p.physical = Some(false);
        p.device = "bond0".into();
        i.details.pifs.insert(master, p);
        for m in &members {
            let p = i.details.pifs.get_mut(&m.pif).unwrap();
            p.management = Some(false);
            p.ip_configuration_mode = "None".into();
            p.ip.clear();
            p.netmask.clear();
            p.gateway.clear();
            p.dns.clear();
        }
        i.details.bonds.insert(
            bond,
            xenadmin_core::NetworkBond {
                id: bond,
                master: Some(master),
                slaves: members.iter().map(|m| m.pif).collect(),
                primary_slave: Some(source.pif),
                mode: "active-backup".into(),
                hashing_algorithm: None,
                hashing_algorithm_known: true,
                links_up: Some(2),
            },
        );
        remove(&mut s, master);
        let r = s
            .network_workspace
            .as_ref()
            .unwrap()
            .review
            .as_ref()
            .unwrap();
        let xenadmin_core::ResourceAction::ManagementNetwork(op) = &r.operation else {
            panic!()
        };
        assert!(
            matches!(op.as_ref(),Operation::Bond{operation:Bond::Destroy{primary,..},..} if *primary==source.pif)
        );
        assert!(
            r.lines
                .iter()
                .any(|v| v.contains("network objects are retained"))
        );
        let i = Arc::make_mut(s.inventory.as_mut().unwrap());
        i.details.bonds.get_mut(&bond).unwrap().primary_slave = Some(members[1].pif);
        assert!(
            management::review_unavailable(&s, s.network_workspace.as_ref().unwrap()).is_some()
        );
    }
    #[test]
    fn changed_member_or_added_consumer_invalidates_captured_bond_review() {
        let mut s = crate::init();
        scene(&mut s);
        let before = s.inventory.clone().unwrap();
        let op = s
            .network_workspace
            .as_ref()
            .unwrap()
            .review
            .as_ref()
            .unwrap()
            .operation
            .clone();
        let xenadmin_core::ResourceAction::ManagementNetwork(op) = op else {
            panic!()
        };
        for fault in ["mac", "ip", "protected", "class", "pool"] {
            s.inventory = Some(before.clone());
            let i = Arc::make_mut(s.inventory.as_mut().unwrap());
            let p = i
                .details
                .pifs
                .get_mut(&uuid::Uuid::from_u128(0xb001).into())
                .unwrap();
            match fault {
                "mac" => p.mac = "02:00:00:00:ff:ff".into(),
                "ip" => p.ip = "192.0.2.99".into(),
                "protected" => p.disallow_unplug = Some(true),
                "class" => {
                    i.details.classes.remove("vif");
                }
                "pool" => i.pool_id = uuid::Uuid::new_v4().into(),
                _ => unreachable!(),
            };
            assert!(
                management::review_unavailable(&s, s.network_workspace.as_ref().unwrap()).is_some(),
                "{fault}"
            );
        }
        s.inventory = Some(before);
        assert!(
            op.availability(s.inventory.as_ref().unwrap())
                .is_available()
        );
    }
}
