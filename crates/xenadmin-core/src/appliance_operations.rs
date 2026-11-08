use super::{
    ResourceAction, ResourceOperationRecord, ResourceTarget,
    xapi::{self, Check, Step},
};
use crate::{
    ApplianceId, ApplianceOperation as Op, ApplianceOrder, AppliancePower, Error, PowerState, VmId,
    xapi::Session,
};
use serde_json::{Value, json};
use std::collections::BTreeSet;

#[derive(Clone)]
pub(super) enum Verify {
    Async,
    Field {
        vm: Option<VmId>,
        field: &'static str,
        previous: Value,
        value: Value,
    },
    Membership,
}
fn operation(record: &ResourceOperationRecord) -> Result<&Op, Error> {
    match &record.action {
        ResourceAction::Appliance(op) => Ok(op),
        _ => Err(Error::InvalidResourceCommand),
    }
}
fn group_id(op: &Op) -> Result<ApplianceId, Error> {
    match op {
        Op::Create { .. } => Err(Error::InvalidResourceCommand),
        Op::Rename { appliance, .. }
        | Op::DeleteEmpty { appliance, .. }
        | Op::Membership { appliance, .. }
        | Op::SetOrder { appliance, .. }
        | Op::Power { appliance, .. } => Ok(*appliance),
    }
}
fn idle(record: &Value) -> Result<(), Error> {
    match record["current_operations"].as_object() {
        Some(v) if v.is_empty() => Ok(()),
        Some(_) => Err(Error::ResourceInUse),
        None => Err(Error::Protocol("vApp task information unavailable")),
    }
}
fn ref_value(value: &Value) -> Result<String, Error> {
    xapi::reply_reference(value)
}
async fn group(session: &Session, id: ApplianceId) -> Result<(String, Value), Error> {
    xapi::read(session, ResourceTarget::Appliance(id)).await
}
async fn vm(session: &Session, id: VmId) -> Result<(String, Value), Error> {
    let reference = ref_value(
        &session
            .call("VM.get_by_uuid", vec![json!(id.to_string())])
            .await?,
    )?;
    let record = session
        .call("VM.get_record", vec![json!(&reference)])
        .await?;
    if record["uuid"] != id.to_string() {
        return Err(Error::Stale);
    }
    if record["is_a_template"] != false
        || record["is_a_snapshot"] != false
        || record["is_control_domain"] != false
    {
        return Err(Error::InvalidResourceCommand);
    }
    Ok((reference, record))
}
async fn members(session: &Session, record: &Value) -> Result<BTreeSet<VmId>, Error> {
    let refs = record["VMs"]
        .as_array()
        .filter(|v| v.len() <= 256)
        .ok_or(Error::Protocol(
            "vApp membership unavailable or exceeds 256 VMs",
        ))?;
    let mut ids = BTreeSet::new();
    for reference in refs {
        let r = session
            .call("VM.get_record", vec![json!(ref_value(reference)?)])
            .await?;
        let id = r["uuid"]
            .as_str()
            .and_then(|s| VmId::parse(s).ok())
            .ok_or(Error::Protocol("vApp member UUID unavailable"))?;
        if !ids.insert(id) {
            return Err(Error::ResourceConflict);
        }
    }
    Ok(ids)
}
fn number(value: &Value) -> Option<i64> {
    value.as_i64().or_else(|| value.as_str()?.parse().ok())
}
fn order(record: &Value) -> Result<ApplianceOrder, Error> {
    Ok(ApplianceOrder {
        order: number(&record["order"]).ok_or(Error::ResourceConflict)?,
        start_delay: number(&record["start_delay"])
            .and_then(|v| v.try_into().ok())
            .ok_or(Error::ResourceConflict)?,
        shutdown_delay: number(&record["shutdown_delay"])
            .and_then(|v| v.try_into().ok())
            .ok_or(Error::ResourceConflict)?,
    })
}
fn power(record: &Value) -> Result<PowerState, Error> {
    match record["power_state"].as_str() {
        Some("Running") => Ok(PowerState::Running),
        Some("Halted") => Ok(PowerState::Halted),
        Some("Paused") => Ok(PowerState::Paused),
        Some("Suspended") => Ok(PowerState::Suspended),
        _ => Err(Error::ResourceConflict),
    }
}
fn field_equal(actual: &Value, expected: &Value, field: &str) -> bool {
    if matches!(field, "order" | "start_delay" | "shutdown_delay") {
        number(actual).is_some() && number(actual) == number(expected)
    } else {
        actual == expected
    }
}
fn step(method: String, check: Verify, label: &str) -> Step {
    Step {
        label: label.into(),
        method,
        argument: None,
        expected: Value::Null,
        check: Check::Appliance(check),
    }
}
fn checks(op: &Op) -> Vec<Verify> {
    let mut checks = Vec::new();
    let mut field = |vm, field, previous: Value, value: Value| {
        if previous != value {
            checks.push(Verify::Field {
                vm,
                field,
                previous,
                value,
            });
        }
    };
    match op {
        Op::Rename {
            previous_name,
            previous_description,
            name,
            description,
            ..
        } => {
            field(None, "name_label", json!(previous_name), json!(name));
            field(
                None,
                "name_description",
                json!(previous_description),
                json!(description),
            );
        }
        Op::SetOrder {
            vm,
            previous,
            value,
            ..
        } => {
            field(
                Some(*vm),
                "order",
                json!(previous.order.to_string()),
                json!(value.order.to_string()),
            );
            field(
                Some(*vm),
                "start_delay",
                json!(previous.start_delay.to_string()),
                json!(value.start_delay.to_string()),
            );
            field(
                Some(*vm),
                "shutdown_delay",
                json!(previous.shutdown_delay.to_string()),
                json!(value.shutdown_delay.to_string()),
            );
        }
        Op::Membership { .. } => checks.push(Verify::Membership),
        _ => checks.push(Verify::Async),
    }
    checks
}
pub(super) fn reconciliation_checks(record: &ResourceOperationRecord) -> Vec<Check> {
    operation(record)
        .map(|op| checks(op).into_iter().map(Check::Appliance).collect())
        .unwrap_or_default()
}
pub(super) async fn prepare(
    session: &Session,
    record: &mut ResourceOperationRecord,
) -> Result<Vec<Step>, Error> {
    let op = operation(record)?.clone();
    if matches!(op, Op::Create { .. }) {
        let all = session.call("VM_appliance.get_all_records", vec![]).await?;
        let all = all
            .as_object()
            .filter(|r| r.len() <= 10_000)
            .ok_or(Error::Protocol("vApp baseline unavailable"))?;
        record.appliance_baseline = Some(
            all.values()
                .map(|r| {
                    r["uuid"]
                        .as_str()
                        .and_then(|s| ApplianceId::parse(s).ok())
                        .ok_or(Error::ResourceConflict)
                })
                .collect::<Result<_, _>>()?,
        );
    }
    let mut result = Vec::new();
    for check in checks(&op) {
        let method = match &check {
            Verify::Field { vm, field, .. } => format!(
                "{}.set_{field}",
                if vm.is_some() { "VM" } else { "VM_appliance" }
            ),
            Verify::Membership => "VM.set_appliance".into(),
            Verify::Async => format!(
                "Async.VM_appliance.{}",
                match &op {
                    Op::Create { .. } => "create",
                    Op::DeleteEmpty { .. } => "destroy",
                    Op::Power { action, .. } => action.wire(),
                    _ => return Err(Error::InvalidResourceCommand),
                }
            ),
        };
        arguments(session, record, &check).await?;
        result.push(step(method, check, op.label()));
    }
    Ok(result)
}
pub(super) async fn arguments(
    session: &Session,
    record: &ResourceOperationRecord,
    check: &Verify,
) -> Result<Vec<Value>, Error> {
    let op = operation(record)?;
    if let Op::Create { name, description } = op {
        return Ok(vec![
            json!({"name_label":name,"name_description":description}),
        ]);
    }
    let (reference, fresh) = group(session, group_id(op)?).await?;
    idle(&fresh)?;
    match check {
        Verify::Field {
            vm: target,
            field,
            previous,
            value,
        } => {
            let (reference, fresh) = if let Some(id) = target {
                let (vr, v) = vm(session, *id).await?;
                idle(&v)?;
                if v["appliance"] != reference || !members(session, &fresh).await?.contains(id) {
                    return Err(Error::ResourceConflict);
                }
                (vr, v)
            } else {
                (reference, fresh)
            };
            if !field_equal(&fresh[*field], previous, field) {
                return Err(Error::ResourceConflict);
            }
            Ok(vec![json!(reference), value.clone()])
        }
        Verify::Membership => {
            let Op::Membership {
                expected_members,
                vm: id,
                join,
                ..
            } = op
            else {
                return Err(Error::InvalidResourceCommand);
            };
            if members(session, &fresh).await? != expected_members.iter().copied().collect() {
                return Err(Error::ResourceConflict);
            }
            let (vr, v) = vm(session, *id).await?;
            idle(&v)?;
            let previous = if *join {
                "OpaqueRef:NULL"
            } else {
                reference.as_str()
            };
            if v["appliance"] != previous {
                return Err(Error::ResourceConflict);
            }
            Ok(vec![
                json!(vr),
                json!(if *join {
                    reference.as_str()
                } else {
                    "OpaqueRef:NULL"
                }),
            ])
        }
        Verify::Async => match op {
            Op::DeleteEmpty { expected_name, .. } => {
                if fresh["name_label"] != *expected_name
                    || !members(session, &fresh).await?.is_empty()
                {
                    return Err(Error::ResourceConflict);
                }
                Ok(vec![json!(reference)])
            }
            Op::Power {
                members: reviewed,
                action,
                ..
            } => {
                if members(session, &fresh).await? != reviewed.iter().map(|m| m.vm).collect()
                    || !fresh["allowed_operations"]
                        .as_array()
                        .is_some_and(|v| v.iter().any(|op| op == action.wire()))
                {
                    return Err(Error::ResourceConflict);
                }
                let mut change = false;
                for member in reviewed {
                    let (vr, v) = vm(session, member.vm).await?;
                    idle(&v)?;
                    if v["appliance"] != reference
                        || order(&v)? != member.order
                        || power(&v)? != member.state
                    {
                        return Err(Error::ResourceConflict);
                    }
                    if member.state == action.destination() {
                        continue;
                    }
                    if *action == AppliancePower::Start && member.state != PowerState::Halted {
                        return Err(Error::UnavailableAction);
                    }
                    session
                        .call(
                            "VM.assert_operation_valid",
                            vec![json!(vr), json!(action.wire())],
                        )
                        .await?;
                    change = true;
                }
                if !change {
                    return Err(Error::UnavailableAction);
                }
                let mut args = vec![json!(reference)];
                if *action == AppliancePower::Start {
                    args.push(json!(false));
                }
                Ok(args)
            }
            _ => Err(Error::InvalidResourceCommand),
        },
    }
}
pub(super) async fn verify(
    session: &Session,
    record: &ResourceOperationRecord,
    check: &Verify,
) -> Result<Option<ResourceTarget>, Error> {
    let op = operation(record)?;
    if let Verify::Async = check {
        if !record.appliance_completed {
            return Err(Error::Protocol(
                "vApp completion receipt unavailable; no request will be replayed",
            ));
        }
        if let Op::Create { name, description } = op {
            let reference = record
                .result_reference
                .as_ref()
                .ok_or(Error::ResourceConflict)?;
            let r = session
                .call("VM_appliance.get_record", vec![json!(reference)])
                .await?;
            let id = r["uuid"]
                .as_str()
                .and_then(|s| ApplianceId::parse(s).ok())
                .ok_or(Error::ResourceConflict)?;
            if record
                .appliance_baseline
                .as_ref()
                .ok_or(Error::ResourceConflict)?
                .contains(&id)
                || r["name_label"] != *name
                || r["name_description"] != *description
                || !members(session, &r).await?.is_empty()
            {
                return Err(Error::ResourceConflict);
            }
            return Ok(Some(ResourceTarget::Appliance(id)));
        }
        if let Op::DeleteEmpty { appliance, .. } = op {
            return match group(session, *appliance).await {
                Err(Error::Api(e)) if e == "UUID_INVALID" => Ok(None),
                Err(e) => Err(e),
                Ok(_) => Err(Error::ResourceConflict),
            };
        }
    }
    let (reference, fresh) = group(session, group_id(op)?).await?;
    match check {
        Verify::Field {
            vm: target,
            field,
            value,
            ..
        } => {
            let fresh = if let Some(id) = target {
                let (_, v) = vm(session, *id).await?;
                if v["appliance"] != reference || !members(session, &fresh).await?.contains(id) {
                    return Err(Error::ResourceConflict);
                }
                v
            } else {
                fresh
            };
            if !field_equal(&fresh[*field], value, field) {
                return Err(Error::ResourceConflict);
            }
        }
        Verify::Membership => {
            let Op::Membership {
                expected_members,
                vm: id,
                join,
                ..
            } = op
            else {
                return Err(Error::InvalidResourceCommand);
            };
            let mut expected: BTreeSet<_> = expected_members.iter().copied().collect();
            if *join {
                expected.insert(*id);
            } else {
                expected.remove(id);
            }
            let (_, v) = vm(session, *id).await?;
            if members(session, &fresh).await? != expected
                || v["appliance"]
                    != if *join {
                        reference.as_str()
                    } else {
                        "OpaqueRef:NULL"
                    }
            {
                return Err(Error::ResourceConflict);
            }
        }
        Verify::Async => {
            let Op::Power {
                members: reviewed,
                action,
                ..
            } = op
            else {
                return Err(Error::InvalidResourceCommand);
            };
            idle(&fresh)?;
            if members(session, &fresh).await? != reviewed.iter().map(|m| m.vm).collect() {
                return Err(Error::ResourceConflict);
            }
            for member in reviewed {
                let (_, v) = vm(session, member.vm).await?;
                idle(&v)?;
                if v["appliance"] != reference
                    || order(&v)? != member.order
                    || power(&v)? != action.destination()
                {
                    return Err(Error::ResourceConflict);
                }
            }
        }
    }
    Ok(None)
}
