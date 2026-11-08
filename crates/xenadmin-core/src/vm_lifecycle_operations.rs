use super::*;
use crate::{OperationStep, StepState, vm_lifecycle_backend as lifecycle};
use serde_json::Value;

impl MutationRequest {
    fn sync_lifecycle(&mut self, plan: &lifecycle::Plan) {
        while self.record.steps.len() < plan.steps.len() {
            let step = &plan.steps[self.record.steps.len()];
            self.record.steps.push(OperationStep {
                label: step.label().into(),
                state: StepState::Pending,
                detail: String::new(),
            });
        }
        self.record.created_vm = plan.created;
        for device in &plan.created_devices {
            if !self.record.created_devices.contains(device) {
                self.record.created_devices.push(device.clone());
            }
        }
        self.record.lifecycle_plan = Some(Box::new(plan.clone()));
    }
    pub(super) async fn execute_vm_lifecycle(
        &mut self,
        session: &Session,
        journal: &mut Journal,
        cancel: &CancellationToken,
    ) -> Result<(), Error> {
        let OperationAction::Lifecycle(operation) = &self.record.action else {
            return Err(Error::InvalidVmLifecycle);
        };
        let mut plan = lifecycle::plan(session, self.record.vm, operation).await?;
        self.record.state = OperationState::Preparing;
        self.record.can_cancel = true;
        self.sync_lifecycle(&plan);
        self.persist_steps(journal).await?;
        let mut index = 0;
        while index < plan.steps.len() {
            if cancel.is_cancelled() {
                return Err(Error::Closed);
            }
            let prepared = lifecycle::prepare(session, self.record.id, &mut plan, index).await?;
            self.sync_lifecycle(&plan);
            if !self.begin_step(journal, index).await? {
                return Ok(());
            }
            let result = tokio::select! {_=cancel.cancelled()=>Err(Error::Closed),result=prepared.send(session)=>result};
            let response = match result {
                Ok(response) => response,
                Err(error) => return self.lifecycle_error(journal, &plan, index, error).await,
            };
            if plan.steps[index].asynchronous() {
                plan.evidence[index].task = Some(crate::snapshots::reference(&response)?);
                self.sync_lifecycle(&plan);
                self.record.state = OperationState::Tracking;
                self.persist_steps(journal).await?;
                loop {
                    let task = plan.evidence[index].task.as_ref().ok_or(Error::Journal)?;
                    let result = tokio::select! {_=cancel.cancelled()=>Err(Error::Closed),result=session.call("task.get_record",vec![json!(task)])=>result};
                    let record = match result {
                        Ok(record) => record,
                        Err(error) => {
                            return self.lifecycle_error(journal, &plan, index, error).await;
                        }
                    };
                    match record["status"].as_str() {
                        Some("success") => {
                            if matches!(plan.steps[index], lifecycle::Step::Copy { .. }) {
                                plan.evidence[index].result =
                                    Some(crate::xapi::snapshot_task_result(&record)?);
                            }
                            plan.evidence[index].completed = true;
                            break;
                        }
                        Some("failure") | Some("cancelled") => {
                            let code = record["error_info"]
                                .as_array()
                                .and_then(|v| v.first())
                                .and_then(Value::as_str)
                                .unwrap_or("TASK_FAILED");
                            return self
                                .finish_lifecycle_error(
                                    journal,
                                    &plan,
                                    index,
                                    Error::Api(crate::xapi::safe_error_code(code).into()),
                                    true,
                                )
                                .await;
                        }
                        Some("pending") | Some("cancelling") => {
                            self.record.state = OperationState::Tracking;
                            self.record.detail="Waiting for the host task; stopping here only prevents later steps".into();
                            self.record.can_cancel = true;
                            self.persist_steps(journal).await?;
                            tokio::select! {_=cancel.cancelled()=>return self.lifecycle_error(journal,&plan,index,Error::Closed).await,_=tokio::time::sleep(Duration::from_millis(250))=>{}}
                        }
                        _ => {
                            return self
                                .lifecycle_error(
                                    journal,
                                    &plan,
                                    index,
                                    Error::Protocol("invalid VM lifecycle task status"),
                                )
                                .await;
                        }
                    }
                }
            } else {
                lifecycle::record_result(&mut plan, index, &response)?;
                plan.evidence[index].completed = true;
            }
            self.record.state = OperationState::Verifying;
            self.sync_lifecycle(&plan);
            self.persist_steps(journal).await?;
            let verified = lifecycle::verify(session, self.record.id, &mut plan, index, true).await;
            self.sync_lifecycle(&plan);
            match verified {
                Ok(true) => {
                    self.record.steps[index].state = StepState::Verified;
                    self.record.steps[index].detail =
                        "Result and resource identities verified".into();
                }
                Ok(false) => {
                    return self
                        .lifecycle_error(
                            journal,
                            &plan,
                            index,
                            Error::Protocol("VM lifecycle result could not be verified"),
                        )
                        .await;
                }
                Err(error) => return self.lifecycle_error(journal, &plan, index, error).await,
            }
            self.record.state = OperationState::Preparing;
            self.record.progress = Some((index + 1) as f64 / plan.steps.len() as f64);
            self.persist_steps(journal).await?;
            index += 1;
        }
        self.record.state = OperationState::Succeeded;
        self.record.detail = "Requested changes verified; no VM power actions submitted".into();
        self.record.can_cancel = false;
        self.persist_steps(journal).await
    }
    async fn lifecycle_error(
        &mut self,
        journal: &mut Journal,
        plan: &lifecycle::Plan,
        index: usize,
        error: Error,
    ) -> Result<(), Error> {
        let refused = matches!(error, Error::Api(_) | Error::Coordinator(_))
            && self.record.state == OperationState::Dispatching;
        self.finish_lifecycle_error(journal, plan, index, error, refused)
            .await
    }
    async fn finish_lifecycle_error(
        &mut self,
        journal: &mut Journal,
        plan: &lifecycle::Plan,
        index: usize,
        error: Error,
        refused: bool,
    ) -> Result<(), Error> {
        self.sync_lifecycle(plan);
        self.record.steps[index].state = if refused {
            StepState::Failed
        } else {
            StepState::OutcomeUnknown
        };
        self.record.steps[index].detail = error.to_string();
        self.record.state = if !refused {
            OperationState::OutcomeUnknown
        } else if self
            .record
            .steps
            .iter()
            .any(|s| s.state == StepState::Verified)
        {
            OperationState::PartiallyCompleted
        } else {
            OperationState::Failed
        };
        self.record.detail = format!(
            "{error}; later steps were not submitted. Known created resources remain listed for review."
        );
        self.record.can_cancel = false;
        self.persist_steps(journal).await
    }
    pub(super) async fn recheck_vm_lifecycle(
        &mut self,
        session: &Session,
        journal: &mut Journal,
    ) -> Result<(), Error> {
        let mut plan = *self
            .record
            .lifecycle_plan
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
            if !plan.evidence[index].completed
                && let Some(task) = plan.evidence[index].task.as_ref()
            {
                match session.call("task.get_record", vec![json!(task)]).await {
                    Ok(record) if record["status"] == "success" => {
                        if matches!(plan.steps[index], lifecycle::Step::Copy { .. }) {
                            plan.evidence[index].result =
                                Some(crate::xapi::snapshot_task_result(&record)?);
                        }
                        plan.evidence[index].completed = true;
                    }
                    Ok(record)
                        if record["status"] == "failure" || record["status"] == "cancelled" =>
                    {
                        self.record.steps[index].state = StepState::Failed;
                        self.record.steps[index].detail="Host task reported failure or cancellation during read-only reconciliation".into();
                        continue;
                    }
                    _ => {
                        unresolved = true;
                        continue;
                    }
                }
            }
            match lifecycle::verify(session, self.record.id, &mut plan, index, false).await {
                Ok(true) => {
                    self.record.steps[index].state = StepState::Verified;
                    self.record.steps[index].detail =
                        "Observed during read-only reconciliation".into();
                }
                _ => {
                    unresolved = true;
                    self.record.steps[index].state = StepState::OutcomeUnknown;
                    self.record.steps[index].detail =
                        "Current result could not be verified; no writes repeated".into();
                }
            }
        }
        self.sync_lifecycle(&plan);
        self.record.state = if unresolved {
            OperationState::OutcomeUnknown
        } else if plan.workflow_planned
            && self
                .record
                .steps
                .iter()
                .all(|s| s.state == StepState::Verified)
        {
            OperationState::Succeeded
        } else if self
            .record
            .steps
            .iter()
            .any(|s| s.state == StepState::Verified)
        {
            OperationState::PartiallyCompleted
        } else {
            OperationState::Failed
        };
        self.record.detail = if self.record.state == OperationState::Succeeded {
            "Results verified; no writes repeated"
        } else {
            "Known results retained; pending steps were not submitted and no writes were repeated"
        }
        .into();
        self.record.can_cancel = false;
        self.persist_steps(journal).await
    }
}
#[cfg(test)]
#[path = "vm_lifecycle_tests.rs"]
mod tests;
