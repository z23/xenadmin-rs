use super::*;
use crate::{OwnedDiskSource, VdiId, operations::Journal};
impl ResourceOperationRecord {
    pub fn owned_disk_candidates(&self) -> Vec<VdiId> {
        if !self.state.terminal() {
            return Vec::new();
        }
        let mut result = Vec::new();
        if let Some(p) = &self.disk_plan
            && matches!(p.operation, crate::DiskOperation::Copy { .. })
            && p.evidence
                .first()
                .is_some_and(|e| e.completed && e.verified && e.result.is_some())
            && let Some(id) = p.created
            && !p.disks_before.contains(&id)
            && self.created == Some(ResourceTarget::Disk(id))
        {
            result.push(id);
        }
        if let Some(p) = &self.transfer_plan
            && let Some(c) = &p.disk_creation
            && let Some(id) = c.uuid
            && c.dispatched
            && c.reference.is_some()
            && !c.baseline.contains(&id.uuid())
            && self.transfer_created.disks.contains(&id)
        {
            result.push(id);
        }
        result.sort();
        result.dedup();
        result
    }
}
impl Journal {
    pub(crate) fn disk_ownership(
        &self,
        pool: PoolId,
        disk: VdiId,
        source: OwnedDiskSource,
        fresh: &serde_json::Value,
    ) -> Result<(), Error> {
        let marker = match source {
            OwnedDiskSource::VmOperation(id) => {
                let r = self.load(id)?.ok_or(Error::ResourceConflict)?;
                if r.pool != pool {
                    return Err(Error::ResourceConflict);
                }
                r.owned_disk_marker(disk).ok_or(Error::Protocol(
                    "The originating VM operation does not prove ownership of this disk",
                ))?
            }
            OwnedDiskSource::ResourceOperation(id) => {
                let r = self.resource_load(id)?.ok_or(Error::ResourceConflict)?;
                if r.pool != pool || !r.owned_disk_candidates().contains(&disk) {
                    return Err(Error::Protocol(
                        "Resolve the originating operation and verify its exact created disk before cleanup",
                    ));
                }
                if r.disk_plan
                    .as_ref()
                    .is_some_and(|p| p.created == Some(disk))
                {
                    None
                } else {
                    Some(("xenadmin-rs-disk-transfer", id.to_string()))
                }
            }
        };
        if let Some((key, value)) = marker
            && fresh["other_config"][key] != value
        {
            return Err(Error::Protocol(
                "The disk ownership marker no longer matches its originating operation",
            ));
        }
        Ok(())
    }
    pub(crate) fn disk_evidence(
        &self,
        evidence: &ResourceOperationRecord,
        index: usize,
    ) -> Result<ResourceOperationRecord, Error> {
        let p = evidence.disk_plan.as_ref().ok_or(Error::Journal)?;
        let incoming = p.evidence.get(index).ok_or(Error::Journal)?;
        for _ in 0..16 {
            let mut current = self.resource_load(evidence.id)?.ok_or(Error::Journal)?;
            if current.pool != evidence.pool
                || current.target != evidence.target
                || current.action != evidence.action
            {
                return Err(Error::JournalChanged);
            }
            let plan = current.disk_plan.as_mut().ok_or(Error::Journal)?;
            let step = plan.evidence.get_mut(index).ok_or(Error::Journal)?;
            macro_rules! merge {
                ($field:ident) => {
                    if step.$field.is_some()
                        && incoming.$field.is_some()
                        && step.$field != incoming.$field
                    {
                        return Err(Error::JournalChanged);
                    }
                    if step.$field.is_none() {
                        step.$field = incoming.$field.clone();
                    }
                };
            }
            merge!(task);
            merge!(task_uuid);
            merge!(result);
            step.completed |= incoming.completed;
            step.ended |= incoming.ended;
            step.verified |= incoming.verified;
            if plan.created.is_some() && p.created.is_some() && plan.created != p.created
                || plan.actual_size.is_some()
                    && p.actual_size.is_some()
                    && plan.actual_size != p.actual_size
            {
                return Err(Error::JournalChanged);
            }
            if plan.created.is_none() {
                plan.created = p.created;
            }
            if plan.actual_size.is_none() {
                plan.actual_size = p.actual_size;
            }
            current.created = plan.created.map(ResourceTarget::Disk);
            current.disk_size = plan.actual_size;
            if !current.state.terminal()
                && !current.interrupted
                && current.state != OperationState::OutcomeUnknown
            {
                current.state = OperationState::Verifying;
            }
            match self.resource_save(&mut current) {
                Ok(()) => return Ok(current),
                Err(Error::JournalChanged) => continue,
                Err(e) => return Err(e),
            }
        }
        Err(Error::JournalChanged)
    }
}
