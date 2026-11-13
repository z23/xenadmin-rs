use super::*;
use crate::{
    HostPowerOperation, ResourceAction, host_power::Plan, resources::host_power_backend as backend,
};
use backend::require;
use xenadmin_xapi::{calls, refs};

async fn bounded<T>(
    cancel: &CancellationToken,
    future: impl std::future::Future<Output = Result<T, Error>>,
) -> Result<T, Error> {
    tokio::select! { _=cancel.cancelled()=>Err(Error::Closed), result=tokio::time::timeout(Duration::from_secs(60),future)=>result.map_err(|_|Error::Timeout)? }
}
fn step(label: &str) -> OperationStep {
    OperationStep {
        label: label.into(),
        state: StepState::Pending,
        detail: String::new(),
    }
}
impl ResourceRequest {
    async fn power_merge(&mut self, journal: &mut Journal) -> Result<bool, Error> {
        self.record = journal.host_power_evidence(&self.record)?;
        self.announce().await;
        Ok(!self.stopped_after_recovery())
    }
    async fn power_begin(
        &mut self,
        journal: &mut Journal,
        cancel: &CancellationToken,
        index: usize,
        mark: impl FnOnce(&mut Plan),
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
            self.record.state == OperationState::Preparing
                && self
                    .record
                    .steps
                    .get(index)
                    .is_some_and(|s| s.state == StepState::Pending),
            "Host power step is no longer ready to dispatch",
        )?;
        mark(self.record.host_power_plan.as_mut().ok_or(Error::Journal)?);
        self.record.state = OperationState::Dispatching;
        self.record.steps[index].state = StepState::Dispatching;
        self.record.detail = self.record.steps[index].label.clone();
        self.save(journal).await?;
        Ok(true)
    }
    async fn power_finish(
        &mut self,
        journal: &mut Journal,
        state: OperationState,
        detail: &str,
    ) -> Result<(), Error> {
        self.reload(journal)?;
        if self.stopped_after_recovery() {
            return Ok(());
        }
        let plan = self.record.host_power_plan.as_ref().ok_or(Error::Journal)?;
        match &plan.operation {
            HostPowerOperation::CleanupSecret { .. } => {
                if plan.cleaned {
                    self.record.steps[0].state = StepState::Verified;
                }
            }
            HostPowerOperation::Configure { settings, .. } => {
                let index = usize::from(settings.needs_password());
                if index == 1 && plan.secret.is_some() {
                    self.record.steps[0].state = StepState::Verified;
                }
                if plan.verified {
                    self.record.steps[index].state = StepState::Verified;
                }
            }
        }
        self.record.state = state;
        self.record.can_cancel = false;
        self.record.detail = detail.into();
        self.save(journal).await
    }
    fn power_cleanup_source(
        &self,
        journal: &Journal,
    ) -> Result<(OperationId, crate::SecretId), Error> {
        let ResourceAction::HostPower(HostPowerOperation::CleanupSecret { operation, secret }) =
            self.record.action
        else {
            return Err(Error::Journal);
        };
        let source = journal.resource_load(operation)?.ok_or(Error::Journal)?;
        require(
            source.pool == self.record.pool
                && source.target == self.record.target
                && source.state.terminal(),
            "Acknowledge the original host power operation before reviewing Secret cleanup",
        )?;
        let plan = source.host_power_plan.as_ref().ok_or(Error::Journal)?;
        require(
            matches!(plan.operation, HostPowerOperation::Configure { .. })
                && plan.secret == Some(secret)
                && (!plan.configure_sent || plan.configure_completed),
            "The original host setting write may still be in flight; Secret cleanup is not allowed",
        )?;
        Ok((operation, secret))
    }
    pub(super) async fn execute_host_power(
        &mut self,
        journal: &mut Journal,
        session: &Session,
        cancel: &CancellationToken,
    ) -> Result<(), Error> {
        let ResourceAction::HostPower(operation) = self.record.action.clone() else {
            return Err(Error::Journal);
        };
        self.record.action.validate(self.record.target)?;
        match &operation {
            HostPowerOperation::CleanupSecret { .. } => {
                let (source, secret) = self.power_cleanup_source(journal)?;
                bounded(
                    cancel,
                    xapi::read(
                        session,
                        super::super::ResourceTarget::Pool(self.record.pool),
                    ),
                )
                .await?;
                bounded(cancel, xapi::read(session, self.record.target)).await?;
                let reference = bounded(cancel, backend::owned(session, source, secret)).await?;
                bounded(cancel, backend::unused(session, secret)).await?;
                let mut plan = Plan::new(operation, vec![]);
                plan.secret = Some(secret);
                self.record.host_power_plan = Some(Box::new(plan));
                crate::host_power::project(&mut self.record);
                self.record.steps = vec![step("Remove proven-unused host power Secret")];
                self.save(journal).await?;
                if !self
                    .power_begin(journal, cancel, 0, |p| p.cleanup_sent = true)
                    .await?
                {
                    return Ok(());
                }
                bounded(
                    cancel,
                    session.send(calls::secret::Destroy(refs::secret(&reference))),
                )
                .await?;
                self.record.host_power_plan.as_mut().unwrap().cleaned = true;
                if !self.power_merge(journal).await? {
                    return Ok(());
                }
                require(
                    bounded(cancel, backend::absent(session, secret)).await?,
                    "Secret deletion could not be verified",
                )?;
                self.power_finish(
                    journal,
                    OperationState::Succeeded,
                    "Owned unused host power Secret removed",
                )
                .await
            }
            HostPowerOperation::Configure { settings, .. } => {
                let password = self.host_power_password.take();
                require(
                    password.is_some() == settings.needs_password(),
                    "IPMI requires a new transient password",
                )?;
                bounded(cancel, backend::expected(session, &self.record)).await?;
                let baseline = if settings.needs_password() {
                    bounded(cancel, backend::secret_ids(session)).await?
                } else {
                    vec![]
                };
                self.record.host_power_plan =
                    Some(Box::new(Plan::new(operation.clone(), baseline)));
                self.record.steps = if settings.needs_password() {
                    vec![
                        step("Create host power-on Secret"),
                        step("Save host power-on settings"),
                    ]
                } else {
                    vec![step("Save host power-on settings")]
                };
                self.save(journal).await?;
                if let Some(password) = password {
                    if !self
                        .power_begin(journal, cancel, 0, |p| p.secret_sent = true)
                        .await?
                    {
                        return Ok(());
                    }
                    let result = bounded(
                        cancel,
                        session.send(calls::secret::Create(backend::secret_record(
                            self.record.id,
                            &password.0,
                        ))),
                    )
                    .await?;
                    let reference = xapi::reply_reference(&result)?;
                    self.record
                        .host_power_plan
                        .as_mut()
                        .unwrap()
                        .secret_completed = true;
                    let proceed = self.power_merge(journal).await?;
                    let id = bounded(cancel, backend::secret_uuid(session, &reference)).await?;
                    bounded(cancel, backend::owned(session, self.record.id, id)).await?;
                    require(
                        !self
                            .record
                            .host_power_plan
                            .as_ref()
                            .unwrap()
                            .baseline
                            .contains(&id),
                        "Host power Secret creation returned a pre-existing identity",
                    )?;
                    self.record.host_power_plan.as_mut().unwrap().secret = Some(id);
                    if !self.power_merge(journal).await? || !proceed {
                        return Ok(());
                    }
                    self.reload(journal)?;
                    if self.stopped_after_recovery() {
                        return Ok(());
                    }
                    self.record.state = OperationState::Preparing;
                    self.record.steps[0].state = StepState::Verified;
                    self.save(journal).await?;
                }
                let reference = bounded(cancel, backend::expected(session, &self.record)).await?;
                let secret = self.record.host_power_plan.as_ref().unwrap().secret;
                if let Some(id) = secret {
                    bounded(cancel, backend::owned(session, self.record.id, id)).await?;
                }
                let argument = backend::arguments(settings, secret)?;
                if !self
                    .power_begin(
                        journal,
                        cancel,
                        usize::from(settings.needs_password()),
                        |p| p.configure_sent = true,
                    )
                    .await?
                {
                    return Ok(());
                }
                bounded(
                    cancel,
                    session.send(calls::host::SetPowerOnMode {
                        self_: refs::host(&reference),
                        power_on_mode: settings.mode(),
                        power_on_config: argument,
                    }),
                )
                .await?;
                self.record
                    .host_power_plan
                    .as_mut()
                    .unwrap()
                    .configure_completed = true;
                if !self.power_merge(journal).await? {
                    return Ok(());
                }
                bounded(cancel, backend::verify(session, &self.record)).await?;
                self.record.host_power_plan.as_mut().unwrap().verified = true;
                if !self.power_merge(journal).await? {
                    return Ok(());
                }
                self.power_finish(
                    journal,
                    OperationState::Succeeded,
                    "Power-on settings and Secret verified; physical power-on was not tested",
                )
                .await
            }
        }
    }
    pub(super) async fn reconcile_host_power(
        &mut self,
        journal: &mut Journal,
        session: &Session,
        cancel: &CancellationToken,
    ) -> Result<(), Error> {
        self.reload(journal)?;
        if self.stopped_after_recovery() {
            return Ok(());
        }
        let plan = self.record.host_power_plan.clone().ok_or(Error::Journal)?;
        self.record.state = OperationState::Verifying;
        self.record.can_cancel = false;
        self.record.detail =
            "Inspecting existing host power evidence; no changes will be replayed".into();
        self.save(journal).await?;
        match &plan.operation {
            HostPowerOperation::CleanupSecret { secret, .. } => {
                require(
                    plan.cleaned,
                    "Secret deletion has no completion receipt; no deletion will be replayed",
                )?;
                require(
                    bounded(cancel, backend::absent(session, *secret)).await?,
                    "Secret deletion could not be verified",
                )?;
                self.power_finish(
                    journal,
                    OperationState::Succeeded,
                    "Owned unused host power Secret removal verified",
                )
                .await
            }
            HostPowerOperation::Configure { .. } => {
                if plan.secret_sent && plan.secret.is_none() {
                    let secret =
                        bounded(cancel, backend::recover_secret(session, &self.record)).await?;
                    self.record.host_power_plan.as_mut().unwrap().secret = secret;
                    if !self.power_merge(journal).await? {
                        return Ok(());
                    }
                }
                if plan.configure_sent {
                    bounded(cancel, backend::verify(session, &self.record)).await?;
                    self.record.host_power_plan.as_mut().unwrap().verified = true;
                    if !self.power_merge(journal).await? {
                        return Ok(());
                    }
                    self.power_finish(
                        journal,
                        OperationState::Succeeded,
                        "Completed host power-on configuration verified without replay",
                    )
                    .await
                } else {
                    require(
                        self.record
                            .host_power_plan
                            .as_ref()
                            .unwrap()
                            .secret
                            .is_some(),
                        "Secret creation outcome remains unknown; no change will be replayed",
                    )?;
                    self.power_finish(journal,OperationState::PartiallyCompleted,"Host power Secret retained; settings were not dispatched. Review unused Secret cleanup separately").await
                }
            }
        }
    }
}
