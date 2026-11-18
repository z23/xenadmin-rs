//! One VM moved within its pool: the logic of the Rune `vm_migration.rs`.
//!
//! What a migration is, how its [`VmMigration`] is built from the captured
//! inventory and the draft, the staleness check and the submission are
//! carried over; the pool is the pool of the VM (the command's target)
//! instead of the scope session's, and the command goes through
//! `Model::submit_tracked_command_for`.
//!
//! Three stages, as in Rune: the destination host; storage, networks and
//! the transfer network; the review. The core decides what is suitable
//! (`VmMigration::validate` when the review is asked for, `availability` in
//! the review and again at submission): no list here is filtered, and the
//! current host is a destination like any other (it is how a move of the
//! disks alone is expressed).

use std::collections::BTreeSet;
use std::sync::Arc;

use xenadmin_core::{
    DiskPlacement, HostId, Inventory, NetworkId, NicPlacement, ObservedVm, PoolId, PowerState,
    SrId, Vm, VmCommand, VmMigration,
};

use crate::prelude::*;

/// The caption Rune put under its disabled "Migrate / move VM" button.
pub const UNSUPPORTED_STATE: &str =
    "Migration is available for running or halted VMs. Resume or shut down this guest first.";
/// Why the command is disabled outside the demo without a writable current
/// connection. Rune disabled the button without saying why; the words are
/// those of the Rune transfer dialogs for the same condition.
pub const NOT_WRITABLE: &str = "This connection is read-only, disconnected or stale.";
/// What the dialog says while the VM is not what was captured.
pub const STALE: &str =
    "The source connection changed. Close this review and reopen it from current inventory.";
/// The reason a synthetic pool gives a disabled confirm button (the text
/// every area uses).
pub const DEMO: &str = "Demo mode cannot change infrastructure.";
/// The refusal of "Next" on the first step, and of a migration without a
/// destination.
pub const NO_DESTINATION: &str = "Select a destination host.";
pub const DESTINATION_HELP: &str = "Running VMs migrate live. Halted VMs remain halted and the selected host becomes their home host. Host compatibility is checked again before dispatch.";
pub const PLACEMENT_HELP: &str = "Keep current placement for a shared-storage host move. To move only disks, choose the current host and change their storage repositories. CD media stays in its current repository and must be accessible to the destination.";
pub const NO_DISKS: &str = "No data disks in this captured view. Missing device inventory prevents applying a migration.";
pub const TRANSFER_HELP: &str = "Required for disk/network relocation and halted moves. Choose a network with a configured IP on the destination host. A host-only live move uses XAPI's ordinary migration transport.";
pub const TRANSPORT_LIMITS: &str = "Storage-transport moves with snapshot chains, shared disks or CBT are not supported yet. Use Move or Copy to Another Pool for cross-pool transfers.";
pub const CONSEQUENCES: &str = "VM, attachment and interface UUIDs are preserved. Moving disks may replace their UUIDs; verified new disks are recorded on the Events tab of the Notifications view. A failed or interrupted move can leave partial placement. Migration moves the selected disks. Leftover resources are retained for review, and interrupted migrations are never automatically repeated.";
pub const LIVE: &str = "Live migration; preserve running state";
pub const HALTED: &str = "Remain halted; change home host after verified move";

pub struct Editor {
    /// The VM as observed when the wizard opened: the token the command is
    /// submitted with. `None` only in a synthetic pool.
    pub target: Option<ObservedVm>,
    pub pool: PoolId,
    /// The VM as it was when captured.
    pub source: Vm,
    /// The inventory the lists and labels are drawn from.
    pub initial: Arc<Inventory>,
    pub destination: Option<HostId>,
    pub disks: Vec<DiskPlacement>,
    pub networks: Vec<NicPlacement>,
    pub transfer: Option<NetworkId>,
    /// 0, 1 or 2.
    pub stage: u8,
    pub review: Option<VmMigration>,
    pub error: Option<String>,
}

fn edit(m: &mut Model) -> Option<&mut Editor> {
    m.areas.migration.wizard.as_deref_mut()
}

pub(crate) fn close(m: &mut Model) {
    m.areas.migration.wizard = None;
}

/// The VM is still a guest of the captured pool and, outside a synthetic
/// pool, still what was observed.
pub(crate) fn current(m: &Model, f: &Editor) -> bool {
    m.inventory_for(f.pool).is_some_and(|i| {
        i.pool_id == f.pool
            && i.vms
                .get(&f.source.id)
                .is_some_and(|v| !v.is_template && !v.is_control_domain)
    }) && (m.is_synthetic_pool(f.pool)
        || f.target.is_some_and(|o| {
            m.snapshot_for(f.pool)
                .and_then(|p| p.observe_vm(f.source.id).ok())
                == Some(o)
        }))
}

