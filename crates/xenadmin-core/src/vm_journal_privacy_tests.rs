use super::*;
use crate::{
    CreatedDevice, HostId, SnapshotId, SrId, VdiId, VmCopyMode, VmLifecycleOperation, VmMigration,
};
fn fixture() -> OperationRecord {
    let mut r:OperationRecord=serde_json::from_value(json!({"id":OperationId::new(),"pool":Uuid::from_u128(1),"vm":Uuid::from_u128(2),"action":"Start","state":"OutcomeUnknown","progress":null,"detail":"Fixture interrupted operation"})).unwrap();
    let disk = VdiId::from(Uuid::from_u128(3));
    let created = VmId::from(Uuid::from_u128(4));
    r.task = Some("OpaqueRef:top-level-task".into());
    r.result_reference = Some("OpaqueRef:top-level-result".into());
    r.cancel_dispatched = true;
    r.created_vm = Some(created);
    r.created_devices = vec![CreatedDevice::Disk(disk)];
    r.snapshot_deletion = Some(crate::snapshots::DeletionPlan {
        snapshot: SnapshotId::from(Uuid::from_u128(5)),
        checkpoint: true,
        disks: vec![disk],
        memory: Some(VdiId::from(Uuid::from_u128(6))),
    });
    r.snapshot_clone_baseline = Some(vec![r.vm]);
    let device = crate::device_lifecycle::Plan {
        steps: vec![crate::device_lifecycle::Step::CreateDisk {
            sr: SrId::from(Uuid::from_u128(7)),
            name: "Synthetic disk".into(),
            size: 4096,
        }],
        evidence: vec![Some(crate::device_lifecycle::Evidence {
            baseline: vec![Uuid::from_u128(8)],
            result_reference: Some("OpaqueRef:device-result".into()),
            created: Some(CreatedDevice::Disk(disk)),
        })],
    };
    r.device_plan = Some(device.clone());
    r.lifecycle_plan = Some(Box::new(crate::vm_lifecycle_backend::Plan {
        source_graph: Default::default(),
        metadata: Some(("Synthetic source".into(), "Synthetic description".into())),
        source: r.vm,
        operation: VmLifecycleOperation::Clone {
            name: "Synthetic clone".into(),
            expected_template: false,
            mode: VmCopyMode::Fast,
            as_template: false,
        },
        steps: vec![crate::vm_lifecycle_backend::Step::Copy {
            mode: VmCopyMode::Fast,
            name: "Synthetic clone".into(),
        }],
        evidence: vec![crate::vm_lifecycle_backend::Evidence {
            task: Some("OpaqueRef:copy-task".into()),
            result: Some("OpaqueRef:copy-result".into()),
            completed: true,
        }],
        created: Some(created),
        baseline: std::collections::BTreeMap::from([("VM".into(), vec![r.vm.uuid()])]),
        created_devices: vec![CreatedDevice::Disk(disk)],
        groups: vec![device],
        owned_disks: vec![disk],
        workflow_planned: true,
    }));
    r.migration_plan = Some(Box::new(crate::migration_backend::Plan {
        vm: r.vm,
        operation: VmMigration {
            destination: HostId::from(Uuid::from_u128(9)),
            expected_host: Some(HostId::from(Uuid::from_u128(10))),
            expected_affinity: None,
            live: false,
            disks: vec![],
            networks: vec![],
            transfer_network: None,
        },
        graph: crate::migration_backend::Graph {
            disks: vec![],
            nics: vec![],
        },
        baseline: vec![disk.uuid()],
        completed: false,
    }));
    r
}
fn assert_private_public_boundary(r: &OperationRecord) {
    let value = serde_json::to_value(r).unwrap();
    for key in [
        "task",
        "cancel_dispatched",
        "result_reference",
        "snapshot_deletion",
        "snapshot_clone_baseline",
        "device_plan",
        "lifecycle_plan",
        "migration_plan",
    ] {
        assert!(
            value.get(key).is_none(),
            "private field {key} was serialized"
        );
    }
    assert!(!value.to_string().contains("OpaqueRef:"));
    assert_eq!(value["created_vm"], json!(r.created_vm));
    assert_eq!(value["created_devices"], json!(r.created_devices));
    assert_eq!(value["state"], "OutcomeUnknown");
}
#[test]
fn public_vm_record_hides_reconciliation_evidence_while_journal_history_and_recovery_preserve_it() {
    let directory = tempfile::tempdir().unwrap();
    let mut journal = Journal::open(directory.path()).unwrap();
    let mut original = fixture();
    assert_private_public_boundary(&original);
    journal.admit(&mut original).unwrap();
    assert_eq!(journal.load(original.id).unwrap().unwrap(), original);
    assert_eq!(journal.history().unwrap(), vec![original.clone()]);
    drop(journal);
    let recovered = recover(directory.path(), original.pool).unwrap().remove(0);
    assert_eq!(recovered.task, original.task);
    assert_eq!(recovered.result_reference, original.result_reference);
    assert_eq!(recovered.snapshot_deletion, original.snapshot_deletion);
    assert_eq!(
        recovered.snapshot_clone_baseline,
        original.snapshot_clone_baseline
    );
    assert_eq!(recovered.device_plan, original.device_plan);
    assert_eq!(recovered.lifecycle_plan, original.lifecycle_plan);
    assert_eq!(recovered.migration_plan, original.migration_plan);
    assert!(recovered.cancel_dispatched);
    assert_private_public_boundary(&recovered);
    let mut journal = Journal::open(directory.path()).unwrap();
    let mut conflict = fixture();
    conflict.id = OperationId::new();
    conflict.vm = original.created_vm.unwrap();
    assert!(matches!(
        journal.admit(&mut conflict),
        Err(Error::ConflictingOperation)
    ));
}
#[test]
fn legacy_flat_private_payload_loads_without_schema_change_and_task_column_remains_authoritative() {
    let directory = tempfile::tempdir().unwrap();
    let journal = Journal::open(directory.path()).unwrap();
    let id = OperationId::new();
    let legacy = json!({"id":id,"pool":Uuid::from_u128(20),"vm":Uuid::from_u128(21),"action":"Start","state":"OutcomeUnknown","progress":null,"detail":"Legacy fixture","task":"OpaqueRef:ignored-payload-task","cancel_dispatched":true,"result_reference":"OpaqueRef:legacy-result","snapshot_deletion":{"snapshot":Uuid::from_u128(22),"checkpoint":true,"disks":[Uuid::from_u128(23)],"memory":Uuid::from_u128(24)},"snapshot_clone_baseline":[Uuid::from_u128(25)],"device_plan":{"steps":[],"evidence":[{"baseline":[],"result_reference":"OpaqueRef:legacy-device","created":null}]}});
    journal.0.execute("INSERT INTO operations(id,pool,vm,payload,task,terminal,revision) VALUES(?1,?2,?3,?4,?5,0,8)",rusqlite::params![id.to_string(),Uuid::from_u128(20).to_string(),Uuid::from_u128(21).to_string(),legacy.to_string(),"OpaqueRef:column-task"]).unwrap();
    let loaded = journal.load(id).unwrap().unwrap();
    assert_eq!(loaded.task.as_deref(), Some("OpaqueRef:column-task"));
    assert_eq!(
        loaded.result_reference.as_deref(),
        Some("OpaqueRef:legacy-result")
    );
    assert_eq!(
        loaded.device_plan.as_ref().unwrap().evidence[0]
            .as_ref()
            .unwrap()
            .result_reference
            .as_deref(),
        Some("OpaqueRef:legacy-device")
    );
    assert_eq!(
        loaded.snapshot_deletion.as_ref().unwrap().memory,
        Some(VdiId::from(Uuid::from_u128(24)))
    );
    assert_eq!(
        loaded.snapshot_clone_baseline,
        Some(vec![VmId::from(Uuid::from_u128(25))])
    );
    assert_eq!(loaded.revision(), 8);
    assert!(loaded.cancel_dispatched);
    assert_private_public_boundary(&loaded);
    assert_eq!(
        journal
            .0
            .query_row("PRAGMA user_version", [], |row| row.get::<_, u64>(0))
            .unwrap(),
        3
    );
}
#[test]
fn malformed_private_plan_is_not_silently_dropped_from_journal_evidence() {
    let directory = tempfile::tempdir().unwrap();
    let mut journal = Journal::open(directory.path()).unwrap();
    let mut original = fixture();
    journal.admit(&mut original).unwrap();
    let mut payload: serde_json::Value =
        serde_json::from_str(&Journal::encode_record(&original).unwrap()).unwrap();
    payload["lifecycle_plan"] = json!({"steps":"malformed"});
    journal
        .0
        .execute(
            "UPDATE operations SET payload=?1 WHERE id=?2",
            rusqlite::params![payload.to_string(), original.id.to_string()],
        )
        .unwrap();
    assert!(matches!(journal.load(original.id), Err(Error::Journal)));
    assert!(matches!(journal.history(), Err(Error::Journal)));
}
