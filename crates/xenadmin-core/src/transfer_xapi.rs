use super::*;
use crate::{Endpoint, NetworkId, VdiId, xapi::Session};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Baseline {
    pub vms: Vec<Uuid>,
    pub disks: Vec<Uuid>,
    pub networks: Vec<Uuid>,
}
fn id(r: &Value) -> Result<Uuid, Error> {
    r["uuid"]
        .as_str()
        .and_then(|s| s.parse().ok())
        .ok_or(Error::TransferConflict)
}
fn idle(r: &Value) -> Result<(), Error> {
    if r["current_operations"]
        .as_object()
        .is_some_and(|r| r.is_empty())
    {
        Ok(())
    } else {
        Err(Error::Busy)
    }
}
fn refs(r: &Value, key: &str) -> Result<Vec<String>, Error> {
    let values = r[key]
        .as_array()
        .filter(|a| a.len() <= 256)
        .ok_or(Error::TransferConflict)?;
    values.iter().map(crate::snapshots::reference).collect()
}
async fn fresh(s: &Session, class: &str, id: Uuid) -> Result<(String, Value), Error> {
    crate::snapshots::optional_record(s, class, &id.to_string())
        .await?
        .ok_or(Error::TransferConflict)
}
async fn records(s: &Session, class: &str) -> Result<BTreeMap<String, Value>, Error> {
    let value = s.call(&format!("{class}.get_all_records"), vec![]).await?;
    let values = value
        .as_object()
        .filter(|v| v.len() <= 10000)
        .ok_or(Error::TransferLimit)?;
    let mut ids = BTreeSet::new();
    for r in values.values() {
        if !ids.insert(id(r)?) {
            return Err(Error::TransferConflict);
        }
    }
    Ok(values.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
}
pub(crate) async fn baseline(s: &Session) -> Result<Baseline, Error> {
    Ok(Baseline {
        vms: crate::device_lifecycle::baseline(s, "VM").await?,
        disks: crate::device_lifecycle::baseline(s, "VDI").await?,
        networks: crate::device_lifecycle::baseline(s, "network").await?,
    })
}
pub(crate) async fn prepare(
    s: &Session,
    pool: crate::PoolId,
    action: &TransferAction,
    plan: Option<&Plan>,
) -> Result<http::Prepared, Error> {
    if let TransferAction::Host {
        host,
        expected_version,
        operation,
    } = action
    {
        return crate::host_support::prepare(s, pool, *host, expected_version, operation).await;
    }
    if matches!(
        action,
        TransferAction::ImportDisk { .. } | TransferAction::ExportDisk { .. }
    ) {
        return super::disk::prepare(s, pool, action, plan).await;
    }
    if let TransferAction::ImportXva { sr } | TransferAction::ImportCheckpointXva { sr } = action {
        let summary = plan
            .and_then(|p| p.file.manifest.as_ref())
            .ok_or(Error::TransferConflict)?;
        if (summary.memory_images() > 0)
            != matches!(action, TransferAction::ImportCheckpointXva { .. })
        {
            return Err(Error::TransferConflict);
        }
        if s.options().read_only {
            return Err(Error::ReadOnly);
        }
        let mut prepared = storage_route(s, pool, *sr, true).await?;
        let (reference, _) = fresh(s, "SR", sr.uuid()).await?;
        prepared.kind = http::Kind::Archive;
        prepared.query = vec![
            ("sr_id".into(), reference),
            ("restore".into(), "false".into()),
            ("force".into(), "false".into()),
        ];
        return Ok(prepared);
    }
    let _ = fresh(s, "pool", pool.uuid()).await?;
    let hosts = records(s, "host").await?;
    let known_hosts = hosts
        .values()
        .map(|r| {
            r["address"]
                .as_str()
                .filter(|s| s.len() <= 253 && !s.contains('/'))
                .map(str::to_owned)
                .ok_or(Error::TransferEndpoint)
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    let endpoint = s.options().endpoint.clone();
    let mut query = vec![];
    match action {
        TransferAction::ExportXva {
            source,
            compression,
        } => {
            let (reference, r) = match source {
                ExportSource::Vm {
                    vm,
                    expected_template,
                } => {
                    let (r, v) = fresh(s, "VM", vm.uuid()).await?;
                    if v["is_a_template"] != *expected_template || v["is_a_snapshot"] != false {
                        return Err(Error::TransferConflict);
                    }
                    (r, v)
                }
                ExportSource::Snapshot { snapshot, parent }
                | ExportSource::Checkpoint {
                    snapshot, parent, ..
                } => {
                    let (parent, _) = fresh(s, "VM", parent.uuid()).await?;
                    let (r, v) = fresh(s, "VM", snapshot.uuid()).await?;
                    if v["is_a_snapshot"] != true || v["snapshot_of"] != parent {
                        return Err(Error::TransferConflict);
                    }
                    (r, v)
                }
            };
            if let ExportSource::Checkpoint { memory, .. } = source {
                super::checkpoint::fresh(s, &r, memory).await?;
                let vms = records(s, "VM").await?;
                if vms
                    .iter()
                    .filter(|(_, v)| v["suspend_VDI"] == r["suspend_VDI"])
                    .map(|(k, _)| k)
                    .collect::<Vec<_>>()
                    != vec![&reference]
                {
                    return Err(Error::TransferConflict);
                }
            }
            idle(&r)?;
            if r["is_control_domain"] != false
                || r["power_state"]
                    != if matches!(source, ExportSource::Checkpoint { .. }) {
                        "Suspended"
                    } else {
                        "Halted"
                    }
                || r["blocked_operations"]
                    .as_object()
                    .is_some_and(|v| v.contains_key("export"))
                || !crate::xapi::advertises(&r, "export")
            {
                return Err(Error::UnavailableAction);
            }
            query.push(("uuid".into(), id(&r)?.to_string()));
            query.push(("use_compression".into(), compression.query().into()));
            query.push((
                "preserve_power_state".into(),
                matches!(
                    source,
                    ExportSource::Checkpoint {
                        preserve_memory: true,
                        ..
                    }
                )
                .to_string(),
            ));
        }
        TransferAction::ImportDisk { .. } | TransferAction::ExportDisk { .. } => {
            return Err(Error::TransferConflict);
        }
        TransferAction::Host { .. }
        | TransferAction::ImportXva { .. }
        | TransferAction::ImportCheckpointXva { .. }
        | TransferAction::ImportOvf { .. }
        | TransferAction::ExportOvf { .. } => {
            return Err(Error::TransferConflict);
        }
    }
    Ok(http::Prepared {
        peer_identity: None,
        kind: http::Kind::Archive,
        endpoint,
        known_hosts,
        query,
        upload: !action.is_export(),
    })
}
pub(crate) fn result(task: &Value) -> Result<Vec<String>, Error> {
    let xml = task["result"]
        .as_str()
        .filter(|s| s.len() <= 64 * 1024)
        .ok_or(Error::TransferConflict)?;
    let value = super::archive::xml_value(xml)?;
    let values = value
        .as_array()
        .filter(|a| !a.is_empty() && a.len() <= 64)
        .ok_or(Error::TransferConflict)?;
    let refs = values
        .iter()
        .map(crate::snapshots::reference)
        .collect::<Result<Vec<_>, _>>()?;
    if refs.iter().collect::<BTreeSet<_>>().len() != refs.len() {
        return Err(Error::TransferConflict);
    }
    Ok(refs)
}
pub(crate) async fn created(
    s: &Session,
    plan: &Plan,
    task: &str,
    destination: SrId,
    complete: bool,
) -> Result<TransferCreated, Error> {
    let baseline = plan.baseline.as_ref().ok_or(Error::TransferConflict)?;
    let vms = records(s, "VM").await?;
    let disks = records(s, "VDI").await?;
    let networks = records(s, "network").await?;
    let mut result = TransferCreated::default();
    let mut found = BTreeSet::new();
    let mut graph_disks = BTreeSet::new();
    let mut graph_memory = BTreeSet::new();
    let mut names = vec![];
    for (reference, r) in &vms {
        if r["other_config"]["import_task"] != task {
            continue;
        }
        let uuid = id(r)?;
        if baseline.vms.contains(&uuid) || r["is_control_domain"] != false {
            return Err(Error::TransferConflict);
        }
        if complete {
            idle(r)?;
            if !matches!(r["power_state"].as_str(), Some("Halted" | "Suspended"))
                || r["is_a_snapshot"] != false
            {
                return Err(Error::TransferConflict);
            }
        }
        let template = r["is_a_template"]
            .as_bool()
            .ok_or(Error::TransferConflict)?;
        let memory = match r["suspend_VDI"].as_str() {
            Some(reference) if !matches!(reference, "OpaqueRef:NULL" | "Ref:NULL") => {
                if !graph_memory.insert(reference.to_owned()) {
                    return Err(Error::TransferConflict);
                }
                let d = disks.get(reference).ok_or(Error::TransferConflict)?;
                Some(super::checkpoint::size(d)?)
            }
            _ => None,
        };
        if complete && ((r["power_state"] == "Suspended") != memory.is_some()) {
            return Err(Error::TransferConflict);
        }
        names.push((
            r["name_label"]
                .as_str()
                .ok_or(Error::TransferConflict)?
                .to_owned(),
            template,
            memory,
        ));
        found.insert(reference.clone());
        result.vms.push(VmId::from(uuid));
        for vbd in refs(r, "VBDs")? {
            let a = s.call("VBD.get_record", vec![json!(vbd)]).await?;
            if a["VM"] != *reference {
                return Err(Error::TransferConflict);
            }
            if a["type"] == "Disk" && a["empty"] == false {
                graph_disks.insert(crate::snapshots::reference(&a["VDI"])?);
            }
        }
    }
    for (reference, r) in &disks {
        let tagged = r["sm_config"]["import_task"] == task;
        if !tagged && !graph_disks.contains(reference) && !graph_memory.contains(reference) {
            continue;
        }
        let uuid = id(r)?;
        let memory = r["type"] == "suspend";
        if baseline.disks.contains(&uuid)
            || (!memory && r["type"] != "user")
            || r["managed"] != true
            || (memory
                && (!graph_memory.contains(reference)
                    || graph_disks.contains(reference)
                    || !refs(r, "VBDs")?.is_empty()))
            || (memory
                && vms
                    .iter()
                    .any(|(vm, v)| v["suspend_VDI"] == *reference && !found.contains(vm)))
        {
            return Err(Error::TransferConflict);
        }
        let sr = crate::snapshots::reference(&r["SR"])?;
        let sr = s.call("SR.get_record", vec![json!(sr)]).await?;
        if id(&sr)? != destination.uuid() {
            return Err(Error::TransferConflict);
        }
        if complete {
            idle(r)?;
            if r["missing"] != false {
                return Err(Error::TransferConflict);
            }
        }
        for attachment in refs(r, "VBDs")? {
            let a = s.call("VBD.get_record", vec![json!(attachment)]).await?;
            if !found.contains(a["VM"].as_str().ok_or(Error::TransferConflict)?) {
                return Err(Error::TransferConflict);
            }
        }
        if memory {
            result.suspend_images.push(VdiId::from(uuid));
        } else {
            result.disks.push(VdiId::from(uuid));
        }
    }
    for r in networks.values() {
        if r["other_config"]["import_task"] == task {
            let uuid = id(r)?;
            if baseline.networks.contains(&uuid) {
                return Err(Error::TransferConflict);
            }
            result.networks.push(NetworkId::from(uuid));
        }
    }
    let manifest = plan.file.manifest.as_ref().ok_or(Error::TransferConflict)?;
    if result.vms.len() > manifest.guests.len()
        || result.disks.len() > manifest.data_disks
        || result.suspend_images.len() > manifest.memory_images()
        || result.networks.len() > manifest.networks
    {
        return Err(Error::TransferConflict);
    }
    if complete {
        let expected = plan
            .result
            .as_ref()
            .ok_or(Error::TransferConflict)?
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        if expected != found {
            return Err(Error::TransferConflict);
        }
        let mut expected_names = manifest
            .guests
            .iter()
            .map(|g| {
                (
                    g.name.clone(),
                    g.template,
                    g.memory.as_ref().map(|m| m.virtual_size),
                )
            })
            .collect::<Vec<_>>();
        names.sort();
        expected_names.sort();
        if names != expected_names
            || result.disks.len() != manifest.data_disks
            || result.suspend_images.len() != manifest.memory_images()
        {
            return Err(Error::TransferConflict);
        }
    }
    result.vms.sort();
    result.disks.sort();
    result.networks.sort();
    result.suspend_images.sort();
    Ok(result)
}

pub(crate) async fn storage_route(
    s: &Session,
    pool: crate::PoolId,
    sr: SrId,
    write: bool,
) -> Result<http::Prepared, Error> {
    let (_, pool) = fresh(s, "pool", pool.uuid()).await?;
    let hosts = records(s, "host").await?;
    let known_hosts = hosts
        .values()
        .map(|h| {
            h["address"]
                .as_str()
                .filter(|v| v.len() <= 253 && !v.contains('/'))
                .map(str::to_owned)
                .ok_or(Error::TransferEndpoint)
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    let (reference, r) = fresh(s, "SR", sr.uuid()).await?;
    idle(&r)?;
    if r["content_type"] != "user" || (write && !crate::xapi::advertises(&r, "vdi_create")) {
        return Err(Error::UnavailableAction);
    }
    let mut candidates = vec![];
    for p in refs(&r, "PBDs")? {
        let p = s.call("PBD.get_record", vec![json!(p)]).await?;
        if p["SR"] != reference {
            return Err(Error::TransferConflict);
        }
        if p["currently_attached"] == true {
            let h = crate::snapshots::reference(&p["host"])?;
            let host = hosts.get(&h).ok_or(Error::TransferConflict)?;
            if host["enabled"] == true {
                candidates.push((h, host));
            }
        }
    }
    candidates.sort_by_key(|(r, _)| r != pool["master"].as_str().unwrap_or_default());
    let (_, host) = candidates.first().ok_or(Error::UnavailableAction)?;
    let address = host["address"].as_str().ok_or(Error::TransferEndpoint)?;
    let mut endpoint = s.options().endpoint.clone();
    let original = url::Url::parse(endpoint.as_str()).map_err(|_| Error::TransferEndpoint)?;
    if original.host_str() != Some(address) {
        let mut target = original;
        target
            .set_host(Some(address))
            .map_err(|_| Error::TransferEndpoint)?;
        target
            .set_scheme("https")
            .map_err(|_| Error::TransferEndpoint)?;
        target
            .set_port(Some(443))
            .map_err(|_| Error::TransferEndpoint)?;
        endpoint = Endpoint::parse(target.as_str()).map_err(|_| Error::TransferEndpoint)?;
    }
    Ok(http::Prepared {
        peer_identity: None,
        kind: http::Kind::Disk,
        endpoint,
        known_hosts,
        query: vec![],
        upload: write,
    })
}
