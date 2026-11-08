use super::*;
use crate::{ActionAvailability, ClassAvailability, PowerAction};
use std::sync::atomic::AtomicUsize;
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
};

pub(crate) struct Fixture {
    options: ConnectionOptions,
    server: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}
impl Fixture {
    pub(crate) async fn new(handler: impl Fn(Value) -> Value + Send + Sync + 'static) -> Self {
        Self::with_permissions(handler, json!({"is_local_superuser":true})).await
    }
    pub(crate) async fn with_permissions(
        handler: impl Fn(Value) -> Value + Send + Sync + 'static,
        permissions: Value,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let options = ConnectionOptions::new(
            crate::Endpoint::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap(),
        );
        let handler = Arc::new(handler);
        let server = tokio::spawn(async move {
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let handler = handler.clone();
                let permissions = permissions.clone();
                tokio::spawn(async move {
                    let mut reader = BufReader::new(stream);
                    let mut length = 0;
                    let mut line = String::new();
                    loop {
                        line.clear();
                        if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
                            return;
                        }
                        if line == "\r\n" {
                            break;
                        }
                        if let Some((key, value)) = line.split_once(':')
                            && key.eq_ignore_ascii_case("content-length")
                        {
                            length = value.trim().parse().unwrap();
                        }
                    }
                    assert!(length < 16384);
                    let mut data = vec![0; length];
                    reader.read_exact(&mut data).await.unwrap();
                    let request: Value = serde_json::from_slice(&data).unwrap();
                    if request["method"] == "event.from" && request["params"][2] != "" {
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    }
                    let mut response = if request["method"] == "session.login_with_password" {
                        json!({"result":"OpaqueRef:fixture-session"})
                    } else if request["method"] == "session.get_record" {
                        json!({"result":permissions})
                    } else {
                        handler(request.clone())
                    };
                    if response["_fixture_close_without_reply"] == true {
                        return;
                    }
                    response["id"] = request["id"].clone();
                    response["jsonrpc"] = json!("2.0");
                    let body = serde_json::to_vec(&response).unwrap();
                    let header = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let mut stream = reader.into_inner();
                    let _ = stream.write_all(header.as_bytes()).await;
                    let _ = stream.write_all(&body).await;
                });
            }
        });
        Self { options, server }
    }
    pub(crate) async fn session(&self) -> Session {
        Session::login(
            &self.options,
            &Credentials::new("fixture".into(), "fixture-password".into()),
        )
        .await
        .unwrap()
    }
    pub(crate) fn options(&self) -> ConnectionOptions {
        self.options.clone()
    }
}
fn error(code: &str) -> Value {
    json!({"error":{"message":code,"data":["private-error-detail"]}})
}

