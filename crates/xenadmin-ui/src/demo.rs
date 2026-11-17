//! The synthetic pool used by the demo and large-inventory scenes, the
//! saved connection that stands for it, and the scenes the kernel owns.
//!
//! [`demonstration`] builds the inventory; detail records are filled by
//! [`crate::populate_demo_details`] afterwards, as the Rune scene does.
//! [`crate::Model::load_demo`] is the Rune `demo_sized`: it makes the pool a
//! synthetic session and selects it. [`crate::Model::demo`] is a fixture with
//! it loaded.

use std::collections::BTreeMap;

use crate::command::KERNEL;
use crate::model::Model;
use crate::object::{ObjectRef, Target};
use crate::scene::SceneSpec;
use xenadmin_core::{
    ConnectionProfile, Endpoint, Host, HostId, Inventory, Network, PoolId, PowerState, ProfileId,
    Snapshot, SnapshotAction, SnapshotId, SnapshotInventory, Storage, Vm, VmId,
};

/// The id of the saved connection that stands for the synthetic pool.
pub const PROFILE: &str = "d0000000-0000-0000-0000-000000000001";

/// The saved connection of the synthetic pool: a preview profile with a
/// fixed id. It is never stored and can never connect.
pub fn profile() -> ConnectionProfile {
    let mut profile = ConnectionProfile::new(
        "Demonstration pool".into(),
        Endpoint::parse("https://demonstration.invalid").expect("demo endpoint"),
        "root".into(),
    );
    profile.id = profile_id();
    profile
}

/// [`PROFILE`] as a profile id.
pub fn profile_id() -> ProfileId {
    crate::session::profile_id(PROFILE).expect("demo profile id")
}

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

// -- The kernel's scenes ------------------------------------------------------

/// The Rune `demo`: three hosts and 48 guests.
fn demo(m: &mut Model) {
    m.load_demo(3, 48);
}

/// A pool larger than the lab's, for measuring the interface.
fn large_inventory(m: &mut Model) {
    m.load_demo(16, 2000);
}

/// One host outside the qualified release: the status bar says so.
fn host_version_notice(m: &mut Model) {
    demo(m);
    if let Some(inventory) = m.inventory.take() {
        let mut inventory = inventory.as_ref().clone();
        if let Some(host) = inventory.hosts.values_mut().next() {
            host.version = "8.2.1".into();
        }
        m.inventory = Some(std::sync::Arc::new(inventory));
    }
}

/// The interface at 125 percent.
fn interface_large(m: &mut Model) {
    demo(m);
    m.preferences.options.interface_scale = 125;
}

/// An inventory that is not a demo and has no connection: every action is
/// unavailable, each with its reason. A running guest is selected.
fn unavailable_actions(m: &mut Model) {
    demo(m);
    m.demo = false;
    m.error = Some("Synthetic unavailable-action preview • no host connection".into());
    if let Some(inventory) = m.inventory.clone()
        && let Ok(vm) = VmId::parse("30000000-0000-0000-0000-000000000001")
    {
        m.select(Target::one(ObjectRef::vm(inventory.pool_id, vm)));
    }
}

/// The scenes the kernel owns, under their Rune names. The other scenes of
/// the Rune kernel modules belong to the `general`, `search`, `settings` and
/// `connections` areas.
pub fn scenes() -> Vec<SceneSpec> {
    vec![
        SceneSpec::new("demo", KERNEL, demo),
        SceneSpec::new("large-inventory", KERNEL, large_inventory),
        SceneSpec::new("host-version-notice", KERNEL, host_version_notice),
        SceneSpec::new("interface-large", KERNEL, interface_large),
        SceneSpec::new("unavailable-actions", KERNEL, unavailable_actions),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::ObjectKind;
    use crate::scene::apply;
    use crate::tabs::TabId;

    #[test]
    fn the_kernel_scenes_set_up_the_demo_inventory() {
        let mut m = Model::fixture();
        assert!(apply("demo", &mut m));
        assert!(m.demo);
        let inventory = m.inventory.clone().expect("inventory");
        assert_eq!((inventory.hosts.len(), inventory.vms.len()), (3, 48));
        assert!(inventory.details.pool.is_some(), "details are populated");
        assert_eq!(m.scope().kind(), ObjectKind::Pool);
        assert_eq!(m.status().demo, Some(crate::model::DEMO_BANNER));

        let mut m = Model::fixture();
        assert!(apply("large-inventory", &mut m));
        let inventory = m.inventory.clone().expect("inventory");
        assert_eq!((inventory.hosts.len(), inventory.vms.len()), (16, 2000));
        // Hosts and guests are all in the tree, under one pool.
        let rows = crate::nav::rows(&m);
        assert_eq!(
            rows.iter().filter(|row| row.icon == ObjectKind::Vm).count(),
            2000
        );
        assert_eq!(
            rows.iter()
                .filter(|row| row.icon == ObjectKind::Host)
                .count(),
            16
        );

        let mut m = Model::fixture();
        assert!(apply("host-version-notice", &mut m));
        let inventory = m.inventory.clone().expect("inventory");
        assert_eq!(
            inventory
                .hosts
                .values()
                .next()
                .map(|host| host.version.as_str()),
            Some("8.2.1")
        );
        assert_eq!(
            m.status().host_notice.as_deref(),
            Some(
                "Not qualified on this host version: xcp-lab-01 (8.2.1). This client has only been run against XCP-ng 8.3. Looking is safe; treat any change here as untested."
            )
        );

        let mut m = Model::fixture();
        assert!(apply("interface-large", &mut m));
        assert!(m.demo);
        assert_eq!(m.preferences.options.interface_scale, 125);
        assert_eq!(m.preferences.options.zoom(), 1.25);

        let mut m = Model::fixture();
        assert!(apply("unavailable-actions", &mut m));
        assert!(!m.demo);
        assert_eq!(
            m.status().error.as_deref(),
            Some("Synthetic unavailable-action preview • no host connection")
        );
        assert_eq!(m.selection.scope.vms().len(), 1);
        assert_eq!(m.active_tab(), TabId::General);
        assert_eq!(m.header().state.as_deref(), Some("Running"));
        assert!(!m.can_mutate());
        assert_eq!(m.status().connection, "No host connected");
        assert_eq!(m.status().demo, None);

        assert!(!apply("no-such-scene", &mut m));
    }

    #[test]
    fn the_five_kernel_scenes_are_registered_under_their_rune_names() {
        let names: Vec<_> = scenes().iter().map(|scene| scene.name).collect();
        assert_eq!(
            names,
            [
                "demo",
                "large-inventory",
                "host-version-notice",
                "interface-large",
                "unavailable-actions"
            ]
        );
        for name in names {
            assert!(crate::scene::rune_app_scene(name).is_some(), "{name}");
            assert_eq!(
                crate::scene::find(name).map(|scene| scene.area),
                Some(KERNEL)
            );
        }
    }

    #[test]
    fn the_demo_profile_has_a_fixed_id_and_cannot_be_mistaken_for_a_host() {
        let profile = profile();
        assert_eq!(profile.id.to_string(), PROFILE);
        assert_eq!(profile.id, profile_id());
        assert_eq!(profile.name, "Demonstration pool");
        assert!(profile.endpoint.as_str().contains("demonstration.invalid"));
    }
}
