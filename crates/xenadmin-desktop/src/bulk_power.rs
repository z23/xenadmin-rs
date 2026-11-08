use super::*;
use xenadmin_core::{
    BatchVmPower, ObservedResource, PowerAction, ResourceCommand, ResourceTarget, VmPowerBatch,
};
pub(super) struct Editor {
    pool: PoolId,
    pool_name: String,
    observed: Option<ObservedResource>,
    vms: Vec<Vm>,
    action: PowerAction,
    review: Option<VmPowerBatch>,
    force_ack: bool,
    error: Option<String>,
}
fn edit(s: &mut State) -> &mut Editor {
    s.bulk_power.as_mut().expect("batch editor")
}
fn selection(s: &State) -> Vec<Vm> {
    if s.page != Page::Guests {
        return Vec::new();
    }
    s.rows
        .iter()
        .filter(|r| s.table.is_selected(&r.id))
        .filter_map(|r| r.vm)
        .filter_map(|id| s.inventory.as_ref()?.vms.get(&id))
        .filter(|vm| !vm.is_template && !vm.is_control_domain)
        .cloned()
        .collect()
}
pub(super) fn open(s: &mut State) {
    let vms = selection(s);
    let Some(inventory) = &s.inventory else {
        return;
    };
    if vms.is_empty() || vms.len() > 32 {
        s.error = Some("Select between 1 and 32 visible VMs for a power batch".into());
        return;
    }
    let action = PowerAction::ALL
        .into_iter()
        .find(|action| {
            vms.iter().all(|v| {
                inventory
                    .details
                    .vms
                    .get(&v.id)
                    .is_some_and(|d| d.power_availability(*action, true).is_available())
            })
        })
        .unwrap_or(PowerAction::Start);
    s.bulk_power = Some(Editor {
        pool: inventory.pool_id,
        pool_name: inventory.pool_name.clone(),
        observed: s.snapshot.as_ref().and_then(|snapshot| {
            snapshot
                .observe_resource(ResourceTarget::Pool(inventory.pool_id))
                .ok()
        }),
        vms,
        action,
        review: None,
        force_ack: false,
        error: None,
    });
}
fn current(s: &State, e: &Editor) -> bool {
    !s.demo
        && s.can_mutate()
        && e.observed.is_some()
        && s.snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.observe_resource(ResourceTarget::Pool(e.pool)).ok())
            == e.observed
}
fn unavailable(s: &State, e: &Editor, action: PowerAction) -> Vec<String> {
    e.vms
        .iter()
        .filter_map(|vm| {
            let inventory = s.inventory.as_ref().filter(|i| i.pool_id == e.pool);
            let now = inventory.and_then(|i| i.vms.get(&vm.id));
            let explanation = if s
                .snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.power_permission(action).is_denied())
            {
                Some(
                    xenadmin_core::PermissionDecision::Denied
                        .explanation()
                        .into(),
                )
            } else if now.is_none_or(|v| {
                v.is_template || v.is_control_domain || v.power_state != vm.power_state
            }) {
                Some("VM identity or power state changed; reopen the selection".into())
            } else {
                inventory
                    .and_then(|i| i.details.vms.get(&vm.id))
                    .map(|d| d.power_availability(action, s.demo || current(s, e)))
                    .filter(|a| !a.is_available())
                    .map(|a| a.explanation())
                    .or_else(|| {
                        inventory
                            .filter(|i| !i.details.vms.contains_key(&vm.id))
                            .map(|_| "Action availability is unknown".into())
                    })
            };
            explanation.map(|reason| format!("{}: {reason}", vm.name))
        })
        .collect()
}
fn review(s: &mut State) {
    let Some(e) = s.bulk_power.as_ref() else {
        return;
    };
    let problems = unavailable(s, e, e.action);
    if !problems.is_empty() {
        edit(s).error = Some(problems.join("\n"));
        return;
    }
    let e = edit(s);
    e.review = Some(VmPowerBatch {
        action: e.action,
        vms: e
            .vms
            .iter()
            .map(|vm| BatchVmPower {
                vm: vm.id,
                expected_state: vm.power_state.clone(),
            })
            .collect(),
    });
    e.force_ack = false;
}
fn submit(s: &mut State) {
    let result = (|| {
        let e = s.bulk_power.as_ref().ok_or(xenadmin_core::Error::Stale)?;
        let selection = e.review.clone().ok_or(xenadmin_core::Error::Stale)?;
        if !current(s, e)
            || !unavailable(s, e, selection.action).is_empty()
            || selection.action.is_force() && !e.force_ack
        {
            return Err(xenadmin_core::Error::Stale);
        }
        s.connection
            .as_ref()
            .ok_or(xenadmin_core::Error::Closed)?
            .submit_resource(
                xenadmin_core::OperationId::new(),
                ResourceCommand::PowerBatch {
                    pool: e.observed.ok_or(xenadmin_core::Error::Stale)?,
                    selection,
                },
            )
    })();
    match result {
        Ok(operation) => {
            if let Some(waker) = &s.waker {
                operation.notify_on_change(waker.clone());
            }
            s.resource_operations.push(operation);
            s.bulk_power = None;
        }
        Err(error) => edit(s).error = Some(error.to_string()),
    }
}
pub(super) fn selection_bar(s: &State, k: &Kit) -> El {
    let count = selection(s).len();
    row()
        .gap(10.0)
        .center_y()
        .child(
            caption(
                if count > 32 {
                    format!("{count} selected · Narrow the selection to at most 32 VMs")
                } else {
                    format!("{count} selected · Shift/Ctrl-click to select VMs")
                },
                k,
            )
            .wrap()
            .grow(1.0),
        )
        .child(
            Button::new("Power selection…")
                .disabled(count == 0 || count > 32)
                .on_press(open)
                .build(k)
                .key("batch-power-open"),
        )
}
pub(super) fn dialog(s: &State, k: &Kit) -> El {
    let e = s.bulk_power.as_ref().unwrap();
    let mut body = col()
        .gap(10.0)
        .padding(20.0)
        .w((s.window_width - 24.0).clamp(280.0, 760.0))
        .max_h((s.window_height - 24.0).max(280.0))
        .fill(k.c.bg_base)
        .radius(12.0)
        .child(heading(
            if e.review.is_some() {
                "Review VM power batch"
            } else {
                "Power actions for selected VMs"
            },
            k,
        ))
        .child(
            caption(
                format!(
                    "{} · {}\n{} VMs captured in this pool",
                    e.pool_name,
                    e.pool,
                    e.vms.len()
                ),
                k,
            )
            .wrap_anywhere(),
        );
    for vm in &e.vms {
        body = body.child(
            caption(format!("{} · {}\n{}", vm.name, vm.power_state, vm.id), k).wrap_anywhere(),
        );
    }
    if e.review.is_none() {
        for action in PowerAction::ALL {
            let reasons = unavailable(s, e, action);
            body = body.child(
                Checkbox::new(e.action == action)
                    .label(action.label())
                    .disabled(!reasons.is_empty())
                    .on_toggle(move |s: &mut State| {
                        edit(s).action = action;
                        edit(s).error = None;
                    })
                    .build(k)
                    .key(format!("batch-action-{}", action.label())),
            );
            if !reasons.is_empty() {
                body = body.child(
                    caption(
                        format!("Unavailable for {} VM(s): {}", reasons.len(), reasons[0]),
                        k,
                    )
                    .wrap(),
                );
            }
        }
    } else {
        body = body.child(heading(format!("{} {} VMs",e.action.label(),e.vms.len()),k))
            .child(caption("VMs are processed one at a time. A failed or uncertain result stops the remaining batch. Completed actions are kept; there is no automatic rollback. Other changes in this pool are blocked until this workflow is resolved.",k).wrap())
            .child(caption("Stopping the batch waits for the current server task and stops remaining VMs. Recovery inspects recorded outcomes and never repeats power actions.",k).wrap());
        if e.action.is_force() {
            body = body
                .child(
                    caption(
                        "Force actions can lose unsaved guest data on every listed VM.",
                        k,
                    )
                    .color(k.c.danger.icon)
                    .wrap(),
                )
                .child(
                    Checkbox::new(e.force_ack)
                        .label("I accept the risk for every listed VM")
                        .on_toggle(|s: &mut State| edit(s).force_ack = !edit(s).force_ack)
                        .build(k)
                        .key("batch-force-ack"),
                );
        }
    }
    if !s.demo && !current(s, e) {
        body = body.child(caption("Connection changed or inventory is stale. Reopen the selection after reconnecting.",k).wrap());
    }
    if let Some(error) = &e.error {
        body = body.child(caption(error, k).color(k.c.danger.icon).wrap());
    }
    let buttons = row().gap(8.0).justify_end().child(
        Button::new("Close")
            .on_press(|s: &mut State| s.bulk_power = None)
            .build(k),
    );
    body = body.child(if e.review.is_some() {
        buttons
            .child(
                Button::new("Back")
                    .on_press(|s: &mut State| edit(s).review = None)
                    .build(k),
            )
            .child(
                Button::new("Run batch")
                    .disabled(
                        !current(s, e)
                            || !unavailable(s, e, e.action).is_empty()
                            || e.action.is_force() && !e.force_ack,
                    )
                    .on_press(submit)
                    .build(k)
                    .key("batch-power-submit"),
            )
    } else {
        buttons.child(
            Button::new("Review batch")
                .disabled(!unavailable(s, e, e.action).is_empty())
                .on_press(review)
                .build(k)
                .key("batch-power-review"),
        )
    });
    el().w_full()
        .h_full()
        .absolute(0.0, 0.0)
        .center()
        .fill(rune::Color::BLACK.alpha(0.55))
        .role("dialog")
        .label("VM power batch")
        .modal(false, |s: &mut State| s.bulk_power = None)
        .child(body.scroll(rune::ScrollSpec {
            stick_bottom: false,
        }))
}

pub(super) fn scene(s: &mut State) {
    demo(s);
    if let Some(row) = s.rows.first() {
        s.table.select(row.id.clone());
    }
    open(s);
    let first = edit(s).vms[0].clone();
    if let Some(other) = s
        .inventory
        .as_ref()
        .and_then(|i| {
            i.guests()
                .find(|v| v.id != first.id && v.power_state == first.power_state)
        })
        .cloned()
    {
        edit(s).vms.push(other);
    }
    review(s);
}

#[cfg(test)]
#[path = "bulk_power_tests.rs"]
mod tests;
