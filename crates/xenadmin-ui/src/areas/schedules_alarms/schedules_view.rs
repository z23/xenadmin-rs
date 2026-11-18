//! Snapshot schedules, described: the dialog with the list of schedules
//! (`snapshot-schedules`) and the wizard that creates or edits one
//! (`snapshot-schedule-wizard`). Nothing here changes the model.
//!
//! Keys and labels are the Rune ones: `policy-create`, `policy-refresh`,
//! `policy-select-<id>` (now the options of the list), `policy-edit`,
//! `policy-toggle`, `policy-run`, `policy-delete`, `policy-member-<vm>`,
//! `policy-name`, `policy-description`, `policy-frequency-<0|1|2>` (now the
//! options of one choice), `policy-hour`, `policy-minute`,
//! `policy-day-<0..6>`, `policy-retention`, `policy-checkpoint`,
//! `policy-close`, `policy-history`, `policy-back`, `policy-review`,
//! `policy-confirm`.
//!
//! New keys, for elements Rune left without one or that did not exist:
//! `policy-list` (the list), `policy-frequency` (the choice), `policy-next`
//! (Next of the first two steps of the wizard), `policy-change-review` (the
//! review, so `policy-change-review-stale` is its stale line),
//! `policy-about`, `policy-clock`, `policy-detail`, `policy-assignments`,
//! `policy-vm`, `policy-unsupported`, `policy-empty`, `policy-preview`,
//! `policy-error`.

use xenadmin_core::{PermissionAssessment, SnapshotPolicy};

use super::schedules::{self, Action, LAST_DRAFT_STEP, Mode, STEPS, WEEKDAYS, Workspace};
use crate::prelude::*;

pub const DIALOG: &str = "snapshot-schedules";
pub const WIZARD: &str = "snapshot-schedule-wizard";
pub const REVIEW: &str = "policy-change-review";

const PREVIEW: &str = "Synthetic preview · no host changes can be submitted";
const DEMO: &str = "Demo mode cannot change infrastructure.";

const ABOUT: &str = "Schedules run on the pool coordinator, independently of this desktop. The \
    host checks every 15 minutes. This view shows configuration and last-run evidence; \
    scheduling service health is not available.";

fn workspace(m: &mut Model) -> Option<&mut Workspace> {
    m.areas.schedules_alarms.schedules.as_mut()
}

pub fn surface(m: &Model) -> Option<Surface> {
    let w = m.areas.schedules_alarms.schedules.as_ref()?;
    let permission = w
        .review
        .as_ref()
        .map(|op| permissions::assess_for(m, w.pool(), &op.permission_requirements()));
    let surface = if w.mode == Mode::Browse {
        Surface::dialog(DIALOG, "Snapshot schedules").large()
    } else {
        let current = if w.review.is_some() {
            STEPS.len() - 1
        } else {
            w.step
        };
        let title = if w.mode == Mode::Create {
            "New schedule"
        } else {
            "Edit settings"
        };
        let wizard = Surface::wizard(WIZARD, title, STEPS, current);
        match selected(w).filter(|_| w.mode == Mode::Edit) {
            Some(p) => wizard.subtitle(format!("{} ({})", p.name, p.id)),
            None => wizard,
        }
    };
    let surface = surface
        .experimental(true)
        .body(body(m, w))
        .footer(footer(m, w, permission.as_ref()))
        .on_close(super::close_schedules);
    Some(match &permission {
        Some(permission) => surface.footer_note(permissions::footer_note(permission)),
        None => surface,
    })
}

fn selected(w: &Workspace) -> Option<&SnapshotPolicy> {
    w.selected
        .and_then(|id| w.initial.details.snapshot_policies.get(&id))
}

fn body(m: &Model, w: &Workspace) -> Vec<Block> {
    blocks![
        w.review.as_ref().map(|op| heading(op.label())),
        caption(ABOUT).key("policy-about"),
        clock_line(w),
        match &w.review {
            Some(op) => {
                blocks![review(REVIEW, schedules::review_lines(w, op)).stale(schedules::stale(m, w))]
            }
            None if w.mode == Mode::Browse => browse(w),
            None => step(w),
        },
        schedules::synthetic(m, w).then(|| notice(NoticeKind::Preview, PREVIEW).key("policy-preview")),
        w.error.clone().map(|e| alert(e).key("policy-error")),
    ]
}

