//! The open surface, described. Nothing here changes the model: it is
//! rebuilt on every render from the editor and the model as they are.
//!
//! Two wizards, each with the three stages of its Rune dialog as steps.
//! `cross-pool` (the name of its scene) is one VM, moved or copied;
//! `batch-cross-pool` is several VMs, moved.
//!
//! Rune drew every list as a column of buttons with a check mark on the
//! chosen one. Each list is one choice here and each Rune button key is the
//! key of its option:
//!
//! - `cross-mode-<mode>` in the choice `cross-mode`, `cross-pool-<n>` in
//!   `cross-destination`, `cross-host-<host>` in `cross-host`;
//! - `cross-disk-<n>-<sr>` in `cross-disk-<n>`, `cross-nic-<n>-<network>`
//!   in `cross-nic-<n>`, `cross-address-<network>-<address>` in
//!   `cross-address`;
//! - `batch-cross-pool-<n>` in `batch-cross-destination`,
//!   `batch-cross-host-<host>` in `batch-cross-host`;
//! - `batch-cross-sr-<sr>` in `batch-cross-sr`,
//!   `batch-cross-net-<n>-<network>` in `batch-cross-net-<n>`,
//!   `batch-cross-address-<network>-<address>` in `batch-cross-address`.
//!
//! The two acknowledgement buttons (`cross-transport-reviewed`,
//! `batch-cross-transport-reviewed`) are checkboxes with the Rune key and
//! label.
//!
//! Other Rune keys kept: `cross-back`, `cross-next`, `cross-apply`,
//! `batch-cross-pool-close`, `batch-cross-pool-back`,
//! `batch-cross-pool-next`, `batch-cross-pool-submit`,
//! `review-permission-explanation` (the source pool's session) and
//! `review-permission-destination`.
//!
//! New keys, for elements Rune left without one: `cross-close` (Rune's
//! unkeyed "Close"), `cross-stale`, `cross-error`, `cross-no-peers`,
//! `cross-review` (and so `cross-review-stale`); `batch-cross-pool-source`,
//! `batch-cross-pool-no-peers`, `batch-cross-refusal-<n>`,
//! `batch-cross-pool-summary`, `batch-cross-pool-item-<n>`,
//! `batch-cross-pool-plan` (and so `batch-cross-pool-plan-stale`),
//! `batch-cross-pool-proposal`, `batch-cross-pool-error`,
//! `batch-cross-pool-stale`, `batch-cross-pool-preview`.

use xenadmin_core::{CrossPoolMode, Inventory, NetworkId, PowerState};

use super::{batch, single};
use crate::areas::migration::{host_label, net_label};
use crate::prelude::*;

/// The id of the wizard of one VM: the name of its Rune scene.
pub const WIZARD: &str = "cross-pool";
/// The id of the wizard of several VMs: the name of its Rune scene.
pub const BATCH: &str = "batch-cross-pool";

/// The Rune stage headings, without their "n of 3 ·".
pub const STEPS: [&str; 3] = [
    "Destination and mode",
    "Storage, NICs and transfer address",
    "Review both pools",
];
pub const BATCH_STEPS: [&str; 3] = [
    "Destination pool and host",
    "Storage, networks and transfer address",
    "Review both pools",
];

/// At most one of the two is open ([`super::State`]).
pub fn surfaces(m: &Model) -> Vec<Surface> {
    let state = &m.areas.cross_pool;
    let mut surfaces = Vec::new();
    surfaces.extend(state.wizard.as_deref().map(|e| one(m, e)));
    surfaces.extend(state.batch.as_ref().map(|e| several(m, e)));
    surfaces
}

/// The pools a destination is chosen from, each option keyed as its Rune
/// button was.
fn pools(peers: &[single::Peer], key: impl Fn(usize) -> String) -> Vec<OptionItem> {
    peers
        .iter()
        .enumerate()
        .map(|(index, p)| {
            option(
                index.to_string(),
                format!("{} · {}", p.inventory.pool_name, p.inventory.pool_id),
            )
            .key(key(index))
        })
        .collect()
}

/// The enabled hosts of the destination pool.
fn hosts(i: &Inventory, key: impl Fn(String) -> String) -> Vec<OptionItem> {
    i.hosts
        .values()
        .filter(|h| h.enabled)
        .map(|h| {
            option(h.id.to_string(), format!("Host: {} ({})", h.name, h.id))
                .key(key(h.id.to_string()))
        })
        .collect()
}

fn networks(i: &Inventory, key: impl Fn(NetworkId) -> String) -> Vec<OptionItem> {
    i.networks
        .iter()
        .map(|n| {
            let id = NetworkId::from(n.uuid);
            option(id.to_string(), n.name.clone()).key(key(id))
        })
        .collect()
}

