use crate::{
    BondId, ClassAvailability, InventoryDetails, ManagementInterface, NetworkBond,
    PhysicalInterface, PifId, Vlan, VlanId,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// The connected VLAN/bond component observed when management settings were reviewed.
/// Runtime counters are omitted; interface configuration and relationships are retained.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagementTopology {
    pub pifs: BTreeMap<PifId, PhysicalInterface>,
    pub bonds: BTreeMap<BondId, NetworkBond>,
    pub vlans: BTreeMap<VlanId, Vlan>,
}
impl ManagementTopology {
    pub(crate) fn capture(details: &InventoryDetails, root: PifId) -> Option<Self> {
        if ["pif", "bond", "vlan"]
            .iter()
            .any(|class| details.classes.get(*class) != Some(&ClassAvailability::Available))
        {
            return None;
        }
        let mut pifs = BTreeSet::from([root]);
        let mut bonds = BTreeMap::new();
        let mut vlans = BTreeMap::new();
        loop {
            let size = pifs.len();
            for bond in details.bonds.values() {
                if bond.master.is_some_and(|p| pifs.contains(&p))
                    || bond.slaves.iter().any(|p| pifs.contains(p))
                {
                    pifs.insert(bond.master?);
                    pifs.extend(bond.slaves.iter().copied());
                    let mut bond = bond.clone();
                    bond.links_up = None;
                    bond.slaves.sort();
                    bonds.insert(bond.id, bond);
                }
            }
            for vlan in details.vlans.values() {
                if vlan.tagged_pif.is_some_and(|p| pifs.contains(&p))
                    || vlan.untagged_pif.is_some_and(|p| pifs.contains(&p))
                {
                    pifs.insert(vlan.tagged_pif?);
                    pifs.insert(vlan.untagged_pif?);
                    vlans.insert(vlan.id, vlan.clone());
                }
            }
            if pifs.len() > 256 || bonds.len() > 16 || vlans.len() > 128 {
                return None;
            }
            if size == pifs.len() {
                break;
            }
        }
        let pifs = pifs
            .into_iter()
            .map(|id| {
                let mut p = details.pifs.get(&id)?.clone();
                p.metrics = None;
                p.capabilities = None;
                p.ipv6 = crate::management_network::canonical_ipv6(&p.ipv6)?;
                Some((id, p))
            })
            .collect::<Option<BTreeMap<_, _>>>()?;
        let value = Self { pifs, bonds, vlans };
        value.valid(root).then_some(value)
    }
    pub(crate) fn valid(&self, root: PifId) -> bool {
        let Some(root_pif) = self.pifs.get(&root) else {
            return false;
        };
        if self.pifs.is_empty()
            || self.pifs.len() > 256
            || self.bonds.len() > 16
            || self.vlans.len() > 128
            || root_pif.host.is_none()
        {
            return false;
        }
        if self.pifs.iter().any(|(id, p)| {
            *id != p.id
                || id.uuid().is_nil()
                || p.host != root_pif.host
                || p.network.is_none_or(|id| id.uuid().is_nil())
                || p.managed != Some(true)
                || p.mtu != Some(1500)
                || p.management.is_none()
                || p.currently_attached.is_none()
                || p.disallow_unplug.is_none()
                || p.metrics.is_some()
                || p.capabilities.is_some()
                || p.management_purpose
                    .as_ref()
                    .is_some_and(|v| v.len() > 256 || v.chars().any(char::is_control))
                || ManagementInterface::from_inventory(p).is_none()
                || p.device.is_empty()
                || p.device.len() > 256
                || p.mac.len() > 64
                || [
                    p.ip.as_str(),
                    p.netmask.as_str(),
                    p.gateway.as_str(),
                    p.ipv6_gateway.as_str(),
                ]
                .iter()
                .any(|v| v.len() > 128)
                || p.dns.len() > 2048
                || p.ipv6.len() > 64
                || [p.ip.as_str(), p.netmask.as_str(), p.gateway.as_str()]
                    .iter()
                    .any(|v| !v.is_empty() && v.parse::<std::net::Ipv4Addr>().is_err())
                || (!p.ipv6_gateway.is_empty()
                    && p.ipv6_gateway.parse::<std::net::Ipv6Addr>().is_err())
                || p.dns
                    .split(',')
                    .filter(|v| !v.is_empty())
                    .any(|v| v.parse::<std::net::IpAddr>().is_err())
        }) {
            return false;
        }
        if self.bonds.iter().any(|(id, b)| {
            *id != b.id
                || id.uuid().is_nil()
                || b.master.is_none()
                || b.slaves.len() < 2
                || b.slaves.len() > 16
                || b.slaves.iter().copied().collect::<BTreeSet<_>>().len() != b.slaves.len()
                || b.slaves
                    .iter()
                    .any(|p| Some(*p) == b.master || !self.pifs.contains_key(p))
                || b.master.is_none_or(|p| !self.pifs.contains_key(&p))
                || b.primary_slave.is_none_or(|p| !b.slaves.contains(&p))
                || !["active-backup", "balance-slb", "lacp"].contains(&b.mode.as_str())
                || !b.hashing_algorithm_known
                || b.hashing_algorithm
                    .as_ref()
                    .is_some_and(|v| !["src_mac", "tcpudp_ports"].contains(&v.as_str()))
                || b.links_up.is_some()
        }) {
            return false;
        }
        if self.vlans.iter().any(|(id, v)| {
            *id != v.id
                || id.uuid().is_nil()
                || v.tag.is_none_or(|tag| tag > 4094)
                || v.tagged_pif.is_none()
                || v.tagged_pif == v.untagged_pif
                || v.tagged_pif.is_none_or(|p| !self.pifs.contains_key(&p))
                || v.untagged_pif.is_none_or(|p| !self.pifs.contains_key(&p))
        }) {
            return false;
        }
        if self
            .vlans
            .values()
            .map(|v| (v.tagged_pif, v.tag))
            .collect::<BTreeSet<_>>()
            .len()
            != self.vlans.len()
        {
            return false;
        }
        for (id, p) in &self.pifs {
            let masters = self
                .bonds
                .values()
                .filter(|b| b.master == Some(*id))
                .count();
            let slaves = self
                .bonds
                .values()
                .filter(|b| b.slaves.contains(id))
                .count();
            let vlans = self
                .vlans
                .values()
                .filter(|v| v.untagged_pif == Some(*id))
                .collect::<Vec<_>>();
            if masters > 1 || slaves > 1 || vlans.len() > 1 || (*id == root && slaves != 0) {
                return false;
            }
            let valid = match (p.physical, p.vlan, masters, slaves, vlans.as_slice()) {
                (Some(true), Some(-1), 0, _, []) => true,
                (Some(false), Some(-1), 1, 0, []) => true,
                (Some(false), Some(tag), 0, 0, [v]) if Some(tag as u64) == v.tag && tag >= 0 => {
                    let parent = &self.pifs[&v.tagged_pif.unwrap()];
                    parent.vlan == Some(-1)
                        && !self.bonds.values().any(|b| b.slaves.contains(&parent.id))
                }
                _ => false,
            };
            if !valid {
                return false;
            }
        }
        let mut reachable = BTreeSet::from([root]);
        loop {
            let size = reachable.len();
            for b in self.bonds.values() {
                if reachable.contains(&b.master.unwrap())
                    || b.slaves.iter().any(|p| reachable.contains(p))
                {
                    reachable.insert(b.master.unwrap());
                    reachable.extend(&b.slaves);
                }
            }
            for v in self.vlans.values() {
                if reachable.contains(&v.tagged_pif.unwrap())
                    || reachable.contains(&v.untagged_pif.unwrap())
                {
                    reachable.insert(v.tagged_pif.unwrap());
                    reachable.insert(v.untagged_pif.unwrap());
                }
            }
            if size == reachable.len() {
                return reachable == self.pifs.keys().copied().collect();
            }
        }
    }
    pub(crate) fn matches_root(&self, root: &ManagementInterface) -> bool {
        let Some(p) = self.pifs.get(&root.pif) else {
            return false;
        };
        let mut expected = root.clone();
        expected.topology = None;
        ManagementInterface::from_inventory(p).as_ref() == Some(&expected)
    }
    pub(crate) fn without_endpoints(&self, endpoints: &BTreeSet<PifId>) -> Self {
        let mut result = self.clone();
        for (id, p) in &mut result.pifs {
            if endpoints.contains(id) {
                p.management = None;
                p.ip_configuration_mode.clear();
                p.ip.clear();
                p.netmask.clear();
                p.gateway.clear();
                p.dns.clear();
                p.ipv6_configuration_mode.clear();
                p.ipv6.clear();
                p.ipv6_gateway.clear();
            }
        }
        result
    }
}
