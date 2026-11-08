use super::*;
use xenadmin_core::{ClassAvailability, NetworkId, PhysicalInterface, SrId};

fn field(label: impl Into<String>, value: impl Into<String>, k: &Kit) -> El {
    col().gap(4.0).child(caption(label, k)).child(
        text(value.into())
            .font(k.ty.text_small)
            .color(k.c.text_base)
            .wrap_anywhere()
            .selectable(true),
    )
}
fn known(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "Yes",
        Some(false) => "No",
        None => "Unknown",
    }
}
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
fn class_notice(mut view: El, inventory: &Inventory, class: &str, k: &Kit) -> El {
    let state = inventory.details.classes.get(&class.to_ascii_lowercase());
    let message = match state {
        Some(ClassAvailability::Available) => return view,
        Some(ClassAvailability::PermissionDenied) => {
            "Your account cannot read this inventory class"
        }
        Some(ClassAvailability::Unsupported) => "The host does not support this inventory class",
        Some(ClassAvailability::InvalidData) => "This inventory class could not be decoded",
        _ => "Availability of this inventory class is unknown",
    };
    view = view.child(caption(format!("{class}: {message}"), k).wrap());
    view
}
fn pif(mut view: El, inventory: &Inventory, interface: &PhysicalInterface, k: &Kit) -> El {
    view = view
        .child(heading(
            format!(
                "{} · {}",
                interface.device,
                host_name(inventory, interface.host)
            ),
            k,
        ))
        .child(field("Interface UUID", interface.id.to_string(), k))
        .child(field("MAC", supplied(&interface.mac), k))
        .child(field(
            "Attached / management / physical",
            format!(
                "{} / {} / {}",
                known(interface.currently_attached),
                known(interface.management),
                known(interface.physical)
            ),
            k,
        ))
        .child(field(
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
            k,
        ))
        .child(field(
            "IPv4 mode / address",
            format!(
                "{} / {}",
                supplied(&interface.ip_configuration_mode),
                supplied(&interface.ip)
            ),
            k,
        ))
        .child(field(
            "Netmask / gateway",
            format!(
                "{} / {}",
                supplied(&interface.netmask),
                supplied(&interface.gateway)
            ),
            k,
        ))
        .child(field("DNS", supplied(&interface.dns), k))
        .child(field(
            "IPv6 mode / addresses",
            format!(
                "{} / {}",
                supplied(&interface.ipv6_configuration_mode),
                supplied(&interface.ipv6.join(", "))
            ),
            k,
        ));
    if !interface.ipv6_gateway.is_empty() {
        view = view.child(field("IPv6 gateway", &interface.ipv6_gateway, k));
    }
    if let Some(network) = interface.network {
        let name = inventory
            .networks
            .iter()
            .find(|n| n.uuid == network.uuid())
            .map(|n| n.name.clone())
            .unwrap_or_else(|| network.to_string());
        view = view.child(field("Network", name, k));
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
        view = view.child(field(
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
            k,
        ));
    }
    view
}

