//! Snapshot schedules: the logic carried over from Rune
//! `snapshot_policy_view.rs`. What a workspace captures, how a draft becomes
//! a reviewed [`SnapshotPolicyOperation`], the staleness check and the
//! submission. Validation, refusals and review texts are word for word.
//!
//! What the port changed here:
//!
//! - The workspace follows the pool of its TARGET (`inventory_for`,
//!   `snapshot_for`, `connection_for`), not the scope session.
//! - The form is a wizard of three draft steps and the review
//!   ([`STEPS`]); `step` says which draft step shows. The checks still run
//!   together when the review is asked for, in the Rune order.
//! - `vm` is the VM of the command "Assign to Snapshot Schedule": it is
//!   listed first among the members and its schedule starts selected.
//!   Every membership change is still one reviewed operation.
//! - A confirm button that is disabled says why ([`STALE`]).

use std::sync::Arc;

use xenadmin_core::{
    Error, Inventory, ObservedResource, OperationId, PoolId, ResourceCommand, ResourceOperation,
    ResourceTarget, ScheduledSnapshotKind, SnapshotPolicyClock, SnapshotPolicyConfig,
    SnapshotPolicyId, SnapshotPolicyOperation as Op, SnapshotPolicyState, SnapshotPolicySupport,
    SnapshotPolicySupportQuery, SnapshotSchedule, VmId,
};

use crate::prelude::*;

/// The steps of the wizard. The last is the review.
pub const STEPS: [&str; 4] = ["Name", "Schedule", "Snapshot Type", "Review"];

/// The last step that is a draft: its Next is "Review".
pub const LAST_DRAFT_STEP: usize = 2;

/// Shown with a review that is no longer current (Rune disabled the button
/// without a word).
pub const STALE: &str = "The connection or the pool changed. Close and reopen snapshot schedules \
    before applying this review.";

pub const WEEKDAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Browse,
    Create,
    Edit,
}

pub struct Workspace {
    pub initial: Arc<Inventory>,
    pub observed: Option<ObservedResource>,
    pub query: Option<SnapshotPolicySupportQuery>,
    pub support: Option<Result<SnapshotPolicySupport, String>>,
    pub selected: Option<SnapshotPolicyId>,
    /// The VM "Assign to Snapshot Schedule" was invoked on.
    pub vm: Option<VmId>,
    pub mode: Mode,
    /// The draft step of the wizard that shows, 0 to [`LAST_DRAFT_STEP`].
    pub step: usize,
    pub name: String,
    pub description: String,
    pub hour: String,
    pub minute: String,
    pub retention: String,
    /// 0 hourly, 1 daily, 2 weekly.
    pub frequency: u8,
    /// Monday first.
    pub days: [bool; 7],
    pub checkpoint: bool,
    pub review: Option<Op>,
    pub error: Option<String>,
}

impl Workspace {
    pub fn pool(&self) -> PoolId {
        self.initial.pool_id
    }
}

/// Captures the pool as it is and asks its coordinator for its clock. Also
/// the "Refresh" button: it rebuilds the whole workspace and discards drafts.
pub fn open(m: &mut Model, pool: PoolId, vm: Option<VmId>) {
    let Some(initial) = m.inventory_for(pool) else {
        return;
    };
    let observed = m.snapshot_for(pool).and_then(|snap| {
        snap.observe_resource(ResourceTarget::Pool(initial.pool_id))
            .ok()
    });
    let query = observed.and_then(|o| m.connection_for(pool)?.snapshot_policy_support(o).ok());
    if let (Some(q), Some(waker)) = (&query, &m.waker) {
        q.notify_on_change(waker.clone());
    }
    let selected = vm
        .and_then(|vm| initial.details.vms.get(&vm)?.snapshot_policy)
        .filter(|id| initial.details.snapshot_policies.contains_key(id));
    m.areas.schedules_alarms.schedules = Some(Workspace {
        initial,
        observed,
        query,
        support: None,
        selected,
        vm,
        mode: Mode::Browse,
        step: 0,
        name: String::new(),
        description: String::new(),
        hour: "2".into(),
        minute: "0".into(),
        retention: "3".into(),
        frequency: 1,
        days: [true, false, false, false, false, false, false],
        checkpoint: false,
        review: None,
        error: None,
    });
}

/// The "Refresh" button.
pub fn refresh(m: &mut Model) {
    let Some(w) = &m.areas.schedules_alarms.schedules else {
        return;
    };
    let (pool, vm) = (w.pool(), w.vm);
    open(m, pool, vm);
}

/// Moves the answer of the coordinator into the workspace.
pub fn poll(m: &mut Model) -> bool {
    let Some(w) = &mut m.areas.schedules_alarms.schedules else {
        return false;
    };
    if let Some(result) = w.query.as_ref().and_then(|q| q.latest()) {
        w.support = Some(result);
        w.query = None;
        return true;
    }
    false
}

