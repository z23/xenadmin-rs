use super::*;
use xenadmin_core::{
    ObservedResource, OperationId, ResourceCommand, ResourceTarget, ScheduledSnapshotKind,
    SnapshotPolicyConfig, SnapshotPolicyId, SnapshotPolicyOperation as Op, SnapshotPolicyState,
    SnapshotPolicySupport, SnapshotPolicySupportQuery, SnapshotSchedule,
};
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Browse,
    Create,
    Edit,
}
pub(super) struct Workspace {
    initial: Arc<Inventory>,
    observed: Option<ObservedResource>,
    query: Option<SnapshotPolicySupportQuery>,
    support: Option<Result<SnapshotPolicySupport, String>>,
    selected: Option<SnapshotPolicyId>,
    mode: Mode,
    name: TextEdit,
    description: TextEdit,
    hour: TextEdit,
    minute: TextEdit,
    retention: TextEdit,
    frequency: u8,
    days: [bool; 7],
    checkpoint: bool,
    review: Option<Op>,
    error: Option<String>,
}
pub(super) fn open(s: &mut State) {
    let Some(initial) = s.inventory.clone() else {
        return;
    };
    let observed = s.snapshot.as_ref().and_then(|snap| {
        snap.observe_resource(ResourceTarget::Pool(initial.pool_id))
            .ok()
    });
    let query = observed.and_then(|o| s.connection.as_ref()?.snapshot_policy_support(o).ok());
    if let (Some(q), Some(waker)) = (&query, &s.waker) {
        q.notify_on_change(waker.clone());
    }
    s.snapshot_policy_workspace = Some(Workspace {
        initial,
        observed,
        query,
        support: None,
        selected: None,
        mode: Mode::Browse,
        name: TextEdit::new(""),
        description: TextEdit::new(""),
        hour: TextEdit::new("2"),
        minute: TextEdit::new("0"),
        retention: TextEdit::new("3"),
        frequency: 1,
        days: [true, false, false, false, false, false, false],
        checkpoint: false,
        review: None,
        error: None,
    });
}
pub(super) fn poll(s: &mut State) -> bool {
    let Some(w) = &mut s.snapshot_policy_workspace else {
        return false;
    };
    if let Some(result) = w.query.as_ref().and_then(|q| q.latest()) {
        w.support = Some(result);
        w.query = None;
        return true;
    }
    false
}
fn expected(w: &Workspace) -> Result<SnapshotPolicyState, String> {
    let p = w
        .selected
        .and_then(|id| w.initial.details.snapshot_policies.get(&id))
        .ok_or("Choose a snapshot schedule")?;
    SnapshotPolicyState::from_policy(p)
        .map_err(|_| "This schedule has unsupported settings or incomplete VM membership".into())
}
fn clock(w: &Workspace) -> Result<xenadmin_core::SnapshotPolicyClock, String> {
    w.support
        .as_ref()
        .and_then(|v| v.as_ref().ok())
        .map(|v| v.clock)
        .ok_or(
            "Refresh host support and local clock before configuring or enabling a schedule".into(),
        )
}
fn config(w: &Workspace) -> Result<SnapshotPolicyConfig, String> {
    let minute = w
        .minute
        .text
        .parse::<u8>()
        .map_err(|_| "Choose minute 0, 15, 30 or 45")?;
    let hour = w.hour.text.parse::<u8>().map_err(|_| "Choose hour 0–23")?;
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
        name: w.name.text.clone(),
        description: w.description.text.clone(),
        kind: if w.checkpoint {
            ScheduledSnapshotKind::Checkpoint
        } else {
            ScheduledSnapshotKind::Disk
        },
        retained: w
            .retention
            .text
            .parse()
            .map_err(|_| "Retention must be 1–10 snapshots per VM")?,
        schedule,
    };
    c.validate().map_err(|_|"Use a name, retention 1–10, hour 0–23 and minute 0, 15, 30 or 45; weekly schedules need at least one day".to_owned())?;
    Ok(c)
}
fn edit(s: &mut State, create: bool) {
    let Some(w) = &mut s.snapshot_policy_workspace else {
        return;
    };
    w.review = None;
    w.error = None;
    w.mode = if create { Mode::Create } else { Mode::Edit };
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
    w.name = TextEdit::new(&c.name);
    w.description = TextEdit::new(&c.description);
    w.retention = TextEdit::new(c.retained.to_string());
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
    w.hour = TextEdit::new(hour.to_string());
    w.minute = TextEdit::new(minute.to_string());
    w.days = std::array::from_fn(|i| days.contains(&(i as u8)));
}
fn review(s: &mut State) {
    let Some(w) = &mut s.snapshot_policy_workspace else {
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
#[derive(Clone, Copy)]
enum Action {
    Toggle,
    Run,
    Delete,
    Member(VmId, bool),
}
fn action(s: &mut State, action: Action) {
    let Some(w) = &mut s.snapshot_policy_workspace else {
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
                    return Err("Run now requires an enabled schedule with assigned VMs".into());
                }
                clock(w)?;
                Op::RunNow { expected }
            }
            Action::Delete => {
                if expected.enabled || !expected.members.is_empty() {
                    return Err(
                        "Disable the schedule and remove its VM assignments before deleting it"
                            .into(),
                    );
                }
                Op::DeleteEmpty { expected }
            }
            Action::Member(vm, join) => {
                if expected.enabled {
                    return Err("Disable the schedule before changing VM assignments".into());
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
fn current(s: &State, w: &Workspace) -> bool {
    !s.demo
        && s.can_mutate()
        && w.observed.is_some_and(|o| {
            s.snapshot
                .as_ref()
                .and_then(|snap| snap.observe_resource(o.target()).ok())
                == Some(o)
        })
}
fn submit(s: &mut State) {
    let result = (|| {
        let w = s
            .snapshot_policy_workspace
            .as_ref()
            .ok_or(xenadmin_core::Error::Stale)?;
        if !current(s, w) {
            return Err(xenadmin_core::Error::Stale);
        }
        let permission = session_permissions_view::assess(
            s,
            &w.review
                .as_ref()
                .ok_or(xenadmin_core::Error::Stale)?
                .permission_requirements(),
        );
        if permission.decision().is_denied() {
            return Err(xenadmin_core::Error::Api("RBAC_PERMISSION_DENIED".into()));
        }
        s.connection
            .as_ref()
            .ok_or(xenadmin_core::Error::Closed)?
            .submit_resource(
                OperationId::new(),
                ResourceCommand::SnapshotPolicy {
                    pool: w.observed.ok_or(xenadmin_core::Error::Stale)?,
                    operation: w.review.clone().ok_or(xenadmin_core::Error::Stale)?,
                },
            )
    })();
    match result {
        Ok(op) => {
            if let Some(waker) = &s.waker {
                op.notify_on_change(waker.clone());
            }
            s.resource_operations.push(op);
            s.snapshot_policy_workspace = None;
        }
        Err(e) => {
            if let Some(w) = &mut s.snapshot_policy_workspace {
                w.error = Some(e.to_string());
            }
        }
    }
}
fn clock_label(value: &str) -> String {
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
                .filter_map(|d| [
                    "Monday",
                    "Tuesday",
                    "Wednesday",
                    "Thursday",
                    "Friday",
                    "Saturday",
                    "Sunday"
                ]
                .get(*d as usize)
                .copied())
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
fn review_lines(w: &Workspace, op: &Op) -> Vec<String> {
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
            lines.push(if *enabled{"Enable host-side scheduling. The next coordinator cron check can run missed work immediately."}else{"Disable future scheduled runs. An already running snapshot job can continue."}.into());
            Some(expected)
        }
        Op::Membership { expected, vm, join } => {
            lines.push(format!(
                "{} VM {} ({vm})",
                if *join { "Assign" } else { "Remove" },
                w.initial
                    .vms
                    .get(vm)
                    .map(|v| v.name.as_str())
                    .unwrap_or("Unknown")
            ));
            Some(expected)
        }
        Op::DeleteEmpty { expected } => {
            lines.push("Delete this disabled, empty schedule and its associated policy messages. Guests and existing snapshots are retained.".into());
            Some(expected)
        }
        Op::RunNow { expected } => {
            lines.push("Run the host VMSS plugin for exactly these reviewed members. Partial snapshots and retention cleanup can remain after failure. A lost reply is never replayed.".into());
            Some(expected)
        }
    };
    if let Some(e) = e {
        lines.push(format!("Schedule {} ({})", e.config.name, e.id));
        for vm in &e.members {
            lines.push(format!(
                "Assigned VM: {} ({vm})",
                w.initial
                    .vms
                    .get(vm)
                    .map(|v| v.name.as_str())
                    .unwrap_or("Unknown")
            ));
        }
    }
    if matches!(op, Op::SetEnabled { enabled: true, .. } | Op::RunNow { .. }) {
        lines.push("Retention can delete the oldest VMSS-created snapshot for each VM, including snapshots created by a previous schedule. Manual snapshots are excluded. Checkpoint mode captures memory as well as disks.".into());
    }
    lines
}
pub(super) fn dialog(s: &State, k: &Kit) -> El {
    let Some(w) = &s.snapshot_policy_workspace else {
        return el();
    };
    let permission = w
        .review
        .as_ref()
        .map(|op| session_permissions_view::assess(s, &op.permission_requirements()));
    let denied = permission
        .as_ref()
        .is_some_and(|p| p.decision().is_denied());
    let mut body = col().gap(10.0).child(heading(
        w.review
            .as_ref()
            .map(|o| o.label())
            .unwrap_or("Snapshot schedules"),
        k,
    ));
    body=body.child(caption("Schedules run on the pool coordinator, independently of this desktop. The host checks every 15 minutes. This view shows configuration and last-run evidence; scheduling service health is not available.",k).wrap());
    match &w.support {
        Some(Ok(v)) => {
            body=body.child(caption(format!("{} local clock: {} (UTC{}{hours:02}:{minutes:02}) · UTC clock: {}. Hours and weekdays follow the coordinator's timezone, including future daylight-saving changes.",v.coordinator_name,clock_label(&v.local_time),if v.clock.utc_offset_minutes<0{"−"}else{"+"},clock_label(&v.utc_time),hours=v.clock.utc_offset_minutes.abs()/60,minutes=v.clock.utc_offset_minutes.abs()%60),k).wrap());
        }
        Some(Err(e)) => body = body.child(caption(e, k).wrap()),
        None => {
            body = body.child(
                caption(
                    if w.query.is_some() {
                        "Reading host support and local clock…"
                    } else {
                        "Host support and local clock have not been read"
                    },
                    k,
                )
                .wrap(),
            )
        }
    }
    if let Some(op) = &w.review {
        for line in review_lines(w, op) {
            body = body.child(caption(line, k).wrap_anywhere());
        }
    } else if w.mode == Mode::Browse {
        body = body.child(
            row()
                .gap(8.0)
                .flex_wrap()
                .child(
                    Button::new("New schedule")
                        .on_press(|s: &mut State| edit(s, true))
                        .build(k)
                        .key("policy-create"),
                )
                .child(
                    Button::new("Refresh")
                        .on_press(open)
                        .build(k)
                        .key("policy-refresh"),
                ),
        );
        for p in w.initial.details.snapshot_policies.values() {
            let id = p.id;
            body = body.child(
                Button::new(format!(
                    "{} · {} · {} VMs",
                    p.name,
                    match p.enabled {
                        Some(true) => "enabled",
                        Some(false) => "disabled",
                        None => "unknown",
                    },
                    p.vms.len()
                ))
                .on_press(move |s: &mut State| {
                    if let Some(w) = &mut s.snapshot_policy_workspace {
                        w.selected = Some(id);
                        w.error = None;
                    }
                })
                .build(k)
                .key(format!("policy-select-{id}")),
            );
        }
        if w.initial.details.snapshot_policies.is_empty() {
            body = body.child(caption(
                "No snapshot schedules were returned by the host.",
                k,
            ));
        }
        if let Some(p) = w
            .selected
            .and_then(|id| w.initial.details.snapshot_policies.get(&id))
        {
            body = body.child(heading(&p.name, k)).child(
                caption(
                    format!(
                        "UUID {} · {} · last run {}",
                        p.id,
                        p.frequency,
                        p.last_run_time.as_deref().unwrap_or("unknown")
                    ),
                    k,
                )
                .wrap(),
            );
            if let Ok(e) = expected(w) {
                body = body.child(
                    row()
                        .gap(6.0)
                        .flex_wrap()
                        .child(
                            Button::new("Edit settings")
                                .disabled(e.enabled)
                                .on_press(|s: &mut State| edit(s, false))
                                .build(k)
                                .key("policy-edit"),
                        )
                        .child(
                            Button::new(if e.enabled { "Disable" } else { "Enable" })
                                .on_press(|s: &mut State| action(s, Action::Toggle))
                                .build(k)
                                .key("policy-toggle"),
                        )
                        .child(
                            Button::new("Run now")
                                .disabled(!e.enabled || e.members.is_empty())
                                .on_press(|s: &mut State| action(s, Action::Run))
                                .build(k)
                                .key("policy-run"),
                        )
                        .child(
                            Button::new("Delete empty schedule")
                                .disabled(e.enabled || !e.members.is_empty())
                                .on_press(|s: &mut State| action(s, Action::Delete))
                                .build(k)
                                .key("policy-delete"),
                        ),
                );
                body=body.child(caption("Disable this schedule before editing settings or VM assignments. To delete it, remove each reviewed VM assignment first.",k).wrap());
                for vm in w
                    .initial
                    .vms
                    .values()
                    .filter(|v| !v.is_template && !v.is_control_domain)
                {
                    let id = vm.id;
                    let assigned = e.members.contains(&id);
                    if !assigned
                        && w.initial
                            .details
                            .vms
                            .get(&id)
                            .is_some_and(|d| d.snapshot_policy.is_some())
                    {
                        continue;
                    }
                    body = body.child(
                        Button::new(format!(
                            "{} {}",
                            if assigned { "Remove" } else { "Assign" },
                            vm.name
                        ))
                        .disabled(e.enabled)
                        .on_press(move |s: &mut State| action(s, Action::Member(id, !assigned)))
                        .build(k)
                        .key(format!("policy-member-{id}")),
                    );
                }
            } else {
                body=body.child(caption("This policy's settings or membership are incomplete or unsupported. Quiesced/VSS schedules are not supported.",k).wrap());
            }
        }
    } else {
        body = body.child(caption("Schedule name", k).wrap()).child(
            TextInput::new(&w.name)
                .label("Schedule name")
                .on_edit(|s: &mut State| &mut s.snapshot_policy_workspace.as_mut().unwrap().name)
                .build(k)
                .key("policy-name"),
        );
        body = body.child(caption("Description", k).wrap()).child(
            TextInput::new(&w.description)
                .label("Description")
                .on_edit(|s: &mut State| {
                    &mut s.snapshot_policy_workspace.as_mut().unwrap().description
                })
                .build(k)
                .key("policy-description"),
        );
        let mut frequencies = row().gap(6.0).flex_wrap();
        for (i, label) in ["Hourly", "Daily", "Weekly"].iter().enumerate() {
            frequencies = frequencies.child(
                Button::new(*label)
                    .disabled(w.frequency == i as u8)
                    .on_press(move |s: &mut State| {
                        s.snapshot_policy_workspace.as_mut().unwrap().frequency = i as u8
                    })
                    .build(k)
                    .key(format!("policy-frequency-{i}")),
            );
        }
        body = body.child(frequencies);
        if w.frequency != 0 {
            body = body
                .child(caption("Coordinator local hour (0–23)", k).wrap())
                .child(
                    TextInput::new(&w.hour)
                        .label("Coordinator local hour (0–23)")
                        .on_edit(|s: &mut State| {
                            &mut s.snapshot_policy_workspace.as_mut().unwrap().hour
                        })
                        .build(k)
                        .key("policy-hour"),
                );
        }
        body = body
            .child(caption("Minute (0, 15, 30 or 45)", k).wrap())
            .child(
                TextInput::new(&w.minute)
                    .label("Minute (0, 15, 30 or 45)")
                    .on_edit(|s: &mut State| {
                        &mut s.snapshot_policy_workspace.as_mut().unwrap().minute
                    })
                    .build(k)
                    .key("policy-minute"),
            );
        if w.frequency == 2 {
            for (i, label) in [
                "Monday",
                "Tuesday",
                "Wednesday",
                "Thursday",
                "Friday",
                "Saturday",
                "Sunday",
            ]
            .iter()
            .enumerate()
            {
                body = body.child(
                    Checkbox::new(w.days[i])
                        .label(*label)
                        .on_toggle(move |s: &mut State| {
                            let w = s.snapshot_policy_workspace.as_mut().unwrap();
                            w.days[i] = !w.days[i];
                        })
                        .build(k)
                        .key(format!("policy-day-{i}")),
                );
            }
        }
        body = body
            .child(caption("Retained scheduled snapshots per VM (1–10)", k).wrap())
            .child(
                TextInput::new(&w.retention)
                    .label("Retained scheduled snapshots per VM (1–10)")
                    .on_edit(|s: &mut State| {
                        &mut s.snapshot_policy_workspace.as_mut().unwrap().retention
                    })
                    .build(k)
                    .key("policy-retention"),
            );
        body = body.child(
            Checkbox::new(w.checkpoint)
                .label("Include VM memory (checkpoint)")
                .on_toggle(|s: &mut State| {
                    let w = s.snapshot_policy_workspace.as_mut().unwrap();
                    w.checkpoint = !w.checkpoint;
                })
                .build(k)
                .key("policy-checkpoint"),
        );
        body=body.child(caption("Disk snapshots are the default. Quiesced/VSS snapshots are unavailable. Creating or editing a schedule leaves it disabled.",k).wrap());
    }
    if s.demo {
        body =
            body.child(caption("Synthetic preview · no host changes can be submitted", k).wrap());
    }
    if let Some(e) = &w.error {
        body = body.child(caption(e, k).wrap().color(k.c.danger.icon).role("alert"));
    }
    let mut controls = row().gap(8.0).flex_wrap().justify_end().child(
        Button::new("Close")
            .on_press(|s: &mut State| s.snapshot_policy_workspace = None)
            .build(k)
            .key("policy-close"),
    );
    controls = controls.child(
        Button::new("Operation history")
            .on_press(|s: &mut State| {
                s.snapshot_policy_workspace = None;
                activity::open(s);
            })
            .build(k)
            .key("policy-history"),
    );
    if w.review.is_some() || w.mode != Mode::Browse {
        controls = controls.child(
            Button::new("Back")
                .on_press(|s: &mut State| {
                    if let Some(w) = &mut s.snapshot_policy_workspace {
                        if w.review.take().is_none() {
                            w.mode = Mode::Browse;
                        }
                        w.error = None;
                    }
                })
                .build(k)
                .key("policy-back"),
        );
        controls = if w.review.is_some() {
            controls.child(
                Button::new("Confirm change")
                    .variant(ButtonVariant::Danger)
                    .disabled(!current(s, w) || denied)
                    .on_press(submit)
                    .build(k)
                    .key("policy-confirm"),
            )
        } else {
            controls.child(
                Button::new("Review")
                    .on_press(review)
                    .build(k)
                    .key("policy-review"),
            )
        };
    }
    el().w_full()
        .h_full()
        .absolute(0.0, 0.0)
        .center()
        .fill(rune::Color::BLACK.alpha(0.5))
        .role("dialog")
        .label("Snapshot schedules")
        .modal(false, |s: &mut State| s.snapshot_policy_workspace = None)
        .child(
            col()
                .w((s.window_width - 32.0).clamp(280.0, 720.0))
                .h((s.window_height - 56.0).clamp(280.0, 820.0))
                .padding(18.0)
                .gap(12.0)
                .radius(10.0)
                .fill(k.c.bg_base)
                .child(body.grow(1.0).min_h(0.0).scroll(rune::ScrollSpec {
                    stick_bottom: false,
                }))
                .child(session_permissions_view::footer(
                    controls,
                    permission.as_ref(),
                    (s.window_width - 32.0).clamp(280.0, 720.0) - 36.0,
                    k,
                )),
        )
}
pub(super) fn scene(s: &mut State) {
    demo(s);
    open(s);
    let w = s.snapshot_policy_workspace.as_mut().unwrap();
    let coordinator = *w.initial.hosts.keys().next().unwrap();
    w.support = Some(Ok(SnapshotPolicySupport {
        clock: xenadmin_core::SnapshotPolicyClock {
            coordinator,
            utc_offset_minutes: 600,
        },
        coordinator_name: "Example coordinator".into(),
        local_time: "20261108T10:00:00".into(),
        utc_time: "20261108T00:00:00Z".into(),
    }));
    edit(s, true);
    s.snapshot_policy_workspace.as_mut().unwrap().name = TextEdit::new("Daily recovery");
}
pub(super) fn review_scene(s: &mut State) {
    scene(s);
    review(s);
}
#[cfg(test)]
mod tests {
    use super::*;
    fn state() -> State {
        let mut s = init();
        scene(&mut s);
        s
    }
    fn runtime(mut s: State, size: (f32, f32)) -> rune::Runtime<State> {
        s.window_width = size.0;
        s.window_height = size.1;
        let mut r = rune::Runtime::new(
            s,
            view,
            size,
            rune::Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        settle(&mut r);
        r
    }
    fn settle(r: &mut rune::Runtime<State>) {
        for _ in 0..30 {
            r.frame(1.0 / 60.0);
        }
    }
    fn click(r: &mut rune::Runtime<State>, key: &str) {
        r.update(|_, tx| tx.scroll_into_view(key));
        settle(r);
        let (x, y) = r.find(key).unwrap();
        assert!(x >= 0.0 && x <= r.state.window_width && y >= 0.0 && y <= r.state.window_height);
        r.pointer_down(x, y);
        r.pointer_up(x, y);
        settle(r);
    }
    #[test]
    fn schedule_review_captures_coordinator_and_pool_at_both_widths() {
        for size in [(1440.0, 900.0), (513.0, 652.0)] {
            let mut r = runtime(state(), size);
            click(&mut r, "policy-review");
            let op = r
                .state
                .snapshot_policy_workspace
                .as_ref()
                .unwrap()
                .review
                .clone()
                .unwrap();
            assert!(matches!(op, Op::Create { .. }));
            Arc::make_mut(r.state.inventory.as_mut().unwrap()).pool_id =
                PoolId::from(uuid::Uuid::new_v4());
            click(&mut r, "policy-confirm");
            assert!(r.state.resource_operations.is_empty());
            assert_eq!(
                r.state.snapshot_policy_workspace.as_ref().unwrap().review,
                Some(op)
            );
        }
    }
    #[test]
    fn invalid_minutes_retention_and_unknown_clock_block_review() {
        let mut s = state();
        for value in ["1", "16", "60", "-1"] {
            s.snapshot_policy_workspace.as_mut().unwrap().minute = TextEdit::new(value);
            review(&mut s);
            assert!(
                s.snapshot_policy_workspace
                    .as_ref()
                    .unwrap()
                    .review
                    .is_none()
            );
        }
        s.snapshot_policy_workspace.as_mut().unwrap().minute = TextEdit::new("15");
        s.snapshot_policy_workspace.as_mut().unwrap().retention = TextEdit::new("11");
        review(&mut s);
        assert!(
            s.snapshot_policy_workspace
                .as_ref()
                .unwrap()
                .review
                .is_none()
        );
        s.snapshot_policy_workspace.as_mut().unwrap().retention = TextEdit::new("3");
        s.snapshot_policy_workspace.as_mut().unwrap().support = None;
        review(&mut s);
        assert!(
            s.snapshot_policy_workspace
                .as_ref()
                .unwrap()
                .review
                .is_none()
        );
    }
    #[test]
    fn weekly_day_selection_and_checkpoint_are_reviewed() {
        let mut r = runtime(state(), (513.0, 652.0));
        click(&mut r, "policy-frequency-2");
        click(&mut r, "policy-day-1");
        click(&mut r, "policy-checkpoint");
        click(&mut r, "policy-review");
        assert!(
            matches!(r.state.snapshot_policy_workspace.as_ref().unwrap().review,Some(Op::Create{config:SnapshotPolicyConfig{kind:ScheduledSnapshotKind::Checkpoint,schedule:SnapshotSchedule::Weekly{ref days,..},..},..})if days==&vec![1])
        );
    }
}