/// The coordinator's clock, on every page: the review needs it.
fn clock_line(w: &Workspace) -> Text {
    let line = match &w.support {
        Some(Ok(v)) => format!(
            "{} local clock: {} (UTC{}{hours:02}:{minutes:02}) · UTC clock: {}. Hours and \
             weekdays follow the coordinator's timezone, including future daylight-saving \
             changes.",
            v.coordinator_name,
            schedules::clock_label(&v.local_time),
            if v.clock.utc_offset_minutes < 0 {
                "−"
            } else {
                "+"
            },
            schedules::clock_label(&v.utc_time),
            hours = v.clock.utc_offset_minutes.abs() / 60,
            minutes = v.clock.utc_offset_minutes.abs() % 60
        ),
        Some(Err(e)) => e.clone(),
        None if w.query.is_some() => "Reading host support and local clock…".to_owned(),
        None => "Host support and local clock have not been read".to_owned(),
    };
    caption(line).key("policy-clock")
}

/// The list and, for the schedule chosen in it, what can be done with it.
fn browse(w: &Workspace) -> Vec<Block> {
    let policies = &w.initial.details.snapshot_policies;
    let chosen = w.selected.map(|id| id.to_string());
    let focus = w.vm.and_then(|vm| w.initial.vms.get(&vm)).map(|vm| {
        caption(format!(
            "Assign to Snapshot Schedule: {}. Choose a schedule; this VM is first in its list of \
             assignments.",
            vm.name
        ))
        .key("policy-vm")
    });
    blocks![
        actions([
            button("policy-create", "New schedule", |m| schedules::edit(
                m, true
            )),
            button("policy-refresh", "Refresh", schedules::refresh),
        ]),
        focus,
        (!policies.is_empty()).then(|| {
            choice(
                "policy-list",
                policies.values().map(|p| {
                    option(
                        p.id.to_string(),
                        format!(
                            "{} · {} · {} VMs",
                            p.name,
                            match p.enabled {
                                Some(true) => "enabled",
                                Some(false) => "disabled",
                                None => "unknown",
                            },
                            p.vms.len()
                        ),
                    )
                    .key(format!("policy-select-{}", p.id))
                }),
                chosen.as_deref(),
                schedules::select,
            )
            .label("Schedules")
        }),
        policies.is_empty().then(|| {
            caption("No snapshot schedules were returned by the host.").key("policy-empty")
        }),
        selected(w).map(|p| detail(w, p)),
    ]
}

fn detail(w: &Workspace, p: &SnapshotPolicy) -> Vec<Block> {
    let head = blocks![
        divider(),
        heading(&p.name),
        caption(format!(
            "UUID {} · {} · last run {}",
            p.id,
            p.frequency,
            p.last_run_time.as_deref().unwrap_or("unknown")
        ))
        .key("policy-detail"),
    ];
    let Ok(e) = schedules::expected(w) else {
        return blocks![
            head,
            caption(
                "This policy's settings or membership are incomplete or unsupported. \
                 Quiesced/VSS schedules are not supported."
            )
            .key("policy-unsupported"),
        ];
    };
    let mut guests: Vec<_> = w
        .initial
        .vms
        .values()
        .filter(|v| !v.is_template && !v.is_control_domain)
        .collect();
    // The VM of "Assign to Snapshot Schedule" comes first; the rest keep
    // the order of the inventory.
    guests.sort_by_key(|v| Some(v.id) != w.vm);
    let members: Vec<Button> = guests
        .into_iter()
        .filter_map(|vm| {
            let id = vm.id;
            let assigned = e.members.contains(&id);
            // A VM of another schedule is not offered: it is removed there
            // first, in a review of its own.
            let elsewhere = (w.initial.details.vms.get(&id))
                .is_some_and(|d| d.snapshot_policy.is_some());
            if !assigned && elsewhere {
                return None;
            }
            Some(
                button(
                    format!("policy-member-{id}"),
                    format!("{} {}", if assigned { "Remove" } else { "Assign" }, vm.name),
                    move |m| schedules::action(m, Action::Member(id, !assigned)),
                )
                .disabled_if(e.enabled, schedules::MEMBER_NEEDS_DISABLED),
            )
        })
        .collect();
    blocks![
        head,
        actions([
            button("policy-edit", "Edit settings", |m| schedules::edit(
                m, false
            ))
            .disabled_if(e.enabled, schedules::EDIT_NEEDS_DISABLED),
            button(
                "policy-toggle",
                if e.enabled { "Disable" } else { "Enable" },
                |m| schedules::action(m, Action::Toggle)
            ),
            button("policy-run", "Run now", |m| schedules::action(
                m,
                Action::Run
            ))
            .disabled_if(!e.enabled || e.members.is_empty(), schedules::RUN_NEEDS),
            button("policy-delete", "Delete empty schedule", |m| {
                schedules::action(m, Action::Delete)
            })
            .disabled_if(e.enabled || !e.members.is_empty(), schedules::DELETE_NEEDS),
        ]),
        caption(
            "Disable this schedule before editing settings or VM assignments. To delete it, \
             remove each reviewed VM assignment first."
        ),
        group("VM assignments", blocks![actions(members)]).key("policy-assignments"),
    ]
}

