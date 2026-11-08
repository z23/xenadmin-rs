use super::*;
use crate::{TransferAction, transfer::disk};
impl ResourceRequest {
    pub(super) async fn create_transfer_disk(
        &mut self,
        journal: &mut Journal,
        session: &Session,
        cancel: &CancellationToken,
    ) -> Result<(), Error> {
        let action = self.transfer_action()?;
        let TransferAction::ImportDisk { sr, name, format } = action.clone() else {
            return Err(Error::TransferConflict);
        };
        if session.options().read_only {
            return Err(Error::ReadOnly);
        }
        crate::transfer::disk::prepare(session, self.record.pool, &action, None).await?;
        let size = self
            .transfer_plan()?
            .file
            .disk
            .as_ref()
            .filter(|d| d.format == format)
            .ok_or(Error::DiskImage)?
            .virtual_size;
        let input = self.transfer_plan()?.file.clone();
        tokio::task::spawn_blocking(move || crate::transfer::files::source(&input))
            .await
            .map_err(|_| Error::TransferFile)??;
        let baseline = crate::device_lifecycle::baseline(session, "VDI").await?;
        let mut arguments =
            disk::creation_arguments(session, sr, &name, size, self.record.id).await?;
        if let crate::ResourceAction::CloudInit(config) = &self.record.action {
            self.cloud_target(session, true).await?;
            arguments[0]["other_config"][crate::cloud_init::MARKER] =
                json!(config.configuration_id);
            arguments[0]["name_description"] = json!(
                "Private NoCloud user-data; remove only after guest provisioning is complete"
            );
        }
        self.reload(journal)?;
        if self.stop_before_dispatch() {
            return self.save(journal).await;
        }
        self.transfer_plan_mut()?.disk_creation = Some(disk::Creation {
            baseline,
            dispatched: true,
            reference: None,
            uuid: None,
        });
        self.record.steps = vec![
            OperationStep {
                label: "Create unattached disk".into(),
                state: StepState::Dispatching,
                detail: String::new(),
            },
            OperationStep {
                label: "Transfer image".into(),
                state: StepState::Pending,
                detail: String::new(),
            },
        ];
        // Creation errors can leave a storage-side object even without a returned reference.
        self.record.state = OperationState::Tracking;
        self.record.detail = "Creating a new disk; existing disks are not modified".into();
        self.save(journal).await?;
        let value = tokio::select! {_=cancel.cancelled()=>return Err(Error::Closed),value=session.call("VDI.create",arguments)=>value?};
        let reference = crate::resources::xapi::reply_reference(&value)?;
        self.save_disk_evidence(journal, &reference, None, true)
            .await?;
        let (reference, id) =
            disk::identify(session, self.transfer_plan()?, &action, self.record.id).await?;
        self.save_disk_evidence(journal, &reference, Some(id), true)
            .await
    }
    async fn save_disk_evidence(
        &mut self,
        journal: &mut Journal,
        reference: &str,
        id: Option<crate::VdiId>,
        continue_upload: bool,
    ) -> Result<(), Error> {
        for _ in 0..16 {
            self.reload(journal)?;
            if !continue_upload && self.transfer_plan()?.dispatched {
                return Err(Error::JournalChanged);
            }
            let created = self
                .transfer_plan_mut()?
                .disk_creation
                .as_mut()
                .ok_or(Error::TransferConflict)?;
            if created.reference.as_deref().is_some_and(|r| r != reference)
                || id.is_some_and(|id| created.uuid.is_some_and(|old| old != id))
            {
                return Err(Error::TransferConflict);
            }
            created.reference = Some(reference.into());
            if let Some(id) = id {
                created.uuid = Some(id);
                if !self.record.transfer_created.disks.contains(&id) {
                    self.record.transfer_created.disks.push(id);
                }
                if let Some(step) = self.record.steps.first_mut() {
                    step.state = StepState::Verified;
                }
                if !self.record.state.terminal() {
                    let continue_upload = continue_upload
                        && self.record.state != OperationState::OutcomeUnknown
                        && !self.record.cancel_requested;
                    self.record.state = if continue_upload {
                        OperationState::Preparing
                    } else {
                        OperationState::PartiallyCompleted
                    };
                    self.record.can_cancel = false;
                    self.record.detail=if continue_upload {"Created disk verified; preparing image transfer"}else{"New disk identity verified after interruption. Image upload was not dispatched and will not be replayed; the disk remains unattached."}.into();
                }
            }
            match self.save(journal).await {
                Err(Error::JournalChanged) => continue,
                result => return result,
            }
        }
        Err(Error::JournalChanged)
    }

    pub(super) async fn recover_transfer_disk_creation(
        &mut self,
        journal: &mut Journal,
        session: &Session,
    ) -> Result<(), Error> {
        let (reference, id) = disk::identify(
            session,
            self.transfer_plan()?,
            &self.transfer_action()?,
            self.record.id,
        )
        .await?;
        self.save_disk_evidence(journal, &reference, Some(id), false)
            .await?;
        if self.record.state.terminal() {
            self.cleanup_transfer_task(session).await;
        }
        Ok(())
    }
}
