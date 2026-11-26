use super::*;

pub(super) fn after_write(s: &mut Control) {
    if s.records.contains_key("_reboot_after_write")
        && let Some(o) = s.records.get_mut(&reference("network_sriov", 92))
    {
        o["requires_reboot"] = json!(true);
    }
}

fn install(s: &mut Control) {
    s.records.get_mut(&reference("pool", 1)).unwrap()["ha_enabled"] = json!(false);
    s.records.get_mut(&reference("pool", 1)).unwrap()["restrictions"] =
        json!({"restrict_network_sriov": "false"});
    let mut network = s.records[&reference("network", 6)].clone();
    network["uuid"] = json!(id(7));
    network["PIFs"] = json!([reference("PIF", 62), reference("PIF", 63)]);
    s.records.insert(reference("network", 7), network);
    for n in [5, 6, 7] {
        let r = s.records.get_mut(&reference("network", n)).unwrap();
        r["purpose"] = json!([]);
        r["default_locking_mode"] = json!("unlocked");
    }
    for h in [2, 3] {
        let host = s.records.get_mut(&reference("host", h)).unwrap();
        host["enabled"] = json!(false);
        host["current_operations"] = json!({});
        let logical = s.records.get_mut(&reference("PIF", h + 10)).unwrap();
        logical["physical"] = json!(false);
        logical["PCI"] = json!("OpaqueRef:NULL");
        logical["sriov_logical_PIF_of"] = json!([reference("network_sriov", h + 90)]);
        logical["sriov_physical_PIF_of"] = json!([]);
        let mut physical = logical.clone();
        physical["uuid"] = json!(id(h + 60));
        physical["network"] = json!(reference("network", 7));
        physical["physical"] = json!(true);
        physical["PCI"] = json!(reference("PCI", h + 80));
        physical["VLAN_slave_of"] = json!([]);
        physical["sriov_logical_PIF_of"] = json!([]);
        physical["sriov_physical_PIF_of"] = json!([reference("network_sriov", h + 90)]);
        s.records.insert(reference("PIF", h + 60), physical);
        s.records.insert(
            reference("PCI", h + 80),
            json!({
                "uuid": id(h + 80),
                "host": reference("host", h),
                "pci_id": "0000:01:00.1",
                "driver_name": "fixture_nic",
                "vendor_id": "8086",
                "device_id": "100e",
            }),
        );
        s.records.insert(
            reference("network_sriov", h + 90),
            json!({
                "uuid": id(h + 90),
                "physical_PIF": reference("PIF", h + 60),
                "logical_PIF": reference("PIF", h + 10),
                "requires_reboot": false,
                "configuration_mode": "sysfs",
            }),
        );
    }
}

fn reviewed(c: &xenadmin_core::Connection, retag: bool) -> NetworkTopologyOperation {
    let snap = c.latest();
    let i = snap.inventory().unwrap();
    let mut op = operation(c, retag);
    match &mut op {
        NetworkTopologyOperation::CreateVlans { uplinks, .. } => {
            for p in uplinks {
                *p = VlanUplink::capture(i, p.pif, id(5).into()).unwrap();
            }
        }
        NetworkTopologyOperation::RetagVlans { interfaces, .. } => {
            for v in interfaces {
                v.uplink = VlanUplink::capture(i, v.uplink.pif, id(5).into()).unwrap();
            }
        }
        _ => unreachable!(),
    }
    op
}

#[tokio::test]
async fn logical_sriov_vlan_create_retag_and_remove_preserve_exact_parents() {
    for mode in [0, 1, 2] {
        let (mock, s) = fixture(mode != 0, Fault::None).await;
        install(&mut s.lock().unwrap());
        let dir = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&mock, &b).await;
        let mut op = reviewed(&c, mode != 0);
        if mode == 2 {
            let NetworkTopologyOperation::RetagVlans { interfaces, .. } = op else {
                unreachable!()
            };
            op = NetworkTopologyOperation::DeleteVlans { interfaces };
        }
        let result = finish(&submit(&c, op)).await;
        assert_eq!(result.state, OperationState::Succeeded, "{}", result.detail);
        let rows = &s.lock().unwrap().records;
        for h in [2, 3] {
            assert_eq!(rows[&reference("PIF", h + 10)]["currently_attached"], true);
            assert_eq!(rows[&reference("PIF", h + 60)]["currently_attached"], true);
            assert!(rows.contains_key(&reference("network_sriov", h + 90)));
        }
        assert_eq!(
            result.topology_changes.created_vlans.len(),
            if mode == 2 { 0 } else { 2 }
        );
        assert_eq!(
            result.topology_changes.removed_vlans.len(),
            if mode == 0 { 0 } else { 2 }
        );
        assert_eq!(count(&mock, "Async.network_sriov.create"), 0);
        assert_eq!(count(&mock, "Async.network_sriov.destroy"), 0);
    }
}

