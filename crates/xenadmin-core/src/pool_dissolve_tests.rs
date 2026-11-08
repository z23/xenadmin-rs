use crate::*;
use crate::{
    operations::{Journal, OperationControl},
    xapi::inventory_tests::Fixture,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
use uuid::Uuid;
fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}
fn operation(members: usize, description: bool) -> PoolDissolve {
    PoolDissolve {
        coordinator: id(2).into(),
        expected_name: "Example pool".into(),
        expected_description: if description { "Pool description" } else { "" }.into(),
        members: (3..3 + members as u128)
            .map(|n| PoolDissolveMember {
                host: id(n).into(),
                name: format!("Host {n}"),
            })
            .collect(),
    }
}
struct Server {
    fixture: Fixture,
    records: Arc<Mutex<BTreeMap<String, Value>>>,
    calls: Arc<Mutex<Vec<String>>>,
}
async fn server(
    op: &PoolDissolve,
    fault: &str,
    directory: &std::path::Path,
    operation: OperationId,
) -> Server {
    let mut records = BTreeMap::from([
        (
            "OpaqueRef:pool".into(),
            json!({"uuid":id(1),"name_label":op.expected_name,"name_description":op.expected_description,"master":"OpaqueRef:host2","ha_enabled":false,"current_operations":{}}),
        ),
        ("OpaqueRef:metrics".into(), json!({"live":true})),
    ]);
    for n in 2..3 + op.members.len() as u128 {
        records.insert(format!("OpaqueRef:host{n}"),json!({"uuid":id(n),"name_label":format!("Host {n}"),"enabled":false,"metrics":"OpaqueRef:metrics","current_operations":{}}));
    }
    let records = Arc::new(Mutex::new(records));
    let data = records.clone();
    let calls = Arc::new(Mutex::new(vec![]));
    let seen = calls.clone();
    let permissions = if fault == "denied-final-setter" {
        json!({"is_local_superuser":false,"rbac_permissions":["pool.eject", "pool.set_name_label"]})
    } else {
        json!({"is_local_superuser":true})
    };
    let fault = fault.to_owned();
    let directory = directory.to_path_buf();
    let fixture=Fixture::with_permissions(move|r| {
        let method=r["method"].as_str().unwrap();let p=&r["params"];
        seen.lock().unwrap().push(method.into());
        let mut data=data.lock().unwrap();
        if let Some(class)=method.strip_suffix(".get_by_uuid") {
            return data.iter().find(|(k,v)|k.starts_with(&format!("OpaqueRef:{class}"))&&v["uuid"]==p[1]).map(|(k,_)|json!({"result":k})).unwrap_or(json!({"error":{"message":"UUID_INVALID"}}));
        }
        if let Some(class)=method.strip_suffix(".get_all_records") {
            let rows=data.iter().filter(|(k,_)|k.starts_with(&format!("OpaqueRef:{class}"))).map(|(k,v)|(k.clone(),v.clone())).collect::<serde_json::Map<_,_>>();
            return json!({"result":rows});
        }
        if method=="Async.pool.eject" {
            let target=p[1].as_str().unwrap();data.remove(target).unwrap();
            let number=if fault=="old-task" || fault=="reused-task" {100} else {100+data.keys().filter(|k|k.starts_with("OpaqueRef:task")).count() as u128};
            let reference=format!("OpaqueRef:task{number}");
            data.insert(reference.clone(),json!({"uuid":id(number),"status":if fault=="task-failure"{"failure"}else if fault=="recover-pending"{"pending"}else{"success"},"progress":1.0,"result":"<value></value>"}));
            if target=="OpaqueRef:host3" {
                match fault.as_str() {
                    "lost-eject"=>return json!({"_fixture_close_without_reply":true}),
                    "changed-next"=>{data.get_mut("OpaqueRef:host4").unwrap()["enabled"]=json!(true);},
                    "extra-host"=>{data.insert("OpaqueRef:host9".into(),json!({"uuid":id(9)}));},
                    "coordinator"=>data.get_mut("OpaqueRef:pool").unwrap()["master"]=json!("OpaqueRef:host4"),
                    "ha"=>data.get_mut("OpaqueRef:pool").unwrap()["ha_enabled"]=json!(true),
                    "recover-eject"|"ack-eject"|"recover-pending"=>{
                        let j=Journal::open(&directory).unwrap();j.resource_recover(id(1).into()).unwrap();
                        if fault=="ack-eject" {j.resource_control(id(1).into(),operation,OperationControl::AcknowledgeUnknown).unwrap();}
                    },
                    "stop"=>{Journal::open(&directory).unwrap().resource_control(id(1).into(),operation,OperationControl::RequestCancellation).unwrap();},
                    _=>{}
                }
            }
            return json!({"result":reference});
        }
        if method=="pool.set_name_label"||method=="pool.set_name_description" {
            data.get_mut("OpaqueRef:pool").unwrap()[if method.ends_with("label"){"name_label"}else{"name_description"}]=p[2].clone();
            if method.ends_with("label")&&fault=="lost-name" {return json!({"_fixture_close_without_reply":true});}
            if method.ends_with("label") && ["recover-name","ack-name"].contains(&fault.as_str()) {
                let j=Journal::open(&directory).unwrap();j.resource_recover(id(1).into()).unwrap();
                if fault=="ack-name" {j.resource_control(id(1).into(),operation,OperationControl::AcknowledgeUnknown).unwrap();}
            }
            return json!({"result":null});
        }
        if method=="task.destroy" {data.remove(p[1].as_str().unwrap());return json!({"result":null});}
        if method.ends_with(".get_record") {return json!({"result":data.get(p[1].as_str().unwrap()).cloned().unwrap_or(Value::Null)});}
        json!({"error":{"message":"UNEXPECTED_CALL"}})
    },permissions).await;
    Server {
        fixture,
        records,
        calls,
    }
}
fn pending(
    op: PoolDissolve,
    operation: OperationId,
) -> (ResourceOperation, crate::resources::ResourceRequest) {
    crate::resources::channel(
        operation,
        ObservedResource {
            pool: id(1).into(),
            target: ResourceTarget::Pool(id(1).into()),
            generation: 1,
        },
        ResourceAction::DissolvePool(op),
        tokio::runtime::Handle::current(),
    )
}
async fn run(s: &Server, dir: &std::path::Path, request: crate::resources::ResourceRequest) {
    let (tx, mut rx) = tokio::sync::mpsc::channel(32);
    let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
    request
        .run(
            Arc::new(s.fixture.session().await),
            dir.into(),
            tokio_util::sync::CancellationToken::new(),
            tx,
        )
        .await;
    drain.await.unwrap();
}
async fn recheck(
    s: &Server,
    dir: &std::path::Path,
    record: ResourceOperationRecord,
) -> ResourceOperationRecord {
    let operation = record.id;
    run(s, dir, crate::resources::ResourceRequest::tracking(record)).await;
    Journal::open(dir)
        .unwrap()
        .resource_load(operation)
        .unwrap()
        .unwrap()
}
fn writes(s: &Server) -> Vec<String> {
    s.calls
        .lock()
        .unwrap()
        .iter()
        .filter(|m| m.as_str() == "Async.pool.eject" || m.starts_with("pool.set_"))
        .cloned()
        .collect()
}
#[tokio::test]
async fn dissolve_standalone_and_multiple_members_preserves_coordinator_and_clears_metadata_last() {
    for (members, description) in [(0, false), (0, true), (2, true)] {
        let op = operation(members, description);
        let operation = OperationId::new();
        let directory = tempfile::tempdir().unwrap();
        let s = server(&op, "", directory.path(), operation).await;
        let (handle, request) = pending(op, operation);
        run(&s, directory.path(), request).await;
        let result = handle.latest();
        assert_eq!(result.state, OperationState::Succeeded, "{}", result.detail);
        let mut expected = vec!["Async.pool.eject".to_owned(); members];
        expected.push("pool.set_name_label".into());
        if description {
            expected.push("pool.set_name_description".into());
        }
        assert_eq!(writes(&s), expected);
        assert_eq!(
            result.pool_dissolve.as_ref().unwrap().ejected.len(),
            members
        );
        assert!(s.records.lock().unwrap().contains_key("OpaqueRef:host2"));
        let public = serde_json::to_string(result.as_ref()).unwrap();
        assert!(!public.contains("pool_dissolve_plan"));
        assert!(!public.contains("OpaqueRef:"));
    }
}
#[tokio::test]
async fn dissolve_checks_every_member_before_ejecting_any_and_stops_if_next_changes() {
    for fault in [
        "initial-enabled",
        "initial-resident",
        "initial-disk",
        "changed-next",
        "extra-host",
        "coordinator",
        "ha",
    ] {
        let op = operation(2, true);
        let operation = OperationId::new();
        let directory = tempfile::tempdir().unwrap();
        let s = server(&op, fault, directory.path(), operation).await;
        {
            let mut d = s.records.lock().unwrap();
            match fault {
                "initial-enabled" => d.get_mut("OpaqueRef:host4").unwrap()["enabled"] = json!(true),
                "initial-resident" => {
                    d.insert(
                        "OpaqueRef:VM1".into(),
                        json!({"is_control_domain":false,"resident_on":"OpaqueRef:host4"}),
                    );
                }
                "initial-disk" => {
                    d.insert(
                        "OpaqueRef:PBD1".into(),
                        json!({"host":"OpaqueRef:host4","SR":"OpaqueRef:SR1"}),
                    );
                    d.insert(
                        "OpaqueRef:SR1".into(),
                        json!({"shared":false,"content_type":"user","VDIs":["OpaqueRef:VDI1"]}),
                    );
                }
                _ => {}
            }
        }
        let (handle, request) = pending(op, operation);
        run(&s, directory.path(), request).await;
        assert_ne!(handle.latest().state, OperationState::Succeeded, "{fault}");
        assert_eq!(
            writes(&s).len(),
            usize::from(!fault.starts_with("initial")),
            "{fault}: {}",
            handle.latest().detail
        );
    }
}
#[tokio::test]
async fn dissolve_lost_replies_and_failed_tasks_never_infer_completion_or_replay() {
    for fault in ["lost-eject", "lost-name", "task-failure"] {
        let op = operation(1, true);
        let operation = OperationId::new();
        let directory = tempfile::tempdir().unwrap();
        let s = server(&op, fault, directory.path(), operation).await;
        let (handle, request) = pending(op, operation);
        run(&s, directory.path(), request).await;
        assert_eq!(
            handle.latest().state,
            OperationState::OutcomeUnknown,
            "{fault}: {}",
            handle.latest().detail
        );
        let before = writes(&s);
        let result = recheck(&s, directory.path(), (*handle.latest()).clone()).await;
        assert_eq!(
            result.state,
            OperationState::OutcomeUnknown,
            "{fault}: {}",
            result.detail
        );
        assert_eq!(writes(&s), before);
        assert!(
            Journal::open(directory.path())
                .unwrap()
                .resource_pool_busy(id(1).into())
                .unwrap()
        );
    }
}
#[tokio::test]
async fn dissolve_late_ejection_receipts_preserve_recovery_ack_and_stop_pending_members() {
    for fault in ["recover-eject", "ack-eject", "stop"] {
        let op = operation(2, true);
        let operation = OperationId::new();
        let directory = tempfile::tempdir().unwrap();
        let s = server(&op, fault, directory.path(), operation).await;
        let (handle, request) = pending(op, operation);
        run(&s, directory.path(), request).await;
        let record = (*handle.latest()).clone();
        assert_eq!(writes(&s), vec!["Async.pool.eject"]);
        assert!(
            record.pool_dissolve_plan.as_ref().unwrap().evidence[0]
                .task
                .is_some()
        );
        if fault == "ack-eject" {
            assert_eq!(record.state, OperationState::AcknowledgedUnknown);
            continue;
        }
        let before = s.calls.lock().unwrap().len();
        let result = recheck(&s, directory.path(), record).await;
        assert_eq!(
            result.state,
            OperationState::PartiallyCompleted,
            "{fault}: {}",
            result.detail
        );
        assert_eq!(
            result.pool_dissolve.as_ref().unwrap().ejected,
            vec![id(3).into()]
        );
        assert_eq!(writes(&s), vec!["Async.pool.eject"]);
        assert!(
            !s.calls.lock().unwrap()[before..]
                .iter()
                .any(|m| m == "task.cancel" || m == "task.destroy")
        );
    }
}
#[tokio::test]
async fn dissolve_journal_rejects_rebound_or_retracted_completion_proof() {
    let op = operation(1, true);
    let operation = OperationId::new();
    let directory = tempfile::tempdir().unwrap();
    let s = server(&op, "", directory.path(), operation).await;
    let (handle, request) = pending(op, operation);
    run(&s, directory.path(), request).await;
    assert_eq!(
        handle.latest().state,
        OperationState::Succeeded,
        "{}",
        handle.latest().detail
    );
    let saved = (*handle.latest()).clone();
    let j = Journal::open(directory.path()).unwrap();
    for fault in 0..6 {
        let mut altered = saved.clone();
        match fault {
            0 => altered.pool_dissolve_plan.as_mut().unwrap().evidence[0].task = None,
            1 => altered.pool_dissolve_plan.as_mut().unwrap().evidence[1].completed = false,
            2 => altered.pool_dissolve.as_mut().unwrap().ejected.clear(),
            3 => {
                altered
                    .pool_dissolve_plan
                    .as_mut()
                    .unwrap()
                    .operation
                    .coordinator = id(99).into()
            }
            4 => {
                altered.pool_dissolve_plan.as_mut().unwrap().evidence[0].task_uuid =
                    Some(id(88).into())
            }
            _ => altered.pool_dissolve_plan.as_mut().unwrap().evidence[0].sent = false,
        }
        assert!(j.resource_save(&mut altered).is_err(), "{fault}");
    }
    let mut json = serde_json::to_value(&saved.pool_dissolve_plan).unwrap();
    json["evidence"][0]["task_uuid"] = Value::Null;
    j.0.execute("UPDATE resource_operations SET payload=json_set(payload,'$.pool_dissolve_plan',json(?1)) WHERE id=?2",rusqlite::params![json.to_string(),operation.to_string()]).unwrap();
    assert!(j.resource_load(operation).is_err());
}

