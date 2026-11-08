use crate::*;
use serde_json::json;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

fn policy() -> SnapshotPolicyState {
    SnapshotPolicyState {
        id: Uuid::from_u128(3).into(),
        config: SnapshotPolicyConfig {
            name: "Daily".into(),
            description: "Keep".into(),
            kind: ScheduledSnapshotKind::Disk,
            retained: 2,
            schedule: SnapshotSchedule::Daily { hour: 3, minute: 0 },
        },
        enabled: false,
        members: vec![],
    }
}
fn permissions(methods: &[&str]) -> SessionPermissions {
    SessionPermissions::decode(&json!({"is_local_superuser":false,"rbac_permissions":methods}))
}
#[tokio::test]
async fn resource_review_denial_prevents_the_first_step_when_a_later_write_is_forbidden() {
    let state = policy();
    let mut config = state.config.clone();
    config.name = "Renamed".into();
    config.description = "New details".into();
    let cases = [
        (
            ResourceAction::Appliance(ApplianceOperation::Rename {
                appliance: Uuid::from_u128(3).into(),
                previous_name: "Old".into(),
                previous_description: "Before".into(),
                name: "New".into(),
                description: "After".into(),
            }),
            vec!["vm_appliance.set_name_label"],
        ),
        (
            ResourceAction::Appliance(ApplianceOperation::SetOrder {
                appliance: Uuid::from_u128(3).into(),
                vm: Uuid::from_u128(4).into(),
                previous: ApplianceOrder {
                    order: 0,
                    start_delay: 0,
                    shutdown_delay: 0,
                },
                value: ApplianceOrder {
                    order: 1,
                    start_delay: 30,
                    shutdown_delay: 0,
                },
            }),
            vec!["vm.set_order"],
        ),
        (
            ResourceAction::SnapshotPolicy(SnapshotPolicyOperation::Configure {
                expected: state,
                config,
                clock: SnapshotPolicyClock {
                    coordinator: Uuid::from_u128(5).into(),
                    utc_offset_minutes: 0,
                },
            }),
            vec!["vmss.set_name_label"],
        ),
        (
            ResourceAction::Organization(
                OrganizationPatch::new(vec![
                    OrganizationEdit {
                        object: OrganizationObjectId::Vm(Uuid::from_u128(4).into()),
                        change: OrganizationChange::Tags {
                            previous: vec![],
                            value: vec!["Production".into()],
                        },
                    },
                    OrganizationEdit {
                        object: OrganizationObjectId::Host(Uuid::from_u128(5).into()),
                        change: OrganizationChange::Tags {
                            previous: vec![],
                            value: vec!["Production".into()],
                        },
                    },
                ])
                .unwrap(),
            ),
            vec!["vm.set_tags"],
        ),
    ];
    for (action, allowed) in cases {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let output = calls.clone();
        let fixture = crate::xapi::inventory_tests::Fixture::with_permissions(
            move |r| {
                output.lock().unwrap().push(r);
                json!({"error":{"message":"UNEXPECTED_CALL"}})
            },
            json!({"is_local_superuser":false,"rbac_permissions":allowed}),
        )
        .await;
        let (handle, request) = crate::resources::channel(
            OperationId::new(),
            ObservedResource {
                pool: Uuid::from_u128(1).into(),
                generation: 1,
                target: ResourceTarget::Pool(Uuid::from_u128(1).into()),
            },
            action,
            tokio::runtime::Handle::current(),
        );
        let directory = tempfile::tempdir().unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::channel(32);
        let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
        request
            .run(
                Arc::new(fixture.session().await),
                directory.path().into(),
                tokio_util::sync::CancellationToken::new(),
                tx,
            )
            .await;
        drain.await.unwrap();
        let record = handle.latest();
        assert_eq!(record.state, OperationState::Rejected, "{}", record.detail);
        assert!(
            record.detail.contains("RBAC_PERMISSION_DENIED"),
            "{}",
            record.detail
        );
        assert!(calls.lock().unwrap().is_empty());
    }
}
#[test]
fn resource_permission_reviews_ignore_unchanged_fields_and_keep_http_authority_unknown() {
    let appliance = ApplianceOperation::Rename {
        appliance: Uuid::from_u128(3).into(),
        previous_name: "Old".into(),
        previous_description: "Keep".into(),
        name: "New".into(),
        description: "Keep".into(),
    };
    assert_eq!(
        appliance
            .permission_requirements()
            .assess(&permissions(&["vm_appliance.set_name_label"]))
            .decision(),
        PermissionDecision::Granted
    );
    let expected = policy();
    let mut config = expected.config.clone();
    config.name = "New".into();
    let op = SnapshotPolicyOperation::Configure {
        expected,
        config,
        clock: SnapshotPolicyClock {
            coordinator: Uuid::from_u128(5).into(),
            utc_offset_minutes: 0,
        },
    };
    assert_eq!(
        op.permission_requirements()
            .assess(&permissions(&["vmss.set_name_label"]))
            .decision(),
        PermissionDecision::Granted
    );
    let wake = HostPowerOperation::Configure {
        expected: HostPowerConfiguration {
            settings: HostPowerSettings::Disabled,
            secret: None,
        },
        settings: HostPowerSettings::WakeOnLan,
    };
    assert_eq!(
        wake.permission_requirements()
            .assess(&permissions(&["host.set_power_on_mode"]))
            .decision(),
        PermissionDecision::Granted
    );
    let cloud = CloudInitConfiguration {
        sr: Uuid::from_u128(6).into(),
        device: "1".into(),
        configuration_id: Uuid::from_u128(7),
    };
    assert!(
        cloud
            .permission_requirements()
            .assess(&permissions(&["vdi.create", "task.create"]))
            .decision()
            .is_denied()
    );
    assert_eq!(
        cloud
            .permission_requirements()
            .assess(&permissions(&["vdi.create", "task.create", "vbd.create"]))
            .decision(),
        PermissionDecision::Unknown(PermissionUnknown::WorkflowDependent)
    );
}
#[test]
fn credential_provisioning_requires_its_later_configuration_and_cleanup_permissions() {
    let power = HostPowerOperation::Configure {
        expected: HostPowerConfiguration {
            settings: HostPowerSettings::Disabled,
            secret: None,
        },
        settings: HostPowerSettings::Ipmi {
            address: "192.0.2.10".parse().unwrap(),
            username: "operator".into(),
        },
    };
    assert!(
        power
            .permission_requirements()
            .assess(&permissions(&["secret.create"]))
            .decision()
            .is_denied()
    );
    assert!(
        power
            .permission_requirements()
            .assess(&permissions(&["host.set_power_on_mode"]))
            .decision()
            .is_denied()
    );
    let storage = StorageOperation::CreateAttachment {
        host: Uuid::from_u128(5).into(),
        location: StorageLocation::IsoSmb {
            server: "files.example.test".into(),
            share: "images".into(),
            path: "/".into(),
        },
    };
    let review = storage.authenticated_permission_requirements();
    let allowed = permissions(&["pbd.create", "secret.create"]);
    let denied = review.assess(&allowed);
    assert_eq!(denied.denied_methods(), &["secret.destroy"]);
    assert_eq!(
        review
            .assess(&permissions(&[
                "pbd.create",
                "secret.create",
                "secret.destroy"
            ]))
            .decision(),
        PermissionDecision::Granted
    );
}
