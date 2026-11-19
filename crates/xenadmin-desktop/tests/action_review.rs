//! Gate G2, criterion 5 of docs/GPUI-MIGRATION.md: "Review finds no action
//! button outside the toolbar, a tab's local action row, or a dialog."
//!
//! The review, made so that it stays made. The test goes through every
//! registered scene and collects every control that is in none of the
//! places the criterion and the shell allow: the menu bar, a menu, the
//! toolbar, the navigation pane, the tab strip, a tab's row of local
//! actions, the status bar, and a dialog, wizard or Properties surface.
//! What is left is what a tab body (and the detached console window) holds
//! of its own. Each such control must be in [`REVIEWED`], which names it
//! and says what it is: a control of the view, or an action. A control
//! that is not in the table fails the test, and so does a row of the table
//! that no scene shows any more.
//!
//! Two passes find the controls:
//!
//! - the element tree of the window of each scene as it is registered, at
//!   1440 by 900 (what the generic renderer and the hand-written views
//!   draw, including what the Kit adds: sort buttons, clear buttons, copy
//!   buttons);
//! - the description the model gives of every tab its scope offers, for
//!   each scene and for one object of every kind of the demonstration
//!   (what is below the fold, behind a dialog or on a tab no scene shows).
//!
//! A row of a table is not a control here: selecting it sets the target of
//! the commands and changes nothing else. What a double click or Enter on
//! a row does is collected (`<table>/activate`).
//!
//! The platform is the Kit's test platform: nothing here establishes what a
//! real window shows.
#![cfg(feature = "scenes")]

mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::{WIDE, close, model, on, open, open_scene, run};
use xenadmin_core::{
    OperationAction, OperationId, OperationRecord, OperationState, SnapshotOperation,
    SnapshotWorkflow,
};
use xenadmin_desktop::console::windows as console_windows;
use xenadmin_desktop::kit::testing::{TestAppContext, TestQueryExt as _, TestWindowExt as _};
use xenadmin_desktop::kit::*;
use xenadmin_desktop::scene_view::{LOCAL_ROW_KEY, TAB_BASE, TAB_BODY_KEY, TABS_KEY};
use xenadmin_desktop::scenes::{self, Options, ShellWindow};
use xenadmin_desktop::shell::{nav as pane, status, toolbar};
use xenadmin_ui::areas::console::view as console_view;
use xenadmin_ui::command::CommandId;
use xenadmin_ui::intent::ConsoleWindowId;
use xenadmin_ui::nav::{self, NavView};
use xenadmin_ui::object::{ObjectKind, Target};
use xenadmin_ui::scene;
use xenadmin_ui::surface::{self, Block, Handle};

/// What a control outside the allowed places is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Class {
    /// It changes only what is shown: a filter, a range, a sort, a
    /// disclosure, a page of a list, a choice of a form that is not
    /// submitted from there. Copying a shown value to the clipboard is
    /// counted here too: it changes nothing.
    View,
    /// It changes an object, starts an operation or opens a dialog, and it
    /// stays where it is for the reason its row gives. The same rows are in
    /// the Deviations table of docs/GPUI-MIGRATION-LEDGER.md.
    Action,
}

use Class::{Action, View};

/// One reviewed control: where it is (the key of a tab, `console-window`,
/// `shell`, or `*` for a part the renderer gives every tab), its key (a
/// `*` stands for any run of characters), its class and what it does. For
/// an action the text ends with the reason it stays.
struct Reviewed {
    place: &'static str,
    key: &'static str,
    class: Class,
    what: &'static str,
}

const fn reviewed(
    place: &'static str,
    key: &'static str,
    class: Class,
    what: &'static str,
) -> Reviewed {
    Reviewed {
        place,
        key,
        class,
        what,
    }
}

/// The review. Keep it sorted by place.
const REVIEWED: &[Reviewed] = &[];

/// The path segments under which a control is in an allowed place.
const ALLOWED: &[&str] = &[
    "app-menu-bar",
    "popup-menu",
    toolbar::TOOLBAR_KEY,
    pane::PANE_KEY,
    TABS_KEY,
    LOCAL_ROW_KEY,
    status::STATUS_KEY,
    // Every dialog, wizard and Properties surface is drawn under the Kit's
    // dialog layer.
    "dialog",
];

