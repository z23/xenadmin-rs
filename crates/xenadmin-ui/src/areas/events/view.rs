//! The two tabs and the four dialogs, described. Nothing here changes the
//! model: each description is rebuilt on every render.
//!
//! Rune keys kept: `activity-search`, `activity-state-filter`,
//! `activity-pool` (the button that cycled the pools is a select of them),
//! `activity-show-hidden`, `activity-table`, `activity-confirm`,
//! `activity-Tasks` and `activity-History` (options of the list choice),
//! `failure-help-dialog` (the dialog's id), `failure-help-outcome`,
//! `failure-help-code`, `failure-help-close`.
//!
//! Rune keys that are commands now (the key of a command's button is its
//! id): `activity-review`, `activity-failure-help`, `activity-hide`,
//! `activity-recheck`, `activity-export`.
//!
//! New keys, for elements Rune left without one: `activity-list`,
//! `activity-notice`, `activity-lists-note`, `activity-inventory`,
//! `activity-count`, `activity-row-source`, `activity-outcome`,
//! `activity-row-name`, `activity-unresolved`, `activity-export-note`;
//! in the two reviews `activity-change-review` (and so
//! `activity-change-review-stale`), `activity-review-preview`,
//! `activity-review-error`, `activity-review-close`; in the acknowledgement
//! `acknowledge-review`, `acknowledge-preview`, `acknowledge-error`,
//! `acknowledge-cancel`, `acknowledge-confirm`; in the explanation
//! `failure-help-source`, `failure-help-captured`.

use xenadmin_core::{OperationState, ResourceTarget};

use super::activity::{self, DEMO, Row, Tab, Target as RowTarget};
use super::commands::{ALERTS_MENU, EVENTS_MENU};
use super::control::{self, Acknowledge};
use super::evidence;
use super::failure_help::{self, FailureHelp};
use super::hidden::Family;
use crate::prelude::*;

pub const LISTS_NOTE: &str = "Lists use the latest observed inventory for each pool. Offline or switched-pool records remain visible; actions require a current writable connection to that pool.";
pub const EXPORT_NOTE: &str = "JSON exports contain the displayed task/message fields and public operation details, including message text. Files are saved locally with owner-only permissions.";
pub const SOURCE_SYNTHETIC: &str = "Synthetic fixture · host actions unavailable";
pub const SOURCE_CURRENT: &str = "Current connected pool";
pub const SOURCE_CACHED: &str =
    "Cached inventory · this pool is not currently selected and connected";
pub const CANCEL_CONSEQUENCE: &str = "The host may finish normally, fail, or continue working. This requests cancellation and tracks fresh status; it does not release the original operation's reservation.";
pub const DISMISS_CONSEQUENCE: &str = "This removes the reviewed message from the host for all clients. It does not resolve the underlying condition or change alarm configuration.";
pub const ACK_PREVIEW: &str = "Synthetic preview · host changes cannot be submitted.";

/// The id of the pool filter's first option.
const ALL_POOLS: &str = "all";

// -- Tabs ---------------------------------------------------------------------

pub fn tab(m: &Model, tab: TabId, _scope: &Target) -> Option<TabBody> {
    if !matches!(tab, TabId::Alerts | TabId::Events) {
        return None;
    }
    let a = &m.areas.events;
    let list = activity::list_of(m, tab);
    let sheet = activity::current(m, list);
    let selected = a
        .selected
        .as_ref()
        .and_then(|key| sheet.rows.iter().find(|row| &row.key == key));
    let table = Table {
        rows: sheet.table.clone(),
        ..table(
            "activity-table",
            [
                column("name", "Name / action"),
                column(
                    "status",
                    if list == Tab::Messages {
                        "Priority"
                    } else {
                        "State"
                    },
                ),
            ],
            Vec::<crate::surface::Row>::new(),
        )
    }
    .single(selected.map(|row| row.key.as_str()), |m, id| {
        m.areas.events.selected = Some(id.to_owned());
    })
    .row_menu(if tab == TabId::Alerts {
        ALERTS_MENU.to_vec()
    } else {
        EVENTS_MENU.to_vec()
    })
    .empty("No matching entries");
    Some(TabBody::Blocks(blocks![
        a.notice
            .clone()
            .map(|text| notice(NoticeKind::Info, text).key("activity-notice")),
        (tab == TabId::Events).then(|| list_choice(list)),
        filters(m, list),
        scope_notes(m, list),
        caption(format!("{} matching entries", sheet.rows.len())).key("activity-count"),
        table,
        selected.map(|row| details(m, row)),
        caption(EXPORT_NOTE).key("activity-export-note"),
    ]))
}

