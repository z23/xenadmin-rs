use super::*;

fn private_directory() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
    directory
}

#[test]
fn private_socket_handles_status_and_cleans_up_on_drop() {
    let directory = private_directory();
    let server = DevInspection::start(directory.path().into()).unwrap();
    let path = server.socket_path().to_owned();
    assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
    assert_eq!(
        fs::metadata(directory.path()).unwrap().mode() & 0o777,
        0o700
    );
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let status = runtime
        .block_on(inspect_live(&path, &InspectionRequest::Status))
        .unwrap();
    assert_eq!(status["connections"], json!([]));
    assert_eq!(status["total"], 0);
    assert_eq!(status["process_id"], std::process::id());
    assert_eq!(status["application_version"], env!("CARGO_PKG_VERSION"));
    assert!(status["started_at_ms"].as_u64().unwrap() > 0);
    assert!(DevInspection::start(directory.path().into()).is_err());
    drop(server);
    assert!(!path.exists());
}

#[test]
fn recovers_only_owned_stale_sockets_and_preserves_other_paths() {
    let directory = private_directory();
    let path = directory.path().join("dev-inspect.sock");
    let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    drop(listener);
    let server = DevInspection::start(directory.path().into()).unwrap();
    drop(server);
    fs::write(&path, "leave this file alone").unwrap();
    assert!(DevInspection::start(directory.path().into()).is_err());
    assert_eq!(fs::read_to_string(&path).unwrap(), "leave this file alone");
    fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(directory.path().join("elsewhere"), &path).unwrap();
    assert!(DevInspection::start(directory.path().into()).is_err());
    assert!(
        fs::symlink_metadata(&path)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
fn cleanup_does_not_remove_a_replacement_file() {
    let directory = private_directory();
    let server = DevInspection::start(directory.path().into()).unwrap();
    let path = server.socket_path().to_owned();
    fs::remove_file(&path).unwrap();
    fs::write(&path, "replacement").unwrap();
    drop(server);
    assert_eq!(fs::read_to_string(path).unwrap(), "replacement");
}

#[test]
fn rejects_public_directory_and_socket_permissions() {
    let directory = private_directory();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o755)).unwrap();
    assert!(DevInspection::start(directory.path().into()).is_err());
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let server = DevInspection::start(directory.path().into()).unwrap();
    fs::set_permissions(server.socket_path(), fs::Permissions::from_mode(0o666)).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    assert_eq!(
        runtime
            .block_on(inspect_live(
                server.socket_path(),
                &InspectionRequest::Status
            ))
            .unwrap_err()
            .kind(),
        io::ErrorKind::PermissionDenied
    );
}

#[test]
fn malformed_oversized_and_mutating_requests_are_refused() {
    let directory = private_directory();
    let server = DevInspection::start(directory.path().into()).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        for request in [
            b"not-json\n".to_vec(),
            b"{\"command\":\"power\",\"action\":\"hard_shutdown\"}\n".to_vec(),
            vec![b'x'; REQUEST_LIMIT + 1],
        ] {
            let mut stream = UnixStream::connect(server.socket_path()).await.unwrap();
            stream.write_all(&request).await.unwrap();
            let reply: Value =
                serde_json::from_slice(&read_line(stream, RESPONSE_LIMIT).await.unwrap()).unwrap();
            assert_eq!(reply["ok"], false);
            assert!(!reply.to_string().contains("hard_shutdown"));
        }
    });
}

