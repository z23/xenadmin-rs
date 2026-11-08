use super::*;
use xenadmin_core::{
    MessageId, OperationControl, OperationId, OperationState, ResourceCommand, ResourceOperation,
    ResourceTarget, TaskId,
};
const TASK: &str = "77777777-7777-7777-7777-777777777777";
const MESSAGE: &str = "88888888-8888-8888-8888-888888888888";
#[derive(Clone, Copy, PartialEq)]
enum Fault {
    None,
    DropCancel,
    DropDismiss,
    ChangedMessage,
    RevokedCancel,
    WrongUuid,
}
type Records = Arc<Mutex<(Value, Option<Value>)>>;
async fn fixture(fault: Fault, after: &'static str) -> (Mock, Records) {
    let records: Records = Arc::new(Mutex::new((
        json!({"uuid":TASK,"name_label":"Fixture task","name_description":"Transfer","status":"pending","progress":"0.2","resident_on":"OpaqueRef:host","type":"VM","allowed_operations":["cancel"]}),
        Some(
            json!({"uuid":MESSAGE,"name":"Fixture message","priority":"2","cls":"VM","obj_uuid":"33333333-3333-3333-3333-333333333333","timestamp":"20261108T00:00:00Z","body":"Fixture alert body"}),
        ),
    )));
    let state = records.clone();
    let mock=mock_overridden(override_rpc(move |request:Value|{let state=state.clone();async move{
        let method=request["method"].as_str().unwrap();
        if method=="Async.VM.start" {return Some(RpcReply::Json(json!({"result":"OpaqueRef:selected-task"})));}
        if method=="event.from"&&request["params"][2]=="" {
            let mut initial=batch("initial","Halted");let mut data=state.lock().unwrap();
            initial["events"].as_array_mut().unwrap().push(json!({"class":"task","operation":"add","ref":"OpaqueRef:selected-task","snapshot":data.0}));
            if let Some(message)=&data.1{initial["events"].as_array_mut().unwrap().push(json!({"class":"message","operation":"add","ref":"OpaqueRef:selected-message","snapshot":message}));}
            if fault==Fault::ChangedMessage{data.1.as_mut().unwrap()["body"]=json!("Changed after review");}
            if fault==Fault::RevokedCancel{data.0["allowed_operations"]=json!([]);}
            if fault==Fault::WrongUuid{data.0["uuid"]=json!(MESSAGE);}
            return Some(RpcReply::Json(json!({"result":initial})));
        }
        if method=="task.get_by_uuid"{return Some(RpcReply::Json(json!({"result":"OpaqueRef:selected-task"})));}
        if method=="message.get_by_uuid"{return Some(RpcReply::Json(if state.lock().unwrap().1.is_some(){json!({"result":"OpaqueRef:selected-message"})}else{json!({"error":{"code":1,"message":"UUID_INVALID","data":[]}})}));}
        if method=="task.get_record"&&request["params"][1]=="OpaqueRef:selected-task"{return Some(RpcReply::Json(json!({"result":state.lock().unwrap().0})));}
        if method=="message.get_record"{return Some(RpcReply::Json(json!({"result":state.lock().unwrap().1})));}
        if method=="task.cancel"{state.lock().unwrap().0["status"]=json!(after);return Some(if fault==Fault::DropCancel{RpcReply::Drop}else{RpcReply::Json(json!({"result":null}))});}
        if method=="message.destroy"{state.lock().unwrap().1=None;return Some(if fault==Fault::DropDismiss{RpcReply::Drop}else{RpcReply::Json(json!({"result":null}))});}
        None
    }})).await;
    (mock, records)
}
async fn connect(mock: &Mock, backend: &Backend, read_only: bool) -> xenadmin_core::Connection {
    let mut options = ConnectionOptions::new(mock.endpoint.clone());
    options.read_only = read_only;
    let connection = backend.connect(
        options,
        Credentials::new("root".into(), "fixture-password".into()),
    );
    connection.wait_ready(Duration::from_secs(3)).await.unwrap();
    let snapshot = connection.latest();
    let id = TaskId::parse(TASK).unwrap();
    let task = &snapshot.inventory().unwrap().details.tasks[&id];
    assert_eq!(
        task.resident_host.unwrap().to_string(),
        "22222222-2222-2222-2222-222222222222"
    );
    assert_eq!(task.parent, None);
    assert!(task.subtasks.is_empty());
    assert_eq!(task.result_type.as_deref(), Some("VM"));
    connection
}
fn task_command(c: &xenadmin_core::Connection) -> ResourceCommand {
    ResourceCommand::CancelTask {
        task: c
            .latest()
            .observe_resource(ResourceTarget::Task(TaskId::parse(TASK).unwrap()))
            .unwrap(),
    }
}
fn message_command(c: &xenadmin_core::Connection) -> ResourceCommand {
    let snapshot = c.latest();
    let message = snapshot
        .inventory()
        .unwrap()
        .details
        .messages
        .get(&MessageId::parse(MESSAGE).unwrap())
        .unwrap();
    ResourceCommand::DismissMessage {
        message: snapshot
            .observe_resource(ResourceTarget::Message(message.id))
            .unwrap(),
        expected_digest: message.review_digest(),
    }
}
async fn finish(operation: &ResourceOperation) -> Arc<xenadmin_core::ResourceOperationRecord> {
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            let row = operation.latest();
            if row.state.terminal() || row.state == OperationState::OutcomeUnknown {
                return row;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}
async fn recheck(
    c: &xenadmin_core::Connection,
    id: OperationId,
) -> Arc<Vec<xenadmin_core::ResourceOperationRecord>> {
    let revision = c
        .resource_history()
        .iter()
        .find(|r| r.id == id)
        .unwrap()
        .revision();
    c.control_resource(id, OperationControl::Recheck).unwrap();
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            let rows = c.resource_history();
            if rows.iter().any(|r| {
                r.id == id
                    && r.revision() > revision
                    && (r.state.terminal() || r.state == OperationState::OutcomeUnknown)
            }) {
                return rows;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}
#[tokio::test]
async fn cancellation_reports_actual_terminal_status_and_never_destroys_the_original_task() {
    for (status, expected) in [
        ("cancelled", OperationState::Succeeded),
        ("success", OperationState::PartiallyCompleted),
        ("failure", OperationState::PartiallyCompleted),
    ] {
        let (mock, _) = fixture(Fault::None, status).await;
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&mock, &backend, false).await;
        let op = c
            .submit_resource(OperationId::new(), task_command(&c))
            .unwrap();
        assert_eq!(finish(&op).await.state, expected);
        c.close().await;
        let methods = mock.methods.lock().unwrap();
        assert_eq!(
            methods
                .iter()
                .filter(|m| m.as_str() == "task.cancel")
                .count(),
            1
        );
        assert!(!methods.iter().any(|m| m == "task.destroy"));
    }
}
#[tokio::test]
async fn accepted_request_is_not_completion_and_duplicate_task_cancellation_is_rejected() {
    let (mock, data) = fixture(Fault::None, "pending").await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&mock, &backend, false).await;
    let op = c
        .submit_resource(OperationId::new(), task_command(&c))
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while op.latest().state != OperationState::CancellationRequested {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(!op.latest().state.terminal());
    let duplicate = c
        .submit_resource(OperationId::new(), task_command(&c))
        .unwrap();
    assert_eq!(finish(&duplicate).await.state, OperationState::Rejected);
    data.lock().unwrap().0["status"] = json!("cancelling");
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert!(op.latest().detail.contains("cancelling"));
    assert!(!op.latest().state.terminal());
    data.lock().unwrap().0["status"] = json!("cancelled");
    assert_eq!(finish(&op).await.state, OperationState::Succeeded);
    c.close().await;
}
#[tokio::test]
async fn lost_cancel_response_remains_unknown_until_read_evidence_and_never_replays() {
    let (mock, data) = fixture(Fault::DropCancel, "pending").await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&mock, &backend, false).await;
    let id = OperationId::new();
    let op = c.submit_resource(id, task_command(&c)).unwrap();
    assert_eq!(finish(&op).await.state, OperationState::OutcomeUnknown);
    let rows = recheck(&c, id).await;
    assert_eq!(
        rows.iter().find(|r| r.id == id).unwrap().state,
        OperationState::OutcomeUnknown
    );
    data.lock().unwrap().0["status"] = json!("cancelled");
    let rows = recheck(&c, id).await;
    assert_eq!(
        rows.iter().find(|r| r.id == id).unwrap().state,
        OperationState::Succeeded
    );
    assert_eq!(
        mock.methods
            .lock()
            .unwrap()
            .iter()
            .filter(|m| m.as_str() == "task.cancel")
            .count(),
        1
    );
    c.close().await;
}
#[tokio::test]
async fn fresh_identity_capability_and_review_content_prevent_wrong_dispatch() {
    for fault in [
        Fault::RevokedCancel,
        Fault::WrongUuid,
        Fault::ChangedMessage,
    ] {
        let (mock, _) = fixture(fault, "cancelled").await;
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&mock, &backend, false).await;
        let command = if fault == Fault::ChangedMessage {
            message_command(&c)
        } else {
            task_command(&c)
        };
        let op = c.submit_resource(OperationId::new(), command).unwrap();
        assert_eq!(finish(&op).await.state, OperationState::Rejected);
        c.close().await;
        assert!(
            !mock
                .methods
                .lock()
                .unwrap()
                .iter()
                .any(|m| m == "task.cancel" || m == "message.destroy")
        );
    }
}
#[tokio::test]
async fn message_dismissal_verifies_absence_and_lost_reply_recheck_does_not_repeat_destroy() {
    for fault in [Fault::None, Fault::DropDismiss] {
        let (mock, _) = fixture(fault, "cancelled").await;
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&mock, &backend, false).await;
        let id = OperationId::new();
        let op = c.submit_resource(id, message_command(&c)).unwrap();
        let row = finish(&op).await;
        if fault == Fault::DropDismiss {
            assert_eq!(row.state, OperationState::OutcomeUnknown);
            let rows = recheck(&c, id).await;
            assert_eq!(
                rows.iter().find(|r| r.id == id).unwrap().state,
                OperationState::Succeeded
            );
        } else {
            assert_eq!(row.state, OperationState::Succeeded);
        }
        let payload = std::fs::read(dir.path().join("operations.sqlite3")).unwrap();
        assert!(!String::from_utf8_lossy(&payload).contains("Fixture alert body"));
        c.close().await;
        assert_eq!(
            mock.methods
                .lock()
                .unwrap()
                .iter()
                .filter(|m| m.as_str() == "message.destroy")
                .count(),
            1
        );
    }
}
#[tokio::test]
async fn read_only_and_old_generation_reject_activity_mutations() {
    let (mock, _) = fixture(Fault::None, "cancelled").await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&mock, &backend, true).await;
    assert!(matches!(
        c.submit_resource(OperationId::new(), task_command(&c)),
        Err(xenadmin_core::Error::ReadOnly)
    ));
    assert!(matches!(
        c.submit_resource(OperationId::new(), message_command(&c)),
        Err(xenadmin_core::Error::ReadOnly)
    ));
    c.close().await;
    let c = connect(&mock, &backend, false).await;
    let command = task_command(&c);
    c.close().await;
    let c = connect(&mock, &backend, false).await;
    assert!(matches!(
        c.submit_resource(OperationId::new(), command),
        Err(xenadmin_core::Error::Stale)
    ));
    c.close().await;
}

