//! `Model`: the toolkit-free successor of the Rune `State`.
//!
//! It keeps the names of the shared fields area code reads all the time
//! (`backend`, `connection`, `snapshot`, `inventory`, `waker`, `demo`,
//! `error`, `preferences`, `operations`, `operation_rows`,
//! `resource_operations`, `resource_rows`, `diagnostics`) and the shared
//! helpers of the Rune `impl State` with unchanged meaning.
//!
//! What is new against Rune:
//!
//! - Several pools are shown in one tree. `connection`, `snapshot`,
//!   `inventory` and `demo` are those of the **scope session**: the saved
//!   connection that owns the tree selection (see [`crate::session`]). They
//!   are re-pointed when the selection moves to another session, and every
//!   area is told (`on_session_change`, then `on_scope_change`).
//! - The selection is by [`ObjectRef`], with a history and a tab per object
//!   kind ([`crate::selection`]); the navigation pane is plain rows
//!   ([`crate::nav`]).
//! - The front end is asked for services through [`Intent`]s and polls the
//!   model with [`Model::poll`]; the model asks for a timed poll with
//!   [`Model::request_tick`].
//!
//! Rules that hold everywhere: every method returns without waiting for a
//! host; nothing here calls `block_on`; nothing is written to disk unless
//! the process owns the state directory ([`Model::owns_state`]); an area's
//! state is reached only as `m.areas.<id>` by that area.

use std::sync::Arc;

use serde::Serialize;
use xenadmin_core::{
    Backend, Connection, ConnectionSnapshot, ConnectionState, DesktopInstance, DevInspection,
    Error, HostId, Inventory, Operation, OperationControl, OperationId, OperationRecord,
    OperationState, PoolId, ProfileId, ResourceOperation, ResourceOperationRecord, ResourceTarget,
    VmCommand, VmId,
};

use crate::areas::{self, Areas};
use crate::command::{self, CommandId, Eligibility, MenuEntry, MenuItem, Refused};
use crate::diagnostics::{ActionDiagnosticContext, ActionDiagnostics};
use crate::intent::{FileRequest, Intent};
use crate::nav::{NavView, Navigation};
use crate::object::{ObjectKind, ObjectRef, Target};
use crate::prefs::Preferences;
use crate::rows::DisplayRow;
use crate::selection::{Anchor, MenuContext, MenuOrigin, MenuScope, Place, STALE_MENU, Selection};
use crate::session::{self, Workspace};
use crate::surface::Surface;
use crate::tabs::{self, TabBody, TabId};

/// The type core handles take in `notify_on_change`.
pub type Waker = Arc<dyn Fn() + Send + Sync>;

/// How long the navigation pane and column widths rest before they are
/// written, so a drag saves once.
pub const LAYOUT_SAVE_DELAY: f64 = 0.6;

/// How many VM operations may be unresolved at once
/// ([`Model::submit_tracked_command`]).
pub const MAX_OPERATIONS: usize = 64;

/// The name the window and the empty header show.
pub const APPLICATION_NAME: &str = "XenAdmin RS";

/// The banner of a synthetic inventory.
pub const DEMO_BANNER: &str = "DEMO  •  Synthetic data. Host actions are unavailable.";

pub struct Model {
    /// One `State` per area. An area touches only its own.
    pub areas: Areas,
    /// The outbox. The front end drains it with [`Model::take_intents`]
    /// after every update.
    pub intents: Vec<Intent>,

    // -- The shared fields of the Rune `State`, under their names -----------
    /// One per process. `Err` in a fixture and in a preview, with the text
    /// the connect form shows.
    pub backend: Result<Backend, String>,
    /// The connection of the scope session.
    pub connection: Option<Connection>,
    /// The latest snapshot of the scope session, as of the last poll.
    pub snapshot: Option<Arc<ConnectionSnapshot>>,
    /// The inventory of the scope session. It changes together with
    /// `snapshot`.
    pub inventory: Option<Arc<Inventory>>,
    /// Cloned into every core handle (`handle.notify_on_change(waker)`) and
    /// called by worker threads when they have a result.
    pub waker: Option<Waker>,
    /// The scope session is synthetic: nothing may be submitted.
    pub demo: bool,
    /// The shared error line.
    pub error: Option<String>,
    pub preferences: Preferences,
    /// The handles of the VM operations submitted through
    /// [`Model::submit_tracked_command`] and
    /// [`Model::submit_tracked_command_for`], of every session. A handle
    /// belongs to the session it was submitted through and stays here,
    /// polled, whichever pool the tree shows, until its operation has
    /// finished or its session's connection is replaced or removed.
    pub operations: Vec<Operation>,
    /// The session each handle in `operations` was submitted through. A
    /// handle without an entry (one an area pushed itself) belongs to the
    /// session that shows its record's pool.
    pub(crate) operation_sessions: std::collections::BTreeMap<OperationId, ProfileId>,
    /// The merged history of VM operations, oldest first.
    pub operation_rows: Vec<OperationRecord>,
    /// The handles of resource operations, of every pool.
    pub resource_operations: Vec<ResourceOperation>,
    /// The merged history of resource operations, oldest first.
    pub resource_rows: Vec<ResourceOperationRecord>,
    /// The power availability checks of one VM. The `power` area starts
    /// and shows them; the model drops them when they are no longer current.
    pub diagnostics: Option<ActionDiagnostics>,
    /// The last results of `diagnostics`, kept to detect a change.
    pub diagnostic_results: Vec<String>,
    /// The saved connections and their sessions (the Rune `workspace`).
    pub workspace: Workspace,
    /// The developer inspection socket, when it was asked for.
    pub dev_inspection: Option<DevInspection>,

    // -- The shell -----------------------------------------------------------
    pub selection: Selection,
    /// The navigation pane: view, filter, View toggles.
    pub nav: Navigation,
    /// A notice for the status bar: why the selection moved by itself.
    pub notice: Option<String>,
    /// The open Properties dialog ([`crate::properties`]). Changed only by
    /// [`Model::open_properties`], [`Model::show_properties_page`] and
    /// [`Model::close_properties`].
    pub properties: Option<crate::properties::Open>,

    /// The lock on the state directory (the Rune `_instance`). `Some` only
    /// in the one owning process; it gates every write to disk.
    instance: Option<DesktopInstance>,
    layout_changed_at: Option<f64>,
    clock: f64,
    tick: Option<f64>,
    /// A change that may move the active tab is running: its end reports
    /// the tab change once ([`Model::watch_tab`]).
    tab_watched: bool,
}

impl Default for Model {
    fn default() -> Self {
        Self::fixture()
    }
}

/// What the object header shows of the selection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Header {
    /// The kind of icon.
    pub icon: ObjectKind,
    pub name: String,
    /// The state word: a VM's power state, Enabled or Disabled for a host,
    /// the status word of a pool's or a saved connection's session.
    pub state: Option<String>,
    /// The name of the pool the object is in.
    pub pool: Option<String>,
    /// The name of the host a VM runs on.
    pub host: Option<String>,
}

/// What the status bar shows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Status {
    /// The connection state and freshness of the scope session.
    pub connection: String,
    /// Operations of both families that are not finished and not unknown.
    pub operations: usize,
    /// Operations whose outcome is unknown and not acknowledged.
    pub unknown_outcomes: usize,
    /// What the commands act on: empty, a name, or a count.
    pub selection: String,
    /// The banner of a synthetic inventory.
    pub demo: Option<&'static str>,
    /// Hosts of the scope session that are outside the qualified release.
    pub host_notice: Option<String>,
    /// Why the selection moved by itself.
    pub notice: Option<String>,
    /// The shared error line, or else the scope session's connection error.
    pub error: Option<String>,
}

/// The state filter the unknown-outcomes counter of the status bar sets
/// ([`Model::show_unknown_outcomes`]): the name of the state.
pub const UNKNOWN_OUTCOME_STATE: &str = "OutcomeUnknown";

/// The table whose column widths the preferences keep by column id alone,
/// as the Rune inventory grid stored them: the result grid of the Search
/// tab, which took its place.
pub const INVENTORY_TABLE: &str = "search-results";

/// The id a column width is kept under in `Preferences::columns`: the
/// column id for [`INVENTORY_TABLE`] (the ids a Rune preferences file
/// has), `<table>/<column>` for every other table.
fn column_width_id(table: &str, column: &str) -> String {
    if table == INVENTORY_TABLE {
        column.to_owned()
    } else {
        format!("{table}/{column}")
    }
}

impl Model {
    // -- Constructors -------------------------------------------------------

    /// The state scenes and tests start from: no backend, nothing connected,
    /// nothing selected, default preferences, default area states. It does
    /// not own the state directory, so it never reads or writes a file. The
    /// counterpart of the Rune `init()`.
    pub fn fixture() -> Self {
        Self::with_preferences(None, Preferences::default())
    }

