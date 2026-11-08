//! Typed storage intent. Authentication material is intentionally absent from journalled inputs.
use crate::{Error, HostId, PbdId, ResourceTarget, SrId, VdiId};
use serde::{Deserialize, Serialize};

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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StorageLocation {
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
        target_iqn: String,
        scsi_id: String,
    },
}
impl StorageLocation {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Nfs { .. } => "nfs",
            Self::IsoNfs { .. } => "iso",
            Self::Iscsi { .. } => "lvmoiscsi",
        }
    }
    pub(crate) fn content_type(&self) -> &'static str {
        if matches!(self, Self::IsoNfs { .. }) {
            "iso"
        } else {
            "user"
        }
    }
    pub(crate) fn validate(&self) -> bool {
        let host = |s: &str| {
            !s.is_empty()
                && s.len() <= 253
                && s.bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b".-:[]".contains(&c))
        };
        let path =
            |s: &str| s.starts_with('/') && s.len() <= 4096 && !s.chars().any(char::is_control);
        let token = |s: &str| {
            !s.is_empty()
                && s.len() <= 256
                && s.bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"-_.:".contains(&c))
        };
        match self {
            Self::Nfs {
                server, path: p, ..
            }
            | Self::IsoNfs {
                server, path: p, ..
            } => host(server) && path(p),
            Self::Iscsi {
                portal,
                target_iqn,
                scsi_id,
            } => {
                host(portal)
                    && token(target_iqn)
                    && target_iqn.starts_with("iqn.")
                    && token(scsi_id)
            }
        }
    }
    pub(crate) fn device_config(&self) -> serde_json::Value {
        use serde_json::json;
        let mut config = match self {
            Self::Nfs { server, path, .. } => json!({"server":server,"serverpath":path}),
            Self::IsoNfs { server, path, .. } => {
                json!({"location":format!("{server}:{path}"),"type":"nfs_iso"})
            }
            Self::Iscsi {
                portal,
                target_iqn,
                scsi_id,
            } => json!({"target":portal,"targetIQN":target_iqn,"SCSIid":scsi_id}),
        };
        if let Self::Nfs { version, .. } | Self::IsoNfs { version, .. } = self
            && let Some(version) = version.value()
        {
            config["nfsversion"] = json!(version);
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
        if valid {
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

pub(crate) fn requires_credentials(config: &serde_json::Value) -> bool {
    config.as_object().is_some_and(|config| {
        config.keys().any(|k| {
            let k = k.to_ascii_lowercase();
            k.contains("password") || k.contains("secret") || k == "chapuser" || k == "username"
        })
    })
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
    let location = if let (Some(server), Some(path)) = (text("server"), text("serverpath")) {
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
            target_iqn: text("targetIQN")?,
            scsi_id: text("SCSIid")?,
        }
    };
    location.validate().then_some(location)
}
pub(crate) fn path_counts(value: &serde_json::Value) -> Vec<crate::StoragePathCount> {
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
    fn typed_locations_reject_embedded_credentials_and_invalid_targets() {
        let location = StorageLocation::Nfs {
            server: "user:password@example.test".into(),
            path: "/images".into(),
            version: NfsVersion::Automatic,
        };
        assert!(!location.validate());
        let location = StorageLocation::Iscsi {
            portal: "192.0.2.12".into(),
            target_iqn: "iqn.2026-01.example:test".into(),
            scsi_id: "3600abcdef".into(),
        };
        assert!(location.validate());
        assert_eq!(location.device_config()["SCSIid"], "3600abcdef");
    }
}
