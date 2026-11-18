//! A reviewed batch of moves from one pool to another connected pool: the
//! logic of the Rune `bulk_cross_pool.rs`. The editor follows the single
//! cross-pool wizard's three stages: the destination pool and host, the
//! mappings and the migration address, and a review of both pools with the
//! transport acknowledgement. One destination repository takes every disk
//! of the selection and each source network maps to one destination
//! network; the review lists the mapping that results for each VM. Each VM
//! then moves by the single cross-pool move
//! (`Connection::move_vms_to_pool`).
//!
//! The source is the pool of the command's target, not the scope session:
//! "the pool in front" of the Rune checks is "a session still shows this
//! pool" here, and "demo" is the source pool being synthetic.

use std::collections::BTreeMap;

use xenadmin_core::{
    BatchVmWorkflow, CrossPoolBatchSelection, CrossPoolMode, CrossPoolVmOperation, DiskPlacement,
    Error, HostId, Inventory, NetworkId, NicPlacement, OperationId, PermissionAssessment, PoolId,
    PowerState, ResourceTarget, SrId, VdiId, VifId, Vm, VmWorkflow, VmWorkflowBatch,
};

use super::single::{self, Peer};
use crate::areas::events::{self, Family, OperationKey};
use crate::areas::migration::{self, net_label, sr_label};
use crate::bytes;
use crate::prelude::*;

/// What the command says, and what `open` puts in `m.error`, without a
/// writable current connection to the pool of the selection.
pub const NOT_WRITABLE: &str = single::NOT_WRITABLE;
pub const NO_PEERS: &str =
    "Connect another pool with write access using Add Server… or Connect… of the Server menu, then reopen this review.";
pub const NO_DESTINATION: &str = "Choose a destination pool and host";
pub const SELECTION_HELP: &str = "One destination pool and host take the whole selection. A running guest moves live and keeps running; a halted guest stays halted; both may be in one batch. A suspended or paused guest, a template or a guest the single cross-pool move would refuse makes the whole selection unavailable: none is left out. Copies are made one VM at a time with Move or Copy to Another Pool on that VM.";
pub const MAPPING_HELP: &str = "One destination repository takes every disk of the selection, and each source network maps to one destination network for every NIC on it. The review lists the mapping this gives each VM. To map one VM differently, move it by itself.";
pub const MAPPING_LIMITS: &str = "Each VM must be an ordinary guest with exclusive owned data disks and empty CD drives. Snapshot chains, shared disks, mounted media, HA protection and vApps need separate preparation, as for a single cross-pool move.";
pub const MOVES: &str = "XAPI moves each VM and removes its exclusive source disks. VM and device UUIDs and MAC addresses are kept; destination disk UUIDs may change.";
pub const TRANSPORT: &str = "Host-to-host transport follows the hosts' migration policy and may use unencrypted HTTP or TLS without certificate verification. Desktop certificate trust does not secure these transfers. Use a trusted migration network. The application does not force TLS or silently retry with a different policy.";
pub const ACKNOWLEDGE: &str =
    "I reviewed the host-to-host transport policy for every VM in this batch";
pub const SEQUENCE: &str = "Every VM is checked on both pools before the first transfer, and the destination repository must have room for the whole selection. The VMs then move one after another. Stop finishes the current VM's move and skips the rest; it does not ask the hosts to cancel a transfer. Both pools are reserved until the batch ends or is acknowledged.";
pub const INTERRUPTION: &str = "After an interruption or a restart nothing continues by itself, because the receiver a move was prepared with is not kept. Reconnect both pools and use Recheck: it reads the moves that were started, never repeats a transfer and never starts a remaining VM. A move whose outcome cannot be read stays unknown until it is acknowledged, and partial source or destination resources may remain.";
pub const CONSOLE: &str = "The console of a running guest in this selection closes when the batch is submitted. After its move is verified, open it from the destination pool.";
pub const CHANGED: &str = "A pool changed since this review was made. Go back and review again.";
pub const STALE: &str =
    "A captured connection changed or its inventory is stale. Reopen this review.";
