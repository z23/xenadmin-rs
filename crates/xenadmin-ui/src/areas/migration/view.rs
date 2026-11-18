//! The open surface, described. Nothing here changes the model: it is
//! rebuilt on every render from the editor and the model as they are.
//!
//! Two wizards. `migration-dialog` (the Rune key of the dialog) is one VM:
//! the three Rune stages are its steps. `batch-move` is several VMs: the
//! draft and the review of the Rune dialog are its two steps.
//!
//! Rune drew every list as a column of buttons with a check mark on the
//! chosen one. Each list is one choice here and each Rune button key is the
//! key of its option:
//!
//! - `migration-host-<host>` in the choice `migration-host`;
//! - `migration-disk-<n>-<sr>` in `migration-disk-<n>`,
//!   `migration-nic-<n>-<network>` in `migration-nic-<n>`,
//!   `migration-transfer-<network>` in `migration-transfer`;
//! - `batch-move-host-<host>` in `batch-move-host`,
//!   `batch-move-transfer-<network>` in `batch-move-transfer`,
//!   `batch-move-disk-<n>-<d>-<sr>` in `batch-move-disk-<n>-<d>`,
//!   `batch-move-nic-<n>-<i>-<network>` in `batch-move-nic-<n>-<i>`.
//!
//! Other Rune keys kept: `migration-back`, `migration-next`,
//! `migration-apply`, `batch-move-mappings-<n>`, `batch-move-close`,
//! `batch-move-back`, `batch-move-review`, `batch-move-submit`, and
//! `review-permission-explanation` (the footer note).
//!
//! New keys, for elements Rune left without one: `migration-cancel`,
//! `migration-stale`, `migration-error`, `migration-no-disks`,
//! `migration-review` (and so `migration-review-stale`),
//! `migration-cross-pool`; `batch-move-pool`, `batch-move-target-<n>`,
//! `batch-move-refusal-<n>`, `batch-move-item-<n>`, `batch-move-plan` (and
//! so `batch-move-plan-stale`), `batch-move-proposal`, `batch-move-error`,
//! `batch-move-stale`, `batch-move-preview`.

use xenadmin_core::{NetworkId, PowerState, SrId};

use super::commands::MOVE_TO_POOL;
use super::{batch, net_label, sr_label, wizard};
use crate::prelude::*;

/// The id of the wizard of one VM: the key of the Rune dialog.
pub const WIZARD: &str = "migration-dialog";
/// The id of the wizard of several VMs: the name of its Rune scene.
pub const BATCH: &str = "batch-move";

pub const STEPS: [&str; 3] = [
    "Destination host",
    "Storage and networks",
    "Review migration",
];
pub const BATCH_STEPS: [&str; 2] = ["Destination and device mappings", "Review moves"];

/// At most one of the two is open ([`super::State`]).
pub fn surfaces(m: &Model) -> Vec<Surface> {
    let state = &m.areas.migration;
    let mut surfaces = Vec::new();
    surfaces.extend(state.wizard.as_deref().map(|f| single(m, f)));
    surfaces.extend(state.batch.as_ref().map(|e| several(m, e)));
    surfaces
}

// -- One VM -------------------------------------------------------------------------

fn single(m: &Model, f: &wizard::Editor) -> Surface {
    let stale = (!wizard::current(m, f)).then(|| wizard::STALE.to_owned());
    let body = blocks![
        // The review has the stale line of its own.
        (f.stage < 2)
            .then(|| stale.clone())
            .flatten()
            .map(|stale| notice(NoticeKind::Warning, stale).key("migration-stale")),
        match f.stage {
            0 => destination(f),
            1 => placement(m, f),
            _ => reviewed(m, f, stale),
        },
        f.error
            .clone()
            .map(|error| alert(error).key("migration-error")),
    ];
    let title = if f.source.power_state == PowerState::Running {
        "Migrate running VM"
    } else {
        "Move halted VM"
    };
    let surface = Surface::wizard(WIZARD, title, STEPS, usize::from(f.stage.min(2)))
        .subtitle(format!("{}\nVM UUID: {}", f.source.name, f.source.id))
        .body(body)
        .footer(single_footer(m, f))
        .on_close(wizard::close);
    // The permission line belongs to the review: before it there is no
    // operation to assess.
    match &f.review {
        Some(o) => surface.footer_note(permissions::footer_note(&permissions::assess_for(
            m,
            f.pool,
            &o.permission_requirements(),
        ))),
        None => surface,
    }
}

