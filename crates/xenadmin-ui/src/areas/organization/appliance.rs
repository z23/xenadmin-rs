//! vApps: the logic of the Rune module `appliance_view.rs`, carried over.
//!
//! One workspace per open dialog: the captured inventory of a pool, the
//! vApp and the VM chosen in it, the drafts, and the reviewed operation.
//! Every plan, every refusal text, the confirmation rule, the staleness
//! check and the submission are the Rune ones. New: the workspace follows
//! the pool of its target, not the scope session, and it can be opened for
//! a VM (`assign`, the command "Assign to vApp"), which then stays chosen
//! while the user picks the vApp.

use std::sync::Arc;

use xenadmin_core::{
    ApplianceId, ApplianceMember, ApplianceOperation as Op, ApplianceOrder, AppliancePower,
    Inventory, ObservedResource, OperationId, PoolId, ResourceCommand, ResourceTarget, VmAppliance,
    VmId,
};

use crate::prelude::*;

/// The Rune caption under a draft or review that cannot be applied; here
/// also the stale line of the review and the reason of the disabled button.
pub const NOT_CURRENT: &str = "Read-only or stale connection. Reopen this workspace on a current writable connection to apply changes.";

/// The Rune refusal of a power operation over unknown or empty membership.
pub const NO_MEMBERS: &str =
    "Refresh with complete, nonempty vApp membership before a power operation";

/// The Rune refusal of an apply without the typed vApp UUID.
pub const CONFIRM: &str = "Enter the exact requested vApp UUID";

pub struct Workspace {
    pub pool: PoolId,
    pub observed: Option<ObservedResource>,
    pub inventory: Arc<Inventory>,
    pub selected: Option<ApplianceId>,
    pub vm: Option<VmId>,
    /// The VM "Assign to vApp" was invoked on. It is chosen again in every
    /// vApp the user selects.
    pub assign: Option<VmId>,
    pub name: String,
    pub description: String,
    pub search: String,
    pub group_search: String,
    pub order: String,
    pub start_delay: String,
    pub shutdown_delay: String,
    pub confirmation: String,
    pub review: Option<Op>,
    pub error: Option<String>,
}

/// The workspace of a pool as it is now. `None` for a pool no session shows.
pub fn capture(m: &Model, pool: PoolId) -> Option<Workspace> {
    let inventory = m.inventory_for(pool)?;
    Some(Workspace {
        pool,
        observed: m
            .snapshot_for(pool)
            .and_then(|s| s.observe_resource(ResourceTarget::Pool(pool)).ok()),
        inventory,
        selected: None,
        vm: None,
        assign: None,
        name: String::new(),
        description: String::new(),
        search: String::new(),
        group_search: String::new(),
        order: "0".into(),
        start_delay: "0".into(),
        shutdown_delay: "0".into(),
        confirmation: String::new(),
        review: None,
        error: None,
    })
}

/// What a target opens the dialog on: its pool, the vApp when it is one
/// (or the vApp of the VM), and the VM when `assign` asks for it.
pub fn capture_for(m: &Model, target: &Target, assign: bool) -> Option<Workspace> {
    let object = target.single()?;
    let mut w = capture(m, object.pool?)?;
    match object.kind {
        ObjectKind::Appliance if !assign => {
            let id = ApplianceId::from(object.uuid()?);
            w.inventory.details.appliances.get(&id)?;
            select_in(&mut w, id);
        }
        ObjectKind::Vm if assign => {
            let vm = object.vm_id()?;
            w.inventory.vms.get(&vm).filter(|v| !v.is_template)?;
            w.assign = Some(vm);
            let group =
                w.inventory.details.vms.get(&vm).and_then(|d| d.appliance).filter(|group| {
                    w.inventory.details.appliances.contains_key(group)
                });
            if let Some(group) = group {
                select_in(&mut w, group);
            }
        }
        _ if assign => return None,
        ObjectKind::Folder | ObjectKind::Tag | ObjectKind::SavedSearch => return None,
        _ => {}
    }
    Some(w)
}

/// Opens the dialog. The organization change gives way, as in Rune.
pub fn open(m: &mut Model, w: Workspace) {
    super::close(m);
    m.areas.organization.appliances = Some(w);
}

/// The Rune Refresh: captures the pool again and drops the draft. The VM
/// the dialog was opened for stays.
pub fn refresh(m: &mut Model) {
    let Some(w) = &m.areas.organization.appliances else {
        return;
    };
    let (pool, assign) = (w.pool, w.assign);
    match capture(m, pool) {
        Some(mut fresh) => {
            fresh.assign = assign.filter(|vm| fresh.inventory.vms.contains_key(vm));
            m.areas.organization.appliances = Some(fresh);
        }
        None => m.areas.organization.appliances = None,
    }
}

pub fn selected(w: &Workspace) -> Option<&VmAppliance> {
    w.inventory.details.appliances.get(&w.selected?)
}

pub fn synthetic(m: &Model, w: &Workspace) -> bool {
    m.is_synthetic_pool(w.pool)
}

