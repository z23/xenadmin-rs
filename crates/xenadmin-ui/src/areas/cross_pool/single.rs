//! One VM moved or copied to another connected pool: the logic of the Rune
//! `cross_pool_vm.rs`. A capture of each pool ([`Peer`]), the draft of the
//! three stages ([`Editor`]), the reviewed `CrossPoolVmOperation` and its
//! submission through the connections of both pools
//! (`Connection::transfer_vm_to_pool`).
//!
//! Also here, because the Rune module had them: reading a transfer between
//! two pools again ([`recheck`], [`can_recheck`], [`connection_for_pool`])
//! and finding what a finished transfer created ([`destination_guest`],
//! [`destination_target`], [`show_destination`]). The list of transfers
//! they served is the History list of the Events tab now.
//!
//! The source is the pool of the command's target, not the scope session:
//! "the pool in front" of the Rune checks is "a session still shows this
//! pool" here. "Demo" is the source pool being synthetic; then, as in Rune,
//! every captured inventory stands for the current one.

use std::collections::BTreeSet;
use std::sync::Arc;

use uuid::Uuid;
use xenadmin_core::{
    ActionAvailability, Connection, CrossPoolMode, CrossPoolVmOperation, CrossPoolVmSelection,
    DiskPlacement, Error, HostId, Inventory, NetworkId, NicPlacement, ObservedResource, ObservedVm,
    OperationId, OperationState, PoolId, PowerState, ProfileId, ResourceAction, ResourceOperation,
    ResourceOperationRecord, ResourceTarget, SrId, Vm, VmId,
};

use crate::areas::events::hidden::{Family, HiddenKey};
use crate::areas::migration::{net_label, sr_label};
use crate::areas::{console, events};
use crate::prelude::*;
use crate::session;

/// What the wizard says while a captured connection is not what it was.
pub const STALE: &str = "A captured connection changed. Close and reopen this review.";
/// The reason a synthetic pool gives (the text every area uses).
pub const DEMO: &str = "Demo mode cannot change infrastructure.";
/// Why the command is disabled without a writable current connection to
/// the VM's pool. Rune disabled the button without saying why; the words
/// are those the batch gives for the same condition.
pub const NOT_WRITABLE: &str =
    "Moving VMs to another pool needs a current connection to this pool that can make changes";
pub const LIVE_ONLY: &str =
    "Live move preserves the running state. Cross-pool copy requires a halted guest.";
pub const NO_PEERS: &str =
    "Connect another pool with write access using Add Server… or Connect… of the Server menu, then reopen this transfer.";
pub const NO_DESTINATION: &str = "Choose a destination pool and host";
pub const LIMITS: &str = "This workflow accepts ordinary guests with exclusive owned data disks and empty CD drives. Snapshot chains, shared disks, mounted media, HA protection and vApps require separate preparation.";
pub const COPY_IDENTITY: &str = "Copy creates new VM, disk and device identities and new MAC addresses. The source stays halted. The destination is not started.";
pub const MOVE_IDENTITY: &str = "XAPI moves the VM and removes the exclusive source disks. VM/device UUIDs and MAC addresses are retained; destination disk UUIDs may change.";
pub const TRANSPORT: &str = "Host-to-host transport follows the hosts' migration policy and may use unencrypted HTTP or TLS without certificate verification. Desktop certificate trust does not secure this transfer. Use a trusted migration network. The application does not force TLS or silently retry with a different policy.";
pub const ACKNOWLEDGE: &str = "I reviewed the host-to-host transport policy";
pub const RESERVED: &str = "Both pools are reserved until completion or explicit acknowledgement. An interrupted transfer can leave partial source or destination resources. Recheck reads existing evidence; it never repeats a transfer. Verified destination UUIDs remain on the Events tab of the Notifications view.";
pub const CONSOLE: &str = "The source console closes when submitted. After verified completion, use Open destination console in this transfer's entry on the Events tab of the Notifications view.";
pub const UNAVAILABLE: &str = "The reviewed source or destination is no longer available";
pub const WAIT: &str = "Wait for current destination inventory to confirm this VM and host.";

