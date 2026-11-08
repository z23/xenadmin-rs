use super::*;
use serde_json::json;
use xenadmin_core::{OperationRecord, OperationState, ResourceOperationRecord};

fn state() -> State {
    let mut s = init();
    demo(&mut s);
    scene(&mut s);
    s
}
fn runtime(mut s: State, size: (f32, f32)) -> rune::Runtime<State> {
    s.window_width = size.0;
    s.window_height = size.1;
    let mut rt = rune::Runtime::new(
        s,
        view,
        size,
        rune::Env {
            system_dark: true,
            scale: 1.0,
        },
    );
    settle(&mut rt);
    rt
}
fn settle(rt: &mut rune::Runtime<State>) {
    for _ in 0..30 {
        rt.frame(1.0 / 60.0);
    }
}
fn click(rt: &mut rune::Runtime<State>, key: &str) {
    rt.update(|_, tx| tx.scroll_into_view(key));
    settle(rt);
    let (x, y) = rt.find(key).unwrap_or_else(|| panic!("missing {key}"));
    assert!(
        x >= 0.0 && x <= rt.state.window_width && y >= 0.0 && y <= rt.state.window_height,
        "{key} outside viewport: {x},{y}"
    );
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    settle(rt);
}
fn vm_record(pool: PoolId, state: OperationState) -> OperationRecord {
    serde_json::from_value(json!({"id":OperationId::new(),"pool":pool,"vm":uuid::Uuid::from_u128(30),"action":"Start","state":state,"progress":null,"detail":"Synthetic operation result","result_reference":"OpaqueRef:private-result","task":"OpaqueRef:private-task"})).unwrap()
}
fn resource_record(pool: PoolId) -> ResourceOperationRecord {
    serde_json::from_value(json!({"id":OperationId::new(),"pool":pool,"target":{"Storage":uuid::Uuid::from_u128(40)},"action":"ScanStorage","state":"Succeeded","progress":1.0,"detail":"Synthetic scan completed","created_at_ms":123,"can_cancel":false,"cancel_requested":false,"steps":[],"created":null,"baseline_networks":[uuid::Uuid::from_u128(41)]})).unwrap()
}

