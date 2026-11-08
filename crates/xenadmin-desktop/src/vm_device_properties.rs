use super::{State, caption, field, heading};
use rune::{El, ScrollSpec, TextEdit, col, el, row};
use rune_kit::{Button, ButtonVariant, Kit, TextInput};
use xenadmin_core::{
    DeviceCommand, DeviceOperation, DeviceProperty as Change, DeviceQos, DiskAccess, ObservedVm,
    PoolId, VbdId, VifId, VirtualBlockDevice, VirtualInterface, VmId,
};

pub(super) struct Form {
    pool: PoolId,
    vm: VmId,
    target: Option<ObservedVm>,
    vm_name: String,
    device_name: String,
    choices: Vec<Change>,
    notes: Vec<String>,
    choice: Option<Change>,
    name: TextEdit,
    description: TextEdit,
    value: TextEdit,
    enabled: bool,
    review: Option<Change>,
    error: Option<String>,
}
fn form_mut(s: &mut State) -> Option<&mut Form> {
    s.devices.as_mut()?.properties.as_mut()
}
fn close(s: &mut State) {
    if let Some(view) = &mut s.devices {
        view.properties = None;
    }
}
fn open(s: &mut State, disk: Option<VbdId>, nic: Option<VifId>) {
    let Some(view) = &s.devices else { return };
    let Some(i) = s.inventory.as_ref().filter(|i| i.pool_id == view.pool) else {
        return;
    };
    let mut choices = Vec::new();
    let mut notes = Vec::new();
    let device_name;
    if let Some(vbd) = disk {
        let Some(v) = i
            .details
            .vbds
            .get(&vbd)
            .filter(|v| v.vm == Some(view.vm) && v.kind == "Disk")
        else {
            return;
        };
        let Some(d) = v.vdi.and_then(|id| i.details.vdis.get(&id)) else {
            return;
        };
        device_name = format!("{} · disk {} · attachment {vbd}", d.name, d.id);
        let vdi = d.id;
        choices.push(Change::DiskMetadata {
            vbd,
            vdi,
            previous_name: d.name.clone(),
            previous_description: d.description.clone(),
            name: d.name.clone(),
            description: d.description.clone(),
        });
        if let Some(previous) = match v.mode.as_str() {
            "RO" => Some(DiskAccess::ReadOnly),
            "RW" => Some(DiskAccess::ReadWrite),
            _ => None,
        } {
            choices.push(Change::DiskAccess {
                vbd,
                vdi,
                previous,
                value: if previous == DiskAccess::ReadOnly {
                    DiskAccess::ReadWrite
                } else {
                    DiskAccess::ReadOnly
                },
            });
        }
        if v.userdevice
            .parse::<u8>()
            .is_ok_and(|n| n.to_string() == v.userdevice)
        {
            choices.push(Change::DiskPosition {
                vbd,
                vdi,
                previous: v.userdevice.clone(),
                value: String::new(),
            });
        }
        if let Some(previous) = v.bootable {
            choices.push(Change::Bootable {
                vbd,
                vdi,
                previous,
                value: !previous,
            });
        }
        if let Some(previous) = v
            .qos
            .as_ref()
            .filter(|q| matches!(q.algorithm.as_str(), "" | "ionice"))
        {
            choices.push(Change::DiskPriority {
                vbd,
                vdi,
                previous: previous.clone(),
                priority: None,
            });
            if !v
                .qos_supported_algorithms
                .as_ref()
                .is_some_and(|a| a.iter().any(|a| a == "ionice"))
            {
                notes.push("The host has not advertised I/O priority support. Disabling an existing policy remains possible; enabling one will be refused.".into());
            }
        } else {
            notes.push("Disk I/O priority is unavailable because the QoS configuration is unknown or uses another algorithm.".into());
        }
        if d.cbt_enabled == Some(true) {
            choices.push(Change::DisableCbt { vbd, vdi });
        }
        notes.push("Configuration changes require a halted VM and a disconnected device. Disk changes require an exclusive ordinary disk. Name and description can be changed while the VM is running.".into());
    } else if let Some(vif) = nic {
        let Some(v) = i.details.vifs.get(&vif).filter(|v| v.vm == Some(view.vm)) else {
            return;
        };
        device_name = format!("NIC {} · {vif}", v.device);
        if v.qos_known && matches!(v.qos_algorithm.as_str(), "" | "ratelimit") {
            choices.push(Change::NicRate {
                vif,
                previous: DeviceQos {
                    algorithm: v.qos_algorithm.clone(),
                    parameters: v.qos_parameters.clone(),
                },
                kilobytes_per_second: None,
            });
        } else {
            notes.push("Rate limit editing is unavailable because the QoS configuration is unknown or uses another algorithm.".into());
        }
        notes.push("Rate limit changes require a halted VM and a disconnected NIC. The host must advertise ratelimit support to enable a limit.".into());
    } else {
        return;
    }
    let form = Form {
        pool: view.pool,
        vm: view.vm,
        target: s
            .snapshot
            .as_ref()
            .and_then(|snap| snap.observe_vm(view.vm).ok()),
        vm_name: view.name.clone(),
        device_name,
        choices,
        notes,
        choice: None,
        name: TextEdit::new(""),
        description: TextEdit::new(""),
        value: TextEdit::new(""),
        enabled: false,
        review: None,
        error: None,
    };
    let view = s.devices.as_mut().expect("device view open");
    view.form = None;
    view.lifecycle = None;
    view.properties = Some(form);
    s.error = None;
}
pub(super) fn disk_control(s: &State, v: &VirtualBlockDevice, k: &Kit) -> El {
    if v.kind != "Disk" {
        return el();
    }
    let id = v.id;
    Button::new("Disk properties…")
        .variant(ButtonVariant::Outline)
        .disabled(!s.demo && !s.can_mutate())
        .on_press(move |s: &mut State| open(s, Some(id), None))
        .build(k)
        .key(format!("disk-properties-{id}"))
}
pub(super) fn nic_control(s: &State, v: &VirtualInterface, k: &Kit) -> El {
    let id = v.id;
    Button::new("NIC rate limit…")
        .variant(ButtonVariant::Outline)
        .disabled(!s.demo && !s.can_mutate())
        .on_press(move |s: &mut State| {
            open(s, None, Some(id));
            if let Some(choice) = form_mut(s).and_then(|f| f.choices.first().cloned()) {
                choose(s, choice);
            }
        })
        .build(k)
        .key(format!("nic-properties-{id}"))
}
fn choose(s: &mut State, choice: Change) {
    let Some(f) = form_mut(s) else { return };
    f.value = TextEdit::new("");
    f.enabled = false;
    match &choice {
        Change::DiskMetadata {
            name, description, ..
        } => {
            f.name = TextEdit::new(name);
            f.description = TextEdit::new(description);
        }
        Change::DiskPriority { previous, .. } => {
            f.enabled = previous.algorithm == "ionice";
            f.value = TextEdit::new(
                previous
                    .parameters
                    .get("class")
                    .cloned()
                    .unwrap_or_else(|| "4".into()),
            );
        }
        Change::NicRate { previous, .. } => {
            f.enabled = previous.algorithm == "ratelimit";
            f.value = TextEdit::new(
                previous
                    .parameters
                    .get("kbps")
                    .cloned()
                    .unwrap_or_else(|| "12500".into()),
            );
        }
        _ => {}
    }
    f.choice = Some(choice);
    f.review = None;
    f.error = None;
}
fn build(f: &Form) -> Result<Change, String> {
    let mut choice = f.choice.clone().ok_or("Select a property to change.")?;
    match &mut choice {
        Change::DiskMetadata {
            name,
            description,
            previous_name,
            previous_description,
            ..
        } => {
            *name = f.name.text.clone();
            *description = f.description.text.clone();
            if name.trim().is_empty()
                || name.chars().count() > 255
                || name.chars().any(char::is_control)
                || description.len() > 16_384
                || description.contains('\0')
            {
                return Err(
                    "Enter a nonempty name up to 255 characters and a description up to 16 KiB."
                        .into(),
                );
            }
            if name == previous_name && description == previous_description {
                return Err("Change the name or description before reviewing.".into());
            }
        }
        Change::DiskPosition {
            previous, value, ..
        } => {
            *value = f.value.text.clone();
            if !value.parse::<u8>().is_ok_and(|n| n.to_string() == *value) {
                return Err("Enter an unused slot from 0 to 255 without leading zeros.".into());
            }
            if value == previous {
                return Err("Choose a different device position.".into());
            }
        }
        Change::DiskPriority {
            previous, priority, ..
        } => {
            *priority = if f.enabled {
                Some(
                    f.value
                        .text
                        .parse::<u8>()
                        .ok()
                        .filter(|n| *n <= 7)
                        .ok_or("Enter a priority from 0 (highest) to 7 (lowest).")?,
                )
            } else {
                None
            };
            let unchanged = match priority {
                None => previous.algorithm.is_empty(),
                Some(p) => {
                    previous.algorithm == "ionice"
                        && previous.parameters.get("class") == Some(&p.to_string())
                        && previous.parameters.get("sched").is_some_and(|v| v == "be")
                }
            };
            if unchanged {
                return Err("Change the priority before reviewing.".into());
            }
        }
        Change::NicRate {
            previous,
            kilobytes_per_second,
            ..
        } => {
            *kilobytes_per_second = if f.enabled {
                Some(
                    f.value
                        .text
                        .parse::<u64>()
                        .ok()
                        .filter(|n| *n > 0 && *n <= i64::MAX as u64)
                        .ok_or("Enter a positive whole number of kilobytes per second.")?,
                )
            } else {
                None
            };
            let unchanged = match kilobytes_per_second {
                None => previous.algorithm.is_empty(),
                Some(n) => {
                    previous.algorithm == "ratelimit"
                        && previous.parameters.get("kbps") == Some(&n.to_string())
                }
            };
            if unchanged {
                return Err("Change the rate limit before reviewing.".into());
            }
        }
        Change::DisableCbt { vdi, .. } if f.value.text != vdi.to_string() => {
            return Err(
                "Enter the exact disk UUID to confirm disabling changed-block tracking.".into(),
            );
        }
        _ => {}
    }
    Ok(choice)
}
fn operation(change: &Change) -> DeviceOperation {
    DeviceOperation::Property {
        change: change.clone(),
    }
}
fn current(s: &State, f: &Form) -> bool {
    s.inventory
        .as_ref()
        .is_some_and(|i| i.pool_id == f.pool && i.vms.contains_key(&f.vm))
        && (s.demo
            || f.target.is_some_and(|t| {
                s.snapshot.as_ref().and_then(|v| v.observe_vm(f.vm).ok()) == Some(t)
            }))
}
fn review(s: &mut State) {
    let Some(f) = s.devices.as_ref().and_then(|d| d.properties.as_ref()) else {
        return;
    };
    let result = build(f).and_then(|change| {
        if !current(s, f) {
            return Err("The connection changed. Close and reopen the editor.".into());
        }
        let a =
            operation(&change).availability(s.inventory.as_ref().expect("current inventory"), f.vm);
        if !a.is_available() {
            return Err(a.explanation());
        }
        Ok(change)
    });
    if let Some(f) = form_mut(s) {
        match result {
            Ok(change) => {
                f.review = Some(change);
                f.error = None;
            }
            Err(e) => f.error = Some(e),
        }
    }
}
fn apply(s: &mut State) {
    let Some(f) = s.devices.as_ref().and_then(|d| d.properties.as_ref()) else {
        return;
    };
    let Some(target) = f.target else { return };
    let Some(change) = &f.review else { return };
    if s.demo
        || !s.can_mutate()
        || !current(s, f)
        || !operation(change)
            .availability(s.inventory.as_ref().expect("current inventory"), f.vm)
            .is_available()
    {
        return;
    }
    let command = DeviceCommand {
        vm: target,
        operation: operation(change),
    };
    s.submit_command(xenadmin_core::VmCommand::Device(command));
    if s.error.is_none() {
        close(s);
    }
}
fn summary(change: &Change) -> String {
    match change {
        Change::DiskMetadata { previous_name, previous_description, name, description, .. } => format!("Name: {previous_name} → {name}\nDescription: {previous_description} → {description}\nThese are separate changes. A failure can leave only the name updated."),
        Change::DiskAccess { previous, value, .. } => format!("Access: {} → {}. The attachment and disk identities are retained.", access(*previous), access(*value)),
        Change::DiskPosition { previous, value, .. } => format!("Device position: {previous} → {value}. The new slot must be unused. Guest device naming or boot configuration may change."),
        Change::Bootable { previous, value, .. } => format!("Bootable: {previous} → {value}. This changes the disk attachment flag; it does not reorder the VM firmware boot sequence."),
        Change::DiskPriority { previous, priority, .. } => format!("Previous algorithm: {} · class {}. New I/O priority: {}. Other QoS parameters are retained. Parameter and algorithm changes are tracked separately.", qos_name(&previous.algorithm), previous.parameters.get("class").map_or("not set", String::as_str), priority.map_or_else(|| "disabled".into(), |p| format!("{p} (0 highest, 7 lowest)"))),
        Change::NicRate { previous, kilobytes_per_second, .. } => format!("Previous algorithm: {} · limit {} KB/s. New rate limit: {}. Units are kilobytes per second (KB/s). Other QoS parameters, the NIC UUID and MAC address are retained.", qos_name(&previous.algorithm), previous.parameters.get("kbps").map_or("not set", String::as_str), kilobytes_per_second.map_or_else(|| "disabled".into(), |n| format!("{n} KB/s"))),
        Change::DisableCbt { .. } => "Disable changed-block tracking for this disk. This breaks the existing incremental backup chain; the backup system may need a new full backup. Existing disk data is retained. Coordinate this with the backup owner before applying.".into(),
    }
}
fn qos_name(s: &str) -> &str {
    if s.is_empty() { "disabled" } else { s }
}
fn access(v: DiskAccess) -> &'static str {
    match v {
        DiskAccess::ReadOnly => "read only",
        DiskAccess::ReadWrite => "read/write",
    }
}
fn input(f: &Form, label: &'static str, k: &Kit) -> El {
    col().gap(5.0).child(caption(label, k)).child(
        TextInput::new(&f.value)
            .label(label)
            .on_edit(|s: &mut State| &mut form_mut(s).expect("properties open").value)
            .build(k)
            .key("property-value"),
    )
}
pub(super) fn dialog(s: &State, k: &Kit) -> Option<El> {
    let f = s.devices.as_ref()?.properties.as_ref()?;
    let mut body = col()
        .w((s.window_width - 40.0).clamp(280.0, 660.0))
        .max_h((s.window_height - 60.0).clamp(280.0, 800.0))
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .padding(20.0)
        .gap(12.0)
        .radius(10.0)
        .fill(k.c.bg_base)
        .child(heading(
            if f.review.is_some() {
                "Review device properties"
            } else {
                "Device properties"
            },
            k,
        ))
        .child(field("VM", &f.vm_name, k))
        .child(field("VM UUID", f.vm.to_string(), k))
        .child(field("Device", &f.device_name, k));
    if let Some(change) = &f.review {
        body = body
            .child(heading(change.label(), k))
            .child(caption(summary(change), k).wrap_anywhere());
    } else if let Some(choice) = &f.choice {
        body = body.child(heading(choice.label(), k));
        match choice {
            Change::DiskMetadata { .. } => {
                body = body
                    .child(caption("Disk name", k))
                    .child(
                        TextInput::new(&f.name)
                            .label("Disk name")
                            .on_edit(|s: &mut State| {
                                &mut form_mut(s).expect("properties open").name
                            })
                            .build(k)
                            .key("property-name"),
                    )
                    .child(caption("Description", k))
                    .child(
                        TextInput::new(&f.description)
                            .label("Description")
                            .on_edit(|s: &mut State| {
                                &mut form_mut(s).expect("properties open").description
                            })
                            .build(k)
                            .key("property-description"),
                    );
            }
            Change::DiskPosition { previous, .. } => {
                body = body
                    .child(field("Current position", previous, k))
                    .child(input(f, "New unused device position (0–255)", k));
            }
            Change::DiskPriority { .. } | Change::NicRate { .. } => {
                body = body.child(
                    Button::new(if f.enabled {
                        "Policy enabled · click to disable"
                    } else {
                        "Policy disabled · click to enable"
                    })
                    .variant(ButtonVariant::Outline)
                    .on_press(|s: &mut State| {
                        if let Some(f) = form_mut(s) {
                            f.enabled = !f.enabled;
                        }
                    })
                    .build(k)
                    .key("property-enabled"),
                );
                if f.enabled {
                    body = body.child(input(
                        f,
                        if matches!(choice, Change::DiskPriority { .. }) {
                            "I/O priority: 0 (highest) to 7 (lowest)"
                        } else {
                            "Rate limit (kilobytes per second, KB/s)"
                        },
                        k,
                    ));
                }
            }
            Change::DisableCbt { vdi, .. } => {
                body = body
                    .child(caption(summary(choice), k).wrap())
                    .child(field("Disk UUID to confirm", vdi.to_string(), k))
                    .child(input(f, "Type the disk UUID", k));
            }
            _ => body = body.child(caption(summary(choice), k).wrap()),
        }
    } else {
        for (index, choice) in f.choices.iter().enumerate() {
            let choice = choice.clone();
            let a = s
                .inventory
                .as_ref()
                .filter(|i| i.pool_id == f.pool)
                .map(|i| operation(&choice).availability(i, f.vm));
            let enabled = a.as_ref().is_some_and(|a| a.is_available());
            body = body.child(
                Button::new(choice.label())
                    .variant(ButtonVariant::Outline)
                    .disabled(!enabled)
                    .on_press(move |s: &mut State| choose(s, choice.clone()))
                    .build(k)
                    .key(format!("property-choice-{index}")),
            );
            if let Some(a) = a.filter(|a| !a.is_available()) {
                body = body.child(caption(a.explanation(), k).wrap());
            }
        }
        for note in &f.notes {
            body = body.child(caption(note, k).wrap());
        }
    }
    let valid = current(s, f);
    if !valid {
        body = body.child(
            caption(
                "The connection or VM changed. Close and reopen the editor.",
                k,
            )
            .wrap(),
        );
    }
    if s.demo {
        body =
            body.child(caption("Synthetic review · no host changes can be submitted.", k).wrap());
    }
    if let Some(error) = f.error.as_ref().or(s.error.as_ref()) {
        body = body.child(
            caption(error, k)
                .wrap()
                .color(k.c.danger.icon)
                .role("alert"),
        );
    }
    let mut buttons = row().w_full().flex_wrap().gap(8.0).justify_end().child(
        Button::new("Cancel")
            .variant(ButtonVariant::Ghost)
            .on_press(close)
            .build(k)
            .key("property-cancel"),
    );
    if f.choice.is_some() {
        buttons = buttons.child(
            Button::new("Back")
                .variant(ButtonVariant::Ghost)
                .on_press(|s: &mut State| {
                    if let Some(f) = form_mut(s) {
                        if f.review.take().is_none() {
                            f.choice = None;
                        }
                        f.error = None;
                    }
                })
                .build(k)
                .key("property-back"),
        );
        if let Some(change) = &f.review {
            let allowed = s
                .inventory
                .as_ref()
                .is_some_and(|i| operation(change).availability(i, f.vm).is_available());
            buttons = buttons.child(
                Button::new("Apply change")
                    .variant(if matches!(change, Change::DisableCbt { .. }) {
                        ButtonVariant::Danger
                    } else {
                        ButtonVariant::Outline
                    })
                    .disabled(s.demo || !s.can_mutate() || !valid || !allowed)
                    .on_press(apply)
                    .build(k)
                    .key("property-apply"),
            );
        } else {
            buttons = buttons.child(
                Button::new("Review change")
                    .variant(ButtonVariant::Outline)
                    .disabled(!valid || (!s.demo && !s.can_mutate()))
                    .on_press(review)
                    .build(k)
                    .key("property-review"),
            );
        }
    }
    Some(
        el().w_full()
            .h_full()
            .absolute(0.0, 0.0)
            .center()
            .fill(rune::Color::BLACK.alpha(0.5))
            .role("dialog")
            .label("Device properties")
            .modal(false, close)
            .child(body.child(buttons)),
    )
}
pub(super) fn scene(s: &mut State) {
    crate::demo(s);
    s.table.select("30000000-0000-0000-0000-000000000000");
    super::open(s);
    open(
        s,
        Some(VbdId::parse("72000000-0000-0000-0000-000000000001").unwrap()),
        None,
    );
    if let Some(choice) = form_mut(s).and_then(|f| {
        f.choices
            .iter()
            .find(|c| matches!(c, Change::DiskPriority { .. }))
            .cloned()
    }) {
        choose(s, choice);
    }
}

#[cfg(test)]
#[path = "vm_device_properties_tests.rs"]
mod tests;
