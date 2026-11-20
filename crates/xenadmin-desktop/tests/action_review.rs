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
//! A control that no state the test makes shows is not found: the review
//! of this test on 2026-11-20 found that its operation records had hidden
//! the controls of the Snapshots and device records (see `further_states`).
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
    DeviceOperation, OperationAction, OperationId, OperationRecord, OperationState, ResourceAction,
    ResourceOperationRecord, ResourceTarget, SnapshotOperation, SnapshotWorkflow,
};
use xenadmin_desktop::console::windows as console_windows;
use xenadmin_desktop::kit::testing::{TestAppContext, TestQueryExt as _, TestWindowExt as _};
use xenadmin_desktop::kit::*;
use xenadmin_desktop::scene_view::{LOCAL_ROW_KEY, TAB_BASE, TAB_BODY_KEY, TABS_KEY};
use xenadmin_desktop::scenes::{self, Options, ShellWindow};
use xenadmin_desktop::shell::{nav as pane, status, toolbar};
use xenadmin_ui::areas::console::view as console_view;
use xenadmin_ui::areas::events::{self, Family, activity};
use xenadmin_ui::command::CommandId;
use xenadmin_ui::intent::ConsoleWindowId;
use xenadmin_ui::nav::{self, NavView};
use xenadmin_ui::object::Target;
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
/// `*` stands for any run of characters; the first row that matches is the
/// control's), its class, what it does and, for an action, why it stays
/// where it is.
struct Reviewed {
    place: &'static str,
    key: &'static str,
    class: Class,
    what: &'static str,
    stays: &'static str,
}

const fn view(place: &'static str, key: &'static str, what: &'static str) -> Reviewed {
    Reviewed {
        place,
        key,
        class: View,
        what,
        stays: "",
    }
}

const fn action(
    place: &'static str,
    key: &'static str,
    what: &'static str,
    stays: &'static str,
) -> Reviewed {
    Reviewed {
        place,
        key,
        class: Action,
        what,
        stays,
    }
}

/// Why a button of the detached console window is not a command.
const WINDOW: &str = "The detached console window has no menu bar, toolbar or row of commands, and a command \
    acts on the selection of the main window, while this acts on the console of its own \
    window. In the main window the same control is a command of the Console tab's row.";

/// Why a control under an operation record is not a command.
const RECORD: &str = "Not moved, by choice. Its target is one operation record, shown as a line of the tab. \
    The plan lets a command of a tab's row act on the selection of a grid; the records are \
    lines without a selection and were not made a grid in this port, so the control stays \
    the record's own.";

/// Why the two buttons of the saved graph presets are not commands.
const PRESET: &str = "Not moved, by choice. It acts on the preset named in the field beside it or on the \
    preset of its own row, inside the body that replaces the graphs (opened by the command \
    performance.presets); the presets were left inside the tab. It writes only the \
    client's own preferences.";

