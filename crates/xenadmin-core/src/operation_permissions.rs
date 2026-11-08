use crate::{
    GuestFirmware, OperationAction, PermissionRequirements as Required, PowerAction,
    SnapshotOperation, SnapshotWorkflow, VmCopyMode, VmLifecycleOperation,
};

impl OperationAction {
    pub fn permission_requirements(&self) -> Required {
        match self {
            Self::Recovery(op) => op.permission_requirements(),
            Self::Power(op) => Required::writes([format!("VM.{}", op.wire())]),
            Self::Placement(op) => Required::writes([if op.placement == PowerAction::Resume {
                "VM.resume_on"
            } else {
                "VM.start_on"
            }]),
            Self::Settings(op) => op.permission_requirements(),
            Self::Device(op) => op.permission_requirements(),
            Self::Lifecycle(op) => op.permission_requirements(),
            Self::Snapshot(op) => op.permission_requirements(),
            Self::Migration(_) => {
                Required::writes(Vec::<String>::new()).with_host_dependent_steps()
            }
        }
    }
}
impl SnapshotOperation {
    pub fn deletion_permission_requirements(
        disks: &[crate::VdiId],
        checkpoint: Option<bool>,
    ) -> Required {
        let mut methods = vec!["VM.destroy"];
        if !disks.is_empty() {
            methods.push("VDI.destroy");
        }
        if checkpoint == Some(true) {
            methods.push("VM.hard_shutdown");
        }
        let requirements = Required::writes(methods);
        if checkpoint.is_none() {
            requirements.with_host_dependent_steps()
        } else {
            requirements
        }
    }
    pub fn permission_requirements(&self) -> Required {
        match self {
            Self::Workflow { operation } => operation.permission_requirements(),
            Self::Checkpoint { .. } => Required::writes(["VM.checkpoint"]),
            Self::Create { .. } => Required::writes(["VM.snapshot"]),
            Self::Revert { .. } => Required::writes(["VM.revert"]),
            Self::Clone { as_template, .. } => {
                let mut methods = vec!["VM.clone"];
                if !as_template {
                    methods.push("VM.set_is_a_template");
                }
                Required::writes(methods)
            }
            Self::Delete { disks, .. } => Self::deletion_permission_requirements(disks, None),
        }
    }
}
impl SnapshotWorkflow {
    pub fn permission_requirements(&self) -> Required {
        Required::writes(self.steps().iter().map(|step| step.method(self)))
    }
}
impl VmLifecycleOperation {
    pub fn permission_requirements(&self) -> Required {
        match self {
            Self::Create {
                firmware,
                bios_host,
                disk_size,
                iso,
                ..
            } => {
                let mut methods = vec![
                    "VM.provision",
                    "VM.set_name_description",
                    "VM.set_memory_limits",
                    "VM.set_VCPUs_max",
                    "VM.set_VCPUs_at_startup",
                ];
                if *firmware != GuestFirmware::Inherited {
                    methods.push("VM.set_HVM_boot_params");
                }
                if matches!(firmware, GuestFirmware::Uefi | GuestFirmware::SecureUefi) {
                    methods.push("VM.set_platform");
                }
                if bios_host.is_some() {
                    methods.push("VM.copy_bios_strings");
                }
                if disk_size.is_some() {
                    methods.extend(["VDI.create", "VBD.create"]);
                }
                if iso.is_some() {
                    methods.push("VBD.insert");
                }
                Required::writes(methods).with_host_dependent_steps()
            }
            Self::Clone { mode, .. } => Required::writes([match mode {
                VmCopyMode::Fast => "VM.clone",
                VmCopyMode::Full { .. } => "VM.copy",
            }])
            .with_host_dependent_steps(),
            Self::ConvertToTemplate => Required::writes(["VM.set_is_a_template"]),
            Self::EditTemplate { .. } => {
                Required::writes(["VM.set_name_label", "VM.set_name_description"])
            }
            Self::Delete {
                disks, snapshots, ..
            } => {
                let mut methods = vec!["VM.destroy"];
                if !disks.is_empty() || snapshots.iter().any(|s| !s.disks.is_empty()) {
                    methods.push("VDI.destroy");
                }
                Required::writes(methods)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        OperationId, OperationState, PowerState, operations, xapi::inventory_tests::Fixture,
    };
    use serde_json::json;
    use std::sync::{Arc, Mutex};
    use uuid::Uuid;
    #[tokio::test]
    async fn later_snapshot_and_lifecycle_denials_prevent_all_earlier_mutations() {
        let actions = vec![
            OperationAction::Snapshot(SnapshotOperation::Workflow {
                operation: SnapshotWorkflow::Create {
                    name: "Before".into(),
                    description: "Details".into(),
                    memory: false,
                },
            }),
            OperationAction::Snapshot(SnapshotOperation::Workflow {
                operation: SnapshotWorkflow::Restore {
                    snapshot: Uuid::from_u128(3).into(),
                    expected_state: PowerState::Running,
                    checkpoint: false,
                    shutdown: true,
                    start_after: true,
                },
            }),
            OperationAction::Snapshot(SnapshotOperation::Clone {
                snapshot: Uuid::from_u128(3).into(),
                name: "Copy".into(),
                as_template: false,
            }),
            OperationAction::Snapshot(SnapshotOperation::Delete {
                snapshot: Uuid::from_u128(3).into(),
                disks: vec![Uuid::from_u128(4).into()],
            }),
            OperationAction::Lifecycle(VmLifecycleOperation::Delete {
                expected_template: false,
                disks: vec![Uuid::from_u128(4).into()],
                snapshots: vec![],
            }),
            OperationAction::Lifecycle(VmLifecycleOperation::Create {
                name: "New".into(),
                description: "".into(),
                sr: Uuid::from_u128(4).into(),
                network: Uuid::from_u128(5).into(),
                iso: None,
                vcpus: 2,
                memory: 1024 * 1024 * 1024,
                disk_size: None,
                firmware: GuestFirmware::Inherited,
                bios_host: None,
            }),
        ];
        for action in actions {
            let calls = Arc::new(Mutex::new(Vec::new()));
            let output = calls.clone();
            let permissions = [
                "vm.snapshot",
                "vm.clean_shutdown",
                "vm.revert",
                "vm.clone",
                "vm.destroy",
                "vm.provision",
                "vm.set_name_description",
                "vm.set_memory_limits",
                "vm.set_vcpus_max",
            ]
            .into_iter()
            .filter(|method| {
                !matches!(
                    &action,
                    OperationAction::Snapshot(SnapshotOperation::Workflow {
                        operation: SnapshotWorkflow::Create { .. }
                    })
                ) || *method != "vm.set_name_description"
            })
            .collect::<Vec<_>>();
            let f = Fixture::with_permissions(
                move |r| {
                    output.lock().unwrap().push(r);
                    json!({"error":{"message":"UNEXPECTED_CALL"}})
                },
                json!({"is_local_superuser":false,"rbac_permissions":permissions}),
            )
            .await;
            let d = tempfile::tempdir().unwrap();
            let (handle, request) = operations::channel(
                OperationId::new(),
                Uuid::from_u128(1).into(),
                Uuid::from_u128(2).into(),
                1,
                action,
                tokio::runtime::Handle::current(),
            );
            let (tx, mut rx) = tokio::sync::mpsc::channel(32);
            let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
            request
                .run(
                    Arc::new(f.session().await),
                    Some("OpaqueRef:VM:source".into()),
                    d.path().into(),
                    tokio_util::sync::CancellationToken::new(),
                    tx,
                )
                .await;
            drain.await.unwrap();
            assert_eq!(
                handle.latest().state,
                OperationState::Rejected,
                "{}",
                handle.latest().detail
            );
            assert!(handle.latest().detail.contains("RBAC_PERMISSION_DENIED"));
            assert!(calls.lock().unwrap().is_empty());
        }
    }
    #[test]
    fn optional_restore_cleanup_and_unchanged_metadata_do_not_invent_requirements() {
        let snapshot = Uuid::from_u128(3).into();
        let perms = crate::SessionPermissions::decode(
            &json!({"is_local_superuser":false,"rbac_permissions":["vm.revert","vm.destroy","vm.set_name_label"]}),
        );
        assert_eq!(
            SnapshotOperation::deletion_permission_requirements(&[], Some(false))
                .assess(&perms)
                .decision(),
            crate::PermissionDecision::Granted
        );
        assert!(matches!(
            SnapshotOperation::Delete {
                snapshot,
                disks: vec![]
            }
            .permission_requirements()
            .assess(&perms)
            .decision(),
            crate::PermissionDecision::Unknown(_)
        ));
        assert!(
            SnapshotOperation::deletion_permission_requirements(&[], Some(true))
                .assess(&perms)
                .decision()
                .is_denied()
        );
        for op in [
            SnapshotOperation::Workflow {
                operation: SnapshotWorkflow::Restore {
                    snapshot,
                    expected_state: PowerState::Halted,
                    checkpoint: true,
                    shutdown: false,
                    start_after: false,
                },
            },
            SnapshotOperation::Workflow {
                operation: SnapshotWorkflow::Edit {
                    snapshot,
                    expected_name: "Old".into(),
                    expected_description: "Keep".into(),
                    name: "New".into(),
                    description: "Keep".into(),
                },
            },
        ] {
            assert_eq!(
                op.permission_requirements().assess(&perms).decision(),
                crate::PermissionDecision::Granted
            );
        }
    }
}
