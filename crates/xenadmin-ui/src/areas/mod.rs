//! Areas: the feature groups of the client, one directory each.
//!
//! An area owns its state (`m.areas.<id>`), its commands, its scenes, the
//! surfaces it has open and the tab bodies it provides. It hands all of that
//! to the kernel through one [`AreaSpec`], returned by `areas::<id>::spec()`.
//! Nothing outside an area's directory needs editing to add a command, a
//! scene, a surface, a tab or state to it.
//!
//! An area never reaches into another area's `State`. It calls the other
//! area's public functions or invokes a command.

use std::sync::OnceLock;

use crate::command::CommandSpec;
use crate::model::Model;
use crate::object::Target;
use crate::scene::SceneSpec;
use crate::surface::Surface;
use crate::tabs::{TabBody, TabId};

#[cfg(test)]
mod example_tests;

/// What an area gives the kernel. Every field but `id` has a default that
/// does nothing: write `AreaSpec { commands, surfaces, ..AreaSpec::new("id") }`.
#[derive(Clone, Copy)]
pub struct AreaSpec {
    /// The directory name and the field of [`Areas`].
    pub id: &'static str,
    /// The area's commands, for [`crate::command::registry`].
    pub commands: fn() -> Vec<CommandSpec>,
    /// The area's scenes, for [`crate::scene::registry`].
    pub scenes: fn() -> Vec<SceneSpec>,
    /// The surfaces the area has open, back to front. Usually none or one.
    pub surfaces: fn(&Model) -> Vec<Surface>,
    /// The body of a tab for the tree scope, when the area provides that tab
    /// for that scope ([`crate::tabs::providers`] says who is asked). `None`
    /// passes the question to the next provider.
    pub tab: fn(&Model, TabId, &Target) -> Option<TabBody>,
    /// Reads the area's core handles. Called on every wake and timed tick
    /// with the monotonic clock in seconds. Never waits. Returns whether
    /// anything the area shows changed.
    pub poll: fn(&mut Model, f64) -> bool,
    /// The tree scope changed. An area closes an editor that belongs to the
    /// previous scope here.
    pub on_scope_change: fn(&mut Model),
}

fn no_commands() -> Vec<CommandSpec> {
    Vec::new()
}

fn no_scenes() -> Vec<SceneSpec> {
    Vec::new()
}

fn no_surfaces(_: &Model) -> Vec<Surface> {
    Vec::new()
}

fn no_tab(_: &Model, _: TabId, _: &Target) -> Option<TabBody> {
    None
}

fn no_poll(_: &mut Model, _: f64) -> bool {
    false
}

fn no_scope_change(_: &mut Model) {}

impl AreaSpec {
    /// An area that provides nothing yet.
    pub const fn new(id: &'static str) -> Self {
        Self {
            id,
            commands: no_commands,
            scenes: no_scenes,
            surfaces: no_surfaces,
            tab: no_tab,
            poll: no_poll,
            on_scope_change: no_scope_change,
        }
    }
}

/// Declares the areas once: the modules, the [`Areas`] struct with one field
/// per area, and the [`AREAS`] list, all in the order given.
macro_rules! areas {
    ($($id:ident),* $(,)?) => {
        $(pub mod $id;)*

        /// The state of every area. `m.areas.<id>` is that area's `State`.
        #[derive(Default)]
        pub struct Areas {
            $(pub $id: $id::State,)*
            /// The worked example of `MODEL-API.md`. Tests only.
            #[cfg(test)]
            pub example: example_tests::State,
            /// The area the harness tests drive. Tests only.
            #[cfg(test)]
            pub probe: crate::harness::probe::State,
        }

        /// The `spec` function of every area, in layering order.
        pub const AREAS: &[fn() -> AreaSpec] = &[
            $($id::spec,)*
            #[cfg(test)]
            example_tests::spec,
            #[cfg(test)]
            crate::harness::probe::spec,
        ];

        /// The area ids, in the same order (without the two test-only areas).
        pub const AREA_IDS: &[&str] = &[$(stringify!($id),)*];
    };
}

