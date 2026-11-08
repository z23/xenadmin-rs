use super::{State, caption, field, heading};
use rune::{El, ScrollSpec, TextEdit, col, el, row};
use rune_kit::{Button, ButtonVariant, Kit, TextInput};
use xenadmin_core::{
    DeviceCommand, DeviceOperation, Inventory, ObservedVm, VbdId, VdiId, VirtualBlockDevice,
    VirtualInterface, VmId,
};

pub(super) struct Form {
    target: ObservedVm,
    vm_name: String,
    operation: Option<DeviceOperation>,
    media_drive: Option<VbdId>,
    growth: Option<(VdiId, u64)>,
    size: TextEdit,
    review: bool,
    error: Option<String>,
}
fn permitted(allowed: &Option<Vec<String>>, operation: &str) -> bool {
    allowed
        .as_ref()
        .is_some_and(|allowed| allowed.iter().any(|entry| entry == operation))
}
fn ready(s: &State) -> bool {
    s.can_mutate()
}
fn open(
    s: &mut State,
    operation: Option<DeviceOperation>,
    media_drive: Option<VbdId>,
    growth: Option<(VdiId, u64)>,
) {
    let Some(view) = s.devices.as_ref() else {
        return;
    };
    let Some(inventory) = s
        .inventory
        .as_ref()
        .filter(|inventory| inventory.pool_id == view.pool)
    else {
        return;
    };
    let Some(target) = s
        .snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.observe_vm(view.vm).ok())
    else {
        return;
    };
    let Some(vm) = inventory.vms.get(&view.vm) else {
        return;
    };
    let form = Form {
        target,
        vm_name: vm.name.clone(),
        operation,
        media_drive,
        growth,
        size: TextEdit::new(""),
        review: media_drive.is_none() && growth.is_none(),
        error: None,
    };
    if let Some(view) = &mut s.devices {
        view.form = Some(form);
    }
}
fn button(label: &'static str, operation: DeviceOperation, enabled: bool, k: &Kit) -> El {
    Button::new(label)
        .variant(ButtonVariant::Outline)
        .disabled(!enabled)
        .on_press(move |s: &mut State| open(s, Some(operation.clone()), None, None))
        .build(k)
}
fn operation_control(
    s: &State,
    inventory: &Inventory,
    vm: VmId,
    label: &'static str,
    operation: DeviceOperation,
    k: &Kit,
) -> El {
    let availability = operation.availability(inventory, vm);
    let enabled = ready(s) && availability.is_available();
    let mut control = col().gap(5.0).child(button(label, operation, enabled, k));
    if !enabled {
        control = control.child(
            caption(
                if ready(s) {
                    availability.explanation()
                } else {
                    "Reconnect and wait for current inventory.".into()
                },
                k,
            )
            .wrap(),
        );
    }
    control
}
pub(super) fn disk_buttons(
    s: &State,
    inventory: &Inventory,
    vm: VmId,
    attachment: &VirtualBlockDevice,
    k: &Kit,
) -> El {
    let mut controls = col().gap(8.0);
    if attachment.kind.eq_ignore_ascii_case("CD") {
        let vbd = attachment.id;
        if attachment.empty == Some(true) {
            controls = controls.child(
                Button::new("Insert ISO…")
                    .variant(ButtonVariant::Outline)
                    .disabled(!ready(s) || !permitted(&attachment.allowed_operations, "insert"))
                    .on_press(move |s: &mut State| open(s, None, Some(vbd), None))
                    .build(k),
            );
            if !permitted(&attachment.allowed_operations, "insert") {
                controls = controls.child(caption("The host does not currently advertise permission to insert media into this drive.", k).wrap());
            }
        } else {
            controls = controls.child(operation_control(
                s,
                inventory,
                vm,
                "Eject media…",
                DeviceOperation::EjectCd { vbd },
                k,
            ));
        }
    } else {
        let (label, operation) = if attachment.currently_attached == Some(true) {
            (
                "Disconnect disk…",
                DeviceOperation::UnplugDisk { vbd: attachment.id },
            )
        } else {
            (
                "Connect disk…",
                DeviceOperation::PlugDisk { vbd: attachment.id },
            )
        };
        controls = controls.child(operation_control(s, inventory, vm, label, operation, k));
        if let Some(disk) = attachment
            .vdi
            .and_then(|id| inventory.details.vdis.get(&id))
        {
            let growth = disk.virtual_size.map(|size| (disk.id, size));
            let availability = disk.virtual_size.and_then(|size| {
                (size / 512)
                    .checked_add(1)
                    .and_then(|blocks| blocks.checked_mul(512))
                    .map(|new_size| {
                        DeviceOperation::GrowDisk {
                            vdi: disk.id,
                            expected_size: size,
                            new_size,
                        }
                        .availability(inventory, vm)
                    })
            });
            controls = controls.child(
                Button::new("Grow disk…")
                    .variant(ButtonVariant::Outline)
                    .disabled(
                        !ready(s)
                            || !availability
                                .as_ref()
                                .is_some_and(|status| status.is_available()),
                    )
                    .on_press(move |s: &mut State| open(s, None, None, growth))
                    .build(k),
            );
            controls = controls.child(caption("Disk growth requires a halted VM, disconnected attachments, and an exclusive writable disk.", k).wrap());
            if let Some(availability) = availability.filter(|status| !status.is_available()) {
                controls = controls.child(
                    caption(format!("Disk growth: {}", availability.explanation()), k).wrap(),
                );
            }
        }
    }
    controls
}
pub(super) fn nic_buttons(
    s: &State,
    inventory: &Inventory,
    interface: &VirtualInterface,
    k: &Kit,
) -> El {
    let Some(vm) = interface.vm else {
        return caption("The owning VM is not reported.", k);
    };
    let (label, operation) = if interface.currently_attached == Some(true) {
        (
            "Disconnect NIC…",
            DeviceOperation::UnplugNic { vif: interface.id },
        )
    } else {
        (
            "Connect NIC…",
            DeviceOperation::PlugNic { vif: interface.id },
        )
    };
    operation_control(s, inventory, vm, label, operation, k)
}
fn form_mut(s: &mut State) -> Option<&mut Form> {
    s.devices.as_mut()?.form.as_mut()
}
fn close(s: &mut State) {
    if let Some(view) = &mut s.devices {
        view.form = None;
    }
}
fn media_choices(inventory: &Inventory, vm: VmId, vbd: VbdId) -> Vec<(VdiId, String)> {
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
fn parse_size(text: &str) -> Result<u64, String> {
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
fn review(s: &mut State) {
    let Some(form) = form_mut(s) else { return };
    if let Some((vdi, expected_size)) = form.growth {
        match parse_size(&form.size.text) {
            Ok(new_size) if new_size > expected_size => {
                form.operation = Some(DeviceOperation::GrowDisk {
                    vdi,
                    expected_size,
                    new_size,
                })
            }
            Ok(_) => {
                form.error =
                    Some("The new capacity must be greater than the current disk size.".into());
                return;
            }
            Err(error) => {
                form.error = Some(error);
                return;
            }
        }
    }
    if form.operation.is_none() {
        form.error = Some("Select an ISO image before continuing.".into());
        return;
    }
    form.error = None;
    form.review = true;
}
fn apply(s: &mut State) {
    let Some(form) = s.devices.as_ref().and_then(|view| view.form.as_ref()) else {
        return;
    };
    let Some(operation) = form.operation.clone() else {
        return;
    };
    let command = DeviceCommand {
        vm: form.target,
        operation,
    };
    s.submit_command(xenadmin_core::VmCommand::Device(command));
    if s.error.is_none() {
        close(s);
    }
}
pub(super) fn dialog(s: &State, k: &Kit) -> Option<El> {
    let target = s.devices.as_ref()?;
    let form = target.form.as_ref()?;
    let inventory = s
        .inventory
        .as_ref()
        .filter(|inventory| inventory.pool_id == target.pool)?;
    let mut body = col()
        .w((s.window_width - 60.0).clamp(280.0, 620.0))
        .max_h(680.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .padding(22.0)
        .gap(12.0)
        .radius(10.0)
        .fill(k.c.bg_base)
        .child(heading(
            if form.review {
                "Review device change"
            } else if form.media_drive.is_some() {
                "Insert ISO media"
            } else {
                "Grow virtual disk"
            },
            k,
        ))
        .child(field("VM", &form.vm_name, k))
        .child(field("VM UUID", target.vm.to_string(), k));
    if form.review {
        if let Some(operation) = &form.operation {
            let summary = match operation {
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
            };
            body = body.child(caption(summary, k).wrap_anywhere());
        }
    } else if let Some(vbd) = form.media_drive {
        body = body.child(field("CD/DVD attachment", vbd.to_string(), k));
        let choices = media_choices(inventory, target.vm, vbd);
        if choices.is_empty() {
            let reason = super::unavailable(inventory, "VDI", "ISO image records")
                .or_else(|| super::unavailable(inventory, "SR", "Storage repositories"))
                .unwrap_or_else(|| {
                    "No eligible ISO images are available in the visible ISO storage repositories."
                        .into()
                });
            body = body.child(caption(reason, k).wrap());
        }
        for (image, label) in choices {
            let selected = matches!(form.operation,Some(DeviceOperation::InsertCd{image:current,..}) if current==image);
            body = body.child(
                Button::new(if selected {
                    format!("✓ {label}")
                } else {
                    label
                })
                .variant(ButtonVariant::Outline)
                .on_press(move |s: &mut State| {
                    if let Some(form) = form_mut(s) {
                        form.operation = Some(DeviceOperation::InsertCd { vbd, image });
                        form.error = None;
                    }
                })
                .build(k),
            );
        }
    } else if let Some((vdi, size)) = form.growth {
        body = body
            .child(field("Disk UUID", vdi.to_string(), k))
            .child(field("Current capacity", format!("{size} bytes"), k))
            .child(
                TextInput::new(&form.size)
                    .label("New capacity (bytes, KiB, MiB or GiB)")
                    .on_edit(|s: &mut State| &mut form_mut(s).expect("device editor open").size)
                    .build(k),
            );
    }
    if let Some(error) = form.error.as_ref().or(s.error.as_ref()) {
        body = body.child(
            caption(error, k)
                .wrap()
                .color(k.c.danger.icon)
                .role("alert"),
        );
    }
    let eligible = form
        .operation
        .as_ref()
        .map(|operation| operation.availability(inventory, target.vm));
    if form.review
        && let Some(status) = eligible.as_ref().filter(|status| !status.is_available())
    {
        body = body.child(caption(format!("This change is no longer available: {} Cancel and reopen the editor after refreshing inventory.", status.explanation()), k).wrap());
    }
    let target_current = s
        .devices
        .as_ref()
        .and_then(|d| s.snapshot.as_ref()?.observe_vm(d.vm).ok())
        == Some(form.target);
    if !target_current {
        body = body.child(
            caption(
                "The connection changed. Close and reopen the device editor to refresh it.",
                k,
            )
            .wrap(),
        );
    }
    let mut controls = row().gap(8.0).justify_end().child(
        Button::new("Cancel")
            .variant(ButtonVariant::Ghost)
            .on_press(close)
            .build(k),
    );
    if form.review {
        controls = controls.child(
            Button::new("Apply change")
                .variant(ButtonVariant::Outline)
                .disabled(
                    !ready(s)
                        || !target_current
                        || !eligible
                            .as_ref()
                            .is_some_and(|status| status.is_available()),
                )
                .on_press(apply)
                .build(k)
                .key("apply-device-change"),
        );
    } else {
        controls = controls.child(
            Button::new("Review change")
                .variant(ButtonVariant::Outline)
                .disabled(!ready(s) || !target_current)
                .on_press(review)
                .build(k)
                .key("review-device-change"),
        );
    }
    Some(
        el().w_full()
            .h_full()
            .absolute(0.0, 0.0)
            .center()
            .fill(rune::Color::BLACK.alpha(0.5))
            .role("dialog")
            .label("VM device change")
            .modal(false, close)
            .child(body.child(controls)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn growth_input_preserves_exact_capacity_and_rejects_overflow() {
        assert_eq!(parse_size("16 GiB").unwrap(), 16 << 30);
        assert_eq!(parse_size("123456512").unwrap(), 123456512);
        assert!(parse_size("0").is_err());
        assert!(parse_size("123456789").is_err());
        assert!(parse_size("2.5 GiB").is_err());
        assert!(parse_size("18446744073709551615 GiB").is_err());
    }
}
