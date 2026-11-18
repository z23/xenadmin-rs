//! Several VMs moved within their pool: the logic of the Rune
//! `bulk_move.rs`. One destination host and one transfer network for the
//! selection, device mappings per VM, one review, and one
//! `ResourceCommand::VmWorkflowBatch` of `VmWorkflow::Move` items.
//!
//! The selection is the command's target (the grid selection, or several
//! objects of the tree) instead of the visible selected rows of the Guests
//! table; the pool is the target's pool instead of the scope session's.
//! Nothing is ever dropped from a selection: one VM that cannot move
//! refuses the whole batch.

use std::sync::Arc;

use xenadmin_core::{
    BatchVmWorkflow, DiskPlacement, Error, HostId, Inventory, NetworkId, NicPlacement,
    ObservedResource, OperationId, PoolId, PowerState, ResourceCommand, ResourceTarget, Vm,
    VmMigration, VmWorkflow, VmWorkflowBatch,
};

use super::wizard::{DEMO, NOT_WRITABLE};
use super::{host_label, net_label, sr_label};
use crate::prelude::*;

/// The refusals of a selection, in the Rune words.
pub const ORDINARY: &str = "Select ordinary VM rows";
pub const NO_INVENTORY: &str = "Current inventory is unavailable";
pub const LIMIT: &str = "Select between 1 and 32 visible VMs";
pub const MISSING: &str = "A selected VM is missing";
/// A selection across pools: the batch is one command to one pool. (New
/// text: the Rune table showed one pool at a time.)
pub const ONE_POOL: &str = "Select VMs of one pool to move them together";
/// What the dialog says while its pool observation is not the current one.
pub const STALE: &str = "The connection changed or inventory is stale. Reopen this review.";
pub const PREVIEW: &str = "Synthetic preview; submission is disabled.";
/// Why "Move reviewed VMs" waits when the inventory moved on under the
/// review without refusing it. (New text: Rune disabled the button without
/// saying why.)
pub const CHANGED: &str =
    "The moves that would be reviewed now differ from this review. Go back and review again.";
/// The reason of a host that cannot be chosen. (New text: Rune disabled the
/// button without saying why.)
pub const HOST_DISABLED: &str = "This host is disabled.";
pub const TRANSFER_HELP: &str = "Required for halted moves or disk/network relocation. Ordinary live host moves keep the host's normal migration transport.";
pub const MEDIA: &str = "CD media remains in its current repository. Shared disks, CBT or snapshot chains may prevent storage relocation; the full selection is refused when any target is unsupported.";
pub const SEQUENCE: &str = "Every VM is checked before the first move. VMs move serially; Stop finishes the current VM workflow and skips the rest. Failed moves can leave partial placement or replacement disks. Recovery never starts an unsent target, repeats a move or acquires another receiver token. This review covers this pool only.";

pub struct Editor {
    pub pool: PoolId,
    pub name: String,
    /// The pool as observed when the selection was captured: the token the
    /// batch is submitted with.
    pub observed: Option<ObservedResource>,
    pub initial: Arc<Inventory>,
    pub choices: Vec<Choice>,
    pub destination: Option<HostId>,
    pub transfer: Option<NetworkId>,
    /// The one target whose device mappings are open.
    pub expanded: Option<usize>,
    pub review: Option<Review>,
    pub error: Option<String>,
}

