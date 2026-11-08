use super::*;
use xenadmin_core::{
    ManagementAddressFamily, ManagementBondOperation as Bond, ManagementInterface,
    ManagementIpv6Bond as Operation, OperationState,
};
pub(super) struct Editor {
    source: ManagementInterface,
    operation: Bond,
    temporary: Option<ManagementInterface>,
}
pub(super) struct Pending {
    query: xenadmin_core::ManagementIpv6BondReviewQuery,
    observed: ObservedResource,
    operation: Operation,
}
fn editor(s: &mut State) -> &mut Editor {
    let Form::Ipv6Bond(v) = &mut edit(s).form else {
        unreachable!()
    };
    v
}
pub(super) fn open(s: &mut State, id: PifId) {
    let Some(i) = s.inventory.as_ref() else {
        return;
    };
    let Some(source) = ManagementInterface::capture(i, id) else {
        return;
    };
    let Some(parent) = Bond::management_parent(&source) else {
        return;
    };
    let operation = if let Some(b) = source
        .topology
        .as_ref()
        .and_then(|t| t.bonds.values().find(|b| b.master == Some(parent)))
    {
        let Some(primary) = b.primary_slave else {
            return;
        };
        Bond::Destroy {
            bond: b.id,
            primary,
        }
    } else {
        let Some(member) = ManagementInterface::capture(i, parent) else {
            return;
        };
        Bond::Create {
            members: vec![member],
            network: NetworkId::from(uuid::Uuid::nil()),
            mode: BondMode::ActiveBackup,
            hash: None,
        }
    };
    set_form(
        s,
        Form::Ipv6Bond(Box::new(Editor {
            source,
            operation,
            temporary: None,
        })),
    );
}
fn operation(v: &Editor) -> Result<Operation, String> {
    Ok(Operation {
        purpose: Default::default(),
        source: v.source.clone(),
        temporary: v
            .temporary
            .clone()
            .ok_or("Choose the independent static IPv6 recovery interface")?,
        operation: v.operation.clone(),
        consumers: None,
        predecessor: None,
    })
}
fn lines(op: &Operation, i: &Inventory, stage: usize) -> Vec<String> {
    if op.purpose == xenadmin_core::ManagementIpv6Next::LeavePartial {
        return vec![format!("Leave the partial configuration after {stage}/5 verified stages. Host {} · exact predecessor {}",op.source.identity.host,op.predecessor.unwrap()),format!("Retained temporary interface {} · {}",op.temporary.pif,op.temporary.ipv6.addresses.join(", ")),"This only releases the local pool reservation. No XAPI calls, rollback or topology completion will be claimed. Temporary addresses and any created bond/VLAN objects remain in place.".into(),"Continue will be permanently disabled for this handover. Inspect the current host and use a new independent review for subsequent changes. An active or uncertain child cannot be abandoned through this action.".into()];
    }
    let mut lines=vec![format!("Host {} · stage {} of 5",op.source.identity.host,stage+1),format!("Original management: {} ({}) · {} · gateway {} · DNS {}",pif_name(i,op.source.pif),op.source.pif,op.source.ipv6.addresses.join(", "),op.source.ipv6.gateway,op.source.ipv6.dns),format!("Temporary management: {} ({}) · network {} · {} · gateway {} · DNS {}",pif_name(i,op.temporary.pif),op.temporary.pif,name(i,op.temporary.identity.network),op.temporary.ipv6.addresses.join(", "),op.temporary.ipv6.gateway,op.temporary.ipv6.dns),"The temporary interface must already be configured and reachable on HTTPS at the current connection port. Its component must be separate from the affected bond/VLAN graph, with no guest or service consumers. Its exact host certificate is checked on the login connection before any write; current CA, pin or skip-validation and proxy policy still apply.".into()];
    match &op.operation{Bond::Create{members,network,mode,hash}=>{lines.push(format!("Create bond on {} ({network}) · {}{}",name(i,*network),mode.as_str(),hash.map(|h|format!(" · {}",h.as_str())).unwrap_or_default()));for p in members{lines.push(format!("Member {} ({}) · MAC {} · network {}",pif_name(i,p.pif),p.pif,p.identity.mac,name(i,p.identity.network)));}},Bond::Destroy{bond,primary}=>lines.push(format!("Remove bond {bond}; return the reviewed management endpoint/VLAN to primary {} ({primary})",pif_name(i,*primary)))}
    let components = match &op.operation {
        Bond::Create { members, .. } => members
            .iter()
            .filter_map(|p| p.topology.as_deref())
            .collect::<Vec<_>>(),
        Bond::Destroy { .. } => op.source.topology.as_deref().into_iter().collect(),
    };
    for t in components {
        for vlan in t.vlans.values() {
            if let Some(p) = vlan.untagged_pif.and_then(|id| t.pifs.get(&id)) {
                lines.push(format!(
                    "Recreate VLAN {} · tag {} · PIF {} · network {} · IPv4 {} {} / {} · IPv6 {}",
                    vlan.id,
                    vlan.tag.unwrap_or_default(),
                    p.id,
                    p.network.map(|n| name(i, n)).unwrap_or_default(),
                    p.ip_configuration_mode,
                    p.ip,
                    p.netmask,
                    p.ipv6.join(", ")
                ));
            }
        }
    }
    for (n, label) in [
        "Move management to the verified temporary interface",
        "Clear IPv6 from the original non-management interface",
        "Change the reviewed bond and VLAN topology",
        "Restore the original IPv6 settings on the proven target",
        "Verify the target path and return management",
    ]
    .iter()
    .enumerate()
    {
        lines.push(format!(
            "{}{}. {label}",
            if n == stage { "NEXT: " } else { "" },
            n + 1
        ));
    }
    if let Some(consumers) = &op.consumers {
        for (id, n) in &consumers.networks {
            lines.push(format!(
                "Network {} ({id}) · MTU {} · default locking {}",
                n.name,
                n.mtu,
                n.default_locking_mode.as_deref().unwrap_or("unavailable")
            ));
        }
        for v in consumers.interfaces.values() {
            lines.push(format!(
                "{}: {} ({}) · NIC {} ({}) · MAC {} · {} ({}) → {} ({}) · {:?}",
                v.change.label(),
                v.vm_name,
                v.vm,
                v.device,
                v.interface,
                v.mac,
                name(i, v.network),
                v.network,
                name(i, v.destination),
                v.destination,
                v.locality
            ));
        }
    }
    lines.extend(["Only the displayed next stage is submitted. After it and its authenticated host route are verified, review Continue for the next stage. The pool remains reserved throughout the unfinished handover.".into(),"Management and guest traffic can be interrupted. Configure the connected switch first. During stages 1–4, recover through the temporary address shown above. The temporary interface remains configured after completion.".into(),"Lost, failed or cancelled task receipts remain unresolved; Recheck only reads evidence and never repeats a write. Acknowledging an uncertain result does not authorize continuation.".into(),"This workflow requires one static IPv6 management address and members already using IPv6 as their primary family. Other affected VLANs may use static IPv4 or no IP. HA, clustering, service-purpose, tunnel and SR-IOV dependencies are excluded from this particular handover.".into()]);
    lines
}
fn finish(s: &mut State, op: Operation, stage: usize) {
    let Some(i) = s.inventory.as_ref().filter(|i| {
        s.network_workspace
            .as_ref()
            .is_some_and(|w| w.pool == i.pool_id)
    }) else {
        return;
    };
    let a = op.availability(i);
    if !a.is_available() {
        edit(s).error = Some(a.explanation());
        return;
    }
    let text = lines(&op, i, stage);
    prepare_action(
        s,
        ResourceTarget::Host(op.source.identity.host),
        xenadmin_core::ResourceAction::ManagementIpv6Bond(Box::new(op)),
        text,
    );
}
pub(super) fn prepare(s: &mut State) {
    let op = match operation(editor(s)) {
        Ok(v) => v,
        Err(e) => {
            edit(s).error = Some(e);
            return;
        }
    };
    if s.demo {
        finish(s, op, 0);
        return;
    }
    let result = (|| {
        let observed = s
            .snapshot
            .as_ref()
            .ok_or(xenadmin_core::Error::Stale)?
            .observe_resource(ResourceTarget::Host(op.source.identity.host))?;
        let query = s
            .connection
            .as_ref()
            .ok_or(xenadmin_core::Error::Closed)?
            .review_management_ipv6_bond(observed, op.clone())?;
        if let Some(w) = &s.waker {
            query.notify_on_change(w.clone());
        }
        Ok::<_, xenadmin_core::Error>(Pending {
            query,
            observed,
            operation: op,
        })
    })();
    match result {
        Ok(v) => {
            let w = edit(s);
            w.ipv6_review = Some(v);
            w.error = None;
        }
        Err(e) => edit(s).error = Some(e.to_string()),
    }
}
pub(super) fn poll(s: &mut State) -> bool {
    let Some(result) = s
        .network_workspace
        .as_ref()
        .and_then(|w| w.ipv6_review.as_ref())
        .and_then(|p| p.query.latest())
    else {
        return false;
    };
    let p = edit(s).ipv6_review.take().unwrap();
    let same = s
        .snapshot
        .as_ref()
        .and_then(|v| v.observe_resource(p.observed.target()).ok())
        == Some(p.observed)
        && s.network_workspace.as_ref().is_some_and(
            |w| matches!(&w.form,Form::Ipv6Bond(v) if operation(v).as_ref()==Ok(&p.operation)),
        );
    if !same {
        edit(s).error =
            Some("The connection or reviewed inputs changed; read the guest impact again".into());
    } else {
        match result {
            Ok(op) => finish(s, op, 0),
            Err(e) => edit(s).error = Some(e),
        }
    }
    true
}
pub(super) fn form(v: &Editor, i: &Inventory, k: &Kit) -> El {
    let mut body=col().gap(10.0).child(heading("Staged IPv6 management bond",k)).child(caption("Choose an independently reachable temporary interface. Each stage requires its own review; the temporary address remains configured.",k).wrap());
    if let Bond::Create {
        members,
        network,
        mode,
        ..
    } = &v.operation
    {
        for p in i.details.pifs.values().filter(|p| {
            p.host == Some(v.source.identity.host)
                && p.physical == Some(true)
                && p.primary_address_type.as_deref() == Some("IPv6")
        }) {
            let Some(captured) = ManagementInterface::capture(i, p.id) else {
                continue;
            };
            let id = p.id;
            body = body.child(
                Button::new(format!(
                    "{} member {} ({id})",
                    if members.iter().any(|v| v.pif == id) {
                        "✓"
                    } else {
                        "+"
                    },
                    p.device
                ))
                .disabled(Some(id) == Bond::management_parent(&v.source))
                .on_press(move |s: &mut State| {
                    if let Bond::Create { members, .. } = &mut editor(s).operation {
                        if let Some(n) = members.iter().position(|p| p.pif == id) {
                            members.remove(n);
                        } else {
                            members.push(captured.clone());
                        }
                    }
                })
                .build(k)
                .key(format!("ipv6-bond-member-{id}")),
            );
        }
        for n in i.networks.iter().filter(|n| {
            !i.details
                .pifs
                .values()
                .any(|p| p.network.is_some_and(|id| id.uuid() == n.uuid))
                && !i
                    .details
                    .vifs
                    .values()
                    .any(|v| v.network.is_some_and(|id| id.uuid() == n.uuid))
        }) {
            let id = NetworkId::from(n.uuid);
            body = body.child(
                Button::new(format!(
                    "{} destination {} ({id})",
                    if *network == id { "✓" } else { "+" },
                    n.name
                ))
                .on_press(move |s: &mut State| {
                    if let Bond::Create { network, .. } = &mut editor(s).operation {
                        *network = id;
                    }
                })
                .build(k)
                .key(format!("ipv6-bond-network-{id}")),
            );
        }
        for next in [BondMode::ActiveBackup, BondMode::BalanceSlb, BondMode::Lacp] {
            body = body.child(
                Button::new(format!(
                    "{} {}",
                    if *mode == next { "✓" } else { "" },
                    next.as_str()
                ))
                .on_press(move |s: &mut State| {
                    if let Bond::Create { mode, hash, .. } = &mut editor(s).operation {
                        *mode = next;
                        *hash = (next == BondMode::Lacp).then_some(BondHash::SourceMac);
                    }
                })
                .build(k),
            );
        }
    }
    if let Bond::Create {
        mode: BondMode::Lacp,
        hash,
        ..
    } = &v.operation
    {
        for next in [BondHash::SourceMac, BondHash::TcpUdpPorts] {
            body = body.child(
                Button::new(format!(
                    "{} LACP hash {}",
                    if *hash == Some(next) { "✓" } else { "" },
                    next.as_str()
                ))
                .on_press(move |s: &mut State| {
                    if let Bond::Create { hash, .. } = &mut editor(s).operation {
                        *hash = Some(next);
                    }
                })
                .build(k),
            );
        }
    }
    body = body.child(heading("Independent temporary IPv6 path", k));
    for p in i
        .details
        .pifs
        .values()
        .filter(|p| p.host == Some(v.source.identity.host) && p.management == Some(false))
    {
        let Some(captured) = ManagementInterface::capture(i, p.id).filter(|p| {
            p.primary == ManagementAddressFamily::Ipv6
                && p.ipv6.mode == xenadmin_core::Ipv6Mode::Static
        }) else {
            continue;
        };
        let id = p.id;
        body = body.child(
            Button::new(format!(
                "{} {} · {} ({id})",
                if v.temporary.as_ref().is_some_and(|p| p.pif == id) {
                    "✓"
                } else {
                    "+"
                },
                p.device,
                captured.ipv6.addresses.join(", ")
            ))
            .on_press(move |s: &mut State| editor(s).temporary = Some(captured.clone()))
            .build(k)
            .key(format!("ipv6-bond-temporary-{id}")),
        );
    }
    if let Ok(op) = operation(v) {
        let a = op.availability(i);
        if !a.is_available() {
            body = body.child(caption(a.explanation(), k).wrap());
        }
    }
    body
}
pub(super) fn unavailable(s: &State, w: &Workspace) -> Option<String> {
    let r = w.review.as_ref()?;
    let xenadmin_core::ResourceAction::ManagementIpv6Bond(op) = &r.operation else {
        return None;
    };
    let i = s.inventory.as_ref().filter(|i| i.pool_id == w.pool)?;
    let a = op.availability(i);
    (!a.is_available()).then(|| a.explanation())
}
pub(super) fn continuation(s: &State, w: &Workspace, k: &Kit) -> El {
    let mut body = col().gap(8.0);
    for r in s.resource_rows.iter().filter(|r| {
        r.pool == w.pool
            && r.state == OperationState::PartiallyCompleted
            && r.management_ipv6_bond
                .as_ref()
                .is_some_and(|v| v.retains_reservation && v.successor.is_none())
    }) {
        let xenadmin_core::ResourceAction::ManagementIpv6Bond(op) = &r.action else {
            continue;
        };
        let Some(e) = &r.management_ipv6_bond else {
            continue;
        };
        let stage = e.completed_stages;
        let mut leave = (**op).clone();
        leave.predecessor = Some(r.id);
        leave.purpose = xenadmin_core::ManagementIpv6Next::LeavePartial;
        body = body.child(
            Button::new("Leave partial configuration…")
                .disabled(!s.can_mutate())
                .on_press(move |s: &mut State| finish(s, leave.clone(), stage))
                .build(k)
                .key(format!("ipv6-bond-leave-{}", r.id)),
        );
        let mut op = (**op).clone();
        op.predecessor = Some(r.id);
        body=body.child(caption(format!("IPv6 handover: {stage}/5 stages verified. Pool reserved. Temporary recovery: {}. {}",e.temporary_endpoint,r.detail),k).wrap()).child(Button::new(format!("Review stage {}…",stage+1)).disabled(!s.can_mutate()).on_press(move|s:&mut State|finish(s,op.clone(),stage)).build(k).key(format!("ipv6-bond-continue-{}",r.id)));
    }
    body
}
pub(super) fn scene(s: &mut State) {
    management_bond::scene(s);
    let (source, members, network) = {
        let w = s.network_workspace.as_ref().unwrap();
        let xenadmin_core::ResourceAction::ManagementNetwork(op) =
            &w.review.as_ref().unwrap().operation
        else {
            unreachable!()
        };
        let xenadmin_core::ManagementNetworkOperation::Bond {
            source,
            operation: Bond::Create {
                members, network, ..
            },
            ..
        } = op.as_ref()
        else {
            unreachable!()
        };
        (
            source.pif,
            members.iter().map(|p| p.pif).collect::<Vec<_>>(),
            *network,
        )
    };
    let i = Arc::make_mut(s.inventory.as_mut().unwrap());
    for id in &members {
        let p = i.details.pifs.get_mut(id).unwrap();
        p.primary_address_type = Some("IPv6".into());
        p.ip_configuration_mode = "None".into();
        p.ip.clear();
        p.netmask.clear();
        p.gateway.clear();
        p.dns.clear();
        p.ipv6_configuration_mode = if *id == source { "Static" } else { "None" }.into();
        p.ipv6 = if *id == source {
            vec!["2001:db8:1::10/64".into()]
        } else {
            vec![]
        };
        p.ipv6_gateway.clear();
    }
    let temporary = PifId::from(uuid::Uuid::from_u128(0x6b001));
    let net = NetworkId::from(uuid::Uuid::from_u128(0x6b002));
    let mut p = i.details.pifs[&source].clone();
    p.id = temporary;
    p.network = Some(net);
    p.management = Some(false);
    p.device = "eth-recovery".into();
    p.mac = "02:00:00:00:06:01".into();
    p.ipv6 = vec!["2001:db8:2::10/64".into()];
    i.details.pifs.insert(temporary, p);
    i.networks.push(xenadmin_core::Network {
        uuid: net.uuid(),
        name: "Independent IPv6 recovery".into(),
        bridge: "xenbr-recovery".into(),
    });
    let captured = members
        .iter()
        .map(|id| ManagementInterface::capture(i, *id).unwrap())
        .collect();
    let recovery = ManagementInterface::capture(i, temporary).unwrap();
    open(s, source);
    let v = editor(s);
    v.operation = Bond::Create {
        members: captured,
        network,
        mode: BondMode::ActiveBackup,
        hash: None,
    };
    v.temporary = Some(recovery);
    prepare(s);
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ipv6_stage_review_renders_at_two_widths_and_keeps_demo_apply_disabled() {
        for size in [(513.0, 652.0), (1440.0, 900.0)] {
            let mut s = crate::init();
            scene(&mut s);
            let w = s.network_workspace.as_ref().unwrap();
            let r = w.review.as_ref().unwrap();
            assert!(r.lines.iter().any(|v| v.contains("stage 1 of 5")));
            assert!(
                r.lines
                    .iter()
                    .any(|v| v.contains("temporary interface remains configured"))
            );
            let xenadmin_core::ResourceAction::ManagementIpv6Bond(op) = &r.operation else {
                panic!()
            };
            assert_ne!(op.source.pif, op.temporary.pif);
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
    fn ipv6_review_refuses_changed_temporary_or_member_identity() {
        for temporary in [false, true] {
            let mut s = crate::init();
            scene(&mut s);
            let id = {
                let xenadmin_core::ResourceAction::ManagementIpv6Bond(op) = &s
                    .network_workspace
                    .as_ref()
                    .unwrap()
                    .review
                    .as_ref()
                    .unwrap()
                    .operation
                else {
                    panic!()
                };
                if temporary {
                    op.temporary.pif
                } else {
                    let Bond::Create { members, .. } = &op.operation else {
                        panic!()
                    };
                    members[1].pif
                }
            };
            Arc::make_mut(s.inventory.as_mut().unwrap())
                .details
                .pifs
                .get_mut(&id)
                .unwrap()
                .mac = "02:00:00:00:ff:ff".into();
            assert!(unavailable(&s, s.network_workspace.as_ref().unwrap()).is_some());
        }
    }
    #[test]
    fn ipv6_continuation_retains_original_review_and_targets_exact_predecessor() {
        let mut s = crate::init();
        scene(&mut s);
        let pool = s.inventory.as_ref().unwrap().pool_id;
        let action = s
            .network_workspace
            .as_ref()
            .unwrap()
            .review
            .as_ref()
            .unwrap()
            .operation
            .clone();
        let xenadmin_core::ResourceAction::ManagementIpv6Bond(op) = &action else {
            panic!()
        };
        let id = OperationId::new();
        let evidence = xenadmin_core::ManagementIpv6BondEvidence {
            left_partial: false,
            completed_stages: 1,
            retains_reservation: true,
            temporary_endpoint: xenadmin_core::Endpoint::parse("https://[2001:db8:2::10]").unwrap(),
            target_endpoint: xenadmin_core::Endpoint::parse("https://[2001:db8:1::10]").unwrap(),
            predecessor: None,
            successor: None,
            created_bond: None,
            moved_vlans: vec![],
        };
        let record:xenadmin_core::ResourceOperationRecord=serde_json::from_value(serde_json::json!({"id":id,"pool":pool,"target":ResourceTarget::Host(op.source.identity.host),"action":action,"state":OperationState::PartiallyCompleted,"progress":0.2,"detail":"Stage verified","created_at_ms":0,"can_cancel":false,"cancel_requested":false,"steps":[],"created":null,"management_ipv6_bond":evidence})).unwrap();
        s.resource_rows.push(record);
        let mut next = (**op).clone();
        next.predecessor = Some(id);
        finish(&mut s, next, 1);
        let w = s.network_workspace.as_ref().unwrap();
        let xenadmin_core::ResourceAction::ManagementIpv6Bond(next) =
            &w.review.as_ref().unwrap().operation
        else {
            panic!()
        };
        assert_eq!(next.predecessor, Some(id));
        assert_eq!(next.source, op.source);
        assert!(
            w.review
                .as_ref()
                .unwrap()
                .lines
                .iter()
                .any(|v| v.contains("NEXT: 2."))
        );
        let mut leave = next.as_ref().clone();
        leave.purpose = xenadmin_core::ManagementIpv6Next::LeavePartial;
        finish(&mut s, leave, 1);
        let w = s.network_workspace.as_ref().unwrap();
        assert!(
            w.review
                .as_ref()
                .unwrap()
                .lines
                .iter()
                .any(|v| v.contains("No XAPI calls"))
        );
        assert_eq!(
            w.review
                .as_ref()
                .unwrap()
                .operation
                .permission_requirements(w.review.as_ref().unwrap().target)
                .methods()
                .count(),
            0
        );
    }
}
