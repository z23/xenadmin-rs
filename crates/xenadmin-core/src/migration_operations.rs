use super::*;
use crate::{CreatedDevice, OperationStep, StepState, migration_backend as migration};
use serde_json::Value;
impl MutationRequest {
    pub(super) async fn execute_migration(
        &mut self,
        session: &Session,
        journal: &mut Journal,
        cancel: &CancellationToken,
    ) -> Result<(), Error> {
        let OperationAction::Migration(operation) = &self.record.action else {
            return Err(Error::InvalidMigration);
        };
        let (plan, prepared) = migration::prepare(session, self.record.vm, operation).await?;
        self.record.steps = vec![OperationStep {
            label: "Migrate VM and reviewed devices".into(),
            state: StepState::Pending,
            detail: String::new(),
        }];
        if !operation.live {
            self.record.steps.push(OperationStep {
                label: "Set home host".into(),
                state: StepState::Pending,
                detail: String::new(),
            });
        }
        self.record.migration_plan = Some(Box::new(plan));
        self.record.state = OperationState::Preparing;
        self.record.can_cancel = true;
        if self.save_migration_evidence(journal, false).await? {
            return Ok(());
        }
        if !self.begin_step(journal, 0).await? {
            return Ok(());
        }
        let task = tokio::select! {_=cancel.cancelled()=>return Err(Error::Closed),result=prepared.send(session)=>result?};
        self.record.task = Some(crate::snapshots::reference(&task)?);
        self.record.state = OperationState::Tracking;
        if self.save_migration_evidence(journal, false).await? {
            return Ok(());
        }
        self.track_migration(session, journal, cancel, false).await
    }
    pub(super) async fn complete_migration(
        &mut self,
        session: &Session,
        journal: &mut Journal,
        task: &Value,
        allow_writes: bool,
    ) -> Result<(), Error> {
        let mut plan = *self.record.migration_plan.clone().ok_or(Error::Journal)?;
        if plan.operation.uses_storage_transport() && self.record.result_reference.is_none() {
            self.record.result_reference = Some(crate::xapi::snapshot_task_result(task)?);
        }
        plan.completed = true;
        plan.outcome = Some(migration::TaskOutcome::Success);
        self.record.migration_plan = Some(Box::new(plan.clone()));
        self.record.state = OperationState::Verifying;
        if self.save_migration_evidence(journal, !allow_writes).await? {
            return Ok(());
        }
        self.finish_migration(session, journal, &plan, allow_writes)
            .await?;
        if allow_writes {
            self.cleanup_migration_task(session).await;
        }
        Ok(())
    }
    async fn finish_migration(
        &mut self,
        session: &Session,
        journal: &mut Journal,
        plan: &migration::Plan,
        allow_writes: bool,
    ) -> Result<(), Error> {
        let created =
            migration::verify(session, plan, self.record.result_reference.as_deref()).await?;
        for disk in created {
            let resource = CreatedDevice::Disk(disk);
            if !self.record.created_devices.contains(&resource) {
                self.record.created_devices.push(resource);
            }
        }
        self.record.steps[0].state = StepState::Verified;
        self.record.steps[0].detail =
            "VM identity, state, host and device mappings verified".into();
        if self.save_migration_evidence(journal, !allow_writes).await? {
            return Ok(());
        }
        if !plan.operation.live {
            let home_host_was_dispatched = matches!(
                self.record.steps[1].state,
                StepState::Dispatching | StepState::OutcomeUnknown
            );
            if migration::affinity_matches(session, plan).await? {
                self.record.steps[1].state = StepState::Verified;
            } else if allow_writes && self.record.steps[1].state == StepState::Pending {
                let prepared = migration::prepare_affinity(session, plan).await?;
                if !self.begin_step(journal, 1).await? {
                    return Ok(());
                }
                if let Err(error) = prepared.send(session).await {
                    let definite = matches!(
                        error,
                        Error::Api(_)
                            | Error::Coordinator(_)
                            | Error::MigrationConflict
                            | Error::UnavailableAction
                            | Error::Busy
                    );
                    self.record.state = if definite {
                        OperationState::PartiallyCompleted
                    } else {
                        OperationState::OutcomeUnknown
                    };
                    self.record.steps[1].state = if definite {
                        StepState::Failed
                    } else {
                        StepState::OutcomeUnknown
                    };
                    self.record.detail = format!(
                        "Migration verified, but home-host change did not complete: {error}. No writes will be repeated automatically."
                    );
                    self.record.can_cancel = false;
                    self.save_migration_evidence(journal, false).await?;
                    return Ok(());
                }
                self.record.state = OperationState::Verifying;
                self.record
                    .migration_plan
                    .as_mut()
                    .ok_or(Error::Journal)?
                    .affinity_received = true;
                if self.save_migration_evidence(journal, false).await? {
                    return Ok(());
                }
                if !migration::affinity_matches(session, plan).await? {
                    return Err(Error::MigrationConflict);
                }
                self.record.steps[1].state = StepState::Verified;
            } else if self.record.steps[1].state == StepState::Pending {
                self.record.state = OperationState::PartiallyCompleted;
                self.record.detail="VM and devices moved; home host was not changed. Read-only recheck does not submit remaining writes.".into();
                self.record.can_cancel = false;
                self.save_migration_evidence(journal, !allow_writes).await?;
                return Ok(());
            } else {
                return Err(Error::MigrationConflict);
            }
            self.record.steps[1].detail = if self.record.migration_plan.as_ref().is_some_and(|p|p.affinity_received) {
                "Home-host setter reply received and desired value verified; VM remains halted"
            } else if home_host_was_dispatched {
                "Desired home-host value currently observed; setter completion is not attributable from this read; VM remains halted"
            } else {
                "Desired home host already observed; no setter was needed; VM remains halted"
            }.into();
        }
        self.record.state = OperationState::Succeeded;
        self.record.progress = Some(1.0);
        self.record.can_cancel = false;
        self.record.detail = if plan.operation.live {
            "Migration completed; VM identity and reviewed destination verified"
        } else {
            "Move completed and home host verified; no VM power actions submitted"
        }
        .into();
        self.save_migration_evidence(journal, !allow_writes).await?;
        Ok(())
    }
    pub(super) async fn failed_migration(
        &mut self,
        session: &Session,
        journal: &mut Journal,
        task: &Value,
        cancelled: bool,
        rechecking: bool,
    ) -> Result<(), Error> {
        let mut plan = self.record.migration_plan.clone().ok_or(Error::Journal)?;
        let code = task["error_info"]
            .as_array()
            .and_then(|a| a.first())
            .and_then(Value::as_str)
            .unwrap_or("TASK_FAILED");
        let code = crate::xapi::safe_error_code(code);
        plan.outcome = Some(if cancelled {
            migration::TaskOutcome::Cancelled
        } else {
            migration::TaskOutcome::Failure(code.to_owned())
        });
        self.record.migration_plan = Some(plan.clone());
        self.record.state = OperationState::Verifying;
        if self.save_migration_evidence(journal, rechecking).await? {
            return Ok(());
        }
        self.record.can_cancel = false;
        if let Ok(created) = migration::verify(session, &plan, None).await {
            for id in created {
                let value = CreatedDevice::Disk(id);
                if !self.record.created_devices.contains(&value) {
                    self.record.created_devices.push(value)
                }
            }
            self.record.state = OperationState::PartiallyCompleted;
            self.record.steps[0].state = StepState::Verified;
            self.record.detail="Host task did not succeed, but destination placement is observed. Review the VM and retained disks; no writes repeated.".into();
        } else if matches!(migration::unchanged(session, &plan).await, Ok(true)) {
            self.record.state = if cancelled {
                OperationState::Cancelled
            } else {
                OperationState::Failed
            };
            self.record.steps[0].state = StepState::Failed;
            self.record.detail = format!(
                "Host task {}; original placement verified. {}",
                if cancelled { "cancelled" } else { "failed" },
                code
            );
        } else {
            for id in migration::observed_disks(session, &plan)
                .await
                .unwrap_or_default()
            {
                let value = CreatedDevice::Disk(id);
                if !self.record.created_devices.contains(&value) {
                    self.record.created_devices.push(value)
                }
            }
            self.record.state = OperationState::OutcomeUnknown;
            self.record.steps[0].state = StepState::OutcomeUnknown;
            self.record.detail="Host task stopped, but the VM/device placement is incomplete or cannot be verified. Inspect and recheck; no writes repeated.".into();
        }
        if self.save_migration_evidence(journal, rechecking).await? {
            return Ok(());
        }
        if !rechecking {
            self.cleanup_migration_task(session).await;
        }
        Ok(())
    }
    async fn cleanup_migration_task(&self, session: &Session) {
        if self.record.state.terminal()
            && self
                .record
                .migration_plan
                .as_ref()
                .is_some_and(|plan| !plan.interrupted)
            && let Some(task) = &self.record.task
        {
            let _ = tokio::time::timeout(
                Duration::from_secs(5),
                session.call("task.destroy", vec![json!(task)]),
            )
            .await;
        }
    }
    pub(super) async fn recheck_migration(
        &mut self,
        session: &Session,
        journal: &mut Journal,
        cancel: &CancellationToken,
    ) -> Result<(), Error> {
        self.record = journal.load(self.record.id)?.ok_or(Error::Journal)?;
        if self.record.state.terminal() {
            return Ok(());
        }
        self.record.state = OperationState::Verifying;
        self.record.detail =
            "Reading existing migration evidence; no new token or migration will be submitted"
                .into();
        if self.save_migration_evidence(journal, true).await? {
            return Ok(());
        }
        if let Some(plan) = self.record.migration_plan.clone()
            && plan.completed
        {
            self.finish_migration(session, journal, &plan, false)
                .await?;
            return Ok(());
        }
        if let Some(outcome) = self
            .record
            .migration_plan
            .as_ref()
            .and_then(|p| p.outcome.clone())
        {
            let (cancelled, code) = match outcome {
                migration::TaskOutcome::Failure(code) => (false, code),
                migration::TaskOutcome::Cancelled => (true, "TASK_CANCELLED".into()),
                migration::TaskOutcome::Success => return Err(Error::Journal),
            };
            return self
                .failed_migration(
                    session,
                    journal,
                    &json!({"error_info":[code]}),
                    cancelled,
                    true,
                )
                .await;
        }
        if self.record.task.is_some() {
            return self.track_migration(session, journal, cancel, true).await;
        }
        let plan = self
            .record
            .migration_plan
            .clone()
            .ok_or(Error::UnavailableAction)?;
        // Without a task reference, matching placement cannot prove attribution or server completion.
        if let Ok(created) = migration::verify(session, &plan, None).await {
            for id in created {
                let value = CreatedDevice::Disk(id);
                if !self.record.created_devices.contains(&value) {
                    self.record.created_devices.push(value)
                }
            }
            self.record.detail="Reviewed destination is currently observed, but the migration task is unknown. Inspect before resolving; no writes repeated.".into();
        } else {
            for id in migration::observed_disks(session, &plan)
                .await
                .unwrap_or_default()
            {
                let value = CreatedDevice::Disk(id);
                if !self.record.created_devices.contains(&value) {
                    self.record.created_devices.push(value)
                }
            }
            self.record.detail="Migration acceptance or final placement remains unknown; verified replacement disk UUIDs remain listed. No writes repeated.".into();
        }
        self.record.state = OperationState::OutcomeUnknown;
        self.record.can_cancel = false;
        self.save_migration_evidence(journal, true).await?;
        Ok(())
    }
}
#[cfg(test)]
#[path = "migration_tests.rs"]
mod tests;
