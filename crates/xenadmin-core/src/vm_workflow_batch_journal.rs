use super::*;
use crate::vm_workflow_batch::{ChildPermit, Plan};
use rusqlite::params;

impl Journal {
    pub(crate) fn workflow_owner(&self, child: OperationId) -> Result<Option<OperationId>, Error> {
        let link: Option<(String, usize, String)> = self
            .0
            .query_row(
                "SELECT parent,ordinal,kind FROM workflow_batch_children WHERE child=?1",
                [child.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(|_| Error::Journal)?;
        let Some((parent, index, kind)) = link else {
            return Ok(None);
        };
        let parent: OperationId =
            serde_json::from_value(json!(parent)).map_err(|_| Error::Journal)?;
        let record = self.resource_load(parent)?.ok_or(Error::Journal)?;
        self.workflow_validate_links(&record)?;
        let plan = record.workflow_batch_plan.as_ref().ok_or(Error::Journal)?;
        if kind != "vm" || plan.children.get(index) != Some(&child) {
            return Err(Error::Journal);
        }
        Ok(Some(parent))
    }
    pub(crate) fn workflow_validate_links(
        &self,
        record: &crate::ResourceOperationRecord,
    ) -> Result<(), Error> {
        let Some(p) = &record.workflow_batch_plan else {
            return Ok(());
        };
        crate::vm_workflow_batch::validate_record(record)?;
        let mut q = self.0.prepare("SELECT ordinal,child,kind FROM workflow_batch_children WHERE parent=?1 ORDER BY ordinal LIMIT 33").map_err(|_|Error::Journal)?;
        let rows = q
            .query_map([record.id.to_string()], |r| {
                Ok((
                    r.get::<_, usize>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .map_err(|_| Error::Journal)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| Error::Journal)?;
        if rows.len() != p.children.len()
            || rows.iter().enumerate().any(|(i, (n, id, kind))| {
                *n != i || *id != p.children[i].to_string() || kind != "vm"
            })
        {
            return Err(Error::Journal);
        }
        Ok(())
    }
    pub(crate) fn workflow_insert_links(
        &self,
        r: &crate::ResourceOperationRecord,
    ) -> Result<(), Error> {
        let Some(p) = &r.workflow_batch_plan else {
            return Ok(());
        };
        for (i, child) in p.children.iter().enumerate() {
            if self.load(*child)?.is_some() || self.resource_load(*child)?.is_some() {
                return Err(Error::OperationConflict);
            }
            self.0.execute("INSERT INTO workflow_batch_children(parent,ordinal,child,kind) VALUES(?1,?2,?3,'vm')", params![r.id.to_string(),i,child.to_string()]).map_err(|_|Error::OperationConflict)?;
        }
        Ok(())
    }
    /// Read children without recursive owner lookup; compare every stored identity with the plan.
    pub(crate) fn workflow_validate_persisted(
        &self,
        parent: &crate::ResourceOperationRecord,
    ) -> Result<(), Error> {
        let Some(plan) = &parent.workflow_batch_plan else {
            return Ok(());
        };
        self.workflow_validate_links(parent)?;
        for (index, id) in plan.children.iter().enumerate() {
            let payload: Option<(String, Option<String>)> = self
                .0
                .query_row(
                    "SELECT payload,task FROM operations WHERE id=?1",
                    [id.to_string()],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
                .map_err(|_| Error::Journal)?;
            if index >= plan.admitted {
                if payload.is_some() {
                    return Err(Error::Journal);
                }
                continue;
            }
            let (payload, task) = payload.ok_or(Error::Journal)?;
            let mut child = Self::decode_record(&payload)?;
            child.task = task;
            if !crate::migration_backend::valid_receipts(&child, child.migration_plan.as_deref()) {
                return Err(Error::Journal);
            }
            self.workflow_child_matches(&child, parent, index)?;
            if parent.state.terminal()
                && (!child.state.terminal()
                    || parent.workflow_batch[index].state.as_ref() != Some(&child.state))
            {
                return Err(Error::Journal);
            }
        }
        Ok(())
    }
    pub(crate) fn workflow_permit(
        &self,
        parent: OperationId,
        index: usize,
    ) -> Result<ChildPermit, Error> {
        let record = self.resource_load(parent)?.ok_or(Error::Journal)?;
        self.workflow_validate_links(&record)?;
        if index
            >= record
                .workflow_batch_plan
                .as_ref()
                .ok_or(Error::Journal)?
                .children
                .len()
        {
            return Err(Error::Journal);
        }
        Ok(ChildPermit { parent, index })
    }
    fn workflow_child_matches(
        &self,
        record: &OperationRecord,
        parent: &crate::ResourceOperationRecord,
        index: usize,
    ) -> Result<(), Error> {
        let plan = parent.workflow_batch_plan.as_ref().ok_or(Error::Journal)?;
        let item = plan.selection.items.get(index).ok_or(Error::Journal)?;
        if plan.children.get(index) != Some(&record.id)
            || record.batch_parent != Some(parent.id)
            || parent.pool != record.pool
            || item.vm != record.vm
            || item.workflow.action() != record.action
        {
            return Err(Error::OperationConflict);
        }
        Ok(())
    }
    pub(super) fn workflow_validate_child(&self, record: &OperationRecord) -> Result<(), Error> {
        let owner = self.workflow_owner(record.id)?;
        if owner != record.batch_parent {
            return Err(Error::Journal);
        }
        if let Some(owner) = owner {
            let parent = self.resource_load(owner)?.ok_or(Error::Journal)?;
            let plan = parent.workflow_batch_plan.as_ref().ok_or(Error::Journal)?;
            let index = plan
                .children
                .iter()
                .position(|id| *id == record.id)
                .ok_or(Error::Journal)?;
            if index >= plan.admitted {
                return Err(Error::Journal);
            }
            self.workflow_child_matches(record, &parent, index)?;
        }
        Ok(())
    }
    /// Called only inside the child's admission transaction.
    pub(super) fn workflow_admit_child(
        &self,
        record: &OperationRecord,
        permit: Option<&ChildPermit>,
    ) -> Result<Option<OperationId>, Error> {
        let owner = self.workflow_owner(record.id)?;
        let Some(permit) = permit else {
            return if owner.is_some() || record.batch_parent.is_some() {
                Err(Error::OperationConflict)
            } else {
                Ok(None)
            };
        };
        if owner != Some(permit.parent) {
            return Err(Error::OperationConflict);
        }
        let parent = self.resource_load(permit.parent)?.ok_or(Error::Journal)?;
        self.workflow_child_matches(record, &parent, permit.index)?;
        if parent.interrupted
            || parent.state.terminal()
            || parent.cancel_requested
            || parent.state == OperationState::OutcomeUnknown
        {
            return Err(Error::JournalChanged);
        }
        let plan = parent.workflow_batch_plan.as_ref().ok_or(Error::Journal)?;
        if let Some(old) = self.load(record.id)? {
            self.workflow_child_matches(&old, &parent, permit.index)?;
            if permit.index >= plan.admitted {
                return Err(Error::Journal);
            }
            return Ok(owner);
        }
        if plan.admitted != permit.index {
            return Err(Error::JournalChanged);
        }
        for (index, id) in plan.children.iter().take(permit.index).enumerate() {
            let old = self.load(*id)?.ok_or(Error::Journal)?;
            self.workflow_child_matches(&old, &parent, index)?;
            if old.state != OperationState::Succeeded {
                return Err(Error::JournalChanged);
            }
        }
        Ok(owner)
    }
    pub(super) fn workflow_mark_admitted(
        &self,
        record: &OperationRecord,
        permit: &ChildPermit,
    ) -> Result<(), Error> {
        let mut parent = self.resource_load(permit.parent)?.ok_or(Error::Journal)?;
        self.workflow_child_matches(record, &parent, permit.index)?;
        parent
            .workflow_batch_plan
            .as_mut()
            .ok_or(Error::Journal)?
            .admitted += 1;
        parent.workflow_batch[permit.index].state = Some(record.state.clone());
        parent.workflow_batch[permit.index].detail =
            "Child admitted; no completion inferred".into();
        parent.state = OperationState::Tracking;
        parent.can_cancel = true;
        parent.steps[permit.index].state = crate::StepState::Dispatching;
        parent.steps[permit.index].detail =
            "Child admitted; its durable workflow owns this VM".into();
        self.resource_save(&mut parent)?;
        Ok(())
    }
    pub(crate) fn workflow_child_stopped(&self, child: OperationId) -> Result<bool, Error> {
        let Some(parent) = self.workflow_owner(child)? else {
            return Ok(false);
        };
        let r = self.resource_load(parent)?.ok_or(Error::Journal)?;
        Ok(r.interrupted || r.state.terminal() || r.state == OperationState::OutcomeUnknown)
    }
    pub(super) fn save_vm_dispatch(&self, r: &mut OperationRecord) -> Result<(), Error> {
        self.0
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(|_| Error::Journal)?;
        let result = (|| {
            if self.workflow_child_stopped(r.id)? {
                return Err(Error::JournalChanged);
            }
            self.save(r)
        })();
        self.workflow_transaction(result)
    }
    fn workflow_transaction<T>(&self, result: Result<T, Error>) -> Result<T, Error> {
        match result {
            Ok(v) => {
                self.0.execute_batch("COMMIT").map_err(|_| Error::Journal)?;
                Ok(v)
            }
            Err(e) => {
                let _ = self.0.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }
    /// Recovery and acknowledgement fence both rows before releasing either reservation.
    pub(crate) fn workflow_interrupt(
        &self,
        pool: PoolId,
        id: OperationId,
        acknowledge: bool,
    ) -> Result<crate::ResourceOperationRecord, Error> {
        self.0
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(|_| Error::Journal)?;
        let result = (|| {
            let mut parent = self.resource_load(id)?.ok_or(Error::Journal)?;
            self.workflow_validate_links(&parent)?;
            if parent.pool != pool
                || parent.state.terminal()
                || (acknowledge && parent.state != OperationState::OutcomeUnknown)
            {
                return Err(Error::UnavailableAction);
            }
            let plan = parent
                .workflow_batch_plan
                .as_ref()
                .ok_or(Error::Journal)?
                .clone();
            let mut active = 0;
            for (index, id) in plan.children.iter().take(plan.admitted).enumerate() {
                let mut child = self.load(*id)?.ok_or(Error::Journal)?;
                self.workflow_child_matches(&child, &parent, index)?;
                if !child.state.terminal() {
                    active += 1;
                    child.state = if acknowledge {
                        OperationState::AcknowledgedUnknown
                    } else {
                        OperationState::OutcomeUnknown
                    };
                    child.can_cancel = false;
                    child.detail = "Batch interrupted; unsent child steps will not run".into();
                    self.save(&mut child)?;
                }
                parent.workflow_batch[index].state = Some(child.state.clone());
                parent.workflow_batch[index].detail = child.detail.chars().take(1024).collect();
                parent.steps[index].state = if child.state == OperationState::Succeeded {
                    crate::StepState::Verified
                } else if child.state == OperationState::AcknowledgedUnknown
                    || child.state == OperationState::OutcomeUnknown
                {
                    crate::StepState::OutcomeUnknown
                } else {
                    crate::StepState::Failed
                };
            }
            if active > 1 {
                return Err(Error::Journal);
            }
            parent.interrupted = true;
            parent.state = if acknowledge {
                OperationState::AcknowledgedUnknown
            } else {
                OperationState::OutcomeUnknown
            };
            parent.can_cancel = false;
            parent.detail = if acknowledge { "Unknown batch and active child acknowledged together; this does not establish host success" } else { "Batch interrupted; recheck reads admitted children and never starts remaining VMs" }.into();
            self.resource_save(&mut parent)?;
            Ok(parent)
        })();
        self.workflow_transaction(result)
    }
    pub(crate) fn workflow_prune(&self) -> Result<(), Error> {
        // Called under the admission transaction; unresolved children always retain their parent.
        let mut q = self.0.prepare("SELECT id FROM resource_operations WHERE terminal=1 AND id IN (SELECT parent FROM workflow_batch_children) AND rowid NOT IN (SELECT rowid FROM resource_operations WHERE terminal=1 ORDER BY rowid DESC LIMIT 500)").map_err(|_|Error::Journal)?;
        let ids = q
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|_| Error::Journal)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| Error::Journal)?;
        for id in ids {
            let id: OperationId = serde_json::from_value(json!(id)).map_err(|_| Error::Journal)?;
            let p = self.resource_load(id)?.ok_or(Error::Journal)?;
            self.workflow_validate_links(&p)?;
            let Plan {
                children, admitted, ..
            } = p.workflow_batch_plan.ok_or(Error::Journal)?;
            if children.iter().take(admitted).any(|id| {
                self.load(*id)
                    .map(|r| r.is_none_or(|r| !r.state.terminal()))
                    .unwrap_or(true)
            }) {
                continue;
            }
            for child in children {
                self.0
                    .execute(
                        "DELETE FROM operations WHERE id=?1 AND terminal=1",
                        [child.to_string()],
                    )
                    .map_err(|_| Error::Journal)?;
            }
            self.0
                .execute(
                    "DELETE FROM workflow_batch_children WHERE parent=?1",
                    [id.to_string()],
                )
                .map_err(|_| Error::Journal)?;
            self.0
                .execute(
                    "DELETE FROM resource_operations WHERE id=?1 AND terminal=1",
                    [id.to_string()],
                )
                .map_err(|_| Error::Journal)?;
        }
        Ok(())
    }
}
