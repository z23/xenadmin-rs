//! The GPUI shell. Actions come from the command table. No extra top-level buttons.

use gpui_kit::Focusable as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::menu::AppMenuBar;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::table::{Column, DataTable, TableDelegate, TableState};
use gpui_kit::component::tree::tree as navigation_tree;
use gpui_kit::component::tree::{TreeItem, TreeState};
use gpui_kit::component::{WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{App, Context, Entity, Menu, MenuItem, SharedString, Window, div, px};

#[cfg(feature = "scenes")]
use gpui_kit::TestSupportExt as _;

use xenadmin_ui::{CommandId, DialogId, Eligibility, ObjectKind, TabId, eligibility};

macro_rules! probe {
    ($id:literal) => {{
        #[cfg(feature = "scenes")]
        {
            div().id($id).test_support()
        }
        #[cfg(not(feature = "scenes"))]
        {
            div().id($id)
        }
    }};
}

use crate::shell_state::ShellState;

pub struct ShellView {
    pub state: ShellState,
    menus: Entity<AppMenuBar>,
    tree: Entity<TreeState>,
    table: Entity<TableState<Rows>>,
    presented: bool,
}

struct Rows {
    lines: Vec<(String, String, String)>,
}

impl TableDelegate for Rows {
    fn columns_count(&self, _: &App) -> usize {
        3
    }

    fn rows_count(&self, _: &App) -> usize {
        self.lines.len()
    }

    fn column(&self, index: usize, _: &App) -> Column {
        let (id, title) = [
            ("name", "Name"),
            ("state", "State"),
            ("location", "Location"),
        ][index];
        Column::new(id, title).width(px(160.))
    }

    fn render_td(
        &mut self,
        row: usize,
        column: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let line = &self.lines[row];
        let text = match column {
            0 => line.0.as_str(),
            1 => line.1.as_str(),
            _ => line.2.as_str(),
        };
        div().child(text.to_string())
    }
}

impl ShellView {
    pub fn new(state: ShellState, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let rows = rows_of(&state);
        let tree = cx.new(|cx| TreeState::new(cx).items(tree_items(&state)));
        let table = cx.new(|cx| TableState::new(Rows { lines: rows }, window, cx));
        let menus = AppMenuBar::new(cx);
        let view = Self {
            state,
            menus,
            tree,
            table,
            presented: false,
        };
        view.install_menus(cx);
        view
    }

    pub fn focus_navigation(&self, window: &mut Window, cx: &mut App) {
        self.tree.update(cx, |tree, cx| tree.focus(window, cx));
    }

    pub fn navigation_index(&self, cx: &App) -> Option<usize> {
        self.tree.read(cx).selected_index()
    }

    pub fn focus_grid(&self, window: &mut Window, cx: &mut App) {
        self.table
            .update(cx, |table, cx| table.focus_handle(cx).focus(window, cx));
    }

    pub fn grid_row(&self, cx: &App) -> Option<usize> {
        self.table.read(cx).selected_row()
    }

    fn install_menus(&self, cx: &mut Context<Self>) {
        let selection = self.state.selection();
        cx.defer(move |cx| install_menus(cx, selection));
    }
}

fn rows_of(state: &ShellState) -> Vec<(String, String, String)> {
    xenadmin_ui::project_rows(&state.inventory, state.rows_page(), None, &state.query)
        .into_iter()
        .map(|row| (row.name, row.state, row.location))
        .collect()
}

fn tree_items(state: &ShellState) -> Vec<TreeItem> {
    let (inserts, _) = xenadmin_ui::infrastructure_nodes(&state.inventory, |_| true);
    let mut hosts: Vec<TreeItem> = Vec::new();
    let mut loose: Vec<TreeItem> = Vec::new();
    let mut pool_label = state.inventory.pool_name.clone();
    for insert in inserts {
        if insert.parent.is_none() {
            pool_label = insert.node.label;
            continue;
        }
        let item = TreeItem::new(insert.key, insert.node.label);
        if insert.node.host.is_some() && insert.node.vm.is_none() {
            hosts.push(item);
        } else if insert.node.vm.is_some() && insert.node.host.is_none() {
            loose.push(item);
        } else if let Some(host) = hosts.last_mut() {
            *host = host.clone().child(item);
        }
    }
    vec![
        TreeItem::new(state.inventory.pool_id.to_string(), pool_label)
            .expanded(true)
            .children(hosts.into_iter().chain(loose)),
    ]
}

fn install_menus(cx: &mut App, selection: xenadmin_ui::Selection) {
    let mut menus: Vec<(SharedString, Vec<MenuItem>)> = Vec::new();
    for command in xenadmin_ui::COMMANDS {
        let item = match eligibility(command.id, selection) {
            Eligibility::Hidden => continue,
            Eligibility::Enabled => MenuItem::action(command.label, crate::ShowGeneral),
            Eligibility::Disabled(reason) => {
                MenuItem::action(format!("{} ({reason})", command.label), crate::ShowGeneral)
                    .disabled(true)
            }
        };
        if let Some((_, items)) = menus
            .iter_mut()
            .find(|(name, _)| name.as_ref() == command.menu)
        {
            items.push(item);
        } else {
            menus.push((command.menu.into(), vec![item]));
        }
    }
    cx.set_menus(
        menus
            .into_iter()
            .map(|(name, items)| Menu::new(name).items(items)),
    );
    if let Some(owned) = cx.get_menus() {
        gpui_kit::component::GlobalState::global_mut(cx).set_app_menus(owned);
    }
}

impl Render for ShellView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.state.clone();
        let tabs = state.tabs();
        let selection = state.selection();
        if state.dialog.is_some() && !self.presented {
            self.presented = true;
            let text = state
                .dialog_text
                .clone()
                .unwrap_or_else(|| "Address, username, and password stay in this preview.".into());
            let title = state.dialog_title.clone();
            let add_server = state.dialog == Some(DialogId::AddServer);
            cx.defer_in(_window, move |_, window, cx| {
                open_text_dialog(window, cx, title, text, add_server);
            });
        }
        v_flex()
            .child(probe!("shell"))
            .size_full()
            .gap_2()
            .p_2()
            .child(self.menus.clone())
            .child(toolbar(selection, cx))
            .child(
                h_flex()
                    .flex_1()
                    .gap_2()
                    .child(
                        div()
                            .id("navigation")
                            .w(px(240.))
                            .h_full()
                            .child(navigation_tree(&self.tree, |_, entry, _, _, _| {
                                gpui_kit::component::list::ListItem::new(entry.item().id.clone())
                                    .child(entry.item().label.clone())
                            })),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .gap_2()
                            .child(header(&state))
                            .child(tab_strip(tabs, state.tab, cx))
                            .child(wizard(&state, cx))
                            .child(body(&state, &self.table)),
                    ),
            )
            .child(status(&state))
    }
}

