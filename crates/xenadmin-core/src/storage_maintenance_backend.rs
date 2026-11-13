use super::*;
use crate::{
    ReviewedStorageAttachment, StorageMaintenance,
    storage_maintenance::{Evidence, Kind, Plan},
    xapi::Session,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use xenadmin_xapi::enumerations::storage_operations;
use xenadmin_xapi::records::{PbdRecord, PoolRecord, SrRecord, TaskRecord};
fn conflict<T>() -> Result<T, Error> {
    Err(Error::ResourceConflict)
}
fn idle(v: &Value) -> Result<(), Error> {
    if v["current_operations"]
        .as_object()
        .is_some_and(|m| m.is_empty())
    {
        Ok(())
    } else {
        Err(Error::Busy)
    }
}
fn cache(v: &Value) -> Result<Option<String>, Error> {
    let m = v["other_config"]
        .as_object()
        .ok_or(Error::ResourceConflict)?;
    match m.get("o_direct") {
        None => Ok(None),
        Some(Value::String(s)) if matches!(s.as_str(), "true" | "false") => Ok(Some(s.clone())),
        _ => conflict(),
    }
}
async fn storage(
    s: &Session,
    r: &ResourceOperationRecord,
    backend: Option<&str>,
) -> Result<(String, Value), Error> {
    super::disk::pool(s, r.pool).await?;
    let (reference, v) = xapi::read(s, r.target).await?;
    idle(&v)?;
    let kind = v["type"]
        .as_str()
        .filter(|v| !v.is_empty() && v.len() <= 128)
        .ok_or(Error::ResourceConflict)?;
    if backend.is_some_and(|b| b != kind) {
        return conflict();
    }
    Ok((reference, v))
}
fn config(v: &Value) -> Result<String, Error> {
    let m = PbdRecord::new(v)
        .device_config()
        .entries()
        .filter(|m| m.len() <= 256)
        .ok_or(Error::ResourceConflict)?;
    if m.iter()
        .any(|(k, v)| k.len() > 4096 || v.as_str().is_none_or(|s| s.len() > 65536))
    {
        return conflict();
    }
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(m).map_err(|_| Error::ResourceConflict)?,)
    ))
}
async fn attachment(
    s: &Session,
    r: &ResourceOperationRecord,
    sr_ref: &str,
    sr: &Value,
    p: &ReviewedStorageAttachment,
    attached: bool,
    expected: Option<&str>,
) -> Result<(String, String), Error> {
    let (host_ref, host) = xapi::read(s, ResourceTarget::Host(p.host)).await?;
    idle(&host)?;
    let (reference, v) = xapi::read(s, ResourceTarget::Pbd(p.attachment)).await?;
    if PbdRecord::new(&v).sr() != sr_ref
        || PbdRecord::new(&v).host() != host_ref
        || PbdRecord::new(&v).currently_attached() != attached
        || crate::storage::safe_location_for_kind(
            PbdRecord::new(&v).device_config(),
            SrRecord::new(sr).type_().text(),
        ) != p.location
        || crate::storage::requires_credentials(PbdRecord::new(&v).device_config())
            != p.requires_credentials
        || !SrRecord::new(sr)
            .pbds()
            .members()
            .is_some_and(|a| a.contains(&json!(reference)))
    {
        return conflict();
    }
    crate::storage_local::attachment_graph(sr, &reference)?;
    let fingerprint = config(&v)?;
    if expected.is_some_and(|v| v != fingerprint) {
        return conflict();
    }
    let _ = r;
    Ok((reference, fingerprint))
}
async fn coordinator(
    s: &Session,
    r: &ResourceOperationRecord,
    expected: HostId,
) -> Result<(), Error> {
    let p = super::disk::pool(s, r.pool).await?;
    let (h, _) = xapi::read(s, ResourceTarget::Host(expected)).await?;
    if PoolRecord::new(&p).master() != h {
        return conflict();
    }
    Ok(())
}
pub(super) async fn prepare(s: &Session, r: &ResourceOperationRecord) -> Result<Plan, Error> {
    let ResourceAction::StorageMaintenance(operation) = &r.action else {
        return Err(Error::InvalidResourceCommand);
    };
    operation.validate(r.target)?;
    let ResourceTarget::Storage(sr) = r.target else {
        return conflict();
    };
    let (sr_ref, v) = storage(s, r, None).await?;
    let backend = v["type"]
        .as_str()
        .ok_or(Error::ResourceConflict)?
        .to_owned();
    let mut configuration = BTreeMap::new();
    match operation {
        StorageMaintenance::Repair {
            coordinator: host,
            attachments,
        } => {
            coordinator(s, r, *host).await?;
            if !v["allowed_operations"]
                .as_array()
                .is_some_and(|a| a.iter().any(|v| v == storage_operations::PLUG))
            {
                return Err(Error::UnavailableAction);
            }
            for p in attachments {
                configuration.insert(
                    p.attachment,
                    attachment(s, r, &sr_ref, &v, p, false, None).await?.1,
                );
            }
        }
        StorageMaintenance::ReadCaching { expected, .. } => {
            if !matches!(backend.as_str(), "nfs" | "ext" | "smb") || cache(&v)? != *expected {
                return conflict();
            }
            super::storage::capability(s, &backend, "VDI_READ_CACHING").await?;
        }
    }
    let tasks = if matches!(operation, StorageMaintenance::Repair { .. }) {
        crate::security_discovery::records(s, "task", 100_000).await?
    } else {
        Default::default()
    };
    let tasks_before = tasks
        .values()
        .map(|v| {
            TaskRecord::new(v)
                .uuid()
                .text()
                .and_then(|v| TaskId::parse(v).ok())
                .ok_or(Error::ResourceConflict)
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    if tasks_before.len() != tasks.len() || tasks_before.iter().any(|id| id.uuid().is_nil()) {
        return conflict();
    }
    let kinds = Plan::kinds(operation);
    Ok(Plan {
        pool: r.pool,
        sr,
        operation: operation.clone(),
        backend,
        configuration,
        tasks_before,
        evidence: vec![Evidence::default(); kinds.len()],
        kinds,
    })
}
pub(super) async fn arguments(
    s: &Session,
    r: &ResourceOperationRecord,
    index: usize,
) -> Result<(&'static str, Vec<Value>), Error> {
    let p = r.storage_maintenance_plan.as_ref().ok_or(Error::Journal)?;
    p.operation
        .permission_requirements()
        .guard(s.permissions())?;
    let (sr_ref, v) = storage(s, r, Some(&p.backend)).await?;
    match &p.operation {
        StorageMaintenance::Repair {
            coordinator: host,
            attachments,
        } => {
            coordinator(s, r, *host).await?;
            if !v["allowed_operations"]
                .as_array()
                .is_some_and(|a| a.iter().any(|v| v == storage_operations::PLUG))
            {
                return Err(Error::UnavailableAction);
            }
            let Kind::Plug(id) = p.kinds[index] else {
                return conflict();
            };
            // Revalidate all reviewed hosts before each write, including earlier successes.
            let mut selected = None;
            for a in attachments {
                let completed = p
                    .kinds
                    .iter()
                    .position(|k| *k == Kind::Plug(a.attachment))
                    .is_some_and(|n| p.evidence[n].verified);
                let checked = attachment(
                    s,
                    r,
                    &sr_ref,
                    &v,
                    a,
                    completed,
                    p.configuration.get(&a.attachment).map(String::as_str),
                )
                .await?;
                if a.attachment == id {
                    selected = Some(checked.0);
                }
            }
            // Bind the selected PBD to the last fresh configuration used for dispatch.
            let a = attachments
                .iter()
                .find(|a| a.attachment == id)
                .ok_or(Error::Journal)?;
            let final_ref = attachment(
                s,
                r,
                &sr_ref,
                &v,
                a,
                false,
                p.configuration.get(&id).map(String::as_str),
            )
            .await?
            .0;
            if selected.as_ref() != Some(&final_ref) {
                return conflict();
            }
            Ok(("Async.PBD.plug", vec![json!(final_ref)]))
        }
        StorageMaintenance::ReadCaching { expected, .. } => {
            super::storage::capability(s, &p.backend, "VDI_READ_CACHING").await?;
            let (sr_ref, v) = storage(s, r, Some(&p.backend)).await?;
            let expected = if index > 0 { None } else { expected.clone() };
            if cache(&v)? != expected {
                return conflict();
            }
            match p.kinds[index] {
                Kind::RemoveOverride => Ok((
                    "SR.remove_from_other_config",
                    vec![json!(sr_ref), json!("o_direct")],
                )),
                Kind::DisableCaching => Ok((
                    "SR.add_to_other_config",
                    vec![json!(sr_ref), json!("o_direct"), json!("true")],
                )),
                _ => conflict(),
            }
        }
    }
}
pub(super) async fn verify(
    s: &Session,
    r: &mut ResourceOperationRecord,
    index: usize,
) -> Result<(), Error> {
    let p = r.storage_maintenance_plan.as_ref().ok_or(Error::Journal)?;
    if !p.evidence[index].completed {
        return Err(Error::Protocol(
            "Storage completion receipt unavailable; no write will be replayed",
        ));
    }
    let (sr_ref, v) = storage(s, r, Some(&p.backend)).await?;
    match &p.kinds[index] {
        Kind::Plug(id) => {
            let StorageMaintenance::Repair { attachments, .. } = &p.operation else {
                return conflict();
            };
            let a = attachments
                .iter()
                .find(|a| a.attachment == *id)
                .ok_or(Error::Journal)?;
            attachment(
                s,
                r,
                &sr_ref,
                &v,
                a,
                true,
                p.configuration.get(id).map(String::as_str),
            )
            .await?;
        }
        Kind::RemoveOverride => {
            if cache(&v)?.is_some() {
                return conflict();
            }
        }
        Kind::DisableCaching => {
            if cache(&v)?.as_deref() != Some("true") {
                return conflict();
            }
        }
    }
    r.storage_maintenance_plan
        .as_mut()
        .ok_or(Error::Journal)?
        .evidence[index]
        .verified = true;
    Ok(())
}
pub(super) async fn task(
    s: &Session,
    r: &ResourceOperationRecord,
    index: usize,
) -> Result<(TaskId, Value), Error> {
    let p = r.storage_maintenance_plan.as_ref().ok_or(Error::Journal)?;
    let e = &p.evidence[index];
    let reference = e.task.as_ref().ok_or(Error::Protocol(
        "Storage task receipt unavailable; no write will be replayed",
    ))?;
    let value = s.call("task.get_record", vec![json!(reference)]).await?;
    let id = TaskRecord::new(&value)
        .uuid()
        .text()
        .and_then(|v| TaskId::parse(v).ok())
        .ok_or(Error::ResourceConflict)?;
    if id.uuid().is_nil()
        || p.tasks_before.contains(&id)
        || e.task_uuid.is_some_and(|old| old != id)
        || p.evidence
            .iter()
            .enumerate()
            .any(|(n, e)| n != index && e.task_uuid == Some(id))
    {
        return conflict();
    }
    Ok((id, value))
}
