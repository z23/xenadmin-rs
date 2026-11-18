//! Area `events`: the Alerts and Events tabs of the Notifications view, and
//! everything that is done to an operation after it was submitted.
//!
//! Rune modules: `activity.rs`, `activity_export.rs`, `failure_help.rs`; the
//! history list with Recheck, Acknowledge and Cancel of `resource_admin.rs`;
//! the "Acknowledge unknown outcome?" dialog and the per-record buttons of
//! the details pane in `main.rs`. Plan package P3.2. Supported core: no
//! surface of the area carries the experimental notice.
//!
//! The Rune Activity dialog was three lists behind a tab strip. The
//! Notifications navigation view replaces it (AREAS.md decision 6):
//!
//! | Rune | Here |
//! | --- | --- |
//! | Activity, Messages | the Alerts tab |
//! | Activity, Tasks and History | the Events tab, two lists behind the choice `activity-list` (options `activity-Tasks`, `activity-History`) |
//! | pool history of `resource_admin.rs` (rows, evidence lines) | the History list: both families in one list; the evidence lines are in the selected row's outcome |
//! | "Pool: … · change" (`activity-pool`) | a select with the same key |
//! | review mode of the dialog | the dialogs `task-cancellation` and `message-dismissal` |
//! | "Acknowledge unknown outcome?" (`main.rs`), `Kind::Acknowledge` (`resource_admin.rs`) | the dialog `acknowledge-unknown`, one for both families |
//! | "Outcome explanation" | the dialog `failure-help-dialog` |
//!
//! Commands (none has a menu path: the plan's Menus table lists none; all
//! are on the local action row of their tab and in the row menu, and act on
//! the selected row of the list, `State::selected`, not on the `Target`):
//!
//! | id | Label | Tab | Rune |
//! | --- | --- | --- | --- |
//! | `events.cancel-task` | Review cancellation… | Events (Tasks) | `activity-review` |
//! | `events.explain` | Explain outcome… | Events | `activity-failure-help` |
//! | `events.hide` | Hide completed entry / Restore entry | Events (History) | `activity-hide` |
//! | `events.recheck` | Recheck outcome | Events (History) | `activity-recheck` |
//! | `events.acknowledge` | Acknowledge unknown outcome… | Events (History) | "Review unknown outcome", "Acknowledge unknown outcome…" |
//! | `events.request-cancel` | Request cancellation / Stop remaining steps / Finish current VM and stop / Finish current snapshot and stop | Events (History) | the unkeyed buttons of the details pane and pool history |
//! | `events.export` | Export filtered entries… | Events | `activity-export` |
//! | `alerts.dismiss` | Review dismissal… | Alerts | `activity-review` |
//! | `alerts.export` | Export filtered entries… | Alerts | `activity-export` |
//!
//! Surfaces: `task-cancellation`, `message-dismissal`, `acknowledge-unknown`
//! (small), `failure-help-dialog`; all dialogs, none experimental.
//! Scenes: `activity`, `failure-help`.
//!
//! # For other areas
//!
//! An operation is named by its [`OperationKey`]: pool, operation id and
//! family (`Family::Vm` for a record of `m.operation_rows`,
//! `Family::Resource` for one of `m.resource_rows`). The functions below are
//! what an outcome banner, General's Operations section and the power area
//! use instead of the Rune buttons:
//!
//! - [`show`]: the Events tab (the Rune `activity::open`).
//! - [`show_history`]: the History list of one pool, filters cleared (the
//!   Rune `activity::open_history`).
//! - [`show_states`]: the History list of every pool with a state filter,
//!   for the status bar's counters.
//! - [`show_operation`]: the History list with one operation selected.
//! - [`can_explain`], [`explain`]: the outcome explanation of a record;
//!   [`show_help`] for an explanation the caller built itself.
//! - [`recheck_eligibility`], [`recheck`]: read the outcome again.
//! - [`acknowledge_eligibility`], [`acknowledge`]: open the reviewed
//!   acknowledgement of an unknown outcome.
//! - [`cancel_eligibility`], [`cancel_label`], [`cancel`]: ask for
//!   cancellation.
//! - [`outcome_actions`]: the buttons Rune put under a record, ready for
//!   `outcome(..).actions(..)`, with the Rune rules for when each shows.
//!
//! A caller closes its own surface before it calls a `show` function: the
//! tabs are behind every dialog.
//!
//! # What has no successor
//!
//! - The Activity button (`activity-open`) and the dialog's Close
//!   (`activity-close`), its scroll container (`activity-scroll`) and the
//!   tab button `activity-Messages`: the navigation view and its two tabs.
//! - The clear buttons of the two filters: the front end's input has its
//!   own clear affordance; clearing is typing the empty text.
//! - "Cannot open save dialog: {error}": a file prompt that fails answers
//!   like a cancelled one.
//! - The focus return of the outcome explanation (`return_focus`).
//! - The pool history's limit of twelve records and its hiding of batch
//!   children: the History list shows every record, as Activity did.
//! - `resource-operation-notice` ("… · View operation history") of
//!   `resource_admin.rs`: the status bar counts operations and unknown
//!   outcomes (the kernel's), and [`show_history`] is what a link calls.
//!
//! The files, in the order to read them: [`activity`] (the lists and the
//! two reviews of host objects), [`control`] (recheck, acknowledge and
//! cancel of an operation), [`evidence`] (the detail lines of a record),
//! [`failure_help`], [`export`], [`commands`], [`view`], [`scenes`], `tests`.