#[test]
fn registration_is_dynamic_and_status_excludes_authentication_fields() {
    let directory = private_directory();
    let server = DevInspection::start(directory.path().into()).unwrap();
    let backend = crate::Backend::with_state_directory(directory.path().join("backend")).unwrap();
    let mut options =
        crate::ConnectionOptions::new(crate::Endpoint::parse("http://127.0.0.1:9").unwrap());
    options.read_only = true;
    let connection = backend.connect(
        options,
        crate::Credentials::new(
            "fixture-private-user".into(),
            "fixture-private-password".into(),
        ),
    );
    let id = ProfileId::new();
    server.set_connections([(id, connection)]);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let status = runtime
        .block_on(inspect_live(
            server.socket_path(),
            &InspectionRequest::Status,
        ))
        .unwrap();
    assert_eq!(status["connections"][0]["connection"], json!(id));
    assert_eq!(status["connections"][0]["read_only"], true);
    let encoded = status.to_string();
    for forbidden in [
        "127.0.0.1",
        "fixture-private-user",
        "fixture-private-password",
        "OpaqueRef:",
    ] {
        assert!(!encoded.contains(forbidden));
    }
    server.set_connections([]);
    assert_eq!(
        runtime
            .block_on(inspect_live(
                server.socket_path(),
                &InspectionRequest::Status
            ))
            .unwrap()["total"],
        0
    );
}

#[test]
fn operation_summary_does_not_serialize_private_intent_or_rpc_references() {
    let record: crate::OperationRecord = serde_json::from_value(json!({
        "id":"00000000-0000-0000-0000-000000000001", "pool":"00000000-0000-0000-0000-000000000002",
        "vm":"00000000-0000-0000-0000-000000000003", "action":{"kind":"Create","name":"private-snapshot-name"},
        "state":"Succeeded", "progress":1.0, "detail":"Completed operation for OpaqueRef:hidden",
        "steps":[{"label":"Copy snapshot", "state":"Verified", "detail":"Copy verified"}],
        "result_reference":"OpaqueRef:private-result", "task":"OpaqueRef:private-task"
    })).unwrap();
    let value = operation_value(&record);
    assert_eq!(value["action"], "Create snapshot");
    assert_eq!(value["state"], "Succeeded");
    assert_eq!(value["steps"][0]["detail"], "Copy verified");
    assert!(
        value["detail"]
            .as_str()
            .unwrap()
            .contains("Completed operation")
    );
    assert!(!value.to_string().contains("private"));
    assert!(!value.to_string().contains("OpaqueRef:"));
    let diagnostic = diagnostic_value(ActionAvailability::Blocked {
        reason: "Blocked by OpaqueRef:hidden-reference pending maintenance".into(),
    });
    assert!(!diagnostic.to_string().contains("hidden-reference"));
    assert!(diagnostic.to_string().contains("pending maintenance"));
}

#[tokio::test]
async fn message_reader_requires_a_bounded_complete_frame() {
    assert!(read_line(&b"{}"[..], 10).await.is_err());
    assert!(read_line(&b"{}\n"[..], 2).await.is_err());
    assert_eq!(read_line(&b"{}\ntrailing"[..], 3).await.unwrap(), b"{}\n");
}

