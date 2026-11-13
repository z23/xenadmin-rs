use crate::{Error, Inventory, SnapshotId, VdiId, VmId, xapi::Session};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use xenadmin_xapi::enumerations::{
    vbd_type, vdi_operations, vdi_type, vm_operations, vm_power_state,
};
use xenadmin_xapi::records::{VbdRecord, VdiRecord, VmRecord};
use xenadmin_xapi::{Reads, kind};
use xenadmin_xapi::{calls, refs};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DeletionPlan {
    pub snapshot: SnapshotId,
    pub checkpoint: bool,
    pub disks: Vec<VdiId>,
    pub memory: Option<VdiId>,
}

impl Inventory {
    /// Conservative candidates for review. Dispatch checks the selected UUIDs again.
    pub fn snapshot_deletion_disks(&self, snapshot: SnapshotId) -> Vec<VdiId> {
        if ["vdi", "vbd"].iter().any(|class| {
            self.details.classes.get(*class) != Some(&crate::ClassAvailability::Available)
        }) {
            return Vec::new();
        }
        self.details
            .vdis
            .values()
            .filter(|disk| {
                disk.is_snapshot == Some(true)
                    && disk.sharable == Some(false)
                    && disk.managed == Some(true)
                    && disk.missing == Some(false)
                    && disk.kind == vdi_type::USER
                    && self
                        .details
                        .vdi_attachments
                        .get(&disk.id)
                        .is_some_and(|attachments| {
                            !attachments.is_empty()
                                && attachments.iter().all(|id| {
                                    self.details.vbds.get(id).is_some_and(|vbd| {
                                        vbd.snapshot == Some(snapshot)
                                            && vbd.owned
                                            && vbd.kind == vbd_type::DISK
                                    })
                                })
                        })
            })
            .map(|disk| disk.id)
            .collect()
    }
}

pub(crate) fn reference<'a>(value: impl Reads<'a, kind::Reference>) -> Result<String, Error> {
    let value = value.raw();
    value
        .as_str()
        .filter(|s| {
            s.starts_with("OpaqueRef:")
                && *s != "OpaqueRef:NULL"
                && s.len() > 10
                && s.len() <= 1024
                && s.bytes().all(|b| b.is_ascii_graphic())
        })
        .map(str::to_owned)
        .ok_or(Error::Protocol("invalid snapshot object reference"))
}

pub(crate) async fn optional_record(
    session: &Session,
    class: &str,
    id: &str,
) -> Result<Option<(String, Value)>, Error> {
    let result = async {
        let object = reference(
            &session
                .call(&format!("{class}.get_by_uuid"), vec![json!(id)])
                .await?,
        )?;
        let record = session
            .call(&format!("{class}.get_record"), vec![json!(&object)])
            .await?;
        if record["uuid"].as_str() != Some(id) {
            return Err(Error::SnapshotConflict);
        }
        Ok((object, record))
    }
    .await;
    match result {
        Err(Error::Api(code)) if matches!(code.as_str(), "UUID_INVALID" | "HANDLE_INVALID") => {
            Ok(None)
        }
        other => other.map(Some),
    }
}

pub(crate) async fn snapshot_record(
    session: &Session,
    parent: VmId,
    snapshot: SnapshotId,
) -> Result<(String, Value), Error> {
    let (parent_ref, parent_record) = optional_record(session, "VM", &parent.to_string())
        .await?
        .ok_or(Error::Stale)?;
    if VmRecord::new(&parent_record).is_a_snapshot() != false
        || !VmRecord::new(&parent_record)
            .is_a_template()
            .get()
            .is_some()
        || VmRecord::new(&parent_record).is_control_domain() != false
    {
        return Err(Error::Stale);
    }
    let (reference, record) = optional_record(session, "VM", &snapshot.to_string())
        .await?
        .ok_or(Error::Stale)?;
    if VmRecord::new(&record).is_a_snapshot() != true
        || VmRecord::new(&record).is_control_domain() != false
        || VmRecord::new(&record).snapshot_of().reference() != Some(&parent_ref)
    {
        return Err(Error::SnapshotConflict);
    }
    Ok((reference, record))
}

