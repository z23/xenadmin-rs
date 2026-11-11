use super::*;
use crate::{DeviceProperty as Change, DeviceQos, DiskAccess};
use xenadmin_xapi::records::{VbdRecord, VdiRecord, VmRecord};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Field {
    Name,
    Description,
    Access,
    Position,
    Bootable,
    QosParameters,
    QosAlgorithm,
    Cbt,
}
impl Field {
    fn wire(self) -> &'static str {
        match self {
            Self::Name => "name_label",
            Self::Description => "name_description",
            Self::Access => "mode",
            Self::Position => "userdevice",
            Self::Bootable => "bootable",
            Self::QosParameters => "qos_algorithm_params",
            Self::QosAlgorithm => "qos_algorithm_type",
            Self::Cbt => "cbt_enabled",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PropertyStep {
    pub change: Change,
    field: Field,
    previous: Value,
    value: Value,
    guard: Option<Value>,
}
impl PropertyStep {
    fn method(&self) -> &'static str {
        match (self.field, matches!(self.change, Change::NicRate { .. })) {
            (Field::Name, _) => "VDI.set_name_label",
            (Field::Description, _) => "VDI.set_name_description",
            (Field::Access, _) => "VBD.set_mode",
            (Field::Position, _) => "VBD.set_userdevice",
            (Field::Bootable, _) => "VBD.set_bootable",
            (Field::QosParameters, true) => "VIF.set_qos_algorithm_params",
            (Field::QosAlgorithm, true) => "VIF.set_qos_algorithm_type",
            (Field::QosParameters, false) => "VBD.set_qos_algorithm_params",
            (Field::QosAlgorithm, false) => "VBD.set_qos_algorithm_type",
            (Field::Cbt, _) => "VDI.disable_cbt",
        }
    }
}
pub(super) fn availability(change: &Change, inventory: &Inventory, vm: VmId) -> ActionAvailability {
    let Some(guest) = inventory.vms.get(&vm) else {
        return ActionAvailability::Stale;
    };
    if change.requires_halted() && guest.power_state != PowerState::Halted {
        return ActionAvailability::Prerequisite {
            reason: "Shut down the VM before editing device configuration".into(),
        };
    }
    let valid = match change {
        Change::NicRate { vif, .. } => inventory
            .details
            .vifs
            .get(vif)
            .is_some_and(|v| v.vm == Some(vm) && v.currently_attached == Some(false)),
        Change::DiskMetadata { vbd, vdi, .. }
        | Change::DiskAccess { vbd, vdi, .. }
        | Change::DiskPosition { vbd, vdi, .. }
        | Change::Bootable { vbd, vdi, .. }
        | Change::DiskPriority { vbd, vdi, .. }
        | Change::DisableCbt { vbd, vdi } => inventory.details.vbds.get(vbd).is_some_and(|v| {
            v.vm == Some(vm)
                && v.vdi == Some(*vdi)
                && v.kind == "Disk"
                && (!change.requires_halted() || v.currently_attached == Some(false))
        }),
    };
    if valid {
        ActionAvailability::Available
    } else {
        ActionAvailability::Unavailable
    }
}
struct Current {
    reference: String,
    record: Value,
    attachment: Option<(String, Value)>,
}
async fn current(
    session: &Session,
    vm: VmId,
    change: &Change,
    verify: bool,
) -> Result<Current, Error> {
    let vm_ref = guest(session, vm, !verify && change.requires_halted()).await?;
    if !verify {
        let (_, v) = fresh(session, "VM", vm.uuid()).await?;
        idle(&v)?;
        if !matches!(
            VmRecord::new(&v).power_state().as_str(),
            Some("Halted" | "Running")
        ) {
            return Err(Error::UnavailableAction);
        }
    }
    if let Change::NicRate { vif, .. } = change {
        let (reference, record) = fresh(session, "VIF", vif.uuid()).await?;
        if record["VM"] != vm_ref || (!verify && record["currently_attached"] != false) {
            return Err(Error::DeviceConflict);
        }
        idle(&record)?;
        return Ok(Current {
            reference,
            record,
            attachment: None,
        });
    }
    let (vbd, vdi) = match change {
        Change::DiskMetadata { vbd, vdi, .. }
        | Change::DiskAccess { vbd, vdi, .. }
        | Change::DiskPosition { vbd, vdi, .. }
        | Change::Bootable { vbd, vdi, .. }
        | Change::DiskPriority { vbd, vdi, .. }
        | Change::DisableCbt { vbd, vdi } => (*vbd, *vdi),
        _ => unreachable!(),
    };
    let (vr, v) = fresh(session, "VBD", vbd.uuid()).await?;
    let (reference, record) = fresh(session, "VDI", vdi.uuid()).await?;
    if VbdRecord::new(&v).vm() != vm_ref
        || VbdRecord::new(&v).vdi() != reference
        || VbdRecord::new(&v).type_() != "Disk"
        || (!verify && change.requires_halted() && VbdRecord::new(&v).currently_attached() != false)
        || !ordinary_disk(&record)
        || refs(&record, "VBDs")? != vec![vr.clone()]
    {
        return Err(Error::DeviceConflict);
    }
    idle(&v)?;
    idle(&record)?;
    if matches!(
        change,
        Change::DiskAccess {
            value: DiskAccess::ReadWrite,
            ..
        }
    ) && VdiRecord::new(&record).read_only() != false
    {
        return Err(Error::DeviceInUse);
    }
    Ok(Current {
        reference,
        record,
        attachment: Some((vr, v)),
    })
}
fn subject<'a>(current: &'a Current, change: &Change) -> (&'a str, &'a Value) {
    match change {
        Change::DiskMetadata { .. } | Change::DisableCbt { .. } => {
            (&current.reference, &current.record)
        }
        Change::NicRate { .. } => (&current.reference, &current.record),
        _ => {
            let (r, v) = current
                .attachment
                .as_ref()
                .expect("disk attachment checked");
            (r, v)
        }
    }
}
fn qos(record: &Value) -> Result<DeviceQos, Error> {
    let q = DeviceQos {
        algorithm: record["qos_algorithm_type"]
            .as_str()
            .ok_or(Error::DeviceConflict)?
            .into(),
        parameters: serde_json::from_value(record["qos_algorithm_params"].clone())
            .map_err(|_| Error::DeviceConflict)?,
    };
    if q.valid() {
        Ok(q)
    } else {
        Err(Error::DeviceConflict)
    }
}
fn supported(record: &Value, algorithm: &str) -> Result<(), Error> {
    if !algorithm.is_empty()
        && !record["qos_supported_algorithms"]
            .as_array()
            .is_some_and(|v| v.iter().any(|s| s == algorithm))
    {
        return Err(Error::Protocol(
            "The device does not advertise the requested QoS algorithm",
        ));
    }
    Ok(())
}
fn reviewed_steps(change: &Change) -> Vec<Step> {
    let mut steps = Vec::new();
    let mut add = |field, previous: Value, value: Value, guard| {
        if previous != value {
            steps.push(Step::Property(PropertyStep {
                change: change.clone(),
                field,
                previous,
                value,
                guard,
            }));
        }
    };
    match change {
        Change::DiskMetadata {
            previous_name,
            previous_description,
            name,
            description,
            ..
        } => {
            add(Field::Name, json!(previous_name), json!(name), None);
            add(
                Field::Description,
                json!(previous_description),
                json!(description),
                None,
            );
        }
        Change::DiskAccess {
            previous, value, ..
        } => add(
            Field::Access,
            json!(previous.wire()),
            json!(value.wire()),
            None,
        ),
        Change::DiskPosition {
            previous, value, ..
        } => add(Field::Position, json!(previous), json!(value), None),
        Change::Bootable {
            previous, value, ..
        } => add(Field::Bootable, json!(previous), json!(value), None),
        Change::DisableCbt { .. } => {
            add(Field::Cbt, json!(true), json!(false), None);
        }
        Change::DiskPriority {
            previous, priority, ..
        } => {
            let algorithm = if priority.is_some() { "ionice" } else { "" };
            let mut params = previous.parameters.clone();
            if let Some(priority) = priority {
                params.insert("class".into(), priority.to_string());
                params.insert("sched".into(), "be".into());
            }
            add(
                Field::QosParameters,
                json!(previous.parameters),
                json!(params),
                Some(json!(previous.algorithm)),
            );
            add(
                Field::QosAlgorithm,
                json!(previous.algorithm),
                json!(algorithm),
                Some(json!(params)),
            );
        }
        Change::NicRate {
            previous,
            kilobytes_per_second,
            ..
        } => {
            let algorithm = if kilobytes_per_second.is_some() {
                "ratelimit"
            } else {
                ""
            };
            let mut params = previous.parameters.clone();
            if let Some(value) = kilobytes_per_second {
                params.insert("kbps".into(), value.to_string());
            }
            add(
                Field::QosParameters,
                json!(previous.parameters),
                json!(params),
                Some(json!(previous.algorithm)),
            );
            add(
                Field::QosAlgorithm,
                json!(previous.algorithm),
                json!(algorithm),
                Some(json!(params)),
            );
        }
    }
    steps
}

