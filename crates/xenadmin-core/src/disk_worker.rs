use super::*;
use crate::{
    disk_administration::{Evidence, Kind},
    resources::disk,
};
impl ResourceRequest {
    fn disk_step(&self, index: usize) -> Result<&Evidence, Error> {
        self.record
            .disk_plan
            .as_ref()
            .and_then(|p| p.evidence.get(index))
            .ok_or(Error::Journal)
    }
    fn disk_step_mut(&mut self, index: usize) -> Result<&mut Evidence, Error> {
        self.record
            .disk_plan
            .as_mut()
            .and_then(|p| p.evidence.get_mut(index))
            .ok_or(Error::Journal)
    }
    pub(super) async fn execute_disk(
        &mut self,
        j: &mut Journal,
        s: &Session,
        c: &CancellationToken,
    ) -> Result<(), Error> {
        self.record.detail = "Checking disk identity, every consumer and recorded ownership".into();
        self.save(j).await?;
        let plan = tokio::select! {_=c.cancelled()=>return Err(Error::Closed),p=disk::prepare(s,&self.record,j)=>p?};
        self.record.steps = plan
            .kinds
            .iter()
            .map(|k| OperationStep {
                label: k.label().into(),
                state: StepState::Pending,
                detail: String::new(),
            })
            .collect();
        self.record.disk_plan = Some(plan);
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
            let (method, args) = tokio::select! {_=c.cancelled()=>return Err(Error::Closed),v=disk::arguments(s,&self.record,j,index)=>v?};
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
            self.record.state = OperationState::Dispatching;
            self.record.steps[index].state = StepState::Dispatching;
            self.record.detail = format!("Submitting {}", self.record.steps[index].label);
            self.record.can_cancel = false;
            self.save(j).await?;
            let result = tokio::select! {_=c.cancelled()=>return Err(Error::Closed),v=s.call(method,args)=>v?};
            let asynchronous =
                self.record.disk_plan.as_ref().ok_or(Error::Journal)?.kinds[index].asynchronous();
            if asynchronous {
                self.disk_step_mut(index)?.task = Some(crate::snapshots::reference(&result)?);
            } else {
                self.disk_step_mut(index)?.completed = true;
                self.disk_step_mut(index)?.ended = true;
            }
            self.record = j.disk_evidence(&self.record, index)?;
            self.announce().await;
            if self.stopped_after_recovery() {
                return Ok(());
            }
            if !self.track_disk(j, s, c, index).await? {
                return Ok(());
            }
        }
        self.finish_disk(j, s, c).await
    }
    pub(super) async fn reconcile_disk(
        &mut self,
        j: &mut Journal,
        s: &Session,
        c: &CancellationToken,
    ) -> Result<(), Error> {
        self.reload(j)?;
        if self.stopped_after_recovery() {
            return Ok(());
        }
        crate::disk_administration::validate_record(&self.record)?;
        if self.record.disk_plan.is_none() {
            return Err(Error::Protocol(
                "Disk preparation evidence unavailable; no write was replayed",
            ));
        }
        self.record.state = OperationState::Verifying;
        self.record.detail =
            "Inspecting previously submitted disk changes; remaining steps will not run".into();
        self.record.can_cancel = false;
        self.save(j).await?;
        for index in 0..self.record.steps.len() {
            self.reload(j)?;
            if self.stopped_after_recovery() {
                return Ok(());
            }
            if self.record.steps[index].state == StepState::Pending {
                continue;
            }
            if !self.track_disk(j, s, c, index).await? {
                return Ok(());
            }
        }
        self.finish_disk(j, s, c).await
    }
    async fn track_disk(
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
            if self.disk_step(index)?.completed {
                tokio::select! {_=c.cancelled()=>return Err(Error::Closed),v=disk::verify(s,&mut self.record,index)=>v?};
                self.record = j.disk_evidence(&self.record, index)?;
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
                        "Completion receipt and fresh disk state verified".into();
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
            if self.disk_step(index)?.ended {
                return Err(Error::Protocol(
                    "The disk task failed or was cancelled; inspect any retained changes before acknowledging",
                ));
            }
            let (uuid, value) = tokio::select! {_=c.cancelled()=>return Err(Error::Closed),v=disk::task(s,&self.record,index)=>v?};
            self.disk_step_mut(index)?.task_uuid = Some(uuid);
            match value["status"].as_str() {
                Some("success") => {
                    self.disk_step_mut(index)?.completed = true;
                    self.disk_step_mut(index)?.ended = true;
                    if self.record.disk_plan.as_ref().ok_or(Error::Journal)?.kinds[index]
                        == Kind::Copy
                    {
                        self.disk_step_mut(index)?.result =
                            Some(crate::xapi::snapshot_task_result(&value)?);
                    }
                }
                Some("failure" | "cancelled") => self.disk_step_mut(index)?.ended = true,
                Some("pending" | "cancelling") => {}
                _ => return Err(Error::Protocol("Disk task status unavailable")),
            }
            self.record = j.disk_evidence(&self.record, index)?;
            self.announce().await;
            if self.stopped_after_recovery() {
                return Ok(false);
            }
            if self.disk_step(index)?.ended {
                continue;
            }
            self.record.state = OperationState::Tracking;
            self.record.can_cancel = !self.reconcile;
            self.record.detail =
                "Host disk task is running. Stop prevents later steps; it does not cancel host I/O"
                    .into();
            self.record.progress = value["progress"]
                .as_f64()
                .or_else(|| value["progress"].as_str()?.parse().ok())
                .filter(|v| v.is_finite())
                .map(|v| v.clamp(0., 1.));
            if self.reconcile || tokio::time::Instant::now() >= deadline {
                self.record.state = OperationState::OutcomeUnknown;
                self.record.can_cancel = false;
                self.record.detail="Disk task is still pending; the pool remains reserved and no write was replayed".into();
                self.save(j).await?;
                return Ok(false);
            }
            self.save(j).await?;
            tokio::select! {_=c.cancelled()=>return Err(Error::Closed),_=tokio::time::sleep(Duration::from_millis(500))=>{}}
        }
    }
    async fn finish_disk(
        &mut self,
        j: &mut Journal,
        s: &Session,
        c: &CancellationToken,
    ) -> Result<(), Error> {
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
            if let Some(size) = self.record.disk_size {
                format!(
                    "Disk backing size verified at {size} bytes; guest partition and filesystem growth remains separate"
                )
            } else {
                "Reviewed disk changes verified; any copied disk remains detached".into()
            }
        } else {
            "Dispatched disk changes verified; unsubmitted steps were not replayed. Inspect retained disk UUIDs".into()
        };
        self.save(j).await?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        for index in 0..self.record.steps.len() {
            if c.is_cancelled() || tokio::time::Instant::now() >= deadline {
                break;
            }
            let evidence = self.disk_step(index)?.clone();
            if evidence.ended
                && evidence.task_uuid.is_some()
                && let Ok(Ok((_, task))) =
                    tokio::time::timeout_at(deadline, disk::task(s, &self.record, index)).await
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
