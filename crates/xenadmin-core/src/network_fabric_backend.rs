#[path = "network_fabric_pool_backend.rs"]
pub(super) mod pool;
use super::{network, xapi};
use crate::{
    Error, NetworkFabricChange as Change, NetworkFabricEvidence as Evidence,
    NetworkFabricKind as Kind, NetworkFabricObject as Object, NetworkFabricOperation as Operation,
    NetworkOperation, ResourceAction, ResourceOperationRecord as Record, ResourceTarget,
    xapi::Session,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;
use xenadmin_xapi::enumerations::{ip_configuration_mode, ipv6_configuration_mode};
use xenadmin_xapi::records::{
    HostMetricsRecord, HostRecord, NetworkRecord, NetworkSriovRecord, PifRecord, PoolRecord,
    VmRecord,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Plan {
    pool: crate::PoolId,
    network: crate::NetworkId,
    operation: Operation,
    baseline: BTreeMap<String, BTreeSet<Uuid>>,
    task_uuid: Option<crate::TaskId>,
    pub(crate) completed: bool,
    created: Option<Evidence>,
}
impl Plan {
    pub(crate) fn merge(&mut self, new: &Self) -> Result<(), Error> {
        if self.pool != new.pool
            || self.network != new.network
            || self.operation != new.operation
            || self.baseline != new.baseline
        {
            return Err(Error::JournalChanged);
        }
        if self.task_uuid.is_some() && new.task_uuid.is_some() && self.task_uuid != new.task_uuid
            || self.created.is_some() && new.created.is_some() && self.created != new.created
        {
            return Err(Error::JournalChanged);
        }
        self.task_uuid = self.task_uuid.or(new.task_uuid);
        self.completed |= new.completed;
        if self.created.is_none() {
            self.created = new.created.clone();
        }
        Ok(())
    }
    pub(crate) fn follows(&self, old: &Self) -> bool {
        let mut merged = old.clone();
        merged.merge(self).is_ok() && merged == *self
    }
}
pub(crate) fn validate_record(r: &Record) -> Result<(), Error> {
    let Some(op) = operation(r) else {
        return if r.network_fabric_plan.is_none() && r.network_fabric_created.is_none() {
            Ok(())
        } else {
            Err(Error::Journal)
        };
    };
    op.validate(r.target).map_err(|_| Error::Journal)?;
    if op.review.pool != r.pool {
        return Err(Error::Journal);
    }
    use crate::{OperationState as State, StepState};
    let before_dispatch = r.task.is_none()
        && r.result_reference.is_none()
        && r.network_fabric_created.is_none()
        && r.steps.is_empty()
        && matches!(
            r.state,
            State::Preparing
                | State::Failed
                | State::Cancelled
                | State::Rejected
                | State::OutcomeUnknown
                | State::AcknowledgedUnknown
                | State::Verifying
        );
    if r.steps.len() > 1 || r.steps.first().is_some_and(|step| step.label != op.label()) {
        return Err(Error::Journal);
    }

    let Some(p) = &r.network_fabric_plan else {
        return if before_dispatch {
            Ok(())
        } else {
            Err(Error::Journal)
        };
    };
    let class = class(op.review.kind).to_ascii_lowercase();
    if p.pool != r.pool
        || p.pool != op.review.pool
        || p.network != op.review.network
        || p.operation != *op
        || p.baseline.len() != 3
        || !["pif", "task", class.as_str()]
            .iter()
            .all(|c| p.baseline.get(*c).is_some_and(|v| v.len() <= 10_000))
        || p.task_uuid
            .is_some_and(|id| p.baseline["task"].contains(&id.uuid()))
        || p.completed
            && (p.task_uuid.is_none()
                || r.task.is_none()
                || op.creates() && r.result_reference.is_none())
        || r.steps.is_empty() && !before_dispatch
        || r.steps.iter().any(|step| step.state == StepState::Verified)
            && (!p.completed || op.creates() && p.created.is_none())
        || r.state == State::Succeeded
            && (r
                .steps
                .first()
                .is_none_or(|step| step.state != StepState::Verified)
                || !p.completed
                || op.creates()
                    && (p.created.is_none()
                        || r.created != p.created.as_ref().map(|e| e.object.target())))
        || !p.baseline.get("pif").is_some_and(|ids| {
            ids.contains(&op.review.input.id.uuid())
                && op.review.members.keys().all(|id| ids.contains(&id.uuid()))
        })
        || !p.baseline.get(&class).is_some_and(|ids| {
            op.review
                .members
                .values()
                .all(|m| ids.contains(&m.object.uuid()))
        })
        || p.task_uuid.is_some() && r.task.is_none()
        || r.result_reference.is_some() && (!op.creates() || !p.completed)
        || r.created
            .is_some_and(|target| p.created.as_ref().map(|e| e.object.target()) != Some(target))
        || p.created != r.network_fabric_created
    {
        return Err(Error::Journal);
    }
    if let Some(e) = &p.created
        && (!op.creates()
            || !p.completed
            || r.result_reference.is_none()
            || e.host != op.review.host
            || e.network != p.network
            || p.baseline["pif"].contains(&e.interface.uuid())
            || p.baseline[&class].contains(&e.object.uuid())
            || !matches!(
                (op.review.kind, e.object),
                (Kind::Tunnel(_), Object::Tunnel(_)) | (Kind::Sriov, Object::Sriov(_))
            ))
    {
        return Err(Error::Journal);
    }
    Ok(())
}
pub(crate) fn operation(r: &Record) -> Option<&Operation> {
    match &r.action {
        ResourceAction::Network(NetworkOperation::Fabric(op)) => Some(op),
        _ => None,
    }
}
fn class(k: Kind) -> &'static str {
    match k {
        Kind::Tunnel(_) => "Tunnel",
        Kind::Sriov => "Network_sriov",
    }
}
fn fields(k: Kind) -> (&'static str, &'static str, &'static str, &'static str) {
    match k {
        Kind::Tunnel(_) => (
            "transport_PIF",
            "access_PIF",
            "tunnel_transport_PIF_of",
            "tunnel_access_PIF_of",
        ),
        Kind::Sriov => (
            "physical_PIF",
            "logical_PIF",
            "sriov_physical_PIF_of",
            "sriov_logical_PIF_of",
        ),
    }
}
struct Fresh {
    raw: BTreeMap<String, BTreeMap<String, Value>>,
    inventory: crate::Inventory,
}
impl Fresh {
    async fn read(s: &Session, kind: Kind, creating: bool) -> Result<Self, Error> {
        let mut classes = vec![
            "pool",
            "host",
            "host_metrics",
            "network",
            "PIF",
            "VIF",
            "VM",
            "Bond",
            "VLAN",
            "Tunnel",
            "Network_sriov",
        ];
        match kind {
            Kind::Tunnel(_) if creating => classes.push("SDN_controller"),
            Kind::Tunnel(_) => {}
            Kind::Sriov => classes.push("PCI"),
        };
        let mut raw = BTreeMap::new();
        for c in classes {
            let map = network::all(s, c).await?;
            let ids = map
                .values()
                .map(network::uuid)
                .collect::<Result<BTreeSet<_>, _>>()?;
            if ids.len() != map.len() {
                return Err(Error::ResourceConflict);
            }
            raw.insert(c.to_ascii_lowercase(), map.into_iter().collect());
        }
        let inventory = crate::xapi::Store::network_fabric_inventory(raw.clone())?;
        Ok(Self { raw, inventory })
    }
    fn by_id(&self, class: &str, id: Uuid) -> Result<(&str, &Value), Error> {
        self.raw
            .get(&class.to_ascii_lowercase())
            .ok_or(Error::ResourceConflict)?
            .iter()
            .find(|(_, v)| network::uuid(v).ok() == Some(id))
            .map(|(k, v)| (k.as_str(), v))
            .ok_or(Error::ResourceConflict)
    }
    fn ref_ids(&self, class: &str, v: &Value) -> Result<BTreeSet<Uuid>, Error> {
        network::refs(v)?
            .iter()
            .map(|r| {
                network::uuid(
                    self.raw
                        .get(&class.to_ascii_lowercase())
                        .and_then(|m| m.get(r))
                        .ok_or(Error::ResourceConflict)?,
                )
            })
            .collect()
    }
    fn clean_child(&self, p: &Value, kind: Kind) -> Result<(), Error> {
        if PifRecord::new(p).management() != false
            || PifRecord::new(p).managed() != true
            || PifRecord::new(p).physical() != false
            || PifRecord::new(p).disallow_unplug() != false
            || PifRecord::new(p).ip_configuration_mode() != ip_configuration_mode::NONE
            || PifRecord::new(p).ipv6_configuration_mode() != ipv6_configuration_mode::NONE
            || ["IP", "netmask", "gateway", "DNS", "ipv6_gateway"]
                .iter()
                .any(|f| p[f] != "")
            || PifRecord::new(p)
                .ipv6()
                .as_array()
                .is_none_or(|ips| ips.iter().any(|ip| ip != ""))
            || network::number(&PifRecord::new(p).vlan()) != Some(-1)
            || network::refs(
                &p[match kind {
                    Kind::Tunnel(_) => "sriov_logical_PIF_of",
                    Kind::Sriov => "tunnel_access_PIF_of",
                }],
            )
            .map_or(true, |refs| !refs.is_empty())
            || PifRecord::new(p).bond_slave_of() != "OpaqueRef:NULL"
            || PifRecord::new(p).vlan_master_of() != "OpaqueRef:NULL"
            || [
                "bond_master_of",
                "VLAN_slave_of",
                "tunnel_transport_PIF_of",
                "sriov_physical_PIF_of",
            ]
            .iter()
            .any(|f| network::refs(&p[f]).map_or(true, |v| !v.is_empty()))
            || PifRecord::new(p)
                .other_config()
                .as_object()
                .is_none_or(|m| !m.is_empty())
            || PifRecord::new(p)
                .properties()
                .as_object()
                .is_none_or(|m| !m.is_empty())
        {
            return Err(Error::ResourceInUse);
        }
        Ok(())
    }
    fn graph(&self, op: &Operation) -> Result<(), Error> {
        let r = &op.review;
        if self.inventory.pool_id != r.pool {
            return Err(Error::ResourceConflict);
        }
        let (network_ref, n) = self.by_id("network", r.network.uuid())?;
        let actual = self.raw["pif"]
            .values()
            .filter(|p| p["network"] == network_ref)
            .map(network::uuid)
            .collect::<Result<BTreeSet<_>, _>>()?;
        if actual != self.ref_ids("PIF", &NetworkRecord::new(n).pifs())?
            || !network::refs(&NetworkRecord::new(n).vifs())?.is_empty()
            || self.raw["vif"]
                .values()
                .any(|v| v["network"] == network_ref)
        {
            return Err(Error::ResourceConflict);
        }
        let (input_ref, input) = self.by_id("PIF", r.input.id.uuid())?;
        let host_ref = self.by_id("host", r.host.uuid())?.0;
        if PifRecord::new(input).host() != host_ref {
            return Err(Error::ResourceConflict);
        }
        let (parent, child, parents, children) = fields(r.kind);
        for p in self.raw["pif"].values() {
            for (kind, field) in [
                (
                    Kind::Tunnel(crate::TunnelProtocol::Gre),
                    "tunnel_access_PIF_of",
                ),
                (Kind::Sriov, "sriov_logical_PIF_of"),
            ] {
                let (_, child, _, _) = fields(kind);
                let p_ref = self.by_id("PIF", network::uuid(p)?)?.0;
                let expected = self.raw[&class(kind).to_ascii_lowercase()]
                    .iter()
                    .filter(|(_, o)| o[child] == p_ref)
                    .map(|(k, _)| k.clone())
                    .collect::<BTreeSet<_>>();
                if network::refs(&p[field])? != expected {
                    return Err(Error::ResourceConflict);
                }
            }
        }
        let objects = &self.raw[&class(r.kind).to_ascii_lowercase()];
        let expected = objects
            .iter()
            .filter(|(_, o)| o[parent] == input_ref)
            .map(|(k, _)| k.clone())
            .collect::<BTreeSet<_>>();
        if network::refs(&input[parents])? != expected {
            return Err(Error::ResourceConflict);
        }
        for member in r.members.values() {
            let (object_ref, o) = self.by_id(class(r.kind), member.object.uuid())?;
            let (pif_ref, p) = self.by_id("PIF", member.interface.id.uuid())?;
            if o[child] != pif_ref
                || o[parent] != self.by_id("PIF", member.input.uuid())?.0
                || p["network"] != network_ref
                || network::refs(&p[children])? != [object_ref.to_string()].into()
            {
                return Err(Error::ResourceConflict);
            }
            if matches!(op.change,Change::Delete(id) if id==member.object) {
                self.clean_child(p, r.kind)?;
            }
        }
        if r.kind == Kind::Sriov {
            let allowed = match op.change {
                Change::Create => None,
                Change::Delete(id) => Some(id.uuid()),
            };
            for p in r.driver_interfaces.values() {
                let (pr, raw) = self.by_id("PIF", p.id.uuid())?;
                if PifRecord::new(raw)
                    .other_config()
                    .as_object()
                    .is_none_or(|m| !m.is_empty())
                    || PifRecord::new(raw)
                        .properties()
                        .as_object()
                        .is_none_or(|m| !m.is_empty())
                {
                    return Err(Error::ResourceInUse);
                }
                if self.raw["network_sriov"].values().any(|o| {
                    NetworkSriovRecord::new(o).physical_pif() == pr
                        && network::uuid(o).ok() != allowed
                }) {
                    return Err(Error::Protocol(
                        "Another same-driver SR-IOV interface is already configured; review it separately",
                    ));
                }
            }
            let (_, host) = self.by_id("host", r.host.uuid())?;
            if HostRecord::new(host).enabled() != false
                || HostRecord::new(host)
                    .current_operations()
                    .as_object()
                    .is_none_or(|v| !v.is_empty())
                || self.raw["vm"].values().any(|v| {
                    VmRecord::new(v).resident_on() == host_ref
                        && VmRecord::new(v).is_control_domain() != true
                })
                || self.raw["pool"]
                    .values()
                    .any(|p| PoolRecord::new(p).ha_enabled() != false)
            {
                return Err(Error::Protocol(
                    "SR-IOV changes require a disabled evacuated host and disabled pool HA",
                ));
            }
            let metrics = HostRecord::new(host)
                .metrics()
                .as_str()
                .and_then(|r| self.raw["host_metrics"].get(r))
                .ok_or(Error::ResourceConflict)?;
            if HostMetricsRecord::new(metrics).live() != true {
                return Err(Error::Protocol("The selected host must be live"));
            }
        }
        Ok(())
    }
}
pub(super) async fn arguments(s: &Session, r: &Record) -> Result<Vec<Value>, Error> {
    let op = operation(r).ok_or(Error::InvalidResourceCommand)?;
    let f = Fresh::read(s, op.review.kind, op.creates()).await?;
    arguments_fresh(&f, op)
}
fn arguments_fresh(f: &Fresh, op: &Operation) -> Result<Vec<Value>, Error> {
    op.availability(&f.inventory)?;
    f.graph(op)?;
    match op.change {
        Change::Create => {
            let mut args = vec![
                json!(f.by_id("PIF", op.review.input.id.uuid())?.0),
                json!(f.by_id("network", op.review.network.uuid())?.0),
            ];
            if let Kind::Tunnel(protocol) = op.review.kind {
                args.push(json!(protocol.as_str()));
            }
            Ok(args)
        }
        Change::Delete(object) => Ok(vec![json!(
            f.by_id(class(op.review.kind), object.uuid())?.0
        )]),
    }
}
pub(super) async fn prepare(s: &Session, r: &mut Record) -> Result<(), Error> {
    let op = operation(r).ok_or(Error::InvalidResourceCommand)?.clone();
    let f = Fresh::read(s, op.review.kind, op.creates()).await?;
    op.availability(&f.inventory)?;
    f.graph(&op)?;
    let mut baseline = BTreeMap::new();
    for c in ["PIF", class(op.review.kind), "task"] {
        let values = if c == "task" {
            network::all(s, c).await?.into_iter().collect()
        } else {
            f.raw[&c.to_ascii_lowercase()].clone()
        };
        let ids = values
            .values()
            .map(network::uuid)
            .collect::<Result<BTreeSet<_>, _>>()?;
        if ids.len() != values.len() {
            return Err(Error::ResourceConflict);
        }
        baseline.insert(c.to_ascii_lowercase(), ids);
    }
    r.network_fabric_plan = Some(Box::new(Plan {
        pool: r.pool,
        network: op.review.network,
        operation: op,
        baseline,
        task_uuid: None,
        completed: false,
        created: None,
    }));
    Ok(())
}
pub(super) fn task_receipt(r: &mut Record, task: &Value) -> Result<(), Error> {
    let p = r.network_fabric_plan.as_mut().ok_or(Error::Journal)?;
    if p.completed {
        return Ok(());
    }
    let id = crate::TaskId::from(network::uuid(task)?);
    if p.baseline["task"].contains(&id.uuid()) || p.task_uuid.is_some_and(|old| old != id) {
        return Err(Error::ResourceConflict);
    }
    p.task_uuid = Some(id);
    if task["status"] == "success" {
        if p.operation.creates() && r.result_reference.is_none() {
            r.result_reference = Some(crate::xapi::snapshot_task_result(task)?);
        }
        p.completed = true;
    }
    Ok(())
}
pub(super) async fn identity(s: &Session, r: &mut Record) -> Result<(), Error> {
    let op = operation(r).ok_or(Error::InvalidResourceCommand)?.clone();
    if !op.creates() {
        return Ok(());
    }
    let p = r.network_fabric_plan.as_ref().ok_or(Error::Journal)?;
    if !p.completed {
        return Err(Error::ResourceConflict);
    }
    let evidence = identity_for(
        s,
        &op,
        &p.baseline,
        r.result_reference
            .as_deref()
            .ok_or(Error::ResourceConflict)?,
        p.created.as_ref(),
    )
    .await?;
    r.network_fabric_plan.as_mut().unwrap().created = Some(evidence.clone());
    r.network_fabric_created = Some(evidence);
    Ok(())
}
async fn identity_for(
    s: &Session,
    op: &Operation,
    baseline: &BTreeMap<String, BTreeSet<Uuid>>,
    reference: &str,
    saved: Option<&Evidence>,
) -> Result<Evidence, Error> {
    let value = s
        .call(
            &format!("{}.get_record", class(op.review.kind)),
            vec![json!(reference)],
        )
        .await?;
    let id = network::uuid(&value)?;
    let (parent, child, _, _) = fields(op.review.kind);
    let child_ref = xapi::reply_reference(&value[child])?;
    let child_record = s.call("PIF.get_record", vec![json!(&child_ref)]).await?;
    let child_id = crate::PifId::from(network::uuid(&child_record)?);
    let (resolved_ref, resolved) = xapi::read(s, ResourceTarget::Pif(child_id)).await?;
    if resolved_ref != child_ref
        || resolved != child_record
        || baseline[&class(op.review.kind).to_ascii_lowercase()].contains(&id)
        || baseline["pif"].contains(&child_id.uuid())
        || network::uuid(&network::related(s, "PIF", &value[parent]).await?)?
            != op.review.input.id.uuid()
        || network::uuid(
            &network::related(s, "host", &PifRecord::new(&child_record).host()).await?,
        )? != op.review.host.uuid()
        || network::uuid(
            &network::related(s, "network", &PifRecord::new(&child_record).network()).await?,
        )? != op.review.network.uuid()
    {
        return Err(Error::ResourceConflict);
    }
    let object = match op.review.kind {
        Kind::Tunnel(_) => Object::Tunnel(id.into()),
        Kind::Sriov => Object::Sriov(id.into()),
    };
    let (resolved_ref, resolved) = xapi::read(s, object.target()).await?;
    if resolved_ref != reference || resolved != value {
        return Err(Error::ResourceConflict);
    }
    let evidence = Evidence {
        object: match op.review.kind {
            Kind::Tunnel(_) => Object::Tunnel(id.into()),
            Kind::Sriov => Object::Sriov(id.into()),
        },
        interface: child_id,
        host: op.review.host,
        network: op.review.network,
    };
    if saved.is_some_and(|old| old != &evidence) {
        return Err(Error::ResourceConflict);
    }
    Ok(evidence)
}
pub(super) async fn verify(s: &Session, r: &Record) -> Result<Option<ResourceTarget>, Error> {
    let op = operation(r).ok_or(Error::InvalidResourceCommand)?;
    if !r.network_fabric_plan.as_ref().is_some_and(|p| p.completed) {
        return Err(Error::ResourceConflict);
    }
    let f = Fresh::read(s, op.review.kind, op.creates()).await?;
    let (target, _) = verify_fresh(
        &f,
        op,
        r.result_reference.as_deref(),
        r.network_fabric_created.as_ref(),
    )?;
    Ok(target)
}
fn verify_fresh(
    f: &Fresh,
    op: &Operation,
    result: Option<&str>,
    created_evidence: Option<&Evidence>,
) -> Result<(Option<ResourceTarget>, Option<crate::NetworkFabricMember>), Error> {
    let mut expected = op.review.clone();
    let created = match op.change {
        Change::Create => {
            let e = created_evidence.ok_or(Error::ResourceConflict)?;
            let (_, child) = f.by_id("PIF", e.interface.uuid())?;
            f.clean_child(child, op.review.kind)?;
            let p = f
                .inventory
                .details
                .pifs
                .get(&e.interface)
                .ok_or(Error::ResourceConflict)?
                .clone();
            if p.host != Some(op.review.host)
                || p.network != Some(op.review.network)
                || p.primary_address_type != op.review.input.primary_address_type
                || p.capabilities.as_ref().is_none_or(|v| !v.is_empty())
            {
                return Err(Error::ResourceConflict);
            }
            let (oraw, o) = f.by_id(class(op.review.kind), e.object.uuid())?;
            if Some(oraw) != result {
                return Err(Error::ResourceConflict);
            }
            match op.review.kind {
                Kind::Tunnel(protocol) => {
                    if o["protocol"] != protocol.as_str() || p.currently_attached != Some(true) {
                        return Err(Error::ResourceConflict);
                    }
                }
                Kind::Sriov => {
                    let reboot = o["requires_reboot"]
                        .as_bool()
                        .ok_or(Error::ResourceConflict)?;
                    if !matches!(
                        o["configuration_mode"].as_str(),
                        Some("sysfs" | "modprobe" | "manual")
                    ) || p.currently_attached != Some(!reboot)
                        || p.device != op.review.input.device
                        || p.mac != "fe:ff:ff:ff:ff:ff"
                        || p.mtu != op.review.input.mtu
                    {
                        return Err(Error::ResourceConflict);
                    }
                }
            }
            expected.members.insert(
                e.interface,
                crate::NetworkFabricMember {
                    object: e.object,
                    interface: p,
                    input: op.review.input.id,
                },
            );
            Some(e.object.target())
        }
        Change::Delete(object) => {
            let old = expected
                .members
                .iter()
                .find(|(_, m)| m.object == object)
                .map(|(id, _)| *id)
                .ok_or(Error::ResourceConflict)?;
            if f.by_id(class(op.review.kind), object.uuid()).is_ok()
                || f.by_id("PIF", old.uuid()).is_ok()
            {
                return Err(Error::ResourceConflict);
            }
            expected.members.remove(&old);
            None
        }
    };
    let current = crate::NetworkFabricReview::capture(
        &f.inventory,
        op.review.network,
        op.review.input.id,
        op.review.kind,
        op.creates(),
    )?;
    if current != expected {
        return Err(Error::ResourceConflict);
    }
    // Validate the completed graph without reapplying creation's pre-existing-object exclusion.
    let mut checked = op.clone();
    checked.review = current;
    if let Some(e) = created_evidence {
        checked.change = Change::Delete(e.object);
    }
    f.graph(&checked)?;
    let member = created_evidence
        .and_then(|e| checked.review.members.get(&e.interface))
        .cloned();
    Ok((created, member))
}