/// The refusal of a guest that is neither running nor halted: the words of
/// the batch for the same guest. (Rune's `open` returned silently.)
pub fn unsupported_state(state: &PowerState) -> String {
    format!(
        "The VM is {state}. A running guest moves live and a halted guest moves halted; resume or shut this one down first"
    )
}

/// One connected pool as a transfer review captured it: the connection it
/// came from, the observation of the pool and its inventory. The batch of
/// cross-pool moves uses the same capture for its source and destination.
#[derive(Clone)]
pub struct Peer {
    /// `None` only in a synthetic capture (scenes and tests).
    pub profile: Option<ProfileId>,
    pub observed: Option<ObservedResource>,
    pub inventory: Arc<Inventory>,
}

impl Peer {
    pub fn pool(&self) -> PoolId {
        self.inventory.pool_id
    }

    /// The connection the capture came from, while it is writable and
    /// still observes the pool as it was captured.
    pub(crate) fn connection(&self, m: &Model) -> Option<Connection> {
        let connection = m.workspace.active.get(&self.profile?)?;
        if connection.is_read_only()
            || self.observed.is_none()
            || connection
                .latest()
                .observe_resource(ResourceTarget::Pool(self.inventory.pool_id))
                .ok()
                != self.observed
        {
            return None;
        }
        Some(connection.clone())
    }

    /// The pool's inventory as it is now; the captured one when the review
    /// is synthetic.
    pub(crate) fn current(&self, m: &Model, synthetic: bool) -> Option<Arc<Inventory>> {
        if synthetic {
            return Some(self.inventory.clone());
        }
        self.connection(m)?.latest().inventory().cloned()
    }
}

/// Whether a review whose source is this capture is a synthetic one: the
/// Rune `s.demo`, asked of the source pool's session.
pub(crate) fn synthetic(m: &Model, source: &Peer) -> bool {
    m.is_synthetic_pool(source.pool())
}

pub struct Editor {
    pub source: Peer,
    /// The VM as observed when the wizard opened: the token the transfer
    /// is submitted with. `None` only in a synthetic capture.
    pub observed: Option<ObservedVm>,
    pub vm: Vm,
    /// The other writable pools, as captured when the wizard opened.
    pub peers: Vec<Peer>,
    /// An index into `peers`.
    pub destination: Option<usize>,
    pub host: Option<HostId>,
    pub mode: CrossPoolMode,
    pub disks: Vec<DiskPlacement>,
    pub networks: Vec<NicPlacement>,
    pub transfer: Option<(NetworkId, String)>,
    /// The acknowledgement of host-to-host transport. It belongs to the
    /// review: building one and going back both withdraw it.
    pub transport_reviewed: bool,
    /// 0, 1 or 2.
    pub stage: u8,
    pub review: Option<CrossPoolVmOperation>,
    pub error: Option<String>,
}

impl Editor {
    pub fn pool(&self) -> PoolId {
        self.source.pool()
    }
}

fn edit(m: &mut Model) -> Option<&mut Editor> {
    m.areas.cross_pool.wizard.as_deref_mut()
}

pub(crate) fn close(m: &mut Model) {
    m.areas.cross_pool.wizard = None;
}

/// The writable connected pools, one capture per pool.
pub fn peers(m: &Model) -> Vec<Peer> {
    let mut pools = BTreeSet::new();
    m.workspace
        .active
        .iter()
        .filter_map(|(profile, c)| {
            if c.is_read_only() {
                return None;
            }
            let snapshot = c.latest();
            let inventory = snapshot.inventory()?.clone();
            let observed = snapshot
                .observe_resource(ResourceTarget::Pool(inventory.pool_id))
                .ok()?;
            pools.insert(inventory.pool_id).then_some(Peer {
                profile: Some(*profile),
                observed: Some(observed),
                inventory,
            })
        })
        .take(128)
        .collect()
}

