//! The open surfaces and the Read Caching page, described. Nothing here
//! changes the model.
//!
//! Three shapes:
//!
//! - the dialog `storage-repair` ("Repair Storage Repository"): the
//!   detached attachments to plug, then the review;
//! - the Read Caching page of a repository's Properties dialog (the
//!   kernel's): what the driver says about provisioning and read caching,
//!   then the review of the policy change;
//! - the dialog `storage-graph` ("Storage workflow"): on a repository the
//!   source and the hosts, then the review; on a server the review; under
//!   both the results of earlier runs.
//!
//! Keys and labels are the Rune ones: `repair-pbd-<attachment>`,
//! `storage-repair-review`, `storage-cache-review`,
//! `storage-maintenance-back`, `storage-maintenance-confirm`,
//! `storage-close`, `storage-graph-open`, `storage-source-<connection>` (a
//! chooser button in Rune, an option of the choice `storage-source` here),
//! `storage-host-<host>`, `storage-graph-close`, `storage-graph-back`,
//! `storage-graph-review`, `storage-graph-submit`,
//! `storage-restore-<connection>`.
//!
//! New keys, for elements Rune left without one: `storage-source` (the
//! choice), `storage-maintenance-review` and `storage-workflow-review` (the
//! review panels, so `…-stale` are their stale lines),
//! `storage-maintenance-unavailable`, `storage-maintenance-error`,
//! `storage-maintenance-preview`, `storage-capabilities-refresh`,
//! `storage-cache-policy`, `storage-graph-target`,
//! `storage-graph-unavailable`, `storage-graph-stale`,
//! `storage-graph-changed`, `storage-graph-error`, `storage-graph-preview`,
//! `storage-graph-result-<operation>`, `storage-graph-activity`.

use xenadmin_core::{
    HostId, OperationState, PbdId, ResourceAction, ResourceOperationRecord, ResourceTarget,
    StorageMaintenance,
};

use super::graph;
use super::maintenance::{self, Editor, Kind};
use crate::prelude::*;

/// The ids of the two dialogs.
pub const REPAIR: &str = "storage-repair";
pub const WORKFLOW: &str = "storage-graph";

/// The keys of the two review panels.
pub const MAINTENANCE_REVIEW: &str = "storage-maintenance-review";
pub const WORKFLOW_REVIEW: &str = "storage-workflow-review";

const PREVIEW: &str = "Synthetic preview · no changes can be submitted";

/// What `storage::review_restore` refuses with; here the reason a Restore
/// button is disabled.
// The Rune text said "reopen its storage view" (decision 10).
const RESTORE_CHANGED: &str = "The detached attachment changed; close and reopen this dialog";

/// Back to front: the dialogs of the area. Only one is ever open. The Read
/// Caching page is no surface of this area: the kernel describes it from
/// [`page`].
pub fn surfaces(m: &Model) -> Vec<Surface> {
    let state = &m.areas.storage_maintenance;
    let repair = state
        .editor
        .as_ref()
        .filter(|e| e.kind == Kind::Repair)
        .map(|e| repair(m, e));
    let workflow = state.graph.as_ref().map(|e| workflow(m, e));
    repair.into_iter().chain(workflow).collect()
}

// -- Repair and read caching --------------------------------------------------------

fn repair(m: &Model, e: &Editor) -> Surface {
    let surface = Surface::dialog(REPAIR, "Repair Storage Repository")
        .subtitle(format!("{} ({})", e.name, e.sr.uuid()))
        .experimental(true)
        .body(match &e.review {
            Some(op) => review_body(m, e, op),
            None => repair_draft(m, e),
        })
        .footer(footer(m, e))
        .on_close(super::close);
    match maintenance::permission(m, e) {
        Some(permission) => surface.footer_note(permissions::footer_note(&permission)),
        None => surface,
    }
}

/// The page this area has for the Properties dialog of a repository the
/// inventory of its pool has. Storage administration is outside the
/// supported part.
pub fn pages(m: &Model, target: &Target) -> Vec<PropertyPage> {
    let known = target
        .single_of(ObjectKind::StorageRepository)
        .is_some_and(|object| {
            object
                .pool
                .and_then(|pool| m.inventory_for(pool))
                .zip(object.uuid())
                .is_some_and(|(inventory, uuid)| inventory.storage.iter().any(|r| r.uuid == uuid))
        });
    if !known {
        return Vec::new();
    }
    vec![PropertyPage::planned(PropertiesKind::Storage, maintenance::PAGE).experimental(true)]
}

