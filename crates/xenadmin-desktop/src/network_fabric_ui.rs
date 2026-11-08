use super::*;
use xenadmin_core::{
    NetworkFabricChange as Change, NetworkFabricKind as Kind, NetworkFabricObject as Object,
    NetworkFabricOperation as Operation, TunnelProtocol,
};
pub(super) struct Editor {
    network: NetworkId,
    kind: Kind,
    input: Option<PifId>,
}
fn lines(i: &Inventory, op: &Operation) -> Vec<String> {
    let r = &op.review;
    let mut result = vec![
        format!("Network: {} ({})", name(i, r.network), r.network),
        format!(
            "Host: {} ({})",
            i.hosts
                .get(&r.host)
                .map(|h| h.name.as_str())
                .unwrap_or("Unknown"),
            r.host
        ),
        format!(
            "Input: {} · interface {} · MAC {}",
            r.input.device, r.input.id, r.input.mac
        ),
        format!(
            "{} on this host only. Other pool hosts are not changed.",
            op.label()
        ),
        format!("Target MTU {} · default locking {}", r.mtu, r.locking_mode),
    ];
    match r.kind {
        Kind::Tunnel(protocol) => {
            result.push(format!(
                "Protocol: {} · transport IPv4 {}",
                protocol.as_str(),
                r.input.ip
            ));
            if let Some(controller) = &r.controller {
                result.push(format!(
                    "Configured controller {} · {}",
                    controller.id,
                    controller_description(controller.configuration.as_ref())
                ));
            }
            result.push("The configured controller must supply tunnel peers and keys. Completion verifies the interface configuration; it does not establish traffic reachability. Controller activity is shown separately in coverage.".into());
        }
        Kind::Sriov => {
            if let Some(pci) = &r.hardware {
                result.push(format!(
                    "PCI {} · vendor {} · device {} · driver {}",
                    pci.address.as_deref().unwrap_or("Unknown"),
                    pci.vendor_id.as_deref().unwrap_or("Unknown"),
                    pci.device_id.as_deref().unwrap_or("Unknown"),
                    pci.driver.as_deref().unwrap_or("Unknown")
                ));
            }
            result.push("The disabled, evacuated host must remain live. The NIC driver may be reloaded; every interface using that driver is included below. Host reboot is never submitted by this operation.".into());
            for p in r.driver_interfaces.values() {
                result.push(format!(
                    "Same-driver interface: {} · {} · MAC {}",
                    p.device, p.id, p.mac
                ));
            }
            result.push("If XAPI reports a reboot requirement, review a separate host reboot before expecting VF capacity. Removing manually configured SR-IOV does not necessarily disable hardware VFs.".into());
        }
    }
    match op.change {Change::Create=>result.push("A newly identified network object and interface UUID will be retained in history. Partial creation is not automatically cleaned up or retried.".into()),Change::Delete(object)=>{let p=r.members.values().find(|m|m.object==object).unwrap();result.push(format!("Remove exactly object {} and interface {}. The network record and underlying interface remain.",object.uuid(),p.interface.id));}}
    result.push(format!("Existing target coverage: {} interface(s), {} pool host(s). Other clients can change coverage; the captured identities are rechecked immediately before dispatch.",r.members.len(),r.hosts.len()));
    result
}
fn open(s: &mut State, network: NetworkId, kind: Kind) {
    set_form(
        s,
        Form::Fabric(Box::new(Editor {
            network,
            kind,
            input: None,
        })),
    );
}
pub(super) fn view(i: &Inventory, network: NetworkId, enabled: bool, k: &Kit) -> El {
    let f = &i.details.network_fabric;
    let mut body=col().gap(8.0).child(heading("Tunnel and SR-IOV coverage",k)).child(caption("Coverage is configuration per host. Controller activity and a saved SR-IOV interface are not a traffic or hardware qualification test.",k).wrap());
    for (class, label) in [
        ("tunnel", "Tunnel records"),
        ("sdn_controller", "Controller settings"),
        ("network_sriov", "SR-IOV records"),
        ("pci", "PCI hardware"),
    ] {
        let status = match i.details.classes.get(class).cloned().unwrap_or_default() {
            xenadmin_core::ClassAvailability::Available => "available",
            xenadmin_core::ClassAvailability::Unsupported => "unsupported by this host",
            xenadmin_core::ClassAvailability::PermissionDenied => "not permitted",
            xenadmin_core::ClassAvailability::InvalidData => "invalid response",
            xenadmin_core::ClassAvailability::Unknown => "not yet known",
        };
        body = body.child(caption(format!("{label}: {status}"), k).wrap());
    }
    for h in i.hosts.values() {
        let members = i
            .details
            .pifs
            .values()
            .filter(|p| p.host == Some(h.id) && p.network == Some(network))
            .collect::<Vec<_>>();
        body = body.child(heading(h.name.clone(), k));
        if members.is_empty()
            && i.details.classes.get("pif") != Some(&xenadmin_core::ClassAvailability::Available)
        {
            body = body.child(
                caption(
                    "Coverage unknown: host interface inventory is unavailable",
                    k,
                )
                .wrap(),
            );
        } else if members.is_empty() {
            body = body.child(
                caption(
                    "No interface reported on this network; this host is not covered.",
                    k,
                )
                .wrap(),
            );
        }
        for p in members {
            let mut found = false;
            for t in f.tunnels.values().filter(|t| t.access == Some(p.id)) {
                found = true;
                body=body.child(caption(format!("Tunnel {} · interface {} · protocol {} · controller active {} · controller error {}",t.id,p.id,t.protocol.map(TunnelProtocol::as_str).unwrap_or("unknown"),flag(t.controller_active),flag(t.controller_error)),k).wrap_anywhere());
                if let (Some(input), Some(protocol)) = (t.transport, t.protocol) {
                    body = body.child(remove(
                        i,
                        network,
                        input,
                        Kind::Tunnel(protocol),
                        Object::Tunnel(t.id),
                        enabled,
                        k,
                    ));
                }
            }
            for v in f.sriovs.values().filter(|v| v.logical == Some(p.id)) {
                found = true;
                body = body.child(
                    caption(
                        format!(
                            "SR-IOV {} · interface {} · mode {} · attached {} · requires reboot {}",
                            v.id,
                            p.id,
                            sriov_mode(v.mode),
                            flag(p.currently_attached),
                            flag(v.requires_reboot)
                        ),
                        k,
                    )
                    .wrap_anywhere(),
                );
                if let Some(input) = v.physical {
                    body = body.child(remove(
                        i,
                        network,
                        input,
                        Kind::Sriov,
                        Object::Sriov(v.id),
                        enabled,
                        k,
                    ));
                }
            }
            if !found {
                body = body.child(
                    caption(
                        format!("{} · {} · other or unknown interface kind", p.device, p.id),
                        k,
                    )
                    .wrap_anywhere(),
                );
            }
        }
    }
    for (label, kind) in [
        (
            "Add GRE tunnel interface…",
            Kind::Tunnel(TunnelProtocol::Gre),
        ),
        (
            "Add VXLAN tunnel interface…",
            Kind::Tunnel(TunnelProtocol::Vxlan),
        ),
        ("Enable SR-IOV on one host…", Kind::Sriov),
    ] {
        body = body.child(
            Button::new(label)
                .disabled(!enabled)
                .on_press(move |s: &mut State| open(s, network, kind))
                .build(k),
        );
    }
    body
}
fn controller_description(value: Option<&xenadmin_core::SdnControllerConfiguration>) -> String {
    match value {
        Some(xenadmin_core::SdnControllerConfiguration::Ssl { address, port }) => {
            format!("TLS to {address}:{port}")
        }
        Some(xenadmin_core::SdnControllerConfiguration::PassiveSsl) => {
            "Passive TLS connection".into()
        }
        None => "Unknown configuration".into(),
    }
}
fn sriov_mode(value: Option<xenadmin_core::SriovConfigurationMode>) -> &'static str {
    match value {
        Some(xenadmin_core::SriovConfigurationMode::Sysfs) => "sysfs",
        Some(xenadmin_core::SriovConfigurationMode::Modprobe) => "driver reload",
        Some(xenadmin_core::SriovConfigurationMode::Manual) => "manual",
        _ => "unknown",
    }
}
fn flag(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "yes",
        Some(false) => "no",
        None => "unknown",
    }
}
fn remove(
    i: &Inventory,
    network: NetworkId,
    input: PifId,
    kind: Kind,
    object: Object,
    enabled: bool,
    k: &Kit,
) -> El {
    match Operation::capture(i, network, input, kind, Change::Delete(object)) {
        Ok(op) => review_button(
            "Review removal…",
            ResourceTarget::Network(network),
            NetworkOperation::Fabric(Box::new(op.clone())),
            lines(i, &op),
            enabled,
            k,
        ),
        Err(e) => caption(format!("Removal unavailable: {e}"), k).wrap(),
    }
}
pub(super) fn form(editor: &Editor, i: &Inventory, k: &Kit) -> El {
    let mut body = col().gap(8.0).child(heading(
        match editor.kind {
            Kind::Tunnel(_) => "Choose tunnel transport",
            Kind::Sriov => "Choose physical SR-IOV interface",
        },
        k,
    ));
    for p in i.details.pifs.values().filter(|p| {
        p.network != Some(editor.network)
            && (editor.kind != Kind::Sriov || p.physical == Some(true))
    }) {
        let id = p.id;
        let available = Operation::capture(i, editor.network, id, editor.kind, Change::Create);
        body = body.child(
            Button::new(format!(
                "{}{} · {}",
                if editor.input == Some(id) { "✓ " } else { "" },
                pif_name(i, id),
                p.mac
            ))
            .disabled(available.is_err())
            .on_press(move |s: &mut State| {
                let Form::Fabric(editor) = &mut edit(s).form else {
                    return;
                };
                editor.input = Some(id);
            })
            .build(k),
        );
        if let Err(error) = available {
            body = body.child(caption(error.to_string(), k).wrap());
        }
    }
    body
}
pub(super) fn review(
    editor: &Editor,
    i: &Inventory,
) -> Result<(ResourceTarget, NetworkOperation, Vec<String>), String> {
    let input = editor.input.ok_or("Select an eligible interface")?;
    let op = Operation::capture(i, editor.network, input, editor.kind, Change::Create)
        .map_err(|e| e.to_string())?;
    let lines = lines(i, &op);
    Ok((
        ResourceTarget::Network(editor.network),
        NetworkOperation::Fabric(Box::new(op)),
        lines,
    ))
}
pub(super) fn unavailable(s: &State, w: &Workspace) -> Option<String> {
    let review = w.review.as_ref()?;
    let xenadmin_core::ResourceAction::Network(NetworkOperation::Fabric(op)) = &review.operation
    else {
        return None;
    };
    let Some(i) = s.inventory.as_ref().filter(|i| i.pool_id == w.pool) else {
        return Some("The connection changed; reopen the network review".into());
    };
    op.availability(i).err().map(|e| e.to_string())
}
pub(super) fn scene(s: &mut State) {
    super::scene(s);
    let i = Arc::make_mut(s.inventory.as_mut().unwrap());
    let host = *i.hosts.keys().next().unwrap();
    i.hosts.retain(|id, _| *id == host);
    i.coordinator = Some(host);
    i.vms.clear();
    i.details.vifs.clear();
    i.details.bonds.clear();
    i.details.vlans.clear();
    for c in [
        "network",
        "pif",
        "vif",
        "bond",
        "vlan",
        "tunnel",
        "network_sriov",
        "sdn_controller",
        "pci",
        "host_metrics",
    ] {
        i.details
            .classes
            .insert(c.into(), xenadmin_core::ClassAvailability::Available);
    }
    i.details
        .hosts
        .entry(host)
        .or_default()
        .software_version
        .insert("network_backend".into(), "openvswitch".into());
    let n = NetworkId::from(i.networks[0].uuid);
    let nd = i.details.networks.get_mut(&n).unwrap();
    nd.managed = Some(true);
    nd.purposes = Some(Default::default());
    nd.mtu = Some(1500);
    nd.default_locking_mode = Some("unlocked".into());
    let p = i
        .details
        .pifs
        .values_mut()
        .find(|p| p.host == Some(host) && p.network != Some(n))
        .unwrap();
    p.ip_configuration_mode = "Static".into();
    p.ip = "192.0.2.20".into();
    p.netmask = "255.255.255.0".into();
    let input = p.id;
    let id = xenadmin_core::SdnControllerId::from(uuid::Uuid::from_u128(
        0x55000000000000000000000000000001,
    ));
    i.details.network_fabric.controllers.insert(
        id,
        xenadmin_core::SdnController {
            id,
            configuration: Some(xenadmin_core::SdnControllerConfiguration::Ssl {
                address: "192.0.2.40".parse().unwrap(),
                port: 6632,
            }),
        },
    );
    let op = Operation::capture(
        i,
        n,
        input,
        Kind::Tunnel(TunnelProtocol::Gre),
        Change::Create,
    )
    .unwrap();
    let lines = lines(i, &op);
    prepare(
        s,
        ResourceTarget::Network(n),
        NetworkOperation::Fabric(Box::new(op)),
        lines,
    );
}
pub(super) fn sriov_scene(s: &mut State) {
    scene(s);
    let i = Arc::make_mut(s.inventory.as_mut().unwrap());
    let host = *i.hosts.keys().next().unwrap();
    let network = NetworkId::from(i.networks[0].uuid);
    let input = i
        .details
        .pifs
        .values()
        .find(|p| p.host == Some(host) && p.network != Some(network))
        .unwrap()
        .id;
    i.details.pifs.retain(|id, _| *id == input);
    let p = i.details.pifs.get_mut(&input).unwrap();
    p.ip_configuration_mode = "None".into();
    p.ip.clear();
    p.netmask.clear();
    p.capabilities = Some(vec!["sriov".into()]);
    let underlay = p.network.unwrap();
    if !i.details.networks.contains_key(&underlay) {
        let mut details = i.details.networks[&network].clone();
        details.id = underlay;
        i.details.networks.insert(underlay, details);
    }
    let hd = i.details.hosts.get_mut(&host).unwrap();
    hd.enabled = Some(false);
    hd.current_operations = Some(vec![]);
    let pci = xenadmin_core::PciId::from(uuid::Uuid::from_u128(0x56000000000000000000000000000001));
    i.details.network_fabric.pool_sriov_restricted = Some(false);
    i.details.network_fabric.pif_pci.insert(input, pci);
    i.details.network_fabric.pci_devices.insert(
        pci,
        xenadmin_core::NetworkPciDevice {
            id: pci,
            host: Some(host),
            address: Some("0000:01:00.1".into()),
            vendor_id: Some("8086".into()),
            device_id: Some("100e".into()),
            driver: Some("fixture_nic".into()),
        },
    );
    let op = Operation::capture(i, network, input, Kind::Sriov, Change::Create).unwrap();
    let lines = lines(i, &op);
    prepare(
        s,
        ResourceTarget::Network(network),
        NetworkOperation::Fabric(Box::new(op)),
        lines,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn host_scoped_tunnel_review_is_visible_at_both_sizes_and_demo_cannot_submit() {
        for size in [(513.0, 652.0), (1440.0, 900.0)] {
            let mut s = crate::init();
            scene(&mut s);
            let lines = &s
                .network_workspace
                .as_ref()
                .unwrap()
                .review
                .as_ref()
                .unwrap()
                .lines;
            assert!(lines.iter().any(|v| v.contains("host only")));
            assert!(
                lines
                    .iter()
                    .any(|v| v.contains("does not establish traffic"))
            );
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
            assert!(rt.state.resource_operations.is_empty());
        }
    }
    #[test]
    fn review_fences_changed_controller_transport_and_pool() {
        let mut s = crate::init();
        scene(&mut s);
        assert!(unavailable(&s, s.network_workspace.as_ref().unwrap()).is_none());
        let i = Arc::make_mut(s.inventory.as_mut().unwrap());
        i.details
            .network_fabric
            .controllers
            .values_mut()
            .next()
            .unwrap()
            .configuration = None;
        assert!(unavailable(&s, s.network_workspace.as_ref().unwrap()).is_some());
    }
    #[test]
    fn sriov_review_names_driver_impact_and_reboot_boundary_at_both_sizes() {
        for size in [(513.0, 652.0), (1440.0, 900.0)] {
            let mut s = crate::init();
            sriov_scene(&mut s);
            let w = s.network_workspace.as_ref().unwrap();
            assert!(unavailable(&s, w).is_none());
            let lines = &w.review.as_ref().unwrap().lines;
            assert!(lines.iter().any(|v| v.contains("fixture_nic")));
            assert!(
                lines
                    .iter()
                    .any(|v| v.contains("reboot is never submitted"))
            );
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
            let i = Arc::make_mut(rt.state.inventory.as_mut().unwrap());
            i.details
                .network_fabric
                .pci_devices
                .values_mut()
                .next()
                .unwrap()
                .driver = None;
            assert!(unavailable(&rt.state, rt.state.network_workspace.as_ref().unwrap()).is_some());
        }
    }
}
