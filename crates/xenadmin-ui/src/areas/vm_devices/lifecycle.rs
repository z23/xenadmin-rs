//! Devices that come and go: create, attach, detach and delete a disk, add a
//! CD/DVD drive, add, change and delete a network interface. Carried over
//! from `vm_device_lifecycle.rs` (`vm_devices::lifecycle`).

use std::sync::Arc;

use xenadmin_core::{
    DeviceCommand, DeviceOperation, Inventory, NetworkId, ObservedVm, PoolId, PowerState, SrId,
    VdiId, VifId, VirtualBlockDevice, VirtualInterface, VmCommand, VmId,
};

use crate::prelude::*;

/// Why the entry points are disabled (the caption of the Rune page).
pub const NOT_EDITABLE: &str = "Creating, attaching, detaching or deleting devices requires a halted VM and a current writable connection.";
pub const STALE: &str =
    "The connection or VM changed. Cancel and reopen this editor before continuing.";
pub const PREVIEW: &str =
    "Synthetic device review • Apply is disabled and no host changes can be submitted.";
pub const NO_CHOICES: &str = "No eligible resources are reported in the current inventory.";

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    CreateDisk,
    AttachDisk,
    AddCd,
    AddNic,
    EditNic {
        vif: VifId,
        network: NetworkId,
        mac: String,
    },
    Fixed(DeviceOperation),
}

impl Kind {
    pub fn title(&self) -> &'static str {
        match self {
            Kind::CreateDisk => "Create and attach disk",
            Kind::AttachDisk => "Attach existing disk",
            Kind::AddCd => "Add CD/DVD drive",
            Kind::AddNic => "Add network interface",
            Kind::EditNic { .. } => "Change network or MAC",
            Kind::Fixed(_) => "Review device change",
        }
    }
}

pub struct Review {
    pub operation: DeviceOperation,
    pub lines: Vec<String>,
    pub destructive: bool,
}

pub struct Form {
    pub target: Option<ObservedVm>,
    pub pool: PoolId,
    pub vm: VmId,
    pub vm_name: String,
    pub kind: Kind,
    pub name: String,
    pub size: String,
    pub device: String,
    pub mac: String,
    pub sr: Option<SrId>,
    pub network: Option<NetworkId>,
    pub disk: Option<VdiId>,
    pub read_only: bool,
    pub review: Option<Review>,
    pub error: Option<String>,
}

pub fn editable(m: &Model, pool: PoolId, inventory: &Inventory, vm: VmId) -> bool {
    (m.is_synthetic_pool(pool) || m.can_mutate_pool(pool))
        && inventory.vms.get(&vm).is_some_and(|guest| {
            guest.power_state == PowerState::Halted
                && !guest.is_template
                && !guest.is_control_domain
        })
}

/// `editable`, and the live observation `open` needs outside a synthetic
/// pool.
pub fn can_open(m: &Model, pool: PoolId, inventory: &Inventory, vm: VmId) -> bool {
    editable(m, pool, inventory, vm)
        && (m.is_synthetic_pool(pool) || super::observed(m, pool, vm).is_some())
}

/// Whether a fixed operation (detach, delete) can be opened: the host's
/// explanation whenever it is unavailable, also for a VM that is not
/// editable, as the Rune page showed it.
pub fn fixed_control(
    m: &Model,
    pool: PoolId,
    inventory: &Inventory,
    vm: VmId,
    operation: &DeviceOperation,
) -> Eligibility {
    let availability = operation.availability(inventory, vm);
    if !availability.is_available() {
        return Eligibility::disabled(availability.explanation());
    }
    Eligibility::enabled_if(can_open(m, pool, inventory, vm), NOT_EDITABLE)
}

/// Detach or delete the disk of an attachment; none for a CD/DVD drive.
pub fn disk_operation(attachment: &VirtualBlockDevice, delete: bool) -> Option<DeviceOperation> {
    let vdi = attachment.vdi.filter(|_| attachment.kind == "Disk")?;
    let vbd = attachment.id;
    Some(if delete {
        DeviceOperation::DeleteDisk { vbd, vdi }
    } else {
        DeviceOperation::DetachDisk { vbd, vdi }
    })
}

/// "Change network or MAC…": only for an interface with a known network.
pub fn edit_kind(interface: &VirtualInterface) -> Option<Kind> {
    Some(Kind::EditNic {
        vif: interface.id,
        network: interface.network?,
        mac: interface.mac.clone(),
    })
}

