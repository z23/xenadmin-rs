//! The generic renderer, driven through real events on the Kit's headless
//! test platform: one test per kind of block and surface, each proving that
//! what the user does reaches the model through the handler of the
//! description, and that what the model says reaches the widget.
//!
//! The descriptions come from the renderer's lab
//! (`xenadmin_gpui::scenes::gallery`), whose handlers write what happened to
//! a log.
//!
//! These tests do not establish Wayland input, the system clipboard or a
//! screen reader: the platform is the Kit's test platform.
#![cfg(feature = "scenes")]

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use xenadmin_gpui::kit::testing::{
    TestAppContext, TestAppContextExt as _, TestEventExt as _, TestQueryExt as _,
    TestWindowExt as _,
};
use xenadmin_gpui::kit::*;
use xenadmin_gpui::render::{ColumnWidths, surface_scope};
use xenadmin_gpui::scenes::gallery::{self, Body, CONFIRM, DIALOG, Lab, PROPERTIES, TABLE, WIZARD};
use xenadmin_gpui::scenes::{self, Options, SceneWindow, Setup};
use xenadmin_ui::intent::Intent;
use xenadmin_ui::surface::Table;
use xenadmin_ui::tabs::TabId;

fn id(key: &str) -> ElementId {
    ElementId::Name(SharedString::from(key.to_owned()))
}

/// A scene window over a demo model and a lab the caller prepared.
fn open_sized(
    cx: &mut TestAppContext,
    width: f32,
    height: f32,
    prepare: impl FnOnce(&mut Lab),
) -> SceneWindow {
    let setup = Setup::lab();
    prepare(&mut setup.lab.as_ref().expect("lab").borrow_mut());
    let options = Options {
        width,
        height,
        ..Options::default()
    };
    let scene = cx
        .update(|cx| scenes::open(cx, setup, &options))
        .expect("scene window");
    cx.run_until_parked();
    settle(cx, &scene);
    scene
}

fn open(cx: &mut TestAppContext, prepare: impl FnOnce(&mut Lab)) -> SceneWindow {
    open_sized(cx, 1000., 800., prepare)
}

/// Draws until what the last event caused is on screen: the dialog host
/// acts on a store notification, and a widget created in one frame is
/// corrected in the next.
fn settle(cx: &mut TestAppContext, scene: &SceneWindow) {
    for _ in 0..3 {
        cx.update_window(scene.window, |_, window, cx| window.render_frame(cx))
            .expect("window");
        cx.run_until_parked();
    }
}

/// Runs `act` in the window, then settles.
fn on<R>(
    cx: &mut TestAppContext,
    scene: &SceneWindow,
    act: impl FnOnce(&mut Window, &mut App) -> R,
) -> R {
    let result = cx
        .update_window(scene.window, |_, window, cx| act(window, cx))
        .expect("window");
    cx.run_until_parked();
    settle(cx, scene);
    result
}

/// A change made by the model, not by the user.
fn model_side(cx: &mut TestAppContext, scene: &SceneWindow, change: impl FnOnce(&mut Lab)) {
    let lab = scene.lab.clone().expect("lab");
    cx.update(|cx| {
        scene.store.update(cx, |store, cx| {
            store.run(|_| change(&mut lab.borrow_mut()), cx);
        });
    });
    cx.run_until_parked();
    settle(cx, scene);
}

fn lab<R>(scene: &SceneWindow, read: impl FnOnce(&Lab) -> R) -> R {
    read(&scene.lab.as_ref().expect("lab").borrow())
}

fn log(scene: &SceneWindow) -> Vec<String> {
    lab(scene, |lab| lab.log.clone())
}

fn presented(cx: &mut TestAppContext, scene: &SceneWindow) -> Vec<String> {
    cx.update(|cx| {
        let dialogs = scene.view.read(cx).dialogs().clone();
        dialogs.read(cx).presented().to_vec()
    })
}

/// The ids of the guests in the order the lab's table shows them first.
fn guests(cx: &mut TestAppContext, scene: &SceneWindow) -> Vec<String> {
    cx.update(|cx| {
        let store = scene.store.read(cx);
        let inventory = store.model().inventory.clone().expect("demo inventory");
        let mut guests: Vec<_> = inventory.guests().collect();
        guests.sort_by(|left, right| {
            xenadmin_ui::natural_cmp(&left.name, &right.name)
                .then_with(|| left.id.to_string().cmp(&right.id.to_string()))
        });
        guests.iter().map(|vm| vm.id.to_string()).collect()
    })
}

fn row(guest: &str) -> ElementId {
    id(&Table::row_key(TABLE, guest))
}

#[gpui_kit::test]
fn a_button_runs_its_handler_and_a_disabled_one_shows_its_reason(cx: &mut TestAppContext) {
    let scene = open_sized(cx, 1000., 2400., |lab| lab.body = Body::Blocks);
    on(cx, &scene, |window, cx| {
        let primary = window.find("lab-primary");
        assert_eq!(primary.role(), Some(Role::Button));
        assert_eq!(primary.label(), Some("Primary"));
        window.click("lab-primary", cx);
        window.click("lab-danger-button", cx);
        window.click("lab-ghost", cx);
    });
    assert_eq!(
        log(&scene),
        [
            "press:lab-primary",
            "press:lab-danger-button",
            "press:lab-ghost"
        ]
    );

    on(cx, &scene, |window, cx| {
        let disabled = window.find("lab-disabled");
        assert_eq!(disabled.disabled(), Some(true));
        assert_eq!(
            disabled.description(),
            Some("Demo mode cannot change infrastructure."),
            "the reason is the tooltip"
        );
        window.click("lab-disabled", cx);
    });
    assert_eq!(log(&scene).len(), 3, "a disabled button does nothing");
}

