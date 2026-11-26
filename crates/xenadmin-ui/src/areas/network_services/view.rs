//! The open surface of the area, described. Nothing here changes the model.
//!
//! | Surface | Kind | Draft |
//! | --- | --- | --- |
//! | `network-topology` | dialog: form, then its review | the Rune `topology::form` |
//! | `network-fabric` | wizard: Type, Name, Settings, Review | the Rune `fabric::form` (Settings) |
//! | `network-fabric-pool` | wizard: Type, Name, Settings, Review | the Rune `fabric_pool::form` (Settings) |
//! | `network-fabric-remove` | dialog: the records, then the review | the Rune "Review removal…" buttons |
//!
//! The Type step is the kind the three Rune buttons of each module chose
//! ("Add GRE tunnel interface…", ..). The Name step shows the network the
//! interfaces are added to (these workflows never created the network
//! itself) with its coverage, the Rune `fabric::view` without its buttons.
//!
//! Rune keys kept: `network-review` ("Review network change") and
//! `network-confirm` ("Confirm network change"). The Rune keys
//! `topology-create` and `topology-retag` were buttons of the hub: they
//! are the commands `network.vlan.create-pool` and `network.vlan.retag-pool`.
//!
//! New keys, for elements Rune left without one (those of the shared
//! elements are the keys the `network` area gave the same elements of its
//! dialog): `network-close`, `network-back`, `network-next`,
//! `network-services-review` (and its `-stale` line), `network-unavailable`,
//! `network-preview`, `network-error`, `network-pool-changed`,
//! `network-topology-scope`, `network-topology-tag`,
//! `network-topology-uplink`, `network-topology-reason`,
//! `network-fabric-type`, `network-fabric-network`, `network-fabric-input`,
//! `network-fabric-pool-input-<host>`, `network-fabric-record`.

use xenadmin_core::{HostId, Inventory, NetworkId, PermissionAssessment, PifId};

use super::edit::{self, Draft, Editor, NAME, TYPE};
use super::fabric::{self, HostCoverage, Kind};
use super::topology::{self, Selection};
use crate::areas::network::{
    self,
    commands::NOT_WRITABLE,
    name, pif_name,
    view::{DEMO, PREVIEW},
};
use crate::prelude::*;

pub const TOPOLOGY: &str = "network-topology";
pub const FABRIC: &str = "network-fabric";
pub const FABRIC_POOL: &str = "network-fabric-pool";
pub const FABRIC_REMOVE: &str = "network-fabric-remove";
/// The steps of the plan's New Network wizard.
pub const STEPS: [&str; 4] = ["Type", "Name", "Settings", "Review"];
/// The review block; `<key>-stale` is its stale line.
pub const REVIEW: &str = "network-services-review";
/// Rune offered no removal for a record that names no input interface or
/// no protocol, without a text.
pub const NO_INPUT: &str =
    "This record does not report its input interface or protocol: its removal cannot be reviewed.";

pub fn surfaces(m: &Model) -> Vec<Surface> {
    m.areas
        .network_services
        .editor
        .as_ref()
        .map(|e| surface(m, e))
        .into_iter()
        .collect()
}

fn surface(m: &Model, e: &Editor) -> Surface {
    let permission = edit::permission(m, e);
    let inventory = m.inventory_for(e.pool);
    let current = if e.review.is_some() { 3 } else { e.step.min(2) };
    let reviewed = e.review.as_ref().map(|review| review.operation.label());
    let surface = match &e.draft {
        Draft::Topology(t) => Surface::dialog(
            TOPOLOGY,
            reviewed.unwrap_or(match t.selection {
                Selection::Create(_) => "Pool-wide VLAN creation",
                Selection::Retag(_) => "Pool-wide VLAN retagging",
                Selection::Delete(_) => "Pool-wide VLAN removal",
            }),
        ),
        Draft::Removal(_) => Surface::dialog(
            FABRIC_REMOVE,
            reviewed.unwrap_or("Remove tunnel or SR-IOV interface"),
        ),
        Draft::Fabric(_) => Surface::wizard(FABRIC, "Add Tunnel or SR-IOV Interface", STEPS, current),
        Draft::FabricPool(_) => {
            Surface::wizard(FABRIC_POOL, "Complete Pool Coverage", STEPS, current)
        }
    };
    // The Rune captions under the heading: the network of every form, and
    // target and pool of a review.
    let network = match &inventory {
        Some(i) => format!("Network: {} ({})", name(i, e.network), e.network),
        None => format!("Network: {}", e.network),
    };
    let subtitle = match &e.review {
        Some(review) => format!(
            "{network}\nTarget {} · pool {}",
            review.target.uuid(),
            e.pool
        ),
        None => network,
    };
    let surface = surface
        .subtitle(subtitle)
        .experimental(true)
        .body(body(m, e, inventory.as_deref()))
        .footer(footer(m, e, permission.as_ref()))
        .on_close(super::close);
    match &permission {
        Some(permission) => surface.footer_note(permissions::footer_note(permission)),
        None => surface,
    }
}

