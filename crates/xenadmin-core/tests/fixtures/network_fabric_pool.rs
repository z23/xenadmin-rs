use super::*;
use xenadmin_core::{NetworkFabricPool, NetworkTopologyOperation};
const HOST2: &str = "22222222-2222-2222-2222-222222222223";
const PIF2: &str = "77777777-7777-7777-7777-777777777778";
pub(super) async fn request(
    records: &Records,
    writes: &AtomicUsize,
    r: &Value,
) -> Option<RpcReply> {
    let method = r["method"].as_str().unwrap();
    if method == "PIF.get_all_records"
        && records.lock().unwrap().contains_key("_pool_deny_after")
        && writes.load(Ordering::SeqCst) > 0
    {
        return Some(RpcReply::Json(
            json!({"error":{"code":1,"message":"RBAC_PERMISSION_DENIED","data":[]}}),
        ));
    }
    if method == "task.get_record" {
        let hold = {
            let mut map = records.lock().unwrap();
            if map.contains_key("_pool_hold") {
                if map.contains_key("_pool_read") {
                    return Some(RpcReply::Json(
                        json!({"error":{"code":1,"message":"HANDLE_INVALID","data":[]}}),
                    ));
                }
                map.insert("_pool_read".into(), json!(true));
                true
            } else {
                false
            }
        };
        if hold {
            while records.lock().unwrap().contains_key("_pool_hold") {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }
    }
    if method == "task.cancel" {
        let mut m = records.lock().unwrap();
        let count = m.get("_pool_cancel").and_then(Value::as_u64).unwrap_or(0);
        m.insert("_pool_cancel".into(), json!(count + 1));
        return Some(RpcReply::Json(json!({"result":""})));
    }
    if method == "task.destroy" {
        records.lock().unwrap().remove(
            r["params"][1]
                .as_str()
                .unwrap()
                .trim_start_matches("OpaqueRef:"),
        );
        return Some(RpcReply::Json(json!({"result":""})));
    }
    if !method.starts_with("Async.Tunnel.") && !method.starts_with("Async.Network_sriov.") {
        return None;
    }
    let mut m = records.lock().unwrap();
    let index = writes.fetch_add(1, Ordering::SeqCst) + 1;
    let class = if method.contains("Tunnel") {
        "Tunnel"
    } else {
        "Network_sriov"
    };
    let (parent, child, parents, children) = if class == "Tunnel" {
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
    let object_key = format!("{class}-pool{index}");
    let child_key = format!("PIF-pool{index}");
    let object_ref = format!("OpaqueRef:{object_key}");
    let child_ref = format!("OpaqueRef:{child_key}");
    let input = r["params"][1]
        .as_str()
        .unwrap()
        .trim_start_matches("OpaqueRef:");
    let mut result = object_ref.clone();
    if method.ends_with("create") {
        let original = m[input].clone();
        let object_id = uuid::Uuid::from_u128(0x88000000000000000000000000000000 + index as u128);
        let child_id = uuid::Uuid::from_u128(0x99000000000000000000000000000000 + index as u128);
        let mut new = pif();
        new["uuid"] = json!(child_id);
        new["host"] = original["host"].clone();
        new["physical"] = json!(false);
        new["capabilities"] = json!([]);
        new["PCI"] = json!("OpaqueRef:NULL");
        new["network"] = json!("OpaqueRef:network");
        new[children] = json!([object_ref]);
        if class == "Tunnel" {
            new["MTU"] = json!("-1");
            new["device"] = json!(format!("tunnel{index}"));
        } else {
            new["MAC"] = json!("fe:ff:ff:ff:ff:ff");
        }
        m.insert(object_key.clone(),json!({"uuid":object_id,parent:r["params"][1],child:child_ref,"protocol":r["params"].get(3).cloned().unwrap_or(json!("gre")),"status":{"active":"false"},"requires_reboot":false,"configuration_mode":"sysfs"}));
        m.insert(child_key, new);
        m.get_mut(input).unwrap()[parents] = json!([object_ref]);
        m.get_mut("network").unwrap()["PIFs"]
            .as_array_mut()
            .unwrap()
            .push(json!(child_ref));
        if m.contains_key("_pool_rebind") {
            result = "OpaqueRef:old-object".into();
        }
    } else {
        let old = m.remove(input).unwrap();
        let old_child = old[child].as_str().unwrap();
        m.remove(old_child.trim_start_matches("OpaqueRef:"));
        let parent_key = old[parent]
            .as_str()
            .unwrap()
            .trim_start_matches("OpaqueRef:");
        m.get_mut(parent_key).unwrap()[parents] = json!([]);
        m.get_mut("network").unwrap()["PIFs"]
            .as_array_mut()
            .unwrap()
            .retain(|v| v != old_child);
    }
    if index > 1 && m.contains_key("_pool_reuse_result") {
        result = format!("OpaqueRef:{class}-pool1");
    }
    let task_number = if index > 1 && m.contains_key("_pool_reuse_task") {
        1
    } else {
        index
    };
    let status = if m.contains_key("_pool_pending") {
        "pending"
    } else if m.contains_key("_pool_fail") {
        "failure"
    } else {
        "success"
    };
    m.insert(format!("task-pool{index}"),json!({"uuid":uuid::Uuid::from_u128(0xaa000000000000000000000000000000+task_number as u128),"status":status,"progress":1.0,"allowed_operations":["cancel"],"error_info":["FIXTURE_FAILURE"],"result":format!("<value><string>{result}</string></value>")}));
    if m.contains_key("_pool_change_after") && index == 1 {
        m.get_mut("PIF-two").unwrap()["MAC"] = json!("02:00:00:00:00:ff");
    }
    if m.contains_key("_pool_drop") {
        Some(RpcReply::Drop)
    } else {
        Some(RpcReply::Json(
            json!({"result":format!("OpaqueRef:task-pool{index}")}),
        ))
    }
}
async fn setup(kind: Kind) -> (Mock, Records, Arc<AtomicUsize>) {
    let (m, r, w) = fixture(kind, Fault::None).await;
    {
        let mut map = r.lock().unwrap();
        let mut host = map["host"].clone();
        host["uuid"] = json!(HOST2);
        host["name_label"] = json!("Synthetic second host");
        host["metrics"] = json!("OpaqueRef:host_metrics-two");
        map.insert("host-two".into(), host);
        map.insert(
            "host_metrics-two".into(),
            json!({"uuid":"00000000-0000-0000-0000-000000000011","live":true}),
        );
        let mut p = map["PIF"].clone();
        p["uuid"] = json!(PIF2);
        p["host"] = json!("OpaqueRef:host-two");
        p["PCI"] = json!("OpaqueRef:PCI-two");
        if matches!(kind, Kind::Tunnel(_)) {
            p["IP"] = json!("192.0.2.21");
        }
        map.insert("PIF-two".into(), p);
        let mut pci = map["PCI"].clone();
        pci["uuid"] = json!("00000000-0000-0000-0000-000000000012");
        pci["host"] = json!("OpaqueRef:host-two");
        map.insert("PCI-two".into(), pci);
        map.get_mut("network-underlay").unwrap()["PIFs"] =
            json!(["OpaqueRef:PIF", "OpaqueRef:PIF-two"]);
        map.insert("_pool".into(), json!(true));
    }
    (m, r, w)
}
fn create_pool(c: &xenadmin_core::Connection, kind: Kind) -> NetworkFabricPool {
    NetworkFabricPool::create(
        c.latest().inventory().unwrap(),
        NetworkId::parse(NET).unwrap(),
        kind,
        vec![PifId::parse(PIF2).unwrap(), PifId::parse(PIF).unwrap()],
    )
    .unwrap()
}
fn submit_pool(
    c: &xenadmin_core::Connection,
    op: NetworkFabricPool,
) -> xenadmin_core::ResourceOperation {
    c.submit_resource(
        OperationId::new(),
        ResourceCommand::NetworkTopology {
            target: c.latest().observe_resource(target()).unwrap(),
            operation: NetworkTopologyOperation::Fabric(Box::new(op)),
        },
    )
    .unwrap()
}
#[tokio::test]
async fn creates_every_pool_host_in_coordinator_order_and_removes_exact_members() {
    for kind in [
        Kind::Tunnel(TunnelProtocol::Gre),
        Kind::Tunnel(TunnelProtocol::Vxlan),
        Kind::Sriov,
    ] {
        let (m, records, writes) = setup(kind).await;
        let dir = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&m, &b, false).await;
        let op = create_pool(&c, kind);
        assert_eq!(op.operations[0].review.host.to_string(), HOST);
        let r = finish(&submit_pool(&c, op)).await;
        assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
        assert_eq!(r.topology_changes.created_fabric.len(), 2);
        assert_eq!(writes.load(Ordering::SeqCst), 2);
        assert!(!serde_json::to_string(&*r).unwrap().contains("OpaqueRef:"));
        c.close().await;
        let c = connect(&m, &b, false).await;
        let removal = NetworkFabricPool::remove(
            c.latest().inventory().unwrap(),
            NetworkId::parse(NET).unwrap(),
            kind,
        )
        .unwrap();
        assert_eq!(
            removal.operations.last().unwrap().review.host.to_string(),
            HOST
        );
        let deleted = finish(&submit_pool(&c, removal)).await;
        assert_eq!(
            deleted.state,
            OperationState::Succeeded,
            "{}",
            deleted.detail
        );
        assert_eq!(deleted.topology_changes.removed_fabric.len(), 2);
        assert_eq!(writes.load(Ordering::SeqCst), 4);
        assert!(
            records.lock().unwrap()["network"]["PIFs"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
}
#[tokio::test]
async fn captured_hosts_input_controller_and_hardware_changes_refuse_before_first_write() {
    for kind in [Kind::Tunnel(TunnelProtocol::Gre), Kind::Sriov] {
        for case in 0..5 {
            let (m, records, writes) = setup(kind).await;
            let dir = tempfile::tempdir().unwrap();
            let b = Backend::with_state_directory(dir.path().into()).unwrap();
            let c = connect(&m, &b, false).await;
            let op = create_pool(&c, kind);
            {
                let mut map = records.lock().unwrap();
                match case {
                    0 => {
                        map.remove("host-two");
                    }
                    1 => map.get_mut("host_metrics-two").unwrap()["live"] = json!(false),
                    2 => map.get_mut("PIF-two").unwrap()["MAC"] = json!("02:00:00:00:00:ff"),
                    3 => map.get_mut("network").unwrap()["VIFs"] = json!(["OpaqueRef:unreviewed"]),
                    _ => {
                        if kind == Kind::Sriov {
                            map.get_mut("PCI-two").unwrap()["device_id"] = json!("ffff")
                        } else {
                            map.get_mut("SDN_controller").unwrap()["address"] = json!("192.0.2.99")
                        }
                    }
                }
            }
            let r = finish(&submit_pool(&c, op)).await;
            assert_ne!(
                r.state,
                OperationState::Succeeded,
                "case{case}:{}",
                r.detail
            );
            assert_eq!(writes.load(Ordering::SeqCst), 0, "case{case}:{}", r.detail);
        }
    }
}
#[tokio::test]
async fn lost_and_failed_task_stop_pool_suffix_and_recheck_never_replays() {
    for flag in ["_pool_drop", "_pool_fail", "_pool_change_after"] {
        let (m, records, writes) = setup(Kind::Tunnel(TunnelProtocol::Gre)).await;
        records.lock().unwrap().insert(flag.into(), json!(true));
        let dir = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&m, &b, false).await;
        let r = finish(&submit_pool(
            &c,
            create_pool(&c, Kind::Tunnel(TunnelProtocol::Gre)),
        ))
        .await;
        assert_eq!(
            r.state,
            OperationState::OutcomeUnknown,
            "{flag}:{}",
            r.detail
        );
        assert_eq!(writes.load(Ordering::SeqCst), 1);
        c.control_resource(r.id, OperationControl::Recheck).unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(writes.load(Ordering::SeqCst), 1);
    }
}
async fn changed(
    c: &xenadmin_core::Connection,
    old: &xenadmin_core::ResourceOperationRecord,
) -> Arc<xenadmin_core::ResourceOperationRecord> {
    tokio::time::timeout(deadline(5), async {
        loop {
            if let Some(r) = c.resource_history().iter().find(|r| {
                r.id == old.id
                    && r.revision() > old.revision()
                    && (r.state.terminal() || r.state == OperationState::OutcomeUnknown)
            }) {
                return Arc::new(r.clone());
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap()
}
#[tokio::test]
async fn saved_completion_rechecks_without_task_then_fresh_review_fills_only_missing_host() {
    let kind = Kind::Tunnel(TunnelProtocol::Vxlan);
    let (m, records, writes) = setup(kind).await;
    records
        .lock()
        .unwrap()
        .insert("_pool_deny_after".into(), json!(true));
    let dir = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&m, &b, false).await;
    let r = finish(&submit_pool(&c, create_pool(&c, kind))).await;
    assert_eq!(r.state, OperationState::OutcomeUnknown, "{}", r.detail);
    assert_eq!(r.topology_changes.created_fabric.len(), 1);
    {
        let mut m = records.lock().unwrap();
        m.remove("_pool_deny_after");
        m.remove("task-pool1");
    }
    c.control_resource(r.id, OperationControl::Recheck).unwrap();
    let partial = changed(&c, &r).await;
    assert_eq!(
        partial.state,
        OperationState::PartiallyCompleted,
        "{}",
        partial.detail
    );
    assert_eq!(writes.load(Ordering::SeqCst), 1);
    c.close().await;
    let c = connect(&m, &b, false).await;
    let next = NetworkFabricPool::create(
        c.latest().inventory().unwrap(),
        NetworkId::parse(NET).unwrap(),
        kind,
        vec![PifId::parse(PIF2).unwrap()],
    )
    .unwrap();
    let completed = finish(&submit_pool(&c, next)).await;
    assert_eq!(
        completed.state,
        OperationState::Succeeded,
        "{}",
        completed.detail
    );
    assert_eq!(completed.topology_changes.created_fabric.len(), 1);
    assert_eq!(writes.load(Ordering::SeqCst), 2);
}
#[tokio::test]
async fn late_completion_keeps_owned_ids_without_resuming_suffix_after_recovery_or_ack() {
    for ack in [false, true] {
        let kind = Kind::Tunnel(TunnelProtocol::Gre);
        let (m, records, writes) = setup(kind).await;
        records
            .lock()
            .unwrap()
            .insert("_pool_hold".into(), json!(true));
        let dir = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&m, &b, false).await;
        let op = submit_pool(&c, create_pool(&c, kind));
        tokio::time::timeout(deadline(5), async {
            while !records.lock().unwrap().contains_key("_pool_read") {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        let b2 = Backend::with_state_directory(dir.path().into()).unwrap();
        let c2 = connect(&m, &b2, false).await;
        let before = op.latest();
        let _ = before;
        let unknown = tokio::time::timeout(deadline(5), async {
            loop {
                if let Some(r) = c2.resource_history().iter().find(|r| {
                    r.id == op.latest().id
                        && r.state == OperationState::OutcomeUnknown
                        && r.steps[0].state == xenadmin_core::StepState::OutcomeUnknown
                }) {
                    return r.clone();
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        if ack {
            acknowledge_unknown(&c2, unknown.id, 5).await;
        }
        c2.close().await;
        records.lock().unwrap().remove("_pool_hold");
        tokio::time::timeout(deadline(5), async {
            while op.latest().topology_changes.created_fabric.is_empty() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        let db = rusqlite::Connection::open(dir.path().join("operations.sqlite3")).unwrap();
        let payload: String = db
            .query_row(
                "SELECT payload FROM resource_operations WHERE id=?1",
                [unknown.id.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        let row: Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(
            op.latest().state,
            if ack {
                OperationState::AcknowledgedUnknown
            } else {
                OperationState::OutcomeUnknown
            },
            "watch {:?}, durable state {:?}, interrupted {:?}, writes {}",
            op.latest().detail,
            row["state"],
            row["interrupted"],
            writes.load(Ordering::SeqCst)
        );
        assert_eq!(writes.load(Ordering::SeqCst), 1);
        c.close().await;
        c2.close().await;
    }
}
#[tokio::test]
async fn readonly_and_missing_host_selection_cannot_submit() {
    let kind = Kind::Sriov;
    let (m, _, writes) = setup(kind).await;
    let dir = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&m, &b, true).await;
    assert!(
        NetworkFabricPool::create(
            c.latest().inventory().unwrap(),
            NetworkId::parse(NET).unwrap(),
            kind,
            vec![PifId::parse(PIF).unwrap()]
        )
        .is_err()
    );
    assert!(
        c.submit_resource(
            OperationId::new(),
            ResourceCommand::NetworkTopology {
                target: c.latest().observe_resource(target()).unwrap(),
                operation: NetworkTopologyOperation::Fabric(Box::new(create_pool(&c, kind)))
            }
        )
        .is_err()
    );
    assert_eq!(writes.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn corrupt_pool_receipts_prefix_and_public_success_are_refused_on_restart() {
    for case in 0..6 {
        let kind = Kind::Tunnel(TunnelProtocol::Gre);
        let (m, _, writes) = setup(kind).await;
        let dir = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&m, &b, false).await;
        let r = finish(&submit_pool(&c, create_pool(&c, kind))).await;
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
        match case {
            0 => v["network_topology_plan"]["steps"][0]["completed"] = json!(false),
            1 => v["network_topology_plan"]["fabric_before"] = Value::Null,
            2 => v["network_topology_plan"]["steps"][0]["fabric_member"]["input"] = json!(PIF2),
            3 => {
                v["network_topology_plan"]["steps"][0]["verified"] = json!(false);
                v["steps"][0]["state"] = json!("Tracking");
            }
            4 => v["steps"][0]["label"] = json!("Unreviewed"),
            _ => v["network_topology_plan"]["pifs_before"] = json!([]),
        }
        db.execute(
            "UPDATE resource_operations SET payload=?1 WHERE id=?2",
            [v.to_string(), r.id.to_string()],
        )
        .unwrap();
        drop(db);
        let b = Backend::with_state_directory(dir.path().into()).unwrap();
        let mut options = ConnectionOptions::new(m.endpoint.clone());
        options.read_only = false;
        let c = b.connect(
            options,
            Credentials::new("root".into(), "fixture-password".into()),
        );
        c.wait_ready(deadline(5)).await.unwrap();
        assert!(c.latest().error().is_some(), "case{case}");
        assert!(
            !c.resource_history().iter().any(|row| row.id == r.id),
            "case{case}"
        );
        assert_eq!(writes.load(Ordering::SeqCst), 2);
        c.close().await;
    }
}
#[tokio::test]
async fn cancellation_keeps_uncertain_pool_reserved_and_does_not_start_remaining_hosts() {
    let kind = Kind::Tunnel(TunnelProtocol::Gre);
    let (m, records, writes) = setup(kind).await;
    records
        .lock()
        .unwrap()
        .insert("_pool_pending".into(), json!(true));
    let dir = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&m, &b, false).await;
    let command = create_pool(&c, kind);
    let op = submit_pool(&c, command.clone());
    tokio::time::timeout(deadline(5), async {
        while !op.latest().can_cancel || op.latest().state != OperationState::Tracking {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    c.control_resource(op.latest().id, OperationControl::RequestCancellation)
        .unwrap();
    tokio::time::timeout(deadline(5), async {
        while records.lock().unwrap().get("_pool_cancel") != Some(&json!(1)) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    records.lock().unwrap().get_mut("task-pool1").unwrap()["status"] = json!("cancelled");
    let r = finish(&op).await;
    assert_eq!(r.state, OperationState::OutcomeUnknown, "{}", r.detail);
    let rejected = finish(&submit_pool(&c, command)).await;
    assert_eq!(rejected.state, OperationState::Rejected);
    c.control_resource(r.id, OperationControl::Recheck).unwrap();
    let checked = changed(&c, &r).await;
    assert_eq!(
        checked.state,
        OperationState::OutcomeUnknown,
        "{}",
        checked.detail
    );
    assert_eq!(records.lock().unwrap()["_pool_cancel"], json!(1));
    assert_eq!(writes.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn all_verified_preparing_restart_still_requires_fresh_final_graph() {
    for change in [false, true] {
        let kind = Kind::Tunnel(TunnelProtocol::Gre);
        let (m, records, writes) = setup(kind).await;
        let dir = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&m, &b, false).await;
        let r = finish(&submit_pool(&c, create_pool(&c, kind))).await;
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
        let mut payload: Value = serde_json::from_str(&raw).unwrap();
        payload["state"] = json!("Preparing");
        db.execute(
            "UPDATE resource_operations SET payload=?1,terminal=0,revision=revision+1 WHERE id=?2",
            [payload.to_string(), r.id.to_string()],
        )
        .unwrap();
        drop(db);
        if change {
            records.lock().unwrap().get_mut("PIF-pool1").unwrap()["management"] = json!(true);
        }
        let b = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&m, &b, false).await;
        let row = tokio::time::timeout(deadline(5), async {
            loop {
                if let Some(row) = c.resource_history().iter().find(|row| {
                    row.id == r.id
                        && row.revision() > r.revision() + 2
                        && (row.state.terminal() || row.state == OperationState::OutcomeUnknown)
                }) {
                    return row.clone();
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            row.state,
            if change {
                OperationState::OutcomeUnknown
            } else {
                OperationState::Succeeded
            },
            "{}",
            row.detail
        );
        assert_eq!(writes.load(Ordering::SeqCst), 2);
        c.close().await;
    }
}

#[tokio::test]
async fn later_host_cannot_reuse_an_earlier_task_or_created_pair() {
    for flag in ["_pool_reuse_result", "_pool_reuse_task"] {
        let kind = Kind::Sriov;
        let (m, records, writes) = setup(kind).await;
        records.lock().unwrap().insert(flag.into(), json!(true));
        let dir = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&m, &b, false).await;
        let r = finish(&submit_pool(&c, create_pool(&c, kind))).await;
        assert_eq!(
            r.state,
            OperationState::OutcomeUnknown,
            "{flag}:{}",
            r.detail
        );
        assert_eq!(r.topology_changes.created_fabric.len(), 1);
        assert_eq!(writes.load(Ordering::SeqCst), 2);
        c.control_resource(r.id, OperationControl::Recheck).unwrap();
        let reread = changed(&c, &r).await;
        assert_eq!(
            reread.state,
            OperationState::OutcomeUnknown,
            "{}",
            reread.detail
        );
        assert_eq!(writes.load(Ordering::SeqCst), 2);
    }
}
