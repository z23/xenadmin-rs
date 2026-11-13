use crate::{
    Error, Inventory, OperationId, PermissionRequirements, PowerState, TaskId, VmId, xapi::Session,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use xenadmin_xapi::enumerations::{domain_type, task_status_type, vm_operations, vm_power_state};
use xenadmin_xapi::records::{TaskRecord, VmRecord};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ForceRecoveryAction {
    Shutdown,
    Reboot,
}
impl ForceRecoveryAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::Shutdown => "Force shutdown with running tasks",
            Self::Reboot => "Force reboot with running tasks",
        }
    }
    pub(crate) fn wire(self) -> &'static str {
        match self {
            Self::Shutdown => "hard_shutdown",
            Self::Reboot => "hard_reboot",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RecoveryTask {
    pub task: TaskId,
    pub operation: String,
}
impl RecoveryTask {
    /// Whether the captured task is moving the VM between hosts or repositories. A forced
    /// action during one is admitted by the host, which does not always unwind the
    /// migration afterwards (lab finding 63), so a review says so.
    pub fn is_migration(&self) -> bool {
        matches!(
            self.operation.as_str(),
            vm_operations::MIGRATE_SEND | vm_operations::POOL_MIGRATE
        )
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryBootConfiguration {
    pub order: Option<String>,
    pub firmware: String,
    pub secure_boot: Option<bool>,
}
impl RecoveryBootConfiguration {
    pub fn from_inventory(inventory: &Inventory, vm: VmId) -> Result<Self, Error> {
        let v = inventory.vms.get(&vm).ok_or(Error::Stale)?;
        let d = inventory.details.vms.get(&vm).ok_or(Error::Stale)?;
        if v.is_template
            || v.is_control_domain
            || v.power_state != PowerState::Halted
            || !d.current_operations.is_empty()
            || d.domain_type.as_deref() != Some(domain_type::HVM)
            || d.boot_policy.as_deref() != Some("BIOS order")
        {
            return Err(Error::UnavailableAction);
        }
        let value = Self {
            order: d.boot_order.clone(),
            firmware: d.firmware.clone().ok_or(Error::UnavailableAction)?,
            secure_boot: d.secure_boot,
        };
        value.validate()?;
        Ok(value)
    }
    fn validate(&self) -> Result<(), Error> {
        if !matches!(self.firmware.as_str(), "bios" | "uefi")
            || self.order.as_ref().is_some_and(|v| {
                v.len() > 3
                    || !v.chars().all(|c| matches!(c, 'c' | 'd' | 'n'))
                    || v.chars().collect::<BTreeSet<_>>().len() != v.len()
            })
        {
            return Err(Error::UnavailableAction);
        }
        Ok(())
    }
    fn matches(&self, record: &Value, order: Option<&str>) -> bool {
        VmRecord::new(record).domain_type() == domain_type::HVM
            && VmRecord::new(record).hvm_boot_policy() == "BIOS order"
            && crate::guest_boot::firmware(record) == Some(self.firmware.as_str())
            && crate::guest_boot::policy(record) == self.secure_boot
            && VmRecord::new(record)
                .hvm_boot_params()
                .entries()
                .is_some_and(|map| map.get("order").map(Value::as_str) == order.map(Some))
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "recovery")]
pub enum VmRecoveryOperation {
    Force {
        action: ForceRecoveryAction,
        tasks: Vec<RecoveryTask>,
        /// The operator's acceptance, given in the review, that a migration among the
        /// captured tasks may be left unfinished on the hosts. The host admits a forced
        /// action during a migration and does not always end the migration afterwards
        /// (lab finding 63), so without this such a forced action is refused. It is set
        /// only when a captured task is a migration.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        interrupts_migration: bool,
    },
    Boot {
        configuration: RecoveryBootConfiguration,
    },
    RestoreBoot {
        source: OperationId,
        configuration: RecoveryBootConfiguration,
    },
}
impl VmRecoveryOperation {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Force { action, .. } => action.label(),
            Self::Boot { .. } => "Boot recovery media",
            Self::RestoreBoot { .. } => "Restore saved boot order",
        }
    }
    pub fn permission_requirements(&self) -> PermissionRequirements {
        match self {
            Self::Force { action, .. } => {
                PermissionRequirements::writes([format!("VM.{}", action.wire())])
            }
            Self::Boot { configuration } if configuration.order.as_deref() == Some("dn") => {
                PermissionRequirements::writes(["VM.start"])
            }
            Self::Boot { .. } => {
                PermissionRequirements::writes(["VM.set_HVM_boot_params", "VM.start"])
            }
            Self::RestoreBoot { .. } => PermissionRequirements::writes(["VM.set_HVM_boot_params"]),
        }
    }
    /// Whether this is a forced action that would interrupt a migration without the
    /// operator having accepted what that can leave behind.
    pub fn migration_unaccepted(&self) -> bool {
        matches!(self, Self::Force { tasks, interrupts_migration, .. }
            if !interrupts_migration && tasks.iter().any(RecoveryTask::is_migration))
    }
    /// An acceptance is recorded only where there is a migration to accept.
    fn acceptance_fits(&self) -> bool {
        !matches!(self, Self::Force { tasks, interrupts_migration, .. }
            if *interrupts_migration && !tasks.iter().any(RecoveryTask::is_migration))
    }
    pub fn tasks(inventory: &Inventory, vm: VmId) -> Result<Vec<RecoveryTask>, Error> {
        let v = inventory.vms.get(&vm).ok_or(Error::Stale)?;
        let d = inventory.details.vms.get(&vm).ok_or(Error::Stale)?;
        if v.is_template || v.is_control_domain || v.power_state != PowerState::Running {
            return Err(Error::UnavailableAction);
        }
        let mut tasks = d
            .current_operations
            .iter()
            .map(|o| {
                Ok(RecoveryTask {
                    task: o.task.ok_or(Error::Stale)?,
                    operation: o.operation.clone(),
                })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        tasks.sort();
        validate_tasks(&tasks)?;
        Ok(tasks)
    }
    pub fn unavailable_reason(&self, i: &Inventory, vm: VmId) -> Option<String> {
        let Some(v) = i.vms.get(&vm) else {
            return Some("VM is no longer present".into());
        };
        let Some(d) = i.details.vms.get(&vm) else {
            return Some("VM configuration is unavailable".into());
        };
        if v.is_template || v.is_control_domain {
            return Some("Recovery requires an ordinary VM".into());
        }
        match self {
            Self::Force { action, tasks, .. } => {
                if let Some(reason) = d.blocked_operations.get(action.wire()) {
                    return Some(format!("Administratively blocked: {reason}"));
                }
                if !Self::tasks(i, vm).is_ok_and(|actual| actual == *tasks) {
                    return Some(
                        "Running VM or captured task identities changed. Refresh this review."
                            .into(),
                    );
                }
            }
            Self::Boot { configuration } => {
                if !RecoveryBootConfiguration::from_inventory(i, vm)
                    .is_ok_and(|actual| actual == *configuration)
                {
                    return Some("Recovery boot requires a halted, idle HVM guest with unchanged BIOS/UEFI settings. PV guests and firmware conversion are unsupported.".into());
                }
                if !v.allowed_actions.contains(&crate::PowerAction::Start) {
                    return Some("The host does not currently advertise VM start.".into());
                }
            }
            Self::RestoreBoot { configuration, .. } => {
                if !d.current_operations.is_empty()
                    || d.boot_order.as_deref() != Some("dn")
                    || d.boot_policy.as_deref() != Some("BIOS order")
                    || d.domain_type.as_deref() != Some(domain_type::HVM)
                    || d.firmware.as_deref() != Some(configuration.firmware.as_str())
                    || d.secure_boot != configuration.secure_boot
                {
                    return Some(
                        "The saved recovery override is no longer unchanged, or the VM is busy."
                            .into(),
                    );
                }
            }
        }
        None
    }
    pub(crate) fn allows(&self, r: &Value) -> bool {
        normal(r)
            && match self {
                Self::Force { action, tasks, .. } => {
                    validate_tasks(tasks).is_ok()
                        && !self.migration_unaccepted()
                        && self.acceptance_fits()
                        && VmRecord::new(r).power_state() == vm_power_state::RUNNING
                        && VmRecord::new(r)
                            .blocked_operations()
                            .entries()
                            .is_some_and(|b| !b.contains_key(action.wire()))
                        && VmRecord::new(r)
                            .current_operations()
                            .entries()
                            .is_some_and(|o| o.len() == tasks.len())
                }
                Self::Boot { configuration } => {
                    configuration.validate().is_ok()
                        && configuration.matches(r, configuration.order.as_deref())
                        && VmRecord::new(r).power_state() == vm_power_state::HALTED
                        && idle(r)
                        && advertised(r, vm_operations::START)
                }
                Self::RestoreBoot { configuration, .. } => {
                    configuration.validate().is_ok()
                        && configuration.order.as_deref() != Some("dn")
                        && configuration.matches(r, Some("dn"))
                        && idle(r)
                }
            }
    }
    fn steps(&self) -> Vec<Step> {
        match self {
            Self::Force { action, .. } => vec![Step::Force(*action)],
            Self::Boot { configuration } if configuration.order.as_deref() == Some("dn") => {
                vec![Step::Start]
            }
            Self::Boot { .. } => vec![Step::Override, Step::Start, Step::Restore],
            Self::RestoreBoot { .. } => vec![Step::Restore],
        }
    }
    fn configuration(&self) -> Option<&RecoveryBootConfiguration> {
        match self {
            Self::Boot { configuration } | Self::RestoreBoot { configuration, .. } => {
                Some(configuration)
            }
            _ => None,
        }
    }
}
fn validate_tasks(tasks: &[RecoveryTask]) -> Result<(), Error> {
    if tasks.is_empty()
        || tasks.len() > 256
        || tasks.iter().any(|t| {
            t.operation.is_empty()
                || t.operation.len() > 80
                || !t
                    .operation
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c == '_')
        })
        || tasks.windows(2).any(|w| w[0] >= w[1])
        || tasks.iter().map(|t| t.task).collect::<BTreeSet<_>>().len() != tasks.len()
    {
        Err(Error::UnavailableAction)
    } else {
        Ok(())
    }
}
fn normal(r: &Value) -> bool {
    VmRecord::new(r).is_a_template() == false
        && VmRecord::new(r).is_a_snapshot() == false
        && VmRecord::new(r).is_control_domain() == false
}
fn idle(r: &Value) -> bool {
    VmRecord::new(r)
        .current_operations()
        .entries()
        .is_some_and(|o| o.is_empty())
}
fn advertised(r: &Value, op: &str) -> bool {
    VmRecord::new(r)
        .allowed_operations()
        .members()
        .is_some_and(|v| v.iter().any(|v| v == op))
        && VmRecord::new(r)
            .blocked_operations()
            .entries()
            .is_some_and(|b| !b.contains_key(op))
}
async fn guest(s: &Session, vm: VmId) -> Result<(String, Value), Error> {
    let (reference, r) = crate::snapshots::optional_record(s, "VM", &vm.to_string())
        .await?
        .ok_or(Error::Stale)?;
    if !normal(&r) {
        return Err(Error::Stale);
    }
    Ok((reference, r))
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Step {
    Override,
    Start,
    Restore,
    Force(ForceRecoveryAction),
}
impl Step {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Override => "Temporarily boot CD then network",
            Self::Start => "Start VM with recovery boot order",
            Self::Restore => "Restore original boot order",
            Self::Force(a) => a.label(),
        }
    }
    pub fn asynchronous(&self) -> bool {
        matches!(self, Self::Start | Self::Force(_))
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Evidence {
    pub task: Option<String>,
    pub task_id: Option<TaskId>,
    pub completed: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Plan {
    pub operation: VmRecoveryOperation,
    pub steps: Vec<Step>,
    pub evidence: Vec<Evidence>,
    pub baseline: Vec<TaskId>,
}
impl Plan {
    pub fn valid(&self, record: &crate::OperationRecord) -> bool {
        let crate::OperationAction::Recovery(operation) = &record.action else {
            return false;
        };
        let operation_valid = match operation {
            VmRecoveryOperation::Force { tasks, .. } => {
                validate_tasks(tasks).is_ok()
                    && !operation.migration_unaccepted()
                    && operation.acceptance_fits()
            }
            VmRecoveryOperation::Boot { configuration } => configuration.validate().is_ok(),
            VmRecoveryOperation::RestoreBoot {
                source,
                configuration,
            } => {
                *source != record.id
                    && configuration.order.as_deref() != Some("dn")
                    && configuration.validate().is_ok()
            }
        };
        operation_valid
            && self.operation == *operation
            && self.steps == operation.steps()
            && self.evidence.len() == self.steps.len()
            && record.steps.len() == self.steps.len()
            && self.baseline.len() <= 10000
            && self.baseline.windows(2).all(|w| w[0] < w[1])
            && self
                .evidence
                .iter()
                .zip(&self.steps)
                .zip(&record.steps)
                .enumerate()
                .all(|(index, ((e, s), public))| {
                    let evidence_valid = if s.asynchronous() {
                        e.task.as_ref().is_none_or(|t| {
                            t.starts_with("OpaqueRef:") && t != "OpaqueRef:NULL" && t.len() <= 1024
                        }) && e
                            .task_id
                            .is_none_or(|id| e.task.is_some() && !self.baseline.contains(&id))
                            && (!e.completed || e.task_id.is_some())
                    } else {
                        e.task.is_none() && e.task_id.is_none()
                    };
                    evidence_valid
                        && public.label == s.label()
                        && (public.state != crate::StepState::Verified || e.completed)
                        && (!(e.completed || e.task.is_some())
                            || self.evidence[..index].iter().all(|e| e.completed))
                })
            && (record.state != crate::OperationState::Succeeded
                || record
                    .steps
                    .iter()
                    .all(|s| s.state == crate::StepState::Verified)
                    && self.evidence.iter().all(|e| e.completed))
    }
    pub fn extends(&self, old: &Self) -> bool {
        self.operation == old.operation
            && self.steps == old.steps
            && self.baseline == old.baseline
            && self.evidence.len() == old.evidence.len()
            && self.evidence.iter().zip(&old.evidence).all(|(n, o)| {
                (!o.completed || n.completed)
                    && (o.task.is_none() || o.task == n.task)
                    && (o.task_id.is_none() || o.task_id == n.task_id)
            })
    }
}
pub(crate) async fn plan(
    s: &Session,
    vm: VmId,
    operation: &VmRecoveryOperation,
) -> Result<Plan, Error> {
    operation.permission_requirements().guard(s.permissions())?;
    if operation.migration_unaccepted() {
        return Err(Error::ForceDuringMigration);
    }
    let (_, r) = guest(s, vm).await?;
    if !operation.allows(&r) {
        return Err(Error::UnavailableAction);
    }
    let baseline = if matches!(operation, VmRecoveryOperation::RestoreBoot { .. }) {
        vec![]
    } else {
        let all = s.call("task.get_all_records", vec![]).await?;
        let map = all
            .as_object()
            .filter(|m| m.len() <= 10000)
            .ok_or(Error::Protocol(
                "Task identity baseline unavailable or too large",
            ))?;
        let mut ids = map
            .values()
            .map(|r| {
                TaskRecord::new(r)
                    .uuid()
                    .text()
                    .ok_or(Error::Stale)
                    .and_then(|v| TaskId::parse(v).map_err(|_| Error::Stale))
            })
            .collect::<Result<Vec<_>, Error>>()?;
        ids.sort();
        if ids.windows(2).any(|w| w[0] == w[1]) {
            return Err(Error::Stale);
        }
        ids
    };
    let steps = operation.steps();
    Ok(Plan {
        operation: operation.clone(),
        evidence: vec![Evidence::default(); steps.len()],
        steps,
        baseline,
    })
}
pub(crate) struct Prepared {
    method: String,
    params: Vec<Value>,
}
impl Prepared {
    pub async fn send(self, s: &Session) -> Result<Value, Error> {
        s.call(&self.method, self.params).await
    }
}
pub(crate) async fn prepare(
    s: &Session,
    vm: VmId,
    plan: &Plan,
    index: usize,
) -> Result<Prepared, Error> {
    let (reference, r) = guest(s, vm).await?;
    let step = &plan.steps[index];
    if let Step::Force(action) = step {
        if !plan.operation.allows(&r) {
            return Err(Error::UnavailableAction);
        }
        let VmRecoveryOperation::Force { tasks, .. } = &plan.operation else {
            return Err(Error::Journal);
        };
        let raw = VmRecord::new(&r)
            .current_operations()
            .entries()
            .ok_or(Error::Stale)?;
        let mut actual = Vec::new();
        for (reference, op) in raw {
            let task = s.call("task.get_record", vec![json!(reference)]).await?;
            actual.push(RecoveryTask {
                task: TaskId::parse(TaskRecord::new(&task).uuid().text().ok_or(Error::Stale)?)
                    .map_err(|_| Error::Stale)?,
                operation: op.as_str().ok_or(Error::Stale)?.into(),
            });
        }
        actual.sort();
        if actual != *tasks {
            return Err(Error::Stale);
        }
        let (_, fresh) = guest(s, vm).await?;
        if !plan.operation.allows(&fresh)
            || VmRecord::new(&fresh).current_operations() != VmRecord::new(&r).current_operations()
        {
            return Err(Error::Stale);
        }
        s.call(
            "VM.assert_operation_valid",
            vec![json!(&reference), json!(action.wire())],
        )
        .await?;
        return Ok(Prepared {
            method: format!("Async.VM.{}", action.wire()),
            params: vec![json!(reference)],
        });
    }
    let config = plan.operation.configuration().ok_or(Error::Journal)?;
    let expected = if matches!(step, Step::Override) {
        config.order.as_deref()
    } else {
        Some("dn")
    };
    if !config.matches(&r, expected) || !idle(&r) {
        return Err(Error::Stale);
    }
    if matches!(step, Step::Start | Step::Override)
        && VmRecord::new(&r).power_state() != vm_power_state::HALTED
    {
        return Err(Error::Stale);
    }
    match step {
        Step::Start => {
            if !advertised(&r, vm_operations::START) {
                return Err(Error::UnavailableAction);
            }
            s.call(
                "VM.assert_operation_valid",
                vec![json!(&reference), json!(vm_operations::START)],
            )
            .await?;
            Ok(Prepared {
                method: "Async.VM.start".into(),
                params: vec![json!(reference), json!(false), json!(false)],
            })
        }
        Step::Override | Step::Restore => {
            let mut params = VmRecord::new(&r)
                .hvm_boot_params()
                .entries()
                .cloned()
                .ok_or(Error::Stale)?;
            let order = if *step == Step::Override {
                Some("dn")
            } else {
                config.order.as_deref()
            };
            if let Some(order) = order {
                params.insert("order".into(), json!(order));
            } else {
                params.remove("order");
            }
            Ok(Prepared {
                method: "VM.set_HVM_boot_params".into(),
                params: vec![json!(reference), json!(params)],
            })
        }
        _ => Err(Error::Journal),
    }
}
pub(crate) async fn task(s: &Session, plan: &mut Plan, index: usize) -> Result<Value, Error> {
    let e = &mut plan.evidence[index];
    let r = s
        .call(
            "task.get_record",
            vec![json!(e.task.as_ref().ok_or(Error::Journal)?)],
        )
        .await?;
    let id = TaskId::parse(TaskRecord::new(&r).uuid().text().ok_or(Error::Stale)?)
        .map_err(|_| Error::Stale)?;
    if plan.baseline.contains(&id) || e.task_id.is_some_and(|old| old != id) {
        return Err(Error::Stale);
    }
    e.task_id = Some(id);
    if TaskRecord::new(&r).status() == task_status_type::SUCCESS {
        e.completed = true
    }
    Ok(r)
}
pub(crate) async fn verify(
    s: &Session,
    vm: VmId,
    plan: &Plan,
    index: usize,
) -> Result<bool, Error> {
    if !plan.evidence[index].completed {
        return Ok(false);
    }
    let (_, r) = guest(s, vm).await?;
    Ok(match &plan.steps[index] {
        Step::Force(a) => {
            VmRecord::new(&r).power_state()
                == if *a == ForceRecoveryAction::Shutdown {
                    vm_power_state::HALTED
                } else {
                    vm_power_state::RUNNING
                }
        }
        step => {
            let c = plan.operation.configuration().ok_or(Error::Journal)?;
            let order = if *step == Step::Restore {
                c.order.as_deref()
            } else {
                Some("dn")
            };
            c.matches(&r, order)
                && (*step != Step::Start
                    || VmRecord::new(&r).power_state() == vm_power_state::RUNNING)
        }
    })
}
