//! The three lists of the Rune Activity dialog and what is done from them:
//! pool tasks with reviewed cancellation, server messages with reviewed
//! dismissal, and the local history of both operation families with hiding,
//! restoring and rechecking.
//!
//! The Rune `activity.rs` without its view. The row texts, the filters, the
//! order, the review capture, the staleness check and the submission are
//! word for word. What changed:
//!
//! - `Tab::Messages` is the Alerts tab of the Notifications view; `Tasks`
//!   and `History` are the two lists of its Events tab (`State::list`).
//! - Rune rebuilt `rows` in every handler and kept them in the state. Here
//!   [`current`] builds the rows of a list on demand and remembers the last
//!   result with the inputs it was built from, so no handler has to rebuild
//!   and a description never shows rows older than the model.
//! - An action follows the pool of its row (`can_mutate_pool`,
//!   `snapshot_for`, `connection_for`), not the scope session. Rune had one
//!   selected connection and required it to be the row's pool.
//! - Every session's inventory is an observed pool, not only the pools that
//!   have been the selected connection: the tree shows them all at once.
//! - The pool filter matches a resource record by `applies_to_pool`, as the
//!   Rune pool history did: a join is listed at its destination too.
//! - The state filter (Tasks, History) and the priority filter (Alerts) are
//!   two fields. Rune had one and cleared it when the tab changed; the
//!   switch between Tasks and History still clears the state filter.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;

use xenadmin_core::{
    Error, Inventory, ObservedResource, OperationId, OperationState, OperationStep, PoolId,
    ResourceAction, ResourceCommand, ResourceTarget,
};

