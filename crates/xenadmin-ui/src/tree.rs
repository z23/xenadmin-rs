//! Infrastructure tree rows. The widget that displays them stays in the front end.

use xenadmin_core::{HostId, Inventory, VmId};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub label: String,
    pub host: Option<HostId>,
    pub vm: Option<VmId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeInsert {
    pub parent: Option<String>,
    pub key: String,
    pub node: Node,
}

/// Inserts, then the keys that should be expanded, in the order the Rune
/// tree applied them: each host, then the pool.
pub fn infrastructure_nodes(
    inventory: &Inventory,
    host_expanded: impl Fn(&str) -> bool,
) -> (Vec<TreeInsert>, Vec<String>) {
    let mut inserts = Vec::new();
    let mut expand = Vec::new();
    let pool_key = inventory.pool_id.to_string();
    inserts.push(TreeInsert {
        parent: None,
        key: pool_key.clone(),
        node: Node {
            label: inventory.pool_name.clone(),
            host: None,
            vm: None,
        },
    });
    for host in inventory.hosts.values() {
        let host_key = host.id.to_string();
        inserts.push(TreeInsert {
            parent: Some(pool_key.clone()),
            key: host_key.clone(),
            node: Node {
                label: host.name.clone(),
                host: Some(host.id),
                vm: None,
            },
        });
        for vm in inventory.guests().filter(|vm| vm.host == Some(host.id)) {
            inserts.push(TreeInsert {
                parent: Some(host_key.clone()),
                key: vm.id.to_string(),
                node: Node {
                    label: vm.name.clone(),
                    host: Some(host.id),
                    vm: Some(vm.id),
                },
            });
        }
        if host_expanded(&host_key) {
            expand.push(host_key);
        }
    }
    for vm in inventory.guests().filter(|vm| vm.host.is_none()) {
        inserts.push(TreeInsert {
            parent: Some(pool_key.clone()),
            key: vm.id.to_string(),
            node: Node {
                label: vm.name.clone(),
                host: None,
                vm: Some(vm.id),
            },
        });
    }
    expand.push(pool_key);
    (inserts, expand)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_demo_tree_contains_the_pool_hosts_and_guests() {
        let inventory = crate::demonstration(3, 48);
        let (inserts, expand) = infrastructure_nodes(&inventory, |_| true);
        assert_eq!(inserts.len(), 1 + 3 + 48);
        assert_eq!(inserts[0].node.label, "Demonstration pool");
        assert_eq!(expand.len(), 4);
        assert_eq!(
            expand.last().map(String::as_str),
            Some(inserts[0].key.as_str())
        );
    }
}
