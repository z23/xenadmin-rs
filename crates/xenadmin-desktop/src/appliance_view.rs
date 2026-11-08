use super::*;
use xenadmin_core::{
    ApplianceId, ApplianceMember, ApplianceOperation as Op, ApplianceOrder, AppliancePower,
    ObservedResource, ResourceCommand, ResourceTarget, VmAppliance,
};

pub(super) struct Workspace {
    pool: PoolId,
    observed: Option<ObservedResource>,
    inventory: Arc<Inventory>,
    selected: Option<ApplianceId>,
    vm: Option<VmId>,
    name: TextEdit,
    description: TextEdit,
    search: TextEdit,
    group_search: TextEdit,
    order: TextEdit,
    start_delay: TextEdit,
    shutdown_delay: TextEdit,
    confirmation: TextEdit,
    review: Option<Op>,
    error: Option<String>,
}
pub(super) fn open(s: &mut State) {
    let Some(inventory) = s.inventory.clone() else {
        return;
    };
    s.appliances = Some(Workspace {
        pool: inventory.pool_id,
        observed: s.snapshot.as_ref().and_then(|s| {
            s.observe_resource(ResourceTarget::Pool(inventory.pool_id))
                .ok()
        }),
        inventory,
        selected: None,
        vm: None,
        name: TextEdit::new(""),
        description: TextEdit::new(""),
        search: TextEdit::new(""),
        group_search: TextEdit::new(""),
        order: TextEdit::new("0"),
        start_delay: TextEdit::new("0"),
        shutdown_delay: TextEdit::new("0"),
        confirmation: TextEdit::new(""),
        review: None,
        error: None,
    });
    s.organization = None;
}
fn edit(s: &mut State) -> &mut Workspace {
    s.appliances.as_mut().expect("vApps open")
}
fn selected(w: &Workspace) -> Option<&VmAppliance> {
    w.inventory.details.appliances.get(&w.selected?)
}
fn current(s: &State, w: &Workspace) -> bool {
    !s.demo
        && s.can_mutate()
        && w.observed.is_some()
        && w.observed
            == s.snapshot
                .as_ref()
                .and_then(|s| s.observe_resource(ResourceTarget::Pool(w.pool)).ok())
}
fn select(s: &mut State, id: ApplianceId) {
    let w = edit(s);
    w.selected = Some(id);
    w.vm = None;
    w.review = None;
    w.error = None;
    if let Some(group) = selected(w) {
        let (name, description) = (group.name.clone(), group.description.clone());
        w.name = TextEdit::new(name);
        w.description = TextEdit::new(description);
    }
}
fn select_vm(s: &mut State, id: VmId) {
    let w = edit(s);
    w.vm = Some(id);
    if let Some(order) = w
        .inventory
        .details
        .vms
        .get(&id)
        .and_then(|v| v.appliance_order)
    {
        w.order = TextEdit::new(order.order.to_string());
        w.start_delay = TextEdit::new(order.start_delay.to_string());
        w.shutdown_delay = TextEdit::new(order.shutdown_delay.to_string());
    }
}
fn stage(s: &mut State, op: Op) {
    let w = edit(s);
    w.review = Some(op);
    w.confirmation = TextEdit::new("");
    w.error = None;
}
fn power_plan(w: &Workspace, action: AppliancePower) -> Result<Op, String> {
    let group = selected(w).ok_or("Select a vApp")?;
    if !group.vms_known || group.vms.is_empty() {
        return Err(
            "Refresh with complete, nonempty vApp membership before a power operation".into(),
        );
    }
    let mut members = Vec::new();
    for id in &group.vms {
        let vm = w
            .inventory
            .vms
            .get(id)
            .filter(|v| !v.is_template)
            .ok_or("A vApp member is unavailable")?;
        let detail = w
            .inventory
            .details
            .vms
            .get(id)
            .ok_or("VM ordering is unavailable")?;
        if !detail.appliance_known || detail.appliance != Some(group.id) {
            return Err("VM membership is incomplete; refresh first".into());
        }
        members.push(ApplianceMember {
            vm: *id,
            state: vm.power_state.clone(),
            order: detail
                .appliance_order
                .ok_or("VM ordering or delays are unavailable")?,
        });
    }
    members.sort_by_key(|m| (m.order.order, m.vm));
    if action != AppliancePower::Start {
        members.reverse();
    }
    Ok(Op::Power {
        appliance: group.id,
        members,
        action,
    })
}
fn stage_power(s: &mut State, action: AppliancePower) {
    match power_plan(edit(s), action) {
        Ok(op) => stage(s, op),
        Err(e) => edit(s).error = Some(e),
    }
}
fn stage_membership(s: &mut State) {
    let result = (|| {
        let w = edit(s);
        let group = selected(w)
            .filter(|g| g.vms_known)
            .ok_or("vApp membership is unavailable")?;
        let id = w.vm.ok_or("Select a VM")?;
        let detail = w
            .inventory
            .details
            .vms
            .get(&id)
            .filter(|d| d.appliance_known)
            .ok_or("VM membership is unavailable")?;
        if detail.appliance.is_some_and(|a| a != group.id) {
            return Err("Remove this VM from its current vApp first");
        };
        Ok(Op::Membership {
            appliance: group.id,
            expected_members: group.vms.clone(),
            vm: id,
            join: !group.vms.contains(&id),
        })
    })();
    match result {
        Ok(op) => stage(s, op),
        Err(e) => edit(s).error = Some(e.into()),
    }
}
fn stage_order(s: &mut State) {
    let result = (|| {
        let w = edit(s);
        let group = selected(w).ok_or("Select a vApp")?;
        let vm =
            w.vm.filter(|v| group.vms.contains(v))
                .ok_or("Select a VM in this vApp")?;
        let previous = w
            .inventory
            .details
            .vms
            .get(&vm)
            .and_then(|v| v.appliance_order)
            .ok_or("VM ordering is unavailable")?;
        let value = ApplianceOrder {
            order: w.order.text.parse().map_err(|_| "Enter an integer order")?,
            start_delay: w
                .start_delay
                .text
                .parse()
                .map_err(|_| "Enter seconds from 0 to 86400")?,
            shutdown_delay: w
                .shutdown_delay
                .text
                .parse()
                .map_err(|_| "Enter seconds from 0 to 86400")?,
        };
        value
            .validate()
            .map_err(|_| "Delays must be from 0 to 86400 seconds")?;
        if previous == value {
            return Err("Order and delays have not changed");
        };
        Ok(Op::SetOrder {
            appliance: group.id,
            vm,
            previous,
            value,
        })
    })();
    match result {
        Ok(op) => stage(s, op),
        Err(e) => edit(s).error = Some(e.into()),
    }
}
fn confirmation(op: &Op) -> Option<String> {
    match op {
        Op::Power {
            appliance,
            action: AppliancePower::HardShutdown,
            ..
        }
        | Op::DeleteEmpty { appliance, .. } => Some(appliance.to_string()),
        _ => None,
    }
}
fn apply(s: &mut State) {
    let result = (|| {
        let w = s.appliances.as_ref().ok_or("vApps closed")?;
        if !current(s, w) {
            return Err("Connection changed; reopen vApps".into());
        }
        let operation = w.review.clone().ok_or("Review a change first")?;
        let permission = session_permissions_view::assess(s, &operation.permission_requirements());
        if permission.decision().is_denied() {
            return Err(permission.explanation());
        }
        if confirmation(&operation).is_some_and(|v| v != w.confirmation.text) {
            return Err("Enter the exact requested vApp UUID".into());
        }
        s.connection
            .as_ref()
            .ok_or("Connect first")?
            .submit_resource(
                xenadmin_core::OperationId::new(),
                ResourceCommand::Appliance {
                    pool: w.observed.ok_or("Current pool is unavailable")?,
                    operation,
                },
            )
            .map_err(|e| e.to_string())
    })();
    match result {
        Ok(op) => {
            if let Some(w) = &s.waker {
                op.notify_on_change(w.clone());
            }
            s.resource_operations.push(op);
            s.appliances = None;
            resource_admin::open_pool(s, resource_admin::Kind::History);
        }
        Err(e) => edit(s).error = Some(e),
    }
}
fn vm_name(w: &Workspace, id: VmId) -> String {
    w.inventory
        .vms
        .get(&id)
        .map(|v| v.name.clone())
        .unwrap_or_else(|| id.to_string())
}
fn review(w: &Workspace, op: &Op, k: &Kit) -> El {
    let mut body = col()
        .gap(10.0)
        .child(heading(op.label(), k))
        .child(caption(format!("Pool {}", w.pool), k).wrap_anywhere());
    match op {
        Op::Create { name, description } => {
            body=body.child(caption(format!("Create empty vApp: {name}\n{description}\nAdd existing VMs after creation completes."),k).wrap());
        }
        Op::Rename {
            appliance,
            previous_name,
            previous_description,
            name,
            description,
        } => {
            body=body.child(caption(format!("vApp {appliance}\nName: {previous_name} → {name}\nDescription: {previous_description} → {description}"),k).wrap_anywhere());
        }
        Op::DeleteEmpty {
            appliance,
            expected_name,
        } => {
            body=body.child(caption(format!("Delete empty vApp {expected_name} ({appliance}). Only the group record is deleted. Remove its members first; VMs and disks are retained."),k).wrap());
        }
        Op::Membership {
            appliance,
            vm,
            join,
            expected_members,
        } => {
            body=body.child(caption(format!("{} {} ({vm}) {} vApp {appliance}. Reviewed current membership: {} VMs. No VM power or disk change is requested.",if *join{"Add"}else{"Remove"},vm_name(w,*vm),if *join{"to"}else{"from"},expected_members.len()),k).wrap_anywhere());
            for id in expected_members {
                body =
                    body.child(caption(format!("{} · {id}", vm_name(w, *id)), k).wrap_anywhere());
            }
        }
        Op::SetOrder {
            vm,
            previous,
            value,
            ..
        } => {
            body=body.child(caption(format!("{} ({vm})\nOrder: {} → {}\nStart delay: {} → {} seconds\nShutdown delay: {} → {} seconds\nEach field is saved and verified separately; completed fields remain if a later edit fails.",vm_name(w,*vm),previous.order,value.order,previous.start_delay,value.start_delay,previous.shutdown_delay,value.shutdown_delay),k).wrap_anywhere());
        }
        Op::Power {
            appliance,
            members,
            action,
        } => {
            body=body.child(caption(format!("vApp {appliance} · {} VMs\n{}",members.len(),match action{AppliancePower::Start=>"Start in ascending order; VMs at the same order start together. Running VMs are skipped. Suspended/paused VMs need a separate recovery action first.",AppliancePower::CleanShutdown=>"Request clean shutdown in descending order. Halted VMs are skipped. Guest shutdown failures remain partial; no force fallback runs.",AppliancePower::HardShutdown=>"Force shutdown in descending order. Unsaved guest data can be lost. Halted VMs are skipped."}),k).wrap());
            for m in members {
                body = body.child(
                    caption(
                        format!(
                            "Order {} · {} · {:?}\n{} · start {}s / shutdown {}s",
                            m.order.order,
                            vm_name(w, m.vm),
                            m.state,
                            m.vm,
                            m.order.start_delay,
                            m.order.shutdown_delay
                        ),
                        k,
                    )
                    .wrap_anywhere(),
                );
            }
            body=body.child(caption("The server runs the sequence. Cancellation or task failure may leave some guests changed; inspect the group before resolving an unknown outcome.",k).wrap());
        }
    }
    if let Some(required) = confirmation(op) {
        body = body
            .child(caption(format!("Enter vApp UUID to confirm: {required}"), k).wrap_anywhere())
            .child(
                TextInput::new(&w.confirmation)
                    .label("Confirm vApp UUID")
                    .on_edit(|s: &mut State| &mut edit(s).confirmation)
                    .build(k),
            );
    }
    body
}
pub(super) fn dialog(s: &State, k: &Kit) -> El {
    let Some(w) = &s.appliances else { return el() };
    let permission = w
        .review
        .as_ref()
        .map(|op| session_permissions_view::assess(s, &op.permission_requirements()));
    let denied = permission
        .as_ref()
        .is_some_and(|p| p.decision().is_denied());
    let mut body = col()
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        })
        .padding(18.0)
        .gap(10.0);
    if let Some(op) = &w.review {
        body = body.child(review(w, op, k));
    } else {
        body=body.child(heading("vApps",k)).child(caption("Group existing VMs and set their start/shutdown order. Refresh captures the current pool before preparing another change.",k).wrap());
        let mut groups = row().gap(6.0).flex_wrap().child(
            Button::new("New vApp")
                .on_press(|s: &mut State| {
                    let w = edit(s);
                    w.selected = None;
                    w.vm = None;
                    w.name = TextEdit::new("");
                    w.description = TextEdit::new("");
                })
                .build(k),
        );
        body = body.child(
            TextInput::new(&w.group_search)
                .label("Find vApp")
                .placeholder("Find vApp by name or UUID")
                .on_edit(|s: &mut State| &mut edit(s).group_search)
                .build(k),
        );
        let query = w.group_search.text.to_lowercase();
        let matches: Vec<_> = w
            .inventory
            .details
            .appliances
            .values()
            .filter(|g| g.name.to_lowercase().contains(&query) || g.id.to_string().contains(&query))
            .collect();
        body = body.child(caption(
            format!("{} matching vApps; showing up to 256", matches.len()),
            k,
        ));
        for group in matches.into_iter().take(256) {
            let id = group.id;
            groups = groups.child(
                Button::new(format!("{} · {} VMs", group.name, group.vms.len()))
                    .on_press(move |s: &mut State| select(s, id))
                    .build(k),
            );
        }
        body = body.child(groups);
        body = body
            .child(
                TextInput::new(&w.name)
                    .label("vApp name")
                    .placeholder("vApp name")
                    .on_edit(|s: &mut State| &mut edit(s).name)
                    .build(k),
            )
            .child(
                TextInput::new(&w.description)
                    .label("vApp description")
                    .placeholder("Description")
                    .on_edit(|s: &mut State| &mut edit(s).description)
                    .build(k),
            );
        if let Some(group) = selected(w) {
            body = body.child(
                caption(format!("Selected: {} · {}", group.name, group.id), k).wrap_anywhere(),
            );
            body = body.child(
                row()
                    .gap(6.0)
                    .flex_wrap()
                    .child(
                        Button::new("Review name / description")
                            .on_press(|s: &mut State| {
                                let w = edit(s);
                                let Some(g) = selected(w) else { return };
                                let op = Op::Rename {
                                    appliance: g.id,
                                    previous_name: g.name.clone(),
                                    previous_description: g.description.clone(),
                                    name: w.name.text.clone(),
                                    description: w.description.text.clone(),
                                };
                                stage(s, op)
                            })
                            .build(k),
                    )
                    .child(
                        Button::new("Delete empty vApp")
                            .disabled(!group.vms_known || !group.vms.is_empty())
                            .on_press(|s: &mut State| {
                                let Some(g) = selected(edit(s)) else { return };
                                let op = Op::DeleteEmpty {
                                    appliance: g.id,
                                    expected_name: g.name.clone(),
                                };
                                stage(s, op)
                            })
                            .build(k),
                    ),
            );
            let mut power = row().gap(6.0).flex_wrap();
            for (title, action) in [
                ("Start vApp", AppliancePower::Start),
                ("Shut down vApp", AppliancePower::CleanShutdown),
                ("Force shutdown", AppliancePower::HardShutdown),
            ] {
                power = power.child(
                    Button::new(title)
                        .disabled(!group.vms_known || group.vms.is_empty())
                        .on_press(move |s: &mut State| stage_power(s, action))
                        .build(k),
                );
            }
            body = body.child(power);
            if !group.vms_known {
                body=body.child(caption("Membership is incomplete; refresh with permission to read this group and its VMs.",k).wrap());
            }
            body = body.child(
                TextInput::new(&w.search)
                    .label("Find vApp members or ungrouped VMs")
                    .placeholder("Filter VMs by name or UUID")
                    .on_edit(|s: &mut State| &mut edit(s).search)
                    .build(k),
            );
            let query = w.search.text.to_lowercase();
            let mut vms: Vec<_> = w
                .inventory
                .vms
                .values()
                .filter(|v| {
                    !v.is_template
                        && (v.name.to_lowercase().contains(&query)
                            || v.id.to_string().contains(&query))
                })
                .collect();
            body = body.child(
                caption(
                    format!(
                        "{} matching VMs; showing up to 100. Members are marked below.",
                        vms.len()
                    ),
                    k,
                )
                .wrap(),
            );
            vms.sort_by_key(|v| (!group.vms.contains(&v.id), v.name.to_lowercase(), v.id));
            let mut vm_list = col()
                .w_full()
                .gap(5.0)
                .max_h(180.0)
                .scroll(rune::ScrollSpec {
                    stick_bottom: false,
                });
            for vm in vms.into_iter().take(100) {
                let id = vm.id;
                let member = group.vms.contains(&id);
                let selected = w.vm == Some(id);
                vm_list = vm_list.child(
                    Button::new(format!(
                        "{}{}{}",
                        if selected { "● " } else { "" },
                        vm.name,
                        if member { " · member" } else { "" }
                    ))
                    .on_press(move |s: &mut State| select_vm(s, id))
                    .build(k),
                );
            }
            body = body.child(vm_list);
            if let Some(id) = w.vm {
                body = body
                    .child(heading(format!("Selected VM: {}", vm_name(w, id)), k))
                    .child(
                        Button::new(if group.vms.contains(&id) {
                            "Review removal from vApp"
                        } else {
                            "Review addition to vApp"
                        })
                        .on_press(stage_membership)
                        .build(k),
                    );
                if group.vms.contains(&id) {
                    body=body.child(caption("Order (ascending start, descending shutdown); equal orders run together.",k).wrap()).child(TextInput::new(&w.order).label("vApp order").on_edit(|s:&mut State|&mut edit(s).order).build(k)).child(caption("Start delay in seconds",k)).child(TextInput::new(&w.start_delay).label("Start delay seconds").on_edit(|s:&mut State|&mut edit(s).start_delay).build(k)).child(caption("Shutdown delay in seconds",k)).child(TextInput::new(&w.shutdown_delay).label("Shutdown delay seconds").on_edit(|s:&mut State|&mut edit(s).shutdown_delay).build(k)).child(Button::new("Review order and delays").on_press(stage_order).build(k));
                }
            }
        } else {
            body = body.child(
                Button::new("Review new vApp")
                    .disabled(w.name.text.trim().is_empty())
                    .on_press(|s: &mut State| {
                        let w = edit(s);
                        let op = Op::Create {
                            name: w.name.text.clone(),
                            description: w.description.text.clone(),
                        };
                        stage(s, op)
                    })
                    .build(k)
                    .key("appliance-create-review"),
            );
        }
    }
    if let Some(error) = &w.error {
        body = body.child(
            caption(error, k)
                .color(k.c.danger.icon)
                .wrap()
                .role("alert"),
        );
    }
    if s.demo {
        body = body.child(caption("Synthetic preview; submission is disabled.", k).wrap());
    } else if !current(s, w) {
        body=body.child(caption("Read-only or stale connection. Reopen this workspace on a current writable connection to apply changes.",k).wrap());
    }
    let mut controls = row().gap(8.0).flex_wrap().child(
        Button::new("Close")
            .on_press(|s: &mut State| s.appliances = None)
            .build(k),
    );
    if let Some(op) = &w.review {
        controls = controls
            .child(
                Button::new("Back")
                    .on_press(|s: &mut State| edit(s).review = None)
                    .build(k),
            )
            .child(
                Button::new("Apply reviewed change")
                    .disabled(
                        !current(s, w)
                            || denied
                            || confirmation(op).is_some_and(|v| v != w.confirmation.text),
                    )
                    .on_press(apply)
                    .build(k)
                    .key("appliance-apply"),
            );
    } else {
        controls = controls.child(Button::new("Refresh").on_press(open).build(k));
    }
    el().w_full()
        .h_full()
        .absolute(0.0, 0.0)
        .center()
        .fill(rune::Color::BLACK.alpha(0.5))
        .modal(false, |s: &mut State| s.appliances = None)
        .role("dialog")
        .label("vApps")
        .child(
            col()
                .w((s.window_width - 32.0).clamp(280.0, 1000.0))
                .h((s.window_height - 40.0).max(280.0))
                .radius(10.0)
                .fill(k.c.bg_base)
                .child(body)
                .child(
                    session_permissions_view::footer(
                        controls,
                        permission.as_ref(),
                        (s.window_width - 32.0).clamp(280.0, 1000.0) - 28.0,
                        k,
                    )
                    .padding(14.0),
                ),
        )
}
pub(super) fn scene(s: &mut State) {
    demo(s);
    let mut inventory = s.inventory.as_ref().unwrap().as_ref().clone();
    let id = ApplianceId::parse("a0000000-0000-4000-8000-000000000001").unwrap();
    let vms: Vec<_> = inventory
        .vms
        .values()
        .filter(|v| !v.is_template)
        .take(3)
        .map(|v| v.id)
        .collect();
    for (i, vm) in vms.iter().enumerate() {
        let d = inventory.details.vms.entry(*vm).or_default();
        d.appliance = Some(id);
        d.appliance_known = true;
        d.appliance_order = Some(ApplianceOrder {
            order: i as i64,
            start_delay: 15,
            shutdown_delay: 5,
        });
    }
    inventory.details.appliances.insert(
        id,
        VmAppliance {
            id,
            name: "Example application".into(),
            description: "Synthetic database and web guests".into(),
            vms,
            vms_known: true,
            allowed_operations: Some(vec![
                "start".into(),
                "clean_shutdown".into(),
                "hard_shutdown".into(),
            ]),
        },
    );
    s.inventory = Some(Arc::new(inventory));
    open(s);
    select(s, id);
}
pub(super) fn review_scene(s: &mut State) {
    scene(s);
    stage_power(s, AppliancePower::HardShutdown);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn power_review_captures_members_order_delays_and_requires_force_confirmation() {
        let mut s = init();
        scene(&mut s);
        let w = edit(&mut s);
        let group = selected(w).unwrap().id;
        let op = power_plan(w, AppliancePower::HardShutdown).unwrap();
        let Op::Power { members, .. } = &op else {
            panic!()
        };
        assert!(
            members
                .windows(2)
                .all(|pair| pair[0].order.order >= pair[1].order.order)
        );
        assert!(members.iter().all(|m| m.order.start_delay == 15));
        assert_eq!(confirmation(&op), Some(group.to_string()));
        assert!(confirmation(&power_plan(w, AppliancePower::CleanShutdown).unwrap()).is_none());
        let mut changed = w.inventory.as_ref().clone();
        changed
            .details
            .appliances
            .get_mut(&group)
            .unwrap()
            .vms_known = false;
        w.inventory = Arc::new(changed);
        assert!(power_plan(w, AppliancePower::Start).is_err());
    }
    #[test]
    fn membership_review_preserves_exact_group_and_refuses_stealing_other_group_vm() {
        let mut s = init();
        scene(&mut s);
        let id = selected(edit(&mut s)).unwrap().vms[0];
        select_vm(&mut s, id);
        stage_membership(&mut s);
        assert!(
            matches!(&edit(&mut s).review,Some(Op::Membership{vm,join:false,expected_members,..}) if *vm==id && expected_members.contains(&id))
        );
        let w = edit(&mut s);
        w.review = None;
        let mut inventory = w.inventory.as_ref().clone();
        inventory.details.vms.get_mut(&id).unwrap().appliance =
            Some(ApplianceId::from(uuid::Uuid::new_v4()));
        w.inventory = Arc::new(inventory);
        stage_membership(&mut s);
        assert!(edit(&mut s).review.is_none());
        assert!(edit(&mut s).error.is_some());
    }
    #[test]
    fn compact_review_keeps_controls_visible_and_refuses_preview_submission() {
        let mut s = init();
        review_scene(&mut s);
        let required = confirmation(edit(&mut s).review.as_ref().unwrap()).unwrap();
        edit(&mut s).confirmation = TextEdit::new(required);
        let mut runtime = rune::Runtime::new(
            s,
            super::super::view,
            (513.0, 652.0),
            rune::Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        for _ in 0..30 {
            runtime.frame(1.0 / 60.0);
        }
        let (x, y) = runtime.find("appliance-apply").unwrap();
        assert!(x > 0.0 && x < 513.0 && y > 0.0 && y < 652.0);
        runtime.pointer_down(x, y);
        runtime.pointer_up(x, y);
        runtime.frame(0.0);
        assert!(runtime.state.resource_operations.is_empty());
        assert!(runtime.state.appliances.is_some());
        apply(&mut runtime.state);
        assert!(edit(&mut runtime.state).error.is_some());
    }
}