use super::export::Entry;
use super::failure_help::FailureHelp;
use super::hidden::{Family, HiddenKey};
use crate::model::Model;
use crate::nav::NavView;
use crate::permissions;
use crate::session;
use crate::tabs::TabId;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    Tasks,
    Messages,
    History,
}
impl Tab {
    pub fn title(self) -> &'static str {
        match self {
            Self::Tasks => "Tasks",
            Self::Messages => "Messages",
            Self::History => "History",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Resource(ResourceTarget),
    History(HiddenKey),
}
#[derive(Clone)]
pub struct Row {
    pub key: String,
    pub pool: PoolId,
    /// The other pool a resource record applies to: the destination of a
    /// join or of a cross-pool transfer.
    pub peer: Option<PoolId>,
    pub name: String,
    pub status: String,
    pub details: String,
    pub target: Target,
    pub terminal: bool,
    pub hidden: bool,
    pub export: Entry,
    pub help: Option<FailureHelp>,
}
/// An open review of a task cancellation or a message dismissal.
pub struct Review {
    pub pool: PoolId,
    pub target: ResourceTarget,
    pub observed: Option<ObservedResource>,
    pub digest: Option<[u8; 32]>,
    pub name: String,
    pub details: String,
    /// Why the last confirmation did nothing. (Rune wrote it to the
    /// dialog's status line, which the review shared.)
    pub error: Option<String>,
}

type Revision = (
    PoolId,
    OperationId,
    Family,
    u64,
    OperationState,
    Option<String>,
);

/// The rows of one list with the inputs they were built from.
pub struct Sheet {
    tab: Tab,
    search: String,
    status: String,
    pool: Option<PoolId>,
    show_hidden: bool,
    hidden: Vec<HiddenKey>,
    sources: Vec<usize>,
    revisions: Vec<Revision>,
    pub rows: Vec<Row>,
    /// The same rows as the table shows them.
    pub table: Rc<Vec<crate::surface::Row>>,
}

/// What the lists remember between renders. Private to this module.
#[derive(Default)]
pub struct Cache {
    /// The latest inventory seen for each pool. Kept after a disconnect.
    pools: BTreeMap<PoolId, Arc<Inventory>>,
    /// The change detector of `poll` over both record lists.
    history_revisions: Vec<Revision>,
    /// The last sheet built. A memo: filling it changes nothing a
    /// description can see.
    sheet: RefCell<Option<Rc<Sheet>>>,
}

pub const STALE_REVIEW: &str = "Connection or object changed. Reopen the review before proceeding.";
pub const STALE_SUBMIT: &str = "The connection or reviewed object changed. Reopen the review.";
pub const SUBMITTED: &str = "Change submitted. History tracks the host outcome.";
pub const HIDE_LIMIT: &str = "Restore hidden history entries before hiding more.";
pub const UNRESOLVED: &str = "Unresolved operations remain visible and cannot be hidden. Rechecking never repeats a mutation.";
pub const CANCEL_REQUIRES: &str = "Cancellation requires a current writable connection, a pending task, and the host's advertised cancel capability.";
pub const DISMISS_REQUIRES: &str =
    "Message dismissal requires a current writable connection to its pool.";
/// The reason every area gives a disabled confirm button in a synthetic pool.
pub const DEMO: &str = "Demo mode cannot change infrastructure.";

/// The list a tab of the Notifications view shows.
pub fn list_of(m: &Model, tab: TabId) -> Tab {
    if tab == TabId::Alerts {
        Tab::Messages
    } else {
        m.areas.events.list
    }
}

/// The list on screen, when the Notifications view is.
pub fn shown(m: &Model) -> Option<Tab> {
    (m.nav.view == NavView::Notifications).then(|| list_of(m, m.active_tab()))
}

/// The status filter of a list: the priority filter on Alerts, the state
/// filter on Tasks and History.
pub fn status_filter(m: &Model, tab: Tab) -> &str {
    if tab == Tab::Messages {
        &m.areas.events.priority_filter
    } else {
        &m.areas.events.state_filter
    }
}

/// Whether the pool is shown from a synthetic inventory: the Rune `s.demo`
/// for the pool of a row.
pub fn synthetic(m: &Model, pool: PoolId) -> bool {
    match session::session_for_pool(m, pool) {
        Some(id) if m.workspace.selected == Some(id) => m.demo,
        Some(id) => m.workspace.synthetic.contains_key(&id),
        None => false,
    }
}

fn live_inventories(m: &Model) -> Vec<Arc<Inventory>> {
    let mut found: Vec<Arc<Inventory>> = m
        .workspace
        .profiles()
        .iter()
        .filter_map(|profile| session::inventory_of(m, profile.id))
        .collect();
    // The scope session last: it wins where two sessions show one pool.
    found.extend(m.inventory.clone());
    found
}

/// The latest inventory of every pool seen so far: the remembered ones with
/// the live ones over them.
pub fn observed_pools(m: &Model) -> BTreeMap<PoolId, Arc<Inventory>> {
    let mut pools = m.areas.events.cache.pools.clone();
    for inventory in live_inventories(m) {
        pools.insert(inventory.pool_id, inventory);
    }
    pools
}

/// Every pool the filter can name: the observed ones and those of both
/// record lists, sorted.
pub fn known_pools(m: &Model) -> Vec<PoolId> {
    let mut pools = observed_pools(m)
        .keys()
        .copied()
        .chain(m.operation_rows.iter().map(|r| r.pool))
        .chain(m.resource_rows.iter().map(|r| r.pool))
        .collect::<Vec<_>>();
    pools.sort();
    pools.dedup();
    pools
}

fn revisions(m: &Model) -> impl Iterator<Item = Revision> + '_ {
    m.operation_rows
        .iter()
        .map(|r| {
            (
                r.pool,
                r.id,
                Family::Vm,
                r.revision(),
                r.state.clone(),
                r.local_error.clone(),
            )
        })
        .chain(m.resource_rows.iter().map(|r| {
            (
                r.pool,
                r.id,
                Family::Resource,
                r.revision(),
                r.state.clone(),
                r.local_error.clone(),
            )
        }))
}

