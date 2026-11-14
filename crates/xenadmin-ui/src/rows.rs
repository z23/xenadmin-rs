//! Inventory rows for the current object-type page.
//!
//! The Rune table still owns sort direction and the natural name order. This
//! module owns the projection and the column comparison.

use xenadmin_core::{HostId, Inventory, VmId};

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

pub fn memory_label(value: u64) -> String {
    if value == 0 {
        "—".into()
    } else if value >= 1 << 30 {
        format!("{:.1} GiB", value as f64 / (1u64 << 30) as f64)
    } else {
        format!("{:.0} MiB", value as f64 / (1u64 << 20) as f64)
    }
}

pub fn storage_bytes(value: Option<u64>) -> String {
    match value {
        None => "Unknown".into(),
        Some(0) => "0 B".into(),
        Some(value) => memory_label(value),
    }
}

pub fn project_rows(
    inventory: &Inventory,
    page: InventoryPage,
    host_filter: Option<HostId>,
    query: &str,
) -> Vec<DisplayRow> {
    let query = query.trim().to_lowercase();
    let host_name = |id: Option<HostId>| {
        id.and_then(|id| inventory.hosts.get(&id))
            .map(|host| host.name.clone())
            .unwrap_or_else(|| "—".into())
    };
    let mut rows = Vec::new();
    match page {
        InventoryPage::Guests | InventoryPage::Templates => {
            for vm in inventory.vms.values().filter(|vm| {
                !vm.is_control_domain
                    && vm.is_template == (page == InventoryPage::Templates)
                    && host_filter.is_none_or(|id| vm.host == Some(id))
            }) {
                rows.push(DisplayRow {
                    id: vm.id.to_string(),
                    name: vm.name.clone(),
                    state: vm.power_state.to_string(),
                    location: host_name(vm.host),
                    cpu: vm.vcpus.to_string(),
                    memory: memory_label(vm.memory_bytes),
                    description: vm.description.clone(),
                    vm: Some(vm.id),
                });
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
    fn storage_labels_keep_the_desktop_wording() {
        assert_eq!(storage_bytes(None), "Unknown");
        assert_eq!(storage_bytes(Some(0)), "0 B");
        assert_eq!(memory_label(0), "—");
    }

    #[test]
    fn a_name_query_keeps_matching_guests() {
        let inventory = crate::demonstration(3, 48);
        let rows = project_rows(&inventory, InventoryPage::Guests, None, "Xen Orchestra");
        assert!(!rows.is_empty());
        assert!(rows.iter().all(|row| row.name.contains("Xen Orchestra")));
    }
}