/// The Read Caching page: the editor `maintenance::page_event` created
/// when the page was entered.
pub fn page(m: &Model, _target: &Target, id: &str) -> PageView {
    let editor = m.areas.storage_maintenance.editor.as_ref();
    let Some(e) = editor.filter(|e| id == maintenance::PAGE && e.kind == Kind::Cache) else {
        return PageView::default();
    };
    PageView {
        body: match &e.review {
            Some(op) => review_body(m, e, op),
            None => cache_draft(m, e),
        },
        footer: footer(m, e),
        footer_note: maintenance::permission(m, e)
            .as_ref()
            .map(permissions::footer_note),
    }
}

fn status(m: &Model, e: &Editor) -> Vec<Block> {
    blocks![
        maintenance::synthetic(m, e)
            .then(|| notice(NoticeKind::Preview, PREVIEW).key("storage-maintenance-preview")),
        e.error
            .clone()
            .map(|error| alert(error).key("storage-maintenance-error")),
    ]
}

fn review_body(m: &Model, e: &Editor, op: &StorageMaintenance) -> Vec<Block> {
    let availability = maintenance::availability(m, e, op);
    blocks![
        heading(op.label()),
        review(MAINTENANCE_REVIEW, maintenance::lines(m, e, op)).stale(maintenance::stale(m, e)),
        (!availability.is_available())
            .then(|| alert(availability.explanation()).key("storage-maintenance-unavailable")),
        status(m, e),
    ]
}

fn repair_draft(m: &Model, e: &Editor) -> Vec<Block> {
    let Some(i) = m.inventory_for(e.pool) else {
        return status(m, e);
    };
    let candidates = i
        .details
        .pbds
        .values()
        .filter(|p| p.sr == Some(e.sr) && p.currently_attached == Some(false))
        .collect::<Vec<_>>();
    let rows = candidates
        .iter()
        .take(128)
        .map(|p| {
            let id = p.id;
            let name = p
                .host
                .and_then(|h| i.hosts.get(&h))
                .map(|h| h.name.clone())
                .unwrap_or_else(|| "Unknown host".into());
            group(
                name,
                blocks![
                    caption(format!("Attachment {id}")),
                    checkbox(
                        format!("repair-pbd-{id}"),
                        "Include this host attachment",
                        e.selection.contains(&id),
                        move |m| maintenance::toggle(m, id),
                    ),
                ],
            )
        })
        .collect::<Vec<_>>();
    blocks![
        // The other repair, which can also create what is missing: the
        // storage workflow on this repository.
        button(
            "storage-graph-open",
            "Repair missing or detached host connections…",
            |m| {
                let Some(e) = &m.areas.storage_maintenance.editor else {
                    return;
                };
                let (pool, sr) = (e.pool, e.sr);
                graph::open(m, pool, ResourceTarget::Storage(sr));
            }
        ),
        heading("Repair across hosts"),
        caption(
            "Select existing detached attachments. The coordinator runs first when selected. \
             Use Repair missing or detached host connections above to reconstruct supported \
             missing shared-storage connections."
        ),
        rows,
        candidates
            .is_empty()
            .then(|| caption("No existing detached attachments were reported.")),
        (candidates.len() > 128).then(|| caption(
            "Only the first 128 attachments are shown. Repair smaller reviewed groups."
        )),
        status(m, e),
    ]
}

/// What the Read Caching page can offer: `None` when no change of the
/// policy is offered; else whether the policy is known and whether caching
/// is disabled now.
fn cache_offer(m: &Model, e: &Editor) -> Option<(bool, bool)> {
    let i = m.inventory_for(e.pool)?;
    let kind = i
        .storage
        .iter()
        .find(|row| row.uuid == e.sr.uuid())
        .map(|row| row.kind.as_str())?;
    let backend = e.backends.iter().find(|b| b.kind == kind)?;
    let supported = matches!(kind, "nfs" | "ext" | "smb")
        && backend.features.iter().any(|f| f == "VDI_READ_CACHING");
    let d = i.details.storage.get(&e.sr)?;
    supported.then(|| {
        (
            d.read_cache_policy_known,
            d.read_cache_override.as_deref() == Some("true"),
        )
    })
}