pub fn current(m: &Model, w: &Workspace) -> bool {
    !synthetic(m, w)
        && m.can_mutate_pool(w.pool)
        && w.observed.is_some()
        && w.observed
            == m.snapshot_for(w.pool)
                .and_then(|s| s.observe_resource(ResourceTarget::Pool(w.pool)).ok())
}

/// Why a review cannot be applied as it stands. `None` in a synthetic
/// pool, which says so itself.
pub fn stale(m: &Model, w: &Workspace) -> Option<String> {
    (w.review.is_some() && !synthetic(m, w) && !current(m, w)).then(|| NOT_CURRENT.to_owned())
}

fn select_in(w: &mut Workspace, id: ApplianceId) {
    w.selected = Some(id);
    w.vm = None;
    w.review = None;
    w.error = None;
    if let Some(group) = selected(w) {
        let (name, description) = (group.name.clone(), group.description.clone());
        w.name = name;
        w.description = description;
    }
    if let Some(vm) = w.assign {
        select_vm_in(w, vm);
    }
}

fn select_vm_in(w: &mut Workspace, id: VmId) {
    w.vm = Some(id);
    if let Some(order) = w
        .inventory
        .details
        .vms
        .get(&id)
        .and_then(|v| v.appliance_order)
    {
        w.order = order.order.to_string();
        w.start_delay = order.start_delay.to_string();
        w.shutdown_delay = order.shutdown_delay.to_string();
    }
}

pub fn select(m: &mut Model, id: ApplianceId) {
    if let Some(w) = &mut m.areas.organization.appliances {
        select_in(w, id);
    }
}

pub fn select_vm(m: &mut Model, id: VmId) {
    if let Some(w) = &mut m.areas.organization.appliances {
        select_vm_in(w, id);
    }
}

/// The Rune "New vApp" button: no vApp chosen, empty drafts.
pub fn new_group(m: &mut Model) {
    if let Some(w) = &mut m.areas.organization.appliances {
        w.selected = None;
        w.vm = None;
        w.name.clear();
        w.description.clear();
    }
}

fn stage_in(w: &mut Workspace, result: Result<Op, String>) {
    match result {
        Ok(op) => {
            w.review = Some(op);
            w.confirmation.clear();
            w.error = None;
        }
        Err(e) => w.error = Some(e),
    }
}

fn stage(m: &mut Model, plan: impl FnOnce(&Workspace) -> Result<Op, String>) {
    if let Some(w) = &mut m.areas.organization.appliances {
        let result = plan(w);
        stage_in(w, result);
    }
}

pub fn power_plan(w: &Workspace, action: AppliancePower) -> Result<Op, String> {
    let group = selected(w).ok_or("Select a vApp")?;
    if !group.vms_known || group.vms.is_empty() {
        return Err(NO_MEMBERS.into());
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

pub fn stage_power(m: &mut Model, action: AppliancePower) {
    stage(m, |w| power_plan(w, action));
}

pub fn membership_plan(w: &Workspace) -> Result<Op, String> {
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
        return Err("Remove this VM from its current vApp first".into());
    };
    Ok(Op::Membership {
        appliance: group.id,
        expected_members: group.vms.clone(),
        vm: id,
        join: !group.vms.contains(&id),
    })
}

pub fn stage_membership(m: &mut Model) {
    stage(m, membership_plan);
}

pub fn order_plan(w: &Workspace) -> Result<Op, String> {
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
        order: w.order.parse().map_err(|_| "Enter an integer order")?,
        start_delay: w
            .start_delay
            .parse()
            .map_err(|_| "Enter seconds from 0 to 86400")?,
        shutdown_delay: w
            .shutdown_delay
            .parse()
            .map_err(|_| "Enter seconds from 0 to 86400")?,
    };
    value
        .validate()
        .map_err(|_| "Delays must be from 0 to 86400 seconds")?;
    if previous == value {
        return Err("Order and delays have not changed".into());
    };
    Ok(Op::SetOrder {
        appliance: group.id,
        vm,
        previous,
        value,
    })
}

pub fn stage_order(m: &mut Model) {
    stage(m, order_plan);
}

/// "Review name / description". No client-side validation, as in Rune.
pub fn stage_rename(m: &mut Model) {
    stage(m, |w| {
        let g = selected(w).ok_or("Select a vApp")?;
        Ok(Op::Rename {
            appliance: g.id,
            previous_name: g.name.clone(),
            previous_description: g.description.clone(),
            name: w.name.clone(),
            description: w.description.clone(),
        })
    });
}

pub fn stage_delete(m: &mut Model) {
    stage(m, |w| {
        let g = selected(w).ok_or("Select a vApp")?;
        Ok(Op::DeleteEmpty {
            appliance: g.id,
            expected_name: g.name.clone(),
        })
    });
}

/// "Review new vApp". The name and the description go untrimmed.
pub fn stage_create(m: &mut Model) {
    stage(m, |w| {
        Ok(Op::Create {
            name: w.name.clone(),
            description: w.description.clone(),
        })
    });
}

/// Back from the review to the editor. The error line stays, as in Rune.
pub fn back(m: &mut Model) {
    if let Some(w) = &mut m.areas.organization.appliances {
        w.review = None;
    }
}

