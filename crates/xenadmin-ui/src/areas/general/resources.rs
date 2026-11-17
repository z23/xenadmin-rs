//! What the Rune details pane appended for a server, a storage repository
//! and a network (`resource_details.rs`), spread over the tabs it belongs
//! to: the scalar fields are sections of General, storage attachments and
//! virtual disks are the information-only Storage tab, uplinks and VM
//! interfaces the information-only Networking tab, a server's interfaces the
//! information-only NICs tab. The areas that own those tabs replace these
//! bodies with grids and actions; until then this is what is shown.
//!
//! An unknown value is "Unknown", never "No"; an inventory class that could
//! not be read says so above the list it would have filled.

use xenadmin_core::{
    ClassAvailability, HostId, Inventory, NetworkId, PhysicalInterface, SrId, VdiId, VmId,
};

use super::information::{View, known};
use crate::prelude::*;

fn supplied(value: &str) -> &str {
    if value.is_empty() {
        "Not supplied"
    } else {
        value
    }
}

fn host_name(inventory: &Inventory, id: Option<HostId>) -> String {
    id.map(|id| {
        inventory
            .hosts
            .get(&id)
            .map(|host| host.name.clone())
            .unwrap_or_else(|| id.to_string())
    })
    .unwrap_or_else(|| "Unresolved host".into())
}

fn sr_name(inventory: &Inventory, id: Option<SrId>) -> String {
    id.map(|id| {
        inventory
            .storage
            .iter()
            .find(|sr| sr.uuid == id.uuid())
            .map(|sr| sr.name.clone())
            .unwrap_or_else(|| id.to_string())
    })
    .unwrap_or_else(|| "Unresolved SR".into())
}

fn network_name(inventory: &Inventory, id: Option<NetworkId>) -> String {
    id.map(|id| {
        inventory
            .networks
            .iter()
            .find(|n| n.uuid == id.uuid())
            .map(|n| n.name.clone())
            .unwrap_or_else(|| id.to_string())
    })
    .unwrap_or_else(|| "Unresolved network".into())
}

fn vm_name(inventory: &Inventory, id: Option<VmId>) -> Option<String> {
    id.map(|vm| {
        inventory
            .vms
            .get(&vm)
            .map(|v| v.name.clone())
            .unwrap_or_else(|| vm.to_string())
    })
}

/// "{class}: {message}" unless the class is available.
fn class_notice(inventory: &Inventory, class: &str) -> Option<Notice> {
    let state = inventory.details.classes.get(&class.to_ascii_lowercase());
    let message = match state {
        Some(ClassAvailability::Available) => return None,
        Some(ClassAvailability::PermissionDenied) => {
            "Your account cannot read this inventory class"
        }
        Some(ClassAvailability::Unsupported) => "The host does not support this inventory class",
        Some(ClassAvailability::InvalidData) => "This inventory class could not be decoded",
        _ => "Availability of this inventory class is unknown",
    };
    Some(
        notice(NoticeKind::Warning, format!("{class}: {message}"))
            .key(format!("class-notice-{}", class.to_ascii_lowercase())),
    )
}

fn missing(what: &str) -> Vec<Block> {
    blocks![
        caption(format!("This {what} is no longer present in the inventory"))
            .key("general-missing")
    ]
}

fn titled(key: &str, title: &str, blocks: Vec<Block>) -> Group {
    group(title, blocks).key(key)
}

// -- General: storage repository, network, virtual disk, saved connection ----------

pub(super) fn storage_general(m: &Model, view: &View, object: &ObjectRef) -> Vec<Block> {
    let i = view.inventory;
    let Some(uuid) = object.uuid() else {
        return missing("storage repository");
    };
    let Some(repository) = i.storage.iter().find(|sr| sr.uuid == uuid) else {
        return missing("storage repository");
    };
    let id = SrId::from(uuid);
    let row = m.object_row(object);
    let mut items = vec![
        property("Name", &repository.name),
        property("UUID", uuid.to_string()).copyable(),
        property("Type", &repository.kind),
        property(
            "Used",
            row.as_ref()
                .map(|row| row.location.clone())
                .unwrap_or_else(|| "Usage unknown".into()),
        ),
        property("Capacity", storage_bytes(repository.capacity_bytes)),
    ];
    if let Some(details) = i.details.storage.get(&id) {
        items.extend([
            property("Content type", supplied(&details.content_type)),
            property("Shared", known(details.shared)),
            property(
                "Virtual allocation",
                storage_bytes(details.virtual_allocation),
            ),
            property("Tags", supplied(&details.tags.join(", "))),
        ]);
    }
    items.push(view.freshness());
    blocks![titled(
        "general-summary",
        "General",
        vec![properties(items)]
    )]
}

