//! Local storage uses an operator-provided persistent path, not an invented disk inventory.
use crate::{Error, HostId, StorageLocation, storage_discovery, xapi::Session};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use xenadmin_xapi::records::{HostRecord, PbdRecord, SmRecord, SrRecord};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LocalStorageKind {
    Ext,
    Lvm,
}
impl LocalStorageKind {
    pub fn kind(self) -> &'static str {
        match self {
            Self::Ext => "ext",
            Self::Lvm => "lvm",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Ext => "Local EXT",
            Self::Lvm => "Local LVM",
        }
    }
}
/// A declaration by the operator; it is never treated as server-verified physical evidence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalDeviceDeclaration {
    pub identity_statement: String,
    pub acknowledge_data_loss: bool,
}
impl LocalDeviceDeclaration {
    pub(crate) fn valid(&self) -> bool {
        self.acknowledge_data_loss
            && !self.identity_statement.trim().is_empty()
            && self.identity_statement.len() <= 512
            && !self.identity_statement.chars().any(char::is_control)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredLocalRepository {
    pub kind: LocalStorageKind,
    pub repository: crate::DiscoveredRepository,
}
pub(crate) fn valid_path(device: &str) -> bool {
    device.strip_prefix("/dev/disk/by-id/").is_some_and(|name| {
        !name.is_empty()
            && name.len() <= 255
            && name != "."
            && name != ".."
            && !name.starts_with(['.', '-'])
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b))
            && !["dm-", "md-", "lvm-pv-uuid-"]
                .iter()
                .any(|prefix| name.starts_with(prefix))
    })
}
fn device_family(device: &str) -> &str {
    let device = device.strip_prefix("/dev/disk/by-id/").unwrap_or(device);
    match device.rsplit_once("-part") {
        Some((base, partition))
            if !partition.is_empty() && partition.bytes().all(|b| b.is_ascii_digit()) =>
        {
            base
        }
        _ => device,
    }
}
fn same_declared_device(a: &str, b: &str) -> bool {
    device_family(a).eq_ignore_ascii_case(device_family(b))
}
fn records(value: &Value) -> Result<&serde_json::Map<String, Value>, Error> {
    value
        .as_object()
        .filter(|m| m.len() <= 10000)
        .ok_or(Error::Protocol("Local storage ownership data unavailable"))
}
pub(crate) async fn probe(
    session: &Session,
    host: HostId,
    device: &str,
) -> Result<Vec<DiscoveredLocalRepository>, Error> {
    if !valid_path(device) {
        return Err(Error::InvalidResourceCommand);
    }
    let (host_ref, _) = read_host(session, host).await?;
    let drivers = session.call("SM.get_all_records", vec![]).await?;
    let drivers = drivers
        .as_object()
        .filter(|r| r.len() <= 256)
        .ok_or(Error::Protocol("Storage driver capabilities unavailable"))?;
    let mut result = Vec::new();
    for kind in [LocalStorageKind::Ext, LocalStorageKind::Lvm] {
        let mut matching = drivers
            .values()
            .filter(|r| SmRecord::new(r).type_() == kind.kind());
        let driver = matching.next().ok_or(Error::Protocol(
            "Both local EXT and LVM probing drivers are required",
        ))?;
        if matching.next().is_some()
            || !SmRecord::new(driver)
                .configuration()
                .entries()
                .is_some_and(|m| m.contains_key("device"))
            || !(SmRecord::new(driver)
                .capabilities()
                .members()
                .is_some_and(|a| a.iter().any(|v| v == "SR_PROBE"))
                || SmRecord::new(driver)
                    .features()
                    .entries()
                    .is_some_and(|m| m.contains_key("SR_PROBE")))
        {
            return Err(Error::UnavailableAction);
        }
        let xml = session
            .storage_probe_xml(
                vec![
                    json!(&host_ref),
                    json!({"device":device}),
                    json!(kind.kind()),
                    json!({}),
                ],
                &[],
            )
            .await?;
        for repository in storage_discovery::decode_repositories(&xml)? {
            result.push(DiscoveredLocalRepository { kind, repository });
        }
    }
    Ok(result)
}
pub(crate) async fn preflight(
    session: &Session,
    host: HostId,
    location: &StorageLocation,
) -> Result<(), Error> {
    let StorageLocation::Local { device, .. } = location else {
        return Err(Error::InvalidResourceCommand);
    };
    let (host_ref, host_record) = read_host(session, host).await?;
    if !HostRecord::new(&host_record)
        .current_operations()
        .entries()
        .is_some_and(|m| m.is_empty())
    {
        return Err(Error::UnavailableAction);
    }
    let pbds = session.call("PBD.get_all_records", vec![]).await?;
    for pbd in records(&pbds)?.values() {
        let owner = PbdRecord::new(pbd)
            .host()
            .reference()
            .ok_or(Error::Protocol("Storage attachment host unavailable"))?;
        if owner != host_ref {
            continue;
        }
        let config = PbdRecord::new(pbd)
            .device_config()
            .entries()
            .ok_or(Error::Protocol("Storage attachment location unavailable"))?;
        if let Some(existing) = config.get("device") {
            let existing = existing
                .as_str()
                .filter(|s| s.len() <= 16384)
                .ok_or(Error::Protocol("Storage attachment location unavailable"))?;
            if existing.split(',').any(|v| same_declared_device(v, device)) {
                return Err(Error::StorageInUse);
            }
        }
        if let Some(id) = config.get("SCSIid") {
            let id = id
                .as_str()
                .ok_or(Error::Protocol("Storage SCSI identity unavailable"))?;
            if same_declared_device(&format!("scsi-{id}"), device) {
                return Err(Error::StorageInUse);
            }
        }
    }
    let srs = session.call("SR.get_all_records", vec![]).await?;
    for sr in records(&srs)?.values() {
        let config = SrRecord::new(sr)
            .sm_config()
            .entries()
            .ok_or(Error::Protocol("Storage serial ownership data unavailable"))?;
        for key in ["devserial", "SCSIid"] {
            if let Some(serial) = config.get(key) {
                let serial = serial
                    .as_str()
                    .filter(|s| s.len() <= 65536)
                    .ok_or(Error::Protocol("Storage serial ownership data unavailable"))?;
                if serial.split(',').any(|v| {
                    same_declared_device(v, device)
                        || key == "SCSIid" && same_declared_device(&format!("scsi-{v}"), device)
                }) {
                    return Err(Error::StorageInUse);
                }
            }
        }
    }
    if !probe(session, host, device).await?.is_empty() {
        return Err(Error::Protocol(
            "An existing EXT or LVM repository was found; introduce it rather than formatting this device",
        ));
    }
    // Empty probe and unmatched aliases do not establish physical ownership or blank media.
    Ok(())
}

