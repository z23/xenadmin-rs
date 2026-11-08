use super::*;
use crate::{
    NetworkId, OperationStep, ResourceAction, ResourceOperationRecord, ResourceTarget, StepState,
};

fn resource(pool: PoolId) -> ResourceOperationRecord {
    ResourceOperationRecord {
        topology_changes: Default::default(),
        network_topology_plan: None,
        interrupted: false,
        id: OperationId::new(),
        pool,
        target: ResourceTarget::Network(NetworkId::from(uuid::Uuid::new_v4())),
        action: ResourceAction::ConfigureInternalNetwork {
            expected_mtu: 1500,
            mtu: 9000,
        },
        state: OperationState::Preparing,
        progress: None,
        detail: "Fixture operation".into(),
        created_at_ms: 0,
        can_cancel: false,
        cancel_requested: false,
        steps: vec![OperationStep {
            label: "Set MTU".into(),
            state: StepState::Pending,
            detail: String::new(),
        }],
        created: None,
        transfer: None,
        transfer_created: Default::default(),
        transfer_plan: None,
        cloud_init_plan: None,
        local_error: None,
        revision: 0,
        task: None,
        result_reference: None,
        cancel_dispatched: false,
        baseline_networks: None,
        network_baseline: None,
        storage_baseline: None,
        storage_trim_succeeded: false,
        pool_completed: false,
        appliance_completed: false,
        appliance_baseline: None,
        host_plan: None,
        host_power_plan: None,
        host_power_secret: None,
        power_batch_plan: None,
        storage_auth_plan: None,
        storage_secret_cleanup: None,
        storage_secrets: Vec::new(),
        subject_baseline: None,
        snapshot_policy_evidence: None,
        join_baseline: None,
        cross_pool_plan: None,
    }
}
fn vm(pool: PoolId) -> OperationRecord {
    serde_json::from_value(json!({"id":OperationId::new(),"pool":pool,"vm":uuid::Uuid::new_v4(),"action":"Start","state":"Preparing","progress":null,"detail":"Fixture VM operation"})).unwrap()
}
#[test]
fn vm_and_resource_admission_interlock_both_directions_but_not_across_pools() {
    let dir = tempfile::tempdir().unwrap();
    let mut journal = Journal::open(dir.path()).unwrap();
    let pool = PoolId::from(uuid::Uuid::new_v4());
    let mut infrastructure = resource(pool);
    assert!(journal.resource_admit(&mut infrastructure).unwrap());
    assert!(matches!(
        journal.admit(&mut vm(pool)),
        Err(Error::ConflictingPoolOperation)
    ));
    let other_pool = PoolId::from(uuid::Uuid::new_v4());
    assert!(journal.admit(&mut vm(other_pool)).unwrap());
    infrastructure.state = OperationState::Succeeded;
    journal.resource_save(&mut infrastructure).unwrap();
    let mut virtual_machine = vm(pool);
    assert!(journal.admit(&mut virtual_machine).unwrap());
    assert!(matches!(
        journal.resource_admit(&mut resource(pool)),
        Err(Error::ConflictingPoolOperation)
    ));
}
#[test]
fn id_deduplication_and_cross_family_identity_conflicts_do_not_create_second_operations() {
    let dir = tempfile::tempdir().unwrap();
    let mut journal = Journal::open(dir.path()).unwrap();
    let pool = PoolId::from(uuid::Uuid::new_v4());
    let mut original = resource(pool);
    assert!(journal.resource_admit(&mut original).unwrap());
    let mut duplicate = original.clone();
    assert!(!journal.resource_admit(&mut duplicate).unwrap());
    duplicate.action = ResourceAction::DeleteInternalNetwork {
        expected_name: "Fixture network".into(),
    };
    assert!(matches!(
        journal.resource_admit(&mut duplicate),
        Err(Error::OperationConflict)
    ));
    let mut virtual_machine = vm(pool);
    virtual_machine.id = original.id;
    assert!(matches!(
        journal.admit(&mut virtual_machine),
        Err(Error::OperationConflict)
    ));
    assert_eq!(journal.resource_history(pool).unwrap().len(), 1);
}
#[test]
fn dispatch_evidence_merges_cancellation_without_resurrecting_acknowledged_operations() {
    let dir = tempfile::tempdir().unwrap();
    let journal = Journal::open(dir.path()).unwrap();
    let pool = PoolId::from(uuid::Uuid::new_v4());
    let mut operation = resource(pool);
    journal.resource_admit(&mut operation).unwrap();
    operation.state = OperationState::Dispatching;
    journal.resource_save(&mut operation).unwrap();
    journal
        .resource_control(pool, operation.id, OperationControl::RequestCancellation)
        .unwrap();
    operation.task = Some("OpaqueRef:resource-task".into());
    let current = journal.resource_evidence(&operation).unwrap();
    assert!(current.cancel_requested);
    assert_eq!(current.task, operation.task);
    let mut unknown = current.clone();
    unknown.state = OperationState::OutcomeUnknown;
    journal.resource_save(&mut unknown).unwrap();
    operation.result_reference = Some("OpaqueRef:created-resource".into());
    let recovered = journal.resource_evidence(&operation).unwrap();
    assert_eq!(recovered.state, OperationState::OutcomeUnknown);
    assert_eq!(recovered.result_reference, operation.result_reference);
    assert!(journal.resource_pool_busy(pool).unwrap());
    journal
        .resource_control(pool, operation.id, OperationControl::AcknowledgeUnknown)
        .unwrap();
    let current = journal.resource_evidence(&operation).unwrap();
    assert_eq!(current.state, OperationState::AcknowledgedUnknown);
    assert!(current.cancel_requested);
    assert!(matches!(
        journal.resource_save(&mut unknown),
        Err(Error::JournalChanged)
    ));
    assert!(journal.resource_admit(&mut resource(pool)).unwrap());
}
#[test]
fn recovery_never_replays_unsubmitted_steps_and_revokes_old_worker_revision() {
    for states in [
        vec![StepState::Pending],
        vec![StepState::Verified, StepState::Pending],
        vec![StepState::Verified],
        vec![StepState::Dispatching],
    ] {
        let dir = tempfile::tempdir().unwrap();
        let journal = Journal::open(dir.path()).unwrap();
        let pool = PoolId::from(uuid::Uuid::new_v4());
        let mut operation = resource(pool);
        operation.steps = states
            .iter()
            .map(|state| OperationStep {
                label: "Fixture step".into(),
                state: state.clone(),
                detail: String::new(),
            })
            .collect();
        if states[0] == StepState::Dispatching {
            operation.state = OperationState::Dispatching;
        }
        journal.resource_admit(&mut operation).unwrap();
        let recovered = journal.resource_recover(pool).unwrap();
        let expected = match states.as_slice() {
            [StepState::Pending] => OperationState::Cancelled,
            [StepState::Verified] => OperationState::Succeeded,
            [StepState::Dispatching] => OperationState::OutcomeUnknown,
            _ => OperationState::PartiallyCompleted,
        };
        assert_eq!(recovered[0].state, expected);
        operation.state = OperationState::Dispatching;
        assert!(matches!(
            journal.resource_save(&mut operation),
            Err(Error::JournalChanged)
        ));
    }
}
#[test]
fn v2_migration_preserves_vm_evidence_and_installs_resource_journal_once() {
    let dir = tempfile::tempdir().unwrap();
    let pool = PoolId::from(uuid::Uuid::new_v4());
    let mut virtual_machine = vm(pool);
    {
        let mut journal = Journal::open(dir.path()).unwrap();
        journal.admit(&mut virtual_machine).unwrap();
        journal
            .0
            .execute_batch("DROP TABLE resource_operations; PRAGMA user_version=2;")
            .unwrap();
    }
    let journal = Journal::open(dir.path()).unwrap();
    assert_eq!(
        journal.load(virtual_machine.id).unwrap().unwrap().id,
        virtual_machine.id
    );
    assert!(journal.resource_history(pool).unwrap().is_empty());
    assert_eq!(
        journal
            .0
            .query_row("PRAGMA user_version", [], |row| row.get::<_, u64>(0))
            .unwrap(),
        3
    );
    assert!(matches!(
        journal.resource_admit(&mut resource(pool)),
        Err(Error::ConflictingPoolOperation)
    ));
}
#[test]
fn resource_intents_are_closed_typed_and_reject_invalid_or_no_op_values() {
    let target = ResourceTarget::Network(NetworkId::from(uuid::Uuid::new_v4()));
    for action in [
        ResourceAction::ConfigureInternalNetwork {
            expected_mtu: 1500,
            mtu: 1500,
        },
        ResourceAction::ConfigureInternalNetwork {
            expected_mtu: 1500,
            mtu: 65536,
        },
        ResourceAction::ScanStorage,
        ResourceAction::RenameMetadata {
            expected_name: "Old".into(),
            expected_description: "".into(),
            name: "\u{1b}rename".into(),
            description: "".into(),
        },
    ] {
        assert!(action.validate(target).is_err());
    }
}

