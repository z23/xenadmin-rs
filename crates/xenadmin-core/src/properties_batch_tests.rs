use super::*;
use crate::{
    ConnectionOptions, Credentials, Endpoint, OperationControl, OperationId, OperationState,
    PoolId, StepState, VmSettingEdit,
    operations::Journal,
    resources::{self, ObservedResource, ResourceRequest},
    xapi::Session,
};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
    sync::mpsc,
};
use tokio_util::sync::CancellationToken;
fn poolid() -> PoolId {
    uuid::Uuid::from_u128(1).into()
}
fn vm(n: u128) -> VmId {
    uuid::Uuid::from_u128(n).into()
}
fn selection() -> VmPropertiesBatch {
    VmPropertiesBatch {
        vms: ["old-a", "old-b"]
            .iter()
            .enumerate()
            .map(|(i, name)| BatchVmProperties {
                vm: vm(i as u128 + 2),
                expected_state: PowerState::Halted,
                patch: VmSettingsPatch::new(vec![
                    VmSettingEdit::Name {
                        previous: (*name).into(),
                        value: format!("new-{i}"),
                    },
                    VmSettingEdit::Description {
                        previous: "old-description".into(),
                        value: "new-description".into(),
                    },
                ])
                .unwrap(),
            })
            .collect(),
    }
}
fn channel() -> (crate::ResourceOperation, ResourceRequest) {
    resources::channel(
        OperationId::new(),
        ObservedResource {
            pool: poolid(),
            generation: 4,
            target: ResourceTarget::Pool(poolid()),
        },
        ResourceAction::PropertiesBatch(selection()),
        tokio::runtime::Handle::current(),
    )
}
#[derive(Clone, Copy, PartialEq, Debug)]
enum Fault {
    None,
    Before,
    After,
    Within,
    LostReply,
    WrongResult,
    WrongIdentity,
    WrongPool,
    CancelBefore,
    CancelAfter,
    Recover,
    Acknowledge,
    RecoverVerify,
    AcknowledgeVerify,
    ActiveRecheck,
    Denied,
}
struct Data {
    calls: Vec<(String, Value)>,
    values: [[String; 2]; 2],
    fault: Fault,
    directory: PathBuf,
    operation: OperationId,
    hooked: bool,
}
struct Fixture {
    options: ConnectionOptions,
    data: Arc<Mutex<Data>>,
    server: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}
impl Fixture {
    async fn new(fault: Fault, directory: &Path, operation: OperationId) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let options = ConnectionOptions::new(
            Endpoint::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap(),
        );
        let data = Arc::new(Mutex::new(Data {
            calls: vec![],
            values: [
                ["old-a".into(), "old-description".into()],
                ["old-b".into(), "old-description".into()],
            ],
            fault,
            directory: directory.into(),
            operation,
            hooked: false,
        }));
        let shared = data.clone();
        let server = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let data = shared.clone();
                tokio::spawn(async move {
                    let mut stream = BufReader::new(stream);
                    let mut line = String::new();
                    if stream.read_line(&mut line).await.unwrap_or(0) == 0 {
                        return;
                    }
                    let mut len = 0usize;
                    loop {
                        line.clear();
                        if stream.read_line(&mut line).await.unwrap_or(0) == 0 {
                            return;
                        }
                        if line == "\r\n" {
                            break;
                        }
                        if let Some((key, v)) = line.split_once(':')
                            && key.eq_ignore_ascii_case("content-length")
                        {
                            len = v.trim().parse().unwrap();
                        }
                    }
                    assert!(len < 16384);
                    let mut body = vec![0; len];
                    stream.read_exact(&mut body).await.unwrap();
                    let request: Value = serde_json::from_slice(&body).unwrap();
                    let reply = respond(&mut data.lock().unwrap(), &request);
                    let Some(result) = reply else {
                        return;
                    };
                    let body = serde_json::to_vec(
                        &json!({"jsonrpc":"2.0","id":request["id"],"result":result}),
                    )
                    .unwrap();
                    let header = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(header.as_bytes()).await;
                    let _ = stream.write_all(&body).await;
                });
            }
        });
        Self {
            options,
            data,
            server,
        }
    }
    async fn session(&self) -> Arc<Session> {
        Arc::new(
            Session::login(
                &self.options,
                &Credentials::new("fixture".into(), "synthetic-private-password".into()),
            )
            .await
            .unwrap(),
        )
    }
    fn writes(&self) -> Vec<String> {
        self.data
            .lock()
            .unwrap()
            .calls
            .iter()
            .filter(|(m, _)| m.starts_with("VM.set_"))
            .map(|(_, p)| p[1].as_str().unwrap().into())
            .collect()
    }
    async fn run(&self, request: ResourceRequest, directory: &Path) {
        let (tx, _rx) = mpsc::channel(512);
        request
            .run(
                self.session().await,
                directory.into(),
                CancellationToken::new(),
                tx,
            )
            .await;
    }
}

