//! The tab bodies: Storage and Networking of a VM or template, Storage of a
//! repository. The information of the Rune Devices page (`vm_devices.rs`)
//! and of the list of the Rune "Virtual disks" workspace.
//!
//! Keys. The Rune page had cards and no keys of its own; the grids keep the
//! table keys of the information-only tabs of `general` that they replace
//! (`storage-vbds`, `networking-vifs-table`, `storage-pbds`), and
//! `disk-search` is the Rune search field. New keys: `devices-stale`,
//! `devices-unavailable-<class>`, `vm-disk-details`, `vm-nic-details`,
//! `device-history`, `device-operation-<id>`, `device-recheck-<id>`,
//! `guest-tools`, `storage-attachments`, `storage-disks`,
//! `repository-disks`, `disk-count`.

use xenadmin_core::{
    ClassAvailability, Inventory, OperationAction, OperationControl, OperationState, PoolId, SrId,
    VbdId, VifId, VirtualBlockDevice, VirtualInterface, VmId,
};

use super::actions::{self, RECONNECT};
use super::commands;
use crate::prelude::*;

pub const STALE: &str =
    "Inventory is stale. Device details show the last observed state; reconnect to refresh them.";

pub fn tab(m: &Model, tab: TabId, scope: &Target) -> Option<TabBody> {
    let object = scope.single()?;
    let pool = object.pool?;
    let inventory = m.inventory_for(pool)?;
    let guest = || {
        object
            .vm_id()
            .filter(|vm| inventory.vms.contains_key(vm))
    };
    let blocks = match (tab, object.kind) {
        (TabId::Storage, ObjectKind::Vm | ObjectKind::Template) => {
            vm_storage(m, pool, &inventory, guest()?)
        }
        (TabId::Networking, ObjectKind::Vm | ObjectKind::Template) => {
            vm_networking(m, pool, &inventory, guest()?)
        }
        (TabId::Storage, ObjectKind::StorageRepository) => {
            let uuid = object
                .uuid()
                .filter(|uuid| inventory.storage.iter().any(|sr| sr.uuid == *uuid))?;
            repository(m, pool, &inventory, SrId::from(uuid))
        }
        _ => return None,
    };
    Some(TabBody::Blocks(blocks))
}

pub fn boolean(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "Yes",
        Some(false) => "No",
        None => "Not reported",
    }
}

pub fn connected(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "Connected",
        Some(false) => "Disconnected",
        None => "Not reported",
    }
}

fn nonempty(value: &str) -> &str {
    if value.is_empty() {
        "Not reported"
    } else {
        value
    }
}

pub fn unavailable(inventory: &Inventory, class: &str, label: &str) -> Option<String> {
    class_message(
        inventory.details.classes.get(&class.to_ascii_lowercase()),
        label,
    )
}

pub fn class_message(availability: Option<&ClassAvailability>, label: &str) -> Option<String> {
    Some(match availability {
        Some(ClassAvailability::Available) => return None,
        Some(ClassAvailability::PermissionDenied) => {
            format!("{label} are unavailable: this account does not have permission to read them.")
        }
        Some(ClassAvailability::Unsupported) => {
            format!("{label} are not supported by this host's API.")
        }
        Some(ClassAvailability::InvalidData) => {
            format!("{label} could not be decoded from the host response.")
        }
        Some(ClassAvailability::Unknown) | None => {
            format!("{label} have not been reported by this connection.")
        }
    })
}

fn class_caption(inventory: &Inventory, class: &str, label: &str) -> Option<Text> {
    unavailable(inventory, class, label).map(|reason| {
        caption(reason).key(format!(
            "devices-unavailable-{}",
            class.to_ascii_lowercase()
        ))
    })
}

pub fn disk_rows(inventory: &Inventory, vm: VmId) -> Vec<&VirtualBlockDevice> {
    let mut rows: Vec<_> = inventory
        .details
        .vm_disks
        .get(&vm)
        .into_iter()
        .flatten()
        .filter_map(|id| inventory.details.vbds.get(id))
        .filter(|disk| disk.vm == Some(vm))
        .collect();
    rows.sort_by_key(|disk| {
        (
            disk.userdevice.parse::<u64>().unwrap_or(u64::MAX),
            disk.userdevice.as_str(),
        )
    });
    rows
}

pub fn interface_rows(inventory: &Inventory, vm: VmId) -> Vec<&VirtualInterface> {
    let mut rows: Vec<_> = inventory
        .details
        .vm_interfaces
        .get(&vm)
        .into_iter()
        .flatten()
        .filter_map(|id| inventory.details.vifs.get(id))
        .filter(|interface| interface.vm == Some(vm))
        .collect();
    rows.sort_by_key(|interface| {
        (
            interface.device.parse::<u64>().unwrap_or(u64::MAX),
            interface.device.as_str(),
        )
    });
    rows
}

