//! A reviewed batch of moves from the selected pool to another connected pool. The editor
//! follows the single cross-pool form's three stages: the destination pool and host, the
//! mappings and the migration address, and a review of both pools with the transport
//! acknowledgement. One destination repository takes every disk of the selection and each
//! source network maps to one destination network; the review lists the mapping that
//! results for each VM. Each VM then moves by the single cross-pool move.
use super::*;
use cross_pool_vm::Peer;
use xenadmin_core::{
    BatchVmWorkflow, CrossPoolBatchSelection, CrossPoolMode, CrossPoolVmOperation, DiskPlacement,
    NetworkId, NicPlacement, OperationId, PermissionAssessment, ResourceTarget, SrId, VdiId, VifId,
    VmWorkflowBatch,
};

pub(super) struct Editor {
    source: Peer,
    choices: Vec<Choice>,
    peers: Vec<Peer>,
    destination: Option<usize>,
    host: Option<HostId>,
    /// The one destination repository for every disk of the selection.
    repository: Option<SrId>,
    /// The destination network for each source network a selected VM has a NIC on.
    networks: BTreeMap<NetworkId, NetworkId>,
    transfer: Option<(NetworkId, String)>,
    /// The acknowledgement of host-to-host transport, given once for the whole batch.
    transport_reviewed: bool,
    stage: u8,
    review: Option<Review>,
    error: Option<String>,
}
/// A selected VM as it was when the editor opened.
struct Choice {
    source: Vm,
    affinity: Option<HostId>,
    disks: Vec<(VdiId, SrId)>,
    nics: Vec<(VifId, NetworkId)>,
    refusal: Option<String>,
}
#[derive(Clone, Debug, PartialEq)]
struct Review {
    batch: VmWorkflowBatch,
    /// For each VM: its name and UUID, how it moves, and where each disk and NIC goes.
    rows: Vec<Vec<String>>,
    /// What the whole selection asks of the destination repository.
    capacity: String,
}
fn edit(s: &mut State) -> &mut Editor {
    s.bulk_cross_pool.as_mut().expect("cross-pool batch editor")
}
fn choice(i: &Inventory, source: Vm) -> Choice {
    let mut refusal = None;
    if source.is_template || source.is_control_domain {
        refusal = Some("Only ordinary VMs can move to another pool".to_string());
    } else if !matches!(source.power_state, PowerState::Running | PowerState::Halted) {
        refusal = Some(format!(
            "The VM is {}. A running guest moves live and a halted guest moves halted; resume or shut this one down first",
            source.power_state
        ));
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
pub(super) fn open(s: &mut State) {
    let selected = match bulk_move::selected(s) {
        Ok(rows) => rows,
        Err(e) => {
            s.error = Some(e);
            return;
        }
    };
    let Some(i) = s.inventory.clone() else {
        return;
    };
    let pool = ResourceTarget::Pool(i.pool_id);
    let observed = s
        .snapshot
        .as_ref()
        .and_then(|v| v.observe_resource(pool).ok());
    let profile = profiles_view::inspection_connections(s)
        .find(|(_, c)| observed.is_some() && c.latest().observe_resource(pool).ok() == observed)
        .map(|(profile, _)| profile);
    // A review that could never be submitted is not opened. The synthetic session has no
    // connection at all and opens it to be looked at.
    if !s.demo && (!s.can_mutate() || profile.is_none()) {
        s.error = Some(
            "Moving VMs to another pool needs a current connection to this pool that can make changes"
                .into(),
        );
        return;
    }
    let peers = cross_pool_vm::peers(s)
        .into_iter()
        .filter(|p| p.inventory.pool_id != i.pool_id)
        .collect();
    s.bulk_cross_pool = Some(Editor {
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
    });
}
pub(super) fn button(s: &State, k: &Kit) -> El {
    Button::new("Move selection to another pool…")
        .disabled(bulk_move::selected(s).is_err())
        .on_press(open)
        .build(k)
        .key("batch-cross-pool-open")
}
fn destination(e: &Editor) -> Option<&Peer> {
    e.peers.get(e.destination?)
}
/// The source pool is still the connection that was captured and the one in front, and
/// so is the destination pool once one is chosen.
fn captured(s: &State, e: &Editor) -> bool {
    !s.demo
        && s.can_mutate()
        && s.inventory
            .as_ref()
            .is_some_and(|i| i.pool_id == e.source.inventory.pool_id)
        && e.source.connection(s).is_some()
        && s.snapshot.as_ref().and_then(|v| {
            v.observe_resource(ResourceTarget::Pool(e.source.inventory.pool_id))
                .ok()
        }) == e.source.observed
        && destination(e).is_none_or(|p| p.connection(s).is_some())
}
/// Both pools are chosen and are still the connections that were captured.
fn current(s: &State, e: &Editor) -> bool {
    destination(e).is_some() && captured(s, e)
}
/// A change of destination pool or host ends everything chosen for the previous one.
fn reset_mappings(e: &mut Editor) {
    e.repository = None;
    e.networks.clear();
    e.transfer = None;
    e.transport_reviewed = false;
    e.review = None;
    e.error = None;
}
fn choose_pool(s: &mut State, index: usize) {
    let e = edit(s);
    if index >= e.peers.len() {
        return;
    }
    e.destination = Some(index);
    e.host = None;
    reset_mappings(e);
}
fn choose_host(s: &mut State, host: HostId) {
    let e = edit(s);
    e.host = Some(host);
    reset_mappings(e);
}
/// The source networks the selection's NICs are on, each to be mapped once.
fn source_networks(e: &Editor) -> Vec<NetworkId> {
    let mut networks = e
        .choices
        .iter()
        .flat_map(|c| c.nics.iter().map(|(_, network)| *network))
        .collect::<Vec<_>>();
    networks.sort();
    networks.dedup();
    networks
}
/// What each pool's session is asked for: the source to send every VM, the destination
/// to prepare a receiver for each.
fn permissions(
    s: &State,
    e: &Editor,
    batch: &VmWorkflowBatch,
) -> (PermissionAssessment, Option<PermissionAssessment>) {
    let peer = destination(e)
        .and_then(|p| p.connection(s))
        .map(|c| c.latest().session_permissions())
        .unwrap_or_default();
    (
        session_permissions_view::assess(s, &batch.permission_requirements()),
        batch
            .peer_permission_requirements()
            .map(|required| required.assess(&peer)),
    )
}
/// The batch the current choices make from both pools as they are now, with each VM's
/// resulting mapping. An error names every VM that cannot be part of it; none is left out.
fn build_review(s: &State, e: &Editor) -> Result<Review, String> {
    let source = e
        .source
        .current(s)
        .filter(|i| {
            s.inventory
                .as_ref()
                .is_some_and(|front| front.pool_id == i.pool_id)
        })
        .ok_or("The captured source pool is no longer current")?;
    let peer = destination(e).ok_or("Choose a connected destination pool")?;
    let target = peer
        .current(s)
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
            vm_migration::net_label(&source, unmapped)
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
                                vm_migration::net_label(&source, *network)
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
                    vm_migration::sr_label(&source, disk.expected_sr),
                    vm_migration::sr_label(&target, disk.destination_sr)
                ));
            }
            for nic in &operation.networks {
                lines.push(format!(
                    "NIC {}: {} → {}",
                    nic.vif,
                    vm_migration::net_label(&source, nic.expected_network),
                    vm_migration::net_label(&target, nic.destination_network)
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
    let label = vm_migration::sr_label(&target, repository);
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
    let (sending, receiving) = permissions(s, e, &batch);
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
fn next(s: &mut State) {
    let Some(e) = &s.bulk_cross_pool else {
        return;
    };
    if e.stage == 0 {
        let chosen = e.destination.is_some() && e.host.is_some();
        let e = edit(s);
        if chosen {
            e.stage = 1;
            e.error = None;
        } else {
            e.error = Some("Choose a destination pool and host".into());
        }
        return;
    }
    // The acknowledgement belongs to the review, after the exact mappings are listed.
    edit(s).transport_reviewed = false;
    let e = s.bulk_cross_pool.as_ref().unwrap();
    match build_review(s, e) {
        Ok(review) => {
            let e = edit(s);
            e.review = Some(review);
            e.stage = 2;
            e.error = None;
        }
        Err(error) => edit(s).error = Some(error),
    }
}
fn back(s: &mut State) {
    let e = edit(s);
    e.stage = e.stage.saturating_sub(1);
    e.review = None;
    e.transport_reviewed = false;
    e.error = None;
}
/// Gives or withdraws the one acknowledgement, on every VM of the reviewed batch.
fn acknowledge(s: &mut State) {
    let e = edit(s);
    e.transport_reviewed = !e.transport_reviewed;
    let reviewed = e.transport_reviewed;
    if let Some(review) = &mut e.review {
        for item in &mut review.batch.items {
            if let xenadmin_core::VmWorkflow::MoveToPool(operation) = &mut item.workflow {
                operation.host_transport_reviewed = reviewed;
            }
        }
    }
}
fn submit(s: &mut State) {
    let result = (|| {
        let e = s
            .bulk_cross_pool
            .as_ref()
            .ok_or(xenadmin_core::Error::Stale)?;
        let r = e.review.as_ref().ok_or(xenadmin_core::Error::Stale)?;
        if !current(s, e)
            || !e.transport_reviewed
            || build_review(s, e).ok().as_ref() != Some(r)
            || s.resource_operations.len() >= 64
        {
            return Err(xenadmin_core::Error::Stale);
        }
        let peer = destination(e).ok_or(xenadmin_core::Error::Stale)?;
        let source = e.source.connection(s).ok_or(xenadmin_core::Error::Closed)?;
        let target = peer.connection(s).ok_or(xenadmin_core::Error::Closed)?;
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
                    source: e.source.observed.ok_or(xenadmin_core::Error::Stale)?,
                    destination: peer.observed.ok_or(xenadmin_core::Error::Stale)?,
                    batch: r.batch.clone(),
                },
                &target,
            )
            .map(|op| (op, e.source.inventory.pool_id, live))
    })();
    match result {
        Ok((op, pool, live)) => {
            if let Some(w) = &s.waker {
                op.notify_on_change(w.clone());
            }
            // As with the single live move, the console of a guest that will move live
            // is closed when the batch is submitted.
            if s.console_ui
                .target()
                .is_some_and(|t| t.pool == pool && live.contains(&t.vm))
            {
                console_view::release(s);
                s.console = None;
                s.console_snapshot = None;
                s.console_ui = Default::default();
            }
            s.resource_operations.push(op);
            s.bulk_cross_pool = None;
            resource_admin::open_pool(s, resource_admin::Kind::History);
        }
        Err(error) => edit(s).error = Some(error.to_string()),
    }
}
fn selector(
    label: String,
    key: String,
    selected: bool,
    k: &Kit,
    change: impl Fn(&mut State) + 'static,
) -> El {
    Button::new(format!("{}{label}", if selected { "✓ " } else { "" }))
        .variant(crate::theme::choice(selected))
        .on_press(change)
        .build(k)
        .key(key)
}
fn destination_stage(e: &Editor, mut body: El, k: &Kit) -> El {
    body = body.child(heading("1 of 3 · Destination pool and host", k));
    if e.peers.is_empty() {
        body = body.child(
            caption(
                "Connect another pool with write access using Connections, then reopen this review.",
                k,
            )
            .wrap(),
        );
    }
    for (index, p) in e.peers.iter().enumerate() {
        body = body.child(selector(
            format!("{} · {}", p.inventory.pool_name, p.inventory.pool_id),
            format!("batch-cross-pool-{index}"),
            e.destination == Some(index),
            k,
            move |s| choose_pool(s, index),
        ));
    }
    if let Some(d) = destination(e) {
        for host in d.inventory.hosts.values().filter(|h| h.enabled) {
            let id = host.id;
            body = body.child(selector(
                format!("Host: {} ({id})", host.name),
                format!("batch-cross-host-{id}"),
                e.host == Some(id),
                k,
                move |s| choose_host(s, id),
            ));
        }
    }
    body = body.child(heading("Captured VMs", k));
    for c in &e.choices {
        body = body.child(
            caption(
                format!(
                    "{} ({}) · {}",
                    c.source.name, c.source.id, c.source.power_state
                ),
                k,
            )
            .wrap_anywhere(),
        );
        if let Some(error) = &c.refusal {
            body = body.child(caption(error, k).wrap().color(k.c.danger.icon));
        }
    }
    body.child(caption("One destination pool and host take the whole selection. A running guest moves live and keeps running; a halted guest stays halted; both may be in one batch. A suspended or paused guest, a template or a guest the single cross-pool move would refuse makes the whole selection unavailable: none is left out. Copies are made one VM at a time from the VM's own cross-pool form.", k).wrap())
}
fn mapping_stage(e: &Editor, mut body: El, k: &Kit) -> El {
    body = body.child(heading(
        "2 of 3 · Storage, networks and transfer address",
        k,
    ));
    let Some(d) = destination(e) else {
        return body;
    };
    let i = &d.inventory;
    body = body
        .child(caption("One destination repository takes every disk of the selection, and each source network maps to one destination network for every NIC on it. The review lists the mapping this gives each VM. To map one VM differently, move it by itself.", k).wrap())
        .child(heading("Destination repository for every disk", k));
    for sr in i.storage.iter().filter(|sr| {
        i.details
            .storage
            .get(&SrId::from(sr.uuid))
            .is_some_and(|r| r.content_type == "user")
            && i.details.pbds.values().any(|p| {
                p.sr == Some(SrId::from(sr.uuid))
                    && p.host == e.host
                    && p.currently_attached == Some(true)
            })
    }) {
        let id = SrId::from(sr.uuid);
        body = body.child(selector(
            sr.name.clone(),
            format!("batch-cross-sr-{id}"),
            e.repository == Some(id),
            k,
            move |s| {
                let e = edit(s);
                e.repository = Some(id);
                e.error = None;
            },
        ));
    }
    for (index, network) in source_networks(e).into_iter().enumerate() {
        body = body.child(heading(
            format!(
                "Source network {}",
                vm_migration::net_label(&e.source.inventory, network)
            ),
            k,
        ));
        for n in &i.networks {
            let id = NetworkId::from(n.uuid);
            body = body.child(selector(
                n.name.clone(),
                format!("batch-cross-net-{index}-{id}"),
                e.networks.get(&network) == Some(&id),
                k,
                move |s| {
                    let e = edit(s);
                    e.networks.insert(network, id);
                    e.error = None;
                },
            ));
        }
    }
    body = body.child(heading("Destination transfer address", k));
    for p in i
        .details
        .pifs
        .values()
        .filter(|p| p.host == e.host && p.currently_attached == Some(true))
    {
        let Some(network) = p.network else {
            continue;
        };
        for address in std::iter::once(p.ip.clone())
            .chain(
                p.ipv6
                    .iter()
                    .map(|a| a.split('/').next().unwrap_or_default().into()),
            )
            .filter(|a| {
                a.parse::<std::net::IpAddr>()
                    .is_ok_and(|a| !a.is_unspecified() && !a.is_loopback())
            })
        {
            let value = (network, address.clone());
            body = body.child(selector(
                format!("{} · {address}", vm_migration::net_label(i, network)),
                format!("batch-cross-address-{network}-{address}"),
                e.transfer.as_ref() == Some(&value),
                k,
                move |s| {
                    let e = edit(s);
                    e.transfer = Some(value.clone());
                    e.error = None;
                },
            ));
        }
    }
    body.child(caption("Each VM must be an ordinary guest with exclusive owned data disks and empty CD drives. Snapshot chains, shared disks, mounted media, HA protection and vApps need separate preparation, as for a single cross-pool move.", k).wrap())
}
fn review_stage(s: &State, e: &Editor, review: &Review, mut body: El, k: &Kit) -> El {
    body = body.child(heading("3 of 3 · Review both pools", k));
    if let (Some(d), Some(host), Some((_, address))) = (destination(e), e.host, &e.transfer) {
        body = body.child(
            caption(
                format!(
                    "Move {} VMs to another pool\nDestination: {} ({})\nHost: {}\nHost-to-host address: {address}",
                    review.rows.len(),
                    d.inventory.pool_name,
                    d.inventory.pool_id,
                    vm_migration::host_label(&d.inventory, host),
                ),
                k,
            )
            .wrap_anywhere(),
        );
    }
    for (index, lines) in review.rows.iter().enumerate() {
        body = body.child(heading(format!("Target {}", index + 1), k));
        for line in lines {
            body = body.child(caption(line, k).wrap_anywhere());
        }
    }
    body = body
        .child(caption(&review.capacity, k).wrap_anywhere())
        .child(caption("XAPI moves each VM and removes its exclusive source disks. VM and device UUIDs and MAC addresses are kept; destination disk UUIDs may change.", k).wrap())
        .child(caption("Host-to-host transport follows the hosts' migration policy and may use unencrypted HTTP or TLS without certificate verification. Desktop certificate trust does not secure these transfers. Use a trusted migration network. The application does not force TLS or silently retry with a different policy.", k).wrap().color(k.c.danger.icon))
        .child(selector(
            "I reviewed the host-to-host transport policy for every VM in this batch".into(),
            "batch-cross-transport-reviewed".into(),
            e.transport_reviewed,
            k,
            acknowledge,
        ))
        .child(caption("Every VM is checked on both pools before the first transfer, and the destination repository must have room for the whole selection. The VMs then move one after another. Stop finishes the current VM's move and skips the rest; it does not ask the hosts to cancel a transfer. Both pools are reserved until the batch ends or is acknowledged.", k).wrap())
        .child(caption("After an interruption or a restart nothing continues by itself, because the receiver a move was prepared with is not kept. Reconnect both pools and use Recheck: it reads the moves that were started, never repeats a transfer and never starts a remaining VM. A move whose outcome cannot be read stays unknown until it is acknowledged, and partial source or destination resources may remain.", k).wrap());
    if review
        .batch
        .items
        .iter()
        .any(|i| i.expected_state == PowerState::Running)
    {
        body = body.child(caption("The console of a running guest in this selection closes when the batch is submitted. After its move is verified, open it from the destination pool.", k).wrap());
    }
    let (sending, receiving) = permissions(s, e, &review.batch);
    body = body
        .child(caption("Source pool session", k))
        .child(session_permissions_view::review_notice(&sending, k));
    if let Some(receiving) = receiving {
        body = body.child(caption("Destination pool session", k)).child(
            caption(receiving.explanation(), k)
                .wrap()
                .role("status")
                .key("review-permission-destination"),
        );
    }
    body
}
pub(super) fn dialog(s: &State, k: &Kit) -> El {
    let e = s.bulk_cross_pool.as_ref().unwrap();
    let mut body = col().w_full().gap(10.0).child(
        caption(
            format!(
                "{} · {}\n{} captured VMs",
                e.source.inventory.pool_name,
                e.source.inventory.pool_id,
                e.choices.len()
            ),
            k,
        )
        .wrap_anywhere(),
    );
    let proposal = (e.stage > 0).then(|| build_review(s, e));
    body = match (e.stage, &e.review) {
        (0, _) => destination_stage(e, body, k),
        (2, Some(review)) => review_stage(s, e, review, body, k),
        _ => mapping_stage(e, body, k),
    };
    if e.stage == 2
        && let Some(Err(reason)) = &proposal
    {
        body = body.child(
            caption(reason, k)
                .wrap()
                .color(k.c.danger.icon)
                .role("alert"),
        );
    } else if e.stage == 2 && proposal.as_ref().and_then(|p| p.as_ref().ok()) != e.review.as_ref() {
        body = body.child(
            caption(
                "A pool changed since this review was made. Go back and review again.",
                k,
            )
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
    if !s.demo && !captured(s, e) {
        body = body.child(
            caption(
                "A captured connection changed or its inventory is stale. Reopen this review.",
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
            .on_press(|s: &mut State| s.bulk_cross_pool = None)
            .build(k)
            .key("batch-cross-pool-close"),
    );
    if e.stage > 0 {
        footer = footer.child(
            Button::new("Back")
                .on_press(back)
                .build(k)
                .key("batch-cross-pool-back"),
        );
    }
    footer = footer.child(if e.stage == 2 {
        Button::new("Move reviewed VMs to the other pool")
            .disabled(
                !current(s, e)
                    || !e.transport_reviewed
                    || e.review.is_none()
                    || proposal.as_ref().and_then(|p| p.as_ref().ok()) != e.review.as_ref()
                    || s.resource_operations.len() >= 64,
            )
            .on_press(submit)
            .build(k)
            .key("batch-cross-pool-submit")
    } else {
        Button::new(if e.stage == 0 { "Next" } else { "Review moves" })
            .disabled(!s.demo && !captured(s, e))
            .on_press(next)
            .build(k)
            .key("batch-cross-pool-next")
    });
    let panel = col()
        .w((s.window_width - 24.0).clamp(280.0, 820.0))
        .h((s.window_height - 24.0).clamp(300.0, 900.0))
        .padding(16.0)
        .gap(10.0)
        .fill(k.c.bg_base)
        .radius(12.0)
        .child(heading(
            if e.stage == 2 {
                "Review moves to another pool"
            } else {
                "Move selected VMs to another pool"
            },
            k,
        ))
        .child(
            body.grow(1.0)
                .min_h(0.0)
                .scroll(rune::ScrollSpec {
                    stick_bottom: false,
                })
                .key("batch-cross-pool-body"),
        )
        .child(footer);
    el().absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .center()
        .fill(rune::Color::BLACK.alpha(0.55))
        .role("dialog")
        .label("Move selected VMs to another pool")
        .modal(false, |s: &mut State| s.bulk_cross_pool = None)
        .child(panel)
}
/// A synthetic review of a running and a halted VM moving to a second synthetic pool.
pub(super) fn scene(s: &mut State) {
    // The single form's scene makes the first VM and the pool's storage fit for a
    // transfer, and leaves the first VM running.
    cross_pool_vm::scene(s);
    s.cross_pool = Default::default();
    let first = VmId::parse("30000000-0000-0000-0000-000000000000").unwrap();
    let second = VmId::parse("30000000-0000-0000-0000-000000000001").unwrap();
    let i = Arc::make_mut(s.inventory.as_mut().unwrap());
    // The second VM is halted, with no resident host, and otherwise as plain as the first.
    let halted = i.vms.get_mut(&second).unwrap();
    halted.power_state = PowerState::Halted;
    halted.host = None;
    let details = i.details.vms.get_mut(&second).unwrap();
    details.ha_restart_priority = Some(String::new());
    details.snapshot_policy = None;
    details.appliance = None;
    details.appliance_known = true;
    details.affinity_known = true;
    details.blocked_operations.remove("migrate_send");
    details.current_operations.clear();
    details.allowed_operations = Some(vec!["migrate_send".into()]);
    // The synthetic inventory gives only the first VM devices. The second gets a disk and
    // a NIC of its own, so that the review has a mapping to list for each.
    let block = i
        .details
        .vbds
        .values()
        .find(|b| b.vm == Some(first) && b.kind == "Disk")
        .cloned()
        .unwrap();
    let mut disk = i.details.vdis[&block.vdi.unwrap()].clone();
    disk.id = VdiId::from(uuid::Uuid::from_u128(0x71000000000000000000000000000002));
    disk.name = "Demo data disk".into();
    disk.virtual_size = Some(8 << 30);
    let mut attachment = block;
    attachment.id =
        xenadmin_core::VbdId::from(uuid::Uuid::from_u128(0x72000000000000000000000000000011));
    attachment.vm = Some(second);
    attachment.vdi = Some(disk.id);
    attachment.currently_attached = Some(false);
    i.details
        .vdi_attachments
        .insert(disk.id, vec![attachment.id]);
    i.details.vdis.insert(disk.id, disk);
    i.details.vbds.insert(attachment.id, attachment);
    let mut nic = i
        .details
        .vifs
        .values()
        .find(|n| n.vm == Some(first))
        .cloned()
        .unwrap();
    nic.id = VifId::from(uuid::Uuid::from_u128(0x73000000000000000000000000000002));
    nic.vm = Some(second);
    nic.mac = "02:00:00:00:00:24".into();
    nic.currently_attached = Some(false);
    i.details.vifs.insert(nic.id, nic);
    let target = cross_pool_vm::scene_destination(s);
    s.rebuild();
    s.table.select(first.to_string());
    open(s);
    let i = s.inventory.clone().unwrap();
    let e = edit(s);
    e.source.inventory = i.clone();
    e.choices = vec![
        choice(&i, i.vms[&first].clone()),
        choice(&i, i.vms[&second].clone()),
    ];
    e.peers = vec![target];
    e.destination = Some(0);
    let dest = e.peers[0].inventory.clone();
    e.host = dest.hosts.values().find(|h| h.enabled).map(|h| h.id);
    e.repository = dest
        .storage
        .iter()
        .map(|sr| SrId::from(sr.uuid))
        .find(|id| {
            dest.details.pbds.values().any(|p| {
                p.sr == Some(*id) && p.host == e.host && p.currently_attached == Some(true)
            })
        });
    let network = dest.networks.first().map(|n| NetworkId::from(n.uuid));
    for source in source_networks(e) {
        if let Some(network) = network {
            e.networks.insert(source, network);
        }
    }
    e.transfer = dest
        .details
        .pifs
        .values()
        .find(|p| p.host == e.host && p.currently_attached == Some(true))
        .and_then(|p| Some((p.network?, p.ip.clone())));
    e.stage = 1;
    next(s);
    if edit(s).review.is_some() {
        acknowledge(s);
    }
}
#[cfg(test)]
#[path = "bulk_cross_pool_tests.rs"]
mod tests;
