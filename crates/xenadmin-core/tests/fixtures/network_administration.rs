use super::*;
use std::collections::BTreeMap;
use xenadmin_core::{
    HostId, NetworkId, NetworkOperation, OperationId, OperationState, PifId, ResourceCommand,
    ResourceTarget,
};
const POOL: &str = "11111111-1111-1111-1111-111111111111";
const HOST: &str = "22222222-2222-2222-2222-222222222222";
const NET: &str = "55555555-5555-5555-5555-555555555555";
const UNDERLAY: &str = "66666666-6666-6666-6666-666666666666";
const PIF: &str = "77777777-7777-7777-7777-777777777777";
const VLAN: &str = "88888888-8888-8888-8888-888888888888";
const CHILD: &str = "99999999-9999-9999-9999-999999999999";
#[derive(Clone, Copy, PartialEq)]
enum Fault {
    None,
    DropReply,
    ChangedIdentity,
    WrongVlan,
    OldVlan,
    TaskRead,
    VerifyRead,
    MissingMembers,
    TaskFailed,
    TaskCancelled,
    ServicePolicyChanged,
    ServiceNetworkRebound,
    ServiceNetworkMissing,
}
type Records = Arc<Mutex<BTreeMap<String, Value>>>;
async fn fixture(fault: Fault) -> (Mock, Records) {
    let records: Records = Arc::new(Mutex::new(BTreeMap::from([
        (
            "pool".into(),
            json!({"uuid":POOL,"name_label":"Network fixture","master":"OpaqueRef:host","igmp_snooping_enabled":false}),
        ),
        (
            "host".into(),
            json!({"uuid":HOST,"name_label":"Fixture host","enabled":true,"address":"192.0.2.10"}),
        ),
        (
            "network".into(),
            json!({"uuid":NET,"name_label":"VLAN target","managed":true,"purpose":[],"bridge":"xapi0","MTU":"1500","PIFs":[],"VIFs":[],"other_config":{"unrelated":"preserve"}}),
        ),
        (
            "network-underlay".into(),
            json!({"uuid":UNDERLAY,"name_label":"Underlay","managed":true,"purpose":[],"bridge":"xenbr0","MTU":"1500","PIFs":["OpaqueRef:PIF"],"VIFs":[],"other_config":{}}),
        ),
        (
            "PIF".into(),
            json!({"uuid":PIF,"host":"OpaqueRef:host","network":"OpaqueRef:network-underlay","device":"eth0","MAC":"02:00:00:00:00:01","MTU":"1500","VLAN":"-1","management":false,"physical":true,"managed":true,"currently_attached":true,"bond_slave_of":"OpaqueRef:NULL","bond_master_of":[],"VLAN_slave_of":[],"ip_configuration_mode":"None","IP":"","netmask":"","gateway":"","DNS":"","ipv6_configuration_mode":"None","IPv6":[""],"ipv6_gateway":"","disallow_unplug":false,"primary_address_type":"IPv4","other_config":{"unrelated":"preserve"}}),
        ),
    ])));
    if fault == Fault::OldVlan {
        records.lock().unwrap().insert("VLAN".into(),json!({"uuid":VLAN,"tagged_PIF":"OpaqueRef:PIF","untagged_PIF":"OpaqueRef:PIF-child","tag":"999"}));
    }
    let mut second = records.lock().unwrap()["PIF"].clone();
    second["uuid"] = json!("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa");
    second["device"] = json!("eth1");
    second["MAC"] = json!("02:00:00:00:00:02");
    records.lock().unwrap().insert("PIF-second".into(), second);
    let calls = Arc::new(Mutex::new(Vec::<Value>::new()));
    let source = records.clone();
    let writes = Arc::new(AtomicUsize::new(0));
    let callback = move |request: Value| {
        let records = source.clone();
        let calls = calls.clone();
        let writes = writes.clone();
        async move {
            let method = request["method"].as_str().unwrap();
            calls.lock().unwrap().push(request.clone());
            if method == "event.from" && request["params"][2] == "" {
                let mut initial = batch("initial", "Halted");
                initial["events"]
                    .as_array_mut()
                    .unwrap()
                    .retain(|e| e["class"] == "VM");
                let mut map = records.lock().unwrap();
                for (key, record) in map.iter().filter(|(key, _)| key.as_str() != "task-result") {
                    let class = key.split('-').next().unwrap();
                    initial["events"].as_array_mut().unwrap().push(json!({"class":class,"operation":"add","ref":format!("OpaqueRef:{key}"),"snapshot":record}));
                }
                if fault == Fault::ChangedIdentity {
                    map.get_mut("PIF").unwrap()["MAC"] = json!("02:00:00:00:00:ff");
                }
                if fault == Fault::MissingMembers {
                    map.get_mut("network")
                        .unwrap()
                        .as_object_mut()
                        .unwrap()
                        .remove("PIFs");
                }
                return Some(RpcReply::Json(json!({"result":initial})));
            }
            if let Some(class) = method.strip_suffix(".get_all_records")
                && ["pool", "host", "network", "PIF", "VLAN", "Bond"].contains(&class)
            {
                let map = records.lock().unwrap();
                let mut values = map
                    .iter()
                    .filter(|(key, _)| key.split('-').next() == Some(class))
                    .map(|(key, value)| (format!("OpaqueRef:{key}"), value.clone()))
                    .collect::<serde_json::Map<_, _>>();
                if class == "network" && writes.load(Ordering::SeqCst) > 0 {
                    match fault {
                        Fault::ServicePolicyChanged => {
                            values.get_mut("OpaqueRef:network-underlay").unwrap()["purpose"] =
                                json!([])
                        }
                        Fault::ServiceNetworkRebound => {
                            values.get_mut("OpaqueRef:network-underlay").unwrap()["uuid"] =
                                json!("00000000-0000-0000-0000-000000000099")
                        }
                        Fault::ServiceNetworkMissing => {
                            values.remove("OpaqueRef:network-underlay");
                        }
                        _ => {}
                    }
                }
                return Some(RpcReply::Json(json!({"result":values})));
            }
            if let Some(class) = method.strip_suffix(".get_by_uuid")
                && ["pool", "host", "network", "PIF", "VLAN", "Bond"].contains(&class)
            {
                let map = records.lock().unwrap();
                let found = map.iter().find(|(key, v)| {
                    key.split('-').next() == Some(class) && v["uuid"] == request["params"][1]
                });
                return Some(RpcReply::Json(match found {
                    Some((key, _)) => json!({"result":format!("OpaqueRef:{key}")}),
                    None => json!({"error":{"code":1,"message":"UUID_INVALID","data":[]}}),
                }));
            }
            if method == "task.get_record" {
                if matches!(fault, Fault::TaskFailed | Fault::TaskCancelled) {
                    return Some(RpcReply::Json(
                        json!({"result":{"status":if fault==Fault::TaskFailed{"failure"}else{"cancelled"},"error_info":[],"allowed_operations":[]}}),
                    ));
                }

                if fault == Fault::TaskRead {
                    return Some(RpcReply::Json(
                        json!({"error":{"code":1,"message":"HANDLE_INVALID","data":[]}}),
                    ));
                }
                return Some(RpcReply::Json(
                    json!({"result":{"status":"success","progress":1.0,"result":format!("<value><string>{}</string></value>",records.lock().unwrap().get("task-result").and_then(Value::as_str).unwrap_or("OpaqueRef:VLAN")),"allowed_operations":[]}}),
                ));
            }
            if let Some(class) = method.strip_suffix(".get_record")
                && ["pool", "host", "network", "PIF", "VLAN", "Bond"].contains(&class)
            {
                if fault == Fault::VerifyRead && writes.load(Ordering::SeqCst) > 0 {
                    return Some(RpcReply::Json(
                        json!({"error":{"code":1,"message":"RBAC_PERMISSION_DENIED","data":[]}}),
                    ));
                }
                let key = request["params"][1]
                    .as_str()
                    .unwrap()
                    .strip_prefix("OpaqueRef:")
                    .unwrap();
                return Some(RpcReply::Json(match records.lock().unwrap().get(key) {
                    Some(v) => json!({"result":v}),
                    None => json!({"error":{"code":1,"message":"HANDLE_INVALID","data":[]}}),
                }));
            }
            if method.starts_with("Async.") {
                let mut map = records.lock().unwrap();
                match method {
                    "Async.VLAN.create" => {
                        assert_eq!(request["params"][1], "OpaqueRef:PIF");
                        assert_eq!(request["params"][2], "100");
                        assert_eq!(request["params"][3], "OpaqueRef:network");
                        map.insert("VLAN".into(),json!({"uuid":VLAN,"tagged_PIF":"OpaqueRef:PIF","untagged_PIF":"OpaqueRef:PIF-child","tag":if fault==Fault::WrongVlan{"101"}else{"100"}}));
                        let mut child = map["PIF"].clone();
                        child["uuid"] = json!(CHILD);
                        child["network"] = json!("OpaqueRef:network");
                        child["VLAN"] = json!("100");
                        child["physical"] = json!(false);
                        map.insert("PIF-child".into(), child);
                        map.get_mut("network").unwrap()["PIFs"] = json!(["OpaqueRef:PIF-child"]);
                    }
                    "Async.Bond.create" => {
                        assert_eq!(request["params"][1], "OpaqueRef:network");
                        assert_eq!(
                            request["params"][2],
                            json!(["OpaqueRef:PIF", "OpaqueRef:PIF-second"])
                        );
                        assert_eq!(request["params"][3], "");
                        assert_eq!(request["params"][4], "lacp");
                        assert_eq!(
                            request["params"][5],
                            json!({"hashing_algorithm":"tcpudp_ports"})
                        );
                        map.insert("Bond".into(),json!({"uuid":"bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb","master":"OpaqueRef:PIF-bond","slaves":["OpaqueRef:PIF","OpaqueRef:PIF-second"],"primary_slave":"OpaqueRef:PIF","mode":"lacp","properties":{"hashing_algorithm":"tcpudp_ports"}}));
                        let mut pif = map["PIF"].clone();
                        pif["uuid"] = json!("cccccccc-cccc-cccc-cccc-cccccccccccc");
                        pif["network"] = json!("OpaqueRef:network");
                        pif["device"] = json!("bond0");
                        pif["physical"] = json!(false);
                        map.insert("PIF-bond".into(), pif);
                        map.insert("task-result".into(), json!("OpaqueRef:Bond"));
                    }
                    "Async.VLAN.destroy" => {
                        assert_eq!(request["params"][1], "OpaqueRef:VLAN");
                        map.remove("VLAN");
                        map.remove("PIF-child");
                        map.get_mut("network").unwrap()["PIFs"] = json!([]);
                    }
                    "Async.Bond.destroy" => {
                        assert_eq!(request["params"][1], "OpaqueRef:Bond");
                        map.remove("Bond");
                        map.remove("PIF-bond");
                        map.get_mut("network").unwrap()["PIFs"] = json!([]);
                    }
                    "Async.PIF.reconfigure_ipv6" => {
                        let pif = map.get_mut("PIF").unwrap();
                        pif["ipv6_configuration_mode"] = request["params"][2].clone();
                        pif["IPv6"] = if request["params"][3] == "" {
                            json!([])
                        } else {
                            json!([request["params"][3].clone()])
                        };
                        pif["ipv6_gateway"] = request["params"][4].clone();
                        pif["DNS"] = request["params"][5].clone();
                    }
                    "Async.PIF.set_other_config" => {
                        map.get_mut("PIF").unwrap()["other_config"] = request["params"][2].clone()
                    }
                    "Async.PIF.set_disallow_unplug" => {
                        map.get_mut("PIF").unwrap()["disallow_unplug"] =
                            request["params"][2].clone()
                    }
                    "Async.PIF.reconfigure_ip" => {
                        let pif = map.get_mut("PIF").unwrap();
                        for (field, index) in [
                            ("ip_configuration_mode", 2),
                            ("IP", 3),
                            ("netmask", 4),
                            ("gateway", 5),
                            ("DNS", 6),
                        ] {
                            pif[field] = request["params"][index].clone();
                        }
                    }
                    "Async.network.add_purpose" | "Async.network.remove_purpose" => {
                        let key = request["params"][1]
                            .as_str()
                            .unwrap()
                            .strip_prefix("OpaqueRef:")
                            .unwrap();
                        let purposes = map.get_mut(key).unwrap()["purpose"].as_array_mut().unwrap();
                        let purpose = request["params"][2].clone();
                        if method.ends_with(".add_purpose") {
                            purposes.push(purpose);
                        } else {
                            purposes.retain(|p| p != &purpose);
                        }
                    }
                    "Async.network.set_MTU" => {
                        map.get_mut("network").unwrap()["MTU"] = request["params"][2].clone();
                    }
                    "Async.network.set_other_config" => {
                        map.get_mut("network").unwrap()["other_config"] =
                            request["params"][2].clone()
                    }
                    "Async.PIF.unplug" => {
                        map.get_mut("PIF").unwrap()["currently_attached"] = json!(false)
                    }
                    "Async.PIF.scan" => assert_eq!(request["params"][1], "OpaqueRef:host"),
                    "Async.pool.set_igmp_snooping_enabled" => {
                        map.get_mut("pool").unwrap()["igmp_snooping_enabled"] =
                            request["params"][2].clone()
                    }
                    _ => return None,
                }
                writes.fetch_add(1, Ordering::SeqCst);
                return Some(if fault == Fault::DropReply {
                    RpcReply::Drop
                } else {
                    RpcReply::Json(json!({"result":"OpaqueRef:network-task"}))
                });
            }
            None
        }
    };
    (mock_overridden(override_rpc(callback)).await, records)
}
async fn connect(mock: &Mock, backend: &Backend, readonly: bool) -> xenadmin_core::Connection {
    let mut options = ConnectionOptions::new(mock.endpoint.clone());
    options.read_only = readonly;
    let connection = backend.connect(
        options,
        Credentials::new("root".into(), "fixture-password".into()),
    );
    connection.wait_ready(deadline(3)).await.unwrap();
    connection
}
fn network() -> ResourceTarget {
    ResourceTarget::Network(NetworkId::parse(NET).unwrap())
}
fn vlan(connection: &xenadmin_core::Connection) -> NetworkOperation {
    let pif = PifId::parse(PIF).unwrap();
    let snapshot = connection.latest();
    let i = snapshot.inventory().unwrap();
    NetworkOperation::CreateVlan {
        parent: pif,
        expected: xenadmin_core::PifIdentity::from_inventory(&i.details.pifs[&pif]).unwrap(),
        tag: 100,
    }
}
async fn finish(
    operation: &xenadmin_core::ResourceOperation,
) -> Arc<xenadmin_core::ResourceOperationRecord> {
    tokio::time::timeout(deadline(4), async {
        loop {
            let r = operation.latest();
            if r.state.terminal() || r.state == OperationState::OutcomeUnknown {
                return r;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}
fn submit(
    connection: &xenadmin_core::Connection,
    target: ResourceTarget,
    operation: NetworkOperation,
) -> xenadmin_core::ResourceOperation {
    connection
        .submit_resource(
            OperationId::new(),
            ResourceCommand::Network {
                target: connection.latest().observe_resource(target).unwrap(),
                operation,
            },
        )
        .unwrap()
}
#[tokio::test]
async fn vlan_creation_verifies_new_graph_and_result_without_leaking_references() {
    let (mock, _) = fixture(Fault::None).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&mock, &backend, false).await;
    let op = submit(&c, network(), vlan(&c));
    let r = finish(&op).await;
    assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
    assert_eq!(
        r.created,
        Some(ResourceTarget::Vlan(
            xenadmin_core::VlanId::parse(VLAN).unwrap()
        ))
    );
    assert!(!serde_json::to_string(&*r).unwrap().contains("OpaqueRef:"));
    assert_eq!(
        mock.methods
            .lock()
            .unwrap()
            .iter()
            .filter(|m| *m == "Async.VLAN.create")
            .count(),
        1
    );
}
#[tokio::test]
async fn vlan_preflight_rejects_changed_identity_and_missing_membership_before_dispatch() {
    for fault in [Fault::ChangedIdentity, Fault::MissingMembers] {
        let (mock, _) = fixture(fault).await;
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&mock, &backend, false).await;
        let op = submit(&c, network(), vlan(&c));
        let r = finish(&op).await;
        assert_eq!(r.state, OperationState::Rejected, "{}", r.detail);
        assert!(
            !mock
                .methods
                .lock()
                .unwrap()
                .iter()
                .any(|m| m == "Async.VLAN.create")
        );
    }
}
#[tokio::test]
async fn task_success_does_not_accept_wrong_or_preexisting_created_objects() {
    for fault in [Fault::WrongVlan, Fault::OldVlan] {
        let (mock, _) = fixture(fault).await;
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&mock, &backend, false).await;
        let op = submit(&c, network(), vlan(&c));
        let r = finish(&op).await;
        assert_eq!(r.state, OperationState::OutcomeUnknown, "{}", r.detail);
        assert!(r.created.is_none());
    }
}
#[tokio::test]
async fn lost_dispatch_response_is_not_inferred_from_matching_topology_or_replayed() {
    let (mock, _) = fixture(Fault::DropReply).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&mock, &backend, false).await;
    let op = submit(&c, network(), vlan(&c));
    let r = finish(&op).await;
    assert_eq!(r.state, OperationState::OutcomeUnknown);
    c.control_resource(r.id, xenadmin_core::OperationControl::Recheck)
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        mock.methods
            .lock()
            .unwrap()
            .iter()
            .filter(|m| *m == "Async.VLAN.create")
            .count(),
        1
    );
    assert!(
        c.resource_history()
            .iter()
            .any(|r| r.state == OperationState::OutcomeUnknown)
    );
}
#[tokio::test]
async fn task_read_and_verification_errors_preserve_unknown_not_rejection() {
    for fault in [Fault::TaskRead, Fault::VerifyRead] {
        let (mock, _) = fixture(fault).await;
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&mock, &backend, false).await;
        let op = submit(&c, network(), vlan(&c));
        let r = finish(&op).await;
        assert_eq!(r.state, OperationState::OutcomeUnknown, "{}", r.detail);
        assert!(
            !mock
                .methods
                .lock()
                .unwrap()
                .iter()
                .any(|m| m == "task.destroy")
        );
    }
}
#[tokio::test]
async fn automatic_attachment_preserves_unrelated_configuration_and_scan_is_task_backed() {
    let (mock, records) = fixture(Fault::None).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&mock, &backend, false).await;
    let op = submit(
        &c,
        network(),
        NetworkOperation::SetAutomaticAttachment {
            expected: None,
            enabled: false,
        },
    );
    assert_eq!(finish(&op).await.state, OperationState::Succeeded);
    assert_eq!(
        records.lock().unwrap()["network"]["other_config"],
        json!({"automatic":"false","unrelated":"preserve"})
    );
    let op = submit(
        &c,
        ResourceTarget::Host(HostId::parse(HOST).unwrap()),
        NetworkOperation::ScanInterfaces,
    );
    assert_eq!(finish(&op).await.state, OperationState::Succeeded);
}
#[tokio::test]
async fn readonly_connections_refuse_network_mutations() {
    let (mock, _) = fixture(Fault::None).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&mock, &backend, true).await;
    let result = c.submit_resource(
        OperationId::new(),
        ResourceCommand::Network {
            target: c.latest().observe_resource(network()).unwrap(),
            operation: vlan(&c),
        },
    );
    assert!(result.is_err());
    assert!(
        !mock
            .methods
            .lock()
            .unwrap()
            .iter()
            .any(|m| m.starts_with("Async."))
    );
}

