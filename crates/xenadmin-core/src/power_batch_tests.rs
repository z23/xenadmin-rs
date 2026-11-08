use super::*;
use crate::{
    ConnectionOptions, Credentials, Endpoint, OperationControl, OperationId, OperationState,
    PoolId, StepState,
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
fn selection() -> VmPowerBatch {
    VmPowerBatch {
        action: PowerAction::Start,
        vms: vec![
            BatchVmPower {
                vm: vm(2),
                expected_state: PowerState::Halted,
            },
            BatchVmPower {
                vm: vm(3),
                expected_state: PowerState::Halted,
            },
        ],
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
        ResourceAction::PowerBatch(selection()),
        tokio::runtime::Handle::current(),
    )
}
#[derive(Clone, Copy, PartialEq)]
enum Fault {
    None,
    BlockBefore,
    BlockAfter,
    ChangedAfter,
    LostReply,
    Failure,
    WrongResult,
    WrongIdentity,
    WrongPool,
    CancelBefore,
    CancelAfter,
    Recover,
    Acknowledge,
    RecoverTask,
    AcknowledgeTask,
    RecoverVerify,
    AcknowledgeVerify,
}
struct Data {
    calls: Vec<(String, Value)>,
    running: [bool; 2],
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
            running: [false, false],
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
            .filter(|(m, _)| m.starts_with("Async.VM."))
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
fn respond(data: &mut Data, request: &Value) -> Option<Value> {
    let method = request["method"].as_str().unwrap();
    let p = &request["params"];
    data.calls.push((method.into(), p.clone()));
    match method {
        "session.get_record" => Some(json!({"is_local_superuser":true})),
        "session.login_with_password" => Some(json!("OpaqueRef:synthetic-secret-session")),
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
                json!({"uuid":if data.fault==Fault::WrongPool{uuid::Uuid::from_u128(99)}else{poolid().uuid()}}),
            )
        }
        "VM.get_by_uuid" => Some(json!(if p[1] == vm(2).to_string() {
            "OpaqueRef:vm2"
        } else {
            "OpaqueRef:vm3"
        })),
        "VM.get_record" => {
            if data.running[0]
                && matches!(data.fault, Fault::RecoverVerify | Fault::AcknowledgeVerify)
                && !data.hooked
            {
                recover_hook(data);
            }
            let index = usize::from(p[1] == "OpaqueRef:vm3");
            let blocked = index == 1
                && (data.fault == Fault::BlockBefore
                    || (data.running[0] && data.fault == Fault::BlockAfter));
            let changed = index == 1 && data.running[0] && data.fault == Fault::ChangedAfter;
            Some(
                json!({"uuid":vm(if data.fault==Fault::WrongIdentity{88}else{index as u128+2}).to_string(),"is_a_template":false,"is_a_snapshot":false,"is_control_domain":false,"power_state":if data.running[index]||changed{"Running"}else{"Halted"},"allowed_operations":["start"],"blocked_operations":if blocked{json!({"start":"Fixture policy"})}else{json!({})},"current_operations":{}}),
            )
        }
        "VM.assert_operation_valid" => Some(json!(null)),
        "Async.VM.start" => {
            assert_eq!(p.as_array().unwrap().len(), 4);
            assert_eq!(p[2], false);
            assert_eq!(p[3], false);
            let index = usize::from(p[1] == "OpaqueRef:vm3");
            data.running[index] = data.fault != Fault::WrongResult;
            if matches!(data.fault, Fault::Recover | Fault::Acknowledge) && !data.hooked {
                data.hooked = true;
                let journal = Journal::open(&data.directory).unwrap();
                journal.resource_recover(poolid()).unwrap();
                if data.fault == Fault::Acknowledge {
                    journal
                        .resource_control(
                            poolid(),
                            data.operation,
                            OperationControl::AcknowledgeUnknown,
                        )
                        .unwrap();
                }
            }
            if data.fault == Fault::LostReply {
                None
            } else {
                Some(json!(format!("OpaqueRef:task{index}")))
            }
        }
        "task.get_record" => {
            if matches!(data.fault, Fault::RecoverTask | Fault::AcknowledgeTask) && !data.hooked {
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
            Some(
                json!({"status":if data.fault==Fault::Failure{"failure"}else{"success"},"error_info":["FIXTURE_FAILURE"],"progress":1.0}),
            )
        }
        "session.logout" | "task.destroy" => Some(json!(null)),
        _ => panic!("Unexpected fixture RPC {method}"),
    }
}
fn recover_hook(data: &mut Data) {
    data.hooked = true;
    let journal = Journal::open(&data.directory).unwrap();
    journal.resource_recover(poolid()).unwrap();
    if matches!(
        data.fault,
        Fault::AcknowledgeTask | Fault::AcknowledgeVerify
    ) {
        journal
            .resource_control(
                poolid(),
                data.operation,
                OperationControl::AcknowledgeUnknown,
            )
            .unwrap();
    }
}
#[tokio::test]
async fn ordered_power_batch_tracks_each_task_and_preserves_private_receipts() {
    let d = tempfile::tempdir().unwrap();
    let (op, request) = channel();
    let f = Fixture::new(Fault::None, d.path(), request.record.id).await;
    f.run(request, d.path()).await;
    let result = op.latest();
    assert_eq!(result.state, OperationState::Succeeded, "{}", result.detail);
    assert!(result.steps.iter().all(|s| s.state == StepState::Verified));
    assert_eq!(f.writes(), ["OpaqueRef:vm2", "OpaqueRef:vm3"]);
    let calls = &f.data.lock().unwrap().calls;
    assert_eq!(calls.iter().filter(|(m, _)| m == "task.destroy").count(), 2);
    let first = calls
        .iter()
        .position(|(m, _)| m == "Async.VM.start")
        .unwrap();
    assert_eq!(
        calls[..first]
            .iter()
            .filter(|(m, _)| m == "VM.assert_operation_valid")
            .count(),
        3
    );
    let public = serde_json::to_string(&*result).unwrap();
    assert!(!public.contains("OpaqueRef:"));
    assert!(!public.contains("synthetic-private"));
    assert!(!public.contains("power_batch_plan"));
    assert!(public.contains(&vm(2).to_string()));
    let stored = Journal::open(d.path())
        .unwrap()
        .resource_load(result.id)
        .unwrap()
        .unwrap();
    assert!(
        stored
            .power_batch_plan
            .unwrap()
            .items
            .iter()
            .all(|i| i.succeeded && i.verified && i.task.is_some())
    );
}
#[tokio::test]
async fn all_selection_preflight_refuses_blocked_or_replaced_objects_before_any_write() {
    for fault in [Fault::BlockBefore, Fault::WrongIdentity, Fault::WrongPool] {
        let d = tempfile::tempdir().unwrap();
        let (op, request) = channel();
        let f = Fixture::new(fault, d.path(), request.record.id).await;
        f.run(request, d.path()).await;
        assert_eq!(op.latest().state, OperationState::Rejected);
        assert!(f.writes().is_empty());
    }
}
#[tokio::test]
async fn changed_second_guest_stops_with_verified_subset_and_no_second_dispatch() {
    for fault in [Fault::BlockAfter, Fault::ChangedAfter] {
        let d = tempfile::tempdir().unwrap();
        let (op, request) = channel();
        let f = Fixture::new(fault, d.path(), request.record.id).await;
        f.run(request, d.path()).await;
        let r = op.latest();
        assert_eq!(r.state, OperationState::PartiallyCompleted, "{}", r.detail);
        assert_eq!(r.steps[0].state, StepState::Verified);
        assert_eq!(r.steps[1].state, StepState::Pending);
        assert_eq!(f.writes().len(), 1);
        assert!(
            !Journal::open(d.path())
                .unwrap()
                .resource_pool_busy(poolid())
                .unwrap()
        );
    }
}
#[tokio::test]
async fn uncertain_dispatch_or_task_failure_never_replays_even_when_state_matches() {
    for fault in [Fault::LostReply, Fault::Failure, Fault::WrongResult] {
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
        f.run(ResourceRequest::tracking(rows[0].clone()), d.path())
            .await;
        let r = journal.resource_load(op.latest().id).unwrap().unwrap();
        assert_eq!(r.state, OperationState::OutcomeUnknown);
        assert!(journal.resource_pool_busy(poolid()).unwrap());
        assert_eq!(f.writes().len(), 1);
    }
}
#[tokio::test]
async fn stop_finishes_current_task_without_task_cancel_and_skips_remaining_vms() {
    for (fault, state, writes) in [
        (Fault::CancelBefore, OperationState::Cancelled, 0),
        (Fault::CancelAfter, OperationState::PartiallyCompleted, 1),
    ] {
        let d = tempfile::tempdir().unwrap();
        let (op, request) = channel();
        let f = Fixture::new(fault, d.path(), request.record.id).await;
        f.run(request, d.path()).await;
        assert_eq!(op.latest().state, state, "{}", op.latest().detail);
        assert_eq!(f.writes().len(), writes);
        assert!(
            !f.data
                .lock()
                .unwrap()
                .calls
                .iter()
                .any(|(m, _)| m == "task.cancel")
        );
    }
}
#[tokio::test]
async fn late_receipt_preserves_recovered_unknown_or_acknowledgement_without_continuation() {
    for fault in [
        Fault::Recover,
        Fault::Acknowledge,
        Fault::RecoverTask,
        Fault::AcknowledgeTask,
        Fault::RecoverVerify,
        Fault::AcknowledgeVerify,
    ] {
        let d = tempfile::tempdir().unwrap();
        let (op, request) = channel();
        let f = Fixture::new(fault, d.path(), request.record.id).await;
        f.run(request, d.path()).await;
        let r = op.latest();
        assert_eq!(
            r.state,
            if matches!(
                fault,
                Fault::Acknowledge | Fault::AcknowledgeTask | Fault::AcknowledgeVerify
            ) {
                OperationState::AcknowledgedUnknown
            } else {
                OperationState::OutcomeUnknown
            }
        );
        assert_eq!(f.writes().len(), 1);
        let journal = Journal::open(d.path()).unwrap();
        let saved = journal.resource_load(r.id).unwrap().unwrap();
        assert!(
            saved.power_batch_plan.as_ref().unwrap().items[0]
                .task
                .is_some()
        );
        assert!(
            saved.power_batch_plan.as_ref().unwrap().items[1]
                .task
                .is_none()
        );
        if matches!(
            fault,
            Fault::Recover | Fault::RecoverTask | Fault::RecoverVerify
        ) {
            f.run(ResourceRequest::tracking(saved), d.path()).await;
            let saved = journal.resource_load(r.id).unwrap().unwrap();
            assert_eq!(
                saved.state,
                OperationState::PartiallyCompleted,
                "{}",
                saved.detail
            );
            assert_eq!(saved.steps[0].state, StepState::Verified);
            assert_eq!(f.writes().len(), 1);
        }
    }
}
#[tokio::test]
async fn private_plan_rejects_rewriting_receipts_or_selection_and_corrupt_reservations_fail_closed()
{
    let d = tempfile::tempdir().unwrap();
    let (_, mut request) = channel();
    let journal = Journal::open(d.path()).unwrap();
    journal.resource_admit(&mut request.record).unwrap();
    request.record.steps = selection()
        .vms
        .iter()
        .map(|v| crate::OperationStep {
            label: v.vm.to_string(),
            state: StepState::Pending,
            detail: String::new(),
        })
        .collect();
    request.record.power_batch_plan = Some(Box::new(Plan::new(selection())));
    journal.resource_save(&mut request.record).unwrap();
    request.record.state = OperationState::Dispatching;
    request.record.power_batch_plan.as_mut().unwrap().items[0].dispatched = true;
    request.record.power_batch_plan.as_mut().unwrap().items[0].task =
        Some("OpaqueRef:first".into());
    journal.resource_save(&mut request.record).unwrap();
    let mut changed = request.record.clone();
    changed.power_batch_plan.as_mut().unwrap().items[0].task = Some("OpaqueRef:second".into());
    assert!(matches!(
        journal.resource_save(&mut changed),
        Err(Error::JournalChanged)
    ));
    let mut changed = request.record.clone();
    changed
        .power_batch_plan
        .as_mut()
        .unwrap()
        .selection
        .vms
        .reverse();
    assert!(matches!(
        journal.resource_save(&mut changed),
        Err(Error::Journal)
    ));
    let db = rusqlite::Connection::open(d.path().join("operations.sqlite3")).unwrap();
    // Open through SQLite, never raw file reads (which disturb advisory locks).
    let tables: Vec<String> = db
        .prepare("SELECT name FROM sqlite_master WHERE type='table'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(tables.contains(&"resource_operations".into()));
    db.execute("UPDATE resource_operations SET payload=json_remove(payload,'$.power_batch_plan') WHERE id=?1",[request.record.id.to_string()]).unwrap();
    assert!(matches!(
        journal.resource_pool_busy(poolid()),
        Err(Error::Journal)
    ));
}
#[test]
fn batch_selection_is_bounded_unique_and_requires_known_compatible_states() {
    let target = ResourceTarget::Pool(poolid());
    let mut batch = selection();
    assert!(batch.validate(target).is_ok());
    batch.vms[1] = batch.vms[0].clone();
    assert!(batch.validate(target).is_err());
    batch.vms.clear();
    assert!(batch.validate(target).is_err());
    batch.vms = (1..=33)
        .map(|n| BatchVmPower {
            vm: vm(n),
            expected_state: PowerState::Halted,
        })
        .collect();
    assert!(batch.validate(target).is_err());
    batch = selection();
    batch.vms[0].expected_state = PowerState::Unknown("Future".into());
    assert!(batch.validate(target).is_err());
    batch = selection();
    assert!(
        batch
            .validate(ResourceTarget::Host(uuid::Uuid::from_u128(1).into()))
            .is_err()
    );
}
