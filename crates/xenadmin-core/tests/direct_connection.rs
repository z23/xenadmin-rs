#[path = "fixtures/acknowledge.rs"]
mod acknowledge;
#[path = "fixtures/crashdump_deletion.rs"]
mod crashdump_deletion;
use acknowledge::acknowledge_unknown;
#[path = "fixtures/deadlines.rs"]
mod deadlines;
use deadlines::deadline;
#[path = "fixtures/management_network.rs"]
mod management_network;
use serde_json::{Value, json};
#[path = "fixtures/permissions.rs"]
mod permissions;
use std::{future::Future, pin::Pin};
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
enum RpcReply {
    Json(Value),
    Drop,
}
type RpcOverride =
    Arc<dyn Fn(Value) -> Pin<Box<dyn Future<Output = Option<RpcReply>> + Send>> + Send + Sync>;
fn override_rpc<F, Fut>(callback: F) -> RpcOverride
where
    F: Fn(Value) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Option<RpcReply>> + Send + 'static,
{
    Arc::new(move |value| Box::pin(callback(value)))
}
async fn mock_overridden(overrides: RpcOverride) -> Mock {
    mock_with_overrides(false, false, None, false, Some(overrides)).await
}
async fn mock_full(
    expire_once: bool,
    denied: bool,
    console_location: Option<String>,
    drop_mutation_reply: bool,
) -> Mock {
    mock_with_overrides(
        expire_once,
        denied,
        console_location,
        drop_mutation_reply,
        None,
    )
    .await
}
async fn mock_with_overrides(
    expire_once: bool,
    denied: bool,
    console_location: Option<String>,
    drop_mutation_reply: bool,
    overrides: Option<RpcOverride>,
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
                let overrides = overrides.clone();
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
                    let injected = if let Some(callback) = overrides {
                        callback(request.clone()).await
                    } else {
                        None
                    };
                    let result = match injected {
                        Some(RpcReply::Json(response)) => response,
                        Some(RpcReply::Drop) => return,
                        None => match method {
                            "session.get_record" => json!({"result":{"is_local_superuser":true}}),
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
                            "session.logout" | "task.destroy" => json!({"result":null}),
                            "VM.get_consoles" => json!({"result":["OpaqueRef:console"]}),
                            "console.get_record" => {
                                json!({"result":{"protocol":"rfb","location":console_location,"VM":"OpaqueRef:vm"}})
                            }
                            other => panic!("Unexpected API method: {other}"),
                        },
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
    let state_directory = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state_directory.path().into()).unwrap();
    let connection = backend.connect(
        ConnectionOptions::new(server.endpoint.clone()),
        Credentials::new("root".into(), "fixture-password".into()),
    );
    let inventory = connection.wait_ready(deadline(3)).await.unwrap();
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
    let state_directory = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state_directory.path().into()).unwrap();
    let connection = backend.connect(
        ConnectionOptions::new(server.endpoint.clone()),
        Credentials::new("root".into(), "fixture-password".into()),
    );
    let initial = connection.wait_ready(deadline(3)).await.unwrap();
    let first_generation = connection.latest().generation();
    let observed = connection
        .latest()
        .observe_vm(initial.guests().next().unwrap().id)
        .unwrap();
    let mut receiver = connection.watch();
    tokio::time::timeout(deadline(6), async {
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
        connection.submit(
            xenadmin_core::OperationId::new(),
            xenadmin_core::VmCommand::Power {
                vm: observed,
                action: xenadmin_core::PowerAction::Start
            }
        ),
        Err(xenadmin_core::Error::Stale)
    ));
    connection.close().await;
}

#[tokio::test]
async fn delayed_initial_inventory_reports_synchronizing_and_admits_no_command_until_ready() {
    use xenadmin_core::{
        Error, OperationControl, OperationId, OperationState, PowerAction, VmCommand,
    };
    let hold = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let held = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let server = mock_overridden(override_rpc({
        let (hold, held, release) = (hold.clone(), held.clone(), release.clone());
        move |request| {
            let (hold, held, release) = (hold.clone(), held.clone(), release.clone());
            async move {
                if request["method"] == "event.from"
                    && request["params"][2] == ""
                    && hold.load(Ordering::SeqCst)
                {
                    held.notify_one();
                    release.notified().await;
                }
                None
            }
        }
    }))
    .await;
    let directory = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(directory.path().into()).unwrap();
    let connect = || {
        backend.connect(
            ConnectionOptions::new(server.endpoint.clone()),
            Credentials::new("root".into(), "fixture-password".into()),
        )
    };
    // A command needs a guest observation and a synchronizing connection has none to give,
    // so an earlier connection to the same pool supplies one to offer.
    let earlier = connect();
    let guest = earlier
        .wait_ready(deadline(3))
        .await
        .unwrap()
        .guests()
        .next()
        .unwrap()
        .id;
    let observed = earlier.latest().observe_vm(guest).unwrap();
    earlier.close().await;
    let start = |vm| VmCommand::Power {
        vm,
        action: PowerAction::Start,
    };

    hold.store(true, Ordering::SeqCst);
    let connection = connect();
    tokio::time::timeout(deadline(3), held.notified())
        .await
        .expect("the inventory request follows a successful login");
    assert_eq!(server.logins.load(Ordering::SeqCst), 2);
    let snapshot = connection.latest();
    assert_eq!(snapshot.state(), ConnectionState::Synchronizing);
    assert!(!snapshot.is_current());
    assert!(snapshot.inventory().is_none());
    assert!(matches!(snapshot.observe_vm(guest), Err(Error::Stale)));
    assert!(matches!(
        connection.submit(OperationId::new(), start(observed)),
        Err(Error::Stale)
    ));
    assert!(matches!(
        connection.control_operation(OperationId::new(), OperationControl::RequestCancellation),
        Err(Error::Stale)
    ));
    assert!(matches!(
        connection.open_console(observed),
        Err(Error::Stale)
    ));
    assert_eq!(connection.latest().state(), ConnectionState::Synchronizing);
    assert_eq!(server.mutations.load(Ordering::SeqCst), 0);
    assert!(!directory.path().join("operations.sqlite3").exists());

    release.notify_one();
    connection.wait_ready(deadline(3)).await.unwrap();
    let snapshot = connection.latest();
    assert_eq!(snapshot.state(), ConnectionState::Ready);
    let operation = connection
        .submit(
            OperationId::new(),
            start(snapshot.observe_vm(guest).unwrap()),
        )
        .unwrap();
    assert_eq!(finish(&operation).await, OperationState::Succeeded);
    assert_eq!(server.mutations.load(Ordering::SeqCst), 1);
    connection.close().await;
}

