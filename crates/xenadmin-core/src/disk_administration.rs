use crate::{
    ActionAvailability, ClassAvailability, Inventory, OperationId, ResourceTarget, SrId, VbdId,
    VdiId, VmId,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use xenadmin_xapi::enumerations::{vbd_type, vdi_operations, vdi_type};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiskConsumer {
    pub attachment: VbdId,
    pub vm: VmId,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewedDisk {
    pub sr: SrId,
    pub size: u64,
    pub consumers: Vec<DiskConsumer>,
}
impl ReviewedDisk {
    pub fn from_inventory(inventory: &Inventory, disk: VdiId) -> Option<Self> {
        let vdi = inventory.details.vdis.get(&disk)?;
        let mut consumers = inventory
            .details
            .vbds
            .values()
            .filter(|v| v.vdi == Some(disk))
            .map(|v| {
                Some(DiskConsumer {
                    attachment: v.id,
                    vm: v.vm?,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        consumers.sort_by_key(|v| v.attachment);
        Some(Self {
            sr: vdi.sr?,
            size: vdi.virtual_size?,
            consumers,
        })
    }
    fn valid(&self) -> bool {
        self.size > 0
            && self.size <= i64::MAX as u64
            && self.consumers.len() <= 256
            && self
                .consumers
                .iter()
                .map(|c| c.attachment)
                .collect::<BTreeSet<_>>()
                .len()
                == self.consumers.len()
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OwnedDiskSource {
    VmOperation(OperationId),
    ResourceOperation(OperationId),
}
impl OwnedDiskSource {
    pub fn operation(self) -> OperationId {
        match self {
            Self::VmOperation(id) | Self::ResourceOperation(id) => id,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiskOperation {
    Grow {
        expected: ReviewedDisk,
        size: u64,
    },
    Copy {
        expected: ReviewedDisk,
        destination: SrId,
        name: String,
    },
    Cleanup {
        expected: ReviewedDisk,
        source: OwnedDiskSource,
    },
}
impl DiskOperation {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Grow { .. } => "Grow virtual disk",
            Self::Copy { .. } => "Copy virtual disk",
            Self::Cleanup { .. } => "Delete detached owned disk",
        }
    }
    pub fn expected(&self) -> &ReviewedDisk {
        match self {
            Self::Grow { expected, .. }
            | Self::Copy { expected, .. }
            | Self::Cleanup { expected, .. } => expected,
        }
    }
    pub fn permission_requirements(&self) -> crate::PermissionRequirements {
        crate::PermissionRequirements::writes(match self {
            Self::Grow { .. } => vec!["VDI.resize"],
            Self::Copy { .. } => vec!["VDI.copy", "VDI.set_name_label"],
            Self::Cleanup { .. } => vec!["VDI.destroy"],
        })
    }
    pub(crate) fn validate(&self, target: ResourceTarget) -> Result<(), crate::Error> {
        if !matches!(target, ResourceTarget::Disk(_)) || !self.expected().valid() {
            return Err(crate::Error::InvalidResourceCommand);
        }
        let valid = match self {
            Self::Grow { expected, size } => {
                *size > expected.size && *size <= i64::MAX as u64 && size.is_multiple_of(512)
            }
            Self::Copy { name, .. } => {
                !name.trim().is_empty()
                    && name.chars().count() <= 255
                    && !name.chars().any(char::is_control)
            }
            Self::Cleanup { expected, .. } => expected.consumers.is_empty(),
        };
        if valid {
            Ok(())
        } else {
            Err(crate::Error::InvalidResourceCommand)
        }
    }
    pub fn availability(&self, inventory: &Inventory, disk: VdiId) -> ActionAvailability {
        if self.validate(ResourceTarget::Disk(disk)).is_err() {
            return ActionAvailability::Prerequisite { reason: "Review a larger sector-aligned size, a valid copy name, or a detached owned disk".into() };
        }
        if ["vdi", "vbd"].iter().any(|c| {
            !matches!(
                inventory.details.classes.get(*c),
                Some(ClassAvailability::Available)
            )
        }) {
            return ActionAvailability::Unknown;
        }
        if ReviewedDisk::from_inventory(inventory, disk).as_ref() != Some(self.expected()) {
            return ActionAvailability::Stale;
        }
        let Some(vdi) = inventory.details.vdis.get(&disk) else {
            return ActionAvailability::Stale;
        };
        if vdi.managed != Some(true)
            || vdi.missing != Some(false)
            || vdi.read_only != Some(false)
            || vdi.sharable != Some(false)
            || vdi.is_snapshot != Some(false)
            || vdi.snapshot_of.is_some()
            || vdi.cbt_enabled != Some(false)
            || vdi.kind != vdi_type::USER
        {
            return ActionAvailability::Prerequisite { reason: "Requires an ordinary managed, writable, nonshared data disk without snapshot or CBT state".into() };
        }
        for consumer in &self.expected().consumers {
            if inventory
                .details
                .vbds
                .get(&consumer.attachment)
                .is_none_or(|v| {
                    v.currently_attached != Some(false)
                        || v.kind != vbd_type::DISK
                        || v.empty != Some(false)
                })
                || inventory.vms.get(&consumer.vm).is_none_or(|v| {
                    v.is_template
                        || !matches!(
                            v.power_state,
                            crate::PowerState::Halted | crate::PowerState::Running
                        )
                })
            {
                return ActionAvailability::Prerequisite { reason: "Unplug every attachment first; suspended, paused, template and snapshot consumers are unsupported".into() };
            }
        }
        let capability = match self {
            Self::Grow { .. } => vdi_operations::RESIZE,
            Self::Copy { .. } => vdi_operations::COPY,
            Self::Cleanup { .. } => vdi_operations::DESTROY,
        };
        if vdi
            .allowed_operations
            .as_ref()
            .is_none_or(|ops| !ops.iter().any(|op| op == capability))
        {
            return ActionAvailability::Unknown;
        }
        if let Self::Copy { destination, .. } = self
            && !inventory
                .storage
                .iter()
                .any(|s| s.uuid == destination.uuid())
        {
            return ActionAvailability::Stale;
        }
        ActionAvailability::Available
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Kind {
    Grow,
    Copy,
    Name,
    Cleanup,
}
impl Kind {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Grow => "Grow disk backing storage",
            Self::Copy => "Copy disk to chosen storage",
            Self::Name => "Name copied disk",
            Self::Cleanup => "Delete reviewed detached disk",
        }
    }
    pub(crate) fn asynchronous(self) -> bool {
        matches!(self, Self::Grow | Self::Copy | Self::Cleanup)
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Evidence {
    pub task: Option<String>,
    pub task_uuid: Option<crate::TaskId>,
    pub result: Option<String>,
    pub completed: bool,
    pub ended: bool,
    pub verified: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Plan {
    pub pool: crate::PoolId,
    pub disk: VdiId,
    pub operation: DiskOperation,
    pub tasks_before: BTreeSet<crate::TaskId>,
    pub disks_before: BTreeSet<VdiId>,
    pub kinds: Vec<Kind>,
    pub evidence: Vec<Evidence>,
    pub created: Option<VdiId>,
    pub actual_size: Option<u64>,
}
impl Plan {
    pub(crate) fn kinds(operation: &DiskOperation) -> Vec<Kind> {
        match operation {
            DiskOperation::Grow { .. } => vec![Kind::Grow],
            DiskOperation::Copy { .. } => vec![Kind::Copy, Kind::Name],
            DiskOperation::Cleanup { .. } => vec![Kind::Cleanup],
        }
    }
    pub(crate) fn extends(&self, old: &Self) -> bool {
        self.pool == old.pool
            && self.disk == old.disk
            && self.operation == old.operation
            && self.tasks_before == old.tasks_before
            && self.disks_before == old.disks_before
            && self.kinds == old.kinds
            && self.evidence.len() == old.evidence.len()
            && (old.created.is_none() || self.created == old.created)
            && (old.actual_size.is_none() || self.actual_size == old.actual_size)
            && self.evidence.iter().zip(&old.evidence).all(|(n, o)| {
                (o.task.is_none() || n.task == o.task)
                    && (o.task_uuid.is_none() || n.task_uuid == o.task_uuid)
                    && (o.result.is_none() || n.result == o.result)
                    && (!o.completed || n.completed)
                    && (!o.ended || n.ended)
                    && (!o.verified || n.verified)
            })
    }
}
pub(crate) fn validate_record(r: &crate::ResourceOperationRecord) -> Result<(), crate::Error> {
    use crate::{OperationState, ResourceAction, StepState};
    let fail = || crate::Error::Journal;
    let Some(p) = &r.disk_plan else {
        if matches!(r.action, ResourceAction::Disk(_))
            && (!r.steps.is_empty() || r.state == OperationState::Succeeded || r.created.is_some())
        {
            return Err(fail());
        }
        return Ok(());
    };
    let ResourceAction::Disk(op) = &r.action else {
        return Err(fail());
    };
    if p.pool != r.pool
        || r.target != ResourceTarget::Disk(p.disk)
        || p.operation != *op
        || op.validate(r.target).is_err()
        || p.kinds != Plan::kinds(op)
        || p.evidence.len() != p.kinds.len()
        || r.steps.len() != p.kinds.len()
        || p.tasks_before.len() > 100_000
        || p.disks_before.len() > 100_000
        || r.created != p.created.map(ResourceTarget::Disk)
        || r.disk_size != p.actual_size
    {
        return Err(fail());
    }
    if let Some(id) = p.created
        && (!matches!(op, DiskOperation::Copy { .. })
            || p.disks_before.contains(&id)
            || !p.evidence[0].completed
            || !p.evidence[0].verified
            || p.evidence[0].result.is_none())
    {
        return Err(fail());
    }
    if let Some(actual) = p.actual_size
        && (!matches!(op, DiskOperation::Grow { size, .. } if actual >= *size)
            || !p.evidence[0].verified)
    {
        return Err(fail());
    }
    for (i, ((kind, e), step)) in p.kinds.iter().zip(&p.evidence).zip(&r.steps).enumerate() {
        if step.label != kind.label()
            || (step.state == StepState::Verified && !e.verified)
            || (e.verified && !e.completed)
            || (e.completed && !e.ended)
            || (e.task_uuid.is_some() && e.task.is_none())
            || (kind.asynchronous() && e.completed && e.task_uuid.is_none())
            || e.task_uuid.is_some_and(|id| p.tasks_before.contains(&id))
            || e.result.is_some() && (*kind != Kind::Copy || !e.completed)
            || (i > 0
                && (step.state != StepState::Pending || e.completed || e.task.is_some())
                && !p.evidence[i - 1].verified)
            || (r.state == OperationState::Succeeded
                && (!e.verified || step.state != StepState::Verified))
        {
            return Err(fail());
        }
    }
    Ok(())
}
