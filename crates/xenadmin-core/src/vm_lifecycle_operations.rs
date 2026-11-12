use super::*;
use crate::{OperationStep, StepState, vm_lifecycle_backend as lifecycle};
use serde_json::Value;
use xenadmin_xapi::enumerations::task_status_type;
use xenadmin_xapi::records::TaskRecord;

impl MutationRequest {
    async fn save_lifecycle_evidence(
        &mut self,
        journal: &mut Journal,
        rechecking: bool,
    ) -> Result<bool, Error> {
        let received = self.record.clone();
        for _ in 0..16 {
            let mut current = journal.load(received.id)?.ok_or(Error::Journal)?;
            if current.action != received.action
                || current.vm != received.vm
                || current.pool != received.pool
            {
                return Err(Error::OperationConflict);
            }
            let plan = received.lifecycle_plan.as_ref().ok_or(Error::Journal)?;
            if current
                .lifecycle_plan
                .as_ref()
                .is_some_and(|old| !plan.extends(old))
            {
                self.record = current;
                self.emit().await;
                return Ok(true);
            }
            let stop = current.state.terminal()
                || !rechecking
                    && (current.state == OperationState::OutcomeUnknown
                        || journal.workflow_child_stopped(current.id)?);
            if !stop {
                current.state = received.state.clone();
                current.steps = received.steps.clone();
                current.detail = received.detail.clone();
                current.progress = received.progress;
                current.can_cancel = received.can_cancel;
            } else {
                while current.steps.len() < received.steps.len() {
                    current
                        .steps
                        .push(received.steps[current.steps.len()].clone());
                }
            }
            current.lifecycle_plan = received.lifecycle_plan.clone();
            current.created_vm = received.created_vm;
            current.created_devices = received.created_devices.clone();
            match journal.save(&mut current) {
                Ok(()) => {
                    self.record = current;
                    self.emit().await;
                    return Ok(stop);
                }
                Err(Error::JournalChanged) => continue,
                Err(e) => return Err(e),
            }
        }
        Err(Error::JournalChanged)
    }

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
        if self.save_lifecycle_evidence(journal, false).await? {
            return Ok(());
        }
        let mut index = 0;
        while index < plan.steps.len() {
            if cancel.is_cancelled() {
                return Err(Error::Closed);
            }
            let prepared = lifecycle::prepare(session, self.record.id, &mut plan, index).await?;
            self.sync_lifecycle(&plan);
            if self.save_lifecycle_evidence(journal, false).await? {
                return Ok(());
            }
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
                if self.save_lifecycle_evidence(journal, false).await? {
                    return Ok(());
                }
                loop {
                    let task = plan.evidence[index].task.as_ref().ok_or(Error::Journal)?;
                    let result = tokio::select! {_=cancel.cancelled()=>Err(Error::Closed),result=session.call("task.get_record",vec![json!(task)])=>result};
                    let record = match result {
                        Ok(record) => record,
                        Err(error) => {
                            return self.lifecycle_error(journal, &plan, index, error).await;
                        }
                    };
                    match TaskRecord::new(&record).status().as_str() {
                        Some(task_status_type::SUCCESS) => {
                            if matches!(plan.steps[index], lifecycle::Step::Copy { .. }) {
                                plan.evidence[index].result =
                                    Some(crate::xapi::snapshot_task_result(&record)?);
                            }
                            plan.evidence[index].completed = true;
                            break;
                        }
                        Some(task_status_type::FAILURE) | Some(task_status_type::CANCELLED) => {
                            let code = TaskRecord::new(&record)
                                .error_info()
                                .value()
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
                        Some(task_status_type::PENDING) | Some(task_status_type::CANCELLING) => {
                            self.record.state = OperationState::Tracking;
                            self.record.detail="Waiting for the host task; stopping here only prevents later steps".into();
                            self.record.can_cancel = true;
                            if self.save_lifecycle_evidence(journal, false).await? {
                                return Ok(());
                            }
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
            if self.save_lifecycle_evidence(journal, false).await? {
                return Ok(());
            }
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
            if self.save_lifecycle_evidence(journal, false).await? {
                return Ok(());
            }
            index += 1;
        }
        self.record.state = OperationState::Succeeded;
        self.record.detail = "Requested changes verified; no VM power actions submitted".into();
        self.record.can_cancel = false;
        self.save_lifecycle_evidence(journal, false)
            .await
            .map(|_| ())
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
        self.save_lifecycle_evidence(journal, false)
            .await
            .map(|_| ())
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
        self.save_lifecycle_evidence(journal, true)
            .await
            .map(|_| ())
    }
}
#[cfg(test)]
#[path = "vm_lifecycle_tests.rs"]
mod tests;
