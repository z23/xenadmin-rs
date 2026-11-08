#[path = "host_worker.rs"]
mod host;
#[path = "ovf_export_worker.rs"]
mod ovf_export_worker;
#[path = "ovf_worker.rs"]
mod ovf_worker;
#[path = "pool_join_worker.rs"]
mod pool_join_worker;
#[path = "power_batch_worker.rs"]
mod power_batch_worker;
#[path = "transfer_disk_worker.rs"]
mod transfer_disk_worker;
#[path = "transfer_worker.rs"]
mod transfer_worker;
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
    pub(crate) sensitive: Option<crate::pool_security::SensitiveInputs>,
    pub generation: u64,
    pub join: Option<crate::pool_join::Inputs>,
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
            sensitive: None,
            generation,
            join: None,
            record,
            sender,
            updates: None,
            reconcile: false,
        }
    }
    pub(crate) fn tracking(record: ResourceOperationRecord) -> Self {
        let (sender, _) = watch::channel(Arc::new(record.clone()));
        Self {
            sensitive: None,
            generation: 0,
            join: None,
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
    fn stopped_after_recovery(&self) -> bool {
        self.record.state.terminal()
            || (!self.reconcile && self.record.state == OperationState::OutcomeUnknown)
    }
    fn stop_before_dispatch(&mut self) -> bool {
        if self.record.state.terminal() || self.record.state == OperationState::OutcomeUnknown {
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
        if matches!(self.record.action, super::ResourceAction::PowerBatch(_)) {
            return self.execute_power_batch(journal, session, cancel).await;
        }
        if crate::pool_join::destination(&self.record.action).is_some() {
            return self.execute_join(journal, session, cancel).await;
        }
        if matches!(self.record.action, super::ResourceAction::Host(_)) {
            return self.execute_host(journal, session, cancel).await;
        }
        if matches!(self.record.action, super::ResourceAction::Transfer(_)) {
            return self.execute_transfer(journal, session, cancel).await;
        }
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
            let mut arguments = tokio::select! {_=cancel.cancelled()=>return Err(Error::Closed),result=xapi::arguments(session,&self.record,step)=>result?};
            // A final journal read closes cancellation/acknowledgement races before the dispatch CAS.
            self.reload(journal)?;
            if self.stop_before_dispatch() {
                return self.save(journal).await;
            }
            if cancel.is_cancelled() {
                return Err(Error::Closed);
            }
            if matches!(
                &self.record.action,
                super::ResourceAction::Pool(crate::PoolOperation::Security(_))
            ) {
                super::security::add_sensitive(
                    &self.record,
                    self.sensitive.as_ref(),
                    &mut arguments,
                )?;
            }
            self.record.state = OperationState::Dispatching;
            self.record.steps[index].state = StepState::Dispatching;
            self.record.detail = format!("Submitting {}", step.label);
            self.save(journal).await?;
            let value = tokio::select! {_=cancel.cancelled()=>return Err(Error::Closed),result=session.call(&step.method,arguments)=>result?};
            if matches!(&self.record.action, super::ResourceAction::SnapshotPolicy(op) if op.asynchronous())
                || matches!(&self.record.action, super::ResourceAction::Appliance(op) if op.asynchronous())
                || matches!(&self.record.action, super::ResourceAction::Pool(op) if op.asynchronous())
                || matches!(
                    step.check,
                    xapi::Check::Scan | xapi::Check::Network | xapi::Check::Storage
                )
            {
                self.record.task = Some(xapi::reply_reference(&value)?);
                self.record = journal.resource_evidence(&self.record)?;
                self.announce().await;
                if self.stopped_after_recovery() {
                    return Ok(());
                }
                return self.track(journal, session, cancel).await;
            }
            if matches!(step.check, xapi::Check::CreatedNetwork) {
                self.record.result_reference = Some(xapi::reply_reference(&value)?);
            }
            if matches!(
                &self.record.action,
                super::ResourceAction::Pool(crate::PoolOperation::Security(
                    crate::PoolSecurityOperation::AddSubject { .. }
                ))
            ) {
                self.record.result_reference = Some(xapi::reply_reference(&value)?);
            }
            if matches!(step.check, xapi::Check::Pool) {
                self.record.pool_completed = true;
            }
            self.record = journal.resource_evidence(&self.record)?;
            if self.stopped_after_recovery() {
                return Ok(());
            }
            self.announce().await;
            if matches!(step.check, xapi::Check::TaskCancellation) {
                self.record.steps[index].state = StepState::Verified;
                self.record.steps[index].detail =
                    "Host accepted the cancellation request; task outcome is not yet confirmed"
                        .into();
                self.save(journal).await?;
                return self.track_cancellation(journal, session, cancel).await;
            }
            let created = tokio::select! {_=cancel.cancelled()=>return Err(Error::Closed),result=xapi::verify(session,&self.record,&step.check)=>result?};
            self.reload(journal)?;
            if self.stopped_after_recovery() {
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
        if matches!(self.record.action, super::ResourceAction::PowerBatch(_)) {
            return self.reconcile_power_batch(journal, session, cancel).await;
        }
        if crate::pool_join::destination(&self.record.action).is_some() {
            return self.reconcile_join(journal, session, cancel).await;
        }
        self.reload(journal)?;
        if self.stopped_after_recovery() {
            return Ok(());
        }
        if matches!(self.record.action, super::ResourceAction::Host(_)) {
            return self.reconcile_host(journal, session, cancel).await;
        }
        if matches!(self.record.action, super::ResourceAction::Transfer(_)) {
            return self.recheck_transfer(journal, session, cancel).await;
        }
        if self.record.action == super::ResourceAction::CancelTask {
            if self.record.steps.len() != 1 || self.record.steps[0].state == StepState::Pending {
                return Err(Error::Protocol(
                    "task cancellation was not dispatched; no request will be replayed",
                ));
            }
            return self.track_cancellation(journal, session, cancel).await;
        }
        if self.record.task.is_some()
            && !self.record.storage_trim_succeeded
            && !self.record.pool_completed
            && !self.record.appliance_completed
            && !self
                .record
                .snapshot_policy_evidence
                .as_ref()
                .is_some_and(|e| e.completed)
        {
            return self.track(journal, session, cancel).await;
        }
        if matches!(&self.record.action,super::ResourceAction::SnapshotPolicy(op) if op.asynchronous())
            && !self
                .record
                .snapshot_policy_evidence
                .as_ref()
                .is_some_and(|e| e.completed)
        {
            return Err(Error::Protocol(
                "Snapshot schedule task identity or completion unavailable; no write will be replayed",
            ));
        }
        if matches!(&self.record.action, super::ResourceAction::Appliance(op) if op.asynchronous())
            && !self.record.appliance_completed
        {
            return Err(Error::Protocol(
                "vApp task identity or completion receipt unavailable; no write will be replayed",
            ));
        }
        if matches!(self.record.action, super::ResourceAction::Pool(_))
            && !self.record.pool_completed
        {
            return Err(Error::Protocol(
                "pool task identity or completion receipt is unavailable; no write will be replayed",
            ));
        }
        if matches!(self.record.action, super::ResourceAction::Storage(_))
            && self.record.result_reference.is_none()
            && !self.record.storage_trim_succeeded
        {
            return Err(Error::Protocol(
                "storage task identity is unavailable; matching resource state cannot establish asynchronous completion",
            ));
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
            if self.stopped_after_recovery() {
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
    async fn track_cancellation(
        &mut self,
        journal: &mut Journal,
        session: &Session,
        cancel: &CancellationToken,
    ) -> Result<(), Error> {
        loop {
            self.reload(journal)?;
            if self.stopped_after_recovery() {
                return Ok(());
            }
            let (_, task) = tokio::select! { _=cancel.cancelled()=>return Err(Error::Closed), result=xapi::read(session,self.record.target)=>result? };
            // Only status establishes completion. The original task remains owned
            // by its creator and is never destroyed by this cancellation observer.
            let (state, detail) = match task["status"].as_str() {
                Some("pending") if self.record.steps[0].state != StepState::Verified => (
                    OperationState::OutcomeUnknown,
                    "Cancellation request delivery is unknown; task is still pending. No request was replayed",
                ),
                Some("pending") => (
                    OperationState::CancellationRequested,
                    "Cancellation requested; host task is still pending",
                ),
                Some("cancelling") => (
                    OperationState::CancellationRequested,
                    "Host task is cancelling; completion is not confirmed",
                ),
                Some("cancelled") => (
                    OperationState::Succeeded,
                    "Host confirmed that the task was cancelled",
                ),
                Some("success") => (
                    OperationState::PartiallyCompleted,
                    "Task completed successfully instead of being cancelled",
                ),
                Some("failure") => (
                    OperationState::PartiallyCompleted,
                    "Task failed instead of reporting cancellation",
                ),
                _ => {
                    return Err(Error::Protocol(
                        "unknown task status while checking cancellation",
                    ));
                }
            };
            self.record.state = state.clone();
            self.record.detail = detail.into();
            self.record.can_cancel = false;
            self.record.progress = task["progress"]
                .as_f64()
                .or_else(|| task["progress"].as_str()?.parse().ok())
                .filter(|v| v.is_finite())
                .map(|v| v.clamp(0.0, 1.0));
            if state.terminal() {
                self.record.steps[0].state = StepState::Verified;
                self.record.steps[0].detail = detail.into();
            }
            self.save(journal).await?;
            if state.terminal() || state == OperationState::OutcomeUnknown {
                return Ok(());
            }
            tokio::select! { _=cancel.cancelled()=>return Err(Error::Closed), _=tokio::time::sleep(Duration::from_secs(1))=>{} }
        }
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
            if self.stopped_after_recovery() {
                return Ok(());
            }
            let task = tokio::select! {_=cancel.cancelled()=>return Err(Error::Closed),result=async { if self.record.result_reference.is_some() || self.record.pool_completed || self.record.appliance_completed || self.record.snapshot_policy_evidence.as_ref().is_some_and(|e|e.completed) { Ok(json!({"status":"success","progress":1.0})) } else { session.call("task.get_record",vec![json!(&reference)]).await } }=>result};
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
            if (matches!(&self.record.action, super::ResourceAction::Storage(operation) if operation.creates_reference())
                || matches!(&self.record.action,super::ResourceAction::Network(operation) if operation.creates())
                || matches!(&self.record.action, super::ResourceAction::Pool(_))
                || matches!(&self.record.action, super::ResourceAction::Appliance(_))
                || matches!(
                    &self.record.action,
                    super::ResourceAction::SnapshotPolicy(_)
                ))
                && matches!(task["status"].as_str(), Some("failure" | "cancelled"))
            {
                let code = task["error_info"]
                    .as_array()
                    .and_then(|values| values.first())
                    .and_then(|value| value.as_str())
                    .map(crate::xapi::safe_error_code);
                self.record.state = OperationState::OutcomeUnknown;
                self.record.can_cancel = false;
                let outcome = if task["status"] == "failure" {
                    "failed"
                } else {
                    "was cancelled"
                };
                self.record.detail = format!(
                    "Host task {outcome}{}. Partial changes may remain; inspect before acknowledging the unknown outcome",
                    code.map(|code| format!(": {code}")).unwrap_or_default()
                );
                for step in &mut self.record.steps {
                    step.state = StepState::OutcomeUnknown;
                    step.detail = self.record.detail.clone();
                }
                // Keep the task and pool reservation: failure/cancellation does not prove cleanup.
                return self.save(journal).await;
            }
            match task["status"].as_str() {
                Some("success") => {
                    if let super::ResourceAction::SnapshotPolicy(op) = &self.record.action {
                        if matches!(op, crate::SnapshotPolicyOperation::Create { .. })
                            && self.record.result_reference.is_none()
                        {
                            self.record.result_reference =
                                Some(crate::xapi::snapshot_task_result(&task)?);
                        }
                        let evidence = self
                            .record
                            .snapshot_policy_evidence
                            .as_mut()
                            .ok_or(Error::Journal)?;
                        if matches!(op, crate::SnapshotPolicyOperation::RunNow { .. })
                            && !evidence.completed
                        {
                            evidence.plugin_success = super::snapshot_policy::plugin_success(&task);
                        }
                        evidence.completed = true;
                        self.record = journal.resource_evidence(&self.record)?;
                        if self.stopped_after_recovery() {
                            return Ok(());
                        }
                        let created = tokio::select! {_=cancel.cancelled()=>return Err(Error::Closed),result=super::snapshot_policy::verify(session,&self.record,&super::snapshot_policy::Verify::Async)=>result?};
                        self.reload(journal)?;
                        if self.stopped_after_recovery() {
                            return Ok(());
                        }
                        self.record.created = created;
                    }
                    if let super::ResourceAction::Appliance(op) = &self.record.action {
                        if matches!(op, crate::ApplianceOperation::Create { .. })
                            && self.record.result_reference.is_none()
                        {
                            self.record.result_reference =
                                Some(crate::xapi::snapshot_task_result(&task)?);
                        }
                        self.record.appliance_completed = true;
                        self.record = journal.resource_evidence(&self.record)?;
                        if self.stopped_after_recovery() {
                            return Ok(());
                        }
                        let created = tokio::select! { _=cancel.cancelled()=>return Err(Error::Closed), result=super::appliance::verify(session,&self.record,&super::appliance::Verify::Async)=>result? };
                        self.reload(journal)?;
                        if self.stopped_after_recovery() {
                            return Ok(());
                        }
                        self.record.created = created;
                    }
                    if matches!(self.record.action, super::ResourceAction::Pool(_)) {
                        self.record.pool_completed = true;
                        self.record = journal.resource_evidence(&self.record)?;
                        if self.stopped_after_recovery() {
                            return Ok(());
                        }
                        tokio::select! { _=cancel.cancelled()=>return Err(Error::Closed), result=super::pool::verify(session,&self.record)=>{result?;} }
                        self.reload(journal)?;
                        if self.stopped_after_recovery() {
                            return Ok(());
                        }
                    }
                    if let super::ResourceAction::Network(op) = &self.record.action {
                        if op.creates() && self.record.result_reference.is_none() {
                            self.record.result_reference =
                                Some(crate::xapi::snapshot_task_result(&task)?);
                            self.record = journal.resource_evidence(&self.record)?;
                            if self.stopped_after_recovery() {
                                return Ok(());
                            }
                        }
                        let created = tokio::select! { _=cancel.cancelled()=>return Err(Error::Closed), result=super::network::verify(session,&self.record)=>result? };
                        self.reload(journal)?;
                        if self.stopped_after_recovery() {
                            return Ok(());
                        }
                        self.record.created = created;
                    }
                    if let super::ResourceAction::Storage(operation) = &self.record.action {
                        if matches!(operation, crate::StorageOperation::Trim { .. }) {
                            if !super::storage::trim_succeeded(&task)? {
                                self.record.state = OperationState::Failed;
                                self.record.detail = "The storage trim plugin reported failure; no backend error payload was retained".into();
                                self.record.can_cancel = false;
                                for step in &mut self.record.steps {
                                    step.state = StepState::Failed;
                                    step.detail = self.record.detail.clone();
                                }
                                self.save(journal).await?;
                                let _ = tokio::time::timeout(
                                    Duration::from_secs(5),
                                    session.call("task.destroy", vec![json!(&reference)]),
                                )
                                .await;
                                return Ok(());
                            }
                            self.record.storage_trim_succeeded = true;
                        }
                        if operation.creates_reference() && self.record.result_reference.is_none() {
                            self.record.result_reference =
                                Some(crate::xapi::snapshot_task_result(&task)?);
                            self.record = journal.resource_evidence(&self.record)?;
                            if self.stopped_after_recovery() {
                                return Ok(());
                            }
                        }
                        self.record.state = OperationState::Verifying;
                        self.save(journal).await?;
                        let created = tokio::select! { _=cancel.cancelled()=>return Err(Error::Closed), result=xapi::verify(session,&self.record,&xapi::Check::Storage)=>result? };
                        self.reload(journal)?;
                        if self.stopped_after_recovery() {
                            return Ok(());
                        }
                        if let Some(created) = created {
                            self.record.created = Some(created);
                        }
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
            if self.stopped_after_recovery() {
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
        if matches!(error, Error::JournalChanged) {
            self.finish_conflict(journal).await;
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
        let explicit_rejection = !matches!(
            self.record.action,
            super::ResourceAction::PowerBatch(_)
                | super::ResourceAction::Pool(_)
                | super::ResourceAction::Appliance(_)
                | super::ResourceAction::SnapshotPolicy(_)
        ) && self.record.state == OperationState::Dispatching
            && matches!(error, Error::Api(_) | Error::Coordinator(_));
        let partial = self
            .record
            .steps
            .iter()
            .any(|step| step.state == StepState::Verified);
        if let Ok(Some(current)) = journal.resource_load(self.record.id) {
            if current.state.terminal()
                || (!self.reconcile && current.state == OperationState::OutcomeUnknown)
            {
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
            if matches!(save_error, Error::JournalChanged) {
                self.finish_conflict(journal).await;
                return;
            }
            self.record.local_error = Some(save_error.to_string());
            self.record.state = OperationState::OutcomeUnknown;
        }
    }

    async fn finish_conflict(&mut self, journal: &mut Journal) {
        // This worker has stopped. A concurrent cancellation/evidence update must
        // not leave its row advertising an active tracker that no longer exists.
        for _ in 0..16 {
            if let Err(error) = self.reload(journal) {
                self.record.local_error = Some(error.to_string());
                self.record.state = OperationState::OutcomeUnknown;
                return;
            }
            if self.record.state.terminal() || self.record.state == OperationState::OutcomeUnknown {
                return;
            }
            if self.record.state != OperationState::Preparing || !self.stop_before_dispatch() {
                self.record.state = OperationState::OutcomeUnknown;
                self.record.can_cancel = false;
                self.record.detail = "Local tracking stopped after a concurrent update. Recheck the host before retrying; cancellation is not confirmed.".into();
                for step in &mut self.record.steps {
                    if step.state == StepState::Dispatching {
                        step.state = StepState::OutcomeUnknown;
                        step.detail = self.record.detail.clone();
                    }
                }
            }
            match self.save(journal).await {
                Ok(()) => return,
                Err(Error::JournalChanged) => continue,
                Err(error) => {
                    self.record.local_error = Some(error.to_string());
                    self.record.state = OperationState::OutcomeUnknown;
                    return;
                }
            }
        }
        self.record.local_error = Some(Error::JournalChanged.to_string());
        self.record.state = OperationState::OutcomeUnknown;
    }
}

#[cfg(test)]
mod stopped_worker_tests {
    use super::*;
    use crate::{ObservedResource, OperationControl, PoolId, ResourceAction, ResourceTarget};

    #[tokio::test]
    async fn cancellation_cas_does_not_leave_a_stopped_worker_tracking_or_release_its_guard() {
        for state in [
            OperationState::Dispatching,
            OperationState::Tracking,
            OperationState::Verifying,
            OperationState::CancellationRequested,
        ] {
            let directory = tempfile::tempdir().unwrap();
            let mut journal = Journal::open(directory.path()).unwrap();
            let pool: PoolId = uuid::Uuid::new_v4().into();
            let (_, mut request) = crate::resources::channel(
                OperationId::new(),
                ObservedResource {
                    pool,
                    generation: 1,
                    target: ResourceTarget::Pool(pool),
                },
                ResourceAction::CreateInternalNetwork {
                    name: "Fixture network".into(),
                    description: String::new(),
                    mtu: 1500,
                },
                tokio::runtime::Handle::current(),
            );
            assert!(journal.resource_admit(&mut request.record).unwrap());
            request.record.state = state;
            request.record.task = Some("OpaqueRef:owned-task".into());
            let created = uuid::Uuid::new_v4().into();
            request.record.transfer_created.disks.push(created);
            request.save(&mut journal).await.unwrap();
            journal
                .resource_control(
                    pool,
                    request.record.id,
                    OperationControl::RequestCancellation,
                )
                .unwrap();
            assert!(matches!(
                request.save(&mut journal).await,
                Err(Error::JournalChanged)
            ));
            request
                .finish_error(&mut journal, Error::JournalChanged)
                .await;
            let result = journal.resource_load(request.record.id).unwrap().unwrap();
            assert_eq!(result.state, OperationState::OutcomeUnknown);
            assert!(result.cancel_requested);
            assert_eq!(result.task.as_deref(), Some("OpaqueRef:owned-task"));
            assert_eq!(result.transfer_created.disks, vec![created]);
            assert!(journal.resource_pool_busy(pool).unwrap());
            // A late positive reply may add evidence, but the original worker
            // must not revive tracking or submit a subsequent configuration step.
            request.record.result_reference = Some("OpaqueRef:late-result".into());
            request.record = journal.resource_evidence(&request.record).unwrap();
            assert_eq!(request.record.state, OperationState::OutcomeUnknown);
            assert!(request.stopped_after_recovery());
            assert!(request.stop_before_dispatch());
            journal
                .resource_control(pool, result.id, OperationControl::AcknowledgeUnknown)
                .unwrap();
            request
                .finish_error(&mut journal, Error::JournalChanged)
                .await;
            assert_eq!(request.record.state, OperationState::AcknowledgedUnknown);
            assert!(!journal.resource_pool_busy(pool).unwrap());
        }
    }

    #[tokio::test]
    async fn cancellation_before_dispatch_remains_definite_after_a_concurrent_update() {
        let directory = tempfile::tempdir().unwrap();
        let mut journal = Journal::open(directory.path()).unwrap();
        let pool = uuid::Uuid::new_v4().into();
        let (_, mut request) = crate::resources::channel(
            OperationId::new(),
            ObservedResource {
                pool,
                generation: 1,
                target: ResourceTarget::Pool(pool),
            },
            ResourceAction::CreateInternalNetwork {
                name: "Fixture network".into(),
                description: String::new(),
                mtu: 1500,
            },
            tokio::runtime::Handle::current(),
        );
        journal.resource_admit(&mut request.record).unwrap();
        journal
            .resource_control(
                pool,
                request.record.id,
                OperationControl::RequestCancellation,
            )
            .unwrap();
        request
            .finish_error(&mut journal, Error::JournalChanged)
            .await;
        assert_eq!(request.record.state, OperationState::Cancelled);
        assert!(!journal.resource_pool_busy(pool).unwrap());
    }
}