/// Remembers the inventories and notices changed records: the Rune `poll`
/// without the export and without the rebuild, which [`current`] makes
/// unnecessary. Returns whether anything a list shows changed.
pub fn poll(m: &mut Model) -> bool {
    let mut changed = false;
    for inventory in live_inventories(m) {
        let old = m.areas.events.cache.pools.get(&inventory.pool_id);
        if !old.is_some_and(|old| Arc::ptr_eq(old, &inventory)) {
            m.areas
                .events
                .cache
                .pools
                .insert(inventory.pool_id, inventory);
            changed = true;
        }
    }
    let revisions = revisions(m).collect::<Vec<_>>();
    if revisions != m.areas.events.cache.history_revisions {
        m.areas.events.cache.history_revisions = revisions;
        changed = true;
    }
    changed
}

pub fn row_key(pool: PoolId, kind: &str, id: impl std::fmt::Display) -> String {
    format!("{pool}/{kind}/{id}")
}
pub fn history_key(pool: PoolId, id: OperationId, family: Family) -> HiddenKey {
    HiddenKey { pool, id, family }
}
/// The row key of a history entry.
pub fn key_of(key: HiddenKey) -> String {
    row_key(
        key.pool,
        match key.family {
            Family::Vm => "vm",
            Family::Resource => "resource",
        },
        key.id,
    )
}
fn hidden(m: &Model, key: HiddenKey, terminal: bool) -> bool {
    terminal && m.preferences.hidden_history.contains(&key)
}

fn sources(tab: Tab, pools: &BTreeMap<PoolId, Arc<Inventory>>) -> Vec<usize> {
    if tab == Tab::History {
        return Vec::new();
    }
    pools
        .values()
        .map(|inventory| Arc::as_ptr(inventory) as usize)
        .collect()
}

fn fresh(sheet: &Sheet, m: &Model, tab: Tab, pools: &BTreeMap<PoolId, Arc<Inventory>>) -> bool {
    let a = &m.areas.events;
    sheet.tab == tab
        && sheet.search == a.search
        && sheet.status == status_filter(m, tab)
        && sheet.pool == a.pool
        && sheet.show_hidden == a.show_hidden
        && sheet.sources == sources(tab, pools)
        && (tab != Tab::History
            || sheet.hidden == m.preferences.hidden_history
                && sheet.revisions.iter().cloned().eq(revisions(m)))
}

/// The rows of a list as the model stands: the remembered sheet when
/// nothing it was built from changed, a new one otherwise.
pub fn current(m: &Model, tab: Tab) -> Rc<Sheet> {
    let pools = observed_pools(m);
    let cache = &m.areas.events.cache.sheet;
    if let Some(sheet) = cache.borrow().as_ref()
        && fresh(sheet, m, tab, &pools)
    {
        return sheet.clone();
    }
    let sheet = Rc::new(build(m, tab, &pools));
    *cache.borrow_mut() = Some(sheet.clone());
    sheet
}

/// Forgets the remembered sheet. For a test that changes an inventory in
/// place, which no comparison of the inputs can see.
#[cfg(test)]
pub fn invalidate(m: &Model) {
    *m.areas.events.cache.sheet.borrow_mut() = None;
}

