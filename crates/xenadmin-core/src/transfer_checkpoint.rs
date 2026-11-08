use super::*;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

impl ExportSource {
    pub fn checkpoint(
        inventory: &Inventory,
        snapshot: SnapshotId,
        parent: VmId,
        preserve_memory: bool,
    ) -> Option<Self> {
        let id = inventory.details.snapshots.get(&snapshot)?.memory_vdi?;
        let disk = inventory.details.vdis.get(&id)?;
        let memory = CheckpointMemory {
            vdi: id,
            sr: disk.sr?,
            virtual_size: disk.virtual_size?,
        };
        availability(inventory, snapshot, parent, &memory)
            .is_available()
            .then_some(Self::Checkpoint {
                snapshot,
                parent,
                memory,
                preserve_memory,
            })
    }
}

pub(crate) fn availability(
    i: &Inventory,
    snapshot: SnapshotId,
    parent: VmId,
    memory: &CheckpointMemory,
) -> ActionAvailability {
    if i.snapshots
        .get(&snapshot)
        .is_none_or(|s| s.vm != Some(parent) || s.power_state != crate::PowerState::Suspended)
        || i.details
            .snapshots
            .get(&snapshot)
            .is_none_or(|v| !v.memory_vdi_known || v.memory_vdi != Some(memory.vdi))
        || i.details.vdis.get(&memory.vdi).is_none_or(|d| {
            d.sr != Some(memory.sr)
                || d.virtual_size != Some(memory.virtual_size)
                || d.kind != "suspend"
                || d.managed != Some(true)
                || d.missing != Some(false)
        })
        || memory.virtual_size == 0
        || memory.virtual_size > MAX_TRANSFER_BYTES
    {
        return ActionAvailability::Stale;
    }
    if i.details
        .snapshots
        .get(&snapshot)
        .and_then(|s| s.can_export)
        != Some(true)
    {
        return ActionAvailability::Unavailable;
    }
    ActionAvailability::Available
}

pub(crate) fn size(record: &Value) -> Result<u64, Error> {
    record["virtual_size"]
        .as_u64()
        .or_else(|| record["virtual_size"].as_str()?.parse().ok())
        .filter(|v| *v > 0 && *v <= MAX_TRANSFER_BYTES)
        .ok_or(Error::TransferArchive)
}

/// Every suspend payload must be owned by exactly one suspended guest, never a VBD or ISO.
pub(crate) fn memory_graph(objects: &[Value]) -> Result<BTreeMap<String, ArchiveMemory>, Error> {
    let mut records = BTreeMap::new();
    for object in objects {
        let id = object["id"].as_str().ok_or(Error::TransferArchive)?;
        if records.insert(id, object).is_some() {
            return Err(Error::TransferArchive);
        }
    }
    let mut used = BTreeSet::new();
    let mut identities = BTreeSet::new();
    let mut guests = BTreeMap::new();
    for object in objects.iter().filter(|o| o["class"] == "VM") {
        let record = &object["snapshot"];
        let suspended = match record["power_state"].as_str() {
            Some("Halted") => false,
            Some("Suspended") => true,
            _ => return Err(Error::TransferArchive),
        };
        let memory = record["suspend_VDI"].as_str();
        if !suspended {
            if memory.is_some_and(|r| !matches!(r, "OpaqueRef:NULL" | "Ref:NULL")) {
                return Err(Error::TransferArchive);
            }
            continue;
        }
        let reference = memory.ok_or(Error::TransferArchive)?;
        let disk = records.get(reference).ok_or(Error::TransferArchive)?;
        let d = &disk["snapshot"];
        let uuid: uuid::Uuid = d["uuid"]
            .as_str()
            .ok_or(Error::TransferArchive)?
            .parse()
            .map_err(|_| Error::TransferArchive)?;
        let sr = d["SR"]
            .as_str()
            .and_then(|r| records.get(r))
            .ok_or(Error::TransferArchive)?;
        if disk["class"] != "VDI"
            || d["type"] != "suspend"
            || uuid.is_nil()
            || sr["class"] != "SR"
            || sr["snapshot"]["content_type"] != "user"
            || !used.insert(reference)
            || !identities.insert(uuid)
            || objects
                .iter()
                .any(|o| o["class"] == "VBD" && o["snapshot"]["VDI"] == reference)
        {
            return Err(Error::TransferArchive);
        }
        guests.insert(
            object["id"]
                .as_str()
                .ok_or(Error::TransferArchive)?
                .to_owned(),
            ArchiveMemory {
                uuid,
                virtual_size: size(d)?,
            },
        );
    }
    if objects.iter().any(|o| {
        o["class"] == "VDI"
            && o["snapshot"]["type"] == "suspend"
            && o["id"].as_str().is_none_or(|r| !used.contains(r))
    }) {
        return Err(Error::TransferArchive);
    }
    Ok(guests)
}

