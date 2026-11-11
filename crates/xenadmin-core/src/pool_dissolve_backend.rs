use super::{ResourceAction, ResourceOperationRecord, ResourceTarget, xapi};
use crate::{
    Error, HostId, PoolDissolve, PoolOperation, TaskId,
    pool_dissolve::{Evidence, Kind, Plan},
    xapi::Session,
};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use xenadmin_xapi::records::PoolRecord;
fn operation(r: &ResourceOperationRecord) -> Result<&PoolDissolve, Error> {
    match &r.action {
        ResourceAction::DissolvePool(op) => Ok(op),
        _ => Err(Error::InvalidResourceCommand),
    }
}
fn plan(r: &ResourceOperationRecord) -> Result<&Plan, Error> {
    r.pool_dissolve_plan.as_deref().ok_or(Error::Journal)
}
pub(super) async fn state(s: &Session, r: &ResourceOperationRecord) -> Result<String, Error> {
    let op = operation(r)?;
    let (pool, record) = xapi::read(s, ResourceTarget::Pool(r.pool)).await?;
    if PoolRecord::new(&record).ha_enabled() != false {
        return Err(Error::ResourceInUse);
    }
    let hosts = s.call("host.get_all_records", vec![]).await?;
    let hosts = hosts
        .as_object()
        .filter(|m| !m.is_empty() && m.len() <= 128)
        .ok_or(Error::ResourceConflict)?;
    let mut actual = BTreeSet::new();
    let mut coordinator = false;
    for (reference, h) in hosts {
        let id = h["uuid"]
            .as_str()
            .and_then(|v| HostId::parse(v).ok())
            .ok_or(Error::ResourceConflict)?;
        if !actual.insert(id) {
            return Err(Error::ResourceConflict);
        }
        if PoolRecord::new(&record).master() == *reference {
            if id != op.coordinator {
                return Err(Error::ResourceConflict);
            }
            coordinator = true;
        }
    }
    let mut expected = op
        .members
        .iter()
        .map(|m| m.host)
        .chain([op.coordinator])
        .collect::<BTreeSet<_>>();
    let mut name = op.expected_name.as_str();
    let mut description = op.expected_description.as_str();
    if let Some(p) = &r.pool_dissolve_plan {
        for (k, e) in Plan::kinds(op).iter().zip(&p.evidence) {
            if !e.completed {
                continue;
            }
            match k {
                Kind::Eject(host) => {
                    expected.remove(host);
                }
                Kind::ClearName => name = "",
                Kind::ClearDescription => description = "",
            }
        }
    }
    if !coordinator
        || expected != actual
        || PoolRecord::new(&record).name_label() != name
        || PoolRecord::new(&record).name_description() != description
    {
        return Err(Error::ResourceConflict);
    }
    Ok(pool)
}
async fn eject_arguments(
    s: &Session,
    r: &ResourceOperationRecord,
    host: HostId,
) -> Result<Vec<Value>, Error> {
    let member = operation(r)?
        .members
        .iter()
        .find(|m| m.host == host)
        .ok_or(Error::Journal)?;
    let mut single = r.clone();
    single.action = ResourceAction::Pool(PoolOperation::EjectHost {
        host,
        expected_name: member.name.clone(),
    });
    super::pool::arguments(s, &single).await
}
pub(super) async fn prepare(s: &Session, r: &ResourceOperationRecord) -> Result<Plan, Error> {
    let op = operation(r)?;
    op.validate(r.target)?;
    state(s, r).await?;
    for h in &op.members {
        eject_arguments(s, r, h.host).await?;
    }
    state(s, r).await?;
    let tasks = s.call("task.get_all_records", vec![]).await?;
    let tasks = tasks
        .as_object()
        .filter(|m| m.len() <= 100_000)
        .ok_or(Error::ResourceConflict)?;
    let tasks_before = tasks
        .values()
        .map(|t| {
            t["uuid"]
                .as_str()
                .and_then(|v| TaskId::parse(v).ok())
                .ok_or(Error::ResourceConflict)
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    if tasks_before.len() != tasks.len() {
        return Err(Error::ResourceConflict);
    }
    Ok(Plan {
        pool: r.pool,
        operation: op.clone(),
        evidence: vec![Evidence::default(); Plan::kinds(op).len()],
        tasks_before,
    })
}
pub(super) async fn arguments(
    s: &Session,
    r: &ResourceOperationRecord,
    index: usize,
) -> Result<(&'static str, Vec<Value>), Error> {
    let p = plan(r)?;
    let kinds = Plan::kinds(&p.operation);
    let kind = kinds.get(index).ok_or(Error::Journal)?;
    if p.evidence.get(index).is_none_or(|e| e.sent)
        || p.evidence[..index].iter().any(|e| !e.verified)
    {
        return Err(Error::JournalChanged);
    }
    let pool = state(s, r).await?;
    let args = match kind {
        Kind::Eject(host) => eject_arguments(s, r, *host).await?,
        Kind::ClearName | Kind::ClearDescription => {
            let (_, fresh) = xapi::read(s, ResourceTarget::Pool(r.pool)).await?;
            if !fresh["current_operations"]
                .as_object()
                .is_some_and(|m| m.is_empty())
            {
                return Err(Error::ResourceInUse);
            }
            vec![json!(pool), json!("")]
        }
    };
    state(s, r).await?;
    Ok((kind.method(), args))
}
pub(super) async fn task(
    s: &Session,
    r: &ResourceOperationRecord,
    index: usize,
) -> Result<(TaskId, Value), Error> {
    let p = plan(r)?;
    let e = p.evidence.get(index).ok_or(Error::Journal)?;
    let reference = e.task.as_ref().ok_or(Error::Protocol(
        "Ejection reply was lost; host disappearance cannot prove task completion",
    ))?;
    let value = s.call("task.get_record", vec![json!(reference)]).await?;
    let id = value["uuid"]
        .as_str()
        .and_then(|v| TaskId::parse(v).ok())
        .ok_or(Error::ResourceConflict)?;
    if p.tasks_before.contains(&id)
        || e.task_uuid.is_some_and(|old| old != id)
        || p.evidence
            .iter()
            .enumerate()
            .any(|(n, other)| n != index && other.task_uuid == Some(id))
    {
        return Err(Error::ResourceConflict);
    }
    Ok((id, value))
}
pub(super) async fn verify(
    s: &Session,
    r: &mut ResourceOperationRecord,
    index: usize,
) -> Result<(), Error> {
    if !plan(r)?.evidence.get(index).is_some_and(|e| e.completed) {
        return Err(Error::ResourceConflict);
    }
    state(s, r).await?;
    r.pool_dissolve_plan
        .as_mut()
        .ok_or(Error::Journal)?
        .evidence[index]
        .verified = true;
    r.pool_dissolve = Some(plan(r)?.result());
    Ok(())
}
