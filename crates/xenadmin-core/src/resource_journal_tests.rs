use super::*;
use crate::{
    NetworkId, OperationStep, ResourceAction, ResourceOperationRecord, ResourceTarget, StepState,
};

fn resource(pool: PoolId) -> ResourceOperationRecord {
    ResourceOperationRecord {
        snapshot_preview_plan: None,
        alarm_refresh_receipts: Vec::new(),
        crashdump_deletion: None,
        storage_maintenance_plan: None,
        disk_plan: None,
        disk_size: None,
        management_network: None,
        management_network_plan: None,
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
        root_password_plan: None,
        root_password: None,
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

#[test]
fn export_publication_after_validation_is_fenced_by_recovery_ack_cancel_and_file_identity() {
    use crate::transfer::{Plan, files};
    use sha2::{Digest, Sha256};
    use std::{io::Write, os::unix::fs::PermissionsExt};
    for mode in [
        "complete",
        "recover",
        "ack",
        "cancel",
        "closed",
        "replace-source",
        "existing-destination",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("output");
        std::fs::create_dir(&output).unwrap();
        let journal = Journal::open(dir.path()).unwrap();
        let pool = PoolId::from(uuid::Uuid::new_v4());
        let mut row = resource(pool);
        row.target = ResourceTarget::Pool(pool);
        row.action = ResourceAction::Transfer(crate::TransferAction::ExportDisk {
            vdi: uuid::Uuid::new_v4().into(),
            expected_sr: uuid::Uuid::new_v4().into(),
            format: crate::DiskImageFormat::Raw,
            expected_size: 512,
        });
        let destination = output.join("image.raw");
        let mut plan = Plan::new(crate::TransferFile::destination(&destination).unwrap());
        files::create_output(&mut plan, row.id).unwrap();
        let mut file = files::output(&plan).unwrap();
        let bytes = vec![9u8; 512];
        file.write_all(&bytes).unwrap();
        file.sync_all().unwrap();
        plan.stream_receipt =
            Some(files::receipt(&file, 512, Sha256::digest(&bytes).into()).unwrap());
        plan.dispatched = true;
        plan.task_success = true;
        plan.task_uuid = Some(uuid::Uuid::new_v4());
        row.transfer_plan = Some(Box::new(plan.clone()));
        row.task = Some("OpaqueRef:fixture-export-task".into());
        journal.resource_admit(&mut row).unwrap();
        row.state = OperationState::Verifying;
        journal.resource_save(&mut row).unwrap();
        let expected = row.clone();
        let directory = dir.path().to_owned();
        let cancel = tokio_util::sync::CancellationToken::new();
        let worker_cancel = cancel.clone();
        let (ready_send, ready_receive) = std::sync::mpsc::channel();
        let (release_send, release_receive) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let publication = files::prepare_publication(&plan).unwrap();
            ready_send.send(()).unwrap();
            release_receive.recv().unwrap();
            Journal::open(&directory)
                .unwrap()
                .transfer_publication_guard(&expected, false, &worker_cancel, || {
                    publication.publish()
                })
        });
        ready_receive.recv().unwrap();
        match mode {
            "recover" | "ack" => {
                row.state = OperationState::OutcomeUnknown;
                journal.resource_save(&mut row).unwrap();
                if mode == "ack" {
                    journal
                        .resource_control(pool, row.id, crate::OperationControl::AcknowledgeUnknown)
                        .unwrap();
                }
            }
            "cancel" => {
                row.cancel_requested = true;
                journal.resource_save(&mut row).unwrap();
            }
            "closed" => cancel.cancel(),
            "replace-source" => {
                std::fs::write(
                    row.transfer_plan
                        .as_ref()
                        .unwrap()
                        .temporary
                        .as_ref()
                        .unwrap(),
                    vec![8u8; 512],
                )
                .unwrap();
            }
            "existing-destination" => {
                std::fs::write(&destination, b"operator-owned existing data").unwrap();
            }
            _ => {}
        }
        release_send.send(()).unwrap();
        let result = worker.join().unwrap();
        if mode == "complete" {
            assert!(result.is_ok(), "{result:?}");
            assert_eq!(std::fs::read(&destination).unwrap(), bytes);
            assert_eq!(
                destination.metadata().unwrap().permissions().mode() & 0o777,
                0o600
            );
        } else {
            assert!(result.is_err(), "{mode} must prevent publication");
            if mode == "existing-destination" {
                assert_eq!(
                    std::fs::read(&destination).unwrap(),
                    b"operator-owned existing data"
                );
            } else {
                assert!(!destination.exists(), "{mode}");
            }
        }
    }
}

