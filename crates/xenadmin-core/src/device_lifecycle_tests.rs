use super::*;
use crate::xapi::inventory_tests::Fixture;
use crate::{CreatedDevice, DeviceOperation, NetworkId, SrId, VbdId, VdiId, VifId};
use serde_json::Value;
use std::{collections::BTreeMap, sync::Mutex};
type Records = Arc<Mutex<BTreeMap<String, Value>>>;
type Calls = Arc<Mutex<Vec<Value>>>;
fn uuid(n: u128) -> Uuid {
    Uuid::from_u128(n)
}
fn vm() -> VmId {
    uuid(2).into()
}
#[derive(Clone, Copy, PartialEq)]
enum Fault {
    None,
    LostDiskCreate,
    LostAttachmentCreate,
    RefuseAttachmentCreate,
    LostNicCreate,
    RefuseNicCreate,
    LostDiskDelete,
    RefuseDiskDelete,
    MisboundDiskCreate,
    UnexpectedAttachmentOwner,
    ReorderedNicConfig,
    LostProperty,
    RefuseSecondProperty,
    ChangePropertyAlgorithm,
}
fn normal_disk(n: u128) -> Value {
    json!({"uuid":uuid(n),"SR":"OpaqueRef:SR:base","name_label":"Existing disk","virtual_size":"1073741824","type":"user","read_only":false,"sharable":false,"managed":true,"missing":false,"is_a_snapshot":false,"VBDs":[],"current_operations":{},"allowed_operations":["destroy"],"other_config":{}})
}
async fn fixture(fault: Fault) -> (Fixture, Records, Calls) {
    let mut disk = normal_disk(4);
    disk["VBDs"] = json!(["OpaqueRef:VBD:disk"]);
    let records: Records = Arc::new(Mutex::new(BTreeMap::from([
        (
            "OpaqueRef:VM:guest".into(),
            json!({"uuid":uuid(2),"power_state":"Halted","is_a_snapshot":false,"is_a_template":false,"is_control_domain":false,"current_operations":{}}),
        ),
        (
            "OpaqueRef:SR:base".into(),
            json!({"uuid":uuid(3),"content_type":"user","current_operations":{},"allowed_operations":["vdi_create"]}),
        ),
        ("OpaqueRef:VDI:disk".into(), disk),
        (
            "OpaqueRef:VBD:disk".into(),
            json!({"uuid":uuid(5),"VM":"OpaqueRef:VM:guest","VDI":"OpaqueRef:VDI:disk","userdevice":"0","type":"Disk","mode":"RW","empty":false,"currently_attached":false,"current_operations":{},"other_config":{"owner":"true"}}),
        ),
        ("OpaqueRef:VDI:spare".into(), normal_disk(6)),
        (
            "OpaqueRef:network:source".into(),
            json!({"uuid":uuid(7),"MTU":"1500"}),
        ),
        (
            "OpaqueRef:network:destination".into(),
            json!({"uuid":uuid(8),"MTU":"1500"}),
        ),
        (
            "OpaqueRef:VIF:nic".into(),
            json!({"uuid":uuid(9),"VM":"OpaqueRef:VM:guest","network":"OpaqueRef:network:source","device":"0","MAC":"02:00:00:00:00:09","MTU":"1500","currently_attached":false,"current_operations":{},"other_config":{},"qos_algorithm_type":"ratelimit","qos_algorithm_params":{"kbps":"10000"},"locking_mode":"locked","ipv4_allowed":["192.0.2.9"],"ipv6_allowed":[]}),
        ),
    ])));
    let calls: Calls = Arc::new(Mutex::new(Vec::new()));
    let fixture=Fixture::new({let records=records.clone();let calls=calls.clone();move|request|{
        let method=request["method"].as_str().unwrap();calls.lock().unwrap().push(request.clone());
        let mut records=records.lock().unwrap();
        if let Some(class)=method.strip_suffix(".get_all_records") {return json!({"result":records.iter().filter(|(reference,_)|reference.starts_with(&format!("OpaqueRef:{class}:"))).collect::<BTreeMap<_,_>>()});}
        if let Some(class)=method.strip_suffix(".get_by_uuid") {return records.iter().find(|(reference,record)|reference.starts_with(&format!("OpaqueRef:{class}:"))&&record["uuid"]==request["params"][1]).map_or_else(||json!({"error":{"message":"UUID_INVALID"}}),|(reference,_)|json!({"result":reference}));}
        if method.ends_with(".get_record") {return records.get(request["params"][1].as_str().unwrap()).map_or_else(||json!({"error":{"message":"HANDLE_INVALID"}}),|record|json!({"result":record}));}
        if matches!(method,"VM.get_allowed_VBD_devices"|"VM.get_allowed_VIF_devices") {
            let (class,key)=if method.contains("VBD"){("VBD","userdevice")}else{("VIF","device")};
            return json!({"result":(0..16).map(|n|n.to_string()).filter(|slot|!records.iter().any(|(reference,record)|reference.starts_with(&format!("OpaqueRef:{class}:"))&&record["VM"]==request["params"][1]&&record[key]==*slot)).collect::<Vec<_>>()});
        }
        if let Some(class)=method.strip_suffix(".create") {
            if fault==Fault::MisboundDiskCreate&&class=="VDI"{return json!({"result":"OpaqueRef:VDI:spare"});}
            if (fault==Fault::RefuseAttachmentCreate&&class=="VBD")||(fault==Fault::RefuseNicCreate&&class=="VIF"){return json!({"error":{"message":"RBAC_PERMISSION_DENIED"}});}
            let number=match class{"VDI"=>201,"VBD"=>202,"VIF"=>203,_=>panic!("unexpected create")};
            let reference=format!("OpaqueRef:{class}:created");
            let mut record=request["params"][1].clone();
            record["uuid"]=json!(uuid(number));record["current_operations"]=json!({});
            // A disk has no attachment state of its own; its attachment and a NIC have.
            if class!="VDI"{record["currently_attached"]=json!(false);}
            match class{
                "VDI"=>{record["managed"]=json!(true);record["missing"]=json!(false);record["is_a_snapshot"]=json!(false);record["VBDs"]=json!([]);record["allowed_operations"]=json!(["destroy"]);},
                "VBD"=>{if record["VDI"]!="OpaqueRef:NULL"{records.get_mut(record["VDI"].as_str().unwrap()).unwrap()["VBDs"]=json!([&reference]);}},
                "VIF"=>{
                    if record["MAC"]==""{record["MAC"]=json!("02:00:00:00:00:ff");}
                    if fault==Fault::ReorderedNicConfig {record["ipv4_allowed"].as_array_mut().unwrap().reverse();}
                },
                _=>unreachable!(),
            }
            if class=="VBD"&&fault==Fault::UnexpectedAttachmentOwner {record["other_config"]["owner"]=json!("true");}
            records.insert(reference.clone(),record);
            return if (fault==Fault::LostDiskCreate&&class=="VDI")||(fault==Fault::LostAttachmentCreate&&class=="VBD")||(fault==Fault::LostNicCreate&&class=="VIF"){json!({"_fixture_close_without_reply":true})}else{json!({"result":reference})};
        }
        if let Some((class,field))=method.split_once(".set_") {
            assert!(["VDI","VBD","VIF"].contains(&class));
            let setters=calls.lock().unwrap().iter().filter(|r|r["method"].as_str().unwrap().contains(".set_")).count();
            if fault==Fault::RefuseSecondProperty && setters==2 {return json!({"error":{"message":"RBAC_PERMISSION_DENIED"}});}
            records.get_mut(request["params"][1].as_str().unwrap()).unwrap()[field]=request["params"][2].clone();
            if fault==Fault::ChangePropertyAlgorithm && field=="qos_algorithm_params" { records.get_mut(request["params"][1].as_str().unwrap()).unwrap()["qos_algorithm_type"]=json!(""); }
            return if fault==Fault::LostProperty{json!({"_fixture_close_without_reply":true})}else{json!({"result":null})};
        }
        match method{
            "VDI.disable_cbt"=>{records.get_mut(request["params"][1].as_str().unwrap()).unwrap()["cbt_enabled"]=json!(false);if fault==Fault::LostProperty{json!({"_fixture_close_without_reply":true})}else{json!({"result":null})}},
            "VBD.destroy"=>{
                let reference=request["params"][1].as_str().unwrap();
                let record=records.remove(reference).unwrap();
                records.get_mut(record["VDI"].as_str().unwrap()).unwrap()["VBDs"]=json!([]);
                json!({"result":null})
            },
            "VDI.destroy"=>{
                if fault==Fault::RefuseDiskDelete{return json!({"error":{"message":"SR_BACKEND_FAILURE"}});}
                records.remove(request["params"][1].as_str().unwrap());
                if fault==Fault::LostDiskDelete{json!({"_fixture_close_without_reply":true})}else{json!({"result":null})}
            },
            "VIF.destroy"=>{records.remove(request["params"][1].as_str().unwrap());json!({"result":null})},
            "VIF.move"=>{records.get_mut(request["params"][1].as_str().unwrap()).unwrap()["network"]=request["params"][2].clone();json!({"result":null})},
            _=>panic!("unexpected fixture method {method}"),
        }
    }}).await;
    (fixture, records, calls)
}
async fn execute(fixture: &Fixture, directory: &Path, action: DeviceOperation) -> OperationRecord {
    let (operation, request) = channel(
        OperationId::new(),
        uuid(1).into(),
        vm(),
        1,
        OperationAction::Device(action),
        tokio::runtime::Handle::current(),
    );
    let (tx, _rx) = mpsc::channel(128);
    tokio::time::timeout(
        Duration::from_secs(5),
        request.run(
            Arc::new(fixture.session().await),
            Some("OpaqueRef:VM:guest".into()),
            directory.into(),
            CancellationToken::new(),
            tx,
        ),
    )
    .await
    .unwrap();
    (*operation.latest()).clone()
}
async fn recheck(fixture: &Fixture, directory: &Path, record: OperationRecord) -> OperationRecord {
    let id = record.id;
    let (tx, _rx) = mpsc::channel(128);
    tokio::time::timeout(
        Duration::from_secs(5),
        MutationRequest::tracking(record).run(
            Arc::new(fixture.session().await),
            None,
            directory.into(),
            CancellationToken::new(),
            tx,
        ),
    )
    .await
    .unwrap();
    Journal::open(directory).unwrap().load(id).unwrap().unwrap()
}
fn count(calls: &Calls, method: &str) -> usize {
    calls
        .lock()
        .unwrap()
        .iter()
        .filter(|call| call["method"] == method)
        .count()
}
fn create_disk() -> DeviceOperation {
    DeviceOperation::CreateDisk {
        sr: SrId::from(uuid(3)),
        name: "Reviewed new disk".into(),
        size: 2 * 1024 * 1024 * 1024,
        device: "1".into(),
    }
}
fn delete_disk() -> DeviceOperation {
    DeviceOperation::DeleteDisk {
        vbd: VbdId::from(uuid(5)),
        vdi: VdiId::from(uuid(4)),
    }
}
fn edit_nic(address: &str) -> DeviceOperation {
    DeviceOperation::EditNic {
        vif: VifId::from(uuid(9)),
        expected_network: NetworkId::from(uuid(7)),
        expected_mac: "02:00:00:00:00:09".into(),
        network: NetworkId::from(uuid(8)),
        mac: address.into(),
    }
}

