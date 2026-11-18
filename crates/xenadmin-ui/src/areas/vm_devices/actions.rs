//! Device changes with nothing or one thing to enter: connect and
//! disconnect a disk or a NIC, eject media, insert an ISO. Carried over from
//! `vm_device_actions.rs` (`vm_devices::actions`), with `parse_size`, which
//! the disk creation and the disk dialog share.
//!
//! What changed against Rune: the form names its pool and VM itself (Rune
//! read them from the Devices page), and `apply` makes every check of the
//! confirm button again (Rune relied on the disabled button alone).

use std::sync::Arc;

use xenadmin_core::{
    DeviceCommand, DeviceOperation, Inventory, ObservedVm, PoolId, VbdId, VdiId,
    VirtualBlockDevice, VirtualInterface, VmCommand, VmId,
};

use crate::prelude::*;

/// Shown where a change needs a writable, current connection and has none.
pub const RECONNECT: &str = "Reconnect and wait for current inventory.";
pub const INSERT_REFUSED: &str =
    "The host does not currently advertise permission to insert media into this drive.";
pub const NO_IMAGE: &str = "Select an ISO image before continuing.";
pub const STALE: &str = "The connection changed. Close and reopen the device editor to refresh it.";

pub struct Form {
    pub pool: PoolId,
    pub vm: VmId,
    pub target: ObservedVm,
    pub vm_name: String,
    pub operation: Option<DeviceOperation>,
    pub media_drive: Option<VbdId>,
    pub review: bool,
    pub error: Option<String>,
}

pub fn permitted(allowed: &Option<Vec<String>>, operation: &str) -> bool {
    allowed
        .as_ref()
        .is_some_and(|allowed| allowed.iter().any(|entry| entry == operation))
}

pub fn ready(m: &Model, pool: PoolId) -> bool {
    m.can_mutate_pool(pool)
}

/// Opens the change for a VM of `pool`. A fixed operation opens on its
/// review, the ISO chooser on the picker. Needs a live observation of the
/// VM, so it never opens in a synthetic pool.
pub fn open(
    m: &mut Model,
    pool: PoolId,
    vm: VmId,
    operation: Option<DeviceOperation>,
    media_drive: Option<VbdId>,
) -> bool {
    let Some(inventory) = m.inventory_for(pool) else {
        return false;
    };
    let Some(target) = super::observed(m, pool, vm) else {
        return false;
    };
    let Some(guest) = inventory.vms.get(&vm) else {
        return false;
    };
    let form = Form {
        pool,
        vm,
        target,
        vm_name: guest.name.clone(),
        operation,
        media_drive,
        review: media_drive.is_none(),
        error: None,
    };
    super::clear(m);
    m.areas.vm_devices.form = Some(form);
    true
}

/// Whether a fixed operation can be opened, with the reason the Rune page
/// showed under its button.
pub fn control(m: &Model, pool: PoolId, inventory: &Inventory, vm: VmId, operation: &DeviceOperation) -> Eligibility {
    let availability = operation.availability(inventory, vm);
    if !ready(m, pool) {
        return Eligibility::disabled(RECONNECT);
    }
    if !availability.is_available() {
        return Eligibility::disabled(availability.explanation());
    }
    Eligibility::enabled_if(super::observed(m, pool, vm).is_some(), RECONNECT)
}

/// Connect or disconnect, by what the attachment reports.
pub fn disk_plug(attachment: &VirtualBlockDevice) -> DeviceOperation {
    if attachment.currently_attached == Some(true) {
        DeviceOperation::UnplugDisk { vbd: attachment.id }
    } else {
        DeviceOperation::PlugDisk { vbd: attachment.id }
    }
}

pub fn nic_plug(interface: &VirtualInterface) -> DeviceOperation {
    if interface.currently_attached == Some(true) {
        DeviceOperation::UnplugNic { vif: interface.id }
    } else {
        DeviceOperation::PlugNic { vif: interface.id }
    }
}

pub fn is_cd(attachment: &VirtualBlockDevice) -> bool {
    attachment.kind.eq_ignore_ascii_case("CD")
}

pub fn close(m: &mut Model) {
    m.areas.vm_devices.form = None;
}

pub fn media_choices(inventory: &Inventory, vm: VmId, vbd: VbdId) -> Vec<(VdiId, String)> {
    let mut choices: Vec<_> = inventory
        .details
        .vdis
        .values()
        .filter(|disk| {
            DeviceOperation::InsertCd {
                vbd,
                image: disk.id,
            }
            .availability(inventory, vm)
            .is_available()
                && disk
                    .sr
                    .and_then(|id| inventory.details.storage.get(&id))
                    .is_some_and(|sr| sr.content_type.eq_ignore_ascii_case("iso"))
        })
        .map(|disk| (disk.id, format!("{} · {}", disk.name, disk.id)))
        .collect();
    choices.sort_by_cached_key(|choice| choice.1.to_lowercase());
    choices
}

