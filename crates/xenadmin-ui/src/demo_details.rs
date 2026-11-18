//! Synthetic detail records used only by the demonstration inventory and render scenes.
use std::collections::BTreeMap;
use uuid::Uuid;
use xenadmin_core::{
    ClassAvailability, GuestMetrics, GuestMetricsId, HostDetails, HostMetrics, HostMetricsId,
    Inventory, NetworkDetails, NetworkId, PowerState, SrId, StorageDetails, VbdId, VdiId, VifId,
    VirtualBlockDevice, VirtualDisk, VirtualInterface, VmDetails, VmId,
};

/// The synthetic metrics record of the demo host at `index`. The first
/// host's is the one the Rune scene of host administration used.
pub fn host_metrics_id(index: usize) -> HostMetricsId {
    HostMetricsId::from(Uuid::from_u128(0x9901 + ((index as u128) << 16)))
}

/// The synthetic control domain of the demo host at `index`. It has a
/// detail record and is not a guest of the inventory.
pub fn control_domain_id(index: usize) -> VmId {
    VmId::from(Uuid::from_u128(0x9902 + ((index as u128) << 16)))
}

/// What a server needs for its changes to be reviewable: live metrics with
/// 64 GiB, a control domain of 2 GiB, the operations it allows, boot and
/// toolstack start times, no power-on mode, a known log destination and
/// multipathing off. Every demo host has it, so in the demo the server
/// commands open their reviews instead of saying that host data is missing.
fn populate_hosts(inventory: &mut Inventory) {
    let hosts: Vec<_> = inventory.hosts.keys().copied().collect();
    for (index, id) in hosts.into_iter().enumerate() {
        let metrics = host_metrics_id(index);
        let control = control_domain_id(index);
        inventory.details.host_metrics.insert(
            metrics,
            HostMetrics {
                id: metrics,
                live: Some(true),
                memory_total: Some(64 * 1024 * 1024 * 1024),
                memory_free: Some(32 * 1024 * 1024 * 1024),
                last_updated: None,
            },
        );
        inventory.details.hosts.insert(
            id,
            HostDetails {
                allowed_operations: Some(vec![
                    "enable".into(),
                    "evacuate".into(),
                    "shutdown".into(),
                    "reboot".into(),
                    "power_on".into(),
                ]),
                current_operations: Some(vec![]),
                maintenance_mode: Some(false),
                boot_time: Some("100".into()),
                agent_start_time: Some("110".into()),
                metrics: Some(metrics),
                control_domain: Some(control),
                power_on_mode: Some(String::new()),
                power_on_config_empty: Some(true),
                syslog_known: true,
                multipathing: Some(false),
                ..Default::default()
            },
        );
        inventory.details.vms.insert(
            control,
            VmDetails {
                memory_static_min: Some(1024 * 1024),
                memory_static_max: Some(2 * 1024 * 1024 * 1024),
                memory_dynamic_min: Some(2 * 1024 * 1024 * 1024),
                memory_dynamic_max: Some(2 * 1024 * 1024 * 1024),
                ..Default::default()
            },
        );
    }
    for class in ["task", "pbd"] {
        inventory
            .details
            .classes
            .insert(class.into(), ClassAvailability::Available);
    }
}

pub fn populate(inventory: &mut Inventory) {
    inventory.details.pool = Some(xenadmin_core::PoolDetails {
        administration: xenadmin_core::PoolAdministration {
            coordinator: inventory.hosts.keys().next().copied(),
            ha_enabled: Some(false),
            ha_failures_to_tolerate: Some(0),
            migration_compression: Some(false),
            https_only: Some(false),
            tls_verification_enabled: Some(false),
            secret_rotation_pending: Some(false),
            ..Default::default()
        },
        auto_poweron: Some(false),
        ha_enabled: Some(false),
        igmp_snooping_enabled: Some(false),
        id: inventory.pool_id,
        name: inventory.pool_name.clone(),
        description: "Synthetic pool for isolated UI tests".into(),
        default_sr: inventory.storage.first().map(|sr| SrId::from(sr.uuid)),
        default_sr_known: true,
    });
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
                topology_known: true,
                cores_per_socket: Some("1".into()),
                cpu_weight_known: true,
                cpu_weight: Some("256".into()),
                shadow_multiplier: Some("1".into()),
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
                secure_boot: Some(false),
                bios_identity_empty: Some(true),
                affinity: vm.host.or(inventory.coordinator),
                affinity_known: true,
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
                read_cache_override: None,
                read_cache_policy_known: true,
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
                purposes: Some(Default::default()),
                automatic: None,
                automatic_known: true,
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
    populate_hosts(inventory);
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
            qos: Some(xenadmin_core::DeviceQos {
                algorithm: "ionice".into(),
                parameters: std::collections::BTreeMap::from([
                    ("class".into(), "4".into()),
                    ("sched".into(), "be".into()),
                ]),
            }),
            qos_supported_algorithms: Some(vec!["ionice".into()]),
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
            qos: Some(xenadmin_core::DeviceQos {
                algorithm: "ionice".into(),
                parameters: std::collections::BTreeMap::from([
                    ("class".into(), "4".into()),
                    ("sched".into(), "be".into()),
                ]),
            }),
            qos_supported_algorithms: Some(vec!["ionice".into()]),
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
            qos_known: true,
            qos_supported_algorithms: Some(vec!["ratelimit".into()]),
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