    /// The model of the real process, which owns the state directory:
    /// `Model::live(owner, Preferences::load())`. It starts the backend,
    /// opens the profile store and, when asked for, the developer
    /// inspection socket. A start-up failure is in `error`. The counterpart
    /// of the Rune `init_with_instance(Some(owner))`.
    pub fn live(instance: DesktopInstance, preferences: Preferences) -> Self {
        Self::with_preferences(Some(instance), preferences)
    }

    /// A fixture showing the synthetic pool: `fixture()` then
    /// [`Model::load_demo`].
    pub fn demo(hosts: usize, vms: usize) -> Self {
        let mut model = Self::fixture();
        model.load_demo(hosts, vms);
        model
    }

    /// The Rune `init_with_preferences`.
    pub fn with_preferences(instance: Option<DesktopInstance>, preferences: Preferences) -> Self {
        let live_state = instance.is_some();
        let backend = if live_state {
            Backend::start().map_err(|e| e.to_string())
        } else {
            Err("UI fixtures cannot connect to hosts".into())
        };
        let (dev_inspection, inspection_error) = start_dev_inspection(live_state);
        let startup_error = live_state
            .then(|| backend.as_ref().err().cloned())
            .flatten()
            .or(inspection_error);
        let mut workspace = Workspace::load(preferences.last_profile, live_state);
        workspace.endpoint = preferences.endpoint.clone();
        workspace.username = preferences.username.clone();
        workspace.fingerprint = preferences.fingerprint.clone();
        workspace.host_certificates = preferences.host_certificates.clone();
        workspace.accept_invalid_certificates = preferences.accept_invalid_certificates;
        let mut m = Self {
            areas: Areas::default(),
            intents: Vec::new(),
            backend,
            connection: None,
            snapshot: None,
            inventory: None,
            waker: None,
            demo: false,
            error: startup_error,
            preferences,
            operations: Vec::new(),
            operation_sessions: std::collections::BTreeMap::new(),
            operation_rows: Vec::new(),
            resource_operations: Vec::new(),
            resource_rows: Vec::new(),
            diagnostics: None,
            diagnostic_results: Vec::new(),
            workspace,
            dev_inspection,
            selection: Selection::default(),
            nav: Navigation::default(),
            notice: None,
            properties: None,
            instance,
            layout_changed_at: None,
            clock: 0.0,
            tick: None,
            tab_watched: false,
        };
        session::apply_initial(&mut m);
        m
    }

    /// Whether this process owns the state directory. Only then are
    /// preferences, profiles and the keyring written.
    pub fn owns_state(&self) -> bool {
        self.instance.is_some()
    }

    /// Stores the waker and restores the saved sessions that ask for it.
    /// The front end calls this once, before the first poll. The Rune
    /// `setup`.
    pub fn setup(&mut self, waker: Waker) {
        self.waker = Some(waker);
        session::restore_sessions(self);
    }

    /// What the close of the main window runs: preferences are saved, the
    /// detached console windows are closed and every session is
    /// disconnected. (The Rune `App::on_close`. The windows are closed
    /// here and not by the `console` area's `on_session_change`: that hook
    /// also runs when the main window shows another pool, where a detached
    /// console must stay.)
    pub fn close(&mut self) {
        self.save_preferences();
        areas::console::windows::close_all(self);
        session::disconnect_all(self);
    }

    /// Shows the synthetic pool of the `demo` scenes: a synthetic session
    /// becomes the scope session, `demo` is set and the pool is selected.
    /// The counterpart of the Rune `demo_sized`: the operation lists, the
    /// error line and every area's view of the previous session are dropped.
    pub fn load_demo(&mut self, hosts: usize, vms: usize) {
        let mut inventory = crate::demo::demonstration(hosts, vms);
        crate::demo_details::populate(&mut inventory);
        session::clear_view(self);
        self.operations.clear();
        self.operation_sessions.clear();
        self.resource_operations.clear();
        self.resource_rows.clear();
        self.operation_rows.clear();
        self.connection = None;
        self.error = None;
        self.workspace.show_connect = false;
        let profile = crate::demo::profile();
        let id = profile.id;
        let pool = inventory.pool_id;
        self.workspace
            .preview_profiles
            .retain(|other| other.id != id);
        self.workspace.preview_profiles.push(profile);
        let inventory = Arc::new(inventory);
        self.workspace.synthetic.insert(id, inventory.clone());
        self.workspace.selected = Some(id);
        self.inventory = Some(inventory);
        self.demo = true;
        self.selection = Selection::default();
        self.nav.view = NavView::Infrastructure;
        self.nav.filter.clear();
        self.notice = None;
        self.go(Target::one(ObjectRef::pool(pool)), None);
    }

    /// Adds a second synthetic pool beside the ones there are: a saved
    /// connection with this name whose session has the inventory and no
    /// connection. For scenes and tests of several pools. The scope session
    /// and the selection stay.
    pub fn add_synthetic_pool(&mut self, name: &str, inventory: Inventory) -> ProfileId {
        let mut profile = crate::demo::profile();
        profile.id = ProfileId::new();
        profile.name = name.to_owned();
        let id = profile.id;
        self.workspace.preview_profiles.push(profile);
        self.workspace.synthetic.insert(id, Arc::new(inventory));
        id
    }

    // -- Shared helpers of the Rune `impl State` ----------------------------

    /// Whether a change may be submitted in the scope session: not a
    /// synthetic inventory, a writable connection, a current snapshot.
    pub fn can_mutate(&self) -> bool {
        !self.demo
            && self.connection.as_ref().is_some_and(|c| !c.is_read_only())
            && self.snapshot.as_ref().is_some_and(|s| s.is_current())
    }

    /// [`Model::can_mutate`] for the session through which a pool is shown,
    /// which may be another than the scope session.
    pub fn can_mutate_pool(&self, pool: PoolId) -> bool {
        match session::session_for_pool(self, pool) {
            Some(id) if self.workspace.selected == Some(id) => self.can_mutate(),
            Some(id) => {
                !self.workspace.synthetic.contains_key(&id)
                    && self
                        .workspace
                        .active
                        .get(&id)
                        .is_some_and(|c| !c.is_read_only())
                    && self.workspace.snapshot(id).is_some_and(|s| s.is_current())
            }
            None => false,
        }
    }

    /// Whether a pool is a synthetic one, in which nothing can be
    /// submitted: `demo` for the pool of the scope session, a synthetic
    /// session for another. False for a pool no session shows.
    pub fn is_synthetic_pool(&self, pool: PoolId) -> bool {
        match session::session_for_pool(self, pool) {
            Some(id) if self.workspace.selected == Some(id) => self.demo,
            Some(id) => self.workspace.synthetic.contains_key(&id),
            None => false,
        }
    }

    /// The connection through which a pool is reached.
    pub fn connection_for(&self, pool: PoolId) -> Option<Connection> {
        let id = session::session_for_pool(self, pool)?;
        if self.workspace.selected == Some(id) {
            self.connection.clone()
        } else {
            self.workspace.active.get(&id).cloned()
        }
    }

    /// The latest snapshot of the session through which a pool is shown.
    pub fn snapshot_for(&self, pool: PoolId) -> Option<Arc<ConnectionSnapshot>> {
        let id = session::session_for_pool(self, pool)?;
        if self.workspace.selected == Some(id) {
            self.snapshot.clone()
        } else {
            self.workspace.snapshot(id)
        }
    }

    /// The inventory of a pool, whichever session shows it.
    pub fn inventory_for(&self, pool: PoolId) -> Option<Arc<Inventory>> {
        if let Some(inventory) = &self.inventory
            && inventory.pool_id == pool
        {
            return Some(inventory.clone());
        }
        session::inventory_of(self, session::session_for_pool(self, pool)?)
    }

    /// Starts the power availability checks of a VM of the scope session
    /// and keeps their handles in `diagnostics`. It executes nothing on the
    /// host. A failure is in `error`. (The Rune `diagnose_actions`, with the
    /// VM given instead of read from the table.)
    pub fn diagnose_actions(&mut self, vm: VmId) {
        let result = (|| {
            let snapshot = self.snapshot.as_ref().ok_or(Error::Stale)?;
            let observed = snapshot.observe_vm(vm)?;
            let context = ActionDiagnosticContext::capture(
                snapshot.is_current(),
                snapshot.generation(),
                snapshot.inventory().ok_or(Error::Stale)?,
                vm,
            )
            .ok_or(Error::Stale)?;
            let connection = self.connection.as_ref().ok_or(Error::Closed)?;
            let mut handles = Vec::new();
            for action in xenadmin_core::PowerAction::ALL {
                let handle = connection.diagnose_power(observed, action)?;
                if let Some(waker) = &self.waker {
                    handle.notify_on_change(waker.clone());
                }
                handles.push((action, handle));
            }
            self.diagnostics = Some(ActionDiagnostics { context, handles });
            Ok::<(), Error>(())
        })();
        self.error = result.err().map(|error| error.to_string());
    }

