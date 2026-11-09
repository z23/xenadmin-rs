use super::*;
use crate::xapi::inventory_tests::Fixture;
use crate::{
    GuestFirmware, SnapshotDeletionSelection, SnapshotId, VdiId, VmCopyMode, VmLifecycleOperation,
};
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
    LostClone,
    PollMissing,
    PollMissingOnce,
    TaskFailure,
    ConcurrentMetadata,
    RefuseClone,
    LostProvision,
    RefuseDisk,
    LostDelete,
    RefuseDiskDelete,
    ExistingClone,
    MissingCopiedDisk,
    NoPossibleHost,
    LostBios,
    WrongBiosAffinity,
    NoSecureBootApi,
}
fn guest(n: u128, template: bool) -> Value {
    json!({"uuid":uuid(n),"name_label":"Source template","name_description":"original","is_a_template":template,"is_default_template":false,"is_a_snapshot":false,"is_control_domain":false,"snapshot_of":"OpaqueRef:NULL","power_state":"Halted","current_operations":{},"blocked_operations":{},"allowed_operations":["clone","copy","provision","make_into_template","destroy","changing_memory_limits","changing_VCPUs"],"VBDs":[],"VIFs":[],"VCPUs_max":"2","VCPUs_at_startup":"2","memory_static_min":"16777216","memory_static_max":"1073741824","memory_dynamic_min":"1073741824","memory_dynamic_max":"1073741824","HVM_boot_policy":"BIOS order","HVM_boot_params":{"order":"dc","firmware":"uefi","extra":"preserve"},"platform":{"secureboot":"false"},"other_config":{"disks":"<invalid unsafe xml>must not be interpreted","keep":"unchanged"}})
}
fn disk(n: u128) -> Value {
    json!({"uuid":uuid(n),"name_label":"Disk","SR":"OpaqueRef:SR:source","virtual_size":"1073741824","VBDs":[],"type":"user","managed":true,"missing":false,"sharable":false,"read_only":false,"is_a_snapshot":false,"current_operations":{},"allowed_operations":["destroy"],"other_config":{}})
}
async fn fixture(fault: Fault) -> (Fixture, Records, Calls) {
    fixture_with_recovery(fault, None).await
}
async fn fixture_with_recovery(
    fault: Fault,
    recovery: Option<(std::path::PathBuf, bool)>,
) -> (Fixture, Records, Calls) {
    fixture_with_authority(fault, recovery, json!({"is_local_superuser":true})).await
}
async fn fixture_with_authority(
    fault: Fault,
    recovery: Option<(std::path::PathBuf, bool)>,
    permissions: Value,
) -> (Fixture, Records, Calls) {
    let mut source = guest(2, true);
    source["VBDs"] = json!(["OpaqueRef:VBD:source"]);
    source["VIFs"] = json!(["OpaqueRef:VIF:source"]);
    let mut source_disk = disk(4);
    source_disk["VBDs"] = json!(["OpaqueRef:VBD:source"]);
    let records: Records = Arc::new(Mutex::new(BTreeMap::from([
        ("OpaqueRef:VM:source".into(), source),
        ("OpaqueRef:VM:unrelated".into(), guest(99, false)),
        (
            "OpaqueRef:SR:source".into(),
            json!({"uuid":uuid(3),"content_type":"user","current_operations":{},"allowed_operations":["vdi_create"]}),
        ),
        (
            "OpaqueRef:SR:destination".into(),
            json!({"uuid":uuid(13),"content_type":"user","current_operations":{},"allowed_operations":["vdi_create"]}),
        ),
        ("OpaqueRef:VDI:source".into(), source_disk),
        (
            "OpaqueRef:VBD:source".into(),
            json!({"uuid":uuid(5),"VM":"OpaqueRef:VM:source","VDI":"OpaqueRef:VDI:source","type":"Disk","mode":"RW","userdevice":"0","currently_attached":false,"empty":false,"other_config":{"owner":"true"},"current_operations":{},"allowed_operations":[]}),
        ),
        (
            "OpaqueRef:network:source".into(),
            json!({"uuid":uuid(6),"MTU":"1500","current_operations":{}}),
        ),
        (
            "OpaqueRef:network:destination".into(),
            json!({"uuid":uuid(7),"MTU":"1500","current_operations":{}}),
        ),
        (
            "OpaqueRef:VIF:source".into(),
            json!({"uuid":uuid(8),"VM":"OpaqueRef:VM:source","network":"OpaqueRef:network:source","device":"0","MAC":"02:00:00:00:00:08","MTU":"1500","currently_attached":false,"current_operations":{},"other_config":{}}),
        ),
        (
            "OpaqueRef:VDI:iso".into(),
            json!({"uuid":uuid(9),"type":"iso","read_only":true,"missing":false,"VBDs":[]}),
        ),
    ])));
    let calls: Calls = Arc::new(Mutex::new(vec![]));
    let handler = {
        let records = records.clone();
        let calls = calls.clone();
        move |request: Value| {
            let method = request["method"].as_str().unwrap();
            calls.lock().unwrap().push(request.clone());
            let mut r = records.lock().unwrap();
            let p = &request["params"];
            if let Some(class) = method.strip_suffix(".get_all_records") {
                return json!({"result":r.iter().filter(|(k,_)|k.starts_with(&format!("OpaqueRef:{class}:"))).collect::<BTreeMap<_,_>>()});
            }
            if let Some(class) = method.strip_suffix(".get_by_uuid") {
                return r
                    .iter()
                    .find(|(k, v)| {
                        k.starts_with(&format!("OpaqueRef:{class}:")) && v["uuid"] == p[1]
                    })
                    .map_or(
                        json!({"error":{"message":"UUID_INVALID"}}),
                        |(k, _)| json!({"result":k}),
                    );
            }
            if method == "task.get_record"
                && (fault == Fault::PollMissing
                    || (fault == Fault::PollMissingOnce
                        && calls
                            .lock()
                            .unwrap()
                            .iter()
                            .filter(|c| c["method"] == method)
                            .count()
                            == 1))
            {
                return json!({"error":{"message":"HANDLE_INVALID"}});
            }
            if method.ends_with(".get_record") {
                return r.get(p[1].as_str().unwrap()).map_or(
                    json!({"error":{"message":"HANDLE_INVALID"}}),
                    |v| json!({"result":v}),
                );
            }
            if matches!(method, "Async.VM.clone" | "Async.VM.copy") {
                if fault == Fault::RefuseClone {
                    return json!({"error":{"message":"RBAC_PERMISSION_DENIED"}});
                }
                let result = if fault == Fault::ExistingClone {
                    "OpaqueRef:VM:unrelated"
                } else {
                    "OpaqueRef:VM:new"
                };
                if fault != Fault::ExistingClone {
                    let mut new = r[p[1].as_str().unwrap()].clone();
                    new["uuid"] = json!(uuid(300));
                    new["name_label"] = p[2].clone();
                    new["other_config"]
                        .as_object_mut()
                        .unwrap()
                        .remove("default_template");
                    new["is_default_template"] = json!(false);
                    let mut vbds = vec![];
                    for (i, old) in new["VBDs"]
                        .as_array()
                        .unwrap()
                        .clone()
                        .into_iter()
                        .enumerate()
                    {
                        if fault == Fault::MissingCopiedDisk {
                            continue;
                        }
                        let mut vbd = r[old.as_str().unwrap()].clone();
                        let attachment = format!("OpaqueRef:VBD:clone{i}");
                        vbd["uuid"] = json!(uuid(302 + i as u128));
                        vbd["VM"] = json!(result);
                        if vbd["type"] == "Disk" {
                            let mut d = r[vbd["VDI"].as_str().unwrap()].clone();
                            let disk = format!("OpaqueRef:VDI:clone{i}");
                            d["uuid"] = json!(uuid(301 + i as u128));
                            d["VBDs"] = json!([attachment]);
                            if method.ends_with("copy") {
                                d["SR"] = p[3].clone();
                            }
                            vbd["VDI"] = json!(disk);
                            r.insert(disk, d);
                        }
                        r.insert(attachment.clone(), vbd);
                        vbds.push(attachment);
                    }
                    new["VBDs"] = json!(vbds);
                    let mut vifs = vec![];
                    for (i, old) in new["VIFs"]
                        .as_array()
                        .unwrap()
                        .clone()
                        .into_iter()
                        .enumerate()
                    {
                        let mut nic = r[old.as_str().unwrap()].clone();
                        let nic_ref = format!("OpaqueRef:VIF:clone{i}");
                        nic["uuid"] = json!(uuid(310 + i as u128));
                        nic["VM"] = json!(result);
                        nic["MAC"] = json!("02:00:00:00:01:00");
                        r.insert(nic_ref.clone(), nic);
                        vifs.push(nic_ref);
                    }
                    new["VIFs"] = json!(vifs);
                    r.insert(result.into(), new);
                }
                r.insert(
                    "OpaqueRef:task:clone".into(),
                    json!({"status":"success","result":format!("<value>{result}</value>")}),
                );
                if fault == Fault::TaskFailure {
                    r.get_mut("OpaqueRef:task:clone").unwrap()["status"] = json!("failure");
                    r.get_mut("OpaqueRef:task:clone").unwrap()["error_info"] = json!(["SR_FULL"]);
                }
                return if fault == Fault::LostClone {
                    json!({"_fixture_close_without_reply":true})
                } else {
                    json!({"result":"OpaqueRef:task:clone"})
                };
            }
            if method == "Async.VM.provision" {
                r.get_mut(p[1].as_str().unwrap()).unwrap()["is_a_template"] = json!(false);
                r.insert(
                    "OpaqueRef:task:provision".into(),
                    json!({"status":"success","result":""}),
                );
                return if fault == Fault::LostProvision {
                    json!({"_fixture_close_without_reply":true})
                } else {
                    json!({"result":"OpaqueRef:task:provision"})
                };
            }
            if method == "VM.get_secureboot_readiness" {
                return if fault == Fault::NoSecureBootApi {
                    json!({"error":{"message":"MESSAGE_METHOD_UNKNOWN"}})
                } else {
                    json!({"result":"disabled"})
                };
            }
            if method == "VM.copy_bios_strings" {
                let h = r[p[2].as_str().unwrap()].clone();
                let v = r.get_mut(p[1].as_str().unwrap()).unwrap();
                v["bios_strings"] = h["bios_strings"].clone();
                v["affinity"] = if fault == Fault::WrongBiosAffinity {
                    json!("OpaqueRef:NULL")
                } else {
                    p[2].clone()
                };
                if let Some((directory, acknowledge)) = &recovery {
                    let pool = uuid(1).into();
                    let recovered = recover(directory, pool).unwrap();
                    assert_eq!(recovered.len(), 1);
                    if *acknowledge {
                        Journal::open(directory)
                            .unwrap()
                            .control(pool, recovered[0].id, true)
                            .unwrap();
                    }
                }
                return if fault == Fault::LostBios {
                    json!({"_fixture_close_without_reply":true})
                } else {
                    json!({"result":null})
                };
            }
            if method == "VM.get_possible_hosts" {
                return json!({"result":if fault==Fault::NoPossibleHost{vec![]}else{vec!["OpaqueRef:host:available"]}});
            }
            if matches!(
                method,
                "VM.get_allowed_VBD_devices" | "VM.get_allowed_VIF_devices"
            ) {
                let (class, key) = if method.contains("VBD") {
                    ("VBD", "userdevice")
                } else {
                    ("VIF", "device")
                };
                return json!({"result":(0..16).map(|n|n.to_string()).filter(|slot|!r.iter().any(|(k,v)|k.starts_with(&format!("OpaqueRef:{class}:"))&&v["VM"]==p[1]&&v[key]==*slot)).collect::<Vec<_>>()});
            }
            if method == "VM.remove_from_other_config" {
                r.get_mut(p[1].as_str().unwrap()).unwrap()["other_config"]
                    .as_object_mut()
                    .unwrap()
                    .remove(p[2].as_str().unwrap());
                return json!({"result":null});
            }
            if let Some(field) = method.strip_prefix("VM.set_") {
                let vm = r.get_mut(p[1].as_str().unwrap()).unwrap();
                if field == "memory_limits" {
                    for (i, key) in [
                        "memory_static_min",
                        "memory_static_max",
                        "memory_dynamic_min",
                        "memory_dynamic_max",
                    ]
                    .iter()
                    .enumerate()
                    {
                        vm[*key] = p[i + 2].clone()
                    }
                } else {
                    vm[field] = p[2].clone()
                }
                if field == "name_label" && fault == Fault::ConcurrentMetadata {
                    vm["name_description"] = json!("Another operator changed this");
                }
                return json!({"result":null});
            }
            if let Some(class) = method.strip_suffix(".create") {
                if class == "VDI" && fault == Fault::RefuseDisk {
                    return json!({"error":{"message":"SR_FULL"}});
                }
                let count = r.len() as u128 + 1000;
                let reference = format!("OpaqueRef:{class}:new{count}");
                let mut value = p[1].clone();
                value["uuid"] = json!(uuid(count));
                value["current_operations"] = json!({});
                value["currently_attached"] = json!(false);
                match class {
                    "VDI" => {
                        value["managed"] = json!(true);
                        value["missing"] = json!(false);
                        value["is_a_snapshot"] = json!(false);
                        value["VBDs"] = json!([]);
                        value["allowed_operations"] = json!(["destroy"]);
                    }
                    "VBD" => {
                        value["allowed_operations"] = json!(["insert", "eject"]);
                        if value["VDI"] != "OpaqueRef:NULL" {
                            r.get_mut(value["VDI"].as_str().unwrap()).unwrap()["VBDs"]
                                .as_array_mut()
                                .unwrap()
                                .push(json!(reference));
                        }
                        r.get_mut(value["VM"].as_str().unwrap()).unwrap()["VBDs"]
                            .as_array_mut()
                            .unwrap()
                            .push(json!(reference));
                    }
                    "VIF" => {
                        value["MAC"] = json!("02:00:00:00:02:00");
                        r.get_mut(value["VM"].as_str().unwrap()).unwrap()["VIFs"]
                            .as_array_mut()
                            .unwrap()
                            .push(json!(reference));
                    }
                    _ => panic!("unexpected create {class}"),
                }
                r.insert(reference.clone(), value);
                return json!({"result":reference});
            }
            if method == "VBD.add_to_other_config" {
                r.get_mut(p[1].as_str().unwrap()).unwrap()["other_config"]
                    [p[2].as_str().unwrap()] = p[3].clone();
                return json!({"result":null});
            }
            if method == "VIF.move" {
                r.get_mut(p[1].as_str().unwrap()).unwrap()["network"] = p[2].clone();
                return json!({"result":null});
            }
            if method == "VBD.insert" {
                let v = r.get_mut(p[1].as_str().unwrap()).unwrap();
                v["VDI"] = p[2].clone();
                v["empty"] = json!(false);
                return json!({"result":null});
            }
            if method == "VBD.eject" {
                let v = r.get_mut(p[1].as_str().unwrap()).unwrap();
                v["VDI"] = json!("OpaqueRef:NULL");
                v["empty"] = json!(true);
                return json!({"result":null});
            }
            if method == "VM.destroy" {
                let reference = p[1].as_str().unwrap();
                r.remove(reference);
                let attachments = r
                    .iter()
                    .filter(|(_, v)| v["VM"] == reference)
                    .map(|(k, _)| k.clone())
                    .collect::<Vec<_>>();
                for a in attachments {
                    let v = r.remove(&a).unwrap();
                    if let Some(d) = v["VDI"].as_str().and_then(|id| r.get_mut(id)) {
                        d["VBDs"].as_array_mut().unwrap().retain(|id| id != &a);
                    }
                }
                return if fault == Fault::LostDelete {
                    json!({"_fixture_close_without_reply":true})
                } else {
                    json!({"result":null})
                };
            }
            if method == "VDI.destroy" {
                if fault == Fault::RefuseDiskDelete {
                    return json!({"error":{"message":"SR_BACKEND_FAILURE"}});
                }
                r.remove(p[1].as_str().unwrap());
                return json!({"result":null});
            }
            panic!("unexpected lifecycle fixture method {method}")
        }
    };
    let fixture = Fixture::with_permissions(handler, permissions).await;
    (fixture, records, calls)
}
async fn execute(
    fixture: &Fixture,
    directory: &Path,
    action: VmLifecycleOperation,
) -> OperationRecord {
    let (operation, request) = channel(
        OperationId::new(),
        uuid(1).into(),
        vm(),
        1,
        OperationAction::Lifecycle(action),
        tokio::runtime::Handle::current(),
    );
    let (tx, _rx) = mpsc::channel(256);
    tokio::time::timeout(
        Duration::from_secs(8),
        request.run(
            Arc::new(fixture.session().await),
            Some("OpaqueRef:VM:source".into()),
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
    let (tx, _rx) = mpsc::channel(256);
    MutationRequest::tracking(record)
        .run(
            Arc::new(fixture.session().await),
            None,
            directory.into(),
            CancellationToken::new(),
            tx,
        )
        .await;
    Journal::open(directory).unwrap().load(id).unwrap().unwrap()
}
fn count(calls: &Calls, method: &str) -> usize {
    calls
        .lock()
        .unwrap()
        .iter()
        .filter(|r| r["method"] == method)
        .count()
}
fn cloning(mode: VmCopyMode) -> VmLifecycleOperation {
    VmLifecycleOperation::Clone {
        name: "Reviewed clone".into(),
        expected_template: true,
        mode,
        as_template: true,
    }
}
fn creating() -> VmLifecycleOperation {
    VmLifecycleOperation::Create {
        name: "New VM".into(),
        description: "Reviewed description".into(),
        sr: uuid(13).into(),
        network: uuid(7).into(),
        iso: Some(uuid(9).into()),
        vcpus: 4,
        memory: 2 * 1024 * 1024 * 1024,
        disk_size: Some(1024 * 1024 * 1024),
        firmware: GuestFirmware::Uefi,
        bios_host: None,
    }
}
fn deleting(disks: Vec<VdiId>) -> VmLifecycleOperation {
    VmLifecycleOperation::Delete {
        expected_template: true,
        disks,
        snapshots: vec![],
    }
}
#[tokio::test]
async fn fast_clone_and_full_copy_preserve_source_and_verify_new_resource_ids() {
    for mode in [
        VmCopyMode::Fast,
        VmCopyMode::Full {
            sr: uuid(13).into(),
        },
    ] {
        let (f, r, c) = fixture(Fault::None).await;
        let dir = tempfile::tempdir().unwrap();
        let result = execute(&f, dir.path(), cloning(mode.clone())).await;
        assert_eq!(result.state, OperationState::Succeeded, "{}", result.detail);
        assert_eq!(result.created_vm, Some(uuid(300).into()));
        assert_eq!(result.created_devices.len(), 3);
        assert_eq!(
            r.lock().unwrap()["OpaqueRef:VM:source"]["name_label"],
            "Source template"
        );
        assert_eq!(
            count(
                &c,
                if matches!(mode, VmCopyMode::Fast) {
                    "Async.VM.clone"
                } else {
                    "Async.VM.copy"
                }
            ),
            1
        );
    }
}
#[tokio::test]
async fn creates_vm_with_resources_disk_network_iso_and_merged_firmware() {
    let (f, r, c) = fixture(Fault::None).await;
    let dir = tempfile::tempdir().unwrap();
    let result = execute(&f, dir.path(), creating()).await;
    assert_eq!(result.state, OperationState::Succeeded, "{}", result.detail);
    let records = r.lock().unwrap();
    let vm = &records["OpaqueRef:VM:new"];
    assert_eq!(vm["is_a_template"], false);
    assert_eq!(vm["VCPUs_max"], "4");
    assert_eq!(vm["memory_static_max"], "2147483648");
    assert_eq!(vm["HVM_boot_params"]["extra"], "preserve");
    assert_eq!(vm["HVM_boot_params"]["firmware"], "uefi");
    assert!(vm["other_config"].get("disks").is_none());
    assert_eq!(vm["other_config"]["keep"], "unchanged");
    assert_eq!(
        records["OpaqueRef:VIF:clone0"]["network"],
        "OpaqueRef:network:destination"
    );
    assert_eq!(count(&c, "VM.start"), 0);
    assert_eq!(count(&c, "Async.VM.copy"), 1);
    assert_eq!(count(&c, "VDI.create"), 1);
    assert_eq!(count(&c, "VBD.insert"), 1);
}
/// XCP-ng 8.3.0 advertises only the memory range operations for halted guests and no vCPU
/// operation at all, although the setters succeed. Creation must not refuse on their absence.
#[tokio::test]
async fn creates_vm_when_the_host_advertises_only_range_operations() {
    let (f, r, c) = fixture(Fault::None).await;
    {
        let mut r = r.lock().unwrap();
        r.get_mut("OpaqueRef:VM:source").unwrap()["allowed_operations"] = json!([
            "clone",
            "copy",
            "provision",
            "make_into_template",
            "destroy",
            "changing_static_range",
            "changing_dynamic_range"
        ]);
    }
    let dir = tempfile::tempdir().unwrap();
    let result = execute(&f, dir.path(), creating()).await;
    assert_eq!(result.state, OperationState::Succeeded, "{}", result.detail);
    let records = r.lock().unwrap();
    assert_eq!(records["OpaqueRef:VM:new"]["VCPUs_max"], "4");
    assert_eq!(
        records["OpaqueRef:VM:new"]["memory_static_max"],
        "2147483648"
    );
    assert_eq!(count(&c, "VM.set_memory_limits"), 1);
    assert_eq!(count(&c, "VM.set_VCPUs_max"), 1);
    let mut running = records["OpaqueRef:VM:source"].clone();
    running["power_state"] = json!("Running");
    assert!(crate::vm_lifecycle_backend::permits_vcpu_change_for_test(&running).is_err());
    let mut halted_partial = records["OpaqueRef:VM:source"].clone();
    halted_partial["allowed_operations"] = json!(["changing_static_range"]);
    assert!(crate::vm_lifecycle_backend::permits_memory_limits_for_test(&halted_partial).is_err());
}
#[tokio::test]
async fn diskless_template_uses_clone_then_explicit_owned_disk_and_new_nic() {
    let (f, r, c) = fixture(Fault::None).await;
    {
        let mut r = r.lock().unwrap();
        r.get_mut("OpaqueRef:VM:source").unwrap()["VBDs"] = json!([]);
        r.get_mut("OpaqueRef:VM:source").unwrap()["VIFs"] = json!([]);
    }
    let dir = tempfile::tempdir().unwrap();
    let result = execute(&f, dir.path(), creating()).await;
    assert_eq!(result.state, OperationState::Succeeded, "{}", result.detail);
    assert_eq!(count(&c, "Async.VM.clone"), 1);
    assert_eq!(count(&c, "VIF.create"), 1);
    assert_eq!(count(&c, "VDI.create"), 1);
    assert!(
        r.lock()
            .unwrap()
            .values()
            .any(|v| v["VM"] == "OpaqueRef:VM:new" && v["other_config"]["owner"] == "true")
    );
}
#[tokio::test]
async fn converts_halted_vm_and_edits_custom_template_metadata() {
    let (f, r, c) = fixture(Fault::None).await;
    r.lock().unwrap().get_mut("OpaqueRef:VM:source").unwrap()["is_a_template"] = json!(false);
    let dir = tempfile::tempdir().unwrap();
    let result = execute(&f, dir.path(), VmLifecycleOperation::ConvertToTemplate).await;
    assert_eq!(result.state, OperationState::Succeeded, "{}", result.detail);
    let result = execute(
        &f,
        dir.path(),
        VmLifecycleOperation::EditTemplate {
            name: "Edited".into(),
            description: "Updated".into(),
        },
    )
    .await;
    assert_eq!(result.state, OperationState::Succeeded, "{}", result.detail);
    assert_eq!(
        r.lock().unwrap()["OpaqueRef:VM:source"]["name_label"],
        "Edited"
    );
    assert_eq!(count(&c, "VM.set_is_a_template"), 1);
}
#[tokio::test]
async fn record_only_deletion_keeps_disks_and_explicit_owned_cleanup_removes_only_selection() {
    for delete_data in [false, true] {
        let (f, r, c) = fixture(Fault::None).await;
        let dir = tempfile::tempdir().unwrap();
        let result = execute(
            &f,
            dir.path(),
            deleting(if delete_data {
                vec![uuid(4).into()]
            } else {
                vec![]
            }),
        )
        .await;
        assert_eq!(result.state, OperationState::Succeeded, "{}", result.detail);
        assert!(!r.lock().unwrap().contains_key("OpaqueRef:VM:source"));
        assert_eq!(
            r.lock().unwrap().contains_key("OpaqueRef:VDI:source"),
            !delete_data
        );
        assert!(r.lock().unwrap().contains_key("OpaqueRef:VM:unrelated"));
        assert_eq!(count(&c, "VDI.destroy"), usize::from(delete_data));
    }
}
#[tokio::test]
async fn lost_clone_reply_remains_unknown_without_replay_or_name_search() {
    let (f, _, c) = fixture(Fault::LostClone).await;
    let dir = tempfile::tempdir().unwrap();
    let result = execute(&f, dir.path(), cloning(VmCopyMode::Fast)).await;
    assert_eq!(result.state, OperationState::OutcomeUnknown);
    let recovered = Journal::open(dir.path())
        .unwrap()
        .history()
        .unwrap()
        .remove(0);
    let result = recheck(&f, dir.path(), recovered).await;
    assert_eq!(result.state, OperationState::OutcomeUnknown);
    assert_eq!(count(&c, "Async.VM.clone"), 1);
}
#[tokio::test]
async fn task_poll_api_failure_is_unknown_while_submission_refusal_is_definite() {
    for (fault, state) in [
        (Fault::PollMissing, OperationState::OutcomeUnknown),
        (Fault::RefuseClone, OperationState::Failed),
    ] {
        let (f, _, c) = fixture(fault).await;
        let dir = tempfile::tempdir().unwrap();
        let result = execute(&f, dir.path(), cloning(VmCopyMode::Fast)).await;
        assert_eq!(result.state, state, "{}", result.detail);
        assert_eq!(count(&c, "VM.set_is_a_template"), 0);
    }
}
#[tokio::test]
async fn lost_provision_reply_retains_created_vm_and_never_submits_configuration() {
    let (f, _, c) = fixture(Fault::LostProvision).await;
    let dir = tempfile::tempdir().unwrap();
    let result = execute(&f, dir.path(), creating()).await;
    assert_eq!(
        result.state,
        OperationState::OutcomeUnknown,
        "{}",
        result.detail
    );
    assert_eq!(result.created_vm, Some(uuid(300).into()));
    let result = recheck(&f, dir.path(), result).await;
    assert_eq!(result.state, OperationState::OutcomeUnknown);
    assert_eq!(count(&c, "Async.VM.provision"), 1);
    assert_eq!(count(&c, "VM.set_memory_limits"), 0);
}
#[tokio::test]
async fn creation_failure_retains_vm_and_copied_disks_without_automatic_cleanup() {
    let (f, r, c) = fixture(Fault::RefuseDisk).await;
    let dir = tempfile::tempdir().unwrap();
    let result = execute(&f, dir.path(), creating()).await;
    assert_eq!(
        result.state,
        OperationState::PartiallyCompleted,
        "{}",
        result.detail
    );
    assert_eq!(result.created_vm, Some(uuid(300).into()));
    assert!(r.lock().unwrap().contains_key("OpaqueRef:VDI:clone0"));
    assert_eq!(count(&c, "VM.destroy"), 0);
    assert_eq!(count(&c, "VDI.destroy"), 0);
}
#[tokio::test]
async fn lost_parent_delete_reply_reconciles_without_submitting_pending_disk_deletion() {
    let (f, r, c) = fixture(Fault::LostDelete).await;
    let dir = tempfile::tempdir().unwrap();
    let result = execute(&f, dir.path(), deleting(vec![uuid(4).into()])).await;
    assert_eq!(result.state, OperationState::OutcomeUnknown);
    let result = recheck(&f, dir.path(), result).await;
    assert_eq!(result.state, OperationState::PartiallyCompleted);
    assert!(r.lock().unwrap().contains_key("OpaqueRef:VDI:source"));
    assert_eq!(count(&c, "VM.destroy"), 1);
    assert_eq!(count(&c, "VDI.destroy"), 0);
}
#[tokio::test]
async fn disk_delete_failure_after_record_deletion_is_explicit_partial() {
    let (f, r, _) = fixture(Fault::RefuseDiskDelete).await;
    let dir = tempfile::tempdir().unwrap();
    let result = execute(&f, dir.path(), deleting(vec![uuid(4).into()])).await;
    assert_eq!(
        result.state,
        OperationState::PartiallyCompleted,
        "{}",
        result.detail
    );
    assert!(r.lock().unwrap().contains_key("OpaqueRef:VDI:source"));
}
#[tokio::test]
async fn existing_or_incomplete_clone_result_is_never_mutated_or_claimed_complete() {
    for fault in [Fault::ExistingClone, Fault::MissingCopiedDisk] {
        let (f, _, c) = fixture(fault).await;
        let dir = tempfile::tempdir().unwrap();
        let result = execute(&f, dir.path(), creating()).await;
        assert_eq!(
            result.state,
            OperationState::OutcomeUnknown,
            "{}",
            result.detail
        );
        assert_eq!(count(&c, "Async.VM.provision"), 0);
        assert_eq!(count(&c, "VM.set_is_a_template"), 0);
        if fault == Fault::ExistingClone {
            assert_eq!(result.created_vm, None);
        }
    }
}
#[tokio::test]
async fn refuses_changed_ownership_shared_disk_and_default_template_before_deletion() {
    for field in [
        "owner", "shared", "consumer", "default", "running", "control",
    ] {
        let (f, r, c) = fixture(Fault::None).await;
        {
            let mut r = r.lock().unwrap();
            match field {
                "owner" => r.get_mut("OpaqueRef:VBD:source").unwrap()["other_config"] = json!({}),
                "shared" => r.get_mut("OpaqueRef:VDI:source").unwrap()["sharable"] = json!(true),
                "consumer" => {
                    r.get_mut("OpaqueRef:VDI:source").unwrap()["VBDs"] =
                        json!(["OpaqueRef:VBD:source", "OpaqueRef:VBD:other"])
                }
                "default" => {
                    r.get_mut("OpaqueRef:VM:source").unwrap()["other_config"]["default_template"] =
                        json!("true")
                }
                "running" => {
                    r.get_mut("OpaqueRef:VM:source").unwrap()["power_state"] = json!("Running")
                }
                _ => r.get_mut("OpaqueRef:VM:source").unwrap()["is_control_domain"] = json!(true),
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let result = execute(&f, dir.path(), deleting(vec![uuid(4).into()])).await;
        assert_ne!(result.state, OperationState::Succeeded, "{field}");
        assert_eq!(count(&c, "VM.destroy"), 0);
    }
}
#[tokio::test]
async fn deleted_snapshots_are_explicit_and_unselected_snapshots_survive() {
    let (f, r, c) = fixture(Fault::None).await;
    {
        let mut r = r.lock().unwrap();
        for n in [21, 22] {
            let mut snapshot = guest(n, true);
            snapshot["is_a_snapshot"] = json!(true);
            snapshot["snapshot_of"] = json!("OpaqueRef:VM:source");
            r.insert(format!("OpaqueRef:VM:snapshot{n}"), snapshot);
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let result = execute(
        &f,
        dir.path(),
        VmLifecycleOperation::Delete {
            expected_template: true,
            disks: vec![],
            snapshots: vec![SnapshotDeletionSelection {
                snapshot: SnapshotId::from(uuid(21)),
                disks: vec![],
            }],
        },
    )
    .await;
    assert_eq!(result.state, OperationState::Succeeded, "{}", result.detail);
    assert!(!r.lock().unwrap().contains_key("OpaqueRef:VM:snapshot21"));
    assert!(r.lock().unwrap().contains_key("OpaqueRef:VM:snapshot22"));
    assert_eq!(count(&c, "VM.destroy"), 2);
}
#[tokio::test]
async fn task_success_recovered_after_poll_failure_does_not_continue_mutations() {
    let (f, _, c) = fixture(Fault::PollMissingOnce).await;
    let dir = tempfile::tempdir().unwrap();
    let result = execute(&f, dir.path(), creating()).await;
    assert_eq!(result.state, OperationState::OutcomeUnknown);
    let result = recheck(&f, dir.path(), result).await;
    assert_eq!(
        result.state,
        OperationState::PartiallyCompleted,
        "{}",
        result.detail
    );
    assert_eq!(result.created_vm, Some(uuid(300).into()));
    assert_eq!(count(&c, "Async.VM.copy"), 1);
    assert_eq!(count(&c, "Async.VM.provision"), 0);
}
#[tokio::test]
async fn observed_failed_task_is_definite_and_never_continues() {
    let (f, _, c) = fixture(Fault::TaskFailure).await;
    let dir = tempfile::tempdir().unwrap();
    let result = execute(&f, dir.path(), creating()).await;
    assert_eq!(result.state, OperationState::Failed, "{}", result.detail);
    assert_eq!(count(&c, "Async.VM.provision"), 0);
}
#[tokio::test]
async fn changed_template_metadata_stops_before_overwriting_concurrent_description() {
    let (f, r, c) = fixture(Fault::ConcurrentMetadata).await;
    let dir = tempfile::tempdir().unwrap();
    let result = execute(
        &f,
        dir.path(),
        VmLifecycleOperation::EditTemplate {
            name: "Reviewed name".into(),
            description: "Requested description".into(),
        },
    )
    .await;
    assert_eq!(
        result.state,
        OperationState::PartiallyCompleted,
        "{}",
        result.detail
    );
    assert_eq!(
        r.lock().unwrap()["OpaqueRef:VM:source"]["name_description"],
        "Another operator changed this"
    );
    assert_eq!(count(&c, "VM.set_name_description"), 0);
}
#[tokio::test]
async fn no_compatible_host_retains_new_vm_as_partial_without_starting_it() {
    let (f, r, c) = fixture(Fault::NoPossibleHost).await;
    let dir = tempfile::tempdir().unwrap();
    let result = execute(&f, dir.path(), creating()).await;
    assert_eq!(
        result.state,
        OperationState::PartiallyCompleted,
        "{}",
        result.detail
    );
    assert_eq!(result.created_vm, Some(uuid(300).into()));
    assert!(result.detail.contains("compatible host"));
    assert_eq!(
        r.lock().unwrap()["OpaqueRef:VM:new"]["power_state"],
        "Halted"
    );
    assert_eq!(count(&c, "Async.VM.start"), 0);
}
#[tokio::test]
async fn bios_choice_does_not_disable_inherited_secure_boot() {
    let (f, r, c) = fixture(Fault::None).await;
    r.lock().unwrap().get_mut("OpaqueRef:VM:source").unwrap()["platform"]["secureboot"] =
        json!("true");
    let mut operation = creating();
    let VmLifecycleOperation::Create { firmware, .. } = &mut operation else {
        unreachable!()
    };
    *firmware = GuestFirmware::Bios;
    let dir = tempfile::tempdir().unwrap();
    let result = execute(&f, dir.path(), operation).await;
    assert_eq!(result.state, OperationState::Rejected);
    assert_eq!(count(&c, "Async.VM.copy"), 0);
    assert_eq!(count(&c, "VM.set_HVM_boot_params"), 0);
}
#[tokio::test]
async fn copied_previously_unowned_disks_gain_ownership_only_on_the_new_attachment() {
    let (f, r, c) = fixture(Fault::None).await;
    r.lock().unwrap().get_mut("OpaqueRef:VBD:source").unwrap()["other_config"] = json!({});
    let dir = tempfile::tempdir().unwrap();
    let result = execute(&f, dir.path(), cloning(VmCopyMode::Fast)).await;
    assert_eq!(result.state, OperationState::Succeeded, "{}", result.detail);
    assert!(
        r.lock().unwrap()["OpaqueRef:VBD:source"]["other_config"]
            .get("owner")
            .is_none()
    );
    assert_eq!(
        r.lock().unwrap()["OpaqueRef:VBD:clone0"]["other_config"]["owner"],
        "true"
    );
    assert_eq!(count(&c, "VBD.add_to_other_config"), 1);
}
#[tokio::test]
async fn source_capability_blocks_and_invalid_sr_refuse_before_copy() {
    for field in ["blocked", "sr", "special"] {
        let (f, r, c) = fixture(Fault::None).await;
        {
            let mut r = r.lock().unwrap();
            match field {
                "blocked" => {
                    r.get_mut("OpaqueRef:VM:source").unwrap()["blocked_operations"]["copy"] =
                        json!("restricted")
                }
                "sr" => {
                    r.get_mut("OpaqueRef:SR:destination").unwrap()["content_type"] = Value::Null
                }
                _ => {
                    r.get_mut("OpaqueRef:VM:source").unwrap()["VGPUs"] =
                        json!(["OpaqueRef:VGPU:some"])
                }
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let result = execute(&f, dir.path(), creating()).await;
        assert_eq!(
            result.state,
            OperationState::Rejected,
            "{field}: {}",
            result.detail
        );
        assert_eq!(count(&c, "Async.VM.copy"), 0);
    }
}
#[tokio::test]
async fn ordinary_vm_clone_can_be_converted_to_template_without_changing_source() {
    let (f, r, c) = fixture(Fault::None).await;
    r.lock().unwrap().get_mut("OpaqueRef:VM:source").unwrap()["is_a_template"] = json!(false);
    let dir = tempfile::tempdir().unwrap();
    let result = execute(
        &f,
        dir.path(),
        VmLifecycleOperation::Clone {
            name: "Template copy".into(),
            expected_template: false,
            mode: VmCopyMode::Fast,
            as_template: true,
        },
    )
    .await;
    assert_eq!(result.state, OperationState::Succeeded, "{}", result.detail);
    assert_eq!(
        r.lock().unwrap()["OpaqueRef:VM:source"]["is_a_template"],
        false
    );
    assert_eq!(r.lock().unwrap()["OpaqueRef:VM:new"]["is_a_template"], true);
    assert_eq!(count(&c, "VM.set_is_a_template"), 1);
}
#[tokio::test]
async fn uncertain_creation_reserves_created_vm_and_disk_identities() {
    let (f, _, _) = fixture(Fault::LostProvision).await;
    let dir = tempfile::tempdir().unwrap();
    let result = execute(&f, dir.path(), creating()).await;
    assert_eq!(result.state, OperationState::OutcomeUnknown);
    let mut journal = Journal::open(dir.path()).unwrap();
    let (_, request) = channel(
        OperationId::new(),
        uuid(1).into(),
        result.created_vm.unwrap(),
        1,
        OperationAction::Power(crate::PowerAction::Start),
        tokio::runtime::Handle::current(),
    );
    let mut attempted = request.record;
    assert!(matches!(
        journal.admit(&mut attempted),
        Err(Error::PoolCreationConflict)
    ));
}
#[tokio::test]
async fn reviewed_snapshot_owned_disk_cleanup_deletes_only_selected_snapshot_data() {
    let (f, r, c) = fixture(Fault::None).await;
    {
        let mut r = r.lock().unwrap();
        let mut snapshot = guest(21, true);
        snapshot["is_a_snapshot"] = json!(true);
        snapshot["snapshot_of"] = json!("OpaqueRef:VM:source");
        snapshot["VBDs"] = json!(["OpaqueRef:VBD:snapshot"]);
        let mut d = disk(24);
        d["is_a_snapshot"] = json!(true);
        d["VBDs"] = json!(["OpaqueRef:VBD:snapshot"]);
        r.insert("OpaqueRef:VM:snapshot".into(), snapshot);
        r.insert("OpaqueRef:VDI:snapshot".into(), d);
        r.insert("OpaqueRef:VBD:snapshot".into(),json!({"uuid":uuid(25),"VM":"OpaqueRef:VM:snapshot","VDI":"OpaqueRef:VDI:snapshot","type":"Disk","other_config":{"owner":"true"}}));
    }
    let dir = tempfile::tempdir().unwrap();
    let result = execute(
        &f,
        dir.path(),
        VmLifecycleOperation::Delete {
            expected_template: true,
            disks: vec![],
            snapshots: vec![SnapshotDeletionSelection {
                snapshot: uuid(21).into(),
                disks: vec![uuid(24).into()],
            }],
        },
    )
    .await;
    assert_eq!(result.state, OperationState::Succeeded, "{}", result.detail);
    assert!(!r.lock().unwrap().contains_key("OpaqueRef:VDI:snapshot"));
    assert!(r.lock().unwrap().contains_key("OpaqueRef:VDI:source"));
    assert_eq!(count(&c, "VDI.destroy"), 1);
}
#[tokio::test]
async fn unresolved_creation_without_result_blocks_other_vms_and_reverse_admission() {
    let (f, _, _) = fixture(Fault::LostClone).await;
    let dir = tempfile::tempdir().unwrap();
    let result = execute(&f, dir.path(), cloning(VmCopyMode::Fast)).await;
    assert_eq!(result.created_vm, None);
    assert_eq!(result.state, OperationState::OutcomeUnknown);
    let mut journal = Journal::open(dir.path()).unwrap();
    let (_, request) = channel(
        OperationId::new(),
        uuid(1).into(),
        uuid(99).into(),
        1,
        OperationAction::Power(crate::PowerAction::Start),
        tokio::runtime::Handle::current(),
    );
    let mut other = request.record.clone();
    assert!(matches!(
        journal.admit(&mut other),
        Err(Error::PoolCreationConflict)
    ));
    let separate = tempfile::tempdir().unwrap();
    let mut journal = Journal::open(separate.path()).unwrap();
    assert!(journal.admit(&mut other).unwrap());
    let (_, creation) = channel(
        OperationId::new(),
        uuid(1).into(),
        vm(),
        1,
        OperationAction::Lifecycle(cloning(VmCopyMode::Fast)),
        tokio::runtime::Handle::current(),
    );
    let mut creation = creation.record;
    assert!(matches!(
        journal.admit(&mut creation),
        Err(Error::PoolCreationConflict)
    ));
}

#[tokio::test]
async fn installed_template_firmware_cannot_change_but_secure_policy_can() {
    let (f, r, c) = fixture(Fault::None).await;
    let mut operation = creating();
    let VmLifecycleOperation::Create { firmware, .. } = &mut operation else {
        unreachable!()
    };
    *firmware = GuestFirmware::Bios;
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        execute(&f, dir.path(), operation).await.state,
        OperationState::Rejected
    );
    assert_eq!(count(&c, "Async.VM.copy"), 0);
    let mut operation = creating();
    let VmLifecycleOperation::Create { firmware, .. } = &mut operation else {
        unreachable!()
    };
    *firmware = GuestFirmware::SecureUefi;
    let result = execute(&f, dir.path(), operation).await;
    assert_eq!(result.state, OperationState::Succeeded, "{}", result.detail);
    let records = r.lock().unwrap();
    assert_eq!(
        records["OpaqueRef:VM:new"]["platform"]["secureboot"],
        "true"
    );
    assert_eq!(
        records["OpaqueRef:VM:new"]["HVM_boot_params"]["extra"],
        "preserve"
    );
}
fn bios_creation(r: &Records) -> VmLifecycleOperation {
    let mut records = r.lock().unwrap();
    let source = records.get_mut("OpaqueRef:VM:source").unwrap();
    source["HVM_boot_params"]["firmware"] = json!("bios");
    source["bios_strings"] = json!({});
    source["affinity"] = json!("OpaqueRef:NULL");
    records.insert("OpaqueRef:host:bios".into(),json!({"uuid":uuid(71),"bios_strings":{"bios-vendor":"Fixture vendor","system-serial-number":"synthetic-serial"}}));
    let mut operation = creating();
    let VmLifecycleOperation::Create {
        firmware,
        bios_host,
        ..
    } = &mut operation
    else {
        unreachable!()
    };
    *firmware = GuestFirmware::Bios;
    *bios_host = Some(uuid(71).into());
    operation
}
#[tokio::test]
async fn copied_bios_identity_and_affinity_are_verified_before_provisioning() {
    let (f, r, c) = fixture(Fault::None).await;
    let operation = bios_creation(&r);
    let dir = tempfile::tempdir().unwrap();
    let result = execute(&f, dir.path(), operation).await;
    assert_eq!(result.state, OperationState::Succeeded, "{}", result.detail);
    assert_eq!(
        r.lock().unwrap()["OpaqueRef:VM:new"]["affinity"],
        "OpaqueRef:host:bios"
    );
    let calls = c.lock().unwrap();
    let at = |m: &str| calls.iter().position(|r| r["method"] == m).unwrap();
    assert!(at("VM.copy_bios_strings") < at("Async.VM.provision"));
    assert!(
        !serde_json::to_string(&result)
            .unwrap()
            .contains("synthetic-serial")
    );
}
#[tokio::test]
async fn bios_copy_loss_or_wrong_affinity_does_not_provision_or_replay() {
    for fault in [Fault::LostBios, Fault::WrongBiosAffinity] {
        let (f, r, c) = fixture(fault).await;
        let operation = bios_creation(&r);
        let dir = tempfile::tempdir().unwrap();
        let result = execute(&f, dir.path(), operation).await;
        assert_eq!(
            result.state,
            OperationState::OutcomeUnknown,
            "{}",
            result.detail
        );
        assert!(result.created_vm.is_some());
        assert_eq!(count(&c, "VM.copy_bios_strings"), 1);
        assert_eq!(count(&c, "Async.VM.provision"), 0);
    }
}
#[tokio::test]
async fn existing_bios_identity_is_rejected_before_clone() {
    let (f, r, c) = fixture(Fault::None).await;
    let operation = bios_creation(&r);
    r.lock().unwrap().get_mut("OpaqueRef:VM:source").unwrap()["bios_strings"] =
        json!({"bios-vendor":"already set"});
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        execute(&f, dir.path(), operation).await.state,
        OperationState::Rejected
    );
    assert_eq!(count(&c, "Async.VM.copy"), 0);
}
#[test]
fn legacy_creation_intent_has_no_implicit_bios_copy() {
    let mut value = serde_json::to_value(creating()).unwrap();
    value.as_object_mut().unwrap().remove("bios_host");
    assert!(matches!(
        serde_json::from_value::<VmLifecycleOperation>(value).unwrap(),
        VmLifecycleOperation::Create {
            bios_host: None,
            ..
        }
    ));
}

#[tokio::test]
async fn late_bios_reply_preserves_recovery_and_acknowledgement_without_provisioning() {
    for acknowledge in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let (f, r, c) =
            fixture_with_recovery(Fault::None, Some((dir.path().into(), acknowledge))).await;
        let operation = bios_creation(&r);
        let result = execute(&f, dir.path(), operation).await;
        assert_eq!(
            result.state,
            if acknowledge {
                OperationState::AcknowledgedUnknown
            } else {
                OperationState::OutcomeUnknown
            },
            "{}",
            result.detail
        );
        assert!(result.created_vm.is_some());
        let plan = result.lifecycle_plan.as_ref().unwrap();
        let index = plan
            .steps
            .iter()
            .position(|s| matches!(s, crate::vm_lifecycle_backend::Step::CopyBios { .. }))
            .unwrap();
        assert!(plan.evidence[index].completed);
        assert_eq!(count(&c, "VM.copy_bios_strings"), 1);
        assert_eq!(count(&c, "Async.VM.provision"), 0);
        let mut changed = result.clone();
        changed.lifecycle_plan.as_mut().unwrap().bios_identity = Some(serde_json::Map::new());
        assert!(matches!(
            Journal::open(dir.path()).unwrap().save(&mut changed),
            Err(Error::JournalChanged)
        ));
    }
}
#[tokio::test]
async fn secure_boot_without_a_supported_readiness_api_is_refused_before_clone() {
    let (f, _, c) = fixture(Fault::NoSecureBootApi).await;
    let mut operation = creating();
    let VmLifecycleOperation::Create { firmware, .. } = &mut operation else {
        unreachable!()
    };
    *firmware = GuestFirmware::SecureUefi;
    let dir = tempfile::tempdir().unwrap();
    let result = execute(&f, dir.path(), operation).await;
    assert_eq!(result.state, OperationState::Rejected);
    assert_eq!(count(&c, "Async.VM.copy"), 0);
}

#[tokio::test]
async fn source_nic_requirements_are_checked_before_copy_without_denial_for_unchanged_network() {
    let (f, _, calls) = fixture(Fault::None).await;
    let d = tempfile::tempdir().unwrap();
    assert_eq!(
        execute(&f, d.path(), creating()).await.state,
        OperationState::Succeeded
    );
    let methods = calls
        .lock()
        .unwrap()
        .iter()
        .map(|r| {
            r["method"]
                .as_str()
                .unwrap()
                .trim_start_matches("Async.")
                .to_ascii_lowercase()
        })
        .filter(|m| m != "vif.move")
        .collect::<Vec<_>>();
    for unchanged in [false, true] {
        let (f, _, calls) = fixture_with_authority(
            Fault::None,
            None,
            json!({"is_local_superuser":false,"rbac_permissions":methods}),
        )
        .await;
        let d = tempfile::tempdir().unwrap();
        let mut operation = creating();
        if unchanged && let VmLifecycleOperation::Create { network, .. } = &mut operation {
            *network = uuid(6).into();
        }
        let result = execute(&f, d.path(), operation).await;
        if unchanged {
            assert_eq!(result.state, OperationState::Succeeded, "{}", result.detail);
        } else {
            assert!(
                result.detail.contains("RBAC_PERMISSION_DENIED"),
                "{}",
                result.detail
            );
            assert_eq!(count(&calls, "Async.VM.copy"), 0);
            assert_eq!(count(&calls, "Async.VM.clone"), 0);
            assert!(result.created_vm.is_none());
        }
    }
}
