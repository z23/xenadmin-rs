//! The commands of the area: what the Rune button "Migrate / move VM" of
//! the details pane and "Move selection…" of the selection bar and the row
//! menu became.
//!
//! The plan's VM menu has two items, "Migrate to Server" and "Move VM".
//! This client has one workflow behind both (a host move, a move of the
//! disks, or both, of a running or a halted VM), so both commands open the
//! same thing and have the same eligibility:
//!
//! - on several objects: `Hidden` unless all are VMs; `Disabled` with the
//!   refusal of the selection ([`batch::selected`]), or with what no draft
//!   could fix ([`batch::hopeless`]); otherwise the batch opens;
//! - on one object: `Hidden` unless it is an ordinary VM of its pool's
//!   inventory (Rune showed no button); `Disabled` where Rune disabled the
//!   button, with the caption it showed or, where it showed none,
//!   [`wizard::NOT_WRITABLE`]; otherwise the wizard opens on its first
//!   step.

use super::{batch, wizard};
use crate::prelude::*;

const AREA: &str = "migration";

pub const MIGRATE: CommandId = CommandId("vm.migrate");
pub const MOVE: CommandId = CommandId("vm.move");

/// The command of the `cross_pool` area (not ported yet) that the second
/// step of the wizard leads to: the Rune button "Move / copy to another
/// pool…". Its id is the one the map of that area proposes.
pub const MOVE_TO_POOL: CommandId = CommandId("vm.move-to-pool");

/// In menu order. Labels are the plan's (Menus table).
pub fn commands() -> Vec<CommandSpec> {
    const VM: &[ObjectKind] = &[ObjectKind::Vm];
    vec![
        CommandSpec::new(MIGRATE.0, "Migrate to Server", AREA)
            .menu(place::VM_MIGRATE)
            .context(VM)
            .multi()
            .eligibility(eligibility)
            .invoke(invoke),
        CommandSpec::new(MOVE.0, "Move VM", AREA)
            .menu(place::VM_MOVE)
            .context(VM)
            .multi()
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
            Ok(editor) => Eligibility::enabled_unless(batch::hopeless(m, &editor)),
            Err(reason) => Eligibility::disabled(reason),
        };
    }
    match target.single().map(|object| wizard::capture(m, object)) {
        Some(Ok(_)) => Eligibility::Enabled,
        Some(Err(eligibility)) => eligibility,
        None => Eligibility::Hidden,
    }
}

fn invoke(m: &mut Model, target: &Target) {
    if target.is_multi() {
        batch::open(m, target);
    } else if let Some(object) = target.single() {
        wizard::open(m, object);
    }
}
