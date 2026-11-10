use super::*;
use crate::{
    NfsVersion, ResourceAction, ReviewedPbd, StepState, StorageGraph, StorageLocation,
    StorageRepairHost,
};
pub(super) fn rpc(
    method: &str,
    p: &Value,
    d: &mut BTreeMap<String, Value>,
) -> Option<super::disk_administration_tests::Reply> {
    let fault = d.get("fixture:graph")?.as_str()?.to_owned();
    let reply = |value, lost, paused| {
        Some(super::disk_administration_tests::Reply {
            value,
            lost,
            paused,
        })
    };
    if method == "secret.get_uuid" {
        return reply(
            json!({"result":d[p[1].as_str().unwrap()]["uuid"]}),
            false,
            false,
        );
    }
    if method == "secret.get_all" {
        return reply(
            json!({"result":d.keys().filter(|k|k.starts_with("OpaqueRef:secret:")).collect::<Vec<_>>()}),
            false,
            false,
        );
    }
    if method == "PBD.get_all_records"
        && d.contains_key("OpaqueRef:PBD:new")
        && fault == "late-verify"
    {
        return reply(
            json!({"result":d.iter().filter(|(k,_)|k.starts_with("OpaqueRef:PBD:")).collect::<BTreeMap<_,_>>()}),
            false,
            true,
        );
    }
    if method == "task.get_record" && fault == "late-task" {
        return reply(json!({"result":d[p[1].as_str().unwrap()]}), false, true);
    }
    if method == "Async.PBD.create" {
        if fault == "refuse-create" {
            return reply(
                json!({"error":{"message":"OPERATION_NOT_ALLOWED"}}),
                false,
                false,
            );
        }
        let mut v = p[1].clone();
        v["uuid"] = json!(uuid(30));
        v["currently_attached"] = json!(false);
        if v["device_config"].get("password_secret").is_some() {
            v["device_config"]["password_secret"] =
                json!(uuid(if fault == "reuse-secret" { 50 } else { 51 }).to_string());
            d.insert("OpaqueRef:secret:new".into(), json!({"uuid":uuid(51)}));
        }
        d.insert("OpaqueRef:PBD:new".into(), v);
        if fault == "secret-consumer" {
            d.get_mut("OpaqueRef:SR:s").unwrap()["sm_config"]["password_secret"] =
                json!(uuid(51).to_string());
        }

        d.get_mut("OpaqueRef:SR:s").unwrap()["PBDs"]
            .as_array_mut()
            .unwrap()
            .push(json!("OpaqueRef:PBD:new"));
        d.get_mut("OpaqueRef:host:j").unwrap()["PBDs"] = json!(["OpaqueRef:PBD:new"]);
        let result = if fault == "existing-result" {
            "OpaqueRef:PBD:p"
        } else {
            "OpaqueRef:PBD:new"
        };
        d.insert(
            "OpaqueRef:task:g0".into(),
            json!({"uuid":uuid(200),"status":"success","result":xml(&json!(result))}),
        );
        return reply(
            json!({"result":"OpaqueRef:task:g0"}),
            fault == "lost-create",
            fault == "late-create",
        );
    }
    if matches!(method, "Async.PBD.plug" | "Async.PBD.unplug") {
        if (fault == "refuse-plug" && method == "Async.PBD.plug")
            || (fault == "refuse-unplug" && method == "Async.PBD.unplug")
        {
            return reply(
                json!({"error":{"message":"OPERATION_NOT_ALLOWED"}}),
                false,
                false,
            );
        }
        let key = p[1].as_str().unwrap();
        let plug = method.ends_with(".plug");
        d.get_mut(key).unwrap()["currently_attached"] = json!(plug);
        // Like the iSCSI driver, persist the discovered portal list on a plug.
        let sr_ref = d[key]["SR"].as_str().unwrap().to_owned();
        if plug && d[&sr_ref]["type"] == "lvmoiscsi" {
            d.get_mut(key).unwrap()["device_config"]["multihomelist"] =
                json!("192.0.2.10:3260,192.0.2.11:3260");
        }
        let n = d
            .keys()
            .filter(|k| k.starts_with("OpaqueRef:task:g"))
            .count();
        let task = format!("OpaqueRef:task:g{n}");
        d.insert(task.clone(),json!({"uuid":uuid(if fault=="reuse-task"{200}else{200+n as u128}),"status":if fault=="task-failure"{"failure"}else{"success"},"result":xml(&json!(""))}));
        return reply(
            json!({"result":task}),
            fault == "lost-attach",
            fault == "late-attach",
        );
    }
    if method == "host.set_multipathing" {
        if fault == "refuse-mode" {
            return reply(
                json!({"error":{"message":"OPERATION_NOT_ALLOWED"}}),
                false,
                false,
            );
        }
        d.get_mut(p[1].as_str().unwrap()).unwrap()["multipathing"] = p[2].clone();
        return reply(
            json!({"result":null}),
            fault == "lost-mode",
            fault == "late-mode",
        );
    }
    if matches!(
        method,
        "host.add_to_other_config" | "host.remove_from_other_config"
    ) {
        assert_eq!(p[2], "multipathhandle");
        let m = d.get_mut(p[1].as_str().unwrap()).unwrap()["other_config"]
            .as_object_mut()
            .unwrap();
        if method == "host.add_to_other_config" {
            m.insert("multipathhandle".into(), p[3].clone());
        } else {
            m.remove("multipathhandle");
        }
        return reply(json!({"result":null}), false, false);
    }
    None
}
async fn fixture(fault: &str, auth: bool) -> Fixture {
    let f = Fixture::new(Fault::None).await;
    {
        let mut d = f.records.lock().unwrap();
        d.insert("fixture:graph".into(), json!(fault));
        d.get_mut("OpaqueRef:pool:p").unwrap()["ha_enabled"] = json!(false);
        // A pool whose HA was disabled keeps its statefile listed (XCP-ng 8.3).
        d.get_mut("OpaqueRef:pool:p").unwrap()["ha_statefiles"] =
            json!(["OpaqueRef:VDI:statefile"]);
        d.get_mut("OpaqueRef:pool:p").unwrap()["current_operations"] = json!({});
        for (key, id, pbds) in [
            ("OpaqueRef:host:h", 3, json!(["OpaqueRef:PBD:p"])),
            ("OpaqueRef:host:j", 6, json!([])),
        ] {
            d.insert(key.into(),json!({"uuid":uuid(id),"metrics":"OpaqueRef:host_metrics:graph","current_operations":{},"enabled":false,"multipathing":false,"other_config":{"MAINTENANCE_MODE":"true","keep":"value"},"PBDs":pbds,"local_cache_sr":"OpaqueRef:NULL","power_on_config":{}}));
        }
        d.insert(
            "OpaqueRef:host_metrics:graph".into(),
            json!({"uuid":uuid(99),"live":true}),
        );
        let sr = d.get_mut("OpaqueRef:SR:s").unwrap();
        sr["shared"] = json!(true);
        sr["type"] = json!("nfs");
        sr["allowed_operations"] = json!(["plug", "unplug", "pbd_create"]);
        sr["local_cache_enabled"] = json!(false);
        sr["sm_config"] = json!({});
        sr["PBDs"] = json!(["OpaqueRef:PBD:p"]);
        sr["VDIs"] = json!([]);
        let v = d.get_mut("OpaqueRef:PBD:p").unwrap();
        v["currently_attached"] = json!(true);
        v["device_config"] = json!({"server":"storage.example.test","serverpath":"/data"});
        if auth {
            v["device_config"]["password_secret"] = json!(uuid(50).to_string());
            d.insert("OpaqueRef:secret:original".into(), json!({"uuid":uuid(50)}));
        }
    }
    f
}
fn attachment(auth: bool) -> ReviewedPbd {
    ReviewedPbd {
        pbd: uuid(5).into(),
        host: uuid(3).into(),
        sr: uuid(4).into(),
        attached: true,
        location: Some(StorageLocation::Nfs {
            server: "storage.example.test".into(),
            path: "/data".into(),
            version: NfsVersion::Automatic,
        }),
        requires_credentials: auth,
        local_cache: false,
    }
}
fn repair(auth: bool) -> StorageGraph {
    StorageGraph::Repair {
        coordinator: uuid(3).into(),
        source: attachment(auth),
        hosts: vec![StorageRepairHost {
            host: uuid(6).into(),
            attachment: None,
        }],
    }
}
fn multipath() -> StorageGraph {
    StorageGraph::Multipath {
        host: uuid(3).into(),
        expected: false,
        expected_handler: false,
        enabled: true,
        attachments: vec![attachment(false)],
    }
}
fn pending(op: StorageGraph) -> (crate::ResourceOperation, ResourceRequest) {
    let target = match &op {
        StorageGraph::Repair { source, .. } => ResourceTarget::Storage(source.sr),
        StorageGraph::Multipath { host, .. } => ResourceTarget::Host(*host),
    };
    resources::channel(
        OperationId::new(),
        ObservedResource {
            pool: uuid(1).into(),
            generation: 1,
            target,
        },
        ResourceAction::StorageGraph(op),
        tokio::runtime::Handle::current(),
    )
}
async fn run(f: &Fixture, dir: &Path, r: ResourceRequest) {
    let (tx, _rx) = mpsc::channel(512);
    tokio::time::timeout(
        std::time::Duration::from_secs(30),
        r.run(
            Arc::new(f.session().await),
            dir.into(),
            CancellationToken::new(),
            tx,
        ),
    )
    .await
    .unwrap();
}
async fn execute(f: &Fixture, dir: &Path, op: StorageGraph) -> crate::ResourceOperationRecord {
    let (o, r) = pending(op);
    run(f, dir, r).await;
    (*o.latest()).clone()
}
#[tokio::test]
async fn creates_missing_attachment_and_proves_fresh_cloned_credentials_without_values() {
    for auth in [false, true] {
        let f = fixture("", auth).await;
        let dir = tempfile::tempdir().unwrap();
        let r = execute(&f, dir.path(), repair(auth)).await;
        assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
        assert_eq!(r.storage_graph.len(), 1);
        assert!(r.storage_graph[0].created && r.storage_graph[0].attached);
        assert_eq!(f.count("Async.PBD.create"), 1);
        assert_eq!(f.count("Async.PBD.plug"), 1);
        assert_eq!(
            f.count("secret.get_value")
                + f.count("secret.get_record")
                + f.count("secret.get_all_records"),
            0
        );
        let public = serde_json::to_string(&r).unwrap();
        assert!(
            !public.contains("OpaqueRef:")
                && !public.contains("password_secret")
                && !public.contains("configuration")
                && !public.contains("secrets_before")
        );
    }
}
#[tokio::test]
async fn missing_repair_refuses_custom_config_duplicate_pair_and_changed_source() {
    for fault in [
        "custom",
        "duplicate",
        "source-host",
        "source-location",
        "coordinator",
        "unshared",
        "capability",
    ] {
        let f = fixture("", false).await;
        {
            let mut d = f.records.lock().unwrap();
            match fault {
                "custom" => {
                    d.get_mut("OpaqueRef:PBD:p").unwrap()["device_config"]["host-specific"] =
                        json!("no-copy")
                }
                "duplicate" => {
                    let mut p = d["OpaqueRef:PBD:p"].clone();
                    p["host"] = json!("OpaqueRef:host:j");
                    p["uuid"] = json!(uuid(31));
                    d.insert("OpaqueRef:PBD:q".into(), p);
                }
                "source-host" => {
                    d.get_mut("OpaqueRef:PBD:p").unwrap()["host"] = json!("OpaqueRef:host:j")
                }
                "source-location" => {
                    d.get_mut("OpaqueRef:PBD:p").unwrap()["device_config"]["serverpath"] =
                        json!("/other")
                }
                "coordinator" => {
                    d.get_mut("OpaqueRef:pool:p").unwrap()["master"] = json!("OpaqueRef:host:j")
                }
                "unshared" => d.get_mut("OpaqueRef:SR:s").unwrap()["shared"] = json!(false),
                _ => d.get_mut("OpaqueRef:SR:s").unwrap()["allowed_operations"] = json!([]),
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let r = execute(&f, dir.path(), repair(false)).await;
        assert_eq!(r.state, OperationState::Rejected, "{fault}: {}", r.detail);
        assert_eq!(f.count("Async.PBD.create"), 0);
    }
}
#[tokio::test]
async fn result_or_secret_rebinding_never_plugs_unproven_attachment() {
    for fault in ["existing-result", "reuse-secret", "secret-consumer"] {
        let f = fixture(fault, true).await;
        let dir = tempfile::tempdir().unwrap();
        let r = execute(&f, dir.path(), repair(true)).await;
        assert_eq!(
            r.state,
            OperationState::OutcomeUnknown,
            "{fault}: {}",
            r.detail
        );
        assert_eq!(f.count("Async.PBD.plug"), 0);
        if fault == "existing-result" {
            assert!(r.storage_graph.is_empty());
        } else {
            assert_eq!(r.storage_graph[0].pbd, uuid(30).into());
            assert!(!r.storage_graph[0].verified);
        }
    }
}
#[tokio::test]
async fn known_later_refusal_retains_created_connection_for_separate_repair() {
    let f = fixture("refuse-plug", false).await;
    let dir = tempfile::tempdir().unwrap();
    let r = execute(&f, dir.path(), repair(false)).await;
    assert_eq!(r.state, OperationState::PartiallyCompleted, "{}", r.detail);
    assert!(r.storage_graph[0].created && !r.storage_graph[0].attached);
    assert_eq!(f.count("PBD.destroy"), 0);
}
#[tokio::test]
async fn lost_create_or_attach_never_infers_success_or_replays() {
    for fault in ["lost-create", "lost-attach", "task-failure", "reuse-task"] {
        let f = fixture(fault, false).await;
        let dir = tempfile::tempdir().unwrap();
        let r = execute(&f, dir.path(), repair(false)).await;
        assert_eq!(
            r.state,
            OperationState::OutcomeUnknown,
            "{fault}: {}",
            r.detail
        );
        let calls = (f.count("Async.PBD.create"), f.count("Async.PBD.plug"));
        let r = recheck(&f, dir.path(), r).await;
        assert_eq!(
            r.state,
            OperationState::OutcomeUnknown,
            "{fault}: {}",
            r.detail
        );
        assert_eq!(
            calls,
            (f.count("Async.PBD.create"), f.count("Async.PBD.plug"))
        );
    }
}
#[tokio::test]
async fn multipath_detaches_sets_policy_and_replugs_only_original_subset() {
    let f = fixture("", false).await;
    let mut op = multipath();
    {
        let mut d = f.records.lock().unwrap();
        let mut sr = d["OpaqueRef:SR:s"].clone();
        sr["uuid"] = json!(uuid(7));
        sr["PBDs"] = json!(["OpaqueRef:PBD:q"]);
        d.insert("OpaqueRef:SR:q".into(), sr);
        let mut p = d["OpaqueRef:PBD:p"].clone();
        p["uuid"] = json!(uuid(8));
        p["SR"] = json!("OpaqueRef:SR:q");
        p["currently_attached"] = json!(false);
        d.insert("OpaqueRef:PBD:q".into(), p);
        d.get_mut("OpaqueRef:host:h").unwrap()["PBDs"] =
            json!(["OpaqueRef:PBD:p", "OpaqueRef:PBD:q"]);
        if let StorageGraph::Multipath { attachments, .. } = &mut op {
            let mut p = attachment(false);
            p.pbd = uuid(8).into();
            p.sr = uuid(7).into();
            p.attached = false;
            attachments.push(p)
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let r = execute(&f, dir.path(), op).await;
    assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
    assert_eq!(f.count("Async.PBD.unplug"), 1);
    assert_eq!(f.count("Async.PBD.plug"), 1);
    let d = f.records.lock().unwrap();
    assert_eq!(d["OpaqueRef:PBD:p"]["currently_attached"], true);
    assert_eq!(d["OpaqueRef:PBD:q"]["currently_attached"], false);
    assert_eq!(
        d["OpaqueRef:host:h"]["other_config"]["multipathhandle"],
        "dmp"
    );
    assert_eq!(d["OpaqueRef:host:h"]["other_config"]["keep"], "value");
}
#[tokio::test]
async fn multipath_refuses_ha_resident_guests_changed_graph_and_shared_active_io() {
    for fault in [
        "ha",
        "enabled",
        "maintenance",
        "guest",
        "graph",
        "cache",
        "busy-shared",
        "handler",
    ] {
        let f = fixture("", false).await;
        let mut op = multipath();
        {
            let mut d = f.records.lock().unwrap();
            match fault {
                "ha" => d.get_mut("OpaqueRef:pool:p").unwrap()["ha_enabled"] = json!(true),
                "enabled" => d.get_mut("OpaqueRef:host:h").unwrap()["enabled"] = json!(true),
                "maintenance" => {
                    d.get_mut("OpaqueRef:host:h").unwrap()["other_config"]["MAINTENANCE_MODE"] =
                        json!("false")
                }
                "guest" => {
                    d.get_mut("OpaqueRef:VM:source").unwrap()["resident_on"] =
                        json!("OpaqueRef:host:h")
                }
                "cache" => {
                    d.get_mut("OpaqueRef:SR:s").unwrap()["local_cache_enabled"] = json!(true)
                }
                "handler" => {
                    d.get_mut("OpaqueRef:host:h").unwrap()["other_config"]["multipathhandle"] =
                        json!("custom")
                }
                "busy-shared" => {
                    d.get_mut("OpaqueRef:SR:s").unwrap()["type"] = json!("lvmohba");
                    let p = d.get_mut("OpaqueRef:PBD:p").unwrap();
                    p["device_config"] = json!({"SCSIid":"3600fixture"});
                    if let StorageGraph::Multipath { attachments, .. } = &mut op {
                        attachments[0].location = Some(StorageLocation::Hba {
                            scsi_id: "3600fixture".into(),
                            allocation: crate::StorageAllocation::DriverDefault,
                        })
                    }
                    d.insert("OpaqueRef:VDI:busy".into(),json!({"uuid":uuid(80),"SR":"OpaqueRef:SR:s","type":"user","current_operations":{},"VBDs":["OpaqueRef:VBD:busy"]}));
                    d.insert("OpaqueRef:VBD:busy".into(),json!({"uuid":uuid(81),"VDI":"OpaqueRef:VDI:busy","VM":"OpaqueRef:VM:source","currently_attached":true,"reserved":false}));
                    d.get_mut("OpaqueRef:SR:s").unwrap()["VDIs"] = json!(["OpaqueRef:VDI:busy"]);
                    d.get_mut("OpaqueRef:VM:source").unwrap()["VBDs"] =
                        json!(["OpaqueRef:VBD:busy"]);
                }
                _ => d.get_mut("OpaqueRef:host:h").unwrap()["PBDs"] = json!([]),
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let r = execute(&f, dir.path(), op).await;
        assert_eq!(r.state, OperationState::Rejected, "{fault}: {}", r.detail);
        assert_eq!(f.count("Async.PBD.unplug"), 0);
        assert_eq!(f.count("host.set_multipathing"), 0);
    }
}
#[tokio::test]
async fn multipath_refusal_or_loss_does_not_automatically_reconnect() {
    for fault in ["refuse-mode", "lost-mode"] {
        let f = fixture(fault, false).await;
        let dir = tempfile::tempdir().unwrap();
        let r = execute(&f, dir.path(), multipath()).await;
        assert_eq!(
            r.state,
            if fault == "lost-mode" {
                OperationState::OutcomeUnknown
            } else {
                OperationState::PartiallyCompleted
            },
            "{}",
            r.detail
        );
        assert_eq!(r.steps[0].state, StepState::Verified);
        assert_eq!(f.count("Async.PBD.plug"), 0);
        if fault == "lost-mode" {
            let r = recheck(&f, dir.path(), r).await;
            assert_eq!(r.state, OperationState::OutcomeUnknown);
            assert_eq!(f.count("host.set_multipathing"), 1);
        }
    }
}
#[tokio::test]
async fn late_receipts_preserve_recovery_ack_and_never_continue() {
    for fault in ["late-create", "late-task", "late-mode", "late-verify"] {
        for ack in [false, true] {
            let f = fixture(fault, false).await;
            let dir = tempfile::tempdir().unwrap();
            let (o, r) = pending(if fault == "late-mode" {
                multipath()
            } else {
                repair(false)
            });
            let execute = run(&f, dir.path(), r);
            let recover = async {
                f.response_gate.wait().await;
                let j = Journal::open(dir.path()).unwrap();
                let rows = j.resource_recover(uuid(1).into()).unwrap();
                if ack {
                    j.resource_control(
                        uuid(1).into(),
                        rows[0].id,
                        crate::OperationControl::AcknowledgeUnknown,
                    )
                    .unwrap();
                }
                f.response_gate.resume();
            };
            tokio::join!(execute, recover);
            let r = o.latest();
            assert_eq!(
                r.state,
                if ack {
                    OperationState::AcknowledgedUnknown
                } else {
                    OperationState::OutcomeUnknown
                },
                "{}",
                r.detail
            );
            assert_eq!(f.count("Async.PBD.plug"), 0);
            if fault == "late-mode" {
                assert!(r.storage_graph_plan.as_ref().unwrap().evidence[1].completed)
            } else if fault == "late-verify" {
                assert_eq!(r.storage_graph[0].pbd, uuid(30).into());
                assert!(r.storage_graph[0].created);
            } else {
                assert!(
                    r.storage_graph_plan.as_ref().unwrap().evidence[0]
                        .task
                        .is_some()
                )
            }
        }
    }
}
#[tokio::test]
async fn private_plan_rejects_rebound_shape_and_impossible_completion() {
    let f = fixture("", false).await;
    let dir = tempfile::tempdir().unwrap();
    let r = execute(&f, dir.path(), repair(false)).await;
    assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
    let j = Journal::open(dir.path()).unwrap();
    for mode in 0..5 {
        let mut bad = r.clone();
        match mode {
            0 => bad.storage_graph_plan.as_mut().unwrap().pool = uuid(999).into(),
            1 => bad.steps.clear(),
            2 => bad.storage_graph_plan.as_mut().unwrap().evidence[0].completed = false,
            3 => bad.storage_graph.clear(),
            _ => bad.storage_graph_plan = None,
        }
        assert!(j.resource_save(&mut bad).is_err());
        bad.id = OperationId::new();
        assert!(j.resource_admit(&mut bad).is_err());
    }
}
#[tokio::test]
async fn repair_reuses_existing_connection_and_runs_selected_coordinator_first() {
    let f = fixture("", false).await;
    {
        let mut d = f.records.lock().unwrap();
        d.get_mut("OpaqueRef:PBD:p").unwrap()["currently_attached"] = json!(false);
        let mut p = d["OpaqueRef:PBD:p"].clone();
        p["uuid"] = json!(uuid(7));
        p["host"] = json!("OpaqueRef:host:j");
        d.insert("OpaqueRef:PBD:q".into(), p);
        d.get_mut("OpaqueRef:SR:s").unwrap()["PBDs"] =
            json!(["OpaqueRef:PBD:p", "OpaqueRef:PBD:q"]);
    }
    let mut source = attachment(false);
    source.attached = false;
    let mut second = source.clone();
    second.pbd = uuid(7).into();
    second.host = uuid(6).into();
    let op = StorageGraph::Repair {
        coordinator: source.host,
        source: source.clone(),
        hosts: vec![
            StorageRepairHost {
                host: second.host,
                attachment: Some(second),
            },
            StorageRepairHost {
                host: source.host,
                attachment: Some(source),
            },
        ],
    };
    let dir = tempfile::tempdir().unwrap();
    let r = execute(&f, dir.path(), op).await;
    assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
    assert!(r.steps[0].label.contains(&uuid(3).to_string()));
    assert_eq!(f.count("Async.PBD.create"), 0);
    assert_eq!(f.count("Async.PBD.plug"), 2);
    assert!(r.storage_graph.iter().all(|p| !p.created && p.attached));
}
#[tokio::test]
async fn multipath_disabling_removes_only_standard_handler_and_reconnects() {
    let f = fixture("", false).await;
    {
        let mut d = f.records.lock().unwrap();
        let h = d.get_mut("OpaqueRef:host:h").unwrap();
        h["multipathing"] = json!(true);
        h["other_config"]["multipathhandle"] = json!("dmp");
    }
    let mut op = multipath();
    if let StorageGraph::Multipath {
        expected,
        expected_handler,
        enabled,
        ..
    } = &mut op
    {
        *expected = true;
        *expected_handler = true;
        *enabled = false;
    }
    let dir = tempfile::tempdir().unwrap();
    let r = execute(&f, dir.path(), op).await;
    assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
    let d = f.records.lock().unwrap();
    assert!(
        d["OpaqueRef:host:h"]["other_config"]
            .get("multipathhandle")
            .is_none()
    );
    assert_eq!(d["OpaqueRef:host:h"]["other_config"]["keep"], "value");
    assert_eq!(d["OpaqueRef:PBD:p"]["currently_attached"], true);
}
#[tokio::test]
async fn incomplete_disk_graph_refuses_before_any_detachment() {
    for fault in [
        "missing-vdi",
        "missing-vbd",
        "vbd-backref",
        "vm-backref",
        "duplicate-vdi",
    ] {
        let f = fixture("", false).await;
        {
            let mut d = f.records.lock().unwrap();
            d.get_mut("OpaqueRef:SR:s").unwrap()["VDIs"] = json!(["OpaqueRef:VDI:graph"]);
            if fault != "missing-vdi" {
                d.insert("OpaqueRef:VDI:graph".into(),json!({"uuid":uuid(82),"SR":"OpaqueRef:SR:s","type":"user","current_operations":{},"VBDs":["OpaqueRef:VBD:graph"]}));
            }
            if fault != "missing-vbd" {
                d.insert("OpaqueRef:VBD:graph".into(),json!({"uuid":uuid(83),"VDI":if fault=="vbd-backref"{"OpaqueRef:VDI:else"}else{"OpaqueRef:VDI:graph"},"VM":"OpaqueRef:VM:source","currently_attached":false,"reserved":false}));
            }
            d.get_mut("OpaqueRef:VM:source").unwrap()["VBDs"] = if fault == "vm-backref" {
                json!([])
            } else {
                json!(["OpaqueRef:VBD:graph"])
            };
            if fault == "duplicate-vdi" {
                d.get_mut("OpaqueRef:SR:s").unwrap()["VDIs"] =
                    json!(["OpaqueRef:VDI:graph", "OpaqueRef:VDI:graph"]);
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let r = execute(&f, dir.path(), multipath()).await;
        assert_eq!(r.state, OperationState::Rejected, "{fault}: {}", r.detail);
        assert_eq!(f.count("Async.PBD.unplug"), 0);
        assert_eq!(f.count("host.set_multipathing"), 0);
    }
}
#[tokio::test]
async fn repository_detach_refusals_name_the_repository_reason() {
    for (case, expected) in [
        (
            "busy",
            "The repository has running operations, or their availability is unknown",
        ),
        (
            "unadvertised",
            "The host no longer advertises the reviewed storage operation",
        ),
    ] {
        let f = fixture("", false).await;
        {
            let mut d = f.records.lock().unwrap();
            let sr = d.get_mut("OpaqueRef:SR:s").unwrap();
            if case == "busy" {
                sr["current_operations"] = json!({"OpaqueRef:task:scan": "scan"});
            } else {
                sr["allowed_operations"] = json!(["plug"]);
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let (o, r) = resources::channel(
            OperationId::new(),
            ObservedResource {
                pool: uuid(1).into(),
                generation: 1,
                target: ResourceTarget::Pbd(uuid(5).into()),
            },
            ResourceAction::Storage(crate::StorageOperation::SetAttached {
                sr: uuid(4).into(),
                host: uuid(3).into(),
                expected_attached: true,
                attached: false,
            }),
            tokio::runtime::Handle::current(),
        );
        run(&f, dir.path(), r).await;
        let r = (*o.latest()).clone();
        assert_eq!(r.state, OperationState::Rejected, "{case}: {}", r.detail);
        assert_eq!(r.detail, expected, "{case}");
        assert_eq!(f.count("Async.PBD.unplug"), 0);
    }
}
#[tokio::test]
async fn multipath_refuses_enabled_ha_or_a_pool_operation_but_not_a_leftover_statefile() {
    for (case, expected) in [
        (
            "enabled",
            "Disable HA separately before multipath maintenance",
        ),
        (
            "busy",
            "Wait for the pool's active operation before multipath maintenance",
        ),
    ] {
        let f = fixture("", false).await;
        {
            let mut d = f.records.lock().unwrap();
            let pool = d.get_mut("OpaqueRef:pool:p").unwrap();
            if case == "enabled" {
                pool["ha_enabled"] = json!(true);
            } else {
                pool["current_operations"] = json!({"OpaqueRef:task:ha": "ha_enable"});
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let r = execute(&f, dir.path(), multipath()).await;
        assert_eq!(r.state, OperationState::Rejected, "{case}: {}", r.detail);
        assert_eq!(r.detail, expected, "{case}");
        assert_eq!(f.count("Async.PBD.unplug"), 0);
    }
}
#[tokio::test]
async fn multipath_keeps_the_reviewed_local_cache_attachment_attached() {
    let f = fixture("", false).await;
    let mut op = multipath();
    {
        let mut d = f.records.lock().unwrap();
        // XCP-ng's thin-provisioned local storage is the host's local-cache repository.
        d.get_mut("OpaqueRef:host:h").unwrap()["local_cache_sr"] = json!("OpaqueRef:SR:s");
        d.get_mut("OpaqueRef:SR:s").unwrap()["local_cache_enabled"] = json!(true);
        let mut sr = d["OpaqueRef:SR:s"].clone();
        sr["uuid"] = json!(uuid(7));
        sr["PBDs"] = json!(["OpaqueRef:PBD:q"]);
        sr["local_cache_enabled"] = json!(false);
        d.insert("OpaqueRef:SR:q".into(), sr);
        let mut p = d["OpaqueRef:PBD:p"].clone();
        p["uuid"] = json!(uuid(8));
        p["SR"] = json!("OpaqueRef:SR:q");
        d.insert("OpaqueRef:PBD:q".into(), p);
        d.get_mut("OpaqueRef:host:h").unwrap()["PBDs"] =
            json!(["OpaqueRef:PBD:p", "OpaqueRef:PBD:q"]);
        if let StorageGraph::Multipath { attachments, .. } = &mut op {
            attachments[0].local_cache = true;
            let mut p = attachment(false);
            p.pbd = uuid(8).into();
            p.sr = uuid(7).into();
            attachments.push(p)
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let r = execute(&f, dir.path(), op).await;
    assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
    assert_eq!(f.count("Async.PBD.unplug"), 1);
    assert_eq!(f.count("Async.PBD.plug"), 1);
    assert_eq!(f.count("host.set_multipathing"), 1);
    assert_eq!(r.storage_graph.len(), 1);
    assert_eq!(r.storage_graph[0].sr, uuid(7).into());
    let d = f.records.lock().unwrap();
    assert_eq!(d["OpaqueRef:PBD:p"]["currently_attached"], true);
    assert_eq!(d["OpaqueRef:PBD:q"]["currently_attached"], true);
    assert_eq!(d["OpaqueRef:host:h"]["local_cache_sr"], "OpaqueRef:SR:s");
}
#[tokio::test]
async fn multipath_tolerates_the_iscsi_driver_persisting_its_portal_list() {
    let f = fixture("", false).await;
    let mut op = multipath();
    {
        let mut d = f.records.lock().unwrap();
        let sr = d.get_mut("OpaqueRef:SR:s").unwrap();
        sr["type"] = json!("lvmoiscsi");
        let p = d.get_mut("OpaqueRef:PBD:p").unwrap();
        p["device_config"] = json!({"target":"192.0.2.10","port":"3260","targetIQN":"iqn.2026-11.test:fixture","SCSIid":"3600fixture"});
        if let StorageGraph::Multipath { attachments, .. } = &mut op {
            attachments[0].location = Some(StorageLocation::Iscsi {
                portal: "192.0.2.10".into(),
                port: 3260,
                target_iqn: "iqn.2026-11.test:fixture".into(),
                scsi_id: "3600fixture".into(),
                allocation: crate::StorageAllocation::DriverDefault,
            });
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let r = execute(&f, dir.path(), op).await;
    assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
    assert_eq!(f.count("Async.PBD.unplug"), 1);
    assert_eq!(f.count("Async.PBD.plug"), 1);
    assert_eq!(r.storage_graph.len(), 1);
    assert!(r.storage_graph[0].attached && r.storage_graph[0].verified);
    let d = f.records.lock().unwrap();
    assert_eq!(
        d["OpaqueRef:PBD:p"]["device_config"]["multihomelist"],
        "192.0.2.10:3260,192.0.2.11:3260"
    );
}