#[tokio::test]
async fn a_vm_observation_cannot_be_used_in_another_pool_with_the_same_vm_uuid() {
    let first = mock(false, false).await;
    let second = mock(false, false).await;
    let state_directory = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state_directory.path().into()).unwrap();
    let a = backend.connect(
        ConnectionOptions::new(first.endpoint.clone()),
        Credentials::new("root".into(), "fixture".into()),
    );
    let b = backend.connect(
        ConnectionOptions::new(second.endpoint.clone()),
        Credentials::new("root".into(), "fixture".into()),
    );
    let inventory_a = a.wait_ready(deadline(3)).await.unwrap();
    let inventory_b = b.wait_ready(deadline(3)).await.unwrap();
    assert_ne!(inventory_a.pool_id, inventory_b.pool_id);
    let observed = a
        .latest()
        .observe_vm(inventory_a.guests().next().unwrap().id)
        .unwrap();
    assert!(matches!(
        b.submit(
            xenadmin_core::OperationId::new(),
            xenadmin_core::VmCommand::Power {
                vm: observed,
                action: xenadmin_core::PowerAction::Start
            }
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
    let state_directory = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state_directory.path().into()).unwrap();
    let a = backend.connect(
        ConnectionOptions::new(server.endpoint.clone()),
        Credentials::new("root".into(), "fixture".into()),
    );
    let inventory = a.wait_ready(deadline(3)).await.unwrap();
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
        b.wait_ready(deadline(3)).await.unwrap().pool_id,
        inventory.pool_id
    );
    assert!(matches!(
        b.submit(
            xenadmin_core::OperationId::new(),
            xenadmin_core::VmCommand::Power {
                vm: observed,
                action: xenadmin_core::PowerAction::Start
            }
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
    let state_directory = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state_directory.path().into()).unwrap();
    let connection = backend.connect(
        ConnectionOptions::new(server.endpoint.clone()),
        Credentials::new("root".into(), "fixture-password".into()),
    );
    let error = connection
        .wait_ready(deadline(3))
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
    authenticated_console_roundtrip(false).await;
}
#[tokio::test]
async fn authenticated_host_console_preserves_the_handshake_and_transmits_input() {
    authenticated_console_roundtrip(true).await;
}
async fn authenticated_console_roundtrip(host: bool) {
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
    let server = if host {
        host_console::server(Some(location), 0).await
    } else {
        mock_console(false, false, Some(location)).await
    };
    let state_directory = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state_directory.path().into()).unwrap();
    let connection = backend.connect(
        ConnectionOptions::new(server.endpoint.clone()),
        Credentials::new("root".into(), "fixture-password".into()),
    );
    let inventory = connection.wait_ready(deadline(3)).await.unwrap();
    let console = if host {
        let host = *inventory.hosts.keys().next().unwrap();
        let vm = inventory.details.hosts[&host].control_domain.unwrap();
        connection
            .open_host_console(
                connection
                    .latest()
                    .observe_resource(xenadmin_core::ResourceTarget::Host(host))
                    .unwrap(),
                vm,
            )
            .unwrap()
    } else {
        connection
            .open_console(
                connection
                    .latest()
                    .observe_vm(inventory.guests().next().unwrap().id)
                    .unwrap(),
            )
            .unwrap()
    };
    tokio::time::timeout(deadline(5), async {
        loop {
            let snapshot = console.latest();
            if let Some(frame) = &snapshot.frame
                && *frame.rgba == [255, 0, 0, 255, 0, 255, 0, 255]
            {
                assert_eq!(snapshot.state, ConsoleState::Connected);
                assert_eq!((frame.width, frame.height), (2, 1));
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
    tokio::time::timeout(deadline(3), async {
        while !keys.lock().unwrap().contains(&(0xff0d, false))
            || pointer_count.load(Ordering::SeqCst) < 2
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();

    // A typed text arrives as the key presses of a US keyboard, a character at a time,
    // with Shift held around the characters that need it and a line break as Return.
    const SHIFT: u32 = 0xffe1;
    let arrived = |from: usize, count: usize| {
        let keys = keys.clone();
        async move {
            tokio::time::timeout(deadline(8), async {
                while keys.lock().unwrap().len() < from + count {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("the typed keys arrive");
        }
    };
    let before = keys.lock().unwrap().len();
    console
        .input(ConsoleInput::TypeText("aB!\r\n".into()))
        .unwrap();
    let expected = [
        (0x61, true),
        (0x61, false),
        (SHIFT, true),
        (0x42, true),
        (0x42, false),
        (SHIFT, false),
        (SHIFT, true),
        (0x21, true),
        (0x21, false),
        (SHIFT, false),
        (0xff0d, true),
        (0xff0d, false),
    ];
    arrived(before, expected.len()).await;
    assert_eq!(keys.lock().unwrap()[before..], expected);
    tokio::time::timeout(deadline(3), async {
        while console.latest().typing != 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the snapshot reports the text finished");

    // A text with a character no US key types is refused whole, and nothing is sent.
    assert!(matches!(
        console.input(ConsoleInput::TypeText("caf\u{e9}".into())),
        Err(xenadmin_core::Error::ConsoleTextUntypable)
    ));

    // A release stops a longer text between two characters: nothing more arrives, and
    // every Shift that went down came up.
    let before = keys.lock().unwrap().len();
    console
        .input(ConsoleInput::TypeText("X".repeat(200)))
        .unwrap();
    arrived(before, 8).await;
    assert!(console.latest().typing > 0);
    console.input(ConsoleInput::ReleaseAll).unwrap();
    tokio::time::timeout(deadline(3), async {
        while console.latest().typing != 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the release stops the text");
    tokio::time::sleep(Duration::from_millis(150)).await;
    let stopped = keys.lock().unwrap()[before..].to_vec();
    assert!(stopped.len() < 200 * 4, "{} key events", stopped.len());
    assert_eq!(stopped.len() % 4, 0, "the text stopped inside a character");
    let count = |key: (u32, bool)| stopped.iter().filter(|k| **k == key).count();
    assert_eq!(count((SHIFT, true)), count((SHIFT, false)));
    assert_eq!(count((0x58, true)), count((0x58, false)));
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(keys.lock().unwrap().len(), before + stopped.len());
    assert!(
        !keys.lock().unwrap().contains(&(0xe9, true)),
        "the refused text sent a key"
    );

    console.close();
    connection.close().await;
    let _ = tokio::time::timeout(deadline(3), rfb).await;
}

async fn finish(operation: &xenadmin_core::Operation) -> xenadmin_core::OperationState {
    tokio::time::timeout(deadline(5), async {
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
    let inventory = connection.wait_ready(deadline(3)).await.unwrap();
    let vm = connection
        .latest()
        .observe_vm(inventory.guests().next().unwrap().id)
        .unwrap();
    let id = OperationId::new();
    let operation = connection
        .submit(
            id,
            xenadmin_core::VmCommand::Power {
                vm,
                action: PowerAction::Start,
            },
        )
        .unwrap();
    assert_eq!(finish(&operation).await, OperationState::Succeeded);
    let duplicate = connection
        .submit(
            id,
            xenadmin_core::VmCommand::Power {
                vm,
                action: PowerAction::Start,
            },
        )
        .unwrap();
    assert_eq!(finish(&duplicate).await, OperationState::Succeeded);
    assert_eq!(server.mutations.load(Ordering::SeqCst), 1);
    let database = journal_text(directory.path());
    assert!(!database.contains("fixture-password"));
    assert!(!database.contains("fixture-session"));
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
    let inventory = connection.wait_ready(deadline(3)).await.unwrap();
    let vm = connection
        .latest()
        .observe_vm(inventory.guests().next().unwrap().id)
        .unwrap();
    let operation = connection
        .submit(
            OperationId::new(),
            xenadmin_core::VmCommand::Power {
                vm,
                action: PowerAction::Start,
            },
        )
        .unwrap();
    assert_eq!(finish(&operation).await, OperationState::OutcomeUnknown);
    let second = connection
        .submit(
            OperationId::new(),
            xenadmin_core::VmCommand::Power {
                vm,
                action: PowerAction::Start,
            },
        )
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
    connection.wait_ready(deadline(3)).await.unwrap();
    tokio::time::timeout(deadline(3), async {
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

/// A server whose event stream says nothing after the first inventory, and whose direct
/// read of the hosts gives `enabled` as `read_enabled` decides from the number of the read.
/// A second full inventory, if the client asks for one, has the host disabled.
async fn silent_event_stream(
    read_enabled: impl Fn(usize) -> bool + Send + Sync + 'static,
) -> (Mock, Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let full_reads = Arc::new(AtomicUsize::new(0));
    let host_reads = Arc::new(AtomicUsize::new(0));
    let read_enabled = Arc::new(read_enabled);
    let server = mock_overridden(override_rpc({
        let (full_reads, host_reads) = (full_reads.clone(), host_reads.clone());
        move |request| {
            let (full_reads, host_reads, read_enabled) =
                (full_reads.clone(), host_reads.clone(), read_enabled.clone());
            async move {
                match request["method"].as_str()? {
                    "event.from" if request["params"][2] == "" => {
                        let n = full_reads.fetch_add(1, Ordering::SeqCst);
                        let mut all = batch("initial", "Halted");
                        all["events"][1]["snapshot"]["enabled"] = json!(n == 0);
                        Some(RpcReply::Json(json!({"result": all})))
                    }
                    "event.from" => {
                        tokio::time::sleep(Duration::from_secs(60)).await;
                        None
                    }
                    "host.get_all_records" => {
                        let n = host_reads.fetch_add(1, Ordering::SeqCst);
                        let mut host = batch("initial", "Halted")["events"][1]["snapshot"].clone();
                        host["enabled"] = json!(read_enabled(n));
                        Some(RpcReply::Json(json!({"result": {"OpaqueRef:host": host}})))
                    }
                    _ => None,
                }
            }
        }
    }))
    .await;
    (server, full_reads, host_reads)
}
async fn connect_checking(
    server: &Mock,
    every: Duration,
) -> (tempfile::TempDir, Backend, xenadmin_core::Connection) {
    let directory = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(directory.path().into()).unwrap();
    let mut options = ConnectionOptions::new(server.endpoint.clone());
    options.inventory_check_interval = every;
    let connection = backend.connect(
        options,
        Credentials::new("fixture".into(), "fixture".into()),
    );
    connection.wait_ready(deadline(3)).await.unwrap();
    (directory, backend, connection)
}
fn host_enabled(connection: &xenadmin_core::Connection) -> Option<bool> {
    connection
        .latest()
        .inventory()
        .and_then(|i| i.hosts.values().next().map(|h| h.enabled))
}

#[tokio::test]
async fn an_inventory_the_events_no_longer_keep_current_is_read_again_in_full() {
    // The host is disabled, no event says so, and every direct read shows it.
    let (server, full_reads, host_reads) = silent_event_stream(|_| false).await;
    let (_directory, _backend, connection) =
        connect_checking(&server, Duration::from_millis(100)).await;
    assert_eq!(host_enabled(&connection), Some(true));
    let generation = connection.latest().generation();
    tokio::time::timeout(deadline(5), async {
        while host_enabled(&connection) != Some(false) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the inventory must be read again once two checks running disagree with it");
    // One disagreement was not enough, the second was, and the session is the same one.
    assert_eq!(full_reads.load(Ordering::SeqCst), 2);
    assert!(host_reads.load(Ordering::SeqCst) >= 2);
    assert_eq!(connection.latest().generation(), generation);
    assert_eq!(server.logins.load(Ordering::SeqCst), 1);
    // With the inventory current again the checks agree and nothing more is read in full.
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(full_reads.load(Ordering::SeqCst), 2);
    assert_eq!(host_enabled(&connection), Some(false));
    connection.close().await;
}

#[tokio::test]
async fn an_inventory_that_agrees_with_the_hosts_or_disagrees_once_is_left_alone() {
    // Agreeing at every check: nothing is read again.
    let (server, full_reads, host_reads) = silent_event_stream(|_| true).await;
    let (_directory, _backend, connection) =
        connect_checking(&server, Duration::from_millis(100)).await;
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert!(host_reads.load(Ordering::SeqCst) >= 3);
    assert_eq!(full_reads.load(Ordering::SeqCst), 1);
    assert_eq!(host_enabled(&connection), Some(true));
    connection.close().await;
    // Disagreeing at one check only, as when an event is on its way: nothing is read again.
    let (server, full_reads, host_reads) = silent_event_stream(|n| n != 1).await;
    let (_directory, _backend, connection) =
        connect_checking(&server, Duration::from_millis(100)).await;
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert!(host_reads.load(Ordering::SeqCst) >= 4);
    assert_eq!(full_reads.load(Ordering::SeqCst), 1);
    assert_eq!(host_enabled(&connection), Some(true));
    connection.close().await;
    // Switched off: the hosts are never read.
    let (server, full_reads, host_reads) = silent_event_stream(|_| false).await;
    let (_directory, _backend, connection) = connect_checking(&server, Duration::ZERO).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(host_reads.load(Ordering::SeqCst), 0);
    assert_eq!(full_reads.load(Ordering::SeqCst), 1);
    connection.close().await;
}

#[tokio::test]
async fn task_progress_does_not_cancel_the_inventory_long_poll() {
    let polls = Arc::new(AtomicUsize::new(0));
    let events = Arc::new(AtomicUsize::new(0));
    let server=mock_overridden(override_rpc({let polls=polls.clone();let events=events.clone();move|request|{
        let (polls,events)=(polls.clone(),events.clone());async move {
            match request["method"].as_str()? {
                "task.get_record"=>{
                    let count=polls.fetch_add(1,Ordering::SeqCst);
                    Some(RpcReply::Json(json!({"result":{"status":if count<4 {"pending"} else {"success"},"progress":0.1*count as f64,"allowed_operations":[]}})))
                }
                "event.from" if request["params"][2]!=""=>{
                    events.fetch_add(1,Ordering::SeqCst);
                    if request["params"][2]=="initial" {
                        tokio::time::sleep(Duration::from_millis(1500)).await;
                        Some(RpcReply::Json(json!({"result":{"token":"changed","events":[batch("unused","Running")["events"][2].clone()]}})))
                    } else {tokio::time::sleep(Duration::from_secs(60)).await;None}
                }
                _=>None,
            }
        }
    }})).await;
    let directory = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(directory.path().into()).unwrap();
    let connection = backend.connect(
        ConnectionOptions::new(server.endpoint.clone()),
        Credentials::new("fixture".into(), "fixture".into()),
    );
    let inventory = connection.wait_ready(deadline(3)).await.unwrap();
    let vm = inventory.guests().next().unwrap().id;
    let observed = connection.latest().observe_vm(vm).unwrap();
    let _operation = connection
        .submit(
            xenadmin_core::OperationId::new(),
            xenadmin_core::VmCommand::Power {
                vm: observed,
                action: xenadmin_core::PowerAction::Start,
            },
        )
        .unwrap();
    tokio::time::timeout(deadline(3), async {
        loop {
            if connection
                .latest()
                .inventory()
                .is_some_and(|inventory| inventory.vms[&vm].power_state == PowerState::Running)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("inventory change must arrive while task progress continues");
    assert!(polls.load(Ordering::SeqCst) >= 2);
    assert!(
        events.load(Ordering::SeqCst) <= 2,
        "progress must not restart event.from"
    );
    connection.close().await;
}

#[tokio::test]
async fn a_transient_task_read_failure_does_not_abandon_the_task_or_repeat_the_mutation() {
    let calls = Arc::new(AtomicUsize::new(0));
    let server = mock_overridden(override_rpc({
        let calls = calls.clone();
        move |request| {
            let calls = calls.clone();
            async move {
                if request["method"] == "task.get_record"
                    && calls.fetch_add(1, Ordering::SeqCst) == 0
                {
                    Some(RpcReply::Drop)
                } else {
                    None
                }
            }
        }
    }))
    .await;
    let directory = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(directory.path().into()).unwrap();
    let connection = backend.connect(
        ConnectionOptions::new(server.endpoint.clone()),
        Credentials::new("fixture".into(), "fixture".into()),
    );
    let inventory = connection.wait_ready(deadline(3)).await.unwrap();
    let observed = connection
        .latest()
        .observe_vm(inventory.guests().next().unwrap().id)
        .unwrap();
    let operation = connection
        .submit(
            xenadmin_core::OperationId::new(),
            xenadmin_core::VmCommand::Power {
                vm: observed,
                action: xenadmin_core::PowerAction::Start,
            },
        )
        .unwrap();
    assert_eq!(
        finish(&operation).await,
        xenadmin_core::OperationState::Succeeded
    );
    assert_eq!(server.mutations.load(Ordering::SeqCst), 1);
    connection.close().await;
}

#[tokio::test]
async fn failed_tasks_show_the_safe_host_error_code_and_are_cleaned_up() {
    let server=mock_overridden(override_rpc(|request|async move {
        (request["method"]=="task.get_record").then(||RpcReply::Json(json!({"result":{"status":"failure","progress":0,"error_info":["NO_HOSTS_AVAILABLE","sensitive-host-detail"],"allowed_operations":[]}})))
    })).await;
    let directory = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(directory.path().into()).unwrap();
    let connection = backend.connect(
        ConnectionOptions::new(server.endpoint.clone()),
        Credentials::new("fixture".into(), "fixture".into()),
    );
    let inventory = connection.wait_ready(deadline(3)).await.unwrap();
    let observed = connection
        .latest()
        .observe_vm(inventory.guests().next().unwrap().id)
        .unwrap();
    let operation = connection
        .submit(
            xenadmin_core::OperationId::new(),
            xenadmin_core::VmCommand::Power {
                vm: observed,
                action: xenadmin_core::PowerAction::Start,
            },
        )
        .unwrap();
    assert_eq!(
        finish(&operation).await,
        xenadmin_core::OperationState::Failed
    );
    assert!(operation.latest().detail.contains("NO_HOSTS_AVAILABLE"));
    assert!(!operation.latest().detail.contains("sensitive-host-detail"));
    connection.close().await;
    assert_eq!(
        server
            .methods
            .lock()
            .unwrap()
            .iter()
            .filter(|method| method.as_str() == "task.destroy")
            .count(),
        1
    );
}

#[tokio::test]
async fn suspended_and_paused_guests_have_nondestructive_recovery_actions() {
    use xenadmin_core::{OperationId, OperationState, PowerAction};
    for (state, action, wire) in [
        ("Suspended", PowerAction::Resume, "resume"),
        ("Paused", PowerAction::Unpause, "unpause"),
    ] {
        let server = mock_overridden(override_rpc(move |request| async move {
            if request["method"] == "event.from" && request["params"][2] == "" {
                let mut initial = batch("initial", state);
                initial["events"][2]["snapshot"]["allowed_operations"] = json!([wire]);
                Some(RpcReply::Json(json!({"result":initial})))
            } else if request["method"] == format!("Async.VM.{wire}") {
                assert_eq!(
                    request["params"].as_array().unwrap().len(),
                    if action == PowerAction::Resume { 4 } else { 2 }
                );
                if action == PowerAction::Resume {
                    assert_eq!(request["params"][2], false);
                    assert_eq!(request["params"][3], false);
                }
                Some(RpcReply::Json(json!({"result":"OpaqueRef:task"})))
            } else {
                None
            }
        }))
        .await;
        let directory = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(directory.path().into()).unwrap();
        let connection = backend.connect(
            ConnectionOptions::new(server.endpoint.clone()),
            Credentials::new("fixture".into(), "fixture".into()),
        );
        let inventory = connection.wait_ready(deadline(3)).await.unwrap();
        let vm = inventory.guests().next().unwrap();
        assert_eq!(vm.allowed_actions, vec![action]);
        let operation = connection
            .submit(
                OperationId::new(),
                xenadmin_core::VmCommand::Power {
                    vm: connection.latest().observe_vm(vm.id).unwrap(),
                    action,
                },
            )
            .unwrap();
        assert_eq!(finish(&operation).await, OperationState::Succeeded);
        connection.close().await;
    }
}

#[tokio::test]
async fn an_established_session_follows_a_new_coordinator_in_a_new_generation() {
    let destination = mock_overridden(override_rpc(|request| async move {
        (request["method"] == "event.from" && request["params"][2] == "")
            .then(|| RpcReply::Json(json!({"result":batch("initial","Running")})))
    }))
    .await;
    let redirect = destination.endpoint.to_string();
    let release = Arc::new(tokio::sync::Notify::new());
    let source = mock_overridden(override_rpc({
        let release = release.clone();
        move |request| {
            let (redirect, release) = (redirect.clone(), release.clone());
            async move {
                if request["method"] == "event.from" && request["params"][2] == "" {
                    Some(RpcReply::Json(json!({"result":batch("initial","Halted")})))
                } else if request["method"] == "event.from" {
                    release.notified().await;
                    Some(RpcReply::Json(
                        json!({"error":{"code":1,"message":"HOST_IS_SLAVE","data":[redirect]}}),
                    ))
                } else {
                    None
                }
            }
        }
    }))
    .await;
    let directory = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(directory.path().into()).unwrap();
    let connection = backend.connect(
        ConnectionOptions::new(source.endpoint.clone()),
        Credentials::new("fixture".into(), "fixture".into()),
    );
    connection.wait_ready(deadline(3)).await.unwrap();
    let first = connection.latest().generation();
    release.notify_one();
    tokio::time::timeout(deadline(6), async {
        loop {
            let snapshot = connection.latest();
            if snapshot.is_current() && snapshot.generation() != first {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(source.logins.load(Ordering::SeqCst), 1);
    assert_eq!(destination.logins.load(Ordering::SeqCst), 1);
    assert_eq!(
        connection
            .latest()
            .inventory()
            .unwrap()
            .guests()
            .next()
            .unwrap()
            .power_state,
        PowerState::Running
    );
    connection.close().await;
}

#[tokio::test]
async fn a_late_event_reply_from_the_previous_coordinator_is_not_applied_to_the_new_generation() {
    use xenadmin_core::{OperationId, OperationState, PowerAction, VmCommand};
    fn guest_change(token: &str, name: &str, state: &str) -> RpcReply {
        let mut event = batch("unused", state)["events"][2].clone();
        event["snapshot"]["name_label"] = json!(name);
        RpcReply::Json(json!({"result":{"token":token,"events":[event]}}))
    }
    let late = Arc::new(tokio::sync::Notify::new());
    let answered = Arc::new(tokio::sync::Notify::new());
    let tokens = Arc::new(Mutex::new(Vec::new()));
    let destination = mock_overridden(override_rpc({
        let (answered, tokens) = (answered.clone(), tokens.clone());
        move |request| {
            let (answered, tokens) = (answered.clone(), tokens.clone());
            async move {
                if request["method"] != "event.from" {
                    return None;
                }
                let token = request["params"][2].as_str()?.to_owned();
                tokens.lock().unwrap().push(token.clone());
                match token.as_str() {
                    "" => Some(RpcReply::Json(json!({"result":batch("initial","Running")}))),
                    // The current generation's next change is held until the previous
                    // coordinator has been released to answer, so the late reply is
                    // written first.
                    "initial" => {
                        answered.notified().await;
                        Some(guest_change(
                            "current",
                            "Renamed on the new coordinator",
                            "Running",
                        ))
                    }
                    _ => {
                        tokio::time::sleep(Duration::from_secs(60)).await;
                        Some(RpcReply::Json(
                            json!({"result":{"token":token,"events":[]}}),
                        ))
                    }
                }
            }
        }
    }))
    .await;
    let redirect = destination.endpoint.to_string();
    let polling = Arc::new(tokio::sync::Notify::new());
    let polls = Arc::new(AtomicUsize::new(0));
    let source = mock_overridden(override_rpc({
        let (late, answered, polling, polls) = (
            late.clone(),
            answered.clone(),
            polling.clone(),
            polls.clone(),
        );
        move |request| {
            let (redirect, late, answered, polling, polls) = (
                redirect.clone(),
                late.clone(),
                answered.clone(),
                polling.clone(),
                polls.clone(),
            );
            async move {
                match request["method"].as_str()? {
                    "event.from" if request["params"][2] == "" => {
                        Some(RpcReply::Json(json!({"result":batch("initial","Halted")})))
                    }
                    "event.from" => {
                        polls.fetch_add(1, Ordering::SeqCst);
                        polling.notify_one();
                        late.notified().await;
                        answered.notify_one();
                        Some(guest_change(
                            "late",
                            "Late change from the previous coordinator",
                            "Suspended",
                        ))
                    }
                    "Async.VM.start" => Some(RpcReply::Json(
                        json!({"error":{"code":1,"message":"HOST_IS_SLAVE","data":[redirect]}}),
                    )),
                    _ => None,
                }
            }
        }
    }))
    .await;
    let directory = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(directory.path().into()).unwrap();
    let connection = backend.connect(
        ConnectionOptions::new(source.endpoint.clone()),
        Credentials::new("fixture".into(), "fixture".into()),
    );
    let guest = connection
        .wait_ready(deadline(3))
        .await
        .unwrap()
        .guests()
        .next()
        .unwrap()
        .id;
    let first = connection.latest().generation();
    tokio::time::timeout(deadline(3), polling.notified())
        .await
        .expect("the event long poll must be outstanding before the session moves");
    // A command answered HOST_IS_SLAVE moves the session while that long poll is unanswered.
    let refused = connection
        .submit(
            OperationId::new(),
            VmCommand::Power {
                vm: connection.latest().observe_vm(guest).unwrap(),
                action: PowerAction::Start,
            },
        )
        .unwrap();
    assert_eq!(finish(&refused).await, OperationState::Failed);
    tokio::time::timeout(deadline(6), async {
        loop {
            let snapshot = connection.latest();
            if snapshot.is_current() && snapshot.generation() != first {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let second = connection.latest().generation();

    late.notify_one();
    tokio::time::timeout(deadline(3), async {
        loop {
            if connection
                .latest()
                .inventory()
                .is_some_and(|inventory| inventory.vms[&guest].name != "XO recovery guest")
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("a change must reach the inventory after the late reply was released");
    tokio::time::timeout(deadline(3), async {
        while tokens.lock().unwrap().len() < 3 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the new generation must continue its own event stream");
    let snapshot = connection.latest();
    assert!(snapshot.is_current());
    assert_eq!(snapshot.generation(), second);
    let vm = &snapshot.inventory().unwrap().vms[&guest];
    assert_eq!(vm.name, "Renamed on the new coordinator");
    assert_eq!(vm.power_state, PowerState::Running);
    assert_eq!(
        *tokens.lock().unwrap(),
        ["", "initial", "current"],
        "the late reply's token must not continue the new generation's stream"
    );
    assert_eq!(
        polls.load(Ordering::SeqCst),
        1,
        "the previous coordinator must not be polled again"
    );
    assert_eq!(destination.mutations.load(Ordering::SeqCst), 0);
    connection.close().await;
}

fn guest_event(name: &str, state: &str) -> Value {
    let mut event = batch("unused", state)["events"][2].clone();
    event["snapshot"]["name_label"] = json!(name);
    event
}

/// Serves `event.from` from a log of guest events the way XAPI does. A token names a
/// position in the log, the bootstrap describes the guest as it was when its request
/// arrived, and a continuation waits for whatever was logged after its token.
#[derive(Clone)]
struct EventLog {
    events: tokio::sync::watch::Sender<Vec<Value>>,
    /// The token each `event.from` carried, in the order the calls reached the log.
    received: Arc<Mutex<Vec<String>>>,
    /// The token each reply returned, in the order the replies were prepared.
    returned: Arc<Mutex<Vec<String>>>,
}
impl EventLog {
    fn new() -> Self {
        Self {
            events: tokio::sync::watch::Sender::new(Vec::new()),
            received: Arc::default(),
            returned: Arc::default(),
        }
    }
    fn push(&self, event: Value) {
        self.events.send_modify(|events| events.push(event));
    }
    async fn reply(&self, request: &Value) -> Option<RpcReply> {
        if request["method"] != "event.from" {
            return None;
        }
        let token = request["params"][2].as_str()?.to_owned();
        self.received.lock().unwrap().push(token.clone());
        let (position, events) = if token.is_empty() {
            let logged = self.events.borrow().clone();
            let mut initial = batch("unused", "Halted");
            if let Some(latest) = logged.last() {
                initial["events"][2] = latest.clone();
            }
            (logged.len(), initial["events"].take())
        } else {
            let position: usize = token.strip_prefix("position:")?.parse().ok()?;
            let mut events = self.events.subscribe();
            let logged = events
                .wait_for(|logged| logged.len() > position)
                .await
                .ok()?
                .clone();
            (logged.len(), json!(logged[position..]))
        };
        let token = format!("position:{position}");
        self.returned.lock().unwrap().push(token.clone());
        Some(RpcReply::Json(
            json!({"result":{"token":token,"events":events}}),
        ))
    }
}

async fn inventory_at_revision(
    connection: &xenadmin_core::Connection,
    revision: u64,
) -> Arc<xenadmin_core::Inventory> {
    tokio::time::timeout(deadline(3), async {
        loop {
            if let Some(inventory) = connection
                .latest()
                .inventory()
                .filter(|inventory| inventory.revision >= revision)
            {
                return inventory.clone();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the logged change must reach the published inventory")
}

#[tokio::test]
async fn changes_racing_the_initial_inventory_are_applied_in_order_from_consecutive_tokens() {
    let log = EventLog::new();
    let held = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let observed = Arc::new(tokio::sync::Notify::new());
    let server = mock_overridden(override_rpc({
        let (log, held, release, observed) =
            (log.clone(), held.clone(), release.clone(), observed.clone());
        move |request| {
            let (log, held, release, observed) =
                (log.clone(), held.clone(), release.clone(), observed.clone());
            async move {
                if request["method"] != "event.from" {
                    return None;
                }
                if request["params"][2] == "" {
                    // The bootstrap is prepared when its request arrives and sent when the
                    // test releases it, so a change logged in between is one it does not
                    // describe.
                    let reply = log.reply(&request).await;
                    held.notify_one();
                    release.notified().await;
                    return reply;
                }
                if request["params"][2] == "position:0" {
                    // The batch after the bootstrap is answered once the test has read the
                    // bootstrap inventory, so that inventory is seen before the change.
                    observed.notified().await;
                }
                log.reply(&request).await
            }
        }
    }))
    .await;
    let directory = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(directory.path().into()).unwrap();
    let connection = backend.connect(
        ConnectionOptions::new(server.endpoint.clone()),
        Credentials::new("fixture".into(), "fixture".into()),
    );
    tokio::time::timeout(deadline(3), held.notified())
        .await
        .expect("the inventory request follows a successful login");
    assert_eq!(connection.latest().state(), ConnectionState::Synchronizing);
    // The guest changes on the host while the initial inventory is still outstanding.
    log.push(guest_event(
        "Renamed during the initial inventory",
        "Running",
    ));
    release.notify_one();
    let initial = connection.wait_ready(deadline(3)).await.unwrap();
    let generation = connection.latest().generation();
    let guest = initial.guests().next().unwrap().id;
    assert_eq!(initial.revision, 1);
    assert_eq!(initial.vms[&guest].name, "XO recovery guest");
    assert_eq!(initial.vms[&guest].power_state, PowerState::Halted);

    observed.notify_one();
    let first = inventory_at_revision(&connection, 2).await;
    assert_eq!(first.revision, 2);
    assert_eq!(
        first.vms[&guest].name,
        "Renamed during the initial inventory"
    );
    assert_eq!(first.vms[&guest].power_state, PowerState::Running);

    log.push(guest_event("Renamed after the first change", "Suspended"));
    let second = inventory_at_revision(&connection, 3).await;
    assert_eq!(second.revision, 3);
    assert_eq!(second.vms[&guest].name, "Renamed after the first change");
    assert_eq!(second.vms[&guest].power_state, PowerState::Suspended);

    tokio::time::timeout(deadline(3), async {
        while log.received.lock().unwrap().len() < 4 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the client must keep listening after the second change");
    assert_eq!(
        *log.received.lock().unwrap(),
        ["", "position:0", "position:1", "position:2"],
        "one download, then each call continues from the previous reply"
    );
    assert_eq!(
        log.received.lock().unwrap()[1..],
        *log.returned.lock().unwrap(),
        "every returned token is sent back once, in order"
    );
    let snapshot = connection.latest();
    assert!(snapshot.is_current());
    assert_eq!(snapshot.generation(), generation);
    assert_eq!(snapshot.inventory().unwrap().revision, 3);
    assert_eq!(server.logins.load(Ordering::SeqCst), 1);
    connection.close().await;
}

const FAULTY_CHANGE: &str = "Changed by a faulty batch";

/// After a healthy bootstrap the first continuation is answered with `faulty`, whose token
/// is "faulty" and whose first event validly renames the guest to [`FAULTY_CHANGE`] and
/// suspends it. Nothing of that batch may be published, the fault is reported as
/// `reported` on a reconnecting snapshot, and the client downloads the inventory again
/// without ever sending the faulty batch's token.
async fn a_faulty_event_batch_is_reported_and_resynchronized(
    faulty: fn() -> Value,
    reported: &str,
) {
    use xenadmin_core::{ConnectionStage, Error};
    let tokens = Arc::new(Mutex::new(Vec::new()));
    let release = Arc::new(tokio::sync::Notify::new());
    let server = mock_overridden(override_rpc({
        let (tokens, release) = (tokens.clone(), release.clone());
        move |request| {
            let (tokens, release) = (tokens.clone(), release.clone());
            async move {
                if request["method"] != "event.from" {
                    return None;
                }
                let token = request["params"][2].as_str()?.to_owned();
                let bootstraps = {
                    let mut tokens = tokens.lock().unwrap();
                    tokens.push(token.clone());
                    tokens.iter().filter(|token| token.is_empty()).count()
                };
                let result = match token.as_str() {
                    "" if bootstraps == 1 => batch("initial", "Halted"),
                    // The host never held the faulty batch's change: a later download
                    // shows the guest under its own name, now running.
                    "" => batch("resynchronized", "Running"),
                    "initial" => {
                        release.notified().await;
                        faulty()
                    }
                    _ => {
                        tokio::time::sleep(Duration::from_secs(60)).await;
                        json!({"token":token,"events":[]})
                    }
                };
                Some(RpcReply::Json(json!({"result":result})))
            }
        }
    }))
    .await;
    let directory = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(directory.path().into()).unwrap();
    let connection = backend.connect(
        ConnectionOptions::new(server.endpoint.clone()),
        Credentials::new("fixture".into(), "fixture".into()),
    );
    let before = connection.wait_ready(deadline(3)).await.unwrap();
    let guest = before.guests().next().unwrap().id;
    let first = connection.latest().generation();
    assert_eq!(before.revision, 1);
    assert_eq!(before.vms[&guest].power_state, PowerState::Halted);
    // Every snapshot read from here on is checked for the faulty batch's valid event.
    let unapplied = |snapshot: &xenadmin_core::ConnectionSnapshot| {
        if let Some(inventory) = snapshot.inventory() {
            assert_ne!(
                inventory.vms[&guest].name, FAULTY_CHANGE,
                "the valid event of a faulty batch was applied"
            );
            assert_ne!(inventory.vms[&guest].power_state, PowerState::Suspended);
        }
    };

    release.notify_one();
    let faulted = tokio::time::timeout(deadline(5), async {
        loop {
            let snapshot = connection.latest();
            unapplied(&snapshot);
            if snapshot.failure().is_some() {
                break snapshot;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the connection must report the faulty batch");
    assert_eq!(faulted.state(), ConnectionState::Reconnecting);
    assert!(!faulted.is_current());
    assert_eq!(faulted.generation(), first);
    assert_eq!(faulted.error(), Some(reported));
    let failure = faulted.failure().unwrap();
    assert_eq!(failure.stage, ConnectionStage::EventStream);
    assert_eq!(failure.message, reported);
    assert!(
        Arc::ptr_eq(faulted.inventory().unwrap(), &before),
        "the inventory shown with the fault must be the one from before the faulty batch"
    );
    assert!(matches!(faulted.observe_vm(guest), Err(Error::Stale)));

    let recovered = tokio::time::timeout(deadline(8), async {
        loop {
            let snapshot = connection.latest();
            unapplied(&snapshot);
            if snapshot.is_current() && snapshot.generation() != first {
                break snapshot;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the connection must resynchronize once the host answers normally");
    assert_eq!(recovered.error(), None);
    assert!(recovered.failure().is_none());
    let inventory = recovered.inventory().unwrap();
    assert_eq!(
        inventory.revision, 1,
        "the new inventory is built from a full download alone"
    );
    assert_eq!(inventory.pool_id, before.pool_id);
    assert_eq!(inventory.vms[&guest].name, "XO recovery guest");
    assert_eq!(inventory.vms[&guest].power_state, PowerState::Running);
    tokio::time::timeout(deadline(3), async {
        while tokens.lock().unwrap().len() < 4 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the new session must continue from its own download");
    assert_eq!(
        *tokens.lock().unwrap(),
        ["", "initial", "", "resynchronized"],
        "the faulty batch's token must never be sent back"
    );
    assert_eq!(server.logins.load(Ordering::SeqCst), 2);
    connection.close().await;
}

#[tokio::test]
async fn a_malformed_event_batch_is_not_applied_and_its_token_is_never_sent_back() {
    a_faulty_event_batch_is_reported_and_resynchronized(
        // A valid change, then a host record whose required UUID is a number.
        || {
            json!({"token":"faulty","events":[
                guest_event(FAULTY_CHANGE, "Suspended"),
                {"class":"host","operation":"mod","ref":"OpaqueRef:host","snapshot":{"uuid":42,"name_label":"Fixture host","address":"192.0.2.10","enabled":true}}
            ]})
        },
        "Invalid XAPI response: missing required inventory field",
    )
    .await;
}

#[tokio::test]
async fn an_undecodable_event_batch_is_not_applied_and_its_token_is_never_sent_back() {
    a_faulty_event_batch_is_reported_and_resynchronized(
        // A valid change, then an event whose own object reference is a number.
        || {
            json!({"token":"faulty","events":[
                guest_event(FAULTY_CHANGE, "Suspended"),
                {"class":"host","operation":"mod","ref":7,"snapshot":{"uuid":"22222222-2222-2222-2222-222222222222","name_label":"Fixture host","address":"192.0.2.10","enabled":true}}
            ]})
        },
        "Invalid XAPI response: invalid event batch",
    )
    .await;
}

#[tokio::test]
async fn an_event_batch_over_the_object_limit_is_not_applied_and_its_token_is_never_sent_back() {
    a_faulty_event_batch_is_reported_and_resynchronized(
        // A valid change, then enough valid networks to pass the inventory's limit of
        // 100,000 objects on top of the three the bootstrap holds.
        || {
            let mut events = vec![guest_event(FAULTY_CHANGE, "Suspended")];
            events.extend((0..100_000).map(|n| {
                json!({"class":"network","operation":"add","ref":format!("OpaqueRef:network-{n}"),"snapshot":{"uuid":format!("{n:08x}-0000-4000-8000-000000000000"),"name_label":"Surplus network","bridge":"xenbr0"}})
            }));
            json!({"token":"faulty","events":events})
        },
        "Invalid XAPI response: inventory exceeds the object limit",
    )
    .await;
}

#[tokio::test]
async fn an_event_reply_over_the_size_limit_is_not_applied_and_its_token_is_never_sent_back() {
    a_faulty_event_batch_is_reported_and_resynchronized(
        // One valid change whose description alone passes the 32 MiB reply limit.
        || {
            let mut event = guest_event(FAULTY_CHANGE, "Suspended");
            event["snapshot"]["name_description"] = json!("x".repeat(32 * 1024 * 1024 + 1));
            json!({"token":"faulty","events":[event]})
        },
        "Invalid XAPI response: response exceeds the size limit",
    )
    .await;
}

#[tokio::test]
async fn a_slow_or_closed_observer_does_not_hold_back_snapshots_or_operation_results() {
    use xenadmin_core::{OperationId, OperationState, PowerAction, VmCommand};
    let log = EventLog::new();
    let server = mock_overridden(override_rpc({
        let log = log.clone();
        move |request| {
            let log = log.clone();
            async move { log.reply(&request).await }
        }
    }))
    .await;
    let directory = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(directory.path().into()).unwrap();
    let connection = backend.connect(
        ConnectionOptions::new(server.endpoint.clone()),
        Credentials::new("fixture".into(), "fixture".into()),
    );
    let initial = connection.wait_ready(deadline(3)).await.unwrap();
    let guest = initial.guests().next().unwrap().id;
    let generation = connection.latest().generation();

    // One observer reads the first snapshot and is not polled again until the end, one is
    // closed part-way, and a third keeps up and records every state it is shown.
    let mut slow = connection.watch();
    assert_eq!(slow.borrow_and_update().inventory().unwrap().revision, 1);
    let mut closed = connection.watch();
    assert!(closed.borrow_and_update().is_current());
    let states = Arc::new(Mutex::new(Vec::new()));
    let attentive = tokio::spawn({
        let (mut receiver, states) = (connection.watch(), states.clone());
        async move {
            loop {
                let state = receiver.borrow_and_update().state();
                states.lock().unwrap().push(state);
                if receiver.changed().await.is_err() {
                    break;
                }
            }
        }
    });

    let mut revision = 1;
    let mut change = async |name: &str, state: &str| {
        log.push(guest_event(name, state));
        revision += 1;
        let inventory = inventory_at_revision(&connection, revision).await;
        assert_eq!(inventory.revision, revision);
        assert_eq!(inventory.vms[&guest].name, name);
    };
    change("First rename", "Halted").await;
    change("Second rename", "Halted").await;
    drop(closed);
    change("Third rename", "Halted").await;
    let operation = connection
        .submit(
            OperationId::new(),
            VmCommand::Power {
                vm: connection.latest().observe_vm(guest).unwrap(),
                action: PowerAction::Start,
            },
        )
        .unwrap();
    assert_eq!(finish(&operation).await, OperationState::Succeeded);
    change("Fourth rename", "Running").await;
    change("Fifth rename", "Running").await;

    let newest = connection.latest();
    assert!(newest.is_current());
    assert_eq!(newest.error(), None);
    assert_eq!(newest.inventory().unwrap().revision, 6);
    assert_eq!(newest.inventory().unwrap().vms[&guest].name, "Fifth rename");

    // The slow observer is told of a change at once and one read brings it up to date:
    // it is given the newest snapshot, not the five it never read.
    assert!(slow.has_changed().unwrap());
    tokio::time::timeout(deadline(1), slow.changed())
        .await
        .expect("an unread change is reported without waiting")
        .expect("the snapshot channel stays open for a slow observer");
    let read = slow.borrow_and_update().clone();
    assert!(Arc::ptr_eq(&read, &newest));
    assert!(!slow.has_changed().unwrap());

    // The accepted operation's result is still answered by its handle and by the history.
    let id = operation.latest().id;
    assert_eq!(operation.latest().state, OperationState::Succeeded);
    tokio::time::timeout(deadline(3), async {
        while !connection
            .operation_history()
            .iter()
            .any(|record| record.id == id && record.state == OperationState::Succeeded)
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the operation history must report the accepted operation's outcome");
    assert_eq!(server.mutations.load(Ordering::SeqCst), 1);

    // Leaving Ready always starts a new generation with a new login and download.
    attentive.abort();
    let states = states.lock().unwrap().clone();
    assert!(!states.is_empty());
    assert!(
        states.iter().all(|state| *state == ConnectionState::Ready),
        "{states:?}"
    );
    assert_eq!(connection.latest().generation(), generation);
    assert_eq!(server.logins.load(Ordering::SeqCst), 1);
    assert_eq!(
        *log.received.lock().unwrap(),
        [
            "",
            "position:0",
            "position:1",
            "position:2",
            "position:3",
            "position:4",
            "position:5"
        ]
    );
    connection.close().await;
}

#[tokio::test]
async fn a_control_queued_across_a_reconnect_does_not_publish_ready_before_the_new_inventory() {
    use xenadmin_core::{Error, OperationControl, OperationId};
    let bootstraps = Arc::new(AtomicUsize::new(0));
    let expire = Arc::new(tokio::sync::Notify::new());
    let held = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let server = mock_overridden(override_rpc({
        let (bootstraps, expire, held, release) = (
            bootstraps.clone(),
            expire.clone(),
            held.clone(),
            release.clone(),
        );
        move |request| {
            let (bootstraps, expire, held, release) = (
                bootstraps.clone(),
                expire.clone(),
                held.clone(),
                release.clone(),
            );
            async move {
                if request["method"] != "event.from" {
                    return None;
                }
                if request["params"][2] != "" {
                    // Released once, so only the first session's long poll is answered.
                    expire.notified().await;
                    return Some(RpcReply::Json(
                        json!({"error":{"code":1,"message":"SESSION_INVALID","data":[]}}),
                    ));
                }
                if bootstraps.fetch_add(1, Ordering::SeqCst) == 0 {
                    return Some(RpcReply::Json(json!({"result":batch("initial","Halted")})));
                }
                held.notify_one();
                release.notified().await;
                Some(RpcReply::Json(json!({"result":batch("initial","Running")})))
            }
        }
    }))
    .await;
    let directory = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(directory.path().into()).unwrap();
    let connection = backend.connect(
        ConnectionOptions::new(server.endpoint.clone()),
        Credentials::new("fixture".into(), "fixture".into()),
    );
    let previous = connection.wait_ready(deadline(3)).await.unwrap();
    let guest = previous.guests().next().unwrap().id;
    let first = connection.latest().generation();

    // The facade admits a control only while the snapshot is current, but the queue it
    // fills outlives the session: what the owner has not taken when the session ends is
    // taken by the next session before its inventory arrives. Nothing can hold the owner
    // still, so a thread that does not yield keeps the queue's 32 places full (the owner
    // opens the journal for each control it takes) until the facade refuses.
    let saturated = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let controls = tokio::task::spawn_blocking({
        let (connection, saturated) = (connection.clone(), saturated.clone());
        move || {
            let limit = std::time::Instant::now() + deadline(5);
            while std::time::Instant::now() < limit {
                match connection
                    .control_operation(OperationId::new(), OperationControl::RequestCancellation)
                {
                    Ok(()) => {}
                    Err(Error::Busy) => saturated.store(true, Ordering::SeqCst),
                    Err(Error::Stale) => return true,
                    Err(error) => panic!("unexpected refusal: {error}"),
                }
            }
            false
        }
    });
    tokio::time::timeout(deadline(3), async {
        while !saturated.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("the control queue must fill");
    expire.notify_one();
    assert!(
        tokio::time::timeout(deadline(6), controls)
            .await
            .unwrap()
            .unwrap(),
        "the facade must refuse controls once the session is lost"
    );

    tokio::time::timeout(deadline(6), held.notified())
        .await
        .expect("the client must log in again and ask for the inventory");
    // The owner takes everything left in the queue within microseconds of starting the
    // new session, so a quarter of a second after its inventory request is ample.
    let mut snapshot = connection.latest();
    let until = tokio::time::Instant::now() + deadline(1) / 4;
    while snapshot.state() == ConnectionState::Synchronizing && tokio::time::Instant::now() < until
    {
        tokio::time::sleep(Duration::from_millis(5)).await;
        snapshot = connection.latest();
    }
    assert_ne!(snapshot.generation(), first);
    assert_eq!(
        snapshot.state(),
        ConnectionState::Synchronizing,
        "a queued control was answered as if the connection were ready (error {:?}; the previous generation's inventory presented: {})",
        snapshot.error(),
        snapshot
            .inventory()
            .is_some_and(|inventory| Arc::ptr_eq(inventory, &previous)),
    );
    assert!(!snapshot.is_current());
    assert!(matches!(snapshot.observe_vm(guest), Err(Error::Stale)));

    release.notify_one();
    let current = tokio::time::timeout(deadline(3), async {
        loop {
            let snapshot = connection.latest();
            if snapshot.is_current()
                && snapshot.inventory().is_some_and(|inventory| {
                    inventory.vms[&guest].power_state == PowerState::Running
                })
            {
                break snapshot;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the new inventory must make the connection ready");
    assert_ne!(current.generation(), first);
    assert_eq!(server.logins.load(Ordering::SeqCst), 2);
    connection.close().await;
}

#[tokio::test]
async fn cancellation_after_reconnect_reaches_the_recovered_task() {
    use xenadmin_core::{OperationControl, OperationId, OperationState, PowerAction};
    let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let server=mock_overridden(override_rpc({let cancelled=cancelled.clone();move|request|{let cancelled=cancelled.clone();async move {
        match request["method"].as_str()? {
            "task.get_record"=>Some(RpcReply::Json(json!({"result":{"status":if cancelled.load(Ordering::SeqCst){"cancelled"}else{"pending"},"progress":0.2,"allowed_operations":["cancel"]}}))),
            "task.cancel"=>{cancelled.store(true,Ordering::SeqCst);Some(RpcReply::Json(json!({"result":null})))},
            _=>None,
        }
    }}})).await;
    let directory = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(directory.path().into()).unwrap();
    let connect = || {
        backend.connect(
            ConnectionOptions::new(server.endpoint.clone()),
            Credentials::new("fixture".into(), "fixture".into()),
        )
    };
    let connection = connect();
    let inventory = connection.wait_ready(deadline(3)).await.unwrap();
    let id = OperationId::new();
    let old = connection
        .submit(
            id,
            xenadmin_core::VmCommand::Power {
                vm: connection
                    .latest()
                    .observe_vm(inventory.guests().next().unwrap().id)
                    .unwrap(),
                action: PowerAction::Start,
            },
        )
        .unwrap();
    tokio::time::timeout(deadline(3), async {
        while !old.latest().has_task() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    connection.close().await;
    let replacement = connect();
    replacement.wait_ready(deadline(3)).await.unwrap();
    replacement
        .control_operation(id, OperationControl::RequestCancellation)
        .unwrap();
    tokio::time::timeout(deadline(5), async {
        loop {
            if replacement
                .operation_history()
                .iter()
                .any(|record| record.id == id && record.state == OperationState::Cancelled)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let current = replacement
        .operation_history()
        .iter()
        .find(|record| record.id == id)
        .unwrap()
        .clone();
    assert!(current.supersedes(&old.latest()));
    assert_eq!(server.mutations.load(Ordering::SeqCst), 1);
    replacement.close().await;
    assert_eq!(
        server
            .methods
            .lock()
            .unwrap()
            .iter()
            .filter(|method| method.as_str() == "task.cancel")
            .count(),
        1
    );
}

#[tokio::test]
async fn terminal_persistence_failure_is_visible_and_keeps_the_vm_reserved() {
    use xenadmin_core::{OperationId, OperationState, PowerAction};
    let server = mock(false, false).await;
    let directory = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(directory.path().into()).unwrap();
    let connection = backend.connect(
        ConnectionOptions::new(server.endpoint.clone()),
        Credentials::new("fixture".into(), "fixture".into()),
    );
    let inventory = connection.wait_ready(deadline(3)).await.unwrap();
    // Create the journal through a first operation, then inject a durable-write failure.
    let observed = connection
        .latest()
        .observe_vm(inventory.guests().next().unwrap().id)
        .unwrap();
    let first = connection
        .submit(
            OperationId::new(),
            xenadmin_core::VmCommand::Power {
                vm: observed,
                action: PowerAction::Start,
            },
        )
        .unwrap();
    assert_eq!(finish(&first).await, OperationState::Succeeded);
    let database = rusqlite::Connection::open(directory.path().join("operations.sqlite3")).unwrap();
    database.execute_batch("CREATE TRIGGER fail_terminal BEFORE UPDATE ON operations WHEN NEW.terminal=1 BEGIN SELECT RAISE(ABORT,'injected write failure'); END;").unwrap();
    let operation = connection
        .submit(
            OperationId::new(),
            xenadmin_core::VmCommand::Power {
                vm: observed,
                action: PowerAction::Start,
            },
        )
        .unwrap();
    assert_eq!(finish(&operation).await, OperationState::OutcomeUnknown);
    assert!(!operation.latest().state.terminal());
    let another = connection
        .submit(
            OperationId::new(),
            xenadmin_core::VmCommand::Power {
                vm: observed,
                action: PowerAction::Start,
            },
        )
        .unwrap();
    assert_eq!(finish(&another).await, OperationState::Rejected);
    assert_eq!(server.mutations.load(Ordering::SeqCst), 2);
    database
        .execute_batch("DROP TRIGGER fail_terminal")
        .unwrap();
    connection.close().await;
}

#[tokio::test]
async fn a_full_journal_refuses_the_submission_with_its_reason_and_sends_no_mutation() {
    use xenadmin_core::{Error, OperationId, OperationState, PowerAction, VmCommand};
    let server = mock(false, false).await;
    let directory = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(directory.path().into()).unwrap();
    let connection = backend.connect(
        ConnectionOptions::new(server.endpoint.clone()),
        Credentials::new("fixture".into(), "fixture".into()),
    );
    let inventory = connection.wait_ready(deadline(3)).await.unwrap();
    let observed = connection
        .latest()
        .observe_vm(inventory.guests().next().unwrap().id)
        .unwrap();
    let start = || VmCommand::Power {
        vm: observed,
        action: PowerAction::Start,
    };
    // Create the journal through a first operation.
    let first = connection.submit(OperationId::new(), start()).unwrap();
    assert_eq!(finish(&first).await, OperationState::Succeeded);
    assert_eq!(server.mutations.load(Ordering::SeqCst), 1);

    // This mock has one guest, so the journal cannot be filled by submitting. The limit
    // counts every pool: 1000 unresolved operations of other pools are written as copies
    // of the row above.
    let database = rusqlite::Connection::open(directory.path().join("operations.sqlite3")).unwrap();
    database.busy_timeout(deadline(5)).unwrap();
    let saved: String = database
        .query_row("SELECT payload FROM operations", [], |row| row.get(0))
        .unwrap();
    let mut payload: Value = serde_json::from_str(&saved).unwrap();
    payload["state"] = json!("OutcomeUnknown");
    database.execute_batch("BEGIN IMMEDIATE").unwrap();
    for _ in 0..1000 {
        let [id, pool, vm] = std::array::from_fn(|_| uuid::Uuid::new_v4().to_string());
        payload["id"] = json!(id);
        payload["pool"] = json!(pool);
        payload["vm"] = json!(vm);
        database
            .execute(
                "INSERT INTO operations(id,pool,vm,payload,terminal,revision) VALUES(?1,?2,?3,?4,0,1)",
                rusqlite::params![id, pool, vm, payload.to_string()],
            )
            .unwrap();
    }
    database.execute_batch("COMMIT").unwrap();
    let rows = |filter: &str| -> i64 {
        database
            .query_row(
                &format!("SELECT COUNT(*) FROM operations WHERE {filter}"),
                [],
                |row| row.get(0),
            )
            .unwrap()
    };
    assert_eq!(rows(&format!("id='{}'", first.latest().id)), 1);

    let id = OperationId::new();
    let refused = connection.submit(id, start()).unwrap();
    assert_eq!(finish(&refused).await, OperationState::Rejected);
    assert_eq!(refused.latest().detail, Error::JournalFull.to_string());
    assert_eq!(server.mutations.load(Ordering::SeqCst), 1);
    assert_eq!(rows(&format!("id='{id}'")), 0);
    assert_eq!(rows("terminal=0"), 1000);
    assert!(connection.operation_history().iter().all(|r| r.id != id));

    // One unresolved operation fewer and the same command is admitted and dispatched.
    database
        .execute(
            "DELETE FROM operations WHERE rowid=(SELECT MAX(rowid) FROM operations WHERE terminal=0)",
            [],
        )
        .unwrap();
    let admitted = connection.submit(OperationId::new(), start()).unwrap();
    assert_eq!(finish(&admitted).await, OperationState::Succeeded);
    assert_eq!(server.mutations.load(Ordering::SeqCst), 2);
    connection.close().await;
}

#[tokio::test]
async fn a_full_command_queue_refuses_the_submission_at_once_and_dispatches_nothing_for_it() {
    use xenadmin_core::{Error, OperationId, OperationState, PowerAction, VmCommand};
    // The host task stays pending, so the one admitted operation keeps the guest reserved
    // and every other queued command is refused at admission: exactly one mutation.
    let server = mock_overridden(override_rpc(|request| async move {
        (request["method"] == "task.get_record").then(|| {
            RpcReply::Json(
                json!({"result":{"status":"pending","progress":0.2,"allowed_operations":[]}}),
            )
        })
    }))
    .await;
    let directory = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(directory.path().into()).unwrap();
    let connection = backend.connect(
        ConnectionOptions::new(server.endpoint.clone()),
        Credentials::new("fixture".into(), "fixture".into()),
    );
    let inventory = connection.wait_ready(deadline(3)).await.unwrap();
    let observed = connection
        .latest()
        .observe_vm(inventory.guests().next().unwrap().id)
        .unwrap();
    // Nothing can hold the connection's owner still from here. It opens the journal for
    // each command it takes, so a caller that does not yield fills the queue's 32 places
    // long before they drain.
    let mut queued = Vec::new();
    let refusal = loop {
        match connection.submit(
            OperationId::new(),
            VmCommand::Power {
                vm: observed,
                action: PowerAction::Start,
            },
        ) {
            Ok(operation) => queued.push(operation),
            Err(error) => break error,
        }
        assert!(queued.len() < 10_000, "the command queue never filled");
    };
    assert!(matches!(refusal, Error::Busy), "{refusal}");
    assert!(queued.len() >= 32);
    tokio::time::timeout(deadline(10), async {
        while !queued.iter().all(|operation| {
            let record = operation.latest();
            record.has_task() || record.state.terminal()
        }) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("every queued command must be admitted or refused");
    let (tracked, refused): (Vec<_>, Vec<_>) = queued
        .iter()
        .map(|operation| operation.latest())
        .partition(|record| record.has_task());
    assert_eq!(tracked.len(), 1);
    assert!(
        refused
            .iter()
            .all(|record| record.state == OperationState::Rejected
                && record.detail == Error::ConflictingOperation.to_string())
    );
    assert_eq!(server.mutations.load(Ordering::SeqCst), 1);
    connection.close().await;
}

#[tokio::test]
async fn read_only_connections_do_not_open_the_journal_or_admit_mutations() {
    let server = mock(false, false).await;
    let directory = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(directory.path().into()).unwrap();
    let mut options = ConnectionOptions::new(server.endpoint.clone());
    options.read_only = true;
    let connection = backend.connect(
        options,
        Credentials::new("fixture".into(), "fixture".into()),
    );
    let inventory = connection.wait_ready(deadline(3)).await.unwrap();
    let observed = connection
        .latest()
        .observe_vm(inventory.guests().next().unwrap().id)
        .unwrap();
    assert!(matches!(
        connection.submit(
            xenadmin_core::OperationId::new(),
            xenadmin_core::VmCommand::Power {
                vm: observed,
                action: xenadmin_core::PowerAction::Start
            }
        ),
        Err(xenadmin_core::Error::ReadOnly)
    ));
    assert!(matches!(
        connection.control_operation(
            xenadmin_core::OperationId::new(),
            xenadmin_core::OperationControl::RequestCancellation
        ),
        Err(xenadmin_core::Error::ReadOnly)
    ));
    assert!(matches!(
        connection.submit(
            xenadmin_core::OperationId::new(),
            xenadmin_core::VmCommand::Migrate {
                vm: observed,
                operation: xenadmin_core::VmMigration {
                    destination: *inventory.hosts.keys().next().unwrap(),
                    expected_host: None,
                    expected_affinity: None,
                    live: false,
                    disks: vec![],
                    networks: vec![],
                    transfer_network: None
                }
            }
        ),
        Err(xenadmin_core::Error::ReadOnly)
    ));
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!directory.path().join("operations.sqlite3").exists());
    assert!(matches!(
        connection.open_console(observed),
        Err(xenadmin_core::Error::ReadOnly)
    ));
    assert_eq!(server.mutations.load(Ordering::SeqCst), 0);
    connection.close().await;
}

const SNAPSHOT_UUID: &str = "55555555-5555-5555-5555-555555555555";
const CREATED_UUID: &str = "66666666-6666-6666-6666-666666666666";
const GUEST_UUID: &str = "33333333-3333-3333-3333-333333333333";
#[derive(Default)]
struct SnapshotFaults {
    running_on_preflight: bool,
    reparent_on_preflight: bool,
    wrong_created_parent: bool,
    drop_create_reply: bool,
    stop_verification: bool,
    created: bool,
}
fn snapshot_record(created: bool) -> Value {
    json!({"uuid":if created {CREATED_UUID} else {SNAPSHOT_UUID},"name_label":"Before upgrade","name_description":"Fixture disk snapshot","is_a_snapshot":true,"is_a_template":false,"is_control_domain":false,"snapshot_of":"OpaqueRef:vm","snapshot_time":"20261107T10:00:00Z","power_state":"Halted","allowed_operations":["revert","destroy","clone"]})
}
async fn snapshot_mock(faults: Arc<Mutex<SnapshotFaults>>, calls: Arc<Mutex<Vec<Value>>>) -> Mock {
    mock_overridden(override_rpc(move |request| {
        let faults = faults.clone();
        let calls = calls.clone();
        async move {
            calls.lock().unwrap().push(request.clone());
            let method = request["method"].as_str().unwrap();
            if method == "event.from" {
                if request["params"][2] != "" {return None;}
                let mut initial = batch("initial", "Halted");
                let vm = &mut initial["events"][2]["snapshot"];
                vm["is_a_snapshot"]=json!(false);vm["is_a_template"]=json!(false);vm["is_control_domain"]=json!(false);
                vm["allowed_operations"]=json!(["snapshot","checkpoint","start"]);
                // Deliberately publish the snapshot before its parent in this batch.
                initial["events"].as_array_mut().unwrap().insert(0,json!({"class":"VM","operation":"add","ref":"OpaqueRef:snapshot","snapshot":snapshot_record(false)}));
                return Some(RpcReply::Json(json!({"result":initial})));
            }
            let mut state = faults.lock().unwrap();
            let result = match method {
                "VM.get_record" if request["params"][1] == "OpaqueRef:vm" => {
                    let mut vm=batch("unused","Halted")["events"][2]["snapshot"].clone();
                    vm["is_a_snapshot"]=json!(false);vm["is_a_template"]=json!(false);vm["is_control_domain"]=json!(false);
                    vm["allowed_operations"]=json!(["snapshot","checkpoint","start"]);
                    if state.running_on_preflight {vm["power_state"]=json!("Running");}
                    vm
                }
                "VM.get_record" => {
                    let created=request["params"][1]=="OpaqueRef:created";
                    if created && state.stop_verification {return Some(RpcReply::Json(json!({"error":{"code":1,"message":"HANDLE_INVALID","data":[]}})));}
                    let mut record=snapshot_record(created);
                    if (created && state.wrong_created_parent) || (!created && state.reparent_on_preflight) {record["snapshot_of"]=json!("OpaqueRef:other-vm");}
                    record
                }
                "VM.get_by_uuid" => if request["params"][1]==GUEST_UUID {json!("OpaqueRef:vm")} else {assert_eq!(request["params"][1],SNAPSHOT_UUID);json!("OpaqueRef:snapshot")},
                "Async.VM.snapshot" | "Async.VM.checkpoint" => {
                    state.created=true;
                    if state.drop_create_reply {return Some(RpcReply::Drop);}
                    json!("OpaqueRef:snapshot-task")
                }
                "Async.VM.revert" => json!("OpaqueRef:revert-task"),
                "task.get_record" => json!({"status":"success","progress":1.0,"allowed_operations":[],"result":"<value>OpaqueRef:created</value>"}),
                _=>return None,
            };
            Some(RpcReply::Json(json!({"result":result})))
        }
    })).await
}

#[tokio::test]
async fn snapshot_creation_verifies_result_identity_and_deduplicates_the_full_intent() {
    use xenadmin_core::{OperationId, OperationState, SnapshotName, VmCommand};
    let calls = Arc::new(Mutex::new(Vec::new()));
    let server = snapshot_mock(Default::default(), calls.clone()).await;
    let directory = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(directory.path().into()).unwrap();
    let connection = backend.connect(
        ConnectionOptions::new(server.endpoint.clone()),
        Credentials::new("root".into(), "fixture".into()),
    );
    let inventory = connection.wait_ready(deadline(3)).await.unwrap();
    assert_eq!(inventory.vms.len(), 1);
    let vm = inventory.guests().next().unwrap().id;
    assert_eq!(inventory.snapshots.for_vm(vm).count(), 1);
    let observed = connection.latest().observe_vm(vm).unwrap();
    let id = OperationId::new();
    let create = || VmCommand::CreateSnapshot {
        vm: observed,
        name: SnapshotName::new("  Before maintenance  ").unwrap(),
    };
    let operation = connection.submit(id, create()).unwrap();
    assert_eq!(finish(&operation).await, OperationState::Succeeded);
    assert_eq!(
        operation.latest().created_snapshot.unwrap().to_string(),
        CREATED_UUID
    );
    assert_eq!(
        finish(&connection.submit(id, create()).unwrap()).await,
        OperationState::Succeeded
    );
    let conflict = connection
        .submit(
            id,
            VmCommand::CreateSnapshot {
                vm: observed,
                name: SnapshotName::new("Different intent").unwrap(),
            },
        )
        .unwrap();
    assert_eq!(finish(&conflict).await, OperationState::Rejected);
    let requests = calls.lock().unwrap().clone();
    let writes: Vec<_> = requests
        .iter()
        .filter(|r| r["method"] == "Async.VM.snapshot")
        .collect();
    assert_eq!(writes.len(), 1);
    assert_eq!(
        writes[0]["params"],
        json!([
            "OpaqueRef:fixture-session",
            "OpaqueRef:vm",
            "Before maintenance"
        ])
    );
    connection.close().await;
}

#[tokio::test]
async fn snapshot_restore_uses_snapshot_reference_and_never_starts_the_guest() {
    use xenadmin_core::{OperationId, OperationState, VmCommand};
    let calls = Arc::new(Mutex::new(Vec::new()));
    let server = snapshot_mock(Default::default(), calls.clone()).await;
    let directory = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(directory.path().into()).unwrap();
    let connection = backend.connect(
        ConnectionOptions::new(server.endpoint.clone()),
        Credentials::new("root".into(), "fixture".into()),
    );
    let inventory = connection.wait_ready(deadline(3)).await.unwrap();
    let vm = inventory.guests().next().unwrap().id;
    let observed = connection
        .latest()
        .observe_snapshot(vm, xenadmin_core::SnapshotId::parse(SNAPSHOT_UUID).unwrap())
        .unwrap();
    let operation = connection
        .submit(
            OperationId::new(),
            VmCommand::RevertSnapshot { snapshot: observed },
        )
        .unwrap();
    assert_eq!(finish(&operation).await, OperationState::Succeeded);
    connection.close().await;
    let requests = calls.lock().unwrap();
    let writes: Vec<_> = requests
        .iter()
        .filter(|r| r["method"].as_str().unwrap().starts_with("Async.VM."))
        .collect();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0]["method"], "Async.VM.revert");
    assert_eq!(
        writes[0]["params"],
        json!(["OpaqueRef:fixture-session", "OpaqueRef:snapshot"])
    );
}

#[tokio::test]
async fn snapshot_restore_rechecks_power_state_and_parent_relationship_before_dispatch() {
    use xenadmin_core::{OperationId, OperationState, VmCommand};
    for reparent in [false, true] {
        let faults = Arc::new(Mutex::new(SnapshotFaults {
            running_on_preflight: !reparent,
            reparent_on_preflight: reparent,
            ..Default::default()
        }));
        let calls = Arc::new(Mutex::new(Vec::new()));
        let server = snapshot_mock(faults, calls.clone()).await;
        let directory = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(directory.path().into()).unwrap();
        let connection = backend.connect(
            ConnectionOptions::new(server.endpoint.clone()),
            Credentials::new("root".into(), "fixture".into()),
        );
        let inventory = connection.wait_ready(deadline(3)).await.unwrap();
        let vm = inventory.guests().next().unwrap().id;
        let snapshot = connection
            .latest()
            .observe_snapshot(vm, xenadmin_core::SnapshotId::parse(SNAPSHOT_UUID).unwrap())
            .unwrap();
        let operation = connection
            .submit(OperationId::new(), VmCommand::RevertSnapshot { snapshot })
            .unwrap();
        assert_eq!(finish(&operation).await, OperationState::Rejected);
        assert!(
            !calls
                .lock()
                .unwrap()
                .iter()
                .any(|r| r["method"] == "Async.VM.revert")
        );
        connection.close().await;
    }
}

#[tokio::test]
async fn unverified_snapshot_creation_preserves_evidence_and_blocks_conflicting_writes() {
    use xenadmin_core::{OperationId, OperationState, SnapshotName, VmCommand};
    for drop_reply in [false, true] {
        let faults = Arc::new(Mutex::new(SnapshotFaults {
            wrong_created_parent: !drop_reply,
            drop_create_reply: drop_reply,
            ..Default::default()
        }));
        let calls = Arc::new(Mutex::new(Vec::new()));
        let server = snapshot_mock(faults, calls.clone()).await;
        let directory = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(directory.path().into()).unwrap();
        let connection = backend.connect(
            ConnectionOptions::new(server.endpoint.clone()),
            Credentials::new("root".into(), "fixture".into()),
        );
        let inventory = connection.wait_ready(deadline(3)).await.unwrap();
        let vm = connection
            .latest()
            .observe_vm(inventory.guests().next().unwrap().id)
            .unwrap();
        let create = || VmCommand::CreateSnapshot {
            vm,
            name: SnapshotName::new("Before maintenance").unwrap(),
        };
        let operation = connection.submit(OperationId::new(), create()).unwrap();
        assert_eq!(finish(&operation).await, OperationState::OutcomeUnknown);
        assert_eq!(operation.latest().created_snapshot, None);
        let duplicate = connection.submit(OperationId::new(), create()).unwrap();
        assert_eq!(finish(&duplicate).await, OperationState::Rejected);
        assert_eq!(
            calls
                .lock()
                .unwrap()
                .iter()
                .filter(|r| r["method"] == "Async.VM.snapshot")
                .count(),
            1
        );
        assert!(
            !calls
                .lock()
                .unwrap()
                .iter()
                .any(|r| r["method"] == "task.destroy")
        );
        connection.close().await;
    }
}

#[tokio::test]
async fn saved_snapshot_result_can_be_verified_after_reconnect_without_the_task_record() {
    use xenadmin_core::{OperationControl, OperationId, OperationState, SnapshotName, VmCommand};
    let faults = Arc::new(Mutex::new(SnapshotFaults {
        stop_verification: true,
        ..Default::default()
    }));
    let calls = Arc::new(Mutex::new(Vec::new()));
    let server = snapshot_mock(faults.clone(), calls.clone()).await;
    let directory = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(directory.path().into()).unwrap();
    let connect = || {
        backend.connect(
            ConnectionOptions::new(server.endpoint.clone()),
            Credentials::new("root".into(), "fixture".into()),
        )
    };
    let first = connect();
    let inventory = first.wait_ready(deadline(3)).await.unwrap();
    let vm = first
        .latest()
        .observe_vm(inventory.guests().next().unwrap().id)
        .unwrap();
    let id = OperationId::new();
    let operation = first
        .submit(
            id,
            VmCommand::CreateSnapshot {
                vm,
                name: SnapshotName::new("Before maintenance").unwrap(),
            },
        )
        .unwrap();
    assert_eq!(finish(&operation).await, OperationState::OutcomeUnknown);
    first.close().await;
    faults.lock().unwrap().stop_verification = false;
    let task_reads = calls
        .lock()
        .unwrap()
        .iter()
        .filter(|r| r["method"] == "task.get_record")
        .count();
    let second = connect();
    second.wait_ready(deadline(3)).await.unwrap();
    second
        .control_operation(id, OperationControl::Recheck)
        .unwrap();
    tokio::time::timeout(deadline(5), async {
        loop {
            if second.operation_history().iter().any(|r| {
                r.id == id && r.state == OperationState::Succeeded && r.created_snapshot.is_some()
            }) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    second.close().await;
    let calls = calls.lock().unwrap();
    assert_eq!(
        calls
            .iter()
            .filter(|r| r["method"] == "Async.VM.snapshot")
            .count(),
        1
    );
    assert_eq!(
        calls
            .iter()
            .filter(|r| r["method"] == "task.get_record")
            .count(),
        task_reads
    );
}

#[tokio::test]
async fn snapshot_observations_are_connection_scoped_and_read_only_mode_cannot_mutate() {
    use xenadmin_core::{Error, OperationId, SnapshotId, SnapshotName, VmCommand};
    let calls = Arc::new(Mutex::new(Vec::new()));
    let server = snapshot_mock(Default::default(), calls.clone()).await;
    let directory = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(directory.path().into()).unwrap();
    let options = ConnectionOptions::new(server.endpoint.clone());
    let first = backend.connect(
        options.clone(),
        Credentials::new("root".into(), "fixture".into()),
    );
    let inventory = first.wait_ready(deadline(3)).await.unwrap();
    let vm = inventory.guests().next().unwrap().id;
    let id = SnapshotId::parse(SNAPSHOT_UUID).unwrap();
    let snapshot = first.latest().observe_snapshot(vm, id).unwrap();
    assert!(
        first
            .latest()
            .observe_snapshot(vm, SnapshotId::parse(CREATED_UUID).unwrap())
            .is_err()
    );
    first.close().await;
    let second = backend.connect(
        options.clone(),
        Credentials::new("root".into(), "fixture".into()),
    );
    second.wait_ready(deadline(3)).await.unwrap();
    assert!(matches!(
        second.submit(OperationId::new(), VmCommand::RevertSnapshot { snapshot }),
        Err(Error::Stale)
    ));
    second.close().await;
    let mut options = options;
    options.read_only = true;
    let read_only = backend.connect(options, Credentials::new("root".into(), "fixture".into()));
    read_only.wait_ready(deadline(3)).await.unwrap();
    let observed = read_only.latest().observe_vm(vm).unwrap();
    assert!(matches!(
        read_only.submit(
            OperationId::new(),
            VmCommand::CreateSnapshot {
                vm: observed,
                name: SnapshotName::new("Fixture").unwrap()
            }
        ),
        Err(Error::ReadOnly)
    ));
    let snapshot = read_only.latest().observe_snapshot(vm, id).unwrap();
    assert!(matches!(
        read_only.submit(OperationId::new(), VmCommand::RevertSnapshot { snapshot }),
        Err(Error::ReadOnly)
    ));
    assert!(!directory.path().join("operations.sqlite3").exists());
    assert!(
        !calls
            .lock()
            .unwrap()
            .iter()
            .any(|r| r["method"].as_str().unwrap().starts_with("Async."))
    );
    first.close().await;
    second.close().await;
    read_only.close().await;
}

async fn settings_mock(
    fail_description: bool,
    lose_name_reply: bool,
    fail_read: Option<usize>,
) -> (Mock, Arc<Mutex<Value>>) {
    let mut initial = batch("initial", "Halted")["events"][2]["snapshot"].clone();
    initial["is_a_snapshot"] = json!(false);
    initial["is_a_template"] = json!(false);
    initial["is_control_domain"] = json!(false);
    initial["name_description"] = json!("Original description");
    initial["current_operations"] = json!({});
    initial["other_config"] = json!({"auto_poweron":"false","preserve_this":"value"});
    let record = Arc::new(Mutex::new(initial));
    let captured = record.clone();
    let reads = Arc::new(AtomicUsize::new(0));
    let server=mock_overridden(override_rpc(move|request|{
        let record=captured.clone();
        let reads=reads.clone();
        async move {
            let method=request["method"].as_str().unwrap();
            let mut current=record.lock().unwrap();
            let result=match method {
                "event.from" if request["params"][2]=="" => {
                    let mut initial=batch("initial","Halted");
                    initial["events"][2]["snapshot"]=current.clone();
                    initial
                },
                "VM.get_record"=>{
                    let number=reads.fetch_add(1,Ordering::SeqCst)+1;
                    if fail_read==Some(number){return Some(RpcReply::Json(json!({"error":{"code":1,"message":"RBAC_PERMISSION_DENIED","data":[]}})))}
                    current.clone()
                },
                "VM.get_by_uuid"=>json!("OpaqueRef:vm"),
                "VM.set_name_label"=>{
                    current["name_label"]=request["params"][2].clone();
                    if current["fixture_flip_kind"]==true {current["is_a_template"]=json!(false);}
                    if lose_name_reply{return Some(RpcReply::Drop)}
                    Value::Null
                },
                "VM.set_name_description"=>{
                    if fail_description{return Some(RpcReply::Json(json!({"error":{"code":1,"message":"RBAC_PERMISSION_DENIED","data":[]}})))}
                    current["name_description"]=request["params"][2].clone();Value::Null
                },
                "VM.set_other_config"=>{current["other_config"]=request["params"][2].clone();Value::Null},
                "host.get_by_uuid"=>{
                    if request["params"][1] != "22222222-2222-2222-2222-222222222222" || current["fixture_missing_host"] == true {return Some(RpcReply::Json(json!({"error":{"message":"UUID_INVALID"}})))}
                    json!("OpaqueRef:host")
                },
                "host.get_record"=>{
                    if request["params"][1] != "OpaqueRef:host" || current["fixture_missing_host"] == true {return Some(RpcReply::Json(json!({"error":{"message":"HANDLE_INVALID"}})))}
                    json!({"uuid":"22222222-2222-2222-2222-222222222222"})
                },
                "VM.set_affinity"=>{current["affinity"]=request["params"][2].clone();if lose_name_reply{return Some(RpcReply::Drop)}Value::Null},
                "VM.set_VCPUs_number_live"=>{current["VCPUs_at_startup"]=request["params"][2].clone();if lose_name_reply{return Some(RpcReply::Drop)}Value::Null},
                "VM.set_memory_dynamic_range"=>{current["memory_dynamic_min"]=request["params"][2].clone();current["memory_dynamic_max"]=request["params"][3].clone();Value::Null},
                "VM.set_platform"=>{current["platform"]=request["params"][2].clone();if current["fixture_lose_platform"]==true{return Some(RpcReply::Drop)}Value::Null},
                "VM.set_HVM_boot_params"=>{current["HVM_boot_params"]=request["params"][2].clone();Value::Null},
                "pool.get_by_uuid"=>json!("OpaqueRef:pool"),
                "pool.get_record"=>batch("initial","Halted")["events"][0]["snapshot"].clone(),
                "VM.get_secureboot_readiness"=>json!("setup_mode"),
                "pool.get_guest_secureboot_readiness"=>json!("ready"),
                "VM.set_VCPUs_params"=>{current["VCPUs_params"]=request["params"][2].clone();Value::Null},
                "VM.set_HVM_shadow_multiplier"|"VM.set_shadow_multiplier_live"=>{assert!(request["params"][2].is_number());current["HVM_shadow_multiplier"]=request["params"][2].clone();Value::Null},
                _=>return None,
            };
            Some(RpcReply::Json(json!({"result":result})))
        }
    })).await;
    (server, record)
}

#[tokio::test]
async fn settings_changes_preserve_other_map_keys_and_record_verified_steps() {
    use xenadmin_core::{
        OperationId, OperationState, StepState, VmCommand, VmSettingEdit, VmSettingsPatch,
    };
    let (server, record) = settings_mock(false, false, None).await;
    let directory = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(directory.path().into()).unwrap();
    let connection = backend.connect(
        ConnectionOptions::new(server.endpoint.clone()),
        Credentials::new("root".into(), "fixture".into()),
    );
    let inventory = connection.wait_ready(deadline(3)).await.unwrap();
    let vm = connection
        .latest()
        .observe_vm(inventory.guests().next().unwrap().id)
        .unwrap();
    let operation = connection
        .submit(
            OperationId::new(),
            VmCommand::Configure {
                vm,
                patch: VmSettingsPatch::new(vec![
                    VmSettingEdit::Name {
                        previous: "XO recovery guest".into(),
                        value: "Renamed fixture".into(),
                    },
                    VmSettingEdit::Autostart {
                        previous: false,
                        value: true,
                    },
                ])
                .unwrap(),
            },
        )
        .unwrap();
    assert_eq!(finish(&operation).await, OperationState::Succeeded);
    assert_eq!(
        record.lock().unwrap()["other_config"],
        json!({"auto_poweron":"true","preserve_this":"value"})
    );
    assert_eq!(operation.latest().steps.len(), 2);
    assert!(
        operation
            .latest()
            .steps
            .iter()
            .all(|s| s.state == StepState::Verified)
    );
    connection.close().await;
}

#[tokio::test]
async fn settings_failure_retains_applied_values_and_stops_later_steps() {
    use xenadmin_core::{
        OperationId, OperationState, StepState, VmCommand, VmSettingEdit, VmSettingsPatch,
    };
    let (server, record) = settings_mock(true, false, None).await;
    let directory = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(directory.path().into()).unwrap();
    let connection = backend.connect(
        ConnectionOptions::new(server.endpoint.clone()),
        Credentials::new("root".into(), "fixture".into()),
    );
    let inventory = connection.wait_ready(deadline(3)).await.unwrap();
    let vm = connection
        .latest()
        .observe_vm(inventory.guests().next().unwrap().id)
        .unwrap();
    let operation = connection
        .submit(
            OperationId::new(),
            VmCommand::Configure {
                vm,
                patch: VmSettingsPatch::new(vec![
                    VmSettingEdit::Name {
                        previous: "XO recovery guest".into(),
                        value: "Renamed fixture".into(),
                    },
                    VmSettingEdit::Description {
                        previous: "Original description".into(),
                        value: "New description".into(),
                    },
                    VmSettingEdit::Autostart {
                        previous: false,
                        value: true,
                    },
                ])
                .unwrap(),
            },
        )
        .unwrap();
    assert_eq!(finish(&operation).await, OperationState::PartiallyCompleted);
    assert_eq!(record.lock().unwrap()["name_label"], "Renamed fixture");
    assert_eq!(
        record.lock().unwrap()["other_config"]["auto_poweron"],
        "false"
    );
    assert_eq!(operation.latest().steps[0].state, StepState::Verified);
    assert_eq!(operation.latest().steps[1].state, StepState::Failed);
    assert_eq!(operation.latest().steps[2].state, StepState::Pending);
    connection.close().await;
}

#[tokio::test]
async fn lost_settings_response_reconciles_values_without_repeating_the_write() {
    use xenadmin_core::{
        OperationControl, OperationId, OperationState, VmCommand, VmSettingEdit, VmSettingsPatch,
    };
    // Exercise Recheck at the outcome-publication/worker-exit boundary repeatedly.
    for _ in 0..16 {
        let (server, _) = settings_mock(false, true, None).await;
        let directory = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(directory.path().into()).unwrap();
        let connection = backend.connect(
            ConnectionOptions::new(server.endpoint.clone()),
            Credentials::new("root".into(), "fixture".into()),
        );
        let inventory = connection.wait_ready(deadline(3)).await.unwrap();
        let vm = connection
            .latest()
            .observe_vm(inventory.guests().next().unwrap().id)
            .unwrap();
        let id = OperationId::new();
        let operation = connection
            .submit(
                id,
                VmCommand::Configure {
                    vm,
                    patch: VmSettingsPatch::new(vec![VmSettingEdit::Name {
                        previous: "XO recovery guest".into(),
                        value: "Renamed fixture".into(),
                    }])
                    .unwrap(),
                },
            )
            .unwrap();
        tokio::time::timeout(deadline(3), async {
            while operation.latest().state != OperationState::OutcomeUnknown {
                tokio::time::sleep(Duration::from_millis(10)).await
            }
        })
        .await
        .unwrap();
        connection
            .control_operation(id, OperationControl::Recheck)
            .unwrap();
        tokio::time::timeout(deadline(3), async {
            while !connection
                .operation_history()
                .iter()
                .any(|r| r.id == id && r.state == OperationState::Succeeded)
            {
                tokio::time::sleep(Duration::from_millis(10)).await
            }
        })
        .await
        .unwrap();
        assert_eq!(
            server
                .methods
                .lock()
                .unwrap()
                .iter()
                .filter(|m| m.as_str() == "VM.set_name_label")
                .count(),
            1
        );
        connection.close().await;
    }
}

#[tokio::test]
async fn chosen_host_start_checks_destination_before_dispatch_and_preserves_unknown_writes() {
    use xenadmin_core::{HostId, OperationId, OperationState, PowerAction, VmCommand};
    for lose_reply in [false, true] {
        let calls = Arc::new(Mutex::new(Vec::<Value>::new()));
        let capture = calls.clone();
        let server = mock_overridden(override_rpc(move |request| {
            let calls = capture.clone();
            async move {
                calls.lock().unwrap().push(request.clone());
                let mut record = batch("initial", "Halted")["events"][2]["snapshot"].clone();
                record["is_a_snapshot"] = json!(false);
                record["is_a_template"] = json!(false);
                record["is_control_domain"] = json!(false);
                record["allowed_operations"] = json!(["start_on"]);
                let result = match request["method"].as_str().unwrap() {
                    "event.from" if request["params"][2] == "" => {
                        let mut initial = batch("initial", "Halted");
                        initial["events"][2]["snapshot"] = record;
                        initial
                    }
                    "VM.get_by_uuid" => json!("OpaqueRef:vm"),
                    "VM.get_record" => record,
                    "host.get_by_uuid" => json!("OpaqueRef:host"),
                    "host.get_record" => {
                        json!({"uuid":"22222222-2222-2222-2222-222222222222","enabled":true})
                    }
                    "VM.assert_operation_valid" | "VM.assert_can_boot_here" => Value::Null,
                    "Async.VM.start_on" => {
                        assert_eq!(
                            request["params"].as_array().unwrap()[1..],
                            json!(["OpaqueRef:vm", "OpaqueRef:host", false, false])
                                .as_array()
                                .unwrap()[..]
                        );
                        if lose_reply {
                            return Some(RpcReply::Drop);
                        }
                        json!("OpaqueRef:task")
                    }
                    _ => return None,
                };
                Some(RpcReply::Json(json!({"result":result})))
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
        let vm = connection
            .latest()
            .observe_vm(inventory.guests().next().unwrap().id)
            .unwrap();
        let operation = connection
            .submit(
                OperationId::new(),
                VmCommand::PowerOnHost {
                    vm,
                    action: PowerAction::Start,
                    host: HostId::parse("22222222-2222-2222-2222-222222222222").unwrap(),
                },
            )
            .unwrap();
        assert_eq!(
            finish(&operation).await,
            if lose_reply {
                OperationState::OutcomeUnknown
            } else {
                OperationState::Succeeded
            }
        );
        let methods: Vec<_> = calls
            .lock()
            .unwrap()
            .iter()
            .map(|r| r["method"].as_str().unwrap().to_owned())
            .collect();
        assert!(
            methods
                .iter()
                .position(|m| m == "VM.assert_can_boot_here")
                .unwrap()
                < methods
                    .iter()
                    .position(|m| m == "Async.VM.start_on")
                    .unwrap()
        );
        assert_eq!(
            methods.iter().filter(|m| *m == "Async.VM.start_on").count(),
            1
        );
        connection.close().await;
    }
}

#[tokio::test]
async fn memory_snapshot_uses_checkpoint_and_verifies_the_result_without_starting_vm() {
    use xenadmin_core::{OperationId, OperationState, SnapshotName, VmCommand};
    let calls = Arc::new(Mutex::new(Vec::new()));
    let server = snapshot_mock(Default::default(), calls.clone()).await;
    let directory = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(directory.path().into()).unwrap();
    let connection = backend.connect(
        ConnectionOptions::new(server.endpoint.clone()),
        Credentials::new("root".into(), "fixture".into()),
    );
    let inventory = connection.wait_ready(deadline(3)).await.unwrap();
    let vm = connection
        .latest()
        .observe_vm(inventory.guests().next().unwrap().id)
        .unwrap();
    let operation = connection
        .submit(
            OperationId::new(),
            VmCommand::CreateCheckpoint {
                vm,
                name: SnapshotName::new("Memory fixture").unwrap(),
            },
        )
        .unwrap();
    assert_eq!(finish(&operation).await, OperationState::Succeeded);
    assert!(operation.latest().created_snapshot.is_some());
    assert_eq!(
        calls
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r["method"] == "Async.VM.checkpoint")
            .count(),
        1
    );
    assert!(
        !calls
            .lock()
            .unwrap()
            .iter()
            .any(|r| r["method"] == "Async.VM.snapshot" || r["method"] == "Async.VM.start")
    );
    connection.close().await;
}

#[tokio::test]
async fn settings_read_errors_do_not_report_an_applied_mutation_as_failed() {
    use xenadmin_core::{
        OperationId, OperationState, StepState, VmCommand, VmSettingEdit, VmSettingsPatch,
    };
    for (read, state) in [
        (3, OperationState::OutcomeUnknown),
        (4, OperationState::PartiallyCompleted),
    ] {
        let (server, record) = settings_mock(false, false, Some(read)).await;
        let directory = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(directory.path().into()).unwrap();
        let connection = backend.connect(
            ConnectionOptions::new(server.endpoint.clone()),
            Credentials::new("root".into(), "fixture".into()),
        );
        let inventory = connection.wait_ready(deadline(3)).await.unwrap();
        let vm = connection
            .latest()
            .observe_vm(inventory.guests().next().unwrap().id)
            .unwrap();
        let operation = connection
            .submit(
                OperationId::new(),
                VmCommand::Configure {
                    vm,
                    patch: VmSettingsPatch::new(vec![
                        VmSettingEdit::Name {
                            previous: "XO recovery guest".into(),
                            value: "Renamed fixture".into(),
                        },
                        VmSettingEdit::Description {
                            previous: "Original description".into(),
                            value: "New description".into(),
                        },
                    ])
                    .unwrap(),
                },
            )
            .unwrap();
        assert_eq!(finish(&operation).await, state);
        assert_eq!(record.lock().unwrap()["name_label"], "Renamed fixture");
        assert_eq!(
            record.lock().unwrap()["name_description"],
            "Original description"
        );
        assert_eq!(
            operation.latest().steps[0].state,
            if read == 3 {
                StepState::OutcomeUnknown
            } else {
                StepState::Verified
            }
        );
        assert_eq!(
            server
                .methods
                .lock()
                .unwrap()
                .iter()
                .filter(|m| m.as_str() == "VM.set_name_label")
                .count(),
            1
        );
        assert!(
            !server
                .methods
                .lock()
                .unwrap()
                .iter()
                .any(|m| m == "VM.set_name_description")
        );
        connection.close().await;
    }
}

#[path = "fixtures/activity.rs"]
mod activity;
#[path = "fixtures/resource_administration.rs"]
mod resource_administration;

#[path = "fixtures/advanced_properties.rs"]
mod advanced_properties;
#[path = "fixtures/network_administration.rs"]
mod network_administration;
#[path = "fixtures/storage_lifecycle.rs"]
mod storage_lifecycle;

#[path = "fixtures/pool_administration.rs"]
mod pool_administration;

#[path = "fixtures/appliances.rs"]
mod appliances;
#[path = "fixtures/host_administration.rs"]
mod host_administration;
#[path = "fixtures/organization.rs"]
mod organization;
#[path = "fixtures/pool_security.rs"]
mod pool_security;

#[path = "fixtures/snapshot_policy.rs"]
mod snapshot_policy;

fn journal_text(directory: &std::path::Path) -> String {
    let db = rusqlite::Connection::open_with_flags(
        directory.join("operations.sqlite3"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    db.busy_timeout(Duration::from_secs(5)).unwrap();
    let mut query = db.prepare("SELECT payload || coalesce(task,'') FROM operations UNION ALL SELECT payload || coalesce(task,'') || coalesce(result_reference,'') FROM resource_operations").unwrap();
    query
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect::<Vec<_>>()
        .join("\n")
}
#[path = "fixtures/cross_pool.rs"]
mod cross_pool;
#[path = "fixtures/cross_pool_batch.rs"]
mod cross_pool_batch;
#[path = "fixtures/pool_join.rs"]
mod pool_join;

#[path = "fixtures/firmware_properties.rs"]
mod firmware_properties;

#[path = "fixtures/cloud_init.rs"]
mod cloud_init;
#[path = "fixtures/host_console.rs"]
mod host_console;
#[path = "fixtures/storage_authentication.rs"]
mod storage_authentication;

#[path = "fixtures/host_power.rs"]
mod host_power;
#[path = "fixtures/network_topology.rs"]
mod network_topology;

#[path = "fixtures/root_password.rs"]
mod root_password;

#[path = "fixtures/snapshot_preview.rs"]
mod snapshot_preview;

#[path = "fixtures/alarms.rs"]
mod alarms;

#[path = "fixtures/network_fabric.rs"]
mod network_fabric;
