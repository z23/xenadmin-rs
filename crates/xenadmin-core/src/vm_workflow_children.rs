use super::*;
use crate::ResourceOperationRecord;

pub(crate) enum WorkflowChild {
    Vm(Box<OperationRecord>),
    Resource(Box<ResourceOperationRecord>),
}
impl WorkflowChild {
    pub(crate) fn state(&self) -> &OperationState {
        match self {
            Self::Vm(r) => &r.state,
            Self::Resource(r) => &r.state,
        }
    }
    pub(crate) fn detail(&self) -> &str {
        match self {
            Self::Vm(r) => &r.detail,
            Self::Resource(r) => &r.detail,
        }
    }
    fn matches(&self, parent: &ResourceOperationRecord, index: usize) -> Result<(), Error> {
        let plan = parent.workflow_batch_plan.as_ref().ok_or(Error::Journal)?;
        let item = plan.selection.items.get(index).ok_or(Error::Journal)?;
        let expected = plan.children.get(index).ok_or(Error::Journal)?;
        let valid = match self {
            Self::Vm(r) => {
                item.workflow.kind() == "vm"
                    && r.id == *expected
                    && r.batch_parent == Some(parent.id)
                    && r.pool == parent.pool
                    && r.vm == item.vm
                    && r.action == item.workflow.action()
            }
            Self::Resource(r) => {
                item.workflow.kind() == "resource"
                    && r.id == *expected
                    && r.batch_parent == Some(parent.id)
                    && r.pool == parent.pool
                    && r.target == crate::ResourceTarget::Pool(parent.pool)
                    && item
                        .workflow
                        .transfer(item.vm)
                        .is_some_and(|a| r.action == crate::ResourceAction::Transfer(a))
                    && r.transfer_plan
                        .as_ref()
                        .is_some_and(|p| plan.outputs.get(index) == Some(&p.file))
                    && r.workflow_batch_plan.is_none()
            }
        };
        if valid {
            Ok(())
        } else {
            Err(Error::OperationConflict)
        }
    }
}
impl Journal {
    pub(crate) fn resource_dispatch(&self, r: &mut ResourceOperationRecord) -> Result<(), Error> {
        self.0
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(|_| Error::Journal)?;
        let result = (|| {
            let current = self.resource_load(r.id)?.ok_or(Error::Journal)?;
            if current.state.terminal()
                || current.interrupted
                || current.state == OperationState::OutcomeUnknown
                || self.workflow_child_stopped(r.id)?
            {
                return Err(Error::JournalChanged);
            }
            self.resource_save(r)
        })();
        match result {
            Ok(()) => self.0.execute_batch("COMMIT").map_err(|_| Error::Journal),
            Err(e) => {
                let _ = self.0.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }
    pub(crate) fn workflow_load_child(
        &self,
        parent: &ResourceOperationRecord,
        index: usize,
    ) -> Result<Option<WorkflowChild>, Error> {
        let p = parent.workflow_batch_plan.as_ref().ok_or(Error::Journal)?;
        let id = *p.children.get(index).ok_or(Error::Journal)?;
        let item = p.selection.items.get(index).ok_or(Error::Journal)?;
        let child = if item.workflow.kind() == "resource" {
            if self.raw_vm_child(id)?.is_some() {
                return Err(Error::Journal);
            }
            self.resource_load_linked(id, false)?
                .map(|r| WorkflowChild::Resource(Box::new(r)))
        } else {
            if self.resource_load_linked(id, false)?.is_some() {
                return Err(Error::Journal);
            }
            self.raw_vm_child(id)?
                .map(|r| WorkflowChild::Vm(Box::new(r)))
        };
        if let Some(child) = &child {
            child.matches(parent, index)?;
        }
        Ok(child)
    }
    fn raw_vm_child(&self, id: OperationId) -> Result<Option<OperationRecord>, Error> {
        let data: Option<(String, Option<String>, u64)> = self
            .0
            .query_row(
                "SELECT payload,task,revision FROM operations WHERE id=?1",
                [id.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(|_| Error::Journal)?;
        data.map(|(payload, task, revision)| {
            let mut r = Self::decode_record(&payload)?;
            r.task = task;
            r.revision = revision;
            if !crate::migration_backend::valid_receipts(&r, r.migration_plan.as_deref()) {
                return Err(Error::Journal);
            }
            Ok(r)
        })
        .transpose()
    }
    pub(crate) fn workflow_validate_resource_child(
        &self,
        r: &ResourceOperationRecord,
    ) -> Result<(), Error> {
        let owner = self.workflow_owner(r.id)?;
        if owner != r.batch_parent {
            return Err(Error::Journal);
        }
        if let Some(owner) = owner {
            let parent = self
                .resource_load_linked(owner, false)?
                .ok_or(Error::Journal)?;
            let p = parent.workflow_batch_plan.as_ref().ok_or(Error::Journal)?;
            let index = p
                .children
                .iter()
                .position(|id| *id == r.id)
                .ok_or(Error::Journal)?;
            if index >= p.admitted {
                return Err(Error::Journal);
            }
            WorkflowChild::Resource(Box::new(r.clone())).matches(&parent, index)?;
        }
        Ok(())
    }
    pub(crate) fn workflow_reconcile_resource_child(
        &self,
        record: &ResourceOperationRecord,
        permit: Option<&crate::vm_workflow_batch::ChildPermit>,
    ) -> Result<(), Error> {
        let owner = self.workflow_owner(record.id)?;
        match (owner, record.batch_parent, permit) {
            (None, None, None) => Ok(()),
            (Some(owner), Some(expected), Some(permit))
                if owner == expected && owner == permit.parent =>
            {
                let parent = self.resource_load(owner)?.ok_or(Error::Journal)?;
                let plan = parent.workflow_batch_plan.as_ref().ok_or(Error::Journal)?;
                if parent.state != OperationState::Verifying
                    || !parent.interrupted
                    || permit.index >= plan.admitted
                {
                    return Err(Error::OperationConflict);
                }
                WorkflowChild::Resource(Box::new(record.clone())).matches(&parent, permit.index)
            }
            _ => Err(Error::OperationConflict),
        }
    }
    pub(crate) fn workflow_admit_resource_child(
        &self,
        r: &ResourceOperationRecord,
        permit: Option<&crate::vm_workflow_batch::ChildPermit>,
    ) -> Result<Option<OperationId>, Error> {
        self.workflow_admit_any_child(
            WorkflowChild::Resource(Box::new(r.clone())),
            r.id,
            r.batch_parent,
            permit,
        )
    }
    pub(crate) fn workflow_admit_any_child(
        &self,
        child: WorkflowChild,
        id: OperationId,
        parent_id: Option<OperationId>,
        permit: Option<&crate::vm_workflow_batch::ChildPermit>,
    ) -> Result<Option<OperationId>, Error> {
        let owner = self.workflow_owner(id)?;
        let Some(permit) = permit else {
            return if owner.is_some() || parent_id.is_some() {
                Err(Error::OperationConflict)
            } else {
                Ok(None)
            };
        };
        if owner != Some(permit.parent) {
            return Err(Error::OperationConflict);
        }
        let parent = self.resource_load(permit.parent)?.ok_or(Error::Journal)?;
        child.matches(&parent, permit.index)?;
        if parent.interrupted
            || parent.state.terminal()
            || parent.cancel_requested
            || parent.state == OperationState::OutcomeUnknown
        {
            return Err(Error::JournalChanged);
        }
        let p = parent.workflow_batch_plan.as_ref().ok_or(Error::Journal)?;
        if self.workflow_load_child(&parent, permit.index)?.is_some() {
            return if permit.index < p.admitted {
                Ok(owner)
            } else {
                Err(Error::Journal)
            };
        }
        if p.admitted != permit.index {
            return Err(Error::JournalChanged);
        }
        for index in 0..permit.index {
            if self
                .workflow_load_child(&parent, index)?
                .is_none_or(|r| *r.state() != OperationState::Succeeded)
            {
                return Err(Error::JournalChanged);
            }
        }
        Ok(owner)
    }
    pub(crate) fn workflow_mark_resource_admitted(
        &self,
        r: &ResourceOperationRecord,
        p: &crate::vm_workflow_batch::ChildPermit,
    ) -> Result<(), Error> {
        self.workflow_mark_any_admitted(WorkflowChild::Resource(Box::new(r.clone())), p)
    }
    pub(crate) fn workflow_mark_any_admitted(
        &self,
        child: WorkflowChild,
        permit: &crate::vm_workflow_batch::ChildPermit,
    ) -> Result<(), Error> {
        // The child will be inserted in this transaction after advancing the parent's admitted count.
        let mut parent = self
            .resource_load_linked(permit.parent, false)?
            .ok_or(Error::Journal)?;
        child.matches(&parent, permit.index)?;
        parent
            .workflow_batch_plan
            .as_mut()
            .ok_or(Error::Journal)?
            .admitted += 1;
        parent.workflow_batch[permit.index].state = Some(child.state().clone());
        parent.workflow_batch[permit.index].detail =
            "Child admitted; no completion inferred".into();
        parent.state = OperationState::Tracking;
        parent.can_cancel = true;
        parent.steps[permit.index].state = crate::StepState::Dispatching;
        parent.steps[permit.index].detail =
            "Child admitted; its durable workflow owns this VM".into();
        self.resource_save(&mut parent)
    }
    pub(crate) fn workflow_interrupt_child(
        &self,
        child: &mut WorkflowChild,
        acknowledge: bool,
    ) -> Result<(), Error> {
        let state = if acknowledge {
            OperationState::AcknowledgedUnknown
        } else {
            OperationState::OutcomeUnknown
        };
        match child {
            WorkflowChild::Vm(r) => {
                r.state = state;
                r.can_cancel = false;
                r.detail = "Batch interrupted; unsent child steps will not run".into();
                self.save(r)
            }
            WorkflowChild::Resource(r) => {
                r.state = state;
                r.interrupted = true;
                r.can_cancel = false;
                r.detail =
                    "Batch interrupted; no further transfer or publication is authorized".into();
                self.resource_save(r)
            }
        }
    }
}