/// The draft of a captured VM: every disk and interface placed where it
/// is, until a destination pool is chosen.
pub(crate) fn editor(
    source: Peer,
    observed: Option<ObservedVm>,
    vm: Vm,
    peers: Vec<Peer>,
) -> Editor {
    let disks = source
        .inventory
        .details
        .vbds
        .values()
        .filter(|b| b.vm == Some(vm.id) && b.kind == "Disk")
        .filter_map(|b| source.inventory.details.vdis.get(&b.vdi?))
        .filter_map(|d| {
            Some(DiskPlacement {
                vdi: d.id,
                expected_sr: d.sr?,
                destination_sr: d.sr?,
            })
        })
        .collect();
    let networks = source
        .inventory
        .details
        .vifs
        .values()
        .filter(|n| n.vm == Some(vm.id))
        .filter_map(|n| {
            Some(NicPlacement {
                vif: n.id,
                expected_network: n.network?,
                destination_network: n.network?,
            })
        })
        .collect();
    let mode = if vm.power_state == PowerState::Running {
        CrossPoolMode::MoveRunning
    } else {
        CrossPoolMode::MoveHalted
    };
    Editor {
        source,
        observed,
        vm,
        peers,
        destination: None,
        host: None,
        mode,
        disks,
        networks,
        transfer: None,
        transport_reviewed: false,
        stage: 0,
        review: None,
        error: None,
    }
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
        .filter(|v| !v.is_template && !v.is_control_domain)
    else {
        return Err(Eligibility::Hidden);
    };
    if m.is_synthetic_pool(pool) {
        return Err(Eligibility::disabled(DEMO));
    }
    if !m.can_mutate_pool(pool) {
        return Err(Eligibility::disabled(NOT_WRITABLE));
    }
    if !matches!(vm.power_state, PowerState::Running | PowerState::Halted) {
        return Err(Eligibility::disabled(unsupported_state(&vm.power_state)));
    }
    let observed = m.snapshot_for(pool).and_then(|p| p.observe_vm(id).ok());
    let Some((profile, connection)) = m
        .workspace
        .active
        .iter()
        .find(|(_, c)| observed.is_some() && c.latest().observe_vm(id).ok() == observed)
    else {
        return Err(Eligibility::disabled(NOT_WRITABLE));
    };
    let source = Peer {
        profile: Some(*profile),
        observed: connection
            .latest()
            .observe_resource(ResourceTarget::Pool(i.pool_id))
            .ok(),
        inventory: i.clone(),
    };
    let available = peers(m)
        .into_iter()
        .filter(|p| p.inventory.pool_id != i.pool_id)
        .collect();
    Ok(editor(source, observed, vm.clone(), available))
}

/// Opens the wizard on its first step. Nothing opens for an object the
/// command is hidden or disabled for.
pub(crate) fn open(m: &mut Model, object: &ObjectRef) {
    if let Ok(editor) = capture(m, object) {
        super::close(m);
        m.areas.cross_pool.wizard = Some(Box::new(editor));
    }
}

/// The repositories of a pool that can take a disk for a host: user
/// content, attached to that host.
pub(crate) fn repositories(i: &Inventory, host: Option<HostId>) -> Vec<(SrId, String)> {
    i.storage
        .iter()
        .filter(|sr| {
            i.details
                .storage
                .get(&SrId::from(sr.uuid))
                .is_some_and(|r| r.content_type == "user")
                && i.details.pbds.values().any(|p| {
                    p.sr == Some(SrId::from(sr.uuid))
                        && p.host == host
                        && p.currently_attached == Some(true)
                })
        })
        .map(|sr| (SrId::from(sr.uuid), sr.name.clone()))
        .collect()
}

/// The addresses a host can receive a transfer on: the IPv4 address and
/// every IPv6 address (without its prefix) of its attached interfaces,
/// with their network. Unspecified and loopback addresses are left out.
pub(crate) fn addresses(i: &Inventory, host: Option<HostId>) -> Vec<(NetworkId, String)> {
    let mut found: Vec<(NetworkId, String)> = Vec::new();
    for p in i
        .details
        .pifs
        .values()
        .filter(|p| p.host == host && p.currently_attached == Some(true))
    {
        let Some(network) = p.network else {
            continue;
        };
        for address in std::iter::once(p.ip.clone())
            .chain(
                p.ipv6
                    .iter()
                    .map(|a| a.split('/').next().unwrap_or_default().to_owned()),
            )
            .filter(|a| {
                a.parse::<std::net::IpAddr>()
                    .is_ok_and(|a| !a.is_unspecified() && !a.is_loopback())
            })
        {
            let value = (network, address);
            if !found.contains(&value) {
                found.push(value);
            }
        }
    }
    found
}

