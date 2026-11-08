use super::*;
use crate::{pool_dissolve::Evidence, resources::dissolution as pool_dissolve};
impl ResourceRequest {
    fn pool_dissolve_step(&self, index: usize) -> Result<&Evidence, Error> {
        self.record
            .pool_dissolve_plan
            .as_ref()
            .and_then(|p| p.evidence.get(index))
            .ok_or(Error::Journal)
    }
    fn pool_dissolve_step_mut(&mut self, index: usize) -> Result<&mut Evidence, Error> {
        self.record
            .pool_dissolve_plan
            .as_mut()
            .and_then(|p| p.evidence.get_mut(index))
            .ok_or(Error::Journal)
    }
    pub(super) async fn execute_pool_dissolve(
        &mut self,
        j: &mut Journal,
        s: &Session,
        c: &CancellationToken,
    ) -> Result<(), Error> {
        self.record.detail = "Checking every reviewed member before pool dissolution".into();
        self.save(j).await?;
        let plan = tokio::select! {_=c.cancelled()=>return Err(Error::Closed),p=pool_dissolve::prepare(s,&self.record)=>p?};
        self.record.steps = crate::pool_dissolve::Plan::kinds(&plan.operation)
            .iter()
            .map(|k| OperationStep {
                label: k.label(),
                state: StepState::Pending,
                detail: String::new(),
            })
            .collect();
        self.record.pool_dissolve = Some(plan.result());
        self.record.pool_dissolve_plan = Some(Box::new(plan));
        self.record.can_cancel = true;
        self.save(j).await?;
        for index in 0..self.record.steps.len() {
            self.reload(j)?;
            if self.stopped_after_recovery() {
                return Ok(());
            }
            if self.stop_before_dispatch() {
                return self.save(j).await;
            }
            let (method, args) = tokio::select! {_=c.cancelled()=>return Err(Error::Closed),v=pool_dissolve::arguments(s,&self.record,index)=>v?};
            self.reload(j)?;
            if self.stopped_after_recovery() {
                return Ok(());
            }
            if self.stop_before_dispatch() {
                return self.save(j).await;
            }
            if c.is_cancelled() {
                return Err(Error::Closed);
            }
            if self.record.steps[index].state != StepState::Pending {
                return Err(Error::JournalChanged);
            }
            self.pool_dissolve_step_mut(index)?.sent = true;
            self.record.state = OperationState::Dispatching;
            self.record.steps[index].state = StepState::Dispatching;
            self.record.detail = format!("Submitting {}", self.record.steps[index].label);
            self.record.can_cancel = false;
            self.save_dispatch(j)?;
            let result = tokio::select! {_=c.cancelled()=>return Err(Error::Closed),v=s.call(method,args)=>v?};
            let asynchronous = crate::pool_dissolve::Plan::kinds(
                &self
                    .record
                    .pool_dissolve_plan
                    .as_ref()
                    .ok_or(Error::Journal)?
                    .operation,
            )[index]
                .asynchronous();
            if asynchronous {
                self.pool_dissolve_step_mut(index)?.task =
                    Some(crate::snapshots::reference(&result)?);
            } else {
                self.pool_dissolve_step_mut(index)?.completed = true;
                self.pool_dissolve_step_mut(index)?.ended = true;
            }
            self.record = j.pool_dissolve_evidence(&self.record, index)?;
            self.announce().await;
            if self.stopped_after_recovery() {
                return Ok(());
            }
            if !self.track_pool_dissolve(j, s, c, index).await? {
                return Ok(());
            }
        }
        self.finish_pool_dissolve(j, s, c).await
    }
    pub(super) async fn reconcile_pool_dissolve(
        &mut self,
        j: &mut Journal,
        s: &Session,
        c: &CancellationToken,
    ) -> Result<(), Error> {
        self.reload(j)?;
        if self.stopped_after_recovery() {
            return Ok(());
        }
        crate::pool_dissolve::validate_record(&self.record)?;
        if self.record.pool_dissolve_plan.is_none() {
            return Err(Error::Protocol(
                "Pool dissolution preparation evidence unavailable; no write was replayed",
            ));
        }
        self.record.state = OperationState::Verifying;
        self.record.detail =
            "Inspecting previously submitted pool dissolution changes; remaining steps will not run".into();
        self.record.can_cancel = false;
        self.save(j).await?;
        for index in 0..self.record.steps.len() {
            self.reload(j)?;
            if self.stopped_after_recovery() {
                return Ok(());
            }
            if matches!(
                self.record.steps[index].state,
                StepState::Pending | StepState::Verified
            ) {
                continue;
            }
            if !self.track_pool_dissolve(j, s, c, index).await? {
                return Ok(());
            }
        }
        self.finish_pool_dissolve(j, s, c).await
    }
    async fn track_pool_dissolve(
        &mut self,
        j: &mut Journal,
        s: &Session,
        c: &CancellationToken,
        index: usize,
    ) -> Result<bool, Error> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3600);
        loop {
            self.reload(j)?;
            if self.stopped_after_recovery() {
                return Ok(false);
            }
            if self.pool_dissolve_step(index)?.completed {
                tokio::select! {_=c.cancelled()=>return Err(Error::Closed),v=pool_dissolve::verify(s,&mut self.record,index)=>v?};
                self.record = j.pool_dissolve_evidence(&self.record, index)?;
                self.announce().await;
                if self.stopped_after_recovery() {
                    return Ok(false);
                }
                for _ in 0..16 {
                    self.reload(j)?;
                    if self.stopped_after_recovery() {
                        return Ok(false);
                    }
                    self.record.steps[index].state = StepState::Verified;
                    self.record.steps[index].detail =
                        "Completion receipt and exact remaining pool membership verified".into();
                    self.record.state = if self.reconcile {
                        OperationState::Verifying
                    } else {
                        OperationState::Preparing
                    };
                    self.record.can_cancel = !self.reconcile;
                    match self.save(j).await {
                        Ok(()) => return Ok(true),
                        Err(Error::JournalChanged) => continue,
                        Err(e) => return Err(e),
                    }
                }
                return Err(Error::JournalChanged);
            }
            if self.pool_dissolve_step(index)?.ended {
                return Err(Error::Protocol(
                    "The pool dissolution task failed or was cancelled; inspect any retained changes before acknowledging",
                ));
            }
            if !crate::pool_dissolve::Plan::kinds(
                &self
                    .record
                    .pool_dissolve_plan
                    .as_ref()
                    .ok_or(Error::Journal)?
                    .operation,
            )[index]
                .asynchronous()
            {
                return Err(Error::Protocol(
                    "Pool metadata reply was lost; matching field values cannot establish a completion receipt",
                ));
            }
            let (uuid, value) = tokio::select! {_=c.cancelled()=>return Err(Error::Closed),v=pool_dissolve::task(s,&self.record,index)=>v?};
            self.pool_dissolve_step_mut(index)?.task_uuid = Some(uuid);
            match value["status"].as_str() {
                Some("success") => {
                    self.pool_dissolve_step_mut(index)?.completed = true;
                    self.pool_dissolve_step_mut(index)?.ended = true;
                }
                Some("failure" | "cancelled") => self.pool_dissolve_step_mut(index)?.ended = true,
                Some("pending" | "cancelling") => {}
                _ => return Err(Error::Protocol("Pool dissolution task status unavailable")),
            }
            self.record = j.pool_dissolve_evidence(&self.record, index)?;
            self.announce().await;
            if self.stopped_after_recovery() {
                return Ok(false);
            }
            if self.pool_dissolve_step(index)?.ended {
                continue;
            }
            self.record.state = OperationState::Tracking;
            self.record.can_cancel = !self.reconcile;
            self.record.detail =
                "Member ejection is running. Stop skips later hosts and metadata changes; it does not cancel this ejection"
                    .into();
            self.record.progress = value["progress"]
                .as_f64()
                .or_else(|| value["progress"].as_str()?.parse().ok())
                .filter(|v| v.is_finite())
                .map(|v| v.clamp(0., 1.));
            if self.reconcile || tokio::time::Instant::now() >= deadline {
                self.record.state = OperationState::OutcomeUnknown;
                self.record.can_cancel = false;
                self.record.detail="Pool dissolution task is still pending; the pool remains reserved and no write was replayed".into();
                self.save(j).await?;
                return Ok(false);
            }
            self.save(j).await?;
            tokio::select! {_=c.cancelled()=>return Err(Error::Closed),_=tokio::time::sleep(Duration::from_millis(500))=>{}}
        }
    }
    async fn finish_pool_dissolve(
        &mut self,
        j: &mut Journal,
        s: &Session,
        c: &CancellationToken,
    ) -> Result<(), Error> {
        self.reload(j)?;
        if self.stopped_after_recovery() {
            return Ok(());
        }
        tokio::select! {_=c.cancelled()=>return Err(Error::Closed),v=pool_dissolve::state(s,&self.record)=>v?};
        self.reload(j)?;
        if self.stopped_after_recovery() {
            return Ok(());
        }
        let complete = self
            .record
            .steps
            .iter()
            .all(|v| v.state == StepState::Verified);
        self.record.state = if complete {
            OperationState::Succeeded
        } else {
            OperationState::PartiallyCompleted
        };
        self.record.can_cancel = false;
        self.record.progress = Some(
            self.record
                .steps
                .iter()
                .filter(|v| v.state == StepState::Verified)
                .count() as f64
                / self.record.steps.len() as f64,
        );
        self.record.detail = if complete {
            "Reviewed members were ejected and the remaining single-host pool name and description were cleared. Ejected-host reboot and independent connectivity are not verified.".into()
        } else {
            "Submitted dissolution steps verified; remaining member ejections or metadata changes were not replayed. Review the remaining pool separately.".into()
        };
        self.save(j).await?;
        if self.reconcile {
            return Ok(());
        }
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        for index in 0..self.record.steps.len() {
            if c.is_cancelled() || tokio::time::Instant::now() >= deadline {
                break;
            }
            let evidence = self.pool_dissolve_step(index)?.clone();
            if evidence.ended
                && evidence.task_uuid.is_some()
                && let Ok(Ok((_, task))) =
                    tokio::time::timeout_at(deadline, pool_dissolve::task(s, &self.record, index))
                        .await
                && matches!(
                    task["status"].as_str(),
                    Some("success" | "failure" | "cancelled")
                )
            {
                let _ = tokio::time::timeout_at(
                    deadline,
                    s.call("task.destroy", vec![json!(evidence.task)]),
                )
                .await;
            }
        }
        Ok(())
    }
}
