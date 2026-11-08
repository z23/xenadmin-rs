#[path = "management_bond_topology.rs"]
mod topology;
use crate::{BondHash, BondId, BondMode, ManagementInterface, NetworkId, PifId};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
pub(crate) use topology::CapturedVlan;

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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagementVlanReplacement {
    pub old_vlan: crate::VlanId,
    pub new_vlan: crate::VlanId,
    pub old_interface: PifId,
    pub new_interface: PifId,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Baseline {
    #[serde(default)]
    pub vlans: Option<BTreeSet<crate::VlanId>>,
    #[serde(default)]
    pub replacements: BTreeMap<crate::VlanId, ManagementVlanReplacement>,
    pub bonds: BTreeSet<BondId>,
    pub pifs: BTreeSet<PifId>,
    pub created: Option<ManagementBondCreated>,
    pub properties: std::collections::BTreeMap<String, String>,
}
impl Baseline {
    pub fn accepts(&self, next: &Self) -> bool {
        self.vlans == next.vlans
            && self
                .replacements
                .iter()
                .all(|(id, v)| next.replacements.get(id) == Some(v))
            && self.bonds == next.bonds
            && self.pifs == next.pifs
            && self.properties == next.properties
            && self.created.is_none_or(|v| next.created == Some(v))
    }
    pub fn merge(&mut self, other: &Self) -> Result<(), crate::Error> {
        if self.bonds != other.bonds
            || self.pifs != other.pifs
            || self.vlans != other.vlans
            || self.properties != other.properties
            || self.created.zip(other.created).is_some_and(|(a, b)| a != b)
            || other
                .replacements
                .iter()
                .any(|(id, v)| self.replacements.get(id).is_some_and(|old| old != v))
        {
            return Err(crate::Error::JournalChanged);
        }
        if self.created.is_none() {
            self.created = other.created;
        }
        self.replacements
            .extend(other.replacements.iter().map(|(id, v)| (*id, *v)));
        Ok(())
    }
    pub fn valid_for(
        &self,
        source: &ManagementInterface,
        operation: &ManagementBondOperation,
    ) -> bool {
        let Some(expected) = operation.raw_vlans(source) else {
            return false;
        };
        self.valid()
            && (expected.is_empty()
                || self
                    .vlans
                    .as_ref()
                    .is_some_and(|ids| expected.keys().all(|id| ids.contains(id))))
            && expected
                .values()
                .all(|v| self.pifs.contains(&v.interface.id))
            && self.replacements.iter().all(|(id, r)| {
                expected
                    .get(id)
                    .is_some_and(|v| r.old_vlan == *id && r.old_interface == v.interface.id)
            })
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
        self.vlans
            .as_ref()
            .is_none_or(|v| v.len() <= 10_000 && v.iter().all(|id| !id.uuid().is_nil()))
            && self.replacements.len() <= 128
            && self
                .replacements
                .values()
                .map(|v| v.new_vlan)
                .collect::<BTreeSet<_>>()
                .len()
                == self.replacements.len()
            && self
                .replacements
                .values()
                .map(|v| v.new_interface)
                .collect::<BTreeSet<_>>()
                .len()
                == self.replacements.len()
            && self.replacements.values().all(|v| {
                !v.new_vlan.uuid().is_nil()
                    && !v.new_interface.uuid().is_nil()
                    && self
                        .vlans
                        .as_ref()
                        .is_some_and(|ids| ids.contains(&v.old_vlan) && !ids.contains(&v.new_vlan))
                    && self.pifs.contains(&v.old_interface)
                    && !self.pifs.contains(&v.new_interface)
                    && self
                        .created
                        .is_none_or(|parent| parent.master != v.new_interface)
            })
            && self.properties.len() <= 64
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
        self.topology_valid(source)
    }
    pub(crate) fn availability(
        &self,
        source: &ManagementInterface,
        i: &crate::Inventory,
        consumers: Option<&crate::ManagementBondConsumers>,
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
                members
                    .iter()
                    .flat_map(|m| {
                        m.topology
                            .as_ref()
                            .map(|t| t.pifs.keys().copied().collect::<Vec<_>>())
                            .unwrap_or(vec![m.pif])
                    })
                    .collect::<BTreeSet<_>>()
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
        if consumers.is_none()
            && i.details
                .vifs
                .values()
                .any(|v| v.network.is_some_and(|n| networks.contains(&n)))
        {
            return A::Prerequisite {
                reason: "Read and review the guest interface migration before changing this bond"
                    .into(),
            };
        }
        if let Some(consumers) = consumers {
            let Some((networks, _, _)) = crate::management_bond_consumers::scope(source, self)
            else {
                return A::Unknown;
            };
            let actual = i
                .details
                .vifs
                .values()
                .filter(|v| v.network.is_some_and(|n| networks.contains(&n)))
                .collect::<Vec<_>>();
            if actual.len() != consumers.interfaces.len()
                || actual.iter().any(|v| {
                    consumers.interfaces.get(&v.id).is_none_or(|old| {
                        old.vm != v.vm.unwrap_or(crate::VmId::from(uuid::Uuid::nil()))
                            || Some(old.network) != v.network
                            || old.device != v.device
                            || !old.mac.eq_ignore_ascii_case(&v.mac)
                            || Some(old.mtu) != v.mtu
                            || Some(old.attached) != v.currently_attached
                    })
                })
            {
                return A::Stale;
            }
            for (id, expected) in &consumers.networks {
                if i.details
                    .networks
                    .get(id)
                    .and_then(|n| n.default_locking_mode.as_ref())
                    .is_some_and(|mode| Some(mode) != expected.default_locking_mode.as_ref())
                {
                    return A::Stale;
                }
            }
            for old in consumers.interfaces.values() {
                if let Some(vm) = i.vms.get(&old.vm) {
                    let resident = match old.locality {
                        crate::ManagementVmLocality::Resident(host) => Some(host),
                        _ => None,
                    };
                    if vm.power_state.to_string() != old.power_state || vm.host != resident {
                        return A::Stale;
                    }
                }
            }
        }
        A::Available
    }
}