/// The editor an object would open: the Rune `open` without its side
/// effects. `Err` is what the command is instead: hidden for anything but
/// an ordinary VM its pool's inventory has (where Rune showed no button),
/// disabled where Rune disabled the button or `open` returned silently.
pub(crate) fn capture(m: &Model, object: &ObjectRef) -> Result<Editor, Eligibility> {
    if object.kind != ObjectKind::Vm {
        return Err(Eligibility::Hidden);
    }
    let (Some(pool), Some(id)) = (object.pool, object.vm_id()) else {
        return Err(Eligibility::Hidden);
    };
    let Some(i) = m.inventory_for(pool) else {
        return Err(Eligibility::Hidden);
    };
    let Some(vm) = i
        .vms
        .get(&id)
        .filter(|v| !v.is_control_domain && !v.is_template)
    else {
        return Err(Eligibility::Hidden);
    };
    if !matches!(vm.power_state, PowerState::Running | PowerState::Halted) {
        return Err(Eligibility::disabled(UNSUPPORTED_STATE));
    }
    let synthetic = m.is_synthetic_pool(pool);
    if !synthetic && !m.can_mutate_pool(pool) {
        return Err(Eligibility::disabled(NOT_WRITABLE));
    }
    let target = m.snapshot_for(pool).and_then(|p| p.observe_vm(id).ok());
    if !synthetic && target.is_none() {
        return Err(Eligibility::disabled(NOT_WRITABLE));
    }
    let diskids = i
        .details
        .vbds
        .values()
        .filter(|v| v.vm == Some(id) && v.kind == "Disk")
        .filter_map(|v| v.vdi)
        .collect::<BTreeSet<_>>();
    let disks = diskids
        .into_iter()
        .filter_map(|id| {
            i.details.vdis.get(&id).and_then(|d| {
                d.sr.map(|sr| DiskPlacement {
                    vdi: id,
                    expected_sr: sr,
                    destination_sr: sr,
                })
            })
        })
        .collect();
    let networks = i
        .details
        .vifs
        .values()
        .filter(|n| n.vm == Some(id))
        .filter_map(|n| {
            n.network.map(|network| NicPlacement {
                vif: n.id,
                expected_network: network,
                destination_network: network,
            })
        })
        .collect();
    Ok(Editor {
        target,
        pool,
        source: vm.clone(),
        initial: i.clone(),
        destination: None,
        disks,
        networks,
        transfer: None,
        stage: 0,
        review: None,
        error: None,
    })
}

/// Opens the wizard on its first step. Nothing opens for an object the
/// command is hidden or disabled for.
pub(crate) fn open(m: &mut Model, object: &ObjectRef) {
    if let Ok(editor) = capture(m, object) {
        super::close(m);
        m.areas.migration.wizard = Some(Box::new(editor));
    }
}

/// The migration the draft describes, as the core accepts it.
pub(crate) fn operation(f: &Editor) -> Result<VmMigration, String> {
    let o = VmMigration {
        destination: f.destination.ok_or(NO_DESTINATION)?,
        expected_host: f.source.host,
        expected_affinity: f
            .initial
            .details
            .vms
            .get(&f.source.id)
            .and_then(|v| v.affinity),
        live: f.source.power_state == PowerState::Running,
        disks: f.disks.clone(),
        networks: f.networks.clone(),
        transfer_network: f.transfer,
    };
    o.validate().map_err(|e| e.to_string())?;
    Ok(o)
}

/// "Next" and "Review migration". The error is cleared only by a step that
/// succeeds.
pub(crate) fn next(m: &mut Model) {
    let Some(f) = edit(m) else { return };
    if f.stage == 0 {
        if f.destination.is_some() {
            f.stage = 1;
            f.error = None;
        } else {
            f.error = Some(NO_DESTINATION.into())
        }
    } else {
        match operation(f) {
            Ok(o) => {
                f.review = Some(o);
                f.stage = 2;
                f.error = None;
            }
            Err(e) => f.error = Some(e),
        }
    }
}

/// One step back. The review is dropped, the error is not (as in Rune).
pub(crate) fn back(m: &mut Model) {
    if let Some(f) = edit(m) {
        f.stage = f.stage.saturating_sub(1);
        f.review = None;
    }
}

/// The picked option of a host list: the host of the captured inventory
/// with that id.
pub(crate) fn choose_host(m: &mut Model, id: &str) {
    if let Some(f) = edit(m) {
        let host = f.initial.hosts.keys().find(|h| h.to_string() == id);
        if let Some(host) = host.copied() {
            f.destination = Some(host);
        }
    }
}

pub(crate) fn choose_disk(m: &mut Model, index: usize, id: &str) {
    if let Some(f) = edit(m)
        && let Some(sr) = repository(&f.initial, id)
        && let Some(disk) = f.disks.get_mut(index)
    {
        disk.destination_sr = sr;
    }
}

pub(crate) fn choose_nic(m: &mut Model, index: usize, id: &str) {
    if let Some(f) = edit(m)
        && let Some(network) = network(&f.initial, id)
        && let Some(nic) = f.networks.get_mut(index)
    {
        nic.destination_network = network;
    }
}

