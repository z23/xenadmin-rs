use super::*;
use xenadmin_core::{BondMode, ManagementBondOperation as Bond};
fn clear(p: &mut Value) {
    for k in ["IP", "netmask", "gateway", "DNS"] {
        p[k] = json!("");
    }
    p["ip_configuration_mode"] = json!("None");
    p["management"] = json!(false);
}
pub(super) fn install(c: &mut Control, destroy: bool) {
    bond_changes::install(c, false);
    for (child, vlan, tag) in [(90, 95, 17), (91, 96, 18)] {
        let mut p = c.rows[&reference("PIF", 13)].clone();
        p["uuid"] = json!(id(child));
        p["network"] = json!(reference("network", child));
        p["physical"] = json!(false);
        p["VLAN"] = json!(tag);
        p["MAC"] = json!("fe:ff:ff:ff:ff:ff");
        p["VLAN_master_of"] = json!(reference("VLAN", vlan));
        if child == 91 {
            clear(&mut p);
            p["currently_attached"] = json!(false);
        }
        c.rows.insert(reference("PIF", child), p);
        c.rows.insert(reference("VLAN",vlan),json!({"uuid":id(vlan),"tagged_PIF":reference("PIF",13),"untagged_PIF":reference("PIF",child),"tag":tag,"other_config":{}}));
        c.rows.insert(reference("network",child),json!({"uuid":id(child+1000),"name_label":format!("VLAN {tag}"),"MTU":1500,"managed":true,"PIFs":[reference("PIF",child)],"VIFs":[]}));
    }
    let parent = c.rows.get_mut(&reference("PIF", 13)).unwrap();
    clear(parent);
    parent["VLAN_slave_of"] = json!([reference("VLAN", 95), reference("VLAN", 96)]);
    // The earlier bond fixture uses network:90 as its empty destination; use another UUID/ref here.
    c.rows.insert(reference("network",500),json!({"uuid":id(500),"name_label":"Bond uplink","MTU":1500,"managed":true,"PIFs":[],"VIFs":[]}));
    if destroy {
        let fault = c.fault;
        c.fault = Fault::None;
        bond_changes::change(
            c,
            &json!({"method":"Async.Bond.create","params":["session",reference("network",500),[reference("PIF",13),reference("PIF",14)],"","active-backup",{}]}),
        );
        c.fault = fault;
    }
}
pub(super) fn migrate(c: &mut Control, create: bool) {
    let parent = if create { 81 } else { 13 };
    let old = c
        .rows
        .iter()
        .filter(|(r, _)| r.starts_with("OpaqueRef:VLAN:"))
        .map(|(r, v)| (r.clone(), v.clone()))
        .collect::<Vec<_>>();
    for (index, (old_ref, old_vlan)) in old.into_iter().enumerate() {
        if index == 1 && c.fault == Fault::VlanIncompleteAfterWrite {
            continue;
        }
        let old_child = old_vlan["untagged_PIF"].as_str().unwrap().to_owned();
        let old_parent = old_vlan["tagged_PIF"].as_str().unwrap().to_owned();
        let mut p = c.rows.remove(&old_child).unwrap();
        let vlan = id(old_vlan["uuid"]
            .as_str()
            .unwrap()
            .parse::<uuid::Uuid>()
            .unwrap()
            .as_u128()
            + 100);
        let child = id(p["uuid"]
            .as_str()
            .unwrap()
            .parse::<uuid::Uuid>()
            .unwrap()
            .as_u128()
            + 100);
        let new_ref = reference("VLAN", vlan.as_u128());
        let child_ref = reference("PIF", child.as_u128());
        p["uuid"] = json!(child);
        p["device"] = c.rows[&reference("PIF", parent)]["device"].clone();
        p["VLAN_master_of"] = json!(&new_ref);
        p["properties"] = json!({});
        p["other_config"] = json!({});
        if c.fault == Fault::VlanChangedAfterWrite && p["management"] == true {
            p["gateway"] = json!("127.0.0.88");
        }
        c.rows.insert(child_ref.clone(), p);
        c.rows.remove(&old_ref);
        if let Some(p) = c.rows.get_mut(&old_parent) {
            p["VLAN_slave_of"]
                .as_array_mut()
                .unwrap()
                .retain(|v| v != &old_ref);
        }
        c.rows.get_mut(&reference("PIF", parent)).unwrap()["VLAN_slave_of"]
            .as_array_mut()
            .unwrap()
            .push(json!(&new_ref));
        c.rows.insert(new_ref.clone(),json!({"uuid":vlan,"tagged_PIF":reference("PIF",parent),"untagged_PIF":child_ref,"tag":old_vlan["tag"],"other_config":{}}));
    }
    // A freshly created bond has no inherited VLAN references before move_vlan runs.
    for v in c
        .rows
        .values_mut()
        .filter(|v| v.get("VLAN_slave_of").is_some())
    {
        v["VLAN_slave_of"]
            .as_array_mut()
            .unwrap()
            .sort_by_key(|v| v.as_str().unwrap().to_owned());
        v["VLAN_slave_of"].as_array_mut().unwrap().dedup();
    }
}
pub(super) fn reviewed(c: &xenadmin_core::Connection, destroy: bool) -> ManagementNetworkOperation {
    let view = c.latest();
    let i = view.inventory().unwrap();
    let source =
        ManagementInterface::capture(i, id(if destroy { 190 } else { 90 }).into()).unwrap();
    ManagementNetworkOperation::Bond {
        consumers: None,
        source,
        operation: if destroy {
            Bond::Destroy {
                bond: id(80).into(),
                primary: id(13).into(),
            }
        } else {
            Bond::Create {
                members: vec![
                    ManagementInterface::capture(i, id(13).into()).unwrap(),
                    ManagementInterface::capture(i, id(14).into()).unwrap(),
                ],
                network: id(500).into(),
                mode: BondMode::ActiveBackup,
                hash: None,
            }
        },
    }
}
#[tokio::test]
async fn management_vlan_bond_creation_and_removal_verify_new_ids_and_unplugged_sibling() {
    for destroy in [false, true] {
        let (_mock, bridge, control) = fixture(false, Fault::None).await;
        install(&mut control.lock().unwrap(), destroy);
        let dir = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&bridge, &b).await;
        let op = reviewed(&c, destroy);
        assert!(
            op.availability(c.latest().inventory().unwrap())
                .is_available(),
            "{:?}",
            op.availability(c.latest().inventory().unwrap())
        );
        let row = finish(&submit(&c, op)).await;
        assert_eq!(
            row.state,
            OperationState::Succeeded,
            "destroy {destroy}: {}",
            row.detail
        );
        let e = row.management_network.as_ref().unwrap();
        assert!(e.recovery_endpoint_verified);
        assert_eq!(e.moved_vlans.len(), 2);
        assert!(
            e.moved_vlans
                .iter()
                .all(|v| v.old_vlan != v.new_vlan && v.old_interface != v.new_interface)
        );
        assert_eq!(
            control.lock().unwrap().rows[&reference("PIF", if destroy { 291 } else { 191 })]["currently_attached"],
            false
        );
        assert_eq!(control.lock().unwrap().writes, 1);
        c.close().await;
    }
}
#[tokio::test]
async fn management_vlan_bond_refuses_metadata_consumers_and_duplicate_tags() {
    for fault in [
        "vlan_metadata",
        "pif_metadata",
        "properties",
        "vif",
        "tag",
        "mac",
        "ipv6",
    ] {
        let (_mock, bridge, control) = fixture(false, Fault::None).await;
        install(&mut control.lock().unwrap(), false);
        let dir = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&bridge, &b).await;
        let mut op = reviewed(&c, false);
        if ["tag", "mac", "ipv6"].contains(&fault) {
            if let ManagementNetworkOperation::Bond {
                operation: Bond::Create { members, .. },
                ..
            } = &mut op
            {
                let t = members[0].topology.as_mut().unwrap();
                match fault {
                    "tag" => {
                        t.vlans.get_mut(&id(96).into()).unwrap().tag = Some(17);
                        t.pifs.get_mut(&id(91).into()).unwrap().vlan = Some(17);
                    }
                    "mac" => {
                        t.pifs.get_mut(&id(91).into()).unwrap().mac = "02:00:00:00:ff:ff".into()
                    }
                    "ipv6" => {
                        t.pifs
                            .get_mut(&id(91).into())
                            .unwrap()
                            .ipv6_configuration_mode = "Autoconf".into()
                    }
                    _ => unreachable!(),
                }
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
                "{fault}"
            );
        } else {
            {
                let mut state = control.lock().unwrap();
                match fault {
                    "vlan_metadata" => {
                        state.rows.get_mut(&reference("VLAN", 95)).unwrap()["other_config"] =
                            json!({"label":"keep me"})
                    }
                    "pif_metadata" => {
                        state.rows.get_mut(&reference("PIF", 90)).unwrap()["other_config"] =
                            json!({"custom":"keep me"})
                    }
                    "properties" => {
                        state.rows.get_mut(&reference("PIF", 90)).unwrap()["properties"] =
                            json!({"gro":"off"})
                    }
                    "vif" => {
                        state.rows.insert(
                            reference("VIF", 100),
                            json!({"uuid":id(100),"network":reference("network",91)}),
                        );
                    }
                    _ => unreachable!(),
                };
            }
            let row = finish(&submit(&c, op)).await;
            assert_eq!(
                row.state,
                OperationState::Rejected,
                "{fault}: {}",
                row.detail
            );
        }
        assert_eq!(control.lock().unwrap().writes, 0);
        c.close().await;
    }
}
#[tokio::test]
async fn partial_or_wrong_vlan_transition_retains_identified_pairs_without_replay() {
    for fault in [
        Fault::VlanChangedAfterWrite,
        Fault::VlanIncompleteAfterWrite,
    ] {
        let (_mock, bridge, control) = fixture(false, fault).await;
        install(&mut control.lock().unwrap(), false);
        let dir = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&bridge, &b).await;
        let row = finish(&submit(&c, reviewed(&c, false))).await;
        assert_eq!(row.state, OperationState::OutcomeUnknown, "{}", row.detail);
        assert!(
            !row.management_network
                .as_ref()
                .unwrap()
                .moved_vlans
                .is_empty(),
            "{}",
            row.detail
        );
        assert_eq!(control.lock().unwrap().writes, 1);
        c.close().await;
    }
}
#[tokio::test]
async fn saved_vlan_replacement_ids_survive_restart_without_task_and_refuse_rebinding() {
    for (destroy, rebind) in [(false, false), (true, false), (false, true)] {
        let (_mock, bridge, control) = fixture(false, Fault::VlanChangedAfterWrite).await;
        install(&mut control.lock().unwrap(), destroy);
        let dir = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&bridge, &b).await;
        let row = finish(&submit(&c, reviewed(&c, destroy))).await;
        assert_eq!(row.state, OperationState::OutcomeUnknown, "{}", row.detail);
        let expected = row.management_network.as_ref().unwrap().moved_vlans.clone();
        assert_eq!(expected.len(), 2);
        c.close().await;
        drop(b);
        {
            let mut state = control.lock().unwrap();
            state.fault = Fault::None;
            state.rows.remove(&reference("task", 51));
            let p = state
                .rows
                .get_mut(&reference("PIF", if destroy { 290 } else { 190 }))
                .unwrap();
            p["gateway"] = json!("");
            if rebind {
                p["uuid"] = json!(id(999));
            }
        }
        let b = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&bridge, &b).await;
        let recovered = tokio::time::timeout(deadline(8), async {
            loop {
                if let Some(r) = c
                    .resource_history()
                    .iter()
                    .find(|r| {
                        r.id == row.id
                            && r.revision() > row.revision()
                            && if rebind {
                                r.state == OperationState::OutcomeUnknown
                                    && r.detail.contains("VLAN replacement")
                            } else {
                                r.state == OperationState::Succeeded
                            }
                    })
                    .cloned()
                {
                    return r;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            recovered.management_network.as_ref().unwrap().moved_vlans,
            expected
        );
        assert_eq!(control.lock().unwrap().writes, 1);
        c.close().await;
    }
}
