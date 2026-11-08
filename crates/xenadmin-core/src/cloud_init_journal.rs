use super::*;
use crate::{OperationState, ResourceAction, ResourceOperationRecord, ResourceTarget};
use std::collections::BTreeSet;

fn baseline(ids: &[Uuid]) -> bool {
    ids.len() <= 10_000
        && ids.iter().all(|id| !id.is_nil())
        && ids.iter().collect::<BTreeSet<_>>().len() == ids.len()
}
fn reference(value: &Option<String>) -> bool {
    value.as_ref().is_none_or(|v| {
        v.starts_with("OpaqueRef:")
            && v.len() > 10
            && v.len() <= 512
            && v != "OpaqueRef:NULL"
            && v.bytes().all(|b| b.is_ascii_graphic())
    })
}

/// Validate durable structure without opening the private artifact: a completed upload may
/// already have removed it. Host identity and artifact integrity are checked by the worker.
pub(crate) fn validate_record(record: &ResourceOperationRecord) -> Result<(), Error> {
    let ResourceAction::CloudInit(configuration) = &record.action else {
        return if record.cloud_init_plan.is_none() {
            Ok(())
        } else {
            Err(Error::Journal)
        };
    };
    let Some(plan) = &record.cloud_init_plan else {
        return if record.state == OperationState::Rejected
            && record.transfer_plan.is_none()
            && record.task.is_none()
            && record.created.is_none()
            && record.transfer_created == Default::default()
            && record.steps.is_empty()
        {
            Ok(())
        } else {
            Err(Error::Journal)
        };
    };
    if !configuration.valid()
        || plan.configuration != *configuration
        || plan.operation != record.id
        || plan.pool != record.pool
        || ResourceTarget::Vm(plan.vm) != record.target
        || plan.sha256 == [0; 32]
        || (matches!(
            record.state,
            OperationState::OutcomeUnknown | OperationState::AcknowledgedUnknown
        ) && !plan.interrupted)
        || record.created.is_some()
        || !record.transfer_created.vms.is_empty()
        || !record.transfer_created.networks.is_empty()
        || !record.transfer_created.vifs.is_empty()
        || record.transfer_created.disks.len() > 1
        || record.transfer_created.vbds.len() > 1
        || !reference(&record.task)
        || record.result_reference.is_some()
    {
        return Err(Error::Journal);
    }
    let transfer = record.transfer_plan.as_ref().ok_or(Error::Journal)?;
    let file = &transfer.file;
    let image = file.disk.as_ref().ok_or(Error::Journal)?;
    let source = file.identity.as_ref().ok_or(Error::Journal)?;
    if !file.path.is_absolute()
        || file.path.file_name().is_none_or(|n| n != "seed.img")
        || file
            .path
            .parent()
            .and_then(Path::file_name)
            .is_none_or(|n| {
                n != std::ffi::OsStr::new(&format!(
                    "xenadmin-nocloud-{}",
                    configuration.configuration_id
                ))
            })
        || file.path.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
        || source.size != MEDIA_BYTES as u64
        || image.format != DiskImageFormat::Raw
        || image.virtual_size != MEDIA_BYTES as u64
        || image.sparse
        || file.manifest.is_some()
        || file.range.is_some()
        || file.ovf.is_some()
        || transfer.ovf_import.is_some()
        || transfer.ovf_export.is_some()
        || transfer.temporary.is_some()
        || transfer.temporary_identity.is_some()
        || transfer
            .baseline
            .as_ref()
            .is_some_and(|b| !baseline(&b.vms) || !baseline(&b.disks) || !baseline(&b.networks))
        || [
            transfer.task_success,
            transfer.task_failed,
            transfer.task_cancelled,
        ]
        .into_iter()
        .filter(|v| *v)
        .count()
            > 1
        || (transfer.task_uuid.is_some() && record.task.is_none())
        || transfer.task_uuid.is_some_and(|id| id.is_nil())
        || (transfer.dispatched && (transfer.task_uuid.is_none() || record.task.is_none()))
        || ((transfer.task_success || transfer.task_failed || transfer.task_cancelled)
            && !transfer.dispatched)
    {
        return Err(Error::Journal);
    }
    if let Some(receipt) = &transfer.stream_receipt
        && (!transfer.dispatched
            || receipt.bytes != MEDIA_BYTES as u64
            || receipt.identity != *source)
    {
        return Err(Error::Journal);
    }
    if plan.upload_verified && transfer.stream_receipt.is_none() {
        return Err(Error::Journal);
    }
    let disk = transfer.disk_creation.as_ref();
    if transfer.result.as_ref().is_some_and(|result| {
        !transfer.task_success
            || result.len() != 1
            || disk.and_then(|d| d.reference.as_ref()) != result.first()
    }) || (transfer.task_success && transfer.result.is_none())
    {
        return Err(Error::Journal);
    }
    if let Some(created) = disk {
        if !created.dispatched
            || !baseline(&created.baseline)
            || !reference(&created.reference)
            || created.uuid.is_some_and(|id| {
                created.reference.is_none() || created.baseline.contains(&id.uuid())
            })
            || record.transfer_created.disks != created.uuid.into_iter().collect::<Vec<_>>()
        {
            return Err(Error::Journal);
        }
    } else if !record.transfer_created.disks.is_empty()
        || transfer.dispatched
        || record.task.is_some()
    {
        return Err(Error::Journal);
    }
    if let Some(attachment) = &plan.attachment {
        if !plan.upload_verified
            || !transfer.task_success
            || transfer.stream_invalid
            || disk.is_none_or(|d| d.uuid.is_none())
            || !baseline(&attachment.baseline)
            || !reference(&attachment.reference)
            || attachment.uuid.is_some_and(|id| {
                attachment.reference.is_none() || attachment.baseline.contains(&id.uuid())
            })
            || attachment.verified != attachment.uuid.is_some()
            || record.transfer_created.vbds != attachment.uuid.into_iter().collect::<Vec<_>>()
        {
            return Err(Error::Journal);
        }
    } else if !record.transfer_created.vbds.is_empty() {
        return Err(Error::Journal);
    }
    if record.state == OperationState::Succeeded
        && plan.attachment.as_ref().is_none_or(|a| !a.verified)
    {
        return Err(Error::Journal);
    }
    Ok(())
}
