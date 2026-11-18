//! The logic carried over from the Rune module `storage_maintenance_view.rs`
//! (a child of `storage_view`): repair of existing detached host attachments
//! in a reviewed order, and the read-cache policy of a repository. Texts of
//! errors and reviews are the Rune ones, word for word.
//!
//! What changed with the port:
//!
//! - The Rune module shared the editor of the "Storage administration"
//!   dialog (`maintenance_review`, `repair_selection`, `backends`, `name`,
//!   `observed`). That dialog is gone; this editor has those fields of its
//!   own and is one of two things, by [`Kind`]: the dialog `storage-repair`
//!   that `storage.repair` opens, or the Read Caching page of the
//!   repository's Properties dialog.
//! - The editor follows the pool of its repository, not the scope session.
//! - Driver capabilities ("backends") were read when the Rune dialog opened;
//!   here they are read when the Read Caching page is entered, the only
//!   place that shows them. The fence is the Rune one: a result is accepted
//!   only while the pool observation it was asked under is still current.
//! - Rune pruned finished handles from `resource_operations` before it
//!   counted to 64. The list is the kernel's here: the cap counts the
//!   unfinished handles and removes nothing.
//! - Rune's submit did not look at the permission again (only its button
//!   did); the port refuses a known denial in the handler too.

use std::collections::BTreeSet;

use xenadmin_core::{
    ActionAvailability, Error, ObservedResource, OperationId, PbdId, PermissionAssessment, PoolId,
    ResourceCommand, ResourceOperation, ResourceTarget, ReviewedStorageAttachment, SrId,
    StorageBackend, StorageDiscovery, StorageDiscoveryResult, StorageMaintenance, StorageQuery,
};

use crate::prelude::*;

/// The id of the Read Caching page (the plan's page table).
pub const PAGE: &str = "read-caching";

/// What the review says when the connection it was captured on is not the
/// current one any more. Also the reason its Confirm button is disabled.
// It names "storage administration" of the Rune front end; kept word for
// word until every module's review texts are changed together (decision 10).
pub const STALE_REVIEW: &str = "The captured connection changed; close and reopen storage administration before submitting.";

/// What `stage` refuses with for the same reason.
pub const STALE_STAGE: &str = "The captured connection changed; reopen storage administration";

/// What `stage` refuses with when the reviewed operation is not available:
/// nothing selected, a selected attachment that is attached now, a cache
/// policy that is what the change would make it.
pub const NOTHING_TO_DO: &str =
    "Choose current detached host attachments or a changed cache policy";

pub const POLICY_UNKNOWN: &str = "Read-cache policy was not provided by the host";

/// The reason a synthetic pool gives a disabled confirm button (the text
/// every area uses).
pub const DEMO: &str = "Demo mode cannot change infrastructure.";

/// The reason Rune's Confirm button was disabled without saying so on a
/// connection that cannot write.
pub const NOT_WRITABLE: &str = "This connection is read-only, disconnected or stale.";

/// Which of the two changes the editor is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Plug existing detached attachments: the dialog `storage-repair`.
    Repair,
    /// The read-cache policy: the Read Caching page of the Properties dialog.
    Cache,
}

pub struct Editor {
    pub pool: PoolId,
    pub sr: SrId,
    /// The repository as the connection showed it at open. `None` in a
    /// synthetic pool.
    pub observed: Option<ObservedResource>,
    pub name: String,
    pub kind: Kind,
    /// Repair: the detached attachments chosen for the review.
    pub selection: BTreeSet<PbdId>,
    pub review: Option<StorageMaintenance>,
    /// Cache: the driver capabilities, once discovery answered.
    pub backends: Vec<StorageBackend>,
    pub discovery: Option<StorageDiscovery>,
    /// The pool observation the running or last discovery was asked under.
    pub query_observed: Option<ObservedResource>,
    pub error: Option<String>,
}

impl Editor {
    pub fn target(&self) -> ResourceTarget {
        ResourceTarget::Storage(self.sr)
    }

