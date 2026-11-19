//! The shell, driven through real events on the Kit's headless test
//! platform: the menu bar, the toolbar, the navigation pane, the tabs, the
//! status bar and the intents, each through what a user does.
//!
//! No area is in the registry of this crate's tests beyond the kernel's
//! commands, so the commands that open a surface come from a `Describe`
//! override over the renderer's lab (`xenadmin_desktop::scenes::gallery`): the
//! shell reads menus and runs commands only through the store, which is
//! what lets it work with whatever the registry holds.
//!
//! These tests do not establish Wayland input, the desktop's file chooser,
//! the system clipboard or a screen reader: the platform is the Kit's test
//! platform.
#![cfg(feature = "scenes")]

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use xenadmin_desktop::intents::{self, Hooks};
use xenadmin_desktop::kit::testing::{
    ElementSnapshot, TestAppContext, TestAppContextExt as _, TestEventExt as _, TestQueryExt as _,
    TestWindowExt as _,
    VisualTestContext,
};
use xenadmin_desktop::kit::*;
use xenadmin_desktop::scenes::gallery::{DIALOG, Lab, LabRef, TABLE};
use xenadmin_desktop::scenes::{self, Options, Setup, ShellWindow};
use xenadmin_desktop::shell::{self, nav as pane, status, toolbar};
use xenadmin_desktop::store::Describe;
use xenadmin_ui::command::{self, CommandId, Menu, MenuEntry, MenuItem};
use xenadmin_ui::intent::{FileRequest, Intent};
use xenadmin_ui::nav::{self, NavRow, NavView};
use xenadmin_ui::object::ObjectKind;
use xenadmin_ui::tabs::TabId;
use xenadmin_ui::{Model, scene};

/// The commands the lab adds to what the registry holds.
const OPEN_DIALOG: &str = "lab.open-dialog";
const BUSY: &str = "lab.busy";
const ADD_SERVER: &str = "lab.add-server";
const START: &str = "lab.start";
const BUSY_REASON: &str = "The lab is busy.";

fn id(key: &str) -> ElementId {
    ElementId::Name(SharedString::from(key.to_owned()))
}

fn item(id: &'static str, label: &str, disabled: Option<&str>) -> MenuItem {
    MenuItem {
        id: CommandId(id),
        label: label.to_owned(),
        shortcut: None,
        disabled: disabled.map(str::to_owned),
        checked: None,
    }
}

/// The lab's descriptions, with a Tools menu group, two toolbar groups and
/// the commands behind them.
fn describe(lab: &LabRef) -> Describe {
    let invoked = lab.clone();
    let fallback = Describe::default().invoke;
    Describe {
        menu_bar: Rc::new(|m| {
            let mut menus = command::menu_bar(m);
            let tools = menus
                .iter_mut()
                .find(|view| view.menu == Menu::Tools)
                .expect("the Tools menu");
            tools.groups.push(vec![
                MenuEntry::Item(item(OPEN_DIALOG, "Open Lab Dialog", None)),
                MenuEntry::Item(item(BUSY, "Busy Command", Some(BUSY_REASON))),
            ]);
            menus
        }),
        toolbar: Rc::new(|m| {
            let mut groups = command::toolbar(m);
            groups.push(vec![
                item(ADD_SERVER, "Add Server", None),
                item(BUSY, "Busy Command", Some(BUSY_REASON)),
            ]);
            // A power command is in the toolbar only while it applies.
            if m.scope().kind() == ObjectKind::Vm {
                groups.push(vec![item(START, "Start", None)]);
            }
            groups
        }),
        invoke: Rc::new(move |m, command| match command.0 {
            OPEN_DIALOG => invoked.borrow_mut().open.push(DIALOG),
            ADD_SERVER | START | BUSY => invoked.borrow_mut().log.push(format!("run:{command}")),
            _ => fallback(m, command),
        }),
        ..Lab::describe(lab)
    }
}

/// The whole window over a demo model and the lab, with the lab's commands.
fn open_sized(
    cx: &mut TestAppContext,
    width: f32,
    height: f32,
    prepare: impl FnOnce(&mut Setup),
) -> ShellWindow {
    let mut setup = Setup::lab();
    prepare(&mut setup);
    let options = Options {
        width,
        height,
        ..Options::default()
    };
    let scene = cx
        .update(|cx| scenes::open_shell(cx, setup, &options))
        .expect("shell window");
    if let Some(lab) = scene.lab.clone() {
        cx.update(|cx| {
            scene
                .store
                .update(cx, |store, cx| store.set_describe(describe(&lab), cx));
        });
    }
    cx.run_until_parked();
    settle(cx, &scene);
    scene
}

fn open(cx: &mut TestAppContext) -> ShellWindow {
    open_sized(cx, 1440., 900., |_| {})
}

/// Draws until what the last event caused is on screen.
fn settle(cx: &mut TestAppContext, scene: &ShellWindow) {
    for _ in 0..3 {
        cx.update_window(scene.window, |_, window, cx| window.render_frame(cx))
            .expect("window");
        cx.run_until_parked();
    }
}

/// Runs `act` in the window, then settles.
fn on<R>(
    cx: &mut TestAppContext,
    scene: &ShellWindow,
    act: impl FnOnce(&mut Window, &mut App) -> R,
) -> R {
    let result = cx
        .update_window(scene.window, |_, window, cx| act(window, cx))
        .expect("window");
    cx.run_until_parked();
    settle(cx, scene);
    result
}

/// Reads the model.
fn model<R>(cx: &mut TestAppContext, scene: &ShellWindow, read: impl FnOnce(&Model) -> R) -> R {
    cx.update(|cx| read(scene.store.read(cx).model()))
}

/// A change made by the model, not by the user.
fn model_side(cx: &mut TestAppContext, scene: &ShellWindow, change: impl FnOnce(&mut Model)) {
    cx.update(|cx| scene.store.update(cx, |store, cx| store.run(change, cx)));
    cx.run_until_parked();
    settle(cx, scene);
}

fn log(scene: &ShellWindow) -> Vec<String> {
    scene.lab.as_ref().expect("lab").borrow().log.clone()
}

/// The row of the active navigation view with this label.
fn nav_row(cx: &mut TestAppContext, scene: &ShellWindow, label: &str) -> NavRow {
    model(cx, scene, |m| {
        nav::rows(m)
            .into_iter()
            .find(|row| row.label == label)
            .unwrap_or_else(|| panic!("no navigation row is labelled {label}"))
    })
}

/// The id of the row the tree's cursor is on.
fn cursor(cx: &mut TestAppContext, scene: &ShellWindow) -> Option<String> {
    cx.update(|cx| {
        let pane = scene.view.read(cx).nav().clone();
        pane.read(cx).cursor().map(str::to_owned)
    })
}

/// Presses Tab until `reached` holds, at most once around the window.
fn tab_to(
    cx: &mut TestAppContext,
    scene: &ShellWindow,
    what: &str,
    reached: impl Fn(&mut Window, &mut App) -> bool,
) {
    for _ in 0..120 {
        if on(cx, scene, |window, cx| reached(window, cx)) {
            return;
        }
        on(cx, scene, |window, cx| window.press("tab", cx));
    }
    panic!("Tab never reached {what}");
}

/// The row of the tree with this label. The same name is also in the
/// title bar, the header and the status bar.
fn tree_row(window: &mut Window, label: &str) -> ElementSnapshot {
    window.within(pane::TREE_KEY).find_by_label(label)
}

fn try_tree_row(window: &mut Window, label: &str) -> Option<ElementSnapshot> {
    window.within(pane::TREE_KEY).try_find_by_label(label)
}

fn focused(window: &mut Window, key: &str) -> bool {
    window
        .try_find(id(key))
        .is_some_and(|element| element.focused() == Some(true))
}

/// The name, disabled and checked state of every item of a menu of the
/// application menus (a disabled item is not exposed as such by the Kit's
/// popup).
fn menu_flags(cx: &mut TestAppContext, menu: &str) -> Vec<(String, bool, bool)> {
    cx.update(|cx| {
        let menus = GlobalState::global(cx).app_menus().to_vec();
        let menu = menus
            .iter()
            .find(|owned| owned.name.as_ref() == menu)
            .unwrap_or_else(|| panic!("no menu is named {menu}"));
        menu.items
            .iter()
            .filter_map(|item| match item {
                OwnedMenuItem::Action {
                    name,
                    disabled,
                    checked,
                    ..
                } => Some((name.clone(), *disabled, *checked)),
                _ => None,
            })
            .collect()
    })
}

/// Opens a menu of the menu bar with the pointer.
fn open_menu(cx: &mut TestAppContext, scene: &ShellWindow, menu: Menu) {
    let index = Menu::ALL
        .iter()
        .position(|other| *other == menu)
        .expect("menu");
    on(cx, scene, |window, cx| {
        window
            .within("app-menu-bar")
            .within(index)
            .click("menu", cx);
    });
}

/// Chooses the item with this name from the open menu.
fn choose(cx: &mut TestAppContext, scene: &ShellWindow, name: &str) {
    on(cx, scene, |window, cx| {
        // An item's id is its place in the menu.
        let item = window
            .find_by_label(name)
            .path()
            .last()
            .cloned()
            .expect("the id of the menu item");
        window.within("popup-menu").click(item, cx);
    });
}