use xenadmin_core::{OperationState, PoolId};

use crate::prelude::*;

pub mod activity;
pub mod commands;
pub mod control;
pub mod evidence;
pub mod export;
pub mod failure_help;
/// `Family` and `HiddenKey` of the Rune `activity.rs`: what `Preferences`
/// stores for a hidden history entry.
pub mod hidden;
pub mod scenes;
pub mod view;

#[cfg(test)]
mod live_tests;
#[cfg(test)]
mod tests;

pub use failure_help::FailureHelp;
pub use hidden::{Family, HiddenKey};

/// What names one operation record: its pool, its id and its family.
pub type OperationKey = HiddenKey;

/// The state of the area. `m.areas.events`.
#[derive(Default)]
pub struct State {
    /// The list of the Events tab: `Tasks` or `History`. (The Alerts tab is
    /// always the Rune `Messages` list.)
    pub list: activity::Tab,
    /// Free-text filter of every list.
    pub search: String,
    /// Substring filter on the state column of Tasks and History.
    pub state_filter: String,
    /// Substring filter on the priority column of Alerts.
    pub priority_filter: String,
    /// Pool filter; `None` means all observed pools.
    pub pool: Option<PoolId>,
    /// The key of the selected row. One selection for all lists, as in
    /// Rune: a key the list on screen does not have selects nothing.
    pub selected: Option<String>,
    /// Include locally hidden completed entries in History.
    pub show_hidden: bool,
    /// The status line of the tabs.
    pub notice: Option<String>,
    /// The open review of a task cancellation or a message dismissal.
    pub review: Option<activity::Review>,
    /// The open acknowledgement of an unknown outcome.
    pub acknowledge: Option<control::Acknowledge>,
    /// The open outcome explanation.
    pub help: Option<FailureHelp>,
    pub export: export::Export,
    pub(crate) cache: activity::Cache,
}

pub fn spec() -> AreaSpec {
    AreaSpec {
        commands: commands::commands,
        scenes: scenes::scenes,
        surfaces: view::surfaces,
        tab: view::tab,
        poll,
        ..AreaSpec::new("events")
    }
}

/// Nothing of the area belongs to the scope or to the scope session (Rune
/// kept Activity across pool switches), so there is no scope hook. The poll
/// takes the export's result, remembers the inventories and notices changed
/// records; it reports a change only while the Notifications view shows it.
fn poll(m: &mut Model, _clock: f64) -> bool {
    let exported = export::poll(m);
    let changed = activity::poll(m);
    (exported || changed) && m.nav.view == NavView::Notifications
}

/// Shows one of the two tabs of the Notifications view.
pub fn show_tab(m: &mut Model, tab: TabId) {
    m.set_nav_view(NavView::Notifications);
    m.set_active_tab(tab);
}

/// Shows the Events tab: the Rune `activity::open`. The list, the filters,
/// the selection and the notice are kept from the last visit; the pool
/// filter becomes the pool of the scope session.
pub fn show(m: &mut Model) {
    show_tab(m, TabId::Events);
    m.areas.events.review = None;
    m.areas.events.pool = m.inventory.as_ref().map(|i| i.pool_id);
}

/// Shows the History list of one pool with both filters cleared: the Rune
/// `activity::open_history`.
pub fn show_history(m: &mut Model, pool: PoolId) {
    show(m);
    let a = &mut m.areas.events;
    a.pool = Some(pool);
    a.list = activity::Tab::History;
    a.search.clear();
    a.state_filter.clear();
}

