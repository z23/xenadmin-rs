use super::*;
use xenadmin_core::{
    Connection, HostId, OperationControl, OperationId, OperationState, PoolId, PoolJoinSelection,
    ResourceCommand, ResourceOperation, ResourceOperationRecord, ResourceTarget,
};
const SOURCE: &str = "11111111-1111-1111-1111-111111111111";
const DEST: &str = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa";
const SOURCE_HOST: &str = "22222222-2222-2222-2222-222222222222";
const DEST_HOST: &str = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb";
#[derive(Clone, Copy, PartialEq, Debug)]
enum Fault {
    None,
    DropReply,
    WrongAuthPool,
    WrongAuthCoordinator,
    AuthRedirect,
    Unprivileged,
    SourceHa,
    DestinationHa,
    RunningGuest,
    SharedStorage,
    Bond,
    VerifyFailure,
    DelayAuth,
    DelayJoin,
}
struct Control {
    fault: Fault,
    joined: bool,
}
struct Fixture {
    source: Mock,
    destination: Mock,
    control: Arc<Mutex<Control>>,
    entered: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
}
fn pool_record(source: bool) -> Value {
    json!({"uuid":if source{SOURCE}else{DEST},"name_label":if source{"Standalone"}else{"Destination"},"master":if source{"OpaqueRef:source-host"}else{"OpaqueRef:dest-host"},"current_operations":{},"ha_enabled":false})
}
fn host_record(source: bool) -> Value {
    json!({"uuid":if source{SOURCE_HOST}else{DEST_HOST},"name_label":if source{"Join host"}else{"Coordinator"},"address":if source{"192.0.2.10"}else{"192.0.2.20"},"enabled":true,"metrics":"OpaqueRef:metrics","current_operations":{}})
}
async fn fixture(fault: Fault) -> Fixture {
    let control = Arc::new(Mutex::new(Control {
        fault,
        joined: false,
    }));
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let mut mocks = vec![];
    for source in [true, false] {
        let c = control.clone();
        let entered = entered.clone();
        let release = release.clone();
        mocks.push(mock_overridden(override_rpc(move|r|{let c=c.clone();let entered=entered.clone();let release=release.clone();async move{
            let method=r["method"].as_str().unwrap();let p=&r["params"];
            let (fault,joined)={let c=c.lock().unwrap();(c.fault,c.joined)};
            let auth=p[0]=="OpaqueRef:join-session";
            let result=match method {
                "session.login_with_password" if p[0]=="join-admin"=>{
                    assert!(!source);assert_eq!(p[1],"join-private-password");
                    if fault==Fault::DelayAuth {entered.notify_one();release.notified().await;}
                    if fault==Fault::AuthRedirect {return Some(RpcReply::Json(json!({"error":{"message":"HOST_IS_SLAVE","data":["192.0.2.99"]}})));}
                    json!("OpaqueRef:join-session")
                },
                "event.from" if p[2]==""=>json!({"token":"initial","events":[
                    {"class":"pool","operation":"add","ref":"OpaqueRef:pool","snapshot":pool_record(source)},
                    {"class":"host","operation":"add","ref":if source{"OpaqueRef:source-host"}else{"OpaqueRef:dest-host"},"snapshot":host_record(source)}
                ]}),
                "session.get_record"=>json!({"is_local_superuser":fault!=Fault::Unprivileged}),
                "host.list_methods"=>json!(["pool.join"]),
                "pool.get_all_records"=>{
                    let mut record=pool_record(source);
                    if !source && auth && fault==Fault::WrongAuthPool {record["uuid"]=json!(SOURCE);}
                    if (source&&fault==Fault::SourceHa)||(!source&&fault==Fault::DestinationHa){record["ha_enabled"]=json!(true);}
                    json!({"OpaqueRef:pool":record})
                },
                "host.get_all_records"=>{
                    if !source && joined && fault==Fault::VerifyFailure {return Some(RpcReply::Json(json!({"error":{"message":"HOST_OFFLINE","data":["join-private-password"]}})));}
                    let mut records=serde_json::Map::new();let mut record=host_record(source);
                    if !source && auth && fault==Fault::WrongAuthCoordinator {record["uuid"]=json!(SOURCE_HOST);}
                    records.insert(if source{"OpaqueRef:source-host"}else{"OpaqueRef:dest-host"}.into(),record);
                    if !source&&joined {records.insert("OpaqueRef:source-host".into(),host_record(true));}
                    json!(records)
                },
                "host.get_by_uuid"=>{assert_eq!(p[1],SOURCE_HOST);json!("OpaqueRef:source-host")},
                "host.get_record"=>{assert_eq!(p[1],"OpaqueRef:source-host");host_record(true)},
                "host_metrics.get_record"=>json!({"live":true}),
                "VM.get_all_records"=>json!({"OpaqueRef:guest":{"is_control_domain":false,"power_state":if fault==Fault::RunningGuest{"Running"}else{"Halted"},"scheduled_to_be_resident_on":"OpaqueRef:NULL","current_operations":{}}}),
                "SR.get_all_records"=>if fault==Fault::SharedStorage{json!({"OpaqueRef:sr":{"shared":true,"is_tools_sr":false,"current_operations":{}}})}else{json!({})},
                "Bond.get_all_records"=>if fault==Fault::Bond{json!({"OpaqueRef:bond":{}})}else{json!({})},
                "tunnel.get_all_records"=>json!({}),
                "pool.join"=>{
                    assert!(source);assert_eq!(p[1],"192.0.2.20");assert_eq!(p[2],"join-admin");assert_eq!(p[3],"join-private-password");
                    if fault==Fault::DelayJoin {entered.notify_one();release.notified().await;}
                    c.lock().unwrap().joined=true;
                    if fault==Fault::DropReply {return Some(RpcReply::Drop)}
                    Value::Null
                },
                _=>return None,
            };Some(RpcReply::Json(json!({"result":result})))
        }})).await);
    }
    let destination = mocks.pop().unwrap();
    let source = mocks.pop().unwrap();
    Fixture {
        source,
        destination,
        control,
        entered,
        release,
    }
}
async fn connect(mock: &Mock, backend: &Backend, readonly: bool) -> Connection {
    let mut options = ConnectionOptions::new(mock.endpoint.clone());
    options.read_only = readonly;
    let c = backend.connect(
        options,
        Credentials::new("fixture".into(), "fixture-password".into()),
    );
    let mut w = c.watch();
    tokio::time::timeout(Duration::from_secs(4), async {
        while !w.borrow().is_current() {
            w.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    c
}
fn selection(source: &Connection, destination: &Connection) -> PoolJoinSelection {
    PoolJoinSelection {
        source: source
            .latest()
            .observe_resource(ResourceTarget::Pool(PoolId::parse(SOURCE).unwrap()))
            .unwrap(),
        source_host: HostId::parse(SOURCE_HOST).unwrap(),
        destination: destination
            .latest()
            .observe_resource(ResourceTarget::Pool(PoolId::parse(DEST).unwrap()))
            .unwrap(),
        destination_coordinator: HostId::parse(DEST_HOST).unwrap(),
        destination_address: "192.0.2.20".into(),
    }
}
fn credentials() -> Credentials {
    Credentials::new("join-admin".into(), "join-private-password".into())
}
async fn finished(op: &ResourceOperation) -> Arc<ResourceOperationRecord> {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let r = op.latest();
            if r.state.terminal() || r.state == OperationState::OutcomeUnknown {
                return r;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}
fn joins(mock: &Mock) -> usize {
    mock.methods
        .lock()
        .unwrap()
        .iter()
        .filter(|m| m.as_str() == "pool.join")
        .count()
}
fn privacy(dir: &std::path::Path, record: &ResourceOperationRecord) {
    let public = serde_json::to_string(record).unwrap();
    let db = rusqlite::Connection::open_with_flags(
        dir.join("operations.sqlite3"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let (payload, task, result): (String, Option<String>, Option<String>) = db
        .query_row(
            "SELECT payload,task,result_reference FROM resource_operations WHERE id=?1",
            [record.id.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    for text in [
        public,
        payload,
        task.unwrap_or_default(),
        result.unwrap_or_default(),
    ] {
        for secret in [
            "join-private-password",
            "join-admin",
            "OpaqueRef:join-session",
            "OpaqueRef:source-host",
            "OpaqueRef:dest-host",
        ] {
            assert!(!text.contains(secret), "{secret}");
        }
    }
}
#[tokio::test]
async fn join_verifies_destination_credentials_and_membership_without_persisting_secrets() {
    let f = fixture(Fault::None).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let source = connect(&f.source, &backend, false).await;
    let dest = connect(&f.destination, &backend, false).await;
    let op = source
        .join_pool(
            OperationId::new(),
            selection(&source, &dest),
            &dest,
            credentials(),
        )
        .unwrap();
    let result = finished(&op).await;
    assert_eq!(result.state, OperationState::Succeeded, "{}", result.detail);
    assert_eq!(joins(&f.source), 1);
    assert_eq!(joins(&f.destination), 0);
    assert!(result.applies_to_pool(PoolId::parse(SOURCE).unwrap()));
    assert!(result.applies_to_pool(PoolId::parse(DEST).unwrap()));
    assert_eq!(result.recovery_pool(), PoolId::parse(DEST).unwrap());
    privacy(dir.path(), &result);
}
#[tokio::test]
async fn join_rejects_changed_identity_privilege_ha_and_source_prerequisites_before_dispatch() {
    for fault in [
        Fault::WrongAuthPool,
        Fault::WrongAuthCoordinator,
        Fault::AuthRedirect,
        Fault::Unprivileged,
        Fault::SourceHa,
        Fault::DestinationHa,
        Fault::RunningGuest,
        Fault::SharedStorage,
        Fault::Bond,
    ] {
        let f = fixture(fault).await;
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let source = connect(&f.source, &backend, false).await;
        let dest = connect(&f.destination, &backend, false).await;
        let op = source
            .join_pool(
                OperationId::new(),
                selection(&source, &dest),
                &dest,
                credentials(),
            )
            .unwrap();
        let result = finished(&op).await;
        assert_eq!(
            result.state,
            OperationState::Rejected,
            "{fault:?}: {}",
            result.detail
        );
        assert_eq!(joins(&f.source), 0);
        privacy(dir.path(), &result);
    }
}
#[tokio::test]
async fn lost_join_reply_stays_unknown_after_destination_recheck_and_never_replays() {
    let f = fixture(Fault::DropReply).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let source = connect(&f.source, &backend, false).await;
    let dest = connect(&f.destination, &backend, false).await;
    let op = source
        .join_pool(
            OperationId::new(),
            selection(&source, &dest),
            &dest,
            credentials(),
        )
        .unwrap();
    let result = finished(&op).await;
    assert_eq!(result.state, OperationState::OutcomeUnknown);
    assert!(f.control.lock().unwrap().joined);
    dest.control_resource(result.id, OperationControl::Recheck)
        .unwrap();
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            if dest
                .resource_history()
                .iter()
                .any(|r| r.id == result.id && r.detail.contains("reply was not recorded"))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(joins(&f.source), 1);
    privacy(dir.path(), &result);
}
#[tokio::test]
async fn recorded_join_reply_can_be_rechecked_through_the_destination_connection() {
    let f = fixture(Fault::VerifyFailure).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let source = connect(&f.source, &backend, false).await;
    let dest = connect(&f.destination, &backend, false).await;
    let op = source
        .join_pool(
            OperationId::new(),
            selection(&source, &dest),
            &dest,
            credentials(),
        )
        .unwrap();
    let result = finished(&op).await;
    assert_eq!(result.state, OperationState::OutcomeUnknown);
    drop(source);
    f.control.lock().unwrap().fault = Fault::None;
    dest.control_resource(result.id, OperationControl::Recheck)
        .unwrap();
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            if dest
                .resource_history()
                .iter()
                .any(|r| r.id == result.id && r.state == OperationState::Succeeded)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(joins(&f.source), 1);
}
#[tokio::test]
async fn dropped_destination_cancels_preflight_without_forwarding_its_credentials() {
    let f = fixture(Fault::DelayAuth).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let source = connect(&f.source, &backend, false).await;
    let dest = connect(&f.destination, &backend, false).await;
    let op = source
        .join_pool(
            OperationId::new(),
            selection(&source, &dest),
            &dest,
            credentials(),
        )
        .unwrap();
    f.entered.notified().await;
    drop(dest);
    f.release.notify_one();
    let result = finished(&op).await;
    assert!(matches!(
        result.state,
        OperationState::Cancelled | OperationState::Rejected
    ));
    assert_eq!(joins(&f.source), 0);
}
#[tokio::test]
async fn disconnect_after_dispatch_is_unknown_even_if_the_host_finishes_joining() {
    let f = fixture(Fault::DelayJoin).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let source = connect(&f.source, &backend, false).await;
    let dest = connect(&f.destination, &backend, false).await;
    let op = source
        .join_pool(
            OperationId::new(),
            selection(&source, &dest),
            &dest,
            credentials(),
        )
        .unwrap();
    f.entered.notified().await;
    drop(dest);
    let result = finished(&op).await;
    assert_eq!(result.state, OperationState::OutcomeUnknown);
    f.release.notify_one();
    assert_eq!(joins(&f.source), 1);
}
#[tokio::test]
async fn join_requires_both_writable_connections_in_the_same_backend_and_transient_inputs() {
    let f = fixture(Fault::None).await;
    let dir = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let foreign = Backend::with_state_directory(other.path().into()).unwrap();
    let source = connect(&f.source, &backend, false).await;
    let dest = connect(&f.destination, &backend, true).await;
    assert!(matches!(
        source.join_pool(
            OperationId::new(),
            selection(&source, &dest),
            &dest,
            credentials()
        ),
        Err(xenadmin_core::Error::ReadOnly)
    ));
    drop(dest);
    let dest = connect(&f.destination, &foreign, false).await;
    assert!(matches!(
        source.join_pool(
            OperationId::new(),
            selection(&source, &dest),
            &dest,
            credentials()
        ),
        Err(xenadmin_core::Error::InvalidResourceCommand)
    ));
    let selected = selection(&source, &dest);
    let command = ResourceCommand::Pool {
        target: selected.source,
        operation: xenadmin_core::PoolOperation::Join {
            source_host: selected.source_host,
            destination_pool: PoolId::parse(DEST).unwrap(),
            destination_coordinator: selected.destination_coordinator,
            destination_address: selected.destination_address,
        },
    };
    assert!(matches!(
        source.submit_resource(OperationId::new(), command),
        Err(xenadmin_core::Error::InvalidResourceCommand)
    ));
    assert_eq!(joins(&f.source), 0);
}