#[gpui_kit::test]
fn the_window_has_its_regions_from_the_model(cx: &mut TestAppContext) {
    let scene = open(cx);
    on(cx, &scene, |window, _| {
        let viewport = window.viewport_size();
        let root = window.find(shell::ROOT_KEY).bounds();
        assert_eq!(root.size, viewport);
        // Top to bottom: title bar with the menus, toolbar, the split,
        // status bar.
        let title_bar = window.find("title-bar").bounds();
        let bar = window.find(toolbar::TOOLBAR_KEY).bounds();
        let side = window.find(pane::PANE_KEY).bounds();
        let content = window.find(shell::CONTENT_KEY).bounds();
        let bottom = window.find(status::STATUS_KEY).bounds();
        assert_eq!(title_bar.top(), px(0.));
        assert!(title_bar.bottom() <= bar.top());
        assert!(bar.bottom() <= side.top());
        assert!(side.bottom() <= bottom.top());
        assert_eq!(bottom.bottom(), viewport.height);
        // The pane is left of the content and as wide as the preferences
        // say.
        assert_eq!(side.left(), px(0.));
        assert_eq!(side.size.width, px(260.));
        assert!(side.right() <= content.left());
        assert_eq!(content.right(), viewport.width);
        // Inside the pane: search box, tree, view switcher.
        let search = window.find(pane::SEARCH_KEY).bounds();
        let tree = window.find(pane::TREE_KEY).bounds();
        let switcher = window.find(pane::SWITCHER_KEY).bounds();
        assert!(search.bottom() <= tree.top());
        assert!(tree.bottom() <= switcher.top());
        assert_eq!(
            window.find(pane::SEARCH_KEY).placeholder(),
            Some(pane::SEARCH_PLACEHOLDER)
        );
        assert_eq!(window.find(pane::TREE_KEY).label(), Some(pane::TREE_LABEL));
        // The title, the status line and the demo banner say what the
        // model says.
        assert_eq!(
            window.find(shell::TITLE_KEY).label(),
            Some("Demonstration pool")
        );
        assert_eq!(
            window.find(status::CONNECTION_KEY).label(),
            Some("Demo inventory • no host connection")
        );
        assert_eq!(
            window.find(status::OPERATIONS_KEY).label(),
            Some("0 operations")
        );
        assert_eq!(
            window.find(status::UNKNOWN_KEY).label(),
            Some("0 unknown outcomes")
        );
        assert_eq!(
            window.find(status::SELECTION_KEY).label(),
            Some("Demonstration pool")
        );
        assert_eq!(
            window.find(status::DEMO_KEY).label(),
            Some(xenadmin_ui::model::DEMO_BANNER)
        );
        // No toggle in a wide window.
        assert!(window.try_find(shell::TOGGLE_KEY).is_none());
        // The five views, the active one marked.
        for view in NavView::ALL {
            let entry = window.find(view.key());
            assert_eq!(entry.label(), Some(view.title()));
            assert_eq!(
                entry.selected(),
                Some(view == NavView::Infrastructure),
                "{}",
                view.title()
            );
        }
    });
    let title = cx.update(|cx| scene.view.read(cx).title().to_owned());
    assert_eq!(title, "Demonstration pool");
    assert_eq!(
        shell::window_title(&title),
        "Demonstration pool - XenAdmin RS"
    );
    assert_eq!(shell::window_title("XenAdmin RS"), "XenAdmin RS");
}

/// Gate G2's review item, for the shell: an action is a command in the
/// menu bar, the toolbar or a tab's local action row, or a button of a
/// dialog. Every button the shell draws is in the menu bar or the toolbar,
/// or is the pane toggle, which shows and hides the pane and acts on
/// nothing. (What is inside the tab body is the description of an area:
/// its local action row and, here, the sort buttons of the lab's grid.)
#[gpui_kit::test]
fn no_region_has_an_action_button_of_its_own(cx: &mut TestAppContext) {
    for (width, height) in [(1440., 900.), (900., 700.)] {
        let scene = open_sized(cx, width, height, |_| {});
        let stray: Vec<String> = on(cx, &scene, |window, _| {
            window
                .find_all_by_role(Role::Button)
                .into_iter()
                .filter(|button| {
                    let path: Vec<String> = scenes::id_path(button);
                    let within = |key: &str| path.iter().any(|segment| segment == key);
                    !(within("app-menu-bar")
                        || within(toolbar::TOOLBAR_KEY)
                        || within(xenadmin_desktop::scene_view::TAB_BODY_KEY)
                        || within(shell::TOGGLE_KEY))
                })
                .map(|button| format!("{:?} at {:?}", button.label(), scenes::id_path(&button)))
                .collect()
        });
        assert!(stray.is_empty(), "buttons outside their places: {stray:#?}");
        // And the buttons that are there are the commands of the store.
        let toolbar_ids: Vec<String> = cx.update(|cx| {
            scene
                .store
                .read(cx)
                .toolbar()
                .into_iter()
                .flatten()
                .map(|item| item.id.0.to_owned())
                .collect()
        });
        on(cx, &scene, |window, _| {
            let bar = window.within(toolbar::TOOLBAR_KEY);
            for id in &toolbar_ids {
                assert_eq!(bar.find(self::id(id)).role(), Some(Role::Button), "{id}");
            }
        });
    }
}

#[gpui_kit::test]
fn the_tree_shows_the_rows_of_the_model_with_their_state(cx: &mut TestAppContext) {
    let scene = open(cx);
    let pool = nav_row(cx, &scene, "Demonstration pool");
    let host = nav_row(cx, &scene, "xcp-lab-01");
    let rows = model(cx, &scene, nav::rows);
    let guest = rows
        .iter()
        .find(|row| row.parent.as_deref() == Some(host.id.as_str()))
        .cloned()
        .expect("a guest on the first host");
    on(cx, &scene, |window, _| {
        // The row of the scope is the selected one; a folder says whether
        // it is open.
        let row = tree_row(window, "Demonstration pool");
        assert_eq!(row.role(), Some(Role::TreeItem));
        assert_eq!(row.selected(), Some(true));
        assert_eq!(row.expanded(), Some(true));
        let host_row = tree_row(window, "xcp-lab-01");
        assert_eq!(host_row.selected(), Some(false));
        assert_eq!(host_row.expanded(), Some(true));
        // Indentation follows the depth.
        let (pool_x, host_x, guest_x) = (
            window.find(id(&pool.id)).bounds(),
            window.find(id(&host.id)).bounds(),
            window.find(id(&guest.id)).bounds(),
        );
        assert_eq!(pool_x.left(), host_x.left());
        assert_eq!(pool_x.size.height, guest_x.size.height);
        // The label of a guest starts further right than its host's, which
        // starts further right than the pool's.
        let icon = |row: &ElementSnapshot| row.bounds().left();
        let (pool_row, host_row, guest_row) = (
            tree_row(window, "Demonstration pool"),
            tree_row(window, "xcp-lab-01"),
            tree_row(window, &guest.label),
        );
        assert_eq!(icon(&pool_row), icon(&host_row));
        assert_eq!(icon(&host_row), icon(&guest_row));
    });
    // Folding the host with its chevron hides its guests and keeps the
    // selection where it is.
    let before = cx.update(|cx| {
        let pane = scene.view.read(cx).nav().clone();
        pane.read(cx).visible_rows()
    });
    assert!(before.contains(&guest.id));
    let host_index = before
        .iter()
        .position(|row| *row == host.id)
        .expect("the host row");
    on(cx, &scene, |window, cx| {
        window.click(
            ElementId::NamedInteger(pane::FOLD_KEY.into(), host_index as u64),
            cx,
        );
    });
    let after = cx.update(|cx| {
        let pane = scene.view.read(cx).nav().clone();
        pane.read(cx).visible_rows()
    });
    assert!(!after.contains(&guest.id));
    assert!(after.contains(&host.id));
    assert_eq!(cursor(cx, &scene), Some(pool.id.clone()));
    assert_eq!(model(cx, &scene, |m| m.scope().kind()), ObjectKind::Pool);
    on(cx, &scene, |window, _| {
        assert_eq!(tree_row(window, "xcp-lab-01").expanded(), Some(false));
        assert!(window.try_find(id(&guest.id)).is_none());
    });
    // A press on the row selects it and does not unfold it.
    on(cx, &scene, |window, cx| window.click(id(&host.id), cx));
    assert_eq!(model(cx, &scene, |m| m.scope().kind()), ObjectKind::Host);
    assert_eq!(cursor(cx, &scene), Some(host.id.clone()));
    on(cx, &scene, |window, _| {
        assert_eq!(tree_row(window, "xcp-lab-01").expanded(), Some(false));
        assert_eq!(tree_row(window, "xcp-lab-01").selected(), Some(true));
        assert_eq!(window.find(shell::TITLE_KEY).label(), Some("xcp-lab-01"));
    });
}

#[gpui_kit::test]
async fn the_keyboard_alone_reaches_the_tree_the_tabs_the_grid_and_a_menu_command(
    cx: &mut TestAppContext,
) {
    let scene = open(cx);
    let host = nav_row(cx, &scene, "xcp-lab-01");
    let pool = nav_row(cx, &scene, "Demonstration pool");

    // The tree.
    tab_to(cx, &scene, "the tree", |window, _| {
        focused(window, pane::TREE_KEY)
    });
    on(cx, &scene, |window, cx| window.press("down", cx));
    assert_eq!(cursor(cx, &scene), Some(host.id.clone()));
    assert_eq!(model(cx, &scene, |m| m.scope().kind()), ObjectKind::Host);
    on(cx, &scene, |window, cx| window.press("down", cx));
    assert_eq!(model(cx, &scene, |m| m.scope().kind()), ObjectKind::Vm);
    // Left on a guest goes to its host; left again folds the host; right
    // unfolds it; Home goes to the first row.
    on(cx, &scene, |window, cx| window.press("left", cx));
    assert_eq!(cursor(cx, &scene), Some(host.id.clone()));
    on(cx, &scene, |window, cx| window.press("left", cx));
    on(cx, &scene, |window, _| {
        assert_eq!(tree_row(window, "xcp-lab-01").expanded(), Some(false));
    });
    on(cx, &scene, |window, cx| window.press("right", cx));
    on(cx, &scene, |window, _| {
        assert_eq!(tree_row(window, "xcp-lab-01").expanded(), Some(true));
    });
    on(cx, &scene, |window, cx| window.press("home", cx));
    assert_eq!(cursor(cx, &scene), Some(pool.id.clone()));
    assert_eq!(model(cx, &scene, |m| m.scope().kind()), ObjectKind::Pool);
    on(cx, &scene, |window, _| {
        assert!(focused(window, pane::TREE_KEY), "the keys stay in the tree");
    });

    // The tab strip.
    tab_to(cx, &scene, "the General tab", |window, _| {
        focused(window, TabId::General.key())
    });
    on(cx, &scene, |window, cx| window.press("tab", cx));
    on(cx, &scene, |window, cx| {
        assert!(focused(window, TabId::Memory.key()));
        window.press("enter", cx);
    });
    assert_eq!(model(cx, &scene, Model::active_tab), TabId::Memory);

    // The grid of the tab: Tab until an arrow key moves its selection.
    let lab = scene.lab.clone().expect("lab");
    assert_eq!(lab.borrow().selected, None);
    let mut reached = false;
    for _ in 0..40 {
        on(cx, &scene, |window, cx| window.press("tab", cx));
        let in_grid = on(cx, &scene, |window, _| {
            window
                .within(TABLE)
                .try_find("table")
                .is_some_and(|table| table.focused() == Some(true))
        });
        if in_grid {
            reached = true;
            break;
        }
    }
    assert!(reached, "Tab never reached the grid");
    on(cx, &scene, |window, cx| window.press("down", cx));
    let first = lab.borrow().selected.clone();
    assert!(first.is_some(), "an arrow key selects a row of the grid");
    on(cx, &scene, |window, cx| window.press("down", cx));
    let second = lab.borrow().selected.clone();
    assert!(second.is_some() && second != first);
    // The grid selection is the target, not the scope.
    assert_eq!(model(cx, &scene, |m| m.scope().kind()), ObjectKind::Pool);
    assert_eq!(model(cx, &scene, |m| m.target().kind()), ObjectKind::Vm);

    // A menu command: Tab on to the View menu, open it, go down to Toolbar
    // and choose it.
    let view = Menu::ALL
        .iter()
        .position(|menu| *menu == Menu::View)
        .expect("the View menu");
    tab_to(cx, &scene, "the View menu", move |window, _| {
        window
            .within("app-menu-bar")
            .within(view)
            .try_find("menu")
            .is_some_and(|button| button.focused() == Some(true))
    });
    assert!(model(cx, &scene, |m| m.nav.show_toolbar));
    on(cx, &scene, |window, cx| window.press("enter", cx));
    on(cx, &scene, |window, _| {
        assert_eq!(window.find("popup-menu").role(), Some(Role::Menu));
    });
    let toolbar_item = on(cx, &scene, |window, _| {
        window
            .within("popup-menu")
            .find_by_label("Toolbar")
            .bounds()
    });
    let mut chosen = false;
    for _ in 0..12 {
        on(cx, &scene, |window, cx| window.press("down", cx));
        let highlighted = on(cx, &scene, |window, _| {
            window
                .find_all_by_role(Role::MenuItem)
                .into_iter()
                .any(|item| item.selected() == Some(true) && item.bounds() == toolbar_item)
        });
        if highlighted {
            on(cx, &scene, |window, cx| window.press("enter", cx));
            chosen = true;
            break;
        }
    }
    assert!(chosen, "the arrow keys never reached Toolbar");
    cx.wait_for(scene.window, Duration::from_secs(2), |window, _| {
        window.try_find("popup-menu").is_none()
    })
    .await;
    settle(cx, &scene);
    assert!(!model(cx, &scene, |m| m.nav.show_toolbar));
    on(cx, &scene, |window, _| {
        assert!(window.try_find(toolbar::TOOLBAR_KEY).is_none());
    });
    // The toggle shows its new state in the rebuilt menu.
    assert!(menu_flags(cx, "View").contains(&("Toolbar".into(), false, false)));
}

