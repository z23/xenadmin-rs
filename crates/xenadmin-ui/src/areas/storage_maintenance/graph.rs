//! The logic carried over from the Rune module `storage_graph_view.rs`: the
//! storage workflow. On a repository it repairs the connections of shared
//! storage and can create the ones that are missing; on a server it changes
//! multipathing with every repository disconnected and reconnected around
//! the change. Texts of errors and reviews are the Rune ones, word for word.
//!
//! What changed with the port:
//!
//! - The editor follows the pool of its target, not the scope session.
//! - The workflow on a server has nothing to enter: it opens on its review
//!   when the proposal can be reviewed (Rune showed the proposal as a
//!   preview and asked for a press on "Review changes…"), and that review
//!   has no Back. Where it cannot be reviewed the dialog opens as in Rune,
//!   with the reason and the history.
//! - Rune's submit answered every refusal with `Stale`; a known permission
//!   denial is answered with its own text here.

use std::collections::BTreeSet;
use std::sync::Arc;

use xenadmin_core::{
    ClassAvailability, Error, HostId, Inventory, ObservedResource, OperationId, PbdId,
    PermissionAssessment, PoolId, ResourceCommand, ResourceOperation, ResourceTarget, ReviewedPbd,
    StorageGraph, StorageRepairHost,
};

use crate::prelude::*;

/// Shown when the connection or the pool the workflow was opened on is not
/// the current one any more.
pub const STALE: &str = "Captured connection changed or inventory is stale. Reopen this workflow.";

/// Shown when the reviewed operation is not available against the live
/// inventory any more.
pub const CHANGED: &str = "Reviewed host or attachment data changed; submission is disabled.";

/// The editor's line after a submission was accepted.
pub const SUBMITTED: &str = "Submitted. History below tracks each host; reopen the workflow before submitting another change.";

pub const DEMO: &str = super::maintenance::DEMO;
pub const NOT_WRITABLE: &str = super::maintenance::NOT_WRITABLE;

pub struct Editor {
    pub pool: PoolId,
    /// `Storage(sr)` or `Host(host)`.
    pub target: ResourceTarget,
    pub observed: Option<ObservedResource>,
    /// The inventory at open. Proposals are built from it, not from the
    /// live one; the live one is what availability is checked against.
    pub initial: Arc<Inventory>,
    /// The existing connection whose configuration new ones copy.
    pub source: Option<PbdId>,
    pub hosts: BTreeSet<HostId>,
    pub review: Option<StorageGraph>,
    /// A refusal, or the line after a submission.
    pub error: Option<String>,
}

/// Opens the workflow on a repository or a server of `pool`. Every other
/// surface of this area and the storage area's dialog give way, as in Rune.
pub fn open(m: &mut Model, pool: PoolId, target: ResourceTarget) -> bool {
    let Some(initial) = m.inventory_for(pool) else {
        return false;
    };
    let source = if let ResourceTarget::Storage(sr) = target {
        initial
            .details
            .pbds
            .values()
            .filter(|p| p.sr == Some(sr) && p.location.as_ref().is_some_and(|l| l.shared()))
            .map(|p| p.id)
            .next()
    } else {
        None
    };
    let observed = m
        .snapshot_for(pool)
        .and_then(|snapshot| snapshot.observe_resource(target).ok());
    super::close(m);
    crate::areas::storage::close(m);
    let editor = Editor {
        pool: initial.pool_id,
        target,
        observed,
        initial,
        source,
        hosts: BTreeSet::new(),
        review: None,
        error: None,
    };
    // Nothing to enter on a server: straight to the review when there is
    // one. A refusal is not an error of the user's: the dialog says why.
    let straight = matches!(target, ResourceTarget::Host(_)) && proposal(&editor).is_ok();
    m.areas.storage_maintenance.graph = Some(editor);
    if straight {
        stage(m);
    }
    true
}

/// Whether the review has a draft to go back to.
pub fn has_draft(e: &Editor) -> bool {
    matches!(e.target, ResourceTarget::Storage(_))
}

