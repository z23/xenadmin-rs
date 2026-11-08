use super::*;
use crate::{
    ObservedResource, ResourceTarget, StorageAllocation, StorageAuthentication, StorageLocation,
    StorageOperation,
    storage_auth::{Plan, SecretKind, SecretStep},
};

fn dispatched(journal: &mut Journal) -> ResourceRequest {
    let pool = uuid::Uuid::new_v4().into();
    let operation = StorageOperation::Create {
        host: uuid::Uuid::new_v4().into(),
        hba_device: None,
        local_device: None,
        name: "Fixture storage".into(),
        description: String::new(),
        location: StorageLocation::Iscsi {
            portal: "storage.example.test".into(),
            port: 3260,
            target_iqn: "iqn.2026-01.example:fixture".into(),
            scsi_id: "3600fixture".into(),
            allocation: StorageAllocation::DriverDefault,
        },
    };
    let (_, mut request) = crate::resources::channel(
        OperationId::new(),
        ObservedResource {
            pool,
            generation: 1,
            target: ResourceTarget::Pool(pool),
        },
        ResourceAction::AuthenticatedStorage {
            operation: operation.clone(),
            authentication: StorageAuthentication::Chap,
        },
        tokio::runtime::Handle::current(),
    );
    assert!(journal.resource_admit(&mut request.record).unwrap());
    request.record.storage_auth_plan = Some(Box::new(Plan {
        operation,
        authentication: StorageAuthentication::Chap,
        baseline: vec![],
        task_baseline: vec![],
        task_uuid: None,
        secrets: vec![SecretStep {
            kind: SecretKind::Chap,
            username_digest: [1; 32],
            sent: true,
            completed: false,
            id: None,
            cleanup_sent: false,
            cleaned: false,
        }],
        provision_sent: false,
        provision_succeeded: false,
        verified: false,
    }));
    request.record.steps = vec![
        step("Create temporary storage Secret"),
        step("Provision authenticated storage"),
        step("Remove unused temporary Secret"),
    ];
    request.record.steps[0].state = StepState::Dispatching;
    request.record.state = OperationState::Dispatching;
    journal.resource_save(&mut request.record).unwrap();
    request
}

#[tokio::test]
async fn first_late_evidence_is_not_retirement_and_recheck_after_join_is_known_partial() {
    let directory = tempfile::tempdir().unwrap();
    let mut journal = Journal::open(directory.path()).unwrap();
    let mut original = dispatched(&mut journal);
    let id = original.record.id;
    let pool = original.record.pool;
    let secret = uuid::Uuid::new_v4().into();
    journal.resource_recover(pool).unwrap();
    // The automatic recheck has identified the Secret, but its create reply is held.
    let mut metadata = journal.resource_load(id).unwrap().unwrap();
    metadata.storage_auth_plan.as_mut().unwrap().secrets[0].id = Some(secret);
    journal.storage_auth_evidence(&metadata).unwrap();
    let (published, publication) = tokio::sync::oneshot::channel();
    let (release, released) = tokio::sync::oneshot::channel();
    let worker_directory = directory.path().to_path_buf();
    let worker = tokio::spawn(async move {
        let mut journal = Journal::open(&worker_directory).unwrap();
        original.record.storage_auth_plan.as_mut().unwrap().secrets[0].completed = true;
        assert!(!original.auth_merge(&mut journal).await.unwrap());
        published.send(()).unwrap();
        released.await.unwrap();
        // The late identity read completes after the first evidence publication.
        original.record.storage_auth_plan.as_mut().unwrap().secrets[0].id = Some(secret);
        assert!(!original.auth_merge(&mut journal).await.unwrap());
        original
    });
    publication.await.unwrap();
    let mut recheck = ResourceRequest::tracking(journal.resource_load(id).unwrap().unwrap());
    assert_eq!(recheck.record.state, OperationState::OutcomeUnknown);
    assert_eq!(recheck.record.storage_secrets[0].secret, secret);
    // A reader can finish its classification before the final late-evidence save.
    recheck.record.state = OperationState::PartiallyCompleted;
    release.send(()).unwrap();
    let original = worker.await.unwrap();
    assert_eq!(original.record.state, OperationState::OutcomeUnknown);
    assert!(matches!(
        recheck.save(&mut journal).await,
        Err(Error::JournalChanged)
    ));
    recheck
        .finish_error(&mut journal, Error::JournalChanged)
        .await;
    assert_eq!(recheck.record.state, OperationState::OutcomeUnknown);
    assert!(journal.resource_pool_busy(pool).unwrap());

    // Joining the writer supplies an actual completion boundary. This contrary
    // ordering classifies the positive receipt without provisioning or cleanup.
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen = calls.clone();
    let fixture = crate::xapi::inventory_tests::Fixture::new(move |_| {
        seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        json!({"error":{"message":"MESSAGE_METHOD_UNKNOWN","data":[]}})
    })
    .await;
    let session = fixture.session().await;
    let mut recheck = ResourceRequest::tracking(journal.resource_load(id).unwrap().unwrap());
    recheck
        .reconcile_storage_auth(&mut journal, &session, &CancellationToken::new())
        .await
        .unwrap();
    let result = journal.resource_load(id).unwrap().unwrap();
    assert_eq!(result.state, OperationState::PartiallyCompleted);
    assert!(result.detail.contains("Storage was not submitted"));
    assert_eq!(result.storage_secrets, original.record.storage_secrets);
    assert!(!result.storage_secrets[0].cleaned);
    assert!(!result.storage_auth_plan.unwrap().provision_sent);
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
}
