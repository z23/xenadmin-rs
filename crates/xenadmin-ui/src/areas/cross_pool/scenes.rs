//! The scenes of the area, under their Rune names.
//!
//! Both start from the fixture of the migration scenes
//! (`migration::scenes::prepare`, the Rune `vm_migration::scene`) and move
//! guests of the demo pool to a second synthetic pool that no session
//! shows ([`scene_destination`]).

use std::sync::Arc;

use uuid::Uuid;
use xenadmin_core::{Inventory, NetworkId, PoolId, PowerState, SrId, VbdId, VdiId, VifId, VmId};

use super::commands::MOVE_TO_POOL;
use super::single::{self, Peer};
use super::{batch, view};
use crate::areas::migration::scenes::{amend, demo_vm, prepare, second_demo_vm};
use crate::prelude::*;

pub fn scenes() -> Vec<SceneSpec> {
    vec![
        SceneSpec::new(view::WIZARD, "cross_pool", single_scene),
        SceneSpec::new(view::BATCH, "cross_pool", batch_scene),
    ]
}

/// A second synthetic pool for renders and tests: the given pool's hosts,
/// repositories and networks under another identity, with no guests.
pub fn scene_destination(source: &Inventory) -> Peer {
    let mut destination = source.clone();
    destination.pool_id = PoolId::from(Uuid::from_u128(85001));
    destination.pool_name = "Destination lab pool".into();
    destination.vms.clear();
    destination.details.vms.clear();
    destination.details.vdis.clear();
    destination.details.vbds.clear();
    destination.details.vifs.clear();
    destination.details.vdi_attachments.clear();
    Peer {
        profile: None,
        observed: None,
        inventory: Arc::new(destination),
    }
}

/// What the Rune `cross_pool_vm::scene` did to the pool before it built
/// its editor: the first guest without HA, schedule or vApp and with a
/// known home host, every disk plain, every repository for user disks.
/// The guest is selected and returned.
pub(crate) fn fixture(m: &mut Model) -> Option<ObjectRef> {
    let object = prepare(m)?;
    let id = demo_vm();
    amend(m, |i| {
        if let Some(details) = i.details.vms.get_mut(&id) {
            details.ha_restart_priority = Some(String::new());
            details.snapshot_policy = None;
            details.appliance = None;
            details.appliance_known = true;
            details.affinity_known = true;
        }
        for disk in i.details.vdis.values_mut() {
            disk.sharable = Some(false);
            disk.read_only = Some(false);
            disk.is_snapshot = Some(false);
            disk.cbt_enabled = Some(false);
            disk.managed = Some(true);
            disk.missing = Some(false);
        }
        for sr in i.details.storage.values_mut() {
            sr.content_type = "user".into();
            sr.allowed_operations = Some(vec!["vdi_create".into()]);
        }
    });
    Some(object)
}

/// The review of a live move to the first host of the destination pool,
/// acknowledged. The wizard does not open on a synthetic pool (the command
/// is disabled there, as the Rune button was): the editor is built as the
/// Rune scene built it.
fn single_scene(m: &mut Model) {
    if fixture(m).is_none() {
        return;
    }
    let Some(i) = m.inventory.clone() else { return };
    let Some(vm) = i.vms.get(&demo_vm()).cloned() else {
        return;
    };
    let source = Peer {
        profile: None,
        observed: None,
        inventory: i.clone(),
    };
    let mut e = single::editor(source, None, vm, vec![scene_destination(&i)]);
    e.destination = Some(0);
    let dest = e.peers[0].inventory.clone();
    e.host = dest.hosts.keys().next().copied();
    e.transfer = dest
        .details
        .pifs
        .values()
        .find(|p| p.host == e.host)
        .and_then(|p| Some((p.network?, p.ip.clone())));
    e.transport_reviewed = true;
    e.stage = 2;
    e.review = single::operation(&e).ok();
    m.areas.cross_pool.wizard = Some(Box::new(e));
}

