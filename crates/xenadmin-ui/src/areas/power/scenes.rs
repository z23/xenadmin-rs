//! The scenes of the area, under their Rune names.
//!
//! Each sets the demo pool up as its Rune scene did and then goes the way a
//! user goes: an object selected, a command invoked. The Rune scenes wrote
//! the editors' fields; going through the commands shows that the commands
//! lead to the same states.
//!
//! `unavailable-actions`, the fourth scene of the Rune power code, is the
//! kernel's (`demo.rs`): it has no surface, and what it shows of this area
//! is the reason on each disabled power command.

use std::sync::Arc;

use xenadmin_core::{Inventory, PowerAction, PowerState, RunningOperation, TaskId, VmId};

use super::batch;
use super::commands::{FORCE_SHUT_DOWN, POWER_SELECTION, RECOVERY_START};
use crate::prelude::*;

pub fn scenes() -> Vec<SceneSpec> {
    vec![
        SceneSpec::new("batch-power", "power", batch_scene),
        SceneSpec::new("vm-recovery", "power", recovery_scene),
        SceneSpec::new("vm-recovery-force", "power", force_scene),
    ]
}

/// Replaces the scope session's inventory with an edited copy.
pub(crate) fn amend(m: &mut Model, edit: impl FnOnce(&mut Inventory)) {
    if let Some(inventory) = m.inventory.take() {
        let mut inventory = inventory.as_ref().clone();
        edit(&mut inventory);
        m.inventory = Some(Arc::new(inventory));
    }
}

/// The review of a batch over two guests in the same power state, with the
/// first action both are available for.
fn batch_scene(m: &mut Model) {
    m.load_demo(3, 48);
    let Some(inventory) = m.inventory.clone() else {
        return;
    };
    let Some(first) = inventory.guests().next() else {
        return;
    };
    let pool = inventory.pool_id;
    let target = Target::many(
        std::iter::once(first)
            .chain(
                inventory
                    .guests()
                    .find(|v| v.id != first.id && v.power_state == first.power_state),
            )
            .map(|vm| ObjectRef::vm(pool, vm.id)),
    );
    m.select(target.clone());
    m.invoke(POWER_SELECTION, &target);
    batch::review(m);
}

/// The first halted guest of the demo pool, made an HVM guest that allows
/// Start, and selected. Public to the area's tests, which start from it as
/// the Rune tests started from `vm_recovery_view::scene`.
pub(crate) fn prepare_recovery(m: &mut Model) -> Option<Target> {
    m.load_demo(3, 48);
    let inventory = m.inventory.clone()?;
    let id = inventory
        .vms
        .values()
        .find(|v| !v.is_template && !v.is_control_domain && v.power_state == PowerState::Halted)
        .map(|v| v.id)?;
    amend(m, |i| {
        if let Some(vm) = i.vms.get_mut(&id) {
            vm.allowed_actions = vec![PowerAction::Start];
        }
        if let Some(details) = i.details.vms.get_mut(&id) {
            details.domain_type = Some("hvm".into());
        }
    });
    let target = Target::one(ObjectRef::vm(inventory.pool_id, id));
    m.select(target.clone());
    Some(target)
}

/// The same guest running, with nothing allowed and one task of the host.
pub(crate) fn make_busy(m: &mut Model, id: VmId, operation: &str, task: Option<TaskId>) {
    amend(m, |i| {
        if let Some(vm) = i.vms.get_mut(&id) {
            vm.power_state = PowerState::Running;
            vm.allowed_actions.clear();
        }
        if let Some(details) = i.details.vms.get_mut(&id) {
            details.current_operations = vec![RunningOperation {
                task,
                operation: operation.into(),
            }];
        }
    });
}

/// The review of a boot from recovery media.
fn recovery_scene(m: &mut Model) {
    if let Some(target) = prepare_recovery(m) {
        m.invoke(RECOVERY_START, &target);
    }
}

/// The task-aware review of a force shutdown over one captured task (an
/// export, so no migration caution).
fn force_scene(m: &mut Model) {
    let Some(target) = prepare_recovery(m) else {
        return;
    };
    let Some(id) = target.vms().first().copied() else {
        return;
    };
    make_busy(
        m,
        id,
        "export",
        Some(TaskId::from(uuid::Uuid::from_u128(900))),
    );
    m.invoke(FORCE_SHUT_DOWN, &target);
}