pub fn next_device(inventory: &Inventory, vm: VmId, nic: bool) -> String {
    (0..=255)
        .find(|slot| {
            let device = slot.to_string();
            if nic {
                !inventory
                    .details
                    .vifs
                    .values()
                    .any(|vif| vif.vm == Some(vm) && vif.device == device)
            } else {
                !inventory
                    .details
                    .vbds
                    .values()
                    .any(|vbd| vbd.vm == Some(vm) && vbd.userdevice == device)
            }
        })
        .map(|slot| slot.to_string())
        .unwrap_or_default()
}

pub fn open(m: &mut Model, pool: PoolId, vm: VmId, kind: Kind) -> bool {
    let Some(inventory) = m.inventory_for(pool) else {
        return false;
    };
    if !editable(m, pool, &inventory, vm) {
        return false;
    }
    let target = super::observed(m, pool, vm);
    if !m.is_synthetic_pool(pool) && target.is_none() {
        return false;
    }
    let Some(guest) = inventory.vms.get(&vm) else {
        return false;
    };
    let mut form = Form {
        target,
        pool,
        vm,
        vm_name: guest.name.clone(),
        device: next_device(&inventory, vm, matches!(kind, Kind::AddNic)),
        name: String::new(),
        size: String::new(),
        mac: String::new(),
        sr: None,
        network: None,
        disk: None,
        read_only: false,
        review: None,
        error: None,
        kind,
    };
    if let Kind::EditNic { network, mac, .. } = &form.kind {
        form.network = Some(*network);
        form.mac = mac.clone();
    }
    if let Kind::Fixed(operation) = &form.kind {
        form.review = Some(review_for(operation.clone(), &inventory));
    }
    super::clear(m);
    m.areas.vm_devices.lifecycle = Some(form);
    m.error = None;
    true
}

pub fn close(m: &mut Model) {
    m.areas.vm_devices.lifecycle = None;
}

pub fn back(m: &mut Model) {
    if let Some(form) = &mut m.areas.vm_devices.lifecycle {
        form.review = None;
    }
}

pub fn parse_device(value: &str) -> Result<String, String> {
    let value = value.trim();
    value.parse::<u8>().ok().filter(|number| number.to_string() == value)
        .map(|number| number.to_string()).ok_or_else(|| "Enter a device slot from 0 to 255 without leading zeros. The host checks whether the slot is available.".into())
}

pub fn parse_mac(value: &str) -> Result<String, String> {
    let parts: Vec<_> = value.trim().split(':').collect();
    let bytes = parts
        .iter()
        .map(|part| {
            if part.len() == 2 {
                u8::from_str_radix(part, 16).ok()
            } else {
                None
            }
        })
        .collect::<Option<Vec<_>>>();
    match bytes {
        Some(bytes) if bytes.len() == 6 && bytes[0] & 1 == 0 && bytes.iter().any(|byte| *byte != 0) =>
            Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect::<Vec<_>>().join(":")),
        _ => Err("Enter a nonzero unicast MAC address as six hexadecimal octets, for example 02:00:00:00:00:01.".into()),
    }
}

pub fn build_operation(form: &Form) -> Result<DeviceOperation, String> {
    match &form.kind {
        Kind::CreateDisk => {
            let name = form.name.trim();
            if name.is_empty()
                || name.chars().count() > 255
                || form.name.chars().any(char::is_control)
            {
                return Err(
                    "Enter a disk name of 1–255 characters without control characters.".into(),
                );
            }
            Ok(DeviceOperation::CreateDisk {
                sr: form.sr.ok_or("Select a storage repository.")?,
                name: name.into(),
                size: super::actions::parse_size(&form.size)?,
                device: parse_device(&form.device)?,
            })
        }
        Kind::AttachDisk => Ok(DeviceOperation::AttachDisk {
            vdi: form.disk.ok_or("Select a disk.")?,
            device: parse_device(&form.device)?,
            read_only: form.read_only,
        }),
        Kind::AddCd => Ok(DeviceOperation::AddCdDrive {
            device: parse_device(&form.device)?,
        }),
        Kind::AddNic => Ok(DeviceOperation::AddNic {
            network: form.network.ok_or("Select a network.")?,
            device: parse_device(&form.device)?,
            mac: if form.mac.trim().is_empty() {
                None
            } else {
                Some(parse_mac(&form.mac)?)
            },
        }),
        Kind::EditNic { vif, network, mac } => {
            let selected_network = form.network.ok_or("Select a network.")?;
            let selected_mac = parse_mac(&form.mac)?;
            if selected_network == *network && selected_mac.eq_ignore_ascii_case(mac) {
                return Err("Choose a different network or MAC address before reviewing.".into());
            }
            Ok(DeviceOperation::EditNic {
                vif: *vif,
                expected_network: *network,
                expected_mac: mac.clone(),
                network: selected_network,
                mac: selected_mac,
            })
        }
        Kind::Fixed(operation) => Ok(operation.clone()),
    }
}

