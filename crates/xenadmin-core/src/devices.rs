use crate::{
    ActionAvailability, Error, Inventory, NetworkId, ObservedVm, OperationAction, OperationRecord,
    PowerState, SrId, VbdId, VdiId, VifId, VmId, xapi::Session,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use xenadmin_xapi::records::{SrRecord, VbdRecord, VdiRecord, VmRecord};

pub struct DeviceCommand {
    pub vm: ObservedVm,
    pub operation: DeviceOperation,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "device_action", rename_all = "snake_case")]
pub enum DeviceOperation {
    MountGuestTools {
        vbd: VbdId,
        image: VdiId,
        sr: SrId,
        previous: Option<VdiId>,
    },
    Property {
        change: crate::DeviceProperty,
    },
    CreateDisk {
        sr: SrId,
        name: String,
        size: u64,
        device: String,
    },
    AttachDisk {
        vdi: VdiId,
        device: String,
        read_only: bool,
    },
    DetachDisk {
        vbd: VbdId,
        vdi: VdiId,
    },
    DeleteDisk {
        vbd: VbdId,
        vdi: VdiId,
    },
    AddCdDrive {
        device: String,
    },
    AddNic {
        network: NetworkId,
        device: String,
        mac: Option<String>,
    },
    DeleteNic {
        vif: VifId,
    },
    EditNic {
        vif: VifId,
        expected_network: NetworkId,
        expected_mac: String,
        network: NetworkId,
        mac: String,
    },
    InsertCd {
        vbd: VbdId,
        image: VdiId,
    },
    EjectCd {
        vbd: VbdId,
    },
    PlugDisk {
        vbd: VbdId,
    },
    UnplugDisk {
        vbd: VbdId,
    },
    PlugNic {
        vif: VifId,
    },
    UnplugNic {
        vif: VifId,
    },
    GrowDisk {
        vdi: VdiId,
        expected_size: u64,
        new_size: u64,
    },
}

