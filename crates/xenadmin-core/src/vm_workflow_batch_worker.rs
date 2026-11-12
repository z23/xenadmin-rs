use super::*;
use crate::{OperationAction, operations::workflow_children::WorkflowChild};

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
        let plan = self
            .record
            .workflow_batch_plan
            .clone()
            .ok_or(Error::Journal)?;
        self.record.detail = format!(
            "Checking every selected {} and reviewed workflow before the first child",
            plan.selection.noun()
        );
        self.save(j).await?;
        if plan.selection.is_export() {
            let outputs = plan.outputs.clone();
            tokio::task::spawn_blocking(move || crate::vm_workflow_export::preflight(&outputs))
                .await
                .map_err(|_| Error::TransferFile)??;
        }
        tokio::select! { _=c.cancelled()=>return Err(Error::Closed), value=async {
            crate::power_batch::pool(s, &self.record).await?;
            for item in &plan.selection.items {
                if let Some(action) = item.workflow.transfer(item.vm) {
                    match &action {
                        crate::TransferAction::ExportOvf { source, .. } => { crate::ovf::export::capture(s,self.record.pool,source).await?; },
                        _ => { crate::transfer::xapi::prepare(s,self.record.pool,&action,None).await?; },
                    }
                    continue;
                }
                match item.workflow.action() {
                    OperationAction::Lifecycle(op) => {crate::vm_lifecycle_backend::plan(s,item.vm,&op).await?;},
                    OperationAction::Migration(op) => {crate::migration_backend::preflight(s,item.vm,&op).await?;},
                    _=>return Err(Error::InvalidResourceCommand),
                }
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
            let directory = self.directory.clone().ok_or(Error::Journal)?;
            let updates = self.updates.clone().ok_or(Error::Journal)?;
            let permit = j.workflow_permit(self.record.id, index)?;
            if let Some(action) = item.workflow.transfer(item.vm) {
                let observed = crate::ObservedResource {
                    pool: self.record.pool,
                    target: crate::ResourceTarget::Pool(self.record.pool),
                    generation: self.generation,
                };
                let (_, mut child) = crate::resources::transfer_channel(
                    plan.children[index],
                    observed,
                    crate::TransferRequest {
                        action,
                        file: plan.outputs.get(index).ok_or(Error::Journal)?.clone(),
                    },
                    tokio::runtime::Handle::current(),
                );
                child.record.batch_parent = Some(self.record.id);
                child.batch_permit = Some(permit);
                Box::pin(child.run_session(s, directory, c.clone(), updates)).await;
            } else {
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
                child.batch_permit = Some(permit);
                child
                    .run_session(s, Some(reference), directory, c.clone(), updates)
                    .await;
            }
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
        let plan = self
            .record
            .workflow_batch_plan
            .clone()
            .ok_or(Error::Journal)?;
        self.record.detail = format!(
            "Reading admitted child evidence; no remaining {} will start",
            plan.selection.noun()
        );
        self.save(j).await?;
        for index in 0..plan.admitted {
            let record = j
                .workflow_load_child(&self.record, index)?
                .ok_or(Error::Journal)?;
            if record.state().terminal() {
                continue;
            }
            let permit = j.workflow_permit(self.record.id, index)?;
            let directory = self.directory.clone().ok_or(Error::Journal)?;
            let updates = self.updates.clone().ok_or(Error::Journal)?;
            match record {
                WorkflowChild::Vm(record) => {
                    let mut child = crate::operations::MutationRequest::tracking(*record);
                    child.batch_permit = Some(permit);
                    child
                        .run_session(s, None, directory, c.clone(), updates)
                        .await;
                }
                WorkflowChild::Resource(record) => {
                    let mut child = Self::tracking(*record);
                    child.batch_permit = Some(permit);
                    Box::pin(child.run_session(s, directory, c.clone(), updates)).await;
                }
            }
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
            let admitted = self
                .record
                .workflow_batch_plan
                .as_ref()
                .ok_or(Error::Journal)?
                .admitted;
            for index in 0..admitted {
                let child = j
                    .workflow_load_child(&self.record, index)?
                    .ok_or(Error::Journal)?;
                self.record.workflow_batch[index].state = Some(child.state().clone());
                self.record.workflow_batch[index].detail =
                    child.detail().chars().take(1024).collect();
                self.record.steps[index].state = match child.state() {
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
            "{succeeded} of {} {} workflows verified; {} not started. No rollback or automatic continuation.",
            p.children.len(),
            p.selection.noun(),
            p.children.len() - p.admitted
        );
        for step in self.record.steps.iter_mut().skip(p.admitted) {
            step.state = StepState::Pending;
            step.detail = "Not started; a new review is required".into();
        }
        self.save(j).await
    }
}
