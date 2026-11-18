//! The command table of the area.
//!
//! `vm.snapshot` acts on a VM: the scope, a VM's context menu, or the VM
//! whose Snapshots tab is showing. The five row actions act on the one
//! selected row of that tab ([`edit::selected_snapshot`]); they are in the
//! tab's local action row and in the menu of a row, and nowhere else.
//!
//! Eligibility is what the Rune buttons applied: enabled in a synthetic
//! pool (the dialog opens as a preview), otherwise a writable, current
//! connection and what the host allows for the VM or the snapshot. The
//! reasons are the captions the Rune page showed under its buttons; three
//! are new because Rune disabled the button without a word
//! ([`NONE_SELECTED`] is the page's own caption cut short,
//! [`RESTORE_REFUSED`] follows the clone caption, and a VM the inventory no
//! longer has gives `Error::Stale`).

use xenadmin_core::{Error, PoolId, PowerAction, PowerState, SnapshotAction, VmId};

use super::{edit, shown_vm};
use crate::prelude::*;

pub const TAKE: CommandId = CommandId("vm.snapshot");
pub const REVERT: CommandId = CommandId("snapshot.revert");
pub const PROPERTIES: CommandId = CommandId("snapshot.properties");
pub const NEW_VM: CommandId = CommandId("snapshot.new-vm");
pub const NEW_TEMPLATE: CommandId = CommandId("snapshot.new-template");
pub const DELETE: CommandId = CommandId("snapshot.delete");

/// The export of the selected snapshots: `bulk_export`'s command. The tab's
/// button to it follows the command's eligibility.
pub const EXPORT: CommandId = CommandId("snapshot.export");

/// The menu of a snapshot row.
pub const ROW_MENU: [CommandId; 5] = [REVERT, PROPERTIES, NEW_VM, NEW_TEMPLATE, DELETE];

pub const NONE_SELECTED: &str = "Select a snapshot to restore, copy or delete.";
pub const RESTORE_STATE: &str = "Restore can cleanly shut down a running VM when explicitly selected. Paused or suspended VMs must be shut down separately.";
pub const RESTORE_REFUSED: &str =
    "The host does not currently allow restoring the selected snapshot.";
pub const CLONE_REFUSED: &str =
    "The host does not currently allow cloning the selected snapshot into a VM or template.";
pub const DELETE_REFUSED: &str = "The selected snapshot cannot currently be deleted. Deletion requires a halted snapshot or a memory checkpoint the host permits stopping.";

/// What the page said with several rows selected.
pub fn several(count: usize) -> String {
    format!(
        "{count} snapshots selected. Export selection reviews them together; the other actions take one snapshot."
    )
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RowAction {
    Revert,
    Edit,
    CloneVm,
    CloneTemplate,
    Delete,
}

pub fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new(TAKE.0, "Take a Snapshot", "snapshots")
            .menu(place::VM_SNAPSHOT)
            .context(&[ObjectKind::Vm])
            .local(TabId::Snapshots, 10)
            .local_scopes(&[ObjectKind::Vm])
            .eligibility(take_eligibility)
            .invoke(|m, target| {
                if let Some((pool, vm)) = subject(m, target) {
                    edit::begin_create(m, pool, vm);
                }
            }),
        CommandSpec::new(REVERT.0, "Revert", "snapshots")
            .local(TabId::Snapshots, 20)
            .local_scopes(&[ObjectKind::Vm])
            .eligibility(|m, target| row_eligibility(m, target, RowAction::Revert))
            .invoke(|m, _| edit::begin_restore(m)),
        CommandSpec::new(PROPERTIES.0, "Edit properties", "snapshots")
            .local(TabId::Snapshots, 30)
            .local_scopes(&[ObjectKind::Vm])
            .eligibility(|m, target| row_eligibility(m, target, RowAction::Edit))
            .invoke(|m, _| edit::begin_edit(m)),
        CommandSpec::new(NEW_VM.0, "Create VM", "snapshots")
            .local(TabId::Snapshots, 40)
            .local_scopes(&[ObjectKind::Vm])
            .eligibility(|m, target| row_eligibility(m, target, RowAction::CloneVm))
            .invoke(|m, _| edit::begin_clone(m, false)),
        CommandSpec::new(NEW_TEMPLATE.0, "Create template", "snapshots")
            .local(TabId::Snapshots, 50)
            .local_scopes(&[ObjectKind::Vm])
            .eligibility(|m, target| row_eligibility(m, target, RowAction::CloneTemplate))
            .invoke(|m, _| edit::begin_clone(m, true)),
        CommandSpec::new(DELETE.0, "Delete", "snapshots")
            .local(TabId::Snapshots, 90)
            .local_scopes(&[ObjectKind::Vm])
            .eligibility(|m, target| row_eligibility(m, target, RowAction::Delete))
            .invoke(|m, _| edit::begin_delete(m)),
    ]
}

