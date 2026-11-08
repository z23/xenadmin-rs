use super::{ResourceOperationRecord, xapi};
use crate::{
    Error, OperationId, OperationState, OperationStep, StepState,
    operations::{Journal, OperationUpdate},
    xapi::Session,
};
use serde_json::json;
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

pub(crate) struct ResourceRequest {
    pub generation: u64,
    pub record: ResourceOperationRecord,
    sender: watch::Sender<Arc<ResourceOperationRecord>>,
    updates: Option<mpsc::Sender<OperationUpdate>>,
    reconcile: bool,
}
impl ResourceRequest {
    pub(super) fn new(
        generation: u64,
        record: ResourceOperationRecord,
        sender: watch::Sender<Arc<ResourceOperationRecord>>,
    ) -> Self {
        Self {
            generation,
            record,
            sender,
            updates: None,
            reconcile: false,
        }
    }
    pub(crate) fn tracking(record: ResourceOperationRecord) -> Self {
        let (sender, _) = watch::channel(Arc::new(record.clone()));
        Self {
            generation: 0,
            record,
            sender,
            updates: None,
            reconcile: true,
        }
    }
    pub(crate) fn reject(mut self, error: Error) {
        self.record.state = OperationState::Rejected;
        self.record.detail = error.to_string();
        self.sender.send_replace(Arc::new(self.record));
    }
    async fn announce(&self) {
        self.sender.send_replace(Arc::new(self.record.clone()));
        if let Some(updates) = &self.updates {
            let _ = updates
                .send(OperationUpdate::Resource(Box::new(self.record.clone())))
                .await;
        }
    }
    async fn save(&mut self, journal: &mut Journal) -> Result<(), Error> {
        journal.resource_save(&mut self.record)?;
        self.announce().await;
        Ok(())
    }
    fn reload(&mut self, journal: &mut Journal) -> Result<(), Error> {
        self.record = journal
            .resource_load(self.record.id)?
            .ok_or(Error::Journal)?;
        Ok(())
    }
    pub(crate) async fn run(
        mut self,
        session: Arc<Session>,
        directory: PathBuf,
        cancel: CancellationToken,
        updates: mpsc::Sender<OperationUpdate>,
    ) -> OperationId {
        let id = self.record.id;
        self.updates = Some(updates);
        let mut journal = match Journal::open(&directory) {
            Ok(journal) => journal,
            Err(error) => {
                self.reject(error);
                return id;
            }
        };
        let result = if self.reconcile {
            self.reconcile(&mut journal, &session, &cancel).await
        } else {
            match journal.resource_admit(&mut self.record) {
                Ok(true) => self.execute(&mut journal, &session, &cancel).await,
                Ok(false) => self.observe(&mut journal, &cancel).await,
                Err(error) => {
                    self.reject(error);
                    return id;
                }
            }
        };
        if let Err(error) = result {
            self.finish_error(&mut journal, error).await;
        }
        self.announce().await;
        id
    }
    async fn observe(
        &mut self,
        journal: &mut Journal,
        cancel: &CancellationToken,
    ) -> Result<(), Error> {
        loop {
            self.reload(journal)?;
            self.announce().await;
            if self.record.state.terminal() || self.record.state == OperationState::OutcomeUnknown {
                return Ok(());
            }
            tokio::select! {_=cancel.cancelled()=>return Ok(()),_=tokio::time::sleep(Duration::from_millis(250))=>{}}
        }
    }
    fn stop_before_dispatch(&mut self) -> bool {
        if self.record.state.terminal() {
            return true;
        }
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
            self.record.detail = "Stopped before dispatching the next change".into();
            return true;
        }
        false
    }
    async fn execute(
        &mut self,
        journal: &mut Journal,
        session: &Session,
        cancel: &CancellationToken,
    ) -> Result<(), Error> {
        self.record.detail = "Checking fresh resource state".into();
        self.save(journal).await?;
        let steps = tokio::select! {_=cancel.cancelled()=>return Err(Error::Closed),result=xapi::prepare(session,&mut self.record)=>result?};
        self.record.steps = steps
            .iter()
            .map(|step| OperationStep {
                label: step.label.clone(),
                state: StepState::Pending,
                detail: String::new(),
            })
            .collect();
        self.save(journal).await?;
        for (index, step) in steps.iter().enumerate() {
            self.reload(journal)?;
            if self.stop_before_dispatch() {
                return self.save(journal).await;
            }
            let arguments = tokio::select! {_=cancel.cancelled()=>return Err(Error::Closed),result=xapi::arguments(session,&self.record,step)=>result?};
            // A final journal read closes cancellation/acknowledgement races before the dispatch CAS.
            self.reload(journal)?;
            if self.stop_before_dispatch() {
                return self.save(journal).await;
            }
            if cancel.is_cancelled() {
                return Err(Error::Closed);
            }
            self.record.state = OperationState::Dispatching;
            self.record.steps[index].state = StepState::Dispatching;
            self.record.detail = format!("Submitting {}", step.label);
            self.save(journal).await?;
            let value = tokio::select! {_=cancel.cancelled()=>return Err(Error::Closed),result=session.call(&step.method,arguments)=>result?};
            if matches!(step.check, xapi::Check::Scan | xapi::Check::Network) {
                self.record.task = Some(xapi::reply_reference(&value)?);
                self.record = journal.resource_evidence(&self.record)?;
                self.announce().await;
                return self.track(journal, session, cancel).await;
            }
            if matches!(step.check, xapi::Check::CreatedNetwork) {
                self.record.result_reference = Some(xapi::reply_reference(&value)?);
            }
            self.record = journal.resource_evidence(&self.record)?;
            if self.record.state.terminal() {
                return Ok(());
            }
            self.announce().await;
            let created = tokio::select! {_=cancel.cancelled()=>return Err(Error::Closed),result=xapi::verify(session,&self.record,&step.check)=>result?};
            self.reload(journal)?;
            if self.record.state.terminal() {
                return Ok(());
            }
            if let Some(target) = created {
                self.record.created = Some(target);
            }
            self.record.steps[index].state = StepState::Verified;
            self.record.steps[index].detail = "Confirmed by a fresh host read".into();
            self.record.state = OperationState::Preparing;
            self.record.detail = format!("Verified {}", step.label);
            self.save(journal).await?;
        }
        self.record.state = OperationState::Succeeded;
        self.record.progress = Some(1.0);
        self.record.can_cancel = false;
        self.record.detail = "All requested changes were verified".into();
        self.save(journal).await
    }
    async fn reconcile(
        &mut self,
        journal: &mut Journal,
        session: &Session,
        cancel: &CancellationToken,
    ) -> Result<(), Error> {
        self.reload(journal)?;
        if self.record.state.terminal() {
            return Ok(());
        }
        if self.record.task.is_some() {
            return self.track(journal, session, cancel).await;
        }
        if matches!(
            self.record.action,
            super::ResourceAction::ScanStorage | super::ResourceAction::Network(_)
        ) {
            return Err(Error::Protocol(
                "host task identity is unavailable; outcome cannot be verified",
            ));
        }
        let checks = xapi::reconciliation_steps(&self.record);
        if self.record.steps.is_empty() || checks.len() != self.record.steps.len() {
            return Err(Error::Protocol(
                "resource operation dispatch evidence unavailable",
            ));
        }
        for (index, check) in checks.iter().enumerate() {
            if self.record.steps[index].state == StepState::Pending {
                continue;
            }
            let created = tokio::select! {_=cancel.cancelled()=>return Err(Error::Closed),result=xapi::verify(session,&self.record,check)=>result?};
            self.reload(journal)?;
            if self.record.state.terminal() {
                return Ok(());
            }
            if let Some(target) = created {
                self.record.created = Some(target);
            }
            self.record.steps[index].state = StepState::Verified;
            self.record.steps[index].detail =
                "Reconciled by a fresh host read; no write repeated".into();
            self.save(journal).await?;
        }
        self.record.state = if self
            .record
            .steps
            .iter()
            .all(|step| step.state == StepState::Verified)
        {
            OperationState::Succeeded
        } else {
            OperationState::PartiallyCompleted
        };
        self.record.can_cancel = false;
        self.record.detail =
            "Dispatched changes reconciled; unsubmitted steps were not replayed".into();
        self.save(journal).await
    }
    async fn track(
        &mut self,
        journal: &mut Journal,
        session: &Session,
        cancel: &CancellationToken,
    ) -> Result<(), Error> {
        let reference = self
            .record
            .task
            .clone()
            .ok_or(Error::Protocol("resource task missing"))?;
        let mut backoff = 1;
        loop {
            self.reload(journal)?;
            if self.record.state.terminal() {
                return Ok(());
            }
            let task = tokio::select! {_=cancel.cancelled()=>return Err(Error::Closed),result=async { if self.record.result_reference.is_some() { Ok(json!({"status":"success","progress":1.0})) } else { session.call("task.get_record",vec![json!(&reference)]).await } }=>result};
            let task = match task {
                Ok(task) => {
                    backoff = 1;
                    task
                }
                Err(error) if error.retryable() && !matches!(error, Error::Coordinator(_)) => {
                    tokio::select! {_=cancel.cancelled()=>return Err(Error::Closed),_=tokio::time::sleep(Duration::from_secs(backoff))=>{}}
                    backoff = (backoff * 2).min(10);
                    continue;
                }
                Err(error) => return Err(error),
            };
            self.record.progress = task["progress"]
                .as_f64()
                .or_else(|| task["progress"].as_str()?.parse().ok())
                .filter(|v| v.is_finite())
                .map(|v| v.clamp(0.0, 1.0));
            self.record.can_cancel = task["allowed_operations"]
                .as_array()
                .is_some_and(|ops| ops.iter().any(|op| op == "cancel"));
            match task["status"].as_str() {
                Some("success") => {
                    if let super::ResourceAction::Network(op) = &self.record.action {
                        if op.creates() && self.record.result_reference.is_none() {
                            self.record.result_reference =
                                Some(crate::xapi::snapshot_task_result(&task)?);
                            self.record = journal.resource_evidence(&self.record)?;
                            if self.record.state.terminal() {
                                return Ok(());
                            }
                        }
                        let created = tokio::select! { _=cancel.cancelled()=>return Err(Error::Closed), result=super::network::verify(session,&self.record)=>result? };
                        self.reload(journal)?;
                        if self.record.state.terminal() {
                            return Ok(());
                        }
                        self.record.created = created;
                    }
                    self.record.state = OperationState::Succeeded;
                    self.record.progress = Some(1.0);
                    self.record.detail =
                        "Host confirmed task completion and requested state".into();
                    for step in &mut self.record.steps {
                        step.state = StepState::Verified;
                        step.detail = "Host task completed".into();
                    }
                }
                Some("failure") => {
                    self.record.state = OperationState::Failed;
                    let code = task["error_info"]
                        .as_array()
                        .and_then(|values| values.first())
                        .and_then(|value| value.as_str())
                        .unwrap_or("UNKNOWN_API_ERROR");
                    self.record.detail =
                        format!("Host task failed: {}", crate::xapi::safe_error_code(code));
                    for step in &mut self.record.steps {
                        step.state = StepState::Failed;
                        step.detail = self.record.detail.clone();
                    }
                }
                Some("cancelled") => {
                    self.record.state = OperationState::Cancelled;
                    self.record.detail = "Host confirmed task cancellation".into();
                }
                Some("pending" | "cancelling") => {
                    self.record.state = OperationState::Tracking;
                    self.record.detail = "Waiting for the host task".into();
                }
                _ => return Err(Error::Protocol("unknown resource task status")),
            }
            if self.record.state.terminal() {
                self.record.can_cancel = false;
                match self.save(journal).await {
                    Err(Error::JournalChanged) => continue,
                    result => result?,
                };
                let _ = tokio::time::timeout(
                    Duration::from_secs(5),
                    session.call("task.destroy", vec![json!(&reference)]),
                )
                .await;
                return Ok(());
            }
            if self.record.cancel_requested
                && self.record.can_cancel
                && !self.record.cancel_dispatched
            {
                self.record.cancel_dispatched = true;
                self.record.state = OperationState::CancellationRequested;
                match self.save(journal).await {
                    Err(Error::JournalChanged) => continue,
                    result => result?,
                };
                tokio::select! {_=cancel.cancelled()=>return Err(Error::Closed),result=session.call("task.cancel",vec![json!(&reference)])=>{result?;}}
            } else {
                match self.save(journal).await {
                    Ok(()) | Err(Error::JournalChanged) => {}
                    Err(error) => return Err(error),
                }
            }
            tokio::select! {_=cancel.cancelled()=>return Err(Error::Closed),_=tokio::time::sleep(Duration::from_secs(1))=>{}}
        }
    }
    async fn finish_error(&mut self, journal: &mut Journal, error: Error) {
        if let Error::Coordinator(endpoint) = &error
            && let Some(updates) = &self.updates
        {
            let _ = updates
                .send(OperationUpdate::Coordinator(endpoint.clone()))
                .await;
        }
        if matches!(error, Error::JournalChanged) && self.reload(journal).is_ok() {
            if self.record.state == OperationState::Preparing && self.stop_before_dispatch() {
                let _ = self.save(journal).await;
            }
            return;
        }
        let after_dispatch = matches!(
            self.record.state,
            OperationState::Dispatching
                | OperationState::Tracking
                | OperationState::Verifying
                | OperationState::CancellationRequested
                | OperationState::OutcomeUnknown
        );
        let explicit_rejection = self.record.state == OperationState::Dispatching
            && matches!(error, Error::Api(_) | Error::Coordinator(_));
        let partial = self
            .record
            .steps
            .iter()
            .any(|step| step.state == StepState::Verified);
        if let Ok(Some(current)) = journal.resource_load(self.record.id) {
            if current.state.terminal() {
                self.record = current;
                return;
            }
            self.record = current;
        }
        self.record.state = if after_dispatch && !explicit_rejection {
            OperationState::OutcomeUnknown
        } else if partial {
            OperationState::PartiallyCompleted
        } else if matches!(error, Error::Closed) {
            OperationState::Cancelled
        } else {
            OperationState::Rejected
        };
        self.record.can_cancel = false;
        self.record.detail = error.to_string();
        for step in &mut self.record.steps {
            if step.state == StepState::Dispatching {
                step.state = if self.record.state == OperationState::OutcomeUnknown {
                    StepState::OutcomeUnknown
                } else {
                    StepState::Failed
                };
                step.detail = self.record.detail.clone();
            }
        }
        if let Err(save_error) = self.save(journal).await {
            if matches!(save_error, Error::JournalChanged) && self.reload(journal).is_ok() {
                return;
            }
            self.record.local_error = Some(save_error.to_string());
            self.record.state = OperationState::OutcomeUnknown;
        }
    }
}