pub(super) fn network_general(view: &View, object: &ObjectRef) -> Vec<Block> {
    let i = view.inventory;
    let Some(uuid) = object.uuid() else {
        return missing("network");
    };
    let Some(network) = i.networks.iter().find(|n| n.uuid == uuid) else {
        return missing("network");
    };
    let mut items = vec![
        property("Name", &network.name),
        property("UUID", uuid.to_string()).copyable(),
        property("Bridge", &network.bridge),
    ];
    if let Some(details) = i.details.networks.get(&NetworkId::from(uuid)) {
        items.extend([
            property(
                "MTU",
                details
                    .mtu
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "Unknown".into()),
            ),
            property("Managed", known(details.managed)),
            property(
                "Default locking mode",
                details.default_locking_mode.as_deref().unwrap_or("Unknown"),
            ),
            property("Tags", supplied(&details.tags.join(", "))),
        ]);
    }
    items.push(view.freshness());
    blocks![titled(
        "general-summary",
        "General",
        vec![properties(items)]
    )]
}

pub(super) fn disk_general(view: &View, object: &ObjectRef) -> Vec<Block> {
    let i = view.inventory;
    let Some(disk) = object
        .uuid()
        .and_then(|uuid| i.details.vdis.get(&VdiId::from(uuid)))
    else {
        return missing("virtual disk");
    };
    let mut items = vec![
        property("Name", supplied(&disk.name)),
        property("Disk UUID", disk.id.to_string()).copyable(),
        property("Description", supplied(&disk.description)),
        property("Storage repository", sr_name(i, disk.sr)),
        property(
            "Virtual / allocated size",
            format!(
                "{} / {}",
                storage_bytes(disk.virtual_size),
                storage_bytes(disk.physical_utilisation)
            ),
        ),
        property(
            "Type / read-only / shared / missing",
            format!(
                "{} / {} / {} / {}",
                disk.kind,
                known(disk.read_only),
                known(disk.sharable),
                known(disk.missing)
            ),
        ),
    ];
    for attachment in disk_attachments(i, disk.id) {
        items.push(property("Attachment", attachment));
    }
    items.push(view.freshness());
    blocks![
        class_notice(i, "VDI"),
        titled("general-summary", "General", vec![properties(items)]),
    ]
}

/// A saved connection that is not showing a pool: what the Home tab lists
/// for it.
pub(super) fn connection_general(m: &Model, object: &ObjectRef) -> Vec<Block> {
    let Some(id) = crate::session::profile_id(&object.id) else {
        return missing("connection");
    };
    let Some(profile) = m.workspace.profile(id) else {
        return missing("connection");
    };
    let status = crate::session::sessions(m)
        .into_iter()
        .find(|view| view.profile == id)
        .map(|view| view.status)
        .unwrap_or("Disconnected");
    blocks![titled(
        "general-summary",
        "General",
        vec![properties([
            property("Name", &profile.name),
            property("Address", profile.endpoint.to_string()).copyable(),
            property("User name", &profile.username),
            property("Status", status),
            property("Read-only", known(Some(profile.read_only))),
        ])]
    )]
}

// -- Storage: attachments and virtual disks -----------------------------------

fn attachment_columns(first: &str) -> Vec<Column> {
    vec![
        column("name", first),
        column("uuid", "Attachment UUID"),
        column("attached", "Attached"),
    ]
}

/// The storage attachments (PBDs) of a server: one row per repository.
pub(super) fn host_storage(view: &View, id: HostId) -> Vec<Block> {
    let i = view.inventory;
    if !i.hosts.contains_key(&id) {
        return missing("host");
    }
    let rows = i
        .details
        .pbds
        .values()
        .filter(|a| a.host == Some(id))
        .map(|a| {
            row(
                a.id,
                [
                    sr_name(i, a.sr),
                    a.id.to_string(),
                    known(a.currently_attached).to_owned(),
                ],
            )
        });
    blocks![titled(
        "storage-attachments",
        "Storage attachments",
        blocks![
            class_notice(i, "PBD"),
            table(
                "storage-pbds",
                attachment_columns("Storage repository"),
                rows
            )
            .empty("No storage attachment of this server is in the readable inventory."),
        ]
    )]
}

