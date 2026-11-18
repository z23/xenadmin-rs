//! The command of the area: what the Rune buttons "Move / copy to another
//! pool…" of the details pane (`cross-pool-open`) and "Move selection to
//! another pool…" of the selection bar and the row menu
//! (`batch-cross-pool-open`) became.
//!
//! - On several objects: `Hidden` unless all are VMs; `Disabled` with the
//!   refusal of the selection (`migration::selected`), with
//!   [`batch::NOT_WRITABLE`] where the Rune `open` refused, or with what no
//!   draft could fix ([`batch::hopeless`]); otherwise the batch opens. A
//!   synthetic pool opens it to be looked at, as in Rune.
//! - On one object: `Hidden` unless it is an ordinary VM of its pool's
//!   inventory (Rune showed no button); `Disabled` where Rune disabled the
//!   button or its `open` returned without a word ([`single::capture`]
//!   names each case); otherwise the wizard opens on its first step.

use super::{batch, single};
use crate::command::{Menu, MenuPath};
use crate::prelude::*;

const AREA: &str = "cross_pool";

pub const MOVE_TO_POOL: CommandId = CommandId("vm.move-to-pool");

/// What the command is called on one VM, which can also be copied.
pub const SINGLE_LABEL: &str = "Move or Copy to Another Pool";

pub fn commands() -> Vec<CommandSpec> {
    vec![
        // With the two moves within a pool (`place::VM_MIGRATE`, `VM_MOVE`), before Copy.
        CommandSpec::new(MOVE_TO_POOL.0, "Move to Another Pool", AREA)
            .menu(MenuPath::new(Menu::Vm, 3, 25))
            .context(&[ObjectKind::Vm])
            .multi()
            .dynamic_label(|_, target| (!target.is_multi()).then(|| SINGLE_LABEL.to_owned()))
            .eligibility(eligibility)
            .invoke(invoke),
    ]
}

fn eligibility(m: &Model, target: &Target) -> Eligibility {
    if target.is_multi() {
        if !target.all_of(ObjectKind::Vm) {
            return Eligibility::Hidden;
        }
        return match batch::capture(m, target) {
            Ok(editor) => Eligibility::enabled_unless(batch::hopeless(&editor)),
            Err(reason) => Eligibility::disabled(reason),
        };
    }
    match target.single().map(|object| single::capture(m, object)) {
        Some(Ok(_)) => Eligibility::Enabled,
        Some(Err(eligibility)) => eligibility,
        None => Eligibility::Hidden,
    }
}

fn invoke(m: &mut Model, target: &Target) {
    if target.is_multi() {
        batch::open(m, target);
    } else if let Some(object) = target.single() {
        single::open(m, object);
    }
}