/// The addresses the chosen host can receive a transfer on.
fn addresses(
    i: &Inventory,
    host: Option<xenadmin_core::HostId>,
    key: impl Fn(&(NetworkId, String)) -> String,
) -> Vec<OptionItem> {
    single::addresses(i, host)
        .into_iter()
        .map(|value| {
            option(
                single::address_id(&value),
                format!("{} · {}", net_label(i, value.0), value.1),
            )
            .key(key(&value))
        })
        .collect()
}

// -- One VM -------------------------------------------------------------------------

fn one(m: &Model, e: &single::Editor) -> Surface {
    let stale = (!single::current(m, e)).then(|| single::STALE.to_owned());
    let reviewing = e.stage >= 2 && e.review.is_some();
    let body = blocks![
        // The review has the stale line of its own.
        (!reviewing)
            .then(|| stale.clone())
            .flatten()
            .map(|stale| notice(NoticeKind::Warning, stale).key("cross-stale")),
        match e.stage {
            0 => destination(e),
            1 => mappings(e),
            _ => reviewed(m, e, stale),
        },
        e.error.clone().map(|error| alert(error).key("cross-error")),
    ];
    Surface::wizard(WIZARD, e.mode.label(), STEPS, usize::from(e.stage.min(2)))
        .subtitle(format!(
            "{}\nSource pool: {}\nVM: {}",
            e.vm.name, e.source.inventory.pool_name, e.vm.id
        ))
        .experimental(true)
        .body(body)
        .footer(footer(m, e))
        .on_close(single::close)
}

fn destination(e: &single::Editor) -> Vec<Block> {
    let halted = e.vm.power_state == PowerState::Halted;
    let mode = format!("{:?}", e.mode);
    let chosen = e.destination.map(|index| index.to_string());
    let host = e.host.map(|id| id.to_string());
    blocks![
        halted.then(|| {
            choice(
                "cross-mode",
                [CrossPoolMode::MoveHalted, CrossPoolMode::CopyHalted].map(|mode| {
                    option(format!("{mode:?}"), mode.label()).key(format!("cross-mode-{mode:?}"))
                }),
                Some(&mode),
                single::choose_mode,
            )
            .label("Mode")
        }),
        (!halted).then(|| caption(single::LIVE_ONLY)),
        e.peers
            .is_empty()
            .then(|| caption(single::NO_PEERS).key("cross-no-peers")),
        (!e.peers.is_empty()).then(|| {
            choice(
                "cross-destination",
                pools(&e.peers, |index| format!("cross-pool-{index}")),
                chosen.as_deref(),
                single::choose_pool,
            )
            .label("Destination pool")
        }),
        single::destination(e).map(|d| {
            choice(
                "cross-host",
                hosts(&d.inventory, |id| format!("cross-host-{id}")),
                host.as_deref(),
                single::choose_host,
            )
            .label("Destination host")
        }),
    ]
}

/// Nothing is mapped until it is chosen: a disk or an interface whose
/// destination is still the nil UUID has no option selected.
fn mappings(e: &single::Editor) -> Vec<Block> {
    let Some(d) = single::destination(e) else {
        return blocks![caption(single::LIMITS)];
    };
    let i = &d.inventory;
    let repositories = single::repositories(i, e.host);
    let disks = e.disks.iter().enumerate().map(|(index, disk)| {
        let chosen =
            (!disk.destination_sr.uuid().is_nil()).then(|| disk.destination_sr.to_string());
        choice(
            format!("cross-disk-{index}"),
            repositories.iter().map(|(id, name)| {
                option(id.to_string(), name.clone()).key(format!("cross-disk-{index}-{id}"))
            }),
            chosen.as_deref(),
            move |m, id| single::choose_disk(m, index, id),
        )
        .label(format!("Disk {}", disk.vdi))
    });
    let nics = e.networks.iter().enumerate().map(|(index, nic)| {
        let chosen = (!nic.destination_network.uuid().is_nil())
            .then(|| nic.destination_network.to_string());
        choice(
            format!("cross-nic-{index}"),
            networks(i, |id| format!("cross-nic-{index}-{id}")),
            chosen.as_deref(),
            move |m, id| single::choose_nic(m, index, id),
        )
        .label(format!("NIC {}", nic.vif))
    });
    let transfer = e.transfer.as_ref().map(single::address_id);
    blocks![
        disks.collect::<Vec<_>>(),
        nics.collect::<Vec<_>>(),
        choice(
            "cross-address",
            addresses(i, e.host, |(network, address)| {
                format!("cross-address-{network}-{address}")
            }),
            transfer.as_deref(),
            single::choose_address,
        )
        .label("Destination transfer address"),
        caption(single::LIMITS),
    ]
}