    /// The editor of `kind` for a repository the inventory of its pool has.
    pub fn capture(m: &Model, object: &ObjectRef, kind: Kind) -> Option<Self> {
        if object.kind != ObjectKind::StorageRepository {
            return None;
        }
        let pool = object.pool?;
        let uuid = object.uuid()?;
        let inventory = m.inventory_for(pool)?;
        let name = inventory
            .storage
            .iter()
            .find(|row| row.uuid == uuid)?
            .name
            .clone();
        let sr = SrId::from(uuid);
        let observed = m
            .snapshot_for(pool)
            .and_then(|snapshot| snapshot.observe_resource(ResourceTarget::Storage(sr)).ok());
        Some(Self {
            pool,
            sr,
            observed,
            name,
            kind,
            selection: BTreeSet::new(),
            review: None,
            backends: Vec::new(),
            discovery: None,
            query_observed: None,
            error: None,
        })
    }
}

/// Opens the Repair dialog on a repository. `false` when the repository is
/// not in the inventory of its pool.
pub fn open_repair(m: &mut Model, pool: PoolId, sr: SrId) -> bool {
    let object = ObjectRef::storage(pool, sr.uuid());
    let Some(editor) = Editor::capture(m, &object, Kind::Repair) else {
        return false;
    };
    super::close(m);
    m.areas.storage_maintenance.editor = Some(editor);
    true
}

/// Whether the editor's pool is a synthetic one, in which nothing can be
/// submitted. For the scope session this is `m.demo`.
pub fn synthetic(m: &Model, e: &Editor) -> bool {
    m.is_synthetic_pool(e.pool)
}

/// The Rune `current`: the repository still reads as it did at open.
pub fn current(m: &Model, e: &Editor) -> bool {
    m.snapshot_for(e.pool).is_some_and(|snapshot| {
        e.observed
            .is_some_and(|observed| snapshot.observe_resource(e.target()).ok() == Some(observed))
    })
}

/// Why the open review cannot be confirmed any more. `None` in a synthetic
/// pool, where nothing can be submitted anyway, and while nothing is
/// reviewed.
pub fn stale(m: &Model, e: &Editor) -> Option<String> {
    (e.review.is_some() && !synthetic(m, e) && !current(m, e)).then(|| STALE_REVIEW.to_owned())
}

/// The availability of an operation against the live inventory.
pub fn availability(m: &Model, e: &Editor, op: &StorageMaintenance) -> ActionAvailability {
    m.inventory_for(e.pool)
        .map(|inventory| op.availability(&inventory, e.sr))
        .unwrap_or(ActionAvailability::Stale)
}

pub fn permission(m: &Model, e: &Editor) -> Option<PermissionAssessment> {
    e.review
        .as_ref()
        .map(|op| permissions::assess_for(m, e.pool, &op.permission_requirements()))
}

/// Builds the operation of the editor's kind from the live inventory and
/// puts it up for review, or says why not.
pub fn stage(m: &mut Model) {
    let result = (|| {
        let e = m
            .areas
            .storage_maintenance
            .editor
            .as_ref()
            .ok_or("Storage view closed")?;
        let i = m
            .inventory_for(e.pool)
            .ok_or("The original pool is no longer current")?;
        if !synthetic(m, e) && !current(m, e) {
            return Err(STALE_STAGE);
        }
        let sr = e.sr;
        let op = match e.kind {
            Kind::Cache => {
                let d = i
                    .details
                    .storage
                    .get(&sr)
                    .ok_or("Storage details unavailable")?;
                if !d.read_cache_policy_known {
                    return Err(POLICY_UNKNOWN);
                }
                // The host's override "true" means caching is disabled, so
                // `enabled` is the state after the change.
                StorageMaintenance::ReadCaching {
                    expected: d.read_cache_override.clone(),
                    enabled: d.read_cache_override.as_deref() == Some("true"),
                }
            }
            Kind::Repair => {
                let attachments = e
                    .selection
                    .iter()
                    .map(|id| ReviewedStorageAttachment::from_inventory(&i, sr, *id))
                    .collect::<Option<Vec<_>>>()
                    .ok_or("A selected attachment changed; select it again")?;
                StorageMaintenance::Repair {
                    coordinator: i.coordinator.ok_or("Pool coordinator is unavailable")?,
                    attachments,
                }
            }
        };
        if !op.availability(&i, sr).is_available() {
            return Err(NOTHING_TO_DO);
        }
        Ok(op)
    })();
    let Some(e) = &mut m.areas.storage_maintenance.editor else {
        return;
    };
    match result {
        Ok(op) => {
            e.review = Some(op);
            e.error = None;
        }
        Err(error) => e.error = Some(error.into()),
    }
}

/// Back from the review to the selection or the page.
pub fn back(m: &mut Model) {
    if let Some(e) = &mut m.areas.storage_maintenance.editor {
        e.review = None;
    }
}

