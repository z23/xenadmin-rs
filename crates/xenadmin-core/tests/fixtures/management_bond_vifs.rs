use super::*;
use xenadmin_core::{ManagementVifChange as Change, ManagementVmLocality as Locality};
pub(super) fn guest(
    c: &mut Control,
    n: u128,
    network: u128,
    resident: Option<u128>,
    possible: Option<Vec<u128>>,
    state: &str,
) {
    let vm = reference("VM", n);
    let vif = reference("VIF", n);
    c.rows.insert(vm.clone(),json!({"uuid":id(n),"name_label":format!("Reviewed guest {n}"),"is_a_template":false,"is_a_snapshot":false,"is_control_domain":false,"power_state":state,"resident_on":resident.map(|h|reference("host",h)).unwrap_or("OpaqueRef:NULL".into()),"scheduled_to_be_resident_on":"OpaqueRef:NULL","current_operations":{},"VIFs":[vif],"VBDs":[],"VCPUs_at_startup":"2","memory_static_max":"1073741824","fixture_possible_hosts":possible.map(|hosts|hosts.into_iter().map(|h|reference("host",h)).collect::<Vec<_>>())}));
    c.rows.insert(vif.clone(),json!({"uuid":id(n+1000),"VM":vm,"network":reference("network",network),"device":"0","MAC":"02:00:00:00:00:01","MTU":"1500","currently_attached":state!="Halted","current_operations":{},"allowed_operations":[],"other_config":{"preserved":"value"},"qos_algorithm_type":"ratelimit","qos_algorithm_params":{"kbps":"4096"},"locking_mode":"locked","ipv4_allowed":["192.0.2.2","192.0.2.1"],"ipv6_allowed":[],"ipv4_configuration_mode":"None","ipv4_addresses":[],"ipv4_gateway":"","ipv6_configuration_mode":"None","ipv6_addresses":[],"ipv6_gateway":"","trunks":[]}));
    c.rows.get_mut(&reference("network", network)).unwrap()["VIFs"]
        .as_array_mut()
        .unwrap()
        .push(json!(vif));
}
fn install(c: &mut Control, destroy: bool, vlan: bool) {
    if vlan {
        vlan_bond_changes::install(c, destroy);
    } else {
        bond_changes::install(c, destroy);
    }
    c.rows.insert(
        reference("host", 9),
        json!({"uuid":id(9),"name_label":"Other host","address":"192.0.2.9","enabled":true,"fixture_distinct_certificate":true}),
    );
    let network = if vlan || destroy { 90 } else { 3 };
    for (n, resident, hosts, state) in [
        (600, Some(2), None, "Running"),
        (601, Some(9), None, "Running"),
        (602, None, Some(vec![]), "Halted"),
        (603, None, Some(vec![2]), "Halted"),
        (604, None, Some(vec![9, 2]), "Halted"),
        (605, None, Some(vec![9]), "Halted"),
        (606, Some(2), None, "Paused"),
        (607, None, Some(vec![2]), "Suspended"),
    ] {
        guest(c, n, network, resident, hosts, state);
    }
    for (r, v) in &mut c.rows {
        if r.starts_with("OpaqueRef:network:") {
            v["default_locking_mode"] = json!("unlocked");
            v["other_config"] = json!({});
        }
    }
    if vlan {
        guest(c, 608, 91, Some(2), None, "Running");
    }
}
pub(super) fn migrate(c: &mut Control, r: &Value) {
    let create = r["method"] == "Async.Bond.create";
    let sources = if create {
        r["params"][2]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|p| {
                c.rows
                    .get(p.as_str().unwrap())
                    .map(|p| p["network"].clone())
            })
            .collect::<Vec<_>>()
    } else {
        vec![c.rows[&reference("PIF", 81)]["network"].clone()]
    };
    let destination = if create {
        r["params"][1].clone()
    } else {
        c.rows[&reference("PIF", 13)]["network"].clone()
    };
    let vifs = c
        .rows
        .iter()
        .filter(|(r, v)| r.starts_with("OpaqueRef:VIF:") && sources.contains(&v["network"]))
        .map(|(r, v)| (r.clone(), v.clone()))
        .collect::<Vec<_>>();
    for (reference, v) in vifs {
        let vm = &c.rows[v["VM"].as_str().unwrap()];
        let local = vm["resident_on"] == super::reference("host", 2)
            || vm["resident_on"] == "OpaqueRef:NULL"
                && (vm["fixture_possible_hosts"] == json!([])
                    || vm["fixture_possible_hosts"] == json!([super::reference("host", 2)]));
        if local && c.fault != Fault::VifIgnored {
            c.rows.get_mut(&reference).unwrap()["network"] = destination.clone();
        }
        if c.fault == Fault::VifSettingsChanged {
            c.rows.get_mut(&reference).unwrap()["MAC"] = json!("02:00:00:00:00:ff");
        }
        if c.fault == Fault::VifRebound {
            c.rows.get_mut(&reference).unwrap()["uuid"] = json!(id(9999));
        }
    }
    if c.fault == Fault::NetworkPolicyChanged {
        c.rows.get_mut(destination.as_str().unwrap()).unwrap()["default_locking_mode"] =
            json!("disabled");
    }
    let membership = c
        .rows
        .iter()
        .filter(|(r, _)| r.starts_with("OpaqueRef:VIF:"))
        .map(|(r, v)| (r.clone(), v["network"].clone()))
        .collect::<Vec<_>>();
    for (r, n) in &mut c.rows {
        if r.starts_with("OpaqueRef:network:") {
            n["VIFs"] = json!(
                membership
                    .iter()
                    .filter(|(_, net)| net == r)
                    .map(|(r, _)| r.clone())
                    .collect::<Vec<_>>()
            );
        }
    }
}
async fn review(
    c: &xenadmin_core::Connection,
    op: ManagementNetworkOperation,
) -> Result<ManagementNetworkOperation, String> {
    let q = c
        .review_management_bond(
            c.latest()
                .observe_resource(ResourceTarget::Host(id(2).into()))
                .unwrap(),
            op,
        )
        .unwrap();
    tokio::time::timeout(deadline(8), async {
        loop {
            if let Some(r) = q.latest() {
                break r;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap()
}
fn intent(c: &xenadmin_core::Connection, destroy: bool, vlan: bool) -> ManagementNetworkOperation {
    if vlan {
        vlan_bond_changes::reviewed(c, destroy)
    } else {
        bond_changes::reviewed(c, destroy)
    }
}
#[tokio::test]
async fn management_bond_guest_review_moves_only_authoritative_local_interfaces() {
    for destroy in [false, true] {
        let (_mock, bridge, control) = fixture(false, Fault::None).await;
        install(&mut control.lock().unwrap(), destroy, false);
        let dir = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&bridge, &b).await;
        let op = review(&c, intent(&c, destroy, false)).await.unwrap();
        let ManagementNetworkOperation::Bond {
            consumers: Some(ref consumers),
            ..
        } = op
        else {
            panic!()
        };
        assert_eq!(consumers.interfaces.len(), 8);
        for n in [600, 602, 603, 606, 607] {
            assert_eq!(
                consumers.interfaces[&id(n + 1000).into()].change,
                Change::Move
            );
        }
        for n in [601, 604, 605] {
            assert_eq!(
                consumers.interfaces[&id(n + 1000).into()].change,
                Change::Retain
            );
        }
        assert_eq!(
            consumers.interfaces[&id(1602).into()].locality,
            Locality::PossibleHosts(vec![])
        );
        assert_eq!(control.lock().unwrap().writes, 0, "review is read-only");
        let row = finish(&submit(&c, op)).await;
        assert_eq!(row.state, OperationState::Succeeded, "{}", row.detail);
        assert_eq!(control.lock().unwrap().writes, 1);
        assert!(
            !serde_json::to_string(row.as_ref())
                .unwrap()
                .contains("OpaqueRef:")
        );
        c.close().await;
    }
}
#[tokio::test]
async fn management_vlan_bond_guest_review_preserves_networks_and_attachment() {
    for destroy in [false, true] {
        let (_mock, bridge, control) = fixture(false, Fault::None).await;
        install(&mut control.lock().unwrap(), destroy, true);
        let dir = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&bridge, &b).await;
        let op = review(&c, intent(&c, destroy, true)).await.unwrap();
        let ManagementNetworkOperation::Bond {
            consumers: Some(ref consumers),
            ..
        } = op
        else {
            panic!()
        };
        assert_eq!(
            consumers.interfaces[&id(1600).into()].change,
            Change::Reattach
        );
        assert_eq!(
            consumers.interfaces[&id(1608).into()].change,
            Change::Retain
        );
        assert!(
            consumers
                .interfaces
                .values()
                .all(|v| v.network == v.destination)
        );
        let row = finish(&submit(&c, op)).await;
        assert_eq!(row.state, OperationState::Succeeded, "{}", row.detail);
        c.close().await;
    }
}
#[tokio::test]
async fn management_guest_review_refuses_unknown_locality_and_inconsistent_graph() {
    for change in 0..4 {
        let (_mock, bridge, control) = fixture(false, Fault::None).await;
        install(&mut control.lock().unwrap(), false, false);
        {
            let mut s = control.lock().unwrap();
            match change {
                0 => {
                    s.rows.get_mut(&reference("VM", 602)).unwrap()["fixture_possible_hosts"] =
                        Value::Null
                }
                1 => s.rows.get_mut(&reference("network", 3)).unwrap()["VIFs"] = json!([]),
                2 => s.rows.get_mut(&reference("VM", 600)).unwrap()["VIFs"] = json!([]),
                _ => {
                    s.rows.get_mut(&reference("VM", 600)).unwrap()["is_control_domain"] =
                        json!(true)
                }
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&bridge, &b).await;
        assert!(review(&c, intent(&c, false, false)).await.is_err());
        assert_eq!(control.lock().unwrap().writes, 0);
        c.close().await;
    }
}
#[tokio::test]
async fn management_bond_dispatch_rechecks_exact_consumers_settings_and_locality() {
    for change in 0..8 {
        let (_mock, bridge, control) = fixture(false, Fault::None).await;
        install(&mut control.lock().unwrap(), false, false);
        let dir = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&bridge, &b).await;
        let op = review(&c, intent(&c, false, false)).await.unwrap();
        {
            let mut s = control.lock().unwrap();
            match change {
                0 => {
                    s.rows.get_mut(&reference("VIF", 600)).unwrap()["MAC"] =
                        json!("02:00:00:00:00:ff")
                }
                1 => guest(&mut s, 609, 3, Some(2), None, "Running"),
                2 => {
                    s.rows.get_mut(&reference("VM", 602)).unwrap()["fixture_possible_hosts"] =
                        json!([reference("host", 9)])
                }
                3 => {
                    s.rows.get_mut(&reference("VM", 600)).unwrap()["resident_on"] =
                        json!(reference("host", 9))
                }
                4 => {
                    s.rows.get_mut(&reference("VM", 600)).unwrap()["power_state"] = json!("Halted")
                }
                6 => {
                    s.rows.get_mut(&reference("network", 90)).unwrap()["default_locking_mode"] =
                        json!("disabled")
                }
                7 => {
                    s.rows.get_mut(&reference("network", 3)).unwrap()["other_config"] =
                        json!({"changed":"setting"})
                }
                _ => {
                    s.rows.get_mut(&reference("VIF", 600)).unwrap()["qos_algorithm_params"] =
                        json!({"kbps":"8192"})
                }
            }
        }
        let row = finish(&submit(&c, op)).await;
        assert_eq!(
            row.state,
            OperationState::Rejected,
            "{change}: {}",
            row.detail
        );
        assert_eq!(control.lock().unwrap().writes, 0);
        c.close().await;
    }
}
#[tokio::test]
async fn management_bond_guest_uncertainty_retains_identity_and_never_replays() {
    for fault in [
        Fault::LostReply,
        Fault::VifIgnored,
        Fault::VifSettingsChanged,
        Fault::VifRebound,
        Fault::NetworkPolicyChanged,
    ] {
        let (_mock, bridge, control) = fixture(false, fault).await;
        install(&mut control.lock().unwrap(), false, false);
        let dir = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&bridge, &b).await;
        let op = review(&c, intent(&c, false, false)).await.unwrap();
        let row = finish(&submit(&c, op)).await;
        assert_eq!(row.state, OperationState::OutcomeUnknown, "{}", row.detail);
        c.control_resource(row.id, OperationControl::Recheck)
            .unwrap();
        let again = bond_changes::rechecked(&c, row.id, row.revision()).await;
        assert_eq!(
            again.state,
            OperationState::OutcomeUnknown,
            "{}",
            again.detail
        );
        assert_eq!(control.lock().unwrap().writes, 1);
        c.close().await;
    }
}
#[tokio::test]
async fn management_guest_review_allows_read_only_and_fences_closed_connection() {
    let (_mock, bridge, control) = fixture(false, Fault::None).await;
    install(&mut control.lock().unwrap(), false, false);
    let dir = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(dir.path().into()).unwrap();
    let mut options = ConnectionOptions::new(bridge.endpoint.clone());
    options.accept_invalid_certificates = true;
    options.read_only = true;
    let c = b.connect(options, Credentials::new("root".into(), "fixture".into()));
    c.wait_ready(deadline(5)).await.unwrap();
    let op = review(&c, intent(&c, false, false)).await.unwrap();
    assert!(matches!(
        c.submit_resource(
            OperationId::new(),
            ResourceCommand::ManagementNetwork {
                host: c
                    .latest()
                    .observe_resource(ResourceTarget::Host(id(2).into()))
                    .unwrap(),
                operation: Box::new(op)
            }
        ),
        Err(xenadmin_core::Error::ReadOnly)
    ));
    let (entered, release) = {
        let mut c = control.lock().unwrap();
        c.review_delay = true;
        (c.entered.clone(), c.release.clone())
    };
    let q = c
        .review_management_bond(
            c.latest()
                .observe_resource(ResourceTarget::Host(id(2).into()))
                .unwrap(),
            intent(&c, false, false),
        )
        .unwrap();
    tokio::time::timeout(deadline(5), entered.notified())
        .await
        .unwrap();
    c.close().await;
    let result = tokio::time::timeout(deadline(5), async {
        loop {
            if let Some(r) = q.latest() {
                break r;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert!(result.is_err());
    release.notify_one();
    assert_eq!(control.lock().unwrap().writes, 0);
}
#[tokio::test]
async fn management_guest_restart_verifies_saved_task_and_preserves_review_without_replay() {
    let (_mock, bridge, control) = fixture(false, Fault::VifSettingsChanged).await;
    install(&mut control.lock().unwrap(), false, false);
    let dir = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&bridge, &b).await;
    let op = review(&c, intent(&c, false, false)).await.unwrap();
    let row = finish(&submit(&c, op.clone())).await;
    assert_eq!(row.state, OperationState::OutcomeUnknown, "{}", row.detail);
    assert!(
        row.management_network
            .as_ref()
            .unwrap()
            .created_bond
            .is_some()
    );
    c.close().await;
    {
        let mut s = control.lock().unwrap();
        for (r, v) in &mut s.rows {
            if r.starts_with("OpaqueRef:VIF:") {
                v["MAC"] = json!("02:00:00:00:00:01");
                v["ipv4_allowed"] = json!(["192.0.2.1", "192.0.2.2"]);
            }
        }
        s.rows.remove(&reference("task", 51));
    }
    let b = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&bridge, &b).await;
    let saved = c
        .resource_history()
        .iter()
        .find(|r| r.id == row.id)
        .cloned()
        .unwrap();
    c.control_resource(row.id, OperationControl::Recheck)
        .unwrap();
    let done = bond_changes::rechecked(&c, row.id, saved.revision()).await;
    assert_eq!(done.state, OperationState::Succeeded, "{}", done.detail);
    assert_eq!(
        done.action,
        xenadmin_core::ResourceAction::ManagementNetwork(Box::new(op))
    );
    assert_eq!(control.lock().unwrap().writes, 1);
    c.close().await;
}
