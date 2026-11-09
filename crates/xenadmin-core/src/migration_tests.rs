use super::*;
use crate::xapi::inventory_tests::Fixture;
use crate::{DiskPlacement, NicPlacement, VmMigration};
use serde_json::Value;
use std::{collections::BTreeMap, sync::Mutex};
type Records = Arc<Mutex<BTreeMap<String, Value>>>;
type Calls = Arc<Mutex<Vec<Value>>>;
#[path = "migration_evidence_tests.rs"]
mod evidence;
fn uuid(n: u128) -> Uuid {
    Uuid::from_u128(n)
}
fn vm() -> VmId {
    uuid(2).into()
}
fn operation(live: bool, storage: bool) -> VmMigration {
    VmMigration {
        destination: uuid(4).into(),
        expected_host: live.then(|| uuid(3).into()),
        expected_affinity: Some(uuid(3).into()),
        live,
        disks: vec![DiskPlacement {
            vdi: uuid(6).into(),
            expected_sr: uuid(5).into(),
            destination_sr: uuid(if storage { 15 } else { 5 }).into(),
        }],
        networks: vec![NicPlacement {
            vif: uuid(8).into(),
            expected_network: uuid(9).into(),
            destination_network: uuid(9).into(),
        }],
        transfer_network: Some(uuid(9).into()),
    }
}
#[derive(Clone, Copy, PartialEq)]
enum Fault {
    None,
    Refuse,
    LostReply,
    PollMissing,
    TaskRemovedAfterCompletion,
    Failure,
    Cancelled,
    WrongDisk,
    WrongVm,
    PartialDisk,
    AffinityLost,
    AffinityRefuse,
    ChangedDuringAssert,
    WrongToken,
    Incompatible,
}
type Hook = Arc<dyn Fn(&Value) + Send + Sync>;
async fn fixture(live: bool, storage: bool, fault: Fault) -> (Fixture, Records, Calls) {
    fixture_hook(live, storage, fault, None).await
}
async fn fixture_hook(
    live: bool,
    storage: bool,
    fault: Fault,
    hook: Option<Hook>,
) -> (Fixture, Records, Calls) {
    let data = BTreeMap::from([
        (
            "OpaqueRef:VM:source".into(),
            json!({"uuid":uuid(2),"is_a_template":false,"is_a_snapshot":false,"is_control_domain":false,"power_state":if live{"Running"}else{"Halted"},"resident_on":if live{"OpaqueRef:host:source"}else{"OpaqueRef:NULL"},"affinity":"OpaqueRef:host:source","VBDs":["OpaqueRef:VBD:source"],"VIFs":["OpaqueRef:VIF:source"],"snapshots":[],"allowed_operations":["migrate_send","pool_migrate"],"blocked_operations":{},"current_operations":{}}),
        ),
        (
            "OpaqueRef:host:source".into(),
            json!({"uuid":uuid(3),"enabled":true,"metrics":"OpaqueRef:host_metrics:source","current_operations":{}}),
        ),
        (
            "OpaqueRef:host:destination".into(),
            json!({"uuid":uuid(4),"enabled":true,"metrics":"OpaqueRef:host_metrics:destination","current_operations":{}}),
        ),
        (
            "OpaqueRef:host_metrics:source".into(),
            json!({"uuid":uuid(30),"live":true}),
        ),
        (
            "OpaqueRef:host_metrics:destination".into(),
            json!({"uuid":uuid(40),"live":true}),
        ),
        (
            "OpaqueRef:SR:source".into(),
            json!({"uuid":uuid(5),"content_type":"user","allowed_operations":["vdi_create"],"PBDs":["OpaqueRef:PBD:source"],"current_operations":{}}),
        ),
        (
            "OpaqueRef:SR:destination".into(),
            json!({"uuid":uuid(15),"content_type":"user","allowed_operations":["vdi_create"],"PBDs":["OpaqueRef:PBD:destination"],"current_operations":{}}),
        ),
        (
            "OpaqueRef:PBD:source".into(),
            json!({"uuid":uuid(50),"host":"OpaqueRef:host:destination","SR":"OpaqueRef:SR:source","currently_attached":true}),
        ),
        (
            "OpaqueRef:PBD:destination".into(),
            json!({"uuid":uuid(150),"host":"OpaqueRef:host:destination","SR":"OpaqueRef:SR:destination","currently_attached":true}),
        ),
        (
            "OpaqueRef:VDI:source".into(),
            json!({"uuid":uuid(6),"SR":"OpaqueRef:SR:source","virtual_size":"1073741824","type":"user","missing":false,"managed":true,"read_only":false,"sharable":false,"is_a_snapshot":false,"cbt_enabled":false,"VBDs":["OpaqueRef:VBD:source"],"current_operations":{}}),
        ),
        (
            "OpaqueRef:VDI:unrelated".into(),
            json!({"uuid":uuid(99),"SR":"OpaqueRef:SR:destination","virtual_size":"1073741824","type":"user","missing":false,"managed":true,"read_only":false,"sharable":false,"is_a_snapshot":false,"cbt_enabled":false,"VBDs":["OpaqueRef:VBD:source"],"current_operations":{}}),
        ),
        (
            "OpaqueRef:VBD:source".into(),
            json!({"uuid":uuid(7),"VM":"OpaqueRef:VM:source","VDI":"OpaqueRef:VDI:source","empty":false,"type":"Disk","mode":"RW","userdevice":"0","bootable":true,"other_config":{"owner":"true"},"current_operations":{}}),
        ),
        (
            "OpaqueRef:VIF:source".into(),
            json!({"uuid":uuid(8),"VM":"OpaqueRef:VM:source","network":"OpaqueRef:network:source","device":"0","MAC":"02:00:00:00:00:08","current_operations":{}}),
        ),
        (
            "OpaqueRef:network:source".into(),
            json!({"uuid":uuid(9),"PIFs":["OpaqueRef:PIF:source"],"current_operations":{}}),
        ),
        (
            "OpaqueRef:PIF:source".into(),
            json!({"uuid":uuid(90),"network":"OpaqueRef:network:source","host":"OpaqueRef:host:destination","currently_attached":true,"IP":"192.0.2.4"}),
        ),
        (
            "OpaqueRef:network:destination".into(),
            json!({"uuid":uuid(19),"PIFs":[],"current_operations":{}}),
        ),
    ]);
    let records: Records = Arc::new(Mutex::new(data));
    let calls: Calls = Arc::new(Mutex::new(vec![]));
    let handler = {
        let records = records.clone();
        let calls = calls.clone();
        move |request: Value| {
            calls.lock().unwrap().push(request.clone());
            if let Some(hook) = &hook {
                hook(&request);
            }
            let method = request["method"].as_str().unwrap();
            let p = &request["params"];
            let mut r = records.lock().unwrap();
            if let Some(class) = method.strip_suffix(".get_all_records") {
                return json!({"result":r.iter().filter(|(k,_)|k.starts_with(&format!("OpaqueRef:{class}:"))).collect::<BTreeMap<_,_>>()});
            }
            if let Some(class) = method.strip_suffix(".get_by_uuid") {
                return r
                    .iter()
                    .find(|(k, v)| {
                        k.starts_with(&format!("OpaqueRef:{class}:")) && v["uuid"] == p[1]
                    })
                    .map(|(k, _)| json!({"result":k}))
                    .unwrap_or_else(|| json!({"error":{"message":"UUID_INVALID"}}));
            }
            if method == "host.migrate_receive" {
                return json!({"result":{"host":if fault==Fault::WrongToken{"OpaqueRef:host:source"}else{"OpaqueRef:host:destination"},"session_id":"SECRET-MIGRATION-TOKEN","master":"https://fixture.invalid"}});
            }
            if matches!(method, "VM.assert_can_migrate" | "VM.assert_can_boot_here") {
                if fault == Fault::Incompatible {
                    return json!({"error":{"message":"HOST_NOT_ENOUGH_FREE_MEMORY"}});
                }
                if fault == Fault::ChangedDuringAssert {
                    r.get_mut("OpaqueRef:VIF:source").unwrap()["MAC"] = json!("02:00:00:00:00:09");
                }
                return json!({"result":null});
            }
            if method.starts_with("Async.VM.") {
                if fault == Fault::Refuse {
                    return json!({"error":{"message":"VM_BAD_POWER_STATE"}});
                }
                if !matches!(fault, Fault::Failure | Fault::Cancelled) {
                    if live && fault != Fault::PartialDisk {
                        r.get_mut("OpaqueRef:VM:source").unwrap()["resident_on"] =
                            json!("OpaqueRef:host:destination");
                    }
                    if storage {
                        let mut d = r["OpaqueRef:VDI:source"].clone();
                        d["uuid"] = json!(uuid(16));
                        d["SR"] = json!("OpaqueRef:SR:destination");
                        r.insert("OpaqueRef:VDI:new".into(), d);
                        r.get_mut("OpaqueRef:VBD:source").unwrap()["VDI"] =
                            json!(if fault == Fault::WrongDisk {
                                "OpaqueRef:VDI:unrelated"
                            } else {
                                "OpaqueRef:VDI:new"
                            });
                    }
                    if let Some(map) = p.get(5).and_then(Value::as_object) {
                        for (vif, network) in map {
                            if let Some(v) = r.get_mut(vif) {
                                v["network"] = network.clone();
                            }
                        }
                    }
                }
                if fault == Fault::LostReply {
                    return json!({"_fixture_close_without_reply":true});
                }
                return json!({"result":"OpaqueRef:task:migration"});
            }
            if method == "task.get_record" {
                if fault == Fault::PollMissing
                    || (fault == Fault::TaskRemovedAfterCompletion
                        && calls
                            .lock()
                            .unwrap()
                            .iter()
                            .filter(|r| r["method"] == "task.get_record")
                            .count()
                            > 1)
                {
                    return json!({"error":{"message":"HANDLE_INVALID"}});
                }
                let status = match fault {
                    Fault::Failure | Fault::PartialDisk => "failure",
                    Fault::Cancelled => "cancelled",
                    _ => "success",
                };
                return json!({"result":{"status":status,"progress":1.0,"allowed_operations":[],"error_info":["HOST_CANNOT_ATTACH_NETWORK"],"result":format!("<value><string>{}</string></value>",if fault==Fault::WrongVm{"OpaqueRef:VM:unrelated"}else{"OpaqueRef:VM:source"})}});
            }
            if method == "VM.set_affinity" {
                if fault == Fault::AffinityRefuse {
                    return json!({"error":{"message":"OPERATION_NOT_ALLOWED"}});
                }
                r.get_mut("OpaqueRef:VM:source").unwrap()["affinity"] = p[2].clone();
                if fault == Fault::AffinityLost {
                    return json!({"_fixture_close_without_reply":true});
                }
                return json!({"result":null});
            }
            if method == "task.destroy" {
                return json!({"result":null});
            }
            if fault == Fault::TaskRemovedAfterCompletion
                && method == "VM.get_record"
                && calls
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|r| r["method"] == "task.get_record")
                && !r.contains_key("fixture:verification_failed")
            {
                r.insert("fixture:verification_failed".into(), json!(true));
                return json!({"error":{"message":"HANDLE_INVALID"}});
            }
            if method.ends_with(".get_record") {
                return r
                    .get(p[1].as_str().unwrap())
                    .map(|v| json!({"result":v}))
                    .unwrap_or_else(|| json!({"error":{"message":"HANDLE_INVALID"}}));
            }
            panic!("unexpected migration method {method}")
        }
    };
    (Fixture::new(handler).await, records, calls)
}
async fn execute(f: &Fixture, dir: &Path, o: VmMigration) -> OperationRecord {
    execute_id(f, dir, o, OperationId::new()).await
}
async fn execute_id(f: &Fixture, dir: &Path, o: VmMigration, id: OperationId) -> OperationRecord {
    let (op, request) = channel(
        id,
        uuid(1).into(),
        vm(),
        1,
        OperationAction::Migration(o),
        tokio::runtime::Handle::current(),
    );
    let (tx, _rx) = mpsc::channel(256);
    tokio::time::timeout(
        Duration::from_secs(10),
        request.run(
            Arc::new(f.session().await),
            Some("OpaqueRef:VM:source".into()),
            dir.into(),
            CancellationToken::new(),
            tx,
        ),
    )
    .await
    .unwrap();
    (*op.latest()).clone()
}
async fn recheck(f: &Fixture, dir: &Path, r: OperationRecord) -> OperationRecord {
    let id = r.id;
    let (tx, _rx) = mpsc::channel(256);
    MutationRequest::tracking(r)
        .run(
            Arc::new(f.session().await),
            None,
            dir.into(),
            CancellationToken::new(),
            tx,
        )
        .await;
    Journal::open(dir).unwrap().load(id).unwrap().unwrap()
}
fn count(c: &Calls, method: &str) -> usize {
    c.lock()
        .unwrap()
        .iter()
        .filter(|r| r["method"] == method)
        .count()
}
#[tokio::test]
async fn running_shared_storage_migration_preserves_identity_and_snapshots() {
    let (f, r, c) = fixture(true, false, Fault::None).await;
    r.lock().unwrap().get_mut("OpaqueRef:VM:source").unwrap()["snapshots"] =
        json!(["OpaqueRef:VM:snapshot"]);
    let dir = tempfile::tempdir().unwrap();
    let result = execute(&f, dir.path(), operation(true, false)).await;
    assert_eq!(result.state, OperationState::Succeeded, "{}", result.detail);
    assert!(result.created_devices.is_empty());
    assert_eq!(count(&c, "Async.VM.pool_migrate"), 1);
    assert_eq!(count(&c, "host.migrate_receive"), 0);
    assert_eq!(count(&c, "VM.set_affinity"), 0);
}
#[tokio::test]
async fn live_storage_migration_reports_new_disk_and_keeps_credentials_out_of_journal() {
    let (f, _, c) = fixture(true, true, Fault::None).await;
    let dir = tempfile::tempdir().unwrap();
    let result = execute(&f, dir.path(), operation(true, true)).await;
    assert_eq!(result.state, OperationState::Succeeded, "{}", result.detail);
    assert_eq!(
        result.created_devices,
        vec![crate::CreatedDevice::Disk(uuid(16).into())]
    );
    assert_eq!(count(&c, "VM.assert_can_migrate"), 1);
    assert_eq!(count(&c, "Async.VM.migrate_send"), 1);
    let serialized = serde_json::to_string(&result).unwrap();
    assert!(!serialized.contains("SECRET-MIGRATION-TOKEN"));
    assert!(!serialized.contains("fixture.invalid"));
}
#[tokio::test]
async fn halted_storage_move_sets_home_host_without_starting_vm() {
    let (f, r, c) = fixture(false, true, Fault::None).await;
    let dir = tempfile::tempdir().unwrap();
    let result = execute(&f, dir.path(), operation(false, true)).await;
    assert_eq!(result.state, OperationState::Succeeded, "{}", result.detail);
    assert_eq!(count(&c, "VM.set_affinity"), 1);
    assert_eq!(
        r.lock().unwrap()["OpaqueRef:VM:source"]["power_state"],
        "Halted"
    );
    assert_eq!(result.steps.len(), 2);
}
/// XCP-ng 8.3.0 refuses any VIF entry in a same-pool migration map, so identity placements
/// are sent as an empty map and a real network change is refused before dispatch.
#[tokio::test]
async fn same_pool_nic_placement_sends_no_vif_map_and_refuses_network_changes() {
    let (f, r, c) = fixture(true, true, Fault::None).await;
    let dir = tempfile::tempdir().unwrap();
    let result = execute(&f, dir.path(), operation(true, true)).await;
    assert_eq!(result.state, OperationState::Succeeded, "{}", result.detail);
    let sent = c
        .lock()
        .unwrap()
        .iter()
        .find(|call| call["method"] == "Async.VM.migrate_send")
        .cloned()
        .expect("migration dispatched");
    assert_eq!(
        sent["params"][5],
        serde_json::json!({}),
        "VIF map must be empty"
    );
    assert!(
        sent["params"][4].as_object().is_some_and(|m| !m.is_empty()),
        "VDI map kept"
    );

    let (f, r2, c2) = fixture(true, true, Fault::None).await;
    let dir = tempfile::tempdir().unwrap();
    let mut o = operation(true, true);
    o.networks[0].destination_network = uuid(19).into();
    let result = execute(&f, dir.path(), o).await;
    assert_ne!(result.state, OperationState::Succeeded);
    assert_eq!(count(&c2, "Async.VM.migrate_send"), 0);
    assert_eq!(
        r2.lock().unwrap()["OpaqueRef:VIF:source"]["network"],
        r.lock().unwrap()["OpaqueRef:VIF:source"]["network"]
    );
}
#[tokio::test]
async fn refusal_is_definite_but_missing_task_is_unknown() {
    for (fault, state) in [
        (Fault::Refuse, OperationState::Failed),
        (Fault::PollMissing, OperationState::OutcomeUnknown),
        (Fault::Failure, OperationState::Failed),
        (Fault::Cancelled, OperationState::Cancelled),
    ] {
        let (f, _, c) = fixture(true, true, fault).await;
        let dir = tempfile::tempdir().unwrap();
        let result = execute(&f, dir.path(), operation(true, true)).await;
        assert_eq!(result.state, state, "{}", result.detail);
        assert_eq!(count(&c, "Async.VM.migrate_send"), 1);
    }
}
#[tokio::test]
async fn lost_reply_and_restart_never_replay_migration_or_affinity() {
    let (f, _, c) = fixture(false, true, Fault::LostReply).await;
    let dir = tempfile::tempdir().unwrap();
    let result = execute(&f, dir.path(), operation(false, true)).await;
    assert_eq!(result.state, OperationState::OutcomeUnknown);
    let recovered = recheck(&f, dir.path(), result).await;
    assert_eq!(recovered.state, OperationState::OutcomeUnknown);
    assert_eq!(count(&c, "Async.VM.migrate_send"), 1);
    assert_eq!(count(&c, "VM.set_affinity"), 0);
    assert_eq!(
        recovered.created_devices,
        vec![crate::CreatedDevice::Disk(uuid(16).into())]
    );
}
#[tokio::test]
async fn existing_disk_or_wrong_vm_result_is_never_claimed_as_success() {
    for fault in [Fault::WrongDisk, Fault::WrongVm] {
        let (f, _, c) = fixture(false, true, fault).await;
        let dir = tempfile::tempdir().unwrap();
        let result = execute(&f, dir.path(), operation(false, true)).await;
        assert_eq!(
            result.state,
            OperationState::OutcomeUnknown,
            "{}",
            result.detail
        );
        assert_eq!(count(&c, "VM.set_affinity"), 0);
        assert!(result.created_devices.is_empty());
    }
}
#[tokio::test]
async fn lost_affinity_reply_reconciles_without_another_write() {
    let (f, _, c) = fixture(false, true, Fault::AffinityLost).await;
    let dir = tempfile::tempdir().unwrap();
    let result = execute(&f, dir.path(), operation(false, true)).await;
    assert_eq!(result.state, OperationState::OutcomeUnknown);
    let result = recheck(&f, dir.path(), result).await;
    assert_eq!(result.state, OperationState::Succeeded, "{}", result.detail);
    assert_eq!(count(&c, "VM.set_affinity"), 1);
    assert_eq!(count(&c, "Async.VM.migrate_send"), 1);
}
#[tokio::test]
async fn fresh_topology_restrictions_refuse_before_migration() {
    for (reference, field, value) in [
        ("OpaqueRef:host:destination", "enabled", json!(false)),
        ("OpaqueRef:host_metrics:destination", "live", json!(false)),
        (
            "OpaqueRef:PBD:destination",
            "currently_attached",
            json!(false),
        ),
        ("OpaqueRef:PIF:source", "IP", json!("")),
        ("OpaqueRef:VDI:source", "sharable", json!(true)),
        ("OpaqueRef:VDI:source", "cbt_enabled", json!(true)),
        (
            "OpaqueRef:VM:source",
            "snapshots",
            json!(["OpaqueRef:VM:snapshot"]),
        ),
        (
            "OpaqueRef:VM:source",
            "blocked_operations",
            json!({"migrate_send":"policy"}),
        ),
    ] {
        let (f, r, c) = fixture(true, true, Fault::None).await;
        r.lock().unwrap().get_mut(reference).unwrap()[field] = value;
        let dir = tempfile::tempdir().unwrap();
        let result = execute(&f, dir.path(), operation(true, true)).await;
        assert_eq!(
            result.state,
            OperationState::Rejected,
            "{reference}.{field}: {}",
            result.detail
        );
        assert_eq!(count(&c, "Async.VM.migrate_send"), 0);
    }
}
#[tokio::test]
async fn changes_during_compatibility_check_and_bad_receiver_refuse_dispatch() {
    for fault in [
        Fault::ChangedDuringAssert,
        Fault::WrongToken,
        Fault::Incompatible,
    ] {
        let (f, _, c) = fixture(true, true, fault).await;
        let dir = tempfile::tempdir().unwrap();
        let result = execute(&f, dir.path(), operation(true, true)).await;
        assert_eq!(result.state, OperationState::Rejected, "{}", result.detail);
        assert_eq!(count(&c, "Async.VM.migrate_send"), 0);
    }
}
#[tokio::test]
async fn failed_task_with_changed_placement_stays_unknown() {
    let (f, _, c) = fixture(true, true, Fault::PartialDisk).await;
    let dir = tempfile::tempdir().unwrap();
    let result = execute(&f, dir.path(), operation(true, true)).await;
    assert_eq!(
        result.state,
        OperationState::OutcomeUnknown,
        "{}",
        result.detail
    );
    assert_eq!(count(&c, "Async.VM.migrate_send"), 1);
}
#[tokio::test]
async fn missing_reviewed_device_and_changed_home_host_refuse_dispatch() {
    let (f, r, c) = fixture(false, true, Fault::None).await;
    r.lock().unwrap().get_mut("OpaqueRef:VM:source").unwrap()["affinity"] =
        json!("OpaqueRef:host:destination");
    let dir = tempfile::tempdir().unwrap();
    let result = execute(&f, dir.path(), operation(false, true)).await;
    assert_eq!(result.state, OperationState::Rejected);
    assert_eq!(count(&c, "Async.VM.migrate_send"), 0);
    let (f, _, c) = fixture(true, true, Fault::None).await;
    let mut o = operation(true, true);
    o.networks.clear();
    let dir = tempfile::tempdir().unwrap();
    let result = execute(&f, dir.path(), o).await;
    assert_eq!(result.state, OperationState::Rejected);
    assert_eq!(count(&c, "Async.VM.migrate_send"), 0);
}
#[tokio::test]
async fn refused_home_host_change_retains_successful_move_as_partial() {
    let (f, _, c) = fixture(false, true, Fault::AffinityRefuse).await;
    let dir = tempfile::tempdir().unwrap();
    let result = execute(&f, dir.path(), operation(false, true)).await;
    assert_eq!(
        result.state,
        OperationState::PartiallyCompleted,
        "{}",
        result.detail
    );
    assert_eq!(result.steps[0].state, StepState::Verified);
    assert_eq!(count(&c, "VM.set_affinity"), 1);
    assert_eq!(
        result.created_devices,
        vec![crate::CreatedDevice::Disk(uuid(16).into())]
    );
}
#[tokio::test]
async fn partial_migration_keeps_verified_replacement_disk_identity() {
    let (f, _, c) = fixture(true, true, Fault::PartialDisk).await;
    let dir = tempfile::tempdir().unwrap();
    let result = execute(&f, dir.path(), operation(true, true)).await;
    assert_eq!(result.state, OperationState::OutcomeUnknown);
    assert_eq!(
        result.created_devices,
        vec![crate::CreatedDevice::Disk(uuid(16).into())]
    );
    let recovered = recheck(&f, dir.path(), result).await;
    assert_eq!(recovered.state, OperationState::OutcomeUnknown);
    assert_eq!(count(&c, "Async.VM.migrate_send"), 1);
    assert_eq!(
        recovered.created_devices,
        vec![crate::CreatedDevice::Disk(uuid(16).into())]
    );
}
#[test]
fn migration_action_journal_roundtrip_is_not_confused_with_other_workflows() {
    let action = OperationAction::Migration(operation(true, true));
    let restored: OperationAction =
        serde_json::from_str(&serde_json::to_string(&action).unwrap()).unwrap();
    assert_eq!(restored, action);
}
#[tokio::test]
async fn persisted_pool_migration_completion_reconciles_after_task_removal() {
    let (f, _, c) = fixture(true, false, Fault::TaskRemovedAfterCompletion).await;
    let dir = tempfile::tempdir().unwrap();
    let result = execute(&f, dir.path(), operation(true, false)).await;
    assert_eq!(result.state, OperationState::OutcomeUnknown);
    assert!(result.migration_plan.as_ref().unwrap().completed);
    assert_eq!(count(&c, "task.destroy"), 0);
    let result = recheck(&f, dir.path(), result).await;
    assert_eq!(result.state, OperationState::Succeeded, "{}", result.detail);
    assert_eq!(count(&c, "task.get_record"), 1);
    assert_eq!(count(&c, "Async.VM.pool_migrate"), 1);
    assert_eq!(
        count(&c, "task.destroy"),
        0,
        "explicit recheck is read-only"
    );
}
#[tokio::test]
async fn unresolved_migration_reserves_vm_resources_and_composes_with_creation_guard() {
    let (f, _, _) = fixture(true, true, Fault::LostReply).await;
    let dir = tempfile::tempdir().unwrap();
    let result = execute(&f, dir.path(), operation(true, true)).await;
    assert_eq!(result.state, OperationState::OutcomeUnknown);
    let mut journal = Journal::open(dir.path()).unwrap();
    let (_, request) = channel(
        OperationId::new(),
        uuid(1).into(),
        vm(),
        1,
        OperationAction::Power(crate::PowerAction::Start),
        tokio::runtime::Handle::current(),
    );
    let mut next = request.record;
    assert!(journal.admit(&mut next).is_err());
    let (_, request) = channel(
        OperationId::new(),
        uuid(1).into(),
        uuid(99).into(),
        1,
        OperationAction::Lifecycle(crate::VmLifecycleOperation::Clone {
            name: "new".into(),
            expected_template: false,
            mode: crate::VmCopyMode::Fast,
            as_template: false,
        }),
        tokio::runtime::Handle::current(),
    );
    let mut next = request.record;
    assert!(matches!(
        journal.admit(&mut next),
        Err(Error::PoolCreationConflict)
    ));
}
#[tokio::test]
async fn current_host_selection_moves_only_selected_disk_placement() {
    let (f, r, c) = fixture(true, true, Fault::None).await;
    r.lock().unwrap().get_mut("OpaqueRef:VM:source").unwrap()["resident_on"] =
        json!("OpaqueRef:host:destination");
    let mut o = operation(true, true);
    o.expected_host = Some(o.destination);
    let dir = tempfile::tempdir().unwrap();
    let result = execute(&f, dir.path(), o).await;
    assert_eq!(result.state, OperationState::Succeeded, "{}", result.detail);
    assert_eq!(
        result.created_devices,
        vec![crate::CreatedDevice::Disk(uuid(16).into())]
    );
    assert_eq!(count(&c, "Async.VM.pool_migrate"), 0);
    assert_eq!(count(&c, "Async.VM.migrate_send"), 1);
    assert_eq!(
        r.lock().unwrap()["OpaqueRef:VM:source"]["resident_on"],
        "OpaqueRef:host:destination"
    );
}