pub fn synthetic(m: &Model, e: &Editor) -> bool {
    m.is_synthetic_pool(e.pool)
}

/// The Rune `current`: the pool is still shown, and outside a synthetic pool
/// the target still reads as it did at open.
pub fn current(m: &Model, e: &Editor) -> bool {
    m.inventory_for(e.pool).is_some()
        && (synthetic(m, e)
            || e.observed.is_some()
                && e.observed
                    == m.snapshot_for(e.pool)
                        .and_then(|snapshot| snapshot.observe_resource(e.target).ok()))
}

/// What the workflow would do, from the inventory captured at open and the
/// draft, or why it cannot be reviewed.
pub fn proposal(e: &Editor) -> Result<StorageGraph, String> {
    let i = &e.initial;
    if i.details.classes.get("pbd") != Some(&ClassAvailability::Available) {
        return Err("Complete attachment inventory is required".into());
    }
    match e.target {
        ResourceTarget::Storage(sr) => {
            let source = ReviewedPbd::from_inventory(
                i,
                e.source.ok_or("Choose an existing source connection")?,
            )
            .filter(|p| p.sr == sr)
            .ok_or("Source connection details are incomplete")?;
            let hosts = e
                .hosts
                .iter()
                .map(|h| {
                    let pairs = i
                        .details
                        .pbds
                        .values()
                        .filter(|p| p.host == Some(*h) && p.sr == Some(sr))
                        .collect::<Vec<_>>();
                    if pairs.len() > 1 {
                        return Err("A host has duplicate storage connections".to_string());
                    }
                    let attachment = pairs
                        .first()
                        .map(|p| {
                            ReviewedPbd::from_inventory(i, p.id)
                                .ok_or("Connection details are incomplete".to_string())
                        })
                        .transpose()?;
                    Ok(StorageRepairHost {
                        host: *h,
                        attachment,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            let op = StorageGraph::Repair {
                coordinator: i.coordinator.ok_or("Pool coordinator is unknown")?,
                source,
                hosts,
            };
            if !op.availability(i, e.target).is_available() {
                return Err(
                    "Select 1–32 missing or detached host connections on shared supported storage"
                        .into(),
                );
            }
            Ok(op)
        }
        ResourceTarget::Host(host) => {
            let d = i
                .details
                .hosts
                .get(&host)
                .ok_or("Host details are incomplete")?;
            let expected = d.multipathing.ok_or("Host multipathing mode is unknown")?;
            let expected_handler = d
                .standard_multipath_handler
                .ok_or("Custom or unknown multipath handler needs separate configuration")?;
            let attachments = i
                .details
                .pbds
                .values()
                .filter(|p| p.host == Some(host))
                .map(|p| {
                    ReviewedPbd::from_inventory(i, p.id)
                        .ok_or("Complete attachment details are required")
                })
                .collect::<Result<Vec<_>, _>>()?;
            if d.enabled != Some(false)
                || d.maintenance_mode != Some(true)
                || i.details.pool.as_ref().and_then(|p| p.ha_enabled) != Some(false)
            {
                return Err(
                    "First disable HA, evacuate guests and enter maintenance for this host".into(),
                );
            }
            if i.vms
                .values()
                .any(|v| !v.is_control_domain && v.host == Some(host))
            {
                return Err("Evacuate all resident guests first".into());
            }
            let op = StorageGraph::Multipath {
                host,
                expected,
                expected_handler,
                enabled: !expected,
                attachments,
            };
            if !op.availability(i, e.target).is_available() {
                return Err(
                    "The complete host attachment graph is unavailable or exceeds 64 connections"
                        .into(),
                );
            }
            Ok(op)
        }
        _ => Err("Choose a host or shared storage repository".into()),
    }
}

/// Whether an operation is available against the LIVE inventory.
pub fn available(m: &Model, e: &Editor, op: &StorageGraph) -> bool {
    m.inventory_for(e.pool)
        .is_some_and(|i| op.availability(&i, e.target).is_available())
}

/// Puts the proposal up for review, or says why not.
pub fn stage(m: &mut Model) {
    let Some(e) = m.areas.storage_maintenance.graph.as_ref() else {
        return;
    };
    let result = if !current(m, e) {
        Err("Captured connection changed; reopen this workflow".into())
    } else {
        proposal(e).and_then(|op| {
            if available(m, e, &op) {
                Ok(op)
            } else {
                Err("Reviewed attachment or host state changed; reopen this workflow".into())
            }
        })
    };
    let Some(e) = m.areas.storage_maintenance.graph.as_mut() else {
        return;
    };
    match result {
        Ok(op) => {
            e.review = Some(op);
            e.error = None;
        }
        Err(error) => e.error = Some(error),
    }
}

pub fn back(m: &mut Model) {
    if let Some(e) = &mut m.areas.storage_maintenance.graph {
        e.review = None;
    }
}

/// Chooses the source connection. The hosts chosen for another source go.
pub fn set_source(m: &mut Model, id: PbdId) {
    if let Some(e) = &mut m.areas.storage_maintenance.graph {
        e.source = Some(id);
        e.hosts.clear();
    }
}

pub fn toggle_host(m: &mut Model, id: HostId) {
    if let Some(e) = &mut m.areas.storage_maintenance.graph {
        if !e.hosts.remove(&id) {
            e.hosts.insert(id);
        }
        e.error = None;
    }
}

pub fn permission(m: &Model, e: &Editor, op: &StorageGraph) -> PermissionAssessment {
    permissions::assess_for(m, e.pool, &op.permission_requirements())
}

/// Why a reviewed operation cannot be submitted now (the Rune `can_submit`,
/// with the reason): the first of the stale capture, the changed inventory,
/// a known denial, the synthetic pool, a connection that cannot write, 64
/// resource operations held.
pub fn blocked(m: &Model, e: &Editor, op: &StorageGraph) -> Option<String> {
    if !current(m, e) {
        return Some(STALE.into());
    }
    if !available(m, e, op) {
        return Some(CHANGED.into());
    }
    if let Some(denial) = permissions::denial(&permission(m, e, op)) {
        return Some(denial);
    }
    if synthetic(m, e) {
        return Some(DEMO.into());
    }
    if !m.can_mutate_pool(e.pool) {
        return Some(NOT_WRITABLE.into());
    }
    (m.resource_operations.len() >= 64).then(|| Error::Busy.to_string())
}

/// Submits the review. Accepted, the dialog stays open on the history: the
/// review is gone and the editor says what to do next (a second submission
/// needs a reopen, because the observation is stale now).
pub fn submit(m: &mut Model) {
    match submission(m) {
        Ok(operation) => {
            m.track_resource(operation);
            if let Some(e) = &mut m.areas.storage_maintenance.graph {
                e.review = None;
                e.error = Some(SUBMITTED.into());
            }
        }
        Err(error) => {
            if let Some(e) = &mut m.areas.storage_maintenance.graph {
                e.error = Some(error);
            }
        }
    }
}

fn submission(m: &Model) -> Result<ResourceOperation, String> {
    let stale = || Error::Stale.to_string();
    let e = m
        .areas
        .storage_maintenance
        .graph
        .as_ref()
        .ok_or_else(|| Error::Closed.to_string())?;
    let op = e.review.as_ref().ok_or_else(stale)?;
    if let Some(denial) = permissions::denial(&permission(m, e, op)) {
        return Err(denial);
    }
    if blocked(m, e, op).is_some() {
        return Err(stale());
    }
    m.connection_for(e.pool)
        .ok_or_else(|| Error::Closed.to_string())?
        .submit_resource(
            OperationId::new(),
            ResourceCommand::StorageGraph {
                target: e.observed.ok_or_else(stale)?,
                operation: op.clone(),
            },
        )
        .map_err(|error| error.to_string())
}

// -- Review texts --------------------------------------------------------------------

pub fn host_name(i: &Inventory, id: HostId) -> String {
    i.hosts
        .get(&id)
        .map(|h| h.name.clone())
        .unwrap_or_else(|| "Host".into())
}

fn attachment_lines(i: &Inventory, p: &ReviewedPbd) -> Vec<String> {
    let mut lines = vec![
        format!(
            "{} · {}",
            host_name(i, p.host),
            if p.attached { "attached" } else { "detached" }
        ),
        format!("Host {}", p.host),
        format!("Repository {}", p.sr),
        format!("Connection {}", p.pbd),
    ];
    if let Some(location) = &p.location {
        lines.extend(crate::areas::storage::location_lines(location));
    }
    lines
}

/// The closing line of every review of the workflow.
// "Restore connection controls below" are the history of the same dialog.
pub const CLOSING: &str = "Each step is journaled. A refusal or uncertain response stops later \
    writes; earlier changes remain. Recheck only reads state and never continues pending steps. \
    No automatic compensation or cleanup occurs. After a known partial result, use the separately \
    reviewed Restore connection controls below. Resolve unknown outcomes before starting another \
    change.";

/// The review of an operation, word for word from Rune. Hosts of a repair
/// are listed coordinator first, then by host id.
pub fn lines(i: &Inventory, op: &StorageGraph) -> Vec<String> {
    let mut lines = vec![op.label().to_owned()];
    match op {
        StorageGraph::Repair {
            coordinator,
            source,
            hosts,
        } => {
            lines.push("Configuration source".into());
            lines.extend(attachment_lines(i, source));
            lines.push(
                "Creates only reviewed missing host/SR connections, then plugs selected \
                 connections. Coordinator runs first when selected. Custom or host-dependent \
                 configuration is refused before any change."
                    .into(),
            );
            lines.push(
                if source.requires_credentials {
                    "The host copies source credential references into fresh attachment-owned \
                     Secrets. Password values are not read or shown. Failed creation may leave \
                     unattributed resources; no automatic deletion is attempted."
                } else {
                    "Uses the exact existing source connection configuration, rechecked before \
                     creation."
                }
                .into(),
            );
            let mut hosts = hosts.clone();
            hosts.sort_by_key(|h| (h.host != *coordinator, h.host));
            for h in hosts {
                lines.push(host_name(i, h.host));
                lines.push(format!(
                    "{} · {}{}",
                    h.host,
                    if h.attachment.is_none() {
                        "Create and connect"
                    } else {
                        "Connect existing"
                    },
                    if h.host == *coordinator {
                        " · coordinator"
                    } else {
                        ""
                    }
                ));
                if let Some(p) = h.attachment {
                    lines.push(format!("Connection {}", p.pbd));
                }
            }
        }
        StorageGraph::Multipath {
            host,
            expected,
            expected_handler,
            enabled,
            attachments,
        } => {
            let handler = |on: bool| if on { "dmp" } else { "absent" };
            lines.push(format!("{} ({host})", host_name(i, *host)));
            lines.push(format!("Multipathing: {expected} → {enabled}"));
            lines.push(format!(
                "Handler: {} → {}",
                handler(*expected_handler),
                handler(*enabled)
            ));
            lines.push(
                "Disconnects every originally attached repository except the host's local-cache \
                 one, changes the host policy, then reconnects exactly that subset. Originally \
                 detached connections remain detached. The backend requires a disabled, empty \
                 maintenance host, HA off and no active/reserved iSCSI or HBA storage consumers \
                 across the pool. Storage used for caching or metadata replication is refused."
                    .into(),
            );
            for p in attachments {
                lines.extend(attachment_lines(i, p));
                lines.push(
                    if p.local_cache {
                        "Local-cache repository: kept attached, multipathing does not affect it"
                    } else if p.attached {
                        "Disconnect, then reconnect after the policy change"
                    } else {
                        "Preserve detached state"
                    }
                    .into(),
                );
            }
            lines.push(
                "This does not configure storage-array paths, switches or custom multipath.conf. \
                 Attached status and policy readback do not prove working multipath I/O."
                    .into(),
            );
        }
    }
    lines.push(CLOSING.into());
    lines
}