fn cache_draft(m: &Model, e: &Editor) -> Vec<Block> {
    let Some(i) = m.inventory_for(e.pool) else {
        return status(m, e);
    };
    let kind = i
        .storage
        .iter()
        .find(|row| row.uuid == e.sr.uuid())
        .map(|row| row.kind.as_str())
        .unwrap_or("");
    let body = match e.backends.iter().find(|b| b.kind == kind) {
        Some(b) => {
            let thin = b.features.iter().any(|f| f == "THIN_PROVISIONING");
            let supported = matches!(kind, "nfs" | "ext" | "smb")
                && b.features.iter().any(|f| f == "VDI_READ_CACHING");
            blocks![
                caption(format!(
                    "Driver: {} · Thin provisioning: {}",
                    b.name,
                    if thin { "advertised" } else { "not advertised" }
                )),
                caption(
                    "Provisioning is determined by the repository driver. This view does not \
                     convert existing storage between thin and thick allocation."
                ),
                match cache_offer(m, e) {
                    Some((known, disabled)) => blocks![
                        text(format!(
                            "Read-cache policy: {}",
                            if !known {
                                "Unknown"
                            } else if disabled {
                                "Disabled"
                            } else {
                                "Permitted"
                            }
                        ))
                        .key("storage-cache-policy"),
                        caption(
                            "The driver applies this policy when opening disk backing storage. \
                             Licensing, disk type and host state can still prevent caching; \
                             active disks are not reopened."
                        ),
                    ],
                    None if supported => Vec::new(),
                    None => blocks![caption(
                        "This driver does not advertise supported read caching. No cache \
                         setting is offered."
                    )],
                },
            ]
        }
        None => blocks![
            caption("Read driver capabilities to inspect provisioning and read-cache support."),
            button(
                "storage-capabilities-refresh",
                "Refresh storage capabilities",
                maintenance::query
            ),
        ],
    };
    blocks![
        heading("Provisioning and read caching"),
        body,
        status(m, e)
    ]
}

/// Close, and what the state of the editor adds: Back and Confirm on a
/// review, else the button that asks for the review.
fn footer(m: &Model, e: &Editor) -> Vec<Button> {
    let close = button("storage-close", "Close", super::close);
    if let Some(op) = &e.review {
        return vec![
            close,
            button("storage-maintenance-back", "Back", maintenance::back),
            button(
                "storage-maintenance-confirm",
                "Confirm change",
                maintenance::apply,
            )
            .primary()
            .disabled_opt(maintenance::blocked(m, e, op)),
        ];
    }
    match e.kind {
        Kind::Repair => vec![
            close,
            button(
                "storage-repair-review",
                "Review selected repairs…",
                maintenance::stage,
            )
            .primary()
            .disabled_if(e.selection.is_empty(), maintenance::NOTHING_TO_DO),
        ],
        Kind::Cache => match cache_offer(m, e) {
            Some((known, disabled)) => vec![
                close,
                button(
                    "storage-cache-review",
                    if disabled {
                        "Review enabling read caching…"
                    } else {
                        "Review disabling read caching…"
                    },
                    maintenance::stage,
                )
                .primary()
                .disabled_if(!known, maintenance::POLICY_UNKNOWN),
            ],
            None => vec![close],
        },
    }
}

// -- The storage workflow -------------------------------------------------------------