/// A control that was found.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Found {
    place: String,
    key: String,
}

#[derive(Default)]
struct Findings {
    /// The control, with a label and the first scene that shows it.
    seen: BTreeMap<Found, (String, String)>,
}

impl Findings {
    fn add(&mut self, place: &str, key: String, label: &str, scene: &str) {
        self.seen
            .entry(Found {
                place: place.to_owned(),
                key,
            })
            .or_insert_with(|| (label.to_owned(), scene.to_owned()));
    }
}

/// Whether an element with this role takes a press or text. Anything the
/// list below does not name counts, so that a role the renderer starts to
/// use is reviewed and not passed over.
fn is_control(role: Role) -> bool {
    !matches!(
        role,
        Role::Label
            | Role::Heading
            | Role::Group
            | Role::Status
            | Role::Cell
            | Role::Row
            | Role::ColumnHeader
            | Role::ListItem
            | Role::List
            | Role::TreeItem
            | Role::Tree
            | Role::Table
            | Role::Grid
            | Role::Toolbar
            | Role::TabList
            | Role::TabPanel
            | Role::Navigation
            | Role::Dialog
            | Role::Image
            | Role::Figure
            | Role::Alert
            | Role::Note
            | Role::ProgressIndicator
            | Role::Application
            | Role::Splitter
            | Role::RadioGroup
            | Role::Menu
            | Role::Window
            | Role::GenericContainer
    )
}

/// The key of a drawn control. The parts the Kit adds to a field or a
/// table are named after what they belong to: `<field>/clear`,
/// `<table>/sort`, `property/copy`.
fn drawn_key(path: &[String]) -> String {
    let names: Vec<&str> = path
        .iter()
        .map(String::as_str)
        .filter(|segment| !segment.contains("::") && !segment.contains('('))
        .collect();
    let Some((&last, before)) = names.split_last() else {
        return String::new();
    };
    let back = |steps: usize| {
        before
            .len()
            .checked_sub(steps)
            .and_then(|index| before.get(index))
            .copied()
            .unwrap_or("?")
    };
    if last == "clean" {
        // <field>/suffix/clean
        return format!("{}/clear", back(2));
    }
    if last == "input-state" {
        return back(1).to_owned();
    }
    if last.starts_with("icon-sort:") {
        // <table>/table/table-inner/header/table-head/col-header:n/icon-sort:n
        let table = before
            .iter()
            .position(|segment| *segment == "table")
            .and_then(|index| index.checked_sub(1))
            .and_then(|index| before.get(index))
            .copied()
            .unwrap_or("?");
        return format!("{table}/sort");
    }
    if last.ends_with("-copy") {
        return "property/copy".to_owned();
    }
    last.to_owned()
}

/// The controls the window draws outside the allowed places.
fn drawn(window: &mut Window, place: &str, scene: &str, findings: &mut Findings) {
    for element in window.elements() {
        let Some(role) = element.role() else {
            continue;
        };
        if !is_control(role) {
            continue;
        }
        let path = scenes::id_path(&element);
        if path
            .iter()
            .any(|segment| ALLOWED.contains(&segment.as_str()))
        {
            continue;
        }
        let place = if path.iter().any(|segment| segment == TAB_BODY_KEY)
            || path
                .iter()
                .any(|segment| segment == console_windows::ROOT_KEY)
        {
            place
        } else {
            "shell"
        };
        findings.add(
            place,
            drawn_key(&path),
            element.label().unwrap_or_default(),
            scene,
        );
    }
}

/// The controls a description holds.
fn described(blocks: &[Block], base: &str, place: &str, scene: &str, findings: &mut Findings) {
    for element in surface::block_elements(base, blocks) {
        let node = &element.node;
        let key = node.key.clone().unwrap_or_else(|| node.path.clone());
        let label = node
            .label
            .as_deref()
            .or(node.text.as_deref())
            .unwrap_or_default();
        match element.handle {
            Some(
                Handle::Press(_) | Handle::Edit { .. } | Handle::Toggle(_) | Handle::Pick { .. },
            ) => {
                findings.add(place, key, label, scene);
            }
            Some(Handle::Sort { table, .. }) => {
                findings.add(place, format!("{table}/sort"), label, scene);
            }
            Some(Handle::Row {
                table,
                on_activate: Some(_),
                ..
            }) => {
                findings.add(place, format!("{table}/activate"), "", scene);
            }
            // Selecting a row sets the target of the commands; the row menu
            // of a table is made of commands; a command button is in a row
            // of commands.
            Some(Handle::Row { .. } | Handle::Table { .. } | Handle::Command(_)) | None => {}
        }
    }
}

