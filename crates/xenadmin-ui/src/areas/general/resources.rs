//! What the Rune details pane appended for a server, a storage repository
//! and a network (`resource_details.rs`): the scalar fields, as sections of
//! General. Its storage attachments, virtual disks, uplinks, VM interfaces
//! and host interfaces are the Storage, Networking and NICs tabs of the
//! `storage`, `vm_devices` and `network` areas; the information-only
//! bodies this file had for them until those areas landed are gone.
//!
//! An unknown value is "Unknown", never "No"; an inventory class that could
//! not be read says so above the list it would have filled.

use xenadmin_core::{
    ClassAvailability, Inventory, NetworkId, SrId, VdiId, VmId,
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

/// What the Storage, Networking and NICs tabs say of an object that is no
/// longer in the inventory of its pool. The tabs themselves belong to the
/// `vm_devices`, `storage` and `network` areas, which are asked first
/// (`tabs::providers`) and answer for every kind whenever the object is in
/// the inventory; they leave this case to the next provider.
pub(super) fn gone(kind: ObjectKind) -> Option<Vec<Block>> {
    use ObjectKind as K;
    Some(missing(match kind {
        K::Pool => "pool",
        K::Host => "host",
        K::Vm | K::Template => "VM",
        K::StorageRepository => "storage repository",
        K::Network => "network",
        _ => return None,
    }))
}
