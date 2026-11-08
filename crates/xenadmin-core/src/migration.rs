use crate::{
    ActionAvailability, ClassAvailability, Error, HostId, Inventory, NetworkId, PowerState, SrId,
    VdiId, VifId, VmId,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiskPlacement {
    pub vdi: VdiId,
    pub expected_sr: SrId,
    pub destination_sr: SrId,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NicPlacement {
    pub vif: VifId,
    pub expected_network: NetworkId,
    pub destination_network: NetworkId,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "vm_migration")]
pub struct VmMigration {
    pub destination: HostId,
    pub expected_host: Option<HostId>,
    pub expected_affinity: Option<HostId>,
    pub live: bool,
    pub disks: Vec<DiskPlacement>,
    pub networks: Vec<NicPlacement>,
    pub transfer_network: Option<NetworkId>,
}
impl VmMigration {
    pub fn storage_changes(&self) -> bool {
        self.disks.iter().any(|d| d.expected_sr != d.destination_sr)
    }
    pub fn uses_storage_transport(&self) -> bool {
        !self.live
            || self.storage_changes()
            || self
                .networks
                .iter()
                .any(|n| n.expected_network != n.destination_network)
    }
    pub(crate) fn capability(&self) -> &'static str {
        if self.uses_storage_transport() {
            "migrate_send"
        } else {
            "pool_migrate"
        }
    }
    pub fn label(&self) -> &'static str {
        if self.live {
            "Migrate running VM"
        } else {
            "Move halted VM"
        }
    }
    pub fn validate(&self) -> Result<(), Error> {
        if self.disks.len() > 256
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
                .map(|d| d.vif)
                .collect::<BTreeSet<_>>()
                .len()
                != self.networks.len()
            || self.live != self.expected_host.is_some()
            || (self.uses_storage_transport() && self.transfer_network.is_none())
            || (!self.uses_storage_transport() && self.expected_host == Some(self.destination))
            || (!self.live
                && self.expected_affinity == Some(self.destination)
                && !self.storage_changes()
                && self
                    .networks
                    .iter()
                    .all(|n| n.expected_network == n.destination_network))
        {
            return Err(Error::InvalidMigration);
        }
        Ok(())
    }
    pub(crate) fn resources(&self) -> Vec<String> {
        let mut ids = vec![format!("host:{}", self.destination)];
        ids.extend(self.expected_host.iter().map(|h| format!("host:{h}")));
        for d in &self.disks {
            ids.extend([
                format!("vdi:{}", d.vdi),
                format!("sr:{}", d.expected_sr),
                format!("sr:{}", d.destination_sr),
            ]);
        }
        for n in &self.networks {
            ids.extend([
                format!("vif:{}", n.vif),
                format!("network:{}", n.expected_network),
                format!("network:{}", n.destination_network),
            ]);
        }
        ids.extend(self.transfer_network.iter().map(|n| format!("network:{n}")));
        ids
    }
    pub fn availability(&self, inventory: &Inventory, vm: VmId) -> ActionAvailability {
        let no = |reason: &str| ActionAvailability::Prerequisite {
            reason: reason.into(),
        };
        if self.validate().is_err() {
            return no(
                "Choose a destination, complete device mappings and a transfer network for storage moves",
            );
        }
        let Some(guest) = inventory.vms.get(&vm) else {
            return ActionAvailability::Stale;
        };
        if guest.is_template
            || guest.is_control_domain
            || guest.host != self.expected_host
            || guest.power_state
                != if self.live {
                    PowerState::Running
                } else {
                    PowerState::Halted
                }
        {
            return no("The VM state or resident host changed since this review");
        }
        let d = &inventory.details;
        if ["vbd", "vdi", "vif", "sr", "network", "pbd", "pif"]
            .iter()
            .any(|c| d.classes.get(*c) != Some(&ClassAvailability::Available))
        {
            return ActionAvailability::Unknown;
        }
        let Some(details) = d.vms.get(&vm) else {
            return ActionAvailability::Unknown;
        };
        if !self.live && details.affinity != self.expected_affinity {
            return no("The VM home host changed since review");
        }
        if !details.current_operations.is_empty() {
            return ActionAvailability::Busy {
                operations: details.current_operations.clone(),
            };
        }
        if let Some(reason) = details.blocked_operations.get(self.capability()) {
            return ActionAvailability::Blocked {
                reason: reason.clone(),
            };
        }
        let Some(allowed) = &details.allowed_operations else {
            return ActionAvailability::Unknown;
        };
        if !allowed.iter().any(|a| a == self.capability()) {
            return ActionAvailability::Unavailable;
        }
        if !inventory.hosts.contains_key(&self.destination) {
            return ActionAvailability::Stale;
        }
        if !inventory.hosts[&self.destination].enabled {
            return no("The destination host is disabled");
        }
        if self.uses_storage_transport() && inventory.snapshots.iter().any(|s| s.vm == Some(vm)) {
            return no("Storage-transport moves with snapshot chains are not supported yet");
        }
        let bindings = d
            .vbds
            .values()
            .filter(|v| v.vm == Some(vm))
            .collect::<Vec<_>>();
        let expected = bindings
            .iter()
            .filter(|v| v.kind == "Disk" && v.empty == Some(false))
            .filter_map(|v| v.vdi)
            .collect::<BTreeSet<_>>();
        if expected != self.disks.iter().map(|d| d.vdi).collect() {
            return no("The disk attachment set changed; reopen the migration review");
        }
        let nics = d
            .vifs
            .values()
            .filter(|v| v.vm == Some(vm))
            .map(|v| v.id)
            .collect::<BTreeSet<_>>();
        if nics != self.networks.iter().map(|n| n.vif).collect() {
            return no("The network interface set changed; reopen the migration review");
        }
        for p in &self.disks {
            let Some(disk) = d.vdis.get(&p.vdi) else {
                return ActionAvailability::Stale;
            };
            if disk.sr != Some(p.expected_sr) {
                return no("A disk changed storage repository since review");
            }
            if p.destination_sr != p.expected_sr
                && (disk.sharable != Some(false)
                    || disk.read_only != Some(false)
                    || disk.is_snapshot != Some(false)
                    || disk.cbt_enabled != Some(false)
                    || disk.managed != Some(true)
                    || disk.missing != Some(false)
                    || disk.kind != "user"
                    || d.vdi_attachments.get(&p.vdi).is_none_or(|a| a.len() != 1))
            {
                return no(
                    "Moving shared, snapshot, CBT, missing or read-only disks is not supported",
                );
            }
            if p.destination_sr != p.expected_sr
                && d.storage.get(&p.destination_sr).is_none_or(|sr| {
                    sr.content_type != "user"
                        || sr
                            .allowed_operations
                            .as_ref()
                            .is_none_or(|ops| !ops.iter().any(|op| op == "vdi_create"))
                })
            {
                return no(
                    "The selected destination repository does not allow creating data disks",
                );
            }
            if !d.pbds.values().any(|a| {
                a.host == Some(self.destination)
                    && a.sr == Some(p.destination_sr)
                    && a.currently_attached == Some(true)
            }) {
                return no("The destination host cannot access a selected storage repository");
            }
        }
        for p in &self.networks {
            if d.vifs
                .get(&p.vif)
                .is_none_or(|v| v.network != Some(p.expected_network))
            {
                return no("A network interface changed since review");
            }
            if !inventory
                .networks
                .iter()
                .any(|n| n.uuid == p.destination_network.uuid())
            {
                return ActionAvailability::Stale;
            }
            let uplinks = d
                .pifs
                .values()
                .filter(|a| a.network == Some(p.destination_network))
                .collect::<Vec<_>>();
            if !uplinks.is_empty()
                && !uplinks
                    .iter()
                    .any(|p| p.host == Some(self.destination) && p.currently_attached == Some(true))
            {
                return no("The destination host has no attached uplink for a selected network");
            }
        }
        for attachment in bindings
            .iter()
            .filter(|a| a.kind == "CD" && a.empty == Some(false))
        {
            let Some(sr) = attachment
                .vdi
                .and_then(|id| d.vdis.get(&id))
                .and_then(|disk| disk.sr)
            else {
                return ActionAvailability::Unknown;
            };
            if !d.pbds.values().any(|a| {
                a.sr == Some(sr)
                    && a.host == Some(self.destination)
                    && a.currently_attached == Some(true)
            }) {
                return no("The destination host cannot access the inserted CD image");
            }
        }
        if self.uses_storage_transport()
            && !self.transfer_network.is_some_and(|network| {
                d.pifs.values().any(|p| {
                    p.network == Some(network)
                        && p.host == Some(self.destination)
                        && p.currently_attached == Some(true)
                        && p.ip
                            .parse::<std::net::IpAddr>()
                            .is_ok_and(|ip| !ip.is_unspecified() && !ip.is_loopback())
                })
            })
        {
            return no(
                "Choose a transfer network with an attached, IP-configured destination interface",
            );
        }
        ActionAvailability::Available
    }
}
