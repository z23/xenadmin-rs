use super::*;
use std::collections::BTreeMap;
use xenadmin_core::{
    HostId, HostPowerConfiguration, HostPowerOperation, HostPowerPassword, HostPowerSettings,
    OperationControl, OperationId, OperationState, ResourceCommand, ResourceOperation,
    ResourceOperationRecord, ResourceTarget,
};
const POOL: &str = "11111111-1111-1111-1111-111111111111";
const HOST: &str = "22222222-2222-2222-2222-222222222222";
const PASSWORD: &str = "private-bmc-test-password-7452";
type Records = Arc<Mutex<BTreeMap<String, Value>>>;
type Gate = Option<(
    &'static str,
    Arc<tokio::sync::Notify>,
    Arc<tokio::sync::Notify>,
)>;
fn target() -> ResourceTarget {
    ResourceTarget::Host(HostId::parse(HOST).unwrap())
}
fn empty() -> HostPowerConfiguration {
    HostPowerConfiguration {
        settings: HostPowerSettings::Disabled,
        secret: None,
    }
}
fn settings() -> HostPowerSettings {
    HostPowerSettings::Ipmi {
        address: "192.0.2.60".parse().unwrap(),
        username: "fixture-admin".into(),
    }
}
async fn fixture(lost: &'static str, gate: Gate) -> (Mock, Records) {
    let data: Records = Arc::new(Mutex::new(BTreeMap::from([
        (
            "pool".into(),
            json!({"uuid":POOL,"name_label":"Power fixture","master":"OpaqueRef:host"}),
        ),
        (
            "host".into(),
            json!({"uuid":HOST,"name_label":"Host fixture","address":"192.0.2.10","enabled":true,"power_on_mode":"","power_on_config":{},"current_operations":{}}),
        ),
    ])));
    let records = data.clone();
    let mock=mock_overridden(override_rpc(move|r|{let data=records.clone();let gate=gate.clone();async move{
  let method=r["method"].as_str().unwrap();let p=&r["params"];
  let result={let mut m=data.lock().unwrap();
   if method=="event.from"&&p[2]==""{let mut b=batch("initial","Halted");for e in b["events"].as_array_mut().unwrap(){if let Some(v)=m.get(e["class"].as_str().unwrap()){e["snapshot"]=v.clone();}}return Some(RpcReply::Json(json!({"result":b})));}
   if let Some(class)=method.strip_suffix(".get_by_uuid")&&["pool","host","secret"].contains(&class){return Some(RpcReply::Json(match m.iter().find(|(k,v)|k.starts_with(class)&&v["uuid"]==p[1]){Some((k,_))=>json!({"result":format!("OpaqueRef:{k}")}),None=>json!({"error":{"message":"UUID_INVALID","data":[]}})}));}
   if method=="secret.get_all"{return Some(RpcReply::Json(json!({"result":m.keys().filter(|k|k.starts_with("secret")).map(|k|format!("OpaqueRef:{k}")).collect::<Vec<_>>()})));}
   if ["secret.get_uuid","secret.get_other_config"].contains(&method){let k=p[1].as_str().unwrap().trim_start_matches("OpaqueRef:");return Some(RpcReply::Json(json!({"result":m[k][if method.ends_with("uuid"){"uuid"}else{"other_config"}]})));}
   if let Some(class)=method.strip_suffix(".get_all_records")&&["host","PBD","SR"].contains(&class){if class=="host" && m.remove("_change_on_scan").is_some(){m.get_mut("host").unwrap()["power_on_config"]["power_on_user"]=json!("changed-after-review");}return Some(RpcReply::Json(json!({"result":m.iter().filter(|(k,_)|k.starts_with(class)).map(|(k,v)|(format!("OpaqueRef:{k}"),v.clone())).collect::<serde_json::Map<_,_>>()})));}
   if ["host.get_record","pool.get_record"].contains(&method){let k=p[1].as_str().unwrap().trim_start_matches("OpaqueRef:");return Some(RpcReply::Json(json!({"result":m[k]})));}
   match method {
    "secret.create"=>{assert_eq!(p[1]["value"],PASSWORD);let mut v=p[1].clone();v["uuid"]=json!("33333333-3333-3333-3333-333333333333");m.insert("secret-new".into(),v);Some(json!("OpaqueRef:secret-new"))},
    "secret.destroy"=>{let k=p[1].as_str().unwrap().trim_start_matches("OpaqueRef:");m.remove(k).unwrap();Some(Value::Null)},
    "host.set_power_on_mode"=>{assert!(!p.to_string().contains(PASSWORD));let old=m["host"]["power_on_config"]["power_on_password_secret"].clone();m.get_mut("host").unwrap()["power_on_mode"]=p[2].clone();m.get_mut("host").unwrap()["power_on_config"]=p[3].clone();if old.is_string(){let k=m.iter().find(|(k,v)|k.starts_with("secret")&&v["uuid"]==old).map(|(k,_)|k.clone());if let Some(k)=k{m.remove(&k);}}Some(Value::Null)},
    _=>{assert!(!method.starts_with("secret."),"Secret values must not be read: {method}");None}
   }
  };
  if let Some(result)=result {if let Some((name,entered,release))=gate&&name==method{entered.notify_one();release.notified().await;}return Some(if lost==method{RpcReply::Drop}else{RpcReply::Json(json!({"result":result}))});}None
 }})).await;
    (mock, data)
}
async fn connect(mock: &Mock, b: &Backend, ro: bool) -> xenadmin_core::Connection {
    let mut o = ConnectionOptions::new(mock.endpoint.clone());
    o.read_only = ro;
    let c = b.connect(o, Credentials::new("root".into(), "fixture".into()));
    c.wait_ready(deadline(3)).await.unwrap();
    c
}
fn submit(c: &xenadmin_core::Connection) -> ResourceOperation {
    c.configure_host_power(
        OperationId::new(),
        c.latest().observe_resource(target()).unwrap(),
        empty(),
        settings(),
        Some(HostPowerPassword::new(PASSWORD.into()).unwrap()),
    )
    .unwrap()
}
async fn done(op: &ResourceOperation) -> Arc<ResourceOperationRecord> {
    tokio::time::timeout(deadline(5), async {
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
    p: impl Fn(&ResourceOperationRecord) -> bool,
) -> ResourceOperationRecord {
    tokio::time::timeout(deadline(5), async {
        loop {
            if let Some(r) = c.resource_history().iter().find(|r| r.id == id && p(r)) {
                return r.clone();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}
async fn recheck(c: &xenadmin_core::Connection, id: OperationId) -> ResourceOperationRecord {
    let old = history(c, id, |r| r.state == OperationState::OutcomeUnknown)
        .await
        .revision();
    c.control_resource(id, OperationControl::Recheck).unwrap();
    history(c, id, |r| {
        r.revision() > old && (r.state.terminal() || r.state == OperationState::OutcomeUnknown)
    })
    .await
}
fn count(m: &Mock, method: &str) -> usize {
    m.methods
        .lock()
        .unwrap()
        .iter()
        .filter(|v| v.as_str() == method)
        .count()
}
fn privacy(dir: &std::path::Path, r: &ResourceOperationRecord) {
    let db = rusqlite::Connection::open_with_flags(
        dir.join("operations.sqlite3"),
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
    assert!(!public.contains(PASSWORD));
    assert!(!public.contains("OpaqueRef:"));
    assert!(!public.contains("host_power_plan"));
}
#[tokio::test]
async fn ipmi_uses_server_secret_then_disabled_mode_replaces_and_removes_old_secret() {
    let (m, data) = fixture("", None).await;
    let d = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(d.path().into()).unwrap();
    let c = connect(&m, &b, false).await;
    let r = done(&submit(&c)).await;
    assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
    let secret = r.host_power_secret.as_ref().unwrap().secret;
    assert_eq!(
        data.lock().unwrap()["host"]["power_on_config"]["power_on_password_secret"],
        secret.to_string()
    );
    privacy(d.path(), &r);
    let op = c
        .configure_host_power(
            OperationId::new(),
            c.latest().observe_resource(target()).unwrap(),
            HostPowerConfiguration {
                settings: settings(),
                secret: Some(secret),
            },
            HostPowerSettings::Disabled,
            None,
        )
        .unwrap();
    let r = done(&op).await;
    assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
    assert!(!data.lock().unwrap().contains_key("secret-new"));
    assert_eq!(count(&m, "secret.destroy"), 0);
    c.close().await;
}
#[tokio::test]
async fn lost_configuration_reply_cannot_be_promoted_by_matching_fields_or_cleaned_after_ack() {
    let (m, _) = fixture("host.set_power_on_mode", None).await;
    let d = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(d.path().into()).unwrap();
    let c = connect(&m, &b, false).await;
    let r = done(&submit(&c)).await;
    assert_eq!(r.state, OperationState::OutcomeUnknown);
    let checked = recheck(&c, r.id).await;
    assert_eq!(checked.state, OperationState::OutcomeUnknown);
    assert_eq!(count(&m, "host.set_power_on_mode"), 1);
    c.control_resource(r.id, OperationControl::AcknowledgeUnknown)
        .unwrap();
    history(&c, r.id, |r| r.state.terminal()).await;
    let cleanup = c
        .submit_resource(
            OperationId::new(),
            ResourceCommand::HostPower {
                host: c.latest().observe_resource(target()).unwrap(),
                operation: HostPowerOperation::CleanupSecret {
                    operation: r.id,
                    secret: r.host_power_secret.as_ref().unwrap().secret,
                },
            },
        )
        .unwrap();
    assert_eq!(done(&cleanup).await.state, OperationState::Rejected);
    assert_eq!(count(&m, "secret.destroy"), 0);
    c.close().await;
}
#[tokio::test]
async fn lost_secret_creation_is_recovered_for_reviewed_cleanup_without_setting_replay() {
    let (m, _) = fixture("secret.create", None).await;
    let d = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(d.path().into()).unwrap();
    let c = connect(&m, &b, false).await;
    let r = done(&submit(&c)).await;
    assert_eq!(r.state, OperationState::OutcomeUnknown);
    let r = recheck(&c, r.id).await;
    assert_eq!(r.state, OperationState::PartiallyCompleted, "{}", r.detail);
    let cleanup = c
        .submit_resource(
            OperationId::new(),
            ResourceCommand::HostPower {
                host: c.latest().observe_resource(target()).unwrap(),
                operation: HostPowerOperation::CleanupSecret {
                    operation: r.id,
                    secret: r.host_power_secret.as_ref().unwrap().secret,
                },
            },
        )
        .unwrap();
    assert_eq!(done(&cleanup).await.state, OperationState::Succeeded);
    assert_eq!(count(&m, "secret.create"), 1);
    assert_eq!(count(&m, "host.set_power_on_mode"), 0);
    assert_eq!(count(&m, "secret.destroy"), 1);
    c.close().await;
}
#[tokio::test]
async fn changed_review_or_shared_old_secret_is_refused_before_change() {
    for shared in [false, true] {
        let (m, data) = fixture("", None).await;
        let d = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(d.path().into()).unwrap();
        let c = connect(&m, &b, false).await;
        if !shared {
            data.lock().unwrap().get_mut("host").unwrap()["power_on_mode"] = json!("wake-on-lan");
            let r = done(&submit(&c)).await;
            assert_eq!(r.state, OperationState::Rejected);
            assert_eq!(count(&m, "secret.create"), 0);
        } else {
            let r = done(&submit(&c)).await;
            let secret = r.host_power_secret.as_ref().unwrap().secret;
            data.lock().unwrap().insert("host-other".into(),json!({"uuid":"44444444-4444-4444-4444-444444444444","power_on_config":{"power_on_password_secret":secret.to_string()}}));
            let op = c
                .configure_host_power(
                    OperationId::new(),
                    c.latest().observe_resource(target()).unwrap(),
                    HostPowerConfiguration {
                        settings: settings(),
                        secret: Some(secret),
                    },
                    HostPowerSettings::Disabled,
                    None,
                )
                .unwrap();
            assert_eq!(done(&op).await.state, OperationState::Rejected);
            assert_eq!(count(&m, "host.set_power_on_mode"), 1);
        }
        c.close().await;
    }
}
#[tokio::test]
async fn recovered_late_secret_receipt_preserves_acknowledgement_and_never_sets_host() {
    for acknowledge in [false, true] {
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let (m, _) = fixture(
            "",
            Some(("secret.create", entered.clone(), release.clone())),
        )
        .await;
        let d = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(d.path().into()).unwrap();
        let c = connect(&m, &b, false).await;
        let op = submit(&c);
        tokio::time::timeout(deadline(3), entered.notified())
            .await
            .unwrap();
        let recovery = Backend::with_state_directory(d.path().into()).unwrap();
        let peer = connect(&m, &recovery, false).await;
        history(&peer, op.latest().id, |r| {
            r.state == OperationState::OutcomeUnknown
        })
        .await;
        if acknowledge {
            acknowledge_unknown(&peer, op.latest().id, 5).await;
        }
        release.notify_one();
        let r = history(&c, op.latest().id, |r| r.host_power_secret.is_some()).await;
        assert_eq!(
            r.state,
            if acknowledge {
                OperationState::AcknowledgedUnknown
            } else {
                OperationState::OutcomeUnknown
            }
        );
        assert_eq!(count(&m, "host.set_power_on_mode"), 0);
        privacy(d.path(), &r);
        c.close().await;
        peer.close().await;
    }
}
#[tokio::test]
async fn host_power_requires_write_authority_and_matching_observation() {
    let (m, _) = fixture("", None).await;
    let d = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(d.path().into()).unwrap();
    let c = connect(&m, &b, true).await;
    let observed = c.latest().observe_resource(target()).unwrap();
    assert!(matches!(
        c.configure_host_power(
            OperationId::new(),
            observed,
            empty(),
            settings(),
            Some(HostPowerPassword::new(PASSWORD.into()).unwrap())
        ),
        Err(xenadmin_core::Error::ReadOnly)
    ));
    c.close().await;
    let writer = connect(&m, &b, false).await;
    assert!(matches!(
        writer.configure_host_power(
            OperationId::new(),
            observed,
            empty(),
            settings(),
            Some(HostPowerPassword::new(PASSWORD.into()).unwrap())
        ),
        Err(xenadmin_core::Error::Stale)
    ));
    assert_eq!(count(&m, "secret.create"), 0);
    c.close().await;
    writer.close().await;
}
#[tokio::test]
async fn changed_selected_host_during_consumer_scan_prevents_replacing_its_secret() {
    let (m, data) = fixture("", None).await;
    let d = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(d.path().into()).unwrap();
    let c = connect(&m, &b, false).await;
    let r = done(&submit(&c)).await;
    let secret = r.host_power_secret.as_ref().unwrap().secret;
    data.lock()
        .unwrap()
        .insert("_change_on_scan".into(), json!(true));
    let op = c
        .configure_host_power(
            OperationId::new(),
            c.latest().observe_resource(target()).unwrap(),
            HostPowerConfiguration {
                settings: settings(),
                secret: Some(secret),
            },
            HostPowerSettings::Disabled,
            None,
        )
        .unwrap();
    let r = done(&op).await;
    assert_eq!(r.state, OperationState::Rejected, "{}", r.detail);
    assert_eq!(count(&m, "host.set_power_on_mode"), 1);
    assert!(data.lock().unwrap().contains_key("secret-new"));
    c.close().await;
}
#[tokio::test]
async fn retained_secret_cleanup_refuses_a_new_storage_consumer() {
    let (m, data) = fixture("secret.create", None).await;
    let d = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(d.path().into()).unwrap();
    let c = connect(&m, &b, false).await;
    let r = done(&submit(&c)).await;
    let r = recheck(&c, r.id).await;
    let secret = r.host_power_secret.as_ref().unwrap().secret;
    data.lock().unwrap().insert("PBD-consumer".into(),json!({"uuid":"44444444-4444-4444-4444-444444444444","device_config":{"credential_secret":secret.to_string()}}));
    let cleanup = c
        .submit_resource(
            OperationId::new(),
            ResourceCommand::HostPower {
                host: c.latest().observe_resource(target()).unwrap(),
                operation: HostPowerOperation::CleanupSecret {
                    operation: r.id,
                    secret,
                },
            },
        )
        .unwrap();
    assert_eq!(done(&cleanup).await.state, OperationState::Rejected);
    assert_eq!(count(&m, "secret.destroy"), 0);
    assert!(data.lock().unwrap().contains_key("secret-new"));
    c.close().await;
}
fn spelled(id: &str, spelling: &str) -> String {
    match spelling {
        "simple" => id.to_string(),
        "urn" => format!("urn:uuid:{id}"),
        "braced" => format!("{{{id}}}"),
        "upper" => id.to_ascii_uppercase(),
        "other" => "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb".into(),
        _ => unreachable!("{spelling}"),
    }
}
async fn host_cleanup_spelling(spelling: &str) -> String {
    let (m, data) = fixture("secret.create", None).await;
    let d = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(d.path().into()).unwrap();
    let c = connect(&m, &b, false).await;
    let r = done(&submit(&c)).await;
    assert_eq!(r.state, OperationState::OutcomeUnknown, "{}", r.detail);
    data.lock().unwrap().get_mut("secret-new").unwrap()["uuid"] =
        json!("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa");
    let r = recheck(&c, r.id).await;
    assert_eq!(r.state, OperationState::PartiallyCompleted, "{}", r.detail);
    let secret = r.host_power_secret.as_ref().unwrap().secret;
    assert_eq!(secret.to_string(), "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa");
    data.lock().unwrap().get_mut("host").unwrap()["power_on_config"] =
        json!({"power_on_password_secret": spelled(&secret.to_string(), spelling)});
    let cleanup = c
        .submit_resource(
            OperationId::new(),
            ResourceCommand::HostPower {
                host: c.latest().observe_resource(target()).unwrap(),
                operation: HostPowerOperation::CleanupSecret {
                    operation: r.id,
                    secret,
                },
            },
        )
        .unwrap();
    let outcome = done(&cleanup).await;
    let present = data.lock().unwrap().contains_key("secret-new");
    let destroy = count(&m, "secret.destroy");
    let row = format!(
        "spelling={spelling} state={:?} destroy={destroy} secret_present={present} detail={}",
        outcome.state, outcome.detail
    );
    c.close().await;
    row
}
#[tokio::test]
async fn cleanup_keeps_a_host_secret_for_every_id_spelling() {
    let mut rows = Vec::new();
    for spelling in ["simple", "urn", "braced", "upper", "other"] {
        rows.push(host_cleanup_spelling(spelling).await);
    }
    assert_eq!(
        rows,
        [
            "spelling=simple state=Rejected destroy=0 secret_present=true detail=Invalid XAPI response: Secret is still referenced by a host; it was not deleted",
            "spelling=urn state=Rejected destroy=0 secret_present=true detail=Invalid XAPI response: Secret is still referenced by a host; it was not deleted",
            "spelling=braced state=Rejected destroy=0 secret_present=true detail=Invalid XAPI response: Secret is still referenced by a host; it was not deleted",
            "spelling=upper state=Rejected destroy=0 secret_present=true detail=Invalid XAPI response: Secret is still referenced by a host; it was not deleted",
            "spelling=other state=Succeeded destroy=1 secret_present=false detail=Owned unused host power Secret removed",
        ]
    );
}