pub fn expected(w: &Workspace) -> Result<SnapshotPolicyState, String> {
    let p = w
        .selected
        .and_then(|id| w.initial.details.snapshot_policies.get(&id))
        .ok_or("Choose a snapshot schedule")?;
    SnapshotPolicyState::from_policy(p)
        .map_err(|_| "This schedule has unsupported settings or incomplete VM membership".into())
}

fn clock(w: &Workspace) -> Result<SnapshotPolicyClock, String> {
    w.support
        .as_ref()
        .and_then(|v| v.as_ref().ok())
        .map(|v| v.clock)
        .ok_or(
            "Refresh host support and local clock before configuring or enabling a schedule".into(),
        )
}

/// The draft as a configuration. The hour is parsed even for an hourly
/// schedule, whose step does not show it; the order of the refusals is
/// minute, hour, retention, then the core's own check.
fn config(w: &Workspace) -> Result<SnapshotPolicyConfig, String> {
    let minute = w
        .minute
        .parse::<u8>()
        .map_err(|_| "Choose minute 0, 15, 30 or 45")?;
    let hour = w.hour.parse::<u8>().map_err(|_| "Choose hour 0–23")?;
    let schedule = match w.frequency {
        0 => SnapshotSchedule::Hourly { minute },
        1 => SnapshotSchedule::Daily { hour, minute },
        _ => SnapshotSchedule::Weekly {
            hour,
            minute,
            days: w
                .days
                .iter()
                .enumerate()
                .filter_map(|(i, b)| b.then_some(i as u8))
                .collect(),
        },
    };
    let c = SnapshotPolicyConfig {
        name: w.name.clone(),
        description: w.description.clone(),
        kind: if w.checkpoint {
            ScheduledSnapshotKind::Checkpoint
        } else {
            ScheduledSnapshotKind::Disk
        },
        retained: w
            .retention
            .parse()
            .map_err(|_| "Retention must be 1–10 snapshots per VM")?,
        schedule,
    };
    c.validate().map_err(|_| {
        "Use a name, retention 1–10, hour 0–23 and minute 0, 15, 30 or 45; weekly schedules need \
         at least one day"
            .to_owned()
    })?;
    Ok(c)
}

/// "New schedule" and "Edit settings": the wizard, on its first step. A new
/// schedule starts with no weekday ticked.
pub fn edit(m: &mut Model, create: bool) {
    let Some(w) = &mut m.areas.schedules_alarms.schedules else {
        return;
    };
    w.review = None;
    w.error = None;
    let c = if create {
        SnapshotPolicyConfig {
            name: "".into(),
            description: "".into(),
            kind: ScheduledSnapshotKind::Disk,
            retained: 3,
            schedule: SnapshotSchedule::Daily { hour: 2, minute: 0 },
        }
    } else {
        let Ok(e) = expected(w) else {
            w.error = Some("This schedule cannot be edited".into());
            return;
        };
        e.config
    };
    w.mode = if create { Mode::Create } else { Mode::Edit };
    w.step = 0;
    w.name = c.name.clone();
    w.description = c.description.clone();
    w.retention = c.retained.to_string();
    w.checkpoint = c.kind == ScheduledSnapshotKind::Checkpoint;
    let (hour, minute, days) = match c.schedule {
        SnapshotSchedule::Hourly { minute } => {
            w.frequency = 0;
            (0, minute, vec![])
        }
        SnapshotSchedule::Daily { hour, minute } => {
            w.frequency = 1;
            (hour, minute, vec![])
        }
        SnapshotSchedule::Weekly { hour, minute, days } => {
            w.frequency = 2;
            (hour, minute, days)
        }
    };
    w.hour = hour.to_string();
    w.minute = minute.to_string();
    w.days = std::array::from_fn(|i| days.contains(&(i as u8)));
}

/// "Next" of a draft step that is not the last.
pub fn next(m: &mut Model) {
    if let Some(w) = &mut m.areas.schedules_alarms.schedules {
        w.step = (w.step + 1).min(LAST_DRAFT_STEP);
        w.error = None;
    }
}

/// "Review" of the wizard: every check of the form, then the operation.
pub fn review(m: &mut Model) {
    let Some(w) = &mut m.areas.schedules_alarms.schedules else {
        return;
    };
    let result = (|| {
        let config = config(w)?;
        let clock = clock(w)?;
        Ok::<_, String>(if w.mode == Mode::Create {
            Op::Create { config, clock }
        } else {
            let expected = expected(w)?;
            if expected.enabled {
                return Err("Disable the schedule before editing".into());
            }
            if expected.config == config {
                return Err("No settings changed".into());
            }
            Op::Configure {
                expected,
                config,
                clock,
            }
        })
    })();
    match result {
        Ok(op) => {
            w.review = Some(op);
            w.error = None
        }
        Err(e) => w.error = Some(e),
    }
}