/// Includes an attachment in the repair, or takes it out.
pub fn toggle(m: &mut Model, id: PbdId) {
    if let Some(e) = &mut m.areas.storage_maintenance.editor
        && !e.selection.remove(&id)
    {
        e.selection.insert(id);
    }
}

/// The unfinished resource operations the model holds. Rune dropped the
/// finished handles first and counted what was left.
fn unfinished(m: &Model) -> usize {
    m.resource_operations
        .iter()
        .filter(|operation| !operation.latest().state.terminal())
        .count()
}

/// Why the reviewed operation cannot be submitted now; the reason on the
/// Confirm button. The first that applies: the stale review, the operation
/// not available any more, a known denial, the synthetic pool, a connection
/// that cannot write.
pub fn blocked(m: &Model, e: &Editor, op: &StorageMaintenance) -> Option<String> {
    if let Some(reason) = stale(m, e) {
        return Some(reason);
    }
    let availability = availability(m, e, op);
    if !availability.is_available() {
        return Some(availability.explanation());
    }
    let permission = permissions::assess_for(m, e.pool, &op.permission_requirements());
    if let Some(denial) = permissions::denial(&permission) {
        return Some(denial);
    }
    if synthetic(m, e) {
        return Some(DEMO.into());
    }
    (!m.can_mutate_pool(e.pool)).then(|| NOT_WRITABLE.to_owned())
}

/// Submits the reviewed operation. Accepted, it is tracked and the editor
/// closes (it is followed in Events); refused, the reason is the editor's
/// error and the review stays.
pub fn apply(m: &mut Model) {
    match submission(m) {
        Ok(operation) => {
            m.track_resource(operation);
            super::close(m);
        }
        Err(error) => {
            if let Some(e) = &mut m.areas.storage_maintenance.editor {
                e.error = Some(error);
            }
        }
    }
}

/// The checks of the Rune `apply`, in its order, then the denial.
fn submission(m: &Model) -> Result<ResourceOperation, String> {
    let e = m
        .areas
        .storage_maintenance
        .editor
        .as_ref()
        .ok_or_else(|| Error::Closed.to_string())?;
    if synthetic(m, e) || !current(m, e) {
        return Err(Error::Stale.to_string());
    }
    if unfinished(m) >= 64 {
        return Err(Error::Busy.to_string());
    }
    let op = e
        .review
        .as_ref()
        .ok_or_else(|| Error::InvalidResourceCommand.to_string())?;
    if !availability(m, e, op).is_available() {
        return Err(Error::Stale.to_string());
    }
    let permission = permissions::assess_for(m, e.pool, &op.permission_requirements());
    if let Some(denial) = permissions::denial(&permission) {
        return Err(denial);
    }
    m.connection_for(e.pool)
        .ok_or_else(|| Error::Closed.to_string())?
        .submit_resource(
            OperationId::new(),
            ResourceCommand::StorageMaintenance {
                target: e.observed.ok_or_else(|| Error::Stale.to_string())?,
                operation: op.clone(),
            },
        )
        .map_err(|error| error.to_string())
}

// -- Driver capabilities ---------------------------------------------------------------

/// Asks the pool for its storage drivers (the Rune `query(s, 0)`). A
/// refusal is the editor's error, except in a synthetic pool, where there
/// is nothing to ask and Rune stayed silent.
pub fn query(m: &mut Model) {
    let result = (|| {
        let e = m
            .areas
            .storage_maintenance
            .editor
            .as_ref()
            .ok_or("Storage view is closed")?;
        let snapshot = m.snapshot_for(e.pool).ok_or("Connect to discover storage")?;
        if snapshot.inventory().is_none_or(|i| i.pool_id != e.pool) {
            return Err("Connection changed; reopen storage".to_owned());
        }
        let observed = snapshot
            .observe_resource(ResourceTarget::Pool(e.pool))
            .map_err(|error| error.to_string())?;
        let connection = m
            .connection_for(e.pool)
            .ok_or("Connect before discovery")?;
        let handle = connection
            .discover_storage(observed, StorageQuery::Backends)
            .map_err(|error| error.to_string())?;
        if let Some(waker) = &m.waker {
            handle.notify_on_change(waker.clone());
        }
        Ok::<_, String>((handle, observed))
    })();
    let synthetic = m
        .areas
        .storage_maintenance
        .editor
        .as_ref()
        .is_some_and(|e| m.is_synthetic_pool(e.pool));
    let Some(e) = &mut m.areas.storage_maintenance.editor else {
        return;
    };
    match result {
        Ok((handle, observed)) => {
            e.discovery = Some(handle);
            e.query_observed = Some(observed);
            e.error = None;
        }
        Err(error) => {
            if !synthetic {
                e.error = Some(error);
            }
        }
    }
}