pub const PREVIEW: &str = "Synthetic preview; submission is disabled.";
/// Why the submit button is disabled while the acknowledgement is not
/// given. Rune disabled the button without a text.
pub const NOT_ACKNOWLEDGED: &str =
    "Acknowledge the host-to-host transport policy for every VM in this batch first.";

pub struct Editor {
    pub source: Peer,
    pub choices: Vec<Choice>,
    /// The other writable pools, as captured when the editor opened.
    pub peers: Vec<Peer>,
    /// An index into `peers`.
    pub destination: Option<usize>,
    pub host: Option<HostId>,
    /// The one destination repository for every disk of the selection.
    pub repository: Option<SrId>,
    /// The destination network for each source network a selected VM has a NIC on.
    pub networks: BTreeMap<NetworkId, NetworkId>,
    pub transfer: Option<(NetworkId, String)>,
    /// The acknowledgement of host-to-host transport, given once for the whole batch.
    pub transport_reviewed: bool,
    /// 0, 1 or 2.
    pub stage: u8,
    pub review: Option<Review>,
    pub error: Option<String>,
}

impl Editor {
    pub fn pool(&self) -> PoolId {
        self.source.pool()
    }
}

/// A selected VM as it was when the editor opened.
pub struct Choice {
    pub source: Vm,
    pub affinity: Option<HostId>,
    pub disks: Vec<(VdiId, SrId)>,
    pub nics: Vec<(VifId, NetworkId)>,
    pub refusal: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Review {
    pub batch: VmWorkflowBatch,
    /// For each VM: its name and UUID, how it moves, and where each disk and NIC goes.
    pub rows: Vec<Vec<String>>,
    /// What the whole selection asks of the destination repository.
    pub capacity: String,
}

fn edit(m: &mut Model) -> Option<&mut Editor> {
    m.areas.cross_pool.batch.as_mut()
}

pub(crate) fn close(m: &mut Model) {
    m.areas.cross_pool.batch = None;
}

/// One captured VM. A later reason replaces an earlier one, as in Rune.
pub(crate) fn choice(i: &Inventory, source: Vm) -> Choice {
    let mut refusal = None;
    if source.is_template || source.is_control_domain {
        refusal = Some("Only ordinary VMs can move to another pool".to_string());
    } else if !matches!(source.power_state, PowerState::Running | PowerState::Halted) {
        refusal = Some(single::unsupported_state(&source.power_state));
    }
    let details = i.details.vms.get(&source.id);
    if details.is_none_or(|d| !d.affinity_known) {
        refusal = Some("VM details or home-host identity are unavailable".into());
    }
    let disks = i
        .details
        .vbds
        .values()
        .filter(|v| v.vm == Some(source.id) && v.kind == "Disk")
        .map(|v| {
            let id = v.vdi?;
            Some((id, i.details.vdis.get(&id)?.sr?))
        })
        .collect::<Option<Vec<_>>>();
    let nics = i
        .details
        .vifs
        .values()
        .filter(|n| n.vm == Some(source.id))
        .map(|n| Some((n.id, n.network?)))
        .collect::<Option<Vec<_>>>();
    if disks.is_none() || nics.is_none() {
        refusal = Some("A disk or network interface of this VM is incompletely reported".into());
    }
    Choice {
        affinity: details.and_then(|d| d.affinity),
        source,
        disks: disks.unwrap_or_default(),
        nics: nics.unwrap_or_default(),
        refusal,
    }
}

/// The editor a selection would open: the Rune `open` without its side
/// effects. The refusal is the one of the selection
/// (`migration::selected`, the Rune `bulk_move::selected`) or
/// [`NOT_WRITABLE`]: a review that could never be submitted is not opened.
/// A synthetic pool has no connection at all and opens it to be looked at.
pub(crate) fn capture(m: &Model, target: &Target) -> Result<Editor, String> {
    let (pool, selected) = migration::selected(m, target)?;
    let i = m
        .inventory_for(pool)
        .ok_or(migration::batch::NO_INVENTORY)?;
    let resource = ResourceTarget::Pool(pool);
    let observed = m
        .snapshot_for(pool)
        .and_then(|v| v.observe_resource(resource).ok());
    let profile = m
        .workspace
        .active
        .iter()
        .find(|(_, c)| observed.is_some() && c.latest().observe_resource(resource).ok() == observed)
        .map(|(profile, _)| *profile);
    if !m.is_synthetic_pool(pool) && (!m.can_mutate_pool(pool) || profile.is_none()) {
        return Err(NOT_WRITABLE.into());
    }
    let peers = single::peers(m)
        .into_iter()
        .filter(|p| p.inventory.pool_id != i.pool_id)
        .collect();
    Ok(Editor {
        choices: selected.into_iter().map(|vm| choice(&i, vm)).collect(),
        source: Peer {
            profile,
            observed,
            inventory: i,
        },
        peers,
        destination: None,
        host: None,
        repository: None,
        networks: BTreeMap::new(),
        transfer: None,
        transport_reviewed: false,
        stage: 0,
        review: None,
        error: None,
    })
}

/// Why no draft could make the batch of a captured selection go ahead: a
/// VM that was refused when captured, in the line its review would give.
/// `None`: the command can open it.
pub(crate) fn hopeless(e: &Editor) -> Option<String> {
    e.choices.iter().find_map(|c| {
        c.refusal
            .as_ref()
            .map(|reason| format!("{} ({}): {reason}", c.source.name, c.source.id))
    })
}

/// Opens the batch for a target on its first step. A refusal of the
/// selection is in `m.error` and nothing opens (the Rune `open`).
pub(crate) fn open(m: &mut Model, target: &Target) {
    match capture(m, target) {
        Ok(editor) => {
            super::close(m);
            m.areas.cross_pool.batch = Some(editor);
        }
        Err(reason) => m.error = Some(reason),
    }
}

pub(crate) fn destination(e: &Editor) -> Option<&Peer> {
    e.peers.get(e.destination?)
}

fn synthetic(m: &Model, e: &Editor) -> bool {
    single::synthetic(m, &e.source)
}

/// The source pool is still the connection that was captured and a session
/// still shows it, and so is the destination pool once one is chosen.
/// Never in a synthetic pool.
pub(crate) fn captured(m: &Model, e: &Editor) -> bool {
    let pool = e.pool();
    !synthetic(m, e)
        && m.can_mutate_pool(pool)
        && m.inventory_for(pool).is_some()
        && e.source.connection(m).is_some()
        && m.snapshot_for(pool)
            .and_then(|v| v.observe_resource(ResourceTarget::Pool(pool)).ok())
            == e.source.observed
        && destination(e).is_none_or(|p| p.connection(m).is_some())
}

/// Both pools are chosen and are still the connections that were captured.
pub(crate) fn current(m: &Model, e: &Editor) -> bool {
    destination(e).is_some() && captured(m, e)
}

/// A change of destination pool or host ends everything chosen for the previous one.
pub(crate) fn reset_mappings(e: &mut Editor) {
    e.repository = None;
    e.networks.clear();
    e.transfer = None;
    e.transport_reviewed = false;
    e.review = None;
    e.error = None;
}

pub(crate) fn choose_pool(m: &mut Model, id: &str) {
    let Some(e) = edit(m) else { return };
    let Some(index) = id.parse::<usize>().ok().filter(|i| *i < e.peers.len()) else {
        return;
    };
    e.destination = Some(index);
    e.host = None;
    reset_mappings(e);
}

pub(crate) fn choose_host(m: &mut Model, id: &str) {
    let Some(e) = edit(m) else { return };
    let host = destination(e).and_then(|d| {
        d.inventory
            .hosts
            .values()
            .find(|h| h.enabled && h.id.to_string() == id)
            .map(|h| h.id)
    });
    if let Some(host) = host {
        e.host = Some(host);
        reset_mappings(e);
    }
}

pub(crate) fn choose_repository(m: &mut Model, id: &str) {
    let Some(e) = edit(m) else { return };
    let sr = destination(e).and_then(|d| {
        single::repositories(&d.inventory, e.host)
            .into_iter()
            .find(|(sr, _)| sr.to_string() == id)
            .map(|(sr, _)| sr)
    });
    if sr.is_some() {
        e.repository = sr;
        e.error = None;
    }
}

/// Maps the source network at `index` of [`source_networks`].
pub(crate) fn choose_network(m: &mut Model, index: usize, id: &str) {
    let Some(e) = edit(m) else { return };
    let network = destination(e).and_then(|d| {
        d.inventory
            .networks
            .iter()
            .map(|n| NetworkId::from(n.uuid))
            .find(|n| n.to_string() == id)
    });
    if let (Some(source), Some(network)) = (source_networks(e).get(index).copied(), network) {
        e.networks.insert(source, network);
        e.error = None;
    }
}

pub(crate) fn choose_address(m: &mut Model, id: &str) {
    let Some(e) = edit(m) else { return };
    let value = destination(e).and_then(|d| {
        single::addresses(&d.inventory, e.host)
            .into_iter()
            .find(|value| single::address_id(value) == id)
    });
    if value.is_some() {
        e.transfer = value;
        e.error = None;
    }
}

/// The source networks the selection's NICs are on, each to be mapped once.
pub(crate) fn source_networks(e: &Editor) -> Vec<NetworkId> {
    let mut networks = e
        .choices
        .iter()
        .flat_map(|c| c.nics.iter().map(|(_, network)| *network))
        .collect::<Vec<_>>();
    networks.sort();
    networks.dedup();
    networks
}

/// What each pool's session is asked for: the source to send every VM, the
/// destination to prepare a receiver for each.
pub(crate) fn permissions(
    m: &Model,
    e: &Editor,
    batch: &VmWorkflowBatch,
) -> (PermissionAssessment, Option<PermissionAssessment>) {
    let peer = destination(e)
        .and_then(|p| p.connection(m))
        .map(|c| c.latest().session_permissions())
        .unwrap_or_default();
    (
        crate::permissions::assess_for(m, e.pool(), &batch.permission_requirements()),
        batch
            .peer_permission_requirements()
            .map(|required| required.assess(&peer)),
    )
}

/// The batch the current choices make from both pools as they are now,
/// with each VM's resulting mapping. An error names every VM that cannot
/// be part of it; none is left out.
pub(crate) fn build_review(m: &Model, e: &Editor) -> Result<Review, String> {
    let synthetic = synthetic(m, e);
    let source = e
        .source
        .current(m, synthetic)
        .filter(|i| m.inventory_for(i.pool_id).is_some())
        .ok_or("The captured source pool is no longer current")?;
    let peer = destination(e).ok_or("Choose a connected destination pool")?;
    let target = peer
        .current(m, synthetic)
        .ok_or("The destination pool is no longer connected as it was captured")?;
    let host = e.host.ok_or("Choose a destination host")?;
    let repository = e
        .repository
        .ok_or("Choose the destination repository for the selection's disks")?;
    if let Some(unmapped) = source_networks(e)
        .into_iter()
        .find(|network| !e.networks.contains_key(network))
    {
        return Err(format!(
            "Choose a destination network for {}",
            net_label(&source, unmapped)
        ));
    }
    let (transfer_network, transfer_address) = e
        .transfer
        .clone()
        .ok_or("Choose an attached destination transfer address")?;
    let mut rows = Vec::new();
    let mut items = Vec::new();
    let mut errors = Vec::new();
    let mut needed = Some(0u64);
    for c in &e.choices {
        let result: Result<_, String> = (|| {
            if let Some(error) = &c.refusal {
                return Err(error.clone());
            }
            let vm = source
                .vms
                .get(&c.source.id)
                .filter(|vm| {
                    !vm.is_template
                        && !vm.is_control_domain
                        && vm.power_state == c.source.power_state
                        && vm.host == c.source.host
                })
                .ok_or("VM identity, state or resident host changed")?;
            let live = vm.power_state == PowerState::Running;
            let networks = c
                .nics
                .iter()
                .map(|(vif, network)| {
                    Ok(NicPlacement {
                        vif: *vif,
                        expected_network: *network,
                        destination_network: *e.networks.get(network).ok_or_else(|| {
                            format!(
                                "Choose a destination network for {}",
                                net_label(&source, *network)
                            )
                        })?,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            let mut operation = CrossPoolVmOperation {
                vm: vm.id,
                destination_pool: target.pool_id,
                destination_host: host,
                expected_host: vm.host.filter(|_| live),
                expected_affinity: c.affinity,
                transfer_network,
                transfer_address: transfer_address.clone(),
                mode: if live {
                    CrossPoolMode::MoveRunning
                } else {
                    CrossPoolMode::MoveHalted
                },
                disks: c
                    .disks
                    .iter()
                    .map(|(vdi, sr)| DiskPlacement {
                        vdi: *vdi,
                        expected_sr: *sr,
                        destination_sr: repository,
                    })
                    .collect(),
                networks,
                // The acknowledgement is asked for on the review itself. What the two
                // pools allow is judged as it will be once it is given.
                host_transport_reviewed: true,
            };
            let availability = operation.availability(&source, &target);
            if !availability.is_available() {
                return Err(availability.explanation());
            }
            operation.host_transport_reviewed = e.transport_reviewed;
            let mut lines = vec![
                format!("{} ({})", vm.name, vm.id),
                if live {
                    "Live move: the guest keeps running".to_string()
                } else {
                    "Halted move: the guest stays halted".to_string()
                },
            ];
            for disk in &operation.disks {
                lines.push(format!(
                    "Disk {}: {} → {}",
                    disk.vdi,
                    sr_label(&source, disk.expected_sr),
                    sr_label(&target, disk.destination_sr)
                ));
            }
            for nic in &operation.networks {
                lines.push(format!(
                    "NIC {}: {} → {}",
                    nic.vif,
                    net_label(&source, nic.expected_network),
                    net_label(&target, nic.destination_network)
                ));
            }
            let size = operation.disks.iter().try_fold(0u64, |total, disk| {
                total.checked_add(source.details.vdis.get(&disk.vdi)?.virtual_size?)
            });
            Ok((BatchVmWorkflow::move_to_pool(operation), lines, size))
        })();
        match result {
            Ok((item, lines, size)) => {
                items.push(item);
                rows.push(lines);
                needed = needed.zip(size).and_then(|(a, b)| a.checked_add(b));
            }
            Err(error) => errors.push(format!("{} ({}): {error}", c.source.name, c.source.id)),
        }
    }
    if !errors.is_empty() {
        return Err(errors.join("\n"));
    }
    // The hosts are asked again before the first transfer. Here the inventory's figures
    // refuse a selection it already shows as too large for the one repository.
    let label = sr_label(&target, repository);
    let free = target
        .storage
        .iter()
        .find(|sr| sr.uuid == repository.uuid())
        .and_then(|sr| Some(sr.capacity_bytes?.saturating_sub(sr.used_bytes?)));
    let capacity = match (needed, free) {
        (Some(needed), Some(free)) if needed > free => {
            return Err(format!(
                "{label} reports {} free and the disks of the whole selection need {}",
                bytes(free),
                bytes(needed)
            ));
        }
        (Some(needed), Some(free)) => format!(
            "Destination repository {label}: the disks of the whole selection need {}; it reports {} free",
            bytes(needed),
            bytes(free)
        ),
        _ => format!(
            "Destination repository {label}: a disk size or the free space is not reported here; the hosts are asked before the first transfer"
        ),
    };
    let batch = VmWorkflowBatch { items };
    let (sending, receiving) = permissions(m, e, &batch);
    if sending.decision().is_denied() {
        return Err(format!("Source pool: {}", sending.explanation()));
    }
    if let Some(receiving) = receiving.filter(|p| p.decision().is_denied()) {
        return Err(format!("Destination pool: {}", receiving.explanation()));
    }
    Ok(Review {
        batch,
        rows,
        capacity,
    })
}

pub(crate) fn next(m: &mut Model) {
    let Some(e) = edit(m) else { return };
    if e.stage == 0 {
        if e.destination.is_some() && e.host.is_some() {
            e.stage = 1;
            e.error = None;
        } else {
            e.error = Some(NO_DESTINATION.into());
        }
        return;
    }
    // The acknowledgement belongs to the review, after the exact mappings are listed.
    e.transport_reviewed = false;
    let result = match m.areas.cross_pool.batch.as_ref() {
        Some(e) => build_review(m, e),
        None => return,
    };
    let Some(e) = edit(m) else { return };
    match result {
        Ok(review) => {
            e.review = Some(review);
            e.stage = 2;
            e.error = None;
        }
        Err(error) => e.error = Some(error),
    }
}

pub(crate) fn back(m: &mut Model) {
    let Some(e) = edit(m) else { return };
    e.stage = e.stage.saturating_sub(1);
    e.review = None;
    e.transport_reviewed = false;
    e.error = None;
}

/// Gives or withdraws the one acknowledgement, on every VM of the reviewed batch.
pub(crate) fn acknowledge(m: &mut Model) {
    let Some(e) = edit(m) else { return };
    e.transport_reviewed = !e.transport_reviewed;
    let reviewed = e.transport_reviewed;
    if let Some(review) = &mut e.review {
        for item in &mut review.batch.items {
            if let VmWorkflow::MoveToPool(operation) = &mut item.workflow {
                operation.host_transport_reviewed = reviewed;
            }
        }
    }
}

/// Why "Next" and "Review moves" are disabled.
pub(crate) fn step_blocked(m: &Model, e: &Editor) -> Option<String> {
    (!synthetic(m, e) && !captured(m, e)).then(|| STALE.into())
}

/// Why "Move reviewed VMs to the other pool" is disabled, the first that
/// applies; `None` when the reviewed batch can be submitted. The synthetic
/// pool first: there the pools have no observation at all.
pub(crate) fn blocked(
    m: &Model,
    e: &Editor,
    proposal: Option<&Result<Review, String>>,
) -> Option<String> {
    if synthetic(m, e) {
        return Some(PREVIEW.into());
    }
    if !current(m, e) {
        return Some(STALE.into());
    }
    match proposal {
        Some(Err(reason)) => return Some(reason.clone()),
        Some(Ok(now)) if Some(now) != e.review.as_ref() => return Some(CHANGED.into()),
        Some(Ok(_)) => {}
        None => return Some(Error::Stale.to_string()),
    }
    if !e.transport_reviewed {
        return Some(NOT_ACKNOWLEDGED.into());
    }
    (m.resource_operations.len() >= 64).then(|| Error::Busy.to_string())
}

/// Submits the reviewed batch through the connections of both pools. Every
/// refusal before the connections is the same `Stale` in the editor's
/// error line, as in Rune: a pool is not the one captured, the
/// acknowledgement is missing, the review is not what would be reviewed
/// now, or 64 resource operations are already held. Accepted, the console
/// of a guest that will move live is closed, the batch is tracked, the
/// editor closes and the Events tab shows the batch (Rune opened the pool
/// history).
pub(crate) fn submit(m: &mut Model) {
    let result = (|| {
        let e = m.areas.cross_pool.batch.as_ref().ok_or(Error::Stale)?;
        let r = e.review.as_ref().ok_or(Error::Stale)?;
        if !current(m, e)
            || !e.transport_reviewed
            || build_review(m, e).ok().as_ref() != Some(r)
            || m.resource_operations.len() >= 64
        {
            return Err(Error::Stale);
        }
        let peer = destination(e).ok_or(Error::Stale)?;
        let source = e.source.connection(m).ok_or(Error::Closed)?;
        let target = peer.connection(m).ok_or(Error::Closed)?;
        let live = r
            .batch
            .items
            .iter()
            .filter(|i| i.expected_state == PowerState::Running)
            .map(|i| i.vm)
            .collect::<Vec<_>>();
        source
            .move_vms_to_pool(
                OperationId::new(),
                CrossPoolBatchSelection {
                    source: e.source.observed.ok_or(Error::Stale)?,
                    destination: peer.observed.ok_or(Error::Stale)?,
                    batch: r.batch.clone(),
                },
                &target,
            )
            .map(|op| (op, e.pool(), live))
    })();
    match result {
        Ok((op, pool, live)) => {
            // As with the single live move, the console of a guest that will move live
            // is closed when the batch is submitted.
            single::release_console(m, pool, &live);
            let id = m.track_resource(op);
            close(m);
            events::show_operation(
                m,
                OperationKey {
                    pool,
                    id,
                    family: Family::Resource,
                },
            );
        }
        Err(error) => {
            if let Some(e) = edit(m) {
                e.error = Some(error.to_string());
            }
        }
    }
}