fn refs(record: &Value, key: &str) -> Result<Vec<String>, Error> {
    let values = record[key]
        .as_array()
        .filter(|v| v.len() <= 256)
        .ok_or(Error::SnapshotConflict)?;
    values.iter().map(reference).collect()
}
/// Whether a VM (a snapshot is one) offers `operation` and no administrator blocked it.
fn can(record: &Value, operation: &str) -> bool {
    VmRecord::new(record)
        .allowed_operations()
        .members()
        .is_some_and(|ops| ops.iter().any(|op| op == operation))
        && VmRecord::new(record)
            .blocked_operations()
            .get(operation)
            .is_none()
}
fn disk_is_snapshot(record: &Value) -> bool {
    VdiRecord::new(record).is_a_snapshot() == true
        && VdiRecord::new(record).sharable() == false
        && VdiRecord::new(record).managed() == true
        && VdiRecord::new(record).missing() == false
        && VdiRecord::new(record).type_() == vdi_type::USER
}

pub(crate) async fn deletion_plan(
    session: &Session,
    parent: VmId,
    snapshot: SnapshotId,
    disks: &[VdiId],
) -> Result<DeletionPlan, Error> {
    if disks.len() > 256 || disks.iter().collect::<BTreeSet<_>>().len() != disks.len() {
        return Err(Error::SnapshotConflict);
    }
    let (snapshot_ref, record) = snapshot_record(session, parent, snapshot).await?;
    let checkpoint = VmRecord::new(&record).power_state() == vm_power_state::SUSPENDED;
    if (!checkpoint && VmRecord::new(&record).power_state() != vm_power_state::HALTED)
        || !can(
            &record,
            if checkpoint {
                vm_operations::HARD_SHUTDOWN
            } else {
                vm_operations::DESTROY
            },
        )
    {
        return Err(Error::UnavailableAction);
    }
    let attachments = refs(&record, "VBDs")?;
    for id in disks {
        let (disk_ref, disk) = optional_record(session, "VDI", &id.to_string())
            .await?
            .ok_or(Error::SnapshotConflict)?;
        if !disk_is_snapshot(&disk) {
            return Err(Error::SnapshotConflict);
        }
        let consumers = refs(&disk, "VBDs")?;
        if consumers.is_empty() || consumers.iter().any(|vbd| !attachments.contains(vbd)) {
            return Err(Error::SnapshotConflict);
        }
        for vbd in consumers {
            let attachment = session.send(calls::vbd::GetRecord(refs::vbd(&vbd))).await?;
            if VbdRecord::new(&attachment).vm() != snapshot_ref
                || VbdRecord::new(&attachment).vdi() != disk_ref
                || VbdRecord::new(&attachment).type_() != vbd_type::DISK
                || !VbdRecord::new(&attachment)
                    .other_config()
                    .get("owner")
                    .is_some_and(Value::is_string)
            {
                return Err(Error::SnapshotConflict);
            }
        }
    }
    let memory = if checkpoint {
        let reference = reference(VmRecord::new(&record).suspend_vdi())?;
        let disk = session
            .send(calls::vdi::GetRecord(refs::vdi(&reference)))
            .await?;
        if VdiRecord::new(&disk).type_() != vdi_type::SUSPEND
            || VdiRecord::new(&disk).sharable() != false
            || !refs(&disk, "VBDs")?.is_empty()
        {
            return Err(Error::SnapshotConflict);
        }
        let id = VdiRecord::new(&disk)
            .uuid()
            .text()
            .and_then(|id| VdiId::parse(id).ok())
            .ok_or(Error::SnapshotConflict)?;
        let vms = session.send(calls::vm::GetAllRecords).await?;
        if vms
            .as_object()
            .ok_or(Error::SnapshotConflict)?
            .iter()
            .any(|(vm, value)| {
                *vm != snapshot_ref && VmRecord::new(value).suspend_vdi() == reference
            })
        {
            return Err(Error::SnapshotConflict);
        }
        Some(id)
    } else {
        None
    };
    Ok(DeletionPlan {
        snapshot,
        checkpoint,
        disks: disks.to_vec(),
        memory,
    })
}