#[gpui_kit::test]
fn the_selection_stays_on_its_object_across_an_inventory_update(cx: &mut TestAppContext) {
    let scene = open(cx);
    let database = nav_row(cx, &scene, "Database");
    on(cx, &scene, |window, cx| window.click(id(&database.id), cx));
    let selected = model(cx, &scene, |m| m.scope().single().cloned()).expect("a selection");
    assert_eq!(selected.kind, ObjectKind::Vm);
    assert_eq!(Some(&selected), database.object.as_ref());
    assert_eq!(cursor(cx, &scene), Some(database.id.clone()));
    let folded = on(cx, &scene, |window, cx| {
        // Fold the first host, so expansion is something to keep too.
        let rows = window
            .find_all_by_role(Role::TreeItem)
            .into_iter()
            .position(|row| row.label() == Some("xcp-lab-01"))
            .expect("the host row");
        window.click(
            ElementId::NamedInteger(pane::FOLD_KEY.into(), rows as u64),
            cx,
        );
        rows
    });
    assert_eq!(folded, 1);
    // The inventory changes under the selection: another guest is renamed
    // so that it sorts first on the selected guest's host, and one goes.
    model_side(cx, &scene, |m| {
        let mut inventory = m.inventory.clone().expect("inventory").as_ref().clone();
        let database = selected.vm_id().expect("a VM");
        let home = inventory.vms[&database].host;
        let mut others: Vec<_> = inventory
            .vms
            .iter()
            .filter(|(id, vm)| **id != database && vm.host == home && !vm.is_template)
            .map(|(id, vm)| (vm.name.clone(), *id))
            .collect();
        others.sort();
        let (_, renamed) = others.last().cloned().expect("another guest on the host");
        let (_, removed) = others.first().cloned().expect("another guest on the host");
        inventory.vms.get_mut(&renamed).expect("guest").name = "AAA first".into();
        inventory.vms.remove(&removed);
        m.inventory = Some(std::sync::Arc::new(inventory));
        m.poll(m.clock());
    });
    // Same object, same row id, still marked, and the fold is kept.
    assert_eq!(
        model(cx, &scene, |m| m.scope().single().cloned()),
        Some(selected.clone())
    );
    assert_eq!(cursor(cx, &scene), Some(database.id.clone()));
    on(cx, &scene, |window, _| {
        let row = tree_row(window, "Database");
        assert_eq!(row.selected(), Some(true));
        assert_eq!(tree_row(window, "xcp-lab-01").expanded(), Some(false));
        assert!(try_tree_row(window, "AAA first").is_some());
        assert_eq!(window.find(status::SELECTION_KEY).label(), Some("Database"));
        // The rows above it changed by one going and none coming before
        // it on a folded host, so the row itself stays where it was or
        // moves by whole rows: it is still on screen.
        assert!(window.find(id(&database.id)).visible());
    });

    // The object goes: the selection moves to its parent and the status
    // bar says why.
    model_side(cx, &scene, |m| {
        let mut inventory = m.inventory.clone().expect("inventory").as_ref().clone();
        inventory.vms.remove(&selected.vm_id().expect("a VM"));
        m.inventory = Some(std::sync::Arc::new(inventory));
        m.poll(m.clock());
    });
    let scope = model(cx, &scene, |m| m.scope().single().cloned()).expect("a selection");
    assert_ne!(scope, selected);
    let notice = model(cx, &scene, |m| m.notice.clone()).expect("a notice");
    assert!(notice.contains("Database"), "{notice}");
    on(cx, &scene, |window, _| {
        assert_eq!(
            window.find(status::NOTICE_KEY).label(),
            Some(notice.as_str())
        );
        assert!(window.try_find(id(&database.id)).is_none());
    });
}

#[gpui_kit::test]
async fn a_menu_command_opens_its_surface(cx: &mut TestAppContext) {
    let scene = open(cx);
    assert!(
        menu_flags(cx, "Tools").contains(&("Open Lab Dialog".into(), false, false)),
        "{:?}",
        menu_flags(cx, "Tools")
    );
    on(cx, &scene, |window, _| {
        assert!(window.try_find(DIALOG).is_none());
        // The bar has the ten menus by name.
        for (index, menu) in Menu::ALL.iter().enumerate() {
            let button = window.within("app-menu-bar").within(index).find("menu");
            assert_eq!(button.label(), Some(menu.title()));
        }
    });
    open_menu(cx, &scene, Menu::Tools);
    choose(cx, &scene, "Open Lab Dialog");
    cx.wait_for(scene.window, Duration::from_secs(2), |window, _| {
        window.try_find("popup-menu").is_none()
    })
    .await;
    settle(cx, &scene);
    assert_eq!(scene.lab.as_ref().expect("lab").borrow().open, [DIALOG]);
    on(cx, &scene, |window, _| {
        assert!(window.find(DIALOG).visible());
    });
    let presented = cx.update(|cx| {
        let dialogs = scene.view.read(cx).dialogs().clone();
        dialogs.read(cx).presented().to_vec()
    });
    assert_eq!(presented, [DIALOG]);
    // While the dialog has the keyboard no command shortcut reaches the
    // shell: Alt+Left would go back.
    on(cx, &scene, |window, cx| window.press("ctrl-q", cx));
    let quits = cx.update(|cx| {
        scene
            .store
            .read(cx)
            .recorded_intents()
            .iter()
            .filter(|intent| matches!(intent, Intent::Quit))
            .count()
    });
    assert_eq!(quits, 0);
}

#[gpui_kit::test]
async fn a_disabled_command_cannot_run_and_shows_its_reason(cx: &mut TestAppContext) {
    let scene = open(cx);
    // In the menu: the reason is in the item's name, and the item is
    // disabled.
    let busy = format!("Busy Command — {BUSY_REASON}");
    assert!(menu_flags(cx, "Tools").contains(&(busy.clone(), true, false)));
    let reset = "Reset Interface Size — The interface is at its normal size.";
    assert!(menu_flags(cx, "View").contains(&(reset.to_owned(), true, false)));
    open_menu(cx, &scene, Menu::Tools);
    choose(cx, &scene, &busy);
    settle(cx, &scene);
    assert!(log(&scene).is_empty(), "a disabled item does not run");
    on(cx, &scene, |window, cx| window.press("escape", cx));
    cx.wait_for(scene.window, Duration::from_secs(2), |window, _| {
        window.try_find("popup-menu").is_none()
    })
    .await;
    settle(cx, &scene);

    // In the toolbar: greyed, with the reason as its description; a press
    // does nothing.
    on(cx, &scene, |window, cx| {
        let mut bar = window.within(toolbar::TOOLBAR_KEY);
        let button = bar.find(BUSY);
        assert_eq!(button.disabled(), Some(true));
        assert_eq!(button.description(), Some(BUSY_REASON));
        let back = bar.find("nav.back");
        assert_eq!(back.disabled(), Some(true));
        assert_eq!(back.label(), Some("Back"));
        assert_eq!(
            back.description(),
            Some("Back — There is no earlier selection to go back to.")
        );
        bar.click(BUSY, cx);
        bar.click("nav.back", cx);
    });
    assert!(log(&scene).is_empty());
    // An enabled one runs.
    on(cx, &scene, |window, cx| {
        window.within(toolbar::TOOLBAR_KEY).click(ADD_SERVER, cx);
    });
    assert_eq!(log(&scene), [format!("run:{ADD_SERVER}")]);

    // By its shortcut: the model refuses and the status bar says why.
    assert_eq!(model(cx, &scene, |m| m.error.clone()), None);
    on(cx, &scene, |window, cx| window.press("ctrl-0", cx));
    let reason = "The interface is at its normal size.";
    assert_eq!(
        model(cx, &scene, |m| m.error.clone()).as_deref(),
        Some(reason)
    );
    assert_eq!(
        model(cx, &scene, |m| m.preferences.options.interface_scale),
        100
    );
    on(cx, &scene, |window, _| {
        let line = window.find(status::ERROR_KEY);
        assert_eq!(line.role(), Some(Role::Alert));
        assert_eq!(line.label(), Some(reason));
    });
}

