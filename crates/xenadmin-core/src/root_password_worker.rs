use super::*;
use crate::resources::host_power_backend::require;
use crate::{
    Credentials, HostId, PoolRootPasswordChange, ResourceAction, ResourceTarget,
    root_password::Plan,
};
use std::collections::BTreeSet;
use xenadmin_xapi::records::{PoolRecord, SessionRecord};

async fn bounded<T>(
    cancel: &CancellationToken,
    future: impl std::future::Future<Output = Result<T, Error>>,
) -> Result<T, Error> {
    tokio::select! { _=cancel.cancelled()=>Err(Error::Closed), result=tokio::time::timeout(Duration::from_secs(60),future)=>result.map_err(|_|Error::Timeout)? }
}
async fn expected(
    session: &Session,
    record: &ResourceOperationRecord,
    change: &PoolRootPasswordChange,
) -> Result<(), Error> {
    let (_, pool) = xapi::read(session, ResourceTarget::Pool(record.pool)).await?;
    let hosts = session.call("host.get_all_records", vec![]).await?;
    let hosts = hosts
        .as_object()
        .filter(|v| !v.is_empty() && v.len() <= 1024)
        .ok_or(Error::Protocol("invalid pool membership"))?;
    let actual: BTreeSet<_> = hosts
        .values()
        .map(|v| {
            HostId::parse(
                v["uuid"]
                    .as_str()
                    .ok_or(Error::Protocol("invalid host UUID"))?,
            )
            .map_err(|_| Error::Protocol("invalid host UUID"))
        })
        .collect::<Result<_, Error>>()?;
    let coordinator = PoolRecord::new(&pool)
        .master()
        .as_str()
        .and_then(|r| hosts.get(r))
        .and_then(|h| h["uuid"].as_str());
    require(
        actual.len() == hosts.len()
            && actual == change.members.iter().copied().collect()
            && coordinator == Some(change.coordinator.to_string().as_str()),
        "Pool membership or coordinator changed since review",
    )?;
    let own = session
        .call("session.get_record", vec![json!(session.session_cookie())])
        .await?;
    require(
        SessionRecord::new(&own).is_local_superuser() == true,
        "Changing the pool root password requires a local superuser session",
    )
}
struct TemporarySession(Arc<Session>);
impl Drop for TemporarySession {
    fn drop(&mut self) {
        let session = self.0.clone();
        tokio::spawn(async move {
            let _ = tokio::time::timeout(Duration::from_secs(3), session.logout()).await;
        });
    }
}
fn step(label: &str) -> OperationStep {
    OperationStep {
        label: label.into(),
        state: StepState::Pending,
        detail: String::new(),
    }
}
impl ResourceRequest {
    async fn password_merge(&mut self, journal: &mut Journal) -> Result<bool, Error> {
        self.record = journal.root_password_evidence(&self.record)?;
        self.announce().await;
        Ok(!self.stopped_after_recovery())
    }
    pub(super) async fn execute_root_password(
        &mut self,
        journal: &mut Journal,
        session: &Session,
        cancel: &CancellationToken,
    ) -> Result<(), Error> {
        let ResourceAction::RootPassword(change) = self.record.action.clone() else {
            return Err(Error::Journal);
        };
        let password = self
            .root_password
            .take()
            .ok_or(Error::InvalidResourceCommand)?;
        self.record.action.validate(self.record.target)?;
        bounded(cancel, expected(session, &self.record, &change)).await?;
        self.record.root_password_plan = Some(Box::new(Plan {
            expected: change.clone(),
            sent: false,
            completed: false,
            login_verified: false,
            credentials_updated: false,
        }));
        self.record.steps = vec![
            step("Change coordinator root password and request member synchronization"),
            step("Verify a new coordinator root login"),
            step("Update this client's reconnect credentials"),
        ];
        crate::root_password::project(&mut self.record);
        self.save(journal).await?;
        bounded(cancel, expected(session, &self.record, &change)).await?;
        self.reload(journal)?;
        if self.stop_before_dispatch() {
            return self.save(journal).await;
        }
        if cancel.is_cancelled() {
            return Err(Error::Closed);
        }
        require(
            self.record.state == OperationState::Preparing,
            "Password change is no longer ready to dispatch",
        )?;
        self.record.root_password_plan.as_mut().unwrap().sent = true;
        self.record.state = OperationState::Dispatching;
        self.record.steps[0].state = StepState::Dispatching;
        self.record.can_cancel = false;
        self.record.detail =
            "Changing the coordinator's local root password; member synchronization is best effort"
                .into();
        self.save(journal).await?;
        bounded(
            cancel,
            session.call(
                "session.change_password",
                vec![json!(""), json!(&*password.0)],
            ),
        )
        .await?;
        self.record.root_password_plan.as_mut().unwrap().completed = true;
        self.record.steps[0].state = StepState::Verified;
        self.record.state = OperationState::Verifying;
        self.record.detail = "Coordinator password change completed; verifying a new login".into();
        if !self.password_merge(journal).await? {
            return Ok(());
        }
        let credentials = Credentials::new("root".into(), password.0.to_string());
        let verified = TemporarySession(Arc::new(
            bounded(
                cancel,
                Session::login_exact(session.options(), &credentials),
            )
            .await?,
        ));
        bounded(cancel, expected(&verified.0, &self.record, &change)).await?;
        self.record
            .root_password_plan
            .as_mut()
            .unwrap()
            .login_verified = true;
        self.record.steps[1].state = StepState::Verified;
        if !self.password_merge(journal).await? {
            return Ok(());
        }
        if cancel.is_cancelled() {
            return Err(Error::Closed);
        }
        let (send, receive) = tokio::sync::oneshot::channel();
        self.updates
            .as_ref()
            .ok_or(Error::Closed)?
            .send(OperationUpdate::RootCredentials(Box::new(
                crate::connection::RootCredentialUpdate {
                    operation: self.record.id,
                    pool: self.record.pool,
                    generation: self.generation,
                    credentials,
                    reply: send,
                },
            )))
            .await
            .map_err(|_| Error::Closed)?;
        bounded(cancel, async { receive.await.map_err(|_| Error::Closed)? }).await?;
        self.record
            .root_password_plan
            .as_mut()
            .unwrap()
            .credentials_updated = true;
        self.record.steps[2].state = StepState::Verified;
        if !self.password_merge(journal).await? {
            return Ok(());
        }
        self.password_finish(journal).await
    }
    async fn password_finish(&mut self, journal: &mut Journal) -> Result<(), Error> {
        self.reload(journal)?;
        if self.stopped_after_recovery() {
            return Ok(());
        }
        let plan = self
            .record
            .root_password_plan
            .as_ref()
            .ok_or(Error::Journal)?;
        self.record.can_cancel = false;
        self.record.state = if plan.credentials_updated && plan.expected.members.len() == 1 {
            OperationState::Succeeded
        } else {
            OperationState::PartiallyCompleted
        };
        self.record.detail = if plan.credentials_updated {
            "Coordinator root login verified and client reconnect credentials updated. Other members, if any, remain unverified; verify their local access separately."
        } else {
            "The coordinator password change completed. Client credentials or member propagation remain unverified. Reconnect explicitly with the new password; this operation will not repeat the change."
        }.into();
        self.save(journal).await
    }
    pub(super) async fn reconcile_root_password(
        &mut self,
        journal: &mut Journal,
    ) -> Result<(), Error> {
        self.reload(journal)?;
        if self.record.state.terminal() {
            return Ok(());
        }
        let plan = self
            .record
            .root_password_plan
            .as_ref()
            .ok_or(Error::Journal)?;
        require(
            plan.completed,
            "The password change has no completion receipt. No change or credential update will be replayed; use an explicit reconnect to verify access",
        )?;
        self.password_finish(journal).await
    }
}
