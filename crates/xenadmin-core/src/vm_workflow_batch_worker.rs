use super::*;
use crate::{OperationAction, operations::workflow_children::WorkflowChild};
use xenadmin_xapi::calls;

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
        // A batch of cross-pool moves is submitted with the destination pool's connection
        // as it was reviewed. Without it there is nothing to check the selection against.
        let peer = if plan.selection.cross_pool_destination().is_some() {
            Some(self.cross_pool.take().ok_or(Error::InvalidMigration)?)
        } else {
            None
        };
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
        if let Some(inputs) = &peer {
            self.preflight_cross_pool_batch(s, c, inputs, &plan).await?;
        }
        tokio::select! { _=c.cancelled()=>return Err(Error::Closed), value=async {
            crate::power_batch::pool(s, &self.record).await?;
            for item in &plan.selection.items {
                // A cross-pool item was checked above, on both pools.
                if item.workflow.cross_pool().is_some() { continue; }
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
            if let Some(operation) = item.workflow.cross_pool() {
                // The child is the single cross-pool move of this guest with the mappings
                // reviewed for it. It fetches the destination session itself, checks both
                // pools again and prepares its own receiver, which nothing stores.
                let observed = crate::ObservedResource {
                    pool: self.record.pool,
                    target: crate::ResourceTarget::Pool(self.record.pool),
                    generation: self.generation,
                };
                let (_, mut child) = crate::resources::channel(
                    plan.children[index],
                    observed,
                    crate::ResourceAction::CrossPoolVm(operation.clone()),
                    tokio::runtime::Handle::current(),
                );
                child.record.batch_parent = Some(self.record.id);
                child.batch_permit = Some(permit);
                child.cross_pool = Some(peer.clone().ok_or(Error::Journal)?);
                Box::pin(child.run_session(s, directory, c.clone(), updates)).await;
            } else if let Some(action) = item.workflow.transfer(item.vm) {
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
                let reference = s.send(calls::vm::GetByUuid(&item.vm.to_string())).await?;
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
        // A move between pools can only be read with both pools connected. A recheck that
        // was not given the destination changes nothing and says what is needed.
        let peer = self.cross_pool.take();
        if crate::cross_pool::destination(&self.record.action).is_some() && peer.is_none() {
            self.record.local_error = Some(
                "Reconnect both pools and recheck this batch from its transfer history; no transfer will be replayed"
                    .into(),
            );
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
                    if crate::cross_pool::destination(&child.record.action).is_some() {
                        child.cross_pool = peer.clone();
                    }
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
    /// The whole-selection check of a batch of cross-pool moves. It reads both pools with
    /// the single move's own preparation for every guest and asks the destination for room
    /// for all of them. It prepares no receiver and sends nothing.
    async fn preflight_cross_pool_batch(
        &self,
        s: &Session,
        c: &CancellationToken,
        inputs: &crate::cross_pool::Inputs,
        plan: &crate::vm_workflow_batch::Plan,
    ) -> Result<(), Error> {
        let operations = plan
            .selection
            .cross_pool_operations()
            .ok_or(Error::Journal)?;
        let peer = tokio::select! { _=c.cancelled()=>return Err(Error::Closed), peer=inputs.destination.get(inputs.observation)=>peer? };
        super::cross_pool_worker::peer_current(inputs, &peer)?;
        if let Some(required) = plan.selection.peer_permission_requirements() {
            let assessment = required.assess(peer.session.permissions());
            if assessment.decision().is_denied() {
                return Err(Error::RoleDenied(format!(
                    "{} on the destination pool (RBAC_PERMISSION_DENIED)",
                    assessment.denied_methods().join(", ")
                )));
            }
        }
        tokio::select! {
            _=c.cancelled()=>return Err(Error::Closed),
            _=peer.cancel.cancelled()=>return Err(Error::Closed),
            result=async {
                crate::power_batch::pool(s, &self.record).await?;
                crate::cross_pool_backend::preflight_selection(s, &peer.session, self.record.pool, &operations).await
            }=>result?
        }
        super::cross_pool_worker::peer_current(inputs, &peer)
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
