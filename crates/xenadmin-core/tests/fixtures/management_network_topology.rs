use super::*;

fn ordinary(c: &mut Control, n: u128, template: u128) {
    let mut p = c.rows[&reference("PIF", template)].clone();
    p["uuid"] = json!(id(n));
    p["network"] = json!(reference("network", n));
    p["device"] = json!(format!("eth{n}"));
    p["MAC"] = json!(format!("02:00:00:00:00:{n:02x}"));
    p["management"] = json!(false);
    p["physical"] = json!(true);
    p["VLAN"] = json!(-1);
    p["ip_configuration_mode"] = json!("None");
    p["IP"] = json!("");
    p["netmask"] = json!("");
    p["gateway"] = json!("");
    p["DNS"] = json!("");
    p["ipv6_configuration_mode"] = json!("None");
    p["IPv6"] = json!([]);
    p["ipv6_gateway"] = json!("");
    p["bond_master_of"] = json!([]);
    p["bond_slave_of"] = json!("OpaqueRef:NULL");
    p["VLAN_slave_of"] = json!([]);
    p["VLAN_master_of"] = json!("OpaqueRef:NULL");
    c.rows.insert(reference("PIF", n), p);
    c.rows.insert(reference("network",n),json!({"uuid":id(n+100),"name_label":format!("Network {n}"),"managed":true,"MTU":1500,"PIFs":[reference("PIF",n)],"VIFs":[]}));
}
fn bond(c: &mut Control, master: u128) {
    for member in [16, 17] {
        ordinary(c, member, master);
        c.rows.get_mut(&reference("PIF", member)).unwrap()["bond_slave_of"] =
            json!(reference("Bond", 60));
    }
    let p = c.rows.get_mut(&reference("PIF", master)).unwrap();
    p["physical"] = json!(false);
    p["device"] = json!("bond0");
    p["bond_master_of"] = json!([reference("Bond", 60)]);
    c.rows.insert(reference("Bond",60),json!({"uuid":id(60),"master":reference("PIF",master),"slaves":[reference("PIF",16),reference("PIF",17)],"primary_slave":reference("PIF",16),"mode":"active-backup","properties":{},"links_up":2}));
}
fn vlan(c: &mut Control, parent: u128, child: u128, n: u128, tag: u16) {
    let p = c.rows.get_mut(&reference("PIF", child)).unwrap();
    p["physical"] = json!(false);
    p["VLAN"] = json!(tag);
    p["VLAN_master_of"] = json!(reference("VLAN", n));
    c.rows.get_mut(&reference("PIF", parent)).unwrap()["VLAN_slave_of"]
        .as_array_mut()
        .unwrap()
        .push(json!(reference("VLAN", n)));
    c.rows.insert(reference("VLAN",n),json!({"uuid":id(n),"tagged_PIF":reference("PIF",parent),"untagged_PIF":reference("PIF",child),"tag":tag}));
}
fn topology(c: &mut Control, root: u128, kind: u8) {
    match kind {
        0 => {
            ordinary(c, 16, root);
            vlan(c, 16, root, 61, 17);
        }
        1 => bond(c, root),
        _ => {
            ordinary(c, 18, root);
            bond(c, 18);
            vlan(c, 18, root, 61, 17);
        }
    }
}