pub(super) fn requirements(change: &Change) -> crate::PermissionRequirements {
    crate::PermissionRequirements::writes(reviewed_steps(change).iter().map(|step| {
        let Step::Property(step) = step else {
            unreachable!()
        };
        step.method()
    }))
}

pub(super) async fn plan(session: &Session, vm: VmId, change: &Change) -> Result<Plan, Error> {
    change.validate()?;
    let current = current(session, vm, change, false).await?;
    let (_, record) = subject(&current, change);
    match change {
        Change::DisableCbt { .. } => crate::devices::permits_cbt_disable(record)?,
        Change::DiskPriority {
            previous, priority, ..
        } => {
            if qos(record)? != *previous {
                return Err(Error::DeviceConflict);
            }
            supported(record, if priority.is_some() { "ionice" } else { "" })?;
        }
        Change::NicRate {
            previous,
            kilobytes_per_second,
            ..
        } => {
            if qos(record)? != *previous {
                return Err(Error::DeviceConflict);
            }
            supported(
                record,
                if kilobytes_per_second.is_some() {
                    "ratelimit"
                } else {
                    ""
                },
            )?;
        }
        _ => (),
    }
    let steps = reviewed_steps(change);
    if steps.is_empty() {
        return Err(Error::InvalidDeviceConfiguration);
    }
    for step in &steps {
        let Step::Property(step) = step else {
            unreachable!()
        };
        if record[step.field.wire()] != step.previous {
            return Err(Error::DeviceConflict);
        }
    }
    if let Change::DiskPosition { value, .. } = change {
        let vm_ref = guest(session, vm, true).await?;
        allowed_slot(session, &vm_ref, "VBD", value).await?;
    }
    Ok(Plan {
        evidence: vec![None; steps.len()],
        steps,
    })
}
pub(super) async fn prepare(
    session: &Session,
    vm: VmId,
    step: &PropertyStep,
) -> Result<Prepared, Error> {
    let current = current(session, vm, &step.change, false).await?;
    let (reference, record) = subject(&current, &step.change);
    if record[step.field.wire()] != step.previous {
        return Err(Error::DeviceConflict);
    }
    if let Some(guard) = &step.guard {
        let field = if step.field == Field::QosParameters {
            "qos_algorithm_type"
        } else {
            "qos_algorithm_params"
        };
        if record[field] != *guard {
            return Err(Error::DeviceConflict);
        }
    }
    if step.field == Field::Position {
        let vm_ref = guest(session, vm, true).await?;
        allowed_slot(
            session,
            &vm_ref,
            "VBD",
            step.value.as_str().ok_or(Error::DeviceConflict)?,
        )
        .await?;
    }
    if step.field == Field::Cbt {
        crate::devices::permits_cbt_disable(record)?;
    }
    if step.field == Field::QosAlgorithm {
        supported(record, step.value.as_str().ok_or(Error::DeviceConflict)?)?;
    }
    let method = step.method();
    let mut parameters = vec![json!(reference)];
    if step.field != Field::Cbt {
        parameters.push(step.value.clone());
    }
    Ok(Prepared { method, parameters })
}
pub(super) async fn verify(
    session: &Session,
    vm: VmId,
    step: &PropertyStep,
) -> Result<bool, Error> {
    let current = current(session, vm, &step.change, true).await?;
    let (_, record) = subject(&current, &step.change);
    let algorithm_matches = match (&step.field, &step.change) {
        (
            Field::QosParameters,
            Change::DiskPriority {
                previous, priority, ..
            },
        ) => {
            record["qos_algorithm_type"] == previous.algorithm
                || record["qos_algorithm_type"] == if priority.is_some() { "ionice" } else { "" }
        }
        (
            Field::QosParameters,
            Change::NicRate {
                previous,
                kilobytes_per_second,
                ..
            },
        ) => {
            record["qos_algorithm_type"] == previous.algorithm
                || record["qos_algorithm_type"]
                    == if kilobytes_per_second.is_some() {
                        "ratelimit"
                    } else {
                        ""
                    }
        }
        _ => true,
    };
    Ok(record[step.field.wire()] == step.value
        && algorithm_matches
        && (step.field != Field::QosAlgorithm
            || step
                .guard
                .as_ref()
                .is_none_or(|params| record["qos_algorithm_params"] == *params)))
}
