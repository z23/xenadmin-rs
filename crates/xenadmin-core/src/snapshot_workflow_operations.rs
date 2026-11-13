use super::*;
use crate::{OperationStep, StepState, snapshot_workflow as workflow};
use xenadmin_xapi::enumerations::task_status_type;
use xenadmin_xapi::records::TaskRecord;
use xenadmin_xapi::{calls, refs};

impl Journal {
    fn snapshot_workflow_evidence(
        &self,
        received: &OperationRecord,
        rechecking: bool,
    ) -> Result<(OperationRecord, bool), Error> {
        fn merge<T: Clone + PartialEq>(
            current: &mut Option<T>,
            received: &Option<T>,
        ) -> Result<(), Error> {
            if current.is_some() && received.is_some() && current != received {
                return Err(Error::JournalChanged);
            }
            if current.is_none() {
                *current = received.clone()
            }
            Ok(())
        }
        for _ in 0..16 {
            let mut current = self.load(received.id)?.ok_or(Error::Journal)?;
            if current.pool != received.pool
                || current.vm != received.vm
                || current.action != received.action
            {
                return Err(Error::OperationConflict);
            }
            let before = current.snapshot_workflow.as_mut().ok_or(Error::Journal)?;
            let new = received.snapshot_workflow.as_ref().ok_or(Error::Journal)?;
            if before.operation != new.operation
                || before.steps != new.steps
                || before.baseline != new.baseline
                || before.memory != new.memory
                || before.evidence.len() != new.evidence.len()
            {
                return Err(Error::JournalChanged);
            }
            merge(&mut before.created, &new.created)?;
            merge(&mut before.created_description, &new.created_description)?;
            for (old, new) in before.evidence.iter_mut().zip(&new.evidence) {
                merge(&mut old.task, &new.task)?;
                merge(&mut old.result, &new.result)?;
                old.completed |= new.completed;
            }
            merge(&mut current.created_snapshot, &received.created_snapshot)?;
            let stop = current.state.terminal()
                || (!rechecking && current.state == OperationState::OutcomeUnknown);
            if !stop {
                current.state = received.state.clone();
                current.steps = received.steps.clone();
                current.detail = received.detail.clone();
                current.progress = received.progress;
                current.can_cancel = received.can_cancel;
            }
            match self.save(&mut current) {
                Ok(()) => return Ok((current, stop)),
                Err(Error::JournalChanged) => continue,
                Err(error) => return Err(error),
            }
        }
        Err(Error::JournalChanged)
    }
}