#[gpui_kit::test]
fn the_toolbar_shows_a_power_command_only_while_it_applies(cx: &mut TestAppContext) {
    let scene = open(cx);
    on(cx, &scene, |window, _| {
        let bar = window.within(toolbar::TOOLBAR_KEY);
        assert!(bar.try_find(START).is_none());
        assert_eq!(bar.find(ADD_SERVER).label(), Some("Add Server"));
    });
    let database = nav_row(cx, &scene, "Database");
    on(cx, &scene, |window, cx| window.click(id(&database.id), cx));
    on(cx, &scene, |window, cx| {
        let mut bar = window.within(toolbar::TOOLBAR_KEY);
        assert_eq!(bar.find(START).label(), Some("Start"));
        bar.click(START, cx);
    });
    assert_eq!(log(&scene), [format!("run:{START}")]);
    // The toolbar is a row of buttons from the command table and nothing
    // else: every button in it is one of its commands.
    let commands: Vec<String> = cx.update(|cx| {
        scene
            .store
            .read(cx)
            .toolbar()
            .into_iter()
            .flatten()
            .map(|item| item.id.0.to_owned())
            // The ported areas put their own commands in the toolbar (Add
            // Server of the connections area, and more as areas land); this
            // test follows the kernel's and its own.
            .filter(|id| id.starts_with("nav.") || id.starts_with("lab."))
            .collect()
    });
    assert_eq!(
        commands,
        ["nav.back", "nav.forward", ADD_SERVER, BUSY, START]
    );
}

/// Where an entry of the top level of the model's open context menu (an
/// item, or a submenu by its label) is in the popup the shell builds from
/// it: the entries in order, with one separator between groups.
fn popup_index(cx: &mut TestAppContext, scene: &ShellWindow, label: &str) -> usize {
    model(cx, scene, |m| {
        let mut index = 0;
        for (group, entries) in m.context_menu().into_iter().enumerate() {
            if group > 0 {
                index += 1;
            }
            for entry in entries {
                let found = match &entry {
                    MenuEntry::Item(item) => item.label == label,
                    MenuEntry::Submenu { label: name, .. } => *name == label,
                };
                if found {
                    return index;
                }
                index += 1;
            }
        }
        panic!("the top level of the context menu has no entry {label}")
    })
}

/// The items of a submenu of the model's open context menu.
fn submenu_items(cx: &mut TestAppContext, scene: &ShellWindow, label: &str) -> Vec<MenuItem> {
    model(cx, scene, |m| {
        m.context_menu()
            .into_iter()
            .flatten()
            .find(|entry| entry.submenu() == Some(label))
            .map(MenuEntry::into_items)
            .unwrap_or_else(|| panic!("the context menu has no submenu {label}"))
    })
}

#[gpui_kit::test]
async fn a_tree_row_has_the_context_menu_of_its_object(cx: &mut TestAppContext) {
    let scene = open(cx);
    let database = nav_row(cx, &scene, "Database");
    let object = database.object.clone().expect("the guest");
    // The secondary button on a row that is not the scope: the row is
    // selected and its menu opens.
    on(cx, &scene, |window, cx| {
        window.right_click(id(&database.id), cx)
    });
    assert_eq!(
        model(cx, &scene, |m| m.scope().single().cloned()),
        Some(object.clone())
    );
    assert!(model(cx, &scene, |m| m.selection.menu.is_some()));
    assert!(model(cx, &scene, Model::context_menu_valid));
    let items: Vec<String> = model(cx, &scene, |m| {
        m.context_items()
            .into_iter()
            .map(|item| item.label)
            .collect()
    });
    // The areas' commands for a guest are in it too; this test follows the
    // kernel's Copy UUID.
    assert!(items.iter().any(|label| label == "Copy UUID"), "{items:?}");
    let copy = popup_index(cx, &scene, "Copy UUID");
    on(cx, &scene, |window, cx| {
        let menu = window.find("popup-menu");
        assert_eq!(menu.role(), Some(Role::Menu));
        assert_eq!(
            window.find_by_label("Copy UUID").role(),
            Some(Role::MenuItem)
        );
        window.within("popup-menu").click(copy, cx);
    });
    cx.wait_for(scene.window, Duration::from_secs(2), |window, _| {
        window.try_find("popup-menu").is_none()
    })
    .await;
    settle(cx, &scene);
    // The command ran on the row's object, and the model's menu is over.
    let copied: Vec<String> = cx.update(|cx| {
        scene
            .store
            .read(cx)
            .recorded_intents()
            .iter()
            .filter_map(|intent| match intent {
                Intent::CopyText(text) => Some(text.clone()),
                _ => None,
            })
            .collect()
    });
    assert_eq!(copied, std::slice::from_ref(&object.id));
    assert!(model(cx, &scene, |m| m.selection.menu.is_none()));
    assert!(!cx.update(|cx| scene.view.read(cx).menu_open()));

    // By the keyboard: Shift+F10 on the cursor's row, Escape to leave.
    on(cx, &scene, |window, cx| {
        assert!(focused(window, pane::TREE_KEY));
        window.press("shift-f10", cx);
    });
    assert!(model(cx, &scene, |m| m.selection.menu.is_some()));
    on(cx, &scene, |window, cx| {
        assert!(window.find("popup-menu").visible());
        window.press("escape", cx);
    });
    cx.wait_for(scene.window, Duration::from_secs(2), |window, _| {
        window.try_find("popup-menu").is_none()
    })
    .await;
    settle(cx, &scene);
    assert!(model(cx, &scene, |m| m.selection.menu.is_none()));
    on(cx, &scene, |window, _| {
        assert!(
            focused(window, pane::TREE_KEY),
            "the keys are back in the tree"
        );
    });
    // A heading has no object and no menu.
    on(cx, &scene, |window, cx| {
        window.click(NavView::Objects.key(), cx)
    });
    let heading = nav_row(cx, &scene, "Servers");
    assert!(heading.object.is_none());
    on(cx, &scene, |window, cx| {
        window.right_click(id(&heading.id), cx)
    });
    assert!(model(cx, &scene, |m| m.selection.menu.is_none()));
    on(cx, &scene, |window, _| {
        assert!(window.try_find("popup-menu").is_none());
    });
}

#[gpui_kit::test]
async fn the_shell_shows_whatever_context_menu_the_model_has_open(cx: &mut TestAppContext) {
    use xenadmin_ui::selection::MenuOrigin;
    let scene = open(cx);
    let database = nav_row(cx, &scene, "Database");
    let object = database.object.clone().expect("the guest");
    let host = nav_row(cx, &scene, "xcp-lab-01");
    let shown = |cx: &mut TestAppContext| {
        on(cx, &scene, |window, _| {
            window.try_find("popup-menu").is_some()
        })
    };
    // The model opens a menu by itself, as a scene does: it is on screen,
    // inside the window, with the model's items.
    assert!(!shown(cx));
    let target = object.clone();
    model_side(cx, &scene, move |m| {
        assert!(m.open_context_menu(target, MenuOrigin::Grid));
    });
    on(cx, &scene, |window, _| {
        let menu = window.find("popup-menu").bounds();
        let viewport = window.viewport_size();
        assert!(menu.left() >= px(0.) && menu.right() <= viewport.width);
        assert!(menu.top() >= px(0.) && menu.bottom() <= viewport.height);
        assert_eq!(
            window.find_by_label("Copy UUID").role(),
            Some(Role::MenuItem)
        );
    });
    assert!(cx.update(|cx| scene.view.read(cx).menu_open()));
    // The model ends it (another selection does): it is gone.
    let elsewhere = host.object.clone().expect("the host");
    model_side(cx, &scene, move |m| {
        m.select(xenadmin_ui::object::Target::one(elsewhere));
    });
    assert!(model(cx, &scene, |m| m.selection.menu.is_none()));
    assert!(!shown(cx));
    assert!(!cx.update(|cx| scene.view.read(cx).menu_open()));

    // The secondary button on a row, then on another row while the first
    // menu is open: the second menu replaces the first, at the new place.
    on(cx, &scene, |window, cx| {
        window.right_click(id(&database.id), cx)
    });
    let first = on(cx, &scene, |window, _| window.find("popup-menu").bounds());
    on(cx, &scene, |window, cx| {
        window.right_click(id(&host.id), cx)
    });
    let second = on(cx, &scene, |window, _| window.find("popup-menu").bounds());
    assert_ne!(first.origin, second.origin);
    assert_eq!(
        model(cx, &scene, |m| m
            .selection
            .menu
            .as_ref()
            .map(|menu| menu.object.clone())),
        host.object
    );
    assert_eq!(
        model(cx, &scene, |m| m.scope().single().cloned()),
        host.object
    );
    // A press outside the menu closes it, in the model too. The far end of
    // the status bar: a server's menu, with every area's commands, covers
    // the tab strip beside the tree.
    on(cx, &scene, |window, cx| {
        window.click(status::SELECTION_KEY, cx)
    });
    cx.wait_for(scene.window, Duration::from_secs(2), |window, _| {
        window.try_find("popup-menu").is_none()
    })
    .await;
    settle(cx, &scene);
    assert!(model(cx, &scene, |m| m.selection.menu.is_none()));
}

#[gpui_kit::test]
async fn a_grid_row_that_stands_for_an_object_has_its_context_menu_at_the_pointer(
    cx: &mut TestAppContext,
) {
    use xenadmin_ui::selection::MenuOrigin;
    let scene = open_sized(cx, 1440., 900., |setup| {
        let mut lab = setup.lab.as_ref().expect("lab").borrow_mut();
        lab.object_rows = true;
    });
    let guest = model(cx, &scene, |m| {
        let inventory = m.inventory.clone().expect("demo inventory");
        let mut guests: Vec<_> = inventory.guests().collect();
        guests.sort_by(|left, right| {
            xenadmin_ui::natural_cmp(&left.name, &right.name)
                .then_with(|| left.id.to_string().cmp(&right.id.to_string()))
        });
        guests[1].id.to_string()
    });
    let row = id(&xenadmin_ui::surface::Table::row_key(TABLE, &guest));
    let place = on(cx, &scene, |window, cx| {
        let place = window.find(row.clone()).bounds();
        window.right_click(row.clone(), cx);
        place
    });
    // The model has the kernel's menu of the row's object open, in the
    // grid, and the row is the selection.
    let open = model(cx, &scene, |m| m.selection.menu.clone()).expect("a context menu");
    assert_eq!(open.origin, MenuOrigin::Grid);
    assert_eq!(open.object.id, guest);
    assert_eq!(
        model(cx, &scene, |m| m.target().single().cloned()),
        Some(open.object.clone())
    );
    // The shell presents it where the pointer pressed: on the row, not at
    // the place a menu without a pointer gets.
    on(cx, &scene, |window, _| {
        let menu = window.find("popup-menu").bounds();
        assert!(
            menu.left() >= place.left() && menu.left() <= place.right(),
            "{menu:?} {place:?}"
        );
        // At the row when the menu fits below it; a menu too tall for that
        // (a guest's, with every area's commands) is moved up to stay
        // inside the window, never below the row.
        assert!(menu.top() <= place.bottom() + px(2.), "{menu:?} {place:?}");
        assert!(menu.bottom() <= px(900.), "{menu:?}");
        if place.top() + menu.size.height <= px(900.) {
            assert!(menu.top() >= place.top() - px(2.), "{menu:?} {place:?}");
        }
        assert_eq!(
            window.find_by_label("Copy UUID").role(),
            Some(Role::MenuItem)
        );
    });
    assert!(cx.update(|cx| scene.view.read(cx).menu_open()));
    // Choosing an item runs the command on the row's object and ends the
    // menu.
    let copy = popup_index(cx, &scene, "Copy UUID");
    on(cx, &scene, |window, cx| {
        window.within("popup-menu").click(copy, cx);
    });
    cx.wait_for(scene.window, Duration::from_secs(2), |window, _| {
        window.try_find("popup-menu").is_none()
    })
    .await;
    settle(cx, &scene);
    let copied: Vec<String> = cx.update(|cx| {
        scene
            .store
            .read(cx)
            .recorded_intents()
            .iter()
            .filter_map(|intent| match intent {
                Intent::CopyText(text) => Some(text.clone()),
                _ => None,
            })
            .collect()
    });
    assert_eq!(copied, std::slice::from_ref(&guest));
    assert!(model(cx, &scene, |m| m.selection.menu.is_none()));
}

