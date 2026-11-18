//! Properties of one device, one change per review: a disk's name and
//! description, the access mode, position, bootable flag and I/O priority of
//! its attachment, disabling changed-block tracking, and the rate limit of a
//! network interface. Carried over from `vm_device_properties.rs`
//! (`vm_devices::properties`).
//!
//! The same form is the draft of the dialog `device-properties` and of the
//! page General of a virtual disk's Properties dialog ([`Form::page`]).

use std::sync::Arc;

use xenadmin_core::{
    DeviceCommand, DeviceOperation, DeviceProperty as Change, DeviceQos, DiskAccess, Inventory,
    ObservedVm, PoolId, VbdId, VdiId, VifId, VmCommand, VmId,
};

use crate::prelude::*;

pub const STALE: &str = "The connection or VM changed. Close and reopen the editor.";
pub const PREVIEW: &str = "Synthetic review · no host changes can be submitted.";
pub const NO_RATE: &str = "Rate limit editing is unavailable because the QoS configuration is unknown or uses another algorithm.";
pub const NO_DISK_RECORD: &str = "The backing disk record is unavailable.";

pub struct Form {
    pub pool: PoolId,
    pub vm: VmId,
    pub target: Option<ObservedVm>,
    pub vm_name: String,
    pub device_name: String,
    pub choices: Vec<Change>,
    pub notes: Vec<String>,
    pub choice: Option<Change>,
    pub name: String,
    pub description: String,
    pub value: String,
    pub enabled: bool,
    pub review: Option<Change>,
    pub error: Option<String>,
    /// The form is the draft of the page General of the disk's Properties
    /// dialog, not of the dialog `device-properties`.
    pub page: bool,
}

/// Closes the editor. A page's draft takes its Properties dialog with it.
pub fn close(m: &mut Model) {
    let page = m
        .areas
        .vm_devices
        .properties
        .take()
        .is_some_and(|form| form.page);
    if page {
        m.close_properties();
    }
}

/// The form for an attachment (`disk`) or an interface (`nic`) of a VM,
/// with the changes the inventory allows offering.
pub fn capture(
    m: &Model,
    pool: PoolId,
    vm: VmId,
    disk: Option<VbdId>,
    nic: Option<VifId>,
) -> Option<Form> {
    let i = m.inventory_for(pool)?;
    let guest = i.vms.get(&vm)?;
    let mut choices = Vec::new();
    let mut notes = Vec::new();
    let device_name;
    if let Some(vbd) = disk {
        let v = i
            .details
            .vbds
            .get(&vbd)
            .filter(|v| v.vm == Some(vm) && v.kind == "Disk")?;
        let d = v.vdi.and_then(|id| i.details.vdis.get(&id))?;
        device_name = format!("{} · disk {} · attachment {vbd}", d.name, d.id);
        let vdi = d.id;
        choices.push(Change::DiskMetadata {
            vbd,
            vdi,
            previous_name: d.name.clone(),
            previous_description: d.description.clone(),
            name: d.name.clone(),
            description: d.description.clone(),
        });
        if let Some(previous) = match v.mode.as_str() {
            "RO" => Some(DiskAccess::ReadOnly),
            "RW" => Some(DiskAccess::ReadWrite),
            _ => None,
        } {
            choices.push(Change::DiskAccess {
                vbd,
                vdi,
                previous,
                value: if previous == DiskAccess::ReadOnly {
                    DiskAccess::ReadWrite
                } else {
                    DiskAccess::ReadOnly
                },
            });
        }
        if v.userdevice
            .parse::<u8>()
            .is_ok_and(|n| n.to_string() == v.userdevice)
        {
            choices.push(Change::DiskPosition {
                vbd,
                vdi,
                previous: v.userdevice.clone(),
                value: String::new(),
            });
        }
        if let Some(previous) = v.bootable {
            choices.push(Change::Bootable {
                vbd,
                vdi,
                previous,
                value: !previous,
            });
        }
        if let Some(previous) = v
            .qos
            .as_ref()
            .filter(|q| matches!(q.algorithm.as_str(), "" | "ionice"))
        {
            choices.push(Change::DiskPriority {
                vbd,
                vdi,
                previous: previous.clone(),
                priority: None,
            });
            if !v
                .qos_supported_algorithms
                .as_ref()
                .is_some_and(|a| a.iter().any(|a| a == "ionice"))
            {
                notes.push("The host has not advertised I/O priority support. Disabling an existing policy remains possible; enabling one will be refused.".into());
            }
        } else {
            notes.push("Disk I/O priority is unavailable because the QoS configuration is unknown or uses another algorithm.".into());
        }
        if d.cbt_enabled == Some(true) {
            choices.push(Change::DisableCbt { vbd, vdi });
        }
        notes.push("Configuration changes require a halted VM and a disconnected device. Disk changes require an exclusive ordinary disk. Name and description can be changed while the VM is running.".into());
    } else {
        let vif = nic?;
        let v = i.details.vifs.get(&vif).filter(|v| v.vm == Some(vm))?;
        device_name = format!("NIC {} · {vif}", v.device);
        if v.qos_known && matches!(v.qos_algorithm.as_str(), "" | "ratelimit") {
            choices.push(Change::NicRate {
                vif,
                previous: DeviceQos {
                    algorithm: v.qos_algorithm.clone(),
                    parameters: v.qos_parameters.clone(),
                },
                kilobytes_per_second: None,
            });
        } else {
            notes.push(NO_RATE.into());
        }
        notes.push("Rate limit changes require a halted VM and a disconnected NIC. The host must advertise ratelimit support to enable a limit.".into());
    }
    Some(Form {
        pool,
        vm,
        target: super::observed(m, pool, vm),
        vm_name: guest.name.clone(),
        device_name,
        choices,
        notes,
        choice: None,
        name: String::new(),
        description: String::new(),
        value: String::new(),
        enabled: false,
        review: None,
        error: None,
        page: false,
    })
}