/// The description of every tab the scope of the model offers.
fn described_tabs(
    cx: &mut TestAppContext,
    shell: &ShellWindow,
    scene: &str,
    findings: &mut Findings,
) {
    let bodies: Vec<(&'static str, Vec<Block>)> = cx.update(|cx| {
        let store = shell.store.read(cx);
        store
            .model()
            .tabs()
            .iter()
            .filter_map(|tab| Some((tab.key(), store.tab_body(*tab)?.blocks())))
            .collect()
    });
    for (tab, blocks) in bodies {
        described(&blocks, TAB_BASE, tab, scene, findings);
    }
}

fn matches(pattern: &str, text: &str) -> bool {
    let mut parts = pattern.split('*');
    let first = parts.next().unwrap_or_default();
    let Some(mut rest) = text.strip_prefix(first) else {
        return false;
    };
    let mut parts: Vec<&str> = parts.collect();
    let Some(last) = parts.pop() else {
        return rest.is_empty();
    };
    for part in parts {
        match rest.find(part) {
            Some(index) => rest = &rest[index + part.len()..],
            None => return false,
        }
    }
    rest.ends_with(last)
}

/// States of a tab that no registered scene shows.
fn further_states(cx: &mut TestAppContext, findings: &mut Findings) {
    // The source picker of the Performance tab, which replaces the graphs.
    let state = "performance, after Data Sources…";
    let shell = open_scene(cx, "performance", WIDE);
    cx.update(|cx| {
        shell.store.update(cx, |store, cx| {
            store.invoke(CommandId("performance.data-sources"), cx);
        });
    });
    cx.run_until_parked();
    let tab = model(cx, &shell, |m| m.active_tab().key());
    on(cx, &shell, |window, _| drawn(window, tab, state, findings));
    described_tabs(cx, &shell, state, findings);
    close(cx, shell);
    // A VM with operation records in every state that offers a control:
    // its tabs list them, each with the controls of the record.
    let state = "snapshots, with operation records";
    let shell = open_scene(cx, "snapshots", WIDE);
    run(cx, &shell, |m| {
        let scope = m.scope().objects.first().cloned().expect("the VM of the scene");
        let pool = scope.pool.expect("the pool of the VM");
        let record = |state: OperationState| -> OperationRecord {
            serde_json::from_value(serde_json::json!({
                "id": OperationId::new(), "pool": pool, "vm": scope.id,
                "action": OperationAction::Snapshot(SnapshotOperation::Workflow {
                    operation: SnapshotWorkflow::Create {
                        name: "Before upgrade".into(), description: String::new(), memory: false,
                    },
                }),
                "state": state, "progress": null, "detail": "Synthetic record",
            }))
            .expect("operation record")
        };
        let mut running = record(OperationState::Tracking);
        running.can_cancel = true;
        m.operation_rows = vec![
            running,
            record(OperationState::OutcomeUnknown),
            record(OperationState::Failed),
            record(OperationState::PartiallyCompleted),
            record(OperationState::Succeeded),
        ];
    });
    let tab = model(cx, &shell, |m| m.active_tab().key());
    on(cx, &shell, |window, _| drawn(window, tab, state, findings));
    described_tabs(cx, &shell, state, findings);
    close(cx, shell);
}