    /// Asks the scope session's connection to cancel, recheck or
    /// acknowledge a VM operation. The result is in `error`.
    pub fn control_operation(&mut self, id: OperationId, control: OperationControl) {
        self.error = self
            .connection
            .as_ref()
            .ok_or(Error::Closed)
            .and_then(|connection| connection.control_operation(id, control))
            .err()
            .map(|error| error.to_string());
    }

    /// [`Model::control_operation`] over the connection through which a
    /// pool is reached, for a record of another pool than the scope
    /// session's (`record.pool`). The result is in `error`, which a success
    /// clears. A recheck is a control message on the existing operation: it
    /// never replays a write.
    pub fn control_operation_for(
        &mut self,
        pool: PoolId,
        id: OperationId,
        control: OperationControl,
    ) {
        self.error = self
            .try_control_operation_for(pool, id, control)
            .err()
            .map(|error| error.to_string());
    }

    /// [`Model::control_operation_for`] for a caller that shows the refusal
    /// itself: `error` is left alone. Without a session that shows the pool
    /// the answer is `Error::Closed`; a read-only connection answers
    /// `Error::ReadOnly`, one that is not current `Error::Stale`. `Ok` means
    /// the connection took the request, not that the host answered.
    pub fn try_control_operation_for(
        &self,
        pool: PoolId,
        id: OperationId,
        control: OperationControl,
    ) -> Result<(), Error> {
        self.connection_for(pool)
            .ok_or(Error::Closed)?
            .control_operation(id, control)
    }

    /// [`Model::submit_tracked_command`] for a caller that does not need the
    /// id: the result is in `error`, which is cleared on success.
    pub fn submit_command(&mut self, command: VmCommand) {
        self.error = self
            .submit_tracked_command(command)
            .err()
            .map(|e| e.to_string());
    }

    /// [`Model::submit_tracked_command_for`] with the result in `error`,
    /// which is cleared on success.
    pub fn submit_command_for(&mut self, pool: PoolId, command: VmCommand) {
        self.error = self
            .submit_tracked_command_for(pool, command)
            .err()
            .map(|e| e.to_string());
    }

    /// Submits a VM command in the scope session. A synthetic inventory
    /// refuses with `Error::Stale`; [`MAX_OPERATIONS`] unresolved operations
    /// of the session refuse with `Error::Busy`; without a connection the
    /// answer is `Error::Closed` (in that order: [`admit`]). The handle is
    /// kept in `operations` and wakes the model when it changes; its record
    /// reaches `operation_rows` at the next poll.
    ///
    /// A command whose target may be in another pool goes through
    /// [`Model::submit_tracked_command_for`].
    pub fn submit_tracked_command(&mut self, command: VmCommand) -> Result<OperationId, Error> {
        let (owner, synthetic) = (self.workspace.selected, self.demo);
        let connection = self.connection.clone();
        self.submit_through(owner, synthetic, connection, command)
    }

    /// The one place a VM command is submitted for a target of any pool:
    /// through the session that shows `pool`, with the refusals of
    /// [`Model::submit_tracked_command`] in the same order, made of that
    /// session (a synthetic pool: `Stale`; [`MAX_OPERATIONS`] unresolved
    /// operations of that session: `Busy`; not connected: `Closed`). The
    /// handle belongs to that session: it stays polled, and its record
    /// stays in `operation_rows`, when the scope moves to another pool.
    ///
    /// A pool no session shows is submitted in the scope session, whose
    /// connection refuses an observation of another pool as stale.
    pub fn submit_tracked_command_for(
        &mut self,
        pool: PoolId,
        command: VmCommand,
    ) -> Result<OperationId, Error> {
        let Some(owner) = self.other_session(pool) else {
            return self.submit_tracked_command(command);
        };
        let synthetic = self.workspace.synthetic.contains_key(&owner);
        let connection = self.workspace.active.get(&owner).cloned();
        self.submit_through(Some(owner), synthetic, connection, command)
    }

    /// The refusals of [`Model::submit_tracked_command`] that do not depend
    /// on the command, without submitting anything. Handles of finished
    /// operations are forgotten first, as a submission does.
    pub fn admit_command(&mut self) -> Result<(), Error> {
        let connected = self.connection.is_some();
        self.admit_through(self.workspace.selected, self.demo, connected)
    }

    /// [`Model::admit_command`] for the session that shows a pool: the
    /// refusals of [`Model::submit_tracked_command_for`] without submitting.
    pub fn admit_command_for(&mut self, pool: PoolId) -> Result<(), Error> {
        let Some(owner) = self.other_session(pool) else {
            return self.admit_command();
        };
        let synthetic = self.workspace.synthetic.contains_key(&owner);
        let connected = self.workspace.active.contains_key(&owner);
        self.admit_through(Some(owner), synthetic, connected)
    }

    /// The session that shows a pool, when it is not the scope session.
    fn other_session(&self, pool: PoolId) -> Option<ProfileId> {
        session::session_for_pool(self, pool).filter(|id| self.workspace.selected != Some(*id))
    }

    /// The session a handle belongs to: the one it was submitted through,
    /// else the one that shows its record's pool.
    fn operation_owner(&self, operation: &Operation) -> Option<ProfileId> {
        let record = operation.latest();
        self.operation_sessions
            .get(&record.id)
            .copied()
            .or_else(|| session::session_for_pool(self, record.pool))
    }

    fn admit_through(
        &mut self,
        owner: Option<ProfileId>,
        synthetic: bool,
        connected: bool,
    ) -> Result<(), Error> {
        if !synthetic {
            self.operations.retain(|op| !op.latest().state.terminal());
            self.forget_operation_sessions();
        }
        let unresolved = self
            .operations
            .iter()
            .filter(|operation| self.operation_owner(operation) == owner)
            .count();
        admit(synthetic, unresolved, connected)
    }

    fn submit_through(
        &mut self,
        owner: Option<ProfileId>,
        synthetic: bool,
        connection: Option<Connection>,
        command: VmCommand,
    ) -> Result<OperationId, Error> {
        self.admit_through(owner, synthetic, connection.is_some())?;
        let id = OperationId::new();
        let operation = connection.ok_or(Error::Closed)?.submit(id, command)?;
        if let Some(waker) = &self.waker {
            operation.notify_on_change(waker.clone());
        }
        if let Some(owner) = owner {
            self.operation_sessions.insert(id, owner);
        }
        self.operations.push(operation);
        Ok(id)
    }

    /// Drops the entries of `operation_sessions` whose handle is gone.
    fn forget_operation_sessions(&mut self) {
        if self.operation_sessions.is_empty() {
            return;
        }
        let kept: std::collections::BTreeSet<OperationId> =
            self.operations.iter().map(|op| op.latest().id).collect();
        self.operation_sessions.retain(|id, _| kept.contains(id));
    }

    /// Forgets the handles of the VM operations of one session: its
    /// connection is being replaced or removed, so they will not change
    /// again. Their records stay in `operation_rows`, and what the journal
    /// knows of them comes back through the history of the session's next
    /// connection. (`session::start_connection` and
    /// `session::remove_record` call it.)
    pub fn forget_operations_of(&mut self, profile: ProfileId) {
        let gone: Vec<OperationId> = self
            .operations
            .iter()
            .filter(|operation| self.operation_owner(operation) == Some(profile))
            .map(|operation| operation.latest().id)
            .collect();
        if gone.is_empty() {
            return;
        }
        self.operations
            .retain(|operation| !gone.contains(&operation.latest().id));
        self.forget_operation_sessions();
    }

    /// Keeps the handle of a resource operation a module has just
    /// submitted: it wakes the model when it changes, and its record
    /// reaches `resource_rows` at the next poll. Returns the operation's id.
    pub fn track_resource(&mut self, operation: ResourceOperation) -> OperationId {
        if let Some(waker) = &self.waker {
            operation.notify_on_change(waker.clone());
        }
        let id = operation.latest().id;
        self.resource_operations.push(operation);
        id
    }

    /// Writes the preferences, when this process owns the state directory.
    /// A failure is in `error`.
    pub fn save_preferences(&mut self) {
        self.layout_changed_at = None;
        if self.instance.is_none() {
            return;
        }
        if let Err(error) = self.preferences.save() {
            self.error = Some(error);
        }
    }

    /// Sets the interface size and keeps it.
    pub fn set_interface_scale(&mut self, percent: u16) {
        if percent == self.preferences.options.interface_scale {
            return;
        }
        self.preferences.options.interface_scale = percent;
        self.save_preferences();
    }