pub fn open(
    m: &mut Model,
    pool: PoolId,
    vm: VmId,
    disk: Option<VbdId>,
    nic: Option<VifId>,
) -> bool {
    let Some(form) = capture(m, pool, vm, disk, nic) else {
        return false;
    };
    super::clear(m);
    m.areas.vm_devices.properties = Some(form);
    m.error = None;
    true
}

/// "NIC rate limit…": the editor of the first change, past the chooser.
pub fn open_rate(m: &mut Model, pool: PoolId, vm: VmId, nic: VifId) {
    if !open(m, pool, vm, None, Some(nic)) {
        return;
    }
    let first = m
        .areas
        .vm_devices
        .properties
        .as_ref()
        .and_then(|f| f.choices.first().cloned());
    if let Some(choice) = first {
        choose(m, choice);
    }
}

/// The attachment through which a disk's name and description can be
/// changed: a device command needs a VM, so a disk no VM uses has none.
pub fn metadata_route(i: &Inventory, disk: VdiId) -> Option<(VmId, VbdId)> {
    let mut routes: Vec<_> = i
        .details
        .vbds
        .values()
        .filter(|v| v.vdi == Some(disk) && v.kind == "Disk")
        .filter_map(|v| Some((v.vm.filter(|vm| i.vms.contains_key(vm))?, v.id)))
        .collect();
    routes.sort();
    routes.into_iter().next()
}

/// The draft of the page General of a disk's Properties dialog: the form
/// of its attachment, on the name and description.
pub fn open_page(m: &mut Model, pool: PoolId, disk: VdiId) -> bool {
    let Some((vm, vbd)) = m
        .inventory_for(pool)
        .and_then(|i| metadata_route(&i, disk))
    else {
        return false;
    };
    let Some(mut form) = capture(m, pool, vm, Some(vbd), None) else {
        return false;
    };
    form.page = true;
    let metadata = form.choices.first().cloned();
    m.areas.vm_devices.form = None;
    m.areas.vm_devices.lifecycle = None;
    m.areas.vm_devices.properties = Some(form);
    if let Some(choice) = metadata {
        choose(m, choice);
    }
    true
}