pub(crate) async fn fresh(
    s: &crate::xapi::Session,
    vm: &Value,
    expected: &CheckpointMemory,
) -> Result<(), Error> {
    let (reference, disk) = crate::snapshots::optional_record(s, "VDI", &expected.vdi.to_string())
        .await?
        .ok_or(Error::TransferConflict)?;
    let (sr_ref, sr) = crate::snapshots::optional_record(s, "SR", &expected.sr.to_string())
        .await?
        .ok_or(Error::TransferConflict)?;
    if vm["suspend_VDI"] != reference
        || vm["power_state"] != "Suspended"
        || disk["type"] != "suspend"
        || disk["managed"] != true
        || disk["missing"] != false
        || size(&disk)? != expected.virtual_size
        || disk["SR"] != sr_ref
        || sr["content_type"] != "user"
        || disk["current_operations"]
            .as_object()
            .is_none_or(|v| !v.is_empty())
        || disk["VBDs"].as_array().is_none_or(|v| !v.is_empty())
    {
        return Err(Error::TransferConflict);
    }
    Ok(())
}

pub(crate) fn validate_record(record: &crate::ResourceOperationRecord) -> Result<(), Error> {
    use crate::{OperationState, ResourceAction};
    let ResourceAction::Transfer(action) = &record.action else {
        return Ok(());
    };
    let checkpoint = matches!(
        action,
        TransferAction::ImportCheckpointXva { .. }
            | TransferAction::ExportXva {
                source: ExportSource::Checkpoint { .. },
                ..
            }
    );
    if !checkpoint {
        return Ok(());
    }
    let plan = record.transfer_plan.as_ref().ok_or(Error::Journal)?;
    if plan.file.disk.is_some()
        || plan.file.ovf.is_some()
        || plan.file.host_backup.is_some()
        || plan.disk_creation.is_some()
        || plan.ovf_import.is_some()
        || plan.ovf_export.is_some()
        || record.transfer_created.suspend_images.len() > 64
        || record
            .transfer_created
            .suspend_images
            .iter()
            .any(|v| v.uuid().is_nil())
        || record
            .transfer_created
            .suspend_images
            .iter()
            .collect::<BTreeSet<_>>()
            .len()
            != record.transfer_created.suspend_images.len()
    {
        return Err(Error::Journal);
    }
    match action {
        TransferAction::ImportCheckpointXva { sr } => {
            let summary = plan.file.manifest.as_ref().ok_or(Error::Journal)?;
            if sr.uuid().is_nil()
                || plan.file.identity.is_none()
                || summary.memory_images() == 0
                || summary.guests.is_empty()
                || summary.guests.len() > 64
                || summary.data_disks > 256
                || summary
                    .guests
                    .iter()
                    .filter_map(|g| g.memory.as_ref())
                    .any(|m| {
                        m.uuid.is_nil()
                            || m.virtual_size == 0
                            || m.virtual_size > MAX_TRANSFER_BYTES
                    })
                || record.transfer_created.suspend_images.len() > summary.memory_images()
                || plan.dispatched && plan.baseline.is_none()
            {
                return Err(Error::Journal);
            }
            if record.state == OperationState::Succeeded
                && (plan.result.is_none()
                    || record.transfer_created.suspend_images.len() != summary.memory_images()
                    || record.transfer_created.vms.len() != summary.guests.len())
            {
                return Err(Error::Journal);
            }
        }
        TransferAction::ExportXva {
            source:
                ExportSource::Checkpoint {
                    snapshot,
                    parent,
                    memory,
                    ..
                },
            ..
        } => {
            if snapshot.uuid().is_nil()
                || parent.uuid().is_nil()
                || memory.vdi.uuid().is_nil()
                || memory.sr.uuid().is_nil()
                || memory.virtual_size == 0
                || memory.virtual_size > MAX_TRANSFER_BYTES
                || plan.file.identity.is_some()
                || plan.file.manifest.is_some()
            {
                return Err(Error::Journal);
            }
        }
        _ => return Err(Error::Journal),
    }
    if record.state == OperationState::Succeeded
        && (!plan.task_success
            || plan.stream_receipt.is_none()
            || plan.stream_invalid
            || plan.task_uuid.is_none()
            || !plan.dispatched)
    {
        return Err(Error::Journal);
    }
    Ok(())
}
