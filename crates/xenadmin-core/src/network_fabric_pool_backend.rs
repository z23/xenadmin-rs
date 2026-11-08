use super::*;
use crate::network_topology::{Kind as StepKind, Plan as TopologyPlan, Receipt};
use crate::{NetworkTopologyOperation, OperationState, StepState};

fn operation(p: &TopologyPlan) -> Result<&crate::NetworkFabricPool, Error> {
    match &p.operation {
        NetworkTopologyOperation::Fabric(op) => Ok(op),
        _ => Err(Error::Journal),
    }
}
pub(crate) fn kinds(op: &crate::NetworkFabricPool) -> Vec<StepKind> {
    op.operations
        .iter()
        .cloned()
        .map(|op| StepKind::Fabric(Box::new(op)))
        .collect()
}
fn initial(p: &TopologyPlan, index: usize) -> Result<&Operation, Error> {
    match &p.steps.get(index).ok_or(Error::Journal)?.kind {
        StepKind::Fabric(op) => Ok(op),
        _ => Err(Error::Journal),
    }
}
fn adjusted(p: &TopologyPlan, index: usize, through: usize) -> Result<Operation, Error> {
    let mut op = initial(p, index)?.clone();
    for step in p.steps.iter().take(through) {
        if !step.verified {
            return Err(Error::ResourceConflict);
        }
        let StepKind::Fabric(prior) = &step.kind else {
            return Err(Error::Journal);
        };
        match prior.change {
            Change::Create => {
                let m = step.fabric_member.as_ref().ok_or(Error::Journal)?;
                op.review.members.insert(m.interface.id, m.clone());
            }
            Change::Delete(object) => {
                op.review.members.retain(|_, m| m.object != object);
            }
        }
    }
    Ok(op)
}
fn live(f: &Fresh, p: &TopologyPlan) -> Result<(), Error> {
    let first = initial(p, 0)?;
    if f.inventory.pool_id != p.pool
        || f.inventory.hosts.keys().copied().collect::<BTreeSet<_>>() != first.review.hosts
    {
        return Err(Error::ResourceConflict);
    }
    for host in &first.review.hosts {
        let (_, h) = f.by_id("host", host.uuid())?;
        let metrics = h["metrics"]
            .as_str()
            .and_then(|r| f.raw["host_metrics"].get(r))
            .ok_or(Error::ResourceConflict)?;
        if metrics["live"] != true {
            return Err(Error::Protocol(
                "Every pool host must be live for a pool-wide fabric change",
            ));
        }
    }
    Ok(())
}
fn review_all(f: &Fresh, p: &TopologyPlan, through: usize) -> Result<(), Error> {
    live(f, p)?;
    for index in 0..p.steps.len() {
        let mut op = adjusted(p, index, through)?;
        let review = crate::NetworkFabricReview::capture(
            &f.inventory,
            p.network,
            op.review.input.id,
            op.review.kind,
            op.creates(),
        )?;
        if review != op.review {
            return Err(Error::ResourceConflict);
        }
        // Earlier successful creation owns its exact object; permit only that association.
        if index < through && op.creates() {
            op.change = Change::Delete(
                p.steps[index]
                    .fabric_created
                    .as_ref()
                    .ok_or(Error::Journal)?
                    .object,
            );
        }
        f.graph(&op)?;
    }
    Ok(())
}
pub(crate) async fn prepare(
    s: &Session,
    r: &Record,
    op: &crate::NetworkFabricPool,
) -> Result<TopologyPlan, Error> {
    op.validate(r.target)?;
    let first = op.operations.first().ok_or(Error::InvalidResourceCommand)?;
    if first.review.pool != r.pool {
        return Err(Error::ResourceConflict);
    }
    let f = Fresh::read(s, first.review.kind, first.creates()).await?;
    op.availability(&f.inventory)?;
    let before = f.raw[&class(first.review.kind).to_ascii_lowercase()]
        .values()
        .map(|v| {
            let id = network::uuid(v)?;
            Ok(match first.review.kind {
                Kind::Tunnel(_) => Object::Tunnel(id.into()),
                Kind::Sriov => Object::Sriov(id.into()),
            })
        })
        .collect::<Result<_, Error>>()?;
    let tasks = network::all(s, "task").await?;
    let tasks_before: BTreeSet<crate::TaskId> = tasks
        .values()
        .map(|v| network::uuid(v).map(Into::into))
        .collect::<Result<_, _>>()?;
    if tasks_before.len() != tasks.len() {
        return Err(Error::ResourceConflict);
    }
    let p = TopologyPlan {
        pool: r.pool,
        network: first.review.network,
        operation: NetworkTopologyOperation::Fabric(Box::new(op.clone())),
        vlans_before: BTreeSet::new(),
        pifs_before: f.raw["pif"]
            .values()
            .map(|v| network::uuid(v).map(Into::into))
            .collect::<Result<_, _>>()?,
        tasks_before,
        fabric_before: Some(before),
        expires_at_ms: crate::resources::topology::now().saturating_add(2 * 60 * 60 * 1000),
        steps: kinds(op)
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
    };
    review_all(&f, &p, 0)?;
    Ok(p)
}
pub(crate) async fn arguments(
    s: &Session,
    p: &TopologyPlan,
    index: usize,
) -> Result<(&'static str, Vec<Value>), Error> {
    if p.steps
        .iter()
        .skip(index)
        .any(|s| s.task.is_some() || s.completed || s.verified)
    {
        return Err(Error::ResourceConflict);
    }
    let op = adjusted(p, index, index)?;
    let f = Fresh::read(s, op.review.kind, op.creates()).await?;
    review_all(&f, p, index)?;
    Ok((op.method(), arguments_fresh(&f, &op)?))
}
fn baseline(p: &TopologyPlan) -> Result<BTreeMap<String, BTreeSet<Uuid>>, Error> {
    let first = initial(p, 0)?;
    Ok([
        (
            String::from("pif"),
            p.pifs_before.iter().map(|id| id.uuid()).collect(),
        ),
        (
            class(first.review.kind).to_ascii_lowercase(),
            p.fabric_before
                .as_ref()
                .ok_or(Error::Journal)?
                .iter()
                .map(|id| id.uuid())
                .collect(),
        ),
    ]
    .into())
}
pub(crate) async fn identity(
    s: &Session,
    p: &TopologyPlan,
    index: usize,
) -> Result<Evidence, Error> {
    let step = p.steps.get(index).ok_or(Error::Journal)?;
    if !step.completed || !step.ended {
        return Err(Error::ResourceConflict);
    }
    let op = initial(p, index)?;
    let e = identity_for(
        s,
        op,
        &baseline(p)?,
        step.result.as_deref().ok_or(Error::ResourceConflict)?,
        step.fabric_created.as_ref(),
    )
    .await?;
    if p.steps.iter().enumerate().any(|(i, s)| {
        i != index
            && s.fabric_created
                .as_ref()
                .is_some_and(|v| v.object == e.object || v.interface == e.interface)
    }) {
        return Err(Error::ResourceConflict);
    }
    Ok(e)
}
pub(crate) async fn verify(
    s: &Session,
    p: &TopologyPlan,
    index: usize,
) -> Result<Option<crate::NetworkFabricMember>, Error> {
    let op = adjusted(p, index, index)?;
    let step = &p.steps[index];
    if !step.completed {
        return Err(Error::ResourceConflict);
    }
    if op.creates()
        && identity(s, p, index).await?.clone()
            != *step.fabric_created.as_ref().ok_or(Error::Journal)?
    {
        return Err(Error::ResourceConflict);
    }
    let f = Fresh::read(s, op.review.kind, op.creates()).await?;
    let (_, member) = verify_fresh(
        &f,
        &op,
        step.result.as_deref(),
        step.fabric_created.as_ref(),
    )?;
    let mut after = p.clone();
    after.steps[index].fabric_member = member.clone();
    after.steps[index].verified = true;
    review_all(&f, &after, index + 1)?;
    Ok(member)
}
pub(crate) async fn finish(s: &Session, p: &TopologyPlan) -> Result<(), Error> {
    let first = initial(p, 0)?;
    let through = p.steps.iter().take_while(|s| s.verified).count();
    if p.steps
        .iter()
        .skip(through)
        .any(|s| s.verified || s.task.is_some() || s.completed)
    {
        return Err(Error::ResourceConflict);
    }
    let f = Fresh::read(s, first.review.kind, first.creates()).await?;
    review_all(&f, p, through)
}
pub(crate) fn validate_record(r: &Record, p: &TopologyPlan) -> Result<(), Error> {
    let op = operation(p)?;
    op.validate(r.target).map_err(|_| Error::Journal)?;
    let first = op.operations.first().ok_or(Error::Journal)?;
    let before = p.fabric_before.as_ref().ok_or(Error::Journal)?;
    if p.pool != r.pool
        || p.network != first.review.network
        || first.review.pool != r.pool
        || r.action != ResourceAction::NetworkTopology(p.operation.clone())
        || !p.vlans_before.is_empty()
        || before.len() > 10_000
        || p.pifs_before.len() > 10_000
        || p.tasks_before.len() > 10_000
        || p.expires_at_ms < r.created_at_ms
        || p.expires_at_ms > r.created_at_ms.saturating_add(24 * 60 * 60 * 1000)
        || p.steps.len() != op.operations.len()
        || r.steps.len() != p.steps.len()
        || r.topology_changes != p.changes()
        || p.pifs_before.iter().any(|id| id.uuid().is_nil())
        || p.tasks_before.iter().any(|id| id.uuid().is_nil())
        || before.iter().any(|id| {
            id.uuid().is_nil()
                || !matches!(
                    (first.review.kind, id),
                    (Kind::Tunnel(_), Object::Tunnel(_)) | (Kind::Sriov, Object::Sriov(_))
                )
        })
    {
        return Err(Error::Journal);
    }
    let mut task_ids = BTreeSet::new();
    let mut objects = BTreeSet::new();
    let mut pifs = BTreeSet::new();
    let mut prior_verified = true;
    for ((step, expected), public) in p.steps.iter().zip(&op.operations).zip(&r.steps) {
        if step.kind != StepKind::Fabric(Box::new(expected.clone()))
            || public.label != step.kind.label()
            || (public.state == StepState::Verified) != step.verified
            || step.created.is_some()
            || !p.pifs_before.contains(&expected.review.input.id)
            || !expected
                .review
                .members
                .values()
                .all(|m| before.contains(&m.object) && p.pifs_before.contains(&m.interface.id))
            || !prior_verified
                && (step.task.is_some()
                    || step.completed
                    || step.verified
                    || public.state != StepState::Pending)
            || step.ended && step.task_uuid.is_none()
            || step.completed && !step.ended
            || step.cancel_sent && step.task.is_none()
            || step.verified && !step.completed
            || public.state == StepState::Pending
                && (step.task.is_some()
                    || step.task_uuid.is_some()
                    || step.ended
                    || step.completed
                    || step.result.is_some()
                    || step.fabric_created.is_some()
                    || step.fabric_member.is_some()
                    || step.cancel_sent)
        {
            return Err(Error::Journal);
        }
        if let Some(task) = &step.task {
            xapi::reply_reference(&json!(task))?;
        }
        if let Some(id) = step.task_uuid
            && (id.uuid().is_nil()
                || step.task.is_none()
                || p.tasks_before.contains(&id)
                || !task_ids.insert(id))
        {
            return Err(Error::Journal);
        }
        if let Some(result) = &step.result {
            xapi::reply_reference(&json!(result))?;
            if !step.completed || !expected.creates() {
                return Err(Error::Journal);
            }
        }
        if step.completed && expected.creates() && step.result.is_none() {
            return Err(Error::Journal);
        }
        if let Some(e) = &step.fabric_created
            && (!expected.creates()
                || !step.completed
                || step.result.is_none()
                || e.host != expected.review.host
                || e.network != p.network
                || e.object.uuid().is_nil()
                || e.interface.uuid().is_nil()
                || before.contains(&e.object)
                || p.pifs_before.contains(&e.interface)
                || !objects.insert(e.object)
                || !pifs.insert(e.interface)
                || !matches!(
                    (first.review.kind, e.object),
                    (Kind::Tunnel(_), Object::Tunnel(_)) | (Kind::Sriov, Object::Sriov(_))
                ))
        {
            return Err(Error::Journal);
        }
        if let Some(m) = &step.fabric_member
            && (step.fabric_created.as_ref().is_none_or(|e| {
                m.object != e.object
                    || m.interface.id != e.interface
                    || m.interface.host != Some(e.host)
                    || m.interface.network != Some(e.network)
            }) || m.input != expected.review.input.id)
        {
            return Err(Error::Journal);
        }
        if step.verified && expected.creates() && step.fabric_member.is_none() {
            return Err(Error::Journal);
        }
        prior_verified = step.verified;
    }
    if r.state == OperationState::Succeeded && !p.steps.iter().all(|s| s.verified) {
        return Err(Error::Journal);
    }
    Ok(())
}
