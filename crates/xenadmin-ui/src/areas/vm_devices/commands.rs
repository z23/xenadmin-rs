//! The command table of the area.
//!
//! Three kinds of target:
//!
//! - the VM whose Storage or Networking tab is showing (the scope): the
//!   commands that add a device;
//! - the selected row of one of those tabs, which is an attachment or an
//!   interface and not a tree object: the area's own selection
//!   ([`super::selected_attachment`], [`super::selected_interface`]);
//! - a virtual disk: the grid selection of a repository's Storage tab, a
//!   disk in the Objects view, or the disk of the selected attachment.
//!
//! Eligibility is what the Rune buttons applied, with the caption the page
//! showed under a disabled button as the reason. A command is `Hidden` for
//! a scope that is not a VM (a template's tabs are information only) and
//! for a row of the other kind (Insert ISO on a disk); with no row selected
//! the row commands are disabled with [`SELECT_DISK`] or [`SELECT_NIC`].

use std::sync::Arc;

use xenadmin_core::{
    Inventory, PoolId, SrId, VdiId, VirtualBlockDevice, VirtualInterface, VmId,
};

use super::disks::Mode;
use super::lifecycle::Kind;
use super::{actions, disks, lifecycle, properties};
use crate::prelude::*;

pub const NEW: CommandId = CommandId("vm.disk.new");
pub const ATTACH: CommandId = CommandId("vm.disk.attach");
pub const ADD_CD: CommandId = CommandId("vm.cd.add");
pub const DETACH: CommandId = CommandId("vm.disk.detach");
pub const DELETE: CommandId = CommandId("vm.disk.delete");
pub const ACTIVATE: CommandId = CommandId("vm.disk.activate");
pub const DEACTIVATE: CommandId = CommandId("vm.disk.deactivate");
pub const INSERT: CommandId = CommandId("vm.cd.insert");
pub const EJECT: CommandId = CommandId("vm.cd.eject");
pub const DISK_PROPERTIES: CommandId = CommandId("vm.disk.properties");
pub const GROW: CommandId = CommandId("disk.grow");
pub const COPY: CommandId = CommandId("disk.copy");
pub const CLEANUP: CommandId = CommandId("disk.cleanup");
pub const NIC_ADD: CommandId = CommandId("vm.nic.add");
pub const NIC_PROPERTIES: CommandId = CommandId("vm.nic.properties");
pub const NIC_RATE: CommandId = CommandId("vm.nic.rate-limit");
pub const NIC_ACTIVATE: CommandId = CommandId("vm.nic.activate");
pub const NIC_DEACTIVATE: CommandId = CommandId("vm.nic.deactivate");
pub const NIC_REMOVE: CommandId = CommandId("vm.nic.remove");

/// The menu of a row of a VM's Storage tab.
pub const DISK_ROW_MENU: [CommandId; 10] = [
    DETACH,
    ACTIVATE,
    DEACTIVATE,
    INSERT,
    EJECT,
    DISK_PROPERTIES,
    GROW,
    COPY,
    CLEANUP,
    DELETE,
];

/// The menu of a row of a VM's Networking tab.
pub const NIC_ROW_MENU: [CommandId; 5] = [
    NIC_PROPERTIES,
    NIC_RATE,
    NIC_ACTIVATE,
    NIC_DEACTIVATE,
    NIC_REMOVE,
];

pub const SELECT_DISK: &str = "Select a disk or drive in the list.";
pub const SELECT_NIC: &str = "Select a network interface in the list.";
pub const SELECT_VIRTUAL_DISK: &str = "Select a virtual disk in the list.";