/// The VM a command of this area acts on: the one VM of the target, or,
/// when the target is a selection of snapshots (a search result opened on
/// this tab), the VM whose Snapshots tab is showing.
pub fn subject(m: &Model, target: &Target) -> Option<(PoolId, VmId)> {
    if let Some(object) = target.single_of(ObjectKind::Vm) {
        return Some((object.pool?, object.vm_id()?));
    }
    let snapshots = !target.is_empty()
        && target
            .objects
            .iter()
            .all(|object| object.kind == ObjectKind::Snapshot);
    if snapshots { shown_vm(m) } else { None }
}

fn take_eligibility(m: &Model, target: &Target) -> Eligibility {
    let Some((pool, vm)) = subject(m, target) else {
        return Eligibility::Hidden;
    };
    let stale = || Eligibility::disabled(Error::Stale.to_string());
    let Some(inventory) = m.inventory_for(pool) else {
        return stale();
    };
    let Some(guest) = inventory.vms.get(&vm) else {
        return stale();
    };
    if guest.is_template || guest.is_control_domain {
        return Eligibility::Hidden;
    }
    if m.areas.snapshots.capture.busy() {
        return Eligibility::disabled(edit::CAPTURE_PENDING);
    }
    if m.is_synthetic_pool(pool) {
        return Eligibility::Enabled;
    }
    if !edit::current(m, pool) {
        return Eligibility::disabled(edit::not_writable(m, pool));
    }
    Eligibility::enabled_if(
        guest.can_snapshot || edit::memory_available(&inventory, vm),
        edit::MODE_UNAVAILABLE,
    )
}

/// The eligibility of an action on the selected row. Hidden anywhere but
/// on the VM whose Snapshots tab lists the rows.
pub fn row_eligibility(m: &Model, target: &Target, action: RowAction) -> Eligibility {
    let Some((pool, vm)) = subject(m, target) else {
        return Eligibility::Hidden;
    };
    if shown_vm(m) != Some((pool, vm)) {
        return Eligibility::Hidden;
    }
    let stale = || Eligibility::disabled(Error::Stale.to_string());
    let Some(inventory) = m.inventory_for(pool) else {
        return stale();
    };
    let Some(guest) = inventory.vms.get(&vm) else {
        return stale();
    };
    let selected = edit::selected_snapshots(m);
    let snapshot = match selected.as_slice() {
        [] => return Eligibility::disabled(NONE_SELECTED),
        [one] => one,
        many => return Eligibility::disabled(several(many.len())),
    };
    if m.is_synthetic_pool(pool) {
        return Eligibility::Enabled;
    }
    if !edit::current(m, pool) {
        return Eligibility::disabled(edit::not_writable(m, pool));
    }
    let allows = |action: SnapshotAction| snapshot.allowed_actions.contains(&action);
    match action {
        RowAction::Revert => {
            let state = guest.power_state == PowerState::Halted
                || (guest.power_state == PowerState::Running
                    && guest.allowed_actions.contains(&PowerAction::CleanShutdown));
            if !state {
                return Eligibility::disabled(RESTORE_STATE);
            }
            Eligibility::enabled_if(allows(SnapshotAction::Revert), RESTORE_REFUSED)
        }
        RowAction::Edit => Eligibility::Enabled,
        RowAction::CloneVm | RowAction::CloneTemplate => {
            Eligibility::enabled_if(allows(SnapshotAction::Clone), CLONE_REFUSED)
        }
        RowAction::Delete => Eligibility::enabled_if(
            matches!(
                snapshot.power_state,
                PowerState::Halted | PowerState::Suspended
            ) && allows(SnapshotAction::Delete),
            DELETE_REFUSED,
        ),
    }
}
