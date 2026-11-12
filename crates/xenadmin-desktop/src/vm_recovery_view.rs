use super::*;
use xenadmin_core::{
    ForceRecoveryAction, ObservedVm, OperationAction, OperationControl, OperationId,
    OperationState, RecoveryBootConfiguration, VmRecoveryOperation,
};
/// Shown in a forced-action review that would interrupt a migration (lab finding 63).
const MIGRATION_CAUTION: &str = "A migration of this VM is running. The host admits a forced action during one and does not always end the migration afterwards: on the project's test lab its task then stayed pending and left partial disk copies that took a host restart to clear. If the client that started the migration can still cancel it, do that first and wait for it to end. This action is refused unless you accept below what it can leave behind.";
#[derive(Clone)]
enum Review {
    Cancel {
        id: OperationId,
        revision: u64,
        label: String,
    },
    Action(VmRecoveryOperation),
}
pub(super) struct Panel {
    target: Option<ObservedVm>,
    pool: PoolId,
    vm: VmId,
    name: String,
    initial: Arc<Inventory>,
    review: Option<Review>,
}
fn close(s: &mut State) {
    s.vm_recovery = None;
}
fn current(s: &State, p: &Panel) -> bool {
    s.inventory
        .as_ref()
        .is_some_and(|i| i.pool_id == p.pool && i.vms.contains_key(&p.vm))
        && (s.demo
            || p.target.is_some_and(|o| {
                s.snapshot.as_ref().and_then(|s| s.observe_vm(p.vm).ok()) == Some(o)
            }))
}
pub(super) fn open(s: &mut State, id: VmId) {
    let Some(i) = &s.inventory else { return };
    let Some(vm) = i
        .vms
        .get(&id)
        .filter(|v| !v.is_template && !v.is_control_domain)
    else {
        return;
    };
    s.vm_recovery = Some(Box::new(Panel {
        target: s.snapshot.as_ref().and_then(|s| s.observe_vm(id).ok()),
        pool: i.pool_id,
        vm: id,
        name: vm.name.clone(),
        initial: i.clone(),
        review: None,
    }));
    s.show_details = false;
}
fn refresh(s: &mut State) {
    if let Some(p) = &s.vm_recovery {
        let id = p.vm;
        open(s, id)
    }
}
pub(super) fn button(s: &State, id: VmId, k: &Kit) -> El {
    if !s.inventory.as_ref().is_some_and(|i| {
        i.vms
            .get(&id)
            .is_some_and(|v| !v.is_template && !v.is_control_domain)
    }) {
        return el();
    }
    Button::new("Recovery controls…")
        .on_press(move |s: &mut State| open(s, id))
        .build(k)
        .key("vm-recovery-open")
}
fn review(s: &mut State, r: Review) {
    if let Some(p) = &mut s.vm_recovery {
        p.review = Some(r)
    }
}
fn apply(s: &mut State) {
    let Some(p) = s.vm_recovery.as_ref() else {
        return;
    };
    if !current(s, p) || !s.can_mutate() {
        return;
    }
    let Some(target) = p.target else { return };
    let Some(review) = p.review.clone() else {
        return;
    };
    match review {
        Review::Cancel { id, revision, .. } => {
            if !s.operation_rows.iter().any(|r| {
                r.id == id
                    && r.pool == p.pool
                    && (r.vm == p.vm || r.created_vm == Some(p.vm))
                    && r.revision() == revision
                    && r.can_cancel
                    && !r.cancel_requested
                    && !r.state.terminal()
                    && r.state != OperationState::OutcomeUnknown
            }) {
                return;
            }
            s.control_operation(id, OperationControl::RequestCancellation);
            if let Some(p) = &mut s.vm_recovery {
                p.review = None;
            }
        }
        Review::Action(operation) => {
            if !s
                .inventory
                .as_ref()
                .is_some_and(|i| operation.unavailable_reason(i, p.vm).is_none())
                || operation.migration_unaccepted()
                || !session_permissions_view::check(s, &operation.permission_requirements())
            {
                return;
            }
            s.submit_command(xenadmin_core::VmCommand::Recovery {
                vm: target,
                operation,
            });
            if s.error.is_none() {
                close(s)
            }
        }
    }
}
/// Gives or withdraws, in the open review of a forced action, the operator's acceptance
/// that a captured migration may be left unfinished.
fn accept_migration(s: &mut State) {
    if let Some(Review::Action(VmRecoveryOperation::Force {
        tasks,
        interrupts_migration,
        ..
    })) = s.vm_recovery.as_mut().and_then(|p| p.review.as_mut())
        && tasks.iter().any(|t| t.is_migration())
    {
        *interrupts_migration = !*interrupts_migration;
    }
}
fn order(c: &RecoveryBootConfiguration) -> String {
    c.order
        .clone()
        .unwrap_or_else(|| "host default (order key absent)".into())
}
fn action_button(s: &State, p: &Panel, operation: VmRecoveryOperation, key: &str, k: &Kit) -> El {
    let why = s
        .inventory
        .as_ref()
        .and_then(|i| operation.unavailable_reason(i, p.vm));
    let denied = session_permissions_view::assess(s, &operation.permission_requirements())
        .decision()
        .is_denied();
    let label = operation.label();
    let button = Button::new(label)
        .disabled(!current(s, p) || why.is_some() || denied || (!s.demo && !s.can_mutate()))
        .on_press(move |s: &mut State| review(s, Review::Action(operation.clone())))
        .build(k)
        .key(key);
    let mut result = col().gap(5.0).child(button);
    if let Some(why) = why {
        result = result.child(caption(why, k).wrap())
    }
    result
}
pub(super) fn dialog(s: &State, k: &Kit) -> El {
    let Some(p) = &s.vm_recovery else { return el() };
    let width = (s.window_width - 64.0).clamp(240.0, 700.0);
    let mut body = col()
        .w(width)
        .gap(12.0)
        .child(caption(format!("{} · {}\nPool {}", p.name, p.vm, p.pool), k).wrap());
    let mut enabled = current(s, p) && s.can_mutate() && !s.demo;
    let mut permission = None;
    if !current(s, p) {
        body=body.child(caption("The captured connection changed. Close this panel and reopen it from current inventory.",k).wrap());
    }
    match &p.review {
        Some(Review::Cancel {
            id,
            revision,
            label,
        }) => {
            enabled &= s.operation_rows.iter().any(|r| {
                r.id == *id
                    && r.pool == p.pool
                    && (r.vm == p.vm || r.created_vm == Some(p.vm))
                    && r.revision() == *revision
                    && r.can_cancel
                    && !r.cancel_requested
                    && !r.state.terminal()
                    && r.state != OperationState::OutcomeUnknown
            });
            if !enabled && !s.demo {
                body=body.child(caption("The operation or connection changed. Go Back and review its current state before requesting cancellation.",k).wrap());
            }
            body=body.child(heading("Review local operation cancellation",k)).child(caption(format!("{label}\nOperation {id}"),k).wrap()).child(caption("Request cancellation through this client's operation journal. Some workflows only stop their remaining steps; an in-flight host task may still finish normally. This does not cancel another client's tasks, reset VM state, or submit a force action. Wait for the recorded outcome, then refresh and review any force action separately.",k).wrap());
        }
        Some(Review::Action(o)) => {
            permission = Some(session_permissions_view::assess(
                s,
                &o.permission_requirements(),
            ));
            enabled &= !permission.as_ref().unwrap().decision().is_denied();
            if let Some(reason) = s
                .inventory
                .as_ref()
                .and_then(|i| o.unavailable_reason(i, p.vm))
            {
                enabled = false;
                body = body.child(caption(reason, k).wrap());
            }
            body = body.child(heading(o.label(), k));
            match o {
                VmRecoveryOperation::Force { tasks, .. } => {
                    body=body.child(caption("This can interrupt guest writes and lose data. The host will freshly validate the force action even when it is missing from the host’s usual action list. Administrative blocks and session permissions still apply. No task listed here will be cancelled by this action.",k).wrap());
                    if tasks.iter().any(|t| t.is_migration()) {
                        // Refused unless the operator accepts what it can leave behind.
                        enabled &= !o.migration_unaccepted();
                        body = body
                            .child(
                                caption(MIGRATION_CAUTION, k)
                                    .wrap()
                                    .color(k.c.danger.icon)
                                    .key("recovery-migration-caution"),
                            )
                            .child(
                                Checkbox::new(!o.migration_unaccepted())
                                    .label("Force anyway: I accept that the migration may be left unfinished on the hosts")
                                    .on_toggle(accept_migration)
                                    .build(k)
                                    .key("recovery-accept-migration"),
                            );
                    }
                    for t in tasks {
                        body =
                            body.child(caption(format!("{} · {}", t.operation, t.task), k).wrap())
                    }
                }
                VmRecoveryOperation::Boot { configuration } => {
                    body=body.child(caption(format!("Saved boot order: {}\nTemporary order: CD, then network (dn)\nFirmware: {} · Secure Boot policy: {}",order(configuration),configuration.firmware,configuration.secure_boot.map(|v|if v {"enabled"}else{"disabled"}).unwrap_or("unknown")),k).wrap()).child(caption("The VM starts once using CD/network boot. Its original boot order is restored only after verified startup and only while the temporary value is unchanged. Firmware, Secure Boot policy and mounted media are preserved. This does not verify that a guest recovery environment actually started. A lost reply stops later writes and can leave the temporary order in place.",k).wrap());
                }
                VmRecoveryOperation::RestoreBoot {
                    source,
                    configuration,
                } => {
                    body=body.child(caption(format!("Source operation: {source}\nRestore boot order: {}",order(configuration)),k).wrap()).child(caption("Restore only the exact temporary order set by this completed or acknowledged recovery operation. The backend requires a saved setter completion receipt and unchanged firmware/settings. This changes next-boot configuration only; it does not start, stop or cancel anything.",k).wrap());
                }
            }
        }
        None => {
            body=body.child(heading("Current host tasks",k)).child(caption("These are the task identities captured when this panel opened. They may belong to any client. Refresh before reviewing a changed VM.",k).wrap());
            if let Some(d) = p.initial.details.vms.get(&p.vm) {
                if d.current_operations.is_empty() {
                    body = body.child(caption("No current host tasks were reported.", k));
                }
                for t in &d.current_operations {
                    let name = t
                        .task
                        .and_then(|id| p.initial.details.tasks.get(&id))
                        .map(|t| t.name.as_str())
                        .unwrap_or("Task details unavailable");
                    body = body.child(
                        caption(
                            format!(
                                "{} · {name}\n{}",
                                t.operation,
                                t.task
                                    .map(|id| id.to_string())
                                    .unwrap_or_else(|| "Task identity unavailable".into())
                            ),
                            k,
                        )
                        .wrap(),
                    );
                }
            }
            body = body.child(heading("This client's unfinished operations", k));
            let mut found = false;
            for r in s.operation_rows.iter().filter(|r| {
                r.pool == p.pool
                    && (r.vm == p.vm || r.created_vm == Some(p.vm))
                    && !r.state.terminal()
            }) {
                found = true;
                body = body.child(
                    caption(
                        format!(
                            "{} · {:?}\n{}\n{}",
                            r.action.label(),
                            r.state,
                            r.id,
                            r.detail
                        ),
                        k,
                    )
                    .wrap(),
                );
                if r.can_cancel && !r.cancel_requested && r.state != OperationState::OutcomeUnknown
                {
                    let review = Review::Cancel {
                        id: r.id,
                        revision: r.revision(),
                        label: r.action.label().into(),
                    };
                    body = body.child(
                        Button::new("Review cancellation request…")
                            .disabled(!current(s, p) || (!s.demo && !s.can_mutate()))
                            .on_press(move |s: &mut State| self::review(s, review.clone()))
                            .build(k)
                            .key(format!("recovery-cancel-{}", r.id)),
                    );
                }
            }
            if !found {
                body = body.child(caption("No unfinished local VM operations are listed.", k));
            }
            body=body.child(caption("Unresolved local operations retain their reservations. Inspect unknown outcomes in Activity before acknowledging them; cancellation requests do not release reservations or prove host cancellation.",k).wrap());
            match VmRecoveryOperation::tasks(&p.initial,p.vm){Ok(tasks)=>{for (a,key) in [(ForceRecoveryAction::Shutdown,"recovery-force-shutdown"),(ForceRecoveryAction::Reboot,"recovery-force-reboot")]{body=body.child(action_button(s,p,VmRecoveryOperation::Force{action:a,tasks:tasks.clone(),interrupts_migration:false},key,k));}},Err(_)=>body=body.child(caption("Task-aware force recovery requires a running VM with known current task identities. For an idle VM, use its normal power actions.",k).wrap())}
            match RecoveryBootConfiguration::from_inventory(&p.initial,p.vm){Ok(configuration)=>body=body.child(action_button(s,p,VmRecoveryOperation::Boot{configuration},"recovery-boot",k)),Err(_)=>body=body.child(caption("Recovery boot requires a halted, idle HVM guest. PV guests and changing installed BIOS/UEFI firmware are unsupported.",k).wrap())}
            for r in s.operation_rows.iter().filter(|r| {
                r.pool == p.pool
                    && (r.vm == p.vm || r.created_vm == Some(p.vm))
                    && r.state.terminal()
            }) {
                if let Some(operation) = r.recovery_boot_restoration() {
                    let attempted=s.operation_rows.iter().any(|other|other.id!=r.id && other.pool==p.pool && other.vm==p.vm && matches!(&other.action,OperationAction::Recovery(VmRecoveryOperation::RestoreBoot{source,..}) if *source==r.id) && other.steps.iter().any(|step|!matches!(step.state,xenadmin_core::StepState::Pending)));
                    if !attempted {
                        body = body.child(action_button(
                            s,
                            p,
                            operation,
                            &format!("recovery-restore-{}", r.id),
                            k,
                        ));
                    }
                }
            }
        }
    }
    if let Some(error) = &s.error {
        body = body.child(caption(error, k).wrap().color(k.c.danger.icon));
    }
    let mut controls = row().gap(8.0).flex_wrap().child(
        Button::new("Close")
            .on_press(close)
            .build(k)
            .key("recovery-close"),
    );
    if p.review.is_some() {
        controls = controls
            .child(
                Button::new("Back")
                    .on_press(|s: &mut State| {
                        if let Some(p) = &mut s.vm_recovery {
                            p.review = None;
                        }
                    })
                    .build(k),
            )
            .child(
                Button::new("Confirm reviewed recovery action")
                    .disabled(!enabled)
                    .on_press(apply)
                    .build(k)
                    .key("recovery-confirm"),
            );
    } else {
        controls = controls.child(
            Button::new("Refresh captured state")
                .on_press(refresh)
                .build(k)
                .key("recovery-refresh"),
        );
    }
    let panel = col()
        .w(width + 32.0)
        .h((s.window_height - 32.0).clamp(240.0, 820.0))
        .padding(16.0)
        .gap(12.0)
        .radius(10.0)
        .fill(k.c.bg_base)
        .child(heading("VM recovery controls", k))
        .child(body.min_h(0.0).grow(1.0).scroll(rune::ScrollSpec {
            stick_bottom: false,
        }))
        .child(session_permissions_view::footer(
            controls,
            permission.as_ref(),
            width,
            k,
        ));
    el().w_full()
        .h_full()
        .absolute(0.0, 0.0)
        .center()
        .fill(rune::Color::BLACK.alpha(0.5))
        .role("dialog")
        .label("VM recovery controls")
        .modal(false, close)
        .child(panel)
}
pub(super) fn scene(s: &mut State) {
    demo(s);
    let Some(id) = s
        .inventory
        .as_ref()
        .and_then(|i| {
            i.vms.values().find(|v| {
                !v.is_template && !v.is_control_domain && v.power_state == PowerState::Halted
            })
        })
        .map(|v| v.id)
    else {
        return;
    };
    Arc::make_mut(s.inventory.as_mut().unwrap())
        .vms
        .get_mut(&id)
        .unwrap()
        .allowed_actions = vec![xenadmin_core::PowerAction::Start];
    s.table.select(id.to_string());
    Arc::make_mut(s.inventory.as_mut().unwrap())
        .details
        .vms
        .get_mut(&id)
        .unwrap()
        .domain_type = Some("hvm".into());
    open(s, id);
    if let Some(p) = &mut s.vm_recovery
        && let Ok(configuration) = RecoveryBootConfiguration::from_inventory(&p.initial, id)
    {
        p.review = Some(Review::Action(VmRecoveryOperation::Boot { configuration }));
    }
}
pub(super) fn force_scene(s: &mut State) {
    scene(s);
    let id = s.vm_recovery.as_ref().unwrap().vm;
    let i = Arc::make_mut(s.inventory.as_mut().unwrap());
    let v = i.vms.get_mut(&id).unwrap();
    v.power_state = PowerState::Running;
    v.allowed_actions.clear();
    i.details.vms.get_mut(&id).unwrap().current_operations =
        vec![xenadmin_core::RunningOperation {
            task: Some(xenadmin_core::TaskId::from(uuid::Uuid::from_u128(900))),
            operation: "export".into(),
        }];
    open(s, id);
    let p = s.vm_recovery.as_mut().unwrap();
    let tasks = VmRecoveryOperation::tasks(&p.initial, id).unwrap();
    p.review = Some(Review::Action(VmRecoveryOperation::Force {
        action: ForceRecoveryAction::Shutdown,
        tasks,
        interrupts_migration: false,
    }));
}
#[cfg(test)]
#[path = "vm_recovery_view_tests.rs"]
mod tests;
