//! The tests of the area, in three parts.
//!
//! 1. **The Rune tests**: the nine of `activity_tests.rs`, the Runtime test
//!    of `failure_help.rs` (its pure one moved with the module), the two of
//!    `resource_admin.rs` that are about the history, the export test of
//!    `network_topology_ui.rs`, and this area's share of
//!    `dialogs_outside_the_supported_core_carry_the_experimental_notice`.
//!    Each keeps its Rune name where it asserts the same thing. Where the
//!    Rune test wrote a field or called a private function, the test goes
//!    the way a user goes (a command, a key typed, a row selected).
//! 2. **What the port changed**: two tabs and a list choice, commands that
//!    follow the selected row, one acknowledgement for both families, the
//!    functions other areas call.
//! 3. **The texts** that named Rune places and name this client's now
//!    (AREAS.md decision 10).
//!
//! No test can have a current snapshot, so none submits successfully: the
//! tests are of what is captured, shown and refused.

use std::rc::Rc;
use std::sync::Arc;

use serde_json::json;
use xenadmin_core::{
    Inventory, OperationId, OperationRecord, OperationState, PoolId, ResourceOperationRecord,
    ResourceTarget,
};

use super::activity::{self, DEMO, Tab};
use super::commands::{self, EXPORTING};
use super::control::{self, NOT_WRITABLE};
use super::export::{self, Entry};
use super::hidden::{Family, HiddenKey};
use super::{failure_help, view};
use crate::Harness;
use crate::prelude::*;
use crate::surface::EXPERIMENTAL_NOTICE_KEY;

// -- Fixtures -------------------------------------------------------------------

const TABLE: &str = "activity-table";

/// The scene: the Events tab on Tasks, the first task selected.
fn events() -> Harness {
    Harness::scene("activity")
}

fn pool(h: &Harness) -> PoolId {
    h.model.inventory.as_ref().expect("inventory").pool_id
}

fn vm_json(pool: PoolId, state: OperationState) -> serde_json::Value {
    json!({"id":OperationId::new(),"pool":pool,"vm":uuid::Uuid::from_u128(30),"action":"Start","state":state,"progress":null,"detail":"Synthetic operation result","result_reference":"OpaqueRef:private-result","task":"OpaqueRef:private-task"})
}
fn vm_record(pool: PoolId, state: OperationState) -> OperationRecord {
    serde_json::from_value(vm_json(pool, state)).unwrap()
}
fn resource_json(pool: PoolId) -> serde_json::Value {
    json!({"id":OperationId::new(),"pool":pool,"target":{"Storage":uuid::Uuid::from_u128(40)},"action":"ScanStorage","state":"Succeeded","progress":1.0,"detail":"Synthetic scan completed","created_at_ms":123,"can_cancel":false,"cancel_requested":false,"steps":[],"created":null,"baseline_networks":[uuid::Uuid::from_u128(41)]})
}
fn resource_record(pool: PoolId) -> ResourceOperationRecord {
    serde_json::from_value(resource_json(pool)).unwrap()
}
/// An interrupted pool join: the record of the source pool, which applies
/// to the destination pool too.
fn join_record(source: PoolId, destination: PoolId) -> ResourceOperationRecord {
    serde_json::from_value(json!({
        "id":OperationId::new(),"pool":source,"target":{"Pool":source},"action":{"Pool":{"Join":{
            "source_host":uuid::Uuid::new_v4(),"destination_pool":destination,
            "destination_coordinator":uuid::Uuid::new_v4(),"destination_address":"192.0.2.20"
        }}},"state":"OutcomeUnknown","progress":null,"detail":"Fixture interrupted join",
        "created_at_ms":0,"can_cancel":false,"cancel_requested":false,"steps":[],"created":null
    }))
    .unwrap()
}

fn vm_key(record: &OperationRecord) -> HiddenKey {
    activity::history_key(record.pool, record.id, Family::Vm)
}
fn resource_key(record: &ResourceOperationRecord) -> HiddenKey {
    activity::history_key(record.pool, record.id, Family::Resource)
}

/// The inventory moves on: what a poll does when the host reports a change.
fn amend(h: &mut Harness, change: impl FnOnce(&mut Inventory)) {
    let mut inventory = h
        .model
        .inventory
        .take()
        .expect("inventory")
        .as_ref()
        .clone();
    change(&mut inventory);
    h.model.inventory = Some(Arc::new(inventory));
}

fn rows(h: &Harness, tab: Tab) -> Vec<activity::Row> {
    activity::current(&h.model, tab).rows.clone()
}

fn local(h: &Harness) -> Vec<&'static str> {
    h.local_row().iter().map(|item| item.id.0).collect()
}

#[track_caller]
fn review(h: &Harness) -> &activity::Review {
    h.model.areas.events.review.as_ref().expect("a review")
}

