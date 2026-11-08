use super::*;
use serde::{Deserialize, Serialize};
use xenadmin_core::{
    ObservedResource, OperationControl, OperationId, ResourceCommand, ResourceTarget,
};

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum Family {
    Vm,
    Resource,
}
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct HiddenKey {
    pub pool: PoolId,
    pub id: OperationId,
    pub family: Family,
}
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum Tab {
    #[default]
    Tasks,
    Messages,
    History,
}
impl Tab {
    fn title(self) -> &'static str {
        match self {
            Self::Tasks => "Tasks",
            Self::Messages => "Messages",
            Self::History => "History",
        }
    }
}
#[derive(Clone)]
pub(super) enum Target {
    Resource(ResourceTarget),
    History(HiddenKey),
}
#[derive(Clone)]
pub(super) struct Row {
    pub key: String,
    pub pool: PoolId,
    pub name: String,
    pub status: String,
    pub details: String,
    pub target: Target,
    pub terminal: bool,
    pub hidden: bool,
    pub export: activity_export::Entry,
}
pub(super) struct Review {
    pool: PoolId,
    target: ResourceTarget,
    observed: Option<ObservedResource>,
    digest: Option<[u8; 32]>,
    name: String,
    details: String,
}
pub(super) struct Activity {
    pub show: bool,
    tab: Tab,
    search: TextEdit,
    state_filter: TextEdit,
    pool: Option<PoolId>,
    pub rows: Vec<Row>,
    table: TableState,
    pools: BTreeMap<PoolId, Arc<Inventory>>,
    history_revisions: Vec<(
        PoolId,
        OperationId,
        Family,
        u64,
        xenadmin_core::OperationState,
        Option<String>,
    )>,
    pub review: Option<Review>,
    show_hidden: bool,
    pub notice: Option<String>,
    pub export: activity_export::Export,
}
impl Default for Activity {
    fn default() -> Self {
        Self {
            show: false,
            tab: Tab::Tasks,
            search: TextEdit::new(""),
            state_filter: TextEdit::new(""),
            pool: None,
            rows: Vec::new(),
            table: TableState::new().key("activity-table"),
            pools: BTreeMap::new(),
            history_revisions: Vec::new(),
            review: None,
            show_hidden: false,
            notice: None,
            export: Default::default(),
        }
    }
}
impl Activity {
    pub fn scope_label(&self) -> String {
        format!(
            "{}; pool {}; search {:?}; state/priority {:?}; hidden {}",
            self.tab.title(),
            self.pool
                .map(|id| id.to_string())
                .unwrap_or_else(|| "all observed pools".into()),
            self.search.text,
            self.state_filter.text,
            self.show_hidden
        )
    }
}
pub(super) fn open(s: &mut State) {
    s.activity.show = true;
    s.activity.review = None;
    s.activity.pool = s.inventory.as_ref().map(|i| i.pool_id);
    poll(s);
    rebuild(s);
}
pub(super) fn poll(s: &mut State) -> bool {
    let mut changed = activity_export::poll(s);
    if let Some(inventory) = &s.inventory {
        let old = s.activity.pools.get(&inventory.pool_id);
        if !old.is_some_and(|old| Arc::ptr_eq(old, inventory)) {
            s.activity
                .pools
                .insert(inventory.pool_id, inventory.clone());
            changed = true;
        }
    }
    let revisions = s
        .operation_rows
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
        .chain(s.resource_rows.iter().map(|r| {
            (
                r.pool,
                r.id,
                Family::Resource,
                r.revision(),
                r.state.clone(),
                r.local_error.clone(),
            )
        }))
        .collect::<Vec<_>>();
    if revisions != s.activity.history_revisions {
        s.activity.history_revisions = revisions;
        changed = true;
    }
    if s.activity.show && changed {
        rebuild(s);
    }
    changed
}
fn row_key(pool: PoolId, kind: &str, id: impl std::fmt::Display) -> String {
    format!("{pool}/{kind}/{id}")
}
fn history_key(pool: PoolId, id: OperationId, family: Family) -> HiddenKey {
    HiddenKey { pool, id, family }
}
fn hidden(s: &State, key: HiddenKey, terminal: bool) -> bool {
    terminal && s.preferences.hidden_history.contains(&key)
}
fn rebuild(s: &mut State) {
    let mut rows = Vec::new();
    match s.activity.tab {
        Tab::Tasks => {
            for (&pool, inventory) in &s.activity.pools {
                for task in inventory.details.tasks.values() {
                    rows.push(Row {
                        key: row_key(pool, "task", task.id),
                        pool,
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
                            task.resident_host.map(|id| inventory.hosts.get(&id).map(|h| format!("{} ({id})",h.name)).unwrap_or_else(||id.to_string())).unwrap_or_else(||"Not reported".into()),
                            task.parent.map(|id|id.to_string()).unwrap_or_else(||"None reported".into()),
                            if task.subtasks.is_empty() {"None reported".into()} else {task.subtasks.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")},
                            task.result_type.as_deref().unwrap_or("Not reported"),
                            task.error_code.as_deref().unwrap_or("")
                        ),
                        target: Target::Resource(ResourceTarget::Task(task.id)),
                        terminal: matches!(
                            task.status.as_str(),
                            "success" | "failure" | "cancelled"
                        ),
                        hidden: false,
                        export: activity_export::Entry::task(pool, task),
                    });
                }
            }
        }
        Tab::Messages => {
            for (&pool, inventory) in &s.activity.pools {
                for message in inventory.details.messages.values() {
                    rows.push(Row {
                        key: row_key(pool, "message", message.id),
                        pool,
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
                        export: activity_export::Entry::message(pool, message),
                    });
                }
            }
        }
        Tab::History => {
            for record in &s.operation_rows {
                let key = history_key(record.pool, record.id, Family::Vm);
                let is_hidden = hidden(s, key, record.state.terminal());
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
                    name: record.action.label().into(),
                    status: format!("{:?}", record.state),
                    details,
                    target: Target::History(key),
                    terminal: record.state.terminal(),
                    hidden: is_hidden,
                    export: activity_export::Entry::vm(record),
                });
            }
            for record in &s.resource_rows {
                let key = history_key(record.pool, record.id, Family::Resource);
                let is_hidden = hidden(s, key, record.state.terminal());
                let mut details = format!(
                    "{}\nTarget {}\nPool {}\nOperation {}",
                    record.detail,
                    record.target.uuid(),
                    record.pool,
                    record.id
                );
                if let Some(created) = record.created {
                    details.push_str(&format!("\nCreated resource: {}", created.uuid()));
                }
                append_steps(&mut details, &record.steps);
                rows.push(Row {
                    key: row_key(record.pool, "resource", record.id),
                    pool: record.pool,
                    name: record.action.label().into(),
                    status: format!("{:?}", record.state),
                    details,
                    target: Target::History(key),
                    terminal: record.state.terminal(),
                    hidden: is_hidden,
                    export: activity_export::Entry::resource(record),
                });
            }
        }
    }
    let search = s.activity.search.text.to_lowercase();
    let status = s.activity.state_filter.text.to_lowercase();
    rows.retain(|r| {
        s.activity.pool.is_none_or(|p| p == r.pool)
            && (!r.hidden || s.activity.show_hidden)
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
    s.activity.rows = rows;
}
fn append_steps(text: &mut String, steps: &[xenadmin_core::OperationStep]) {
    for step in steps {
        text.push_str(&format!(
            "\n{}: {:?} — {}",
            step.label, step.state, step.detail
        ));
    }
}
fn selected(s: &State) -> Option<&Row> {
    s.activity
        .table
        .selected()
        .next()
        .and_then(|key| s.activity.rows.iter().find(|row| row.key == key))
}
fn observed(s: &State, pool: PoolId, target: ResourceTarget) -> Option<ObservedResource> {
    if !s.can_mutate() || s.inventory.as_ref()?.pool_id != pool {
        return None;
    }
    s.snapshot.as_ref()?.observe_resource(target).ok()
}
fn allowed(s: &State, row: &Row) -> bool {
    let Target::Resource(target) = row.target else {
        return false;
    };
    if observed(s, row.pool, target).is_none() {
        return false;
    }
    let Some(inventory) = &s.inventory else {
        return false;
    };
    match target {
        ResourceTarget::Task(id) => inventory.details.tasks.get(&id).is_some_and(|task| {
            task.status == "pending"
                && task
                    .allowed_operations
                    .as_ref()
                    .is_some_and(|ops| ops.iter().any(|op| op == "cancel"))
        }),
        ResourceTarget::Message(id) => inventory.details.messages.contains_key(&id),
        _ => false,
    }
}
fn begin_review(s: &mut State) {
    let Some(row) = selected(s).cloned().filter(|row| s.demo || allowed(s, row)) else {
        return;
    };
    let Target::Resource(target) = row.target else {
        return;
    };
    let observed = observed(s, row.pool, target);
    let digest = match target {
        ResourceTarget::Message(id) => s
            .activity
            .pools
            .get(&row.pool)
            .and_then(|i| i.details.messages.get(&id))
            .map(|m| m.review_digest()),
        _ => None,
    };
    s.activity.review = Some(Review {
        pool: row.pool,
        target,
        observed,
        digest,
        name: row.name,
        details: row.details,
    });
}
fn review_current(s: &State, review: &Review) -> bool {
    if review.observed.is_none() || observed(s, review.pool, review.target) != review.observed {
        return false;
    }
    let Some(inventory) = &s.inventory else {
        return false;
    };
    match review.target {
        ResourceTarget::Task(id) => inventory.details.tasks.get(&id).is_some_and(|task| {
            task.status == "pending"
                && task
                    .allowed_operations
                    .as_ref()
                    .is_some_and(|ops| ops.iter().any(|op| op == "cancel"))
        }),
        ResourceTarget::Message(id) => inventory
            .details
            .messages
            .get(&id)
            .is_some_and(|m| Some(m.review_digest()) == review.digest),
        _ => false,
    }
}
fn submit(s: &mut State) {
    let Some(review) = &s.activity.review else {
        return;
    };
    if !review_current(s, review) {
        s.activity.notice =
            Some("The connection or reviewed object changed. Reopen the review.".into());
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
    let result = s
        .connection
        .as_ref()
        .ok_or(xenadmin_core::Error::Closed)
        .and_then(|c| c.submit_resource(OperationId::new(), command));
    match result {
        Ok(operation) => {
            if let Some(wake) = &s.waker {
                operation.notify_on_change(wake.clone());
            }
            s.resource_operations.push(operation);
            s.activity.review = None;
            s.activity.tab = Tab::History;
            s.activity.notice = Some("Change submitted. History tracks the host outcome.".into());
            rebuild(s);
        }
        Err(error) => s.activity.notice = Some(error.to_string()),
    }
}
fn toggle_hidden(s: &mut State) {
    let Some(row) = selected(s).cloned() else {
        return;
    };
    let Target::History(key) = row.target else {
        return;
    };
    if !row.terminal {
        return;
    }
    if row.hidden {
        s.preferences.hidden_history.retain(|old| old != &key);
    } else if s.preferences.hidden_history.len() < 5000 {
        s.preferences.hidden_history.push(key);
    } else {
        s.activity.notice = Some("Restore hidden history entries before hiding more.".into());
        return;
    }
    s.save_preferences();
    rebuild(s);
}
fn recheck(s: &mut State) {
    let Some(row) = selected(s).cloned() else {
        return;
    };
    let Target::History(key) = row.target else {
        return;
    };
    if !s.can_mutate() || s.inventory.as_ref().is_none_or(|i| i.pool_id != key.pool) || row.terminal
    {
        return;
    }
    let result = s
        .connection
        .as_ref()
        .ok_or(xenadmin_core::Error::Closed)
        .and_then(|c| match key.family {
            Family::Vm => c.control_operation(key.id, OperationControl::Recheck),
            Family::Resource => c.control_resource(key.id, OperationControl::Recheck),
        });
    s.activity.notice = Some(
        result
            .map(|()| "Checking existing evidence; no mutation is replayed.".into())
            .unwrap_or_else(|e| e.to_string()),
    );
}
fn next_pool(s: &mut State) {
    let mut pools = s
        .activity
        .pools
        .keys()
        .copied()
        .chain(s.operation_rows.iter().map(|r| r.pool))
        .chain(s.resource_rows.iter().map(|r| r.pool))
        .collect::<Vec<_>>();
    pools.sort();
    pools.dedup();
    s.activity.pool = match s.activity.pool {
        None => pools.first().copied(),
        Some(pool) => pools
            .iter()
            .position(|p| *p == pool)
            .and_then(|i| pools.get(i + 1).copied()),
    };
    rebuild(s);
}
pub(super) fn dialog(s: &State, k: &Kit) -> El {
    let a = &s.activity;
    let mut body = col().w_full().gap(10.0).child(heading("Activity", k));
    if let Some(review) = &a.review {
        let cancel = matches!(review.target, ResourceTarget::Task(_));
        body=body.child(heading(if cancel{"Request task cancellation?"}else{"Dismiss this server message?"},k)).child(caption(&review.name,k).wrap()).child(caption(&review.details,k).wrap_anywhere()).child(caption(if cancel{"The host may finish normally, fail, or continue working. This requests cancellation and tracks fresh status; it does not release the original operation's reservation."}else{"This removes the reviewed message from the host for all clients. It does not resolve the underlying condition or change alarm configuration."},k).wrap());
        if !review_current(s, review) {
            body = body.child(
                caption(
                    "Connection or object changed. Reopen the review before proceeding.",
                    k,
                )
                .wrap(),
            );
        }
        body = body.child(
            row()
                .flex_wrap()
                .gap(8.0)
                .child(
                    Button::new("Back")
                        .on_press(|s: &mut State| s.activity.review = None)
                        .build(k),
                )
                .child(
                    Button::new(if cancel {
                        "Request cancellation"
                    } else {
                        "Dismiss message"
                    })
                    .variant(ButtonVariant::Danger)
                    .disabled(!review_current(s, review))
                    .on_press(submit)
                    .build(k)
                    .key("activity-confirm"),
                ),
        );
    } else {
        let mut tabs = row()
            .flex_wrap()
            .gap(4.0)
            .role("tablist")
            .label("Activity views");
        for tab in [Tab::Tasks, Tab::Messages, Tab::History] {
            tabs = tabs.child(
                Button::new(tab.title())
                    .variant(if tab == a.tab {
                        ButtonVariant::Neutral
                    } else {
                        ButtonVariant::Ghost
                    })
                    .on_press(move |s: &mut State| {
                        s.activity.tab = tab;
                        s.activity.state_filter = TextEdit::new("");
                        rebuild(s);
                    })
                    .build(k)
                    .key(format!("activity-{}", tab.title())),
            );
        }
        body = body.child(tabs).child(
            row()
                .flex_wrap()
                .gap(8.0)
                .child(
                    TextInput::new(&a.search)
                        .label("Search activity")
                        .placeholder("Search name, text or UUID")
                        .on_edit(|s: &mut State| &mut s.activity.search)
                        .on_change(|s: &mut State, _| rebuild(s))
                        .clear_button()
                        .build(k)
                        .w(260.0)
                        .max_w((s.window_width - 80.0).max(200.0))
                        .key("activity-search"),
                )
                .child(
                    TextInput::new(&a.state_filter)
                        .label(if a.tab == Tab::Messages {
                            "Priority filter"
                        } else {
                            "State filter"
                        })
                        .placeholder(if a.tab == Tab::Messages {
                            "Priority"
                        } else {
                            "State"
                        })
                        .on_edit(|s: &mut State| &mut s.activity.state_filter)
                        .on_change(|s: &mut State, _| rebuild(s))
                        .clear_button()
                        .build(k)
                        .w(150.0)
                        .key("activity-state-filter"),
                ),
        );
        let pool = a
            .pool
            .map(|p| {
                a.pools
                    .get(&p)
                    .map(|i| i.pool_name.clone())
                    .unwrap_or_else(|| p.to_string())
            })
            .unwrap_or_else(|| "All observed pools".into());
        body = body.child(
            Button::new(format!("Pool: {pool} · change"))
                .on_press(next_pool)
                .build(k)
                .key("activity-pool"),
        );
        if a.tab != Tab::History {
            body=body.child(caption("Lists use the latest observed inventory for each pool. Offline or switched-pool records remain visible; actions require a current writable connection to that pool.",k).wrap());
            if let Some(pool) = a.pool.and_then(|pool| a.pools.get(&pool)) {
                let class = if a.tab == Tab::Tasks {
                    "task"
                } else {
                    "message"
                };
                if let Some(status) = pool.details.classes.get(class) {
                    body = body.child(caption(format!("Inventory: {status:?}"), k).wrap());
                }
            }
        } else {
            body = body.child(
                Checkbox::new(a.show_hidden)
                    .label("Include locally hidden completed entries")
                    .on_toggle(|s: &mut State| {
                        s.activity.show_hidden = !s.activity.show_hidden;
                        rebuild(s);
                    })
                    .build(k)
                    .key("activity-show-hidden"),
            );
        }
        let table = Table::new(
            |s: &State| &s.activity.table,
            |s: &mut State| &mut s.activity.table,
            |s: &State| s.activity.rows.len(),
            |s: &State, i| Cow::Borrowed(&s.activity.rows[i].key),
        )
        .name("Activity entries")
        .column(
            TableColumn::new("name", "Name / action")
                .flex(2.0)
                .min(160.0),
        )
        .column(
            TableColumn::new(
                "status",
                if a.tab == Tab::Messages {
                    "Priority"
                } else {
                    "State"
                },
            )
            .flex(1.0)
            .min(100.0),
        )
        .cell(|s: &State, i, column| {
            let row = &s.activity.rows[i];
            if column == "name" {
                Cell::text(&row.name)
            } else {
                Cell::text(&row.status)
            }
        })
        .zebra(true)
        .build(s, k)
        .h(220.0)
        .min_h(120.0);
        body = body
            .child(caption(format!("{} matching entries", a.rows.len()), k))
            .child(table);
        if let Some(selected) = selected(s) {
            body = body
                .child(heading(&selected.name, k))
                .child(
                    caption(
                        if s.demo {
                            "Synthetic fixture · host actions unavailable"
                        } else if s.snapshot.as_ref().is_some_and(|snapshot| {
                            snapshot.is_current()
                                && snapshot
                                    .inventory()
                                    .is_some_and(|i| i.pool_id == selected.pool)
                        }) {
                            "Current connected pool"
                        } else {
                            "Cached inventory · this pool is not currently selected and connected"
                        },
                        k,
                    )
                    .wrap(),
                )
                .child(caption(&selected.details, k).wrap_anywhere());
            match selected.target {
                Target::Resource(target) => {
                    let task = matches!(target, ResourceTarget::Task(_));
                    body = body.child(
                        Button::new(if task {
                            "Review cancellation…"
                        } else {
                            "Review dismissal…"
                        })
                        .disabled(!s.demo && !allowed(s, selected))
                        .on_press(begin_review)
                        .build(k)
                        .key("activity-review"),
                    );
                    if !allowed(s, selected) {
                        body=body.child(caption(if task{"Cancellation requires a current writable connection, a pending task, and the host's advertised cancel capability."}else{"Message dismissal requires a current writable connection to its pool."},k).wrap());
                    }
                }
                Target::History(_) => {
                    body = body.child(
                        row()
                            .flex_wrap()
                            .gap(8.0)
                            .child(
                                Button::new(if selected.hidden {
                                    "Restore entry"
                                } else {
                                    "Hide completed entry"
                                })
                                .disabled(!selected.terminal)
                                .on_press(toggle_hidden)
                                .build(k)
                                .key("activity-hide"),
                            )
                            .child(
                                Button::new("Recheck outcome")
                                    .disabled(
                                        selected.terminal
                                            || !s.can_mutate()
                                            || s.inventory
                                                .as_ref()
                                                .is_none_or(|i| i.pool_id != selected.pool),
                                    )
                                    .on_press(recheck)
                                    .build(k)
                                    .key("activity-recheck"),
                            ),
                    );
                    if !selected.terminal {
                        body=body.child(caption("Unresolved operations remain visible and cannot be hidden. Rechecking never repeats a mutation.",k).wrap());
                    }
                }
            }
        }
        body=body.child(Button::new("Export filtered entries…").disabled(a.export.pending).on_press_tx(activity_export::choose).build(k).key("activity-export")).child(caption("JSON exports contain the displayed task/message fields and public operation details, including message text. Files are saved locally with owner-only permissions.",k).wrap());
    }
    if let Some(notice) = &a.notice {
        body = body.child(caption(notice, k).wrap_anywhere().role("status"));
    }
    let width = (s.window_width - 32.0).clamp(280.0, 980.0);
    let height = (s.window_height - 56.0).clamp(280.0, 900.0);
    el().w_full()
        .h_full()
        .absolute(0.0, 0.0)
        .center()
        .fill(rune::Color::BLACK.alpha(0.5))
        .role("dialog")
        .label("Activity")
        .modal(false, |s: &mut State| s.activity.show = false)
        .child(
            col()
                .w(width)
                .h(height)
                .padding(18.0)
                .gap(12.0)
                .radius(10.0)
                .fill(k.c.bg_base)
                .child(
                    body.grow(1.0)
                        .min_h(0.0)
                        .scroll(rune::ScrollSpec {
                            stick_bottom: false,
                        })
                        .key("activity-scroll"),
                )
                .child(
                    Button::new("Close")
                        .on_press(|s: &mut State| s.activity.show = false)
                        .build(k)
                        .key("activity-close"),
                ),
        )
}

pub(super) fn scene(s: &mut State) {
    use xenadmin_core::{MessageId, ServerMessage, ServerTask, TaskId};
    demo(s);
    let inventory = Arc::make_mut(s.inventory.as_mut().expect("synthetic inventory"));
    for index in 1..=2 {
        let id = TaskId::from(uuid::Uuid::from_u128(900 + index));
        inventory.details.tasks.insert(
            id,
            ServerTask {
                id,
                name: format!("Synthetic task {index}"),
                description: "Synthetic storage operation".into(),
                status: if index == 1 { "pending" } else { "success" }.into(),
                progress: Some(if index == 1 { 0.35 } else { 1.0 }),
                created: Some("20261108T10:00:00Z".into()),
                finished: None,
                error_code: None,
                allowed_operations: Some(vec!["cancel".into()]),
                resident_host: None,
                parent: None,
                subtasks: Vec::new(),
                result_type: None,
            },
        );
        let id = MessageId::from(uuid::Uuid::from_u128(1000 + index));
        inventory.details.messages.insert(
            id,
            ServerMessage {
                id,
                name: format!("Synthetic message {index}"),
                priority: Some(index as i64),
                object_class: "VM".into(),
                object_uuid: None,
                timestamp: Some("20261108T10:00:00Z".into()),
                body: format!("Synthetic alert text {index}. No host data is used."),
            },
        );
    }
    open(s);
    if let Some(row) = s.activity.rows.first() {
        s.activity.table.select(row.key.clone());
    }
}
#[cfg(test)]
#[path = "activity_tests.rs"]
mod tests;
