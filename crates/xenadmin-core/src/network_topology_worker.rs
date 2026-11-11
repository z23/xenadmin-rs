use super::*;
use crate::{
    network_topology::{Kind, Receipt},
    resources::topology,
};
use xenadmin_xapi::records::TaskRecord;

impl ResourceRequest {
    fn topology_step(&self, index: usize) -> Result<&Receipt, Error> {
        self.record
            .network_topology_plan
            .as_ref()
            .and_then(|p| p.steps.get(index))
            .ok_or(Error::Journal)
    }
    fn topology_step_mut(&mut self, index: usize) -> Result<&mut Receipt, Error> {
        self.record
            .network_topology_plan
            .as_mut()
            .and_then(|p| p.steps.get_mut(index))
            .ok_or(Error::Journal)
    }
    pub(super) async fn execute_topology(
        &mut self,
        j: &mut Journal,
        s: &Session,
        c: &CancellationToken,
    ) -> Result<(), Error> {
        self.record.detail = "Checking every pool host and the reviewed network graph".into();
        self.save(j).await?;
        let plan = tokio::select! {_=c.cancelled()=>return Err(Error::Closed),value=topology::prepare(s,&self.record)=>value?};
        self.record.steps = plan
            .steps
            .iter()
            .map(|step| OperationStep {
                label: step.kind.label(),
                state: StepState::Pending,
                detail: String::new(),
            })
            .collect();
        self.record.network_topology_plan = Some(Box::new(plan));
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
            if topology::now()
                >= self
                    .record
                    .network_topology_plan
                    .as_ref()
                    .ok_or(Error::Journal)?
                    .expires_at_ms
            {
                return Err(Error::Timeout);
            }
            let (method, args) = tokio::select! {_=c.cancelled()=>return Err(Error::Closed),value=topology::arguments(s,&self.record,index)=>value?};
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
            if topology::now()
                >= self
                    .record
                    .network_topology_plan
                    .as_ref()
                    .ok_or(Error::Journal)?
                    .expires_at_ms
            {
                return Err(Error::Timeout);
            }
            self.record.state = OperationState::Dispatching;
            self.record.can_cancel = false;
            self.record.steps[index].state = StepState::Dispatching;
            self.record.detail = format!("Submitting {}", self.record.steps[index].label);
            self.save_dispatch(j)?;
            let value = tokio::select! {biased; _=c.cancelled()=>return Err(Error::Closed),value=s.call(method,args)=>value?};
            self.topology_step_mut(index)?.task = Some(xapi::reply_reference(&value)?);
            self.record = j.topology_evidence(&self.record, index)?;
            self.announce().await;
            if self.stopped_after_recovery() {
                return Ok(());
            }
            if !self.track_topology_step(j, s, c, index).await? {
                return Ok(());
            }
        }
        self.finish_topology(j, s, c).await
    }
    pub(super) async fn reconcile_topology(
        &mut self,
        j: &mut Journal,
        s: &Session,
        c: &CancellationToken,
    ) -> Result<(), Error> {
        self.reload(j)?;
        if self.stopped_after_recovery() {
            return Ok(());
        }
        topology::validate_record(&self.record)?;
        if self.record.network_topology_plan.is_none() {
            return Err(Error::Protocol(
                "Topology preparation evidence unavailable; no request was replayed",
            ));
        }
        self.record.state = OperationState::Verifying;
        self.record.can_cancel = false;
        self.record.detail =
            "Inspecting dispatched network changes; unsubmitted steps will not run".into();
        self.save(j).await?;
        for index in 0..self.record.steps.len() {
            self.reload(j)?;
            if self.stopped_after_recovery() {
                return Ok(());
            }
            if self.record.steps[index].state == StepState::Pending
                || self.topology_step(index)?.verified
            {
                continue;
            }
            if !self.track_topology_step(j, s, c, index).await? {
                return Ok(());
            }
        }
        self.finish_topology(j, s, c).await
    }
    async fn track_topology_step(
        &mut self,
        j: &mut Journal,
        s: &Session,
        c: &CancellationToken,
        index: usize,
    ) -> Result<bool, Error> {
        loop {
            self.reload(j)?;
            if self.stopped_after_recovery() {
                return Ok(false);
            }
            let receipt = self.topology_step(index)?.clone();
            if receipt.completed {
                self.verify_topology_step(j, s, c, index).await?;
                return Ok(!self.stopped_after_recovery());
            }
            if receipt.ended {
                return Err(Error::Protocol(
                    "The network task failed or was cancelled; partial host changes may remain",
                ));
            }
            let task = receipt.task.ok_or(Error::Protocol(
                "Topology task receipt unavailable; no operation will be replayed",
            ))?;
            let value = tokio::select! {_=c.cancelled()=>return Err(Error::Closed),value=s.call("task.get_record",vec![json!(&task)])=>value?};
            let uuid: crate::TaskId = crate::resources::network::uuid(&value)?.into();
            let plan = self
                .record
                .network_topology_plan
                .as_ref()
                .ok_or(Error::Journal)?;
            if uuid.uuid().is_nil()
                || receipt.task_uuid.is_some_and(|id| id != uuid)
                || plan.tasks_before.contains(&uuid)
                || plan
                    .steps
                    .iter()
                    .enumerate()
                    .any(|(i, step)| i != index && step.task_uuid == Some(uuid))
            {
                return Err(Error::Protocol(
                    "Topology task identity does not match this operation",
                ));
            }
            let step = self.topology_step_mut(index)?;
            step.task_uuid = Some(uuid);
            match TaskRecord::new(&value).status().as_str() {
                Some("success") => {
                    step.completed = true;
                    step.ended = true;
                    if step.kind.creates() {
                        step.result = Some(crate::xapi::snapshot_task_result(&value)?);
                    }
                }
                Some("failure" | "cancelled") => step.ended = true,
                Some("pending" | "cancelling") => {}
                _ => return Err(Error::Protocol("Topology task status unavailable")),
            }
            self.record = j.topology_evidence(&self.record, index)?;
            if self.topology_step(index)?.completed
                && matches!(self.topology_step(index)?.kind, Kind::Fabric(ref op) if op.creates())
            {
                let plan = self
                    .record
                    .network_topology_plan
                    .as_ref()
                    .ok_or(Error::Journal)?;
                let ids = tokio::select! {_=c.cancelled()=>return Err(Error::Closed),value=crate::resources::network_fabric::pool::identity(s,plan,index)=>value?};
                self.topology_step_mut(index)?.fabric_created = Some(ids);
                self.record = j.topology_evidence(&self.record, index)?;
            }
            self.announce().await;
            if self.stopped_after_recovery() {
                return Ok(false);
            }
            if self.topology_step(index)?.ended {
                continue;
            }
            self.record.state = OperationState::Tracking;
            self.record.progress = TaskRecord::new(&value)
                .progress()
                .as_f64()
                .or_else(|| TaskRecord::new(&value).progress().as_str()?.parse().ok())
                .filter(|v| v.is_finite())
                .map(|v| (index as f64 + v.clamp(0.0, 1.0)) / self.record.steps.len() as f64);
            self.record.can_cancel = TaskRecord::new(&value)
                .allowed_operations()
                .as_array()
                .is_some_and(|v| v.iter().any(|s| s == "cancel"));
            self.record.detail = format!("Host task running: {}", self.record.steps[index].label);
            self.save(j).await?;
            if !self.reconcile
                && self.record.cancel_requested
                && self.record.can_cancel
                && !self.topology_step(index)?.cancel_sent
            {
                self.reload(j)?;
                if self.stopped_after_recovery() {
                    return Ok(false);
                }
                self.topology_step_mut(index)?.cancel_sent = true;
                self.record.state = OperationState::CancellationRequested;
                self.save_dispatch(j)?;
                tokio::select! {biased; _=c.cancelled()=>return Err(Error::Closed),value=s.call("task.cancel",vec![json!(&task)])=>{value?;}}
            }
            if self.reconcile
                || topology::now()
                    >= self
                        .record
                        .network_topology_plan
                        .as_ref()
                        .ok_or(Error::Journal)?
                        .expires_at_ms
            {
                self.record.state = OperationState::OutcomeUnknown;
                self.record.can_cancel = false;
                self.record.detail="The network task is still pending; the pool remains reserved and no steps were replayed".into();
                self.save(j).await?;
                return Ok(false);
            }
            tokio::select! {_=c.cancelled()=>return Err(Error::Closed),_=tokio::time::sleep(Duration::from_millis(500))=>{}}
        }
    }
    async fn verify_topology_step(
        &mut self,
        j: &mut Journal,
        s: &Session,
        c: &CancellationToken,
        index: usize,
    ) -> Result<(), Error> {
        if matches!(self.topology_step(index)?.kind, Kind::Create { .. })
            && self.topology_step(index)?.created.is_none()
        {
            let plan = self
                .record
                .network_topology_plan
                .as_ref()
                .ok_or(Error::Journal)?;
            let ids = tokio::select! {_=c.cancelled()=>return Err(Error::Closed),value=topology::creation_identity(s,plan,index)=>value?};
            self.topology_step_mut(index)?.created = Some(ids);
            self.record = j.topology_evidence(&self.record, index)?;
            self.announce().await;
            if self.stopped_after_recovery() {
                return Ok(());
            }
        }
        if matches!(self.topology_step(index)?.kind, Kind::Fabric(ref op) if op.creates()) {
            let plan = self
                .record
                .network_topology_plan
                .as_ref()
                .ok_or(Error::Journal)?;
            let ids = tokio::select! {_=c.cancelled()=>return Err(Error::Closed),value=crate::resources::network_fabric::pool::identity(s,plan,index)=>value?};
            self.topology_step_mut(index)?.fabric_created = Some(ids);
            self.record = j.topology_evidence(&self.record, index)?;
            self.announce().await;
            if self.stopped_after_recovery() {
                return Ok(());
            }
        }
        let member = tokio::select! {_=c.cancelled()=>return Err(Error::Closed),value=topology::verify(s,&self.record,index)=>value?};
        for _ in 0..16 {
            self.reload(j)?;
            if self.stopped_after_recovery() {
                return Ok(());
            }
            self.topology_step_mut(index)?.fabric_member = member.clone();
            self.topology_step_mut(index)?.verified = true;
            self.record.topology_changes = self
                .record
                .network_topology_plan
                .as_ref()
                .ok_or(Error::Journal)?
                .changes();
            self.record.steps[index].state = StepState::Verified;
            self.record.steps[index].detail = "Task completion and fresh topology verified".into();
            self.record.state = if self.reconcile {
                OperationState::Verifying
            } else {
                OperationState::Preparing
            };
            self.record.detail = format!("Verified {}", self.record.steps[index].label);
            self.record.can_cancel = !self.reconcile;
            match self.save(j).await {
                Ok(()) => return Ok(()),
                Err(Error::JournalChanged) => continue,
                Err(error) => return Err(error),
            }
        }
        Err(Error::JournalChanged)
    }
    async fn finish_topology(
        &mut self,
        j: &mut Journal,
        s: &Session,
        c: &CancellationToken,
    ) -> Result<(), Error> {
        self.reload(j)?;
        if self.stopped_after_recovery() {
            return Ok(());
        }
        tokio::select! {_=c.cancelled()=>return Err(Error::Closed),value=topology::finish(s,&self.record)=>value?};
        self.reload(j)?;
        if self.stopped_after_recovery() {
            return Ok(());
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
        self.record.progress = Some(
            self.record
                .steps
                .iter()
                .filter(|step| step.state == StepState::Verified)
                .count() as f64
                / self.record.steps.len() as f64,
        );
        self.record.can_cancel = false;
        self.record.detail=if self.record.state==OperationState::Succeeded {"All reviewed pool network changes were verified"}else{"Dispatched network changes verified; unsubmitted steps remain. Review current topology before a new operation"}.into();
        self.save(j).await?;
        let steps = self
            .record
            .network_topology_plan
            .as_ref()
            .ok_or(Error::Journal)?
            .steps
            .clone();
        let cleanup_deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        for step in steps {
            if c.is_cancelled() || tokio::time::Instant::now() >= cleanup_deadline {
                break;
            }
            if let (Some(reference), Some(uuid)) = (step.task, step.task_uuid)
                && step.ended
                && !c.is_cancelled()
            {
                let current = tokio::time::timeout_at(
                    cleanup_deadline,
                    s.call("task.get_record", vec![json!(&reference)]),
                )
                .await;
                if current.is_ok_and(|v| {
                    v.is_ok_and(|v| crate::resources::network::uuid(&v).ok() == Some(uuid.uuid()))
                }) {
                    let _ = tokio::time::timeout_at(
                        cleanup_deadline,
                        s.call("task.destroy", vec![json!(reference)]),
                    )
                    .await;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod dispatch_tests {
    use super::*;
    #[tokio::test]
    async fn final_dispatch_does_not_wait_for_publication_and_recovery_invalidates_its_cas() {
        for recovered in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let j = Journal::open(dir.path()).unwrap();
            let pool = uuid::Uuid::new_v4().into();
            let (operation, mut request) = crate::resources::channel(
                OperationId::new(),
                crate::ObservedResource {
                    pool,
                    generation: 1,
                    target: crate::ResourceTarget::Network(uuid::Uuid::new_v4().into()),
                },
                crate::ResourceAction::ConfigureInternalNetwork {
                    expected_mtu: 1500,
                    mtu: 9000,
                },
                tokio::runtime::Handle::current(),
            );
            j.resource_admit(&mut request.record).unwrap();
            request.record.state = OperationState::Dispatching;
            j.resource_save(&mut request.record).unwrap();
            let (updates, mut queued) = tokio::sync::mpsc::channel(1);
            updates
                .try_send(OperationUpdate::Resource(Box::new(request.record.clone())))
                .unwrap();
            request.updates = Some(updates);
            if recovered {
                j.resource_recover(pool).unwrap();
            }
            request.record.detail = "Final authorized submission".into();
            let result = request.save_dispatch(&j);
            if recovered {
                assert!(matches!(result, Err(Error::JournalChanged)));
                assert_eq!(
                    j.resource_load(request.record.id).unwrap().unwrap().state,
                    OperationState::OutcomeUnknown
                );
            } else {
                result.unwrap();
                assert_eq!(operation.latest().detail, "Final authorized submission");
                assert_eq!(
                    j.resource_load(request.record.id)
                        .unwrap()
                        .unwrap()
                        .revision(),
                    request.record.revision()
                );
                assert!(queued.try_recv().is_ok());
                assert!(queued.try_recv().is_err());
            }
        }
    }
}
