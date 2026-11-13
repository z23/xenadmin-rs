use super::*;
use crate::{resources::storage_graph, storage_graph::Evidence};
use xenadmin_xapi::enumerations::task_status_type;
use xenadmin_xapi::records::TaskRecord;
use xenadmin_xapi::{calls, refs};
impl ResourceRequest {
    fn storage_graph_step(&self, index: usize) -> Result<&Evidence, Error> {
        self.record
            .storage_graph_plan
            .as_ref()
            .and_then(|p| p.evidence.get(index))
            .ok_or(Error::Journal)
    }
    fn storage_graph_step_mut(&mut self, index: usize) -> Result<&mut Evidence, Error> {
        self.record
            .storage_graph_plan
            .as_mut()
            .and_then(|p| p.evidence.get_mut(index))
            .ok_or(Error::Journal)
    }
    pub(super) async fn execute_storage_graph(
        &mut self,
        j: &mut Journal,
        s: &Session,
        c: &CancellationToken,
    ) -> Result<(), Error> {
        self.record.detail = "Checking storage identity and reviewed host configuration".into();
        self.save(j).await?;
        let plan = tokio::select! {_=c.cancelled()=>return Err(Error::Closed),p=storage_graph::prepare(s,&self.record)=>p?};
        self.record.steps = plan
            .kinds
            .iter()
            .map(|k| OperationStep {
                label: k.label(),
                state: StepState::Pending,
                detail: String::new(),
            })
            .collect();
        self.record.storage_graph_plan = Some(Box::new(plan));
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
            let (method, args) = tokio::select! {_=c.cancelled()=>return Err(Error::Closed),v=storage_graph::arguments(s,&self.record,index)=>v?};
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
            let asynchronous = self
                .record
                .storage_graph_plan
                .as_ref()
                .ok_or(Error::Journal)?
                .kinds[index]
                .asynchronous();
            if asynchronous {
                self.storage_graph_step_mut(index)?.task =
                    Some(crate::snapshots::reference(&result)?);
            } else {
                self.storage_graph_step_mut(index)?.completed = true;
                self.storage_graph_step_mut(index)?.ended = true;
            }
            self.record = j.storage_graph_evidence(&self.record, index)?;
            self.announce().await;
            if self.stopped_after_recovery() {
                return Ok(());
            }
            if !self.track_storage_graph(j, s, c, index).await? {
                return Ok(());
            }
        }
        self.finish_storage_graph(j, s, c).await
    }
    pub(super) async fn reconcile_storage_graph(
        &mut self,
        j: &mut Journal,
        s: &Session,
        c: &CancellationToken,
    ) -> Result<(), Error> {
        self.reload(j)?;
        if self.stopped_after_recovery() {
            return Ok(());
        }
        crate::storage_graph::validate_record(&self.record)?;
        if self.record.storage_graph_plan.is_none() {
            return Err(Error::Protocol(
                "Storage preparation evidence unavailable; no write was replayed",
            ));
        }
        self.record.state = OperationState::Verifying;
        self.record.detail =
            "Inspecting previously submitted storage changes; remaining steps will not run".into();
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
            if !self.track_storage_graph(j, s, c, index).await? {
                return Ok(());
            }
        }
        self.finish_storage_graph(j, s, c).await
    }
    async fn track_storage_graph(
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
            if self.storage_graph_step(index)?.completed {
                let discovered = tokio::select! {_=c.cancelled()=>return Err(Error::Closed),v=storage_graph::discover_created(s,&mut self.record,index)=>v?};
                if discovered {
                    self.record = j.storage_graph_evidence(&self.record, index)?;
                    self.announce().await;
                    if self.stopped_after_recovery() {
                        return Ok(false);
                    }
                }
                tokio::select! {_=c.cancelled()=>return Err(Error::Closed),v=storage_graph::verify(s,&mut self.record,index)=>v?};
                self.record = j.storage_graph_evidence(&self.record, index)?;
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
                        "Completion receipt and fresh storage state verified".into();
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
            if self.storage_graph_step(index)?.ended {
                return Err(Error::Protocol(
                    "The storage task failed or was cancelled; inspect any retained changes before acknowledging",
                ));
            }
            let (uuid, value) = tokio::select! {_=c.cancelled()=>return Err(Error::Closed),v=storage_graph::task(s,&self.record,index)=>v?};
            self.storage_graph_step_mut(index)?.task_uuid = Some(uuid);
            match TaskRecord::new(&value).status().name() {
                Some(task_status_type::SUCCESS) => {
                    if matches!(
                        self.record
                            .storage_graph_plan
                            .as_ref()
                            .ok_or(Error::Journal)?
                            .kinds[index],
                        crate::storage_graph::Kind::Create(_)
                    ) {
                        self.storage_graph_step_mut(index)?.result =
                            Some(crate::xapi::snapshot_task_result(&value)?);
                    }
                    self.storage_graph_step_mut(index)?.completed = true;
                    self.storage_graph_step_mut(index)?.ended = true;
                }
                Some(task_status_type::FAILURE | task_status_type::CANCELLED) => {
                    self.storage_graph_step_mut(index)?.ended = true
                }
                Some(task_status_type::PENDING | task_status_type::CANCELLING) => {}
                _ => return Err(Error::Protocol("Storage task status unavailable")),
            }
            self.record = j.storage_graph_evidence(&self.record, index)?;
            self.announce().await;
            if self.stopped_after_recovery() {
                return Ok(false);
            }
            if self.storage_graph_step(index)?.ended {
                continue;
            }
            self.record.state = OperationState::Tracking;
            self.record.can_cancel = !self.reconcile;
            self.record.detail =
                "Host storage task is running. Stop prevents later steps; it does not cancel host I/O"
                    .into();
            self.record.progress = TaskRecord::new(&value)
                .progress()
                .number()
                .filter(|v| v.is_finite())
                .map(|v| v.clamp(0., 1.));
            if self.reconcile || tokio::time::Instant::now() >= deadline {
                self.record.state = OperationState::OutcomeUnknown;
                self.record.can_cancel = false;
                self.record.detail="Storage task is still pending; the pool remains reserved and no write was replayed".into();
                self.save(j).await?;
                return Ok(false);
            }
            self.save(j).await?;
            tokio::select! {_=c.cancelled()=>return Err(Error::Closed),_=tokio::time::sleep(Duration::from_millis(500))=>{}}
        }
    }
    async fn finish_storage_graph(
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
            "Reviewed storage connections and host policy verified. Attached status does not establish healthy paths or storage I/O.".into()
        } else {
            "Submitted storage changes verified; remaining steps were not replayed. Review Restore connections separately to reconnect retained detached attachments.".into()
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
            let evidence = self.storage_graph_step(index)?.clone();
            if evidence.ended
                && evidence.task_uuid.is_some()
                && let Some(reference) = evidence.task.as_deref()
                && let Ok(Ok((_, task))) =
                    tokio::time::timeout_at(deadline, storage_graph::task(s, &self.record, index))
                        .await
                && matches!(
                    TaskRecord::new(&task).status().name(),
                    Some(
                        task_status_type::SUCCESS
                            | task_status_type::FAILURE
                            | task_status_type::CANCELLED
                    )
                )
            {
                let _ = tokio::time::timeout_at(
                    deadline,
                    s.send(calls::task::Destroy(refs::task(reference))),
                )
                .await;
            }
        }
        Ok(())
    }
}
