//! The draft of the settings of one VM or template, carried over from the
//! Rune module `vm_properties.rs` by a mechanical rewrite (`TextEdit` became
//! `String`, the items became `pub`): the fields, which of them is an edit
//! and in which order ([`Editor::patch`]), the texts of the parse errors, the
//! review line of each edit ([`describe`]) and the memory notation
//! ([`parse_memory`], [`memory_text`]). Nothing here knows the model or the
//! pages: `change.rs` captures an [`Editor`] and submits its patch, `view.rs`
//! describes it.
//!
//! Three fields are new: the pool of the target (an editor follows the pool
//! of its target, not the scope session), whether the boot readiness answer
//! has been seen by `poll`, and the page Back returns to from the review.

use xenadmin_core::{
    MemoryLimits, ObservedVm, PowerState, Vm, VmDetails, VmId, VmSettingEdit, VmSettingsPatch,
};

#[derive(Clone, Copy)]
pub enum Field {
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
pub struct Editor {
    /// The pool of the target.
    pub pool: xenadmin_core::PoolId,
    /// Whether `poll` has seen the answer of `boot_query`.
    pub boot_seen: bool,
    /// The id of the page Back returns to from the review.
    pub back: &'static str,
    pub target: Option<ObservedVm>,
    pub id: VmId,
    pub original: Vm,
    pub details: VmDetails,
    pub name: String,
    pub description: String,
    pub max_cpus: String,
    pub startup_cpus: String,
    pub static_min: String,
    pub dynamic_min: String,
    pub dynamic_max: String,
    pub static_max: String,
    pub boot_order: String,
    pub topology: String,
    pub cpu_weight: String,
    pub shadow: String,
    pub autostart: bool,
    pub secure_boot: bool,
    pub firmware: Option<String>,
    pub empty_template: bool,
    pub boot_query: Option<xenadmin_core::GuestBootStatusQuery>,
    pub boot_error: Option<String>,
    pub home_host: Option<xenadmin_core::HostId>,
    pub hosts: Vec<(xenadmin_core::HostId, String)>,
    pub review: Option<VmSettingsPatch>,
    pub error: Option<String>,
}
impl Editor {
    pub fn field(&self, field: Field) -> &String {
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
    pub fn field_mut(&mut self, field: Field) -> &mut String {
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
    pub fn memory(&self) -> Option<MemoryLimits> {
        Some(MemoryLimits {
            static_min: self.details.memory_static_min?,
            dynamic_min: self.details.memory_dynamic_min?,
            dynamic_max: self.details.memory_dynamic_max?,
            static_max: self.details.memory_static_max?,
        })
    }
    pub fn patch(&self) -> Result<VmSettingsPatch, String> {
        let mut edits = Vec::new();
        if self.details.affinity_known && self.home_host != self.details.affinity {
            edits.push(VmSettingEdit::HomeHost {
                previous: self.details.affinity,
                value: self.home_host,
            });
        }
        if self.name != self.original.name {
            edits.push(VmSettingEdit::Name {
                previous: self.original.name.clone(),
                value: self.name.clone(),
            });
        }
        if self.description != self.original.description {
            edits.push(VmSettingEdit::Description {
                previous: self.original.description.clone(),
                value: self.description.clone(),
            });
        }
        if self.original.power_state == PowerState::Halted {
            if let (Some(previous_max), Some(previous_startup)) =
                (self.details.vcpus_max, self.details.vcpus_at_startup)
            {
                let max = self
                    .max_cpus
                    .trim()
                    .parse::<u64>()
                    .map_err(|_| "Enter whole positive CPU counts")?;
                let startup = self
                    .startup_cpus
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
                    static_min: parse_memory(&self.static_min)?,
                    dynamic_min: parse_memory(&self.dynamic_min)?,
                    dynamic_max: parse_memory(&self.dynamic_max)?,
                    static_max: parse_memory(&self.static_max)?,
                };
                if value != previous {
                    edits.push(VmSettingEdit::Memory { previous, value });
                }
            }
            if let Some(previous) = &self.details.boot_order
                && &self.boot_order != previous
            {
                edits.push(VmSettingEdit::BootOrder {
                    previous: previous.clone(),
                    value: self.boot_order.clone(),
                });
            }
        }
        if self.original.power_state == PowerState::Running {
            if let (Some(previous), Some(max)) =
                (self.details.vcpus_at_startup, self.details.vcpus_max)
            {
                let value = self
                    .startup_cpus
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
                let min = parse_memory(&self.dynamic_min)?;
                let max = parse_memory(&self.dynamic_max)?;
                if min != previous.dynamic_min || max != previous.dynamic_max {
                    edits.push(VmSettingEdit::DynamicMemory { previous, min, max });
                }
            }
        }
        if self.original.power_state == PowerState::Halted
            && self.details.topology_known
            && self.topology != self.details.cores_per_socket.clone().unwrap_or_default()
        {
            edits.push(VmSettingEdit::CpuTopology {
                previous: self.details.cores_per_socket.clone(),
                cores_per_socket: self
                    .topology
                    .parse::<u64>()
                    .map_err(|_| "Enter a positive cores-per-socket count")?,
            });
        }
        if self.details.cpu_weight_known
            && self.cpu_weight
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
                    .parse::<u16>()
                    .map_err(|_| "CPU weight must be 1 to 65535")?,
            });
        }
        if let Some(previous) = &self.details.shadow_multiplier
            && self.shadow != *previous
        {
            edits.push(VmSettingEdit::ShadowMultiplier {
                previous: previous.clone(),
                value: self.shadow.clone(),
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
        if let (Some(previous), Some(value)) = (&self.details.firmware, &self.firmware)
            && previous != value
        {
            edits.push(VmSettingEdit::Firmware {
                previous: previous.clone(),
                value: value.clone(),
            });
        }
        if let Some(previous) = self.details.secure_boot
            && previous != self.secure_boot
        {
            edits.push(VmSettingEdit::SecureBoot {
                previous,
                value: self.secure_boot,
            });
        }
        if edits.is_empty() {
            return Err("No properties have changed".into());
        }
        if self.original.is_template {
            VmSettingsPatch::custom_template(edits)
        } else {
            VmSettingsPatch::new(edits)
        }
        .map_err(|e| e.to_string())
    }
}
pub fn describe(edit: &VmSettingEdit, hosts: &[(xenadmin_core::HostId, String)]) -> String {
    match edit {
        VmSettingEdit::Firmware { previous, value } => format!(
            "Empty custom template firmware: {previous} → {value}. No disks may be attached. Installed guest images cannot change firmware mode."
        ),
        VmSettingEdit::SecureBoot { previous, value } => format!(
            "Secure Boot policy: {previous} → {value}. Certificate state is preserved; this does not establish actual guest enforcement."
        ),
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
        } => {
            format!("vCPUs: maximum {previous_max} → {max}; startup {previous_startup} → {startup}")
        }
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
        VmSettingEdit::HomeHost { previous, value } => {
            let name = |id: &Option<xenadmin_core::HostId>| {
                id.map_or_else(
                    || "No preference".into(),
                    |id| {
                        format!(
                            "{} ({id})",
                            hosts
                                .iter()
                                .find(|(host, _)| *host == id)
                                .map_or("Unknown host", |(_, name)| name.as_str())
                        )
                    },
                )
            };
            format!(
                "Home host: {} → {}. This is a preference for future starts. It does not migrate the VM or guarantee placement.",
                name(previous),
                name(value)
            )
        }
    }
}
pub fn memory_text(value: Option<u64>) -> String {
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
pub fn parse_memory(text: &str) -> Result<u64, String> {
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