/// The review. Keep it sorted by place; within a place a narrower key
/// comes before the wider one that would also match.
const REVIEWED: &[Reviewed] = &[
    // What the renderer and the Kit give every tab.
    view(
        "*",
        "property/copy",
        "copies the value of a property line to the clipboard",
    ),
    // The detached console window.
    view(
        "console-window",
        "close-console",
        "closes the window and the console connection it shows",
    ),
    view(
        "console-window",
        "console-actual-size",
        "draws the console at its own size instead of fitted to the window",
    ),
    action(
        "console-window",
        "console-alt-tab",
        "sends Alt+Tab to the guest",
        WINDOW,
    ),
    view(
        "console-window",
        "console-auto-reconnect",
        "whether this console view connects again by itself when its connection ends",
    ),
    action(
        "console-window",
        "console-ctrl-alt-delete",
        "sends Ctrl+Alt+Delete to the guest",
        WINDOW,
    ),
    action(
        "console-window",
        "console-ctrl-alt-f1",
        "sends Ctrl+Alt+F1 to the guest",
        WINDOW,
    ),
    view(
        "console-window",
        "console-fullscreen",
        "shows the window fullscreen",
    ),
    view(
        "console-window",
        "console-release-input",
        "gives the keyboard and the pointer back to the client",
    ),
    view(
        "console-window",
        "copy-console-clipboard",
        "copies the text the guest offered to the local clipboard",
    ),
    action(
        "console-window",
        "reconnect-console",
        "opens the console connection again",
        WINDOW,
    ),
    action(
        "console-window",
        "save-console-screenshot",
        "asks for a file and writes the console picture to it",
        WINDOW,
    ),
    action(
        "console-window",
        "send-console-clipboard",
        "sends the local clipboard text to the guest's clipboard",
        WINDOW,
    ),
    action(
        "console-window",
        "type-console-clipboard",
        "opens the review of the clipboard text that would be typed into the guest",
        WINDOW,
    ),
    // The shell.
    view(
        "shell",
        "sidebar-toggle",
        "shows or hides the navigation pane in a narrow window",
    ),
    // Notifications: Alerts.
    view(
        "tab-alerts",
        "activity-pool*",
        "the pool whose alerts are listed",
    ),
    view(
        "tab-alerts",
        "activity-search",
        "the text filter of the list",
    ),
    view(
        "tab-alerts",
        "activity-state-filter",
        "the priority filter of the list",
    ),
    // Console.
    view(
        "tab-console",
        "cancel-console-typing",
        "drops the review of the text to type; nothing is sent",
    ),
    action(
        "tab-console",
        "confirm-console-typing",
        "types the reviewed text into the guest as key presses",
        "Not moved, by choice. It confirms an action, and the review it confirms is an inline \
         group of the tab (opened by the command console.type-clipboard) where the project's \
         rule would make it a dialog; the review was left as the old front end had it, with \
         the confirmation beside the text it confirms.",
    ),
    view(
        "tab-console",
        "console-auto-reconnect",
        "whether this console view connects again by itself when its connection ends",
    ),
    // Notifications: Events.
    view(
        "tab-events",
        "activity-History",
        "lists the history of operations",
    ),
    view(
        "tab-events",
        "activity-Tasks",
        "lists the tasks of the pools",
    ),
    view(
        "tab-events",
        "activity-pool*",
        "the pool whose records are listed",
    ),
    view(
        "tab-events",
        "activity-show-children",
        "whether the records of the members of a batch are rows of the history",
    ),
    view(
        "tab-events",
        "activity-show-hidden",
        "whether the locally hidden finished entries are rows of the history",
    ),
    view(
        "tab-events",
        "activity-search",
        "the text filter of the list",
    ),
    view(
        "tab-events",
        "activity-state-filter",
        "the state filter of the list",
    ),
    view(
        "tab-events",
        "cross-destination-*",
        "selects the VM of a finished transfer in its destination pool (after a live move, \
         with its console); it changes no object",
    ),
    // General: the operation records of a VM.
    action(
        "tab-general",
        "operation-cancel-*",
        "asks the server to cancel the operation of the record, or stops the remaining steps \
         of a batch",
        RECORD,
    ),
    action(
        "tab-general",
        "operation-recheck-*",
        "asks the server again for the outcome of an operation whose response was lost",
        RECORD,
    ),
    action(
        "tab-general",
        "operation-unknown-*",
        "opens the review of an unknown outcome",
        RECORD,
    ),
    action(
        "tab-general",
        "recovery-cancel-*",
        "opens the review of a cancellation request",
        RECORD,
    ),
    action(
        "tab-general",
        "vm-failure-help-*",
        "opens the explanation of the record's outcome",
        RECORD,
    ),
    // HA.
    view("tab-ha", "ha-vm-filter", "the text filter of the VM grid"),
    action(
        "tab-ha",
        "ha-vms/activate",
        "a double click or Enter on a VM's row selects the VM and invokes the command \
         vm.ha.restart-priority, which opens its dialog",
        "No control is drawn for it: it is the command of the tab's row, invoked through the \
         command table with the command's own eligibility, for the row it is pressed on.",
    ),
    // Home.
    action(
        "tab-home",
        "connections/activate",
        "a double click or Enter on a connection shows the pool of a connected one, and opens \
         the settings dialog of one that is not connected",
        "No control is drawn for it: it is the row's own default action, what the old \
         interface's \"View pool\" and edit buttons of a connection did; the commands of the \
         row are in the tab's row and the row's menu.",
    ),
    // Networking and NICs.
    action(
        "tab-networking",
        "device-recheck-*",
        "asks the server again for the outcome of a device change whose response was lost",
        RECORD,
    ),
    view(
        "tab-networking",
        "network-operation-history",
        "shows the Events tab with the history of the pool",
    ),
    view(
        "tab-networking",
        "networking-networks-table/activate",
        "a double click or Enter on a network selects it in the tree",
    ),
    view(
        "tab-nics",
        "network-operation-history",
        "shows the Events tab with the history of the pool",
    ),
    // Performance.
    view(
        "tab-performance",
        "apply-metric-sources",
        "draws the graphs of the chosen data sources",
    ),
    view(
        "tab-performance",
        "metric-preset-name",
        "the name a preset would be saved under",
    ),
    view(
        "tab-performance",
        "metric-preset-name/clear",
        "empties the name field",
    ),
    action(
        "tab-performance",
        "metric-preset-remove-*",
        "removes a saved graph preset from the client's preferences",
        PRESET,
    ),
    action(
        "tab-performance",
        "metric-preset-save",
        "saves the range and the sources of the graph as a preset in the client's preferences",
        PRESET,
    ),
    view(
        "tab-performance",
        "metric-preset-*",
        "draws the graphs with the range and the sources of a saved preset",
    ),
    view(
        "tab-performance",
        "metric-presets-close",
        "goes back from the presets to the graphs",
    ),
    view(
        "tab-performance",
        "metrics-source-filter",
        "the text filter of the list of data sources",
    ),
    view(
        "tab-performance",
        "metrics-source-*",
        "whether a data source is among those to draw",
    ),
    view(
        "tab-performance",
        "metrics-sources-cancel",
        "goes back to the graphs without changing the sources",
    ),
    view(
        "tab-performance",
        "metrics-sources-common",
        "draws the graphs of the common data sources",
    ),
    view(
        "tab-performance",
        "metrics-window-*",
        "the time range of the graphs",
    ),
    // Search.
    view(
        "tab-search",
        "search-apply",
        "runs the query over the inventory the client holds",
    ),
    view("tab-search", "search-filters", "shows or hides the filters"),
    view(
        "tab-search",
        "search-group-*",
        "how the results are grouped",
    ),
    view("tab-search", "search-reset", "empties the filters"),
    view(
        "tab-search",
        "search-results/activate",
        "a double click or Enter on a result selects the object in the tree",
    ),
    view("tab-search", "search-results/sort", "sorts the results"),
    view("tab-search", "search-text", "the text of the query"),
    // Snapshots.
    action(
        "tab-snapshots",
        "snapshot-preview-load",
        "asks the host for the saved picture of the selected snapshot and shows it; it changes \
         nothing on the host",
        "Not moved, by choice. It acts on the selected snapshot, as the commands of the tab's \
         row do, and could be one of them. It was left a button of the section it fills, \
         \"Snapshot information\", where it reports a refusal; it is a read.",
    ),
    action(
        "tab-snapshots",
        "snapshot-operation-cancel-*",
        "asks that the remaining steps of a running snapshot workflow are not started",
        RECORD,
    ),
    action(
        "tab-snapshots",
        "snapshot-operation-recheck-*",
        "asks the server again for the outcome of a snapshot workflow whose response was lost",
        RECORD,
    ),
    view(
        "tab-snapshots",
        "snapshot-show-created-*",
        "selects the VM or template a snapshot workflow created",
    ),
    view(
        "tab-snapshots",
        "snapshot-tree-toggle",
        "shows the snapshots as a tree or as a list",
    ),
    // Storage.
    action(
        "tab-storage",
        "device-recheck-*",
        "asks the server again for the outcome of a device change whose response was lost",
        RECORD,
    ),
    view("tab-storage", "disk-search", "the text filter of the disks"),
    view(
        "tab-storage",
        "storage-pbds/activate",
        "a double click or Enter on an attachment selects its repository in the tree",
    ),
    view(
        "tab-storage",
        "storage-repositories-table/activate",
        "a double click or Enter on a repository selects it in the tree",
    ),
    // Users.
    view(
        "tab-users",
        "security-subject-filter",
        "the text filter of the grid of directory users and groups",
    ),
];

