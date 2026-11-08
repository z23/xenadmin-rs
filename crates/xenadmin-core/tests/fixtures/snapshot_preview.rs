use super::*;
use std::sync::atomic::AtomicBool;
use tokio::sync::Notify;

async fn preview_mock(block: Arc<AtomicBool>, entered: Arc<Notify>, release: Arc<Notify>) -> Mock {
    mock_overridden(override_rpc(move |r|{
        let block=block.clone();let entered=entered.clone();let release=release.clone();
        async move {
            let vm=||{let mut record=batch("unused","Halted")["events"][2]["snapshot"].clone();record["is_a_snapshot"]=json!(false);record["is_a_template"]=json!(false);record["is_control_domain"]=json!(false);record};
            let result=match r["method"].as_str().unwrap(){
                "event.from" if r["params"][2]==""=>{
                    let mut initial=batch("initial","Halted");initial["events"][2]["snapshot"]=vm();initial["events"].as_array_mut().unwrap().push(json!({"class":"VM","operation":"add","ref":"OpaqueRef:snapshot","snapshot":snapshot_record(false)}));initial
                }
                "pool.get_by_uuid"=>{
                    if block.load(Ordering::SeqCst){entered.notify_one();release.notified().await;}
                    json!("OpaqueRef:pool")
                },
                "pool.get_record"=>batch("unused","Halted")["events"][0]["snapshot"].clone(),
                "VM.get_by_uuid"=>if r["params"][1]==GUEST_UUID{json!("OpaqueRef:vm")}else{json!("OpaqueRef:snapshot")},
                "VM.get_record"=>if r["params"][1]=="OpaqueRef:vm"{vm()}else{snapshot_record(false)},
                _=>return None,
            };Some(RpcReply::Json(json!({"result":result})))
        }
    })).await
}
async fn result(
    query: &xenadmin_core::SnapshotPreviewQuery,
) -> Result<Option<xenadmin_core::SnapshotPreview>, String> {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(v) = query.latest() {
                return v;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap()
}
#[tokio::test]
async fn snapshot_preview_facade_allows_readonly_and_rejects_another_connection_generation() {
    let server = preview_mock(Default::default(), Default::default(), Default::default()).await;
    let backend = Backend::start().unwrap();
    let mut options = ConnectionOptions::new(server.endpoint.clone());
    options.read_only = true;
    let connection = backend.connect(
        options.clone(),
        Credentials::new("root".into(), "fixture".into()),
    );
    let i = connection.wait_ready(Duration::from_secs(3)).await.unwrap();
    let vm = i.guests().next().unwrap().id;
    let snapshot = xenadmin_core::SnapshotId::parse(SNAPSHOT_UUID).unwrap();
    let observed = connection.latest().observe_snapshot(vm, snapshot).unwrap();
    assert!(
        result(&connection.snapshot_preview(observed).unwrap())
            .await
            .unwrap()
            .is_none()
    );
    let other_backend = Backend::start().unwrap();
    let other = other_backend.connect(options, Credentials::new("root".into(), "fixture".into()));
    other.wait_ready(Duration::from_secs(3)).await.unwrap();
    assert!(matches!(
        other.snapshot_preview(observed),
        Err(xenadmin_core::Error::Stale)
    ));
    assert_eq!(server.mutations.load(Ordering::SeqCst), 0);
    connection.close().await;
    other.close().await;
}
#[tokio::test]
async fn snapshot_preview_pending_read_is_cancelled_by_connection_retirement() {
    let block = Arc::new(AtomicBool::new(false));
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let server = preview_mock(block.clone(), entered.clone(), release.clone()).await;
    let backend = Backend::start().unwrap();
    let connection = backend.connect(
        ConnectionOptions::new(server.endpoint.clone()),
        Credentials::new("root".into(), "fixture".into()),
    );
    let i = connection.wait_ready(Duration::from_secs(3)).await.unwrap();
    let vm = i.guests().next().unwrap().id;
    let observed = connection
        .latest()
        .observe_snapshot(vm, xenadmin_core::SnapshotId::parse(SNAPSHOT_UUID).unwrap())
        .unwrap();
    block.store(true, Ordering::SeqCst);
    let query = connection.snapshot_preview(observed).unwrap();
    tokio::time::timeout(Duration::from_secs(3), entered.notified())
        .await
        .unwrap();
    connection.close().await;
    assert!(result(&query).await.is_err());
    release.notify_one();
    assert_eq!(server.mutations.load(Ordering::SeqCst), 0);
}