/// The host attachments and the virtual disks of a storage repository.
pub(super) fn repository_storage(view: &View, object: &ObjectRef) -> Vec<Block> {
    let i = view.inventory;
    let Some(uuid) = object
        .uuid()
        .filter(|uuid| i.storage.iter().any(|sr| sr.uuid == *uuid))
    else {
        return missing("storage repository");
    };
    let id = SrId::from(uuid);
    let attachments = i
        .details
        .pbds
        .values()
        .filter(|a| a.sr == Some(id))
        .map(|a| {
            row(
                a.id,
                [
                    host_name(i, a.host),
                    a.id.to_string(),
                    known(a.currently_attached).to_owned(),
                ],
            )
        });
    blocks![
        titled(
            "storage-attachments",
            "Host attachments",
            blocks![
                class_notice(i, "PBD"),
                table("storage-pbds", attachment_columns("Server"), attachments)
                    .empty("No host attachment of this repository is in the readable inventory."),
            ]
        ),
        titled(
            "storage-disks",
            "Virtual disks",
            blocks![
                class_notice(i, "VDI"),
                disk_table(i, i.details.vdis.values().filter(|disk| disk.sr == Some(id)))
                    .empty("No virtual disk of this repository is in the readable inventory."),
            ]
        ),
    ]
}

/// The repositories of a pool with their capacity.
pub(super) fn pool_storage(view: &View) -> Vec<Block> {
    let i = view.inventory;
    let mut repositories: Vec<_> = i.storage.iter().collect();
    repositories.sort_by(|a, b| natural_cmp(&a.name, &b.name).then_with(|| a.uuid.cmp(&b.uuid)));
    let rows = repositories.into_iter().map(|repository| {
        let details = i.details.storage.get(&SrId::from(repository.uuid));
        row(
            repository.uuid,
            [
                repository.name.clone(),
                repository.kind.clone(),
                known(details.and_then(|d| d.shared)).to_owned(),
                storage_bytes(repository.used_bytes),
                storage_bytes(repository.capacity_bytes),
            ],
        )
    });
    blocks![titled(
        "storage-repositories",
        "Storage repositories",
        blocks![
            table(
                "storage-repositories-table",
                [
                    column("name", "Name"),
                    column("type", "Type"),
                    column("shared", "Shared"),
                    column("used", "Used").right(),
                    column("capacity", "Capacity").right(),
                ],
                rows
            )
            .empty("No storage repository of this pool is in the readable inventory."),
        ]
    )]
}

/// The virtual disks attached to a VM or a template.
pub(super) fn vm_storage(view: &View, vm: VmId) -> Vec<Block> {
    let i = view.inventory;
    if !i.vms.contains_key(&vm) {
        return missing("VM");
    }
    let rows = i
        .details
        .vbds
        .values()
        .filter(|vbd| vbd.vm == Some(vm))
        .map(|vbd| {
            let disk = vbd.vdi.and_then(|id| i.details.vdis.get(&id));
            row(
                vbd.id,
                [
                    disk.map(|disk| supplied(&disk.name).to_owned())
                        .unwrap_or_else(|| {
                            if vbd.empty == Some(true) {
                                "Empty drive".into()
                            } else {
                                "Unresolved disk".into()
                            }
                        }),
                    vbd.userdevice.clone(),
                    vbd.kind.clone(),
                    vbd.mode.clone(),
                    disk.map(|disk| storage_bytes(disk.virtual_size))
                        .unwrap_or_else(|| "Unknown".into()),
                    disk.map(|disk| sr_name(i, disk.sr))
                        .unwrap_or_else(|| "Unknown".into()),
                    known(vbd.currently_attached).to_owned(),
                ],
            )
        });
    blocks![titled(
        "storage-disks",
        "Virtual disks",
        blocks![
            class_notice(i, "VBD"),
            class_notice(i, "VDI"),
            table(
                "storage-vbds",
                [
                    column("name", "Disk"),
                    column("device", "Device"),
                    column("type", "Type"),
                    column("mode", "Mode"),
                    column("size", "Virtual size").right(),
                    column("sr", "Storage repository"),
                    column("attached", "Attached"),
                ],
                rows
            )
            .empty("No virtual disk of this VM is in the readable inventory."),
        ]
    )]
}

