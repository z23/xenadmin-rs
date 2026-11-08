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
    let Some(parent) = Bond::management_parent(&source).and_then(|id| {
        s.inventory
            .as_ref()
            .and_then(|i| ManagementInterface::capture(i, id))
    }) else {
        edit(s).error = Some("Management VLAN parent is unavailable".into());
        return;
    };
    set_form(
        s,
        Form::ManagementBond(Box::new(Editor {
            members: vec![parent],
            source,
            network: None,
            mode: BondMode::ActiveBackup,
            hash: BondHash::SourceMac,
        })),
    );
}
fn operation(v: &Editor) -> Result<Operation, String> {
    Ok(Operation::Bond {
        consumers: None,
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
                lines.push("The host creates a bond using the reviewed primary physical member’s MAC. It transfers management and the listed VLANs, retaining their network and IPv4 settings. Recreated VLANs receive new interface and VLAN UUIDs.".into());
            }
            Bond::Destroy { bond, primary } => {
                lines.push(format!("Remove bond {bond}; return management or its VLAN to primary physical member {} ({primary})",pif_name(i,*primary)));
                lines.push("The host removes the bond master and returns its members to their original networks. The network objects are retained.".into());
            }
        }
    }
    if let Operation::Bond {
        source, operation, ..
    } = op
    {
        let components = match operation {
            Bond::Create { members, .. } => members
                .iter()
                .filter_map(|m| m.topology.as_deref())
                .collect::<Vec<_>>(),
            Bond::Destroy { .. } => source.topology.as_deref().into_iter().collect(),
        };
        for t in components {
            for v in t.vlans.values() {
                let Some(p) = v.untagged_pif.and_then(|id| t.pifs.get(&id)) else {
                    continue;
                };
                lines.push(format!("Recreate VLAN {} · tag {} · interface {} · network {} ({}) · {} {} / {} · gateway {} · DNS {} · attached {}",v.id,v.tag.unwrap_or_default(),p.id,p.network.map(|id|name(i,id)).unwrap_or_default(),p.network.map(|v|v.to_string()).unwrap_or_default(),p.ip_configuration_mode,p.ip,p.netmask,p.gateway,p.dns,p.currently_attached.unwrap_or(false)));
            }
        }
    }
    if let Operation::Bond {
        consumers: Some(consumers),
        ..
    } = op
    {
        lines.push(format!("Guest interface impact: {} reviewed interfaces. VM power state and NIC identity, MAC, attachment and settings are preserved.",consumers.interfaces.len()));
        lines.push("NICs using network_default inherit their destination network’s default locking policy.".into());
        for (id, n) in &consumers.networks {
            lines.push(format!(
                "Network {} ({id}) · MTU {} · default locking policy {}.",
                n.name,
                n.mtu,
                n.default_locking_mode
                    .as_deref()
                    .unwrap_or("unavailable (no guest consumers)")
            ));
        }
        for v in consumers.interfaces.values() {
            let locality = match &v.locality {
                xenadmin_core::ManagementVmLocality::Resident(host) => {
                    format!("resident on {host}")
                }
                xenadmin_core::ManagementVmLocality::PossibleHosts(hosts) if hosts.is_empty() => {
                    "no eligible host (authoritative server response; treated as local)".into()
                }
                xenadmin_core::ManagementVmLocality::PossibleHosts(hosts) => format!(
                    "eligible hosts: {}",
                    hosts
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            };
            lines.push(format!("{}: {} ({}) · {}{}{} · NIC {} ({}) · MAC {} · attached {} · {} ({}) → {} ({}) · {locality}",v.change.label(),v.vm_name,v.vm,v.power_state,if v.template {" template"}else{""},if v.snapshot {" snapshot"}else{""},v.device,v.interface,v.mac,v.attached,name(i,v.network),v.network,name(i,v.destination),v.destination));
        }
        lines.push("The host moves the reviewed local NICs inside the bond task. Guest traffic may be interrupted. Remote and otherwise eligible guests retain their network. Changes to the consumer set, guest placement or NIC settings require a new review.".into());
    }
    lines.extend(["Traffic and the management connection can be interrupted. Configure the connected switch for the reviewed bond mode first and keep an independent recovery path available.".into(),format!("Recovery endpoint: {} using HTTPS on the current connection port. Success requires the same host certificate, exact resulting topology and address; existing trust settings still apply.",op.recovery_address().map(|v|v.to_string()).unwrap_or_default()),"Lost replies remain unresolved. Recheck reads existing task and topology evidence; no write, rollback or duplicate creation is sent. New bond/master UUIDs are retained when positively identified.".into(),"This workflow supports static IPv4 management and reviewed VLANs with unique tags. VLAN metadata/properties must be empty because the host recreates those objects. Guest interfaces require the captured impact review. Clustering, storage-purpose, tunnel and SR-IOV dependencies require separate workflows.".into()]);
    lines
}
fn finish_review(s: &mut State, op: Operation) {
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
pub(super) struct Pending {
    query: xenadmin_core::ManagementBondReviewQuery,
    observed: ObservedResource,
    operation: Operation,
}
fn review(s: &mut State, op: Operation) {
    if s.demo {
        finish_review(s, op);
        return;
    }
    let result = (|| {
        let observed = s
            .snapshot
            .as_ref()
            .ok_or("Connect before reviewing guest interfaces")?
            .observe_resource(ResourceTarget::Host(op.source().identity.host))
            .map_err(|e| e.to_string())?;
        let query = s
            .connection
            .as_ref()
            .ok_or("Connect before reviewing guest interfaces")?
            .review_management_bond(observed, op.clone())
            .map_err(|e| e.to_string())?;
        if let Some(waker) = &s.waker {
            query.notify_on_change(waker.clone());
        }
        Ok::<_, String>(Pending {
            query,
            observed,
            operation: op,
        })
    })();
    match result {
        Ok(pending) => {
            let w = edit(s);
            w.bond_review = Some(pending);
            w.review = None;
            w.error = None;
        }
        Err(e) => edit(s).error = Some(e),
    }
}
pub(super) fn poll(s: &mut State) -> bool {
    let Some(result) = s
        .network_workspace
        .as_ref()
        .and_then(|w| w.bond_review.as_ref())
        .and_then(|p| p.query.latest())
    else {
        return false;
    };
    let pending = edit(s).bond_review.take().unwrap();
    let current = inputs_current(s, &pending.operation)
        && s.snapshot
            .as_ref()
            .and_then(|v| v.observe_resource(pending.observed.target()).ok())
            == Some(pending.observed);
    if !current {
        edit(s).error =
            Some("The bond inputs or connection changed. Review guest interfaces again.".into());
        return true;
    }
    match result {
        Ok(op) => finish_review(s, op),
        Err(e) => edit(s).error = Some(e),
    }
    true
}
fn inputs_current(s: &State, requested: &Operation) -> bool {
    let Some(w) = s.network_workspace.as_ref() else {
        return false;
    };
    s.inventory.as_ref().is_some_and(|i| i.pool_id == w.pool)
        && match (requested, &w.form) {
            (
                Operation::Bond {
                    operation: Bond::Create { .. },
                    ..
                },
                Form::ManagementBond(v),
            ) => operation(v).is_ok_and(|v| &v == requested),
            (
                Operation::Bond {
                    operation: Bond::Destroy { .. },
                    ..
                },
                Form::Browse,
            ) => true,
            _ => false,
        }
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
    let Some(b) = source.topology.as_ref().and_then(|t| {
        t.bonds
            .values()
            .find(|b| b.master == Bond::management_parent(&source))
    }) else {
        return;
    };
    let Some(primary) = b.primary_slave else {
        return;
    };
    let bond = b.id;
    review(
        s,
        Operation::Bond {
            consumers: None,
            source,
            operation: Bond::Destroy { bond, primary },
        },
    );
}
pub(super) fn buttons(
    id: PifId,
    _p: &xenadmin_core::PhysicalInterface,
    i: &Inventory,
    enabled: bool,
    k: &Kit,
) -> El {
    if _p.primary_address_type.as_deref() == Some("IPv6") {
        return Button::new("Staged IPv6 management bond…")
            .disabled(!enabled)
            .on_press(move |s: &mut State| ipv6_bond::open(s, id))
            .build(k)
            .key("ipv6-management-bond");
    }
    let physical_parent = ManagementInterface::capture(i, id)
        .and_then(|v| Bond::management_parent(&v))
        .and_then(|id| i.details.pifs.get(&id))
        .is_some_and(|p| p.physical == Some(true));
    row().gap(8.0).flex_wrap().child(if physical_parent {
        Button::new("Create management bond…")
            .disabled(!enabled)
            .on_press(move |s: &mut State| open(s, id))
            .build(k)
            .key("management-bond-create")
    } else {
        Button::new("Remove management bond…")
            .disabled(!enabled)
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
                    "Management endpoint: {} · {}",
                    pif_name(i, v.source.pif),
                    v.source.ipv4.address
                ),
                k,
            )
            .wrap(),
        );
    body=body.child(caption("Choose 2–4 physical members on this host. The management VLAN parent stays first. Reviewed VLANs retain their networks and static IPv4 settings.",k).wrap());
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
            .disabled(Some(id) == Bond::management_parent(&v.source))
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
pub(super) fn vlan_scene(s: &mut State) {
    scene(s);
    let original = editor(s).source.pif;
    let target = editor(s).network.unwrap();
    let second = editor(s).members[1].pif;
    let i = Arc::make_mut(s.inventory.as_mut().unwrap());
    let base = i.details.pifs[&original].clone();
    let source = uuid::Uuid::from_u128(0xb010).into();
    for (n, tag) in [(0xb010, 17), (0xb011, 18)] {
        let id = uuid::Uuid::from_u128(n).into();
        let vlan = uuid::Uuid::from_u128(n + 100).into();
        let network = NetworkId::from(uuid::Uuid::from_u128(n + 200));
        let mut p = base.clone();
        p.id = id;
        p.network = Some(network);
        p.physical = Some(false);
        p.vlan = Some(tag);
        p.mac = "fe:ff:ff:ff:ff:ff".into();
        if id != source {
            p.management = Some(false);
            p.currently_attached = Some(false);
            p.ip_configuration_mode = "None".into();
            p.ip.clear();
            p.netmask.clear();
            p.gateway.clear();
            p.dns.clear();
        }
        i.details.pifs.insert(id, p);
        i.details.vlans.insert(
            vlan,
            xenadmin_core::Vlan {
                id: vlan,
                tagged_pif: Some(original),
                untagged_pif: Some(id),
                tag: Some(tag as u64),
            },
        );
        i.networks.push(xenadmin_core::Network {
            uuid: network.uuid(),
            name: format!("Reviewed VLAN {tag}"),
            bridge: format!("xapi-{tag}"),
        });
    }
    let p = i.details.pifs.get_mut(&original).unwrap();
    p.management = Some(false);
    p.ip_configuration_mode = "None".into();
    p.ip.clear();
    p.netmask.clear();
    p.gateway.clear();
    p.dns.clear();
    let member = ManagementInterface::capture(i, second).unwrap();
    open(s, source);
    editor(s).members.push(member);
    editor(s).network = Some(target);
    prepare(s);
}
pub(super) fn consumer_scene(s: &mut State) {
    scene(s);
    let mut op = operation(editor(s)).unwrap();
    let host = op.source().identity.host;
    let source = op.source().identity.network;
    let target = editor(s).network.unwrap();
    let i = Arc::make_mut(s.inventory.as_mut().unwrap());
    let guests = i.vms.keys().copied().take(3).collect::<Vec<_>>();
    let other = *i.hosts.keys().find(|id| **id != host).unwrap();
    let mut consumers = xenadmin_core::ManagementBondConsumers::default();
    for (index, vm) in guests.into_iter().enumerate() {
        let id = xenadmin_core::VifId::from(uuid::Uuid::from_u128(0xc000 + index as u128));
        let guest = i.vms.get_mut(&vm).unwrap();
        guest.power_state = if index == 2 {
            PowerState::Halted
        } else {
            PowerState::Running
        };
        guest.host = if index == 2 {
            None
        } else if index == 1 {
            Some(other)
        } else {
            Some(host)
        };
        let locality = guest
            .host
            .map(xenadmin_core::ManagementVmLocality::Resident)
            .unwrap_or(xenadmin_core::ManagementVmLocality::PossibleHosts(vec![]));
        let destination = if index == 1 { source } else { target };
        i.details.vifs.insert(
            id,
            xenadmin_core::VirtualInterface {
                qos_known: true,
                qos_supported_algorithms: Some(vec!["ratelimit".into()]),
                id,
                vm: Some(vm),
                network: Some(source),
                device: "0".into(),
                mac: "02:00:00:00:00:01".into(),
                mtu: Some(1500),
                currently_attached: Some(index != 2),
                qos_algorithm: "ratelimit".into(),
                qos_parameters: std::collections::BTreeMap::from([("kbps".into(), "4096".into())]),
                allowed_operations: Some(vec![]),
            },
        );
        consumers.interfaces.insert(
            id,
            xenadmin_core::ManagementBondConsumer {
                interface: id,
                vm,
                vm_name: guest.name.clone(),
                power_state: guest.power_state.to_string(),
                template: false,
                snapshot: false,
                locality,
                network: source,
                destination,
                change: if index == 1 {
                    xenadmin_core::ManagementVifChange::Retain
                } else {
                    xenadmin_core::ManagementVifChange::Move
                },
                device: "0".into(),
                mac: "02:00:00:00:00:01".into(),
                mtu: 1500,
                attached: index != 2,
                settings_fingerprint: [1; 32],
            },
        );
    }
    if let Operation::Bond {
        operation: Bond::Create {
            members, network, ..
        },
        ..
    } = &op
    {
        for id in members
            .iter()
            .map(|m| m.identity.network)
            .chain(std::iter::once(*network))
        {
            consumers.networks.insert(
                id,
                xenadmin_core::ManagementBondNetwork {
                    name: name(i, id),
                    mtu: 1500,
                    default_locking_mode: Some("unlocked".into()),
                    settings_fingerprint: [2; 32],
                },
            );
        }
    }
    if let Operation::Bond {
        consumers: impact, ..
    } = &mut op
    {
        *impact = Some(Box::new(consumers));
    }
    finish_review(s, op);
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn management_guest_query_result_is_fenced_to_exact_form_and_pool() {
        let mut s = crate::init();
        scene(&mut s);
        let requested = operation(editor(&mut s)).unwrap();
        assert!(inputs_current(&s, &requested));
        editor(&mut s).mode = BondMode::Lacp;
        assert!(!inputs_current(&s, &requested));
        editor(&mut s).mode = BondMode::ActiveBackup;
        assert!(inputs_current(&s, &requested));
        Arc::make_mut(s.inventory.as_mut().unwrap()).pool_id = uuid::Uuid::new_v4().into();
        assert!(!inputs_current(&s, &requested));
        s.network_workspace = None;
        assert!(!inputs_current(&s, &requested));
    }
    #[test]
    fn management_guest_impact_review_lists_moved_retained_and_no_eligible_guests() {
        for size in [(513.0, 652.0), (1440.0, 900.0)] {
            let mut s = crate::init();
            consumer_scene(&mut s);
            let w = s.network_workspace.as_ref().unwrap();
            let review = w.review.as_ref().unwrap();
            assert!(review.lines.iter().any(|v| v.contains("Move network:")));
            assert!(review.lines.iter().any(|v| v.contains("Retain network:")));
            assert!(
                review
                    .lines
                    .iter()
                    .any(|v| v.contains("no eligible host (authoritative server response"))
            );
            assert!(
                review
                    .lines
                    .iter()
                    .any(|v| v.contains("3 reviewed interfaces"))
            );
            let first_guest = review
                .lines
                .iter()
                .position(|v| v.starts_with("Move network:"))
                .unwrap();
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
            let key = format!("network-review-line-{first_guest}");
            let mut visible = false;
            for _ in 0..40 {
                if rt
                    .find(&key)
                    .is_some_and(|(_, y)| y > 100.0 && y < size.1 - 150.0)
                {
                    visible = true;
                    break;
                }
                rt.wheel(size.0 / 2.0, size.1 / 2.0, -100.0);
                for _ in 0..10 {
                    rt.frame(1.0 / 60.0);
                }
            }
            assert!(
                visible,
                "guest interface review must be reachable by scrolling"
            );
            let (x, y) = rt.find("network-confirm").unwrap();
            assert!(x > 0.0 && x < size.0 && y > 0.0 && y < size.1);
            rt.pointer_down(x, y);
            rt.pointer_up(x, y);
            rt.frame(0.0);
            assert!(rt.state.resource_operations.is_empty());
        }
    }
    #[test]
    fn management_guest_impact_becomes_stale_when_consumers_or_guest_placement_change() {
        for change in 0..3 {
            let mut s = crate::init();
            consumer_scene(&mut s);
            let i = Arc::make_mut(s.inventory.as_mut().unwrap());
            let (id, v) = i
                .details
                .vifs
                .first_key_value()
                .map(|(id, v)| (*id, v.clone()))
                .unwrap();
            match change {
                0 => {
                    i.details.vifs.remove(&id);
                }
                1 => i.details.vifs.get_mut(&id).unwrap().mac = "02:00:00:00:00:ff".into(),
                _ => i.vms.get_mut(&v.vm.unwrap()).unwrap().power_state = PowerState::Paused,
            }
            assert!(
                management::review_unavailable(&s, s.network_workspace.as_ref().unwrap()).is_some()
            );
        }
    }
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
    fn management_vlan_bond_review_lists_recreated_ids_and_captures_parent_at_both_widths() {
        for remove_bond in [false, true] {
            for size in [(513.0, 652.0), (1440.0, 900.0)] {
                let mut s = crate::init();
                vlan_scene(&mut s);
                let source = editor(&mut s).source.clone();
                let primary = editor(&mut s).members[0].pif;
                if remove_bond {
                    let members = editor(&mut s)
                        .members
                        .iter()
                        .map(|m| m.pif)
                        .collect::<Vec<_>>();
                    let network = editor(&mut s).network.unwrap();
                    let master = uuid::Uuid::from_u128(0xb050).into();
                    let bond = uuid::Uuid::from_u128(0xb051).into();
                    let i = Arc::make_mut(s.inventory.as_mut().unwrap());
                    let mut p = i.details.pifs[&primary].clone();
                    p.id = master;
                    p.network = Some(network);
                    p.physical = Some(false);
                    p.device = "bond0".into();
                    i.details.pifs.insert(master, p);
                    i.details.bonds.insert(
                        bond,
                        xenadmin_core::NetworkBond {
                            id: bond,
                            master: Some(master),
                            slaves: members,
                            primary_slave: Some(primary),
                            mode: "active-backup".into(),
                            hashing_algorithm: None,
                            hashing_algorithm_known: true,
                            links_up: Some(2),
                        },
                    );
                    for v in i.details.vlans.values_mut() {
                        v.tagged_pif = Some(master);
                    }
                    remove(&mut s, source.pif);
                }
                let r = s
                    .network_workspace
                    .as_ref()
                    .unwrap()
                    .review
                    .as_ref()
                    .unwrap();
                assert_eq!(
                    r.lines
                        .iter()
                        .filter(|v| v.starts_with("Recreate VLAN"))
                        .count(),
                    2
                );
                assert!(r.lines.iter().any(|v| v.contains("tag 17")));
                assert!(r.lines.iter().any(|v| v.contains("attached false")));
                let xenadmin_core::ResourceAction::ManagementNetwork(op) = &r.operation else {
                    panic!()
                };
                assert_eq!(op.source().pif, source.pif);
                if !remove_bond {
                    assert!(
                        matches!(op.as_ref(),Operation::Bond{operation:Bond::Create{members,..},..} if members[0].pif==primary)
                    );
                }
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
