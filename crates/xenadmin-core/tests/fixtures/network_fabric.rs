#[path = "network_fabric_pool.rs"]
mod pool;
use super::*;
use std::collections::BTreeMap;
use xenadmin_core::{
    NetworkFabricChange as Change, NetworkFabricKind as Kind, NetworkFabricOperation as Operation,
    NetworkId, NetworkOperation, OperationControl, OperationId, OperationState, PifId,
    ResourceCommand, ResourceTarget, TunnelProtocol,
};
const POOL: &str = "11111111-1111-1111-1111-111111111111";
const HOST: &str = "22222222-2222-2222-2222-222222222222";
const NET: &str = "55555555-5555-5555-5555-555555555555";
const UNDERLAY: &str = "66666666-6666-6666-6666-666666666666";
const PIF: &str = "77777777-7777-7777-7777-777777777777";
const OBJECT: &str = "88888888-8888-8888-8888-888888888888";
const CHILD: &str = "99999999-9999-9999-9999-999999999999";
const TASK: &str = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa";
#[derive(Clone, Copy, PartialEq)]
enum Fault {
    None,
    DropReply,
    Failed,
    Cancelled,
    WrongGraph,
    OldIdentity,
    TaskRead,
    VerifyRead,
    Reboot,
    Pending,
    LateTask,
}
type Records = Arc<Mutex<BTreeMap<String, Value>>>;
fn pif() -> Value {
    json!({"uuid":PIF,"host":"OpaqueRef:host","network":"OpaqueRef:network-underlay","device":"eth1","MAC":"02:00:00:00:00:01","MTU":"1500","VLAN":"-1","management":false,"physical":true,"managed":true,"currently_attached":true,"bond_slave_of":"OpaqueRef:NULL","bond_master_of":[],"VLAN_master_of":"OpaqueRef:NULL","VLAN_slave_of":[],"tunnel_access_PIF_of":[],"tunnel_transport_PIF_of":[],"sriov_physical_PIF_of":[],"sriov_logical_PIF_of":[],"ip_configuration_mode":"None","IP":"","netmask":"","gateway":"","DNS":"","ipv6_configuration_mode":"None","IPv6":[],"ipv6_gateway":"","disallow_unplug":false,"primary_address_type":"IPv4","other_config":{},"properties":{},"capabilities":["sriov"],"PCI":"OpaqueRef:PCI"})
}
async fn fixture(kind: Kind, fault: Fault) -> (Mock, Records, Arc<AtomicUsize>) {
    let mut physical = pif();
    if matches!(kind, Kind::Tunnel(_)) {
        physical["ip_configuration_mode"] = json!("Static");
        physical["IP"] = json!("192.0.2.20");
        physical["netmask"] = json!("255.255.255.0");
    }
    let records: Records = Arc::new(Mutex::new(BTreeMap::from([
        (
            "pool".into(),
            json!({"uuid":POOL,"name_label":"Synthetic pool","master":"OpaqueRef:host","ha_enabled":false,"restrictions":{"restrict_network_sriov":"false"}}),
        ),
        (
            "host".into(),
            json!({"uuid":HOST,"name_label":"Synthetic host","enabled":false,"current_operations":{},"address":"192.0.2.10","metrics":"OpaqueRef:host_metrics","software_version":{"network_backend":"openvswitch"}}),
        ),
        (
            "host_metrics".into(),
            json!({"uuid":"00000000-0000-0000-0000-000000000001","live":true}),
        ),
        (
            "network".into(),
            json!({"uuid":NET,"name_label":"Fabric network","managed":true,"purpose":[],"bridge":"xapi0","MTU":"1500","default_locking_mode":"unlocked","PIFs":[],"VIFs":[],"other_config":{}}),
        ),
        (
            "network-underlay".into(),
            json!({"uuid":UNDERLAY,"name_label":"Transport","managed":true,"purpose":[],"bridge":"xenbr0","MTU":"1500","default_locking_mode":"unlocked","PIFs":["OpaqueRef:PIF"],"VIFs":[],"other_config":{}}),
        ),
        ("PIF".into(), physical),
        (
            "PCI".into(),
            json!({"uuid":"00000000-0000-0000-0000-000000000002","host":"OpaqueRef:host","pci_id":"0000:01:00.1","vendor_id":"8086","device_id":"100e","driver_name":"fixture_nic"}),
        ),
        (
            "SDN_controller".into(),
            json!({"uuid":"00000000-0000-0000-0000-000000000003","protocol":"ssl","address":"192.0.2.40","port":"6632"}),
        ),
    ])));
    if fault == Fault::OldIdentity {
        let class = if matches!(kind, Kind::Tunnel(_)) {
            "Tunnel"
        } else {
            "Network_sriov"
        };
        records
            .lock()
            .unwrap()
            .insert(format!("{class}-old"), json!({"uuid":OBJECT}));
    }
    let writes = Arc::new(AtomicUsize::new(0));
    if fault == Fault::LateTask {
        records
            .lock()
            .unwrap()
            .insert("_hold_task".into(), json!({}));
    }
    let source = records.clone();
    let written = writes.clone();
    let callback = move |request: Value| {
        let records = source.clone();
        let writes = written.clone();
        async move {
            let method = request["method"].as_str().unwrap();
            let pool_fixture = records.lock().unwrap().contains_key("_pool");
            if pool_fixture && let Some(reply) = pool::request(&records, &writes, &request).await {
                return Some(reply);
            }
            if method == "event.from" && request["params"][2] == "" {
                let events=records.lock().unwrap().iter().filter(|(k,_)|!k.starts_with('_')).map(|(k,v)|json!({"class":k.split('-').next().unwrap(),"operation":"add","ref":format!("OpaqueRef:{k}"),"snapshot":v})).collect::<Vec<_>>();
                return Some(RpcReply::Json(
                    json!({"result":{"token":"initial","events":events}}),
                ));
            }
            if let Some(class) = method.strip_suffix(".get_all_records") {
                let map = records
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|(k, _)| k.split('-').next() == Some(class))
                    .map(|(k, v)| (format!("OpaqueRef:{k}"), v.clone()))
                    .collect::<serde_json::Map<_, _>>();
                if class == "PIF"
                    && writes.load(Ordering::SeqCst) > 0
                    && ((fault == Fault::VerifyRead
                        && records.lock().unwrap().get("allow-read").is_none())
                        || records.lock().unwrap().contains_key("_deny_pif"))
                {
                    return Some(RpcReply::Json(
                        json!({"error":{"code":1,"message":"RBAC_PERMISSION_DENIED","data":[]}}),
                    ));
                }
                return Some(RpcReply::Json(json!({"result":map})));
            }
            if let Some(class) = method.strip_suffix(".get_by_uuid") {
                let map = records.lock().unwrap();
                let found = map.iter().find(|(k, v)| {
                    k.split('-').next() == Some(class) && v["uuid"] == request["params"][1]
                });
                return Some(RpcReply::Json(match found {
                    Some((k, _)) => json!({"result":format!("OpaqueRef:{k}")}),
                    None => json!({"error":{"code":1,"message":"UUID_INVALID","data":[]}}),
                }));
            }
            if method == "task.get_record" && fault == Fault::LateTask {
                {
                    let mut map = records.lock().unwrap();
                    let reads = map.get("_task_reads").and_then(Value::as_u64).unwrap_or(0);
                    map.insert("_task_reads".into(), json!(reads + 1));
                }
                while records.lock().unwrap().contains_key("_hold_task") {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            }
            if method == "task.get_record" && fault == Fault::TaskRead {
                return Some(RpcReply::Json(
                    json!({"error":{"code":1,"message":"HANDLE_INVALID","data":[]}}),
                ));
            }
            if method.ends_with(".get_record") {
                let key = request["params"][1]
                    .as_str()
                    .unwrap()
                    .strip_prefix("OpaqueRef:")
                    .unwrap();
                let map = records.lock().unwrap();
                return Some(RpcReply::Json(match map.get(key) {
                    Some(v) => json!({"result":v}),
                    None => json!({"error":{"code":1,"message":"HANDLE_INVALID","data":[]}}),
                }));
            }
            if method == "task.destroy" {
                records.lock().unwrap().remove("task");
                return Some(RpcReply::Json(json!({"result":""})));
            }
            if method == "task.cancel" {
                return Some(RpcReply::Json(json!({"result":""})));
            }
            if method.starts_with("Async.Tunnel.") || method.starts_with("Async.Network_sriov.") {
                let mut map = records.lock().unwrap();
                let class = if matches!(kind, Kind::Tunnel(_)) {
                    "Tunnel"
                } else {
                    "Network_sriov"
                };
                let object_ref = format!("OpaqueRef:{class}");
                let (parent, child, parents, children) = if matches!(kind, Kind::Tunnel(_)) {
                    (
                        "transport_PIF",
                        "access_PIF",
                        "tunnel_transport_PIF_of",
                        "tunnel_access_PIF_of",
                    )
                } else {
                    (
                        "physical_PIF",
                        "logical_PIF",
                        "sriov_physical_PIF_of",
                        "sriov_logical_PIF_of",
                    )
                };
                if method.ends_with(".create") {
                    assert_eq!(request["params"][1], "OpaqueRef:PIF");
                    assert_eq!(request["params"][2], "OpaqueRef:network");
                    let mut obj = json!({"uuid":OBJECT,parent:"OpaqueRef:PIF",child:"OpaqueRef:PIF-child","protocol":"gre","status":{"active":"false"},"requires_reboot":fault==Fault::Reboot,"configuration_mode":if fault==Fault::Reboot{"modprobe"}else{"sysfs"}});
                    if let Kind::Tunnel(protocol) = kind {
                        assert_eq!(request["params"][3], protocol.as_str());
                        obj["protocol"] = json!(protocol.as_str());
                    }
                    map.insert(class.into(), obj);
                    map.get_mut("PIF").unwrap()[parents] = json!([object_ref]);
                    let mut new = pif();
                    new["uuid"] = json!(CHILD);
                    new["network"] = json!(if fault == Fault::WrongGraph {
                        "OpaqueRef:network-underlay"
                    } else {
                        "OpaqueRef:network"
                    });
                    new["physical"] = json!(false);
                    new["PCI"] = json!("OpaqueRef:NULL");
                    new[children] = json!([object_ref]);
                    new["capabilities"] = json!([]);
                    new["currently_attached"] = json!(fault != Fault::Reboot);
                    if matches!(kind, Kind::Tunnel(_)) {
                        new["MTU"] = json!("-1");
                        new["device"] = json!("tunnel0");
                    } else {
                        new["MAC"] = json!("fe:ff:ff:ff:ff:ff");
                    }
                    map.insert("PIF-child".into(), new);
                    map.get_mut("network").unwrap()["PIFs"] = json!(["OpaqueRef:PIF-child"]);
                } else {
                    assert_eq!(request["params"][1], object_ref);
                    map.remove(class);
                    map.remove("PIF-child");
                    map.get_mut("network").unwrap()["PIFs"] = json!([]);
                    map.get_mut("PIF").unwrap()[parents] = json!([]);
                    if map.contains_key("_deny_after_delete") {
                        map.insert("_deny_pif".into(), json!({}));
                    }
                }
                map.insert("task".into(),json!({"uuid":uuid::Uuid::from_u128(uuid::Uuid::parse_str(TASK).unwrap().as_u128()+writes.load(Ordering::SeqCst) as u128).to_string(),"status":match fault{Fault::Failed=>"failure",Fault::Cancelled=>"cancelled",Fault::Pending=>"pending",_=>"success"},"progress":1.0,"error_info":["FIXTURE_FAILURE"],"allowed_operations":["cancel"],"result":format!("<value><string>{object_ref}</string></value>")}));
                writes.fetch_add(1, Ordering::SeqCst);
                return Some(
                    if fault == Fault::DropReply
                        || method.ends_with(".destroy") && map.contains_key("_drop_after_delete")
                    {
                        RpcReply::Drop
                    } else {
                        RpcReply::Json(json!({"result":"OpaqueRef:task"}))
                    },
                );
            }
            None
        }
    };
    (
        mock_overridden(override_rpc(callback)).await,
        records,
        writes,
    )
}
async fn connect(mock: &Mock, backend: &Backend, readonly: bool) -> xenadmin_core::Connection {
    let mut options = ConnectionOptions::new(mock.endpoint.clone());
    options.read_only = readonly;
    let c = backend.connect(
        options,
        Credentials::new("root".into(), "fixture-password".into()),
    );
    c.wait_ready(Duration::from_secs(5)).await.unwrap();
    c
}
fn target() -> ResourceTarget {
    ResourceTarget::Network(NetworkId::parse(NET).unwrap())
}
fn create(c: &xenadmin_core::Connection, kind: Kind) -> Operation {
    Operation::capture(
        c.latest().inventory().unwrap(),
        NetworkId::parse(NET).unwrap(),
        PifId::parse(PIF).unwrap(),
        kind,
        Change::Create,
    )
    .unwrap_or_else(|e| panic!("{e}: {:?}", c.latest().inventory().unwrap().details.classes))
}
fn submit(c: &xenadmin_core::Connection, op: Operation) -> xenadmin_core::ResourceOperation {
    c.submit_resource(
        OperationId::new(),
        ResourceCommand::Network {
            target: c.latest().observe_resource(target()).unwrap(),
            operation: NetworkOperation::Fabric(Box::new(op)),
        },
    )
    .unwrap()
}
async fn finish(
    op: &xenadmin_core::ResourceOperation,
) -> Arc<xenadmin_core::ResourceOperationRecord> {
    tokio::time::timeout(Duration::from_secs(8), async {
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
async fn tunnel_and_sriov_creation_return_new_object_and_interface_without_claiming_traffic() {
    for (kind, fault) in [
        (Kind::Tunnel(TunnelProtocol::Gre), Fault::None),
        (Kind::Tunnel(TunnelProtocol::Vxlan), Fault::None),
        (Kind::Sriov, Fault::None),
        (Kind::Sriov, Fault::Reboot),
    ] {
        let (m, _, writes) = fixture(kind, fault).await;
        let dir = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&m, &b, false).await;
        let r = finish(&submit(&c, create(&c, kind))).await;
        assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
        let e = r.network_fabric_created.as_ref().unwrap();
        assert_eq!(e.object.uuid().to_string(), OBJECT);
        assert_eq!(e.interface.to_string(), CHILD);
        assert_eq!(writes.load(Ordering::SeqCst), 1);
        let encoded = serde_json::to_string(&*r).unwrap();
        assert!(!encoded.contains("OpaqueRef:"));
        assert!(!encoded.contains("network_fabric_plan"));
        assert!(!encoded.contains("\"baseline\""));
        assert!(r.detail.contains("Pool coverage"));
    }
}
#[tokio::test]
async fn preflight_rejects_changed_identity_consumers_controller_and_driver_impact_without_writes()
{
    for kind in [Kind::Tunnel(TunnelProtocol::Gre), Kind::Sriov] {
        for change in 0..6 {
            let (m, records, writes) = fixture(kind, Fault::None).await;
            let dir = tempfile::tempdir().unwrap();
            let b = Backend::with_state_directory(dir.path().into()).unwrap();
            let c = connect(&m, &b, false).await;
            let op = create(&c, kind);
            {
                let mut map = records.lock().unwrap();
                match change {
                    0 => map.get_mut("PIF").unwrap()["MAC"] = json!("02:00:00:00:00:ff"),
                    1 => map.get_mut("network").unwrap()["VIFs"] = json!(["OpaqueRef:missing"]),
                    2 => map.get_mut("network").unwrap()["purpose"] = json!(["nbd"]),
                    3 => {
                        if kind == Kind::Sriov {
                            map.get_mut("PIF").unwrap()["management"] = json!(true);
                        } else {
                            map.get_mut("SDN_controller").unwrap()["address"] = json!("192.0.2.41");
                        }
                    }
                    4 => {
                        map.get_mut("pool").unwrap()["uuid"] =
                            json!("00000000-0000-0000-0000-000000000099");
                    }
                    _ => {
                        if kind == Kind::Sriov {
                            map.get_mut("network-underlay").unwrap()["purpose"] = json!(["nbd"]);
                        } else {
                            map.get_mut("host").unwrap()["software_version"]["network_backend"] =
                                json!("bridge");
                        }
                    }
                }
            }
            let r = finish(&submit(&c, op)).await;
            assert_ne!(r.state, OperationState::Succeeded, "{change}: {}", r.detail);
            assert_eq!(writes.load(Ordering::SeqCst), 0, "{change}: {}", r.detail);
        }
    }
}
#[tokio::test]
async fn lost_failed_cancelled_or_misbound_fabric_results_never_replay_or_report_success() {
    for fault in [
        Fault::DropReply,
        Fault::Failed,
        Fault::Cancelled,
        Fault::WrongGraph,
        Fault::OldIdentity,
        Fault::TaskRead,
    ] {
        let kind = Kind::Tunnel(TunnelProtocol::Gre);
        let (m, _, writes) = fixture(kind, fault).await;
        let dir = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&m, &b, false).await;
        let op = submit(&c, create(&c, kind));
        let r = finish(&op).await;
        assert_eq!(r.state, OperationState::OutcomeUnknown, "{}", r.detail);
        assert!(r.network_fabric_created.is_none());
        c.control_resource(r.id, OperationControl::Recheck).unwrap();
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(writes.load(Ordering::SeqCst), 1);
        assert_ne!(op.latest().state, OperationState::Succeeded);
    }
}
#[tokio::test]
async fn positive_creation_identity_survives_failed_verification_and_removed_task_recheck() {
    let kind = Kind::Sriov;
    let (m, records, writes) = fixture(kind, Fault::VerifyRead).await;
    let dir = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&m, &b, false).await;
    let op = submit(&c, create(&c, kind));
    let r = finish(&op).await;
    assert_eq!(r.state, OperationState::OutcomeUnknown, "{}", r.detail);
    assert!(r.network_fabric_created.is_some());
    {
        let mut map = records.lock().unwrap();
        map.remove("task");
        map.insert("allow-read".into(), json!({}));
    }
    c.control_resource(r.id, OperationControl::Recheck).unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let now = c
                .resource_history()
                .iter()
                .find(|row| row.id == r.id)
                .cloned()
                .unwrap();
            if now.revision() > r.revision() && now.state.terminal() {
                assert_eq!(now.state, OperationState::Succeeded, "{}", now.detail);
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("recheck: {:?}", c.resource_history()));
    assert_eq!(writes.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn readonly_fabric_commands_are_rejected_before_dispatch() {
    let kind = Kind::Tunnel(TunnelProtocol::Gre);
    let (m, _, writes) = fixture(kind, Fault::None).await;
    let dir = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&m, &b, true).await;
    let result = c.submit_resource(
        OperationId::new(),
        ResourceCommand::Network {
            target: c.latest().observe_resource(target()).unwrap(),
            operation: NetworkOperation::Fabric(Box::new(create(&c, kind))),
        },
    );
    assert!(result.is_err());
    assert_eq!(writes.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn deletion_removes_only_reviewed_host_pair_and_retains_underlay_and_network() {
    for kind in [Kind::Tunnel(TunnelProtocol::Gre), Kind::Sriov] {
        let (m, records, writes) = fixture(kind, Fault::None).await;
        let dir = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&m, &b, false).await;
        let first = finish(&submit(&c, create(&c, kind))).await;
        assert_eq!(first.state, OperationState::Succeeded, "{}", first.detail);
        c.close().await;
        let c = connect(&m, &b, false).await;
        let object = first.network_fabric_created.as_ref().unwrap().object;
        let op = Operation::capture(
            c.latest().inventory().unwrap(),
            NetworkId::parse(NET).unwrap(),
            PifId::parse(PIF).unwrap(),
            kind,
            Change::Delete(object),
        )
        .unwrap();
        let result = finish(&submit(&c, op)).await;
        assert_eq!(result.state, OperationState::Succeeded, "{}", result.detail);
        let map = records.lock().unwrap();
        assert!(!map.contains_key("PIF-child"));
        assert!(map.contains_key("PIF"));
        assert!(map.contains_key("network"));
        assert_eq!(writes.load(Ordering::SeqCst), 2);
    }
}
#[tokio::test]
async fn pending_task_cancellation_is_not_completion_and_no_new_create_is_admitted() {
    let kind = Kind::Tunnel(TunnelProtocol::Gre);
    let (m, records, writes) = fixture(kind, Fault::Pending).await;
    let dir = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&m, &b, false).await;
    let command = create(&c, kind);
    let op = submit(&c, command.clone());
    tokio::time::timeout(Duration::from_secs(5), async {
        while !op.latest().can_cancel {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    c.control_resource(op.latest().id, OperationControl::RequestCancellation)
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !m.methods.lock().unwrap().iter().any(|m| m == "task.cancel") {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(!op.latest().state.terminal());
    records.lock().unwrap().get_mut("task").unwrap()["status"] = json!("cancelled");
    let r = finish(&op).await;
    assert_eq!(r.state, OperationState::OutcomeUnknown, "{}", r.detail);
    let rejected = finish(&submit(&c, command)).await;
    assert_eq!(rejected.state, OperationState::Rejected);
    assert_eq!(writes.load(Ordering::SeqCst), 1);
    assert_eq!(
        m.methods
            .lock()
            .unwrap()
            .iter()
            .filter(|m| *m == "task.cancel")
            .count(),
        1
    );
}
#[tokio::test]
async fn corrupted_fabric_receipts_and_step_claims_are_refused_on_restart() {
    for change in 0..6 {
        let kind = Kind::Tunnel(TunnelProtocol::Gre);
        let (m, _, writes) = fixture(kind, Fault::None).await;
        let dir = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&m, &b, false).await;
        let r = finish(&submit(&c, create(&c, kind))).await;
        assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
        c.close().await;
        drop(b);
        let db = rusqlite::Connection::open(dir.path().join("operations.sqlite3")).unwrap();
        let raw: String = db
            .query_row(
                "SELECT payload FROM resource_operations WHERE id=?1",
                [r.id.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        let mut v: Value = serde_json::from_str(&raw).unwrap();
        match change {
            0 => {
                v["network_fabric_plan"] = Value::Null;
                v["network_fabric_created"] = Value::Null;
                v["created"] = Value::Null;
                v["steps"] = json!([]);
                db.execute(
                    "UPDATE resource_operations SET task=NULL,result_reference=NULL WHERE id=?1",
                    [r.id.to_string()],
                )
                .unwrap();
            }
            1 => v["network_fabric_plan"]["completed"] = json!(false),
            2 => v["steps"][0]["label"] = json!("Unreviewed operation"),
            3 => v["network_fabric_plan"]["baseline"]["pif"] = json!([]),
            4 => v["network_fabric_plan"]["pool"] = json!("00000000-0000-0000-0000-000000000099"),
            _ => v["network_fabric_created"]["interface"] = json!(PIF),
        }
        db.execute(
            "UPDATE resource_operations SET payload=?1 WHERE id=?2",
            rusqlite::params![v.to_string(), r.id.to_string()],
        )
        .unwrap();
        drop(db);
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let recovered = connect(&m, &backend, false).await;
        assert!(recovered.latest().error().is_some(), "corruption {change}");
        assert!(
            !recovered
                .resource_history()
                .iter()
                .any(|record| record.id == r.id),
            "corruption {change}"
        );
        assert_eq!(writes.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn late_success_receipts_keep_new_ids_after_recovery_and_acknowledgement_without_revival() {
    for acknowledge in [false, true] {
        let kind = Kind::Tunnel(TunnelProtocol::Gre);
        let (m, records, writes) = fixture(kind, Fault::LateTask).await;
        let dir = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&m, &b, false).await;
        let op = submit(&c, create(&c, kind));
        tokio::time::timeout(Duration::from_secs(5), async {
            while records
                .lock()
                .unwrap()
                .get("_task_reads")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                < 1
            {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        let b2 = Backend::with_state_directory(dir.path().into()).unwrap();
        let c2 = connect(&m, &b2, false).await;
        tokio::time::timeout(Duration::from_secs(5), async {
            while !c2
                .resource_history()
                .iter()
                .any(|r| r.id == op.latest().id && r.state == OperationState::OutcomeUnknown)
            {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        let id = op.latest().id;
        if acknowledge {
            c2.control_resource(id, OperationControl::AcknowledgeUnknown)
                .unwrap();
            tokio::time::timeout(Duration::from_secs(5), async {
                while !c2
                    .resource_history()
                    .iter()
                    .any(|r| r.id == id && r.state == OperationState::AcknowledgedUnknown)
                {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .unwrap();
        }
        records.lock().unwrap().remove("_hold_task");
        tokio::time::timeout(Duration::from_secs(5), async {
            while op.latest().network_fabric_created.is_none() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            op.latest().state,
            if acknowledge {
                OperationState::AcknowledgedUnknown
            } else {
                OperationState::OutcomeUnknown
            }
        );
        assert_eq!(
            op.latest()
                .network_fabric_created
                .as_ref()
                .unwrap()
                .interface
                .to_string(),
            CHILD
        );
        assert_eq!(writes.load(Ordering::SeqCst), 1);
        c.close().await;
        c2.close().await;
    }
}

#[tokio::test]
async fn deletion_lost_reply_stays_unknown_but_saved_completion_survives_removed_task() {
    for loss in [false, true] {
        let kind = Kind::Sriov;
        let (m, records, writes) = fixture(kind, Fault::None).await;
        let dir = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&m, &b, false).await;
        let first = finish(&submit(&c, create(&c, kind))).await;
        assert_eq!(first.state, OperationState::Succeeded, "{}", first.detail);
        c.close().await;
        let c = connect(&m, &b, false).await;
        let object = first.network_fabric_created.as_ref().unwrap().object;
        let command = Operation::capture(
            c.latest().inventory().unwrap(),
            NetworkId::parse(NET).unwrap(),
            PifId::parse(PIF).unwrap(),
            kind,
            Change::Delete(object),
        )
        .unwrap();
        records.lock().unwrap().insert(
            if loss {
                "_drop_after_delete"
            } else {
                "_deny_after_delete"
            }
            .into(),
            json!({}),
        );
        let op = submit(&c, command);
        let r = finish(&op).await;
        assert_eq!(r.state, OperationState::OutcomeUnknown, "{}", r.detail);
        {
            let mut map = records.lock().unwrap();
            assert!(!map.contains_key("PIF-child"));
            map.remove("_deny_pif");
            map.remove("task");
        }
        c.control_resource(r.id, OperationControl::Recheck).unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let row = c
                    .resource_history()
                    .iter()
                    .find(|row| row.id == r.id)
                    .cloned()
                    .unwrap();
                if row.revision() > r.revision()
                    && (row.state.terminal() || row.state == OperationState::OutcomeUnknown)
                {
                    assert_eq!(
                        row.state,
                        if loss {
                            OperationState::OutcomeUnknown
                        } else {
                            OperationState::Succeeded
                        },
                        "{}",
                        row.detail
                    );
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(writes.load(Ordering::SeqCst), 2);
    }
}

#[tokio::test]
async fn removal_does_not_require_creation_only_controller_or_feature_prerequisites() {
    for kind in [Kind::Tunnel(TunnelProtocol::Gre), Kind::Sriov] {
        let (m, records, _) = fixture(kind, Fault::None).await;
        let dir = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&m, &b, false).await;
        let first = finish(&submit(&c, create(&c, kind))).await;
        assert_eq!(first.state, OperationState::Succeeded, "{}", first.detail);
        c.close().await;
        {
            let mut map = records.lock().unwrap();
            if kind == Kind::Sriov {
                map.get_mut("pool").unwrap()["restrictions"]["restrict_network_sriov"] =
                    json!("true");
                map.get_mut("PIF").unwrap()["capabilities"] = json!([]);
            } else {
                map.remove("SDN_controller");
                map.get_mut("PIF").unwrap()["currently_attached"] = json!(false);
            }
        }
        let c = connect(&m, &b, false).await;
        let command = Operation::capture(
            c.latest().inventory().unwrap(),
            NetworkId::parse(NET).unwrap(),
            PifId::parse(PIF).unwrap(),
            kind,
            Change::Delete(first.network_fabric_created.as_ref().unwrap().object),
        )
        .unwrap();
        let result = finish(&submit(&c, command)).await;
        assert_eq!(result.state, OperationState::Succeeded, "{}", result.detail);
    }
}