/// The option id of a transfer address in its choice.
pub(crate) fn address_id(value: &(NetworkId, String)) -> String {
    format!("{} {}", value.0, value.1)
}

/// The Rune `choose`: another destination pool ends the host, the address,
/// the acknowledgement and the review, and unmaps every device.
pub(crate) fn choose_pool(m: &mut Model, id: &str) {
    let Some(e) = edit(m) else { return };
    let Some(index) = id.parse::<usize>().ok().filter(|i| *i < e.peers.len()) else {
        return;
    };
    e.destination = Some(index);
    e.host = None;
    e.transfer = None;
    e.transport_reviewed = false;
    e.review = None;
    e.error = None;
    // Destination UUIDs are deliberately invalid until explicitly selected for this pool.
    for d in &mut e.disks {
        d.destination_sr = SrId::from(Uuid::nil());
    }
    for n in &mut e.networks {
        n.destination_network = NetworkId::from(Uuid::nil());
    }
}

/// Another host ends the transfer address; the device mappings stay (the
/// review catches a repository the new host has not attached).
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
        e.transfer = None;
    }
}

/// Move or copy, of a halted VM only.
pub(crate) fn choose_mode(m: &mut Model, id: &str) {
    let Some(e) = edit(m) else { return };
    if e.vm.power_state != PowerState::Halted {
        return;
    }
    for mode in [CrossPoolMode::MoveHalted, CrossPoolMode::CopyHalted] {
        if format!("{mode:?}") == id {
            e.mode = mode;
        }
    }
}

pub(crate) fn choose_disk(m: &mut Model, index: usize, id: &str) {
    let Some(e) = edit(m) else { return };
    let sr = destination(e).and_then(|d| {
        repositories(&d.inventory, e.host)
            .into_iter()
            .find(|(sr, _)| sr.to_string() == id)
            .map(|(sr, _)| sr)
    });
    if let (Some(sr), Some(disk)) = (sr, e.disks.get_mut(index)) {
        disk.destination_sr = sr;
    }
}

pub(crate) fn choose_nic(m: &mut Model, index: usize, id: &str) {
    let Some(e) = edit(m) else { return };
    let network = destination(e).and_then(|d| {
        d.inventory
            .networks
            .iter()
            .map(|n| NetworkId::from(n.uuid))
            .find(|n| n.to_string() == id)
    });
    if let (Some(network), Some(nic)) = (network, e.networks.get_mut(index)) {
        nic.destination_network = network;
    }
}

pub(crate) fn choose_address(m: &mut Model, id: &str) {
    let Some(e) = edit(m) else { return };
    let value = destination(e).and_then(|d| {
        addresses(&d.inventory, e.host)
            .into_iter()
            .find(|value| address_id(value) == id)
    });
    if value.is_some() {
        e.transfer = value;
    }
}

pub(crate) fn destination(e: &Editor) -> Option<&Peer> {
    e.peers.get(e.destination?)
}

/// The source still has the VM, a session still shows the source pool,
/// the VM is what was observed (always, in a synthetic review), and the
/// chosen destination is still the connection that was captured.
pub(crate) fn current(m: &Model, e: &Editor) -> bool {
    let synthetic = synthetic(m, &e.source);
    e.source
        .current(m, synthetic)
        .is_some_and(|i| i.vms.contains_key(&e.vm.id))
        && m.inventory_for(e.pool()).is_some()
        && (synthetic
            || e.observed.is_some_and(|o| {
                m.snapshot_for(e.pool())
                    .and_then(|p| p.observe_vm(e.vm.id).ok())
                    == Some(o)
            }))
        && destination(e).is_none_or(|p| p.current(m, synthetic).is_some())
}

