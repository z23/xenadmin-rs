use super::*;
use xenadmin_core::{ClassAvailability, MemoryLimits, VmDetails, VmSettingEdit, VmSettingsPatch};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Property {
    Name,
    Description,
    HomeHost,
    Cpus,
    Memory,
    LiveCpus,
    DynamicMemory,
    Topology,
    Weight,
    Shadow,
    BootOrder,
    Autostart,
    Firmware,
    SecureBoot,
}
impl Property {
    pub(super) const ALL: [Self; 14] = [
        Self::Name,
        Self::Description,
        Self::HomeHost,
        Self::Cpus,
        Self::Memory,
        Self::LiveCpus,
        Self::DynamicMemory,
        Self::Topology,
        Self::Weight,
        Self::Shadow,
        Self::BootOrder,
        Self::Autostart,
        Self::Firmware,
        Self::SecureBoot,
    ];
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Description => "Description",
            Self::HomeHost => "Home host",
            Self::Cpus => "Halted vCPUs",
            Self::Memory => "Halted memory limits",
            Self::LiveCpus => "Live vCPUs",
            Self::DynamicMemory => "Live dynamic memory",
            Self::Topology => "CPU topology",
            Self::Weight => "CPU scheduling weight",
            Self::Shadow => "Shadow memory multiplier",
            Self::BootOrder => "Boot order",
            Self::Autostart => "Autostart",
            Self::Firmware => "Empty template firmware",
            Self::SecureBoot => "Secure Boot policy",
        }
    }
    pub(super) fn labels(self) -> &'static [&'static str] {
        match self {
            Self::Name => &["Common name"],
            Self::Description => &["Common description"],
            Self::Cpus => &["Maximum vCPUs", "Startup vCPUs"],
            Self::Memory => &[
                "Static minimum",
                "Dynamic minimum",
                "Dynamic maximum",
                "Static maximum",
            ],
            Self::LiveCpus => &["Live vCPUs"],
            Self::DynamicMemory => &["Dynamic minimum", "Dynamic maximum"],
            Self::Topology => &["Cores per socket"],
            Self::Weight => &["Weight (1–65535)"],
            Self::Shadow => &["Multiplier (1–1000)"],
            Self::BootOrder => &["Boot order (c=disk, d=DVD, n=network)"],
            _ => &[],
        }
    }
    pub(super) fn note(self) -> &'static str {
        match self {
            Self::Name => {
                "Every changed target receives this exact name. UUIDs remain distinct; no automatic numbering is added."
            }
            Self::HomeHost => {
                "Sets a preference for future starts. This does not migrate guests or guarantee placement."
            }
            Self::Cpus | Self::Topology => {
                "Every target must be halted. Cores per socket must divide each target's maximum vCPU count."
            }
            Self::Memory => {
                "Every target must be halted. Use whole bytes, KiB, MiB or GiB. Static minimum ≤ dynamic minimum ≤ dynamic maximum ≤ static maximum."
            }
            Self::LiveCpus => {
                "Every target must be running. Increase only, within each VM's existing maximum and advertised live capability."
            }
            Self::DynamicMemory => {
                "Every target must be running with the live capability. Values must fit each VM's static limits. Ballooning may reduce memory available to guest applications."
            }
            Self::Shadow => {
                "Only HVM guests in halted or running state with the relevant advertised capability. Running targets use the live setter."
            }
            Self::Firmware => {
                "Only halted empty custom HVM templates with Secure Boot disabled. Installed guest images cannot switch firmware mode."
            }
            Self::SecureBoot => {
                "Every target must be halted and use UEFI. This changes policy, preserves certificates and does not prove guest enforcement. The backend checks certificate support before enabling."
            }
            Self::Autostart => {
                "Sets each guest's autostart preference. The pool's autostart policy remains separate."
            }
            _ => {
                "One common property is reviewed across all captured targets. Unknown or unsupported targets block the entire batch."
            }
        }
    }
}
fn known<T>(value: Option<T>, field: &str) -> Result<T, String> {
    value.ok_or_else(|| format!("{field} is unknown; refresh this target first"))
}
fn memory(d: &VmDetails) -> Result<MemoryLimits, String> {
    Ok(MemoryLimits {
        static_min: known(d.memory_static_min, "Static memory minimum")?,
        dynamic_min: known(d.memory_dynamic_min, "Dynamic memory minimum")?,
        dynamic_max: known(d.memory_dynamic_max, "Dynamic memory maximum")?,
        static_max: known(d.memory_static_max, "Static memory maximum")?,
    })
}
fn advertised(d: &VmDetails, name: &str) -> Result<(), String> {
    if let Some(reason) = d.blocked_operations.get(name) {
        return Err(format!("Host blocks this property: {reason}"));
    }
    if !d
        .allowed_operations
        .as_ref()
        .is_some_and(|ops| ops.iter().any(|op| op == name))
    {
        return Err("The host does not advertise the required capability".into());
    }
    Ok(())
}
fn hvm(d: &VmDetails) -> bool {
    match d.domain_type.as_deref() {
        Some("hvm") => true,
        None | Some("unspecified") => d.boot_policy.as_ref().is_some_and(|p| !p.is_empty()),
        _ => false,
    }
}
pub(super) fn proposal(
    i: &Inventory,
    vm: &Vm,
    e: &Editor,
) -> Result<(Option<VmSettingsPatch>, String), String> {
    let d = known(i.details.vms.get(&vm.id), "VM properties")?;
    if vm.is_control_domain || matches!(vm.power_state, PowerState::Unknown(_)) {
        return Err("This target is not an ordinary VM with a known power state".into());
    }
    if vm.is_template
        && (d.is_default_template != Some(false) || vm.power_state != PowerState::Halted)
    {
        return Err("Only halted custom templates can be edited; built-in or unknown template kinds are read-only".into());
    }
    if !d.current_operations.is_empty() {
        return Err("This target has an operation in progress".into());
    }
    let halted = vm.power_state == PowerState::Halted;
    let running = vm.power_state == PowerState::Running;
    use Property::*;
    if matches!(
        e.property,
        Cpus | Memory | Topology | BootOrder | Firmware | SecureBoot
    ) && !halted
    {
        return Err("This property requires a halted target".into());
    }
    if matches!(e.property, LiveCpus | DynamicMemory) && (!running || vm.is_template) {
        return Err("This property requires a running VM".into());
    }
    if matches!(e.property, HomeHost | Weight | Shadow) && !halted && !running {
        return Err("This property requires a halted or running target".into());
    }
    let value = |n: usize| e.values[n].text.clone();
    let number = |n: usize| {
        value(n)
            .trim()
            .parse::<u64>()
            .map_err(|_| "Enter a positive whole number".to_string())
    };
    let bytes = |n: usize| vm_properties::parse_memory(&value(n));
    let (edit, unchanged) = match e.property {
        Name => (
            VmSettingEdit::Name {
                previous: vm.name.clone(),
                value: value(0),
            },
            vm.name == value(0),
        ),
        Description => (
            VmSettingEdit::Description {
                previous: vm.description.clone(),
                value: value(0),
            },
            vm.description == value(0),
        ),
        HomeHost => {
            if !d.affinity_known {
                return Err("Home host preference is unknown".into());
            }
            if e.host.is_some_and(|id| !i.hosts.contains_key(&id)) {
                return Err("Chosen home host no longer belongs to this pool".into());
            }
            (
                VmSettingEdit::HomeHost {
                    previous: d.affinity,
                    value: e.host,
                },
                d.affinity == e.host,
            )
        }
        Cpus => {
            let previous_max = known(d.vcpus_max, "Maximum vCPUs")?;
            let previous_startup = known(d.vcpus_at_startup, "Startup vCPUs")?;
            let max = number(0)?;
            let startup = number(1)?;
            (
                VmSettingEdit::Cpus {
                    previous_max,
                    previous_startup,
                    max,
                    startup,
                },
                (max, startup) == (previous_max, previous_startup),
            )
        }
        Memory => {
            let previous = memory(d)?;
            let value = MemoryLimits {
                static_min: bytes(0)?,
                dynamic_min: bytes(1)?,
                dynamic_max: bytes(2)?,
                static_max: bytes(3)?,
            };
            let same = previous == value;
            (VmSettingEdit::Memory { previous, value }, same)
        }
        LiveCpus => {
            let previous = known(d.vcpus_at_startup, "Current vCPUs")?;
            let max = known(d.vcpus_max, "Maximum vCPUs")?;
            let value = number(0)?;
            advertised(d, "changing_VCPUs_live")?;
            (
                VmSettingEdit::LiveCpus {
                    previous,
                    max,
                    value,
                },
                previous == value,
            )
        }
        DynamicMemory => {
            let previous = memory(d)?;
            let min = bytes(0)?;
            let max = bytes(1)?;
            advertised(d, "changing_dynamic_range")?;
            let same = previous.dynamic_min == min && previous.dynamic_max == max;
            (VmSettingEdit::DynamicMemory { previous, min, max }, same)
        }
        Topology => {
            if !d.topology_known {
                return Err("CPU topology is unknown".into());
            }
            let cores_per_socket = number(0)?;
            let max = known(d.vcpus_max, "Maximum vCPUs")?;
            if cores_per_socket == 0
                || max < cores_per_socket
                || !max.is_multiple_of(cores_per_socket)
            {
                return Err("Cores per socket must divide this target's maximum vCPUs".into());
            }
            (
                VmSettingEdit::CpuTopology {
                    previous: d.cores_per_socket.clone(),
                    cores_per_socket,
                },
                d.cores_per_socket
                    .as_ref()
                    .and_then(|v| v.parse::<u64>().ok())
                    == Some(cores_per_socket),
            )
        }
        Weight => {
            if !d.cpu_weight_known {
                return Err("CPU scheduling weight is unknown".into());
            }
            let weight = u16::try_from(number(0)?).map_err(|_| "Weight must be 1–65535")?;
            let old = d
                .cpu_weight
                .as_deref()
                .unwrap_or("256")
                .parse::<u16>()
                .map_err(|_| "Current CPU weight is invalid")?;
            (
                VmSettingEdit::CpuWeight {
                    previous: d.cpu_weight.clone(),
                    weight,
                },
                old == weight,
            )
        }
        Shadow => {
            if !hvm(d) {
                return Err("Shadow memory is available only for HVM targets".into());
            }
            advertised(
                d,
                if running {
                    "changing_shadow_memory_live"
                } else {
                    "changing_shadow_memory"
                },
            )?;
            let previous = known(d.shadow_multiplier.clone(), "Shadow multiplier")?;
            let desired = value(0)
                .parse::<f64>()
                .ok()
                .filter(|n| n.is_finite() && (1.0..=1000.0).contains(n))
                .ok_or("Multiplier must be a finite number from 1 to 1000")?;
            let same = previous.parse::<f64>().ok() == Some(desired);
            (
                VmSettingEdit::ShadowMultiplier {
                    previous,
                    value: desired.to_string(),
                    live: running,
                },
                same,
            )
        }
        BootOrder => {
            let previous = known(d.boot_order.clone(), "Boot order")?;
            (
                VmSettingEdit::BootOrder {
                    previous: previous.clone(),
                    value: value(0),
                },
                previous == value(0),
            )
        }
        Autostart => {
            let previous = known(d.auto_power_on, "Autostart policy")?;
            (
                VmSettingEdit::Autostart {
                    previous,
                    value: e.toggle,
                },
                previous == e.toggle,
            )
        }
        Firmware => {
            if !vm.is_template
                || !hvm(d)
                || d.secure_boot != Some(false)
                || i.details.classes.get("vbd") != Some(&ClassAvailability::Available)
                || i.details.vbds.values().any(|v| v.vm == Some(vm.id))
            {
                return Err("Firmware needs an empty custom HVM template with Secure Boot disabled and complete disk inventory".into());
            }
            let previous = known(d.firmware.clone(), "Firmware")?;
            let value = if e.toggle { "uefi" } else { "bios" }.to_string();
            let same = previous == value;
            (VmSettingEdit::Firmware { previous, value }, same)
        }
        SecureBoot => {
            if d.firmware.as_deref() != Some("uefi") {
                return Err("Secure Boot policy requires UEFI firmware".into());
            }
            let previous = known(d.secure_boot, "Secure Boot policy")?;
            (
                VmSettingEdit::SecureBoot {
                    previous,
                    value: e.toggle,
                },
                previous == e.toggle,
            )
        }
    };
    let summary = vm_properties::describe(&edit, &e.hosts);
    if unchanged {
        return Ok((None, summary));
    }
    let patch = if vm.is_template {
        VmSettingsPatch::custom_template(vec![edit])
    } else {
        VmSettingsPatch::new(vec![edit])
    }
    .map_err(|error| error.to_string())?;
    Ok((Some(patch), summary))
}
