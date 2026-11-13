use super::*;
use crate::{
    HostId, PoolId, PoolOperation, ResourceAction, ResourceTarget, security_discovery as discovery,
};
use serde_json::Value;
use std::collections::BTreeSet;
use xenadmin_xapi::enumerations::vm_power_state;
use xenadmin_xapi::records::{HostMetricsRecord, HostRecord, PoolRecord, SrRecord, VmRecord};
use xenadmin_xapi::{calls, refs};

struct JoinSpec {
    source: PoolId,
    source_host: HostId,
    destination: PoolId,
    coordinator: HostId,
    address: String,
}
impl JoinSpec {
    fn from(record: &ResourceOperationRecord) -> Result<Self, Error> {
        let ResourceAction::Pool(PoolOperation::Join {
            source_host,
            destination_pool,
            destination_coordinator,
            destination_address,
        }) = &record.action
        else {
            return Err(Error::InvalidResourceCommand);
        };
        record.action.validate(record.target)?;
        Ok(Self {
            source: record.pool,
            source_host: *source_host,
            destination: *destination_pool,
            coordinator: *destination_coordinator,
            address: destination_address.clone(),
        })
    }
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
fn require(condition: bool, message: &'static str) -> Result<(), Error> {
    if condition {
        Ok(())
    } else {
        Err(Error::Protocol(message))
    }
}
fn idle(record: &Value) -> Result<(), Error> {
    require(
        record["current_operations"]
            .as_object()
            .is_some_and(|v| v.is_empty()),
        "Pool join requires idle hosts and resources",
    )
}
async fn pool(session: &Session, id: PoolId) -> Result<Value, Error> {
    let records = discovery::records(session, "pool", 1).await?;
    let record = records.values().next().ok_or(Error::Stale)?;
    require(
        records.len() == 1 && PoolRecord::new(record).uuid() == id.to_string(),
        "Connected pool identity changed; reopen the join review",
    )?;
    Ok(record.clone())
}
async fn host_ids(
    session: &Session,
) -> Result<(BTreeSet<HostId>, serde_json::Map<String, Value>), Error> {
    let hosts = discovery::records(session, "host", 256).await?;
    let ids = hosts
        .values()
        .map(|host| {
            HostId::parse(
                HostRecord::new(host)
                    .uuid()
                    .text()
                    .ok_or(Error::Protocol("Host identity unavailable"))?,
            )
            .map_err(|_| Error::Protocol("Invalid host identity"))
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    require(
        !ids.is_empty() && ids.len() == hosts.len(),
        "Pool host membership is unavailable or ambiguous",
    )?;
    Ok((ids, hosts))
}
async fn live(session: &Session, host: &Value) -> Result<(), Error> {
    let reference = xapi::reply_reference(HostRecord::new(host).metrics())?;
    require(
        HostMetricsRecord::new(
            &session
                .send(calls::host_metrics::GetRecord(refs::host_metrics(
                    &reference,
                )))
                .await?,
        )
        .live()
            == true,
        "Pool join requires live hosts",
    )
}
async fn destination(session: &Session, spec: &JoinSpec) -> Result<Vec<HostId>, Error> {
    let record = pool(session, spec.destination).await?;
    require(
        PoolRecord::new(&record).ha_enabled() == false,
        "Disable HA separately before reviewing a pool join",
    )?;
    idle(&record)?;
    let (ids, hosts) = host_ids(session).await?;
    let master = xapi::reply_reference(PoolRecord::new(&record).master())?;
    let host = hosts.get(&master).ok_or(Error::Stale)?;
    require(
        HostRecord::new(host).uuid() == spec.coordinator.to_string()
            && HostRecord::new(host).address() == spec.address,
        "Destination coordinator or address changed; reopen the join review",
    )?;
    require(
        HostRecord::new(host).enabled() == true,
        "Destination coordinator must be enabled",
    )?;
    idle(host)?;
    live(session, host).await?;
    Ok(ids.into_iter().collect())
}
async fn source(session: &Session, spec: &JoinSpec) -> Result<(), Error> {
    let record = pool(session, spec.source).await?;
    require(
        PoolRecord::new(&record).ha_enabled() == false,
        "Disable HA separately before reviewing a pool join",
    )?;
    idle(&record)?;
    let (ids, hosts) = host_ids(session).await?;
    require(
        ids.len() == 1 && ids.contains(&spec.source_host),
        "Only a standalone source host can join another pool",
    )?;
    let host = hosts.values().next().ok_or(Error::Stale)?;
    idle(host)?;
    live(session, host).await?;
    require(
        discovery::local_superuser(session).await?,
        "Reconnect the source with a local superuser before joining a pool",
    )?;
    require(
        discovery::advertised(session, "pool.join").await? != Some(false),
        "This host does not advertise pool joining",
    )?;
    for vm in discovery::records(session, "VM", 10_000).await?.values() {
        match VmRecord::new(vm).is_control_domain().get() {
            Some(true) => {}
            Some(false) => {
                require(
                    VmRecord::new(vm).power_state() == vm_power_state::HALTED
                        && VmRecord::new(vm).scheduled_to_be_resident_on() == "OpaqueRef:NULL",
                    "All source guests must already be halted and unscheduled before joining",
                )?;
                idle(vm)?;
            }
            None => return Err(Error::Protocol("Source guest classification unavailable")),
        }
    }
    for sr in discovery::records(session, "SR", 10_000).await?.values() {
        require(
            SrRecord::new(sr).shared() == false
                || (SrRecord::new(sr).shared() == true && SrRecord::new(sr).is_tools_sr() == true),
            "Detach shared storage separately before reviewing a pool join",
        )?;
        idle(sr)?;
    }
    for class in ["Bond", "tunnel"] {
        require(
            discovery::records(session, class, 4096).await?.is_empty(),
            "Remove source bonds and tunnels separately before reviewing a pool join",
        )?;
    }
    Ok(())
}
fn peer_current(
    inputs: &crate::pool_join::Inputs,
    peer: &crate::connection::JoinPeer,
) -> Result<(), Error> {
    if inputs.destination.cancel.is_cancelled() || peer.cancel.is_cancelled() {
        return Err(Error::Closed);
    }
    require(
        peer.receiver
            .borrow()
            .observe_resource(inputs.observation.target())
            .is_ok_and(|v| v == inputs.observation),
        "Destination connection changed; reopen the join review",
    )
}
async fn verify(session: &Session, record: &ResourceOperationRecord) -> Result<(), Error> {
    let spec = JoinSpec::from(record)?;
    require(
        record.pool_completed,
        "Pool join reply was not recorded; membership alone cannot establish completion. No join will be replayed",
    )?;
    let baseline = record
        .join_baseline
        .as_ref()
        .filter(|v| !v.is_empty() && v.len() <= 256)
        .ok_or(Error::Protocol(
            "Destination membership baseline unavailable",
        ))?;
    let mut expected: BTreeSet<_> = baseline.iter().copied().collect();
    require(
        expected.len() == baseline.len() && expected.insert(spec.source_host),
        "Destination membership baseline is invalid",
    )?;
    let actual: BTreeSet<_> = destination(session, &spec).await?.into_iter().collect();
    require(
        actual == expected,
        "Destination membership does not yet match the reviewed join; recheck through the destination connection",
    )?;
    // A member restarts its toolstack right after joining and reports itself not live
    // for a short window. Keep re-reading until the caller's timeout expires; a final
    // negative read still yields an unknown outcome that a later recheck can resolve.
    loop {
        let (_, host) = xapi::read(session, ResourceTarget::Host(spec.source_host)).await?;
        idle(&host)?;
        match live(session, &host).await {
            Ok(()) => return Ok(()),
            Err(Error::Protocol(_)) => tokio::time::sleep(Duration::from_secs(2)).await,
            Err(error) => return Err(error),
        }
    }
}
impl ResourceRequest {
    pub(super) async fn execute_join(
        &mut self,
        journal: &mut Journal,
        session: &Session,
        cancel: &CancellationToken,
    ) -> Result<(), Error> {
        let inputs = self.join.take().ok_or(Error::InvalidResourceCommand)?;
        let spec = JoinSpec::from(&self.record)?;
        let peer = tokio::select! {_=cancel.cancelled()=>return Err(Error::Closed),r=inputs.destination.get(inputs.observation)=>r?};
        peer_current(&inputs, &peer)?;
        self.record.detail = "Checking both pools and validating destination credentials".into();
        self.save(journal).await?;
        let auth = tokio::select! {
            _=cancel.cancelled()=>return Err(Error::Closed),
            _=inputs.destination.cancel.cancelled()=>return Err(Error::Closed),
            _=peer.cancel.cancelled()=>return Err(Error::Stale),
            r=tokio::time::timeout(Duration::from_secs(45),Session::login_exact(peer.session.options(),&inputs.credentials))=>TemporarySession(Arc::new(r.map_err(|_|Error::Timeout)??)),
        };
        let prepare = async {
            peer_current(&inputs, &peer)?;
            require(
                discovery::local_superuser(&auth.0).await?,
                "Destination credentials must identify a local superuser",
            )?;
            let baseline = destination(&auth.0, &spec).await?;
            require(
                !baseline.contains(&spec.source_host),
                "Source host already belongs to the destination pool",
            )?;
            require(
                destination(&peer.session, &spec).await? == baseline,
                "Destination membership changed while validating credentials",
            )?;
            source(session, &spec).await?;
            Ok::<_, Error>(baseline)
        };
        self.record.join_baseline = Some(tokio::select! {
            _=cancel.cancelled()=>return Err(Error::Closed),
            _=inputs.destination.cancel.cancelled()=>return Err(Error::Closed),
            _=peer.cancel.cancelled()=>return Err(Error::Stale),
            r=tokio::time::timeout(Duration::from_secs(60),prepare)=>r.map_err(|_|Error::Timeout)??,
        });
        self.record.steps = vec![OperationStep {
            label: "Join destination pool".into(),
            state: StepState::Pending,
            detail: String::new(),
        }];
        self.save(journal).await?;
        // Check destination again immediately before admitting the write. Credentials and
        // the temporary session never enter the durable operation record.
        let baseline = tokio::select! {
            _=cancel.cancelled()=>return Err(Error::Closed),
            _=inputs.destination.cancel.cancelled()=>return Err(Error::Closed),
            _=peer.cancel.cancelled()=>return Err(Error::Stale),
            r=tokio::time::timeout(Duration::from_secs(45),destination(&auth.0,&spec))=>r.map_err(|_|Error::Timeout)??,
        };
        require(
            Some(&baseline) == self.record.join_baseline.as_ref(),
            "Destination membership changed; reopen the join review",
        )?;
        peer_current(&inputs, &peer)?;
        self.reload(journal)?;
        if self.stop_before_dispatch() {
            return self.save(journal).await;
        }
        if cancel.is_cancelled() {
            return Err(Error::Closed);
        }
        self.record.state = OperationState::Dispatching;
        self.record.can_cancel = false;
        self.record.steps[0].state = StepState::Dispatching;
        self.record.detail =
            "Joining the reviewed destination pool; interruption may leave an uncertain outcome"
                .into();
        self.save(journal).await?;
        tokio::select! {
            _=cancel.cancelled()=>return Err(Error::Closed),
            _=inputs.destination.cancel.cancelled()=>return Err(Error::Closed),
            _=peer.cancel.cancelled()=>return Err(Error::Stale),
            r=session.send(calls::pool::Join { master_address: &spec.address, master_username: &inputs.credentials.username, master_password: &inputs.credentials.password })=>{r?;},
        }
        self.record.pool_completed = true;
        self.record = journal.resource_evidence(&self.record)?;
        self.announce().await;
        if self.stopped_after_recovery() {
            return Ok(());
        }
        peer_current(&inputs, &peer)?;
        tokio::select! {
            _=inputs.destination.cancel.cancelled()=>return Err(Error::Closed),
            _=peer.cancel.cancelled()=>return Err(Error::Stale),
            r=tokio::time::timeout(Duration::from_secs(45),verify(&peer.session,&self.record))=>r.map_err(|_|Error::Timeout)??,
        }
        peer_current(&inputs, &peer)?;
        self.complete_join(journal).await
    }
    pub(super) async fn reconcile_join(
        &mut self,
        journal: &mut Journal,
        session: &Session,
        cancel: &CancellationToken,
    ) -> Result<(), Error> {
        self.reload(journal)?;
        if self.record.state.terminal() {
            return Ok(());
        }
        tokio::select! {_=cancel.cancelled()=>return Err(Error::Closed),r=tokio::time::timeout(Duration::from_secs(45),verify(session,&self.record))=>r.map_err(|_|Error::Timeout)??};
        self.complete_join(journal).await
    }
    async fn complete_join(&mut self, journal: &mut Journal) -> Result<(), Error> {
        self.reload(journal)?;
        if self.record.state.terminal() {
            return Ok(());
        }
        require(
            self.record.steps.len() == 1,
            "Pool join dispatch evidence unavailable",
        )?;
        self.record.steps[0].state = StepState::Verified;
        self.record.steps[0].detail =
            "Recorded join completion and verified destination host membership".into();
        self.record.state = OperationState::Succeeded;
        self.record.detail="Host joined the destination pool. The old standalone connection remains bound to its original pool identity.".into();
        self.record.can_cancel = false;
        self.save(journal).await
    }
}