/// What a host reports and the records above leave out: every interface links a metrics
/// record, a logical or VLAN interface sharing its parent's, and XAPI gives each physical
/// interface the receive-offload property.
fn as_a_host_reports(s: &mut Control) {
    for h in [2u128, 3] {
        s.records.insert(
            reference("PIF_metrics", h + 70),
            json!({
                "uuid": id(h + 70),
                "carrier": true,
                "speed": "10000",
                "duplex": true,
                "vendor_id": "8086",
                "vendor_name": "Fixture",
                "device_id": "100e",
                "device_name": "Fixture NIC",
                "pci_bus_path": "0000:01:00.1",
            }),
        );
        let host = reference("host", h);
        for (r, p) in s.records.iter_mut() {
            if r.starts_with("OpaqueRef:PIF:") && p["host"] == host.as_str() {
                p["metrics"] = json!(reference("PIF_metrics", h + 70));
                if p["physical"] == true {
                    p["properties"] = json!({"gro": "on"});
                }
            }
        }
    }
}

#[tokio::test]
async fn logical_sriov_vlan_accepts_records_as_a_host_reports_them() {
    for mode in [0, 1, 2] {
        let (mock, s) = fixture(mode != 0, Fault::None).await;
        {
            let mut s = s.lock().unwrap();
            install(&mut s);
            as_a_host_reports(&mut s);
        }
        let dir = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&mock, &b).await;
        // The inventory the review is captured from knows the metrics records. The
        // records read to check the review before each write do not include them.
        let snapshot = c.latest();
        let linked = snapshot
            .inventory()
            .unwrap()
            .details
            .pifs
            .values()
            .filter(|p| p.metrics.is_some())
            .count();
        assert_eq!(linked, if mode == 0 { 4 } else { 6 });
        let mut op = reviewed(&c, mode != 0);
        if mode == 2 {
            let NetworkTopologyOperation::RetagVlans { interfaces, .. } = op else {
                unreachable!()
            };
            op = NetworkTopologyOperation::DeleteVlans { interfaces };
        }
        let result = finish(&submit(&c, op)).await;
        assert_eq!(
            result.state,
            OperationState::Succeeded,
            "mode {mode}: {}",
            result.detail
        );
        assert_eq!(
            result.topology_changes.created_vlans.len(),
            if mode == 2 { 0 } else { 2 }
        );
        assert_eq!(
            result.topology_changes.removed_vlans.len(),
            if mode == 0 { 0 } else { 2 }
        );
        assert_eq!(count(&mock, "Async.network_sriov.create"), 0);
        assert_eq!(count(&mock, "Async.network_sriov.destroy"), 0);
    }
}

#[tokio::test]
async fn logical_sriov_vlan_refuses_a_same_driver_interface_somebody_configured() {
    for (field, value) in [
        ("properties", json!({"gro": "on", "tso": "off"})),
        ("properties", json!({"gro": "auto"})),
        ("other_config", json!({"ethtool-speed": "1000"})),
    ] {
        let (mock, s) = fixture(false, Fault::None).await;
        {
            let mut s = s.lock().unwrap();
            install(&mut s);
            as_a_host_reports(&mut s);
            s.records.get_mut(&reference("PIF", 62)).unwrap()[field] = value.clone();
        }
        let dir = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&mock, &b).await;
        let r = finish(&submit(&c, reviewed(&c, false))).await;
        assert_ne!(r.state, OperationState::Succeeded, "{field} {value}");
        assert_eq!(s.lock().unwrap().writes, 0, "{field} {value}");
        assert!(
            r.detail.contains("unreviewed configuration"),
            "{field} {value}: {}",
            r.detail
        );
    }
}

