use serde_json::{Value, json};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
};
use xenadmin_core::{
    Backend, ConnectionOptions, ConnectionState, Credentials, Endpoint, PowerState,
};

fn batch(token: &str, state: &str) -> Value {
    json!({"token":token,"events":[
        {"class":"pool","operation":"add","ref":"OpaqueRef:pool","snapshot":{"uuid":"11111111-1111-1111-1111-111111111111","name_label":"Fixture pool","master":"OpaqueRef:host"}},
        {"class":"host","operation":"add","ref":"OpaqueRef:host","snapshot":{"uuid":"22222222-2222-2222-2222-222222222222","name_label":"Fixture host","address":"192.0.2.10","enabled":true}},
        {"class":"VM","operation":"mod","ref":"OpaqueRef:vm","snapshot":{"uuid":"33333333-3333-3333-3333-333333333333","name_label":"XO recovery guest","power_state":state,"VCPUs_at_startup":"2","memory_static_max":"2147483648","allowed_operations":["start","clean_shutdown"]}}
    ]})
}

struct Mock {
    endpoint: Endpoint,
    methods: Arc<Mutex<Vec<String>>>,
    logins: Arc<AtomicUsize>,
    mutations: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Mock {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn mock(expire_once: bool, denied: bool) -> Mock {
    mock_console(expire_once, denied, None).await
}
async fn mock_console(expire_once: bool, denied: bool, console_location: Option<String>) -> Mock {
    mock_full(expire_once, denied, console_location, false).await
}
async fn mock_full(
    expire_once: bool,
    denied: bool,
    console_location: Option<String>,
    drop_mutation_reply: bool,
) -> Mock {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = Endpoint::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
    let methods = Arc::new(Mutex::new(Vec::new()));
    let logins = Arc::new(AtomicUsize::new(0));
    let event_calls = Arc::new(AtomicUsize::new(0));
    let pool_uuid = uuid::Uuid::new_v4().to_string();
    let mutations = Arc::new(AtomicUsize::new(0));
    let task = tokio::spawn({
        let methods = methods.clone();
        let logins = logins.clone();
        let mutations = mutations.clone();
        async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                let (methods, logins, event_calls) =
                    (methods.clone(), logins.clone(), event_calls.clone());
                let console_location = console_location.clone();
                let pool_uuid = pool_uuid.clone();
                let mutations = mutations.clone();
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
                        if let Some((name, value)) = line.split_once(':')
                            && name.eq_ignore_ascii_case("content-length")
                        {
                            length = value.trim().parse().unwrap();
                        }
                    }
                    assert!(length < 16384);
                    let mut bytes = vec![0; length];
                    reader.read_exact(&mut bytes).await.unwrap();
                    let request: Value = serde_json::from_slice(&bytes).unwrap();
                    let method = request["method"].as_str().unwrap();
                    methods.lock().unwrap().push(method.into());
                    let result = match method {
                        "session.login_with_password" => {
                            logins.fetch_add(1, Ordering::SeqCst);
                            if denied {
                                json!({"error":{"code":1,"message":"SESSION_AUTHENTICATION_FAILED","data":["never-return-this-password"]}})
                            } else {
                                json!({"result":"OpaqueRef:fixture-session"})
                            }
                        }
                        "event.from" => {
                            let n = event_calls.fetch_add(1, Ordering::SeqCst);
                            if expire_once && n == 1 {
                                tokio::time::sleep(Duration::from_millis(100)).await;
                                json!({"error":{"code":1,"message":"SESSION_INVALID","data":[]}})
                            } else if request["params"][2] == "" {
                                let mut initial =
                                    batch("initial", if n == 0 { "Halted" } else { "Running" });
                                initial["events"][0]["snapshot"]["uuid"] = json!(pool_uuid);
                                json!({"result":initial})
                            } else {
                                assert_eq!(request["params"][2], "initial");
                                tokio::time::sleep(Duration::from_secs(60)).await;
                                json!({"result":{"token":"initial","events":[]}})
                            }
                        }
                        "Async.VM.start" => {
                            mutations.fetch_add(1, Ordering::SeqCst);
                            if drop_mutation_reply {
                                return;
                            }
                            json!({"result":"OpaqueRef:task"})
                        }
                        "task.get_record" => {
                            json!({"result":{"uuid":"44444444-4444-4444-4444-444444444444","status":"success","progress":1.0,"allowed_operations":[]}})
                        }
                        "session.logout" => json!({"result":null}),
                        "VM.get_consoles" => json!({"result":["OpaqueRef:console"]}),
                        "console.get_record" => {
                            json!({"result":{"protocol":"rfb","location":console_location}})
                        }
                        other => panic!("Unexpected API method: {other}"),
                    };
                    let mut response = result;
                    response["id"] = request["id"].clone();
                    response["jsonrpc"] = json!("2.0");
                    let body = serde_json::to_vec(&response).unwrap();
                    let header = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let mut stream = reader.into_inner();
                    if stream.write_all(header.as_bytes()).await.is_ok() {
                        let _ = stream.write_all(&body).await;
                    }
                });
            }
        }
    });
    Mock {
        endpoint,
        methods,
        logins,
        mutations,
        task,
    }
}