pub fn commands() -> Vec<CommandSpec> {
    let area = "vm_devices";
    vec![
        CommandSpec::new(NEW.0, "New Virtual Disk", area)
            .menu(place::STORAGE_NEW_VIRTUAL_DISK)
            .local(TabId::Storage, 10)
            .local_scopes(&[ObjectKind::Vm])
            .eligibility(add_eligibility)
            .invoke(|m, t| add(m, t, Kind::CreateDisk)),
        CommandSpec::new(ATTACH.0, "Attach Virtual Disk", area)
            .menu(place::STORAGE_ATTACH_VIRTUAL_DISK)
            .local(TabId::Storage, 12)
            .local_scopes(&[ObjectKind::Vm])
            .eligibility(add_eligibility)
            .invoke(|m, t| add(m, t, Kind::AttachDisk)),
        CommandSpec::new(ADD_CD.0, "Add CD/DVD Drive", area)
            .local(TabId::Storage, 14)
            .local_scopes(&[ObjectKind::Vm])
            .eligibility(add_eligibility)
            .invoke(|m, t| add(m, t, Kind::AddCd)),
        CommandSpec::new(DETACH.0, "Detach", area)
            .local(TabId::Storage, 20)
            .local_scopes(&[ObjectKind::Vm])
            .eligibility(|m, t| removal_eligibility(m, t, false))
            .invoke(|m, t| remove(m, t, false)),
        CommandSpec::new(ACTIVATE.0, "Activate", area)
            .local(TabId::Storage, 22)
            .local_scopes(&[ObjectKind::Vm])
            .eligibility(|m, t| plug_eligibility(m, t, false))
            .invoke(plug),
        CommandSpec::new(DEACTIVATE.0, "Deactivate", area)
            .local(TabId::Storage, 24)
            .local_scopes(&[ObjectKind::Vm])
            .eligibility(|m, t| plug_eligibility(m, t, true))
            .invoke(plug),
        CommandSpec::new(INSERT.0, "Insert ISO", area)
            .local(TabId::Storage, 26)
            .local_scopes(&[ObjectKind::Vm])
            .eligibility(insert_eligibility)
            .invoke(|m, t| {
                if let Some((c, row)) = disk_row(m, t) {
                    actions::open(m, c.pool, c.vm, None, Some(row.id));
                }
            }),
        CommandSpec::new(EJECT.0, "Eject Media", area)
            .local(TabId::Storage, 28)
            .local_scopes(&[ObjectKind::Vm])
            .eligibility(eject_eligibility)
            .invoke(|m, t| {
                if let Some((c, row)) = disk_row(m, t) {
                    let operation = xenadmin_core::DeviceOperation::EjectCd { vbd: row.id };
                    actions::open(m, c.pool, c.vm, Some(operation), None);
                }
            }),
        CommandSpec::new(DISK_PROPERTIES.0, "Properties", area)
            .local(TabId::Storage, 30)
            .local_scopes(&[ObjectKind::Vm])
            .eligibility(disk_properties_eligibility)
            .invoke(|m, t| {
                if let Some((c, row)) = disk_row(m, t) {
                    properties::open(m, c.pool, c.vm, Some(row.id), None);
                }
            }),
        // These three are also on a repository's own Storage tab, whose
        // repository commands have the orders 20 to 80: they stand after
        // them, together, and before the export of a disk (88).
        CommandSpec::new(GROW.0, "Resize", area)
            .local(TabId::Storage, 82)
            .local_scopes(&[ObjectKind::Vm, ObjectKind::StorageRepository])
            .context(&[ObjectKind::VirtualDisk])
            .eligibility(|m, t| disk_eligibility(m, t, Mode::Grow))
            .invoke(|m, t| open_disk(m, t, Mode::Grow)),
        CommandSpec::new(COPY.0, "Copy", area)
            .local(TabId::Storage, 84)
            .local_scopes(&[ObjectKind::Vm, ObjectKind::StorageRepository])
            .context(&[ObjectKind::VirtualDisk])
            .eligibility(|m, t| disk_eligibility(m, t, Mode::Copy))
            .invoke(|m, t| open_disk(m, t, Mode::Copy)),
        CommandSpec::new(CLEANUP.0, "Delete Owned Disk", area)
            .local(TabId::Storage, 86)
            .local_scopes(&[ObjectKind::Vm, ObjectKind::StorageRepository])
            .context(&[ObjectKind::VirtualDisk])
            .eligibility(|m, t| disk_eligibility(m, t, Mode::Cleanup))
            .invoke(|m, t| open_disk(m, t, Mode::Cleanup)),
        CommandSpec::new(DELETE.0, "Delete", area)
            .local(TabId::Storage, 38)
            .local_scopes(&[ObjectKind::Vm])
            .eligibility(|m, t| removal_eligibility(m, t, true))
            .invoke(|m, t| remove(m, t, true)),
        CommandSpec::new(NIC_ADD.0, "Add", area)
            .local(TabId::Networking, 10)
            .local_scopes(&[ObjectKind::Vm])
            .eligibility(add_eligibility)
            .invoke(|m, t| add(m, t, Kind::AddNic)),
        CommandSpec::new(NIC_PROPERTIES.0, "Properties", area)
            .local(TabId::Networking, 20)
            .local_scopes(&[ObjectKind::Vm])
            .eligibility(nic_edit_eligibility)
            .invoke(|m, t| {
                if let Some((c, row)) = nic_row(m, t)
                    && let Some(kind) = lifecycle::edit_kind(&row)
                {
                    lifecycle::open(m, c.pool, c.vm, kind);
                }
            }),
        CommandSpec::new(NIC_RATE.0, "Rate Limit", area)
            .local(TabId::Networking, 30)
            .local_scopes(&[ObjectKind::Vm])
            .eligibility(nic_rate_eligibility)
            .invoke(|m, t| {
                if let Some((c, row)) = nic_row(m, t) {
                    properties::open_rate(m, c.pool, c.vm, row.id);
                }
            }),
        CommandSpec::new(NIC_ACTIVATE.0, "Activate", area)
            .local(TabId::Networking, 40)
            .local_scopes(&[ObjectKind::Vm])
            .eligibility(|m, t| nic_plug_eligibility(m, t, false))
            .invoke(nic_plug),
        CommandSpec::new(NIC_DEACTIVATE.0, "Deactivate", area)
            .local(TabId::Networking, 50)
            .local_scopes(&[ObjectKind::Vm])
            .eligibility(|m, t| nic_plug_eligibility(m, t, true))
            .invoke(nic_plug),
        CommandSpec::new(NIC_REMOVE.0, "Remove", area)
            .local(TabId::Networking, 90)
            .local_scopes(&[ObjectKind::Vm])
            .eligibility(nic_remove_eligibility)
            .invoke(|m, t| {
                if let Some((c, row)) = nic_row(m, t) {
                    let operation = xenadmin_core::DeviceOperation::DeleteNic { vif: row.id };
                    lifecycle::open(m, c.pool, c.vm, Kind::Fixed(operation));
                }
            }),
    ]
}

