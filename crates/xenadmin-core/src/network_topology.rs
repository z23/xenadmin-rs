use crate::{
    ActionAvailability, ClassAvailability, Error, HostId, Inventory, NetworkId, PifId, PifIdentity,
    ResourceTarget, VlanId,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use xenadmin_xapi::enumerations::{ip_configuration_mode, ipv6_configuration_mode};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VlanUplink {
    pub pif: PifId,
    pub expected: PifIdentity,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewedVlan {
    pub vlan: VlanId,
    pub child: PifId,
    pub uplink: VlanUplink,
    pub tag: u16,
    pub attached: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NetworkTopologyOperation {
    Fabric(Box<crate::NetworkFabricPool>),
    CreateVlans {
        uplinks: Vec<VlanUplink>,
        tag: u16,
    },
    RetagVlans {
        interfaces: Vec<ReviewedVlan>,
        tag: u16,
    },
}
impl NetworkTopologyOperation {
    pub fn permission_requirements(&self) -> crate::PermissionRequirements {
        if let Self::Fabric(op) = self {
            return op.permission_requirements();
        }
        let mut methods = vec!["VLAN.create"];
        if let Self::RetagVlans { interfaces, .. } = self {
            methods.push("VLAN.destroy");
            if interfaces.iter().any(|i| !i.attached) {
                methods.push("PIF.unplug");
            }
        }
        crate::PermissionRequirements::writes(methods)
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Fabric(op) => op.label(),
            Self::CreateVlans { .. } => "Create pool-wide VLAN interfaces",
            Self::RetagVlans { .. } => "Retag pool-wide VLAN interfaces",
        }
    }
    pub(crate) fn uplinks(&self) -> Vec<&VlanUplink> {
        match self {
            Self::Fabric(_) => vec![],
            Self::CreateVlans { uplinks, .. } => uplinks.iter().collect(),
            Self::RetagVlans { interfaces, .. } => interfaces.iter().map(|v| &v.uplink).collect(),
        }
    }
    pub(crate) fn tag(&self) -> u16 {
        match self {
            Self::Fabric(_) => 0,
            Self::CreateVlans { tag, .. } | Self::RetagVlans { tag, .. } => *tag,
        }
    }
    pub(crate) fn validate(&self, target: ResourceTarget) -> Result<(), Error> {
        if let Self::Fabric(op) = self {
            return op.validate(target);
        }
        let ResourceTarget::Network(network) = target else {
            return Err(Error::InvalidResourceCommand);
        };
        let uplinks = self.uplinks();
        let hosts = uplinks
            .iter()
            .map(|p| p.expected.host)
            .collect::<BTreeSet<_>>();
        let pifs = uplinks.iter().map(|p| p.pif).collect::<BTreeSet<_>>();
        if uplinks.is_empty()
            || uplinks.len() > 128
            || hosts.len() != uplinks.len()
            || pifs.len() != uplinks.len()
            || self.tag() > 4094
            || uplinks.iter().any(|p| {
                p.expected.network == network
                    || p.expected.network != uplinks[0].expected.network
                    || p.expected.device.is_empty()
                    || p.expected.device.len() > 256
                    || p.expected.mac.len() > 64
            })
        {
            return Err(Error::InvalidResourceCommand);
        }
        if let Self::RetagVlans { interfaces, tag } = self
            && (interfaces
                .iter()
                .any(|v| v.tag > 4094 || v.tag == *tag || v.child == v.uplink.pif)
                || interfaces
                    .iter()
                    .map(|v| v.vlan)
                    .collect::<BTreeSet<_>>()
                    .len()
                    != interfaces.len()
                || interfaces
                    .iter()
                    .map(|v| v.child)
                    .collect::<BTreeSet<_>>()
                    .len()
                    != interfaces.len())
        {
            return Err(Error::InvalidResourceCommand);
        }
        Ok(())
    }
    pub fn availability(&self, inventory: &Inventory, network: NetworkId) -> ActionAvailability {
        if let Self::Fabric(op) = self {
            return match op
                .validate(ResourceTarget::Network(network))
                .and_then(|_| op.availability(inventory))
            {
                Ok(()) => ActionAvailability::Available,
                Err(e) => ActionAvailability::Prerequisite {
                    reason: e.to_string(),
                },
            };
        }
        if self.validate(ResourceTarget::Network(network)).is_err() {
            return ActionAvailability::Prerequisite {
                reason:
                    "Choose one distinct uplink per pool host and a changed VLAN tag from 0 to 4094"
                        .into(),
            };
        }
        if ["pif", "vif", "vlan"].iter().any(|class| {
            !matches!(
                inventory.details.classes.get(*class),
                Some(ClassAvailability::Available)
            )
        }) {
            return ActionAvailability::Unknown;
        }
        if !inventory.networks.iter().any(|n| n.uuid == network.uuid()) {
            return ActionAvailability::Stale;
        }
        let uplinks = self.uplinks();
        if uplinks
            .iter()
            .map(|p| p.expected.host)
            .collect::<BTreeSet<HostId>>()
            != inventory.hosts.keys().copied().collect()
        {
            return ActionAvailability::Prerequisite {
                reason: "Review exactly one uplink for every current pool host".into(),
            };
        }
        if uplinks.iter().any(|p| {
            inventory.details.pifs.get(&p.pif).is_none_or(|current| {
                PifIdentity::from_inventory(current).as_ref() != Some(&p.expected)
                    || current.vlan != Some(-1)
                    || current.managed != Some(true)
            })
        }) {
            return ActionAvailability::Stale;
        }
        if inventory
            .details
            .vifs
            .values()
            .any(|v| v.network == Some(network))
        {
            return ActionAvailability::Prerequisite {
                reason: "Move all VM interfaces off this network before changing its VLAN topology"
                    .into(),
            };
        }
        let members = inventory
            .details
            .pifs
            .values()
            .filter(|p| p.network == Some(network))
            .map(|p| p.id)
            .collect::<BTreeSet<_>>();
        match self {
            Self::CreateVlans { .. } if !members.is_empty() => {
                return ActionAvailability::Prerequisite {
                    reason: "Choose a network with no host interfaces, or review retagging its existing VLAN interfaces".into(),
                };
            }
            Self::RetagVlans { interfaces, .. } => {
                if members != interfaces.iter().map(|v| v.child).collect() {
                    return ActionAvailability::Stale;
                }
                for vlan in interfaces {
                    if inventory.details.vlans.get(&vlan.vlan).is_none_or(|v| {
                        v.tag != Some(u64::from(vlan.tag))
                            || v.tagged_pif != Some(vlan.uplink.pif)
                            || v.untagged_pif != Some(vlan.child)
                    }) {
                        return ActionAvailability::Stale;
                    }
                    if inventory.details.pifs.get(&vlan.child).is_none_or(|p| {
                        p.currently_attached != Some(vlan.attached)
                            || p.management != Some(false)
                            || p.managed != Some(true)
                            || p.disallow_unplug != Some(false)
                            || p.management_purpose.is_some()
                            || p.ip_configuration_mode != ip_configuration_mode::NONE
                            || p.ipv6_configuration_mode != ipv6_configuration_mode::NONE
                            || !p.ip.is_empty()
                            || !p.ipv6.is_empty()
                    }) {
                        return ActionAvailability::Prerequisite { reason: "Retagging requires unprotected VLAN interfaces with no management, storage or other IP configuration".into() };
                    }
                }
            }
            _ => {}
        }
        ActionAvailability::Available
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkTopologyChanges {
    #[serde(default)]
    pub created_fabric: Vec<crate::NetworkFabricEvidence>,
    #[serde(default)]
    pub removed_fabric: Vec<crate::NetworkFabricEvidence>,
    pub created_vlans: Vec<VlanId>,
    pub created_pifs: Vec<PifId>,
    pub removed_vlans: Vec<VlanId>,
    pub removed_pifs: Vec<PifId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Kind {
    Fabric(Box<crate::NetworkFabricOperation>),
    Create {
        uplink: VlanUplink,
        tag: u16,
    },
    Destroy(ReviewedVlan),
    DetachCreated {
        create_step: usize,
        uplink: VlanUplink,
    },
}
impl Kind {
    pub(crate) fn creates(&self) -> bool {
        matches!(self, Self::Create { .. }) || matches!(self, Self::Fabric(op) if op.creates())
    }
    pub(crate) fn label(&self) -> String {
        match self {
            Self::Fabric(op) => format!(
                "{} on {} ({})",
                op.label(),
                op.review.input.device,
                op.review.host
            ),
            Self::Create { uplink, tag } => format!(
                "Create VLAN {tag} on {} ({})",
                uplink.expected.device, uplink.expected.host
            ),
            Self::DetachCreated { uplink, .. } => format!(
                "Restore unplugged VLAN state on {} ({})",
                uplink.expected.device, uplink.expected.host
            ),
            Self::Destroy(vlan) => format!(
                "Remove VLAN {} on {} ({})",
                vlan.tag, vlan.uplink.expected.device, vlan.uplink.expected.host
            ),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Receipt {
    pub kind: Kind,
    pub task: Option<String>,
    pub task_uuid: Option<crate::TaskId>,
    pub result: Option<String>,
    pub completed: bool,
    pub ended: bool,
    pub cancel_sent: bool,
    pub created: Option<(VlanId, PifId)>,
    #[serde(default)]
    pub fabric_created: Option<crate::NetworkFabricEvidence>,
    #[serde(default)]
    pub fabric_member: Option<crate::NetworkFabricMember>,
    pub verified: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Plan {
    pub pool: crate::PoolId,
    pub network: NetworkId,
    pub operation: NetworkTopologyOperation,
    pub vlans_before: BTreeSet<VlanId>,
    pub pifs_before: BTreeSet<PifId>,
    pub tasks_before: BTreeSet<crate::TaskId>,
    pub steps: Vec<Receipt>,
    pub expires_at_ms: u64,
    #[serde(default)]
    pub fabric_before: Option<BTreeSet<crate::NetworkFabricObject>>,
}
impl Plan {
    pub(crate) fn accepts(&self, next: &Self) -> bool {
        self.pool == next.pool
            && self.network == next.network
            && self.operation == next.operation
            && self.vlans_before == next.vlans_before
            && self.pifs_before == next.pifs_before
            && self.tasks_before == next.tasks_before
            && self.expires_at_ms == next.expires_at_ms
            && self.fabric_before == next.fabric_before
            && self.steps.len() == next.steps.len()
            && self.steps.iter().zip(&next.steps).all(|(old, new)| {
                old.kind == new.kind
                    && old
                        .task
                        .as_ref()
                        .is_none_or(|v| Some(v) == new.task.as_ref())
                    && old.task_uuid.is_none_or(|v| Some(v) == new.task_uuid)
                    && old
                        .result
                        .as_ref()
                        .is_none_or(|v| Some(v) == new.result.as_ref())
                    && old.created.is_none_or(|v| Some(v) == new.created)
                    && old
                        .fabric_created
                        .as_ref()
                        .is_none_or(|v| Some(v) == new.fabric_created.as_ref())
                    && old
                        .fabric_member
                        .as_ref()
                        .is_none_or(|v| Some(v) == new.fabric_member.as_ref())
                    && (!old.completed || new.completed)
                    && (!old.ended || new.ended)
                    && (!old.cancel_sent || new.cancel_sent)
                    && (!old.verified || new.verified)
            })
    }
    pub(crate) fn changes(&self) -> NetworkTopologyChanges {
        let mut result = NetworkTopologyChanges::default();
        for step in &self.steps {
            if let Some(e) = &step.fabric_created {
                result.created_fabric.push(e.clone());
            }
            if step.verified
                && let Kind::Fabric(op) = &step.kind
                && let crate::NetworkFabricChange::Delete(object) = op.change
                && let Some(m) = op.review.members.values().find(|m| m.object == object)
            {
                result.removed_fabric.push(crate::NetworkFabricEvidence {
                    object,
                    interface: m.interface.id,
                    host: op.review.host,
                    network: op.review.network,
                });
            }
            if let Some((vlan, pif)) = step.created {
                result.created_vlans.push(vlan);
                result.created_pifs.push(pif);
            }
            if step.verified
                && let Kind::Destroy(vlan) = &step.kind
            {
                result.removed_vlans.push(vlan.vlan);
                result.removed_pifs.push(vlan.child);
            }
        }
        result
    }
}
