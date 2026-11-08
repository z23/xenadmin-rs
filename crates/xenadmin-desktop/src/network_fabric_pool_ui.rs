use super::*;
use std::collections::BTreeMap;
use xenadmin_core::{
    NetworkFabricKind as Kind, NetworkFabricPool, NetworkTopologyOperation, ResourceAction,
    TunnelProtocol,
};
pub(super) struct Editor {
    network: NetworkId,
    kind: Kind,
    inputs: BTreeMap<HostId, PifId>,
}
fn open(s: &mut State, network: NetworkId, kind: Kind) {
    set_form(
        s,
        Form::FabricPool(Box::new(Editor {
            network,
            kind,
            inputs: BTreeMap::new(),
        })),
    );
}
fn review_lines(i: &Inventory, op: &NetworkFabricPool) -> Vec<String> {
    let first = &op.operations[0];
    let mut result=vec![format!("{} · network {} ({})",op.label(),name(i,first.review.network),first.review.network),
        format!("{} host step(s) · {} existing member(s) · {} current pool hosts",op.operations.len(),first.review.members.len(),first.review.hosts.len()),
        "Every current pool host must be online. Steps run serially; creation starts with the coordinator when needed, and removal leaves its interface until last. Existing members are preserved when completing coverage.".into(),
        "A failed or interrupted step stops the remaining hosts. Recheck only reads submitted work; it never repeats a write or starts the remaining steps. Review current coverage again before a new operation.".into(),
        "Completion verifies configuration. Controller activity, inter-host traffic and usable hardware VF capacity remain separate checks. No host reboot is submitted.".into()];
    for m in first.review.members.values() {
        result.push(format!(
            "Captured existing member: object {} · interface {} · host {}",
            m.object.uuid(),
            m.interface.id,
            m.interface
                .host
                .map(|id| id.to_string())
                .unwrap_or_else(|| "Unknown".into())
        ));
    }
    for (index, item) in op.operations.iter().enumerate() {
        result.push(format!(
            "Step {}: {} · host {} · input {}",
            index + 1,
            item.label(),
            item.review.host,
            item.review.input.id
        ));
        result.extend(super::fabric::lines(i, item).into_iter().filter(|line| {
            !line.contains("this host only") && !line.starts_with("Existing target coverage:")
        }));
    }
    result
}
pub(super) fn buttons(i: &Inventory, network: NetworkId, enabled: bool, k: &Kit) -> El {
    let mut body = col().gap(8.0).child(heading("Pool-wide coverage", k));
    for (label, kind) in [
        ("Complete GRE coverage…", Kind::Tunnel(TunnelProtocol::Gre)),
        (
            "Complete VXLAN coverage…",
            Kind::Tunnel(TunnelProtocol::Vxlan),
        ),
        ("Complete SR-IOV coverage…", Kind::Sriov),
    ] {
        body = body.child(
            Button::new(label)
                .disabled(!enabled)
                .on_press(move |s: &mut State| open(s, network, kind))
                .build(k),
        );
        if let Ok(op) = NetworkFabricPool::remove(i, network, kind) {
            let lines = review_lines(i, &op);
            body = body.child(
                Button::new(format!("{}…", op.label()))
                    .disabled(!enabled)
                    .on_press(move |s: &mut State| {
                        prepare_action(
                            s,
                            ResourceTarget::Network(network),
                            ResourceAction::NetworkTopology(NetworkTopologyOperation::Fabric(
                                Box::new(op.clone()),
                            )),
                            lines.clone(),
                        )
                    })
                    .build(k),
            );
        }
    }
    body
}
pub(super) fn form(e: &Editor, i: &Inventory, k: &Kit) -> El {
    let mut body=col().gap(8.0).child(heading("Complete pool network coverage",k)).child(caption("Choose one input on every host that is missing coverage. Existing interfaces are captured and preserved. Every host must be online; SR-IOV changes additionally require disabled, evacuated hosts and disabled HA.",k).wrap());
    for host in i.hosts.values() {
        body = body.child(heading(host.name.clone(), k));
        let members = i
            .details
            .pifs
            .values()
            .filter(|p| p.network == Some(e.network) && p.host == Some(host.id))
            .collect::<Vec<_>>();
        if !members.is_empty() {
            for p in members {
                body = body.child(
                    caption(format!("Existing interface {} · preserved", p.id), k).wrap_anywhere(),
                );
            }
            continue;
        }
        body = body.child(caption("Missing coverage — select an input", k));
        for p in i.details.pifs.values().filter(|p| {
            p.host == Some(host.id)
                && p.network != Some(e.network)
                && (e.kind != Kind::Sriov || p.physical == Some(true))
        }) {
            let available = NetworkFabricPool::review_input(i, e.network, e.kind, p.id);
            let id = p.id;
            let host = host.id;
            body = body.child(
                Button::new(format!(
                    "{}{} · {}",
                    if e.inputs.get(&host) == Some(&id) {
                        "✓ "
                    } else {
                        ""
                    },
                    pif_name(i, id),
                    p.mac
                ))
                .disabled(available.is_err())
                .on_press(move |s: &mut State| {
                    if let Form::FabricPool(e) = &mut edit(s).form {
                        e.inputs.insert(host, id);
                    }
                })
                .build(k),
            );
            if let Err(error) = available {
                let reason = match error {
                    xenadmin_core::Error::Protocol(message) => message.into(),
                    other => other.to_string(),
                };
                body = body.child(caption(reason, k).wrap());
            }
        }
    }
    body
}
fn reviewed(e: &Editor, i: &Inventory) -> Result<(NetworkFabricPool, Vec<String>), String> {
    let op = NetworkFabricPool::create(i, e.network, e.kind, e.inputs.values().copied().collect())
        .map_err(|e| e.to_string())?;
    let lines = review_lines(i, &op);
    Ok((op, lines))
}
pub(super) fn prepare(s: &mut State) {
    let result = (|| {
        let w = s.network_workspace.as_ref().ok_or("Reopen networking")?;
        let Form::FabricPool(e) = &w.form else {
            return Err("Choose pool coverage".into());
        };
        let i = s
            .inventory
            .as_ref()
            .filter(|i| i.pool_id == w.pool)
            .ok_or("The pool changed")?;
        let (op, lines) = reviewed(e, i)?;
        Ok::<_, String>((e.network, op, lines))
    })();
    match result {
        Ok((network, op, lines)) => prepare_action(
            s,
            ResourceTarget::Network(network),
            ResourceAction::NetworkTopology(NetworkTopologyOperation::Fabric(Box::new(op))),
            lines,
        ),
        Err(e) => edit(s).error = Some(e),
    }
}
pub(super) fn scene(s: &mut State) {
    super::fabric::scene(s);
    let i = Arc::make_mut(s.inventory.as_mut().unwrap());
    let network = s.network_workspace.as_ref().unwrap().network.unwrap();
    let original = i.hosts.values().next().unwrap().clone();
    let mut second = original.clone();
    second.id = HostId::from(uuid::Uuid::from_u128(0x22000000000000000000000000000022));
    second.name = "Synthetic second host".into();
    second.address = "192.0.2.21".into();
    i.details
        .hosts
        .insert(second.id, i.details.hosts[&original.id].clone());
    let mut input = i
        .details
        .pifs
        .values()
        .find(|p| p.host == Some(original.id) && p.network != Some(network))
        .unwrap()
        .clone();
    let first = input.id;
    input.id = PifId::from(uuid::Uuid::from_u128(0x77000000000000000000000000000022));
    input.host = Some(second.id);
    input.ip = "192.0.2.21".into();
    let next = input.id;
    i.details.pifs.insert(next, input);
    i.hosts.insert(second.id, second);
    let kind = Kind::Tunnel(TunnelProtocol::Vxlan);
    let op = NetworkFabricPool::create(i, network, kind, vec![first, next]).unwrap();
    let lines = review_lines(i, &op);
    prepare_action(
        s,
        ResourceTarget::Network(network),
        ResourceAction::NetworkTopology(NetworkTopologyOperation::Fabric(Box::new(op))),
        lines,
    );
}
pub(super) fn selection_scene(s: &mut State) {
    scene(s);
    let w = s.network_workspace.as_ref().unwrap();
    let ResourceAction::NetworkTopology(NetworkTopologyOperation::Fabric(op)) =
        &w.review.as_ref().unwrap().operation
    else {
        unreachable!()
    };
    let editor = Editor {
        network: op.operations[0].review.network,
        kind: op.operations[0].review.kind,
        inputs: op
            .operations
            .iter()
            .map(|op| (op.review.host, op.review.input.id))
            .collect(),
    };
    set_form(s, Form::FabricPool(Box::new(editor)));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_order_and_all_hosts_are_reviewed_at_both_widths_and_demo_cannot_apply() {
        for size in [(513.0, 652.0), (1440.0, 900.0)] {
            let mut s = crate::init();
            scene(&mut s);
            let w = s.network_workspace.as_ref().unwrap();
            let ResourceAction::NetworkTopology(NetworkTopologyOperation::Fabric(op)) =
                &w.review.as_ref().unwrap().operation
            else {
                panic!("pool review")
            };
            assert_eq!(op.operations.len(), 2);
            assert_eq!(
                op.operations[0].review.host,
                op.operations[0].review.coordinator
            );
            assert!(
                w.review
                    .as_ref()
                    .unwrap()
                    .lines
                    .iter()
                    .any(|l| l.contains("never repeats a write"))
            );
            assert!(super::super::topology::review_unavailable(&s, w).is_none());
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
            let (x, y) = rt.find("network-confirm").unwrap();
            assert!(x > 0.0 && x < size.0 && y > 0.0 && y < size.1);
            rt.pointer_down(x, y);
            rt.pointer_up(x, y);
            for _ in 0..30 {
                rt.frame(1.0 / 60.0);
            }
            assert!(rt.state.resource_operations.is_empty());
        }
    }
    #[test]
    fn pool_controller_and_input_changes_invalidate_captured_review() {
        for change in 0..3 {
            let mut s = crate::init();
            scene(&mut s);
            let i = Arc::make_mut(s.inventory.as_mut().unwrap());
            match change {
                0 => {
                    let id = *i.hosts.keys().next_back().unwrap();
                    i.hosts.remove(&id);
                }
                1 => {
                    i.details
                        .network_fabric
                        .controllers
                        .values_mut()
                        .next()
                        .unwrap()
                        .configuration = None
                }
                _ => {
                    i.details
                        .pifs
                        .values_mut()
                        .find(|p| p.ip == "192.0.2.21")
                        .unwrap()
                        .mac = "02:00:00:00:00:ff".into()
                }
            }
            assert!(
                super::super::topology::review_unavailable(
                    &s,
                    s.network_workspace.as_ref().unwrap()
                )
                .is_some()
            );
        }
    }
    #[test]
    fn input_editor_requires_all_missing_hosts_and_preserves_exact_selection() {
        let mut s = crate::init();
        scene(&mut s);
        let i = s.inventory.as_ref().unwrap();
        let network = s.network_workspace.as_ref().unwrap().network.unwrap();
        let mut e = Editor {
            network,
            kind: Kind::Tunnel(TunnelProtocol::Vxlan),
            inputs: BTreeMap::new(),
        };
        assert!(reviewed(&e, i).is_err());
        for p in i
            .details
            .pifs
            .values()
            .filter(|p| p.network != Some(network) && p.ip_configuration_mode == "Static")
        {
            e.inputs.insert(p.host.unwrap(), p.id);
        }
        assert_eq!(reviewed(&e, i).unwrap().0.operations.len(), 2);
        let host = *e.inputs.keys().next().unwrap();
        e.inputs.remove(&host);
        assert!(reviewed(&e, i).is_err());
        for size in [(513.0, 652.0), (1440.0, 900.0)] {
            let mut s = crate::init();
            selection_scene(&mut s);
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
            let (x, y) = rt.find("network-review").unwrap();
            assert!(x > 0.0 && x < size.0 && y > 0.0 && y < size.1);
            rt.pointer_down(x, y);
            rt.pointer_up(x, y);
            for _ in 0..30 {
                rt.frame(1.0 / 60.0);
            }
            let w = rt.state.network_workspace.as_ref().unwrap();
            assert!(matches!(
                w.review.as_ref().unwrap().operation,
                ResourceAction::NetworkTopology(NetworkTopologyOperation::Fabric(_))
            ));
        }
    }
}