pub fn disk_name(inventory: &Inventory, id: VdiId) -> String {
    inventory.details.vdis.get(&id).map_or_else(
        || id.to_string(),
        |disk| format!("{} ({id}), {}", disk.name, storage_bytes(disk.virtual_size)),
    )
}

pub fn network_name(inventory: &Inventory, id: NetworkId) -> String {
    inventory
        .networks
        .iter()
        .find(|network| network.uuid == id.uuid())
        .map_or_else(
            || id.to_string(),
            |network| format!("{} ({id})", network.name),
        )
}

fn consumers(inventory: &Inventory, id: VdiId) -> Vec<String> {
    inventory
        .details
        .vbds
        .values()
        .filter(|vbd| vbd.vdi == Some(id))
        .map(|vbd| {
            let owner = vbd
                .vm
                .and_then(|id| {
                    inventory
                        .vms
                        .get(&id)
                        .map(|vm| format!("{} ({id})", vm.name))
                })
                .or_else(|| {
                    vbd.snapshot.and_then(|id| {
                        inventory
                            .snapshots
                            .get(&id)
                            .map(|snapshot| format!("Snapshot {} ({id})", snapshot.name))
                    })
                })
                .unwrap_or_else(|| "Owner not reported".into());
            format!(
                "Consumer: {owner}; attachment {}; slot {}",
                vbd.id, vbd.userdevice
            )
        })
        .collect()
}

pub fn review_for(operation: DeviceOperation, inventory: &Inventory) -> Review {
    let mut lines = Vec::new();
    let mut destructive = false;
    match &operation {
        DeviceOperation::CreateDisk { sr, name, size, device } => {
            let sr_name = inventory.storage.iter().find(|storage| storage.uuid == sr.uuid()).map_or("Storage repository", |storage| storage.name.as_str());
            lines.push(format!("Create “{name}” with {size} bytes ({}) on {sr_name} ({sr}), then attach it in slot {device}.", storage_bytes(Some(*size))));
            lines.push("If attachment fails after creation, the new disk is retained and identified in the operation history. It is not silently deleted.".into());
        }
        DeviceOperation::AttachDisk { vdi, device, read_only } => lines.push(format!("Attach {} in slot {device} with {} access. Existing disk contents are retained.", disk_name(inventory, *vdi), if *read_only { "read-only" } else { "read/write" })),
        DeviceOperation::DetachDisk { vbd, vdi } => {
            lines.push(format!("Remove attachment {vbd} for {}. The disk and its data remain in storage and can be attached again.", disk_name(inventory, *vdi)));
            lines.extend(consumers(inventory, *vdi));
        }
        DeviceOperation::DeleteDisk { vbd, vdi } => {
            destructive = true;
            lines.push(format!("Permanently delete {} after removing attachment {vbd}. All data on this disk will be lost. This cannot be undone.", disk_name(inventory, *vdi)));
            lines.extend(consumers(inventory, *vdi));
            lines.push("Deletion is refused for shared disks, additional consumers, or disks this VM does not own. Ownership is checked again on the host. A later failure may leave the disk detached but retained.".into());
        }
        DeviceOperation::AddCdDrive { device } => lines.push(format!("Add an empty CD/DVD drive in slot {device}. Use Insert ISO afterwards to select installation media.")),
        DeviceOperation::AddNic { network, device, mac } => lines.push(format!("Add a network interface in slot {device} on {}. MAC: {}. Guest network configuration may be required before it can connect.", network_name(inventory, *network), mac.as_deref().unwrap_or("generated by the host"))),
        DeviceOperation::DeleteNic { vif } => {
            destructive = true;
            lines.push(format!("Delete network interface {vif} from this VM. Its network attachment and MAC assignment are removed. Guest network configuration may need updating."));
        }
        DeviceOperation::EditNic { vif, expected_network, expected_mac, network, mac } => {
            lines.push(format!("Interface: {vif}\nNetwork: {} → {}\nMAC: {expected_mac} → {mac}", network_name(inventory, *expected_network), network_name(inventory, *network)));
            if !expected_mac.eq_ignore_ascii_case(mac) {
                destructive = true;
                lines.push("Changing the MAC replaces this interface because XAPI fixes MAC addresses at creation. If replacement fails after removal, the VM may be left without this NIC. Unsupported configuration is refused; completed steps and new device identities remain in the operation history.".into());
            } else {
                lines.push("The interface moves to the selected network. Guest network configuration may need updating.".into());
            }
        }
        _ => lines.push(operation.label().into()),
    }
    Review {
        operation,
        lines,
        destructive,
    }
}

