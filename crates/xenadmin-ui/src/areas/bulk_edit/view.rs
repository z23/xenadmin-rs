//! What the area shows: the dialog of the batch deletion and the page
//! "Bulk properties" of the Properties dialog of several VMs or templates.
//!
//! Rune keys are kept. Keys that are new here, for elements Rune left
//! unkeyed or that did not exist:
//!
//! - deletion: `batch-delete-pool` (pool and count), `batch-delete-target-<vm>`
//!   (the group of one target in the draft), `batch-delete-item-<n>` (the
//!   lines of one target in the review), `batch-delete-plan` (what the batch
//!   does as a whole, with the stale line), `batch-delete-proposal` (the live
//!   refusal), `batch-delete-error`, `batch-delete-stale`,
//!   `batch-delete-preview`, `batch-delete-back`;
//! - bulk properties: `batch-property` (the choice of the property; its
//!   options have the Rune keys `batch-property-<Property>`),
//!   `batch-property-host` (the home host), `batch-property-toggle` (its
//!   options have the Rune keys `batch-property-toggle-<false|true>`),
//!   `batch-property-note`, `batch-properties-pool`,
//!   `batch-properties-row-<vm>` (one target of the preview or the review),
//!   `batch-properties-plan` (the review), `batch-properties-proposal` (the
//!   live refusal), `batch-properties-read-only`, `batch-properties-error`,
//!   `batch-properties-refused` (a selection the page cannot edit).

use super::property::Property as Setting;
use super::{delete, edit};
use crate::areas::vm_lifecycle;
use crate::prelude::*;

pub fn surfaces(m: &Model) -> Vec<Surface> {
    m.areas
        .bulk_edit
        .delete
        .as_ref()
        .map(|e| deletion(m, e))
        .into_iter()
        .collect()
}

// -- Deleting several ---------------------------------------------------------------

fn deletion(m: &Model, e: &delete::Editor) -> Surface {
    let proposal = delete::build_review(m, e);
    let synthetic = m.is_synthetic_pool(e.pool);
    let stale = (!synthetic && !delete::current(m, e)).then(|| delete::STALE.to_owned());
    let step = match &e.review {
        Some(review) => plan(e, review, stale.clone()),
        None => draft(e),
    };
    let body = blocks![
        heading(if e.review.is_some() {
            "Review selected deletions"
        } else {
            "Delete selected VMs / templates"
        }),
        caption(format!(
            "{} · {}\n{} captured targets",
            e.pool_name,
            e.pool,
            e.choices.len()
        ))
        .key("batch-delete-pool"),
        step,
        proposal
            .as_ref()
            .err()
            .map(|reason| alert(reason.clone()).key("batch-delete-proposal")),
        e.error
            .clone()
            .map(|error| alert(error).key("batch-delete-error")),
        // The review has the stale line of its own.
        e.review
            .is_none()
            .then_some(stale)
            .flatten()
            .map(|stale| notice(NoticeKind::Warning, stale).key("batch-delete-stale")),
        synthetic.then(|| notice(NoticeKind::Preview, delete::PREVIEW).key("batch-delete-preview")),
    ];
    let close = button("batch-delete-close", "Cancel", delete::close);
    let footer = if e.review.is_some() {
        vec![
            close,
            button("batch-delete-back", "Back", delete::back),
            button(
                "batch-delete-submit",
                "Delete reviewed targets",
                delete::submit,
            )
            .danger()
            .disabled_opt(delete::blocked(m, e, &proposal)),
        ]
    } else {
        vec![
            close,
            button("batch-delete-review", "Review deletions", delete::review)
                .primary()
                .disabled_opt(proposal.err()),
        ]
    };
    Surface::dialog("batch-delete", "Batch VM deletion")
        .experimental(true)
        .large()
        .body(body)
        .footer(footer)
        .on_close(delete::close)
}