async fn read_host(session: &Session, host: HostId) -> Result<(String, Value), Error> {
    let reference = session
        .call("host.get_by_uuid", vec![json!(host.to_string())])
        .await?;
    let reference = crate::xapi::opaque_reference(&reference)?;
    let record = session
        .call("host.get_record", vec![json!(&reference)])
        .await?;
    if HostRecord::new(&record).uuid() != host.to_string() {
        return Err(Error::Stale);
    }
    Ok((reference, record))
}

pub(crate) fn attachment_graph(sr: &Value, reference: &str) -> Result<(), Error> {
    if matches!(SrRecord::new(sr).type_().text(), Some("ext" | "lvm"))
        && (SrRecord::new(sr).shared() != false
            || !SrRecord::new(sr)
                .pbds()
                .members()
                .is_some_and(|a| a.len() == 1 && a[0] == reference))
    {
        return Err(Error::Protocol(
            "Local storage must have exactly one selected host attachment",
        ));
    }
    Ok(())
}
pub(crate) async fn no_attachments(session: &Session, sr_ref: &str) -> Result<(), Error> {
    let pbds = session.call("PBD.get_all_records", vec![]).await?;
    if records(&pbds)?
        .values()
        .any(|p| PbdRecord::new(p).sr() == sr_ref)
    {
        return Err(Error::Protocol(
            "Local storage already has a host attachment; repair that attachment instead",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_paths_and_declarations_do_not_accept_lists_traversal_or_unacknowledged_formatting() {
        for path in [
            "/dev/sda",
            "/dev/disk/by-id/../sda",
            "/dev/disk/by-id/a,b",
            "/dev/disk/by-id/",
            "/dev/disk/by-id/dm-name-data",
            "/dev/disk/by-id/ata-one;rm",
            "/dev/disk/by-id/ata-one\n",
        ] {
            assert!(!valid_path(path), "{path}");
        }
        for path in [
            "/dev/disk/by-id/ata-FIXTURE",
            "/dev/disk/by-id/nvme-eui.11223344-part3",
        ] {
            assert!(valid_path(path));
        }
        assert!(same_declared_device(
            "/dev/disk/by-id/ata-FIXTURE-part2",
            "/dev/disk/by-id/ata-FIXTURE"
        ));
        assert!(!same_declared_device(
            "/dev/sdb1",
            "/dev/disk/by-id/ata-FIXTURE"
        ));
        assert!(
            !LocalDeviceDeclaration {
                identity_statement: "".into(),
                acknowledge_data_loss: true
            }
            .valid()
        );
        assert!(
            !LocalDeviceDeclaration {
                identity_statement: "operator declaration".into(),
                acknowledge_data_loss: false
            }
            .valid()
        );
        let location = StorageLocation::Local {
            kind: LocalStorageKind::Ext,
            device: "/dev/disk/by-id/ata-FIXTURE".into(),
        };
        assert_eq!(
            crate::storage::safe_location_for_kind(&location.device_config(), Some("ext")),
            Some(location)
        );
    }
}
