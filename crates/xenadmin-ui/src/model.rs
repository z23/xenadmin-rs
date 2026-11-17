//! `Model`: the toolkit-free successor of the Rune `State`.
//!
//! This is the first version (task K1). It has what the vocabulary, the
//! command table, the scenes and the harness need to work end to end: the
//! area states, the intent outbox, the demo flag and the demo inventory, a
//! selection, the tick request, and the lookups through the areas.
//!
//! # What K2 fills in
//!
//! The seams are marked `K2:` below. In the order they block area ports:
//!
//! 1. **Sessions** (`session.rs`): the saved profiles and their connections.
//!    `backend`, `connection`, `snapshot` and `inventory` become those of the
//!    scope session and are re-pointed when the selection moves to another
//!    pool; an open surface of the previous pool is closed on that switch
//!    (call every area's `on_scope_change`, as [`Model::select`] does now for
//!    any scope change). `waker`, `setup(waker)`.
//! 2. **Shared helpers of `impl State`** with unchanged meaning:
//!    `can_mutate()` and the rest (map `rune-kernel.md`, 19.5), `operations`,
//!    `operation_rows`, `resource_operations`, `resource_rows`,
//!    `merge_operation_rows`, `preferences`.
//! 3. **`poll`**: today it only stores the clock and asks each area. The
//!    kernel part of the Rune `poll` (map section 14) goes in front of that
//!    loop.
//! 4. **Selection and navigation** (`selection.rs`, `nav.rs`): Back and
//!    Forward, the last tab per kind, the navigation views and their tree
//!    rows, the Notifications view with [`crate::tabs::NOTIFICATION_TABS`],
//!    re-selecting the parent when the object disappears.
//!    [`Model::host_live`] is a placeholder.
//! 5. **Kernel commands and scenes**: Back, Forward, the View toggles and
//!    the interface scale commands in `command.rs` (`kernel_commands`); the
//!    kernel scenes in `scene.rs` (`kernel_scenes`) set only what this model
//!    has and must be completed.
//! 6. **Permissions** (`permissions.rs`) and **preferences** (`prefs.rs`).
//! 7. **The layering order** of `areas::AREAS`, once areas have landed.
//!
//! Rules that hold now and must keep holding: every method returns without
//! waiting for a host; nothing here calls `block_on`; an area's state is
//! reached only as `m.areas.<id>` by that area.

use std::sync::Arc;

use xenadmin_core::Inventory;

use crate::areas::{self, Areas};
use crate::command::{self, CommandId, Eligibility, Refused};
use crate::intent::{FileRequest, Intent};
use crate::object::{ObjectKind, ObjectRef, Target};
use crate::selection::Selection;
use crate::surface::Surface;
use crate::tabs::{self, TabBody, TabId};

/// The type core handles take in `notify_on_change`.
pub type Waker = Arc<dyn Fn() + Send + Sync>;

pub struct Model {
    /// One `State` per area. An area touches only its own.
    pub areas: Areas,
    /// The outbox. The front end drains it with [`Model::take_intents`]
    /// after every update.
    pub intents: Vec<Intent>,
    /// Synthetic inventory: nothing may be submitted.
    pub demo: bool,
    /// The shared error line of the Rune `State`.
    pub error: Option<String>,
    /// The inventory of the scope session. K2: re-pointed with the scope.
    pub inventory: Option<Arc<Inventory>>,
    pub selection: Selection,
    clock: f64,
    tick: Option<f64>,
}

impl Default for Model {
    fn default() -> Self {
        Self::fixture()
    }
}

impl Model {
    /// The state scenes and tests start from: no backend, nothing connected,
    /// nothing selected, default area states. The counterpart of the Rune
    /// `init()` for fixtures. K2: add the constructor for the real
    /// application next to it.
    pub fn fixture() -> Self {
        Self {
            areas: Areas::default(),
            intents: Vec::new(),
            demo: false,
            error: None,
            inventory: None,
            selection: Selection::default(),
            clock: 0.0,
            tick: None,
        }
    }

    /// Loads the synthetic pool of the `demo` scenes and sets the demo flag.
    /// The counterpart of the Rune `demo_sized`. K2: clear the session state
    /// the Rune function clears and rebuild the rows.
    pub fn load_demo(&mut self, hosts: usize, vms: usize) {
        let mut inventory = crate::demo::demonstration(hosts, vms);
        crate::demo_details::populate(&mut inventory);
        self.demo = true;
        self.error = None;
        self.inventory = Some(Arc::new(inventory));
        self.selection = Selection::default();
    }

    // -- Surfaces -----------------------------------------------------------

    /// Every open surface in layering order: the last is in front and is the
    /// one that takes input. The order across areas is that of
    /// [`areas::AREAS`].
    pub fn surfaces(&self) -> Vec<Surface> {
        areas::specs()
            .iter()
            .flat_map(|area| (area.surfaces)(self))
            .collect()
    }

    /// The surface in front, when one is open.
    pub fn front_surface(&self) -> Option<Surface> {
        self.surfaces().pop()
    }