#[test]
fn activity_review_captures_exact_target_and_never_dispatches_from_synthetic_or_stale_views() {
    for size in [(1440.0, 900.0), (513.0, 652.0)] {
        let mut rt = runtime(state(), size);
        click(&mut rt, "activity-review");
        let target = rt.state.activity.review.as_ref().unwrap().target;
        assert!(matches!(target, ResourceTarget::Task(_)));
        rt.state.activity.table.select("different-selection");
        let inventory = Arc::make_mut(rt.state.inventory.as_mut().unwrap());
        inventory.pool_id = PoolId::from(uuid::Uuid::new_v4());
        click(&mut rt, "activity-confirm");
        assert_eq!(rt.state.activity.review.as_ref().unwrap().target, target);
        assert!(rt.state.resource_operations.is_empty());
        assert!(!review_current(
            &rt.state,
            rt.state.activity.review.as_ref().unwrap()
        ));
        click(&mut rt, "activity-close");
        assert!(!rt.state.activity.show);
        click(&mut rt, "activity-open");
        assert!(rt.state.activity.show);
    }
}
#[test]
fn message_review_keeps_original_content_digest_when_selection_or_inventory_changes() {
    let mut s = state();
    s.activity.tab = Tab::Messages;
    rebuild(&mut s);
    s.activity.table.select(s.activity.rows[0].key.clone());
    begin_review(&mut s);
    let review = s.activity.review.as_ref().unwrap();
    let target = review.target;
    let digest = review.digest;
    s.activity.table.select(s.activity.rows[1].key.clone());
    let ResourceTarget::Message(id) = target else {
        panic!()
    };
    Arc::make_mut(s.inventory.as_mut().unwrap())
        .details
        .messages
        .get_mut(&id)
        .unwrap()
        .body = "Changed text".into();
    poll(&mut s);
    assert_eq!(s.activity.review.as_ref().unwrap().target, target);
    assert_eq!(s.activity.review.as_ref().unwrap().digest, digest);
    assert_ne!(
        s.inventory.as_ref().unwrap().details.messages[&id].review_digest(),
        digest.unwrap()
    );
    submit(&mut s);
    assert!(s.resource_operations.is_empty());
}
#[test]
fn filters_cover_message_body_priority_task_state_and_cached_pools() {
    let mut s = state();
    assert_eq!(s.activity.rows.len(), 2);
    s.activity.state_filter = TextEdit::new("pending");
    rebuild(&mut s);
    assert_eq!(s.activity.rows.len(), 1);
    s.activity.tab = Tab::Messages;
    s.activity.state_filter = TextEdit::new("2");
    s.activity.search = TextEdit::new("alert text 2");
    rebuild(&mut s);
    assert_eq!(s.activity.rows.len(), 1);
    let original_pool = s.activity.rows[0].pool;
    s.inventory = None;
    s.snapshot = None;
    poll(&mut s);
    assert_eq!(s.activity.rows[0].pool, original_pool);
    assert!(!allowed(&s, &s.activity.rows[0]));
    let mut second = s.activity.pools[&original_pool].as_ref().clone();
    second.pool_id = PoolId::from(uuid::Uuid::new_v4());
    second.pool_name = "Second synthetic pool".into();
    s.inventory = Some(Arc::new(second));
    poll(&mut s);
    s.activity.pool = None;
    rebuild(&mut s);
    assert_eq!(s.activity.rows.len(), 2);
    next_pool(&mut s);
    assert_eq!(s.activity.rows.len(), 1);
}
#[test]
fn history_hiding_is_terminal_only_pool_scoped_restorable_and_survives_disconnect() {
    let mut s = state();
    let pool = s.inventory.as_ref().unwrap().pool_id;
    let done = vm_record(pool, OperationState::Succeeded);
    let unknown = vm_record(pool, OperationState::OutcomeUnknown);
    let done_id = done.id;
    let unknown_id = unknown.id;
    s.operation_rows = vec![done, unknown];
    s.resource_rows = vec![resource_record(pool)];
    s.activity.tab = Tab::History;
    rebuild(&mut s);
    assert_eq!(s.activity.rows.len(), 3);
    s.activity.table.select(row_key(pool, "vm", done_id));
    toggle_hidden(&mut s);
    assert_eq!(s.activity.rows.len(), 2);
    assert_eq!(s.operation_rows.len(), 2);
    let serialized = serde_json::to_vec(&s.preferences).unwrap();
    s.preferences = serde_json::from_slice(&serialized).unwrap();
    s.inventory = None;
    s.snapshot = None;
    poll(&mut s);
    assert_eq!(s.activity.rows.len(), 2);
    s.activity.table.select(row_key(pool, "vm", unknown_id));
    toggle_hidden(&mut s);
    assert_eq!(s.preferences.hidden_history.len(), 1);
    // A stale preference can never hide an unresolved result.
    s.preferences
        .hidden_history
        .push(history_key(pool, unknown_id, Family::Vm));
    rebuild(&mut s);
    assert!(
        s.activity
            .rows
            .iter()
            .any(|r| r.key == row_key(pool, "vm", unknown_id))
    );
    s.activity.show_hidden = true;
    rebuild(&mut s);
    s.activity.table.select(row_key(pool, "vm", done_id));
    toggle_hidden(&mut s);
    s.activity.show_hidden = false;
    rebuild(&mut s);
    assert_eq!(s.activity.rows.len(), 3);
}
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
    let bytes = serde_json::to_string(&vec![
        activity_export::Entry::vm(&vm),
        activity_export::Entry::resource(&resource),
    ])
    .unwrap();
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
#[test]
fn save_dialog_exports_captured_selection_even_after_switch_and_cancel_writes_nothing() {
    let dir = std::env::temp_dir().join(format!("xenadmin-activity-ui-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    let mut rt = rune::Runtime::new(
        state(),
        |_: &State, cx: &mut Cx<State>| {
            Button::new("Export")
                .on_press_tx(activity_export::choose)
                .build(&Kit::neutral(cx.system_dark()))
                .key("export")
        },
        (480.0, 640.0),
        rune::Env {
            system_dark: true,
            scale: 1.0,
        },
    );
    let pick = |rt: &mut rune::Runtime<State>| {
        rt.frame(0.0);
        let (x, y) = rt.find("export").unwrap();
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(0.0);
        assert_eq!(rt.file_dialog().unwrap().0, rune::FileDialogKind::Save);
    };
    pick(&mut rt);
    rt.resolve_file_dialog(None).unwrap();
    rt.frame(0.0);
    assert!(!rt.state.activity.export.pending);
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
    pick(&mut rt);
    rt.state.activity.tab = Tab::Messages;
    rebuild(&mut rt.state);
    rt.state.activity.show = false;
    let path = dir.join("activity.json");
    rt.resolve_file_dialog(Some(path.clone())).unwrap();
    rt.frame(0.0);
    for _ in 0..500 {
        if activity_export::poll(&mut rt.state) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(!rt.state.activity.export.pending);
    let document: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(document["entries"].as_array().unwrap().len(), 2);
    assert_eq!(document["entries"][0]["entry"]["kind"], "task");
    assert_eq!(document["entries"][0]["current_connection"], false);
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(path.metadata().unwrap().permissions().mode() & 0o777, 0o600);
    let link = dir.join("symlink.json");
    std::os::unix::fs::symlink(&path, &link).unwrap();
    let before = std::fs::read(&path).unwrap();
    assert!(activity_export::write_private(&link, b"{} ").is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn outcome_help_opens_from_tasks_and_both_history_families_without_mutating_or_retargeting() {
    for size in [(1440.0, 900.0), (513.0, 652.0)] {
        for family in [0, 1, 2] {
            let mut s = state();
            let pool = s.inventory.as_ref().unwrap().pool_id;
            let key = match family {
                0 => {
                    let task = Arc::make_mut(s.activity.pools.get_mut(&pool).unwrap())
                        .details
                        .tasks
                        .values_mut()
                        .next()
                        .unwrap();
                    task.status = "failure".into();
                    task.error_code = Some("HA_OPERATION_WOULD_BREAK_FAILOVER_PLAN".into());
                    row_key(pool, "task", task.id)
                }
                1 => {
                    s.activity.tab = Tab::History;
                    let mut record = vm_record(pool, OperationState::OutcomeUnknown);
                    record.detail = "XAPI refused the request: SR_FULL".into();
                    let key = row_key(pool, "vm", record.id);
                    s.operation_rows.push(record);
                    key
                }
                _ => {
                    s.activity.tab = Tab::History;
                    let mut record = resource_record(pool);
                    record.state = OperationState::PartiallyCompleted;
                    record.detail = "Host task failed: SR_BACKEND_FAILURE_44".into();
                    let key = row_key(pool, "resource", record.id);
                    s.resource_rows.push(record);
                    key
                }
            };
            rebuild(&mut s);
            s.activity.table.select(key);
            let mut rt = runtime(s, size);
            click(&mut rt, "activity-failure-help");
            assert!(rt.state.failure_help.is_some());
            let text = rt.accessibility_json().to_string();
            assert!(text.contains("Outcome explanation"));
            let expected = match family {
                0 => "HA restart protection",
                1 => "Do not submit the whole operation again",
                _ => "Some work completed",
            };
            assert!(text.contains(expected), "missing {expected}");
            assert!(!text.contains("OpaqueRef:private"));
            rt.update(|s, _| s.activity.table.select("different row"));
            settle(&mut rt);
            assert!(rt.accessibility_json().to_string().contains(expected));
            click(&mut rt, "failure-help-close");
            assert!(rt.state.failure_help.is_none());
            assert!(rt.state.operations.is_empty());
            assert!(rt.state.resource_operations.is_empty());
        }
    }
}

#[test]
fn successful_history_and_message_text_never_offer_error_code_guidance() {
    let mut s = state();
    let pool = s.inventory.as_ref().unwrap().pool_id;
    let mut record = vm_record(pool, OperationState::Succeeded);
    record.detail = "Host task failed: SR_FULL".into();
    s.operation_rows.push(record);
    s.activity.tab = Tab::History;
    rebuild(&mut s);
    assert!(s.activity.rows.iter().all(|r| r.help.is_none()));
    s.activity.tab = Tab::Messages;
    for message in Arc::make_mut(s.activity.pools.get_mut(&pool).unwrap())
        .details
        .messages
        .values_mut()
    {
        message.body = "XAPI refused the request: SR_FULL".into();
    }
    rebuild(&mut s);
    assert!(s.activity.rows.iter().all(|r| r.help.is_none()));
}
