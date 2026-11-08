use super::*;
use xenadmin_core::{BondMode, ManagementBondOperation as Bond};
fn clear(p: &mut Value) {
    for key in ["IP", "netmask", "gateway", "DNS"] {
        p[key] = json!("");
    }
    p["ip_configuration_mode"] = json!("None");
    p["management"] = json!(false);
}
pub(super) fn install(c: &mut Control, destroy: bool) {
    let p = c.rows.get_mut(&reference("PIF", 14)).unwrap();
    clear(p);
    c.rows.insert(reference("network",90),json!({"uuid":id(190),"name_label":"Management bond","managed":true,"MTU":1500,"PIFs":[],"VIFs":[]}));
    if destroy {
        change(
            c,
            &json!({"method":"Async.Bond.create","params":["session",reference("network",90),[reference("PIF",13),reference("PIF",14)],"","active-backup",{}]}),
        );
    }
}
pub(super) fn change(c: &mut Control, r: &Value) {
    if r["method"] == "Async.Bond.create" {
        let mut master = c.rows[&reference("PIF", 13)].clone();
        master["uuid"] = json!(id(81));
        master["network"] = r["params"][1].clone();
        master["device"] = json!("bond0");
        master["physical"] = json!(false);
        master["bond_master_of"] = json!([reference("Bond", 80)]);
        master["VLAN_slave_of"] = json!([]);
        c.rows.insert(reference("PIF", 81), master);
        for n in [13, 14] {
            let p = c.rows.get_mut(&reference("PIF", n)).unwrap();
            clear(p);
            p["bond_slave_of"] = json!(reference("Bond", 80));
        }
        c.rows.insert(reference("Bond",80),json!({"uuid":id(80),"master":reference("PIF",81),"slaves":r["params"][2],"primary_slave":reference("PIF",13),"mode":r["params"][4],"properties":r["params"][5],"links_up":2}));
        if c.fault == Fault::BondChangedAfterWrite {
            c.rows.get_mut(&reference("PIF", 81)).unwrap()["netmask"] = json!("255.255.255.0");
        }
    } else {
        let master = c.rows.remove(&reference("PIF", 81)).unwrap();
        let primary = c.rows.get_mut(&reference("PIF", 13)).unwrap();
        for key in ["IP", "netmask", "gateway", "DNS", "ip_configuration_mode"] {
            primary[key] = master[key].clone();
        }
        primary["management"] = master["management"].clone();
        for n in [13, 14] {
            c.rows.get_mut(&reference("PIF", n)).unwrap()["bond_slave_of"] =
                json!("OpaqueRef:NULL");
        }
        c.rows.remove(&reference("Bond", 80));
    }
    super::vlan_bond_changes::migrate(c, r["method"] == "Async.Bond.create");
}

