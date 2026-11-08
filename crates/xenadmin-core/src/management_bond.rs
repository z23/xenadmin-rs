use crate::{BondHash, BondId, BondMode, ManagementInterface, NetworkId, PifId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// A host-local management bond change. The host moves its existing static IPv4 settings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ManagementBondOperation {
    Create {
        members: Vec<ManagementInterface>,
        network: NetworkId,
        mode: BondMode,
        hash: Option<BondHash>,
    },
    Destroy {
        bond: BondId,
        primary: PifId,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagementBondCreated {
    pub bond: BondId,
    pub master: PifId,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Baseline {
    pub bonds: BTreeSet<BondId>,
    pub pifs: BTreeSet<PifId>,
    pub created: Option<ManagementBondCreated>,
    pub properties: std::collections::BTreeMap<String, String>,
}
impl Baseline {
    pub fn accepts(&self, next: &Self) -> bool {
        self.bonds == next.bonds
            && self.pifs == next.pifs
            && self.properties == next.properties
            && self.created.is_none_or(|v| next.created == Some(v))
    }
    pub fn valid_for(
        &self,
        source: &ManagementInterface,
        operation: &ManagementBondOperation,
    ) -> bool {
        self.valid()
            && self.pifs.contains(&source.pif)
            && match operation {
                ManagementBondOperation::Create { members, .. } => {
                    members.iter().all(|m| self.pifs.contains(&m.pif))
                }
                ManagementBondOperation::Destroy { bond, .. } => {
                    self.created.is_none()
                        && self.bonds.contains(bond)
                        && source
                            .topology
                            .as_ref()
                            .is_some_and(|t| t.pifs.keys().all(|p| self.pifs.contains(p)))
                }
            }
    }
    pub fn valid(&self) -> bool {
        self.properties.len() <= 64
            && self
                .properties
                .iter()
                .all(|(k, v)| k.len() <= 128 && v.len() <= 128)
            && self.bonds.len() <= 10_000
            && self.pifs.len() <= 10_000
            && self.bonds.iter().all(|v| !v.uuid().is_nil())
            && self.pifs.iter().all(|v| !v.uuid().is_nil())
            && self.created.is_none_or(|v| {
                !v.bond.uuid().is_nil()
                    && !v.master.uuid().is_nil()
                    && !self.bonds.contains(&v.bond)
                    && !self.pifs.contains(&v.master)
            })
    }
}
impl ManagementBondOperation {
    pub fn creates(&self) -> bool {
        matches!(self, Self::Create { .. })
    }
    pub(crate) fn valid(&self, source: &ManagementInterface) -> bool {
        if source.primary != crate::ManagementAddressFamily::Ipv4
            || source.ipv4.mode != crate::Ipv4Mode::Static
            || !source.ipv4.valid()
            || source.protected
            || source.ipv6.mode != crate::Ipv6Mode::None
            || !source.ipv6.addresses.is_empty()
            || !source.ipv6.gateway.is_empty()
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
                source.topology.is_none()
                    && !network.uuid().is_nil()
                    && (2..=4).contains(&members.len())
                    && (*mode == BondMode::Lacp || hash.is_none())
                    && members.iter().map(|p| p.pif).collect::<BTreeSet<_>>().len() == members.len()
                    && members.iter().filter(|p| p.pif == source.pif).count() == 1
                    && members.iter().all(|p| {
                        p.bounded()
                            && p.identity.host == source.identity.host
                            && p.identity.network != *network
                            && p.topology.is_none()
                            && p.primary == source.primary
                            && !p.protected
                            && p.ipv6.mode == crate::Ipv6Mode::None
                            && p.ipv6.addresses.is_empty()
                            && p.ipv6.gateway.is_empty()
                            && if p.pif == source.pif {
                                p == source
                            } else {
                                !p.identity.management
                                    && p.ipv4.mode == crate::Ipv4Mode::None
                                    && p.ipv4.valid()
                                    && p.ipv4.dns.is_empty()
                            }
                    })
            }
            Self::Destroy { bond, primary } => source.topology.as_ref().is_some_and(|t| {
                t.valid(source.pif)
                    && t.vlans.is_empty()
                    && t.bonds.len() == 1
                    && t.bonds.get(bond).is_some_and(|b| {
                        b.master == Some(source.pif)
                            && b.primary_slave == Some(*primary)
                            && b.slaves.contains(primary)
                            && (2..=16).contains(&b.slaves.len())
                    })
                    && t.pifs.iter().all(|(id, p)| {
                        *id == source.pif || {
                            p.physical == Some(true)
                                && p.management_purpose.is_none()
                                && p.disallow_unplug == Some(false)
                                && p.management == Some(false)
                                && p.ip_configuration_mode == "None"
                                && p.ip.is_empty()
                                && p.netmask.is_empty()
                                && p.gateway.is_empty()
                                && p.dns.is_empty()
                                && p.ipv6_configuration_mode == "None"
                                && p.ipv6.iter().all(String::is_empty)
                                && p.ipv6_gateway.is_empty()
                        }
                    })
            }),
        }
    }
    pub(crate) fn availability(
        &self,
        source: &ManagementInterface,
        i: &crate::Inventory,
    ) -> crate::ActionAvailability {
        use crate::{ActionAvailability as A, ClassAvailability as C};
        if ["pif", "bond", "vlan", "vif"]
            .iter()
            .any(|c| i.details.classes.get(*c) != Some(&C::Available))
        {
            return A::Unknown;
        }
        let members = match self {
            Self::Create {
                members, network, ..
            } => {
                if !i.networks.iter().any(|n| n.uuid == network.uuid()) {
                    return A::Stale;
                }
                if i.details.pifs.values().any(|p| p.network == Some(*network))
                    || i.details.vifs.values().any(|v| v.network == Some(*network))
                {
                    return A::Prerequisite {
                        reason:
                            "Choose an empty destination network with no physical or VM interfaces"
                                .into(),
                    };
                }
                members.iter().map(|p| p.pif).collect::<BTreeSet<_>>()
            }
            Self::Destroy { .. } => source
                .topology
                .as_ref()
                .unwrap()
                .pifs
                .keys()
                .copied()
                .collect(),
        };
        let networks = members
            .iter()
            .filter_map(|id| i.details.pifs.get(id)?.network)
            .collect::<BTreeSet<_>>();
        if i.details
            .vifs
            .values()
            .any(|v| v.network.is_some_and(|n| networks.contains(&n)))
        {
            return A::Prerequisite { reason: "This management bond workflow requires networks without VM interfaces; migration of guest consumers remains separate".into() };
        }
        A::Available
    }
}