// The order is the layering order of surfaces: `Model::surfaces` stacks the
// open surfaces of the areas in this order, so a later area's surface is in
// front of an earlier one's. Within one area the area's own `surfaces`
// function gives the order.
//
// It is derived from the Rune `view()` (map `rune-kernel.md`, section 13).
// Rune drew one exclusive chain where the first open dialog of a fixed
// ranking hid the others, then independent layers over it. An earlier rank
// in the chain is therefore "in front", and the modules rely on that for
// drill-down: a dialog opens a child that ranks before it. So the list
// below is the chain in reverse rank (back to front), then the independent
// layers back to front:
//
//   chain:  power (force review, unknown outcome, placement, recovery),
//           snapshots, vm_properties, vm_lifecycle (and cloud-init),
//           migration, cross_pool, transfers, network (generic resource
//           editor, network workspace) with its children network_services
//           and management_network, pool (pool editor, dissolve),
//           pool_security, storage, vm_devices (disk workspace), events
//           (activity), organization (and vApps), host_admin, performance
//           (host monitoring), storage_maintenance (storage graph),
//           host_services (support, power), schedules_alarms, connections
//   layers: search, bulk_export, bulk_edit, vm_tools, general, settings,
//           console
//
// Known approximations, for the kernel engineer to refine when the areas
// have landed: an area has one position although Rune ranked its dialogs
// separately (the connection manager was a layer while the connect form led
// the chain; the bulk dialogs of power, migration and cross_pool were
// layers; snapshot inspection and the outcome explanation were front
// layers). If a child surface turns out behind its opener, the opener
// should close itself or the order here should change.
areas!(
    power,
    snapshots,
    vm_properties,
    vm_lifecycle,
    migration,
    cross_pool,
    transfers,
    network,
    network_services,
    management_network,
    pool,
    pool_security,
    storage,
    vm_devices,
    events,
    organization,
    host_admin,
    performance,
    storage_maintenance,
    host_services,
    schedules_alarms,
    connections,
    search,
    bulk_export,
    bulk_edit,
    vm_tools,
    general,
    settings,
    console,
);

/// The spec of every area, in layering order. Built once.
pub fn specs() -> &'static [AreaSpec] {
    static SPECS: OnceLock<Vec<AreaSpec>> = OnceLock::new();
    SPECS.get_or_init(|| AREAS.iter().map(|spec| spec()).collect())
}

/// The spec of the area with this id.
pub fn area(id: &str) -> Option<&'static AreaSpec> {
    specs().iter().find(|spec| spec.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_area_of_the_assignment_has_a_directory_and_its_own_id() {
        // The ids of notes/briefs/AREAS.md, in its order.
        let mut assigned = [
            "connections",
            "general",
            "search",
            "settings",
            "power",
            "events",
            "console",
            "snapshots",
            "performance",
            "host_admin",
            "migration",
            "vm_properties",
            "vm_devices",
            "vm_lifecycle",
            "bulk_edit",
            "vm_tools",
            "storage",
            "storage_maintenance",
            "network",
            "network_services",
            "management_network",
            "host_services",
            "pool",
            "pool_security",
            "transfers",
            "bulk_export",
            "cross_pool",
            "organization",
            "schedules_alarms",
        ];
        assigned.sort_unstable();
        let mut declared = AREA_IDS.to_vec();
        declared.sort_unstable();
        assert_eq!(declared, assigned);
        // Each spec carries the id of its module, so `area(id)` finds it.
        for (id, spec) in AREA_IDS.iter().zip(specs()) {
            assert_eq!(spec.id, *id);
            assert!(area(id).is_some());
        }
        assert!(area("no-such-area").is_none());
    }

    #[test]
    fn a_child_surface_is_in_front_of_the_surface_that_opens_it() {
        let position = |id: &str| {
            AREA_IDS
                .iter()
                .position(|area| *area == id)
                .unwrap_or_else(|| panic!("{id} is not an area"))
        };
        // Opener, then the child it drills down to (map rune-kernel, 13).
        for (opener, child) in [
            ("network", "pool"),
            ("network", "host_admin"),
            ("pool", "pool_security"),
            ("host_admin", "host_services"),
            ("host_admin", "performance"),
            ("host_admin", "storage_maintenance"),
            ("storage", "storage_maintenance"),
            ("network", "network_services"),
            ("network", "management_network"),
        ] {
            assert!(position(opener) < position(child), "{opener} -> {child}");
        }
    }
}