/// The Rune `rebuild`.
fn build(m: &Model, tab: Tab, pools: &BTreeMap<PoolId, Arc<Inventory>>) -> Sheet {
    let a = &m.areas.events;
    let mut rows = Vec::new();
    match tab {
        Tab::Tasks => {
            for (&pool, inventory) in pools {
                for task in inventory.details.tasks.values() {
                    rows.push(Row {
                        key: row_key(pool, "task", task.id),
                        pool,
                        peer: None,
                        name: task.name.clone(),
                        status: task.status.clone(),
                        details: format!(
                            "{}\nTask {}\nPool {}\nCreated: {}\nFinished: {}\nProgress: {}\nHost: {}\nParent task: {}\nSubtasks: {}\nResult type: {}\n{}",
                            task.description,
                            task.id,
                            pool,
                            task.created.as_deref().unwrap_or("Unknown"),
                            task.finished.as_deref().unwrap_or("Not reported"),
                            task.progress
                                .map(|p| format!("{:.0}%", p * 100.0))
                                .unwrap_or_else(|| "Unavailable".into()),
                            task.resident_host
                                .map(|id| inventory
                                    .hosts
                                    .get(&id)
                                    .map(|h| format!("{} ({id})", h.name))
                                    .unwrap_or_else(|| id.to_string()))
                                .unwrap_or_else(|| "Not reported".into()),
                            task.parent
                                .map(|id| id.to_string())
                                .unwrap_or_else(|| "None reported".into()),
                            if task.subtasks.is_empty() {
                                "None reported".into()
                            } else {
                                task.subtasks
                                    .iter()
                                    .map(ToString::to_string)
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            },
                            task.result_type.as_deref().unwrap_or("Not reported"),
                            task.error_code.as_deref().unwrap_or("")
                        ),
                        target: Target::Resource(ResourceTarget::Task(task.id)),
                        terminal: matches!(
                            task.status.as_str(),
                            "success" | "failure" | "cancelled"
                        ),
                        hidden: false,
                        export: Entry::task(pool, task),
                        help: FailureHelp::task(pool, task),
                    });
                }
            }
        }
        Tab::Messages => {
            for (&pool, inventory) in pools {
                for message in inventory.details.messages.values() {
                    rows.push(Row {
                        key: row_key(pool, "message", message.id),
                        pool,
                        peer: None,
                        name: message.name.clone(),
                        status: message
                            .priority
                            .map(|p| p.to_string())
                            .unwrap_or_else(|| "Unknown".into()),
                        details: format!(
                            "{}\nMessage {}\nPool {}\nTime: {}\nObject: {} {}",
                            message.body,
                            message.id,
                            pool,
                            message.timestamp.as_deref().unwrap_or("Unknown"),
                            message.object_class,
                            message
                                .object_uuid
                                .map(|id| id.to_string())
                                .unwrap_or_else(|| "Unknown".into())
                        ),
                        target: Target::Resource(ResourceTarget::Message(message.id)),
                        terminal: false,
                        hidden: false,
                        export: Entry::message(pool, message),
                        help: None,
                    });
                }
            }
        }
        Tab::History => {
            for record in &m.operation_rows {
                let key = history_key(record.pool, record.id, Family::Vm);
                let is_hidden = hidden(m, key, record.state.terminal());
                let mut details = format!(
                    "{}\nVM {}\nPool {}\nOperation {}",
                    record.detail, record.vm, record.pool, record.id
                );
                for device in &record.created_devices {
                    details.push_str(&format!("\nCreated {}: {}", device.label(), device.uuid()));
                }
                if let Some(id) = record.created_vm {
                    details.push_str(&format!("\nCreated VM: {id}"));
                }
                if let Some(id) = record.created_snapshot {
                    details.push_str(&format!("\nCreated snapshot: {id}"));
                }
                append_steps(&mut details, &record.steps);
                rows.push(Row {
                    key: row_key(record.pool, "vm", record.id),
                    pool: record.pool,
                    peer: None,
                    name: record.action.label().into(),
                    status: format!("{:?}", record.state),
                    details,
                    target: Target::History(key),
                    terminal: record.state.terminal(),
                    hidden: is_hidden,
                    export: Entry::vm(record),
                    help: FailureHelp::vm(record),
                });
            }
            for record in &m.resource_rows {
                let key = history_key(record.pool, record.id, Family::Resource);

                let is_hidden = hidden(m, key, record.state.terminal());
                let mut details = format!(
                    "{}\nTarget {}\nPool {}\nOperation {}",
                    record.detail,
                    record.target.uuid(),
                    record.pool,
                    record.id
                );
                if let ResourceAction::CrossPoolVm(operation) = &record.action {
                    details.push_str(&format!(
                        "\nSource VM: {}\nDestination pool: {}",
                        operation.vm, operation.destination_pool
                    ));
                } else if let Some(destination) =
                    record.peer_pool().filter(|_| record.action.spans_pools())
                {
                    // A batch of cross-pool moves: the pool every one of its VMs goes to.
                    details.push_str(&format!("\nDestination pool: {destination}"));
                }
                if let Some(created) = record.created {
                    details.push_str(&format!("\nCreated resource: {}", created.uuid()));
                }
                append_steps(&mut details, &record.steps);
                rows.push(Row {
                    key: row_key(record.pool, "resource", record.id),
                    pool: record.pool,
                    peer: record.peer_pool(),
                    name: record.action.label().into(),
                    status: format!("{:?}", record.state),
                    details,
                    target: Target::History(key),
                    terminal: record.state.terminal(),
                    hidden: is_hidden,
                    export: Entry::resource(record),
                    help: FailureHelp::resource(record),
                });
            }
        }
    }
    let search = a.search.to_lowercase();
    let status = status_filter(m, tab).to_lowercase();
    rows.retain(|r| {
        a.pool.is_none_or(|p| p == r.pool || r.peer == Some(p))
            && (!r.hidden || a.show_hidden)
            && r.status.to_lowercase().contains(&status)
            && (format!("{} {} {}", r.name, r.status, r.details)
                .to_lowercase()
                .contains(&search))
    });
    rows.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then(a.key.cmp(&b.key))
    });
    let table = rows
        .iter()
        .map(|r| crate::surface::row(&r.key, [r.name.clone(), r.status.clone()]))
        .collect();
    Sheet {
        tab,
        search: a.search.clone(),
        status: status_filter(m, tab).to_owned(),
        pool: a.pool,
        show_hidden: a.show_hidden,
        hidden: if tab == Tab::History {
            m.preferences.hidden_history.clone()
        } else {
            Vec::new()
        },
        sources: sources(tab, pools),
        revisions: if tab == Tab::History {
            revisions(m).collect()
        } else {
            Vec::new()
        },
        rows,
        table: Rc::new(table),
    }
}
fn append_steps(text: &mut String, steps: &[OperationStep]) {
    for step in steps {
        text.push_str(&format!(
            "\n{}: {:?} — {}",
            step.label, step.state, step.detail
        ));
    }
}

