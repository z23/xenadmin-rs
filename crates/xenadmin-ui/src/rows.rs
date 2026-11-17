//! Inventory rows for the current object-type page.
//!
//! The Rune table still owns sort direction and passes its natural name
//! order in. This module owns the projection and the column comparison. The
//! byte labels are in [`crate::format`].

use xenadmin_core::{HostId, Inventory, VmId};

use crate::format::{memory_label, storage_bytes};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InventoryPage {
    Guests,
    Templates,
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

pub fn project_rows(
    inventory: &Inventory,
    page: InventoryPage,
    host_filter: Option<HostId>,
    query: &str,
) -> Vec<DisplayRow> {
    let query = query.trim().to_lowercase();
    let mut rows = Vec::new();
    match page {
        InventoryPage::Guests | InventoryPage::Templates => {
            for vm in inventory.vms.values().filter(|vm| {
                !vm.is_control_domain
                    && vm.is_template == (page == InventoryPage::Templates)
                    && host_filter.is_none_or(|id| vm.host == Some(id))
            }) {
                rows.push(vm_row(inventory, vm));
            }
        }
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
    rows.retain(|row| {
        query.is_empty()
            || row.name.to_lowercase().contains(&query)
            || row.id.contains(&query)
            || row.location.to_lowercase().contains(&query)
    });
    rows
}

pub fn compare_rows(
    column: &str,
    left: &DisplayRow,
    right: &DisplayRow,
    names: impl Fn(&str, &str) -> std::cmp::Ordering,
) -> std::cmp::Ordering {
    match column {
        "state" => left.state.cmp(&right.state),
        "location" => left.location.cmp(&right.location),
        "cpu" => left
            .cpu
            .parse::<u64>()
            .unwrap_or(0)
            .cmp(&right.cpu.parse().unwrap_or(0)),
        _ => names(&left.name, &right.name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_query_keeps_matching_guests() {
        let inventory = crate::demonstration(3, 48);
        let rows = project_rows(&inventory, InventoryPage::Guests, None, "Xen Orchestra");
        assert!(!rows.is_empty());
        assert!(rows.iter().all(|row| row.name.contains("Xen Orchestra")));
    }

    // Rune: main.rs `search_and_host_filter_select_matching_objects`, the
    // second half.
    #[test]
    fn a_host_filter_keeps_only_the_guests_on_that_host() {
        let inventory = crate::demonstration(3, 48);
        let host = inventory.hosts.keys().next().copied();
        let rows = project_rows(&inventory, InventoryPage::Guests, host, "Xen Orchestra");
        assert!(!rows.is_empty());
        for row in &rows {
            assert_eq!(inventory.vms[&row.vm.unwrap()].host, host);
        }
        // One VM's row is the row the page shows for it.
        let vm = inventory.vms.values().next().unwrap();
        let all = project_rows(&inventory, InventoryPage::Guests, None, "");
        assert_eq!(
            all.iter().find(|row| row.vm == Some(vm.id)),
            Some(&vm_row(&inventory, vm))
        );
    }
}
