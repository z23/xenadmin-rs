use super::*;
use std::collections::BTreeMap;
use xenadmin_core::{
    HostId, NfsVersion, OperationControl, OperationId, OperationState, PbdId, PoolId,
    ResourceCommand, ResourceOperation, ResourceOperationRecord, ResourceTarget, SrId,
    StorageDiscoveryResult, StorageLocation, StorageOperation, StorageQuery,
};
const POOL: &str = "11111111-1111-1111-1111-111111111111";
const HOST: &str = "22222222-2222-2222-2222-222222222222";
const SR: &str = "66666666-6666-6666-6666-666666666666";
const PBD: &str = "77777777-7777-7777-7777-777777777777";
const CREATED: &str = "88888888-8888-8888-8888-888888888888";
#[derive(Clone, Copy, PartialEq)]
enum Fault {
    None,
    DropReply,
    VerifyFailure,
    WrongResult,
    DiskConsumer,
    DefaultStorage,
    SlowProbe,
    ProbeError,
}
type Records = Arc<Mutex<BTreeMap<String, Value>>>;
fn nfs() -> StorageLocation {
    StorageLocation::Nfs {
        server: "nfs.example.test".into(),
        path: "/images".into(),
        version: NfsVersion::V3,
    }
}
async fn fixture(fault: Fault) -> (Mock, Records) {
    let records: Records = Arc::new(Mutex::new(BTreeMap::from([
        (
            "pool".into(),
            json!({"uuid":POOL,"name_label":"Fixture pool","default_SR":if fault==Fault::DefaultStorage{"OpaqueRef:SR"}else{"OpaqueRef:NULL"},"suspend_image_SR":"OpaqueRef:NULL","crash_dump_SR":"OpaqueRef:NULL","ha_statefiles":[],"master":"OpaqueRef:host"}),
        ),
        (
            "host".into(),
            json!({"uuid":HOST,"name_label":"Fixture host","address":"192.0.2.10","enabled":false,"local_cache_sr":"OpaqueRef:NULL","suspend_image_sr":"OpaqueRef:NULL","crash_dump_sr":"OpaqueRef:NULL"}),
        ),
        (
            "SR".into(),
            json!({"uuid":SR,"name_label":"Fixture storage","name_description":"Storage","type":"nfs","content_type":"user","shared":true,"is_tools_sr":false,"local_cache_enabled":false,"PBDs":["OpaqueRef:PBD"],"VDIs":if fault==Fault::DiskConsumer{json!(["OpaqueRef:VDI"])}else{json!([])},"current_operations":{},"allowed_operations":["scan","plug","unplug","pbd_create","pbd_destroy","forget","destroy"],"physical_size":"1048576","physical_utilisation":"0"}),
        ),
        (
            "PBD".into(),
            json!({"uuid":PBD,"host":"OpaqueRef:host","SR":"OpaqueRef:SR","currently_attached":false,"device_config":{"server":"nfs.example.test","serverpath":"/images","nfsversion":"3"},"other_config":{"multipathed":"true","mpath-lun":"[2, 4]","password":"never-project-secret"}}),
        ),
        (
            "SM".into(),
            json!({"uuid":"99999999-9999-9999-9999-999999999999","type":"nfs","name_label":"NFS","capabilities":["SR_PROBE"],"features":{},"configuration":{"server":"NFS server"}}),
        ),
        (
            "VDI".into(),
            json!({"uuid":"aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa","type":"user","current_operations":{},"VBDs":["OpaqueRef:VBD"]}),
        ),
        ("VBD".into(), json!({"currently_attached":true})),
    ])));
    let map = records.clone();
    let writes = Arc::new(AtomicUsize::new(0));
    let failures = Arc::new(AtomicUsize::new(0));
    let result_ref = Arc::new(Mutex::new(None::<String>));
    let mock=mock_overridden(override_rpc(move|request|{let records=map.clone();let writes=writes.clone();let failures=failures.clone();let result_ref=result_ref.clone();async move {
        let method=request["method"].as_str().unwrap();let params=&request["params"];
        if method=="event.from"&&params[2]=="" {
            let map=records.lock().unwrap();let mut value=batch("initial","Halted");
            for event in value["events"].as_array_mut().unwrap() {if let Some(record)=map.get(event["class"].as_str().unwrap()){event["snapshot"]=record.clone();}}
            for class in ["SR","PBD","SM"] {value["events"].as_array_mut().unwrap().push(json!({"class":class,"operation":"add","ref":format!("OpaqueRef:{class}"),"snapshot":map[class]}));}
            return Some(RpcReply::Json(json!({"result":value})));
        }
        if let Some(class)=method.strip_suffix(".get_by_uuid") && ["pool","host","SR","PBD"].contains(&class) {
            let map=records.lock().unwrap();let found=map.iter().find(|(key,r)|key.starts_with(class)&&r["uuid"]==params[1]);
            return Some(RpcReply::Json(match found {Some((key,_))=>json!({"result":format!("OpaqueRef:{key}")}),None=>json!({"error":{"message":"UUID_INVALID","data":[]}})}));
        }
        if let Some(class)=method.strip_suffix(".get_all_records") && ["SM","SR","PBD","host"].contains(&class) {
            let values=records.lock().unwrap().iter().filter(|(key,_)|key.starts_with(class)).map(|(key,r)|(format!("OpaqueRef:{key}"),r.clone())).collect::<serde_json::Map<_,_>>();
            return Some(RpcReply::Json(json!({"result":values})));
        }
        if let Some(class)=method.strip_suffix(".get_record") && ["pool","host","SR","PBD","VDI","VBD"].contains(&class) {
            if fault==Fault::VerifyFailure && writes.load(Ordering::SeqCst)>0 && class=="SR" && failures.fetch_add(1,Ordering::SeqCst)==0 {return Some(RpcReply::Json(json!({"error":{"message":"RBAC_PERMISSION_DENIED","data":["never-project-secret"]}})));}
            let key=params[1].as_str().unwrap().strip_prefix("OpaqueRef:").unwrap();
            return Some(RpcReply::Json(json!({"result":records.lock().unwrap().get(key).cloned().unwrap_or(Value::Null)})));
        }
        if method=="SR.probe" {
            if fault==Fault::SlowProbe {tokio::time::sleep(Duration::from_secs(1)).await;}
            if fault==Fault::ProbeError {return Some(RpcReply::Json(json!({"error":{"message":"SR_BACKEND_FAILURE_68","data":["private-probe-password"]}})));}
            return Some(RpcReply::Json(json!({"result":format!("<SRlist><SR><UUID>{CREATED}</UUID><name_label>Existing</name_label><size>4096</size></SR></SRlist>")})));}
        if let Some(method)=method.strip_prefix("Async.") {
            writes.fetch_add(1,Ordering::SeqCst);let mut map=records.lock().unwrap();
            match method {
                "host.call_plugin"=>{assert_eq!(params[2],"trim");assert_eq!(params[3],"do_trim");assert_eq!(params[4]["sr_uuid"],SR);*result_ref.lock().unwrap()=Some("true".into());},
                "PBD.plug"=>map.get_mut("PBD").unwrap()["currently_attached"]=json!(true),
                "PBD.unplug"=>map.get_mut("PBD").unwrap()["currently_attached"]=json!(false),
                "PBD.destroy"=>{map.remove("PBD");},
                "SR.forget"|"SR.destroy"=>{map.remove("SR");},
                "SR.create"=>{
                    let created=json!({"uuid":CREATED,"name_label":params[4],"name_description":params[5],"type":params[6],"content_type":params[7],"shared":params[8],"PBDs":["OpaqueRef:PBD-created"],"VDIs":[],"current_operations":{},"allowed_operations":[]});
                    map.insert("SR-created".into(),created);map.insert("PBD-created".into(),json!({"uuid":"bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb","host":params[1],"SR":"OpaqueRef:SR-created","currently_attached":true,"device_config":params[2]}));
                    *result_ref.lock().unwrap()=Some(if fault==Fault::WrongResult{"OpaqueRef:SR"}else{"OpaqueRef:SR-created"}.into());
                }
                "SR.introduce"=>{map.insert("SR-created".into(),json!({"uuid":params[1],"name_label":params[2],"name_description":params[3],"type":params[4],"content_type":params[5],"shared":params[6],"PBDs":[],"VDIs":[]}));*result_ref.lock().unwrap()=Some("OpaqueRef:SR-created".into());},
                "PBD.create"=>{let mut pbd=params[1].clone();pbd["uuid"]=json!(CREATED);pbd["currently_attached"]=json!(false);map.insert("PBD-created".into(),pbd);*result_ref.lock().unwrap()=Some("OpaqueRef:PBD-created".into());},
                _=>panic!("unexpected storage mutation {method}"),
            }
            return Some(if fault==Fault::DropReply{RpcReply::Drop}else{RpcReply::Json(json!({"result":"OpaqueRef:storage-task"}))});
        }
        if method=="task.get_record" {
            let result=result_ref.lock().unwrap().as_ref().map(|r|format!("<value>{r}</value>")).unwrap_or_else(||"<value/>".into());
            return Some(RpcReply::Json(json!({"result":{"status":"success","progress":"1","allowed_operations":[],"result":result}})));
        }
        None
    }})).await;
    (mock, records)
}
async fn connection(mock: &Mock, backend: &Backend, read_only: bool) -> xenadmin_core::Connection {
    let mut options = ConnectionOptions::new(mock.endpoint.clone());
    options.read_only = read_only;
    let c = backend.connect(
        options,
        Credentials::new("fixture".into(), "fixture-password".into()),
    );
    let mut watch = c.watch();
    tokio::time::timeout(Duration::from_secs(3), async {
        while !watch.borrow().is_current() {
            watch.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    c
}
async fn finished(operation: &ResourceOperation) -> Arc<ResourceOperationRecord> {
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            let r = operation.latest();
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
fn sr() -> ResourceTarget {
    ResourceTarget::Storage(SrId::parse(SR).unwrap())
}
fn pbd() -> ResourceTarget {
    ResourceTarget::Pbd(PbdId::parse(PBD).unwrap())
}
fn create() -> StorageOperation {
    StorageOperation::Create {
        host: HostId::parse(HOST).unwrap(),
        name: "New storage".into(),
        description: "Synthetic".into(),
        location: nfs(),
    }
}
fn submit(
    c: &xenadmin_core::Connection,
    target: ResourceTarget,
    operation: StorageOperation,
) -> ResourceOperation {
    c.submit_resource(
        OperationId::new(),
        ResourceCommand::Storage {
            target: c.latest().observe_resource(target).unwrap(),
            operation,
        },
    )
    .unwrap()
}
#[tokio::test]
async fn storage_create_uses_async_task_and_checks_created_identity_and_topology() {
    for fault in [Fault::None, Fault::WrongResult, Fault::VerifyFailure] {
        let (mock, _) = fixture(fault).await;
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connection(&mock, &backend, false).await;
        let op = submit(&c, pool(), create());
        let record = finished(&op).await;
        assert_eq!(
            record.state,
            if fault == Fault::None {
                OperationState::Succeeded
            } else {
                OperationState::OutcomeUnknown
            },
            "{}",
            record.detail
        );
        if fault == Fault::None {
            assert_eq!(
                record.created,
                Some(ResourceTarget::Storage(SrId::parse(CREATED).unwrap()))
            );
        }
        if fault == Fault::VerifyFailure {
            c.control_resource(record.id, OperationControl::Recheck)
                .unwrap();
            tokio::time::timeout(Duration::from_secs(3), async {
                loop {
                    if c.resource_history()
                        .iter()
                        .any(|r| r.id == record.id && r.state == OperationState::Succeeded)
                    {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .unwrap();
        }
        let public = serde_json::to_string(&*record).unwrap();
        assert!(!public.contains("OpaqueRef:"));
        assert!(!public.contains("fixture-password"));
        assert_eq!(
            mock.methods
                .lock()
                .unwrap()
                .iter()
                .filter(|m| m.as_str() == "Async.SR.create")
                .count(),
            1
        );
        c.close().await;
    }
}
#[tokio::test]
async fn lost_storage_create_response_is_never_replayed() {
    let (mock, _) = fixture(Fault::DropReply).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connection(&mock, &backend, false).await;
    let op = submit(&c, pool(), create());
    let record = finished(&op).await;
    assert_eq!(record.state, OperationState::OutcomeUnknown);
    c.control_resource(record.id, OperationControl::Recheck)
        .unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(
        mock.methods
            .lock()
            .unwrap()
            .iter()
            .filter(|m| m.as_str() == "Async.SR.create")
            .count(),
        1
    );
    assert!(
        !c.resource_history()
            .iter()
            .any(|r| r.id == record.id && r.state.terminal())
    );
    c.close().await;
}
#[tokio::test]
async fn attachment_and_removal_commands_preserve_selected_identities_and_block_dependencies() {
    for fault in [Fault::None, Fault::DiskConsumer, Fault::DefaultStorage] {
        let (mock, _) = fixture(fault).await;
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connection(&mock, &backend, false).await;
        let op = submit(
            &c,
            sr(),
            StorageOperation::Forget {
                expected_name: "Fixture storage".into(),
                expected_disks: vec![],
            },
        );
        let record = finished(&op).await;
        assert_eq!(
            record.state,
            if fault == Fault::None {
                OperationState::Succeeded
            } else {
                OperationState::Rejected
            },
            "{}",
            record.detail
        );
        assert_eq!(
            mock.methods
                .lock()
                .unwrap()
                .iter()
                .any(|m| m == "Async.SR.forget"),
            fault == Fault::None
        );
        c.close().await;
    }
    let (mock, _) = fixture(Fault::None).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connection(&mock, &backend, false).await;
    let op = submit(
        &c,
        pbd(),
        StorageOperation::SetAttached {
            sr: SrId::parse(SR).unwrap(),
            host: HostId::parse(HOST).unwrap(),
            expected_attached: false,
            attached: true,
        },
    );
    assert_eq!(finished(&op).await.state, OperationState::Succeeded);
    let op = submit(
        &c,
        pbd(),
        StorageOperation::RemoveAttachment {
            sr: SrId::parse(SR).unwrap(),
            host: HostId::parse(HOST).unwrap(),
        },
    );
    assert_eq!(finished(&op).await.state, OperationState::Rejected);
    let op = submit(
        &c,
        pbd(),
        StorageOperation::SetAttached {
            sr: SrId::parse(SR).unwrap(),
            host: HostId::parse(HOST).unwrap(),
            expected_attached: true,
            attached: false,
        },
    );
    assert_eq!(finished(&op).await.state, OperationState::Succeeded);
    c.close().await;
}
#[tokio::test]
async fn storage_discovery_is_read_only_and_inventory_hides_secret_config() {
    let (mock, _) = fixture(Fault::None).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connection(&mock, &backend, true).await;
    let observed = c.latest().observe_resource(pool()).unwrap();
    let query = c
        .discover_storage(
            observed,
            StorageQuery::Probe {
                host: HostId::parse(HOST).unwrap(),
                location: nfs(),
            },
        )
        .unwrap();
    let result = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(r) = query.latest() {
                break r;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
    .unwrap();
    assert!(
        matches!(result,StorageDiscoveryResult::Repositories(ref values) if values[0].uuid==SrId::parse(CREATED).unwrap())
    );
    let snapshot = c.latest();
    let pbd = &snapshot.inventory().unwrap().details.pbds[&PbdId::parse(PBD).unwrap()];
    assert_eq!(pbd.paths[0].active, 2);
    assert!(
        !serde_json::to_string(pbd)
            .unwrap()
            .contains("never-project-secret")
    );
    assert!(matches!(
        c.submit_resource(
            OperationId::new(),
            ResourceCommand::Storage {
                target: observed,
                operation: create()
            }
        ),
        Err(xenadmin_core::Error::ReadOnly)
    ));
    assert!(!dir.path().join("operations.sqlite3").exists());
    c.close().await;
}
#[tokio::test]
async fn introduce_never_forgets_existing_uuid_and_attachment_creation_never_duplicates_host() {
    let (mock, records) = fixture(Fault::None).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connection(&mock, &backend, false).await;
    let op = submit(
        &c,
        pool(),
        StorageOperation::Introduce {
            uuid: SrId::parse(SR).unwrap(),
            name: "Existing".into(),
            description: "".into(),
            location: nfs(),
        },
    );
    assert_eq!(finished(&op).await.state, OperationState::Rejected);
    let op = submit(
        &c,
        sr(),
        StorageOperation::CreateAttachment {
            host: HostId::parse(HOST).unwrap(),
            location: nfs(),
        },
    );
    assert_eq!(finished(&op).await.state, OperationState::Rejected);
    records.lock().unwrap().get_mut("SR").unwrap()["PBDs"] = json!([]);
    let op = submit(
        &c,
        sr(),
        StorageOperation::CreateAttachment {
            host: HostId::parse(HOST).unwrap(),
            location: nfs(),
        },
    );
    let r = finished(&op).await;
    assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
    assert_eq!(
        r.created,
        Some(ResourceTarget::Pbd(PbdId::parse(CREATED).unwrap()))
    );
    assert!(
        !mock
            .methods
            .lock()
            .unwrap()
            .iter()
            .any(|m| m == "Async.SR.forget")
    );
    c.close().await;
}

#[tokio::test]
async fn trim_requires_advertised_support_and_a_verified_attached_host() {
    let (mock, records) = fixture(Fault::None).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connection(&mock, &backend, false).await;
    let action = StorageOperation::Trim {
        host: HostId::parse(HOST).unwrap(),
        expected_name: "Fixture storage".into(),
    };
    let op = submit(&c, sr(), action.clone());
    assert_eq!(finished(&op).await.state, OperationState::Rejected);
    records.lock().unwrap().get_mut("SM").unwrap()["features"] = json!({"SR_TRIM":"1"});
    let op = submit(&c, sr(), action.clone());
    assert_eq!(finished(&op).await.state, OperationState::Rejected);
    records.lock().unwrap().get_mut("PBD").unwrap()["currently_attached"] = json!(true);
    let op = submit(&c, sr(), action);
    let record = finished(&op).await;
    assert_eq!(record.state, OperationState::Succeeded, "{}", record.detail);
    assert_eq!(
        mock.methods
            .lock()
            .unwrap()
            .iter()
            .filter(|m| m.as_str() == "Async.host.call_plugin")
            .count(),
        1
    );
    c.close().await;
}

#[tokio::test]
async fn storage_discovery_cancels_with_connection_generation_and_sanitizes_errors() {
    for fault in [Fault::SlowProbe, Fault::ProbeError] {
        let (mock, _) = fixture(fault).await;
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connection(&mock, &backend, true).await;
        let query = c
            .discover_storage(
                c.latest().observe_resource(pool()).unwrap(),
                StorageQuery::Probe {
                    host: HostId::parse(HOST).unwrap(),
                    location: nfs(),
                },
            )
            .unwrap();
        if fault == Fault::SlowProbe {
            c.close().await;
        }
        let result = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Some(result) = query.latest() {
                    break result;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let error = result.unwrap_err();
        assert!(!error.contains("private-probe-password"));
        assert!(!error.contains("OpaqueRef:"));
        if fault == Fault::ProbeError {
            assert!(error.contains("SR_BACKEND_FAILURE_68"));
            c.close().await;
        }
    }
}

#[tokio::test]
async fn typed_iso_and_iscsi_creation_and_existing_sr_introduction_use_exact_backend_arguments() {
    let locations = [
        StorageLocation::IsoNfs {
            server: "iso.example.test".into(),
            path: "/images".into(),
            version: NfsVersion::V41,
        },
        StorageLocation::Iscsi {
            portal: "192.0.2.50".into(),
            target_iqn: "iqn.2026-01.example:fixture".into(),
            scsi_id: "3600fixture".into(),
        },
    ];
    for location in locations {
        let (mock, records) = fixture(Fault::None).await;
        records.lock().unwrap().insert(
            "SM-extra".into(),
            json!({"type":location.kind(),"name_label":"Fixture backend","capabilities":[]}),
        );
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connection(&mock, &backend, false).await;
        let op = submit(
            &c,
            pool(),
            StorageOperation::Create {
                host: HostId::parse(HOST).unwrap(),
                name: "New storage".into(),
                description: "".into(),
                location,
            },
        );
        let record = finished(&op).await;
        assert_eq!(record.state, OperationState::Succeeded, "{}", record.detail);
        c.close().await;
    }
    let (mock, _) = fixture(Fault::None).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connection(&mock, &backend, false).await;
    let op = submit(
        &c,
        pool(),
        StorageOperation::Introduce {
            uuid: SrId::parse(CREATED).unwrap(),
            name: "Existing storage".into(),
            description: "".into(),
            location: nfs(),
        },
    );
    let record = finished(&op).await;
    assert_eq!(record.state, OperationState::Succeeded, "{}", record.detail);
    assert_eq!(
        record.created,
        Some(ResourceTarget::Storage(SrId::parse(CREATED).unwrap()))
    );
    c.close().await;
}
#[tokio::test]
async fn destroy_dispatches_only_after_exact_detached_repository_review() {
    let (mock, _) = fixture(Fault::None).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connection(&mock, &backend, false).await;
    let op = submit(
        &c,
        sr(),
        StorageOperation::Destroy {
            expected_name: "Changed name".into(),
            expected_disks: vec![],
        },
    );
    assert_eq!(finished(&op).await.state, OperationState::Rejected);
    let op = submit(
        &c,
        sr(),
        StorageOperation::Destroy {
            expected_name: "Fixture storage".into(),
            expected_disks: vec![],
        },
    );
    let record = finished(&op).await;
    assert_eq!(record.state, OperationState::Succeeded, "{}", record.detail);
    assert_eq!(
        mock.methods
            .lock()
            .unwrap()
            .iter()
            .filter(|m| m.as_str() == "Async.SR.destroy")
            .count(),
        1
    );
    assert!(
        !mock
            .methods
            .lock()
            .unwrap()
            .iter()
            .any(|m| m == "Async.SR.forget")
    );
    c.close().await;
}