/// A directory of its own under the temporary directory, removed on drop.
struct Scratch(std::path::PathBuf);
impl Scratch {
    fn new() -> Self {
        let dir =
            std::env::temp_dir().join(format!("xenadmin-activity-ui-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        Self(dir)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// -- 1. The Rune tests ----------------------------------------------------------

/// `activity_tests.rs`. The model half: the capture and the refusals. (The
/// GPUI half, open, close and the disabled button on screen at both sizes,
/// is in `gpui-tests-wanted.md`.)
#[test]
fn activity_review_captures_exact_target_and_never_dispatches_from_synthetic_or_stale_views() {
    let mut h = events();
    h.click("events.cancel-task");
    assert_eq!(h.surface_id().as_deref(), Some("task-cancellation"));
    let target = review(&h).target;
    assert!(matches!(target, ResourceTarget::Task(_)));
    // The synthetic pool: the review is open and can never be confirmed.
    assert_eq!(h.try_click("activity-confirm"), Err(DEMO.into()));
    assert!(h.find("activity-review-preview").is_some());
    h.model.areas.events.selected = Some("different-selection".into());
    amend(&mut h, |i| i.pool_id = PoolId::from(uuid::Uuid::new_v4()));
    // The pool it was reviewed in is gone: stale, with the Rune words.
    assert_eq!(
        h.try_click("activity-confirm"),
        Err(activity::STALE_REVIEW.into())
    );
    assert_eq!(
        h.node("activity-change-review-stale").text.as_deref(),
        Some(activity::STALE_REVIEW)
    );
    activity::submit(&mut h.model);
    assert_eq!(review(&h).target, target);
    assert_eq!(review(&h).error.as_deref(), Some(activity::STALE_SUBMIT));
    assert!(h.model.resource_operations.is_empty());
    assert!(!activity::review_current(&h.model, review(&h)));
    h.click("activity-review-close");
    assert!(h.model.areas.events.review.is_none());
    // The Rune Activity button: any area gets back here with `show`.
    h.open_view(NavView::Infrastructure);
    super::show(&mut h.model);
    assert_eq!(h.model.nav.view, NavView::Notifications);
    assert_eq!(h.model.active_tab(), TabId::Events);
}

/// `activity_tests.rs`, as it was.
#[test]
fn message_review_keeps_original_content_digest_when_selection_or_inventory_changes() {
    let mut h = events();
    h.open_tab(TabId::Alerts);
    let messages = rows(&h, Tab::Messages);
    h.select_row(TABLE, &messages[0].key);
    h.click("alerts.dismiss");
    assert_eq!(h.surface_id().as_deref(), Some("message-dismissal"));
    let target = review(&h).target;
    let digest = review(&h).digest;
    h.model.areas.events.selected = Some(messages[1].key.clone());
    let ResourceTarget::Message(id) = target else {
        panic!()
    };
    amend(&mut h, |i| {
        i.details.messages.get_mut(&id).unwrap().body = "Changed text".into()
    });
    super::poll(&mut h.model, 0.0);
    assert_eq!(review(&h).target, target);
    assert_eq!(review(&h).digest, digest);
    assert_ne!(
        h.model.inventory.as_ref().unwrap().details.messages[&id].review_digest(),
        digest.unwrap()
    );
    activity::submit(&mut h.model);
    assert!(h.model.resource_operations.is_empty());
    // The review still shows what was reviewed, not the changed text.
    assert!(h.has_text("Synthetic alert text 1"));
    assert!(!h.has_text("Changed text"));
}

/// `activity_tests.rs`: `clear_buttons_empty_the_activity_filters_and_restore_the_rows`.
/// The clear button is the input's own in the new front end (its test is
/// in `gpui-tests-wanted.md`); what it does is type the empty text.
#[test]
fn emptying_a_filter_leaves_the_other_and_restores_the_rows() {
    let mut h = events();
    assert_eq!(rows(&h, Tab::Tasks).len(), 2);
    h.type_text("activity-search", "Synthetic task 1");
    h.type_text("activity-state-filter", "pending");
    assert_eq!(rows(&h, Tab::Tasks).len(), 1);
    h.type_text("activity-search", "");
    assert!(h.model.areas.events.search.is_empty());
    assert_eq!(h.model.areas.events.state_filter, "pending");
    h.type_text("activity-state-filter", "");
    assert!(h.model.areas.events.state_filter.is_empty());
    assert_eq!(rows(&h, Tab::Tasks).len(), 2);
    assert!(h.has_text("2 matching entries"));
}

/// `activity_tests.rs`. The pool that stays listed after its session is
/// gone is shown first with the session's inventory taken away, as in the
/// Rune test, and then beside a second pool.
#[test]
fn filters_cover_message_body_priority_task_state_and_cached_pools() {
    let mut h = events();
    super::poll(&mut h.model, 0.0);
    assert_eq!(rows(&h, Tab::Tasks).len(), 2);
    h.type_text("activity-state-filter", "pending");
    assert_eq!(rows(&h, Tab::Tasks).len(), 1);
    h.open_tab(TabId::Alerts);
    assert_eq!(
        h.node("activity-state-filter").label.as_deref(),
        Some("Priority filter")
    );
    h.type_text("activity-state-filter", "2");
    h.type_text("activity-search", "alert text 2");
    let found = rows(&h, Tab::Messages);
    assert_eq!(found.len(), 1);
    let original_pool = found[0].pool;
    // The session's inventory is gone: its rows stay, and nothing can be
    // done with them.
    let original = h.model.inventory.take().unwrap();
    h.model.snapshot = None;
    super::poll(&mut h.model, 0.0);
    let found = rows(&h, Tab::Messages);
    assert_eq!(found[0].pool, original_pool);
    assert!(!activity::allowed(&h.model, &found[0]));
    h.select_row(TABLE, &found[0].key);
    assert_eq!(
        h.eligibility("alerts.dismiss"),
        Eligibility::disabled(activity::DISMISS_REQUIRES)
    );
    assert!(h.has_text(view::SOURCE_CACHED));
    let mut second = original.as_ref().clone();
    second.pool_id = PoolId::from(uuid::Uuid::new_v4());
    second.pool_name = "Second synthetic pool".into();
    h.model.inventory = Some(Arc::new(second));
    super::poll(&mut h.model, 0.0);
    h.pick("activity-pool", "all");
    assert_eq!(h.model.areas.events.pool, None);
    assert_eq!(rows(&h, Tab::Messages).len(), 2);
    activity::next_pool(&mut h.model);
    assert_eq!(rows(&h, Tab::Messages).len(), 1);
    // The select names the same pools as the Rune button cycled through.
    let known = activity::known_pools(&h.model);
    assert_eq!(known.len(), 2);
    assert_eq!(h.model.areas.events.pool, Some(known[0]));
    h.pick("activity-pool", &known[1].to_string());
    assert_eq!(h.model.areas.events.pool, Some(known[1]));
    assert_eq!(rows(&h, Tab::Messages).len(), 1);
}

/// `activity_tests.rs`.
#[test]
fn history_hiding_is_terminal_only_pool_scoped_restorable_and_survives_disconnect() {
    let mut h = events();
    let pool = pool(&h);
    let done = vm_record(pool, OperationState::Succeeded);
    let unknown = vm_record(pool, OperationState::OutcomeUnknown);
    let done_key = activity::key_of(vm_key(&done));
    let unknown_key = activity::key_of(vm_key(&unknown));
    let unknown_id = unknown.id;
    h.model.operation_rows = vec![done, unknown];
    h.model.resource_rows = vec![resource_record(pool)];
    h.click("activity-History");
    assert_eq!(rows(&h, Tab::History).len(), 3);
    h.select_row(TABLE, &done_key);
    h.click("events.hide");
    assert_eq!(rows(&h, Tab::History).len(), 2);
    assert_eq!(h.model.operation_rows.len(), 2);
    let serialized = serde_json::to_vec(&h.model.preferences).unwrap();
    h.model.preferences = serde_json::from_slice(&serialized).unwrap();
    h.model.inventory = None;
    h.model.snapshot = None;
    super::poll(&mut h.model, 0.0);
    assert_eq!(rows(&h, Tab::History).len(), 2);
    h.select_row(TABLE, &unknown_key);
    assert_eq!(h.try_click("events.hide"), Err(activity::UNRESOLVED.into()));
    assert!(h.find("activity-unresolved").is_some());
    activity::toggle_hidden(&mut h.model);
    assert_eq!(h.model.preferences.hidden_history.len(), 1);
    // A stale preference can never hide an unresolved result.
    h.model
        .preferences
        .hidden_history
        .push(activity::history_key(pool, unknown_id, Family::Vm));
    assert!(rows(&h, Tab::History).iter().any(|r| r.key == unknown_key));
    h.toggle("activity-show-hidden");
    h.select_row(TABLE, &done_key);
    let hide = h
        .local_row()
        .into_iter()
        .find(|item| item.id == commands::HIDE)
        .expect("the hide command");
    assert_eq!(hide.label, "Restore entry");
    h.click("events.hide");
    h.toggle("activity-show-hidden");
    assert_eq!(rows(&h, Tab::History).len(), 3);
}

/// `activity_tests.rs`, as it was.
#[test]
fn public_export_projection_excludes_task_refs_private_plans_and_resource_baselines() {
    let pool = PoolId::from(uuid::Uuid::new_v4());
    let vm = vm_record(pool, OperationState::OutcomeUnknown);
    let resource = resource_record(pool);
    let public_vm = serde_json::to_string(&vm).unwrap();
    assert!(!public_vm.contains("OpaqueRef:"));
    assert!(!public_vm.contains("result_reference"));
    assert!(
        serde_json::to_string(&resource)
            .unwrap()
            .contains("baseline_networks")
    );
    let bytes = serde_json::to_string(&vec![Entry::vm(&vm), Entry::resource(&resource)]).unwrap();
    for private in [
        "OpaqueRef:",
        "result_reference",
        "task",
        "lifecycle_plan",
        "device_plan",
        "baseline_networks",
        "expected_digest",
    ] {
        assert!(!bytes.contains(private), "leaked {private}");
    }
    assert!(bytes.contains(&vm.id.to_string()));
    assert!(bytes.contains("OutcomeUnknown"));
}

/// `activity_tests.rs`. The model half: the prompt that is asked for, the
/// capture, the writer and its result. (That the front end opens a save
/// prompt for the intent is in `gpui-tests-wanted.md`.)
#[test]
fn save_dialog_exports_captured_selection_even_after_switch_and_cancel_writes_nothing() {
    let dir = Scratch::new();
    let mut h = events();
    h.click("events.export");
    assert!(h.model.areas.events.export.pending);
    assert_eq!(
        h.eligibility("events.export"),
        Eligibility::disabled(EXPORTING)
    );
    let prompt = h
        .model
        .intents
        .iter()
        .find_map(|intent| match intent {
            Intent::ChooseFile(request) => Some(request.clone()),
            _ => None,
        })
        .expect("a file prompt");
    assert_eq!(prompt.title, "Export activity JSON");
    assert_eq!(prompt.mode, FileMode::Save);
    assert_eq!(
        prompt.suggested_name.as_deref(),
        Some("xenadmin-activity.json")
    );
    h.answer_file(None);
    assert!(!h.model.areas.events.export.pending);
    assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 0);
    assert_eq!(h.model.areas.events.notice, None, "a cancel says nothing");
    // A prompt that could not be shown ends the export too, and says why
    // in the Rune words.
    h.click("events.export");
    assert!(h.model.areas.events.export.pending);
    h.fail_file("org.freedesktop.portal.Desktop is not running");
    assert!(!h.model.areas.events.export.pending);
    assert_eq!(
        h.node("activity-notice").text.as_deref(),
        Some("Cannot open save dialog: org.freedesktop.portal.Desktop is not running")
    );
    assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 0);
    assert_eq!(h.eligibility("events.export"), Eligibility::Enabled);
    h.click("events.export");
    // The list changes under the open prompt: the export is the capture.
    h.open_tab(TabId::Alerts);
    h.open_view(NavView::Infrastructure);
    let path = dir.0.join("activity.json");
    h.answer_file(Some(path.clone()));
    for _ in 0..500 {
        if export::poll(&mut h.model) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(!h.model.areas.events.export.pending);
    assert_eq!(
        h.model.areas.events.notice,
        Some(format!("Activity saved to {}", path.display()))
    );
    let document: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(document["entries"].as_array().unwrap().len(), 2);
    assert_eq!(document["entries"][0]["entry"]["kind"], "task");
    assert_eq!(document["entries"][0]["current_connection"], false);
    assert!(
        document["scope"]
            .as_str()
            .unwrap()
            .starts_with("Tasks; pool ")
    );
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(path.metadata().unwrap().permissions().mode() & 0o777, 0o600);
    let link = dir.0.join("symlink.json");
    std::os::unix::fs::symlink(&path, &link).unwrap();
    let before = std::fs::read(&path).unwrap();
    assert!(crate::files::write_private(&link, b"{} ").is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

/// `activity_tests.rs`. The model half; the two window sizes are in
/// `gpui-tests-wanted.md`.
#[test]
fn outcome_help_opens_from_tasks_and_both_history_families_without_mutating_or_retargeting() {
    for family in [0, 1, 2] {
        let mut h = events();
        let pool = pool(&h);
        let key = match family {
            0 => {
                let mut key = String::new();
                amend(&mut h, |i| {
                    let task = i.details.tasks.values_mut().next().unwrap();
                    task.status = "failure".into();
                    task.error_code = Some("HA_OPERATION_WOULD_BREAK_FAILOVER_PLAN".into());
                    key = activity::row_key(pool, "task", task.id);
                });
                key
            }
            1 => {
                let mut record = vm_record(pool, OperationState::OutcomeUnknown);
                record.detail = "XAPI refused the request: SR_FULL".into();
                let key = activity::key_of(vm_key(&record));
                h.model.operation_rows.push(record);
                h.click("activity-History");
                key
            }
            _ => {
                let mut record = resource_record(pool);
                record.state = OperationState::PartiallyCompleted;
                record.detail = "Host task failed: SR_BACKEND_FAILURE_44".into();
                let key = activity::key_of(resource_key(&record));
                h.model.resource_rows.push(record);
                h.click("activity-History");
                key
            }
        };
        h.select_row(TABLE, &key);
        h.click("events.explain");
        assert!(h.model.areas.events.help.is_some());
        assert_eq!(h.surface_id().as_deref(), Some("failure-help-dialog"));
        assert_eq!(h.surface().title, "Outcome explanation");
        let expected = match family {
            0 => "HA restart protection",
            1 => "Do not submit the whole operation again",
            _ => "Some work completed",
        };
        assert!(h.has_text(expected), "missing {expected}");
        assert!(!h.has_text("OpaqueRef:private"));
        h.model.areas.events.selected = Some("different row".into());
        assert!(h.has_text(expected));
        h.click("failure-help-close");
        assert!(h.model.areas.events.help.is_none());
        assert!(h.model.operations.is_empty());
        assert!(h.model.resource_operations.is_empty());
    }
}

/// `activity_tests.rs`, as it was.
#[test]
fn successful_history_and_message_text_never_offer_error_code_guidance() {
    let mut h = events();
    let pool = pool(&h);
    let mut record = vm_record(pool, OperationState::Succeeded);
    record.detail = "Host task failed: SR_FULL".into();
    h.model.operation_rows.push(record);
    assert_eq!(rows(&h, Tab::History).len(), 1);
    assert!(rows(&h, Tab::History).iter().all(|r| r.help.is_none()));
    amend(&mut h, |i| {
        for message in i.details.messages.values_mut() {
            message.body = "XAPI refused the request: SR_FULL".into();
        }
    });
    assert_eq!(rows(&h, Tab::Messages).len(), 2);
    assert!(rows(&h, Tab::Messages).iter().all(|r| r.help.is_none()));
}

/// `failure_help.rs`. The model half; that Close is inside the window at
/// both sizes is in `gpui-tests-wanted.md`.
#[test]
fn captured_help_remains_read_only_and_close_is_visible_at_both_sizes() {
    let mut h = Harness::scene("failure-help");
    let captured = h.model.areas.events.help.as_ref().unwrap().source.clone();
    assert_eq!(
        h.node("failure-help-code").value.as_deref(),
        Some("SR_BACKEND_FAILURE_44")
    );
    assert!(h.has_text("Some work completed and later work stopped."));
    h.model.inventory = None;
    h.model.areas.events.selected = Some("different".into());
    assert_eq!(h.model.areas.events.help.as_ref().unwrap().source, captured);
    h.click("failure-help-close");
    assert!(h.model.areas.events.help.is_none());
    assert!(h.model.operations.is_empty());
    assert!(h.model.resource_operations.is_empty());
}

/// `resource_admin.rs`. The join is the record of its source pool; the
/// History list of the destination pool has it, and acknowledging it names
/// both pools and submits nothing from a view that cannot.
#[test]
fn joined_pool_history_is_visible_at_destination_and_acknowledges_both_reservations() {
    let mut h = events();
    let destination = pool(&h);
    let source = PoolId::from(uuid::Uuid::new_v4());
    let record = join_record(source, destination);
    let key = resource_key(&record);
    h.model.resource_rows.push(record.clone());
    h.click("activity-History");
    assert_eq!(h.model.areas.events.pool, Some(destination));
    h.select_row(TABLE, &activity::key_of(key));
    assert!(h.has_text("Fixture interrupted join"));
    assert!(control::resource_lines(&record).iter().any(|v| {
        v.contains("BOTH pools")
            && v.contains(&source.to_string())
            && v.contains(&destination.to_string())
    }));
    // No connection can control it here, as in the Rune fixture.
    assert_eq!(
        h.eligibility("events.acknowledge"),
        Eligibility::disabled(NOT_WRITABLE)
    );
    assert!(control::open_acknowledge(&mut h.model, key));
    assert_eq!(h.surface_id().as_deref(), Some("acknowledge-unknown"));
    assert!(h.has_text("BOTH pools"));
    assert_eq!(
        h.try_click("acknowledge-confirm"),
        Err(control::ACK_STALE.into())
    );
    control::confirm_acknowledge(&mut h.model);
    assert!(h.find("acknowledge-error").is_some());
    assert!(h.model.resource_operations.is_empty());
    h.click("acknowledge-cancel");
    assert!(h.model.areas.events.acknowledge.is_none());
}

/// `resource_admin.rs`:
/// `used_network_explains_disabled_mtu_and_delete_and_offline_pool_history_opens`,
/// the history half (the network half belongs to the `network` area). The
/// history of a pool is there without a connection, and offers no change.
#[test]
fn offline_pool_history_opens() {
    let mut h = events();
    let pool = pool(&h);
    let record = resource_record(pool);
    let key = resource_key(&record);
    h.model.resource_rows.push(record);
    h.model.demo = false;
    h.open_view(NavView::Infrastructure);
    super::show_history(&mut h.model, pool);
    assert_eq!(h.model.active_tab(), TabId::Events);
    assert_eq!(rows(&h, Tab::History).len(), 1);
    h.select_row(TABLE, &activity::key_of(key));
    assert!(h.has_text("Synthetic scan completed"));
    assert!(h.surfaces().is_empty());
    assert_eq!(local(&h), ["events.hide", "events.export"]);
}

/// `network_topology_ui.rs`. The record is built by hand here: the export
/// reads `topology_changes` whatever the action is.
#[test]
fn topology_export_contains_only_public_created_and_removed_ids() {
    let pool = PoolId::from(uuid::Uuid::new_v4());
    let mut value = resource_json(pool);
    value["state"] = json!("PartiallyCompleted");
    value["detail"] = json!("Synthetic partial VLAN change");
    value["topology_changes"] = json!({
        "created_vlans":[uuid::Uuid::from_u128(800)],"created_pifs":[uuid::Uuid::from_u128(801)],
        "removed_vlans":[uuid::Uuid::from_u128(700)],"removed_pifs":[uuid::Uuid::from_u128(701)]
    });
    let row: ResourceOperationRecord = serde_json::from_value(value).unwrap();
    let export = serde_json::to_value(Entry::resource(&row)).unwrap();
    assert_eq!(export["created"].as_array().unwrap().len(), 2);
    assert_eq!(export["removed"].as_array().unwrap().len(), 2);
    assert!(!export.to_string().contains("OpaqueRef:"));
    // The same ids are evidence lines of the selected row.
    let lines = super::evidence::resource(&row);
    assert!(lines.iter().any(|line| line.starts_with("Created VLAN: ")));
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("Removed interface: "))
    );
}

/// `main.rs`: `dialogs_outside_the_supported_core_carry_the_experimental_notice`,
/// this area's share: Activity is in the supported core, and so is every
/// surface that came from it.
#[test]
fn activity_and_its_dialogs_carry_no_experimental_notice() {
    for scene in ["activity", "failure-help"] {
        let h = Harness::scene(scene);
        assert!(!h.model.experimental_notice(), "{scene}");
        assert!(h.find(EXPERIMENTAL_NOTICE_KEY).is_none(), "{scene}");
    }
    let mut h = events();
    h.click("events.cancel-task");
    assert!(!h.model.experimental_notice());
    h.dismiss();
    let record = vm_record(pool(&h), OperationState::OutcomeUnknown);
    let key = vm_key(&record);
    h.model.operation_rows.push(record);
    assert!(super::acknowledge(&mut h.model, key));
    assert!(!h.model.experimental_notice());
    assert!(h.find(EXPERIMENTAL_NOTICE_KEY).is_none());
}

// -- 2. What the port changed ---------------------------------------------------

#[test]
fn the_scene_is_the_events_tab_on_tasks_with_the_first_task_selected() {
    let h = events();
    assert_eq!(h.model.nav.view, NavView::Notifications);
    assert_eq!(h.model.active_tab(), TabId::Events);
    assert_eq!(h.model.areas.events.list, Tab::Tasks);
    assert!(h.surfaces().is_empty());
    assert!(h.has_text("Synthetic task 1 · pending"));
    assert!(h.has_text(view::SOURCE_SYNTHETIC));
    assert!(h.has_text("Progress: 35%"));
    assert!(h.has_text("2 matching entries"));
    assert_eq!(local(&h), ["events.cancel-task", "events.export"]);
    let nodes = crate::surface::flatten_blocks("tab", &h.tab().blocks());
    assert!(crate::surface::duplicate_keys(&nodes).is_empty());
}

#[test]
fn the_alerts_tab_lists_messages_by_priority() {
    let mut h = events();
    h.open_tab(TabId::Alerts);
    assert!(h.find("activity-list").is_none());
    assert!(h.has_text("2 matching entries"));
    // The selection is one for all lists: the task is not a row here.
    assert_eq!(local(&h), ["events.export"]);
    let messages = rows(&h, Tab::Messages);
    assert_eq!(messages[0].status, "1");
    h.select_row(TABLE, &messages[1].key);
    assert_eq!(local(&h), ["alerts.dismiss", "events.export"]);
    assert!(h.has_text("Synthetic alert text 2. No host data is used."));
    assert!(h.has_text("Object: VM Unknown"));
    assert_eq!(
        h.row_menu(TABLE)
            .iter()
            .map(|item| item.id)
            .collect::<Vec<_>>(),
        [commands::DISMISS]
    );
    let nodes = crate::surface::flatten_blocks("tab", &h.tab().blocks());
    assert!(crate::surface::duplicate_keys(&nodes).is_empty());
}

#[test]
fn switching_between_tasks_and_history_clears_the_state_filter_and_keeps_the_search() {
    let mut h = events();
    h.type_text("activity-search", "synthetic");
    h.type_text("activity-state-filter", "pending");
    h.click("activity-History");
    assert_eq!(h.model.areas.events.list, Tab::History);
    assert!(h.model.areas.events.state_filter.is_empty());
    assert_eq!(h.model.areas.events.search, "synthetic");
    assert!(h.find("activity-show-hidden").is_some());
    assert!(h.find("activity-lists-note").is_none());
    // A switch between Alerts and Events empties the filter too, as the
    // tab switch of the Rune dialog did, and keeps the search.
    h.type_text("activity-state-filter", "succeeded");
    h.open_tab(TabId::Alerts);
    assert_eq!(h.node("activity-state-filter").value.as_deref(), Some(""));
    assert!(h.model.areas.events.state_filter.is_empty());
    h.type_text("activity-state-filter", "2");
    assert_eq!(h.model.areas.events.priority_filter, "2");
    h.open_tab(TabId::Events);
    assert_eq!(h.node("activity-state-filter").value.as_deref(), Some(""));
    h.open_tab(TabId::Alerts);
    assert!(h.model.areas.events.priority_filter.is_empty());
    assert_eq!(h.model.areas.events.search, "synthetic");
    // Choosing the tab that is shown is no switch: the filter stays.
    h.type_text("activity-state-filter", "3");
    h.open_tab(TabId::Alerts);
    assert_eq!(h.model.areas.events.priority_filter, "3");
    // A priority typed on Alerts never filters states, also when the
    // Notifications view was left in between (no switch between its two
    // tabs is reported then: this is why there are two filters).
    h.model.set_nav_view(NavView::Infrastructure);
    super::show(&mut h.model);
    assert_eq!(h.model.active_tab(), TabId::Events);
    assert_eq!(h.node("activity-state-filter").value.as_deref(), Some(""));
    // The counters of the status bar set their filter after the switch.
    h.open_tab(TabId::Alerts);
    h.model.show_unknown_outcomes();
    assert_eq!(h.model.active_tab(), TabId::Events);
    assert_eq!(h.model.areas.events.list, Tab::History);
    assert_eq!(h.model.areas.events.state_filter, "OutcomeUnknown");
    assert_eq!(h.model.areas.events.pool, None);
    h.model.show_operations();
    assert!(h.model.areas.events.state_filter.is_empty());
    assert_eq!(h.model.areas.events.list, Tab::History);
}

#[test]
fn a_task_can_be_reviewed_only_while_its_host_allows_cancelling() {
    let mut h = events();
    // The finished task: in the synthetic pool its review still opens, as
    // in Rune; outside it the command says what is missing.
    let tasks = rows(&h, Tab::Tasks);
    h.select_row(TABLE, &tasks[1].key);
    assert_eq!(h.eligibility("events.cancel-task"), Eligibility::Enabled);
    h.model.demo = false;
    assert_eq!(
        h.eligibility("events.cancel-task"),
        Eligibility::disabled(activity::CANCEL_REQUIRES)
    );
    activity::begin_review(&mut h.model, Tab::Tasks);
    assert!(h.model.areas.events.review.is_none());
    assert!(h.has_text(view::SOURCE_CACHED));
    // A task is not a message.
    assert_eq!(h.eligibility("alerts.dismiss"), Eligibility::Hidden);
}

#[test]
fn the_rows_are_remembered_until_what_they_were_built_from_changes() {
    let mut h = events();
    let first = activity::current(&h.model, Tab::Tasks);
    assert!(Rc::ptr_eq(&first, &activity::current(&h.model, Tab::Tasks)));
    // No hook runs between a change of the records and the next render.
    h.click("activity-History");
    assert!(activity::current(&h.model, Tab::History).rows.is_empty());
    let record = vm_record(pool(&h), OperationState::Succeeded);
    h.model.operation_rows.push(record);
    assert_eq!(activity::current(&h.model, Tab::History).rows.len(), 1);
    h.model.operation_rows[0].state = OperationState::Failed;
    assert_eq!(
        activity::current(&h.model, Tab::History).rows[0].status,
        "Failed"
    );
    amend(&mut h, |i| {
        i.details.tasks.clear();
    });
    assert!(activity::current(&h.model, Tab::Tasks).rows.is_empty());
}

#[test]
fn the_poll_reports_a_change_only_while_the_notifications_view_shows_it() {
    let mut quiet = Model::fixture();
    assert!(!super::poll(&mut quiet, 0.0));
    let mut h = events();
    // Whatever the first poll had to remember, the second has nothing new.
    super::poll(&mut h.model, 0.0);
    assert!(!super::poll(&mut h.model, 0.1));
    let record = vm_record(pool(&h), OperationState::Succeeded);
    h.model.operation_rows.push(record.clone());
    assert!(super::poll(&mut h.model, 0.2));
    h.open_view(NavView::Infrastructure);
    h.model
        .operation_rows
        .push(vm_record(record.pool, OperationState::Failed));
    assert!(!super::poll(&mut h.model, 0.3));
}

#[test]
fn an_unknown_vm_outcome_is_rechecked_and_acknowledged_only_through_its_pool() {
    let mut h = events();
    let pool = pool(&h);
    let record = vm_record(pool, OperationState::OutcomeUnknown);
    let key = vm_key(&record);
    h.model.operation_rows.push(record);
    h.click("activity-History");
    h.select_row(TABLE, &activity::key_of(key));
    assert!(h.has_text("Start · OutcomeUnknown"));
    assert_eq!(
        local(&h),
        [
            "events.explain",
            "events.hide",
            "events.recheck",
            "events.acknowledge",
            "events.export"
        ]
    );
    // Synthetic: nothing reaches a host.
    assert_eq!(h.eligibility("events.recheck"), Eligibility::disabled(DEMO));
    assert_eq!(control::recheck(&mut h.model, key), Err(DEMO.into()));
    h.click("events.acknowledge");
    // The one acknowledgement of a VM operation: the Rune dialog of
    // `main.rs`, by its key, its title and its texts.
    assert_eq!(h.surface_id().as_deref(), Some("unknown-outcome"));
    assert_eq!(h.surface().title, "Acknowledge unknown outcome");
    assert!(h.has_text("Acknowledge unknown outcome?"));
    assert!(
        h.node("unknown-target")
            .text
            .is_some_and(|text| text.starts_with("VM: "))
    );
    assert_eq!(
        h.node("unknown-consequence").text.as_deref(),
        Some(control::ACK_VM)
    );
    assert_eq!(h.node("unknown-acknowledge").tone, Some(Tone::Warning));
    assert!(h.find("unknown-preview").is_some());
    assert_eq!(h.try_click("unknown-acknowledge"), Err(DEMO.into()));
    control::confirm_acknowledge(&mut h.model);
    assert!(h.find("unknown-error").is_some());
    assert!(h.model.areas.events.acknowledge.is_some());
    // The record resolves under the open review: it cannot be confirmed.
    h.model.demo = false;
    h.model.operation_rows[0].state = OperationState::Succeeded;
    assert_eq!(
        h.try_click("unknown-acknowledge"),
        Err(control::ACK_STALE.into())
    );
    assert_eq!(
        h.node("unknown-stale").text.as_deref(),
        Some(control::ACK_STALE)
    );
    h.dismiss();
    assert!(h.model.areas.events.acknowledge.is_none());
    // Not synthetic and not connected: the reason says so.
    h.model.operation_rows[0].state = OperationState::OutcomeUnknown;
    assert_eq!(h.try_click("events.recheck"), Err(NOT_WRITABLE.into()));
    assert!(h.model.operations.is_empty());
}

#[test]
fn cancellation_is_named_by_what_stops_and_a_batch_child_is_left_to_its_batch() {
    let mut h = events();
    let pool = pool(&h);
    let mut running = vm_json(pool, OperationState::Tracking);
    running["can_cancel"] = json!(true);
    let running: OperationRecord = serde_json::from_value(running).unwrap();
    assert!(running.can_cancel, "the fixture can be cancelled");
    let key = vm_key(&running);
    h.model.operation_rows.push(running);
    h.click("activity-History");
    h.select_row(TABLE, &activity::key_of(key));
    assert_eq!(control::cancel_label(&h.model, key), "Request cancellation");
    assert_eq!(
        h.eligibility("events.request-cancel"),
        Eligibility::disabled(DEMO)
    );
    assert_eq!(control::cancel(&mut h.model, key), Err(DEMO.into()));
    // Unfinished: it stays visible, and Rune offered Recheck for it too.
    assert_eq!(
        h.eligibility("events.hide"),
        Eligibility::disabled(activity::UNRESOLVED)
    );
    assert_eq!(h.eligibility("events.acknowledge"), Eligibility::Hidden);
    let parent = OperationId::new();
    h.model.operation_rows[0].batch_parent = Some(parent);
    let reason = control::batch_reason(parent);
    assert_eq!(
        h.eligibility("events.request-cancel"),
        Eligibility::disabled(reason.clone())
    );
    assert_eq!(
        h.eligibility("events.recheck"),
        Eligibility::disabled(reason)
    );
    // Asked for already, or finished: no such button.
    h.model.operation_rows[0].batch_parent = None;
    h.model.operation_rows[0].cancel_requested = true;
    assert_eq!(h.eligibility("events.request-cancel"), Eligibility::Hidden);
    h.model.operation_rows[0].state = OperationState::Cancelled;
    assert_eq!(h.eligibility("events.recheck"), Eligibility::Hidden);
    assert_eq!(h.eligibility("events.hide"), Eligibility::Enabled);
    // A resource operation that can be cancelled needs its connection.
    let mut scan = resource_json(pool);
    scan["state"] = json!("Tracking");
    scan["can_cancel"] = json!(true);
    let scan: ResourceOperationRecord = serde_json::from_value(scan).unwrap();
    let key = resource_key(&scan);
    h.model.resource_rows.push(scan);
    assert_eq!(
        control::cancel_eligibility(&h.model, key),
        Eligibility::disabled(DEMO)
    );
    assert_eq!(
        control::recheck_eligibility(&h.model, key),
        Eligibility::disabled(DEMO)
    );
}

#[test]
fn another_area_shows_explains_and_controls_an_operation_by_its_key() {
    let mut h = events();
    let pool = pool(&h);
    let done = vm_record(pool, OperationState::Succeeded);
    let done_key = vm_key(&done);
    let mut unknown = resource_json(pool);
    unknown["state"] = json!("OutcomeUnknown");
    unknown["detail"] = json!("Host task failed: SR_FULL");
    let unknown: ResourceOperationRecord = serde_json::from_value(unknown).unwrap();
    let unknown_key = resource_key(&unknown);
    let id = unknown.id;
    h.model.operation_rows.push(done);
    h.model.resource_rows.push(unknown);
    h.model.preferences.hidden_history.push(done_key);
    h.model.areas.events.search = "nothing matches".into();
    h.model.areas.events.state_filter = "pending".into();
    h.open_view(NavView::Infrastructure);
    // A hidden finished record is shown when it is asked for by key.
    super::show_operation(&mut h.model, done_key);
    assert_eq!(h.model.nav.view, NavView::Notifications);
    assert_eq!(h.model.active_tab(), TabId::Events);
    assert_eq!(h.model.areas.events.list, Tab::History);
    assert!(h.model.areas.events.show_hidden);
    assert_eq!(
        activity::selected(&h.model, Tab::History).map(|row| row.key),
        Some(activity::key_of(done_key))
    );
    assert!(h.has_text("Start · Succeeded"));
    assert!(!super::can_explain(&h.model, done_key));
    assert!(!super::explain(&mut h.model, done_key));
    assert!(super::outcome_actions(&h.model, done_key).is_empty());
    // The banner of an unknown outcome: explain, recheck, acknowledge.
    let buttons = super::outcome_actions(&h.model, unknown_key);
    assert_eq!(
        buttons.iter().map(|b| b.key.clone()).collect::<Vec<_>>(),
        [
            format!("resource-failure-help-{id}"),
            format!("operation-recheck-{id}"),
            format!("operation-acknowledge-{id}"),
        ]
    );
    assert_eq!(buttons[1].label, "Recheck outcome");
    assert_eq!(buttons[1].disabled.as_deref(), Some(DEMO));
    assert_eq!(buttons[2].label, "Acknowledge unknown outcome…");
    assert!(!super::acknowledge(&mut h.model, unknown_key));
    assert_eq!(h.model.error.as_deref(), Some(DEMO));
    assert!(super::explain(&mut h.model, unknown_key));
    assert_eq!(h.surface_id().as_deref(), Some("failure-help-dialog"));
    assert!(h.has_text("Do not submit the whole operation again"));
    assert_eq!(
        h.node("failure-help-code").value.as_deref(),
        Some("SR_FULL")
    );
    h.dismiss();
    // The status bar's unknown outcomes: every pool, that state.
    super::show_states(&mut h.model, "OutcomeUnknown");
    assert_eq!(h.model.areas.events.pool, None);
    assert_eq!(rows(&h, Tab::History).len(), 1);
}

#[test]
fn a_resource_record_shows_its_evidence_and_local_error_in_the_outcome() {
    let mut h = events();
    let pool = pool(&h);
    let mut value = resource_json(pool);
    value["state"] = json!("PartiallyCompleted");
    value["steps"] =
        json!([{"label":"Scan","state":"Failed","detail":"Host task failed: SR_FULL"}]);
    value["topology_changes"] = json!({
        "created_vlans":[uuid::Uuid::from_u128(800)],"created_pifs":[],
        "removed_vlans":[],"removed_pifs":[]
    });
    let mut record: ResourceOperationRecord = serde_json::from_value(value).unwrap();
    record.local_error = Some("Journal write failed".into());
    let key = resource_key(&record);
    h.model.resource_rows.push(record);
    super::show_operation(&mut h.model, key);
    assert!(h.has_text("· PartiallyCompleted"));
    assert!(h.has_text("Scan: Failed — Host task failed: SR_FULL"));
    assert!(h.has_text("Journal write failed"));
    assert!(h.has_text("Created VLAN: "));
    assert_eq!(
        h.node("activity-outcome").value.as_deref(),
        Some(OutcomeState::Partial.name())
    );
    // The detail lines are one preformatted text, one line each, so that
    // an identifier or a host's error can be selected and copied whole.
    let details = h.node("activity-details");
    assert_eq!(details.role, "preformatted");
    let text = details.text.expect("the detail lines");
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines.len() >= 3, "{lines:?}");
    assert!(lines.contains(&"Scan: Failed — Host task failed: SR_FULL"));
    assert!(lines.contains(&"Journal write failed"));
    assert!(lines.iter().all(|line| !line.is_empty()));
}

// -- 3. Texts that named Rune places (AREAS.md decision 10) ---------------------

#[test]
fn texts_name_the_places_of_this_client() {
    assert!(failure_help::NO_CODE.contains("in the operation or its entry on the Events tab of the Notifications view"));
    assert_eq!(
        activity::SUBMITTED,
        "Change submitted. History tracks the host outcome."
    );
    let parent = OperationId::new();
    assert_eq!(
        control::batch_reason(parent),
        format!("Managed by batch {parent}. Use the batch controls on the Events tab of the Notifications view.")
    );
    assert_eq!(
        view::SOURCE_CACHED,
        "Cached inventory · this pool is not currently selected and connected"
    );
    assert_eq!(EXPORTING, "Saving the captured activity selection…");
    assert!(view::EXPORT_NOTE.starts_with("JSON exports contain the displayed task/message"));
}
