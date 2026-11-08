use super::*;
use crate::{OperationStep, StepState, device_lifecycle};

impl MutationRequest {
    pub(super) async fn begin_step(
        &mut self,
        journal: &mut Journal,
        index: usize,
    ) -> Result<bool, Error> {
        let current = journal.load(self.record.id)?.ok_or(Error::Journal)?;
        if current.state.terminal() || current.state == OperationState::OutcomeUnknown {
            self.record = current;
            self.emit().await;
            return Ok(false);
        }
        self.record.revision = current.revision;
        self.record.cancel_requested |= current.cancel_requested;
        if self.record.cancel_requested {
            self.record.state = if self
                .record
                .steps
                .iter()
                .any(|step| step.state == StepState::Verified)
            {
                OperationState::PartiallyCompleted
            } else {
                OperationState::Cancelled
            };
            self.record.can_cancel = false;
            self.record.detail =
                "Stopped before the next step; verified changes and created resources remain"
                    .into();
            self.persist_steps(journal).await?;
            return Ok(false);
        }
        self.record.state = OperationState::Dispatching;
        self.record.steps[index].state = StepState::Dispatching;
        self.record.detail = format!(
            "{} ({}/{})",
            self.record.steps[index].label,
            index + 1,
            self.record.steps.len()
        );
        match self.publish(journal).await {
            Ok(()) => Ok(true),
            Err(Error::JournalChanged) => {
                self.record = journal.load(self.record.id)?.ok_or(Error::Journal)?;
                self.emit().await;
                Ok(false)
            }
            Err(e) => Err(e),
        }
    }
    pub(super) async fn execute_device_lifecycle(
        &mut self,
        session: &Session,
        journal: &mut Journal,
        cancel: &CancellationToken,
    ) -> Result<(), Error> {
        let OperationAction::Device(operation) = &self.record.action else {
            return Err(Error::InvalidDeviceOperation);
        };
        let mut plan = tokio::select! {_=cancel.cancelled()=>return Err(Error::Closed),result=device_lifecycle::plan(session,self.record.vm,operation)=>result?};
        self.record.steps = plan
            .steps
            .iter()
            .map(|step| OperationStep {
                label: step.label().into(),
                state: StepState::Pending,
                detail: String::new(),
            })
            .collect();
        self.record.device_plan = Some(plan.clone());
        self.record.state = OperationState::Preparing;
        self.record.can_cancel = true;
        self.persist_steps(journal).await?;
        for index in 0..plan.steps.len() {
            if cancel.is_cancelled() {
                return Err(Error::Closed);
            }
            if let Some(class) = plan.steps[index].creation_class() {
                let baseline = device_lifecycle::baseline(session, class).await?;
                plan.evidence[index] = Some(device_lifecycle::Evidence {
                    baseline,
                    ..Default::default()
                });
            }
            let prepared = device_lifecycle::prepare_step(
                session,
                self.record.vm,
                self.record.id,
                &plan,
                index,
            )
            .await?;
            self.record.device_plan = Some(plan.clone());
            if !self.begin_step(journal, index).await? {
                return Ok(());
            }
            let result = tokio::select! {_=cancel.cancelled()=>Err(Error::Closed),result=prepared.send(session)=>result};
            let response = match result {
                Ok(response) => response,
                Err(error) => {
                    let refused = !matches!(plan.steps[index], device_lifecycle::Step::Property(_))
                        && matches!(error, Error::Api(_) | Error::Coordinator(_));
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
                    self.record.detail =
                        format!("{error}; remaining device changes were not submitted");
                    self.record.can_cancel = false;
                    return self.persist_steps(journal).await;
                }
            };
            if let Some(evidence) = &mut plan.evidence[index] {
                evidence.result_reference = Some(crate::snapshots::reference(&response)?);
            }
            self.record.device_plan = Some(plan.clone());
            self.record.state = OperationState::Verifying;
            self.persist_steps(journal).await?;
            if !device_lifecycle::verify_step(
                session,
                self.record.vm,
                self.record.id,
                &mut plan,
                index,
            )
            .await?
            {
                self.record.state = OperationState::OutcomeUnknown;
                self.record.steps[index].state = StepState::OutcomeUnknown;
                self.record.detail="The host accepted this step, but its result could not be verified; later steps were not submitted".into();
                self.record.can_cancel = false;
                return self.persist_steps(journal).await;
            }
            self.record.steps[index].state = StepState::Verified;
            self.record.steps[index].detail =
                "Result and resource identity verified on host".into();
            self.record.state = OperationState::Preparing;
            self.record.progress = Some((index + 1) as f64 / plan.steps.len() as f64);
            self.capture_created_devices(&plan);
            self.record.device_plan = Some(plan.clone());
            self.persist_steps(journal).await?;
        }
        self.record.state = OperationState::Succeeded;
        self.record.detail =
            "Device configuration saved and verified; no VM power actions submitted".into();
        self.record.can_cancel = false;
        self.persist_steps(journal).await
    }
    fn capture_created_devices(&mut self, plan: &device_lifecycle::Plan) {
        for created in plan
            .evidence
            .iter()
            .flatten()
            .filter_map(|e| e.created.as_ref())
        {
            if !self.record.created_devices.contains(created) {
                self.record.created_devices.push(created.clone());
            }
        }
    }
    pub(super) async fn recheck_device_lifecycle(
        &mut self,
        session: &Session,
        journal: &mut Journal,
    ) -> Result<(), Error> {
        let mut plan = self
            .record
            .device_plan
            .clone()
            .ok_or(Error::UnavailableAction)?;
        if plan.steps.len() != self.record.steps.len() || plan.evidence.len() != plan.steps.len() {
            return Err(Error::Journal);
        }
        let mut unresolved = false;
        for index in 0..plan.steps.len() {
            if matches!(
                self.record.steps[index].state,
                StepState::Pending | StepState::Failed
            ) {
                continue;
            }
            if device_lifecycle::verify_step(
                session,
                self.record.vm,
                self.record.id,
                &mut plan,
                index,
            )
            .await?
            {
                self.record.steps[index].state = StepState::Verified;
                self.record.steps[index].detail =
                    "Result observed during read-only reconciliation".into();
            } else {
                unresolved = true;
            }
        }
        self.capture_created_devices(&plan);
        self.record.device_plan = Some(plan);
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
        self.record.detail=match self.record.state {
            OperationState::Succeeded=>"Device changes verified; no writes were repeated",
            OperationState::PartiallyCompleted=>"Completed steps and created resources were identified; pending changes were not submitted",
            _=>"Some device outcomes remain uncertain; no writes were repeated",
        }.into();
        self.record.can_cancel = false;
        self.persist_steps(journal).await
    }
}

#[cfg(test)]
#[path = "device_lifecycle_tests.rs"]
mod tests;
