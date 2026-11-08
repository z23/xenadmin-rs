use super::*;
use std::collections::BTreeMap;
use xenadmin_core::{
    HostId, OperationId, OperationState, PoolId, ResourceCommand, ResourceOperation,
    ResourceTarget, StorageAuthentication, StorageCredentials, StorageLocation, StorageOperation,
};
const POOL: &str = "11111111-1111-1111-1111-111111111111";
const HOST: &str = "22222222-2222-2222-2222-222222222222";
const HOST2: &str = "22222222-2222-2222-2222-222222222223";
const SR: &str = "66666666-6666-6666-6666-666666666666";
const NEW_SR: &str = "88888888-8888-8888-8888-888888888888";
const PBD: &str = "77777777-7777-7777-7777-777777777777";
const PASSWORD: &str = "private-storage-password-8934";
const USERNAME: &str = "private-storage-username-8934";
#[derive(Clone, Copy, PartialEq, Debug)]
enum Fault {
    None,
    LostSecret,
    LostProvision,
    FailedProvision,
    LostCleanup,
    Consumer,
    BootstrapCopy,
    WrongHost,
    ChangedAttachmentRead,
}
type Records = Arc<Mutex<BTreeMap<String, Value>>>;
fn location(auth: StorageAuthentication) -> StorageLocation {
    if auth == StorageAuthentication::Smb {
        StorageLocation::IsoSmb {
            server: "smb.example.test".into(),
            share: "images".into(),
            path: "install".into(),
        }
    } else {
        StorageLocation::Iscsi {
            portal: "iscsi.example.test".into(),
            port: 3261,
            target_iqn: "iqn.2026-01.example:fixture".into(),
            scsi_id: "3600fixture".into(),
            allocation: xenadmin_core::StorageAllocation::DriverDefault,
        }
    }
}
fn credentials(auth: StorageAuthentication) -> StorageCredentials {
    match auth {
        StorageAuthentication::Chap => StorageCredentials::chap(USERNAME.into(), PASSWORD.into()),
        StorageAuthentication::MutualChap => StorageCredentials::mutual_chap(
            USERNAME.into(),
            PASSWORD.into(),
            "private-incoming-user".into(),
            "private-incoming-password".into(),
        ),
        StorageAuthentication::Smb => StorageCredentials::smb(USERNAME.into(), PASSWORD.into()),
    }
}
async fn fixture(auth: StorageAuthentication, fault: Fault) -> (Mock, Records) {
    fixture_delayed(auth, fault, None).await
}
async fn fixture_delayed(
    auth: StorageAuthentication,
    fault: Fault,
    delay: Option<(Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>)>,
) -> (Mock, Records) {
    let kind = if auth == StorageAuthentication::Smb {
        "iso"
    } else {
        "lvmoiscsi"
    };
    let content = if auth == StorageAuthentication::Smb {
        "iso"
    } else {
        "user"
    };
    let map: Records = Arc::new(Mutex::new(BTreeMap::from([
        (
            "pool".into(),
            json!({"uuid":POOL,"name_label":"Fixture pool","master":"OpaqueRef:host"}),
        ),
        (
            "host".into(),
            json!({"uuid":HOST,"name_label":"Fixture host","address":"192.0.2.10","enabled":true}),
        ),
        (
            "host2".into(),
            json!({"uuid":HOST2,"name_label":"Fixture second host","address":"192.0.2.11","enabled":true}),
        ),
        (
            "SR".into(),
            json!({"uuid":SR,"type":kind,"content_type":content,"name_label":"Existing storage","name_description":"","PBDs":[],"VDIs":[],"shared":true,"current_operations":{},"allowed_operations":["pbd_create"]}),
        ),
        (
            "SM".into(),
            json!({"uuid":"99999999-9999-9999-9999-999999999999","type":kind,"name_label":"Fixture driver","features":{"SR_PROBE":1},"configuration":{}}),
        ),
        (
            "secret-existing".into(),
            json!({"uuid":"00000000-0000-0000-0000-000000000009","other_config":{},"value":"preexisting-never-read"}),
        ),
    ])));
    let data = map.clone();
    let pbd_reads = Arc::new(AtomicUsize::new(0));
    let counter = Arc::new(AtomicUsize::new(100));
    let task_result = Arc::new(Mutex::new("OpaqueRef:SR-new".to_string()));
    let mock=mock_overridden(override_rpc(move|request|{let data=data.clone();let counter=counter.clone();let task_result=task_result.clone();let delay=delay.clone();let pbd_reads=pbd_reads.clone();async move{
        let method=request["method"].as_str().unwrap();let p=&request["params"];
        if method=="event.from"&&p[2]==""{
            let m=data.lock().unwrap();let mut b=batch("initial","Halted");
            for e in b["events"].as_array_mut().unwrap(){if let Some(r)=m.get(e["class"].as_str().unwrap()){e["snapshot"]=r.clone();}}
            for class in["host2","SR","SM"]{b["events"].as_array_mut().unwrap().push(json!({"class":if class=="host2"{"host"}else{class},"operation":"add","ref":format!("OpaqueRef:{class}"),"snapshot":m[class]}));}
            return Some(RpcReply::Json(json!({"result":b})));
        }
        if let Some(class)=method.strip_suffix(".get_by_uuid")&&["pool","host","SR","PBD","secret"].contains(&class){
            let m=data.lock().unwrap();let found=m.iter().find(|(k,r)|k.starts_with(class)&&r["uuid"]==p[1]);
            return Some(RpcReply::Json(match found{Some((k,_))=>json!({"result":format!("OpaqueRef:{k}")}),None=>json!({"error":{"message":"UUID_INVALID","data":[]}})}));
        }
        if method=="secret.get_all"{return Some(RpcReply::Json(json!({"result":data.lock().unwrap().keys().filter(|k|k.starts_with("secret")).map(|k|format!("OpaqueRef:{k}")).collect::<Vec<_>>()})));}
        if ["secret.get_uuid","secret.get_other_config"].contains(&method){
            let m=data.lock().unwrap();let k=p[1].as_str().unwrap().strip_prefix("OpaqueRef:").unwrap();
            return Some(RpcReply::Json(json!({"result":m[k][if method=="secret.get_uuid"{"uuid"}else{"other_config"}]})));
        }
        if method=="secret.create"{
            assert!([PASSWORD,"private-incoming-password"].contains(&p[1]["value"].as_str().unwrap()));
            let n=counter.fetch_add(1,Ordering::SeqCst);let key=format!("secret-{n}");let mut r=p[1].clone();r["uuid"]=json!(uuid::Uuid::from_u128(n as u128).to_string());data.lock().unwrap().insert(key.clone(),r);
            if let Some((entered,release))=&delay{entered.notify_one();release.notified().await;}
            return Some(if fault==Fault::LostSecret{RpcReply::Drop}else{RpcReply::Json(json!({"result":format!("OpaqueRef:{key}")}))});
        }
        if method=="secret.destroy"{
            let k=p[1].as_str().unwrap().strip_prefix("OpaqueRef:").unwrap();assert!(k.starts_with("secret-"));assert_ne!(k,"secret-existing");
            let removed=data.lock().unwrap().remove(k).unwrap();assert!(removed["other_config"].as_object().unwrap().len()==1,"must not destroy attachment-owned copies");
            return Some(if fault==Fault::LostCleanup{RpcReply::Drop}else{RpcReply::Json(json!({"result":""}))});
        }
        assert!(!method.starts_with("secret."),"Unexpected Secret read {method}; values must never be retrieved");
        if method=="task.get_all_records"{return Some(RpcReply::Json(json!({"result":{}})));}
        if let Some(class)=method.strip_suffix(".get_all_records")&&["SM","SR","PBD","host"].contains(&class){
            let records=data.lock().unwrap().iter().filter(|(k,_)|k.starts_with(class)).map(|(k,r)|(format!("OpaqueRef:{k}"),r.clone())).collect::<serde_json::Map<_,_>>();return Some(RpcReply::Json(json!({"result":records})));
        }
        if let Some(class)=method.strip_suffix(".get_record")&&["pool","host","SR","PBD"].contains(&class){
            let k=p[1].as_str().unwrap().strip_prefix("OpaqueRef:").unwrap();
            let mut r=data.lock().unwrap().get(k).cloned().unwrap_or(Value::Null);
            if k=="PBD-new"&&fault==Fault::ChangedAttachmentRead&&pbd_reads.fetch_add(1,Ordering::SeqCst)>0{r["SR"]=json!("OpaqueRef:SR-unrelated");}
            return Some(RpcReply::Json(json!({"result":r})));
        }
        if ["Async.SR.create","Async.PBD.create"].contains(&method){
            let mut config=if method=="Async.SR.create"{p[2].clone()}else{p[1]["device_config"].clone()};
            assert!(!config.to_string().contains(PASSWORD));assert_eq!(config[if auth==StorageAuthentication::Smb{"username"}else{"chapuser"}],USERNAME);
            if auth==StorageAuthentication::Smb{assert_eq!(config["vers"],"3.0");assert_eq!(config["type"],"cifs");assert_eq!(config["location"],"//smb.example.test/images/install");}
            else{assert_eq!(config["port"],"3261");assert!(config.get("chappassword").is_none());}
            let mut m=data.lock().unwrap();
            for key in config.as_object().unwrap().keys().filter(|k|k.ends_with("password_secret")).cloned().collect::<Vec<_>>(){
                let old=config[&key].clone();let original=m.values().find(|v|v["uuid"]==old).unwrap();
                let password=original["value"].clone();assert!([PASSWORD,"private-incoming-password"].contains(&password.as_str().unwrap()));
                let n=counter.fetch_add(1,Ordering::SeqCst);let id=uuid::Uuid::from_u128(n as u128).to_string();
                m.insert(format!("secret-copy-{n}"),json!({"uuid":id,"value":password,"other_config":{}}));
                if fault!=Fault::BootstrapCopy{config[&key]=json!(id);}
                if fault==Fault::Consumer{m.get_mut("SR").unwrap()["sm_config"]=json!({"held-secret":old});}
            }
            let sr_ref=if method=="Async.SR.create"{"OpaqueRef:SR-new"}else{"OpaqueRef:SR"};
            let host=if fault==Fault::WrongHost{json!("OpaqueRef:host2")}else if method=="Async.SR.create"{p[1].clone()}else{p[1]["host"].clone()};
            m.insert("PBD-new".into(),json!({"uuid":PBD,"host":host,"SR":sr_ref,"currently_attached":method=="Async.SR.create","device_config":config,"other_config":{}}));
            if method=="Async.SR.create"{m.insert("SR-new".into(),json!({"uuid":NEW_SR,"name_label":p[4],"name_description":p[5],"type":p[6],"content_type":p[7],"shared":p[8],"PBDs":["OpaqueRef:PBD-new"],"VDIs":[],"current_operations":{},"sm_config":{"allocation":p[2].get("allocation").cloned().unwrap_or(json!("thick"))}}));}
            *task_result.lock().unwrap()=if method=="Async.SR.create"{sr_ref}else{"OpaqueRef:PBD-new"}.into();
            return Some(if fault==Fault::LostProvision{RpcReply::Drop}else{RpcReply::Json(json!({"result":"OpaqueRef:storage-auth-task"}))});
        }
        if method=="task.get_record"{return Some(RpcReply::Json(json!({"result":{"uuid":data.lock().unwrap().get("fixture-task-uuid").cloned().unwrap_or(json!("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa")),"status":if fault==Fault::FailedProvision{"failure"}else{"success"},"result":format!("<value>{}</value>",task_result.lock().unwrap()),"error_info":["SR_BACKEND_FAILURE_1",PASSWORD]}})));}
        if method=="SR.probe"{assert_eq!(p[2]["chapuser"],USERNAME);assert_eq!(p[2]["chappassword"],PASSWORD);return Some(RpcReply::Json(json!({"error":{"message":"SR_BACKEND_FAILURE_96","data":["private", "", "<iscsi-target-iqns><TGT><IPAddress>192.0.2.20</IPAddress><Port>3261</Port><TargetIQN>iqn.2026-01.example:fixture</TargetIQN></TGT></iscsi-target-iqns>"]}})));}
        None
    }})).await;
    map.lock().unwrap().get_mut("SR").unwrap()["sm_config"] = json!({});
    (mock, map)
}
async fn connect(mock: &Mock, backend: &Backend, read_only: bool) -> xenadmin_core::Connection {
    let mut options = ConnectionOptions::new(mock.endpoint.clone());
    options.read_only = read_only;
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
async fn finished(op: &ResourceOperation) -> Arc<xenadmin_core::ResourceOperationRecord> {
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
fn pool() -> ResourceTarget {
    ResourceTarget::Pool(PoolId::parse(POOL).unwrap())
}
fn command(auth: StorageAuthentication, attachment: bool) -> StorageOperation {
    if attachment {
        StorageOperation::CreateAttachment {
            host: HostId::parse(HOST2).unwrap(),
            location: location(auth),
        }
    } else {
        StorageOperation::Create {
            hba_device: None,
            local_device: None,
            host: HostId::parse(HOST).unwrap(),
            name: "Authenticated fixture".into(),
            description: "".into(),
            location: location(auth),
        }
    }
}
fn privacy(record: &xenadmin_core::ResourceOperationRecord, dir: &std::path::Path) {
    let public = serde_json::to_string(record).unwrap();
    assert!(!public.contains("storage_auth_plan"));
    assert!(!public.contains("OpaqueRef:"));
    let db = rusqlite::Connection::open_with_flags(
        dir.join("operations.sqlite3"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let payload: String = db
        .query_row(
            "SELECT payload FROM resource_operations WHERE id=?1",
            [record.id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    for text in [&public, &payload] {
        for secret in [
            PASSWORD,
            USERNAME,
            "private-incoming-password",
            "private-incoming-user",
        ] {
            assert!(!text.contains(secret), "credential leaked");
        }
    }
}
#[tokio::test]
async fn authenticated_storage_provisions_secret_copies_and_cleans_only_owned_bootstrap() {
    for auth in [
        StorageAuthentication::Chap,
        StorageAuthentication::MutualChap,
        StorageAuthentication::Smb,
    ] {
        for attachment in [false, true] {
            let (mock, data) = fixture(auth, Fault::None).await;
            let dir = tempfile::tempdir().unwrap();
            let backend = Backend::with_state_directory(dir.path().into()).unwrap();
            let c = connect(&mock, &backend, false).await;
            let target = if attachment {
                ResourceTarget::Storage(xenadmin_core::SrId::parse(SR).unwrap())
            } else {
                pool()
            };
            let op = c
                .submit_authenticated_storage(
                    OperationId::new(),
                    c.latest().observe_resource(target).unwrap(),
                    command(auth, attachment),
                    credentials(auth),
                )
                .unwrap();
            let r = finished(&op).await;
            assert_eq!(
                r.state,
                OperationState::Succeeded,
                "{auth:?} {attachment}: {}",
                r.detail
            );
            assert!(r.storage_secrets.iter().all(|s| s.cleaned));
            assert_eq!(
                r.storage_secrets.len(),
                if auth == StorageAuthentication::MutualChap {
                    2
                } else {
                    1
                }
            );
            assert!(data.lock().unwrap().contains_key("secret-existing"));
            privacy(&r, dir.path());
            c.disconnect();
        }
    }
}
#[tokio::test]
async fn uncertain_auth_storage_keeps_secret_evidence_and_never_cleans_potential_consumers() {
    for fault in [
        Fault::LostSecret,
        Fault::LostProvision,
        Fault::FailedProvision,
        Fault::LostCleanup,
        Fault::Consumer,
        Fault::BootstrapCopy,
        Fault::WrongHost,
        Fault::ChangedAttachmentRead,
    ] {
        let (mock, data) = fixture(StorageAuthentication::Chap, fault).await;
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&mock, &backend, false).await;
        let op = c
            .submit_authenticated_storage(
                OperationId::new(),
                c.latest().observe_resource(pool()).unwrap(),
                command(StorageAuthentication::Chap, false),
                credentials(StorageAuthentication::Chap),
            )
            .unwrap();
        let r = finished(&op).await;
        assert_eq!(
            r.state,
            if fault == Fault::Consumer {
                OperationState::PartiallyCompleted
            } else {
                OperationState::OutcomeUnknown
            },
            "{fault:?}: {}",
            r.detail
        );
        privacy(&r, dir.path());
        if fault != Fault::LostCleanup {
            assert!(data.lock().unwrap().keys().any(|k| k == "secret-100"));
        }
        c.disconnect();
    }
}
#[tokio::test]
async fn storage_auth_read_only_refuses_writes_but_allows_transient_chap_discovery() {
    let (mock, _) = fixture(StorageAuthentication::Chap, Fault::None).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&mock, &backend, true).await;
    let observed = c.latest().observe_resource(pool()).unwrap();
    assert!(matches!(
        c.submit_authenticated_storage(
            OperationId::new(),
            observed,
            command(StorageAuthentication::Chap, false),
            credentials(StorageAuthentication::Chap)
        ),
        Err(xenadmin_core::Error::ReadOnly)
    ));
    let discovery = c
        .discover_authenticated_storage(
            observed,
            xenadmin_core::StorageQuery::IscsiTargets {
                host: HostId::parse(HOST).unwrap(),
                portal: "iscsi.example.test".into(),
                port: 3261,
            },
            credentials(StorageAuthentication::Chap),
        )
        .unwrap();
    let result = tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            if let Some(r) = discovery.latest() {
                return r;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
    .unwrap();
    assert!(matches!(result,xenadmin_core::StorageDiscoveryResult::IscsiTargets(v) if v.len()==1));
    c.disconnect();
}

#[track_caller]
fn history_state(
    c: &xenadmin_core::Connection,
    id: OperationId,
    predicate: impl Fn(&xenadmin_core::ResourceOperationRecord) -> bool,
) -> impl std::future::Future<Output = xenadmin_core::ResourceOperationRecord> {
    let caller = std::panic::Location::caller();
    async move {
        let mut observed = Vec::new();
        let result = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(r) = c.resource_history().iter().find(|r| r.id == id) {
                    if predicate(r) {
                        return r.clone();
                    }
                    let public_state = format!(
                        "{:?}: {} (steps {:?}; secrets {:?})",
                        r.state, r.detail, r.steps, r.storage_secrets
                    );
                    if observed.last() != Some(&public_state) {
                        observed.push(public_state);
                    }
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        result.unwrap_or_else(|_| panic!("history wait at {caller} timed out: {observed:?}"))
    }
}

#[tokio::test]
async fn explicit_cleanup_requires_source_provenance_and_absent_consumers() {
    let (mock, data) = fixture(StorageAuthentication::Chap, Fault::Consumer).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&mock, &backend, false).await;
    let op = c
        .submit_authenticated_storage(
            OperationId::new(),
            c.latest().observe_resource(pool()).unwrap(),
            command(StorageAuthentication::Chap, false),
            credentials(StorageAuthentication::Chap),
        )
        .unwrap();
    let r = finished(&op).await;
    assert_eq!(r.state, OperationState::PartiallyCompleted);
    let id = r.storage_secrets[0].secret;
    let submit = || {
        c.submit_resource(
            OperationId::new(),
            ResourceCommand::CleanupStorageCredentials {
                pool: c.latest().observe_resource(pool()).unwrap(),
                operation: r.id,
                secrets: vec![id],
            },
        )
        .unwrap()
    };
    let rejected = finished(&submit()).await;
    assert_eq!(rejected.state, OperationState::Rejected);
    assert!(
        !mock
            .methods
            .lock()
            .unwrap()
            .iter()
            .any(|m| m == "secret.destroy")
    );
    data.lock().unwrap().get_mut("SR").unwrap()["sm_config"] = json!({});
    let cleaned = finished(&submit()).await;
    assert_eq!(
        cleaned.state,
        OperationState::Succeeded,
        "{}",
        cleaned.detail
    );
    assert!(cleaned.storage_secrets[0].cleaned);
    assert!(
        data.lock()
            .unwrap()
            .keys()
            .any(|k| k.starts_with("secret-copy-"))
    );
    privacy(&cleaned, dir.path());
    c.disconnect();
}
#[tokio::test]
async fn lost_provision_receipt_blocks_cleanup_even_after_acknowledgement() {
    let (mock, _) = fixture(StorageAuthentication::Chap, Fault::LostProvision).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&mock, &backend, false).await;
    let op = c
        .submit_authenticated_storage(
            OperationId::new(),
            c.latest().observe_resource(pool()).unwrap(),
            command(StorageAuthentication::Chap, false),
            credentials(StorageAuthentication::Chap),
        )
        .unwrap();
    let r = finished(&op).await;
    history_state(&c, r.id, |r| r.state == OperationState::OutcomeUnknown).await;
    c.control_resource(r.id, xenadmin_core::OperationControl::AcknowledgeUnknown)
        .unwrap();
    history_state(&c, r.id, |r| r.state.terminal()).await;
    let cleanup = c
        .submit_resource(
            OperationId::new(),
            ResourceCommand::CleanupStorageCredentials {
                pool: c.latest().observe_resource(pool()).unwrap(),
                operation: r.id,
                secrets: vec![r.storage_secrets[0].secret],
            },
        )
        .unwrap();
    let result = finished(&cleanup).await;
    assert_eq!(result.state, OperationState::Rejected);
    assert!(result.detail.contains("missing task receipt"));
    assert!(
        !mock
            .methods
            .lock()
            .unwrap()
            .iter()
            .any(|m| m == "secret.destroy")
    );
    c.disconnect();
}
#[tokio::test]
async fn lost_secret_reply_recheck_retains_uuid_without_provisioning_or_cleanup() {
    let (mock, _) = fixture(StorageAuthentication::Chap, Fault::LostSecret).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&mock, &backend, false).await;
    let op = c
        .submit_authenticated_storage(
            OperationId::new(),
            c.latest().observe_resource(pool()).unwrap(),
            command(StorageAuthentication::Chap, false),
            credentials(StorageAuthentication::Chap),
        )
        .unwrap();
    let r = finished(&op).await;
    history_state(&c, r.id, |r| r.state == OperationState::OutcomeUnknown).await;
    c.control_resource(r.id, xenadmin_core::OperationControl::Recheck)
        .unwrap();
    let recovered = history_state(&c, r.id, |r| {
        !r.storage_secrets.is_empty() && r.state == OperationState::OutcomeUnknown
    })
    .await;
    assert_eq!(recovered.storage_secrets.len(), 1);
    assert!(!recovered.storage_secrets[0].cleaned);
    let methods = mock.methods.lock().unwrap();
    assert_eq!(methods.iter().filter(|m| *m == "secret.create").count(), 1);
    assert!(
        !methods
            .iter()
            .any(|m| m == "Async.SR.create" || m == "secret.destroy")
    );
    c.disconnect();
}

#[tokio::test]
async fn delayed_secret_receipt_preserves_recovery_and_ack_but_still_retains_identity() {
    for ack in [false, true] {
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let (mock, _) = fixture_delayed(
            StorageAuthentication::Chap,
            Fault::None,
            Some((entered.clone(), release.clone())),
        )
        .await;
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&mock, &backend, false).await;
        let id = OperationId::new();
        let op = c
            .submit_authenticated_storage(
                id,
                c.latest().observe_resource(pool()).unwrap(),
                command(StorageAuthentication::Chap, false),
                credentials(StorageAuthentication::Chap),
            )
            .unwrap();
        tokio::time::timeout(Duration::from_secs(4), entered.notified())
            .await
            .unwrap();
        let recovering = Backend::with_state_directory(dir.path().into()).unwrap();
        let other = connect(&mock, &recovering, false).await;
        // Startup first publishes the recovered row, then reconciles it asynchronously.
        // Wait for that read-only attempt to finish before releasing the positive reply.
        // Otherwise it may legitimately observe the receipt and report a known partial result.
        history_state(&other, id, |r| {
            r.state == OperationState::OutcomeUnknown
                && r.detail.contains("Secret creation reply was lost")
        })
        .await;
        if ack {
            other
                .control_resource(id, xenadmin_core::OperationControl::AcknowledgeUnknown)
                .unwrap();
            history_state(&other, id, |r| {
                r.state == OperationState::AcknowledgedUnknown
            })
            .await;
        }
        release.notify_one();
        let r = tokio::time::timeout(Duration::from_secs(4), async {
            loop {
                let r = op.latest();
                if !r.storage_secrets.is_empty() {
                    return r;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            r.state,
            if ack {
                OperationState::AcknowledgedUnknown
            } else {
                OperationState::OutcomeUnknown
            }
        );
        assert_eq!(r.storage_secrets.len(), 1);

        let methods = mock.methods.lock().unwrap();
        assert_eq!(methods.iter().filter(|m| *m == "secret.create").count(), 1);
        assert!(
            !methods
                .iter()
                .any(|m| m == "Async.SR.create" || m == "secret.destroy")
        );
        privacy(&r, dir.path());
        c.disconnect();
        other.disconnect();
    }
}

#[tokio::test]
async fn changed_attachment_and_rebound_task_cannot_authorize_credential_cleanup() {
    let (mock, _) = fixture(StorageAuthentication::Chap, Fault::ChangedAttachmentRead).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&mock, &backend, false).await;
    let sr = ResourceTarget::Storage(xenadmin_core::SrId::parse(SR).unwrap());
    let op = c
        .submit_authenticated_storage(
            OperationId::new(),
            c.latest().observe_resource(sr).unwrap(),
            command(StorageAuthentication::Chap, true),
            credentials(StorageAuthentication::Chap),
        )
        .unwrap();
    assert_eq!(finished(&op).await.state, OperationState::OutcomeUnknown);
    c.disconnect();
    for rebind in [false, true] {
        let (mock, data) = fixture(StorageAuthentication::Chap, Fault::FailedProvision).await;
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&mock, &backend, false).await;
        let op = c
            .submit_authenticated_storage(
                OperationId::new(),
                c.latest().observe_resource(pool()).unwrap(),
                command(StorageAuthentication::Chap, false),
                credentials(StorageAuthentication::Chap),
            )
            .unwrap();
        let r = finished(&op).await;
        history_state(&c, r.id, |r| r.state == OperationState::OutcomeUnknown).await;
        c.control_resource(r.id, xenadmin_core::OperationControl::AcknowledgeUnknown)
            .unwrap();
        history_state(&c, r.id, |r| r.state.terminal()).await;
        if rebind {
            data.lock().unwrap().insert(
                "fixture-task-uuid".into(),
                json!("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"),
            );
        }
        let cleanup = c
            .submit_resource(
                OperationId::new(),
                ResourceCommand::CleanupStorageCredentials {
                    pool: c.latest().observe_resource(pool()).unwrap(),
                    operation: r.id,
                    secrets: vec![r.storage_secrets[0].secret],
                },
            )
            .unwrap();
        let outcome = finished(&cleanup).await;
        assert_eq!(
            outcome.state,
            if rebind {
                OperationState::Rejected
            } else {
                OperationState::Succeeded
            },
            "{}",
            outcome.detail
        );
        assert_eq!(
            mock.methods
                .lock()
                .unwrap()
                .iter()
                .any(|m| m == "secret.destroy"),
            !rebind
        );
        c.disconnect();
    }
}

#[tokio::test]
async fn authenticated_storage_thin_allocation_is_captured_without_persisting_credentials() {
    for auth in [
        StorageAuthentication::Chap,
        StorageAuthentication::MutualChap,
    ] {
        let (mock, data) = fixture(auth, Fault::None).await;
        data.lock().unwrap().get_mut("SM").unwrap()["configuration"] =
            json!({"allocation":"thick or thin"});
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&mock, &backend, false).await;
        let mut operation = command(auth, false);
        if let StorageOperation::Create {
            location: StorageLocation::Iscsi { allocation, .. },
            ..
        } = &mut operation
        {
            *allocation = xenadmin_core::StorageAllocation::Thin;
        }
        let op = c
            .submit_authenticated_storage(
                OperationId::new(),
                c.latest().observe_resource(pool()).unwrap(),
                operation,
                credentials(auth),
            )
            .unwrap();
        let r = finished(&op).await;
        assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
        assert_eq!(
            data.lock().unwrap()["SR-new"]["sm_config"]["allocation"],
            "thin"
        );
        assert_eq!(
            data.lock().unwrap()["PBD-new"]["device_config"]["allocation"],
            "thin"
        );
        assert!(r.storage_secrets.iter().all(|s| s.cleaned));
        privacy(&r, dir.path());
        c.close().await;
    }
}
