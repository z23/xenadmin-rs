use super::*;
use crate::{
    ResourceAction, ResourceTarget, VbdId,
    cloud_init::{self, Attachment, Plan},
};
use serde_json::Value;
use xenadmin_xapi::enumerations::{vbd_mode, vbd_type, vm_power_state};
use xenadmin_xapi::records::{VbdRecord, VdiRecord, VmRecord};
use xenadmin_xapi::{calls, refs};

impl ResourceRequest {
    fn cloud_config(&self) -> Result<&crate::CloudInitConfiguration, Error> {
        match &self.record.action {
            ResourceAction::CloudInit(c) => Ok(c),
            _ => Err(Error::TransferConflict),
        }
    }
    fn cloud_plan(&self) -> Result<&Plan, Error> {
        self.record
            .cloud_init_plan
            .as_deref()
            .ok_or(Error::TransferConflict)
    }
    pub(super) async fn cloud_target(
        &self,
        session: &Session,
        vacant_slot: bool,
    ) -> Result<(String, Value), Error> {
        if session.options().read_only {
            return Err(Error::ReadOnly);
        }
        let config = self.cloud_config()?;
        if !config.valid() {
            return Err(Error::InvalidCloudInit);
        }
        let ResourceTarget::Vm(_) = self.record.target else {
            return Err(Error::Stale);
        };
        let (reference, vm) = crate::resources::xapi::read(session, self.record.target).await?;
        if VmRecord::new(&vm).is_a_snapshot() != false
            || VmRecord::new(&vm).is_a_template() != false
            || VmRecord::new(&vm).is_control_domain() != false
            || VmRecord::new(&vm).power_state() != vm_power_state::HALTED
        {
            return Err(Error::DeviceLifecycleRequiresHaltedVm);
        }
        if VmRecord::new(&vm)
            .current_operations()
            .entries()
            .is_none_or(|o| !o.is_empty())
        {
            return Err(Error::Busy);
        }
        if vacant_slot {
            let slots = session
                .send(calls::vm::GetAllowedVbdDevices(refs::vm(&reference)))
                .await?;
            if slots
                .as_array()
                .is_none_or(|s| s.len() > 256 || !s.iter().any(|s| s == &config.device))
            {
                return Err(Error::DeviceConflict);
            }
            let attachments = VmRecord::new(&vm)
                .vbds()
                .members()
                .filter(|v| v.len() <= 256)
                .ok_or(Error::DeviceConflict)?;
            for attachment in attachments {
                let r = crate::snapshots::reference(attachment)?;
                let vbd = session.send(calls::vbd::GetRecord(refs::vbd(&r))).await?;
                if VbdRecord::new(&vbd).vm() != reference {
                    return Err(Error::DeviceConflict);
                }
                if VbdRecord::new(&vbd)
                    .other_config()
                    .get(cloud_init::MARKER)
                    .is_some()
                {
                    return Err(Error::HostPrerequisite(
                        "This VM already has an app-created cloud-init drive; review its removal separately",
                    ));
                }
                if VbdRecord::new(&vbd).type_() == vbd_type::DISK {
                    let vdi = session
                        .send(calls::vdi::GetRecord(refs::vdi(
                            &crate::snapshots::reference(VbdRecord::new(&vbd).vdi())?,
                        )))
                        .await?;
                    if VdiRecord::new(&vdi)
                        .other_config()
                        .get(cloud_init::MARKER)
                        .is_some()
                    {
                        return Err(Error::HostPrerequisite(
                            "This VM already has an app-created cloud-init drive; review its removal separately",
                        ));
                    }
                }
            }
        }
        Ok((reference, vm))
    }
    pub(super) async fn execute_cloud_init(
        &mut self,
        journal: &mut Journal,
        session: &Session,
        cancel: &CancellationToken,
    ) -> Result<(), Error> {
        self.cloud_target(session, true).await?;
        let file = self.transfer_plan()?.file.clone();
        let hash = self.cloud_plan()?.sha256;
        tokio::task::spawn_blocking(move || cloud_init::verify_artifact(&file, hash))
            .await
            .map_err(|_| Error::CloudInitMedia)??;
        self.execute_transfer(journal, session, cancel).await
    }
    pub(super) async fn save_cloud_stream_evidence(
        &mut self,
        journal: &mut Journal,
        outcome: Result<crate::transfer::files::Receipt, Error>,
        mut progress: crate::TransferProgress,
    ) -> Result<bool, Error> {
        let file = self.transfer_plan()?.file.clone();
        let hash = self.cloud_plan()?.sha256;
        let source_valid =
            tokio::task::spawn_blocking(move || cloud_init::verify_artifact(&file, hash))
                .await
                .map_err(|_| Error::CloudInitMedia)?
                .is_ok();
        progress.stream_complete = outcome.is_ok();
        progress.phase = if outcome.is_ok() {
            "Upload stream completed; checking host result"
        } else {
            "Upload stream stopped; checking host result"
        }
        .into();
        for _ in 0..16 {
            self.reload(journal)?;
            if let Ok(receipt) = &outcome {
                if receipt.bytes != cloud_init::MEDIA_BYTES as u64 {
                    return Err(Error::TransferConflict);
                }
                self.transfer_plan_mut()?.stream_receipt = Some(receipt.clone());
                if source_valid {
                    self.record
                        .cloud_init_plan
                        .as_mut()
                        .ok_or(Error::TransferConflict)?
                        .upload_verified = true;
                }
                if let Some(step) = self.record.steps.get_mut(1) {
                    step.state = StepState::Verified;
                    step.detail = "Complete stream receipt saved".into();
                }
            }
            if !source_valid {
                self.transfer_plan_mut()?.stream_invalid = true;
            }
            self.record.transfer = Some(progress.clone());
            match self.save(journal).await {
                Err(Error::JournalChanged) => continue,
                Err(error) => return Err(error),
                Ok(()) => {
                    return Ok(!self.stopped_after_recovery() && !self.cloud_plan()?.interrupted);
                }
            }
        }
        Err(Error::JournalChanged)
    }
    pub(super) async fn complete_cloud_upload(
        &mut self,
        journal: &mut Journal,
        session: &Session,
        cancel: &CancellationToken,
    ) -> Result<(), Error> {
        self.reload(journal)?;
        if self.record.state.terminal() {
            return Ok(());
        }
        let plan = self.transfer_plan()?;
        if !plan.task_success
            || plan.stream_invalid
            || plan
                .stream_receipt
                .as_ref()
                .is_none_or(|r| r.bytes != cloud_init::MEDIA_BYTES as u64)
        {
            return Err(Error::TransferConflict);
        }
        if !self.cloud_plan()?.upload_verified {
            let file = self.transfer_plan()?.file.clone();
            let hash = self.cloud_plan()?.sha256;
            tokio::task::spawn_blocking(move || cloud_init::verify_artifact(&file, hash))
                .await
                .map_err(|_| Error::CloudInitMedia)??;
            self.reload(journal)?;
            self.record
                .cloud_init_plan
                .as_mut()
                .ok_or(Error::TransferConflict)?
                .upload_verified = true;
            self.save(journal).await?;
        }
        if self.cloud_plan()?.interrupted && !self.reconcile {
            self.record.state = OperationState::OutcomeUnknown;
            self.record.detail = "Upload evidence arrived after interruption. Recheck can verify retained resources; attachment will not be replayed.".into();
            return self.save(journal).await;
        }
        if self.reconcile {
            self.record.state = OperationState::PartiallyCompleted;
            self.record.can_cancel = false;
            self.record.detail = "Cloud-init image upload verified. Attachment was not dispatched and will not be replayed; the private data disk remains unattached.".into();
            self.save(journal).await?;
            self.cleanup_transfer_task(session).await;
            return Ok(());
        }
        if self.record.cancel_requested {
            self.record.state = OperationState::PartiallyCompleted;
            self.record.can_cancel = false;
            self.record.detail =
                "Stopped after upload; the private cloud-init data disk remains unattached.".into();
            return self.save(journal).await;
        }
        let (vm_ref, _) = self.cloud_target(session, true).await?;
        let (disk_ref, _) = crate::transfer::disk::identify(
            session,
            self.transfer_plan()?,
            &self.transfer_action()?,
            self.record.id,
        )
        .await?;
        let config = self.cloud_config()?.clone();
        let baseline = crate::device_lifecycle::baseline(session, "VBD").await?;
        let arguments = vec![
            json!({"VM":vm_ref,"VDI":disk_ref,"userdevice":config.device,"bootable":false,"mode":vbd_mode::RO,"type":vbd_type::DISK,"unpluggable":true,"empty":false,"other_config":{"owner":self.record.id.to_string(),cloud_init::MARKER:config.configuration_id.to_string()},"qos_algorithm_type":"","qos_algorithm_params":{}}),
        ];
        self.reload(journal)?;
        if self.cloud_plan()?.interrupted || self.stop_before_dispatch() {
            return self.save(journal).await;
        }
        if self.cloud_plan()?.attachment.is_some() {
            return Err(Error::TransferConflict);
        }
        self.record.cloud_init_plan.as_mut().unwrap().attachment = Some(Attachment {
            baseline,
            reference: None,
            uuid: None,
            verified: false,
        });
        self.record.steps.push(OperationStep {
            label: "Attach read-only cloud-init drive".into(),
            state: StepState::Dispatching,
            detail: String::new(),
        });
        self.record.state = OperationState::Tracking;
        self.record.can_cancel = false;
        self.record.detail =
            "Attaching the private cloud-init disk read-only; no guest is started".into();
        self.save(journal).await?;
        let result = tokio::select! { _=cancel.cancelled()=>return Err(Error::Closed), result=session.call("VBD.create", arguments)=>result? };
        let reference = crate::resources::xapi::reply_reference(&result)?;
        self.save_cloud_evidence(journal, Some(&reference), None, false)
            .await?;
        let (reference, id) = self.verify_cloud_attachment(session).await?;
        self.save_cloud_evidence(journal, Some(&reference), Some(id), true)
            .await?;
        self.finish_cloud_artifact(session).await;
        Ok(())
    }
    async fn save_cloud_evidence(
        &mut self,
        journal: &mut Journal,
        reference: Option<&str>,
        id: Option<VbdId>,
        verified: bool,
    ) -> Result<(), Error> {
        for _ in 0..16 {
            self.reload(journal)?;
            let stopped = self.record.state.terminal()
                || (!self.reconcile
                    && (self.record.state == OperationState::OutcomeUnknown
                        || self.cloud_plan()?.interrupted));
            let a = self
                .record
                .cloud_init_plan
                .as_mut()
                .and_then(|p| p.attachment.as_mut())
                .ok_or(Error::TransferConflict)?;
            if reference.is_some_and(|r| a.reference.as_deref().is_some_and(|old| old != r))
                || id.is_some_and(|id| a.uuid.is_some_and(|old| old != id))
            {
                return Err(Error::TransferConflict);
            }
            if let Some(r) = reference {
                a.reference = Some(r.into());
            }
            if let Some(id) = id {
                a.uuid = Some(id);
                if !self.record.transfer_created.vbds.contains(&id) {
                    self.record.transfer_created.vbds.push(id);
                }
            }
            a.verified |= verified;
            if verified && !stopped {
                self.record.state = OperationState::Succeeded;
                self.record.progress = Some(1.0);
                self.record.can_cancel = false;
                self.record.detail = "NoCloud drive uploaded and attached read-only. It contains user-data and remains attached. The VM was not started; guest execution is not verified.".into();
                for step in &mut self.record.steps {
                    step.state = StepState::Verified;
                }
                if let Some(p) = &mut self.record.transfer {
                    p.phase = self.record.detail.clone();
                }
            }
            match self.save(journal).await {
                Err(Error::JournalChanged) => continue,
                result => return result,
            }
        }
        Err(Error::JournalChanged)
    }
    async fn verify_cloud_attachment(&self, session: &Session) -> Result<(String, VbdId), Error> {
        let (vm_ref, vm) = self.cloud_target(session, false).await?;
        let config = self.cloud_config()?;
        if !self.cloud_plan()?.upload_verified || !self.transfer_plan()?.task_success {
            return Err(Error::TransferConflict);
        }
        let a = self
            .cloud_plan()?
            .attachment
            .as_ref()
            .ok_or(Error::TransferConflict)?;
        let creation = self
            .transfer_plan()?
            .disk_creation
            .as_ref()
            .ok_or(Error::TransferConflict)?;
        let disk = creation.uuid.ok_or(Error::TransferConflict)?;
        let (disk_ref, d) = crate::transfer::disk::disk_record(
            session,
            disk,
            config.sr,
            crate::DiskImageFormat::Raw,
            false,
        )
        .await?;
        if creation.reference.as_ref() != Some(&disk_ref)
            || VdiRecord::new(&d).other_config()[cloud_init::MARKER]
                != config.configuration_id.to_string()
            || VdiRecord::new(&d).other_config()[crate::transfer::disk::MARKER]
                != self.record.id.to_string()
            || VdiRecord::new(&d).is_a_snapshot() != false
            || VdiRecord::new(&d).read_only() != false
        {
            return Err(Error::TransferConflict);
        }
        let all = session.send(calls::vbd::GetAllRecords).await?;
        let matches = all
            .as_object()
            .filter(|m| m.len() <= 10000)
            .ok_or(Error::TransferLimit)?
            .iter()
            .filter(|(_, v)| {
                VbdRecord::new(v).other_config()[cloud_init::MARKER]
                    == config.configuration_id.to_string()
            })
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            return Err(Error::TransferConflict);
        }
        let (reference, candidate) = matches[0];
        let uuid = VbdRecord::new(candidate)
            .uuid()
            .text()
            .and_then(|s| s.parse::<Uuid>().ok())
            .filter(|u| !a.baseline.contains(u))
            .ok_or(Error::TransferConflict)?;
        let id = VbdId::from(uuid);
        if a.reference.as_ref().is_some_and(|r| r != reference)
            || a.uuid.is_some_and(|old| old != id)
        {
            return Err(Error::TransferConflict);
        }
        let fresh = session
            .send(calls::vbd::GetRecord(refs::vbd(reference)))
            .await?;
        if VbdRecord::new(&fresh).uuid() != uuid.to_string()
            || VbdRecord::new(&fresh).vm() != vm_ref
            || VbdRecord::new(&fresh).vdi() != disk_ref
            || VbdRecord::new(&fresh).userdevice() != config.device
            || VbdRecord::new(&fresh).mode() != vbd_mode::RO
            || VbdRecord::new(&fresh).type_() != vbd_type::DISK
            || VbdRecord::new(&fresh).empty() != false
            || VbdRecord::new(&fresh).bootable() != false
            || VbdRecord::new(&fresh).currently_attached() != false
            || VbdRecord::new(&fresh).other_config()[cloud_init::MARKER]
                != config.configuration_id.to_string()
            || VbdRecord::new(&fresh).other_config()["owner"] != self.record.id.to_string()
            || VbdRecord::new(&fresh)
                .current_operations()
                .entries()
                .is_none_or(|o| !o.is_empty())
            || VdiRecord::new(&d).vbds() != json!([reference])
            || VmRecord::new(&vm)
                .vbds()
                .members()
                .is_none_or(|v| !v.iter().any(|r| r == reference))
        {
            return Err(Error::TransferConflict);
        }
        Ok((reference.clone(), id))
    }
    pub(super) async fn recheck_cloud_init(
        &mut self,
        journal: &mut Journal,
        session: &Session,
        cancel: &CancellationToken,
    ) -> Result<(), Error> {
        self.reload(journal)?;
        if self.record.state.terminal() {
            return Ok(());
        }
        if self.cloud_plan()?.attachment.is_some() {
            let (reference, id) = self.verify_cloud_attachment(session).await?;
            self.save_cloud_evidence(journal, Some(&reference), Some(id), true)
                .await?;
            self.finish_cloud_artifact(session).await;
            Ok(())
        } else {
            self.recheck_transfer(journal, session, cancel).await
        }
    }
    async fn finish_cloud_artifact(&self, session: &Session) {
        if self.record.state != OperationState::Succeeded {
            return;
        }
        self.cleanup_transfer_task(session).await;
        if let Ok(plan) = self.transfer_plan() {
            let file = plan.file.clone();
            let _ = tokio::task::spawn_blocking(move || cloud_init::remove_artifact(&file)).await;
        }
    }
}
use uuid::Uuid;