fn workflow(m: &Model, e: &graph::Editor) -> Surface {
    let proposed = graph::proposal(e);
    let current = graph::current(m, e);
    // The permission line is the review's or, before it, the proposal's.
    let assessment = e
        .review
        .as_ref()
        .or(proposed.as_ref().ok())
        .map(|op| graph::permission(m, e, op));
    let main = match (&e.review, e.target) {
        (Some(op), _) => blocks![
            review(WORKFLOW_REVIEW, graph::lines(&e.initial, op))
                .stale((!current).then_some(graph::STALE))
        ],
        (None, ResourceTarget::Storage(_)) => repair_form(e),
        // A server's proposal that is not reviewed (it could not be staged,
        // or it was submitted): what the review would say.
        (None, _) => match &proposed {
            Ok(op) => graph::lines(&e.initial, op)
                .into_iter()
                .map(|line| caption(line).into())
                .collect(),
            Err(_) => Vec::new(),
        },
    };
    let body = blocks![
        heading(if e.review.is_some() {
            "Review storage workflow"
        } else {
            "Storage connections and multipathing"
        }),
        caption(e.initial.pool_name.clone()),
        caption(format!("Pool {} · target {}", e.pool, e.target.uuid())).key("storage-graph-target"),
        main,
        proposed
            .as_ref()
            .err()
            .map(|reason| alert(reason.clone()).key("storage-graph-unavailable")),
        (e.review.is_none() && !current).then(|| alert(graph::STALE).key("storage-graph-stale")),
        e.review
            .as_ref()
            .filter(|op| !graph::available(m, e, op))
            .map(|_| alert(graph::CHANGED).key("storage-graph-changed")),
        e.error
            .clone()
            .map(|line| caption(line).key("storage-graph-error")),
        graph::synthetic(m, e)
            .then(|| notice(NoticeKind::Preview, PREVIEW).key("storage-graph-preview")),
        history(m, e, current),
    ];
    let close = button("storage-graph-close", "Close", super::close);
    let footer = match &e.review {
        Some(op) => {
            let confirm = button(
                "storage-graph-submit",
                "Confirm storage changes",
                graph::submit,
            )
            .danger()
            .disabled_opt(graph::blocked(m, e, op));
            if graph::has_draft(e) {
                vec![
                    close,
                    button("storage-graph-back", "Back", graph::back),
                    confirm,
                ]
            } else {
                vec![close, confirm]
            }
        }
        None => vec![
            close,
            button("storage-graph-review", "Review changes…", graph::stage)
                .primary()
                .disabled_opt(proposed.as_ref().err().cloned())
                .disabled_if(!current, graph::STALE),
        ],
    };
    let surface = Surface::dialog(WORKFLOW, "Storage workflow")
        .subtitle(format!("{} ({})", target_name(e), e.target.uuid()))
        .experimental(true)
        .large()
        .body(body)
        .footer(footer)
        .on_close(super::close);
    match assessment {
        Some(assessment) => surface.footer_note(permissions::footer_note(&assessment)),
        None => surface,
    }
}

fn target_name(e: &graph::Editor) -> String {
    match e.target {
        ResourceTarget::Host(host) => graph::host_name(&e.initial, host),
        target => e
            .initial
            .storage
            .iter()
            .find(|row| row.uuid == target.uuid())
            .map(|row| row.name.clone())
            .unwrap_or_else(|| "Storage repository".into()),
    }
}

/// The draft of the repair of shared storage: the source connection and
/// the hosts to connect.
fn repair_form(e: &graph::Editor) -> Vec<Block> {
    let ResourceTarget::Storage(sr) = e.target else {
        return Vec::new();
    };
    let i = &e.initial;
    let sources = i
        .details
        .pbds
        .values()
        .filter(|p| p.sr == Some(sr))
        .take(128)
        .map(|p| {
            let id = p.id;
            option(
                id.to_string(),
                format!(
                    "{} · {}",
                    p.host
                        .map(|h| graph::host_name(i, h))
                        .unwrap_or_else(|| "Unknown host".into()),
                    id
                ),
            )
            .key(format!("storage-source-{id}"))
        })
        .collect::<Vec<_>>();
    let selected = e.source.map(|id| id.to_string());
    let hosts = i
        .hosts
        .values()
        .take(128)
        .map(|host| {
            let id: HostId = host.id;
            let pairs = i
                .details
                .pbds
                .values()
                .filter(|p| p.sr == Some(sr) && p.host == Some(id))
                .collect::<Vec<_>>();
            let eligible =
                pairs.is_empty() || pairs.len() == 1 && pairs[0].currently_attached == Some(false);
            let status = if pairs.is_empty() {
                "missing"
            } else if eligible {
                "detached"
            } else {
                "already attached or incomplete"
            };
            checkbox(
                format!("storage-host-{id}"),
                format!("{} · {status}", host.name),
                e.hosts.contains(&id),
                move |m| graph::toggle_host(m, id),
            )
            .disabled_if(
                !eligible,
                "Only a missing or detached host connection can be repaired.",
            )
        })
        .collect::<Vec<_>>();
    blocks![
        heading("Choose a configuration source"),
        caption(
            "Shared supported repositories only. Choose one existing connection as the source, \
             then up to 32 missing or detached host connections. Physical storage ownership and \
             custom per-host options are not inferred."
        ),
        (!sources.is_empty()).then(|| choice(
            "storage-source",
            sources,
            selected.as_deref(),
            |m, id| {
                if let Ok(uuid) = uuid::Uuid::parse_str(id) {
                    graph::set_source(m, PbdId::from(uuid));
                }
            }
        )),
        heading("Selected host connections"),
        hosts,
    ]
}

