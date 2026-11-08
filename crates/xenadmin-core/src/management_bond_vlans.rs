use super::*;
use crate::{ManagementVlanReplacement, VlanId, management_bond::CapturedVlan};
pub(in crate::resources) fn before(
    t: &Records,
    expected: &BTreeMap<VlanId, CapturedVlan>,
) -> Result<(), Error> {
    for captured in expected.values() {
        let (_, v) = lookup(t.records("vlan"), captured.vlan.id.uuid())?;
        let (_, p) = t.pif(captured.interface.id)?;
        require(
            v["other_config"].as_object().is_some_and(|v| v.is_empty())
                && p["other_config"].as_object().is_some_and(|v| v.is_empty())
                && properties(p)?.is_empty(),
            "VLAN metadata or interface properties would be reset by the host migration; review or clear them separately",
        )?;
    }
    Ok(())
}
fn find(
    t: &Records,
    r: &ResourceOperationRecord,
    parent: PifId,
    expected: &BTreeMap<VlanId, CapturedVlan>,
    strict: bool,
) -> Result<BTreeMap<VlanId, ManagementVlanReplacement>, Error> {
    let b = r
        .management_network_plan
        .as_ref()
        .and_then(|p| p.bond.as_ref())
        .ok_or(Error::Journal)?;
    identities(t, b, parent, expected, strict)
}
pub(in crate::resources) fn identities(
    t: &Records,
    b: &Baseline,
    parent: PifId,
    expected: &BTreeMap<VlanId, CapturedVlan>,
    strict: bool,
) -> Result<BTreeMap<VlanId, ManagementVlanReplacement>, Error> {
    if expected.is_empty() {
        return Ok(BTreeMap::new());
    }
    let before = b.vlans.as_ref().ok_or(Error::Journal)?;
    let parent_ref = t.pif(parent)?.0;
    let mut out = BTreeMap::new();
    for (id, captured) in expected {
        let found = (|| {
            require(
                !t.records("vlan")
                    .values()
                    .any(|v| network::uuid(v).ok() == Some(id.uuid()))
                    && !t
                        .pifs()
                        .values()
                        .any(|v| network::uuid(v).ok() == Some(captured.interface.id.uuid())),
                "Old VLAN or interface still exists after the bond transition",
            )?;
            let candidates = t
                .records("vlan")
                .values()
                .filter(|v| {
                    v["tagged_PIF"] == parent_ref
                        && network::number(&v["tag"])
                            == captured.vlan.tag.and_then(|v| i64::try_from(v).ok())
                })
                .collect::<Vec<_>>();
            require(
                candidates.len() == 1,
                "VLAN replacement is missing or ambiguous",
            )?;
            let v = candidates[0];
            let child = t
                .pifs()
                .get(v["untagged_PIF"].as_str().ok_or(Error::ResourceConflict)?)
                .ok_or(Error::ResourceConflict)?;
            let new_vlan = VlanId::from(network::uuid(v)?);
            let new_interface = PifId::from(network::uuid(child)?);
            let replacement = ManagementVlanReplacement {
                old_vlan: *id,
                new_vlan,
                old_interface: captured.interface.id,
                new_interface,
            };
            require(
                !before.contains(&new_vlan)
                    && !b.pifs.contains(&new_interface)
                    && b.replacements.get(id).is_none_or(|old| old == &replacement)
                    && child["network"]
                        == lookup(
                            t.records("network"),
                            captured.interface.network.ok_or(Error::Journal)?.uuid(),
                        )?
                        .0
                    && child["host"] == receiver_host(t, parent)?,
                "VLAN replacement identity, network or provenance differs from review",
            )?;
            Ok(replacement)
        })();
        match found {
            Ok(replacement) => {
                out.insert(*id, replacement);
            }
            Err(e) if strict => return Err(e),
            Err(_) => {}
        }
    }
    Ok(out)
}
pub(in crate::resources) async fn replacements(
    s: &Session,
    r: &ResourceOperationRecord,
) -> Result<BTreeMap<VlanId, ManagementVlanReplacement>, Error> {
    let (source, op) = bond_operation(r)?;
    let expected = op
        .captured_vlans(source)
        .ok_or(Error::InvalidResourceCommand)?;
    let t = Records::read(s).await?;
    let parent = match op {
        Op::Create { .. } => created_from(&t, r)?.master,
        Op::Destroy { primary, .. } => *primary,
    };
    find(&t, r, parent, &expected, false)
}
pub(super) fn verify(
    t: &Records,
    r: &ResourceOperationRecord,
    parent: PifId,
    expected: &BTreeMap<VlanId, CapturedVlan>,
) -> Result<(), Error> {
    let recorded = &r
        .management_network_plan
        .as_ref()
        .and_then(|p| p.bond.as_ref())
        .ok_or(Error::Journal)?
        .replacements;
    require(
        &find(t, r, parent, expected, true)? == recorded,
        "Recorded VLAN replacement identities differ from the resulting topology",
    )?;
    for (id, captured) in expected {
        let replacement = recorded.get(id).ok_or(Error::Journal)?;
        let p = t.pif(replacement.new_interface)?.1;
        let before = &captured.interface;
        let v = lookup(t.records("vlan"), replacement.new_vlan.uuid())?.1;
        require(
            basic(p)
                && p["physical"] == false
                && network::number(&p["VLAN"])
                    == captured.vlan.tag.and_then(|v| i64::try_from(v).ok())
                && p["MAC"]
                    .as_str()
                    .is_some_and(|v| v.eq_ignore_ascii_case("fe:ff:ff:ff:ff:ff"))
                && p["device"] == t.pif(parent)?.1["device"]
                && p["management"].as_bool() == before.management
                && p["currently_attached"].as_bool() == before.currently_attached
                && p["primary_address_type"] == "IPv4"
                && p["ip_configuration_mode"] == before.ip_configuration_mode
                && p["IP"] == before.ip
                && p["netmask"] == before.netmask
                && p["gateway"] == before.gateway
                && dns(p["DNS"].as_str().ok_or(Error::ResourceConflict)?)? == dns(&before.dns)?
                && p["ipv6_configuration_mode"] == "None"
                && addresses(&p["IPv6"])?.is_empty()
                && p["ipv6_gateway"] == ""
                && p["other_config"].as_object().is_some_and(|v| v.is_empty())
                && properties(p)?.is_empty()
                && v["other_config"].as_object().is_some_and(|v| v.is_empty()),
            "Migrated VLAN configuration differs from the reviewed interface",
        )?;
    }
    Ok(())
}
