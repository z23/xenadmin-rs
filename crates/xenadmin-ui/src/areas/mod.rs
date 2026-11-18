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
use crate::properties::{PageEvent, PageView, PropertyPage};
use crate::scene::SceneSpec;
use crate::surface::Surface;
use crate::tabs::{TabBody, TabId};

#[cfg(test)]
pub(crate) mod example_tests;

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
    /// The active tab is another one than before: `from` was shown, `to` is
    /// shown now ([`Model::active_tab`]). It runs after whatever changed it
    /// has finished: [`Model::set_active_tab`], a change of the scope whose
    /// new object shows another tab (after `on_scope_change`), Back and
    /// Forward, and a change of the navigation view (the Notifications view
    /// has its own two tabs). It does not run while the tab stays the same
    /// for another object, nor for a tab that disappears under the selection
    /// without any of those calls (a server that stopped being live).
    pub on_tab_change: fn(&mut Model, TabId, TabId),
    /// The scope session changed, was reset or is being disconnected: the
    /// model is about to show another pool, or none. An area drops here
    /// everything that belongs to the view of the previous pool (what the
    /// Rune `clear_view`, `disconnect_selected` and `demo_sized` cleared of
    /// its modules). It runs while `m.connection`, `m.snapshot` and
    /// `m.inventory` still describe the previous session, and before
    /// `on_scope_change`.
    pub on_session_change: fn(&mut Model),
    /// The rows of a navigation view the area provides
    /// ([`crate::nav::NavView::provider`] says who is asked): Organization
    /// from `organization`, Saved Searches from `search`. Parents before
    /// children, ids unique within the view.
    pub nav_rows: fn(&Model, crate::nav::NavView) -> Vec<crate::nav::NavRow>,
    /// The navigation view changed ([`Model::set_nav_view`]): `m.nav.view`
    /// is the new one. The scope did not change. An area whose cached rows
    /// depend on the view brings them up to date here.
    pub on_nav_change: fn(&mut Model),
    /// The pages the area provides for the Properties dialog of this
    /// target ([`crate::properties`]). Asked for every target a Properties
    /// command is resolved on and on every render of the open dialog: pure
    /// and cheap. None for a target that is not the area's.
    pub property_pages: fn(&Model, &Target) -> Vec<PropertyPage>,
    /// The body, footer buttons and footer note of the area's page with
    /// this id, which is the current page of the open Properties dialog of
    /// this target. Only the current page is asked for.
    pub property_page: fn(&Model, &Target, &str) -> PageView,
    /// A page of the area was entered or left, or the Properties dialog
    /// closed ([`PageEvent`]). The area creates the draft of its page when
    /// it is entered and drops it when it is left.
    pub on_property_page: fn(&mut Model, &Target, PageEvent),
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

fn no_tab_change(_: &mut Model, _: TabId, _: TabId) {}

fn no_nav_rows(_: &Model, _: crate::nav::NavView) -> Vec<crate::nav::NavRow> {
    Vec::new()
}

fn no_property_pages(_: &Model, _: &Target) -> Vec<PropertyPage> {
    Vec::new()
}

fn no_property_page(_: &Model, _: &Target, _: &str) -> PageView {
    PageView::default()
}

fn no_property_event(_: &mut Model, _: &Target, _: PageEvent) {}

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
            on_tab_change: no_tab_change,
            on_session_change: no_scope_change,
            nav_rows: no_nav_rows,
            on_nav_change: no_scope_change,
            property_pages: no_property_pages,
            property_page: no_property_page,
            on_property_page: no_property_event,
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
            /// The area the tests of the Properties dialog drive. Tests only.
            #[cfg(test)]
            pub pages: crate::properties::tests::State,
        }

        /// The `spec` function of every area, in layering order.
        pub const AREAS: &[fn() -> AreaSpec] = &[
            $($id::spec,)*
            #[cfg(test)]
            example_tests::spec,
            #[cfg(test)]
            crate::harness::probe::spec,
            #[cfg(test)]
            crate::properties::tests::spec,
        ];

        /// The area ids, in the same order (without the test-only areas).
        pub const AREA_IDS: &[&str] = &[$(stringify!($id),)*];
    };
}