#[test]
fn inventory_pages_expose_typed_devices_and_snapshot_relationships() {
    let mut inventory: Inventory = serde_json::from_value(json!({
        "pool_id":"00000000-0000-0000-0000-000000000001","pool_name":"Fixture pool",
        "coordinator":null,"hosts":{},"vms":{},"storage":[],"networks":[],"revision":1
    }))
    .unwrap();
    let vm = VmId::parse("00000000-0000-0000-0000-000000000002").unwrap();
    let disk: crate::VirtualDisk = serde_json::from_value(json!({"id":"00000000-0000-0000-0000-000000000003","name":"Fixture disk","description":"Do not serialize raw record","kind":"user","virtual_size":4096})).unwrap();
    let disk_id = disk.id;
    inventory.details.vdis.insert(disk.id, disk);
    let device: crate::VirtualBlockDevice = serde_json::from_value(json!({"id":"00000000-0000-0000-0000-000000000004","vm":vm,"vdi":disk_id,"device":"xvda","userdevice":"0","kind":"Disk","mode":"RW","currently_attached":true})).unwrap();
    inventory.details.vbds.insert(device.id, device);
    let report = inventory_page(
        &inventory,
        InspectionClass::Disks,
        Some(disk_id.uuid()),
        Some(vm),
        0,
        1,
    )
    .unwrap();
    assert_eq!(report["total"], 1);
    assert_eq!(report["objects"][0]["virtual_size"], 4096);
    assert_eq!(report["objects"][0]["attachments"][0]["vm"], json!(vm));
    assert!(!report.to_string().contains("Do not serialize raw record"));
    assert_eq!(
        inventory_page(&inventory, InspectionClass::Disks, None, None, 1, 1).unwrap()["objects"],
        json!([])
    );
    let snapshot: crate::Snapshot = serde_json::from_value(json!({"id":"00000000-0000-0000-0000-000000000005","vm":vm,"name":"Fixture snapshot","description":"Private description","power_state":"Halted","allowed_actions":[]})).unwrap();
    inventory.snapshots.insert(snapshot);
    assert_eq!(
        inventory_page(&inventory, InspectionClass::Snapshots, None, Some(vm), 0, 1).unwrap()["objects"]
            [0]["vm"],
        json!(vm)
    );
    let task: crate::ServerTask = serde_json::from_value(json!({"id":"00000000-0000-0000-0000-000000000006","name":"Copy task","description":"Private task detail","status":"failure","progress":0.5,"error_code":"SR_BACKEND_FAILURE"})).unwrap();
    inventory.details.tasks.insert(task.id, task);
    let tasks = inventory_page(&inventory, InspectionClass::Tasks, None, None, 0, 1).unwrap();
    assert_eq!(tasks["objects"][0]["error_code"], "SR_BACKEND_FAILURE");
    assert!(inventory_page(&inventory, InspectionClass::Tasks, None, Some(vm), 0, 1).is_err());
}

#[test]
fn live_diagnostic_uses_registered_session_and_reads_current_vm_without_mutation() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let directory = private_directory();
    let server = DevInspection::start(directory.path().into()).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let reads = Arc::new(AtomicUsize::new(0));
        let fixture = crate::xapi::inventory_tests::Fixture::new({
            let reads = reads.clone();
            move |request| {
                let record = json!({"uuid":"00000000-0000-0000-0000-000000000003", "name_label":"Inspection guest", "power_state":"Running",
                    "VCPUs_at_startup":"2","memory_static_max":"1024", "is_a_snapshot":false,"is_a_template":false,"is_control_domain":false,
                    "allowed_operations":[],"current_operations":{},"blocked_operations":{"pause":"Maintenance in progress"}});
                match request["method"].as_str().unwrap() {
                    "event.from" if request["params"][2] == "" => json!({"result":{"token":"fixture","events":[
                        {"class":"pool","operation":"add","ref":"OpaqueRef:pool","snapshot":{"uuid":"00000000-0000-0000-0000-000000000001","name_label":"Inspection pool","master":"OpaqueRef:host"}},
                        {"class":"host","operation":"add","ref":"OpaqueRef:host","snapshot":{"uuid":"00000000-0000-0000-0000-000000000002","name_label":"Inspection host","address":"192.0.2.1","enabled":true}},
                        {"class":"VM","operation":"add","ref":"OpaqueRef:vm","snapshot":record}
                    ]}}),
                    "event.from" => json!({"result":{"token":"fixture","events":[]}}),
                    "VM.get_by_uuid" => json!({"result":"OpaqueRef:vm"}),
                    "VM.get_record" => {
                        reads.fetch_add(1,Ordering::SeqCst);
                        json!({"result":record})
                    }
                    "VM.assert_operation_valid" => json!({"error":{"message":"VM_BAD_POWER_STATE","data":["secret-error-parameter"]}}),
                    "session.logout" => json!({"result":null}),
                    method => panic!("Unexpected call, including any mutation: {method}"),
                }
            }
        }).await;
        let backend = crate::Backend::with_state_directory(directory.path().join("backend")).unwrap();
        let mut options = fixture.options();
        options.read_only = true;
        let connection = backend.connect(options, crate::Credentials::new("fixture".into(),"private-password".into()));
        connection.wait_ready(Duration::from_secs(3)).await.unwrap();
        let id = ProfileId::new();
        server.set_connections([(id,connection.clone())]);
        let vm = VmId::parse("00000000-0000-0000-0000-000000000003").unwrap();
        let request = InspectionRequest::DiagnosePower {connection:Some(id),vm};
        let report = inspect_live(server.socket_path(),&request).await.unwrap();
        assert_eq!(report["complete"],true);
        let pause = report["power_actions"].as_array().unwrap().iter().find(|row| row["action"]=="Pause").unwrap();
        assert_eq!(pause["availability"]["Blocked"]["reason"],"Maintenance in progress");
        assert_eq!(reads.load(Ordering::SeqCst),9);
        let encoded = report.to_string();
        for forbidden in ["OpaqueRef:","private-password","secret-error-parameter","192.0.2.1"] {
            assert!(!encoded.contains(forbidden));
        }
        connection.close().await;
        assert!(inspect_live(server.socket_path(),&request).await.is_err());
        server.set_connections([]);
    });
}

