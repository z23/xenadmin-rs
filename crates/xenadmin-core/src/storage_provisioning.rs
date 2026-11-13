//! Driver-qualified allocation and destructive HBA selection checks.
use crate::{DiscoveredHbaDevice, Error, StorageLocation, storage_discovery, xapi::Session};
use serde_json::{Value, json};
use xenadmin_xapi::records::{HostRecord, SrRecord};

pub(crate) async fn allocation_capability(
    session: &Session,
    location: &StorageLocation,
) -> Result<(), Error> {
    if location.allocation().value().is_none() {
        return Ok(());
    }
    let records = session.call("SM.get_all_records", vec![]).await?;
    let records = records
        .as_object()
        .filter(|v| v.len() <= 256)
        .ok_or(Error::Protocol("storage driver capabilities unavailable"))?;
    let mut matching = records.values().filter(|v| v["type"] == location.kind());
    let driver = matching.next().ok_or(Error::UnavailableAction)?;
    if matching.next().is_some()
        || !driver["configuration"]
            .as_object()
            .is_some_and(|v| v.contains_key("allocation"))
    {
        return Err(Error::Protocol(
            "This storage driver does not advertise an allocation choice",
        ));
    }
    Ok(())
}

pub(crate) fn verify_allocation(sr: &Value, location: &StorageLocation) -> Result<(), Error> {
    if let Some(expected) = location.allocation().value()
        && SrRecord::new(sr).sm_config()["allocation"] != expected
    {
        return Err(Error::Protocol(
            "The storage allocation mode could not be verified",
        ));
    }
    Ok(())
}

pub(crate) async fn hba_creation(
    session: &Session,
    host: &str,
    host_id: crate::HostId,
    location: &StorageLocation,
    device: &DiscoveredHbaDevice,
) -> Result<(), Error> {
    let StorageLocation::Hba { scsi_id, .. } = location else {
        return Err(Error::InvalidResourceCommand);
    };
    if !device.valid() || &device.scsi_id != scsi_id {
        return Err(Error::InvalidResourceCommand);
    }
    let host_record = session.call("host.get_record", vec![json!(host)]).await?;
    if HostRecord::new(&host_record).uuid() != host_id.to_string() {
        return Err(Error::Stale);
    }
    if !HostRecord::new(&host_record)
        .current_operations()
        .entries()
        .is_some_and(|v| v.is_empty())
    {
        return Err(Error::UnavailableAction);
    }
    // A complete database scan also covers detached attachments. The driver independently
    // excludes the host root device and currently registered LUNs from its discovery list.
    for (class, field) in [("PBD", "device_config"), ("SR", "sm_config")] {
        let records = session
            .call(&format!("{class}.get_all_records"), vec![])
            .await?;
        let records = records
            .as_object()
            .filter(|v| v.len() <= 10000)
            .ok_or(Error::Protocol("Storage device ownership data unavailable"))?;
        for record in records.values() {
            let config = record[field]
                .as_object()
                .ok_or(Error::Protocol("Storage device ownership data incomplete"))?;
            if config.iter().any(|(key, value)| {
                key.eq_ignore_ascii_case("SCSIid")
                    && value
                        .as_str()
                        .is_none_or(|id| id.eq_ignore_ascii_case(scsi_id))
            }) {
                return Err(Error::StorageInUse);
            }
        }
    }
    let xml = session
        .storage_probe_xml(
            vec![
                json!(host),
                location.device_config(),
                json!(location.kind()),
                json!({}),
            ],
            &[],
        )
        .await?;
    if !storage_discovery::decode_repositories(&xml)?.is_empty() {
        return Err(Error::Protocol(
            "Existing storage was found on this LUN; introduce it instead of formatting it",
        ));
    }
    let devices = storage_discovery::hba_devices(session, host).await?;
    if devices.iter().find(|v| v.scsi_id == *scsi_id) != Some(device) {
        return Err(Error::Protocol(
            "The selected HBA device changed or is no longer available; discover and review it again",
        ));
    }
    Ok(())
}
