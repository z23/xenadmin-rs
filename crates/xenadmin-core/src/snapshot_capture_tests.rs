use super::*;
use crate::{
    ConnectionOptions, Credentials, Endpoint, ResourceAction, ResourceTarget, SnapshotId,
    SnapshotPreviewImage, SnapshotWorkflow, snapshot_preview::KEY,
};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
};
fn uuid(n: u128) -> String {
    Uuid::from_u128(n).to_string()
}
fn pool() -> PoolId {
    Uuid::from_u128(1).into()
}
fn snap() -> SnapshotId {
    Uuid::from_u128(3).into()
}
fn png() -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, 2, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&[12, 34, 56, 255, 78, 90, 12, 255])
            .unwrap();
    }
    out
}
fn source(path: &Path) -> OperationId {
    let mut journal = Journal::open(path).unwrap();
    let operation = SnapshotWorkflow::Create {
        name: "Recovery point".into(),
        description: "Before upgrade".into(),
        memory: false,
    };
    let (_, request) = channel(
        OperationId::new(),
        pool(),
        Uuid::from_u128(2).into(),
        1,
        OperationAction::Snapshot(SnapshotOperation::Workflow {
            operation: operation.clone(),
        }),
        tokio::runtime::Handle::current(),
    );
    let mut r = request.record;
    journal.admit(&mut r).unwrap();
    use crate::snapshot_workflow::{Evidence, Plan, Step};
    r.snapshot_workflow = Some(Box::new(Plan {
        operation,
        steps: vec![Step::Create, Step::Description],
        evidence: vec![
            Evidence {
                task: Some("OpaqueRef:task:create".into()),
                result: Some("OpaqueRef:snapshot".into()),
                completed: true,
            },
            Evidence {
                completed: true,
                ..Default::default()
            },
        ],
        baseline: vec![Uuid::from_u128(2).into()],
        created: Some(snap()),
        created_description: Some("Before upgrade".into()),
        memory: None,
    }));
    r.created_snapshot = Some(snap());
    r.state = OperationState::Succeeded;
    r.steps = r
        .snapshot_workflow
        .as_ref()
        .unwrap()
        .steps
        .iter()
        .map(|s| crate::OperationStep {
            label: s.label().into(),
            state: crate::StepState::Verified,
            detail: "Verified".into(),
        })
        .collect();
    journal.save(&mut r).unwrap();
    assert!(r.verified_snapshot_creation(snap()));
    r.id
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fault {
    None,
    Existing,
    ExistingResult,
    Denied,
    LostPut,
    RecoverPut,
    Parent,
    Public,
    LostCreate,
    Redirect,
    Truncated,
    WrongContent,
    EarlyOk,
    RecoverBeforePut,
    CancelBeforePut,
    AckCreate,
    AckPut,
}
struct Fixture {
    options: ConnectionOptions,
    server: tokio::task::JoinHandle<()>,
    creates: Arc<AtomicUsize>,
    puts: Arc<AtomicUsize>,
    image: Arc<Mutex<Vec<u8>>>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}
fn interrupt(path: &Path, ack: bool) {
    let j = Journal::open(path).unwrap();
    let rows = j.resource_recover(pool()).unwrap();
    if ack {
        j.resource_control(pool(), rows[0].id, OperationControl::AcknowledgeUnknown)
            .unwrap();
    }
}
impl Fixture {
    async fn new(fault: Fault, path: PathBuf) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let options = ConnectionOptions::new(
            Endpoint::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap(),
        );
        let creates = Arc::new(AtomicUsize::new(0));
        let puts = Arc::new(AtomicUsize::new(0));
        let image = Arc::new(Mutex::new(Vec::new()));
        let server = tokio::spawn({
            let creates = creates.clone();
            let puts = puts.clone();
            let image = image.clone();
            let made = Arc::new(AtomicBool::new(fault == Fault::Existing));
            let blob_reads = Arc::new(AtomicUsize::new(0));
            async move {
                loop {
                    let (stream, _) = listener.accept().await.unwrap();
                    let creates = creates.clone();
                    let puts = puts.clone();
                    let image = image.clone();
                    let made = made.clone();
                    let path = path.clone();
                    let blob_reads = blob_reads.clone();
                    tokio::spawn(async move {
                        let mut r = BufReader::new(stream);
                        let mut first = String::new();
                        r.read_line(&mut first).await.unwrap();
                        let mut len = 0;
                        let mut cookie = String::new();
                        loop {
                            let mut l = String::new();
                            r.read_line(&mut l).await.unwrap();
                            if l == "\r\n" {
                                break;
                            }
                            if let Some((k, v)) = l.split_once(':') {
                                if k.eq_ignore_ascii_case("content-length") {
                                    len = v.trim().parse::<usize>().unwrap();
                                }
                                if k.eq_ignore_ascii_case("cookie") {
                                    cookie = v.trim().into();
                                }
                            }
                        }
                        assert!(len <= 8 * 1024 * 1024);
                        let is_put = first.starts_with("PUT ");
                        let is_get = first.starts_with("GET ");
                        if is_put || is_get {
                            assert_eq!(cookie, "session_id=OpaqueRef:preview-session");
                            assert!(first.starts_with(&format!(
                                "{} /blob?uuid={} ",
                                if is_put { "PUT" } else { "GET" },
                                uuid(4)
                            )));
                        }
                        if is_put && fault == Fault::EarlyOk {
                            r.get_mut().write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
                            r.get_mut().flush().await.unwrap();
                        }
                        let mut body = vec![0; len];
                        r.read_exact(&mut body).await.unwrap();
                        let mut status = "200 OK";
                        let mut extra = "";
                        let output = if is_put {
                            puts.fetch_add(1, Ordering::SeqCst);
                            if fault == Fault::Redirect {
                                status = "302 Found";
                                extra = "Location: http://127.0.0.1:1/credential-sink\r\n";
                            } else {
                                *image.lock().unwrap() = if fault == Fault::WrongContent {
                                    png()
                                        .into_iter()
                                        .enumerate()
                                        .map(|(i, b)| if i == 25 { b ^ 1 } else { b })
                                        .collect()
                                } else {
                                    body
                                };
                            }
                            if matches!(fault, Fault::AckPut | Fault::RecoverPut) {
                                interrupt(&path, fault == Fault::AckPut);
                            }
                            if fault == Fault::LostPut {
                                return;
                            }
                            if fault == Fault::EarlyOk {
                                return;
                            }
                            Vec::new()
                        } else if is_get {
                            image.lock().unwrap().clone()
                        } else {
                            let req: serde_json::Value = serde_json::from_slice(&body).unwrap();
                            let method = req["method"].as_str().unwrap();
                            let result = match method {
                                "session.login_with_password" => json!("OpaqueRef:preview-session"),
                                "session.get_record" => {
                                    json!({"is_local_superuser":fault!=Fault::Denied,"rbac_permissions":[]})
                                }
                                "blob.get_all_records" => {
                                    if fault == Fault::ExistingResult {
                                        json!({"OpaqueRef:existing":{"uuid":uuid(4)}})
                                    } else {
                                        json!({})
                                    }
                                }
                                "pool.get_by_uuid" => json!("OpaqueRef:pool"),
                                "pool.get_record" => json!({"uuid":uuid(1)}),
                                "VM.get_by_uuid" => json!(if req["params"][1] == uuid(2) {
                                    "OpaqueRef:parent"
                                } else {
                                    "OpaqueRef:snapshot"
                                }),
                                "VM.get_record" => {
                                    if req["params"][1] == "OpaqueRef:parent" {
                                        json!({"uuid":uuid(2),"is_a_snapshot":false,"is_a_template":false,"is_control_domain":false})
                                    } else {
                                        let mut record = json!({"uuid":uuid(3),"is_a_snapshot":true,"is_control_domain":false,"snapshot_of":if fault==Fault::Parent{"OpaqueRef:foreign"}else{"OpaqueRef:parent"},"current_operations":{},"blobs":{},"other_config":{}});
                                        if made.load(Ordering::SeqCst) {
                                            record["blobs"][KEY] = json!("OpaqueRef:blob");
                                        }
                                        record
                                    }
                                }
                                "VM.create_new_blob" => {
                                    assert_eq!(
                                        req["params"],
                                        json!([
                                            "OpaqueRef:preview-session",
                                            "OpaqueRef:snapshot",
                                            KEY,
                                            "image/png",
                                            false
                                        ])
                                    );
                                    creates.fetch_add(1, Ordering::SeqCst);
                                    made.store(true, Ordering::SeqCst);
                                    if fault == Fault::LostCreate {
                                        return;
                                    }
                                    if fault == Fault::AckCreate {
                                        interrupt(&path, true);
                                    }
                                    json!("OpaqueRef:blob")
                                }
                                "blob.get_record" => {
                                    let n = blob_reads.fetch_add(1, Ordering::SeqCst);
                                    if n == 1
                                        && matches!(
                                            fault,
                                            Fault::RecoverBeforePut | Fault::CancelBeforePut
                                        )
                                    {
                                        if fault == Fault::CancelBeforePut {
                                            let j = Journal::open(&path).unwrap();
                                            let r = j.resource_history(pool()).unwrap().remove(0);
                                            j.resource_control(
                                                pool(),
                                                r.id,
                                                OperationControl::RequestCancellation,
                                            )
                                            .unwrap();
                                        } else {
                                            interrupt(&path, false);
                                        }
                                    }
                                    json!({"uuid":uuid(4),"mime_type":"image/png","public":fault==Fault::Public,"size":image.lock().unwrap().len(),"last_updated":"20260101T00:00:00Z"})
                                }
                                _ => panic!("Unexpected preview RPC {method}"),
                            };
                            serde_json::to_vec(
                                &json!({"jsonrpc":"2.0","id":req["id"],"result":result}),
                            )
                            .unwrap()
                        };
                        let declared = output.len()
                            + if is_get && fault == Fault::Truncated {
                                5
                            } else {
                                0
                            };
                        let header = format!(
                            "HTTP/1.1 {status}\r\nContent-Length: {declared}\r\n{extra}Connection: close\r\n\r\n"
                        );
                        r.get_mut().write_all(header.as_bytes()).await.unwrap();
                        r.get_mut().write_all(&output).await.unwrap();
                    });
                }
            }
        });
        Self {
            options,
            server,
            creates,
            puts,
            image,
        }
    }
    async fn session(&self) -> Arc<Session> {
        Arc::new(
            Session::login(
                &self.options,
                &Credentials::new("fixture".into(), "fixture".into()),
            )
            .await
            .unwrap(),
        )
    }
}
async fn execute(f: &Fixture, path: &Path, source: OperationId) -> crate::ResourceOperationRecord {
    let image = SnapshotPreviewImage::from_png(png()).unwrap();
    let (handle, mut request) = crate::resources::channel(
        OperationId::new(),
        crate::ObservedResource {
            pool: pool(),
            generation: 1,
            target: ResourceTarget::Vm(Uuid::from_u128(2).into()),
        },
        ResourceAction::SnapshotPreviewUpload(image.descriptor(source, snap())),
        tokio::runtime::Handle::current(),
    );
    request.snapshot_preview_image = Some(image);
    let (tx, _rx) = mpsc::channel(128);
    tokio::time::timeout(
        Duration::from_secs(15),
        request.run(f.session().await, path.into(), CancellationToken::new(), tx),
    )
    .await
    .unwrap();
    handle.latest().as_ref().clone()
}
async fn recheck(
    f: &Fixture,
    path: &Path,
    r: crate::ResourceOperationRecord,
) -> crate::ResourceOperationRecord {
    let id = r.id;
    let request = crate::resources::ResourceRequest::tracking(r);
    let (tx, _rx) = mpsc::channel(128);
    request
        .run(f.session().await, path.into(), CancellationToken::new(), tx)
        .await;
    Journal::open(path)
        .unwrap()
        .resource_load(id)
        .unwrap()
        .unwrap()
}
#[tokio::test]
async fn snapshot_preview_upload_is_private_content_verified_and_receipted() {
    for fault in [Fault::None, Fault::EarlyOk] {
        let dir = tempfile::tempdir().unwrap();
        let id = source(dir.path());
        let f = Fixture::new(fault, dir.path().into()).await;
        let result = execute(&f, dir.path(), id).await;
        assert_eq!(
            result.state,
            OperationState::Succeeded,
            "{fault:?}: {}",
            result.detail
        );
        assert_eq!(*f.image.lock().unwrap(), png());
        assert_eq!(f.creates.load(Ordering::SeqCst), 1);
        assert_eq!(f.puts.load(Ordering::SeqCst), 1);
        let public = serde_json::to_string(&result).unwrap();
        assert!(!public.contains("OpaqueRef"));
        assert!(!public.contains("sha256"));
        assert!(!public.contains("snapshot_preview_plan"));
        assert!(
            Journal::open(dir.path())
                .unwrap()
                .resource_load(result.id)
                .unwrap()
                .unwrap()
                .snapshot_preview_plan
                .unwrap()
                .content_verified
        );
    }
}
#[tokio::test]
async fn snapshot_preview_refuses_wrong_source_parent_existing_and_public_blob() {
    for fault in [
        Fault::Existing,
        Fault::Parent,
        Fault::Public,
        Fault::ExistingResult,
        Fault::Denied,
    ] {
        let dir = tempfile::tempdir().unwrap();
        let id = source(dir.path());
        let f = Fixture::new(fault, dir.path().into()).await;
        let result = execute(&f, dir.path(), id).await;
        assert_ne!(result.state, OperationState::Succeeded);
        assert_eq!(f.puts.load(Ordering::SeqCst), 0);
        assert_eq!(
            f.creates.load(Ordering::SeqCst),
            usize::from(matches!(fault, Fault::Public | Fault::ExistingResult))
        );
    }
    let dir = tempfile::tempdir().unwrap();
    let id = source(dir.path());
    let j = Journal::open(dir.path()).unwrap();
    let mut r = j.load(id).unwrap().unwrap();
    r.snapshot_workflow.as_mut().unwrap().evidence[0].completed = false;
    assert!(!r.verified_snapshot_creation(snap()));
    r.snapshot_workflow = None;
    assert!(!r.verified_snapshot_creation(snap()));
    let f = Fixture::new(Fault::None, dir.path().into()).await;
    let result = execute(&f, dir.path(), OperationId::new()).await;
    assert_eq!(result.state, OperationState::Rejected);
    assert_eq!(f.creates.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn snapshot_preview_lost_create_never_adopts_or_repeats_existing_blob() {
    let dir = tempfile::tempdir().unwrap();
    let id = source(dir.path());
    let f = Fixture::new(Fault::LostCreate, dir.path().into()).await;
    let result = execute(&f, dir.path(), id).await;
    assert_eq!(
        result.state,
        OperationState::OutcomeUnknown,
        "{}",
        result.detail
    );
    let result = recheck(&f, dir.path(), result).await;
    assert_eq!(result.state, OperationState::OutcomeUnknown);
    assert_eq!(f.creates.load(Ordering::SeqCst), 1);
    assert_eq!(f.puts.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn snapshot_preview_recovery_fences_upload_and_preserves_late_receipts() {
    for fault in [
        Fault::RecoverBeforePut,
        Fault::CancelBeforePut,
        Fault::AckCreate,
        Fault::AckPut,
    ] {
        let dir = tempfile::tempdir().unwrap();
        let id = source(dir.path());
        let f = Fixture::new(fault, dir.path().into()).await;
        let r = execute(&f, dir.path(), id).await;
        assert_eq!(
            r.state,
            if matches!(fault, Fault::RecoverBeforePut | Fault::CancelBeforePut) {
                OperationState::PartiallyCompleted
            } else {
                OperationState::AcknowledgedUnknown
            },
            "{fault:?}: {}",
            r.detail
        );
        let plan = r.snapshot_preview_plan.as_ref().unwrap();
        assert!(plan.create_reference.is_some());
        assert_eq!(
            f.puts.load(Ordering::SeqCst),
            usize::from(fault == Fault::AckPut)
        );
        if fault == Fault::AckPut {
            assert!(plan.content_verified);
        }
        if fault == Fault::RecoverBeforePut {
            let r = recheck(&f, dir.path(), r).await;
            assert_eq!(r.state, OperationState::PartiallyCompleted);
            assert_eq!(f.puts.load(Ordering::SeqCst), 0);
        }
    }
}
#[tokio::test]
async fn snapshot_preview_rejects_redirect_truncation_and_wrong_content_without_reupload() {
    for fault in [Fault::Redirect, Fault::Truncated, Fault::WrongContent] {
        let dir = tempfile::tempdir().unwrap();
        let id = source(dir.path());
        let f = Fixture::new(fault, dir.path().into()).await;
        let r = execute(&f, dir.path(), id).await;
        assert_ne!(r.state, OperationState::Succeeded, "{fault:?}");
        assert_eq!(f.puts.load(Ordering::SeqCst), 1);
        let r = recheck(&f, dir.path(), r).await;
        assert_ne!(r.state, OperationState::Succeeded);
        assert_eq!(f.puts.load(Ordering::SeqCst), 1);
    }
}
#[test]
fn snapshot_preview_png_bounds_and_private_debug() {
    let valid = SnapshotPreviewImage::from_png(png()).unwrap();
    assert_eq!(valid.dimensions(), (2, 1));
    assert!(!format!("{valid:?}").contains("12, 34"));
    assert!(SnapshotPreviewImage::from_png(vec![0; 8 * 1024 * 1024 + 1]).is_err());
    let mut truncated = png();
    truncated.truncate(truncated.len() / 2);
    assert!(SnapshotPreviewImage::from_png(truncated).is_err());
    assert!(SnapshotPreviewImage::from_png(b"\x89PNG\r\n\x1a\nbad".to_vec()).is_err());
}

#[tokio::test]
async fn snapshot_preview_lost_upload_is_resolved_by_content_without_replay() {
    for fault in [Fault::LostPut, Fault::RecoverPut] {
        let dir = tempfile::tempdir().unwrap();
        let id = source(dir.path());
        let f = Fixture::new(fault, dir.path().into()).await;
        let result = execute(&f, dir.path(), id).await;
        assert_eq!(
            result.state,
            OperationState::OutcomeUnknown,
            "{fault:?}: {}",
            result.detail
        );
        let result = recheck(&f, dir.path(), result).await;
        assert_eq!(
            result.state,
            OperationState::Succeeded,
            "{fault:?}: {}",
            result.detail
        );
        assert_eq!(f.creates.load(Ordering::SeqCst), 1);
        assert_eq!(f.puts.load(Ordering::SeqCst), 1);
    }
}
#[tokio::test]
async fn snapshot_preview_read_only_and_private_plan_changes_cannot_dispatch() {
    let dir = tempfile::tempdir().unwrap();
    let id = source(dir.path());
    let mut f = Fixture::new(Fault::None, dir.path().into()).await;
    f.options.read_only = true;
    let result = execute(&f, dir.path(), id).await;
    assert_eq!(result.state, OperationState::Rejected);
    assert_eq!(f.creates.load(Ordering::SeqCst), 0);
    f.options.read_only = false;
    let result = execute(&f, dir.path(), id).await;
    assert_eq!(result.state, OperationState::Succeeded);
    let journal = Journal::open(dir.path()).unwrap();
    for field in ["source", "hash", "baseline", "blob", "dispatch"] {
        let mut r = result.clone();
        let plan = r.snapshot_preview_plan.as_mut().unwrap();
        match field {
            "source" => plan.expected.source = OperationId::new(),
            "hash" => plan.sha256[0] ^= 1,
            "baseline" => plan.baseline.push(Uuid::from_u128(4)),
            "blob" => plan.blob = Some(Uuid::from_u128(55)),
            _ => plan.upload_sent = false,
        }
        assert!(journal.resource_save(&mut r).is_err(), "{field}");
    }
}
