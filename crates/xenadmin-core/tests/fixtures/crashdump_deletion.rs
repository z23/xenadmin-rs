use super::*;
use xenadmin_core::{
    HostCrashdump, HostCrashdumpId, HostId, OperationControl, OperationId, OperationState,
    ResourceCommand, ResourceOperation, ResourceOperationRecord, ResourceTarget,
};

const HOST: &str = "22222222-2222-2222-2222-222222222222";
const DUMP: &str = "44444444-4444-4444-4444-444444444444";
#[derive(Clone, Copy, PartialEq)]
enum Fault {
    None,
    Lost,
    FailedAfterRemoval,
    Retained,
    Timestamp,
    Size,
    Owner,
    Rebound,
    Missing,
    Offline,
    Busy,
    Denied,
}
fn dump() -> HostCrashdump {
    HostCrashdump {
        uuid: HostCrashdumpId::parse(DUMP).unwrap(),
        timestamp: "20261108T00:00:00Z".into(),
        size: 4096,
    }
}
async fn fixture(fault: Fault) -> Mock {
    let removed = Arc::new(std::sync::atomic::AtomicBool::new(fault == Fault::Missing));
    mock_overridden(override_rpc(move |r| {
        let removed=removed.clone();
        async move {
            let method=r["method"].as_str().unwrap();
            let p=&r["params"];
            let value=match method {
                "event.from" if p[2]==""=>Some(batch("initial","Halted")),
                "session.get_record"=>Some(json!({"is_local_superuser":fault!=Fault::Denied,"rbac_permissions":[]})),
                "pool.get_by_uuid"=>Some(json!("OpaqueRef:pool")),
                "host.get_by_uuid"=>Some(json!("OpaqueRef:host")),
                "pool.get_record"=>Some(batch("x","Halted")["events"][0]["snapshot"].clone()),
                "host.get_record"=>Some(json!({"uuid":HOST,"metrics":"OpaqueRef:host-metrics","current_operations":if fault==Fault::Busy {json!({"OpaqueRef:busy":"reboot"})}else{json!({})}})),
                "host_metrics.get_record"=>Some(json!({"live":fault!=Fault::Offline})),
                "host_crashdump.get_by_uuid"=>{
                    assert_eq!(p[1],DUMP);
                    if removed.load(Ordering::SeqCst) {return Some(RpcReply::Json(json!({"error":{"message":"UUID_INVALID"}})));}
                    Some(json!("OpaqueRef:crashdump"))
                },
                "host_crashdump.get_record"=>{
                    assert_eq!(p[1],"OpaqueRef:crashdump");
                    Some(json!({"uuid":if fault==Fault::Rebound {"55555555-5555-5555-5555-555555555555"}else{DUMP},"host":if fault==Fault::Owner {"OpaqueRef:other-host"}else{"OpaqueRef:host"},"timestamp":if fault==Fault::Timestamp {"20261107T00:00:00Z"}else{"20261108T00:00:00Z"},"size":if fault==Fault::Size {8192}else{4096}}))
                },
                "host_crashdump.destroy"=>{
                    assert_eq!(p[1],"OpaqueRef:crashdump");
                    assert_eq!(p.as_array().unwrap().len(),2);
                    if fault!=Fault::Retained {removed.store(true,Ordering::SeqCst);}
                    if fault==Fault::Lost {return Some(RpcReply::Drop);}
                    if fault==Fault::FailedAfterRemoval {return Some(RpcReply::Json(json!({"error":{"message":"INTERNAL_ERROR"}})));}
                    Some(Value::Null)
                },
                _=>None,
            };
            value.map(|v|RpcReply::Json(json!({"result":v})))
        }
    })).await
}
async fn connect(mock: &Mock, backend: &Backend, read_only: bool) -> xenadmin_core::Connection {
    let mut options = ConnectionOptions::new(mock.endpoint.clone());
    options.read_only = read_only;
    let c = backend.connect(options, Credentials::new("root".into(), "fixture".into()));
    c.wait_ready(Duration::from_secs(8)).await.unwrap();
    c
}
fn command(c: &xenadmin_core::Connection) -> ResourceCommand {
    ResourceCommand::DeleteHostCrashdump {
        host: c
            .latest()
            .observe_resource(ResourceTarget::Host(HostId::parse(HOST).unwrap()))
            .unwrap(),
        dump: dump(),
    }
}
async fn finish(op: &ResourceOperation) -> Arc<ResourceOperationRecord> {
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            let row = op.latest();
            if row.state.terminal() || row.state == OperationState::OutcomeUnknown {
                return row;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap()
}
fn calls(mock: &Mock) -> usize {
    mock.methods
        .lock()
        .unwrap()
        .iter()
        .filter(|m| m.as_str() == "host_crashdump.destroy")
        .count()
}
#[tokio::test]
async fn reviewed_crashdump_deletion_requires_server_completion_and_record_absence() {
    let m = fixture(Fault::None).await;
    let d = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(d.path().into()).unwrap();
    let c = connect(&m, &b, false).await;
    let op = c.submit_resource(OperationId::new(), command(&c)).unwrap();
    let row = finish(&op).await;
    assert_eq!(
        row.state,
        OperationState::Succeeded,
        "{} {:?}",
        row.detail,
        m.methods.lock().unwrap()
    );
    assert!(row.detail.contains("not independently inspected"));
    assert_eq!(calls(&m), 1);
    let public = serde_json::to_string(row.as_ref()).unwrap();
    assert!(!public.contains("OpaqueRef:"));
    assert!(!public.contains("crashdump_deletion"));
    assert!(!public.contains("filename"));
    c.close().await;
}
#[tokio::test]
async fn changed_crashdump_identity_owner_metadata_or_host_state_refuses_before_delete() {
    for fault in [
        Fault::Timestamp,
        Fault::Size,
        Fault::Owner,
        Fault::Rebound,
        Fault::Missing,
        Fault::Offline,
        Fault::Busy,
        Fault::Denied,
    ] {
        let m = fixture(fault).await;
        let d = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(d.path().into()).unwrap();
        let c = connect(&m, &b, false).await;
        let op = c.submit_resource(OperationId::new(), command(&c)).unwrap();
        let row = finish(&op).await;
        assert_eq!(row.state, OperationState::Rejected, "{}", row.detail);
        assert_eq!(calls(&m), 0);
        c.close().await;
    }
    let m = fixture(Fault::None).await;
    let d = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(d.path().into()).unwrap();
    let c = connect(&m, &b, true).await;
    assert!(matches!(
        c.submit_resource(OperationId::new(), command(&c)),
        Err(xenadmin_core::Error::ReadOnly)
    ));
    assert_eq!(calls(&m), 0);
    c.close().await;
}
#[tokio::test]
async fn vanished_record_after_failed_or_lost_delete_is_unknown_and_recheck_never_repeats_delete() {
    for fault in [Fault::Lost, Fault::FailedAfterRemoval, Fault::Retained] {
        let m = fixture(fault).await;
        let d = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(d.path().into()).unwrap();
        let c = connect(&m, &b, false).await;
        let id = OperationId::new();
        let op = c.submit_resource(id, command(&c)).unwrap();
        let before = finish(&op).await;
        assert_eq!(
            before.state,
            OperationState::OutcomeUnknown,
            "{}",
            before.detail
        );
        c.control_resource(id, OperationControl::Recheck).unwrap();
        let after = tokio::time::timeout(Duration::from_secs(8), async {
            loop {
                if let Some(row) = c
                    .resource_history()
                    .iter()
                    .find(|r| {
                        r.id == id
                            && r.revision() > before.revision()
                            && r.state == OperationState::OutcomeUnknown
                    })
                    .cloned()
                {
                    return row;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(after.state, OperationState::OutcomeUnknown);
        assert_eq!(calls(&m), 1);
        c.close().await;
    }
}
