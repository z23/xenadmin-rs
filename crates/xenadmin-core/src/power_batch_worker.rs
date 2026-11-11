use super::*;
use crate::power_batch::{self, Item, Plan};
use xenadmin_xapi::records::TaskRecord;

impl ResourceRequest {
    pub(super) async fn execute_power_batch(
        &mut self,
        journal: &mut Journal,
        session: &Session,
        cancel: &CancellationToken,
    ) -> Result<(), Error> {
        let super::super::ResourceAction::PowerBatch(selection) = self.record.action.clone() else {
            return Err(Error::InvalidResourceCommand);
        };
        selection.validate(self.record.target)?;
        self.record.power_batch_plan = Some(Box::new(Plan::new(selection.clone())));
        self.record.steps = selection
            .vms
            .iter()
            .map(|item| OperationStep {
                label: format!("{} VM {}", selection.action.label(), item.vm),
                state: StepState::Pending,
                detail: "Not submitted".into(),
            })
            .collect();
        self.record.can_cancel = true;
        self.record.detail = "Checking every selected VM before the first power action".into();
        self.save(journal).await?;
        tokio::select! { _=cancel.cancelled()=>return Err(Error::Closed), result=async {
            power_batch::pool(session,&self.record).await?;
            for item in &selection.vms { power_batch::prepare_item(session,item,selection.action).await?; }
            Ok::<_,Error>(())
        }=>result? }
        for index in 0..selection.vms.len() {
            self.reload(journal)?;
            if self.stop_before_dispatch() {
                return self.save(journal).await;
            }
            let args = tokio::select! { _=cancel.cancelled()=>return Err(Error::Closed), result=async {
                power_batch::pool(session,&self.record).await?;
                power_batch::prepare_item(session,&selection.vms[index],selection.action).await
            }=>result? };
            self.reload(journal)?;
            if self.stop_before_dispatch() {
                return self.save(journal).await;
            }
            if cancel.is_cancelled() {
                return Err(Error::Closed);
            }
            if self.batch_item(index)?.dispatched
                || (index > 0 && !self.batch_item(index - 1)?.verified)
            {
                return Err(Error::JournalChanged);
            }
            self.record
                .power_batch_plan
                .as_mut()
                .ok_or(Error::Journal)?
                .items[index]
                .dispatched = true;
            self.record.state = OperationState::Dispatching;
            self.record.steps[index].state = StepState::Dispatching;
            self.record.steps[index].detail = "Dispatch recorded; task receipt pending".into();
            self.record.detail = format!("Submitting {}", self.record.steps[index].label);
            // A single CAS immediately precedes the only send for this item.
            self.save(journal).await?;
            let method = format!("Async.VM.{}", selection.action.wire());
            let result = tokio::select! { _=cancel.cancelled()=>return Err(Error::Closed), result=session.call(&method,args)=>result? };
            self.record
                .power_batch_plan
                .as_mut()
                .ok_or(Error::Journal)?
                .items[index]
                .task = Some(xapi::reply_reference(&result)?);
            if !self.merge_batch_evidence(journal, index).await? {
                return Ok(());
            }
            if !self
                .track_batch_item(journal, session, cancel, index)
                .await?
            {
                return Ok(());
            }
            if !self
                .verify_batch_item(journal, session, cancel, index)
                .await?
            {
                return Ok(());
            }
        }
        self.finish_batch(journal).await
    }
    pub(super) async fn cleanup_power_batch(&self, session: &Session, cancel: &CancellationToken) {
        if !self.record.state.terminal() || self.record.state == OperationState::AcknowledgedUnknown
        {
            return;
        }
        let Some(plan) = &self.record.power_batch_plan else {
            return;
        };
        // Completion evidence is already durable; cleanup is best effort and never
        // changes the outcome. Unknown tasks remain available for inspection.
        let cleanup = async {
            for task in plan
                .items
                .iter()
                .filter(|item| item.verified)
                .filter_map(|item| item.task.as_ref())
            {
                let _ = session.call("task.destroy", vec![json!(task)]).await;
            }
        };
        tokio::select! { _=cancel.cancelled()=>{}, _=tokio::time::timeout(Duration::from_secs(3),cleanup)=>{} }
    }
    fn batch_item(&self, index: usize) -> Result<&Item, Error> {
        self.record
            .power_batch_plan
            .as_ref()
            .and_then(|p| p.items.get(index))
            .ok_or(Error::Journal)
    }
    async fn merge_batch_evidence(
        &mut self,
        journal: &mut Journal,
        index: usize,
    ) -> Result<bool, Error> {
        let received = self.batch_item(index)?.clone();
        for _ in 0..16 {
            self.reload(journal)?;
            let item = self
                .record
                .power_batch_plan
                .as_mut()
                .and_then(|p| p.items.get_mut(index))
                .ok_or(Error::Journal)?;
            if received.selected != item.selected
                || (item.task.is_some() && received.task.is_some() && item.task != received.task)
                || !item.dispatched
            {
                return Err(Error::Journal);
            }
            if received.task.is_some() {
                item.task = received.task.clone();
            }
            item.succeeded |= received.succeeded;
            item.verified |= received.verified;
            let verified = item.verified;
            if verified {
                self.record.steps[index].state = StepState::Verified;
                self.record.steps[index].detail =
                    "Task success and resulting VM state verified".into();
            }
            let preserve =
                self.record.state.terminal() || self.record.state == OperationState::OutcomeUnknown;
            if !preserve {
                self.record.state = if verified {
                    if self.reconcile {
                        OperationState::Verifying
                    } else {
                        OperationState::Preparing
                    }
                } else {
                    OperationState::Tracking
                };
                self.record.detail = if verified {
                    format!("Verified {}", self.record.steps[index].label)
                } else {
                    format!("Tracking {}", self.record.steps[index].label)
                };
                let done = self
                    .record
                    .power_batch_plan
                    .as_ref()
                    .ok_or(Error::Journal)?
                    .items
                    .iter()
                    .filter(|i| i.verified)
                    .count();
                self.record.progress = Some(done as f64 / self.record.steps.len() as f64);
            }
            match self.save(journal).await {
                Ok(()) => return Ok(!self.stopped_after_recovery()),
                Err(Error::JournalChanged) => continue,
                Err(error) => return Err(error),
            }
        }
        Err(Error::JournalChanged)
    }
    async fn track_batch_item(
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
            let item = self.batch_item(index)?.clone();
            if item.succeeded {
                return Ok(true);
            }
            let reference = item.task.ok_or(Error::Protocol(
                "Power action has no saved task receipt; matching state cannot prove completion",
            ))?;
            let task = tokio::select! { _=cancel.cancelled()=>return Err(Error::Closed), result=session.call("task.get_record",vec![json!(&reference)])=>result? };
            match TaskRecord::new(&task).status().as_str() {
                Some("success") => {
                    self.record
                        .power_batch_plan
                        .as_mut()
                        .ok_or(Error::Journal)?
                        .items[index]
                        .succeeded = true;
                    return self.merge_batch_evidence(journal, index).await;
                }
                Some("failure" | "cancelled") => {
                    let code = TaskRecord::new(&task)
                        .error_info()
                        .value()
                        .as_array()
                        .and_then(|v| v.first())
                        .and_then(|v| v.as_str())
                        .map(crate::xapi::safe_error_code);
                    self.reload(journal)?;
                    if self.stopped_after_recovery() {
                        return Ok(false);
                    }
                    self.record.state = OperationState::OutcomeUnknown;
                    self.record.can_cancel = false;
                    self.record.detail = format!(
                        "{} task {}; remaining VMs were not submitted. Inspect before acknowledging{}",
                        self.record.steps[index].label,
                        TaskRecord::new(&task).status().as_str().unwrap_or("failed"),
                        code.map(|c| format!(": {c}")).unwrap_or_default()
                    );
                    self.record.steps[index].state = StepState::OutcomeUnknown;
                    self.record.steps[index].detail = self.record.detail.clone();
                    self.save(journal).await?;
                    return Ok(false);
                }
                Some("pending" | "cancelling") => {}
                _ => return Err(Error::Protocol("Unknown power task status")),
            }
            // User stop is deliberately not forwarded as task.cancel: finish the
            // current task and skip untouched VMs at the next dispatch boundary.
            tokio::select! { _=cancel.cancelled()=>return Err(Error::Closed), _=tokio::time::sleep(Duration::from_millis(250))=>{} }
        }
    }
    async fn verify_batch_item(
        &mut self,
        journal: &mut Journal,
        session: &Session,
        cancel: &CancellationToken,
        index: usize,
    ) -> Result<bool, Error> {
        let item = self.batch_item(index)?.clone();
        let action = self
            .record
            .power_batch_plan
            .as_ref()
            .ok_or(Error::Journal)?
            .selection
            .action;
        tokio::select! { _=cancel.cancelled()=>return Err(Error::Closed), result=power_batch::verify_item(session,&item,action)=>result? }
        self.record
            .power_batch_plan
            .as_mut()
            .ok_or(Error::Journal)?
            .items[index]
            .verified = true;
        self.merge_batch_evidence(journal, index).await
    }
    pub(super) async fn reconcile_power_batch(
        &mut self,
        journal: &mut Journal,
        session: &Session,
        cancel: &CancellationToken,
    ) -> Result<(), Error> {
        self.reload(journal)?;
        if self.record.state.terminal() {
            return Ok(());
        }
        power_batch::validate_record(&self.record)?;
        let count = self
            .record
            .power_batch_plan
            .as_ref()
            .ok_or(Error::Journal)?
            .items
            .len();
        self.record.state = OperationState::Verifying;
        self.record.can_cancel = false;
        self.record.detail =
            "Checking saved power tasks; remaining VMs will not be submitted".into();
        self.save(journal).await?;
        tokio::select! { _=cancel.cancelled()=>return Err(Error::Closed), result=power_batch::pool(session,&self.record)=>result? }
        for index in 0..count {
            self.reload(journal)?;
            if self.record.state.terminal() {
                return Ok(());
            }
            let item = self.batch_item(index)?;
            if !item.dispatched {
                break;
            }
            if item.verified {
                continue;
            }
            if !self
                .track_batch_item(journal, session, cancel, index)
                .await?
                || !self
                    .verify_batch_item(journal, session, cancel, index)
                    .await?
            {
                return Ok(());
            }
        }
        self.finish_batch(journal).await
    }
    async fn finish_batch(&mut self, journal: &mut Journal) -> Result<(), Error> {
        self.reload(journal)?;
        if self.stopped_after_recovery() {
            return Ok(());
        }
        let plan = self
            .record
            .power_batch_plan
            .as_ref()
            .ok_or(Error::Journal)?;
        let done = plan.items.iter().filter(|i| i.verified).count();
        if plan.items.iter().any(|i| i.dispatched && !i.verified) {
            return Err(Error::Protocol("Power batch still has an uncertain item"));
        }
        self.record.state = if done == plan.items.len() {
            OperationState::Succeeded
        } else if done > 0 {
            OperationState::PartiallyCompleted
        } else {
            OperationState::Cancelled
        };
        self.record.progress = Some(done as f64 / plan.items.len() as f64);
        self.record.can_cancel = false;
        self.record.detail = format!(
            "{done} of {} VM actions verified; {} were not submitted",
            plan.items.len(),
            plan.items.len() - done
        );
        self.save(journal).await
    }
}