fn disk_attachments(i: &Inventory, disk: VdiId) -> Vec<String> {
    i.details
        .vdi_attachments
        .get(&disk)
        .into_iter()
        .flatten()
        .filter_map(|id| i.details.vbds.get(id))
        .map(|attachment| {
            let consumer = vm_name(i, attachment.vm)
                .or_else(|| {
                    attachment
                        .snapshot
                        .map(|snapshot| format!("Snapshot {snapshot}"))
                })
                .unwrap_or_else(|| "Unresolved consumer".into());
            format!(
                "{consumer} · device {} · {} · attached: {}",
                attachment.userdevice,
                attachment.mode,
                known(attachment.currently_attached)
            )
        })
        .collect()
}

fn disk_table<'a>(
    i: &Inventory,
    disks: impl Iterator<Item = &'a xenadmin_core::VirtualDisk>,
) -> Table {
    let rows = disks.map(|disk| {
        let attachments = disk_attachments(i, disk.id);
        row(
            disk.id,
            [
                supplied(&disk.name).to_owned(),
                disk.id.to_string(),
                storage_bytes(disk.virtual_size),
                storage_bytes(disk.physical_utilisation),
                disk.kind.clone(),
                known(disk.read_only).to_owned(),
                known(disk.sharable).to_owned(),
                known(disk.missing).to_owned(),
                if attachments.is_empty() {
                    "None".to_owned()
                } else {
                    attachments.join("; ")
                },
            ],
        )
    });
    table(
        "storage-vdis",
        [
            column("name", "Name"),
            column("uuid", "Disk UUID"),
            column("virtual", "Virtual size").right(),
            column("allocated", "Allocated size").right(),
            column("type", "Type"),
            column("read-only", "Read-only"),
            column("shared", "Shared"),
            column("missing", "Missing"),
            column("attachments", "Attachments"),
        ],
        rows,
    )
}

// -- Networking and NICs ---------------------------------------------------------

/// The Rune `pif` block: one titled section per physical interface.
fn pif(inventory: &Inventory, interface: &PhysicalInterface) -> Group {
    let mut items = vec![
        property("Interface UUID", interface.id.to_string()).copyable(),
        property("MAC", supplied(&interface.mac)),
        property(
            "Attached / management / physical",
            format!(
                "{} / {} / {}",
                known(interface.currently_attached),
                known(interface.management),
                known(interface.physical)
            ),
        ),
        property(
            "MTU / VLAN",
            format!(
                "{} / {}",
                interface
                    .mtu
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "Unknown".into()),
                interface
                    .vlan
                    .map(|n| if n < 0 {
                        "Untagged".into()
                    } else {
                        n.to_string()
                    })
                    .unwrap_or_else(|| "Unknown".into())
            ),
        ),
        property(
            "IPv4 mode / address",
            format!(
                "{} / {}",
                supplied(&interface.ip_configuration_mode),
                supplied(&interface.ip)
            ),
        ),
        property(
            "Netmask / gateway",
            format!(
                "{} / {}",
                supplied(&interface.netmask),
                supplied(&interface.gateway)
            ),
        ),
        property("DNS", supplied(&interface.dns)),
        property(
            "IPv6 mode / addresses",
            format!(
                "{} / {}",
                supplied(&interface.ipv6_configuration_mode),
                supplied(&interface.ipv6.join(", "))
            ),
        ),
    ];
    if !interface.ipv6_gateway.is_empty() {
        items.push(property("IPv6 gateway", &interface.ipv6_gateway));
    }
    let mut notes: Vec<Text> = Vec::new();
    let unavailable = class_notice(inventory, "PIF_metrics");
    if let Some(notice) = &unavailable {
        items.push(property("PIF_metrics", notice.text.clone()).tone(Tone::Warning));
    }
    if let Some(metrics) = interface
        .metrics
        .and_then(|id| inventory.details.pif_metrics.get(&id))
    {
        items.extend([
            property("Link carrier", known(metrics.carrier)),
            property(
                "Link speed",
                metrics
                    .speed_mbps
                    .map(|v| format!("{v} Mbit/s"))
                    .unwrap_or_else(|| "Not reported".into()),
            ),
            property("Full duplex", known(metrics.full_duplex)),
            property(
                "NIC vendor / device",
                format!(
                    "{} ({}) / {} ({})",
                    supplied(&metrics.vendor_name),
                    supplied(&metrics.vendor_id),
                    supplied(&metrics.device_name),
                    supplied(&metrics.device_id)
                ),
            ),
            property("PCI bus path", supplied(&metrics.pci_bus_path)),
            property(
                "Link metrics updated",
                metrics.last_updated.as_deref().unwrap_or("Unknown"),
            ),
        ]);
    } else {
        notes.push(caption(
            "No link metrics record is available for this interface",
        ));
    }
    items.push(property(
        "Interface capabilities",
        interface
            .capabilities
            .as_ref()
            .map(|v| {
                if v.is_empty() {
                    "None advertised".into()
                } else {
                    v.join(", ")
                }
            })
            .unwrap_or_else(|| "Unknown".into()),
    ));
    if interface.network.is_some() {
        items.push(property(
            "Network",
            network_name(inventory, interface.network),
        ));
    }
    for bond in inventory
        .details
        .bonds
        .values()
        .filter(|bond| bond.master == Some(interface.id) || bond.slaves.contains(&interface.id))
    {
        let members = bond
            .slaves
            .iter()
            .map(|id| {
                inventory
                    .details
                    .pifs
                    .get(id)
                    .map(|p| p.device.clone())
                    .unwrap_or_else(|| id.to_string())
            })
            .collect::<Vec<_>>()
            .join(", ");
        items.push(property(
            "Bond",
            format!(
                "{} · {} · members: {} · links up: {}",
                bond.id,
                supplied(&bond.mode),
                supplied(&members),
                bond.links_up
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "Unknown".into())
            ),
        ));
    }
    let mut blocks = vec![properties(items)];
    blocks.extend(notes.into_iter().map(Block::from));
    group(
        format!(
            "{} · {}",
            interface.device,
            host_name(inventory, interface.host)
        ),
        blocks,
    )
    .key(format!("pif-{}", interface.id))
}