#[gpui_kit::test]
fn back_and_forward_step_through_the_selections(cx: &mut TestAppContext) {
    let scene = open(cx);
    let pool = model(cx, &scene, |m| m.scope().clone());
    let host = nav_row(cx, &scene, "xcp-lab-01");
    let database = nav_row(cx, &scene, "Database");
    on(cx, &scene, |window, cx| window.click(id(&host.id), cx));
    on(cx, &scene, |window, cx| window.click(id(&database.id), cx));
    let scope = |cx: &mut TestAppContext| model(cx, &scene, |m| m.scope().single().cloned());
    assert_eq!(scope(cx), database.object);
    // The toolbar buttons.
    on(cx, &scene, |window, cx| {
        let mut bar = window.within(toolbar::TOOLBAR_KEY);
        assert_eq!(bar.find("nav.back").disabled(), None);
        assert_eq!(bar.find("nav.forward").disabled(), Some(true));
        bar.click("nav.back", cx);
    });
    assert_eq!(scope(cx), host.object);
    assert_eq!(cursor(cx, &scene), Some(host.id.clone()));
    on(cx, &scene, |window, cx| {
        window.within(toolbar::TOOLBAR_KEY).click("nav.forward", cx);
    });
    assert_eq!(scope(cx), database.object);
    assert_eq!(cursor(cx, &scene), Some(database.id.clone()));
    // The shortcuts.
    on(cx, &scene, |window, cx| {
        window.press("alt-left", cx);
        window.press("alt-left", cx);
    });
    assert_eq!(model(cx, &scene, |m| m.scope().clone()), pool);
    on(cx, &scene, |window, cx| window.press("alt-right", cx));
    assert_eq!(scope(cx), host.object);
    // A tab change is a step too.
    on(cx, &scene, |window, cx| {
        window.click(TabId::Search.key(), cx)
    });
    assert_eq!(model(cx, &scene, Model::active_tab), TabId::Search);
    on(cx, &scene, |window, cx| window.press("alt-left", cx));
    assert_eq!(model(cx, &scene, Model::active_tab), TabId::General);
    assert_eq!(scope(cx), host.object);
    on(cx, &scene, |window, _| {
        assert_eq!(window.find(TabId::General.key()).selected(), Some(true));
        assert_eq!(tree_row(window, "xcp-lab-01").selected(), Some(true));
    });
}

#[gpui_kit::test]
fn the_navigation_pane_width_round_trips_through_the_preferences(cx: &mut TestAppContext) {
    // The width of the preferences is the width of the pane.
    let scene = open_sized(cx, 1440., 900., |setup| {
        setup.model.preferences.sidebar_width = 320.;
    });
    let split = cx.update(|cx| scene.view.read(cx).split().clone());
    on(cx, &scene, |window, _| {
        assert_eq!(window.find(pane::PANE_KEY).bounds().size.width, px(320.));
    });
    assert_eq!(cx.update(|cx| split.read(cx).sizes()[0]), px(320.));
    assert!(!model(cx, &scene, Model::layout_pending));

    // Dragging the divider is kept by the model, in the preferences.
    let (from, to) = on(cx, &scene, |window, _| {
        let content = window.find(shell::CONTENT_KEY).bounds();
        let y = content.center().y;
        (
            point(content.left() + px(1.), y),
            point(content.left() + px(61.), y),
        )
    });
    on(cx, &scene, |window, cx| window.drag(from, to, cx));
    let dragged = model(cx, &scene, |m| m.preferences.sidebar_width);
    assert!((dragged - 380.).abs() < 2., "{dragged}");
    assert!(model(cx, &scene, Model::layout_pending));
    on(cx, &scene, |window, _| {
        let width = f32::from(window.find(pane::PANE_KEY).bounds().size.width);
        assert!((width - dragged).abs() < 1., "{width} and {dragged}");
    });
    // It is written once it has rested for the delay the Rune poll used
    // (nothing is written here: a fixture does not own the state directory).
    cx.executor().advance_clock(Duration::from_millis(700));
    cx.run_until_parked();
    assert!(!model(cx, &scene, Model::layout_pending));
    assert!((model(cx, &scene, |m| m.preferences.sidebar_width) - dragged).abs() < 0.01);

    // The pane stays inside what the preferences allow.
    on(cx, &scene, |window, cx| {
        split.update(cx, |split, cx| split.resize_panel(0, px(900.), window, cx));
    });
    assert_eq!(model(cx, &scene, |m| m.preferences.sidebar_width), 420.);
    on(cx, &scene, |window, cx| {
        split.update(cx, |split, cx| split.resize_panel(0, px(20.), window, cx));
    });
    assert_eq!(model(cx, &scene, |m| m.preferences.sidebar_width), 180.);

    // A window opened with the stored width shows it again.
    let stored = model(cx, &scene, |m| m.preferences.sidebar_width);
    let again = open_sized(cx, 1440., 900., |setup| {
        setup.model.preferences.sidebar_width = stored;
    });
    on(cx, &again, |window, _| {
        assert_eq!(window.find(pane::PANE_KEY).bounds().size.width, px(stored));
    });
}

#[gpui_kit::test]
fn the_pane_gives_way_to_a_toggle_in_a_narrow_window(cx: &mut TestAppContext) {
    let scene = open_sized(cx, 1000., 700., |_| {});
    on(cx, &scene, |window, cx| {
        assert!(window.try_find(pane::PANE_KEY).is_none());
        let content = window.find(shell::CONTENT_KEY).bounds();
        assert_eq!(content.left(), px(0.));
        assert_eq!(content.size.width, window.viewport_size().width);
        let toggle = window.find(shell::TOGGLE_KEY);
        assert_eq!(toggle.label(), Some("Show sidebar"));
        window.click(shell::TOGGLE_KEY, cx);
    });
    on(cx, &scene, |window, cx| {
        let side = window.find(pane::PANE_KEY).bounds();
        assert_eq!(side.size.width, px(260.));
        let content = window.find(shell::CONTENT_KEY).bounds();
        assert!(side.right() <= content.left());
        assert_eq!(content.right(), window.viewport_size().width);
        assert_eq!(window.find(shell::TOGGLE_KEY).label(), Some("Hide sidebar"));
        window.click(shell::TOGGLE_KEY, cx);
    });
    on(cx, &scene, |window, _| {
        assert!(window.try_find(pane::PANE_KEY).is_none());
    });
    // Wide again: the pane is back and the toggle is gone.
    cx.simulate_window_resize(scene.window, size(px(1300.), px(700.)));
    settle(cx, &scene);
    on(cx, &scene, |window, _| {
        assert!(window.find(pane::PANE_KEY).visible());
        assert!(window.try_find(shell::TOGGLE_KEY).is_none());
    });
    // At a larger interface the window is narrow sooner.
    model_side(cx, &scene, |m| m.preferences.options.interface_scale = 125);
    on(cx, &scene, |window, _| {
        assert!(window.try_find(pane::PANE_KEY).is_none());
        assert!(window.find(shell::TOGGLE_KEY).visible());
    });
}

#[gpui_kit::test]
fn the_zoom_commands_change_the_interface_scale(cx: &mut TestAppContext) {
    let scene = open(cx);
    let scale =
        |cx: &mut TestAppContext| model(cx, &scene, |m| m.preferences.options.interface_scale);
    let font = |cx: &mut TestAppContext| cx.update(|cx| f32::from(cx.theme().font_size));
    let (row, pane_width) = on(cx, &scene, |window, _| {
        (
            window.find(TabId::General.key()).bounds().size.height,
            window.find(pane::PANE_KEY).bounds().size.width,
        )
    });
    assert_eq!((scale(cx), font(cx)), (100, 16.));
    // Ctrl with equals, and Ctrl with plus.
    on(cx, &scene, |window, cx| window.press("ctrl-=", cx));
    assert_eq!((scale(cx), font(cx)), (110, 17.6));
    on(cx, &scene, |window, cx| window.press("ctrl-+", cx));
    assert_eq!((scale(cx), font(cx)), (125, 20.));
    on(cx, &scene, |window, _| {
        // Everything in rems follows; the pane keeps its width in the units
        // of the scale.
        let tab = window.find(TabId::General.key()).bounds().size.height;
        assert!(tab > row, "{tab:?} and {row:?}");
        let width = window.find(pane::PANE_KEY).bounds().size.width;
        assert_eq!(width, pane_width * 1.25);
    });
    assert_eq!(model(cx, &scene, |m| m.preferences.sidebar_width), 260.);
    on(cx, &scene, |window, cx| window.press("ctrl-0", cx));
    assert_eq!((scale(cx), font(cx)), (100, 16.));
    on(cx, &scene, |window, cx| {
        window.press("ctrl--", cx);
        window.press("ctrl--", cx);
        window.press("ctrl--", cx);
    });
    // 80 is the smallest: the third press is refused with its reason.
    assert_eq!((scale(cx), font(cx)), (80, 12.8));
    assert_eq!(
        model(cx, &scene, |m| m.error.clone()).as_deref(),
        Some("The interface is at its smallest size.")
    );
    // The menu says the same.
    let view = menu_flags(cx, "View");
    assert!(view.contains(&(
        "Smaller Interface — The interface is at its smallest size.".into(),
        true,
        false
    )));
    assert!(view.contains(&("Larger Interface".into(), false, false)));
    assert!(view.contains(&("Reset Interface Size".into(), false, false)));
}