#[test]
fn root_password_journal_rejects_changed_scope_and_retains_late_completion_after_ack() {
    let dir = tempfile::tempdir().unwrap();
    let journal = Journal::open(dir.path()).unwrap();
    let pool = PoolId::from(uuid::Uuid::new_v4());
    let host = crate::HostId::from(uuid::Uuid::new_v4());
    let change = crate::PoolRootPasswordChange {
        coordinator: host,
        members: vec![host],
    };
    let mut row = resource(pool);
    row.target = ResourceTarget::Pool(pool);
    row.action = ResourceAction::RootPassword(change.clone());
    row.steps = (0..3)
        .map(|i| OperationStep {
            label: format!("Step {i}"),
            state: StepState::Pending,
            detail: String::new(),
        })
        .collect();
    row.root_password_plan = Some(crate::root_password::Plan {
        expected: change,
        sent: false,
        completed: false,
        login_verified: false,
        credentials_updated: false,
    });
    crate::root_password::project(&mut row);
    journal.resource_admit(&mut row).unwrap();
    let mut bad = row.clone();
    bad.root_password_plan
        .as_mut()
        .unwrap()
        .expected
        .members
        .push(crate::HostId::from(uuid::Uuid::new_v4()));
    assert!(journal.resource_save(&mut bad).is_err());
    row.root_password_plan.as_mut().unwrap().sent = true;
    row.state = OperationState::Dispatching;
    row.steps[0].state = StepState::Dispatching;
    journal.resource_save(&mut row).unwrap();
    let mut late = row.clone();
    row.state = OperationState::OutcomeUnknown;
    journal.resource_save(&mut row).unwrap();
    row = journal
        .resource_control(pool, row.id, crate::OperationControl::AcknowledgeUnknown)
        .unwrap();
    late.root_password_plan.as_mut().unwrap().completed = true;
    late.state = OperationState::Verifying;
    late.steps[0].state = StepState::Verified;
    let merged = journal.root_password_evidence(&late).unwrap();
    assert_eq!(merged.state, row.state);
    assert!(merged.root_password.as_ref().unwrap().coordinator_changed);
    assert!(
        !merged
            .root_password
            .as_ref()
            .unwrap()
            .reconnect_credentials_updated
    );
    let mut cleared = merged.clone();
    cleared.root_password_plan.as_mut().unwrap().completed = false;
    crate::root_password::project(&mut cleared);
    assert!(journal.resource_save(&mut cleared).is_err());
    let public = serde_json::to_string(&merged).unwrap();
    assert!(!public.contains("root_password_plan"));
    let bad_payload = serde_json::json!({"unexpected":true});
    journal.0.execute("UPDATE resource_operations SET payload=json_set(payload,'$.root_password_plan',json(?1)) WHERE id=?2",rusqlite::params![bad_payload.to_string(),row.id.to_string()]).unwrap();
    assert!(journal.resource_load(row.id).is_err());
}