/// The vApp UUID the user must type before this operation is applied:
/// force shutdown and deletion.
pub fn confirmation(op: &Op) -> Option<String> {
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

/// Whether the typed confirmation is missing for the reviewed operation.
pub fn unconfirmed(w: &Workspace) -> bool {
    w.review
        .as_ref()
        .and_then(confirmation)
        .is_some_and(|required| required != w.confirmation)
}

pub fn apply(m: &mut Model) {
    let result = (|| {
        let w = m
            .areas
            .organization
            .appliances
            .as_ref()
            .ok_or("vApps closed")?;
        if !current(m, w) {
            return Err("Connection changed; reopen vApps".to_owned());
        }
        let operation = w.review.clone().ok_or("Review a change first")?;
        let permission =
            permissions::assess_for(m, w.pool, &operation.permission_requirements());
        if permission.decision().is_denied() {
            return Err(permission.explanation());
        }
        if confirmation(&operation).is_some_and(|v| v != w.confirmation) {
            return Err(CONFIRM.into());
        }
        m.connection_for(w.pool)
            .ok_or("Connect first")?
            .submit_resource(
                OperationId::new(),
                ResourceCommand::Appliance {
                    pool: w.observed.ok_or("Current pool is unavailable")?,
                    operation,
                },
            )
            .map_err(|e| e.to_string())
    })();
    match result {
        Ok(op) => {
            // Rune went on to the pool history; the operation is followed
            // in the Events tab now.
            m.track_resource(op);
            m.areas.organization.appliances = None;
        }
        Err(e) => {
            if let Some(w) = &mut m.areas.organization.appliances {
                w.error = Some(e);
            }
        }
    }
}

pub fn vm_name(w: &Workspace, id: VmId) -> String {
    w.inventory
        .vms
        .get(&id)
        .map(|v| v.name.clone())
        .unwrap_or_else(|| id.to_string())
}

/// The lines of a review, in the Rune order: the pool, what the operation
/// does, and for a membership or a power operation one line per member.
pub fn lines(w: &Workspace, op: &Op) -> Vec<String> {
    let mut lines = vec![format!("Pool {}", w.pool)];
    match op {
        Op::Create { name, description } => {
            lines.push(format!(
                "Create empty vApp: {name}\n{description}\nAdd existing VMs after creation completes."
            ));
        }
        Op::Rename {
            appliance,
            previous_name,
            previous_description,
            name,
            description,
        } => {
            lines.push(format!(
                "vApp {appliance}\nName: {previous_name} → {name}\nDescription: {previous_description} → {description}"
            ));
        }
        Op::DeleteEmpty {
            appliance,
            expected_name,
        } => {
            lines.push(format!(
                "Delete empty vApp {expected_name} ({appliance}). Only the group record is deleted. Remove its members first; VMs and disks are retained."
            ));
        }
        Op::Membership {
            appliance,
            vm,
            join,
            expected_members,
        } => {
            lines.push(format!(
                "{} {} ({vm}) {} vApp {appliance}. Reviewed current membership: {} VMs. No VM power or disk change is requested.",
                if *join { "Add" } else { "Remove" },
                vm_name(w, *vm),
                if *join { "to" } else { "from" },
                expected_members.len()
            ));
            for id in expected_members {
                lines.push(format!("{} · {id}", vm_name(w, *id)));
            }
        }
        Op::SetOrder {
            vm,
            previous,
            value,
            ..
        } => {
            lines.push(format!(
                "{} ({vm})\nOrder: {} → {}\nStart delay: {} → {} seconds\nShutdown delay: {} → {} seconds\nEach field is saved and verified separately; completed fields remain if a later edit fails.",
                vm_name(w, *vm),
                previous.order,
                value.order,
                previous.start_delay,
                value.start_delay,
                previous.shutdown_delay,
                value.shutdown_delay
            ));
        }
        Op::Power {
            appliance,
            members,
            action,
        } => {
            lines.push(format!(
                "vApp {appliance} · {} VMs\n{}",
                members.len(),
                match action {
                    AppliancePower::Start => "Start in ascending order; VMs at the same order start together. Running VMs are skipped. Suspended/paused VMs need a separate recovery action first.",
                    AppliancePower::CleanShutdown => "Request clean shutdown in descending order. Halted VMs are skipped. Guest shutdown failures remain partial; no force fallback runs.",
                    AppliancePower::HardShutdown => "Force shutdown in descending order. Unsaved guest data can be lost. Halted VMs are skipped.",
                }
            ));
            for m in members {
                lines.push(format!(
                    "Order {} · {} · {:?}\n{} · start {}s / shutdown {}s",
                    m.order.order,
                    vm_name(w, m.vm),
                    m.state,
                    m.vm,
                    m.order.start_delay,
                    m.order.shutdown_delay
                ));
            }
            lines.push("The server runs the sequence. Cancellation or task failure may leave some guests changed; inspect the group before resolving an unknown outcome.".into());
        }
    }
    // A review line is one line: the Rune captions held line breaks.
    lines
        .iter()
        .flat_map(|line| line.lines())
        .map(str::to_owned)
        .collect()
}