/// Once chosen, the transfer network can be changed but not unset.
pub(crate) fn choose_transfer(m: &mut Model, id: &str) {
    if let Some(f) = edit(m)
        && let Some(network) = network(&f.initial, id)
    {
        f.transfer = Some(network);
    }
}

/// The repository of an inventory with that id as an option id.
pub(crate) fn repository(i: &Inventory, id: &str) -> Option<SrId> {
    i.storage
        .iter()
        .map(|sr| SrId::from(sr.uuid))
        .find(|sr| sr.to_string() == id)
}

/// The network of an inventory with that id as an option id.
pub(crate) fn network(i: &Inventory, id: &str) -> Option<NetworkId> {
    i.networks
        .iter()
        .map(|n| NetworkId::from(n.uuid))
        .find(|n| n.to_string() == id)
}

/// The lines of the review. Rune wrote each placement on two lines of one
/// caption ("Disk {vdi}" above "{from} → {to}"); here it is one line, in
/// the form the batch review always had.
pub(crate) fn lines(f: &Editor, o: &VmMigration) -> Vec<String> {
    let i = &f.initial;
    let mut lines = vec![
        format!("Destination: {}", super::host_label(i, o.destination)),
        format!("Mode: {}", if o.live { LIVE } else { HALTED }),
    ];
    for d in &o.disks {
        lines.push(format!(
            "Disk {}: {} → {}",
            d.vdi,
            super::sr_label(i, d.expected_sr),
            super::sr_label(i, d.destination_sr)
        ));
    }
    for n in &o.networks {
        lines.push(format!(
            "NIC {}: {} → {}",
            n.vif,
            super::net_label(i, n.expected_network),
            super::net_label(i, n.destination_network)
        ));
    }
    if o.uses_storage_transport()
        && let Some(n) = o.transfer_network
    {
        lines.push(format!("Transfer network: {}", super::net_label(i, n)));
    }
    lines.push(CONSEQUENCES.into());
    lines
}

/// What the core says against the reviewed migration in the inventory as
/// it is now; `None` when it is available (or there is no inventory, which
/// [`current`] reports).
pub(crate) fn unavailable(m: &Model, f: &Editor) -> Option<String> {
    let o = f.review.as_ref()?;
    let i = m.inventory_for(f.pool)?;
    let status = o.availability(&i, f.source.id);
    (!status.is_available()).then(|| status.explanation())
}

/// Why "Next" and "Review migration" are disabled.
pub(crate) fn step_blocked(m: &Model, f: &Editor) -> Option<String> {
    if !current(m, f) {
        return Some(STALE.into());
    }
    (!m.is_synthetic_pool(f.pool) && !m.can_mutate_pool(f.pool)).then(|| NOT_WRITABLE.into())
}

/// Why "Apply migration" is disabled, the first that applies; `None` when
/// the reviewed migration can be submitted.
pub(crate) fn blocked(m: &Model, f: &Editor) -> Option<String> {
    if !current(m, f) {
        return Some(STALE.into());
    }
    if let Some(reason) = unavailable(m, f) {
        return Some(reason);
    }
    let denial = f.review.as_ref().and_then(|o| {
        permissions::denial(&permissions::assess_for(
            m,
            f.pool,
            &o.permission_requirements(),
        ))
    });
    if denial.is_some() {
        return denial;
    }
    if m.is_synthetic_pool(f.pool) {
        return Some(DEMO.into());
    }
    (!m.can_mutate_pool(f.pool) || f.target.is_none()).then(|| NOT_WRITABLE.into())
}

/// Submits the reviewed migration. Every check of the button is made
/// again; a migration that is refused before it reaches the connection
/// changes nothing (the Rune `apply` returned silently). The wizard closes
/// only when the command was accepted; a refusal of the submission, and a
/// permission the session is known not to have, stay in its error line.
pub(crate) fn apply(m: &mut Model) {
    let Some(f) = m.areas.migration.wizard.as_ref() else {
        return;
    };
    let Some(target) = f.target else { return };
    let Some(o) = f.review.clone() else { return };
    let (pool, id) = (f.pool, f.source.id);
    if !m.can_mutate_pool(pool)
        || !current(m, f)
        || !m
            .inventory_for(pool)
            .is_some_and(|i| o.availability(&i, id).is_available())
    {
        return;
    }
    let permission = permissions::assess_for(m, pool, &o.permission_requirements());
    let result = match permissions::denial(&permission) {
        Some(denial) => Err(denial),
        None => m
            .submit_tracked_command_for(
                pool,
                VmCommand::Migrate {
                    vm: target,
                    operation: o,
                },
            )
            .map_err(|e| e.to_string()),
    };
    match result {
        Ok(_) => {
            m.error = None;
            close(m);
        }
        Err(error) => {
            if let Some(f) = edit(m) {
                f.error = Some(error);
            }
        }
    }
}