fn reviewed(c: &xenadmin_core::Connection, destroy: bool) -> ManagementNetworkOperation {
    let view = c.latest();
    let i = view.inventory().unwrap();
    let source = ManagementInterface::capture(i, id(if destroy { 81 } else { 13 }).into()).unwrap();
    ManagementNetworkOperation::Bond {
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
                network: id(190).into(),
                mode: BondMode::ActiveBackup,
                hash: None,
            }
        },
    }
}
#[tokio::test]
async fn management_bond_creation_and_removal_preserve_address_and_verify_bound_route() {
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
        assert_eq!(row.state, OperationState::Succeeded, "{}", row.detail);
        let evidence = row.management_network.as_ref().unwrap();
        assert!(evidence.configuration_observed && evidence.recovery_endpoint_verified);
        assert_eq!(evidence.created_bond.is_some(), !destroy);
        assert_eq!(control.lock().unwrap().writes, 1);
        c.close().await;
    }
}
#[tokio::test]
async fn failed_lost_or_unverified_management_bond_never_replays() {
    for fault in [
        Fault::LostReply,
        Fault::Failed,
        Fault::Pending,
        Fault::BondWrongResult,
        Fault::BondChangedAfterWrite,
    ] {
        let (_mock, bridge, control) = fixture(false, fault).await;
        install(&mut control.lock().unwrap(), false);
        let dir = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&bridge, &b).await;
        let op = submit(&c, reviewed(&c, false));
        if fault == Fault::Pending {
            tokio::time::timeout(Duration::from_secs(5), async {
                while op.latest().state != OperationState::Tracking {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
            c.control_resource(op.latest().id, OperationControl::RequestCancellation)
                .unwrap();
        }
        let row = finish(&op).await;
        assert_eq!(row.state, OperationState::OutcomeUnknown, "{}", row.detail);
        if fault == Fault::BondChangedAfterWrite {
            assert!(
                row.management_network
                    .as_ref()
                    .unwrap()
                    .created_bond
                    .is_some()
            );
        }
        c.control_resource(row.id, OperationControl::Recheck)
            .unwrap();
        let after = rechecked(&c, row.id, row.revision()).await;
        assert_eq!(
            after.state,
            OperationState::OutcomeUnknown,
            "{}",
            after.detail
        );
        assert_eq!(control.lock().unwrap().writes, 1);
        c.close().await;
    }
}
async fn rechecked(
    c: &xenadmin_core::Connection,
    id: xenadmin_core::OperationId,
    revision: u64,
) -> ResourceOperationRecord {
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            if let Some(row) = c
                .resource_history()
                .iter()
                .find(|r| {
                    r.id == id
                        && r.revision() > revision
                        && (r.state.terminal() || r.state == OperationState::OutcomeUnknown)
                })
                .cloned()
            {
                return row;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}
#[tokio::test]
async fn changed_management_bond_dependencies_refuse_before_dispatch() {
    for fault in [
        "member",
        "vif",
        "cluster",
        "properties",
        "destination",
        "protected",
    ] {
        let (_mock, bridge, control) = fixture(false, Fault::None).await;
        install(&mut control.lock().unwrap(), false);
        let dir = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&bridge, &b).await;
        let op = reviewed(&c, false);
        {
            let mut v = control.lock().unwrap();
            match fault {
                "member" => {
                    v.rows.get_mut(&reference("PIF", 14)).unwrap()["MAC"] =
                        json!("02:00:00:aa:bb:cc")
                }
                "vif" => {
                    v.rows.insert(
                        reference("VIF", 90),
                        json!({"uuid":id(390),"network":reference("network",3)}),
                    );
                }
                "cluster" => {
                    v.rows.insert(
                        reference("Cluster_host", 90),
                        json!({"uuid":id(390),"host":reference("host",2)}),
                    );
                }
                "properties" => {
                    v.rows.get_mut(&reference("PIF", 14)).unwrap()["properties"] =
                        json!({"gro":"off"})
                }
                "destination" => {
                    v.rows.get_mut(&reference("network", 90)).unwrap()["MTU"] = json!(9000)
                }
                "protected" => {
                    v.rows.get_mut(&reference("PIF", 14)).unwrap()["disallow_unplug"] = json!(true)
                }
                _ => unreachable!(),
            }
        }
        let row = finish(&submit(&c, op)).await;
        assert_eq!(
            row.state,
            OperationState::Rejected,
            "{fault}: {}",
            row.detail
        );
        assert_eq!(control.lock().unwrap().writes, 0);
        c.close().await;
    }
}
#[tokio::test]
async fn recorded_management_bond_completion_recovers_without_task_or_second_write() {
    for destroy in [false, true] {
        let (_mock, bridge, control) = fixture(false, Fault::TopologyReadAfterCompletion).await;
        install(&mut control.lock().unwrap(), destroy);
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&bridge, &backend).await;
        let result = finish(&submit(&c, reviewed(&c, destroy))).await;
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
        let row = tokio::time::timeout(Duration::from_secs(8), async {
            loop {
                if let Some(row) = c
                    .resource_history()
                    .iter()
                    .find(|r| r.id == result.id && r.state == OperationState::Succeeded)
                    .cloned()
                {
                    return row;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert!(
            row.management_network
                .as_ref()
                .unwrap()
                .recovery_endpoint_verified
        );
        assert_eq!(control.lock().unwrap().writes, 1);
        c.close().await;
    }
}
#[tokio::test]
async fn late_management_bond_receipt_preserves_recovered_state_without_opening_route() {
    for acknowledge in [false, true] {
        let (mock, bridge, control) = fixture(false, Fault::None).await;
        let (entered, release) = {
            let mut v = control.lock().unwrap();
            install(&mut v, false);
            v.delay = true;
            (v.entered.clone(), v.release.clone())
        };
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&bridge, &backend).await;
        let op = submit(&c, reviewed(&c, false));
        tokio::time::timeout(Duration::from_secs(5), entered.notified())
            .await
            .unwrap();
        let second = Backend::with_state_directory(dir.path().into()).unwrap();
        let recovered = connect(&bridge, &second).await;
        tokio::time::timeout(Duration::from_secs(5), async {
            while !recovered
                .resource_history()
                .iter()
                .any(|r| r.id == op.latest().id && r.state == OperationState::OutcomeUnknown)
            {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        if acknowledge {
            recovered
                .control_resource(op.latest().id, OperationControl::AcknowledgeUnknown)
                .unwrap();
            tokio::time::timeout(Duration::from_secs(5), async {
                while !recovered.resource_history().iter().any(|r| {
                    r.id == op.latest().id && r.state == OperationState::AcknowledgedUnknown
                }) {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
        }
        recovered.close().await;
        let logins = mock.logins.load(Ordering::Relaxed);
        release.notify_one();
        let result = finish(&op).await;
        assert_eq!(
            result.state,
            if acknowledge {
                OperationState::AcknowledgedUnknown
            } else {
                OperationState::OutcomeUnknown
            }
        );
        assert!(
            !result
                .management_network
                .as_ref()
                .unwrap()
                .configuration_observed
        );
        assert_eq!(mock.logins.load(Ordering::Relaxed), logins);
        assert_eq!(control.lock().unwrap().writes, 1);
        c.close().await;
    }
}