fn toolbar(selection: xenadmin_ui::Selection, cx: &mut Context<ShellView>) -> impl IntoElement {
    let this = cx.entity();
    let start_ok = matches!(
        eligibility(CommandId("vm.start"), selection),
        Eligibility::Enabled
    );
    let mut bar = h_flex().child(probe!("toolbar")).gap_2().child(
        Button::new("cmd-add-server").label("Add Server").on_click({
            let this = this.clone();
            move |_, _, cx| {
                this.update(cx, |shell, cx| {
                    shell.state.dialog = Some(DialogId::AddServer);
                    shell.state.dialog_text =
                        Some("Address, username, and password stay in this preview.".into());
                    shell.presented = false;
                    cx.notify();
                });
            }
        }),
    );
    bar = bar.child(Button::new("cmd-back").label("Back").on_click({
        let this = this.clone();
        move |_, _, cx| {
            this.update(cx, |shell, cx| {
                shell.state.back();
                cx.notify();
            });
        }
    }));
    bar = bar.child(Button::new("cmd-forward").label("Forward").on_click({
        let this = this.clone();
        move |_, _, cx| {
            this.update(cx, |shell, cx| {
                shell.state.forward();
                cx.notify();
            });
        }
    }));
    let shutdown_ok = matches!(
        eligibility(CommandId("vm.shutdown"), selection),
        Eligibility::Enabled
    );
    let migrate_ok = matches!(
        eligibility(CommandId("vm.migrate"), selection),
        Eligibility::Enabled
    );
    let maintenance_ok = matches!(
        eligibility(CommandId("server.maintenance"), selection),
        Eligibility::Enabled
    );
    if shutdown_ok {
        bar = bar.child(review_button(
            this.clone(),
            "cmd-shutdown",
            "Shut Down",
            "Shut Down",
            "Shut down this guest? This preview does not call a host.",
        ));
    }
    if migrate_ok {
        bar = bar.child(Button::new("cmd-migrate").label("Migrate").on_click({
            let this = this.clone();
            move |_, _, cx| {
                this.update(cx, |shell, cx| {
                    shell.state.wizard_step = 1;
                    cx.notify();
                });
            }
        }));
    }
    if maintenance_ok {
        bar = bar.child(review_button(
            this.clone(),
            "cmd-maintenance",
            "Maintenance",
            "Maintenance",
            "This preview does not evacuate guests or call a host.",
        ));
    }
    if start_ok {
        bar = bar.child(
            Button::new("cmd-start")
                .label("Start")
                .on_click(move |_, window, cx| {
                    this.update(cx, |shell, cx| {
                        let name = shell.state.selected_name();
                        window.open_dialog(cx, move |dialog, _, _| {
                            dialog.title("Start").child(
                                probe!("power-review").child(format!(
                                    "Start {name}? This preview does not call a host."
                                )),
                            )
                        });
                        cx.notify();
                    });
                }),
        );
    }
    bar
}

fn header(state: &ShellState) -> impl IntoElement {
    div().id("object-header").child(state.selected_name())
}