/// An action on the selected schedule of the list.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Toggle,
    Run,
    Delete,
    Member(VmId, bool),
}

/// The refusals Rune showed as a disabled button, with the words its click
/// would have given.
pub const EDIT_NEEDS_DISABLED: &str = "Disable the schedule before editing";
pub const RUN_NEEDS: &str = "Run now requires an enabled schedule with assigned VMs";
pub const DELETE_NEEDS: &str =
    "Disable the schedule and remove its VM assignments before deleting it";
pub const MEMBER_NEEDS_DISABLED: &str = "Disable the schedule before changing VM assignments";

pub fn action(m: &mut Model, action: Action) {
    let Some(w) = &mut m.areas.schedules_alarms.schedules else {
        return;
    };
    let result = (|| {
        let expected = expected(w)?;
        Ok::<_, String>(match action {
            Action::Toggle => {
                let enabled = !expected.enabled;
                let clock = if enabled {
                    if expected.members.is_empty() {
                        return Err("Assign a VM before enabling this schedule".into());
                    }
                    Some(clock(w)?)
                } else {
                    None
                };
                Op::SetEnabled {
                    expected,
                    enabled,
                    clock,
                }
            }
            Action::Run => {
                if !expected.enabled || expected.members.is_empty() {
                    return Err(RUN_NEEDS.into());
                }
                clock(w)?;
                Op::RunNow { expected }
            }
            Action::Delete => {
                if expected.enabled || !expected.members.is_empty() {
                    return Err(DELETE_NEEDS.into());
                }
                Op::DeleteEmpty { expected }
            }
            Action::Member(vm, join) => {
                if expected.enabled {
                    return Err(MEMBER_NEEDS_DISABLED.into());
                }
                Op::Membership { expected, vm, join }
            }
        })
    })();
    match result {
        Ok(op) => {
            w.review = Some(op);
            w.error = None
        }
        Err(e) => w.error = Some(e),
    }
}

/// A row of the list was chosen: only the selection moves.
pub fn select(m: &mut Model, id: &str) {
    let Some(w) = &mut m.areas.schedules_alarms.schedules else {
        return;
    };
    let found = w
        .initial
        .details
        .snapshot_policies
        .keys()
        .find(|policy| policy.to_string() == id)
        .copied();
    if let Some(policy) = found {
        w.selected = Some(policy);
        w.error = None;
    }
}

/// "Back": from a review to what was reviewed, from a later step of the
/// wizard to the one before, from its first step to the list.
pub fn back(m: &mut Model) {
    if let Some(w) = &mut m.areas.schedules_alarms.schedules {
        if w.review.take().is_none() {
            if w.step == 0 {
                w.mode = Mode::Browse;
            } else {
                w.step -= 1;
            }
        }
        w.error = None;
    }
}

/// Whether the pool of the workspace is a synthetic one.
pub fn synthetic(m: &Model, w: &Workspace) -> bool {
    m.is_synthetic_pool(w.pool())
}

pub fn current(m: &Model, w: &Workspace) -> bool {
    let pool = w.pool();
    !synthetic(m, w)
        && m.can_mutate_pool(pool)
        && w.observed.is_some_and(|o| {
            m.snapshot_for(pool)
                .and_then(|snap| snap.observe_resource(o.target()).ok())
                == Some(o)
        })
}

/// Why the review showing cannot be confirmed, when it is no longer
/// current. In a synthetic pool the preview notice says it instead.
pub fn stale(m: &Model, w: &Workspace) -> Option<String> {
    (w.review.is_some() && !synthetic(m, w) && !current(m, w)).then(|| STALE.to_owned())
}

/// "Confirm change". On success the handle goes to the model and the
/// workspace closes; on a refusal it stays with the reason and the review.
/// A recheck of the operation is a control on its record and never comes
/// back here: nothing replays this write.
pub fn submit(m: &mut Model) {
    let result = (|| {
        let w = m
            .areas
            .schedules_alarms
            .schedules
            .as_ref()
            .ok_or(Error::Stale)?;
        if !current(m, w) {
            return Err(Error::Stale);
        }
        let permission = permissions::assess_for(
            m,
            w.pool(),
            &w.review
                .as_ref()
                .ok_or(Error::Stale)?
                .permission_requirements(),
        );
        if permissions::denial(&permission).is_some() {
            return Err(Error::Api("RBAC_PERMISSION_DENIED".into()));
        }
        m.connection_for(w.pool())
            .ok_or(Error::Closed)?
            .submit_resource(
                OperationId::new(),
                ResourceCommand::SnapshotPolicy {
                    pool: w.observed.ok_or(Error::Stale)?,
                    operation: w.review.clone().ok_or(Error::Stale)?,
                },
            )
    })();
    finish(m, result);
}