#[gpui_kit::test]
fn typing_reaches_the_model_and_the_model_reaches_the_widget(cx: &mut TestAppContext) {
    let scene = open(cx, |lab| lab.open = vec![DIALOG]);
    on(cx, &scene, |window, cx| {
        let name = window.find("lab-name");
        assert_eq!(name.value(), Some("web-01"));
        assert_eq!(
            name.label(),
            Some("Name"),
            "the field is named by its label"
        );
        assert_eq!(
            name.focused(),
            Some(true),
            "the first text field has the keyboard when a surface opens"
        );
        window.click("lab-name", cx);
        window.input("-b", cx);
    });
    assert_eq!(lab(&scene, |lab| lab.name.clone()), "web-01-b");

    // The model changes the text: the widget follows.
    model_side(cx, &scene, |lab| lab.name = "renamed".into());
    on(cx, &scene, |window, _| {
        assert_eq!(window.find("lab-name").value(), Some("renamed"));
    });

    // An empty name: the error line, and the default action disabled with
    // the same reason.
    model_side(cx, &scene, |lab| lab.name.clear());
    on(cx, &scene, |window, cx| {
        let error = window.find("lab-name-error");
        assert_eq!(error.role(), Some(Role::Alert));
        assert_eq!(error.label(), Some("Enter a name for the VM."));
        let copy = window.find("lab-copy");
        assert_eq!(copy.disabled(), Some(true));
        assert_eq!(copy.description(), Some("Enter a name for the VM."));
        window.click("lab-copy", cx);
    });
    assert!(!log(&scene).contains(&"press:lab-copy".to_owned()));

    // Typing again clears the error. Enter in the field runs its submit
    // handler and does not close the dialog.
    on(cx, &scene, |window, cx| {
        window.click("lab-name", cx);
        window.input("db-01", cx);
    });
    on(cx, &scene, |window, cx| {
        assert!(window.try_find("lab-name-error").is_none());
        window.press("enter", cx);
    });
    assert_eq!(lab(&scene, |lab| lab.name.clone()), "db-01");
    assert_eq!(log(&scene), ["submit:lab-name"]);
    assert_eq!(presented(cx, &scene), [DIALOG]);
    on(cx, &scene, |window, cx| {
        assert!(window.has_active_dialog(cx));
        // A multi-line field.
        let notes = window.find("lab-notes");
        assert!(notes.visible());
        window.click("lab-notes", cx);
        window.input("first", cx);
        window.press("enter", cx);
        window.input("second", cx);
    });
    assert_eq!(lab(&scene, |lab| lab.notes.clone()), "first\nsecond");
    assert_eq!(presented(cx, &scene), [DIALOG]);
}

#[gpui_kit::test]
async fn checkbox_select_and_choices_write_the_model(cx: &mut TestAppContext) {
    let scene = open_sized(cx, 1000., 1400., |lab| lab.open = vec![DIALOG]);
    on(cx, &scene, |window, cx| {
        let start = window.find("lab-start");
        assert_eq!(start.role(), Some(Role::CheckBox));
        assert_eq!(start.checked(), Some(true));
        window.click("lab-start", cx);
    });
    assert!(!lab(&scene, |lab| lab.start));
    on(cx, &scene, |window, cx| {
        assert_eq!(window.find("lab-start").checked(), Some(false));
        // A disabled checkbox keeps its state and carries its reason.
        let locked = window.find("lab-locked");
        assert_eq!(locked.disabled(), Some(true));
        assert_eq!(
            window.find("lab-locked-reason").description(),
            Some("The pool policy keeps the addresses.")
        );
        window.click("lab-locked", cx);
        assert_eq!(window.find("lab-locked").checked(), Some(true));
    });

    // A radio choice.
    on(cx, &scene, |window, cx| {
        assert_eq!(window.find("lab-mode").role(), Some(Role::RadioGroup));
        assert_eq!(window.find("lab-mode-full").checked(), Some(true));
        window.click("lab-mode-fast", cx);
    });
    assert_eq!(lab(&scene, |lab| lab.mode.clone()), "fast");
    on(cx, &scene, |window, cx| {
        assert_eq!(window.find("lab-mode-fast").checked(), Some(true));
        assert_eq!(window.find("lab-mode-full").checked(), Some(false));
        // An option that cannot be picked.
        window.click("lab-mode-linked", cx);
    });
    assert_eq!(lab(&scene, |lab| lab.mode.clone()), "fast");

    // A select control: the second option, by keyboard.
    on(cx, &scene, |window, cx| {
        assert_eq!(window.find("lab-template").value(), Some("Debian 12"));
        window.within("lab-template").click("input", cx);
    });
    on(cx, &scene, |window, cx| {
        window.press("down", cx);
        window.press("enter", cx);
    });
    let chosen = scene.lab.clone().expect("lab");
    cx.wait_for(scene.window, Duration::from_secs(2), move |_, _| {
        chosen.borrow().template.as_deref() == Some("alma")
    })
    .await;
    settle(cx, &scene);
    on(cx, &scene, |window, _| {
        assert_eq!(window.find("lab-template").value(), Some("AlmaLinux 9"));
    });
    // The model picks another option: the select shows it.
    model_side(cx, &scene, |lab| lab.template = Some("debian".into()));
    on(cx, &scene, |window, _| {
        assert_eq!(window.find("lab-template").value(), Some("Debian 12"));
    });
}