fn tab_strip(
    tabs: &'static [TabId],
    active: TabId,
    cx: &mut Context<ShellView>,
) -> impl IntoElement {
    let this = cx.entity();
    let mut bar = TabBar::new("tabs");
    for tab in tabs {
        let label = tab_label(*tab);
        let id = *tab;
        let this = this.clone();
        bar = bar.child(Tab::new().label(label).on_click(move |_, _, cx| {
            this.update(cx, |shell, cx| {
                shell.state.set_tab(id);
                cx.notify();
            });
        }));
    }
    bar.selected_index(tabs.iter().position(|tab| *tab == active).unwrap_or(0))
}

fn tab_label(tab: TabId) -> &'static str {
    match tab {
        TabId::Home => "Home",
        TabId::General => "General",
        TabId::Memory => "Memory",
        TabId::Storage => "Storage",
        TabId::Networking => "Networking",
        TabId::Nics => "NICs",
        TabId::Ha => "HA",
        TabId::Users => "Users",
        TabId::Console => "Console",
        TabId::Performance => "Performance",
        TabId::Snapshots => "Snapshots",
        TabId::Search => "Search",
    }
}

fn body(state: &ShellState, table: &Entity<TableState<Rows>>) -> impl IntoElement {
    match state.tab {
        TabId::Home => probe!("tab-home")
            .child("Add a server from the toolbar. Xen Orchestra is not required."),
        TabId::Search => probe!("tab-search")
            .h(px(280.))
            .child(DataTable::new(table)),
        TabId::General => probe!("tab-general").child(general_text(state)),
        TabId::Console => probe!("console-canvas").child(
            "Console preview. Physical keys stay on route A. The guest cursor is not drawn. Pointer, frames, clipboard, and fullscreen need a focused seat.",
        ),
        TabId::Snapshots => probe!("tab-snapshots").child(snapshot_text(state)),
        TabId::Performance => probe!("tab-performance")
            .child("Performance charts are not sampled in this preview."),
        _ => probe!("tab-body").child(format!("{} is not ported yet.", tab_label(state.tab))),
    }
}

fn general_text(state: &ShellState) -> String {
    match state.kind {
        ObjectKind::Vm => {
            let vm = state.vm.and_then(|id| state.inventory.vms.get(&id));
            match vm {
                Some(vm) => format!("{} is {}. {}", vm.name, vm.power_state, vm.description),
                None => "No virtual machine selected.".into(),
            }
        }
        ObjectKind::Host => format!("Server {}", state.selected_name()),
        ObjectKind::Pool => format!("Pool {}", state.inventory.pool_name),
        _ => "Nothing selected.".into(),
    }
}

fn status(state: &ShellState) -> impl IntoElement {
    let version = state
        .inventory
        .hosts
        .values()
        .find(|host| host.version != "8.3" && !host.version.starts_with("8.3."))
        .map(|host| {
            format!(
                "Host version {} is outside the qualified 8.3 line.",
                host.version
            )
        });
    probe!("status-bar")
        .child(
            state
                .notice
                .clone()
                .unwrap_or_else(|| "Not connected to a pool.".into()),
        )
        .children(state.notice.as_ref().map(|_| probe!("connection-notice")))
        .children(version.clone())
        .children(version.map(|_| probe!("host-version-notice")))
        .child(probe!("activity-counters").child("No operations in this preview."))
}

fn snapshot_text(state: &ShellState) -> String {
    let names = state.snapshot_names();
    if names.is_empty() {
        "This guest has no snapshots.".into()
    } else {
        format!(
            "Snapshots: {}. Create, restore, and delete do not call a host.",
            names.join(", ")
        )
    }
}

fn wizard(state: &ShellState, cx: &mut Context<ShellView>) -> impl IntoElement {
    let step = state.wizard_step;
    let this = cx.entity();
    probe!("migrate-wizard")
        .children((step > 0).then(|| {
            div().child(if step == 1 {
                "Choose a server in this pool. Nothing is moved."
            } else {
                "Review the move. This preview does not migrate the guest."
            })
        }))
        .children((step == 1).then(|| {
            Button::new("migrate-next")
                .label("Next")
                .on_click(move |_, _, cx| {
                    this.update(cx, |shell, cx| {
                        shell.state.wizard_step = 2;
                        cx.notify();
                    });
                })
        }))
        .children((step >= 2).then(|| probe!("migrate-review")))
}

fn review_button(
    this: Entity<ShellView>,
    id: &'static str,
    label: &'static str,
    title: &'static str,
    text: &'static str,
) -> impl IntoElement {
    Button::new(id).label(label).on_click(move |_, _, cx| {
        this.update(cx, |shell, cx| {
            shell.state.dialog = Some(DialogId::Power);
            shell.state.dialog_title = title.into();
            shell.state.dialog_text = Some(text.into());
            shell.presented = false;
            cx.notify();
        });
    })
}

fn open_text_dialog(
    window: &mut Window,
    cx: &mut App,
    title: String,
    text: String,
    add_server: bool,
) {
    let title: SharedString = title.into();
    let text: SharedString = text.into();
    window.open_dialog(cx, move |dialog, _, _| {
        let body = if add_server {
            probe!("add-server").child(text.clone())
        } else {
            probe!("review-dialog").child(text.clone())
        };
        dialog.title(title.clone()).child(body)
    });
}
