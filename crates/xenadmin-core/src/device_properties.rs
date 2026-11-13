use crate::{Error, VbdId, VdiId, VifId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use xenadmin_xapi::enumerations::vbd_mode;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiskAccess {
    ReadOnly,
    ReadWrite,
}
impl DiskAccess {
    pub(crate) fn wire(self) -> &'static str {
        match self {
            Self::ReadOnly => vbd_mode::RO,
            Self::ReadWrite => vbd_mode::RW,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceQos {
    pub algorithm: String,
    pub parameters: BTreeMap<String, String>,
}
impl DeviceQos {
    pub(crate) fn from_record(record: &serde_json::Value) -> Option<Self> {
        let qos = Self {
            algorithm: record["qos_algorithm_type"].as_str()?.into(),
            parameters: serde_json::from_value(record["qos_algorithm_params"].clone()).ok()?,
        };
        qos.valid().then_some(qos)
    }
    pub(crate) fn valid(&self) -> bool {
        self.algorithm.len() <= 64
            && self.parameters.len() <= 64
            && self.parameters.iter().all(|(k, v)| {
                k.len() <= 128 && v.len() <= 1024 && !k.contains('\0') && !v.contains('\0')
            })
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeviceProperty {
    DiskMetadata {
        vbd: VbdId,
        vdi: VdiId,
        previous_name: String,
        previous_description: String,
        name: String,
        description: String,
    },
    DiskAccess {
        vbd: VbdId,
        vdi: VdiId,
        previous: DiskAccess,
        value: DiskAccess,
    },
    DiskPosition {
        vbd: VbdId,
        vdi: VdiId,
        previous: String,
        value: String,
    },
    Bootable {
        vbd: VbdId,
        vdi: VdiId,
        previous: bool,
        value: bool,
    },
    DiskPriority {
        vbd: VbdId,
        vdi: VdiId,
        previous: DeviceQos,
        priority: Option<u8>,
    },
    NicRate {
        vif: VifId,
        previous: DeviceQos,
        kilobytes_per_second: Option<u64>,
    },
    DisableCbt {
        vbd: VbdId,
        vdi: VdiId,
    },
}
impl DeviceProperty {
    pub fn label(&self) -> &'static str {
        match self {
            Self::DiskMetadata { .. } => "Edit disk name and description",
            Self::DiskAccess { .. } => "Set disk access mode",
            Self::DiskPosition { .. } => "Set disk device position",
            Self::Bootable { .. } => "Set disk bootable flag",
            Self::DiskPriority { .. } => "Set disk I/O priority",
            Self::NicRate { .. } => "Set NIC rate limit",
            Self::DisableCbt { .. } => "Disable changed-block tracking",
        }
    }
    pub(crate) fn requires_halted(&self) -> bool {
        !matches!(self, Self::DiskMetadata { .. })
    }
    pub(crate) fn validate(&self) -> Result<(), Error> {
        let text = |v: &str| v.len() <= 16_384 && !v.contains('\0');
        let slot = |v: &str| v.parse::<u8>().is_ok_and(|n| n.to_string() == v);
        let valid = match self {
            Self::DiskMetadata {
                previous_name,
                previous_description,
                name,
                description,
                ..
            } => {
                !name.trim().is_empty()
                    && name.chars().count() <= 255
                    && !name.chars().any(char::is_control)
                    && text(description)
                    && text(previous_name)
                    && text(previous_description)
                    && (name != previous_name || description != previous_description)
            }
            Self::DiskAccess {
                previous, value, ..
            } => previous != value,
            Self::DiskPosition {
                previous, value, ..
            } => slot(previous) && slot(value) && previous != value,
            Self::Bootable {
                previous, value, ..
            } => previous != value,
            Self::DiskPriority {
                previous, priority, ..
            } => {
                previous.valid()
                    && matches!(previous.algorithm.as_str(), "" | "ionice")
                    && priority.is_none_or(|n| n <= 7)
            }
            Self::NicRate {
                previous,
                kilobytes_per_second,
                ..
            } => {
                previous.valid()
                    && matches!(previous.algorithm.as_str(), "" | "ratelimit")
                    && kilobytes_per_second.is_none_or(|n| n > 0 && n <= i64::MAX as u64)
            }
            Self::DisableCbt { .. } => true,
        };
        if valid {
            Ok(())
        } else {
            Err(Error::InvalidDeviceConfiguration)
        }
    }
    pub(crate) fn resources(&self) -> Vec<String> {
        match self {
            Self::NicRate { vif, .. } => vec![format!("vif:{vif}")],
            Self::DiskMetadata { vbd, vdi, .. }
            | Self::DiskAccess { vbd, vdi, .. }
            | Self::DiskPosition { vbd, vdi, .. }
            | Self::Bootable { vbd, vdi, .. }
            | Self::DiskPriority { vbd, vdi, .. }
            | Self::DisableCbt { vbd, vdi } => vec![format!("vbd:{vbd}"), format!("vdi:{vdi}")],
        }
    }
}