#[tokio::test]
async fn lacp_creation_sends_reviewed_members_and_hash_and_verifies_new_master() {
    let (mock, _) = fixture(Fault::None).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&mock, &backend, false).await;
    let snap = c.latest();
    let i = snap.inventory().unwrap();
    let members = [PIF, "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"]
        .iter()
        .map(|id| {
            let id = PifId::parse(id).unwrap();
            (
                id,
                xenadmin_core::PifIdentity::from_inventory(&i.details.pifs[&id]).unwrap(),
            )
        })
        .collect();
    let op = submit(
        &c,
        network(),
        NetworkOperation::CreateBond {
            members,
            mode: xenadmin_core::BondMode::Lacp,
            hash: xenadmin_core::BondHash::TcpUdpPorts,
        },
    );
    let result = finish(&op).await;
    assert_eq!(result.state, OperationState::Succeeded, "{}", result.detail);
    assert!(matches!(result.created, Some(ResourceTarget::Bond(_))));
}
#[tokio::test]
async fn ipv4_mtu_and_igmp_edits_verify_actual_settings() {
    let (mock, records) = fixture(Fault::None).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&mock, &backend, false).await;
    let id = PifId::parse(PIF).unwrap();
    let snap = c.latest();
    let expected =
        xenadmin_core::PifIdentity::from_inventory(&snap.inventory().unwrap().details.pifs[&id])
            .unwrap();
    let before = xenadmin_core::Ipv4Configuration {
        mode: xenadmin_core::Ipv4Mode::None,
        address: String::new(),
        netmask: String::new(),
        gateway: String::new(),
        dns: String::new(),
    };
    let configuration = xenadmin_core::Ipv4Configuration {
        mode: xenadmin_core::Ipv4Mode::Static,
        address: "192.0.2.30".into(),
        netmask: "255.255.255.0".into(),
        gateway: String::new(),
        dns: "192.0.2.53".into(),
    };
    let op = submit(
        &c,
        ResourceTarget::Pif(id),
        NetworkOperation::ConfigureIpv4 {
            expected,
            before,
            configuration,
        },
    );
    assert_eq!(finish(&op).await.state, OperationState::Succeeded);
    assert_eq!(records.lock().unwrap()["PIF"]["IP"], "192.0.2.30");
    let op = submit(
        &c,
        network(),
        NetworkOperation::SetMtu {
            expected: 1500,
            mtu: 9000,
            pifs: vec![],
        },
    );
    assert_eq!(finish(&op).await.state, OperationState::Succeeded);
    let op = submit(
        &c,
        ResourceTarget::Pool(xenadmin_core::PoolId::parse(POOL).unwrap()),
        NetworkOperation::SetIgmpSnooping {
            expected: false,
            enabled: true,
        },
    );
    assert_eq!(finish(&op).await.state, OperationState::Succeeded);
}