#[gpui_kit::test]
async fn segmented_and_drop_down_choices_write_the_model(cx: &mut TestAppContext) {
    let scene = open(cx, |lab| {
        lab.open = vec![WIZARD];
        lab.step = 2;
    });
    on(cx, &scene, |window, cx| {
        assert_eq!(window.find("lab-view-list").checked(), Some(true));
        window.click("lab-view-tree", cx);
    });
    assert_eq!(lab(&scene, |lab| lab.view.clone()), "tree");
    on(cx, &scene, |window, cx| {
        assert_eq!(window.find("lab-view-tree").checked(), Some(true));
        assert_eq!(window.find("lab-view-list").checked(), Some(false));
        window.within("lab-storage").click("input", cx);
    });
    on(cx, &scene, |window, cx| {
        window.press("down", cx);
        window.press("down", cx);
        window.press("enter", cx);
    });
    let chosen = scene.lab.clone().expect("lab");
    cx.wait_for(scene.window, Duration::from_secs(2), move |_, _| {
        chosen.borrow().storage.is_some()
    })
    .await;
    assert_eq!(
        lab(&scene, |lab| lab.storage.clone()).as_deref(),
        Some("nfs")
    );
}

#[gpui_kit::test]
fn a_secret_field_is_masked_and_its_buffer_is_cleared(cx: &mut TestAppContext) {
    let scene = open(cx, |lab| lab.open = vec![DIALOG]);
    let scope = surface_scope(DIALOG);
    on(cx, &scene, |window, cx| {
        window.click("lab-password", cx);
        window.input("hunter2", cx);
    });
    assert_eq!(
        lab(&scene, |lab| lab.password.expose().to_owned()),
        "hunter2"
    );
    let state = cx.update(|cx| {
        let ui = scene.view.read(cx).ui().clone();
        ui.widgets().text_input(&scope, "lab-password")
    });
    let state = state.expect("the password field has a widget");
    on(cx, &scene, |window, cx| {
        assert_eq!(
            window.find("lab-password").value(),
            None,
            "a masked field exposes no value"
        );
        assert_eq!(state.read(cx).value().as_ref(), "hunter2");
    });

    // The model clears the secret: the widget's buffer is emptied.
    model_side(cx, &scene, |lab| lab.password.clear());
    cx.update(|cx| assert_eq!(state.read(cx).value().as_ref(), ""));

    // The surface closes with a secret typed: the buffer is emptied before
    // the widget is dropped.
    on(cx, &scene, |window, cx| {
        window.click("lab-password", cx);
        window.input("again", cx);
    });
    cx.update(|cx| assert_eq!(state.read(cx).value().as_ref(), "again"));
    model_side(cx, &scene, |lab| lab.open.clear());
    cx.update(|cx| {
        assert_eq!(state.read(cx).value().as_ref(), "");
        let ui = scene.view.read(cx).ui().clone();
        assert!(ui.widgets().text_input(&scope, "lab-password").is_none());
        assert!(!ui.widgets().scopes().contains(&scope));
    });
}

#[gpui_kit::test]
fn a_table_selects_activates_sorts_and_opens_its_row_menu(cx: &mut TestAppContext) {
    let scene = open(cx, |lab| lab.body = Body::Table);
    let guests = guests(cx, &scene);
    let (first, third) = (guests[0].clone(), guests[2].clone());

    on(cx, &scene, |window, cx| {
        assert!(window.find(id(TABLE)).visible());
        assert_eq!(window.find(row(&first)).selected(), Some(false));
        window.click(row(&third), cx);
    });
    assert_eq!(lab(&scene, |lab| lab.selected.clone()), Some(third.clone()));
    cx.update(|cx| {
        let target = scene.store.read(cx).model().target();
        assert_eq!(
            target.single().map(|object| object.id.clone()),
            Some(third.clone())
        );
    });
    on(cx, &scene, |window, cx| {
        assert_eq!(window.find(row(&third)).selected(), Some(true));
        // Activation: a double click, and Enter on the selected row.
        window.double_click(row(&third), cx);
    });
    on(cx, &scene, |window, cx| window.press("enter", cx));
    let activations = log(&scene)
        .iter()
        .filter(|entry| **entry == format!("activate:{third}"))
        .count();
    assert_eq!(activations, 2);

    // The arrow keys move the selection, and the model hears of it.
    on(cx, &scene, |window, cx| window.press("up", cx));
    assert_eq!(
        lab(&scene, |lab| lab.selected.clone()),
        Some(guests[1].clone())
    );

    // A press on a sortable header asks the model for the order; the model
    // sorts, and the selection stays on its row.
    on(cx, &scene, |window, cx| {
        let header = window.find(id(&Table::column_key(TABLE, "name")));
        assert_eq!(header.role(), Some(Role::ColumnHeader));
        assert_eq!(header.value(), Some("ascending"));
        window.click(id(&Table::column_key(TABLE, "name")), cx);
    });
    assert_eq!(
        lab(&scene, |lab| lab.sort.clone()),
        ("name".to_owned(), true)
    );
    on(cx, &scene, |window, cx| {
        let header = window.find(id(&Table::column_key(TABLE, "name")));
        assert_eq!(header.value(), Some("descending"));
        window.click(id(&Table::column_key(TABLE, "memory")), cx);
    });
    assert_eq!(
        lab(&scene, |lab| lab.sort.clone()),
        ("memory".to_owned(), false),
        "another column starts ascending"
    );
    assert_eq!(
        lab(&scene, |lab| lab.selected.clone()),
        Some(guests[1].clone())
    );

    // The model selects a row: the table shows it selected.
    model_side(cx, &scene, |lab| {
        lab.sort = ("name".into(), false);
        lab.selected = Some(first.clone());
    });
    on(cx, &scene, |window, _| {
        assert_eq!(window.find(row(&first)).selected(), Some(true));
        assert_eq!(window.find(row(&guests[1])).selected(), Some(false));
    });

    // A right click selects the row and opens the menu of the table's
    // commands, resolved on that row.
    let selections_before = log(&scene).len();
    cx.update_window(scene.window, |_, window, cx| {
        window.right_click(row(&third), cx);
    })
    .expect("window");
    cx.run_until_parked();
    settle(cx, &scene);
    assert_eq!(lab(&scene, |lab| lab.selected.clone()), Some(third.clone()));
    assert_eq!(
        log(&scene).len(),
        selections_before + 1,
        "one selection, not two"
    );
    cx.update_window(scene.window, |_, window, cx| {
        let item = window.find_by_label("Copy UUID");
        assert_eq!(item.role(), Some(Role::MenuItem));
        window.within("popup-menu").click(0usize, cx);
    })
    .expect("window");
    cx.run_until_parked();
    cx.update(|cx| {
        let intents = scene.store.read(cx).recorded_intents().to_vec();
        assert!(
            intents
                .iter()
                .any(|intent| matches!(intent, Intent::CopyText(text) if *text == third)),
            "the command ran on the row that was right-clicked: {intents:?}"
        );
    });
}

