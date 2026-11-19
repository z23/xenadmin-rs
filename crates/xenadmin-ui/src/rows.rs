//! The row of one object: what the search result and the general tab say of
//! a guest, a server, a storage repository or a network. The byte labels are
//! in [`crate::format`].

use xenadmin_core::{Inventory, VmId};

use crate::format::{memory_label, storage_bytes};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InventoryPage {
    Hosts,
    Storage,
    Networks,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisplayRow {
    pub id: String,
    pub name: String,
    pub state: String,
    pub location: String,
    pub cpu: String,
    pub memory: String,
    pub description: String,
    pub vm: Option<VmId>,
}

/// The row of one VM or template.
pub fn vm_row(inventory: &Inventory, vm: &xenadmin_core::Vm) -> DisplayRow {
    DisplayRow {
        id: vm.id.to_string(),
        name: vm.name.clone(),
        state: vm.power_state.to_string(),
        location: vm
            .host
            .and_then(|id| inventory.hosts.get(&id))
            .map(|host| host.name.clone())
            .unwrap_or_else(|| "—".into()),
        cpu: vm.vcpus.to_string(),
        memory: memory_label(vm.memory_bytes),
        description: vm.description.clone(),
        vm: Some(vm.id),
    }
}

/// The rows of every object of one kind. A guest's or a template's row is
/// [`vm_row`].
pub fn project_rows(inventory: &Inventory, page: InventoryPage) -> Vec<DisplayRow> {
    let mut rows = Vec::new();
    match page {
        InventoryPage::Hosts => {
            for host in inventory.hosts.values() {
                rows.push(DisplayRow {
                    id: host.id.to_string(),
                    name: host.name.clone(),
                    state: if host.enabled { "Enabled" } else { "Disabled" }.into(),
                    location: host.address.clone(),
                    cpu: "—".into(),
                    memory: "—".into(),
                    description: format!("XCP-ng {}", host.version),
                    vm: None,
                });
            }
        }
        InventoryPage::Storage => {
            for repository in &inventory.storage {
                rows.push(DisplayRow {
                    id: repository.uuid.to_string(),
                    name: repository.name.clone(),
                    state: repository.kind.clone(),
                    location: repository
                        .used_bytes
                        .map(|value| format!("{} used", storage_bytes(Some(value))))
                        .unwrap_or_else(|| "Usage unknown".into()),
                    cpu: "—".into(),
                    memory: storage_bytes(repository.capacity_bytes),
                    description: "Storage repository".into(),
                    vm: None,
                });
            }
        }
        InventoryPage::Networks => {
            for network in &inventory.networks {
                rows.push(DisplayRow {
                    id: network.uuid.to_string(),
                    name: network.name.clone(),
                    state: "Network".into(),
                    location: network.bridge.clone(),
                    cpu: "—".into(),
                    memory: "—".into(),
                    description: "Virtual network".into(),
                    vm: None,
                });
            }
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_object_of_a_kind_has_one_row_under_its_own_id() {
        let inventory = crate::demonstration(3, 48);
        let hosts = project_rows(&inventory, InventoryPage::Hosts);
        assert_eq!(hosts.len(), inventory.hosts.len());
        for host in inventory.hosts.values() {
            let row = hosts
                .iter()
                .find(|row| row.id == host.id.to_string())
                .expect("the host's row");
            assert_eq!(row.name, host.name);
            assert_eq!(row.vm, None);
        }
        assert_eq!(
            project_rows(&inventory, InventoryPage::Storage).len(),
            inventory.storage.len()
        );
        assert_eq!(
            project_rows(&inventory, InventoryPage::Networks).len(),
            inventory.networks.len()
        );
        // A guest's row names the guest.
        let vm = inventory.vms.values().next().unwrap();
        let row = vm_row(&inventory, vm);
        assert_eq!((row.vm, row.name.as_str()), (Some(vm.id), vm.name.as_str()));
    }
}
