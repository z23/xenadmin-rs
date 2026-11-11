use super::{
    ResourceAction, ResourceOperationRecord, ResourceTarget,
    management::{self, bond},
    network, xapi,
};
use crate::{
    Error, ManagementBondOperation as Op, ManagementInterface, ManagementNetworkOperation,
    ManagementTopology, PhysicalInterface, PifId,
    management_ipv6_bond::{Plan, Receipt, Stage},
    xapi::{ManagementTopologyRecords as Records, Session},
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use xenadmin_xapi::records::{HostRecord, PifRecord, PoolRecord};
fn require(v: bool, message: &'static str) -> Result<(), Error> {
    if v {
        Ok(())
    } else {
        Err(Error::Protocol(message))
    }
}
fn impact(p: &Plan) -> ManagementNetworkOperation {
    ManagementNetworkOperation::Bond {
        source: p.intent.source.clone(),
        operation: p.intent.operation.clone(),
        consumers: p.intent.consumers.clone(),
    }
}
fn roots(op: &crate::ManagementIpv6Bond) -> Vec<&ManagementInterface> {
    match &op.operation {
        Op::Create { members, .. } => members.iter().collect(),
        Op::Destroy { .. } => vec![&op.source],
    }
}
fn plain_matches(t: &ManagementTopology, p: &ManagementInterface) -> bool {
    t.valid(p.pif)
        && t.matches_root(p)
        && p.topology
            .as_deref()
            .map_or(t.bonds.is_empty() && t.vlans.is_empty(), |v| v == t)
}
pub(super) async fn prepare(s: &Session, r: &ResourceOperationRecord) -> Result<Plan, Error> {
    let ResourceAction::ManagementIpv6Bond(op) = &r.action else {
        return Err(Error::InvalidResourceCommand);
    };
    op.validate(r.target)?;
    let t = Records::read(s).await?;
    let mut originals = vec![];
    for p in roots(op) {
        let component = t.topology(p.pif)?;
        require(
            plain_matches(&component, p),
            "Reviewed IPv6 management topology changed",
        )?;
        originals.push(component);
    }
    require(
        originals.iter().all(|t| {
            t.pifs
                .values()
                .all(|p| p.disallow_unplug == Some(false) && p.management_purpose.is_none())
        }),
        "Affected IPv6 topology has protected or service-purpose interfaces",
    )?;
    let temporary = t.topology(op.temporary.pif)?;
    require(
        temporary
            .pifs
            .values()
            .all(|p| p.disallow_unplug == Some(false) && p.management_purpose.is_none()),
        "Temporary IPv6 topology has protected or service-purpose interfaces",
    )?;
    require(
        plain_matches(&temporary, &op.temporary),
        "Temporary IPv6 interface changed",
    )?;
    let parent = Op::management_parent(&op.source).ok_or(Error::InvalidResourceCommand)?;
    let props = bond::properties(t.pif(parent)?.1)?;
    let baseline = bond::baseline(&t, props.clone())?;
    let mut temp_props = BTreeMap::new();
    for id in temporary.pifs.keys() {
        temp_props.insert(*id, bond::properties(t.pif(*id)?.1)?);
    }
    let original = s.options().endpoint.clone();
    require(
        original.as_str().starts_with("https://"),
        "IPv6 management handover requires HTTPS",
    )?;
    let plan = Plan {
        version: 1,
        pool: r.pool,
        host: op.source.identity.host,
        origin: r.id,
        predecessor: None,
        successor: None,
        depth: 0,
        lineage: 0,
        intent: (**op).clone(),
        original_endpoint: original.clone(),
        temporary_endpoint: crate::management_ipv6_bond::endpoint(
            &original,
            op.temporary
                .address()
                .ok_or(Error::InvalidResourceCommand)?,
        )?,
        final_endpoint: crate::management_ipv6_bond::endpoint(
            &original,
            op.source.address().ok_or(Error::InvalidResourceCommand)?,
        )?,
        certificate: management::unique_certificate(s, op.source.identity.host).await?,
        registered: false,
        temporary_route_verified: false,
        final_route_verified: false,
        expires_at_ms: r.created_at_ms + 10 * 60 * 1000,
        receipts: vec![Receipt::pending(Stage::MoveToTemporary)],
        baseline,
        change_baseline: None,
        original_components: originals,
        temporary_component: temporary,
        temporary_properties: temp_props,
    };
    verify(s, &plan, 0).await?;
    Ok(plan)
}
fn clear_ipv6(p: &mut PhysicalInterface) -> Result<(), Error> {
    p.ipv6_configuration_mode = "None".into();
    p.ipv6.clear();
    p.ipv6_gateway.clear();
    p.dns = management::configured_dns(&p.dns, "", false)?;
    Ok(())
}
fn combine(components: &[ManagementTopology]) -> Result<ManagementTopology, Error> {
    let mut out = ManagementTopology {
        pifs: BTreeMap::new(),
        bonds: BTreeMap::new(),
        vlans: BTreeMap::new(),
    };
    for t in components {
        for (id, p) in &t.pifs {
            require(
                out.pifs.insert(*id, p.clone()).is_none(),
                "Affected management components overlap",
            )?;
        }
        out.bonds.extend(t.bonds.clone());
        out.vlans.extend(t.vlans.clone());
    }
    Ok(out)
}
pub(super) fn receiver(p: &Plan) -> Result<PifId, Error> {
    let old = Op::management_parent(&p.intent.source).ok_or(Error::Journal)?;
    let b = p.change_baseline.as_ref().ok_or(Error::Journal)?;
    let parent = match &p.intent.operation {
        Op::Create { .. } => b.created.ok_or(Error::Journal)?.master,
        Op::Destroy { primary, .. } => *primary,
    };
    if old == p.intent.source.pif {
        Ok(parent)
    } else {
        b.replacements
            .values()
            .find(|v| v.old_interface == p.intent.source.pif)
            .map(|v| v.new_interface)
            .ok_or(Error::Protocol(
                "IPv6 management VLAN replacement is unproven",
            ))
    }
}
fn expected(p: &Plan, t: &Records, prefix: usize) -> Result<ManagementTopology, Error> {
    let op = &p.intent;
    let mut graph = combine(&p.original_components)?;
    if prefix >= 1 {
        graph
            .pifs
            .get_mut(&op.source.pif)
            .ok_or(Error::Journal)?
            .management = Some(false);
    }
    if prefix >= 2 {
        clear_ipv6(graph.pifs.get_mut(&op.source.pif).ok_or(Error::Journal)?)?;
    }
    if prefix >= 3 {
        let b = p.change_baseline.as_ref().ok_or(Error::Journal)?;
        let old_parent = Op::management_parent(&op.source).ok_or(Error::Journal)?;
        let parent = match &op.operation {
            Op::Create {
                members,
                network,
                mode,
                hash,
            } => {
                let made = b.created.ok_or(Error::Journal)?;
                require(
                    bond::identity(
                        t,
                        &op.source,
                        &op.operation,
                        b,
                        p.receipts[2].result.as_deref().ok_or(Error::Journal)?,
                    )? == made,
                    "Created IPv6 bond identity changed",
                )?;
                let mut master = graph.pifs[&old_parent].clone();
                master.id = made.master;
                master.network = Some(*network);
                master.physical = Some(false);
                master.currently_attached = Some(true);
                master.management = Some(false);
                let actual = t.topology(made.master)?;
                let device = &actual.pifs[&made.master].device;
                require(
                    device.strip_prefix("bond").is_some_and(|v| {
                        !v.is_empty() && v.len() <= 6 && v.bytes().all(|b| b.is_ascii_digit())
                    }),
                    "Created bond device name is not an ordinary bond interface",
                )?;
                master.device = device.clone();
                master.ipv6_configuration_mode = "None".into();
                master.ipv6.clear();
                master.ipv6_gateway.clear();
                let primary = graph.pifs.get_mut(&old_parent).unwrap();
                primary.ip_configuration_mode = "None".into();
                primary.ip.clear();
                primary.netmask.clear();
                primary.gateway.clear();
                primary.dns.clear();
                graph.pifs.insert(made.master, master);
                let mut slaves = members.iter().map(|m| m.pif).collect::<Vec<_>>();
                slaves.sort();
                graph.bonds.insert(
                    made.bond,
                    crate::NetworkBond {
                        id: made.bond,
                        master: Some(made.master),
                        primary_slave: Some(old_parent),
                        slaves,
                        mode: mode.as_str().into(),
                        hashing_algorithm: hash.map(|v| v.as_str().into()),
                        hashing_algorithm_known: true,
                        links_up: None,
                    },
                );
                made.master
            }
            Op::Destroy { bond: old, primary } => {
                require(
                    !t.records("bond")
                        .values()
                        .any(|v| network::uuid(v).ok() == Some(old.uuid()))
                        && !t
                            .pifs()
                            .values()
                            .any(|v| network::uuid(v).ok() == Some(old_parent.uuid())),
                    "Removed IPv6 bond or master is still present",
                )?;
                let old = graph.pifs.remove(&old_parent).ok_or(Error::Journal)?;
                let dest = graph.pifs.get_mut(primary).ok_or(Error::Journal)?;
                dest.ip_configuration_mode = old.ip_configuration_mode;
                dest.ip = old.ip;
                dest.netmask = old.netmask;
                dest.gateway = old.gateway;
                dest.dns = old.dns;
                graph.bonds.clear();
                *primary
            }
        };
        let captured = op.operation.raw_vlans(&op.source).ok_or(Error::Journal)?;
        require(
            bond::vlans::identities(t, b, parent, &captured, true)? == b.replacements,
            "IPv6 VLAN replacement identity differs from saved proof",
        )?;
        for (id, v) in captured {
            let r = b.replacements.get(&id).ok_or(Error::Journal)?;
            let mut child = graph.pifs.remove(&v.interface.id).ok_or(Error::Journal)?;
            graph.vlans.remove(&id);
            child.id = r.new_interface;
            child.device = graph.pifs[&parent].device.clone();
            child.ipv6_configuration_mode = "None".into();
            child.ipv6.clear();
            child.ipv6_gateway.clear();
            graph.pifs.insert(child.id, child);
            graph.vlans.insert(
                r.new_vlan,
                crate::Vlan {
                    id: r.new_vlan,
                    tagged_pif: Some(parent),
                    untagged_pif: Some(r.new_interface),
                    tag: v.vlan.tag,
                },
            );
        }
        // Bond bring-up changes physical member attachment state. Compare all stable fields;
        // the actual receiver and VLAN attachment expectations remain exact.
        for (id, v) in &mut graph.pifs {
            if op.operation.creates() && v.physical == Some(true) && *id != receiver(p)? {
                v.currently_attached = t.topology(parent)?.pifs[id].currently_attached;
            }
        }
        if !op.operation.creates() {
            for v in graph.pifs.values_mut().filter(|v| v.physical == Some(true)) {
                v.currently_attached = Some(true);
            }
        }
    }
    if prefix >= 4 {
        let v = graph.pifs.get_mut(&receiver(p)?).ok_or(Error::Journal)?;
        v.ipv6_configuration_mode = "Static".into();
        v.ipv6 = op.source.ipv6.addresses.clone();
        v.ipv6_gateway = op.source.ipv6.gateway.clone();
        v.dns = management::configured_dns(&v.dns, &op.source.ipv6.dns, false)?;
    }
    if prefix >= 5 {
        graph
            .pifs
            .get_mut(&receiver(p)?)
            .ok_or(Error::Journal)?
            .management = Some(true);
    }
    Ok(graph)
}
async fn environment(
    s: &Session,
    p: &Plan,
    t: &Records,
    management: PifId,
    address: std::net::IpAddr,
) -> Result<(), Error> {
    let (_, pool) = xapi::read(s, ResourceTarget::Pool(p.pool)).await?;
    require(
        PoolRecord::new(&pool).ha_enabled() == false,
        "Disable HA before a staged IPv6 management handover",
    )?;
    let (host, record) = xapi::read(s, ResourceTarget::Host(p.host)).await?;
    require(
        HostRecord::new(&record)
            .current_operations()
            .as_object()
            .is_some_and(|v| v.is_empty())
            && HostRecord::new(&record)
                .address()
                .as_str()
                .and_then(|v| v.parse().ok())
                == Some(address),
        "Host management role, address or task state changed",
    )?;
    let active = t
        .pifs()
        .iter()
        .filter(|(_, v)| PifRecord::new(v).host() == host && PifRecord::new(v).management() == true)
        .map(|(r, _)| r.as_str())
        .collect::<Vec<_>>();
    require(
        active == vec![t.pif(management)?.0],
        "Host management interface identity is ambiguous",
    )?;
    let clusters = match network::all(s, "Cluster_host").await {
        Ok(v) => v,
        Err(Error::Api(c)) if c == "MESSAGE_METHOD_UNKNOWN" || c == "NOT_IMPLEMENTED" => {
            Default::default()
        }
        Err(e) => return Err(e),
    };
    require(
        clusters.values().all(|v| PifRecord::new(v).host() != host),
        "Host clustering requires a separate management handover",
    )?;
    let temp_networks = p
        .temporary_component
        .pifs
        .values()
        .filter_map(|v| v.network)
        .collect::<BTreeSet<_>>();
    let mut temp_refs = BTreeSet::new();
    for id in temp_networks {
        temp_refs.insert(bond::lookup(t.records("network"), id.uuid())?.0.to_owned());
    }
    require(
        network::all(s, "VIF").await?.values().all(|v| {
            PifRecord::new(v)
                .network()
                .as_str()
                .is_some_and(|n| !temp_refs.contains(n))
        }),
        "Temporary management component has guest consumers",
    )?;
    for (id, props) in &p.temporary_properties {
        require(
            bond::properties(t.pif(*id)?.1)? == *props,
            "Temporary interface properties changed",
        )?;
    }
    for (address, owner) in [
        (
            p.intent.temporary.address().ok_or(Error::Journal)?,
            p.intent.temporary.pif,
        ),
        (
            p.intent.source.address().ok_or(Error::Journal)?,
            if p.receipts.get(2).is_some_and(|v| v.completed)
                && p.change_baseline.as_ref().is_some_and(|v| {
                    (!p.intent.operation.creates() || v.created.is_some())
                        && v.replacements.len()
                            == p.intent
                                .operation
                                .raw_vlans(&p.intent.source)
                                .map(|v| v.len())
                                .unwrap_or(0)
                })
            {
                receiver(p)?
            } else {
                p.intent.source.pif
            },
        ),
    ] {
        for v in t.pifs().values() {
            if network::uuid(v)? != owner.uuid() {
                let addresses = PifRecord::new(v)
                    .ipv6()
                    .value()
                    .as_array()
                    .ok_or(Error::ResourceConflict)?;
                require(
                    !addresses
                        .iter()
                        .filter_map(Value::as_str)
                        .filter_map(crate::network_types::ipv6_cidr)
                        .any(|(ip, _)| std::net::IpAddr::V6(ip) == address),
                    "Reviewed IPv6 address appears on another pool interface",
                )?;
            }
        }
    }
    Ok(())
}
pub(super) async fn verify(s: &Session, p: &Plan, prefix: usize) -> Result<Records, Error> {
    let t = Records::read(s).await?;
    let mut temporary = p.temporary_component.clone();
    temporary
        .pifs
        .get_mut(&p.intent.temporary.pif)
        .ok_or(Error::Journal)?
        .management = Some((1..5).contains(&prefix));
    require(
        t.topology(p.intent.temporary.pif)? == temporary,
        "Temporary IPv6 management component changed",
    )?;
    let expected = expected(p, &t, prefix)?;
    let mut actual = ManagementTopology {
        pifs: BTreeMap::new(),
        bonds: BTreeMap::new(),
        vlans: BTreeMap::new(),
    };
    for id in expected
        .pifs
        .keys()
        .filter(|id| !expected.bonds.values().any(|b| b.slaves.contains(id)))
    {
        let g = t.topology(*id)?;
        actual.pifs.extend(g.pifs);
        actual.bonds.extend(g.bonds);
        actual.vlans.extend(g.vlans);
    }
    require(
        actual == expected,
        "IPv6 handover topology or interface configuration differs from the verified stage",
    )?;
    for (id, v) in &expected.pifs {
        let props = bond::properties(t.pif(*id)?.1)?;
        require(
            if v.vlan == Some(-1) {
                props == p.baseline.properties
            } else {
                props.is_empty()
            },
            "Affected interface properties changed",
        )?;
    }
    if prefix < 3 {
        bond::vlans::before(
            &t,
            &p.intent
                .operation
                .raw_vlans(&p.intent.source)
                .ok_or(Error::Journal)?,
        )?;
        bond::consumers::before(s, &impact(p)).await?;
    } else {
        for r in p
            .change_baseline
            .as_ref()
            .ok_or(Error::Journal)?
            .replacements
            .values()
        {
            require(
                t.pif(r.new_interface)?.1["other_config"]
                    .as_object()
                    .is_some_and(|v| v.is_empty())
                    && bond::lookup(t.records("vlan"), r.new_vlan.uuid())?.1["other_config"]
                        .as_object()
                        .is_some_and(|v| v.is_empty()),
                "Migrated VLAN metadata changed",
            )?;
        }
        bond::consumers::verify(s, &impact(p)).await?;
    }
    if prefix < 3
        && let Op::Create { network, .. } = &p.intent.operation
    {
        let (n, v) = bond::lookup(t.records("network"), network.uuid())?;
        require(
            v["managed"] == true
                && network::number(&v["MTU"]) == Some(1500)
                && t.pifs().values().all(|p| p["network"] != n),
            "Destination network is no longer empty, managed at MTU 1500",
        )?;
    }
    let (management, address) = if prefix == 0 {
        (p.intent.source.pif, p.intent.source.address())
    } else if prefix < 5 {
        (p.intent.temporary.pif, p.intent.temporary.address())
    } else {
        (receiver(p)?, p.intent.source.address())
    };
    environment(s, p, &t, management, address.ok_or(Error::Journal)?).await?;
    Ok(t)
}
pub(super) async fn arguments(
    s: &Session,
    p: &mut Plan,
) -> Result<(&'static str, Vec<Value>), Error> {
    let t = verify(s, p, p.depth as usize).await?;
    let args = match p.stage() {
        Stage::MoveToTemporary => (
            "Async.host.management_reconfigure",
            vec![json!(t.pif(p.intent.temporary.pif)?.0)],
        ),
        Stage::ClearSourceIpv6 => (
            "Async.PIF.reconfigure_ipv6",
            vec![
                json!(t.pif(p.intent.source.pif)?.0),
                json!("None"),
                json!(""),
                json!(""),
                json!(""),
            ],
        ),
        Stage::ChangeBond => {
            p.change_baseline = Some(bond::baseline(&t, p.baseline.properties.clone())?);
            match &p.intent.operation {
                Op::Create {
                    members,
                    network,
                    mode,
                    hash,
                } => {
                    let refs = members
                        .iter()
                        .map(|p| t.pif(p.pif).map(|v| v.0.to_owned()))
                        .collect::<Result<Vec<_>, _>>()?;
                    (
                        "Async.Bond.create",
                        vec![
                            json!(bond::lookup(t.records("network"), network.uuid())?.0),
                            json!(refs),
                            json!(""),
                            json!(mode.as_str()),
                            hash.map(|h| json!({"hashing_algorithm":h.as_str()}))
                                .unwrap_or(json!({})),
                        ],
                    )
                }
                Op::Destroy { bond, .. } => (
                    "Async.Bond.destroy",
                    vec![json!(bond::lookup(t.records("bond"), bond.uuid())?.0)],
                ),
            }
        }
        Stage::RestoreTargetIpv6 => (
            "Async.PIF.reconfigure_ipv6",
            vec![
                json!(t.pif(receiver(p)?)?.0),
                json!("Static"),
                json!(p.intent.source.ipv6.addresses[0]),
                json!(p.intent.source.ipv6.gateway),
                json!(p.intent.source.ipv6.dns),
            ],
        ),
        Stage::MoveToTarget => (
            "Async.host.management_reconfigure",
            vec![json!(t.pif(receiver(p)?)?.0)],
        ),
    };
    p.current_mut().task_baseline = network::all(s, "task")
        .await?
        .values()
        .map(|v| network::uuid(v).map(Into::into))
        .collect::<Result<_, _>>()?;
    Ok(args)
}
pub(super) async fn identities(s: &Session, p: &mut Plan) -> Result<(), Error> {
    if p.stage() != Stage::ChangeBond {
        return Ok(());
    }
    let t = Records::read(s).await?;
    let b = p.change_baseline.as_mut().ok_or(Error::Journal)?;
    let parent = match &p.intent.operation {
        Op::Create { .. } => {
            let made = bond::identity(
                &t,
                &p.intent.source,
                &p.intent.operation,
                b,
                p.receipts[2].result.as_deref().ok_or(Error::Journal)?,
            )?;
            b.created = Some(made);
            made.master
        }
        Op::Destroy { primary, .. } => *primary,
    };
    b.replacements.extend(bond::vlans::identities(
        &t,
        b,
        parent,
        &p.intent
            .operation
            .raw_vlans(&p.intent.source)
            .ok_or(Error::Journal)?,
        false,
    )?);
    Ok(())
}
pub(crate) async fn review(
    s: &Session,
    observed: crate::ObservedResource,
    mut op: crate::ManagementIpv6Bond,
) -> Result<crate::ManagementIpv6Bond, Error> {
    op.validate(observed.target())?;
    xapi::read(s, ResourceTarget::Pool(observed.pool)).await?;
    xapi::read(s, observed.target()).await?;
    let t = Records::read(s).await?;
    for p in roots(&op).into_iter().chain([&op.temporary]) {
        require(
            plain_matches(&t.topology(p.pif)?, p),
            "IPv6 management topology changed during guest review",
        )?;
    }
    op.consumers = Some(Box::new(
        bond::consumers::capture(s, &op.source, &op.operation).await?,
    ));
    Ok(op)
}