#[tokio::test]
async fn creates_disk_and_owned_attachment_with_verified_new_identities() {
    let (fixture, records, calls) = fixture(Fault::None).await;
    let directory = tempfile::tempdir().unwrap();
    let result = execute(&fixture, directory.path(), create_disk()).await;
    assert_eq!(result.state, OperationState::Succeeded, "{}", result.detail);
    assert_eq!(
        result.created_devices,
        vec![
            CreatedDevice::Disk(VdiId::from(uuid(201))),
            CreatedDevice::Attachment(VbdId::from(uuid(202)))
        ]
    );
    assert_eq!(
        records.lock().unwrap()["OpaqueRef:VBD:created"]["other_config"]["owner"],
        "true"
    );
    assert_eq!(count(&calls, "VDI.create"), 1);
    assert_eq!(count(&calls, "VBD.create"), 1);
}
#[tokio::test]
async fn existing_disk_attach_and_detach_preserve_the_disk_and_do_not_claim_ownership() {
    let (fixture, records, calls) = fixture(Fault::None).await;
    let directory = tempfile::tempdir().unwrap();
    let result = execute(
        &fixture,
        directory.path(),
        DeviceOperation::AttachDisk {
            vdi: VdiId::from(uuid(6)),
            device: "1".into(),
            read_only: true,
        },
    )
    .await;
    assert_eq!(result.state, OperationState::Succeeded, "{}", result.detail);
    assert!(
        records.lock().unwrap()["OpaqueRef:VBD:created"]["other_config"]
            .get("owner")
            .is_none()
    );
    assert_eq!(
        records.lock().unwrap()["OpaqueRef:VBD:created"]["mode"],
        "RO"
    );
    let result = execute(
        &fixture,
        directory.path(),
        DeviceOperation::DetachDisk {
            vbd: VbdId::from(uuid(202)),
            vdi: VdiId::from(uuid(6)),
        },
    )
    .await;
    assert_eq!(result.state, OperationState::Succeeded, "{}", result.detail);
    assert!(records.lock().unwrap().contains_key("OpaqueRef:VDI:spare"));
    assert_eq!(count(&calls, "VDI.destroy"), 0);
}
#[tokio::test]
async fn creates_empty_cd_drive_and_nic_without_mutating_existing_devices() {
    let (fixture, records, calls) = fixture(Fault::None).await;
    let directory = tempfile::tempdir().unwrap();
    let result = execute(
        &fixture,
        directory.path(),
        DeviceOperation::AddCdDrive { device: "3".into() },
    )
    .await;
    assert_eq!(result.state, OperationState::Succeeded, "{}", result.detail);
    assert_eq!(
        records.lock().unwrap()["OpaqueRef:VBD:created"]["empty"],
        true
    );
    let result = execute(
        &fixture,
        directory.path(),
        DeviceOperation::AddNic {
            network: NetworkId::from(uuid(8)),
            device: "1".into(),
            mac: None,
        },
    )
    .await;
    assert_eq!(result.state, OperationState::Succeeded, "{}", result.detail);
    assert_eq!(
        result.created_devices,
        vec![CreatedDevice::Nic(VifId::from(uuid(203)))]
    );
    assert!(records.lock().unwrap().contains_key("OpaqueRef:VIF:nic"));
    assert_eq!(count(&calls, "VIF.destroy"), 0);
}
#[tokio::test]
async fn deletes_only_selected_owned_exclusive_disk() {
    let (fixture, records, calls) = fixture(Fault::None).await;
    let directory = tempfile::tempdir().unwrap();
    let result = execute(&fixture, directory.path(), delete_disk()).await;
    assert_eq!(result.state, OperationState::Succeeded, "{}", result.detail);
    assert!(!records.lock().unwrap().contains_key("OpaqueRef:VDI:disk"));
    assert!(records.lock().unwrap().contains_key("OpaqueRef:VDI:spare"));
    assert_eq!(count(&calls, "VBD.destroy"), 1);
    assert_eq!(count(&calls, "VDI.destroy"), 1);
}
#[tokio::test]
async fn network_move_preserves_nic_identity_and_mac_replacement_preserves_settings() {
    for address in ["02:00:00:00:00:09", "02:00:00:00:00:0a"] {
        let (fixture, records, calls) = fixture(Fault::None).await;
        let directory = tempfile::tempdir().unwrap();
        let result = execute(&fixture, directory.path(), edit_nic(address)).await;
        assert_eq!(result.state, OperationState::Succeeded, "{}", result.detail);
        if address.ends_with("09") {
            assert_eq!(count(&calls, "VIF.destroy"), 0);
            assert_eq!(count(&calls, "VIF.move"), 1);
        } else {
            assert!(!records.lock().unwrap().contains_key("OpaqueRef:VIF:nic"));
            let records = records.lock().unwrap();
            let nic = &records["OpaqueRef:VIF:created"];
            assert_eq!(nic["qos_algorithm_params"]["kbps"], "10000");
            assert_eq!(nic["locking_mode"], "locked");
            assert_eq!(nic["ipv4_allowed"], json!(["192.0.2.9"]));
        }
    }
}
#[tokio::test]
async fn deleting_nic_removes_only_selected_interface() {
    let (fixture, records, calls) = fixture(Fault::None).await;
    let directory = tempfile::tempdir().unwrap();
    let result = execute(
        &fixture,
        directory.path(),
        DeviceOperation::DeleteNic {
            vif: VifId::from(uuid(9)),
        },
    )
    .await;
    assert_eq!(result.state, OperationState::Succeeded, "{}", result.detail);
    assert!(!records.lock().unwrap().contains_key("OpaqueRef:VIF:nic"));
    assert_eq!(count(&calls, "VIF.destroy"), 1);
}

