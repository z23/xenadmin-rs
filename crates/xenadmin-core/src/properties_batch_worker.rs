use super::*;
use crate::properties_batch::{self, Plan};

impl ResourceRequest {
    pub(super) async fn execute_properties_batch(
        &mut self,
        journal: &mut Journal,
        session: &Session,
        cancel: &CancellationToken,
    ) -> Result<(), Error> {
        let super::super::ResourceAction::PropertiesBatch(selection) = self.record.action.clone()
        else {
            return Err(Error::InvalidResourceCommand);
        };
        selection.validate(self.record.target)?;
        let plan = Plan::new(selection.clone());
        self.record.steps = (0..plan.receipts.len())
            .map(|index| {
                let (item, step) = plan.step(index).expect("validated properties plan");
                OperationStep {
                    label: format!("VM {} · {}", item.vm, step.label),
                    state: StepState::Pending,
                    detail: "Not submitted".into(),
                }
            })
            .collect();
        self.record.properties_batch_plan = Some(Box::new(plan));
        self.record.can_cancel = true;
        self.record.detail =
            "Checking every selected VM and property before the first change".into();
        self.save(journal).await?;
        tokio::select! { _=cancel.cancelled()=>return Err(Error::Closed), result=async {
            crate::power_batch::pool(session,&self.record).await?;
            for item in &selection.vms { properties_batch::preflight(session,item).await?; }
            Ok::<_,Error>(())
        }=>result? }
        for index in 0..self.record.steps.len() {
            self.reload(journal)?;
            if self.stop_before_dispatch() {
                return self.save(journal).await;
            }
            let plan = self.properties_plan()?.clone();
            let (method, params) = tokio::select! { _=cancel.cancelled()=>return Err(Error::Closed), result=async {
                crate::power_batch::pool(session,&self.record).await?;
                properties_batch::prepare(session,&plan,index).await
            }=>result? };
            self.reload(journal)?;
            if self.stop_before_dispatch() {
                return self.save(journal).await;
            }
            if cancel.is_cancelled() {
                return Err(Error::Closed);
            }
            if self.properties_plan()?.receipts[index].dispatched
                || (index > 0 && !self.properties_plan()?.receipts[index - 1].verified)
            {
                return Err(Error::JournalChanged);
            }
            self.record
                .properties_batch_plan
                .as_mut()
                .ok_or(Error::Journal)?
                .receipts[index]
                .dispatched = true;
            self.record.state = OperationState::Dispatching;
            self.record.steps[index].state = StepState::Dispatching;
            self.record.steps[index].detail = "Dispatch recorded; setter reply pending".into();
            self.record.detail = format!("Applying {}", self.record.steps[index].label);
            self.save(journal).await?;
            tokio::select! { _=cancel.cancelled()=>return Err(Error::Closed), result=session.call(&method,params)=>{result?;} }
            if !self.merge_properties_receipt(journal, index, false).await? {
                return Ok(());
            }
            if !self
                .verify_properties_step(journal, session, cancel, index)
                .await?
            {
                return Ok(());
            }
        }
        self.finish_properties_batch(journal).await
    }
    fn properties_plan(&self) -> Result<&Plan, Error> {
        self.record
            .properties_batch_plan
            .as_deref()
            .ok_or(Error::Journal)
    }
    async fn merge_properties_receipt(
        &mut self,
        journal: &mut Journal,
        index: usize,
        verified: bool,
    ) -> Result<bool, Error> {
        let selection = self.properties_plan()?.selection.clone();
        for _ in 0..16 {
            self.reload(journal)?;
            let preserve = self.stopped_after_recovery()
                || self.record.state == OperationState::OutcomeUnknown;
            let plan = self
                .record
                .properties_batch_plan
                .as_mut()
                .ok_or(Error::Journal)?;
            if plan.selection != selection {
                return Err(Error::Journal);
            }
            let receipt = plan.receipts.get_mut(index).ok_or(Error::Journal)?;
            if !receipt.dispatched || (verified && !receipt.succeeded) {
                return Err(Error::Journal);
            }
            receipt.succeeded = true;
            receipt.verified |= verified;
            if verified {
                self.record.steps[index].state = StepState::Verified;
                self.record.steps[index].detail =
                    "Setter success and resulting value verified".into();
            }
            if !preserve {
                self.record.state = if verified && !self.reconcile {
                    OperationState::Preparing
                } else {
                    OperationState::Verifying
                };
                self.record.progress = Some(
                    plan.receipts.iter().filter(|r| r.verified).count() as f64
                        / plan.receipts.len() as f64,
                );
            }
            match self.save(journal).await {
                Ok(()) => return Ok(!self.stopped_after_recovery()),
                Err(Error::JournalChanged) => continue,
                Err(error) => return Err(error),
            }
        }
        Err(Error::JournalChanged)
    }
    async fn verify_properties_step(
        &mut self,
        journal: &mut Journal,
        session: &Session,
        cancel: &CancellationToken,
        index: usize,
    ) -> Result<bool, Error> {
        let plan = self.properties_plan()?.clone();
        tokio::select! { _=cancel.cancelled()=>return Err(Error::Closed), result=properties_batch::verify(session,&plan,index)=>result? }
        self.merge_properties_receipt(journal, index, true).await
    }
    pub(super) async fn reconcile_properties_batch(
        &mut self,
        journal: &mut Journal,
        session: &Session,
        cancel: &CancellationToken,
    ) -> Result<(), Error> {
        self.reload(journal)?;
        if self.record.state.terminal() {
            return Ok(());
        }
        properties_batch::validate_record(&self.record)?;
        self.record.state = OperationState::Verifying;
        self.record.can_cancel = false;
        self.record.detail = "Checking saved setter receipts and resulting values; remaining changes will not be submitted".into();
        self.save(journal).await?;
        tokio::select! { _=cancel.cancelled()=>return Err(Error::Closed), result=crate::power_batch::pool(session,&self.record)=>result? }
        for index in 0..self.record.steps.len() {
            self.reload(journal)?;
            if self.record.state.terminal() {
                return Ok(());
            }
            let receipt = &self.properties_plan()?.receipts[index];
            if !receipt.dispatched {
                break;
            }
            if receipt.verified {
                continue;
            }
            if !self
                .verify_properties_step(journal, session, cancel, index)
                .await?
            {
                return Ok(());
            }
        }
        self.finish_properties_batch(journal).await
    }
    async fn finish_properties_batch(&mut self, journal: &mut Journal) -> Result<(), Error> {
        self.reload(journal)?;
        if self.stopped_after_recovery() {
            return Ok(());
        }
        let plan = self.properties_plan()?;
        if plan.receipts.iter().any(|r| r.dispatched && !r.verified) {
            return Err(Error::Protocol(
                "Properties batch still has an uncertain setter",
            ));
        }
        let done = plan.receipts.iter().filter(|r| r.verified).count();
        let count = plan.receipts.len();
        self.record.state = if done == count {
            OperationState::Succeeded
        } else if done > 0 {
            OperationState::PartiallyCompleted
        } else {
            OperationState::Cancelled
        };
        self.record.progress = Some(done as f64 / count as f64);
        self.record.can_cancel = false;
        self.record.detail = format!(
            "{done} of {count} property setters verified; {} were not submitted",
            count - done
        );
        self.save(journal).await
    }
}
