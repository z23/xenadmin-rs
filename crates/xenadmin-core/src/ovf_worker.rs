use super::*;
use crate::{
    TransferAction,
    ovf::import::{self, ImportPlan, Kind, Step},
    transfer::{Plan, files, http},
};
use uuid::Uuid;
use xenadmin_xapi::records::TaskRecord;
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
impl ResourceRequest {
    fn ovf_plan(&self) -> Result<&ImportPlan, Error> {
        self.transfer_plan()?
            .ovf_import
            .as_deref()
            .ok_or(Error::TransferConflict)
    }
    fn ovf_plan_mut(&mut self) -> Result<&mut ImportPlan, Error> {
        self.transfer_plan_mut()?
            .ovf_import
            .as_deref_mut()
            .ok_or(Error::TransferConflict)
    }
    fn ovf_step(&self, index: usize) -> Result<&Step, Error> {
        self.ovf_plan()?
            .steps
            .get(index)
            .ok_or(Error::TransferConflict)
    }
    fn ovf_step_mut(&mut self, index: usize) -> Result<&mut Step, Error> {
        self.ovf_plan_mut()?
            .steps
            .get_mut(index)
            .ok_or(Error::TransferConflict)
    }
    fn ovf_active(&self) -> bool {
        !self.record.state.terminal()
            && self.record.state != OperationState::OutcomeUnknown
            && !self.record.cancel_requested
    }
    async fn ovf_inputs(&self) -> Result<(), Error> {
        let file = self.transfer_plan()?.file.clone();
        tokio::task::spawn_blocking(move || {
            files::source(&file)?;
            for source in file.ovf.as_ref().ok_or(Error::TransferFile)?.files.values() {
                files::source(source)?;
            }
            Ok::<_, Error>(())
        })
        .await
        .map_err(|_| Error::TransferFile)?
    }
    pub(super) async fn execute_ovf_import(
        &mut self,
        j: &mut Journal,
        s: &Session,
        c: &CancellationToken,
    ) -> Result<(), Error> {
        let TransferAction::ImportOvf { sr, networks } = self.transfer_action()? else {
            return Err(Error::TransferConflict);
        };
        self.ovf_inputs().await?;
        let plan = import::prepare(
            s,
            self.record.pool,
            &self.transfer_plan()?.file,
            sr,
            &networks,
        )
        .await?;
        self.reload(j)?;
        if self.stop_before_dispatch() {
            return self.save(j).await;
        }
        self.record.steps = plan
            .steps
            .iter()
            .map(|st| OperationStep {
                label: st.kind.label().into(),
                state: StepState::Pending,
                detail: String::new(),
            })
            .collect();
        self.transfer_plan_mut()?.ovf_import = Some(Box::new(plan));
        self.transfer_plan_mut()?.expires_at_ms = now().saturating_add(24 * 60 * 60 * 1000);
        self.save(j).await?;
        for index in 0..self.ovf_plan()?.steps.len() {
            self.reload(j)?;
            if self.stop_before_dispatch() {
                return self.save(j).await;
            }
            if !self.ovf_active() {
                return Ok(());
            }
            if now() >= self.transfer_plan()?.expires_at_ms {
                return Err(Error::TransferExpired);
            }
            self.ovf_inputs().await?;
            match self.ovf_step(index)?.kind {
                Kind::Upload { .. } => self.ovf_upload(j, s, c, index).await?,
                Kind::Verify { guest } => {
                    import::verify_guest(s, self.ovf_plan()?, guest).await?;
                    self.reload(j)?;
                    if !self.ovf_active() {
                        return Ok(());
                    }
                    self.ovf_step_mut(index)?.complete = true;
                    self.record.steps[index].state = StepState::Verified;
                    self.save(j).await?;
                }
                _ => self.ovf_create(j, s, c, index).await?,
            }
            if self.stop_before_dispatch() {
                return self.save(j).await;
            }
            if !self.ovf_active() {
                return Ok(());
            }
        }
        self.ovf_finish(j).await
    }
    async fn ovf_finish(&mut self, j: &mut Journal) -> Result<(), Error> {
        self.reload(j)?;
        if self.record.state.terminal() {
            return Ok(());
        }
        if self.ovf_plan()?.steps.iter().any(|s| !s.complete) {
            return Err(Error::TransferConflict);
        }
        self.record.state = OperationState::Succeeded;
        self.record.progress = Some(1.0);
        self.record.can_cancel = false;
        self.record.detail =
            "OVF guests, disks and mapped networks verified; no VM power actions submitted".into();
        if let Some(p) = &mut self.record.transfer {
            p.phase = self.record.detail.clone();
        }
        self.save(j).await
    }
    async fn ovf_create(
        &mut self,
        j: &mut Journal,
        s: &Session,
        c: &CancellationToken,
        index: usize,
    ) -> Result<(), Error> {
        let (method, args) = import::arguments(s, self.ovf_plan()?, index).await?;
        self.reload(j)?;
        if self.stop_before_dispatch() {
            return self.save(j).await;
        }
        if !self.ovf_active() {
            return Ok(());
        }
        if self.ovf_step(index)?.dispatched {
            return Err(Error::TransferConflict);
        }
        self.ovf_step_mut(index)?.dispatched = true;
        self.record.steps[index].state = StepState::Dispatching;
        self.record.state = OperationState::Tracking;
        self.record.can_cancel = true;
        self.record.detail = self.ovf_step(index)?.kind.label().into();
        self.save(j).await?;
        let value =
            tokio::select! {_=c.cancelled()=>return Err(Error::Closed),v=s.call(method,args)=>v?};
        let reference = crate::resources::xapi::reply_reference(&value)?;
        self.ovf_evidence(j, index, &reference, None, false).await?;
        let (reference, id, _) = import::provenance(s, self.ovf_plan()?, index).await?;
        self.ovf_evidence(j, index, &reference, Some(id), false)
            .await?;
        let (reference, id) = import::identify(s, self.ovf_plan()?, index).await?;
        self.ovf_evidence(j, index, &reference, Some(id), true)
            .await
    }
    async fn ovf_evidence(
        &mut self,
        j: &mut Journal,
        index: usize,
        reference: &str,
        id: Option<Uuid>,
        verified: bool,
    ) -> Result<(), Error> {
        for _ in 0..16 {
            self.reload(j)?;
            let st = self.ovf_step_mut(index)?;
            if st.reference.as_deref().is_some_and(|r| r != reference)
                || id.is_some_and(|id| st.uuid.is_some_and(|old| old != id))
            {
                return Err(Error::TransferConflict);
            }
            st.reference = Some(reference.into());
            if let Some(id) = id {
                st.uuid = Some(id);
                st.complete |= verified;
                match st.kind.class() {
                    Some("VM") => {
                        let id = id.into();
                        if !self.record.transfer_created.vms.contains(&id) {
                            self.record.transfer_created.vms.push(id)
                        }
                    }
                    Some("VDI") => {
                        let id = id.into();
                        if !self.record.transfer_created.disks.contains(&id) {
                            self.record.transfer_created.disks.push(id)
                        }
                    }
                    Some("VBD") => {
                        let id = id.into();
                        if !self.record.transfer_created.vbds.contains(&id) {
                            self.record.transfer_created.vbds.push(id)
                        }
                    }
                    Some("VIF") => {
                        let id = id.into();
                        if !self.record.transfer_created.vifs.contains(&id) {
                            self.record.transfer_created.vifs.push(id)
                        }
                    }
                    _ => return Err(Error::TransferConflict),
                }
                if verified {
                    self.record.steps[index].state = StepState::Verified;
                    self.record.steps[index].detail =
                        "New UUID, ownership and requested configuration verified".into();
                    if self.ovf_active() {
                        self.record.state = OperationState::Preparing;
                    }
                }
            }
            match self.save(j).await {
                Err(Error::JournalChanged) => continue,
                result => return result,
            }
        }
        Err(Error::JournalChanged)
    }
    async fn ovf_upload(
        &mut self,
        j: &mut Journal,
        s: &Session,
        c: &CancellationToken,
        index: usize,
    ) -> Result<(), Error> {
        let Kind::Upload { guest, disk } = self.ovf_step(index)?.kind else {
            return Err(Error::TransferConflict);
        };
        let source = self
            .ovf_plan()?
            .disk(guest, disk)?
            .file
            .as_ref()
            .ok_or(Error::TransferConflict)?;
        let file = self
            .transfer_plan()?
            .file
            .ovf
            .as_ref()
            .ok_or(Error::TransferFile)?
            .files
            .get(source)
            .ok_or(Error::TransferFile)?
            .clone();
        let mut stream = Plan::new(file);
        stream.expires_at_ms = self.transfer_plan()?.expires_at_ms;
        import::upload_target(s, self.record.pool, self.ovf_plan()?, guest, disk).await?;
        self.reload(j)?;
        if self.stop_before_dispatch() {
            return self.save(j).await;
        }
        if !self.ovf_active() {
            return Ok(());
        }
        let st = self.ovf_step_mut(index)?;
        if st.dispatched {
            return Err(Error::TransferConflict);
        }
        st.dispatched = true;
        st.stream = Some(Box::new(stream));
        let name = format!("XenAdmin RS OVF disk {}", st.token);
        self.record.state = OperationState::Tracking;
        self.record.steps[index].state = StepState::Dispatching;
        self.record.detail = "Creating an owned disk upload task".into();
        self.save(j).await?;
        let value = tokio::select! {_=c.cancelled()=>return Err(Error::Closed),v=s.call("task.create",vec![json!(name),json!("Client-owned OVF disk upload")])=>v?};
        let task = crate::resources::xapi::reply_reference(&value)?;
        self.ovf_task_evidence(j, index, &task, None).await?;
        let task_record = s.call("task.get_record", vec![json!(task)]).await?;
        let id = self.ovf_task_id(index, &task_record, &name)?;
        self.ovf_task_evidence(j, index, &task, Some(id)).await?;
        if !self.ovf_active() {
            return Ok(());
        }
        let mut route =
            import::upload_target(s, self.record.pool, self.ovf_plan()?, guest, disk).await?;
        route.query.push(("task_id".into(), task));
        self.ovf_inputs().await?;
        self.reload(j)?;
        if self.stop_before_dispatch() {
            self.save(j).await?;
            self.ovf_cleanup_task(s, index).await;
            return Ok(());
        }
        if !self.ovf_active() {
            return Ok(());
        }
        self.ovf_step_mut(index)?
            .stream
            .as_mut()
            .ok_or(Error::TransferConflict)?
            .dispatched = true;
        self.record.detail = "Streaming an OVF disk into its new unattached VDI".into();
        self.record.can_cancel = true;
        self.save(j).await?;
        let stream = *self
            .ovf_step(index)?
            .stream
            .clone()
            .ok_or(Error::TransferConflict)?;
        let meter = Arc::new(http::Meter::default());
        let outcome =
            http::drive(s, route, stream, meter.clone(), c, |progress| {
                self.reload(j)?;
                if self.record.state.terminal() {
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
        self.reload(j)?;
        if self.record.state.terminal() {
            return Ok(());
        }
        let changed = self.ovf_inputs().await.is_err();
        let stream = self
            .ovf_step_mut(index)?
            .stream
            .as_mut()
            .ok_or(Error::TransferConflict)?;
        stream.stream_invalid |= changed;
        let mut progress = meter.progress();
        match outcome {
            Ok(receipt) => {
                stream.stream_receipt = Some(receipt);
                progress.stream_complete = true;
            }
            Err(Error::TransferFileChanged | Error::TransferLimit | Error::DiskImage) => {
                stream.stream_invalid = true
            }
            Err(_) => {}
        }
        progress.phase = "Checking host disk upload outcome".into();
        self.record.transfer = Some(progress);
        self.save(j).await?;
        self.ovf_check_upload(j, s, c, index).await
    }
    fn ovf_task_id(&self, index: usize, v: &serde_json::Value, name: &str) -> Result<Uuid, Error> {
        let id = TaskRecord::new(v)
            .uuid()
            .as_str()
            .and_then(|v| v.parse().ok())
            .ok_or(Error::TransferConflict)?;
        if self
            .ovf_plan()?
            .baseline
            .get("task")
            .is_none_or(|ids| ids.contains(&id))
            || TaskRecord::new(v).name_label() != name
            || TaskRecord::new(v).status() != "pending"
            || self.ovf_plan()?.steps.iter().enumerate().any(|(i, st)| {
                i != index && st.stream.as_ref().is_some_and(|p| p.task_uuid == Some(id))
            })
        {
            return Err(Error::TransferConflict);
        }
        Ok(id)
    }
    async fn ovf_task_evidence(
        &mut self,
        j: &mut Journal,
        index: usize,
        task: &str,
        id: Option<Uuid>,
    ) -> Result<(), Error> {
        for _ in 0..16 {
            self.reload(j)?;
            let st = self.ovf_step_mut(index)?;
            if st.task.as_deref().is_some_and(|r| r != task) {
                return Err(Error::TransferConflict);
            }
            st.task = Some(task.into());
            if let Some(id) = id {
                let stream = st.stream.as_mut().ok_or(Error::TransferConflict)?;
                if stream.task_uuid.is_some_and(|old| old != id) {
                    return Err(Error::TransferConflict);
                }
                stream.task_uuid = Some(id);
            }
            match self.save(j).await {
                Err(Error::JournalChanged) => continue,
                result => return result,
            }
        }
        Err(Error::JournalChanged)
    }
    async fn ovf_check_upload(
        &mut self,
        j: &mut Journal,
        s: &Session,
        c: &CancellationToken,
        index: usize,
    ) -> Result<(), Error> {
        loop {
            self.reload(j)?;
            if self.record.state.terminal() {
                return Ok(());
            }
            let st = self.ovf_step(index)?;
            let stream = st.stream.as_ref().ok_or(Error::TransferConflict)?;
            if !stream.dispatched {
                return self
                    .ovf_partial(
                        j,
                        "Upload was not dispatched; the new guest and disk are retained",
                    )
                    .await;
            }
            if stream.task_success || stream.task_failed || stream.task_cancelled {
                break;
            }
            let task = st.task.clone().ok_or(Error::TransferConflict)?;
            let id = stream.task_uuid.ok_or(Error::TransferConflict)?;
            let v = tokio::select! {_=c.cancelled()=>return Err(Error::Closed),v=s.call("task.get_record",vec![json!(task)])=>v?};
            self.reload(j)?;
            if self.record.state.terminal() {
                return Ok(());
            }
            if TaskRecord::new(&v)
                .uuid()
                .as_str()
                .and_then(|v| v.parse::<Uuid>().ok())
                != Some(id)
            {
                return Err(Error::TransferConflict);
            }
            let stream = self
                .ovf_step_mut(index)?
                .stream
                .as_mut()
                .ok_or(Error::TransferConflict)?;
            match TaskRecord::new(&v).status().as_str() {
                Some("success") => {
                    stream.task_success = true;
                    stream.result = Some(vec![crate::xapi::snapshot_task_result(&v)?]);
                }
                Some("failure") => stream.task_failed = true,
                Some("cancelled") => stream.task_cancelled = true,
                Some("pending" | "cancelling") => {}
                _ => return Err(Error::TransferConflict),
            }
            self.save(j).await?;
            let st = self.ovf_step(index)?;
            let stream = st.stream.as_ref().ok_or(Error::TransferConflict)?;
            if stream.task_success || stream.task_failed || stream.task_cancelled {
                break;
            }
            if self.record.cancel_requested
                && !st.cancelled
                && TaskRecord::new(&v)
                    .allowed_operations()
                    .as_array()
                    .is_some_and(|a| a.iter().any(|v| v == "cancel"))
            {
                self.ovf_step_mut(index)?.cancelled = true;
                self.save(j).await?;
                tokio::select! {_=c.cancelled()=>return Err(Error::Closed),v=s.call("task.cancel",vec![json!(task)])=>{v?;}}
            }
            if self.reconcile
                || self
                    .ovf_step(index)?
                    .stream
                    .as_ref()
                    .is_none_or(|p| p.stream_receipt.is_none())
            {
                self.record.state = OperationState::OutcomeUnknown;
                self.record.detail="OVF upload stopped while its host task is pending. Recheck never repeats an upload or creates remaining devices.".into();
                return self.save(j).await;
            }
            if now() >= self.transfer_plan()?.expires_at_ms {
                return Err(Error::TransferExpired);
            }
            tokio::select! {_=c.cancelled()=>return Err(Error::Closed),_=tokio::time::sleep(Duration::from_secs(1))=>{}}
        }
        let Kind::Upload { guest, disk } = self.ovf_step(index)?.kind else {
            return Err(Error::TransferConflict);
        };
        let st = self.ovf_step(index)?;
        let stream = st.stream.as_ref().ok_or(Error::TransferConflict)?;
        if !stream.task_success || stream.stream_invalid {
            self.ovf_partial(j,"Disk upload did not verify. Retained guest and disk UUIDs require inspection; remaining devices were not created").await?;
            self.ovf_cleanup_task(s, index).await;
            return Ok(());
        }
        let expected = self.ovf_plan()?.reference(Kind::Disk { guest, disk })?;
        if stream.result.as_deref() != Some(&[expected.to_owned()]) {
            return Err(Error::TransferConflict);
        }
        import::upload_target(s, self.record.pool, self.ovf_plan()?, guest, disk).await?;
        self.reload(j)?;
        if self.record.state.terminal() {
            return Ok(());
        }
        self.ovf_step_mut(index)?.complete = true;
        self.record.steps[index].state = StepState::Verified;
        self.record.steps[index].detail =
            "Host task confirmed upload into the verified new disk".into();
        if self.ovf_active() {
            self.record.state = OperationState::Preparing;
        }
        self.save(j).await?;
        self.ovf_cleanup_task(s, index).await;
        Ok(())
    }
    async fn ovf_cleanup_task(&self, s: &Session, index: usize) {
        if let Ok(st) = self.ovf_step(index)
            && let Some(task) = &st.task
            && let Some(id) = st.stream.as_ref().and_then(|p| p.task_uuid)
            && let Ok(Ok(v)) = tokio::time::timeout(
                Duration::from_secs(5),
                s.call("task.get_record", vec![json!(task)]),
            )
            .await
            && v["uuid"].as_str().and_then(|v| v.parse::<Uuid>().ok()) == Some(id)
        {
            let _ = tokio::time::timeout(
                Duration::from_secs(5),
                s.call("task.destroy", vec![json!(task)]),
            )
            .await;
        }
    }
    async fn ovf_partial(&mut self, j: &mut Journal, detail: &str) -> Result<(), Error> {
        self.reload(j)?;
        if self.record.state.terminal() {
            return Ok(());
        }
        self.record.state = OperationState::PartiallyCompleted;
        self.record.can_cancel = false;
        self.record.detail = detail.into();
        self.save(j).await
    }
    pub(super) async fn recheck_ovf_import(
        &mut self,
        j: &mut Journal,
        s: &Session,
        c: &CancellationToken,
    ) -> Result<(), Error> {
        self.reload(j)?;
        if self.record.state.terminal() {
            return Ok(());
        }
        for index in 0..self.ovf_plan()?.steps.len() {
            let st = self.ovf_step(index)?.clone();
            if st.complete {
                continue;
            }
            match st.kind{
                Kind::Verify{guest}=>{
                    import::verify_guest(s,self.ovf_plan()?,guest).await?;
                    self.reload(j)?;if self.record.state.terminal(){return Ok(())}
                    self.ovf_step_mut(index)?.complete=true;self.record.steps[index].state=StepState::Verified;self.save(j).await?;
                },
                Kind::Upload{..} if st.dispatched=>{
                    if st.stream.as_ref().is_some_and(|p|p.dispatched){self.ovf_check_upload(j,s,c,index).await?;if !self.ovf_step(index)?.complete{return Ok(())}}
                    else {return self.ovf_partial(j,"Import interrupted before disk upload; identified resources are retained and no upload was replayed").await}
                },
                _ if st.dispatched=>{
                    let (r,id,_)=import::provenance(s,self.ovf_plan()?,index).await?;
                    self.ovf_evidence(j,index,&r,Some(id),false).await?;
                    let (r,id)=import::identify(s,self.ovf_plan()?,index).await?;
                    self.ovf_evidence(j,index,&r,Some(id),true).await?;
                },
                _=>return self.ovf_partial(j,"Imported resources were identified after interruption. Remaining changes were not dispatched and will not be replayed; inspect the retained UUIDs").await,
            }
        }
        self.ovf_inputs().await?;
        for g in 0..self.ovf_plan()?.summary.guests.len() {
            import::verify_guest(s, self.ovf_plan()?, g).await?;
        }
        self.ovf_finish(j).await
    }
}
