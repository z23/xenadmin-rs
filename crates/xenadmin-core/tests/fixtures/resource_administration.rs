use super::*;
use std::collections::BTreeMap;
use xenadmin_core::{
    HostId, NetworkId, OperationControl, OperationId, OperationState, PoolId, PowerAction,
    ResourceCommand, ResourceOperation, ResourceOperationRecord, ResourceTarget, SrId, VmCommand,
};
const POOL: &str = "11111111-1111-1111-1111-111111111111";
const HOST: &str = "22222222-2222-2222-2222-222222222222";
const NETWORK: &str = "55555555-5555-5555-5555-555555555555";
const STORAGE: &str = "66666666-6666-6666-6666-666666666666";
const CREATED: &str = "77777777-7777-7777-7777-777777777777";
#[derive(Clone, Copy, PartialEq)]
enum Fault {
    None,
    Description,
    DropRename,
    DropCreate,
    VerifyRead,
    ChangedMtu,
    AttachedPif,
    AttachedVif,
    MissingInterfaces,
    PendingTask,
    SlowRead,
}
type Records = Arc<Mutex<BTreeMap<String, Value>>>;
#[derive(Default)]
struct ReadGate {
    enabled: std::sync::atomic::AtomicBool,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
async fn fixture(fault: Fault) -> (Mock, Records) {
    let (mock, records, _) = gated_fixture(fault).await;
    (mock, records)
}
async fn gated_fixture(fault: Fault) -> (Mock, Records, Arc<ReadGate>) {
    let gate = Arc::new(ReadGate::default());
    let reader_gate = gate.clone();
    let records: Records = Arc::new(Mutex::new(BTreeMap::from([
        (
            "pool".into(),
            json!({"uuid":POOL,"name_label":"Fixture pool","name_description":"Pool description","default_SR":"OpaqueRef:NULL","master":"OpaqueRef:host"}),
        ),
        (
            "host".into(),
            json!({"uuid":HOST,"name_label":"Fixture host","name_description":"Host description","enabled":true,"address":"192.0.2.10"}),
        ),
        (
            "network".into(),
            json!({"uuid":NETWORK,"name_label":"Internal network","name_description":"Network description","bridge":"xapi0","MTU":"1500","PIFs":[],"VIFs":[],"allowed_operations":["attaching"],"other_config":{}}),
        ),
        (
            "SR".into(),
            json!({"uuid":STORAGE,"name_label":"Fixture storage","name_description":"Storage description","type":"lvm","content_type":"user","allowed_operations":["scan"],"physical_size":"1024","physical_utilisation":"0"}),
        ),
    ])));
    let mutations = Arc::new(AtomicUsize::new(0));
    let captured_records = records.clone();
    let callback = move |request: Value| {
        let gate = reader_gate.clone();
        let records = captured_records.clone();
        let records = records.clone();
        let mutations = mutations.clone();
        async move {
            let method = request["method"].as_str().unwrap();
            if method == "event.from" && request["params"][2] == "" {
                let mut initial = batch("initial", "Halted");
                let mut map = records.lock().unwrap();
                for event in initial["events"].as_array_mut().unwrap() {
                    if let Some(record) = map.get(event["class"].as_str().unwrap()) {
                        event["snapshot"] = record.clone();
                    }
                }
                for class in ["network", "SR"] {
                    initial["events"].as_array_mut().unwrap().push(json!({"class":class,"operation":"add","ref":format!("OpaqueRef:{class}"),"snapshot":map[class]}));
                }
                // Changes after the inventory read exercise fresh preflight, not button gating.
                if fault == Fault::ChangedMtu {
                    map.get_mut("network").unwrap()["MTU"] = json!(9000);
                }
                if fault == Fault::AttachedPif {
                    map.get_mut("network").unwrap()["PIFs"] = json!(["OpaqueRef:attached"]);
                }
                if fault == Fault::AttachedVif {
                    map.get_mut("network").unwrap()["VIFs"] = json!(["OpaqueRef:attached"]);
                }
                if fault == Fault::MissingInterfaces {
                    map.get_mut("network")
                        .unwrap()
                        .as_object_mut()
                        .unwrap()
                        .remove("VIFs");
                }
                return Some(RpcReply::Json(json!({"result":initial})));
            }
            if let Some(class) = method.strip_suffix(".get_by_uuid")
                && ["pool", "host", "network", "SR"].contains(&class)
            {
                let map = records.lock().unwrap();
                let found = map.iter().find(|(_, r)| r["uuid"] == request["params"][1]);
                return Some(RpcReply::Json(match found {
                    Some((key, _)) => json!({"result":format!("OpaqueRef:{key}")}),
                    None => json!({"error":{"code":1,"message":"UUID_INVALID","data":[]}}),
                }));
            }
            if let Some(class) = method.strip_suffix(".get_record")
                && ["pool", "host", "network", "SR"].contains(&class)
            {
                if gate.enabled.swap(false, Ordering::SeqCst) {
                    gate.entered.notify_one();
                    gate.release.notified().await;
                }
                if fault == Fault::SlowRead {
                    tokio::time::sleep(Duration::from_millis(150)).await;
                }
                if fault == Fault::VerifyRead && mutations.load(Ordering::SeqCst) > 0 {
                    return Some(RpcReply::Json(
                        json!({"error":{"code":1,"message":"RBAC_PERMISSION_DENIED","data":[]}}),
                    ));
                }
                let key = request["params"][1]
                    .as_str()
                    .unwrap()
                    .strip_prefix("OpaqueRef:")
                    .unwrap();
                return Some(RpcReply::Json(
                    json!({"result":records.lock().unwrap()[key]}),
                ));
            }
            if method == "network.get_all_records" {
                let map = records.lock().unwrap();
                let all = map
                    .iter()
                    .filter(|(key, _)| key.starts_with("network"))
                    .map(|(key, r)| (format!("OpaqueRef:{key}"), r.clone()))
                    .collect::<serde_json::Map<_, _>>();
                return Some(RpcReply::Json(json!({"result":all})));
            }
            if method == "network.create" {
                let mut fresh = request["params"][1].clone();
                fresh["uuid"] = json!(CREATED);
                fresh["PIFs"] = json!([]);
                fresh["VIFs"] = json!([]);
                records
                    .lock()
                    .unwrap()
                    .insert("network-created".into(), fresh);
                mutations.fetch_add(1, Ordering::SeqCst);
                return Some(if fault == Fault::DropCreate {
                    RpcReply::Drop
                } else {
                    RpcReply::Json(json!({"result":"OpaqueRef:network-created"}))
                });
            }
            if method == "network.destroy" {
                records.lock().unwrap().remove("network");
                return Some(RpcReply::Json(json!({"result":null})));
            }
            if method == "host.enable" || method == "host.disable" {
                records.lock().unwrap().get_mut("host").unwrap()["enabled"] =
                    json!(method == "host.enable");
                return Some(RpcReply::Json(json!({"result":null})));
            }
            if let Some((class, field)) = method.split_once(".set_")
                && ["pool", "host", "network", "SR"].contains(&class)
            {
                if fault == Fault::Description && field == "name_description" {
                    return Some(RpcReply::Json(
                        json!({"error":{"code":1,"message":"RBAC_PERMISSION_DENIED","data":["do-not-disclose"]}}),
                    ));
                }
                records.lock().unwrap().get_mut(class).unwrap()[field] =
                    request["params"][2].clone();
                mutations.fetch_add(1, Ordering::SeqCst);
                return Some(if fault == Fault::DropRename {
                    RpcReply::Drop
                } else {
                    RpcReply::Json(json!({"result":null}))
                });
            }
            if method == "Async.SR.scan" {
                return Some(RpcReply::Json(json!({"result":"OpaqueRef:resource-task"})));
            }
            if method == "task.get_record" && fault == Fault::PendingTask {
                return Some(RpcReply::Json(
                    json!({"result":{"status":"pending","progress":"0.25","allowed_operations":["cancel"]}}),
                ));
            }
            None
        }
    };
    let mock = mock_overridden(override_rpc(callback)).await;
    (mock, records, gate)
}
fn pool() -> ResourceTarget {
    ResourceTarget::Pool(PoolId::parse(POOL).unwrap())
}
fn network() -> ResourceTarget {
    ResourceTarget::Network(NetworkId::parse(NETWORK).unwrap())
}
fn storage() -> ResourceTarget {
    ResourceTarget::Storage(SrId::parse(STORAGE).unwrap())
}
async fn connection(mock: &Mock, backend: &Backend, read_only: bool) -> xenadmin_core::Connection {
    let mut options = ConnectionOptions::new(mock.endpoint.clone());
    options.read_only = read_only;
    let connection = backend.connect(
        options,
        Credentials::new("root".into(), "fixture-password".into()),
    );
    connection.wait_ready(deadline(3)).await.unwrap();
    connection
}
async fn finish_resource(operation: &ResourceOperation) -> Arc<ResourceOperationRecord> {
    tokio::time::timeout(deadline(4), async {
        loop {
            let record = operation.latest();
            if record.state.terminal() || record.state == OperationState::OutcomeUnknown {
                return record;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}
fn rename(
    connection: &xenadmin_core::Connection,
    name: &str,
    description: &str,
) -> ResourceCommand {
    ResourceCommand::RenameMetadata {
        target: connection.latest().observe_resource(network()).unwrap(),
        expected_name: "Internal network".into(),
        expected_description: "Network description".into(),
        name: name.into(),
        description: description.into(),
    }
}
async fn rechecked(
    connection: &xenadmin_core::Connection,
    id: OperationId,
) -> Arc<Vec<ResourceOperationRecord>> {
    connection
        .control_resource(id, OperationControl::Recheck)
        .unwrap();
    tokio::time::timeout(deadline(3), async {
        loop {
            let rows = connection.resource_history();
            if rows.iter().any(|r| r.id == id && r.state.terminal()) {
                return rows;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}
#[tokio::test]
async fn metadata_edits_report_verified_and_partial_steps() {
    for fault in [Fault::None, Fault::Description] {
        let (mock, records) = fixture(fault).await;
        let directory = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(directory.path().into()).unwrap();
        let connection = connection(&mock, &backend, false).await;
        let operation = connection
            .submit_resource(
                OperationId::new(),
                rename(&connection, "Renamed", "Changed description"),
            )
            .unwrap();
        let result = finish_resource(&operation).await;
        assert_eq!(
            result.state,
            if fault == Fault::None {
                OperationState::Succeeded
            } else {
                OperationState::PartiallyCompleted
            }
        );
        assert_eq!(result.steps[0].state, xenadmin_core::StepState::Verified);
        assert_eq!(records.lock().unwrap()["network"]["name_label"], "Renamed");
        assert!(!result.detail.contains("do-not-disclose"));
        connection.close().await;
    }
}
#[tokio::test]
async fn lost_metadata_reply_reserves_pool_and_reconciles_without_replay() {
    let (mock, _) = fixture(Fault::DropRename).await;
    let directory = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(directory.path().into()).unwrap();
    let connection = connection(&mock, &backend, false).await;
    let id = OperationId::new();
    let operation = connection
        .submit_resource(id, rename(&connection, "Renamed", "Network description"))
        .unwrap();
    assert_eq!(
        finish_resource(&operation).await.state,
        OperationState::OutcomeUnknown
    );
    let next = connection
        .submit_resource(
            OperationId::new(),
            rename(&connection, "Other", "Network description"),
        )
        .unwrap();
    assert_eq!(finish_resource(&next).await.state, OperationState::Rejected);
    let vm = connection
        .latest()
        .inventory()
        .unwrap()
        .guests()
        .next()
        .unwrap()
        .id;
    let vm_op = connection
        .submit(
            OperationId::new(),
            VmCommand::Power {
                vm: connection.latest().observe_vm(vm).unwrap(),
                action: PowerAction::Start,
            },
        )
        .unwrap();
    assert_eq!(super::finish(&vm_op).await, OperationState::Rejected);
    assert_eq!(
        rechecked(&connection, id)
            .await
            .iter()
            .find(|r| r.id == id)
            .unwrap()
            .state,
        OperationState::Succeeded
    );
    assert_eq!(
        mock.methods
            .lock()
            .unwrap()
            .iter()
            .filter(|m| m.as_str() == "network.set_name_label")
            .count(),
        1
    );
    connection.close().await;
}
#[tokio::test]
async fn lost_create_reply_recovers_the_new_identity_using_the_saved_marker() {
    let (mock, records) = fixture(Fault::DropCreate).await;
    let directory = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(directory.path().into()).unwrap();
    let connection = connection(&mock, &backend, false).await;
    let id = OperationId::new();
    let operation = connection
        .submit_resource(
            id,
            ResourceCommand::CreateInternalNetwork {
                pool: connection.latest().observe_resource(pool()).unwrap(),
                name: "New network".into(),
                description: "Test".into(),
                mtu: 9000,
            },
        )
        .unwrap();
    assert_eq!(
        finish_resource(&operation).await.state,
        OperationState::OutcomeUnknown
    );
    let rows = rechecked(&connection, id).await;
    assert_eq!(
        rows.iter().find(|r| r.id == id).unwrap().created,
        Some(ResourceTarget::Network(NetworkId::parse(CREATED).unwrap()))
    );
    assert_eq!(
        records.lock().unwrap()["network-created"]["other_config"]["xenadmin-rs:operation"],
        id.to_string()
    );
    assert_eq!(
        mock.methods
            .lock()
            .unwrap()
            .iter()
            .filter(|m| m.as_str() == "network.create")
            .count(),
        1
    );
    connection.close().await;
}
#[tokio::test]
async fn fresh_network_checks_prevent_attached_or_changed_network_mutations() {
    for fault in [
        Fault::AttachedPif,
        Fault::AttachedVif,
        Fault::MissingInterfaces,
        Fault::ChangedMtu,
    ] {
        let (mock, _) = fixture(fault).await;
        let directory = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(directory.path().into()).unwrap();
        let connection = connection(&mock, &backend, false).await;
        let operation = connection
            .submit_resource(
                OperationId::new(),
                ResourceCommand::ConfigureInternalNetwork {
                    network: connection.latest().observe_resource(network()).unwrap(),
                    expected_mtu: 1500,
                    mtu: 1600,
                },
            )
            .unwrap();
        assert_eq!(
            finish_resource(&operation).await.state,
            OperationState::Rejected
        );
        if fault != Fault::ChangedMtu {
            let operation = connection
                .submit_resource(
                    OperationId::new(),
                    ResourceCommand::DeleteInternalNetwork {
                        network: connection.latest().observe_resource(network()).unwrap(),
                        expected_name: "Internal network".into(),
                    },
                )
                .unwrap();
            assert_eq!(
                finish_resource(&operation).await.state,
                OperationState::Rejected
            );
        }
        assert!(
            !mock
                .methods
                .lock()
                .unwrap()
                .iter()
                .any(|m| m == "network.set_MTU" || m == "network.destroy")
        );
        connection.close().await;
    }
}
#[tokio::test]
async fn post_write_read_failure_is_unknown_instead_of_rejected() {
    let (mock, _) = fixture(Fault::VerifyRead).await;
    let directory = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(directory.path().into()).unwrap();
    let connection = connection(&mock, &backend, false).await;
    let operation = connection
        .submit_resource(
            OperationId::new(),
            rename(&connection, "Renamed", "Network description"),
        )
        .unwrap();
    let result = finish_resource(&operation).await;
    assert_eq!(result.state, OperationState::OutcomeUnknown);
    assert_eq!(
        result.steps[0].state,
        xenadmin_core::StepState::OutcomeUnknown
    );
    connection.close().await;
}
#[tokio::test]
async fn host_storage_pool_and_network_commands_use_typed_targets() {
    let (mock, records) = fixture(Fault::None).await;
    let directory = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(directory.path().into()).unwrap();
    let connection = connection(&mock, &backend, false).await;
    let snapshot = connection.latest();
    assert!(
        snapshot
            .inventory()
            .unwrap()
            .details
            .pool
            .as_ref()
            .unwrap()
            .default_sr_known
    );
    let commands = [
        ResourceCommand::SetHostEnabled {
            host: snapshot
                .observe_resource(ResourceTarget::Host(HostId::parse(HOST).unwrap()))
                .unwrap(),
            expected_enabled: true,
            enabled: false,
        },
        ResourceCommand::SetDefaultStorage {
            pool: snapshot.observe_resource(pool()).unwrap(),
            sr: SrId::parse(STORAGE).unwrap(),
            expected_default: None,
        },
        ResourceCommand::ScanStorage {
            sr: snapshot.observe_resource(storage()).unwrap(),
        },
        ResourceCommand::ConfigureInternalNetwork {
            network: snapshot.observe_resource(network()).unwrap(),
            expected_mtu: 1500,
            mtu: 9000,
        },
        ResourceCommand::DeleteInternalNetwork {
            network: snapshot.observe_resource(network()).unwrap(),
            expected_name: "Internal network".into(),
        },
    ];
    for command in commands {
        let operation = connection
            .submit_resource(OperationId::new(), command)
            .unwrap();
        let result = finish_resource(&operation).await;
        assert_eq!(
            result.state,
            OperationState::Succeeded,
            "{}: {}",
            result.action.label(),
            result.detail
        );
    }
    assert_eq!(records.lock().unwrap()["host"]["enabled"], false);
    assert_eq!(
        records.lock().unwrap()["pool"]["default_SR"],
        "OpaqueRef:SR"
    );
    assert!(!records.lock().unwrap().contains_key("network"));
    connection.close().await;
}
#[tokio::test]
async fn resource_observations_and_read_only_guards_prevent_dispatch() {
    let (mock, _) = fixture(Fault::None).await;
    let directory = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(directory.path().into()).unwrap();
    let connection = connection(&mock, &backend, true).await;
    assert!(matches!(
        connection.submit_resource(
            OperationId::new(),
            rename(&connection, "Renamed", "Network description")
        ),
        Err(xenadmin_core::Error::ReadOnly)
    ));
    assert!(matches!(
        connection.control_resource(OperationId::new(), OperationControl::Recheck),
        Err(xenadmin_core::Error::ReadOnly)
    ));
    assert!(
        connection
            .latest()
            .observe_resource(ResourceTarget::Network(NetworkId::from(
                uuid::Uuid::new_v4()
            )))
            .is_err()
    );
    assert!(!directory.path().join("operations.sqlite3").exists());
    connection.close().await;
}
#[tokio::test]
async fn scan_task_survives_disconnect_and_keeps_the_pool_reserved() {
    let (mock, _) = fixture(Fault::PendingTask).await;
    let directory = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(directory.path().into()).unwrap();
    let first = connection(&mock, &backend, false).await;
    let id = OperationId::new();
    let operation = first
        .submit_resource(
            id,
            ResourceCommand::ScanStorage {
                sr: first.latest().observe_resource(storage()).unwrap(),
            },
        )
        .unwrap();
    tokio::time::timeout(deadline(3), async {
        while operation.latest().progress != Some(0.25) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    first.close().await;
    assert_eq!(
        finish_resource(&operation).await.state,
        OperationState::OutcomeUnknown
    );
    let second = connection(&mock, &backend, false).await;
    tokio::time::timeout(deadline(3), async {
        while !second
            .resource_history()
            .iter()
            .any(|r| r.id == id && r.progress == Some(0.25))
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let blocked = second
        .submit_resource(
            OperationId::new(),
            rename(&second, "Renamed", "Network description"),
        )
        .unwrap();
    assert_eq!(
        finish_resource(&blocked).await.state,
        OperationState::Rejected
    );
    assert_eq!(
        mock.methods
            .lock()
            .unwrap()
            .iter()
            .filter(|m| m.as_str() == "Async.SR.scan")
            .count(),
        1
    );
    second.close().await;
}

#[tokio::test]
async fn cancellation_during_preflight_prevents_every_write() {
    let (mock, _) = fixture(Fault::SlowRead).await;
    let directory = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(directory.path().into()).unwrap();
    let connection = connection(&mock, &backend, false).await;
    let id = OperationId::new();
    let operation = connection
        .submit_resource(id, rename(&connection, "Renamed", "Changed description"))
        .unwrap();
    tokio::time::timeout(deadline(2), async {
        while operation.latest().revision() == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    connection
        .control_resource(id, OperationControl::RequestCancellation)
        .unwrap();
    assert_eq!(
        finish_resource(&operation).await.state,
        OperationState::Cancelled
    );
    assert!(
        !mock
            .methods
            .lock()
            .unwrap()
            .iter()
            .any(|method| method.starts_with("network.set_"))
    );
    connection.close().await;
}
#[tokio::test]
async fn reconciliation_of_partial_metadata_change_never_submits_the_remaining_step() {
    let (mock, records) = fixture(Fault::DropRename).await;
    let directory = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(directory.path().into()).unwrap();
    let connection = connection(&mock, &backend, false).await;
    let id = OperationId::new();
    let operation = connection
        .submit_resource(id, rename(&connection, "Renamed", "Changed description"))
        .unwrap();
    assert_eq!(
        finish_resource(&operation).await.state,
        OperationState::OutcomeUnknown
    );
    let rows = rechecked(&connection, id).await;
    let record = rows.iter().find(|r| r.id == id).unwrap();
    assert_eq!(record.state, OperationState::PartiallyCompleted);
    assert_eq!(record.steps[1].state, xenadmin_core::StepState::Pending);
    assert_eq!(
        records.lock().unwrap()["network"]["name_description"],
        "Network description"
    );
    assert!(
        !mock
            .methods
            .lock()
            .unwrap()
            .iter()
            .any(|method| method == "network.set_name_description")
    );
    connection.close().await;
}
#[tokio::test]
async fn creation_reconciliation_cannot_adopt_a_resource_from_the_preexisting_baseline() {
    let (mock, records) = fixture(Fault::DropCreate).await;
    let directory = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(directory.path().into()).unwrap();
    let connection = connection(&mock, &backend, false).await;
    let id = OperationId::new();
    let operation = connection
        .submit_resource(
            id,
            ResourceCommand::CreateInternalNetwork {
                pool: connection.latest().observe_resource(pool()).unwrap(),
                name: "New network".into(),
                description: "Test".into(),
                mtu: 1500,
            },
        )
        .unwrap();
    let initial = finish_resource(&operation).await;
    assert_eq!(initial.state, OperationState::OutcomeUnknown);
    records.lock().unwrap().get_mut("network-created").unwrap()["uuid"] = json!(NETWORK);
    connection
        .control_resource(id, OperationControl::Recheck)
        .unwrap();
    tokio::time::timeout(deadline(3), async {
        loop {
            if let Some(record) = connection.resource_history().iter().find(|r| {
                r.id == id
                    && r.revision() > initial.revision()
                    && (r.state.terminal() || r.state == OperationState::OutcomeUnknown)
            }) {
                assert_eq!(record.state, OperationState::OutcomeUnknown);
                assert!(record.created.is_none());
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
            .filter(|m| m.as_str() == "network.create")
            .count(),
        1
    );
    connection.close().await;
}
#[tokio::test]
async fn same_pool_alias_routes_resource_commands_and_history_to_one_owner() {
    let (mock, _) = fixture(Fault::None).await;
    let directory = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(directory.path().into()).unwrap();
    let first = connection(&mock, &backend, false).await;
    let second = connection(&mock, &backend, false).await;
    assert_eq!(first.latest().generation(), second.latest().generation());
    let id = OperationId::new();
    let operation = second
        .submit_resource(id, rename(&second, "Renamed", "Network description"))
        .unwrap();
    assert_eq!(
        finish_resource(&operation).await.state,
        OperationState::Succeeded
    );
    tokio::time::timeout(deadline(2), async {
        while !first
            .resource_history()
            .iter()
            .any(|r| r.id == id && r.state == OperationState::Succeeded)
            || !second
                .resource_history()
                .iter()
                .any(|r| r.id == id && r.state == OperationState::Succeeded)
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let observed = first.latest().observe_resource(network()).unwrap();
    first.close().await;
    let third = connection(&mock, &backend, false).await;
    assert!(matches!(
        third.submit_resource(
            OperationId::new(),
            ResourceCommand::RenameMetadata {
                target: observed,
                expected_name: "Renamed".into(),
                expected_description: "Network description".into(),
                name: "Next".into(),
                description: "Network description".into()
            }
        ),
        Err(xenadmin_core::Error::Stale)
    ));
    third.close().await;
}
#[tokio::test]
async fn default_storage_requires_positive_disk_repository_identification() {
    for content_type in [json!("iso"), Value::Null, json!("unknown")] {
        let (mock, records) = fixture(Fault::None).await;
        let directory = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(directory.path().into()).unwrap();
        let connection = connection(&mock, &backend, false).await;
        records.lock().unwrap().get_mut("SR").unwrap()["content_type"] = content_type;
        let operation = connection
            .submit_resource(
                OperationId::new(),
                ResourceCommand::SetDefaultStorage {
                    pool: connection.latest().observe_resource(pool()).unwrap(),
                    sr: SrId::parse(STORAGE).unwrap(),
                    expected_default: None,
                },
            )
            .unwrap();
        assert_eq!(
            finish_resource(&operation).await.state,
            OperationState::Rejected
        );
        assert!(
            !mock
                .methods
                .lock()
                .unwrap()
                .iter()
                .any(|method| method == "pool.set_default_SR")
        );
        connection.close().await;
    }
}

#[tokio::test]
async fn bulk_power_refuses_read_only_and_previous_connection_generation_before_admission() {
    let (mock, _) = fixture(Fault::None).await;
    let directory = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(directory.path().into()).unwrap();
    let first = connection(&mock, &backend, true).await;
    let pool = first
        .latest()
        .observe_resource(ResourceTarget::Pool(PoolId::parse(POOL).unwrap()))
        .unwrap();
    let command = || ResourceCommand::PowerBatch {
        pool,
        selection: xenadmin_core::VmPowerBatch {
            action: PowerAction::Start,
            vms: vec![xenadmin_core::BatchVmPower {
                vm: uuid::Uuid::from_u128(2).into(),
                expected_state: PowerState::Halted,
            }],
        },
    };
    assert!(matches!(
        first.submit_resource(OperationId::new(), command()),
        Err(xenadmin_core::Error::ReadOnly)
    ));
    first.close().await;
    let second = connection(&mock, &backend, false).await;
    assert!(matches!(
        second.submit_resource(OperationId::new(), command()),
        Err(xenadmin_core::Error::Stale)
    ));
    assert!(second.resource_history().is_empty());
    assert!(
        !mock
            .methods
            .lock()
            .unwrap()
            .iter()
            .any(|m| m.starts_with("Async.VM."))
    );
    second.close().await;
}

#[tokio::test]
async fn explicit_recheck_publishes_verifying_until_final_read_finishes() {
    let (mock, _, gate) = gated_fixture(Fault::DropRename).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let connection = connection(&mock, &backend, false).await;
    let id = OperationId::new();
    let operation = connection
        .submit_resource(id, rename(&connection, "Renamed", "Network description"))
        .unwrap();
    let original = finish_resource(&operation).await;
    assert_eq!(original.state, OperationState::OutcomeUnknown);
    gate.enabled.store(true, Ordering::SeqCst);
    connection
        .control_resource(id, OperationControl::Recheck)
        .unwrap();
    tokio::time::timeout(deadline(4), gate.entered.notified())
        .await
        .unwrap();
    let db = rusqlite::Connection::open(dir.path().join("operations.sqlite3")).unwrap();
    let state: String = db
        .query_row(
            "SELECT payload FROM resource_operations WHERE id=?1",
            [id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&state).unwrap()["state"],
        "Verifying"
    );
    tokio::time::timeout(deadline(4), async {
        loop {
            if let Some(row) = connection
                .resource_history()
                .iter()
                .find(|row| row.id == id && row.revision() > original.revision())
            {
                assert_eq!(row.state, OperationState::Verifying, "{}", row.detail);
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    gate.release.notify_one();
    tokio::time::timeout(deadline(4), async {
        loop {
            if let Some(row) = connection
                .resource_history()
                .iter()
                .find(|row| row.id == id && row.state.terminal())
            {
                assert_eq!(row.state, OperationState::Succeeded, "{}", row.detail);
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        mock.methods
            .lock()
            .unwrap()
            .iter()
            .filter(|method| method.as_str() == "network.set_name_label")
            .count(),
        1
    );
    connection.close().await;
}