/// The draft: for each target what besides its record may be deleted. A
/// disk or snapshot that cannot be chosen says why it is retained.
fn draft(e: &delete::Editor) -> Vec<Block> {
    let targets = e.choices.iter().enumerate().map(|(index, c)| {
        let candidates = e.initial.vm_deletion_disks(c.vm.id);
        let mut items = blocks![caption(format!("{} · {}", c.vm.id, c.vm.power_state))];
        for id in delete::data_disks(e, c) {
            items.push(Block::from(
                checkbox(
                    format!("batch-delete-disk-{id}"),
                    format!(
                        "Delete disk data: {}",
                        vm_lifecycle::disk_label(&e.initial, id)
                    ),
                    c.disks.contains(&id),
                    move |m| delete::toggle_disk(m, index, id),
                )
                .disabled_if(!candidates.contains(&id), delete::DISK_RETAINED),
            ));
        }
        for snapshot in e.initial.snapshots.for_vm(c.vm.id) {
            let id = snapshot.id;
            items.push(Block::from(
                checkbox(
                    format!("batch-delete-snapshot-{id}"),
                    format!("Delete snapshot: {} ({id})", snapshot.name),
                    c.snapshots.contains(&id),
                    move |m| delete::toggle_snapshot(m, index, id),
                )
                .disabled_if(
                    !delete::snapshot_eligible(snapshot),
                    delete::SNAPSHOT_RETAINED,
                ),
            ));
            if !c.snapshots.contains(&id) {
                continue;
            }
            for disk in e.initial.snapshot_deletion_disks(id) {
                items.push(Block::from(checkbox(
                    format!("batch-delete-snapshot-disk-{disk}"),
                    format!(
                        "Delete snapshot data: {}",
                        vm_lifecycle::disk_label(&e.initial, disk)
                    ),
                    c.snapshot_disks.contains(&disk),
                    move |m| delete::toggle_snapshot_disk(m, index, disk),
                )));
            }
        }
        group(c.vm.name.clone(), items).key(format!("batch-delete-target-{}", c.vm.id))
    });
    blocks![
        caption(delete::RECORD_ONLY),
        targets.collect::<Vec<_>>()
    ]
}

/// The review: the lines of each target under its heading, what the batch
/// does as a whole, and the typed confirmation.
fn plan(e: &delete::Editor, reviewed: &delete::Review, stale: Option<String>) -> Vec<Block> {
    let targets = reviewed.rows.iter().enumerate().map(|(index, lines)| {
        group(
            format!("Target {}", index + 1),
            lines
                .iter()
                .map(|line| Block::from(caption(line.clone())))
                .collect(),
        )
        .key(format!("batch-delete-item-{index}"))
    });
    blocks![
        targets.collect::<Vec<_>>(),
        review("batch-delete-plan", [delete::SEQUENCE]).stale(stale),
        caption(format!(
            "Type {} to confirm irreversible deletion.",
            delete::confirmation(e)
        )),
        input(
            "batch-delete-confirm",
            "Deletion confirmation",
            &e.confirm,
            delete::set_confirm,
        ),
    ]
}

// -- Bulk properties ----------------------------------------------------------------

/// Several VMs or several templates have the one page "Bulk properties"
/// (the kernel gives them the dialog of a VM or of a template). It is
/// offered for every such selection: one the page cannot edit (too many,
/// several pools) says so on the page, where Rune disabled its button
/// without a reason. Outside the supported core, as the Rune dialog was.
pub fn pages(_m: &Model, target: &Target) -> Vec<PropertyPage> {
    let several = target.is_multi()
        && matches!(
            target.common_kind(),
            Some(ObjectKind::Vm | ObjectKind::Template)
        );
    if several {
        vec![PropertyPage::BULK.experimental(true)]
    } else {
        Vec::new()
    }
}

pub fn page(m: &Model, target: &Target, id: &str) -> PageView {
    if id != PropertyPage::BULK.id {
        return PageView::default();
    }
    let close = button("batch-properties-close", "Close", |m| {
        m.close_properties();
    });
    let Some(e) = m.areas.bulk_edit.properties.as_ref() else {
        let reason = edit::selected(m, target)
            .err()
            .unwrap_or_else(|| edit::STALE.into());
        return PageView::new(blocks![alert(reason).key("batch-properties-refused")])
            .footer([close]);
    };
    let proposal = edit::proposed(m, e);
    let permission = e
        .review
        .as_ref()
        .map(|r| permissions::assess_for(m, e.pool, &r.batch.permission_requirements()));
    let body = blocks![
        heading(if e.review.is_some() {
            "Review bulk property changes"
        } else {
            "Properties for selected targets"
        }),
        caption(format!(
            "{} · {}\n{} targets captured",
            e.pool_name,
            e.pool,
            e.targets.len()
        ))
        .key("batch-properties-pool"),
        if e.review.is_none() {
            fields(e)
        } else {
            Vec::new()
        },
        caption(e.property.note()).key("batch-property-note"),
        rows(e, &proposal),
        proposal
            .as_ref()
            .err()
            .map(|error| alert(error.clone()).key("batch-properties-proposal")),
        (!m.is_synthetic_pool(e.pool) && !m.can_mutate_pool(e.pool))
            .then(|| caption(edit::NOT_WRITABLE).key("batch-properties-read-only")),
        e.error
            .clone()
            .map(|error| alert(error).key("batch-properties-error")),
    ];
    let footer = if e.review.is_some() {
        let denial = permission.as_ref().and_then(permissions::denial);
        vec![
            close,
            button("batch-properties-back", "Back", edit::back),
            button("batch-properties-submit", "Apply changes", edit::submit)
                .primary()
                .disabled_opt(edit::blocked(m, e, &proposal, denial)),
        ]
    } else {
        let refusal = match &proposal {
            Err(reason) => Some(reason.clone()),
            Ok(r) if r.batch.vms.is_empty() => Some(edit::NOTHING_TO_CHANGE.to_owned()),
            Ok(_) => None,
        };
        vec![
            close,
            button("batch-properties-review", "Review changes", edit::review)
                .primary()
                .disabled_opt(refusal),
        ]
    };
    PageView {
        body,
        footer,
        footer_note: permission.as_ref().map(permissions::footer_note),
    }
}