#[tokio::test]
async fn dissolve_pending_rechecks_reserve_pool_and_refuse_rebound_task_identity() {
    for rebound in [false, true] {
        let op = operation(2, true);
        let operation = OperationId::new();
        let directory = tempfile::tempdir().unwrap();
        let s = server(&op, "recover-pending", directory.path(), operation).await;
        let (handle, request) = pending(op, operation);
        run(&s, directory.path(), request).await;
        let record = recheck(&s, directory.path(), (*handle.latest()).clone()).await;
        assert_eq!(
            record.state,
            OperationState::OutcomeUnknown,
            "{}",
            record.detail
        );
        assert!(
            Journal::open(directory.path())
                .unwrap()
                .resource_pool_busy(id(1).into())
                .unwrap()
        );
        assert_eq!(
            record.pool_dissolve_plan.as_ref().unwrap().evidence[0].task_uuid,
            Some(id(100).into())
        );
        {
            let mut data = s.records.lock().unwrap();
            let task = data.get_mut("OpaqueRef:task100").unwrap();
            task["status"] = json!("success");
            if rebound {
                task["uuid"] = json!(id(900));
            }
        }
        let calls_before = s.calls.lock().unwrap().len();
        let result = recheck(&s, directory.path(), record).await;
        assert_eq!(
            result.state,
            if rebound {
                OperationState::OutcomeUnknown
            } else {
                OperationState::PartiallyCompleted
            },
            "{}",
            result.detail
        );
        assert_eq!(writes(&s), vec!["Async.pool.eject"]);
        assert!(
            !s.calls.lock().unwrap()[calls_before..]
                .iter()
                .any(|m| m == "task.destroy" || m == "task.cancel")
        );
        assert_eq!(
            Journal::open(directory.path())
                .unwrap()
                .resource_pool_busy(id(1).into())
                .unwrap(),
            rebound
        );
    }
}
#[tokio::test]
async fn dissolve_preexisting_and_reused_task_ids_never_establish_ejection_proof() {
    for fault in ["old-task", "reused-task"] {
        let op = operation(2, true);
        let operation = OperationId::new();
        let directory = tempfile::tempdir().unwrap();
        let s = server(&op, fault, directory.path(), operation).await;
        if fault == "old-task" {
            s.records.lock().unwrap().insert(
                "OpaqueRef:task100".into(),
                json!({"uuid":id(100),"status":"success"}),
            );
        }
        let (handle, request) = pending(op, operation);
        run(&s, directory.path(), request).await;
        assert_eq!(
            handle.latest().state,
            OperationState::OutcomeUnknown,
            "{}",
            handle.latest().detail
        );
        assert!(!writes(&s).iter().any(|m| m.starts_with("pool.set_")));
        assert!(!s.calls.lock().unwrap().iter().any(|m| m == "task.destroy"));
        assert_eq!(
            handle
                .latest()
                .pool_dissolve
                .as_ref()
                .unwrap()
                .ejected
                .len(),
            usize::from(fault == "reused-task")
        );
    }
}
#[tokio::test]
async fn dissolve_late_name_receipt_cannot_resume_description_after_recovery_or_ack() {
    for fault in ["recover-name", "ack-name"] {
        let op = operation(0, true);
        let operation = OperationId::new();
        let directory = tempfile::tempdir().unwrap();
        let s = server(&op, fault, directory.path(), operation).await;
        let (handle, request) = pending(op, operation);
        run(&s, directory.path(), request).await;
        let record = (*handle.latest()).clone();
        assert!(record.pool_dissolve_plan.as_ref().unwrap().evidence[0].completed);
        if fault == "ack-name" {
            assert_eq!(record.state, OperationState::AcknowledgedUnknown);
        } else {
            let result = recheck(&s, directory.path(), record).await;
            assert_eq!(
                result.state,
                OperationState::PartiallyCompleted,
                "{}",
                result.detail
            );
            assert!(result.pool_dissolve.unwrap().name_cleared);
        }
        assert_eq!(writes(&s), vec!["pool.set_name_label"]);
    }
}
#[tokio::test]
async fn dissolve_denied_final_setter_prevents_every_ejection() {
    let op = operation(2, true);
    let operation = OperationId::new();
    let directory = tempfile::tempdir().unwrap();
    let s = server(&op, "denied-final-setter", directory.path(), operation).await;
    let (handle, request) = pending(op, operation);
    run(&s, directory.path(), request).await;
    assert_eq!(
        handle.latest().state,
        OperationState::Rejected,
        "{}",
        handle.latest().detail
    );
    assert!(writes(&s).is_empty());
}