#[tokio::test]
async fn direct_login_bootstrap_and_logout_need_no_xo() {
    let server = mock(false, false).await;
    let backend = Backend::start().unwrap();
    let connection = backend.connect(
        ConnectionOptions::new(server.endpoint.clone()),
        Credentials::new("root".into(), "fixture-password".into()),
    );
    let inventory = connection.wait_ready(Duration::from_secs(3)).await.unwrap();
    assert_eq!(inventory.pool_name, "Fixture pool");
    let vm = inventory.guests().next().unwrap();
    assert_eq!(vm.power_state, PowerState::Halted);
    assert_eq!(vm.memory_bytes, 2_147_483_648);
    assert_eq!(vm.vcpus, 2);
    connection.close().await;
    assert_eq!(connection.latest().state(), ConnectionState::Disconnected);
    assert_eq!(
        server.methods.lock().unwrap().last().unwrap(),
        "session.logout"
    );
}

#[tokio::test]
async fn session_expiry_resynchronizes_in_a_new_generation() {
    let server = mock(true, false).await;
    let backend = Backend::start().unwrap();
    let connection = backend.connect(
        ConnectionOptions::new(server.endpoint.clone()),
        Credentials::new("root".into(), "fixture-password".into()),
    );
    let initial = connection.wait_ready(Duration::from_secs(3)).await.unwrap();
    let first_generation = connection.latest().generation();
    let observed = connection
        .latest()
        .observe_vm(initial.guests().next().unwrap().id)
        .unwrap();
    let mut receiver = connection.watch();
    tokio::time::timeout(Duration::from_secs(6), async {
        loop {
            {
                let snapshot = receiver.borrow_and_update();
                if snapshot.generation() != first_generation && snapshot.is_current() {
                    assert_eq!(
                        snapshot
                            .inventory()
                            .unwrap()
                            .guests()
                            .next()
                            .unwrap()
                            .power_state,
                        PowerState::Running
                    );
                    break;
                }
            }
            receiver.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    assert_eq!(server.logins.load(Ordering::SeqCst), 2);
    assert!(matches!(
        connection.power_action(
            xenadmin_core::OperationId::new(),
            observed,
            xenadmin_core::PowerAction::Start
        ),
        Err(xenadmin_core::Error::Stale)
    ));
    connection.close().await;
}

#[tokio::test]
async fn a_vm_observation_cannot_be_used_in_another_pool_with_the_same_vm_uuid() {
    let first = mock(false, false).await;
    let second = mock(false, false).await;
    let backend = Backend::start().unwrap();
    let a = backend.connect(
        ConnectionOptions::new(first.endpoint.clone()),
        Credentials::new("root".into(), "fixture".into()),
    );
    let b = backend.connect(
        ConnectionOptions::new(second.endpoint.clone()),
        Credentials::new("root".into(), "fixture".into()),
    );
    let inventory_a = a.wait_ready(Duration::from_secs(3)).await.unwrap();
    let inventory_b = b.wait_ready(Duration::from_secs(3)).await.unwrap();
    assert_ne!(inventory_a.pool_id, inventory_b.pool_id);
    let observed = a
        .latest()
        .observe_vm(inventory_a.guests().next().unwrap().id)
        .unwrap();
    assert!(matches!(
        b.power_action(
            xenadmin_core::OperationId::new(),
            observed,
            xenadmin_core::PowerAction::Start
        ),
        Err(xenadmin_core::Error::Stale)
    ));
    assert_eq!(second.mutations.load(Ordering::SeqCst), 0);
    a.close().await;
    b.close().await;
}

#[tokio::test]
async fn reconnecting_the_same_pool_rejects_the_previous_connection_observation() {
    let server = mock(false, false).await;
    let backend = Backend::start().unwrap();
    let a = backend.connect(
        ConnectionOptions::new(server.endpoint.clone()),
        Credentials::new("root".into(), "fixture".into()),
    );
    let inventory = a.wait_ready(Duration::from_secs(3)).await.unwrap();
    let observed = a
        .latest()
        .observe_vm(inventory.guests().next().unwrap().id)
        .unwrap();
    a.close().await;
    let b = backend.connect(
        ConnectionOptions::new(server.endpoint.clone()),
        Credentials::new("root".into(), "fixture".into()),
    );
    assert_eq!(
        b.wait_ready(Duration::from_secs(3)).await.unwrap().pool_id,
        inventory.pool_id
    );
    assert!(matches!(
        b.power_action(
            xenadmin_core::OperationId::new(),
            observed,
            xenadmin_core::PowerAction::Start
        ),
        Err(xenadmin_core::Error::Stale)
    ));
    assert!(matches!(
        b.open_console(observed),
        Err(xenadmin_core::Error::Stale)
    ));
    assert_eq!(server.mutations.load(Ordering::SeqCst), 0);
    b.close().await;
}

#[tokio::test]
async fn authentication_failure_is_terminal_and_does_not_expose_error_data() {
    let server = mock(false, true).await;
    let backend = Backend::start().unwrap();
    let connection = backend.connect(
        ConnectionOptions::new(server.endpoint.clone()),
        Credentials::new("root".into(), "fixture-password".into()),
    );
    let error = connection
        .wait_ready(Duration::from_secs(3))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("SESSION_AUTHENTICATION_FAILED"));
    assert!(!error.contains("never-return-this-password"));
    assert_eq!(server.logins.load(Ordering::SeqCst), 1);
    assert_eq!(connection.latest().state(), ConnectionState::Failed);
}

#[tokio::test]
async fn authenticated_console_preserves_the_handshake_and_transmits_input() {
    use xenadmin_core::{ConsoleInput, ConsoleState};
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let location = format!(
        "http://{}/console?ref=OpaqueRef:console",
        listener.local_addr().unwrap()
    );
    let keys = Arc::new(Mutex::new(Vec::new()));
    let observed_keys = keys.clone();
    let pointer_count = Arc::new(AtomicUsize::new(0));
    let observed_pointers = pointer_count.clone();
    let rfb = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut header = Vec::new();
        while !header.ends_with(b"\r\n\r\n") {
            header.push(stream.read_u8().await.unwrap());
            assert!(header.len() < 16384);
        }
        let header = String::from_utf8(header).unwrap();
        assert!(header.starts_with("CONNECT /console?ref=OpaqueRef:console HTTP/1.0\r\n"));
        assert!(header.contains("Cookie: session_id=OpaqueRef:fixture-session\r\n"));
        stream
            .write_all(b"HTTP/1.0 200 OK\r\n\r\nRFB 003.003\n")
            .await
            .unwrap();
        let mut version = [0; 12];
        stream.read_exact(&mut version).await.unwrap();
        assert_eq!(&version, b"RFB 003.003\n");
        stream.write_u32(1).await.unwrap();
        assert_eq!(stream.read_u8().await.unwrap(), 1);
        stream
            .write_all(&[
                0, 2, 0, 1, 32, 24, 0, 1, 0, 255, 0, 255, 0, 255, 0, 8, 16, 0, 0, 0, 0, 0, 0, 4,
                b'T', b'e', b's', b't',
            ])
            .await
            .unwrap();
        let mut sent = false;
        while let Ok(kind) = stream.read_u8().await {
            match kind {
                0 => {
                    let mut format = [0; 19];
                    stream.read_exact(&mut format).await.unwrap();
                }
                2 => {
                    stream.read_u8().await.unwrap();
                    let count = stream.read_u16().await.unwrap();
                    let mut encodings = vec![0; usize::from(count) * 4];
                    stream.read_exact(&mut encodings).await.unwrap();
                }
                3 => {
                    let mut request = [0; 9];
                    stream.read_exact(&mut request).await.unwrap();
                    if !sent {
                        stream
                            .write_all(&[
                                0, 0, 0, 1, 0, 0, 0, 0, 0, 2, 0, 1, 0, 0, 0, 0, 255, 0, 0, 0, 0,
                                255, 0, 0,
                            ])
                            .await
                            .unwrap();
                        sent = true;
                    }
                }
                4 => {
                    let down = stream.read_u8().await.unwrap() != 0;
                    stream.read_u16().await.unwrap();
                    let key = stream.read_u32().await.unwrap();
                    observed_keys.lock().unwrap().push((key, down));
                }
                5 => {
                    let mut pointer = [0; 5];
                    stream.read_exact(&mut pointer).await.unwrap();
                    observed_pointers.fetch_add(1, Ordering::SeqCst);
                }
                other => panic!("Unexpected RFB client message {other}"),
            }
        }
    });
    let server = mock_console(false, false, Some(location)).await;
    let backend = Backend::start().unwrap();
    let connection = backend.connect(
        ConnectionOptions::new(server.endpoint.clone()),
        Credentials::new("root".into(), "fixture-password".into()),
    );
    let inventory = connection.wait_ready(Duration::from_secs(3)).await.unwrap();
    let console = connection
        .open_console(
            connection
                .latest()
                .observe_vm(inventory.guests().next().unwrap().id)
                .unwrap(),
        )
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let snapshot = console.latest();
            if let Some(frame) = &snapshot.frame {
                assert_eq!(snapshot.state, ConsoleState::Connected);
                assert_eq!((frame.width, frame.height), (2, 1));
                assert_eq!(&*frame.rgba, &[255, 0, 0, 255, 0, 255, 0, 255]);
                break;
            }
            if let ConsoleState::Failed(error) = &snapshot.state {
                panic!("{error}");
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    console
        .input(ConsoleInput::Key {
            keysym: 0xff0d,
            down: true,
        })
        .unwrap();
    console
        .input(ConsoleInput::Pointer {
            x: 1,
            y: 0,
            buttons: 1,
        })
        .unwrap();
    console.input(ConsoleInput::ReleaseAll).unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while !keys.lock().unwrap().contains(&(0xff0d, false))
            || pointer_count.load(Ordering::SeqCst) < 2
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    console.close();
    connection.close().await;
    let _ = tokio::time::timeout(Duration::from_secs(3), rfb).await;
}

async fn finish(operation: &xenadmin_core::Operation) -> xenadmin_core::OperationState {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let record = operation.latest();
            if record.state.terminal()
                || record.state == xenadmin_core::OperationState::OutcomeUnknown
            {
                return record.state.clone();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn power_action_tracks_the_host_task_and_deduplicates_submission() {
    use xenadmin_core::{OperationId, OperationState, PowerAction};
    let server = mock(false, false).await;
    let directory = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(directory.path().into()).unwrap();
    let connection = backend.connect(
        ConnectionOptions::new(server.endpoint.clone()),
        Credentials::new("root".into(), "fixture-password".into()),
    );
    let inventory = connection.wait_ready(Duration::from_secs(3)).await.unwrap();
    let vm = connection
        .latest()
        .observe_vm(inventory.guests().next().unwrap().id)
        .unwrap();
    let id = OperationId::new();
    let operation = connection.power_action(id, vm, PowerAction::Start).unwrap();
    assert_eq!(finish(&operation).await, OperationState::Succeeded);
    let duplicate = connection.power_action(id, vm, PowerAction::Start).unwrap();
    assert_eq!(finish(&duplicate).await, OperationState::Succeeded);
    assert_eq!(server.mutations.load(Ordering::SeqCst), 1);
    let database = std::fs::read(directory.path().join("operations.sqlite3")).unwrap();
    assert!(
        !database
            .windows(b"fixture-password".len())
            .any(|bytes| bytes == b"fixture-password")
    );
    assert!(
        !database
            .windows(b"fixture-session".len())
            .any(|bytes| bytes == b"fixture-session")
    );
    connection.close().await;
}

#[tokio::test]
async fn lost_mutation_reply_remains_unknown_and_blocks_a_second_mutation() {
    use xenadmin_core::{OperationId, OperationState, PowerAction};
    let server = mock_full(false, false, None, true).await;
    let directory = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(directory.path().into()).unwrap();
    let connection = backend.connect(
        ConnectionOptions::new(server.endpoint.clone()),
        Credentials::new("root".into(), "fixture-password".into()),
    );
    let inventory = connection.wait_ready(Duration::from_secs(3)).await.unwrap();
    let vm = connection
        .latest()
        .observe_vm(inventory.guests().next().unwrap().id)
        .unwrap();
    let operation = connection
        .power_action(OperationId::new(), vm, PowerAction::Start)
        .unwrap();
    assert_eq!(finish(&operation).await, OperationState::OutcomeUnknown);
    let second = connection
        .power_action(OperationId::new(), vm, PowerAction::Start)
        .unwrap();
    assert_eq!(finish(&second).await, OperationState::Rejected);
    assert_eq!(server.mutations.load(Ordering::SeqCst), 1);
    connection.close().await;
    drop(backend);
    let backend = Backend::with_state_directory(directory.path().into()).unwrap();
    let connection = backend.connect(
        ConnectionOptions::new(server.endpoint.clone()),
        Credentials::new("root".into(), "fixture-password".into()),
    );
    connection.wait_ready(Duration::from_secs(3)).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if connection
                .operation_history()
                .iter()
                .any(|record| record.state == OperationState::OutcomeUnknown)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        server.mutations.load(Ordering::SeqCst),
        1,
        "restart must not replay the mutation"
    );
    connection.close().await;
}