/// The draft: which property, and its value. Rune had a button that opened
/// a list of fourteen buttons; here it is one choice.
fn fields(e: &edit::Editor) -> Vec<Block> {
    let current = e.property.id();
    let picker = choice(
        "batch-property",
        Setting::ALL.map(|p| option(p.id(), p.label())),
        Some(current.as_str()),
        |m, id| {
            if let Some(property) = Setting::from_id(id) {
                edit::choose(m, property);
            }
        },
    )
    .label("Property")
    .dropdown();
    let values = e.property.labels().iter().enumerate().map(|(n, label)| {
        input(
            format!("batch-property-value-{n}"),
            *label,
            &e.values[n],
            move |m, text| edit::set_value(m, n, text),
        )
    });
    let special = match e.property {
        Setting::HomeHost => {
            let chosen = e.host.map(|host| host.to_string());
            let options = std::iter::once(option("none", "No preference")).chain(
                e.hosts
                    .iter()
                    .map(|(id, name)| option(id.to_string(), format!("{name} ({id})"))),
            );
            Some(
                choice(
                    "batch-property-host",
                    options,
                    Some(chosen.as_deref().unwrap_or("none")),
                    |m, id| {
                        let host = m.areas.bulk_edit.properties.as_ref().and_then(|e| {
                            e.hosts
                                .iter()
                                .map(|(host, _)| *host)
                                .find(|host| host.to_string() == id)
                        });
                        if host.is_some() || id == "none" {
                            edit::set_host(m, host);
                        }
                    },
                )
                .label(e.property.label()),
            )
        }
        Setting::Autostart | Setting::SecureBoot | Setting::Firmware => {
            let (off, on) = if e.property == Setting::Firmware {
                ("BIOS", "UEFI")
            } else {
                ("Disabled", "Enabled")
            };
            Some(
                choice(
                    "batch-property-toggle",
                    [option("false", off), option("true", on)],
                    Some(if e.toggle { "true" } else { "false" }),
                    |m, id| edit::set_toggle(m, id == "true"),
                )
                .label(e.property.label())
                .segmented(),
            )
        }
        _ => None,
    };
    blocks![picker, values.collect::<Vec<_>>(), special]
}

/// Every captured target with what happens to it: the stored review, else
/// the live proposal while it is valid, else only who was captured.
fn rows(e: &edit::Editor, proposal: &Result<edit::Review, String>) -> Vec<Block> {
    let Some(r) = e.review.as_ref().or(proposal.as_ref().ok()) else {
        return e
            .targets
            .iter()
            .map(|target| {
                Block::from(caption(format!(
                    "{} · {} · {}",
                    target.name, target.power_state, target.id
                )))
            })
            .collect();
    };
    let count = format!(
        "{} changes · {} already desired",
        r.batch.vms.len(),
        r.rows.len() - r.batch.vms.len()
    );
    let targets = r.rows.iter().map(|row| {
        property(
            row.name.clone(),
            format!(
                "{} · {}\n{}{}",
                row.id,
                row.state,
                if row.changed {
                    ""
                } else {
                    "Already desired — excluded from changes\n"
                },
                row.summary
            ),
        )
        .key(format!("batch-properties-row-{}", row.id))
    });
    if e.review.is_some() {
        let stale = (proposal.as_ref() != Ok(r)).then(|| edit::REVIEW_CHANGED.to_owned());
        blocks![
            review("batch-properties-plan", [count, edit::SEQUENCE.to_owned()])
                .affected(targets)
                .stale(stale)
        ]
    } else {
        blocks![heading(count), Block::Properties(targets.collect())]
    }
}