impl DeviceOperation {
    pub fn permission_requirements(&self) -> crate::PermissionRequirements {
        use DeviceOperation::*;
        let methods: &[&str] = match self {
            MountGuestTools {
                previous: Some(_), ..
            } => &["VBD.eject", "VBD.insert"],
            MountGuestTools { previous: None, .. } => &["VBD.insert"],
            Property { change } => return crate::device_lifecycle::property_requirements(change),
            CreateDisk { .. } => &["VDI.create", "VBD.create"],
            AttachDisk { .. } | AddCdDrive { .. } => &["VBD.create"],
            DetachDisk { .. } => &["VBD.destroy"],
            DeleteDisk { .. } => &["VBD.destroy", "VDI.destroy"],
            AddNic { .. } => &["VIF.create"],
            DeleteNic { .. } => &["VIF.destroy"],
            EditNic {
                expected_mac,
                mac,
                expected_network,
                network,
                ..
            } => {
                if !expected_mac.eq_ignore_ascii_case(mac) {
                    &["VIF.destroy", "VIF.create"]
                } else if expected_network != network {
                    &["VIF.move"]
                } else {
                    &[]
                }
            }
            InsertCd { .. } => &["VBD.insert"],
            EjectCd { .. } => &["VBD.eject"],
            PlugDisk { .. } => &["VBD.plug"],
            UnplugDisk { .. } => &["VBD.unplug"],
            PlugNic { .. } => &["VIF.plug"],
            UnplugNic { .. } => &["VIF.unplug"],
            GrowDisk { .. } => &["VDI.resize"],
        };
        crate::PermissionRequirements::writes(methods)
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::MountGuestTools { .. } => "Mount guest tools media",
            Self::Property { change } => change.label(),
            Self::CreateDisk { .. } => "Create and attach disk",
            Self::AttachDisk { .. } => "Attach existing disk",
            Self::DetachDisk { .. } => "Detach disk",
            Self::DeleteDisk { .. } => "Delete disk and attachment",
            Self::AddCdDrive { .. } => "Add CD drive",
            Self::AddNic { .. } => "Add network interface",
            Self::DeleteNic { .. } => "Delete network interface",
            Self::EditNic { .. } => "Edit network interface",
            Self::InsertCd { .. } => "Insert ISO",
            Self::EjectCd { .. } => "Eject ISO",
            Self::PlugDisk { .. } => "Activate disk attachment",
            Self::UnplugDisk { .. } => "Deactivate disk attachment",
            Self::PlugNic { .. } => "Connect NIC",
            Self::UnplugNic { .. } => "Disconnect NIC",
            Self::GrowDisk { .. } => "Grow disk",
        }
    }

    pub(crate) fn resources(&self) -> Vec<String> {
        match self {
            Self::MountGuestTools {
                vbd,
                image,
                sr,
                previous,
            } => {
                let mut resources = vec![
                    format!("vbd:{vbd}"),
                    format!("vdi:{image}"),
                    format!("sr:{sr}"),
                ];
                resources.extend(previous.iter().map(|id| format!("vdi:{id}")));
                resources
            }
            Self::Property { change } => change.resources(),
            Self::CreateDisk { sr, .. } => vec![format!("sr:{sr}")],
            Self::AttachDisk { vdi, .. } => vec![format!("vdi:{vdi}")],
            Self::DetachDisk { vbd, vdi } | Self::DeleteDisk { vbd, vdi } => {
                vec![format!("vbd:{vbd}"), format!("vdi:{vdi}")]
            }
            Self::AddCdDrive { .. } => vec![],
            Self::AddNic { network, .. } => vec![format!("network:{network}")],
            Self::DeleteNic { vif } => vec![format!("vif:{vif}")],
            Self::EditNic { vif, network, .. } => {
                vec![format!("vif:{vif}"), format!("network:{network}")]
            }
            Self::InsertCd { vbd, image } => vec![format!("vbd:{vbd}"), format!("vdi:{image}")],
            Self::EjectCd { vbd } | Self::PlugDisk { vbd } | Self::UnplugDisk { vbd } => {
                vec![format!("vbd:{vbd}")]
            }
            Self::PlugNic { vif } | Self::UnplugNic { vif } => vec![format!("vif:{vif}")],
            Self::GrowDisk { vdi, .. } => vec![format!("vdi:{vdi}")],
        }
    }

    pub(crate) fn validate(&self) -> Result<(), Error> {
        crate::device_lifecycle::validate(self)?;
        if let Self::GrowDisk {
            expected_size,
            new_size,
            ..
        } = self
            && (*expected_size == 0
                || new_size <= expected_size
                || *new_size > i64::MAX as u64
                || new_size % 512 != 0)
        {
            return Err(Error::InvalidDeviceOperation);
        }
        Ok(())
    }

    /// Advisory eligibility for a current inventory view; dispatch always rechecks the host.
    pub fn availability(&self, inventory: &Inventory, vm: VmId) -> ActionAvailability {
        let Some(guest) = inventory.vms.get(&vm) else {
            return ActionAvailability::Stale;
        };
        if guest.is_template || guest.is_control_domain || self.validate().is_err() {
            return ActionAvailability::Unavailable;
        }
        let details = &inventory.details;
        if self.is_lifecycle() {
            return crate::device_lifecycle::availability(self, inventory, vm);
        }
        match self {
            Self::InsertCd { vbd, image } => {
                let Some(cd) = details.vbds.get(vbd) else {
                    return ActionAvailability::Unknown;
                };
                if cd.vm != Some(vm) {
                    return ActionAvailability::Stale;
                }
                let Some(disk) = details.vdis.get(image) else {
                    return ActionAvailability::Unknown;
                };
                let Some(sr) = disk.sr.and_then(|id| details.storage.get(&id)) else {
                    return ActionAvailability::Unknown;
                };
                if cd.empty.is_none() || disk.missing.is_none() || sr.content_type.is_empty() {
                    return ActionAvailability::Unknown;
                }
                if cd.kind != "CD"
                    || cd.empty != Some(true)
                    || sr.content_type != "iso"
                    || disk.missing != Some(false)
                {
                    return ActionAvailability::Unavailable;
                }
                advertised(cd.allowed_operations.as_deref(), "insert")
            }
            Self::EjectCd { vbd } | Self::PlugDisk { vbd } | Self::UnplugDisk { vbd } => {
                let Some(disk) = details.vbds.get(vbd) else {
                    return ActionAvailability::Unknown;
                };
                if disk.vm != Some(vm) {
                    return ActionAvailability::Stale;
                }
                if (matches!(self, Self::EjectCd { .. }) && disk.empty.is_none())
                    || (!matches!(self, Self::EjectCd { .. }) && disk.currently_attached.is_none())
                {
                    return ActionAvailability::Unknown;
                }
                let operation = match self {
                    Self::EjectCd { .. } if disk.kind == "CD" && disk.empty == Some(false) => {
                        "eject"
                    }
                    Self::PlugDisk { .. }
                        if disk.kind == "Disk"
                            && disk.currently_attached == Some(false)
                            && guest.power_state == PowerState::Running =>
                    {
                        "plug"
                    }
                    Self::UnplugDisk { .. }
                        if disk.kind == "Disk"
                            && disk.currently_attached == Some(true)
                            && guest.power_state == PowerState::Running =>
                    {
                        "unplug"
                    }
                    _ => return ActionAvailability::Unavailable,
                };
                advertised(disk.allowed_operations.as_deref(), operation)
            }
            Self::PlugNic { vif } | Self::UnplugNic { vif } => {
                let Some(nic) = details.vifs.get(vif) else {
                    return ActionAvailability::Unknown;
                };
                if nic.vm != Some(vm) {
                    return ActionAvailability::Stale;
                }
                if nic.currently_attached.is_none() {
                    return ActionAvailability::Unknown;
                }
                let plugging = matches!(self, Self::PlugNic { .. });
                if guest.power_state != PowerState::Running
                    || nic.currently_attached != Some(!plugging)
                {
                    return ActionAvailability::Unavailable;
                }
                advertised(
                    nic.allowed_operations.as_deref(),
                    if plugging { "plug" } else { "unplug" },
                )
            }
            Self::GrowDisk {
                vdi, expected_size, ..
            } => {
                let Some(disk) = details.vdis.get(vdi) else {
                    return ActionAvailability::Unknown;
                };
                if disk.virtual_size.is_none()
                    || [
                        disk.read_only,
                        disk.sharable,
                        disk.managed,
                        disk.missing,
                        disk.is_snapshot,
                    ]
                    .iter()
                    .any(Option::is_none)
                    || disk.kind.is_empty()
                {
                    return ActionAvailability::Unknown;
                }
                if guest.power_state != PowerState::Halted
                    || disk.virtual_size != Some(*expected_size)
                    || disk.read_only != Some(false)
                    || disk.sharable != Some(false)
                    || disk.managed != Some(true)
                    || disk.missing != Some(false)
                    || disk.is_snapshot != Some(false)
                    || disk.kind != "user"
                {
                    return ActionAvailability::Unavailable;
                }
                let Some(consumers) = details
                    .vdi_attachments
                    .get(vdi)
                    .filter(|ids| !ids.is_empty())
                else {
                    return ActionAvailability::Unknown;
                };
                if consumers.iter().any(|id| {
                    details.vbds.get(id).is_none_or(|vbd| {
                        vbd.vm != Some(vm)
                            || vbd.currently_attached != Some(false)
                            || vbd.kind != "Disk"
                    })
                }) {
                    return ActionAvailability::Unavailable;
                }
                advertised(disk.allowed_operations.as_deref(), "resize")
            }
            _ => ActionAvailability::Unavailable,
        }
    }

    pub(crate) fn is_lifecycle(&self) -> bool {
        matches!(
            self,
            Self::MountGuestTools { .. }
                | Self::Property { .. }
                | Self::CreateDisk { .. }
                | Self::AttachDisk { .. }
                | Self::DetachDisk { .. }
                | Self::DeleteDisk { .. }
                | Self::AddCdDrive { .. }
                | Self::AddNic { .. }
                | Self::DeleteNic { .. }
                | Self::EditNic { .. }
        )
    }
}

