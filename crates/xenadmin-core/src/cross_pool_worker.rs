use super::*;
use crate::{ResourceAction, cross_pool_backend as backend};

struct ReceiverSession {
    session: Arc<Session>,
    token: zeroize::Zeroizing<String>,
    keep_for_server: bool,
}
impl Drop for ReceiverSession {
    fn drop(&mut self) {
        if !self.keep_for_server {
            let session = self.session.clone();
            let token = self.token.clone();
            tokio::spawn(async move {
                let _ = tokio::time::timeout(
                    Duration::from_secs(3),
                    session.logout_migration_receiver(&token),
                )
                .await;
            });
        }
    }
}

fn peer_current(
    inputs: &crate::cross_pool::Inputs,
    peer: &crate::connection::JoinPeer,
) -> Result<(), Error> {
    if inputs.destination.cancel.is_cancelled() || peer.cancel.is_cancelled() {
        return Err(Error::Closed);
    }
    if peer
        .receiver
        .borrow()
        .observe_resource(inputs.observation.target())
        .is_ok_and(|v| v == inputs.observation)
    {
        Ok(())
    } else {
        Err(Error::Stale)
    }
}
impl ResourceRequest {
    fn cross_plan(&self) -> Result<&backend::Plan, Error> {
        self.record
            .cross_pool_plan
            .as_deref()
            .ok_or(Error::MigrationConflict)
    }
    fn cross_plan_mut(&mut self) -> Result<&mut backend::Plan, Error> {
        self.record
            .cross_pool_plan
            .as_deref_mut()
            .ok_or(Error::MigrationConflict)
    }
    pub(super) async fn execute_cross_pool(
        &mut self,
        journal: &mut Journal,
        source: &Session,
        cancel: &CancellationToken,
    ) -> Result<(), Error> {
        let ResourceAction::CrossPoolVm(operation) = &self.record.action else {
            return Err(Error::InvalidMigration);
        };
        let operation = operation.clone();
        let inputs = self.cross_pool.take().ok_or(Error::InvalidMigration)?;
        let peer = tokio::select! { _=cancel.cancelled()=>return Err(Error::Closed), peer=inputs.destination.get(inputs.observation)=>peer? };
        peer_current(&inputs, &peer)?;
        self.record.detail =
            "Checking both pools, exact guest devices and destination capacity".into();
        self.save(journal).await?;
        let (plan, prepared) = tokio::select! { _=cancel.cancelled()=>return Err(Error::Closed), _=peer.cancel.cancelled()=>return Err(Error::Closed), result=backend::prepare(source,&peer.session,self.record.pool,&operation)=>result? };
        self.reload(journal)?;
        if self.stop_before_dispatch() {
            return self.save(journal).await;
        }
        self.record.cross_pool_plan = Some(Box::new(plan));
        self.record.steps = [
            "Check host migration compatibility",
            "Transfer VM between pools",
            "Verify destination and source disposition",
        ]
        .into_iter()
        .map(|label| OperationStep {
            label: label.into(),
            state: StepState::Pending,
            detail: String::new(),
        })
        .collect();
        self.record.state = OperationState::Dispatching;
        self.record.steps[0].state = StepState::Dispatching;
        self.record.detail = "Preparing the reviewed host-to-host migration transport".into();
        self.save(journal).await?;
        peer_current(&inputs, &peer)?;
        let value = tokio::select! { _=cancel.cancelled()=>return Err(Error::Closed), _=peer.cancel.cancelled()=>return Err(Error::Closed), value=peer.session.call("host.migrate_receive",vec![json!(&prepared.host),json!(&prepared.network),json!({})])=>value? };
        let receiver = backend::receiver(value, &prepared, &operation.transfer_address)?;
        if receiver["session_id"] == peer.session.session_cookie() {
            return Err(Error::Protocol(
                "Migration receiver reused the active desktop session",
            ));
        }
        let mut receiver_session = ReceiverSession {
            session: peer.session.clone(),
            token: zeroize::Zeroizing::new(
                receiver["session_id"]
                    .as_str()
                    .ok_or(Error::MigrationConflict)?
                    .into(),
            ),
            keep_for_server: false,
        };
        let args = backend::arguments(self.cross_plan()?, &prepared, receiver);
        tokio::select! { _=cancel.cancelled()=>return Err(Error::Closed), _=peer.cancel.cancelled()=>return Err(Error::Closed), result=source.call("VM.assert_can_migrate",args.clone())=>{result?;} }
        self.reload(journal)?;
        if self.stopped_after_recovery() {
            return Ok(());
        }
        self.record.steps[0].state = StepState::Verified;
        self.record.steps[0].detail = "Both hosts accepted the reviewed compatibility check".into();
        self.record.state = OperationState::Preparing;
        self.save(journal).await?;
        if self.stop_before_dispatch() {
            return self.save(journal).await;
        }
        peer_current(&inputs, &peer)?;
        let current = tokio::select! { _=cancel.cancelled()=>return Err(Error::Closed), _=peer.cancel.cancelled()=>return Err(Error::Closed), result=backend::revalidate(source,&peer.session,self.cross_plan()?)=>result? };
        if current.vm != prepared.vm
            || current.host != prepared.host
            || current.network != prepared.network
            || current.coordinator_address != prepared.coordinator_address
            || current.disks != prepared.disks
            || current.nics != prepared.nics
        {
            return Err(Error::MigrationConflict);
        }
        let tasks = crate::device_lifecycle::baseline(source, "task")
            .await?
            .into_iter()
            .collect();
        self.reload(journal)?;
        if self.stop_before_dispatch() {
            return self.save(journal).await;
        }
        peer_current(&inputs, &peer)?;
        if backend::now() >= self.cross_plan()?.expires_at_ms {
            return Err(Error::Timeout);
        }
        self.cross_plan_mut()?.source_tasks = Some(tasks);
        self.record.state = OperationState::Dispatching;
        self.record.steps[1].state = StepState::Dispatching;
        self.record.detail =
            "Submitting the reviewed cross-pool transfer; both pools remain reserved".into();
        self.save(journal).await?;
        receiver_session.keep_for_server = true;
        let reply = tokio::select! { _=cancel.cancelled()=>return Err(Error::Closed), _=peer.cancel.cancelled()=>return Err(Error::Closed), value=source.call("Async.VM.migrate_send",args)=>value };
        if matches!(reply, Err(Error::Api(_) | Error::Coordinator(_))) {
            receiver_session.keep_for_server = false;
        }
        let reply = reply?;
        self.record.task = Some(xapi::reply_reference(&reply)?);
        self.record = journal.resource_evidence(&self.record)?;
        self.announce().await;
        if self.stopped_after_recovery() {
            return Ok(());
        }
        let result = self
            .track_cross_pool(journal, source, &peer, &inputs, cancel)
            .await;
        if self.cross_plan().is_ok_and(|p| p.ended) {
            receiver_session.keep_for_server = false;
        }
        result
    }
    pub(super) async fn recheck_cross_pool(
        &mut self,
        journal: &mut Journal,
        source: &Session,
        cancel: &CancellationToken,
    ) -> Result<(), Error> {
        self.reload(journal)?;
        if self.record.state.terminal() {
            return Ok(());
        }
        self.record.state = OperationState::Verifying;
        self.record.can_cancel = false;
        self.record.detail =
            "Rechecking the recorded cross-pool transfer; no transfer will be replayed".into();
        self.save(journal).await?;
        let inputs = self.cross_pool.take().ok_or(Error::Protocol("Reconnect both pools and use Recheck cross-pool transfer; no transfer will be replayed"))?;
        let peer = tokio::select! { _=cancel.cancelled()=>return Err(Error::Closed), peer=inputs.destination.get(inputs.observation)=>peer? };
        peer_current(&inputs, &peer)?;
        backend::pool(source, self.record.pool).await?;
        backend::pool(&peer.session, self.cross_plan()?.operation.destination_pool).await?;
        self.track_cross_pool(journal, source, &peer, &inputs, cancel)
            .await
    }
    async fn track_cross_pool(
        &mut self,
        journal: &mut Journal,
        source: &Session,
        peer: &crate::connection::JoinPeer,
        inputs: &crate::cross_pool::Inputs,
        cancel: &CancellationToken,
    ) -> Result<(), Error> {
        loop {
            self.reload(journal)?;
            if self.stopped_after_recovery() {
                return Ok(());
            }
            peer_current(inputs, peer)?;
            if self.cross_plan()?.completed {
                return self
                    .finish_cross_pool(journal, source, peer, inputs, cancel)
                    .await;
            }
            if self.cross_plan()?.ended {
                return Err(Error::Protocol(
                    "The host transfer task ended without success; partial destination resources remain possible. Nothing was replayed",
                ));
            }
            let task = self.record.task.clone().ok_or(Error::Protocol(
                "Cross-pool submission receipt is unavailable; nothing will be replayed",
            ))?;
            let value = tokio::select! { _=cancel.cancelled()=>return Err(Error::Closed), _=peer.cancel.cancelled()=>return Err(Error::Closed), value=source.call("task.get_record",vec![json!(&task)])=>value? };
            let uuid = crate::migration_backend::id::<uuid::Uuid>(&value)?;
            let plan = self.cross_plan()?;
            if uuid.is_nil()
                || plan.task_uuid.is_some_and(|id| id != uuid)
                || plan
                    .source_tasks
                    .as_ref()
                    .is_none_or(|ids| ids.contains(&uuid))
            {
                return Err(Error::Protocol(
                    "Cross-pool task identity does not match the submitted operation",
                ));
            }
            self.cross_plan_mut()?.task_uuid = Some(uuid);
            match value["status"].as_str() {
                Some("success") => {
                    self.cross_plan_mut()?.completed = true;
                    self.cross_plan_mut()?.ended = true;
                    self.record.result_reference = Some(crate::xapi::snapshot_task_result(&value)?);
                }
                Some("failure" | "cancelled") => {
                    self.cross_plan_mut()?.ended = true;
                }
                Some("pending" | "cancelling") => {}
                _ => return Err(Error::Protocol("Unknown cross-pool task state")),
            }
            self.record = journal.resource_evidence(&self.record)?;
            self.announce().await;
            if self.stopped_after_recovery() {
                return Ok(());
            }
            if self.cross_plan()?.completed {
                continue;
            }
            if self.cross_plan()?.ended {
                return Err(Error::Protocol(
                    "The host transfer task failed or was cancelled. Partial destination resources may remain; recheck or acknowledge the unresolved outcome",
                ));
            }
            self.record.state = OperationState::Tracking;
            self.record.progress = value["progress"]
                .as_f64()
                .or_else(|| value["progress"].as_str()?.parse().ok())
                .filter(|n| n.is_finite())
                .map(|n| n.clamp(0.0, 1.0));
            self.record.can_cancel = value["allowed_operations"]
                .as_array()
                .is_some_and(|a| a.iter().any(|s| s == "cancel"));
            self.record.detail =
                "Hosts are transferring the VM; source and destination pools are reserved".into();
            self.save(journal).await?;
            if self.record.cancel_requested
                && self.record.can_cancel
                && !self.record.cancel_dispatched
            {
                self.reload(journal)?;
                if self.stopped_after_recovery() {
                    return Ok(());
                }
                self.record.cancel_dispatched = true;
                self.record.state = OperationState::CancellationRequested;
                self.record.detail =
                    "Requesting cancellation; source or destination changes may already exist"
                        .into();
                self.save(journal).await?;
                tokio::select! { _=cancel.cancelled()=>return Err(Error::Closed), _=peer.cancel.cancelled()=>return Err(Error::Closed), value=source.call("task.cancel",vec![json!(&task)])=>{value?;} }
            }
            if self.reconcile || backend::now() >= self.cross_plan()?.expires_at_ms {
                self.record.state = OperationState::OutcomeUnknown;
                self.record.can_cancel = false;
                self.record.detail = "The existing host task is still pending. Both pools remain reserved; recheck never repeats the transfer".into();
                return self.save(journal).await;
            }
            tokio::select! { _=cancel.cancelled()=>return Err(Error::Closed), _=peer.cancel.cancelled()=>return Err(Error::Closed), _=tokio::time::sleep(Duration::from_secs(1))=>{} }
        }
    }
    async fn finish_cross_pool(
        &mut self,
        journal: &mut Journal,
        source: &Session,
        peer: &crate::connection::JoinPeer,
        inputs: &crate::cross_pool::Inputs,
        cancel: &CancellationToken,
    ) -> Result<(), Error> {
        let reference = self
            .record
            .result_reference
            .clone()
            .ok_or(Error::MigrationConflict)?;
        let id = tokio::select! { _=cancel.cancelled()=>return Err(Error::Closed), _=peer.cancel.cancelled()=>return Err(Error::Closed), result=backend::identify(&peer.session,self.cross_plan()?,&reference)=>result? };
        self.cross_plan_mut()?.destination_vm = Some(id);
        if !self.record.transfer_created.vms.contains(&id) {
            self.record.transfer_created.vms.push(id);
        }
        self.record = journal.resource_evidence(&self.record)?;
        self.announce().await;
        if self.stopped_after_recovery() {
            return Ok(());
        }
        self.record.state = OperationState::Verifying;
        self.record.steps[1].state = StepState::Verified;
        self.record.steps[2].state = StepState::Dispatching;
        self.record.detail =
            "Verifying exact destination identities, device placement and source disposition"
                .into();
        self.save(journal).await?;
        let created = tokio::select! { _=cancel.cancelled()=>return Err(Error::Closed), _=peer.cancel.cancelled()=>return Err(Error::Closed), result=backend::verify_destination(&peer.session,self.cross_plan()?,&reference)=>result? };
        peer_current(inputs, peer)?;
        self.record.transfer_created = created;
        self.record = journal.resource_evidence(&self.record)?;
        if self.stopped_after_recovery() {
            return Ok(());
        }
        // XAPI removes the source VM and its devices after the migration task has already
        // reported success, and releases the source disks only on a later storage scan. Wait
        // within the connection's bound instead of leaving every ordinary move unknown.
        let wait = source.options().cross_pool_cleanup_wait;
        let started = std::time::Instant::now();
        let mut waited = false;
        let disposition = loop {
            let result = tokio::select! { _=cancel.cancelled()=>return Err(Error::Closed), _=peer.cancel.cancelled()=>return Err(Error::Closed), result=backend::verify_source(source,self.cross_plan()?)=>result };
            let elapsed = started.elapsed();
            match result {
                Ok(backend::SourceDisposition::Clean) => break backend::SourceDisposition::Clean,
                Ok(pending @ backend::SourceDisposition::DisksPending(_))
                    if elapsed >= wait.min(Duration::from_secs(10)) =>
                {
                    break pending;
                }
                Ok(backend::SourceDisposition::DisksPending(_)) => {}
                Err(Error::Protocol(reason))
                    if reason.contains("not yet verified") && elapsed < wait => {}
                Err(error) => return Err(error),
            }
            if !waited {
                waited = true;
                self.record.detail = "Destination verified; waiting for XAPI to finish removing the source VM, its devices and disks".into();
                let _ = self.save(journal).await;
            }
            tokio::select! { _=cancel.cancelled()=>return Err(Error::Closed), _=peer.cancel.cancelled()=>return Err(Error::Closed), _=tokio::time::sleep(Duration::from_secs(2))=>{} }
        };
        self.reload(journal)?;
        if self.stopped_after_recovery() {
            return Ok(());
        }
        peer_current(inputs, peer)?;
        self.record.state = OperationState::Succeeded;
        self.record.progress = Some(1.0);
        self.record.can_cancel = false;
        for step in &mut self.record.steps {
            step.state = StepState::Verified;
        }
        self.record.detail = if self.cross_plan()?.operation.mode.copy() {
            "Copy verified in the destination pool with new identities and MAC addresses; source VM remains halted and unchanged".into()
        } else {
            match disposition {
                backend::SourceDisposition::Clean => "Move verified in the destination pool; source VM and exclusive source disks were removed by XAPI".into(),
                backend::SourceDisposition::DisksPending(count) => format!("Move verified in the destination pool; the source VM and its attachments were removed, and {count} source disk record(s) remain until XAPI's next storage scan releases them"),
            }
        };
        self.save(journal).await?;
        if let Some(task) = &self.record.task {
            let _ = tokio::time::timeout(
                Duration::from_secs(5),
                source.call("task.destroy", vec![json!(task)]),
            )
            .await;
        }
        Ok(())
    }
}
