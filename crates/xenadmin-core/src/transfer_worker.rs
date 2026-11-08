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
    pub(super) fn save_transfer_progress(&mut self, journal: &mut Journal) -> Result<(), Error> {
        journal.resource_save(&mut self.record)?;
        self.sender.send_replace(Arc::new(self.record.clone()));
        // Progress can coalesce; final state is announced through the reliable path.
        if let Some(updates) = &self.updates {
            let _ = updates.try_send(OperationUpdate::Resource(Box::new(self.record.clone())));
        }
        Ok(())
    }

    async fn save_host_restore_stream(
        &mut self,
        journal: &mut Journal,
        outcome: Result<files::Receipt, Error>,
        mut progress: crate::TransferProgress,
    ) -> Result<bool, Error> {
        progress.stream_complete = outcome.is_ok();
        progress.phase = if outcome.is_ok() {
            "Upload stream complete; checking staged recovery"
        } else {
            "Upload stopped; backup partition outcome is uncertain"
        }
        .into();
        for _ in 0..16 {
            self.reload(journal)?;
            let expected = self
                .transfer_plan()?
                .file
                .host_backup
                .as_ref()
                .ok_or(Error::TransferConflict)?
                .sha256;
            match &outcome {
                Ok(receipt) => {
                    let p = self.transfer_plan_mut()?;
                    if p.stream_receipt.as_ref().is_some_and(|old| old != receipt) {
                        return Err(Error::TransferConflict);
                    }
                    p.stream_invalid |=
                        receipt.sha256 != expected || Some(receipt.bytes) != p.file.size();
                    p.stream_receipt = Some(receipt.clone());
                    if let Some(step) = self.record.steps.first_mut() {
                        step.state = StepState::Verified;
                        step.detail="Complete upload receipt saved; host staging still requires task verification".into();
                    }
                }
                Err(error) => {
                    if matches!(
                        error,
                        Error::TransferFileChanged | Error::TransferLimit | Error::TransferArchive
                    ) {
                        self.transfer_plan_mut()?.stream_invalid = true;
                    }
                    if let Some(step) = self.record.steps.first_mut() {
                        step.state = StepState::OutcomeUnknown;
                        step.detail = error.to_string();
                    }
                }
            }
            self.record.transfer = Some(progress.clone());
            match self.save(journal).await {
                Err(Error::JournalChanged) => continue,
                Err(e) => return Err(e),
                Ok(()) => return Ok(!self.stopped_after_recovery()),
            }
        }
        Err(Error::JournalChanged)
    }

    fn checkpoint_transfer(&self) -> bool {
        matches!(
            self.record.action,
            ResourceAction::Transfer(
                TransferAction::ImportCheckpointXva { .. }
                    | TransferAction::ExportXva {
                        source: crate::ExportSource::Checkpoint { .. }
                    }
            )
        )
    }
    async fn save_checkpoint_evidence(&mut self, journal: &mut Journal) -> Result<bool, Error> {
        let evidence = self.record.clone();
        for _ in 0..16 {
            self.reload(journal)?;
            let proceed = !self.stopped_after_recovery();
            if self.record.action != evidence.action
                || self.record.pool != evidence.pool
                || self.record.task != evidence.task
            {
                return Err(Error::TransferConflict);
            }
            let incoming = evidence
                .transfer_plan
                .as_ref()
                .ok_or(Error::TransferConflict)?;
            let current = self.transfer_plan_mut()?;
            if current.file != incoming.file
                || current.baseline != incoming.baseline
                || current.task_uuid != incoming.task_uuid
            {
                return Err(Error::TransferConflict);
            }
            if let Some(result) = &incoming.result {
                if current.result.as_ref().is_some_and(|old| old != result) {
                    return Err(Error::TransferConflict);
                }
                current.result = Some(result.clone());
            }
            current.task_success |= incoming.task_success;
            current.task_failed |= incoming.task_failed;
            current.task_cancelled |= incoming.task_cancelled;
            let out = &mut self.record.transfer_created;
            macro_rules! retain {
                ($field:ident) => {
                    for value in &evidence.transfer_created.$field {
                        if !out.$field.contains(value) {
                            out.$field.push(*value);
                        }
                    }
                };
            }
            retain!(vms);
            retain!(disks);
            retain!(networks);
            retain!(suspend_images);
            if proceed {
                self.record.state = evidence.state.clone();
                self.record.progress = evidence.progress;
                self.record.can_cancel = evidence.can_cancel;
                self.record.detail = evidence.detail.clone();
                self.record.steps = evidence.steps.clone();
            }
            match self.save(journal).await {
                Err(Error::JournalChanged) => continue,
                Err(error) => return Err(error),
                Ok(()) => return Ok(proceed),
            }
        }
        Err(Error::JournalChanged)
    }
    async fn save_checkpoint_stream(
        &mut self,
        journal: &mut Journal,
        outcome: Result<files::Receipt, Error>,
        mut progress: crate::TransferProgress,
    ) -> Result<bool, Error> {
        progress.stream_complete = outcome.is_ok();
        progress.phase = "Checkpoint stream stopped; checking existing evidence".into();
        for _ in 0..16 {
            self.reload(journal)?;
            let plan = self.transfer_plan_mut()?;
            match &outcome {
                Ok(receipt) => {
                    if plan
                        .stream_receipt
                        .as_ref()
                        .is_some_and(|old| old != receipt)
                    {
                        return Err(Error::TransferConflict);
                    }
                    plan.stream_receipt = Some(receipt.clone());
                }
                Err(error) => {
                    plan.stream_invalid |= matches!(
                        error,
                        Error::TransferArchive | Error::TransferFileChanged | Error::TransferLimit
                    )
                }
            }
            self.record.transfer = Some(progress.clone());
            match self.save(journal).await {
                Err(Error::JournalChanged) => continue,
                Err(e) => return Err(e),
                Ok(()) => return Ok(!self.stopped_after_recovery()),
            }
        }
        Err(Error::JournalChanged)
    }
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
        } else if let ResourceAction::CloudInit(config) = &self.record.action {
            Ok(config.transfer_action())
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
        if matches!(action, TransferAction::ExportOvf { .. }) {
            return self.execute_ovf_export(journal, session, cancel).await;
        }
        if matches!(action, TransferAction::ImportOvf { .. }) {
            return self.execute_ovf_import(journal, session, cancel).await;
        }
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
        if let TransferAction::Host {
            operation: crate::HostArchiveOperation::StageRestore { archive },
            ..
        } = &action
        {
            let file = self.transfer_plan()?.file.clone();
            let archive = archive.clone();
            let validation_cancel = cancel.clone();
            tokio::task::spawn_blocking(move || {
                crate::host_support::restore::revalidate(&file, &archive, &validation_cancel)
            })
            .await
            .map_err(|_| Error::TransferFile)??;
        }
        if matches!(action, TransferAction::ImportCheckpointXva { .. }) {
            let file = self.transfer_plan()?.file.clone();
            let validation_cancel = cancel.clone();
            tokio::task::spawn_blocking(move || {
                files::revalidate_checkpoint_source(&file, &validation_cancel)
            })
            .await
            .map_err(|_| Error::TransferFile)??;
        }
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
            if matches!(
                action,
                TransferAction::ImportXva { .. } | TransferAction::ImportCheckpointXva { .. }
            ) {
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
                label: if matches!(action, TransferAction::Host { .. }) {
                    "Transfer archive"
                } else {
                    "Transfer image"
                }
                .into(),
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
        if self.stopped_after_recovery() {
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
        let outcome = http::drive(
            session,
            prepared,
            self.transfer_plan()?.clone(),
            meter.clone(),
            cancel,
            |progress| {
                self.reload(journal)?;
                if self.record.state.terminal() {
                    return Ok(false);
                }
                self.record.transfer = Some(progress);
                self.record.detail = if self.record.cancel_requested {
                    "Stopping the local stream; server cancellation still requires confirmation"
                } else {
                    "Streaming image"
                }
                .into();
                match self.save_transfer_progress(journal) {
                    Ok(()) | Err(Error::JournalChanged) => {}
                    Err(error) => return Err(error),
                }
                Ok(!self.record.cancel_requested
                    && !((action.is_host_restore() || self.checkpoint_transfer())
                        && self.stopped_after_recovery()))
            },
        )
        .await;
        if matches!(self.record.action, ResourceAction::CloudInit(_)) {
            if self
                .save_cloud_stream_evidence(journal, outcome, meter.progress())
                .await?
            {
                return self.recheck_transfer(journal, session, cancel).await;
            }
            return Ok(());
        }
        if action.is_host_restore() {
            if self
                .save_host_restore_stream(journal, outcome, meter.progress())
                .await?
            {
                return self.recheck_transfer(journal, session, cancel).await;
            }
            return Ok(());
        }
        if self.checkpoint_transfer() {
            if self
                .save_checkpoint_stream(journal, outcome, meter.progress())
                .await?
            {
                return self.recheck_transfer(journal, session, cancel).await;
            }
            return Ok(());
        }
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
        if let TransferAction::ImportXva { sr } | TransferAction::ImportCheckpointXva { sr } =
            self.transfer_action()?
        {
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
            for v in found.suspend_images {
                if !old.suspend_images.contains(&v) {
                    old.suspend_images.push(v);
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
        if matches!(self.transfer_action()?, TransferAction::ExportOvf { .. }) {
            return self.recheck_ovf_export(journal, session, cancel).await;
        }
        if matches!(self.transfer_action()?, TransferAction::ImportOvf { .. }) {
            return self.recheck_ovf_import(journal, session, cancel).await;
        }
        self.reload(journal)?;
        if self.record.state.terminal()
            || self.checkpoint_transfer() && self.stopped_after_recovery()
        {
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
            if self.record.state.terminal()
                || self.checkpoint_transfer() && self.stopped_after_recovery()
            {
                return Ok(());
            }
            let known = self.transfer_plan()?;
            if known.task_success || known.task_failed || known.task_cancelled {
                break;
            }
            let value = tokio::select! {_=cancel.cancelled()=>return Err(Error::Closed),result=session.call("task.get_record",vec![json!(&task)])=>result?};
            self.reload(journal)?;
            if self.record.state.terminal() && !self.checkpoint_transfer() {
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
                    let imported = !self.transfer_action()?.is_export()
                        && !self.transfer_action()?.is_host_restore();
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
            if self.checkpoint_transfer() {
                if !self.save_checkpoint_evidence(journal).await? {
                    return Ok(());
                }
            } else {
                self.save(journal).await?;
            }
            let p = self.transfer_plan()?;
            if p.task_success || p.task_failed || p.task_cancelled {
                break;
            }
            if self.record.cancel_requested
                && self.record.can_cancel
                && !self.record.cancel_dispatched
                && (!self.checkpoint_transfer() || !self.reconcile && !self.record.interrupted)
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
                return if self.checkpoint_transfer() {
                    self.save_checkpoint_evidence(journal).await.map(|_| ())
                } else {
                    self.save(journal).await
                };
            }
            tokio::select! {_=cancel.cancelled()=>return Err(Error::Closed),_=tokio::time::sleep(Duration::from_secs(1))=>{}}
        }
        self.record.state = OperationState::Verifying;
        self.record.can_cancel = false;
        self.save(journal).await?;
        let success = self.transfer_plan()?.task_success;
        self.collect_transfer_created(session, success).await?;
        if self.checkpoint_transfer() {
            if !self.save_checkpoint_evidence(journal).await? {
                return Ok(());
            }
        } else {
            self.save(journal).await?;
        }
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
                let validation_cancel = cancel.clone();
                let publication = tokio::task::spawn_blocking(move || {
                    match action {
                        TransferAction::Host {
                            host,
                            expected_version,
                            operation,
                        } => files::validate_host_export(
                            &plan,
                            host,
                            &expected_version,
                            &operation,
                            &validation_cancel,
                        )?,
                        TransferAction::ExportXva { source } => {
                            files::validate_export(&plan, &source, &validation_cancel)?
                        }
                        TransferAction::ExportDisk {
                            format,
                            expected_size,
                            ..
                        } => files::validate_disk_export(&plan, format, expected_size)?,
                        _ => return Err(Error::TransferConflict),
                    }
                    files::prepare_publication(&plan)
                })
                .await
                .map_err(|_| Error::TransferFile)??;
                journal.transfer_publication_guard(&self.record, self.reconcile, cancel, || {
                    publication.publish()
                })?;
                self.record.state = OperationState::Succeeded;
                self.record.detail =
                    if matches!(self.transfer_action()?, TransferAction::Host { .. }) {
                        "Host task and archive verified; private archive saved locally"
                    } else {
                        "Export verified and saved as a private image"
                    }
                    .into();
            }
        } else if success && self.transfer_action()?.is_host_restore() {
            self.record.state = if self.transfer_plan()?.stream_receipt.is_some()
                && !self.transfer_plan()?.stream_invalid
            {
                OperationState::Succeeded
            } else {
                OperationState::OutcomeUnknown
            };
            self.record.detail=if self.record.state==OperationState::Succeeded {"Host task confirmed staging the reviewed archive on its configured backup partition. Running configuration was not restored; installer-assisted recovery and pool database restoration remain separate operator actions."}else{"Host staging task completed, but complete unchanged input evidence is unavailable. Inspect the backup partition; upload will not be repeated."}.into();
        } else if success && matches!(self.record.action, ResourceAction::CloudInit(_)) {
            return self.complete_cloud_upload(journal, session, cancel).await;
        } else if success {
            self.record.state = if self.transfer_plan()?.stream_invalid
                || self.checkpoint_transfer() && self.transfer_plan()?.stream_receipt.is_none()
            {
                OperationState::PartiallyCompleted
            } else {
                OperationState::Succeeded
            };
            self.record.detail = if matches!(self.transfer_action()?,TransferAction::ImportDisk{..})&&!self.transfer_plan()?.stream_invalid {"Disk image imported into a new verified unattached disk"}else if self.transfer_plan()?.stream_invalid {"Host completed import, but source file integrity changed during upload. Retained resource UUIDs require inspection."}else if self.checkpoint_transfer() && self.transfer_plan()?.stream_receipt.is_none(){"Host completed import, but a complete local upload receipt is unavailable. Retained suspended guest and memory image UUIDs require inspection."}else if self.checkpoint_transfer(){"Imported guest and memory image identities verified; memory guests remain suspended. Resume is a separate action requiring compatible CPU and host support."}else{"Imported guest identities and storage verified; guests remain halted"}.into();
        } else if matches!(self.transfer_action()?, TransferAction::ImportDisk { .. }) {
            self.record.state = OperationState::PartiallyCompleted;
            self.record.detail="Disk transfer did not complete. The new unattached disk is retained with partial or unknown contents; no existing disk was overwritten.".into();
        } else if self.transfer_action()?.is_host_restore() {
            self.record.state = OperationState::OutcomeUnknown;
            self.record.detail="Host recovery staging did not complete successfully. The configured backup partition may have been reformatted or partially populated. Inspect it before acknowledging; upload is never repeated.".into();
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
        if self.record.state.terminal()
            || self.checkpoint_transfer() && self.stopped_after_recovery()
        {
            self.cleanup_transfer_task(session).await;
        }
        if self.transfer_action()?.is_export() && self.record.state != OperationState::Succeeded {
            files::cleanup(self.transfer_plan()?)
        }
        Ok(())
    }
}
