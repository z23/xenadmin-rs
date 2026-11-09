use super::*;
use xenadmin_core::{
    HostId, OperationControl, OperationId, OperationState, PoolRootPasswordChange,
    ResourceOperation, ResourceOperationRecord, ResourceTarget, RootPassword,
};
const POOL: &str = "11111111-1111-1111-1111-111111111111";
const HOST: &str = "22222222-2222-2222-2222-222222222222";
const MEMBER: &str = "44444444-4444-4444-4444-444444444444";
const PASSWORD: &str = "synthetic-new-root-password-8291";
type Gate = Option<(Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>)>;
#[derive(Default)]
struct Control {
    changed: bool,
    deny: bool,
    member: bool,
    expire: bool,
    wrong_membership: bool,
    logins: Vec<String>,
}
async fn fixture(lost: bool, gate: Gate) -> (Mock, Arc<Mutex<Control>>) {
    let control = Arc::new(Mutex::new(Control::default()));
    let state = control.clone();
    let mock = mock_overridden(override_rpc(move |r| {
        let state = state.clone(); let gate = gate.clone();
        async move {
            let method = r["method"].as_str().unwrap(); let p = &r["params"];
            if method == "event.from" && p[2] != "" { tokio::time::sleep(Duration::from_millis(40)).await; }
            let response = {
                let mut s = state.lock().unwrap();
                match method {
                    "session.login_with_password" => {
                        s.logins.push(p[1].as_str().unwrap().into());
                        if p[0] != "root" || p[1] != if s.changed { PASSWORD } else { "fixture" } {
                            return Some(RpcReply::Json(json!({"error":{"message":"SESSION_AUTHENTICATION_FAILED","data":[]}})));
                        }
                        Some(json!("OpaqueRef:session"))
                    },
                    "event.from" if p[2] == "" => {
                        let mut b = batch("initial", "Halted");
                        if s.member { b["events"].as_array_mut().unwrap().push(json!({"class":"host","operation":"add","ref":"OpaqueRef:member","snapshot":{"uuid":MEMBER,"name_label":"Fixture member","address":"192.0.2.11","enabled":true}})); }
                        Some(b)
                    },
                    "event.from" if s.expire => { s.expire = false; return Some(RpcReply::Json(json!({"error":{"message":"SESSION_INVALID","data":[]}}))); },
                    "event.from" => Some(json!({"token":"initial","events":[]})),
                    "session.get_record" => Some(json!({"is_local_superuser":!s.deny})),
                    "pool.get_by_uuid" => Some(json!("OpaqueRef:pool")),
                    "pool.get_record" => Some(json!({"uuid":POOL,"master":"OpaqueRef:host"})),
                    "host.get_all_records" => {
                        let mut hosts = json!({"OpaqueRef:host":{"uuid":HOST}});
                        if s.member || s.wrong_membership { hosts["OpaqueRef:member"] = json!({"uuid":MEMBER}); }
                        Some(hosts)
                    },
                    "session.change_password" => {
                        assert_eq!(p[1], "", "The API ignores the old password; do not retain or send it");
                        assert_eq!(p[2], PASSWORD); s.changed = true; Some(Value::Null)
                    },
                    _ => None,
                }
            };
            if method == "session.change_password" {
                if let Some((entered, release)) = gate { entered.notify_one(); release.notified().await; }
                if lost { return Some(RpcReply::Drop); }
            }
            response.map(|value| RpcReply::Json(json!({"result":value})))
        }
    })).await;
    (mock, control)
}
async fn connect(mock: &Mock, backend: &Backend, password: &str) -> xenadmin_core::Connection {
    let c = backend.connect(
        ConnectionOptions::new(mock.endpoint.clone()),
        Credentials::new("root".into(), password.into()),
    );
    c.wait_ready(deadline(5)).await.unwrap();
    c
}
fn submit(c: &xenadmin_core::Connection) -> ResourceOperation {
    let snapshot = c.latest();
    let inventory = snapshot.inventory().unwrap();
    c.change_pool_root_password(
        OperationId::new(),
        snapshot
            .observe_resource(ResourceTarget::Pool(inventory.pool_id))
            .unwrap(),
        PoolRootPasswordChange {
            coordinator: inventory.coordinator.unwrap(),
            members: inventory.hosts.keys().copied().collect(),
        },
        RootPassword::new(PASSWORD.into()).unwrap(),
    )
    .unwrap()
}
async fn done(op: &ResourceOperation) -> Arc<ResourceOperationRecord> {
    tokio::time::timeout(deadline(8), async {
        loop {
            let r = op.latest();
            if r.state.terminal() || r.state == OperationState::OutcomeUnknown {
                return r;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}
async fn history(
    c: &xenadmin_core::Connection,
    id: OperationId,
    predicate: impl Fn(&ResourceOperationRecord) -> bool,
) -> ResourceOperationRecord {
    tokio::time::timeout(deadline(8), async {
        loop {
            if let Some(r) = c
                .resource_history()
                .iter()
                .find(|r| r.id == id && predicate(r))
            {
                return r.clone();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}
fn count(m: &Mock, method: &str) -> usize {
    m.methods
        .lock()
        .unwrap()
        .iter()
        .filter(|v| v.as_str() == method)
        .count()
}
fn privacy(directory: &std::path::Path, r: &ResourceOperationRecord) {
    let db = rusqlite::Connection::open_with_flags(
        directory.join("operations.sqlite3"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let payload: String = db
        .query_row(
            "SELECT payload FROM resource_operations WHERE id=?1",
            [r.id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    let public = serde_json::to_string(r).unwrap();
    assert!(!payload.contains(PASSWORD));
    assert!(!payload.contains("fixture"));
    assert!(!public.contains(PASSWORD));
    assert!(!public.contains("OpaqueRef:"));
    assert!(!public.contains("root_password_plan"));
}
#[tokio::test]
async fn password_change_verifies_login_rotates_reconnect_and_invalidates_old_aliases() {
    let (m, state) = fixture(false, None).await;
    let d = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(d.path().into()).unwrap();
    let c = connect(&m, &b, "fixture").await;
    let alias = connect(&m, &b, "fixture").await;
    let r = done(&submit(&c)).await;
    assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
    assert!(
        r.root_password
            .as_ref()
            .unwrap()
            .reconnect_credentials_updated
    );
    privacy(d.path(), &r);
    tokio::time::timeout(deadline(3), async {
        while alias.latest().state() != ConnectionState::Disconnected {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(c.latest().is_current());
    let before = state.lock().unwrap().logins.len();
    state.lock().unwrap().expire = true;
    tokio::time::timeout(deadline(8), async {
        loop {
            if state.lock().unwrap().logins.len() > before && c.latest().is_current() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(state.lock().unwrap().logins.last().unwrap(), PASSWORD);
    let new_alias = connect(&m, &b, PASSWORD).await;
    assert_eq!(new_alias.latest().generation(), c.latest().generation());
    assert_eq!(count(&m, "session.change_password"), 1);
}
#[tokio::test]
async fn member_password_propagation_stays_explicitly_unverified() {
    let (m, state) = fixture(false, None).await;
    state.lock().unwrap().member = true;
    let d = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(d.path().into()).unwrap();
    let c = connect(&m, &b, "fixture").await;
    let r = done(&submit(&c)).await;
    assert_eq!(r.state, OperationState::PartiallyCompleted, "{}", r.detail);
    assert_eq!(
        r.root_password.as_ref().unwrap().unverified_members,
        vec![HostId::parse(MEMBER).unwrap()]
    );
}
#[tokio::test]
async fn lost_password_reply_never_replays_or_updates_reconnect_credentials() {
    let (m, _) = fixture(true, None).await;
    let d = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(d.path().into()).unwrap();
    let c = connect(&m, &b, "fixture").await;
    let op = submit(&c);
    let r = done(&op).await;
    assert_eq!(r.state, OperationState::OutcomeUnknown);
    assert!(!r.root_password.as_ref().unwrap().coordinator_changed);
    privacy(d.path(), &r);
    let old = history(&c, r.id, |r| r.state == OperationState::OutcomeUnknown)
        .await
        .revision();
    c.control_resource(r.id, OperationControl::Recheck).unwrap();
    let r = history(&c, r.id, |r| {
        r.revision() > old && r.state == OperationState::OutcomeUnknown
    })
    .await;
    assert!(!r.root_password.unwrap().reconnect_credentials_updated);
    assert_eq!(count(&m, "session.change_password"), 1);
    assert_eq!(count(&m, "session.login_with_password"), 1);
}
#[tokio::test]
async fn changed_membership_and_nonlocal_account_refuse_before_password_dispatch() {
    for denied in [false, true] {
        let (m, state) = fixture(false, None).await;
        let d = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(d.path().into()).unwrap();
        let c = connect(&m, &b, "fixture").await;
        {
            let mut s = state.lock().unwrap();
            s.deny = denied;
            s.wrong_membership = !denied;
        }
        let r = done(&submit(&c)).await;
        assert_eq!(r.state, OperationState::Rejected, "{}", r.detail);
        assert_eq!(count(&m, "session.change_password"), 0);
    }
}
#[tokio::test]
async fn late_password_completion_after_recovery_retains_receipt_without_login_or_rotation() {
    for acknowledge in [false, true] {
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let (m, _) = fixture(false, Some((entered.clone(), release.clone()))).await;
        let d = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(d.path().into()).unwrap();
        let c = connect(&m, &b, "fixture").await;
        let op = submit(&c);
        entered.notified().await;
        let recovery = Backend::with_state_directory(d.path().into()).unwrap();
        let peer = connect(&m, &recovery, PASSWORD).await;
        let id = op.latest().id;
        history(&peer, id, |r| r.state == OperationState::OutcomeUnknown).await;
        if acknowledge {
            acknowledge_unknown(&peer, id, 5).await;
        }
        let logins = count(&m, "session.login_with_password");
        release.notify_one();
        let r = history(&c, id, |r| {
            r.root_password
                .as_ref()
                .is_some_and(|p| p.coordinator_changed)
        })
        .await;
        assert_eq!(count(&m, "session.login_with_password"), logins);
        assert!(
            !r.root_password
                .as_ref()
                .unwrap()
                .reconnect_credentials_updated
        );
        privacy(d.path(), &r);
        assert_eq!(
            r.state,
            if acknowledge {
                OperationState::AcknowledgedUnknown
            } else {
                OperationState::OutcomeUnknown
            }
        );
        if !acknowledge {
            peer.control_resource(id, OperationControl::Recheck)
                .unwrap();
            let r = history(&peer, id, |r| r.state.terminal()).await;
            assert_eq!(r.state, OperationState::PartiallyCompleted);
        }
        assert_eq!(count(&m, "session.change_password"), 1);
    }
}