fn body(m: &Model, e: &Editor, inventory: Option<&Inventory>) -> Vec<Block> {
    blocks![
        match (&e.review, inventory) {
            (Some(review), _) => blocks![
                e.draft.wizard().then(|| heading(review.operation.label())),
                crate::surface::review(REVIEW, review.lines.clone()).stale(edit::stale(m, e)),
            ],
            (None, Some(i)) => form(e, i),
            (None, None) => blocks![
                caption("The connection changed. Close and reopen this dialog.")
                    .key("network-pool-changed")
            ],
        },
        edit::unavailable(m, e).map(|reason| alert(reason).key("network-unavailable")),
        edit::synthetic(m, e).then(|| notice(NoticeKind::Preview, PREVIEW).key("network-preview")),
        e.error.clone().map(|error| alert(error).key("network-error")),
    ]
}

/// A handler that edits the open draft with the new text (an edited text,
/// or the id of a picked option).
fn drafted(change: impl Fn(&mut Draft, &str) + 'static) -> impl Fn(&mut Model, &str) + 'static {
    move |m, text| {
        if let Some(e) = edit::edit(m) {
            change(&mut e.draft, text);
        }
    }
}

fn pif_of(id: &str) -> Option<PifId> {
    uuid::Uuid::parse_str(id).ok().map(PifId::from)
}

fn form(e: &Editor, i: &Inventory) -> Vec<Block> {
    match &e.draft {
        Draft::Topology(t) => topology_form(t, i),
        Draft::Removal(removal) => removal_form(e.network, removal, i),
        Draft::Fabric(f) => match e.step {
            TYPE => type_step(f.kind, fabric::add_label),
            NAME => name_step(e.network, i),
            _ => fabric_inputs(e.network, f, i),
        },
        Draft::FabricPool(p) => match e.step {
            TYPE => type_step(p.kind, fabric::complete_label),
            NAME => name_step(e.network, i),
            _ => pool_inputs(e.network, p, i),
        },
    }
}

// -- Pool-wide VLANs -----------------------------------------------------------------

fn topology_form(t: &topology::Editor, i: &Inventory) -> Vec<Block> {
    blocks![
        caption(format!(
            "Network: {} · {} pool hosts",
            name(i, t.network),
            i.hosts.len()
        ))
        .key("network-topology-scope"),
        (!matches!(t.selection, Selection::Delete(_))).then(|| {
            input(
                "network-topology-tag",
                "VLAN tag (0–4094)",
                &t.tag,
                drafted(|draft, text| {
                    if let Draft::Topology(t) = draft {
                        t.tag = text.to_owned();
                    }
                }),
            )
            .on_submit(edit::review)
        }),
        match &t.selection {
            Selection::Create(selected) => {
                let candidates = topology::candidates(i, t.network);
                let sources = candidates
                    .iter()
                    .filter_map(|(source, uplinks)| Some((*source, uplinks.as_ref().ok()?)))
                    .collect::<Vec<_>>();
                let chosen = sources
                    .iter()
                    .find(|(_, choices)| *choices == selected)
                    .map(|(source, _)| source.to_string());
                blocks![
                    caption("Choose an uplink network. It must have exactly one managed untagged interface on every pool host."),
                    choice(
                        "network-topology-uplink",
                        sources
                            .iter()
                            .map(|(source, _)| option(source.to_string(), name(i, *source))),
                        chosen.as_deref(),
                        pick_uplink,
                    )
                    .label("Uplink network"),
                    candidates
                        .iter()
                        .filter_map(|(source, uplinks)| Some((*source, uplinks.as_ref().err()?)))
                        .map(|(source, reason)| {
                            caption(format!("{} cannot be chosen: {reason}", name(i, source)))
                        })
                        .collect::<Vec<_>>(),
                    selected
                        .iter()
                        .map(|p| caption(format!("{} · {}", pif_name(i, p.pif), p.pif)))
                        .collect::<Vec<_>>(),
                ]
            }
            Selection::Retag(interfaces) | Selection::Delete(interfaces) => blocks![
                caption(if matches!(t.selection, Selection::Delete(_)) {
                    "The captured interfaces will be removed. Move VM attachments and IP configuration off this network first."
                } else {
                    "The captured interfaces will be replaced. Move VM attachments and IP configuration off this network first."
                }),
                interfaces
                    .iter()
                    .map(|p| caption(format!(
                        "{} · VLAN {} · tag {} · {}",
                        pif_name(i, p.uplink.pif),
                        p.vlan,
                        p.tag,
                        if p.attached { "plugged" } else { "unplugged" }
                    )))
                    .collect::<Vec<_>>(),
            ],
        },
        // The live validation of the Rune form: the gate of the review.
        topology::review(t, i)
            .err()
            .map(|reason| caption(reason).key("network-topology-reason")),
    ]
}

/// The Rune button of an uplink network: its uplinks become the selection,
/// read from the inventory the options were listed from.
fn pick_uplink(m: &mut Model, id: &str) {
    let Some(e) = m.areas.network_services.editor.as_ref() else {
        return;
    };
    let Some(i) = m.inventory_for(e.pool) else {
        return;
    };
    let choices = topology::sources(&i, e.network)
        .into_iter()
        .find(|(source, _)| source.to_string() == id)
        .map(|(_, choices)| choices);
    if let (Some(choices), Some(e)) = (choices, edit::edit(m))
        && let Draft::Topology(t) = &mut e.draft
    {
        t.selection = Selection::Create(choices);
    }
}

// -- The steps of the two wizards ----------------------------------------------------

fn type_step(kind: Option<Kind>, label: fn(Kind) -> &'static str) -> Vec<Block> {
    blocks![
        choice(
            "network-fabric-type",
            fabric::KINDS.map(|kind| option(fabric::kind_id(kind), label(kind))),
            kind.map(fabric::kind_id),
            drafted(|draft, id| {
                let Some(kind) = fabric::kind_of(id) else {
                    return;
                };
                // Another kind has other inputs: what was chosen goes.
                match draft {
                    Draft::Fabric(f) if f.kind != Some(kind) => {
                        f.kind = Some(kind);
                        f.input = None;
                    }
                    Draft::FabricPool(p) if p.kind != Some(kind) => {
                        p.kind = Some(kind);
                        p.inputs.clear();
                    }
                    _ => {}
                }
            }),
        )
        .label("Type"),
    ]
}

fn name_step(network: NetworkId, i: &Inventory) -> Vec<Block> {
    blocks![
        properties([
            property("Name", name(i, network)),
            property("UUID", network.to_string()).copyable(),
        ]),
        caption("The interfaces are added to this network. Its name and description are on the General page of its Properties.")
            .key("network-fabric-network"),
        fabric::coverage(i, network),
    ]
}

fn candidate_options(candidates: &[fabric::Candidate]) -> Vec<OptionItem> {
    candidates
        .iter()
        .map(|candidate| {
            let item = option(candidate.pif.to_string(), candidate.label.clone());
            match &candidate.available {
                // Rune: the button disabled, the error as a caption under it.
                Err(reason) => item.detail(reason.clone()).disabled_if(true, reason.clone()),
                Ok(()) => item,
            }
        })
        .collect()
}

fn fabric_inputs(network: NetworkId, f: &fabric::Editor, i: &Inventory) -> Vec<Block> {
    let Some(kind) = f.kind else {
        return blocks![caption(fabric::CHOOSE_TYPE)];
    };
    let chosen = f.input.map(|id| id.to_string());
    blocks![
        choice(
            "network-fabric-input",
            candidate_options(&fabric::candidates(i, network, kind)),
            chosen.as_deref(),
            drafted(|draft, id| {
                if let Draft::Fabric(f) = draft {
                    f.input = pif_of(id);
                }
            }),
        )
        .label(match kind {
            Kind::Tunnel(_) => "Choose tunnel transport",
            Kind::Sriov => "Choose physical SR-IOV interface",
        }),
    ]
}

fn pool_inputs(network: NetworkId, p: &fabric::PoolEditor, i: &Inventory) -> Vec<Block> {
    let Some(kind) = p.kind else {
        return blocks![caption(fabric::CHOOSE_TYPE)];
    };
    blocks![
        heading("Complete pool network coverage"),
        caption("Choose one input on every host that is missing coverage. Existing interfaces are captured and preserved. Every host must be online; SR-IOV changes additionally require disabled, evacuated hosts and disabled HA."),
        fabric::pool_hosts(i, network, kind)
            .into_iter()
            .map(|(host, name, coverage)| group(
                name,
                match coverage {
                    HostCoverage::Existing(members) => members
                        .into_iter()
                        .map(|id| caption(format!("Existing interface {id} · preserved")).into())
                        .collect(),
                    HostCoverage::Missing(candidates) => {
                        let chosen = p.inputs.get(&host).map(|id| id.to_string());
                        blocks![
                            choice(
                                format!("network-fabric-pool-input-{host}"),
                                candidate_options(&candidates),
                                chosen.as_deref(),
                                pick_pool_input(host),
                            )
                            .label("Missing coverage — select an input"),
                        ]
                    }
                }
            ))
            .collect::<Vec<_>>(),
    ]
}

fn pick_pool_input(host: HostId) -> impl Fn(&mut Model, &str) + 'static {
    drafted(move |draft, id| {
        if let (Draft::FabricPool(p), Some(pif)) = (draft, pif_of(id)) {
            p.inputs.insert(host, pif);
        }
    })
}

