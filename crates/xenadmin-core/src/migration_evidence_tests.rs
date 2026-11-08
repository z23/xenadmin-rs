use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

#[tokio::test]
async fn legacy_migration_plan_reopens_with_interruption_fence_and_success_requires_task_receipt() {
    let (f, _, _) = fixture(true, true, Fault::LostReply).await;
    let dir = tempfile::tempdir().unwrap();
    let row = execute(&f, dir.path(), operation(true, true)).await;
    let j = Journal::open(dir.path()).unwrap();
    let encoded = Journal::encode_record(&row).unwrap();
    let mut value: Value = serde_json::from_str(&encoded).unwrap();
    for key in ["interrupted", "outcome", "affinity_received"] {
        value["migration_plan"].as_object_mut().unwrap().remove(key);
    }
    j.0.execute(
        "UPDATE operations SET payload=?1 WHERE id=?2",
        rusqlite::params![value.to_string(), row.id.to_string()],
    )
    .unwrap();
    let loaded = j.load(row.id).unwrap().unwrap();
    assert!(loaded.migration_plan.as_ref().unwrap().interrupted);
    assert_eq!(
        loaded.migration_plan.as_ref().unwrap().operation,
        operation(true, true)
    );
    value["migration_plan"]["completed"] = json!(true);
    value["state"] = json!("Succeeded");
    value["steps"][0]["state"] = json!("Verified");
    j.0.execute(
        "UPDATE operations SET payload=?1,task=NULL WHERE id=?2",
        rusqlite::params![value.to_string(), row.id.to_string()],
    )
    .unwrap();
    assert!(
        j.load(row.id).is_err(),
        "success without any received task must fail closed"
    );
}

#[tokio::test]
async fn late_replacement_disk_proof_survives_recovery_and_ack_without_home_host_change() {
    for acknowledge in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_owned();
        let id = OperationId::new();
        let complete = Arc::new(AtomicBool::new(false));
        let fired = Arc::new(AtomicBool::new(false));
        let triggered = fired.clone();
        let hook: Hook = Arc::new(move |request| {
            if request["method"] == "task.get_record" {
                complete.store(true, Ordering::SeqCst);
            }
            if complete.load(Ordering::SeqCst)
                && request["method"] == "SR.get_record"
                && !triggered.swap(true, Ordering::SeqCst)
            {
                recover(&path, uuid(1).into()).unwrap();
                if acknowledge {
                    Journal::open(&path)
                        .unwrap()
                        .control(uuid(1).into(), id, true)
                        .unwrap();
                }
            }
        });
        let (f, _, calls) = fixture_hook(false, true, Fault::None, Some(hook)).await;
        let record = execute_id(&f, dir.path(), operation(false, true), id).await;
        assert!(fired.load(Ordering::SeqCst));
        assert_eq!(
            record.state,
            if acknowledge {
                OperationState::AcknowledgedUnknown
            } else {
                OperationState::OutcomeUnknown
            },
            "{}",
            record.detail
        );
        assert_eq!(
            record.created_devices,
            vec![CreatedDevice::Disk(uuid(16).into())]
        );
        assert!(record.migration_plan.as_ref().unwrap().completed);
        assert!(record.migration_plan.as_ref().unwrap().interrupted);
        assert_eq!(count(&calls, "VM.set_affinity"), 0);
        assert_eq!(count(&calls, "Async.VM.migrate_send"), 1);
        assert_eq!(count(&calls, "host.migrate_receive"), 1);
        assert_eq!(count(&calls, "task.destroy"), 0);
        if !acknowledge {
            let after = recheck(&f, dir.path(), record).await;
            assert_eq!(
                after.state,
                OperationState::PartiallyCompleted,
                "{}",
                after.detail
            );
            assert_eq!(count(&calls, "VM.set_affinity"), 0);
        }
    }
}

#[tokio::test]
async fn immutable_migration_plan_and_public_steps_fail_closed_and_private_tokens_stay_absent() {
    let (f, _, _) = fixture(true, true, Fault::LostReply).await;
    let dir = tempfile::tempdir().unwrap();
    let row = execute(&f, dir.path(), operation(true, true)).await;
    assert_eq!(row.state, OperationState::OutcomeUnknown);
    let j = Journal::open(dir.path()).unwrap();
    let public = serde_json::to_string(&row).unwrap();
    assert!(!public.contains("migration_plan"));
    assert!(!public.contains("OpaqueRef:"));
    let payload: String =
        j.0.query_row(
            "SELECT payload FROM operations WHERE id=?1",
            [row.id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!payload.contains("PRIVATE-RECEIVER-TOKEN"));
    assert!(!payload.contains("SECRET-MIGRATION-TOKEN"));
    for fault in 0..5 {
        let mut changed = row.clone();
        match fault {
            0 => {
                changed
                    .migration_plan
                    .as_mut()
                    .unwrap()
                    .operation
                    .destination = uuid(99).into()
            }
            1 => changed.migration_plan.as_mut().unwrap().graph.disks.clear(),
            2 => changed.migration_plan.as_mut().unwrap().baseline.clear(),
            3 => changed.steps.clear(),
            _ => {
                changed.state = OperationState::Verifying;
                changed.migration_plan.as_mut().unwrap().interrupted = false;
            }
        }
        assert!(j.save(&mut changed).is_err(), "{fault}");
    }
    let mut invalid: Value = serde_json::from_str(&payload).unwrap();
    invalid["migration_plan"]["vm"] = json!(uuid(99));
    j.0.execute(
        "UPDATE operations SET payload=?1 WHERE id=?2",
        rusqlite::params![invalid.to_string(), row.id.to_string()],
    )
    .unwrap();
    assert!(j.load(row.id).is_err());
}

#[tokio::test]
async fn pending_migration_recheck_never_sends_unsent_cancellation() {
    let (f, _, _) = fixture(true, false, Fault::PollMissing).await;
    let dir = tempfile::tempdir().unwrap();
    let mut row = execute(&f, dir.path(), operation(true, false)).await;
    let j = Journal::open(dir.path()).unwrap();
    row.cancel_requested = true;
    j.save(&mut row).unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let log = calls.clone();
    let pending = Fixture::new(move |request| {
        let method = request["method"].as_str().unwrap().to_string();
        log.lock().unwrap().push(method.clone());
        assert_eq!(method, "task.get_record");
        json!({"result":{"status":"pending","progress":0.5,"allowed_operations":["cancel"]}})
    })
    .await;
    let result = recheck(&pending, dir.path(), row).await;
    assert_eq!(result.state, OperationState::OutcomeUnknown);
    assert!(!result.cancel_dispatched);
    assert_eq!(*calls.lock().unwrap(), vec!["task.get_record"]);
}
