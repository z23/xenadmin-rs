use super::*;
use std::collections::BTreeMap;
use xenadmin_core::{
    AlarmEmailSettings, AlarmMetric, AlarmOperation, AlarmPolicyChange, AlarmRule, HostId,
    OperationControl, OperationId, OperationState, PoolId, ResourceCommand, ResourceOperation,
    ResourceOperationRecord, ResourceTarget, VmId,
};
const POOL: &str = "11111111-1111-1111-1111-111111111111";
const HOST: &str = "22222222-2222-2222-2222-222222222222";
const VM: &str = "33333333-3333-3333-3333-333333333333";
const XML: &str = r#"<config><variable><name value="cpu_usage"/><alarm_trigger_level value="0.5"/><alarm_priority value="2"/></variable><variable><name value="custom"/><rrd_regex value="private-custom-source"/></variable></config>"#;
#[derive(Clone, Copy, Debug, PartialEq)]
enum Fault {
    None,
    Lost,
    Stale,
    Rebound,
    Dom0,
    Denied,
    RefreshFalse,
    ReadbackCustom,
}
type Records = Arc<Mutex<BTreeMap<String, Value>>>;
async fn fixture(fault: Fault) -> (Mock, Records) {
    let mut vm = batch("initial", "Halted")["events"][2]["snapshot"].clone();
    vm["is_a_template"] = json!(false);
    vm["is_a_snapshot"] = json!(false);
    vm["is_control_domain"] = json!(false);
    vm["other_config"] = json!({"perfmon":XML,"preserve":"private-other"});
    let records = Arc::new(Mutex::new(BTreeMap::from([
        ("VM".into(), vm),
        (
            "pool".into(),
            json!({"uuid":POOL,"master":"OpaqueRef:host","other_config":{"mail-destination":"old@example.invalid","ssmtp-mailhub":"relay.example.invalid:25","ssmtp-authpass":"private-smtp-secret","preserve":"keep"}}),
        ),
        (
            "host".into(),
            json!({"uuid":HOST,"name_label":"Fixture host","address":"192.0.2.10","enabled":true,"metrics":"OpaqueRef:metrics","other_config":{"perfmon":XML}}),
        ),
    ])));
    let captured = records.clone();
    let mock = mock_overridden(override_rpc(move |r| {
        let records = captured.clone();
        async move {
            let method = r["method"].as_str().unwrap();
            let p = &r["params"];
            let value = if method == "event.from" && p[2] == "" {
                let mut b = batch("initial", "Halted");
                let mut map = records.lock().unwrap();
                for event in b["events"].as_array_mut().unwrap() {
                    if let Some(value) = map.get(event["class"].as_str().unwrap()) {
                        event["snapshot"] = value.clone();
                    }
                }
                match fault {
                    Fault::Stale => {
                        map.get_mut("VM").unwrap()["other_config"]["perfmon"] = json!("<config/>")
                    }
                    Fault::Rebound => {
                        map.get_mut("VM").unwrap()["uuid"] =
                            json!("44444444-4444-4444-4444-444444444444")
                    }
                    Fault::Dom0 => map.get_mut("VM").unwrap()["is_control_domain"] = json!(true),
                    _ => {}
                }
                Some(b)
            } else if method == "event.from" {
                tokio::time::sleep(Duration::from_millis(30)).await;
                Some(json!({"token":"initial","events":[]}))
            } else if method == "session.get_record" {
                Some(json!({"is_local_superuser":fault!=Fault::Denied,"rbac_permissions":[]}))
            } else if let Some(class) = method.strip_suffix(".get_by_uuid") {
                if ["VM", "host", "pool"].contains(&class) {
                    Some(json!(format!("OpaqueRef:{class}")))
                } else {
                    None
                }
            } else if let Some(class) = method.strip_suffix(".get_record") {
                if ["VM", "host", "pool"].contains(&class) {
                    Some(records.lock().unwrap()[class].clone())
                } else if class == "host_metrics" {
                    Some(json!({"live":true}))
                } else {
                    None
                }
            } else if let Some(class) = method.strip_suffix(".set_other_config") {
                assert!(["VM", "host", "pool"].contains(&class));
                records.lock().unwrap().get_mut(class).unwrap()["other_config"] = p[2].clone();
                if fault == Fault::ReadbackCustom {
                    let xml = p[2]["perfmon"].as_str().unwrap().replace(
                        "<alarm_trigger_level",
                        "<rrd_regex value=\"custom.*\"/><alarm_trigger_level",
                    );
                    records.lock().unwrap().get_mut(class).unwrap()["other_config"]["perfmon"] =
                        json!(xml);
                }
                if fault == Fault::Lost {
                    return Some(RpcReply::Drop);
                }
                Some(Value::Null)
            } else if method == "host.call_plugin" {
                assert_eq!(p[2], "perfmon");
                assert_eq!(p[3], "refresh");
                assert_eq!(p[4], json!({}));
                if fault == Fault::Lost {
                    return Some(RpcReply::Drop);
                }
                Some(json!(if fault == Fault::RefreshFalse {
                    "False"
                } else {
                    "True"
                }))
            } else {
                None
            };
            value.map(|value| RpcReply::Json(json!({"result":value})))
        }
    }))
    .await;
    (mock, records)
}
async fn connect(mock: &Mock, backend: &Backend) -> xenadmin_core::Connection {
    let c = backend.connect(
        ConnectionOptions::new(mock.endpoint.clone()),
        Credentials::new("root".into(), "fixture".into()),
    );
    c.wait_ready(Duration::from_secs(8)).await.unwrap();
    c
}
fn policy(c: &xenadmin_core::Connection) -> ResourceCommand {
    let snapshot = c.latest();
    let target = ResourceTarget::Vm(VmId::parse(VM).unwrap());
    let p = snapshot
        .inventory()
        .unwrap()
        .details
        .alarm_policies
        .iter()
        .find(|p| p.target == target)
        .unwrap();
    ResourceCommand::Alarm {
        target: snapshot.observe_resource(target).unwrap(),
        operation: AlarmOperation::SetPolicy(AlarmPolicyChange {
            expected_revision: p.revision.clone(),
            control_domain_host: None,
            metric: AlarmMetric::Cpu,
            rule: Some(AlarmRule {
                metric: AlarmMetric::Cpu,
                threshold: "0.85".into(),
                duration_seconds: 120,
                repeat_seconds: 600,
            }),
        }),
    }
}
async fn finish(op: &ResourceOperation) -> Arc<ResourceOperationRecord> {
    tokio::time::timeout(Duration::from_secs(10), async {
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
#[tokio::test]
async fn alarm_threshold_changes_preserve_private_metadata_custom_variables_and_priority() {
    let (mock, records) = fixture(Fault::None).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&mock, &backend).await;
    let projected = serde_json::to_string(c.latest().inventory().unwrap().as_ref()).unwrap();
    assert!(!projected.contains("private-custom-source"));
    assert!(!projected.contains("private-smtp-secret"));
    let op = c.submit_resource(OperationId::new(), policy(&c)).unwrap();
    assert_eq!(finish(&op).await.state, OperationState::Succeeded);
    {
        let map = records.lock().unwrap();
        let xml = map["VM"]["other_config"]["perfmon"].as_str().unwrap();
        assert!(
            xml.contains("0.85")
                && xml.contains("private-custom-source")
                && xml.contains("alarm_priority value=\"2\"")
        );
        assert_eq!(map["VM"]["other_config"]["preserve"], "private-other");
    }
    assert!(
        !mock
            .methods
            .lock()
            .unwrap()
            .iter()
            .any(|m| m == "host.call_plugin")
    );
    c.close().await;
}
#[tokio::test]
async fn alarm_stale_identity_dom0_and_permission_changes_refuse_before_writes() {
    for fault in [Fault::Stale, Fault::Rebound, Fault::Dom0, Fault::Denied] {
        let (mock, _) = fixture(fault).await;
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&mock, &backend).await;
        let op = c.submit_resource(OperationId::new(), policy(&c)).unwrap();
        assert_eq!(
            finish(&op).await.state,
            OperationState::Rejected,
            "{fault:?}"
        );
        assert!(
            !mock
                .methods
                .lock()
                .unwrap()
                .iter()
                .any(|m| m.ends_with(".set_other_config"))
        );
        c.close().await;
    }
}
#[tokio::test]
async fn alarm_email_changes_preserve_smtp_authentication_and_do_not_send_mail() {
    let (mock, records) = fixture(Fault::None).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&mock, &backend).await;
    let snapshot = c.latest();
    let email = snapshot
        .inventory()
        .unwrap()
        .details
        .alarm_email
        .as_ref()
        .unwrap();
    let command = ResourceCommand::Alarm {
        target: snapshot
            .observe_resource(ResourceTarget::Pool(PoolId::parse(POOL).unwrap()))
            .unwrap(),
        operation: AlarmOperation::SetEmail {
            expected_revision: email.revision.clone(),
            settings: AlarmEmailSettings {
                destination: "new@example.invalid".into(),
                relay: "smtp.example.invalid:2525".into(),
                language: "ja-JP".into(),
            },
        },
    };
    let op = c.submit_resource(OperationId::new(), command).unwrap();
    assert_eq!(finish(&op).await.state, OperationState::Succeeded);
    assert_eq!(
        records.lock().unwrap()["pool"]["other_config"]["ssmtp-authpass"],
        "private-smtp-secret"
    );
    assert!(
        !mock
            .methods
            .lock()
            .unwrap()
            .iter()
            .any(|m| m == "host.call_plugin")
    );
    c.close().await;
}
#[tokio::test]
async fn alarm_refresh_requires_positive_receipt_and_never_replays_lost_reply() {
    for fault in [Fault::None, Fault::RefreshFalse, Fault::Lost] {
        let (mock, _) = fixture(fault).await;
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&mock, &backend).await;
        let id = OperationId::new();
        let command = ResourceCommand::Alarm {
            target: c
                .latest()
                .observe_resource(ResourceTarget::Pool(PoolId::parse(POOL).unwrap()))
                .unwrap(),
            operation: AlarmOperation::Refresh {
                hosts: vec![HostId::parse(HOST).unwrap()],
            },
        };
        let op = c.submit_resource(id, command).unwrap();
        let final_record = finish(&op).await;
        if fault == Fault::None {
            assert_eq!(final_record.state, OperationState::Succeeded);
            assert!(
                final_record.steps[0]
                    .detail
                    .contains("email delivery are not verified")
            );
        } else {
            assert_ne!(final_record.state, OperationState::Succeeded);
        }
        if fault == Fault::Lost {
            c.control_resource(id, OperationControl::Recheck).unwrap();
            tokio::time::timeout(Duration::from_secs(8), async {
                loop {
                    if c.resource_history()
                        .iter()
                        .any(|r| r.id == id && r.revision() > final_record.revision())
                    {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
        }
        assert_eq!(
            mock.methods
                .lock()
                .unwrap()
                .iter()
                .filter(|m| m.as_str() == "host.call_plugin")
                .count(),
            1
        );
        c.close().await;
    }
}

#[tokio::test]
async fn alarm_lost_setter_reconciles_by_readback_without_replay() {
    let (mock, _) = fixture(Fault::Lost).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&mock, &backend).await;
    let id = OperationId::new();
    let op = c.submit_resource(id, policy(&c)).unwrap();
    let first = finish(&op).await;
    assert_eq!(first.state, OperationState::OutcomeUnknown);
    c.control_resource(id, OperationControl::Recheck).unwrap();
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            if c.resource_history()
                .iter()
                .any(|r| r.id == id && r.state == OperationState::Succeeded)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        mock.methods
            .lock()
            .unwrap()
            .iter()
            .filter(|m| m.as_str() == "VM.set_other_config")
            .count(),
        1
    );
    c.close().await;
}

#[tokio::test]
async fn alarm_changed_source_after_setter_is_not_certified_as_plain_threshold() {
    let (mock, _) = fixture(Fault::ReadbackCustom).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&mock, &backend).await;
    let op = c.submit_resource(OperationId::new(), policy(&c)).unwrap();
    let record = finish(&op).await;
    assert_eq!(record.state, OperationState::OutcomeUnknown);
    assert_eq!(
        mock.methods
            .lock()
            .unwrap()
            .iter()
            .filter(|m| m.as_str() == "VM.set_other_config")
            .count(),
        1
    );
    c.close().await;
}