#[gpui_kit::test]
fn the_theme_follows_the_appearance_preference_and_keeps_the_accent(cx: &mut TestAppContext) {
    use xenadmin_ui::prefs::Appearance;
    // The scene asked for the dark theme; the preference `System` leaves it.
    let scene = open(cx);
    let look = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            let theme = cx.theme();
            (
                theme.mode,
                theme.primary,
                theme.button_primary,
                theme.background,
            )
        })
    };
    let (mode, dark_accent, dark_button, dark_background) = look(cx);
    assert_eq!(mode, ThemeMode::Dark);
    assert_eq!(dark_accent, dark_button);
    model_side(cx, &scene, |m| {
        m.preferences.options.appearance = Appearance::Light
    });
    let (mode, light_accent, light_button, light_background) = look(cx);
    assert_eq!(mode, ThemeMode::Light);
    // The Kit's own primary is neutral: the accent has a hue, in both modes.
    assert_eq!(light_accent, light_button);
    assert!(light_accent.s > 0.3 && dark_accent.s > 0.3);
    assert!((light_accent.h - dark_accent.h).abs() < 0.02);
    assert!(light_background.l > 0.9 && dark_background.l < 0.2);
    model_side(cx, &scene, |m| {
        m.preferences.options.appearance = Appearance::Dark
    });
    let (mode, accent, _, background) = look(cx);
    assert_eq!(
        (mode, accent, background),
        (ThemeMode::Dark, dark_accent, dark_background)
    );
    // A change of the scale keeps the mode and the accent.
    model_side(cx, &scene, |m| m.preferences.options.interface_scale = 150);
    let (mode, accent, _, _) = look(cx);
    assert_eq!((mode, accent), (ThemeMode::Dark, dark_accent));
    assert_eq!(cx.update(|cx| f32::from(cx.theme().font_size)), 24.);
}

#[gpui_kit::test]
fn the_zoom_shortcuts_apply_while_a_dialog_is_open(cx: &mut TestAppContext) {
    let scene = open(cx);
    let scale =
        |cx: &mut TestAppContext| model(cx, &scene, |m| m.preferences.options.interface_scale);
    let lab = scene.lab.clone().expect("lab");
    cx.update(|cx| {
        scene.store.update(cx, |store, cx| {
            store.run(|_| lab.borrow_mut().open.push(DIALOG), cx);
        });
    });
    cx.run_until_parked();
    settle(cx, &scene);
    on(cx, &scene, |window, cx| {
        assert!(window.find(DIALOG).visible());
        window.press("ctrl-=", cx);
    });
    assert_eq!(scale(cx), 110);
    on(cx, &scene, |window, cx| window.press("ctrl-0", cx));
    assert_eq!(scale(cx), 100);
}

/// A stand-in for the shell with a console in it: the two key contexts and
/// a record of the commands that arrive.
struct ConsoleProbe {
    shell: FocusHandle,
    console: FocusHandle,
    commands: Rc<RefCell<Vec<&'static str>>>,
}

impl Render for ConsoleProbe {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("probe-shell")
            .test_support()
            .track_focus(&self.shell)
            .key_context(shell::actions::SHELL_CONTEXT)
            .on_action(
                cx.listener(|probe, action: &shell::actions::RunCommand, _, _| {
                    probe.commands.borrow_mut().push(action.id);
                }),
            )
            .size_full()
            .child(
                div()
                    .id("probe-console")
                    .test_support()
                    .track_focus(&self.console)
                    .key_context(shell::actions::CONSOLE_CONTEXT)
                    .size_8(),
            )
    }
}

#[gpui_kit::test]
fn no_command_shortcut_is_delivered_while_a_console_has_the_keyboard(cx: &mut TestAppContext) {
    let commands: Rc<RefCell<Vec<&'static str>>> = Rc::default();
    let (window, probe) = cx
        .update(|cx| {
            gpui_kit::init(cx);
            xenadmin_desktop::init(cx);
            let commands = commands.clone();
            gpui_kit::open_window(WindowOptions::default(), cx, move |_, cx| {
                cx.new(|cx| ConsoleProbe {
                    shell: cx.focus_handle(),
                    console: cx.focus_handle(),
                    commands,
                })
            })
        })
        .expect("window");
    let (in_shell, in_console) = cx.update(|cx| {
        let probe = probe.read(cx);
        (probe.shell.clone(), probe.console.clone())
    });
    let press_all = |cx: &mut TestAppContext, focus: &FocusHandle| {
        cx.update_window(window, |_, window, cx| {
            focus.focus(window, cx);
            window.render_frame(cx);
            for keys in [
                "ctrl-=",
                "ctrl-+",
                "ctrl--",
                "ctrl-0",
                "alt-left",
                "alt-right",
                "ctrl-q",
            ] {
                window.press(keys, cx);
            }
        })
        .expect("window");
        cx.run_until_parked();
    };
    // In the shell every shortcut of the kernel's commands arrives.
    press_all(cx, &in_shell);
    assert_eq!(
        *commands.borrow(),
        [
            "view.zoom-in",
            "view.zoom-in",
            "view.zoom-out",
            "view.zoom-reset",
            "nav.back",
            "nav.forward",
            "file.exit"
        ]
    );
    // In the console none does: every key is the guest's.
    commands.borrow_mut().clear();
    press_all(cx, &in_console);
    assert!(commands.borrow().is_empty(), "{:?}", commands.borrow());
}

#[gpui_kit::test]
fn the_view_switcher_and_the_search_box_drive_the_model(cx: &mut TestAppContext) {
    let scene = open(cx);
    on(cx, &scene, |window, cx| {
        window.click(NavView::Objects.key(), cx)
    });
    assert_eq!(model(cx, &scene, |m| m.nav.view), NavView::Objects);
    on(cx, &scene, |window, _| {
        assert_eq!(window.find(NavView::Objects.key()).selected(), Some(true));
        assert_eq!(
            window.find(NavView::Infrastructure.key()).selected(),
            Some(false)
        );
        // The headings of the Objects view.
        // (the later headings are below what the pane has room for).
        for heading in ["Pools", "Servers", "VMs"] {
            assert_eq!(
                tree_row(window, heading).role(),
                Some(Role::TreeItem),
                "{heading}"
            );
        }
        // The selection stayed.
        assert_eq!(
            window.find(status::SELECTION_KEY).label(),
            Some("Demonstration pool")
        );
    });
    // The search box narrows the rows.
    on(cx, &scene, |window, cx| {
        window.click(pane::SEARCH_KEY, cx);
        window.input("xcp-lab-02", cx);
    });
    assert_eq!(model(cx, &scene, |m| m.nav.filter.clone()), "xcp-lab-02");
    let rows = model(cx, &scene, nav::rows);
    assert!(rows.iter().any(|row| row.label == "xcp-lab-02"));
    assert!(rows.iter().all(|row| row.label != "xcp-lab-01"));
    on(cx, &scene, |window, _| {
        assert_eq!(window.find(pane::SEARCH_KEY).value(), Some("xcp-lab-02"));
        assert!(try_tree_row(window, "xcp-lab-02").is_some());
        assert!(try_tree_row(window, "xcp-lab-01").is_none());
        assert!(window.try_find(pane::EMPTY_KEY).is_none());
    });
    // Nothing matches: the pane says so.
    on(cx, &scene, |window, cx| window.input(" zzz", cx));
    on(cx, &scene, |window, _| {
        let empty = window.find(pane::EMPTY_KEY);
        assert_eq!(
            empty.label(),
            Some("No matching objects. Clear the search to see everything.")
        );
    });
    // The model clears the filter: the box follows.
    model_side(cx, &scene, |m| m.set_nav_filter(""));
    on(cx, &scene, |window, _| {
        assert_eq!(window.find(pane::SEARCH_KEY).value(), Some(""));
        assert!(window.try_find(pane::EMPTY_KEY).is_none());
    });
    // A view no area provides rows for has an empty state of its own.
    on(cx, &scene, |window, cx| {
        window.click(NavView::SavedSearches.key(), cx);
    });
    on(cx, &scene, |window, _| {
        assert_eq!(
            window.find(pane::EMPTY_KEY).label(),
            Some("No saved searches.")
        );
    });
}

/// The search box's own clear button, pressed with the pointer: the text is
/// empty, the model's filter is empty and every row is back. The button is
/// the Kit's (`Input::cleanable`); it is in the tree only while the box has
/// text.
#[gpui_kit::test]
fn the_search_box_clear_button_empties_the_search_and_restores_every_row(cx: &mut TestAppContext) {
    /// The clear button of the search box: the Kit keys it `clean`.
    fn clear_button(window: &mut Window) -> Option<ElementSnapshot> {
        window.elements().into_iter().find(|element| {
            let path = element.path();
            let Some((last, before)) = path.split_last() else {
                return false;
            };
            *last == id("clean") && before.contains(&id(pane::SEARCH_KEY))
        })
    }

    let scene = open(cx);
    let every_row: Vec<String> = model(cx, &scene, nav::rows)
        .into_iter()
        .map(|row| row.id)
        .collect();
    assert!(every_row.len() > 2, "the demonstration has rows to hide");
    // An empty box has nothing to clear.
    on(cx, &scene, |window, _| {
        assert!(clear_button(window).is_none());
    });
    on(cx, &scene, |window, cx| {
        window.click(pane::SEARCH_KEY, cx);
        window.input("xcp-lab-02", cx);
    });
    assert_eq!(model(cx, &scene, |m| m.nav.filter.clone()), "xcp-lab-02");
    let narrowed = model(cx, &scene, nav::rows);
    assert!(narrowed.len() < every_row.len());
    on(cx, &scene, |window, _| {
        assert!(try_tree_row(window, "xcp-lab-01").is_none());
    });
    // The pointer goes to the button and presses it.
    on(cx, &scene, |window, cx| {
        let button = clear_button(window).expect("the search box has a clear button");
        let at = button.bounds().center();
        let viewport = window.viewport_size();
        assert!(
            at.x > px(0.) && at.y > px(0.) && at.x < viewport.width && at.y < viewport.height,
            "the clear button is on screen"
        );
        assert!(
            window.find(pane::SEARCH_KEY).bounds().contains(&at),
            "the clear button is inside the search box"
        );
        window.pointer_move(at, None, cx);
        window.render_frame(cx);
        window.pointer_down(at, MouseButton::Left, cx);
        window.pointer_up(at, MouseButton::Left, cx);
    });
    assert_eq!(model(cx, &scene, |m| m.nav.filter.clone()), "");
    let restored: Vec<String> = model(cx, &scene, nav::rows)
        .into_iter()
        .map(|row| row.id)
        .collect();
    assert_eq!(restored, every_row);
    on(cx, &scene, |window, _| {
        assert_eq!(window.find(pane::SEARCH_KEY).value(), Some(""));
        assert!(try_tree_row(window, "xcp-lab-01").is_some());
        assert!(try_tree_row(window, "xcp-lab-02").is_some());
        assert!(window.try_find(pane::EMPTY_KEY).is_none());
        assert!(clear_button(window).is_none());
    });
}