#[test]
fn task_cancellation_reserves_only_its_task_and_preserves_original_unknown_reservations() {
    let dir = tempfile::tempdir().unwrap();
    let mut journal = Journal::open(dir.path()).unwrap();
    let pool = PoolId::from(uuid::Uuid::new_v4());
    let mut original = vm(pool);
    journal.admit(&mut original).unwrap();
    original.state = OperationState::OutcomeUnknown;
    journal.save(&mut original).unwrap();
    let mut cancellation = resource(pool);
    cancellation.action = ResourceAction::CancelTask;
    cancellation.target = ResourceTarget::Task(crate::TaskId::from(uuid::Uuid::new_v4()));
    assert!(journal.resource_admit(&mut cancellation).unwrap());
    let mut duplicate = cancellation.clone();
    duplicate.id = OperationId::new();
    duplicate.revision = 0;
    assert!(matches!(
        journal.resource_admit(&mut duplicate),
        Err(Error::ConflictingPoolOperation)
    ));
    assert_eq!(
        journal.load(original.id).unwrap().unwrap().state,
        OperationState::OutcomeUnknown
    );
    let mut overlap = vm(pool);
    overlap.vm = original.vm;
    assert!(matches!(
        journal.admit(&mut overlap),
        Err(Error::ConflictingOperation)
    ));
    assert!(journal.admit(&mut vm(pool)).unwrap());
    cancellation.state = OperationState::OutcomeUnknown;
    journal.resource_save(&mut cancellation).unwrap();
    journal
        .resource_control(pool, cancellation.id, OperationControl::AcknowledgeUnknown)
        .unwrap();
    assert_eq!(
        journal.load(original.id).unwrap().unwrap().state,
        OperationState::OutcomeUnknown
    );
    assert!(matches!(
        journal.admit(&mut overlap),
        Err(Error::ConflictingOperation)
    ));
}
#[test]
fn storage_baselines_and_positive_trim_proof_are_durable_and_cannot_be_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let journal = Journal::open(dir.path()).unwrap();
    let mut record = resource(PoolId::from(uuid::Uuid::new_v4()));
    record.storage_baseline = Some(crate::storage::StorageBaseline {
        srs: vec![crate::SrId::from(uuid::Uuid::new_v4())],
        pbds: vec![],
    });
    assert!(journal.resource_admit(&mut record).unwrap());
    record.storage_trim_succeeded = true;
    journal.resource_save(&mut record).unwrap();
    let saved = journal.resource_load(record.id).unwrap().unwrap();
    assert!(saved.storage_trim_succeeded);
    assert_eq!(saved.storage_baseline, record.storage_baseline);
    record.storage_trim_succeeded = false;
    assert!(matches!(
        journal.resource_save(&mut record),
        Err(Error::JournalChanged)
    ));
    record.storage_trim_succeeded = true;
    record.storage_baseline = None;
    assert!(matches!(
        journal.resource_save(&mut record),
        Err(Error::JournalChanged)
    ));
}

