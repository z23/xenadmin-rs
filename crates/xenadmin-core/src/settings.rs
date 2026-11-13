use crate::{Error, HostId, xapi::Session};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use xenadmin_xapi::enumerations::{domain_type, vm_operations, vm_power_state};
use xenadmin_xapi::records::{HostRecord, VmRecord};
use xenadmin_xapi::{Reads, kind};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryLimits {
    pub static_min: u64,
    pub dynamic_min: u64,
    pub dynamic_max: u64,
    pub static_max: u64,
}
impl MemoryLimits {
    fn valid(&self) -> bool {
        self.static_min <= self.dynamic_min
            && self.dynamic_min <= self.dynamic_max
            && self.dynamic_max <= self.static_max
            && self.dynamic_min > 0
            && self.static_max <= i64::MAX as u64
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "field")]
pub enum VmSettingEdit {
    SecureBoot {
        previous: bool,
        value: bool,
    },
    Firmware {
        previous: String,
        value: String,
    },
    HomeHost {
        previous: Option<HostId>,
        value: Option<HostId>,
    },
    LiveCpus {
        previous: u64,
        max: u64,
        value: u64,
    },
    DynamicMemory {
        previous: MemoryLimits,
        min: u64,
        max: u64,
    },
    CpuTopology {
        previous: Option<String>,
        cores_per_socket: u64,
    },
    CpuWeight {
        previous: Option<String>,
        weight: u16,
    },
    ShadowMultiplier {
        previous: String,
        value: String,
        live: bool,
    },
    Name {
        previous: String,
        value: String,
    },
    Description {
        previous: String,
        value: String,
    },
    Cpus {
        previous_max: u64,
        previous_startup: u64,
        max: u64,
        startup: u64,
    },
    Memory {
        previous: MemoryLimits,
        value: MemoryLimits,
    },
    BootOrder {
        previous: String,
        value: String,
    },
    Autostart {
        previous: bool,
        value: bool,
    },
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SettingsTarget {
    #[default]
    Vm,
    CustomTemplate,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VmSettingsPatch {
    #[serde(default)]
    target: SettingsTarget,
    edits: Vec<VmSettingEdit>,
}
impl VmSettingsPatch {
    pub fn new(edits: Vec<VmSettingEdit>) -> Result<Self, Error> {
        let patch = Self {
            edits,
            target: SettingsTarget::Vm,
        };
        patch.validate()?;
        Ok(patch)
    }
    pub fn custom_template(edits: Vec<VmSettingEdit>) -> Result<Self, Error> {
        let patch = Self {
            edits,
            target: SettingsTarget::CustomTemplate,
        };
        patch.validate()?;
        Ok(patch)
    }
    pub fn is_template(&self) -> bool {
        self.target == SettingsTarget::CustomTemplate
    }
    pub(crate) fn target_guard(&self, r: &Value) -> Result<(), Error> {
        if VmRecord::new(r).is_a_template() != self.is_template()
            || VmRecord::new(r).is_a_snapshot() != false
            || VmRecord::new(r).is_control_domain() != false
        {
            return Err(Error::Stale);
        }
        if VmRecord::new(r)
            .current_operations()
            .entries()
            .is_none_or(|v| !v.is_empty())
        {
            return Err(Error::SettingsConflict);
        }
        if self.is_template()
            && (VmRecord::new(r).power_state() != vm_power_state::HALTED
                || !crate::guest_boot::custom_template(r))
        {
            return Err(Error::UnavailableAction);
        }
        Ok(())
    }
    pub fn edits(&self) -> &[VmSettingEdit] {
        &self.edits
    }
    pub(crate) fn validate(&self) -> Result<(), Error> {
        if self.edits.is_empty() || self.edits.len() > 12 {
            return Err(Error::InvalidSettings);
        }
        let mut seen = std::collections::BTreeSet::new();
        for edit in &self.edits {
            if self.is_template()
                && matches!(
                    edit,
                    VmSettingEdit::LiveCpus { .. }
                        | VmSettingEdit::DynamicMemory { .. }
                        | VmSettingEdit::ShadowMultiplier { live: true, .. }
                )
            {
                return Err(Error::InvalidSettings);
            }
            let (field, valid) = match edit {
                VmSettingEdit::SecureBoot { previous, value } => (10, previous != value),
                VmSettingEdit::Firmware { previous, value } => (
                    11,
                    self.is_template()
                        && previous != value
                        && ["bios", "uefi"].contains(&previous.as_str())
                        && ["bios", "uefi"].contains(&value.as_str()),
                ),
                VmSettingEdit::HomeHost { previous, value } => (9, previous != value),
                VmSettingEdit::LiveCpus {
                    previous,
                    max,
                    value,
                } => (
                    2,
                    *previous > 0 && value > previous && value <= max && *max <= i64::MAX as u64,
                ),
                VmSettingEdit::DynamicMemory { previous, min, max } => (
                    3,
                    previous.valid()
                        && *min >= previous.static_min
                        && min <= max
                        && *max <= previous.static_max
                        && *min > 0,
                ),
                VmSettingEdit::CpuTopology {
                    previous,
                    cores_per_socket,
                } => (
                    6,
                    *cores_per_socket > 0
                        && *cores_per_socket <= i64::MAX as u64
                        && previous.as_ref().is_none_or(|v| v.len() <= 64),
                ),
                VmSettingEdit::CpuWeight { previous, weight } => (
                    7,
                    *weight > 0 && previous.as_ref().is_none_or(|v| v.len() <= 64),
                ),
                VmSettingEdit::ShadowMultiplier {
                    previous, value, ..
                } => (8, shadow(previous).is_some() && shadow(value).is_some()),
                VmSettingEdit::Name { value, .. } => (
                    0,
                    !value.trim().is_empty()
                        && value.chars().count() <= 255
                        && !value.chars().any(char::is_control),
                ),
                VmSettingEdit::Description { value, .. } => {
                    (1, value.len() <= 16384 && !value.contains('\0'))
                }
                VmSettingEdit::Cpus {
                    previous_max,
                    previous_startup,
                    max,
                    startup,
                } => (
                    2,
                    *startup > 0
                        && startup <= max
                        && *max <= i64::MAX as u64
                        && *previous_startup > 0
                        && previous_startup <= previous_max,
                ),
                VmSettingEdit::Memory { previous, value } => (3, previous.valid() && value.valid()),
                VmSettingEdit::BootOrder { value, .. } => (
                    4,
                    value.len() <= 3
                        && value.bytes().all(|b| b"cdn".contains(&b))
                        && value
                            .bytes()
                            .collect::<std::collections::BTreeSet<_>>()
                            .len()
                            == value.len(),
                ),
                VmSettingEdit::Autostart { .. } => (5, true),
            };
            if !valid || !seen.insert(field) {
                return Err(Error::InvalidSettings);
            }
        }
        Ok(())
    }
    pub(crate) fn preflight(&self, record: &Value) -> Result<(), Error> {
        let mut projected = record.clone();
        for step in self.steps() {
            step.preflight(&projected)?;
            if let Some(key) = &step.map_key {
                projected[&step.fields[0]][key] = step.value[0].clone();
            } else {
                for (field, value) in step.fields.iter().zip(&step.value) {
                    projected[field] = value.clone();
                }
            }
        }
        Ok(())
    }
    pub(crate) async fn preflight_remote(
        &self,
        session: &Session,
        reference: &str,
    ) -> Result<(), Error> {
        if self
            .edits
            .iter()
            .any(|e| matches!(e, VmSettingEdit::SecureBoot { value: true, .. }))
        {
            crate::guest_boot::require_policy_support(session, reference).await?;
        }
        Ok(())
    }
    pub(crate) async fn resolve_record(
        &self,
        session: &Session,
        mut record: Value,
    ) -> Result<Value, Error> {
        if self
            .edits
            .iter()
            .any(|edit| matches!(edit, VmSettingEdit::HomeHost { .. }))
        {
            let reference = VmRecord::new(&record)
                .affinity()
                .reference()
                .ok_or(Error::SettingsConflict)?;
            record["affinity"] = if reference == "OpaqueRef:NULL" {
                Value::Null
            } else {
                let host = session
                    .call("host.get_record", vec![json!(reference)])
                    .await?;
                let id = HostId::parse(
                    HostRecord::new(&host)
                        .uuid()
                        .text()
                        .ok_or(Error::SettingsConflict)?,
                )
                .map_err(|_| Error::SettingsConflict)?;
                json!(id)
            };
        }
        Ok(record)
    }
    pub fn permission_requirements(&self) -> crate::PermissionRequirements {
        crate::PermissionRequirements::writes(self.steps().iter().map(|step| step.method.as_str()))
    }

    pub(crate) fn steps(&self) -> Vec<SettingsStep> {
        let mut steps = Vec::new();
        let planned_cores = self.edits.iter().find_map(|edit| match edit {
            VmSettingEdit::CpuTopology {
                cores_per_socket, ..
            } => Some(*cores_per_socket),
            _ => None,
        });
        for edit in &self.edits {
            match edit {
                VmSettingEdit::SecureBoot { previous, value } => {
                    let mut step = SettingsStep::scalar(
                        "Secure Boot policy",
                        "platform",
                        "VM.set_platform",
                        json!(previous.to_string()),
                        json!(value.to_string()),
                    );
                    step.map_key = Some("secureboot".into());
                    step.requires_halted = true;
                    step.guard = Some(SettingsGuard::SecureBoot);
                    steps.push(step);
                }
                VmSettingEdit::Firmware { previous, value } => {
                    let mut step = SettingsStep::scalar(
                        "Empty template firmware",
                        "HVM_boot_params",
                        "VM.set_HVM_boot_params",
                        json!(previous),
                        json!(value),
                    );
                    step.map_key = Some("firmware".into());
                    step.requires_halted = true;
                    step.guard = Some(SettingsGuard::EmptyTemplateFirmware);
                    steps.push(step);
                }
                VmSettingEdit::HomeHost { previous, value } => {
                    let mut step = SettingsStep::scalar(
                        "Home host",
                        "affinity",
                        "VM.set_affinity",
                        json!(previous),
                        json!(value),
                    );
                    step.guard = Some(SettingsGuard::OrdinaryState);
                    steps.push(step);
                }
                VmSettingEdit::LiveCpus {
                    previous,
                    max,
                    value,
                } => {
                    let mut step = SettingsStep::number(
                        "Live vCPUs",
                        "VCPUs_at_startup",
                        "VM.set_VCPUs_number_live",
                        *previous,
                        *value,
                    );
                    step.requires_halted = false;
                    step.guard = Some(SettingsGuard::LiveCpus(*max));
                    steps.push(step);
                }
                VmSettingEdit::DynamicMemory { previous, min, max } => steps.push(SettingsStep {
                    label: "Dynamic memory range".into(),
                    fields: vec!["memory_dynamic_min".into(), "memory_dynamic_max".into()],
                    previous: vec![
                        json!(previous.dynamic_min.to_string()),
                        json!(previous.dynamic_max.to_string()),
                    ],
                    value: vec![json!(min.to_string()), json!(max.to_string())],
                    method: "VM.set_memory_dynamic_range".into(),
                    map_key: None,
                    requires_halted: false,
                    guard: Some(SettingsGuard::DynamicMemory(
                        previous.static_min,
                        previous.static_max,
                    )),
                }),
                VmSettingEdit::CpuTopology {
                    previous,
                    cores_per_socket,
                } => steps.push(SettingsStep {
                    label: "CPU topology".into(),
                    fields: vec!["platform".into()],
                    previous: vec![json!(previous)],
                    value: vec![json!(cores_per_socket.to_string())],
                    method: "VM.set_platform".into(),
                    map_key: Some("cores-per-socket".into()),
                    requires_halted: true,
                    guard: Some(SettingsGuard::Topology(*cores_per_socket)),
                }),
                VmSettingEdit::CpuWeight { previous, weight } => steps.push(SettingsStep {
                    label: "CPU scheduling weight".into(),
                    fields: vec!["VCPUs_params".into()],
                    previous: vec![json!(previous)],
                    value: vec![json!(weight.to_string())],
                    method: "VM.set_VCPUs_params".into(),
                    map_key: Some("weight".into()),
                    requires_halted: false,
                    guard: Some(SettingsGuard::OrdinaryState),
                }),
                VmSettingEdit::ShadowMultiplier {
                    previous,
                    value,
                    live,
                } => steps.push(SettingsStep {
                    label: "Shadow memory multiplier".into(),
                    fields: vec!["HVM_shadow_multiplier".into()],
                    previous: vec![
                        shadow(previous)
                            .map(|v| json!(v.to_string()))
                            .unwrap_or(Value::Null),
                    ],
                    value: vec![
                        shadow(value)
                            .map(|v| json!(v.to_string()))
                            .unwrap_or(Value::Null),
                    ],
                    method: if *live {
                        "VM.set_shadow_multiplier_live"
                    } else {
                        "VM.set_HVM_shadow_multiplier"
                    }
                    .into(),
                    map_key: None,
                    requires_halted: !*live,
                    guard: Some(SettingsGuard::Shadow(*live)),
                }),
                VmSettingEdit::Name { previous, value } => steps.push(SettingsStep::scalar(
                    "Name",
                    "name_label",
                    "VM.set_name_label",
                    json!(previous),
                    json!(value),
                )),
                VmSettingEdit::Description { previous, value } => steps.push(SettingsStep::scalar(
                    "Description",
                    "name_description",
                    "VM.set_name_description",
                    json!(previous),
                    json!(value),
                )),
                VmSettingEdit::Cpus {
                    previous_max,
                    previous_startup,
                    max,
                    startup,
                } => {
                    let mut maximum = SettingsStep::number(
                        "Maximum vCPUs",
                        "VCPUs_max",
                        "VM.set_VCPUs_max",
                        *previous_max,
                        *max,
                    );
                    maximum.guard = Some(SettingsGuard::CpuCount(*max, planned_cores));
                    let startup = SettingsStep::number(
                        "Startup vCPUs",
                        "VCPUs_at_startup",
                        "VM.set_VCPUs_at_startup",
                        *previous_startup,
                        *startup,
                    );
                    if max >= previous_max {
                        steps.extend([maximum, startup]);
                    } else {
                        steps.extend([startup, maximum]);
                    }
                }
                VmSettingEdit::Memory { previous, value } => steps.push(SettingsStep {
                    label: "Memory limits".into(),
                    fields: vec![
                        "memory_static_min".into(),
                        "memory_static_max".into(),
                        "memory_dynamic_min".into(),
                        "memory_dynamic_max".into(),
                    ],
                    previous: memory_values(previous),
                    value: memory_values(value),
                    method: "VM.set_memory_limits".into(),
                    map_key: None,
                    requires_halted: true,
                    guard: None,
                }),
                VmSettingEdit::BootOrder { previous, value } => steps.push(SettingsStep {
                    label: "Boot order".into(),
                    fields: vec!["HVM_boot_params".into()],
                    previous: vec![json!(previous)],
                    value: vec![json!(value)],
                    method: "VM.set_HVM_boot_params".into(),
                    map_key: Some("order".into()),
                    requires_halted: true,
                    guard: None,
                }),
                VmSettingEdit::Autostart { previous, value } => steps.push(SettingsStep {
                    label: "Autostart".into(),
                    fields: vec!["other_config".into()],
                    previous: vec![json!(previous.to_string())],
                    value: vec![json!(value.to_string())],
                    method: "VM.set_other_config".into(),
                    map_key: Some("auto_poweron".into()),
                    requires_halted: false,
                    guard: None,
                }),
            }
        }
        steps.retain(|s| s.previous != s.value);
        steps
    }
}
fn shadow(value: &str) -> Option<f64> {
    if value.len() > 64 {
        return None;
    }
    value
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite() && (1.0..=1000.0).contains(v))
}
fn integer<'a>(value: impl Reads<'a, kind::Whole>) -> Option<u64> {
    let value = value.raw();
    value.as_u64().or_else(|| value.as_str()?.parse().ok())
}
#[derive(Clone, Debug)]
enum SettingsGuard {
    SecureBoot,
    EmptyTemplateFirmware,
    LiveCpus(u64),
    DynamicMemory(u64, u64),
    Topology(u64),
    CpuCount(u64, Option<u64>),
    OrdinaryState,
    Shadow(bool),
}
impl SettingsGuard {
    fn preflight(&self, record: &Value) -> Result<(), Error> {
        let halted = VmRecord::new(record).power_state() == vm_power_state::HALTED;
        let running = VmRecord::new(record).power_state() == vm_power_state::RUNNING;
        if !halted && !running {
            return Err(Error::UnavailableAction);
        }
        let advertised = match self {
            Self::LiveCpus(_) => Some(vm_operations::CHANGING_VCPUS_LIVE),
            Self::DynamicMemory(..) => Some(vm_operations::CHANGING_DYNAMIC_RANGE),
            Self::Shadow(true) => Some(vm_operations::CHANGING_SHADOW_MEMORY_LIVE),
            Self::Shadow(false) => Some(vm_operations::CHANGING_SHADOW_MEMORY),
            _ => None,
        };
        if advertised.is_some_and(|operation| !crate::xapi::advertises(record, operation)) {
            return Err(Error::SettingsNotAdvertised);
        }
        let valid = match self {
            Self::SecureBoot => halted && crate::guest_boot::firmware(record) == Some("uefi"),
            Self::EmptyTemplateFirmware => {
                halted
                    && VmRecord::new(record).is_a_template() == true
                    && crate::guest_boot::custom_template(record)
                    && crate::guest_boot::is_hvm(record)
                    && VmRecord::new(record)
                        .vbds()
                        .members()
                        .is_some_and(Vec::is_empty)
                    && crate::guest_boot::policy(record) == Some(false)
            }
            Self::LiveCpus(max) => {
                running
                    && integer(VmRecord::new(record).vcpus_max()) == Some(*max)
                    && crate::xapi::advertises(record, vm_operations::CHANGING_VCPUS_LIVE)
            }
            Self::DynamicMemory(min, max) => {
                integer(VmRecord::new(record).memory_static_min()) == Some(*min)
                    && integer(VmRecord::new(record).memory_static_max()) == Some(*max)
                    && crate::xapi::advertises(record, vm_operations::CHANGING_DYNAMIC_RANGE)
            }
            Self::Topology(cores) => {
                halted
                    && integer(VmRecord::new(record).vcpus_max())
                        .is_some_and(|v| v >= *cores && v % cores == 0)
            }
            Self::CpuCount(max, planned) => {
                let cores = planned.or_else(|| {
                    VmRecord::new(record).platform()["cores-per-socket"]
                        .as_str()
                        .and_then(|c| c.parse::<u64>().ok())
                });
                if cores.is_some_and(|c| c == 0 || max % c != 0) {
                    return Err(Error::SettingsTopologyMismatch);
                }
                halted
            }
            Self::OrdinaryState => true,
            Self::Shadow(live) => {
                (if *live {
                    running
                        && crate::xapi::advertises(
                            record,
                            vm_operations::CHANGING_SHADOW_MEMORY_LIVE,
                        )
                } else {
                    halted && crate::xapi::advertises(record, vm_operations::CHANGING_SHADOW_MEMORY)
                }) && match VmRecord::new(record).domain_type().name() {
                    Some(domain_type::HVM) => true,
                    None | Some(domain_type::UNSPECIFIED) => VmRecord::new(record)
                        .hvm_boot_policy()
                        .text()
                        .is_some_and(|p| !p.is_empty()),
                    _ => false,
                }
            }
        };
        if valid {
            Ok(())
        } else {
            Err(Error::SettingsConflict)
        }
    }
}
fn memory_values(m: &MemoryLimits) -> Vec<Value> {
    [m.static_min, m.static_max, m.dynamic_min, m.dynamic_max]
        .into_iter()
        .map(|v| json!(v.to_string()))
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StepState {
    Pending,
    Dispatching,
    Verified,
    Failed,
    OutcomeUnknown,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationStep {
    pub label: String,
    pub state: StepState,
    pub detail: String,
}
#[derive(Clone, Debug)]
pub(crate) struct SettingsStep {
    pub label: String,
    fields: Vec<String>,
    previous: Vec<Value>,
    value: Vec<Value>,
    method: String,
    map_key: Option<String>,
    requires_halted: bool,
    guard: Option<SettingsGuard>,
}
impl SettingsStep {
    fn scalar(label: &str, field: &str, method: &str, previous: Value, value: Value) -> Self {
        Self {
            label: label.into(),
            fields: vec![field.into()],
            previous: vec![previous],
            value: vec![value],
            method: method.into(),
            map_key: None,
            requires_halted: false,
            guard: None,
        }
    }
    fn number(label: &str, field: &str, method: &str, previous: u64, value: u64) -> Self {
        let mut step = Self::scalar(
            label,
            field,
            method,
            json!(previous.to_string()),
            json!(value.to_string()),
        );
        step.requires_halted = true;
        step
    }
    fn values(&self, record: &Value) -> Vec<Value> {
        self.fields
            .iter()
            .map(|field| {
                let value = &record[field];
                if let Some(key) = &self.map_key {
                    value
                        .get(key)
                        .cloned()
                        .unwrap_or_else(|| match key.as_str() {
                            "auto_poweron" | "secureboot" => json!("false"),
                            "firmware" => json!("bios"),
                            "order" => json!(""),
                            _ => Value::Null,
                        })
                } else if field == "HVM_shadow_multiplier" {
                    value
                        .as_f64()
                        .or_else(|| value.as_str()?.parse().ok())
                        .filter(|v| v.is_finite())
                        .map(|v| json!(v.to_string()))
                        .unwrap_or(Value::Null)
                } else if value.is_number() {
                    json!(value.to_string())
                } else {
                    value.clone()
                }
            })
            .collect()
    }
    pub(crate) fn preflight(&self, record: &Value) -> Result<(), Error> {
        if self.map_key.is_some() && !record[&self.fields[0]].is_object() {
            return Err(Error::SettingsConflict);
        }
        if self.requires_halted && VmRecord::new(record).power_state() != vm_power_state::HALTED {
            return Err(Error::SettingsRequiresHaltedVm);
        }
        if let Some(guard) = &self.guard {
            guard.preflight(record)?;
        }
        if self.values(record) != self.previous {
            return Err(Error::SettingsConflict);
        }
        Ok(())
    }
    pub(crate) fn verified(&self, record: &Value) -> bool {
        (!matches!(
            self.guard,
            Some(SettingsGuard::SecureBoot | SettingsGuard::EmptyTemplateFirmware)
        ) || self
            .guard
            .as_ref()
            .is_none_or(|g| g.preflight(record).is_ok()))
            && self.values(record) == self.value
    }
    pub(crate) async fn prepare_call(
        &self,
        session: &Session,
        reference: &str,
        before: &Value,
    ) -> Result<(String, Vec<Value>), Error> {
        if matches!(self.guard, Some(SettingsGuard::SecureBoot)) && self.value[0] == "true" {
            crate::guest_boot::require_policy_support(session, reference).await?;
        }
        let mut params = vec![json!(reference)];
        if self.method == "VM.set_affinity" {
            let value = if self.value[0].is_null() {
                json!("OpaqueRef:NULL")
            } else {
                let id: HostId = serde_json::from_value(self.value[0].clone())
                    .map_err(|_| Error::InvalidSettings)?;
                let reference = session
                    .call("host.get_by_uuid", vec![json!(id.to_string())])
                    .await?;
                let record = session
                    .call("host.get_record", vec![reference.clone()])
                    .await?;
                if HostRecord::new(&record).uuid() != id.to_string() {
                    return Err(Error::SettingsConflict);
                }
                reference
            };
            params.push(value);
        } else if let Some(key) = &self.map_key {
            let mut map = before[&self.fields[0]]
                .as_object()
                .cloned()
                .ok_or(Error::SettingsConflict)?;
            map.insert(key.clone(), self.value[0].clone());
            params.push(Value::Object(map));
        } else if matches!(self.guard, Some(SettingsGuard::Shadow(_))) {
            params.push(json!(
                self.value[0]
                    .as_str()
                    .and_then(shadow)
                    .ok_or(Error::InvalidSettings)?
            ));
        } else {
            params.extend(self.value.iter().cloned());
        }
        Ok((self.method.clone(), params))
    }
    pub(crate) async fn send(
        &self,
        session: &Session,
        reference: &str,
        before: &Value,
    ) -> Result<(), Error> {
        let (method, params) = self.prepare_call(session, reference, before).await?;
        session.call(&method, params).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shrinking_cpu_max_changes_startup_first_and_growth_changes_max_first() {
        for (old, new, first) in [(8, 4, "VCPUs_at_startup"), (4, 8, "VCPUs_max")] {
            let p = VmSettingsPatch::new(vec![VmSettingEdit::Cpus {
                previous_max: old,
                previous_startup: old,
                max: new,
                startup: new,
            }])
            .unwrap();
            assert_eq!(p.steps()[0].fields, vec![first]);
        }
    }
    #[test]
    fn unadvertised_live_changes_are_named_and_changed_limits_remain_conflicts() {
        let p = VmSettingsPatch::new(vec![VmSettingEdit::LiveCpus {
            previous: 2,
            max: 4,
            value: 3,
        }])
        .unwrap();
        let running = |operations: Vec<&str>, max: &str| json!({"power_state": "Running", "VCPUs_max": max, "VCPUs_at_startup": "2", "allowed_operations": operations, "blocked_operations": {}});
        assert!(matches!(
            p.steps()[0].preflight(&running(vec![], "4")),
            Err(Error::SettingsNotAdvertised)
        ));
        assert!(matches!(
            p.steps()[0].preflight(&running(vec!["changing_VCPUs_live"], "3")),
            Err(Error::SettingsConflict)
        ));
        assert!(
            p.steps()[0]
                .preflight(&running(vec!["changing_VCPUs_live"], "4"))
                .is_ok()
        );
        let shadow = VmSettingsPatch::new(vec![VmSettingEdit::ShadowMultiplier {
            previous: "1.0".into(),
            value: "2.0".into(),
            live: true,
        }])
        .unwrap();
        assert!(matches!(
            shadow.steps()[0].preflight(&json!({"power_state": "Running", "domain_type": "hvm", "HVM_shadow_multiplier": 1.0, "allowed_operations": ["changing_shadow_memory"], "blocked_operations": {}})),
            Err(Error::SettingsNotAdvertised)
        ));
    }
    #[test]
    fn vcpu_maximum_must_stay_a_multiple_of_the_current_or_planned_cores_per_socket() {
        let halted = json!({"power_state":"Halted","VCPUs_max":"4","VCPUs_at_startup":"4","platform":{"cores-per-socket":"4"}});
        let shrink = VmSettingsPatch::new(vec![VmSettingEdit::Cpus {
            previous_max: 4,
            previous_startup: 4,
            max: 2,
            startup: 2,
        }])
        .unwrap();
        assert!(matches!(
            shrink.preflight(&halted),
            Err(Error::SettingsTopologyMismatch)
        ));
        let with_topology = VmSettingsPatch::new(vec![
            VmSettingEdit::Cpus {
                previous_max: 4,
                previous_startup: 4,
                max: 2,
                startup: 2,
            },
            VmSettingEdit::CpuTopology {
                previous: Some("4".into()),
                cores_per_socket: 2,
            },
        ])
        .unwrap();
        assert!(with_topology.preflight(&halted).is_ok());
        let grow = VmSettingsPatch::new(vec![VmSettingEdit::Cpus {
            previous_max: 4,
            previous_startup: 4,
            max: 8,
            startup: 4,
        }])
        .unwrap();
        assert!(grow.preflight(&halted).is_ok());
    }
    #[test]
    fn stale_property_drafts_and_running_resource_edits_are_rejected() {
        let p = VmSettingsPatch::new(vec![VmSettingEdit::Name {
            previous: "Old".into(),
            value: "New".into(),
        }])
        .unwrap();
        assert!(matches!(
            p.steps()[0].preflight(&json!({"name_label":"Someone else's edit"})),
            Err(Error::SettingsConflict)
        ));
        let p = VmSettingsPatch::new(vec![VmSettingEdit::Cpus {
            previous_max: 2,
            previous_startup: 2,
            max: 4,
            startup: 4,
        }])
        .unwrap();
        assert!(matches!(
            p.steps()[0].preflight(&json!({"power_state":"Running","VCPUs_max":"2"})),
            Err(Error::SettingsRequiresHaltedVm)
        ));
    }
    #[test]
    fn invalid_and_duplicate_edits_are_rejected() {
        assert!(VmSettingsPatch::new(vec![]).is_err());
        assert!(
            VmSettingsPatch::new(vec![VmSettingEdit::Cpus {
                previous_max: 2,
                previous_startup: 2,
                max: 2,
                startup: 3
            }])
            .is_err()
        );
        let e = VmSettingEdit::Name {
            previous: "Old".into(),
            value: "New".into(),
        };
        assert!(VmSettingsPatch::new(vec![e.clone(), e]).is_err());
    }
}

#[cfg(test)]
mod advanced_tests {
    use super::*;
    #[test]
    fn initial_review_checks_topology_against_the_planned_cpu_count_then_runtime_checks_fresh_count()
     {
        let patch = VmSettingsPatch::new(vec![
            VmSettingEdit::Cpus {
                previous_max: 2,
                previous_startup: 2,
                max: 8,
                startup: 4,
            },
            VmSettingEdit::CpuTopology {
                previous: None,
                cores_per_socket: 4,
            },
        ])
        .unwrap();
        let before = json!({"power_state":"Halted","VCPUs_max":"2","VCPUs_at_startup":"2","platform":{"keep":"yes"}});
        assert!(patch.preflight(&before).is_ok());
        assert!(patch.steps()[2].preflight(&before).is_err());
        let mut after = before;
        after["VCPUs_max"] = json!("8");
        after["VCPUs_at_startup"] = json!("4");
        assert!(patch.steps()[2].preflight(&after).is_ok());
    }
}

#[cfg(test)]
mod firmware_tests {
    use super::*;
    fn template() -> Value {
        json!({"is_a_template":true,"is_default_template":false,"is_control_domain":false,"is_a_snapshot":false,"current_operations":{},"power_state":"Halted","VBDs":[],"HVM_boot_policy":"BIOS order","HVM_boot_params":{"firmware":"bios","order":"dc"},"platform":{"secureboot":"false","keep":"yes"}})
    }
    #[test]
    fn legacy_settings_journal_remains_ordinary_vm_only() {
        let patch: VmSettingsPatch =
            serde_json::from_value(json!({"edits":[{"field":"Name","previous":"a","value":"b"}]}))
                .unwrap();
        assert!(!patch.is_template());
        assert!(patch.target_guard(&template()).is_err());
        let mut vm = template();
        vm["is_a_template"] = json!(false);
        assert!(patch.target_guard(&vm).is_ok());
        let custom = VmSettingsPatch::custom_template(patch.edits().to_vec()).unwrap();
        assert!(custom.target_guard(&vm).is_err());
        assert!(custom.target_guard(&template()).is_ok());
        vm = template();
        vm["other_config"] = json!({"default_template":"true"});
        assert!(custom.target_guard(&vm).is_err());
        vm = template();
        vm["is_default_template"] = Value::Null;
        assert!(custom.target_guard(&vm).is_err());
    }
    #[test]
    fn empty_template_firmware_then_secure_policy_are_guarded_and_projected() {
        let patch = VmSettingsPatch::custom_template(vec![
            VmSettingEdit::Firmware {
                previous: "bios".into(),
                value: "uefi".into(),
            },
            VmSettingEdit::SecureBoot {
                previous: false,
                value: true,
            },
        ])
        .unwrap();
        let before = template();
        assert!(patch.preflight(&before).is_ok());
        let mut attached = before.clone();
        attached["VBDs"] = json!(["OpaqueRef:disk"]);
        assert!(patch.preflight(&attached).is_err());
        let mut ordinary = before.clone();
        ordinary["is_a_template"] = json!(false);
        assert!(patch.preflight(&ordinary).is_err());
        let mut after = before;
        after["HVM_boot_params"]["firmware"] = json!("uefi");
        after["platform"]["secureboot"] = json!("true");
        assert!(patch.steps()[1].verified(&after));
        after["HVM_boot_params"]["firmware"] = json!("bios");
        assert!(!patch.steps()[1].verified(&after));
    }
    #[test]
    fn live_only_edits_and_ordinary_firmware_edits_cannot_target_templates() {
        assert!(
            VmSettingsPatch::custom_template(vec![VmSettingEdit::LiveCpus {
                previous: 1,
                max: 4,
                value: 2
            }])
            .is_err()
        );
        assert!(
            VmSettingsPatch::new(vec![VmSettingEdit::Firmware {
                previous: "bios".into(),
                value: "uefi".into()
            }])
            .is_err()
        );
    }
}
