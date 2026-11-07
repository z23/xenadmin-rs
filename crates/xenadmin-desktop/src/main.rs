mod console_view;
mod preferences;

use preferences::Preferences;
use rune::{AppSpec, Cx, El, Icon, TextEdit, Tx, Waker, col, el, icon, row, text};
use rune_kit::{
    Button, ButtonVariant, Cell, Kit, PaneState, SortDir, SplitView, Table, TableColumn,
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
struct State {
    window_width: f32,
    show_details: bool,
    operations: Vec<xenadmin_core::Operation>,
    operation_rows: Vec<xenadmin_core::OperationRecord>,
    force_action: Option<(xenadmin_core::ObservedVm, xenadmin_core::PowerAction)>,
    console: Option<xenadmin_core::Console>,
    console_snapshot: Option<Arc<xenadmin_core::ConsoleSnapshot>>,
    console_pixels: rune::Pixels,
    console_bounds: [f32; 2],
    console_buttons: u8,
    console_keys: Vec<(rune::PhysicalKeyCode, u32)>,
    backend: Result<Backend, String>,
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
    let preferences = Preferences::load();
    let mut table = TableState::new().key("inventory-table");
    for (name, width) in &preferences.columns {
        table.set_column_width(name, *width);
    }
    let mut s = State {
        window_width: 1440.0,
        show_details: false,
        operations: Vec::new(),
        operation_rows: Vec::new(),
        force_action: None,
        console: None,
        console_snapshot: None,
        console_pixels: rune::Pixels::new(),
        console_bounds: [0.0; 2],
        console_buttons: 0,
        console_keys: Vec::new(),
        backend: Backend::start().map_err(|e| e.to_string()),
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
        show_connect: false,
        error: None,
        preferences,
    };
    if rune::extra_args().iter().any(|a| a == "--demo") {
        demo(&mut s);
    }
    s
}
impl State {
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
                self.force_action = Some((observed, action));
            } else {
                self.submit_power(observed, action);
            }
        }
    }
    fn submit_power(&mut self, vm: xenadmin_core::ObservedVm, action: xenadmin_core::PowerAction) {
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
                .power_action(xenadmin_core::OperationId::new(), vm, action)?;
            if let Some(waker) = &self.waker {
                operation.notify_on_change(waker.clone());
            }
            self.operations.push(operation);
            Ok::<(), xenadmin_core::Error>(())
        })();
        self.force_action = None;
        self.error = result.err().map(|error| error.to_string());
    }
    fn open_console(&mut self) {
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
            self.console = Some(console);
            self.console_snapshot = None;
            Ok::<(), xenadmin_core::Error>(())
        })();
        self.error = result.err().map(|e| e.to_string());
    }
    fn connect(&mut self) {
        self.console = None;
        let result = (|| {
            let mut options = ConnectionOptions::new(
                Endpoint::parse(&self.endpoint.text).map_err(|e| e.to_string())?,
            );
            if !self.fingerprint.text.trim().is_empty() {
                options.certificate_fingerprint = Some(
                    self.fingerprint
                        .text
                        .parse()
                        .map_err(|e: xenadmin_core::Error| e.to_string())?,
                );
            }
            options
                .add_host_certificates(&self.host_certificates.text)
                .map_err(|error| error.to_string())?;
            let backend = self.backend.as_ref().map_err(Clone::clone)?;
            if self.username.text.trim().is_empty() || self.password.text.is_empty() {
                return Err("Enter the host username and password.".into());
            }
            let connection = backend.connect(
                options,
                Credentials::new(self.username.text.trim().into(), self.password.text.clone()),
            );
            if let Some(waker) = &self.waker {
                connection.notify_on_change(waker.clone());
            }
            self.connection = Some(connection);
            self.snapshot = None;
            self.inventory = None;
            self.password = secure_field();
            self.demo = false;
            self.host_filter = None;
            self.show_connect = false;
            self.preferences.endpoint = self.endpoint.text.trim().into();
            self.preferences.username = self.username.text.trim().into();
            self.preferences.fingerprint = self.fingerprint.text.trim().into();
            self.preferences.host_certificates = self.host_certificates.text.trim().into();
            self.rebuild();
            self.save_preferences();
            Ok::<(), String>(())
        })();
        self.error = result.err();
    }
    fn save_preferences(&mut self) {
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
                        && !v.is_snapshot
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
                        location: format!("{} used", bytes(sr.used_bytes)),
                        cpu: "—".into(),
                        memory: bytes(sr.capacity_bytes),
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
    fn selected(&self) -> Option<&DisplayRow> {
        let id = self.table.cursor()?;
        self.rows.iter().find(|row| row.id == id)
    }
}
fn setup(s: &mut State, waker: Waker) {
    s.waker = Some(waker);
}
fn poll(s: &mut State, _: f64, _: &mut Tx) -> bool {
    let mut changed = false;
    let mut rows: BTreeMap<_, _> = s
        .connection
        .as_ref()
        .map(|connection| {
            connection
                .operation_history()
                .iter()
                .map(|record| (record.id, record.clone()))
                .collect()
        })
        .unwrap_or_default();
    for operation in &s.operations {
        let record = operation.latest();
        if rows
            .get(&record.id)
            .is_none_or(|existing: &xenadmin_core::OperationRecord| {
                !existing.state.terminal() || record.state.terminal()
            })
        {
            rows.insert(record.id, record.as_ref().clone());
        }
    }
    let rows: Vec<_> = rows.into_values().collect();
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
            if let Some(frame) = &snapshot.frame {
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
        return changed;
    };
    let snapshot = connection.latest();
    if s.snapshot
        .as_ref()
        .is_some_and(|last| Arc::ptr_eq(last, &snapshot))
    {
        return changed;
    }
    s.inventory = snapshot.inventory().cloned();
    s.snapshot = Some(snapshot);
    s.rebuild();
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
fn caption(value: impl Into<String>, k: &Kit) -> El {
    text(value).font(k.ty.text_small).color(k.c.text_muted)
}
fn heading(value: impl Into<String>, k: &Kit) -> El {
    text(value).font(k.ty.ui_medium).color(k.c.text_base)
}
fn view(s: &State, cx: &mut Cx<State>) -> El {
    let k = Kit::neutral(cx.system_dark());
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
                let Some(node) = s.tree.get(id) else {
                    return;
                };
                let (host, vm) = (node.host, node.vm);
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
                .on_press(|s: &mut State| {
                    s.show_connect = true;
                    s.error = None;
                })
                .build(&k)
                .key("add-connection"),
        )
        .child(
            Button::new("Explore demo inventory")
                .variant(ButtonVariant::Ghost)
                .on_press(demo)
                .build(&k)
                .key("demo"),
        )
        .child(caption("Independent of Xen Orchestra", &k));
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
    )
    .column(
        TableColumn::new("state", "State")
            .width(100.0)
            .min(90.0)
            .sortable(),
    );
    let table = if s.window_width >= 600.0 {
        table.column(
            TableColumn::new("location", "Host / location")
                .flex(1.0)
                .min(120.0)
                .sortable(),
        )
    } else {
        table
    };
    let table = if s.window_width >= 900.0 {
        table
            .column(TableColumn::new("cpu", "vCPUs").width(64.0).sortable())
            .column(
                TableColumn::new(
                    "memory",
                    if s.page == Page::Storage {
                        "Capacity"
                    } else {
                        "Memory"
                    },
                )
                .width(96.0),
            )
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
    let status = if s.demo {
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
    if s.console.is_some() {
        content = console_view::view(s, &k);
    }
    if s.show_details && s.window_width < 1200.0 && s.console.is_none() {
        content = col()
            .w_full()
            .h_full()
            .min_h(0.0)
            .gap(8.0)
            .child(
                Button::new("Back to inventory")
                    .variant(ButtonVariant::Ghost)
                    .on_press(|s: &mut State| s.show_details = false)
                    .build(&k)
                    .key("back-to-inventory"),
            )
            .child(details(s, &k).grow(1.0).min_h(0.0));
    }
    let mut controls = row().gap(4.0);
    if s.window_width < 800.0 {
        controls = controls
            .child(
                Button::new("Connect")
                    .variant(ButtonVariant::Outline)
                    .on_press(|s: &mut State| {
                        s.show_connect = true;
                        s.error = None;
                    })
                    .build(&k)
                    .key("add-connection"),
            )
            .child(
                Button::new("Demo")
                    .variant(ButtonVariant::Ghost)
                    .on_press(demo)
                    .build(&k)
                    .key("demo"),
            );
    }
    if s.window_width < 1200.0 {
        controls = controls.child(
            Button::new("Details")
                .variant(ButtonVariant::Ghost)
                .disabled(s.selected().is_none())
                .on_press(|s: &mut State| s.show_details = !s.show_details)
                .build(&k)
                .key("show-details"),
        );
    }
    controls = controls.child(
        Button::new("Disconnect")
            .variant(ButtonVariant::Ghost)
            .disabled(s.connection.is_none())
            .on_press(|s: &mut State| {
                if let Some(connection) = &s.connection {
                    connection.disconnect();
                }
            })
            .build(&k)
            .key("disconnect"),
    );
    let toolbar = if s.window_width < 800.0 {
        content = col()
            .w_full()
            .h_full()
            .min_h(0.0)
            .child(controls.padding(8.0))
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
    } else if let Some((vm, action)) = s.force_action {
        let dialog=col().w(460.0).padding(24.0).gap(16.0).radius(10.0).fill(k.c.bg_base)
            .child(heading(format!("{} this VM?",action.label()),&k))
            .child(caption(format!("Target: {vm}"),&k).wrap())
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
    } else {
        base
    };
    root.key("application")
        .on_layout_tx(|s: &mut State, rect, _| {
            if rect[2].is_finite() && rect[2] > 0.0 && (s.window_width - rect[2]).abs() > 0.5 {
                s.window_width = rect[2];
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
    detail = detail
        .child(
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
        )
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
            .child(field("Memory", &selected.memory))
            .child(field("Virtual CPUs", &selected.cpu));
    }
    let mut actions = col().gap(8.0);
    if let Some(vm) = selected
        .vm
        .and_then(|id| s.inventory.as_ref().and_then(|i| i.vms.get(&id)))
    {
        for action in [
            xenadmin_core::PowerAction::Start,
            xenadmin_core::PowerAction::CleanShutdown,
            xenadmin_core::PowerAction::Reboot,
            xenadmin_core::PowerAction::HardShutdown,
            xenadmin_core::PowerAction::HardReboot,
        ] {
            let enabled = !s.demo
                && s.snapshot
                    .as_ref()
                    .is_some_and(|snapshot| snapshot.is_current())
                && vm.allowed_actions.contains(&action);
            actions = actions.child(
                Button::new(action.label())
                    .variant(if action.is_force() {
                        ButtonVariant::Ghost
                    } else {
                        ButtonVariant::Outline
                    })
                    .disabled(!enabled)
                    .on_press(move |s: &mut State| s.request_power(action))
                    .build(k),
            );
        }
    }
    for record in s
        .operation_rows
        .iter()
        .filter(|record| {
            Some(record.vm) == selected.vm
                && s.inventory
                    .as_ref()
                    .is_some_and(|inventory| inventory.pool_id == record.pool)
        })
        .rev()
        .take(5)
    {
        actions = actions.child(
            col()
                .gap(4.0)
                .child(heading(
                    format!("{} • {:?}", record.action.label(), record.state),
                    k,
                ))
                .child(caption(&record.detail, k).wrap()),
        );
        if let Some(handle) = s
            .operations
            .iter()
            .find(|op| op.latest().id == record.id)
            .filter(|_| !record.state.terminal())
        {
            let id = handle.latest().id;
            actions = actions.child(
                Button::new("Request cancellation")
                    .variant(ButtonVariant::Ghost)
                    .on_press(move |s: &mut State| {
                        if let Some(op) = s.operations.iter().find(|op| op.latest().id == id) {
                            op.request_cancel();
                        }
                    })
                    .build(k),
            );
        }
    }
    detail
        .child(el().h(1.0).w_full().fill(k.c.border_weak))
        .child(
            Button::new("Open console")
                .variant(ButtonVariant::Outline)
                .disabled(
                    s.demo
                        || selected.vm.is_none()
                        || !s
                            .snapshot
                            .as_ref()
                            .is_some_and(|snapshot| snapshot.is_current()),
                )
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
                    "Power actions use XAPI tasks. Interrupted outcomes remain in the local operation history."
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
    let dialog = col().w(480.0).max_h(620.0).scroll(rune::ScrollSpec{stick_bottom:false}).padding(24.0).gap(12.0).radius(12.0).fill(k.c.bg_base).border(1.0, k.c.border_weak)
        .child(heading("Connect to XCP-ng", k)).child(caption("Use a host management address. Credentials go directly to that host over HTTPS.", k).wrap())
        .child(field("Host endpoint", &s.endpoint, |s| &mut s.endpoint, "https://xcp-host.example"))
        .child(field("Username", &s.username, |s| &mut s.username, "root"))
        .child(col().gap(6.0).child(caption("Password • this session only", k)).child(TextInput::new(&s.password).secure().label("Password").placeholder("Host password").on_edit(|s: &mut State| &mut s.password).on_submit(|s: &mut State, _| s.connect()).build(k).key("password")))
        .child(field("Certificate SHA-256 • optional", &s.fingerprint, |s| &mut s.fingerprint, "Verified fingerprint for a self-signed host"))
        .child(caption("Leave the fingerprint empty for CA and hostname verification. Verify a fingerprint through a trusted channel before using it.", k).wrap())
        .child(col().gap(6.0).child(caption("Other pool host certificates • optional",k))
            .child(rune_kit::TextArea::new(&s.host_certificates).label("Additional host certificates").placeholder("One https://host=SHA256 entry per line").on_edit(|s:&mut State|&mut s.host_certificates).build(k).h(72.0)))
        .child(if let Some(error) = &s.error { text(error.to_owned()).font(k.ty.text_small).color(k.c.danger.icon).wrap().role("alert") } else { el() })
        .child(row().justify_end().gap(8.0).child(Button::new("Cancel").variant(ButtonVariant::Ghost).on_press(|s: &mut State| { s.show_connect = false; s.password = secure_field(); }).build(k).key("cancel-connect"))
            .child(Button::new("Connect").on_press(State::connect).build(k).key("connect")));
    el().key("connect-dialog")
        .role("dialog")
        .label("Connect to XCP-ng")
        .w_full()
        .h_full()
        .absolute(0.0, 0.0)
        .fill(rune::Color::BLACK.alpha(0.5))
        .center()
        .modal(false, |s: &mut State| {
            s.show_connect = false;
            s.password = secure_field();
        })
        .child(dialog)
}
fn demo(s: &mut State) {
    s.console = None;
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
                is_snapshot: false,
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
        storage: vec![],
        networks: vec![],
        revision: 1,
    }));
    s.page = Page::Guests;
    s.rebuild();
}
fn main() {
    let spec = AppSpec {
        setup: Some(setup),
        poll: Some(poll),
        scenes: &[("demo", demo), ("connect", |s| s.show_connect = true)],
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
            if let Some(connection) = &s.connection {
                connection.disconnect();
            }
        })
        .run();
}
#[cfg(test)]
mod tests {
    use super::*;
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