#[test]
fn pool_completion_receipt_survives_reload_and_late_control_without_being_retracted() {
    let dir = tempfile::tempdir().unwrap();
    let journal = Journal::open(dir.path()).unwrap();
    let mut record = resource(PoolId::from(uuid::Uuid::new_v4()));
    record.target = ResourceTarget::Pool(record.pool);
    record.action = ResourceAction::Pool(crate::PoolOperation::EnableTlsVerification);
    journal.resource_admit(&mut record).unwrap();
    record.state = OperationState::Dispatching;
    journal.resource_save(&mut record).unwrap();
    let mut late = record.clone();
    record.state = OperationState::OutcomeUnknown;
    journal.resource_save(&mut record).unwrap();
    journal
        .resource_control(record.pool, record.id, OperationControl::AcknowledgeUnknown)
        .unwrap();
    late.pool_completed = true;
    let receipt = journal.resource_evidence(&late).unwrap();
    assert!(receipt.pool_completed);
    assert!(receipt.state.terminal());
    drop(journal);
    let journal = Journal::open(dir.path()).unwrap();
    let mut saved = journal.resource_load(record.id).unwrap().unwrap();
    assert!(saved.pool_completed);
    saved.pool_completed = false;
    assert!(matches!(
        journal.resource_save(&mut saved),
        Err(Error::JournalChanged)
    ));
}

#[test]
fn host_step_receipts_survive_cancellation_and_acknowledgement_without_resurrection() {
    use crate::resources::host::{Kind, Plan, Step};
    let dir = tempfile::tempdir().unwrap();
    let journal = Journal::open(dir.path()).unwrap();
    let pool = PoolId::from(uuid::Uuid::new_v4());
    let mut record = resource(pool);
    record.action = ResourceAction::Host(crate::HostOperation::RestartToolstack {
        expected_agent_start_time: "100".into(),
    });
    record.target = ResourceTarget::Host(crate::HostId::from(uuid::Uuid::new_v4()));
    record.host_plan = Some(Plan {
        steps: vec![Step {
            kind: Kind::Restart,
            task: None,
            accepted: false,
            task_succeeded: false,
        }],
    });
    record.steps = vec![OperationStep {
        label: "Restart".into(),
        state: StepState::Dispatching,
        detail: String::new(),
    }];
    record.state = OperationState::Dispatching;
    journal.resource_admit(&mut record).unwrap();
    journal
        .resource_control(pool, record.id, OperationControl::RequestCancellation)
        .unwrap();
    record.host_plan.as_mut().unwrap().steps[0].task = Some("OpaqueRef:restart-task".into());
    record.host_plan.as_mut().unwrap().steps[0].accepted = true;
    let mut current = journal.host_evidence(&record, 0).unwrap();
    assert!(
        !serde_json::to_string(&current)
            .unwrap()
            .contains("OpaqueRef:")
    );
    assert!(
        !serde_json::to_string(&current)
            .unwrap()
            .contains("host_plan")
    );
    assert!(current.cancel_requested);
    assert_eq!(current.host_plan, record.host_plan);
    current.state = OperationState::OutcomeUnknown;
    journal.resource_save(&mut current).unwrap();
    journal
        .resource_control(pool, record.id, OperationControl::AcknowledgeUnknown)
        .unwrap();
    record.host_plan.as_mut().unwrap().steps[0].task_succeeded = true;
    let current = journal.host_evidence(&record, 0).unwrap();
    assert_eq!(current.state, OperationState::AcknowledgedUnknown);
    assert!(current.host_plan.as_ref().unwrap().steps[0].task_succeeded);
    let mut replacement = current.clone();
    replacement.host_plan.as_mut().unwrap().steps[0].task = Some("OpaqueRef:other-task".into());
    assert!(matches!(
        journal.resource_save(&mut replacement),
        Err(Error::JournalChanged)
    ));
    let mut rollback = current;
    rollback.host_plan.as_mut().unwrap().steps[0].task_succeeded = false;
    assert!(matches!(
        journal.resource_save(&mut rollback),
        Err(Error::JournalChanged)
    ));
}

