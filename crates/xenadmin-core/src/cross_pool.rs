use crate::{
    ActionAvailability, ClassAvailability, Connection, DiskPlacement, Error, HostId, Inventory,
    NetworkId, NicPlacement, ObservedResource, ObservedVm, OperationId, PoolId, PowerState,
    ResourceAction, ResourceOperation, VmId,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use xenadmin_xapi::enumerations::{
    storage_operations, vbd_mode, vbd_type, vdi_type, vm_operations,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CrossPoolMode {
    MoveHalted,
    MoveRunning,
    CopyHalted,
}
impl CrossPoolMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::MoveHalted => "Move halted VM to another pool",
            Self::MoveRunning => "Migrate running VM to another pool",
            Self::CopyHalted => "Copy halted VM to another pool",
        }
    }
    pub fn live(self) -> bool {
        self == Self::MoveRunning
    }
    pub fn copy(self) -> bool {
        self == Self::CopyHalted
    }
}

/// Exact reviewed mappings. IDs in destination fields belong to destination_pool.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrossPoolVmOperation {
    pub vm: VmId,
    pub destination_pool: PoolId,
    pub destination_host: HostId,
    pub expected_host: Option<HostId>,
    pub expected_affinity: Option<HostId>,
    pub transfer_network: NetworkId,
    pub transfer_address: String,
    pub mode: CrossPoolMode,
    pub disks: Vec<DiskPlacement>,
    pub networks: Vec<NicPlacement>,
    /// The review explains that host-to-host policy is separate from desktop TLS.
    pub host_transport_reviewed: bool,
}
#[derive(Clone, Debug)]
pub struct CrossPoolVmSelection {
    pub source: ObservedVm,
    pub destination: ObservedResource,
    pub operation: CrossPoolVmOperation,
}
impl CrossPoolVmOperation {
    pub(crate) fn validate(&self, source: PoolId) -> Result<(), Error> {
        if source == self.destination_pool
            || !self.host_transport_reviewed
            || self.vm.uuid().is_nil()
            || self.destination_pool.uuid().is_nil()
            || self.destination_host.uuid().is_nil()
            || self.transfer_network.uuid().is_nil()
            || self.mode.live() != self.expected_host.is_some()
            || !usable_address(&self.transfer_address)
            || self.expected_host.is_some_and(|id| id.uuid().is_nil())
            || self.expected_affinity.is_some_and(|id| id.uuid().is_nil())
            || self.disks.iter().any(|d| {
                d.vdi.uuid().is_nil()
                    || d.expected_sr.uuid().is_nil()
                    || d.destination_sr.uuid().is_nil()
            })
            || self.networks.iter().any(|n| {
                n.vif.uuid().is_nil()
                    || n.expected_network.uuid().is_nil()
                    || n.destination_network.uuid().is_nil()
            })
            || self.disks.len() > 256
            || self.networks.len() > 256
            || self
                .disks
                .iter()
                .map(|d| d.vdi)
                .collect::<BTreeSet<_>>()
                .len()
                != self.disks.len()
            || self
                .networks
                .iter()
                .map(|n| n.vif)
                .collect::<BTreeSet<_>>()
                .len()
                != self.networks.len()
        {
            return Err(Error::InvalidMigration);
        }
        Ok(())
    }
    pub fn availability(&self, source: &Inventory, destination: &Inventory) -> ActionAvailability {
        let no = |text: &str| ActionAvailability::Prerequisite {
            reason: text.into(),
        };
        if self.validate(source.pool_id).is_err() || destination.pool_id != self.destination_pool {
            return no(
                "Choose a different pool, complete mappings and review host-to-host transport",
            );
        }
        for inventory in [source, destination] {
            if ["vbd", "vdi", "vif", "sr", "network", "pbd", "pif"]
                .iter()
                .any(|c| inventory.details.classes.get(*c) != Some(&ClassAvailability::Available))
            {
                return ActionAvailability::Unknown;
            }
        }
        let Some(vm) = source.vms.get(&self.vm) else {
            return ActionAvailability::Stale;
        };
        if vm.is_control_domain
            || vm.is_template
            || vm.host != self.expected_host
            || vm.power_state
                != if self.mode.live() {
                    PowerState::Running
                } else {
                    PowerState::Halted
                }
        {
            return no("The ordinary guest's state or resident host changed; reopen this review");
        }
        let Some(details) = source.details.vms.get(&self.vm) else {
            return ActionAvailability::Unknown;
        };
        if !details.affinity_known
            || details.affinity != self.expected_affinity
            || !details.appliance_known
        {
            return no("Home-host or appliance membership is incomplete or changed");
        }
        if details.appliance.is_some()
            || details.snapshot_policy.is_some()
            || details
                .ha_restart_priority
                .as_deref()
                .is_none_or(|v| !v.is_empty())
            || source.snapshots.iter().any(|s| s.vm == Some(self.vm))
        {
            return no(
                "Snapshot chains, schedules, HA protection and vApp membership require a separate transfer workflow",
            );
        }
        if let Some(reason) = details.blocked_operations.get(vm_operations::MIGRATE_SEND) {
            return ActionAvailability::Blocked {
                reason: reason.clone(),
            };
        }
        if !details.current_operations.is_empty() {
            return ActionAvailability::Busy {
                operations: details.current_operations.clone(),
            };
        }
        if details
            .allowed_operations
            .as_ref()
            .is_none_or(|ops| !ops.iter().any(|op| op == vm_operations::MIGRATE_SEND))
        {
            return ActionAvailability::Unavailable;
        }
        if destination
            .hosts
            .get(&self.destination_host)
            .is_none_or(|h| !h.enabled)
        {
            return no("Choose an enabled destination host");
        }
        if !destination.details.pifs.values().any(|p| {
            p.host == Some(self.destination_host)
                && p.network == Some(self.transfer_network)
                && p.currently_attached == Some(true)
                && (p.ip == self.transfer_address
                    || p.ipv6
                        .iter()
                        .any(|ip| ip.split('/').next() == Some(self.transfer_address.as_str())))
        }) {
            return no("The reviewed migration address is not attached to the destination host");
        }
        let blocks: Vec<_> = source
            .details
            .vbds
            .values()
            .filter(|b| b.vm == Some(self.vm))
            .collect();
        let disk_ids: BTreeSet<_> = blocks.iter().filter_map(|b| b.vdi).collect();
        if blocks.iter().any(|b| {
            (b.kind == vbd_type::CD && b.empty != Some(true))
                || (b.kind == vbd_type::DISK
                    && (!b.owned || b.mode != vbd_mode::RW || b.vdi.is_none()))
                || (b.kind != vbd_type::DISK && b.kind != vbd_type::CD)
        }) {
            return no("Only owned writable data disks and empty CD drives are supported");
        }
        if disk_ids != self.disks.iter().map(|d| d.vdi).collect() {
            return no("Disk attachments changed; reopen the transfer review");
        }
        for mapping in &self.disks {
            let Some(disk) = source.details.vdis.get(&mapping.vdi) else {
                return ActionAvailability::Stale;
            };
            if disk.sr != Some(mapping.expected_sr)
                || disk.kind != vdi_type::USER
                || disk.read_only != Some(false)
                || disk.sharable != Some(false)
                || disk.is_snapshot != Some(false)
                || disk.cbt_enabled != Some(false)
                || disk.managed != Some(true)
                || disk.missing != Some(false)
                || source
                    .details
                    .vdi_attachments
                    .get(&disk.id)
                    .is_none_or(|v| v.len() != 1)
            {
                return no(
                    "Shared, unowned, missing, snapshot or CBT disks cannot use this workflow",
                );
            }
            if destination
                .details
                .storage
                .get(&mapping.destination_sr)
                .is_none_or(|sr| {
                    sr.content_type != "user"
                        || sr.allowed_operations.as_ref().is_none_or(|ops| {
                            !ops.iter().any(|op| op == storage_operations::VDI_CREATE)
                        })
                })
                || !destination.details.pbds.values().any(|p| {
                    p.host == Some(self.destination_host)
                        && p.sr == Some(mapping.destination_sr)
                        && p.currently_attached == Some(true)
                })
            {
                return no(
                    "A selected destination repository is not writable and attached to the chosen host",
                );
            }
        }
        let nics: BTreeSet<_> = source
            .details
            .vifs
            .values()
            .filter(|v| v.vm == Some(self.vm))
            .map(|v| v.id)
            .collect();
        if nics != self.networks.iter().map(|n| n.vif).collect() {
            return no("Network interfaces changed; reopen the transfer review");
        }
        for mapping in &self.networks {
            if source
                .details
                .vifs
                .get(&mapping.vif)
                .is_none_or(|n| n.network != Some(mapping.expected_network))
                || !destination
                    .networks
                    .iter()
                    .any(|n| n.uuid == mapping.destination_network.uuid())
            {
                return no("A reviewed source or destination network is no longer available");
            }
        }
        ActionAvailability::Available
    }
}
pub(crate) fn usable_address(value: &str) -> bool {
    value.parse::<std::net::IpAddr>().is_ok_and(|address| {
        !address.is_unspecified()
            && !address.is_loopback()
            && !address.is_multicast()
            && match address {
                std::net::IpAddr::V4(v) => !v.is_broadcast() && !v.is_link_local(),
                std::net::IpAddr::V6(v) => !v.is_unicast_link_local(),
            }
    })
}
/// The destination pool's connection as it was reviewed. A batch hands each of its
/// children a copy; none of it is stored.
#[derive(Clone)]
pub(crate) struct Inputs {
    pub destination: crate::connection::JoinPeerHandle,
    pub observation: ObservedResource,
}
/// The other pool a transfer reserves and needs connected: the destination of a single
/// cross-pool move or copy, or of a batch of cross-pool moves.
pub(crate) fn destination(action: &ResourceAction) -> Option<PoolId> {
    match action {
        ResourceAction::CrossPoolVm(op) => Some(op.destination_pool),
        ResourceAction::VmWorkflowBatch(selection) => selection.cross_pool_destination(),
        _ => None,
    }
}
/// A reviewed batch of moves from one connected pool to another: both pools as they were
/// observed, and one item per guest with the mappings reviewed for it.
#[derive(Clone, Debug)]
pub struct CrossPoolBatchSelection {
    pub source: ObservedResource,
    pub destination: ObservedResource,
    pub batch: crate::VmWorkflowBatch,
}
impl Connection {
    /// Moves 1 to 32 guests of this pool to another connected pool, one after another,
    /// each by the single cross-pool move with its own reviewed mappings.
    pub fn move_vms_to_pool(
        &self,
        id: OperationId,
        selection: CrossPoolBatchSelection,
        destination: &Connection,
    ) -> Result<ResourceOperation, Error> {
        self.submit_cross_pool_batch(id, selection, destination)
    }
    /// Rechecks an unresolved batch of cross-pool moves with both pools connected. It
    /// reads the children that were started and never starts another.
    pub fn recheck_cross_pool_batch(
        &self,
        id: OperationId,
        destination: &Connection,
    ) -> Result<ResourceOperation, Error> {
        self.recheck_cross_pool(id, destination)
    }
    pub fn transfer_vm_to_pool(
        &self,
        id: OperationId,
        selection: CrossPoolVmSelection,
        destination: &Connection,
    ) -> Result<ResourceOperation, Error> {
        self.submit_cross_pool(id, selection, destination)
    }
    pub fn recheck_cross_pool_vm(
        &self,
        id: OperationId,
        destination: &Connection,
    ) -> Result<ResourceOperation, Error> {
        self.recheck_cross_pool(id, destination)
    }
}
