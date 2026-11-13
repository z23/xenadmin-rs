use super::*;
use crate::{
    ResourceAction, StorageSecretEvidence,
    resources::storage_auth_backend as backend,
    storage_auth::{CleanupPlan, CleanupStep},
};
use backend::require;
use xenadmin_xapi::enumerations::task_status_type;
use xenadmin_xapi::records::TaskRecord;

async fn bounded<T>(
    cancel: &CancellationToken,
    future: impl std::future::Future<Output = Result<T, Error>>,
) -> Result<T, Error> {
    tokio::select! {_=cancel.cancelled()=>Err(Error::Closed),r=tokio::time::timeout(Duration::from_secs(60),future)=>r.map_err(|_|Error::Timeout)?}
}
fn step(label: &str) -> OperationStep {
    OperationStep {
        label: label.into(),
        state: StepState::Pending,
        detail: String::new(),
    }
}
impl ResourceRequest {
    async fn auth_merge(&mut self, journal: &mut Journal) -> Result<bool, Error> {
        self.record = journal.storage_auth_evidence(&self.record)?;
        self.announce().await;
        Ok(!self.stopped_after_recovery())
    }
    async fn auth_begin(
        &mut self,
        journal: &mut Journal,
        cancel: &CancellationToken,
        index: usize,
        mark: impl FnOnce(&mut ResourceOperationRecord),
    ) -> Result<bool, Error> {
        self.reload(journal)?;
        if self.stop_before_dispatch() {
            self.save(journal).await?;
            return Ok(false);
        }
        if cancel.is_cancelled() {
            return Err(Error::Closed);
        }
        require(
            self.record.state == OperationState::Preparing,
            "Storage workflow is no longer ready to dispatch",
        )?;
        require(
            self.record
                .steps
                .get(index)
                .is_some_and(|s| s.state == StepState::Pending),
            "Storage step was already dispatched",
        )?;
        mark(&mut self.record);
        self.record.state = OperationState::Dispatching;
        self.record.steps[index].state = StepState::Dispatching;
        self.record.detail = self.record.steps[index].label.clone();
        self.save(journal).await?;
        Ok(true)
    }
    async fn auth_next(&mut self, journal: &mut Journal) -> Result<bool, Error> {
        self.reload(journal)?;
        if self.stopped_after_recovery() {
            return Ok(false);
        }
        self.record.state = OperationState::Preparing;
        self.record.can_cancel = true;
        self.save(journal).await?;
        Ok(true)
    }
    pub(super) async fn execute_storage_auth(
        &mut self,
        journal: &mut Journal,
        session: &Session,
        cancel: &CancellationToken,
    ) -> Result<(), Error> {
        if matches!(
            self.record.action,
            ResourceAction::CleanupStorageCredentials { .. }
        ) {
            return self.execute_secret_cleanup(journal, session, cancel).await;
        }
        let credentials = self
            .storage_credentials
            .take()
            .ok_or(Error::InvalidResourceCommand)?;
        bounded(
            cancel,
            backend::prepare(session, &mut self.record, &credentials),
        )
        .await?;
        let count = credentials.entries.len();
        self.record.steps = (0..count)
            .map(|_| step("Create temporary storage Secret"))
            .collect();
        self.record
            .steps
            .push(step("Provision authenticated storage"));
        self.record
            .steps
            .extend((0..count).map(|_| step("Remove unused temporary Secret")));
        self.save(journal).await?;
        for (index, credential) in credentials.entries.iter().enumerate() {
            let kind = self
                .record
                .storage_auth_plan
                .as_ref()
                .ok_or(Error::Journal)?
                .secrets[index]
                .kind;
            if !self
                .auth_begin(journal, cancel, index, |r| {
                    r.storage_auth_plan.as_mut().unwrap().secrets[index].sent = true
                })
                .await?
            {
                return Ok(());
            }
            let reply = bounded(
                cancel,
                session.call(
                    "secret.create",
                    vec![backend::secret_record(
                        self.record.id,
                        kind,
                        &credential.password,
                    )],
                ),
            )
            .await?;
            let reference = xapi::reply_reference(&reply)?;
            self.record
                .storage_auth_plan
                .as_mut()
                .ok_or(Error::Journal)?
                .secrets[index]
                .completed = true;
            // Record receipt before any follow-up read. The transient reference never enters history.
            let continue_work = self.auth_merge(journal).await?;
            let id = bounded(cancel, backend::secret_uuid(session, &reference)).await?;
            bounded(cancel, backend::owned(session, self.record.id, kind, id)).await?;
            require(
                !self
                    .record
                    .storage_auth_plan
                    .as_ref()
                    .unwrap()
                    .baseline
                    .contains(&id),
                "Secret creation returned a pre-existing identity",
            )?;
            self.record.storage_auth_plan.as_mut().unwrap().secrets[index].id = Some(id);
            if !self.auth_merge(journal).await?
                || !continue_work
                || !self.auth_next(journal).await?
            {
                return Ok(());
            }
        }
        let (method, args) = bounded(
            cancel,
            backend::arguments(session, &self.record, &credentials),
        )
        .await?;
        if !self
            .auth_begin(journal, cancel, count, |r| {
                r.storage_auth_plan.as_mut().unwrap().provision_sent = true
            })
            .await?
        {
            return Ok(());
        }
        let result = bounded(cancel, session.call(method, args)).await?;
        self.record.task = Some(xapi::reply_reference(&result)?);
        if !self.auth_merge(journal).await? {
            return Ok(());
        }
        self.record.state = OperationState::Tracking;
        self.save(journal).await?;
        if !self.auth_track(journal, session, cancel).await? {
            return Ok(());
        }
        self.auth_verify(journal, session, cancel).await?;
        if self.stopped_after_recovery() {
            return Ok(());
        }
        for index in 0..count {
            if !self.auth_next(journal).await? {
                return Ok(());
            }
            let secret = self.record.storage_auth_plan.as_ref().unwrap().secrets[index].clone();
            let id = secret.id.ok_or(Error::Journal)?;
            let reference = bounded(
                cancel,
                backend::owned(session, self.record.id, secret.kind, id),
            )
            .await?;
            bounded(cancel, backend::unused(session, id)).await?;
            if !self
                .auth_begin(journal, cancel, count + 1 + index, |r| {
                    r.storage_auth_plan.as_mut().unwrap().secrets[index].cleanup_sent = true
                })
                .await?
            {
                return Ok(());
            }
            bounded(
                cancel,
                session.call("secret.destroy", vec![json!(reference)]),
            )
            .await?;
            self.record.storage_auth_plan.as_mut().unwrap().secrets[index].cleaned = true;
            if !self.auth_merge(journal).await? {
                return Ok(());
            }
        }
        self.auth_finish(
            journal,
            OperationState::Succeeded,
            "Storage provisioning and temporary Secret cleanup verified",
        )
        .await?;
        self.cleanup_auth_task(session).await;
        Ok(())
    }
    async fn cleanup_auth_task(&self, session: &Session) {
        if self.record.state != OperationState::Succeeded {
            return;
        }
        let Some((reference, id)) = self.record.task.as_ref().zip(
            self.record
                .storage_auth_plan
                .as_ref()
                .and_then(|p| p.task_uuid),
        ) else {
            return;
        };
        let _ = tokio::time::timeout(Duration::from_secs(3), async {
            let task = session
                .call("task.get_record", vec![json!(reference)])
                .await?;
            require(
                backend::task_uuid(&task)? == id
                    && TaskRecord::new(&task).status() == task_status_type::SUCCESS,
                "Storage task identity changed before cleanup",
            )?;
            session.call("task.destroy", vec![json!(reference)]).await?;
            Ok::<_, Error>(())
        })
        .await;
    }
    async fn auth_track(
        &mut self,
        journal: &mut Journal,
        session: &Session,
        cancel: &CancellationToken,
    ) -> Result<bool, Error> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(300);
        loop {
            self.reload(journal)?;
            if self.stopped_after_recovery() {
                return Ok(false);
            }
            if self
                .record
                .storage_auth_plan
                .as_ref()
                .ok_or(Error::Journal)?
                .provision_succeeded
            {
                return Ok(true);
            }
            let reference=self.record.task.clone().ok_or(Error::Protocol("Storage dispatch has no task receipt; no provisioning or Secret cleanup will be replayed"))?;
            let task = bounded(
                cancel,
                session.call("task.get_record", vec![json!(reference)]),
            )
            .await?;
            let id = backend::task_uuid(&task)?;
            let plan = self
                .record
                .storage_auth_plan
                .as_mut()
                .ok_or(Error::Journal)?;
            require(
                !plan.task_baseline.contains(&id) && plan.task_uuid.is_none_or(|old| old == id),
                "Storage task identity no longer matches its receipt",
            )?;
            plan.task_uuid = Some(id);
            if !self.auth_merge(journal).await? {
                return Ok(false);
            }
            match TaskRecord::new(&task).status().name() {
                Some(task_status_type::SUCCESS) => {
                    self.record.result_reference = Some(crate::xapi::snapshot_task_result(&task)?);
                    self.record
                        .storage_auth_plan
                        .as_mut()
                        .unwrap()
                        .provision_succeeded = true;
                    return self.auth_merge(journal).await;
                }
                Some(task_status_type::FAILURE | task_status_type::CANCELLED) => {
                    return Err(Error::Protocol(
                        "Storage task did not succeed. Partial storage and Secrets may remain; inspect before cleanup",
                    ));
                }
                Some(task_status_type::PENDING | task_status_type::CANCELLING) => {}
                _ => return Err(Error::Protocol("Storage task status unavailable")),
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(Error::Timeout);
            }
            tokio::select! {_=cancel.cancelled()=>return Err(Error::Closed),_=tokio::time::sleep(Duration::from_secs(1))=>{}}
        }
    }
    async fn auth_verify(
        &mut self,
        journal: &mut Journal,
        session: &Session,
        cancel: &CancellationToken,
    ) -> Result<(), Error> {
        let created = bounded(cancel, backend::verify(session, &self.record)).await?;
        self.record.created = Some(created);
        self.record
            .storage_auth_plan
            .as_mut()
            .ok_or(Error::Journal)?
            .verified = true;
        self.auth_merge(journal).await?;
        Ok(())
    }
    async fn auth_finish(
        &mut self,
        journal: &mut Journal,
        state: OperationState,
        detail: &str,
    ) -> Result<(), Error> {
        self.reload(journal)?;
        if self.stopped_after_recovery() {
            return Ok(());
        }
        self.record.state = state;
        self.record.can_cancel = false;
        self.record.detail = detail.into();
        self.save(journal).await
    }
    pub(super) async fn reconcile_storage_auth(
        &mut self,
        journal: &mut Journal,
        session: &Session,
        cancel: &CancellationToken,
    ) -> Result<(), Error> {
        self.reload(journal)?;
        if self.record.state.terminal() {
            return Ok(());
        }
        if matches!(
            self.record.action,
            ResourceAction::CleanupStorageCredentials { .. }
        ) {
            let plan = self
                .record
                .storage_secret_cleanup
                .clone()
                .ok_or(Error::Journal)?;
            require(
                plan.secrets.iter().all(|s| !s.sent || s.completed),
                "Secret deletion reply was lost; absence alone does not prove completion. No deletion was repeated",
            )?;
            for secret in &plan.secrets {
                if secret.completed {
                    require(
                        bounded(cancel, backend::absent(session, secret.id)).await?,
                        "Deleted Secret is still present",
                    )?;
                }
            }
            let complete = plan.secrets.iter().all(|s| s.completed);
            return self
                .auth_finish(
                    journal,
                    if complete {
                        OperationState::Succeeded
                    } else {
                        OperationState::PartiallyCompleted
                    },
                    "Recorded Secret cleanup reconciled; pending deletions were not submitted",
                )
                .await;
        }
        let plan = self
            .record
            .storage_auth_plan
            .clone()
            .ok_or(Error::Journal)?;
        for (index, secret) in plan.secrets.iter().enumerate() {
            if secret.sent && secret.id.is_none() {
                if let Some(id) = bounded(
                    cancel,
                    backend::recover_secret(session, &self.record, index),
                )
                .await?
                {
                    self.record.storage_auth_plan.as_mut().unwrap().secrets[index].id = Some(id);
                    self.auth_merge(journal).await?;
                }
                require(
                    secret.completed,
                    "Secret creation reply was lost. Any identified Secret is retained; provisioning was not replayed",
                )?;
            }
        }
        if !plan.provision_sent {
            return self.auth_finish(journal,OperationState::PartiallyCompleted,"Storage was not submitted. Identified temporary Secrets are retained for explicit cleanup").await;
        }
        if !self.auth_track(journal, session, cancel).await? {
            return Ok(());
        }
        self.auth_verify(journal, session, cancel).await?;
        let plan = self
            .record
            .storage_auth_plan
            .as_ref()
            .ok_or(Error::Journal)?;
        require(
            plan.secrets.iter().all(|s| !s.cleanup_sent || s.cleaned),
            "Temporary Secret deletion reply was lost; no deletion was repeated",
        )?;
        let cleaned = plan.secrets.iter().all(|s| s.cleaned);
        self.auth_finish(
            journal,
            if cleaned {
                OperationState::Succeeded
            } else {
                OperationState::PartiallyCompleted
            },
            if cleaned {
                "Storage and recorded Secret cleanup verified"
            } else {
                "Storage verified; temporary Secrets retained for explicit cleanup"
            },
        )
        .await
    }
    async fn cleanup_source(
        &self,
        journal: &mut Journal,
        session: &Session,
    ) -> Result<ResourceOperationRecord, Error> {
        let ResourceAction::CleanupStorageCredentials { operation, secrets } = &self.record.action
        else {
            return Err(Error::InvalidResourceCommand);
        };
        let source = journal.resource_load(*operation)?.ok_or(Error::Protocol(
            "Source storage operation is no longer available",
        ))?;
        let plan = source
            .storage_auth_plan
            .as_ref()
            .ok_or(Error::InvalidResourceCommand)?;
        require(
            source.pool == self.record.pool && source.state.terminal(),
            "Resolve the source operation before requesting separate Secret cleanup",
        )?;
        for id in secrets {
            require(
                plan.secrets.iter().any(|s| s.id == Some(*id) && !s.cleaned),
                "Secret is not retained by the selected storage operation",
            )?;
        }
        if plan.provision_sent && !plan.provision_succeeded {
            let task = source.task.as_ref().ok_or(Error::Protocol(
                "Provisioning may still be running: missing task receipt prevents Secret cleanup",
            ))?;
            let status = session.call("task.get_record", vec![json!(task)]).await?;
            require(
                plan.task_uuid
                    .is_some_and(|id| backend::task_uuid(&status).ok() == Some(id)),
                "Storage task UUID was not verified; Secret cleanup is unavailable",
            )?;
            require(
                matches!(
                    TaskRecord::new(&status).status().name(),
                    Some(
                        task_status_type::SUCCESS
                            | task_status_type::FAILURE
                            | task_status_type::CANCELLED
                    )
                ),
                "Storage provisioning is still active; its Secrets cannot be deleted",
            )?;
        }
        Ok(source)
    }
    async fn execute_secret_cleanup(
        &mut self,
        journal: &mut Journal,
        session: &Session,
        cancel: &CancellationToken,
    ) -> Result<(), Error> {
        bounded(cancel, self.cleanup_source(journal, session)).await?;
        let ResourceAction::CleanupStorageCredentials { operation, secrets } = &self.record.action
        else {
            return Err(Error::InvalidResourceCommand);
        };
        self.record.storage_secret_cleanup = Some(Box::new(CleanupPlan {
            source: *operation,
            secrets: secrets
                .iter()
                .map(|id| CleanupStep {
                    id: *id,
                    sent: false,
                    completed: false,
                })
                .collect(),
        }));
        self.record.steps = secrets
            .iter()
            .map(|_| step("Remove reviewed unused storage Secret"))
            .collect();
        self.record.storage_secrets = secrets
            .iter()
            .map(|secret| StorageSecretEvidence {
                secret: *secret,
                cleaned: false,
            })
            .collect();
        let count = secrets.len();
        self.save(journal).await?;
        for index in 0..count {
            let source = bounded(cancel, self.cleanup_source(journal, session)).await?;
            let id = self.record.storage_secret_cleanup.as_ref().unwrap().secrets[index].id;
            let secret = source
                .storage_auth_plan
                .as_ref()
                .unwrap()
                .secrets
                .iter()
                .find(|s| s.id == Some(id))
                .ok_or(Error::Journal)?;
            let reference =
                bounded(cancel, backend::owned(session, source.id, secret.kind, id)).await?;
            bounded(cancel, backend::unused(session, id)).await?;
            if !self
                .auth_begin(journal, cancel, index, |r| {
                    r.storage_secret_cleanup.as_mut().unwrap().secrets[index].sent = true
                })
                .await?
            {
                return Ok(());
            }
            bounded(
                cancel,
                session.call("secret.destroy", vec![json!(reference)]),
            )
            .await?;
            self.record.storage_secret_cleanup.as_mut().unwrap().secrets[index].completed = true;
            if !self.auth_merge(journal).await? || !self.auth_next(journal).await? {
                return Ok(());
            }
        }
        self.auth_finish(
            journal,
            OperationState::Succeeded,
            "Reviewed unused temporary Secrets were deleted",
        )
        .await
    }
}

#[cfg(test)]
#[path = "storage_auth_worker_tests.rs"]
mod tests;