/// What the `*` of a row may stand for. A row's key with a `*` is the
/// class of every control whose key begins the same, so a new control
/// with that prefix (`metric-preset-rename-…`, `metrics-source-reset`)
/// would be passed as reviewed. For the rows named here the keys are
/// those the fixtures of the scenes make, each listed; for every other
/// row with a `*` the generated part must be the id of a record (a UUID).
const GENERATED: &[(&str, &[&str])] = &[
    (
        "activity-pool*",
        &[
            "activity-pool",
            "activity-pool-all",
            "activity-pool-10000000-0000-0000-0000-000000000001",
        ],
    ),
    ("metric-preset-*", &["metric-preset-CPU and memory"]),
    (
        "metric-preset-remove-*",
        &["metric-preset-remove-CPU and memory"],
    ),
    (
        "metrics-source-*",
        &[
            "metrics-source-cpu0",
            "metrics-source-filter",
            "metrics-source-memory",
            "metrics-source-vbd_xvda_read",
            "metrics-source-vif_0_rx",
        ],
    ),
    (
        "metrics-window-*",
        &[
            "metrics-window-600",
            "metrics-window-7200",
            "metrics-window-604800",
            "metrics-window-31536000",
        ],
    ),
    (
        "search-group-*",
        &[
            "search-group-Folder",
            "search-group-Name",
            "search-group-Pool",
            "search-group-Type",
        ],
    ),
];