#[gpui_kit::test]
fn a_multi_selection_is_kept_by_id(cx: &mut TestAppContext) {
    let scene = open(cx, |lab| lab.body = Body::MultiTable);
    let guests = guests(cx, &scene);
    let control = Modifiers {
        control: true,
        ..Modifiers::default()
    };
    let shift = Modifiers {
        shift: true,
        ..Modifiers::default()
    };

    on(cx, &scene, |window, cx| window.click(row(&guests[1]), cx));
    assert_eq!(lab(&scene, |lab| lab.marked.clone()), [guests[1].clone()]);
    on(cx, &scene, |window, cx| {
        window.click_with_modifiers(row(&guests[3]), control, cx);
    });
    assert_eq!(
        lab(&scene, |lab| lab.marked.clone()),
        [guests[1].clone(), guests[3].clone()]
    );
    // Shift extends from the row pressed last.
    on(cx, &scene, |window, cx| {
        window.click_with_modifiers(row(&guests[5]), shift, cx);
    });
    assert_eq!(
        lab(&scene, |lab| lab.marked.clone()),
        [guests[3].clone(), guests[4].clone(), guests[5].clone()]
    );
    // Control on a selected row takes it out.
    on(cx, &scene, |window, cx| {
        window.click_with_modifiers(row(&guests[4]), control, cx);
    });
    let marked = [guests[3].clone(), guests[5].clone()];
    assert_eq!(lab(&scene, |lab| lab.marked.clone()), marked);
    cx.update(|cx| {
        let target = scene.store.read(cx).model().target();
        assert_eq!(target.objects.len(), 2, "the model's target is both rows");
    });

    // The rows change order: the selection is the same rows.
    on(cx, &scene, |window, cx| {
        window.click(id(&Table::column_key(TABLE, "name")), cx);
    });
    assert_eq!(
        lab(&scene, |lab| lab.sort.clone()),
        ("name".to_owned(), true)
    );
    assert_eq!(lab(&scene, |lab| lab.marked.clone()), marked);
    model_side(cx, &scene, |lab| lab.sort = ("name".into(), false));
    assert_eq!(lab(&scene, |lab| lab.marked.clone()), marked);

    // A plain click selects one row again.
    on(cx, &scene, |window, cx| window.click(row(&guests[0]), cx));
    assert_eq!(lab(&scene, |lab| lab.marked.clone()), [guests[0].clone()]);
}

#[gpui_kit::test]
fn a_dragged_column_width_is_reported_and_kept(cx: &mut TestAppContext) {
    let scene = open(cx, |lab| lab.body = Body::Table);
    // Where the shell keeps the widths.
    let stored: Rc<RefCell<HashMap<(String, String), f32>>> = Rc::default();
    cx.update(|cx| {
        let ui = scene.view.read(cx).ui().clone();
        let (reading, writing) = (stored.clone(), stored.clone());
        ui.persist_column_widths(ColumnWidths {
            get: Rc::new(move |_, table, column| {
                reading
                    .borrow()
                    .get(&(table.to_owned(), column.to_owned()))
                    .copied()
            }),
            set: Rc::new(move |_, table, column, width| {
                writing
                    .borrow_mut()
                    .insert((table.to_owned(), column.to_owned()), width);
            }),
        });
    });
    settle(cx, &scene);
    let state = ("col-header", 1usize);
    on(cx, &scene, |window, cx| {
        let header = window.find(state).bounds();
        assert_eq!(header.size.width, px(140.), "the declared width");
        // The first column has no declared width: it has the rest.
        let name = window.find(("col-header", 0usize)).bounds();
        assert!(name.size.width > px(400.), "{name:?}");
        let edge = point(header.right() - px(1.), header.center().y);
        window.drag(edge, edge + point(px(60.), px(0.)), cx);
    });
    let width = stored
        .borrow()
        .get(&(TABLE.to_owned(), "state".to_owned()))
        .copied()
        .expect("the width was reported");
    assert!((width - 200.).abs() < 2., "{width}");
    assert_eq!(stored.borrow().len(), 1, "only the column that was dragged");
    on(cx, &scene, |window, _| {
        let header = window.find(state).bounds();
        assert!(
            (header.size.width - px(width)).abs() < px(1.),
            "kept: {header:?}"
        );
    });
    // The rows change: the width stays.
    model_side(cx, &scene, |lab| lab.sort = ("name".into(), true));
    on(cx, &scene, |window, _| {
        let header = window.find(state).bounds();
        assert!((header.size.width - px(width)).abs() < px(1.), "{header:?}");
    });
}