    /// The front end reports the width the user gave the navigation pane.
    /// It is kept inside what the pane allows and written once it rests.
    pub fn set_sidebar_width(&mut self, width: f32) {
        if !width.is_finite() {
            return;
        }
        let width = width.clamp(Preferences::SIDEBAR_RANGE.0, Preferences::SIDEBAR_RANGE.1);
        if (self.preferences.sidebar_width - width).abs() > 0.5 {
            self.preferences.sidebar_width = width;
            self.layout_changed();
        }
    }

    /// The front end reports the width the user gave a column of a table,
    /// by the table's key and the column's id. `None`: the column is back
    /// at its declared width and is forgotten. Widths are kept in the
    /// preferences and written once they rest.
    pub fn set_column_width(&mut self, table: &str, column: &str, width: Option<f32>) {
        let id = column_width_id(table, column);
        let changed = match width {
            Some(width) if width.is_finite() && (32.0..=2000.0).contains(&width) => {
                self.preferences.columns.insert(id, width) != Some(width)
            }
            Some(_) => false,
            // A double press on the divider puts a column back to its declared width.
            None => self.preferences.columns.remove(&id).is_some(),
        };
        if changed {
            self.layout_changed();
        }
    }

    /// The width the user gave a column of a table, when they gave one.
    pub fn column_width(&self, table: &str, column: &str) -> Option<f32> {
        self.preferences
            .columns
            .get(&column_width_id(table, column))
            .copied()
    }

    fn layout_changed(&mut self) {
        self.layout_changed_at = Some(self.clock);
        self.request_tick(LAYOUT_SAVE_DELAY);
    }

    /// Whether a width changed and is not written yet.
    pub fn layout_pending(&self) -> bool {
        self.layout_changed_at.is_some()
    }

    // -- What is selected, as the Rune `selected()` family ------------------

    /// The one object the commands act on: the grid selection of the tab
    /// when it is one row, otherwise the tree selection when it is one
    /// object. The counterpart of the Rune `State::selected`.
    pub fn selected(&self) -> Option<&ObjectRef> {
        self.selection.target().single()
    }

    /// The Rune `DisplayRow` of [`Model::selected`]: name, state, location,
    /// vCPUs, memory and description of a VM, a template, a host, a storage
    /// repository or a network.
    pub fn selected_row(&self) -> Option<DisplayRow> {
        self.object_row(self.selected()?)
    }

    /// The VM id of [`Model::selected`] when it is a VM or a template.
    pub fn selected_vm(&self) -> Option<VmId> {
        self.selected()?.vm_id()
    }

    /// The host, storage repository or network the commands act on, as the
    /// core names it. `None` for every other kind (the Rune
    /// `resource_admin::selected_target`).
    pub fn selected_target(&self) -> Option<ResourceTarget> {
        let object = self.selected()?;
        match object.kind {
            ObjectKind::Host | ObjectKind::StorageRepository | ObjectKind::Network => {
                resource_target(object)
            }
            _ => None,
        }
    }

    /// The row of one object, from the inventory of its pool.
    pub fn object_row(&self, object: &ObjectRef) -> Option<DisplayRow> {
        let inventory = self.inventory_for(object.pool?)?;
        let page = match object.kind {
            ObjectKind::Vm | ObjectKind::Template => {
                let vm = inventory.vms.get(&object.vm_id()?)?;
                return Some(crate::rows::vm_row(&inventory, vm));
            }
            ObjectKind::Host => crate::rows::InventoryPage::Hosts,
            ObjectKind::StorageRepository => crate::rows::InventoryPage::Storage,
            ObjectKind::Network => crate::rows::InventoryPage::Networks,
            _ => return None,
        };
        crate::rows::project_rows(&inventory, page, None, "")
            .into_iter()
            .find(|row| row.id == object.id)
    }

    /// The name of an object as the inventory of its pool gives it.
    pub fn name_of(&self, object: &ObjectRef) -> Option<String> {
        if object.kind == ObjectKind::Connection {
            let id = session::profile_id(&object.id)?;
            return self.workspace.profile(id).map(|p| p.name.clone());
        }
        let inventory = self.inventory_for(object.pool?)?;
        let uuid = object.uuid()?;
        match object.kind {
            ObjectKind::Pool => Some(inventory.pool_name.clone()),
            ObjectKind::Host => inventory
                .hosts
                .get(&HostId::from(uuid))
                .map(|host| host.name.clone()),
            ObjectKind::Vm | ObjectKind::Template => inventory
                .vms
                .get(&VmId::from(uuid))
                .map(|vm| vm.name.clone()),
            ObjectKind::StorageRepository => inventory
                .storage
                .iter()
                .find(|storage| storage.uuid == uuid)
                .map(|storage| storage.name.clone()),
            ObjectKind::Network => inventory
                .networks
                .iter()
                .find(|network| network.uuid == uuid)
                .map(|network| network.name.clone()),
            ObjectKind::VirtualDisk => inventory
                .details
                .vdis
                .get(&uuid.into())
                .map(|disk| disk.name.clone()),
            ObjectKind::Snapshot => inventory
                .snapshots
                .get(&uuid.into())
                .map(|snapshot| snapshot.name.clone()),
            ObjectKind::Appliance => inventory
                .details
                .appliances
                .get(&uuid.into())
                .map(|appliance| appliance.name.clone()),
            _ => None,
        }
    }

    /// The object as it is now: itself while it exists; the template a VM
    /// was converted to (and the reverse); the pool a saved connection
    /// became when it connected, and the saved connection of a pool that is
    /// no longer shown. `None` when it is gone. Folders, tags and saved
    /// searches are the areas' and always resolve.
    pub fn resolve(&self, object: &ObjectRef) -> Option<ObjectRef> {
        match object.kind {
            ObjectKind::None
            | ObjectKind::Multi
            | ObjectKind::Folder
            | ObjectKind::Tag
            | ObjectKind::SavedSearch => Some(object.clone()),
            ObjectKind::Connection => {
                let id = session::profile_id(&object.id)?;
                self.workspace.profile(id)?;
                Some(session::session_object(self, id))
            }
            _ => {
                let pool = object.pool?;
                let Some(inventory) = self.inventory_for(pool) else {
                    // The pool is not shown any more. Its node is the saved
                    // connection it was reached through.
                    return (object.kind == ObjectKind::Pool)
                        .then(|| self.connection_node_of(pool))
                        .flatten();
                };
                let uuid = object.uuid()?;
                let found = match object.kind {
                    ObjectKind::Pool => true,
                    ObjectKind::Host => inventory.hosts.contains_key(&HostId::from(uuid)),
                    ObjectKind::Vm | ObjectKind::Template => {
                        let vm = inventory.vms.get(&VmId::from(uuid))?;
                        if vm.is_control_domain {
                            return None;
                        }
                        return Some(if vm.is_template {
                            ObjectRef::template(pool, vm.id)
                        } else {
                            ObjectRef::vm(pool, vm.id)
                        });
                    }
                    ObjectKind::StorageRepository => {
                        inventory.storage.iter().any(|s| s.uuid == uuid)
                    }
                    ObjectKind::Network => inventory.networks.iter().any(|n| n.uuid == uuid),
                    ObjectKind::VirtualDisk => inventory.details.vdis.contains_key(&uuid.into()),
                    ObjectKind::Snapshot => inventory.snapshots.get(&uuid.into()).is_some(),
                    ObjectKind::Appliance => {
                        inventory.details.appliances.contains_key(&uuid.into())
                    }
                    _ => false,
                };
                found.then(|| object.clone())
            }
        }
    }

    /// Whether the object is in the inventory of its pool now.
    pub fn exists(&self, object: &ObjectRef) -> bool {
        self.resolve(object).is_some()
    }

    /// The saved connection a pool was reached through, as a tree node:
    /// the scope session when its profile remembers that pool, else the
    /// first saved connection that does.
    fn connection_node_of(&self, pool: PoolId) -> Option<ObjectRef> {
        let remembers = |id: &ProfileId| {
            self.workspace
                .profile(*id)
                .is_some_and(|p| p.pool == Some(pool))
        };
        let anchored = self
            .selection
            .anchor
            .as_ref()
            .filter(|anchor| anchor.object == ObjectRef::pool(pool))
            .and_then(|anchor| anchor.parent.clone());
        anchored.or_else(|| {
            self.workspace
                .selected
                .filter(remembers)
                .or_else(|| {
                    self.workspace
                        .profiles()
                        .into_iter()
                        .map(|p| p.id)
                        .find(remembers)
                })
                .map(|id| ObjectRef::new(None, ObjectKind::Connection, id))
        })
    }