#[test]
fn explicit_blocks_override_advertised_vm_and_snapshot_actions() {
    use crate::{OperationAction, SnapshotAction, SnapshotOperation};
    let parent = json!({"uuid":"00000000-0000-0000-0000-000000000002", "power_state":"Halted",
        "VCPUs_at_startup":2,"memory_static_max":1073741824,
        "is_a_snapshot":false,"is_a_template":false,"is_control_domain":false,
        "allowed_operations":["start","snapshot"],"blocked_operations":{"start":"Protected","snapshot":"Protected"}});
    let mut snapshot = json!({"uuid":"00000000-0000-0000-0000-000000000003", "power_state":"Halted",
        "is_a_snapshot":true,"is_a_template":true,"is_control_domain":false,"snapshot_of":"OpaqueRef:parent",
        "allowed_operations":["clone","revert","destroy"],"blocked_operations":{"clone":"Protected","revert":"Protected","destroy":"Protected"}});
    let batch: EventBatch = serde_json::from_value(json!({"token":"one", "events":[
        {"class":"pool","operation":"add","ref":"OpaqueRef:pool","snapshot":{"uuid":"00000000-0000-0000-0000-000000000001"}},
        {"class":"VM","operation":"add","ref":"OpaqueRef:parent","snapshot":parent},
        {"class":"VM","operation":"add","ref":"OpaqueRef:snapshot","snapshot":snapshot}
    ]})).unwrap();
    let mut store = Store::default();
    let inventory = store.apply(&batch).unwrap();
    let vm = VmId::parse("00000000-0000-0000-0000-000000000002").unwrap();
    let id = crate::SnapshotId::parse("00000000-0000-0000-0000-000000000003").unwrap();
    assert!(inventory.vms[&vm].allowed_actions.is_empty());
    assert!(!inventory.vms[&vm].can_snapshot);
    assert!(
        inventory
            .snapshots
            .get(&id)
            .unwrap()
            .allowed_actions
            .is_empty()
    );
    for action in [
        OperationAction::Power(PowerAction::Start),
        OperationAction::Snapshot(SnapshotOperation::Create {
            name: "Blocked".into(),
        }),
        OperationAction::Snapshot(SnapshotOperation::Clone {
            snapshot: id,
            name: "Blocked".into(),
            as_template: false,
        }),
        OperationAction::Snapshot(SnapshotOperation::Revert { snapshot: id }),
        OperationAction::Snapshot(SnapshotOperation::Delete {
            snapshot: id,
            disks: vec![],
        }),
    ] {
        assert!(!store.allows(vm, &action));
    }
    snapshot["blocked_operations"] = json!({});
    for (state, allowed, expected) in [
        ("Halted", "hard_shutdown", false),
        ("Suspended", "destroy", false),
        ("Running", "destroy", false),
        ("Halted", "destroy", true),
        ("Suspended", "hard_shutdown", true),
    ] {
        snapshot["power_state"] = json!(state);
        snapshot["allowed_operations"] = json!([allowed]);
        let update: EventBatch = serde_json::from_value(json!({"token":"next","events":[
            {"class":"VM","operation":"mod","ref":"OpaqueRef:snapshot","snapshot":snapshot}
        ]}))
        .unwrap();
        let inventory = store.apply(&update).unwrap();
        assert_eq!(
            inventory
                .snapshots
                .get(&id)
                .unwrap()
                .allowed_actions
                .contains(&SnapshotAction::Delete),
            expected,
            "{state}: {allowed}"
        );
        assert_eq!(
            store.allows(
                vm,
                &OperationAction::Snapshot(SnapshotOperation::Delete {
                    snapshot: id,
                    disks: vec![]
                })
            ),
            expected
        );
    }
}

