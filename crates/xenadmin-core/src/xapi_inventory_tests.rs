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
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let options = ConnectionOptions::new(
            crate::Endpoint::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap(),
        );
        let handler = Arc::new(handler);
        let server = tokio::spawn(async move {
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let handler = handler.clone();
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