// The order is the layering order of surfaces: `Model::surfaces` stacks the
// open surfaces of the areas in this order, so a later area's surface is in
// front of an earlier one's, and the surface in front is the only one that
// takes input. Within one area the area's own `surfaces` function gives the
// order. The experimental notice follows the surface in front
// (`Model::experimental_notice`): a supported surface in front of an
// experimental one suppresses it.
//
// The order is set from the Rune `view()` (map `rune-kernel.md`, section
// 13). Rune drew one exclusive chain in which the first open dialog of a
// fixed ranking hid the others, then independent layers over it. An earlier
// rank in the chain is therefore "in front", and the modules rely on that
// for drill-down: a dialog opens a child that ranks before it. The list is
// the chain in reverse rank (back to front), each area at the rank of its
// backmost dialog, then the independent layers back to front:
//
//   chain:  power (force review 29, placement 26, recovery 23; the
//           unknown outcome review 30 is the events area's dialog now, in
//           front), snapshots (28), vm_properties (27), vm_lifecycle
//           (25, cloud-init 20), migration (24), cross_pool (22), transfers
//           (21, local conversion 7), network (generic resource editor 19,
//           network workspace 10) with its children network_services and
//           management_network, pool (pool editor 18, dissolve 17),
//           pool_security (16), storage (15), vm_devices (disk workspace
//           14), organization (12, vApps 11), host_admin (9), performance
//           (host monitoring 8), storage_maintenance (storage graph 6),
//           host_services (support 5, power 4), schedules_alarms (snapshot
//           schedules 3, alarms 2), connections (the connect form 1)
//   layers: search (32), bulk_export (33), bulk_edit (bulk delete 33, bulk
//           properties 34), general (38), settings (39), console (console
//           windows 40), vm_tools (external tools 41), events (outcome
//           explanation 42)
//
// Where one area had dialogs at several ranks, the choices are:
//
// - `events` is in front of everything. Its Rune Activity dialog (13) is the
//   Notifications view now and no longer a surface; what remains are the
//   outcome explanation (the front-most Rune layer, opened from other areas'
//   surfaces), the cancel and dismiss reviews and the acknowledgement.
// - `vm_tools` is at the rank of External Tools (41), which Options opens;
//   its guest tools dialog (37) is opened from a tab and never over another
//   area's surface.
// - `power`, `migration` and `cross_pool` are at the rank of their chain
//   dialogs. Their batch dialogs were layers (33) because Rune opened them
//   from the row menu over the Search and Connections dialogs, which are
//   tabs now.
// - `snapshots` is at the rank of its dialog (28); snapshot inspection (36)
//   opens from the Snapshots tab.
// - `connections` is at the rank of the connect form: in front of the whole
//   chain. The Connections manager (31) is the Home tab now.
//
// If a child surface turns out behind its opener, the opener closes itself
// when it opens the child, or the order here changes; the test below lists
// the drill-downs that are known.
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
    general,
    settings,
    console,
    vm_tools,
    events,
);

/// The spec of every area, in layering order, and after them the kernel's
/// own entry (`crate::properties::spec`, id `kernel`): the kernel hears of
/// a scope change and a session change through the same two hooks, after
/// every area. Built once.
pub fn specs() -> &'static [AreaSpec] {
    static SPECS: OnceLock<Vec<AreaSpec>> = OnceLock::new();
    SPECS.get_or_init(|| {
        AREAS
            .iter()
            .map(|spec| spec())
            .chain([crate::properties::spec()])
            .collect()
    })
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
            // Reconnect As from the pool security dialog.
            ("pool_security", "connections"),
            // Snapshot export from the snapshot dialogs.
            ("snapshots", "bulk_export"),
            ("snapshots", "transfers"),
            // Options opens External Tools; the console windows manager is
            // over Options.
            ("settings", "console"),
            ("settings", "vm_tools"),
            ("console", "vm_tools"),
        ] {
            assert!(position(opener) < position(child), "{opener} -> {child}");
        }
        // "Explain outcome" opens from the surfaces of every other area.
        assert_eq!(AREA_IDS.last(), Some(&"events"));
        // The power confirmation is behind every other dialog, as in Rune.
        // (The unknown-outcome acknowledgement is one dialog of `events`.)
        assert_eq!(AREA_IDS.first(), Some(&"power"));
    }

    #[test]
    fn the_experimental_notice_follows_the_surface_in_front() {
        // Rune: main.rs `dialogs_outside_the_supported_core_carry_the_experimental_notice`,
        // the rule that a supported dialog in front of an experimental one
        // suppresses the notice. The example area's dialog is experimental,
        // the probe's is not, and the probe is the later area.
        let mut m = Model::demo(3, 48);
        assert!(!m.experimental_notice(), "nothing is open");
        let inventory = m.inventory.clone().expect("inventory");
        let vm = *inventory.vms.keys().next().expect("a VM");
        m.select(crate::object::Target::one(crate::object::ObjectRef::vm(
            inventory.pool_id,
            vm,
        )));
        let target = m.target();
        assert!(m.invoke(example_tests::RENAME, &target));
        assert!(m.experimental_notice());
        m.areas.probe.open = true;
        let surfaces = m.surfaces();
        assert_eq!(
            surfaces
                .iter()
                .map(|surface| surface.id.as_str())
                .collect::<Vec<_>>(),
            ["example-rename", "probe"]
        );
        assert!(!surfaces[1].experimental);
        assert!(!m.experimental_notice(), "a supported surface is in front");
        m.areas.probe.open = false;
        assert!(m.experimental_notice());
    }
}