    // -- Selection and tabs -------------------------------------------------

    /// The tree scope.
    pub fn scope(&self) -> &Target {
        &self.selection.scope
    }

    /// What a command acts on: the grid selection of the active tab when
    /// there is one, otherwise the tree scope.
    pub fn target(&self) -> Target {
        self.selection.target().clone()
    }

    /// Moves the tree scope. The grid selection is cleared, and every area
    /// hears about a change through `on_scope_change`.
    /// K2: history, last tab per kind, re-pointing the session.
    pub fn select(&mut self, scope: Target) {
        if self.selection.scope == scope {
            return;
        }
        self.selection.scope = scope;
        self.selection.grid = Target::none();
        for area in areas::specs() {
            (area.on_scope_change)(self);
        }
    }

    /// Sets the grid selection of the active tab: the target of its commands.
    pub fn select_in_grid(&mut self, grid: Target) {
        self.selection.grid = grid;
    }

    /// Whether a host counts as live for the tab matrix.
    /// K2: placeholder. It is true when the host is in the inventory; the
    /// real rule needs the session state and the host metrics.
    pub fn host_live(&self, object: &ObjectRef) -> bool {
        let Some(inventory) = &self.inventory else {
            return false;
        };
        object
            .host_id()
            .is_some_and(|host| inventory.hosts.contains_key(&host))
    }