#[test]
fn resource_history_projection_excludes_private_rpc_evidence_and_intent() {
    let mut record:crate::ResourceOperationRecord=serde_json::from_value(json!({
        "id":uuid::Uuid::new_v4(),"pool":uuid::Uuid::new_v4(),"target":{"Network":uuid::Uuid::new_v4()},
        "action":{"CreateInternalNetwork":{"name":"private-name","description":"private-description","mtu":1500}},
        "state":"OutcomeUnknown","progress":null,"created_at_ms":0,"detail":"Lost reply OpaqueRef:fixture-secret",
        "can_cancel":false,"cancel_requested":false,"steps":[{"label":"Create network","state":"OutcomeUnknown","detail":"Uncertain"}],"created":null
    })).unwrap();
    record.task = Some("OpaqueRef:task-secret".into());
    record.result_reference = Some("OpaqueRef:result-secret".into());
    let value = resource_operation_value(&record);
    assert_eq!(value["state"], "OutcomeUnknown");
    assert_eq!(value["steps"][0]["label"], "Create network");
    let vm = uuid::Uuid::new_v4().into();
    let selection = crate::VmPowerBatch {
        action: crate::PowerAction::Start,
        vms: vec![crate::BatchVmPower {
            vm,
            expected_state: crate::PowerState::Halted,
        }],
    };
    record.action = crate::ResourceAction::PowerBatch(selection.clone());
    record.power_batch_plan = Some(Box::new(crate::power_batch::Plan::new(selection)));
    record.power_batch_plan.as_mut().unwrap().items[0].task =
        Some("OpaqueRef:batch-private-task".into());
    let value = resource_operation_value(&record);
    assert_eq!(value["power_batch"]["vms"][0]["vm"], vm.to_string());
    assert_eq!(value["power_batch"]["action"], "Start");
    assert!(!value.to_string().contains("batch-private-task"));
    assert!(!value.to_string().contains("power_batch_plan"));
    record.power_batch_plan = None;
    record.action = crate::ResourceAction::PropertiesBatch(crate::VmPropertiesBatch {
        vms: vec![crate::BatchVmProperties {
            vm,
            expected_state: crate::PowerState::Halted,
            patch: crate::VmSettingsPatch::new(vec![crate::VmSettingEdit::Description {
                previous: "private-description".into(),
                value: "another-private-description".into(),
            }])
            .unwrap(),
        }],
    });
    let value = resource_operation_value(&record);
    assert_eq!(value["properties_batch"]["vms"][0]["vm"], vm.to_string());
    assert_eq!(value["properties_batch"]["vms"][0]["setters"], 1);
    assert!(!value.to_string().contains("properties_batch_plan"));
    for forbidden in [
        "private-name",
        "private-description",
        "OpaqueRef:",
        "task-secret",
        "result-secret",
        "fixture-secret",
    ] {
        assert!(!value.to_string().contains(forbidden));
    }
}
