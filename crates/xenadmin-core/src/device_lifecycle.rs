#[path = "device_property_operations.rs"]
mod properties;
use crate::{
    ActionAvailability, ClassAvailability, DeviceOperation, Error, Inventory, NetworkId,
    OperationId, PowerState, SrId, VbdId, VdiId, VifId, VmId, xapi::Session,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;
use xenadmin_xapi::records::{NetworkRecord, SrRecord, VbdRecord, VdiRecord, VifRecord, VmRecord};

const MARKER: &str = "xenadmin-rs-operation";
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CreatedDevice {
    Disk(VdiId),
    Attachment(VbdId),
    Nic(VifId),
}
impl CreatedDevice {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Disk(_) => "Disk",
            Self::Attachment(_) => "Disk attachment",
            Self::Nic(_) => "Network interface",
        }
    }
    pub fn uuid(&self) -> Uuid {
        match self {
            Self::Disk(id) => id.uuid(),
            Self::Attachment(id) => id.uuid(),
            Self::Nic(id) => id.uuid(),
        }
    }
}
fn slot(value: &str) -> bool {
    value.parse::<u8>().is_ok_and(|n| n.to_string() == value)
}
fn mac(value: &str) -> bool {
    let bytes = value
        .split(':')
        .map(|s| {
            if s.len() == 2 {
                u8::from_str_radix(s, 16).ok()
            } else {
                None
            }
        })
        .collect::<Option<Vec<_>>>();
    bytes
        .is_some_and(|bytes| bytes.len() == 6 && bytes[0] & 1 == 0 && bytes.iter().any(|b| *b != 0))
}
pub(crate) fn validate(operation: &DeviceOperation) -> Result<(), Error> {
    use DeviceOperation::*;
    let valid = match operation {
        Property { change } => return change.validate(),
        CreateDisk {
            name, size, device, ..
        } => {
            !name.trim().is_empty()
                && name.chars().count() <= 255
                && !name.chars().any(char::is_control)
                && *size > 0
                && *size <= i64::MAX as u64
                && size % 512 == 0
                && slot(device)
        }
        AttachDisk { device, .. } | AddCdDrive { device } => slot(device),
        AddNic {
            device,
            mac: address,
            ..
        } => slot(device) && address.as_deref().is_none_or(mac),
        EditNic {
            mac: address,
            expected_mac,
            ..
        } => mac(address) && mac(expected_mac),
        _ => true,
    };
    if valid {
        Ok(())
    } else {
        Err(Error::InvalidDeviceConfiguration)
    }
}
pub(crate) fn availability(
    operation: &DeviceOperation,
    inventory: &Inventory,
    vm: VmId,
) -> ActionAvailability {
    use DeviceOperation::*;
    let Some(guest) = inventory.vms.get(&vm) else {
        return ActionAvailability::Stale;
    };
    if let MountGuestTools { .. } = operation {
        return crate::guest_tools::availability(operation, inventory, vm);
    }
    if let Property { change } = operation {
        return properties::availability(change, inventory, vm);
    }
    if guest.power_state != PowerState::Halted {
        return ActionAvailability::Prerequisite {
            reason: "Shut down the VM before changing its device configuration".into(),
        };
    }
    let details = &inventory.details;
    let required = match operation {
        AddNic { .. } | DeleteNic { .. } | EditNic { .. } => &["vif", "network"][..],
        _ => &["vbd", "vdi", "sr"][..],
    };
    if required
        .iter()
        .any(|class| details.classes.get(*class) != Some(&ClassAvailability::Available))
    {
        return ActionAvailability::Unknown;
    }
    if let DeleteDisk { vbd, vdi } = operation {
        if details.vbds.get(vbd).is_some_and(|vbd| !vbd.owned) {
            return ActionAvailability::Prerequisite {
                reason: "This attachment does not own the disk. Detach it to preserve the data."
                    .into(),
            };
        }
        if details
            .vdis
            .get(vdi)
            .is_some_and(|vdi| vdi.sharable == Some(true))
            || details
                .vdi_attachments
                .get(vdi)
                .is_some_and(|ids| ids.iter().any(|id| id != vbd))
        {
            return ActionAvailability::Prerequisite{reason:"This disk is shared or has other attachments. Detach this attachment to preserve their data.".into()};
        }
    }
    let disk_valid = |id: &VdiId| {
        details.vdis.get(id).is_some_and(|disk| {
            disk.kind == "user"
                && disk.sharable == Some(false)
                && disk.managed == Some(true)
                && disk.missing == Some(false)
                && disk.is_snapshot == Some(false)
        })
    };
    let network_valid = |id: &NetworkId| details.networks.contains_key(id);
    let available = match operation {
        CreateDisk { sr, .. } => details.storage.get(sr).is_some_and(|sr| {
            sr.content_type != "iso"
                && sr
                    .allowed_operations
                    .as_ref()
                    .is_some_and(|ops| ops.iter().any(|op| op == "vdi_create"))
        }),
        AttachDisk { vdi, read_only, .. } => {
            disk_valid(vdi)
                && details.vdi_attachments.get(vdi).is_none_or(Vec::is_empty)
                && details
                    .vdis
                    .get(vdi)
                    .is_some_and(|disk| *read_only || disk.read_only == Some(false))
        }
        DetachDisk { vbd, vdi } | DeleteDisk { vbd, vdi } => {
            details.vbds.get(vbd).is_some_and(|attachment| {
                attachment.vm == Some(vm)
                    && attachment.vdi == Some(*vdi)
                    && attachment.kind == "Disk"
                    && attachment.currently_attached == Some(false)
                    && (!matches!(operation, DeleteDisk { .. })
                        || (attachment.owned
                            && disk_valid(vdi)
                            && details
                                .vdi_attachments
                                .get(vdi)
                                .is_some_and(|ids| ids == &vec![*vbd])
                            && details.vdis[vdi]
                                .allowed_operations
                                .as_ref()
                                .is_some_and(|ops| ops.iter().any(|op| op == "destroy"))))
            })
        }
        AddCdDrive { .. } => true,
        AddNic { network, .. } => network_valid(network),
        DeleteNic { vif } | EditNic { vif, .. } => details.vifs.get(vif).is_some_and(|nic| {
            nic.vm == Some(vm)
                && nic.currently_attached == Some(false)
                && match operation {
                    EditNic {
                        expected_network,
                        expected_mac,
                        network,
                        ..
                    } => {
                        nic.network == Some(*expected_network)
                            && nic.mac.eq_ignore_ascii_case(expected_mac)
                            && network_valid(network)
                    }
                    _ => true,
                }
        }),
        _ => false,
    };
    if available {
        ActionAvailability::Available
    } else {
        ActionAvailability::Unavailable
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct NicSettings {
    mtu: u64,
    qos_algorithm: String,
    qos_parameters: BTreeMap<String, String>,
    locking_mode: String,
    ipv4_allowed: Vec<String>,
    ipv6_allowed: Vec<String>,
}
impl NicSettings {
    fn new(mtu: u64) -> Self {
        Self {
            mtu,
            qos_algorithm: String::new(),
            qos_parameters: BTreeMap::new(),
            locking_mode: "network_default".into(),
            ipv4_allowed: Vec::new(),
            ipv6_allowed: Vec::new(),
        }
    }
    fn from_record(record: &Value) -> Result<Self, Error> {
        if VifRecord::new(record)
            .other_config()
            .as_object()
            .is_none_or(|map| map.keys().any(|k| k != MARKER))
            || ["ipv4_configuration_mode", "ipv6_configuration_mode"]
                .iter()
                .any(|field| record[*field].as_str().is_some_and(|v| v != "None"))
        {
            return Err(Error::UnsupportedNicConfiguration);
        }
        let mtu = number(&VifRecord::new(record).mtu())
            .filter(|n| *n >= 576 && *n <= 65535)
            .ok_or(Error::UnsupportedNicConfiguration)?;
        let qos_algorithm = VifRecord::new(record)
            .qos_algorithm_type()
            .as_str()
            .ok_or(Error::UnsupportedNicConfiguration)?
            .to_owned();
        if !matches!(qos_algorithm.as_str(), "" | "ratelimit") {
            return Err(Error::UnsupportedNicConfiguration);
        }
        let qos_parameters = VifRecord::new(record)
            .qos_algorithm_params()
            .as_object()
            .ok_or(Error::UnsupportedNicConfiguration)?
            .iter()
            .map(|(k, v)| {
                if k != "kbps" || v.as_str().is_none_or(|s| s.parse::<u64>().is_err()) {
                    return Err(Error::UnsupportedNicConfiguration);
                }
                Ok((
                    k.clone(),
                    v.as_str().unwrap().parse::<u64>().unwrap().to_string(),
                ))
            })
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        let locking_mode = VifRecord::new(record)
            .locking_mode()
            .as_str()
            .unwrap_or("network_default")
            .to_owned();
        if !matches!(
            locking_mode.as_str(),
            "network_default" | "locked" | "unlocked" | "disabled"
        ) {
            return Err(Error::UnsupportedNicConfiguration);
        }
        let addresses = |key: &str, version: bool| -> Result<Vec<String>, Error> {
            record[key].as_array().map_or(Ok(Vec::new()), |values| {
                if values.len() > 256 {
                    return Err(Error::UnsupportedNicConfiguration);
                }
                let addresses = values
                    .iter()
                    .map(|v| {
                        v.as_str()
                            .filter(|s| {
                                s.parse::<std::net::IpAddr>()
                                    .is_ok_and(|ip| ip.is_ipv4() == version)
                            })
                            .map(|s| s.parse::<std::net::IpAddr>().unwrap().to_string())
                            .ok_or(Error::UnsupportedNicConfiguration)
                    })
                    .collect::<Result<BTreeSet<_>, _>>()?;
                Ok(addresses.into_iter().collect())
            })
        };
        Ok(Self {
            mtu,
            qos_algorithm,
            qos_parameters,
            locking_mode,
            ipv4_allowed: addresses("ipv4_allowed", true)?,
            ipv6_allowed: addresses("ipv6_allowed", false)?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Step {
    GuestTools(crate::guest_tools::Step),
    Property(properties::PropertyStep),
    CreateDisk {
        sr: SrId,
        name: String,
        size: u64,
    },
    Attach {
        disk: Option<VdiId>,
        device: String,
        read_only: bool,
        owned: bool,
        cd: bool,
    },
    DestroyAttachment {
        vbd: VbdId,
        disk: VdiId,
        delete_disk: bool,
    },
    DestroyDisk {
        disk: VdiId,
    },
    CreateNic {
        network: NetworkId,
        device: String,
        mac: Option<String>,
        settings: NicSettings,
    },
    DestroyNic {
        vif: VifId,
        network: NetworkId,
        mac: String,
        settings: Option<NicSettings>,
    },
    MoveNic {
        vif: VifId,
        from: NetworkId,
        to: NetworkId,
        mac: String,
    },
}
impl Step {
    pub fn label(&self) -> &'static str {
        match self {
            Self::GuestTools(step) => step.label(),
            Self::Property(step) => step.change.label(),
            Self::CreateDisk { .. } => "Create disk",
            Self::Attach { cd: true, .. } => "Create CD drive",
            Self::Attach { .. } => "Create disk attachment",
            Self::DestroyAttachment { .. } => "Remove disk attachment",
            Self::DestroyDisk { .. } => "Delete disk data",
            Self::CreateNic { .. } => "Create network interface",
            Self::DestroyNic { .. } => "Remove network interface",
            Self::MoveNic { .. } => "Change interface network",
        }
    }
    pub fn creation_class(&self) -> Option<&'static str> {
        match self {
            Self::CreateDisk { .. } => Some("VDI"),
            Self::Attach { .. } => Some("VBD"),
            Self::CreateNic { .. } => Some("VIF"),
            _ => None,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub(crate) struct Evidence {
    pub baseline: Vec<Uuid>,
    pub result_reference: Option<String>,
    pub created: Option<CreatedDevice>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Plan {
    pub steps: Vec<Step>,
    pub evidence: Vec<Option<Evidence>>,
}
fn number(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| value.as_str()?.parse().ok())
}
fn reference(value: &Value) -> Result<String, Error> {
    crate::snapshots::reference(value)
}
async fn fresh(session: &Session, class: &str, id: Uuid) -> Result<(String, Value), Error> {
    crate::snapshots::optional_record(session, class, &id.to_string())
        .await?
        .ok_or(Error::DeviceConflict)
}
fn idle(record: &Value) -> Result<(), Error> {
    if record["current_operations"]
        .as_object()
        .is_none_or(|map| !map.is_empty())
    {
        return Err(Error::Busy);
    }
    Ok(())
}
fn permits(record: &Value, operation: &str) -> Result<(), Error> {
    idle(record)?;
    if VmRecord::new(record)
        .allowed_operations()
        .as_array()
        .is_none_or(|ops| !ops.iter().any(|op| op == operation))
        || VmRecord::new(record)
            .blocked_operations()
            .get(operation)
            .is_some()
    {
        return Err(Error::UnavailableAction);
    }
    Ok(())
}
async fn guest(session: &Session, vm: VmId, halted: bool) -> Result<String, Error> {
    let (reference, record) = fresh(session, "VM", vm.uuid()).await?;
    if VmRecord::new(&record).is_a_snapshot() != false
        || VmRecord::new(&record).is_a_template() != false
        || VmRecord::new(&record).is_control_domain() != false
    {
        return Err(Error::Stale);
    }
    if halted {
        if VmRecord::new(&record).power_state() != "Halted" {
            return Err(Error::DeviceLifecycleRequiresHaltedVm);
        }
        idle(&record)?;
    }
    Ok(reference)
}
async fn network(session: &Session, id: NetworkId) -> Result<(String, u64), Error> {
    let (reference, record) = fresh(session, "network", id.uuid()).await?;
    let mtu = number(&record["MTU"])
        .filter(|n| *n >= 576 && *n <= 65535)
        .ok_or(Error::DeviceConflict)?;
    Ok((reference, mtu))
}
async fn allowed_slot(session: &Session, vm: &str, class: &str, device: &str) -> Result<(), Error> {
    let slots = session
        .call(&format!("VM.get_allowed_{class}_devices"), vec![json!(vm)])
        .await?;
    if !slots
        .as_array()
        .is_some_and(|slots| slots.len() <= 256 && slots.iter().any(|slot| slot == device))
    {
        return Err(Error::DeviceConflict);
    }
    Ok(())
}
fn ordinary_disk(record: &Value) -> bool {
    VdiRecord::new(record).type_() == "user"
        && VdiRecord::new(record).managed() == true
        && VdiRecord::new(record).missing() == false
        && VdiRecord::new(record).sharable() == false
        && VdiRecord::new(record).is_a_snapshot() == false
}
fn refs(record: &Value, key: &str) -> Result<Vec<String>, Error> {
    record[key]
        .as_array()
        .filter(|items| items.len() <= 256)
        .ok_or(Error::DeviceConflict)?
        .iter()
        .map(reference)
        .collect()
}
async fn attachment(
    session: &Session,
    vm: &str,
    vbd: VbdId,
    disk: VdiId,
    deleting: bool,
) -> Result<(String, Value), Error> {
    let (reference, record) = fresh(session, "VBD", vbd.uuid()).await?;
    let (disk_ref, disk_record) = fresh(session, "VDI", disk.uuid()).await?;
    if VbdRecord::new(&record).vm() != vm
        || VbdRecord::new(&record).vdi() != disk_ref
        || VbdRecord::new(&record).type_() != "Disk"
        || VbdRecord::new(&record).currently_attached() != false
    {
        return Err(Error::DeviceConflict);
    }
    idle(&record)?;
    if deleting {
        if !VbdRecord::new(&record)
            .other_config()
            .get("owner")
            .is_some_and(Value::is_string)
            || !ordinary_disk(&disk_record)
            || refs(&disk_record, "VBDs")? != vec![reference.clone()]
        {
            return Err(Error::DeviceInUse);
        }
        permits(&disk_record, "destroy")?;
    }
    Ok((reference, record))
}
async fn nic(session: &Session, vm: &str, id: VifId) -> Result<(String, Value), Error> {
    let (reference, record) = fresh(session, "VIF", id.uuid()).await?;
    if record["VM"] != vm || record["currently_attached"] != false {
        return Err(Error::DeviceConflict);
    }
    idle(&record)?;
    Ok((reference, record))
}
pub(crate) async fn plan(
    session: &Session,
    vm: VmId,
    operation: &DeviceOperation,
) -> Result<Plan, Error> {
    use DeviceOperation::*;
    validate(operation)?;
    if let MountGuestTools { .. } = operation {
        return crate::guest_tools::plan(session, vm, operation).await;
    }
    if let Property { change } = operation {
        return properties::plan(session, vm, change).await;
    }
    let vm_ref = guest(session, vm, true).await?;
    let steps = match operation {
        CreateDisk {
            sr,
            name,
            size,
            device,
        } => {
            let (_, record) = fresh(session, "SR", sr.uuid()).await?;
            if SrRecord::new(&record).content_type() == "iso" {
                return Err(Error::DeviceConflict);
            }
            permits(&record, "vdi_create")?;
            allowed_slot(session, &vm_ref, "VBD", device).await?;
            vec![
                Step::CreateDisk {
                    sr: *sr,
                    name: name.clone(),
                    size: *size,
                },
                Step::Attach {
                    disk: None,
                    device: device.clone(),
                    read_only: false,
                    owned: true,
                    cd: false,
                },
            ]
        }
        AttachDisk {
            vdi,
            device,
            read_only,
        } => {
            let (_, disk) = fresh(session, "VDI", vdi.uuid()).await?;
            if !ordinary_disk(&disk)
                || !refs(&disk, "VBDs")?.is_empty()
                || (!*read_only && VdiRecord::new(&disk).read_only() != false)
            {
                return Err(Error::DeviceInUse);
            }
            allowed_slot(session, &vm_ref, "VBD", device).await?;
            vec![Step::Attach {
                disk: Some(*vdi),
                device: device.clone(),
                read_only: *read_only,
                owned: false,
                cd: false,
            }]
        }
        DetachDisk { vbd, vdi } | DeleteDisk { vbd, vdi } => {
            let deleting = matches!(operation, DeleteDisk { .. });
            attachment(session, &vm_ref, *vbd, *vdi, deleting).await?;
            let mut steps = vec![Step::DestroyAttachment {
                vbd: *vbd,
                disk: *vdi,
                delete_disk: deleting,
            }];
            if deleting {
                steps.push(Step::DestroyDisk { disk: *vdi });
            }
            steps
        }
        AddCdDrive { device } => {
            allowed_slot(session, &vm_ref, "VBD", device).await?;
            vec![Step::Attach {
                disk: None,
                device: device.clone(),
                read_only: true,
                owned: false,
                cd: true,
            }]
        }
        AddNic {
            network: id,
            device,
            mac,
        } => {
            let (_, mtu) = network(session, *id).await?;
            allowed_slot(session, &vm_ref, "VIF", device).await?;
            vec![Step::CreateNic {
                network: *id,
                device: device.clone(),
                mac: mac.clone(),
                settings: NicSettings::new(mtu),
            }]
        }
        DeleteNic { vif } | EditNic { vif, .. } => {
            let (_, record) = nic(session, &vm_ref, *vif).await?;
            let network_ref = reference(&record["network"])?;
            let network_record = session
                .call("network.get_record", vec![json!(network_ref)])
                .await?;
            let from = NetworkRecord::new(&network_record)
                .uuid()
                .as_str()
                .and_then(|id| NetworkId::parse(id).ok())
                .ok_or(Error::DeviceConflict)?;
            let address = record["MAC"]
                .as_str()
                .filter(|s| mac(s))
                .ok_or(Error::DeviceConflict)?
                .to_owned();
            if let EditNic {
                expected_network,
                expected_mac,
                network: to,
                mac: new_mac,
                ..
            } = operation
            {
                if from != *expected_network || !address.eq_ignore_ascii_case(expected_mac) {
                    return Err(Error::DeviceConflict);
                }
                network(session, *to).await?;
                if address.eq_ignore_ascii_case(new_mac) {
                    if from == *to {
                        return Err(Error::InvalidDeviceConfiguration);
                    }
                    vec![Step::MoveNic {
                        vif: *vif,
                        from,
                        to: *to,
                        mac: address,
                    }]
                } else {
                    let settings = NicSettings::from_record(&record)?;
                    let device = record["device"]
                        .as_str()
                        .filter(|s| slot(s))
                        .ok_or(Error::DeviceConflict)?
                        .to_owned();
                    vec![
                        Step::DestroyNic {
                            vif: *vif,
                            network: from,
                            mac: address,
                            settings: Some(settings.clone()),
                        },
                        Step::CreateNic {
                            network: *to,
                            device,
                            mac: Some(new_mac.clone()),
                            settings,
                        },
                    ]
                }
            } else {
                vec![Step::DestroyNic {
                    vif: *vif,
                    network: from,
                    mac: address,
                    settings: None,
                }]
            }
        }
        _ => return Err(Error::InvalidDeviceOperation),
    };
    Ok(Plan {
        evidence: vec![None; steps.len()],
        steps,
    })
}

pub(crate) async fn baseline(session: &Session, class: &str) -> Result<Vec<Uuid>, Error> {
    let records = session
        .call(&format!("{class}.get_all_records"), vec![])
        .await?;
    let records = records
        .as_object()
        .filter(|values| values.len() <= 10_000)
        .ok_or(Error::Protocol(
            "device identity baseline exceeds 10000 records or is invalid",
        ))?;
    let ids = records
        .values()
        .map(|record| {
            record["uuid"]
                .as_str()
                .and_then(|id| Uuid::parse_str(id).ok())
                .ok_or(Error::DeviceConflict)
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    if ids.len() != records.len() {
        return Err(Error::DeviceConflict);
    }
    Ok(ids.into_iter().collect())
}

fn created_disk(plan: &Plan) -> Result<VdiId, Error> {
    plan.evidence
        .iter()
        .flatten()
        .find_map(|e| match e.created {
            Some(CreatedDevice::Disk(id)) => Some(id),
            _ => None,
        })
        .ok_or(Error::DeviceConflict)
}
fn other_config(id: OperationId, owned: bool) -> Value {
    if owned {
        json!({MARKER:id.to_string(),"owner":"true"})
    } else {
        json!({MARKER:id.to_string()})
    }
}

pub(crate) struct Prepared {
    pub(crate) method: &'static str,
    pub(crate) parameters: Vec<Value>,
}
impl Prepared {
    pub async fn send(self, session: &Session) -> Result<Value, Error> {
        session.call(self.method, self.parameters).await
    }
}

pub(crate) async fn prepare_step(
    session: &Session,
    vm: VmId,
    id: OperationId,
    plan: &Plan,
    index: usize,
) -> Result<Prepared, Error> {
    if let Step::GuestTools(step) = &plan.steps[index] {
        return crate::guest_tools::prepare(session, vm, step).await;
    }
    if let Step::Property(step) = &plan.steps[index] {
        return properties::prepare(session, vm, step).await;
    }
    let vm_ref = guest(session, vm, true).await?;
    let prepared = match &plan.steps[index] {
        Step::GuestTools(_) | Step::Property(_) => unreachable!(),
        Step::CreateDisk { sr, name, size } => {
            let (sr_ref, record) = fresh(session, "SR", sr.uuid()).await?;
            if SrRecord::new(&record).content_type() == "iso" {
                return Err(Error::DeviceConflict);
            }
            permits(&record, "vdi_create")?;
            Prepared {
                method: "VDI.create",
                parameters: vec![
                    json!({"name_label":name,"name_description":"","SR":sr_ref,"virtual_size":size.to_string(),"type":"user","sharable":false,"read_only":false,"other_config":other_config(id,false),"xenstore_data":{},"sm_config":{},"tags":[]}),
                ],
            }
        }
        Step::Attach {
            disk,
            device,
            read_only,
            owned,
            cd,
        } => {
            allowed_slot(session, &vm_ref, "VBD", device).await?;
            let disk_ref = if *cd {
                "OpaqueRef:NULL".into()
            } else {
                let id = match disk {
                    Some(id) => *id,
                    None => created_disk(plan)?,
                };
                let (reference, record) = fresh(session, "VDI", id.uuid()).await?;
                if !ordinary_disk(&record)
                    || !refs(&record, "VBDs")?.is_empty()
                    || (!*read_only && VdiRecord::new(&record).read_only() != false)
                {
                    return Err(Error::DeviceInUse);
                }
                idle(&record)?;
                reference
            };
            Prepared {
                method: "VBD.create",
                parameters: vec![
                    json!({"VM":vm_ref,"VDI":disk_ref,"userdevice":device,"bootable":false,"mode":if *read_only{"RO"}else{"RW"},"type":if *cd{"CD"}else{"Disk"},"unpluggable":true,"empty":cd,"other_config":other_config(id,*owned),"qos_algorithm_type":"","qos_algorithm_params":{}}),
                ],
            }
        }
        Step::DestroyAttachment {
            vbd,
            disk,
            delete_disk,
        } => {
            let (reference, _) = attachment(session, &vm_ref, *vbd, *disk, *delete_disk).await?;
            Prepared {
                method: "VBD.destroy",
                parameters: vec![json!(reference)],
            }
        }
        Step::DestroyDisk { disk } => {
            let (reference, record) = fresh(session, "VDI", disk.uuid()).await?;
            if !ordinary_disk(&record) || !refs(&record, "VBDs")?.is_empty() {
                return Err(Error::DeviceInUse);
            }
            permits(&record, "destroy")?;
            let vms = session.call("VM.get_all_records", vec![]).await?;
            if vms
                .as_object()
                .ok_or(Error::DeviceConflict)?
                .values()
                .any(|record| VmRecord::new(record).suspend_vdi() == reference)
            {
                return Err(Error::DeviceInUse);
            }
            Prepared {
                method: "VDI.destroy",
                parameters: vec![json!(reference)],
            }
        }
        Step::CreateNic {
            network: id_network,
            device,
            mac,
            settings,
        } => {
            let (network_ref, _) = network(session, *id_network).await?;
            allowed_slot(session, &vm_ref, "VIF", device).await?;
            Prepared {
                method: "VIF.create",
                parameters: vec![
                    json!({"VM":vm_ref,"network":network_ref,"device":device,"MAC":mac.as_deref().unwrap_or(""),"MTU":settings.mtu.to_string(),"other_config":other_config(id,false),"qos_algorithm_type":settings.qos_algorithm,"qos_algorithm_params":settings.qos_parameters,"locking_mode":settings.locking_mode,"ipv4_allowed":settings.ipv4_allowed,"ipv6_allowed":settings.ipv6_allowed}),
                ],
            }
        }
        Step::DestroyNic {
            vif,
            network: id_network,
            mac,
            settings,
        } => {
            let (reference, record) = nic(session, &vm_ref, *vif).await?;
            let (network_ref, _) = network(session, *id_network).await?;
            if record["network"] != network_ref
                || !record["MAC"]
                    .as_str()
                    .is_some_and(|m| m.eq_ignore_ascii_case(mac))
                || settings.as_ref().is_some_and(|s| {
                    !NicSettings::from_record(&record).is_ok_and(|actual| actual == *s)
                })
            {
                return Err(Error::DeviceConflict);
            }
            Prepared {
                method: "VIF.destroy",
                parameters: vec![json!(reference)],
            }
        }
        Step::MoveNic { vif, from, to, mac } => {
            let (reference, record) = nic(session, &vm_ref, *vif).await?;
            let (source, _) = network(session, *from).await?;
            let (target, _) = network(session, *to).await?;
            if record["network"] != source
                || !record["MAC"]
                    .as_str()
                    .is_some_and(|m| m.eq_ignore_ascii_case(mac))
            {
                return Err(Error::DeviceConflict);
            }
            Prepared {
                method: "VIF.move",
                parameters: vec![json!(reference), json!(target)],
            }
        }
    };
    Ok(prepared)
}

async fn creation_matches(
    session: &Session,
    vm: VmId,
    id: OperationId,
    plan: &Plan,
    index: usize,
    record: &Value,
) -> Result<bool, Error> {
    let vm_ref = guest(session, vm, false).await?;
    idle(record)?;
    if record["other_config"][MARKER] != id.to_string() {
        return Ok(false);
    }
    Ok(match &plan.steps[index] {
        Step::CreateDisk { sr, name, size } => {
            let (sr_ref, sr_record) = fresh(session, "SR", sr.uuid()).await?;
            idle(&sr_record)?;
            ordinary_disk(record)
                && record["SR"] == sr_ref
                && record["name_label"] == *name
                && number(&record["virtual_size"]).is_some_and(|actual| actual >= *size)
                && record["read_only"] == false
        }
        Step::Attach {
            disk,
            device,
            read_only,
            owned,
            cd,
        } => {
            let disk_ref = if *cd {
                "OpaqueRef:NULL".into()
            } else {
                fresh(
                    session,
                    "VDI",
                    match disk {
                        Some(id) => *id,
                        None => created_disk(plan)?,
                    }
                    .uuid(),
                )
                .await?
                .0
            };
            record["VM"] == vm_ref
                && record["VDI"] == disk_ref
                && record["userdevice"] == *device
                && record["type"] == if *cd { "CD" } else { "Disk" }
                && record["mode"] == if *read_only { "RO" } else { "RW" }
                && record["empty"] == *cd
                && (record["other_config"]
                    .get("owner")
                    .is_some_and(Value::is_string)
                    == *owned)
        }
        Step::CreateNic {
            network: id_network,
            device,
            mac: address,
            settings,
        } => {
            let (network_ref, _) = network(session, *id_network).await?;
            record["VM"] == vm_ref
                && record["network"] == network_ref
                && record["device"] == *device
                && record["MAC"].as_str().is_some_and(|m| {
                    mac(m)
                        && address
                            .as_ref()
                            .is_none_or(|expected| m.eq_ignore_ascii_case(expected))
                })
                && number(&record["MTU"]) == Some(settings.mtu)
                && NicSettings::from_record(record).is_ok_and(|actual| actual == *settings)
        }
        _ => false,
    })
}

pub(crate) async fn verify_step(
    session: &Session,
    vm: VmId,
    id: OperationId,
    plan: &mut Plan,
    index: usize,
) -> Result<bool, Error> {
    if let Step::GuestTools(step) = &plan.steps[index] {
        return crate::guest_tools::verify(session, vm, step).await;
    }
    if let Step::Property(step) = &plan.steps[index] {
        return properties::verify(session, vm, step).await;
    }
    if let Some(class) = plan.steps[index].creation_class() {
        let evidence = plan.evidence[index].clone().ok_or(Error::DeviceConflict)?;
        let (reference, record) = if let Some(reference) = evidence.result_reference {
            let record = session
                .call(&format!("{class}.get_record"), vec![json!(&reference)])
                .await?;
            (reference, record)
        } else {
            let records = session
                .call(&format!("{class}.get_all_records"), vec![])
                .await?;
            let records = records
                .as_object()
                .filter(|values| values.len() <= 10_000)
                .ok_or(Error::DeviceConflict)?;
            let mut candidates = records.iter().filter(|(_, record)| {
                VifRecord::new(record).other_config()[MARKER] == id.to_string()
            });
            let Some((reference, record)) = candidates.next() else {
                return Ok(false);
            };
            if candidates.next().is_some() {
                return Err(Error::DeviceConflict);
            }
            (self::reference(&json!(reference))?, record.clone())
        };
        let uuid = VifRecord::new(&record)
            .uuid()
            .as_str()
            .and_then(|id| Uuid::parse_str(id).ok())
            .ok_or(Error::DeviceConflict)?;
        if plan
            .steps
            .iter()
            .any(|step| matches!(step,Step::DestroyNic{vif,..} if vif.uuid()==uuid))
        {
            return Err(Error::DeviceConflict);
        }
        if evidence.baseline.contains(&uuid)
            || !creation_matches(session, vm, id, plan, index, &record).await?
        {
            return Err(Error::DeviceConflict);
        }
        let created = match class {
            "VDI" => CreatedDevice::Disk(VdiId::from(uuid)),
            "VBD" => CreatedDevice::Attachment(VbdId::from(uuid)),
            _ => CreatedDevice::Nic(VifId::from(uuid)),
        };
        if evidence
            .created
            .as_ref()
            .is_some_and(|previous| previous != &created)
        {
            return Err(Error::DeviceConflict);
        }
        let evidence = plan.evidence[index].as_mut().unwrap();
        evidence.result_reference = Some(reference);
        evidence.created = Some(created);
        return Ok(true);
    }
    match &plan.steps[index] {
        Step::DestroyAttachment { vbd, .. } => {
            Ok(
                crate::snapshots::optional_record(session, "VBD", &vbd.to_string())
                    .await?
                    .is_none(),
            )
        }
        Step::DestroyDisk { disk } => {
            Ok(
                crate::snapshots::optional_record(session, "VDI", &disk.to_string())
                    .await?
                    .is_none(),
            )
        }
        Step::DestroyNic { vif, .. } => {
            Ok(
                crate::snapshots::optional_record(session, "VIF", &vif.to_string())
                    .await?
                    .is_none(),
            )
        }
        Step::MoveNic { vif, to, mac, .. } => {
            let (network_ref, _) = network(session, *to).await?;
            let (_, record) = fresh(session, "VIF", vif.uuid()).await?;
            let vm_ref = guest(session, vm, false).await?;
            Ok(VifRecord::new(&record).vm() == vm_ref
                && VifRecord::new(&record).network() == network_ref
                && VifRecord::new(&record)
                    .mac()
                    .as_str()
                    .is_some_and(|m| m.eq_ignore_ascii_case(mac)))
        }
        _ => Err(Error::InvalidDeviceOperation),
    }
}

pub(crate) fn property_requirements(
    change: &crate::DeviceProperty,
) -> crate::PermissionRequirements {
    properties::requirements(change)
}
