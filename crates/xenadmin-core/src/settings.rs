use crate::{Error, xapi::Session};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VmSettingsPatch {
    edits: Vec<VmSettingEdit>,
}
impl VmSettingsPatch {
    pub fn new(edits: Vec<VmSettingEdit>) -> Result<Self, Error> {
        let patch = Self { edits };
        patch.validate()?;
        Ok(patch)
    }
    pub fn edits(&self) -> &[VmSettingEdit] {
        &self.edits
    }
    pub(crate) fn validate(&self) -> Result<(), Error> {
        if self.edits.is_empty() || self.edits.len() > 6 {
            return Err(Error::InvalidSettings);
        }
        let mut seen = std::collections::BTreeSet::new();
        for edit in &self.edits {
            let (field, valid) = match edit {
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
    pub(crate) fn steps(&self) -> Vec<SettingsStep> {
        let mut steps = Vec::new();
        for edit in &self.edits {
            match edit {
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
                    let maximum = SettingsStep::number(
                        "Maximum vCPUs",
                        "VCPUs_max",
                        "VM.set_VCPUs_max",
                        *previous_max,
                        *max,
                    );
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
                }),
                VmSettingEdit::BootOrder { previous, value } => steps.push(SettingsStep {
                    label: "Boot order".into(),
                    fields: vec!["HVM_boot_params".into()],
                    previous: vec![json!(previous)],
                    value: vec![json!(value)],
                    method: "VM.set_HVM_boot_params".into(),
                    map_key: Some("order".into()),
                    requires_halted: true,
                }),
                VmSettingEdit::Autostart { previous, value } => steps.push(SettingsStep {
                    label: "Autostart".into(),
                    fields: vec!["other_config".into()],
                    previous: vec![json!(previous.to_string())],
                    value: vec![json!(value.to_string())],
                    method: "VM.set_other_config".into(),
                    map_key: Some("auto_poweron".into()),
                    requires_halted: false,
                }),
            }
        }
        steps.retain(|s| s.previous != s.value);
        steps
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
                        .unwrap_or_else(|| json!(if key == "auto_poweron" { "false" } else { "" }))
                } else if value.is_number() {
                    json!(value.to_string())
                } else {
                    value.clone()
                }
            })
            .collect()
    }
    pub(crate) fn preflight(&self, record: &Value) -> Result<(), Error> {
        if self.requires_halted && record["power_state"] != "Halted" {
            return Err(Error::SettingsRequiresHaltedVm);
        }
        if self.values(record) != self.previous {
            return Err(Error::SettingsConflict);
        }
        Ok(())
    }
    pub(crate) fn verified(&self, record: &Value) -> bool {
        self.values(record) == self.value
    }
    pub(crate) async fn send(
        &self,
        session: &Session,
        reference: &str,
        before: &Value,
    ) -> Result<(), Error> {
        let mut params = vec![json!(reference)];
        if let Some(key) = &self.map_key {
            let mut map = before[&self.fields[0]]
                .as_object()
                .cloned()
                .ok_or(Error::SettingsConflict)?;
            map.insert(key.clone(), self.value[0].clone());
            params.push(Value::Object(map));
        } else {
            params.extend(self.value.iter().cloned());
        }
        session.call(&self.method, params).await?;
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