/// The VM a command acts on, with the inventory of its pool.
pub struct Subject {
    pub pool: PoolId,
    pub vm: VmId,
    pub inventory: Arc<Inventory>,
}

/// The VM of the target: a VM (never a template) its pool's inventory has.
pub fn subject(m: &Model, target: &Target) -> Option<Subject> {
    let object = target.single_of(ObjectKind::Vm)?;
    let pool = object.pool?;
    let vm = object.vm_id()?;
    let inventory = m.inventory_for(pool)?;
    inventory.vms.contains_key(&vm).then_some(Subject {
        pool,
        vm,
        inventory,
    })
}

/// The selected attachment of the subject's Storage tab.
fn disk_row(m: &Model, target: &Target) -> Option<(Subject, VirtualBlockDevice)> {
    let c = subject(m, target)?;
    let row = m.areas.vm_devices.disk_row?;
    let record = c
        .inventory
        .details
        .vbds
        .get(&row)
        .filter(|vbd| vbd.vm == Some(c.vm))?
        .clone();
    Some((c, record))
}

/// The selected interface of the subject's Networking tab.
fn nic_row(m: &Model, target: &Target) -> Option<(Subject, VirtualInterface)> {
    let c = subject(m, target)?;
    let row = m.areas.vm_devices.nic_row?;
    let record = c
        .inventory
        .details
        .vifs
        .get(&row)
        .filter(|vif| vif.vm == Some(c.vm))?
        .clone();
    Some((c, record))
}

fn add_eligibility(m: &Model, target: &Target) -> Eligibility {
    match subject(m, target) {
        None => Eligibility::Hidden,
        Some(c) => Eligibility::enabled_if(
            lifecycle::can_open(m, c.pool, &c.inventory, c.vm),
            lifecycle::NOT_EDITABLE,
        ),
    }
}