/// The review of both pools: its lines, the transport warning, what the
/// pools say against it now, and the acknowledgement that belongs to it.
fn reviewed(m: &Model, e: &single::Editor, stale: Option<String>) -> Vec<Block> {
    let Some(o) = &e.review else {
        return Vec::new();
    };
    blocks![
        review("cross-review", single::lines(e, o))
            .warning(single::TRANSPORT)
            .warnings(single::unavailable(m, e))
            .stale(stale),
        checkbox(
            "cross-transport-reviewed",
            single::ACKNOWLEDGE,
            e.transport_reviewed,
            single::toggle_transport,
        ),
    ]
}

fn footer(m: &Model, e: &single::Editor) -> Vec<Button> {
    let cancel = button("cross-close", "Cancel", single::close);
    let back = (e.stage > 0).then(|| button("cross-back", "Back", single::back));
    let forward = if e.stage == 2 {
        button("cross-apply", "Apply reviewed transfer", single::apply)
            .primary()
            .disabled_opt(single::blocked(m, e))
    } else {
        button("cross-next", "Next", single::next)
            .primary()
            .disabled_opt(single::step_blocked(m, e))
    };
    std::iter::once(cancel)
        .chain(back)
        .chain(std::iter::once(forward))
        .collect()
}

// -- Several VMs --------------------------------------------------------------------

fn several(m: &Model, e: &batch::Editor) -> Surface {
    let proposal = (e.stage > 0).then(|| batch::build_review(m, e));
    let synthetic = m.is_synthetic_pool(e.pool());
    let stale = (!synthetic && !batch::captured(m, e)).then(|| batch::STALE.to_owned());
    let reviewed = e.review.as_ref().filter(|_| e.stage == 2);
    let step = match (e.stage, reviewed) {
        (0, _) => chosen(e),
        (_, Some(review)) => plan(m, e, review, stale.clone()),
        _ => mapped(e),
    };
    // At the review: why what was reviewed is not what would be reviewed now.
    let outdated = match &proposal {
        Some(Err(reason)) if e.stage == 2 => Some(reason.clone()),
        Some(Ok(now)) if e.stage == 2 && Some(now) != e.review.as_ref() => {
            Some(batch::CHANGED.to_owned())
        }
        _ => None,
    };
    let body = blocks![
        caption(format!(
            "{} · {}\n{} captured VMs",
            e.source.inventory.pool_name,
            e.source.inventory.pool_id,
            e.choices.len()
        ))
        .key("batch-cross-pool-source"),
        step,
        outdated.map(|reason| alert(reason).key("batch-cross-pool-proposal")),
        e.error
            .clone()
            .map(|error| alert(error).key("batch-cross-pool-error")),
        // The review has the stale line of its own.
        reviewed
            .is_none()
            .then_some(stale)
            .flatten()
            .map(|stale| notice(NoticeKind::Warning, stale).key("batch-cross-pool-stale")),
        synthetic
            .then(|| notice(NoticeKind::Preview, batch::PREVIEW).key("batch-cross-pool-preview")),
    ];
    let close = button("batch-cross-pool-close", "Cancel", batch::close);
    let back = (e.stage > 0).then(|| button("batch-cross-pool-back", "Back", batch::back));
    let forward = if e.stage == 2 {
        button(
            "batch-cross-pool-submit",
            "Move reviewed VMs to the other pool",
            batch::submit,
        )
        .primary()
        .disabled_opt(batch::blocked(m, e, proposal.as_ref()))
    } else {
        let label = if e.stage == 0 { "Next" } else { "Review moves" };
        button("batch-cross-pool-next", label, batch::next)
            .primary()
            .disabled_opt(batch::step_blocked(m, e))
    };
    let title = if e.stage == 2 {
        "Review moves to another pool"
    } else {
        "Move selected VMs to another pool"
    };
    Surface::wizard(BATCH, title, BATCH_STEPS, usize::from(e.stage.min(2)))
        .large()
        .experimental(true)
        .body(body)
        .footer(
            std::iter::once(close)
                .chain(back)
                .chain(std::iter::once(forward))
                .collect::<Vec<_>>(),
        )
        .on_close(batch::close)
}

