use super::*;
use std::collections::BTreeMap;
use xenadmin_core::{
    NetworkId, NetworkTopologyOperation, OperationControl, OperationId, OperationState, PifId,
    PifIdentity, ResourceCommand, ResourceOperation, ResourceOperationRecord, ResourceTarget,
    ReviewedVlan, VlanUplink,
};
fn id(value: u128) -> uuid::Uuid {
    uuid::Uuid::from_u128(value)
}
fn reference(class: &str, value: u128) -> String {
    format!("OpaqueRef:{class}:{value}")
}
#[derive(Clone, Copy, PartialEq)]
enum Fault {
    None,
    DropReply,
    TaskFailure,
    WrongResult,
    OfflineSecondAfterFirst,
    VerificationRead,
    ReboundBeforeUnplug,
    ConfigureBeforeUnplug,
    Pending,
}
struct Control {
    records: BTreeMap<String, Value>,
    fault: Fault,
    writes: usize,
    task_reads: usize,
    next: u128,
    delay_submit: bool,
    entered: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
}
fn rows(retag: bool) -> BTreeMap<String, Value> {
    let mut rows = BTreeMap::from([
        (
            reference("pool", 1),
            json!({"uuid":id(1),"name_label":"Topology pool","master":reference("host",2)}),
        ),
        (
            reference("network", 5),
            json!({"uuid":id(5),"name_label":"VLAN network","bridge":"xapi0","MTU":"1500","managed":true,"PIFs":[],"VIFs":[],"other_config":{}}),
        ),
        (
            reference("network", 6),
            json!({"uuid":id(6),"name_label":"Uplinks","bridge":"xenbr0","MTU":"1500","managed":true,"PIFs":[reference("PIF",12),reference("PIF",13)],"VIFs":[],"other_config":{}}),
        ),
    ]);
    for host in [2, 3] {
        rows.insert(reference("host",host),json!({"uuid":id(host),"name_label":format!("Host {host}"),"address":format!("192.0.2.{host}"),"enabled":true,"metrics":reference("host_metrics",host)}));
        rows.insert(
            reference("host_metrics", host),
            json!({"uuid":id(host+50),"live":true}),
        );
        let pif = json!({"uuid":id(host+10),"host":reference("host",host),"network":reference("network",6),"device":"eth1","MAC":format!("02:00:00:00:00:0{host}"),"MTU":"1500","VLAN":"-1","physical":true,"management":false,"managed":true,"disallow_unplug":false,"currently_attached":true,"bond_slave_of":"OpaqueRef:NULL","bond_master_of":[],"VLAN_slave_of":[],"VLAN_master_of":"OpaqueRef:NULL","tunnel_access_PIF_of":[],"tunnel_transport_PIF_of":[],"ip_configuration_mode":"None","IP":"","netmask":"","gateway":"","DNS":"","ipv6_configuration_mode":"None","IPv6":[""],"ipv6_gateway":"","primary_address_type":"IPv4","other_config":{},"properties":{}});
        rows.insert(reference("PIF", host + 10), pif.clone());
        if retag {
            let mut child = pif;
            child["uuid"] = json!(id(host + 30));
            child["network"] = json!(reference("network", 5));
            child["VLAN"] = json!("40");
            child["physical"] = json!(false);
            child["currently_attached"] = json!(host == 2);
            child["VLAN_master_of"] = json!(reference("VLAN", host + 20));
            rows.insert(reference("PIF", host + 30), child);
            rows.insert(reference("VLAN",host+20),json!({"uuid":id(host+20),"tagged_PIF":reference("PIF",host+10),"untagged_PIF":reference("PIF",host+30),"tag":"40","other_config":{}}));
            rows.get_mut(&reference("PIF", host + 10)).unwrap()["VLAN_slave_of"] =
                json!([reference("VLAN", host + 20)]);
            rows.get_mut(&reference("network", 5)).unwrap()["PIFs"]
                .as_array_mut()
                .unwrap()
                .push(json!(reference("PIF", host + 30)));
        }
    }
    rows
}
async fn fixture(retag: bool, fault: Fault) -> (Mock, Arc<Mutex<Control>>) {
    let control = Arc::new(Mutex::new(Control {
        records: rows(retag),
        fault,
        writes: 0,
        task_reads: 0,
        next: 100,
        delay_submit: false,
        entered: Arc::new(tokio::sync::Notify::new()),
        release: Arc::new(tokio::sync::Notify::new()),
    }));
    let captured = control.clone();
    let mock=mock_overridden(override_rpc(move|request:Value|{let control=captured.clone();async move {
        let method=request["method"].as_str().unwrap();
        let gate = if method=="Async.VLAN.create" {let mut s=control.lock().unwrap();std::mem::take(&mut s.delay_submit).then(||(s.entered.clone(),s.release.clone()))}else{None};
        if let Some((entered,release))=gate {entered.notify_one();release.notified().await;}
        let mut state=control.lock().unwrap();
        if method=="host.get_all_records" && state.writes==4 {
            if state.fault==Fault::ReboundBeforeUnplug {
                let mut child=state.records[&reference("PIF",201)].clone();child["uuid"]=json!(id(299));
                state.records.insert(reference("PIF",299),child);
                let vlan=state.records.get_mut(&reference("VLAN",101)).unwrap();vlan["uuid"]=json!(id(999));vlan["untagged_PIF"]=json!(reference("PIF",299));
            } else if state.fault==Fault::ConfigureBeforeUnplug {
                state.records.get_mut(&reference("PIF",201)).unwrap()["other_config"]=json!({"management_purpose":"Storage"});
            }
        }
        if method=="event.from" && request["params"][2]=="" {
            let mut batch=batch("initial","Halted");batch["events"].as_array_mut().unwrap().retain(|v|v["class"]=="VM");
            for (reference,record) in &state.records {
                let class=reference.split(':').nth(1).unwrap();
                batch["events"].as_array_mut().unwrap().push(json!({"class":class,"operation":"add","ref":reference,"snapshot":record}));
            }
            return Some(RpcReply::Json(json!({"result":batch})))
        }
        if let Some(class)=method.strip_suffix(".get_all_records") && ["pool","host","host_metrics","network","PIF","VLAN","task"].contains(&class) {
            let rows=state.records.iter().filter(|(reference,_)|reference.split(':').nth(1)==Some(class)).map(|(r,v)|(r.clone(),v.clone())).collect::<serde_json::Map<_,_>>();
            return Some(RpcReply::Json(json!({"result":rows})))
        }
        if let Some(class)=method.strip_suffix(".get_by_uuid") && ["pool","host","host_metrics","network","PIF","VLAN","task"].contains(&class) {
            let found=state.records.iter().find(|(r,v)|r.split(':').nth(1)==Some(class)&&v["uuid"]==request["params"][1]);
            return Some(RpcReply::Json(match found {Some((r,_))=>json!({"result":r}),None=>json!({"error":{"code":1,"message":"UUID_INVALID","data":[]}})}))
        }
        if let Some(class)=method.strip_suffix(".get_record") && ["pool","host","host_metrics","network","PIF","VLAN","task"].contains(&class) {
            if class=="VLAN" && state.writes>0 && state.fault==Fault::VerificationRead {return Some(RpcReply::Json(json!({"error":{"code":1,"message":"RBAC_PERMISSION_DENIED","data":[]}})))}
            if class=="task" && request["params"][1]==reference("task",1) {state.task_reads+=1;}
            let value=state.records.get(request["params"][1].as_str().unwrap()).cloned();
            if class=="task" && state.writes==1 && state.fault==Fault::OfflineSecondAfterFirst {state.records.get_mut(&reference("host_metrics",3)).unwrap()["live"]=json!(false);}
            return Some(RpcReply::Json(match value {Some(v)=>json!({"result":v}),None=>json!({"error":{"code":1,"message":"HANDLE_INVALID","data":[]}})}))
        }
        if method=="task.destroy" {state.records.remove(request["params"][1].as_str().unwrap());return Some(RpcReply::Json(json!({"result":null})))}
        if method.starts_with("Async.") {
            let mut result=String::new();
            match method {
                "Async.VLAN.create"=>{
                    let parent=request["params"][1].as_str().unwrap();let tag=request["params"][2].as_str().unwrap();let net=request["params"][3].as_str().unwrap();
                    assert!(matches!(tag,"0"|"100"));assert_eq!(net,reference("network",5));
                    let n=state.next;state.next+=1;
                    let vlan=reference("VLAN",n);let pif=reference("PIF",n+100);
                    let mut child=state.records[parent].clone();child["uuid"]=json!(id(n+100));child["network"]=json!(net);child["VLAN"]=json!(tag);child["physical"]=json!(false);child["management"]=json!(false);child["currently_attached"]=json!(true);child["VLAN_slave_of"]=json!([]);child["VLAN_master_of"]=json!(vlan);
                    state.records.insert(pif.clone(),child);
                    state.records.insert(vlan.clone(),json!({"uuid":id(n),"tagged_PIF":parent,"untagged_PIF":pif,"tag":tag,"other_config":{}}));
                    state.records.get_mut(parent).unwrap()["VLAN_slave_of"].as_array_mut().unwrap().push(json!(&vlan));
                    state.records.get_mut(net).unwrap()["PIFs"].as_array_mut().unwrap().push(json!(pif));
                    result=if state.fault==Fault::WrongResult {reference("VLAN",22)}else{vlan};
                }
                "Async.VLAN.destroy"=>{
                    let r=request["params"][1].as_str().unwrap();let vlan=state.records.remove(r).unwrap();
                    let pif=state.records.remove(vlan["untagged_PIF"].as_str().unwrap()).unwrap();
                    state.records.get_mut(vlan["tagged_PIF"].as_str().unwrap()).unwrap()["VLAN_slave_of"].as_array_mut().unwrap().retain(|v|v!=r);
                    state.records.get_mut(pif["network"].as_str().unwrap()).unwrap()["PIFs"].as_array_mut().unwrap().retain(|v|v!=&vlan["untagged_PIF"]);
                }
                "Async.PIF.unplug"=>{state.records.get_mut(request["params"][1].as_str().unwrap()).unwrap()["currently_attached"]=json!(false);}
                _=>return None,
            }
            state.writes+=1;
            let task=reference("task",state.writes as u128);
            let status=if state.fault==Fault::TaskFailure {"failure"}else if state.fault==Fault::Pending {"pending"}else{"success"};
            let writes=state.writes;
            state.records.insert(task.clone(),json!({"uuid":id(1000+writes as u128),"status":status,"progress":1.0,"allowed_operations":["cancel"],"result":format!("<value><string>{result}</string></value>")}));
            return Some(if state.fault==Fault::DropReply {RpcReply::Drop}else{RpcReply::Json(json!({"result":task}))})
        }
        None
    }})).await;
    (mock, control)
}
async fn connect(mock: &Mock, backend: &Backend) -> xenadmin_core::Connection {
    let connection = backend.connect(
        ConnectionOptions::new(mock.endpoint.clone()),
        Credentials::new("root".into(), "fixture-password".into()),
    );
    connection.wait_ready(deadline(4)).await.unwrap();
    connection
}
fn operation(connection: &xenadmin_core::Connection, retag: bool) -> NetworkTopologyOperation {
    let snapshot = connection.latest();
    let inventory = snapshot.inventory().unwrap();
    let uplinks = [2, 3]
        .into_iter()
        .map(|host| {
            let pif = PifId::from(id(host + 10));
            VlanUplink {
                pif,
                expected: PifIdentity::from_inventory(&inventory.details.pifs[&pif]).unwrap(),
            }
        })
        .collect::<Vec<_>>();
    if retag {
        NetworkTopologyOperation::RetagVlans {
            interfaces: uplinks
                .into_iter()
                .zip([2, 3])
                .map(|(uplink, host)| ReviewedVlan {
                    vlan: id(host + 20).into(),
                    child: id(host + 30).into(),
                    uplink,
                    tag: 40,
                    attached: host == 2,
                })
                .collect(),
            tag: 100,
        }
    } else {
        NetworkTopologyOperation::CreateVlans { uplinks, tag: 100 }
    }
}
fn submit(
    connection: &xenadmin_core::Connection,
    operation: NetworkTopologyOperation,
) -> ResourceOperation {
    connection
        .submit_resource(
            OperationId::new(),
            ResourceCommand::NetworkTopology {
                target: connection
                    .latest()
                    .observe_resource(ResourceTarget::Network(NetworkId::from(id(5))))
                    .unwrap(),
                operation,
            },
        )
        .unwrap()
}
async fn finish(operation: &ResourceOperation) -> Arc<ResourceOperationRecord> {
    tokio::time::timeout(deadline(5), async {
        loop {
            let row = operation.latest();
            if row.state.terminal() || row.state == OperationState::OutcomeUnknown {
                return row;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
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
#[tokio::test]
async fn pool_wide_creation_and_retagging_verify_exact_graph_and_preserve_unplugged_state() {
    for retag in [false, true] {
        let (mock, control) = fixture(retag, Fault::None).await;
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&mock, &backend).await;
        let op = submit(&c, operation(&c, retag));
        let result = finish(&op).await;
        assert_eq!(result.state, OperationState::Succeeded, "{}", result.detail);
        assert_eq!(result.topology_changes.created_vlans.len(), 2);
        assert_eq!(result.topology_changes.created_pifs.len(), 2);
        assert_eq!(
            result.topology_changes.removed_vlans.len(),
            if retag { 2 } else { 0 }
        );
        assert_eq!(count(&mock, "Async.VLAN.create"), 2);
        assert_eq!(
            count(&mock, "Async.VLAN.destroy"),
            if retag { 2 } else { 0 }
        );
        assert_eq!(count(&mock, "Async.PIF.unplug"), usize::from(retag));
        assert!(
            !serde_json::to_string(result.as_ref())
                .unwrap()
                .contains("OpaqueRef:")
        );
        let rows = &control.lock().unwrap().records;
        for pif in &result.topology_changes.created_pifs {
            let row = rows
                .values()
                .find(|r| r["uuid"] == pif.to_string())
                .unwrap();
            assert_eq!(row["VLAN"], "100");
            assert_eq!(
                row["currently_attached"],
                !retag || row["host"] == reference("host", 2)
            );
        }
    }
}
#[tokio::test]
async fn native_vlan_zero_uses_the_explicit_api_tag() {
    let (mock, _) = fixture(false, Fault::None).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&mock, &backend).await;
    let mut request = operation(&c, false);
    let NetworkTopologyOperation::CreateVlans { tag, .. } = &mut request else {
        panic!()
    };
    *tag = 0;
    let result = finish(&submit(&c, request)).await;
    assert_eq!(result.state, OperationState::Succeeded, "{}", result.detail);
}
#[tokio::test]
async fn offline_missing_or_changed_pool_members_and_consumers_refuse_before_writes() {
    for change in 0..5 {
        let (mock, control) = fixture(false, Fault::None).await;
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&mock, &backend).await;
        let mut request = operation(&c, false);
        {
            let mut state = control.lock().unwrap();
            match change {
                0 => {
                    state
                        .records
                        .get_mut(&reference("host_metrics", 3))
                        .unwrap()["live"] = json!(false)
                }
                1 => {
                    state.records.remove(&reference("host", 3));
                }
                2 => {
                    state.records.get_mut(&reference("network", 5)).unwrap()["VIFs"] =
                        json!(["OpaqueRef:VIF:foreign"])
                }
                3 => {
                    if let NetworkTopologyOperation::CreateVlans { uplinks, .. } = &mut request {
                        uplinks.pop();
                    }
                }
                _ => state.records.get_mut(&reference("pool", 1)).unwrap()["uuid"] = json!(id(99)),
            }
        }
        let result = finish(&submit(&c, request)).await;
        assert_eq!(result.state, OperationState::Rejected, "{}", result.detail);
        assert_eq!(control.lock().unwrap().writes, 0);
    }
}
#[tokio::test]
async fn a_later_offline_host_retains_verified_created_interfaces_as_partial() {
    let (mock, _) = fixture(false, Fault::OfflineSecondAfterFirst).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&mock, &backend).await;
    let result = finish(&submit(&c, operation(&c, false))).await;
    assert_eq!(
        result.state,
        OperationState::PartiallyCompleted,
        "{}",
        result.detail
    );
    assert_eq!(result.topology_changes.created_vlans.len(), 1);
    assert_eq!(count(&mock, "Async.VLAN.create"), 1);
}
#[tokio::test]
async fn lost_or_failed_task_results_hold_the_pool_and_never_replay_creation() {
    for fault in [Fault::DropReply, Fault::TaskFailure] {
        let (mock, _) = fixture(false, fault).await;
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&mock, &backend).await;
        let op = submit(&c, operation(&c, false));
        let row = finish(&op).await;
        assert_eq!(row.state, OperationState::OutcomeUnknown, "{}", row.detail);
        c.control_resource(row.id, OperationControl::Recheck)
            .unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(count(&mock, "Async.VLAN.create"), 1);
        let other = finish(&submit(&c, operation(&c, false))).await;
        assert_eq!(other.state, OperationState::Rejected);
    }
}