    /// The object above this one in the Infrastructure tree: the host a VM
    /// is on, else its pool; the pool of a host, a template, a storage
    /// repository, a network or a vApp; the VM of a snapshot; the
    /// repository of a virtual disk; the saved connection of a pool.
    pub fn parent_of(&self, object: &ObjectRef) -> Option<ObjectRef> {
        let pool = object.pool?;
        let inventory = self.inventory_for(pool)?;
        let uuid = object.uuid()?;
        let in_pool = Some(ObjectRef::pool(pool));
        match object.kind {
            ObjectKind::Pool => session::session_for_pool(self, pool)
                .map(|id| ObjectRef::new(None, ObjectKind::Connection, id)),
            ObjectKind::Vm => inventory
                .vms
                .get(&VmId::from(uuid))
                .and_then(|vm| vm.host)
                .map(|host| ObjectRef::host(pool, host))
                .or(in_pool),
            ObjectKind::Snapshot => inventory
                .snapshots
                .get(&uuid.into())
                .and_then(|snapshot| snapshot.vm)
                .map(|vm| ObjectRef::vm(pool, vm))
                .or(in_pool),
            ObjectKind::VirtualDisk => inventory
                .details
                .vdis
                .get(&uuid.into())
                .and_then(|disk| disk.sr)
                .map(|sr| ObjectRef::new(Some(pool), ObjectKind::StorageRepository, sr))
                .or(in_pool),
            ObjectKind::Host
            | ObjectKind::Template
            | ObjectKind::StorageRepository
            | ObjectKind::Network
            | ObjectKind::Appliance => in_pool,
            _ => None,
        }
    }

    // -- Surfaces -----------------------------------------------------------

    /// Every open surface in layering order: the last is in front and is the
    /// one that takes input. The order across areas is that of
    /// [`areas::AREAS`]. The Properties dialog, which the kernel describes
    /// from the pages of the areas, stands with the surfaces of the area of
    /// its current page, in front of them.
    pub fn surfaces(&self) -> Vec<Surface> {
        let mut properties = crate::properties::surface(self);
        let mut surfaces = Vec::new();
        for area in areas::specs() {
            surfaces.extend((area.surfaces)(self));
            if properties
                .as_ref()
                .is_some_and(|(owner, _)| *owner == area.id)
                && let Some((_, surface)) = properties.take()
            {
                surfaces.push(surface);
            }
        }
        surfaces
    }

    /// The surface in front, when one is open.
    pub fn front_surface(&self) -> Option<Surface> {
        self.surfaces().pop()
    }

    /// Whether the experimental notice is shown: the surface in front
    /// belongs to an area outside the supported core. A supported surface in
    /// front of an experimental one suppresses it (the rule of the Rune
    /// `experimental_dialog_open`).
    pub fn experimental_notice(&self) -> bool {
        self.front_surface()
            .is_some_and(|surface| surface.experimental)
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

    /// Moves the tree scope, as a press on a tree row does. The grid
    /// selection and an open context menu are dropped, the tab is the one
    /// last used for an object of this kind, the place left is where Back
    /// returns to, and every area hears about it (`on_scope_change`). When
    /// the new scope belongs to another session, that session becomes the
    /// scope session first (`on_session_change`).
    pub fn select(&mut self, scope: Target) {
        if self.selection.scope == scope {
            return;
        }
        self.watch_tab(|m| {
            let from = m.place();
            if m.nav.view == NavView::Notifications {
                m.nav.view = NavView::Infrastructure;
            }
            m.notice = None;
            m.go(scope, None);
            m.selection.history.visit(from);
        });
    }

    /// Selects nothing and shows the Home tab: the connection list.
    pub fn show_home(&mut self) {
        self.watch_tab(|m| {
            m.set_nav_view(NavView::Infrastructure);
            m.select(Target::none());
            m.set_active_tab(TabId::Home);
        });
    }

    /// Shows the Notifications view at one of its two tabs, as one change:
    /// the areas hear of one tab change, from the tab that was shown to
    /// `tab`.
    pub fn show_notifications(&mut self, tab: TabId) {
        self.watch_tab(|m| {
            m.set_nav_view(NavView::Notifications);
            m.set_active_tab(tab);
        });
    }

    /// Runs a change that may move the active tab, and tells every area
    /// when the tab that is shown afterwards is another one
    /// (`on_tab_change`), once the change is complete. Nested changes
    /// (`show_home` selects and sets a tab) are reported once, by the
    /// outermost.
    fn watch_tab<R>(&mut self, change: impl FnOnce(&mut Self) -> R) -> R {
        if self.tab_watched {
            return change(self);
        }
        let from = self.active_tab();
        self.tab_watched = true;
        let result = change(self);
        self.tab_watched = false;
        let to = self.active_tab();
        if from != to {
            for area in areas::specs() {
                (area.on_tab_change)(self, from, to);
            }
        }
        result
    }

    /// Where Back and Forward would return to from here.
    fn place(&self) -> Place {
        Place {
            scope: self.selection.scope.clone(),
            tab: self.object_tab(),
        }
    }

    /// Moves the scope without recording history.
    fn go(&mut self, scope: Target, tab: Option<TabId>) {
        if let Some(owner) = session::owner(self, &scope)
            && self.workspace.selected != Some(owner)
        {
            session::adopt(self, owner);
            self.preferences.last_profile = Some(owner);
            self.save_preferences();
        }
        self.selection.scope = scope;
        self.selection.grid = Target::none();
        self.selection.menu = None;
        let kind = self.selection.scope.kind();
        // The tab last used for this kind; without one, the tab that is
        // shown stays when the new scope has it.
        self.selection.tab = tab.or_else(|| self.selection.last_tab(kind).or(self.selection.tab));
        self.refresh_anchor();
        for area in areas::specs() {
            (area.on_scope_change)(self);
        }
    }

    /// Steps back to the place the selection was at before. False when
    /// there is none.
    pub fn back(&mut self) -> bool {
        self.step(true)
    }

    /// Steps forward again after [`Model::back`].
    pub fn forward(&mut self) -> bool {
        self.step(false)
    }

    pub fn can_go_back(&self) -> bool {
        self.selection.history.can_go_back()
    }

    pub fn can_go_forward(&self) -> bool {
        self.selection.history.can_go_forward()
    }

    fn step(&mut self, back: bool) -> bool {
        self.watch_tab(|m| m.step_unwatched(back))
    }

    fn step_unwatched(&mut self, back: bool) -> bool {
        let from = self.place();
        let mut history = std::mem::take(&mut self.selection.history);
        let usable = |place: &Place| place.scope.objects.iter().all(|object| self.exists(object));
        let place = if back {
            history.back(from, usable)
        } else {
            history.forward(from, usable)
        };
        self.selection.history = history;
        let Some(place) = place else {
            return false;
        };
        if self.nav.view == NavView::Notifications {
            self.nav.view = NavView::Infrastructure;
        }
        self.notice = None;
        self.go(place.scope, Some(place.tab));
        true
    }

    /// Sets the grid selection of the active tab: the target of its
    /// commands. The scope does not change.
    pub fn select_in_grid(&mut self, grid: Target) {
        self.selection.grid = grid;
        self.selection.menu = None;
    }

    /// Whether a host counts as live for the tab matrix: it is in the
    /// inventory of a session that is not known to be disconnected or
    /// failed, and its metrics do not say it is dead.
    pub fn host_live(&self, object: &ObjectRef) -> bool {
        let (Some(pool), Some(host)) = (object.pool, object.host_id()) else {
            return false;
        };
        let Some(inventory) = self.inventory_for(pool) else {
            return false;
        };
        if !inventory.hosts.contains_key(&host) {
            return false;
        }
        let disconnected = self.snapshot_for(pool).is_some_and(|snapshot| {
            matches!(
                snapshot.state(),
                ConnectionState::Disconnected | ConnectionState::Failed
            )
        });
        let dead = inventory
            .details
            .hosts
            .get(&host)
            .and_then(|details| details.metrics)
            .and_then(|metrics| inventory.details.host_metrics.get(&metrics))
            .and_then(|metrics| metrics.live)
            == Some(false);
        !disconnected && !dead
    }

    /// The tabs of the tab strip, in order: Alerts and Events in the
    /// Notifications view, otherwise the row of the tab matrix for the
    /// scope.
    pub fn tabs(&self) -> &'static [TabId] {
        if self.nav.view == NavView::Notifications {
            return tabs::NOTIFICATION_TABS;
        }
        self.object_tabs()
    }