fn stale(m: &Model, pool: PoolId) -> Option<Text> {
    let current = m.snapshot_for(pool).is_some_and(|s| s.is_current());
    (!m.is_synthetic_pool(pool) && !current).then(|| caption(STALE).key("devices-stale"))
}

fn sr_name(inventory: &Inventory, id: Option<SrId>) -> &str {
    id.and_then(|id| inventory.storage.iter().find(|sr| sr.uuid == id.uuid()))
        .map_or("Not reported", |sr| sr.name.as_str())
}

fn kind_label(attachment: &VirtualBlockDevice) -> &'static str {
    if actions::is_cd(attachment) {
        "CD/DVD drive"
    } else {
        "Disk"
    }
}

fn vm_storage(m: &Model, pool: PoolId, inventory: &Inventory, vm: VmId) -> Vec<Block> {
    let attachments = disk_rows(inventory, vm);
    let selected = m
        .areas
        .vm_devices
        .disk_row
        .filter(|id| attachments.iter().any(|a| a.id == *id));
    let rows = attachments.iter().map(|a| {
        let disk = a.vdi.and_then(|id| inventory.details.vdis.get(&id));
        let empty = actions::is_cd(a) && a.empty == Some(true);
        row(
            a.id,
            [
                nonempty(&a.userdevice).to_owned(),
                kind_label(a).to_owned(),
                if empty {
                    "No media inserted".to_owned()
                } else {
                    disk.map_or("Not reported", |d| nonempty(&d.name)).to_owned()
                },
                disk.map_or("Not reported", |d| sr_name(inventory, d.sr))
                    .to_owned(),
                disk.map_or_else(|| "Not reported".into(), |d| storage_bytes(d.virtual_size)),
                nonempty(&a.mode).to_owned(),
                boolean(a.bootable).to_owned(),
                connected(a.currently_attached).to_owned(),
            ],
        )
    });
    let selected_id = selected.map(|id| id.to_string());
    let grid = table(
        "storage-vbds",
        [
            column("position", "Position"),
            column("kind", "Kind"),
            column("name", "Name"),
            column("sr", "Storage repository"),
            column("size", "Virtual size").right(),
            column("mode", "Access mode"),
            column("bootable", "Bootable"),
            column("connection", "Connection"),
        ],
        rows,
    )
    .empty("No disk or CD/DVD attachments are reported for this VM.")
    .single(selected_id.as_deref(), |m, id| {
        m.areas.vm_devices.disk_row = VbdId::parse(id).ok();
    })
    .on_clear(|m| m.areas.vm_devices.disk_row = None)
    .row_menu(commands::DISK_ROW_MENU);
    let details = selected
        .and_then(|id| attachments.iter().find(|a| a.id == id))
        .map(|a| disk_details(inventory, a));
    blocks![
        stale(m, pool),
        match class_caption(inventory, "VBD", "Disk attachments") {
            Some(reason) => blocks![reason],
            None => blocks![grid, details],
        },
        history(m, pool, vm),
    ]
}

/// Everything the Rune card of an attachment showed.
fn disk_details(inventory: &Inventory, a: &VirtualBlockDevice) -> Group {
    let cd = actions::is_cd(a);
    let mut items = vec![
        property("Connection", connected(a.currently_attached)),
        property("Guest device", nonempty(&a.device)),
        property("Access mode", nonempty(&a.mode)),
        property("Bootable", boolean(a.bootable)),
    ];
    let mut note = None;
    if cd && a.empty == Some(true) {
        items.push(property("Media", "No media inserted"));
    } else if let Some(reason) = class_caption(inventory, "VDI", "Virtual disk details") {
        note = Some(reason);
    } else if let Some(disk) = a.vdi.and_then(|id| inventory.details.vdis.get(&id)) {
        let sharing = inventory
            .details
            .vdi_attachments
            .get(&disk.id)
            .map_or(0, Vec::len);
        items.extend([
            property(if cd { "Media" } else { "Disk name" }, nonempty(&disk.name)),
            property("Storage repository", sr_name(inventory, disk.sr)),
            property("Virtual size", storage_bytes(disk.virtual_size)),
            property("Physical usage", storage_bytes(disk.physical_utilisation)),
            property("Type", nonempty(&disk.kind)),
            property("Read only", boolean(disk.read_only)),
            property("Shareable", boolean(disk.sharable)),
            property("Missing from storage", boolean(disk.missing)),
            property("Attachment count", sharing.to_string()),
            property("Disk UUID", disk.id.to_string()).copyable(),
        ]);
        if !disk.description.is_empty() {
            items.push(property("Description", &disk.description));
        }
    } else {
        note = Some(caption(if cd {
            "The host has not reported the inserted media record."
        } else {
            "The backing disk record is unavailable."
        }));
    }
    items.push(property("Attachment UUID", a.id.to_string()).copyable());
    group(
        format!("{} · slot {}", kind_label(a), nonempty(&a.userdevice)),
        blocks![properties(items), note],
    )
    .key("vm-disk-details")
}