/// Whether a key is one its row's `*` may stand for ([`GENERATED`]).
fn generated(row: &str, key: &str) -> bool {
    let Some((prefix, suffix)) = row.split_once('*') else {
        return true;
    };
    match GENERATED.iter().find(|(listed, _)| *listed == row) {
        Some((_, keys)) => keys.contains(&key),
        None => key
            .strip_prefix(prefix)
            .and_then(|rest| rest.strip_suffix(suffix))
            .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok()),
    }
}

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
    // its tabs list them, each with the controls of the record. A tab
    // shows only its newest records (the Snapshots tab two), so each record
    // is also shown alone.
    let shell = open_scene(cx, "snapshots", WIDE);
    let (pool, vm, created) = model(cx, &shell, |m| {
        let scope = m
            .scope()
            .objects
            .first()
            .cloned()
            .expect("the VM of the scene");
        let pool = scope.pool.expect("the pool of the VM");
        // Any other VM of the pool stands for what a workflow created.
        let created = m
            .inventory
            .as_ref()
            .and_then(|inventory| {
                inventory
                    .vms
                    .keys()
                    .find(|id| id.to_string() != scope.id)
                    .copied()
            })
            .expect("a second VM");
        (pool, scope.id, created)
    });
    let record = |action: OperationAction, state: OperationState| -> OperationRecord {
        let mut record: OperationRecord = serde_json::from_value(serde_json::json!({
            "id": OperationId::new(), "pool": pool, "vm": vm,
            "action": action,
            "state": state, "progress": null, "detail": "Synthetic record",
        }))
        .expect("operation record");
        record.can_cancel = state == OperationState::Tracking;
        record
    };
    let snapshot = |state: OperationState| {
        record(
            OperationAction::Snapshot(SnapshotOperation::Workflow {
                operation: SnapshotWorkflow::Create {
                    name: "Before upgrade".into(),
                    description: String::new(),
                    memory: false,
                },
            }),
            state,
        )
    };
    let mut finished = snapshot(OperationState::Succeeded);
    finished.created_vm = Some(created);
    let device = record(
        OperationAction::Device(DeviceOperation::AddCdDrive { device: "3".into() }),
        OperationState::OutcomeUnknown,
    );
    let records = vec![
        snapshot(OperationState::Tracking),
        snapshot(OperationState::OutcomeUnknown),
        snapshot(OperationState::Failed),
        snapshot(OperationState::PartiallyCompleted),
        finished,
        device,
    ];
    let mut states: Vec<(String, Vec<OperationRecord>)> = records
        .iter()
        .map(|record| {
            (
                format!(
                    "snapshots, with one operation record ({}, {:?})",
                    record.action.label(),
                    record.state
                ),
                vec![record.clone()],
            )
        })
        .collect();
    states.push(("snapshots, with operation records".into(), records));
    for (state, records) in states {
        run(cx, &shell, |m| m.operation_rows = records);
        let tab = model(cx, &shell, |m| m.active_tab().key());
        on(cx, &shell, |window, _| drawn(window, tab, &state, findings));
        described_tabs(cx, &shell, &state, findings);
    }
    close(cx, shell);
    // A finished transfer to another pool, selected in the history of the
    // Events tab: its outcome offers the destination.
    let state = "cross-pool, a finished transfer selected in the history";
    let shell = open_scene(cx, "cross-pool", WIDE);
    run(cx, &shell, |m| {
        let editor = m.areas.cross_pool.wizard.as_deref().expect("the wizard");
        let pool = editor.source.inventory.pool_id;
        let record: ResourceOperationRecord = serde_json::from_value(serde_json::json!({
            "id": OperationId::new(), "pool": pool,
            "target": ResourceTarget::Pool(pool),
            "action": ResourceAction::CrossPoolVm(editor.review.clone().expect("the review")),
            "state": OperationState::Succeeded, "progress": 1.0,
            "detail": "Synthetic transfer", "created_at_ms": 1,
            "can_cancel": false, "cancel_requested": false, "steps": [], "created": null,
            "transfer_created": {
                "vms": [editor.vm.id], "disks": [], "vbds": [], "vifs": [], "networks": [],
            },
        }))
        .expect("resource record");
        let key = activity::key_of(activity::history_key(pool, record.id, Family::Resource));
        m.resource_rows.push(record);
        m.areas.cross_pool.wizard = None;
        events::show(m);
        m.areas.events.list = activity::Tab::History;
        m.areas.events.selected = Some(key);
    });
    assert!(
        model(cx, &shell, |m| activity::selected(
            m,
            activity::Tab::History
        )
        .is_some()),
        "the transfer is a row of the history"
    );
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
            Some(index) if !generated(REVIEWED[index].key, &found.key) => unlisted.push(format!(
                "{} | {} | {label:?} | first in scene {scene} | it has the prefix of the row \
                 `{}` and is not one of the keys that row stands for (GENERATED)",
                found.place, found.key, REVIEWED[index].key
            )),
            Some(index) => {
                used.insert(index);
            }
            None => unlisted.push(format!(
                "{} | {} | {label:?} | first in scene {scene}",
                found.place, found.key
            )),
        }
    }
    let print = std::env::var_os("XENADMIN_ACTION_REVIEW_PRINT").is_some();
    if print {
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
    // Every action that stays says why, and only an action does.
    for row in REVIEWED {
        assert!(!row.what.is_empty(), "{} | {}", row.place, row.key);
        assert_eq!(
            row.stays.is_empty(),
            row.class == View,
            "{} | {}",
            row.place,
            row.key
        );
    }
    if print {
        let count = |class: Class| REVIEWED.iter().filter(|row| row.class == class).count();
        println!(
            "rows of the review: {} of the view, {} actions that stay",
            count(View),
            count(Action)
        );
        for row in REVIEWED.iter().filter(|row| row.class == Action) {
            println!(
                "stays: {} | {} | {} | {}",
                row.place, row.key, row.what, row.stays
            );
        }
    }
}
