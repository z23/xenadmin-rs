use crate::{
    Error, OperationAction, OperationId, OperationState, PowerState, ResourceAction,
    ResourceOperationRecord, ResourceTarget, SnapshotDeletionSelection, VdiId, VmId,
    VmLifecycleOperation,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum VmWorkflow {
    Export {
        expected_template: bool,
        format: VmExportFormat,
    },
    Move(crate::VmMigration),
    Delete {
        expected_template: bool,
        disks: Vec<VdiId>,
        snapshots: Vec<SnapshotDeletionSelection>,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum VmExportFormat {
    Xva { compression: crate::XvaCompression },
    Ovf { format: crate::OvfPackageFormat },
}
impl VmWorkflow {
    pub(crate) fn label(&self) -> &'static str {
        match self {
            Self::Delete { .. } => "Delete VM",
            Self::Move(_) => "Move VM",
            Self::Export { .. } => "Export VM",
        }
    }
    pub(crate) fn action(&self) -> OperationAction {
        match self {
            Self::Move(operation) => OperationAction::Migration(operation.clone()),
            Self::Export { .. } => unreachable!("resource transfer children have no VM action"),
            Self::Delete {
                expected_template,
                disks,
                snapshots,
            } => OperationAction::Lifecycle(VmLifecycleOperation::Delete {
                expected_template: *expected_template,
                disks: disks.clone(),
                snapshots: snapshots.clone(),
            }),
        }
    }
    pub(crate) fn transfer(&self, vm: VmId) -> Option<crate::TransferAction> {
        let Self::Export {
            expected_template,
            format,
        } = self
        else {
            return None;
        };
        let source = crate::ExportSource::Vm {
            vm,
            expected_template: *expected_template,
        };
        Some(match format {
            VmExportFormat::Xva { compression } => crate::TransferAction::ExportXva {
                source,
                compression: *compression,
            },
            VmExportFormat::Ovf { format } => crate::TransferAction::ExportOvf {
                source,
                format: *format,
            },
        })
    }
    pub(crate) fn kind(&self) -> &'static str {
        if matches!(self, Self::Export { .. }) {
            "resource"
        } else {
            "vm"
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchVmWorkflow {
    pub vm: VmId,
    pub expected_state: PowerState,
    pub workflow: VmWorkflow,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VmWorkflowBatch {
    pub items: Vec<BatchVmWorkflow>,
}
impl VmWorkflowBatch {
    pub(crate) fn is_export(&self) -> bool {
        !self.items.is_empty()
            && self
                .items
                .iter()
                .all(|i| matches!(i.workflow, VmWorkflow::Export { .. }))
    }
    pub(crate) fn validate(&self, target: ResourceTarget) -> Result<(), Error> {
        let mut identities = BTreeSet::new();
        if !matches!(target, ResourceTarget::Pool(_)) || !(1..=32).contains(&self.items.len()) {
            return Err(Error::InvalidResourceCommand);
        }
        for item in &self.items {
            if item.vm.uuid().is_nil()
                || !identities.insert(item.vm.uuid())
                || std::mem::discriminant(&item.workflow)
                    != std::mem::discriminant(&self.items[0].workflow)
            {
                return Err(Error::InvalidResourceCommand);
            }
            if let VmWorkflow::Move(operation) = &item.workflow {
                operation.validate()?;
                if item.expected_state
                    != if operation.live {
                        PowerState::Running
                    } else {
                        PowerState::Halted
                    }
                {
                    return Err(Error::InvalidResourceCommand);
                }
                continue;
            }
            if item.expected_state != PowerState::Halted {
                return Err(Error::InvalidResourceCommand);
            }
            if matches!(item.workflow, VmWorkflow::Export { .. }) {
                continue;
            }
            let OperationAction::Lifecycle(op) = item.workflow.action() else {
                return Err(Error::InvalidResourceCommand);
            };
            op.validate()?;
            let VmWorkflow::Delete {
                disks, snapshots, ..
            } = &item.workflow
            else {
                return Err(Error::InvalidResourceCommand);
            };
            for id in disks
                .iter()
                .map(|id| id.uuid())
                .chain(snapshots.iter().flat_map(|s| {
                    std::iter::once(s.snapshot.uuid()).chain(s.disks.iter().map(|id| id.uuid()))
                }))
            {
                if id.is_nil() || !identities.insert(id) {
                    return Err(Error::InvalidResourceCommand);
                }
            }
        }
        Ok(())
    }
    pub fn permission_requirements(&self) -> crate::PermissionRequirements {
        let mut writes = Vec::new();
        for item in &self.items {
            match &item.workflow {
                VmWorkflow::Export { .. } => {}
                VmWorkflow::Move(operation) => writes.extend(
                    operation
                        .permission_requirements()
                        .methods()
                        .map(str::to_owned),
                ),
                VmWorkflow::Delete { .. } => writes.push("VM.destroy".to_owned()),
            }
        }
        if self.items.iter().any(|i| {
            matches!(&i.workflow, VmWorkflow::Delete { disks, snapshots, .. }
            if !disks.is_empty() || snapshots.iter().any(|s|!s.disks.is_empty()))
        }) {
            writes.push("VDI.destroy".to_owned());
        }
        if self.is_export() {
            crate::PermissionRequirements::writes(["task.create"]).with_host_dependent_steps()
        } else {
            crate::PermissionRequirements::writes(writes)
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VmWorkflowResult {
    pub vm: VmId,
    pub child: OperationId,
    /// None means that the child has never been admitted.
    pub state: Option<OperationState>,
    pub detail: String,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Plan {
    #[serde(default)]
    pub outputs: Vec<crate::TransferFile>,
    pub selection: VmWorkflowBatch,
    pub children: Vec<OperationId>,
    pub admitted: usize,
}
impl std::fmt::Debug for Plan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkflowBatchPlan")
            .field("selection", &self.selection)
            .field("admitted", &self.admitted)
            .finish_non_exhaustive()
    }
}
impl Plan {
    pub(crate) fn new(selection: VmWorkflowBatch) -> Self {
        let children = selection.items.iter().map(|_| OperationId::new()).collect();
        Self {
            outputs: Vec::new(),
            selection,
            children,
            admitted: 0,
        }
    }
    pub(crate) fn extends(&self, old: &Self) -> bool {
        self.selection == old.selection
            && self.outputs == old.outputs
            && self.children == old.children
            && self.admitted >= old.admitted
    }
    pub(crate) fn results(&self) -> Vec<VmWorkflowResult> {
        self.selection
            .items
            .iter()
            .zip(&self.children)
            .map(|(item, child)| VmWorkflowResult {
                vm: item.vm,
                child: *child,
                state: None,
                detail: "Not started".into(),
            })
            .collect()
    }
}
pub(crate) fn validate_record(r: &ResourceOperationRecord) -> Result<(), Error> {
    let (ResourceAction::VmWorkflowBatch(selection), Some(p)) = (&r.action, &r.workflow_batch_plan)
    else {
        return if matches!(r.action, ResourceAction::VmWorkflowBatch(_))
            || r.workflow_batch_plan.is_some()
            || !r.workflow_batch.is_empty()
        {
            Err(Error::Journal)
        } else {
            Ok(())
        };
    };
    selection.validate(r.target).map_err(|_| Error::Journal)?;
    crate::vm_workflow_export::validate(&p.outputs).map_err(|_| Error::Journal)?;
    if p.selection != *selection
        || (selection.is_export() && p.outputs.len() != selection.items.len())
        || (!selection.is_export() && !p.outputs.is_empty())
        || p.children.len() != selection.items.len()
        || p.children.iter().collect::<BTreeSet<_>>().len() != p.children.len()
        || p.children.contains(&r.id)
        || p.children
            .iter()
            .any(|id| id.to_string() == uuid::Uuid::nil().to_string())
        || p.admitted > p.children.len()
        || r.workflow_batch.len() != p.children.len()
        || r.steps.len() != p.children.len()
        || r.task.is_some()
        || r.result_reference.is_some()
    {
        return Err(Error::Journal);
    }
    for (index, ((item, id), result)) in selection
        .items
        .iter()
        .zip(&p.children)
        .zip(&r.workflow_batch)
        .enumerate()
    {
        if result.vm != item.vm
            || result.child != *id
            || result.detail.len() > 4096
            || (index >= p.admitted) == result.state.is_some()
            || r.steps[index].label != format!("{} {}", item.workflow.label(), item.vm)
            || (r.steps[index].state == crate::StepState::Verified
                && result.state != Some(OperationState::Succeeded))
            || (r.state == OperationState::Succeeded
                && result.state != Some(OperationState::Succeeded))
        {
            return Err(Error::Journal);
        }
        if r.state.terminal()
            && index < p.admitted
            && result.state.as_ref().is_none_or(|state| !state.terminal())
        {
            return Err(Error::Journal);
        }
    }
    Ok(())
}

/// Only the journal can mint a grant; its fields are never accepted by the facade.
pub(crate) struct ChildPermit {
    pub(super) parent: OperationId,
    pub(super) index: usize,
}