pub fn review(m: &mut Model) {
    let Some(form) = &m.areas.vm_devices.lifecycle else {
        return;
    };
    let result = build_operation(form).and_then(|operation| {
        let inventory = m
            .inventory_for(form.pool)
            .ok_or("The selected connection changed.")?;
        let availability = operation.availability(&inventory, form.vm);
        if !availability.is_available() {
            return Err(availability.explanation());
        }
        Ok(review_for(operation, &inventory))
    });
    if let Some(form) = &mut m.areas.vm_devices.lifecycle {
        match result {
            Ok(review) => {
                form.review = Some(review);
                form.error = None;
            }
            Err(error) => form.error = Some(error),
        }
    }
}

/// The inventory of the form's pool, while that pool is still shown.
pub fn inventory(m: &Model, form: &Form) -> Option<Arc<Inventory>> {
    m.inventory_for(form.pool)
}

/// The captured VM is still the one the connection observes; in a synthetic
/// pool there is nothing to compare.
pub fn target_current(m: &Model, form: &Form) -> bool {
    inventory(m, form).is_some()
        && (m.is_synthetic_pool(form.pool)
            || form
                .target
                .is_some_and(|target| super::observed(m, form.pool, form.vm) == Some(target)))
}

/// Why the reviewed operation cannot be applied any more.
pub fn unavailable(m: &Model, form: &Form) -> Option<String> {
    let review = form.review.as_ref()?;
    let inventory = inventory(m, form)?;
    let status = review.operation.availability(&inventory, form.vm);
    (!status.is_available()).then(|| {
        format!(
            "This change is no longer available: {} Cancel and review it again.",
            status.explanation()
        )
    })
}

pub fn apply(m: &mut Model) {
    let Some(form) = &m.areas.vm_devices.lifecycle else {
        return;
    };
    let Some(target) = form.target else { return };
    let Some(review) = &form.review else { return };
    let (pool, vm) = (form.pool, form.vm);
    if m.is_synthetic_pool(pool)
        || !m.can_mutate_pool(pool)
        || super::observed(m, pool, vm) != Some(target)
        || !m.inventory_for(pool).is_some_and(|inventory| {
            review
                .operation
                .availability(&inventory, vm)
                .is_available()
        })
    {
        return;
    }
    let command = DeviceCommand {
        vm: target,
        operation: review.operation.clone(),
    };
    if !permissions::check_for(m, pool, &command.operation.permission_requirements()) {
        return;
    }
    m.submit_command_for(pool, VmCommand::Device(command));
    if m.error.is_none() {
        close(m);
    }
}

/// The repositories a new disk can be created on, by name.
pub fn repositories(form: &Form, inventory: &Inventory) -> Vec<(SrId, String)> {
    let mut repositories: Vec<_> = inventory
        .details
        .storage
        .values()
        .filter(|storage| {
            DeviceOperation::CreateDisk {
                sr: storage.id,
                name: "Disk".into(),
                size: 512,
                device: "0".into(),
            }
            .availability(inventory, form.vm)
            .is_available()
        })
        .map(|storage| {
            (
                storage.id,
                inventory
                    .storage
                    .iter()
                    .find(|row| row.uuid == storage.id.uuid())
                    .map_or_else(
                        || storage.id.to_string(),
                        |row| format!("{} ({})", row.name, storage.id),
                    ),
            )
        })
        .collect();
    repositories.sort_by_cached_key(|(_, name)| name.to_lowercase());
    repositories
}

/// The disks that can be attached with the slot and the access mode as
/// entered so far.
pub fn attachable(form: &Form, inventory: &Inventory) -> Vec<(VdiId, String)> {
    let device = parse_device(&form.device).unwrap_or_else(|_| "0".into());
    let mut disks: Vec<_> = inventory
        .details
        .vdis
        .values()
        .filter(|disk| {
            DeviceOperation::AttachDisk {
                vdi: disk.id,
                device: device.clone(),
                read_only: form.read_only,
            }
            .availability(inventory, form.vm)
            .is_available()
        })
        .map(|disk| (disk.id, disk_name(inventory, disk.id)))
        .collect();
    disks.sort_by_cached_key(|(_, label)| label.to_lowercase());
    disks
}

pub fn networks(inventory: &Inventory) -> Vec<(NetworkId, String)> {
    let mut networks: Vec<_> = inventory
        .networks
        .iter()
        .map(|network| {
            let id = NetworkId::from(network.uuid);
            (id, format!("{} ({id})", network.name))
        })
        .collect();
    networks.sort_by_cached_key(|(_, label)| label.to_lowercase());
    networks
}
