//! The open surface, described. Nothing here changes the model: it is
//! rebuilt on every render from the editor and the model as they are.
//!
//! One wizard, `batch-export` (the name of its Rune scene), for VMs and for
//! snapshots: Format, Destinations, Review exports. The Rune dialog was one
//! form and its review; the form is the first two steps.
//!
//! Rune drew each list of alternatives as a row of buttons. Each is one
//! choice here and each Rune button key is the key of its option:
//!
//! - `batch-export-format-<0..4>` in the choice `batch-export-format`;
//! - `batch-export-memory-<n>-omit` and `-include` in
//!   `batch-export-memory-choice-<n>`.
//!
//! Other Rune keys kept: `batch-export-xva-only`, `batch-export-memory-<n>`
//! (the line that says what the choice means), `batch-export-choose-<n>`,
//! `batch-export-close`, `batch-export-back`, `batch-export-review`,
//! `batch-export-submit`, and `review-permission-explanation` (the footer
//! note).
//!
//! New keys, for elements Rune left without one: `batch-export-next`,
//! `batch-export-pool`, `batch-export-parent`, `batch-export-count`,
//! `batch-export-target-<n>`, `batch-export-output-<n>`,
//! `batch-export-plan` (and so `batch-export-plan-stale`),
//! `batch-export-refusal`, `batch-export-error`, `batch-export-pending`,
//! `batch-export-stale`, `batch-export-preview`.

use super::export::{self, Editor, Item, Step};
use crate::prelude::*;

/// The id of the wizard: the name of its Rune scene.
pub const SURFACE: &str = "batch-export";
pub const STEPS: [&str; 3] = ["Format", "Destinations", "Review exports"];

pub fn surfaces(m: &Model) -> Vec<Surface> {
    m.areas
        .bulk_export
        .editor
        .as_ref()
        .map(|e| wizard(m, e))
        .into_iter()
        .collect()
}

fn wizard(m: &Model, e: &Editor) -> Surface {
    let of_snapshots = e.of_snapshots();
    let proposal = export::proposal(m, e);
    let synthetic = m.is_synthetic_pool(e.pool);
    let current = export::current(m, e);
    let waiting = e.pending.is_some();
    let (step, refusal) = match (&e.review, e.step) {
        (Some(_), _) => (2, proposal.as_ref().err().cloned()),
        (None, Step::Destinations) => (1, proposal.as_ref().err().cloned()),
        (None, Step::Format) => (0, export::source_refusal(m, e)),
    };
    let stale = (e.review.is_some() && !synthetic)
        .then(|| export::blocked(m, e, &proposal))
        .flatten()
        .filter(|_| !current || proposal.as_ref().ok() != e.review.as_ref());
    let mut body = captured(e);
    match step {
        0 => body.extend(format(e)),
        1 => body.extend(targets(e, true)),
        _ => {
            body.extend(targets(e, false));
            body.extend(blocks![
                review(
                    "batch-export-plan",
                    [std::format!("Format: {}", export::format_name(&e.format))],
                )
                .stale(stale)
            ]);
        }
    }
    let (sequence, formats) = if of_snapshots {
        (export::SNAPSHOT_SEQUENCE, export::SNAPSHOT_FORMATS)
    } else {
        (export::VM_SEQUENCE, export::VM_FORMATS)
    };
    body.extend(blocks![
        caption(sequence),
        caption(formats),
        refusal.map(|reason| alert(reason).key("batch-export-refusal")),
        e.error
            .clone()
            .map(|error| alert(error).key("batch-export-error")),
        waiting.then(|| caption(export::WAITING).key("batch-export-pending")),
        synthetic.then(|| notice(NoticeKind::Preview, export::PREVIEW).key("batch-export-preview")),
        (!synthetic && !current)
            .then(|| notice(NoticeKind::Warning, export::STALE).key("batch-export-stale")),
    ]);
    let close = button("batch-export-close", "Cancel", export::close);
    let back = button("batch-export-back", "Back", export::back);
    let footer = match step {
        0 => vec![
            close,
            button("batch-export-next", "Next", export::next).primary(),
        ],
        1 => vec![
            close,
            back,
            button("batch-export-review", "Review exports", export::review)
                .primary()
                .disabled_opt(proposal.as_ref().err().cloned())
                .disabled_if(waiting, export::WAITING),
        ],
        _ => vec![
            close,
            back,
            button(
                "batch-export-submit",
                if of_snapshots {
                    "Export reviewed snapshots"
                } else {
                    "Export reviewed VMs"
                },
                export::submit,
            )
            .primary()
            .disabled_opt(export::blocked(m, e, &proposal)),
        ],
    };
    let title = match (of_snapshots, e.review.is_some()) {
        (false, false) => "Export selected VMs",
        (false, true) => "Review selected VM exports",
        (true, false) => "Export selected snapshots",
        (true, true) => "Review selected snapshot exports",
    };
    let surface = Surface::wizard(SURFACE, title, STEPS, step)
        .large()
        .experimental(true)
        .body(body)
        .footer(footer)
        .on_close(export::close);
    match &e.review {
        Some(reviewed) => surface.footer_note(permissions::footer_note(&permissions::assess_for(
            m,
            e.pool,
            &reviewed.batch.permission_requirements(),
        ))),
        None => surface,
    }
}

