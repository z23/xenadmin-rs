//! Deleting one VM or template: the logic of the Rune `vm_deletion.rs`.
//!
//! The default removes the record only. Data disks and snapshots are
//! chosen one by one; a disk can be chosen only when the captured inventory
//! establishes that the VM owns it exclusively.

use std::collections::BTreeSet;

use xenadmin_core::{
    Inventory, PowerState, Snapshot, SnapshotAction, SnapshotDeletionSelection, SnapshotId, VdiId,
    Vm, VmLifecycleOperation,
};

use super::edit::{self, Editor};
use crate::prelude::*;

pub const INTRODUCTION: &str = "The default removes only the VM/template record. Select individual owned disks or snapshots below to delete them too. Disk deletion is irreversible; unselected data is retained.";
pub const NO_DISKS: &str = "No data disks are reported for this VM.";
pub const NO_SNAPSHOTS: &str = "No snapshots are reported for this VM.";
pub const SNAPSHOT_RETAINED: &str = "Retained. Memory checkpoints require the separate snapshot deletion flow; other snapshots must advertise deletion permission.";
pub const CONSEQUENCES: &str = "Shared or unowned data is never included automatically. If another operator changes ownership or consumers, submission is refused. Completed deletions cannot be rolled back after a later failure.";

/// The deletion the draft describes. Of a chosen snapshot only the disks
/// that the captured inventory lets it delete and that are checked go
/// along.
pub(crate) fn operation(form: &Editor) -> VmLifecycleOperation {
    VmLifecycleOperation::Delete {
        expected_template: form.source.is_template,
        disks: form.disks.iter().copied().collect(),
        snapshots: form
            .snapshots
            .iter()
            .map(|id| SnapshotDeletionSelection {
                snapshot: *id,
                disks: form
                    .initial
                    .snapshot_deletion_disks(*id)
                    .into_iter()
                    .filter(|disk| form.snapshot_disks.contains(disk))
                    .collect(),
            })
            .collect(),
    }
}

/// One line per attachment of a disk: who uses it, through which
/// attachment, in which slot.
pub(crate) fn consumers(inventory: &Inventory, disk: VdiId) -> Vec<String> {
    inventory
        .details
        .vbds
        .values()
        .filter(|vbd| vbd.vdi == Some(disk))
        .map(|vbd| {
            let owner = vbd
                .vm
                .and_then(|id| {
                    inventory
                        .vms
                        .get(&id)
                        .map(|vm| format!("{} ({id})", vm.name))
                })
                .or_else(|| {
                    vbd.snapshot.and_then(|id| {
                        inventory
                            .snapshots
                            .get(&id)
                            .map(|snapshot| format!("Snapshot {} ({id})", snapshot.name))
                    })
                })
                .unwrap_or_else(|| "Owner not reported".into());
            format!(
                "Consumer: {owner}; attachment {}; slot {}",
                vbd.id, vbd.userdevice
            )
        })
        .collect()
}

/// The disks whose data may be chosen for deletion.
pub(crate) fn candidate_disks(form: &Editor) -> Vec<VdiId> {
    form.initial.vm_deletion_disks(form.source.id)
}

/// Every data disk attached to the source, chosen or not.
pub(crate) fn data_disks(inventory: &Inventory, source: &Vm) -> BTreeSet<VdiId> {
    inventory
        .details
        .vbds
        .values()
        .filter(|vbd| vbd.vm == Some(source.id) && vbd.kind == "Disk")
        .filter_map(|vbd| vbd.vdi)
        .collect()
}

/// A snapshot can be chosen: halted (no memory checkpoint) and the host
/// lets it be deleted.
pub(crate) fn snapshot_eligible(snapshot: &Snapshot) -> bool {
    snapshot.power_state == PowerState::Halted
        && snapshot.allowed_actions.contains(&SnapshotAction::Delete)
}

pub(crate) fn toggle_disk(m: &mut Model, id: VdiId) {
    if let Some(form) = edit::editor_mut(m)
        && !form.disks.remove(&id)
    {
        form.disks.insert(id);
    }
}

pub(crate) fn toggle_snapshot(m: &mut Model, id: SnapshotId) {
    if let Some(form) = edit::editor_mut(m)
        && !form.snapshots.remove(&id)
    {
        form.snapshots.insert(id);
    }
}

pub(crate) fn toggle_snapshot_disk(m: &mut Model, disk: VdiId) {
    if let Some(form) = edit::editor_mut(m)
        && !form.snapshot_disks.remove(&disk)
    {
        form.snapshot_disks.insert(disk);
    }
}

pub(crate) fn review(form: &Editor) -> Vec<String> {
    describe(
        &form.initial,
        &form.source,
        &form.disks,
        &form.snapshots,
        &form.snapshot_disks,
    )
}

/// The review lines of a deletion: what goes, what is retained, and that
/// only the exact reviewed UUIDs are eligible.
pub fn describe(
    inventory: &Inventory,
    source: &Vm,
    disks: &BTreeSet<VdiId>,
    snapshots: &BTreeSet<SnapshotId>,
    snapshot_disks: &BTreeSet<VdiId>,
) -> Vec<String> {
    let mut lines = vec![format!(
        "Permanently remove {} “{}” ({}) from the pool inventory.",
        if source.is_template { "template" } else { "VM" },
        source.name,
        source.id
    )];
    lines.push("Its disk and network attachment records are also removed. Retained disks remain in storage.".into());
    if disks.is_empty() && snapshots.is_empty() {
        lines.push(
            "Record-only deletion: no disk data or snapshot records were selected for deletion."
                .into(),
        );
    }
    for id in disks {
        lines.push(format!(
            "DELETE DISK DATA: {}",
            edit::disk_label(inventory, *id)
        ));
        lines.extend(consumers(inventory, *id));
    }
    for snapshot in inventory.snapshots.for_vm(source.id) {
        if snapshots.contains(&snapshot.id) {
            lines.push(format!(
                "DELETE SNAPSHOT: {} ({})",
                snapshot.name, snapshot.id
            ));
            for disk in inventory
                .snapshot_deletion_disks(snapshot.id)
                .into_iter()
                .filter(|id| snapshot_disks.contains(id))
            {
                lines.push(format!(
                    "DELETE SNAPSHOT DATA: {}",
                    edit::disk_label(inventory, disk)
                ));
                lines.extend(consumers(inventory, disk));
            }
        } else {
            lines.push(format!(
                "Retain snapshot: {} ({})",
                snapshot.name, snapshot.id
            ));
        }
    }
    for id in data_disks(inventory, source) {
        if !disks.contains(&id) {
            lines.push(format!(
                "Retain disk data: {}",
                edit::disk_label(inventory, id)
            ));
        }
    }
    lines.push("Only the exact reviewed UUIDs are eligible for deletion. New snapshots or disks are not silently added. Selected data deletion cannot be undone. Partial or uncertain results remain in VM changes after the source disappears.".into());
    lines
}
