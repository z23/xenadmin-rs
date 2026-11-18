//! The scenes of the area, under their Rune names.
//!
//! Each sets the demo pool up as its Rune scene did and then goes the way
//! a user goes: the VMs selected, the command invoked. The draft is then
//! written as the Rune scene wrote it, and the review asked for.

use std::sync::Arc;

use uuid::Uuid;
use xenadmin_core::{
    ClassAvailability, Inventory, NetworkId, PbdId, PhysicalInterface, PifId, PowerState, SrId,
    StorageAttachment, VmId,
};

use super::commands::{MIGRATE, MOVE};
use super::{batch, wizard};
use crate::prelude::*;

pub fn scenes() -> Vec<SceneSpec> {
    vec![
        SceneSpec::new("vm-migration", "migration", migration_scene),
        SceneSpec::new("batch-move", "migration", batch_scene),
    ]
}

/// The first guest of the demo pool, which the scenes move.
pub fn demo_vm() -> VmId {
    VmId::from(Uuid::from_u128(0x3000_0000_0000_0000_0000_0000_0000_0000))
}

/// The second guest of the batch scene.
pub fn second_demo_vm() -> VmId {
    VmId::from(Uuid::from_u128(0x3000_0000_0000_0000_0000_0000_0000_0001))
}

/// Replaces the scope session's inventory with an edited copy.
pub(crate) fn amend(m: &mut Model, edit: impl FnOnce(&mut Inventory)) {
    if let Some(inventory) = m.inventory.take() {
        let mut inventory = inventory.as_ref().clone();
        edit(&mut inventory);
        m.inventory = Some(Arc::new(inventory));
    }
}

/// The fixture of the Rune `vm_migration::scene`, before anything opens:
/// the demo pool without snapshots; [`demo_vm`] running on the first host
/// and allowed to migrate; every repository attached to every host; a
/// managed interface with a static IPv4 address on every host for every
/// network. The VM is selected and returned. Public because the scene of
/// the `cross_pool` area starts from the same fixture, as its Rune scene
/// did.
pub fn prepare(m: &mut Model) -> Option<ObjectRef> {
    m.load_demo(3, 48);
    let id = demo_vm();
    amend(m, |i| {
        let hosts = i.hosts.keys().copied().collect::<Vec<_>>();
        i.snapshots = Default::default();
        if let Some(vm) = i.vms.get_mut(&id) {
            vm.power_state = PowerState::Running;
            vm.host = hosts.first().copied();
        }
        if let Some(details) = i.details.vms.get_mut(&id) {
            details.allowed_operations = Some(vec!["pool_migrate".into(), "migrate_send".into()]);
        }
        for class in ["pbd", "pif"] {
            i.details
                .classes
                .insert(class.into(), ClassAvailability::Available);
        }
        for (index, host) in hosts.iter().enumerate() {
            for (offset, sr) in i.storage.iter().enumerate() {
                let p = PbdId::from(Uuid::from_u128(900 + index as u128 * 100 + offset as u128));
                i.details.pbds.insert(
                    p,
                    StorageAttachment {
                        location: None,
                        requires_credentials: false,
                        multipathed: None,
                        paths: Vec::new(),
                        id: p,
                        host: Some(*host),
                        sr: Some(SrId::from(sr.uuid)),
                        currently_attached: Some(true),
                    },
                );
            }
            for (offset, n) in i.networks.iter().enumerate() {
                let p = PifId::from(Uuid::from_u128(9000 + index as u128 * 100 + offset as u128));
                i.details.pifs.insert(
                    p,
                    PhysicalInterface {
                        metrics: None,
                        capabilities: None,
                        managed: Some(true),
                        disallow_unplug: Some(false),
                        management_purpose: None,
                        primary_address_type: Some("IPv4".into()),
                        id: p,
                        host: Some(*host),
                        network: Some(NetworkId::from(n.uuid)),
                        device: "eth0".into(),
                        mac: "02:00:00:00:00:01".into(),
                        mtu: Some(1500),
                        vlan: Some(-1),
                        management: Some(true),
                        physical: Some(true),
                        currently_attached: Some(true),
                        ip_configuration_mode: "Static".into(),
                        ip: format!("192.0.2.{}", index + 10),
                        netmask: "255.255.255.0".into(),
                        gateway: String::new(),
                        dns: String::new(),
                        ipv6: vec![],
                        ipv6_configuration_mode: "None".into(),
                        ipv6_gateway: String::new(),
                    },
                );
            }
        }
    });
    let inventory = m.inventory.as_ref()?;
    inventory.vms.get(&id)?;
    let object = ObjectRef::vm(inventory.pool_id, id);
    m.select(Target::one(object.clone()));
    Some(object)
}

/// "Review migration" of a live move to the second host, with the first
/// network as the transfer network.
fn migration_scene(m: &mut Model) {
    let Some(object) = prepare(m) else { return };
    m.invoke(MIGRATE, &Target::one(object));
    let Some(f) = m.areas.migration.wizard.as_deref_mut() else {
        return;
    };
    f.destination = f.initial.hosts.keys().nth(1).copied();
    f.transfer = f.initial.networks.first().map(|n| NetworkId::from(n.uuid));
    f.stage = 1;
    wizard::next(m);
}

/// The fixture of the Rune `bulk_move::scene`, before anything opens:
/// [`prepare`], then [`second_demo_vm`] running on the same host and
/// allowed to migrate, and the home host of both known. Both are selected
/// and returned.
pub(crate) fn prepare_batch(m: &mut Model) -> Option<Target> {
    let first = prepare(m)?;
    let (id, second) = (demo_vm(), second_demo_vm());
    amend(m, |i| {
        let host = i.vms.get(&id).and_then(|vm| vm.host);
        if let Some(vm) = i.vms.get_mut(&second) {
            vm.power_state = PowerState::Running;
            vm.host = host;
        }
        if let Some(details) = i.details.vms.get_mut(&second) {
            details.allowed_operations = Some(vec!["pool_migrate".into(), "migrate_send".into()]);
        }
        for vm in [id, second] {
            if let Some(details) = i.details.vms.get_mut(&vm) {
                details.affinity_known = true;
            }
        }
    });
    let pool = first.pool?;
    m.inventory.as_ref()?.vms.get(&second)?;
    let target = Target::many([first, ObjectRef::vm(pool, second)]);
    m.select(target.clone());
    Some(target)
}

/// The review of two live moves to another host, with the first network
/// as the transfer network.
fn batch_scene(m: &mut Model) {
    let Some(target) = prepare_batch(m) else {
        return;
    };
    m.invoke(MOVE, &target);
    let Some(e) = m.areas.migration.batch.as_mut() else {
        return;
    };
    let host = e.choices.first().and_then(|c| c.source.host);
    e.destination = e.initial.hosts.keys().copied().find(|h| Some(*h) != host);
    e.transfer = e.initial.networks.first().map(|n| NetworkId::from(n.uuid));
    batch::review(m);
}
