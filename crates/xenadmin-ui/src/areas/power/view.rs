//! The open surfaces, described. Nothing here changes the model: each is
//! rebuilt on every render from its editor and the model as they are.
//!
//! Rune keys kept: `batch-action-<label>` (now the options of one choice),
//! `batch-force-ack`, `batch-power-review`, `batch-power-submit`,
//! `recovery-close`, `recovery-confirm`, `recovery-accept-migration`,
//! `recovery-migration-caution`.
//!
//! New keys, for elements Rune left without one:
//!
//! - confirmation: `power-confirm-target`, `power-confirm-consequence`,
//!   `power-confirm-cancel`, `power-confirm-submit`;
//! - unknown outcome: `unknown-target`, `unknown-consequence`,
//!   `unknown-cancel`, `unknown-acknowledge`;
//! - placement: `placement-target`, `placement-hosts` with
//!   `placement-host-<id>`, `placement-review` (and so
//!   `placement-review-stale`), `placement-error`, `placement-cancel`,
//!   `placement-back`, `placement-confirm`;
//! - recovery: `recovery-target`, `recovery-review` (and so
//!   `recovery-review-stale`), `recovery-host-tasks`, `recovery-error`;
//! - batch: `batch-pool`, `batch-vms`, `batch-action`, `batch-review` (and
//!   so `batch-review-stale`), `batch-stale`, `batch-error`, `batch-close`,
//!   `batch-back`.

use xenadmin_core::{PowerAction, VmRecoveryOperation};

use super::batch::{self, Editor};
use super::placement::{self, Placement};
use super::recovery::{self, Panel, Review};
use super::single::{self, Confirm, Unknown};
use super::synthetic;
use crate::prelude::*;

/// Back to front, in the precedence of the Rune dialog chain: the
/// acknowledgement, the confirmation, placement, recovery; the batch was a
/// layer over the chain.
pub fn surfaces(m: &Model) -> Vec<Surface> {
    let state = &m.areas.power;
    let mut surfaces = Vec::new();
    surfaces.extend(state.unknown.as_ref().map(unknown));
    surfaces.extend(state.confirm.as_ref().map(|c| confirm(m, c)));
    surfaces.extend(state.placement.as_ref().map(|p| placement(m, p)));
    surfaces.extend(state.recovery.as_deref().map(|p| recovery(m, p)));
    surfaces.extend(state.batch.as_ref().map(|e| batch(m, e)));
    surfaces
}

fn confirm(m: &Model, c: &Confirm) -> Surface {
    let force = c.action.is_force();
    let submit = button(
        "power-confirm-submit",
        c.action.label(),
        single::confirm,
    );
    let submit = if force {
        submit.danger()
    } else {
        submit.primary()
    };
    Surface::dialog(
        "power-confirm",
        if force {
            "Confirm force operation"
        } else {
            "Confirm power operation"
        },
    )
    .small()
    .body(blocks![
        heading(format!("{} this VM?", c.action.label())),
        caption(format!("Target: {}\nUUID: {}", c.name, c.target)).key("power-confirm-target"),
        caption(if force {
            single::FORCE
        } else {
            single::ROUTINE
        })
        .key("power-confirm-consequence"),
    ])
    .footer([
        button("power-confirm-cancel", "Cancel", single::cancel_confirm),
        submit.disabled_opt(single::confirm_blocked(m, c)),
    ])
    .on_close(single::cancel_confirm)
}

fn unknown(u: &Unknown) -> Surface {
    Surface::dialog("unknown-outcome", "Acknowledge unknown outcome")
        .small()
        .body(blocks![
            heading("Acknowledge unknown outcome?"),
            caption(format!("VM: {}", u.name)).key("unknown-target"),
            caption(single::ACKNOWLEDGE).key("unknown-consequence"),
        ])
        .footer([
            button("unknown-cancel", "Cancel", single::cancel_unknown),
            button(
                "unknown-acknowledge",
                "Acknowledge outcome",
                single::acknowledge,
            )
            .danger(),
        ])
        .on_close(single::cancel_unknown)
}