fn collect(cx: &mut TestAppContext) -> Findings {
    let mut findings = Findings::default();
    for spec in scene::registry() {
        let name = spec.name;
        let setup = scenes::setup(name).unwrap_or_else(|| panic!("no scene is named {name}"));
        let shell = open(cx, setup, WIDE);
        let tab = model(cx, &shell, |m| m.active_tab().key());
        on(cx, &shell, |window, _| {
            drawn(window, tab, name, &mut findings);
        });
        described_tabs(cx, &shell, name, &mut findings);
        // The demonstration, once more: one object of every kind, every tab
        // it has.
        if name == "demo" {
            let mut kinds: BTreeSet<String> = BTreeSet::new();
            for view in NavView::ALL {
                run(cx, &shell, |m| m.set_nav_view(view));
                for row in model(cx, &shell, nav::rows) {
                    let Some(object) = row.object else { continue };
                    if !kinds.insert(format!("{:?}", object.kind)) {
                        continue;
                    }
                    run(cx, &shell, |m| m.select(Target::one(object)));
                    let label = format!("demo, {}", row.label);
                    described_tabs(cx, &shell, &label, &mut findings);
                }
            }
            assert!(kinds.len() >= 5, "the demonstration has {kinds:?}");
        }
        close(cx, shell);
        // The detached console window is a window of its own.
        if name == scenes::CONSOLE_WINDOW_SCENE {
            let setup = scenes::setup(name).expect("scene");
            let options = Options {
                width: console_windows::DEFAULT_SIZE.0,
                height: console_windows::DEFAULT_SIZE.1,
                ..Options::default()
            };
            let id = ConsoleWindowId(1);
            let console = cx
                .update(|cx| scenes::open_console_window(cx, setup, &options, id))
                .expect("console window");
            cx.run_until_parked();
            for _ in 0..3 {
                cx.update_window(console.window, |_, window, cx| window.render_frame(cx))
                    .expect("window");
                cx.run_until_parked();
            }
            cx.update_window(console.window, |_, window, _| {
                drawn(window, console_windows::ROOT_KEY, name, &mut findings);
            })
            .expect("window");
            let blocks =
                cx.update(|cx| console_view::window(console.store.read(cx).model(), id).blocks);
            described(
                &blocks,
                console_windows::BASE,
                console_windows::ROOT_KEY,
                name,
                &mut findings,
            );
            let _ = cx.update_window(console.window, |_, window, _| window.remove_window());
            cx.run_until_parked();
        }
    }
    further_states(cx, &mut findings);
    // The shell at its smallest size, where the navigation pane is behind a
    // toggle.
    let setup = scenes::setup("demo").expect("scene");
    let shell = open(cx, setup, common::SMALL);
    let tab = model(cx, &shell, |m| m.active_tab().key());
    on(cx, &shell, |window, _| {
        drawn(window, tab, "demo at 800 by 600", &mut findings);
    });
    close(cx, shell);
    findings
}

#[gpui_kit::test]
fn every_control_outside_the_allowed_places_is_reviewed(cx: &mut TestAppContext) {
    let findings = collect(cx);
    let entry = |found: &Found| {
        REVIEWED.iter().position(|row| {
            (row.place == "*" || row.place == found.place) && matches(row.key, &found.key)
        })
    };
    let mut used = BTreeSet::new();
    let mut unlisted = Vec::new();
    for (found, (label, scene)) in &findings.seen {
        match entry(found) {
            Some(index) => {
                used.insert(index);
            }
            None => unlisted.push(format!(
                "{} | {} | {label:?} | first in scene {scene}",
                found.place, found.key
            )),
        }
    }
    if std::env::var_os("XENADMIN_ACTION_REVIEW_PRINT").is_some() {
        for (found, (label, scene)) in &findings.seen {
            let class = entry(found).map(|index| REVIEWED[index].class);
            println!(
                "{} | {} | {label:?} | {scene} | {class:?}",
                found.place, found.key
            );
        }
    }
    assert!(
        unlisted.is_empty(),
        "{} control(s) outside the toolbar, a local action row and the dialogs are not in the \
         review (place | key | label):\n{}",
        unlisted.len(),
        unlisted.join("\n")
    );
    let stale: Vec<String> = REVIEWED
        .iter()
        .enumerate()
        .filter(|(index, _)| !used.contains(index))
        .map(|(_, row)| format!("{} | {}", row.place, row.key))
        .collect();
    assert!(
        stale.is_empty(),
        "rows of the review that no scene shows any more:\n{}",
        stale.join("\n")
    );
    // Every action that stays says why.
    for row in REVIEWED {
        assert!(!row.what.is_empty(), "{} | {}", row.place, row.key);
    }
    let _ = (View, Action, ObjectKind::Vm);
}