#[gpui_kit::test]
fn the_status_bar_counts_open_the_notifications_view(cx: &mut TestAppContext) {
    let mut model_of_scene = Model::fixture();
    assert!(scene::apply("host-version-notice", &mut model_of_scene));
    let options = Options::default();
    let scene = cx
        .update(|cx| scenes::open_shell(cx, Setup::model(model_of_scene), &options))
        .expect("shell window");
    cx.run_until_parked();
    settle(cx, &scene);
    let notice = model(cx, &scene, |m| m.status().host_notice).expect("the host notice");
    on(cx, &scene, |window, cx| {
        let line = window.find(status::HOST_NOTICE_KEY);
        assert_eq!(line.role(), Some(Role::Status));
        assert_eq!(line.label(), Some(notice.as_str()));
        // The notice is above the status line and inside the window.
        let status_line = window.find(status::CONNECTION_KEY).bounds();
        assert!(line.bounds().bottom() <= status_line.top());
        assert!(line.bounds().right() <= window.viewport_size().width);
        let count = window.find(status::UNKNOWN_KEY);
        assert_eq!(count.role(), Some(Role::Link));
        window.click(status::UNKNOWN_KEY, cx);
    });
    assert_eq!(model(cx, &scene, |m| m.nav.view), NavView::Notifications);
    assert_eq!(model(cx, &scene, Model::active_tab), TabId::Events);
    // The count of unknown outcomes shows them: the History list of every
    // pool, filtered to that state.
    let history = xenadmin_ui::areas::events::activity::Tab::History;
    model(cx, &scene, |m| {
        let events = &m.areas.events;
        assert_eq!(events.list, history);
        assert_eq!(events.state_filter, "OutcomeUnknown");
        assert_eq!(events.pool, None);
    });
    on(cx, &scene, |window, _| {
        assert_eq!(
            window.find(NavView::Notifications.key()).selected(),
            Some(true)
        );
        assert_eq!(window.find(TabId::Events.key()).selected(), Some(true));
        assert_eq!(
            window
                .find(xenadmin_desktop::scene_view::HEADER_KEY)
                .label(),
            Some("Notifications")
        );
        // The two rows of the view.
        assert!(try_tree_row(window, "Alerts").is_some());
        assert!(try_tree_row(window, "Events").is_some());
    });
    // The other count goes there too, from another view.
    on(cx, &scene, |window, cx| {
        window.click(NavView::Infrastructure.key(), cx);
    });
    assert_eq!(model(cx, &scene, |m| m.nav.view), NavView::Infrastructure);
    on(cx, &scene, |window, cx| {
        window.click(status::OPERATIONS_KEY, cx)
    });
    assert_eq!(model(cx, &scene, |m| m.nav.view), NavView::Notifications);
    // The count of operations shows every record: the filter is gone.
    model(cx, &scene, |m| {
        let events = &m.areas.events;
        assert_eq!(m.active_tab(), TabId::Events);
        assert_eq!(events.list, history);
        assert_eq!(events.state_filter, "");
        assert_eq!(events.pool, None);
    });
    // The keyboard presses a count as the pointer does.
    on(cx, &scene, |window, cx| {
        window.click(NavView::Infrastructure.key(), cx);
    });
    on(cx, &scene, |window, cx| {
        let mut presses = 0;
        while !focused(window, status::UNKNOWN_KEY) {
            window.press("shift-tab", cx);
            presses += 1;
            assert!(presses < 60, "Shift+Tab never reached the count");
        }
        window.press("enter", cx);
    });
    model(cx, &scene, |m| {
        assert_eq!(m.nav.view, NavView::Notifications);
        assert_eq!(m.areas.events.state_filter, "OutcomeUnknown");
    });
}

#[gpui_kit::test]
fn a_fullscreen_request_toggles_the_main_window_and_a_consoles_goes_to_its_hook(
    cx: &mut TestAppContext,
) {
    use xenadmin_ui::intent::{ConsoleWindowId, WindowRef};
    let asked: Rc<RefCell<Vec<ConsoleWindowId>>> = Rc::default();
    let hooks = Hooks {
        console_fullscreen: {
            let asked = asked.clone();
            Rc::new(move |id, _| asked.borrow_mut().push(id))
        },
        ..Hooks::default()
    };
    let scene = cx
        .update(|cx| scenes::open_shell_with(cx, Setup::lab(), &Options::default(), hooks.clone()))
        .expect("shell window");
    cx.run_until_parked();
    settle(cx, &scene);
    cx.update(|cx| intents::install(&scene.store, hooks, cx));
    assert!(!on(cx, &scene, |window, _| window.is_fullscreen()));
    model_side(cx, &scene, |m| m.toggle_fullscreen(WindowRef::Main));
    assert!(on(cx, &scene, |window, _| window.is_fullscreen()));
    assert!(asked.borrow().is_empty());
    // A second request leaves fullscreen: it is a toggle.
    model_side(cx, &scene, |m| m.toggle_fullscreen(WindowRef::Main));
    assert!(!on(cx, &scene, |window, _| window.is_fullscreen()));
    // A console window's request is the console's to perform.
    model_side(cx, &scene, |m| {
        m.toggle_fullscreen(WindowRef::Console(ConsoleWindowId(4)));
    });
    assert_eq!(*asked.borrow(), [ConsoleWindowId(4)]);
    assert!(!on(cx, &scene, |window, _| window.is_fullscreen()));
}

#[gpui_kit::test]
fn a_prompt_that_failed_and_was_given_no_typed_path_tells_the_request_why(cx: &mut TestAppContext) {
    let scene = open(cx);
    let services = cx.update(|cx| intents::install(&scene.store, Hooks::default(), cx));
    let heard: Rc<RefCell<Vec<String>>> = Rc::default();
    let request = || {
        let (replies, failures) = (heard.clone(), heard.clone());
        FileRequest::save(
            "Export filtered entries",
            "activity.json",
            move |_, path| {
                replies.borrow_mut().push(format!("reply {path:?}"));
            },
        )
        .on_failure(move |m, error| {
            failures.borrow_mut().push(format!("failed {error}"));
            m.error = Some(format!("Cannot open save dialog: {error}"));
        })
    };
    // The chooser did not answer: the path is asked for as text.
    cx.update(|cx| {
        assert!(
            services
                .ask_typed_path(request(), "no portal".into())
                .is_none()
        );
        scene.store.update(cx, |store, cx| store.touch(cx));
    });
    cx.run_until_parked();
    settle(cx, &scene);
    assert!(
        heard.borrow().is_empty(),
        "nothing is said while it is open"
    );
    // Closed without a path: the request hears what the portal said, not a
    // cancellation, and its text is on the error line.
    on(cx, &scene, |window, cx| {
        assert!(window.find(intents::TYPED_PATH).visible());
        window.click(intents::TYPED_PATH_CANCEL, cx);
    });
    assert_eq!(*heard.borrow(), ["failed no portal"]);
    assert!(!services.typed_path_open());
    on(cx, &scene, |window, _| {
        assert_eq!(
            window.find(status::ERROR_KEY).label(),
            Some("Cannot open save dialog: no portal")
        );
    });
    // Escape closes it the same way.
    cx.update(|cx| {
        let _ = services.ask_typed_path(request(), "portal timed out".into());
        scene.store.update(cx, |store, cx| store.touch(cx));
    });
    cx.run_until_parked();
    settle(cx, &scene);
    on(cx, &scene, |window, cx| window.press("escape", cx));
    assert_eq!(
        *heard.borrow(),
        ["failed no portal", "failed portal timed out"]
    );
    // The chooser itself, cancelled by the user, is a cancellation.
    model_side(cx, &scene, |m| m.choose_file(request()));
    assert!(cx.did_prompt_for_new_path());
    cx.simulate_new_path_selection(|_| None);
    cx.run_until_parked();
    assert_eq!(
        heard.borrow().last().map(String::as_str),
        Some("reply None")
    );
}