    /// The tabs of the scope, in the order of the tab matrix.
    pub fn tabs(&self) -> &'static [TabId] {
        let scope = &self.selection.scope;
        let host_live = scope
            .single_of(ObjectKind::Host)
            .is_some_and(|host| self.host_live(host));
        tabs::tabs_for(scope.kind(), host_live)
    }

    /// The tab that is shown: the one the user chose when the scope has it,
    /// otherwise the first tab of the scope.
    pub fn active_tab(&self) -> TabId {
        let tabs = self.tabs();
        self.selection
            .tab
            .filter(|tab| tabs.contains(tab))
            .unwrap_or(tabs[0])
    }

    /// Chooses a tab. The grid selection belongs to the tab and is cleared.
    pub fn set_active_tab(&mut self, tab: TabId) {
        if self.selection.tab != Some(tab) {
            self.selection.tab = Some(tab);
            self.selection.grid = Target::none();
        }
    }

    /// The body of a tab for the scope: the first provider of
    /// [`tabs::providers`] that returns one. `None` when no area provides
    /// the tab yet.
    pub fn tab_body(&self, tab: TabId) -> Option<TabBody> {
        let scope = &self.selection.scope;
        // The harness tests need a tab body before any area provides one.
        #[cfg(test)]
        if let Some(body) = crate::harness::probe::tab(self, tab, scope) {
            return Some(body);
        }
        tabs::providers(tab, scope.kind())
            .iter()
            .filter_map(|id| areas::area(id))
            .find_map(|area| (area.tab)(self, tab, scope))
    }

    // -- Commands -----------------------------------------------------------

    /// The eligibility of a command on a target. `Hidden` for an unknown id.
    pub fn eligibility(&self, id: CommandId, target: &Target) -> Eligibility {
        command::eligibility(self, id, target)
    }

    /// Runs a command on a target, after evaluating its eligibility again.
    /// A command that is not `Enabled` is refused and nothing runs.
    pub fn try_invoke(&mut self, id: CommandId, target: &Target) -> Result<(), Refused> {
        let spec = command::find(id).ok_or(Refused::Unknown(id))?;
        match (spec.eligibility)(self, target) {
            Eligibility::Enabled => {
                (spec.invoke)(self, target);
                Ok(())
            }
            Eligibility::Hidden => Err(Refused::Hidden(id)),
            Eligibility::Disabled(reason) => Err(Refused::Disabled(reason)),
        }
    }

    /// [`Model::try_invoke`] for a caller with nowhere to show a refusal: the
    /// reason goes to [`Model::error`]. Returns whether the command ran.
    pub fn invoke(&mut self, id: CommandId, target: &Target) -> bool {
        match self.try_invoke(id, target) {
            Ok(()) => true,
            Err(refused) => {
                self.error = Some(refused.to_string());
                false
            }
        }
    }

    // -- Intents ------------------------------------------------------------

    /// Queues an intent for the front end.
    pub fn send(&mut self, intent: Intent) {
        self.intents.push(intent);
    }

    /// Asks the user for a file or directory. The request's reply runs with
    /// the answer.
    pub fn choose_file(&mut self, request: FileRequest) {
        self.send(Intent::ChooseFile(request));
    }

    pub fn open_url(&mut self, url: impl Into<String>) {
        self.send(Intent::OpenUrl(url.into()));
    }

    /// Puts text on the clipboard. Only on an explicit user action.
    pub fn copy_text(&mut self, text: impl Into<String>) {
        self.send(Intent::CopyText(text.into()));
    }

    /// Reads the clipboard text. Only on an explicit user action.
    pub fn read_clipboard(&mut self, reply: impl Fn(&mut Model, Option<String>) + 'static) {
        self.send(Intent::ReadClipboard(std::rc::Rc::new(reply)));
    }

    pub fn quit(&mut self) {
        self.send(Intent::Quit);
    }

    /// Empties the outbox. The front end calls this after every update and
    /// performs what it gets, in order.
    pub fn take_intents(&mut self) -> Vec<Intent> {
        std::mem::take(&mut self.intents)
    }

    // -- Time ---------------------------------------------------------------

    /// Asks to be polled again in `seconds` even if no core handle wakes the
    /// model (the Rune `tx.wake_after`). The earliest request wins.
    pub fn request_tick(&mut self, seconds: f64) {
        let seconds = seconds.max(0.0);
        self.tick = Some(self.tick.map_or(seconds, |tick| tick.min(seconds)));
    }

    /// The pending tick request in seconds. The front end takes it after
    /// every update and schedules one call of [`Model::poll`].
    pub fn take_tick_request(&mut self) -> Option<f64> {
        self.tick.take()
    }

    /// The monotonic clock of the last poll, in seconds.
    pub fn clock(&self) -> f64 {
        self.clock
    }

    /// Reads the latest state of every core handle. Called on every wake
    /// and timed tick with the monotonic clock in seconds. Never waits.
    /// Returns whether anything shown may have changed.
    /// K2: the kernel part of the Rune `poll` goes before the area loop.
    pub fn poll(&mut self, clock: f64) -> bool {
        self.clock = clock;
        let mut changed = false;
        for area in areas::specs() {
            changed |= (area.poll)(self, clock);
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xenadmin_core::{HostId, VmId};

    #[test]
    fn a_fixture_model_shows_home_and_has_nothing_open() {
        let m = Model::fixture();
        assert!(m.surfaces().is_empty());
        assert!(m.front_surface().is_none());
        assert_eq!(m.tabs(), [TabId::Home, TabId::Search]);
        assert_eq!(m.active_tab(), TabId::Home);
        assert!(m.target().is_empty());
        // No area is ported yet, so no tab has a body.
        assert!(m.tab_body(TabId::Home).is_none());
    }

    #[test]
    fn the_tabs_follow_the_scope_and_the_chosen_tab_survives_where_it_exists() {
        let mut m = Model::fixture();
        m.load_demo(3, 48);
        let inventory = m.inventory.clone().expect("inventory");
        let pool = inventory.pool_id;
        let vm = *inventory.vms.keys().next().expect("vm");
        let host = *inventory.hosts.keys().next().expect("host");

        m.select(Target::one(ObjectRef::vm(pool, vm)));
        assert_eq!(m.active_tab(), TabId::General);
        m.set_active_tab(TabId::Snapshots);
        assert_eq!(m.active_tab(), TabId::Snapshots);

        // A live host has no Snapshots tab: the first tab shows instead.
        m.select(Target::one(ObjectRef::host(pool, host)));
        assert!(m.tabs().contains(&TabId::Console));
        assert_eq!(m.active_tab(), TabId::General);

        // A host the inventory does not know is not live.
        let unknown = HostId::parse("20000000-0000-0000-0000-00000000ffff").expect("host id");
        m.select(Target::one(ObjectRef::host(pool, unknown)));
        assert_eq!(m.tabs(), [TabId::General, TabId::Search]);

        m.select(Target::many([
            ObjectRef::vm(pool, vm),
            ObjectRef::host(pool, host),
        ]));
        assert_eq!(m.tabs(), [TabId::Search]);
    }

    #[test]
    fn the_grid_selection_is_the_target_until_the_scope_or_the_tab_changes() {
        let mut m = Model::fixture();
        m.load_demo(3, 48);
        let pool = m.inventory.clone().expect("inventory").pool_id;
        let vm = |n: u8| {
            ObjectRef::vm(
                pool,
                VmId::parse(&format!("30000000-0000-0000-0000-0000000000{n:02}")).expect("vm id"),
            )
        };
        m.select(Target::one(ObjectRef::pool(pool)));
        assert_eq!(m.target().kind(), ObjectKind::Pool);
        m.select_in_grid(Target::many([vm(1), vm(2)]));
        assert_eq!(m.target().len(), 2);
        assert_eq!(m.scope().kind(), ObjectKind::Pool);
        m.set_active_tab(TabId::Storage);
        assert_eq!(m.target().kind(), ObjectKind::Pool);
        m.select_in_grid(Target::one(vm(1)));
        m.select(Target::one(vm(2)));
        assert_eq!(m.target(), Target::one(vm(2)));
    }

    #[test]
    fn the_earliest_tick_request_wins_and_is_taken_once() {
        let mut m = Model::fixture();
        assert_eq!(m.take_tick_request(), None);
        m.request_tick(2.0);
        m.request_tick(0.5);
        m.request_tick(1.0);
        assert_eq!(m.take_tick_request(), Some(0.5));
        assert_eq!(m.take_tick_request(), None);
        assert!(!m.poll(12.5));
        assert_eq!(m.clock(), 12.5);
    }
}