// -- The removal of one record -------------------------------------------------------

fn removal_form(network: NetworkId, removal: &fabric::Removal, i: &Inventory) -> Vec<Block> {
    let chosen = removal.chosen.map(|object| object.uuid().to_string());
    blocks![
        choice(
            "network-fabric-record",
            fabric::records(i, network).into_iter().map(|record| {
                let item = option(record.object.uuid().to_string(), record.line);
                match record.removal {
                    Some(Ok(_)) => item,
                    Some(Err(error)) => {
                        let reason = fabric::removal_unavailable(&error);
                        item.detail(reason.clone()).disabled_if(true, reason)
                    }
                    None => item.disabled_if(true, NO_INPUT),
                }
            }),
            chosen.as_deref(),
            pick_record,
        )
        .label("Tunnel and SR-IOV coverage"),
    ]
}

fn pick_record(m: &mut Model, id: &str) {
    let Some(e) = m.areas.network_services.editor.as_ref() else {
        return;
    };
    let Some(i) = m.inventory_for(e.pool) else {
        return;
    };
    let object = fabric::records(&i, e.network)
        .into_iter()
        .map(|record| record.object)
        .find(|object| object.uuid().to_string() == id);
    if let (Some(object), Some(e)) = (object, edit::edit(m))
        && let Draft::Removal(removal) = &mut e.draft
    {
        removal.chosen = Some(object);
    }
}