#[test]
fn root_password_rekey_guard_serializes_recovery_and_refuses_unknown_or_acknowledged_rows() {
    let dir = tempfile::tempdir().unwrap();
    let journal = Journal::open(dir.path()).unwrap();
    let pool = PoolId::from(uuid::Uuid::new_v4());
    let host = crate::HostId::from(uuid::Uuid::new_v4());
    let change = crate::PoolRootPasswordChange {
        coordinator: host,
        members: vec![host],
    };
    let mut row = resource(pool);
    row.target = ResourceTarget::Pool(pool);
    row.action = ResourceAction::RootPassword(change.clone());
    row.steps = (0..3)
        .map(|i| OperationStep {
            label: format!("Step {i}"),
            state: StepState::Pending,
            detail: String::new(),
        })
        .collect();
    row.root_password_plan = Some(crate::root_password::Plan {
        expected: change,
        sent: false,
        completed: false,
        login_verified: false,
        credentials_updated: false,
    });
    crate::root_password::project(&mut row);
    journal.resource_admit(&mut row).unwrap();
    row.state = OperationState::Verifying;
    let p = row.root_password_plan.as_mut().unwrap();
    p.sent = true;
    p.completed = true;
    p.login_verified = true;
    crate::root_password::project(&mut row);
    journal.resource_save(&mut row).unwrap();
    let other = Journal::open(dir.path()).unwrap();
    other.0.busy_timeout(std::time::Duration::ZERO).unwrap();
    let mut replaced = false;
    journal
        .root_password_credential_guard(row.id, pool, || {
            // Recovery cannot enter the validated-row/local-rekey window on another connection.
            let mut stale = other.resource_load(row.id)?.unwrap();
            stale.state = OperationState::OutcomeUnknown;
            assert!(other.resource_save(&mut stale).is_err());
            replaced = true;
            Ok(())
        })
        .unwrap();
    assert!(replaced);
    row.state = OperationState::OutcomeUnknown;
    journal.resource_save(&mut row).unwrap();
    for acknowledge in [false, true] {
        if acknowledge {
            journal
                .resource_control(pool, row.id, crate::OperationControl::AcknowledgeUnknown)
                .unwrap();
        }
        let mut replaced = false;
        assert!(
            journal
                .root_password_credential_guard(row.id, pool, || {
                    replaced = true;
                    Ok(())
                })
                .is_err()
        );
        assert!(
            !replaced,
            "Recovery/ack must prevent the subsequent local credential replacement"
        );
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
        hba_device: None,
        local_device: None,
        host: crate::HostId::from(uuid::Uuid::new_v4()),
        name: "Fixture".into(),
        description: String::new(),
        location: StorageLocation::Iscsi {
            portal: "storage.example.test".into(),
            port: 3260,
            target_iqn: "iqn.2026-01.example:fixture".into(),
            scsi_id: "3600fixture".into(),
            allocation: crate::StorageAllocation::DriverDefault,
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
fn management_record(pool: PoolId) -> ResourceOperationRecord {
    let host = crate::HostId::from(uuid::Uuid::new_v4());
    let interface = crate::ManagementInterface {
        topology: None,
        pif: uuid::Uuid::new_v4().into(),
        identity: crate::PifIdentity {
            host,
            network: uuid::Uuid::new_v4().into(),
            device: "eth0".into(),
            mac: "02:00:00:00:00:01".into(),
            management: true,
        },
        primary: crate::ManagementAddressFamily::Ipv4,
        ipv4: crate::Ipv4Configuration {
            mode: crate::Ipv4Mode::Static,
            address: "192.0.2.10".into(),
            netmask: "255.255.255.0".into(),
            gateway: String::new(),
            dns: String::new(),
        },
        ipv6: crate::Ipv6Configuration {
            mode: crate::Ipv6Mode::None,
            addresses: vec![],
            gateway: String::new(),
            dns: String::new(),
        },
        attached: true,
        protected: false,
    };
    let mut configuration = interface.ipv4.clone();
    configuration.address = "192.0.2.20".into();
    let op = crate::ManagementNetworkOperation::ConfigureIpv4 {
        interface,
        configuration,
    };
    let mut r = resource(pool);
    r.target = ResourceTarget::Host(host);
    r.action = ResourceAction::ManagementNetwork(Box::new(op.clone()));
    r.steps[0].label = op.label().into();
    let endpoint = crate::Endpoint::parse("https://192.0.2.10").unwrap();
    let p = crate::management_network::Plan {
        bond: None,
        pool,
        host,
        recovery: op.endpoint(&endpoint).unwrap(),
        operation: op,
        original_endpoint: endpoint,
        certificate: "aa".repeat(32).parse().unwrap(),
        configuration_observed: false,
        route_verified: false,
        tasks_before: Default::default(),
        task_uuid: None,
        completed: false,
        ended: false,
        registered: false,
        expires_at_ms: 60_000,
    };
    r.management_network = Some(p.evidence());
    r.management_network_plan = Some(p);
    r
}
#[test]
fn management_private_plan_corruption_cannot_change_route_or_scope_or_forge_completion() {
    for corrupt in [
        "host",
        "pool",
        "endpoint",
        "operation",
        "completion",
        "task",
        "public",
        "expiry",
        "missing",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let j = Journal::open(dir.path()).unwrap();
        let pool = PoolId::from(uuid::Uuid::new_v4());
        let mut r = management_record(pool);
        j.resource_admit(&mut r).unwrap();
        let encoded: String =
            j.0.query_row(
                "SELECT payload FROM resource_operations WHERE id=?1",
                [r.id.to_string()],
                |row| row.get(0),
            )
            .unwrap();
        let mut value: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        match corrupt {
            "host" => value["management_network_plan"]["host"] = json!(uuid::Uuid::new_v4()),
            "pool" => value["management_network_plan"]["pool"] = json!(uuid::Uuid::new_v4()),
            "endpoint" => {
                value["management_network_plan"]["recovery"] = json!("https://unreviewed.invalid/")
            }
            "operation" => {
                value["management_network_plan"]["operation"]["ConfigureIpv4"]["configuration"]["address"] =
                    json!("192.0.2.99")
            }
            "completion" => value["management_network_plan"]["completed"] = json!(true),
            "task" => value["management_network_plan"]["task_uuid"] = json!(uuid::Uuid::new_v4()),
            "public" => value["management_network"]["recovery_endpoint_verified"] = json!(true),
            "expiry" => value["management_network_plan"]["expires_at_ms"] = json!(u64::MAX),
            "missing" => {
                value
                    .as_object_mut()
                    .unwrap()
                    .remove("management_network_plan");
            }
            _ => unreachable!(),
        }
        j.0.execute(
            "UPDATE resource_operations SET payload=?1 WHERE id=?2",
            rusqlite::params![value.to_string(), r.id.to_string()],
        )
        .unwrap();
        assert!(
            matches!(j.resource_load(r.id), Err(Error::Journal)),
            "{corrupt}"
        );
        assert!(
            matches!(j.resource_admit(&mut resource(pool)), Err(Error::Journal)),
            "{corrupt}"
        );
    }
}

#[test]
fn alarm_refresh_receipts_are_durable_monotonic_and_bound_to_reviewed_hosts() {
    let dir = tempfile::tempdir().unwrap();
    let journal = Journal::open(dir.path()).unwrap();
    let pool = PoolId::from(uuid::Uuid::new_v4());
    let host = crate::HostId::from(uuid::Uuid::new_v4());
    let mut row = resource(pool);
    row.target = ResourceTarget::Pool(pool);
    row.action = ResourceAction::Alarm(crate::AlarmOperation::Refresh { hosts: vec![host] });
    assert!(journal.resource_admit(&mut row).unwrap());
    row.state = OperationState::Dispatching;
    row.steps[0].state = StepState::Dispatching;
    journal.resource_save(&mut row).unwrap();
    row.alarm_refresh_receipts.push(host);
    row = journal.resource_evidence(&row).unwrap();
    assert_eq!(
        journal
            .resource_load(row.id)
            .unwrap()
            .unwrap()
            .alarm_refresh_receipts,
        vec![host]
    );
    let mut invalid = row.clone();
    invalid.steps[0].state = StepState::Pending;
    assert!(journal.resource_save(&mut invalid).is_err());
    let mut invalid = row.clone();
    invalid.alarm_refresh_receipts.clear();
    assert!(journal.resource_save(&mut invalid).is_err());
    let mut invalid = row.clone();
    invalid
        .alarm_refresh_receipts
        .push(crate::HostId::from(uuid::Uuid::new_v4()));
    assert!(journal.resource_save(&mut invalid).is_err());
    let mut invalid = row.clone();
    invalid.alarm_refresh_receipts.push(host);
    assert!(journal.resource_save(&mut invalid).is_err());
}
fn management_bond_record(pool: PoolId) -> ResourceOperationRecord {
    let mut r = management_record(pool);
    let mut source = r
        .management_network_plan
        .as_ref()
        .unwrap()
        .operation
        .source()
        .clone();
    source.topology = None;
    let mut other = source.clone();
    other.pif = uuid::Uuid::new_v4().into();
    other.identity.management = false;
    other.identity.network = uuid::Uuid::new_v4().into();
    other.ipv4.mode = crate::Ipv4Mode::None;
    other.ipv4.address.clear();
    other.ipv4.netmask.clear();
    other.ipv4.gateway.clear();
    other.ipv4.dns.clear();
    let members = vec![source.clone(), other];
    let pifs = members.iter().map(|p| p.pif).collect();
    let operation = crate::ManagementNetworkOperation::Bond {
        consumers: None,
        source,
        operation: crate::ManagementBondOperation::Create {
            members,
            network: uuid::Uuid::new_v4().into(),
            mode: crate::BondMode::ActiveBackup,
            hash: None,
        },
    };
    r.action = ResourceAction::ManagementNetwork(Box::new(operation.clone()));
    r.steps[0].label = operation.label().into();
    let plan = r.management_network_plan.as_mut().unwrap();
    plan.operation = operation;
    plan.recovery = plan.operation.endpoint(&plan.original_endpoint).unwrap();
    plan.bond = Some(crate::management_bond::Baseline {
        vlans: None,
        replacements: Default::default(),
        pifs,
        bonds: Default::default(),
        created: None,
        properties: Default::default(),
    });
    r.management_network = Some(plan.evidence());
    r
}
#[test]
fn management_bond_baseline_is_private_scope_bound_and_created_evidence_is_monotonic() {
    for corrupt in [
        "missing_member",
        "created_existing",
        "properties",
        "unexpected",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let j = Journal::open(dir.path()).unwrap();
        let pool = PoolId::from(uuid::Uuid::new_v4());
        let mut r = management_bond_record(pool);
        j.resource_admit(&mut r).unwrap();
        let payload: String =
            j.0.query_row(
                "SELECT payload FROM resource_operations WHERE id=?1",
                [r.id.to_string()],
                |row| row.get(0),
            )
            .unwrap();
        let mut v: serde_json::Value = serde_json::from_str(&payload).unwrap();
        match corrupt {
            "missing_member" => v["management_network_plan"]["bond"]["pifs"] = json!([]),
            "created_existing" => {
                let p = r.management_network_plan.as_ref().unwrap();
                v["management_network_plan"]["bond"]["created"] =
                    json!({"bond":uuid::Uuid::new_v4(),"master":p.operation.source().pif});
            }
            "properties" => {
                v["management_network_plan"]["bond"]["properties"] =
                    json!({"oversized":"x".repeat(129)})
            }
            "unexpected" => {
                v["management_network_plan"]["bond"] = serde_json::Value::Null;
            }
            _ => unreachable!(),
        };
        j.0.execute(
            "UPDATE resource_operations SET payload=?1 WHERE id=?2",
            rusqlite::params![v.to_string(), r.id.to_string()],
        )
        .unwrap();
        assert!(j.resource_load(r.id).is_err(), "{corrupt}");
    }
    let dir = tempfile::tempdir().unwrap();
    let j = Journal::open(dir.path()).unwrap();
    let mut r = management_bond_record(uuid::Uuid::new_v4().into());
    j.resource_admit(&mut r).unwrap();
    r.state = OperationState::Dispatching;
    r.steps[0].state = StepState::Dispatching;
    r.management_network_plan.as_mut().unwrap().registered = true;
    r.management_network = Some(r.management_network_plan.as_ref().unwrap().evidence());
    j.resource_save(&mut r).unwrap();
    let mut late = r.clone();
    r.state = OperationState::OutcomeUnknown;
    j.resource_save(&mut r).unwrap();
    late.task = Some("OpaqueRef:bond-task".into());
    late.result_reference = Some("OpaqueRef:new-bond".into());
    let p = late.management_network_plan.as_mut().unwrap();
    p.task_uuid = Some(uuid::Uuid::new_v4().into());
    p.completed = true;
    p.ended = true;
    let created = crate::ManagementBondCreated {
        bond: uuid::Uuid::new_v4().into(),
        master: uuid::Uuid::new_v4().into(),
    };
    p.bond.as_mut().unwrap().created = Some(created);
    late.management_network = Some(p.evidence());
    let mut merged = j.resource_evidence(&late).unwrap();
    assert_eq!(merged.state, OperationState::OutcomeUnknown);
    assert!(merged.interrupted);
    assert_eq!(
        merged.management_network.as_ref().unwrap().created_bond,
        Some(created)
    );
    assert!(
        !serde_json::to_string(&merged)
            .unwrap()
            .contains("OpaqueRef:")
    );
    merged
        .management_network_plan
        .as_mut()
        .unwrap()
        .bond
        .as_mut()
        .unwrap()
        .created = None;
    merged.management_network = Some(merged.management_network_plan.as_ref().unwrap().evidence());
    assert!(j.resource_save(&mut merged).is_err());
}
fn management_vlan_bond_record(pool: PoolId) -> ResourceOperationRecord {
    let mut r = management_bond_record(pool);
    let plan = r.management_network_plan.as_mut().unwrap();
    let crate::ManagementNetworkOperation::Bond {
        source,
        operation: crate::ManagementBondOperation::Create { members, .. },
        ..
    } = &mut plan.operation
    else {
        panic!()
    };
    let vlan = crate::VlanId::from(uuid::Uuid::new_v4());
    let child = crate::PifId::from(uuid::Uuid::new_v4());
    let parent = crate::PhysicalInterface {
        metrics: None,
        capabilities: None,
        managed: Some(true),
        disallow_unplug: Some(false),
        management_purpose: None,
        primary_address_type: Some("IPv4".into()),
        id: source.pif,
        host: Some(source.identity.host),
        network: Some(source.identity.network),
        device: source.identity.device.clone(),
        mac: source.identity.mac.clone(),
        mtu: Some(1500),
        vlan: Some(-1),
        management: Some(true),
        physical: Some(true),
        currently_attached: Some(true),
        ip_configuration_mode: "Static".into(),
        ip: source.ipv4.address.clone(),
        netmask: source.ipv4.netmask.clone(),
        gateway: source.ipv4.gateway.clone(),
        dns: source.ipv4.dns.clone(),
        ipv6: vec![],
        ipv6_configuration_mode: "None".into(),
        ipv6_gateway: String::new(),
    };
    let mut p = parent.clone();
    p.id = child;
    p.network = Some(uuid::Uuid::new_v4().into());
    p.physical = Some(false);
    p.vlan = Some(17);
    p.mac = "fe:ff:ff:ff:ff:ff".into();
    p.management = Some(false);
    p.ip_configuration_mode = "None".into();
    p.ip.clear();
    p.netmask.clear();
    p.gateway.clear();
    p.dns.clear();
    source.topology = Some(Box::new(crate::ManagementTopology {
        pifs: std::collections::BTreeMap::from([(parent.id, parent), (child, p)]),
        bonds: Default::default(),
        vlans: std::collections::BTreeMap::from([(
            vlan,
            crate::Vlan {
                id: vlan,
                tagged_pif: Some(source.pif),
                untagged_pif: Some(child),
                tag: Some(17),
            },
        )]),
    }));
    members[0] = source.clone();
    let baseline = plan.bond.as_mut().unwrap();
    baseline.pifs.insert(child);
    baseline.vlans = Some(std::collections::BTreeSet::from([vlan]));
    r.action = ResourceAction::ManagementNetwork(Box::new(plan.operation.clone()));
    r.management_network = Some(plan.evidence());
    r
}
#[test]
fn management_vlan_replacement_receipts_are_scope_bound_private_and_monotonic() {
    let dir = tempfile::tempdir().unwrap();
    let j = Journal::open(dir.path()).unwrap();
    let mut r = management_vlan_bond_record(uuid::Uuid::new_v4().into());
    j.resource_admit(&mut r).unwrap();
    r.state = OperationState::Dispatching;
    r.steps[0].state = StepState::Dispatching;
    r.management_network_plan.as_mut().unwrap().registered = true;
    r.management_network = Some(r.management_network_plan.as_ref().unwrap().evidence());
    j.resource_save(&mut r).unwrap();
    let mut late = r.clone();
    r.state = OperationState::OutcomeUnknown;
    j.resource_save(&mut r).unwrap();
    late.task = Some("OpaqueRef:vlan-bond-task".into());
    late.result_reference = Some("OpaqueRef:new-bond".into());
    let p = late.management_network_plan.as_mut().unwrap();
    p.completed = true;
    p.ended = true;
    p.task_uuid = Some(uuid::Uuid::new_v4().into());
    let crate::ManagementNetworkOperation::Bond {
        source, operation, ..
    } = &p.operation
    else {
        panic!()
    };
    let expected = operation.captured_vlans(source).unwrap();
    let (old, v) = expected.first_key_value().unwrap();
    let receipt = crate::ManagementVlanReplacement {
        old_vlan: *old,
        old_interface: v.interface.id,
        new_vlan: uuid::Uuid::new_v4().into(),
        new_interface: uuid::Uuid::new_v4().into(),
    };
    p.bond.as_mut().unwrap().created = Some(crate::ManagementBondCreated {
        bond: uuid::Uuid::new_v4().into(),
        master: uuid::Uuid::new_v4().into(),
    });
    p.bond.as_mut().unwrap().replacements.insert(*old, receipt);
    late.management_network = Some(p.evidence());
    let current = j.resource_evidence(&late).unwrap();
    assert_eq!(current.state, OperationState::OutcomeUnknown);
    assert_eq!(
        current.management_network.as_ref().unwrap().moved_vlans,
        vec![receipt]
    );
    assert!(
        !serde_json::to_string(&current)
            .unwrap()
            .contains("management_network_plan")
    );
    let mut rebound = current.clone();
    rebound
        .management_network_plan
        .as_mut()
        .unwrap()
        .bond
        .as_mut()
        .unwrap()
        .replacements
        .get_mut(old)
        .unwrap()
        .new_interface = uuid::Uuid::new_v4().into();
    rebound.management_network = Some(rebound.management_network_plan.as_ref().unwrap().evidence());
    assert!(j.resource_evidence(&rebound).is_err());
    for corrupt in ["baseline", "source", "existing"] {
        let raw: String =
            j.0.query_row(
                "SELECT payload FROM resource_operations WHERE id=?1",
                [current.id.to_string()],
                |row| row.get(0),
            )
            .unwrap();
        let mut payload: serde_json::Value = serde_json::from_str(&raw).unwrap();
        match corrupt {
            "baseline" => payload["management_network_plan"]["bond"]["vlans"] = json!([]),
            "source" => {
                payload["management_network_plan"]["bond"]["replacements"][old.to_string()]["old_interface"] =
                    json!(uuid::Uuid::new_v4())
            }
            "existing" => {
                payload["management_network_plan"]["bond"]["replacements"][old.to_string()]["new_interface"] =
                    json!(receipt.old_interface)
            }
            _ => unreachable!(),
        };
        j.0.execute(
            "UPDATE resource_operations SET payload=?1 WHERE id=?2",
            rusqlite::params![payload.to_string(), current.id.to_string()],
        )
        .unwrap();
        assert!(j.resource_load(current.id).is_err(), "{corrupt}");
        j.0.execute(
            "UPDATE resource_operations SET payload=?1 WHERE id=?2",
            rusqlite::params![raw, current.id.to_string()],
        )
        .unwrap();
    }
}

#[test]
fn management_guest_review_is_durable_immutable_and_rejects_unbound_destinations() {
    let dir = tempfile::tempdir().unwrap();
    let j = Journal::open(dir.path()).unwrap();
    let mut r = management_bond_record(uuid::Uuid::new_v4().into());
    let p = r.management_network_plan.as_mut().unwrap();
    let crate::ManagementNetworkOperation::Bond {
        source,
        operation,
        consumers,
    } = &mut p.operation
    else {
        panic!()
    };
    let (networks, moves, _) = crate::management_bond_consumers::scope(source, operation).unwrap();
    let mut review = crate::ManagementBondConsumers::default();
    for id in networks {
        review.networks.insert(
            id,
            crate::ManagementBondNetwork {
                name: "Reviewed network".into(),
                mtu: 1500,
                default_locking_mode: Some("unlocked".into()),
                settings_fingerprint: [1; 32],
            },
        );
    }
    let id = crate::VifId::from(uuid::Uuid::new_v4());
    review.interfaces.insert(
        id,
        crate::ManagementBondConsumer {
            interface: id,
            vm: uuid::Uuid::new_v4().into(),
            vm_name: "Reviewed guest".into(),
            power_state: "Halted".into(),
            template: false,
            snapshot: false,
            locality: crate::ManagementVmLocality::PossibleHosts(vec![]),
            network: source.identity.network,
            destination: moves[&source.identity.network],
            change: crate::ManagementVifChange::Move,
            device: "0".into(),
            mac: "02:00:00:00:00:01".into(),
            mtu: 1500,
            attached: false,
            settings_fingerprint: [2; 32],
        },
    );
    *consumers = Some(Box::new(review));
    r.action = ResourceAction::ManagementNetwork(Box::new(p.operation.clone()));
    j.resource_admit(&mut r).unwrap();
    let loaded = j.resource_load(r.id).unwrap().unwrap();
    assert_eq!(loaded.action, r.action);
    let mut changed = loaded.clone();
    let crate::ManagementNetworkOperation::Bond {
        consumers: Some(v), ..
    } = &mut changed.management_network_plan.as_mut().unwrap().operation
    else {
        panic!()
    };
    v.interfaces.get_mut(&id).unwrap().settings_fingerprint = [3; 32];
    assert!(j.resource_save(&mut changed).is_err());
    let raw: String =
        j.0.query_row(
            "SELECT payload FROM resource_operations WHERE id=?1",
            [r.id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    for corrupt in ["destination", "locality", "network_policy"] {
        let mut v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        for path in ["action", "management_network_plan"] {
            let operation = if path == "action" {
                &mut v[path]["ManagementNetwork"]["Bond"]
            } else {
                &mut v[path]["operation"]["Bond"]
            };
            match corrupt {
                "destination" => {
                    operation["consumers"]["interfaces"][id.to_string()]["destination"] =
                        json!(uuid::Uuid::new_v4())
                }
                "locality" => {
                    operation["consumers"]["interfaces"][id.to_string()]["locality"] =
                        json!({"PossibleHosts":[uuid::Uuid::new_v4()]})
                }
                _ => operation["consumers"]["networks"] = json!({}),
            }
        }
        j.0.execute(
            "UPDATE resource_operations SET payload=?1 WHERE id=?2",
            rusqlite::params![v.to_string(), r.id.to_string()],
        )
        .unwrap();
        assert!(j.resource_load(r.id).is_err(), "{corrupt}");
    }
}