pub fn choose(m: &mut Model, choice: Change) {
    let Some(f) = &mut m.areas.vm_devices.properties else {
        return;
    };
    f.value = String::new();
    f.enabled = false;
    match &choice {
        Change::DiskMetadata {
            name, description, ..
        } => {
            f.name = name.clone();
            f.description = description.clone();
        }
        Change::DiskPriority { previous, .. } => {
            f.enabled = previous.algorithm == "ionice";
            f.value = previous
                .parameters
                .get("class")
                .cloned()
                .unwrap_or_else(|| "4".into());
        }
        Change::NicRate { previous, .. } => {
            f.enabled = previous.algorithm == "ratelimit";
            f.value = previous
                .parameters
                .get("kbps")
                .cloned()
                .unwrap_or_else(|| "12500".into());
        }
        _ => {}
    }
    f.choice = Some(choice);
    f.review = None;
    f.error = None;
}

/// Back: from the review to the editor, from the editor to the chooser.
pub fn back(m: &mut Model) {
    if let Some(f) = &mut m.areas.vm_devices.properties {
        if f.review.take().is_none() && !f.page {
            f.choice = None;
        }
        f.error = None;
    }
}

pub fn build(f: &Form) -> Result<Change, String> {
    let mut choice = f.choice.clone().ok_or("Select a property to change.")?;
    match &mut choice {
        Change::DiskMetadata {
            name,
            description,
            previous_name,
            previous_description,
            ..
        } => {
            *name = f.name.clone();
            *description = f.description.clone();
            if name.trim().is_empty()
                || name.chars().count() > 255
                || name.chars().any(char::is_control)
                || description.len() > 16_384
                || description.contains('\0')
            {
                return Err(
                    "Enter a nonempty name up to 255 characters and a description up to 16 KiB."
                        .into(),
                );
            }
            if name == previous_name && description == previous_description {
                return Err("Change the name or description before reviewing.".into());
            }
        }
        Change::DiskPosition {
            previous, value, ..
        } => {
            *value = f.value.clone();
            if !value.parse::<u8>().is_ok_and(|n| n.to_string() == *value) {
                return Err("Enter an unused slot from 0 to 255 without leading zeros.".into());
            }
            if value == previous {
                return Err("Choose a different device position.".into());
            }
        }
        Change::DiskPriority {
            previous, priority, ..
        } => {
            *priority = if f.enabled {
                Some(
                    f.value
                        .parse::<u8>()
                        .ok()
                        .filter(|n| *n <= 7)
                        .ok_or("Enter a priority from 0 (highest) to 7 (lowest).")?,
                )
            } else {
                None
            };
            let unchanged = match priority {
                None => previous.algorithm.is_empty(),
                Some(p) => {
                    previous.algorithm == "ionice"
                        && previous.parameters.get("class") == Some(&p.to_string())
                        && previous.parameters.get("sched").is_some_and(|v| v == "be")
                }
            };
            if unchanged {
                return Err("Change the priority before reviewing.".into());
            }
        }
        Change::NicRate {
            previous,
            kilobytes_per_second,
            ..
        } => {
            *kilobytes_per_second = if f.enabled {
                Some(
                    f.value
                        .parse::<u64>()
                        .ok()
                        .filter(|n| *n > 0 && *n <= i64::MAX as u64)
                        .ok_or("Enter a positive whole number of kilobytes per second.")?,
                )
            } else {
                None
            };
            let unchanged = match kilobytes_per_second {
                None => previous.algorithm.is_empty(),
                Some(n) => {
                    previous.algorithm == "ratelimit"
                        && previous.parameters.get("kbps") == Some(&n.to_string())
                }
            };
            if unchanged {
                return Err("Change the rate limit before reviewing.".into());
            }
        }
        Change::DisableCbt { vdi, .. } if f.value != vdi.to_string() => {
            return Err(
                "Enter the exact disk UUID to confirm disabling changed-block tracking.".into(),
            );
        }
        _ => {}
    }
    Ok(choice)
}

pub fn operation(change: &Change) -> DeviceOperation {
    DeviceOperation::Property {
        change: change.clone(),
    }
}

/// The inventory of the form's pool while it still has the VM.
pub fn inventory(m: &Model, f: &Form) -> Option<Arc<Inventory>> {
    m.inventory_for(f.pool)
        .filter(|i| i.vms.contains_key(&f.vm))
}

