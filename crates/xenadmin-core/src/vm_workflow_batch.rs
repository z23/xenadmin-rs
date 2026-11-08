use crate::{
    Error, OperationAction, OperationId, OperationState, PowerState, ResourceAction,
    ResourceOperationRecord, ResourceTarget, SnapshotDeletionSelection, VdiId, VmId,
    VmLifecycleOperation,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum VmWorkflow {
    Delete {
        expected_template: bool,
        disks: Vec<VdiId>,
        snapshots: Vec<SnapshotDeletionSelection>,
    },
}
impl VmWorkflow {
    pub(crate) fn action(&self) -> OperationAction {
        match self {
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
    pub(crate) fn validate(&self, target: ResourceTarget) -> Result<(), Error> {
        let mut identities = BTreeSet::new();
        if !matches!(target, ResourceTarget::Pool(_)) || !(1..=32).contains(&self.items.len()) {
            return Err(Error::InvalidResourceCommand);
        }
        for item in &self.items {
            if item.vm.uuid().is_nil()
                || item.expected_state != PowerState::Halted
                || !identities.insert(item.vm.uuid())
            {
                return Err(Error::InvalidResourceCommand);
            }
            let OperationAction::Lifecycle(op) = item.workflow.action() else {
                unreachable!()
            };
            op.validate()?;
            let VmWorkflow::Delete {
                disks, snapshots, ..
            } = &item.workflow;
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
        let mut writes = vec!["VM.destroy"];
        if self.items.iter().any(|i| {
            matches!(&i.workflow, VmWorkflow::Delete { disks, snapshots, .. }
            if !disks.is_empty() || snapshots.iter().any(|s|!s.disks.is_empty()))
        }) {
            writes.push("VDI.destroy");
        }
        crate::PermissionRequirements::writes(writes)
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Plan {
    pub selection: VmWorkflowBatch,
    pub children: Vec<OperationId>,
    pub admitted: usize,
}
impl Plan {
    pub(crate) fn new(selection: VmWorkflowBatch) -> Self {
        let children = selection.items.iter().map(|_| OperationId::new()).collect();
        Self {
            selection,
            children,
            admitted: 0,
        }
    }
    pub(crate) fn extends(&self, old: &Self) -> bool {
        self.selection == old.selection
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
    if p.selection != *selection
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
            || r.steps[index].label != format!("Delete VM {}", item.vm)
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
