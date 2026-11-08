use crate::{
    ActionAvailability, Error, Inventory, Ipv4Mode, Ipv6Mode, ManagementAddressFamily,
    ManagementBondConsumers, ManagementBondOperation, ManagementInterface, ManagementTopology,
    OperationId, PifId, ResourceTarget,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ManagementIpv6Next {
    #[default]
    Continue,
    LeavePartial,
}
/// A staged IPv6 bond change. A continuation names one immutable, verified predecessor.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagementIpv6Bond {
    #[serde(default)]
    pub purpose: ManagementIpv6Next,
    pub source: ManagementInterface,
    pub temporary: ManagementInterface,
    pub operation: ManagementBondOperation,
    pub consumers: Option<Box<ManagementBondConsumers>>,
    pub predecessor: Option<OperationId>,
}
impl ManagementIpv6Bond {
    pub fn label(&self) -> &'static str {
        if self.purpose == ManagementIpv6Next::LeavePartial {
            "Leave partial IPv6 configuration"
        } else if self.predecessor.is_some() {
            "Continue IPv6 management bond handover"
        } else if self.operation.creates() {
            "Create IPv6 management bond"
        } else {
            "Remove IPv6 management bond"
        }
    }
    pub(crate) fn validate(&self, target: ResourceTarget) -> Result<(), Error> {
        let source = &self.source;
        let temporary = &self.temporary;
        if self.purpose == ManagementIpv6Next::LeavePartial && self.predecessor.is_none()
            || target != ResourceTarget::Host(source.identity.host)
            || !static_ipv6(source)
            || !source.identity.management
            || !static_ipv6(temporary)
            || temporary.identity.management
            || source.identity.host != temporary.identity.host
            || source.address() == temporary.address()
            || source.identity.network == temporary.identity.network
            || !self.shape_valid()
            || self
                .consumers
                .as_ref()
                .is_some_and(|v| !v.valid(source, &self.operation))
        {
            return Err(Error::InvalidResourceCommand);
        }
        let affected = self.interfaces().ok_or(Error::InvalidResourceCommand)?;
        let temporary_ids = temporary
            .topology
            .as_ref()
            .map(|t| t.pifs.keys().copied().collect::<BTreeSet<_>>())
            .unwrap_or(BTreeSet::from([temporary.pif]));
        let temporary_networks = temporary
            .topology
            .as_ref()
            .map(|t| {
                t.pifs
                    .values()
                    .filter_map(|p| p.network)
                    .collect::<BTreeSet<_>>()
            })
            .unwrap_or(BTreeSet::from([temporary.identity.network]));
        let (networks, _, _) = crate::management_bond_consumers::scope(source, &self.operation)
            .ok_or(Error::InvalidResourceCommand)?;
        if !affected.is_disjoint(&temporary_ids) || !networks.is_disjoint(&temporary_networks) {
            return Err(Error::InvalidResourceCommand);
        }
        Ok(())
    }
    pub(crate) fn interfaces(&self) -> Option<BTreeSet<PifId>> {
        self.operation.all_interfaces(&self.source)
    }
    fn shape_valid(&self) -> bool {
        let Some(parent) = ManagementBondOperation::management_parent(&self.source) else {
            return false;
        };
        let Some(vlans) = self.operation.raw_vlans(&self.source) else {
            return false;
        };
        if vlans.values().any(|v| {
            let Some(p) = ManagementInterface::from_inventory(&v.interface) else {
                return true;
            };
            v.interface.physical != Some(false)
                || v.interface.management_purpose.is_some()
                || p.protected
                || p.primary != ManagementAddressFamily::Ipv6
                || !v.interface.mac.eq_ignore_ascii_case("fe:ff:ff:ff:ff:ff")
                || if p.pif == self.source.pif {
                    p != without_topology(&self.source)
                } else {
                    p.identity.management
                        || p.ipv6.mode != Ipv6Mode::None
                        || !p.ipv6.addresses.is_empty()
                        || !p.ipv6.gateway.is_empty()
                        || !matches!(p.ipv4.mode, Ipv4Mode::None | Ipv4Mode::Static)
                        || !p.ipv4.valid()
                }
        }) {
            return false;
        }
        if parent != self.source.pif
            && !vlans
                .values()
                .any(|v| v.interface.id == self.source.pif && v.vlan.tagged_pif == Some(parent))
        {
            return false;
        }
        match &self.operation {
            ManagementBondOperation::Create {
                members,
                network,
                mode,
                hash,
            } => {
                !network.uuid().is_nil()
                    && (2..=4).contains(&members.len())
                    && ((*mode == crate::BondMode::Lacp) == hash.is_some())
                    && members.first().is_some_and(|p| p.pif == parent)
                    && members.iter().map(|p| p.pif).collect::<BTreeSet<_>>().len() == members.len()
                    && members.iter().all(|p| {
                        p.bounded()
                            && p.primary == ManagementAddressFamily::Ipv6
                            && p.identity.host == self.source.identity.host
                            && p.identity.network != *network
                            && !p.protected
                            && p.topology.as_ref().is_none_or(|t| {
                                t.bonds.is_empty()
                                    && t.pifs[&p.pif].physical == Some(true)
                                    && t.vlans.values().all(|v| v.tagged_pif == Some(p.pif))
                            })
                            && if p.pif == self.source.pif {
                                p == &self.source
                            } else {
                                !p.identity.management && no_ip(p)
                            }
                    })
                    && vlans
                        .values()
                        .all(|v| v.interface.network != Some(*network))
            }
            ManagementBondOperation::Destroy { bond, primary } => {
                self.source.topology.as_ref().is_some_and(|t| {
                    t.valid(self.source.pif)
                        && t.bonds.len() == 1
                        && t.bonds.get(bond).is_some_and(|b| {
                            b.master == Some(parent)
                                && b.primary_slave == Some(*primary)
                                && b.slaves.contains(primary)
                                && (2..=16).contains(&b.slaves.len())
                        })
                        && t.vlans.values().all(|v| v.tagged_pif == Some(parent))
                        && t.pifs.iter().all(|(id, p)| {
                            if *id == self.source.pif
                                || vlans.values().any(|v| v.interface.id == *id)
                            {
                                return p.management == Some(*id == self.source.pif);
                            }
                            p.management_purpose.is_none()
                                && p.management == Some(false)
                                && p.disallow_unplug == Some(false)
                                && (*id == parent || p.physical == Some(true))
                                && ManagementInterface::from_inventory(p).is_some_and(|p| {
                                    p.primary == ManagementAddressFamily::Ipv6 && no_ip(&p)
                                })
                        })
                })
            }
        }
    }
    pub fn availability(&self, inventory: &Inventory) -> ActionAvailability {
        if self
            .validate(ResourceTarget::Host(self.source.identity.host))
            .is_err()
        {
            return ActionAvailability::Prerequisite{reason:"Review one static IPv6 management address, a separate reachable static IPv6 temporary management interface and exact ordinary bond/VLAN topology. Members must use the same IPv6 primary family; other affected VLANs may use static IPv4 or no IP.".into()};
        }
        if self.purpose == ManagementIpv6Next::LeavePartial {
            return if inventory.hosts.contains_key(&self.source.identity.host) {
                ActionAvailability::Available
            } else {
                ActionAvailability::Stale
            };
        }
        if inventory.details.pool.as_ref().and_then(|p| p.ha_enabled) != Some(false) {
            return ActionAvailability::Prerequisite {
                reason:
                    "Disable HA and verify host clustering before this staged management handover"
                        .into(),
            };
        }
        if !inventory.hosts.contains_key(&self.source.identity.host) {
            return ActionAvailability::Stale;
        }
        if self.predecessor.is_none()
            && [&self.source, &self.temporary]
                .iter()
                .any(|p| ManagementInterface::capture(inventory, p.pif).as_ref() != Some(*p))
        {
            return ActionAvailability::Stale;
        }
        if self.predecessor.is_none() {
            let members = match &self.operation {
                ManagementBondOperation::Create { members, .. } => {
                    members.iter().collect::<Vec<_>>()
                }
                _ => vec![&self.source],
            };
            if members
                .iter()
                .any(|p| ManagementInterface::capture(inventory, p.pif).as_ref() != Some(*p))
            {
                return ActionAvailability::Stale;
            }
            let a = self
                .operation
                .availability(&self.source, inventory, self.consumers.as_deref());
            if !a.is_available() {
                return a;
            }
        }
        ActionAvailability::Available
    }
}
fn static_ipv6(p: &ManagementInterface) -> bool {
    p.valid()
        && p.primary == ManagementAddressFamily::Ipv6
        && p.ipv6.mode == Ipv6Mode::Static
        && p.ipv6.valid()
        && p.attached
        && !p.protected
        && p.ipv4.mode == Ipv4Mode::None
        && p.ipv4.valid()
}
fn no_ip(p: &ManagementInterface) -> bool {
    p.ipv4.mode == Ipv4Mode::None
        && p.ipv4.valid()
        && p.ipv4.dns.is_empty()
        && p.ipv6.mode == Ipv6Mode::None
        && p.ipv6.addresses.is_empty()
        && p.ipv6.gateway.is_empty()
}
fn without_topology(p: &ManagementInterface) -> ManagementInterface {
    let mut p = p.clone();
    p.topology = None;
    p
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Stage {
    MoveToTemporary,
    ClearSourceIpv6,
    ChangeBond,
    RestoreTargetIpv6,
    MoveToTarget,
}
impl Stage {
    pub const ALL: [Self; 5] = [
        Self::MoveToTemporary,
        Self::ClearSourceIpv6,
        Self::ChangeBond,
        Self::RestoreTargetIpv6,
        Self::MoveToTarget,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::MoveToTemporary => "Move management to the verified temporary interface",
            Self::ClearSourceIpv6 => "Clear IPv6 from the original non-management interface",
            Self::ChangeBond => "Change the reviewed bond and VLAN topology",
            Self::RestoreTargetIpv6 => "Restore IPv6 on the proven target interface",
            Self::MoveToTarget => "Return management to the verified target",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Receipt {
    pub stage: Stage,
    pub task_baseline: BTreeSet<crate::TaskId>,
    pub dispatched: bool,
    pub task: Option<String>,
    pub task_uuid: Option<crate::TaskId>,
    pub result: Option<String>,
    pub completed: bool,
    pub ended: bool,
    pub verified: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Plan {
    pub version: u8,
    pub pool: crate::PoolId,
    pub host: crate::HostId,
    pub origin: OperationId,
    pub predecessor: Option<OperationId>,
    pub successor: Option<OperationId>,
    pub depth: u8,
    pub lineage: u8,
    pub intent: ManagementIpv6Bond,
    pub original_endpoint: crate::Endpoint,
    pub temporary_endpoint: crate::Endpoint,
    pub final_endpoint: crate::Endpoint,
    pub certificate: crate::CertificateFingerprint,
    pub registered: bool,
    pub temporary_route_verified: bool,
    pub final_route_verified: bool,
    pub expires_at_ms: u64,
    pub receipts: Vec<Receipt>,
    pub baseline: crate::management_bond::Baseline,
    pub original_components: Vec<ManagementTopology>,
    pub temporary_component: ManagementTopology,
    pub change_baseline: Option<crate::management_bond::Baseline>,
    pub temporary_properties: BTreeMap<PifId, BTreeMap<String, String>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagementIpv6BondEvidence {
    pub left_partial: bool,
    pub completed_stages: usize,
    pub retains_reservation: bool,
    pub temporary_endpoint: crate::Endpoint,
    pub target_endpoint: crate::Endpoint,
    pub predecessor: Option<OperationId>,
    pub successor: Option<OperationId>,
    pub created_bond: Option<crate::ManagementBondCreated>,
    pub moved_vlans: Vec<crate::ManagementVlanReplacement>,
}
impl Receipt {
    pub fn pending(stage: Stage) -> Self {
        Self {
            stage,
            task_baseline: BTreeSet::new(),
            dispatched: false,
            task: None,
            task_uuid: None,
            result: None,
            completed: false,
            ended: false,
            verified: false,
        }
    }
    fn accepts(&self, n: &Self) -> bool {
        self.stage == n.stage
            && ((!self.dispatched && self.task_baseline.is_empty())
                || self.task_baseline == n.task_baseline)
            && (!self.dispatched || n.dispatched)
            && self
                .task
                .as_ref()
                .is_none_or(|v| n.task.as_ref() == Some(v))
            && self.task_uuid.is_none_or(|v| n.task_uuid == Some(v))
            && self
                .result
                .as_ref()
                .is_none_or(|v| n.result.as_ref() == Some(v))
            && (!self.completed || n.completed)
            && (!self.ended || n.ended)
            && (!self.verified || n.verified)
    }
}
impl Plan {
    pub fn evidence(&self, state: &crate::OperationState) -> ManagementIpv6BondEvidence {
        ManagementIpv6BondEvidence {
            left_partial: self.intent.purpose == ManagementIpv6Next::LeavePartial,
            retains_reservation: self.intent.purpose != ManagementIpv6Next::LeavePartial
                && *state == crate::OperationState::PartiallyCompleted
                && self.successor.is_none()
                && !(self.depth == 4 && self.current().verified)
                && self.receipts.iter().any(|v| v.verified),
            completed_stages: self.receipts.iter().filter(|v| v.verified).count(),
            temporary_endpoint: self.temporary_endpoint.clone(),
            target_endpoint: self.final_endpoint.clone(),
            predecessor: self.predecessor,
            successor: self.successor,
            created_bond: self.change_baseline.as_ref().and_then(|b| b.created),
            moved_vlans: self
                .change_baseline
                .as_ref()
                .map(|b| b.replacements.values().copied().collect())
                .unwrap_or_default(),
        }
    }
    pub fn stage(&self) -> Stage {
        Stage::ALL[self.depth as usize]
    }
    pub fn current(&self) -> &Receipt {
        &self.receipts[self.depth as usize]
    }
    pub fn current_mut(&mut self) -> &mut Receipt {
        &mut self.receipts[self.depth as usize]
    }
    pub fn accepts(&self, n: &Self) -> bool {
        self.version == n.version
            && self.pool == n.pool
            && self.host == n.host
            && self.origin == n.origin
            && self.predecessor == n.predecessor
            && self.successor.is_none_or(|v| n.successor == Some(v))
            && self.depth == n.depth
            && self.lineage == n.lineage
            && self.intent == n.intent
            && self.original_endpoint == n.original_endpoint
            && self.temporary_endpoint == n.temporary_endpoint
            && self.final_endpoint == n.final_endpoint
            && self.certificate == n.certificate
            && self.expires_at_ms == n.expires_at_ms
            && self.baseline == n.baseline
            && self.original_components == n.original_components
            && self.temporary_component == n.temporary_component
            && self.temporary_properties == n.temporary_properties
            && self
                .change_baseline
                .as_ref()
                .is_none_or(|b| n.change_baseline.as_ref().is_some_and(|v| b.accepts(v)))
            && (!self.registered || n.registered)
            && (!self.temporary_route_verified || n.temporary_route_verified)
            && (!self.final_route_verified || n.final_route_verified)
            && self.receipts.len() == n.receipts.len()
            && self
                .receipts
                .iter()
                .zip(&n.receipts)
                .all(|(a, b)| a.accepts(b))
    }
    pub fn project(&self, r: &mut crate::ResourceOperationRecord) {
        r.management_ipv6_bond = Some(self.evidence(&r.state));
        r.steps = Stage::ALL
            .iter()
            .enumerate()
            .map(|(i, s)| crate::OperationStep {
                label: s.label().into(),
                state: if self.receipts.get(i).is_some_and(|r| r.verified) {
                    crate::StepState::Verified
                } else if self.receipts.get(i).is_some_and(|r| r.dispatched) {
                    crate::StepState::Dispatching
                } else {
                    crate::StepState::Pending
                },
                detail: String::new(),
            })
            .collect();
    }
}
pub(crate) fn endpoint(
    base: &crate::Endpoint,
    address: std::net::IpAddr,
) -> Result<crate::Endpoint, Error> {
    let mut u = url::Url::parse(base.as_str()).map_err(|_| Error::InvalidEndpoint)?;
    u.set_ip_host(address).map_err(|_| Error::InvalidEndpoint)?;
    crate::Endpoint::parse(u.as_str())
}
pub(crate) fn validate_record(r: &crate::ResourceOperationRecord) -> Result<(), Error> {
    let crate::ResourceAction::ManagementIpv6Bond(op) = &r.action else {
        return if r.management_ipv6_bond_plan.is_none() && r.management_ipv6_bond.is_none() {
            Ok(())
        } else {
            Err(Error::Journal)
        };
    };
    op.validate(r.target)?;
    let Some(p) = &r.management_ipv6_bond_plan else {
        return if r.management_ipv6_bond.is_none()
            && r.steps.is_empty()
            && matches!(
                r.state,
                crate::OperationState::Preparing
                    | crate::OperationState::Rejected
                    | crate::OperationState::Cancelled
                    | crate::OperationState::OutcomeUnknown
                    | crate::OperationState::AcknowledgedUnknown
            ) {
            Ok(())
        } else {
            Err(Error::Journal)
        };
    };
    if p.version != 1
        || p.pool != r.pool
        || p.host != op.source.identity.host
        || &p.intent != op.as_ref()
        || p.predecessor != op.predecessor
        || p.depth > 4
        || p.lineage > 16
        || p.receipts.len() != p.depth as usize + 1
        || !p.original_endpoint.as_str().starts_with("https://")
        || p.temporary_endpoint
            != endpoint(
                &p.original_endpoint,
                op.temporary.address().ok_or(Error::Journal)?,
            )?
        || p.final_endpoint
            != endpoint(
                &p.original_endpoint,
                op.source.address().ok_or(Error::Journal)?,
            )?
        || p.expires_at_ms < r.created_at_ms
        || p.expires_at_ms > r.created_at_ms.saturating_add(30 * 60 * 1000)
        || !p.baseline.valid_for(&op.source, &op.operation)
        || p.change_baseline
            .as_ref()
            .is_some_and(|b| !b.valid_for(&op.source, &op.operation))
        || p.depth > 2 && p.change_baseline.is_none()
        || (p.lineage == 0) != (p.predecessor.is_none())
        || p.lineage == 0 && p.origin != r.id
        || p.origin.to_string() == uuid::Uuid::nil().to_string()
        || p.successor == Some(r.id)
        || p.successor.is_some() && !matches!(r.state, crate::OperationState::PartiallyCompleted)
        || r.management_ipv6_bond.as_ref() != Some(&p.evidence(&r.state))
        || r.steps.len() != 5
    {
        return Err(Error::Journal);
    }
    let roots = match &op.operation {
        ManagementBondOperation::Create { members, .. } => members.iter().collect::<Vec<_>>(),
        ManagementBondOperation::Destroy { .. } => vec![&op.source],
    };
    let matches = |t: &ManagementTopology, root: &ManagementInterface| {
        t.valid(root.pif)
            && t.matches_root(root)
            && root
                .topology
                .as_deref()
                .map_or(t.bonds.is_empty() && t.vlans.is_empty(), |v| v == t)
    };
    if p.original_components.iter().any(|t| {
        t.pifs
            .values()
            .any(|p| p.disallow_unplug != Some(false) || p.management_purpose.is_some())
    }) || p.original_components.len() != roots.len()
        || !p
            .original_components
            .iter()
            .zip(roots)
            .all(|(t, root)| matches(t, root))
        || !matches(&p.temporary_component, &op.temporary)
        || p.temporary_component
            .pifs
            .values()
            .any(|p| p.disallow_unplug != Some(false) || p.management_purpose.is_some())
        || p.temporary_properties.len() != p.temporary_component.pifs.len()
        || !p.temporary_properties.iter().all(|(id, m)| {
            p.temporary_component.pifs.contains_key(id)
                && m.len() <= 64
                && m.iter().all(|(k, v)| k.len() <= 128 && v.len() <= 128)
        })
        || p.depth < 2 && p.change_baseline.is_some()
        || p.depth == 2 && p.current().dispatched && p.change_baseline.is_none()
    {
        return Err(Error::Journal);
    }
    if op.purpose == ManagementIpv6Next::LeavePartial
        && (p.predecessor.is_none()
            || r.state != crate::OperationState::PartiallyCompleted
            || p.successor.is_some())
    {
        return Err(Error::Journal);
    }
    for (i, v) in p.receipts.iter().enumerate() {
        if v.stage != Stage::ALL[i]
            || i < p.depth as usize && !v.verified
            || v.task_baseline.len() > 10000
            || v.task_baseline.iter().any(|id| id.uuid().is_nil())
            || v.task_uuid.is_some_and(|id| {
                id.uuid().is_nil() || v.task.is_none() || v.task_baseline.contains(&id)
            })
            || v.task.is_some() && !v.dispatched
            || v.ended && v.task_uuid.is_none()
            || v.completed && !v.ended
            || v.verified && !v.completed
            || v.result.is_some()
                && (!v.completed || v.stage != Stage::ChangeBond || !op.operation.creates())
            || i == p.depth as usize
                && v.dispatched
                && (!p.temporary_route_verified
                    || !p.registered
                    || p.depth == 4 && !p.final_route_verified)
        {
            return Err(Error::Journal);
        }
        for t in [&v.task, &v.result].into_iter().flatten() {
            if !t.starts_with("OpaqueRef:") || t == "OpaqueRef:NULL" || t.len() > 512 {
                return Err(Error::Journal);
            }
        }
    }
    if r.task != p.current().task
        || r.result_reference != p.current().result
        || (r.state == crate::OperationState::Succeeded && (!p.current().verified || p.depth != 4))
        || (r.state == crate::OperationState::PartiallyCompleted
            && (!p.current().verified && (p.current().dispatched || p.depth == 0)
                || p.depth == 4 && p.current().verified))
        || p.change_baseline
            .as_ref()
            .is_some_and(|b| b.created.is_some() || !b.replacements.is_empty())
            && !p.receipts.get(2).is_some_and(|v| v.completed)
    {
        return Err(Error::Journal);
    }
    for (i, s) in r.steps.iter().enumerate() {
        if s.label != Stage::ALL[i].label()
            || (s.state == crate::StepState::Verified)
                != p.receipts.get(i).is_some_and(|v| v.verified)
        {
            return Err(Error::Journal);
        }
    }
    Ok(())
}