fn finish(m: &mut Model, result: Result<ResourceOperation, Error>) {
    match result {
        Ok(op) => {
            m.track_resource(op);
            m.areas.schedules_alarms.schedules = None;
        }
        Err(e) => {
            if let Some(w) = &mut m.areas.schedules_alarms.schedules {
                w.error = Some(e.to_string());
            }
        }
    }
}

pub fn clock_label(value: &str) -> String {
    let v = value.strip_suffix('Z').unwrap_or(value);
    if v.len() == 17 && v.is_ascii() {
        format!("{}-{}-{} {}", &v[..4], &v[4..6], &v[6..8], &v[9..])
    } else {
        v.to_owned()
    }
}

fn schedule_label(schedule: &SnapshotSchedule) -> String {
    match schedule {
        SnapshotSchedule::Hourly { minute } => format!("Every hour at minute {minute:02}"),
        SnapshotSchedule::Daily { hour, minute } => format!("Daily at {hour:02}:{minute:02}"),
        SnapshotSchedule::Weekly { hour, minute, days } => format!(
            "{} at {hour:02}:{minute:02}",
            days.iter()
                .filter_map(|d| WEEKDAYS.get(*d as usize).copied())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn config_label(c: &SnapshotPolicyConfig) -> String {
    format!(
        "{} · {} · retain {} per VM · {}",
        c.name,
        if c.kind == ScheduledSnapshotKind::Disk {
            "disks only"
        } else {
            "disks and memory"
        },
        c.retained,
        schedule_label(&c.schedule)
    )
}

fn vm_name(w: &Workspace, vm: &VmId) -> String {
    w.initial
        .vms
        .get(vm)
        .map(|v| v.name.as_str())
        .unwrap_or("Unknown")
        .to_owned()
}

/// The review text of an operation, word for word from Rune.
pub fn review_lines(w: &Workspace, op: &Op) -> Vec<String> {
    let mut lines = vec![format!("Pool {}", w.initial.pool_id)];
    let e = match op {
        Op::Create { config, clock } => {
            lines.push(
                "Creates an empty, disabled schedule. Assign VMs, then enable it separately."
                    .into(),
            );
            lines.push(config_label(config));
            lines.push(format!("Description: {}", config.description));
            lines.push(format!(
                "Coordinator {} · local UTC offset {:+} minutes",
                clock.coordinator, clock.utc_offset_minutes
            ));
            None
        }
        Op::Configure {
            expected, config, ..
        } => {
            lines.push(format!("New settings: {}", config_label(config)));
            lines.push(format!("Description: {}", config.description));
            lines.push(
                "The schedule stays disabled. A failure may leave some reviewed settings changed."
                    .into(),
            );
            Some(expected)
        }
        Op::SetEnabled {
            expected, enabled, ..
        } => {
            lines.push(
                if *enabled {
                    "Enable host-side scheduling. The next coordinator cron check can run missed \
                     work immediately."
                } else {
                    "Disable future scheduled runs. An already running snapshot job can continue."
                }
                .into(),
            );
            Some(expected)
        }
        Op::Membership { expected, vm, join } => {
            lines.push(format!(
                "{} VM {} ({vm})",
                if *join { "Assign" } else { "Remove" },
                vm_name(w, vm)
            ));
            Some(expected)
        }
        Op::DeleteEmpty { expected } => {
            lines.push(
                "Delete this disabled, empty schedule and its associated policy messages. Guests \
                 and existing snapshots are retained."
                    .into(),
            );
            Some(expected)
        }
        Op::RunNow { expected } => {
            lines.push(
                "Run the host VMSS plugin for exactly these reviewed members. Partial snapshots \
                 and retention cleanup can remain after failure. A lost reply is never replayed."
                    .into(),
            );
            Some(expected)
        }
    };
    if let Some(e) = e {
        lines.push(format!("Schedule {} ({})", e.config.name, e.id));
        for vm in &e.members {
            lines.push(format!("Assigned VM: {} ({vm})", vm_name(w, vm)));
        }
    }
    if matches!(op, Op::SetEnabled { enabled: true, .. } | Op::RunNow { .. }) {
        lines.push(
            "Retention can delete the oldest VMSS-created snapshot for each VM, including \
             snapshots created by a previous schedule. Manual snapshots are excluded. Checkpoint \
             mode captures memory as well as disks."
                .into(),
        );
    }
    lines
}