#[tokio::test]
async fn dissolve_rechecks_final_pool_even_when_every_step_was_previously_verified() {
    for fault in ["", "extra-host", "coordinator", "name", "description", "ha"] {
        let op = operation(1, true);
        let operation = OperationId::new();
        let directory = tempfile::tempdir().unwrap();
        let s = server(&op, "", directory.path(), operation).await;
        let (handle, request) = pending(op, operation);
        run(&s, directory.path(), request).await;
        assert_eq!(handle.latest().state, OperationState::Succeeded);
        let j = Journal::open(directory.path()).unwrap();
        // Reconstruct a crash after all verification receipts, before the terminal save.
        j.0.execute("UPDATE resource_operations SET terminal=0,payload=json_set(payload,'$.state',json(?1)) WHERE id=?2",rusqlite::params![serde_json::to_string(&OperationState::Preparing).unwrap(),operation.to_string()]).unwrap();
        j.resource_recover(id(1).into()).unwrap();
        let record = j.resource_load(operation).unwrap().unwrap();
        assert!(
            record
                .steps
                .iter()
                .all(|s| s.state == crate::StepState::Verified)
        );
        {
            let mut data = s.records.lock().unwrap();
            match fault {
                "extra-host" => {
                    data.insert("OpaqueRef:host9".into(), json!({"uuid":id(9)}));
                }
                "coordinator" => {
                    data.get_mut("OpaqueRef:pool").unwrap()["master"] = json!("OpaqueRef:other")
                }
                "name" => data.get_mut("OpaqueRef:pool").unwrap()["name_label"] = json!("New pool"),
                "description" => {
                    data.get_mut("OpaqueRef:pool").unwrap()["name_description"] = json!("Changed")
                }
                "ha" => data.get_mut("OpaqueRef:pool").unwrap()["ha_enabled"] = json!(true),
                _ => {}
            }
        }
        let before = s.calls.lock().unwrap().len();
        let result = recheck(&s, directory.path(), record).await;
        assert_eq!(
            result.state,
            if fault.is_empty() {
                OperationState::Succeeded
            } else {
                OperationState::OutcomeUnknown
            },
            "{fault}: {}",
            result.detail
        );
        let calls = s.calls.lock().unwrap();
        assert!(!calls[before..].iter().any(|m| m.starts_with("Async.")
            || m.starts_with("pool.set_")
            || m == "task.destroy"
            || m == "task.cancel"));
        assert!(calls[before..].iter().any(|m| m == "pool.get_record"));
    }
}

