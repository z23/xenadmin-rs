use super::*;
use crate::{
    DiskOperation, ReviewedDisk, TaskId, VdiId,
    disk_administration::{Evidence, Kind, Plan},
    xapi::Session,
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use xenadmin_xapi::enumerations::{vbd_type, vdi_operations, vdi_type, vm_power_state};
use xenadmin_xapi::records::{PbdRecord, PoolRecord, SrRecord, VbdRecord, VdiRecord, VmRecord};
use xenadmin_xapi::{Call, Int, calls, refs};
use xenadmin_xapi::{Reads, kind};

fn conflict<T>() -> Result<T, Error> {
    Err(Error::ResourceConflict)
}
fn id(record: &Value) -> Result<uuid::Uuid, Error> {
    record["uuid"]
        .as_str()
        .and_then(|s| s.parse().ok())
        .ok_or(Error::ResourceConflict)
}
fn number<'a>(value: impl Reads<'a, kind::Whole>) -> Option<u64> {
    let value = value.raw();
    value.as_u64().or_else(|| value.as_str()?.parse().ok())
}
fn reference(value: &Value) -> Result<String, Error> {
    crate::snapshots::reference(value)
}
fn refs(record: &Value, key: &str) -> Result<BTreeSet<String>, Error> {
    let values = record[key]
        .as_array()
        .filter(|v| v.len() <= 100_000)
        .ok_or(Error::ResourceConflict)?;
    let set = values
        .iter()
        .map(reference)
        .collect::<Result<BTreeSet<_>, _>>()?;
    if set.len() != values.len() {
        return conflict();
    }
    Ok(set)
}
fn idle(record: &Value) -> Result<(), Error> {
    if record["current_operations"]
        .as_object()
        .is_some_and(|v| v.is_empty())
    {
        Ok(())
    } else {
        Err(Error::Busy)
    }
}
fn ordinary(record: &Value) -> Result<(), Error> {
    idle(record)?;
    if VdiRecord::new(record).managed() != true
        || VdiRecord::new(record).missing() != false
        || VdiRecord::new(record).read_only() != false
        || VdiRecord::new(record).sharable() != false
        || VdiRecord::new(record).is_a_snapshot() != false
        || VdiRecord::new(record).type_() != vdi_type::USER
        || VdiRecord::new(record).snapshot_of() != "OpaqueRef:NULL"
        || VdiRecord::new(record).cbt_enabled() != false
    {
        return conflict();
    }
    Ok(())
}
fn permits(record: &Value, operation: &str) -> Result<(), Error> {
    if VdiRecord::new(record)
        .allowed_operations()
        .members()
        .is_some_and(|v| v.iter().any(|v| v == operation))
    {
        Ok(())
    } else {
        Err(Error::Protocol(
            "The host does not currently advertise this disk operation",
        ))
    }
}
async fn records(s: &Session, class: &str) -> Result<BTreeMap<String, Value>, Error> {
    let rows = crate::security_discovery::records(s, class, 100_000).await?;
    let mut ids = BTreeSet::new();
    for (key, r) in &rows {
        reference(&json!(key))?;
        if !ids.insert(id(r)?) {
            return conflict();
        }
    }
    Ok(rows.into_iter().collect())
}
pub(super) async fn pool(s: &Session, expected: PoolId) -> Result<Value, Error> {
    let rows = records(s, "pool").await?;
    if rows.len() != 1 {
        return conflict();
    }
    let r = rows.into_values().next().ok_or(Error::ResourceConflict)?;
    if id(&r)? != expected.uuid() {
        return conflict();
    }
    Ok(r)
}
async fn storage(s: &Session, sr: SrId) -> Result<String, Error> {
    let (reference, r) = xapi::read(s, ResourceTarget::Storage(sr)).await?;
    idle(&r)?;
    if SrRecord::new(&r).content_type() == "iso" {
        return conflict();
    }
    let mut attached = false;
    for pbd in refs(&r, "PBDs")? {
        let p = s.send(calls::pbd::GetRecord(refs::pbd(&pbd))).await?;
        if PbdRecord::new(&p).sr() != reference {
            return conflict();
        }
        if PbdRecord::new(&p).currently_attached() == true {
            attached = true;
        }
    }
    if !attached {
        return Err(Error::StorageInUse);
    }
    Ok(reference)
}
async fn checked_disk(
    s: &Session,
    disk: VdiId,
    expected: &ReviewedDisk,
    size: Option<u64>,
    pool_record: &Value,
) -> Result<(String, Value), Error> {
    let (reference, r) = xapi::read(s, ResourceTarget::Disk(disk)).await?;
    ordinary(&r)?;
    if number(VdiRecord::new(&r).virtual_size()) != Some(size.unwrap_or(expected.size)) {
        return conflict();
    }
    let sr = storage(s, expected.sr).await?;
    if VdiRecord::new(&r).sr() != sr {
        return conflict();
    }
    let vbds = records(s, "VBD").await?;
    let vms = records(s, "VM").await?;
    let vdis = records(s, "VDI").await?;
    let mut consumers = BTreeMap::new();
    let mut consumer_refs = BTreeSet::new();
    for (vbd_ref, vbd) in &vbds {
        if VbdRecord::new(vbd).vdi() != reference {
            continue;
        }
        idle(vbd)?;
        if VbdRecord::new(vbd).currently_attached() != false
            || VbdRecord::new(vbd).empty() != false
            || VbdRecord::new(vbd).type_() != vbd_type::DISK
        {
            return Err(Error::StorageInUse);
        }
        let vm_ref = reference_value(VbdRecord::new(vbd).vm())?;
        let vm = vms.get(&vm_ref).ok_or(Error::ResourceConflict)?;
        idle(vm)?;
        if VmRecord::new(vm).is_a_snapshot() != false
            || VmRecord::new(vm).is_a_template() != false
            || VmRecord::new(vm).is_control_domain() != false
            || !matches!(
                VmRecord::new(vm).power_state().name(),
                Some(vm_power_state::HALTED | vm_power_state::RUNNING)
            )
            || !refs(vm, "VBDs")?.contains(vbd_ref)
        {
            return Err(Error::StorageInUse);
        }
        consumers.insert(crate::VbdId::from(id(vbd)?), crate::VmId::from(id(vm)?));
        consumer_refs.insert(vbd_ref.clone());
    }
    let expected_consumers = expected
        .consumers
        .iter()
        .map(|v| (v.attachment, v.vm))
        .collect::<BTreeMap<_, _>>();
    if consumers != expected_consumers || consumer_refs != refs(&r, "VBDs")? {
        return conflict();
    }
    let statefiles = PoolRecord::new(pool_record)
        .ha_statefiles()
        .members()
        .filter(|a| a.len() <= 1024)
        .ok_or(Error::Protocol("Pool HA disk dependencies unavailable"))?;
    if statefiles.iter().any(|v| v.as_str().is_none()) {
        return conflict();
    }
    if vms
        .values()
        .any(|v| VmRecord::new(v).suspend_vdi() == reference)
        || vdis
            .iter()
            .any(|(key, v)| key != &reference && VdiRecord::new(v).snapshot_of() == reference)
        || statefiles
            .iter()
            .any(|v| v == &reference || v == &disk.to_string())
        || PoolRecord::new(pool_record).redo_log_vdi() == reference
    {
        return Err(Error::StorageInUse);
    }
    // The final disk read binds the consumer scan to the same reviewed identity and state.
    let (latest_ref, latest) = xapi::read(s, ResourceTarget::Disk(disk)).await?;
    ordinary(&latest)?;
    if latest_ref != reference
        || VdiRecord::new(&latest).sr() != sr
        || number(VdiRecord::new(&latest).virtual_size()) != Some(size.unwrap_or(expected.size))
        || refs(&latest, "VBDs")? != consumer_refs
    {
        return conflict();
    }
    Ok((reference, latest))
}
fn reference_value<'a>(v: impl Reads<'a, kind::Reference>) -> Result<String, Error> {
    let v = v.raw();
    reference(v)
}
fn op(r: &ResourceOperationRecord) -> Result<&DiskOperation, Error> {
    match &r.action {
        ResourceAction::Disk(op) => Ok(op),
        _ => Err(Error::InvalidResourceCommand),
    }
}
pub(super) async fn prepare(
    s: &Session,
    r: &ResourceOperationRecord,
    j: &mut crate::operations::Journal,
) -> Result<Plan, Error> {
    let operation = op(r)?.clone();
    operation.validate(r.target)?;
    let ResourceTarget::Disk(disk) = r.target else {
        return Err(Error::InvalidResourceCommand);
    };
    let p = pool(s, r.pool).await?;
    let (_, vdi) = checked_disk(s, disk, operation.expected(), None, &p).await?;
    permits(
        &vdi,
        match operation {
            DiskOperation::Grow { .. } => vdi_operations::RESIZE,
            DiskOperation::Copy { .. } => vdi_operations::COPY,
            DiskOperation::Cleanup { .. } => vdi_operations::DESTROY,
        },
    )?;
    if let DiskOperation::Copy { destination, .. } = operation {
        storage(s, destination).await?;
    }
    if let DiskOperation::Cleanup { source, .. } = operation {
        j.disk_ownership(r.pool, disk, source, &vdi)?;
    }
    let tasks_before = records(s, "task")
        .await?
        .values()
        .map(|v| id(v).map(TaskId::from))
        .collect::<Result<_, _>>()?;
    let disks_before = if matches!(operation, DiskOperation::Copy { .. }) {
        records(s, "VDI")
            .await?
            .values()
            .map(|v| id(v).map(VdiId::from))
            .collect::<Result<_, _>>()?
    } else {
        BTreeSet::new()
    };
    let kinds = Plan::kinds(&operation);
    Ok(Plan {
        pool: r.pool,
        disk,
        operation,
        tasks_before,
        disks_before,
        evidence: vec![Evidence::default(); kinds.len()],
        kinds,
        created: None,
        actual_size: None,
    })
}
pub(super) async fn arguments(
    s: &Session,
    r: &ResourceOperationRecord,
    j: &mut crate::operations::Journal,
    index: usize,
) -> Result<(&'static str, Vec<Value>), Error> {
    let plan = r.disk_plan.as_ref().ok_or(Error::Journal)?;
    let p = pool(s, r.pool).await?;
    if plan.kinds[index] == Kind::Name {
        let (reference, _) = created(s, r).await?;
        let DiskOperation::Copy { name, .. } = &plan.operation else {
            return conflict();
        };
        return Ok(calls::vdi::SetNameLabel {
            self_: refs::vdi(&reference),
            value: name,
        }
        .request());
    }
    let (reference, vdi) = checked_disk(s, plan.disk, plan.operation.expected(), None, &p).await?;
    match &plan.operation {
        DiskOperation::Grow { size, .. } => {
            permits(&vdi, vdi_operations::RESIZE)?;
            Ok(calls::vdi::Resize {
                vdi: refs::vdi(&reference),
                size: Int::from(size),
            }
            .task())
        }
        DiskOperation::Copy { destination, .. } => {
            permits(&vdi, vdi_operations::COPY)?;
            let sr = storage(s, *destination).await?;
            Ok(calls::vdi::CopyWithIntoVdi {
                vdi: refs::vdi(&reference),
                sr: refs::sr(&sr),
                base_vdi: refs::vdi("OpaqueRef:NULL"),
                into_vdi: refs::vdi("OpaqueRef:NULL"),
            }
            .task())
        }
        DiskOperation::Cleanup { source, .. } => {
            permits(&vdi, vdi_operations::DESTROY)?;
            j.disk_ownership(r.pool, plan.disk, *source, &vdi)?;
            Ok(calls::vdi::Destroy(refs::vdi(&reference)).task())
        }
    }
}
async fn created(s: &Session, r: &ResourceOperationRecord) -> Result<(String, Value), Error> {
    let plan = r.disk_plan.as_ref().ok_or(Error::Journal)?;
    let DiskOperation::Copy {
        destination,
        expected,
        ..
    } = &plan.operation
    else {
        return conflict();
    };
    let result = plan.evidence[0].result.as_ref().ok_or(Error::Protocol(
        "Disk operation completion receipt unavailable; no write will be replayed",
    ))?;
    let vdi = s.send(calls::vdi::GetRecord(refs::vdi(result))).await?;
    let disk = VdiId::from(id(&vdi)?);
    if plan.disks_before.contains(&disk) || plan.created.is_some_and(|id| id != disk) {
        return conflict();
    }
    let expected = ReviewedDisk {
        sr: *destination,
        size: expected.size,
        consumers: Vec::new(),
    };
    let p = pool(s, r.pool).await?;
    let (reference, record) = checked_disk(s, disk, &expected, None, &p).await?;
    if &reference != result {
        return conflict();
    }
    Ok((reference, record))
}
pub(super) async fn verify(
    s: &Session,
    r: &mut ResourceOperationRecord,
    index: usize,
) -> Result<(), Error> {
    let plan = r.disk_plan.as_ref().ok_or(Error::Journal)?;
    if !plan.evidence[index].completed {
        return Err(Error::Protocol(
            "Disk operation completion receipt unavailable; no write will be replayed",
        ));
    }
    let p = pool(s, r.pool).await?;
    match plan.kinds[index] {
        Kind::Grow => {
            let (_, vdi) = xapi::read(s, ResourceTarget::Disk(plan.disk)).await?;
            let actual =
                number(VdiRecord::new(&vdi).virtual_size()).ok_or(Error::ResourceConflict)?;
            let DiskOperation::Grow { size, .. } = &plan.operation else {
                return conflict();
            };
            if actual < *size {
                return conflict();
            }
            checked_disk(s, plan.disk, plan.operation.expected(), Some(actual), &p).await?;
            r.disk_plan.as_mut().ok_or(Error::Journal)?.actual_size = Some(actual);
            r.disk_size = Some(actual);
        }
        Kind::Copy | Kind::Name => {
            let (_, vdi) = created(s, r).await?;
            let disk = VdiId::from(id(&vdi)?);
            if plan.kinds[index] == Kind::Name {
                let DiskOperation::Copy { name, .. } = &plan.operation else {
                    return conflict();
                };
                if VdiRecord::new(&vdi).name_label() != *name {
                    return conflict();
                }
            }
            r.disk_plan.as_mut().ok_or(Error::Journal)?.created = Some(disk);
            r.created = Some(ResourceTarget::Disk(disk));
        }
        Kind::Cleanup => {
            if crate::snapshots::optional_record(s, "VDI", &plan.disk.to_string())
                .await?
                .is_some()
            {
                return conflict();
            }
        }
    }
    r.disk_plan.as_mut().ok_or(Error::Journal)?.evidence[index].verified = true;
    Ok(())
}
pub(super) async fn task(
    s: &Session,
    r: &ResourceOperationRecord,
    index: usize,
) -> Result<(TaskId, Value), Error> {
    let plan = r.disk_plan.as_ref().ok_or(Error::Journal)?;
    let evidence = &plan.evidence[index];
    let reference = evidence.task.as_ref().ok_or(Error::Protocol(
        "Disk operation completion receipt unavailable; no write will be replayed",
    ))?;
    let task = s
        .send(calls::task::GetRecord(refs::task(reference)))
        .await?;
    let uuid = TaskId::from(id(&task)?);
    if plan.tasks_before.contains(&uuid) || evidence.task_uuid.is_some_and(|id| id != uuid) {
        return conflict();
    }
    Ok((uuid, task))
}
