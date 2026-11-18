//! What the Networking and NICs tabs say about interfaces and networks.
//!
//! These are the blocks the `general` area's information-only fallbacks of
//! the same tabs show (`general/resources.rs`, from the Rune
//! `resource_details.rs`), copied here unchanged with their keys and texts,
//! so that the tabs this area provides lose nothing of them and do not
//! depend on the fallbacks staying.

use xenadmin_core::{ClassAvailability, HostId, Inventory, NetworkId, PhysicalInterface, VmId};

use crate::prelude::*;

pub(super) fn known(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "Yes",
        Some(false) => "No",
        None => "Unknown",
    }
}

pub(super) fn supplied(value: &str) -> &str {
    if value.is_empty() {
        "Not supplied"
    } else {
        value
    }
}

pub(super) fn host_name(inventory: &Inventory, id: Option<HostId>) -> String {
    id.map(|id| {
        inventory
            .hosts
            .get(&id)
            .map(|host| host.name.clone())
            .unwrap_or_else(|| id.to_string())
    })
    .unwrap_or_else(|| "Unresolved host".into())
}

pub(super) fn network_name(inventory: &Inventory, id: Option<NetworkId>) -> String {
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

pub(super) fn vm_name(inventory: &Inventory, id: Option<VmId>) -> Option<String> {
    id.map(|vm| {
        inventory
            .vms
            .get(&vm)
            .map(|v| v.name.clone())
            .unwrap_or_else(|| vm.to_string())
    })
}

/// "{class}: {message}" unless the class is available.
pub(super) fn class_notice(inventory: &Inventory, class: &str) -> Option<Notice> {
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

/// The Rune `pif` block: one titled section per physical interface.
pub(super) fn pif(inventory: &Inventory, interface: &PhysicalInterface) -> Group {
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

pub(super) fn vif_columns(first: &str) -> Vec<Column> {
    vec![
        column("name", first),
        column("uuid", "Interface UUID"),
        column("device", "Device"),
        column("mac", "MAC"),
        column("attached", "Attached"),
    ]
}
