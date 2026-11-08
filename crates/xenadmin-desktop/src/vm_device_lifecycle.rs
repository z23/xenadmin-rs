use super::{State, caption, field, heading, storage_bytes};
use rune::{El, ScrollSpec, TextEdit, col, el, row};
use rune_kit::{Button, ButtonVariant, Checkbox, Kit, TextInput};
use xenadmin_core::{
    DeviceCommand, DeviceOperation, Inventory, NetworkId, ObservedVm, PoolId, PowerState, SrId,
    VdiId, VifId, VirtualBlockDevice, VirtualInterface, VmId,
};

#[derive(Clone)]
enum Kind {
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
struct Review {
    operation: DeviceOperation,
    lines: Vec<String>,
    destructive: bool,
}
pub(super) struct Form {
    target: Option<ObservedVm>,
    pool: PoolId,
    vm: VmId,
    vm_name: String,
    kind: Kind,
    name: TextEdit,
    size: TextEdit,
    device: TextEdit,
    mac: TextEdit,
    sr: Option<SrId>,
    network: Option<NetworkId>,
    disk: Option<VdiId>,
    read_only: bool,
    review: Option<Review>,
    error: Option<String>,
}
fn editable(s: &State, inventory: &Inventory, vm: VmId) -> bool {
    (s.demo || s.can_mutate())
        && inventory.vms.get(&vm).is_some_and(|guest| {
            guest.power_state == PowerState::Halted
                && !guest.is_template
                && !guest.is_control_domain
        })
}
fn next_device(inventory: &Inventory, vm: VmId, nic: bool) -> String {
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
fn open(s: &mut State, kind: Kind) {
    let Some(view) = &s.devices else { return };
    let Some(inventory) = s
        .inventory
        .as_ref()
        .filter(|inventory| inventory.pool_id == view.pool)
    else {
        return;
    };
    if !editable(s, inventory, view.vm) {
        return;
    }
    let target = s
        .snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.observe_vm(view.vm).ok());
    if !s.demo && target.is_none() {
        return;
    }
    let mut form = Form {
        target,
        pool: view.pool,
        vm: view.vm,
        vm_name: view.name.clone(),
        device: TextEdit::new(next_device(
            inventory,
            view.vm,
            matches!(kind, Kind::AddNic),
        )),
        name: TextEdit::new(""),
        size: TextEdit::new(""),
        mac: TextEdit::new(""),
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
        form.mac = TextEdit::new(mac.clone());
    }
    if let Kind::Fixed(operation) = &form.kind {
        form.review = Some(review_for(operation.clone(), inventory));
    }
    if let Some(view) = &mut s.devices {
        view.form = None;
        view.lifecycle = Some(form);
    }
    s.error = None;
}
fn form_mut(s: &mut State) -> Option<&mut Form> {
    s.devices.as_mut()?.lifecycle.as_mut()
}
fn close(s: &mut State) {
    if let Some(view) = &mut s.devices {
        view.lifecycle = None;
    }
}
fn entry(label: &'static str, key: &str, kind: Kind, enabled: bool, k: &Kit) -> El {
    Button::new(label)
        .variant(ButtonVariant::Outline)
        .disabled(!enabled)
        .on_press(move |s: &mut State| open(s, kind.clone()))
        .build(k)
        .key(key)
}
pub(super) fn disk_creation_controls(s: &State, inventory: &Inventory, vm: VmId, k: &Kit) -> El {
    let enabled = editable(s, inventory, vm);
    let mut controls = col().gap(6.0).child(
        row()
            .w_full()
            .flex_wrap()
            .gap(8.0)
            .child(entry(
                "Create disk…",
                "create-vm-disk",
                Kind::CreateDisk,
                enabled,
                k,
            ))
            .child(entry(
                "Attach existing disk…",
                "attach-vm-disk",
                Kind::AttachDisk,
                enabled,
                k,
            ))
            .child(entry(
                "Add CD/DVD drive…",
                "add-vm-cd",
                Kind::AddCd,
                enabled,
                k,
            )),
    );
    if !enabled {
        controls = controls.child(caption("Creating, attaching, detaching or deleting devices requires a halted VM and a current writable connection.", k).wrap());
    }
    controls
}
pub(super) fn nic_creation_control(s: &State, inventory: &Inventory, vm: VmId, k: &Kit) -> El {
    entry(
        "Add network interface…",
        "add-vm-nic",
        Kind::AddNic,
        editable(s, inventory, vm),
        k,
    )
}
fn fixed_control(
    s: &State,
    inventory: &Inventory,
    vm: VmId,
    label: &'static str,
    key: String,
    operation: DeviceOperation,
    k: &Kit,
) -> El {
    let availability = operation.availability(inventory, vm);
    let enabled = editable(s, inventory, vm) && availability.is_available();
    let mut control = col()
        .gap(5.0)
        .child(entry(label, &key, Kind::Fixed(operation), enabled, k));
    if !availability.is_available() {
        control = control.child(caption(availability.explanation(), k).wrap());
    }
    control
}
pub(super) fn disk_controls(
    s: &State,
    inventory: &Inventory,
    vm: VmId,
    attachment: &VirtualBlockDevice,
    k: &Kit,
) -> El {
    let Some(vdi) = attachment.vdi.filter(|_| attachment.kind == "Disk") else {
        return el();
    };
    col()
        .gap(8.0)
        .child(fixed_control(
            s,
            inventory,
            vm,
            "Detach disk (keep data)…",
            format!("detach-disk-{}", attachment.id),
            DeviceOperation::DetachDisk {
                vbd: attachment.id,
                vdi,
            },
            k,
        ))
        .child(fixed_control(
            s,
            inventory,
            vm,
            "Delete disk and data…",
            format!("delete-disk-{}", attachment.id),
            DeviceOperation::DeleteDisk {
                vbd: attachment.id,
                vdi,
            },
            k,
        ))
}
pub(super) fn nic_controls(
    s: &State,
    inventory: &Inventory,
    interface: &VirtualInterface,
    k: &Kit,
) -> El {
    let Some(vm) = interface.vm else { return el() };
    let mut controls = col().gap(8.0);
    if let Some(network) = interface.network {
        controls = controls.child(entry(
            "Change network or MAC…",
            &format!("edit-nic-{}", interface.id),
            Kind::EditNic {
                vif: interface.id,
                network,
                mac: interface.mac.clone(),
            },
            editable(s, inventory, vm),
            k,
        ));
    }
    controls.child(fixed_control(
        s,
        inventory,
        vm,
        "Delete network interface…",
        format!("delete-nic-{}", interface.id),
        DeviceOperation::DeleteNic { vif: interface.id },
        k,
    ))
}
fn parse_device(value: &str) -> Result<String, String> {
    let value = value.trim();
    value.parse::<u8>().ok().filter(|number| number.to_string() == value)
        .map(|number| number.to_string()).ok_or_else(|| "Enter a device slot from 0 to 255 without leading zeros. The host checks whether the slot is available.".into())
}
fn parse_mac(value: &str) -> Result<String, String> {
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
fn build_operation(form: &Form) -> Result<DeviceOperation, String> {
    match &form.kind {
        Kind::CreateDisk => {
            let name = form.name.text.trim();
            if name.is_empty()
                || name.chars().count() > 255
                || form.name.text.chars().any(char::is_control)
            {
                return Err(
                    "Enter a disk name of 1–255 characters without control characters.".into(),
                );
            }
            Ok(DeviceOperation::CreateDisk {
                sr: form.sr.ok_or("Select a storage repository.")?,
                name: name.into(),
                size: super::actions::parse_size(&form.size.text)?,
                device: parse_device(&form.device.text)?,
            })
        }
        Kind::AttachDisk => Ok(DeviceOperation::AttachDisk {
            vdi: form.disk.ok_or("Select a disk.")?,
            device: parse_device(&form.device.text)?,
            read_only: form.read_only,
        }),
        Kind::AddCd => Ok(DeviceOperation::AddCdDrive {
            device: parse_device(&form.device.text)?,
        }),
        Kind::AddNic => Ok(DeviceOperation::AddNic {
            network: form.network.ok_or("Select a network.")?,
            device: parse_device(&form.device.text)?,
            mac: if form.mac.text.trim().is_empty() {
                None
            } else {
                Some(parse_mac(&form.mac.text)?)
            },
        }),
        Kind::EditNic { vif, network, mac } => {
            let selected_network = form.network.ok_or("Select a network.")?;
            let selected_mac = parse_mac(&form.mac.text)?;
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
fn disk_name(inventory: &Inventory, id: VdiId) -> String {
    inventory.details.vdis.get(&id).map_or_else(
        || id.to_string(),
        |disk| format!("{} ({id}), {}", disk.name, storage_bytes(disk.virtual_size)),
    )
}
fn network_name(inventory: &Inventory, id: NetworkId) -> String {
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
fn review_for(operation: DeviceOperation, inventory: &Inventory) -> Review {
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
fn review(s: &mut State) {
    let Some(form) = s.devices.as_ref().and_then(|view| view.lifecycle.as_ref()) else {
        return;
    };
    let result = build_operation(form).and_then(|operation| {
        let inventory = s
            .inventory
            .as_ref()
            .filter(|inventory| inventory.pool_id == form.pool)
            .ok_or("The selected connection changed.")?;
        let availability = operation.availability(inventory, form.vm);
        if !availability.is_available() {
            return Err(availability.explanation());
        }
        Ok(review_for(operation, inventory))
    });
    if let Some(form) = form_mut(s) {
        match result {
            Ok(review) => {
                form.review = Some(review);
                form.error = None;
            }
            Err(error) => form.error = Some(error),
        }
    }
}
fn apply(s: &mut State) {
    let Some(form) = s.devices.as_ref().and_then(|view| view.lifecycle.as_ref()) else {
        return;
    };
    let Some(target) = form.target else { return };
    let Some(review) = &form.review else { return };
    if !s.can_mutate()
        || s.snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.observe_vm(form.vm).ok())
            != Some(target)
        || !s.inventory.as_ref().is_some_and(|inventory| {
            inventory.pool_id == form.pool
                && review
                    .operation
                    .availability(inventory, form.vm)
                    .is_available()
        })
    {
        return;
    }
    let command = DeviceCommand {
        vm: target,
        operation: review.operation.clone(),
    };
    s.submit_command(xenadmin_core::VmCommand::Device(command));
    if s.error.is_none() {
        close(s);
    }
}
fn choices<T: Copy + std::fmt::Display + 'static>(
    label: &str,
    prefix: &'static str,
    items: Vec<(T, String, bool)>,
    select: fn(&mut Form, T),
    k: &Kit,
) -> El {
    let mut group = col().w_full().gap(6.0).child(heading(label, k));
    if items.is_empty() {
        return group.child(
            caption(
                "No eligible resources are reported in the current inventory.",
                k,
            )
            .wrap(),
        );
    }
    let mut list = col().w_full().gap(5.0).max_h(150.0).scroll(ScrollSpec {
        stick_bottom: false,
    });
    for (id, label, selected) in items {
        list = list.child(
            Button::new(if selected {
                format!("✓ {label}")
            } else {
                label
            })
            .variant(ButtonVariant::Outline)
            .on_press(move |s: &mut State| {
                if let Some(form) = form_mut(s) {
                    select(form, id);
                    form.error = None;
                }
            })
            .build(k)
            .key(format!("{prefix}-{id}")),
        );
    }
    group = group.child(list);
    group
}
fn editor(form: &Form, inventory: &Inventory, k: &Kit) -> El {
    let mut body = col().w_full().gap(12.0);
    if matches!(form.kind, Kind::CreateDisk) {
        body = body
            .child(
                TextInput::new(&form.name)
                    .label("Disk name")
                    .on_edit(|s: &mut State| &mut form_mut(s).expect("device form open").name)
                    .build(k)
                    .key("lifecycle-name"),
            )
            .child(
                TextInput::new(&form.size)
                    .label("Capacity (bytes, KiB, MiB or GiB)")
                    .placeholder("16 GiB")
                    .on_edit(|s: &mut State| &mut form_mut(s).expect("device form open").size)
                    .build(k)
                    .key("lifecycle-size"),
            );
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
                    form.sr == Some(storage.id),
                )
            })
            .collect();
        repositories.sort_by_cached_key(|(_, name, _)| name.to_lowercase());
        body = body.child(choices(
            "Storage repository",
            "lifecycle-sr",
            repositories,
            |form, id| form.sr = Some(id),
            k,
        ));
    }
    if matches!(form.kind, Kind::AttachDisk) {
        body = body.child(
            Checkbox::new(form.read_only)
                .label("Attach read-only")
                .on_toggle(|s: &mut State| {
                    if let Some(form) = form_mut(s) {
                        form.read_only = !form.read_only;
                    }
                })
                .build(k)
                .key("lifecycle-read-only"),
        );
        let device = parse_device(&form.device.text).unwrap_or_else(|_| "0".into());
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
            .map(|disk| {
                (
                    disk.id,
                    disk_name(inventory, disk.id),
                    form.disk == Some(disk.id),
                )
            })
            .collect();
        disks.sort_by_cached_key(|(_, label, _)| label.to_lowercase());
        body = body.child(choices("Unattached disks", "lifecycle-disk", disks, |form, id| form.disk = Some(id), k))
            .child(caption("Only managed, non-shared disks with no consumers can be attached here. This preserves existing disk contents.", k).wrap());
    }
    if matches!(form.kind, Kind::AddNic | Kind::EditNic { .. }) {
        let mut networks: Vec<_> = inventory
            .networks
            .iter()
            .map(|network| {
                let id = NetworkId::from(network.uuid);
                (
                    id,
                    format!("{} ({id})", network.name),
                    form.network == Some(id),
                )
            })
            .collect();
        networks.sort_by_cached_key(|(_, label, _)| label.to_lowercase());
        body = body
            .child(choices(
                "Network",
                "lifecycle-network",
                networks,
                |form, id| form.network = Some(id),
                k,
            ))
            .child(
                TextInput::new(&form.mac)
                    .label(if matches!(form.kind, Kind::AddNic) {
                        "MAC address (blank generates one)"
                    } else {
                        "MAC address"
                    })
                    .on_edit(|s: &mut State| &mut form_mut(s).expect("device form open").mac)
                    .build(k)
                    .key("lifecycle-mac"),
            );
    }
    if matches!(
        form.kind,
        Kind::CreateDisk | Kind::AttachDisk | Kind::AddCd | Kind::AddNic
    ) {
        body = body.child(TextInput::new(&form.device).label("Device slot")
            .on_edit(|s: &mut State| &mut form_mut(s).expect("device form open").device)
            .build(k).key("lifecycle-slot"))
            .child(caption("A currently unused slot is suggested. The host validates the slot before changing devices.", k).wrap());
    }
    body
}
pub(super) fn dialog(s: &State, k: &Kit) -> Option<El> {
    let form = s.devices.as_ref()?.lifecycle.as_ref()?;
    let inventory = s
        .inventory
        .as_ref()
        .filter(|inventory| inventory.pool_id == form.pool);
    let title = if form.review.is_some() {
        "Review device change"
    } else {
        match &form.kind {
            Kind::CreateDisk => "Create and attach disk",
            Kind::AttachDisk => "Attach existing disk",
            Kind::AddCd => "Add CD/DVD drive",
            Kind::AddNic => "Add network interface",
            Kind::EditNic { .. } => "Change network or MAC",
            Kind::Fixed(_) => "Review device change",
        }
    };
    let mut body = col()
        .w((s.window_width - 40.0).clamp(280.0, 640.0))
        .max_h((s.window_height - 60.0).clamp(280.0, 760.0))
        .key("device-lifecycle-body")
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .padding(22.0)
        .gap(12.0)
        .radius(10.0)
        .fill(k.c.bg_base)
        .child(heading(title, k))
        .child(field("VM", &form.vm_name, k))
        .child(field("VM UUID", form.vm.to_string(), k));
    if let Some(review) = &form.review {
        for line in &review.lines {
            body = body.child(caption(line, k).wrap_anywhere());
        }
    } else if let Some(inventory) = inventory {
        body = body.child(editor(form, inventory, k));
    }
    let target_current = inventory.is_some()
        && (s.demo
            || form.target.is_some_and(|target| {
                s.snapshot
                    .as_ref()
                    .and_then(|snapshot| snapshot.observe_vm(form.vm).ok())
                    == Some(target)
            }));
    let eligible = form
        .review
        .as_ref()
        .zip(inventory)
        .map(|(review, inventory)| review.operation.availability(inventory, form.vm));
    if !target_current {
        body = body.child(
            caption(
                "The connection or VM changed. Cancel and reopen this editor before continuing.",
                k,
            )
            .wrap()
            .role("status"),
        );
    }
    if let Some(status) = eligible.as_ref().filter(|status| !status.is_available()) {
        body = body.child(
            caption(
                format!(
                    "This change is no longer available: {} Cancel and review it again.",
                    status.explanation()
                ),
                k,
            )
            .wrap()
            .role("status"),
        );
    }
    if s.demo {
        body = body.child(
            caption(
                "Synthetic device review • Apply is disabled and no host changes can be submitted.",
                k,
            )
            .wrap(),
        );
    }
    if let Some(error) = form.error.as_ref().or(s.error.as_ref()) {
        body = body.child(
            caption(error, k)
                .color(k.c.danger.icon)
                .wrap()
                .role("alert"),
        );
    }
    let mut controls = row().w_full().flex_wrap().gap(8.0).justify_end().child(
        Button::new("Cancel")
            .variant(ButtonVariant::Ghost)
            .on_press(close)
            .build(k)
            .key("cancel-device-lifecycle"),
    );
    if let Some(review) = &form.review {
        if !matches!(form.kind, Kind::Fixed(_)) {
            controls = controls.child(
                Button::new("Back")
                    .variant(ButtonVariant::Ghost)
                    .on_press(|s: &mut State| {
                        if let Some(form) = form_mut(s) {
                            form.review = None;
                        }
                    })
                    .build(k)
                    .key("back-device-lifecycle"),
            );
        }
        controls = controls.child(
            Button::new(if review.destructive {
                "Confirm change"
            } else {
                "Apply change"
            })
            .variant(if review.destructive {
                ButtonVariant::Danger
            } else {
                ButtonVariant::Outline
            })
            .disabled(
                s.demo
                    || !s.can_mutate()
                    || !target_current
                    || !eligible.is_some_and(|status| status.is_available()),
            )
            .on_press(apply)
            .build(k)
            .key("apply-device-lifecycle"),
        );
    } else {
        controls = controls.child(
            Button::new("Review change")
                .variant(ButtonVariant::Outline)
                .disabled(!target_current || (!s.demo && !s.can_mutate()))
                .on_press(review)
                .build(k)
                .key("review-device-lifecycle"),
        );
    }
    Some(
        el().w_full()
            .h_full()
            .absolute(0.0, 0.0)
            .center()
            .fill(rune::Color::BLACK.alpha(0.5))
            .role("dialog")
            .label(title)
            .modal(false, close)
            .key("device-lifecycle-dialog")
            .child(body.child(controls)),
    )
}

pub(super) fn operation_history(s: &State, inventory: &Inventory, vm: VmId, k: &Kit) -> El {
    let rows: Vec<_> = s
        .operation_rows
        .iter()
        .rev()
        .filter(|record| {
            record.pool == inventory.pool_id
                && record.vm == vm
                && matches!(record.action, xenadmin_core::OperationAction::Device(_))
        })
        .take(4)
        .collect();
    if rows.is_empty() {
        return el();
    }
    let mut history = col()
        .w_full()
        .gap(10.0)
        .child(heading("Recent device changes", k));
    for record in rows {
        let mut operation = col()
            .w_full()
            .gap(5.0)
            .child(heading(
                format!("{} • {:?}", record.action.label(), record.state),
                k,
            ))
            .child(caption(&record.detail, k).wrap());
        for device in &record.created_devices {
            operation = operation.child(
                caption(
                    format!("Created {} UUID: {}", device.label(), device.uuid()),
                    k,
                )
                .wrap_anywhere(),
            );
        }
        for step in &record.steps {
            operation = operation.child(
                caption(
                    format!("{} • {:?}\n{}", step.label, step.state, step.detail),
                    k,
                )
                .wrap_anywhere(),
            );
        }
        if record.state == xenadmin_core::OperationState::OutcomeUnknown && record.can_recheck() {
            let id = record.id;
            operation = operation.child(
                Button::new("Recheck outcome")
                    .variant(ButtonVariant::Outline)
                    .disabled(!s.can_mutate())
                    .on_press(move |s: &mut State| {
                        s.control_operation(id, xenadmin_core::OperationControl::Recheck)
                    })
                    .build(k),
            );
        }
        history = history.child(operation);
    }
    history
}

#[cfg(test)]
#[path = "vm_device_lifecycle_tests.rs"]
mod tests;
