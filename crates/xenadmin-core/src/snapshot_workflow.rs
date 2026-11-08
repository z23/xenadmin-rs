use crate::{Error, PowerState, SnapshotId, SnapshotName, VdiId, VmId, snapshots, xapi::Session};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SnapshotWorkflow {
    Create {
        name: String,
        description: String,
        memory: bool,
    },
    Edit {
        snapshot: SnapshotId,
        expected_name: String,
        expected_description: String,
        name: String,
        description: String,
    },
    Restore {
        snapshot: SnapshotId,
        expected_state: PowerState,
        checkpoint: bool,
        shutdown: bool,
        start_after: bool,
    },
}
impl SnapshotWorkflow {
    pub fn validate(&self) -> Result<(), Error> {
        match self {
            Self::Create {
                name, description, ..
            }
            | Self::Edit {
                name, description, ..
            } => {
                SnapshotName::new(name)?;
                if name.trim() != name
                    || description.chars().count() > 4096
                    || description
                        .chars()
                        .any(|c| c.is_control() && c != '\n' && c != '\t')
                {
                    return Err(Error::InvalidSnapshotMetadata);
                }
                if let Self::Edit {
                    expected_name,
                    expected_description,
                    ..
                } = self
                    && expected_name == name
                    && expected_description == description
                {
                    return Err(Error::InvalidSnapshotMetadata);
                }
                Ok(())
            }
            Self::Restore {
                expected_state,
                shutdown,
                ..
            } => match expected_state {
                PowerState::Halted if !shutdown => Ok(()),
                PowerState::Running if *shutdown => Ok(()),
                _ => Err(Error::SnapshotRequiresHaltedVm),
            },
        }
    }
    pub(crate) fn snapshot(&self) -> Option<SnapshotId> {
        match self {
            Self::Create { .. } => None,
            Self::Edit { snapshot, .. } | Self::Restore { snapshot, .. } => Some(*snapshot),
        }
    }
    pub(crate) fn label(&self) -> &'static str {
        match self {
            Self::Create { memory: true, .. } => "Create memory snapshot",
            Self::Create { .. } => "Create snapshot",
            Self::Edit { .. } => "Edit snapshot properties",
            Self::Restore { .. } => "Restore snapshot",
        }
    }
    pub(crate) fn allows(&self, parent: &Value, snapshot: Option<&Value>) -> bool {
        if self.validate().is_err() || !idle(parent) {
            return false;
        }
        match self {
            Self::Create { memory, .. } => {
                can(parent, if *memory { "checkpoint" } else { "snapshot" })
            }
            Self::Edit {
                expected_name,
                expected_description,
                ..
            } => snapshot.is_some_and(|s| {
                idle(s)
                    && s["name_label"] == *expected_name
                    && s["name_description"] == *expected_description
            }),
            Self::Restore {
                expected_state,
                checkpoint,
                shutdown,
                ..
            } => {
                parent["power_state"] == expected_state.to_string()
                    && (!shutdown || can(parent, "clean_shutdown"))
                    && snapshot.is_some_and(|s| {
                        idle(s)
                            && can(s, "revert")
                            && s["power_state"] == if *checkpoint { "Suspended" } else { "Halted" }
                    })
            }
        }
    }
}
fn idle(record: &Value) -> bool {
    record["current_operations"]
        .as_object()
        .is_some_and(|ops| ops.is_empty())
}
fn can(record: &Value, op: &str) -> bool {
    record["allowed_operations"]
        .as_array()
        .is_some_and(|ops| ops.iter().any(|v| v == op))
        && record["blocked_operations"].get(op).is_none()
}
async fn parent(session: &Session, vm: VmId) -> Result<(String, Value), Error> {
    let value = snapshots::optional_record(session, "VM", &vm.to_string())
        .await?
        .ok_or(Error::Stale)?;
    if value.1["is_a_snapshot"] != false
        || value.1["is_a_template"] != false
        || value.1["is_control_domain"] != false
    {
        return Err(Error::Stale);
    }
    Ok(value)
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Step {
    Create,
    Name,
    Description,
    Shutdown,
    Revert,
    Start,
    Resume,
}
impl Step {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Create => "Create snapshot",
            Self::Name => "Set snapshot name",
            Self::Description => "Set snapshot description",
            Self::Shutdown => "Cleanly shut down VM",
            Self::Revert => "Restore configuration and disks",
            Self::Start => "Start restored VM",
            Self::Resume => "Resume restored memory",
        }
    }
    pub fn asynchronous(&self) -> bool {
        !matches!(self, Self::Name | Self::Description)
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Evidence {
    pub task: Option<String>,
    pub result: Option<String>,
    pub completed: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Plan {
    pub operation: SnapshotWorkflow,
    pub steps: Vec<Step>,
    pub evidence: Vec<Evidence>,
    pub baseline: Vec<VmId>,
    pub created: Option<SnapshotId>,
    pub created_description: Option<String>,
    pub memory: Option<VdiId>,
}
impl Plan {
    pub fn extends(&self, old: &Self) -> bool {
        self.operation == old.operation
            && self.steps == old.steps
            && self.baseline == old.baseline
            && self.memory == old.memory
            && (old.created.is_none() || old.created == self.created)
            && (old.created_description.is_none()
                || old.created_description == self.created_description)
            && self.evidence.len() == old.evidence.len()
            && self.evidence.iter().zip(&old.evidence).all(|(new, old)| {
                (!old.completed || new.completed)
                    && (old.task.is_none() || old.task == new.task)
                    && (old.result.is_none() || old.result == new.result)
            })
    }
    fn snapshot(&self) -> Result<SnapshotId, Error> {
        self.operation
            .snapshot()
            .or(self.created)
            .ok_or(Error::SnapshotConflict)
    }
}
async fn memory(session: &Session, record: &Value) -> Result<Option<VdiId>, Error> {
    if record["power_state"] != "Suspended" {
        return Ok(None);
    }
    let reference = snapshots::reference(&record["suspend_VDI"])?;
    let disk = session
        .call("VDI.get_record", vec![json!(reference)])
        .await?;
    if disk["type"] != "suspend" {
        return Err(Error::SnapshotConflict);
    }
    Ok(Some(
        disk["uuid"]
            .as_str()
            .and_then(|v| VdiId::parse(v).ok())
            .ok_or(Error::SnapshotConflict)?,
    ))
}
pub(crate) async fn plan(
    session: &Session,
    vm: VmId,
    operation: &SnapshotWorkflow,
) -> Result<Plan, Error> {
    operation.validate()?;
    let (_, guest) = parent(session, vm).await?;
    let source = if let Some(id) = operation.snapshot() {
        Some(snapshots::snapshot_record(session, vm, id).await?.1)
    } else {
        None
    };
    if !operation.allows(&guest, source.as_ref()) {
        return Err(Error::UnavailableAction);
    }
    let steps = match operation {
        SnapshotWorkflow::Create { .. } => vec![Step::Create, Step::Description],
        SnapshotWorkflow::Edit {
            name,
            expected_name,
            description,
            expected_description,
            ..
        } => {
            let mut steps = Vec::new();
            if name != expected_name {
                steps.push(Step::Name)
            }
            if description != expected_description {
                steps.push(Step::Description)
            }
            steps
        }
        SnapshotWorkflow::Restore {
            shutdown,
            start_after,
            checkpoint,
            ..
        } => {
            let mut steps = Vec::new();
            if *shutdown {
                steps.push(Step::Shutdown)
            }
            steps.push(Step::Revert);
            if *start_after {
                steps.push(if *checkpoint {
                    Step::Resume
                } else {
                    Step::Start
                })
            }
            steps
        }
    };
    Ok(Plan {
        evidence: vec![Evidence::default(); steps.len()],
        steps,
        operation: operation.clone(),
        baseline: if matches!(operation, SnapshotWorkflow::Create { .. }) {
            snapshots::clone_baseline(session).await?
        } else {
            Vec::new()
        },
        created: None,
        created_description: None,
        memory: if let Some(source) = source {
            memory(session, &source).await?
        } else {
            None
        },
    })
}
pub(crate) struct Prepared {
    pub method: &'static str,
    pub args: Vec<Value>,
}
impl Prepared {
    pub async fn send(self, session: &Session) -> Result<Value, Error> {
        session.call(self.method, self.args).await
    }
}
async fn source(session: &Session, vm: VmId, plan: &Plan) -> Result<(String, Value), Error> {
    let result = snapshots::snapshot_record(session, vm, plan.snapshot()?).await?;
    if !idle(&result.1) {
        return Err(Error::UnavailableAction);
    }
    if let SnapshotWorkflow::Restore { checkpoint, .. } = plan.operation
        && (result.1["power_state"] != if checkpoint { "Suspended" } else { "Halted" }
            || memory(session, &result.1).await? != plan.memory)
    {
        return Err(Error::SnapshotConflict);
    }
    Ok(result)
}
pub(crate) async fn prepare(
    session: &Session,
    vm: VmId,
    plan: &Plan,
    index: usize,
) -> Result<Prepared, Error> {
    let (parent_ref, guest) = parent(session, vm).await?;
    if !idle(&guest) {
        return Err(Error::UnavailableAction);
    }
    let step = &plan.steps[index];
    if *step == Step::Create {
        let SnapshotWorkflow::Create { name, memory, .. } = &plan.operation else {
            return Err(Error::SnapshotConflict);
        };
        if !can(&guest, if *memory { "checkpoint" } else { "snapshot" }) {
            return Err(Error::UnavailableAction);
        }
        return Ok(Prepared {
            method: if *memory {
                "Async.VM.checkpoint"
            } else {
                "Async.VM.snapshot"
            },
            args: vec![json!(parent_ref), json!(name)],
        });
    }
    let (snapshot_ref, snapshot) = source(session, vm, plan).await?;
    match step {
        Step::Name | Step::Description => {
            let (name, description, previous_name, previous_description) = match &plan.operation {
                SnapshotWorkflow::Create {
                    name, description, ..
                } => (
                    name,
                    description,
                    name,
                    plan.created_description
                        .as_ref()
                        .ok_or(Error::SnapshotConflict)?,
                ),
                SnapshotWorkflow::Edit {
                    name,
                    description,
                    expected_name,
                    expected_description,
                    ..
                } => (name, description, expected_name, expected_description),
                _ => return Err(Error::SnapshotConflict),
            };
            let wanted_name = if *step == Step::Name {
                previous_name
            } else {
                name
            };
            if snapshot["name_label"] != *wanted_name
                || snapshot["name_description"] != *previous_description
            {
                return Err(Error::SnapshotConflict);
            }
            Ok(Prepared {
                method: if *step == Step::Name {
                    "VM.set_name_label"
                } else {
                    "VM.set_name_description"
                },
                args: vec![
                    json!(snapshot_ref),
                    json!(if *step == Step::Name {
                        name
                    } else {
                        description
                    }),
                ],
            })
        }
        Step::Shutdown => {
            if guest["power_state"] != "Running"
                || !can(&guest, "clean_shutdown")
                || !can(&snapshot, "revert")
            {
                return Err(Error::UnavailableAction);
            }
            session
                .call(
                    "VM.assert_operation_valid",
                    vec![json!(&parent_ref), json!("clean_shutdown")],
                )
                .await?;
            Ok(Prepared {
                method: "Async.VM.clean_shutdown",
                args: vec![json!(parent_ref)],
            })
        }
        Step::Revert => {
            if guest["power_state"] != "Halted" || !can(&snapshot, "revert") {
                return Err(Error::UnavailableAction);
            }
            session
                .call(
                    "VM.assert_operation_valid",
                    vec![json!(&snapshot_ref), json!("revert")],
                )
                .await?;
            Ok(Prepared {
                method: "Async.VM.revert",
                args: vec![json!(snapshot_ref)],
            })
        }
        Step::Start | Step::Resume => {
            let resume = *step == Step::Resume;
            let op = if resume { "resume" } else { "start" };
            if guest["parent"] != snapshot_ref
                || guest["power_state"] != if resume { "Suspended" } else { "Halted" }
                || !can(&guest, op)
            {
                return Err(Error::UnavailableAction);
            }
            if resume && memory(session, &guest).await? == plan.memory {
                return Err(Error::SnapshotConflict);
            }
            session
                .call(
                    "VM.assert_operation_valid",
                    vec![json!(&parent_ref), json!(op)],
                )
                .await?;
            Ok(Prepared {
                method: if resume {
                    "Async.VM.resume"
                } else {
                    "Async.VM.start"
                },
                args: vec![json!(parent_ref), json!(false), json!(false)],
            })
        }
        Step::Create => unreachable!(),
    }
}
pub(crate) async fn verify(
    session: &Session,
    vm: VmId,
    plan: &mut Plan,
    index: usize,
) -> Result<bool, Error> {
    let step = plan.steps[index].clone();
    if step.asynchronous() && !plan.evidence[index].completed {
        return Ok(false);
    }
    let (_, guest) = parent(session, vm).await?;
    if !idle(&guest) {
        return Ok(false);
    }
    if step == Step::Create {
        let reference = plan.evidence[index]
            .result
            .as_ref()
            .ok_or(Error::SnapshotConflict)?;
        let id = crate::xapi::verify_created_snapshot(session, vm, reference).await?;
        if plan
            .baseline
            .iter()
            .any(|previous| previous.uuid() == id.uuid())
            || plan.created.is_some_and(|old| old != id)
        {
            return Err(Error::SnapshotConflict);
        }
        let (_, record) = snapshots::snapshot_record(session, vm, id).await?;
        let SnapshotWorkflow::Create { name, memory, .. } = &plan.operation else {
            return Err(Error::SnapshotConflict);
        };
        if record["name_label"] != *name
            || record["power_state"] != if *memory { "Suspended" } else { "Halted" }
            || !idle(&record)
        {
            return Ok(false);
        }
        plan.created = Some(id);
        if plan.created_description.is_none() {
            plan.created_description = Some(
                record["name_description"]
                    .as_str()
                    .ok_or(Error::SnapshotConflict)?
                    .into(),
            )
        }
        return Ok(true);
    }
    let (reference, snapshot) = source(session, vm, plan).await?;
    Ok(match step {
        Step::Name => match &plan.operation {
            SnapshotWorkflow::Edit { name, .. } => snapshot["name_label"] == *name,
            _ => false,
        },
        Step::Description => match &plan.operation {
            SnapshotWorkflow::Create {
                name, description, ..
            }
            | SnapshotWorkflow::Edit {
                name, description, ..
            } => snapshot["name_label"] == *name && snapshot["name_description"] == *description,
            _ => false,
        },
        Step::Shutdown => guest["power_state"] == "Halted",
        Step::Revert => {
            guest["parent"] == reference
                && guest["power_state"] == snapshot["power_state"]
                && (snapshot["power_state"] != "Suspended"
                    || memory(session, &guest)
                        .await?
                        .is_some_and(|id| Some(id) != plan.memory))
        }
        Step::Start | Step::Resume => {
            guest["power_state"] == "Running" && guest["parent"] == reference
        }
        Step::Create => false,
    })
}