pub(super) fn append(s: &State, selected: &DisplayRow, mut view: El, k: &Kit) -> El {
    let Some(inventory) = &s.inventory else {
        return view;
    };
    match if console_view::host(s).is_some() {
        Page::Hosts
    } else {
        s.page
    } {
        Page::Hosts => {
            let Ok(id) = HostId::parse(&selected.id) else {
                return view;
            };
            let Some(host) = inventory.hosts.get(&id) else {
                return view;
            };
            view = view
                .child(field("Management address", &host.address, k))
                .child(field("Scheduling enabled", known(Some(host.enabled)), k))
                .child(field(
                    "Pool coordinator",
                    known(inventory.coordinator.map(|coordinator| coordinator == id)),
                    k,
                ));
            if let Some(details) = inventory.details.hosts.get(&id) {
                view = view
                    .child(field("Hostname", supplied(&details.hostname), k))
                    .child(field("Maintenance", known(details.maintenance_mode), k))
                    .child(field("Multipathing", known(details.multipathing), k))
                    .child(field(
                        "Remote logging",
                        if details.syslog_known {
                            details
                                .syslog_destination
                                .as_deref()
                                .unwrap_or("Local only")
                        } else {
                            "Unknown"
                        },
                        k,
                    ))
                    .child(field(
                        "Power-on mode",
                        details
                            .power_on_mode
                            .as_deref()
                            .map(|v| if v.is_empty() { "Disabled" } else { v })
                            .unwrap_or("Unknown"),
                        k,
                    ))
                    .child(field(
                        "Capabilities",
                        supplied(&details.capabilities.join(", ")),
                        k,
                    ))
                    .child(field("Tags", supplied(&details.tags.join(", ")), k));
                if let Some(metrics) = details
                    .metrics
                    .and_then(|id| inventory.details.host_metrics.get(&id))
                {
                    view = view
                        .child(field("Host live", known(metrics.live), k))
                        .child(field(
                            "Memory total / free",
                            format!(
                                "{} / {}",
                                storage_bytes(metrics.memory_total),
                                storage_bytes(metrics.memory_free)
                            ),
                            k,
                        ))
                        .child(field(
                            "Metrics updated",
                            metrics.last_updated.as_deref().unwrap_or("Unknown"),
                            k,
                        ));
                }
                for (name, value) in &details.cpu_info {
                    view = view.child(field(format!("CPU · {name}"), value, k));
                }
                for (name, value) in &details.software_version {
                    view = view.child(field(format!("Software · {name}"), value, k));
                }
            }
            view = class_notice(
                view.child(heading("Host interfaces", k)),
                inventory,
                "PIF",
                k,
            );
            for interface in inventory
                .details
                .pifs
                .values()
                .filter(|p| p.host == Some(id))
            {
                view = pif(view, inventory, interface, k);
            }
            view = class_notice(
                view.child(heading("Storage attachments", k)),
                inventory,
                "PBD",
                k,
            );
            for attachment in inventory
                .details
                .pbds
                .values()
                .filter(|a| a.host == Some(id))
            {
                let name = attachment
                    .sr
                    .map(|id| {
                        inventory
                            .storage
                            .iter()
                            .find(|sr| sr.uuid == id.uuid())
                            .map(|sr| sr.name.clone())
                            .unwrap_or_else(|| id.to_string())
                    })
                    .unwrap_or_else(|| "Unresolved SR".into());
                view = view.child(field(
                    name,
                    format!(
                        "{} · attached: {}",
                        attachment.id,
                        known(attachment.currently_attached)
                    ),
                    k,
                ));
            }
        }
        Page::Storage => {
            let Ok(id) = SrId::parse(&selected.id) else {
                return view;
            };
            if let Some(details) = inventory.details.storage.get(&id) {
                view = view
                    .child(field("Content type", supplied(&details.content_type), k))
                    .child(field("Shared", known(details.shared), k))
                    .child(field(
                        "Virtual allocation",
                        storage_bytes(details.virtual_allocation),
                        k,
                    ))
                    .child(field("Tags", supplied(&details.tags.join(", ")), k));
            }
            view = class_notice(
                view.child(heading("Host attachments", k)),
                inventory,
                "PBD",
                k,
            );
            for attachment in inventory.details.pbds.values().filter(|a| a.sr == Some(id)) {
                view = view.child(field(
                    host_name(inventory, attachment.host),
                    format!(
                        "{} · attached: {}",
                        attachment.id,
                        known(attachment.currently_attached)
                    ),
                    k,
                ));
            }
            view = class_notice(view.child(heading("Virtual disks", k)), inventory, "VDI", k);
            for disk in inventory
                .details
                .vdis
                .values()
                .filter(|disk| disk.sr == Some(id))
            {
                view = view
                    .child(heading(supplied(&disk.name), k))
                    .child(field("Disk UUID", disk.id.to_string(), k))
                    .child(field(
                        "Virtual / allocated size",
                        format!(
                            "{} / {}",
                            storage_bytes(disk.virtual_size),
                            storage_bytes(disk.physical_utilisation)
                        ),
                        k,
                    ))
                    .child(field(
                        "Type / read-only / shared / missing",
                        format!(
                            "{} / {} / {} / {}",
                            disk.kind,
                            known(disk.read_only),
                            known(disk.sharable),
                            known(disk.missing)
                        ),
                        k,
                    ));
                for attachment in inventory
                    .details
                    .vdi_attachments
                    .get(&disk.id)
                    .into_iter()
                    .flatten()
                    .filter_map(|id| inventory.details.vbds.get(id))
                {
                    let consumer = attachment
                        .vm
                        .map(|vm| {
                            inventory
                                .vms
                                .get(&vm)
                                .map(|v| v.name.clone())
                                .unwrap_or_else(|| vm.to_string())
                        })
                        .or_else(|| {
                            attachment
                                .snapshot
                                .map(|snapshot| format!("Snapshot {snapshot}"))
                        })
                        .unwrap_or_else(|| "Unresolved consumer".into());
                    view = view.child(field(
                        "Attachment",
                        format!(
                            "{consumer} · device {} · {} · attached: {}",
                            attachment.userdevice,
                            attachment.mode,
                            known(attachment.currently_attached)
                        ),
                        k,
                    ));
                }
            }
        }
        Page::Networks => {
            let Ok(id) = NetworkId::parse(&selected.id) else {
                return view;
            };
            if let Some(details) = inventory.details.networks.get(&id) {
                view = view
                    .child(field(
                        "MTU",
                        details
                            .mtu
                            .map(|n| n.to_string())
                            .unwrap_or_else(|| "Unknown".into()),
                        k,
                    ))
                    .child(field("Managed", known(details.managed), k))
                    .child(field(
                        "Default locking mode",
                        details.default_locking_mode.as_deref().unwrap_or("Unknown"),
                        k,
                    ))
                    .child(field("Tags", supplied(&details.tags.join(", ")), k));
            }
            view = class_notice(view.child(heading("Host uplinks", k)), inventory, "PIF", k);
            for interface in inventory
                .details
                .pifs
                .values()
                .filter(|p| p.network == Some(id))
            {
                view = pif(view, inventory, interface, k);
            }
            view = class_notice(view.child(heading("VM interfaces", k)), inventory, "VIF", k);
            for nic in inventory
                .details
                .vifs
                .values()
                .filter(|nic| nic.network == Some(id))
            {
                let consumer = nic
                    .vm
                    .map(|vm| {
                        inventory
                            .vms
                            .get(&vm)
                            .map(|v| v.name.clone())
                            .unwrap_or_else(|| vm.to_string())
                    })
                    .unwrap_or_else(|| "Unresolved VM".into());
                view = view.child(field(
                    consumer,
                    format!(
                        "{} · device {} · MAC {} · attached: {}",
                        nic.id,
                        nic.device,
                        nic.mac,
                        known(nic.currently_attached)
                    ),
                    k,
                ));
            }
        }
        _ => {}
    }
    view
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_details_show_relationships_and_unknown_attachment_state() {
        for (page, expected) in [
            (Page::Hosts, "Management address"),
            (Page::Storage, "Demo system disk"),
            (Page::Networks, "VM interfaces"),
        ] {
            let mut state = crate::init();
            crate::demo(&mut state);
            let inventory = Arc::make_mut(state.inventory.as_mut().unwrap());
            let attachment = xenadmin_core::PbdId::from(uuid::Uuid::from_u128(77));
            inventory.details.pbds.insert(
                attachment,
                xenadmin_core::StorageAttachment {
                    location: None,
                    requires_credentials: false,
                    multipathed: None,
                    paths: Vec::new(),
                    id: attachment,
                    host: inventory.hosts.keys().next().copied(),
                    sr: Some(SrId::from(inventory.storage[0].uuid)),
                    currently_attached: None,
                },
            );
            inventory
                .details
                .classes
                .insert("pbd".into(), ClassAvailability::Available);
            state.page = page;
            state.rebuild();
            state.table.select(state.rows[0].id.clone());
            let mut runtime = rune::Runtime::new(
                state,
                crate::view,
                (1440.0, 1200.0),
                rune::Env {
                    system_dark: true,
                    scale: 1.0,
                },
            );
            runtime.frame(0.0);
            let accessibility = runtime.accessibility_json().to_string();
            assert!(accessibility.contains(expected), "{expected} missing");
            if page != Page::Networks {
                assert!(accessibility.contains("attached: Unknown"));
            }
            assert!(
                runtime.find("power-Start").is_none(),
                "resource pages do not offer VM power controls"
            );
        }
    }
}