/// Tasks or History: the two lists of the Events tab. Switching clears the
/// state filter and keeps the search, as switching the Rune tab did.
fn list_choice(list: Tab) -> Choice {
    let id = |list: Tab| if list == Tab::History { "history" } else { "tasks" };
    choice(
        "activity-list",
        [
            option(id(Tab::Tasks), Tab::Tasks.title()).key("activity-Tasks"),
            option(id(Tab::History), Tab::History.title()).key("activity-History"),
        ],
        Some(id(list)),
        |m, id| {
            let list = if id == "history" {
                Tab::History
            } else {
                Tab::Tasks
            };
            if m.areas.events.list != list {
                m.areas.events.list = list;
                m.areas.events.state_filter.clear();
            }
        },
    )
    .segmented()
}

fn filters(m: &Model, list: Tab) -> Block {
    let a = &m.areas.events;
    let messages = list == Tab::Messages;
    let pools = activity::observed_pools(m);
    let mut options = vec![option(ALL_POOLS, "All observed pools")];
    for pool in activity::known_pools(m) {
        options.push(option(
            pool.to_string(),
            pools
                .get(&pool)
                .map(|i| i.pool_name.clone())
                .unwrap_or_else(|| pool.to_string()),
        ));
    }
    let chosen = a
        .pool
        .map(|pool| pool.to_string())
        .unwrap_or_else(|| ALL_POOLS.into());
    columns([
        blocks![
            input("activity-search", "Search activity", &a.search, |m, text| {
                m.areas.events.search = text.to_owned();
            })
            .placeholder("Search name, text or UUID")
        ],
        blocks![
            input(
                "activity-state-filter",
                if messages {
                    "Priority filter"
                } else {
                    "State filter"
                },
                activity::status_filter(m, list),
                move |m, text| {
                    if messages {
                        m.areas.events.priority_filter = text.to_owned();
                    } else {
                        m.areas.events.state_filter = text.to_owned();
                    }
                }
            )
            .placeholder(if messages { "Priority" } else { "State" })
        ],
        blocks![select(
            "activity-pool",
            "Pool",
            options,
            Some(chosen.as_str()),
            |m, id| {
                m.areas.events.pool = activity::known_pools(m)
                    .into_iter()
                    .find(|pool| pool.to_string() == id);
            }
        )],
    ])
}

/// Under the filters: for the host's lists where they come from, for the
/// history the option to include hidden entries.
fn scope_notes(m: &Model, list: Tab) -> Vec<Block> {
    let a = &m.areas.events;
    if list == Tab::History {
        return blocks![checkbox(
            "activity-show-hidden",
            "Include locally hidden completed entries",
            a.show_hidden,
            |m| m.areas.events.show_hidden = !m.areas.events.show_hidden,
        )];
    }
    let class = if list == Tab::Tasks {
        "task"
    } else {
        "message"
    };
    let pools = activity::observed_pools(m);
    let inventory = a
        .pool
        .and_then(|pool| pools.get(&pool))
        .and_then(|pool| pool.details.classes.get(class))
        .map(|status| caption(format!("Inventory: {status:?}")).key("activity-inventory"));
    blocks![caption(LISTS_NOTE).key("activity-lists-note"), inventory]
}

/// Where a row's data comes from, in the Rune words.
fn source(m: &Model, row: &Row) -> &'static str {
    if activity::synthetic(m, row.pool) {
        SOURCE_SYNTHETIC
    } else if m.snapshot_for(row.pool).is_some_and(|snapshot| {
        snapshot.is_current() && snapshot.inventory().is_some_and(|i| i.pool_id == row.pool)
    }) {
        SOURCE_CURRENT
    } else {
        SOURCE_CACHED
    }
}

fn operation_state(state: &OperationState, progress: Option<f64>) -> OutcomeState {
    match state {
        OperationState::Succeeded => OutcomeState::Succeeded,
        OperationState::PartiallyCompleted => OutcomeState::Partial,
        OperationState::Failed | OperationState::Rejected => OutcomeState::Failed,
        OperationState::Cancelled => OutcomeState::Cancelled,
        OperationState::OutcomeUnknown | OperationState::AcknowledgedUnknown => {
            OutcomeState::Unknown
        }
        _ => OutcomeState::Running(progress.map(|p| p.clamp(0.0, 1.0) as f32)),
    }
}

