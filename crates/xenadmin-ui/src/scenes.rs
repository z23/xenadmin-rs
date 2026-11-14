//! Scene names from the Rune application. A ported scene uses the shared model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scene {
    pub name: &'static str,
    pub ported: bool,
    pub debug_only: bool,
}
pub const REGISTRY: &[Scene] = &[
    Scene {
        name: "demo",
        ported: true,
        debug_only: false,
    },
    Scene {
        name: "large-inventory",
        ported: true,
        debug_only: false,
    },
    Scene {
        name: "host-version-notice",
        ported: true,
        debug_only: false,
    },
    Scene {
        name: "interface-large",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "inventory-context",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "general-information",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "options",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "help",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "console-details",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "batch-power",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "batch-properties",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "batch-delete",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "lifecycle-permissions",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "batch-move",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "batch-cross-pool",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "batch-export",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "snapshot-batch-export",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "console-recovery",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "console-frame",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "console-actual-size",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "console-typing-review",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "multiple-consoles",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "guest-connections",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "external-tools",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "host-console",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "search-pools",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "performance",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "metric-presets",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "unavailable-actions",
        ported: true,
        debug_only: false,
    },
    Scene {
        name: "vm-devices",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "properties",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "properties-running",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "vm-create",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "device-properties",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "home-host",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "firmware-create",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "creation-cloud",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "creation-cloud-pending",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "secure-boot",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "vm-migration",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "vm-recovery",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "vm-recovery-force",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "cross-pool",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "archive-export",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "ha-policy",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "failure-help",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "checkpoint-export",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "xva-compression",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "cloud-init",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "cloud-init-review",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "ovf-import",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "ovf-export",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "disk-export",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "snapshots",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "snapshot-create",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "snapshot-ancestry",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "snapshot-information",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "snapshot-restore",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "snapshot-edit",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "snapshot-capture",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "connections",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "connection-trust",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "certificate-review",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "network-interfaces",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "network-services",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "network-fabric",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "network-fabric-pool",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "network-fabric-pool-select",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "network-sriov",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "network-ipv6",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "network-topology",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "management-network",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "management-dhcp",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "management-vlan",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "management-bond",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "management-ipv6-bond",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "management-bond-guests",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "management-vlan-bond",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "host-administration",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "host-memory",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "host-monitoring",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "local-conversion",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "local-conversion-review",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "host-monitoring-catalog",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "host-support",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "host-restore",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "host-crashdump-delete",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "host-power-settings",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "snapshot-schedules",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "snapshot-schedule-review",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "alarms",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "alarms-review",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "alarms-email",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "network-create",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "activity",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "organization",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "organization-review",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "appliances",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "appliance-review",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "disk-cleanup",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "disk-grow",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "disk-copy",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "storage-create",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "storage-review",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "storage-local",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "storage-local-review",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "storage-hba",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "storage-hba-review",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "storage-repair",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "storage-graph",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "storage-multipath",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "storage-maintenance",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "storage-cache",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "storage-discovery",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "storage-auth-review",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "pool-security",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "pool-join",
        ported: false,
        debug_only: true,
    },
    Scene {
        name: "root-password",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "pool-dissolve",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "guest-tools",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "guest-tools-review",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "pool-security-review",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "pool-policies",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "pool-policy-review",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "pool-administration",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "connection-settings",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "connect",
        ported: false,
        debug_only: false,
    },
    Scene {
        name: "connect-insecure",
        ported: false,
        debug_only: false,
    },
];

pub fn by_name(name: &str) -> Option<&'static Scene> {
    REGISTRY.iter().find(|scene| scene.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_registry_has_one_entry_per_rune_scene() {
        assert_eq!(REGISTRY.len(), 128);
        assert!(by_name("demo").is_some_and(|scene| scene.ported));
        assert!(by_name("large-inventory").is_some_and(|scene| scene.ported));
        assert!(by_name("host-version-notice").is_some_and(|scene| scene.ported));
        assert!(by_name("unavailable-actions").is_some_and(|scene| scene.ported));
        assert!(by_name("pool-join").is_some_and(|scene| scene.debug_only && !scene.ported));
        let mut names: Vec<_> = REGISTRY.iter().map(|scene| scene.name).collect();
        let count = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), count);
    }
}