#[tokio::test]
async fn vlan_deletion_rechecks_vm_consumers_and_preserves_network_record() {
    let (mock, records) = fixture(Fault::None).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&mock, &backend, false).await;
    let op = submit(&c, network(), vlan(&c));
    assert_eq!(finish(&op).await.state, OperationState::Succeeded);
    c.disconnect();
    tokio::time::sleep(Duration::from_millis(30)).await;
    let c = connect(&mock, &backend, false).await;
    let target = ResourceTarget::Vlan(xenadmin_core::VlanId::parse(VLAN).unwrap());
    let operation = NetworkOperation::DeleteVlan {
        parent: PifId::parse(PIF).unwrap(),
        child: PifId::parse(CHILD).unwrap(),
        network: NetworkId::parse(NET).unwrap(),
        tag: 100,
    };
    records.lock().unwrap().get_mut("network").unwrap()["VIFs"] = json!(["OpaqueRef:vm-nic"]);
    assert_eq!(
        finish(&submit(&c, target, operation.clone())).await.state,
        OperationState::Rejected
    );
    assert!(
        !mock
            .methods
            .lock()
            .unwrap()
            .iter()
            .any(|m| m == "Async.VLAN.destroy")
    );
    records.lock().unwrap().get_mut("network").unwrap()["VIFs"] = json!([]);
    assert_eq!(
        finish(&submit(&c, target, operation)).await.state,
        OperationState::Succeeded
    );
    assert!(records.lock().unwrap().contains_key("network"));
}