/// The limit of an interface as one cell.
fn qos_limit(interface: &VirtualInterface) -> String {
    match interface.qos_algorithm.as_str() {
        "" => "None".into(),
        "ratelimit" => interface
            .qos_parameters
            .get("kbps")
            .map_or_else(|| "ratelimit".into(), |kbps| format!("{kbps} KB/s")),
        other => other.into(),
    }
}

fn network_of<'a>(
    inventory: &'a Inventory,
    interface: &VirtualInterface,
) -> Option<&'a xenadmin_core::Network> {
    interface.network.and_then(|id| {
        inventory
            .networks
            .iter()
            .find(|network| network.uuid == id.uuid())
    })
}

fn vm_networking(m: &Model, pool: PoolId, inventory: &Inventory, vm: VmId) -> Vec<Block> {
    let interfaces = interface_rows(inventory, vm);
    let selected = m
        .areas
        .vm_devices
        .nic_row
        .filter(|id| interfaces.iter().any(|v| v.id == *id));
    let rows = interfaces.iter().map(|v| {
        row(
            v.id,
            [
                nonempty(&v.device).to_owned(),
                nonempty(&v.mac).to_owned(),
                network_of(inventory, v)
                    .map_or("Not reported", |network| network.name.as_str())
                    .to_owned(),
                connected(v.currently_attached).to_owned(),
                qos_limit(v),
            ],
        )
    });
    let selected_id = selected.map(|id| id.to_string());
    let grid = table(
        "networking-vifs-table",
        [
            column("device", "Device"),
            column("mac", "MAC address"),
            column("network", "Network"),
            column("connection", "Connection"),
            column("qos", "QoS limit"),
        ],
        rows,
    )
    .empty("No network interfaces are reported for this VM.")
    .single(selected_id.as_deref(), |m, id| {
        m.areas.vm_devices.nic_row = VifId::parse(id).ok();
    })
    .on_clear(|m| m.areas.vm_devices.nic_row = None)
    .row_menu(commands::NIC_ROW_MENU);
    let details = selected
        .and_then(|id| interfaces.iter().find(|v| v.id == id))
        .map(|v| nic_details(inventory, v));
    blocks![
        stale(m, pool),
        match class_caption(inventory, "VIF", "Network interfaces") {
            Some(reason) => blocks![reason],
            None => blocks![grid, details],
        },
        history(m, pool, vm),
        guest_tools(inventory, vm),
    ]
}

/// Everything the Rune card of an interface showed.
fn nic_details(inventory: &Inventory, v: &VirtualInterface) -> Group {
    let network = network_of(inventory, v);
    let mut items = vec![
        property("Connection", connected(v.currently_attached)),
        property(
            "Network",
            network.map_or("Not reported", |network| network.name.as_str()),
        ),
        property(
            "Bridge",
            network.map_or("Not reported", |network| network.bridge.as_str()),
        ),
        property("MAC address", nonempty(&v.mac)),
        property(
            "MTU",
            v.mtu
                .map_or_else(|| "Not reported".into(), |mtu| mtu.to_string()),
        ),
    ];
    let mut note = None;
    if let Some(network_id) = v.network {
        if let Some(details) = inventory.details.networks.get(&network_id) {
            items.push(property(
                "Default locking mode",
                details
                    .default_locking_mode
                    .as_deref()
                    .unwrap_or("Not reported"),
            ));
        }
        match class_caption(inventory, "PIF", "Physical uplinks") {
            Some(reason) => note = Some(reason),
            None => {
                let uplinks: Vec<_> = inventory
                    .details
                    .network_uplinks
                    .get(&network_id)
                    .into_iter()
                    .flatten()
                    .filter_map(|id| inventory.details.pifs.get(id))
                    .map(|pif| {
                        format!(
                            "{} on {}{}",
                            pif.device,
                            pif.host
                                .and_then(|id| inventory.hosts.get(&id))
                                .map_or("unknown host", |host| host.name.as_str()),
                            pif.vlan
                                .filter(|vlan| *vlan >= 0)
                                .map_or_else(String::new, |vlan| format!(" (VLAN {vlan})"))
                        )
                    })
                    .collect();
                items.push(property(
                    "Physical uplinks",
                    if uplinks.is_empty() {
                        "No physical uplinks reported".into()
                    } else {
                        uplinks.join(", ")
                    },
                ));
            }
        }
    }
    if !v.qos_algorithm.is_empty() {
        items.push(property("QoS algorithm", &v.qos_algorithm));
        for (name, value) in &v.qos_parameters {
            items.push(property(name, value));
        }
    }
    items.push(property("Interface UUID", v.id.to_string()).copyable());
    group(
        format!("NIC {}", nonempty(&v.device)),
        blocks![properties(items), note],
    )
    .key("vm-nic-details")
}

