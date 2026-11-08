use crate::{Error, Inventory, SnapshotId, VdiId, VmId, xapi::Session};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;

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
                    && disk.kind == "user"
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
                                            && vbd.kind == "Disk"
                                    })
                                })
                        })
            })
            .map(|disk| disk.id)
            .collect()
    }
}

pub(crate) fn reference(value: &Value) -> Result<String, Error> {
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
    if parent_record["is_a_snapshot"] != false
        || !parent_record["is_a_template"].is_boolean()
        || parent_record["is_control_domain"] != false
    {
        return Err(Error::Stale);
    }
    let (reference, record) = optional_record(session, "VM", &snapshot.to_string())
        .await?
        .ok_or(Error::Stale)?;
    if record["is_a_snapshot"] != true
        || record["is_control_domain"] != false
        || record["snapshot_of"].as_str() != Some(&parent_ref)
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
fn can(record: &Value, operation: &str) -> bool {
    record["allowed_operations"]
        .as_array()
        .is_some_and(|ops| ops.iter().any(|op| op == operation))
        && record["blocked_operations"].get(operation).is_none()
}
fn disk_is_snapshot(record: &Value) -> bool {
    record["is_a_snapshot"] == true
        && record["sharable"] == false
        && record["managed"] == true
        && record["missing"] == false
        && record["type"] == "user"
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
    let checkpoint = record["power_state"] == "Suspended";
    if (!checkpoint && record["power_state"] != "Halted")
        || !can(
            &record,
            if checkpoint {
                "hard_shutdown"
            } else {
                "destroy"
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
            let attachment = session.call("VBD.get_record", vec![json!(vbd)]).await?;
            if attachment["VM"] != snapshot_ref
                || attachment["VDI"] != disk_ref
                || attachment["type"] != "Disk"
                || !attachment["other_config"]
                    .get("owner")
                    .is_some_and(Value::is_string)
            {
                return Err(Error::SnapshotConflict);
            }
        }
    }
    let memory = if checkpoint {
        let reference = reference(&record["suspend_VDI"])?;
        let disk = session
            .call("VDI.get_record", vec![json!(&reference)])
            .await?;
        if disk["type"] != "suspend"
            || disk["sharable"] != false
            || !refs(&disk, "VBDs")?.is_empty()
        {
            return Err(Error::SnapshotConflict);
        }
        let id = disk["uuid"]
            .as_str()
            .and_then(|id| VdiId::parse(id).ok())
            .ok_or(Error::SnapshotConflict)?;
        let vms = session.call("VM.get_all_records", vec![]).await?;
        if vms
            .as_object()
            .ok_or(Error::SnapshotConflict)?
            .iter()
            .any(|(vm, value)| *vm != snapshot_ref && value["suspend_VDI"] == reference)
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
    if record["power_state"] != "Suspended" || !can(&record, "hard_shutdown") {
        return Err(Error::SnapshotConflict);
    }
    let memory_ref = self::reference(&record["suspend_VDI"])?;
    let saved = session
        .call("VDI.get_record", vec![json!(memory_ref)])
        .await?;
    if saved["uuid"].as_str() != memory.map(|id| id.to_string()).as_deref() {
        return Err(Error::SnapshotConflict);
    }
    session
        .call("VM.hard_shutdown", vec![json!(reference)])
        .await?;
    Ok(())
}

pub(crate) async fn destroy_snapshot(
    session: &Session,
    parent: VmId,
    snapshot: SnapshotId,
) -> Result<(), Error> {
    let (reference, record) = snapshot_record(session, parent, snapshot).await?;
    if record["power_state"] != "Halted" || !can(&record, "destroy") {
        return Err(Error::SnapshotConflict);
    }
    session.call("VM.destroy", vec![json!(reference)]).await?;
    Ok(())
}

pub(crate) async fn destroy_disk(session: &Session, disk: VdiId) -> Result<(), Error> {
    let Some((reference, record)) = optional_record(session, "VDI", &disk.to_string()).await?
    else {
        return Ok(());
    };
    if !disk_is_snapshot(&record) || !refs(&record, "VBDs")?.is_empty() || !can(&record, "destroy")
    {
        return Err(Error::SnapshotConflict);
    }
    let vms = session.call("VM.get_all_records", vec![]).await?;
    if vms
        .as_object()
        .ok_or(Error::SnapshotConflict)?
        .values()
        .any(|vm| vm["suspend_VDI"] == reference)
    {
        return Err(Error::SnapshotConflict);
    }
    session.call("VDI.destroy", vec![json!(reference)]).await?;
    Ok(())
}

pub(crate) async fn verify_clone(
    session: &Session,
    parent: VmId,
    snapshot: SnapshotId,
    reference: &str,
) -> Result<(VmId, Value), Error> {
    let record = session
        .call("VM.get_record", vec![json!(reference)])
        .await?;
    let id = record["uuid"]
        .as_str()
        .and_then(|id| VmId::parse(id).ok())
        .ok_or(Error::Protocol("invalid cloned VM identity"))?;
    if id == parent
        || id.to_string() == snapshot.to_string()
        || record["is_a_snapshot"] != false
        || record["is_control_domain"] != false
        || !record["is_a_template"].is_boolean()
        || record["power_state"] != "Halted"
        || record["snapshot_of"] != "OpaqueRef:NULL"
    {
        return Err(Error::Protocol("invalid cloned VM result"));
    }
    Ok((id, record))
}

pub(crate) async fn clone_baseline(session: &Session) -> Result<Vec<VmId>, Error> {
    let records = session.call("VM.get_all_records", vec![]).await?;
    let records = records
        .as_object()
        .filter(|records| records.len() <= 10_000)
        .ok_or(Error::Protocol(
            "clone identity baseline exceeds 10000 VM records or is invalid",
        ))?;
    let ids = records
        .values()
        .map(|record| {
            record["uuid"]
                .as_str()
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
