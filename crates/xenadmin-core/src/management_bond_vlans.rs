use super::*;
use crate::{ManagementVlanReplacement, VlanId, management_bond::CapturedVlan};
use xenadmin_xapi::enumerations::{ipv6_configuration_mode, primary_address_type};
use xenadmin_xapi::records::{PifRecord, VlanRecord};
pub(in crate::resources) fn before(
    t: &Records,
    expected: &BTreeMap<VlanId, CapturedVlan>,
) -> Result<(), Error> {
    for captured in expected.values() {
        let (_, v) = lookup(t.records("vlan"), captured.vlan.id.uuid())?;
        let (_, p) = t.pif(captured.interface.id)?;
        require(
            VlanRecord::new(v)
                .other_config()
                .entries()
                .is_some_and(|v| v.is_empty())
                && PifRecord::new(p)
                    .other_config()
                    .entries()
                    .is_some_and(|v| v.is_empty())
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
                    VlanRecord::new(v).tagged_pif() == parent_ref
                        && network::number(VlanRecord::new(v).tag())
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
                .get(
                    VlanRecord::new(v)
                        .untagged_pif()
                        .reference()
                        .ok_or(Error::ResourceConflict)?,
                )
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
                    && PifRecord::new(child).network()
                        == lookup(
                            t.records("network"),
                            captured.interface.network.ok_or(Error::Journal)?.uuid(),
                        )?
                        .0
                    && PifRecord::new(child).host() == receiver_host(t, parent)?,
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
                && PifRecord::new(p).physical() == false
                && network::number(PifRecord::new(p).vlan())
                    == captured.vlan.tag.and_then(|v| i64::try_from(v).ok())
                && PifRecord::new(p)
                    .mac()
                    .text()
                    .is_some_and(|v| v.eq_ignore_ascii_case("fe:ff:ff:ff:ff:ff"))
                && PifRecord::new(p).device() == PifRecord::new(t.pif(parent)?.1).device()
                && PifRecord::new(p).management().get() == before.management
                && PifRecord::new(p).currently_attached().get() == before.currently_attached
                && PifRecord::new(p).primary_address_type() == primary_address_type::IPV4
                && PifRecord::new(p).ip_configuration_mode() == before.ip_configuration_mode
                && PifRecord::new(p).ip() == before.ip
                && PifRecord::new(p).netmask() == before.netmask
                && PifRecord::new(p).gateway() == before.gateway
                && dns(PifRecord::new(p)
                    .dns()
                    .text()
                    .ok_or(Error::ResourceConflict)?)?
                    == dns(&before.dns)?
                && PifRecord::new(p).ipv6_configuration_mode() == ipv6_configuration_mode::NONE
                && addresses(PifRecord::new(p).ipv6())?.is_empty()
                && PifRecord::new(p).ipv6_gateway() == ""
                && PifRecord::new(p)
                    .other_config()
                    .entries()
                    .is_some_and(|v| v.is_empty())
                && properties(p)?.is_empty()
                && VlanRecord::new(v)
                    .other_config()
                    .entries()
                    .is_some_and(|v| v.is_empty()),
            "Migrated VLAN configuration differs from the reviewed interface",
        )?;
    }
    Ok(())
}
