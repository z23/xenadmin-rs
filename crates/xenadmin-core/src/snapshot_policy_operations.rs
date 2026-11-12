use super::{
    ResourceAction, ResourceOperationRecord, ResourceTarget,
    xapi::{self, Check, Step},
};
use crate::{
    Error, ScheduledSnapshotKind, SnapshotId, SnapshotPolicyConfig, SnapshotPolicyId,
    SnapshotPolicyOperation as Op, SnapshotPolicyState as State, SnapshotSchedule, VmId,
    xapi::Session,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use xenadmin_xapi::enumerations::vmss_type;
use xenadmin_xapi::records::{TaskRecord, VmRecord, VmssRecord};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Evidence {
    pub baseline: Option<Vec<SnapshotPolicyId>>,
    pub snapshots: Option<BTreeMap<VmId, Vec<SnapshotId>>>,
    pub last_run: Option<String>,
    pub completed: bool,
    pub plugin_success: bool,
}
impl Evidence {
    pub(crate) fn accepts(&self, new: &Self) -> bool {
        (!self.completed || new.completed)
            && (!self.plugin_success || new.plugin_success)
            && (self.baseline.is_none() || self.baseline == new.baseline)
            && (self.snapshots.is_none() || self.snapshots == new.snapshots)
            && (self.last_run.is_none() || self.last_run == new.last_run)
    }
}
#[derive(Clone)]
pub(super) enum Verify {
    Async,
    Field {
        before: State,
        after: State,
        field: &'static str,
        value: Value,
    },
    Membership {
        before: State,
        after: State,
        vm: VmId,
        join: bool,
    },
}
fn op(record: &ResourceOperationRecord) -> Result<&Op, Error> {
    if let ResourceAction::SnapshotPolicy(op) = &record.action {
        Ok(op)
    } else {
        Err(Error::InvalidResourceCommand)
    }
}
fn decode(id: SnapshotPolicyId, r: &Value, members: Vec<VmId>) -> Result<State, Error> {
    let map = VmssRecord::new(r)
        .schedule()
        .as_object()
        .ok_or(Error::ResourceConflict)?
        .iter()
        .map(|(k, v)| {
            v.as_str()
                .map(|v| (k.clone(), v.to_owned()))
                .ok_or(Error::ResourceConflict)
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let config = SnapshotPolicyConfig {
        name: VmssRecord::new(r)
            .name_label()
            .as_str()
            .ok_or(Error::ResourceConflict)?
            .into(),
        description: VmssRecord::new(r)
            .name_description()
            .as_str()
            .ok_or(Error::ResourceConflict)?
            .into(),
        kind: match VmssRecord::new(r).type_().as_str() {
            Some(vmss_type::SNAPSHOT) => ScheduledSnapshotKind::Disk,
            Some(vmss_type::CHECKPOINT) => ScheduledSnapshotKind::Checkpoint,
            _ => return Err(Error::UnavailableAction),
        },
        retained: VmssRecord::new(r)
            .retained_snapshots()
            .as_u64()
            .or_else(|| {
                VmssRecord::new(r)
                    .retained_snapshots()
                    .as_str()?
                    .parse()
                    .ok()
            })
            .and_then(|n| n.try_into().ok())
            .ok_or(Error::ResourceConflict)?,
        schedule: SnapshotSchedule::decode(
            VmssRecord::new(r)
                .frequency()
                .as_str()
                .ok_or(Error::ResourceConflict)?,
            &map,
        )?,
    };
    config.validate()?;
    Ok(State {
        id,
        config,
        enabled: VmssRecord::new(r)
            .enabled()
            .as_bool()
            .ok_or(Error::ResourceConflict)?,
        members,
    })
}
async fn members(session: &Session, r: &Value) -> Result<Vec<VmId>, Error> {
    let refs = VmssRecord::new(r)
        .vms()
        .value()
        .as_array()
        .filter(|v| v.len() <= 256)
        .ok_or(Error::ResourceConflict)?;
    let mut result = BTreeSet::new();
    for vr in refs {
        let vr = xapi::reply_reference(vr)?;
        let v = session.call("VM.get_record", vec![json!(vr)]).await?;
        let id = VmRecord::new(&v)
            .uuid()
            .as_str()
            .and_then(|s| VmId::parse(s).ok())
            .ok_or(Error::ResourceConflict)?;
        if !result.insert(id) {
            return Err(Error::ResourceConflict);
        }
    }
    Ok(result.into_iter().collect())
}
async fn read(session: &Session, id: SnapshotPolicyId) -> Result<(String, Value, State), Error> {
    let (r, v) = xapi::read(session, ResourceTarget::SnapshotPolicy(id)).await?;
    let state = decode(id, &v, members(session, &v).await?)?;
    Ok((r, v, state))
}
fn same(a: &State, b: &State) -> bool {
    a.id == b.id
        && a.config == b.config
        && a.enabled == b.enabled
        && a.members.iter().collect::<BTreeSet<_>>() == b.members.iter().collect()
}
async fn vm(session: &Session, id: VmId) -> Result<(String, Value), Error> {
    let r = xapi::reply_reference(
        &session
            .call("VM.get_by_uuid", vec![json!(id.to_string())])
            .await?,
    )?;
    let v = session.call("VM.get_record", vec![json!(&r)]).await?;
    if VmRecord::new(&v).uuid() != id.to_string() {
        return Err(Error::Stale);
    }
    if VmRecord::new(&v).is_a_template() != false
        || VmRecord::new(&v).is_a_snapshot() != false
        || VmRecord::new(&v).is_control_domain() != false
        || !VmRecord::new(&v)
            .current_operations()
            .as_object()
            .is_some_and(|m| m.is_empty())
    {
        return Err(Error::ResourceInUse);
    }
    Ok((r, v))
}
fn checks(op: &Op) -> Vec<Verify> {
    let mut result = Vec::new();
    match op {
        Op::Configure {
            expected, config, ..
        } => {
            let mut state = expected.clone();
            macro_rules! change {
                ($field:literal,$value:expr,$update:expr) => {{
                    let before = state.clone();
                    $update(&mut state);
                    if before != state {
                        result.push(Verify::Field {
                            before,
                            after: state.clone(),
                            field: $field,
                            value: $value,
                        });
                    }
                }};
            }
            change!("name_label", json!(config.name), |s: &mut State| s
                .config
                .name =
                config.name.clone());
            change!(
                "name_description",
                json!(config.description),
                |s: &mut State| s.config.description = config.description.clone()
            );
            change!("type", json!(config.kind.wire()), |s: &mut State| s
                .config
                .kind =
                config.kind);
            change!(
                "retained_snapshots",
                json!(config.retained.to_string()),
                |s: &mut State| s.config.retained = config.retained
            );
            if state.config.schedule.frequency() != config.schedule.frequency() {
                let old = state.config.schedule.fields();
                let minute = old["min"].parse().unwrap_or(0);
                let hour = old.get("hour").and_then(|s| s.parse().ok()).unwrap_or(0);
                let normalized = match &config.schedule {
                    SnapshotSchedule::Hourly { .. } => SnapshotSchedule::Hourly { minute },
                    SnapshotSchedule::Daily { .. } => SnapshotSchedule::Daily { hour, minute },
                    SnapshotSchedule::Weekly { .. } => SnapshotSchedule::Weekly {
                        hour,
                        minute,
                        days: vec![0],
                    },
                };
                change!(
                    "frequency",
                    json!(config.schedule.frequency()),
                    |s: &mut State| s.config.schedule = normalized.clone()
                );
            }
            change!(
                "schedule",
                json!(config.schedule.fields()),
                |s: &mut State| s.config.schedule = config.schedule.clone()
            );
        }
        Op::SetEnabled {
            expected, enabled, ..
        } => {
            let mut after = expected.clone();
            after.enabled = *enabled;
            result.push(Verify::Field {
                before: expected.clone(),
                after,
                field: "enabled",
                value: json!(enabled),
            });
        }
        Op::Membership { expected, vm, join } => {
            let mut after = expected.clone();
            if *join {
                after.members.push(*vm)
            } else {
                after.members.retain(|id| id != vm)
            }
            result.push(Verify::Membership {
                before: expected.clone(),
                after,
                vm: *vm,
                join: *join,
            });
        }
        _ => result.push(Verify::Async),
    }
    result
}
pub(super) fn reconciliation_checks(r: &ResourceOperationRecord) -> Vec<Check> {
    op(r)
        .map(|o| {
            checks(o)
                .into_iter()
                .map(|v| Check::SnapshotPolicy(Box::new(v)))
                .collect()
        })
        .unwrap_or_default()
}
fn method(op: &Op, check: &Verify) -> Result<String, Error> {
    Ok(match check {
        Verify::Field { field, .. } => format!("VMSS.set_{field}"),
        Verify::Membership { .. } => "VM.set_snapshot_schedule".into(),
        Verify::Async => match op {
            Op::Create { .. } => "Async.VMSS.create".into(),
            Op::DeleteEmpty { .. } => "Async.VMSS.destroy".into(),
            Op::RunNow { .. } => "VMSS.snapshot_now".into(),
            _ => return Err(Error::InvalidResourceCommand),
        },
    })
}
impl Op {
    pub fn permission_requirements(&self) -> crate::PermissionRequirements {
        let op = self;
        let methods = checks(op)
            .iter()
            .map(|check| method(op, check))
            .collect::<Result<Vec<_>, _>>();
        match methods {
            Ok(methods) => crate::PermissionRequirements::writes(methods),
            Err(_) => crate::PermissionRequirements::writes(Vec::<String>::new())
                .with_host_dependent_steps(),
        }
    }
}
pub(super) async fn prepare(
    session: &Session,
    r: &mut ResourceOperationRecord,
) -> Result<Vec<Step>, Error> {
    let operation = op(r)?.clone();
    let mut evidence = Evidence::default();
    if matches!(operation, Op::Create { .. }) {
        let all = session.call("VMSS.get_all_records", vec![]).await?;
        let map = all
            .as_object()
            .filter(|m| m.len() <= 10_000)
            .ok_or(Error::ResourceConflict)?;
        evidence.baseline = Some(
            map.values()
                .map(|v| {
                    v["uuid"]
                        .as_str()
                        .and_then(|s| SnapshotPolicyId::parse(s).ok())
                        .ok_or(Error::ResourceConflict)
                })
                .collect::<Result<_, _>>()?,
        );
    }
    if let Op::RunNow { expected } = &operation {
        let (_, record, fresh) = read(session, expected.id).await?;
        if !same(&fresh, expected) {
            return Err(Error::ResourceConflict);
        }
        evidence.last_run = Some(
            VmssRecord::new(&record)
                .last_run_time()
                .as_str()
                .ok_or(Error::ResourceConflict)?
                .to_owned(),
        );
        evidence.snapshots = Some(snapshot_ids(session, expected, false).await?);
    }
    r.snapshot_policy_evidence = Some(Box::new(evidence));
    let mut steps = Vec::new();
    for check in checks(&operation) {
        let method = method(&operation, &check)?;
        if steps.is_empty() {
            arguments(session, r, &check).await?;
        }
        steps.push(Step {
            label: operation.label().into(),
            method,
            argument: None,
            expected: Value::Null,
            check: Check::SnapshotPolicy(Box::new(check)),
        });
    }
    Ok(steps)
}
async fn support(
    session: &Session,
    r: &ResourceOperationRecord,
    clock: Option<crate::SnapshotPolicyClock>,
) -> Result<(), Error> {
    let fresh = crate::snapshot_policy_support::read(session, ResourceTarget::Pool(r.pool)).await?;
    if clock.is_some_and(|c| c != fresh.clock) {
        return Err(Error::Protocol(
            "Coordinator or local timezone changed since review; refresh schedule support",
        ));
    }
    Ok(())
}
pub(super) async fn arguments(
    session: &Session,
    r: &ResourceOperationRecord,
    check: &Verify,
) -> Result<Vec<Value>, Error> {
    let operation = op(r)?;
    if let Op::Create { config, clock } = operation {
        support(session, r, Some(*clock)).await?;
        return Ok(vec![
            json!({"name_label":config.name,"name_description":config.description,"type":config.kind.wire(),"enabled":false,"retained_snapshots":config.retained.to_string(),"frequency":config.schedule.frequency(),"schedule":config.schedule.fields()}),
        ]);
    }
    let expected = operation.expected().ok_or(Error::InvalidResourceCommand)?;
    let (reference, _, fresh) = read(session, expected.id).await?;
    match check {
        Verify::Field { before, value, .. } => {
            if !same(&fresh, before) {
                return Err(Error::ResourceConflict);
            }
            match operation {
                Op::Configure { clock, .. } => support(session, r, Some(*clock)).await?,
                Op::SetEnabled {
                    enabled: true,
                    clock,
                    ..
                } => {
                    support(session, r, *clock).await?;
                    snapshot_ids(session, expected, true).await?;
                }
                _ => {}
            }
            Ok(vec![json!(reference), value.clone()])
        }
        Verify::Membership {
            before,
            vm: id,
            join,
            ..
        } => {
            if !same(&fresh, before) {
                return Err(Error::ResourceConflict);
            }
            let (vr, v) = vm(session, *id).await?;
            if VmRecord::new(&v).snapshot_schedule()
                != if *join {
                    "OpaqueRef:NULL"
                } else {
                    reference.as_str()
                }
            {
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
        Verify::Async => {
            if !same(&fresh, expected) {
                return Err(Error::ResourceConflict);
            }
            match operation {
                Op::DeleteEmpty { .. } => Ok(vec![json!(reference)]),
                Op::RunNow { .. } => {
                    support(session, r, None).await?;
                    snapshot_ids(session, expected, true).await?;
                    Ok(vec![json!(reference)])
                }
                _ => Err(Error::InvalidResourceCommand),
            }
        }
    }
}
async fn snapshot_ids(
    session: &Session,
    state: &State,
    validate: bool,
) -> Result<BTreeMap<VmId, Vec<SnapshotId>>, Error> {
    let mut result = BTreeMap::new();
    let (pr, _, _) = read(session, state.id).await?;
    for id in &state.members {
        let (vr, v) = vm(session, *id).await?;
        if VmRecord::new(&v).snapshot_schedule() != pr {
            return Err(Error::ResourceConflict);
        }
        if validate {
            session
                .call(
                    "VM.assert_operation_valid",
                    vec![json!(&vr), json!(state.config.kind.wire())],
                )
                .await?;
        }
        let refs = VmRecord::new(&v)
            .snapshots()
            .value()
            .as_array()
            .filter(|v| v.len() <= 10_000)
            .ok_or(Error::ResourceConflict)?;
        let mut ids = Vec::new();
        for sr in refs {
            let snap = session
                .call("VM.get_record", vec![json!(xapi::reply_reference(sr)?)])
                .await?;
            if VmRecord::new(&snap).is_a_snapshot() != true
                || VmRecord::new(&snap).snapshot_of() != vr
            {
                return Err(Error::ResourceConflict);
            }
            if VmRecord::new(&snap).is_vmss_snapshot() == true {
                ids.push(
                    VmRecord::new(&snap)
                        .uuid()
                        .as_str()
                        .and_then(|s| SnapshotId::parse(s).ok())
                        .ok_or(Error::ResourceConflict)?,
                );
            }
        }
        ids.sort();
        ids.dedup();
        result.insert(*id, ids);
    }
    Ok(result)
}
/// The synchronous `VMSS.snapshot_now` reply is the plugin result itself.
pub(super) fn plugin_success_reply(value: &Value) -> bool {
    plugin_success(&json!({ "result": value }))
}
pub(super) fn plugin_success(task: &Value) -> bool {
    let Some(s) = TaskRecord::new(task)
        .result()
        .value()
        .as_str()
        .filter(|s| s.len() <= 4096)
    else {
        return false;
    };
    if s == "True" {
        return true;
    }
    let Ok(doc) = roxmltree::Document::parse(s) else {
        return false;
    };
    let root = doc.root_element();
    if root.tag_name().name() != "value" {
        return false;
    }
    let children = root
        .children()
        .filter(|n| n.is_element())
        .collect::<Vec<_>>();
    if children.is_empty() {
        return root.text() == Some("True");
    }
    children.len() == 1
        && children[0].tag_name().name() == "string"
        && children[0].text() == Some("True")
}
pub(super) async fn verify(
    session: &Session,
    r: &ResourceOperationRecord,
    check: &Verify,
) -> Result<Option<ResourceTarget>, Error> {
    let operation = op(r)?;
    if matches!(check, Verify::Async) {
        let e = r
            .snapshot_policy_evidence
            .as_ref()
            .filter(|e| e.completed)
            .ok_or(Error::Protocol(
                "Snapshot schedule task completion unavailable; no write will be replayed",
            ))?;
        match operation {
            Op::Create { config, .. } => {
                let rr = r.result_reference.as_ref().ok_or(Error::ResourceConflict)?;
                let v = session.call("VMSS.get_record", vec![json!(rr)]).await?;
                let id = VmssRecord::new(&v)
                    .uuid()
                    .as_str()
                    .and_then(|s| SnapshotPolicyId::parse(s).ok())
                    .ok_or(Error::ResourceConflict)?;
                let state = decode(id, &v, members(session, &v).await?)?;
                if e.baseline
                    .as_ref()
                    .ok_or(Error::ResourceConflict)?
                    .contains(&id)
                    || state.config != *config
                    || state.enabled
                    || !state.members.is_empty()
                {
                    return Err(Error::ResourceConflict);
                }
                return Ok(Some(ResourceTarget::SnapshotPolicy(id)));
            }
            Op::DeleteEmpty { expected } => {
                return match read(session, expected.id).await {
                    Err(Error::Api(code)) if code == "UUID_INVALID" => Ok(None),
                    Err(e) => Err(e),
                    Ok(_) => Err(Error::ResourceConflict),
                };
            }
            Op::RunNow { expected } => {
                if !e.plugin_success {
                    return Err(Error::Protocol(
                        "Snapshot schedule plugin did not confirm success; partial snapshots or retention cleanup may remain",
                    ));
                }
                let (_, v, fresh) = read(session, expected.id).await?;
                if !same(&fresh, expected)
                    || crate::snapshot_policy_support::clock_seconds(
                        VmssRecord::new(&v)
                            .last_run_time()
                            .as_str()
                            .ok_or(Error::ResourceConflict)?,
                    )? <= crate::snapshot_policy_support::clock_seconds(
                        e.last_run.as_deref().ok_or(Error::ResourceConflict)?,
                    )?
                {
                    return Err(Error::ResourceConflict);
                }
                let now = snapshot_ids(session, expected, false).await?;
                let before = e.snapshots.as_ref().ok_or(Error::ResourceConflict)?;
                if expected.members.iter().any(|id| {
                    !now.get(id).is_some_and(|ids| {
                        ids.iter()
                            .any(|s| !before.get(id).is_some_and(|v| v.contains(s)))
                    })
                }) {
                    return Err(Error::ResourceConflict);
                }
                return Ok(None);
            }
            _ => return Err(Error::InvalidResourceCommand),
        }
    }
    let expected = operation.expected().ok_or(Error::InvalidResourceCommand)?;
    let (reference, _, fresh) = read(session, expected.id).await?;
    match check {
        Verify::Field { .. } => {
            let all = checks(operation);
            let expected = all
                .iter()
                .zip(&r.steps)
                .rfind(|(_, step)| step.state != crate::StepState::Pending)
                .and_then(|(check, _)| match check {
                    Verify::Field { after, .. } => Some(after),
                    _ => None,
                })
                .ok_or(Error::ResourceConflict)?;
            if !same(&fresh, expected) {
                return Err(Error::ResourceConflict);
            }
        }
        Verify::Membership {
            after,
            vm: id,
            join,
            ..
        } => {
            if !same(&fresh, after) {
                return Err(Error::ResourceConflict);
            }
            let (_, v) = vm(session, *id).await?;
            if VmRecord::new(&v).snapshot_schedule()
                != if *join {
                    reference.as_str()
                } else {
                    "OpaqueRef:NULL"
                }
            {
                return Err(Error::ResourceConflict);
            }
        }
        Verify::Async => unreachable!(),
    };
    Ok(None)
}