#[tokio::test]
async fn logical_sriov_vlan_rejects_changed_readiness_hardware_and_driver_dependencies_before_write()
 {
    for change in [
        "reboot", "attached", "pci", "driver", "enabled", "vm", "service", "legacy",
    ] {
        let (mock, s) = fixture(false, Fault::None).await;
        install(&mut s.lock().unwrap());
        let dir = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&mock, &b).await;
        let mut op = reviewed(&c, false);
        {
            let mut s = s.lock().unwrap();
            match change {
                "reboot" => {
                    s.records.get_mut(&reference("network_sriov", 92)).unwrap()["requires_reboot"] =
                        json!(true)
                }
                "attached" => {
                    s.records.get_mut(&reference("PIF", 12)).unwrap()["currently_attached"] =
                        json!(false)
                }
                "pci" => {
                    s.records.get_mut(&reference("PCI", 82)).unwrap()["device_id"] = json!("ffff")
                }
                "driver" => {
                    s.records.get_mut(&reference("PCI", 82)).unwrap()["driver_name"] =
                        json!("other")
                }
                "enabled" => {
                    s.records.get_mut(&reference("host", 2)).unwrap()["enabled"] = json!(true)
                }
                "vm" => {
                    s.records.insert(
                        reference("VM", 500),
                        json!({
                            "uuid": id(500),
                            "resident_on": reference("host", 2),
                            "power_state": "Running",
                            "is_control_domain": false,
                        }),
                    );
                }
                "service" => {
                    s.records.get_mut(&reference("network", 7)).unwrap()["purpose"] = json!(["nbd"])
                }
                "legacy" => {
                    if let NetworkTopologyOperation::CreateVlans { uplinks, .. } = &mut op {
                        for p in uplinks {
                            p.sriov = None;
                        }
                    }
                }
                _ => unreachable!(),
            }
        }
        let r = finish(&submit(&c, op)).await;
        assert_ne!(r.state, OperationState::Succeeded, "{change}");
        assert_eq!(s.lock().unwrap().writes, 0, "{change}: {}", r.detail);
    }
}