/// What was captured: the pool, the VM of the snapshots, how many.
fn captured(e: &Editor) -> Vec<Block> {
    let pool = caption(std::format!("{} · {}", e.pool_name, e.pool)).key("batch-export-pool");
    match e.targets.first() {
        Some(Item::Snapshot(first)) => blocks![
            pool,
            caption(std::format!(
                "Snapshots of {} · {}",
                first.parent_name,
                first.parent
            ))
            .key("batch-export-parent"),
            caption(std::format!("{} captured snapshots", e.targets.len()))
                .key("batch-export-count"),
        ],
        _ => blocks![
            pool,
            caption(std::format!("{} captured targets", e.targets.len()))
                .key("batch-export-count"),
        ],
    }
}

/// The first step: the format of every archive of the batch.
fn format(e: &Editor) -> Vec<Block> {
    let chosen = export::format_entry(&e.format).0.to_string();
    blocks![
        choice(
            "batch-export-format",
            export::formats(e).into_iter().map(|(index, label, _)| {
                option(index.to_string(), label).key(std::format!("batch-export-format-{index}"))
            }),
            Some(&chosen),
            export::pick_format,
        )
        .label("Format")
        .disabled_if(e.pending.is_some(), export::WAITING),
        export::has_checkpoint(&e.targets)
            .then(|| caption(export::XVA_ONLY).key("batch-export-xva-only")),
    ]
}

/// Every target under its heading: what it is, the memory choice of a
/// checkpoint, its output. While `editing` the choice can be made and the
/// destination chosen; the review shows them.
fn targets(e: &Editor, editing: bool) -> Vec<Block> {
    e.targets
        .iter()
        .enumerate()
        .map(|(index, target)| Block::from(one(e, index, target, editing)))
        .collect()
}

fn one(e: &Editor, index: usize, target: &Item, editing: bool) -> Group {
    let busy = e.pending.is_some();
    let lines = match target {
        Item::Vm(vm) => vec![std::format!(
            "{} · {:?}{}",
            vm.id,
            vm.power_state,
            if vm.is_template { " · template" } else { "" }
        )],
        Item::Snapshot(snapshot) => snapshot.lines(),
    };
    let memory = match target {
        Item::Snapshot(snapshot) if snapshot.memory.is_some() => {
            let choose = editing.then(|| {
                choice(
                    std::format!("batch-export-memory-choice-{index}"),
                    [
                        option("omit", "Disks only")
                            .key(std::format!("batch-export-memory-{index}-omit")),
                        option("include", "Include guest memory")
                            .key(std::format!("batch-export-memory-{index}-include")),
                    ],
                    Some(if snapshot.preserve_memory {
                        "include"
                    } else {
                        "omit"
                    }),
                    move |m, id| export::set_memory(m, index, id == "include"),
                )
                .label("Guest memory")
                .segmented()
                .disabled_if(busy, export::WAITING)
            });
            blocks![
                choose,
                caption(if snapshot.preserve_memory {
                    export::MEMORY_INCLUDED
                } else {
                    export::MEMORY_LEFT_OUT
                })
                .key(std::format!("batch-export-memory-{index}")),
            ]
        }
        _ => Vec::new(),
    };
    let file = e.files.get(index).and_then(Option::as_ref);
    group(
        std::format!("{} · {}", index + 1, target.name()),
        blocks![
            lines.into_iter().map(caption).collect::<Vec<_>>(),
            memory,
            file.map(|(path, _)| {
                caption(std::format!("Output: {}", path.display()))
                    .key(std::format!("batch-export-output-{index}"))
            }),
            editing.then(|| {
                button(
                    std::format!("batch-export-choose-{index}"),
                    if file.is_some() {
                        "Choose another destination"
                    } else {
                        "Choose destination"
                    },
                    move |m| export::choose(m, index),
                )
                .disabled_if(busy, export::WAITING)
            }),
        ],
    )
    .key(std::format!("batch-export-target-{index}"))
}
