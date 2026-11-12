use super::{ResourceAction, ResourceOperationRecord, ResourceTarget, network, xapi};
use crate::{
    Error, HostId, NetworkId, NetworkTopologyOperation, PifId, StepState, VlanId,
    network_topology::{Kind, Plan, Receipt, ReviewedVlan, VlanUplink},
    xapi::Session,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    time::{SystemTime, UNIX_EPOCH},
};
use xenadmin_xapi::enumerations::{ip_configuration_mode, ipv6_configuration_mode};
use xenadmin_xapi::records::{HostMetricsRecord, HostRecord, NetworkRecord, PifRecord, VlanRecord};

pub(super) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
fn require(value: bool, reason: &'static str) -> Result<(), Error> {
    if value {
        Ok(())
    } else {
        Err(Error::Protocol(reason))
    }
}
pub(super) fn operation(
    record: &ResourceOperationRecord,
) -> Result<&NetworkTopologyOperation, Error> {
    match &record.action {
        ResourceAction::NetworkTopology(op) => Ok(op),
        _ => Err(Error::InvalidResourceCommand),
    }
}
fn kinds(operation: &NetworkTopologyOperation) -> Vec<Kind> {
    match operation {
        NetworkTopologyOperation::Fabric(op) => super::network_fabric::pool::kinds(op),
        NetworkTopologyOperation::CreateVlans { uplinks, tag } => uplinks
            .iter()
            .map(|uplink| Kind::Create {
                uplink: uplink.clone(),
                tag: *tag,
            })
            .collect(),
        NetworkTopologyOperation::RetagVlans { interfaces, tag } => {
            let mut result = Vec::new();
            for vlan in interfaces {
                result.push(Kind::Destroy(vlan.clone()));
                let create_step = result.len();
                result.push(Kind::Create {
                    uplink: vlan.uplink.clone(),
                    tag: *tag,
                });
                if !vlan.attached {
                    result.push(Kind::DetachCreated {
                        create_step,
                        uplink: vlan.uplink.clone(),
                    });
                }
            }
            result
        }
    }
}
pub(crate) fn validate_record(record: &ResourceOperationRecord) -> Result<(), Error> {
    let Some(plan) = &record.network_topology_plan else {
        if matches!(record.action, ResourceAction::NetworkTopology(_)) {
            require(
                record.steps.is_empty()
                    && record.topology_changes == Default::default()
                    && matches!(
                        record.state,
                        crate::OperationState::Preparing
                            | crate::OperationState::Rejected
                            | crate::OperationState::Cancelled
                            | crate::OperationState::OutcomeUnknown
                            | crate::OperationState::AcknowledgedUnknown
                    ),
                "network topology evidence missing",
            )?;
        } else {
            require(
                record.topology_changes == Default::default(),
                "unexpected network topology evidence",
            )?;
        }
        return Ok(());
    };
    if matches!(plan.operation, NetworkTopologyOperation::Fabric(_)) {
        return super::network_fabric::pool::validate_record(record, plan);
    }
    require(
        plan.fabric_before.is_none()
            && plan
                .steps
                .iter()
                .all(|s| s.fabric_created.is_none() && s.fabric_member.is_none()),
        "Unexpected fabric topology evidence",
    )?;
    let operation = operation(record)?;
    operation.validate(record.target)?;
    require(
        plan.pool == record.pool
            && record.target == ResourceTarget::Network(plan.network)
            && &plan.operation == operation,
        "network topology plan does not match its intent",
    )?;
    require(
        [
            plan.vlans_before.len(),
            plan.pifs_before.len(),
            plan.tasks_before.len(),
        ]
        .iter()
        .all(|n| *n <= 10_000)
            && plan.expires_at_ms >= record.created_at_ms
            && plan.expires_at_ms <= record.created_at_ms.saturating_add(24 * 60 * 60 * 1000),
        "network topology baseline unavailable",
    )?;
    require(
        operation
            .uplinks()
            .iter()
            .all(|p| plan.pifs_before.contains(&p.pif))
            && plan.pifs_before.iter().all(|id| !id.uuid().is_nil())
            && plan.vlans_before.iter().all(|id| !id.uuid().is_nil())
            && plan.tasks_before.iter().all(|id| !id.uuid().is_nil()),
        "network topology baseline omits reviewed identities",
    )?;
    if let NetworkTopologyOperation::RetagVlans { interfaces, .. } = operation {
        require(
            interfaces.iter().all(|v| {
                plan.vlans_before.contains(&v.vlan) && plan.pifs_before.contains(&v.child)
            }),
            "network topology baseline omits reviewed VLAN interfaces",
        )?;
    }
    let mut created_vlans = BTreeSet::new();
    let mut created_pifs = BTreeSet::new();
    let expected = kinds(operation);
    require(
        plan.steps.len() == expected.len()
            && plan.steps.len() == record.steps.len()
            && record.topology_changes == plan.changes(),
        "network topology step evidence inconsistent",
    )?;
    let mut task_ids = BTreeSet::new();
    for ((step, kind), public) in plan.steps.iter().zip(expected).zip(&record.steps) {
        require(
            step.kind == kind
                && public.label == kind.label()
                && (public.state == StepState::Verified) == step.verified,
            "network topology steps changed",
        )?;
        if let Some(task) = &step.task {
            xapi::reply_reference(&json!(task))?;
        }
        if let Some(task) = step.task_uuid {
            require(
                !task.uuid().is_nil()
                    && step.task.is_some()
                    && !plan.tasks_before.contains(&task)
                    && task_ids.insert(task),
                "network topology task identity invalid",
            )?;
        }
        require(
            !step.ended || step.task_uuid.is_some(),
            "network topology task completion missing identity",
        )?;
        require(
            !step.completed || step.ended,
            "network topology completion evidence inconsistent",
        )?;
        require(
            !step.cancel_sent || step.task.is_some(),
            "network topology cancellation missing task",
        )?;
        if let Some(result) = &step.result {
            xapi::reply_reference(&json!(result))?;
            require(
                step.completed && matches!(step.kind, Kind::Create { .. }),
                "network topology result lacks completion",
            )?;
        }
        if let Some((vlan, pif)) = step.created {
            require(
                !vlan.uuid().is_nil()
                    && !pif.uuid().is_nil()
                    && created_vlans.insert(vlan)
                    && created_pifs.insert(pif),
                "network topology created identities are duplicated",
            )?;
            require(
                step.result.is_some()
                    && !plan.vlans_before.contains(&vlan)
                    && !plan.pifs_before.contains(&pif),
                "network topology result predates this operation",
            )?;
        }
        require(
            !step.verified
                || (step.completed
                    && (matches!(step.kind, Kind::Destroy(_) | Kind::DetachCreated { .. })
                        || step.created.is_some())),
            "network topology verification lacks completion",
        )?;
    }
    require(
        record.state != crate::OperationState::Succeeded
            || plan.steps.iter().all(|step| step.verified),
        "network topology success lacks verification",
    )?;
    Ok(())
}
async fn baseline<T: From<uuid::Uuid> + Ord>(
    session: &Session,
    class: &str,
) -> Result<BTreeSet<T>, Error> {
    network::all(session, class)
        .await?
        .values()
        .map(|v| network::uuid(v).map(Into::into))
        .collect()
}
async fn host_coverage(
    session: &Session,
    operation: &NetworkTopologyOperation,
) -> Result<(), Error> {
    let hosts = baseline::<HostId>(session, "host").await?;
    require(
        hosts
            == operation
                .uplinks()
                .iter()
                .map(|p| p.expected.host)
                .collect(),
        "Pool membership changed; review one interface for every host again",
    )?;
    for host in hosts {
        let (_, host) = xapi::read(session, ResourceTarget::Host(host)).await?;
        let metrics =
            network::related(session, "host_metrics", &HostRecord::new(&host).metrics()).await?;
        require(
            HostMetricsRecord::new(&metrics).live().as_bool() == Some(true),
            "Every pool host must be online for a pool-wide network change",
        )?;
    }
    Ok(())
}
async fn uplink(session: &Session, review: &VlanUplink) -> Result<(String, Value), Error> {
    let (reference, pif) = xapi::read(session, ResourceTarget::Pif(review.pif)).await?;
    network::pif_identity(session, &pif, &review.expected).await?;
    require(
        PifRecord::new(&pif).managed().as_bool() == Some(true)
            && network::number(&PifRecord::new(&pif).vlan()) == Some(-1)
            && PifRecord::new(&pif).bond_slave_of() == "OpaqueRef:NULL",
        "VLAN uplink must be a managed untagged interface or bond master",
    )?;
    Ok((reference, pif))
}
fn empty_set(record: &Value, field: &str) -> Result<(), Error> {
    require(
        network::refs(&record[field])?.is_empty(),
        "The network interface has additional topology consumers",
    )
}
fn empty_map(record: &Value, field: &str) -> Result<(), Error> {
    require(
        record[field].as_object().is_some_and(|map| map.is_empty()),
        "Retagging requires empty interface metadata; preserve or move its configuration first",
    )
}
async fn retaggable(
    session: &Session,
    network: NetworkId,
    review: &ReviewedVlan,
) -> Result<(String, String), Error> {
    let (reference, vlan) = xapi::read(session, ResourceTarget::Vlan(review.vlan)).await?;
    let (parent, _) = uplink(session, &review.uplink).await?;
    let (child, pif) = xapi::read(session, ResourceTarget::Pif(review.child)).await?;
    require(
        VlanRecord::new(&vlan).tagged_pif() == parent
            && VlanRecord::new(&vlan).untagged_pif() == child
            && network::number(&VlanRecord::new(&vlan).tag()) == Some(i64::from(review.tag)),
        "Reviewed VLAN topology changed",
    )?;
    require(
        network::uuid(&network::related(session, "host", &PifRecord::new(&pif).host()).await?)?
            == review.uplink.expected.host.uuid()
            && network::uuid(
                &network::related(session, "network", &PifRecord::new(&pif).network()).await?,
            )? == network.uuid(),
        "Reviewed VLAN host or network changed",
    )?;
    require(
        PifRecord::new(&pif).currently_attached().as_bool() == Some(review.attached)
            && PifRecord::new(&pif).management().as_bool() == Some(false)
            && PifRecord::new(&pif).managed().as_bool() == Some(true)
            && PifRecord::new(&pif).disallow_unplug().as_bool() == Some(false)
            && PifRecord::new(&pif).physical().as_bool() == Some(false)
            && PifRecord::new(&pif).bond_slave_of() == "OpaqueRef:NULL"
            && network::number(&PifRecord::new(&pif).vlan()) == Some(i64::from(review.tag))
            && PifRecord::new(&pif).ip_configuration_mode() == ip_configuration_mode::NONE
            && PifRecord::new(&pif).ipv6_configuration_mode() == ipv6_configuration_mode::NONE
            && PifRecord::new(&pif).ip() == ""
            && PifRecord::new(&pif).netmask() == ""
            && PifRecord::new(&pif).gateway() == ""
            && PifRecord::new(&pif).dns() == ""
            && PifRecord::new(&pif).ipv6_gateway() == ""
            && PifRecord::new(&pif)
                .ipv6()
                .as_array()
                .is_some_and(|a| a.iter().all(|v| v == "")),
        "Retagging refuses management, protected or IP-configured interfaces",
    )?;
    for field in [
        "VLAN_slave_of",
        "bond_master_of",
        "tunnel_access_PIF_of",
        "tunnel_transport_PIF_of",
    ] {
        empty_set(&pif, field)?;
    }
    empty_map(&pif, "other_config")?;
    empty_map(&pif, "properties")?;
    empty_map(&vlan, "other_config")?;
    Ok((reference, child))
}

