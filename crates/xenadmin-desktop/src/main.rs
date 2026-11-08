mod activity;
mod activity_export;
mod appliance_view;
mod console_view;
mod demo_details;
mod dev_inspection;
mod host_admin;
mod metrics_view;
mod network_admin;
mod organization_view;
mod pool_security_view;
mod pool_view;
mod preferences;
mod profiles_view;
mod resource_admin;
mod resource_details;
mod search_view;
mod snapshot_policy_view;
mod storage_view;
mod transfers;
mod vm_devices;
mod vm_lifecycle;
mod vm_migration;
mod vm_placement;
mod vm_properties;
mod vm_snapshots;

use preferences::Preferences;
use rune::{AppSpec, Cx, El, Icon, TextEdit, Tx, Waker, col, el, icon, row, text};
use rune_kit::{
    Button, ButtonVariant, Cell, Checkbox, Kit, PaneState, SortDir, SplitView, Table, TableColumn,
    TableState, TextInput, Toolbar, Tree, TreeItem, TreeModel,
};
use std::{borrow::Cow, collections::BTreeMap, sync::Arc};
use xenadmin_core::{
    Backend, Connection, ConnectionOptions, ConnectionSnapshot, ConnectionState, Credentials,
    Endpoint, Host, HostId, Inventory, PoolId, PowerState, Vm, VmId,
};

