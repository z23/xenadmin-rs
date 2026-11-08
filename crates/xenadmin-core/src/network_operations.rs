use super::{
    ResourceAction, ResourceOperationRecord, ResourceTarget,
    xapi::{self, Check, Step},
};
use crate::{
    Error, NetworkOperation, PifId, PifIdentity, network_types::NetworkCreationBaseline,
    xapi::Session,
};
use serde_json::{Value, json};
use std::collections::BTreeSet;

pub(super) fn operation(record: &ResourceOperationRecord) -> Result<&NetworkOperation, Error> {
    match &record.action {
        ResourceAction::Network(op) => Ok(op),
        _ => Err(Error::InvalidResourceCommand),
    }
}
fn reference(value: &Value) -> Result<String, Error> {
    xapi::reply_reference(value)
}
fn number(value: &Value) -> Option<i64> {
    value.as_i64().or_else(|| value.as_str()?.parse().ok())
}
fn refs(value: &Value) -> Result<BTreeSet<String>, Error> {
    value
        .as_array()
        .filter(|v| v.len() <= 10_000)
        .ok_or(Error::Protocol("interface membership unavailable"))?
        .iter()
        .map(reference)
        .collect()
}
async fn related(session: &Session, class: &str, value: &Value) -> Result<Value, Error> {
    session
        .call(
            &format!("{class}.get_record"),
            vec![json!(reference(value)?)],
        )
        .await
}
fn uuid(value: &Value) -> Result<uuid::Uuid, Error> {
    value["uuid"]
        .as_str()
        .and_then(|v| uuid::Uuid::parse_str(v).ok())
        .ok_or(Error::Protocol("network object UUID unavailable"))
}
async fn pif_identity(
    session: &Session,
    fresh: &Value,
    expected: &PifIdentity,
) -> Result<(), Error> {
    if uuid(&related(session, "host", &fresh["host"]).await?)? != expected.host.uuid()
        || uuid(&related(session, "network", &fresh["network"]).await?)? != expected.network.uuid()
        || fresh["device"] != expected.device
        || fresh["MAC"] != expected.mac
        || fresh["management"].as_bool() != Some(expected.management)
    {
        return Err(Error::ResourceConflict);
    }
    if fresh["managed"].as_bool() == Some(false) {
        return Err(Error::UnavailableAction);
    }
    Ok(())
}
fn empty(fresh: &Value, field: &str) -> Result<(), Error> {
    if refs(&fresh[field])?.is_empty() {
        Ok(())
    } else {
        Err(Error::ResourceInUse)
    }
}
fn nonmanagement(fresh: &Value) -> Result<(), Error> {
    if fresh["management"].as_bool() == Some(false) {
        Ok(())
    } else {
        Err(Error::ResourceInUse)
    }
}
async fn unused_network(session: &Session, reference: &Value) -> Result<(), Error> {
    let network = related(session, "network", reference).await?;
    empty(&network, "VIFs")
}
async fn pif_references(session: &Session, ids: &[PifId]) -> Result<BTreeSet<String>, Error> {
    let mut result = BTreeSet::new();
    for id in ids {
        result.insert(xapi::read(session, ResourceTarget::Pif(*id)).await?.0);
    }
    Ok(result)
}
fn ipv6_addresses(value: &Value) -> Option<BTreeSet<(std::net::Ipv6Addr, u8)>> {
    value
        .as_array()
        .filter(|v| v.len() <= 64)?
        .iter()
        .map(|v| crate::network_types::ipv6_cidr(v.as_str()?))
        .collect()
}
fn unchanged_ipv6(fresh: &Value, before: &crate::Ipv6Configuration) -> bool {
    fresh["ipv6_configuration_mode"] == before.mode.as_str()
        && ipv6_addresses(&fresh["IPv6"])
            == Some(
                before
                    .addresses
                    .iter()
                    .filter_map(|v| crate::network_types::ipv6_cidr(v))
                    .collect(),
            )
        && fresh["ipv6_gateway"] == before.gateway
        && fresh["DNS"] == before.dns
}
async fn configured_ipv6_host(session: &Session, fresh: &Value) -> Result<(), Error> {
    let interfaces = all(session, "PIF").await?;
    let management = interfaces
        .values()
        .filter(|v| v["host"] == fresh["host"] && v["management"].as_bool() == Some(true))
        .collect::<Vec<_>>();
    if management.len() != 1
        || !matches!(
            management[0]["ipv6_configuration_mode"].as_str(),
            Some("DHCP" | "Autoconf" | "Static")
        )
    {
        return Err(Error::Protocol(
            "IPv6 must already be enabled on the host management interface; changing the installed address family is unsupported",
        ));
    }
    Ok(())
}
fn configurable_pif(fresh: &Value) -> Result<(), Error> {
    nonmanagement(fresh)?;
    if fresh["managed"].as_bool() != Some(true)
        || fresh["bond_slave_of"].as_str() != Some("OpaqueRef:NULL")
    {
        return Err(Error::UnavailableAction);
    }
    Ok(())
}
fn unchanged_ipv4(fresh: &Value, before: &crate::Ipv4Configuration) -> bool {
    fresh["ip_configuration_mode"] == before.mode.as_str()
        && fresh["IP"] == before.address
        && fresh["netmask"] == before.netmask
        && fresh["gateway"] == before.gateway
        && fresh["DNS"] == before.dns
}
fn string_map(value: &Value) -> Result<serde_json::Map<String, Value>, Error> {
    let map = value
        .as_object()
        .filter(|m| m.len() <= 1024 && m.values().all(Value::is_string))
        .ok_or(Error::Protocol("network settings unavailable"))?;
    Ok(map.clone())
}
pub(super) async fn arguments(
    session: &Session,
    record: &ResourceOperationRecord,
) -> Result<Vec<Value>, Error> {
    let op = operation(record)?;
    let (target, fresh) = xapi::read(session, record.target).await?;
    let mut args = vec![json!(target)];
    match op {
        NetworkOperation::ScanInterfaces => {}
        NetworkOperation::SetAttached {
            expected,
            expected_attached,
            attached,
        } => {
            pif_identity(session, &fresh, expected).await?;
            if fresh["currently_attached"].as_bool() != Some(*expected_attached) {
                return Err(Error::ResourceConflict);
            }
            if !attached {
                nonmanagement(&fresh)?;
                if fresh["disallow_unplug"].as_bool() == Some(true) {
                    return Err(Error::Protocol("Interface unplug protection is enabled"));
                }
            }
        }
        NetworkOperation::CreateVlan {
            parent,
            expected,
            tag,
        } => {
            let (parent_ref, pif) = xapi::read(session, ResourceTarget::Pif(*parent)).await?;
            pif_identity(session, &pif, expected).await?;
            if number(&pif["VLAN"]) != Some(-1) || pif["bond_slave_of"] != "OpaqueRef:NULL" {
                return Err(Error::ResourceInUse);
            }
            for existing in refs(&fresh["PIFs"])? {
                let p = related(session, "PIF", &json!(existing)).await?;
                if p["host"] == pif["host"] {
                    return Err(Error::ResourceInUse);
                }
            }
            let vlans = all(session, "VLAN").await?;
            if vlans.values().any(|v| {
                v["tagged_PIF"] == parent_ref && number(&v["tag"]) == Some(i64::from(*tag))
            }) {
                return Err(Error::ResourceConflict);
            }
            args = vec![json!(parent_ref), json!(tag.to_string()), json!(target)];
        }
        NetworkOperation::DeleteVlan {
            parent,
            child,
            network,
            tag,
        } => {
            let (parent_ref, _) = xapi::read(session, ResourceTarget::Pif(*parent)).await?;
            let (child_ref, pif) = xapi::read(session, ResourceTarget::Pif(*child)).await?;
            let (network_ref, n) = xapi::read(session, ResourceTarget::Network(*network)).await?;
            if fresh["tagged_PIF"] != parent_ref
                || fresh["untagged_PIF"] != child_ref
                || number(&fresh["tag"]) != Some(i64::from(*tag))
                || pif["network"] != network_ref
            {
                return Err(Error::ResourceConflict);
            }
            nonmanagement(&pif)?;
            empty(&n, "VIFs")?;
        }
        NetworkOperation::CreateBond {
            members,
            mode,
            hash,
        } => {
            let mut member_refs = Vec::new();
            let mut host = None;
            for (id, expected) in members {
                let (reference, pif) = xapi::read(session, ResourceTarget::Pif(*id)).await?;
                pif_identity(session, &pif, expected).await?;
                nonmanagement(&pif)?;
                if pif["physical"] != true
                    || number(&pif["VLAN"]) != Some(-1)
                    || pif["bond_slave_of"] != "OpaqueRef:NULL"
                    || pif["ip_configuration_mode"] != "None"
                    || pif["ipv6_configuration_mode"] != "None"
                {
                    return Err(Error::ResourceInUse);
                }
                empty(&pif, "bond_master_of")?;
                empty(&pif, "VLAN_slave_of")?;
                unused_network(session, &pif["network"]).await?;
                if host.as_ref().is_some_and(|h| h != &pif["host"]) {
                    return Err(Error::ResourceConflict);
                }
                host = Some(pif["host"].clone());
                member_refs.push(reference);
            }
            for existing in refs(&fresh["PIFs"])? {
                let p = related(session, "PIF", &json!(existing)).await?;
                if Some(&p["host"]) == host.as_ref() {
                    return Err(Error::ResourceInUse);
                }
            }
            let props = if *mode == crate::BondMode::Lacp {
                json!({"hashing_algorithm":hash.as_str()})
            } else {
                json!({})
            };
            args.extend([json!(member_refs), json!(""), json!(mode.as_str()), props]);
        }
        NetworkOperation::DeleteBond {
            master,
            members,
            primary,
        } => {
            let (master_ref, pif) = xapi::read(session, ResourceTarget::Pif(*master)).await?;
            let (primary_ref, _) = xapi::read(session, ResourceTarget::Pif(*primary)).await?;
            if fresh["master"] != master_ref
                || fresh["primary_slave"] != primary_ref
                || refs(&fresh["slaves"])? != pif_references(session, members).await?
            {
                return Err(Error::ResourceConflict);
            }
            nonmanagement(&pif)?;
            if pif["ip_configuration_mode"] != "None" || pif["ipv6_configuration_mode"] != "None" {
                return Err(Error::ResourceInUse);
            }
            empty(&pif, "VLAN_slave_of")?;
            unused_network(session, &pif["network"]).await?;
        }
        NetworkOperation::SetBondMode {
            expected_mode,
            mode,
        } => {
            if fresh["mode"] != *expected_mode {
                return Err(Error::ResourceConflict);
            }
            nonmanagement(&related(session, "PIF", &fresh["master"]).await?)?;
            args.push(json!(mode.as_str()));
        }
        NetworkOperation::SetBondHash { expected, hash } => {
            let properties = string_map(&fresh["properties"])?;
            if properties.get("hashing_algorithm").and_then(Value::as_str) != expected.as_deref()
                || fresh["mode"] != "lacp"
            {
                return Err(Error::ResourceConflict);
            }
            nonmanagement(&related(session, "PIF", &fresh["master"]).await?)?;
            args.extend([json!("hashing_algorithm"), json!(hash.as_str())]);
        }
        NetworkOperation::ConfigureIpv6 {
            expected,
            before,
            configuration,
        } => {
            pif_identity(session, &fresh, expected).await?;
            configurable_pif(&fresh)?;
            configured_ipv6_host(session, &fresh).await?;
            if !unchanged_ipv6(&fresh, before) {
                return Err(Error::ResourceConflict);
            }
            args.extend([
                json!(configuration.mode.as_str()),
                json!(
                    configuration
                        .addresses
                        .first()
                        .map(String::as_str)
                        .unwrap_or("")
                ),
                json!(configuration.gateway),
                json!(configuration.dns),
            ]);
        }
        NetworkOperation::SetInterfacePurpose {
            expected,
            before,
            purpose,
        } => {
            pif_identity(session, &fresh, expected).await?;
            configurable_pif(&fresh)?;
            let mut map = string_map(&fresh["other_config"])?;
            if map.get("management_purpose").and_then(Value::as_str) != before.as_deref() {
                return Err(Error::ResourceConflict);
            }
            if let Some(purpose) = purpose {
                map.insert("management_purpose".into(), json!(purpose));
            } else {
                map.remove("management_purpose");
            }
            args.push(json!(map));
        }
        NetworkOperation::SetUnplugProtection {
            expected,
            before,
            enabled,
        } => {
            pif_identity(session, &fresh, expected).await?;
            configurable_pif(&fresh)?;
            if fresh["disallow_unplug"].as_bool() != Some(*before) {
                return Err(Error::ResourceConflict);
            }
            args.push(json!(enabled));
        }
        NetworkOperation::SetAutomaticAttachment { expected, enabled } => {
            let mut map = string_map(&fresh["other_config"])?;
            if map.get("automatic").and_then(Value::as_str) != expected.as_deref() {
                return Err(Error::ResourceConflict);
            }
            map.insert("automatic".into(), json!(enabled.to_string()));
            args.push(json!(map));
        }
        NetworkOperation::SetMtu {
            expected,
            mtu,
            pifs,
        } => {
            if number(&fresh["MTU"]) != i64::try_from(*expected).ok()
                || refs(&fresh["PIFs"])? != pif_references(session, pifs).await?
            {
                return Err(Error::ResourceConflict);
            }
            for id in pifs {
                nonmanagement(&xapi::read(session, ResourceTarget::Pif(*id)).await?.1)?;
            }
            args.push(json!(mtu.to_string()));
        }
        NetworkOperation::SetIgmpSnooping { expected, enabled } => {
            if fresh["igmp_snooping_enabled"].as_bool() != Some(*expected) {
                return Err(Error::ResourceConflict);
            }
            args.push(json!(enabled));
        }
        NetworkOperation::ConfigureIpv4 {
            expected,
            before,
            configuration,
        } => {
            pif_identity(session, &fresh, expected).await?;
            nonmanagement(&fresh)?;
            if !unchanged_ipv4(&fresh, before) {
                return Err(Error::ResourceConflict);
            }
            args.extend([
                json!(configuration.mode.as_str()),
                json!(configuration.address),
                json!(configuration.netmask),
                json!(configuration.gateway),
                json!(configuration.dns),
            ]);
        }
    }
    Ok(args)
}
fn method(op: &NetworkOperation) -> &'static str {
    match op {
        NetworkOperation::ConfigureIpv6 { .. } => "Async.PIF.reconfigure_ipv6",
        NetworkOperation::SetInterfacePurpose { .. } => "Async.PIF.set_other_config",
        NetworkOperation::SetUnplugProtection { .. } => "Async.PIF.set_disallow_unplug",
        NetworkOperation::ScanInterfaces => "Async.PIF.scan",
        NetworkOperation::SetAttached { attached: true, .. } => "Async.PIF.plug",
        NetworkOperation::SetAttached {
            attached: false, ..
        } => "Async.PIF.unplug",
        NetworkOperation::CreateVlan { .. } => "Async.VLAN.create",
        NetworkOperation::DeleteVlan { .. } => "Async.VLAN.destroy",
        NetworkOperation::CreateBond { .. } => "Async.Bond.create",
        NetworkOperation::DeleteBond { .. } => "Async.Bond.destroy",
        NetworkOperation::SetBondMode { .. } => "Async.Bond.set_mode",
        NetworkOperation::SetBondHash { .. } => "Async.Bond.set_property",
        NetworkOperation::SetAutomaticAttachment { .. } => "Async.network.set_other_config",
        NetworkOperation::SetMtu { .. } => "Async.network.set_MTU",
        NetworkOperation::SetIgmpSnooping { .. } => "Async.pool.set_igmp_snooping_enabled",
        NetworkOperation::ConfigureIpv4 { .. } => "Async.PIF.reconfigure_ip",
    }
}
async fn all(session: &Session, class: &str) -> Result<serde_json::Map<String, Value>, Error> {
    let values = session
        .call(&format!("{class}.get_all_records"), vec![])
        .await?;
    values
        .as_object()
        .filter(|v| v.len() <= 10_000)
        .cloned()
        .ok_or(Error::Protocol("network creation baseline unavailable"))
}
pub(super) async fn prepare(
    session: &Session,
    record: &mut ResourceOperationRecord,
) -> Result<Vec<Step>, Error> {
    arguments(session, record).await?;
    if operation(record)?.creates() {
        record.network_baseline = Some(NetworkCreationBaseline {
            vlans: all(session, "VLAN")
                .await?
                .values()
                .map(|v| uuid(v).map(Into::into))
                .collect::<Result<_, _>>()?,
            bonds: all(session, "Bond")
                .await?
                .values()
                .map(|v| uuid(v).map(Into::into))
                .collect::<Result<_, _>>()?,
            pifs: all(session, "PIF")
                .await?
                .values()
                .map(|v| uuid(v).map(Into::into))
                .collect::<Result<_, _>>()?,
        });
    }
    let op = operation(record)?;
    Ok(vec![Step {
        label: op.label().into(),
        method: method(op).into(),
        argument: None,
        expected: Value::Null,
        check: Check::Network,
    }])
}
pub(super) async fn verify(
    session: &Session,
    record: &ResourceOperationRecord,
) -> Result<Option<ResourceTarget>, Error> {
    let op = operation(record)?;
    if op.creates() {
        return verify_created(session, record).await.map(Some);
    }
    if matches!(
        op,
        NetworkOperation::DeleteVlan { .. } | NetworkOperation::DeleteBond { .. }
    ) {
        return match xapi::read(session, record.target).await {
            Err(Error::Api(code)) if code == "UUID_INVALID" => Ok(None),
            Ok(_) => Err(Error::ResourceConflict),
            Err(error) => Err(error),
        };
    }
    let (_, fresh) = xapi::read(session, record.target).await?;
    let matches = match op {
        NetworkOperation::ConfigureIpv6 {
            expected,
            configuration,
            ..
        } => {
            pif_identity(session, &fresh, expected).await?;
            fresh["ipv6_configuration_mode"] == configuration.mode.as_str()
                && (configuration.mode != crate::Ipv6Mode::Static
                    || (ipv6_addresses(&fresh["IPv6"]).is_some_and(|addresses| {
                        configuration.addresses.iter().all(|v| {
                            crate::network_types::ipv6_cidr(v)
                                .is_some_and(|v| addresses.contains(&v))
                        })
                    }) && fresh["ipv6_gateway"] == configuration.gateway))
                && (configuration.dns.is_empty() || fresh["DNS"] == configuration.dns)
        }
        NetworkOperation::SetInterfacePurpose {
            expected, purpose, ..
        } => {
            pif_identity(session, &fresh, expected).await?;
            fresh["other_config"].is_object()
                && fresh["other_config"]["management_purpose"].as_str() == purpose.as_deref()
        }
        NetworkOperation::SetUnplugProtection {
            expected, enabled, ..
        } => {
            pif_identity(session, &fresh, expected).await?;
            fresh["disallow_unplug"].as_bool() == Some(*enabled)
        }
        NetworkOperation::ScanInterfaces => true,
        NetworkOperation::SetAttached {
            expected, attached, ..
        } => {
            pif_identity(session, &fresh, expected).await?;
            fresh["currently_attached"].as_bool() == Some(*attached)
        }
        NetworkOperation::SetBondMode { mode, .. } => fresh["mode"] == mode.as_str(),
        NetworkOperation::SetBondHash { hash, .. } => {
            fresh["properties"]["hashing_algorithm"] == hash.as_str()
        }
        NetworkOperation::SetAutomaticAttachment { enabled, .. } => {
            fresh["other_config"]["automatic"].as_str()
                == Some(if *enabled { "true" } else { "false" })
        }
        NetworkOperation::SetMtu { mtu, .. } => number(&fresh["MTU"]) == i64::try_from(*mtu).ok(),
        NetworkOperation::SetIgmpSnooping { enabled, .. } => {
            fresh["igmp_snooping_enabled"].as_bool() == Some(*enabled)
        }
        NetworkOperation::ConfigureIpv4 {
            expected,
            configuration,
            ..
        } => {
            pif_identity(session, &fresh, expected).await?;
            fresh["ip_configuration_mode"] == configuration.mode.as_str()
                && (configuration.mode != crate::Ipv4Mode::Static
                    || (fresh["IP"] == configuration.address
                        && fresh["netmask"] == configuration.netmask
                        && fresh["gateway"] == configuration.gateway))
                && (configuration.dns.is_empty() || fresh["DNS"] == configuration.dns)
        }
        _ => false,
    };
    if matches {
        Ok(None)
    } else {
        Err(Error::ResourceConflict)
    }
}
async fn verify_created(
    session: &Session,
    record: &ResourceOperationRecord,
) -> Result<ResourceTarget, Error> {
    let reference = record.result_reference.as_deref().ok_or(Error::Protocol(
        "created network object identity unavailable; no write repeated",
    ))?;
    let baseline = record
        .network_baseline
        .as_ref()
        .ok_or(Error::Protocol("network creation evidence unavailable"))?;
    let (target, expected_pif, expected_network, id) = match operation(record)? {
        NetworkOperation::CreateVlan { parent, tag, .. } => {
            let vlan = related(session, "VLAN", &json!(reference)).await?;
            let id = crate::VlanId::from(uuid(&vlan)?);
            let (_, parent_record) = xapi::read(session, ResourceTarget::Pif(*parent)).await?;
            let tagged = related(session, "PIF", &vlan["tagged_PIF"]).await?;
            if baseline.vlans.contains(&id)
                || uuid(&tagged)? != parent.uuid()
                || number(&vlan["tag"]) != Some(i64::from(*tag))
            {
                return Err(Error::ResourceConflict);
            }
            (
                ResourceTarget::Vlan(id),
                vlan["untagged_PIF"].clone(),
                parent_record["host"].clone(),
                id.uuid(),
            )
        }
        NetworkOperation::CreateBond {
            members,
            mode,
            hash,
        } => {
            let bond = related(session, "Bond", &json!(reference)).await?;
            let id = crate::BondId::from(uuid(&bond)?);
            let member_ids = members.iter().map(|(id, _)| *id).collect::<Vec<_>>();
            if baseline.bonds.contains(&id)
                || refs(&bond["slaves"])? != pif_references(session, &member_ids).await?
                || bond["mode"] != mode.as_str()
                || (*mode == crate::BondMode::Lacp
                    && bond["properties"]["hashing_algorithm"] != hash.as_str())
            {
                return Err(Error::ResourceConflict);
            }
            let (_, member) = xapi::read(session, ResourceTarget::Pif(member_ids[0])).await?;
            (
                ResourceTarget::Bond(id),
                bond["master"].clone(),
                member["host"].clone(),
                id.uuid(),
            )
        }
        _ => return Err(Error::InvalidResourceCommand),
    };
    let pif = related(session, "PIF", &expected_pif).await?;
    let pif_id = PifId::from(uuid(&pif)?);
    let network = related(session, "network", &pif["network"]).await?;
    if baseline.pifs.contains(&pif_id)
        || uuid(&network)? != record.target.uuid()
        || pif["host"] != expected_network
    {
        return Err(Error::ResourceConflict);
    }
    let (_, rebound) = xapi::read(session, target).await?;
    if uuid(&rebound)? != id {
        return Err(Error::ResourceConflict);
    }
    Ok(target)
}
