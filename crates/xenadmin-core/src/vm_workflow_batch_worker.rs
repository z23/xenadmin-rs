use super::*;
use crate::{OperationAction, vm_workflow_batch::Plan};

impl ResourceRequest {
    pub(super) async fn execute_workflow_batch(
        &mut self,
        j: &mut Journal,
        s: &Session,
        c: &CancellationToken,
    ) -> Result<(), Error> {
        self.record
            .action
            .permission_requirements(self.record.target)
            .guard(s.permissions())?;
        self.record.can_cancel = true;
        self.record.detail =
            "Checking every selected VM and explicit deletion before the first child".into();
        self.save(j).await?;
        let plan = self
            .record
            .workflow_batch_plan
            .clone()
            .ok_or(Error::Journal)?;
        tokio::select! { _=c.cancelled()=>return Err(Error::Closed), value=async {
            crate::power_batch::pool(s, &self.record).await?;
            for item in &plan.selection.items {
                let OperationAction::Lifecycle(op) = item.workflow.action() else { return Err(Error::InvalidResourceCommand) };
                crate::vm_lifecycle_backend::plan(s,item.vm,&op).await?;
            }
            Ok::<_,Error>(())
        }=>value? }
        for index in 0..plan.children.len() {
            self.reload(j)?;
            if self.stopped_after_recovery() {
                return Ok(());
            }
            if self.record.cancel_requested {
                return self.finish_workflow_batch(j).await;
            }
            if c.is_cancelled() {
                return Err(Error::Closed);
            }
            crate::power_batch::pool(s, &self.record).await?;
            let item = &plan.selection.items[index];
            let reference = s
                .call("VM.get_by_uuid", vec![json!(item.vm.to_string())])
                .await?;
            let reference = crate::xapi::opaque_reference(&reference)?;
            let (_, mut child) = crate::operations::channel(
                plan.children[index],
                self.record.pool,
                item.vm,
                self.generation,
                item.workflow.action(),
                tokio::runtime::Handle::current(),
            );
            child.record.batch_parent = Some(self.record.id);
            child.batch_permit = Some(j.workflow_permit(self.record.id, index)?);
            let directory = self.directory.clone().ok_or(Error::Journal)?;
            let updates = self.updates.clone().ok_or(Error::Journal)?;
            child
                .run_session(s, Some(reference), directory, c.clone(), updates)
                .await;
            self.reload(j)?;
            self.refresh_workflow_results(j).await?;
            if self.stopped_after_recovery() {
                return Ok(());
            }
            if self.record.workflow_batch[index].state != Some(OperationState::Succeeded) {
                return self.finish_workflow_batch(j).await;
            }
        }
        self.finish_workflow_batch(j).await
    }
    pub(super) async fn reconcile_workflow_batch(
        &mut self,
        j: &mut Journal,
        s: &Session,
        c: &CancellationToken,
    ) -> Result<(), Error> {
        self.reload(j)?;
        if self.record.state.terminal() {
            return Ok(());
        }
        self.record = j.workflow_interrupt(self.record.pool, self.record.id, false)?;
        self.record.state = OperationState::Verifying;
        self.record.detail = "Reading admitted child evidence; no remaining VM will start".into();
        self.save(j).await?;
        let plan = self
            .record
            .workflow_batch_plan
            .clone()
            .ok_or(Error::Journal)?;
        for (index, id) in plan.children.iter().take(plan.admitted).enumerate() {
            let record = j.load(*id)?.ok_or(Error::Journal)?;
            if record.state.terminal() {
                continue;
            }
            let mut child = crate::operations::MutationRequest::tracking(record);
            child.batch_permit = Some(j.workflow_permit(self.record.id, index)?);
            child
                .run_session(
                    s,
                    None,
                    self.directory.clone().ok_or(Error::Journal)?,
                    c.clone(),
                    self.updates.clone().ok_or(Error::Journal)?,
                )
                .await;
            self.reload(j)?;
            self.refresh_workflow_results(j).await?;
            if self.record.state.terminal() {
                return Ok(());
            }
        }
        self.finish_workflow_batch(j).await
    }
    async fn refresh_workflow_results(&mut self, j: &mut Journal) -> Result<(), Error> {
        for _ in 0..16 {
            self.reload(j)?;
            j.workflow_validate_links(&self.record)?;
            let Plan {
                children,
                admitted,
                selection,
            } = self
                .record
                .workflow_batch_plan
                .clone()
                .ok_or(Error::Journal)?;
            for (index, id) in children.iter().take(admitted).enumerate() {
                let child = j.load(*id)?.ok_or(Error::Journal)?;
                if child.batch_parent != Some(self.record.id)
                    || child.pool != self.record.pool
                    || child.vm != selection.items[index].vm
                    || child.action != selection.items[index].workflow.action()
                {
                    return Err(Error::Journal);
                }
                self.record.workflow_batch[index].state = Some(child.state.clone());
                self.record.workflow_batch[index].detail =
                    child.detail.chars().take(1024).collect();
                self.record.steps[index].state = match child.state {
                    OperationState::Succeeded => StepState::Verified,
                    OperationState::OutcomeUnknown | OperationState::AcknowledgedUnknown => {
                        StepState::OutcomeUnknown
                    }
                    state if state.terminal() => StepState::Failed,
                    _ => StepState::Dispatching,
                };
                self.record.steps[index].detail = self.record.workflow_batch[index].detail.clone();
            }
            match self.save(j).await {
                Ok(()) => return Ok(()),
                Err(Error::JournalChanged) => continue,
                Err(e) => return Err(e),
            }
        }
        Err(Error::JournalChanged)
    }
    async fn finish_workflow_batch(&mut self, j: &mut Journal) -> Result<(), Error> {
        self.refresh_workflow_results(j).await?;
        if self.stopped_after_recovery() {
            return Ok(());
        }
        let p = self
            .record
            .workflow_batch_plan
            .as_ref()
            .ok_or(Error::Journal)?;
        let unresolved = self
            .record
            .workflow_batch
            .iter()
            .take(p.admitted)
            .any(|r| r.state.as_ref().is_none_or(|s| !s.terminal()));
        if unresolved {
            self.record = j.workflow_interrupt(self.record.pool, self.record.id, false)?;
            self.announce().await;
            return Ok(());
        }
        let succeeded = self
            .record
            .workflow_batch
            .iter()
            .filter(|r| r.state == Some(OperationState::Succeeded))
            .count();
        let partial = self
            .record
            .workflow_batch
            .iter()
            .any(|r| r.state == Some(OperationState::PartiallyCompleted));
        self.record.state = if succeeded == p.children.len() {
            OperationState::Succeeded
        } else if succeeded > 0 || partial {
            OperationState::PartiallyCompleted
        } else if self.record.cancel_requested || p.admitted == 0 {
            OperationState::Cancelled
        } else {
            OperationState::Failed
        };
        self.record.can_cancel = false;
        self.record.progress = Some(succeeded as f64 / p.children.len() as f64);
        self.record.detail = format!(
            "{succeeded} of {} VM deletions verified; {} not started. No rollback or automatic continuation.",
            p.children.len(),
            p.children.len() - p.admitted
        );
        for step in self.record.steps.iter_mut().skip(p.admitted) {
            step.state = StepState::Pending;
            step.detail = "Not started; a new review is required".into();
        }
        self.save(j).await
    }
}