#[tokio::test]
async fn lost_creation_replies_reconcile_owned_resources_without_repeating_writes() {
    for fault in [Fault::LostDiskCreate, Fault::LostAttachmentCreate] {
        let (fixture, records, calls) = fixture(fault).await;
        let directory = tempfile::tempdir().unwrap();
        let result = execute(&fixture, directory.path(), create_disk()).await;
        assert_eq!(
            result.state,
            OperationState::OutcomeUnknown,
            "{}",
            result.detail
        );
        let recovered = crate::operations::recover(directory.path(), uuid(1).into())
            .unwrap()
            .into_iter()
            .find(|record| record.id == result.id)
            .unwrap();
        let result = recheck(&fixture, directory.path(), recovered).await;
        assert_eq!(
            result.state,
            if fault == Fault::LostDiskCreate {
                OperationState::PartiallyCompleted
            } else {
                OperationState::Succeeded
            },
            "{}",
            result.detail
        );
        assert!(
            result
                .created_devices
                .contains(&CreatedDevice::Disk(VdiId::from(uuid(201))))
        );
        assert_eq!(count(&calls, "VDI.create"), 1);
        assert_eq!(
            count(&calls, "VBD.create"),
            usize::from(fault != Fault::LostDiskCreate)
        );
        assert!(
            records
                .lock()
                .unwrap()
                .contains_key("OpaqueRef:VDI:created")
        );
    }
}
#[tokio::test]
async fn failed_attachment_retains_new_disk_as_explicit_partial_result() {
    let (fixture, records, calls) = fixture(Fault::RefuseAttachmentCreate).await;
    let directory = tempfile::tempdir().unwrap();
    let result = execute(&fixture, directory.path(), create_disk()).await;
    assert_eq!(
        result.state,
        OperationState::PartiallyCompleted,
        "{}",
        result.detail
    );
    assert_eq!(
        result.created_devices,
        vec![CreatedDevice::Disk(VdiId::from(uuid(201)))]
    );
    assert!(
        records
            .lock()
            .unwrap()
            .contains_key("OpaqueRef:VDI:created")
    );
    assert_eq!(count(&calls, "VDI.destroy"), 0);
}
#[tokio::test]
async fn missing_reply_or_refusal_during_mac_replacement_has_no_automatic_rollback_or_replay() {
    for fault in [Fault::LostNicCreate, Fault::RefuseNicCreate] {
        let (fixture, records, calls) = fixture(fault).await;
        let directory = tempfile::tempdir().unwrap();
        let result = execute(&fixture, directory.path(), edit_nic("02:00:00:00:00:0a")).await;
        assert!(!records.lock().unwrap().contains_key("OpaqueRef:VIF:nic"));
        if fault == Fault::LostNicCreate {
            assert_eq!(
                result.state,
                OperationState::OutcomeUnknown,
                "{}",
                result.detail
            );
            let result = recheck(&fixture, directory.path(), result).await;
            assert_eq!(result.state, OperationState::Succeeded, "{}", result.detail);
            assert_eq!(
                result.created_devices,
                vec![CreatedDevice::Nic(VifId::from(uuid(203)))]
            );
        } else {
            assert_eq!(
                result.state,
                OperationState::PartiallyCompleted,
                "{}",
                result.detail
            );
        }
        assert_eq!(count(&calls, "VIF.destroy"), 1);
        assert_eq!(count(&calls, "VIF.create"), 1);
    }
}
#[tokio::test]
async fn partial_disk_deletion_and_lost_reply_keep_distinct_outcomes() {
    for fault in [Fault::LostDiskDelete, Fault::RefuseDiskDelete] {
        let (fixture, records, calls) = fixture(fault).await;
        let directory = tempfile::tempdir().unwrap();
        let result = execute(&fixture, directory.path(), delete_disk()).await;
        assert!(!records.lock().unwrap().contains_key("OpaqueRef:VBD:disk"));
        if fault == Fault::LostDiskDelete {
            assert_eq!(
                result.state,
                OperationState::OutcomeUnknown,
                "{}",
                result.detail
            );
            let result = recheck(&fixture, directory.path(), result).await;
            assert_eq!(result.state, OperationState::Succeeded, "{}", result.detail);
        } else {
            assert_eq!(
                result.state,
                OperationState::PartiallyCompleted,
                "{}",
                result.detail
            );
            assert!(records.lock().unwrap().contains_key("OpaqueRef:VDI:disk"));
        }
        assert_eq!(count(&calls, "VBD.destroy"), 1);
        assert_eq!(count(&calls, "VDI.destroy"), 1);
    }
}
#[tokio::test]
async fn misbound_creation_result_cannot_be_attached_or_claimed_as_created() {
    let (fixture, records, calls) = fixture(Fault::MisboundDiskCreate).await;
    let directory = tempfile::tempdir().unwrap();
    let result = execute(&fixture, directory.path(), create_disk()).await;
    assert_eq!(
        result.state,
        OperationState::OutcomeUnknown,
        "{}",
        result.detail
    );
    assert!(result.created_devices.is_empty());
    assert_eq!(count(&calls, "VBD.create"), 0);
    assert!(records.lock().unwrap().contains_key("OpaqueRef:VDI:spare"));
}
#[tokio::test]
async fn changed_disk_ownership_and_running_or_template_guests_refuse_before_writes() {
    for change in [
        "unowned", "shared", "consumer", "moved", "running", "template",
    ] {
        let (fixture, records, calls) = fixture(Fault::None).await;
        let directory = tempfile::tempdir().unwrap();
        {
            let mut records = records.lock().unwrap();
            match change {
                "unowned" => {
                    records.get_mut("OpaqueRef:VBD:disk").unwrap()["other_config"] = json!({})
                }
                "shared" => {
                    records.get_mut("OpaqueRef:VDI:disk").unwrap()["sharable"] = json!(true)
                }
                "consumer" => {
                    records.get_mut("OpaqueRef:VDI:disk").unwrap()["VBDs"] =
                        json!(["OpaqueRef:VBD:disk", "OpaqueRef:VBD:other"])
                }
                "moved" => {
                    records.get_mut("OpaqueRef:VBD:disk").unwrap()["VM"] =
                        json!("OpaqueRef:VM:other")
                }
                "running" => {
                    records.get_mut("OpaqueRef:VM:guest").unwrap()["power_state"] = json!("Running")
                }
                _ => records.get_mut("OpaqueRef:VM:guest").unwrap()["is_a_template"] = json!(true),
            }
        }
        let result = execute(&fixture, directory.path(), delete_disk()).await;
        assert_eq!(
            result.state,
            OperationState::Rejected,
            "{change}: {}",
            result.detail
        );
        assert_eq!(count(&calls, "VBD.destroy"), 0);
        assert_eq!(count(&calls, "VDI.destroy"), 0);
    }
}
#[tokio::test]
async fn nic_replacement_refuses_changed_or_unpreservable_configuration() {
    for change in ["network", "mac", "custom", "attached"] {
        let (fixture, records, calls) = fixture(Fault::None).await;
        let directory = tempfile::tempdir().unwrap();
        {
            let mut records = records.lock().unwrap();
            let nic = records.get_mut("OpaqueRef:VIF:nic").unwrap();
            match change {
                "network" => nic["network"] = json!("OpaqueRef:network:destination"),
                "mac" => nic["MAC"] = json!("02:00:00:00:00:ff"),
                "custom" => {
                    nic["other_config"] = json!({"private-config":"synthetic-private-value"})
                }
                _ => nic["currently_attached"] = json!(true),
            }
        }
        let result = execute(&fixture, directory.path(), edit_nic("02:00:00:00:00:0a")).await;
        assert_eq!(
            result.state,
            OperationState::Rejected,
            "{change}: {}",
            result.detail
        );
        assert_eq!(count(&calls, "VIF.destroy"), 0);
        assert_eq!(count(&calls, "VIF.create"), 0);
        assert!(
            !serde_json::to_string(&result)
                .unwrap()
                .contains("synthetic-private-value")
        );
    }
}
#[test]
fn configuration_validation_rejects_unsafe_mac_slots_sizes_and_names() {
    for address in [
        "",
        "ff:ff:ff:ff:ff:ff",
        "01:00:00:00:00:00",
        "00:00:00:00:00:00",
        "02:0:00:00:00:01",
        "02:00:00:00:00:01\n",
    ] {
        assert!(
            DeviceOperation::AddNic {
                network: NetworkId::from(uuid(7)),
                device: "1".into(),
                mac: Some(address.into())
            }
            .validate()
            .is_err()
        );
    }
    for device in ["-1", "256", "01", "", "1\n"] {
        assert!(
            DeviceOperation::AddCdDrive {
                device: device.into()
            }
            .validate()
            .is_err()
        );
    }
    for size in [0, 1, 513, u64::MAX] {
        assert!(
            DeviceOperation::CreateDisk {
                sr: SrId::from(uuid(3)),
                name: "disk".into(),
                size,
                device: "1".into()
            }
            .validate()
            .is_err()
        );
    }
}