/// The selected row: its source, then what it is. An operation record and
/// a task are an outcome with its progress, partial or unknown state; a
/// message is its text.
fn details(m: &Model, row: &Row) -> Vec<Block> {
    let lines = row
        .details
        .lines()
        .filter(|line| !line.is_empty())
        .map(str::to_owned);
    let title = format!("{} · {}", row.name, row.status);
    let body: Vec<Block> = match row.target {
        RowTarget::History(key) => {
            let (state, more) = match key.family {
                Family::Vm => control::vm_record(m, key)
                    .map(|r| (operation_state(&r.state, r.progress), evidence::vm(r))),
                Family::Resource => control::resource_record(m, key)
                    .map(|r| (operation_state(&r.state, r.progress), evidence::resource(r))),
            }
            .unwrap_or((OutcomeState::Unknown, Vec::new()));
            blocks![
                outcome("activity-outcome", state, title)
                    .lines(lines)
                    .lines(more),
                (!row.terminal).then(|| caption(activity::UNRESOLVED).key("activity-unresolved")),
            ]
        }
        RowTarget::Resource(ResourceTarget::Task(id)) => {
            let progress = activity::observed_pools(m)
                .get(&row.pool)
                .and_then(|i| i.details.tasks.get(&id))
                .and_then(|task| task.progress);
            let state = match row.status.as_str() {
                "success" => OutcomeState::Succeeded,
                "failure" => OutcomeState::Failed,
                "cancelled" => OutcomeState::Cancelled,
                _ => OutcomeState::Running(progress.map(|p| p.clamp(0.0, 1.0) as f32)),
            };
            blocks![outcome("activity-outcome", state, title).lines(lines)]
        }
        RowTarget::Resource(_) => blocks![
            heading(row.name.clone()),
            lines.map(caption).collect::<Vec<_>>(),
        ],
    };
    blocks![
        divider(),
        caption(source(m, row)).key("activity-row-source"),
        body,
    ]
}

// -- Dialogs ------------------------------------------------------------------

/// Back to front: a review, the acknowledgement, the explanation. The
/// explanation can be asked for from anywhere, so it is in front.
pub fn surfaces(m: &Model) -> Vec<Surface> {
    let a = &m.areas.events;
    let mut surfaces = Vec::new();
    surfaces.extend(a.review.as_ref().map(|review| review_surface(m, review)));
    surfaces.extend(a.acknowledge.as_ref().map(|ack| acknowledge_surface(m, ack)));
    surfaces.extend(a.help.as_ref().map(help_surface));
    surfaces
}

fn close_review(m: &mut Model) {
    m.areas.events.review = None;
}

/// The review of a task cancellation or a message dismissal: the captured
/// name and details, what confirming does and does not do, and why it
/// cannot be confirmed when it cannot. It opens on the review (there is
/// nothing to enter), so it has no Back.
fn review_surface(m: &Model, review: &activity::Review) -> Surface {
    let cancel = matches!(review.target, ResourceTarget::Task(_));
    let synthetic = activity::synthetic(m, review.pool);
    let stale = activity::review_stale(m, review);
    let permission = activity::review_action(review).map(|action| {
        permissions::assess_for(
            m,
            review.pool,
            &action.permission_requirements(review.target),
        )
    });
    let lines = std::iter::once(review.name.clone()).chain(
        review
            .details
            .lines()
            .filter(|line| !line.is_empty())
            .map(str::to_owned),
    );
    let confirm = button(
        "activity-confirm",
        if cancel {
            "Request cancellation"
        } else {
            "Dismiss message"
        },
        activity::submit,
    )
    .danger()
    // The first reason wins: stale, then a known denial, then the demo.
    .disabled_opt(stale)
    .disabled_opt(permission.as_ref().and_then(permissions::denial))
    .disabled_if(synthetic, DEMO);
    let surface = Surface::dialog(
        if cancel {
            "task-cancellation"
        } else {
            "message-dismissal"
        },
        if cancel {
            "Request task cancellation?"
        } else {
            "Dismiss this server message?"
        },
    )
    .body(blocks![
        review_block(lines, cancel, stale),
        synthetic.then(|| notice(NoticeKind::Preview, SOURCE_SYNTHETIC)
            .key("activity-review-preview")),
        review
            .error
            .clone()
            .map(|error| alert(error).key("activity-review-error")),
    ])
    .footer([
        button("activity-review-close", "Close", close_review),
        confirm,
    ])
    .on_close(close_review);
    match &permission {
        Some(permission) => surface.footer_note(permissions::footer_note(permission)),
        None => surface,
    }
}

fn review_block(
    lines: impl Iterator<Item = String>,
    cancel: bool,
    stale: Option<&'static str>,
) -> Review {
    review("activity-change-review", lines)
        .warning(if cancel {
            CANCEL_CONSEQUENCE
        } else {
            DISMISS_CONSEQUENCE
        })
        .stale(stale)
}

fn close_acknowledge(m: &mut Model) {
    m.areas.events.acknowledge = None;
}