impl MutationRequest {
    fn sync_snapshot_workflow(&mut self, plan: &workflow::Plan) {
        self.record.created_snapshot = plan.created;
        self.record.snapshot_workflow = Some(Box::new(plan.clone()));
    }
    async fn save_snapshot_evidence(
        &mut self,
        journal: &mut Journal,
        rechecking: bool,
    ) -> Result<bool, Error> {
        let (current, stop) = journal.snapshot_workflow_evidence(&self.record, rechecking)?;
        self.record = current;
        self.emit().await;
        Ok(stop)
    }
    async fn begin_snapshot_step(
        &mut self,
        journal: &mut Journal,
        index: usize,
    ) -> Result<bool, Error> {
        let current = journal.load(self.record.id)?.ok_or(Error::Journal)?;
        if current.state != OperationState::Preparing
            || current.steps[index].state != StepState::Pending
        {
            self.record = current;
            self.emit().await;
            return Ok(false);
        }
        self.record = current;
        if self.record.cancel_requested {
            return self.begin_step(journal, index).await;
        }
        self.record.state = OperationState::Dispatching;
        self.record.steps[index].state = StepState::Dispatching;
        self.record.detail = format!(
            "{} ({}/{})",
            self.record.steps[index].label,
            index + 1,
            self.record.steps.len()
        );
        match journal.save(&mut self.record) {
            Ok(()) => {
                self.emit().await;
                Ok(true)
            }
            Err(Error::JournalChanged) => {
                self.record = journal.load(self.record.id)?.ok_or(Error::Journal)?;
                self.emit().await;
                Ok(false)
            }
            Err(error) => Err(error),
        }
    }
    async fn inspect_late_snapshot_step(
        &mut self,
        session: &Session,
        journal: &mut Journal,
        index: usize,
    ) -> Result<(), Error> {
        let mut plan = *self
            .record
            .snapshot_workflow
            .clone()
            .ok_or(Error::Journal)?;
        if !plan.evidence[index].completed
            && let Some(task) = &plan.evidence[index].task
            && let Ok(record) = session.send(calls::task::GetRecord(refs::task(task))).await
            && TaskRecord::new(&record).status() == task_status_type::SUCCESS
        {
            if plan.steps[index] == workflow::Step::Create {
                plan.evidence[index].result = Some(crate::xapi::snapshot_task_result(&record)?);
            }
            plan.evidence[index].completed = true;
        }
        let _ = workflow::verify(session, self.record.vm, &mut plan, index).await;
        self.sync_snapshot_workflow(&plan);
        self.save_snapshot_evidence(journal, false).await?;
        Ok(())
    }
    pub(super) async fn execute_snapshot_workflow(
        &mut self,
        session: &Session,
        journal: &mut Journal,
        cancel: &CancellationToken,
    ) -> Result<(), Error> {
        let OperationAction::Snapshot(SnapshotOperation::Workflow { operation }) =
            &self.record.action
        else {
            return Err(Error::UnavailableAction);
        };
        let mut plan = workflow::plan(session, self.record.vm, operation).await?;
        self.record.steps = plan
            .steps
            .iter()
            .map(|step| OperationStep {
                label: step.label().into(),
                state: StepState::Pending,
                detail: String::new(),
            })
            .collect();
        self.record.state = OperationState::Preparing;
        self.record.can_cancel = true;
        self.sync_snapshot_workflow(&plan);
        self.persist_steps(journal).await?;
        for index in 0..plan.steps.len() {
            if cancel.is_cancelled() {
                return Err(Error::Closed);
            }
            let prepared = workflow::prepare(session, self.record.vm, &plan, index).await?;
            if !self.begin_snapshot_step(journal, index).await? {
                return Ok(());
            }
            let response = match tokio::select! {_=cancel.cancelled()=>Err(Error::Closed),result=prepared.send(session)=>result}
            {
                Ok(response) => response,
                Err(error) => {
                    let refused = matches!(error, Error::Api(_) | Error::Coordinator(_));
                    return self
                        .snapshot_workflow_error(journal, &plan, index, error, refused)
                        .await;
                }
            };
            if plan.steps[index].asynchronous() {
                plan.evidence[index].task = Some(crate::snapshots::reference(&response)?);
                self.record.state = OperationState::Tracking;
                self.sync_snapshot_workflow(&plan);
                if self.save_snapshot_evidence(journal, false).await? {
                    return self
                        .inspect_late_snapshot_step(session, journal, index)
                        .await;
                }
                loop {
                    let task = plan.evidence[index].task.as_ref().ok_or(Error::Journal)?;
                    let record = match tokio::select! {_=cancel.cancelled()=>Err(Error::Closed),result=session.send(calls::task::GetRecord(refs::task(task)))=>result}
                    {
                        Ok(record) => record,
                        Err(error) => {
                            return self
                                .snapshot_workflow_error(journal, &plan, index, error, false)
                                .await;
                        }
                    };
                    match TaskRecord::new(&record).status().name() {
                        Some(task_status_type::SUCCESS)=>{
                            if plan.steps[index]==workflow::Step::Create {plan.evidence[index].result=Some(crate::xapi::snapshot_task_result(&record)?)}
                            plan.evidence[index].completed=true;break;
                        }
                        Some(task_status_type::FAILURE)|Some(task_status_type::CANCELLED)=>return self.snapshot_workflow_error(journal,&plan,index,Error::Protocol("Host task ended without success; inspect the VM and snapshot before acknowledging this outcome"),false).await,
                        Some(task_status_type::PENDING)|Some(task_status_type::CANCELLING)=>{
                            self.record.detail="Waiting for host task; stopping the workflow prevents later steps only".into();
                            if self.save_snapshot_evidence(journal,false).await? { return Ok(()); }
                            tokio::select! {_=cancel.cancelled()=>return self.snapshot_workflow_error(journal,&plan,index,Error::Closed,false).await,_=tokio::time::sleep(Duration::from_millis(250))=>{}}
                        }
                        _=>return self.snapshot_workflow_error(journal,&plan,index,Error::Protocol("Invalid snapshot workflow task status"),false).await,
                    }
                }
            } else {
                plan.evidence[index].completed = true
            }
            self.record.state = OperationState::Verifying;
            self.sync_snapshot_workflow(&plan);
            if self.save_snapshot_evidence(journal, false).await? {
                return self
                    .inspect_late_snapshot_step(session, journal, index)
                    .await;
            }
            match workflow::verify(session, self.record.vm, &mut plan, index).await {
                Ok(true) => {}
                Ok(false) => {
                    return self
                        .snapshot_workflow_error(
                            journal,
                            &plan,
                            index,
                            Error::Protocol("Snapshot workflow result could not be verified"),
                            false,
                        )
                        .await;
                }
                Err(error) => {
                    return self
                        .snapshot_workflow_error(journal, &plan, index, error, false)
                        .await;
                }
            }
            self.sync_snapshot_workflow(&plan);
            self.record.steps[index].state = StepState::Verified;
            self.record.steps[index].detail = "Host result and exact identities verified".into();
            self.record.state = OperationState::Preparing;
            self.record.progress = Some((index + 1) as f64 / plan.steps.len() as f64);
            if self.save_snapshot_evidence(journal, false).await? {
                return Ok(());
            }
        }
        self.record.state = OperationState::Succeeded;
        self.record.detail = "Reviewed snapshot workflow completed and verified".into();
        self.record.can_cancel = false;
        self.save_snapshot_evidence(journal, false)
            .await
            .map(|_| ())
    }
    async fn snapshot_workflow_error(
        &mut self,
        journal: &mut Journal,
        plan: &workflow::Plan,
        index: usize,
        error: Error,
        refused: bool,
    ) -> Result<(), Error> {
        self.sync_snapshot_workflow(plan);
        self.record.steps[index].state = if refused {
            StepState::Failed
        } else {
            StepState::OutcomeUnknown
        };
        self.record.steps[index].detail = error.to_string();
        self.record.state = if !refused {
            OperationState::OutcomeUnknown
        } else if index == 0 {
            OperationState::Failed
        } else {
            OperationState::PartiallyCompleted
        };
        self.record.detail = format!(
            "{error}; later steps were not submitted. No automatic rollback or restart is attempted."
        );
        self.record.can_cancel = false;
        self.save_snapshot_evidence(journal, false)
            .await
            .map(|_| ())
    }
    pub(super) async fn recheck_snapshot_workflow(
        &mut self,
        session: &Session,
        journal: &mut Journal,
    ) -> Result<(), Error> {
        let mut plan = *self
            .record
            .snapshot_workflow
            .clone()
            .ok_or(Error::UnavailableAction)?;
        if plan.steps.len() != self.record.steps.len() || plan.evidence.len() != plan.steps.len() {
            return Err(Error::Journal);
        }
        let mut unresolved = false;
        for index in 0..plan.steps.len() {
            if matches!(
                self.record.steps[index].state,
                StepState::Pending | StepState::Failed | StepState::Verified
            ) {
                continue;
            }
            if plan.steps[index].asynchronous() && !plan.evidence[index].completed {
                let Some(task) = &plan.evidence[index].task else {
                    unresolved = true;
                    continue;
                };
                let record = session.send(calls::task::GetRecord(refs::task(task))).await;
                match record {
                    Ok(record)
                        if TaskRecord::new(&record).status() == task_status_type::SUCCESS =>
                    {
                        if plan.steps[index] == workflow::Step::Create {
                            plan.evidence[index].result =
                                Some(crate::xapi::snapshot_task_result(&record)?)
                        }
                        plan.evidence[index].completed = true;
                    }
                    _ => {
                        unresolved = true;
                        continue;
                    }
                }
            }
            match workflow::verify(session, self.record.vm, &mut plan, index).await {
                Ok(true) => {
                    self.record.steps[index].state = StepState::Verified;
                    self.record.steps[index].detail =
                        "Result verified by read-only reconciliation".into()
                }
                _ => {
                    unresolved = true;
                    self.record.steps[index].state = StepState::OutcomeUnknown
                }
            }
        }
        self.sync_snapshot_workflow(&plan);
        self.record.state = if unresolved {
            OperationState::OutcomeUnknown
        } else if self
            .record
            .steps
            .iter()
            .all(|s| s.state == StepState::Verified)
        {
            OperationState::Succeeded
        } else {
            OperationState::PartiallyCompleted
        };
        self.record.detail="Read-only reconciliation finished; no writes were repeated and pending shutdown, restore, metadata or startup steps were not submitted".into();
        self.record.can_cancel = false;
        self.save_snapshot_evidence(journal, true).await.map(|_| ())
    }
}
#[cfg(test)]
#[path = "snapshot_workflow_tests.rs"]
mod tests;
