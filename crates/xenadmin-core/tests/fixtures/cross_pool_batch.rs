//! A batch of cross-pool moves between two fixture pools. The source pool holds two
//! guests: the first halted, the second running. Each has one disk and one NIC.
use super::*;
use std::collections::BTreeMap;
use xenadmin_core::{
    BatchVmWorkflow, Connection, CrossPoolBatchSelection, CrossPoolMode, CrossPoolVmOperation,
    DiskPlacement, NicPlacement, OperationControl, OperationId, OperationState, ResourceAction,
    ResourceCommand, ResourceOperation, ResourceOperationRecord, ResourceTarget, VmWorkflow,
    VmWorkflowBatch,
};

const CONFLICT: &str =
    "Another VM or infrastructure operation in this pool is running or unresolved";

fn id(n: u128) -> uuid::Uuid {
    uuid::Uuid::from_u128(n)
}
type Records = BTreeMap<String, Value>;
#[derive(Clone, Copy, Debug, PartialEq)]
enum Fault {
    None,
    /// The hosts run the first guest's transfer, and its reply never arrives.
    LostFirstSubmit,
    /// The first guest's transfer waits at the source host until it is released.
    DelayFirstSubmit,
    /// The session on the source pool has a role that may not send a guest.
    SourceMayNotSend,
    /// The session on the destination pool has a role that may not receive one.
    DestinationMayNotReceive,
}
struct Control {
    source: Records,
    destination: Records,
    fault: Fault,
    /// The guests whose transfer the source pool was asked for, in that order.
    sent: Vec<u128>,
    hold_poll: bool,
    /// How often the destination's guests were listed, and the listing to hold. A move's
    /// preparation lists them once, as its last read.
    listings: usize,
    hold_listing: Option<usize>,
}
struct Fixture {
    source: Mock,
    destination: Mock,
    control: Arc<Mutex<Control>>,
    entered: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
}
fn records(source: bool) -> Records {
    let mut rows = BTreeMap::from([
        (
            "OpaqueRef:pool:pool".into(),
            json!({"uuid":id(if source{1}else{11}),"name_label":if source{"Source pool"}else{"Destination pool"},"name_description":"","master":"OpaqueRef:host:host","current_operations":{},"ha_enabled":false}),
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
        for g in [1u128, 2] {
            let live = g == 2;
            rows.insert(format!("OpaqueRef:VM:guest{g}"),json!({"uuid":id(300+g),"name_label":format!("Batch guest {g}"),"name_description":"Synthetic batch fixture","is_a_template":false,"is_a_snapshot":false,"is_control_domain":false,"power_state":if live{"Running"}else{"Halted"},"resident_on":if live{"OpaqueRef:host:host"}else{"OpaqueRef:NULL"},"affinity":"OpaqueRef:host:host","VBDs":[format!("OpaqueRef:VBD:disk{g}")],"VIFs":[format!("OpaqueRef:VIF:nic{g}")],"snapshots":[],"VGPUs":[],"VUSBs":[],"VTPMs":[],"attached_PCIs":[],"suspend_VDI":"OpaqueRef:NULL","appliance":"OpaqueRef:NULL","protection_policy":"OpaqueRef:NULL","snapshot_schedule":"OpaqueRef:NULL","ha_always_run":false,"ha_restart_priority":"","allowed_operations":["migrate_send"],"blocked_operations":{},"current_operations":{},"VCPUs_max":"2","VCPUs_at_startup":"2","memory_static_min":"16777216","memory_static_max":"2147483648","memory_dynamic_min":"2147483648","memory_dynamic_max":"2147483648","HVM_boot_policy":"BIOS order","HVM_boot_params":{"firmware":"bios","order":"dc"},"platform":{}}));
            rows.insert(format!("OpaqueRef:VDI:disk{g}"),json!({"uuid":id(800+g),"name_label":"Fixture disk","name_description":"Synthetic","SR":"OpaqueRef:SR:storage","virtual_size":"1073741824","type":"user","managed":true,"missing":false,"read_only":false,"sharable":false,"is_a_snapshot":false,"cbt_enabled":false,"on_boot":"persist","VBDs":[format!("OpaqueRef:VBD:disk{g}")],"current_operations":{}}));
            rows.insert(format!("OpaqueRef:VBD:disk{g}"),json!({"uuid":id(600+g),"VM":format!("OpaqueRef:VM:guest{g}"),"VDI":format!("OpaqueRef:VDI:disk{g}"),"type":"Disk","mode":"RW","empty":false,"userdevice":"0","bootable":true,"other_config":{"owner":"true"},"current_operations":{}}));
            rows.insert(format!("OpaqueRef:VIF:nic{g}"),json!({"uuid":id(700+g),"VM":format!("OpaqueRef:VM:guest{g}"),"network":"OpaqueRef:network:network","device":"0","MAC":format!("02:00:00:00:00:0{g}"),"current_operations":{}}));
        }
    }
    rows
}
/// What the hosts do for one guest's transfer: the guest, its attachment and its NIC keep
/// their identities on the destination, the disk gets a new one, and the source keeps none.
fn apply(c: &mut Control, g: u128) {
    let mut disk = c.source.remove(&format!("OpaqueRef:VDI:disk{g}")).unwrap();
    disk["uuid"] = json!(id(1800 + g));
    c.destination.insert(format!("OpaqueRef:VDI:disk{g}"), disk);
    for key in [
        format!("OpaqueRef:VM:guest{g}"),
        format!("OpaqueRef:VBD:disk{g}"),
        format!("OpaqueRef:VIF:nic{g}"),
    ] {
        let row = c.source.remove(&key).unwrap();
        c.destination.insert(key, row);
    }
    c.source.insert(format!("OpaqueRef:task:migrate{g}"),json!({"uuid":id(60+g),"status":"success","progress":1.0,"allowed_operations":[],"error_info":[],"result":format!("<value><string>OpaqueRef:VM:guest{g}</string></value>")}));
    c.sent.push(g);
}
async fn fixture(fault: Fault) -> Fixture {
    let control = Arc::new(Mutex::new(Control {
        source: records(true),
        destination: records(false),
        fault,
        sent: Vec::new(),
        hold_poll: false,
        listings: 0,
        hold_listing: None,
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
            if method=="session.get_record" {
                let granted = match c.lock().unwrap().fault {
                    Fault::SourceMayNotSend if source => json!(["session.logout","host.migrate_receive"]),
                    Fault::DestinationMayNotReceive if !source => json!(["session.logout","vm.migrate_send"]),
                    _ => return None,
                };
                return Some(RpcReply::Json(json!({"result":{"is_local_superuser":false,"rbac_permissions":granted}})));
            }
            let hold_poll = source && method=="task.get_record" && { let mut control=c.lock().unwrap();std::mem::take(&mut control.hold_poll) };
            if hold_poll { entered.notify_one(); release.notified().await; }
            let hold_listing = !source && method=="VM.get_all_records" && { let mut control=c.lock().unwrap();control.listings+=1;control.hold_listing==Some(control.listings) };
            if hold_listing { entered.notify_one(); release.notified().await; }
            if source && method=="Async.VM.migrate_send" && p[1]=="OpaqueRef:VM:guest1" && c.lock().unwrap().fault==Fault::DelayFirstSubmit { entered.notify_one();release.notified().await; }
            let mut c=c.lock().unwrap();let fault=c.fault;
            let rows=if source{&mut c.source}else{&mut c.destination};
            let reply=if method=="event.from" {
                json!({"token":"cross-pool-batch-token","events":if p[2]=="" {rows.iter().map(|(reference,snapshot)|json!({"class":reference.split(':').nth(1).unwrap().to_ascii_lowercase(),"operation":"add","ref":reference,"snapshot":snapshot})).collect::<Vec<_>>()}else{vec![]}})
            } else if let Some(class)=method.strip_suffix(".get_all_records") {
                json!(rows.iter().filter(|(reference,_)|reference.starts_with(&format!("OpaqueRef:{class}:"))).collect::<BTreeMap<_,_>>())
            } else if let Some(class)=method.strip_suffix(".get_by_uuid") {
                match rows.iter().find(|(reference,v)| reference.starts_with(&format!("OpaqueRef:{class}:"))&&v["uuid"]==p[1]) {
                    Some((reference,_))=>json!(reference),None=>return Some(RpcReply::Json(json!({"error":{"message":"UUID_INVALID","data":[class]}})))
                }
            } else if method.ends_with(".get_record") {
                match rows.get(p[1].as_str().unwrap()) {Some(v)=>v.clone(),None=>return Some(RpcReply::Json(json!({"error":{"message":"HANDLE_INVALID","data":[]}})))}
            } else if method.ends_with(".set_name_label") {
                rows.get_mut(p[1].as_str().unwrap()).unwrap()["name_label"]=p[2].clone();Value::Null
            } else {match method {
                "host.migrate_receive"=>{
                    assert!(!source);assert_eq!(p[1],"OpaqueRef:host:host");assert_eq!(p[2],"OpaqueRef:network:network");
                    json!({"host":"OpaqueRef:host:host","SM":"http://192.0.2.20/services/SM?session_id=OpaqueRef:receiver-private-token","xenops":"http://192.0.2.20/services/xenops?session_id=OpaqueRef:receiver-private-token","master":"http://192.0.2.20/","session_id":"OpaqueRef:receiver-private-token"})
                },
                "VM.assert_can_migrate"|"Async.VM.migrate_send"=>{
                    // Each guest goes with its own disk and NIC, mapped as reviewed, live
                    // only where it was running, and never as a copy.
                    assert!(source);
                    let g:u128=p[1].as_str().unwrap().strip_prefix("OpaqueRef:VM:guest").unwrap().parse().unwrap();
                    assert_eq!(p[3],json!(g==2));
                    assert_eq!(p[4],json!({format!("OpaqueRef:VDI:disk{g}"):"OpaqueRef:SR:storage"}));
                    assert_eq!(p[5],json!({format!("OpaqueRef:VIF:nic{g}"):"OpaqueRef:network:network"}));
                    assert_eq!(p[6],json!({}));
                    if method=="VM.assert_can_migrate" {Value::Null} else {
                        apply(&mut c,g);
                        if fault==Fault::LostFirstSubmit&&g==1 {return Some(RpcReply::Drop);}
                        json!(format!("OpaqueRef:task:migrate{g}"))
                    }
                },
                "task.cancel"=>Value::Null,
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
    options.cross_pool_cleanup_wait = Duration::from_secs(3);
    options.read_only = readonly;
    let c = backend.connect(
        options,
        Credentials::new("fixture".into(), "fixture-password".into()),
    );
    let mut watch = c.watch();
    tokio::time::timeout(deadline(5), async {
        while !watch.borrow().is_current() {
            watch.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    c
}
/// The reviewed move of one fixture guest: its disk to the destination repository and its
/// NIC to the destination network. The second guest is running and moves live.
fn operation(g: u128) -> CrossPoolVmOperation {
    let live = g == 2;
    CrossPoolVmOperation {
        vm: id(300 + g).into(),
        destination_pool: id(11).into(),
        destination_host: id(12).into(),
        expected_host: live.then_some(id(2).into()),
        expected_affinity: Some(id(2).into()),
        transfer_network: id(15).into(),
        transfer_address: "192.0.2.20".into(),
        mode: if live {
            CrossPoolMode::MoveRunning
        } else {
            CrossPoolMode::MoveHalted
        },
        disks: vec![DiskPlacement {
            vdi: id(800 + g).into(),
            expected_sr: id(4).into(),
            destination_sr: id(14).into(),
        }],
        networks: vec![NicPlacement {
            vif: id(700 + g).into(),
            expected_network: id(5).into(),
            destination_network: id(15).into(),
        }],
        host_transport_reviewed: true,
    }
}
fn selection(source: &Connection, dest: &Connection) -> CrossPoolBatchSelection {
    CrossPoolBatchSelection {
        source: source
            .latest()
            .observe_resource(ResourceTarget::Pool(id(1).into()))
            .unwrap(),
        destination: dest
            .latest()
            .observe_resource(ResourceTarget::Pool(id(11).into()))
            .unwrap(),
        batch: VmWorkflowBatch {
            items: vec![
                BatchVmWorkflow::move_to_pool(operation(1)),
                BatchVmWorkflow::move_to_pool(operation(2)),
            ],
        },
    }
}
async fn done(op: &ResourceOperation) -> Arc<ResourceOperationRecord> {
    tokio::time::timeout(deadline(30), async {
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
/// Nothing that changes a host: no receiver, no compatibility assertion with one, no
/// transfer and no cancellation.
fn nothing_sent(f: &Fixture) {
    assert_eq!(count(&f.destination, "host.migrate_receive"), 0);
    assert_eq!(count(&f.source, "VM.assert_can_migrate"), 0);
    assert_eq!(count(&f.source, "Async.VM.migrate_send"), 0);
    assert_eq!(count(&f.source, "task.cancel"), 0);
    assert!(f.control.lock().unwrap().sent.is_empty());
}
fn stored(dir: &std::path::Path) -> Vec<(String, String)> {
    let sql = rusqlite::Connection::open_with_flags(
        dir.join("operations.sqlite3"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let mut query = sql
        .prepare("SELECT id,payload FROM resource_operations ORDER BY rowid")
        .unwrap();
    query
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}
/// Receiver credentials and the login password are in no stored row and in no public record.
fn privacy(dir: &std::path::Path, records: &[Arc<ResourceOperationRecord>]) {
    for text in stored(dir)
        .into_iter()
        .map(|(_, payload)| payload)
        .chain(records.iter().map(|r| serde_json::to_string(&**r).unwrap()))
    {
        assert!(!text.contains("receiver-private-token"));
        assert!(!text.contains("fixture-password"));
    }
    for record in records {
        let public = serde_json::to_string(&**record).unwrap();
        assert!(!public.contains("OpaqueRef:"));
        assert!(!public.contains("cross_pool_plan"));
        assert!(!public.contains("workflow_batch_plan"));
    }
}
fn history(connection: &Connection, id: OperationId) -> Option<Arc<ResourceOperationRecord>> {
    connection
        .resource_history()
        .iter()
        .find(|r| r.id == id)
        .cloned()
        .map(Arc::new)
}
async fn history_where<F: Fn(&ResourceOperationRecord) -> bool>(
    connection: &Connection,
    id: OperationId,
    ready: F,
) -> Arc<ResourceOperationRecord> {
    tokio::time::timeout(deadline(10), async {
        loop {
            if let Some(record) = history(connection, id).filter(|r| ready(r)) {
                return record;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}
/// A small reviewed change to a pool, used to ask whether the pool is reserved: it is
/// refused at admission while it is, and otherwise runs to a verified end.
async fn rename_pool(
    connection: &Connection,
    pool: u128,
    from: &str,
    to: &str,
) -> Arc<ResourceOperationRecord> {
    let op = connection
        .submit_resource(
            OperationId::new(),
            ResourceCommand::RenameMetadata {
                target: connection
                    .latest()
                    .observe_resource(ResourceTarget::Pool(id(pool).into()))
                    .unwrap(),
                expected_name: from.into(),
                expected_description: String::new(),
                name: to.into(),
                description: String::new(),
            },
        )
        .unwrap();
    done(&op).await
}

#[tokio::test]
async fn a_batch_moves_a_halted_and_a_running_guest_to_another_pool_one_after_another() {
    let f = fixture(Fault::None).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let source = connect(&f.source, &backend, false).await;
    let dest = connect(&f.destination, &backend, false).await;
    let reviewed = selection(&source, &dest);
    let op = source
        .move_vms_to_pool(OperationId::new(), reviewed.clone(), &dest)
        .unwrap();
    let result = done(&op).await;
    assert_eq!(result.state, OperationState::Succeeded, "{}", result.detail);
    assert_eq!(result.action.label(), "Move selected VMs to another pool");
    assert!(result.action.spans_pools());
    assert!(result.applies_to_pool(id(1).into()) && result.applies_to_pool(id(11).into()));
    assert_eq!(result.peer_pool(), Some(id(11).into()));
    assert_eq!(result.workflow_batch.len(), 2);
    // One transfer per guest, in the reviewed order, each with a receiver of its own.
    assert_eq!(f.control.lock().unwrap().sent, vec![1, 2]);
    assert_eq!(count(&f.source, "Async.VM.migrate_send"), 2);
    assert_eq!(count(&f.source, "VM.assert_can_migrate"), 2);
    assert_eq!(count(&f.destination, "host.migrate_receive"), 2);
    assert_eq!(count(&f.destination, "Async.VM.migrate_send"), 0);
    assert_eq!(count(&f.source, "task.cancel"), 0);
    let mut records = vec![result.clone()];
    for (index, item) in reviewed.batch.items.iter().enumerate() {
        let g = index as u128 + 1;
        let outcome = &result.workflow_batch[index];
        assert_eq!(outcome.vm, id(300 + g).into());
        assert_eq!(outcome.state, Some(OperationState::Succeeded));
        // The child is the single move of exactly the reviewed operation, under the batch.
        let child = history_where(&source, outcome.child, |r| r.state.terminal()).await;
        assert_eq!(child.batch_parent, Some(result.id));
        let VmWorkflow::MoveToPool(operation) = &item.workflow else {
            panic!("a cross-pool item")
        };
        assert_eq!(child.action, ResourceAction::CrossPoolVm(operation.clone()));
        assert_eq!(child.state, OperationState::Succeeded, "{}", child.detail);
        assert_eq!(child.transfer_created.vms, vec![id(300 + g).into()]);
        assert_eq!(child.transfer_created.disks, vec![id(1800 + g).into()]);
        assert_eq!(child.transfer_created.vbds, vec![id(600 + g).into()]);
        assert_eq!(child.transfer_created.vifs, vec![id(700 + g).into()]);
        records.push(child);
    }
    {
        let c = f.control.lock().unwrap();
        for g in [1, 2] {
            assert!(!c.source.contains_key(&format!("OpaqueRef:VM:guest{g}")));
            assert!(
                c.destination
                    .contains_key(&format!("OpaqueRef:VM:guest{g}"))
            );
        }
        // The halted guest is still halted and the running one still running.
        assert_eq!(
            c.destination["OpaqueRef:VM:guest1"]["power_state"],
            "Halted"
        );
        assert_eq!(
            c.destination["OpaqueRef:VM:guest2"]["power_state"],
            "Running"
        );
    }
    // The destination pool's connection shows the batch as well.
    history_where(&dest, result.id, |r| r.state == OperationState::Succeeded).await;
    assert_eq!(stored(dir.path()).len(), 3);
    privacy(dir.path(), &records);
    source.close().await;
    dest.close().await;
}

#[tokio::test]
async fn one_guest_failing_the_single_move_preflight_refuses_the_whole_selection_before_anything_is_sent()
 {
    for (pool_is_source, reference, key, value, refusal) in [
        (
            true,
            "OpaqueRef:VDI:disk2",
            "sharable",
            json!(true),
            "Shared, snapshot, CBT or reset-on-boot disks cannot use this transfer",
        ),
        (
            true,
            "OpaqueRef:VM:guest2",
            "ha_always_run",
            json!(true),
            "Disable VM HA protection separately before transfer",
        ),
        (
            true,
            "OpaqueRef:VM:guest2",
            "snapshots",
            json!(["OpaqueRef:VM:snapshot"]),
            "Snapshot chains or specialized guest devices require another workflow",
        ),
        // The second guest was halted after the review: its live move is no longer the
        // reviewed one.
        (
            true,
            "OpaqueRef:VM:guest2",
            "power_state",
            json!("Halted"),
            "The VM, host, storage or network changed; refresh and review the migration again",
        ),
        // Each disk fits the destination repository alone; the two together do not.
        (
            false,
            "OpaqueRef:SR:storage",
            "physical_size",
            json!(1610612736u64),
            "Destination storage does not report enough space for the disks of the whole selection",
        ),
        // Neither fits: the single move's own check refuses the first guest.
        (
            false,
            "OpaqueRef:SR:storage",
            "physical_size",
            json!(1024),
            "Destination storage does not report enough space for the reviewed disks",
        ),
    ] {
        let f = fixture(Fault::None).await;
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let source = connect(&f.source, &backend, false).await;
        let dest = connect(&f.destination, &backend, false).await;
        // The hosts change after the review was captured.
        let reviewed = selection(&source, &dest);
        {
            let mut c = f.control.lock().unwrap();
            let rows = if pool_is_source {
                &mut c.source
            } else {
                &mut c.destination
            };
            rows.get_mut(reference).unwrap()[key] = value;
        }
        let op = source
            .move_vms_to_pool(OperationId::new(), reviewed, &dest)
            .unwrap();
        let result = done(&op).await;
        assert_eq!(
            result.state,
            OperationState::Rejected,
            "{reference}/{key}: {}",
            result.detail
        );
        assert_eq!(result.detail, refusal, "{reference}/{key}");
        nothing_sent(&f);
        // No guest was started: the batch has no child, in its record or in the journal.
        assert!(result.workflow_batch.iter().all(|r| r.state.is_none()));
        assert_eq!(stored(dir.path()).len(), 1);
        // A refused batch holds neither pool.
        let renamed = rename_pool(&dest, 11, "Destination pool", "Destination renamed").await;
        assert_eq!(
            renamed.state,
            OperationState::Succeeded,
            "{}",
            renamed.detail
        );
        source.close().await;
        dest.close().await;
    }
}

/// A role that is known not to allow the source pool to send a guest, or the destination
/// pool to prepare a receiver, refuses the batch before either pool is asked for anything.
#[tokio::test]
async fn a_role_that_may_not_send_or_receive_refuses_the_batch_before_anything_is_sent() {
    for (fault, refusal) in [
        (
            Fault::SourceMayNotSend,
            "The signed-in role does not allow VM.migrate_send (RBAC_PERMISSION_DENIED); the request was not sent",
        ),
        (
            Fault::DestinationMayNotReceive,
            "The signed-in role does not allow host.migrate_receive on the destination pool (RBAC_PERMISSION_DENIED); the request was not sent",
        ),
    ] {
        let f = fixture(fault).await;
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let source = connect(&f.source, &backend, false).await;
        let dest = connect(&f.destination, &backend, false).await;
        let op = source
            .move_vms_to_pool(OperationId::new(), selection(&source, &dest), &dest)
            .unwrap();
        let result = done(&op).await;
        assert_eq!(result.state, OperationState::Rejected, "{fault:?}");
        assert_eq!(result.detail, refusal, "{fault:?}");
        nothing_sent(&f);
        assert!(result.workflow_batch.iter().all(|r| r.state.is_none()));
        assert_eq!(stored(dir.path()).len(), 1);
        source.close().await;
        dest.close().await;
    }
}

#[tokio::test]
async fn a_batch_needs_the_transport_acknowledgement_one_destination_and_moves_only() {
    let f = fixture(Fault::None).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let source = connect(&f.source, &backend, false).await;
    let dest = connect(&f.destination, &backend, false).await;
    let change = |edit: &dyn Fn(&mut CrossPoolVmOperation)| {
        let mut reviewed = selection(&source, &dest);
        let VmWorkflow::MoveToPool(operation) = &mut reviewed.batch.items[1].workflow else {
            panic!("a cross-pool item")
        };
        edit(operation);
        reviewed
    };
    // The acknowledgement is given once for the batch and must cover every guest in it.
    let mut unacknowledged = selection(&source, &dest);
    for item in &mut unacknowledged.batch.items {
        let VmWorkflow::MoveToPool(operation) = &mut item.workflow else {
            panic!("a cross-pool item")
        };
        operation.host_transport_reviewed = false;
    }
    let mut halted_as_running = selection(&source, &dest);
    halted_as_running.batch.items[0].expected_state = PowerState::Running;
    let mut suspended = selection(&source, &dest);
    suspended.batch.items[0].expected_state = PowerState::Suspended;
    for (label, refused) in [
        ("no acknowledgement", unacknowledged),
        (
            "one guest without it",
            change(&|o| o.host_transport_reviewed = false),
        ),
        (
            "a second host",
            change(&|o| o.destination_host = id(13).into()),
        ),
        (
            "a second pool",
            change(&|o| o.destination_pool = id(99).into()),
        ),
        (
            "a second address",
            change(&|o| o.transfer_address = "192.0.2.21".into()),
        ),
        (
            "a second migration network",
            change(&|o| o.transfer_network = id(16).into()),
        ),
        (
            "a copy",
            change(&|o| {
                o.mode = CrossPoolMode::CopyHalted;
                o.expected_host = None;
            }),
        ),
        (
            "a disk named twice",
            change(&|o| o.disks[0].vdi = id(801).into()),
        ),
        ("a halted guest reviewed as running", halted_as_running),
        ("a suspended guest", suspended),
    ] {
        assert!(
            source
                .move_vms_to_pool(OperationId::new(), refused, &dest)
                .is_err(),
            "{label}"
        );
    }
    // The ordinary submissions carry no second connection and take no such batch.
    let reviewed = selection(&source, &dest);
    assert!(matches!(
        source.submit_resource(
            OperationId::new(),
            ResourceCommand::VmWorkflowBatch {
                pool: reviewed.source,
                selection: reviewed.batch.clone(),
            },
        ),
        Err(xenadmin_core::Error::InvalidResourceCommand)
    ));
    assert!(matches!(
        source.submit_vm_workflow_batch(
            OperationId::new(),
            reviewed.source,
            reviewed.batch.clone(),
            Vec::new(),
        ),
        Err(xenadmin_core::Error::InvalidResourceCommand)
    ));
    tokio::time::sleep(Duration::from_millis(100)).await;
    nothing_sent(&f);
    assert!(source.resource_history().is_empty());
    source.close().await;
    dest.close().await;
}

#[tokio::test]
async fn stop_finishes_the_first_move_and_never_starts_the_second() {
    let f = fixture(Fault::None).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let source = connect(&f.source, &backend, false).await;
    let dest = connect(&f.destination, &backend, false).await;
    f.control.lock().unwrap().hold_poll = true;
    let op = source
        .move_vms_to_pool(OperationId::new(), selection(&source, &dest), &dest)
        .unwrap();
    // The first guest's transfer is with the hosts when Stop is asked for.
    tokio::time::timeout(deadline(10), f.entered.notified())
        .await
        .unwrap();
    let id = op.latest().id;
    source
        .control_resource(id, OperationControl::RequestCancellation)
        .unwrap();
    history_where(&source, id, |r| r.cancel_requested).await;
    f.release.notify_one();
    let result = done(&op).await;
    assert_eq!(
        result.state,
        OperationState::PartiallyCompleted,
        "{}",
        result.detail
    );
    assert!(
        result
            .detail
            .contains("1 of 2 VM workflows verified; 1 not started")
    );
    assert_eq!(
        result.workflow_batch[0].state,
        Some(OperationState::Succeeded)
    );
    assert_eq!(result.workflow_batch[1].state, None);
    // Stop asked the hosts to cancel nothing, and the second guest was not touched.
    assert_eq!(f.control.lock().unwrap().sent, vec![1]);
    assert_eq!(count(&f.source, "Async.VM.migrate_send"), 1);
    assert_eq!(count(&f.destination, "host.migrate_receive"), 1);
    assert_eq!(count(&f.source, "task.cancel"), 0);
    assert!(
        f.control
            .lock()
            .unwrap()
            .source
            .contains_key("OpaqueRef:VM:guest2")
    );
    assert_eq!(stored(dir.path()).len(), 2);
    source.close().await;
    dest.close().await;
}

#[tokio::test]
async fn both_pools_are_reserved_while_a_batch_runs_and_released_when_it_ends() {
    let f = fixture(Fault::None).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let source = connect(&f.source, &backend, false).await;
    let dest = connect(&f.destination, &backend, false).await;
    f.control.lock().unwrap().hold_poll = true;
    let op = source
        .move_vms_to_pool(OperationId::new(), selection(&source, &dest), &dest)
        .unwrap();
    tokio::time::timeout(deadline(10), f.entered.notified())
        .await
        .unwrap();
    // While the first guest is in transfer, neither pool admits another change.
    for (connection, pool, name) in [(&source, 1, "Source pool"), (&dest, 11, "Destination pool")] {
        let refused = rename_pool(connection, pool, name, "Renamed during the batch").await;
        assert_eq!(refused.state, OperationState::Rejected, "{name}");
        assert_eq!(refused.detail, CONFLICT, "{name}");
    }
    // A second batch between the same pools is refused the same way.
    let second = source
        .move_vms_to_pool(OperationId::new(), selection(&source, &dest), &dest)
        .unwrap();
    let second = done(&second).await;
    assert_eq!(second.state, OperationState::Rejected);
    assert_eq!(second.detail, CONFLICT);
    f.release.notify_one();
    let result = done(&op).await;
    assert_eq!(result.state, OperationState::Succeeded, "{}", result.detail);
    assert_eq!(count(&f.source, "Async.VM.migrate_send"), 2);
    // With the batch ended, each pool admits a change again.
    for (connection, pool, name) in [(&source, 1, "Source pool"), (&dest, 11, "Destination pool")] {
        let renamed = rename_pool(connection, pool, name, "Renamed after the batch").await;
        assert_eq!(
            renamed.state,
            OperationState::Succeeded,
            "{name}: {}",
            renamed.detail
        );
    }
    source.close().await;
    dest.close().await;
}

/// The client restarts while the first guest's transfer is with the hosts. Nothing is
/// replayed, the second guest is never started, and a recheck with both pools connected
/// reads what the first transfer did.
#[tokio::test]
async fn an_interrupted_batch_is_read_by_recheck_and_never_replays_or_continues() {
    let f = fixture(Fault::DelayFirstSubmit).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let source = connect(&f.source, &backend, false).await;
    let dest = connect(&f.destination, &backend, false).await;
    let op = source
        .move_vms_to_pool(OperationId::new(), selection(&source, &dest), &dest)
        .unwrap();
    tokio::time::timeout(deadline(10), f.entered.notified())
        .await
        .unwrap();
    let id = op.latest().id;
    // A second instance on the same state directory stands for the restarted client. The
    // destination pool is the first to come back: either pool's recovery fences the batch.
    let restarted = Backend::with_state_directory(dir.path().into()).unwrap();
    let recovered_dest = connect(&f.destination, &restarted, false).await;
    let unknown = history_where(&recovered_dest, id, |r| {
        r.state == OperationState::OutcomeUnknown
    })
    .await;
    assert!(unknown.was_interrupted());
    assert!(unknown.detail.contains("Reconnect both pools and recheck"));
    let first = unknown.workflow_batch[0].child;
    assert_eq!(
        unknown.workflow_batch[0].state,
        Some(OperationState::OutcomeUnknown)
    );
    assert_eq!(unknown.workflow_batch[1].state, None);
    let recovered_source = connect(&f.source, &restarted, false).await;
    // Recovery starts no worker for the batch on either pool: one connection alone
    // cannot read a move between two. A recheck without both pools is refused.
    tokio::time::sleep(Duration::from_millis(300)).await;
    for connection in [&recovered_source, &recovered_dest] {
        let seen = history_where(connection, id, |r| {
            r.state == OperationState::OutcomeUnknown
        })
        .await;
        assert!(seen.local_error.is_none(), "{:?}", seen.local_error);
    }
    assert!(
        recovered_source
            .control_resource(id, OperationControl::Recheck)
            .is_err()
    );
    assert!(
        recovered_source
            .recheck_cross_pool_vm(first, &recovered_dest)
            .is_err()
    );
    let task_reads = count(&f.source, "task.get_record");
    // The hosts now run the transfer and the reply reaches the worker that was fenced.
    f.release.notify_one();
    let result = done(&op).await;
    assert_eq!(result.state, OperationState::OutcomeUnknown);
    tokio::time::sleep(Duration::from_millis(300)).await;
    // That worker kept the receipt and did nothing else: no poll, no second guest.
    assert_eq!(count(&f.source, "task.get_record"), task_reads);
    assert_eq!(f.control.lock().unwrap().sent, vec![1]);
    assert_eq!(count(&f.destination, "host.migrate_receive"), 1);
    assert_eq!(stored(dir.path()).len(), 2);
    // Both pools stay reserved while the outcome is unknown.
    let refused = rename_pool(&recovered_dest, 11, "Destination pool", "Renamed").await;
    assert_eq!(refused.detail, CONFLICT);
    // Recheck reads the recorded task and both pools. It prepares no receiver and sends
    // no transfer, and it does not start the second guest.
    let before = (
        f.source.methods.lock().unwrap().len(),
        f.destination.methods.lock().unwrap().len(),
    );
    let recheck = recovered_source
        .recheck_cross_pool_batch(id, &recovered_dest)
        .unwrap();
    let checked = done(&recheck).await;
    assert_eq!(
        checked.state,
        OperationState::PartiallyCompleted,
        "{} / {:?}",
        checked.detail,
        checked.local_error
    );
    assert!(
        checked
            .detail
            .contains("1 of 2 VM workflows verified; 1 not started")
    );
    assert_eq!(
        checked.workflow_batch[0].state,
        Some(OperationState::Succeeded)
    );
    assert_eq!(checked.workflow_batch[1].state, None);
    for (mock, from) in [(&f.source, before.0), (&f.destination, before.1)] {
        let log = mock.methods.lock().unwrap();
        assert!(
            log[from..].iter().all(|m| m.ends_with(".get_record")
                || m.ends_with(".get_by_uuid")
                || m.ends_with(".get_all_records")
                || m == "event.from"
                || m == "task.destroy"),
            "a recheck only reads: {:?}",
            &log[from..]
        );
    }
    assert_eq!(f.control.lock().unwrap().sent, vec![1]);
    assert_eq!(count(&f.source, "Async.VM.migrate_send"), 1);
    assert_eq!(count(&f.destination, "host.migrate_receive"), 1);
    assert_eq!(count(&f.source, "task.cancel"), 0);
    assert!(
        f.control
            .lock()
            .unwrap()
            .source
            .contains_key("OpaqueRef:VM:guest2")
    );
    assert_eq!(stored(dir.path()).len(), 2);
    let child = history_where(&recovered_source, first, |r| r.state.terminal()).await;
    assert_eq!(child.state, OperationState::Succeeded, "{}", child.detail);
    privacy(dir.path(), &[checked, child]);
    // The verified end releases both pools.
    let renamed = rename_pool(&recovered_dest, 11, "Destination pool", "Renamed").await;
    assert_eq!(
        renamed.state,
        OperationState::Succeeded,
        "{}",
        renamed.detail
    );
    source.close().await;
    dest.close().await;
    recovered_source.close().await;
    recovered_dest.close().await;
}

/// The client restarts while the first guest's own move is still reading the two pools.
/// The journal shows that the move never stored its plan, which it does before its first
/// call that changes a host, so a recheck settles it as not started and sends nothing.
#[tokio::test]
async fn a_move_interrupted_before_it_sent_anything_is_settled_by_recheck_as_not_sent() {
    let f = fixture(Fault::None).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let source = connect(&f.source, &backend, false).await;
    let dest = connect(&f.destination, &backend, false).await;
    // The whole-selection check prepares each of the two guests once. The third
    // preparation is the first guest's own, as a child of the batch.
    f.control.lock().unwrap().hold_listing = Some(3);
    let op = source
        .move_vms_to_pool(OperationId::new(), selection(&source, &dest), &dest)
        .unwrap();
    tokio::time::timeout(deadline(10), f.entered.notified())
        .await
        .unwrap();
    let id = op.latest().id;
    let rows = stored(dir.path());
    assert_eq!(rows.len(), 2, "the first move is admitted");
    assert!(!rows[1].1.contains("cross_pool_plan"));
    let restarted = Backend::with_state_directory(dir.path().into()).unwrap();
    let recovered_source = connect(&f.source, &restarted, false).await;
    let recovered_dest = connect(&f.destination, &restarted, false).await;
    let unknown = history_where(&recovered_source, id, |r| {
        r.state == OperationState::OutcomeUnknown
    })
    .await;
    let first = unknown.workflow_batch[0].child;
    f.release.notify_one();
    let result = done(&op).await;
    assert_eq!(result.state, OperationState::OutcomeUnknown);
    tokio::time::sleep(Duration::from_millis(300)).await;
    nothing_sent(&f);
    assert_eq!(stored(dir.path()).len(), 2);
    let recheck = recovered_source
        .recheck_cross_pool_batch(id, &recovered_dest)
        .unwrap();
    let checked = done(&recheck).await;
    assert_eq!(
        checked.state,
        OperationState::Failed,
        "{} / {:?}",
        checked.detail,
        checked.local_error
    );
    assert!(
        checked
            .detail
            .contains("0 of 2 VM workflows verified; 1 not started")
    );
    assert_eq!(
        checked.workflow_batch[0].state,
        Some(OperationState::Cancelled)
    );
    assert_eq!(checked.workflow_batch[1].state, None);
    let child = history_where(&recovered_source, first, |r| r.state.terminal()).await;
    assert!(
        child
            .detail
            .contains("no receiver was prepared and no transfer was submitted"),
        "{}",
        child.detail
    );
    nothing_sent(&f);
    assert_eq!(stored(dir.path()).len(), 2);
    for (connection, pool, name) in [
        (&recovered_source, 1, "Source pool"),
        (&recovered_dest, 11, "Destination pool"),
    ] {
        let renamed = rename_pool(connection, pool, name, "Renamed after the recheck").await;
        assert_eq!(
            renamed.state,
            OperationState::Succeeded,
            "{name}: {}",
            renamed.detail
        );
    }
    source.close().await;
    dest.close().await;
    recovered_source.close().await;
    recovered_dest.close().await;
}

/// The first guest's transfer was sent and its reply was lost, so no task is known. The
/// batch stays unknown through a recheck, which sends nothing, and only an
/// acknowledgement releases the two pools. The second guest is never started.
#[tokio::test]
async fn a_lost_reply_leaves_the_batch_unknown_until_it_is_acknowledged() {
    let f = fixture(Fault::LostFirstSubmit).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let source = connect(&f.source, &backend, false).await;
    let dest = connect(&f.destination, &backend, false).await;
    let op = source
        .move_vms_to_pool(OperationId::new(), selection(&source, &dest), &dest)
        .unwrap();
    let result = done(&op).await;
    assert_eq!(
        result.state,
        OperationState::OutcomeUnknown,
        "{}",
        result.detail
    );
    assert_eq!(
        result.workflow_batch[0].state,
        Some(OperationState::OutcomeUnknown)
    );
    assert_eq!(result.workflow_batch[1].state, None);
    let recheck = source.recheck_cross_pool_batch(result.id, &dest).unwrap();
    let checked = done(&recheck).await;
    assert_eq!(
        checked.state,
        OperationState::OutcomeUnknown,
        "{} / {:?}",
        checked.detail,
        checked.local_error
    );
    assert!(checked.local_error.is_none(), "{:?}", checked.local_error);
    assert!(
        checked.workflow_batch[0]
            .detail
            .contains("nothing will be replayed"),
        "{}",
        checked.workflow_batch[0].detail
    );
    assert_eq!(checked.workflow_batch[1].state, None);
    assert_eq!(f.control.lock().unwrap().sent, vec![1]);
    assert_eq!(count(&f.source, "Async.VM.migrate_send"), 1);
    assert_eq!(count(&f.destination, "host.migrate_receive"), 1);
    assert_eq!(count(&f.source, "task.cancel"), 0);
    for (connection, pool, name) in [(&source, 1, "Source pool"), (&dest, 11, "Destination pool")] {
        let refused = rename_pool(connection, pool, name, "Renamed while unknown").await;
        assert_eq!(refused.detail, CONFLICT, "{name}");
    }
    // The acknowledgement is given on the destination pool's connection and releases the
    // batch and its unknown move together, on both pools.
    acknowledge_unknown(&dest, result.id, 10).await;
    let acknowledged = history_where(&source, result.id, |r| {
        r.state == OperationState::AcknowledgedUnknown
    })
    .await;
    assert_eq!(
        acknowledged.workflow_batch[0].state,
        Some(OperationState::AcknowledgedUnknown)
    );
    assert_eq!(acknowledged.workflow_batch[1].state, None);
    for (connection, pool, name) in [(&source, 1, "Source pool"), (&dest, 11, "Destination pool")] {
        let renamed = rename_pool(connection, pool, name, "Renamed after acknowledgement").await;
        assert_eq!(
            renamed.state,
            OperationState::Succeeded,
            "{name}: {}",
            renamed.detail
        );
    }
    assert_eq!(count(&f.source, "Async.VM.migrate_send"), 1);
    assert_eq!(stored(dir.path()).len(), 4);
    source.close().await;
    dest.close().await;
}

#[tokio::test]
async fn a_destination_that_changed_since_the_review_is_refused_before_anything_is_sent() {
    // The destination connection is not the one that was reviewed.
    let f = fixture(Fault::None).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let source = connect(&f.source, &backend, false).await;
    let dest = connect(&f.destination, &backend, false).await;
    let stale = selection(&source, &dest);
    dest.close().await;
    let dest = connect(&f.destination, &backend, false).await;
    assert!(matches!(
        source.move_vms_to_pool(OperationId::new(), stale, &dest),
        Err(xenadmin_core::Error::Stale)
    ));
    // A destination connection of another backend, or one that cannot write.
    let other = tempfile::tempdir().unwrap();
    let foreign = Backend::with_state_directory(other.path().into()).unwrap();
    let foreign_dest = connect(&f.destination, &foreign, false).await;
    assert!(matches!(
        source.move_vms_to_pool(
            OperationId::new(),
            selection(&source, &foreign_dest),
            &foreign_dest
        ),
        Err(xenadmin_core::Error::InvalidMigration)
    ));
    foreign_dest.close().await;
    dest.close().await;
    let read_only = connect(&f.destination, &backend, true).await;
    assert!(matches!(
        source.move_vms_to_pool(
            OperationId::new(),
            selection(&source, &read_only),
            &read_only
        ),
        Err(xenadmin_core::Error::ReadOnly)
    ));
    read_only.close().await;
    nothing_sent(&f);
    source.close().await;

    // The destination pool itself changed after the review: the repository is no longer
    // attached to the chosen host, the host was disabled, or the migration address moved.
    for (reference, key, value) in [
        ("OpaqueRef:PBD:storage", "currently_attached", json!(false)),
        ("OpaqueRef:host:host", "enabled", json!(false)),
        ("OpaqueRef:PIF:network", "IP", json!("192.0.2.99")),
    ] {
        let f = fixture(Fault::None).await;
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let source = connect(&f.source, &backend, false).await;
        let dest = connect(&f.destination, &backend, false).await;
        let reviewed = selection(&source, &dest);
        f.control
            .lock()
            .unwrap()
            .destination
            .get_mut(reference)
            .unwrap()[key] = value;
        let op = source
            .move_vms_to_pool(OperationId::new(), reviewed, &dest)
            .unwrap();
        let result = done(&op).await;
        assert_eq!(
            result.state,
            OperationState::Rejected,
            "{reference}/{key}: {}",
            result.detail
        );
        assert!(
            !result.detail.contains("Invalid XAPI response"),
            "{}",
            result.detail
        );
        nothing_sent(&f);
        assert!(result.workflow_batch.iter().all(|r| r.state.is_none()));
        assert_eq!(stored(dir.path()).len(), 1);
        source.close().await;
        dest.close().await;
    }
}
