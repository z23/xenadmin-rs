use super::*;
use crate::{management_network::Plan, resources::management};
use xenadmin_xapi::enumerations::task_status_type;
use xenadmin_xapi::records::TaskRecord;
use xenadmin_xapi::{calls, refs};
impl ResourceRequest {
    fn management_plan(&self) -> Result<&Plan, Error> {
        self.record
            .management_network_plan
            .as_deref()
            .ok_or(Error::Journal)
    }
    fn management_plan_mut(&mut self) -> Result<&mut Plan, Error> {
        self.record
            .management_network_plan
            .as_deref_mut()
            .ok_or(Error::Journal)
    }
    fn project_management(&mut self) -> Result<(), Error> {
        self.record.management_network = Some(self.management_plan()?.evidence());
        Ok(())
    }
    pub(super) async fn execute_management(
        &mut self,
        j: &mut Journal,
        s: &Session,
        c: &CancellationToken,
    ) -> Result<(), Error> {
        self.record.detail =
            "Checking the reviewed management interfaces, HA state and unique host certificate"
                .into();
        self.save(j).await?;
        let plan = tokio::select! {_=c.cancelled()=>return Err(Error::Closed),v=management::prepare(s,&self.record)=>v?};
        self.record.steps = vec![OperationStep {
            label: plan.operation.label().into(),
            state: StepState::Pending,
            detail: String::new(),
        }];
        self.record.management_network_plan = Some(Box::new(plan));
        self.project_management()?;
        self.save(j).await?;
        let inputs = self.management.as_ref().ok_or(Error::Protocol(
            "Recovery authentication context unavailable",
        ))?;
        tokio::select! {_=c.cancelled()=>return Err(Error::Closed),v=inputs.route.register(self.record.id,self.record.pool)=>v?};
        self.reload(j)?;
        if self.stop_before_dispatch() {
            return self.save(j).await;
        }
        self.management_plan_mut()?.registered = true;
        self.save(j).await?;
        let (method, args) = tokio::select! {_=c.cancelled()=>return Err(Error::Closed),v=management::arguments(s,&self.record)=>v?};
        self.reload(j)?;
        if self.stop_before_dispatch() {
            return self.save(j).await;
        }
        if c.is_cancelled() {
            return Err(Error::Closed);
        }
        if crate::resources::topology::now() >= self.management_plan()?.expires_at_ms {
            return Err(Error::Timeout);
        }
        if s.permissions().method(method) == crate::PermissionDecision::Denied {
            return Err(Error::Api("RBAC_PERMISSION_DENIED".into()));
        }
        self.record.state = OperationState::Dispatching;
        self.record.can_cancel = false;
        self.record.steps[0].state = StepState::Dispatching;
        self.record.detail =
            "Submitting the reviewed management change; connection loss does not establish failure"
                .into();
        self.save(j).await?;
        let value =
            tokio::select! {_=c.cancelled()=>return Err(Error::Closed),v=s.call(method,args)=>v?};
        self.record.task = Some(xapi::reply_reference(&value)?);
        self.record = j.resource_evidence(&self.record)?;
        self.announce().await;
        if self.stopped_after_recovery() {
            return Ok(());
        }
        self.track_management(j, s, c).await
    }
    pub(super) async fn reconcile_management(
        &mut self,
        j: &mut Journal,
        s: &Session,
        c: &CancellationToken,
    ) -> Result<(), Error> {
        self.reload(j)?;
        if self.stopped_after_recovery() {
            return Ok(());
        }
        management::validate_record(&self.record)?;
        if self.record.management_network_plan.is_none() {
            return Err(Error::Protocol(
                "Management preparation evidence unavailable; no change was replayed",
            ));
        }
        self.record.state = OperationState::Verifying;
        self.record.can_cancel = false;
        self.record.detail="Inspecting recorded management changes and the recovery endpoint; no configuration will be replayed".into();
        self.save(j).await?;
        self.track_management(j, s, c).await
    }
    async fn track_management(
        &mut self,
        j: &mut Journal,
        s: &Session,
        c: &CancellationToken,
    ) -> Result<(), Error> {
        loop {
            self.reload(j)?;
            if self.stopped_after_recovery() {
                return Ok(());
            }
            if self.management_plan()?.completed {
                return self.finish_management(j, s, c).await;
            }
            if self.management_plan()?.ended {
                return Err(Error::Protocol(
                    "Management task failed or was cancelled; partial network changes may remain",
                ));
            }
            let task=self.record.task.clone().ok_or(Error::Protocol("Management task receipt unavailable; inspect the recovery address before acknowledging, no change will be replayed"))?;
            let v = tokio::select! {_=c.cancelled()=>return Err(Error::Closed),v=s.send(calls::task::GetRecord(refs::task(&task)))=>v?};
            let id = crate::TaskId::from(crate::resources::network::uuid(&v)?);
            if id.uuid().is_nil()
                || self.management_plan()?.tasks_before.contains(&id)
                || self
                    .management_plan()?
                    .task_uuid
                    .is_some_and(|old| old != id)
            {
                return Err(Error::Protocol(
                    "Management task identity differs from this operation",
                ));
            }
            if TaskRecord::new(&v).status() == task_status_type::SUCCESS
                && matches!(&self.management_plan()?.operation, crate::ManagementNetworkOperation::Bond { operation, .. } if operation.creates())
            {
                self.record.result_reference = Some(crate::xapi::snapshot_task_result(&v)?);
            }
            let plan = self.management_plan_mut()?;
            plan.task_uuid = Some(id);
            match TaskRecord::new(&v).status().name() {
                Some(task_status_type::SUCCESS) => {
                    plan.completed = true;
                    plan.ended = true;
                }
                Some(task_status_type::FAILURE | task_status_type::CANCELLED) => plan.ended = true,
                Some(task_status_type::PENDING | task_status_type::CANCELLING) => {}
                _ => return Err(Error::Protocol("Management task status unavailable")),
            }
            self.project_management()?;
            self.record = j.resource_evidence(&self.record)?;
            self.announce().await;
            if self.stopped_after_recovery() {
                return Ok(());
            }
            if self.management_plan()?.ended {
                continue;
            }
            if self.reconcile
                || self.record.cancel_requested
                || crate::resources::topology::now() >= self.management_plan()?.expires_at_ms
            {
                self.record.state = OperationState::OutcomeUnknown;
                self.record.can_cancel = false;
                self.record.detail="The management task is still pending. No cancellation, rollback or configuration retry was sent.".into();
                return self.save(j).await;
            }
            self.record.state = OperationState::Tracking;
            self.record.can_cancel = false;
            self.record.detail="Waiting for the host management task; the reviewed recovery endpoint is available for failover".into();
            self.save(j).await?;
            tokio::select! {_=c.cancelled()=>return Err(Error::Closed),_=tokio::time::sleep(Duration::from_millis(250))=>{}}
        }
    }
    async fn finish_management(
        &mut self,
        j: &mut Journal,
        s: &Session,
        c: &CancellationToken,
    ) -> Result<(), Error> {
        if matches!(&self.management_plan()?.operation, crate::ManagementNetworkOperation::Bond { operation, .. } if operation.creates())
            && self
                .management_plan()?
                .bond
                .as_ref()
                .is_some_and(|b| b.created.is_none())
        {
            let created = tokio::select! {_=c.cancelled()=>return Err(Error::Closed),v=management::created_bond(s,&self.record)=>v?};
            self.management_plan_mut()?
                .bond
                .as_mut()
                .ok_or(Error::Journal)?
                .created = Some(created);
            self.project_management()?;
            self.record = j.resource_evidence(&self.record)?;
            self.announce().await;
            if self.stopped_after_recovery() {
                return Ok(());
            }
        }
        if let crate::ManagementNetworkOperation::Bond {
            source, operation, ..
        } = &self.management_plan()?.operation
        {
            let count = operation
                .captured_vlans(source)
                .ok_or(Error::Journal)?
                .len();
            if self
                .management_plan()?
                .bond
                .as_ref()
                .ok_or(Error::Journal)?
                .replacements
                .len()
                < count
            {
                let replacements = tokio::select! {_=c.cancelled()=>return Err(Error::Closed),v=management::bond_replacements(s,&self.record)=>v?};
                self.management_plan_mut()?
                    .bond
                    .as_mut()
                    .ok_or(Error::Journal)?
                    .replacements
                    .extend(replacements);
                self.project_management()?;
                self.record = j.resource_evidence(&self.record)?;
                self.announce().await;
                if self.stopped_after_recovery() {
                    return Ok(());
                }
            }
        }
        tokio::select! {_=c.cancelled()=>return Err(Error::Closed),v=management::verify(s,&self.record)=>v?};
        self.management_plan_mut()?.configuration_observed = true;
        self.project_management()?;
        self.record = j.resource_evidence(&self.record)?;
        self.announce().await;
        if self.stopped_after_recovery() {
            return Ok(());
        }
        self.record.state = OperationState::Verifying;
        self.record.detail="The new configuration was observed. Verifying the reviewed host at its recovery endpoint using the existing TLS policy.".into();
        self.save(j).await?;
        let plan = self.management_plan()?.clone();
        let certificate = tokio::select! {_=c.cancelled()=>return Err(Error::Closed),v=management::unique_certificate(s,plan.host)=>v?};
        if certificate != plan.certificate {
            return Err(Error::Protocol(
                "Reviewed host certificate changed; the recovery route remains unverified",
            ));
        }

        let inputs=self.management.as_ref().ok_or(Error::Protocol("Recovery authentication context unavailable; configuration observed but the recovery route remains unverified"))?;
        let options = inputs.options.for_endpoint(plan.recovery.clone());
        let recovery = tokio::select! {_=c.cancelled()=>return Err(Error::Closed),v=Session::login_bound(&options,&inputs.credentials,&plan.certificate)=>v?};
        let verification = tokio::select! {_=c.cancelled()=>Err(Error::Closed),v=async {management::verify(&recovery,&self.record).await?;if management::unique_certificate(&recovery,plan.host).await?!=plan.certificate{return Err(Error::Protocol("Reviewed host certificate changed during recovery verification"));}Ok(())}=>v};
        let _ = tokio::time::timeout(Duration::from_secs(2), recovery.logout()).await;
        verification?;
        self.reload(j)?;
        if self.stopped_after_recovery() {
            return Ok(());
        }
        self.management_plan_mut()?.route_verified = true;
        self.project_management()?;
        self.record.steps[0].state = StepState::Verified;
        self.record.steps[0].detail="Task completion, exact configuration and authenticated first-hop host identity verified".into();
        self.record.state = OperationState::Succeeded;
        self.record.progress = Some(1.0);
        self.record.can_cancel = false;
        self.record.detail = if matches!(plan.operation, crate::ManagementNetworkOperation::Bond { .. }) { "Management bond transition and certificate-bound recovery endpoint verified. The host transferred the reviewed static IPv4 configuration; no client rollback or retry was sent." } else { "Management configuration and the reviewed host recovery endpoint verified. Existing connections were not deliberately restarted; old interface addresses were retained." }.into();
        self.save(j).await?;
        if let (Some(task), Some(expected)) =
            (self.record.task.clone(), self.management_plan()?.task_uuid)
        {
            let _ = tokio::time::timeout(Duration::from_secs(2), async {
                let fresh = s.send(calls::task::GetRecord(refs::task(&task))).await?;
                if crate::resources::network::uuid(&fresh)? == expected.uuid() {
                    s.send(calls::task::Destroy(refs::task(&task))).await?;
                }
                Ok::<_, Error>(())
            })
            .await;
        }
        Ok(())
    }
}
