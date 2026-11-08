#[path = "management_bond_consumer_backend.rs"]
mod consumers;
#[path = "management_bond_vlans.rs"]
mod vlans;
use super::*;
use crate::{
    BondId, ManagementBondCreated, ManagementBondOperation as Op, PifId, management_bond::Baseline,
    xapi::ManagementTopologyRecords as Records,
};
use std::collections::BTreeMap;
pub(in crate::resources) use vlans::replacements;
fn bond_operation(r: &ResourceOperationRecord) -> Result<(&ManagementInterface, &Op), Error> {
    match operation(r)? {
        ManagementNetworkOperation::Bond {
            source, operation, ..
        } => Ok((source, operation)),
        _ => Err(Error::InvalidResourceCommand),
    }
}
fn properties(v: &Value) -> Result<BTreeMap<String, String>, Error> {
    let p = v["properties"]
        .as_object()
        .filter(|p| p.len() <= 64)
        .ok_or(Error::Protocol("Bond interface properties unavailable"))?;
    p.iter()
        .map(|(k, v)| {
            Ok((
                k.clone(),
                v.as_str()
                    .filter(|v| v.len() <= 128 && k.len() <= 128)
                    .ok_or(Error::Protocol("Bond properties exceed supported bounds"))?
                    .to_owned(),
            ))
        })
        .collect()
}
fn lookup(records: &BTreeMap<String, Value>, id: uuid::Uuid) -> Result<(&str, &Value), Error> {
    records
        .iter()
        .find(|(_, v)| network::uuid(v).ok() == Some(id))
        .map(|(r, v)| (r.as_str(), v))
        .ok_or(Error::ResourceConflict)
}
fn baseline(t: &Records, properties: BTreeMap<String, String>) -> Result<Baseline, Error> {
    Ok(Baseline {
        vlans: Some(
            t.records("vlan")
                .values()
                .map(|v| network::uuid(v).map(crate::VlanId::from))
                .collect::<Result<_, _>>()?,
        ),
        replacements: BTreeMap::new(),
        bonds: t
            .records("bond")
            .values()
            .map(|v| network::uuid(v).map(BondId::from))
            .collect::<Result<_, _>>()?,
        pifs: t
            .pifs()
            .values()
            .map(|v| network::uuid(v).map(PifId::from))
            .collect::<Result<_, _>>()?,
        created: None,
        properties,
    })
}
async fn environment(
    s: &Session,
    r: &ResourceOperationRecord,
    t: &Records,
    _networks: &BTreeSet<String>,
    management: &str,
) -> Result<(), Error> {
    let (_, pool) = xapi::read(s, ResourceTarget::Pool(r.pool)).await?;
    require(
        pool["ha_enabled"] == false,
        "Disable HA before changing management bonds",
    )?;
    let (host_ref, host) = xapi::read(s, r.target).await?;
    require(
        host["address"].as_str().and_then(|v| v.parse().ok()) == operation(r)?.source().address(),
        "Host management address changed since review",
    )?;
    require(
        host["current_operations"]
            .as_object()
            .is_some_and(|v| v.is_empty()),
        "Host is busy or operation availability is unknown",
    )?;
    let managements = t
        .pifs()
        .iter()
        .filter(|(_, v)| v["host"] == host_ref && v["management"] == true)
        .map(|(r, _)| r.as_str())
        .collect::<Vec<_>>();
    require(
        managements == vec![management],
        "Management interface differs from the reviewed bond transition",
    )?;
    let ip = operation(r)?.recovery_address().unwrap();
    for (reference, p) in t.pifs() {
        if reference != management {
            require(
                p["IP"]
                    .as_str()
                    .and_then(|v| v.parse::<std::net::IpAddr>().ok())
                    != Some(ip),
                "Management address is duplicated on another pool interface",
            )?;
        }
    }
    // Clustering can move its protected interface during Bond.create/destroy. Never infer absence.
    let clusters = match network::all(s, "Cluster_host").await {
        Ok(v) => v,
        Err(Error::Api(code)) if code == "MESSAGE_METHOD_UNKNOWN" || code == "NOT_IMPLEMENTED" => {
            BTreeMap::new().into_iter().collect()
        }
        Err(e) => return Err(e),
    };
    for c in clusters.values() {
        require(
            c["host"] != host_ref,
            "Management bond changes with host clustering require a separate reviewed workflow",
        )?;
    }
    Ok(())
}
fn source_networks(
    t: &Records,
    ids: impl IntoIterator<Item = PifId>,
) -> Result<BTreeSet<String>, Error> {
    ids.into_iter()
        .map(|id| {
            let (_, p) = t.pif(id)?;
            Ok(p["network"]
                .as_str()
                .ok_or(Error::ResourceConflict)?
                .to_owned())
        })
        .collect()
}
fn no_ip(p: &Value) -> bool {
    p["ip_configuration_mode"] == "None"
        && p["IP"] == ""
        && p["netmask"] == ""
        && p["gateway"] == ""
        && p["DNS"] == ""
        && p["ipv6_configuration_mode"] == "None"
        && addresses(&p["IPv6"]).is_ok_and(|v| v.is_empty())
        && p["ipv6_gateway"] == ""
}
fn basic(p: &Value) -> bool {
    p["managed"] == true
        && network::number(&p["MTU"]) == Some(1500)
        && p["disallow_unplug"] == false
        && p["other_config"]
            .as_object()
            .is_some_and(|v| !v.contains_key("management_purpose"))
}
pub(super) async fn before(
    s: &Session,
    r: &ResourceOperationRecord,
) -> Result<(Baseline, Vec<Value>), Error> {
    let (source, op) = bond_operation(r)?;
    let t = Records::read(s).await?;
    let mut nets;
    let args;
    let props;
    let parent = Op::management_parent(source).ok_or(Error::InvalidResourceCommand)?;
    let expected_vlans = op
        .captured_vlans(source)
        .ok_or(Error::InvalidResourceCommand)?;
    vlans::before(&t, &expected_vlans)?;
    match op {
        Op::Create {
            members,
            network: target,
            mode,
            hash,
        } => {
            let (net_ref, net) = lookup(t.records("network"), target.uuid())?;
            require(
                net["managed"] == true && network::number(&net["MTU"]) == Some(1500),
                "Management bond destination network must be managed at MTU 1500",
            )?;
            require(
                t.pifs().values().all(|p| p["network"] != net_ref),
                "Management bond destination network already has physical interfaces",
            )?;
            props = properties(t.pif(parent)?.1)?;
            let mut refs = vec![];
            for m in members {
                let component = t.topology(m.pif)?;
                require(
                    component.bonds.is_empty()
                        && m.topology
                            .as_deref()
                            .map_or(component.vlans.is_empty(), |expected| {
                                expected == &component
                            }),
                    "Member VLAN or bond dependencies changed since review",
                )?;
                refs.push(pif(s, m, None, Some(&t)).await?);
                require(
                    properties(t.pif(m.pif)?.1)? == props,
                    "Bond members have differing interface properties",
                )?;
            }
            nets = source_networks(
                &t,
                op.all_interfaces(source)
                    .ok_or(Error::InvalidResourceCommand)?,
            )?;
            nets.insert(net_ref.into());
            args = vec![
                json!(net_ref),
                json!(refs),
                json!(""),
                json!(mode.as_str()),
                hash.map(|h| json!({"hashing_algorithm":h.as_str()}))
                    .unwrap_or(json!({})),
            ];
        }
        Op::Destroy { bond, .. } => {
            let captured = source
                .topology
                .as_ref()
                .ok_or(Error::InvalidResourceCommand)?;
            require(
                &t.topology(source.pif)? == captured.as_ref(),
                "Management bond component changed since review",
            )?;
            pif(s, source, None, Some(&t)).await?;
            let (bond_ref, b) = lookup(t.records("bond"), bond.uuid())?;
            require(
                b["master"] == t.pif(parent)?.0,
                "Management bond identity changed",
            )?;
            props = properties(t.pif(parent)?.1)?;
            for id in captured
                .pifs
                .keys()
                .filter(|id| !expected_vlans.values().any(|v| v.interface.id == **id))
            {
                require(
                    properties(t.pif(*id)?.1)? == props,
                    "Bond member properties differ from the master",
                )?;
            }
            nets = source_networks(&t, captured.pifs.keys().copied())?;
            args = vec![json!(bond_ref)];
        }
    }
    consumers::before(s, operation(r)?).await?;
    environment(s, r, &t, &nets, t.pif(source.pif)?.0).await?;
    Ok((baseline(&t, props)?, args))
}
pub(in crate::resources) async fn created(
    s: &Session,
    r: &ResourceOperationRecord,
) -> Result<ManagementBondCreated, Error> {
    let t = Records::read(s).await?;
    created_from(&t, r)
}
fn created_from(t: &Records, r: &ResourceOperationRecord) -> Result<ManagementBondCreated, Error> {
    let (source, op) = bond_operation(r)?;
    let Op::Create {
        members,
        network: target,
        ..
    } = op
    else {
        return Err(Error::InvalidResourceCommand);
    };
    let plan = r
        .management_network_plan
        .as_ref()
        .and_then(|p| p.bond.as_ref())
        .ok_or(Error::Journal)?;
    let reference = r.result_reference.as_ref().ok_or(Error::Protocol(
        "Management bond result receipt unavailable",
    ))?;
    let b = t
        .records("bond")
        .get(reference)
        .ok_or(Error::ResourceConflict)?;
    let bond = BondId::from(network::uuid(b)?);
    let master_ref = b["master"].as_str().ok_or(Error::ResourceConflict)?;
    let p = t.pifs().get(master_ref).ok_or(Error::ResourceConflict)?;
    let master = PifId::from(network::uuid(p)?);
    let pair = ManagementBondCreated { bond, master };
    let host = lookup(t.records("host"), source.identity.host.uuid())?.0;
    let target = lookup(t.records("network"), target.uuid())?.0;
    let refs = members
        .iter()
        .map(|m| t.pif(m.pif).map(|(r, _)| r.to_owned()))
        .collect::<Result<BTreeSet<_>, _>>()?;
    require(
        !plan.bonds.contains(&bond)
            && !plan.pifs.contains(&master)
            && plan.created.is_none_or(|id| id == pair)
            && p["host"] == host
            && p["network"] == target
            && b["primary_slave"]
                == t.pif(Op::management_parent(source).ok_or(Error::InvalidResourceCommand)?)?
                    .0
            && network::refs(&b["slaves"])?
                .into_iter()
                .collect::<BTreeSet<_>>()
                == refs,
        "Created management bond identity or provenance differs from the reviewed operation",
    )?;
    Ok(pair)
}
pub(super) async fn verify(s: &Session, r: &ResourceOperationRecord) -> Result<(), Error> {
    let (source, op) = bond_operation(r)?;
    let b = r
        .management_network_plan
        .as_ref()
        .and_then(|p| p.bond.as_ref())
        .ok_or(Error::Journal)?;
    let t = Records::read(s).await?;
    let old_parent = Op::management_parent(source).ok_or(Error::InvalidResourceCommand)?;
    let expected_vlans = op
        .captured_vlans(source)
        .ok_or(Error::InvalidResourceCommand)?;
    let (parent, members) = match op {
        Op::Create {
            members,
            mode,
            hash,
            ..
        } => {
            let made = b.created.ok_or(Error::Protocol(
                "Created management bond identity is unavailable",
            ))?;
            require(
                created_from(&t, r)? == made,
                "Created management bond identity changed",
            )?;
            let component = t.topology(made.master)?;
            let bond = component
                .bonds
                .get(&made.bond)
                .ok_or(Error::ResourceConflict)?;
            require(
                component.bonds.len() == 1
                    && component.vlans.len() == expected_vlans.len()
                    && component.pifs.len() == members.len() + 1 + expected_vlans.len()
                    && bond.mode == mode.as_str()
                    && hash.is_none_or(|h| bond.hashing_algorithm.as_deref() == Some(h.as_str())),
                "Created management bond configuration differs from review",
            )?;
            (made.master, members.clone())
        }
        Op::Destroy { bond, primary } => {
            require(
                !t.records("bond")
                    .values()
                    .any(|v| network::uuid(v).ok() == Some(bond.uuid()))
                    && !t
                        .pifs()
                        .values()
                        .any(|v| network::uuid(v).ok() == Some(old_parent.uuid())),
                "Removed management bond or master is still present",
            )?;
            let captured = source.topology.as_ref().ok_or(Error::Journal)?;
            let bond = captured.bonds.get(bond).ok_or(Error::Journal)?;
            let members = bond
                .slaves
                .iter()
                .map(|id| {
                    ManagementInterface::from_inventory(&captured.pifs[id]).ok_or(Error::Journal)
                })
                .collect::<Result<Vec<_>, _>>()?;
            (*primary, members)
        }
    };
    vlans::verify(&t, r, parent, &expected_vlans)?;
    let receiver = if old_parent == source.pif {
        parent
    } else {
        b.replacements
            .values()
            .find(|v| v.old_interface == source.pif)
            .ok_or(Error::Protocol("Management VLAN replacement is unverified"))?
            .new_interface
    };
    let (receiver_ref, p) = t.pif(receiver)?;
    let expected_network = if old_parent != source.pif {
        source.identity.network
    } else {
        match op {
            Op::Create { network, .. } => *network,
            Op::Destroy { primary, .. } => {
                members
                    .iter()
                    .find(|m| m.pif == *primary)
                    .ok_or(Error::Journal)?
                    .identity
                    .network
            }
        }
    };
    let net_ref = lookup(t.records("network"), expected_network.uuid())?.0;
    require(
        basic(p)
            && p["currently_attached"] == true
            && p["management"] == true
            && p["network"] == net_ref
            && p["primary_address_type"] == "IPv4"
            && p["ip_configuration_mode"] == source.ipv4.mode.as_str()
            && p["IP"] == source.ipv4.address
            && p["netmask"] == source.ipv4.netmask
            && p["gateway"] == source.ipv4.gateway
            && dns(p["DNS"].as_str().ok_or(Error::ResourceConflict)?)? == dns(&source.ipv4.dns)?
            && p["ipv6_configuration_mode"] == "None"
            && addresses(&p["IPv6"])?.is_empty()
            && p["ipv6_gateway"] == "",
        "Management bond receiver does not have the reviewed configuration",
    )?;
    let (_, parent_record) = t.pif(parent)?;
    require(
        properties(parent_record)? == b.properties
            && parent_record["currently_attached"] == true
            && network::number(&parent_record["VLAN"]) == Some(-1)
            && (receiver == parent
                || (parent_record["management"] == false && no_ip(parent_record))),
        "Bond parent properties or IP settings changed unexpectedly",
    )?;
    if op.creates() {
        let physical = members
            .iter()
            .find(|m| m.pif == old_parent)
            .ok_or(Error::Journal)?;
        require(
            parent_record["MAC"] == physical.identity.mac,
            "Management bond did not retain the primary physical MAC",
        )?;
    }
    for m in &members {
        let (_, p) = t.pif(m.pif)?;
        require(
            basic(p)
                && p["physical"] == true
                && network::number(&p["VLAN"]) == Some(-1)
                && p["MAC"] == m.identity.mac
                && p["device"] == m.identity.device
                && p["host"] == receiver_host(&t, receiver)?
                && p["network"] == lookup(t.records("network"), m.identity.network.uuid())?.0
                && properties(p)? == b.properties,
            "Management bond member identity or configuration changed",
        )?;
        if m.pif != receiver {
            require(
                p["management"] == false && no_ip(p),
                "Bond member retained unexpected IP configuration",
            )?;
        }
        if !op.creates() {
            let component = t.topology(m.pif)?;
            require(
                component.bonds.is_empty()
                    && component.vlans.len()
                        == if m.pif == parent {
                            expected_vlans.len()
                        } else {
                            0
                        },
                "Removed bond member acquired another topology",
            )?;
        }
    }
    let mut nets = source_networks(&t, members.iter().map(|m| m.pif))?;
    nets.insert(
        parent_record["network"]
            .as_str()
            .ok_or(Error::ResourceConflict)?
            .into(),
    );
    for v in expected_vlans.values() {
        nets.insert(
            lookup(
                t.records("network"),
                v.interface.network.ok_or(Error::Journal)?.uuid(),
            )?
            .0
            .into(),
        );
    }
    nets.insert(net_ref.into());
    consumers::verify(s, operation(r)?).await?;
    environment(s, r, &t, &nets, receiver_ref).await
}
fn receiver_host(t: &Records, id: PifId) -> Result<&str, Error> {
    t.pif(id)?.1["host"].as_str().ok_or(Error::ResourceConflict)
}

