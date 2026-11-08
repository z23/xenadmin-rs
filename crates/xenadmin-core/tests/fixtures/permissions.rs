use super::*;
use xenadmin_core::{OperationId, OperationState, PermissionDecision, PowerAction, VmCommand};

fn permission_record(start: bool) -> Value {
    let mut permissions = vec!["session.logout", "task.get_record", "task.destroy"];
    if start {
        permissions.push("vm.start");
    }
    json!({"is_local_superuser":false,"rbac_permissions":permissions})
}
async fn ready(server: &Mock) -> (tempfile::TempDir, Backend, xenadmin_core::Connection) {
    let directory = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(directory.path().into()).unwrap();
    let connection = backend.connect(
        ConnectionOptions::new(server.endpoint.clone()),
        Credentials::new("fixture-user".into(), "private-fixture-password".into()),
    );
    connection.wait_ready(Duration::from_secs(4)).await.unwrap();
    (directory, backend, connection)
}
async fn start(connection: &xenadmin_core::Connection) -> xenadmin_core::Operation {
    let snapshot = connection.latest();
    let vm = snapshot.inventory().unwrap().guests().next().unwrap().id;
    connection
        .submit(
            OperationId::new(),
            VmCommand::Power {
                vm: snapshot.observe_vm(vm).unwrap(),
                action: PowerAction::Start,
            },
        )
        .unwrap()
}
#[tokio::test]
async fn known_session_denial_stops_power_before_dispatch_and_is_journaled() {
    let server = mock_overridden(override_rpc(|request| async move {
        (request["method"] == "session.get_record")
            .then(|| RpcReply::Json(json!({"result":permission_record(false)})))
    }))
    .await;
    let (directory, _backend, connection) = ready(&server).await;
    assert_eq!(
        connection.latest().power_permission(PowerAction::Start),
        PermissionDecision::Denied
    );
    let operation = start(&connection).await;
    assert_eq!(finish(&operation).await, OperationState::Failed);
    assert_eq!(server.mutations.load(Ordering::SeqCst), 0);
    assert!(operation.latest().detail.contains("RBAC_PERMISSION_DENIED"));
    assert!(!journal_text(directory.path()).contains("fixture-session"));
    connection.close().await;
}
#[tokio::test]
async fn known_grant_is_advisory_and_server_refusal_is_preserved() {
    let server=mock_overridden(override_rpc(|request|async move {
        match request["method"].as_str().unwrap(){
            "session.get_record"=>Some(RpcReply::Json(json!({"result":permission_record(true)}))),
            "Async.VM.start"=>Some(RpcReply::Json(json!({"error":{"message":"RBAC_PERMISSION_DENIED","data":["private-server-context"]}}))),
            _=>None
        }
    })).await;
    let (_directory, _backend, connection) = ready(&server).await;
    assert_eq!(
        connection.latest().power_permission(PowerAction::Start),
        PermissionDecision::Granted
    );
    let operation = start(&connection).await;
    assert_eq!(finish(&operation).await, OperationState::Failed);
    assert_eq!(
        server
            .methods
            .lock()
            .unwrap()
            .iter()
            .filter(|m| *m == "Async.VM.start")
            .count(),
        1
    );
    assert!(!operation.latest().detail.contains("private-server-context"));
    connection.close().await;
}
#[tokio::test]
async fn unsupported_denied_or_malformed_discovery_defers_to_server() {
    for response in [
        json!({"error":{"message":"MESSAGE_METHOD_UNKNOWN","data":[]}}),
        json!({"error":{"message":"RBAC_PERMISSION_DENIED","data":[]}}),
        json!({"result":{"is_local_superuser":false}}),
    ] {
        let server = mock_overridden(override_rpc(move |request| {
            let response = response.clone();
            async move {
                (request["method"] == "session.get_record").then_some(RpcReply::Json(response))
            }
        }))
        .await;
        let (_directory, _backend, connection) = ready(&server).await;
        assert!(matches!(
            connection.latest().power_permission(PowerAction::Start),
            PermissionDecision::Unknown(_)
        ));
        assert_eq!(
            finish(&start(&connection).await).await,
            OperationState::Succeeded
        );
        assert_eq!(server.mutations.load(Ordering::SeqCst), 1);
        connection.close().await;
    }
}
#[tokio::test]
async fn session_renewal_discards_previous_permission_denial() {
    let reads = Arc::new(AtomicUsize::new(0));
    let counter = reads.clone();
    let server=mock_with_overrides(true,false,None,false,Some(override_rpc(move|request|{
        let counter=counter.clone();async move{
            (request["method"]=="session.get_record").then(||RpcReply::Json(json!({"result":permission_record(counter.fetch_add(1,Ordering::SeqCst)>0)})))
        }
    }))).await;
    let (_directory, _backend, connection) = ready(&server).await;
    let first = connection.latest();
    assert_eq!(
        first.power_permission(PowerAction::Start),
        PermissionDecision::Denied
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let now = connection.latest();
            if now.is_current() && now.generation() != first.generation() {
                assert_eq!(
                    now.power_permission(PowerAction::Start),
                    PermissionDecision::Granted
                );
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(reads.load(Ordering::SeqCst), 2);
    connection.close().await;
}
#[tokio::test]
async fn expired_session_during_discovery_renews_instead_of_becoming_unknown() {
    let reads = Arc::new(AtomicUsize::new(0));
    let counter = reads.clone();
    let server = mock_overridden(override_rpc(move |request| {
        let counter = counter.clone();
        async move {
            (request["method"] == "session.get_record").then(|| {
                RpcReply::Json(if counter.fetch_add(1, Ordering::SeqCst) == 0 {
                    json!({"error":{"message":"SESSION_INVALID","data":[]}})
                } else {
                    json!({"result":permission_record(true)})
                })
            })
        }
    }))
    .await;
    let (_directory, _backend, connection) = ready(&server).await;
    assert_eq!(
        connection.latest().power_permission(PowerAction::Start),
        PermissionDecision::Granted
    );
    assert_eq!(server.logins.load(Ordering::SeqCst), 2);
    connection.close().await;
}
