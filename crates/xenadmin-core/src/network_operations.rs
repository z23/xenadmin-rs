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
use xenadmin_xapi::enumerations::{
    bond_mode, ip_configuration_mode, ipv6_configuration_mode, primary_address_type,
};
use xenadmin_xapi::records::{BondRecord, PifRecord, VlanRecord};
use xenadmin_xapi::{Reads, kind};

pub(super) fn operation(record: &ResourceOperationRecord) -> Result<&NetworkOperation, Error> {
    match &record.action {
        ResourceAction::Network(op) => Ok(op),
        _ => Err(Error::InvalidResourceCommand),
    }
}
fn reference(value: &Value) -> Result<String, Error> {
    xapi::reply_reference(value)
}
pub(super) fn number<'a>(value: impl Reads<'a, kind::Whole>) -> Option<i64> {
    let value = value.raw();
    value.as_i64().or_else(|| value.as_str()?.parse().ok())
}
pub(super) fn refs<'a>(value: impl Reads<'a, kind::Set>) -> Result<BTreeSet<String>, Error> {
    let value = value.raw();
    value
        .as_array()
        .filter(|v| v.len() <= 10_000)
        .ok_or(Error::Protocol("interface membership unavailable"))?
        .iter()
        .map(reference)
        .collect()
}
pub(super) async fn related<'a>(
    session: &Session,
    class: &str,
    value: impl Reads<'a, kind::Reference>,
) -> Result<Value, Error> {
    let value = value.raw();
    session
        .call(
            &format!("{class}.get_record"),
            vec![json!(reference(value)?)],
        )
        .await
}
pub(super) fn uuid(value: &Value) -> Result<uuid::Uuid, Error> {
    value["uuid"]
        .as_str()
        .and_then(|v| uuid::Uuid::parse_str(v).ok())
        .ok_or(Error::Protocol("network object UUID unavailable"))
}
pub(super) async fn pif_identity(
    session: &Session,
    fresh: &Value,
    expected: &PifIdentity,
) -> Result<(), Error> {
    if uuid(&related(session, "host", PifRecord::new(fresh).host()).await?)? != expected.host.uuid()
        || uuid(&related(session, "network", PifRecord::new(fresh).network()).await?)?
            != expected.network.uuid()
        || PifRecord::new(fresh).device() != expected.device
        || PifRecord::new(fresh).mac() != expected.mac
        || PifRecord::new(fresh).management().get() != Some(expected.management)
    {
        return Err(Error::ResourceConflict);
    }
    if PifRecord::new(fresh).managed().get() == Some(false) {
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
    if PifRecord::new(fresh).management().get() == Some(false) {
        Ok(())
    } else {
        Err(Error::ResourceInUse)
    }
}
async fn unused_network<'a>(
    session: &Session,
    reference: impl Reads<'a, kind::Reference>,
) -> Result<(), Error> {
    let reference = reference.raw();
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
fn ipv6_addresses<'a>(
    value: impl Reads<'a, kind::Set>,
) -> Option<BTreeSet<(std::net::Ipv6Addr, u8)>> {
    let value = value.raw();
    value
        .as_array()
        .filter(|v| v.len() <= 64)?
        .iter()
        .filter(|v| v.as_str() != Some(""))
        .map(|v| crate::network_types::ipv6_cidr(v.as_str()?))
        .collect()
}
fn unchanged_ipv6(fresh: &Value, before: &crate::Ipv6Configuration) -> bool {
    PifRecord::new(fresh).ipv6_configuration_mode() == before.mode.as_str()
        && ipv6_addresses(PifRecord::new(fresh).ipv6())
            == Some(
                before
                    .addresses
                    .iter()
                    .filter_map(|v| crate::network_types::ipv6_cidr(v))
                    .collect(),
            )
        && PifRecord::new(fresh).ipv6_gateway() == before.gateway
        && PifRecord::new(fresh).dns() == before.dns
}
async fn configured_ipv6_host(session: &Session, fresh: &Value) -> Result<(), Error> {
    let interfaces = all(session, "PIF").await?;
    let management = interfaces
        .values()
        .filter(|v| {
            PifRecord::new(v).host() == fresh["host"]
                && PifRecord::new(v).management().get() == Some(true)
        })
        .collect::<Vec<_>>();
    if management.len() != 1
        || !matches!(
            management[0]["ipv6_configuration_mode"].as_str(),
            Some(
                ipv6_configuration_mode::DHCP
                    | ipv6_configuration_mode::AUTOCONF
                    | ipv6_configuration_mode::STATIC
            )
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
    if PifRecord::new(fresh).managed().get() != Some(true)
        || PifRecord::new(fresh).bond_slave_of().reference() != Some("OpaqueRef:NULL")
    {
        return Err(Error::UnavailableAction);
    }
    Ok(())
}
fn unchanged_ipv4(fresh: &Value, before: &crate::Ipv4Configuration) -> bool {
    PifRecord::new(fresh).ip_configuration_mode() == before.mode.as_str()
        && PifRecord::new(fresh).ip() == before.address
        && PifRecord::new(fresh).netmask() == before.netmask
        && PifRecord::new(fresh).gateway() == before.gateway
        && PifRecord::new(fresh).dns() == before.dns
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
    if matches!(op, NetworkOperation::Fabric(_)) {
        return super::network_fabric::arguments(session, record).await;
    }
    let (target, fresh) = xapi::read(session, record.target).await?;
    let mut args = vec![json!(target)];
    match op {
        NetworkOperation::Fabric(_) => unreachable!(),
        NetworkOperation::SetServicePurpose { purpose, .. } => {
            super::network_service::check(session, record, &target, false).await?;
            args.push(json!(purpose.as_str()));
        }
        NetworkOperation::ScanInterfaces => {}
        NetworkOperation::ForgetInterface { expected } => {
            pif_identity(session, &fresh, expected).await?;
            nonmanagement(&fresh)?;
            if fresh["physical"] != true
                || number(&fresh["VLAN"]) != Some(-1)
                || fresh["bond_slave_of"] != "OpaqueRef:NULL"
            {
                return Err(Error::ResourceInUse);
            }
            empty(&fresh, "bond_master_of")?;
            empty(&fresh, "VLAN_slave_of")?;
            empty(&fresh, "tunnel_access_PIF_of")?;
        }
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
            if number(PifRecord::new(&pif).vlan()) != Some(-1)
                || PifRecord::new(&pif).bond_slave_of() != "OpaqueRef:NULL"
            {
                return Err(Error::ResourceInUse);
            }
            for existing in refs(&fresh["PIFs"])? {
                let p = related(session, "PIF", &json!(existing)).await?;
                if PifRecord::new(&p).host() == PifRecord::new(&pif).host() {
                    return Err(Error::ResourceInUse);
                }
            }
            let vlans = all(session, "VLAN").await?;
            if vlans.values().any(|v| {
                VlanRecord::new(v).tagged_pif() == parent_ref
                    && number(VlanRecord::new(v).tag()) == Some(i64::from(*tag))
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
                || PifRecord::new(&pif).network() != network_ref
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
                if PifRecord::new(&pif).physical() != true
                    || number(PifRecord::new(&pif).vlan()) != Some(-1)
                    || PifRecord::new(&pif).bond_slave_of() != "OpaqueRef:NULL"
                    || PifRecord::new(&pif).ip_configuration_mode() != ip_configuration_mode::NONE
                    || PifRecord::new(&pif).ipv6_configuration_mode()
                        != ipv6_configuration_mode::NONE
                {
                    return Err(Error::ResourceInUse);
                }
                empty(&pif, "bond_master_of")?;
                empty(&pif, "VLAN_slave_of")?;
                unused_network(session, PifRecord::new(&pif).network()).await?;
                if host
                    .as_ref()
                    .is_some_and(|h| h != &PifRecord::new(&pif).host())
                {
                    return Err(Error::ResourceConflict);
                }
                host = Some(json!(PifRecord::new(&pif).host()));
                member_refs.push(reference);
            }
            for existing in refs(&fresh["PIFs"])? {
                let p = related(session, "PIF", &json!(existing)).await?;
                if host
                    .as_ref()
                    .is_some_and(|h| h == &PifRecord::new(&p).host())
                {
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
            if PifRecord::new(&pif).ip_configuration_mode() != ip_configuration_mode::NONE
                || PifRecord::new(&pif).ipv6_configuration_mode() != ipv6_configuration_mode::NONE
            {
                return Err(Error::ResourceInUse);
            }
            empty(&pif, "VLAN_slave_of")?;
            unused_network(session, PifRecord::new(&pif).network()).await?;
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
                || fresh["mode"] != bond_mode::LACP
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
pub(super) async fn all(
    session: &Session,
    class: &str,
) -> Result<serde_json::Map<String, Value>, Error> {
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
    if matches!(operation(record)?, NetworkOperation::Fabric(_)) {
        super::network_fabric::prepare(session, record).await?;
    } else if operation(record)?.creates() {
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
        method: op.method().into(),
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
    if matches!(op, NetworkOperation::Fabric(_)) {
        return super::network_fabric::verify(session, record).await;
    }
    if op.creates() {
        return verify_created(session, record).await.map(Some);
    }
    if matches!(
        op,
        NetworkOperation::DeleteVlan { .. }
            | NetworkOperation::DeleteBond { .. }
            | NetworkOperation::ForgetInterface { .. }
    ) {
        return match xapi::read(session, record.target).await {
            Err(Error::Api(code)) if code == "UUID_INVALID" => Ok(None),
            Ok(_) => Err(Error::ResourceConflict),
            Err(error) => Err(error),
        };
    }
    let (target, fresh) = xapi::read(session, record.target).await?;
    let matches = match op {
        NetworkOperation::Fabric(_) => unreachable!(),
        NetworkOperation::SetServicePurpose { .. } => {
            super::network_service::check(session, record, &target, true).await?;
            true
        }
        NetworkOperation::ConfigureIpv6 {
            expected,
            configuration,
            ..
        } => {
            pif_identity(session, &fresh, expected).await?;
            fresh["ipv6_configuration_mode"] == configuration.mode.as_str()
                && (configuration.mode != crate::Ipv6Mode::Static
                    || (ipv6_addresses(&fresh[primary_address_type::IPV6]).is_some_and(
                        |addresses| {
                            configuration.addresses.iter().all(|v| {
                                crate::network_types::ipv6_cidr(v)
                                    .is_some_and(|v| addresses.contains(&v))
                            })
                        },
                    ) && fresh["ipv6_gateway"] == configuration.gateway))
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
        NetworkOperation::ForgetInterface { .. } => unreachable!(),
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
            let tagged = related(session, "PIF", VlanRecord::new(&vlan).tagged_pif()).await?;
            if baseline.vlans.contains(&id)
                || uuid(&tagged)? != parent.uuid()
                || number(VlanRecord::new(&vlan).tag()) != Some(i64::from(*tag))
            {
                return Err(Error::ResourceConflict);
            }
            (
                ResourceTarget::Vlan(id),
                json!(VlanRecord::new(&vlan).untagged_pif()),
                json!(PifRecord::new(&parent_record).host()),
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
                || refs(BondRecord::new(&bond).slaves())?
                    != pif_references(session, &member_ids).await?
                || BondRecord::new(&bond).mode() != mode.as_str()
                || (*mode == crate::BondMode::Lacp
                    && BondRecord::new(&bond).properties()["hashing_algorithm"] != hash.as_str())
            {
                return Err(Error::ResourceConflict);
            }
            let (_, member) = xapi::read(session, ResourceTarget::Pif(member_ids[0])).await?;
            (
                ResourceTarget::Bond(id),
                json!(BondRecord::new(&bond).master()),
                json!(PifRecord::new(&member).host()),
                id.uuid(),
            )
        }
        _ => return Err(Error::InvalidResourceCommand),
    };
    let pif = related(session, "PIF", &expected_pif).await?;
    let pif_id = PifId::from(uuid(&pif)?);
    let network = related(session, "network", PifRecord::new(&pif).network()).await?;
    if baseline.pifs.contains(&pif_id)
        || uuid(&network)? != record.target.uuid()
        || PifRecord::new(&pif).host() != expected_network
    {
        return Err(Error::ResourceConflict);
    }
    let (_, rebound) = xapi::read(session, target).await?;
    if uuid(&rebound)? != id {
        return Err(Error::ResourceConflict);
    }
    Ok(target)
}