pub struct Choice {
    pub source: Vm,
    pub affinity: Option<HostId>,
    pub disks: Vec<DiskPlacement>,
    pub networks: Vec<NicPlacement>,
    /// Why this VM cannot move, found when it was captured.
    pub refusal: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Review {
    pub batch: VmWorkflowBatch,
    /// The lines of each item, in the order of the batch.
    pub rows: Vec<Vec<String>>,
}

fn edit(m: &mut Model) -> Option<&mut Editor> {
    m.areas.migration.batch.as_mut()
}

pub(crate) fn close(m: &mut Model) {
    m.areas.migration.batch = None;
}

/// The VMs of a target, as their pool's inventory has them now, in the
/// target's order, with their pool: the Rune `selected`. The refusal is
/// the Rune text. Also what a batch of the `cross_pool` area starts from.
pub fn selected(m: &Model, target: &Target) -> Result<(PoolId, Vec<Vm>), String> {
    if !(1..=32).contains(&target.len()) {
        return Err(LIMIT.into());
    }
    if !target.all_of(ObjectKind::Vm) {
        return Err(ORDINARY.into());
    }
    let pool = target.pool().ok_or(ONE_POOL)?;
    let inventory = m.inventory_for(pool).ok_or(NO_INVENTORY)?;
    let vms = target
        .vms()
        .into_iter()
        .map(|id| inventory.vms.get(&id).cloned().ok_or(MISSING))
        .collect::<Result<Vec<_>, _>>()?;
    Ok((pool, vms))
}

/// One captured VM with its identity mappings. A later reason replaces an
/// earlier one, as in Rune.
pub(crate) fn choice(i: &Inventory, source: Vm) -> Choice {
    let mut refusal = None;
    if source.is_template
        || source.is_control_domain
        || !matches!(source.power_state, PowerState::Running | PowerState::Halted)
    {
        refusal = Some("Only ordinary running or halted VMs can move".into());
    }
    let details = i.details.vms.get(&source.id);
    if details.is_none_or(|d| !d.affinity_known) {
        refusal = Some("VM details or home-host identity are unavailable".into());
    }
    let ids = i
        .details
        .vbds
        .values()
        .filter(|v| v.vm == Some(source.id) && v.kind == "Disk")
        .map(|v| v.vdi)
        .collect::<Vec<_>>();
    let disks = ids
        .into_iter()
        .map(|id| {
            let id = id?;
            let sr = i.details.vdis.get(&id)?.sr?;
            Some(DiskPlacement {
                vdi: id,
                expected_sr: sr,
                destination_sr: sr,
            })
        })
        .collect::<Option<Vec<_>>>();
    let networks = i
        .details
        .vifs
        .values()
        .filter(|n| n.vm == Some(source.id))
        .map(|n| {
            Some(NicPlacement {
                vif: n.id,
                expected_network: n.network?,
                destination_network: n.network?,
            })
        })
        .collect::<Option<Vec<_>>>();
    if disks.is_none() || networks.is_none() {
        refusal = Some("A disk or network mapping is incomplete".into());
    }
    Choice {
        affinity: details.and_then(|d| d.affinity),
        source,
        disks: disks.unwrap_or_default(),
        networks: networks.unwrap_or_default(),
        refusal,
    }
}

/// The editor a target would open: the Rune `open` without its side
/// effects. `Err` is the refusal of the selection.
pub(crate) fn capture(m: &Model, target: &Target) -> Result<Editor, String> {
    let (pool, selected) = selected(m, target)?;
    let i = m.inventory_for(pool).ok_or(NO_INVENTORY)?;
    Ok(Editor {
        pool,
        name: i.pool_name.clone(),
        observed: m
            .snapshot_for(pool)
            .and_then(|v| v.observe_resource(ResourceTarget::Pool(pool)).ok()),
        choices: selected.into_iter().map(|vm| choice(&i, vm)).collect(),
        initial: i,
        destination: None,
        transfer: None,
        expanded: None,
        review: None,
        error: None,
    })
}

/// Why no draft could make the batch of a captured selection go ahead: a
/// VM that was refused when captured (the line its review would give), or
/// a pool that cannot be changed. `None`: the command can open it.
pub(crate) fn hopeless(m: &Model, e: &Editor) -> Option<String> {
    let refused = e.choices.iter().find_map(|c| {
        c.refusal
            .as_ref()
            .map(|reason| format!("{} ({}): {reason}", c.source.name, c.source.id))
    });
    refused.or_else(|| {
        (!m.is_synthetic_pool(e.pool) && !m.can_mutate_pool(e.pool)).then(|| NOT_WRITABLE.into())
    })
}

/// Opens the batch for a target. A refusal of the selection is in
/// `m.error` and nothing opens (the Rune `open`).
pub(crate) fn open(m: &mut Model, target: &Target) {
    match capture(m, target) {
        Ok(editor) => {
            super::close(m);
            m.areas.migration.batch = Some(editor);
        }
        Err(reason) => m.error = Some(reason),
    }
}

/// The pool is still the one observed and can be changed. Never in a
/// synthetic pool.
pub(crate) fn current(m: &Model, e: &Editor) -> bool {
    !m.is_synthetic_pool(e.pool)
        && m.can_mutate_pool(e.pool)
        && e.observed.is_some()
        && m.snapshot_for(e.pool)
            .and_then(|v| v.observe_resource(ResourceTarget::Pool(e.pool)).ok())
            == e.observed
}

/// The batch the draft describes against the inventory as it is now, with
/// the lines of each item; or every reason it cannot be made, one line per
/// refused VM. Cheap: the view calls it on every render to show the live
/// refusal, and the submission compares it with the stored review.
pub(crate) fn build_review(m: &Model, e: &Editor) -> Result<Review, String> {
    let i = m
        .inventory_for(e.pool)
        .filter(|i| i.pool_id == e.pool)
        .ok_or("The captured pool is no longer current")?;
    let i = i.as_ref();
    let destination = e
        .destination
        .ok_or("Choose a destination host for the selection")?;
    let mut rows = Vec::new();
    let mut items = Vec::new();
    let mut errors = Vec::new();
    for c in &e.choices {
        let result: Result<_, String> = (|| {
            if let Some(error) = &c.refusal {
                return Err(error.clone());
            }
            if i.details
                .vms
                .get(&c.source.id)
                .is_none_or(|d| !d.affinity_known)
            {
                return Err("Current home-host identity is unavailable".into());
            }
            let vm = i
                .vms
                .get(&c.source.id)
                .filter(|vm| {
                    !vm.is_template
                        && !vm.is_control_domain
                        && vm.power_state == c.source.power_state
                        && vm.host == c.source.host
                })
                .ok_or("VM identity, state or resident host changed")?;
            let operation = VmMigration {
                destination,
                expected_host: c.source.host,
                expected_affinity: c.affinity,
                live: c.source.power_state == PowerState::Running,
                disks: c.disks.clone(),
                networks: c.networks.clone(),
                transfer_network: e.transfer,
            };
            let availability = operation.availability(i, vm.id);
            if !availability.is_available() {
                return Err(availability.explanation());
            }
            let mut lines = vec![
                format!("{} ({})", vm.name, vm.id),
                format!(
                    "{} → {}",
                    if operation.live {
                        "Live migration; preserve running state"
                    } else {
                        "Remain halted; set home host after verified move"
                    },
                    host_label(i, destination)
                ),
            ];
            for disk in &operation.disks {
                lines.push(format!(
                    "Disk {}: {} → {}",
                    disk.vdi,
                    sr_label(i, disk.expected_sr),
                    sr_label(i, disk.destination_sr)
                ));
            }
            for nic in &operation.networks {
                lines.push(format!(
                    "NIC {}: {} → {}",
                    nic.vif,
                    net_label(i, nic.expected_network),
                    net_label(i, nic.destination_network)
                ));
            }
            if operation.uses_storage_transport() {
                lines.push(format!(
                    "Transfer network: {}",
                    net_label(i, e.transfer.ok_or("A transfer network is required")?)
                ));
            }
            Ok((
                BatchVmWorkflow {
                    vm: vm.id,
                    expected_state: vm.power_state.clone(),
                    workflow: VmWorkflow::Move(operation),
                },
                lines,
            ))
        })();
        match result {
            Ok((item, lines)) => {
                items.push(item);
                rows.push(lines);
            }
            Err(error) => errors.push(format!("{} ({}): {error}", c.source.name, c.source.id)),
        }
    }
    if !errors.is_empty() {
        return Err(errors.join("\n"));
    }
    let batch = VmWorkflowBatch { items };
    let permission = permissions::assess_for(m, e.pool, &batch.permission_requirements());
    if permission.decision().is_denied() {
        return Err(permission.explanation());
    }
    Ok(Review { batch, rows })
}

/// "Review moves".
pub(crate) fn review(m: &mut Model) {
    let Some(e) = &m.areas.migration.batch else {
        return;
    };
    let result = build_review(m, e);
    let Some(e) = edit(m) else { return };
    match result {
        Ok(r) => {
            e.review = Some(r);
            e.error = None;
        }
        Err(error) => e.error = Some(error),
    }
}

pub(crate) fn back(m: &mut Model) {
    if let Some(e) = edit(m) {
        e.review = None;
    }
}

pub(crate) fn choose_host(m: &mut Model, id: &str) {
    if let Some(e) = edit(m) {
        let host = e.initial.hosts.keys().find(|h| h.to_string() == id);
        if let Some(host) = host.copied() {
            e.destination = Some(host);
        }
    }
}

pub(crate) fn choose_transfer(m: &mut Model, id: &str) {
    if let Some(e) = edit(m)
        && let Some(network) = super::wizard::network(&e.initial, id)
    {
        e.transfer = Some(network);
    }
}

/// Opens the device mappings of one target, or closes them.
pub(crate) fn toggle_mappings(m: &mut Model, index: usize) {
    if let Some(e) = edit(m) {
        e.expanded = if e.expanded == Some(index) {
            None
        } else {
            Some(index)
        };
    }
}

pub(crate) fn choose_disk(m: &mut Model, index: usize, disk: usize, id: &str) {
    if let Some(e) = edit(m)
        && let Some(sr) = super::wizard::repository(&e.initial, id)
        && let Some(placement) = e.choices.get_mut(index).and_then(|c| c.disks.get_mut(disk))
    {
        placement.destination_sr = sr;
    }
}

pub(crate) fn choose_nic(m: &mut Model, index: usize, nic: usize, id: &str) {
    if let Some(e) = edit(m)
        && let Some(network) = super::wizard::network(&e.initial, id)
        && let Some(placement) = e
            .choices
            .get_mut(index)
            .and_then(|c| c.networks.get_mut(nic))
    {
        placement.destination_network = network;
    }
}

/// Why "Move reviewed VMs" is disabled, the first that applies; `None`
/// when the reviewed batch can be submitted. The demo first: there the
/// pool has no observation at all.
pub(crate) fn blocked(m: &Model, e: &Editor, proposal: &Result<Review, String>) -> Option<String> {
    if m.is_synthetic_pool(e.pool) {
        return Some(DEMO.into());
    }
    if !current(m, e) {
        return Some(STALE.into());
    }
    match proposal {
        Err(reason) => return Some(reason.clone()),
        Ok(now) if Some(now) != e.review.as_ref() => return Some(CHANGED.into()),
        Ok(_) => {}
    }
    (m.resource_operations.len() >= 64).then(|| Error::Busy.to_string())
}

/// Submits the reviewed batch. Every refusal before the connection is the
/// same `Stale` in the editor's error line, as in Rune: the pool is not
/// the one observed, the review is not what would be reviewed now, or 64
/// resource operations are already held. Accepted, the batch is tracked,
/// the editor closes and the operations are shown (Rune opened the pool
/// history).
pub(crate) fn submit(m: &mut Model) {
    let result = (|| {
        let e = m.areas.migration.batch.as_ref().ok_or(Error::Stale)?;
        let r = e.review.as_ref().ok_or(Error::Stale)?;
        if !current(m, e)
            || build_review(m, e).ok().as_ref() != Some(r)
            || m.resource_operations.len() >= 64
        {
            return Err(Error::Stale);
        }
        m.connection_for(e.pool)
            .ok_or(Error::Closed)?
            .submit_resource(
                OperationId::new(),
                ResourceCommand::VmWorkflowBatch {
                    pool: e.observed.ok_or(Error::Stale)?,
                    selection: r.batch.clone(),
                },
            )
    })();
    match result {
        Ok(operation) => {
            m.track_resource(operation);
            close(m);
            m.show_operations();
        }
        Err(error) => {
            if let Some(e) = edit(m) {
                e.error = Some(error.to_string());
            }
        }
    }
}