/// Every host of the captured inventory, the current one and disabled ones
/// included: the core decides.
fn destination(f: &wizard::Editor) -> Vec<Block> {
    let chosen = f.destination.map(|id| id.to_string());
    blocks![
        caption(wizard::DESTINATION_HELP),
        choice(
            "migration-host",
            f.initial.hosts.values().map(|host| {
                option(host.id.to_string(), host.name.clone())
                    .key(format!("migration-host-{}", host.id))
            }),
            chosen.as_deref(),
            wizard::choose_host,
        )
        .label("Destination host"),
    ]
}

fn placement(m: &Model, f: &wizard::Editor) -> Vec<Block> {
    let i = &f.initial;
    let disks = f.disks.iter().enumerate().map(|(index, disk)| {
        let name = i.details.vdis.get(&disk.vdi).map_or("Disk", |d| &d.name);
        let chosen = disk.destination_sr.to_string();
        choice(
            format!("migration-disk-{index}"),
            repositories(f, |id| format!("migration-disk-{index}-{id}")),
            Some(&chosen),
            move |m, id| wizard::choose_disk(m, index, id),
        )
        .label(format!("{name} · {}", disk.vdi))
        .dropdown()
    });
    let nics = f.networks.iter().enumerate().map(|(index, nic)| {
        let chosen = nic.destination_network.to_string();
        choice(
            format!("migration-nic-{index}"),
            networks(f, |id| format!("migration-nic-{index}-{id}")),
            Some(&chosen),
            move |m, id| wizard::choose_nic(m, index, id),
        )
        .label(format!("Network interface {}", nic.vif))
        .dropdown()
    });
    let transfer = f.transfer.map(|id| id.to_string());
    blocks![
        caption(wizard::PLACEMENT_HELP),
        disks.collect::<Vec<_>>(),
        f.disks
            .is_empty()
            .then(|| caption(wizard::NO_DISKS).key("migration-no-disks")),
        nics.collect::<Vec<_>>(),
        choice(
            "migration-transfer",
            networks(f, |id| format!("migration-transfer-{id}")),
            transfer.as_deref(),
            wizard::choose_transfer,
        )
        .label("Transfer network")
        .dropdown(),
        caption(wizard::TRANSFER_HELP),
        caption(wizard::TRANSPORT_LIMITS),
        cross_pool(m, f),
    ]
}

/// Every repository of the captured inventory as an option, keyed as the
/// Rune button was.
fn repositories(f: &wizard::Editor, key: impl Fn(SrId) -> String) -> Vec<OptionItem> {
    f.initial
        .storage
        .iter()
        .map(|sr| {
            let id = SrId::from(sr.uuid);
            option(id.to_string(), sr.name.clone()).key(key(id))
        })
        .collect()
}

fn networks(f: &wizard::Editor, key: impl Fn(NetworkId) -> String) -> Vec<OptionItem> {
    f.initial
        .networks
        .iter()
        .map(|network| {
            let id = NetworkId::from(network.uuid);
            option(id.to_string(), network.name.clone()).key(key(id))
        })
        .collect()
}

/// The way to the other pool that the caption above it names: the command
/// of the `cross_pool` area on the same VM. It follows that command's
/// eligibility: absent while the command is hidden (or its area has not
/// registered it), disabled with the command's reason. The wizard gives
/// way to what the command opens; refused after all, it stays and
/// `m.error` has the reason.
fn cross_pool(m: &Model, f: &wizard::Editor) -> Option<Button> {
    let target = Target::one(ObjectRef::vm(f.pool, f.source.id));
    let reason = match m.eligibility(MOVE_TO_POOL, &target) {
        Eligibility::Hidden => return None,
        Eligibility::Disabled(reason) => Some(reason),
        Eligibility::Enabled => None,
    };
    Some(
        button(
            "migration-cross-pool",
            "Move / copy to another pool…",
            move |m| {
                let mine = m.areas.migration.wizard.take();
                if !m.invoke(MOVE_TO_POOL, &target) {
                    m.areas.migration.wizard = mine;
                }
            },
        )
        .disabled_opt(reason),
    )
}