pub(crate) async fn review(
    s: &Session,
    observed: crate::ObservedResource,
    mut operation: ManagementNetworkOperation,
) -> Result<ManagementNetworkOperation, Error> {
    operation.validate(observed.target())?;
    xapi::read(s, ResourceTarget::Pool(observed.pool)).await?;
    xapi::read(s, observed.target()).await?;
    let ManagementNetworkOperation::Bond {
        source,
        operation: op,
        consumers: impact,
    } = &mut operation
    else {
        return Err(Error::InvalidResourceCommand);
    };
    let t = Records::read(s).await?;
    pif(s, source, None, Some(&t)).await?;
    let component = t.topology(source.pif)?;
    require(
        source.topology.as_deref().map_or(
            component.vlans.is_empty() && component.bonds.is_empty(),
            |v| v == &component,
        ),
        "Management topology changed while preparing guest interface review",
    )?;
    if let Op::Create { members, .. } = op {
        for member in members {
            pif(s, member, None, Some(&t)).await?;
            let component = t.topology(member.pif)?;
            require(
                member.topology.as_deref().map_or(
                    component.vlans.is_empty() && component.bonds.is_empty(),
                    |v| v == &component,
                ),
                "Member topology changed while preparing guest interface review",
            )?;
        }
    }
    *impact = Some(Box::new(consumers::capture(s, source, op).await?));
    Ok(operation)
}