#[tokio::test]
async fn blocked_snapshot_clone_is_rejected_before_any_host_write() {
    use crate::{OperationAction, OperationId, OperationState, SnapshotOperation};
    let writes = Arc::new(AtomicUsize::new(0));
    let fixture = Fixture::new({
        let writes = writes.clone();
        move |request| {
            let parent = json!({"uuid":"00000000-0000-0000-0000-000000000002", "power_state":"Halted",
                "is_a_snapshot":false,"is_a_template":false,"is_control_domain":false});
            let snapshot = json!({"uuid":"00000000-0000-0000-0000-000000000003", "power_state":"Halted",
                "is_a_snapshot":true,"is_a_template":true,"is_control_domain":false,"snapshot_of":"OpaqueRef:parent",
                "allowed_operations":["clone"],"blocked_operations":{"clone":"Protected recovery point"}});
            match request["method"].as_str().unwrap() {
                "VM.get_by_uuid" => json!({"result":if request["params"][1] == parent["uuid"] {"OpaqueRef:parent"} else {"OpaqueRef:snapshot"}}),
                "VM.get_record" => json!({"result":if request["params"][1] == "OpaqueRef:parent" {parent} else {snapshot}}),
                "VM.get_all_records" => json!({"result":{"OpaqueRef:parent":parent,"OpaqueRef:snapshot":snapshot}}),
                _ => { writes.fetch_add(1, Ordering::SeqCst); json!({"result":null}) }
            }
        }
    }).await;
    let directory = tempfile::tempdir().unwrap();
    let (operation, request) = crate::operations::channel(
        OperationId::new(),
        PoolId::parse("00000000-0000-0000-0000-000000000001").unwrap(),
        VmId::parse("00000000-0000-0000-0000-000000000002").unwrap(),
        1,
        OperationAction::Snapshot(SnapshotOperation::Clone {
            snapshot: crate::SnapshotId::parse("00000000-0000-0000-0000-000000000003").unwrap(),
            name: "Blocked clone".into(),
            as_template: false,
        }),
        tokio::runtime::Handle::current(),
    );
    let (updates, _receiver) = tokio::sync::mpsc::channel(16);
    request
        .run(
            Arc::new(fixture.session().await),
            Some("OpaqueRef:parent".into()),
            directory.path().into(),
            tokio_util::sync::CancellationToken::new(),
            updates,
        )
        .await;
    assert_eq!(operation.latest().state, OperationState::Rejected);
    assert_eq!(writes.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn optional_classes_are_negotiated_without_losing_base_inventory() {
    let calls = Arc::new(AtomicUsize::new(0));
    let fixture = Fixture::new({
        let calls = calls.clone();
        move |request| {
            let method = request["method"].as_str().unwrap();
            match method {
                "event.from" => {
                    let n = calls.fetch_add(1, Ordering::SeqCst);
                    let classes = request["params"][1].as_array().unwrap();
                    if n == 0 {
                        assert!(classes.contains(&json!("VMSS")));
                        return error("EVENT_SUBSCRIPTION_PARSE_FAILURE");
                    }
                    assert!(!classes.contains(&json!("VMSS")));
                    assert!(!classes.contains(&json!("task")));
                    assert!(classes.contains(&json!("VDI")));
                    assert_eq!(request["params"][2], if n == 1 { "" } else { "negotiated" });
                    json!({"result":{"token":"negotiated","events":[
                        {"class":"pool","operation":"add","ref":"OpaqueRef:pool","snapshot":{"uuid":"00000000-0000-0000-0000-000000000001"}},
                        {"class":"VDI","operation":"add","ref":"OpaqueRef:disk","snapshot":{"uuid":"00000000-0000-0000-0000-000000000002","virtual_size":"-1"}}
                    ]}})
                }
                "VMSS.get_all_records" => error("MESSAGE_METHOD_UNKNOWN"),
                "task.get_all_records" => error("RBAC_PERMISSION_DENIED"),
                method if method.ends_with(".get_all_records") => json!({"result":{}}),
                _ => panic!("unexpected fixture request"),
            }
        }
    }).await;
    let session = fixture.session().await;
    let batch = session.events("old-token").await.unwrap();
    assert!(batch.reset);
    let inventory = Store::default().apply(&batch).unwrap();
    assert_eq!(
        inventory.details.classes["vmss"],
        ClassAvailability::Unsupported
    );
    assert_eq!(
        inventory.details.classes["task"],
        ClassAvailability::PermissionDenied
    );
    assert_eq!(inventory.details.vdis.len(), 1);
    let next = session.events(&batch.token).await.unwrap();
    assert!(!next.reset);
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn power_diagnostic_preserves_administrator_message_without_mutating_the_vm() {
    let fixture = Fixture::new(|request| match request["method"].as_str().unwrap() {
        "VM.get_by_uuid" => json!({"result":"OpaqueRef:current-vm"}),
        "VM.get_record" => json!({"result":{
            "uuid":"00000000-0000-0000-0000-000000000002",
            "is_a_snapshot":false,"is_a_template":false,"is_control_domain":false,
            "allowed_operations":[],
            "blocked_operations":{
                "clean_shutdown":"Maintenance: contact the VM owner (équipe).",
                "hard_shutdown":""
            }
        }}),
        _ => panic!("diagnosing a blocked action must only read its current VM record"),
    })
    .await;
    let session = fixture.session().await;
    let vm = VmId::parse("00000000-0000-0000-0000-000000000002").unwrap();
    for (action, reason) in [
        (
            PowerAction::CleanShutdown,
            "Maintenance: contact the VM owner (équipe).",
        ),
        (PowerAction::HardShutdown, "No reason supplied by the host"),
    ] {
        assert_eq!(
            session.diagnose_power(vm, action).await.unwrap(),
            ActionAvailability::Blocked {
                reason: reason.into()
            }
        );
    }
}

#[tokio::test]
async fn power_diagnostic_reads_current_identity_and_preserves_host_rejections() {
    let checks = Arc::new(AtomicUsize::new(0));
    let fixture = Fixture::new({
        let checks = checks.clone();
        move |request| match request["method"].as_str().unwrap() {
            "VM.get_by_uuid" => json!({"result":"OpaqueRef:current-vm"}),
            "VM.get_record" => {
                assert_eq!(request["params"][1], "OpaqueRef:current-vm");
                json!({"result":{"uuid":"00000000-0000-0000-0000-000000000002",
                    "is_a_snapshot":false,"is_a_template":false,"is_control_domain":false,
                    "allowed_operations":[],"current_operations":{"OpaqueRef:task":"snapshot"}}})
            }
            "VM.assert_operation_valid" => {
                checks.fetch_add(1, Ordering::SeqCst);
                match request["params"][2].as_str().unwrap() {
                    "start" => json!({"result":null}),
                    "clean_reboot" => error("RBAC_PERMISSION_DENIED"),
                    "clean_shutdown" => error("VM_BAD_POWER_STATE"),
                    "hard_shutdown" => error("MESSAGE_METHOD_UNKNOWN"),
                    "suspend" => error("OTHER_OPERATION_IN_PROGRESS"),
                    _ => error("OPERATION_NOT_ALLOWED"),
                }
            }
            _ => panic!("unexpected fixture request"),
        }
    })
    .await;
    let session = fixture.session().await;
    let vm = VmId::parse("00000000-0000-0000-0000-000000000002").unwrap();
    assert_eq!(
        session
            .diagnose_power(vm, PowerAction::Start)
            .await
            .unwrap(),
        ActionAvailability::Available
    );
    assert_eq!(
        session
            .diagnose_power(vm, PowerAction::Reboot)
            .await
            .unwrap(),
        ActionAvailability::PermissionDenied
    );
    assert_eq!(
        session
            .diagnose_power(vm, PowerAction::CleanShutdown)
            .await
            .unwrap(),
        ActionAvailability::Unavailable
    );
    assert_eq!(
        session
            .diagnose_power(vm, PowerAction::HardShutdown)
            .await
            .unwrap(),
        ActionAvailability::Unknown
    );
    assert!(matches!(
        session
            .diagnose_power(vm, PowerAction::Suspend)
            .await
            .unwrap(),
        ActionAvailability::Busy { .. }
    ));
    assert_eq!(
        session
            .diagnose_power(vm, PowerAction::Pause)
            .await
            .unwrap(),
        ActionAvailability::Rejected {
            code: "OPERATION_NOT_ALLOWED".into()
        }
    );
    let different = VmId::parse("00000000-0000-0000-0000-000000000003").unwrap();
    assert!(matches!(
        session.diagnose_power(different, PowerAction::Start).await,
        Err(Error::Stale)
    ));
    assert_eq!(checks.load(Ordering::SeqCst), 6);
}

#[tokio::test]
async fn chosen_host_preflight_checks_fresh_identity_state_and_boot_compatibility() {
    let boot_checks = Arc::new(AtomicUsize::new(0));
    let fixture = Fixture::new({
        let boot_checks = boot_checks.clone();
        move |request| match request["method"].as_str().unwrap() {
            "VM.get_by_uuid" => json!({"result":"OpaqueRef:vm"}),
            "VM.get_record" => json!({"result":{"uuid":"00000000-0000-0000-0000-000000000002",
                "is_a_snapshot":false,"is_a_template":false,"is_control_domain":false,
                "power_state":"Halted","allowed_operations":["start_on"]}}),
            "host.get_by_uuid" => json!({"result":"OpaqueRef:host"}),
            "host.get_record" => {
                json!({"result":{"uuid":"00000000-0000-0000-0000-000000000003","enabled":true}})
            }
            "VM.assert_operation_valid" => {
                assert_eq!(request["params"][2], "start_on");
                json!({"result":null})
            }
            "VM.assert_can_boot_here" => {
                assert_eq!(request["params"][1], "OpaqueRef:vm");
                assert_eq!(request["params"][2], "OpaqueRef:host");
                boot_checks.fetch_add(1, Ordering::SeqCst);
                error("VM_REQUIRES_SR")
            }
            _ => panic!("unexpected fixture request"),
        }
    })
    .await;
    let session = fixture.session().await;
    let vm = VmId::parse("00000000-0000-0000-0000-000000000002").unwrap();
    let host = HostId::parse("00000000-0000-0000-0000-000000000003").unwrap();
    assert!(
        matches!(session.prepare_placement(vm, host, PowerAction::Start).await, Err(Error::Api(code)) if code == "VM_REQUIRES_SR")
    );
    assert!(matches!(
        session
            .prepare_placement(vm, host, PowerAction::Resume)
            .await,
        Err(Error::UnavailableAction)
    ));
    let unrelated_host = HostId::parse("00000000-0000-0000-0000-000000000004").unwrap();
    assert!(matches!(
        session
            .prepare_placement(vm, unrelated_host, PowerAction::Start)
            .await,
        Err(Error::Stale)
    ));
    assert_eq!(boot_checks.load(Ordering::SeqCst), 1);
}