fn expected_members(plan: &Plan, finishing: Option<usize>) -> BTreeSet<PifId> {
    let mut ids = match &plan.operation {
        NetworkTopologyOperation::Fabric(op) => op
            .operations
            .first()
            .map(|op| op.review.members.keys().copied().collect())
            .unwrap_or_default(),
        NetworkTopologyOperation::CreateVlans { .. } => BTreeSet::new(),
        NetworkTopologyOperation::RetagVlans { interfaces, .. } => {
            interfaces.iter().map(|v| v.child).collect()
        }
    };
    for (index, step) in plan.steps.iter().enumerate() {
        if let Kind::Destroy(v) = &step.kind
            && (step.verified || finishing == Some(index))
        {
            ids.remove(&v.child);
        }
        if let Some((_, pif)) = step.created {
            ids.insert(pif);
        }
    }
    ids
}
async fn checked_network(
    session: &Session,
    plan: &Plan,
    finishing: Option<usize>,
) -> Result<(String, Value), Error> {
    let (reference, network) = xapi::read(session, ResourceTarget::Network(plan.network)).await?;
    require(
        NetworkRecord::new(&network).managed().as_bool() == Some(true),
        "The selected network must be managed by XAPI",
    )?;
    empty_set(&network, "VIFs")?;
    let mut actual = BTreeSet::new();
    for pif in network::refs(&NetworkRecord::new(&network).pifs())? {
        let value = session.call("PIF.get_record", vec![json!(pif)]).await?;
        actual.insert(PifId::from(network::uuid(&value)?));
    }
    require(
        actual == expected_members(plan, finishing),
        "The network's host interfaces changed after review",
    )?;
    Ok((reference, network))
}
pub(super) async fn prepare(
    session: &Session,
    record: &ResourceOperationRecord,
) -> Result<Plan, Error> {
    let operation = operation(record)?.clone();
    if let NetworkTopologyOperation::Fabric(op) = &operation {
        return super::network_fabric::pool::prepare(session, record, op).await;
    }
    operation.validate(record.target)?;
    let ResourceTarget::Network(network) = record.target else {
        return Err(Error::InvalidResourceCommand);
    };
    xapi::read(session, ResourceTarget::Pool(record.pool)).await?;
    host_coverage(session, &operation).await?;
    let plan = Plan {
        pool: record.pool,
        network,
        operation: operation.clone(),
        vlans_before: baseline(session, "VLAN").await?,
        pifs_before: baseline(session, "PIF").await?,
        tasks_before: baseline(session, "task").await?,
        steps: kinds(&operation)
            .into_iter()
            .map(|kind| Receipt {
                kind,
                task: None,
                task_uuid: None,
                result: None,
                completed: false,
                ended: false,
                cancel_sent: false,
                created: None,
                fabric_created: None,
                fabric_member: None,
                verified: false,
            })
            .collect(),
        expires_at_ms: now().saturating_add(2 * 60 * 60 * 1000),
        fabric_before: None,
    };
    let (_, target) = checked_network(session, &plan, None).await?;
    for parent in operation.uplinks() {
        let (_, pif) = uplink(session, parent).await?;
        require(
            network::number(&target["MTU"])
                .zip(network::number(&PifRecord::new(&pif).mtu()))
                .is_some_and(|(net, pif)| net >= 576 && net <= pif),
            "The VLAN network MTU must not exceed its uplink MTU",
        )?;
        if let NetworkTopologyOperation::CreateVlans { tag, .. } = &operation {
            unused_tag(session, parent.pif, *tag).await?;
        }
    }
    if let NetworkTopologyOperation::RetagVlans { interfaces, tag } = &operation {
        for vlan in interfaces {
            retaggable(session, network, vlan).await?;
            unused_tag(session, vlan.uplink.pif, *tag).await?;
        }
    }
    Ok(plan)
}
async fn unused_tag(session: &Session, parent: PifId, tag: u16) -> Result<(), Error> {
    let (parent, _) = xapi::read(session, ResourceTarget::Pif(parent)).await?;
    let vlans = network::all(session, "VLAN").await?;
    require(
        !vlans.values().any(|v| {
            VlanRecord::new(v).tagged_pif() == parent
                && network::number(&VlanRecord::new(v).tag()) == Some(i64::from(tag))
        }),
        "The selected VLAN tag is already present on an uplink",
    )
}
pub(super) async fn arguments(
    session: &Session,
    record: &ResourceOperationRecord,
    index: usize,
) -> Result<(&'static str, Vec<Value>), Error> {
    let plan = record
        .network_topology_plan
        .as_ref()
        .ok_or(Error::Journal)?;
    let step = plan.steps.get(index).ok_or(Error::Journal)?;
    if matches!(step.kind, Kind::Fabric(_)) {
        return super::network_fabric::pool::arguments(session, plan, index).await;
    }
    xapi::read(session, ResourceTarget::Pool(plan.pool)).await?;
    host_coverage(session, &plan.operation).await?;
    let (network, target) = checked_network(session, plan, None).await?;
    match &step.kind {
        Kind::Fabric(_) => Err(Error::Journal),
        Kind::Create {
            uplink: parent,
            tag,
        } => {
            let (reference, pif) = uplink(session, parent).await?;
            require(
                network::number(&target["MTU"])
                    .zip(network::number(&PifRecord::new(&pif).mtu()))
                    .is_some_and(|(net, pif)| net >= 576 && net <= pif),
                "The VLAN network MTU must not exceed its uplink MTU",
            )?;
            unused_tag(session, parent.pif, *tag).await?;
            Ok((
                "Async.VLAN.create",
                vec![json!(reference), json!(tag.to_string()), json!(network)],
            ))
        }
        Kind::Destroy(vlan) => Ok((
            "Async.VLAN.destroy",
            vec![json!(retaggable(session, plan.network, vlan).await?.0)],
        )),
        Kind::DetachCreated {
            create_step,
            uplink,
        } => {
            let (vlan, child) = creation_identity(session, plan, *create_step).await?;
            let Kind::Create { tag, .. } = &plan.steps[*create_step].kind else {
                return Err(Error::Journal);
            };
            let (_, reference) = retaggable(
                session,
                plan.network,
                &ReviewedVlan {
                    vlan,
                    child,
                    uplink: uplink.clone(),
                    tag: *tag,
                    attached: true,
                },
            )
            .await?;
            Ok(("Async.PIF.unplug", vec![json!(reference)]))
        }
    }
}
pub(super) async fn creation_identity(
    session: &Session,
    plan: &Plan,
    index: usize,
) -> Result<(VlanId, PifId), Error> {
    let step = plan.steps.get(index).ok_or(Error::Journal)?;
    let Kind::Create {
        uplink: parent,
        tag,
    } = &step.kind
    else {
        return Err(Error::Journal);
    };
    xapi::read(session, ResourceTarget::Pool(plan.pool)).await?;
    let reference = step.result.as_ref().ok_or(Error::Protocol(
        "Created VLAN result is unavailable; no request will be replayed",
    ))?;
    let vlan = session
        .call("VLAN.get_record", vec![json!(reference)])
        .await?;
    let id = VlanId::from(network::uuid(&vlan)?);
    let child = network::related(session, "PIF", &VlanRecord::new(&vlan).untagged_pif()).await?;
    let child_id = PifId::from(network::uuid(&child)?);
    let (parent_ref, _) = uplink(session, parent).await?;
    require(
        !plan.vlans_before.contains(&id)
            && !plan.pifs_before.contains(&child_id)
            && step.created.is_none_or(|created| created == (id, child_id))
            && !plan.steps.iter().enumerate().any(|(i, s)| {
                i != index && s.created.is_some_and(|(v, p)| v == id || p == child_id)
            })
            && VlanRecord::new(&vlan).tagged_pif() == parent_ref
            && network::number(&VlanRecord::new(&vlan).tag()) == Some(i64::from(*tag))
            && network::number(&PifRecord::new(&child).vlan()) == Some(i64::from(*tag))
            && PifRecord::new(&child).vlan_master_of() == *reference
            && network::uuid(
                &network::related(session, "host", &PifRecord::new(&child).host()).await?,
            )? == parent.expected.host.uuid()
            && network::uuid(
                &network::related(session, "network", &PifRecord::new(&child).network()).await?,
            )? == plan.network.uuid(),
        "Created VLAN identity does not match the reviewed network, host and tag",
    )?;
    require(
        PifRecord::new(&child).management().as_bool() == Some(false)
            && PifRecord::new(&child).physical().as_bool() == Some(false),
        "Created VLAN interface has an unexpected management role",
    )?;
    Ok((id, child_id))
}
async fn absent(session: &Session, target: ResourceTarget) -> Result<(), Error> {
    match xapi::read(session, target).await {
        Err(Error::Api(code)) if code == "UUID_INVALID" => Ok(()),
        Ok(_) => Err(Error::ResourceConflict),
        Err(error) => Err(error),
    }
}
pub(super) async fn verify(
    session: &Session,
    record: &ResourceOperationRecord,
    index: usize,
) -> Result<Option<crate::NetworkFabricMember>, Error> {
    let plan = record
        .network_topology_plan
        .as_ref()
        .ok_or(Error::Journal)?;
    let step = plan.steps.get(index).ok_or(Error::Journal)?;
    if matches!(step.kind, Kind::Fabric(_)) {
        return super::network_fabric::pool::verify(session, plan, index).await;
    }
    xapi::read(session, ResourceTarget::Pool(plan.pool)).await?;
    require(
        step.completed,
        "Host task completion is unavailable; no write will be repeated",
    )?;
    match &step.kind {
        Kind::Fabric(_) => return Err(Error::Journal),
        Kind::Create { .. } => {
            let ids = creation_identity(session, plan, index).await?;
            require(step.created == Some(ids), "Created VLAN evidence changed")?;
            let (_, child) = xapi::read(session, ResourceTarget::Pif(ids.1)).await?;
            require(
                PifRecord::new(&child).currently_attached().as_bool() == Some(true),
                "The created VLAN interface is not attached",
            )?;
        }
        Kind::DetachCreated { create_step, .. } => {
            let (_, id) = creation_identity(session, plan, *create_step).await?;
            let (_, pif) = xapi::read(session, ResourceTarget::Pif(id)).await?;
            require(
                PifRecord::new(&pif).currently_attached().as_bool() == Some(false),
                "The replacement VLAN is still attached",
            )?;
        }
        Kind::Destroy(vlan) => {
            absent(session, ResourceTarget::Vlan(vlan.vlan)).await?;
            absent(session, ResourceTarget::Pif(vlan.child)).await?;
        }
    }
    checked_network(session, plan, Some(index)).await?;
    Ok(None)
}

