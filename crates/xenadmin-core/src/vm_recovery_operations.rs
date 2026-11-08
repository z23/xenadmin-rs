use super::*;
use crate::{OperationStep, StepState, VmRecoveryOperation, vm_recovery as recovery};
impl Journal {
    fn recovery_evidence(
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
            let old = current.recovery_plan.as_mut().ok_or(Error::Journal)?;
            let new = received.recovery_plan.as_ref().ok_or(Error::Journal)?;
            if old.operation != new.operation
                || old.steps != new.steps
                || old.baseline != new.baseline
                || old.evidence.len() != new.evidence.len()
            {
                return Err(Error::JournalChanged);
            }
            for (o, n) in old.evidence.iter_mut().zip(&new.evidence) {
                merge(&mut o.task, &n.task)?;
                merge(&mut o.task_id, &n.task_id)?;
                o.completed |= n.completed;
            }
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
                Err(e) => return Err(e),
            }
        }
        Err(Error::JournalChanged)
    }
    fn recovery_restore_source(&self, record: &OperationRecord) -> Result<(), Error> {
        let OperationAction::Recovery(VmRecoveryOperation::RestoreBoot {
            source,
            configuration,
        }) = &record.action
        else {
            return Ok(());
        };
        let previous = self.load(*source)?.ok_or(Error::Stale)?;
        if previous.pool != record.pool
            || previous.vm != record.vm
            || previous.recovery_boot_restoration()
                != Some(VmRecoveryOperation::RestoreBoot {
                    source: *source,
                    configuration: configuration.clone(),
                })
        {
            return Err(Error::UnavailableAction);
        }
        if self.history()?.iter().any(|r|r.id!=record.id && r.pool==record.pool && r.vm==record.vm && matches!(&r.action,OperationAction::Recovery(VmRecoveryOperation::RestoreBoot{source:old,..}) if old==source) && r.steps.iter().any(|s|!matches!(s.state,StepState::Pending))){return Err(Error::UnavailableAction)}
        Ok(())
    }
}
impl MutationRequest {
    async fn save_recovery_evidence(
        &mut self,
        j: &mut Journal,
        rechecking: bool,
    ) -> Result<bool, Error> {
        let (current, stop) = j.recovery_evidence(&self.record, rechecking)?;
        self.record = current;
        self.emit().await;
        Ok(stop)
    }
    async fn begin_recovery_step(&mut self, j: &mut Journal, index: usize) -> Result<bool, Error> {
        let current = j.load(self.record.id)?.ok_or(Error::Journal)?;
        if current.state != OperationState::Preparing
            || current.steps[index].state != StepState::Pending
        {
            self.record = current;
            self.emit().await;
            return Ok(false);
        }
        self.record = current;
        if self.record.cancel_requested {
            return self.begin_step(j, index).await;
        }
        j.recovery_restore_source(&self.record)?;
        self.record.state = OperationState::Dispatching;
        self.record.steps[index].state = StepState::Dispatching;
        self.record.detail = self.record.steps[index].label.clone();
        match j.save(&mut self.record) {
            Ok(()) => {
                self.emit().await;
                Ok(true)
            }
            Err(Error::JournalChanged) => {
                self.record = j.load(self.record.id)?.ok_or(Error::Journal)?;
                self.emit().await;
                Ok(false)
            }
            Err(e) => Err(e),
        }
    }
    async fn recovery_error(
        &mut self,
        j: &mut Journal,
        plan: &recovery::Plan,
        index: usize,
        error: Error,
        refused: bool,
    ) -> Result<(), Error> {
        self.record.recovery_plan = Some(Box::new(plan.clone()));
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
            "{error}; later recovery steps were not submitted. Inspect the VM and saved boot order. Recheck is read-only; a retained override requires a separate reviewed restoration."
        );
        self.record.can_cancel = false;
        self.save_recovery_evidence(j, false).await.map(|_| ())
    }
    pub(super) async fn execute_recovery(
        &mut self,
        s: &Session,
        j: &mut Journal,
        cancel: &CancellationToken,
    ) -> Result<(), Error> {
        let OperationAction::Recovery(operation) = &self.record.action else {
            return Err(Error::UnavailableAction);
        };
        j.recovery_restore_source(&self.record)?;
        let mut plan = recovery::plan(s, self.record.vm, operation).await?;
        self.record.steps = plan
            .steps
            .iter()
            .map(|s| OperationStep {
                label: s.label().into(),
                state: StepState::Pending,
                detail: String::new(),
            })
            .collect();
        self.record.recovery_plan = Some(Box::new(plan.clone()));
        self.record.state = OperationState::Preparing;
        self.record.can_cancel = true;
        self.persist_steps(j).await?;
        for index in 0..plan.steps.len() {
            if cancel.is_cancelled() {
                return Err(Error::Closed);
            }
            let prepared = match recovery::prepare(s, self.record.vm, &plan, index).await {
                Ok(p) => p,
                Err(e) => return self.recovery_error(j, &plan, index, e, true).await,
            };
            if !self.begin_recovery_step(j, index).await? {
                return Ok(());
            }
            let response = match tokio::select! {_=cancel.cancelled()=>Err(Error::Closed),r=prepared.send(s)=>r}
            {
                Ok(r) => r,
                Err(e) => {
                    let refused = matches!(e, Error::Api(_) | Error::Coordinator(_));
                    return self.recovery_error(j, &plan, index, e, refused).await;
                }
            };
            if plan.steps[index].asynchronous() {
                plan.evidence[index].task = Some(crate::snapshots::reference(&response)?);
                self.record.recovery_plan = Some(Box::new(plan.clone()));
                self.record.state = OperationState::Tracking;
                if self.save_recovery_evidence(j, false).await? {
                    return Ok(());
                }
                loop {
                    let task = match tokio::select! {_=cancel.cancelled()=>Err(Error::Closed),r=recovery::task(s,&mut plan,index)=>r}
                    {
                        Ok(t) => t,
                        Err(e) => return self.recovery_error(j, &plan, index, e, false).await,
                    };
                    self.record.recovery_plan = Some(Box::new(plan.clone()));
                    if self.save_recovery_evidence(j, false).await? {
                        return Ok(());
                    }
                    match task["status"].as_str(){Some("success")=>break,Some("pending"|"cancelling")=>{tokio::select!{_=cancel.cancelled()=>return self.recovery_error(j,&plan,index,Error::Closed,false).await,_=tokio::time::sleep(Duration::from_millis(250))=>{}}},_=>return self.recovery_error(j,&plan,index,Error::Protocol("Recovery task did not complete successfully; inspect its outcome"),false).await}
                }
            } else {
                plan.evidence[index].completed = true;
            }
            self.record.recovery_plan = Some(Box::new(plan.clone()));
            self.record.state = OperationState::Verifying;
            if self.save_recovery_evidence(j, false).await? {
                return Ok(());
            }
            match recovery::verify(s, self.record.vm, &plan, index).await {
                Ok(true) => (),
                Ok(false) => {
                    return self
                        .recovery_error(
                            j,
                            &plan,
                            index,
                            Error::Protocol(
                                "Recovery result or boot settings changed before verification",
                            ),
                            false,
                        )
                        .await;
                }
                Err(e) => return self.recovery_error(j, &plan, index, e, false).await,
            }
            self.record.steps[index].state = StepState::Verified;
            self.record.steps[index].detail = "Exact VM state and saved completion verified".into();
            self.record.state = OperationState::Preparing;
            self.record.progress = Some((index + 1) as f64 / plan.steps.len() as f64);
            if self.save_recovery_evidence(j, false).await? {
                return Ok(());
            }
        }
        self.record.state = OperationState::Succeeded;
        self.record.can_cancel = false;
        self.record.detail=match plan.operation{VmRecoveryOperation::Boot{..}=>"VM startup and original boot order verified. Guest recovery environment execution has not been verified.",VmRecoveryOperation::Force{..}=>"Force task completed and VM state verified; no other server tasks were cancelled.",VmRecoveryOperation::RestoreBoot{..}=>"Original boot order restored; guest power and firmware were not changed."}.into();
        self.save_recovery_evidence(j, false).await.map(|_| ())
    }
    pub(super) async fn recheck_recovery(
        &mut self,
        s: &Session,
        j: &mut Journal,
    ) -> Result<(), Error> {
        let mut plan = *self
            .record
            .recovery_plan
            .clone()
            .ok_or(Error::UnavailableAction)?;
        let mut unknown = false;
        for index in 0..plan.steps.len() {
            if matches!(
                self.record.steps[index].state,
                StepState::Pending | StepState::Failed | StepState::Verified
            ) {
                continue;
            }
            if plan.steps[index].asynchronous()
                && !plan.evidence[index].completed
                && (plan.evidence[index].task.is_none()
                    || recovery::task(s, &mut plan, index).await.is_err()
                    || !plan.evidence[index].completed)
            {
                unknown = true;
                continue;
            }
            if recovery::verify(s, self.record.vm, &plan, index)
                .await
                .unwrap_or(false)
            {
                self.record.steps[index].state = StepState::Verified;
                self.record.steps[index].detail = "Verified by read-only recheck".into();
            } else {
                unknown = true;
                self.record.steps[index].state = StepState::OutcomeUnknown;
            }
        }
        self.record.recovery_plan = Some(Box::new(plan));
        self.record.can_cancel = false;
        self.record.state = if unknown {
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
        self.record.detail="Read-only recovery inspection complete. No force, startup, cancellation or boot-setting write was submitted. Any retained boot override requires a separate review.".into();
        self.save_recovery_evidence(j, true).await.map(|_| ())
    }
}
#[cfg(test)]
#[path = "vm_recovery_tests.rs"]
mod tests;