#[gpui_kit::test]
fn an_empty_table_shows_its_empty_text(cx: &mut TestAppContext) {
    let scene = open(cx, |lab| {
        lab.body = Body::Table;
        lab.empty = true;
    });
    on(cx, &scene, |window, _| {
        let empty = window.find(id(&format!("{TABLE}-empty")));
        assert_eq!(empty.label(), Some("No virtual machines match."));
        assert!(empty.visible());
    });
    // Rows arrive: the text goes and the rows are there.
    model_side(cx, &scene, |lab| lab.empty = false);
    let guests = guests(cx, &scene);
    on(cx, &scene, |window, _| {
        assert!(window.try_find(id(&format!("{TABLE}-empty"))).is_none());
        assert!(window.find(row(&guests[0])).visible());
    });
}

#[gpui_kit::test]
fn the_dialogs_are_the_surfaces_of_the_model(cx: &mut TestAppContext) {
    let scene = open(cx, |_| {});
    on(cx, &scene, |window, cx| {
        assert!(!window.has_active_dialog(cx))
    });
    assert!(presented(cx, &scene).is_empty());

    // The model opens a surface: its dialog appears.
    model_side(cx, &scene, |lab| lab.open.push(DIALOG));
    assert_eq!(presented(cx, &scene), [DIALOG]);
    on(cx, &scene, |window, cx| {
        assert!(window.has_active_dialog(cx));
        let surface = window.find(DIALOG);
        assert_eq!(surface.label(), Some("Copy VM"));
        assert!(window.find("experimental-notice").visible());
        // Escape runs the surface's close handler.
        window.press("escape", cx);
    });
    assert_eq!(log(&scene), ["close:lab-dialog"]);
    assert!(presented(cx, &scene).is_empty());
    on(cx, &scene, |window, cx| {
        assert!(!window.has_active_dialog(cx));
        assert!(window.try_find(DIALOG).is_none());
    });

    // The close button.
    model_side(cx, &scene, |lab| lab.open.push(DIALOG));
    on(cx, &scene, |window, cx| window.click("close", cx));
    assert_eq!(log(&scene), ["close:lab-dialog", "close:lab-dialog"]);
    assert!(presented(cx, &scene).is_empty());

    // A press on the overlay.
    model_side(cx, &scene, |lab| lab.open.push(DIALOG));
    on(cx, &scene, |window, cx| {
        let position = point(px(12.), px(400.));
        window.pointer_move(position, None, cx);
        window.pointer_down(position, MouseButton::Left, cx);
        window.pointer_up(position, MouseButton::Left, cx);
    });
    assert_eq!(log(&scene).len(), 3);
    assert!(presented(cx, &scene).is_empty());

    // The model closes a surface itself: the dialog goes and the close
    // handler is not run.
    model_side(cx, &scene, |lab| lab.open.push(DIALOG));
    assert_eq!(presented(cx, &scene), [DIALOG]);
    model_side(cx, &scene, |lab| lab.open.clear());
    assert_eq!(log(&scene).len(), 3);
    assert!(presented(cx, &scene).is_empty());
    on(cx, &scene, |window, cx| {
        assert!(!window.has_active_dialog(cx))
    });
}

#[gpui_kit::test]
fn stacked_surfaces_keep_the_order_of_the_model(cx: &mut TestAppContext) {
    let scene = open(cx, |lab| lab.open = vec![PROPERTIES]);
    assert_eq!(presented(cx, &scene), [PROPERTIES]);
    // A button of the surface opens another in front of it.
    on(cx, &scene, |window, cx| window.click("lab-apply", cx));
    assert_eq!(presented(cx, &scene), [PROPERTIES, CONFIRM]);
    on(cx, &scene, |window, cx| {
        assert_eq!(window.find_all("dialog").len(), 2);
        // Escape closes the one in front only.
        window.press("escape", cx);
    });
    assert_eq!(presented(cx, &scene), [PROPERTIES]);
    assert_eq!(lab(&scene, |lab| lab.open.clone()), [PROPERTIES]);
    on(cx, &scene, |window, _| {
        assert_eq!(window.find_all("dialog").len(), 1);
        assert!(window.find(PROPERTIES).visible());
    });

    // The model drops the surface behind while one is in front: the one in
    // front stays, alone.
    model_side(cx, &scene, |lab| lab.open = vec![PROPERTIES, CONFIRM]);
    model_side(cx, &scene, |lab| lab.open = vec![CONFIRM]);
    assert_eq!(presented(cx, &scene), [CONFIRM]);
    on(cx, &scene, |window, cx| {
        assert_eq!(window.find_all("dialog").len(), 1);
        assert!(window.try_find(PROPERTIES).is_none());
        // Its default action closes everything in the model.
        window.click("lab-confirm-yes", cx);
    });
    assert!(presented(cx, &scene).is_empty());
    on(cx, &scene, |window, cx| {
        assert!(!window.has_active_dialog(cx))
    });
}

