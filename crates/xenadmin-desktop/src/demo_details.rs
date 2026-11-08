//! Synthetic detail records used only by the demonstration inventory and render scenes.
use std::collections::BTreeMap;
use uuid::Uuid;
use xenadmin_core::{
    ClassAvailability, GuestMetrics, GuestMetricsId, Inventory, NetworkDetails, NetworkId,
    PowerState, SrId, StorageDetails, VbdId, VdiId, VifId, VirtualBlockDevice, VirtualDisk,
    VirtualInterface, VmDetails,
};

pub(super) fn populate(inventory: &mut Inventory) {
    let representative = inventory
        .guests()
        .find(|vm| vm.power_state == PowerState::Halted)
        .or_else(|| inventory.guests().next())
        .cloned();
    for class in [
        "SR",
        "network",
        "VDI",
        "VBD",
        "VIF",
        "PIF",
        "VM_guest_metrics",
    ] {
        inventory
            .details
            .classes
            .insert(class.to_ascii_lowercase(), ClassAvailability::Available);
    }
    for vm in inventory.vms.values() {
        inventory.details.vms.insert(
            vm.id,
            VmDetails {
                allowed_operations: Some(if vm.power_state == PowerState::Running {
                    vec![
                        "pause".into(),
                        "clean_shutdown".into(),
                        "clean_reboot".into(),
                        "snapshot".into(),
                        "checkpoint".into(),
                    ]
                } else {
                    vec!["start".into(), "snapshot".into()]
                }),
                vcpus_max: Some(vm.vcpus),
                vcpus_at_startup: Some(vm.vcpus),
                memory_static_min: Some(vm.memory_bytes.min(1 << 30)),
                memory_static_max: Some(vm.memory_bytes),
                memory_dynamic_min: Some(vm.memory_bytes),
                memory_dynamic_max: Some(vm.memory_bytes),
                memory_target: Some(vm.memory_bytes),
                boot_order: Some("cdn".into()),
                boot_policy: Some("BIOS order".into()),
                firmware: Some("uefi".into()),
                affinity: vm.host.or(inventory.coordinator),
                tags: vec!["demo".into()],
                auto_power_on: Some(false),
                ..Default::default()
            },
        );
    }
    for storage in &inventory.storage {
        let id = SrId::from(storage.uuid);
        inventory.details.storage.insert(
            id,
            StorageDetails {
                id,
                description: "Synthetic shared storage for UI previews.".into(),
                content_type: "user".into(),
                shared: Some(true),
                virtual_allocation: storage.used_bytes,
                allowed_operations: Some(vec!["scan".into(), "vdi_create".into()]),
                tags: vec!["demo".into()],
            },
        );
    }
    for network in &inventory.networks {
        let id = NetworkId::from(network.uuid);
        inventory.details.networks.insert(
            id,
            NetworkDetails {
                id,
                description: "Synthetic internal guest network for UI previews.".into(),
                mtu: Some(1500),
                managed: Some(true),
                default_locking_mode: Some("unlocked".into()),
                allowed_operations: Some(Vec::new()),
                tags: vec!["demo".into()],
            },
        );
        inventory.details.network_uplinks.insert(id, Vec::new());
    }
    let Some(vm) = representative else {
        return;
    };
    let sr = inventory
        .storage
        .first()
        .map(|storage| SrId::from(storage.uuid));
    let network = inventory
        .networks
        .first()
        .map(|network| NetworkId::from(network.uuid));
    let disk = VdiId::from(Uuid::from_u128(0x71000000000000000000000000000001));
    let disk_attachment = VbdId::from(Uuid::from_u128(0x72000000000000000000000000000001));
    let cd_attachment = VbdId::from(Uuid::from_u128(0x72000000000000000000000000000002));
    let nic = VifId::from(Uuid::from_u128(0x73000000000000000000000000000001));
    let guest = GuestMetricsId::from(Uuid::from_u128(0x74000000000000000000000000000001));
    let running = vm.power_state == PowerState::Running;
    inventory.details.vdis.insert(
        disk,
        VirtualDisk {
            id: disk,
            name: "Demo system disk".into(),
            description: "Synthetic disk; no host storage is attached.".into(),
            sr,
            virtual_size: Some(40 << 30),
            physical_utilisation: Some(12 << 30),
            kind: "user".into(),
            read_only: Some(false),
            sharable: Some(false),
            managed: Some(true),
            missing: Some(false),
            is_snapshot: Some(false),
            snapshot_of: None,
            cbt_enabled: Some(false),
            allowed_operations: Some(vec!["resize".into(), "snapshot".into()]),
        },
    );
    inventory.details.vbds.insert(
        disk_attachment,
        VirtualBlockDevice {
            id: disk_attachment,
            vm: Some(vm.id),
            snapshot: None,
            vdi: Some(disk),
            device: "xvda".into(),
            userdevice: "0".into(),
            kind: "Disk".into(),
            owned: true,
            mode: "RW".into(),
            currently_attached: Some(running),
            bootable: Some(true),
            empty: Some(false),
            allowed_operations: Some(vec![if running { "unplug" } else { "plug" }.into()]),
        },
    );
    inventory.details.vbds.insert(
        cd_attachment,
        VirtualBlockDevice {
            id: cd_attachment,
            vm: Some(vm.id),
            snapshot: None,
            vdi: None,
            device: "xvdd".into(),
            userdevice: "3".into(),
            kind: "CD".into(),
            owned: false,
            mode: "RO".into(),
            currently_attached: Some(running),
            bootable: Some(false),
            empty: Some(true),
            allowed_operations: Some(vec!["insert".into()]),
        },
    );
    inventory.details.vifs.insert(
        nic,
        VirtualInterface {
            id: nic,
            vm: Some(vm.id),
            network,
            device: "0".into(),
            mac: "02:00:00:00:00:23".into(),
            mtu: Some(1500),
            currently_attached: Some(running),
            qos_algorithm: String::new(),
            qos_parameters: BTreeMap::new(),
            allowed_operations: Some(vec![if running { "unplug" } else { "plug" }.into()]),
        },
    );
    inventory.details.guest_metrics.insert(
        guest,
        GuestMetrics {
            id: guest,
            os_version: BTreeMap::from([("name".into(), "Demo Linux (synthetic)".into())]),
            tools_version: BTreeMap::from([("version".into(), "8.3 demo".into())]),
            networks: BTreeMap::from([("0/ip".into(), "192.0.2.23".into())]),
            live: Some(running),
            last_updated: Some("20261108T00:00:00Z".into()),
        },
    );
    if let Some(details) = inventory.details.vms.get_mut(&vm.id) {
        details.guest_metrics = Some(guest);
    }
    inventory
        .details
        .vm_disks
        .insert(vm.id, vec![disk_attachment, cd_attachment]);
    inventory
        .details
        .vdi_attachments
        .insert(disk, vec![disk_attachment]);
    inventory.details.vm_interfaces.insert(vm.id, vec![nic]);
    if let Some(sr) = sr {
        inventory.details.sr_disks.insert(sr, vec![disk]);
    }
    if let Some(network) = network {
        inventory
            .details
            .network_interfaces
            .insert(network, vec![nic]);
    }
}