pub(super) async fn finish(
    session: &Session,
    record: &ResourceOperationRecord,
) -> Result<(), Error> {
    let plan = record
        .network_topology_plan
        .as_ref()
        .ok_or(Error::Journal)?;
    if matches!(plan.operation, NetworkTopologyOperation::Fabric(_)) {
        return super::network_fabric::pool::finish(session, plan).await;
    }
    xapi::read(session, ResourceTarget::Pool(plan.pool)).await?;
    host_coverage(session, &plan.operation).await?;
    checked_network(session, plan, None).await?;
    for (index, step) in plan.steps.iter().enumerate() {
        if !step.verified {
            require(
                step.task.is_none() && !step.completed,
                "Submitted topology step remains uncertain",
            )?;
            if let Kind::Destroy(vlan) = &step.kind {
                retaggable(session, plan.network, vlan).await?;
            }
            continue;
        }
        match &step.kind {
            Kind::Create { uplink, tag } => {
                let (vlan, child) = creation_identity(session, plan, index).await?;
                let detached=plan.steps.iter().any(|later|later.verified && matches!(later.kind,Kind::DetachCreated{create_step,..} if create_step==index));
                retaggable(
                    session,
                    plan.network,
                    &ReviewedVlan {
                        vlan,
                        child,
                        uplink: uplink.clone(),
                        tag: *tag,
                        attached: !detached,
                    },
                )
                .await?;
            }
            Kind::Destroy(vlan) => {
                absent(session, ResourceTarget::Vlan(vlan.vlan)).await?;
                absent(session, ResourceTarget::Pif(vlan.child)).await?;
            }
            Kind::DetachCreated { .. } => {}
            Kind::Fabric(_) => return Err(Error::Journal),
        }
    }
    Ok(())
}
