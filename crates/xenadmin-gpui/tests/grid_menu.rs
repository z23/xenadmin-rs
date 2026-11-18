//! The row menu of a grid, with real events.
//!
//! - A secondary press on the header of a grid or on the empty space under
//!   its rows is on no row: it opens no menu and changes no selection (the
//!   Rune test `header_and_blank_space_do_not_invent_a_row_target`).
//! - The Menu key and Shift+F10 open the menu of the keyboard's row at the
//!   row, also in a table whose rows are no tree objects (the snapshots):
//!   there the menu is the table's own commands.

#![cfg(feature = "scenes")]

mod common;

use common::Step::*;
use common::{WIDE, close, front, model, on, open_scene, run_step};
use xenadmin_gpui::kit::testing::{
    ElementSnapshot, TestAppContext, TestQueryExt as _, TestWindowExt as _,
};
use xenadmin_gpui::kit::*;
use xenadmin_gpui::scenes::ShellWindow;

const SEARCH: &str = "search-results";
const SNAPSHOTS: &str = "snapshots-table";

#[track_caller]
fn must(cx: &mut TestAppContext, scene: &ShellWindow, step: common::Step) {
    if let Err(error) = run_step(cx, scene, step) {
        panic!("{error}");
    }
}

/// The elements under a table with a role, from the top.
fn in_table(
    cx: &mut TestAppContext,
    scene: &ShellWindow,
    table: &str,
    role: Role,
) -> Vec<ElementSnapshot> {
    on(cx, scene, |window, _| {
        let mut found: Vec<_> = window
            .elements()
            .into_iter()
            .filter(|element| element.role() == Some(role) && common::under(element, table))
            .collect();
        found.sort_by(|left, right| {
            let (left, right) = (left.bounds().origin, right.bounds().origin);
            (left.y, left.x)
                .partial_cmp(&(right.y, right.x))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        found
    })
}

fn row_keys(cx: &mut TestAppContext, scene: &ShellWindow, table: &str) -> Vec<String> {
    in_table(cx, scene, table, Role::Row)
        .iter()
        .map(|row| common::key_of(row).unwrap_or_default().to_owned())
        .collect()
}

/// The names of the items of the popup menu on screen.
fn popup_labels(cx: &mut TestAppContext, scene: &ShellWindow) -> Vec<String> {
    on(cx, scene, |window, _| {
        window
            .elements()
            .into_iter()
            .filter(|element| element.role() == Some(Role::MenuItem))
            .filter_map(|element| element.label().map(str::to_owned))
            .collect()
    })
}

/// The rows the description marks as selected.
fn selected_rows(cx: &mut TestAppContext, scene: &ShellWindow, rows: &[String]) -> Vec<String> {
    let nodes: Vec<_> = cx
        .update(|cx| xenadmin_gpui::scenes::described(scene.store.read(cx)))
        .into_values()
        .flatten()
        .collect();
    rows.iter()
        .filter(|key| {
            nodes
                .iter()
                .any(|node| node.key.as_ref() == Some(*key) && node.checked == Some(true))
        })
        .cloned()
        .collect()
}

fn secondary_press_at(cx: &mut TestAppContext, scene: &ShellWindow, at: Point<Pixels>) {
    on(cx, scene, |window, cx| {
        common::click_at(window, cx, at, MouseButton::Right);
    });
    common::settle(cx, scene);
}

#[track_caller]
fn press_row(cx: &mut TestAppContext, scene: &ShellWindow, key: &str) {
    let scope = front(cx, scene);
    on(cx, scene, |window, cx| {
        common::press(window, cx, scope.as_deref(), key)
            .map(|_| ())
            .unwrap_or_else(|error| panic!("{error}"));
    });
}

/// What a press must leave alone: the commands' target, the tree's scope,
/// and no menu, the model's or a popup of the Kit.
fn state(cx: &mut TestAppContext, scene: &ShellWindow) -> (Vec<String>, String, bool, Vec<String>) {
    let (target, scope, menu) = model(cx, scene, |m| {
        (
            m.target().vms().iter().map(ToString::to_string).collect(),
            format!("{:?}", m.selection.scope),
            m.selection.menu.is_some(),
        )
    });
    (target, scope, menu, popup_labels(cx, scene))
}

/// The point on the header of the first column, and the point in the empty
/// space under the last row of a table. A grid that does not fill its tab is
/// as high as its rows: the empty space under them is then the tab's.
fn header_and_blank(
    cx: &mut TestAppContext,
    scene: &ShellWindow,
    table: &str,
) -> (Point<Pixels>, Point<Pixels>) {
    let header = in_table(cx, scene, table, Role::ColumnHeader)
        .into_iter()
        .next()
        .expect("a column heading");
    let rows = in_table(cx, scene, table, Role::Row);
    let last = rows.last().expect("a row").bounds();
    let (frame, window_height) = on(cx, scene, |window, _| {
        (
            common::locate(window, None, table)
                .unwrap_or_else(|error| panic!("{error}"))
                .bounds(),
            window.viewport_size().height,
        )
    });
    let inside = frame.bottom() - px(8.);
    let y = if inside > last.bottom() + px(4.) {
        inside
    } else {
        frame.bottom() + px(12.)
    };
    assert!(
        y < window_height - px(80.),
        "nothing is under the {} rows of {table}: {frame:?}",
        rows.len()
    );
    let heading = point(
        header.bounds().left() + px(5.),
        header.bounds().top() + px(10.),
    );
    (heading, point(frame.left() + px(40.), y))
}

#[gpui_kit::test]
fn header_and_blank_space_do_not_invent_a_row_target(cx: &mut TestAppContext) {
    // The Search grid: its rows are objects, the menu is the model's.
    let scene = open_scene(cx, "demo", WIDE);
    must(&mut *cx, &scene, Press("tab-search"));
    must(&mut *cx, &scene, Type("search-text", "Build runner 0"));
    must(&mut *cx, &scene, Press("search-apply"));
    let rows = row_keys(cx, &scene, SEARCH);
    assert!((2..8).contains(&rows.len()), "{rows:?}");
    press_row(cx, &scene, &rows[0]);
    let before = state(cx, &scene);
    assert_eq!(before.0.len(), 1, "one VM is the target: {before:?}");
    assert!(!before.2 && before.3.is_empty(), "{before:?}");
    let (heading, blank) = header_and_blank(cx, &scene, SEARCH);
    secondary_press_at(cx, &scene, heading);
    assert_eq!(state(cx, &scene), before, "a secondary press on the header");
    secondary_press_at(cx, &scene, blank);
    assert_eq!(
        state(cx, &scene),
        before,
        "a secondary press under the rows"
    );

    // A row's menu was open before: the header and the blank space do not
    // open that row's menu again.
    let row = in_table(cx, &scene, SEARCH, Role::Row)[1].bounds().center();
    secondary_press_at(cx, &scene, row);
    let with_menu = state(cx, &scene);
    assert!(with_menu.2 && !with_menu.3.is_empty(), "{with_menu:?}");
    must(&mut *cx, &scene, Key("escape"));
    let closed = state(cx, &scene);
    assert!(!closed.2 && closed.3.is_empty(), "{closed:?}");
    for at in [heading, blank] {
        secondary_press_at(cx, &scene, at);
        assert_eq!(state(cx, &scene), closed, "a secondary press at {at:?}");
    }
    // A grid without rows is all empty space.
    must(&mut *cx, &scene, Type("search-text", "zz no such object"));
    must(&mut *cx, &scene, Press("search-apply"));
    assert_eq!(row_keys(cx, &scene, SEARCH), Vec::<String>::new());
    let empty = state(cx, &scene);
    assert!(!empty.2 && empty.3.is_empty(), "{empty:?}");
    let middle = on(cx, &scene, |window, _| {
        common::locate(window, None, SEARCH)
            .unwrap_or_else(|error| panic!("{error}"))
            .bounds()
            .center()
    });
    secondary_press_at(cx, &scene, middle);
    assert_eq!(
        state(cx, &scene),
        empty,
        "a secondary press in the empty grid"
    );
    close(cx, scene);

    // The snapshots: the rows are no tree objects, the menu is the Kit's
    // popup with the table's own commands.
    let scene = open_scene(cx, "snapshots", WIDE);
    let rows = row_keys(cx, &scene, SNAPSHOTS);
    press_row(cx, &scene, &rows[0]);
    let selected = |cx: &mut TestAppContext, scene: &ShellWindow| selected_rows(cx, scene, &rows);
    assert_eq!(selected(cx, &scene), [rows[0].clone()]);
    let (heading, blank) = header_and_blank(cx, &scene, SNAPSHOTS);
    let row = in_table(cx, &scene, SNAPSHOTS, Role::Row)[1]
        .bounds()
        .center();
    secondary_press_at(cx, &scene, row);
    assert!(!popup_labels(cx, &scene).is_empty());
    assert_eq!(selected(cx, &scene), [rows[1].clone()]);
    must(&mut *cx, &scene, Key("escape"));
    assert!(popup_labels(cx, &scene).is_empty());
    for at in [heading, blank] {
        secondary_press_at(cx, &scene, at);
        assert_eq!(popup_labels(cx, &scene), Vec::<String>::new(), "{at:?}");
        assert_eq!(selected(cx, &scene), [rows[1].clone()], "{at:?}");
    }
    close(cx, scene);
}

#[gpui_kit::test]
fn the_menu_key_opens_the_menu_of_a_row_that_is_no_tree_object_at_the_row(cx: &mut TestAppContext) {
    for key in ["menu", "shift-f10"] {
        let scene = open_scene(cx, "snapshots", WIDE);
        let rows = row_keys(cx, &scene, SNAPSHOTS);
        press_row(cx, &scene, &rows[1]);
        assert!(popup_labels(cx, &scene).is_empty());
        must(&mut *cx, &scene, Key(key));
        assert_eq!(
            popup_labels(cx, &scene),
            [
                "Revert",
                "Edit properties",
                "Create VM",
                "Create template",
                "Delete"
            ],
            "{key}"
        );
        // At the row: the popup starts inside the table, at or under the
        // row the keyboard is on, not at a corner of the window.
        let (popup, row) = on(cx, &scene, |window, _| {
            (
                window.find("popup-menu").bounds(),
                common::locate(window, None, &rows[1])
                    .unwrap_or_else(|error| panic!("{error}"))
                    .bounds(),
            )
        });
        assert!(
            popup.left() >= row.left()
                && popup.left() < row.right()
                && popup.top() >= row.top()
                && popup.top() <= row.bottom() + px(8.),
            "{key}: the popup {popup:?} is not at the row {row:?}"
        );
        // Escape closes it and the keyboard is the table's again, on the
        // same selection.
        must(&mut *cx, &scene, Key("escape"));
        assert!(popup_labels(cx, &scene).is_empty(), "{key}");
        let keyboard_in_table = on(cx, &scene, |window, _| {
            window.elements().into_iter().any(|element| {
                element.focused() == Some(true) && common::under(&element, SNAPSHOTS)
            })
        });
        assert!(keyboard_in_table, "{key}: the table lost the keyboard");
        assert_eq!(selected_rows(cx, &scene, &rows), [rows[1].clone()], "{key}");
        // An item chosen from the menu runs the table's command on the row.
        must(&mut *cx, &scene, Key(key));
        on(cx, &scene, |window, cx| {
            window.within("popup-menu").click(1, cx);
        });
        common::settle(cx, &scene);
        assert!(popup_labels(cx, &scene).is_empty(), "{key}");
        assert_eq!(front(cx, &scene).as_deref(), Some("snapshot-edit"), "{key}");
        close(cx, scene);
    }
}

/// A command of a row menu that cannot run says why in the name of its
/// item, as the items of the menu bar and of an object's menu do: the Kit's
/// menu item has no tooltip and no description. Without a connection every
/// command of the snapshots is unavailable.
#[gpui_kit::test]
fn a_row_menu_item_that_cannot_be_chosen_says_why_in_its_name(cx: &mut TestAppContext) {
    let scene = common::open_scene_with(cx, "snapshots", WIDE, |m| m.demo = false);
    let rows = row_keys(cx, &scene, SNAPSHOTS);
    press_row(cx, &scene, &rows[1]);
    must(&mut *cx, &scene, Key("menu"));
    let names = popup_labels(cx, &scene);
    assert_eq!(names.len(), 5, "{names:?}");
    for name in &names {
        // The label, a dash, the reason.
        let reason = name.split_once(" \u{2014} ").map(|(_, reason)| reason);
        assert!(
            reason.is_some_and(|reason| reason.contains("require a current connection")),
            "`{name}` does not say why it cannot be chosen"
        );
    }
    // Choosing one does nothing.
    on(cx, &scene, |window, cx| {
        window.within("popup-menu").click(1, cx);
    });
    common::settle(cx, &scene);
    assert_eq!(front(cx, &scene), None);
    must(&mut *cx, &scene, Key("escape"));
    // The same with the pointer: the Kit's own popup of the table.
    let at = on(cx, &scene, |window, _| {
        common::locate(window, None, &rows[1])
            .unwrap_or_else(|error| panic!("{error}"))
            .bounds()
            .center()
    });
    secondary_press_at(cx, &scene, at);
    assert_eq!(popup_labels(cx, &scene), names);
    close(cx, scene);
}
