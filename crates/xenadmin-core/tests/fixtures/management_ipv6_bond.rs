use super::*;
use xenadmin_core::{BondMode, ManagementBondOperation as Bond, ManagementIpv6Bond};
fn install(c: &mut Control, destroy: bool) {
    c.staged_ipv6 = true;
    bond_changes::install(c, false);
    let mut temporary = c.rows[&reference("PIF", 13)].clone();
    temporary["uuid"] = json!(id(18));
    temporary["network"] = json!(reference("network", 8));
    temporary["device"] = json!("eth8");
    temporary["MAC"] = json!("02:00:00:00:00:08");
    temporary["management"] = json!(false);
    temporary["IPv6"] = json!(["::ffff:127.0.0.2/128"]);
    c.rows.insert(reference("PIF", 18), temporary);
    c.rows.insert(reference("network",8),json!({"uuid":id(8),"name_label":"Independent IPv6 management","MTU":1500,"managed":true,"PIFs":[reference("PIF",18)],"VIFs":[]}));
    c.rows.get_mut(&reference("PIF", 14)).unwrap()["ipv6_configuration_mode"] = json!("None");
    c.rows.get_mut(&reference("PIF", 14)).unwrap()["IPv6"] = json!([""]);
    if destroy {
        bond_changes::install(c, true);
        let p = c.rows.get_mut(&reference("PIF", 13)).unwrap();
        p["ipv6_configuration_mode"] = json!("None");
        p["IPv6"] = json!([""]);
    }
}
fn operation(c: &xenadmin_core::Connection, destroy: bool) -> ManagementIpv6Bond {
    let view = c.latest();
    let i = view.inventory().unwrap();
    ManagementIpv6Bond {
        purpose: Default::default(),
        source: ManagementInterface::capture(i, id(if destroy { 81 } else { 13 }).into()).unwrap(),
        temporary: ManagementInterface::capture(i, id(18).into()).unwrap(),
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
        consumers: None,
        predecessor: None,
    }
}
fn submit(c: &xenadmin_core::Connection, op: ManagementIpv6Bond) -> ResourceOperation {
    c.submit_resource(
        OperationId::new(),
        ResourceCommand::ManagementIpv6Bond {
            host: c
                .latest()
                .observe_resource(ResourceTarget::Host(id(2).into()))
                .unwrap(),
            operation: Box::new(op),
        },
    )
    .unwrap()
}
#[tokio::test]
async fn static_ipv6_bond_uses_verified_independent_path_and_explicit_once_only_stages() {
    for destroy in [false, true] {
        let (_mock, bridge, control) = fixture(true, Fault::None).await;
        install(&mut control.lock().unwrap(), destroy);
        let dir = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&bridge, &b).await;
        let mut op = operation(&c, destroy);
        assert!(
            op.availability(c.latest().inventory().unwrap())
                .is_available(),
            "{:?}",
            op.availability(c.latest().inventory().unwrap())
        );
        for stage in 0..5 {
            let r = finish(&submit(&c, op.clone())).await;
            assert_eq!(
                r.state,
                if stage == 4 {
                    OperationState::Succeeded
                } else {
                    OperationState::PartiallyCompleted
                },
                "destroy={destroy}, stage={stage}: {} {:?}",
                r.detail,
                r.local_error
            );
            assert_eq!(
                r.management_ipv6_bond.as_ref().unwrap().completed_stages,
                stage + 1
            );
            assert_eq!(control.lock().unwrap().writes, stage + 1);
            if stage < 4 {
                let blocked = finish(&submit(&c, operation(&c, destroy))).await;
                assert_eq!(blocked.state, OperationState::Rejected);
                assert_eq!(control.lock().unwrap().writes, stage + 1);
                let old = op.clone();
                op.predecessor = Some(r.id);
                if stage > 0 {
                    let duplicate = finish(&submit(&c, old)).await;
                    assert_eq!(duplicate.state, OperationState::Rejected);
                }
            }
            let public = serde_json::to_string(&*r).unwrap();
            assert!(
                !public.contains("OpaqueRef:") && !public.contains("management_ipv6_bond_plan")
            );
        }
        {
            let g = control.lock().unwrap();
            let target = if destroy { 13 } else { 81 };
            assert_eq!(g.rows[&reference("PIF", target)]["management"], true);
            assert_eq!(
                g.rows[&reference("PIF", target)]["IPv6"],
                json!(["::1/128"])
            );
            assert_eq!(
                g.rows[&reference("PIF", 18)]["IPv6"],
                json!(["::ffff:127.0.0.2/128"])
            );
            assert_eq!(g.requests.len(), 5);
        }
        c.close().await;
    }
}
async fn stage(
    c: &xenadmin_core::Connection,
    op: &mut ManagementIpv6Bond,
    n: usize,
) -> Arc<ResourceOperationRecord> {
    let r = finish(&submit(c, op.clone())).await;
    assert_eq!(
        r.state,
        if n == 4 {
            OperationState::Succeeded
        } else {
            OperationState::PartiallyCompleted
        },
        "stage {n}: {} {:?}",
        r.detail,
        r.local_error
    );
    op.predecessor = Some(r.id);
    r
}
async fn settled(
    c: &xenadmin_core::Connection,
    id: OperationId,
    revision: u64,
) -> Arc<ResourceOperationRecord> {
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            if let Some(r) = c.resource_history().iter().find(|r| {
                r.id == id
                    && r.revision() > revision
                    && (r.state.terminal() || r.state == OperationState::OutcomeUnknown)
            }) {
                return Arc::new(r.clone());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}
#[tokio::test]
async fn every_uncertain_ipv6_stage_refuses_continuation_and_recheck_never_replays() {
    for n in 0..5 {
        for fault in [Fault::LostReply, Fault::Failed, Fault::Pending] {
            let (_mock, bridge, control) = fixture(true, Fault::None).await;
            install(&mut control.lock().unwrap(), false);
            let dir = tempfile::tempdir().unwrap();
            let b = Backend::with_state_directory(dir.path().into()).unwrap();
            let c = connect(&bridge, &b).await;
            let mut op = operation(&c, false);
            for prefix in 0..n {
                stage(&c, &mut op, prefix).await;
            }
            control.lock().unwrap().fault = fault;
            let pending = submit(&c, op.clone());
            if fault == Fault::Pending {
                tokio::time::timeout(Duration::from_secs(5), async {
                    while pending.latest().state != OperationState::Tracking {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                })
                .await
                .unwrap();
                c.control_resource(pending.latest().id, OperationControl::RequestCancellation)
                    .unwrap();
            }
            let r = finish(&pending).await;
            assert_eq!(
                r.state,
                OperationState::OutcomeUnknown,
                "stage {n}: {}",
                r.detail
            );
            op.predecessor = Some(r.id);
            let blocked = finish(&submit(&c, op)).await;
            assert_eq!(blocked.state, OperationState::Rejected);
            assert_eq!(control.lock().unwrap().writes, n + 1);
            c.control_resource(r.id, OperationControl::Recheck).unwrap();
            let r = settled(&c, r.id, r.revision()).await;
            assert_eq!(
                r.state,
                OperationState::OutcomeUnknown,
                "stage {n}: {}",
                r.detail
            );
            assert_eq!(control.lock().unwrap().writes, n + 1);
            assert!(
                !control
                    .lock()
                    .unwrap()
                    .requests
                    .iter()
                    .any(|r| r["method"] == "task.cancel")
            );
            c.close().await;
        }
    }
}
#[tokio::test]
async fn concurrent_ipv6_continuations_claim_one_predecessor_and_preserve_pool_reservation() {
    let (_mock, bridge, control) = fixture(true, Fault::None).await;
    install(&mut control.lock().unwrap(), false);
    let dir = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&bridge, &b).await;
    let mut op = operation(&c, false);
    let first = stage(&c, &mut op, 0).await;
    let a = submit(&c, op.clone());
    let z = submit(&c, op.clone());
    let (a, z) = tokio::join!(finish(&a), finish(&z));
    assert_eq!(
        [&a, &z]
            .iter()
            .filter(|r| r.state == OperationState::PartiallyCompleted)
            .count(),
        1
    );
    assert_eq!(
        [&a, &z]
            .iter()
            .filter(|r| r.state == OperationState::Rejected)
            .count(),
        1
    );
    assert_eq!(control.lock().unwrap().writes, 2);
    let child = if a.state == OperationState::PartiallyCompleted {
        a
    } else {
        z
    };
    let history = c.resource_history();
    let parent = history.iter().find(|r| r.id == first.id).unwrap();
    assert_eq!(
        parent.management_ipv6_bond.as_ref().unwrap().successor,
        Some(child.id)
    );
    assert!(
        !parent
            .management_ipv6_bond
            .as_ref()
            .unwrap()
            .retains_reservation
    );
    assert!(
        child
            .management_ipv6_bond
            .as_ref()
            .unwrap()
            .retains_reservation
    );
    let blocked = finish(&submit(&c, op)).await;
    assert_eq!(blocked.state, OperationState::Rejected);
    c.close().await;
}
#[tokio::test]
async fn ipv6_route_and_temporary_component_must_be_proven_before_the_first_write() {
    for fault in [
        "tls",
        "ha",
        "address",
        "protected",
        "purpose",
        "cluster",
        "guest",
    ] {
        let (mock, bridge, control) = fixture(true, Fault::None).await;
        install(&mut control.lock().unwrap(), false);
        let dir = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&bridge, &b).await;
        let op = operation(&c, false);
        {
            let mut g = control.lock().unwrap();
            match fault {
                "tls" => g.wrong_certificate.store(true, Ordering::Relaxed),
                "ha" => g.rows.get_mut(&reference("pool", 1)).unwrap()["ha_enabled"] = json!(true),
                "address" => {
                    g.rows.get_mut(&reference("PIF", 18)).unwrap()["IPv6"] =
                        json!(["2001:db8::7/64"])
                }
                "protected" => {
                    g.rows.get_mut(&reference("PIF", 18)).unwrap()["disallow_unplug"] = json!(true)
                }
                "purpose" => {
                    g.rows.get_mut(&reference("PIF", 18)).unwrap()["other_config"] =
                        json!({"management_purpose":"storage"})
                }
                "cluster" => {
                    g.rows.insert(
                        reference("Cluster_host", 70),
                        json!({"uuid":id(70),"host":reference("host",2)}),
                    );
                }
                "guest" => {
                    g.rows.insert(
                        reference("VIF", 70),
                        json!({"uuid":id(70),"network":reference("network",8)}),
                    );
                }
                _ => unreachable!(),
            }
        }
        let r = finish(&submit(&c, op)).await;
        assert_eq!(r.state, OperationState::Rejected, "{fault}: {}", r.detail);
        assert_eq!(control.lock().unwrap().writes, 0);
        if fault == "tls" {
            assert_eq!(
                mock.logins.load(Ordering::Relaxed),
                1,
                "the wrong TLS leaf must be rejected before credentials"
            );
        }
        c.close().await;
    }
}
#[tokio::test]
async fn predispatch_changed_graph_retains_the_verified_prefix_and_can_continue_only_after_repair()
{
    let (_mock, bridge, control) = fixture(true, Fault::None).await;
    install(&mut control.lock().unwrap(), false);
    let dir = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&bridge, &b).await;
    let mut op = operation(&c, false);
    stage(&c, &mut op, 0).await;
    control
        .lock()
        .unwrap()
        .rows
        .get_mut(&reference("PIF", 14))
        .unwrap()["MAC"] = json!("02:00:00:00:ff:ff");
    let r = finish(&submit(&c, op.clone())).await;
    assert_eq!(
        r.state,
        OperationState::PartiallyCompleted,
        "{} {:?}",
        r.detail,
        r.local_error
    );
    assert_eq!(r.management_ipv6_bond.as_ref().unwrap().completed_stages, 1);
    assert!(r.management_ipv6_bond.as_ref().unwrap().retains_reservation);
    assert_eq!(control.lock().unwrap().writes, 1);
    control
        .lock()
        .unwrap()
        .rows
        .get_mut(&reference("PIF", 14))
        .unwrap()["MAC"] = json!("02:00:00:00:00:04");
    op.predecessor = Some(r.id);
    stage(&c, &mut op, 1).await;
    assert_eq!(control.lock().unwrap().writes, 2);
    c.close().await;
}
#[tokio::test]
async fn ipv6_late_task_receipt_does_not_revive_an_interrupted_stage_and_recheck_finishes_without_replay()
 {
    let (_mock, bridge, control) = fixture(true, Fault::None).await;
    install(&mut control.lock().unwrap(), false);
    let dir = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&bridge, &b).await;
    let mut op = operation(&c, false);
    stage(&c, &mut op, 0).await;
    let (entered, release) = {
        let mut g = control.lock().unwrap();
        g.delay = true;
        (g.entered.clone(), g.release.clone())
    };
    let pending = submit(&c, op.clone());
    tokio::time::timeout(Duration::from_secs(5), entered.notified())
        .await
        .unwrap();
    let second = Backend::with_state_directory(dir.path().into()).unwrap();
    let recovered = connect(&bridge, &second).await;
    tokio::time::timeout(Duration::from_secs(5), async {
        while !recovered
            .resource_history()
            .iter()
            .any(|r| r.id == pending.latest().id && r.state == OperationState::OutcomeUnknown)
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    recovered.close().await;
    release.notify_one();
    let r = finish(&pending).await;
    assert_eq!(r.state, OperationState::OutcomeUnknown, "{}", r.detail);
    assert_eq!(control.lock().unwrap().writes, 2);
    c.control_resource(r.id, OperationControl::Recheck).unwrap();
    let r = settled(&c, r.id, r.revision()).await;
    assert_eq!(r.state, OperationState::PartiallyCompleted, "{}", r.detail);
    assert_eq!(r.management_ipv6_bond.as_ref().unwrap().completed_stages, 2);
    assert_eq!(control.lock().unwrap().writes, 2);
    drop(second);
    c.close().await;
}
#[tokio::test]
async fn ipv6_completion_proof_survives_task_removal_and_restart() {
    let (_mock, bridge, control) = fixture(true, Fault::None).await;
    install(&mut control.lock().unwrap(), false);
    let dir = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&bridge, &b).await;
    let mut op = operation(&c, false);
    control.lock().unwrap().fault = Fault::TopologyReadAfterCompletion;
    let r = finish(&submit(&c, op.clone())).await;
    assert_eq!(r.state, OperationState::OutcomeUnknown, "{}", r.detail);
    c.close().await;
    drop(c);
    drop(b);
    {
        let mut g = control.lock().unwrap();
        g.fault = Fault::None;
        g.rows.remove(&reference("task", 51));
    }
    let b = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&bridge, &b).await;
    let recovered = tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            if let Some(v) = c
                .resource_history()
                .iter()
                .find(|v| v.id == r.id && v.state == OperationState::PartiallyCompleted)
            {
                break v.clone();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        recovered.state,
        OperationState::PartiallyCompleted,
        "{}",
        recovered.detail
    );
    assert_eq!(control.lock().unwrap().writes, 1);
    op.predecessor = Some(r.id);
    stage(&c, &mut op, 1).await;
    assert_eq!(control.lock().unwrap().writes, 2);
    c.close().await;
}
#[tokio::test]
async fn ipv6_vlan_over_bond_round_trips_preserve_reviewed_guest_identity_and_new_vlan_provenance()
{
    for destroy in [false, true] {
        let (_mock, bridge, control) = fixture(true, Fault::None).await;
        {
            let mut g = control.lock().unwrap();
            install(&mut g, false);
            vlan_bond_changes::install(&mut g, false);
            for n in [13, 14, 91] {
                let p = g.rows.get_mut(&reference("PIF", n)).unwrap();
                p["ipv6_configuration_mode"] = json!("None");
                p["IPv6"] = json!([""]);
            }
            if destroy {
                bond_changes::change(
                    &mut g,
                    &json!({"method":"Async.Bond.create","params":["session",reference("network",500),[reference("PIF",13),reference("PIF",14)],"","active-backup",{}]}),
                );
            }
            for n in [90, 91] {
                vif_bond_changes::guest(&mut g, 700 + n, n, Some(2), None, "Running");
            }
            for (r, v) in &mut g.rows {
                if r.starts_with("OpaqueRef:network:") {
                    v["default_locking_mode"] = json!("unlocked");
                    v["other_config"] = json!({});
                }
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&bridge, &b).await;
        let source_id = id(if destroy { 190 } else { 90 }).into();
        let view = c.latest();
        let i = view.inventory().unwrap();
        let mut op = ManagementIpv6Bond {
            purpose: Default::default(),
            source: ManagementInterface::capture(i, source_id).unwrap(),
            temporary: ManagementInterface::capture(i, id(18).into()).unwrap(),
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
            consumers: None,
            predecessor: None,
        };
        let q = c
            .review_management_ipv6_bond(
                view.observe_resource(ResourceTarget::Host(id(2).into()))
                    .unwrap(),
                op.clone(),
            )
            .unwrap();
        op = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(v) = q.latest() {
                    break v.unwrap();
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(op.consumers.as_ref().unwrap().interfaces.len(), 2);
        for n in 0..5 {
            let r = stage(&c, &mut op, n).await;
            if n >= 2 {
                let e = r.management_ipv6_bond.as_ref().unwrap();
                assert_eq!(e.moved_vlans.len(), 2);
                assert!(
                    e.moved_vlans
                        .iter()
                        .all(|v| v.new_interface != v.old_interface && v.new_vlan != v.old_vlan)
                );
            }
        }
        assert_eq!(
            control.lock().unwrap().rows[&reference("PIF", if destroy { 291 } else { 191 })]["currently_attached"],
            false
        );
        assert_eq!(control.lock().unwrap().writes, 5);
        c.close().await;
    }
}
#[tokio::test]
async fn leaving_verified_partial_ipv6_configuration_releases_only_the_exact_staged_guard_without_host_calls()
 {
    for prefix in [1, 3, 4] {
        let (_mock, bridge, control) = fixture(true, Fault::None).await;
        install(&mut control.lock().unwrap(), false);
        let dir = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&bridge, &b).await;
        let mut op = operation(&c, false);
        for n in 0..prefix {
            stage(&c, &mut op, n).await;
        }
        op.purpose = xenadmin_core::ManagementIpv6Next::LeavePartial;
        let command = || ResourceCommand::ManagementIpv6Bond {
            host: c
                .latest()
                .observe_resource(ResourceTarget::Host(id(2).into()))
                .unwrap(),
            operation: Box::new(op.clone()),
        };
        let request_id = OperationId::new();
        let first = c.submit_resource(request_id, command()).unwrap();
        let r = finish(&first).await;
        assert_eq!(
            r.state,
            OperationState::PartiallyCompleted,
            "{} {:?}",
            r.detail,
            r.local_error
        );
        let e = r.management_ipv6_bond.as_ref().unwrap();
        assert!(e.left_partial && !e.retains_reservation);
        assert_eq!(e.completed_stages, prefix);
        assert_eq!(control.lock().unwrap().writes, prefix);
        let duplicate = c.submit_resource(request_id, command()).unwrap();
        assert_eq!(finish(&duplicate).await.id, r.id);
        assert_eq!(control.lock().unwrap().writes, prefix);
        let mut next = op.clone();
        next.predecessor = Some(r.id);
        next.purpose = xenadmin_core::ManagementIpv6Next::Continue;
        assert_eq!(
            finish(&submit(&c, next)).await.state,
            OperationState::Rejected
        );
        let fresh = c
            .submit_resource(
                OperationId::new(),
                ResourceCommand::RenameMetadata {
                    target: c
                        .latest()
                        .observe_resource(ResourceTarget::Host(id(2).into()))
                        .unwrap(),
                    expected_name: "obsolete reviewed name".into(),
                    expected_description: String::new(),
                    name: "changed".into(),
                    description: String::new(),
                },
            )
            .unwrap();
        let result = finish(&fresh).await;
        assert!(
            result.detail != xenadmin_core::Error::ConflictingPoolOperation.to_string(),
            "{}",
            result.detail
        );
        assert_eq!(control.lock().unwrap().writes, prefix);
        c.close().await;
    }
}
#[tokio::test]
async fn leave_partial_cannot_bypass_an_active_or_unknown_child_or_a_stale_predecessor() {
    let (_mock, bridge, control) = fixture(true, Fault::None).await;
    install(&mut control.lock().unwrap(), false);
    let dir = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&bridge, &b).await;
    let mut op = operation(&c, false);
    stage(&c, &mut op, 0).await;
    let mut leave = op.clone();
    leave.purpose = xenadmin_core::ManagementIpv6Next::LeavePartial;
    let (entered, release) = {
        let mut g = control.lock().unwrap();
        g.delay = true;
        g.fault = Fault::LostReply;
        (g.entered.clone(), g.release.clone())
    };
    let child = submit(&c, op.clone());
    tokio::time::timeout(Duration::from_secs(5), entered.notified())
        .await
        .unwrap();
    assert_eq!(
        finish(&submit(&c, leave.clone())).await.state,
        OperationState::Rejected
    );
    release.notify_one();
    let r = finish(&child).await;
    assert_eq!(r.state, OperationState::OutcomeUnknown);
    leave.predecessor = Some(r.id);
    assert_eq!(
        finish(&submit(&c, leave.clone())).await.state,
        OperationState::Rejected
    );
    leave.predecessor = Some(OperationId::new());
    assert_eq!(
        finish(&submit(&c, leave)).await.state,
        OperationState::Rejected
    );
    assert_eq!(control.lock().unwrap().writes, 2);
    c.close().await;
}