/// Reads the discovery handle. `true` when its result arrived.
pub fn poll(m: &mut Model, _clock: f64) -> bool {
    let Some(e) = m.areas.storage_maintenance.editor.as_ref() else {
        return false;
    };
    let Some(result) = e.discovery.as_ref().and_then(StorageDiscovery::latest) else {
        return false;
    };
    let current = synthetic(m, e)
        || e.query_observed.is_some_and(|observed| {
            m.snapshot_for(e.pool)
                .and_then(|snapshot| snapshot.observe_resource(observed.target()).ok())
                == Some(observed)
        });
    let Some(e) = m.areas.storage_maintenance.editor.as_mut() else {
        return false;
    };
    e.discovery = None;
    if !current {
        e.error =
            Some("Storage inputs or connection changed during discovery. Discover again.".into());
        return true;
    }
    match result {
        Ok(StorageDiscoveryResult::Backends(backends)) => e.backends = backends,
        Ok(_) => {}
        Err(error) => e.error = Some(error),
    }
    true
}

// -- The Read Caching page ----------------------------------------------------------

/// Entering the page creates the editor afresh and asks for the driver
/// capabilities, as opening the Rune dialog did; leaving it drops the
/// editor.
pub fn page_event(m: &mut Model, target: &Target, event: PageEvent) {
    match event {
        PageEvent::Entered(id) if id == PAGE => {
            let editor = target
                .single()
                .and_then(|object| Editor::capture(m, object, Kind::Cache));
            m.areas.storage_maintenance.editor = editor;
            query(m);
        }
        PageEvent::Left(id) if id == PAGE => {
            let editor = &mut m.areas.storage_maintenance.editor;
            if editor.as_ref().is_some_and(|e| e.kind == Kind::Cache) {
                *editor = None;
            }
        }
        _ => {}
    }
}

// -- Review texts --------------------------------------------------------------------

/// The review of an operation, word for word from Rune. A repair lists its
/// attachments in the order they are plugged: the coordinator first, then
/// by host id.
pub fn lines(m: &Model, e: &Editor, op: &StorageMaintenance) -> Vec<String> {
    let mut lines = vec![
        format!("Repository: {}", e.name),
        format!("UUID: {}", e.sr.uuid()),
    ];
    match op {
        StorageMaintenance::Repair {
            coordinator,
            attachments,
        } => {
            let inventory = m.inventory_for(e.pool);
            let mut attachments = attachments.clone();
            attachments.sort_by_key(|p| (p.host != *coordinator, p.host));
            lines.push(
                "Only these existing attachments will be plugged, in order. A failure stops \
                 later hosts; earlier successful attachments remain connected. Stop remaining \
                 steps does not cancel an in-flight host task. Attached status does not prove \
                 storage health."
                    .into(),
            );
            for (n, p) in attachments.iter().enumerate() {
                let name = inventory
                    .as_ref()
                    .and_then(|i| i.hosts.get(&p.host))
                    .map(|h| h.name.as_str())
                    .unwrap_or("Host");
                lines.push(format!("{}. {name}", n + 1));
                lines.push(format!(
                    "Host {}{}",
                    p.host,
                    if p.host == *coordinator {
                        " · coordinator"
                    } else {
                        ""
                    }
                ));
                lines.push(format!("Attachment {}", p.attachment));
                if let Some(location) = &p.location {
                    lines.extend(crate::areas::storage::location_lines(location));
                }
                lines.push(
                    if p.requires_credentials {
                        "Uses credentials already stored by the host. No credentials are copied \
                         or replaced."
                    } else {
                        "Uses the existing host attachment configuration."
                    }
                    .into(),
                );
            }
        }
        StorageMaintenance::ReadCaching { enabled, .. } => {
            lines.push(
                if *enabled {
                    "Permit read caching by removing the repository override."
                } else {
                    "Disable read caching with the repository override."
                }
                .into(),
            );
            lines.push(
                "Preserves other repository settings. This changes policy only; it does not \
                 establish effective caching, alter driver provisioning, or reopen active disks. \
                 No VM restart is performed."
                    .into(),
            );
        }
    }
    lines
}
