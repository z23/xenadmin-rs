use crate::{
    ActionAvailability, ClassAvailability, HostId, Inventory, PbdId, ResourceTarget, SrId,
    StorageLocation,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewedStorageAttachment {
    pub attachment: PbdId,
    pub host: HostId,
    pub location: Option<StorageLocation>,
    pub requires_credentials: bool,
}
impl ReviewedStorageAttachment {
    pub fn from_inventory(i: &Inventory, sr: SrId, id: PbdId) -> Option<Self> {
        let p = i.details.pbds.get(&id)?;
        if p.sr != Some(sr) || p.currently_attached != Some(false) {
            return None;
        }
        Some(Self {
            attachment: id,
            host: p.host?,
            location: p.location.clone(),
            requires_credentials: p.requires_credentials,
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StorageMaintenance {
    Repair {
        coordinator: HostId,
        attachments: Vec<ReviewedStorageAttachment>,
    },
    ReadCaching {
        expected: Option<String>,
        enabled: bool,
    },
}
impl StorageMaintenance {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Repair { .. } => "Repair selected host attachments",
            Self::ReadCaching { .. } => "Change storage read-cache policy",
        }
    }
    pub fn permission_requirements(&self) -> crate::PermissionRequirements {
        crate::PermissionRequirements::writes(match self {
            Self::Repair { .. } => vec!["PBD.plug"],
            Self::ReadCaching { expected, enabled } => {
                let mut v = Vec::new();
                if expected.is_some() {
                    v.push("SR.remove_from_other_config");
                }
                if !enabled {
                    v.push("SR.add_to_other_config");
                }
                v
            }
        })
    }
    pub(crate) fn validate(&self, target: ResourceTarget) -> Result<(), crate::Error> {
        let valid = matches!(target, ResourceTarget::Storage(_))
            && match self {
                Self::Repair { attachments, .. } => {
                    !attachments.is_empty()
                        && attachments.len() <= 128
                        && attachments
                            .iter()
                            .all(|p| p.location.as_ref().is_none_or(StorageLocation::validate))
                        && attachments
                            .iter()
                            .map(|p| p.attachment)
                            .collect::<BTreeSet<_>>()
                            .len()
                            == attachments.len()
                        && attachments
                            .iter()
                            .map(|p| p.host)
                            .collect::<BTreeSet<_>>()
                            .len()
                            == attachments.len()
                }
                Self::ReadCaching { expected, enabled } => {
                    matches!(expected.as_deref(), None | Some("true" | "false"))
                        && if *enabled {
                            expected.as_deref() == Some("true")
                        } else {
                            expected.as_deref() != Some("true")
                        }
                }
            };
        if valid {
            Ok(())
        } else {
            Err(crate::Error::InvalidResourceCommand)
        }
    }
    pub fn availability(&self, i: &Inventory, sr: SrId) -> ActionAvailability {
        if self.validate(ResourceTarget::Storage(sr)).is_err() {
            return ActionAvailability::Prerequisite {reason:"Choose one or more distinct detached host attachments, or change the current read-cache policy".into()};
        }
        if !i.details.storage.contains_key(&sr) {
            return ActionAvailability::Stale;
        }
        match self {
            Self::Repair {
                coordinator,
                attachments,
            } => {
                if !matches!(
                    i.details.classes.get("pbd"),
                    Some(ClassAvailability::Available)
                ) {
                    return ActionAvailability::Unknown;
                }
                if i.coordinator != Some(*coordinator)
                    || attachments.iter().any(|p| {
                        ReviewedStorageAttachment::from_inventory(i, sr, p.attachment).as_ref()
                            != Some(p)
                    })
                {
                    return ActionAvailability::Stale;
                }
            }
            Self::ReadCaching { expected, .. } => {
                let d = &i.details.storage[&sr];
                if !d.read_cache_policy_known {
                    return ActionAvailability::Unknown;
                }
                if &d.read_cache_override != expected {
                    return ActionAvailability::Stale;
                }
            }
        }
        ActionAvailability::Available
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Kind {
    Plug(PbdId),
    RemoveOverride,
    DisableCaching,
}
impl Kind {
    pub(crate) fn label(&self) -> String {
        match self {
            Self::Plug(id) => format!("Attach reviewed PBD {id}"),
            Self::RemoveOverride => "Remove reviewed read-cache override".into(),
            Self::DisableCaching => "Disable storage read caching".into(),
        }
    }
    pub(crate) fn asynchronous(&self) -> bool {
        matches!(self, Self::Plug(_))
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Evidence {
    pub task: Option<String>,
    pub task_uuid: Option<crate::TaskId>,
    pub completed: bool,
    pub ended: bool,
    pub verified: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Plan {
    pub pool: crate::PoolId,
    pub sr: SrId,
    pub operation: StorageMaintenance,
    pub kinds: Vec<Kind>,
    pub evidence: Vec<Evidence>,
    pub tasks_before: BTreeSet<crate::TaskId>,
    pub configuration: std::collections::BTreeMap<PbdId, String>,
    pub backend: String,
}
impl Plan {
    pub(crate) fn kinds(op: &StorageMaintenance) -> Vec<Kind> {
        match op {
            StorageMaintenance::Repair {
                coordinator,
                attachments,
            } => {
                let mut a = attachments.clone();
                a.sort_by_key(|p| (p.host != *coordinator, p.host));
                a.into_iter().map(|p| Kind::Plug(p.attachment)).collect()
            }
            StorageMaintenance::ReadCaching { expected, enabled } => {
                let mut k = Vec::new();
                if expected.is_some() {
                    k.push(Kind::RemoveOverride);
                }
                if !enabled {
                    k.push(Kind::DisableCaching);
                }
                k
            }
        }
    }
    pub(crate) fn extends(&self, o: &Self) -> bool {
        self.pool == o.pool
            && self.sr == o.sr
            && self.operation == o.operation
            && self.kinds == o.kinds
            && self.configuration == o.configuration
            && self.backend == o.backend
            && self.tasks_before == o.tasks_before
            && self.evidence.len() == o.evidence.len()
            && self.evidence.iter().zip(&o.evidence).all(|(n, o)| {
                (o.task.is_none() || n.task == o.task)
                    && (o.task_uuid.is_none() || n.task_uuid == o.task_uuid)
                    && (!o.completed || n.completed)
                    && (!o.ended || n.ended)
                    && (!o.verified || n.verified)
            })
    }
}
pub(crate) fn validate_record(r: &crate::ResourceOperationRecord) -> Result<(), crate::Error> {
    use crate::{Error, OperationState, ResourceAction, StepState};
    let Some(p) = &r.storage_maintenance_plan else {
        return if matches!(r.action, ResourceAction::StorageMaintenance(_))
            && (!r.steps.is_empty() || r.state == OperationState::Succeeded)
        {
            Err(Error::Journal)
        } else {
            Ok(())
        };
    };
    let ResourceAction::StorageMaintenance(op) = &r.action else {
        return Err(Error::Journal);
    };
    if p.pool != r.pool
        || r.target != ResourceTarget::Storage(p.sr)
        || p.operation != *op
        || op.validate(r.target).is_err()
        || p.kinds != Plan::kinds(op)
        || p.evidence.len() != p.kinds.len()
        || r.steps.len() != p.kinds.len()
        || p.tasks_before.len() > 100_000
        || p.tasks_before.iter().any(|id| id.uuid().is_nil())
        || p.backend.is_empty()
        || p.backend.len() > 128
    {
        return Err(Error::Journal);
    }
    let ids = match op {
        StorageMaintenance::Repair { attachments, .. } => attachments
            .iter()
            .map(|p| p.attachment)
            .collect::<BTreeSet<_>>(),
        _ => BTreeSet::new(),
    };
    if p.configuration.keys().copied().collect::<BTreeSet<_>>() != ids
        || p.configuration
            .values()
            .any(|v| v.len() != 64 || !v.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err(Error::Journal);
    }
    let mut tasks = BTreeSet::new();
    for (index, ((kind, e), step)) in p.kinds.iter().zip(&p.evidence).zip(&r.steps).enumerate() {
        if step.label != kind.label()
            || e.task
                .as_ref()
                .is_some_and(|v| crate::snapshots::reference(&serde_json::json!(v)).is_err())
            || e.task_uuid
                .is_some_and(|id| id.uuid().is_nil() || !tasks.insert(id))
            || ((e.task.is_some() || e.completed || e.ended || e.verified)
                && step.state == StepState::Pending)
            || (step.state == StepState::Verified && !e.verified)
            || (e.verified && !e.completed)
            || (e.completed && !e.ended)
            || (e.task_uuid.is_some() && e.task.is_none())
            || (kind.asynchronous() && e.ended && e.task_uuid.is_none())
            || e.task_uuid.is_some_and(|id| p.tasks_before.contains(&id))
            || (!kind.asynchronous() && e.task.is_some())
            || (index > 0
                && (step.state != StepState::Pending || e.completed || e.task.is_some())
                && !p.evidence[index - 1].verified)
            || (r.state == OperationState::Succeeded
                && (!e.verified || step.state != StepState::Verified))
        {
            return Err(Error::Journal);
        }
    }
    Ok(())
}
