use crate::{
    Error, HostId, OperationState, PermissionRequirements, PoolId, ResourceAction,
    ResourceOperationRecord, ResourceTarget, StepState, TaskId,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoolDissolveMember {
    pub host: HostId,
    pub name: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoolDissolve {
    pub coordinator: HostId,
    pub expected_name: String,
    pub expected_description: String,
    pub members: Vec<PoolDissolveMember>,
}
impl PoolDissolve {
    pub fn capture(inventory: &crate::Inventory) -> Result<Self, Error> {
        let pool = inventory
            .details
            .pool
            .as_ref()
            .ok_or(Error::UnavailableAction)?;
        let coordinator = pool
            .administration
            .coordinator
            .ok_or(Error::UnavailableAction)?;
        if pool.id != inventory.pool_id
            || !inventory.hosts.contains_key(&coordinator)
            || pool.administration.ha_enabled != Some(false)
        {
            return Err(Error::UnavailableAction);
        }
        let operation = Self {
            coordinator,
            expected_name: pool.name.clone(),
            expected_description: pool.description.clone(),
            members: inventory
                .hosts
                .values()
                .filter(|h| h.id != coordinator)
                .map(|h| PoolDissolveMember {
                    host: h.id,
                    name: h.name.clone(),
                })
                .collect(),
        };
        operation.validate(ResourceTarget::Pool(inventory.pool_id))?;
        Ok(operation)
    }
    pub fn permission_requirements(&self) -> PermissionRequirements {
        let mut methods = vec!["pool.set_name_label"];
        if !self.expected_description.is_empty() {
            methods.push("pool.set_name_description");
        }
        if !self.members.is_empty() {
            methods.push("pool.eject");
        }
        PermissionRequirements::writes(methods)
    }
    pub(crate) fn validate(&self, target: ResourceTarget) -> Result<(), Error> {
        if !matches!(target, ResourceTarget::Pool(_))
            || self.coordinator.uuid().is_nil()
            || self.expected_name.is_empty()
            || self.expected_name.len() > 4096
            || self.expected_description.len() > 16384
            || self.expected_name.contains('\0')
            || self.expected_description.contains('\0')
            || self.members.len() > 127
            || self.members.iter().any(|h| {
                h.host == self.coordinator
                    || h.host.uuid().is_nil()
                    || h.name.len() > 4096
                    || h.name.contains('\0')
            })
            || self
                .members
                .windows(2)
                .any(|pair| pair[0].host >= pair[1].host)
        {
            return Err(Error::InvalidResourceCommand);
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoolDissolveResult {
    pub ejected: Vec<HostId>,
    pub name_cleared: bool,
    pub description_cleared: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Kind {
    Eject(HostId),
    ClearName,
    ClearDescription,
}
impl Kind {
    pub(crate) fn label(&self) -> String {
        match self {
            Self::Eject(host) => format!("Eject pool member {host}"),
            Self::ClearName => "Clear the remaining pool name".into(),
            Self::ClearDescription => "Clear the remaining pool description".into(),
        }
    }
    pub(crate) fn method(&self) -> &'static str {
        match self {
            Self::Eject(_) => "Async.pool.eject",
            Self::ClearName => "pool.set_name_label",
            Self::ClearDescription => "pool.set_name_description",
        }
    }
    pub(crate) fn asynchronous(&self) -> bool {
        matches!(self, Self::Eject(_))
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Evidence {
    pub sent: bool,
    pub task: Option<String>,
    pub task_uuid: Option<TaskId>,
    pub ended: bool,
    pub completed: bool,
    pub verified: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Plan {
    pub pool: PoolId,
    pub operation: PoolDissolve,
    pub evidence: Vec<Evidence>,
    pub tasks_before: BTreeSet<TaskId>,
}
impl Plan {
    pub(crate) fn kinds(op: &PoolDissolve) -> Vec<Kind> {
        let mut kinds = op
            .members
            .iter()
            .map(|m| Kind::Eject(m.host))
            .collect::<Vec<_>>();
        kinds.push(Kind::ClearName);
        if !op.expected_description.is_empty() {
            kinds.push(Kind::ClearDescription);
        }
        kinds
    }
    pub(crate) fn extends(&self, old: &Self) -> bool {
        self.pool == old.pool
            && self.operation == old.operation
            && self.tasks_before == old.tasks_before
            && self.evidence.len() == old.evidence.len()
            && self.evidence.iter().zip(&old.evidence).all(|(n, o)| {
                (!o.sent || n.sent)
                    && (!o.completed || n.completed)
                    && (!o.ended || n.ended)
                    && (!o.verified || n.verified)
                    && (o.task.is_none() || o.task == n.task)
                    && (o.task_uuid.is_none() || o.task_uuid == n.task_uuid)
            })
    }
    pub(crate) fn result(&self) -> PoolDissolveResult {
        let mut result = PoolDissolveResult::default();
        for (kind, e) in Self::kinds(&self.operation).iter().zip(&self.evidence) {
            if !e.verified {
                continue;
            }
            match kind {
                Kind::Eject(host) => result.ejected.push(*host),
                Kind::ClearName => result.name_cleared = true,
                Kind::ClearDescription => result.description_cleared = true,
            }
        }
        result
    }
}
pub(crate) fn validate_record(r: &ResourceOperationRecord) -> Result<(), Error> {
    let Some(p) = &r.pool_dissolve_plan else {
        return if r.pool_dissolve.is_some()
            || matches!(r.action, ResourceAction::DissolvePool(_))
                && (!r.steps.is_empty()
                    || !matches!(
                        r.state,
                        OperationState::Preparing
                            | OperationState::Rejected
                            | OperationState::Cancelled
                            | OperationState::OutcomeUnknown
                            | OperationState::AcknowledgedUnknown
                    ))
        {
            Err(Error::Journal)
        } else {
            Ok(())
        };
    };
    let kinds = Plan::kinds(&p.operation);
    if r.action != ResourceAction::DissolvePool(p.operation.clone())
        || r.target != ResourceTarget::Pool(p.pool)
        || r.pool != p.pool
        || p.operation.validate(r.target).is_err()
        || p.evidence.len() != kinds.len()
        || r.steps.len() != kinds.len()
        || p.tasks_before.len() > 100_000
        || p.tasks_before.iter().any(|id| id.uuid().is_nil())
        || r.pool_dissolve.as_ref() != Some(&p.result())
        || r.task.is_some()
        || r.result_reference.is_some()
    {
        return Err(Error::Journal);
    }
    let mut tasks = BTreeSet::new();
    let mut references = BTreeSet::new();
    for (index, ((kind, e), step)) in kinds.iter().zip(&p.evidence).zip(&r.steps).enumerate() {
        if step.label != kind.label()
            || (step.state == StepState::Verified && !e.verified)
            || (e.verified && !e.completed)
            || (e.completed && !e.ended)
            || (e.ended && !e.sent)
            || (e.task.is_some() && !e.sent)
            || (e.task_uuid.is_some() && e.task.is_none())
            || (kind.asynchronous() && e.ended && e.task_uuid.is_none())
            || (!kind.asynchronous() && (e.task.is_some() || e.task_uuid.is_some()))
            || e.task_uuid.is_some_and(|id| {
                id.uuid().is_nil() || p.tasks_before.contains(&id) || !tasks.insert(id)
            })
            || e.task.as_ref().is_some_and(|v| {
                crate::snapshots::reference(&serde_json::json!(v)).is_err() || !references.insert(v)
            })
            || (step.state == StepState::Pending && e.sent)
            || (e.sent && index > 0 && !p.evidence[index - 1].verified)
            || (r.state == OperationState::Succeeded
                && (!e.verified || step.state != StepState::Verified))
        {
            return Err(Error::Journal);
        }
    }
    Ok(())
}