pub(crate) async fn stop_checkpoint(
    session: &Session,
    parent: VmId,
    snapshot: SnapshotId,
    memory: Option<VdiId>,
) -> Result<(), Error> {
    let (reference, record) = snapshot_record(session, parent, snapshot).await?;
    if VmRecord::new(&record).power_state() != vm_power_state::SUSPENDED
        || !can(&record, vm_operations::HARD_SHUTDOWN)
    {
        return Err(Error::SnapshotConflict);
    }
    let memory_ref = self::reference(VmRecord::new(&record).suspend_vdi())?;
    let saved = session
        .send(calls::vdi::GetRecord(refs::vdi(&memory_ref)))
        .await?;
    if VdiRecord::new(&saved).uuid().text() != memory.map(|id| id.to_string()).as_deref() {
        return Err(Error::SnapshotConflict);
    }
    session
        .send(calls::vm::HardShutdown(refs::vm(&reference)))
        .await?;
    Ok(())
}

pub(crate) async fn destroy_snapshot(
    session: &Session,
    parent: VmId,
    snapshot: SnapshotId,
) -> Result<(), Error> {
    let (reference, record) = snapshot_record(session, parent, snapshot).await?;
    if VmRecord::new(&record).power_state() != vm_power_state::HALTED
        || !can(&record, vm_operations::DESTROY)
    {
        return Err(Error::SnapshotConflict);
    }
    session
        .send(calls::vm::Destroy(refs::vm(&reference)))
        .await?;
    Ok(())
}

pub(crate) async fn destroy_disk(session: &Session, disk: VdiId) -> Result<(), Error> {
    let Some((reference, record)) = optional_record(session, "VDI", &disk.to_string()).await?
    else {
        return Ok(());
    };
    if !disk_is_snapshot(&record)
        || !refs(&record, "VBDs")?.is_empty()
        || !crate::xapi::offers(
            VdiRecord::new(&record).allowed_operations(),
            vdi_operations::DESTROY,
        )
    {
        return Err(Error::SnapshotConflict);
    }
    let vms = session.send(calls::vm::GetAllRecords).await?;
    if vms
        .as_object()
        .ok_or(Error::SnapshotConflict)?
        .values()
        .any(|vm| VmRecord::new(vm).suspend_vdi() == reference)
    {
        return Err(Error::SnapshotConflict);
    }
    session
        .send(calls::vdi::Destroy(refs::vdi(&reference)))
        .await?;
    Ok(())
}

pub(crate) async fn verify_clone(
    session: &Session,
    parent: VmId,
    snapshot: SnapshotId,
    reference: &str,
) -> Result<(VmId, Value), Error> {
    let record = session
        .send(calls::vm::GetRecord(refs::vm(reference)))
        .await?;
    let id = VmRecord::new(&record)
        .uuid()
        .text()
        .and_then(|id| VmId::parse(id).ok())
        .ok_or(Error::Protocol("invalid cloned VM identity"))?;
    if id == parent
        || id.to_string() == snapshot.to_string()
        || VmRecord::new(&record).is_a_snapshot() != false
        || VmRecord::new(&record).is_control_domain() != false
        || !VmRecord::new(&record).is_a_template().get().is_some()
        || VmRecord::new(&record).power_state() != vm_power_state::HALTED
        || VmRecord::new(&record).snapshot_of() != "OpaqueRef:NULL"
    {
        return Err(Error::Protocol("invalid cloned VM result"));
    }
    Ok((id, record))
}

pub(crate) async fn clone_baseline(session: &Session) -> Result<Vec<VmId>, Error> {
    let records = session.send(calls::vm::GetAllRecords).await?;
    let records = records
        .as_object()
        .filter(|records| records.len() <= 10_000)
        .ok_or(Error::Protocol(
            "clone identity baseline exceeds 10000 VM records or is invalid",
        ))?;
    let ids = records
        .values()
        .map(|record| {
            VmRecord::new(record)
                .uuid()
                .text()
                .and_then(|id| VmId::parse(id).ok())
                .ok_or(Error::Protocol(
                    "invalid VM UUID in clone identity baseline",
                ))
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    if ids.len() != records.len() {
        return Err(Error::Protocol(
            "duplicate VM UUID in clone identity baseline",
        ));
    }
    Ok(ids.into_iter().collect())
}