/// "Acknowledge unknown outcome?": what is released and what is not
/// established. One review, two dialogs by family, because what they say
/// differs in substance: a VM operation releases this client's block on one
/// VM, a resource operation the write guard of a pool (or of two).
fn acknowledge_surface(m: &Model, ack: &Acknowledge) -> Surface {
    let synthetic = control::acknowledge_synthetic(m, ack);
    let stale = control::acknowledge_stale(m, ack);
    if ack.key.family == Family::Vm {
        return unknown_outcome(ack, synthetic, stale);
    }
    Surface::dialog("acknowledge-unknown", "Acknowledge unknown outcome?")
        .small()
        .body(blocks![
            review("acknowledge-review", ack.lines.clone()).stale(stale),
            synthetic.then(|| notice(NoticeKind::Preview, ACK_PREVIEW).key("acknowledge-preview")),
            ack.error
                .clone()
                .map(|error| alert(error).key("acknowledge-error")),
        ])
        .footer([
            button("acknowledge-cancel", "Cancel", close_acknowledge),
            button(
                "acknowledge-confirm",
                "Acknowledge outcome",
                control::confirm_acknowledge,
            )
            .danger()
            .disabled_opt(stale)
            .disabled_if(synthetic, DEMO),
        ])
        .on_close(close_acknowledge)
}

/// The acknowledgement of a VM operation: the Rune `unknown_review` dialog
/// of `main.rs`, with its key (`unknown-outcome`), its title, its heading,
/// its two captions and its Warning button. It is reached from the Events
/// tab (`events.acknowledge`), from a record of a VM's General tab and from
/// the power area's "Review unknown outcome" alike.
///
/// Beyond Rune: the reason it cannot be confirmed any more
/// (`unknown-stale`), the synthetic-data notice (`unknown-preview`) and a
/// refusal of the host (`unknown-error`), which keeps the dialog open.
fn unknown_outcome(ack: &Acknowledge, synthetic: bool, stale: Option<&'static str>) -> Surface {
    let mut lines = ack.lines.iter();
    let target = lines.next().cloned().unwrap_or_default();
    let consequence: Vec<Text> = lines
        .enumerate()
        .map(|(index, line)| {
            caption(line.clone()).key(if index == 0 {
                "unknown-consequence".to_owned()
            } else {
                format!("unknown-consequence-{index}")
            })
        })
        .collect();
    Surface::dialog("unknown-outcome", "Acknowledge unknown outcome")
        .small()
        .body(blocks![
            heading("Acknowledge unknown outcome?"),
            caption(target).key("unknown-target"),
            consequence,
            stale.map(|reason| alert(reason).key("unknown-stale")),
            synthetic.then(|| notice(NoticeKind::Preview, ACK_PREVIEW).key("unknown-preview")),
            ack.error
                .clone()
                .map(|error| alert(error).key("unknown-error")),
        ])
        .footer([
            button("unknown-cancel", "Cancel", close_acknowledge),
            button(
                "unknown-acknowledge",
                "Acknowledge outcome",
                control::confirm_acknowledge,
            )
            .warning()
            .disabled_opt(stale)
            .disabled_if(synthetic, DEMO),
        ])
        .on_close(close_acknowledge)
}

fn close_help(m: &mut Model) {
    m.areas.events.help = None;
}

/// "Outcome explanation": the captured source, what the recorded outcome
/// means, and the core's guidance for an exact host error code.
fn help_surface(help: &FailureHelp) -> Surface {
    let guidance: Vec<Block> = match help.guide {
        Some(guide) => blocks![
            heading(guide.title),
            caption(guide.explanation),
            guide
                .checks
                .iter()
                .enumerate()
                .map(|(i, check)| caption(format!("{}. {check}", i + 1)))
                .collect::<Vec<_>>(),
        ],
        None => blocks![caption(failure_help::NO_CODE)],
    };
    Surface::dialog("failure-help-dialog", "Outcome explanation")
        .body(blocks![
            help.source
                .lines()
                .enumerate()
                .map(|(i, line)| caption(line).key(if i == 0 {
                    "failure-help-source".to_owned()
                } else {
                    format!("failure-help-source-{i}")
                }))
                .collect::<Vec<_>>(),
            caption(failure_help::CAPTURED).key("failure-help-captured"),
            heading("Recorded outcome"),
            caption(help.outcome).key("failure-help-outcome"),
            help.code.clone().map(|code| properties([property("Host error", code)
                .key("failure-help-code")
                .copyable()])),
            guidance,
        ])
        .footer([button("failure-help-close", "Close", close_help)])
        .on_close(close_help)
}
