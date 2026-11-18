//! Creating a VM from a template: the logic of the Rune `vm_creation.rs`.
//!
//! Validation, the operation and the review lines are carried over. The
//! three Rune steps (four with cloud-init) are cut along the plan's wizard
//! steps, see [`Step`]; what a step validates when it is left is what the
//! Rune step that owned the fields validated, and nothing earlier.

use std::collections::BTreeSet;

use xenadmin_core::{GuestFirmware, VdiId, VmLifecycleOperation};

use super::edit::{self, Editor};
use crate::prelude::*;

/// The steps of the wizard before its review, in the plan's order.
///
/// | Step | Fields | Rune step |
/// | --- | --- | --- |
/// | Name | name, description | 1, "Identity" |
/// | Installation Media | keep the inherited media, or one ISO | 3 |
/// | CPU and Memory | virtual CPUs, memory, firmware, BIOS identity and home host | 2 |
/// | Storage | repository, the template's disks, an additional disk | 3 |
/// | Networking | the network of every interface | 3 |
/// | Cloud-init | whether to configure it, and its three inputs | 3 and 4 |
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Step {
    Name,
    Media,
    Resources,
    Storage,
    Networking,
    Cloud,
}

impl Step {
    pub const ALL: [Step; 6] = [
        Step::Name,
        Step::Media,
        Step::Resources,
        Step::Storage,
        Step::Networking,
        Step::Cloud,
    ];
    /// The step labels of the wizard: the plan's names, then the review.
    pub const LABELS: [&'static str; 7] = [
        "Name",
        "Installation Media",
        "CPU and Memory",
        "Storage",
        "Networking",
        "Cloud-init",
        "Review",
    ];
    pub const LAST: Step = Step::Cloud;

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|step| *step == self).unwrap_or(0)
    }

    fn at(index: usize) -> Step {
        Self::ALL[index.min(Self::ALL.len() - 1)]
    }
}

pub(crate) fn parse_bytes(text: &str, alignment: u64) -> Result<u64, String> {
    let compact = text.split_whitespace().collect::<String>();
    let (number, multiplier) = if let Some(n) = compact.strip_suffix("GiB") {
        (n, 1u64 << 30)
    } else if let Some(n) = compact.strip_suffix("MiB") {
        (n, 1 << 20)
    } else if let Some(n) = compact.strip_suffix("KiB") {
        (n, 1 << 10)
    } else {
        (compact.as_str(), 1)
    };
    number
        .parse::<u64>()
        .ok()
        .and_then(|n| n.checked_mul(multiplier))
        .filter(|n| *n > 0 && *n <= i64::MAX as u64 && n % alignment == 0)
        .ok_or_else(|| {
            format!(
                "Enter a positive capacity in {alignment}-byte increments, as bytes or a whole number with KiB, MiB or GiB."
            )
        })
}

pub(crate) fn resources(form: &Editor) -> Result<(u64, u64), String> {
    let cpus = form
        .vcpus
        .trim()
        .parse::<u64>()
        .ok()
        .filter(|n| (1..=256).contains(n))
        .ok_or("Enter 1–256 virtual CPUs; the host validates its supported limit.")?;
    let memory = parse_bytes(&form.memory, 1024)?;
    let minimum = form
        .initial
        .details
        .vms
        .get(&form.source.id)
        .and_then(|details| details.memory_static_min)
        .unwrap_or(0)
        .max(16 << 20);
    if memory < minimum {
        return Err(format!(
            "Memory must be at least {} for this template.",
            storage_bytes(Some(minimum))
        ));
    }
    Ok((cpus, memory))
}

/// "Next": the step that is left validates what it owns (name and
/// description; virtual CPUs and memory). Repository and network are
/// checked by the review only, as in Rune.
pub(crate) fn next(m: &mut Model) {
    let Some(form) = edit::editor_mut(m) else {
        return;
    };
    let valid = match form.step {
        Step::Name => edit::validate_name(&form.name)
            .and_then(|_| edit::validate_description(&form.description))
            .map(|_| ()),
        Step::Resources => resources(form).map(|_| ()),
        _ => Ok(()),
    };
    match valid {
        Ok(()) => {
            form.step = Step::at(form.step.index() + 1);
            form.error = None;
        }
        Err(error) => form.error = Some(error),
    }
}

/// "Back" on a step after the first.
pub(crate) fn previous(m: &mut Model) {
    if let Some(form) = edit::editor_mut(m) {
        form.step = Step::at(form.step.index().saturating_sub(1));
        form.error = None;
    }
}

pub(crate) fn operation(form: &Editor) -> Result<VmLifecycleOperation, String> {
    let (vcpus, memory) = resources(form)?;
    if form.cloud {
        form.cloud_input.validate()?;
    }
    Ok(VmLifecycleOperation::Create {
        name: edit::validate_name(&form.name)?,
        description: edit::validate_description(&form.description)?,
        sr: form.sr.ok_or(edit::NO_SR)?,
        network: form.network.ok_or("Select a network.")?,
        iso: form.iso,
        vcpus,
        memory,
        disk_size: if form.disk_size.trim().is_empty() {
            None
        } else {
            Some(parse_bytes(&form.disk_size, 512)?)
        },
        firmware: form.firmware,
        bios_host: form.bios_host,
    })
}

