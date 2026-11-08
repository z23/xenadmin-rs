use super::*;
use crate::{Ipv4Mode, Ipv6Mode, PhysicalInterface, Vlan, VlanId};
use std::collections::BTreeMap;
#[derive(Clone)]
pub(crate) struct CapturedVlan {
    pub vlan: Vlan,
    pub interface: PhysicalInterface,
}
fn empty_ip(p: &ManagementInterface) -> bool {
    p.ipv4.mode == Ipv4Mode::None
        && p.ipv4.valid()
        && p.ipv4.dns.is_empty()
        && p.ipv6.mode == Ipv6Mode::None
        && p.ipv6.addresses.is_empty()
        && p.ipv6.gateway.is_empty()
}
fn supported_vlan(p: &PhysicalInterface) -> bool {
    let Some(i) = ManagementInterface::from_inventory(p) else {
        return false;
    };
    p.physical == Some(false)
        && p.management_purpose.is_none()
        && !i.protected
        && i.primary == crate::ManagementAddressFamily::Ipv4
        && matches!(i.ipv4.mode, Ipv4Mode::None | Ipv4Mode::Static)
        && i.ipv4.valid()
        && i.ipv6.mode == Ipv6Mode::None
        && i.ipv6.addresses.is_empty()
        && i.ipv6.gateway.is_empty()
        && p.mac.eq_ignore_ascii_case("fe:ff:ff:ff:ff:ff")
}
impl ManagementBondOperation {
    /// The physical interface or bond master below the reviewed management endpoint.
    pub fn management_parent(source: &ManagementInterface) -> Option<PifId> {
        source
            .topology
            .as_ref()
            .and_then(|t| {
                t.vlans
                    .values()
                    .find(|v| v.untagged_pif == Some(source.pif))
            })
            .map_or(Some(source.pif), |v| v.tagged_pif)
    }
    pub(crate) fn captured_vlans(
        &self,
        source: &ManagementInterface,
    ) -> Option<BTreeMap<VlanId, CapturedVlan>> {
        let components = match self {
            Self::Create { members, .. } => members
                .iter()
                .filter_map(|m| m.topology.as_deref())
                .collect::<Vec<_>>(),
            Self::Destroy { .. } => vec![source.topology.as_deref()?],
        };
        let mut out = BTreeMap::new();
        let mut tags = BTreeSet::new();
        for t in components {
            for (id, vlan) in &t.vlans {
                if out.contains_key(id) || !tags.insert(vlan.tag?) || out.len() >= 128 {
                    return None;
                }
                let interface = t.pifs.get(&vlan.untagged_pif?)?.clone();
                if !supported_vlan(&interface) {
                    return None;
                }
                out.insert(
                    *id,
                    CapturedVlan {
                        vlan: vlan.clone(),
                        interface,
                    },
                );
            }
        }
        Some(out)
    }
    pub(crate) fn all_interfaces(&self, source: &ManagementInterface) -> Option<BTreeSet<PifId>> {
        Some(match self {
            Self::Create { members, .. } => members
                .iter()
                .flat_map(|m| {
                    m.topology
                        .as_ref()
                        .map(|t| t.pifs.keys().copied().collect::<Vec<_>>())
                        .unwrap_or(vec![m.pif])
                })
                .collect(),
            Self::Destroy { .. } => source.topology.as_ref()?.pifs.keys().copied().collect(),
        })
    }
    pub(crate) fn topology_valid(&self, source: &ManagementInterface) -> bool {
        let Some(parent) = Self::management_parent(source) else {
            return false;
        };
        let Some(vlans) = self.captured_vlans(source) else {
            return false;
        };
        if parent != source.pif
            && !vlans.values().any(|v| {
                v.interface.id == source.pif
                    && v.vlan.tagged_pif == Some(parent)
                    && source
                        .topology
                        .as_ref()
                        .is_some_and(|t| t.pifs.get(&source.pif) == Some(&v.interface))
            })
        {
            return false;
        }
        match self {
            Self::Create {
                members,
                network,
                mode,
                hash,
            } => {
                !network.uuid().is_nil()
                    && (2..=4).contains(&members.len())
                    && (*mode == BondMode::Lacp || hash.is_none())
                    && members.iter().map(|p| p.pif).collect::<BTreeSet<_>>().len() == members.len()
                    && members.iter().any(|p| p.pif == parent)
                    && (parent == source.pif || members[0].pif == parent)
                    && members.iter().all(|p| {
                        p.bounded()
                            && p.identity.host == source.identity.host
                            && p.identity.network != *network
                            && p.primary == source.primary
                            && !p.protected
                            && p.ipv6.mode == Ipv6Mode::None
                            && p.ipv6.addresses.is_empty()
                            && p.ipv6.gateway.is_empty()
                            && p.topology.as_ref().is_none_or(|t| {
                                t.bonds.is_empty()
                                    && t.pifs[&p.pif].physical == Some(true)
                                    && t.vlans.values().all(|v| v.tagged_pif == Some(p.pif))
                            })
                            && if p.pif == source.pif {
                                p == source
                            } else {
                                !p.identity.management && empty_ip(p)
                            }
                    })
                    && vlans.values().all(|v| {
                        v.interface.network != Some(*network)
                            && v.interface.management == Some(v.interface.id == source.pif)
                    })
            }
            Self::Destroy { bond, primary } => source.topology.as_ref().is_some_and(|t| {
                t.valid(source.pif)
                    && t.bonds.len() == 1
                    && t.bonds.get(bond).is_some_and(|b| {
                        b.master == Some(parent)
                            && b.primary_slave == Some(*primary)
                            && b.slaves.contains(primary)
                            && (2..=16).contains(&b.slaves.len())
                    })
                    && t.vlans.values().all(|v| v.tagged_pif == Some(parent))
                    && t.pifs.iter().all(|(id, p)| {
                        if *id == source.pif || vlans.values().any(|v| v.interface.id == *id) {
                            return p.management == Some(*id == source.pif);
                        }
                        p.management_purpose.is_none()
                            && p.management == Some(false)
                            && p.disallow_unplug == Some(false)
                            && (*id == parent || p.physical == Some(true))
                            && ManagementInterface::from_inventory(p).is_some_and(|i| empty_ip(&i))
                    })
            }),
        }
    }
}
