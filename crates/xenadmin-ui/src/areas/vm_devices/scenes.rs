//! The scenes of the area, under their Rune names.

use std::sync::Arc;

use xenadmin_core::{
    DeviceProperty, DiskOperation, Inventory, OperationId, OwnedDiskSource, VbdId, VmId,
};

use super::disks::{self, Mode};
use super::properties;
use crate::prelude::*;

/// The halted demo guest that has a disk, a CD/DVD drive and an interface.
pub const VM: u128 = 0x3000_0000_0000_0000_0000_0000_0000_0000;
/// The attachment of its data disk.
pub const DISK_ATTACHMENT: u128 = 0x7200_0000_0000_0000_0000_0000_0000_0001;

pub fn scenes() -> Vec<SceneSpec> {
    vec![
        SceneSpec::new("vm-devices", "vm_devices", devices),
        SceneSpec::new("device-properties", "vm_devices", device_properties),
        SceneSpec::new("disk-grow", "vm_devices", |m| disk(m, false)),
        SceneSpec::new("disk-copy", "vm_devices", |m| disk(m, true)),
        SceneSpec::new("disk-cleanup", "vm_devices", disk_cleanup),
    ]
}

pub fn vm() -> VmId {
    VmId::from(uuid::Uuid::from_u128(VM))
}

pub fn disk_attachment() -> VbdId {
    VbdId::from(uuid::Uuid::from_u128(DISK_ATTACHMENT))
}

/// Replaces the scope session's inventory with an edited copy.
pub fn amend(m: &mut Model, change: impl FnOnce(&mut Inventory)) {
    let Some(inventory) = m.inventory.take() else {
        return;
    };
    let mut inventory = inventory.as_ref().clone();
    change(&mut inventory);
    m.inventory = Some(Arc::new(inventory));
}

/// The demo guest selected, on its Storage tab (the Rune
/// `vm_devices::open` after selecting the guest).
pub fn open(m: &mut Model) {
    let Some(pool) = m.inventory.as_ref().map(|inventory| inventory.pool_id) else {
        return;
    };
    m.select(Target::one(ObjectRef::vm(pool, vm())));
    m.set_active_tab(TabId::Storage);
}

/// `vm-devices`: the Rune Devices page, now the Storage tab of the guest.
fn devices(m: &mut Model) {
    m.load_demo(3, 48);
    open(m);
    m.areas.vm_devices.disk_row = Some(disk_attachment());
}

/// `device-properties`: the properties of the data disk's attachment, on
/// "I/O priority".
pub fn device_properties(m: &mut Model) {
    m.load_demo(3, 48);
    open(m);
    let Some(pool) = m.inventory.as_ref().map(|inventory| inventory.pool_id) else {
        return;
    };
    m.areas.vm_devices.disk_row = Some(disk_attachment());
    properties::open(m, pool, vm(), Some(disk_attachment()), None);
    let priority = m.areas.vm_devices.properties.as_ref().and_then(|f| {
        f.choices
            .iter()
            .find(|c| matches!(c, DeviceProperty::DiskPriority { .. }))
            .cloned()
    });
    if let Some(choice) = priority {
        properties::choose(m, choice);
    }
}

/// `disk-grow`, `disk-copy`: the review of a detached user disk.
pub fn disk(m: &mut Model, copy: bool) {
    m.load_demo(3, 48);
    let mut found = None;
    amend(m, |i| {
        let Some(disk) = i.details.vdis.values_mut().find(|d| d.kind == "user") else {
            return;
        };
        let id = disk.id;
        disk.cbt_enabled = Some(false);
        disk.allowed_operations = Some(vec!["resize".into(), "copy".into(), "destroy".into()]);
        found = Some((id, disk.sr));
        i.details.vbds.retain(|_, v| v.vdi != Some(id));
    });
    let Some((id, sr)) = found else {
        return;
    };
    let Some(pool) = m.inventory.as_ref().map(|inventory| inventory.pool_id) else {
        return;
    };
    let mode = if copy { Mode::Copy } else { Mode::Grow };
    if !disks::open(m, pool, id, mode, sr) {
        return;
    }
    if let Some(p) = &mut m.areas.vm_devices.disks {
        p.size = "64 GiB".into();
        p.name = "Fixture disk copy".into();
    }
    disks::review(m);
}

/// `disk-cleanup`: the cleanup review with the UUID confirmation.
pub fn disk_cleanup(m: &mut Model) {
    disk(m, true);
    if let Some(p) = &mut m.areas.vm_devices.disks
        && let Some(r) = &mut p.review
    {
        r.operation = DiskOperation::Cleanup {
            expected: r.operation.expected().clone(),
            source: OwnedDiskSource::ResourceOperation(OperationId::new()),
        };
        p.mode = Mode::Cleanup;
    }
}