#[test]
fn appliance_creation_baseline_and_receipt_cannot_be_retracted_after_reload() {
    let dir = tempfile::tempdir().unwrap();
    let journal = Journal::open(dir.path()).unwrap();
    let mut record = resource(PoolId::from(uuid::Uuid::new_v4()));
    record.target = ResourceTarget::Pool(record.pool);
    record.action = ResourceAction::Appliance(crate::ApplianceOperation::Create {
        name: "Group".into(),
        description: "".into(),
    });
    record.appliance_baseline = Some(vec![crate::ApplianceId::from(uuid::Uuid::new_v4())]);
    journal.resource_admit(&mut record).unwrap();
    record.appliance_completed = true;
    journal.resource_evidence(&record).unwrap();
    drop(journal);
    let journal = Journal::open(dir.path()).unwrap();
    let saved = journal.resource_load(record.id).unwrap().unwrap();
    assert!(saved.appliance_completed);
    assert_eq!(saved.appliance_baseline, record.appliance_baseline);
    let mut modified = saved.clone();
    modified.appliance_baseline = None;
    assert!(matches!(
        journal.resource_save(&mut modified),
        Err(Error::JournalChanged)
    ));
    let mut modified = saved;
    modified.appliance_completed = false;
    assert!(matches!(
        journal.resource_save(&mut modified),
        Err(Error::JournalChanged)
    ));
}

