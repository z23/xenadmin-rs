use super::*;
use xenadmin_core::{ConsoleState, ResourceTarget, VmId};
const HOST: &str = "22222222-2222-2222-2222-222222222222";
const VM: &str = "33333333-3333-3333-3333-333333333333";
fn domain() -> Value {
    json!({"uuid":VM,"name_label":"Synthetic control domain","power_state":"Running","is_control_domain":true,
        "is_a_template":false,"is_a_snapshot":false,"resident_on":"OpaqueRef:host","VCPUs_at_startup":"2","memory_static_max":"2147483648"})
}
pub(super) async fn server(location: Option<String>, fault: u8) -> Mock {
    mock_with_overrides(false, false, location, false, Some(override_rpc(move |request| async move {
        let method = request["method"].as_str().unwrap();
        let value = match method {
            "event.from" if request["params"][2] == "" => {
                let mut initial = batch("initial", "Running");
                initial["events"][1]["snapshot"]["control_domain"] = json!("OpaqueRef:vm");
                initial["events"][2]["snapshot"] = domain();
                initial
            },
            "host.get_by_uuid" => { assert_eq!(request["params"][1], HOST); if fault == 8 { json!(format!("OpaqueRef:{}", "x".repeat(1025))) } else { json!("OpaqueRef:host") } },
            "host.get_record" => json!({"uuid":HOST}),
            "host.get_control_domain" => json!(if fault == 10 { "OpaqueRef:NULL" } else { "OpaqueRef:vm" }),
            "VM.get_consoles" if fault == 9 => json!(vec!["OpaqueRef:console"; 33]),
            "VM.get_record" => {
                let mut record = domain();
                match fault {
                    1 => record["uuid"] = json!("44444444-4444-4444-4444-444444444444"),
                    2 => record["resident_on"] = json!("OpaqueRef:another-host"),
                    3 => record["is_control_domain"] = json!(false),
                    4 => record["is_a_template"] = json!(true),
                    5 => record["power_state"] = json!("Halted"),
                    7 => record["is_a_snapshot"] = json!(true),
                    _ => {},
                }
                record
            },
            "console.get_record" if fault == 6 => json!({"protocol":"rfb","VM":"OpaqueRef:another-vm","location":"http://127.0.0.1/console"}),
            _ => return None,
        };
        Some(RpcReply::Json(json!({"result":value})))
    }))).await
}
#[tokio::test]
async fn host_console_rejects_changed_control_identity_ownership_and_console_graph_before_tunneling()
 {
    for fault in 1..=10 {
        let server = server(None, fault).await;
        let directory = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(directory.path().into()).unwrap();
        let connection = backend.connect(
            ConnectionOptions::new(server.endpoint.clone()),
            Credentials::new("root".into(), "fixture".into()),
        );
        let inventory = connection.wait_ready(deadline(3)).await.unwrap();
        let host = *inventory.hosts.keys().next().unwrap();
        let console = connection
            .open_host_console(
                connection
                    .latest()
                    .observe_resource(ResourceTarget::Host(host))
                    .unwrap(),
                VmId::parse(VM).unwrap(),
            )
            .unwrap();
        tokio::time::timeout(deadline(3), async {
            loop {
                if matches!(console.latest().state, ConsoleState::Failed(_)) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        let methods = server.methods.lock().unwrap().clone();
        assert_eq!(
            methods.contains(&"VM.get_consoles".into()),
            fault == 6 || fault == 9
        );
        assert_eq!(server.mutations.load(Ordering::SeqCst), 0);
        connection.close().await;
    }
}
#[tokio::test]
async fn host_console_admission_requires_matching_control_domain_and_writable_current_connection() {
    for read_only in [false, true] {
        let server = server(None, 0).await;
        let directory = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(directory.path().into()).unwrap();
        let mut options = ConnectionOptions::new(server.endpoint.clone());
        options.read_only = read_only;
        let connection =
            backend.connect(options, Credentials::new("root".into(), "fixture".into()));
        let inventory = connection.wait_ready(deadline(3)).await.unwrap();
        let host = *inventory.hosts.keys().next().unwrap();
        let snapshot = connection.latest();
        let observed = snapshot
            .observe_resource(ResourceTarget::Host(host))
            .unwrap();
        let vm = VmId::parse(VM).unwrap();
        assert!(
            connection
                .open_console(snapshot.observe_vm(vm).unwrap())
                .is_err()
        );
        assert!(
            connection
                .open_host_console(observed, VmId::from(uuid::Uuid::new_v4()))
                .is_err()
        );
        if read_only {
            assert!(matches!(
                connection.open_host_console(observed, vm),
                Err(xenadmin_core::Error::ReadOnly)
            ));
        }
        connection.close().await;
        assert!(connection.open_host_console(observed, vm).is_err());
        assert!(
            !server
                .methods
                .lock()
                .unwrap()
                .iter()
                .any(|m| m == "host.get_control_domain")
        );
    }
}
#[tokio::test]
async fn console_close_cancels_pending_discovery_without_opening_a_late_tunnel() {
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let entered_rpc = entered.clone();
    let release_rpc = release.clone();
    let server = mock_overridden(override_rpc(move |request| {
        let entered = entered_rpc.clone();
        let release = release_rpc.clone();
        async move {
            if request["method"] != "VM.get_consoles" {
                return None;
            }
            entered.notify_one();
            release.notified().await;
            Some(RpcReply::Json(json!({"result":["OpaqueRef:console"]})))
        }
    }))
    .await;
    let directory = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(directory.path().into()).unwrap();
    let connection = backend.connect(
        ConnectionOptions::new(server.endpoint.clone()),
        Credentials::new("root".into(), "fixture".into()),
    );
    let inventory = connection.wait_ready(deadline(3)).await.unwrap();
    let console = connection
        .open_console(
            connection
                .latest()
                .observe_vm(inventory.guests().next().unwrap().id)
                .unwrap(),
        )
        .unwrap();
    tokio::time::timeout(deadline(3), entered.notified())
        .await
        .unwrap();
    console.close();
    tokio::time::timeout(deadline(3), async {
        while console.latest().state != ConsoleState::Closed {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    release.notify_one();
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(
        !server
            .methods
            .lock()
            .unwrap()
            .iter()
            .any(|m| m == "console.get_record")
    );
    connection.close().await;
}
