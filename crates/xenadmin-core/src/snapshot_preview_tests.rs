use super::*;
use crate::{ConnectionOptions, Credentials, Endpoint};
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
};
const PNG: &[u8] = b"\x89PNG\r\n\x1a\nfixture payload";
fn uuid(n: u128) -> String {
    uuid::Uuid::from_u128(n).to_string()
}
fn observed() -> ObservedSnapshot {
    ObservedSnapshot {
        pool: uuid::Uuid::from_u128(1).into(),
        generation: 1,
        vm: uuid::Uuid::from_u128(2).into(),
        id: uuid::Uuid::from_u128(3).into(),
    }
}
#[derive(Clone, Copy)]
enum Mode {
    Blob,
    Inline,
    Absent,
    WrongParent,
    ChangedParent,
    ChangedBlob,
    ChangedMetadata,
    Oversized,
    Redirect,
    Truncated,
    WrongType,
}
struct Fixture {
    options: ConnectionOptions,
    server: tokio::task::JoinHandle<()>,
    calls: Arc<Mutex<Vec<String>>>,
    downloads: Arc<AtomicUsize>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}
impl Fixture {
    async fn new(mode: Mode) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let options = ConnectionOptions::new(
            Endpoint::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap(),
        );
        let calls = Arc::new(Mutex::new(Vec::new()));
        let downloads = Arc::new(AtomicUsize::new(0));
        let server = tokio::spawn({
            let calls = calls.clone();
            let downloads = downloads.clone();
            let snapshots = Arc::new(AtomicUsize::new(0));
            let blobs = Arc::new(AtomicUsize::new(0));
            async move {
                loop {
                    let (stream, _) = listener.accept().await.unwrap();
                    let calls = calls.clone();
                    let downloads = downloads.clone();
                    let snapshots = snapshots.clone();
                    let blobs = blobs.clone();
                    tokio::spawn(async move {
                        let mut reader = BufReader::new(stream);
                        let mut first = String::new();
                        reader.read_line(&mut first).await.unwrap();
                        let mut length = 0;
                        let mut cookie = String::new();
                        loop {
                            let mut line = String::new();
                            reader.read_line(&mut line).await.unwrap();
                            if line == "\r\n" {
                                break;
                            }
                            if let Some((key, value)) = line.split_once(':') {
                                if key.eq_ignore_ascii_case("content-length") {
                                    length = value.trim().parse().unwrap();
                                }
                                if key.eq_ignore_ascii_case("cookie") {
                                    cookie = value.trim().to_owned();
                                }
                            }
                        }
                        let (status, body, extra) = if first.starts_with("GET ") {
                            downloads.fetch_add(1, Ordering::SeqCst);
                            assert_eq!(cookie, "session_id=OpaqueRef:preview-session");
                            assert!(first.starts_with(&format!("GET /blob?uuid={} ", uuid(4))));
                            assert!(!first.contains("preview-session"));
                            if matches!(mode, Mode::Redirect) {
                                (
                                    "302 Found",
                                    Vec::new(),
                                    "Location: http://127.0.0.1:1/credential-sink\r\n",
                                )
                            } else {
                                (
                                    "200 OK",
                                    if matches!(mode, Mode::Truncated) {
                                        PNG[..8].to_vec()
                                    } else {
                                        PNG.to_vec()
                                    },
                                    "",
                                )
                            }
                        } else {
                            assert!(length < 16 * 1024);
                            let mut data = vec![0; length];
                            reader.read_exact(&mut data).await.unwrap();
                            let request: Value = serde_json::from_slice(&data).unwrap();
                            let method = request["method"].as_str().unwrap();
                            calls.lock().unwrap().push(method.into());
                            let value = match method {
                                "session.login_with_password" => json!("OpaqueRef:preview-session"),
                                "session.get_record" => json!({"is_local_superuser":true}),
                                "pool.get_by_uuid" => json!("OpaqueRef:pool"),
                                "pool.get_record" => json!({"uuid":uuid(1)}),
                                "VM.get_by_uuid" => {
                                    if request["params"][1] == uuid(2) {
                                        json!("OpaqueRef:parent")
                                    } else {
                                        json!("OpaqueRef:snapshot")
                                    }
                                }
                                "VM.get_record" => {
                                    if request["params"][1] == "OpaqueRef:parent" {
                                        json!({"uuid":uuid(2),"is_a_snapshot":false,"is_a_template":false,"is_control_domain":false})
                                    } else {
                                        let count = snapshots.fetch_add(1, Ordering::SeqCst);
                                        let mut record = json!({"uuid":uuid(3),"is_a_snapshot":true,"is_control_domain":false,"snapshot_of":if matches!(mode,Mode::WrongParent)||(matches!(mode,Mode::ChangedParent)&&count>0){"OpaqueRef:foreign"}else{"OpaqueRef:parent"},"blobs":{},"other_config":{}});
                                        if matches!(mode, Mode::Inline) {
                                            record["other_config"][KEY] = json!(
                                                base64::engine::general_purpose::STANDARD
                                                    .encode(PNG)
                                            );
                                        } else if !matches!(mode, Mode::Absent) {
                                            record["blobs"][KEY] =
                                                json!(if matches!(mode, Mode::ChangedBlob)
                                                    && count > 0
                                                {
                                                    "OpaqueRef:other"
                                                } else {
                                                    "OpaqueRef:blob"
                                                });
                                        }
                                        record
                                    }
                                }
                                "blob.get_record" => {
                                    let count = blobs.fetch_add(1, Ordering::SeqCst);
                                    json!({"uuid":uuid(if matches!(mode,Mode::ChangedMetadata)&&count>0{5}else{4}),"size":if matches!(mode,Mode::Oversized){MAX_BYTES+1}else{PNG.len()},"mime_type":if matches!(mode,Mode::WrongType){"image/jpeg"}else{"image/png"},"last_updated":"20261108T00:00:00Z"})
                                }
                                _ => panic!("Unexpected method {method}"),
                            };
                            (
                                "200 OK",
                                serde_json::to_vec(
                                    &json!({"jsonrpc":"2.0","id":request["id"],"result":value}),
                                )
                                .unwrap(),
                                "",
                            )
                        };
                        let header = format!(
                            "HTTP/1.1 {status}\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n",
                            body.len()
                        );
                        let mut stream = reader.into_inner();
                        let _ = stream.write_all(header.as_bytes()).await;
                        let _ = stream.write_all(&body).await;
                    });
                }
            }
        });
        Self {
            options,
            server,
            calls,
            downloads,
        }
    }
    async fn session(&self) -> Session {
        Session::login(
            &self.options,
            &Credentials::new("fixture".into(), "fixture".into()),
        )
        .await
        .unwrap()
    }
    fn only_reads(&self) {
        assert!(
            self.calls
                .lock()
                .unwrap()
                .iter()
                .all(|m| m.ends_with("get_record")
                    || m.ends_with("get_by_uuid")
                    || m == "session.login_with_password")
        );
    }
}
#[tokio::test]
async fn authenticated_blob_and_legacy_preview_are_bounded_private_reads() {
    for mode in [Mode::Blob, Mode::Inline] {
        let f = Fixture::new(mode).await;
        let result = read(&f.session().await, observed()).await.unwrap().unwrap();
        assert_eq!(result.bytes.as_ref(), PNG);
        assert_eq!(result.mime, "image/png");
        assert!(!format!("{result:?}").contains("fixture payload"));
        f.only_reads();
    }
    let f = Fixture::new(Mode::Absent).await;
    assert!(
        read(&f.session().await, observed())
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(f.downloads.load(Ordering::SeqCst), 0);
    f.only_reads();
}
#[tokio::test]
async fn preview_rejects_changed_identity_parent_metadata_and_oversized_images() {
    for mode in [
        Mode::WrongParent,
        Mode::ChangedParent,
        Mode::ChangedBlob,
        Mode::ChangedMetadata,
        Mode::Oversized,
        Mode::WrongType,
    ] {
        let f = Fixture::new(mode).await;
        assert!(read(&f.session().await, observed()).await.is_err());
        if matches!(mode, Mode::WrongParent | Mode::Oversized) {
            assert_eq!(f.downloads.load(Ordering::SeqCst), 0);
        }
        f.only_reads();
    }
}
#[tokio::test]
async fn preview_refuses_redirects_and_truncated_payloads_without_retry() {
    for mode in [Mode::Redirect, Mode::Truncated] {
        let f = Fixture::new(mode).await;
        let error = read(&f.session().await, observed())
            .await
            .unwrap_err()
            .to_string();
        assert!(!error.contains("preview-session"));
        assert!(!error.contains("/blob"));
        assert_eq!(f.downloads.load(Ordering::SeqCst), 1);
        f.only_reads();
    }
}
#[tokio::test]
async fn dropping_query_cancels_request_and_failure_has_no_image() {
    let (query, request) = channel(observed(), tokio::runtime::Handle::current());
    let cancel = request.cancel.clone();
    assert!(!cancel.is_cancelled());
    drop(query);
    assert!(cancel.is_cancelled());
    let (query, request) = channel(observed(), tokio::runtime::Handle::current());
    request.fail(Error::Stale);
    assert!(query.latest().unwrap().is_err());
}
#[test]
fn malformed_preview_formats_and_metadata_are_rejected() {
    assert!(image(vec![0; 10]).is_err());
    assert!(image(vec![0; MAX_BYTES + 1]).is_err());
    assert!(
        blob_identity(
            &json!({"uuid":uuid(4),"size":-1,"mime_type":"image/png","last_updated":"now"})
        )
        .is_err()
    );
}