#[gpui_kit::test]
fn an_intent_for_a_file_opens_the_prompt_of_the_platform(cx: &mut TestAppContext) {
    let scene = open(cx);
    let services = cx.update(|cx| intents::install(&scene.store, Hooks::default(), cx));
    let answers: Rc<RefCell<Vec<Option<PathBuf>>>> = Rc::default();
    let request = |answers: &Rc<RefCell<Vec<Option<PathBuf>>>>| {
        let answers = answers.clone();
        move |_: &mut Model, path: Option<PathBuf>| answers.borrow_mut().push(path)
    };

    // Save: the platform's prompt for a new path, with the suggested name.
    let save = FileRequest::save("Export report", "inventory.csv", request(&answers));
    model_side(cx, &scene, |m| m.choose_file(save));
    assert!(cx.did_prompt_for_new_path());
    assert!(
        answers.borrow().is_empty(),
        "the reply waits for the answer"
    );
    cx.simulate_new_path_selection(|directory| Some(directory.join("inventory.csv")));
    cx.run_until_parked();
    let saved = answers.borrow().clone();
    assert_eq!(saved.len(), 1);
    let path = saved[0].clone().expect("a path");
    assert_eq!(
        path.file_name().and_then(|name| name.to_str()),
        Some("inventory.csv")
    );

    // Open: the platform's prompt for paths, asking for one file.
    let open_request = FileRequest::open("Import", request(&answers));
    model_side(cx, &scene, |m| m.choose_file(open_request));
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|options| {
        assert!(options.files && !options.directories && !options.multiple);
        Some(vec![PathBuf::from("/tmp/a.ova")])
    });
    cx.run_until_parked();
    assert_eq!(
        answers.borrow().last(),
        Some(&Some(PathBuf::from("/tmp/a.ova")))
    );

    // Cancelled: the reply hears that nothing was chosen.
    let cancelled = FileRequest::directory("Choose a folder", request(&answers));
    model_side(cx, &scene, |m| m.choose_file(cancelled));
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|options| {
        assert!(options.directories && !options.files);
        None
    });
    cx.run_until_parked();
    assert_eq!(answers.borrow().len(), 3);
    assert_eq!(answers.borrow().last(), Some(&None));
    assert!(!services.typed_path_open());

    // No portal answers: the path is asked for as text, in a dialog of the
    // renderer.
    let directory = std::env::temp_dir().join(format!(
        "xenadmin-desktop-shell-test-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("temporary directory");
    let typed = FileRequest::save("Export report", "inventory.csv", request(&answers));
    cx.update(|cx| {
        assert!(
            services
                .ask_typed_path(typed, "The file picker portal is missing.".into())
                .is_none()
        );
        scene.store.update(cx, |store, cx| store.touch(cx));
    });
    cx.run_until_parked();
    settle(cx, &scene);
    let wanted = directory.join("typed.csv");
    on(cx, &scene, |window, cx| {
        assert!(window.find(intents::TYPED_PATH).visible());
        assert_eq!(
            window.find(intents::TYPED_PATH_REASON).role(),
            Some(Role::Status)
        );
        // The field has the keyboard: replace what it suggests.
        assert!(focused(window, intents::TYPED_PATH_INPUT));
        window.press("ctrl-a", cx);
        window.input("relative.csv", cx);
    });
    on(cx, &scene, |window, cx| {
        window.click(intents::TYPED_PATH_CONFIRM, cx)
    });
    // Refused where it stands: the dialog stays and says why.
    assert!(services.typed_path_open());
    assert_eq!(answers.borrow().len(), 3);
    on(cx, &scene, |window, cx| {
        assert!(window.find(intents::TYPED_PATH).visible());
        window.click(intents::TYPED_PATH_INPUT, cx);
        window.press("ctrl-a", cx);
        window.input(&wanted.display().to_string(), cx);
    });
    on(cx, &scene, |window, cx| window.press("enter", cx));
    assert_eq!(answers.borrow().last(), Some(&Some(wanted.clone())));
    assert!(!services.typed_path_open());
    on(cx, &scene, |window, _| {
        assert!(window.try_find(intents::TYPED_PATH).is_none());
    });
    let _ = std::fs::remove_dir_all(&directory);
}

#[gpui_kit::test]
fn the_other_intents_reach_the_platform_and_the_hooks(cx: &mut TestAppContext) {
    let scene = open(cx);
    let quits = Rc::new(Cell::new(0));
    let consoles = Rc::new(Cell::new(0));
    let hooks = Hooks {
        quit: {
            let quits = quits.clone();
            Rc::new(move |_| quits.set(quits.get() + 1))
        },
        console_window: {
            let consoles = consoles.clone();
            Rc::new(move |_, _| consoles.set(consoles.get() + 1))
        },
        ..Hooks::default()
    };
    cx.update(|cx| intents::install(&scene.store, hooks, cx));
    model_side(cx, &scene, |m| m.open_url("https://docs.xcp-ng.org/"));
    assert_eq!(cx.opened_url().as_deref(), Some("https://docs.xcp-ng.org/"));
    model_side(cx, &scene, |m| m.copy_text("3f0c7f6e"));
    assert_eq!(
        cx.read_from_clipboard()
            .and_then(|item| item.text())
            .as_deref(),
        Some("3f0c7f6e")
    );
    let pasted: Rc<RefCell<Vec<Option<String>>>> = Rc::default();
    let reply = pasted.clone();
    model_side(cx, &scene, move |m| {
        m.read_clipboard(move |_, text| reply.borrow_mut().push(text));
    });
    assert_eq!(*pasted.borrow(), [Some("3f0c7f6e".to_owned())]);
    // Exit from the File menu's command: the application's close path.
    on(cx, &scene, |window, cx| window.press("ctrl-q", cx));
    assert_eq!(quits.get(), 1);
    // A console window request goes to the console engineer's hook.
    model_side(cx, &scene, |m| {
        m.send(Intent::ConsoleWindow(
            xenadmin_ui::intent::ConsoleWindowRequest::Activate(
                xenadmin_ui::intent::ConsoleWindowId(2),
            ),
        ));
    });
    assert_eq!(consoles.get(), 1);
    // Nothing is left recorded: every intent was performed.
    assert!(cx.update(|cx| scene.store.read(cx).recorded_intents().is_empty()));
}

#[gpui_kit::test]
fn closing_the_window_runs_the_close_path_and_leaving_it_lets_go_of_console_input(
    cx: &mut TestAppContext,
) {
    let quits = Rc::new(Cell::new(0));
    let releases = Rc::new(Cell::new(0));
    let hooks = Hooks {
        quit: {
            let quits = quits.clone();
            Rc::new(move |_| quits.set(quits.get() + 1))
        },
        release_console_input: {
            let releases = releases.clone();
            Rc::new(move |_| releases.set(releases.get() + 1))
        },
        ..Hooks::default()
    };
    let scene = cx
        .update(|cx| scenes::open_shell_with(cx, Setup::lab(), &Options::default(), hooks))
        .expect("shell window");
    cx.run_until_parked();
    settle(cx, &scene);
    // The window is active, then the user goes elsewhere.
    on(cx, &scene, |window, _| window.activate_window());
    let released = releases.get();
    let mut visual = VisualTestContext::from_window(scene.window, cx);
    visual.deactivate_window();
    visual.run_until_parked();
    assert_eq!(releases.get(), released + 1);
    // The window manager asks to close: the close path runs, and the close
    // is allowed.
    assert_eq!(quits.get(), 0);
    let allowed = visual.simulate_close();
    assert!(allowed);
    assert_eq!(quits.get(), 1);
}

/// The whole window over a live session of the fixture server: a pool, a
/// host and a halted guest the session may change. Power commands ask for
/// their confirmation, so choosing one opens a dialog and writes nothing.
fn open_live(cx: &mut TestAppContext) -> (ShellWindow, xenadmin_ui::testing::Live) {
    let mut live = xenadmin_ui::testing::Live::with_vm();
    let mut model = std::mem::take(&mut live.h.model);
    model.preferences.options.review_power = true;
    let options = Options {
        width: 1440.,
        height: 900.,
        ..Options::default()
    };
    let scene = cx
        .update(|cx| scenes::open_shell(cx, Setup::model(model), &options))
        .expect("shell window");
    cx.run_until_parked();
    settle(cx, &scene);
    (scene, live)
}

/// The id of the dialog in front, if any.
fn front(cx: &mut TestAppContext, scene: &ShellWindow) -> Option<String> {
    model(cx, scene, |m| m.front_surface().map(|surface| surface.id))
}

const POWER: &str = command::place::VM_POWER_SUBMENU;

#[gpui_kit::test]
async fn a_context_menu_nests_and_its_submenu_is_reached_by_pointer_and_by_keyboard(
    cx: &mut TestAppContext,
) {
    let (scene, live) = open_live(cx);
    let guest = live.vm();
    let row = model(cx, &scene, |m| {
        nav::rows(m)
            .into_iter()
            .find(|row| row.object.as_ref() == Some(&guest))
            .expect("the guest is in the tree")
    });
    // The secondary button on the guest: its menu, whose top level is
    // short because the power commands are one entry of it.
    on(cx, &scene, |window, cx| window.right_click(id(&row.id), cx));
    let power = popup_index(cx, &scene, POWER);
    let items = submenu_items(cx, &scene, POWER);
    let start = items
        .iter()
        .position(|item| item.id.0 == "vm.start")
        .expect("Start is in the submenu");
    assert_eq!(items[start].disabled, None, "a halted guest can start");
    let top = model(cx, &scene, |m| m.context_menu().iter().flatten().count());
    assert!(top <= 20, "the top level has {top} entries");
    on(cx, &scene, |window, _| {
        // A submenu's items are not on screen until it is opened.
        assert!(window.try_find("submenu").is_none());
        assert_eq!(
            window.within("popup-menu").find(power).role(),
            Some(Role::MenuItem)
        );
    });
    // Pressing the entry opens the submenu beside it, inside the window.
    on(cx, &scene, |window, cx| {
        window.within("popup-menu").click(power, cx)
    });
    on(cx, &scene, |window, _| {
        let submenu = window.find("submenu").bounds();
        let viewport = window.viewport_size();
        assert!(submenu.left() >= px(0.) && submenu.right() <= viewport.width);
        assert!(submenu.top() >= px(0.) && submenu.bottom() <= viewport.height);
        assert_eq!(
            window.within("submenu").find_by_label("Start").role(),
            Some(Role::MenuItem)
        );
    });
    assert!(model(cx, &scene, |m| m.selection.menu.is_some()));
    // Choosing an item of the submenu closes the whole menu and runs the
    // command on the row's object: Start asks for its confirmation.
    on(cx, &scene, |window, cx| {
        window.within("submenu").click(start, cx)
    });
    cx.wait_for(scene.window, Duration::from_secs(2), |window, _| {
        window.try_find("popup-menu").is_none()
    })
    .await;
    settle(cx, &scene);
    assert_eq!(front(cx, &scene).as_deref(), Some("power-confirm"));
    assert!(model(cx, &scene, |m| m.selection.menu.is_none()));
    assert!(!cx.update(|cx| scene.view.read(cx).menu_open()));
    model_side(cx, &scene, |m| {
        let surface = m.front_surface().expect("the confirmation");
        (surface.on_close)(m);
    });
    assert_eq!(front(cx, &scene), None);

    // By the keyboard alone: the menu of the row the cursor is on, Down to
    // its first entry (the power submenu), Right into the submenu, whose
    // first item that can be chosen is Start, and Enter.
    on(cx, &scene, |window, cx| window.click(id(&row.id), cx));
    on(cx, &scene, |window, cx| {
        assert!(focused(window, pane::TREE_KEY));
        window.press("shift-f10", cx);
    });
    assert!(model(cx, &scene, |m| m.selection.menu.is_some()));
    assert_eq!(popup_index(cx, &scene, POWER), 0);
    on(cx, &scene, |window, cx| window.press("down", cx));
    on(cx, &scene, |window, _| {
        assert!(
            window.try_find("submenu").is_some(),
            "the entry is selected"
        );
    });
    on(cx, &scene, |window, cx| window.press("right", cx));
    // Left goes back to the top level and Right in again: the keys stay
    // in the menu.
    on(cx, &scene, |window, cx| window.press("left", cx));
    assert!(model(cx, &scene, |m| m.selection.menu.is_some()));
    on(cx, &scene, |window, cx| window.press("right", cx));
    // Escape inside the submenu ends the whole menu, and the keys are
    // back in the tree.
    on(cx, &scene, |window, cx| window.press("escape", cx));
    cx.wait_for(scene.window, Duration::from_secs(2), |window, _| {
        window.try_find("popup-menu").is_none()
    })
    .await;
    settle(cx, &scene);
    assert!(model(cx, &scene, |m| m.selection.menu.is_none()));
    assert_eq!(front(cx, &scene), None, "Escape chose nothing");
    on(cx, &scene, |window, cx| {
        assert!(
            focused(window, pane::TREE_KEY),
            "the keys are back in the tree"
        );
        window.press("shift-f10", cx);
    });
    for key in ["down", "right", "enter"] {
        on(cx, &scene, |window, cx| window.press(key, cx));
    }
    cx.wait_for(scene.window, Duration::from_secs(2), |window, _| {
        window.try_find("popup-menu").is_none()
    })
    .await;
    settle(cx, &scene);
    assert_eq!(front(cx, &scene).as_deref(), Some("power-confirm"));
    assert!(model(cx, &scene, |m| m.selection.menu.is_none()));
    assert!(!cx.update(|cx| scene.view.read(cx).menu_open()));
    // Nothing was written: the confirmation was not given.
    assert_eq!(live.server.count("Async.VM.start"), 0);
}