/// The transfer the draft describes; the refusal names the first choice
/// that is missing.
pub(crate) fn operation(e: &Editor) -> Result<CrossPoolVmOperation, String> {
    let d = destination(e).ok_or("Choose a connected destination pool")?;
    let (network, address) = e
        .transfer
        .clone()
        .ok_or("Choose an attached destination transfer address")?;
    if e.disks.iter().any(|v| v.destination_sr.uuid().is_nil())
        || e.networks
            .iter()
            .any(|v| v.destination_network.uuid().is_nil())
    {
        return Err("Map every disk and network interface explicitly".into());
    }
    Ok(CrossPoolVmOperation {
        vm: e.vm.id,
        destination_pool: d.inventory.pool_id,
        destination_host: e.host.ok_or("Choose a destination host")?,
        expected_host: e.vm.host,
        expected_affinity: e
            .source
            .inventory
            .details
            .vms
            .get(&e.vm.id)
            .and_then(|v| v.affinity),
        transfer_network: network,
        transfer_address: address,
        mode: e.mode,
        disks: e.disks.clone(),
        networks: e.networks.clone(),
        host_transport_reviewed: e.transport_reviewed,
    })
}

/// What both pools, as they are now, say of the reviewed transfer. It is
/// unavailable until the transport acknowledgement is given.
pub(crate) fn availability(m: &Model, e: &Editor) -> ActionAvailability {
    let synthetic = synthetic(m, &e.source);
    let Some((source, dest)) = e
        .source
        .current(m, synthetic)
        .zip(destination(e).and_then(|p| p.current(m, synthetic)))
    else {
        return ActionAvailability::Stale;
    };
    e.review
        .as_ref()
        .map_or(ActionAvailability::Unknown, |o| {
            o.availability(&source, &dest)
        })
}

/// The explanation of a reviewed transfer that is not available; `None`
/// when it is.
pub(crate) fn unavailable(m: &Model, e: &Editor) -> Option<String> {
    let status = availability(m, e);
    (!status.is_available()).then(|| status.explanation())
}

pub(crate) fn next(m: &mut Model) {
    let Some(e) = edit(m) else { return };
    e.error = None;
    if e.stage == 0 {
        if e.destination.is_some() && e.host.is_some() {
            e.stage = 1;
        } else {
            e.error = Some(NO_DESTINATION.into());
        }
    } else {
        // Transport acknowledgement belongs to the final review, after exact mapping.
        match operation(e) {
            Ok(o) => {
                e.review = Some(o);
                e.stage = 2;
            }
            Err(error) => e.error = Some(error),
        }
    }
}

pub(crate) fn back(m: &mut Model) {
    let Some(e) = edit(m) else { return };
    e.stage = e.stage.saturating_sub(1);
    e.review = None;
    e.transport_reviewed = false;
}

/// Gives or withdraws the acknowledgement, in the draft and in the
/// reviewed transfer.
pub(crate) fn toggle_transport(m: &mut Model) {
    let Some(e) = edit(m) else { return };
    e.transport_reviewed = !e.transport_reviewed;
    if let Some(o) = &mut e.review {
        o.host_transport_reviewed = e.transport_reviewed;
    }
}

/// The lines of the review.
pub(crate) fn lines(e: &Editor, o: &CrossPoolVmOperation) -> Vec<String> {
    let Some(d) = destination(e) else {
        return Vec::new();
    };
    let mut lines = vec![
        o.mode.label().to_owned(),
        format!(
            "Destination: {} ({})",
            d.inventory.pool_name, o.destination_pool
        ),
        format!("Host: {}", o.destination_host),
        format!("Host-to-host address: {}", o.transfer_address),
    ];
    for disk in &o.disks {
        lines.push(format!(
            "Disk {}: {} → {}",
            disk.vdi,
            sr_label(&e.source.inventory, disk.expected_sr),
            sr_label(&d.inventory, disk.destination_sr)
        ));
    }
    for nic in &o.networks {
        lines.push(format!(
            "NIC {}: {} → {}",
            nic.vif,
            net_label(&e.source.inventory, nic.expected_network),
            net_label(&d.inventory, nic.destination_network)
        ));
    }
    lines.push(
        if o.mode.copy() {
            COPY_IDENTITY
        } else {
            MOVE_IDENTITY
        }
        .into(),
    );
    lines.push(RESERVED.into());
    if o.mode.live() {
        lines.push(CONSOLE.into());
    }
    lines
}

/// Why "Next" is disabled.
pub(crate) fn step_blocked(m: &Model, e: &Editor) -> Option<String> {
    (!current(m, e)).then(|| STALE.into())
}

