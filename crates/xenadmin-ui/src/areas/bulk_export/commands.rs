//! The commands of the area: what the two Rune buttons "Export selection…"
//! (selection bar and row menu of the Guests and Templates pages; the
//! Snapshots page) became.
//!
//! - `vm.export-selection`, `templates.export-selection`: several VMs, or
//!   several templates. They have the label and the menu place of the
//!   `transfers` area's `vm.export` and `templates.export`, which are
//!   `Hidden` for several objects as these are for one: the menu shows one
//!   "Export" whatever the selection. `Hidden` for one object, for another
//!   kind and for a selection that mixes VMs and templates (a Rune page
//!   held one kind); `Disabled` with the refusal of the selection
//!   ([`export::selected`]) or with what no choice in the wizard could fix
//!   ([`export::hopeless`]).
//! - `snapshot.export`: the snapshots selected on the Snapshots tab of the
//!   shown VM, in that tab's row of local actions (it was a button of the
//!   tab's body until the review of gate G2, criterion 5). It reads the
//!   tab's own selection (a snapshot row is not a tree object); its target
//!   is those rows, or the VM. `Hidden` when no VM's snapshots are shown or the
//!   target is something else; `Disabled` with the Rune refusal of the
//!   selection. One snapshot is the single export of `transfers`, several
//!   are the batch.

use super::export::{self, Item};
use crate::areas::{snapshots, transfers};
use crate::prelude::*;

const AREA: &str = "bulk_export";

pub const EXPORT_VMS: CommandId = CommandId("vm.export-selection");
pub const EXPORT_TEMPLATES: CommandId = CommandId("templates.export-selection");
pub const EXPORT_SNAPSHOTS: CommandId = CommandId("snapshot.export");

/// In menu order. The labels are those of the single exports.
pub fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new(EXPORT_VMS.0, "Export", AREA)
            .menu(place::VM_EXPORT)
            .context(&[ObjectKind::Vm])
            .multi()
            .eligibility(|m, target| selection(m, target, ObjectKind::Vm))
            .invoke(open_selection),
        CommandSpec::new(EXPORT_TEMPLATES.0, "Export to File", AREA)
            .menu(place::TEMPLATES_EXPORT)
            .context(&[ObjectKind::Template])
            .multi()
            .eligibility(|m, target| selection(m, target, ObjectKind::Template))
            .invoke(open_selection),
        CommandSpec::new(EXPORT_SNAPSHOTS.0, "Export selection…", AREA)
            .local(TabId::Snapshots, 60)
            .local_scopes(&[ObjectKind::Vm])
            .multi()
            .eligibility(selected_snapshots)
            .invoke(|m, _| {
                super::open_snapshots(m);
            }),
    ]
}

fn selection(m: &Model, target: &Target, kind: ObjectKind) -> Eligibility {
    if !target.is_multi() || !target.all_of(kind) {
        return Eligibility::Hidden;
    }
    match export::selected(m, target) {
        Ok((pool, vms)) => {
            let items = vms.into_iter().map(Item::Vm).collect::<Vec<_>>();
            Eligibility::enabled_unless(export::hopeless(m, pool, &items))
        }
        Err(reason) => Eligibility::disabled(reason),
    }
}

fn open_selection(m: &mut Model, target: &Target) {
    super::open(m, target);
}

fn selected_snapshots(m: &Model, target: &Target) -> Eligibility {
    let Some((pool, vm)) = snapshots::shown_vm(m) else {
        return Eligibility::Hidden;
    };
    let shown = ObjectRef::vm(pool, vm);
    if !target
        .objects
        .iter()
        .all(|object| object.kind == ObjectKind::Snapshot || *object == shown)
    {
        return Eligibility::Hidden;
    }
    match export::selected_snapshots(m) {
        Err(reason) => Eligibility::disabled(reason),
        Ok((pool, targets)) => match (targets.as_slice(), m.inventory_for(pool)) {
            ([one], Some(inventory)) => {
                transfers::export_eligibility(m, pool, &export::single_source(&inventory, one))
            }
            _ => {
                let items = targets.into_iter().map(Item::Snapshot).collect::<Vec<_>>();
                Eligibility::enabled_unless(export::hopeless(m, pool, &items))
            }
        },
    }
}