pub fn current(m: &Model, f: &Form) -> bool {
    inventory(m, f).is_some()
        && (m.is_synthetic_pool(f.pool)
            || f.target
                .is_some_and(|t| super::observed(m, f.pool, f.vm) == Some(t)))
}

pub fn review(m: &mut Model) {
    let Some(f) = &m.areas.vm_devices.properties else {
        return;
    };
    let result = build(f).and_then(|change| {
        if !current(m, f) {
            return Err("The connection changed. Close and reopen the editor.".into());
        }
        let i = inventory(m, f).expect("current inventory");
        let a = operation(&change).availability(&i, f.vm);
        if !a.is_available() {
            return Err(a.explanation());
        }
        Ok(change)
    });
    if let Some(f) = &mut m.areas.vm_devices.properties {
        match result {
            Ok(change) => {
                f.review = Some(change);
                f.error = None;
            }
            Err(e) => f.error = Some(e),
        }
    }
}

/// Whether the host still allows the reviewed change.
pub fn allowed(m: &Model, f: &Form, change: &Change) -> bool {
    inventory(m, f).is_some_and(|i| operation(change).availability(&i, f.vm).is_available())
}

pub fn apply(m: &mut Model) {
    let Some(f) = &m.areas.vm_devices.properties else {
        return;
    };
    let Some(target) = f.target else { return };
    let Some(change) = &f.review else { return };
    let pool = f.pool;
    if m.is_synthetic_pool(pool)
        || !m.can_mutate_pool(pool)
        || !current(m, f)
        || !allowed(m, f, change)
    {
        return;
    }
    let command = DeviceCommand {
        vm: target,
        operation: operation(change),
    };
    if !permissions::check_for(m, pool, &command.operation.permission_requirements()) {
        return;
    }
    m.submit_command_for(pool, VmCommand::Device(command));
    if m.error.is_none() {
        close(m);
    }
}

pub fn summary(change: &Change) -> String {
    match change {
        Change::DiskMetadata { previous_name, previous_description, name, description, .. } => format!("Name: {previous_name} → {name}\nDescription: {previous_description} → {description}\nThese are separate changes. A failure can leave only the name updated."),
        Change::DiskAccess { previous, value, .. } => format!("Access: {} → {}. The attachment and disk identities are retained.", access(*previous), access(*value)),
        Change::DiskPosition { previous, value, .. } => format!("Device position: {previous} → {value}. The new slot must be unused. Guest device naming or boot configuration may change."),
        Change::Bootable { previous, value, .. } => format!("Bootable: {previous} → {value}. This changes the disk attachment flag; it does not reorder the VM firmware boot sequence."),
        Change::DiskPriority { previous, priority, .. } => format!("Previous algorithm: {} · class {}. New I/O priority: {}. Other QoS parameters are retained. Parameter and algorithm changes are tracked separately.", qos_name(&previous.algorithm), previous.parameters.get("class").map_or("not set", String::as_str), priority.map_or_else(|| "disabled".into(), |p| format!("{p} (0 highest, 7 lowest)"))),
        Change::NicRate { previous, kilobytes_per_second, .. } => format!("Previous algorithm: {} · limit {} KB/s. New rate limit: {}. Units are kilobytes per second (KB/s). Other QoS parameters, the NIC UUID and MAC address are retained.", qos_name(&previous.algorithm), previous.parameters.get("kbps").map_or("not set", String::as_str), kilobytes_per_second.map_or_else(|| "disabled".into(), |n| format!("{n} KB/s"))),
        Change::DisableCbt { .. } => "Disable changed-block tracking for this disk. This breaks the existing incremental backup chain; the backup system may need a new full backup. Existing disk data is retained. Coordinate this with the backup owner before applying.".into(),
    }
}

fn qos_name(s: &str) -> &str {
    if s.is_empty() { "disabled" } else { s }
}

fn access(v: DiskAccess) -> &'static str {
    match v {
        DiskAccess::ReadOnly => "read only",
        DiskAccess::ReadWrite => "read/write",
    }
}