/// The data disks of the template, from the captured inventory.
pub(crate) fn inherited_disks(form: &Editor) -> Vec<VdiId> {
    form.initial
        .details
        .vbds
        .values()
        .filter(|vbd| vbd.vm == Some(form.source.id) && vbd.kind == "Disk")
        .filter_map(|vbd| vbd.vdi)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// The template advertises a boot policy that can be configured at all.
pub(crate) fn configurable(form: &Editor) -> bool {
    form.initial
        .details
        .vms
        .get(&form.source.id)
        .is_some_and(|d| {
            d.domain_type.as_deref() == Some("hvm")
                || d.boot_policy.as_deref() == Some("BIOS order")
        })
}

/// Installed template disks keep their mode: another firmware is offered
/// only for a template without disks, or for the mode it already has.
pub(crate) fn firmware_allowed(form: &Editor, mode: GuestFirmware) -> bool {
    let requested = if mode == GuestFirmware::Bios {
        "bios"
    } else {
        "uefi"
    };
    inherited_disks(form).is_empty()
        || form
            .initial
            .details
            .vms
            .get(&form.source.id)
            .and_then(|d| d.firmware.as_deref())
            == Some(requested)
}

pub(crate) fn bios_eligible(form: &Editor) -> bool {
    let Some(d) = form.initial.details.vms.get(&form.source.id) else {
        return false;
    };
    d.bios_identity_empty == Some(true)
        && (d.domain_type.as_deref() == Some("hvm")
            || d.boot_policy.as_deref() == Some("BIOS order"))
        && (form.firmware == GuestFirmware::Bios
            || form.firmware == GuestFirmware::Inherited && d.firmware.as_deref() == Some("bios"))
}

/// Chooses a firmware; the copied BIOS identity goes with the old one. A
/// mode the template cannot have changes nothing.
pub(crate) fn choose_firmware(m: &mut Model, value: GuestFirmware) {
    let Some(form) = edit::editor_mut(m) else {
        return;
    };
    if value != GuestFirmware::Inherited && (!configurable(form) || !firmware_allowed(form, value))
    {
        return;
    }
    form.firmware = value;
    form.bios_host = None;
}

/// Turns the cloud-init configuration on or off. Off discards the draft
/// (and zeroizes it).
pub(crate) fn toggle_cloud(m: &mut Model) {
    if let Some(form) = edit::editor_mut(m) {
        form.cloud = !form.cloud;
        if !form.cloud {
            form.cloud_input = super::cloud::Draft::default();
        }
    }
}

/// The review lines of a creation whose operation was built. The payload of
/// the cloud-init inputs is never among them: byte counts only.
pub(crate) fn review(form: &Editor) -> Vec<String> {
    let (cpus, memory) = resources(form).expect("creation was validated");
    let sr = form.sr.expect("creation storage selected");
    let network = form.network.expect("creation network selected");
    let mut lines = vec![
        format!(
            "Create VM “{}” from template “{}”.",
            form.name.trim(),
            form.source.name
        ),
        format!("Description: {}", form.description),
        format!(
            "Resources: {cpus} virtual CPUs; {} memory; {:?} firmware.",
            storage_bytes(Some(memory)),
            form.firmware
        ),
        format!(
            "Destination storage: {}",
            edit::storage_label(&form.initial, sr)
        ),
        format!(
            "Network for all interfaces: {}",
            edit::network_label(&form.initial, network)
        ),
    ];
    if let Some(host) = form.bios_host {
        lines.push(format!(
            "Copy BIOS identity from {} ({host}) and set its home-host preference to that host before provisioning. This does not guarantee placement.",
            form.initial
                .hosts
                .get(&host)
                .map_or("Unknown host", |h| h.name.as_str())
        ));
    }
    if form.firmware == GuestFirmware::SecureUefi {
        lines.push("Request Secure Boot policy. This is not proof of guest enforcement; inspect guest/pool certificate readiness after creation.".into());
    }
    let disks = inherited_disks(form);
    if disks.is_empty() {
        lines.push("Clone the diskless template and provision it without inherited disk-layout instructions.".into());
    }
    for disk in disks {
        lines.push(format!(
            "Full copy of template disk: {}",
            edit::disk_label(&form.initial, disk)
        ));
    }
    if !form.disk_size.trim().is_empty() {
        let size = parse_bytes(&form.disk_size, 512).expect("disk size validated");
        lines.push(format!(
            "Create an additional blank disk: {size} bytes ({}).",
            storage_bytes(Some(size))
        ));
    }
    lines.push(
        form.iso
            .map(|id| {
                format!(
                    "Replace the first CD drive's media with {}. A CD drive is created if needed.",
                    edit::disk_label(&form.initial, id)
                )
            })
            .unwrap_or_else(|| {
                "Retain inherited installation media; no ISO replacement was selected.".into()
            }),
    );
    if form.cloud {
        lines.push(format!(
            "Cloud-init: hostname {}; {} bytes user-data; {} bytes network-config. After successful creation, review a new NoCloud drive on the selected SR for the exact created VM. Closing the wizard discards this unsaved configuration; it does not cancel creation.",
            form.cloud_input.hostname.trim(),
            form.cloud_input.user_data.len(),
            form.cloud_input.network.len()
        ));
        lines.push("User-data contents are not included in the creation's entry on the Events tab of the Notifications view. Configuration is a separately reviewed, tracked operation; neither stage starts the guest.".into());
    }
    lines.push("The new VM stays halted. Each completed step and created VM/disk/interface UUID remains on the Events tab of the Notifications view if a later step fails. Uncertain operations are not automatically repeated.".into());
    lines.push("Other changes in this pool are blocked while creation is active or its outcome is unknown. Resolve or explicitly acknowledge an unknown outcome to release this block.".into());
    lines
}
