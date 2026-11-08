use super::*;
use crate::{
    management_ipv6_bond::{Plan, Stage},
    resources::management_ipv6 as backend,
};
impl ResourceRequest {
    fn ipv6_plan(&self) -> Result<&Plan, Error> {
        self.record
            .management_ipv6_bond_plan
            .as_ref()
            .ok_or(Error::Journal)
    }
    fn set_ipv6_plan(&mut self, p: Plan) {
        p.project(&mut self.record);
        self.record.management_ipv6_bond_plan = Some(p);
    }
    async fn ipv6_route(
        &self,
        p: &Plan,
        prefix: usize,
        final_route: bool,
        c: &CancellationToken,
    ) -> Result<(), Error> {
        let inputs = self.management.as_ref().ok_or(Error::Protocol(
            "Management recovery authentication unavailable",
        ))?;
        let endpoint = if final_route {
            p.final_endpoint.clone()
        } else {
            p.temporary_endpoint.clone()
        };
        let options = inputs.options.for_endpoint(endpoint);
        let recovery = tokio::select! {_=c.cancelled()=>return Err(Error::Closed),v=Session::login_bound(&options,&inputs.credentials,&p.certificate)=>v?};
        let result = tokio::select! {_=c.cancelled()=>Err(Error::Closed),v=async{
            backend::verify(&recovery,p,prefix).await?;
            if crate::resources::management::unique_certificate(&recovery,p.host).await?!=p.certificate{return Err(Error::Protocol("Reviewed host certificate changed during IPv6 route verification"));}Ok(())
        }=>v};
        let _ = tokio::time::timeout(Duration::from_secs(2), recovery.logout()).await;
        result
    }
    pub(super) async fn execute_ipv6_bond(
        &mut self,
        j: &mut Journal,
        s: &Session,
        c: &CancellationToken,
    ) -> Result<(), Error> {
        if self.record.state.terminal() {
            return Ok(());
        }
        let p = if let Some(p) = &self.record.management_ipv6_bond_plan {
            p.clone()
        } else {
            tokio::select! {_=c.cancelled()=>return Err(Error::Closed),v=backend::prepare(s,&self.record)=>v?}
        };
        self.set_ipv6_plan(p);
        self.record.detail =
            "Checking the exact stage and authenticating the independent temporary IPv6 path"
                .into();
        self.save(j).await?;
        let inputs = self.management.as_ref().ok_or(Error::Closed)?;
        tokio::select! {_=c.cancelled()=>return Err(Error::Closed),v=inputs.route.register(self.record.id,self.record.pool)=>v?};
        self.reload(j)?;
        if self.stop_before_dispatch() {
            return self.save(j).await;
        }
        let mut p = self.ipv6_plan()?.clone();
        p.registered = true;
        self.ipv6_route(&p, p.depth as usize, false, c).await?;
        p.temporary_route_verified = true;
        if p.stage() == Stage::MoveToTarget {
            self.ipv6_route(&p, p.depth as usize, true, c).await?;
            p.final_route_verified = true;
        }
        let (method, args) = tokio::select! {_=c.cancelled()=>return Err(Error::Closed),v=backend::arguments(s,&mut p)=>v?};
        self.reload(j)?;
        if self.stop_before_dispatch() {
            return self.save(j).await;
        }
        if crate::resources::topology::now() >= p.expires_at_ms {
            return Err(Error::Timeout);
        }
        if s.permissions().method(method) == crate::PermissionDecision::Denied {
            return Err(Error::Api("RBAC_PERMISSION_DENIED".into()));
        }
        self.set_ipv6_plan(p);
        self.save(j).await?;
        self.reload(j)?;
        if self.stop_before_dispatch() {
            return self.save(j).await;
        }
        if c.is_cancelled() {
            return Err(Error::Closed);
        }
        let mut p = self.ipv6_plan()?.clone();
        p.current_mut().dispatched = true;
        self.set_ipv6_plan(p);
        self.record.state = OperationState::Dispatching;
        self.record.can_cancel = false;
        self.record.detail="Submitting one reviewed IPv6 handover stage; no rollback or automatic retry will be sent".into();
        self.save(j).await?;
        let reply =
            tokio::select! {_=c.cancelled()=>return Err(Error::Closed),v=s.call(method,args)=>v?};
        let mut p = self.ipv6_plan()?.clone();
        p.current_mut().task = Some(xapi::reply_reference(&reply)?);
        self.record.task = p.current().task.clone();
        self.set_ipv6_plan(p);
        self.record = j.ipv6_evidence(&self.record)?;
        self.announce().await;
        if self.stopped_after_recovery() {
            return Ok(());
        }
        self.track_ipv6_bond(j, s, c).await
    }
    pub(super) async fn reconcile_ipv6_bond(
        &mut self,
        j: &mut Journal,
        s: &Session,
        c: &CancellationToken,
    ) -> Result<(), Error> {
        self.reload(j)?;
        if self.stopped_after_recovery() {
            return Ok(());
        }
        crate::management_ipv6_bond::validate_record(&self.record)?;
        let p = self.ipv6_plan()?.clone();
        self.record.state = OperationState::Verifying;
        self.record.can_cancel = false;
        self.record.detail =
            "Inspecting the saved IPv6 stage receipt and route; no configuration is replayed"
                .into();
        self.save(j).await?;
        if !p.current().dispatched && p.depth > 0 {
            tokio::select! {_=c.cancelled()=>return Err(Error::Closed),v=backend::verify(s,&p,p.depth as usize)=>v?};
            self.ipv6_route(&p, p.depth as usize, false, c).await?;
            self.reload(j)?;
            self.record.state = OperationState::PartiallyCompleted;
            self.record.detail="The previous stages are verified and the current stage was never submitted. Review Continue to submit it once; the staged reservation remains held.".into();
            return self.save(j).await;
        }
        self.track_ipv6_bond(j, s, c).await
    }
    async fn track_ipv6_bond(
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
            let mut p = self.ipv6_plan()?.clone();
            if p.current().completed {
                return self.finish_ipv6_bond(j, s, c).await;
            }
            if p.current().ended {
                return Err(Error::Protocol(
                    "The IPv6 stage task failed or was cancelled; partial changes remain uncertain",
                ));
            }
            let task=p.current().task.clone().ok_or(Error::Protocol("The IPv6 stage receipt is unavailable; no submitted configuration will be replayed"))?;
            let v = tokio::select! {_=c.cancelled()=>return Err(Error::Closed),v=s.call("task.get_record",vec![json!(&task)])=>v?};
            let id = crate::TaskId::from(crate::resources::network::uuid(&v)?);
            if id.uuid().is_nil()
                || p.current().task_baseline.contains(&id)
                || p.current().task_uuid.is_some_and(|old| old != id)
            {
                return Err(Error::Protocol(
                    "IPv6 stage task identity differs from this submission",
                ));
            }
            let create = p.stage() == Stage::ChangeBond && p.intent.operation.creates();
            let receipt = p.current_mut();
            receipt.task_uuid = Some(id);
            match v["status"].as_str() {
                Some("success") => {
                    receipt.completed = true;
                    receipt.ended = true;
                    if create {
                        receipt.result = Some(crate::xapi::snapshot_task_result(&v)?);
                    }
                }
                Some("failure" | "cancelled") => receipt.ended = true,
                Some("pending" | "cancelling") => {}
                _ => return Err(Error::Protocol("IPv6 stage task status unavailable")),
            }
            self.record.result_reference = p.current().result.clone();
            self.set_ipv6_plan(p);
            self.record = j.ipv6_evidence(&self.record)?;
            self.announce().await;
            if self.stopped_after_recovery() {
                return Ok(());
            }
            if self.ipv6_plan()?.current().ended {
                continue;
            }
            if self.reconcile
                || self.record.cancel_requested
                || crate::resources::topology::now() >= self.ipv6_plan()?.expires_at_ms
            {
                self.record.state = OperationState::OutcomeUnknown;
                self.record.detail="IPv6 stage is still pending. The pool reservation remains held; no server cancellation or configuration replay was sent.".into();
                return self.save(j).await;
            }
            self.record.state = OperationState::Tracking;
            self.record.detail = "Waiting for the submitted IPv6 management stage".into();
            self.save(j).await?;
            tokio::select! {_=c.cancelled()=>return Err(Error::Closed),_=tokio::time::sleep(Duration::from_millis(250))=>{}}
        }
    }
    async fn finish_ipv6_bond(
        &mut self,
        j: &mut Journal,
        s: &Session,
        c: &CancellationToken,
    ) -> Result<(), Error> {
        let mut p = self.ipv6_plan()?.clone();
        let result = tokio::select! {_=c.cancelled()=>return Err(Error::Closed),v=backend::identities(s,&mut p)=>v};
        self.set_ipv6_plan(p);
        self.record = j.ipv6_evidence(&self.record)?;
        self.announce().await;
        if self.stopped_after_recovery() {
            return Ok(());
        }
        result?;
        let mut p = self.ipv6_plan()?.clone();
        tokio::select! {_=c.cancelled()=>return Err(Error::Closed),v=backend::verify(s,&p,p.depth as usize+1)=>v?};
        self.ipv6_route(
            &p,
            p.depth as usize + 1,
            p.stage() == Stage::MoveToTarget,
            c,
        )
        .await?;
        p.current_mut().verified = true;
        if p.stage() == Stage::MoveToTarget {
            p.final_route_verified = true;
        }
        self.set_ipv6_plan(p);
        self.record = j.ipv6_evidence(&self.record)?;
        self.announce().await;
        if self.stopped_after_recovery() {
            return Ok(());
        }
        let done = self.ipv6_plan()?.depth == 4;
        self.record.state = if done {
            OperationState::Succeeded
        } else {
            OperationState::PartiallyCompleted
        };
        self.record.progress = Some((self.ipv6_plan()?.depth as f64 + 1.0) / 5.0);
        self.record.can_cancel = false;
        self.record.detail=if done{"IPv6 management bond handover and final host route verified. The temporary interface remains configured."}else{"This handover stage and authenticated host route are verified. The pool remains reserved. Review Continue for the next stage; the temporary interface remains configured."}.into();
        self.save(j).await?;
        if !c.is_cancelled()
            && let (Some(task), Some(expected)) = (
                self.ipv6_plan()?.current().task.clone(),
                self.ipv6_plan()?.current().task_uuid,
            )
        {
            let _ = tokio::time::timeout(Duration::from_secs(2), async {
                let v = s.call("task.get_record", vec![json!(&task)]).await?;
                if crate::resources::network::uuid(&v)? == expected.uuid() {
                    s.call("task.destroy", vec![json!(task)]).await?;
                }
                Ok::<_, Error>(())
            })
            .await;
        }
        Ok(())
    }
}
