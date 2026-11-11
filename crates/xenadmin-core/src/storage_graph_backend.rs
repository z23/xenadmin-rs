use super::*;
use crate::{
    ReviewedPbd, StorageGraph,
    storage_graph::{Evidence, Kind, Plan},
    xapi::Session,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use xenadmin_xapi::records::{PbdRecord, PoolRecord, SrRecord, VbdRecord, VdiRecord, VmRecord};
fn require(ok: bool, msg: &'static str) -> Result<(), Error> {
    if ok {
        Ok(())
    } else {
        Err(Error::StoragePrerequisite(msg))
    }
}
fn idle(v: &Value) -> Result<(), Error> {
    require(
        v["current_operations"]
            .as_object()
            .is_some_and(|m| m.is_empty()),
        "Storage or host has an active operation",
    )
}
fn reference(v: &Value) -> Result<String, Error> {
    crate::snapshots::reference(v)
}
fn uuid(v: &Value) -> Result<uuid::Uuid, Error> {
    v["uuid"]
        .as_str()
        .and_then(|v| v.parse().ok())
        .filter(|id: &uuid::Uuid| !id.is_nil())
        .ok_or(Error::ResourceConflict)
}
async fn records(s: &Session, class: &str, limit: usize) -> Result<BTreeMap<String, Value>, Error> {
    let v = s.call(&format!("{class}.get_all_records"), vec![]).await?;
    let m = v
        .as_object()
        .filter(|m| m.len() <= limit)
        .ok_or(Error::ResourceConflict)?;
    let mut ids = BTreeSet::new();
    for (k, v) in m {
        reference(&json!(k))?;
        require(ids.insert(uuid(v)?), "Duplicate storage graph identity")?;
    }
    Ok(m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
}
fn config(v: &Value) -> Result<&serde_json::Map<String, Value>, Error> {
    let m = PbdRecord::new(v)
        .device_config()
        .value()
        .as_object()
        .filter(|m| m.len() <= 128)
        .ok_or(Error::ResourceConflict)?;
    require(
        m.iter()
            .all(|(k, v)| k.len() <= 128 && v.as_str().is_some_and(|v| v.len() <= 65536)),
        "Invalid attachment configuration",
    )?;
    Ok(m)
}
fn digest(m: &serde_json::Map<String, Value>) -> Result<String, Error> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(m).map_err(|_| Error::ResourceConflict)?)
    ))
}
fn fingerprint(v: &Value) -> Result<String, Error> {
    digest(config(v)?)
}
/// The iSCSI driver persists the portal list it discovers on a multipath plug as
/// `multihomelist` in the attachment configuration (lab finding 34). That bookkeeping is not
/// an operator change, so a reviewed hash may also match the configuration without it.
fn fingerprint_without_discovery(v: &Value, sr: &Value) -> Result<Option<String>, Error> {
    if sr["type"] != "lvmoiscsi" {
        return Ok(None);
    }
    let mut m = config(v)?.clone();
    if m.remove("multihomelist").is_none() {
        return Ok(None);
    }
    digest(&m).map(Some)
}
fn secret_keys() -> [&'static str; 3] {
    [
        "password_secret",
        "chappassword_secret",
        "incoming_chappassword_secret",
    ]
}
fn source_config(v: &Value, location: &crate::StorageLocation) -> Result<(), Error> {
    let m = config(v)?;
    let mut allowed = location
        .device_config()
        .as_object()
        .ok_or(Error::ResourceConflict)?
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    allowed.extend(["username", "chapuser", "incoming_chapuser"].map(str::to_owned));
    allowed.extend(secret_keys().map(str::to_owned));
    require(
        m.keys().all(|k| allowed.contains(k)),
        "Custom or host-dependent attachment configuration needs separate review; automatic reconstruction is unavailable",
    )?;
    require(
        crate::storage::safe_location_from_config(&PbdRecord::new(v).device_config()).as_ref()
            == Some(location),
        "Source attachment location changed",
    )?;
    for key in secret_keys() {
        if let Some(v) = m.get(key) {
            let id = v
                .as_str()
                .and_then(|v| crate::SecretId::parse(v).ok())
                .filter(|id| !id.uuid().is_nil())
                .ok_or(Error::ResourceConflict)?;
            let _ = id;
        }
    }
    Ok(())
}
fn find(m: &BTreeMap<String, Value>, id: uuid::Uuid) -> Result<(String, Value), Error> {
    let mut rows = m.iter().filter(|(_, v)| v["uuid"] == id.to_string());
    let (k, v) = rows.next().ok_or(Error::Stale)?;
    require(rows.next().is_none(), "Duplicate storage identity")?;
    Ok((k.clone(), v.clone()))
}
async fn host(s: &Session, id: crate::HostId, live: bool) -> Result<(String, Value), Error> {
    let (h, v) = xapi::read(s, ResourceTarget::Host(id)).await?;
    idle(&v)?;
    if live {
        let m = s
            .call(
                "host_metrics.get_record",
                vec![json!(reference(&v["metrics"])?)],
            )
            .await?;
        require(m["live"] == true, "Selected host must be online")?;
    }
    Ok((h, v))
}
async fn storage(s: &Session, id: crate::SrId) -> Result<(String, Value), Error> {
    let (r, v) = xapi::read(s, ResourceTarget::Storage(id)).await?;
    idle(&v)?;
    Ok((r, v))
}
fn attachment(
    v: &Value,
    p: &ReviewedPbd,
    host_ref: &str,
    sr_ref: &str,
    sr: &Value,
    attached: bool,
    hash: Option<&str>,
) -> Result<(), Error> {
    require(
        PbdRecord::new(v).uuid() == p.pbd.to_string()
            && PbdRecord::new(v).host() == host_ref
            && PbdRecord::new(v).sr() == sr_ref
            && PbdRecord::new(v).currently_attached() == attached
            && crate::storage::safe_location_for_kind(
                &PbdRecord::new(v).device_config(),
                sr["type"].as_str(),
            ) == p.location
            && crate::storage::requires_credentials(&PbdRecord::new(v).device_config())
                == p.requires_credentials,
        "Reviewed attachment identity, state or location changed",
    )?;
    if let Some(hash) = hash {
        require(
            fingerprint(v)? == hash
                || fingerprint_without_discovery(v, sr)?.as_deref() == Some(hash),
            "Reviewed attachment configuration changed",
        )?;
    }
    Ok(())
}
fn done(p: &Plan, k: &Kind, after: Option<usize>) -> bool {
    p.kinds
        .iter()
        .enumerate()
        .any(|(n, v)| v == k && (p.evidence[n].verified || after == Some(n)))
}
async fn repair_graph(
    s: &Session,
    r: &ResourceOperationRecord,
    p: &Plan,
    after: Option<usize>,
) -> Result<(String, Value, BTreeMap<String, Value>), Error> {
    let StorageGraph::Repair {
        coordinator,
        source,
        hosts,
    } = &p.operation
    else {
        return Err(Error::Journal);
    };
    let pool = super::disk::pool(s, r.pool).await?;
    let (master, _) = host(s, *coordinator, true).await?;
    require(pool["master"] == master, "Pool coordinator changed")?;
    let (sr_ref, sr) = storage(s, source.sr).await?;
    require(
        SrRecord::new(&sr).shared() == true
            && SrRecord::new(&sr).type_() == source.location.as_ref().ok_or(Error::Journal)?.kind(),
        "Repair requires the same shared storage backend",
    )?;
    let pbds = records(s, "PBD", 10000).await?;
    let (source_ref, source_v) = find(&pbds, source.pbd.uuid())?;
    let (source_host, _) = host(s, source.host, false).await?;
    attachment(
        &source_v,
        source,
        &source_host,
        &sr_ref,
        &sr,
        source.attached || done(p, &Kind::Plug(source.host, source.sr), after),
        p.configuration.get(&source.pbd).map(String::as_str),
    )?;
    source_config(&source_v, source.location.as_ref().ok_or(Error::Journal)?)?;
    for entry in hosts {
        let (h, _) = host(s, entry.host, true).await?;
        let pairs = pbds
            .iter()
            .filter(|(_, v)| v["host"] == h && v["SR"] == sr_ref)
            .collect::<Vec<_>>();
        let expected = p.attachment(entry.host, source.sr);
        match expected {
            None => require(
                pairs.is_empty(),
                "An attachment appeared for a reviewed missing host",
            )?,
            Some(id) => {
                require(
                    pairs.len() == 1 && pairs[0].1["uuid"] == id.to_string(),
                    "Reviewed host attachment graph changed",
                )?;
                let attached = done(p, &Kind::Plug(entry.host, source.sr), after);
                if let Some(a) = &entry.attachment {
                    attachment(
                        pairs[0].1,
                        a,
                        &h,
                        &sr_ref,
                        &sr,
                        attached,
                        p.configuration.get(&id).map(String::as_str),
                    )?;
                } else {
                    let evidence = p
                        .kinds
                        .iter()
                        .zip(&p.evidence)
                        .find_map(|(k, e)| (*k == Kind::Create(entry.host)).then_some(e))
                        .ok_or(Error::Journal)?;
                    verify_clone(s, p, &source_v, pairs[0].1, Some(&evidence.secrets)).await?;
                    require(
                        pairs[0].1["currently_attached"] == attached,
                        "Created attachment state changed",
                    )?;
                }
            }
        }
    }
    require(
        SrRecord::new(&sr).pbds().as_array().is_some_and(|a| {
            a.iter().all(|v| {
                v.as_str()
                    .is_some_and(|k| pbds.get(k).is_some_and(|p| p["SR"] == sr_ref))
            }) && pbds
                .iter()
                .filter(|(_, v)| v["SR"] == sr_ref)
                .all(|(k, _)| a.contains(&json!(k)))
        }),
        "Storage attachment reverse graph is incomplete",
    )?;
    Ok((source_ref, source_v, pbds))
}
async fn verify_clone(
    s: &Session,
    p: &Plan,
    source: &Value,
    v: &Value,
    expected: Option<&Vec<crate::SecretId>>,
) -> Result<Vec<crate::SecretId>, Error> {
    let original = config(source)?;
    let actual = config(v)?;
    require(
        original.len() == actual.len(),
        "Created attachment configuration differs from source",
    )?;
    let mut secrets = vec![];
    for (k, value) in original {
        if secret_keys().contains(&k.as_str()) {
            let id = actual
                .get(k)
                .and_then(Value::as_str)
                .and_then(|v| crate::SecretId::parse(v).ok())
                .filter(|id| !id.uuid().is_nil())
                .ok_or(Error::ResourceConflict)?;
            require(
                !p.secrets_before.contains(&id)
                    && actual.get(k) != Some(value)
                    && !secrets.contains(&id),
                "Attachment credentials were not independently cloned",
            )?;
            let reference = super::storage_auth_backend::secret_reference(s, id).await?;
            require(
                super::storage_auth_backend::secret_uuid(s, &reference).await? == id,
                "Cloned credential identity changed",
            )?;
            secrets.push(id);
        } else {
            require(
                actual.get(k) == Some(value),
                "Created attachment configuration differs from source",
            )?;
        }
    }
    secrets.sort();
    if let Some(expected) = expected {
        require(
            &secrets == expected,
            "Created credential identities changed",
        )?;
    }
    if secrets.is_empty() {
        return Ok(secrets);
    }
    let all = records(s, "PBD", 10000).await?;
    let (_, latest) = find(&all, uuid(v)?)?;
    require(
        PbdRecord::new(&latest).host() == PbdRecord::new(v).host()
            && PbdRecord::new(&latest).sr() == PbdRecord::new(v).sr()
            && PbdRecord::new(&latest).currently_attached()
                == PbdRecord::new(v).currently_attached()
            && PbdRecord::new(&latest).device_config() == PbdRecord::new(v).device_config(),
        "Created attachment changed during credential verification",
    )?;
    for (_, other) in all {
        if other["uuid"] == PbdRecord::new(v).uuid() {
            continue;
        }
        for value in config(&other)?.values() {
            require(
                !secrets.iter().any(|id| value == &json!(id.to_string())),
                "A cloned storage credential has another attachment consumer",
            )?;
        }
    }
    for (class, field) in [("SR", "sm_config"), ("host", "power_on_config")] {
        for other in records(s, class, 10000).await?.values() {
            let values = other[field].as_object().ok_or(Error::ResourceConflict)?;
            require(
                !values
                    .values()
                    .any(|v| secrets.iter().any(|id| v == &json!(id.to_string()))),
                "Cloned credential has another storage or host consumer",
            )?;
        }
    }
    Ok(secrets)
}
fn ref_set(value: &Value) -> Result<BTreeSet<String>, Error> {
    let a = value
        .as_array()
        .filter(|a| a.len() <= 20000)
        .ok_or(Error::ResourceConflict)?;
    let set = a
        .iter()
        .map(reference)
        .collect::<Result<BTreeSet<_>, _>>()?;
    require(set.len() == a.len(), "Duplicate storage graph reference")?;
    Ok(set)
}
async fn multipath_graph(
    s: &Session,
    r: &ResourceOperationRecord,
    p: &Plan,
    after: Option<usize>,
) -> Result<(String, Value, BTreeMap<String, Value>), Error> {
    let StorageGraph::Multipath {
        host: id,
        expected,
        expected_handler,
        enabled,
        attachments,
    } = &p.operation
    else {
        return Err(Error::Journal);
    };
    let pool = super::disk::pool(s, r.pool).await?;
    require(
        PoolRecord::new(&pool).ha_enabled() == false,
        "Disable HA separately before multipath maintenance",
    )?;
    // XCP-ng keeps the statefile disk listed in `ha_statefiles` after HA is disabled; an
    // enable or disable still in progress shows in the pool's current operations instead.
    require(
        PoolRecord::new(&pool)
            .current_operations()
            .as_object()
            .is_some_and(|m| m.is_empty()),
        "Wait for the pool's active operation before multipath maintenance",
    )?;
    let (h, v) = host(s, *id, true).await?;
    require(
        v["enabled"] == false && v["other_config"]["MAINTENANCE_MODE"] == "true",
        "Host must already be disabled and in maintenance",
    )?;
    let mode = if done(p, &Kind::Mode, after) {
        *enabled
    } else {
        *expected
    };
    require(v["multipathing"] == mode, "Host multipathing mode changed")?;
    let handler = v["other_config"]
        .as_object()
        .ok_or(Error::ResourceConflict)?
        .get("multipathhandle");
    let handler_expected = if done(p, &Kind::AddHandler, after) {
        true
    } else if done(p, &Kind::RemoveHandler, after) {
        false
    } else {
        *expected_handler
    };
    require(
        if handler_expected {
            handler == Some(&json!("dmp"))
        } else {
            handler.is_none()
        },
        "Standard multipath handler changed or is unsupported",
    )?;
    let pbds = records(s, "PBD", 10000).await?;
    let actual = pbds
        .values()
        .filter(|a| a["host"] == h)
        .map(|a| uuid(a).map(crate::PbdId::from))
        .collect::<Result<BTreeSet<_>, _>>()?;
    require(
        actual == attachments.iter().map(|a| a.pbd).collect(),
        "Complete host storage attachment graph changed",
    )?;
    require(
        v["PBDs"].as_array().is_some_and(|a| {
            a.len() == attachments.len()
                && a.iter().all(|v| {
                    v.as_str()
                        .is_some_and(|k| pbds.get(k).is_some_and(|v| v["host"] == h))
                })
        }),
        "Host storage attachment reverse graph is incomplete",
    )?;
    let vms = records(s, "VM", 10000).await?;
    require(
        !vms.values().any(|vm| {
            VmRecord::new(vm).resident_on() == h && VmRecord::new(vm).is_control_domain() != true
        }),
        "Evacuate all guests before multipath maintenance",
    )?;
    let disks = records(s, "VDI", 10000).await?;
    let vbds = records(s, "VBD", 20000).await?;
    for a in attachments {
        let (sr_ref, sr) = storage(s, a.sr).await?;
        let (pbd_ref, pbd) = find(&pbds, a.pbd.uuid())?;
        let attached = if done(p, &Kind::Plug(a.host, a.sr), after) {
            true
        } else {
            a.attached && !done(p, &Kind::Unplug(a.pbd), after)
        };
        attachment(
            &pbd,
            a,
            &h,
            &sr_ref,
            &sr,
            attached,
            p.configuration.get(&a.pbd).map(String::as_str),
        )?;
        crate::storage_local::attachment_graph(&sr, &pbd_ref)?;
        if a.local_cache {
            require(
                v["local_cache_sr"] == sr_ref,
                "The reviewed local-cache repository changed; review the host again",
            )?;
        } else {
            require(
                SrRecord::new(&sr).local_cache_enabled() == false && v["local_cache_sr"] != sr_ref,
                "Storage used for local caching must be reviewed as kept attached",
            )?;
        }
        let block = matches!(
            SrRecord::new(&sr).type_().as_str(),
            Some("lvmoiscsi" | "lvmohba")
        );
        let expected_disks = disks
            .iter()
            .filter(|(_, d)| VdiRecord::new(d).sr() == sr_ref)
            .map(|(k, _)| k.clone())
            .collect::<BTreeSet<_>>();
        require(
            ref_set(&SrRecord::new(&sr).vdis())? == expected_disks,
            "Repository disk graph is incomplete or changed",
        )?;
        for (dref, d) in disks
            .iter()
            .filter(|(_, d)| VdiRecord::new(d).sr() == sr_ref)
        {
            let expected_vbds = vbds
                .iter()
                .filter(|(_, b)| VbdRecord::new(b).vdi() == *dref)
                .map(|(k, _)| k.clone())
                .collect::<BTreeSet<_>>();
            require(
                ref_set(&VdiRecord::new(d).vbds())? == expected_vbds,
                "Disk attachment graph is incomplete or changed",
            )?;
            for bref in &expected_vbds {
                let b = &vbds[bref];
                let vm_ref = reference(&VbdRecord::new(b).vm())?;
                let vm = vms.get(&vm_ref).ok_or(Error::ResourceConflict)?;
                let declared = ref_set(&VmRecord::new(vm).vbds())?;
                require(
                    declared.contains(bref),
                    "Guest disk attachment graph is inconsistent",
                )?;
                for candidate in declared {
                    require(
                        vbds.get(&candidate)
                            .is_some_and(|b| VbdRecord::new(b).vm() == vm_ref),
                        "Guest references a missing or rebound disk attachment",
                    )?;
                }
            }
            idle(d)?;
            require(
                VdiRecord::new(d).type_() != "metadata",
                "Storage metadata replication requires separate maintenance",
            )?;
            for b in vbds.values().filter(|b| VbdRecord::new(b).vdi() == *dref) {
                if VbdRecord::new(b).currently_attached() != false {
                    let vm = reference(&VbdRecord::new(b).vm())?;
                    let resident = vms.get(&vm).ok_or(Error::ResourceConflict)?;
                    require(
                        !block && resident["resident_on"] != h,
                        "Active virtual disks prevent multipath maintenance",
                    )?;
                }
            }
        }
    }
    Ok((h, v, pbds))
}
pub(super) async fn prepare(s: &Session, r: &ResourceOperationRecord) -> Result<Plan, Error> {
    let ResourceAction::StorageGraph(operation) = &r.action else {
        return Err(Error::Journal);
    };
    operation.validate(r.target)?;
    operation.permission_requirements().guard(s.permissions())?;
    let pbds = records(s, "PBD", 10000).await?;
    let tasks = records(s, "task", 100_000).await?;
    let has_secrets =
        matches!(operation,StorageGraph::Repair{source,..} if source.requires_credentials);
    let secrets_before = if has_secrets {
        super::storage_auth_backend::secret_ids(s)
            .await?
            .into_iter()
            .collect()
    } else {
        BTreeSet::new()
    };
    let attachments = match operation {
        StorageGraph::Repair { source, hosts, .. } => std::iter::once(source)
            .chain(hosts.iter().filter_map(|h| h.attachment.as_ref()))
            .collect::<Vec<_>>(),
        StorageGraph::Multipath { attachments, .. } => attachments.iter().collect(),
    };
    let configuration = attachments
        .into_iter()
        .map(|a| {
            let (_, v) = find(&pbds, a.pbd.uuid())?;
            Ok((a.pbd, fingerprint(&v)?))
        })
        .collect::<Result<BTreeMap<_, _>, Error>>()?;
    let kinds = Plan::kinds(operation);
    let p = Plan {
        pool: r.pool,
        target: r.target,
        operation: operation.clone(),
        evidence: vec![Evidence::default(); kinds.len()],
        kinds,
        configuration,
        tasks_before: tasks
            .values()
            .map(|v| uuid(v).map(crate::TaskId::from))
            .collect::<Result<_, _>>()?,
        pbds_before: pbds
            .values()
            .map(|v| uuid(v).map(crate::PbdId::from))
            .collect::<Result<_, _>>()?,
        secrets_before,
        expires_at_ms: now() + 30 * 60 * 1000,
    };
    match operation {
        StorageGraph::Repair { .. } => {
            repair_graph(s, r, &p, None).await?;
        }
        StorageGraph::Multipath { .. } => {
            multipath_graph(s, r, &p, None).await?;
        }
    }
    Ok(p)
}
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
async fn storage_allowed(s: &Session, pbd: &Value, operation: &str) -> Result<(), Error> {
    let sr = s
        .call("SR.get_record", vec![json!(reference(&pbd["SR"])?)])
        .await?;
    idle(&sr)?;
    require(
        crate::xapi::advertises(&sr, operation),
        "Storage currently refuses the reviewed connection change",
    )
}
async fn latest_attachment(
    s: &Session,
    graph: &BTreeMap<String, Value>,
    id: crate::PbdId,
) -> Result<String, Error> {
    let (_, old) = find(graph, id.uuid())?;
    let (reference, v) = xapi::read(s, ResourceTarget::Pbd(id)).await?;
    for field in ["uuid", "host", "SR", "currently_attached", "device_config"] {
        require(
            v[field] == old[field],
            "Storage attachment changed immediately before dispatch",
        )?;
    }
    Ok(reference)
}
pub(super) async fn arguments(
    s: &Session,
    r: &ResourceOperationRecord,
    index: usize,
) -> Result<(&'static str, Vec<Value>), Error> {
    let p = r.storage_graph_plan.as_ref().ok_or(Error::Journal)?;
    require(
        now() < p.expires_at_ms,
        "Storage maintenance review expired; remaining writes will not run",
    )?;
    p.operation
        .permission_requirements()
        .guard(s.permissions())?;
    match &p.operation {
        StorageGraph::Repair { source, .. } => {
            let (_, src, graph) = repair_graph(s, r, p, None).await?;
            let (sr_ref, sr) = storage(s, source.sr).await?;
            match p.kinds[index] {
                Kind::Create(host_id) => {
                    require(
                        crate::xapi::advertises(&sr, "pbd_create"),
                        "Host currently refuses creating a storage attachment",
                    )?;
                    let (h, _) = host(s, host_id, true).await?;
                    let (_, latest) = xapi::read(s, ResourceTarget::Pbd(source.pbd)).await?;
                    require(
                        fingerprint(&latest)? == fingerprint(&src)?
                            && PbdRecord::new(&latest).sr() == PbdRecord::new(&src).sr()
                            && PbdRecord::new(&latest).host() == PbdRecord::new(&src).host()
                            && PbdRecord::new(&latest).currently_attached()
                                == PbdRecord::new(&src).currently_attached(),
                        "Source configuration changed before copy",
                    )?;
                    Ok((
                        "Async.PBD.create",
                        vec![
                            json!({"host":h,"SR":sr_ref,"device_config":PbdRecord::new(&latest).device_config(),"other_config":{}}),
                        ],
                    ))
                }
                Kind::Plug(h, storage_id) => {
                    let id = p.attachment(h, storage_id).ok_or(Error::Journal)?;
                    let reference = latest_attachment(s, &graph, id).await?;
                    require(
                        crate::xapi::advertises(&sr, "plug"),
                        "Storage currently refuses connection",
                    )?;
                    Ok(("Async.PBD.plug", vec![json!(reference)]))
                }
                _ => Err(Error::Journal),
            }
        }
        StorageGraph::Multipath { enabled, .. } => {
            let (h, previous_host, graph) = multipath_graph(s, r, p, None).await?;
            let latest_host = s.call("host.get_record", vec![json!(h)]).await?;
            for field in [
                "uuid",
                "enabled",
                "multipathing",
                "other_config",
                "PBDs",
                "current_operations",
            ] {
                require(
                    latest_host[field] == previous_host[field],
                    "Host configuration changed before storage maintenance",
                )?;
            }
            match p.kinds[index] {
                Kind::Unplug(id) => {
                    let (_, current_pbd) = find(&graph, id.uuid())?;
                    storage_allowed(s, &current_pbd, "unplug").await?;
                    let reference = latest_attachment(s, &graph, id).await?;
                    Ok(("Async.PBD.unplug", vec![json!(reference)]))
                }
                Kind::Plug(host, sr) => {
                    let id = p.attachment(host, sr).ok_or(Error::Journal)?;
                    let (_, current_pbd) = find(&graph, id.uuid())?;
                    storage_allowed(s, &current_pbd, "plug").await?;
                    let reference = latest_attachment(s, &graph, id).await?;
                    Ok(("Async.PBD.plug", vec![json!(reference)]))
                }
                Kind::Mode => Ok(("host.set_multipathing", vec![json!(h), json!(enabled)])),
                Kind::RemoveHandler => Ok((
                    "host.remove_from_other_config",
                    vec![json!(h), json!("multipathhandle")],
                )),
                Kind::AddHandler => Ok((
                    "host.add_to_other_config",
                    vec![json!(h), json!("multipathhandle"), json!("dmp")],
                )),
                _ => Err(Error::Journal),
            }
        }
    }
}
pub(super) async fn discover_created(
    s: &Session,
    r: &mut ResourceOperationRecord,
    index: usize,
) -> Result<bool, Error> {
    let p = r.storage_graph_plan.as_ref().ok_or(Error::Journal)?;
    let Kind::Create(h) = p.kinds[index] else {
        return Ok(false);
    };
    let e = &p.evidence[index];
    if e.created.is_some() {
        return Ok(false);
    }
    require(e.completed, "Storage creation has no completion receipt")?;
    let StorageGraph::Repair { source, .. } = &p.operation else {
        return Err(Error::Journal);
    };
    super::disk::pool(s, r.pool).await?;
    let result = e.result.as_ref().ok_or(Error::Journal)?;
    let v = s.call("PBD.get_record", vec![json!(result)]).await?;
    let id = crate::PbdId::from(uuid(&v)?);
    let (host_ref, _) = xapi::read(s, ResourceTarget::Host(h)).await?;
    let (sr_ref, _) = xapi::read(s, ResourceTarget::Storage(source.sr)).await?;
    require(
        !p.pbds_before.contains(&id)
            && !p.evidence.iter().any(|v| v.created == Some(id))
            && PbdRecord::new(&v).host() == host_ref
            && PbdRecord::new(&v).sr() == sr_ref
            && PbdRecord::new(&v).currently_attached() == false,
        "Returned attachment is not the new reviewed host/SR connection",
    )?;
    r.storage_graph_plan
        .as_mut()
        .ok_or(Error::Journal)?
        .evidence[index]
        .created = Some(id);
    r.storage_graph = r.storage_graph_plan.as_ref().unwrap().results();
    Ok(true)
}
pub(super) async fn verify(
    s: &Session,
    r: &mut ResourceOperationRecord,
    index: usize,
) -> Result<(), Error> {
    let mut p = r.storage_graph_plan.as_ref().ok_or(Error::Journal)?.clone();
    require(
        p.evidence[index].completed,
        "Storage step has no completion receipt",
    )?;
    if let Kind::Create(h) = p.kinds[index] {
        let StorageGraph::Repair { source, .. } = &p.operation else {
            return Err(Error::Journal);
        };
        let result = p.evidence[index].result.as_ref().ok_or(Error::Journal)?;
        let v = s.call("PBD.get_record", vec![json!(result)]).await?;
        let id = crate::PbdId::from(uuid(&v)?);
        let (host_ref, _) = host(s, h, true).await?;
        let (sr_ref, _) = storage(s, source.sr).await?;
        require(
            !p.pbds_before.contains(&id)
                && p.evidence[index].created == Some(id)
                && PbdRecord::new(&v).host() == host_ref
                && PbdRecord::new(&v).sr() == sr_ref
                && PbdRecord::new(&v).currently_attached() == false,
            "Returned attachment is not the new reviewed host/SR connection",
        )?;
        let (_, source_v) = xapi::read(s, ResourceTarget::Pbd(source.pbd)).await?;
        require(
            fingerprint(&source_v)? == *p.configuration.get(&source.pbd).ok_or(Error::Journal)?,
            "Source attachment changed during creation",
        )?;
        let secrets = verify_clone(s, &p, &source_v, &v, None).await?;
        p.evidence[index].created = Some(id);
        p.evidence[index].secrets = secrets;
    }
    match &p.operation {
        StorageGraph::Repair { .. } => {
            repair_graph(s, r, &p, Some(index)).await?;
        }
        StorageGraph::Multipath { .. } => {
            multipath_graph(s, r, &p, Some(index)).await?;
        }
    }
    p.evidence[index].verified = true;
    r.storage_graph = p.results();
    r.storage_graph_plan = Some(p);
    Ok(())
}
pub(super) async fn task(
    s: &Session,
    r: &ResourceOperationRecord,
    index: usize,
) -> Result<(crate::TaskId, Value), Error> {
    let p = r.storage_graph_plan.as_ref().ok_or(Error::Journal)?;
    let e = &p.evidence[index];
    let reference = e.task.as_ref().ok_or(Error::Protocol(
        "Storage task receipt unavailable; no write will be replayed",
    ))?;
    let v = s.call("task.get_record", vec![json!(reference)]).await?;
    let id = crate::TaskId::from(uuid(&v)?);
    require(
        !p.tasks_before.contains(&id)
            && e.task_uuid.is_none_or(|old| old == id)
            && !p
                .evidence
                .iter()
                .enumerate()
                .any(|(n, e)| n != index && e.task_uuid == Some(id)),
        "Storage task identity changed",
    )?;
    Ok((id, v))
}
