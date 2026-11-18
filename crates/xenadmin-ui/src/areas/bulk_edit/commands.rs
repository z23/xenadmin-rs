//! The commands of the area: what "Delete selection…" of the Rune selection
//! bar, the Templates page and the row menu became.
//!
//! Deleting ONE VM or template is the `vm_lifecycle` area's (`vm.delete`,
//! `templates.delete`), and those are hidden for several objects. These two
//! take their place then: the same label, the same menu place and the same
//! context menu, hidden for one object. So a menu has one "Delete VM" (or
//! "Delete Template") whatever is selected.
//!
//! - `Hidden` unless the target is several VMs, or several templates;
//! - `Disabled` with the refusal of the selection ([`delete::selected`]:
//!   more than 32, several pools, a row that is gone), or with what no
//!   choice in the dialog could fix ([`delete::hopeless`]: a pool that
//!   cannot be changed now, a target that cannot be deleted);
//! - otherwise the batch opens on its draft.
//!
//! "Edit selection…" has no command here: it is the kernel's
//! `vm.properties` / `template.properties` on several objects, for which
//! this area provides the page (`view::pages`).

use super::delete;
use crate::prelude::*;

const AREA: &str = "bulk_edit";

pub const DELETE: CommandId = CommandId("vm.delete-several");
pub const TEMPLATES_DELETE: CommandId = CommandId("templates.delete-several");

/// In menu order. Labels and places are those of `vm.delete` and
/// `templates.delete`.
pub fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new(DELETE.0, "Delete VM", AREA)
            .menu(place::VM_DELETE)
            .context(&[ObjectKind::Vm])
            .multi()
            .eligibility(|m, target| eligibility(m, target, ObjectKind::Vm))
            .invoke(delete::open),
        CommandSpec::new(TEMPLATES_DELETE.0, "Delete Template", AREA)
            .menu(place::TEMPLATES_DELETE)
            .context(&[ObjectKind::Template])
            .multi()
            .eligibility(|m, target| eligibility(m, target, ObjectKind::Template))
            .invoke(delete::open),
    ]
}

fn eligibility(m: &Model, target: &Target, kind: ObjectKind) -> Eligibility {
    if !target.is_multi() || !target.all_of(kind) {
        return Eligibility::Hidden;
    }
    match delete::capture(m, target) {
        Ok(editor) => Eligibility::enabled_unless(delete::hopeless(m, &editor)),
        Err(reason) => Eligibility::disabled(reason),
    }
}