#[tokio::test]
async fn logical_sriov_vlan_lost_reply_is_not_replayed_and_new_ids_survive_parent_reboot_change() {
    for fault in [Fault::DropReply, Fault::None] {
        let (mock, s) = fixture(false, fault).await;
        install(&mut s.lock().unwrap());
        let dir = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&mock, &b).await;
        let op = reviewed(&c, false);
        if fault == Fault::None {
            s.lock()
                .unwrap()
                .records
                .insert("_reboot_after_write".into(), json!(true));
        }
        let pending = submit(&c, op);
        let r = finish(&pending).await;
        assert_ne!(r.state, OperationState::Succeeded);
        assert_eq!(s.lock().unwrap().writes, 1);
        if fault == Fault::None {
            assert_eq!(r.topology_changes.created_vlans.len(), 1, "{}", r.detail);
        }
        c.control_resource(r.id, OperationControl::Recheck).unwrap();
        tokio::time::timeout(deadline(5), async {
            loop {
                if c.resource_history().iter().any(|n| {
                    n.id == r.id
                        && n.revision() > r.revision()
                        && (n.state == OperationState::OutcomeUnknown || n.state.terminal())
                }) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(s.lock().unwrap().writes, 1);
    }
}

#[tokio::test]
async fn logical_sriov_vlan_restart_uses_saved_completion_without_replaying_remaining_hosts() {
    let (mock, s) = fixture(false, Fault::None).await;
    install(&mut s.lock().unwrap());
    let dir = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&mock, &b).await;
    let op = reviewed(&c, false);
    s.lock()
        .unwrap()
        .records
        .insert("_reboot_after_write".into(), json!(true));
    let r = finish(&submit(&c, op)).await;
    assert_eq!(r.state, OperationState::OutcomeUnknown);
    assert_eq!(r.topology_changes.created_vlans.len(), 1);
    c.close().await;
    {
        let mut s = s.lock().unwrap();
        s.records.remove("_reboot_after_write");
        s.records.get_mut(&reference("network_sriov", 92)).unwrap()["requires_reboot"] =
            json!(false);
        s.records.retain(|k, _| !k.starts_with("OpaqueRef:task:"));
    }
    let recovered = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&mock, &recovered).await;
    c.control_resource(r.id, OperationControl::Recheck).unwrap();
    let row = tokio::time::timeout(deadline(5), async {
        loop {
            if let Some(row) = c
                .resource_history()
                .iter()
                .find(|row| row.id == r.id && row.state.terminal())
            {
                break row.clone();
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        row.state,
        OperationState::PartiallyCompleted,
        "{}",
        row.detail
    );
    assert_eq!(s.lock().unwrap().writes, 1);
    assert_eq!(row.topology_changes.created_vlans.len(), 1);
}

fn retained(s: &mut Control) {
    let mut n = s.records[&reference("network", 5)].clone();
    n["uuid"] = json!(id(8));
    n["PIFs"] = json!([reference("PIF", 302), reference("PIF", 303)]);
    s.records.insert(reference("network", 8), n);
    for h in [2, 3] {
        let mut p = s.records[&reference("PIF", h + 10)].clone();
        p["uuid"] = json!(id(h + 300));
        p["network"] = json!(reference("network", 8));
        p["VLAN"] = json!("200");
        p["VLAN_master_of"] = json!(reference("VLAN", h + 400));
        p["VLAN_slave_of"] = json!([]);
        p["sriov_logical_PIF_of"] = json!([]);
        s.records.insert(reference("PIF", h + 300), p);
        s.records.insert(
            reference("VLAN", h + 400),
            json!({
                "uuid": id(h + 400),
                "tagged_PIF": reference("PIF", h + 10),
                "untagged_PIF": reference("PIF", h + 300),
                "tag": "200",
                "other_config": {},
            }),
        );
        s.records.get_mut(&reference("PIF", h + 10)).unwrap()["VLAN_slave_of"]
            .as_array_mut()
            .unwrap()
            .push(json!(reference("VLAN", h + 400)));
    }
}

#[tokio::test]
async fn logical_sriov_vlan_retains_other_vlan_configuration_and_refuses_review_changes() {
    for changed in [false, true] {
        let (mock, s) = fixture(false, Fault::None).await;
        {
            let mut s = s.lock().unwrap();
            install(&mut s);
            retained(&mut s);
        }
        let dir = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&mock, &b).await;
        let op = reviewed(&c, false);
        let original = s.lock().unwrap().records[&reference("PIF", 302)].clone();
        if changed {
            s.lock()
                .unwrap()
                .records
                .get_mut(&reference("PIF", 302))
                .unwrap()["currently_attached"] = json!(false);
        }
        let r = finish(&submit(&c, op)).await;
        if changed {
            assert_ne!(r.state, OperationState::Succeeded);
            assert_eq!(s.lock().unwrap().writes, 0);
        } else {
            assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
            assert_eq!(s.lock().unwrap().records[&reference("PIF", 302)], original);
        }
    }
}

#[tokio::test]
async fn unreviewed_single_vlan_and_generic_attachment_cannot_activate_sriov() {
    for action in [0, 1, 2] {
        let (mock, s) = fixture(false, Fault::None).await;
        install(&mut s.lock().unwrap());
        let dir = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&mock, &b).await;
        let snapshot = c.latest();
        let p = snapshot.inventory().unwrap().details.pifs
            [&PifId::from(id(if action == 2 { 62 } else { 12 }))]
            .clone();
        let expected = PifIdentity::from_inventory(&p).unwrap();
        let (target, operation) = if action == 0 {
            (
                ResourceTarget::Network(id(5).into()),
                xenadmin_core::NetworkOperation::CreateVlan {
                    parent: p.id,
                    expected,
                    tag: 100,
                },
            )
        } else {
            (
                ResourceTarget::Pif(p.id),
                xenadmin_core::NetworkOperation::SetAttached {
                    expected,
                    expected_attached: true,
                    attached: false,
                },
            )
        };
        let pending = c
            .submit_resource(
                OperationId::new(),
                ResourceCommand::Network {
                    target: snapshot.observe_resource(target).unwrap(),
                    operation,
                },
            )
            .unwrap();
        let r = finish(&pending).await;
        assert_ne!(r.state, OperationState::Succeeded);
        assert_eq!(s.lock().unwrap().writes, 0);
        assert!(r.detail.contains("SR-IOV"), "{}", r.detail);
    }
}

#[tokio::test]
async fn logical_sriov_vlan_checks_cluster_dependencies_and_only_tolerates_absent_api() {
    for mode in [
        "physical",
        "logical",
        "MESSAGE_METHOD_UNKNOWN",
        "NOT_IMPLEMENTED",
        "RBAC_PERMISSION_DENIED",
    ] {
        let (mock, s) = fixture(false, Fault::None).await;
        install(&mut s.lock().unwrap());
        let dir = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&mock, &b).await;
        let op = reviewed(&c, false);
        {
            let mut s = s.lock().unwrap();
            if matches!(mode, "physical" | "logical") {
                s.records.insert(
                    reference("Cluster_host", 500),
                    json!({
                        "uuid": id(500),
                        "host": reference("host", 2),
                        "PIF": reference("PIF", if mode == "physical" { 62 } else { 12 }),
                    }),
                );
            } else {
                s.records.insert("_cluster_error".into(), json!(mode));
            }
        }
        let r = finish(&submit(&c, op)).await;
        let absent = matches!(mode, "MESSAGE_METHOD_UNKNOWN" | "NOT_IMPLEMENTED");
        assert_eq!(
            r.state == OperationState::Succeeded,
            absent,
            "{mode}: {}",
            r.detail
        );
        assert_eq!(
            s.lock().unwrap().writes,
            if absent { 2 } else { 0 },
            "{mode}"
        );
    }
}