fn reviewed(m: &Model, f: &wizard::Editor, stale: Option<String>) -> Vec<Block> {
    let Some(o) = &f.review else {
        return Vec::new();
    };
    blocks![
        review("migration-review", wizard::lines(f, o))
            .warnings(wizard::unavailable(m, f))
            .stale(stale)
    ]
}

fn single_footer(m: &Model, f: &wizard::Editor) -> Vec<Button> {
    let cancel = button("migration-cancel", "Cancel", wizard::close);
    let back = (f.stage > 0).then(|| button("migration-back", "Back", wizard::back));
    let forward = if f.stage == 2 {
        button("migration-apply", "Apply migration", wizard::apply)
            .primary()
            .disabled_opt(wizard::blocked(m, f))
    } else {
        let label = if f.stage == 0 {
            "Next"
        } else {
            "Review migration"
        };
        button("migration-next", label, wizard::next)
            .primary()
            .disabled_opt(wizard::step_blocked(m, f))
    };
    std::iter::once(cancel)
        .chain(back)
        .chain(std::iter::once(forward))
        .collect()
}

// -- Several VMs --------------------------------------------------------------------

fn several(m: &Model, e: &batch::Editor) -> Surface {
    let proposal = batch::build_review(m, e);
    let synthetic = m.is_synthetic_pool(e.pool);
    let stale = (!synthetic && !batch::current(m, e)).then(|| batch::STALE.to_owned());
    let step = match &e.review {
        Some(review) => plan(review, stale.clone()),
        None => draft(e),
    };
    let body = blocks![
        caption(format!(
            "{} · {}\n{} captured VMs",
            e.name,
            e.pool,
            e.choices.len()
        ))
        .key("batch-move-pool"),
        step,
        proposal
            .as_ref()
            .err()
            .map(|reason| alert(reason.clone()).key("batch-move-proposal")),
        e.error
            .clone()
            .map(|error| alert(error).key("batch-move-error")),
        // The review has the stale line of its own.
        e.review
            .is_none()
            .then_some(stale)
            .flatten()
            .map(|stale| notice(NoticeKind::Warning, stale).key("batch-move-stale")),
        synthetic.then(|| notice(NoticeKind::Preview, batch::PREVIEW).key("batch-move-preview")),
    ];
    let close = button("batch-move-close", "Cancel", batch::close);
    let footer = if e.review.is_some() {
        vec![
            close,
            button("batch-move-back", "Back", batch::back),
            button("batch-move-submit", "Move reviewed VMs", batch::submit)
                .primary()
                .disabled_opt(batch::blocked(m, e, &proposal)),
        ]
    } else {
        vec![
            close,
            button("batch-move-review", "Review moves", batch::review)
                .primary()
                .disabled_opt(proposal.as_ref().err().cloned()),
        ]
    };
    let (title, current) = if e.review.is_some() {
        ("Review selected VM moves", 1)
    } else {
        ("Move selected VMs", 0)
    };
    let surface = Surface::wizard(BATCH, title, BATCH_STEPS, current)
        .large()
        .experimental(true)
        .body(body)
        .footer(footer)
        .on_close(batch::close);
    match &e.review {
        Some(review) => surface.footer_note(permissions::footer_note(&permissions::assess_for(
            m,
            e.pool,
            &review.batch.permission_requirements(),
        ))),
        None => surface,
    }
}

