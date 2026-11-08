use super::*;
use crate::{
    BondId, ManagementBondCreated, ManagementBondOperation as Op, PifId, management_bond::Baseline,
    xapi::ManagementTopologyRecords as Records,
};
use std::collections::BTreeMap;
fn bond_operation(r: &ResourceOperationRecord) -> Result<(&ManagementInterface, &Op), Error> {
    match operation(r)? {
        ManagementNetworkOperation::Bond { source, operation } => Ok((source, operation)),
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
    networks: &BTreeSet<String>,
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
    for v in network::all(s, "VIF").await?.values() {
        let n = v["network"]
            .as_str()
            .ok_or(Error::Protocol("VM network graph unavailable"))?;
        require(
            !networks.contains(n),
            "Management bond networks have VM consumers; review their migration separately",
        )?;
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
        && network::number(&p["VLAN"]) == Some(-1)
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
            props = properties(t.pif(source.pif)?.1)?;
            let mut refs = vec![];
            for m in members {
                let component = t.topology(m.pif)?;
                require(
                    component.bonds.is_empty() && component.vlans.is_empty(),
                    "Member acquired VLAN or bond dependencies since review",
                )?;
                refs.push(pif(s, m, None, Some(&t)).await?);
                require(
                    properties(t.pif(m.pif)?.1)? == props,
                    "Bond members have differing interface properties",
                )?;
            }
            nets = source_networks(&t, members.iter().map(|m| m.pif))?;
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
                b["master"] == t.pif(source.pif)?.0,
                "Management bond identity changed",
            )?;
            props = properties(t.pif(source.pif)?.1)?;
            for id in captured.pifs.keys() {
                require(
                    properties(t.pif(*id)?.1)? == props,
                    "Bond member properties differ from the master",
                )?;
            }
            nets = source_networks(&t, captured.pifs.keys().copied())?;
            args = vec![json!(bond_ref)];
        }
    }
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
            && b["primary_slave"] == t.pif(source.pif)?.0
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
    let (receiver, members, expected_network) = match op {
        Op::Create {
            members,
            network,
            mode,
            hash,
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
                    && component.vlans.is_empty()
                    && component.pifs.len() == members.len() + 1
                    && bond.mode == mode.as_str()
                    && hash.is_none_or(|h| bond.hashing_algorithm.as_deref() == Some(h.as_str())),
                "Created management bond configuration differs from review",
            )?;
            (made.master, members.clone(), *network)
        }
        Op::Destroy { bond, primary } => {
            require(
                !t.records("bond")
                    .values()
                    .any(|v| network::uuid(v).ok() == Some(bond.uuid()))
                    && !t
                        .pifs()
                        .values()
                        .any(|v| network::uuid(v).ok() == Some(source.pif.uuid())),
                "Removed management bond or master is still present",
            )?;
            let captured = source.topology.as_ref().ok_or(Error::Journal)?;
            let members = captured
                .pifs
                .iter()
                .filter(|(id, _)| **id != source.pif)
                .map(|(_, p)| ManagementInterface::from_inventory(p).ok_or(Error::Journal))
                .collect::<Result<Vec<_>, _>>()?;
            let network = members
                .iter()
                .find(|m| m.pif == *primary)
                .ok_or(Error::Journal)?
                .identity
                .network;
            (*primary, members, network)
        }
    };
    let (receiver_ref, p) = t.pif(receiver)?;
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
            && p["ipv6_gateway"] == ""
            && properties(p)? == b.properties,
        "Management bond receiver does not have the reviewed configuration",
    )?;
    if op.creates() {
        require(
            p["MAC"] == source.identity.mac,
            "Management bond did not retain the source MAC",
        )?;
    }
    for m in &members {
        let (_, p) = t.pif(m.pif)?;
        require(
            basic(p)
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
                component.bonds.is_empty() && component.vlans.is_empty(),
                "Removed bond member acquired another topology",
            )?;
        }
    }
    let mut nets = source_networks(&t, members.iter().map(|m| m.pif))?;
    nets.insert(net_ref.into());
    environment(s, r, &t, &nets, receiver_ref).await
}
fn receiver_host(t: &Records, id: PifId) -> Result<&str, Error> {
    t.pif(id)?.1["host"].as_str().ok_or(Error::ResourceConflict)
}