#[tokio::test]
async fn external_cancellation_keeps_local_task_owner_reconciling_its_original_operation() {
    let (mock, _) = fixture(Fault::None, "cancelled").await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&mock, &backend, false).await;
    let vm = c.latest().inventory().unwrap().guests().next().unwrap().id;
    let original = c
        .submit(
            OperationId::new(),
            xenadmin_core::VmCommand::Power {
                vm: c.latest().observe_vm(vm).unwrap(),
                action: xenadmin_core::PowerAction::Start,
            },
        )
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while original.latest().state != OperationState::Tracking {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let cancellation = c
        .submit_resource(OperationId::new(), task_command(&c))
        .unwrap();
    assert_eq!(finish(&cancellation).await.state, OperationState::Succeeded);
    tokio::time::timeout(Duration::from_secs(4), async {
        while !original.latest().state.terminal() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(original.latest().state, OperationState::Cancelled);
    assert!(c.operation_history().iter().any(
        |record| record.id == original.latest().id && record.state == OperationState::Cancelled
    ));
    c.close().await;
    let methods = mock.methods.lock().unwrap();
    assert_eq!(
        methods
            .iter()
            .filter(|method| method.as_str() == "task.cancel")
            .count(),
        1
    );
    assert_eq!(
        methods
            .iter()
            .filter(|method| method.as_str() == "task.destroy")
            .count(),
        1
    ); // Original owner only.
}
#[tokio::test]
async fn restart_recovers_cancellation_by_reading_status_without_resubmission() {
    let (mock, data) = fixture(Fault::DropCancel, "pending").await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&mock, &backend, false).await;
    let id = OperationId::new();
    let op = c.submit_resource(id, task_command(&c)).unwrap();
    assert_eq!(finish(&op).await.state, OperationState::OutcomeUnknown);
    c.close().await;
    drop(c);
    drop(backend);
    data.lock().unwrap().0["status"] = json!("cancelled");
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&mock, &backend, false).await;
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            if c.resource_history()
                .iter()
                .any(|r| r.id == id && r.state == OperationState::Succeeded)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    c.close().await;
    assert_eq!(
        mock.methods
            .lock()
            .unwrap()
            .iter()
            .filter(|m| m.as_str() == "task.cancel")
            .count(),
        1
    );
}