/// The selected row of a list, when the list still has it.
pub fn selected(m: &Model, tab: Tab) -> Option<Row> {
    let key = m.areas.events.selected.as_ref()?;
    current(m, tab)
        .rows
        .iter()
        .find(|row| &row.key == key)
        .cloned()
}
fn observed(m: &Model, pool: PoolId, target: ResourceTarget) -> Option<ObservedResource> {
    if !m.can_mutate_pool(pool) {
        return None;
    }
    m.snapshot_for(pool)?.observe_resource(target).ok()
}
fn still_allowed(inventory: &Inventory, target: ResourceTarget) -> Option<bool> {
    match target {
        ResourceTarget::Task(id) => Some(inventory.details.tasks.get(&id).is_some_and(|task| {
            task.status == "pending"
                && task
                    .allowed_operations
                    .as_ref()
                    .is_some_and(|ops| ops.iter().any(|op| op == "cancel"))
        })),
        ResourceTarget::Message(_) => None,
        _ => Some(false),
    }
}
pub fn allowed(m: &Model, row: &Row) -> bool {
    let Target::Resource(target) = row.target else {
        return false;
    };
    if observed(m, row.pool, target).is_none() {
        return false;
    }
    let Some(inventory) = m.inventory_for(row.pool) else {
        return false;
    };
    match target {
        ResourceTarget::Message(id) => inventory.details.messages.contains_key(&id),
        other => still_allowed(&inventory, other).unwrap_or(false),
    }
}
/// Opens the review of the selected task or message: in a synthetic pool
/// always (it can never be confirmed there), otherwise when it is allowed.
pub fn begin_review(m: &mut Model, tab: Tab) {
    let Some(row) = selected(m, tab).filter(|row| synthetic(m, row.pool) || allowed(m, row)) else {
        return;
    };
    let Target::Resource(target) = row.target else {
        return;
    };
    let observed = observed(m, row.pool, target);
    let digest = match target {
        ResourceTarget::Message(id) => observed_pools(m)
            .get(&row.pool)
            .and_then(|i| i.details.messages.get(&id))
            .map(|m| m.review_digest()),
        _ => None,
    };
    m.areas.events.review = Some(Review {
        pool: row.pool,
        target,
        observed,
        digest,
        name: row.name,
        details: row.details,
        error: None,
    });
}
pub fn review_current(m: &Model, review: &Review) -> bool {
    if review.observed.is_none() || observed(m, review.pool, review.target) != review.observed {
        return false;
    }
    let Some(inventory) = m.inventory_for(review.pool) else {
        return false;
    };
    match review.target {
        ResourceTarget::Message(id) => inventory
            .details
            .messages
            .get(&id)
            .is_some_and(|m| Some(m.review_digest()) == review.digest),
        other => still_allowed(&inventory, other).unwrap_or(false),
    }
}
/// Why an open review cannot be confirmed although its pool is real: what
/// it was reviewed against changed.
pub fn review_stale(m: &Model, review: &Review) -> Option<&'static str> {
    (!synthetic(m, review.pool) && !review_current(m, review)).then_some(STALE_REVIEW)
}
/// The action a review submits, for the permission assessment.
pub fn review_action(review: &Review) -> Option<ResourceAction> {
    match review.target {
        ResourceTarget::Task(_) => Some(ResourceAction::CancelTask),
        ResourceTarget::Message(_) => Some(ResourceAction::DismissMessage {
            expected_digest: review.digest?,
        }),
        _ => None,
    }
}
pub fn submit(m: &mut Model) {
    let Some(review) = &m.areas.events.review else {
        return;
    };
    let refusal = if !review_current(m, review) {
        Some(STALE_SUBMIT.to_owned())
    } else {
        review_action(review).and_then(|action| {
            permissions::denial(&permissions::assess_for(
                m,
                review.pool,
                &action.permission_requirements(review.target),
            ))
        })
    };
    if let Some(refusal) = refusal {
        if let Some(review) = &mut m.areas.events.review {
            review.error = Some(refusal);
        }
        return;
    }
    let command = match review.target {
        ResourceTarget::Task(_) => ResourceCommand::CancelTask {
            task: review.observed.expect("reviewed observation current"),
        },
        ResourceTarget::Message(_) => ResourceCommand::DismissMessage {
            message: review.observed.expect("reviewed observation current"),
            expected_digest: review.digest.expect("message digest captured"),
        },
        _ => return,
    };
    let result = m
        .connection_for(review.pool)
        .ok_or(Error::Closed)
        .and_then(|c| c.submit_resource(OperationId::new(), command));
    match result {
        Ok(operation) => {
            m.track_resource(operation);
            m.areas.events.review = None;
            m.areas.events.list = Tab::History;
            m.areas.events.state_filter.clear();
            super::show_tab(m, TabId::Events);
            m.areas.events.notice = Some(SUBMITTED.into());
        }
        Err(error) => {
            if let Some(review) = &mut m.areas.events.review {
                review.error = Some(error.to_string());
            }
        }
    }
}
pub fn toggle_hidden(m: &mut Model) {
    let Some(row) = selected(m, Tab::History) else {
        return;
    };
    let Target::History(key) = row.target else {
        return;
    };
    if !row.terminal {
        return;
    }
    if row.hidden {
        m.preferences.hidden_history.retain(|old| old != &key);
    } else if m.preferences.hidden_history.len() < 5000 {
        m.preferences.hidden_history.push(key);
    } else {
        m.areas.events.notice = Some(HIDE_LIMIT.into());
        return;
    }
    m.save_preferences();
}
/// Shows the next pool: all, then each known pool in order, then all again.
/// (The Rune "Pool: … · change" button; the view offers the same pools in a
/// select.)
pub fn next_pool(m: &mut Model) {
    let pools = known_pools(m);
    m.areas.events.pool = match m.areas.events.pool {
        None => pools.first().copied(),
        Some(pool) => pools
            .iter()
            .position(|p| *p == pool)
            .and_then(|i| pools.get(i + 1).copied()),
    };
}