fn pifs<'a>(
    inventory: &Inventory,
    interfaces: impl Iterator<Item = &'a PhysicalInterface>,
    empty: &str,
) -> Vec<Block> {
    let mut out: Vec<Block> = blocks![class_notice(inventory, "PIF")];
    let before = out.len();
    out.extend(interfaces.map(|interface| Block::from(pif(inventory, interface))));
    if out.len() == before {
        out.push(caption(empty).key("pifs-empty").into());
    }
    out
}

/// NICs of a server: every physical interface with its link metrics and
/// bonds (the Rune "Host interfaces").
pub(super) fn host_nics(view: &View, id: HostId) -> Vec<Block> {
    let i = view.inventory;
    if !i.hosts.contains_key(&id) {
        return missing("host");
    }
    blocks![titled(
        "nics-interfaces",
        "Host interfaces",
        pifs(
            i,
            i.details.pifs.values().filter(|p| p.host == Some(id)),
            "No interface of this server is in the readable inventory."
        )
    )]
}

fn uplink_columns(first: &str) -> Vec<Column> {
    vec![
        column("name", first),
        column("device", "Device"),
        column("vlan", "VLAN"),
        column("management", "Management"),
        column("mode", "IPv4 mode"),
        column("address", "IPv4 address"),
        column("attached", "Attached"),
    ]
}

fn uplink_row(first: String, interface: &PhysicalInterface) -> Row {
    row(
        interface.id,
        [
            first,
            interface.device.clone(),
            interface
                .vlan
                .map(|n| {
                    if n < 0 {
                        "Untagged".into()
                    } else {
                        n.to_string()
                    }
                })
                .unwrap_or_else(|| "Unknown".into()),
            known(interface.management).to_owned(),
            supplied(&interface.ip_configuration_mode).to_owned(),
            supplied(&interface.ip).to_owned(),
            known(interface.currently_attached).to_owned(),
        ],
    )
}

/// Networking of a server: the networks its interfaces reach, with the
/// address configuration of each. Link details are on the NICs tab.
pub(super) fn host_networking(view: &View, id: HostId) -> Vec<Block> {
    let i = view.inventory;
    if !i.hosts.contains_key(&id) {
        return missing("host");
    }
    let rows = i
        .details
        .pifs
        .values()
        .filter(|p| p.host == Some(id))
        .map(|p| uplink_row(network_name(i, p.network), p));
    blocks![titled(
        "networking-networks",
        "Networks of this server",
        blocks![
            class_notice(i, "PIF"),
            table("networking-uplinks", uplink_columns("Network"), rows)
                .empty("No interface of this server is in the readable inventory."),
        ]
    )]
}