/// The first step: the destination pool and host, and what was captured.
fn chosen(e: &batch::Editor) -> Vec<Block> {
    let destination = e.destination.map(|index| index.to_string());
    let host = e.host.map(|id| id.to_string());
    let mut vms: Vec<Block> = Vec::new();
    for (index, c) in e.choices.iter().enumerate() {
        vms.push(
            caption(format!(
                "{} ({}) · {}",
                c.source.name, c.source.id, c.source.power_state
            ))
            .into(),
        );
        if let Some(reason) = &c.refusal {
            vms.push(
                alert(reason.clone())
                    .key(format!("batch-cross-refusal-{index}"))
                    .into(),
            );
        }
    }
    blocks![
        e.peers
            .is_empty()
            .then(|| caption(batch::NO_PEERS).key("batch-cross-pool-no-peers")),
        (!e.peers.is_empty()).then(|| {
            choice(
                "batch-cross-destination",
                pools(&e.peers, |index| format!("batch-cross-pool-{index}")),
                destination.as_deref(),
                batch::choose_pool,
            )
            .label("Destination pool")
        }),
        batch::destination(e).map(|d| {
            choice(
                "batch-cross-host",
                hosts(&d.inventory, |id| format!("batch-cross-host-{id}")),
                host.as_deref(),
                batch::choose_host,
            )
            .label("Destination host")
        }),
        heading("Captured VMs"),
        vms,
        caption(batch::SELECTION_HELP),
    ]
}

/// The second step: one repository for every disk, one destination network
/// for each source network, and the address the transfers go to.
fn mapped(e: &batch::Editor) -> Vec<Block> {
    let Some(d) = batch::destination(e) else {
        return Vec::new();
    };
    let i = &d.inventory;
    let repository = e.repository.map(|id| id.to_string());
    let nets = batch::source_networks(e)
        .into_iter()
        .enumerate()
        .map(|(index, network)| {
            let chosen = e.networks.get(&network).map(|id| id.to_string());
            choice(
                format!("batch-cross-net-{index}"),
                networks(i, |id| format!("batch-cross-net-{index}-{id}")),
                chosen.as_deref(),
                move |m, id| batch::choose_network(m, index, id),
            )
            .label(format!(
                "Source network {}",
                net_label(&e.source.inventory, network)
            ))
        });
    let transfer = e.transfer.as_ref().map(single::address_id);
    blocks![
        caption(batch::MAPPING_HELP),
        choice(
            "batch-cross-sr",
            single::repositories(i, e.host)
                .into_iter()
                .map(|(id, name)| option(id.to_string(), name).key(format!("batch-cross-sr-{id}"))),
            repository.as_deref(),
            batch::choose_repository,
        )
        .label("Destination repository for every disk"),
        nets.collect::<Vec<_>>(),
        choice(
            "batch-cross-address",
            addresses(i, e.host, |(network, address)| {
                format!("batch-cross-address-{network}-{address}")
            }),
            transfer.as_deref(),
            batch::choose_address,
        )
        .label("Destination transfer address"),
        caption(batch::MAPPING_LIMITS),
    ]
}

/// The review of both pools, in the Rune order: where the selection goes,
/// each target with its mapping, what the batch does as a whole with the
/// transport warning, the acknowledgement, and what each pool's session
/// may do.
fn plan(
    m: &Model,
    e: &batch::Editor,
    reviewed: &batch::Review,
    stale: Option<String>,
) -> Vec<Block> {
    let summary = match (batch::destination(e), e.host, &e.transfer) {
        (Some(d), Some(host), Some((_, address))) => Some(
            caption(format!(
                "Move {} VMs to another pool\nDestination: {} ({})\nHost: {}\nHost-to-host address: {address}",
                reviewed.rows.len(),
                d.inventory.pool_name,
                d.inventory.pool_id,
                host_label(&d.inventory, host),
            ))
            .key("batch-cross-pool-summary"),
        ),
        _ => None,
    };
    let targets = reviewed.rows.iter().enumerate().map(|(index, lines)| {
        group(
            format!("Target {}", index + 1),
            lines
                .iter()
                .map(|line| Block::from(caption(line.clone())))
                .collect(),
        )
        .key(format!("batch-cross-pool-item-{index}"))
    });
    let running = reviewed
        .batch
        .items
        .iter()
        .any(|item| item.expected_state == PowerState::Running);
    let mut lines = vec![
        reviewed.capacity.clone(),
        batch::MOVES.to_owned(),
        batch::SEQUENCE.to_owned(),
        batch::INTERRUPTION.to_owned(),
    ];
    if running {
        lines.push(batch::CONSOLE.to_owned());
    }
    let (sending, receiving) = batch::permissions(m, e, &reviewed.batch);
    let mut sessions: Vec<Block> = vec![
        caption("Source pool session").into(),
        permissions::footer_note(&sending).into(),
    ];
    if let Some(receiving) = receiving {
        sessions.push(caption("Destination pool session").into());
        sessions.push(
            caption(receiving.explanation())
                .key("review-permission-destination")
                .into(),
        );
    }
    blocks![
        summary,
        targets.collect::<Vec<_>>(),
        review("batch-cross-pool-plan", lines)
            .warning(batch::TRANSPORT)
            .stale(stale),
        checkbox(
            "batch-cross-transport-reviewed",
            batch::ACKNOWLEDGE,
            e.transport_reviewed,
            batch::acknowledge,
        ),
        sessions,
    ]
}