#[gpui_kit::test]
fn a_wizard_advances_and_goes_back(cx: &mut TestAppContext) {
    let scene = open(cx, |lab| lab.open = vec![WIZARD]);
    on(cx, &scene, |window, cx| {
        let steps = window.find("lab-wizard-steps");
        assert_eq!(steps.label(), Some("Step 1 of 4: Template"));
        let back = window.find("lab-back");
        assert_eq!(back.disabled(), Some(true));
        assert_eq!(back.description(), Some("This is the first step."));
        window.click("lab-next", cx);
    });
    assert_eq!(lab(&scene, |lab| lab.step), 1);
    on(cx, &scene, |window, cx| {
        assert_eq!(
            window.find("lab-wizard-steps").label(),
            Some("Step 2 of 4: Name")
        );
        assert!(window.find("lab-name").visible());
        // The page of the step before is gone.
        assert!(window.try_find("lab-template").is_none());
        // Enter does not close a wizard or move it on.
        window.press("enter", cx);
    });
    assert_eq!(lab(&scene, |lab| lab.step), 1);
    assert_eq!(presented(cx, &scene), [WIZARD]);

    // A step that is not valid cannot be left forward.
    model_side(cx, &scene, |lab| lab.name.clear());
    on(cx, &scene, |window, cx| {
        let next = window.find("lab-next");
        assert_eq!(next.disabled(), Some(true));
        assert_eq!(next.description(), Some("Enter a name for the VM."));
        window.click("lab-next", cx);
        window.click("lab-back", cx);
    });
    assert_eq!(lab(&scene, |lab| lab.step), 0);

    // The last step has the surface's own last button.
    model_side(cx, &scene, |lab| {
        lab.name = "web-02".into();
        lab.step = 3;
    });
    on(cx, &scene, |window, cx| {
        assert!(window.try_find("lab-next").is_none());
        assert!(window.find("lab-review").visible());
        window.click("lab-create", cx);
    });
    assert_eq!(log(&scene), ["press:lab-create"]);
}

#[gpui_kit::test]
fn a_properties_dialog_switches_pages(cx: &mut TestAppContext) {
    let scene = open(cx, |lab| lab.open = vec![PROPERTIES]);
    on(cx, &scene, |window, cx| {
        let pages = window.find("lab-properties-pages");
        assert_eq!(pages.role(), Some(Role::TabList));
        assert_eq!(pages.value(), Some("General"));
        assert_eq!(
            window.find("lab-properties-pages-general").selected(),
            Some(true)
        );
        assert!(window.find("lab-name").visible());
        window.click("lab-properties-pages-cpu", cx);
    });
    assert_eq!(lab(&scene, |lab| lab.page.clone()), "cpu");
    on(cx, &scene, |window, cx| {
        assert_eq!(
            window.find("lab-properties-pages-cpu").selected(),
            Some(true)
        );
        assert!(window.try_find("lab-name").is_none());
        assert!(window.find("lab-view").visible());
        // A page that cannot be opened carries its reason.
        let ha = window.find("lab-properties-pages-ha");
        assert_eq!(ha.description(), Some("The pool has no high availability."));
        window.click("lab-properties-pages-ha", cx);
    });
    assert_eq!(lab(&scene, |lab| lab.page.clone()), "cpu");

    // The keyboard reaches a page: Tab stops on the list, Enter opens.
    model_side(cx, &scene, |lab| lab.page = "general".into());
    on(cx, &scene, |window, cx| {
        let mut presses = 0;
        while window.find("lab-properties-pages-advanced").focused() != Some(true) {
            window.press("tab", cx);
            presses += 1;
            assert!(presses < 40, "Tab never reached the Advanced page");
        }
        window.press("enter", cx);
    });
    assert_eq!(lab(&scene, |lab| lab.page.clone()), "advanced");
}

#[gpui_kit::test]
fn the_keyboard_walks_a_dialog_and_presses_its_buttons(cx: &mut TestAppContext) {
    let scene = open_sized(cx, 1000., 1400., |lab| lab.open = vec![DIALOG]);
    on(cx, &scene, |window, cx| {
        assert_eq!(window.find("lab-name").focused(), Some(true));
        window.press("tab", cx);
        assert_eq!(window.find("lab-password").focused(), Some(true));
        window.press("shift-tab", cx);
        assert_eq!(window.find("lab-name").focused(), Some(true));
        // Tab stays inside the dialog and reaches its footer.
        let mut presses = 0;
        while window.find("lab-cancel").focused() != Some(true) {
            window.press("tab", cx);
            presses += 1;
            assert!(presses < 40, "Tab never reached the Cancel button");
        }
        window.press("tab", cx);
        assert_eq!(window.find("lab-copy").focused(), Some(true));
        // Enter on a focused button presses it.
        window.press("enter", cx);
    });
    assert_eq!(log(&scene), ["press:lab-copy"]);
    assert_eq!(
        presented(cx, &scene),
        [DIALOG],
        "the model did not close it"
    );
    on(cx, &scene, |window, cx| {
        window.press("shift-tab", cx);
        assert_eq!(window.find("lab-cancel").focused(), Some(true));
        window.press("enter", cx);
    });
    assert_eq!(log(&scene), ["press:lab-copy", "close:lab-dialog"]);
    assert!(presented(cx, &scene).is_empty());
}