fn placement(m: &Model, p: &Placement) -> Surface {
    let permission = placement::requirements(p)
        .map(|required| permissions::assess_for(m, p.pool, &required));
    let step = match &p.host {
        Some((host, name)) => blocks![
            review(
                "placement-review",
                [
                    format!("Destination: {name}"),
                    format!("Host UUID: {host}"),
                    placement::PREREQUISITES.to_owned(),
                ],
            )
            .stale(placement::stale(m, p))
        ],
        // The servers of the VM's pool; a disabled one is listed and
        // cannot be chosen. Choosing one turns the dialog to its review.
        None => blocks![m.inventory_for(p.pool).map(|inventory| {
            choice(
                "placement-hosts",
                inventory.hosts.values().map(|host| {
                    option(host.id.to_string(), &host.name)
                        .key(format!("placement-host-{}", host.id))
                        .disabled_if(!host.enabled, placement::HOST_DISABLED)
                }),
                None,
                placement::choose,
            )
            .label("Destination server")
        })],
    };
    let mut footer = vec![button("placement-cancel", "Cancel", placement::close)];
    if p.host.is_some() {
        footer.push(button(
            "placement-back",
            "Choose another host",
            placement::choose_another,
        ));
        footer.push(
            button("placement-confirm", p.action.label(), placement::submit)
                .primary()
                .disabled_opt(placement::stale(m, p))
                .disabled_opt(permission.as_ref().and_then(permissions::denial))
                .disabled_opt(placement::not_changeable(m, p)),
        );
    }
    let surface = Surface::dialog("vm-placement", "Choose a destination host")
        .experimental(true)
        .body(blocks![
            heading(format!("{} on a selected host", p.action.label())),
            caption(format!("{}\nVM UUID: {}", p.name, p.vm)).key("placement-target"),
            step,
            m.error
                .clone()
                .map(|error| alert(error).key("placement-error")),
        ])
        .footer(footer)
        .on_close(placement::close);
    match &permission {
        Some(permission) => surface.footer_note(permissions::footer_note(permission)),
        None => surface,
    }
}

fn recovery(m: &Model, p: &Panel) -> Surface {
    let demo = synthetic(m, p.pool);
    let blocked = recovery::confirm_blocked(m, p);
    let stale = (!recovery::current(m, p)).then(|| recovery::STALE.to_owned());
    let unavailable = match &p.review {
        Review::Action(operation) => m
            .inventory_for(p.pool)
            .and_then(|i| operation.unavailable_reason(&i, p.vm)),
        Review::Cancel { .. } => None,
    };
    let permission = match &p.review {
        Review::Action(operation) => Some(permissions::assess_for(
            m,
            p.pool,
            &operation.permission_requirements(),
        )),
        Review::Cancel { .. } => None,
    };
    let reviewed = |lines: Vec<String>, stale: Option<String>| {
        review("recovery-review", lines)
            .warnings(unavailable.clone())
            .stale(stale)
    };
    let step = match &p.review {
        Review::Cancel {
            id,
            revision,
            label,
        } => {
            // Outside the demo, a record that moved on says so (in the demo
            // the confirm button is disabled anyway and Rune said nothing).
            let moved =
                (!demo && !recovery::cancellable(m, p, *id, *revision)).then(|| recovery::CANCEL_CHANGED.to_owned());
            blocks![
                heading("Review local operation cancellation"),
                reviewed(
                    vec![
                        format!("{label}\nOperation {id}"),
                        recovery::CANCEL.to_owned()
                    ],
                    stale.or(moved),
                ),
            ]
        }
        Review::Action(operation) => {
            let lines = match operation {
                VmRecoveryOperation::Force { tasks, .. } => std::iter::once(recovery::FORCE.to_owned())
                    .chain(tasks.iter().map(|t| format!("{} · {}", t.operation, t.task)))
                    .collect(),
                VmRecoveryOperation::Boot { configuration } => vec![
                    format!("Saved boot order: {}", recovery::order(configuration)),
                    "Temporary order: CD, then network (dn)".to_owned(),
                    format!(
                        "Firmware: {} · Secure Boot policy: {}",
                        configuration.firmware,
                        configuration
                            .secure_boot
                            .map(|v| if v { "enabled" } else { "disabled" })
                            .unwrap_or("unknown")
                    ),
                    recovery::BOOT.to_owned(),
                ],
                VmRecoveryOperation::RestoreBoot {
                    source,
                    configuration,
                } => vec![
                    format!("Source operation: {source}"),
                    format!("Restore boot order: {}", recovery::order(configuration)),
                    recovery::RESTORE.to_owned(),
                ],
            };
            // Refused unless the operator accepts what it can leave behind.
            let acceptance = recovery::over_migration(operation).then(|| {
                blocks![
                    notice(NoticeKind::Danger, recovery::MIGRATION_CAUTION)
                        .key("recovery-migration-caution"),
                    checkbox(
                        "recovery-accept-migration",
                        recovery::ACCEPT_MIGRATION,
                        !operation.migration_unaccepted(),
                        recovery::accept_migration,
                    ),
                ]
            });
            blocks![
                heading(operation.label()),
                reviewed(lines, stale),
                acceptance
            ]
        }
    };
    // A forced action lists its captured tasks in the review itself.
    let tasks = (!matches!(
        &p.review,
        Review::Action(VmRecoveryOperation::Force { .. })
    ))
    .then(|| {
        group(
            "Current host tasks",
            blocks![
                caption(recovery::TASKS_NOTE),
                recovery::host_tasks(&p.initial, p.vm)
                    .into_iter()
                    .map(caption)
                    .collect::<Vec<_>>(),
            ],
        )
        .key("recovery-host-tasks")
    });
    let confirm = button(
        "recovery-confirm",
        "Confirm reviewed recovery action",
        recovery::apply,
    );
    let confirm = if matches!(
        &p.review,
        Review::Action(VmRecoveryOperation::Force { .. })
    ) {
        confirm.danger()
    } else {
        confirm.primary()
    };
    let surface = Surface::dialog("vm-recovery", "VM recovery controls")
        .experimental(true)
        .body(blocks![
            caption(format!("{} · {}\nPool {}", p.name, p.vm, p.pool)).key("recovery-target"),
            step,
            tasks,
            m.error
                .clone()
                .map(|error| alert(error).key("recovery-error")),
        ])
        .footer([
            button("recovery-close", "Close", recovery::close),
            confirm.disabled_opt(blocked),
        ])
        .on_close(recovery::close);
    match &permission {
        Some(permission) => surface.footer_note(permissions::footer_note(permission)),
        None => surface,
    }
}