pub(super) fn outcome_state(state: &OperationState) -> OutcomeState {
    match state {
        OperationState::Succeeded => OutcomeState::Succeeded,
        OperationState::PartiallyCompleted => OutcomeState::Partial,
        OperationState::Failed | OperationState::Rejected => OutcomeState::Failed,
        OperationState::Cancelled => OutcomeState::Cancelled,
        OperationState::OutcomeUnknown | OperationState::AcknowledgedUnknown => {
            OutcomeState::Unknown
        }
        _ => OutcomeState::Running(None),
    }
}

/// "Recent device changes": the newest four device operations of the VM,
/// where they were started.
fn history(m: &Model, pool: PoolId, vm: VmId) -> Option<Group> {
    let records: Vec<Block> = m
        .operation_rows
        .iter()
        .rev()
        .filter(|record| {
            record.pool == pool
                && record.vm == vm
                && matches!(record.action, OperationAction::Device(_))
        })
        .take(4)
        .map(|record| {
            let id = record.id;
            let mut view = outcome(
                format!("device-operation-{id}"),
                outcome_state(&record.state),
                format!("{} • {:?}", record.action.label(), record.state),
            )
            .line(&record.detail)
            .lines(
                record
                    .created_devices
                    .iter()
                    .map(|device| format!("Created {} UUID: {}", device.label(), device.uuid())),
            )
            .lines(
                record
                    .steps
                    .iter()
                    .map(|step| format!("{} • {:?}: {}", step.label, step.state, step.detail)),
            );
            if record.state == OperationState::OutcomeUnknown && record.can_recheck() {
                view = view.action(
                    button(format!("device-recheck-{id}"), "Recheck outcome", move |m| {
                        m.control_operation_for(pool, id, OperationControl::Recheck);
                    })
                    .disabled_if(!m.can_mutate_pool(pool), RECONNECT),
                );
            }
            Block::Outcome(view)
        })
        .collect();
    (!records.is_empty()).then(|| group("Recent device changes", records).key("device-history"))
}

/// "Guest tools and reported addresses", at the foot of the Networking tab.
fn guest_tools(inventory: &Inventory, vm: VmId) -> Group {
    let title = "Guest tools and reported addresses";
    if let Some(reason) = class_caption(inventory, "VM_guest_metrics", "Guest tools details") {
        return group(title, blocks![reason]).key("guest-tools");
    }
    let guest = inventory
        .details
        .vms
        .get(&vm)
        .and_then(|details| details.guest_metrics)
        .and_then(|id| inventory.details.guest_metrics.get(&id));
    let Some(guest) = guest else {
        return group(
            title,
            blocks![caption(
                "No guest metrics record is reported. Guest tools may be absent or have not yet reported data."
            )],
        )
        .key("guest-tools");
    };
    let mut body = blocks![
        properties([
            ("Agent live", boolean(guest.live)),
            (
                "Last updated",
                guest.last_updated.as_deref().unwrap_or("Not reported"),
            ),
        ]),
        (guest.live != Some(true)).then(|| caption(
            "Guest tools are not reporting live status. OS details and addresses may be stale."
        )),
    ];
    for (label, values) in [
        ("Operating system", &guest.os_version),
        ("Tools version", &guest.tools_version),
        ("Reported addresses", &guest.networks),
    ] {
        body.push(heading(label));
        if values.is_empty() {
            body.push(Block::Text(caption("Not reported")));
        } else {
            body.push(properties(
                values.iter().map(|(name, value)| property(name, value)),
            ));
        }
    }
    group(title, body).key("guest-tools")
}

fn known(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "Yes",
        Some(false) => "No",
        None => "Unknown",
    }
}

/// The Rune storage hub's caption under an attachment that uses credentials.
pub const ATTACHMENT_CREDENTIALS: &str = "This attachment uses credentials. Repair missing connections can ask the host to clone supported credential references; manual attachment creation requires fresh credentials.";