#[tokio::test]
async fn failed_or_cancelled_vlan_creation_retains_partial_object_and_pool_reservation() {
    for fault in [Fault::TaskFailed, Fault::TaskCancelled] {
        let (mock, records) = fixture(fault).await;
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&mock, &backend, false).await;
        let result = finish(&submit(&c, network(), vlan(&c))).await;
        assert_eq!(result.state, OperationState::OutcomeUnknown);
        assert!(records.lock().unwrap().contains_key("VLAN"));
        assert!(
            !mock
                .methods
                .lock()
                .unwrap()
                .iter()
                .any(|m| m == "task.destroy" || m == "Async.VLAN.destroy")
        );
        let another = submit(
            &c,
            network(),
            NetworkOperation::SetMtu {
                expected: 1500,
                mtu: 9000,
                pifs: vec![],
            },
        );
        assert_eq!(finish(&another).await.state, OperationState::Rejected);
        assert!(
            !mock
                .methods
                .lock()
                .unwrap()
                .iter()
                .any(|m| m == "Async.network.set_MTU")
        );
    }
}

fn ipv6_operation(c: &xenadmin_core::Connection) -> NetworkOperation {
    NetworkOperation::ConfigureIpv6 {
        expected: xenadmin_core::PifIdentity::from_inventory(
            &c.latest().inventory().unwrap().details.pifs[&PifId::parse(PIF).unwrap()],
        )
        .unwrap(),
        before: xenadmin_core::Ipv6Configuration {
            mode: xenadmin_core::Ipv6Mode::None,
            addresses: vec![],
            gateway: String::new(),
            dns: String::new(),
        },
        configuration: xenadmin_core::Ipv6Configuration {
            mode: xenadmin_core::Ipv6Mode::Static,
            addresses: vec!["2001:db8::20/64".into()],
            gateway: "2001:db8::1".into(),
            dns: "2001:db8::53".into(),
        },
    }
}
fn enable_ipv6(records: &Records) {
    let mut map = records.lock().unwrap();
    let mut management = map["PIF"].clone();
    management["uuid"] = json!("eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee");
    management["management"] = json!(true);
    management["device"] = json!("eth2");
    management["ipv6_configuration_mode"] = json!("Static");
    management["IPv6"] = json!(["2001:db8::10/64"]);
    map.insert("PIF-management".into(), management);
}
#[tokio::test]
async fn ipv6_edits_preserve_ipv4_and_purpose_merges_preserve_other_keys() {
    let (mock, records) = fixture(Fault::None).await;
    enable_ipv6(&records);
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&mock, &backend, false).await;
    let target = ResourceTarget::Pif(PifId::parse(PIF).unwrap());
    let op = submit(&c, target, ipv6_operation(&c));
    assert_eq!(finish(&op).await.state, OperationState::Succeeded);
    {
        let map = records.lock().unwrap();
        assert_eq!(map["PIF"]["IPv6"], json!(["2001:db8::20/64"]));
        assert_eq!(map["PIF"]["ip_configuration_mode"], "None");
        assert_eq!(map["PIF"]["IP"], "");
    }
    let expected = xenadmin_core::PifIdentity::from_inventory(
        &c.latest().inventory().unwrap().details.pifs[&PifId::parse(PIF).unwrap()],
    )
    .unwrap();
    let op = submit(
        &c,
        target,
        NetworkOperation::SetInterfacePurpose {
            expected: expected.clone(),
            before: None,
            purpose: Some("Storage fabric".into()),
        },
    );
    assert_eq!(finish(&op).await.state, OperationState::Succeeded);
    assert_eq!(
        records.lock().unwrap()["PIF"]["other_config"],
        json!({"unrelated":"preserve","management_purpose":"Storage fabric"})
    );
    let op = submit(
        &c,
        target,
        NetworkOperation::SetUnplugProtection {
            expected: expected.clone(),
            before: false,
            enabled: true,
        },
    );
    assert_eq!(finish(&op).await.state, OperationState::Succeeded);
    assert_eq!(records.lock().unwrap()["PIF"]["disallow_unplug"], true);
    let op = submit(
        &c,
        target,
        NetworkOperation::SetInterfacePurpose {
            expected,
            before: Some("Storage fabric".into()),
            purpose: None,
        },
    );
    assert_eq!(finish(&op).await.state, OperationState::Succeeded);
    assert_eq!(
        records.lock().unwrap()["PIF"]["other_config"],
        json!({"unrelated":"preserve"})
    );
}
#[tokio::test]
async fn ipv6_refuses_disabled_host_changed_address_and_bond_members_without_dispatch() {
    for fault in [0, 1, 2] {
        let (mock, records) = fixture(Fault::None).await;
        if fault != 0 {
            enable_ipv6(&records);
        }
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&mock, &backend, false).await;
        let operation = ipv6_operation(&c);
        if fault == 1 {
            records.lock().unwrap().get_mut("PIF").unwrap()["IPv6"] = json!(["2001:db8::99/64"]);
        }
        if fault == 2 {
            records.lock().unwrap().get_mut("PIF").unwrap()["bond_slave_of"] =
                json!("OpaqueRef:bond");
        }
        let op = submit(
            &c,
            ResourceTarget::Pif(PifId::parse(PIF).unwrap()),
            operation,
        );
        assert_eq!(finish(&op).await.state, OperationState::Rejected);
        assert!(
            !mock
                .methods
                .lock()
                .unwrap()
                .iter()
                .any(|m| m.starts_with("Async."))
        );
    }
}
#[tokio::test]
async fn ipv6_lost_reply_stays_unknown_and_recheck_does_not_repeat_write() {
    let (mock, records) = fixture(Fault::DropReply).await;
    enable_ipv6(&records);
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&mock, &backend, false).await;
    let op = submit(
        &c,
        ResourceTarget::Pif(PifId::parse(PIF).unwrap()),
        ipv6_operation(&c),
    );
    let record = finish(&op).await;
    assert_eq!(record.state, OperationState::OutcomeUnknown);
    c.control_resource(record.id, xenadmin_core::OperationControl::Recheck)
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        mock.methods
            .lock()
            .unwrap()
            .iter()
            .filter(|m| m.as_str() == "Async.PIF.reconfigure_ipv6")
            .count(),
        1
    );
}

