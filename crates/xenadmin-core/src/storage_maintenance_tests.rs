use super::*;
use crate::{
    NfsVersion, ResourceAction, ReviewedStorageAttachment, StepState, StorageLocation,
    StorageMaintenance,
};
pub(super) fn rpc(
    method: &str,
    p: &Value,
    data: &mut BTreeMap<String, Value>,
) -> Option<super::disk_administration_tests::Reply> {
    let fault = data.get("fixture:maintenance")?.as_str()?.to_owned();
    let reply = |value, lost, paused| {
        Some(super::disk_administration_tests::Reply {
            value,
            lost,
            paused,
        })
    };
    if method == "Async.PBD.plug" {
        let reference = p[1].as_str().unwrap();
        let n = if reference == "OpaqueRef:PBD:p" { 0 } else { 1 };
        if n == 1 && fault == "refuse-second" {
            return reply(
                json!({"error":{"message":"OPERATION_NOT_ALLOWED"}}),
                false,
                false,
            );
        }
        if n == 1 {
            assert_eq!(
                data["OpaqueRef:PBD:p"]["currently_attached"], true,
                "coordinator first"
            );
        }
        data.get_mut(reference).unwrap()["currently_attached"] = json!(true);
        if fault == "change-second" {
            data.get_mut("OpaqueRef:PBD:q").unwrap()["device_config"]["serverpath"] =
                json!("/changed");
        }
        let task = if n == 1 && fault == "reuse-task" {
            "OpaqueRef:task:m0".to_owned()
        } else {
            format!("OpaqueRef:task:m{n}")
        };
        data.insert(task.clone(),json!({"uuid":uuid(if fault=="reuse-task"{200}else{200+n}),"status":if fault=="task-failure"{"failure"}else{"success"},"progress":1.0,"result":xml(&json!(""))}));
        return reply(
            json!({"result":task}),
            fault == "lost",
            fault == "late-reply",
        );
    }
    if matches!(
        method,
        "SR.remove_from_other_config" | "SR.add_to_other_config"
    ) {
        assert_eq!(p[2], "o_direct");
        if fault == "refuse-add" && method == "SR.add_to_other_config" {
            return reply(
                json!({"error":{"message":"OPERATION_NOT_ALLOWED"}}),
                false,
                false,
            );
        }
        let config = data.get_mut("OpaqueRef:SR:s").unwrap()["other_config"]
            .as_object_mut()
            .unwrap();
        if method == "SR.remove_from_other_config" {
            config.remove("o_direct");
        } else {
            assert_eq!(p[3], "true");
            config.insert("o_direct".into(), p[3].clone());
        }
        return reply(
            json!({"result":null}),
            fault == "lost",
            fault == "late-reply",
        );
    }
    if method == "task.get_record" && fault == "late-task" {
        return reply(json!({"result":data[p[1].as_str().unwrap()]}), false, true);
    }
    None
}
async fn fixture(fault: &str) -> Fixture {
    let f = Fixture::new(Fault::None).await;
    {
        let mut d = f.records.lock().unwrap();
        d.insert("fixture:maintenance".into(), json!(fault));
        d.get_mut("OpaqueRef:SR:s").unwrap()["type"] = json!("nfs");
        d.get_mut("OpaqueRef:SR:s").unwrap()["allowed_operations"] = json!(["plug"]);
        d.get_mut("OpaqueRef:SR:s").unwrap()["other_config"] = json!({"unrelated":"preserve"});
        d.get_mut("OpaqueRef:SR:s").unwrap()["PBDs"] =
            json!(["OpaqueRef:PBD:p", "OpaqueRef:PBD:q"]);
        d.insert(
            "OpaqueRef:host:j".into(),
            json!({"uuid":uuid(6),"current_operations":{}}),
        );
        d.insert("OpaqueRef:PBD:q".into(),json!({"uuid":uuid(7),"host":"OpaqueRef:host:j","SR":"OpaqueRef:SR:s","currently_attached":false}));
        for key in ["OpaqueRef:PBD:p", "OpaqueRef:PBD:q"] {
            let p = d.get_mut(key).unwrap();
            p["currently_attached"] = json!(false);
            p["device_config"] = json!({"server":"storage.example.test","serverpath":"/data","password-secret":"private-fixture-secret"});
        }
        d.insert("OpaqueRef:SM:s".into(),json!({"uuid":uuid(8),"type":"nfs","capabilities":["VDI_READ_CACHING","THIN_PROVISIONING"]}));
    }
    f
}
fn repair() -> StorageMaintenance {
    StorageMaintenance::Repair {
        coordinator: uuid(3).into(),
        attachments: [(7, 6), (5, 3)]
            .into_iter()
            .map(|(p, h)| ReviewedStorageAttachment {
                attachment: uuid(p).into(),
                host: uuid(h).into(),
                location: Some(StorageLocation::Nfs {
                    server: "storage.example.test".into(),
                    path: "/data".into(),
                    version: NfsVersion::Automatic,
                }),
                requires_credentials: true,
            })
            .collect(),
    }
}
fn pending(op: StorageMaintenance) -> (crate::ResourceOperation, ResourceRequest) {
    resources::channel(
        OperationId::new(),
        ObservedResource {
            pool: uuid(1).into(),
            generation: 1,
            target: ResourceTarget::Storage(uuid(4).into()),
        },
        ResourceAction::StorageMaintenance(op),
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
async fn execute(
    f: &Fixture,
    dir: &Path,
    op: StorageMaintenance,
) -> crate::ResourceOperationRecord {
    let (o, r) = pending(op);
    run(f, dir, r).await;
    (*o.latest()).clone()
}
#[tokio::test]
async fn repair_existing_authenticated_attachments_coordinator_first_without_copying_credentials() {
    let f = fixture("").await;
    let d = tempfile::tempdir().unwrap();
    let r = execute(&f, d.path(), repair()).await;
    assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
    assert_eq!(f.count("Async.PBD.plug"), 2);
    let j = Journal::open(d.path()).unwrap();
    let (_, fresh) = pending(repair());
    let mut duplicate = fresh.record;
    duplicate.id = r.id;
    assert!(!j.resource_admit(&mut duplicate).unwrap());
    assert_eq!(duplicate.state, OperationState::Succeeded);
    assert_eq!(
        duplicate.storage_maintenance_plan,
        r.storage_maintenance_plan
    );
    assert_eq!(f.count("PBD.create") + f.count("secret.create"), 0);
    let text = serde_json::to_string(&r).unwrap();
    assert!(
        !text.contains("OpaqueRef:")
            && !text.contains("private-fixture-secret")
            && !text.contains("configuration")
            && !text.contains("tasks_before")
    );
}
#[tokio::test]
async fn changed_attachment_or_coordinator_refuses_before_first_write() {
    for changed in ["host", "location", "coordinator", "attached", "capability"] {
        let f = fixture("").await;
        {
            let mut d = f.records.lock().unwrap();
            match changed {
                "host" => d.get_mut("OpaqueRef:PBD:p").unwrap()["host"] = json!("OpaqueRef:host:j"),
                "location" => {
                    d.get_mut("OpaqueRef:PBD:p").unwrap()["device_config"]["serverpath"] =
                        json!("/changed")
                }
                "coordinator" => {
                    d.get_mut("OpaqueRef:pool:p").unwrap()["master"] = json!("OpaqueRef:host:j")
                }
                "attached" => {
                    d.get_mut("OpaqueRef:PBD:p").unwrap()["currently_attached"] = json!(true)
                }
                _ => d.get_mut("OpaqueRef:SR:s").unwrap()["allowed_operations"] = json!([]),
            }
        }
        let d = tempfile::tempdir().unwrap();
        let r = execute(&f, d.path(), repair()).await;
        assert_eq!(r.state, OperationState::Rejected, "{changed}: {}", r.detail);
        assert_eq!(f.count("Async.PBD.plug"), 0);
    }
}
#[tokio::test]
async fn later_refusal_or_configuration_change_retains_first_verified_host() {
    for fault in ["refuse-second", "change-second"] {
        let f = fixture(fault).await;
        let d = tempfile::tempdir().unwrap();
        let r = execute(&f, d.path(), repair()).await;
        assert_eq!(
            r.state,
            OperationState::PartiallyCompleted,
            "{fault}: {}",
            r.detail
        );
        assert_eq!(r.steps[0].state, StepState::Verified);
        assert_ne!(r.steps[1].state, StepState::Verified);
        assert_eq!(
            f.count("Async.PBD.plug"),
            if fault == "refuse-second" { 2 } else { 1 }
        );
    }
}
#[tokio::test]
async fn lost_reply_or_failed_task_never_infers_success_from_attached_state_or_replays() {
    for fault in ["lost", "task-failure", "reuse-task"] {
        let f = fixture(fault).await;
        let d = tempfile::tempdir().unwrap();
        let r = execute(&f, d.path(), repair()).await;
        assert_eq!(
            r.state,
            OperationState::OutcomeUnknown,
            "{fault}: {}",
            r.detail
        );
        let count = f.count("Async.PBD.plug");
        let r = recheck(&f, d.path(), r).await;
        assert_eq!(r.state, OperationState::OutcomeUnknown, "{}", r.detail);
        assert_eq!(f.count("Async.PBD.plug"), count);
    }
}
#[tokio::test]
async fn late_storage_evidence_preserves_recovery_and_ack_without_second_host() {
    for fault in ["late-reply", "late-task"] {
        for ack in [false, true] {
            let f = fixture(fault).await;
            let d = tempfile::tempdir().unwrap();
            let (o, r) = pending(repair());
            let execute = run(&f, d.path(), r);
            let recover = async {
                f.response_gate.wait().await;
                let j = Journal::open(d.path()).unwrap();
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
            assert!(
                r.storage_maintenance_plan.as_ref().unwrap().evidence[0]
                    .task
                    .is_some()
            );
            assert_eq!(f.count("Async.PBD.plug"), 1);
        }
    }
}
#[tokio::test]
async fn cache_policy_preserves_unrelated_keys_and_reports_only_policy() {
    for (previous, enabled) in [(None, false), (Some("false"), false), (Some("true"), true)] {
        let f = fixture("").await;
        if let Some(v) = previous {
            f.records.lock().unwrap().get_mut("OpaqueRef:SR:s").unwrap()["other_config"]["o_direct"] =
                json!(v);
        }
        let d = tempfile::tempdir().unwrap();
        let r = execute(
            &f,
            d.path(),
            StorageMaintenance::ReadCaching {
                expected: previous.map(str::to_owned),
                enabled,
            },
        )
        .await;
        assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
        let data = f.records.lock().unwrap();
        assert_eq!(
            data["OpaqueRef:SR:s"]["other_config"]["unrelated"],
            "preserve"
        );
        assert_eq!(
            data["OpaqueRef:SR:s"]["other_config"]
                .get("o_direct")
                .cloned(),
            if enabled { None } else { Some(json!("true")) }
        );
        assert_eq!(f.count("SR.set_other_config"), 0);
    }
}
#[tokio::test]
async fn cache_policy_requires_advertised_feature_and_exact_previous_value() {
    for fault in ["feature", "backend", "expected"] {
        let f = fixture("").await;
        {
            let mut d = f.records.lock().unwrap();
            match fault {
                "feature" => d.get_mut("OpaqueRef:SM:s").unwrap()["capabilities"] = json!([]),
                "backend" => d.get_mut("OpaqueRef:SR:s").unwrap()["type"] = json!("lvmoiscsi"),
                _ => {
                    d.get_mut("OpaqueRef:SR:s").unwrap()["other_config"]["o_direct"] = json!("true")
                }
            }
        }
        let d = tempfile::tempdir().unwrap();
        let r = execute(
            &f,
            d.path(),
            StorageMaintenance::ReadCaching {
                expected: None,
                enabled: false,
            },
        )
        .await;
        assert_eq!(r.state, OperationState::Rejected, "{fault}: {}", r.detail);
        assert_eq!(f.count("SR.add_to_other_config"), 0);
    }
}
#[tokio::test]
async fn cache_lost_reply_stays_unknown_and_later_refusal_is_partial() {
    for fault in ["lost", "refuse-add"] {
        let f = fixture(fault).await;
        f.records.lock().unwrap().get_mut("OpaqueRef:SR:s").unwrap()["other_config"]["o_direct"] =
            json!("false");
        let d = tempfile::tempdir().unwrap();
        let r = execute(
            &f,
            d.path(),
            StorageMaintenance::ReadCaching {
                expected: Some("false".into()),
                enabled: false,
            },
        )
        .await;
        assert_eq!(
            r.state,
            if fault == "lost" {
                OperationState::OutcomeUnknown
            } else {
                OperationState::PartiallyCompleted
            },
            "{}",
            r.detail
        );
        if fault == "lost" {
            let r = recheck(&f, d.path(), r).await;
            assert_eq!(r.state, OperationState::OutcomeUnknown);
            assert_eq!(f.count("SR.remove_from_other_config"), 1);
            assert_eq!(f.count("SR.add_to_other_config"), 0);
        }
    }
}
#[tokio::test]
async fn maintenance_journal_rejects_rebound_plan_impossible_evidence_and_short_steps() {
    let f = fixture("").await;
    let d = tempfile::tempdir().unwrap();
    let r = execute(&f, d.path(), repair()).await;
    let j = Journal::open(d.path()).unwrap();
    for n in 0..4 {
        let mut b = r.clone();
        match n {
            0 => b.storage_maintenance_plan.as_mut().unwrap().sr = uuid(99).into(),
            1 => b.steps.clear(),
            2 => b.storage_maintenance_plan.as_mut().unwrap().evidence[0].completed = false,
            _ => b.storage_maintenance_plan = None,
        }
        assert!(j.resource_save(&mut b).is_err());
        b.id = OperationId::new();
        assert!(matches!(
            j.resource_admit(&mut b),
            Err(crate::Error::Journal)
        ));
    }
    let payload: String =
        j.0.query_row(
            "SELECT payload FROM resource_operations WHERE id=?1",
            [r.id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    for field in ["steps", "task", "completed", "missing"] {
        let mut value: Value = serde_json::from_str(&payload).unwrap();
        match field {
            "steps" => value["steps"] = json!([]),
            "task" => {
                value["storage_maintenance_plan"]["evidence"][1]["task_uuid"] =
                    value["storage_maintenance_plan"]["evidence"][0]["task_uuid"].clone()
            }
            "completed" => {
                value["storage_maintenance_plan"]["evidence"][0]["completed"] = json!(false)
            }
            _ => {
                value
                    .as_object_mut()
                    .unwrap()
                    .remove("storage_maintenance_plan");
            }
        }
        j.0.execute(
            "UPDATE resource_operations SET payload=?1 WHERE id=?2",
            rusqlite::params![value.to_string(), r.id.to_string()],
        )
        .unwrap();
        assert!(
            matches!(j.resource_load(r.id), Err(crate::Error::Journal)),
            "{field}"
        );
    }
}
#[tokio::test]
async fn known_permission_denial_blocks_all_repairs_before_dispatch() {
    let f = fixture("").await;
    f.records.lock().unwrap().insert(
        "OpaqueRef:synthetic-secret".into(),
        json!({"is_local_superuser":false,"rbac_permissions":["sr.scan"]}),
    );
    let d = tempfile::tempdir().unwrap();
    let r = execute(&f, d.path(), repair()).await;
    assert_eq!(r.state, OperationState::Rejected, "{}", r.detail);
    assert_eq!(f.count("Async.PBD.plug"), 0);
}
#[tokio::test]
async fn recovered_first_host_does_not_auto_submit_remaining_repairs_on_explicit_recheck() {
    let f = fixture("late-reply").await;
    let d = tempfile::tempdir().unwrap();
    let (o, r) = pending(repair());
    let execute = run(&f, d.path(), r);
    let recover = async {
        f.response_gate.wait().await;
        Journal::open(d.path())
            .unwrap()
            .resource_recover(uuid(1).into())
            .unwrap();
        f.response_gate.resume();
    };
    tokio::join!(execute, recover);
    let r = recheck(&f, d.path(), (*o.latest()).clone()).await;
    assert_eq!(r.state, OperationState::PartiallyCompleted, "{}", r.detail);
    assert_eq!(r.steps[0].state, StepState::Verified);
    assert_eq!(r.steps[1].state, StepState::Pending);
    assert_eq!(f.count("Async.PBD.plug"), 1);
}
