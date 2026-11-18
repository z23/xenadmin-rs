//! The scenes of the area, under their Rune names. Each sets the demo pool
//! up as its Rune scene did. The outputs are paths in the system temporary
//! directory that do not exist; nothing is created there.

use std::sync::Arc;

use uuid::Uuid;
use xenadmin_core::{
    Inventory, PowerState, SnapshotDetails, SnapshotId, TransferFile, VdiId, VmId,
};

use super::export::{self, Item};
use crate::areas::snapshots;
use crate::prelude::*;

const AREA: &str = "bulk_export";

/// The demo guest that has the snapshots, and two of them: "Before
/// upgrade" and "Known good".
pub(crate) const DEMO_PARENT: u128 = 0x3000_0000_0000_0000_0000_0000_0000_0000;
pub(crate) const DEMO_SNAPSHOT: u128 = 0x6000_0000_0000_0000_0000_0000_0000_0000;
pub(crate) const DEMO_CHECKPOINT: u128 = 0x6000_0000_0000_0000_0000_0000_0000_0001;

pub fn scenes() -> Vec<SceneSpec> {
    vec![
        SceneSpec::new("batch-export", AREA, vm_scene),
        SceneSpec::new("snapshot-batch-export", AREA, snapshot_scene),
    ]
}

/// Replaces the scope session's inventory with an edited copy.
pub(crate) fn amend(m: &mut Model, change: impl FnOnce(&mut Inventory)) {
    if let Some(inventory) = m.inventory.take() {
        let mut inventory = inventory.as_ref().clone();
        change(&mut inventory);
        m.inventory = Some(Arc::new(inventory));
    }
}

/// The Rune `bulk_export::scene`: the first two guests of the demo pool,
/// halted and allowed to export, selected; the review of their two XVA
/// exports.
pub(crate) fn vm_scene(m: &mut Model) {
    m.load_demo(3, 48);
    let Some(i) = m.inventory.clone() else {
        return;
    };
    let ids = i.vms.keys().copied().take(2).collect::<Vec<VmId>>();
    amend(m, |i| {
        for id in &ids {
            if let Some(vm) = i.vms.get_mut(id) {
                vm.power_state = PowerState::Halted;
                vm.is_template = false;
            }
            let d = i.details.vms.entry(*id).or_default();
            d.allowed_operations = Some(vec!["export".into()]);
            d.current_operations.clear();
            d.blocked_operations.clear();
        }
    });
    let target = Target::many(ids.iter().map(|id| ObjectRef::vm(i.pool_id, *id)));
    m.select(target.clone());
    super::open(m, &target);
    demo_outputs(m);
    export::review(m);
}

/// Gives every target of a synthetic scene a destination that does not exist.
pub(crate) fn demo_outputs(m: &mut Model) {
    let Some(e) = m.areas.bulk_export.editor.as_mut() else {
        return;
    };
    for (index, target) in e.targets.iter().enumerate() {
        let path = std::env::temp_dir().join(format!("xenadmin-demo-{}-{}.xva", e.id, target.id()));
        e.files[index] = TransferFile::destination(&path)
            .ok()
            .map(|file| (path, file));
    }
}

/// Turns one of the synthetic inventory's disk snapshots into a memory checkpoint whose
/// memory image can be reviewed.
pub(crate) fn demo_checkpoint(m: &mut Model, snapshot: SnapshotId, memory: VdiId) {
    amend(m, |i| {
        let Some(mut disk) = i.details.vdis.values().next().cloned() else {
            return;
        };
        disk.id = memory;
        disk.name = "Checkpoint memory".into();
        disk.kind = "suspend".into();
        disk.virtual_size = Some(512 * 1024 * 1024);
        disk.managed = Some(true);
        disk.missing = Some(false);
        i.details.vdis.insert(memory, disk);
        i.details.snapshots.insert(
            snapshot,
            SnapshotDetails {
                memory_vdi: Some(memory),
                memory_vdi_known: true,
                can_export: Some(true),
                ..Default::default()
            },
        );
        let Some(mut suspended) = i.snapshots.get(&snapshot).cloned() else {
            return;
        };
        suspended.power_state = PowerState::Suspended;
        suspended.description = "Synthetic memory checkpoint".into();
        i.snapshots.insert(suspended);
    });
}

/// Opens the editor on the given snapshots of the synthetic VM, in the
/// given order, with those rows selected on its Snapshots tab.
pub(crate) fn demo_editor(m: &mut Model, ids: &[SnapshotId]) {
    let parent = VmId::from(Uuid::from_u128(DEMO_PARENT));
    // The synthetic host offers each of them for export, as a real one says of a snapshot
    // in its allowed operations; a snapshot it does not offer is not exported.
    amend(m, |i| {
        for snapshot in ids {
            i.details
                .snapshots
                .entry(*snapshot)
                .or_default()
                .can_export
                .get_or_insert(true);
        }
    });
    let Some(pool) = m.inventory.as_ref().map(|i| i.pool_id) else {
        return;
    };
    m.select(Target::one(ObjectRef::vm(pool, parent)));
    m.set_active_tab(TabId::Snapshots);
    let rows = ids.iter().map(ToString::to_string).collect::<Vec<_>>();
    snapshots::edit::select(m, &rows);
    match export::snapshots_of(m, pool, parent, ids) {
        Ok(targets) => {
            export::begin(m, pool, targets.into_iter().map(Item::Snapshot).collect());
        }
        Err(reason) => m.error = Some(reason),
    }
}

/// A synthetic review of one disk snapshot and one memory checkpoint with its memory
/// chosen, for renders and tests (the Rune `snapshots::scene`).
pub(crate) fn snapshot_scene(m: &mut Model) {
    m.load_demo(3, 48);
    let plain = SnapshotId::from(Uuid::from_u128(DEMO_SNAPSHOT));
    let checkpoint = SnapshotId::from(Uuid::from_u128(DEMO_CHECKPOINT));
    demo_checkpoint(m, checkpoint, VdiId::from(Uuid::from_u128(0x9902)));
    demo_editor(m, &[plain, checkpoint]);
    export::set_memory(m, 1, true);
    demo_outputs(m);
    export::review(m);
}