#[tokio::test]
async fn changed_purpose_and_unplug_protection_refuse_stale_or_protected_writes() {
    for purpose_change in [false, true] {
        let (mock, records) = fixture(Fault::None).await;
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&mock, &backend, false).await;
        let pif = PifId::parse(PIF).unwrap();
        let expected = xenadmin_core::PifIdentity::from_inventory(
            &c.latest().inventory().unwrap().details.pifs[&pif],
        )
        .unwrap();
        let operation = if purpose_change {
            records.lock().unwrap().get_mut("PIF").unwrap()["other_config"]["management_purpose"] =
                json!("Concurrent edit");
            NetworkOperation::SetInterfacePurpose {
                expected,
                before: None,
                purpose: Some("Storage".into()),
            }
        } else {
            records.lock().unwrap().get_mut("PIF").unwrap()["disallow_unplug"] = json!(true);
            NetworkOperation::SetAttached {
                expected,
                expected_attached: true,
                attached: false,
            }
        };
        let op = submit(&c, ResourceTarget::Pif(pif), operation);
        assert_eq!(finish(&op).await.state, OperationState::Rejected);
        assert!(
            !mock
                .methods
                .lock()
                .unwrap()
                .iter()
                .any(|m| m.starts_with("Async."))
        );
    }
}