/// Shows the History list of every pool, filtered to the states whose name
/// contains `state` (`"OutcomeUnknown"`; empty for all).
pub fn show_states(m: &mut Model, state: &str) {
    show(m);
    let a = &mut m.areas.events;
    a.pool = None;
    a.list = activity::Tab::History;
    a.search.clear();
    a.state_filter = state.to_owned();
}

/// Shows the History list with one operation selected. The filters are
/// cleared, and a locally hidden entry is shown.
pub fn show_operation(m: &mut Model, key: OperationKey) {
    show_history(m, key.pool);
    if m.preferences.hidden_history.contains(&key) {
        m.areas.events.show_hidden = true;
    }
    m.areas.events.selected = Some(activity::key_of(key));
}

/// Whether a record's outcome has an explanation.
pub fn can_explain(m: &Model, key: OperationKey) -> bool {
    control::help(m, key).is_some()
}

/// Opens the outcome explanation of a record. `false` when it has none.
pub fn explain(m: &mut Model, key: OperationKey) -> bool {
    match control::help(m, key) {
        Some(help) => {
            show_help(m, help);
            true
        }
        None => false,
    }
}

/// Opens an outcome explanation. It is a capture: it does not follow the
/// selection or the inventory afterwards.
pub fn show_help(m: &mut Model, help: FailureHelp) {
    m.areas.events.help = Some(help);
}

pub use control::{
    acknowledge_eligibility, cancel, cancel_eligibility, cancel_label, recheck,
    recheck_eligibility,
};

/// Opens the review of acknowledging an unknown outcome. `false`, with the
/// reason in `m.error`, when the record cannot be acknowledged.
pub fn acknowledge(m: &mut Model, key: OperationKey) -> bool {
    match acknowledge_eligibility(m, key) {
        Eligibility::Enabled => control::open_acknowledge(m, key),
        Eligibility::Disabled(reason) => {
            m.error = Some(reason);
            false
        }
        Eligibility::Hidden => false,
    }
}

/// The buttons Rune put under an operation record in the details pane (VM
/// family) and in the pool history (resource family): "Explain outcome…";
/// the cancellation under its name for the record; and for an unknown
/// outcome "Recheck outcome" and the acknowledgement. A result that is not
/// a surface goes to the shared error line, as those buttons' did.
///
/// Keys: `vm-failure-help-{id}` and `resource-failure-help-{id}` are the
/// Rune ones; `operation-cancel-{id}`, `operation-recheck-{id}` and
/// `operation-acknowledge-{id}` are new (Rune left those buttons unkeyed).
pub fn outcome_actions(m: &Model, key: OperationKey) -> Vec<Button> {
    let id = key.id;
    let (state, can_recheck) = match key.family {
        Family::Vm => match control::vm_record(m, key) {
            Some(record) => (record.state.clone(), record.can_recheck()),
            None => return Vec::new(),
        },
        Family::Resource => match control::resource_record(m, key) {
            Some(record) => (record.state.clone(), true),
            None => return Vec::new(),
        },
    };
    let reason = |eligibility: Eligibility| match eligibility {
        Eligibility::Enabled => Some(None),
        Eligibility::Disabled(reason) => Some(Some(reason)),
        Eligibility::Hidden => None,
    };
    let mut buttons = Vec::new();
    if can_explain(m, key) {
        let prefix = match key.family {
            Family::Vm => "vm",
            Family::Resource => "resource",
        };
        buttons.push(button(
            format!("{prefix}-failure-help-{id}"),
            "Explain outcome…",
            move |m| {
                explain(m, key);
            },
        ));
    }
    if let Some(disabled) = reason(cancel_eligibility(m, key)) {
        buttons.push(
            button(
                format!("operation-cancel-{id}"),
                cancel_label(m, key),
                move |m| m.error = cancel(m, key).err(),
            )
            .disabled_opt(disabled),
        );
    }
    if state == OperationState::OutcomeUnknown {
        if can_recheck && let Some(disabled) = reason(recheck_eligibility(m, key)) {
            buttons.push(
                button(format!("operation-recheck-{id}"), "Recheck outcome", move |m| {
                    m.error = recheck(m, key).err()
                })
                .disabled_opt(disabled),
            );
        }
        if let Some(disabled) = reason(acknowledge_eligibility(m, key)) {
            buttons.push(
                button(
                    format!("operation-acknowledge-{id}"),
                    match key.family {
                        Family::Vm => "Review unknown outcome",
                        Family::Resource => "Acknowledge unknown outcome…",
                    },
                    move |m| {
                        acknowledge(m, key);
                    },
                )
                .disabled_opt(disabled),
            );
        }
    }
    buttons
}
