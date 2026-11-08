use super::*;
use crate::{
    NetworkId, OperationStep, ResourceAction, ResourceOperationRecord, ResourceTarget, StepState,
};

fn resource(pool: PoolId) -> ResourceOperationRecord {
    ResourceOperationRecord {
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
        local_error: None,
        revision: 0,
        task: None,
        result_reference: None,
        cancel_dispatched: false,
        baseline_networks: None,
        network_baseline: None,
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
    journal
        .resource_control(pool, operation.id, OperationControl::AcknowledgeUnknown)
        .unwrap();
    operation.result_reference = Some("OpaqueRef:created-resource".into());
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
