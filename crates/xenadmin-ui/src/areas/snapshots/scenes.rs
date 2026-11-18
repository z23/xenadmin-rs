//! The scenes of the area, under their Rune names.
//!
//! Every one starts as the Rune fixtures did: the demo pool, the halted
//! demo guest `30000000-…-0000` selected, its Snapshots tab showing the
//! three demo snapshots. A dialog is opened by its command, so a scene also
//! shows that the command leads there.

use std::sync::Arc;

use xenadmin_core::{
    Inventory, PowerAction, PowerState, SnapshotDetails, SnapshotId, SnapshotParent, VmId,
};

use super::commands::{PROPERTIES, REVERT, TAKE};
use super::inspection::{self, Panel};
use crate::prelude::*;

/// The demo guest that has the snapshots.
pub const VM: u128 = 0x3000_0000_0000_0000_0000_0000_0000_0000;
/// The demo snapshots: "Before upgrade", "Known good", "Initial
/// installation".
pub const FIRST: u128 = 0x6000_0000_0000_0000_0000_0000_0000_0000;
pub const SECOND: u128 = 0x6000_0000_0000_0000_0000_0000_0000_0001;

pub fn scenes() -> Vec<SceneSpec> {
    vec![
        SceneSpec::new("snapshots", "snapshots", page),
        SceneSpec::new("snapshot-create", "snapshots", create),
        SceneSpec::new("snapshot-restore", "snapshots", restore),
        SceneSpec::new("snapshot-edit", "snapshots", edit),
        SceneSpec::new("snapshot-capture", "snapshots", capture),
        SceneSpec::new("snapshot-ancestry", "snapshots", ancestry),
        SceneSpec::new("snapshot-information", "snapshots", information),
    ]
}

pub fn vm() -> VmId {
    VmId::from(uuid::Uuid::from_u128(VM))
}

pub fn snapshot(id: u128) -> SnapshotId {
    SnapshotId::from(uuid::Uuid::from_u128(id))
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

/// The demo guest selected, on its Snapshots tab (the Rune
/// `vm_snapshots::open` after selecting the guest).
pub fn open(m: &mut Model) {
    let Some(pool) = m.inventory.as_ref().map(|inventory| inventory.pool_id) else {
        return;
    };
    m.select(Target::one(ObjectRef::vm(pool, vm())));
    m.set_active_tab(TabId::Snapshots);
}

/// Selects one row, as a press on it does.
pub fn select(m: &mut Model, id: SnapshotId) {
    super::edit::select(m, &[id.to_string()]);
}

fn page(m: &mut Model) {
    m.load_demo(3, 48);
    open(m);
}

fn create(m: &mut Model) {
    page(m);
    let target = m.target();
    m.invoke(TAKE, &target);
}

fn restore(m: &mut Model) {
    m.load_demo(3, 48);
    amend(m, |inventory| {
        if let Some(guest) = inventory.vms.get_mut(&vm()) {
            guest.power_state = PowerState::Running;
            guest.allowed_actions.push(PowerAction::CleanShutdown);
        }
    });
    open(m);
    select(m, snapshot(FIRST));
    let target = m.target();
    m.invoke(REVERT, &target);
}

fn edit(m: &mut Model) {
    page(m);
    select(m, snapshot(FIRST));
    let target = m.target();
    m.invoke(PROPERTIES, &target);
}

fn capture(m: &mut Model) {
    edit(m);
    m.areas.snapshots.dialog = None;
    let target = m.target();
    m.invoke(TAKE, &target);
    let a = &mut m.areas.snapshots;
    a.name = "Before upgrade".into();
    a.description = "Reviewed recovery point".into();
    a.capture.opt_in = true;
}

fn ancestry(m: &mut Model) {
    m.load_demo(3, 48);
    let (first, second) = (snapshot(FIRST), snapshot(SECOND));
    amend(m, |i| {
        i.details.snapshots.insert(
            first,
            SnapshotDetails {
                can_export: None,
                parent: SnapshotParent::Root,
                memory_vdi_known: true,
                ..Default::default()
            },
        );
        i.details.snapshots.insert(
            second,
            SnapshotDetails {
                can_export: None,
                parent: SnapshotParent::Snapshot(first),
                memory_vdi_known: true,
                preview_available: true,
                ..Default::default()
            },
        );
        i.details.vms.entry(vm()).or_default().snapshot_parent = SnapshotParent::Snapshot(second);
        if let Some(vbd) = i
            .details
            .vm_disks
            .get(&vm())
            .and_then(|ids| ids.first())
            .and_then(|id| i.details.vbds.get(id))
            .cloned()
        {
            let mut vbd = vbd;
            vbd.id = uuid::Uuid::from_u128(0x7000_0000_0000_0000_0000_0000_0000_0001).into();
            vbd.vm = None;
            vbd.snapshot = Some(second);
            i.details.snapshot_disks.insert(second, vec![vbd.id]);
            i.details.vbds.insert(vbd.id, vbd);
        }
    });
    open(m);
    select(m, second);
}

fn information(m: &mut Model) {
    ancestry(m);
    let Some((pool, vm)) = super::shown_vm(m) else {
        return;
    };
    let mut rgba = vec![0u8; 320 * 180 * 4];
    for y in 0..180 {
        for x in 0..320 {
            let index = (y * 320 + x) * 4;
            rgba[index..index + 4].copy_from_slice(if (x / 40 + y / 30) % 2 == 0 {
                &[25, 65, 94, 255]
            } else {
                &[239, 133, 48, 255]
            });
        }
    }
    let mut panel = Panel::new(pool, vm, snapshot(SECOND));
    inspection::set_picture(
        &mut panel,
        320,
        180,
        rgba,
        "Synthetic preview fixture · no guest content",
    );
    m.areas.snapshots.inspection = Some(panel);
}