fn accent() -> rune::Color {
    rune::oklch(0.73, 0.15, 55.0)
}
static SERVER: Icon = Icon {
    name: "host",
    view: 16.0,
    body: r#"<rect x="2" y="2" width="12" height="5" rx="1" fill="none" stroke="currentColor"/><rect x="2" y="9" width="12" height="5" rx="1" fill="none" stroke="currentColor"/><path d="M4 4.5h1m-1 7h1" stroke="currentColor"/>"#,
};
static GUEST: Icon = Icon {
    name: "guest",
    view: 16.0,
    body: r#"<rect x="2" y="2" width="12" height="9" rx="1" fill="none" stroke="currentColor"/><path d="M8 11v3m-3 0h6" stroke="currentColor"/>"#,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Guests,
    Templates,
    Hosts,
    Storage,
    Networks,
}
impl Page {
    fn title(self) -> &'static str {
        match self {
            Self::Guests => "Virtual machines",
            Self::Templates => "Templates",
            Self::Hosts => "Hosts",
            Self::Storage => "Storage",
            Self::Networks => "Networks",
        }
    }
}
struct Node {
    label: String,
    host: Option<HostId>,
    vm: Option<VmId>,
}
#[derive(Clone)]
struct DisplayRow {
    id: String,
    name: String,
    state: String,
    location: String,
    cpu: String,
    memory: String,
    description: String,
    vm: Option<VmId>,
}
struct ActionDiagnostics {
    context: ActionDiagnosticContext,
    handles: Vec<(xenadmin_core::PowerAction, xenadmin_core::ActionDiagnostic)>,
}
#[derive(Debug, PartialEq, Eq)]
struct ActionDiagnosticContext {
    pool: PoolId,
    generation: u64,
    vm: VmId,
    power_state: PowerState,
    host: Option<HostId>,
    is_guest: bool,
    allowed_actions: Vec<xenadmin_core::PowerAction>,
    current_operations: Vec<xenadmin_core::RunningOperation>,
    availability: Option<Vec<xenadmin_core::ActionAvailability>>,
}
impl ActionDiagnosticContext {
    fn capture(current: bool, generation: u64, inventory: &Inventory, vm: VmId) -> Option<Self> {
        if !current {
            return None;
        }
        let vm = inventory.vms.get(&vm)?;
        Some(Self {
            pool: inventory.pool_id,
            generation,
            vm: vm.id,
            power_state: vm.power_state.clone(),
            host: vm.host,
            is_guest: !vm.is_template && !vm.is_control_domain,
            allowed_actions: vm.allowed_actions.clone(),
            current_operations: inventory
                .details
                .vms
                .get(&vm.id)
                .map(|details| details.current_operations.clone())
                .unwrap_or_default(),
            availability: inventory.details.vms.get(&vm.id).map(|details| {
                xenadmin_core::PowerAction::ALL
                    .into_iter()
                    .map(|action| details.power_availability(action, true))
                    .collect()
            }),
        })
    }
}
impl ActionDiagnostics {
    fn is_current(&self, snapshot: Option<&ConnectionSnapshot>) -> bool {
        snapshot
            .and_then(|snapshot| {
                ActionDiagnosticContext::capture(
                    snapshot.is_current(),
                    snapshot.generation(),
                    snapshot.inventory()?,
                    self.context.vm,
                )
            })
            .as_ref()
            == Some(&self.context)
    }
}
struct State {
    searches: Option<search_view::Workspace>,
    metrics: Option<metrics_view::MetricsUi>,
    activity: activity::Activity,
    resource_editor: Option<resource_admin::Editor>,
    pool_editor: Option<pool_view::Editor>,
    pool_security: Option<pool_security_view::Editor>,
    network_workspace: Option<network_admin::Workspace>,
    storage_editor: Option<storage_view::Editor>,
    host_workspace: Option<host_admin::Workspace>,
    snapshot_policy_workspace: Option<snapshot_policy_view::Workspace>,
    resource_operations: Vec<xenadmin_core::ResourceOperation>,
    resource_rows: Vec<xenadmin_core::ResourceOperationRecord>,
    dev_inspection: Option<xenadmin_core::DevInspection>,
    placement: Option<vm_placement::Placement>,
    devices: Option<vm_devices::DeviceView>,
    vm_lifecycle: Option<vm_lifecycle::Panel>,
    vm_migration: Option<Box<vm_migration::Editor>>,
    transfers: transfers::Panel,
    workspace: profiles_view::Workspace,
    diagnostic_results: Vec<String>,
    diagnostics: Option<ActionDiagnostics>,
    properties: Option<vm_properties::Editor>,
    organization: Option<organization_view::Workspace>,
    appliances: Option<appliance_view::Workspace>,
    snapshot_view: Option<VmId>,
    snapshot_rows: Vec<xenadmin_core::Snapshot>,
    snapshot_table: TableState,
    snapshot_dialog: Option<vm_snapshots::Dialog>,
    snapshot_name: TextEdit,
    snapshot_memory: bool,
    snapshot_description: TextEdit,
    window_width: f32,
    window_height: f32,
    show_details: bool,
    operations: Vec<xenadmin_core::Operation>,
    operation_rows: Vec<xenadmin_core::OperationRecord>,
    unknown_review: Option<(xenadmin_core::OperationId, String)>,
    force_action: Option<(
        xenadmin_core::ObservedVm,
        xenadmin_core::PowerAction,
        String,
    )>,
    console_ui: console_view::ConsoleUi,
    console: Option<xenadmin_core::Console>,
    console_snapshot: Option<Arc<xenadmin_core::ConsoleSnapshot>>,
    console_pixels: rune::Pixels,
    console_bounds: [f32; 2],
    console_buttons: u8,
    console_keys: Vec<(rune::PhysicalKeyCode, u32)>,
    backend: Result<Backend, String>,
    _instance: Option<preferences::InstanceGuard>,
    connection: Option<Connection>,
    snapshot: Option<Arc<ConnectionSnapshot>>,
    inventory: Option<Arc<Inventory>>,
    waker: Option<Waker>,
    demo: bool,
    page: Page,
    rows: Vec<DisplayRow>,
    tree: TreeModel<Node>,
    table: TableState,
    sidebar: PaneState,
    inspector: PaneState,
    host_filter: Option<HostId>,
    search: TextEdit,
    endpoint: TextEdit,
    username: TextEdit,
    password: TextEdit,
    fingerprint: TextEdit,
    host_certificates: TextEdit,
    accept_invalid_certificates: bool,
    active_insecure: bool,
    show_certificate_details: bool,
    show_connect: bool,
    error: Option<String>,
    preferences: Preferences,
}
fn secure_field() -> TextEdit {
    let mut edit = TextEdit::new("");
    edit.set_secure(true);
    edit
}
fn init() -> State {
    let live_state = !cfg!(test);
    let preferences = if live_state {
        Preferences::load()
    } else {
        Preferences::default()
    };
    let (instance, backend) = if live_state {
        match preferences::InstanceGuard::acquire() {
            Ok(guard) => (Some(guard), Backend::start().map_err(|e| e.to_string())),
            Err(error) => (None, Err(error)),
        }
    } else {
        (None, Err("UI fixtures cannot connect to hosts".into()))
    };
    let mut table = TableState::new().key("inventory-table");
    for (name, width) in &preferences.columns {
        table.set_column_width(name, *width);
    }
    let (dev_inspection, inspection_error) = dev_inspection::start(instance.is_some());
    let startup_error = live_state
        .then(|| backend.as_ref().err().cloned())
        .flatten()
        .or(inspection_error);
    let mut s = State {
        searches: None,
        metrics: None,
        activity: activity::Activity::default(),
        resource_editor: None,
        pool_editor: None,
        pool_security: None,
        network_workspace: None,
        storage_editor: None,
        host_workspace: None,
        snapshot_policy_workspace: None,
        resource_operations: Vec::new(),
        resource_rows: Vec::new(),
        dev_inspection,
        placement: None,
        devices: None,
        vm_lifecycle: None,
        vm_migration: None,
        transfers: Default::default(),
        workspace: profiles_view::Workspace::load(preferences.last_profile, instance.is_some()),
        diagnostic_results: Vec::new(),
        diagnostics: None,
        properties: None,
        organization: None,
        appliances: None,
        snapshot_view: None,
        snapshot_rows: Vec::new(),
        snapshot_table: TableState::new().key("snapshots-table"),
        snapshot_dialog: None,
        snapshot_name: TextEdit::new(""),
        snapshot_memory: false,
        snapshot_description: TextEdit::new(""),
        window_width: 1440.0,
        window_height: 900.0,
        show_details: false,
        operations: Vec::new(),
        operation_rows: Vec::new(),
        unknown_review: None,
        force_action: None,
        console_ui: Default::default(),
        console: None,
        console_snapshot: None,
        console_pixels: rune::Pixels::new(),
        console_bounds: [0.0; 2],
        console_buttons: 0,
        console_keys: Vec::new(),
        backend,
        _instance: instance,
        connection: None,
        snapshot: None,
        inventory: None,
        waker: None,
        demo: false,
        page: Page::Guests,
        rows: Vec::new(),
        tree: TreeModel::new().key("inventory-tree"),
        table,
        sidebar: PaneState::new(preferences.sidebar_width).range(180.0, 420.0),
        inspector: PaneState::new(300.0).range(230.0, 480.0),
        host_filter: None,
        search: TextEdit::new(""),
        endpoint: TextEdit::new(&preferences.endpoint),
        username: TextEdit::new(&preferences.username),
        password: secure_field(),
        fingerprint: TextEdit::new(&preferences.fingerprint),
        host_certificates: TextEdit::new(&preferences.host_certificates),
        accept_invalid_certificates: preferences.accept_invalid_certificates,
        active_insecure: false,
        show_certificate_details: false,
        show_connect: false,
        error: startup_error,
        preferences,
    };
    profiles_view::apply_initial(&mut s);
    if rune::extra_args().iter().any(|a| a == "--demo") {
        demo(&mut s);
    }
    s
}
impl State {
    fn can_mutate(&self) -> bool {
        !self.demo
            && self.connection.as_ref().is_some_and(|c| !c.is_read_only())
            && self.snapshot.as_ref().is_some_and(|s| s.is_current())
    }
    fn diagnose_actions(&mut self) {
        let result = (|| {
            let vm = self
                .selected()
                .and_then(|r| r.vm)
                .ok_or(xenadmin_core::Error::Stale)?;
            let snapshot = self.snapshot.as_ref().ok_or(xenadmin_core::Error::Stale)?;
            let observed = snapshot.observe_vm(vm)?;
            let context = ActionDiagnosticContext::capture(
                snapshot.is_current(),
                snapshot.generation(),
                snapshot.inventory().ok_or(xenadmin_core::Error::Stale)?,
                vm,
            )
            .ok_or(xenadmin_core::Error::Stale)?;
            let connection = self
                .connection
                .as_ref()
                .ok_or(xenadmin_core::Error::Closed)?;
            let mut handles = Vec::new();
            for action in xenadmin_core::PowerAction::ALL {
                let handle = connection.diagnose_power(observed, action)?;
                if let Some(waker) = &self.waker {
                    handle.notify_on_change(waker.clone());
                }
                handles.push((action, handle));
            }
            self.diagnostics = Some(ActionDiagnostics { context, handles });
            Ok::<(), xenadmin_core::Error>(())
        })();
        self.error = result.err().map(|error| error.to_string());
    }
    fn control_operation(
        &mut self,
        id: xenadmin_core::OperationId,
        control: xenadmin_core::OperationControl,
    ) {
        self.error = self
            .connection
            .as_ref()
            .ok_or(xenadmin_core::Error::Closed)
            .and_then(|connection| connection.control_operation(id, control))
            .err()
            .map(|error| error.to_string());
    }
    fn request_power(&mut self, action: xenadmin_core::PowerAction) {
        if let Some(vm) = self.selected().and_then(|row| row.vm) {
            let Some(observed) = self
                .snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.observe_vm(vm).ok())
            else {
                self.error = Some(xenadmin_core::Error::Stale.to_string());
                return;
            };
            if action.is_force() {
                let name = self
                    .selected()
                    .map(|row| row.name.clone())
                    .unwrap_or_default();
                self.force_action = Some((observed, action, name));
            } else {
                self.submit_power(observed, action);
            }
        }
    }
    fn submit_power(&mut self, vm: xenadmin_core::ObservedVm, action: xenadmin_core::PowerAction) {
        self.submit_command(xenadmin_core::VmCommand::Power { vm, action });
        self.force_action = None;
    }
    fn submit_command(&mut self, command: xenadmin_core::VmCommand) {
        let result = (|| {
            if self.demo {
                return Err(xenadmin_core::Error::Stale);
            }
            self.operations
                .retain(|operation| !operation.latest().state.terminal());
            if self.operations.len() >= 64 {
                return Err(xenadmin_core::Error::Busy);
            }
            let operation = self
                .connection
                .as_ref()
                .ok_or(xenadmin_core::Error::Closed)?
                .submit(xenadmin_core::OperationId::new(), command)?;
            if let Some(waker) = &self.waker {
                operation.notify_on_change(waker.clone());
            }
            self.operations.push(operation);
            Ok::<(), xenadmin_core::Error>(())
        })();
        self.error = result.err().map(|error| error.to_string());
    }
    fn open_console(&mut self) {
        self.metrics = None;
        self.snapshot_view = None;
        let result = (|| {
            let vm = self
                .selected()
                .and_then(|r| r.vm)
                .ok_or(xenadmin_core::Error::NoConsole)?;
            let observed = self
                .snapshot
                .as_ref()
                .ok_or(xenadmin_core::Error::Stale)?
                .observe_vm(vm)?;
            let console = self
                .connection
                .as_ref()
                .ok_or(xenadmin_core::Error::Closed)?
                .open_console(observed)?;
            if let Some(waker) = &self.waker {
                console.notify_on_change(waker.clone());
            }
            let inventory = self.inventory.as_ref().ok_or(xenadmin_core::Error::Stale)?;
            let name = inventory
                .vms
                .get(&vm)
                .ok_or(xenadmin_core::Error::Stale)?
                .name
                .clone();
            console_view::opened(self, inventory.pool_id, vm, name);
            self.devices = None;
            self.console = Some(console);
            self.console_snapshot = None;
            Ok::<(), xenadmin_core::Error>(())
        })();
        self.error = result.err().map(|e| e.to_string());
    }
    fn reconnect_console(&mut self) {
        let result = (|| {
            let target = self
                .console_ui
                .target()
                .ok_or(xenadmin_core::Error::NoConsole)?;
            let snapshot = self.snapshot.as_ref().ok_or(xenadmin_core::Error::Stale)?;
            if snapshot
                .inventory()
                .is_none_or(|i| i.pool_id != target.pool)
            {
                return Err(xenadmin_core::Error::Stale);
            }
            let observed = snapshot.observe_vm(target.vm)?;
            let console = self
                .connection
                .as_ref()
                .ok_or(xenadmin_core::Error::Closed)?
                .open_console(observed)?;
            if let Some(waker) = &self.waker {
                console.notify_on_change(waker.clone());
            }
            console_view::reconnecting(self);
            self.console = Some(console);
            self.console_snapshot = None;
            Ok::<(), xenadmin_core::Error>(())
        })();
        self.error = result.err().map(|e| e.to_string());
    }
    fn connect(&mut self) {
        profiles_view::connect(self);
    }
    fn save_preferences(&mut self) {
        if self._instance.is_none() {
            return;
        }
        self.preferences.sidebar_width = self.sidebar.width();
        for name in ["name", "state", "location", "cpu", "memory"] {
            if let Some(width) = self.table.column_width(name) {
                self.preferences.columns.insert(name.into(), width);
            }
        }
        if let Err(error) = self.preferences.save() {
            self.error = Some(error);
        }
    }
    fn rebuild(&mut self) {
        vm_snapshots::refresh(self);
        let selected = self.table.cursor().map(str::to_owned);
        let previous = &self.tree;
        let mut tree = TreeModel::new().key("inventory-tree");
        if let Some(inventory) = &self.inventory {
            let pool_key = inventory.pool_id.to_string();
            tree.insert(
                None,
                &pool_key,
                Node {
                    label: inventory.pool_name.clone(),
                    host: None,
                    vm: None,
                },
            );
            for host in inventory.hosts.values() {
                let host_key = host.id.to_string();
                tree.insert(
                    Some(&pool_key),
                    &host_key,
                    Node {
                        label: host.name.clone(),
                        host: Some(host.id),
                        vm: None,
                    },
                );
                for vm in inventory.guests().filter(|v| v.host == Some(host.id)) {
                    tree.insert(
                        Some(&host_key),
                        vm.id.to_string(),
                        Node {
                            label: vm.name.clone(),
                            host: Some(host.id),
                            vm: Some(vm.id),
                        },
                    );
                }
                if previous.is_empty() || previous.is_expanded(&host_key) {
                    tree.set_expanded(&host_key, true);
                }
            }
            for vm in inventory.guests().filter(|v| v.host.is_none()) {
                tree.insert(
                    Some(&pool_key),
                    vm.id.to_string(),
                    Node {
                        label: vm.name.clone(),
                        host: None,
                        vm: Some(vm.id),
                    },
                );
            }
            tree.set_expanded(&pool_key, true);
        }
        self.tree = tree;
        self.filter_rows();
        if let Some(id) = selected.filter(|id| self.rows.iter().any(|row| &row.id == id)) {
            self.table.select(id);
        } else if let Some(row) = self.rows.first() {
            self.table.select(row.id.clone());
        }
    }
    fn filter_rows(&mut self) {
        self.rows.clear();
        let Some(inventory) = &self.inventory else {
            return;
        };
        let query = self.search.text.trim().to_lowercase();
        let host_name = |id: Option<HostId>| {
            id.and_then(|id| inventory.hosts.get(&id))
                .map(|h| h.name.clone())
                .unwrap_or_else(|| "—".into())
        };
        match self.page {
            Page::Guests | Page::Templates => {
                for vm in inventory.vms.values().filter(|v| {
                    !v.is_control_domain
                        && v.is_template == (self.page == Page::Templates)
                        && self.host_filter.is_none_or(|id| v.host == Some(id))
                }) {
                    self.rows.push(DisplayRow {
                        id: vm.id.to_string(),
                        name: vm.name.clone(),
                        state: vm.power_state.to_string(),
                        location: host_name(vm.host),
                        cpu: vm.vcpus.to_string(),
                        memory: bytes(vm.memory_bytes),
                        description: vm.description.clone(),
                        vm: Some(vm.id),
                    });
                }
            }
            Page::Hosts => {
                for host in inventory.hosts.values() {
                    self.rows.push(DisplayRow {
                        id: host.id.to_string(),
                        name: host.name.clone(),
                        state: if host.enabled { "Enabled" } else { "Disabled" }.into(),
                        location: host.address.clone(),
                        cpu: "—".into(),
                        memory: "—".into(),
                        description: format!("XCP-ng {}", host.version),
                        vm: None,
                    });
                }
            }
            Page::Storage => {
                for sr in &inventory.storage {
                    self.rows.push(DisplayRow {
                        id: sr.uuid.to_string(),
                        name: sr.name.clone(),
                        state: sr.kind.clone(),
                        location: sr
                            .used_bytes
                            .map(|value| format!("{} used", storage_bytes(Some(value))))
                            .unwrap_or_else(|| "Usage unknown".into()),
                        cpu: "—".into(),
                        memory: storage_bytes(sr.capacity_bytes),
                        description: "Storage repository".into(),
                        vm: None,
                    });
                }
            }
            Page::Networks => {
                for network in &inventory.networks {
                    self.rows.push(DisplayRow {
                        id: network.uuid.to_string(),
                        name: network.name.clone(),
                        state: "Network".into(),
                        location: network.bridge.clone(),
                        cpu: "—".into(),
                        memory: "—".into(),
                        description: "Virtual network".into(),
                        vm: None,
                    });
                }
            }
        }
        self.rows.retain(|row| {
            query.is_empty()
                || row.name.to_lowercase().contains(&query)
                || row.id.contains(&query)
                || row.location.to_lowercase().contains(&query)
        });
        let sort = self
            .table
            .sort()
            .map(|(name, direction)| (name.to_owned(), direction));
        let (name, direction) = sort
            .as_ref()
            .map(|(n, d)| (n.as_str(), *d))
            .unwrap_or(("name", SortDir::Ascending));
        self.sort(name, direction);
    }
    fn sort(&mut self, name: &str, direction: SortDir) {
        self.rows.sort_by(|a, b| {
            direction.apply(match name {
                "state" => a.state.cmp(&b.state),
                "location" => a.location.cmp(&b.location),
                "cpu" => a
                    .cpu
                    .parse::<u64>()
                    .unwrap_or(0)
                    .cmp(&b.cpu.parse().unwrap_or(0)),
                _ => rune_kit::natural_cmp(&a.name, &b.name),
            })
        });
    }
    fn selected(&self) -> Option<Cow<'_, DisplayRow>> {
        let target = if self.console.is_some() {
            self.console_ui.target().map(|t| (t.pool, t.vm))
        } else {
            self.devices.as_ref().map(|t| (t.pool, t.vm))
        };
        if let Some((pool, vm)) = target {
            let inventory = self.inventory.as_ref().filter(|i| i.pool_id == pool)?;
            let vm = inventory.vms.get(&vm)?;
            return Some(Cow::Owned(DisplayRow {
                id: vm.id.to_string(),
                name: vm.name.clone(),
                state: vm.power_state.to_string(),
                location: vm
                    .host
                    .and_then(|id| inventory.hosts.get(&id))
                    .map(|h| h.name.clone())
                    .unwrap_or_else(|| "—".into()),
                cpu: vm.vcpus.to_string(),
                memory: bytes(vm.memory_bytes),
                description: vm.description.clone(),
                vm: Some(vm.id),
            }));
        }
        let id = self.table.cursor()?;
        self.rows.iter().find(|row| row.id == id).map(Cow::Borrowed)
    }
}
fn setup(s: &mut State, waker: Waker) {
    s.waker = Some(waker);
    profiles_view::restore_sessions(s);
}
fn merge_operation_rows(
    previous: &[xenadmin_core::OperationRecord],
    incoming: impl IntoIterator<Item = xenadmin_core::OperationRecord>,
) -> Vec<xenadmin_core::OperationRecord> {
    let mut rows: BTreeMap<_, _> = previous
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

fn poll(s: &mut State, _: f64, _: &mut Tx) -> bool {
    let mut changed = profiles_view::refresh(s);
    changed |= search_view::poll(s);
    changed |= resource_admin::poll(s);
    changed |= storage_view::poll(s);
    changed |= snapshot_policy_view::poll(s);
    changed |= transfers::poll(s);
    changed |= pool_security_view::poll(s);
    changed |= console_view::poll_export(s);
    changed |= metrics_view::poll(s);
    dev_inspection::refresh(s);
    let diagnostic_results = s
        .diagnostics
        .as_ref()
        .map(|d| {
            d.handles
                .iter()
                .map(|(_, h)| format!("{:?}", h.latest()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if diagnostic_results != s.diagnostic_results {
        s.diagnostic_results = diagnostic_results;
        changed = true;
    }
    let mut incoming = s
        .connection
        .as_ref()
        .map(|connection| {
            connection
                .operation_history()
                .iter()
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for operation in &s.operations {
        incoming.push(operation.latest().as_ref().clone());
    }
    let rows = merge_operation_rows(&s.operation_rows, incoming);
    if rows != s.operation_rows {
        s.operation_rows = rows;
        changed = true;
    }
    if let Some(console) = &s.console {
        let snapshot = console.latest();
        if !s
            .console_snapshot
            .as_ref()
            .is_some_and(|old| Arc::ptr_eq(old, &snapshot))
        {
            if let Some(frame) = &snapshot.frame
                && !s
                    .console_snapshot
                    .as_ref()
                    .and_then(|old| old.frame.as_ref())
                    .is_some_and(|old| Arc::ptr_eq(old, frame))
            {
                s.console_pixels.set(
                    u32::from(frame.width),
                    u32::from(frame.height),
                    frame.rgba.to_vec(),
                );
            }
            s.console_snapshot = Some(snapshot);
            changed = true;
        }
    }
    let Some(connection) = &s.connection else {
        return activity::poll(s) || changed;
    };
    let snapshot = connection.latest();
    if s.snapshot
        .as_ref()
        .is_some_and(|last| Arc::ptr_eq(last, &snapshot))
    {
        changed |= activity::poll(s);
        changed |= console_view::follow_migration(s);
        return changed;
    }
    s.inventory = snapshot.inventory().cloned();
    s.snapshot = Some(snapshot);
    if s.diagnostics
        .as_ref()
        .is_some_and(|diagnostics| !diagnostics.is_current(s.snapshot.as_deref()))
    {
        s.diagnostics = None;
        s.diagnostic_results.clear();
    }
    s.rebuild();
    activity::poll(s);
    console_view::follow_migration(s);
    true
}
fn bytes(value: u64) -> String {
    if value == 0 {
        "—".into()
    } else if value >= 1 << 30 {
        format!("{:.1} GiB", value as f64 / (1u64 << 30) as f64)
    } else {
        format!("{:.0} MiB", value as f64 / (1u64 << 20) as f64)
    }
}
fn storage_bytes(value: Option<u64>) -> String {
    match value {
        None => "Unknown".into(),
        Some(0) => "0 B".into(),
        Some(value) => bytes(value),
    }
}
fn caption(value: impl Into<String>, k: &Kit) -> El {
    text(value).font(k.ty.text_small).color(k.c.text_muted)
}
fn heading(value: impl Into<String>, k: &Kit) -> El {
    text(value).font(k.ty.ui_medium).color(k.c.text_base)
}
fn view(s: &State, cx: &mut Cx<State>) -> El {
    let k = Kit::neutral(cx.system_dark());
    let repository_label = "z23 / xenadmin-rs";
    let sidebar = col()
        .w_full()
        .h_full()
        .min_h(0.0)
        .gap(16.0)
        .padding(16.0)
        .child(
            row()
                .gap(10.0)
                .center_y()
                .child(icon(&SERVER, 22.0).color(accent()))
                .child(heading("XenAdmin RS", &k)),
        )
        .child(caption("DIRECT HOST MANAGEMENT", &k))
        .child(profiles_view::sidebar(s, &k))
        .child(
            Tree::new(
                |s: &State| &s.tree,
                |s: &mut State| &mut s.tree,
                |node| &node.label,
            )
            .name("Pools, hosts and virtual machines")
            .row(|node, _| {
                TreeItem::new(&node.label).icon(if node.vm.is_some() { &GUEST } else { &SERVER })
            })
            .on_select(|s: &mut State, id, _| {
                s.metrics = None;
                let Some(node) = s.tree.get(id) else {
                    return;
                };
                let (host, vm) = (node.host, node.vm);
                s.snapshot_view = None;
                s.devices = None;
                s.host_filter = host;
                s.page = Page::Guests;
                s.filter_rows();
                if let Some(id) = vm {
                    s.table.select(id.to_string());
                }
            })
            .build(s, &k)
            .grow(1.0)
            .min_h(0.0),
        )
        .child(
            Button::new("Add connection")
                .variant(ButtonVariant::Outline)
                .on_press(profiles_view::begin_new)
                .build(&k)
                .key("add-connection"),
        )
        .child(
            caption(repository_label, &k)
                .link(rune::Link::new(
                    0..repository_label.len(),
                    "https://github.com/z23/xenadmin-rs",
                ))
                .on_link(|s: &mut State, url, tx| {
                    if let Err(error) = tx.open_url(url) {
                        s.error = Some(error.to_string());
                    }
                })
                .key("repository-link"),
        );
    let table = Table::new(
        |s: &State| &s.table,
        |s: &mut State| &mut s.table,
        |s: &State| s.rows.len(),
        |s: &State, i| Cow::Borrowed(&s.rows[i].id),
    )
    .name(s.page.title())
    .column(
        TableColumn::new("name", "Name")
            .flex(2.0)
            .min(180.0)
            .sortable(),
    );
    let table = if s.page != Page::Networks {
        table.column(
            TableColumn::new(
                "state",
                if s.page == Page::Storage {
                    "Type"
                } else {
                    "State"
                },
            )
            .width(100.0)
            .min(90.0)
            .sortable(),
        )
    } else {
        table
    };
    let table = if s.window_width >= 600.0 || s.page == Page::Networks {
        table.column(
            TableColumn::new(
                "location",
                match s.page {
                    Page::Guests | Page::Templates => "Host",
                    Page::Hosts => "Address",
                    Page::Storage => "Used",
                    Page::Networks => "Bridge",
                },
            )
            .flex(1.0)
            .min(120.0)
            .sortable(),
        )
    } else {
        table
    };
    let table = if s.window_width >= 900.0 && matches!(s.page, Page::Guests | Page::Templates) {
        table
            .column(TableColumn::new("cpu", "vCPUs").width(64.0).sortable())
            .column(TableColumn::new("memory", "Memory limit").width(110.0))
    } else if s.page == Page::Storage {
        table.column(TableColumn::new("memory", "Capacity").width(96.0))
    } else {
        table
    };
    let table = table
        .cell(|s: &State, i, name| {
            let r = &s.rows[i];
            match name {
                "name" => Cell::text(&r.name),
                "state" => Cell::text(&r.state),
                "location" => Cell::text(&r.location).muted(),
                "cpu" => Cell::text(&r.cpu).mono(),
                _ => Cell::text(&r.memory).mono(),
            }
        })
        .on_sort(|s: &mut State, name, dir, _| s.sort(name, dir))
        .on_open(|s: &mut State, _, _| s.show_details = true)
        .zebra(true)
        .build(s, &k)
        .grow(1.0)
        .min_h(0.0);
    let tabs = [
        Page::Guests,
        Page::Templates,
        Page::Hosts,
        Page::Storage,
        Page::Networks,
    ]
    .into_iter()
    .fold(
        row().gap(4.0).role("tablist").label("Inventory views"),
        |tabs, page| {
            tabs.child(
                Button::new(page.title())
                    .variant(if s.page == page {
                        ButtonVariant::Neutral
                    } else {
                        ButtonVariant::Ghost
                    })
                    .on_press(move |s: &mut State| {
                        s.snapshot_view = None;
                        s.metrics = None;
                        s.page = page;
                        s.host_filter = None;
                        s.filter_rows();
                    })
                    .build(&k)
                    .role("tab")
                    .focus_selected(s.page == page)
                    .key(format!("tab-{}", page.title())),
            )
        },
    );
    let mut status = if s.demo {
        "Demo inventory • no host connection".into()
    } else {
        match s.snapshot.as_ref().map(|snapshot| snapshot.state()) {
            Some(ConnectionState::Ready) => format!("Connected directly • {}", s.endpoint.text),
            Some(ConnectionState::Connecting) => "Connecting directly to XAPI…".into(),
            Some(ConnectionState::Synchronizing) => "Synchronizing inventory…".into(),
            Some(ConnectionState::Reconnecting) => {
                "Connection lost • inventory is stale • reconnecting…".into()
            }
            Some(ConnectionState::Failed) => "Connection failed • inventory is stale".into(),
            _ => "No host connected".into(),
        }
    };
    if !s.demo && s.connection.is_some() && s.active_insecure {
        status.push_str(" • TLS verification disabled");
    }
    let mut content = col()
        .w_full()
        .h_full()
        .min_h(0.0)
        .padding(20.0)
        .gap(16.0)
        .child(
            row()
                .w_full()
                .justify_between()
                .center_y()
                .child(heading(s.page.title(), &k))
                .child(
                    TextInput::new(&s.search)
                        .placeholder("Search name, UUID or host")
                        .label("Search inventory")
                        .on_edit(|s: &mut State| &mut s.search)
                        .on_change(|s: &mut State, _| s.filter_rows())
                        .clear_button()
                        .build(&k)
                        .w(if s.window_width < 800.0 { 180.0 } else { 270.0 })
                        .key("search"),
                ),
        )
        .child(tabs);
    if s.demo {
        content = content.child(
            row()
                .w_full()
                .padding(10.0)
                .radius(6.0)
                .fill(k.c.layer_01)
                .child(caption(
                    "DEMO  •  Synthetic data. Host actions are unavailable.",
                    &k,
                )),
        );
    }
    if let Some(error) = s
        .error
        .as_deref()
        .or_else(|| s.snapshot.as_ref().and_then(|snapshot| snapshot.error()))
    {
        content = content.child(
            text(error.to_owned())
                .key("connection-error")
                .font(k.ty.text_small)
                .color(k.c.danger.icon)
                .wrap()
                .role("alert"),
        );
    }
    if s.rows.is_empty() {
        let (title, detail) = if s.inventory.is_none() {
            (
                "Connect directly to your hosts",
                "Add a host endpoint to discover its pool and virtual machines. Xen Orchestra is not required.",
            )
        } else {
            (
                "No matching objects",
                "Choose another view or clear the inventory search.",
            )
        };
        content = content.child(
            col()
                .grow(1.0)
                .w_full()
                .center()
                .gap(12.0)
                .child(icon(&SERVER, 40.0).color(accent()))
                .child(heading(title, &k))
                .child(caption(detail, &k).max_w(460.0).wrap()),
        );
    } else {
        content = content.child(table);
    }
    content = content.child(
        row()
            .w_full()
            .justify_between()
            .child(caption(format!("{} objects", s.rows.len()), &k))
            .child(caption(status, &k).wrap().max_w(if s.window_width < 800.0 {
                300.0
            } else {
                600.0
            })),
    );
    if s.snapshot_view.is_some() {
        content = vm_snapshots::view(s, &k);
    }
    if s.devices.is_some() {
        content = vm_devices::view(s, &k);
    }
    if s.console.is_some() {
        content = console_view::view(s, &k);
    }
    if s.metrics.is_some() {
        content = metrics_view::view(s, &k);
    }
    if s.show_details && s.window_width < 1200.0 && s.console.is_none() {
        content = col()
            .w_full()
            .h_full()
            .min_h(0.0)
            .gap(8.0)
            .child(
                Button::new(if s.snapshot_view.is_some() {
                    "Back to snapshots"
                } else {
                    "Back to inventory"
                })
                .variant(ButtonVariant::Ghost)
                .on_press(|s: &mut State| s.show_details = false)
                .build(&k)
                .key("back-to-inventory"),
            )
            .child(details(s, &k).grow(1.0).min_h(0.0));
    }
    let mut controls = row().gap(4.0);
    if s.window_width < 800.0 {
        controls = controls.child(
            Button::new("Connect")
                .variant(ButtonVariant::Outline)
                .on_press(profiles_view::begin_new)
                .build(&k)
                .key("add-connection"),
        );
    }
    if s.window_width < 800.0 {
        controls = controls.child(
            Button::new("Connections")
                .variant(ButtonVariant::Ghost)
                .on_press(|s: &mut State| s.workspace.show_manager = true)
                .build(&k),
        );
    }
    if s.window_width < 1200.0 {
        controls = controls.child(
            Button::new("Details")
                .variant(ButtonVariant::Ghost)
                .disabled(s.selected().is_none())
                .on_press(|s: &mut State| {
                    s.metrics = None;
                    s.show_details = !s.show_details;
                })
                .build(&k)
                .key("show-details"),
        );
    }
    controls = controls.child(vm_lifecycle::history_button(s, &k));
    controls = controls.child(
        Button::new("Search pools")
            .on_press(search_view::open)
            .build(&k)
            .key("search-pools-open"),
    );
    controls = controls.child(
        Button::new("Snapshot schedules")
            .disabled(s.inventory.is_none())
            .on_press(snapshot_policy_view::open)
            .build(&k)
            .key("snapshot-schedules-open"),
    );
    controls = controls.child(
        Button::new("Archives")
            .on_press(transfers::open)
            .build(&k)
            .key("archives-open"),
    );
    controls = controls.child(
        Button::new("Organize")
            .disabled(s.inventory.is_none())
            .on_press(organization_view::open)
            .build(&k)
            .key("organization-open"),
    );
    controls = controls.child(
        Button::new("Activity")
            .on_press(activity::open)
            .build(&k)
            .key("activity-open"),
    );
    controls = controls.child(
        Button::new("Disconnect")
            .variant(ButtonVariant::Ghost)
            .disabled(s.connection.is_none())
            .on_press(profiles_view::disconnect_selected)
            .build(&k)
            .key("disconnect"),
    );
    if s.inventory.is_some() {
        controls = controls.child(
            Button::new("Pool")
                .on_press(|s: &mut State| {
                    resource_admin::open_pool(s, resource_admin::Kind::History)
                })
                .build(&k)
                .key("manage-pool"),
        );
        if s.page == Page::Storage && s.console.is_none() {
            controls = controls.child(
                Button::new("New storage")
                    .on_press(storage_view::open_new)
                    .build(&k)
                    .key("create-storage"),
            );
        }
        if s.page == Page::Networks && s.console.is_none() {
            controls = controls.child(
                Button::new("New network")
                    .disabled(!s.demo && !s.can_mutate())
                    .on_press(|s: &mut State| {
                        resource_admin::open_pool(s, resource_admin::Kind::CreateNetwork)
                    })
                    .build(&k)
                    .key("create-network"),
            );
        }
    }
    if let Some(notice) = resource_admin::notice(s, &k) {
        content = col()
            .w_full()
            .h_full()
            .min_h(0.0)
            .child(notice)
            .child(content.grow(1.0).min_h(0.0));
    }
    let toolbar = if s.window_width < 800.0 {
        controls = controls.flex_wrap();
        content = col()
            .w_full()
            .h_full()
            .min_h(0.0)
            .child(controls.flex_wrap().padding(8.0))
            .child(content.grow(1.0).min_h(0.0));
        Toolbar::new().title("XenAdmin RS")
    } else {
        Toolbar::new()
            .title(
                s.inventory
                    .as_ref()
                    .map(|i| i.pool_name.as_str())
                    .unwrap_or("Direct connections"),
            )
            .trailing(controls)
    };
    let mut shell = SplitView::new(content)
        .toolbar(toolbar)
        .titlebar_inset(cx.titlebar_inset());
    if s.window_width >= 800.0 {
        shell = shell.sidebar(&s.sidebar, |s: &mut State| &mut s.sidebar, sidebar);
    }
    if s.window_width >= 1200.0 {
        shell = shell.inspector(
            &s.inspector,
            |s: &mut State| &mut s.inspector,
            details(s, &k),
        );
    }
    let base = shell.build(&k);
    let root = if s.show_connect {
        rune_kit::root(base, [connection_dialog(s, &k)])
    } else if s.snapshot_policy_workspace.is_some() {
        rune_kit::root(base, [snapshot_policy_view::dialog(s, &k)])
    } else if s.host_workspace.is_some() {
        rune_kit::root(base, [host_admin::dialog(s, &k)])
    } else if s.network_workspace.is_some() {
        rune_kit::root(base, [network_admin::dialog(s, &k)])
    } else if s.appliances.is_some() {
        rune_kit::root(base, [appliance_view::dialog(s, &k)])
    } else if s.organization.is_some() {
        rune_kit::root(base, [organization_view::dialog(s, &k)])
    } else if s.activity.show {
        rune_kit::root(base, [activity::dialog(s, &k)])
    } else if s.storage_editor.is_some() {
        rune_kit::root(base, [storage_view::dialog(s, &k)])
    } else if s.pool_security.is_some() {
        rune_kit::root(base, [pool_security_view::dialog(s, &k)])
    } else if s.pool_editor.is_some() {
        rune_kit::root(base, [pool_view::dialog(s, &k)])
    } else if s.resource_editor.is_some() {
        rune_kit::root(base, [resource_admin::dialog(s, &k)])
    } else if s.transfers.show {
        rune_kit::root(base, [transfers::dialog(s, &k)])
    } else if s.vm_migration.is_some() {
        rune_kit::root(base, [vm_migration::dialog(s, &k)])
    } else if s.vm_lifecycle.is_some() {
        rune_kit::root(base, [vm_lifecycle::dialog(s, &k)])
    } else if s.placement.is_some() {
        rune_kit::root(base, [vm_placement::dialog(s, &k)])
    } else if s.properties.is_some() {
        rune_kit::root(base, [vm_properties::dialog(s, &k)])
    } else if s.snapshot_dialog.is_some() {
        rune_kit::root(base, [vm_snapshots::dialog(s, &k)])
    } else if let Some((vm, action, name)) = &s.force_action {
        let (vm, action) = (*vm, *action);
        let dialog=col().w(460.0).padding(24.0).gap(16.0).radius(10.0).fill(k.c.bg_base)
            .child(heading(format!("{} this VM?",action.label()),&k))
            .child(caption(format!("Target: {name}\nUUID: {vm}"),&k).wrap_anywhere())
            .child(caption("This interrupts the guest immediately and can lose unsaved data. A clean shutdown is safer when the guest responds.",&k).wrap())
            .child(row().gap(8.0).justify_end().child(Button::new("Cancel").variant(ButtonVariant::Ghost).on_press(|s:&mut State|s.force_action=None).build(&k))
                .child(Button::new(action.label()).variant(ButtonVariant::Danger).on_press(move|s:&mut State|s.submit_power(vm,action)).build(&k)));
        rune_kit::root(
            base,
            [el()
                .w_full()
                .h_full()
                .absolute(0.0, 0.0)
                .center()
                .fill(rune::Color::BLACK.alpha(0.5))
                .role("dialog")
                .label("Confirm force operation")
                .modal(false, |s: &mut State| s.force_action = None)
                .child(dialog)],
        )
    } else if let Some((id, name)) = &s.unknown_review {
        let id = *id;
        let dialog = col().w(460.0).padding(24.0).gap(16.0).radius(10.0).fill(k.c.bg_base)
            .child(heading("Acknowledge unknown outcome?", &k))
            .child(caption(format!("VM: {name}"), &k).wrap_anywhere())
            .child(caption("The earlier operation may still be running. Inspect the host and stop other client instances handling it before continuing. Acknowledging releases this client's VM block; it does not cancel the host task or establish its result.", &k).wrap())
            .child(row().gap(8.0).justify_end()
                .child(Button::new("Cancel").variant(ButtonVariant::Ghost).on_press(|s:&mut State|s.unknown_review=None).build(&k))
                .child(Button::new("Acknowledge outcome").variant(ButtonVariant::Warning)
                    .on_press(move|s:&mut State|{s.control_operation(id,xenadmin_core::OperationControl::AcknowledgeUnknown);s.unknown_review=None;}).build(&k)));
        rune_kit::root(
            base,
            [el()
                .w_full()
                .h_full()
                .absolute(0.0, 0.0)
                .center()
                .fill(rune::Color::BLACK.alpha(0.5))
                .role("dialog")
                .label("Acknowledge unknown outcome")
                .modal(false, |s: &mut State| s.unknown_review = None)
                .child(dialog)],
        )
    } else {
        base
    };
    let root = if s.workspace.show_manager {
        rune_kit::root(root, [profiles_view::dialog(s, &k)])
    } else {
        root
    };
    let root = if s.searches.is_some() {
        rune_kit::root(root, [search_view::dialog(s, &k)])
    } else {
        root
    };
    root.key("application")
        .on_layout_tx(|s: &mut State, rect, _| {
            if rect[2].is_finite() && rect[2] > 0.0 && (s.window_width - rect[2]).abs() > 0.5 {
                s.window_width = rect[2];
            }
            if rect[3].is_finite() && rect[3] > 0.0 && (s.window_height - rect[3]).abs() > 0.5 {
                s.window_height = rect[3];
            }
        })
}
fn details(s: &State, k: &Kit) -> El {
    let mut detail = col()
        .w_full()
        .h_full()
        .padding(20.0)
        .gap(18.0)
        .child(heading("Details", k))
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        });
    let Some(selected) = s.selected() else {
        return detail.child(caption("Select an object to inspect it.", k));
    };
    let field = |name: &str, value: &str| {
        col().gap(5.0).child(caption(name, k)).child(
            text(value.to_owned())
                .font(k.ty.text_small)
                .color(k.c.text_base)
                .wrap_anywhere()
                .selectable(true),
        )
    };
    detail = detail.child(
        row()
            .gap(8.0)
            .center_y()
            .child(
                icon(
                    if selected.vm.is_some() {
                        &GUEST
                    } else {
                        &SERVER
                    },
                    20.0,
                )
                .color(accent()),
            )
            .child(heading(&selected.name, k)),
    );
    if (s.page == Page::Guests || s.console.is_some()) && selected.vm.is_some() {
        detail = detail.child(
            Button::new("Disks and network interfaces")
                .variant(ButtonVariant::Outline)
                .on_press(vm_devices::open)
                .build(k),
        );
        detail = detail.child(
            Button::new("Properties")
                .variant(ButtonVariant::Outline)
                .disabled(!s.demo && !s.can_mutate())
                .on_press(vm_properties::open)
                .build(k)
                .key("open-properties"),
        );
        detail = detail.child(
            Button::new("Snapshots")
                .variant(ButtonVariant::Outline)
                .on_press(vm_snapshots::open)
                .build(k)
                .key("open-snapshots"),
        );
    }
    detail = detail
        .child(field("UUID", &selected.id))
        .child(field("State", &selected.state))
        .child(field("Location", &selected.location))
        .child(field(
            "Description",
            if selected.description.is_empty() {
                "No description"
            } else {
                &selected.description
            },
        ));
    if selected.vm.is_some() {
        detail = detail
            .child(field("Static memory maximum", &selected.memory))
            .child(field("Virtual CPUs", &selected.cpu));
    }
    if let Some(vm) = selected.vm {
        detail = detail
            .child(vm_lifecycle::controls(s, vm, k))
            .child(vm_migration::button(s, vm, k))
            .child(transfers::button(s, vm, k));
    }
    detail = detail.child(metrics_view::button(s, k));
    if (s.page != Page::Guests && s.console.is_none()) || selected.vm.is_none() {
        return resource_details::append(
            s,
            &selected,
            detail
                .child(resource_admin::buttons(s, k))
                .child(network_admin::button(s, k))
                .child(storage_view::button(s, k))
                .child(host_admin::button(s, k)),
            k,
        );
    }
    let mut actions = col().gap(8.0);
    if s.connection.as_ref().is_some_and(|c| c.is_read_only()) {
        actions = actions.child(
            caption(
                "Read-only connection: changes and interactive consoles are disabled.",
                k,
            )
            .wrap(),
        );
    }
    if let Some(vm) = selected
        .vm
        .and_then(|id| s.inventory.as_ref().and_then(|i| i.vms.get(&id)))
    {
        if !s.demo && matches!(vm.power_state, PowerState::Halted | PowerState::Suspended) {
            actions = actions.child(
                Button::new(if vm.power_state == PowerState::Suspended {
                    "Resume on host…"
                } else {
                    "Start on host…"
                })
                .variant(ButtonVariant::Outline)
                .disabled(!s.snapshot.as_ref().is_some_and(|s| s.is_current()))
                .on_press(vm_placement::open)
                .build(k),
            );
        }
        if !s.demo && vm.allowed_actions.is_empty() {
            actions = actions.child(
                caption(
                    "No power actions are available in the received VM data. Check action availability below for the host's current reason. This check does not remove restrictions.",
                    k,
                )
                .wrap()
                .role("status")
                .key("power-actions-unavailable"),
            );
        }
        if !s.demo
            && !s
                .snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.is_current())
        {
            actions = actions.child(
                caption("The connection is not current. Power actions will stay disabled until it reconnects.", k)
                    .wrap()
                    .role("status")
                    .key("power-actions-stale"),
            );
        }
        if !s.demo {
            actions = actions.child(
                Button::new("Check action availability")
                    .variant(ButtonVariant::Ghost)
                    .disabled(!s.snapshot.as_ref().is_some_and(|s| s.is_current()))
                    .on_press(State::diagnose_actions)
                    .build(k),
            );
        }
        let diagnostics = s.diagnostics.as_ref().filter(|diagnostics| {
            diagnostics.context.vm == vm.id && diagnostics.is_current(s.snapshot.as_deref())
        });
        for action in xenadmin_core::PowerAction::ALL {
            let enabled = s.can_mutate() && vm.allowed_actions.contains(&action);
            actions = actions.child(
                Button::new(action.label())
                    .variant(if action.is_force() {
                        ButtonVariant::Ghost
                    } else {
                        ButtonVariant::Outline
                    })
                    .disabled(!enabled)
                    .on_press(move |s: &mut State| s.request_power(action))
                    .build(k)
                    .key(format!("power-{action:?}")),
            );
            let diagnostic = diagnostics
                .and_then(|d| d.handles.iter().find(|(a, _)| *a == action))
                .map(|(_, handle)| handle.latest());
            let reason = match diagnostic {
                Some(Some(Ok(xenadmin_core::ActionAvailability::Available)))
                    if !vm.allowed_actions.contains(&action) =>
                {
                    Some("The host check passed, but the inventory does not currently allow this action. The check is advisory; the action stays disabled until availability updates.".into())
                }
                Some(Some(Ok(availability))) => Some(availability.explanation()),
                Some(Some(Err(error))) => Some(error),
                Some(None) => Some("Checking with the host…".into()),
                None if !enabled && !s.demo => s
                    .inventory
                    .as_ref()
                    .and_then(|i| i.details.vms.get(&vm.id))
                    .map(|v| {
                        v.power_availability(
                            action,
                            s.snapshot.as_ref().is_some_and(|s| s.is_current()),
                        )
                        .explanation()
                    }),
                None => None,
            };
            if let Some(reason) = reason {
                actions = actions.child(caption(reason, k).wrap());
            }
        }
    } else {
        actions = actions.child(
            caption(
                "This VM is no longer in the inventory. Select it again after reconnecting.",
                k,
            )
            .wrap()
            .role("status"),
        );
    }
    let mut recent: Vec<_> = s
        .operation_rows
        .iter()
        .filter(|record| {
            (Some(record.vm) == selected.vm
                || record.created_vm.is_some() && record.created_vm == selected.vm)
                && s.inventory
                    .as_ref()
                    .is_some_and(|inventory| inventory.pool_id == record.pool)
        })
        .collect();
    recent.sort_by_key(|record| {
        (
            record.state.terminal(),
            std::cmp::Reverse(record.created_at_ms),
        )
    });
    for record in recent.into_iter().take(10) {
        actions = actions.child(
            col()
                .gap(4.0)
                .child(heading(
                    format!("{} • {:?}", record.action.label(), record.state),
                    k,
                ))
                .child(caption(&record.detail, k).wrap()),
        );
        for step in &record.steps {
            actions = actions.child(
                caption(
                    format!("{} • {:?}: {}", step.label, step.state, step.detail),
                    k,
                )
                .wrap(),
            );
        }
        if let Some(error) = &record.local_error {
            actions = actions.child(
                caption(error, k)
                    .wrap()
                    .color(k.c.danger.icon)
                    .role("alert"),
            );
        }
        if record.can_cancel && !record.cancel_requested && !record.state.terminal() {
            let id = record.id;
            actions = actions.child(
                Button::new(
                    if matches!(
                        record.action,
                        xenadmin_core::OperationAction::Lifecycle(_)
                            | xenadmin_core::OperationAction::Snapshot(
                                xenadmin_core::SnapshotOperation::Workflow { .. }
                            )
                    ) {
                        "Stop remaining steps"
                    } else {
                        "Request cancellation"
                    },
                )
                .variant(ButtonVariant::Ghost)
                .on_press(move |s: &mut State| {
                    s.control_operation(id, xenadmin_core::OperationControl::RequestCancellation);
                })
                .build(k),
            );
        }
        if record.state == xenadmin_core::OperationState::OutcomeUnknown {
            let id = record.id;
            if record.can_recheck() {
                actions = actions.child(
                    Button::new("Recheck outcome")
                        .variant(ButtonVariant::Outline)
                        .on_press(move |s: &mut State| {
                            s.control_operation(id, xenadmin_core::OperationControl::Recheck)
                        })
                        .build(k),
                );
            }
            let name = selected.name.clone();
            actions = actions.child(
                Button::new("Review unknown outcome")
                    .variant(ButtonVariant::Warning)
                    .on_press(move |s: &mut State| s.unknown_review = Some((id, name.clone())))
                    .build(k),
            );
        }
    }
    detail
        .child(el().h(1.0).w_full().fill(k.c.border_weak))
        .child(
            Button::new("Open console")
                .variant(ButtonVariant::Outline)
                .disabled(!s.can_mutate() || selected.vm.is_none())
                .on_press(State::open_console)
                .build(k)
                .key("open-console"),
        )
        .child(heading("Operations", k))
        .child(actions)
        .child(
            caption(
                if s.demo {
                    "Demo mode cannot change infrastructure."
                } else {
                    "Actions and property changes are tracked in the operation history, including partial and uncertain outcomes."
                },
                k,
            )
            .wrap(),
        )
}
fn connection_dialog(s: &State, k: &Kit) -> El {
    let field = |name: &str,
                 edit: &TextEdit,
                 accessor: fn(&mut State) -> &mut TextEdit,
                 placeholder: &str| {
        col().gap(6.0).child(caption(name, k)).child(
            TextInput::new(edit)
                .label(name)
                .placeholder(placeholder)
                .on_edit(accessor)
                .build(k),
        )
    };
    let mut dialog = col().w_full().grow(1.0).min_h(0.0)
        .scroll(rune::ScrollSpec { stick_bottom: false })
        .padding(24.0).gap(12.0).radius(12.0).fill(k.c.bg_base).border(1.0, k.c.border_weak)
        .child(heading("Connect to XCP-ng", k))
        .child(profiles_view::form_fields(s,k))
        .child(caption("Use a host management address. Credentials go directly to that host over HTTPS.", k).wrap())
        .child(col().gap(6.0).child(caption("Host endpoint",k))
            .child(TextInput::new(&s.endpoint).label("Host endpoint").placeholder("https://xcp-host.example")
                .on_edit(|s:&mut State|&mut s.endpoint)
                .on_change(|s:&mut State,_|s.accept_invalid_certificates=false)
                .build(k).key("host-endpoint")))
        .child(field("Username", &s.username, |s| &mut s.username, "root"))
        .child(col().gap(6.0).child(caption("Password • or load a saved password in connection options", k))
            .child(TextInput::new(&s.password).secure().label("Password").placeholder("Host password")
                .on_edit(|s: &mut State| &mut s.password)
                .on_submit(|s: &mut State, _| s.connect()).build(k).key("password")))
        .child(Checkbox::new(s.accept_invalid_certificates)
            .label("Disable certificate validation")
            .on_toggle(|s: &mut State| s.accept_invalid_certificates = !s.accept_invalid_certificates)
            .build(k).key("insecure-tls"))
        .child(caption("Applies to this connection, coordinator redirects and consoles. Host identity is not verified.", k).wrap());
    if !s.accept_invalid_certificates {
        dialog = dialog.child(
            Button::new("Certificate options")
                .variant(ButtonVariant::Ghost)
                .on_press(|s: &mut State| s.show_certificate_details = !s.show_certificate_details)
                .build(k)
                .key("certificate-options"),
        );
    }
    if !s.accept_invalid_certificates && s.show_certificate_details {
        dialog = dialog
            .child(field("Certificate SHA-256 • optional", &s.fingerprint, |s| &mut s.fingerprint, "Verified fingerprint for a self-signed host"))
            .child(caption("Leave the fingerprint empty for CA and hostname verification. Verify a fingerprint through a trusted channel before using it.", k).wrap())
            .child(col().gap(6.0).child(caption("Other pool host certificates • optional", k))
                .child(rune_kit::TextArea::new(&s.host_certificates).label("Additional host certificates")
                    .placeholder("One https://host=SHA256 entry per line")
                    .on_edit(|s: &mut State| &mut s.host_certificates).build(k).h(72.0)));
    }
    dialog = dialog.child(if let Some(error) = &s.error {
        text(error.to_owned())
            .font(k.ty.text_small)
            .color(k.c.danger.icon)
            .wrap()
            .role("alert")
    } else {
        el()
    });
    let controls = row()
        .w_full()
        .padding(16.0)
        .justify_end()
        .gap(8.0)
        .child(
            Button::new("Cancel")
                .variant(ButtonVariant::Ghost)
                .on_press(|s: &mut State| {
                    profiles_view::cancel_editor(s);
                    s.show_connect = false;
                    s.password = secure_field();
                })
                .build(k)
                .key("cancel-connect"),
        )
        .child(
            Button::new("Connect")
                .on_press(State::connect)
                .build(k)
                .key("connect"),
        );
    let dialog = col()
        .w((s.window_width - 32.0).clamp(280.0, 480.0))
        .h((s.window_height - 48.0).clamp(280.0, 720.0))
        .radius(12.0)
        .fill(k.c.bg_base)
        .child(dialog)
        .child(controls);
    el().key("connect-dialog")
        .role("dialog")
        .label("Connect to XCP-ng")
        .w_full()
        .h_full()
        .absolute(0.0, 0.0)
        .fill(rune::Color::BLACK.alpha(0.5))
        .center()
        .modal(false, |s: &mut State| {
            profiles_view::cancel_editor(s);
            s.show_connect = false;
            s.password = secure_field();
        })
        .child(dialog)
}
fn demo(s: &mut State) {
    s.metrics = None;
    console_view::release(s);
    s.resource_editor = None;
    s.network_workspace = None;
    s.storage_editor = None;
    s.pool_editor = None;
    s.pool_security = None;
    s.resource_operations.clear();
    s.resource_rows.clear();
    s.activity = activity::Activity::default();
    s.devices = None;
    s.vm_lifecycle = None;
    s.vm_migration = None;
    s.operation_rows.clear();
    s.operations.clear();
    s.properties = None;
    s.organization = None;
    s.appliances = None;
    s.placement = None;
    s.diagnostics = None;
    s.diagnostic_results.clear();
    s.console_snapshot = None;
    s.console_ui = Default::default();
    s.workspace.show_manager = false;
    s.snapshot_view = None;
    s.snapshot_dialog = None;
    s.console = None;
    s.force_action = None;
    s.unknown_review = None;
    s.connection = None;
    s.snapshot = None;
    s.demo = true;
    s.error = None;
    s.show_connect = false;
    s.host_filter = None;
    let pool_id = PoolId::parse("10000000-0000-0000-0000-000000000001").unwrap();
    let mut hosts = BTreeMap::new();
    for i in 1..=3 {
        let id = HostId::parse(&format!("20000000-0000-0000-0000-{i:012}")).unwrap();
        hosts.insert(
            id,
            Host {
                id,
                name: format!("xcp-lab-{i:02}"),
                address: format!("192.0.2.{}", i + 10),
                enabled: true,
                version: "8.3".into(),
            },
        );
    }
    let host_ids: Vec<_> = hosts.keys().copied().collect();
    let names = [
        "Xen Orchestra",
        "Database",
        "Application",
        "Build runner",
        "Monitoring",
        "Gateway",
        "Development",
        "File services",
    ];
    let mut vms = BTreeMap::new();
    for i in 0..48 {
        let id = VmId::parse(&format!("30000000-0000-0000-0000-{i:012}")).unwrap();
        let running = i % 5 != 0;
        vms.insert(
            id,
            Vm {
                allowed_actions: vec![],
                can_snapshot: true,
                id,
                name: if i < 8 {
                    names[i].into()
                } else {
                    format!("{} {:02}", names[i % 8], i / 8 + 1)
                },
                description: "Synthetic inventory for exploring the desktop interface.".into(),
                power_state: if running {
                    PowerState::Running
                } else {
                    PowerState::Halted
                },
                host: running.then_some(host_ids[i % 3]),
                vcpus: if i % 3 == 0 { 4 } else { 2 },
                memory_bytes: (2 + i % 4) as u64 * (1 << 30),
                is_template: false,
                is_control_domain: false,
            },
        );
    }
    s.inventory = Some(Arc::new(Inventory {
        pool_id,
        pool_name: "Demonstration pool".into(),
        coordinator: Some(host_ids[0]),
        hosts,
        vms,
        snapshots: xenadmin_core::SnapshotInventory::from(BTreeMap::from_iter((0..3).map(|i| {
            let id = xenadmin_core::SnapshotId::parse(&format!("60000000-0000-0000-0000-{i:012}"))
                .unwrap();
            (
                id,
                xenadmin_core::Snapshot {
                    id,
                    vm: Some(VmId::parse("30000000-0000-0000-0000-000000000000").unwrap()),
                    name: ["Before upgrade", "Known good", "Initial installation"][i].into(),
                    description: "Synthetic disk snapshot".into(),
                    created_at: Some(format!("2026110{}T10:00:00Z", 7 - i)),
                    power_state: PowerState::Halted,
                    allowed_actions: vec![xenadmin_core::SnapshotAction::Revert],
                },
            )
        }))),
        storage: vec![xenadmin_core::Storage {
            uuid: uuid::Uuid::from_u128(0x40000000000000000000000000000001),
            name: "Demo storage".into(),
            kind: "nfs".into(),
            capacity_bytes: Some(2 << 40),
            used_bytes: Some(512 << 30),
        }],
        networks: vec![xenadmin_core::Network {
            uuid: uuid::Uuid::from_u128(0x50000000000000000000000000000001),
            name: "Demo network".into(),
            bridge: "xenbr0".into(),
        }],
        revision: 1,
        details: Default::default(),
    }));
    if let Some(inventory) = &mut s.inventory {
        demo_details::populate(Arc::make_mut(inventory));
    }
    s.page = Page::Guests;
    s.rebuild();
}
fn main() {
    let spec = AppSpec {
        setup: Some(setup),
        poll: Some(poll),
        scenes: &[
            ("demo", demo),
            ("search-pools", search_view::scene),
            ("performance", metrics_view::scene),
            ("unavailable-actions", |s| {
                demo(s);
                s.demo = false;
                s.error = Some("Synthetic unavailable-action preview • no host connection".into());
                s.table.select("30000000-0000-0000-0000-000000000001");
                s.show_details = true;
            }),
            ("vm-devices", |s| {
                demo(s);
                s.table.select("30000000-0000-0000-0000-000000000000");
                vm_devices::open(s);
            }),
            ("properties", |s| {
                demo(s);
                s.table.select("30000000-0000-0000-0000-000000000000");
                vm_properties::open(s);
            }),
            ("properties-running", |s| {
                demo(s);
                s.table.select("30000000-0000-0000-0000-000000000001");
                vm_properties::open(s);
            }),
            ("vm-create", vm_lifecycle::scene),
            ("device-properties", vm_devices::properties_scene),
            ("home-host", vm_properties::home_host_scene),
            ("vm-migration", vm_migration::scene),
            ("archive-export", transfers::scene),
            ("ovf-import", |s| transfers::ovf_scene(s, false)),
            ("ovf-export", |s| transfers::ovf_scene(s, true)),
            ("disk-export", transfers::disk_scene),
            ("snapshots", |s| {
                demo(s);
                let id = VmId::parse("30000000-0000-0000-0000-000000000000").unwrap();
                s.table.select(id.to_string());
                vm_snapshots::open(s);
            }),
            ("snapshot-create", |s| {
                demo(s);
                let id = VmId::parse("30000000-0000-0000-0000-000000000000").unwrap();
                s.table.select(id.to_string());
                vm_snapshots::open(s);
                vm_snapshots::begin_create(s);
            }),
            ("snapshot-restore", vm_snapshots::restore_scene),
            ("snapshot-edit", vm_snapshots::edit_scene),
            ("connections", |s| profiles_view::scene(s, false)),
            ("network-interfaces", network_admin::scene),
            ("host-administration", host_admin::scene),
            ("snapshot-schedules", snapshot_policy_view::scene),
            ("network-create", |s| resource_admin::scene(s, false)),
            ("activity", activity::scene),
            ("organization", organization_view::scene),
            ("appliances", appliance_view::scene),
            ("appliance-review", appliance_view::review_scene),
            ("storage-create", |s| storage_view::scene(s, false)),
            ("storage-review", |s| storage_view::scene(s, true)),
            ("storage-discovery", storage_view::discovery_scene),
            ("pool-security", |s| pool_security_view::scene(s, false)),
            ("pool-join", profiles_view::join_scene),
            ("pool-security-review", |s| {
                pool_security_view::scene(s, true)
            }),
            ("pool-policies", |s| pool_view::scene(s, false)),
            ("pool-policy-review", |s| pool_view::scene(s, true)),
            ("pool-administration", |s| resource_admin::scene(s, true)),
            ("connection-settings", |s| profiles_view::scene(s, true)),
            ("connect", |s| s.show_connect = true),
            ("connect-insecure", |s| {
                s.show_connect = true;
                s.accept_invalid_certificates = true;
            }),
        ],
        min_size: Some((480.0, 640.0)),
        ..AppSpec::new("XenAdmin RS", (1440.0, 900.0), init, view)
    };
    rune::App::new(spec)
        .tab_visits_controls(true)
        .modifier_keys()
        .on_focus(|s, focused, _| {
            if !focused {
                console_view::release(s);
            }
        })
        .on_close(|s, _| {
            s.save_preferences();
            profiles_view::disconnect_all(s);
        })
        .run();
}
#[cfg(test)]
mod tests {
    #[test]
    fn sidebar_links_to_the_repository_without_a_demo_entry() {
        for size in [(513.0, 652.0), (1440.0, 900.0)] {
            let mut runtime = rune::Runtime::new(
                super::init(),
                super::view,
                size,
                rune::Env {
                    system_dark: true,
                    scale: 1.0,
                },
            );
            for _ in 0..30 {
                runtime.frame(1.0 / 60.0);
            }
            assert!(runtime.find("demo").is_none());
            if size.0 >= 800.0 {
                let (_, y) = runtime.find("repository-link").unwrap();
                runtime.pointer_down(30.0, y);
                runtime.pointer_up(30.0, y);
                runtime.frame(0.0);
                assert_eq!(
                    runtime.take_url_requests(),
                    ["https://github.com/z23/xenadmin-rs"]
                );
            }
            assert!(!runtime.state.demo);
        }
    }

    #[test]
    fn action_checks_follow_vm_availability_without_expiring_on_unrelated_events() {
        use xenadmin_core::{PowerAction, RunningOperation};
        let mut state = super::init();
        super::demo(&mut state);
        let mut inventory = state.inventory.unwrap().as_ref().clone();
        let vm = *inventory.vms.keys().next().unwrap();
        let original = super::ActionDiagnosticContext::capture(true, 7, &inventory, vm);

        inventory.revision += 1;
        inventory.pool_name = "Renamed pool".into();
        inventory.vms.get_mut(&vm).unwrap().name = "Renamed guest".into();
        let other = *inventory.vms.keys().find(|id| **id != vm).unwrap();
        inventory
            .vms
            .get_mut(&other)
            .unwrap()
            .allowed_actions
            .clear();
        assert_eq!(
            original,
            super::ActionDiagnosticContext::capture(true, 7, &inventory, vm),
            "unrelated events and display names do not invalidate the check"
        );

        let mut blocked = inventory.clone();
        blocked
            .details
            .vms
            .entry(vm)
            .or_default()
            .blocked_operations
            .insert("clean_shutdown".into(), "Protected by operator".into());
        let blocked_context = super::ActionDiagnosticContext::capture(true, 7, &blocked, vm);
        assert_ne!(
            original, blocked_context,
            "a newly added block expires Available"
        );
        assert_ne!(
            blocked_context,
            super::ActionDiagnosticContext::capture(true, 7, &inventory, vm),
            "removing the block expires Blocked"
        );

        inventory.vms.get_mut(&vm).unwrap().allowed_actions = PowerAction::ALL.to_vec();
        let before_task = super::ActionDiagnosticContext::capture(true, 7, &inventory, vm);
        inventory
            .details
            .vms
            .entry(vm)
            .or_default()
            .current_operations
            .push(RunningOperation {
                task: None,
                operation: "snapshot".into(),
            });
        assert_ne!(
            before_task,
            super::ActionDiagnosticContext::capture(true, 7, &inventory, vm),
            "starting a task expires earlier checks even if allowed actions have not updated"
        );
    }

    #[test]
    fn action_checks_require_the_same_current_connection_and_vm_state() {
        let mut state = super::init();
        super::demo(&mut state);
        let mut inventory = state.inventory.unwrap().as_ref().clone();
        let vm = *inventory.vms.keys().next().unwrap();
        let original = super::ActionDiagnosticContext::capture(true, 7, &inventory, vm);
        assert!(original.is_some());
        assert_eq!(
            super::ActionDiagnosticContext::capture(false, 7, &inventory, vm),
            None,
            "a disconnected or stale inventory cannot retain a check"
        );
        assert_ne!(
            original,
            super::ActionDiagnosticContext::capture(true, 8, &inventory, vm)
        );
        inventory.vms.get_mut(&vm).unwrap().power_state = super::PowerState::Paused;
        assert_ne!(
            original,
            super::ActionDiagnosticContext::capture(true, 7, &inventory, vm)
        );
        inventory.vms.remove(&vm);
        assert_eq!(
            super::ActionDiagnosticContext::capture(true, 7, &inventory, vm),
            None
        );
    }

    #[test]
    fn devices_keep_their_target_and_leave_compact_details() {
        let mut state = super::init();
        super::demo(&mut state);
        let original = super::VmId::parse("30000000-0000-0000-0000-000000000000").unwrap();
        state.table.select(original.to_string());
        state.show_details = true;
        super::vm_devices::open(&mut state);
        assert!(!state.show_details);
        state.table.select("30000000-0000-0000-0000-000000000001");
        assert_eq!(state.selected().and_then(|r| r.vm), Some(original));
        let mut runtime = rune::Runtime::new(
            state,
            super::view,
            (513.0, 652.0),
            rune::Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        for _ in 0..30 {
            runtime.frame(1.0 / 60.0);
        }
        assert!(runtime.find("close-vm-devices").is_some());
        super::vm_properties::open(&mut runtime.state);
        super::demo(&mut runtime.state);
        assert!(runtime.state.devices.is_none());
        assert!(runtime.state.properties.is_none());
        assert!(runtime.state.placement.is_none());
    }
    use super::*;
    #[test]
    fn editing_the_host_resets_a_remembered_certificate_bypass() {
        use rune::{Env, Key, Runtime};
        let mut state = init();
        state.show_connect = true;
        state.endpoint.text = "https://original.example".into();
        state.accept_invalid_certificates = true;
        let mut rt = Runtime::new(
            state,
            view,
            (1440.0, 900.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        for _ in 0..60 {
            rt.frame(1.0 / 60.0);
        }
        let (x, y) = rt.find("host-endpoint").unwrap();
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(0.0);
        rt.key(Key::Text("another".into()).into());
        rt.frame(0.0);
        assert!(
            !rt.state.accept_invalid_certificates,
            "a new host needs its own explicit choice"
        );
        assert!(rt.find("certificate-options").is_some());
    }
    #[test]
    fn connection_form_exposes_one_opt_out_and_keeps_verified_options_available() {
        use rune::{Env, Runtime};
        let mut state = init();
        state.show_connect = true;
        state.accept_invalid_certificates = false;
        let mut rt = Runtime::new(
            state,
            view,
            (1440.0, 900.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        for _ in 0..60 {
            rt.frame(1.0 / 60.0);
        }
        let (x, y) = rt.find("insecure-tls").unwrap();
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(0.0);
        assert!(rt.state.accept_invalid_certificates);
        assert!(rt.find("certificate-options").is_none());
        assert!(
            rt.accessibility_json()
                .to_string()
                .contains("coordinator redirects and consoles")
        );
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
        let (x, y) = rt.find("insecure-tls").unwrap();
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(0.0);
        assert!(!rt.state.accept_invalid_certificates);
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
        let (x, y) = rt.find("certificate-options").unwrap();
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(0.0);
        assert!(rt.state.show_certificate_details);
        assert!(
            rt.accessibility_json()
                .to_string()
                .contains("Certificate SHA-256")
        );
        assert_eq!(storage_bytes(None), "Unknown");
        assert_eq!(storage_bytes(Some(0)), "0 B");
    }
    #[test]
    fn search_and_host_filter_select_matching_objects() {
        let mut s = init();
        demo(&mut s);
        s.search.text = "Xen Orchestra".into();
        s.filter_rows();
        assert!(!s.rows.is_empty());
        assert!(s.rows.iter().all(|row| row.name.contains("Xen Orchestra")));
        s.host_filter = s.inventory.as_ref().unwrap().hosts.keys().next().copied();
        s.filter_rows();
        for row in &s.rows {
            assert_eq!(
                s.inventory.as_ref().unwrap().vms[&row.vm.unwrap()].host,
                s.host_filter
            );
        }
    }

    #[test]
    fn real_widgets_open_the_connection_dialog_and_filter_demo_inventory() {
        use rune::{Env, Key, Runtime};
        let mut state = init();
        demo(&mut state);
        let mut rt = Runtime::new(
            state,
            view,
            (1440.0, 900.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        rt.frame(0.0);
        let (x, y) = rt.find("search").expect("search field is visible");
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(0.0);
        rt.key(Key::Text("Xen Orchestra".into()).into());
        rt.frame(0.0);
        assert_eq!(rt.state.rows.len(), 6);
        let tree = rt.accessibility_json().to_string();
        assert!(tree.contains("Demo") || tree.contains("DEMO"));
        let (x, y) = rt.find("add-connection").unwrap();
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(0.0);
        assert!(rt.find("connect-dialog").is_some());
        assert!(
            rt.accessibility_json()
                .to_string()
                .contains("Connect to XCP-ng")
        );
        for _ in 0..60 {
            rt.frame(1.0 / 60.0);
        }
        let (x, y) = rt.find("cancel-connect").unwrap();
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(0.0);
        assert!(
            !rt.state.show_connect,
            "Cancel must close the connection form"
        );
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
        assert!(rt.find("connect-dialog").is_none());
        assert!(rt.state.password.text.is_empty());
    }

    #[test]
    fn narrow_window_keeps_connection_and_vm_details_reachable() {
        use rune::{Env, Runtime};
        let mut state = init();
        demo(&mut state);
        state.table.select(state.rows[0].id.clone());
        let mut rt = Runtime::new(
            state,
            view,
            (513.0, 652.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
        assert_eq!(rt.state.window_width, 513.0);
        for key in ["add-connection", "search", "show-details"] {
            let (x, y) = rt.find(key).expect("compact control remains visible");
            assert!(
                x > 0.0 && x < 513.0 && y > 0.0 && y < 652.0,
                "{key} is within the window"
            );
        }
        let (x, y) = rt.find("show-details").unwrap();
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(0.0);
        assert!(rt.state.show_details);
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
        let (x, y) = rt.find("back-to-inventory").unwrap();
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(0.0);
        assert!(!rt.state.show_details);
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
        let (x, y) = rt.find("add-connection").unwrap();
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(0.0);
        assert!(rt.find("connect-dialog").is_some());
    }
}
