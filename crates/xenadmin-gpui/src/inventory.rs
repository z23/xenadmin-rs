//! Synthetic pool used by spike S3.
//!
//! The counts match the Rune `large-inventory` scene (16 hosts, 2,000 VMs).
//! The scene itself still lives in the Rune `State` and is not moved. This
//! builder uses the public inventory types and does not open a connection.

use std::collections::BTreeMap;

use xenadmin_core::{
    Host, HostId, Inventory, InventoryDetails, Network, PoolId, PowerState, SnapshotInventory,
    Storage, Vm, VmId,
};

/// Selection plus the latest synthetic inventory. The selected id is a `VmId`,
/// not a row index.
pub struct ScaleModel {
    pub inventory: Inventory,
    pub selected: Option<VmId>,
}

impl ScaleModel {
    pub fn demonstration(host_count: usize, vm_count: usize) -> Self {
        let pool_id = PoolId::parse("10000000-0000-0000-0000-000000000001").expect("pool id");
        let mut hosts = BTreeMap::new();
        for i in 1..=host_count {
            let id = HostId::parse(&format!("20000000-0000-0000-0000-{i:012}")).expect("host id");
            hosts.insert(
                id,
                Host {
                    id,
                    name: format!("xcp-lab-{i:02}"),
                    address: format!("192.0.2.{}", (i + 10) % 256),
                    enabled: true,
                    version: "8.3".into(),
                },
            );
        }
        let host_ids: Vec<_> = hosts.keys().copied().collect();
        let names = [
            "Xen Orchestra",
            "Database",
            "Application",
            "Build runner",
            "Monitoring",
            "Gateway",
            "Development",
            "File services",
        ];
        let mut vms = BTreeMap::new();
        for i in 0..vm_count {
            let id = VmId::parse(&format!("30000000-0000-0000-0000-{i:012}")).expect("vm id");
            let running = i % 5 != 0;
            vms.insert(
                id,
                Vm {
                    allowed_actions: Vec::new(),
                    can_snapshot: true,
                    id,
                    name: if i < names.len() {
                        names[i].into()
                    } else {
                        format!("{} {:02}", names[i % names.len()], i / names.len() + 1)
                    },
                    description: "Synthetic inventory for the GPUI scale spike.".into(),
                    power_state: if running {
                        PowerState::Running
                    } else {
                        PowerState::Halted
                    },
                    host: running.then(|| host_ids[i % host_count.max(1)]),
                    vcpus: if i % 3 == 0 { 4 } else { 2 },
                    memory_bytes: (2 + (i % 4) as u64) * (1 << 30),
                    is_template: false,
                    is_control_domain: false,
                },
            );
        }
        let selected = vms.keys().next().copied();
        Self {
            inventory: Inventory {
                pool_id,
                pool_name: "Demonstration pool".into(),
                coordinator: host_ids.first().copied(),
                hosts,
                vms,
                snapshots: SnapshotInventory::default(),
                storage: vec![Storage {
                    uuid: uuid_from_words(0x4000_0000_0000_0000, 0x0000_0000_0000_0001),
                    name: "Demo storage".into(),
                    kind: "nfs".into(),
                    capacity_bytes: Some(2 << 40),
                    used_bytes: Some(512 << 30),
                }],
                networks: vec![Network {
                    uuid: uuid_from_words(0x5000_0000_0000_0000, 0x0000_0000_0000_0001),
                    name: "Demo network".into(),
                    bridge: "xenbr0".into(),
                }],
                details: InventoryDetails::default(),
                revision: 1,
            },
            selected,
        }
    }

    /// Bump the revision and rewrite one description. The selected id stays.
    pub fn touch(&mut self) {
        self.inventory.revision = self.inventory.revision.saturating_add(1);
        if let Some(vm) = self.inventory.vms.values_mut().next() {
            vm.description = format!("revision {}", self.inventory.revision);
        }
    }

    pub fn selected_present(&self) -> bool {
        self.selected
            .is_some_and(|id| self.inventory.vms.contains_key(&id))
    }
}

fn uuid_from_words(high: u64, low: u64) -> uuid::Uuid {
    uuid::Uuid::from_u128(((high as u128) << 64) | low as u128)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_inventory_keeps_its_selection_across_updates() {
        let mut model = ScaleModel::demonstration(16, 2_000);
        assert_eq!(model.inventory.hosts.len(), 16);
        assert_eq!(model.inventory.vms.len(), 2_000);
        let selected = model.selected.expect("a vm");
        for _ in 0..100 {
            model.touch();
        }
        assert_eq!(model.selected, Some(selected));
        assert!(model.selected_present());
        assert_eq!(model.inventory.revision, 101);
    }
}
