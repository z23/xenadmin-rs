use super::*;
use std::collections::BTreeSet;
use xenadmin_core::{
    BatchVmWorkflow, ObservedResource, ResourceCommand, ResourceTarget, SnapshotDeletionSelection,
    SnapshotId, VdiId, VmLifecycleOperation, VmWorkflow, VmWorkflowBatch,
};

pub(super) struct Editor {
    pool: PoolId,
    pool_name: String,
    observed: Option<ObservedResource>,
    initial: Arc<Inventory>,
    choices: Vec<Choice>,
    review: Option<Review>,
    confirm: TextEdit,
    error: Option<String>,
}
struct Choice {
    vm: Vm,
    disks: BTreeSet<VdiId>,
    snapshots: BTreeSet<SnapshotId>,
    snapshot_disks: BTreeSet<VdiId>,
}
#[derive(Clone, PartialEq)]
struct Review {
    batch: VmWorkflowBatch,
    rows: Vec<Vec<String>>,
}
fn edit(s: &mut State) -> &mut Editor {
    s.bulk_delete.as_mut().expect("delete editor")
}
fn selected(s: &State) -> Result<Vec<Vm>, String> {
    if !matches!(s.page, Page::Guests | Page::Templates) {
        return Err("Select VMs or custom templates".into());
    }
    let i = s
        .inventory
        .as_ref()
        .ok_or("Current inventory is unavailable")?;
    let rows = s
        .rows
        .iter()
        .filter(|r| s.table.is_selected(&r.id))
        .collect::<Vec<_>>();
    if !(1..=32).contains(&rows.len()) {
        return Err("Select between 1 and 32 visible VMs or custom templates".into());
    }
    rows.into_iter()
        .map(|r| {
            r.vm.and_then(|id| i.vms.get(&id))
                .filter(|vm| !vm.is_control_domain)
                .cloned()
                .ok_or_else(|| "A selected row is no longer a VM or custom template".into())
        })
        .collect()
}
pub(super) fn open(s: &mut State) {
    let vms = match selected(s) {
        Ok(v) => v,
        Err(e) => {
            s.error = Some(e);
            return;
        }
    };
    let i = s.inventory.as_ref().unwrap();
    s.bulk_delete = Some(Editor {
        pool: i.pool_id,
        pool_name: i.pool_name.clone(),
        initial: i.clone(),
        observed: s
            .snapshot
            .as_ref()
            .and_then(|x| x.observe_resource(ResourceTarget::Pool(i.pool_id)).ok()),
        choices: vms
            .into_iter()
            .map(|vm| Choice {
                vm,
                disks: BTreeSet::new(),
                snapshots: BTreeSet::new(),
                snapshot_disks: BTreeSet::new(),
            })
            .collect(),
        review: None,
        confirm: TextEdit::new(""),
        error: None,
    });
}
fn current(s: &State, e: &Editor) -> bool {
    !s.demo
        && s.can_mutate()
        && e.observed.is_some()
        && s.snapshot
            .as_ref()
            .and_then(|x| x.observe_resource(ResourceTarget::Pool(e.pool)).ok())
            == e.observed
}
fn single(i: &Inventory, c: &Choice) -> VmLifecycleOperation {
    VmLifecycleOperation::Delete {
        expected_template: c.vm.is_template,
        disks: c.disks.iter().copied().collect(),
        snapshots: c
            .snapshots
            .iter()
            .map(|id| SnapshotDeletionSelection {
                snapshot: *id,
                disks: i
                    .snapshot_deletion_disks(*id)
                    .into_iter()
                    .filter(|id| c.snapshot_disks.contains(id))
                    .collect(),
            })
            .collect(),
    }
}
fn build_review(s: &State, e: &Editor) -> Result<Review, String> {
    let i = s
        .inventory
        .as_ref()
        .filter(|i| i.pool_id == e.pool)
        .ok_or("The captured pool is no longer current")?;
    let mut items = Vec::new();
    let mut rows = Vec::new();
    let mut errors = Vec::new();
    for c in &e.choices {
        let Some(vm) = i.vms.get(&c.vm.id).filter(|vm| {
            !vm.is_control_domain
                && vm.is_template == c.vm.is_template
                && vm.power_state == c.vm.power_state
        }) else {
            errors.push(format!(
                "{} ({}): identity or power state changed",
                c.vm.name, c.vm.id
            ));
            continue;
        };
        let op = single(i, c);
        let initial = single(&e.initial, c);
        let availability = op.availability(i, vm.id);
        if !availability.is_available() {
            errors.push(format!(
                "{} ({}): {}",
                vm.name,
                vm.id,
                availability.explanation()
            ));
            continue;
        }
        if initial != op {
            errors.push(format!("{}: snapshot disk selection changed", vm.name));
            continue;
        }
        let VmLifecycleOperation::Delete {
            expected_template,
            disks,
            snapshots,
        } = op
        else {
            unreachable!()
        };
        items.push(BatchVmWorkflow {
            vm: vm.id,
            expected_state: vm.power_state.clone(),
            workflow: VmWorkflow::Delete {
                expected_template,
                disks,
                snapshots,
            },
        });
        rows.push(vm_lifecycle::deletion_review(
            i,
            vm,
            &c.disks,
            &c.snapshots,
            &c.snapshot_disks,
        ));
    }
    if !errors.is_empty() {
        return Err(errors.join("\n"));
    }
    let batch = VmWorkflowBatch { items };
    let permission = session_permissions_view::assess(s, &batch.permission_requirements());
    if permission.decision().is_denied() {
        return Err(xenadmin_core::PermissionDecision::Denied
            .explanation()
            .into());
    }
    Ok(Review { batch, rows })
}
fn review(s: &mut State) {
    let Some(e) = &s.bulk_delete else { return };
    match build_review(s, e) {
        Ok(r) => {
            let e = edit(s);
            e.review = Some(r);
            e.confirm.text.clear();
            e.error = None;
        }
        Err(error) => edit(s).error = Some(error),
    }
}
fn submit(s: &mut State) {
    let result = (|| {
        let e = s.bulk_delete.as_ref().ok_or(xenadmin_core::Error::Stale)?;
        let reviewed = e.review.as_ref().ok_or(xenadmin_core::Error::Stale)?;
        if !current(s, e)
            || e.confirm.text != format!("DELETE {}", e.choices.len())
            || build_review(s, e).ok().as_ref() != Some(reviewed)
            || s.resource_operations.len() >= 64
        {
            return Err(xenadmin_core::Error::Stale);
        }
        s.connection
            .as_ref()
            .ok_or(xenadmin_core::Error::Closed)?
            .submit_resource(
                xenadmin_core::OperationId::new(),
                ResourceCommand::VmWorkflowBatch {
                    pool: e.observed.ok_or(xenadmin_core::Error::Stale)?,
                    selection: reviewed.batch.clone(),
                },
            )
    })();
    match result {
        Ok(op) => {
            if let Some(w) = &s.waker {
                op.notify_on_change(w.clone());
            }
            s.resource_operations.push(op);
            s.bulk_delete = None;
            resource_admin::open_pool(s, resource_admin::Kind::History);
        }
        Err(e) => edit(s).error = Some(e.to_string()),
    }
}
pub(super) fn button(s: &State, k: &Kit) -> El {
    Button::new("Delete selection…")
        .variant(ButtonVariant::Ghost)
        .disabled(selected(s).is_err())
        .on_press(open)
        .build(k)
        .key("batch-delete-open")
}
pub(super) fn dialog(s: &State, k: &Kit) -> El {
    let e = s.bulk_delete.as_ref().unwrap();
    let mut body = col().w_full().gap(12.0).child(
        caption(
            format!(
                "{} · {}\n{} captured targets",
                e.pool_name,
                e.pool,
                e.choices.len()
            ),
            k,
        )
        .wrap_anywhere(),
    );
    if let Some(r) = &e.review {
        for (n, lines) in r.rows.iter().enumerate() {
            let mut card = col()
                .gap(6.0)
                .padding(10.0)
                .fill(k.c.bg_base)
                .radius(6.0)
                .child(heading(format!("Target {}", n + 1), k));
            for line in lines {
                card = card.child(caption(line, k).wrap_anywhere());
            }
            body = body.child(card);
        }
        body=body.child(caption("Every selected target is checked before the first deletion. Targets run serially. Stop lets the current VM workflow finish, then skips the rest. An interruption never starts an unsubmitted target; partial deletions cannot be rolled back.",k).wrap())
            .child(caption(format!("Type DELETE {} to confirm irreversible deletion.",e.choices.len()),k).wrap())
            .child(TextInput::new(&e.confirm).label("Deletion confirmation").on_edit(|s:&mut State|&mut edit(s).confirm).build(k).key("batch-delete-confirm"));
    } else {
        body=body.child(caption("By default, delete only each VM/template record and its device attachments. Disks and snapshots are retained unless individually selected below.",k).wrap());
        for (index, c) in e.choices.iter().enumerate() {
            body = body
                .child(heading(&c.vm.name, k))
                .child(caption(format!("{} · {}", c.vm.id, c.vm.power_state), k).wrap_anywhere());
            let candidates = e.initial.vm_deletion_disks(c.vm.id);
            let disks = e
                .initial
                .details
                .vbds
                .values()
                .filter(|b| b.vm == Some(c.vm.id) && b.kind == "Disk")
                .filter_map(|b| b.vdi)
                .collect::<BTreeSet<_>>();
            for id in disks {
                let allowed = candidates.contains(&id);
                body = body.child(
                    Checkbox::new(c.disks.contains(&id))
                        .label(format!(
                            "Delete disk data: {}",
                            vm_lifecycle::disk_label(&e.initial, id)
                        ))
                        .disabled(!allowed)
                        .on_toggle(move |s: &mut State| {
                            let c = &mut edit(s).choices[index];
                            if !c.disks.remove(&id) {
                                c.disks.insert(id);
                            }
                        })
                        .build(k)
                        .key(format!("batch-delete-disk-{id}")),
                );
                if !allowed {
                    body = body.child(
                        caption("Retained: exclusive ownership is not established.", k).wrap(),
                    );
                }
            }
            for snapshot in e.initial.snapshots.for_vm(c.vm.id) {
                let id = snapshot.id;
                let allowed = snapshot.power_state == PowerState::Halted
                    && snapshot
                        .allowed_actions
                        .contains(&xenadmin_core::SnapshotAction::Delete);
                body = body.child(
                    Checkbox::new(c.snapshots.contains(&id))
                        .label(format!("Delete snapshot: {} ({id})", snapshot.name))
                        .disabled(!allowed)
                        .on_toggle(move |s: &mut State| {
                            let c = &mut edit(s).choices[index];
                            if !c.snapshots.remove(&id) {
                                c.snapshots.insert(id);
                            }
                        })
                        .build(k)
                        .key(format!("batch-delete-snapshot-{id}")),
                );
                if !allowed {
                    body=body.child(caption("Retained: checkpoints require their separate deletion workflow; ordinary snapshots must allow deletion.",k).wrap());
                }
                if c.snapshots.contains(&id) {
                    for disk in e.initial.snapshot_deletion_disks(id) {
                        body = body.child(
                            Checkbox::new(c.snapshot_disks.contains(&disk))
                                .label(format!(
                                    "Delete snapshot data: {}",
                                    vm_lifecycle::disk_label(&e.initial, disk)
                                ))
                                .on_toggle(move |s: &mut State| {
                                    let c = &mut edit(s).choices[index];
                                    if !c.snapshot_disks.remove(&disk) {
                                        c.snapshot_disks.insert(disk);
                                    }
                                })
                                .build(k)
                                .key(format!("batch-delete-snapshot-disk-{disk}")),
                        );
                    }
                }
            }
        }
    }
    let proposal = build_review(s, e);
    if let Err(reason) = &proposal {
        body = body.child(
            caption(reason, k)
                .wrap()
                .color(k.c.danger.icon)
                .role("alert"),
        );
    }
    if let Some(error) = &e.error {
        body = body.child(
            caption(error, k)
                .wrap()
                .color(k.c.danger.icon)
                .role("alert"),
        );
    }
    if !s.demo && !current(s, e) {
        body = body.child(
            caption(
                "The connection changed or inventory is stale. Reopen this review.",
                k,
            )
            .wrap(),
        );
    }
    if s.demo {
        body = body.child(caption("Synthetic preview; submission is disabled.", k).wrap());
    }
    let mut footer = row().w_full().gap(8.0).flex_wrap().child(
        Button::new("Cancel")
            .on_press(|s: &mut State| s.bulk_delete = None)
            .build(k)
            .key("batch-delete-close"),
    );
    if let Some(r) = &e.review {
        footer = footer
            .child(
                Button::new("Back")
                    .on_press(|s: &mut State| {
                        edit(s).review = None;
                        edit(s).confirm.text.clear();
                    })
                    .build(k),
            )
            .child(
                Button::new("Delete reviewed targets")
                    .variant(ButtonVariant::Danger)
                    .disabled(
                        !current(s, e)
                            || proposal.as_ref().ok() != Some(r)
                            || e.confirm.text != format!("DELETE {}", e.choices.len())
                            || s.resource_operations.len() >= 64,
                    )
                    .on_press(submit)
                    .build(k)
                    .key("batch-delete-submit"),
            );
    } else {
        footer = footer.child(
            Button::new("Review deletions")
                .disabled(proposal.is_err())
                .on_press(review)
                .build(k)
                .key("batch-delete-review"),
        );
    }
    let panel = col()
        .w((s.window_width - 24.0).clamp(280.0, 820.0))
        .h((s.window_height - 24.0).clamp(300.0, 900.0))
        .padding(16.0)
        .gap(10.0)
        .fill(k.c.bg_base)
        .radius(12.0)
        .child(heading(
            if e.review.is_some() {
                "Review selected deletions"
            } else {
                "Delete selected VMs / templates"
            },
            k,
        ))
        .child(
            body.scroll(rune::ScrollSpec {
                stick_bottom: false,
            })
            .grow(1.0)
            .min_h(0.0),
        )
        .child(footer);
    el().w_full()
        .h_full()
        .absolute(0.0, 0.0)
        .center()
        .fill(rune::Color::BLACK.alpha(0.55))
        .role("dialog")
        .label("Batch VM deletion")
        .modal(false, |s: &mut State| s.bulk_delete = None)
        .child(panel)
}
pub(super) fn scene(s: &mut State) {
    demo(s);
    let mut ids = Vec::new();
    if let Some(i) = &mut s.inventory {
        let i = Arc::make_mut(i);
        for vm in i
            .vms
            .values_mut()
            .filter(|v| !v.is_template && !v.is_control_domain)
            .take(2)
        {
            ids.push(vm.id.to_string());
            vm.power_state = PowerState::Halted;
            if let Some(d) = i.details.vms.get_mut(&vm.id) {
                d.is_default_template = Some(false);
                d.allowed_operations = Some(vec!["destroy".into()]);
                d.current_operations.clear();
                d.blocked_operations.clear();
            }
        }
    }
    s.rebuild();
    if let Some(id) = ids.first() {
        s.table.select(id.clone());
    }
    open(s);
    if s.bulk_delete.is_some() {
        for id in ids.iter().skip(1) {
            let vm = s
                .inventory
                .as_ref()
                .unwrap()
                .vms
                .values()
                .find(|v| v.id.to_string() == *id)
                .unwrap()
                .clone();
            edit(s).choices.push(Choice {
                vm,
                disks: BTreeSet::new(),
                snapshots: BTreeSet::new(),
                snapshot_disks: BTreeSet::new(),
            });
        }
        review(s);
    }
}
#[cfg(test)]
#[path = "bulk_delete_tests.rs"]
mod tests;
