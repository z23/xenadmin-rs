use crate::{HostId, ManagementBondOperation, ManagementInterface, NetworkId, VifId, VmId};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use xenadmin_xapi::enumerations::{network_default_locking_mode, vm_power_state};

/// The server's authoritative locality rule for a guest affected by a bond change.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ManagementVmLocality {
    Resident(HostId),
    /// A successful, complete VM.get_possible_hosts response; empty means no possible host.
    PossibleHosts(Vec<HostId>),
}
impl ManagementVmLocality {
    pub fn local_to(&self, host: HostId) -> bool {
        match self {
            Self::Resident(id) => *id == host,
            Self::PossibleHosts(ids) => ids.is_empty() || ids.as_slice() == [host],
        }
    }
    fn valid(&self) -> bool {
        match self {
            Self::Resident(id) => !id.uuid().is_nil(),
            Self::PossibleHosts(ids) => {
                ids.len() <= 1024
                    && ids.iter().all(|id| !id.uuid().is_nil())
                    && ids.windows(2).all(|v| v[0] < v[1])
            }
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ManagementVifChange {
    Move,
    Reattach,
    Retain,
}
impl ManagementVifChange {
    pub fn label(self) -> &'static str {
        match self {
            Self::Move => "Move network",
            Self::Reattach => "Reattach on recreated VLAN",
            Self::Retain => "Retain network",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagementBondConsumer {
    pub interface: VifId,
    pub vm: VmId,
    pub vm_name: String,
    pub power_state: String,
    pub template: bool,
    pub snapshot: bool,
    pub locality: ManagementVmLocality,
    pub network: NetworkId,
    pub destination: NetworkId,
    pub change: ManagementVifChange,
    pub device: String,
    pub mac: String,
    pub mtu: u64,
    pub attached: bool,
    /// Canonical configuration evidence; no raw VIF metadata is exposed or submitted.
    pub settings_fingerprint: [u8; 32],
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagementBondNetwork {
    pub name: String,
    pub mtu: u64,
    pub default_locking_mode: Option<String>,
    pub settings_fingerprint: [u8; 32],
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagementBondConsumers {
    pub networks: BTreeMap<NetworkId, ManagementBondNetwork>,
    pub interfaces: BTreeMap<VifId, ManagementBondConsumer>,
}
impl ManagementBondConsumers {
    pub(crate) fn valid(&self, source: &ManagementInterface, op: &ManagementBondOperation) -> bool {
        let Some((networks, moves, reattachments)) = scope(source, op) else {
            return false;
        };
        self.networks.keys().copied().collect::<BTreeSet<_>>() == networks
            && self.networks.values().all(|v| {
                v.name.len() <= 1024
                    && (576..=65535).contains(&v.mtu)
                    && (v.default_locking_mode.as_deref().is_some_and(|s| {
                        matches!(
                            s,
                            network_default_locking_mode::UNLOCKED
                                | network_default_locking_mode::DISABLED
                        )
                    }) || self.interfaces.is_empty() && v.default_locking_mode.is_none())
            })
            && self.interfaces.len() <= 1024
            && self.interfaces.iter().all(|(id, v)| {
                let (destination, change) = disposition(
                    source.identity.host,
                    &moves,
                    &reattachments,
                    v.network,
                    &v.locality,
                    v.attached,
                    &v.power_state,
                );
                *id == v.interface
                    && !id.uuid().is_nil()
                    && !v.vm.uuid().is_nil()
                    && v.vm_name.len() <= 1024
                    && v.device.len() <= 64
                    && !v.device.is_empty()
                    && v.mac.len() == 17
                    && (576..=65535).contains(&v.mtu)
                    && matches!(
                        v.power_state.as_str(),
                        vm_power_state::RUNNING
                            | vm_power_state::HALTED
                            | vm_power_state::PAUSED
                            | vm_power_state::SUSPENDED
                    )
                    && v.locality.valid()
                    && networks.contains(&v.network)
                    && v.destination == destination
                    && v.change == change
            })
    }
}
pub(crate) type Scope = (
    BTreeSet<NetworkId>,
    BTreeMap<NetworkId, NetworkId>,
    BTreeSet<NetworkId>,
);
pub(crate) fn scope(source: &ManagementInterface, op: &ManagementBondOperation) -> Option<Scope> {
    let mut all = BTreeSet::new();
    let mut moves = BTreeMap::new();
    match op {
        ManagementBondOperation::Create {
            members, network, ..
        } => {
            all.insert(*network);
            for m in members {
                all.insert(m.identity.network);
                moves.insert(m.identity.network, *network);
                if let Some(t) = &m.topology {
                    all.extend(
                        t.pifs
                            .values()
                            .map(|p| p.network)
                            .collect::<Option<Vec<_>>>()?,
                    );
                }
            }
        }
        ManagementBondOperation::Destroy { primary, .. } => {
            let t = source.topology.as_ref()?;
            let parent = ManagementBondOperation::management_parent(source)?;
            all.extend(
                t.pifs
                    .values()
                    .map(|p| p.network)
                    .collect::<Option<Vec<_>>>()?,
            );
            moves.insert(t.pifs.get(&parent)?.network?, t.pifs.get(primary)?.network?);
        }
    }
    let reattachments = op
        .raw_vlans(source)?
        .values()
        .filter(|v| v.interface.currently_attached == Some(true))
        .map(|v| v.interface.network)
        .collect::<Option<BTreeSet<_>>>()?;
    Some((all, moves, reattachments))
}
pub(crate) fn disposition(
    host: HostId,
    moves: &BTreeMap<NetworkId, NetworkId>,
    reattachments: &BTreeSet<NetworkId>,
    network: NetworkId,
    locality: &ManagementVmLocality,
    attached: bool,
    state: &str,
) -> (NetworkId, ManagementVifChange) {
    if let Some(destination) = moves.get(&network).filter(|_| locality.local_to(host)) {
        return (*destination, ManagementVifChange::Move);
    }
    (
        network,
        if reattachments.contains(&network)
            && *locality == ManagementVmLocality::Resident(host)
            && attached
            && state != vm_power_state::SUSPENDED
        {
            ManagementVifChange::Reattach
        } else {
            ManagementVifChange::Retain
        },
    )
}