pub fn choose_image(m: &mut Model, image: VdiId) {
    let Some(form) = &mut m.areas.vm_devices.form else {
        return;
    };
    if let Some(vbd) = form.media_drive {
        form.operation = Some(DeviceOperation::InsertCd { vbd, image });
        form.error = None;
    }
}

pub fn parse_size(text: &str) -> Result<u64, String> {
    let compact = text.split_whitespace().collect::<String>();
    let (number, factor) = if let Some(value) = compact.strip_suffix("GiB") {
        (value, 1u64 << 30)
    } else if let Some(value) = compact.strip_suffix("MiB") {
        (value, 1 << 20)
    } else if let Some(value) = compact.strip_suffix("KiB") {
        (value, 1 << 10)
    } else {
        (compact.as_str(), 1)
    };
    number
        .parse::<u64>()
        .ok()
        .and_then(|value| value.checked_mul(factor))
        .filter(|value| *value > 0 && *value <= i64::MAX as u64 && value % 512 == 0)
        .ok_or_else(|| "Enter a positive capacity in 512-byte increments, as bytes or a whole number with KiB, MiB or GiB.".into())
}

pub fn review(m: &mut Model) {
    let Some(form) = &mut m.areas.vm_devices.form else {
        return;
    };
    if form.operation.is_none() {
        form.error = Some(NO_IMAGE.into());
        return;
    }
    form.error = None;
    form.review = true;
}

/// The inventory of the form's pool, while that pool is still shown.
pub fn inventory(m: &Model, form: &Form) -> Option<Arc<Inventory>> {
    m.inventory_for(form.pool)
}

pub fn target_current(m: &Model, form: &Form) -> bool {
    super::observed(m, form.pool, form.vm) == Some(form.target)
}

/// Why the reviewed operation cannot be applied now, as the dialog says it.
pub fn unavailable(m: &Model, form: &Form) -> Option<String> {
    let operation = form.operation.as_ref()?;
    let inventory = inventory(m, form)?;
    let status = operation.availability(&inventory, form.vm);
    (!status.is_available()).then(|| {
        format!(
            "This change is no longer available: {} Cancel and reopen the editor after refreshing inventory.",
            status.explanation()
        )
    })
}

pub fn apply(m: &mut Model) {
    let Some(form) = &m.areas.vm_devices.form else {
        return;
    };
    let Some(operation) = form.operation.clone() else {
        return;
    };
    let (pool, vm, target) = (form.pool, form.vm, form.target);
    if !form.review
        || !ready(m, pool)
        || !target_current(m, form)
        || !m
            .inventory_for(pool)
            .is_some_and(|inventory| operation.availability(&inventory, vm).is_available())
    {
        return;
    }
    let command = DeviceCommand {
        vm: target,
        operation,
    };
    if !permissions::check_for(m, pool, &command.operation.permission_requirements()) {
        return;
    }
    m.submit_command_for(pool, VmCommand::Device(command));
    if m.error.is_none() {
        close(m);
    }
}

/// The one sentence of the review.
pub fn summary(inventory: &Inventory, operation: &DeviceOperation) -> String {
    match operation {
        DeviceOperation::InsertCd { vbd, image } => format!(
            "Insert ISO {} ({image}) into CD/DVD attachment {vbd}.",
            inventory
                .details
                .vdis
                .get(image)
                .map_or("Unknown image", |disk| disk.name.as_str())
        ),
        DeviceOperation::EjectCd { vbd } => format!(
            "Eject the current media from CD/DVD attachment {vbd}. The ISO image is retained."
        ),
        DeviceOperation::PlugDisk { vbd } => {
            format!("Connect disk attachment {vbd} to this VM.")
        }
        DeviceOperation::UnplugDisk { vbd } => format!(
            "Disconnect disk attachment {vbd}. Guest disk access stops; the disk data and attachment are retained. Ensure the guest is no longer using it."
        ),
        DeviceOperation::PlugNic { vif } => {
            format!("Connect network interface {vif} to this VM.")
        }
        DeviceOperation::UnplugNic { vif } => format!(
            "Disconnect network interface {vif}. The VM may lose network access through this interface."
        ),
        DeviceOperation::GrowDisk {
            vdi,
            expected_size,
            new_size,
        } => format!(
            "Grow disk {vdi} from {expected_size} bytes to {new_size} bytes. This cannot be undone by shrinking the disk. Extend partitions/filesystems inside the guest separately."
        ),
        operation => operation.label().into(),
    }
}