fn advertised(operations: Option<&[String]>, operation: &str) -> ActionAvailability {
    match operations {
        None => ActionAvailability::Unknown,
        Some(operations) if operations.iter().any(|candidate| candidate == operation) => {
            ActionAvailability::Available
        }
        Some(_) => ActionAvailability::Unavailable,
    }
}

pub(crate) struct PreparedDevice {
    method: &'static str,
    parameters: Vec<Value>,
}
impl PreparedDevice {
    pub(crate) async fn send(
        self,
        session: &Session,
        cancel: &CancellationToken,
    ) -> Result<Value, Error> {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(Error::Closed),
            result = session.call(self.method, self.parameters) => result,
        }
    }
}

fn reference(value: &Value) -> Result<String, Error> {
    value
        .as_str()
        .filter(|value| {
            value.starts_with("OpaqueRef:")
                && *value != "OpaqueRef:NULL"
                && value.len() > 10
                && value.len() <= 1024
                && value.bytes().all(|byte| byte.is_ascii_graphic())
        })
        .map(str::to_owned)
        .ok_or(Error::Protocol("invalid device reference"))
}
async fn fresh(session: &Session, class: &'static str, id: Uuid) -> Result<(String, Value), Error> {
    let object = session
        .call(&format!("{class}.get_by_uuid"), vec![json!(id.to_string())])
        .await?;
    let object = reference(&object)?;
    let record = session
        .call(&format!("{class}.get_record"), vec![json!(&object)])
        .await?;
    if record["uuid"].as_str() != Some(id.to_string().as_str()) {
        return Err(Error::Stale);
    }
    Ok((object, record))
}
fn permits(record: &Value, operation: &str) -> Result<(), Error> {
    let Some(operations) = record["allowed_operations"].as_array() else {
        return Err(Error::UnavailableAction);
    };
    if operations.iter().any(|entry| !entry.is_string()) {
        return Err(Error::UnavailableAction);
    }
    if !operations.iter().any(|entry| entry == operation) {
        // The record is well formed and the host simply does not offer this operation
        // now (a running tool-less guest never advertises NIC plug or unplug). Name that,
        // instead of the generic unavailable-action text.
        return Err(Error::DeviceNotAdvertised);
    }
    if let Some(current) = record.get("current_operations")
        && current
            .as_object()
            .is_none_or(|current| !current.is_empty())
    {
        return Err(Error::Busy);
    }
    Ok(())
}
/// Changed-block tracking can be disabled when it is enabled on an idle disk. XCP-ng 8.3.0
/// never lists `disable_cbt` among a disk's allowed operations, even with tracking enabled,
/// and offers no `VDI.update_allowed_operations`, so the tracking state and the blocked
/// operations decide; the host still refuses a disable it does not accept.
pub(crate) fn permits_cbt_disable(record: &Value) -> Result<(), Error> {
    if record["cbt_enabled"] != true
        || record["blocked_operations"]
            .as_object()
            .is_some_and(|blocked| blocked.contains_key("disable_cbt"))
    {
        return Err(Error::UnavailableAction);
    }
    if let Some(current) = record.get("current_operations")
        && current
            .as_object()
            .is_none_or(|current| !current.is_empty())
    {
        return Err(Error::Busy);
    }
    Ok(())
}
fn owns(record: &Value, vm_reference: &str) -> Result<(), Error> {
    if record["VM"] != vm_reference {
        return Err(Error::Stale);
    }
    Ok(())
}
fn unsigned(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| value.as_str()?.parse().ok())
}