#[tokio::test]
async fn dissolve_full_update_queue_never_defers_a_claimed_write_until_after_recovery() {
    for after_dispatch in [false, true] {
        let op = operation(2, true);
        let operation = OperationId::new();
        let directory = tempfile::tempdir().unwrap();
        let s = server(&op, "", directory.path(), operation).await;
        let (handle, request) = pending(op, operation);
        let session = Arc::new(s.fixture.session().await);
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        let worker = tokio::spawn(request.run(
            session,
            directory.path().into(),
            tokio_util::sync::CancellationToken::new(),
            tx,
        ));
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while handle.latest().steps.is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        // Preparation publication is blocked behind the first update. The dispatch
        // publication must not introduce a second wait between its CAS and the RPC.
        if after_dispatch {
            rx.recv().await.unwrap();
            tokio::time::timeout(std::time::Duration::from_secs(2), async {
                while writes(&s).is_empty() {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
        }
        Journal::open(directory.path())
            .unwrap()
            .resource_recover(id(1).into())
            .unwrap();
        drop(rx);
        tokio::time::timeout(std::time::Duration::from_secs(2), worker)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(writes(&s).len(), usize::from(after_dispatch));
        assert_eq!(
            handle.latest().state,
            if after_dispatch {
                OperationState::OutcomeUnknown
            } else {
                OperationState::Cancelled
            }
        );
    }
}
