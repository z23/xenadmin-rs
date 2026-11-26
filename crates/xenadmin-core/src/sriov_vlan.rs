use crate::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use xenadmin_xapi::enumerations::{ip_configuration_mode, ipv6_configuration_mode};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SriovVlanUplink {
    pub target: NetworkId,
    pub object: NetworkSriov,
    pub fabric: NetworkFabricReview,
    pub preserved: BTreeMap<VlanId, (Vlan, PhysicalInterface)>,
}

impl SriovVlanUplink {
    pub(crate) fn capture(
        i: &Inventory,
        parent: PifId,
        target: NetworkId,
    ) -> Result<Option<Self>, Error> {
        let f = &i.details.network_fabric;
        let objects = f
            .sriovs
            .values()
            .filter(|s| s.logical == Some(parent))
            .collect::<Vec<_>>();
        if objects.is_empty() {
            let p = i.details.pifs.get(&parent).ok_or(Error::ResourceConflict)?;
            if p.physical == Some(true)
                || i.details.bonds.values().any(|b| b.master == Some(parent))
            {
                return Ok(None);
            }
            return Err(Error::Protocol(
                "The uplink is neither a physical interface, bond master nor a known SR-IOV logical interface",
            ));
        }
        let [object] = objects.as_slice() else {
            return Err(Error::ResourceConflict);
        };
        let physical = object.physical.ok_or(Error::ResourceConflict)?;
        let logical = i.details.pifs.get(&parent).ok_or(Error::ResourceConflict)?;
        let network = logical.network.ok_or(Error::ResourceConflict)?;
        if object.requires_reboot != Some(false)
            || !matches!(
                object.mode,
                Some(
                    SriovConfigurationMode::Sysfs
                        | SriovConfigurationMode::Modprobe
                        | SriovConfigurationMode::Manual
                )
            )
            || logical.currently_attached != Some(true)
            || i.details
                .pifs
                .get(&physical)
                .is_none_or(|p| p.currently_attached != Some(true))
        {
            return Err(Error::Protocol(
                "The physical and logical SR-IOV interfaces must already be attached with a known mode and no pending reboot",
            ));
        }
        let fabric =
            NetworkFabricReview::capture(i, network, physical, NetworkFabricKind::Sriov, false)?;
        let mut preserved = BTreeMap::new();
        for v in i
            .details
            .vlans
            .values()
            .filter(|v| v.tagged_pif == Some(parent))
        {
            let p = v
                .untagged_pif
                .and_then(|id| i.details.pifs.get(&id))
                .ok_or(Error::ResourceConflict)?;
            if p.network != Some(target) {
                preserved.insert(
                    v.id,
                    (v.clone(), crate::network_fabric::reviewed_interface(p)),
                );
            }
        }
        let result = Self {
            target,
            object: (*object).clone(),
            fabric,
            preserved,
        };
        result.validate(parent, target)?;
        for p in std::iter::once(logical).chain(result.preserved.values().map(|(_, p)| p)) {
            if p.management != Some(false)
                || p.disallow_unplug != Some(false)
                || p.management_purpose.is_some()
                || p.ip_configuration_mode != ip_configuration_mode::NONE
                || p.ipv6_configuration_mode != ipv6_configuration_mode::NONE
                || !p.ip.is_empty()
                || !p.ipv6.iter().all(String::is_empty)
                || p.network
                    .and_then(|n| i.details.networks.get(&n))
                    .and_then(|n| n.purposes.as_ref())
                    .is_none_or(|p| !p.is_empty())
                || i.details.vifs.values().any(|v| v.network == p.network)
                || i.details
                    .bonds
                    .values()
                    .any(|b| b.master == Some(p.id) || b.slaves.contains(&p.id))
                || f.tunnels
                    .values()
                    .any(|t| t.access == Some(p.id) || t.transport == Some(p.id))
            {
                return Err(Error::Protocol(
                    "The SR-IOV parent and its retained VLANs must have no protected, IP, service, VM or bond/tunnel consumers",
                ));
            }
        }
        Ok(Some(result))
    }

    pub(crate) fn validate(&self, parent: PifId, target: NetworkId) -> Result<(), Error> {
        let r = &self.fabric;
        let op = NetworkFabricOperation {
            review: r.clone(),
            change: NetworkFabricChange::Delete(NetworkFabricObject::Sriov(self.object.id)),
        };
        op.validate(ResourceTarget::Network(r.network))?;
        if self.target != target
            || r.kind != NetworkFabricKind::Sriov
            || self.object.logical != Some(parent)
            || self.object.physical != Some(r.input.id)
            || r.network == target
            || r.members.get(&parent).is_none_or(|m| {
                m.object != NetworkFabricObject::Sriov(self.object.id) || m.input != r.input.id
            })
            || self.preserved.len() > 1024
            || self.preserved.iter().any(|(id, (v, p))| {
                *id != v.id
                    || v.tagged_pif != Some(parent)
                    || v.untagged_pif != Some(p.id)
                    || p.network == Some(target)
            })
        {
            return Err(Error::InvalidResourceCommand);
        }
        Ok(())
    }
}

impl VlanUplink {
    pub fn capture(i: &Inventory, pif: PifId, target: NetworkId) -> Result<Self, Error> {
        let expected = i
            .details
            .pifs
            .get(&pif)
            .and_then(PifIdentity::from_inventory)
            .ok_or(Error::ResourceConflict)?;
        Ok(Self {
            pif,
            expected,
            sriov: SriovVlanUplink::capture(i, pif, target)?.map(Box::new),
        })
    }
}