fn batch(m: &Model, e: &Editor) -> Surface {
    let stale = (!synthetic(m, e.pool) && !batch::current(m, e)).then(|| batch::STALE.to_owned());
    let force = e.action.is_force();
    let step = if e.review.is_some() {
        let reviewed = review("batch-review", [batch::SEQUENCE, batch::STOPPING])
            .warnings(force.then_some(batch::FORCE_RISK))
            .stale(stale);
        blocks![
            heading(format!("{} {} VMs", e.action.label(), e.vms.len())),
            reviewed,
            force.then(|| checkbox(
                "batch-force-ack",
                "I accept the risk for every listed VM",
                e.force_ack,
                batch::toggle_force_ack,
            )),
        ]
    } else {
        // One choice of the nine actions; an action some VM is unavailable
        // for cannot be chosen and says for how many and why.
        let options = PowerAction::ALL.into_iter().map(|action| {
            let item = option(action.label(), action.label())
                .key(format!("batch-action-{}", action.label()));
            match batch::summary(&batch::unavailable(m, e, action)) {
                Some(reason) => item.detail(reason.clone()).disabled_if(true, reason),
                None => item,
            }
        });
        blocks![
            choice(
                "batch-action",
                options,
                Some(e.action.label()),
                |m, label| {
                    if let Some(action) = PowerAction::ALL
                        .into_iter()
                        .find(|action| action.label() == label)
                    {
                        batch::choose(m, action);
                    }
                }
            )
            .label("Power action"),
            stale.map(|stale| caption(stale).key("batch-stale")),
        ]
    };
    let close = button("batch-close", "Close", batch::close);
    let footer = if e.review.is_some() {
        let run = button("batch-power-submit", "Run batch", batch::submit);
        let run = if force { run.danger() } else { run.primary() };
        vec![
            close,
            button("batch-back", "Back", batch::back),
            run.disabled_opt(batch::blocked(m, e)),
        ]
    } else {
        vec![
            close,
            button("batch-power-review", "Review batch", batch::review)
                .primary()
                .disabled_opt(batch::summary(&batch::unavailable(m, e, e.action))),
        ]
    };
    Surface::dialog("power-batch", "VM power batch")
        .large()
        .body(blocks![
            heading(if e.review.is_some() {
                "Review VM power batch"
            } else {
                "Power actions for selected VMs"
            }),
            caption(format!(
                "{} · {}\n{} VMs captured in this pool",
                e.pool_name,
                e.pool,
                e.vms.len()
            ))
            .key("batch-pool"),
            table(
                "batch-vms",
                [
                    column("name", "Name"),
                    column("state", "Power state"),
                    column("id", "UUID"),
                ],
                e.vms.iter().map(|vm| {
                    row(
                        vm.id,
                        [
                            vm.name.clone(),
                            vm.power_state.to_string(),
                            vm.id.to_string(),
                        ],
                    )
                }),
            ),
            step,
            e.error.clone().map(|error| alert(error).key("batch-error")),
        ])
        .footer(footer)
        .on_close(batch::close)
}