#[tokio::test]
async fn misbound_existing_vlan_result_is_never_adopted_or_followed_by_more_writes() {
    let (mock, control) = fixture(false, Fault::WrongResult).await;
    {
        let mut state = control.lock().unwrap();
        let existing = rows(true);
        for class in ["VLAN", "PIF"] {
            let number = if class == "VLAN" { 22 } else { 32 };
            let mut row = existing[&reference(class, number)].clone();
            if class == "PIF" {
                row["network"] = json!(reference("network", 7));
            }
            state.records.insert(reference(class, number), row);
        }
        state.records.insert(reference("network",7),json!({"uuid":id(7),"managed":true,"MTU":"1500","PIFs":[reference("PIF",32)],"VIFs":[]}));
        state.records.get_mut(&reference("PIF", 12)).unwrap()["VLAN_slave_of"] =
            json!([reference("VLAN", 22)]);
    }
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&mock, &backend).await;
    let result = finish(&submit(&c, operation(&c, false))).await;
    assert_eq!(
        result.state,
        OperationState::OutcomeUnknown,
        "{}",
        result.detail
    );
    assert!(result.topology_changes.created_vlans.is_empty());
    assert!(result.topology_changes.created_pifs.is_empty());
    assert_eq!(count(&mock, "Async.VLAN.create"), 1);
}
#[tokio::test]
async fn recorded_vlan_completion_survives_removed_task_and_restart_without_continuing_plan() {
    let (mock, control) = fixture(false, Fault::VerificationRead).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&mock, &backend).await;
    let result = finish(&submit(&c, operation(&c, false))).await;
    assert_eq!(
        result.state,
        OperationState::OutcomeUnknown,
        "{}",
        result.detail
    );
    c.close().await;
    {
        let mut state = control.lock().unwrap();
        state.fault = Fault::None;
        state
            .records
            .retain(|r, _| !r.starts_with("OpaqueRef:task:"));
    }
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&mock, &backend).await;
    c.control_resource(result.id, OperationControl::Recheck)
        .unwrap();
    let after = tokio::time::timeout(deadline(5), async {
        loop {
            if let Some(row) = c
                .resource_history()
                .iter()
                .find(|r| r.id == result.id && r.state.terminal())
            {
                break row.clone();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        after.state,
        OperationState::PartiallyCompleted,
        "{}",
        after.detail
    );
    assert_eq!(after.topology_changes.created_vlans, vec![id(100).into()]);
    assert_eq!(after.topology_changes.created_pifs, vec![id(200).into()]);
    assert_eq!(count(&mock, "Async.VLAN.create"), 1);
    c.close().await;
}
#[tokio::test]
async fn retag_revalidates_interface_configuration_and_topology_before_deletion() {
    for (field, value) in [
        ("management", json!(true)),
        ("disallow_unplug", json!(true)),
        ("other_config", json!({"management_purpose":"Storage"})),
        ("IP", json!("192.0.2.200")),
        ("ipv6_configuration_mode", json!("DHCP")),
        ("currently_attached", json!(false)),
        (
            "tunnel_transport_PIF_of",
            json!(["OpaqueRef:tunnel:foreign"]),
        ),
        ("VLAN_slave_of", json!(["OpaqueRef:VLAN:foreign"])),
        ("properties", json!({"custom":"value"})),
    ] {
        let (mock, control) = fixture(true, Fault::None).await;
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&mock, &backend).await;
        let op = operation(&c, true);
        control
            .lock()
            .unwrap()
            .records
            .get_mut(&reference("PIF", 32))
            .unwrap()[field] = value;
        let result = finish(&submit(&c, op)).await;
        assert_eq!(
            result.state,
            OperationState::Rejected,
            "{field}: {}",
            result.detail
        );
        assert_eq!(control.lock().unwrap().writes, 0, "{field}");
        c.close().await;
    }
}

#[tokio::test]
async fn late_vlan_task_receipts_survive_recovery_without_reviving_original_worker() {
    for acknowledge in [false, true] {
        let (mock, control) = fixture(false, Fault::None).await;
        let (entered, release) = {
            let mut state = control.lock().unwrap();
            state.delay_submit = true;
            (state.entered.clone(), state.release.clone())
        };
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&mock, &backend).await;
        let op = submit(&c, operation(&c, false));
        tokio::time::timeout(deadline(5), entered.notified())
            .await
            .unwrap();
        let recovered_backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let recovered = connect(&mock, &recovered_backend).await;
        async fn state(c: &xenadmin_core::Connection, id: OperationId, expected: OperationState) {
            tokio::time::timeout(deadline(5), async {
                loop {
                    if c.resource_history().iter().any(|r| {
                        r.id == id
                            && r.state == expected
                            && (expected != OperationState::OutcomeUnknown
                                || r.steps.first().is_some_and(|step| {
                                    step.state == xenadmin_core::StepState::OutcomeUnknown
                                }))
                    }) {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap_or_else(|_| {
                let rows = c.resource_history();
                let row = rows.iter().find(|r| r.id == id);
                panic!(
                    "waiting for {expected:?}: {:?}",
                    row.map(|r| (&r.state, r.revision(), &r.detail))
                );
            });
        }
        // Startup publishes Unknown before its automatic read-only observer starts.
        // Wait for the dispatched step's unknown result, so acknowledgement cannot
        // race that observer's intervening Verifying state.
        state(&recovered, op.latest().id, OperationState::OutcomeUnknown).await;
        if acknowledge {
            recovered
                .control_resource(op.latest().id, OperationControl::AcknowledgeUnknown)
                .unwrap();
            state(
                &recovered,
                op.latest().id,
                OperationState::AcknowledgedUnknown,
            )
            .await;
        }
        // Retire the recovery reader so subsequent reads can only come from the original worker.
        recovered.close().await;
        let task_reads = control.lock().unwrap().task_reads;
        release.notify_one();
        let result = finish(&op).await;
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
        assert_eq!(count(&mock, "Async.VLAN.create"), 1);
        assert_eq!(control.lock().unwrap().task_reads, task_reads);
        let db = rusqlite::Connection::open(dir.path().join("operations.sqlite3")).unwrap();
        let payload: String = db
            .query_row(
                "SELECT payload FROM resource_operations WHERE id=?1",
                [result.id.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        let payload: Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(
            payload["network_topology_plan"]["steps"][0]["task"],
            reference("task", 1)
        );
        assert!(
            !serde_json::to_string(result.as_ref())
                .unwrap()
                .contains("OpaqueRef:")
        );
        c.close().await;
    }
}

#[tokio::test]
async fn replacement_identity_and_configuration_are_rechecked_before_restoring_unplugged_state() {
    for fault in [Fault::ReboundBeforeUnplug, Fault::ConfigureBeforeUnplug] {
        let (mock, _) = fixture(true, fault).await;
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&mock, &backend).await;
        let result = finish(&submit(&c, operation(&c, true))).await;
        assert_eq!(
            result.state,
            OperationState::PartiallyCompleted,
            "{}",
            result.detail
        );
        assert_eq!(
            result.topology_changes.created_pifs,
            vec![id(200).into(), id(201).into()]
        );
        assert_eq!(count(&mock, "Async.PIF.unplug"), 0);
        c.close().await;
    }
}
#[tokio::test]
async fn recheck_does_not_dispatch_a_previously_recorded_cancellation_intent() {
    let (mock, control) = fixture(false, Fault::Pending).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&mock, &backend).await;
    let op = submit(&c, operation(&c, false));
    tokio::time::timeout(deadline(5), async {
        while control.lock().unwrap().task_reads == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    c.close().await;
    let result = finish(&op).await;
    assert_eq!(
        result.state,
        OperationState::OutcomeUnknown,
        "{}",
        result.detail
    );
    let db = rusqlite::Connection::open(dir.path().join("operations.sqlite3")).unwrap();
    let raw: String = db
        .query_row(
            "SELECT payload FROM resource_operations WHERE id=?1",
            [result.id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    let mut payload: Value = serde_json::from_str(&raw).unwrap();
    payload["cancel_requested"] = json!(true);
    db.execute(
        "UPDATE resource_operations SET payload=?1 WHERE id=?2",
        rusqlite::params![payload.to_string(), result.id.to_string()],
    )
    .unwrap();
    drop(db);
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&mock, &backend).await;
    let before_reads = control.lock().unwrap().task_reads;
    c.control_resource(result.id, OperationControl::Recheck)
        .unwrap();
    tokio::time::timeout(deadline(5), async {
        loop {
            if control.lock().unwrap().task_reads > before_reads
                && c.resource_history().iter().any(|r| {
                    r.id == result.id
                        && r.revision() > result.revision() + 1
                        && r.state == OperationState::OutcomeUnknown
                })
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(count(&mock, "task.cancel"), 0);
    assert_eq!(count(&mock, "Async.VLAN.create"), 1);
    c.close().await;
}

#[tokio::test]
async fn an_expired_plan_keeps_completed_first_host_without_dispatching_remaining_hosts() {
    let (mock, control) = fixture(false, Fault::Pending).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&mock, &backend).await;
    let op = submit(&c, operation(&c, false));
    tokio::time::timeout(deadline(5), async {
        while op.latest().state != OperationState::Tracking || !op.latest().can_cancel {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let db = rusqlite::Connection::open(dir.path().join("operations.sqlite3")).unwrap();
    db.execute("UPDATE resource_operations SET payload=json_set(payload,'$.network_topology_plan.expires_at_ms',json_extract(payload,'$.created_at_ms')), revision=revision+1 WHERE id=?1",[op.latest().id.to_string()]).unwrap();
    drop(db);
    control
        .lock()
        .unwrap()
        .records
        .get_mut(&reference("task", 1))
        .unwrap()["status"] = json!("success");
    let result = finish(&op).await;
    assert_eq!(
        result.state,
        OperationState::PartiallyCompleted,
        "{}",
        result.detail
    );
    assert_eq!(result.topology_changes.created_vlans, vec![id(100).into()]);
    assert_eq!(count(&mock, "Async.VLAN.create"), 1);
    c.close().await;
}