#[tokio::test]
async fn malformed_reviewed_graph_cannot_be_submitted_as_management_intent() {
    let (_mock, bridge, control) = fixture(false, Fault::None).await;
    topology(&mut control.lock().unwrap(), 13, 2);
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&bridge, &backend).await;
    for malformed in ["extra", "root", "cross_host"] {
        let mut op = operation(&c, 1);
        let ManagementNetworkOperation::ConfigureIpv4 { interface, .. } = &mut op else {
            panic!()
        };
        let t = interface.topology.as_mut().unwrap();
        match malformed {
            "extra" => {
                let mut extra = t.pifs[&id(16).into()].clone();
                extra.id = id(99).into();
                t.pifs.insert(extra.id, extra);
            }
            "root" => t.pifs.get_mut(&interface.pif).unwrap().mac = "02:00:00:00:ff:ff".into(),
            "cross_host" => t.pifs.get_mut(&id(16).into()).unwrap().host = Some(id(99).into()),
            _ => unreachable!(),
        }
        let result = c.submit_resource(
            OperationId::new(),
            ResourceCommand::ManagementNetwork {
                host: c
                    .latest()
                    .observe_resource(ResourceTarget::Host(id(2).into()))
                    .unwrap(),
                operation: Box::new(op),
            },
        );
        assert!(
            matches!(result, Err(xenadmin_core::Error::InvalidResourceCommand)),
            "{malformed}"
        );
    }
    assert_eq!(control.lock().unwrap().writes, 0);
    c.close().await;
}
#[tokio::test]
async fn existing_physical_vlan_bond_and_vlan_over_bond_management_moves_verify_exact_graphs() {
    for kind in [0, 1, 2] {
        for root in [13, 14] {
            let (_mock, bridge, control) = fixture(false, Fault::None).await;
            topology(&mut control.lock().unwrap(), root, kind);
            let dir = tempfile::tempdir().unwrap();
            let backend = Backend::with_state_directory(dir.path().into()).unwrap();
            let c = connect(&bridge, &backend).await;
            let op = operation(&c, 0);
            assert!(
                op.availability(c.latest().inventory().unwrap())
                    .is_available()
            );
            let result = finish(&submit(&c, op)).await;
            assert_eq!(
                result.state,
                OperationState::Succeeded,
                "kind {kind},root {root}: {}",
                result.detail
            );
            assert!(
                result
                    .management_network
                    .as_ref()
                    .unwrap()
                    .recovery_endpoint_verified
            );
            assert_eq!(control.lock().unwrap().writes, 1);
            c.close().await;
        }
    }
}
#[tokio::test]
async fn management_configuration_on_existing_topology_keeps_ancestor_configuration_and_ids() {
    for kind in [0, 1, 2] {
        for operation_kind in [1, 2, 3, 5] {
            let (_mock, bridge, control) =
                fixture([2, 5].contains(&operation_kind), Fault::None).await;
            topology(&mut control.lock().unwrap(), 13, kind);
            let before = control.lock().unwrap().rows.clone();
            let dir = tempfile::tempdir().unwrap();
            let backend = Backend::with_state_directory(dir.path().into()).unwrap();
            let c = connect(&bridge, &backend).await;
            let result = finish(&submit(&c, operation(&c, operation_kind))).await;
            assert_eq!(
                result.state,
                OperationState::Succeeded,
                "kind {kind},operation {operation_kind}: {}",
                result.detail
            );
            for (r, v) in &before {
                if r.starts_with("OpaqueRef:PIF:") && r != &reference("PIF", 13) {
                    assert_eq!(&control.lock().unwrap().rows[r], v, "ancestor changed");
                }
            }
            assert_eq!(control.lock().unwrap().writes, 1);
            c.close().await;
        }
    }
}
#[tokio::test]
async fn management_can_move_between_two_vlans_on_the_same_reviewed_bond() {
    let (_mock, bridge, control) = fixture(false, Fault::None).await;
    {
        let mut v = control.lock().unwrap();
        topology(&mut v, 13, 2);
        vlan(&mut v, 18, 14, 62, 18);
    }
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&bridge, &backend).await;
    let result = finish(&submit(&c, operation(&c, 0))).await;
    assert_eq!(result.state, OperationState::Succeeded, "{}", result.detail);
    assert_eq!(control.lock().unwrap().writes, 1);
    c.close().await;
}
#[tokio::test]
async fn changed_topology_identity_relationships_protection_and_secondary_purposes_refuse_dispatch()
{
    for change in [
        "primary",
        "tag",
        "mac",
        "purpose",
        "protected",
        "member",
        "new_vlan",
        "tunnel",
        "sriov",
        "reverse",
        "network_mtu",
    ] {
        let (_mock, bridge, control) = fixture(false, Fault::None).await;
        topology(&mut control.lock().unwrap(), 13, 2);
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&bridge, &backend).await;
        let op = operation(&c, 1);
        {
            let mut v = control.lock().unwrap();
            match change {
                "primary" => {
                    v.rows.get_mut(&reference("Bond", 60)).unwrap()["primary_slave"] =
                        json!(reference("PIF", 17))
                }
                "tag" => {
                    v.rows.get_mut(&reference("VLAN", 61)).unwrap()["tag"] = json!(18);
                    v.rows.get_mut(&reference("PIF", 13)).unwrap()["VLAN"] = json!(18);
                }
                "mac" => {
                    v.rows.get_mut(&reference("PIF", 16)).unwrap()["MAC"] =
                        json!("02:00:00:ff:ff:ff")
                }
                "purpose" => {
                    v.rows.get_mut(&reference("PIF", 16)).unwrap()["other_config"] =
                        json!({"management_purpose":"Storage"})
                }
                "protected" => {
                    v.rows.get_mut(&reference("PIF", 16)).unwrap()["disallow_unplug"] = json!(true)
                }
                "member" => {
                    ordinary(&mut v, 19, 16);
                    v.rows.get_mut(&reference("PIF", 16)).unwrap()["bond_slave_of"] =
                        json!("OpaqueRef:NULL");
                    v.rows.get_mut(&reference("PIF", 19)).unwrap()["bond_slave_of"] =
                        json!(reference("Bond", 60));
                    let b = v.rows.get_mut(&reference("Bond", 60)).unwrap();
                    b["slaves"] = json!([reference("PIF", 19), reference("PIF", 17)]);
                    b["primary_slave"] = json!(reference("PIF", 19));
                }
                "new_vlan" => {
                    ordinary(&mut v, 19, 16);
                    vlan(&mut v, 18, 19, 62, 18);
                }
                "tunnel" => {
                    v.rows.get_mut(&reference("PIF", 16)).unwrap()["tunnel_transport_PIF_of"] =
                        json!(["OpaqueRef:Tunnel:90"])
                }
                "sriov" => {
                    v.rows.get_mut(&reference("PIF", 16)).unwrap()["sriov_physical_PIF_of"] =
                        json!(["OpaqueRef:network_sriov:90"]);
                }
                "reverse" => {
                    v.rows.get_mut(&reference("PIF", 16)).unwrap()["bond_slave_of"] =
                        json!("OpaqueRef:NULL")
                }
                "network_mtu" => {
                    v.rows.get_mut(&reference("network", 18)).unwrap()["MTU"] = json!(9000)
                }
                _ => unreachable!(),
            }
        }
        let result = finish(&submit(&c, op)).await;
        assert_eq!(
            result.state,
            OperationState::Rejected,
            "{change}: {}",
            result.detail
        );
        assert_eq!(control.lock().unwrap().writes, 0);
        c.close().await;
    }
}
#[tokio::test]
async fn changed_graph_after_completion_is_unknown_and_recheck_does_not_reconfigure() {
    let (_mock, bridge, control) = fixture(false, Fault::TopologyChangedAfterWrite).await;
    topology(&mut control.lock().unwrap(), 13, 2);
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&bridge, &backend).await;
    let result = finish(&submit(&c, operation(&c, 1))).await;
    assert_eq!(
        result.state,
        OperationState::OutcomeUnknown,
        "{}",
        result.detail
    );
    c.control_resource(result.id, OperationControl::Recheck)
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !c.resource_history().iter().any(|r| {
            r.id == result.id
                && r.revision() > result.revision()
                && r.state == OperationState::OutcomeUnknown
        }) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(control.lock().unwrap().writes, 1);
    c.close().await;
}
#[tokio::test]
async fn completed_topology_handover_recovers_after_restart_without_task_or_write_replay() {
    let (_mock, bridge, control) = fixture(false, Fault::TopologyReadAfterCompletion).await;
    topology(&mut control.lock().unwrap(), 13, 2);
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&bridge, &backend).await;
    let result = finish(&submit(&c, operation(&c, 1))).await;
    assert_eq!(
        result.state,
        OperationState::OutcomeUnknown,
        "{}",
        result.detail
    );
    c.close().await;
    drop(backend);
    {
        let mut v = control.lock().unwrap();
        v.fault = Fault::None;
        v.rows.remove(&reference("task", 51));
    }
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&bridge, &backend).await;
    tokio::time::timeout(Duration::from_secs(5), async {
        while !c
            .resource_history()
            .iter()
            .any(|r| r.id == result.id && r.state == OperationState::Succeeded)
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(control.lock().unwrap().writes, 1);
    c.close().await;
}