/// The draft step of the wizard that shows.
fn step(w: &Workspace) -> Vec<Block> {
    match w.step {
        0 => blocks![
            input("policy-name", "Schedule name", &w.name, |m, text| {
                if let Some(w) = workspace(m) {
                    w.name = text.to_owned();
                }
            }),
            input(
                "policy-description",
                "Description",
                &w.description,
                |m, text| {
                    if let Some(w) = workspace(m) {
                        w.description = text.to_owned();
                    }
                }
            ),
        ],
        1 => {
            let frequency = w.frequency.to_string();
            blocks![
                choice(
                    "policy-frequency",
                    ["Hourly", "Daily", "Weekly"]
                        .into_iter()
                        .enumerate()
                        .map(|(i, label)| {
                            option(i.to_string(), label).key(format!("policy-frequency-{i}"))
                        }),
                    Some(frequency.as_str()),
                    |m, id| {
                        if let (Some(w), Ok(frequency)) = (workspace(m), id.parse::<u8>()) {
                            w.frequency = frequency.min(2);
                        }
                    }
                )
                .label("Frequency")
                .segmented(),
                (w.frequency != 0).then(|| {
                    input(
                        "policy-hour",
                        "Coordinator local hour (0–23)",
                        &w.hour,
                        |m, text| {
                            if let Some(w) = workspace(m) {
                                w.hour = text.to_owned();
                            }
                        },
                    )
                }),
                input(
                    "policy-minute",
                    "Minute (0, 15, 30 or 45)",
                    &w.minute,
                    |m, text| {
                        if let Some(w) = workspace(m) {
                            w.minute = text.to_owned();
                        }
                    }
                ),
                (w.frequency == 2).then(|| {
                    WEEKDAYS
                        .iter()
                        .enumerate()
                        .map(|(i, label)| {
                            checkbox(format!("policy-day-{i}"), *label, w.days[i], move |m| {
                                if let Some(w) = workspace(m) {
                                    w.days[i] = !w.days[i];
                                }
                            })
                        })
                        .collect::<Vec<_>>()
                }),
            ]
        }
        _ => blocks![
            input(
                "policy-retention",
                "Retained scheduled snapshots per VM (1–10)",
                &w.retention,
                |m, text| {
                    if let Some(w) = workspace(m) {
                        w.retention = text.to_owned();
                    }
                }
            ),
            checkbox(
                "policy-checkpoint",
                "Include VM memory (checkpoint)",
                w.checkpoint,
                |m| {
                    if let Some(w) = workspace(m) {
                        w.checkpoint = !w.checkpoint;
                    }
                }
            ),
            caption(
                "Disk snapshots are the default. Quiesced/VSS snapshots are unavailable. \
                 Creating or editing a schedule leaves it disabled."
            ),
        ],
    }
}

fn footer(m: &Model, w: &Workspace, permission: Option<&PermissionAssessment>) -> Vec<Button> {
    let mut footer = vec![
        button("policy-close", "Close", super::close_schedules),
        // Rune closed the dialog and opened the Activity dialog; its
        // successor is the Events tab of the Notifications view.
        button("policy-history", "Operation history", |m| {
            super::close_schedules(m);
            m.show_notifications(TabId::Events);
        }),
    ];
    if w.review.is_none() && w.mode == Mode::Browse {
        return footer;
    }
    footer.push(button("policy-back", "Back", schedules::back));
    footer.push(if w.review.is_some() {
        button("policy-confirm", "Confirm change", schedules::submit)
            .danger()
            // The first reason wins: stale, then a known denial, then the demo.
            .disabled_opt(schedules::stale(m, w))
            .disabled_opt(permission.and_then(permissions::denial))
            .disabled_if(schedules::synthetic(m, w), DEMO)
    } else if w.step < LAST_DRAFT_STEP {
        button("policy-next", "Next", schedules::next).primary()
    } else {
        button("policy-review", "Review", schedules::review).primary()
    });
    footer
}
