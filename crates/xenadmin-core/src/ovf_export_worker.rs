use super::*;
use crate::{
    OvfPackageFormat, TransferAction,
    ovf::{
        export::{self, ExportPlan, Step},
        output,
    },
    transfer::{self, Plan, files, http},
};
use std::sync::atomic::{AtomicBool, Ordering};
use uuid::Uuid;
struct StopOnDrop(Arc<AtomicBool>);
impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
impl ResourceRequest {
    fn ovf_export(&self) -> Result<&ExportPlan, Error> {
        self.transfer_plan()?
            .ovf_export
            .as_deref()
            .ok_or(Error::TransferConflict)
    }
    fn ovf_export_mut(&mut self) -> Result<&mut ExportPlan, Error> {
        self.transfer_plan_mut()?
            .ovf_export
            .as_deref_mut()
            .ok_or(Error::TransferConflict)
    }
    fn export_action(&self) -> Result<(crate::ExportSource, OvfPackageFormat), Error> {
        if let TransferAction::ExportOvf { source, format } = self.transfer_action()? {
            Ok((source, format))
        } else {
            Err(Error::TransferConflict)
        }
    }
    async fn verify_export_capture(&self, s: &Session) -> Result<(), Error> {
        let (source, _) = self.export_action()?;
        if export::capture(s, self.record.pool, &source).await? != self.ovf_export()?.captured {
            return Err(Error::TransferConflict);
        }
        Ok(())
    }
    pub(super) async fn execute_ovf_export(
        &mut self,
        j: &mut Journal,
        s: &Session,
        c: &CancellationToken,
    ) -> Result<(), Error> {
        let (source, format) = self.export_action()?;
        let captured = export::capture(s, self.record.pool, &source).await?;
        let baseline = crate::device_lifecycle::baseline(s, "task").await?;
        let file = self.transfer_plan()?.file.clone();
        let id = self.record.id;
        let mut plan = self.transfer_plan()?.clone();
        let expires = now().saturating_add(24 * 60 * 60 * 1000);
        plan.expires_at_ms = expires;
        plan = tokio::task::spawn_blocking(move || {
            let directory = output::create(&file, id)?;
            let mut steps = Vec::new();
            for i in 0..captured.disks.len() {
                let token = OperationId::new();
                let mut stream = Plan::new(crate::TransferFile::destination(
                    directory.path.join(format!("disk-{i}.vhd")),
                )?);
                stream.expires_at_ms = expires;
                files::create_output(&mut stream, token)?;
                steps.push(Step {
                    token,
                    task: None,
                    task_dispatched: false,
                    cancel_dispatched: false,
                    stream,
                    complete: false,
                });
            }
            plan.ovf_export = Some(Box::new(ExportPlan {
                captured,
                directory,
                task_baseline: baseline,
                steps,
                published: false,
            }));
            if format == OvfPackageFormat::Ova {
                files::create_output(&mut plan, id)?;
            }
            Ok::<_, Error>(plan)
        })
        .await
        .map_err(|_| Error::TransferFile)??;
        self.reload(j)?;
        if self.stop_before_dispatch() {
            output::cleanup(&plan);
            files::cleanup(&plan);
            return self.save(j).await;
        }
        self.record.steps = plan
            .ovf_export
            .as_ref()
            .ok_or(Error::TransferConflict)?
            .steps
            .iter()
            .enumerate()
            .map(|(i, _)| OperationStep {
                label: format!("Export disk {}", i + 1),
                state: StepState::Pending,
                detail: String::new(),
            })
            .chain(std::iter::once(OperationStep {
                label: "Verify and publish package".into(),
                state: StepState::Pending,
                detail: String::new(),
            }))
            .collect();
        self.record.transfer_plan = Some(Box::new(plan));
        self.save(j).await?;
        for index in 0..self.ovf_export()?.steps.len() {
            self.reload(j)?;
            if self.stop_before_dispatch() {
                self.save(j).await?;
                return Ok(());
            }
            self.verify_export_capture(s).await?;
            self.export_ovf_disk(j, s, c, index).await?;
            if self.record.state.terminal() || self.record.state == OperationState::OutcomeUnknown {
                return Ok(());
            }
        }
        self.finish_ovf_export(j, s, c).await
    }
    async fn export_ovf_disk(
        &mut self,
        j: &mut Journal,
        s: &Session,
        c: &CancellationToken,
        index: usize,
    ) -> Result<(), Error> {
        self.reload(j)?;
        if self.stop_before_dispatch() {
            return self.save(j).await;
        }
        let st = &mut self.ovf_export_mut()?.steps[index];
        if st.task_dispatched {
            return Err(Error::TransferConflict);
        }
        st.task_dispatched = true;
        let name = format!("XenAdmin RS OVF export {}", st.token);
        self.record.state = OperationState::Tracking;
        self.record.detail = "Creating an owned disk export task".into();
        self.record.steps[index].state = StepState::Dispatching;
        self.save_dispatch(j)?;
        let task = tokio::select! {_=c.cancelled()=>return Err(Error::Closed),v=s.call("task.create",vec![json!(name),json!("Client-owned OVF disk export")])=>crate::resources::xapi::reply_reference(&v?)?};
        self.export_task_evidence(j, index, &task, None).await?;
        let v = s.call("task.get_record", vec![json!(task)]).await?;
        let id = v["uuid"]
            .as_str()
            .and_then(|v| v.parse::<Uuid>().ok())
            .ok_or(Error::TransferConflict)?;
        if v["name_label"] != name
            || v["status"] != "pending"
            || self.ovf_export()?.task_baseline.contains(&id)
            || self
                .ovf_export()?
                .steps
                .iter()
                .enumerate()
                .any(|(i, st)| i != index && st.stream.task_uuid == Some(id))
        {
            return Err(Error::TransferConflict);
        }
        self.export_task_evidence(j, index, &task, Some(id)).await?;
        self.reload(j)?;
        if self.stop_before_dispatch() {
            self.save(j).await?;
            self.export_cleanup_task(j, s, index).await;
            return Ok(());
        }
        if self.record.state == OperationState::OutcomeUnknown {
            return Ok(());
        }
        let source = &self.ovf_export()?.captured.disks[index];
        let action = TransferAction::ExportDisk {
            vdi: source.id,
            expected_sr: source.sr,
            expected_size: source.size,
            format: crate::DiskImageFormat::Vhd,
        };
        let mut route = transfer::disk::prepare(s, self.record.pool, &action, None).await?;
        route.query.push(("task_id".into(), task));
        self.reload(j)?;
        if self.stop_before_dispatch() {
            self.save(j).await?;
            self.export_cleanup_task(j, s, index).await;
            return Ok(());
        }
        if self.record.state == OperationState::OutcomeUnknown {
            return Ok(());
        }
        self.ovf_export_mut()?.steps[index].stream.dispatched = true;
        self.record.can_cancel = true;
        self.record.detail = "Streaming a verified source disk into the private package".into();
        self.save_dispatch(j)?;
        let stream = self.ovf_export()?.steps[index].stream.clone();
        let meter = Arc::new(http::Meter::default());
        let result =
            http::drive(s, route, stream, meter.clone(), c, |progress| {
                self.reload(j)?;
                if self.record.state.terminal() || self.stopped_after_recovery() {
                    return Ok(false);
                }
                self.record.transfer = Some(progress);
                match self.save_transfer_progress(j) {
                    Ok(()) | Err(Error::JournalChanged) => {}
                    Err(e) => return Err(e),
                }
                Ok(!self.record.cancel_requested
                    && self.record.state != OperationState::OutcomeUnknown)
            })
            .await;
        let mut progress = meter.progress();
        if let Ok(receipt) = result {
            if !self.export_stream_evidence(j, index, receipt).await? {
                return Ok(());
            }
            progress.stream_complete = true;
        }
        self.reload(j)?;
        if self.record.state.terminal() || self.stopped_after_recovery() {
            return Ok(());
        }
        progress.phase = "Verifying disk export task".into();
        self.record.transfer = Some(progress);
        self.save(j).await?;
        self.check_ovf_export_disk(j, s, c, index).await
    }
    async fn export_stream_evidence(
        &mut self,
        j: &mut Journal,
        index: usize,
        receipt: files::Receipt,
    ) -> Result<bool, Error> {
        for _ in 0..16 {
            self.reload(j)?;
            let stream = &mut self
                .ovf_export_mut()?
                .steps
                .get_mut(index)
                .ok_or(Error::Journal)?
                .stream;
            if stream
                .stream_receipt
                .as_ref()
                .is_some_and(|old| old != &receipt)
            {
                return Err(Error::TransferConflict);
            }
            stream.stream_receipt = Some(receipt.clone());
            match self.save(j).await {
                Err(Error::JournalChanged) => continue,
                Err(error) => return Err(error),
                Ok(()) => {
                    return Ok(!self.record.state.terminal() && !self.stopped_after_recovery());
                }
            }
        }
        Err(Error::JournalChanged)
    }
    async fn export_task_evidence(
        &mut self,
        j: &mut Journal,
        index: usize,
        task: &str,
        id: Option<Uuid>,
    ) -> Result<(), Error> {
        for _ in 0..16 {
            self.reload(j)?;
            let st = &mut self.ovf_export_mut()?.steps[index];
            if st.task.as_deref().is_some_and(|r| r != task)
                || id.is_some_and(|id| st.stream.task_uuid.is_some_and(|old| old != id))
            {
                return Err(Error::TransferConflict);
            }
            st.task = Some(task.into());
            if let Some(id) = id {
                st.stream.task_uuid = Some(id)
            }
            match self.save(j).await {
                Err(Error::JournalChanged) => continue,
                result => return result,
            }
        }
        Err(Error::JournalChanged)
    }
    async fn export_outcome_evidence(
        &mut self,
        j: &mut Journal,
        index: usize,
        task: Uuid,
        status: &str,
    ) -> Result<bool, Error> {
        for _ in 0..16 {
            self.reload(j)?;
            let stream = &mut self
                .ovf_export_mut()?
                .steps
                .get_mut(index)
                .ok_or(Error::Journal)?
                .stream;
            if stream.task_uuid != Some(task) {
                return Err(Error::TransferConflict);
            }
            match status {
                "success" => stream.task_success = true,
                "failure" => stream.task_failed = true,
                "cancelled" => stream.task_cancelled = true,
                "pending" | "cancelling" => {}
                _ => return Err(Error::TransferConflict),
            }
            match self.save(j).await {
                Err(Error::JournalChanged) => continue,
                Err(error) => return Err(error),
                Ok(()) => {
                    return Ok(!self.record.state.terminal() && !self.stopped_after_recovery());
                }
            }
        }
        Err(Error::JournalChanged)
    }
    async fn check_ovf_export_disk(
        &mut self,
        j: &mut Journal,
        s: &Session,
        c: &CancellationToken,
        index: usize,
    ) -> Result<(), Error> {
        loop {
            self.reload(j)?;
            if self.record.state.terminal() || self.stopped_after_recovery() {
                return Ok(());
            }
            let st = &self.ovf_export()?.steps[index];
            if st.complete {
                return Ok(());
            }
            if st.stream.task_success || st.stream.task_failed || st.stream.task_cancelled {
                break;
            }
            if !st.stream.dispatched {
                return self.export_partial(j,"Export stopped before downloading all disks; no complete package was published").await;
            }
            let task = st.task.clone().ok_or(Error::TransferConflict)?;
            let id = st.stream.task_uuid.ok_or(Error::TransferConflict)?;
            let value = tokio::select! {_=c.cancelled()=>return Err(Error::Closed),v=s.call("task.get_record",vec![json!(task)])=>v?};
            self.reload(j)?;
            if value["uuid"].as_str().and_then(|v| v.parse::<Uuid>().ok()) != Some(id) {
                return Err(Error::TransferConflict);
            }
            let status = value["status"].as_str().ok_or(Error::TransferConflict)?;
            if !self.export_outcome_evidence(j, index, id, status).await? {
                return Ok(());
            }
            let st = &self.ovf_export()?.steps[index];
            if st.stream.task_success || st.stream.task_failed || st.stream.task_cancelled {
                break;
            }
            if self.record.cancel_requested
                && self.record.batch_parent.is_none()
                && !st.cancel_dispatched
                && value["allowed_operations"]
                    .as_array()
                    .is_some_and(|a| a.iter().any(|v| v == "cancel"))
            {
                self.ovf_export_mut()?.steps[index].cancel_dispatched = true;
                self.save(j).await?;
                tokio::select! {_=c.cancelled()=>return Err(Error::Closed),v=s.call("task.cancel",vec![json!(task)])=>{v?;}}
            }
            if self.reconcile
                || self.ovf_export()?.steps[index]
                    .stream
                    .stream_receipt
                    .is_none()
            {
                self.record.state = OperationState::OutcomeUnknown;
                self.record.detail="Disk export stream stopped while its host task is pending. Recheck reads task evidence without repeating the download.".into();
                return self.save(j).await;
            }
            if now() >= self.transfer_plan()?.expires_at_ms {
                return Err(Error::TransferExpired);
            }
            tokio::select! {_=c.cancelled()=>return Err(Error::Closed),_=tokio::time::sleep(Duration::from_secs(1))=>{}}
        }
        let st = &self.ovf_export()?.steps[index];
        if !st.stream.task_success || st.stream.stream_receipt.is_none() {
            self.export_partial(j,"Host export ended without a complete verified local disk; no final package was published").await?;
            self.export_cleanup_task(j, s, index).await;
            return Ok(());
        }
        self.verify_export_capture(s).await?;
        let stream = st.stream.clone();
        let size = self.ovf_export()?.captured.disks[index].size;
        let publication = tokio::task::spawn_blocking(move || {
            files::validate_disk_export(&stream, crate::DiskImageFormat::Vhd, size)?;
            files::prepare_publication(&stream)
        })
        .await
        .map_err(|_| Error::TransferFile)??;
        self.reload(j)?;
        if self.record.state.terminal() || self.stopped_after_recovery() {
            return Ok(());
        }
        self.record.state = OperationState::Verifying;
        self.save(j).await?;
        if self.reconcile && self.record.batch_parent.is_some() {
            publication.verify_existing()?;
        } else {
            j.ovf_publication_guard(&self.record, self.reconcile, Some(index), c, || {
                publication.publish()
            })?;
        }
        self.reload(j)?;
        if self.record.state.terminal() || self.stopped_after_recovery() {
            return Ok(());
        }
        self.ovf_export_mut()?.steps[index].complete = true;
        self.record.steps[index].state = StepState::Verified;
        if self.record.state != OperationState::OutcomeUnknown {
            self.record.state = OperationState::Preparing;
        }
        self.save(j).await?;
        self.export_cleanup_task(j, s, index).await;
        Ok(())
    }
    async fn export_partial(&mut self, j: &mut Journal, message: &str) -> Result<(), Error> {
        self.reload(j)?;
        if self.record.state.terminal() || self.stopped_after_recovery() {
            return Ok(());
        }
        self.record.state = OperationState::PartiallyCompleted;
        self.record.can_cancel = false;
        self.record.detail = message.into();
        self.save(j).await?;
        if self.record.batch_parent.is_some() && self.reconcile {
            return Ok(());
        }
        let plan = self.transfer_plan()?.clone();
        tokio::task::spawn_blocking(move || {
            output::cleanup(&plan);
            files::cleanup(&plan);
        })
        .await
        .map_err(|_| Error::TransferFile)?;
        Ok(())
    }
    async fn export_cleanup_task(&self, j: &mut Journal, s: &Session, index: usize) {
        if self.record.batch_parent.is_some() && (self.reconcile || self.record.interrupted) {
            return;
        }
        if let Ok(p) = self.ovf_export()
            && let Some(st) = p.steps.get(index)
            && let Some(task) = &st.task
            && let Some(id) = st.stream.task_uuid
            && let Ok(Ok(v)) = tokio::time::timeout(
                Duration::from_secs(5),
                s.call("task.get_record", vec![json!(task)]),
            )
            .await
            && v["uuid"].as_str().and_then(|v| v.parse::<Uuid>().ok()) == Some(id)
        {
            if j.workflow_child_stopped(self.record.id).unwrap_or(true) {
                return;
            }
            let _ = tokio::time::timeout(
                Duration::from_secs(5),
                s.call("task.destroy", vec![json!(task)]),
            )
            .await;
        }
    }
    async fn publish_ova_receipt(
        &mut self,
        j: &mut Journal,
        receipt: files::Receipt,
        c: &CancellationToken,
    ) -> Result<bool, Error> {
        let mut saved = false;
        for _ in 0..16 {
            self.reload(j)?;
            self.transfer_plan_mut()?.stream_receipt = Some(receipt.clone());
            match self.save(j).await {
                Err(Error::JournalChanged) => continue,
                result => {
                    result?;
                    saved = true;
                    break;
                }
            }
        }
        if !saved {
            return Err(Error::JournalChanged);
        }
        // Keep late evidence, but acknowledgement or a stop revokes permission to publish it.
        self.reload(j)?;
        if self.record.state.terminal()
            || self.record.state == OperationState::OutcomeUnknown
            || self.record.cancel_requested
            || c.is_cancelled()
            || j.workflow_child_stopped(self.record.id)?
        {
            return Ok(false);
        }
        let plan = self.transfer_plan()?.clone();
        let publication = tokio::task::spawn_blocking(move || files::prepare_publication(&plan))
            .await
            .map_err(|_| Error::TransferFile)??;
        match j.ovf_publication_guard(&self.record, self.reconcile, None, c, || {
            publication.publish()
        }) {
            Err(Error::Closed) => Ok(false),
            result => {
                result?;
                Ok(true)
            }
        }
    }
    async fn finish_ovf_export(
        &mut self,
        j: &mut Journal,
        s: &Session,
        c: &CancellationToken,
    ) -> Result<(), Error> {
        if self.ovf_export()?.steps.iter().any(|st| !st.complete) {
            return Err(Error::TransferConflict);
        }
        self.verify_export_capture(s).await?;
        let (_, format) = self.export_action()?;
        self.reload(j)?;
        if self.record.state.terminal() || self.stopped_after_recovery() {
            return Ok(());
        }
        if !self.ovf_export()?.published {
            if self.record.cancel_requested {
                return self
                    .export_partial(
                        j,
                        "Export stopped before publishing the completed private package",
                    )
                    .await;
            }
            self.record.state = OperationState::Verifying;
            self.record.detail = "Checking package manifest and publishing completed output".into();
            self.save(j).await?;
            if format == OvfPackageFormat::Ova && self.transfer_plan()?.stream_receipt.is_some() {
                let receipt = self
                    .transfer_plan()?
                    .stream_receipt
                    .clone()
                    .ok_or(Error::TransferFile)?;
                if !self.publish_ova_receipt(j, receipt, c).await? {
                    return self
                        .export_partial(j, "Export stopped before final package publication")
                        .await;
                }
            } else {
                let plan = self.transfer_plan()?.clone();
                let stop = Arc::new(AtomicBool::new(false));
                let _stop_on_drop = StopOnDrop(stop.clone());
                let worker_stop = stop.clone();
                let mut job = tokio::task::spawn_blocking(move || {
                    output::publish(&plan, format, &worker_stop)
                });
                let result = loop {
                    tokio::select! {r=&mut job=>break r.map_err(|_|Error::TransferFile)?,_=c.cancelled()=>{stop.store(true,Ordering::Relaxed);break job.await.map_err(|_|Error::TransferFile)?},_=tokio::time::sleep(Duration::from_millis(250))=>{self.reload(j)?;if self.record.cancel_requested||self.record.state.terminal(){stop.store(true,Ordering::Relaxed);}}}
                };
                let receipt = result?;
                self.reload(j)?;
                if let Some(receipt) = receipt
                    && !self.publish_ova_receipt(j, receipt, c).await?
                {
                    return self
                        .export_partial(j, "Export stopped before final package publication")
                        .await;
                }
                if format == OvfPackageFormat::Ovf {
                    let plan = self.transfer_plan()?.clone();
                    j.ovf_publication_guard(&self.record, self.reconcile, None, c, || {
                        output::finalize_directory(&plan)
                    })?;
                }
            }
            self.reload(j)?;
            self.ovf_export_mut()?.published = true;
            self.save(j).await?;
        }
        if !self.record.state.terminal() {
            self.record.state = OperationState::Succeeded;
            self.record.progress = Some(1.0);
            self.record.can_cancel = false;
            self.record.detail="OVF package, disk formats and manifest verified; saved privately without overwriting existing files".into();
            for st in &mut self.record.steps {
                st.state = StepState::Verified;
            }
            self.save(j).await?;
        }
        if format == OvfPackageFormat::Ova {
            let plan = self.transfer_plan()?.clone();
            let _ = tokio::task::spawn_blocking(move || output::cleanup(&plan)).await;
        }
        Ok(())
    }
    pub(super) async fn recheck_ovf_export(
        &mut self,
        j: &mut Journal,
        s: &Session,
        c: &CancellationToken,
    ) -> Result<(), Error> {
        self.reload(j)?;
        if self.record.state.terminal() || self.stopped_after_recovery() {
            return Ok(());
        }
        for index in 0..self.ovf_export()?.steps.len() {
            if !self.ovf_export()?.steps[index].complete {
                self.check_ovf_export_disk(j, s, c, index).await?;
                if self.record.state.terminal() || !self.ovf_export()?.steps[index].complete {
                    return Ok(());
                }
            }
        }
        if self.record.batch_parent.is_some() {
            let (_, format) = self.export_action()?;
            let plan = self.transfer_plan()?.clone();
            tokio::task::spawn_blocking(move || {
                output::verify_published(&plan, format, &AtomicBool::new(false))
            })
            .await
            .map_err(|_| Error::TransferFile)??;
            self.reload(j)?;
            if self.record.state.terminal() || self.stopped_after_recovery() {
                return Ok(());
            }
            self.ovf_export_mut()?.published = true;
            self.record.state = OperationState::Succeeded;
            self.record.detail="Previously published package identity and contents verified; no file was published during recheck".into();
            self.record.can_cancel = false;
            for step in &mut self.record.steps {
                step.state = StepState::Verified;
            }
            return self.save(j).await;
        }
        self.finish_ovf_export(j, s, c).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ObservedResource, ResourceTarget, TransferFile, TransferRequest};
    use sha2::{Digest, Sha256};
    use std::io::Write;
    #[tokio::test]
    async fn late_package_receipt_keeps_proof_without_publishing_after_a_stop() {
        for mode in 0..5 {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("package.ova");
            let pool = Uuid::from_u128(1).into();
            let id = OperationId::new();
            let (_, mut request) = crate::resources::transfer_channel(
                id,
                ObservedResource {
                    pool,
                    generation: 1,
                    target: ResourceTarget::Pool(pool),
                },
                TransferRequest {
                    action: TransferAction::ExportOvf {
                        source: crate::ExportSource::Vm {
                            vm: Uuid::from_u128(2).into(),
                            expected_template: false,
                        },
                        format: OvfPackageFormat::Ova,
                    },
                    file: TransferFile::destination(&path).unwrap(),
                },
                tokio::runtime::Handle::current(),
            );
            let mut journal = Journal::open(directory.path()).unwrap();
            journal.resource_admit(&mut request.record).unwrap();
            files::create_output(request.transfer_plan_mut().unwrap(), id).unwrap();
            let directory = output::create(&request.transfer_plan().unwrap().file, id).unwrap();
            request.transfer_plan_mut().unwrap().ovf_export = Some(Box::new(ExportPlan {
                captured: export::Captured {
                    summary: crate::OvfSummary {
                        guests: vec![],
                        disks: vec![],
                        networks: vec![],
                        manifest_verified: true,
                    },
                    disks: vec![],
                },
                directory,
                task_baseline: vec![],
                steps: vec![],
                published: false,
            }));
            request.record.state = OperationState::Verifying;
            request.save(&mut journal).await.unwrap();
            let mut output = files::output(request.transfer_plan().unwrap()).unwrap();
            let bytes = vec![42; 512];
            output.write_all(&bytes).unwrap();
            output.sync_all().unwrap();
            let receipt = files::receipt(&output, 512, Sha256::digest(&bytes).into()).unwrap();
            let token = CancellationToken::new();
            if mode == 0 {
                journal
                    .resource_control(pool, id, crate::OperationControl::RequestCancellation)
                    .unwrap();
            }
            if mode == 1 {
                journal.resource_recover(pool).unwrap();
                journal
                    .resource_control(pool, id, crate::OperationControl::AcknowledgeUnknown)
                    .unwrap();
            }
            if mode == 2 {
                token.cancel();
            }
            if mode == 4 {
                journal.resource_recover(pool).unwrap();
            }
            let published = request
                .publish_ova_receipt(&mut journal, receipt.clone(), &token)
                .await
                .unwrap();
            assert_eq!(published, mode == 3);
            assert_eq!(path.exists(), mode == 3);
            let saved = journal.resource_load(id).unwrap().unwrap();
            assert_eq!(saved.transfer_plan.unwrap().stream_receipt, Some(receipt));
            if mode == 1 {
                assert_eq!(saved.state, OperationState::AcknowledgedUnknown);
            }
        }
    }
}