fn join(source: PoolId, destination: PoolId) -> ResourceOperationRecord {
    let mut record = resource(source);
    record.target = ResourceTarget::Pool(source);
    record.action = ResourceAction::Pool(crate::PoolOperation::Join {
        source_host: crate::HostId::from(uuid::Uuid::new_v4()),
        destination_pool: destination,
        destination_coordinator: crate::HostId::from(uuid::Uuid::new_v4()),
        destination_address: "192.0.2.20".into(),
    });
    record
}
fn cross_pool(source: PoolId, destination: PoolId) -> ResourceOperationRecord {
    let mut record = resource(source);
    record.steps.clear();
    record.target = ResourceTarget::Pool(source);
    record.action = ResourceAction::CrossPoolVm(crate::CrossPoolVmOperation {
        vm: uuid::Uuid::new_v4().into(),
        destination_pool: destination,
        destination_host: uuid::Uuid::new_v4().into(),
        expected_host: None,
        expected_affinity: None,
        transfer_network: uuid::Uuid::new_v4().into(),
        transfer_address: "192.0.2.20".into(),
        mode: crate::CrossPoolMode::MoveHalted,
        disks: vec![],
        networks: vec![],
        host_transport_reviewed: true,
    });
    record
}
#[test]
fn cross_pool_workflows_reserve_both_pools_against_writes_in_every_direction() {
    for transfer in [
        join as fn(PoolId, PoolId) -> ResourceOperationRecord,
        cross_pool,
    ] {
        for destination_first in [false, true] {
            for resource_first in [false, true] {
                let dir = tempfile::tempdir().unwrap();
                let mut journal = Journal::open(dir.path()).unwrap();
                let a = PoolId::from(uuid::Uuid::new_v4());
                let b = PoolId::from(uuid::Uuid::new_v4());
                let selected = if destination_first { b } else { a };
                if resource_first {
                    journal.resource_admit(&mut resource(selected)).unwrap();
                } else {
                    journal.admit(&mut vm(selected)).unwrap();
                }
                assert!(matches!(
                    journal.resource_admit(&mut transfer(a, b)),
                    Err(Error::ConflictingPoolOperation)
                ));
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let mut journal = Journal::open(dir.path()).unwrap();
        let a = PoolId::from(uuid::Uuid::new_v4());
        let b = PoolId::from(uuid::Uuid::new_v4());
        let mut record = transfer(a, b);
        journal.resource_admit(&mut record).unwrap();
        for pool in [a, b] {
            assert!(matches!(
                journal.admit(&mut vm(pool)),
                Err(Error::ConflictingPoolOperation)
            ));
            assert!(matches!(
                journal.resource_admit(&mut resource(pool)),
                Err(Error::ConflictingPoolOperation)
            ));
            assert_eq!(journal.resource_history(pool).unwrap()[0].id, record.id);
            assert_eq!(
                journal
                    .resource_control(pool, record.id, OperationControl::Recheck)
                    .unwrap()
                    .id,
                record.id
            );
        }
        let c = PoolId::from(uuid::Uuid::new_v4());
        assert!(journal.admit(&mut vm(c)).unwrap());
        assert!(matches!(
            journal.resource_control(c, record.id, OperationControl::RequestCancellation),
            Err(Error::OperationConflict)
        ));
        record = journal
            .resource_control(b, record.id, OperationControl::RequestCancellation)
            .unwrap();
        assert!(record.cancel_requested);
        record.state = OperationState::OutcomeUnknown;
        journal.resource_save(&mut record).unwrap();
        journal
            .resource_control(b, record.id, OperationControl::AcknowledgeUnknown)
            .unwrap();
        for pool in [a, b] {
            assert!(journal.admit(&mut vm(pool)).unwrap());
        }
        let mut late = record;
        late.pool_completed = true;
        let saved = journal.resource_evidence(&late).unwrap();
        assert_eq!(saved.state, OperationState::AcknowledgedUnknown);
        assert!(saved.pool_completed);
    }
}
#[test]
fn active_malformed_or_inconsistent_scopes_fail_closed_even_for_other_pools() {
    for corrupt in [
        "unknown-action",
        "missing-destination",
        "invalid-destination",
        "wrong-pool",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let mut journal = Journal::open(dir.path()).unwrap();
        let a = PoolId::from(uuid::Uuid::new_v4());
        let b = PoolId::from(uuid::Uuid::new_v4());
        let mut record = join(a, b);
        journal.resource_admit(&mut record).unwrap();
        let mut payload = serde_json::to_value(&record).unwrap();
        match corrupt {
            "unknown-action" => payload["action"] = json!("LegacyJoinWithoutScope"),
            "missing-destination" => {
                payload["action"]["Pool"]["Join"]
                    .as_object_mut()
                    .unwrap()
                    .remove("destination_pool");
            }
            "invalid-destination" => {
                payload["action"]["Pool"]["Join"]["destination_pool"] = json!(a)
            }
            _ => payload["pool"] = json!(b),
        }
        journal
            .0
            .execute(
                "UPDATE resource_operations SET payload=?1 WHERE id=?2",
                rusqlite::params![payload.to_string(), record.id.to_string()],
            )
            .unwrap();
        let c = PoolId::from(uuid::Uuid::new_v4());
        assert!(
            matches!(journal.admit(&mut vm(c)), Err(Error::Journal)),
            "{corrupt}"
        );
        assert!(
            matches!(
                journal.resource_admit(&mut resource(c)),
                Err(Error::Journal)
            ),
            "{corrupt}"
        );
    }
}

#[test]
fn power_batch_reserves_whole_pool_against_vm_and_resource_writes_in_both_directions() {
    let dir = tempfile::tempdir().unwrap();
    let mut journal = Journal::open(dir.path()).unwrap();
    let pool = PoolId::from(uuid::Uuid::new_v4());
    let mut batch = resource(pool);
    batch.target = ResourceTarget::Pool(pool);
    batch.steps.clear();
    batch.action = ResourceAction::PowerBatch(crate::VmPowerBatch {
        action: crate::PowerAction::Start,
        vms: vec![crate::BatchVmPower {
            vm: uuid::Uuid::new_v4().into(),
            expected_state: crate::PowerState::Halted,
        }],
    });
    assert!(journal.resource_admit(&mut batch).unwrap());
    assert!(matches!(
        journal.admit(&mut vm(pool)),
        Err(Error::ConflictingPoolOperation)
    ));
    assert!(matches!(
        journal.resource_admit(&mut resource(pool)),
        Err(Error::ConflictingPoolOperation)
    ));
    batch.state = OperationState::Cancelled;
    journal.resource_save(&mut batch).unwrap();
    let mut running = vm(pool);
    journal.admit(&mut running).unwrap();
    let mut another = batch.clone();
    another.id = OperationId::new();
    another.state = OperationState::Preparing;
    another.revision = 0;
    assert!(matches!(
        journal.resource_admit(&mut another),
        Err(Error::ConflictingPoolOperation)
    ));
    running.state = OperationState::Succeeded;
    journal.save(&mut running).unwrap();
    journal.resource_admit(&mut resource(pool)).unwrap();
    assert!(matches!(
        journal.resource_admit(&mut another),
        Err(Error::ConflictingPoolOperation)
    ));
}

fn planned_cross_pool(source: PoolId, destination: PoolId) -> ResourceOperationRecord {
    let mut record = cross_pool(source, destination);
    let ResourceAction::CrossPoolVm(operation) = &record.action else {
        unreachable!()
    };
    record.cross_pool_plan=Some(Box::new(serde_json::from_value(json!({
        "source_pool":source,"operation":operation,"graph":{"disks":[],"nics":[]},
        "shape":{"name":"Synthetic guest","description":"","cpus":[2,2],"memory":[16777216,1073741824,1073741824,1073741824],"firmware":"bios","boot_order":"dc","secureboot":"false"},
        "baseline":{"VM":[],"VDI":[],"VBD":[],"VIF":[]},"source_tasks":null,"task_uuid":null,"expires_at_ms":10000,
        "completed":false,"ended":false,"destination_vm":null
    })).unwrap()));
    record
}
#[test]
fn cross_pool_private_plan_corruption_cannot_change_reconciliation_identity_or_admission_scope() {
    for corrupt in [
        "source_pool",
        "vm",
        "destination_pool",
        "missing_baseline",
        "extra_baseline",
        "graph",
        "completed",
        "ended",
        "task_uuid",
        "created_vm",
        "missing_plan",
        "unexpected_plan",
        "baseline_collision",
        "premature_success",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let mut journal = Journal::open(dir.path()).unwrap();
        let source = PoolId::from(uuid::Uuid::new_v4());
        let dest = PoolId::from(uuid::Uuid::new_v4());
        let mut record = planned_cross_pool(source, dest);
        record.steps.push(OperationStep {
            label: "Transfer".into(),
            state: StepState::Pending,
            detail: String::new(),
        });
        journal.resource_admit(&mut record).unwrap();
        let text: String = journal
            .0
            .query_row(
                "SELECT payload FROM resource_operations WHERE id=?1",
                [record.id.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        let mut payload: serde_json::Value = serde_json::from_str(&text).unwrap();
        match corrupt {
            "source_pool" => payload["cross_pool_plan"]["source_pool"] = json!(dest),
            "baseline_collision" => {
                payload["cross_pool_plan"]["baseline"]["VM"] =
                    json!([payload["cross_pool_plan"]["operation"]["vm"].clone()])
            }
            "premature_success" => payload["state"] = json!(OperationState::Succeeded),
            "vm" => payload["cross_pool_plan"]["operation"]["vm"] = json!(uuid::Uuid::new_v4()),
            "destination_pool" => {
                payload["cross_pool_plan"]["operation"]["destination_pool"] =
                    json!(uuid::Uuid::new_v4())
            }
            "missing_baseline" => {
                payload["cross_pool_plan"]["baseline"]
                    .as_object_mut()
                    .unwrap()
                    .remove("VDI");
            }
            "extra_baseline" => payload["cross_pool_plan"]["baseline"]["unexpected"] = json!([]),
            "graph" => {
                payload["cross_pool_plan"]["graph"]["nics"] = json!([{"id":uuid::Uuid::new_v4(),"network":uuid::Uuid::new_v4(),"device":"0","mac":"02:00:00:00:00:01"}])
            }
            "completed" => payload["cross_pool_plan"]["completed"] = json!(true),
            "ended" => payload["cross_pool_plan"]["ended"] = json!(true),
            "task_uuid" => payload["cross_pool_plan"]["task_uuid"] = json!(uuid::Uuid::new_v4()),
            "created_vm" => payload["transfer_created"]["vms"] = json!([uuid::Uuid::new_v4()]),
            "missing_plan" => {
                payload.as_object_mut().unwrap().remove("cross_pool_plan");
            }
            "unexpected_plan" => payload["action"] = json!(ResourceAction::CancelTask),
            _ => unreachable!(),
        }
        journal
            .0
            .execute(
                "UPDATE resource_operations SET payload=?1 WHERE id=?2",
                rusqlite::params![payload.to_string(), record.id.to_string()],
            )
            .unwrap();
        assert!(
            matches!(journal.resource_load(record.id), Err(Error::Journal)),
            "{corrupt}"
        );
        assert!(
            matches!(
                journal.resource_control(source, record.id, OperationControl::Recheck),
                Err(Error::Journal)
            ),
            "{corrupt}"
        );
        assert!(
            matches!(journal.admit(&mut vm(source)), Err(Error::Journal)),
            "{corrupt}"
        );
        assert!(
            matches!(
                journal.resource_admit(&mut resource(dest)),
                Err(Error::Journal)
            ),
            "{corrupt}"
        );
        assert!(
            matches!(
                journal.resource_pool_busy(PoolId::from(uuid::Uuid::new_v4())),
                Err(Error::Journal)
            ),
            "{corrupt}"
        );
    }
}

#[test]
fn cross_pool_save_requires_a_plan_before_dispatch_or_success() {
    for state in [
        OperationState::Dispatching,
        OperationState::Tracking,
        OperationState::Verifying,
        OperationState::Succeeded,
    ] {
        let dir = tempfile::tempdir().unwrap();
        let journal = Journal::open(dir.path()).unwrap();
        let mut record = cross_pool(
            PoolId::from(uuid::Uuid::new_v4()),
            PoolId::from(uuid::Uuid::new_v4()),
        );
        record.state = state.clone();
        assert!(
            matches!(journal.resource_admit(&mut record), Err(Error::Journal)),
            "{state:?}"
        );
        assert!(journal.resource_load(record.id).unwrap().is_none());
    }
}

fn authenticated_resource(pool: PoolId) -> ResourceOperationRecord {
    use crate::{
        StorageAuthentication, StorageLocation, StorageOperation,
        storage_auth::{Plan, SecretKind, SecretStep},
    };
    let mut r = resource(pool);
    r.target = ResourceTarget::Pool(pool);
    let operation = StorageOperation::Create {
        host: crate::HostId::from(uuid::Uuid::new_v4()),
        name: "Fixture".into(),
        description: String::new(),
        location: StorageLocation::Iscsi {
            portal: "storage.example.test".into(),
            port: 3260,
            target_iqn: "iqn.2026-01.example:fixture".into(),
            scsi_id: "3600fixture".into(),
        },
    };
    r.action = ResourceAction::AuthenticatedStorage {
        operation: operation.clone(),
        authentication: StorageAuthentication::Chap,
    };
    r.storage_auth_plan = Some(Plan {
        operation,
        authentication: StorageAuthentication::Chap,
        baseline: vec![],
        task_baseline: vec![],
        task_uuid: None,
        secrets: vec![SecretStep {
            kind: SecretKind::Chap,
            username_digest: [1; 32],
            sent: false,
            completed: false,
            id: None,
            cleanup_sent: false,
            cleaned: false,
        }],
        provision_sent: false,
        provision_succeeded: false,
        verified: false,
    });
    r.steps = (0..3)
        .map(|_| OperationStep {
            label: "Fixture".into(),
            state: StepState::Pending,
            detail: String::new(),
        })
        .collect();
    r
}
#[test]
fn storage_auth_journal_rejects_corrupt_private_intent_and_missing_completion_proof() {
    let dir = tempfile::tempdir().unwrap();
    let journal = Journal::open(dir.path()).unwrap();
    let pool = PoolId::from(uuid::Uuid::new_v4());
    let mut r = authenticated_resource(pool);
    journal.resource_save(&mut r).unwrap();
    let mut impossible = r.clone();
    impossible
        .storage_auth_plan
        .as_mut()
        .unwrap()
        .provision_succeeded = true;
    assert!(journal.resource_save(&mut impossible).is_err());
    let payload: String = journal
        .0
        .query_row(
            "SELECT payload FROM resource_operations WHERE id=?1",
            [r.id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    let mut payload: serde_json::Value = serde_json::from_str(&payload).unwrap();
    payload["storage_auth_plan"]["operation"]["Create"]["name"] = json!("Different intent");
    journal
        .0
        .execute(
            "UPDATE resource_operations SET payload=?1 WHERE id=?2",
            rusqlite::params![payload.to_string(), r.id.to_string()],
        )
        .unwrap();
    assert!(journal.resource_load(r.id).is_err());
    assert!(journal.resource_pool_busy(pool).is_err());
}
#[test]
fn storage_auth_journal_merge_preserves_acknowledgement_and_monotonic_secret_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let journal = Journal::open(dir.path()).unwrap();
    let pool = PoolId::from(uuid::Uuid::new_v4());
    let mut r = authenticated_resource(pool);
    r.state = OperationState::Dispatching;
    r.steps[0].state = StepState::Dispatching;
    r.storage_auth_plan.as_mut().unwrap().secrets[0].sent = true;
    journal.resource_save(&mut r).unwrap();
    journal.resource_recover(pool).unwrap();
    journal
        .resource_control(pool, r.id, crate::OperationControl::AcknowledgeUnknown)
        .unwrap();
    let id = crate::SecretId::from(uuid::Uuid::new_v4());
    let secret = &mut r.storage_auth_plan.as_mut().unwrap().secrets[0];
    secret.completed = true;
    secret.id = Some(id);
    let merged = journal.storage_auth_evidence(&r).unwrap();
    assert_eq!(merged.state, OperationState::AcknowledgedUnknown);
    assert_eq!(merged.storage_secrets[0].secret, id);
    let mut changed = merged.clone();
    changed.storage_auth_plan.as_mut().unwrap().secrets[0].username_digest = [2; 32];
    assert!(journal.resource_save(&mut changed).is_err());
    let public = serde_json::to_string(&merged).unwrap();
    assert!(!public.contains("username_digest"));
    assert!(!public.contains("storage_auth_plan"));
}
#[test]
fn host_power_private_plan_is_immutable_bounded_and_cannot_claim_unreceipted_success() {
    use crate::{HostId, HostPowerConfiguration, HostPowerOperation, HostPowerSettings, SecretId};
    let dir = tempfile::tempdir().unwrap();
    let journal = Journal::open(dir.path()).unwrap();
    let pool = PoolId::from(uuid::Uuid::new_v4());
    let mut row = resource(pool);
    row.target = ResourceTarget::Host(HostId::from(uuid::Uuid::new_v4()));
    let operation = HostPowerOperation::Configure {
        expected: HostPowerConfiguration {
            settings: HostPowerSettings::Disabled,
            secret: None,
        },
        settings: HostPowerSettings::Ipmi {
            address: "192.0.2.60".parse().unwrap(),
            username: "fixture".into(),
        },
    };
    row.action = ResourceAction::HostPower(operation.clone());
    row.steps = vec![];
    journal.resource_admit(&mut row).unwrap();
    row.steps = vec![
        OperationStep {
            label: "Secret".into(),
            state: StepState::Pending,
            detail: String::new(),
        },
        OperationStep {
            label: "Settings".into(),
            state: StepState::Pending,
            detail: String::new(),
        },
    ];
    row.host_power_plan = Some(crate::host_power::Plan::new(operation, vec![]));
    journal.resource_save(&mut row).unwrap();
    let mut invalid = row.clone();
    invalid.state = OperationState::Succeeded;
    assert!(matches!(
        journal.resource_save(&mut invalid),
        Err(Error::Journal)
    ));
    let mut invalid = row.clone();
    invalid
        .host_power_plan
        .as_mut()
        .unwrap()
        .configure_completed = true;
    assert!(matches!(
        journal.resource_save(&mut invalid),
        Err(Error::Journal)
    ));
    let mut invalid = row.clone();
    invalid.steps.clear();
    assert!(matches!(
        journal.resource_save(&mut invalid),
        Err(Error::Journal)
    ));
    row.host_power_plan.as_mut().unwrap().secret_sent = true;
    row.state = OperationState::Dispatching;
    journal.resource_save(&mut row).unwrap();
    let mut invalid = row.clone();
    invalid.host_power_plan.as_mut().unwrap().secret_sent = false;
    assert!(matches!(
        journal.resource_save(&mut invalid),
        Err(Error::JournalChanged)
    ));
    row.host_power_plan.as_mut().unwrap().secret_completed = true;
    row.host_power_plan.as_mut().unwrap().secret = Some(SecretId::from(uuid::Uuid::new_v4()));
    crate::host_power::project(&mut row);
    journal.resource_save(&mut row).unwrap();
    let public = serde_json::to_string(&row).unwrap();
    assert!(!public.contains("host_power_plan"));
    let mut invalid = row.clone();
    invalid
        .host_power_plan
        .as_mut()
        .unwrap()
        .baseline
        .push(row.host_power_secret.as_ref().unwrap().secret);
    assert!(matches!(
        journal.resource_save(&mut invalid),
        Err(Error::Journal)
    ));
    let mut recovered = row.clone();
    recovered.state = OperationState::OutcomeUnknown;
    journal.resource_save(&mut recovered).unwrap();
    recovered.state = OperationState::Verifying;
    journal.resource_save(&mut recovered).unwrap();
    let merged = journal.host_power_evidence(&row).unwrap();
    assert_eq!(merged.state, OperationState::Verifying);
    assert!(merged.interrupted);
    assert_eq!(merged.host_power_secret, row.host_power_secret);
}

#[test]
fn topology_private_plan_corruption_refuses_recovery_and_new_admission() {
    for corrupt in [
        "pool",
        "network",
        "operation",
        "baseline",
        "kind",
        "completed",
        "result",
        "created",
        "success",
        "missing_plan",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let mut journal = Journal::open(dir.path()).unwrap();
        let pool = PoolId::from(uuid::Uuid::new_v4());
        let mut record = resource(pool);
        let ResourceTarget::Network(network) = record.target else {
            panic!()
        };
        let uplink = crate::VlanUplink {
            pif: uuid::Uuid::new_v4().into(),
            expected: crate::PifIdentity {
                management: false,
                host: uuid::Uuid::new_v4().into(),
                network: uuid::Uuid::new_v4().into(),
                device: "eth1".into(),
                mac: "02:00:00:00:00:01".into(),
            },
        };
        let operation = crate::NetworkTopologyOperation::CreateVlans {
            uplinks: vec![uplink.clone()],
            tag: 100,
        };
        let kind = crate::network_topology::Kind::Create {
            uplink: uplink.clone(),
            tag: 100,
        };
        record.steps = vec![OperationStep {
            label: kind.label(),
            state: StepState::Pending,
            detail: String::new(),
        }];
        record.action = ResourceAction::NetworkTopology(operation.clone());
        record.network_topology_plan = Some(crate::network_topology::Plan {
            pool,
            network,
            operation,
            vlans_before: Default::default(),
            pifs_before: [uplink.pif].into(),
            tasks_before: Default::default(),
            steps: vec![crate::network_topology::Receipt {
                kind,
                task: None,
                task_uuid: None,
                result: None,
                completed: false,
                ended: false,
                cancel_sent: false,
                created: None,
                verified: false,
            }],
            expires_at_ms: 1000,
        });
        journal.resource_admit(&mut record).unwrap();
        let text: String = journal
            .0
            .query_row(
                "SELECT payload FROM resource_operations WHERE id=?1",
                [record.id.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        let mut payload: serde_json::Value = serde_json::from_str(&text).unwrap();
        match corrupt {
            "pool" => payload["network_topology_plan"]["pool"] = json!(uuid::Uuid::new_v4()),
            "network" => payload["network_topology_plan"]["network"] = json!(uuid::Uuid::new_v4()),
            "operation" => {
                payload["network_topology_plan"]["operation"]["CreateVlans"]["tag"] = json!(101)
            }
            "baseline" => payload["network_topology_plan"]["pifs_before"] = json!([]),
            "kind" => {
                payload["network_topology_plan"]["steps"][0]["kind"]["Create"]["tag"] = json!(101)
            }
            "completed" => payload["network_topology_plan"]["steps"][0]["completed"] = json!(true),
            "result" => {
                payload["network_topology_plan"]["steps"][0]["result"] = json!("OpaqueRef:foreign")
            }
            "created" => {
                payload["topology_changes"]["created_vlans"] = json!([uuid::Uuid::new_v4()])
            }
            "success" => payload["state"] = json!(OperationState::Succeeded),
            "missing_plan" => {
                payload
                    .as_object_mut()
                    .unwrap()
                    .remove("network_topology_plan");
            }
            _ => unreachable!(),
        }
        journal
            .0
            .execute(
                "UPDATE resource_operations SET payload=?1 WHERE id=?2",
                rusqlite::params![payload.to_string(), record.id.to_string()],
            )
            .unwrap();
        assert!(
            matches!(journal.resource_load(record.id), Err(Error::Journal)),
            "{corrupt}"
        );
        assert!(
            matches!(
                journal.resource_control(pool, record.id, OperationControl::Recheck),
                Err(Error::Journal)
            ),
            "{corrupt}"
        );
        assert!(
            matches!(journal.admit(&mut vm(pool)), Err(Error::Journal)),
            "{corrupt}"
        );
    }
}
