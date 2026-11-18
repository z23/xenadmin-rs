//! Area `general`: the General and Memory tabs, and the information-only
//! Storage, Networking and NICs tabs until their areas provide them.
//!
//! Rune modules: `inventory_information.rs` (the "General information"
//! dialog), `resource_details.rs` (the fields appended to the details pane
//! for a server, a storage repository and a network) and the fields of the
//! details pane itself (`details` of `main.rs`).
//!
//! The Rune dialog was a captured observation with "Refresh from client", a
//! pinned pool and "Pool information". That is retired (map `rune-kernel`,
//! section 6.3): a tab is live and follows the tree selection, and the pool
//! has a General tab of its own. Three things of the capture are kept: the
//! freshness of the inventory (a property "Inventory": "Synthetic
//! inventory", "Current" or "Stale"), elapsed times computed against the
//! time of the last inventory update and labelled as before, and the
//! explicit "Unknown" wherever a value is not known.
//!
//! The area has no state, no commands and no surfaces: everything it shows
//! is a tab body. Two sections of a VM's General tab come from other areas:
//! "Power" (`power::availability_section`) and the controls under each
//! record of "Operations" (`power::operation_controls`,
//! `power::cancellation_review_button`, `events::outcome_actions`).

use crate::prelude::*;

mod information;
mod operations;
mod resources;
#[cfg(test)]
mod tests;

use information::View;

/// The state of the area: none. `m.areas.general`.
#[derive(Default)]
pub struct State {}

pub fn spec() -> AreaSpec {
    AreaSpec {
        scenes,
        tab,
        ..AreaSpec::new("general")
    }
}

fn tab(m: &Model, tab: TabId, scope: &Target) -> Option<TabBody> {
    if !matches!(
        tab,
        TabId::General | TabId::Memory | TabId::Storage | TabId::Networking | TabId::Nics
    ) {
        return None;
    }
    let object = scope.single()?;
    if object.kind == ObjectKind::Connection {
        return (tab == TabId::General)
            .then(|| TabBody::Blocks(resources::connection_general(m, object)));
    }
    let pool = object.pool?;
    let Some(inventory) = m.inventory_for(pool) else {
        return Some(TabBody::Blocks(unavailable(tab, object.kind)?));
    };
    let view = View::of(m, pool, &inventory);
    body(m, &view, tab, object).map(TabBody::Blocks)
}

/// What a tab says while its pool has no inventory. `None` when the area
/// does not provide the tab for the kind at all.
fn unavailable(tab: TabId, kind: ObjectKind) -> Option<Vec<Block>> {
    provides(tab, kind).then(|| {
        blocks![
            caption("The inventory of this pool is not available. Reconnect to see this object.")
                .key("general-no-inventory")
        ]
    })
}

/// The tabs this area has a body for, by kind.
fn provides(tab: TabId, kind: ObjectKind) -> bool {
    use ObjectKind as K;
    match tab {
        TabId::General => matches!(
            kind,
            K::Pool
                | K::Host
                | K::Vm
                | K::Template
                | K::StorageRepository
                | K::Network
                | K::VirtualDisk
                | K::Connection
        ),
        TabId::Memory => matches!(kind, K::Pool | K::Host | K::Vm | K::Template),
        TabId::Storage => matches!(
            kind,
            K::Pool | K::Host | K::Vm | K::Template | K::StorageRepository
        ),
        TabId::Networking => matches!(
            kind,
            K::Pool | K::Host | K::Vm | K::Template | K::Network
        ),
        TabId::Nics => kind == K::Host,
        _ => false,
    }
}

fn body(m: &Model, view: &View, tab: TabId, object: &ObjectRef) -> Option<Vec<Block>> {
    use ObjectKind as K;
    if !provides(tab, object.kind) {
        return None;
    }
    let vm = object.vm_id();
    let host = object.host_id();
    Some(match (tab, object.kind) {
        (TabId::General, K::Vm) => {
            // After the fields, as in the Rune details pane: why each power
            // action is or is not available (the power area's section; the
            // actions themselves are commands), then the VM's operations.
            let mut blocks = information::vm_general(view, vm?);
            blocks.extend(
                crate::areas::power::availability_section(m, object).map(Block::from),
            );
            blocks.push(operations::vm_operations(m, object.pool?, vm?).into());
            blocks
        }
        (TabId::General, K::Template) => information::vm_general(view, vm?),
        (TabId::General, K::Host) => information::host_general(view, host?),
        (TabId::General, K::Pool) => information::pool_general(view),
        (TabId::General, K::StorageRepository) => resources::storage_general(m, view, object),
        (TabId::General, K::Network) => resources::network_general(view, object),
        (TabId::General, K::VirtualDisk) => resources::disk_general(view, object),
        (TabId::Memory, K::Vm | K::Template) => information::vm_memory(view, vm?),
        (TabId::Memory, K::Host) => information::host_memory(view, host?),
        (TabId::Memory, K::Pool) => information::pool_memory(view),
        (TabId::Storage, K::Vm | K::Template) => resources::vm_storage(view, vm?),
        (TabId::Storage, K::Host) => resources::host_storage(view, host?),
        (TabId::Storage, K::Pool) => resources::pool_storage(view),
        (TabId::Storage, K::StorageRepository) => resources::repository_storage(view, object),
        (TabId::Networking, K::Vm | K::Template) => resources::vm_networking(view, vm?),
        (TabId::Networking, K::Host) => resources::host_networking(view, host?),
        (TabId::Networking, K::Pool) => resources::pool_networking(view),
        (TabId::Networking, K::Network) => resources::network_networking(view, object),
        (TabId::Nics, K::Host) => resources::host_nics(view, host?),
        _ => return None,
    })
}

// -- Scenes -------------------------------------------------------------------

fn scenes() -> Vec<SceneSpec> {
    vec![SceneSpec::new(
        "general-information",
        "general",
        scene_information,
    )]
}

/// The Rune `inventory_information::scene`: the demo pool, its first running
/// guest given a runtime metrics record, that guest selected, General shown.
pub(crate) fn scene_information(m: &mut Model) {
    use xenadmin_core::{ClassAvailability, PowerState, VmMetrics, VmMetricsId};
    m.load_demo(3, 48);
    let Some(mut inventory) = m.inventory.as_deref().cloned() else {
        return;
    };
    let Some(vm) = inventory
        .guests()
        .find(|v| v.power_state == PowerState::Running)
        .map(|v| v.id)
    else {
        return;
    };
    let id = VmMetricsId::from(uuid::Uuid::from_u128(0x900));
    if let Some(details) = inventory.details.vms.get_mut(&vm) {
        details.metrics = Some(id);
    }
    inventory.details.vm_metrics.insert(
        id,
        VmMetrics {
            id,
            memory_actual: Some(3 << 30),
            vcpus_number: Some(2),
            start_time: Some("2026-11-01T00:00:00Z".into()),
            install_time: Some("2026-01-01T00:00:00Z".into()),
            last_updated: Some("2026-11-08T00:00:00Z".into()),
            state: vec!["running".into()],
            current_domain_type: Some("hvm".into()),
        },
    );
    inventory
        .details
        .classes
        .insert("vm_metrics".into(), ClassAvailability::Available);
    let pool = inventory.pool_id;
    m.inventory = Some(std::sync::Arc::new(inventory));
    m.select(Target::one(ObjectRef::vm(pool, vm)));
    m.set_active_tab(TabId::General);
}