/// What the Rune `bulk_cross_pool::scene` did to the pool: [`fixture`],
/// then the second guest halted with no resident host, as plain as the
/// first, with a disk (8 GiB) and a NIC of its own. Both are selected and
/// returned.
pub(crate) fn batch_fixture(m: &mut Model) -> Option<Target> {
    let first = fixture(m)?;
    let (id, second) = (demo_vm(), second_demo_vm());
    amend(m, |i| {
        // The second VM is halted, with no resident host, and otherwise as plain as the first.
        if let Some(halted) = i.vms.get_mut(&second) {
            halted.power_state = PowerState::Halted;
            halted.host = None;
        }
        if let Some(details) = i.details.vms.get_mut(&second) {
            details.ha_restart_priority = Some(String::new());
            details.snapshot_policy = None;
            details.appliance = None;
            details.appliance_known = true;
            details.affinity_known = true;
            details.blocked_operations.remove("migrate_send");
            details.current_operations.clear();
            details.allowed_operations = Some(vec!["migrate_send".into()]);
        }
        // The synthetic inventory gives only the first VM devices. The second gets a disk
        // and a NIC of its own, so that the review has a mapping to list for each.
        let block = i
            .details
            .vbds
            .values()
            .find(|b| b.vm == Some(id) && b.kind == "Disk")
            .cloned();
        if let Some(block) = block
            && let Some(mut disk) = block.vdi.and_then(|vdi| i.details.vdis.get(&vdi).cloned())
        {
            disk.id = VdiId::from(Uuid::from_u128(0x7100_0000_0000_0000_0000_0000_0000_0002));
            disk.name = "Demo data disk".into();
            disk.virtual_size = Some(8 << 30);
            let mut attachment = block;
            attachment.id =
                VbdId::from(Uuid::from_u128(0x7200_0000_0000_0000_0000_0000_0000_0011));
            attachment.vm = Some(second);
            attachment.vdi = Some(disk.id);
            attachment.currently_attached = Some(false);
            i.details
                .vdi_attachments
                .insert(disk.id, vec![attachment.id]);
            i.details.vdis.insert(disk.id, disk);
            i.details.vbds.insert(attachment.id, attachment);
        }
        let nic = i.details.vifs.values().find(|n| n.vm == Some(id)).cloned();
        if let Some(mut nic) = nic {
            nic.id = VifId::from(Uuid::from_u128(0x7300_0000_0000_0000_0000_0000_0000_0002));
            nic.vm = Some(second);
            nic.mac = "02:00:00:00:00:24".into();
            nic.currently_attached = Some(false);
            i.details.vifs.insert(nic.id, nic);
        }
    });
    let pool = first.pool?;
    m.inventory.as_ref()?.vms.get(&second)?;
    let target = Target::many([first, ObjectRef::vm(pool, second)]);
    m.select(target.clone());
    Some(target)
}

/// The review of a running and a halted VM moving to the destination pool,
/// acknowledged: the command on the two guests, then the draft written as
/// the Rune scene wrote it and the review asked for.
fn batch_scene(m: &mut Model) {
    let Some(target) = batch_fixture(m) else {
        return;
    };
    m.invoke(MOVE_TO_POOL, &target);
    let Some(i) = m.inventory.clone() else { return };
    let ids: [VmId; 2] = [demo_vm(), second_demo_vm()];
    let Some(e) = m.areas.cross_pool.batch.as_mut() else {
        return;
    };
    e.source.inventory = i.clone();
    e.choices = ids
        .iter()
        .filter_map(|id| i.vms.get(id).cloned())
        .map(|vm| batch::choice(&i, vm))
        .collect();
    e.peers = vec![scene_destination(&i)];
    e.destination = Some(0);
    let dest = e.peers[0].inventory.clone();
    e.host = dest.hosts.values().find(|h| h.enabled).map(|h| h.id);
    e.repository = dest
        .storage
        .iter()
        .map(|sr| SrId::from(sr.uuid))
        .find(|id| {
            dest.details.pbds.values().any(|p| {
                p.sr == Some(*id) && p.host == e.host && p.currently_attached == Some(true)
            })
        });
    let network = dest.networks.first().map(|n| NetworkId::from(n.uuid));
    for source in batch::source_networks(e) {
        if let Some(network) = network {
            e.networks.insert(source, network);
        }
    }
    e.transfer = dest
        .details
        .pifs
        .values()
        .find(|p| p.host == e.host && p.currently_attached == Some(true))
        .and_then(|p| Some((p.network?, p.ip.clone())));
    e.stage = 1;
    batch::next(m);
    if m.areas
        .cross_pool
        .batch
        .as_ref()
        .is_some_and(|e| e.review.is_some())
    {
        batch::acknowledge(m);
    }
}