fn service_target() -> ResourceTarget {
    ResourceTarget::Network(NetworkId::parse(UNDERLAY).unwrap())
}
fn service_operation(
    c: &xenadmin_core::Connection,
    purpose: xenadmin_core::NetworkServicePurpose,
    enabled: bool,
) -> NetworkOperation {
    let snapshot = c.latest();
    NetworkOperation::SetServicePurpose {
        review: xenadmin_core::NetworkServiceReview::capture(
            snapshot.inventory().unwrap(),
            NetworkId::parse(UNDERLAY).unwrap(),
        )
        .unwrap(),
        purpose,
        enabled,
    }
}
async fn service_fixture(fault: Fault) -> (Mock, Records) {
    let (mock, records) = fixture(fault).await;
    records.lock().unwrap().get_mut("network-underlay").unwrap()["PIFs"] =
        json!(["OpaqueRef:PIF", "OpaqueRef:PIF-second"]);
    (mock, records)
}
#[tokio::test]
async fn network_services_add_remove_both_modes_with_exact_scope_and_no_other_setting_changes() {
    use xenadmin_core::NetworkServicePurpose as Purpose;
    for purpose in [Purpose::NbdTls, Purpose::NbdPlaintext] {
        for enabled in [true, false] {
            let (mock, records) = service_fixture(Fault::None).await;
            if !enabled {
                records.lock().unwrap().get_mut("network-underlay").unwrap()["purpose"] =
                    json!([purpose.as_str()]);
            }
            let before = records.lock().unwrap().clone();
            let dir = tempfile::tempdir().unwrap();
            let backend = Backend::with_state_directory(dir.path().into()).unwrap();
            let c = connect(&mock, &backend, false).await;
            let r = finish(&submit(
                &c,
                service_target(),
                service_operation(&c, purpose, enabled),
            ))
            .await;
            assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
            let mut after = records.lock().unwrap().clone();
            assert_eq!(
                after["network-underlay"]["purpose"],
                if enabled {
                    json!([purpose.as_str()])
                } else {
                    json!([])
                }
            );
            after.get_mut("network-underlay").unwrap()["purpose"] =
                before["network-underlay"]["purpose"].clone();
            assert_eq!(before, after);
            let methods = mock.methods.lock().unwrap();
            assert_eq!(
                methods.iter().filter(|m| m.starts_with("Async.")).count(),
                1
            );
        }
    }
}
#[tokio::test]
async fn network_services_refuse_pool_conflicts_unknown_policy_and_changed_interface_graph() {
    for case in 0..8 {
        let (mock, records) = service_fixture(Fault::None).await;
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&mock, &backend, false).await;
        let op = service_operation(&c, xenadmin_core::NetworkServicePurpose::NbdTls, true);
        {
            let mut records = records.lock().unwrap();
            match case {
                0 => records.get_mut("network").unwrap()["purpose"] = json!(["insecure_nbd"]),
                1 => {
                    records
                        .get_mut("network")
                        .unwrap()
                        .as_object_mut()
                        .unwrap()
                        .remove("purpose");
                }
                2 => records.get_mut("PIF").unwrap()["IP"] = json!("192.0.2.55"),
                3 => records.get_mut("PIF").unwrap()["currently_attached"] = json!(false),
                4 => {
                    records.get_mut("network-underlay").unwrap()["PIFs"] = json!(["OpaqueRef:PIF"])
                }
                5 => records.get_mut("PIF-second").unwrap()["uuid"] = json!(PIF),
                6 => {
                    records.get_mut("network-underlay").unwrap()["purpose"] =
                        json!(["future-purpose"])
                }
                7 => {
                    records.get_mut("pool").unwrap()["uuid"] =
                        json!("00000000-0000-0000-0000-000000000099")
                }
                _ => unreachable!(),
            }
        }
        let r = finish(&submit(&c, service_target(), op)).await;
        assert_eq!(
            r.state,
            OperationState::Rejected,
            "case {case}: {}",
            r.detail
        );
        assert!(
            !mock
                .methods
                .lock()
                .unwrap()
                .iter()
                .any(|m| m.starts_with("Async."))
        );
    }
}
#[tokio::test]
async fn network_service_lost_reply_retains_guard_and_recheck_never_replays_matching_policy() {
    let (mock, records) = service_fixture(Fault::DropReply).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&mock, &backend, false).await;
    let op = service_operation(&c, xenadmin_core::NetworkServicePurpose::NbdTls, true);
    let r = finish(&submit(&c, service_target(), op.clone())).await;
    assert_eq!(r.state, OperationState::OutcomeUnknown, "{}", r.detail);
    assert_eq!(
        records.lock().unwrap()["network-underlay"]["purpose"],
        json!(["nbd"])
    );
    c.control_resource(r.id, xenadmin_core::OperationControl::Recheck)
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let blocked = finish(&submit(&c, service_target(), op)).await;
    assert_eq!(blocked.state, OperationState::Rejected);
    assert_eq!(
        mock.methods
            .lock()
            .unwrap()
            .iter()
            .filter(|m| *m == "Async.network.add_purpose")
            .count(),
        1
    );
    assert_eq!(
        c.resource_history()
            .iter()
            .find(|row| row.id == r.id)
            .unwrap()
            .state,
        OperationState::OutcomeUnknown
    );
}
#[tokio::test]
async fn network_service_task_or_verification_read_failure_remains_unknown_and_readonly_cannot_write()
 {
    for (fault, readonly) in [
        (Fault::TaskRead, false),
        (Fault::VerifyRead, false),
        (Fault::None, true),
    ] {
        let (mock, _) = service_fixture(fault).await;
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&mock, &backend, readonly).await;
        let result = c.submit_resource(
            OperationId::new(),
            ResourceCommand::Network {
                target: c.latest().observe_resource(service_target()).unwrap(),
                operation: service_operation(
                    &c,
                    xenadmin_core::NetworkServicePurpose::NbdTls,
                    true,
                ),
            },
        );
        if readonly {
            assert!(matches!(result, Err(xenadmin_core::Error::ReadOnly)));
        } else {
            let r = finish(&result.unwrap()).await;
            assert_eq!(r.state, OperationState::OutcomeUnknown, "{}", r.detail);
        }
        assert_eq!(
            mock.methods
                .lock()
                .unwrap()
                .iter()
                .filter(|m| m.starts_with("Async."))
                .count(),
            usize::from(!readonly)
        );
    }
}

#[tokio::test]
async fn network_service_verification_uses_one_current_pool_map_not_an_older_selected_record() {
    for fault in [
        Fault::ServicePolicyChanged,
        Fault::ServiceNetworkRebound,
        Fault::ServiceNetworkMissing,
    ] {
        let (mock, _) = service_fixture(fault).await;
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&mock, &backend, false).await;
        let r = finish(&submit(
            &c,
            service_target(),
            service_operation(&c, xenadmin_core::NetworkServicePurpose::NbdTls, true),
        ))
        .await;
        assert_eq!(r.state, OperationState::OutcomeUnknown, "{}", r.detail);
        assert_eq!(
            mock.methods
                .lock()
                .unwrap()
                .iter()
                .filter(|m| *m == "Async.network.add_purpose")
                .count(),
            1
        );
    }
}