    fn object_tabs(&self) -> &'static [TabId] {
        let scope = &self.selection.scope;
        let host_live = scope
            .single_of(ObjectKind::Host)
            .is_some_and(|host| self.host_live(host));
        tabs::tabs_for(scope.kind(), host_live)
    }

    /// The tab that is shown: in the Notifications view Alerts or Events;
    /// otherwise the one the user chose when the scope has it, else the
    /// first tab of the scope.
    pub fn active_tab(&self) -> TabId {
        if self.nav.view == NavView::Notifications {
            return self.nav.notification_tab;
        }
        self.object_tab()
    }

    fn object_tab(&self) -> TabId {
        let tabs = self.object_tabs();
        self.selection
            .tab
            .filter(|tab| tabs.contains(tab))
            .unwrap_or(tabs[0])
    }

    /// Chooses a tab. The grid selection belongs to the tab and is cleared;
    /// the tab is remembered for the kind of the scope; the place left is
    /// where Back returns to.
    pub fn set_active_tab(&mut self, tab: TabId) {
        self.watch_tab(|m| m.set_tab_unwatched(tab));
    }

    fn set_tab_unwatched(&mut self, tab: TabId) {
        if self.nav.view == NavView::Notifications {
            if tabs::NOTIFICATION_TABS.contains(&tab) && self.nav.notification_tab != tab {
                self.nav.notification_tab = tab;
                self.selection.grid = Target::none();
                self.selection.menu = None;
            }
            return;
        }
        if self.selection.tab != Some(tab) {
            let from = self.place();
            self.selection.tab = Some(tab);
            self.selection.grid = Target::none();
            self.selection.menu = None;
            if self.place() != from {
                self.selection.history.visit(from);
            }
        }
        let kind = self.selection.scope.kind();
        self.selection.last_tab.insert(kind, tab);
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

    /// Switches the navigation view. The selection stays; the grid
    /// selection and an open context menu are dropped, and every area hears
    /// about it (`on_nav_change`).
    pub fn set_nav_view(&mut self, view: NavView) {
        if self.nav.view != view {
            self.watch_tab(|m| {
                m.nav.view = view;
                m.selection.grid = Target::none();
                m.selection.menu = None;
                for area in areas::specs() {
                    (area.on_nav_change)(m);
                }
            });
        }
    }

    /// Sets the text of the navigation search box.
    pub fn set_nav_filter(&mut self, filter: &str) {
        self.nav.filter = filter.to_owned();
    }

    /// A press on a row of the navigation tree, by row id: an object row
    /// selects its object, the two rows of the Notifications view choose
    /// the tab, a heading does nothing.
    pub fn select_nav_row(&mut self, id: &str) {
        match id {
            "notifications:alerts" => self.set_active_tab(TabId::Alerts),
            "notifications:events" => self.set_active_tab(TabId::Events),
            _ => {
                let object = crate::nav::rows(self)
                    .into_iter()
                    .find(|row| row.id == id)
                    .and_then(|row| row.object);
                if let Some(object) = object {
                    self.select(Target::one(object));
                }
            }
        }
    }

    /// Remembers the name and the parent of the scope object while it is
    /// there.
    fn refresh_anchor(&mut self) {
        self.selection.anchor = self.selection.scope.single().and_then(|object| {
            Some(Anchor {
                object: object.clone(),
                name: self.name_of(object)?,
                parent: self.parent_of(object),
            })
        });
    }

    /// Brings the selection in line with the inventories after they
    /// changed. An object that is still there keeps the selection; one that
    /// changed shape (a VM converted to a template, a saved connection that
    /// became its pool) is followed silently; one that is gone gives the
    /// selection to its parent and leaves a notice for the status bar. Rows
    /// of the grid selection that are gone are dropped. Returns whether the
    /// selection changed. `poll` calls this; so does anything that removes
    /// a session.
    pub fn reconcile_selection(&mut self) -> bool {
        self.watch_tab(Self::reconcile_unwatched)
    }

    fn reconcile_unwatched(&mut self) -> bool {
        let mut changed = false;
        let grid: Vec<ObjectRef> = self
            .selection
            .grid
            .objects
            .iter()
            .filter_map(|object| self.resolve(object))
            .collect();
        if grid != self.selection.grid.objects {
            self.selection.grid = Target::many(grid);
            changed = true;
        }
        let scope = self.selection.scope.clone();
        if let Some(object) = scope.single() {
            match self.resolve(object) {
                Some(now) if now == *object => self.refresh_anchor(),
                Some(now) => {
                    // The same thing in another shape.
                    self.go(Target::one(now), None);
                    changed = true;
                }
                None => {
                    let anchor = self.selection.anchor.clone();
                    let mut parent = anchor.as_ref().and_then(|anchor| anchor.parent.clone());
                    let mut next = None;
                    while let Some(candidate) = parent {
                        if let Some(found) = self.resolve(&candidate) {
                            next = Some(found);
                            break;
                        }
                        parent = self.parent_of(&candidate).or_else(|| {
                            (candidate.kind != ObjectKind::Pool)
                                .then(|| candidate.pool.map(ObjectRef::pool))
                                .flatten()
                        });
                    }
                    let name = anchor
                        .map(|anchor| anchor.name)
                        .unwrap_or_else(|| object.id.clone());
                    let shown = next
                        .as_ref()
                        .and_then(|parent| self.name_of(parent))
                        .map(|parent| format!(" Showing {parent} instead."))
                        .unwrap_or_default();
                    self.go(next.map(Target::one).unwrap_or_default(), None);
                    self.notice = Some(format!(
                        "{} \"{name}\" is no longer in the inventory.{shown}",
                        object.kind.label()
                    ));
                    changed = true;
                }
            }
        } else if scope.is_multi() {
            let kept: Vec<ObjectRef> = scope
                .objects
                .iter()
                .filter_map(|object| self.resolve(object))
                .collect();
            if kept != scope.objects {
                let gone = scope.len() - kept.len();
                self.go(Target::many(kept), None);
                if gone > 0 {
                    self.notice = Some(format!(
                        "{gone} selected {} no longer in the inventory.",
                        if gone == 1 {
                            "object is"
                        } else {
                            "objects are"
                        }
                    ));
                }
                changed = true;
            }
        }
        changed
    }

    // -- The context menu (the Rune `inventory_menu`) -----------------------

    fn menu_scope(&self, object: &ObjectRef) -> MenuScope {
        MenuScope {
            pool: object.pool,
            scope: self.selection.scope.clone(),
            tab: self.active_tab(),
            observation: object.pool.and_then(|pool| {
                self.snapshot_for(pool)
                    .and_then(|snapshot| snapshot.observe_resource(ResourceTarget::Pool(pool)).ok())
            }),
        }
    }

    /// A secondary press, the Menu key or Shift+F10 on a row. The row must
    /// exist. In a grid, a row outside the grid selection replaces it and a
    /// row inside keeps it; in the tree, a row outside the scope is
    /// selected. The scope and the selection are captured: every item is
    /// checked against them again when it is chosen. False when no menu
    /// opens.
    pub fn open_context_menu(&mut self, object: ObjectRef, origin: MenuOrigin) -> bool {
        if !self.exists(&object) {
            return false;
        }
        let target = match origin {
            MenuOrigin::Grid => {
                if !self.selection.grid.contains(&object) {
                    self.selection.grid = Target::one(object.clone());
                }
                self.selection.grid.clone()
            }
            MenuOrigin::Tree => {
                if !self.selection.scope.contains(&object) {
                    self.select(Target::one(object.clone()));
                }
                self.selection.scope.clone()
            }
        };
        self.selection.menu = Some(MenuContext {
            scope: self.menu_scope(&object),
            origin,
            object,
            target,
        });
        true
    }

    /// Whether the open context menu still stands: the captured scope
    /// equals a fresh one (same pool, tree scope, tab and connection
    /// generation), its row is still there, and the selection is what it
    /// was when the menu opened.
    pub fn context_menu_valid(&self) -> bool {
        self.selection
            .menu
            .as_ref()
            .is_some_and(|context| self.menu_valid(context))
    }

    fn menu_valid(&self, context: &MenuContext) -> bool {
        self.menu_scope(&context.object) == context.scope
            && self.exists(&context.object)
            && match context.origin {
                MenuOrigin::Grid => self.selection.grid == context.target,
                MenuOrigin::Tree => self.selection.scope == context.target,
            }
    }

    /// The entries of the open context menu, in groups: items and submenus
    /// of items. A menu that no longer stands shows its items disabled with
    /// the reason.
    pub fn context_menu(&self) -> Vec<Vec<MenuEntry>> {
        let Some(context) = &self.selection.menu else {
            return Vec::new();
        };
        let mut groups = command::context_menu(self, &context.target);
        if !self.menu_valid(context) {
            for entry in groups.iter_mut().flatten() {
                for item in entry.items_mut() {
                    item.disabled = Some(STALE_MENU.into());
                }
            }
        }
        groups
    }

    /// Every item of the open context menu, those of its submenus
    /// included, in order.
    pub fn context_items(&self) -> Vec<MenuItem> {
        command::flatten(self.context_menu())
    }

    /// Chooses an item of the open context menu, at its top level or in a
    /// submenu. The menu closes whatever
    /// happens; the command runs on the captured selection only when the
    /// menu still stands and the command is enabled for it.
    pub fn invoke_context(&mut self, id: CommandId) -> Result<(), Refused> {
        let Some(context) = self.selection.menu.take() else {
            return Err(Refused::Hidden(id));
        };
        if !self.menu_valid(&context) {
            return Err(Refused::Disabled(STALE_MENU.into()));
        }
        self.try_invoke(id, &context.target)
    }

    /// Closes the context menu without choosing.
    pub fn close_context_menu(&mut self) {
        self.selection.menu = None;
    }

    // -- Header, status, title ----------------------------------------------

    /// What the object header shows.
    pub fn header(&self) -> Header {
        let plain = |icon, name: String| Header {
            icon,
            name,
            state: None,
            pool: None,
            host: None,
        };
        if self.nav.view == NavView::Notifications {
            return plain(ObjectKind::None, NavView::Notifications.title().into());
        }
        let scope = &self.selection.scope;
        let Some(object) = scope.single() else {
            if scope.is_empty() {
                return plain(ObjectKind::None, APPLICATION_NAME.into());
            }
            let mut header = plain(
                ObjectKind::Multi,
                format!("{} objects selected", scope.len()),
            );
            header.pool = scope
                .pool()
                .and_then(|pool| self.inventory_for(pool))
                .map(|inventory| inventory.pool_name.clone());
            return header;
        };
        let mut header = plain(
            object.kind,
            self.name_of(object).unwrap_or_else(|| object.id.clone()),
        );
        let inventory = object.pool.and_then(|pool| self.inventory_for(pool));
        let pool_name = inventory
            .as_ref()
            .map(|inventory| inventory.pool_name.clone());
        match object.kind {
            ObjectKind::Connection => {
                header.state = session::profile_id(&object.id).map(|id| {
                    session::sessions(self)
                        .into_iter()
                        .find(|view| view.profile == id)
                        .map(|view| view.status)
                        .unwrap_or("Disconnected")
                        .to_owned()
                });
            }
            ObjectKind::Pool => {
                header.state = object
                    .pool
                    .and_then(|pool| session::session_for_pool(self, pool))
                    .and_then(|id| {
                        session::sessions(self)
                            .into_iter()
                            .find(|view| view.profile == id)
                    })
                    .map(|view| {
                        if view.selected && self.demo {
                            "Demo"
                        } else {
                            view.status
                        }
                        .to_owned()
                    });
            }
            ObjectKind::Host | ObjectKind::Vm | ObjectKind::Template => {
                let row = self.object_row(object);
                header.state = row.as_ref().map(|row| row.state.clone());
                header.pool = pool_name;
                if object.kind != ObjectKind::Host {
                    header.host =
                        object
                            .vm_id()
                            .zip(inventory.as_ref())
                            .and_then(|(vm, inventory)| {
                                let host = inventory.vms.get(&vm)?.host?;
                                inventory.hosts.get(&host).map(|host| host.name.clone())
                            });
                }
            }
            ObjectKind::None | ObjectKind::Multi => {}
            _ => header.pool = pool_name,
        }
        header
    }

    /// The connection state and freshness of the scope session, as the
    /// Rune status line words it.
    pub fn connection_status(&self) -> String {
        let mut status: String = if self.demo {
            "Demo inventory • no host connection".into()
        } else {
            match self.snapshot.as_ref().map(|snapshot| snapshot.state()) {
                Some(ConnectionState::Ready) => {
                    let endpoint = self
                        .workspace
                        .selected
                        .and_then(|id| self.workspace.profile(id))
                        .map(|profile| profile.endpoint.to_string())
                        .unwrap_or_default();
                    format!("Connected directly • {endpoint}")
                }
                Some(ConnectionState::Connecting) => "Connecting directly to XAPI…".into(),
                Some(ConnectionState::Synchronizing) => "Synchronizing inventory…".into(),
                Some(ConnectionState::Reconnecting) => {
                    "Connection lost • inventory is stale • reconnecting…".into()
                }
                Some(ConnectionState::Failed) => "Connection failed • inventory is stale".into(),
                _ => "No host connected".into(),
            }
        };
        if !self.demo && self.connection.is_some() && session::active_insecure(self) {
            status.push_str(" • TLS verification disabled");
        }
        status
    }

    /// Says so when a connected host of the scope session is not of the
    /// product line this client has been run against. It restricts nothing:
    /// the operator decides what to do on such a host.
    pub fn unqualified_hosts_notice(&self) -> Option<String> {
        let inventory = self.inventory.as_ref()?;
        let mut hosts = inventory
            .hosts
            .values()
            .filter(|host| !host.qualified())
            .map(|host| {
                if host.version.is_empty() || host.version == "Unknown" {
                    format!("{} (no version reported)", host.name)
                } else {
                    format!("{} ({})", host.name, host.version)
                }
            })
            .collect::<Vec<_>>();
        if hosts.is_empty() {
            return None;
        }
        hosts.sort();
        let more = hosts.len().saturating_sub(3);
        hosts.truncate(3);
        let mut listed = hosts.join(", ");
        if more > 0 {
            listed.push_str(&format!(" and {more} more"));
        }
        Some(format!(
            "Not qualified on this host version: {listed}. This client has only been run against XCP-ng {}. Looking is safe; treat any change here as untested.",
            xenadmin_core::Host::QUALIFIED_RELEASE
        ))
    }

    /// What the status bar shows.
    pub fn status(&self) -> Status {
        let states = self
            .operation_rows
            .iter()
            .filter(|record| record.batch_parent.is_none())
            .map(|record| &record.state)
            .chain(
                self.resource_rows
                    .iter()
                    .filter(|record| record.batch_parent.is_none())
                    .map(|record| &record.state),
            );
        let (mut operations, mut unknown_outcomes) = (0, 0);
        for state in states {
            if *state == OperationState::OutcomeUnknown {
                unknown_outcomes += 1;
            } else if !state.terminal() {
                operations += 1;
            }
        }
        let target = self.selection.target();
        let selection = match target.single() {
            Some(object) => self.name_of(object).unwrap_or_else(|| object.id.clone()),
            None if target.is_empty() => String::new(),
            None => format!("{} objects selected", target.len()),
        };
        Status {
            connection: self.connection_status(),
            operations,
            unknown_outcomes,
            selection,
            demo: self.demo.then_some(DEMO_BANNER),
            host_notice: self.unqualified_hosts_notice(),
            notice: self.notice.clone(),
            error: self.error.clone().or_else(|| {
                self.snapshot
                    .as_ref()
                    .and_then(|snapshot| snapshot.error().map(str::to_owned))
            }),
        }
    }

    /// A press on the status bar's count of operations in progress: the
    /// History list of every pool, unfiltered.
    pub fn show_operations(&mut self) {
        areas::events::show_states(self, "");
    }

    /// A press on the status bar's count of unknown outcomes: the History
    /// list of every pool, filtered to the records whose outcome is unknown.
    pub fn show_unknown_outcomes(&mut self) {
        areas::events::show_states(self, UNKNOWN_OUTCOME_STATE);
    }

    /// The window title: the name of the selected object, else the pool of
    /// the scope session, else the application's name.
    pub fn window_title(&self) -> String {
        if self.nav.view != NavView::Notifications
            && let Some(object) = self.selection.scope.single()
            && let Some(name) = self.name_of(object)
        {
            return name;
        }
        self.inventory
            .as_ref()
            .map(|inventory| inventory.pool_name.clone())
            .unwrap_or_else(|| APPLICATION_NAME.into())
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

    /// Asks the front end to toggle the fullscreen state of a window.
    pub fn toggle_fullscreen(&mut self, window: crate::intent::WindowRef) {
        self.send(Intent::ToggleFullscreen(window));
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

    /// Reads the latest state of every core handle. The front end calls it
    /// on every wake and timed tick with the monotonic clock in seconds. It
    /// never waits. Returns whether anything shown may have changed.
    ///
    /// In order (the Rune `poll`, without the widget work):
    ///
    /// 1. widths that have rested for [`LAYOUT_SAVE_DELAY`] are written;
    /// 2. [`session::refresh`]: keyring results and the snapshot of every
    ///    session;
    /// 3. the scope session's new snapshot, if there is one, becomes
    ///    `snapshot` and `inventory` together; `diagnostics` that are no
    ///    longer current are dropped; the selection is reconciled with the
    ///    inventories;
    /// 4. `resource_rows` is merged from the scope session's history and
    ///    the handles in `resource_operations`;
    /// 5. every area's `poll`, in the order of [`areas::AREAS`];
    /// 6. the developer inspection socket gets the live connections;
    /// 7. new results of `diagnostics` are detected;
    /// 8. `operation_rows` is merged from the history of every session and
    ///    the handles in `operations`.
    pub fn poll(&mut self, clock: f64) -> bool {
        self.clock = clock;
        if self
            .layout_changed_at
            .is_some_and(|at| clock - at >= LAYOUT_SAVE_DELAY)
        {
            self.save_preferences();
        }
        let mut changed = session::refresh(self);
        changed |= self.poll_scope_session();
        changed |= self.reconcile_selection();
        changed |= crate::properties::poll(self);
        changed |= self.poll_resource_rows();
        for area in areas::specs() {
            changed |= (area.poll)(self, clock);
        }
        if let Some(inspection) = &self.dev_inspection {
            inspection.set_connections(session::inspection_connections(self));
        }
        let diagnostic_results = self
            .diagnostics
            .as_ref()
            .map(|d| {
                d.handles
                    .iter()
                    .map(|(_, h)| format!("{:?}", h.latest()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if diagnostic_results != self.diagnostic_results {
            self.diagnostic_results = diagnostic_results;
            changed = true;
        }
        changed |= self.poll_operation_rows();
        changed
    }

    /// Adopts a new snapshot of the scope session: `snapshot` and
    /// `inventory` change together.
    ///
    /// The snapshot is the one `session::refresh` stored for the session in
    /// `workspace.seen` earlier in the same poll, so the Home tab and the
    /// tree (which read `seen`) and everything that reads `snapshot` show
    /// one state. A second `Connection::latest` here could return a newer
    /// one. Only a connection the workspace does not hold (a test that sets
    /// `connection` itself) is asked directly.
    pub(crate) fn poll_scope_session(&mut self) -> bool {
        let Some(connection) = &self.connection else {
            return false;
        };
        let stored = self
            .workspace
            .selected
            .filter(|id| self.workspace.active.contains_key(id))
            .and_then(|id| self.workspace.seen.get(&id).cloned());
        let snapshot = stored.unwrap_or_else(|| connection.latest());
        if self
            .snapshot
            .as_ref()
            .is_some_and(|last| Arc::ptr_eq(last, &snapshot))
        {
            return false;
        }
        self.inventory = snapshot.inventory().cloned();
        self.snapshot = Some(snapshot);
        if self
            .diagnostics
            .as_ref()
            .is_some_and(|diagnostics| !diagnostics.is_current(self.snapshot.as_deref()))
        {
            self.diagnostics = None;
            self.diagnostic_results.clear();
        }
        true
    }

    /// Merges the VM operation history of every session and the handles in
    /// `operations` into `operation_rows` ([`merge_operation_rows`]): the
    /// scope session's connection and every other connection the workspace
    /// holds, so a record of a pool the tree does not show keeps changing
    /// (the Events tab filters by pool). A handle whose session is gone
    /// from the workspace is forgotten first; its record stays.
    fn poll_operation_rows(&mut self) -> bool {
        let scope = self.workspace.selected;
        let orphaned: Vec<OperationId> = self
            .operation_sessions
            .iter()
            .filter(|(_, profile)| {
                Some(**profile) != scope && !self.workspace.active.contains_key(*profile)
            })
            .map(|(id, _)| *id)
            .collect();
        if !orphaned.is_empty() {
            self.operations
                .retain(|operation| !orphaned.contains(&operation.latest().id));
            self.forget_operation_sessions();
        }
        let mut incoming: Vec<OperationRecord> = Vec::new();
        let others = self
            .workspace
            .active
            .iter()
            .filter(|(id, _)| Some(**id) != scope)
            .map(|(_, connection)| connection);
        for connection in self.connection.iter().chain(others) {
            incoming.extend(connection.operation_history().iter().cloned());
        }
        for operation in &self.operations {
            incoming.push(operation.latest().as_ref().clone());
        }
        let rows = merge_operation_rows(&self.operation_rows, incoming);
        if rows == self.operation_rows {
            false
        } else {
            self.operation_rows = rows;
            true
        }
    }

    /// Merges the scope session's resource history and the handles in
    /// `resource_operations` into `resource_rows` (the Rune
    /// `resource_admin::poll`): keyed by operation id, a record replaces
    /// the stored one only when it supersedes it; every unfinished record
    /// and the newest 100 finished ones are kept, oldest first. Rows of
    /// earlier sessions stay.
    fn poll_resource_rows(&mut self) -> bool {
        let mut incoming: Vec<ResourceOperationRecord> = self
            .connection
            .as_ref()
            .map(|connection| connection.resource_history().iter().cloned().collect())
            .unwrap_or_default();
        for operation in &self.resource_operations {
            incoming.push(operation.latest().as_ref().clone());
        }
        let rows = merge_resource_rows(&self.resource_rows, incoming);
        if rows == self.resource_rows {
            false
        } else {
            self.resource_rows = rows;
            true
        }
    }
}

/// Whether a VM command may be submitted: the order of the refusals of
/// [`Model::submit_tracked_command`]. A synthetic inventory is stale
/// whatever else holds; then the cap on unresolved operations; then the
/// missing connection.
pub fn admit(demo: bool, unresolved: usize, connected: bool) -> Result<(), Error> {
    if demo {
        return Err(Error::Stale);
    }
    if unresolved >= MAX_OPERATIONS {
        return Err(Error::Busy);
    }
    if !connected {
        return Err(Error::Closed);
    }
    Ok(())
}

/// The Rune `dev_inspection::start`.
fn start_dev_inspection(owns_instance: bool) -> (Option<DevInspection>, Option<String>) {
    if !owns_instance || std::env::var("XENADMIN_DEV_INSPECT").as_deref() != Ok("1") {
        return (None, None);
    }
    let result = crate::prefs::directory()
        .ok_or_else(|| "No application state directory is available".to_owned())
        .and_then(|directory| DevInspection::start(directory).map_err(|error| error.to_string()));
    match result {
        Ok(inspection) => {
            eprintln!(
                "Read-only developer inspection: {}",
                inspection.socket_path().display()
            );
            (Some(inspection), None)
        }
        Err(error) => (
            None,
            Some(format!("Cannot start developer inspection: {error}")),
        ),
    }
}

/// The object as the core's resource operations name it.
pub fn resource_target(object: &ObjectRef) -> Option<ResourceTarget> {
    let uuid = object.uuid()?;
    Some(match object.kind {
        ObjectKind::Pool => ResourceTarget::Pool(uuid.into()),
        ObjectKind::Host => ResourceTarget::Host(uuid.into()),
        ObjectKind::Vm | ObjectKind::Template => ResourceTarget::Vm(uuid.into()),
        ObjectKind::StorageRepository => ResourceTarget::Storage(uuid.into()),
        ObjectKind::Network => ResourceTarget::Network(uuid.into()),
        ObjectKind::VirtualDisk => ResourceTarget::Disk(uuid.into()),
        ObjectKind::Appliance => ResourceTarget::Appliance(uuid.into()),
        _ => return None,
    })
}

/// Merges incoming VM operation records into the stored history. Records
/// are keyed by pool and id; an incoming record replaces the stored one only
/// when it supersedes it; every unfinished record and the newest 100
/// finished ones are kept; the result is oldest first. Because the stored
/// rows are an input, rows of a pool that is no longer the scope session
/// survive until the cap pushes them out.
pub fn merge_operation_rows(
    previous: &[OperationRecord],
    incoming: impl IntoIterator<Item = OperationRecord>,
) -> Vec<OperationRecord> {
    let mut rows: std::collections::BTreeMap<_, _> = previous
        .iter()
        .map(|record| ((record.pool, record.id), record.clone()))
        .collect();
    for record in incoming {
        let key = (record.pool, record.id);
        if rows.get(&key).is_none_or(|old| record.supersedes(old)) {
            rows.insert(key, record);
        }
    }
    let mut rows: Vec<_> = rows.into_values().collect();
    rows.sort_by_key(|record| std::cmp::Reverse((record.created_at_ms, record.id)));
    let mut terminal = 0;
    rows.retain(|record| {
        if !record.state.terminal() {
            return true;
        }
        terminal += 1;
        terminal <= 100
    });
    rows.reverse();
    rows
}

/// The merge of the resource operation history (the body of the Rune
/// `resource_admin::poll`). Unlike the VM history it is keyed by operation
/// id alone.
pub fn merge_resource_rows(
    previous: &[ResourceOperationRecord],
    incoming: impl IntoIterator<Item = ResourceOperationRecord>,
) -> Vec<ResourceOperationRecord> {
    let mut rows: std::collections::BTreeMap<_, _> = previous
        .iter()
        .map(|record| (record.id, record.clone()))
        .collect();
    for record in incoming {
        if rows
            .get(&record.id)
            .is_none_or(|old| record.supersedes(old))
        {
            rows.insert(record.id, record);
        }
    }
    let mut rows: Vec<_> = rows.into_values().collect();
    rows.sort_by_key(|record| record.created_at_ms);
    let mut terminal_remaining = 100;
    rows.reverse();
    rows.retain(|record| {
        if !record.state.terminal() {
            return true;
        }
        if terminal_remaining == 0 {
            return false;
        }
        terminal_remaining -= 1;
        true
    });
    rows.reverse();
    rows
}

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "per_pool_tests.rs"]
mod per_pool_tests;

#[cfg(test)]
#[path = "context_menu_tests.rs"]
mod context_menu_tests;
