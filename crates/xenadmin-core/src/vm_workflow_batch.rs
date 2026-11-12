use crate::{
    CheckpointMemory, CrossPoolVmOperation, Error, ExportSource, OperationAction, OperationId,
    OperationState, PoolId, PowerState, ResourceAction, ResourceOperationRecord, ResourceTarget,
    SnapshotDeletionSelection, SnapshotId, VdiId, VmId, VmLifecycleOperation,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum VmWorkflow {
    Export {
        expected_template: bool,
        format: VmExportFormat,
    },
    /// One snapshot of the item's VM, exported as an archive of its own. Journal format 5
    /// is the first that may hold this variant.
    ExportSnapshot {
        snapshot: SnapshotId,
        /// Present exactly when the snapshot is a memory checkpoint.
        checkpoint: Option<ReviewedCheckpoint>,
        format: VmExportFormat,
    },
    Move(crate::VmMigration),
    /// The item's VM moved to another connected pool by the single cross-pool move, with
    /// the destination and the disk and NIC mappings reviewed for it. Journal format 5 may
    /// hold this variant; the release that stops at format 4 has no such item.
    MoveToPool(CrossPoolVmOperation),
    Delete {
        expected_template: bool,
        disks: Vec<VdiId>,
        snapshots: Vec<SnapshotDeletionSelection>,
    },
}
/// The memory image a review showed for a checkpoint, and that review's choice of whether
/// the archive carries it. Leaving the memory out is the default; including it is chosen
/// for each checkpoint by itself.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewedCheckpoint {
    pub memory: CheckpointMemory,
    pub preserve_memory: bool,
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
            Self::MoveToPool(_) => "Move VM to another pool",
            Self::Export { .. } => "Export VM",
            Self::ExportSnapshot { .. } => "Export snapshot",
        }
    }
    pub(crate) fn action(&self) -> OperationAction {
        match self {
            Self::Move(operation) => OperationAction::Migration(operation.clone()),
            Self::Export { .. } | Self::ExportSnapshot { .. } | Self::MoveToPool(_) => {
                unreachable!("resource children have no VM action")
            }
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
    /// The single-source transfer a child of this item runs. `vm` is the item's VM: the
    /// exported guest itself, or the parent of the exported snapshot.
    pub(crate) fn transfer(&self, vm: VmId) -> Option<crate::TransferAction> {
        let (source, format) = match self {
            Self::Export {
                expected_template,
                format,
            } => (
                ExportSource::Vm {
                    vm,
                    expected_template: *expected_template,
                },
                format,
            ),
            Self::ExportSnapshot {
                snapshot,
                checkpoint,
                format,
            } => (
                match checkpoint {
                    None => ExportSource::Snapshot {
                        snapshot: *snapshot,
                        parent: vm,
                    },
                    Some(reviewed) => ExportSource::Checkpoint {
                        snapshot: *snapshot,
                        parent: vm,
                        memory: reviewed.memory.clone(),
                        preserve_memory: reviewed.preserve_memory,
                    },
                },
                format,
            ),
            Self::Move(_) | Self::MoveToPool(_) | Self::Delete { .. } => return None,
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
    /// The cross-pool move a child of this item runs, with its reviewed mappings.
    pub fn cross_pool(&self) -> Option<&CrossPoolVmOperation> {
        match self {
            Self::MoveToPool(operation) => Some(operation),
            _ => None,
        }
    }
    /// Which journal table holds a child of this item: a VM operation, or a resource
    /// operation (an archive transfer or a cross-pool move).
    pub(crate) fn kind(&self) -> &'static str {
        if matches!(
            self,
            Self::Export { .. } | Self::ExportSnapshot { .. } | Self::MoveToPool(_)
        ) {
            "resource"
        } else {
            "vm"
        }
    }
}
/// One item of a batch. For `VmWorkflow::ExportSnapshot`, `vm` is the snapshot's parent and
/// `expected_state` is the snapshot's own recorded state: halted for a disk snapshot,
/// suspended for a memory checkpoint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchVmWorkflow {
    pub vm: VmId,
    pub expected_state: PowerState,
    pub workflow: VmWorkflow,
}
impl BatchVmWorkflow {
    /// The batch item that moves a guest to another pool exactly as the reviewed single
    /// move would: a running guest live, a halted guest halted.
    pub fn move_to_pool(operation: CrossPoolVmOperation) -> Self {
        Self {
            vm: operation.vm,
            expected_state: if operation.mode.live() {
                PowerState::Running
            } else {
                PowerState::Halted
            },
            workflow: VmWorkflow::MoveToPool(operation),
        }
    }
    /// The batch item that exports what a single snapshot or checkpoint export of `source`
    /// would, in `format`. None for a source that is not a snapshot.
    pub fn snapshot_export(source: &ExportSource, format: VmExportFormat) -> Option<Self> {
        let (vm, snapshot, checkpoint) = match source {
            ExportSource::Vm { .. } => return None,
            ExportSource::Snapshot { snapshot, parent } => (*parent, *snapshot, None),
            ExportSource::Checkpoint {
                snapshot,
                parent,
                memory,
                preserve_memory,
            } => (
                *parent,
                *snapshot,
                Some(ReviewedCheckpoint {
                    memory: memory.clone(),
                    preserve_memory: *preserve_memory,
                }),
            ),
        };
        Some(Self {
            vm,
            expected_state: if checkpoint.is_some() {
                PowerState::Suspended
            } else {
                PowerState::Halted
            },
            workflow: VmWorkflow::ExportSnapshot {
                snapshot,
                checkpoint,
                format,
            },
        })
    }
    /// The object this item is about: the exported snapshot, or the VM.
    pub(crate) fn subject(&self) -> uuid::Uuid {
        match &self.workflow {
            VmWorkflow::ExportSnapshot { snapshot, .. } => snapshot.uuid(),
            _ => self.vm.uuid(),
        }
    }
    pub(crate) fn step_label(&self) -> String {
        format!("{} {}", self.workflow.label(), self.subject())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VmWorkflowBatch {
    pub items: Vec<BatchVmWorkflow>,
}
impl VmWorkflowBatch {
    pub(crate) fn is_export(&self) -> bool {
        !self.items.is_empty()
            && self.items.iter().all(|i| {
                matches!(
                    i.workflow,
                    VmWorkflow::Export { .. } | VmWorkflow::ExportSnapshot { .. }
                )
            })
    }
    /// The pool every item of a batch of cross-pool moves goes to. None for any other
    /// batch; a valid batch never mixes kinds or destinations.
    pub fn cross_pool_destination(&self) -> Option<PoolId> {
        match self.items.first().map(|i| &i.workflow) {
            Some(VmWorkflow::MoveToPool(operation)) => Some(operation.destination_pool),
            _ => None,
        }
    }
    /// The reviewed move of each item, in order, when this is a batch of cross-pool moves.
    pub(crate) fn cross_pool_operations(&self) -> Option<Vec<&CrossPoolVmOperation>> {
        self.cross_pool_destination()?;
        self.items.iter().map(|i| i.workflow.cross_pool()).collect()
    }
    pub fn is_snapshot_export(&self) -> bool {
        !self.items.is_empty()
            && self
                .items
                .iter()
                .all(|i| matches!(i.workflow, VmWorkflow::ExportSnapshot { .. }))
    }
    /// What one item is called in progress text: a batch is of VMs or of snapshots.
    pub(crate) fn noun(&self) -> &'static str {
        if self.is_snapshot_export() {
            "snapshot"
        } else {
            "VM"
        }
    }
    pub(crate) fn validate(&self, target: ResourceTarget) -> Result<(), Error> {
        let mut identities = BTreeSet::new();
        if !matches!(target, ResourceTarget::Pool(_)) || !(1..=32).contains(&self.items.len()) {
            return Err(Error::InvalidResourceCommand);
        }
        for item in &self.items {
            if item.vm.uuid().is_nil()
                || item.subject().is_nil()
                || !identities.insert(item.subject())
                || std::mem::discriminant(&item.workflow)
                    != std::mem::discriminant(&self.items[0].workflow)
            {
                return Err(Error::InvalidResourceCommand);
            }
            if let VmWorkflow::ExportSnapshot {
                snapshot,
                checkpoint,
                format,
            } = &item.workflow
            {
                // A checkpoint is exported only with its memory image reviewed, and only
                // as XVA: the single export offers no package format for one either.
                let valid = snapshot.uuid() != item.vm.uuid()
                    && match checkpoint {
                        None => item.expected_state == PowerState::Halted,
                        Some(reviewed) => {
                            item.expected_state == PowerState::Suspended
                                && matches!(format, VmExportFormat::Xva { .. })
                                && !reviewed.memory.sr.uuid().is_nil()
                                && (1..=crate::transfer::MAX_TRANSFER_BYTES)
                                    .contains(&reviewed.memory.virtual_size)
                                && !reviewed.memory.vdi.uuid().is_nil()
                                && identities.insert(reviewed.memory.vdi.uuid())
                        }
                    };
                if !valid {
                    return Err(Error::InvalidResourceCommand);
                }
                continue;
            }
            if let VmWorkflow::MoveToPool(operation) = &item.workflow {
                // Each item is the single cross-pool move of its own guest, with the
                // transport acknowledgement that move requires. A batch has one
                // destination pool, host and migration address, holds moves only, and
                // names no disk or NIC twice.
                let ResourceTarget::Pool(source) = target else {
                    return Err(Error::InvalidResourceCommand);
                };
                operation.validate(source)?;
                let Some(VmWorkflow::MoveToPool(first)) = self.items.first().map(|i| &i.workflow)
                else {
                    return Err(Error::InvalidResourceCommand);
                };
                if operation.vm != item.vm
                    || operation.mode.copy()
                    || item.expected_state
                        != if operation.mode.live() {
                            PowerState::Running
                        } else {
                            PowerState::Halted
                        }
                    || operation.destination_pool != first.destination_pool
                    || operation.destination_host != first.destination_host
                    || operation.transfer_network != first.transfer_network
                    || operation.transfer_address != first.transfer_address
                    || operation
                        .disks
                        .iter()
                        .map(|d| d.vdi.uuid())
                        .chain(operation.networks.iter().map(|n| n.vif.uuid()))
                        .any(|id| !identities.insert(id))
                {
                    return Err(Error::InvalidMigration);
                }
                continue;
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
                VmWorkflow::Export { .. } | VmWorkflow::ExportSnapshot { .. } => {}
                // The source pool sends the guest; what the destination pool is asked for
                // is in `peer_permission_requirements`.
                VmWorkflow::MoveToPool(_) => writes.push("VM.migrate_send".to_owned()),
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
    /// What a batch of cross-pool moves asks of the session on the destination pool: the
    /// receiver each move is prepared with. None for a batch that stays in its pool.
    pub fn peer_permission_requirements(&self) -> Option<crate::PermissionRequirements> {
        self.cross_pool_destination()
            .map(|_| crate::PermissionRequirements::writes(["host.migrate_receive"]))
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
            || r.steps[index].label != item.step_label()
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
