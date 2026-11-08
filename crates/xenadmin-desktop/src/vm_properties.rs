use super::*;
use xenadmin_core::{MemoryLimits, ObservedVm, VmDetails, VmSettingEdit, VmSettingsPatch};

#[derive(Clone, Copy)]
enum Field {
    Name,
    Description,
    MaxCpus,
    StartupCpus,
    StaticMin,
    DynamicMin,
    DynamicMax,
    StaticMax,
    BootOrder,
    CpuTopology,
    CpuWeight,
    ShadowMultiplier,
}
pub(super) struct Editor {
    target: Option<ObservedVm>,
    id: VmId,
    original: Vm,
    details: VmDetails,
    name: TextEdit,
    description: TextEdit,
    max_cpus: TextEdit,
    startup_cpus: TextEdit,
    static_min: TextEdit,
    dynamic_min: TextEdit,
    dynamic_max: TextEdit,
    static_max: TextEdit,
    boot_order: TextEdit,
    topology: TextEdit,
    cpu_weight: TextEdit,
    shadow: TextEdit,
    autostart: bool,
    review: Option<VmSettingsPatch>,
    error: Option<String>,
}
impl Editor {
    fn field(&self, field: Field) -> &TextEdit {
        match field {
            Field::Name => &self.name,
            Field::Description => &self.description,
            Field::MaxCpus => &self.max_cpus,
            Field::StartupCpus => &self.startup_cpus,
            Field::StaticMin => &self.static_min,
            Field::DynamicMin => &self.dynamic_min,
            Field::DynamicMax => &self.dynamic_max,
            Field::StaticMax => &self.static_max,
            Field::BootOrder => &self.boot_order,
            Field::CpuTopology => &self.topology,
            Field::CpuWeight => &self.cpu_weight,
            Field::ShadowMultiplier => &self.shadow,
        }
    }
    fn field_mut(&mut self, field: Field) -> &mut TextEdit {
        match field {
            Field::Name => &mut self.name,
            Field::Description => &mut self.description,
            Field::MaxCpus => &mut self.max_cpus,
            Field::StartupCpus => &mut self.startup_cpus,
            Field::StaticMin => &mut self.static_min,
            Field::DynamicMin => &mut self.dynamic_min,
            Field::DynamicMax => &mut self.dynamic_max,
            Field::StaticMax => &mut self.static_max,
            Field::BootOrder => &mut self.boot_order,
            Field::CpuTopology => &mut self.topology,
            Field::CpuWeight => &mut self.cpu_weight,
            Field::ShadowMultiplier => &mut self.shadow,
        }
    }
    fn memory(&self) -> Option<MemoryLimits> {
        Some(MemoryLimits {
            static_min: self.details.memory_static_min?,
            dynamic_min: self.details.memory_dynamic_min?,
            dynamic_max: self.details.memory_dynamic_max?,
            static_max: self.details.memory_static_max?,
        })
    }
    fn patch(&self) -> Result<VmSettingsPatch, String> {
        let mut edits = Vec::new();
        if self.name.text != self.original.name {
            edits.push(VmSettingEdit::Name {
                previous: self.original.name.clone(),
                value: self.name.text.clone(),
            });
        }
        if self.description.text != self.original.description {
            edits.push(VmSettingEdit::Description {
                previous: self.original.description.clone(),
                value: self.description.text.clone(),
            });
        }
        if self.original.power_state == PowerState::Halted {
            if let (Some(previous_max), Some(previous_startup)) =
                (self.details.vcpus_max, self.details.vcpus_at_startup)
            {
                let max = self
                    .max_cpus
                    .text
                    .trim()
                    .parse::<u64>()
                    .map_err(|_| "Enter whole positive CPU counts")?;
                let startup = self
                    .startup_cpus
                    .text
                    .trim()
                    .parse::<u64>()
                    .map_err(|_| "Enter whole positive CPU counts")?;
                if (max, startup) != (previous_max, previous_startup) {
                    edits.push(VmSettingEdit::Cpus {
                        previous_max,
                        previous_startup,
                        max,
                        startup,
                    });
                }
            }
            if let Some(previous) = self.memory() {
                let value = MemoryLimits {
                    static_min: parse_memory(&self.static_min.text)?,
                    dynamic_min: parse_memory(&self.dynamic_min.text)?,
                    dynamic_max: parse_memory(&self.dynamic_max.text)?,
                    static_max: parse_memory(&self.static_max.text)?,
                };
                if value != previous {
                    edits.push(VmSettingEdit::Memory { previous, value });
                }
            }
            if let Some(previous) = &self.details.boot_order
                && &self.boot_order.text != previous
            {
                edits.push(VmSettingEdit::BootOrder {
                    previous: previous.clone(),
                    value: self.boot_order.text.clone(),
                });
            }
        }
        if self.original.power_state == PowerState::Running {
            if let (Some(previous), Some(max)) =
                (self.details.vcpus_at_startup, self.details.vcpus_max)
            {
                let value = self
                    .startup_cpus
                    .text
                    .parse::<u64>()
                    .map_err(|_| "Enter a whole live vCPU count")?;
                if value != previous {
                    edits.push(VmSettingEdit::LiveCpus {
                        previous,
                        max,
                        value,
                    });
                }
            }
            if let Some(previous) = self.memory() {
                let min = parse_memory(&self.dynamic_min.text)?;
                let max = parse_memory(&self.dynamic_max.text)?;
                if min != previous.dynamic_min || max != previous.dynamic_max {
                    edits.push(VmSettingEdit::DynamicMemory { previous, min, max });
                }
            }
        }
        if self.original.power_state == PowerState::Halted
            && self.details.topology_known
            && self.topology.text != self.details.cores_per_socket.clone().unwrap_or_default()
        {
            edits.push(VmSettingEdit::CpuTopology {
                previous: self.details.cores_per_socket.clone(),
                cores_per_socket: self
                    .topology
                    .text
                    .parse::<u64>()
                    .map_err(|_| "Enter a positive cores-per-socket count")?,
            });
        }
        if self.details.cpu_weight_known
            && self.cpu_weight.text
                != self
                    .details
                    .cpu_weight
                    .clone()
                    .unwrap_or_else(|| "256".into())
        {
            edits.push(VmSettingEdit::CpuWeight {
                previous: self.details.cpu_weight.clone(),
                weight: self
                    .cpu_weight
                    .text
                    .parse::<u16>()
                    .map_err(|_| "CPU weight must be 1 to 65535")?,
            });
        }
        if let Some(previous) = &self.details.shadow_multiplier
            && self.shadow.text != *previous
        {
            edits.push(VmSettingEdit::ShadowMultiplier {
                previous: previous.clone(),
                value: self.shadow.text.clone(),
                live: self.original.power_state == PowerState::Running,
            });
        }
        if let Some(previous) = self.details.auto_power_on
            && self.autostart != previous
        {
            edits.push(VmSettingEdit::Autostart {
                previous,
                value: self.autostart,
            });
        }
        if edits.is_empty() {
            return Err("No properties have changed".into());
        }
        VmSettingsPatch::new(edits).map_err(|e| e.to_string())
    }
}
fn memory_text(value: Option<u64>) -> String {
    value
        .map(|v| {
            if v % (1 << 20) == 0 {
                format!("{} MiB", v / (1 << 20))
            } else {
                v.to_string()
            }
        })
        .unwrap_or_default()
}
fn parse_memory(text: &str) -> Result<u64, String> {
    let compact = text.split_whitespace().collect::<String>();
    let (number, multiplier) = if let Some(n) = compact.strip_suffix("GiB") {
        (n, 1_u64 << 30)
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
        .ok_or_else(|| {
            "Memory must be a whole byte count, or a whole number with KiB, MiB or GiB".into()
        })
}
pub(super) fn open(s: &mut State) {
    if !s.demo && !s.can_mutate() {
        s.error = Some("Open properties after reconnecting with a writable connection.".into());
        return;
    }
    let Some(id) = s.selected().and_then(|r| r.vm) else {
        return;
    };
    let Some(inventory) = &s.inventory else {
        return;
    };
    let Some(original) = inventory.vms.get(&id).cloned() else {
        return;
    };
    let details = inventory.details.vms.get(&id).cloned().unwrap_or_default();
    s.properties = Some(Editor {
        target: s
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.observe_vm(id).ok()),
        id,
        name: TextEdit::new(&original.name),
        description: TextEdit::new(&original.description),
        max_cpus: TextEdit::new(details.vcpus_max.map(|v| v.to_string()).unwrap_or_default()),
        startup_cpus: TextEdit::new(
            details
                .vcpus_at_startup
                .map(|v| v.to_string())
                .unwrap_or_default(),
        ),
        static_min: TextEdit::new(memory_text(details.memory_static_min)),
        dynamic_min: TextEdit::new(memory_text(details.memory_dynamic_min)),
        dynamic_max: TextEdit::new(memory_text(details.memory_dynamic_max)),
        static_max: TextEdit::new(memory_text(details.memory_static_max)),
        boot_order: TextEdit::new(details.boot_order.clone().unwrap_or_default()),
        topology: TextEdit::new(details.cores_per_socket.clone().unwrap_or_default()),
        cpu_weight: TextEdit::new(details.cpu_weight.clone().unwrap_or_else(|| "256".into())),
        shadow: TextEdit::new(details.shadow_multiplier.clone().unwrap_or_default()),
        autostart: details.auto_power_on.unwrap_or(false),
        original,
        details,
        review: None,
        error: None,
    });
}
fn input(editor: &Editor, field: Field, label: &str, k: &Kit) -> El {
    col().gap(4.0).child(caption(label, k)).child(
        TextInput::new(editor.field(field))
            .label(label)
            .on_edit(move |s: &mut State| {
                s.properties
                    .as_mut()
                    .expect("properties dialog is open")
                    .field_mut(field)
            })
            .build(k),
    )
}
fn submit(s: &mut State) {
    let Some(editor) = &s.properties else { return };
    let (Some(vm), Some(patch)) = (editor.target, editor.review.clone()) else {
        return;
    };
    s.submit_command(xenadmin_core::VmCommand::Configure { vm, patch });
    if s.error.is_none() {
        s.properties = None;
    }
}
pub(super) fn dialog(s: &State, k: &Kit) -> El {
    let Some(editor) = &s.properties else {
        return el();
    };
    let mut body = col()
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        })
        .padding(24.0)
        .gap(12.0)
        .radius(10.0)
        .fill(k.c.bg_base)
        .child(heading(
            if editor.review.is_some() {
                "Review VM property changes"
            } else {
                "VM properties"
            },
            k,
        ))
        .child(
            caption(format!("{}\nUUID: {}", editor.original.name, editor.id), k).wrap_anywhere(),
        );
    if let Some(patch) = &editor.review {
        for edit in patch.edits() {
            let summary = match edit {
                VmSettingEdit::LiveCpus {
                    previous,
                    max,
                    value,
                } => format!("Live vCPUs: {previous} → {value} (maximum {max})"),
                VmSettingEdit::DynamicMemory { previous, min, max } => format!(
                    "Dynamic memory: {}–{} → {min}–{max} bytes; static limits unchanged",
                    previous.dynamic_min, previous.dynamic_max
                ),
                VmSettingEdit::CpuTopology {
                    previous,
                    cores_per_socket,
                } => format!(
                    "Cores per socket: {} → {cores_per_socket}",
                    previous.as_deref().unwrap_or("default")
                ),
                VmSettingEdit::CpuWeight { previous, weight } => format!(
                    "CPU scheduling weight: {} → {weight}",
                    previous.as_deref().unwrap_or("default (256)")
                ),
                VmSettingEdit::ShadowMultiplier {
                    previous,
                    value,
                    live,
                } => format!(
                    "Shadow memory multiplier: {previous} → {value}{}",
                    if *live { " (live)" } else { "" }
                ),
                VmSettingEdit::Name { previous, value } => format!("Name: {previous} → {value}"),
                VmSettingEdit::Description { previous, value } => {
                    format!("Description: {previous} → {value}")
                }
                VmSettingEdit::Cpus {
                    previous_max,
                    previous_startup,
                    max,
                    startup,
                } => format!(
                    "vCPUs: maximum {previous_max} → {max}; startup {previous_startup} → {startup}"
                ),
                VmSettingEdit::Memory { previous, value } => format!(
                    "Memory (bytes):\nstatic minimum {} → {}\ndynamic minimum {} → {}\ndynamic maximum {} → {}\nstatic maximum {} → {}",
                    previous.static_min,
                    value.static_min,
                    previous.dynamic_min,
                    value.dynamic_min,
                    previous.dynamic_max,
                    value.dynamic_max,
                    previous.static_max,
                    value.static_max
                ),
                VmSettingEdit::BootOrder { previous, value } => {
                    format!("Boot order: {previous} → {value}")
                }
                VmSettingEdit::Autostart { previous, value } => {
                    format!("Autostart: {previous} → {value}")
                }
            };
            body = body.child(caption(summary, k).wrap_anywhere());
        }
        body=body.child(caption("Changes apply in steps. A failure stops remaining steps; already saved values remain applied. Changed values are checked on the host before and after each step.",k).wrap());
    } else {
        body = body
            .child(input(editor, Field::Name, "Name", k).key("property-name"))
            .child(input(editor, Field::Description, "Description", k));
        if editor.original.power_state == PowerState::Halted {
            if editor.details.vcpus_max.is_some() && editor.details.vcpus_at_startup.is_some() {
                body = body
                    .child(input(editor, Field::MaxCpus, "Maximum vCPUs", k))
                    .child(input(editor, Field::StartupCpus, "Startup vCPUs", k));
            }
            if editor.memory().is_some() {
                body=body.child(caption("Memory accepts whole bytes, KiB, MiB or GiB. Minimum ≤ dynamic range ≤ maximum.",k).wrap());
                for (field, label) in [
                    (Field::StaticMin, "Static minimum"),
                    (Field::DynamicMin, "Dynamic minimum"),
                    (Field::DynamicMax, "Dynamic maximum"),
                    (Field::StaticMax, "Static maximum"),
                ] {
                    body = body.child(input(editor, field, label, k));
                }
            }
            if editor.details.boot_order.is_some() {
                body = body.child(input(
                    editor,
                    Field::BootOrder,
                    "Boot order (c=disk, d=DVD, n=network)",
                    k,
                ));
            }
        } else if editor.original.power_state == PowerState::Running {
            body=body.child(caption("Live changes apply to a running guest. vCPUs can only be increased within its current maximum. Ballooning requires compatible guest tools and may reduce the memory available to applications.",k).wrap());
            if editor.details.vcpus_max.is_some() && editor.details.vcpus_at_startup.is_some() {
                body = body.child(input(editor, Field::StartupCpus, "Live vCPUs", k));
            }
            if editor.memory().is_some() {
                body = body
                    .child(input(
                        editor,
                        Field::DynamicMin,
                        "Dynamic memory minimum",
                        k,
                    ))
                    .child(input(
                        editor,
                        Field::DynamicMax,
                        "Dynamic memory maximum",
                        k,
                    ));
            }
            body = body.child(
                caption(
                    "Static memory limits, maximum vCPUs and boot order require a halted VM.",
                    k,
                )
                .wrap(),
            );
        }
        if editor.original.power_state == PowerState::Halted && editor.details.topology_known {
            body = body.child(input(editor, Field::CpuTopology, "Cores per socket", k));
        }
        if matches!(
            editor.original.power_state,
            PowerState::Halted | PowerState::Running
        ) {
            if editor.details.cpu_weight_known {
                body = body.child(input(
                    editor,
                    Field::CpuWeight,
                    "CPU scheduling weight (1–65535)",
                    k,
                ));
            }
            let hvm = match editor.details.domain_type.as_deref() {
                Some("hvm") => true,
                None | Some("unspecified") => editor
                    .details
                    .boot_policy
                    .as_ref()
                    .is_some_and(|p| !p.is_empty()),
                _ => false,
            };
            if hvm && editor.details.shadow_multiplier.is_some() {
                body = body.child(input(
                    editor,
                    Field::ShadowMultiplier,
                    "Shadow memory multiplier (1–1000)",
                    k,
                ));
            }
        }
        if editor.details.auto_power_on.is_some() {
            body = body.child(
                Checkbox::new(editor.autostart)
                    .label("Start automatically with the pool")
                    .on_toggle(|s: &mut State| {
                        if let Some(editor) = &mut s.properties {
                            editor.autostart = !editor.autostart
                        }
                    })
                    .build(k),
            );
        }
    }
    if !s.demo
        && editor.target
            != s.snapshot
                .as_ref()
                .and_then(|s| s.observe_vm(editor.id).ok())
    {
        body=body.child(caption("The connection changed. Close this editor and reopen properties to refresh the target and values.",k).wrap().role("status"));
    }
    if s.demo {
        body = body.child(caption("Demo preview • no host changes can be submitted.", k).wrap());
    }
    if let Some(error) = editor.error.as_ref().or(s.error.as_ref()) {
        body = body.child(
            caption(error, k)
                .wrap()
                .color(k.c.danger.icon)
                .role("alert"),
        );
    }
    let mut controls = row().gap(8.0).justify_end().child(
        Button::new("Cancel")
            .variant(ButtonVariant::Ghost)
            .on_press(|s: &mut State| s.properties = None)
            .build(k),
    );
    if editor.review.is_some() {
        controls = controls
            .child(
                Button::new("Back")
                    .variant(ButtonVariant::Ghost)
                    .on_press(|s: &mut State| {
                        if let Some(editor) = &mut s.properties {
                            editor.review = None
                        }
                    })
                    .build(k),
            )
            .child(
                Button::new("Apply changes")
                    .variant(ButtonVariant::Outline)
                    .disabled(
                        !s.can_mutate()
                            || editor.target.is_none()
                            || editor.target
                                != s.snapshot
                                    .as_ref()
                                    .and_then(|s| s.observe_vm(editor.id).ok()),
                    )
                    .on_press(submit)
                    .build(k)
                    .key("apply-properties"),
            );
    } else {
        controls = controls.child(
            Button::new("Review changes")
                .variant(ButtonVariant::Outline)
                .on_press(|s: &mut State| {
                    if let Some(editor) = &mut s.properties {
                        match editor.patch() {
                            Ok(patch) => {
                                editor.review = Some(patch);
                                editor.error = None
                            }
                            Err(error) => editor.error = Some(error),
                        }
                    }
                })
                .build(k)
                .key("review-properties"),
        );
    }
    el().key("properties-dialog")
        .role("dialog")
        .label("VM properties")
        .w_full()
        .h_full()
        .absolute(0.0, 0.0)
        .center()
        .fill(rune::Color::BLACK.alpha(0.5))
        .modal(false, |s: &mut State| s.properties = None)
        .child(
            col()
                .w((s.window_width - 40.0).clamp(280.0, 580.0))
                .h((s.window_height - 48.0).clamp(280.0, 650.0))
                .radius(10.0)
                .fill(k.c.bg_base)
                .child(body)
                .child(controls.w_full().padding(16.0)),
        )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn properties_review_is_reachable_and_demo_apply_is_disabled() {
        let mut state = init();
        demo(&mut state);
        state.table.select("30000000-0000-0000-0000-000000000001");
        open(&mut state);
        state.properties.as_mut().unwrap().name = TextEdit::new("Reviewed fixture name");
        let mut runtime = rune::Runtime::new(
            state,
            super::super::view,
            (513.0, 652.0),
            rune::Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        for _ in 0..30 {
            runtime.frame(1.0 / 60.0);
        }
        let (x, y) = runtime.find("review-properties").unwrap();
        runtime.pointer_down(x, y);
        runtime.pointer_up(x, y);
        runtime.frame(0.0);
        assert!(runtime.state.properties.as_ref().unwrap().review.is_some());
        for _ in 0..30 {
            runtime.frame(1.0 / 60.0);
        }
        let (x, y) = runtime.find("apply-properties").unwrap();
        runtime.pointer_down(x, y);
        runtime.pointer_up(x, y);
        runtime.frame(0.0);
        assert!(runtime.state.operations.is_empty());
        assert!(runtime.state.properties.is_some());
    }

    #[test]
    fn memory_input_is_exact_and_bounded() {
        assert_eq!(parse_memory("128 GiB").unwrap(), 128 << 30);
        assert_eq!(parse_memory("123456789").unwrap(), 123456789);
        assert!(parse_memory("18446744073709551615 GiB").is_err());
        assert!(parse_memory("1.5 MiB").is_err());
    }
    #[test]
    fn property_draft_keeps_the_original_target_and_values() {
        let mut state = init();
        demo(&mut state);
        state.table.select("30000000-0000-0000-0000-000000000000");
        open(&mut state);
        state.table.select("30000000-0000-0000-0000-000000000001");
        let editor = state.properties.as_mut().unwrap();
        editor.name = TextEdit::new("Changed demo");
        let patch = editor.patch().unwrap();
        assert_eq!(
            editor.id.to_string(),
            "30000000-0000-0000-0000-000000000000"
        );
        assert!(
            matches!(&patch.edits()[0],VmSettingEdit::Name{previous,value}if previous==&editor.original.name&&value=="Changed demo")
        );
    }
    #[test]
    fn running_guest_draft_uses_live_cpu_and_dynamic_memory_commands() {
        let mut s = init();
        demo(&mut s);
        let id = VmId::parse("30000000-0000-0000-0000-000000000001").unwrap();
        Arc::make_mut(s.inventory.as_mut().unwrap())
            .details
            .vms
            .get_mut(&id)
            .unwrap()
            .vcpus_max = Some(8);
        s.table.select(id.to_string());
        open(&mut s);
        let editor = s.properties.as_mut().unwrap();
        editor.startup_cpus = TextEdit::new("4");
        editor.dynamic_min = TextEdit::new("2 GiB");
        editor.cpu_weight = TextEdit::new("512");
        let patch = editor.patch().unwrap();
        assert!(patch.edits().iter().any(|e| matches!(
            e,
            VmSettingEdit::LiveCpus {
                previous: 2,
                max: 8,
                value: 4
            }
        )));
        assert!(
            patch
                .edits()
                .iter()
                .any(|e| matches!(e,VmSettingEdit::DynamicMemory{min,..} if *min==2<<30))
        );
        assert!(
            !patch
                .edits()
                .iter()
                .any(|e| matches!(e, VmSettingEdit::Cpus { .. } | VmSettingEdit::Memory { .. }))
        );
        editor.startup_cpus = TextEdit::new("1");
        assert!(editor.patch().is_err());
    }
}
