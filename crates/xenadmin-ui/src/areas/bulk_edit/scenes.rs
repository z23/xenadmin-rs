//! The scenes of the area, under their Rune names. Each goes the way a
//! user goes: several objects selected, a command invoked.

use std::sync::Arc;

use xenadmin_core::{Inventory, PowerState, VmId};

use super::commands::DELETE;
use super::{delete, edit};
use crate::prelude::*;

pub fn scenes() -> Vec<SceneSpec> {
    vec![
        SceneSpec::new("batch-delete", "bulk_edit", delete_scene),
        SceneSpec::new("batch-properties", "bulk_edit", properties_scene),
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

/// The first guests of the demo pool as a target, selected.
pub(crate) fn guests(m: &mut Model, count: usize) -> Option<Target> {
    let inventory = m.inventory.clone()?;
    let pool = inventory.pool_id;
    let target = Target::many(
        inventory
            .guests()
            .take(count)
            .map(|vm| ObjectRef::vm(pool, vm.id)),
    );
    m.select(target.clone());
    Some(target)
}

/// Rune `bulk_delete::scene`: the first two guests halted and allowed to be
/// destroyed, both captured, reviewed as record-only deletions.
fn delete_scene(m: &mut Model) {
    m.load_demo(3, 48);
    let ids: Vec<VmId> = m
        .inventory
        .as_ref()
        .map(|i| i.guests().take(2).map(|vm| vm.id).collect())
        .unwrap_or_default();
    amend(m, |i| {
        for id in &ids {
            if let Some(vm) = i.vms.get_mut(id) {
                vm.power_state = PowerState::Halted;
            }
            if let Some(d) = i.details.vms.get_mut(id) {
                d.is_default_template = Some(false);
                d.allowed_operations = Some(vec!["destroy".into()]);
                d.current_operations.clear();
                d.blocked_operations.clear();
            }
        }
    });
    let Some(target) = guests(m, 2) else {
        return;
    };
    m.invoke(DELETE, &target);
    delete::review(m);
}

/// Rune `bulk_properties::scene`: the first three guests, a shared
/// description, reviewed.
fn properties_scene(m: &mut Model) {
    m.load_demo(3, 48);
    let Some(target) = guests(m, 3) else {
        return;
    };
    m.invoke(crate::properties::VM, &target);
    if let Some(e) = m.areas.bulk_edit.properties.as_mut() {
        e.values[0] = "Reviewed shared description".into();
    }
    edit::review(m);
}