#[gpui_kit::test]
fn a_review_shows_its_lines_and_goes_stale(cx: &mut TestAppContext) {
    let scene = open_sized(cx, 1000., 1400., |lab| lab.open = vec![DIALOG]);
    on(cx, &scene, |window, _| {
        let review = window.find("lab-review");
        assert_eq!(review.label(), Some("Review"));
        assert_eq!(
            window
                .find(id(&format!("{DIALOG}/lab-review/lines/#0")))
                .label(),
            Some("Copy web-01 to web-01.")
        );
        assert_eq!(
            window
                .find(id(&format!("{DIALOG}/lab-review/warnings/#0")))
                .label(),
            Some("The original keeps running while its disks are copied.")
        );
        let affected = window.find(id(&format!("{DIALOG}/lab-review/affected/#1")));
        assert_eq!(affected.label(), Some("UUID"));
        assert_eq!(
            affected.value(),
            Some("6f1c0a52-8d3e-4b71-9c5a-2e7d0f4b9a13")
        );
        assert!(window.try_find("lab-review-stale").is_none());
        // The footer note.
        assert_eq!(
            window.find(id(&format!("{DIALOG}/footer/#0"))).label(),
            Some("Signed in as root. The role allows this operation.")
        );
    });
    model_side(cx, &scene, |lab| lab.stale = true);
    on(cx, &scene, |window, _| {
        let stale = window.find("lab-review-stale");
        assert_eq!(stale.role(), Some(Role::Alert));
        assert_eq!(
            stale.label(),
            Some("The inventory changed after this review was prepared. Review it again.")
        );
    });
    // A copyable value asks the model to copy it.
    on(cx, &scene, |window, cx| {
        window.click(id(&format!("{DIALOG}/lab-review/affected/#1-copy")), cx);
    });
    cx.update(|cx| {
        let intents = scene.store.read(cx).recorded_intents().to_vec();
        assert!(intents.iter().any(|intent| matches!(
            intent,
            Intent::CopyText(text) if text == "6f1c0a52-8d3e-4b71-9c5a-2e7d0f4b9a13"
        )));
    });
}

#[gpui_kit::test]
fn texts_notices_outcomes_and_a_missing_custom_element(cx: &mut TestAppContext) {
    let scene = open_sized(cx, 1000., 2400., |lab| lab.body = Body::Blocks);
    on(cx, &scene, |window, cx| {
        let heading = window.find("tab/#0");
        assert_eq!(heading.role(), Some(Role::Heading));
        assert_eq!(heading.label(), Some("Every block"));
        assert_eq!(
            window.find("lab-caption").label(),
            Some("A caption in the muted tone.")
        );
        let alert = window.find("lab-alert");
        assert_eq!(
            alert.role(),
            Some(Role::Alert),
            "an alert text takes the alert role"
        );
        assert_eq!(window.find("lab-danger").role(), Some(Role::Alert));
        let warning = window.find("lab-warning");
        assert_eq!(warning.role(), Some(Role::Status));
        assert_eq!(warning.label(), Some("Update pending"));

        // A property with a key and a copy button.
        let uuid = window.find("lab-uuid");
        assert_eq!(uuid.label(), Some("UUID"));
        assert_eq!(uuid.value(), Some("6f1c0a52-8d3e-4b71-9c5a-2e7d0f4b9a13"));

        // Outcomes: the state is the value, the actions are buttons.
        assert_eq!(window.find("lab-running").value(), Some("running 40%"));
        assert_eq!(window.find("lab-waiting").value(), Some("running"));
        for (key, state) in [
            ("lab-succeeded", "succeeded"),
            ("lab-failed", "failed"),
            ("lab-partial", "partial"),
            ("lab-unknown", "unknown"),
            ("lab-cancelled", "cancelled"),
        ] {
            let outcome = window.find(key);
            assert_eq!(outcome.role(), Some(Role::Status));
            assert_eq!(outcome.value(), Some(state));
        }
        assert_eq!(
            window.find("lab-unknown").label(),
            Some("The outcome is unknown")
        );
        window.click("lab-recheck", cx);
        window.click("lab-acknowledge", cx);
        window.click("lab-cancel-task", cx);

        // Nothing draws `lab-missing`: the placeholder names it.
        let custom = window.find("lab-custom");
        assert!(
            custom
                .label()
                .is_some_and(|label| label.contains("lab-missing")),
            "{custom:?}"
        );
    });
    assert_eq!(
        log(&scene),
        [
            "press:lab-recheck",
            "press:lab-acknowledge",
            "press:lab-cancel-task"
        ]
    );

    // Once something is registered under the name, it is drawn instead.
    cx.update(|cx| {
        let ui = scene.view.read(cx).ui().clone();
        ui.register_custom("lab-missing", |custom, _, _, _| {
            div()
                .child(format!("drawn with {}", custom.params))
                .into_any_element()
        });
        scene.store.update(cx, |store, cx| store.run(|_| {}, cx));
    });
    settle(cx, &scene);
    on(cx, &scene, |window, _| {
        assert!(window.try_find("lab-custom").is_none())
    });
}

#[gpui_kit::test]
fn a_section_folds_and_columns_stack_when_narrow(cx: &mut TestAppContext) {
    let scene = open_sized(cx, 1000., 2400., |lab| lab.body = Body::Blocks);
    on(cx, &scene, |window, cx| {
        let folded = window.find("lab-folded");
        assert_eq!(folded.expanded(), Some(false));
        assert!(window.try_find("lab-folded-text").is_none());
        window.click("lab-folded", cx);
    });
    on(cx, &scene, |window, cx| {
        assert_eq!(window.find("lab-folded").expanded(), Some(true));
        assert!(window.find("lab-folded-text").visible());
        // A section that does not fold has its content and no state.
        assert_eq!(window.find("lab-operations").expanded(), None);
        window.click("lab-folded", cx);
    });
    on(cx, &scene, |window, _| {
        assert!(window.try_find("lab-folded-text").is_none());
        // Wide: the two columns are side by side.
        let left = window.find("lab-name").bounds();
        let right = window.find("lab-uuid").bounds();
        assert!(right.origin.x > left.origin.x + left.size.width - px(1.));
        assert!((right.origin.y - left.origin.y).abs() < px(40.));
    });
    cx.simulate_window_resize(scene.window, size(px(520.), px(2400.)));
    settle(cx, &scene);
    on(cx, &scene, |window, _| {
        // Narrow: the second column is under the first.
        let left = window.find("lab-name").bounds();
        let right = window.find("lab-uuid").bounds();
        assert!(right.origin.y > left.origin.y + left.size.height);
        assert!(right.origin.x < left.origin.x + px(40.));
    });
}

