#[path = "vm_device_actions.rs"]
mod actions;
use super::{State, caption, console_view, heading, storage_bytes};
use rune::{El, ScrollSpec, col, row};
use rune_kit::{Button, ButtonVariant, Kit};
use xenadmin_core::{
    ClassAvailability, Inventory, PoolId, VirtualBlockDevice, VirtualInterface, VmId,
};

pub(super) struct DeviceView {
    pub pool: PoolId,
    pub vm: VmId,
    pub name: String,
    form: Option<actions::Form>,
}

pub(super) fn open(s: &mut State) {
    let Some(vm) = s.selected().and_then(|row| row.vm) else {
        return;
    };
    let Some(inventory) = &s.inventory else {
        return;
    };
    let Some(guest) = inventory.vms.get(&vm) else {
        return;
    };
    let target = DeviceView {
        pool: inventory.pool_id,
        vm,
        name: guest.name.clone(),
        form: None,
    };
    console_view::release(s);
    s.console = None;
    s.console_snapshot = None;
    s.snapshot_view = None;
    s.show_details = false;
    s.devices = Some(target);
}

pub(super) fn view(s: &State, k: &Kit) -> El {
    let Some(target) = &s.devices else {
        return rune::el();
    };
    let mut page = col()
        .w_full()
        .h_full()
        .min_h(0.0)
        .padding(20.0)
        .gap(12.0)
        .child(
            row()
                .w_full()
                .justify_between()
                .center_y()
                .child(heading(format!("Devices · {}", target.name), k))
                .child(
                    Button::new("Back to inventory")
                        .variant(ButtonVariant::Outline)
                        .on_press(|s: &mut State| s.devices = None)
                        .build(k)
                        .key("close-vm-devices"),
                ),
        )
        .child(caption(format!("VM UUID: {}", target.vm), k).wrap_anywhere());
    let Some(inventory) = s
        .inventory
        .as_ref()
        .filter(|inventory| inventory.pool_id == target.pool)
    else {
        return page.child(caption("This VM belongs to another connection. Select its pool to inspect current devices.", k).wrap());
    };
    if !inventory.vms.contains_key(&target.vm) {
        return page
            .child(caption("This VM is no longer present in the current inventory.", k).wrap());
    }
    if !s.demo
        && !s
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.is_current())
    {
        page = page.child(caption("Inventory is stale. Device details show the last observed state; reconnect to refresh them.", k).wrap().role("status"));
    }
    let mut content = col()
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .gap(18.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        });
    let mut disks = section("Disks and CD/DVD drives", k);
    if let Some(reason) = unavailable(inventory, "VBD", "Disk attachments") {
        disks = disks.child(caption(reason, k).wrap());
    } else {
        let rows = disk_rows(inventory, target.vm);
        if rows.is_empty() {
            disks = disks.child(caption(
                "No disk or CD/DVD attachments are reported for this VM.",
                k,
            ));
        }
        for disk in rows {
            disks = disks.child(disk_card(s, inventory, target.vm, disk, k));
        }
    }
    content = content.child(disks);
    let mut interfaces = section("Network interfaces", k);
    if let Some(reason) = unavailable(inventory, "VIF", "Network interfaces") {
        interfaces = interfaces.child(caption(reason, k).wrap());
    } else {
        let rows = interface_rows(inventory, target.vm);
        if rows.is_empty() {
            interfaces = interfaces.child(caption(
                "No network interfaces are reported for this VM.",
                k,
            ));
        }
        for interface in rows {
            interfaces = interfaces.child(interface_card(s, inventory, interface, k));
        }
    }
    content = content
        .child(interfaces)
        .child(guest_tools(inventory, target.vm, k));
    page = page.child(content);
    if let Some(dialog) = actions::dialog(s, k) {
        page = page.child(dialog);
    }
    page
}
fn section(title: &str, k: &Kit) -> El {
    col().w_full().gap(10.0).child(heading(title, k))
}
fn card(k: &Kit) -> El {
    col()
        .w_full()
        .padding(14.0)
        .gap(7.0)
        .radius(6.0)
        .fill(k.c.bg_base)
}
fn field(label: &str, value: impl Into<String>, k: &Kit) -> El {
    row()
        .w_full()
        .gap(12.0)
        .child(caption(label, k).w(155.0))
        .child(heading(value, k).grow(1.0).min_w(0.0).wrap_anywhere())
}
fn boolean(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "Yes",
        Some(false) => "No",
        None => "Not reported",
    }
}
fn connected(value: Option<bool>) -> &'static str {
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
fn unavailable(inventory: &Inventory, class: &str, label: &str) -> Option<String> {
    class_message(inventory.details.classes.get(class), label)
}
fn class_message(availability: Option<&ClassAvailability>, label: &str) -> Option<String> {
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
fn disk_rows(inventory: &Inventory, vm: VmId) -> Vec<&VirtualBlockDevice> {
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
fn interface_rows(inventory: &Inventory, vm: VmId) -> Vec<&VirtualInterface> {
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
fn disk_card(
    s: &State,
    inventory: &Inventory,
    vm: VmId,
    attachment: &VirtualBlockDevice,
    k: &Kit,
) -> El {
    let cd = attachment.kind.eq_ignore_ascii_case("CD");
    let title = format!(
        "{} · slot {}",
        if cd { "CD/DVD drive" } else { "Disk" },
        nonempty(&attachment.userdevice)
    );
    let mut view = card(k)
        .child(heading(title, k))
        .child(field(
            "Connection",
            connected(attachment.currently_attached),
            k,
        ))
        .child(field("Guest device", nonempty(&attachment.device), k))
        .child(field("Access mode", nonempty(&attachment.mode), k))
        .child(field("Bootable", boolean(attachment.bootable), k));
    if cd && attachment.empty == Some(true) {
        view = view.child(field("Media", "No media inserted", k));
    } else if let Some(reason) = unavailable(inventory, "VDI", "Virtual disk details") {
        view = view.child(caption(reason, k).wrap());
    } else if let Some(disk) = attachment
        .vdi
        .and_then(|id| inventory.details.vdis.get(&id))
    {
        let storage = disk
            .sr
            .and_then(|id| inventory.storage.iter().find(|sr| sr.uuid == id.uuid()));
        let sharing = inventory
            .details
            .vdi_attachments
            .get(&disk.id)
            .map_or(0, Vec::len);
        view = view
            .child(field(
                if cd { "Media" } else { "Disk name" },
                nonempty(&disk.name),
                k,
            ))
            .child(field(
                "Storage repository",
                storage.map_or("Not reported", |sr| sr.name.as_str()),
                k,
            ))
            .child(field("Virtual size", storage_bytes(disk.virtual_size), k))
            .child(field(
                "Physical usage",
                storage_bytes(disk.physical_utilisation),
                k,
            ))
            .child(field("Type", nonempty(&disk.kind), k))
            .child(field("Read only", boolean(disk.read_only), k))
            .child(field("Shareable", boolean(disk.sharable), k))
            .child(field("Missing from storage", boolean(disk.missing), k))
            .child(field("Attachment count", sharing.to_string(), k))
            .child(field("Disk UUID", disk.id.to_string(), k));
        if !disk.description.is_empty() {
            view = view.child(field("Description", &disk.description, k));
        }
    } else {
        view = view.child(
            caption(
                if cd {
                    "The host has not reported the inserted media record."
                } else {
                    "The backing disk record is unavailable."
                },
                k,
            )
            .wrap(),
        );
    }
    view.child(field("Attachment UUID", attachment.id.to_string(), k))
        .child(actions::disk_buttons(s, inventory, vm, attachment, k))
}
fn interface_card(s: &State, inventory: &Inventory, interface: &VirtualInterface, k: &Kit) -> El {
    let network = interface.network.and_then(|id| {
        inventory
            .networks
            .iter()
            .find(|network| network.uuid == id.uuid())
    });
    let mut view = card(k)
        .child(heading(format!("NIC {}", nonempty(&interface.device)), k))
        .child(field(
            "Connection",
            connected(interface.currently_attached),
            k,
        ))
        .child(field(
            "Network",
            network.map_or("Not reported", |network| network.name.as_str()),
            k,
        ))
        .child(field(
            "Bridge",
            network.map_or("Not reported", |network| network.bridge.as_str()),
            k,
        ))
        .child(field("MAC address", nonempty(&interface.mac), k))
        .child(field(
            "MTU",
            interface
                .mtu
                .map_or_else(|| "Not reported".into(), |mtu| mtu.to_string()),
            k,
        ));
    if let Some(network_id) = interface.network {
        if let Some(details) = inventory.details.networks.get(&network_id) {
            view = view.child(field(
                "Default locking mode",
                details
                    .default_locking_mode
                    .as_deref()
                    .unwrap_or("Not reported"),
                k,
            ));
        }
        if unavailable(inventory, "PIF", "Physical uplinks").is_none() {
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
            view = view.child(field(
                "Physical uplinks",
                if uplinks.is_empty() {
                    "No physical uplinks reported".into()
                } else {
                    uplinks.join("\n")
                },
                k,
            ));
        } else if let Some(reason) = unavailable(inventory, "PIF", "Physical uplinks") {
            view = view.child(caption(reason, k).wrap());
        }
    }
    if !interface.qos_algorithm.is_empty() {
        view = view.child(field("QoS algorithm", &interface.qos_algorithm, k));
        for (name, value) in &interface.qos_parameters {
            view = view.child(field(name, value, k));
        }
    }
    view.child(field("Interface UUID", interface.id.to_string(), k))
        .child(actions::nic_buttons(s, inventory, interface, k))
}
fn guest_tools(inventory: &Inventory, vm: VmId, k: &Kit) -> El {
    let mut section = section("Guest tools and reported addresses", k);
    if let Some(reason) = unavailable(inventory, "VM_guest_metrics", "Guest tools details") {
        return section.child(caption(reason, k).wrap());
    }
    let guest = inventory
        .details
        .vms
        .get(&vm)
        .and_then(|details| details.guest_metrics)
        .and_then(|id| inventory.details.guest_metrics.get(&id));
    let Some(guest) = guest else {
        return section.child(caption("No guest metrics record is reported. Guest tools may be absent or have not yet reported data.", k).wrap());
    };
    let mut view = card(k)
        .child(field("Agent live", boolean(guest.live), k))
        .child(field(
            "Last updated",
            guest.last_updated.as_deref().unwrap_or("Not reported"),
            k,
        ));
    if guest.live != Some(true) {
        view = view.child(
            caption(
                "Guest tools are not reporting live status. OS details and addresses may be stale.",
                k,
            )
            .wrap(),
        );
    }
    for (label, values) in [
        ("Operating system", &guest.os_version),
        ("Tools version", &guest.tools_version),
        ("Reported addresses", &guest.networks),
    ] {
        view = view.child(heading(label, k));
        if values.is_empty() {
            view = view.child(caption("Not reported", k));
        }
        for (name, value) in values {
            view = view.child(field(name, value, k));
        }
    }
    section = section.child(view);
    section
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn absent_permission_and_empty_are_distinct() {
        assert!(class_message(Some(&ClassAvailability::Available), "Disks").is_none());
        assert!(
            class_message(Some(&ClassAvailability::PermissionDenied), "Disks")
                .unwrap()
                .contains("permission")
        );
        assert!(
            class_message(Some(&ClassAvailability::Unsupported), "Disks")
                .unwrap()
                .contains("not supported")
        );
        assert!(
            class_message(None, "Disks")
                .unwrap()
                .contains("not been reported")
        );
        assert_eq!(connected(None), "Not reported");
        assert_eq!(connected(Some(false)), "Disconnected");
    }
}
