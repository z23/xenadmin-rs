use super::*;
use crate::{
    DiskConsumer, DiskOperation, OwnedDiskSource, ResourceAction, ReviewedDisk, StepState,
};
pub(super) struct Reply {
    pub value: Value,
    pub lost: bool,
    pub paused: bool,
}
pub(super) fn rpc(method: &str, p: &Value, data: &mut BTreeMap<String, Value>) -> Option<Reply> {
    if !data.contains_key("fixture:disk-admin") {
        return None;
    }
    let fault = data["fixture:disk-admin"].as_str().unwrap_or("").to_owned();
    let reply = |v, lost, paused| {
        Some(Reply {
            value: v,
            lost,
            paused,
        })
    };
    if method == "VDI.set_name_label" {
        if fault == "name-refused" {
            return reply(
                json!({"error":{"message":"OPERATION_NOT_ALLOWED"}}),
                false,
                false,
            );
        }
        data.get_mut(p[1].as_str().unwrap()).unwrap()["name_label"] = p[2].clone();
        return reply(json!({"result":null}), fault == "lost-name", false);
    }
    if method == "task.get_record"
        && fault == "late-task"
        && data.get(p[1].as_str().unwrap()).is_some()
    {
        return reply(json!({"result":data[p[1].as_str().unwrap()]}), false, true);
    }
    if !matches!(
        method,
        "Async.VDI.resize" | "Async.VDI.copy" | "Async.VDI.destroy"
    ) {
        return None;
    }
    let target = p[1].as_str().unwrap();
    let result = match method {
        "Async.VDI.resize" => {
            let size = p[2].as_str().unwrap().parse::<u64>().unwrap();
            data.get_mut(target).unwrap()["virtual_size"] =
                json!((size + if fault == "round" { 512 } else { 0 }).to_string());
            ""
        }
        "Async.VDI.copy" => {
            assert_eq!(p[3], "OpaqueRef:NULL");
            assert_eq!(p[4], "OpaqueRef:NULL");
            let mut disk = data[target].clone();
            disk["uuid"] = json!(uuid(22));
            disk["SR"] = p[2].clone();
            disk["VBDs"] = json!([]);
            data.insert("OpaqueRef:VDI:new".into(), disk);
            if fault == "old-result" {
                target
            } else {
                "OpaqueRef:VDI:new"
            }
        }
        _ => {
            data.remove(target);
            ""
        }
    };
    data.insert("OpaqueRef:task:t".into(),json!({"uuid":uuid(200),"status":if fault=="task-failure"{"failure"}else{"success"},"progress":1.0,"result":xml(&json!(result)),"allowed_operations":[]}));
    reply(
        json!({"result":"OpaqueRef:task:t"}),
        fault == "lost",
        fault == "late-reply",
    )
}
async fn fixture(fault: &str) -> Fixture {
    let f = Fixture::new(Fault::None).await;
    f.disk();
    {
        let mut rows = f.records.lock().unwrap();
        rows.insert("fixture:disk-admin".into(), json!(fault));
        let v = rows.get_mut("OpaqueRef:VDI:source").unwrap();
        v["snapshot_of"] = json!("OpaqueRef:NULL");
        v["allowed_operations"] = json!(["resize", "copy", "destroy"]);
        rows.get_mut("OpaqueRef:pool:p").unwrap()["ha_statefiles"] = json!([]);
    }
    f
}
fn expected() -> ReviewedDisk {
    ReviewedDisk {
        sr: uuid(4).into(),
        size: 4096,
        consumers: Vec::new(),
    }
}
fn copy() -> DiskOperation {
    DiskOperation::Copy {
        expected: expected(),
        destination: uuid(4).into(),
        name: "Independent copy".into(),
    }
}
fn pending(disk: u128, op: DiskOperation) -> (crate::ResourceOperation, ResourceRequest) {
    resources::channel(
        OperationId::new(),
        ObservedResource {
            pool: uuid(1).into(),
            generation: 1,
            target: ResourceTarget::Disk(uuid(disk).into()),
        },
        ResourceAction::Disk(op),
        tokio::runtime::Handle::current(),
    )
}
async fn run(f: &Fixture, dir: &Path, request: ResourceRequest) {
    let (tx, _rx) = mpsc::channel(512);
    tokio::time::timeout(
        std::time::Duration::from_secs(30),
        request.run(
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
    disk: u128,
    op: DiskOperation,
) -> crate::ResourceOperationRecord {
    let (o, r) = pending(disk, op);
    run(f, dir, r).await;
    (*o.latest()).clone()
}
#[tokio::test]
async fn detached_growth_accepts_driver_rounding_and_reports_actual_size() {
    let f = fixture("round").await;
    let d = tempfile::tempdir().unwrap();
    let r = execute(
        &f,
        d.path(),
        10,
        DiskOperation::Grow {
            expected: expected(),
            size: 8192,
        },
    )
    .await;
    assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
    assert_eq!(r.disk_size, Some(8704));
    assert_eq!(f.count("Async.VDI.resize"), 1);
}
#[tokio::test]
async fn inactive_running_consumer_is_supported_but_attached_reserved_and_unreviewed_consumers_are_refused()
 {
    for bad in [
        "",
        "attached",
        "reserved",
        "unreviewed",
        "snapshot",
        "paused",
    ] {
        let f = fixture("").await;
        {
            let mut rows = f.records.lock().unwrap();
            let vm = rows.get_mut("OpaqueRef:VM:source").unwrap();
            vm["power_state"] = json!(if bad == "paused" { "Paused" } else { "Running" });
            vm["VBDs"] = json!(["OpaqueRef:VBD:old"]);
            vm["is_a_snapshot"] = json!(bad == "snapshot");
            rows.get_mut("OpaqueRef:VDI:source").unwrap()["VBDs"] = json!(["OpaqueRef:VBD:old"]);
            rows.insert("OpaqueRef:VBD:old".into(),json!({"uuid":uuid(12),"VM":"OpaqueRef:VM:source","VDI":"OpaqueRef:VDI:source","type":"Disk","empty":false,"reserved":bad=="reserved","currently_attached":bad=="attached","current_operations":{}}));
        }
        let mut e = expected();
        if bad != "unreviewed" {
            e.consumers.push(DiskConsumer {
                attachment: uuid(12).into(),
                vm: uuid(2).into(),
            });
        }
        let d = tempfile::tempdir().unwrap();
        let r = execute(
            &f,
            d.path(),
            10,
            DiskOperation::Grow {
                expected: e,
                size: 8192,
            },
        )
        .await;
        assert_eq!(
            r.state == OperationState::Succeeded,
            bad.is_empty(),
            "{bad}: {}",
            r.detail
        );
        assert_eq!(f.count("Async.VDI.resize"), usize::from(bad.is_empty()));
    }
}
#[tokio::test]
async fn copy_is_new_detached_named_and_has_private_ownership_proof() {
    let f = fixture("").await;
    let d = tempfile::tempdir().unwrap();
    let r = execute(&f, d.path(), 10, copy()).await;
    assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
    assert_eq!(r.created, Some(ResourceTarget::Disk(uuid(22).into())));
    assert_eq!(r.owned_disk_candidates(), vec![uuid(22).into()]);
    let public = serde_json::to_string(&r).unwrap();
    assert!(
        !public.contains("OpaqueRef:")
            && !public.contains("disk_plan")
            && !public.contains("tasks_before")
    );
    let rows = f.records.lock().unwrap();
    assert_eq!(rows["OpaqueRef:VDI:source"]["name_label"], "Existing disk");
    assert_eq!(rows["OpaqueRef:VDI:new"]["name_label"], "Independent copy");
    assert_eq!(rows["OpaqueRef:VDI:new"]["VBDs"], json!([]));
}
#[tokio::test]
async fn copy_rejects_existing_result_and_retains_partial_new_disk_on_name_refusal() {
    for fault in ["old-result", "name-refused"] {
        let f = fixture(fault).await;
        let d = tempfile::tempdir().unwrap();
        let r = execute(&f, d.path(), 10, copy()).await;
        if fault == "old-result" {
            assert_eq!(r.state, OperationState::OutcomeUnknown, "{}", r.detail);
            assert!(r.created.is_none());
            assert_eq!(f.count("VDI.set_name_label"), 0);
        } else {
            assert_eq!(r.state, OperationState::PartiallyCompleted, "{}", r.detail);
            assert_eq!(r.created, Some(ResourceTarget::Disk(uuid(22).into())));
            assert_eq!(r.owned_disk_candidates(), vec![uuid(22).into()]);
        }
        assert_eq!(f.count("Async.VDI.destroy"), 0);
    }
}
#[tokio::test]
async fn lost_copy_or_resize_reply_never_infers_success_or_replays() {
    for op in [
        copy(),
        DiskOperation::Grow {
            expected: expected(),
            size: 8192,
        },
    ] {
        let f = fixture("lost").await;
        let d = tempfile::tempdir().unwrap();
        let r = execute(&f, d.path(), 10, op).await;
        assert_eq!(r.state, OperationState::OutcomeUnknown, "{}", r.detail);
        let r = recheck(&f, d.path(), r).await;
        assert_eq!(r.state, OperationState::OutcomeUnknown, "{}", r.detail);
        assert_eq!(f.count("Async.VDI.copy") + f.count("Async.VDI.resize"), 1);
        assert_eq!(f.count("VDI.set_name_label"), 0);
    }
}
#[tokio::test]
async fn lost_name_reply_keeps_copy_identity_and_recheck_does_not_repeat_setter() {
    let f = fixture("lost-name").await;
    let d = tempfile::tempdir().unwrap();
    let r = execute(&f, d.path(), 10, copy()).await;
    assert_eq!(r.state, OperationState::OutcomeUnknown, "{}", r.detail);
    assert_eq!(r.created, Some(ResourceTarget::Disk(uuid(22).into())));
    let r = recheck(&f, d.path(), r).await;
    assert_eq!(r.state, OperationState::OutcomeUnknown, "{}", r.detail);
    assert_eq!(f.count("VDI.set_name_label"), 1);
}
#[tokio::test]
async fn cleanup_requires_terminal_owned_creation_and_refuses_all_consumers() {
    for consumer in ["", "vbd", "suspend", "snapshot", "untracked"] {
        let f = fixture("").await;
        let d = tempfile::tempdir().unwrap();
        let created = execute(&f, d.path(), 10, copy()).await;
        assert_eq!(
            created.state,
            OperationState::Succeeded,
            "{}",
            created.detail
        );
        {
            let mut rows = f.records.lock().unwrap();
            match consumer {
                "vbd" => {
                    rows.insert(
                        "OpaqueRef:VBD:surprise".into(),
                        json!({"uuid":uuid(65),"VDI":"OpaqueRef:VDI:new"}),
                    );
                }
                "suspend" => {
                    rows.get_mut("OpaqueRef:VM:source").unwrap()["suspend_VDI"] =
                        json!("OpaqueRef:VDI:new")
                }
                "snapshot" => {
                    rows.insert(
                        "OpaqueRef:VDI:snapshot".into(),
                        json!({"uuid":uuid(66),"snapshot_of":"OpaqueRef:VDI:new"}),
                    );
                }
                _ => {}
            }
        }
        let source = OwnedDiskSource::ResourceOperation(if consumer == "untracked" {
            OperationId::new()
        } else {
            created.id
        });
        let r = execute(
            &f,
            d.path(),
            22,
            DiskOperation::Cleanup {
                expected: expected(),
                source,
            },
        )
        .await;
        assert_eq!(
            r.state == OperationState::Succeeded,
            consumer.is_empty(),
            "{consumer}: {}",
            r.detail
        );
        assert_eq!(
            f.count("Async.VDI.destroy"),
            usize::from(consumer.is_empty())
        );
    }
}
#[tokio::test]
async fn recovered_or_acknowledged_copy_retains_late_receipts_without_naming() {
    for fault in ["late-reply", "late-task"] {
        for ack in [false, true] {
            let f = fixture(fault).await;
            let d = tempfile::tempdir().unwrap();
            let (o, request) = pending(10, copy());
            let execute = run(&f, d.path(), request);
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
            assert!(r.disk_plan.as_ref().unwrap().evidence[0].task.is_some());
            if fault == "late-task" {
                assert!(r.disk_plan.as_ref().unwrap().evidence[0].completed);
            }
            assert_eq!(f.count("VDI.set_name_label"), 0);
            assert_eq!(f.count("Async.VDI.copy"), 1);
        }
    }
}
#[tokio::test]
async fn disk_journal_rejects_changed_identity_impossible_completion_and_public_shape() {
    let f = fixture("").await;
    let d = tempfile::tempdir().unwrap();
    let r = execute(&f, d.path(), 10, copy()).await;
    let j = Journal::open(d.path()).unwrap();
    let mut bad = r.clone();
    bad.disk_plan.as_mut().unwrap().disk = uuid(88).into();
    assert!(j.resource_save(&mut bad).is_err());
    let mut bad = r.clone();
    bad.disk_plan.as_mut().unwrap().evidence[0].completed = false;
    assert!(j.resource_save(&mut bad).is_err());
    let mut bad = r.clone();
    bad.steps.pop();
    assert!(j.resource_save(&mut bad).is_err());
    let mut bad = r.clone();
    bad.disk_plan = None;
    assert!(j.resource_save(&mut bad).is_err());
    assert!(r.steps.iter().all(|s| s.state == StepState::Verified));
}
#[tokio::test]
async fn changed_size_read_only_shared_and_missing_capability_fail_before_dispatch() {
    for field in [
        "virtual_size",
        "read_only",
        "sharable",
        "allowed_operations",
        "cbt_enabled",
    ] {
        let f = fixture("").await;
        f.records
            .lock()
            .unwrap()
            .get_mut("OpaqueRef:VDI:source")
            .unwrap()[field] = match field {
            "virtual_size" => json!(8192),
            "allowed_operations" => json!([]),
            _ => json!(true),
        };
        let d = tempfile::tempdir().unwrap();
        let r = execute(&f, d.path(), 10, copy()).await;
        assert_eq!(r.state, OperationState::Rejected, "{field}: {}", r.detail);
        assert_eq!(f.count("Async.VDI.copy"), 0);
    }
}

#[tokio::test]
async fn imported_disk_cleanup_requires_the_original_creation_marker() {
    for changed in [false, true] {
        let f = Fixture::new(Fault::None).await;
        f.disk();
        let d = tempfile::tempdir().unwrap();
        let imported =
            super::disk_tests::execute_disk(&f, d.path(), crate::DiskImageFormat::Raw, false).await;
        assert_eq!(
            imported.state,
            OperationState::Succeeded,
            "{}",
            imported.detail
        );
        assert_eq!(imported.owned_disk_candidates(), vec![uuid(22).into()]);
        {
            let mut rows = f.records.lock().unwrap();
            rows.insert("fixture:disk-admin".into(), json!(""));
            rows.get_mut("OpaqueRef:pool:p").unwrap()["ha_statefiles"] = json!([]);
            let v = rows.get_mut("OpaqueRef:VDI:new").unwrap();
            v["snapshot_of"] = json!("OpaqueRef:NULL");
            v["allowed_operations"] = json!(["destroy"]);
            if changed {
                v["other_config"] = json!({});
            }
        }
        let r = execute(
            &f,
            d.path(),
            22,
            DiskOperation::Cleanup {
                expected: expected(),
                source: OwnedDiskSource::ResourceOperation(imported.id),
            },
        )
        .await;
        assert_eq!(
            r.state == OperationState::Succeeded,
            !changed,
            "{}",
            r.detail
        );
        assert_eq!(f.count("Async.VDI.destroy"), usize::from(!changed));
    }
}
#[tokio::test]
async fn pool_ha_uuid_dependency_blocks_destroy_even_if_the_disk_type_is_user() {
    let f = fixture("").await;
    let d = tempfile::tempdir().unwrap();
    let origin = execute(&f, d.path(), 10, copy()).await;
    assert_eq!(origin.state, OperationState::Succeeded);
    f.records
        .lock()
        .unwrap()
        .get_mut("OpaqueRef:pool:p")
        .unwrap()["ha_statefiles"] = json!([uuid(22).to_string()]);
    let r = execute(
        &f,
        d.path(),
        22,
        DiskOperation::Cleanup {
            expected: expected(),
            source: OwnedDiskSource::ResourceOperation(origin.id),
        },
    )
    .await;
    assert_eq!(r.state, OperationState::Rejected);
    assert_eq!(f.count("Async.VDI.destroy"), 0);
}

#[tokio::test]
async fn known_missing_name_permission_prevents_copy_before_any_partial_change() {
    let f = fixture("").await;
    f.records.lock().unwrap().insert(
        "OpaqueRef:synthetic-secret".into(),
        json!({"is_local_superuser":false,"rbac_permissions":["vdi.copy"]}),
    );
    let d = tempfile::tempdir().unwrap();
    let r = execute(&f, d.path(), 10, copy()).await;
    assert_eq!(r.state, OperationState::Rejected, "{}", r.detail);
    assert_eq!(f.count("Async.VDI.copy"), 0);
    assert_eq!(f.count("VDI.set_name_label"), 0);
}