fn add(m: &mut Model, target: &Target, kind: Kind) {
    if let Some(c) = subject(m, target) {
        lifecycle::open(m, c.pool, c.vm, kind);
    }
}

fn removal_eligibility(m: &Model, target: &Target, delete: bool) -> Eligibility {
    if subject(m, target).is_none() {
        return Eligibility::Hidden;
    }
    let Some((c, row)) = disk_row(m, target) else {
        return Eligibility::disabled(SELECT_DISK);
    };
    match lifecycle::disk_operation(&row, delete) {
        None => Eligibility::Hidden,
        Some(operation) => lifecycle::fixed_control(m, c.pool, &c.inventory, c.vm, &operation),
    }
}

fn remove(m: &mut Model, target: &Target, delete: bool) {
    if let Some((c, row)) = disk_row(m, target)
        && let Some(operation) = lifecycle::disk_operation(&row, delete)
    {
        lifecycle::open(m, c.pool, c.vm, Kind::Fixed(operation));
    }
}

/// Connect or disconnect a disk: the one of the two that fits the row.
fn plug_eligibility(m: &Model, target: &Target, attached: bool) -> Eligibility {
    if subject(m, target).is_none() {
        return Eligibility::Hidden;
    }
    let Some((c, row)) = disk_row(m, target) else {
        return if attached {
            Eligibility::Hidden
        } else {
            Eligibility::disabled(SELECT_DISK)
        };
    };
    if actions::is_cd(&row) || (row.currently_attached == Some(true)) != attached {
        return Eligibility::Hidden;
    }
    actions::control(m, c.pool, &c.inventory, c.vm, &actions::disk_plug(&row))
}

fn plug(m: &mut Model, target: &Target) {
    if let Some((c, row)) = disk_row(m, target) {
        actions::open(m, c.pool, c.vm, Some(actions::disk_plug(&row)), None);
    }
}

fn insert_eligibility(m: &Model, target: &Target) -> Eligibility {
    let Some((c, row)) = disk_row(m, target) else {
        return Eligibility::Hidden;
    };
    if !actions::is_cd(&row) || row.empty != Some(true) {
        return Eligibility::Hidden;
    }
    if !actions::permitted(&row.allowed_operations, "insert") {
        return Eligibility::disabled(actions::INSERT_REFUSED);
    }
    Eligibility::enabled_if(
        actions::ready(m, c.pool) && super::observed(m, c.pool, c.vm).is_some(),
        actions::RECONNECT,
    )
}

fn eject_eligibility(m: &Model, target: &Target) -> Eligibility {
    let Some((c, row)) = disk_row(m, target) else {
        return Eligibility::Hidden;
    };
    if !actions::is_cd(&row) || row.empty == Some(true) {
        return Eligibility::Hidden;
    }
    let operation = xenadmin_core::DeviceOperation::EjectCd { vbd: row.id };
    actions::control(m, c.pool, &c.inventory, c.vm, &operation)
}

fn disk_properties_eligibility(m: &Model, target: &Target) -> Eligibility {
    if subject(m, target).is_none() {
        return Eligibility::Hidden;
    }
    let Some((c, row)) = disk_row(m, target) else {
        return Eligibility::disabled(SELECT_DISK);
    };
    if row.kind != "Disk" {
        return Eligibility::Hidden;
    }
    if !m.is_synthetic_pool(c.pool) && !m.can_mutate_pool(c.pool) {
        return Eligibility::disabled(actions::RECONNECT);
    }
    Eligibility::enabled_if(
        row.vdi
            .is_some_and(|id| c.inventory.details.vdis.contains_key(&id)),
        properties::NO_DISK_RECORD,
    )
}

fn nic_edit_eligibility(m: &Model, target: &Target) -> Eligibility {
    let Some(c) = subject(m, target) else {
        return Eligibility::Hidden;
    };
    let Some((_, row)) = nic_row(m, target) else {
        return Eligibility::disabled(SELECT_NIC);
    };
    if row.network.is_none() {
        return Eligibility::Hidden;
    }
    Eligibility::enabled_if(
        lifecycle::can_open(m, c.pool, &c.inventory, c.vm),
        lifecycle::NOT_EDITABLE,
    )
}