/// Why "Apply reviewed transfer" is disabled, the first that applies;
/// `None` when the reviewed transfer can be submitted.
pub(crate) fn blocked(m: &Model, e: &Editor) -> Option<String> {
    if !current(m, e) {
        return Some(STALE.into());
    }
    if let Some(reason) = unavailable(m, e) {
        return Some(reason);
    }
    if synthetic(m, &e.source) {
        return Some(DEMO.into());
    }
    (!m.can_mutate_pool(e.pool())).then(|| NOT_WRITABLE.into())
}

/// Ends every console of these guests of this pool, the Console tab's and
/// those in detached console windows: the console of a guest that moves
/// live closes when its move is submitted (the review says so).
pub(crate) fn release_console(m: &mut Model, pool: PoolId, moving: &[VmId]) {
    let of_moving = |t: &console::ConsoleTarget| {
        t.pool == pool && t.host.is_none() && moving.contains(&t.vm)
    };
    if console::tab_target(m).is_some_and(of_moving) {
        console::close_tab(m);
    }
    for window in console::windows::open_windows(m) {
        if of_moving(&window.target) {
            console::windows::close(m, window.id);
        }
    }
}

/// Submits the reviewed transfer through the connections of both pools.
/// Accepted, the console of a guest that moves live is closed, the wizard
/// closes and the Events tab shows the transfer (Rune showed its own
/// history list). A refusal stays in the wizard's error line.
pub(crate) fn apply(m: &mut Model) {
    let result: Result<(ResourceOperation, PoolId, VmId, bool), String> = (|| {
        let e = m
            .areas
            .cross_pool
            .wizard
            .as_deref()
            .ok_or("Transfer review is closed")?;
        if synthetic(m, &e.source)
            || !m.can_mutate_pool(e.pool())
            || !current(m, e)
            || !availability(m, e).is_available()
        {
            return Err(UNAVAILABLE.to_string());
        }
        let d = destination(e).ok_or("Destination is unavailable")?;
        let source = e.source.connection(m).ok_or("Source disconnected")?;
        let dest = d.connection(m).ok_or("Destination disconnected")?;
        let operation = e.review.clone().ok_or("Review the transfer first")?;
        source
            .transfer_vm_to_pool(
                OperationId::new(),
                CrossPoolVmSelection {
                    source: e.observed.ok_or("Source is stale")?,
                    destination: d.observed.ok_or("Destination is stale")?,
                    operation,
                },
                &dest,
            )
            .map(|op| (op, e.pool(), e.vm.id, e.mode.live()))
            .map_err(|e| e.to_string())
    })();
    match result {
        Ok((op, pool, vm, live)) => {
            if live {
                release_console(m, pool, &[vm]);
            }
            let id = m.track_resource(op);
            close(m);
            events::show_operation(
                m,
                HiddenKey {
                    pool,
                    id,
                    family: Family::Resource,
                },
            );
        }
        Err(error) => {
            if let Some(e) = edit(m) {
                e.error = Some(error);
            }
        }
    }
}

// -- A transfer after it was submitted ------------------------------------------------

/// The writable connection that observes a pool, whichever session it is:
/// the scope session's or another connected one.
pub fn connection_for_pool(m: &Model, pool: PoolId) -> Option<Connection> {
    m.workspace
        .active
        .values()
        .chain(m.connection.iter())
        .find(|c| {
            !c.is_read_only()
                && c.latest()
                    .observe_resource(ResourceTarget::Pool(pool))
                    .is_ok()
        })
        .cloned()
}

/// Whether a transfer between two pools can be rechecked now: a single
/// move or copy, or a batch of moves, with both of its pools connected for
/// writing. A move that is part of a batch is read through its batch.
pub fn can_recheck(m: &Model, id: OperationId) -> bool {
    m.resource_rows
        .iter()
        .find(|r| r.id == id)
        .is_some_and(|r| {
            r.action.spans_pools()
                && r.batch_parent.is_none()
                && connection_for_pool(m, r.pool).is_some()
                && r.peer_pool()
                    .is_some_and(|peer| connection_for_pool(m, peer).is_some())
        })
}

