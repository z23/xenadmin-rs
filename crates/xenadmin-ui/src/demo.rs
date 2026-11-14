//! The synthetic pool used by the demo and large-inventory scenes.
//!
//! Detail records are filled by [`crate::populate_demo_details`] afterwards,
//! as the Rune scene does.

use std::collections::BTreeMap;

use xenadmin_core::{
    Host, HostId, Inventory, Network, PoolId, PowerState, Snapshot, SnapshotAction, SnapshotId,
    SnapshotInventory, Storage, Vm, VmId,
};

pub fn demonstration(host_count: usize, vm_count: usize) -> Inventory {
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
                description: "Synthetic inventory for exploring the desktop interface.".into(),
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
    Inventory {
        pool_id,
        pool_name: "Demonstration pool".into(),
        coordinator: host_ids.first().copied(),
        hosts,
        vms,
        snapshots: SnapshotInventory::from(BTreeMap::from_iter((0..3).map(|i| {
            let id =
                SnapshotId::parse(&format!("60000000-0000-0000-0000-{i:012}")).expect("snapshot");
            (
                id,
                Snapshot {
                    id,
                    vm: Some(
                        VmId::parse("30000000-0000-0000-0000-000000000000").expect("snapshot vm"),
                    ),
                    name: ["Before upgrade", "Known good", "Initial installation"][i].into(),
                    description: "Synthetic disk snapshot".into(),
                    created_at: Some(format!("2026110{}T10:00:00Z", 7 - i)),
                    power_state: PowerState::Halted,
                    allowed_actions: vec![SnapshotAction::Revert],
                },
            )
        }))),
        storage: vec![Storage {
            uuid: uuid::Uuid::from_u128(0x4000_0000_0000_0000_0000_0000_0000_0001),
            name: "Demo storage".into(),
            kind: "nfs".into(),
            capacity_bytes: Some(2 << 40),
            used_bytes: Some(512 << 30),
        }],
        networks: vec![Network {
            uuid: uuid::Uuid::from_u128(0x5000_0000_0000_0000_0000_0000_0000_0001),
            name: "Demo network".into(),
            bridge: "xenbr0".into(),
        }],
        revision: 1,
        details: Default::default(),
    }
}
