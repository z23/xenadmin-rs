use super::*;
use crate::{
    ResourceAction, TransferAction,
    transfer::{self, Plan, files, http},
};
use std::time::{SystemTime, UNIX_EPOCH};
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
impl ResourceRequest {
    pub(super) fn transfer_plan(&self) -> Result<&Plan, Error> {
        self.record
            .transfer_plan
            .as_deref()
            .ok_or(Error::TransferConflict)
    }
    pub(super) fn transfer_plan_mut(&mut self) -> Result<&mut Plan, Error> {
        self.record
            .transfer_plan
            .as_deref_mut()
            .ok_or(Error::TransferConflict)
    }
    pub(super) fn transfer_action(&self) -> Result<TransferAction, Error> {
        if let ResourceAction::Transfer(a) = &self.record.action {
            Ok(a.clone())
        } else {
            Err(Error::TransferConflict)
        }
    }
    pub(super) async fn execute_transfer(
        &mut self,
        journal: &mut Journal,
        session: &Session,
        cancel: &CancellationToken,
    ) -> Result<(), Error> {
        let action = self.transfer_action()?;
        if matches!(action, TransferAction::ImportDisk { .. }) {
            self.create_transfer_disk(journal, session, cancel).await?;
            if self.record.state.terminal() {
                return Ok(());
            }
        }
        let stream_index = usize::from(self.transfer_plan()?.disk_creation.is_some());
        transfer::xapi::prepare(
            session,
            self.record.pool,
            &action,
            Some(self.transfer_plan()?),
        )
        .await?;
        let previous_tasks = crate::device_lifecycle::baseline(session, "task").await?;
        let mut plan = self.transfer_plan()?.clone();
        if action.is_export() {
            let id = self.record.id;
            plan = tokio::task::spawn_blocking(move || {
                files::create_output(&mut plan, id)?;
                Ok::<_, Error>(plan)
            })
            .await
            .map_err(|_| Error::TransferFile)??;
        } else {
            let file = plan.file.clone();
            tokio::task::spawn_blocking(move || files::source(&file))
                .await
                .map_err(|_| Error::TransferFile)??;
            if matches!(action, TransferAction::ImportXva { .. }) {
                plan.baseline = Some(transfer::xapi::baseline(session).await?);
            }
        }
        plan.expires_at_ms = now().saturating_add(24 * 60 * 60 * 1000);
        self.reload(journal)?;
        if self.stop_before_dispatch() {
            files::cleanup(&plan);
            return self.save(journal).await;
        }
        self.record.transfer_plan = Some(Box::new(plan));
        self.record.steps = vec![
            OperationStep {
                label: "Transfer image".into(),
                state: StepState::Pending,
                detail: String::new(),
            },
            OperationStep {
                label: "Verify server result".into(),
                state: StepState::Pending,
                detail: String::new(),
            },
        ];
        if stream_index == 1 {
            self.record.steps.insert(
                0,
                OperationStep {
                    label: "Create unattached disk".into(),
                    state: StepState::Verified,
                    detail: "New disk UUID and ownership marker verified".into(),
                },
            );
        }
        let task_name = format!("XenAdmin RS transfer {}", self.record.id);
        self.record.detail = "Creating an owned transfer task".into();
        self.record.state = OperationState::Dispatching;
        self.save(journal).await?;
        let task = tokio::select! {_=cancel.cancelled()=>return Err(Error::Closed),v=session.call("task.create",vec![json!(&task_name),json!("Client-owned image transfer")])=>v?};
        self.record.task = Some(crate::resources::xapi::reply_reference(&task)?);
        self.record = journal.resource_evidence(&self.record)?;
        if self.record.state.terminal() {
            return Ok(());
        }
        let fresh_task = session
            .call("task.get_record", vec![json!(&self.record.task)])
            .await?;
        let task_uuid = fresh_task["uuid"]
            .as_str()
            .and_then(|v| v.parse().ok())
            .filter(|id| !previous_tasks.contains(id))
            .ok_or(Error::TransferConflict)?;
        if fresh_task["name_label"] != task_name || fresh_task["status"] != "pending" {
            return Err(Error::TransferConflict);
        }
        self.transfer_plan_mut()?.task_uuid = Some(task_uuid);
        self.save(journal).await?;
        if self.record.cancel_requested {
            self.record.state = if stream_index == 1 {
                OperationState::PartiallyCompleted
            } else {
                OperationState::Cancelled
            };
            self.record.detail = if stream_index == 1 {
                "Stopped before image upload; the new unattached disk is retained"
            } else {
                "Stopped before transferring image bytes"
            }
            .into();
            self.record.can_cancel = false;
            self.save(journal).await?;
            self.cleanup_transfer_task(session).await;
            files::cleanup(self.transfer_plan()?);
            return Ok(());
        }

        let mut prepared = transfer::xapi::prepare(
            session,
            self.record.pool,
            &action,
            Some(self.transfer_plan()?),
        )
        .await?;
        prepared.query.push((
            "task_id".into(),
            self.record.task.clone().ok_or(Error::TransferConflict)?,
        ));
        self.transfer_plan_mut()?.dispatched = true;
        self.record.state = OperationState::Tracking;
        self.record.can_cancel = true;
        self.record.steps[stream_index].state = StepState::Dispatching;
        self.record.detail = "Streaming image; host completion must also be verified".into();
        self.save(journal).await?;
        let meter = Arc::new(http::Meter::default());
        let outcome = {
            let stream = http::run(
                session,
                prepared,
                self.transfer_plan()?.clone(),
                meter.clone(),
            );
            tokio::pin!(stream);
            loop {
                tokio::select! {
                    _=cancel.cancelled()=>{meter.cancel.cancel();break Err(Error::Closed)},
                    result=&mut stream=>break result,
                    _=tokio::time::sleep(Duration::from_millis(250))=>{
                        self.reload(journal)?;
                        if self.record.state.terminal(){meter.cancel.cancel();return Ok(())}
                        if self.record.cancel_requested {meter.cancel.cancel();}
                        if now()>=self.transfer_plan()?.expires_at_ms {meter.cancel.cancel();break Err(Error::TransferExpired)}
                        self.record.transfer=Some(meter.progress());
                        self.record.detail=if self.record.cancel_requested{"Stopping the local stream; server cancellation still requires confirmation"}else{"Streaming image"}.into();
                        match self.save(journal).await{Ok(())|Err(Error::JournalChanged)=>{},Err(error)=>{meter.cancel.cancel();return Err(error)}}
                    }
                }
            }
        };
        self.reload(journal)?;
        if self.record.state.terminal() {
            return Ok(());
        }
        if !action.is_export() {
            let file = self.transfer_plan()?.file.clone();
            let unchanged = tokio::task::spawn_blocking(move || files::source(&file))
                .await
                .map_err(|_| Error::TransferFile)?;
            if unchanged.is_err() {
                self.transfer_plan_mut()?.stream_invalid = true;
            }
        }
        let mut progress = meter.progress();
        match outcome {
            Ok(receipt) => {
                progress.stream_complete = true;
                progress.phase = "Verifying host result".into();
                self.transfer_plan_mut()?.stream_receipt = Some(receipt);
                self.record.steps[stream_index].state = StepState::Verified;
                self.record.steps[stream_index].detail = "Complete stream receipt saved".into();
            }
            Err(error) => {
                if matches!(
                    error,
                    Error::TransferFileChanged | Error::TransferLimit | Error::TransferArchive
                ) {
                    self.transfer_plan_mut()?.stream_invalid = true;
                }
                progress.phase = "Stream stopped; checking host outcome".into();
                self.record.steps[stream_index].state = StepState::OutcomeUnknown;
                self.record.steps[stream_index].detail = error.to_string();
            }
        }
        self.record.transfer = Some(progress);
        self.save(journal).await?;
        self.recheck_transfer(journal, session, cancel).await
    }
    pub(super) async fn cleanup_transfer_task(&self, session: &Session) {
        if let Some(task) = &self.record.task
            && let Some(uuid) = self.record.transfer_plan.as_ref().and_then(|p| p.task_uuid)
            && let Ok(Ok(record)) = tokio::time::timeout(
                Duration::from_secs(5),
                session.call("task.get_record", vec![json!(task)]),
            )
            .await
            && record["uuid"]
                .as_str()
                .and_then(|v| v.parse::<uuid::Uuid>().ok())
                == Some(uuid)
        {
            let _ = tokio::time::timeout(
                Duration::from_secs(5),
                session.call("task.destroy", vec![json!(task)]),
            )
            .await;
        }
    }
    async fn collect_transfer_created(
        &mut self,
        session: &Session,
        complete: bool,
    ) -> Result<(), Error> {
        if matches!(self.transfer_action()?, TransferAction::ImportDisk { .. }) {
            let (reference, id) = transfer::disk::identify(
                session,
                self.transfer_plan()?,
                &self.transfer_action()?,
                self.record.id,
            )
            .await?;
            if complete && self.transfer_plan()?.result.as_deref() != Some(&[reference]) {
                return Err(Error::TransferConflict);
            }
            if !self.record.transfer_created.disks.contains(&id) {
                self.record.transfer_created.disks.push(id)
            }
            return Ok(());
        }
        if let TransferAction::ImportXva { sr } = self.transfer_action()? {
            let task = self.record.task.as_deref().ok_or(Error::TransferConflict)?;
            let found =
                transfer::xapi::created(session, self.transfer_plan()?, task, sr, complete).await?;
            // Retain positive identities even when a later read no longer sees an earlier resource.
            let old = &mut self.record.transfer_created;
            for v in found.vms {
                if !old.vms.contains(&v) {
                    old.vms.push(v)
                }
            }
            for v in found.disks {
                if !old.disks.contains(&v) {
                    old.disks.push(v)
                }
            }
            for v in found.networks {
                if !old.networks.contains(&v) {
                    old.networks.push(v)
                }
            }
        }
        Ok(())
    }
    pub(super) async fn recheck_transfer(
        &mut self,
        journal: &mut Journal,
        session: &Session,
        cancel: &CancellationToken,
    ) -> Result<(), Error> {
        self.reload(journal)?;
        if self.record.state.terminal() {
            return Ok(());
        }
        if !self.transfer_plan()?.dispatched && self.transfer_plan()?.disk_creation.is_some() {
            return self.recover_transfer_disk_creation(journal, session).await;
        }
        let task = self.record.task.clone().ok_or(Error::TransferConflict)?;
        if !self.transfer_plan()?.dispatched {
            return Err(Error::TransferConflict);
        }
        loop {
            self.reload(journal)?;
            if self.record.state.terminal() {
                return Ok(());
            }
            let known = self.transfer_plan()?;
            if known.task_success || known.task_failed || known.task_cancelled {
                break;
            }
            let value = tokio::select! {_=cancel.cancelled()=>return Err(Error::Closed),result=session.call("task.get_record",vec![json!(&task)])=>result?};
            self.reload(journal)?;
            if self.record.state.terminal() {
                return Ok(());
            }
            if value["uuid"].as_str().and_then(|s| s.parse().ok())
                != self.transfer_plan()?.task_uuid
                || self.transfer_plan()?.task_uuid.is_none()
            {
                return Err(Error::TransferConflict);
            }
            self.record.state = OperationState::Tracking;
            self.record.progress = value["progress"]
                .as_f64()
                .or_else(|| value["progress"].as_str()?.parse().ok())
                .filter(|n| n.is_finite())
                .map(|n| n.clamp(0.0, 1.0));
            self.record.can_cancel = value["allowed_operations"]
                .as_array()
                .is_some_and(|a| a.iter().any(|v| v == "cancel"));
            match value["status"].as_str() {
                Some("success") => {
                    let imported = !self.transfer_action()?.is_export();
                    let result = if imported {
                        Some(
                            if matches!(self.transfer_action()?, TransferAction::ImportDisk { .. })
                            {
                                vec![crate::xapi::snapshot_task_result(&value)?]
                            } else {
                                transfer::xapi::result(&value)?
                            },
                        )
                    } else {
                        None
                    };
                    let plan = self.transfer_plan_mut()?;
                    plan.task_success = true;
                    plan.result = result;
                }
                Some("failure") => {
                    self.transfer_plan_mut()?.task_failed = true;
                    let code = value["error_info"][0]
                        .as_str()
                        .unwrap_or("UNKNOWN_API_ERROR");
                    self.record.detail = format!(
                        "Host transfer task failed: {}",
                        crate::xapi::safe_error_code(code)
                    );
                }
                Some("cancelled") => {
                    self.transfer_plan_mut()?.task_cancelled = true;
                    self.record.detail = "Host confirmed transfer cancellation".into();
                }
                Some("pending" | "cancelling") => {}
                _ => return Err(Error::TransferConflict),
            }
            self.save(journal).await?;
            let p = self.transfer_plan()?;
            if p.task_success || p.task_failed || p.task_cancelled {
                break;
            }
            if self.record.cancel_requested
                && self.record.can_cancel
                && !self.record.cancel_dispatched
            {
                self.record.cancel_dispatched = true;
                self.record.state = OperationState::CancellationRequested;
                self.save(journal).await?;
                tokio::select! {_=cancel.cancelled()=>return Err(Error::Closed),v=session.call("task.cancel",vec![json!(&task)])=>{v?;}}
            }
            if now() >= self.transfer_plan()?.expires_at_ms {
                return Err(Error::TransferExpired);
            }
            // Rechecks and disconnected streams do not wait forever or repeat HTTP requests.
            if self.reconcile
                || !self
                    .record
                    .transfer
                    .as_ref()
                    .is_some_and(|p| p.stream_complete)
            {
                self.collect_transfer_created(session, false).await?;
                self.record.state = OperationState::OutcomeUnknown;
                self.record.detail="Stream is stopped; host task is still pending. Recheck reads the existing task and never transfers again.".into();
                return self.save(journal).await;
            }
            tokio::select! {_=cancel.cancelled()=>return Err(Error::Closed),_=tokio::time::sleep(Duration::from_secs(1))=>{}}
        }
        self.record.state = OperationState::Verifying;
        self.record.can_cancel = false;
        self.save(journal).await?;
        let success = self.transfer_plan()?.task_success;
        self.collect_transfer_created(session, success).await?;
        self.save(journal).await?;
        if success && self.transfer_action()?.is_export() {
            if self.transfer_plan()?.stream_receipt.is_none() {
                self.record.state = OperationState::PartiallyCompleted;
                self.record.detail="Host export completed, but a complete local archive receipt is unavailable. No final file was published.".into();
            } else {
                let plan = self.transfer_plan()?.clone();
                let action = self.transfer_action()?;
                if let TransferAction::ExportDisk {
                    vdi,
                    expected_sr,
                    format,
                    expected_size,
                } = &action
                {
                    let (_, record) =
                        transfer::disk::disk_record(session, *vdi, *expected_sr, *format, false)
                            .await?;
                    if record["virtual_size"]
                        .as_u64()
                        .or_else(|| record["virtual_size"].as_str()?.parse().ok())
                        != Some(*expected_size)
                    {
                        return Err(Error::TransferConflict);
                    }
                }
                tokio::task::spawn_blocking(move || {
                    match action {
                        TransferAction::ExportXva { source } => {
                            files::validate_export(&plan, &source)?
                        }
                        TransferAction::ExportDisk {
                            format,
                            expected_size,
                            ..
                        } => files::validate_disk_export(&plan, format, expected_size)?,
                        _ => return Err(Error::TransferConflict),
                    }
                    files::finalize(&plan)
                })
                .await
                .map_err(|_| Error::TransferFile)??;
                self.record.state = OperationState::Succeeded;
                self.record.detail = "Export verified and saved as a private image".into();
            }
        } else if success {
            self.record.state = if self.transfer_plan()?.stream_invalid {
                OperationState::PartiallyCompleted
            } else {
                OperationState::Succeeded
            };
            self.record.detail = if matches!(self.transfer_action()?,TransferAction::ImportDisk{..})&&!self.transfer_plan()?.stream_invalid {"Disk image imported into a new verified unattached disk"}else if self.transfer_plan()?.stream_invalid {"Host completed import, but source file integrity changed during upload. Retained resource UUIDs require inspection."}else{"Imported guest identities and storage verified; guests remain halted"}.into();
        } else if matches!(self.transfer_action()?, TransferAction::ImportDisk { .. }) {
            self.record.state = OperationState::PartiallyCompleted;
            self.record.detail="Disk transfer did not complete. The new unattached disk is retained with partial or unknown contents; no existing disk was overwritten.".into();
        } else if !self.transfer_action()?.is_export() {
            // VM/network creation precedes import_task tagging in XAPI. A failed task
            // cannot prove that every partially created resource was identified or removed.
            self.record.state = OperationState::OutcomeUnknown;
            self.record.detail="Host import task ended without success. Known retained UUIDs are listed, but untagged partial resources cannot be ruled out. Inspect the pool before acknowledging this outcome.".into();
        } else {
            self.record.state = if self.transfer_plan()?.task_cancelled {
                OperationState::Cancelled
            } else {
                OperationState::Failed
            };
        }
        if self.record.state == OperationState::Succeeded {
            self.record.progress = Some(1.0);
            for step in &mut self.record.steps {
                step.state = StepState::Verified;
            }
        } else {
            if let Some(step) = self.record.steps.last_mut() {
                step.state = StepState::Failed;
            }
        }
        if let Some(p) = &mut self.record.transfer {
            p.phase = self.record.detail.clone()
        }
        self.save(journal).await?;
        if self.record.state.terminal() {
            self.cleanup_transfer_task(session).await;
        }
        if self.transfer_action()?.is_export() && self.record.state != OperationState::Succeeded {
            files::cleanup(self.transfer_plan()?)
        }
        Ok(())
    }
}