/// Reads both pools of a transfer again; the handle of that check is
/// followed like any resource operation. It never repeats a transfer and
/// never starts a VM a batch had not started. `OperationConflict` for an
/// unknown record, `InvalidMigration` for one that does not span pools,
/// `Closed` while either pool has no writable connection.
pub fn recheck(m: &mut Model, id: OperationId) -> Result<(), Error> {
    let r = m
        .resource_rows
        .iter()
        .find(|r| r.id == id)
        .ok_or(Error::OperationConflict)?;
    let peer = r
        .peer_pool()
        .filter(|_| r.action.spans_pools())
        .ok_or(Error::InvalidMigration)?;
    let batch = matches!(r.action, ResourceAction::VmWorkflowBatch(_));
    let source = connection_for_pool(m, r.pool).ok_or(Error::Closed)?;
    let dest = connection_for_pool(m, peer).ok_or(Error::Closed)?;
    let handle = if batch {
        source.recheck_cross_pool_batch(id, &dest)?
    } else {
        source.recheck_cross_pool_vm(id, &dest)?
    };
    m.track_resource(handle);
    Ok(())
}

/// The guest a finished single transfer created, when `inventory` is the
/// destination pool's and proves it: the record succeeded and names
/// exactly one created VM, the guest is there and ordinary, and after a
/// live move it runs on the reviewed host. The flag says whether its
/// console is what to open (a live move).
pub fn destination_guest(
    inventory: &Inventory,
    record: &ResourceOperationRecord,
) -> Option<(VmId, bool)> {
    let ResourceAction::CrossPoolVm(o) = &record.action else {
        return None;
    };
    if record.state != OperationState::Succeeded
        || record.transfer_created.vms.len() != 1
        || inventory.pool_id != o.destination_pool
    {
        return None;
    }
    let vm = record.transfer_created.vms[0];
    let guest = inventory.vms.get(&vm)?;
    if guest.is_template
        || guest.is_control_domain
        || (o.mode.live()
            && (guest.host != Some(o.destination_host) || guest.power_state != PowerState::Running))
    {
        return None;
    }
    Some((vm, o.mode.live()))
}

/// The label of the Rune button under a finished single transfer: "Open
/// destination console" after a live move, "Show destination VM" after a
/// halted move or a copy. `None` for anything else (a batch, a transfer
/// that did not succeed). The button is disabled while
/// [`destination_target`] finds nothing.
pub fn destination_label(record: &ResourceOperationRecord) -> Option<&'static str> {
    match &record.action {
        ResourceAction::CrossPoolVm(o) if record.state == OperationState::Succeeded => {
            Some(if o.mode.live() {
                "Open destination console"
            } else {
                "Show destination VM"
            })
        }
        _ => None,
    }
}

/// The writable connection whose current inventory proves the guest of
/// [`destination_guest`], with the guest and the console flag.
pub fn destination_target(
    m: &Model,
    record: &ResourceOperationRecord,
) -> Option<(ProfileId, VmId, bool)> {
    m.workspace.active.iter().find_map(|(profile, c)| {
        if c.is_read_only() {
            return None;
        }
        let snapshot = c.latest();
        let (vm, console) = destination_guest(snapshot.inventory()?, record)?;
        snapshot
            .observe_vm(vm)
            .ok()
            .map(|_| (*profile, vm, console))
    })
}

/// Shows the VM a finished transfer created in its destination pool, and
/// opens its console after a live move: the Rune "Show destination VM" and
/// "Open destination console". Refused, `m.error` says why and nothing
/// changes.
pub fn show_destination(m: &mut Model, id: OperationId) -> bool {
    let found = m
        .resource_rows
        .iter()
        .find(|r| r.id == id)
        .and_then(|r| Some((destination_target(m, r)?, r.peer_pool()?)));
    let Some(((profile, vm, open_console), pool)) = found else {
        m.error = Some(WAIT.into());
        return false;
    };
    if !session::select_transfer_destination(m, profile, pool, vm) {
        return false;
    }
    if open_console {
        m.invoke(
            console::commands::OPEN,
            &Target::one(ObjectRef::vm(pool, vm)),
        );
    }
    true
}
