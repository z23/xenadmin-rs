//! The commands of the area: the buttons the Rune Activity dialog showed
//! under its selected row, the buttons under a record in the details pane
//! and the pool history, and the export.
//!
//! They act on the selected row of the list on screen
//! (`m.areas.events.selected`), not on the `Target` they are handed: a row
//! of these lists is a task, a message or an operation record, and none of
//! them is an object of the tree.
//!
//! Eligibility, one rule: `Hidden` where Rune showed no such button (no row
//! selected, a row of another kind, a finished record for a control);
//! `Disabled` with the Rune reason where the button was there and disabled;
//! for the buttons Rune disabled without a word, the reason is the one
//! `control` gives.

use super::activity::{self, Row, Tab, Target as RowTarget};
use super::control;
use super::hidden::HiddenKey;
use crate::prelude::*;
use xenadmin_core::ResourceTarget;

const AREA: &str = "events";

pub const CANCEL_TASK: CommandId = CommandId("events.cancel-task");
pub const EXPLAIN: CommandId = CommandId("events.explain");
pub const HIDE: CommandId = CommandId("events.hide");
pub const RECHECK: CommandId = CommandId("events.recheck");
pub const ACKNOWLEDGE: CommandId = CommandId("events.acknowledge");
pub const REQUEST_CANCEL: CommandId = CommandId("events.request-cancel");
pub const EXPORT: CommandId = CommandId("events.export");
pub const DISMISS: CommandId = CommandId("alerts.dismiss");

/// The row menu of the Events table.
pub const EVENTS_MENU: [CommandId; 6] = [
    CANCEL_TASK,
    EXPLAIN,
    HIDE,
    RECHECK,
    ACKNOWLEDGE,
    REQUEST_CANCEL,
];
/// The row menu of the Alerts table.
pub const ALERTS_MENU: [CommandId; 1] = [DISMISS];

/// Why the export command is disabled while one is in flight: the notice
/// Rune showed during it.
pub const EXPORTING: &str = "Saving the captured activity selection…";

pub fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new(CANCEL_TASK.0, "Review cancellation…", AREA)
            .local(TabId::Events, 10)
            .local_scopes(&ObjectKind::ALL)
            .eligibility(|m, _| review_eligibility(m, events_row(m), true))
            .invoke(|m, _| {
                let list = m.areas.events.list;
                activity::begin_review(m, list);
            }),
        CommandSpec::new(EXPLAIN.0, "Explain outcome…", AREA)
            .local(TabId::Events, 20)
            .local_scopes(&ObjectKind::ALL)
            .eligibility(|m, _| match events_row(m).and_then(|row| row.help) {
                Some(_) => Eligibility::Enabled,
                None => Eligibility::Hidden,
            })
            .invoke(|m, _| {
                if let Some(help) = events_row(m).and_then(|row| row.help) {
                    super::show_help(m, help);
                }
            }),
        CommandSpec::new(HIDE.0, "Hide completed entry", AREA)
            .local(TabId::Events, 30)
            .local_scopes(&ObjectKind::ALL)
            .eligibility(|m, _| match history_row(m) {
                Some(row) => Eligibility::enabled_if(row.terminal, activity::UNRESOLVED),
                None => Eligibility::Hidden,
            })
            .dynamic_label(|m, _| {
                history_row(m)
                    .filter(|row| row.hidden)
                    .map(|_| "Restore entry".to_owned())
            })
            .invoke(|m, _| activity::toggle_hidden(m)),
        CommandSpec::new(RECHECK.0, "Recheck outcome", AREA)
            .local(TabId::Events, 40)
            .local_scopes(&ObjectKind::ALL)
            .eligibility(|m, _| on_record(m, control::recheck_eligibility))
            .invoke(|m, _| {
                if let Some(key) = history_key(m) {
                    m.areas.events.notice = Some(match control::recheck(m, key) {
                        Ok(notice) => notice.to_owned(),
                        Err(error) => error,
                    });
                }
            }),
        CommandSpec::new(ACKNOWLEDGE.0, "Acknowledge unknown outcome…", AREA)
            .local(TabId::Events, 50)
            .local_scopes(&ObjectKind::ALL)
            .eligibility(|m, _| on_record(m, control::acknowledge_eligibility))
            .invoke(|m, _| {
                if let Some(key) = history_key(m) {
                    control::open_acknowledge(m, key);
                }
            }),
        CommandSpec::new(REQUEST_CANCEL.0, "Request cancellation", AREA)
            .local(TabId::Events, 60)
            .local_scopes(&ObjectKind::ALL)
            .eligibility(|m, _| on_record(m, control::cancel_eligibility))
            .dynamic_label(|m, _| {
                history_key(m).map(|key| control::cancel_label(m, key).to_owned())
            })
            .invoke(|m, _| {
                if let Some(key) = history_key(m) {
                    m.areas.events.notice = control::cancel(m, key).err();
                }
            }),
        // One command on both tabs: it exports the list that is shown, the
        // messages on Alerts and otherwise the list of the Events tab.
        CommandSpec::new(EXPORT.0, "Export filtered entries…", AREA)
            .local(TabId::Events, 90)
            .local(TabId::Alerts, 90)
            .local_scopes(&ObjectKind::ALL)
            .eligibility(|m, _| export_eligibility(m))
            .invoke(|m, _| {
                let list = activity::shown(m).unwrap_or(m.areas.events.list);
                super::export::choose(m, list);
            }),
        CommandSpec::new(DISMISS.0, "Review dismissal…", AREA)
            .local(TabId::Alerts, 10)
            .local_scopes(&ObjectKind::ALL)
            .eligibility(|m, _| {
                review_eligibility(m, activity::selected(m, Tab::Messages), false)
            })
            .invoke(|m, _| activity::begin_review(m, Tab::Messages)),
    ]
}

/// The selected row of the list the Events tab shows.
fn events_row(m: &Model) -> Option<Row> {
    activity::selected(m, m.areas.events.list)
}

/// The selected row of the History list, when the Events tab shows it.
fn history_row(m: &Model) -> Option<Row> {
    (m.areas.events.list == Tab::History)
        .then(|| activity::selected(m, Tab::History))
        .flatten()
}

/// The operation the selected row of the History list is the record of.
pub(super) fn history_key(m: &Model) -> Option<HiddenKey> {
    match history_row(m)?.target {
        RowTarget::History(key) => Some(key),
        RowTarget::Resource(_) => None,
    }
}

fn on_record(m: &Model, rule: fn(&Model, HiddenKey) -> Eligibility) -> Eligibility {
    history_key(m).map_or(Eligibility::Hidden, |key| rule(m, key))
}

/// "Review cancellation…" of a task row and "Review dismissal…" of a
/// message row: in a synthetic pool always (the review cannot be confirmed
/// there), otherwise when the host object allows it now.
fn review_eligibility(m: &Model, row: Option<Row>, task: bool) -> Eligibility {
    let Some(row) = row else {
        return Eligibility::Hidden;
    };
    match row.target {
        RowTarget::Resource(ResourceTarget::Task(_)) if task => {}
        RowTarget::Resource(ResourceTarget::Message(_)) if !task => {}
        _ => return Eligibility::Hidden,
    }
    Eligibility::enabled_if(
        activity::synthetic(m, row.pool) || activity::allowed(m, &row),
        if task {
            activity::CANCEL_REQUIRES
        } else {
            activity::DISMISS_REQUIRES
        },
    )
}

fn export_eligibility(m: &Model) -> Eligibility {
    Eligibility::enabled_if(!m.areas.events.export.pending, EXPORTING)
}
