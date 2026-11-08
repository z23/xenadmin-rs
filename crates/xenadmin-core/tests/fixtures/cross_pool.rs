use super::*;
use std::collections::BTreeMap;
use xenadmin_core::{
    Connection, CrossPoolMode, CrossPoolVmOperation, CrossPoolVmSelection, DiskPlacement,
    NicPlacement, OperationId, OperationState, ResourceOperation, ResourceOperationRecord,
    ResourceTarget,
};

fn id(n: u128) -> uuid::Uuid {
    uuid::Uuid::from_u128(n)
}
type Records = BTreeMap<String, Value>;
#[derive(Clone, Copy, Debug, PartialEq)]
enum Fault {
    None,
    LostSubmit,
    RejectSubmit,
    PollApi,
    Failure,
    BadReceiver,
    Collision,
    WrongResult,
    ChangedGraph,
    WrongSr,
    VerifyApi,
    Pending,
    DelaySubmit,
    RetainedSource,
}
struct Control {
    source: Records,
    destination: Records,
    fault: Fault,
    sent: bool,
    copy: bool,
}
struct Fixture {
    source: Mock,
    destination: Mock,
    control: Arc<Mutex<Control>>,
    entered: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
}
fn records(source: bool, live: bool) -> Records {
    let mut rows = BTreeMap::from([
        (
            "OpaqueRef:pool:pool".into(),
            json!({"uuid":id(if source{1}else{11}),"name_label":if source{"Source pool"}else{"Destination pool"},"master":"OpaqueRef:host:host","current_operations":{},"ha_enabled":false}),
        ),
        (
            "OpaqueRef:host:host".into(),
            json!({"uuid":id(if source{2}else{12}),"name_label":"Fixture host","address":if source{"192.0.2.10"}else{"192.0.2.20"},"enabled":true,"metrics":"OpaqueRef:host_metrics:host","current_operations":{}}),
        ),
        (
            "OpaqueRef:host_metrics:host".into(),
            json!({"uuid":id(if source{20}else{21}),"live":true}),
        ),
        (
            "OpaqueRef:SR:storage".into(),
            json!({"uuid":id(if source{4}else{14}),"name_label":"Fixture storage","type":"ext","content_type":"user","physical_size":100000000000u64,"physical_utilisation":0,"allowed_operations":["vdi_create"],"current_operations":{},"PBDs":["OpaqueRef:PBD:storage"]}),
        ),
        (
            "OpaqueRef:PBD:storage".into(),
            json!({"uuid":id(if source{40}else{41}),"host":"OpaqueRef:host:host","SR":"OpaqueRef:SR:storage","currently_attached":true}),
        ),
        (
            "OpaqueRef:network:network".into(),
            json!({"uuid":id(if source{5}else{15}),"name_label":"Fixture network","bridge":"xenbr0","PIFs":["OpaqueRef:PIF:network"],"current_operations":{}}),
        ),
        (
            "OpaqueRef:PIF:network".into(),
            json!({"uuid":id(if source{50}else{51}),"host":"OpaqueRef:host:host","network":"OpaqueRef:network:network","currently_attached":true,"IP":if source{"192.0.2.10"}else{"192.0.2.20"},"primary_address_type":"IPv4"}),
        ),
    ]);
    if source {
        rows.insert("OpaqueRef:VM:guest".into(),json!({"uuid":id(3),"name_label":"Portable guest","name_description":"Synthetic transfer fixture","is_a_template":false,"is_a_snapshot":false,"is_control_domain":false,"power_state":if live{"Running"}else{"Halted"},"resident_on":if live{"OpaqueRef:host:host"}else{"OpaqueRef:NULL"},"affinity":"OpaqueRef:host:host","VBDs":["OpaqueRef:VBD:disk"],"VIFs":["OpaqueRef:VIF:nic"],"snapshots":[],"VGPUs":[],"VUSBs":[],"VTPMs":[],"attached_PCIs":[],"suspend_VDI":"OpaqueRef:NULL","appliance":"OpaqueRef:NULL","protection_policy":"OpaqueRef:NULL","snapshot_schedule":"OpaqueRef:NULL","ha_always_run":false,"ha_restart_priority":"","allowed_operations":["migrate_send"],"blocked_operations":{},"current_operations":{},"VCPUs_max":"2","VCPUs_at_startup":"2","memory_static_min":"16777216","memory_static_max":"2147483648","memory_dynamic_min":"2147483648","memory_dynamic_max":"2147483648","HVM_boot_policy":"BIOS order","HVM_boot_params":{"firmware":"bios","order":"dc"},"platform":{}}));
        rows.insert("OpaqueRef:VDI:disk".into(),json!({"uuid":id(8),"name_label":"Fixture disk","name_description":"Synthetic","SR":"OpaqueRef:SR:storage","virtual_size":"1073741824","type":"user","managed":true,"missing":false,"read_only":false,"sharable":false,"is_a_snapshot":false,"cbt_enabled":false,"on_boot":"persist","VBDs":["OpaqueRef:VBD:disk"],"current_operations":{}}));
        rows.insert("OpaqueRef:VBD:disk".into(),json!({"uuid":id(6),"VM":"OpaqueRef:VM:guest","VDI":"OpaqueRef:VDI:disk","type":"Disk","mode":"RW","empty":false,"userdevice":"0","bootable":true,"other_config":{"owner":"true"},"current_operations":{}}));
        rows.insert("OpaqueRef:VIF:nic".into(),json!({"uuid":id(7),"VM":"OpaqueRef:VM:guest","network":"OpaqueRef:network:network","device":"0","MAC":"02:00:00:00:00:01","current_operations":{}}));
    }
    rows
}
fn apply(c: &mut Control) {
    let mut guest = c.source["OpaqueRef:VM:guest"].clone();
    let mut disk = c.source["OpaqueRef:VDI:disk"].clone();
    let mut block = c.source["OpaqueRef:VBD:disk"].clone();
    let mut nic = c.source["OpaqueRef:VIF:nic"].clone();
    if c.copy {
        guest["uuid"] = json!(id(103));
        block["uuid"] = json!(id(106));
        nic["uuid"] = json!(id(107));
        nic["MAC"] = json!("02:00:00:00:00:02");
    }
    disk["uuid"] = json!(id(108));
    if c.fault == Fault::WrongSr {
        c.destination
            .insert("OpaqueRef:SR:wrong".into(), json!({"uuid":id(99)}));
        disk["SR"] = json!("OpaqueRef:SR:wrong");
    }
    c.destination.extend([
        ("OpaqueRef:VM:guest".into(), guest),
        ("OpaqueRef:VDI:disk".into(), disk),
        ("OpaqueRef:VBD:disk".into(), block),
        ("OpaqueRef:VIF:nic".into(), nic),
    ]);
    if !c.copy
        && !matches!(
            c.fault,
            Fault::Failure | Fault::Pending | Fault::RetainedSource
        )
    {
        for key in [
            "OpaqueRef:VM:guest",
            "OpaqueRef:VDI:disk",
            "OpaqueRef:VBD:disk",
            "OpaqueRef:VIF:nic",
        ] {
            c.source.remove(key);
        }
    }
    c.source.insert("OpaqueRef:task:migrate".into(),json!({"uuid":id(60),"status":match c.fault {Fault::Failure=>"failure",Fault::Pending=>"pending",_=>"success"},"progress":0.5,"allowed_operations":["cancel"],"error_info":["IMPORT_ERROR"],"result":format!("<value><string>{}</string></value>",if c.fault==Fault::WrongResult{"OpaqueRef:VM:existing"}else{"OpaqueRef:VM:guest"})}));
    c.sent = true;
}
async fn fixture(fault: Fault, live: bool) -> Fixture {
    let mut destination = records(false, live);
    if matches!(fault, Fault::Collision | Fault::WrongResult) {
        let mut existing = records(true, false)["OpaqueRef:VM:guest"].clone();
        if fault == Fault::WrongResult {
            existing["uuid"] = json!(id(500));
        }
        destination.insert("OpaqueRef:VM:existing".into(), existing);
    }
    let control = Arc::new(Mutex::new(Control {
        source: records(true, live),
        destination,
        fault,
        sent: false,
        copy: false,
    }));
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let mut mocks = Vec::new();
    for source in [true, false] {
        let c = control.clone();
        let entered = entered.clone();
        let release = release.clone();
        mocks.push(mock_overridden(override_rpc(move|r:Value| {let c=c.clone();let entered=entered.clone();let release=release.clone();async move {
            let method=r["method"].as_str().unwrap();let p=&r["params"];
            if method=="event.from" {tokio::time::sleep(Duration::from_millis(20)).await;}
            if method=="Async.VM.migrate_send" && c.lock().unwrap().fault==Fault::DelaySubmit { entered.notify_one();release.notified().await; }
            let mut c=c.lock().unwrap();let fault=c.fault;let sent=c.sent;
            let rows=if source{&mut c.source}else{&mut c.destination};
            let reply=if method=="event.from" {
                json!({"token":"cross-pool-token","events":if p[2]=="" {rows.iter().map(|(reference,snapshot)|json!({"class":reference.split(':').nth(1).unwrap().to_ascii_lowercase(),"operation":"add","ref":reference,"snapshot":snapshot})).collect::<Vec<_>>()}else{vec![]}})
            } else if let Some(class)=method.strip_suffix(".get_all_records") {
                json!(rows.iter().filter(|(reference,_)|reference.starts_with(&format!("OpaqueRef:{class}:"))).collect::<BTreeMap<_,_>>())
            } else if let Some(class)=method.strip_suffix(".get_by_uuid") {
                match rows.iter().find(|(reference,v)| reference.starts_with(&format!("OpaqueRef:{class}:"))&&v["uuid"]==p[1]) {
                    Some((reference,_))=>json!(reference),None=>return Some(RpcReply::Json(json!({"error":{"message":"UUID_INVALID","data":[class]}})))
                }
            } else if method.ends_with(".get_record") {
                if (source && method=="task.get_record"&&fault==Fault::PollApi) || (!source&&method=="VM.get_record"&&sent&&fault==Fault::VerifyApi) {
                    return Some(RpcReply::Json(json!({"error":{"message":"SESSION_INVALID","data":["receiver-private-token"]}})));
                }
                match rows.get(p[1].as_str().unwrap()) {Some(v)=>v.clone(),None=>return Some(RpcReply::Json(json!({"error":{"message":"HANDLE_INVALID","data":[]}})))}
            } else {match method {
                "host.migrate_receive"=>{
                    assert!(!source);assert_eq!(p[1],"OpaqueRef:host:host");assert_eq!(p[2],"OpaqueRef:network:network");
                    json!({"host":"OpaqueRef:host:host","SM":format!("http://{}/services/SM?session_id=OpaqueRef:receiver-private-token",if fault==Fault::BadReceiver{"198.51.100.99"}else{"192.0.2.20"}),"xenops":"http://192.0.2.20/services/xenops?session_id=OpaqueRef:receiver-private-token","master":"http://192.0.2.20/","session_id":"OpaqueRef:receiver-private-token"})
                },
                "VM.assert_can_migrate"=>{
                    assert!(source);assert_eq!(p[1],"OpaqueRef:VM:guest");assert_eq!(p[4]["OpaqueRef:VDI:disk"],"OpaqueRef:SR:storage");assert_eq!(p[5]["OpaqueRef:VIF:nic"],"OpaqueRef:network:network");assert!(p[6].get("force").is_none());
                    if fault==Fault::ChangedGraph {c.source.get_mut("OpaqueRef:VM:guest").unwrap()["VCPUs_max"]=json!("4");}
                    Value::Null
                },
                "Async.VM.migrate_send"=>{
                    if fault==Fault::RejectSubmit {return Some(RpcReply::Json(json!({"error":{"message":"VM_BAD_POWER_STATE","data":[]}})));}
                    assert!(source);c.copy=p[6]["copy"]=="true";apply(&mut c);
                    if fault==Fault::LostSubmit{return Some(RpcReply::Drop);}
                    json!("OpaqueRef:task:migrate")
                },
                "task.cancel"=>{assert!(source);Value::Null},
                "task.destroy"=>{rows.remove(p[1].as_str().unwrap());Value::Null},
                _=>return None,
            }};
            Some(RpcReply::Json(json!({"result":reply})))
        }})).await);
    }
    let destination = mocks.pop().unwrap();
    let source = mocks.pop().unwrap();
    Fixture {
        source,
        destination,
        control,
        entered,
        release,
    }
}
async fn connect(mock: &Mock, backend: &Backend, readonly: bool) -> Connection {
    let mut options = ConnectionOptions::new(mock.endpoint.clone());
    options.read_only = readonly;
    let c = backend.connect(
        options,
        Credentials::new("fixture".into(), "fixture-password".into()),
    );
    let mut watch = c.watch();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !watch.borrow().is_current() {
            watch.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    c
}
fn selection(source: &Connection, dest: &Connection, mode: CrossPoolMode) -> CrossPoolVmSelection {
    CrossPoolVmSelection {
        source: source.latest().observe_vm(id(3).into()).unwrap(),
        destination: dest
            .latest()
            .observe_resource(ResourceTarget::Pool(id(11).into()))
            .unwrap(),
        operation: CrossPoolVmOperation {
            vm: id(3).into(),
            destination_pool: id(11).into(),
            destination_host: id(12).into(),
            expected_host: mode.live().then_some(id(2).into()),
            expected_affinity: Some(id(2).into()),
            transfer_network: id(15).into(),
            transfer_address: "192.0.2.20".into(),
            mode,
            disks: vec![DiskPlacement {
                vdi: id(8).into(),
                expected_sr: id(4).into(),
                destination_sr: id(14).into(),
            }],
            networks: vec![NicPlacement {
                vif: id(7).into(),
                expected_network: id(5).into(),
                destination_network: id(15).into(),
            }],
            host_transport_reviewed: true,
        },
    }
}
async fn done(op: &ResourceOperation) -> Arc<ResourceOperationRecord> {
    tokio::time::timeout(Duration::from_secs(5), async {
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
fn count(mock: &Mock, method: &str) -> usize {
    mock.methods
        .lock()
        .unwrap()
        .iter()
        .filter(|m| m.as_str() == method)
        .count()
}
fn privacy(dir: &std::path::Path, r: &ResourceOperationRecord) {
    let public = serde_json::to_string(r).unwrap();
    assert!(!public.contains("OpaqueRef:"));
    assert!(!public.contains("cross_pool_plan"));
    let sql = rusqlite::Connection::open_with_flags(
        dir.join("operations.sqlite3"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let payload: String = sql
        .query_row(
            "SELECT payload FROM resource_operations WHERE id=?1",
            [r.id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    for text in [public, payload] {
        assert!(!text.contains("receiver-private-token"));
        assert!(!text.contains("fixture-password"));
    }
}

#[tokio::test]
async fn move_and_copy_verify_destination_namespace_and_source_disposition() {
    for mode in [
        CrossPoolMode::MoveHalted,
        CrossPoolMode::MoveRunning,
        CrossPoolMode::CopyHalted,
    ] {
        let f = fixture(Fault::None, mode.live()).await;
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let source = connect(&f.source, &backend, false).await;
        let dest = connect(&f.destination, &backend, false).await;
        let op = source
            .transfer_vm_to_pool(OperationId::new(), selection(&source, &dest, mode), &dest)
            .unwrap();
        let result = done(&op).await;
        assert_eq!(
            result.state,
            OperationState::Succeeded,
            "{mode:?}: {}",
            result.detail
        );
        assert_eq!(
            result.transfer_created.vms,
            vec![id(if mode.copy() { 103 } else { 3 }).into()]
        );
        assert_eq!(result.transfer_created.disks, vec![id(108).into()]);
        assert_eq!(result.transfer_created.vbds.len(), 1);
        assert_eq!(result.transfer_created.vifs.len(), 1);
        assert_eq!(
            f.control
                .lock()
                .unwrap()
                .source
                .contains_key("OpaqueRef:VM:guest"),
            mode.copy()
        );
        assert!(result.applies_to_pool(id(1).into()) && result.applies_to_pool(id(11).into()));
        assert_eq!(count(&f.source, "Async.VM.migrate_send"), 1);
        assert_eq!(count(&f.destination, "Async.VM.migrate_send"), 0);
        privacy(dir.path(), &result);
        source.close().await;
        dest.close().await;
    }
}

#[tokio::test]
async fn source_and_receiver_changes_or_destination_collisions_prevent_transfer() {
    for fault in [Fault::BadReceiver, Fault::Collision, Fault::ChangedGraph] {
        let f = fixture(fault, false).await;
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let source = connect(&f.source, &backend, false).await;
        let dest = connect(&f.destination, &backend, false).await;
        let op = source
            .transfer_vm_to_pool(
                OperationId::new(),
                selection(&source, &dest, CrossPoolMode::MoveHalted),
                &dest,
            )
            .unwrap();
        let result = done(&op).await;
        assert_ne!(result.state, OperationState::Succeeded, "{fault:?}");
        assert_eq!(count(&f.source, "Async.VM.migrate_send"), 0);
        privacy(dir.path(), &result);
        source.close().await;
        dest.close().await;
    }
}

#[tokio::test]
async fn lost_or_failed_task_receipts_remain_unknown_and_rechecks_never_replay() {
    for fault in [
        Fault::LostSubmit,
        Fault::PollApi,
        Fault::Failure,
        Fault::WrongResult,
        Fault::WrongSr,
        Fault::RetainedSource,
    ] {
        let f = fixture(fault, false).await;
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let source = connect(&f.source, &backend, false).await;
        let dest = connect(&f.destination, &backend, false).await;
        let op = source
            .transfer_vm_to_pool(
                OperationId::new(),
                selection(&source, &dest, CrossPoolMode::MoveHalted),
                &dest,
            )
            .unwrap();
        let result = done(&op).await;
        assert_eq!(
            result.state,
            OperationState::OutcomeUnknown,
            "{fault:?}: {}",
            result.detail
        );
        let recheck = source.recheck_cross_pool_vm(result.id, &dest).unwrap();
        let after = done(&recheck).await;
        assert_eq!(
            after.state,
            OperationState::OutcomeUnknown,
            "{fault:?}: {}",
            after.detail
        );
        assert_eq!(count(&f.source, "Async.VM.migrate_send"), 1);
        assert_eq!(count(&f.destination, "host.migrate_receive"), 1);
        if matches!(fault, Fault::WrongSr | Fault::RetainedSource) {
            assert_eq!(after.transfer_created.vms, vec![id(3).into()]);
            if fault == Fault::RetainedSource {
                assert_eq!(after.transfer_created.disks, vec![id(108).into()]);
            }
        }
        privacy(dir.path(), &after);
        source.close().await;
        dest.close().await;
    }
}

#[tokio::test]
async fn recorded_completion_survives_removed_task_and_reconnect_without_a_second_transfer() {
    let f = fixture(Fault::VerifyApi, false).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let source = connect(&f.source, &backend, false).await;
    let dest = connect(&f.destination, &backend, false).await;
    let op = source
        .transfer_vm_to_pool(
            OperationId::new(),
            selection(&source, &dest, CrossPoolMode::MoveHalted),
            &dest,
        )
        .unwrap();
    let result = done(&op).await;
    assert_eq!(
        result.state,
        OperationState::OutcomeUnknown,
        "{}",
        result.detail
    );
    source.close().await;
    dest.close().await;
    {
        let mut c = f.control.lock().unwrap();
        c.fault = Fault::None;
        c.source.remove("OpaqueRef:task:migrate");
    }
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let source = connect(&f.source, &backend, false).await;
    let dest = connect(&f.destination, &backend, false).await;
    let recheck = source.recheck_cross_pool_vm(result.id, &dest).unwrap();
    let result = done(&recheck).await;
    assert_eq!(result.state, OperationState::Succeeded, "{}", result.detail);
    assert_eq!(count(&f.source, "Async.VM.migrate_send"), 1);
    privacy(dir.path(), &result);
    source.close().await;
    dest.close().await;
}

#[tokio::test]
async fn unsupported_source_devices_and_membership_fail_before_receiver_creation() {
    for (reference, key, value) in [
        ("OpaqueRef:VBD:disk", "other_config", json!({})),
        ("OpaqueRef:VDI:disk", "sharable", json!(true)),
        ("OpaqueRef:VDI:disk", "managed", json!(false)),
        (
            "OpaqueRef:VDI:disk",
            "VBDs",
            json!(["OpaqueRef:VBD:disk", "OpaqueRef:VBD:other"]),
        ),
        (
            "OpaqueRef:VM:guest",
            "snapshots",
            json!(["OpaqueRef:VM:snapshot"]),
        ),
        (
            "OpaqueRef:VM:guest",
            "appliance",
            json!("OpaqueRef:VM_appliance:group"),
        ),
        ("OpaqueRef:VM:guest", "ha_always_run", json!(true)),
        (
            "OpaqueRef:VM:guest",
            "snapshot_schedule",
            json!("OpaqueRef:VMSS:policy"),
        ),
        (
            "OpaqueRef:VM:guest",
            "blocked_operations",
            json!({"migrate_send":"Transfer restricted"}),
        ),
        ("OpaqueRef:VBD:disk", "type", json!("CD")),
    ] {
        let f = fixture(Fault::None, false).await;
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let source = connect(&f.source, &backend, false).await;
        let dest = connect(&f.destination, &backend, false).await;
        // Change fresh host records after inventory/review capture.
        let selection = selection(&source, &dest, CrossPoolMode::MoveHalted);
        f.control.lock().unwrap().source.get_mut(reference).unwrap()[key] = value;
        let op = source
            .transfer_vm_to_pool(OperationId::new(), selection, &dest)
            .unwrap();
        let result = done(&op).await;
        assert_eq!(
            result.state,
            OperationState::Rejected,
            "{reference}/{key}: {}",
            result.detail
        );
        assert_eq!(count(&f.destination, "host.migrate_receive"), 0);
        assert_eq!(count(&f.source, "Async.VM.migrate_send"), 0);
        source.close().await;
        dest.close().await;
    }
}

#[tokio::test]
async fn read_only_peer_foreign_backend_and_old_connection_observations_are_rejected_locally() {
    for readonly_source in [true, false] {
        let f = fixture(Fault::None, false).await;
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let source = connect(&f.source, &backend, readonly_source).await;
        let dest = connect(&f.destination, &backend, !readonly_source).await;
        assert!(matches!(
            source.transfer_vm_to_pool(
                OperationId::new(),
                selection(&source, &dest, CrossPoolMode::MoveHalted),
                &dest
            ),
            Err(xenadmin_core::Error::ReadOnly)
        ));
        assert_eq!(count(&f.destination, "host.migrate_receive"), 0);
        source.close().await;
        dest.close().await;
    }
    let f = fixture(Fault::None, false).await;
    let dir = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let foreign = Backend::with_state_directory(other.path().into()).unwrap();
    let source = connect(&f.source, &backend, false).await;
    let dest = connect(&f.destination, &foreign, false).await;
    assert!(matches!(
        source.transfer_vm_to_pool(
            OperationId::new(),
            selection(&source, &dest, CrossPoolMode::MoveHalted),
            &dest
        ),
        Err(xenadmin_core::Error::InvalidMigration)
    ));
    dest.close().await;
    let dest = connect(&f.destination, &backend, false).await;
    let stale = selection(&source, &dest, CrossPoolMode::MoveHalted);
    dest.close().await;
    let dest = connect(&f.destination, &backend, false).await;
    assert!(matches!(
        source.transfer_vm_to_pool(OperationId::new(), stale, &dest),
        Err(xenadmin_core::Error::Stale)
    ));
    assert_eq!(count(&f.destination, "host.migrate_receive"), 0);
    source.close().await;
    dest.close().await;
}

async fn history_state(connection: &Connection, id: OperationId, state: OperationState) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if connection
                .resource_history()
                .iter()
                .any(|r| r.id == id && r.state == state)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn late_task_receipts_survive_recovery_and_acknowledgement_without_old_worker_continuation() {
    for acknowledge in [false, true] {
        let f = fixture(Fault::DelaySubmit, false).await;
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let source = connect(&f.source, &backend, false).await;
        let dest = connect(&f.destination, &backend, false).await;
        let op = source
            .transfer_vm_to_pool(
                OperationId::new(),
                selection(&source, &dest, CrossPoolMode::MoveHalted),
                &dest,
            )
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), f.entered.notified())
            .await
            .unwrap();
        let recovered_backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let recovered = connect(&f.source, &recovered_backend, false).await;
        history_state(&recovered, op.latest().id, OperationState::OutcomeUnknown).await;
        if acknowledge {
            recovered
                .control_resource(
                    op.latest().id,
                    xenadmin_core::OperationControl::AcknowledgeUnknown,
                )
                .unwrap();
            history_state(
                &recovered,
                op.latest().id,
                OperationState::AcknowledgedUnknown,
            )
            .await;
        }
        let task_reads = count(&f.source, "task.get_record");
        f.release.notify_one();
        let result = done(&op).await;
        assert_eq!(
            result.state,
            if acknowledge {
                OperationState::AcknowledgedUnknown
            } else {
                OperationState::OutcomeUnknown
            }
        );
        assert_eq!(count(&f.source, "task.get_record"), task_reads);
        assert_eq!(count(&f.source, "Async.VM.migrate_send"), 1);
        let db = rusqlite::Connection::open(dir.path().join("operations.sqlite3")).unwrap();
        let receipt: Option<String> = db
            .query_row(
                "SELECT task FROM resource_operations WHERE id=?1",
                [op.latest().id.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(receipt.as_deref(), Some("OpaqueRef:task:migrate"));
        privacy(dir.path(), &result);
        source.close().await;
        dest.close().await;
        recovered.close().await;
    }
}

#[tokio::test]
async fn cancellation_remains_unknown_until_server_completion_and_recheck_does_not_repeat_it() {
    let f = fixture(Fault::Pending, false).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let source = connect(&f.source, &backend, false).await;
    let dest = connect(&f.destination, &backend, false).await;
    let op = source
        .transfer_vm_to_pool(
            OperationId::new(),
            selection(&source, &dest, CrossPoolMode::MoveHalted),
            &dest,
        )
        .unwrap();
    history_state(&source, op.latest().id, OperationState::Tracking).await;
    source
        .control_resource(
            op.latest().id,
            xenadmin_core::OperationControl::RequestCancellation,
        )
        .unwrap();
    // Startup recovery explicitly ends local tracking; the server task remains pending.
    let recovered_backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let recovered_source = connect(&f.source, &recovered_backend, false).await;
    let recovered_dest = connect(&f.destination, &recovered_backend, false).await;
    history_state(
        &recovered_source,
        op.latest().id,
        OperationState::OutcomeUnknown,
    )
    .await;
    let result = done(&op).await;
    assert_eq!(result.state, OperationState::OutcomeUnknown);
    let check = recovered_source
        .recheck_cross_pool_vm(op.latest().id, &recovered_dest)
        .unwrap();
    assert_eq!(done(&check).await.state, OperationState::OutcomeUnknown);
    let check = recovered_source
        .recheck_cross_pool_vm(op.latest().id, &recovered_dest)
        .unwrap();
    assert_eq!(done(&check).await.state, OperationState::OutcomeUnknown);
    assert_eq!(count(&f.source, "task.cancel"), 1);
    assert_eq!(count(&f.source, "Async.VM.migrate_send"), 1);
    assert_eq!(count(&f.destination, "host.migrate_receive"), 1);
    source.close().await;
    dest.close().await;
    recovered_source.close().await;
    recovered_dest.close().await;
}

#[tokio::test]
async fn explicit_submission_refusal_ends_without_adopting_or_deleting_destination_resources() {
    let f = fixture(Fault::RejectSubmit, false).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let source = connect(&f.source, &backend, false).await;
    let dest = connect(&f.destination, &backend, false).await;
    let op = source
        .transfer_vm_to_pool(
            OperationId::new(),
            selection(&source, &dest, CrossPoolMode::MoveHalted),
            &dest,
        )
        .unwrap();
    let result = done(&op).await;
    assert_eq!(
        result.state,
        OperationState::PartiallyCompleted,
        "{}",
        result.detail
    );
    assert!(result.transfer_created.vms.is_empty());
    assert!(!f.control.lock().unwrap().sent);
    assert_eq!(count(&f.source, "Async.VM.migrate_send"), 1);
    assert_eq!(count(&f.source, "task.get_record"), 0);
    source.close().await;
    dest.close().await;
}

#[tokio::test]
async fn tracking_deadline_stops_locally_without_repeating_or_claiming_cancellation() {
    let f = fixture(Fault::Pending, false).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let source = connect(&f.source, &backend, false).await;
    let dest = connect(&f.destination, &backend, false).await;
    let op = source
        .transfer_vm_to_pool(
            OperationId::new(),
            selection(&source, &dest, CrossPoolMode::MoveHalted),
            &dest,
        )
        .unwrap();
    history_state(&source, op.latest().id, OperationState::Tracking).await;
    // Advance this synthetic operation's persisted deadline instead of waiting two hours.
    let db = rusqlite::Connection::open(dir.path().join("operations.sqlite3")).unwrap();
    db.execute("UPDATE resource_operations SET payload=json_set(payload,'$.cross_pool_plan.expires_at_ms',1),revision=revision+1 WHERE id=?1",[op.latest().id.to_string()]).unwrap();
    let result = done(&op).await;
    assert_eq!(result.state, OperationState::OutcomeUnknown);
    assert!(!result.cancel_requested);
    assert!(!result.can_cancel);
    let check = source.recheck_cross_pool_vm(result.id, &dest).unwrap();
    assert_eq!(done(&check).await.state, OperationState::OutcomeUnknown);
    assert_eq!(count(&f.source, "Async.VM.migrate_send"), 1);
    assert_eq!(count(&f.source, "task.cancel"), 0);
    source.close().await;
    dest.close().await;
}
