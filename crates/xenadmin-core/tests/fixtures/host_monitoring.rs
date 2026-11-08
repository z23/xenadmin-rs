use super::*;
use xenadmin_core::{HostDataSource, HostRecordingAction};

fn source(enabled: bool, standard: bool) -> Value {
    json!({"name_label":"cpu7","name_description":"CPU 7 utilization","units":"fraction","standard":standard,"enabled":enabled,"min":0,"max":"Infinity"})
}
async fn catalog(c: &xenadmin_core::Connection) -> Result<xenadmin_core::HostDataSources, String> {
    let q = c
        .host_data_sources(c.latest().observe_resource(host()).unwrap())
        .unwrap();
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            if let Some(result) = q.latest() {
                return result;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap()
}
fn operation(expected: HostDataSource, action: HostRecordingAction) -> HostOperation {
    HostOperation::DataSource { expected, action }
}
#[tokio::test]
async fn recording_and_forgetting_use_exact_source_and_distinguish_automatic_restart() {
    for (action, auto) in [
        (HostRecordingAction::StartRecording, false),
        (HostRecordingAction::ForgetHistory, false),
        (HostRecordingAction::ForgetHistory, true),
    ] {
        let (mock, records) = fixture("", "").await;
        records.lock().unwrap().insert(
            "_sources".into(),
            json!([source(action == HostRecordingAction::ForgetHistory, auto)]),
        );
        records
            .lock()
            .unwrap()
            .insert("_auto_record".into(), json!(auto));
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&mock, &backend, false).await;
        let captured = catalog(&c).await.unwrap();
        assert_eq!(captured.host, HostId::parse(HOST).unwrap());
        assert_eq!(captured.sources[0].maximum, "Infinity");
        let op = c
            .submit_resource(
                OperationId::new(),
                command(&c, operation(captured.sources[0].clone(), action)),
            )
            .unwrap();
        let result = done(&op).await;
        assert_eq!(result.state, OperationState::Succeeded, "{}", result.detail);
        assert_eq!(
            catalog(&c).await.unwrap().sources[0].enabled,
            auto || action == HostRecordingAction::StartRecording
        );
        if action == HostRecordingAction::ForgetHistory {
            assert!(result.detail.contains("may already have restarted"));
        }
        let public = serde_json::to_string(&*result).unwrap();
        assert!(!public.contains("OpaqueRef:"));
        let methods = mock.methods.lock().unwrap().clone();
        assert_eq!(
            methods
                .iter()
                .filter(|m| [
                    "host.record_data_source",
                    "host.forget_data_source_archives"
                ]
                .contains(&m.as_str()))
                .count(),
            1
        );
        c.close().await;
    }
}
#[tokio::test]
async fn missing_recording_or_deletion_reply_stays_unknown_even_when_state_matches() {
    for (action, method) in [
        (
            HostRecordingAction::StartRecording,
            "host.record_data_source",
        ),
        (
            HostRecordingAction::ForgetHistory,
            "host.forget_data_source_archives",
        ),
    ] {
        let (mock, records) = fixture(method, "").await;
        records.lock().unwrap().insert(
            "_sources".into(),
            json!([source(action == HostRecordingAction::ForgetHistory, false)]),
        );
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&mock, &backend, false).await;
        let expected = catalog(&c).await.unwrap().sources.remove(0);
        let op = c
            .submit_resource(OperationId::new(), command(&c, operation(expected, action)))
            .unwrap();
        let result = done(&op).await;
        assert_eq!(result.state, OperationState::OutcomeUnknown);
        let rechecked = recheck(&c, result.id, &op).await;
        assert_eq!(rechecked.state, OperationState::OutcomeUnknown);
        assert!(rechecked.detail.contains("completion reply was lost"));
        assert_eq!(
            mock.methods
                .lock()
                .unwrap()
                .iter()
                .filter(|m| *m == method)
                .count(),
            1
        );
        c.close().await;
    }
}
#[tokio::test]
async fn changed_source_metadata_or_recording_state_refuses_before_mutation() {
    for field in [
        "name_label",
        "name_description",
        "units",
        "standard",
        "enabled",
        "min",
        "max",
    ] {
        let (mock, records) = fixture("", "").await;
        records
            .lock()
            .unwrap()
            .insert("_sources".into(), json!([source(false, false)]));
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&mock, &backend, false).await;
        let expected = catalog(&c).await.unwrap().sources.remove(0);
        records.lock().unwrap().get_mut("_sources").unwrap()[0][field] = match field {
            "standard" | "enabled" => json!(true),
            "min" | "max" => json!(42),
            _ => json!("changed"),
        };
        let op = c
            .submit_resource(
                OperationId::new(),
                command(&c, operation(expected, HostRecordingAction::StartRecording)),
            )
            .unwrap();
        assert_eq!(done(&op).await.state, OperationState::Rejected, "{field}");
        assert!(
            !mock
                .methods
                .lock()
                .unwrap()
                .iter()
                .any(|m| m == "host.record_data_source")
        );
        c.close().await;
    }
}
#[tokio::test]
async fn catalog_is_readonly_generation_bound_and_rejects_incomplete_or_duplicate_metadata() {
    let (mock, records) = fixture("", "").await;
    records
        .lock()
        .unwrap()
        .insert("_sources".into(), json!([source(false, false)]));
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&mock, &backend, true).await;
    let expected = catalog(&c).await.unwrap().sources.remove(0);
    assert!(matches!(
        c.submit_resource(
            OperationId::new(),
            command(&c, operation(expected, HostRecordingAction::StartRecording))
        ),
        Err(xenadmin_core::Error::ReadOnly)
    ));
    let observed = c.latest().observe_resource(host()).unwrap();
    for invalid in [
        json!([source(false, false), source(false, false)]),
        json!([{"name_label":"cpu7"}]),
        json!(vec![source(false, false); 513]),
    ] {
        records.lock().unwrap().insert("_sources".into(), invalid);
        assert!(catalog(&c).await.is_err());
    }
    c.close().await;
    let c = connect(&mock, &backend, true).await;
    assert!(matches!(
        c.host_data_sources(observed),
        Err(xenadmin_core::Error::Stale)
    ));
    c.close().await;
}
#[tokio::test]
async fn known_permission_denial_refuses_recording_before_dispatch() {
    let (mock, records) = fixture("", "").await;
    records
        .lock()
        .unwrap()
        .insert("_sources".into(), json!([source(false, false)]));
    records.lock().unwrap().insert(
        "_session".into(),
        json!({"is_local_superuser":false,"rbac_permissions":["session.logout", "pool.get_by_uuid", "pool.get_record", "host.get_by_uuid", "host.get_record", "host.get_data_sources", "host_metrics.get_record"]}),
    );
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&mock, &backend, false).await;
    // Discovery methods are granted; the known recording-write absence is authoritative.
    let expected = catalog(&c).await.unwrap().sources.remove(0);
    let op = c
        .submit_resource(
            OperationId::new(),
            command(&c, operation(expected, HostRecordingAction::StartRecording)),
        )
        .unwrap();
    assert_eq!(done(&op).await.state, OperationState::Rejected);
    assert!(
        !mock
            .methods
            .lock()
            .unwrap()
            .iter()
            .any(|m| m == "host.record_data_source")
    );
    c.close().await;
}