/// Networking of a pool: its networks.
pub(super) fn pool_networking(view: &View) -> Vec<Block> {
    let i = view.inventory;
    let mut networks: Vec<_> = i.networks.iter().collect();
    networks.sort_by(|a, b| natural_cmp(&a.name, &b.name).then_with(|| a.uuid.cmp(&b.uuid)));
    let rows = networks.into_iter().map(|network| {
        let id = NetworkId::from(network.uuid);
        let details = i.details.networks.get(&id);
        row(
            network.uuid,
            [
                network.name.clone(),
                network.bridge.clone(),
                details
                    .and_then(|d| d.mtu)
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "Unknown".into()),
                known(details.and_then(|d| d.managed)).to_owned(),
                i.details
                    .pifs
                    .values()
                    .filter(|p| p.network == Some(id))
                    .count()
                    .to_string(),
                i.details
                    .vifs
                    .values()
                    .filter(|v| v.network == Some(id))
                    .count()
                    .to_string(),
            ],
        )
    });
    blocks![titled(
        "networking-networks",
        "Networks",
        blocks![
            class_notice(i, "PIF"),
            class_notice(i, "VIF"),
            table(
                "networking-networks-table",
                [
                    column("name", "Name"),
                    column("bridge", "Bridge"),
                    column("mtu", "MTU").right(),
                    column("managed", "Managed"),
                    column("uplinks", "Host uplinks").right(),
                    column("vifs", "VM interfaces").right(),
                ],
                rows
            )
            .empty("No network of this pool is in the readable inventory."),
        ]
    )]
}

fn vif_columns(first: &str) -> Vec<Column> {
    vec![
        column("name", first),
        column("uuid", "Interface UUID"),
        column("device", "Device"),
        column("mac", "MAC"),
        column("attached", "Attached"),
    ]
}

/// Networking of a network: its host uplinks and its VM interfaces.
pub(super) fn network_networking(view: &View, object: &ObjectRef) -> Vec<Block> {
    let i = view.inventory;
    let Some(uuid) = object
        .uuid()
        .filter(|uuid| i.networks.iter().any(|n| n.uuid == *uuid))
    else {
        return missing("network");
    };
    let id = NetworkId::from(uuid);
    let interfaces = i
        .details
        .vifs
        .values()
        .filter(|nic| nic.network == Some(id))
        .map(|nic| {
            row(
                nic.id,
                [
                    vm_name(i, nic.vm).unwrap_or_else(|| "Unresolved VM".into()),
                    nic.id.to_string(),
                    nic.device.clone(),
                    nic.mac.clone(),
                    known(nic.currently_attached).to_owned(),
                ],
            )
        });
    blocks![
        titled(
            "networking-uplinks",
            "Host uplinks",
            pifs(
                i,
                i.details.pifs.values().filter(|p| p.network == Some(id)),
                "No host uplink of this network is in the readable inventory."
            )
        ),
        titled(
            "networking-vifs",
            "VM interfaces",
            blocks![
                class_notice(i, "VIF"),
                table("networking-vifs-table", vif_columns("VM"), interfaces)
                    .empty("No VM interface of this network is in the readable inventory."),
            ]
        ),
    ]
}

/// Networking of a VM or a template: its virtual interfaces.
pub(super) fn vm_networking(view: &View, vm: VmId) -> Vec<Block> {
    let i = view.inventory;
    if !i.vms.contains_key(&vm) {
        return missing("VM");
    }
    let interfaces = i
        .details
        .vifs
        .values()
        .filter(|nic| nic.vm == Some(vm))
        .map(|nic| {
            row(
                nic.id,
                [
                    network_name(i, nic.network),
                    nic.id.to_string(),
                    nic.device.clone(),
                    nic.mac.clone(),
                    known(nic.currently_attached).to_owned(),
                ],
            )
        });
    blocks![titled(
        "networking-vifs",
        "Virtual interfaces",
        blocks![
            class_notice(i, "VIF"),
            table("networking-vifs-table", vif_columns("Network"), interfaces)
                .empty("No virtual interface of this VM is in the readable inventory."),
        ]
    )]
}