fn draft(e: &batch::Editor) -> Vec<Block> {
    let i = &e.initial;
    let destination = e.destination.map(|id| id.to_string());
    let transfer = e.transfer.map(|id| id.to_string());
    blocks![
        choice(
            "batch-move-host",
            i.hosts.values().map(|host| {
                option(host.id.to_string(), host.name.clone())
                    .key(format!("batch-move-host-{}", host.id))
                    .disabled_if(!host.enabled, batch::HOST_DISABLED)
            }),
            destination.as_deref(),
            batch::choose_host,
        )
        .label("Destination host"),
        choice(
            "batch-move-transfer",
            i.networks.iter().map(|network| {
                let id = NetworkId::from(network.uuid);
                option(id.to_string(), network.name.clone())
                    .key(format!("batch-move-transfer-{id}"))
            }),
            transfer.as_deref(),
            batch::choose_transfer,
        )
        .label("Transfer network")
        .dropdown(),
        caption(batch::TRANSFER_HELP),
        heading("Captured targets and device mappings"),
        e.choices
            .iter()
            .enumerate()
            .map(|(index, c)| captured(e, index, c))
            .collect::<Vec<_>>(),
    ]
}

/// One captured VM: what it is, why it cannot move if it cannot, and its
/// device mappings behind the Rune button that opens them.
fn captured(e: &batch::Editor, index: usize, c: &batch::Choice) -> Group {
    let i = &e.initial;
    let open = e.expanded == Some(index);
    let disks = c.disks.iter().enumerate().map(|(disk_index, disk)| {
        let chosen = disk.destination_sr.to_string();
        choice(
            format!("batch-move-disk-{index}-{disk_index}"),
            i.storage.iter().map(|sr| {
                let id = SrId::from(sr.uuid);
                option(id.to_string(), sr.name.clone())
                    .key(format!("batch-move-disk-{index}-{disk_index}-{id}"))
            }),
            Some(&chosen),
            move |m, id| batch::choose_disk(m, index, disk_index, id),
        )
        .label(format!(
            "Disk {} · current {}",
            disk.vdi,
            sr_label(i, disk.expected_sr)
        ))
        .dropdown()
    });
    let nics = c.networks.iter().enumerate().map(|(nic_index, nic)| {
        let chosen = nic.destination_network.to_string();
        choice(
            format!("batch-move-nic-{index}-{nic_index}"),
            i.networks.iter().map(|network| {
                let id = NetworkId::from(network.uuid);
                option(id.to_string(), network.name.clone())
                    .key(format!("batch-move-nic-{index}-{nic_index}-{id}"))
            }),
            Some(&chosen),
            move |m, id| batch::choose_nic(m, index, nic_index, id),
        )
        .label(format!(
            "NIC {} · current {}",
            nic.vif,
            net_label(i, nic.expected_network)
        ))
        .dropdown()
    });
    let mappings = if open {
        blocks![
            disks.collect::<Vec<_>>(),
            nics.collect::<Vec<_>>(),
            caption(batch::MEDIA),
        ]
    } else {
        Vec::new()
    };
    group(
        format!(
            "{} ({}) · {}",
            c.source.name, c.source.id, c.source.power_state
        ),
        blocks![
            c.refusal
                .clone()
                .map(|reason| alert(reason).key(format!("batch-move-refusal-{index}"))),
            button(
                format!("batch-move-mappings-{index}"),
                if open {
                    "Close device mappings"
                } else {
                    "Review / edit device mappings"
                },
                move |m| batch::toggle_mappings(m, index),
            ),
            mappings,
        ],
    )
    .key(format!("batch-move-target-{index}"))
}

/// The review: the lines of each target under its heading, then what the
/// batch does as a whole.
fn plan(reviewed: &batch::Review, stale: Option<String>) -> Vec<Block> {
    let targets = reviewed.rows.iter().enumerate().map(|(index, lines)| {
        group(
            format!("Target {}", index + 1),
            lines
                .iter()
                .map(|line| Block::from(caption(line.clone())))
                .collect(),
        )
        .key(format!("batch-move-item-{index}"))
    });
    blocks![
        targets.collect::<Vec<_>>(),
        review("batch-move-plan", [batch::SEQUENCE]).stale(stale),
    ]
}