#[gpui_kit::test]
fn the_tab_strip_and_the_local_action_row_drive_the_model(cx: &mut TestAppContext) {
    let scene = open(cx, |lab| lab.body = Body::Table);
    on(cx, &scene, |window, cx| {
        let header = window.find("object-header");
        assert_eq!(header.label(), Some("Demonstration pool"));
        let general = window.find("tab-general");
        assert_eq!(general.role(), Some(Role::Tab));
        assert_eq!(general.selected(), Some(true));

        // The local action row: commands on the model's target. A command
        // that is not available carries the model's reason.
        let back = window.find("nav.back");
        assert_eq!(back.disabled(), Some(true));
        assert_eq!(
            back.description(),
            Some("There is no earlier selection to go back to.")
        );
        window.click("nav.back", cx);
        window.click("view.zoom-in", cx);
    });
    cx.update(|cx| {
        let model = scene.store.read(cx).model();
        assert_eq!(model.active_tab(), TabId::General);
        assert!(
            model.preferences.options.interface_scale > 100,
            "the command ran on the model"
        );
    });

    on(cx, &scene, |window, cx| window.click("tab-memory", cx));
    cx.update(|cx| {
        assert_eq!(scene.store.read(cx).model().active_tab(), TabId::Memory);
    });
    on(cx, &scene, |window, cx| {
        assert_eq!(window.find("tab-memory").selected(), Some(true));
        assert_eq!(window.find("tab-general").selected(), Some(false));
        assert_eq!(window.find("tab-body").label(), Some("Memory"));
        // The keyboard: Tab reaches a tab, Enter chooses it.
        let mut presses = 0;
        while window.find("tab-storage").focused() != Some(true) {
            window.press("tab", cx);
            presses += 1;
            assert!(presses < 40, "Tab never reached the Storage tab");
        }
        window.press("enter", cx);
    });
    cx.update(|cx| {
        assert_eq!(scene.store.read(cx).model().active_tab(), TabId::Storage);
    });
}

#[gpui_kit::test]
fn a_tab_nothing_provides_says_so(cx: &mut TestAppContext) {
    let scene = open(cx, |lab| lab.body = Body::None);
    on(cx, &scene, |window, _| {
        let empty = window.find("tab-empty");
        assert_eq!(
            empty.label(),
            Some("The General tab is not available for this selection yet.")
        );
        assert!(window.try_find("local-actions").is_none());
    });
    // A bespoke tab body nothing is registered for.
    model_side(cx, &scene, |lab| lab.body = Body::Custom);
    on(cx, &scene, |window, _| {
        let chart = window.find("lab-chart");
        assert!(
            chart
                .label()
                .is_some_and(|label| label.contains("lab-chart"))
        );
    });
}

/// Every scene the tool knows renders, and every key its description
/// requires is an element of the frame.
#[test]
fn every_scene_renders_with_the_keys_of_its_description() {
    let names = scenes::names();
    assert!(names.contains(&"demo"));
    assert!(gallery::SCENES.iter().all(|name| names.contains(name)));
    for name in names {
        for (width, height) in [(1440., 900.), (800., 600.)] {
            let options = Options {
                width,
                height,
                ..Options::default()
            };
            let output = scenes::render(name, &options, false)
                .unwrap_or_else(|error| panic!("{name} at {width}x{height}: {error}"));
            assert!(output.image.is_none());
            let missing = scenes::missing_keys(&output.tree);
            assert!(
                missing.is_empty(),
                "{name} at {width}x{height} lacks {missing:?}"
            );
            let elements = output.tree["elements"].as_array().expect("elements");
            assert!(
                elements
                    .iter()
                    .any(|element| element["id"] == "object-header"),
                "{name}: the header is in the tree"
            );
        }
    }
}

/// The tree of a scene carries what the model describes even where the Kit
/// exposes none of it.
#[test]
fn the_tree_merges_the_frame_with_the_description() {
    let output = scenes::render("gallery-dialog", &Options::default(), false).expect("render");
    let elements = output.tree["elements"].as_array().expect("elements");
    let find = |key: &str| {
        elements
            .iter()
            .find(|element| element["id"] == key)
            .unwrap_or_else(|| panic!("no element `{key}`"))
    };
    let name = find("lab-name");
    assert_eq!(name["node"]["role"], "textbox");
    assert_eq!(name["node"]["label"], "Name");
    assert_eq!(name["node"]["value"], "web-01");
    assert!(name["bounds"][2].as_f64().expect("width") > 100.);
    let locked = find("lab-locked");
    assert_eq!(locked["disabled"], true);
    assert_eq!(
        locked["node"]["disabled"],
        "The pool policy keeps the addresses."
    );
    let text = find("lab-dialog/#0");
    assert_eq!(
        text["node"]["text"],
        "Copies the VM and its disks to the chosen storage repository."
    );
    assert_eq!(
        output.tree["model"]["surfaces"].as_array().map(Vec::len),
        Some(0)
    );
    assert_eq!(output.tree["scene"], "gallery-dialog");
}