// -- The footer ----------------------------------------------------------------------

fn footer(m: &Model, e: &Editor, permission: Option<&PermissionAssessment>) -> Vec<Button> {
    let wizard = e.draft.wizard();
    let close = button(
        "network-close",
        if wizard { "Cancel" } else { "Close" },
        super::close,
    );
    let back = |handler: fn(&mut Model)| button("network-back", "Back", handler);
    if e.review.is_some() {
        // The first reason wins: stale, the rule of the module, a known
        // denial, then the demo.
        let confirm = button("network-confirm", "Confirm network change", edit::submit)
            .primary()
            .disabled_opt(edit::stale(m, e))
            .disabled_opt(edit::unavailable(m, e))
            .disabled_opt(permission.and_then(permissions::denial))
            .disabled_if(edit::synthetic(m, e), DEMO);
        return vec![close, back(edit::back), confirm];
    }
    let review = button(
        "network-review",
        match e.draft {
            Draft::Removal(_) => "Review removal",
            _ => "Review network change",
        },
        edit::review,
    )
    .primary()
    .disabled_if(!network::writable(m, e.pool), NOT_WRITABLE);
    if !wizard {
        return vec![close, review];
    }
    let untyped = match &e.draft {
        Draft::Fabric(f) => f.kind.is_none(),
        Draft::FabricPool(p) => p.kind.is_none(),
        _ => false,
    };
    let next = button("network-next", "Next", |m| edit::step(m, 1)).primary();
    match e.step {
        TYPE => vec![close, next.disabled_if(untyped, fabric::CHOOSE_TYPE)],
        NAME => vec![close, back(|m| edit::step(m, -1)), next],
        _ => vec![close, back(|m| edit::step(m, -1)), review],
    }
}