#[tokio::test]
async fn positive_receipt_recovers_after_restart_without_recording_again() {
    let (mock, records) = fixture("", "").await;
    records
        .lock()
        .unwrap()
        .insert("_sources".into(), json!([source(false, false)]));
    records.lock().unwrap().insert(
        "_after_monitoring_description".into(),
        json!("Changed after setter"),
    );
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&mock, &backend, false).await;
    let expected = catalog(&c).await.unwrap().sources.remove(0);
    let op = c
        .submit_resource(
            OperationId::new(),
            command(&c, operation(expected, HostRecordingAction::StartRecording)),
        )
        .unwrap();
    let result = done(&op).await;
    assert_eq!(result.state, OperationState::OutcomeUnknown);
    c.close().await;
    records.lock().unwrap().get_mut("_sources").unwrap()[0]["name_description"] =
        json!("CPU 7 utilization");
    let recovered = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&mock, &recovered, false).await;
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            if c.resource_history()
                .iter()
                .any(|r| r.id == result.id && r.state == OperationState::Succeeded)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        mock.methods
            .lock()
            .unwrap()
            .iter()
            .filter(|m| *m == "host.record_data_source")
            .count(),
        1
    );
    c.close().await;
}

#[tokio::test]
async fn positive_forget_receipt_allows_an_archival_source_to_disappear() {
    let (mock, records) = fixture("", "").await;
    records
        .lock()
        .unwrap()
        .insert("_sources".into(), json!([source(false, false)]));
    records
        .lock()
        .unwrap()
        .insert("_forget_remove".into(), json!(true));
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&mock, &backend, false).await;
    let expected = catalog(&c).await.unwrap().sources.remove(0);
    let op = c
        .submit_resource(
            OperationId::new(),
            command(&c, operation(expected, HostRecordingAction::ForgetHistory)),
        )
        .unwrap();
    assert_eq!(done(&op).await.state, OperationState::Succeeded);
    assert!(catalog(&c).await.unwrap().sources.is_empty());
    c.close().await;
}
