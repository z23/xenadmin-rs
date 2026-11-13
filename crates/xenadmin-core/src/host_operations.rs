use super::{ResourceAction, ResourceOperationRecord, ResourceTarget};
use crate::{
    Error, HostOperation, MemoryLimits, VmId,
    host_types::{same_vms, valid_epoch},
    xapi::Session,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use xenadmin_xapi::enumerations::{host_allowed_operations, task_status_type, vm_power_state};
use xenadmin_xapi::records::{HostRecord, PbdRecord, PoolRecord, TaskRecord, VmRecord};
use xenadmin_xapi::{Reads, kind};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Kind {
    DataSource(crate::HostRecordingAction),
    Disable,
    Evacuate,
    MarkMaintenance(bool),
    Enable,
    Reboot,
    Shutdown,
    Restart,
    PowerOn,
    PowerMode,
    Memory,
    Logging,
    ReconfigureLogging,
    Multipath,
    MultipathHandle,
    PoolAutostart,
    Forget,
}
impl Kind {
    fn method(&self) -> &'static str {
        match self {
            Self::DataSource(action) => action.method(),
            Self::Disable => "Async.host.disable",
            Self::Evacuate => "Async.host.evacuate",
            Self::MarkMaintenance(_) | Self::MultipathHandle => "host.set_other_config",
            Self::Enable => "Async.host.enable",
            Self::Reboot => "Async.host.reboot",
            Self::Shutdown => "Async.host.shutdown",
            Self::Restart => "Async.host.restart_agent",
            Self::PowerOn => "Async.host.power_on",
            Self::PowerMode => "host.set_power_on_mode",
            Self::Memory => "VM.set_memory",
            Self::Logging => "host.set_logging",
            Self::ReconfigureLogging => "host.syslog_reconfigure",
            Self::Multipath => "host.set_multipathing",
            Self::PoolAutostart => "pool.set_other_config",
            Self::Forget => "Async.host.destroy",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::DataSource(action) => action.label(),
            Self::Disable => "Disable scheduling",
            Self::Evacuate => "Evacuate resident guests",
            Self::MarkMaintenance(true) => "Mark host in maintenance",
            Self::MarkMaintenance(false) => "Clear maintenance marker",
            Self::Enable => "Enable scheduling",
            Self::Reboot => "Reboot host",
            Self::Shutdown => "Request host shutdown",
            Self::Restart => "Restart toolstack",
            Self::PowerOn => "Power on host",
            Self::PowerMode => "Configure power-on mode",
            Self::Memory => "Set control-domain memory",
            Self::Logging => "Set remote logging destination",
            Self::ReconfigureLogging => "Apply logging configuration",
            Self::Multipath => "Set multipathing",
            Self::MultipathHandle => "Set multipath handler",
            Self::PoolAutostart => "Set pool VM autostart",
            Self::Forget => "Forget dead host record",
        }
    }
    pub fn asynchronous(&self) -> bool {
        matches!(
            self,
            Self::Disable
                | Self::Evacuate
                | Self::Enable
                | Self::Reboot
                | Self::Shutdown
                | Self::Restart
                | Self::PowerOn
                | Self::Forget
        )
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Step {
    pub kind: Kind,
    pub task: Option<String>,
    pub accepted: bool,
    pub task_succeeded: bool,
    /// Earlier tasks of this step that the host answered with a definite refusal, oldest
    /// first, each superseded by a repeat of the same call. A stored task reference is
    /// evidence and is never dropped: a repeat moves it here before taking its place.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub refused: Vec<String>,
}
impl Step {
    /// More repeats than any bounded wait produces; a plan beyond it is refused.
    pub const MAX_REFUSED: usize = 512;

    /// Whether `new` is this step after its task was refused and the call repeated: the
    /// task that was current is now the newest refused one, nothing recorded before is
    /// lost, and a task that succeeded is never superseded.
    pub fn superseded_by(&self, new: &Self) -> bool {
        match (&self.task, &new.task) {
            (Some(old), Some(current)) => {
                !self.task_succeeded
                    && old != current
                    && new.refused.len() == self.refused.len() + 1
                    && new.refused.len() <= Self::MAX_REFUSED
                    && new.refused.starts_with(&self.refused)
                    && new.refused.last() == Some(old)
                    && !new.refused.contains(current)
            }
            _ => false,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Plan {
    pub steps: Vec<Step>,
}
impl Plan {
    pub fn accepts(&self, new: &Self) -> bool {
        self.steps.len() == new.steps.len()
            && self.steps.iter().zip(&new.steps).all(|(old, new)| {
                old.kind == new.kind
                    && (old.superseded_by(new)
                        || (old
                            .task
                            .as_ref()
                            .is_none_or(|task| Some(task) == new.task.as_ref())
                            && old.refused == new.refused))
                    && (!old.accepted || new.accepted)
                    && (!old.task_succeeded || new.task_succeeded)
            })
    }
}
fn operation(record: &ResourceOperationRecord) -> Result<&HostOperation, Error> {
    if let ResourceAction::Host(operation) = &record.action {
        Ok(operation)
    } else {
        Err(Error::InvalidResourceCommand)
    }
}
fn require(yes: bool, reason: &'static str) -> Result<(), Error> {
    if yes {
        Ok(())
    } else {
        Err(Error::HostPrerequisite(reason))
    }
}
fn reference<'a>(value: impl Reads<'a, kind::Reference>) -> Result<String, Error> {
    let value = value.raw();
    super::xapi::reply_reference(value)
}
fn integer<'a>(value: impl Reads<'a, kind::Whole>) -> Option<u64> {
    let value = value.raw();
    value.as_u64().or_else(|| value.as_str()?.parse().ok())
}
fn map_bool(record: &Value, key: &str) -> Option<bool> {
    match record["other_config"].as_object()?.get(key) {
        None => Some(false),
        Some(value) => match value.as_str()? {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        },
    }
}
fn destination(record: &Value) -> Result<Option<String>, Error> {
    let map = HostRecord::new(record)
        .logging()
        .entries()
        .ok_or(Error::HostPrerequisite(
            "Host logging configuration is unavailable",
        ))?;
    match map.get("syslog_destination") {
        None => Ok(None),
        Some(value) => value
            .as_str()
            .map(|s| (!s.is_empty()).then(|| s.into()))
            .ok_or(Error::Protocol("invalid syslog destination")),
    }
}
fn idle(record: &Value) -> Result<(), Error> {
    require(
        record["current_operations"]
            .as_object()
            .is_some_and(|ops| ops.is_empty()),
        "The host or VM has running operations, or their availability is unknown",
    )
}
fn allowed(record: &Value, operation: &str) -> Result<(), Error> {
    require(
        record["allowed_operations"]
            .as_array()
            .is_some_and(|ops| ops.iter().any(|op| op == operation)),
        "The host no longer advertises the reviewed operation",
    )
}
fn memory(record: &Value) -> Result<MemoryLimits, Error> {
    Ok(MemoryLimits {
        static_min: integer(VmRecord::new(record).memory_static_min())
            .ok_or(Error::Protocol("control-domain memory limits unavailable"))?,
        dynamic_min: integer(VmRecord::new(record).memory_dynamic_min())
            .ok_or(Error::Protocol("control-domain memory limits unavailable"))?,
        dynamic_max: integer(VmRecord::new(record).memory_dynamic_max())
            .ok_or(Error::Protocol("control-domain memory limits unavailable"))?,
        static_max: integer(VmRecord::new(record).memory_static_max())
            .ok_or(Error::Protocol("control-domain memory limits unavailable"))?,
    })
}
async fn read(session: &Session, class: &str, uuid: uuid::Uuid) -> Result<(String, Value), Error> {
    let object = reference(
        &session
            .call(
                &format!("{class}.get_by_uuid"),
                vec![json!(uuid.to_string())],
            )
            .await?,
    )?;
    let record = session
        .call(&format!("{class}.get_record"), vec![json!(&object)])
        .await?;
    require(
        record["uuid"].as_str() == Some(uuid.to_string().as_str()),
        "Object identity changed since review",
    )?;
    Ok((object, record))
}
async fn host(
    session: &Session,
    record: &ResourceOperationRecord,
) -> Result<(String, Value, Value), Error> {
    let (_, pool) = read(session, "pool", record.pool.uuid()).await?;
    let ResourceTarget::Host(id) = record.target else {
        return Err(Error::InvalidResourceCommand);
    };
    let (reference, host) = read(session, "host", id.uuid()).await?;
    Ok((reference, host, pool))
}
async fn metrics(session: &Session, host: &Value) -> Result<Value, Error> {
    let metrics = reference(HostRecord::new(host).metrics())?;
    session
        .call("host_metrics.get_record", vec![json!(metrics)])
        .await
}
async fn live(session: &Session, host: &Value, expected: bool) -> Result<(), Error> {
    require(
        metrics(session, host).await?["live"] == expected,
        "The host's fresh liveness does not match the reviewed operation",
    )
}
async fn residents(session: &Session, host_ref: &str, host: &Value) -> Result<Vec<VmId>, Error> {
    let values = HostRecord::new(host)
        .resident_vms()
        .members()
        .filter(|v| v.len() <= 10_000)
        .ok_or(Error::HostPrerequisite(
            "Resident VM membership is unavailable",
        ))?;
    let mut result = Vec::new();
    for value in values {
        let vm_ref = reference(value)?;
        let vm = session.call("VM.get_record", vec![json!(&vm_ref)]).await?;
        require(
            VmRecord::new(&vm).resident_on() == host_ref,
            "Resident VM location changed",
        )?;
        let control = VmRecord::new(&vm)
            .is_control_domain()
            .get()
            .ok_or(Error::Protocol("resident VM type unavailable"))?;
        require(
            matches!(
                VmRecord::new(&vm).power_state().name(),
                Some(
                    vm_power_state::RUNNING
                        | vm_power_state::PAUSED
                        | vm_power_state::HALTED
                        | vm_power_state::SUSPENDED
                )
            ),
            "Resident VM power state is unavailable",
        )?;
        if !control
            && matches!(
                VmRecord::new(&vm).power_state().name(),
                Some(vm_power_state::RUNNING | vm_power_state::PAUSED)
            )
        {
            idle(&vm)?;
            let id = VmRecord::new(&vm)
                .uuid()
                .text()
                .and_then(|v| VmId::parse(v).ok())
                .ok_or(Error::Protocol("resident VM UUID unavailable"))?;
            require(
                !result.contains(&id),
                "Resident VM membership is inconsistent",
            )?;
            result.push(id);
        }
    }
    Ok(result)
}
async fn evacuation(
    session: &Session,
    host_ref: &str,
    host: &Value,
    expected: &[VmId],
) -> Result<(), Error> {
    require(
        same_vms(&residents(session, host_ref, host).await?, expected),
        "Resident VMs changed since review; inspect the host before retrying",
    )?;
    let blockers = session
        .call(
            "host.get_vms_which_prevent_evacuation",
            vec![json!(host_ref)],
        )
        .await?;
    let blockers = blockers
        .as_object()
        .ok_or(Error::Protocol("evacuation blocker report unavailable"))?;
    if !blockers.is_empty() {
        // Name the VMs and the host's reason so the operator can act on them.
        let mut names = Vec::new();
        for vm_ref in blockers.keys().take(5) {
            let name = session
                .call("VM.get_name_label", vec![json!(vm_ref)])
                .await
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_else(|| "unnamed VM".into());
            names.push(name);
        }
        return Err(Error::EvacuationBlocked(blocker_summary(blockers, &names)));
    }
    for id in expected {
        let (vm_ref, _) = read(session, "VM", id.uuid()).await?;
        let possible = session
            .call("VM.get_possible_hosts", vec![json!(vm_ref)])
            .await?;
        let possible = possible
            .as_array()
            .filter(|hosts| hosts.len() <= 1024)
            .ok_or(Error::Protocol("evacuation destinations unavailable"))?;
        let mut suitable = false;
        for candidate in possible {
            let candidate = reference(candidate)?;
            if candidate == host_ref {
                continue;
            }
            let record = session
                .call("host.get_record", vec![json!(candidate)])
                .await?;
            if HostRecord::new(&record).enabled() == true
                && HostRecord::new(&record)
                    .uuid()
                    .text()
                    .is_some_and(|uuid| uuid::Uuid::parse_str(uuid).is_ok())
                && metrics(session, &record).await?["live"] == true
            {
                suitable = true;
                break;
            }
        }
        require(
            suitable,
            "A resident VM has no live enabled destination; no automatic shutdown or HA reduction will be attempted",
        )?;
    }
    session
        .call("host.assert_can_evacuate", vec![json!(host_ref)])
        .await?;
    Ok(())
}
async fn empty_disabled(session: &Session, host_ref: &str, host: &Value) -> Result<(), Error> {
    require(
        host["enabled"] == false,
        "Disable host scheduling before this operation",
    )?;
    require(
        residents(session, host_ref, host).await?.is_empty(),
        "Evacuate all running or paused guests before this operation",
    )
}
async fn safe_coordinator(session: &Session, reference: &str, pool: &Value) -> Result<(), Error> {
    if pool["master"] == reference {
        let hosts = session.call("host.get_all", vec![]).await?;
        require(
            hosts.as_array().is_some_and(|hosts| hosts.len() == 1),
            "Hand over the pool coordinator separately before shutting down or rebooting it",
        )?;
    }
    Ok(())
}
fn epoch(host: &Value, key: &str, expected: &str) -> Result<(), Error> {
    require(
        host["other_config"][key].as_str() == Some(expected) && valid_epoch(expected),
        "The host restarted after the view opened, or its start marker is unavailable",
    )
}
async fn detached_storage(
    session: &Session,
    reference_host: &str,
    host: &Value,
) -> Result<(), Error> {
    let pbds = host["PBDs"]
        .as_array()
        .filter(|v| v.len() <= 10_000)
        .ok_or(Error::HostPrerequisite(
            "Host storage attachments are unavailable",
        ))?;
    for pbd in pbds {
        let record = session
            .call("PBD.get_record", vec![json!(reference(pbd)?)])
            .await?;
        require(
            PbdRecord::new(&record).host() == reference_host
                && PbdRecord::new(&record).currently_attached() == false,
            "Detach the host's storage connections before changing multipathing",
        )?;
    }
    Ok(())
}
fn map_argument(
    record: &Value,
    field: &str,
    key: &str,
    value: Option<Value>,
) -> Result<Value, Error> {
    let mut map = record[field]
        .as_object()
        .filter(|map| map.values().all(Value::is_string))
        .cloned()
        .ok_or(Error::Protocol("host configuration map unavailable"))?;
    if let Some(value) = value {
        map.insert(key.into(), value);
    } else {
        map.remove(key);
    }
    Ok(Value::Object(map))
}

pub(crate) fn requirements(op: &HostOperation) -> crate::PermissionRequirements {
    crate::PermissionRequirements::writes(kinds(op).iter().map(Kind::method))
}
fn kinds(op: &HostOperation) -> Vec<Kind> {
    match op {
        HostOperation::DataSource { action, .. } => vec![Kind::DataSource(*action)],
        HostOperation::EnterMaintenance {
            expected_enabled,
            resident_vms,
        } => {
            let mut kinds = Vec::new();
            if *expected_enabled {
                kinds.push(Kind::Disable);
            }
            if !resident_vms.is_empty() {
                kinds.push(Kind::Evacuate);
            }
            kinds.push(Kind::MarkMaintenance(true));
            kinds
        }
        HostOperation::Evacuate { .. } => vec![Kind::Evacuate],
        HostOperation::ExitMaintenance => vec![Kind::Enable, Kind::MarkMaintenance(false)],
        HostOperation::Reboot { .. } => vec![Kind::Reboot],
        HostOperation::Shutdown { .. } => vec![Kind::Shutdown],
        HostOperation::RestartToolstack { .. } => vec![Kind::Restart],
        HostOperation::PowerOn => vec![Kind::PowerOn],
        HostOperation::ConfigurePowerOn { .. } => vec![Kind::PowerMode],
        HostOperation::SetDom0Memory { .. } => vec![Kind::Memory],
        HostOperation::SetSyslog { .. } => vec![Kind::Logging, Kind::ReconfigureLogging],
        HostOperation::SetMultipath { .. } => vec![Kind::Multipath, Kind::MultipathHandle],
        HostOperation::SetPoolAutostart { .. } => vec![Kind::PoolAutostart],
        HostOperation::ForgetDeadHost { .. } => vec![Kind::Forget],
    }
}
pub(crate) async fn prepare(
    session: &Session,
    record: &ResourceOperationRecord,
) -> Result<Plan, Error> {
    let op = operation(record)?;
    op.validate(record.target)?;
    let kinds = kinds(op);
    // Check migration feasibility before disabling scheduling; every dispatch checks again.
    if let HostOperation::EnterMaintenance { resident_vms, .. } = op {
        let (reference, host, _) = host(session, record).await?;
        live(session, &host, true).await?;
        idle(&host)?;
        evacuation(session, &reference, &host, resident_vms).await?;
    }
    let plan = Plan {
        steps: kinds
            .into_iter()
            .map(|kind| Step {
                kind,
                task: None,
                accepted: false,
                task_succeeded: false,
                refused: Vec::new(),
            })
            .collect(),
    };
    arguments(session, record, &plan.steps[0].kind).await?;
    Ok(plan)
}
pub(crate) async fn arguments(
    session: &Session,
    record: &ResourceOperationRecord,
    kind: &Kind,
) -> Result<(&'static str, Vec<Value>), Error> {
    let op = operation(record)?;
    if let HostOperation::SetPoolAutostart { expected, enabled } = op {
        require(*kind == Kind::PoolAutostart, "Invalid pool autostart step")?;
        let (reference, pool) = read(session, "pool", record.pool.uuid()).await?;
        require(
            map_bool(&pool, "auto_poweron") == Some(*expected),
            "Pool VM autostart changed since review",
        )?;
        return Ok((
            kind.method(),
            vec![
                json!(reference),
                map_argument(
                    &pool,
                    "other_config",
                    "auto_poweron",
                    Some(json!(enabled.to_string())),
                )?,
            ],
        ));
    }
    let (reference, host, pool) = host(session, record).await?;
    idle(&host)?;
    let args = match kind {
        Kind::DataSource(action) => {
            let HostOperation::DataSource {
                expected,
                action: reviewed,
            } = op
            else {
                return Err(Error::InvalidResourceCommand);
            };
            require(
                action == reviewed,
                "Host monitoring step does not match the review",
            )?;
            live(session, &host, true).await?;
            crate::host_monitoring::check(session, &reference, expected, *action, false).await?;
            let (current_reference, current_host, _) = self::host(session, record).await?;
            require(
                current_reference == reference,
                "Host identity changed during data-source review",
            )?;
            idle(&current_host)?;
            live(session, &current_host, true).await?;
            (kind.method(), vec![json!(reference), json!(expected.name)])
        }
        Kind::Disable => {
            let HostOperation::EnterMaintenance {
                expected_enabled,
                resident_vms,
            } = op
            else {
                return Err(Error::InvalidResourceCommand);
            };
            require(
                *expected_enabled
                    && HostRecord::new(&host).enabled() == true
                    && map_bool(&host, "MAINTENANCE_MODE") == Some(false),
                "Host scheduling or maintenance state changed",
            )?;
            live(session, &host, true).await?;
            evacuation(session, &reference, &host, resident_vms).await?;
            (kind.method(), vec![json!(&reference)])
        }
        Kind::Evacuate => {
            let expected = match op {
                HostOperation::Evacuate { resident_vms }
                | HostOperation::EnterMaintenance { resident_vms, .. } => resident_vms,
                _ => return Err(Error::InvalidResourceCommand),
            };
            require(
                HostRecord::new(&host).enabled() == false,
                "Disable scheduling before evacuating",
            )?;
            live(session, &host, true).await?;
            allowed(&host, host_allowed_operations::EVACUATE)?;
            evacuation(session, &reference, &host, expected).await?;
            (kind.method(), vec![json!(&reference)])
        }
        Kind::MarkMaintenance(value) => {
            live(session, &host, true).await?;
            if *value {
                empty_disabled(session, &reference, &host).await?;
                require(
                    map_bool(&host, "MAINTENANCE_MODE") == Some(false),
                    "Maintenance marker changed",
                )?;
            } else {
                require(
                    HostRecord::new(&host).enabled() == true
                        && map_bool(&host, "MAINTENANCE_MODE") == Some(true),
                    "Host scheduling or maintenance state changed",
                )?;
            }
            (
                kind.method(),
                vec![
                    json!(&reference),
                    map_argument(
                        &host,
                        "other_config",
                        "MAINTENANCE_MODE",
                        value.then(|| json!("true")),
                    )?,
                ],
            )
        }
        Kind::Enable => {
            // A toolstack restart re-enables a host while the client's maintenance marker
            // stays on it; exiting maintenance is still the way to clear that marker, so the
            // host may already be enabled here and the enable call is then a harmless repeat.
            require(
                matches!(op, HostOperation::ExitMaintenance)
                    && map_bool(&host, "MAINTENANCE_MODE") == Some(true),
                "The host no longer carries the maintenance marker",
            )?;
            live(session, &host, true).await?;
            if HostRecord::new(&host).enabled() != true {
                allowed(&host, host_allowed_operations::ENABLE)?;
            }
            (kind.method(), vec![json!(&reference)])
        }
        Kind::Reboot | Kind::Shutdown => {
            let expected = match op {
                HostOperation::Reboot { expected_boot_time }
                | HostOperation::Shutdown { expected_boot_time } => expected_boot_time,
                _ => return Err(Error::InvalidResourceCommand),
            };
            epoch(&host, "boot_time", expected)?;
            live(session, &host, true).await?;
            empty_disabled(session, &reference, &host).await?;
            safe_coordinator(session, &reference, &pool).await?;
            allowed(
                &host,
                if *kind == Kind::Reboot {
                    host_allowed_operations::REBOOT
                } else {
                    host_allowed_operations::SHUTDOWN
                },
            )?;
            (kind.method(), vec![json!(&reference)])
        }
        Kind::Restart => {
            require(
                PoolRecord::new(&pool).ha_enabled() == false,
                "Disable pool HA separately before restarting the toolstack; active HA may fence the host",
            )?;
            let HostOperation::RestartToolstack {
                expected_agent_start_time,
            } = op
            else {
                return Err(Error::InvalidResourceCommand);
            };
            epoch(&host, "agent_start_time", expected_agent_start_time)?;
            live(session, &host, true).await?;
            let tasks = session.call("task.get_all_records", vec![]).await?;
            require(
                tasks.as_object().is_some_and(|tasks| {
                    tasks.len() <= 10_000
                        && tasks.values().all(|task| {
                            matches!(
                                TaskRecord::new(task).status().name(),
                                Some(
                                    task_status_type::SUCCESS
                                        | task_status_type::FAILURE
                                        | task_status_type::CANCELLED
                                )
                            )
                        })
                }),
                "Finish pending pool tasks before restarting the toolstack. An open VM or host console counts as one",
            )?;
            (kind.method(), vec![json!(&reference)])
        }
        Kind::PowerOn => {
            live(session, &host, false).await?;
            allowed(&host, host_allowed_operations::POWER_ON)?;
            require(
                HostRecord::new(&host)
                    .power_on_mode()
                    .text()
                    .is_some_and(|v| !v.is_empty()),
                "Configure host power-on first",
            )?;
            (kind.method(), vec![json!(&reference)])
        }
        Kind::PowerMode => {
            let HostOperation::ConfigurePowerOn {
                expected_mode,
                mode,
            } = op
            else {
                return Err(Error::InvalidResourceCommand);
            };
            live(session, &host, true).await?;
            require(
                HostRecord::new(&host).power_on_mode() == *expected_mode
                    && HostRecord::new(&host)
                        .power_on_config()
                        .entries()
                        .is_some_and(|m| m.is_empty()),
                "Power-on mode changed or contains BMC configuration that this editor does not replace",
            )?;
            (
                kind.method(),
                vec![json!(&reference), json!(mode.as_str()), json!({})],
            )
        }
        Kind::Memory => {
            let HostOperation::SetDom0Memory {
                control_domain,
                expected,
                memory: desired,
            } = op
            else {
                return Err(Error::InvalidResourceCommand);
            };
            live(session, &host, true).await?;
            let (vm_ref, vm) = read(session, "VM", control_domain.uuid()).await?;
            require(
                HostRecord::new(&host).control_domain() == vm_ref
                    && VmRecord::new(&vm).is_control_domain() == true
                    && VmRecord::new(&vm).resident_on() == reference,
                "Control-domain identity changed",
            )?;
            idle(&vm)?;
            require(
                memory(&vm)? == *expected,
                "Control-domain memory limits changed since review",
            )?;
            let total = integer(&metrics(session, &host).await?["memory_total"])
                .ok_or(Error::HostPrerequisite("Host total memory is unavailable"))?;
            require(
                *desired <= total,
                "Control-domain memory exceeds host physical memory",
            )?;
            (
                kind.method(),
                vec![json!(vm_ref), json!(desired.to_string())],
            )
        }
        Kind::Logging | Kind::ReconfigureLogging => {
            let HostOperation::SetSyslog {
                expected_destination,
                destination: desired,
            } = op
            else {
                return Err(Error::InvalidResourceCommand);
            };
            live(session, &host, true).await?;
            require(
                destination(&host)?
                    == if *kind == Kind::Logging {
                        expected_destination.clone()
                    } else {
                        desired.clone()
                    },
                "Remote logging destination changed",
            )?;
            if *kind == Kind::Logging {
                (
                    kind.method(),
                    vec![
                        json!(&reference),
                        map_argument(
                            &host,
                            "logging",
                            "syslog_destination",
                            desired.as_ref().map(|v| json!(v)),
                        )?,
                    ],
                )
            } else {
                (kind.method(), vec![json!(&reference)])
            }
        }
        Kind::Multipath | Kind::MultipathHandle => {
            let HostOperation::SetMultipath { expected, enabled } = op else {
                return Err(Error::InvalidResourceCommand);
            };
            live(session, &host, true).await?;
            empty_disabled(session, &reference, &host).await?;
            require(
                map_bool(&host, "MAINTENANCE_MODE") == Some(true),
                "Enter maintenance before changing multipathing",
            )?;
            detached_storage(session, &reference, &host).await?;
            require(
                HostRecord::new(&host).multipathing()
                    == if *kind == Kind::Multipath {
                        *expected
                    } else {
                        *enabled
                    },
                "Multipath configuration changed",
            )?;
            let handler = HostRecord::new(&host)
                .other_config()
                .entries()
                .and_then(|m| m.get("multipathhandle"));
            require(
                handler.is_none_or(|v| v == "dmp"),
                "Custom multipath handlers require separate configuration",
            )?;
            if *kind == Kind::Multipath {
                (kind.method(), vec![json!(&reference), json!(enabled)])
            } else {
                (
                    kind.method(),
                    vec![
                        json!(&reference),
                        map_argument(
                            &host,
                            "other_config",
                            "multipathhandle",
                            enabled.then(|| json!("dmp")),
                        )?,
                    ],
                )
            }
        }
        Kind::Forget => {
            require(
                matches!(
                    op,
                    HostOperation::ForgetDeadHost {
                        verified_power_off: true
                    }
                ),
                "Independently verify that the host is powered off before forgetting it",
            )?;
            live(session, &host, false).await?;
            empty_disabled(session, &reference, &host).await?;
            require(
                PoolRecord::new(&pool).master() != reference
                    && PoolRecord::new(&pool).ha_enabled() == false,
                "The coordinator cannot be forgotten and HA must be disabled",
            )?;
            (kind.method(), vec![json!(&reference)])
        }
        Kind::PoolAutostart => return Err(Error::InvalidResourceCommand),
    };
    Ok(args)
}
pub(crate) async fn verify(
    session: &Session,
    record: &ResourceOperationRecord,
    step: &Step,
) -> Result<(), Error> {
    let op = operation(record)?;
    if step.kind.asynchronous() && !matches!(step.kind, Kind::Reboot | Kind::Restart) {
        require(
            step.task_succeeded,
            "Host task completion is unresolved; matching state alone does not establish that the dispatched task finished",
        )?;
    }
    if step.kind == Kind::PoolAutostart {
        let HostOperation::SetPoolAutostart { enabled, .. } = op else {
            return Err(Error::InvalidResourceCommand);
        };
        let (_, pool) = read(session, "pool", record.pool.uuid()).await?;
        return require(
            map_bool(&pool, "auto_poweron") == Some(*enabled),
            "Pool VM autostart has not been verified",
        );
    }
    if step.kind == Kind::Forget {
        read(session, "pool", record.pool.uuid()).await?;
        let ResourceTarget::Host(id) = record.target else {
            return Err(Error::InvalidResourceCommand);
        };
        return match read(session, "host", id.uuid()).await {
            Err(Error::Api(code)) if code == "UUID_INVALID" => Ok(()),
            Ok(_) => Err(Error::HostPrerequisite("The host record still exists")),
            Err(error) => Err(error),
        };
    }
    let (reference, host, _) = host(session, record).await?;
    match &step.kind {
        Kind::DataSource(action) => {
            let HostOperation::DataSource {
                expected,
                action: reviewed,
            } = op
            else {
                return Err(Error::InvalidResourceCommand);
            };
            require(
                action == reviewed && step.accepted,
                "Host monitoring completion reply was lost; current source state does not prove that the write completed",
            )?;
            crate::host_monitoring::check(session, &reference, expected, *action, true).await?;
            let (current_reference, _, _) = self::host(session, record).await?;
            require(
                current_reference == reference,
                "Host identity changed during recording verification",
            )
        }
        Kind::Disable => require(
            HostRecord::new(&host).enabled() == false,
            "Host scheduling is not disabled",
        ),
        Kind::Enable => require(
            HostRecord::new(&host).enabled() == true,
            "Host scheduling is not enabled",
        ),
        Kind::Evacuate => empty_disabled(session, &reference, &host).await,
        Kind::MarkMaintenance(value) => {
            if *value {
                empty_disabled(session, &reference, &host).await?;
            } else {
                require(
                    HostRecord::new(&host).enabled() == true,
                    "Host scheduling is not enabled",
                )?;
            }
            require(
                map_bool(&host, "MAINTENANCE_MODE") == Some(*value),
                "Host maintenance marker is not verified",
            )
        }
        Kind::Reboot | Kind::Restart => {
            let (key, old) = match op {
                HostOperation::Reboot { expected_boot_time } => ("boot_time", expected_boot_time),
                HostOperation::RestartToolstack {
                    expected_agent_start_time,
                } => ("agent_start_time", expected_agent_start_time),
                _ => return Err(Error::InvalidResourceCommand),
            };
            let old = old
                .parse::<f64>()
                .map_err(|_| Error::Protocol("reviewed start marker is invalid"))?;
            // The host writes its new start marker only once it is back; a reboot of a
            // nested or slow host takes minutes, a toolstack restart tens of seconds. Wait
            // within the connection's bound, treating an unreachable host as still restarting.
            let wait = session.options().host_restart_wait;
            let started = std::time::Instant::now();
            let mut current = host;
            loop {
                let confirmed = async {
                    live(session, &current, true).await?;
                    let fresh = current["other_config"][key]
                        .as_str()
                        .filter(|s| valid_epoch(s))
                        .and_then(|s| s.parse::<f64>().ok());
                    Ok::<bool, Error>(fresh.is_some_and(|fresh| fresh > old))
                }
                .await;
                if confirmed.is_ok_and(|confirmed| confirmed) {
                    return Ok(());
                }
                if started.elapsed() >= wait {
                    return Err(Error::Protocol(
                        "Host restart has not been confirmed by a newer start marker",
                    ));
                }
                tokio::time::sleep(std::time::Duration::from_secs(5).min(wait)).await;
                if let Ok((_, fresh_host, _)) = self::host(session, record).await {
                    current = fresh_host;
                }
            }
        }
        Kind::Shutdown => {
            require(
                step.task_succeeded,
                "Shutdown delivery is unresolved; an offline host alone does not prove shutdown",
            )?;
            // The coordinator marks a member offline only once its heartbeat lapses, so a
            // host that still reports live right after the task is not a failed shutdown.
            // Wait within the connection's restart bound for liveness to drop.
            let wait = session.options().host_restart_wait;
            let started = std::time::Instant::now();
            let mut current = host;
            loop {
                if live(session, &current, false).await.is_ok() {
                    return Ok(());
                }
                if started.elapsed() >= wait {
                    return Err(Error::HostPrerequisite(
                        "The host still reports live after the shutdown task; verify its power state",
                    ));
                }
                tokio::time::sleep(std::time::Duration::from_secs(5).min(wait)).await;
                if let Ok((_, fresh_host, _)) = self::host(session, record).await {
                    current = fresh_host;
                }
            }
        }
        Kind::PowerOn => live(session, &host, true).await,
        Kind::PowerMode => {
            let HostOperation::ConfigurePowerOn { mode, .. } = op else {
                return Err(Error::InvalidResourceCommand);
            };
            require(
                HostRecord::new(&host).power_on_mode() == mode.as_str()
                    && HostRecord::new(&host)
                        .power_on_config()
                        .entries()
                        .is_some_and(|m| m.is_empty()),
                "Host power-on configuration is not verified",
            )
        }
        Kind::Memory => {
            let HostOperation::SetDom0Memory {
                control_domain,
                expected,
                memory: desired,
            } = op
            else {
                return Err(Error::InvalidResourceCommand);
            };
            let (vm_ref, vm) = read(session, "VM", control_domain.uuid()).await?;
            require(
                HostRecord::new(&host).control_domain() == vm_ref
                    && VmRecord::new(&vm).is_control_domain() == true
                    && VmRecord::new(&vm).resident_on() == reference,
                "Control-domain identity changed",
            )?;
            require(
                memory(&vm)?
                    == MemoryLimits {
                        static_min: expected.static_min,
                        dynamic_min: *desired,
                        dynamic_max: *desired,
                        static_max: *desired,
                    },
                "Control-domain memory limits are not verified",
            )
        }
        Kind::Logging | Kind::ReconfigureLogging => {
            let HostOperation::SetSyslog {
                destination: desired,
                ..
            } = op
            else {
                return Err(Error::InvalidResourceCommand);
            };
            require(
                destination(&host)? == *desired,
                "Remote logging configuration is not verified",
            )?;
            if step.kind == Kind::ReconfigureLogging {
                require(
                    step.accepted,
                    "Logging configuration was saved, but applying it remains unresolved",
                )?;
            }
            Ok(())
        }
        Kind::Multipath | Kind::MultipathHandle => {
            let HostOperation::SetMultipath { enabled, .. } = op else {
                return Err(Error::InvalidResourceCommand);
            };
            require(
                HostRecord::new(&host).multipathing() == *enabled,
                "Host multipathing mode is not verified",
            )?;
            if step.kind == Kind::MultipathHandle {
                let actual = HostRecord::new(&host)
                    .other_config()
                    .entries()
                    .and_then(|m| m.get("multipathhandle"));
                require(
                    if *enabled {
                        actual == Some(&json!("dmp"))
                    } else {
                        actual.is_none()
                    },
                    "Host multipath handler is not verified",
                )?;
            }
            Ok(())
        }
        Kind::PoolAutostart | Kind::Forget => Err(Error::InvalidResourceCommand),
    }
}

/// Summarize the host's evacuation blocker report: up to five "VM: ERROR_CODE" entries with
/// sanitized codes and control characters removed from names, plus a count of the rest.
fn blocker_summary(blockers: &serde_json::Map<String, Value>, names: &[String]) -> String {
    let mut parts = blockers
        .values()
        .zip(names)
        .map(|(value, name)| {
            let code = value
                .as_array()
                .and_then(|a| a.first())
                .and_then(Value::as_str)
                .map(crate::xapi::safe_error_code)
                .unwrap_or("UNKNOWN_API_ERROR");
            let name: String = name.chars().filter(|c| !c.is_control()).take(64).collect();
            format!("{name}: {code}")
        })
        .collect::<Vec<_>>();
    if blockers.len() > names.len() {
        parts.push(format!("and {} more", blockers.len() - names.len()));
    }
    parts.join("; ")
}

#[cfg(test)]
mod repeated_task_tests {
    use super::*;

    fn step(task: Option<&str>, refused: &[&str]) -> Step {
        Step {
            kind: Kind::Enable,
            task: task.map(str::to_owned),
            accepted: true,
            task_succeeded: false,
            refused: refused.iter().map(|task| (*task).to_owned()).collect(),
        }
    }
    fn plan(step: Step) -> Plan {
        Plan { steps: vec![step] }
    }

    #[test]
    fn a_refused_task_is_superseded_only_by_a_repeat_that_keeps_it() {
        let first = step(Some("task-1"), &[]);
        // The repeat after a definite refusal: the old task moves to the refused list.
        let second = step(Some("task-2"), &["task-1"]);
        assert!(first.superseded_by(&second));
        assert!(plan(first.clone()).accepts(&plan(second.clone())));
        let third = step(Some("task-3"), &["task-1", "task-2"]);
        assert!(plan(second.clone()).accepts(&plan(third)));

        // A task reference that simply changes is still a conflict, as before.
        assert!(!plan(first.clone()).accepts(&plan(step(Some("task-2"), &[]))));
        // So is a repeat that drops an earlier refusal, names the wrong task as refused,
        // or lists its own task among the refused.
        assert!(!plan(second.clone()).accepts(&plan(step(Some("task-3"), &["task-2"]))));
        assert!(!plan(first.clone()).accepts(&plan(step(Some("task-2"), &["task-9"]))));
        assert!(!plan(first.clone()).accepts(&plan(step(Some("task-2"), &["task-2"]))));
        // The refused list never changes without a new task.
        assert!(!plan(second.clone()).accepts(&plan(step(Some("task-2"), &[]))));
        assert!(!plan(first.clone()).accepts(&plan(step(Some("task-1"), &["task-0"]))));
        // A task that succeeded is never superseded.
        let mut succeeded = first.clone();
        succeeded.task_succeeded = true;
        let mut after = second.clone();
        after.task_succeeded = true;
        assert!(!plan(succeeded).accepts(&plan(after)));
        // An unchanged step, and a first task arriving, are accepted as before.
        assert!(plan(first.clone()).accepts(&plan(first.clone())));
        assert!(plan(step(None, &[])).accepts(&plan(first)));
    }

    #[test]
    fn a_plan_written_before_refusals_were_recorded_still_reads() {
        let old: Step = serde_json::from_value(
            json!({"kind": "Enable", "task": "task-1", "accepted": true, "task_succeeded": false}),
        )
        .unwrap();
        assert!(old.refused.is_empty());
        assert!(
            !serde_json::to_string(&old).unwrap().contains("refused"),
            "a step with no refusal is written as it always was"
        );
    }
}

#[cfg(test)]
mod blocker_summary_tests {
    use super::*;

    #[test]
    fn blocker_summary_names_vms_and_sanitizes_codes() {
        let blockers: serde_json::Map<String, Value> = serde_json::from_value(json!({
            "OpaqueRef:a": ["VM_LACKS_FEATURE", "OpaqueRef:a"],
            "OpaqueRef:b": ["bad code; with text"],
            "OpaqueRef:c": [],
        }))
        .unwrap();
        let names = vec!["lab-evac\u{7}\n".to_string(), "second".to_string()];
        assert_eq!(
            blocker_summary(&blockers, &names),
            "lab-evac: VM_LACKS_FEATURE; second: UNKNOWN_API_ERROR; and 1 more"
        );
        assert_eq!(
            Error::EvacuationBlocked("x: VM_LACKS_FEATURE".into()).to_string(),
            "The host reports VMs that prevent evacuation (x: VM_LACKS_FEATURE); resolve their migration restrictions before retrying"
        );
    }
}