#[tokio::test]
async fn imported_attachment_cannot_silently_claim_disk_ownership() {
    let (fixture, _, calls) = fixture(Fault::UnexpectedAttachmentOwner).await;
    let directory = tempfile::tempdir().unwrap();
    let result = execute(
        &fixture,
        directory.path(),
        DeviceOperation::AttachDisk {
            vdi: VdiId::from(uuid(6)),
            device: "1".into(),
            read_only: false,
        },
    )
    .await;
    assert_eq!(
        result.state,
        OperationState::OutcomeUnknown,
        "{}",
        result.detail
    );
    assert!(result.created_devices.is_empty());
    assert_eq!(count(&calls, "VDI.destroy"), 0);
}

#[tokio::test]
async fn nic_ip_sets_and_mac_case_have_semantic_comparison() {
    let (fixture, records, _) = fixture(Fault::ReorderedNicConfig).await;
    let directory = tempfile::tempdir().unwrap();
    records
        .lock()
        .unwrap()
        .get_mut("OpaqueRef:VIF:nic")
        .unwrap()["ipv4_allowed"] = json!(["198.51.100.9", "192.0.2.9", "198.51.100.9"]);
    let result = execute(&fixture, directory.path(), edit_nic("02:00:00:00:00:0a")).await;
    assert_eq!(result.state, OperationState::Succeeded, "{}", result.detail);
    let (fixture, records, calls) = self::fixture(Fault::None).await;
    let directory = tempfile::tempdir().unwrap();
    records
        .lock()
        .unwrap()
        .get_mut("OpaqueRef:VIF:nic")
        .unwrap()["MAC"] = json!("02:AA:BB:CC:DD:09");
    let result = execute(
        &fixture,
        directory.path(),
        DeviceOperation::EditNic {
            vif: VifId::from(uuid(9)),
            expected_network: NetworkId::from(uuid(7)),
            expected_mac: "02:AA:BB:CC:DD:09".into(),
            network: NetworkId::from(uuid(8)),
            mac: "02:aa:bb:cc:dd:09".into(),
        },
    )
    .await;
    assert_eq!(result.state, OperationState::Succeeded, "{}", result.detail);
    assert_eq!(count(&calls, "VIF.destroy"), 0);
    assert_eq!(count(&calls, "VIF.move"), 1);
}

#[path = "device_property_tests.rs"]
mod properties;
