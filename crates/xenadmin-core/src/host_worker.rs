use super::*;
use crate::resources::host;

impl ResourceRequest {
    pub(super) async fn execute_host(
        &mut self,
        journal: &mut Journal,
        session: &Session,
        cancel: &CancellationToken,
    ) -> Result<(), Error> {
        self.record.detail = "Checking fresh host prerequisites and evacuation destinations".into();
        self.save(journal).await?;
        let plan = tokio::select! {_=cancel.cancelled()=>return Err(Error::Closed), result=host::prepare(session,&self.record)=>result?};
        self.record.steps = plan
            .steps
            .iter()
            .map(|step| OperationStep {
                label: step.kind.label().into(),
                state: StepState::Pending,
                detail: String::new(),
            })
            .collect();
        self.record.host_plan = Some(plan);
        self.record.can_cancel = true;
        self.save(journal).await?;
        for index in 0..self.record.steps.len() {
            self.reload(journal)?;
            if self.stop_before_dispatch() {
                return self.save(journal).await;
            }
            let kind = self.host_step(index)?.kind.clone();
            let (method, arguments) = tokio::select! {_=cancel.cancelled()=>return Err(Error::Closed), result=host::arguments(session,&self.record,&kind)=>result?};
            self.reload(journal)?;
            if self.stop_before_dispatch() {
                return self.save(journal).await;
            }
            if cancel.is_cancelled() {
                return Err(Error::Closed);
            }
            self.record.state = OperationState::Dispatching;
            self.record.steps[index].state = StepState::Dispatching;
            self.record.detail = format!("Submitting {}", kind.label());
            self.save(journal).await?;
            let result = tokio::select! {_=cancel.cancelled()=>return Err(Error::Closed), result=session.call(method,arguments)=>result?};
            let step = self
                .record
                .host_plan
                .as_mut()
                .ok_or(Error::Journal)?
                .steps
                .get_mut(index)
                .ok_or(Error::Journal)?;
            step.accepted = true;
            if kind.asynchronous() {
                step.task = Some(xapi::reply_reference(&result)?);
            }
            self.record = journal.host_evidence(&self.record, index)?;
            self.announce().await;
            if self.stopped_after_recovery() {
                return Ok(());
            }
            if kind.asynchronous()
                && !self
                    .track_host_step(journal, session, cancel, index)
                    .await?
            {
                return Ok(());
            }
            self.verify_host_step(journal, session, cancel, index, false)
                .await?;
            if self.stopped_after_recovery() {
                return Ok(());
            }
        }
        self.record.state = OperationState::Succeeded;
        self.record.progress = Some(1.0);
        self.record.can_cancel = false;
        self.record.detail = match &self.record.action {
            crate::ResourceAction::Host(crate::HostOperation::DataSource { action: crate::HostRecordingAction::ForgetHistory, .. }) => "The host confirmed deletion of this source's recorded history. Automatic collection may already have restarted; refresh to inspect current recording.",
            crate::ResourceAction::Host(crate::HostOperation::DataSource { .. }) => "The host confirmed recording and the source is currently enabled.",
            _ => "All host changes verified; no automatic compensation was performed",
        }.into();
        self.save(journal).await
    }
    fn host_step(&self, index: usize) -> Result<&host::Step, Error> {
        self.record
            .host_plan
            .as_ref()
            .and_then(|p| p.steps.get(index))
            .ok_or(Error::Protocol("host operation evidence unavailable"))
    }
    async fn verify_host_step(
        &mut self,
        journal: &mut Journal,
        session: &Session,
        cancel: &CancellationToken,
        index: usize,
        reconcile: bool,
    ) -> Result<(), Error> {
        let step = self.host_step(index)?.clone();
        tokio::select! {_=cancel.cancelled()=>return Err(Error::Closed),result=host::verify(session,&self.record,&step)=>result?};
        let mut saved = false;
        for _ in 0..16 {
            self.reload(journal)?;
            if self.stopped_after_recovery() {
                return Ok(());
            }
            self.record.steps[index].state = StepState::Verified;
            self.record.steps[index].detail = if matches!(
                step.kind,
                host::Kind::DataSource(crate::HostRecordingAction::ForgetHistory)
            ) {
                "Positive history-deletion receipt; current recording may already have restarted"
            } else if reconcile {
                "Confirmed by fresh reads; no write replayed"
            } else {
                "Confirmed by fresh host state"
            }
            .into();
            self.record.state = if reconcile {
                OperationState::Verifying
            } else {
                OperationState::Preparing
            };
            self.record.detail = format!("Verified {}", step.kind.label());
            self.record.progress = Some((index + 1) as f64 / self.record.steps.len() as f64);
            match self.save(journal).await {
                Ok(()) => {
                    saved = true;
                    break;
                }
                Err(Error::JournalChanged) => continue,
                Err(error) => return Err(error),
            }
        }
        if !saved {
            return Err(Error::JournalChanged);
        }
        if let Some(task) = step.task {
            let _ = tokio::time::timeout(
                Duration::from_secs(5),
                session.call("task.destroy", vec![json!(task)]),
            )
            .await;
        }
        Ok(())
    }
    async fn track_host_step(
        &mut self,
        journal: &mut Journal,
        session: &Session,
        cancel: &CancellationToken,
        index: usize,
    ) -> Result<bool, Error> {
        loop {
            self.reload(journal)?;
            if self.stopped_after_recovery() {
                return Ok(false);
            }
            let step = self.host_step(index)?.clone();
            if step.task_succeeded {
                return Ok(true);
            }
            let task_ref = step
                .task
                .ok_or(Error::Protocol("host task receipt unavailable"))?;
            let task = tokio::select! {_=cancel.cancelled()=>return Err(Error::Closed),result=session.call("task.get_record",vec![json!(&task_ref)])=>result?};
            match task["status"].as_str() {
                Some("success") => {
                    self.record.host_plan.as_mut().ok_or(Error::Journal)?.steps[index]
                        .task_succeeded = true;
                    self.record = journal.host_evidence(&self.record, index)?;
                    self.announce().await;
                    return Ok(!self.stopped_after_recovery());
                }
                Some(status @ ("failure" | "cancelled")) => {
                    self.record.state = if self
                        .record
                        .steps
                        .iter()
                        .any(|s| s.state == StepState::Verified)
                    {
                        OperationState::PartiallyCompleted
                    } else if status == "cancelled" {
                        OperationState::Cancelled
                    } else {
                        OperationState::Failed
                    };
                    self.record.can_cancel = false;
                    self.record.detail = if status == "cancelled" {
                        "Host confirmed task cancellation; completed changes were not undone".into()
                    } else {
                        let code = task["error_info"]
                            .as_array()
                            .and_then(|v| v.first())
                            .and_then(|v| v.as_str())
                            .unwrap_or("UNKNOWN_API_ERROR");
                        format!(
                            "Host task failed: {}. Completed changes were not undone",
                            crate::xapi::safe_error_code(code)
                        )
                    };
                    self.record.steps[index].state = StepState::Failed;
                    self.record.steps[index].detail = self.record.detail.clone();
                    match self.save(journal).await {
                        Err(Error::JournalChanged) => continue,
                        result => result?,
                    }
                    return Ok(false);
                }
                Some("pending" | "cancelling") => {
                    self.record.state = OperationState::Tracking;
                    self.record.can_cancel = true;
                    self.record.detail=if self.record.cancel_requested{"Waiting for the current host task; remaining steps will stop afterward"}else{"Waiting for the host task; stopping this workflow does not cancel in-flight host work"}.into();
                    match self.save(journal).await {
                        Ok(()) | Err(Error::JournalChanged) => {}
                        Err(error) => return Err(error),
                    }
                }
                _ => return Err(Error::Protocol("unknown host task status")),
            }
            tokio::select! {_=cancel.cancelled()=>return Err(Error::Closed),_=tokio::time::sleep(Duration::from_secs(1))=>{}}
        }
    }
    pub(super) async fn reconcile_host(
        &mut self,
        journal: &mut Journal,
        session: &Session,
        cancel: &CancellationToken,
    ) -> Result<(), Error> {
        let count = self
            .record
            .host_plan
            .as_ref()
            .map(|p| p.steps.len())
            .filter(|len| *len > 0 && *len == self.record.steps.len())
            .ok_or(Error::Protocol(
                "host dispatch evidence unavailable; no write will be replayed",
            ))?;
        self.reload(journal)?;
        if self.stopped_after_recovery() {
            return Ok(());
        }
        self.record.state = OperationState::Verifying;
        self.record.can_cancel = false;
        self.record.detail =
            "Rechecking existing host task receipts and state; no pending writes will be replayed"
                .into();
        self.save(journal).await?;
        for index in 0..count {
            self.reload(journal)?;
            if self.stopped_after_recovery() {
                return Ok(());
            }
            if matches!(
                self.record.steps[index].state,
                StepState::Pending | StepState::Verified
            ) {
                continue;
            }
            let step = self.host_step(index)?.clone();
            if step.task.is_some() && !step.task_succeeded {
                match self.track_host_step(journal, session, cancel, index).await {
                    Ok(true) => {}
                    Ok(false) => return Ok(()),
                    // Toolstack restarts can lose tasks. Only fresh postconditions may
                    // establish the outcome; shutdown still requires a success receipt.
                    Err(Error::Api(code))
                        if matches!(code.as_str(), "HANDLE_INVALID" | "UUID_INVALID") => {}
                    Err(error) => return Err(error),
                }
            }
            self.verify_host_step(journal, session, cancel, index, true)
                .await?;
        }
        self.record.state = if self
            .record
            .steps
            .iter()
            .all(|s| s.state == StepState::Verified)
        {
            OperationState::Succeeded
        } else {
            OperationState::PartiallyCompleted
        };
        self.record.can_cancel = false;
        self.record.detail =
            "Dispatched host changes reconciled; unsubmitted steps were not replayed or undone"
                .into();
        self.save(journal).await
    }
}
