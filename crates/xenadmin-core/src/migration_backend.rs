use crate::{
    Error, HostId, NetworkId, SrId, VbdId, VdiId, VifId, VmId, VmMigration, snapshots,
    xapi::Session,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;
use xenadmin_xapi::records::{HostRecord, PbdRecord, PifRecord, SrRecord, VdiRecord, VmRecord};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Disk {
    pub id: VdiId,
    pub sr: SrId,
    pub size: u64,
    pub kind: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Attachment {
    pub id: VbdId,
    pub disk: Option<Disk>,
    pub kind: String,
    pub mode: String,
    pub device: String,
    pub owner: bool,
    pub bootable: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Nic {
    pub id: VifId,
    pub network: NetworkId,
    pub device: String,
    pub mac: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Graph {
    pub disks: Vec<Attachment>,
    pub nics: Vec<Nic>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Plan {
    pub vm: VmId,
    pub operation: VmMigration,
    pub graph: Graph,
    pub baseline: Vec<Uuid>,
    pub completed: bool,
    #[serde(default)]
    pub interrupted: bool,
    #[serde(default)]
    pub outcome: Option<TaskOutcome>,
    #[serde(default)]
    pub affinity_received: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum TaskOutcome {
    Success,
    Failure(String),
    Cancelled,
}
impl Plan {
    pub(crate) fn extends(&self, old: &Self) -> bool {
        self.vm == old.vm
            && self.operation == old.operation
            && self.graph == old.graph
            && self.baseline == old.baseline
            && (!old.completed || self.completed)
            && (!old.interrupted || self.interrupted)
            && (!old.affinity_received || self.affinity_received)
            && old
                .outcome
                .as_ref()
                .is_none_or(|outcome| self.outcome.as_ref() == Some(outcome))
    }
    pub(crate) fn valid(&self, record: &crate::OperationRecord) -> bool {
        let valid_text = |s: &str| s.len() <= 1024 && !s.chars().any(char::is_control);
        record.vm == self.vm
            && record.action == crate::OperationAction::Migration(self.operation.clone())
            && self.operation.validate().is_ok()
            && self.graph.disks.len() <= 256
            && self.graph.nics.len() <= 256
            && self.graph.disks.windows(2).all(|w| w[0].id < w[1].id)
            && self.graph.nics.windows(2).all(|w| w[0].id < w[1].id)
            && matches_review(&self.graph, &self.operation)
            && self.graph.disks.iter().all(|d| {
                valid_text(&d.kind)
                    && valid_text(&d.mode)
                    && valid_text(&d.device)
                    && d.disk.as_ref().is_none_or(|d| valid_text(&d.kind))
            })
            && self
                .graph
                .nics
                .iter()
                .all(|n| valid_text(&n.device) && valid_text(&n.mac))
            && self.baseline.len() <= 10_000
            && self.baseline.windows(2).all(|w| w[0] < w[1])
            && (!self.operation.storage_changes()
                || self
                    .graph
                    .disks
                    .iter()
                    .filter_map(|d| d.disk.as_ref())
                    .all(|d| self.baseline.contains(&d.id.uuid())))
            && self.outcome.as_ref().is_none_or(|outcome| match outcome {
                TaskOutcome::Success => self.completed,
                TaskOutcome::Failure(code) => {
                    !self.completed && crate::xapi::safe_error_code(code) == *code
                }
                TaskOutcome::Cancelled => !self.completed,
            })
            && (!self.affinity_received || (!self.operation.live && self.completed))
            && (record
                .steps
                .first()
                .is_none_or(|step| step.state != crate::StepState::Verified)
                || self.completed
                || self.outcome.is_some())
            && record.steps.len() == if self.operation.live { 1 } else { 2 }
            && record.steps[0].label == "Migrate VM and reviewed devices"
            && (self.operation.live || record.steps[1].label == "Set home host")
            && (record.state != crate::OperationState::Succeeded
                || self.completed
                    && record
                        .steps
                        .iter()
                        .all(|step| step.state == crate::StepState::Verified))
    }
}
pub(crate) fn valid_record(record: &crate::OperationRecord, plan: Option<&Plan>) -> bool {
    match plan {
        Some(plan) => plan.valid(record),
        None => {
            !matches!(record.action, crate::OperationAction::Migration(_))
                || record.steps.is_empty() && record.state != crate::OperationState::Succeeded
        }
    }
}
pub(crate) fn valid_receipts(record: &crate::OperationRecord, plan: Option<&Plan>) -> bool {
    let Some(plan) = plan else {
        return true;
    };
    (!(plan.completed||plan.outcome.is_some()||plan.affinity_received)||record.task.is_some())
        && record.created_devices.iter().all(|created| matches!(created,crate::CreatedDevice::Disk(id) if plan.operation.storage_changes() && !plan.baseline.contains(&id.uuid())))
}
pub(crate) fn id<T: From<Uuid>>(r: &Value) -> Result<T, Error> {
    r["uuid"]
        .as_str()
        .and_then(|s| s.parse::<Uuid>().ok())
        .map(T::from)
        .ok_or(Error::MigrationConflict)
}
pub(crate) fn refs(r: &Value, key: &str) -> Result<Vec<String>, Error> {
    let a = r[key]
        .as_array()
        .filter(|a| a.len() <= 256)
        .ok_or(Error::MigrationConflict)?;
    let values = a
        .iter()
        .map(snapshots::reference)
        .collect::<Result<Vec<_>, _>>()?;
    if values.iter().collect::<BTreeSet<_>>().len() != values.len() {
        return Err(Error::MigrationConflict);
    }
    Ok(values)
}
pub(crate) fn text(r: &Value, key: &str) -> Result<String, Error> {
    r[key]
        .as_str()
        .filter(|s| s.len() <= 256)
        .map(str::to_owned)
        .ok_or(Error::MigrationConflict)
}
pub(crate) fn number(r: &Value, key: &str) -> Result<u64, Error> {
    r[key]
        .as_u64()
        .or_else(|| r[key].as_str().and_then(|s| s.parse().ok()))
        .ok_or(Error::MigrationConflict)
}
pub(crate) fn idle(r: &Value) -> Result<(), Error> {
    if r["current_operations"]
        .as_object()
        .is_some_and(|m| m.is_empty())
    {
        Ok(())
    } else {
        Err(Error::Busy)
    }
}
pub(crate) async fn fresh(
    session: &Session,
    class: &str,
    uuid: Uuid,
) -> Result<(String, Value), Error> {
    snapshots::optional_record(session, class, &uuid.to_string())
        .await?
        .ok_or(Error::MigrationConflict)
}
pub(crate) async fn read(session: &Session, class: &str, reference: &str) -> Result<Value, Error> {
    snapshots::reference(&json!(reference))?;
    session
        .call(&format!("{class}.get_record"), vec![json!(reference)])
        .await
}
pub(crate) async fn nullable_host(
    session: &Session,
    value: &Value,
) -> Result<Option<HostId>, Error> {
    if value == "OpaqueRef:NULL" {
        Ok(None)
    } else {
        Ok(Some(id(&read(
            session,
            "host",
            &snapshots::reference(value)?,
        )
        .await?)?))
    }
}
async fn guest(session: &Session, plan: &Plan, original: bool) -> Result<(String, Value), Error> {
    let (reference, r) = fresh(session, "VM", plan.vm.uuid()).await?;
    if VmRecord::new(&r).is_a_template() != false
        || VmRecord::new(&r).is_a_snapshot() != false
        || VmRecord::new(&r).is_control_domain() != false
        || VmRecord::new(&r).power_state()
            != if plan.operation.live {
                "Running"
            } else {
                "Halted"
            }
    {
        return Err(Error::MigrationConflict);
    }
    idle(&r)?;
    let host = nullable_host(session, &VmRecord::new(&r).resident_on()).await?;
    if host
        != if original {
            plan.operation.expected_host
        } else if plan.operation.live {
            Some(plan.operation.destination)
        } else {
            None
        }
    {
        return Err(Error::MigrationConflict);
    }
    if original
        && !plan.operation.live
        && nullable_host(session, &VmRecord::new(&r).affinity()).await?
            != plan.operation.expected_affinity
    {
        return Err(Error::MigrationConflict);
    }
    if original && !crate::xapi::advertises(&r, plan.operation.capability()) {
        return Err(Error::UnavailableAction);
    }
    if plan.operation.uses_storage_transport() && !refs(&r, "snapshots")?.is_empty() {
        return Err(Error::UnsupportedMigration);
    }
    Ok((reference, r))
}
pub(crate) async fn graph(session: &Session, vmref: &str, r: &Value) -> Result<Graph, Error> {
    let mut disks = vec![];
    for reference in refs(r, "VBDs")? {
        let v = read(session, "VBD", &reference).await?;
        idle(&v)?;
        if v["VM"] != vmref {
            return Err(Error::MigrationConflict);
        }
        let kind = text(&v, "type")?;
        if kind != "Disk" && kind != "CD" {
            return Err(Error::UnsupportedMigration);
        }
        let disk = if v["empty"] == true {
            if v["VDI"] != "OpaqueRef:NULL" || kind != "CD" {
                return Err(Error::MigrationConflict);
            }
            None
        } else if v["empty"] == false {
            let d = read(session, "VDI", &snapshots::reference(&v["VDI"])?).await?;
            idle(&d)?;
            if VdiRecord::new(&d).missing() != false || VdiRecord::new(&d).managed() != true {
                return Err(Error::UnsupportedMigration);
            }
            let sr = read(
                session,
                "SR",
                &snapshots::reference(&VdiRecord::new(&d).sr())?,
            )
            .await?;
            Some(Disk {
                id: id(&d)?,
                sr: id(&sr)?,
                size: number(&d, "virtual_size")?,
                kind: text(&d, "type")?,
            })
        } else {
            return Err(Error::MigrationConflict);
        };
        disks.push(Attachment {
            id: id(&v)?,
            disk,
            kind,
            mode: text(&v, "mode")?,
            device: text(&v, "userdevice")?,
            owner: v["other_config"]["owner"].as_str().is_some(),
            bootable: v["bootable"].as_bool().ok_or(Error::MigrationConflict)?,
        });
    }
    let mut nics = vec![];
    for reference in refs(r, "VIFs")? {
        let v = read(session, "VIF", &reference).await?;
        idle(&v)?;
        if v["VM"] != vmref {
            return Err(Error::MigrationConflict);
        }
        let network = read(session, "network", &snapshots::reference(&v["network"])?).await?;
        nics.push(Nic {
            id: id(&v)?,
            network: id(&network)?,
            device: text(&v, "device")?,
            mac: text(&v, "MAC")?.to_ascii_lowercase(),
        });
    }
    disks.sort_by_key(|d| d.id);
    nics.sort_by_key(|n| n.id);
    if disks.iter().map(|d| d.id).collect::<BTreeSet<_>>().len() != disks.len()
        || nics.iter().map(|n| n.id).collect::<BTreeSet<_>>().len() != nics.len()
    {
        return Err(Error::MigrationConflict);
    }
    Ok(Graph { disks, nics })
}
fn matches_review(g: &Graph, o: &VmMigration) -> bool {
    let disks = g
        .disks
        .iter()
        .filter(|d| d.kind == "Disk")
        .filter_map(|a| a.disk.as_ref())
        .map(|d| (d.id, d.sr))
        .collect::<BTreeMap<_, _>>();
    disks == o.disks.iter().map(|d| (d.vdi, d.expected_sr)).collect()
        && g.nics
            .iter()
            .map(|n| (n.id, n.network))
            .collect::<BTreeMap<_, _>>()
            == o.networks
                .iter()
                .map(|n| (n.vif, n.expected_network))
                .collect()
}
pub(crate) async fn host(session: &Session, target: HostId) -> Result<String, Error> {
    let (reference, h) = fresh(session, "host", target.uuid()).await?;
    idle(&h)?;
    if HostRecord::new(&h).enabled() != true {
        return Err(Error::UnavailableAction);
    }
    let metrics = snapshots::reference(&HostRecord::new(&h).metrics())?;
    let m = read(session, "host_metrics", &metrics).await?;
    if m["live"] != true {
        return Err(Error::UnavailableAction);
    }
    Ok(reference)
}
pub(crate) async fn sr_visible(
    session: &Session,
    sr: SrId,
    hostref: &str,
    write: bool,
) -> Result<String, Error> {
    let (reference, r) = fresh(session, "SR", sr.uuid()).await?;
    idle(&r)?;
    if write
        && (SrRecord::new(&r).content_type() != "user"
            || !crate::xapi::advertises(&r, "vdi_create"))
    {
        return Err(Error::UnavailableAction);
    }
    let mut visible = false;
    for p in refs(&r, "PBDs")? {
        let p = read(session, "PBD", &p).await?;
        if PbdRecord::new(&p).sr() != reference {
            return Err(Error::MigrationConflict);
        }
        if PbdRecord::new(&p).host() == hostref && PbdRecord::new(&p).currently_attached() == true {
            visible = true
        }
    }
    if !visible {
        return Err(Error::UnavailableAction);
    }
    Ok(reference)
}
pub(crate) async fn network_visible(
    session: &Session,
    network: NetworkId,
    hostref: &str,
    transfer: bool,
) -> Result<String, Error> {
    let (reference, r) = fresh(session, "network", network.uuid()).await?;
    idle(&r)?;
    let pifs = refs(&r, "PIFs")?;
    let mut visible = pifs.is_empty() && !transfer;
    for p in pifs {
        let p = read(session, "PIF", &p).await?;
        if PifRecord::new(&p).network() != reference {
            return Err(Error::MigrationConflict);
        }
        if PifRecord::new(&p).host() == hostref
            && PifRecord::new(&p).currently_attached() == true
            && (!transfer
                || PifRecord::new(&p).ip().as_str().is_some_and(|s| {
                    s.parse::<std::net::IpAddr>()
                        .is_ok_and(|a| !a.is_unspecified() && !a.is_loopback())
                }))
        {
            visible = true
        }
    }
    if !visible {
        return Err(Error::UnavailableAction);
    }
    Ok(reference)
}
async fn validate_destinations(
    session: &Session,
    plan: &Plan,
    hostref: &str,
) -> Result<(Value, Value), Error> {
    let mut disks = serde_json::Map::new();
    for attachment in &plan.graph.disks {
        if let Some(d) = &attachment.disk {
            let (diskref, r) = fresh(session, "VDI", d.id.uuid()).await?;
            idle(&r)?;
            let placement = plan.operation.disks.iter().find(|p| p.vdi == d.id);
            let destination = placement.map_or(d.sr, |p| p.destination_sr);
            let moving = destination != d.sr;
            if moving {
                if attachment.kind != "Disk"
                    || d.kind != "user"
                    || VdiRecord::new(&r).read_only() != false
                    || VdiRecord::new(&r).sharable() != false
                    || VdiRecord::new(&r).is_a_snapshot() != false
                    || VdiRecord::new(&r).cbt_enabled() != false
                    || refs(&r, "VBDs")?.len() != 1
                {
                    return Err(Error::UnsupportedMigration);
                }
                let only = refs(&r, "VBDs")?.remove(0);
                let v = read(session, "VBD", &only).await?;
                if id::<VbdId>(&v)? != attachment.id {
                    return Err(Error::MigrationConflict);
                }
            }
            let sr = sr_visible(session, destination, hostref, moving).await?;
            disks.insert(diskref, json!(sr));
        }
    }
    // XCP-ng 8.3.0 refuses any VIF entry in a same-pool migrate_send map, even one that
    // names the interface's current network ("VIF mapping is not allowed for intra-pool
    // migration"). Send identity placements as an empty map and refuse a real network
    // change up front; cross-pool moves carry their mappings separately.
    let nics = serde_json::Map::new();
    for p in &plan.operation.networks {
        let (_, vif) = fresh(session, "VIF", p.vif.uuid()).await?;
        let network = network_visible(session, p.destination_network, hostref, false).await?;
        if vif["network"] != network {
            return Err(Error::UnsupportedMigration);
        }
    }
    Ok((Value::Object(disks), Value::Object(nics)))
}
/// Receiver credentials are deliberately held only by this transient dispatch value.
pub(crate) struct Prepared {
    method: &'static str,
    args: Vec<Value>,
}
impl Prepared {
    pub async fn send(self, session: &Session) -> Result<Value, Error> {
        session.call(self.method, self.args).await
    }
}
pub(crate) struct Preflight {
    plan: Plan,
    vmref: String,
    hostref: String,
    disks: Value,
    nics: Value,
    transfer: Option<String>,
}
/// No receiver token is acquired while checking a whole selection.
pub(crate) async fn preflight(
    session: &Session,
    vm: VmId,
    operation: &VmMigration,
) -> Result<Preflight, Error> {
    operation.validate()?;
    operation
        .permission_requirements()
        .guard(session.permissions())?;
    let mut plan = Plan {
        vm,
        operation: operation.clone(),
        graph: Graph {
            disks: vec![],
            nics: vec![],
        },
        baseline: vec![],
        completed: false,
        interrupted: false,
        outcome: None,
        affinity_received: false,
    };
    let (vmref, r) = guest(session, &plan, true).await?;
    plan.graph = graph(session, &vmref, &r).await?;
    if !matches_review(&plan.graph, operation) {
        return Err(Error::MigrationConflict);
    }
    let hostref = host(session, operation.destination).await?;
    let (disks, nics) = validate_destinations(session, &plan, &hostref).await?;
    let transfer = if operation.uses_storage_transport() {
        Some(
            network_visible(
                session,
                operation.transfer_network.ok_or(Error::InvalidMigration)?,
                &hostref,
                true,
            )
            .await?,
        )
    } else {
        session
            .call(
                "VM.assert_can_boot_here",
                vec![json!(&vmref), json!(&hostref)],
            )
            .await?;
        None
    };
    if operation.storage_changes() {
        plan.baseline = crate::device_lifecycle::baseline(session, "VDI").await?;
    }
    let preflight = Preflight {
        plan,
        vmref,
        hostref,
        disks,
        nics,
        transfer,
    };
    preflight.revalidate(session).await?;
    Ok(preflight)
}
impl Preflight {
    async fn revalidate(&self, session: &Session) -> Result<(), Error> {
        self.plan
            .operation
            .permission_requirements()
            .guard(session.permissions())?;
        let (current, r) = guest(session, &self.plan, true).await?;
        if current != self.vmref
            || graph(session, &current, &r).await? != self.plan.graph
            || host(session, self.plan.operation.destination).await? != self.hostref
        {
            return Err(Error::MigrationConflict);
        }
        let (disks, nics) = validate_destinations(session, &self.plan, &self.hostref).await?;
        if disks != self.disks || nics != self.nics {
            return Err(Error::MigrationConflict);
        }
        if let Some(expected) = &self.transfer
            && network_visible(
                session,
                self.plan
                    .operation
                    .transfer_network
                    .ok_or(Error::InvalidMigration)?,
                &self.hostref,
                true,
            )
            .await?
                != *expected
        {
            return Err(Error::MigrationConflict);
        }
        Ok(())
    }
    async fn prepare(self, session: &Session) -> Result<(Plan, Prepared), Error> {
        self.revalidate(session).await?;
        let operation = &self.plan.operation;
        let prepared = if let Some(transfer) = &self.transfer {
            let token = session
                .call(
                    "host.migrate_receive",
                    vec![json!(&self.hostref), json!(transfer), json!({})],
                )
                .await?;
            let map = token
                .as_object()
                .filter(|m| {
                    m.len() <= 32
                        && m.values()
                            .all(|v| v.as_str().is_some_and(|s| s.len() <= 16384))
                })
                .ok_or(Error::Protocol("invalid migration receiver token"))?;
            if map.get("host") != Some(&json!(&self.hostref)) {
                return Err(Error::Protocol("migration receiver host mismatch"));
            }
            let args = vec![
                json!(&self.vmref),
                token,
                json!(operation.live),
                self.disks.clone(),
                self.nics.clone(),
                json!({"live":if operation.live{"true"}else{"false"}}),
            ];
            session.call("VM.assert_can_migrate", args.clone()).await?;
            Prepared {
                method: "Async.VM.migrate_send",
                args,
            }
        } else {
            Prepared {
                method: "Async.VM.pool_migrate",
                args: vec![
                    json!(&self.vmref),
                    json!(&self.hostref),
                    json!({"live":"true"}),
                ],
            }
        };
        self.revalidate(session).await?;
        Ok((self.plan, prepared))
    }
}
pub(crate) async fn prepare(
    session: &Session,
    vm: VmId,
    operation: &VmMigration,
) -> Result<(Plan, Prepared), Error> {
    preflight(session, vm, operation)
        .await?
        .prepare(session)
        .await
}
pub(crate) async fn verify(
    session: &Session,
    plan: &Plan,
    result: Option<&str>,
) -> Result<Vec<VdiId>, Error> {
    let (vmref, r) = guest(session, plan, false).await?;
    if let Some(result) = result
        && (result != vmref || id::<VmId>(&read(session, "VM", result).await?)? != plan.vm)
    {
        return Err(Error::MigrationConflict);
    }
    let after = graph(session, &vmref, &r).await?;
    if after.disks.len() != plan.graph.disks.len() || after.nics.len() != plan.graph.nics.len() {
        return Err(Error::MigrationConflict);
    }
    let mut created = vec![];
    for (old, new) in plan.graph.disks.iter().zip(&after.disks) {
        let mut expected = old.clone();
        if let Some(d) = &old.disk {
            let placement = plan.operation.disks.iter().find(|p| p.vdi == d.id);
            if let Some(p) = placement
                && p.expected_sr != p.destination_sr
            {
                let newdisk = new.disk.as_ref().ok_or(Error::MigrationConflict)?;
                if newdisk.id == d.id
                    || plan.baseline.is_empty()
                    || plan.baseline.contains(&newdisk.id.uuid())
                    || newdisk.sr != p.destination_sr
                    || newdisk.size != d.size
                    || newdisk.kind != d.kind
                {
                    return Err(Error::MigrationConflict);
                }
                let (_, r) = fresh(session, "VDI", newdisk.id.uuid()).await?;
                if VdiRecord::new(&r).sharable() != false
                    || VdiRecord::new(&r).read_only() != false
                    || VdiRecord::new(&r).is_a_snapshot() != false
                    || refs(&r, "VBDs")?.len() != 1
                {
                    return Err(Error::MigrationConflict);
                }
                let v = read(session, "VBD", &refs(&r, "VBDs")?[0]).await?;
                if id::<VbdId>(&v)? != old.id {
                    return Err(Error::MigrationConflict);
                }
                expected.disk = Some(newdisk.clone());
                created.push(newdisk.id);
            }
        }
        if expected != *new {
            return Err(Error::MigrationConflict);
        }
    }
    for (old, new) in plan.graph.nics.iter().zip(&after.nics) {
        let mut expected = old.clone();
        expected.network = plan
            .operation
            .networks
            .iter()
            .find(|p| p.vif == old.id)
            .ok_or(Error::MigrationConflict)?
            .destination_network;
        if expected != *new {
            return Err(Error::MigrationConflict);
        }
    }
    if created.iter().collect::<BTreeSet<_>>().len() != created.len() {
        return Err(Error::MigrationConflict);
    }
    Ok(created)
}
pub(crate) async fn unchanged(session: &Session, plan: &Plan) -> Result<bool, Error> {
    let (reference, r) = guest(session, plan, true).await?;
    Ok(graph(session, &reference, &r).await? == plan.graph)
}
pub(crate) async fn affinity_matches(session: &Session, plan: &Plan) -> Result<bool, Error> {
    let (_, r) = guest(session, plan, false).await?;
    Ok(nullable_host(session, &VmRecord::new(&r).affinity()).await?
        == Some(plan.operation.destination))
}
pub(crate) async fn prepare_affinity(session: &Session, plan: &Plan) -> Result<Prepared, Error> {
    verify(session, plan, None).await?;
    let (vmref, r) = guest(session, plan, false).await?;
    if nullable_host(session, &VmRecord::new(&r).affinity()).await?
        != plan.operation.expected_affinity
    {
        return Err(Error::MigrationConflict);
    }
    let hostref = host(session, plan.operation.destination).await?;
    Ok(Prepared {
        method: "VM.set_affinity",
        args: vec![json!(vmref), json!(hostref)],
    })
}
/// Record individually verified replacement disks even when another part of migration failed.
pub(crate) async fn observed_disks(session: &Session, plan: &Plan) -> Result<Vec<VdiId>, Error> {
    let (vmref, r) = fresh(session, "VM", plan.vm.uuid()).await?;
    if r["is_a_template"] != false || r["is_a_snapshot"] != false || r["is_control_domain"] != false
    {
        return Err(Error::MigrationConflict);
    }
    idle(&r)?;
    let current = graph(session, &vmref, &r).await?;
    let mut observed = vec![];
    for old in &plan.graph.disks {
        let Some(old_disk) = &old.disk else { continue };
        let Some(p) = plan
            .operation
            .disks
            .iter()
            .find(|p| p.vdi == old_disk.id && p.destination_sr != p.expected_sr)
        else {
            continue;
        };
        let Some(new) = current.disks.iter().find(|a| a.id == old.id) else {
            continue;
        };
        let Some(d) = &new.disk else { continue };
        let mut expected = old.clone();
        expected.disk = Some(d.clone());
        if expected != *new
            || d.id == old_disk.id
            || plan.baseline.is_empty()
            || plan.baseline.contains(&d.id.uuid())
            || d.sr != p.destination_sr
            || d.size != old_disk.size
            || d.kind != old_disk.kind
        {
            continue;
        }
        let (_, r) = fresh(session, "VDI", d.id.uuid()).await?;
        if r["sharable"] != false
            || r["read_only"] != false
            || r["is_a_snapshot"] != false
            || refs(&r, "VBDs")?.len() != 1
        {
            continue;
        }
        let v = read(session, "VBD", &refs(&r, "VBDs")?[0]).await?;
        if id::<VbdId>(&v)? == old.id {
            observed.push(d.id);
        }
    }
    observed.sort();
    observed.dedup();
    Ok(observed)
}
