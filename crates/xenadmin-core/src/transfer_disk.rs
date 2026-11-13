use super::*;
use crate::{ClassAvailability, DiskImageFormat, VdiId, xapi::Session};
use serde_json::{Value, json};
use uuid::Uuid;
use xenadmin_xapi::enumerations::{storage_operations, vdi_type, vm_power_state};
use xenadmin_xapi::records::{SrRecord, VbdRecord, VdiRecord, VmRecord};
use xenadmin_xapi::{Reads, kind};
pub(crate) const MARKER: &str = "xenadmin-rs-disk-transfer";
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Creation {
    pub baseline: Vec<Uuid>,
    pub dispatched: bool,
    pub reference: Option<String>,
    pub uuid: Option<VdiId>,
}
pub(crate) fn valid_creation_update(a: Option<&Creation>, b: Option<&Creation>) -> bool {
    match (a, b) {
        (None, _) => true,
        (Some(a), Some(b)) => {
            a.baseline == b.baseline
                && (!a.dispatched || b.dispatched)
                && (a.reference.is_none() || a.reference == b.reference)
                && (a.uuid.is_none() || a.uuid == b.uuid)
        }
        _ => false,
    }
}
pub(crate) fn availability(
    i: &Inventory,
    vdi: VdiId,
    sr: SrId,
    expected_size: u64,
    format: DiskImageFormat,
) -> ActionAvailability {
    let no = |s: &str| ActionAvailability::Prerequisite { reason: s.into() };
    if ["vdi", "vbd"]
        .iter()
        .any(|c| i.details.classes.get(*c) != Some(&ClassAvailability::Available))
    {
        return ActionAvailability::Unknown;
    }
    let Some(d) = i.details.vdis.get(&vdi) else {
        return ActionAvailability::Stale;
    };
    if d.virtual_size != Some(expected_size)
        || d.sr != Some(sr)
        || d.kind != vdi_type::USER
        || d.managed != Some(true)
        || d.missing != Some(false)
        || d.sharable != Some(false)
        || d.cbt_enabled != Some(false)
        || d.virtual_size.is_none_or(|n| n == 0 || n > format.limit())
    {
        return no("Choose an ordinary exclusive data disk within the image format limit");
    }
    for vbd in i.details.vbds.values().filter(|v| v.vdi == Some(vdi)) {
        if vbd.currently_attached != Some(false) {
            return no("Detach live consumers or export a disk snapshot");
        }
        if let Some(vm) = vbd.vm {
            if i.vms
                .get(&vm)
                .is_none_or(|v| v.power_state != crate::PowerState::Halted || v.is_control_domain)
            {
                return no("Every disk consumer must be halted");
            }
        } else if let Some(snapshot) = vbd.snapshot {
            if i.snapshots
                .get(&snapshot)
                .is_none_or(|s| s.power_state != crate::PowerState::Halted)
            {
                return no("A memory checkpoint is not an ordinary disk export source");
            }
        } else {
            return ActionAvailability::Unknown;
        }
    }
    ActionAvailability::Available
}
fn number<'a>(v: impl Reads<'a, kind::Whole>) -> Option<u64> {
    let v = v.raw();
    v.as_u64().or_else(|| v.as_str()?.parse().ok())
}
fn idle(v: &Value) -> Result<(), Error> {
    if v["current_operations"]
        .as_object()
        .is_none_or(|m| !m.is_empty())
    {
        return Err(Error::Busy);
    }
    Ok(())
}
async fn fresh(s: &Session, class: &str, id: Uuid) -> Result<(String, Value), Error> {
    crate::snapshots::optional_record(s, class, &id.to_string())
        .await?
        .ok_or(Error::TransferConflict)
}
pub(crate) async fn disk_record(
    s: &Session,
    vdi: VdiId,
    sr: SrId,
    format: DiskImageFormat,
    unattached: bool,
) -> Result<(String, Value), Error> {
    let (reference, d) = fresh(s, "VDI", vdi.uuid()).await?;
    idle(&d)?;
    if VdiRecord::new(&d).type_() != vdi_type::USER
        || VdiRecord::new(&d).managed() != true
        || VdiRecord::new(&d).missing() != false
        || VdiRecord::new(&d).sharable() != false
        || VdiRecord::new(&d).cbt_enabled() != false
        || number(VdiRecord::new(&d).virtual_size())
            .is_none_or(|n| n == 0 || n > format.limit() || !n.is_multiple_of(512))
    {
        return Err(Error::TransferConflict);
    }
    let (sr_ref, _) = fresh(s, "SR", sr.uuid()).await?;
    if VdiRecord::new(&d).sr() != sr_ref {
        return Err(Error::TransferConflict);
    }
    let attachments = VdiRecord::new(&d)
        .vbds()
        .members()
        .filter(|a| a.len() <= 256)
        .ok_or(Error::TransferConflict)?;
    if unattached && !attachments.is_empty() {
        return Err(Error::TransferConflict);
    }
    for a in attachments {
        let a = s
            .call(
                "VBD.get_record",
                vec![json!(crate::snapshots::reference(a)?)],
            )
            .await?;
        idle(&a)?;
        if VbdRecord::new(&a).vdi() != reference || VbdRecord::new(&a).currently_attached() != false
        {
            return Err(Error::TransferConflict);
        }
        let vm = s
            .call(
                "VM.get_record",
                vec![json!(crate::snapshots::reference(VbdRecord::new(&a).vm())?)],
            )
            .await?;
        idle(&vm)?;
        if VmRecord::new(&vm).power_state() != vm_power_state::HALTED
            || VmRecord::new(&vm).is_control_domain() != false
        {
            return Err(Error::TransferConflict);
        }
    }
    Ok((reference, d))
}
pub(crate) async fn prepare(
    s: &Session,
    pool: crate::PoolId,
    action: &TransferAction,
    plan: Option<&Plan>,
) -> Result<http::Prepared, Error> {
    let (sr, format, vdi) = match action {
        TransferAction::ExportDisk {
            vdi,
            expected_sr,
            format,
            ..
        } => (*expected_sr, *format, Some(*vdi)),
        TransferAction::ImportDisk { sr, format, .. } => (
            *sr,
            *format,
            plan.and_then(|p| p.disk_creation.as_ref()?.uuid),
        ),
        _ => return Err(Error::TransferConflict),
    };
    // Reuse attached-host routing and trust scope, without requiring write access for an export.
    let mut prepared = super::xapi::storage_route(s, pool, sr, !action.is_export()).await?;
    prepared.kind = http::Kind::Disk;
    prepared.upload = !action.is_export();
    prepared.query = vec![("format".into(), format.extension().into())];
    if let Some(vdi) = vdi {
        let (reference, record) = disk_record(s, vdi, sr, format, !action.is_export()).await?;
        if let TransferAction::ExportDisk { expected_size, .. } = action
            && number(VdiRecord::new(&record).virtual_size()) != Some(*expected_size)
        {
            return Err(Error::TransferConflict);
        }
        prepared.query.push(("vdi".into(), reference));
    }
    Ok(prepared)
}
pub(crate) async fn creation_arguments(
    s: &Session,
    sr: SrId,
    name: &str,
    size: u64,
    id: crate::OperationId,
) -> Result<Vec<Value>, Error> {
    if name.trim().is_empty()
        || name.chars().count() > 255
        || name.chars().any(char::is_control)
        || size == 0
        || size > super::MAX_TRANSFER_BYTES
        || !size.is_multiple_of(512)
    {
        return Err(Error::DiskImage);
    }
    let (reference, r) = fresh(s, "SR", sr.uuid()).await?;
    idle(&r)?;
    if SrRecord::new(&r).content_type() != "user"
        || !crate::xapi::offers(
            SrRecord::new(&r).allowed_operations(),
            storage_operations::VDI_CREATE,
        )
    {
        return Err(Error::UnavailableAction);
    }
    Ok(vec![
        json!({"name_label":name,"name_description":"Imported disk image","SR":reference,"virtual_size":size.to_string(),"type":vdi_type::USER,"sharable":false,"read_only":false,"other_config":{MARKER:id.to_string()},"xenstore_data":{},"sm_config":{},"tags":[]}),
    ])
}
pub(crate) async fn identify(
    s: &Session,
    plan: &Plan,
    action: &TransferAction,
    id: crate::OperationId,
) -> Result<(String, VdiId), Error> {
    let TransferAction::ImportDisk { sr, format, name } = action else {
        return Err(Error::TransferConflict);
    };
    let c = plan
        .disk_creation
        .as_ref()
        .filter(|p| p.dispatched)
        .ok_or(Error::TransferConflict)?;
    let records = s.call("VDI.get_all_records", vec![]).await?;
    let records = records
        .as_object()
        .filter(|m| m.len() <= 10000)
        .ok_or(Error::TransferLimit)?;
    let matches = records
        .iter()
        .filter(|(_, r)| r["other_config"][MARKER] == id.to_string())
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err(Error::TransferConflict);
    }
    let (reference, r) = matches[0];
    let uuid = r["uuid"]
        .as_str()
        .and_then(|s| s.parse().ok())
        .ok_or(Error::TransferConflict)?;
    if c.baseline.contains(&uuid)
        || c.reference.as_ref().is_some_and(|r| r != reference)
        || c.uuid.is_some_and(|id| id.uuid() != uuid)
    {
        return Err(Error::TransferConflict);
    }
    let vdi = VdiId::from(uuid);
    let (reference, record) = disk_record(s, vdi, *sr, *format, true).await?;
    let size = plan
        .file
        .disk
        .as_ref()
        .filter(|d| d.format == *format)
        .ok_or(Error::DiskImage)?
        .virtual_size;
    if VdiRecord::new(&record).read_only() != false
        || VdiRecord::new(&record).is_a_snapshot() != false
        || VdiRecord::new(&record).name_label() != *name
        || VdiRecord::new(&record).other_config()[MARKER] != id.to_string()
        || number(VdiRecord::new(&record).virtual_size()).is_none_or(|n| n < size)
    {
        return Err(Error::TransferConflict);
    }
    Ok((reference, vdi))
}
