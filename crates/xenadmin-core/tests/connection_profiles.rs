use serde_json::{Value, json};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
};
use xenadmin_core::{Backend, ConnectionOptions, Credentials, Endpoint, PoolId};
#[path = "fixtures/deadlines.rs"]
mod deadlines;
use deadlines::deadline;

const POOL: &str = "11111111-1111-1111-1111-111111111111";
struct Server {
    endpoint: Endpoint,
    logins: Arc<AtomicUsize>,
    logouts: Arc<AtomicUsize>,
    requests: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.requests.abort();
    }
}
async fn server(pool: &'static str) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = Endpoint::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
    let logins = Arc::new(AtomicUsize::new(0));
    let logouts = Arc::new(AtomicUsize::new(0));
    let requests = tokio::spawn({
        let logins = logins.clone();
        let logouts = logouts.clone();
        async move {
            while let Ok((stream, _)) = listener.accept().await {
                let logins = logins.clone();
                let logouts = logouts.clone();
                tokio::spawn(async move {
                    let mut reader = BufReader::new(stream);
                    let mut length = 0;
                    loop {
                        let mut line = String::new();
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
                    let result = match request["method"].as_str().unwrap() {
                        "session.login_with_password" => {
                            logins.fetch_add(1, Ordering::SeqCst);
                            json!("OpaqueRef:session")
                        }
                        "session.logout" => {
                            logouts.fetch_add(1, Ordering::SeqCst);
                            Value::Null
                        }
                        "event.from" if request["params"][2] == "" => {
                            json!({"token":"ready","events":[
                                {"class":"pool","operation":"add","ref":"OpaqueRef:pool","snapshot":{"uuid":pool,"name_label":"Test pool","master":"OpaqueRef:host"}},
                                {"class":"host","operation":"add","ref":"OpaqueRef:host","snapshot":{"uuid":"22222222-2222-2222-2222-222222222222","name_label":"Test host","address":"192.0.2.10","enabled":true}},
                                {"class":"VM","operation":"add","ref":"OpaqueRef:vm","snapshot":{"uuid":"33333333-3333-3333-3333-333333333333","name_label":"Test guest","power_state":"Halted","VCPUs_at_startup":"2","memory_static_max":"2147483648","allowed_operations":["start"]}}
                            ]})
                        }
                        "event.from" => {
                            tokio::time::sleep(Duration::from_secs(60)).await;
                            json!({"token":"ready","events":[]})
                        }
                        other => panic!("unexpected method {other}"),
                    };
                    let body = serde_json::to_vec(
                        &json!({"jsonrpc":"2.0","id":request["id"],"result":result}),
                    )
                    .unwrap();
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
    Server {
        endpoint,
        logins,
        logouts,
        requests,
    }
}
fn credentials(user: &str) -> Credentials {
    Credentials::new(user.into(), "synthetic-password".into())
}

#[tokio::test]
async fn dropping_a_handle_does_not_disconnect_other_handles() {
    let server = server(POOL).await;
    let directory = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(directory.path().into()).unwrap();
    let connection = backend.connect(
        ConnectionOptions::new(server.endpoint.clone()),
        credentials("root"),
    );
    connection.wait_ready(deadline(3)).await.unwrap();
    let another = connection.clone();
    drop(connection);
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(another.latest().is_current());
    assert_eq!(server.logouts.load(Ordering::SeqCst), 0);
    another.close().await;
    assert_eq!(server.logouts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn authenticated_aliases_share_one_inventory_and_keep_owner_alive() {
    let first = server(POOL).await;
    let second = server(POOL).await;
    let directory = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(directory.path().into()).unwrap();
    let owner = backend.connect(
        ConnectionOptions::new(first.endpoint.clone()),
        credentials("root"),
    );
    owner.wait_ready(deadline(3)).await.unwrap();
    let alias = backend.connect(
        ConnectionOptions::new(second.endpoint.clone()),
        credentials("root"),
    );
    alias.wait_ready(deadline(3)).await.unwrap();
    assert_eq!(owner.latest().generation(), alias.latest().generation());
    assert_eq!(first.logins.load(Ordering::SeqCst), 1);
    assert_eq!(second.logins.load(Ordering::SeqCst), 1);
    assert_eq!(second.logouts.load(Ordering::SeqCst), 1);
    drop(owner);
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(alias.latest().is_current());
    assert_eq!(first.logouts.load(Ordering::SeqCst), 0);
    alias.close().await;
}

#[tokio::test]
async fn alias_with_different_identity_or_access_cannot_borrow_owner_authority() {
    let server = server(POOL).await;
    let directory = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(directory.path().into()).unwrap();
    let owner = backend.connect(
        ConnectionOptions::new(server.endpoint.clone()),
        credentials("root"),
    );
    owner.wait_ready(deadline(3)).await.unwrap();
    let other = backend.connect(
        ConnectionOptions::new(server.endpoint.clone()),
        credentials("observer"),
    );
    let error = other.wait_ready(deadline(3)).await.unwrap_err().to_string();
    assert!(error.contains("different credentials"));
    assert!(!error.contains("synthetic-password"));
    assert!(owner.latest().is_current());
    let mut options = ConnectionOptions::new(server.endpoint.clone());
    options.read_only = true;
    let read_only = backend.connect(options, credentials("root"));
    assert!(read_only.wait_ready(deadline(3)).await.is_err());
    assert!(owner.latest().is_current());
    owner.close().await;
}

#[tokio::test]
async fn profile_pool_identity_is_checked_before_publishing_ready() {
    let server = server("99999999-9999-9999-9999-999999999999").await;
    let directory = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(directory.path().into()).unwrap();
    let mut options = ConnectionOptions::new(server.endpoint.clone());
    options.expected_pool = Some(PoolId::parse(POOL).unwrap());
    let connection = backend.connect(options, credentials("root"));
    assert!(
        connection
            .wait_ready(deadline(3))
            .await
            .unwrap_err()
            .to_string()
            .contains("different pool")
    );
    assert!(connection.latest().inventory().is_none());
}

#[tokio::test]
async fn remembered_member_is_tried_when_primary_is_unreachable() {
    let server = server(POOL).await;
    let unavailable = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint =
        Endpoint::parse(&format!("http://{}", unavailable.local_addr().unwrap())).unwrap();
    drop(unavailable);
    let directory = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(directory.path().into()).unwrap();
    let mut options = ConnectionOptions::new(endpoint);
    options.expected_pool = Some(PoolId::parse(POOL).unwrap());
    options.fallback_endpoints.push(server.endpoint.clone());
    let connection = backend.connect(options, credentials("root"));
    connection.wait_ready(deadline(5)).await.unwrap();
    assert_eq!(
        connection.latest().active_endpoint(),
        Some(&server.endpoint)
    );
    assert!(
        connection
            .latest()
            .verified_members()
            .contains(&server.endpoint)
    );
    connection.close().await;
}