fn nic_rate_eligibility(m: &Model, target: &Target) -> Eligibility {
    let Some(c) = subject(m, target) else {
        return Eligibility::Hidden;
    };
    let Some((_, row)) = nic_row(m, target) else {
        return Eligibility::disabled(SELECT_NIC);
    };
    if !m.is_synthetic_pool(c.pool) && !m.can_mutate_pool(c.pool) {
        return Eligibility::disabled(actions::RECONNECT);
    }
    Eligibility::enabled_if(
        row.qos_known && matches!(row.qos_algorithm.as_str(), "" | "ratelimit"),
        properties::NO_RATE,
    )
}

fn nic_plug_eligibility(m: &Model, target: &Target, attached: bool) -> Eligibility {
    if subject(m, target).is_none() {
        return Eligibility::Hidden;
    }
    let Some((c, row)) = nic_row(m, target) else {
        return if attached {
            Eligibility::Hidden
        } else {
            Eligibility::disabled(SELECT_NIC)
        };
    };
    if (row.currently_attached == Some(true)) != attached {
        return Eligibility::Hidden;
    }
    actions::control(m, c.pool, &c.inventory, c.vm, &actions::nic_plug(&row))
}

fn nic_plug(m: &mut Model, target: &Target) {
    if let Some((c, row)) = nic_row(m, target) {
        actions::open(m, c.pool, c.vm, Some(actions::nic_plug(&row)), None);
    }
}

fn nic_remove_eligibility(m: &Model, target: &Target) -> Eligibility {
    if subject(m, target).is_none() {
        return Eligibility::Hidden;
    }
    let Some((c, row)) = nic_row(m, target) else {
        return Eligibility::disabled(SELECT_NIC);
    };
    let operation = xenadmin_core::DeviceOperation::DeleteNic { vif: row.id };
    lifecycle::fixed_control(m, c.pool, &c.inventory, c.vm, &operation)
}

/// The virtual disk a disk command acts on, or why there is none.
pub fn disk_subject(m: &Model, target: &Target) -> Result<(PoolId, VdiId), Eligibility> {
    if let Some(object) = target.single_of(ObjectKind::VirtualDisk) {
        return object
            .pool
            .zip(object.uuid())
            .map(|(pool, uuid)| (pool, VdiId::from(uuid)))
            .ok_or(Eligibility::Hidden);
    }
    if let Some(c) = subject(m, target) {
        let Some((_, row)) = disk_row(m, target) else {
            return Err(Eligibility::disabled(SELECT_DISK));
        };
        return row
            .vdi
            .filter(|_| !actions::is_cd(&row))
            .map(|disk| (c.pool, disk))
            .ok_or(Eligibility::Hidden);
    }
    if target.single_of(ObjectKind::StorageRepository).is_some() {
        return Err(Eligibility::disabled(SELECT_VIRTUAL_DISK));
    }
    Err(Eligibility::Hidden)
}

fn disk_eligibility(m: &Model, target: &Target, mode: Mode) -> Eligibility {
    let (pool, disk) = match disk_subject(m, target) {
        Ok(found) => found,
        Err(refusal) => return refusal,
    };
    let Some(inventory) = m.inventory_for(pool) else {
        return Eligibility::Hidden;
    };
    if !inventory.details.vdis.contains_key(&disk) {
        return Eligibility::Hidden;
    }
    Eligibility::enabled_unless(disks::blocked(m, pool, &inventory, disk, mode))
}

fn open_disk(m: &mut Model, target: &Target, mode: Mode) {
    let Ok((pool, disk)) = disk_subject(m, target) else {
        return;
    };
    // A copy made from a repository's list goes to that repository unless
    // another one is picked (Rune preset the destination to the list's).
    let destination = m
        .scope()
        .single_of(ObjectKind::StorageRepository)
        .and_then(|object| object.uuid())
        .map(SrId::from);
    disks::open(m, pool, disk, mode, destination);
}