fn recover_hook(data: &mut Data) {
    data.hooked = true;
    let journal = Journal::open(&data.directory).unwrap();
    journal.resource_recover(poolid()).unwrap();
    if matches!(data.fault, Fault::Acknowledge | Fault::AcknowledgeVerify) {
        journal
            .resource_control(
                poolid(),
                data.operation,
                OperationControl::AcknowledgeUnknown,
            )
            .unwrap();
    } else if data.fault == Fault::ActiveRecheck {
        let mut row = journal.resource_load(data.operation).unwrap().unwrap();
        row.state = OperationState::Verifying;
        journal.resource_save(&mut row).unwrap();
    }
}
fn respond(data: &mut Data, request: &Value) -> Option<Value> {
    let method = request["method"].as_str().unwrap();
    let p = &request["params"];
    data.calls.push((method.into(), p.clone()));
    match method {
        "session.login_with_password" => Some(json!("OpaqueRef:synthetic-secret-session")),
        "session.get_record" => {
            Some(json!({"is_local_superuser":data.fault != Fault::Denied,"rbac_permissions":[]}))
        }
        "pool.get_by_uuid" => Some(json!("OpaqueRef:pool")),
        "pool.get_record" => {
            if data.fault == Fault::CancelBefore && !data.hooked {
                data.hooked = true;
                Journal::open(&data.directory)
                    .unwrap()
                    .resource_control(
                        poolid(),
                        data.operation,
                        OperationControl::RequestCancellation,
                    )
                    .unwrap();
            }
            Some(
                json!({"uuid": if data.fault==Fault::WrongPool{uuid::Uuid::from_u128(99)}else{poolid().uuid()}}),
            )
        }
        "VM.get_by_uuid" => Some(json!(if p[1] == vm(2).to_string() {
            "OpaqueRef:vm2"
        } else {
            "OpaqueRef:vm3"
        })),
        "VM.get_record" => {
            if data.values[0][0] != "old-a"
                && matches!(data.fault, Fault::RecoverVerify | Fault::AcknowledgeVerify)
                && !data.hooked
            {
                recover_hook(data);
            }
            let index = usize::from(p[1] == "OpaqueRef:vm3");
            let mut name = data.values[index][0].clone();
            let mut description = data.values[index][1].clone();
            if index == 1
                && (data.fault == Fault::Before
                    || data.fault == Fault::After && data.values[0][0] != "old-a")
            {
                name = "changed-by-another-client".into();
            }
            if index == 0 && data.fault == Fault::Within && name != "old-a" {
                description = "changed-by-another-client".into();
            }
            Some(
                json!({"uuid":vm(if data.fault==Fault::WrongIdentity{88}else{index as u128+2}),"is_a_template":false,"is_a_snapshot":false,"is_control_domain":false,"power_state":"Halted","current_operations":{},"name_label":name,"name_description":description}),
            )
        }
        "VM.set_name_label" | "VM.set_name_description" => {
            let index = usize::from(p[1] == "OpaqueRef:vm3");
            let field = usize::from(method == "VM.set_name_description");
            assert_eq!(p.as_array().unwrap().len(), 3);
            if data.fault != Fault::WrongResult {
                data.values[index][field] = p[2].as_str().unwrap().into();
            }
            if matches!(
                data.fault,
                Fault::Recover | Fault::Acknowledge | Fault::ActiveRecheck
            ) && !data.hooked
            {
                recover_hook(data);
            }
            if data.fault == Fault::CancelAfter && !data.hooked {
                data.hooked = true;
                Journal::open(&data.directory)
                    .unwrap()
                    .resource_control(
                        poolid(),
                        data.operation,
                        OperationControl::RequestCancellation,
                    )
                    .unwrap();
            }
            if data.fault == Fault::LostReply {
                None
            } else {
                Some(json!(null))
            }
        }
        "session.logout" => Some(json!(null)),
        _ => panic!("Unexpected fixture RPC {method}"),
    }
}
#[tokio::test]
async fn properties_batch_checks_all_targets_and_verifies_every_setter_with_private_receipts() {
    let d = tempfile::tempdir().unwrap();
    let (op, request) = channel();
    let f = Fixture::new(Fault::None, d.path(), request.record.id).await;
    f.run(request, d.path()).await;
    let row = op.latest();
    assert_eq!(row.state, OperationState::Succeeded, "{}", row.detail);
    assert_eq!(
        f.writes(),
        [
            "OpaqueRef:vm2",
            "OpaqueRef:vm2",
            "OpaqueRef:vm3",
            "OpaqueRef:vm3"
        ]
    );
    let calls = &f.data.lock().unwrap().calls;
    let first = calls
        .iter()
        .position(|(m, _)| m.starts_with("VM.set_"))
        .unwrap();
    assert_eq!(
        calls[..first]
            .iter()
            .filter(|(m, _)| m == "VM.get_record")
            .count(),
        3
    );
    assert!(row.steps.iter().all(|s| s.state == StepState::Verified));
    let public = serde_json::to_string(&*row).unwrap();
    assert!(!public.contains("properties_batch_plan"));
    assert!(!public.contains("OpaqueRef:"));
    let saved = Journal::open(d.path())
        .unwrap()
        .resource_load(row.id)
        .unwrap()
        .unwrap();
    assert!(
        saved
            .properties_batch_plan
            .unwrap()
            .receipts
            .iter()
            .all(|r| r.succeeded && r.verified)
    );
}
#[tokio::test]
async fn properties_batch_whole_selection_and_permission_refusal_precede_writes() {
    for fault in [
        Fault::Before,
        Fault::WrongIdentity,
        Fault::WrongPool,
        Fault::Denied,
    ] {
        let d = tempfile::tempdir().unwrap();
        let (op, request) = channel();
        let f = Fixture::new(fault, d.path(), request.record.id).await;
        f.run(request, d.path()).await;
        assert_eq!(
            op.latest().state,
            OperationState::Rejected,
            "{fault:?}: {}",
            op.latest().detail
        );
        assert!(f.writes().is_empty());
    }
}
#[tokio::test]
async fn properties_batch_rechecks_each_remaining_field_and_preserves_completed_subset() {
    for (fault, writes) in [
        (Fault::After, 2),
        (Fault::Within, 1),
        (Fault::CancelBefore, 0),
        (Fault::CancelAfter, 1),
    ] {
        let d = tempfile::tempdir().unwrap();
        let (op, request) = channel();
        let f = Fixture::new(fault, d.path(), request.record.id).await;
        f.run(request, d.path()).await;
        assert_eq!(
            op.latest().state,
            if writes == 0 {
                OperationState::Cancelled
            } else {
                OperationState::PartiallyCompleted
            },
            "{fault:?}: {}",
            op.latest().detail
        );
        assert_eq!(f.writes().len(), writes);
        assert!(
            !Journal::open(d.path())
                .unwrap()
                .resource_pool_busy(poolid())
                .unwrap()
        );
    }
}
#[tokio::test]
async fn properties_batch_lost_success_reply_is_unknown_even_if_values_match_and_never_replayed() {
    for fault in [Fault::LostReply, Fault::WrongResult] {
        let d = tempfile::tempdir().unwrap();
        let (op, request) = channel();
        let f = Fixture::new(fault, d.path(), request.record.id).await;
        f.run(request, d.path()).await;
        assert_eq!(
            op.latest().state,
            OperationState::OutcomeUnknown,
            "{}",
            op.latest().detail
        );
        let journal = Journal::open(d.path()).unwrap();
        let rows = journal.resource_recover(poolid()).unwrap();
        let before = f.data.lock().unwrap().calls.len();
        f.run(ResourceRequest::tracking(rows[0].clone()), d.path())
            .await;
        let row = journal.resource_load(op.latest().id).unwrap().unwrap();
        assert_eq!(row.state, OperationState::OutcomeUnknown);
        assert!(journal.resource_pool_busy(poolid()).unwrap());
        assert_eq!(f.writes().len(), 1);
        assert!(
            f.data.lock().unwrap().calls[before..]
                .iter()
                .all(|(m, _)| !m.starts_with("VM.set_") && !m.starts_with("task."))
        );
    }
}
#[tokio::test]
async fn properties_batch_late_success_preserves_recovery_fences_and_recheck_only_finishes_saved_work()
 {
    for fault in [
        Fault::Recover,
        Fault::Acknowledge,
        Fault::RecoverVerify,
        Fault::AcknowledgeVerify,
        Fault::ActiveRecheck,
    ] {
        let d = tempfile::tempdir().unwrap();
        let (op, request) = channel();
        let f = Fixture::new(fault, d.path(), request.record.id).await;
        f.run(request, d.path()).await;
        let acknowledged = matches!(fault, Fault::Acknowledge | Fault::AcknowledgeVerify);
        let expected = if acknowledged {
            OperationState::AcknowledgedUnknown
        } else if fault == Fault::ActiveRecheck {
            OperationState::Verifying
        } else {
            OperationState::OutcomeUnknown
        };
        assert_eq!(
            op.latest().state,
            expected,
            "{fault:?}: {}",
            op.latest().detail
        );
        assert_eq!(f.writes().len(), 1);
        let journal = Journal::open(d.path()).unwrap();
        let saved = journal.resource_load(op.latest().id).unwrap().unwrap();
        assert!(saved.properties_batch_plan.as_ref().unwrap().receipts[0].succeeded);
        if !acknowledged {
            f.run(ResourceRequest::tracking(saved), d.path()).await;
            let row = journal.resource_load(op.latest().id).unwrap().unwrap();
            assert_eq!(
                row.state,
                OperationState::PartiallyCompleted,
                "{fault:?}: {}",
                row.detail
            );
            assert_eq!(f.writes().len(), 1);
        }
    }
}
#[tokio::test]
async fn properties_batch_journal_rejects_forged_success_changed_selection_and_regressing_receipts()
{
    let d = tempfile::tempdir().unwrap();
    let (_, mut request) = channel();
    let journal = Journal::open(d.path()).unwrap();
    journal.resource_admit(&mut request.record).unwrap();
    let mut changed = request.record.clone();
    changed.state = OperationState::Succeeded;
    assert!(journal.resource_save(&mut changed).is_err());
    let plan = Plan::new(selection());
    request.record.steps = (0..plan.receipts.len())
        .map(|i| {
            let (v, s) = plan.step(i).unwrap();
            crate::OperationStep {
                label: format!("VM {} · {}", v.vm, s.label),
                state: StepState::Pending,
                detail: String::new(),
            }
        })
        .collect();
    request.record.properties_batch_plan = Some(Box::new(plan));
    journal.resource_save(&mut request.record).unwrap();
    for fake_label in [false, true] {
        let mut changed = request.record.clone();
        if fake_label {
            changed.steps[0].label = "another target".into();
        } else {
            changed.steps[0].state = StepState::Verified;
        }
        assert!(journal.resource_save(&mut changed).is_err());
    }
    request.record.state = OperationState::Dispatching;
    request
        .record
        .properties_batch_plan
        .as_mut()
        .unwrap()
        .receipts[0]
        .dispatched = true;
    journal.resource_save(&mut request.record).unwrap();
    let mut changed = request.record.clone();
    changed.state = OperationState::Preparing;
    changed.properties_batch_plan.as_mut().unwrap().receipts[0].dispatched = false;
    assert!(matches!(
        journal.resource_save(&mut changed),
        Err(Error::JournalChanged)
    ));
    let mut changed = request.record.clone();
    changed
        .properties_batch_plan
        .as_mut()
        .unwrap()
        .selection
        .vms
        .reverse();
    assert!(journal.resource_save(&mut changed).is_err());
    let db = rusqlite::Connection::open(d.path().join("operations.sqlite3")).unwrap();
    db.execute("UPDATE resource_operations SET payload=json_remove(payload,'$.properties_batch_plan') WHERE id=?1",[request.record.id.to_string()]).unwrap();
    assert!(matches!(
        journal.resource_pool_busy(poolid()),
        Err(Error::Journal)
    ));
}
#[test]
fn properties_batch_selection_requires_bounded_unique_known_targets() {
    let target = ResourceTarget::Pool(poolid());
    let mut s = selection();
    assert!(s.validate(target).is_ok());
    s.vms[1] = s.vms[0].clone();
    assert!(s.validate(target).is_err());
    s.vms.clear();
    assert!(s.validate(target).is_err());
    s = selection();
    s.vms[0].expected_state = PowerState::Unknown("Future".into());
    assert!(s.validate(target).is_err());
    s = selection();
    s.vms = (1..=33)
        .map(|n| {
            let mut item = s.vms[0].clone();
            item.vm = vm(n);
            item
        })
        .collect();
    assert!(s.validate(target).is_err());
}