/// The path counts of an attachment as the Rune storage hub wrote them, one
/// device after the other: "{device} · {active} / {maximum} paths".
fn paths(attachment: &xenadmin_core::StorageAttachment) -> String {
    attachment
        .paths
        .iter()
        .map(|path| format!("{} · {} / {} paths", path.device, path.active, path.maximum))
        .collect::<Vec<_>>()
        .join("; ")
}

/// The Storage tab of a repository: where it is attached (with what the
/// Rune storage hub showed for each attachment: attached, multipathing,
/// path counts, credentials), and its disks.
/// The disks are tree objects: the selection is the grid selection, and a
/// secondary press opens the context menu of the disk.
fn repository(m: &Model, pool: PoolId, inventory: &Inventory, sr: SrId) -> Vec<Block> {
    let attachments = inventory
        .details
        .pbds
        .values()
        .filter(|a| a.sr == Some(sr))
        .map(|a| {
            row(
                a.id,
                [
                    a.host
                        .map(|id| {
                            inventory
                                .hosts
                                .get(&id)
                                .map_or_else(|| id.to_string(), |host| host.name.clone())
                        })
                        .unwrap_or_else(|| "Unresolved host".into()),
                    a.id.to_string(),
                    known(a.currently_attached).to_owned(),
                    known(a.multipathed).to_owned(),
                    paths(a),
                    if a.requires_credentials { "Yes" } else { "No" }.to_owned(),
                ],
            )
        });
    // The Rune caption of each attachment that uses credentials, once.
    let credentials = inventory
        .details
        .pbds
        .values()
        .any(|a| a.sr == Some(sr) && a.requires_credentials)
        .then(|| caption(ATTACHMENT_CREDENTIALS).key("storage-attachment-credentials"));
    let query = m.areas.vm_devices.disk_search.to_lowercase();
    let mut disks: Vec<_> = inventory
        .details
        .vdis
        .values()
        .filter(|d| {
            d.sr == Some(sr)
                && (d.name.to_lowercase().contains(&query) || d.id.to_string().contains(&query))
        })
        .collect();
    disks.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
    let count = disks.len();
    let rows = disks.iter().map(|d| {
        row(
            d.id,
            [
                nonempty(&d.name).to_owned(),
                storage_bytes(d.virtual_size),
                storage_bytes(d.physical_utilisation),
                nonempty(&d.kind).to_owned(),
                inventory
                    .details
                    .vdi_attachments
                    .get(&d.id)
                    .map_or(0, Vec::len)
                    .to_string(),
                d.id.to_string(),
            ],
        )
        .object(ObjectRef::new(Some(pool), ObjectKind::VirtualDisk, d.id))
    });
    let selected = m
        .selection
        .grid
        .single_of(ObjectKind::VirtualDisk)
        .map(|object| object.id.clone())
        .filter(|id| disks.iter().any(|d| d.id.to_string() == *id));
    let grid = table(
        "repository-disks",
        [
            column("name", "Name"),
            column("size", "Virtual size").right(),
            column("used", "Physical usage").right(),
            column("type", "Type"),
            column("attachments", "Attachments").right(),
            column("uuid", "UUID"),
        ],
        rows,
    )
    .empty("No virtual disk of this repository is in the readable inventory.")
    .single(selected.as_deref(), move |m, id| {
        m.select_in_grid(Target::one(ObjectRef::new(
            Some(pool),
            ObjectKind::VirtualDisk,
            id,
        )));
    })
    .on_clear(|m| m.select_in_grid(Target::none()));
    blocks![
        group(
            "Host attachments",
            blocks![
                class_caption(inventory, "PBD", "Host attachments"),
                table(
                    "storage-pbds",
                    [
                        column("name", "Server"),
                        column("uuid", "Attachment UUID"),
                        column("attached", "Attached"),
                        column("multipathing", "Multipathing"),
                        column("paths", "Paths"),
                        column("credentials", "Uses credentials"),
                    ],
                    attachments,
                )
                .empty("No host attachment of this repository is in the readable inventory."),
                credentials,
            ],
        )
        .key("storage-attachments"),
        group(
            "Virtual disks",
            blocks![
                class_caption(inventory, "VDI", "Virtual disks"),
                input(
                    "disk-search",
                    "Search",
                    &m.areas.vm_devices.disk_search,
                    |m, text| m.areas.vm_devices.disk_search = text.to_owned(),
                )
                .placeholder("Search disk name or UUID"),
                caption(format!("{count} disks on selected storage")).key("disk-count"),
                grid,
            ],
        )
        .key("storage-disks"),
    ]
}