fn outcome_state(r: &ResourceOperationRecord) -> OutcomeState {
    match r.state {
        OperationState::Succeeded => OutcomeState::Succeeded,
        OperationState::PartiallyCompleted => OutcomeState::Partial,
        OperationState::Failed | OperationState::Rejected => OutcomeState::Failed,
        OperationState::Cancelled => OutcomeState::Cancelled,
        OperationState::OutcomeUnknown | OperationState::AcknowledgedUnknown => {
            OutcomeState::Unknown
        }
        _ => OutcomeState::Running(r.progress.map(|done| done as f32)),
    }
}

/// The results of earlier runs on this target, newest first as the model
/// has them, and the way to the full history. Restore is offered only for a
/// partial result whose connection is verified detached, never for an
/// unknown outcome; it opens a new review of the storage area that can only
/// attach.
fn history(m: &Model, e: &graph::Editor, current: bool) -> Vec<Block> {
    let pool = e.pool;
    let inventory = m.inventory_for(pool);
    let mut out = blocks![heading("Recent storage workflow results")];
    let records = m
        .resource_rows
        .iter()
        .filter(|r| {
            r.pool == e.pool
                && r.target == e.target
                && matches!(r.action, ResourceAction::StorageGraph(_))
        })
        .take(20);
    for r in records {
        out.extend(blocks![
            outcome(
                format!("storage-graph-result-{}", r.id),
                outcome_state(r),
                format!("{} · {:?}", r.action.label(), r.state),
            )
            .line(r.detail.clone())
            .lines(
                r.steps
                    .iter()
                    .map(|step| format!("{} · {:?}", step.label, step.state))
            )
        ]);
        for result in &r.storage_graph {
            out.extend(blocks![caption(format!(
                "Host {} · connection {} · {}{}",
                result.host,
                result.pbd,
                if !result.verified {
                    "identity retained; configuration unverified"
                } else if result.attached {
                    "verified attached"
                } else {
                    "verified detached"
                },
                if result.created {
                    " · newly created"
                } else {
                    ""
                }
            ))]);
            if r.state == OperationState::PartiallyCompleted && result.verified && !result.attached
            {
                let (id, sr, host) = (result.pbd, result.sr, result.host);
                let available = inventory
                    .as_ref()
                    .and_then(|i| i.details.pbds.get(&id))
                    .is_some_and(|p| {
                        p.host == Some(host)
                            && p.sr == Some(sr)
                            && p.currently_attached == Some(false)
                    });
                out.push(actions([button(
                    format!("storage-restore-{id}"),
                    "Restore this connection…",
                    move |m| {
                        if m.inventory_for(pool).is_some() {
                            m.areas.storage_maintenance.graph = None;
                            crate::areas::storage::review_restore(m, pool, sr, id, host);
                        }
                    },
                )
                .disabled_if(!available, RESTORE_CHANGED)
                .disabled_if(!current, graph::STALE)]));
            }
        }
    }
    // Rune's Activity dialog is the Events tab of the Notifications view
    // (Rune label: "Open Activity history"; decision 10).
    out.extend(blocks![button(
        "storage-graph-activity",
        "Operation history",
        |m| {
            super::close(m);
            m.show_notifications(TabId::Events);
        }
    )]);
    out
}