pub(crate) async fn prepare(
    session: &Session,
    record: &OperationRecord,
) -> Result<PreparedDevice, Error> {
    let OperationAction::Device(operation) = &record.action else {
        return Err(Error::InvalidDeviceOperation);
    };
    prepare_operation(session, record.vm, operation).await
}

async fn prepare_operation(
    session: &Session,
    vm: VmId,
    operation: &DeviceOperation,
) -> Result<PreparedDevice, Error> {
    operation.validate()?;
    let (vm_reference, guest) = fresh(session, "VM", vm.uuid()).await?;
    if VmRecord::new(&guest).is_a_template() != false
        || VmRecord::new(&guest).is_a_snapshot() != false
        || VmRecord::new(&guest).is_control_domain() != false
    {
        return Err(Error::Stale);
    }
    if !matches!(
        VmRecord::new(&guest).power_state().as_str(),
        Some("Halted" | "Running" | "Paused" | "Suspended")
    ) {
        return Err(Error::UnavailableAction);
    }
    let result = match operation {
        DeviceOperation::InsertCd { vbd, image } => {
            let (cd_reference, cd) = fresh(session, "VBD", vbd.uuid()).await?;
            owns(&cd, &vm_reference)?;
            if VbdRecord::new(&cd).type_() != "CD" || VbdRecord::new(&cd).empty() != true {
                return Err(Error::DeviceConflict);
            }
            permits(&cd, "insert")?;
            let (image_reference, disk) = fresh(session, "VDI", image.uuid()).await?;
            if VdiRecord::new(&disk).missing() != false {
                return Err(Error::DeviceInUse);
            }
            let sr_reference = reference(&VdiRecord::new(&disk).sr())?;
            let sr = session
                .call("SR.get_record", vec![json!(sr_reference)])
                .await?;
            if SrRecord::new(&sr).content_type() != "iso" {
                return Err(Error::DeviceInUse);
            }
            PreparedDevice {
                method: "Async.VBD.insert",
                parameters: vec![json!(cd_reference), json!(image_reference)],
            }
        }
        DeviceOperation::EjectCd { vbd } => {
            let (cd_reference, cd) = fresh(session, "VBD", vbd.uuid()).await?;
            owns(&cd, &vm_reference)?;
            if VbdRecord::new(&cd).type_() != "CD" || VbdRecord::new(&cd).empty() != false {
                return Err(Error::DeviceConflict);
            }
            permits(&cd, "eject")?;
            PreparedDevice {
                method: "Async.VBD.eject",
                parameters: vec![json!(cd_reference)],
            }
        }
        DeviceOperation::PlugDisk { vbd } | DeviceOperation::UnplugDisk { vbd } => {
            let (device_reference, device) = fresh(session, "VBD", vbd.uuid()).await?;
            owns(&device, &vm_reference)?;
            let plugging = matches!(operation, DeviceOperation::PlugDisk { .. });
            if device["type"] != "Disk"
                || device["currently_attached"] != !plugging
                || VmRecord::new(&guest).power_state() != "Running"
            {
                return Err(Error::DeviceConflict);
            }
            permits(&device, if plugging { "plug" } else { "unplug" })?;
            PreparedDevice {
                method: if plugging {
                    "Async.VBD.plug"
                } else {
                    "Async.VBD.unplug"
                },
                parameters: vec![json!(device_reference)],
            }
        }
        DeviceOperation::PlugNic { vif } | DeviceOperation::UnplugNic { vif } => {
            let (device_reference, device) = fresh(session, "VIF", vif.uuid()).await?;
            owns(&device, &vm_reference)?;
            let plugging = matches!(operation, DeviceOperation::PlugNic { .. });
            if device["currently_attached"] != !plugging
                || VmRecord::new(&guest).power_state() != "Running"
            {
                return Err(Error::DeviceConflict);
            }
            permits(&device, if plugging { "plug" } else { "unplug" })?;
            PreparedDevice {
                method: if plugging {
                    "Async.VIF.plug"
                } else {
                    "Async.VIF.unplug"
                },
                parameters: vec![json!(device_reference)],
            }
        }
        DeviceOperation::GrowDisk {
            vdi,
            expected_size,
            new_size,
        } => {
            if VmRecord::new(&guest).power_state() != "Halted" {
                return Err(Error::DeviceRequiresHaltedVm);
            }
            let (disk_reference, disk) = fresh(session, "VDI", vdi.uuid()).await?;
            if unsigned(&VdiRecord::new(&disk).virtual_size()) != Some(*expected_size) {
                return Err(Error::DeviceConflict);
            }
            if VdiRecord::new(&disk).read_only() != false
                || VdiRecord::new(&disk).sharable() != false
                || VdiRecord::new(&disk).managed() != true
                || VdiRecord::new(&disk).missing() != false
                || VdiRecord::new(&disk).is_a_snapshot() != false
                || VdiRecord::new(&disk).type_() != "user"
            {
                return Err(Error::DeviceInUse);
            }
            permits(&disk, "resize")?;
            let consumers = VdiRecord::new(&disk)
                .vbds()
                .value()
                .as_array()
                .filter(|consumers| !consumers.is_empty() && consumers.len() <= 256)
                .ok_or(Error::DeviceInUse)?;
            for consumer in consumers {
                let consumer_reference = reference(consumer)?;
                let attachment = session
                    .call("VBD.get_record", vec![json!(consumer_reference)])
                    .await?;
                if VbdRecord::new(&attachment).vdi() != disk_reference
                    || VbdRecord::new(&attachment).vm() != vm_reference
                    || VbdRecord::new(&attachment).currently_attached() != false
                    || VbdRecord::new(&attachment).type_() != "Disk"
                {
                    return Err(Error::DeviceInUse);
                }
            }
            PreparedDevice {
                method: "Async.VDI.resize",
                parameters: vec![json!(disk_reference), json!(new_size.to_string())],
            }
        }
        _ => return Err(Error::InvalidDeviceOperation),
    };
    Ok(result)
}

#[cfg(test)]
#[path = "devices_tests.rs"]
mod tests;
