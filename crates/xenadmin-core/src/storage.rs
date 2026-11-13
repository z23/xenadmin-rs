//! Typed storage intent. Authentication material is intentionally absent from journalled inputs.
use crate::{Error, HostId, PbdId, ResourceTarget, SrId, VdiId};
use serde::{Deserialize, Serialize};
use xenadmin_xapi::{Reads, kind};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NfsVersion {
    Automatic,
    V3,
    V4,
    V41,
}
impl NfsVersion {
    pub(crate) fn value(self) -> Option<&'static str> {
        match self {
            Self::Automatic => None,
            Self::V3 => Some("3"),
            Self::V4 => Some("4"),
            Self::V41 => Some("4.1"),
        }
    }
}
/// Allocation for new LVM-backed storage. Existing repositories are never converted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum StorageAllocation {
    #[default]
    DriverDefault,
    Thick,
    Thin,
}
impl StorageAllocation {
    pub fn label(self) -> &'static str {
        match self {
            Self::DriverDefault => "Driver default",
            Self::Thick => "Thick",
            Self::Thin => "Thin",
        }
    }
    pub(crate) fn value(self) -> Option<&'static str> {
        match self {
            Self::DriverDefault => None,
            Self::Thick => Some("thick"),
            Self::Thin => Some("thin"),
        }
    }
    fn is_default(&self) -> bool {
        *self == Self::DriverDefault
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StorageLocation {
    Local {
        kind: crate::LocalStorageKind,
        device: String,
    },
    IsoSmb {
        server: String,
        share: String,
        path: String,
    },
    Nfs {
        server: String,
        path: String,
        version: NfsVersion,
    },
    IsoNfs {
        server: String,
        path: String,
        version: NfsVersion,
    },
    Iscsi {
        portal: String,
        #[serde(default = "default_iscsi_port")]
        port: u16,
        target_iqn: String,
        scsi_id: String,
        #[serde(default, skip_serializing_if = "StorageAllocation::is_default")]
        allocation: StorageAllocation,
    },
    Hba {
        scsi_id: String,
        #[serde(default, skip_serializing_if = "StorageAllocation::is_default")]
        allocation: StorageAllocation,
    },
}
fn default_iscsi_port() -> u16 {
    3260
}
pub(crate) fn storage_server(value: &str) -> bool {
    let ip = value
        .strip_prefix('[')
        .and_then(|v| v.strip_suffix(']'))
        .unwrap_or(value);
    ip.parse::<std::net::IpAddr>().is_ok()
        || (!value.is_empty()
            && value.len() <= 253
            && value.split('.').all(|part| {
                !part.is_empty()
                    && part.len() <= 63
                    && !part.starts_with('-')
                    && !part.ends_with('-')
                    && part.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
            }))
}
pub(crate) fn storage_token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_.:".contains(&c))
}
impl StorageLocation {
    pub fn shared(&self) -> bool {
        !matches!(self, Self::Local { .. })
    }
    pub fn allocation(&self) -> StorageAllocation {
        match self {
            Self::Iscsi { allocation, .. } | Self::Hba { allocation, .. } => *allocation,
            _ => StorageAllocation::DriverDefault,
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Self::Local { kind, .. } => kind.kind(),
            Self::Nfs { .. } => "nfs",
            Self::IsoNfs { .. } | Self::IsoSmb { .. } => "iso",
            Self::Iscsi { .. } => "lvmoiscsi",
            Self::Hba { .. } => "lvmohba",
        }
    }
    pub(crate) fn content_type(&self) -> &'static str {
        if matches!(self, Self::IsoNfs { .. } | Self::IsoSmb { .. }) {
            "iso"
        } else {
            "user"
        }
    }
    pub(crate) fn validate(&self) -> bool {
        let host = storage_server;
        let path =
            |s: &str| s.starts_with('/') && s.len() <= 4096 && !s.chars().any(char::is_control);
        let token = storage_token;
        match self {
            Self::Local { device, .. } => crate::storage_local::valid_path(device),
            Self::Hba { scsi_id, .. } => token(scsi_id),
            Self::IsoSmb {
                server,
                share,
                path,
            } => {
                host(server)
                    && !share.is_empty()
                    && share.len() <= 255
                    && !share
                        .chars()
                        .any(|c| c.is_control() || matches!(c, '/' | '\\' | ',' | '@'))
                    && !matches!(share.as_str(), "." | "..")
                    && path.len() <= 4096
                    && !path.starts_with('/')
                    && !path.ends_with('/')
                    && !path
                        .chars()
                        .any(|c| c.is_control() || matches!(c, '\\' | ','))
                    && (path.is_empty()
                        || path
                            .split('/')
                            .all(|p| !p.is_empty() && !matches!(p, "." | "..")))
            }
            Self::Nfs {
                server, path: p, ..
            }
            | Self::IsoNfs {
                server, path: p, ..
            } => host(server) && path(p),
            Self::Iscsi {
                portal,
                port,
                target_iqn,
                scsi_id,
                ..
            } => {
                host(portal)
                    && *port > 0
                    && token(target_iqn)
                    && target_iqn.starts_with("iqn.")
                    && token(scsi_id)
            }
        }
    }
    pub(crate) fn device_config(&self) -> serde_json::Value {
        use serde_json::json;
        let mut config = match self {
            Self::Local { device, .. } => json!({"device":device}),
            Self::Hba { scsi_id, .. } => json!({"SCSIid":scsi_id}),
            Self::IsoSmb {
                server,
                share,
                path,
            } => {
                json!({"location":format!("//{server}/{share}{}",if path.is_empty(){String::new()}else{format!("/{path}")}),"type":"cifs","vers":"3.0"})
            }
            Self::Nfs { server, path, .. } => json!({"server":server,"serverpath":path}),
            Self::IsoNfs { server, path, .. } => {
                json!({"location":format!("{server}:{path}"),"type":"nfs_iso"})
            }
            Self::Iscsi {
                portal,
                port,
                target_iqn,
                scsi_id,
                ..
            } => {
                json!({"target":portal,"port":port.to_string(),"targetIQN":target_iqn,"SCSIid":scsi_id})
            }
        };
        if let Self::Nfs { version, .. } | Self::IsoNfs { version, .. } = self
            && let Some(version) = version.value()
        {
            config["nfsversion"] = json!(version);
        }
        if let Some(allocation) = self.allocation().value() {
            config["allocation"] = json!(allocation);
        }
        config
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StorageOperation {
    Trim {
        host: HostId,
        expected_name: String,
    },
    Create {
        host: HostId,
        name: String,
        description: String,
        location: StorageLocation,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hba_device: Option<crate::DiscoveredHbaDevice>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        local_device: Option<crate::LocalDeviceDeclaration>,
    },
    Introduce {
        uuid: SrId,
        name: String,
        description: String,
        location: StorageLocation,
    },
    CreateAttachment {
        host: HostId,
        location: StorageLocation,
    },
    SetAttached {
        sr: SrId,
        host: HostId,
        expected_attached: bool,
        attached: bool,
    },
    RemoveAttachment {
        sr: SrId,
        host: HostId,
    },
    Forget {
        expected_name: String,
        expected_disks: Vec<VdiId>,
    },
    Destroy {
        expected_name: String,
        expected_disks: Vec<VdiId>,
    },
}
impl StorageOperation {
    pub fn permission_requirements(&self) -> crate::PermissionRequirements {
        crate::PermissionRequirements::writes([self.method()])
    }
    pub(crate) fn method(&self) -> &'static str {
        match self {
            StorageOperation::Trim { .. } => "Async.host.call_plugin",
            StorageOperation::Create { .. } => "Async.SR.create",
            StorageOperation::Introduce { .. } => "Async.SR.introduce",
            StorageOperation::CreateAttachment { .. } => "Async.PBD.create",
            StorageOperation::SetAttached { attached: true, .. } => "Async.PBD.plug",
            StorageOperation::SetAttached {
                attached: false, ..
            } => "Async.PBD.unplug",
            StorageOperation::RemoveAttachment { .. } => "Async.PBD.destroy",
            StorageOperation::Forget { .. } => "Async.SR.forget",
            StorageOperation::Destroy { .. } => "Async.SR.destroy",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Trim { .. } => "Reclaim unused storage blocks",
            Self::Create { .. } => "Create storage repository",
            Self::Introduce { .. } => "Introduce existing storage repository",
            Self::CreateAttachment { .. } => "Create host storage attachment",
            Self::SetAttached { attached: true, .. } => "Attach storage on host",
            Self::SetAttached {
                attached: false, ..
            } => "Detach storage from host",
            Self::RemoveAttachment { .. } => "Remove detached host attachment",
            Self::Forget { .. } => "Forget storage repository",
            Self::Destroy { .. } => "Destroy storage repository and data",
        }
    }
    pub(crate) fn validate(&self, target: ResourceTarget) -> Result<(), Error> {
        let metadata = |name: &str, description: &str| {
            !name.trim().is_empty()
                && name.len() <= 255
                && !name.chars().any(char::is_control)
                && description.len() <= 16384
                && !description.contains('\0')
        };
        let valid = match self {
            Self::Trim { expected_name, .. } => {
                matches!(target, ResourceTarget::Storage(_)) && expected_name.len() <= 4096
            }
            Self::Create {
                name,
                description,
                location,
                ..
            }
            | Self::Introduce {
                name,
                description,
                location,
                ..
            } => {
                matches!(target, ResourceTarget::Pool(_))
                    && metadata(name, description)
                    && location.validate()
            }
            Self::CreateAttachment { location, .. } => {
                matches!(target, ResourceTarget::Storage(_)) && location.validate()
            }
            Self::SetAttached {
                expected_attached,
                attached,
                ..
            } => matches!(target, ResourceTarget::Pbd(_)) && expected_attached != attached,
            Self::RemoveAttachment { .. } => matches!(target, ResourceTarget::Pbd(_)),
            Self::Forget {
                expected_name,
                expected_disks,
            }
            | Self::Destroy {
                expected_name,
                expected_disks,
            } => {
                matches!(target, ResourceTarget::Storage(_))
                    && expected_name.len() <= 4096
                    && expected_disks.len() <= 10000
                    && expected_disks
                        .iter()
                        .collect::<std::collections::BTreeSet<_>>()
                        .len()
                        == expected_disks.len()
            }
        };
        let hba_valid = match self {
            Self::Create {
                location: StorageLocation::Hba { scsi_id, .. },
                hba_device: Some(device),
                ..
            } => device.valid() && &device.scsi_id == scsi_id,
            Self::Create {
                location: StorageLocation::Hba { .. },
                ..
            } => false,
            Self::Create { hba_device, .. } => hba_device.is_none(),
            Self::Introduce { location, .. } => {
                location.allocation() == StorageAllocation::DriverDefault
            }
            _ => true,
        };
        let local_valid = match self {
            Self::Create {
                location: StorageLocation::Local { .. },
                local_device: Some(declaration),
                ..
            } => declaration.valid(),
            Self::Create {
                location: StorageLocation::Local { .. },
                ..
            } => false,
            Self::Create { local_device, .. } => local_device.is_none(),
            _ => true,
        };
        if valid && hba_valid && local_valid {
            Ok(())
        } else {
            Err(Error::InvalidResourceCommand)
        }
    }
    pub(crate) fn creates_reference(&self) -> bool {
        matches!(
            self,
            Self::Create { .. } | Self::Introduce { .. } | Self::CreateAttachment { .. }
        )
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct StorageBaseline {
    pub srs: Vec<SrId>,
    pub pbds: Vec<PbdId>,
}

pub(crate) fn requires_credentials<'a>(config: impl Reads<'a, kind::Map>) -> bool {
    let config = config.raw();
    config.as_object().is_some_and(|config| {
        config.keys().any(|k| {
            let k = k.to_ascii_lowercase();
            k.contains("password")
                || k.contains("secret")
                || k == "chapuser"
                || k == "incoming_chapuser"
                || k == "username"
        })
    })
}
pub(crate) fn safe_location_for_kind<'a>(
    config: impl Reads<'a, kind::Map>,
    kind: Option<&str>,
) -> Option<StorageLocation> {
    let config = config.raw();
    let kind = match kind {
        Some("ext") => Some(crate::LocalStorageKind::Ext),
        Some("lvm") => Some(crate::LocalStorageKind::Lvm),
        _ => None,
    };
    if let Some(kind) = kind {
        let device = config["device"].as_str()?.to_owned();
        return crate::storage_local::valid_path(&device)
            .then_some(StorageLocation::Local { kind, device });
    }
    safe_location_from_config(config)
}
pub(crate) fn safe_location_from_config<'a>(
    config: impl Reads<'a, kind::Map>,
) -> Option<StorageLocation> {
    let config = config.raw();
    let mut public = config.clone();
    let map = public.as_object_mut()?;
    map.retain(|k, _| {
        let k = k.to_ascii_lowercase();
        !k.contains("password")
            && !k.contains("secret")
            && k != "chapuser"
            && k != "incoming_chapuser"
            && k != "username"
    });
    location_from_config(&public)
}
pub(crate) fn location_from_config(config: &serde_json::Value) -> Option<StorageLocation> {
    if requires_credentials(config) {
        return None;
    }
    let text = |key: &str| config[key].as_str().map(str::to_owned);
    let version = match config["nfsversion"].as_str() {
        None | Some("") => NfsVersion::Automatic,
        Some("3") => NfsVersion::V3,
        Some("4") => NfsVersion::V4,
        Some("4.1") => NfsVersion::V41,
        _ => return None,
    };
    let allocation = match config["allocation"].as_str() {
        None => StorageAllocation::DriverDefault,
        Some("thin") => StorageAllocation::Thin,
        Some("thick") => StorageAllocation::Thick,
        _ => return None,
    };
    let location = if config.get("target").is_none() && config.get("SCSIid").is_some() {
        StorageLocation::Hba {
            scsi_id: text("SCSIid")?,
            allocation,
        }
    } else if config["type"] == "cifs" {
        if config["vers"] != "3.0" {
            return None;
        }
        let value = text("location")?;
        let tail = value.strip_prefix("//")?;
        let (server, rest) = tail.split_once('/')?;
        let (share, path) = rest.split_once('/').unwrap_or((rest, ""));
        StorageLocation::IsoSmb {
            server: server.into(),
            share: share.into(),
            path: path.into(),
        }
    } else if let (Some(server), Some(path)) = (text("server"), text("serverpath")) {
        StorageLocation::Nfs {
            server,
            path,
            version,
        }
    } else if config["type"] == "nfs_iso" {
        let location = text("location")?;
        let (server, path) = location.split_once(":/")?;
        StorageLocation::IsoNfs {
            server: server.into(),
            path: format!("/{path}"),
            version,
        }
    } else {
        StorageLocation::Iscsi {
            portal: text("target")?,
            port: match config.get("port") {
                None => 3260,
                Some(value) => value.as_str()?.parse().ok()?,
            },
            target_iqn: text("targetIQN")?,
            scsi_id: text("SCSIid")?,
            allocation,
        }
    };
    location.validate().then_some(location)
}
pub(crate) fn path_counts<'a>(value: impl Reads<'a, kind::Map>) -> Vec<crate::StoragePathCount> {
    let value = value.raw();
    value
        .as_object()
        .into_iter()
        .flat_map(|v| v.iter())
        .take(256)
        .filter_map(|(key, value)| {
            let device = key.strip_prefix("mpath-")?;
            if device.len() > 256 || device.chars().any(char::is_control) {
                return None;
            }
            let counts = value.as_str()?.strip_prefix('[')?.strip_suffix(']')?;
            let (active, maximum) = counts.split_once(',')?;
            let active = active.trim().trim_end_matches('L').parse::<u32>().ok()?;
            let maximum = maximum.trim().trim_end_matches('L').parse::<u32>().ok()?;
            (active <= maximum).then(|| crate::StoragePathCount {
                device: device.into(),
                active,
                maximum,
            })
        })
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn legacy_create_payload_keeps_driver_default_and_hba_requires_reviewed_identity() {
        let old = json!({"Create":{"host":"20000000-0000-0000-0000-000000000001","name":"Fixture","description":"","location":{"Iscsi":{"portal":"192.0.2.1","port":3260,"target_iqn":"iqn.2026-11.example:fixture","scsi_id":"3600fixture"}}}});
        let operation: StorageOperation = serde_json::from_value(old.clone()).unwrap();
        assert_eq!(serde_json::to_value(&operation).unwrap(), old);
        let pool = ResourceTarget::Pool(crate::PoolId::from(uuid::Uuid::from_u128(1)));
        assert!(operation.validate(pool).is_ok());
        let mut operation = StorageOperation::Create {
            host: crate::HostId::from(uuid::Uuid::from_u128(2)),
            name: "HBA".into(),
            description: "".into(),
            location: StorageLocation::Hba {
                scsi_id: "3600fixture".into(),
                allocation: StorageAllocation::Thin,
            },
            hba_device: None,
            local_device: None,
        };
        assert!(operation.validate(pool).is_err());
        if let StorageOperation::Create { hba_device, .. } = &mut operation {
            *hba_device = Some(crate::DiscoveredHbaDevice {
                scsi_id: "3600fixture".into(),
                size_bytes: 4096,
                vendor: "Fixture".into(),
                serial: "123".into(),
            });
        }
        assert!(operation.validate(pool).is_ok());
        if let StorageOperation::Create {
            hba_device: Some(device),
            ..
        } = &mut operation
        {
            device.scsi_id = "different".into();
        }
        assert!(operation.validate(pool).is_err());
        let location = StorageLocation::Hba {
            scsi_id: "3600fixture".into(),
            allocation: StorageAllocation::Thin,
        };
        assert_eq!(
            location_from_config(&location.device_config()),
            Some(location)
        );
        assert!(
            location_from_config(&json!({"SCSIid":"3600fixture","allocation":"unexpected"}))
                .is_none()
        );
    }
    #[test]
    fn smb_projection_preserves_only_safe_location_and_never_downgrades() {
        let config = json!({"location":"//smb.example.test/images/install","type":"cifs","vers":"3.0","username":"DOMAIN\\private","cifspassword_secret":"private-secret-uuid"});
        assert!(requires_credentials(&config));
        assert!(location_from_config(&config).is_none());
        let location = safe_location_from_config(&config).unwrap();
        assert_eq!(
            location.device_config(),
            json!({"location":"//smb.example.test/images/install","type":"cifs","vers":"3.0"})
        );
        let mut legacy = config.clone();
        legacy["vers"] = json!("1.0");
        assert!(safe_location_from_config(&legacy).is_none());
        legacy.as_object_mut().unwrap().remove("vers");
        assert!(safe_location_from_config(&legacy).is_none());
    }
    #[test]
    fn attachment_projection_never_copies_authentication_material() {
        let config = json!({"server":"nfs.example.test","serverpath":"/images","chappassword_secret":"private"});
        assert!(location_from_config(&config).is_none());
        assert!(requires_credentials(&config));
        let config = json!({"server":"nfs.example.test","serverpath":"/images"});
        assert!(matches!(
            location_from_config(&config),
            Some(StorageLocation::Nfs { .. })
        ));
        let paths = path_counts(
            &json!({"mpath-test":"[2L, 4L]","mpath-bad":"[5, 2]","password":"private"}),
        );
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].active, 2);
    }
    #[test]
    fn iscsi_port_survives_projection_and_old_journal_defaults() {
        let old = json!({"Iscsi":{"portal":"192.0.2.2","target_iqn":"iqn.2026-01.example:test","scsi_id":"3600abc"}});
        let location: StorageLocation = serde_json::from_value(old).unwrap();
        assert_eq!(location.device_config()["port"], "3260");
        let config = json!({"target":"192.0.2.2","targetIQN":"iqn.2026-01.example:test","SCSIid":"3600abc","port":"3261"});
        assert_eq!(
            location_from_config(&config).unwrap().device_config()["port"],
            "3261"
        );
        for invalid in ["0", "65536", "bad"] {
            let mut config = config.clone();
            config["port"] = json!(invalid);
            assert!(location_from_config(&config).is_none());
        }
    }
    #[test]
    fn typed_locations_reject_embedded_credentials_and_invalid_targets() {
        let location = StorageLocation::Nfs {
            server: "user:password@example.test".into(),
            path: "/images".into(),
            version: NfsVersion::Automatic,
        };
        assert!(!location.validate());
        let location = StorageLocation::Iscsi {
            portal: "192.0.2.12".into(),
            port: 3260,
            target_iqn: "iqn.2026-01.example:test".into(),
            scsi_id: "3600abcdef".into(),
            allocation: crate::StorageAllocation::DriverDefault,
        };
        assert!(location.validate());
        assert_eq!(location.device_config()["SCSIid"], "3600abcdef");
    }
}
