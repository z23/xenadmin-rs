//! Start or Resume on a chosen server (`VmCommand::PowerOnHost`).
//!
//! Carried over from the Rune `vm_placement.rs`: the VM and its observation
//! are captured from the snapshot's own inventory when the dialog opens, a
//! server is chosen, and the review is confirmed.

use xenadmin_core::{
    Error, HostId, ObservedVm, OperationAction, PermissionRequirements, PlacementOperation, PoolId,
    PowerAction, PowerState, VmCommand, VmId,
};

use super::{not_writable, permitted, submit_vm, synthetic};
use crate::prelude::*;

/// The caption of the review.
pub const PREREQUISITES: &str = "The host must be enabled and compatible with this VM's storage and network requirements. These prerequisites are checked again before submission.";
/// Why a disabled server cannot be chosen. (New text: Rune disabled its
/// button without saying why.)
pub const HOST_DISABLED: &str = "This server is disabled.";

pub struct Placement {
    pub target: ObservedVm,
    pub pool: PoolId,
    pub vm: VmId,
    pub name: String,
    pub action: PowerAction,
    pub host: Option<(HostId, String)>,
}

/// The placement an object would open: Start for a halted VM, Resume for a
/// suspended one. It needs the live snapshot of the VM's pool, so it never
/// succeeds in the demo.
pub(crate) fn capture(m: &Model, object: &ObjectRef) -> Result<Placement, Error> {
    let pool = object.pool.ok_or(Error::Stale)?;
    let id = object.vm_id().ok_or(Error::Stale)?;
    let snapshot = m.snapshot_for(pool).ok_or(Error::Stale)?;
    let inventory = snapshot.inventory().ok_or(Error::Stale)?;
    let vm = inventory.vms.get(&id).ok_or(Error::Stale)?;
    let action = match vm.power_state {
        PowerState::Halted => PowerAction::Start,
        PowerState::Suspended => PowerAction::Resume,
        _ => return Err(Error::UnavailableAction),
    };
    Ok(Placement {
        target: snapshot.observe_vm(id)?,
        pool: inventory.pool_id,
        vm: id,
        name: vm.name.clone(),
        action,
        host: None,
    })
}

/// The Rune `open`: a failure is in `m.error` and nothing opens.
pub(crate) fn open(m: &mut Model, object: &ObjectRef) {
    let result = capture(m, object);
    m.error = result.as_ref().err().map(|e| e.to_string());
    if let Ok(placement) = result {
        m.areas.power.placement = Some(placement);
    }
}

pub(crate) fn requirements(placement: &Placement) -> Option<PermissionRequirements> {
    let (host, _) = placement.host.as_ref()?;
    Some(
        OperationAction::Placement(PlacementOperation {
            placement: placement.action,
            host: *host,
        })
        .permission_requirements(),
    )
}

/// A server of the list was picked: the dialog turns to its review.
pub(crate) fn choose(m: &mut Model, id: &str) {
    let Some(p) = &m.areas.power.placement else {
        return;
    };
    let host = m.inventory_for(p.pool).and_then(|inventory| {
        inventory
            .hosts
            .values()
            .find(|host| host.enabled && host.id.to_string() == id)
            .map(|host| (host.id, host.name.clone()))
    });
    if let (Some(host), Some(p)) = (host, &mut m.areas.power.placement) {
        p.host = Some(host);
    }
}

/// "Choose another host": the choice and with it the review are dropped.
pub(crate) fn choose_another(m: &mut Model) {
    if let Some(p) = &mut m.areas.power.placement {
        p.host = None;
    }
}

pub(crate) fn close(m: &mut Model) {
    m.areas.power.placement = None;
}

/// Whether the VM is still the one observed when the dialog opened.
pub(crate) fn stale(m: &Model, p: &Placement) -> Option<String> {
    (m.snapshot_for(p.pool).and_then(|s| s.observe_vm(p.vm).ok()) != Some(p.target))
        .then(|| Error::Stale.to_string())
}

/// Why the confirm button is disabled besides staleness and a permission
/// denial: nothing can be changed in the pool.
pub(crate) fn not_changeable(m: &Model, p: &Placement) -> Option<String> {
    if synthetic(m, p.pool) {
        return Some(super::DEMO.into());
    }
    (!m.can_mutate_pool(p.pool)).then(|| not_writable(m, p.pool).into())
}

/// The Rune `submit`. That the chosen server is still enabled is not
/// checked again here: the core does.
pub(crate) fn submit(m: &mut Model) {
    let Some(placement) = &m.areas.power.placement else {
        return;
    };
    let Some((host, _)) = &placement.host else {
        return;
    };
    if stale(m, placement).is_some() || not_changeable(m, placement).is_some() {
        return;
    }
    let pool = placement.pool;
    let command = VmCommand::PowerOnHost {
        vm: placement.target,
        action: placement.action,
        host: *host,
    };
    let Some(required) = requirements(placement) else {
        return;
    };
    if !permitted(m, pool, &required) {
        return;
    }
    submit_vm(m, pool, command);
    if m.error.is_none() {
        m.areas.power.placement = None;
    }
}
