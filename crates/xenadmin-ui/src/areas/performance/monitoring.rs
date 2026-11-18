//! The recording settings of a server: the Rune `host_monitoring_view.rs`
//! without its drawing.
//!
//! A [`Panel`] is the open dialog. It is captured for one server of one
//! pool and never retargets: the grid selection may move, the panel keeps
//! its server. It reads the server's catalog of RRD data sources with a
//! one-shot query, and for the selected source reviews one of two changes:
//! start recording, or forget the recorded history (destructive, with its
//! own acknowledgement). The reviewed operation holds the source exactly as
//! the catalog listed it; the core checks host, metadata and recording
//! state again before the write, and a lost reply stays unknown.
//!
//! Every text is the Rune one. New are only the reasons of controls Rune
//! disabled without saying why ([`ALREADY`], [`UNACKNOWLEDGED`]).

use xenadmin_core::{
    Error, HostDataSource, HostDataSourceQuery, HostOperation, HostRecordingAction,
    ObservedResource, OperationId, PoolId, ResourceCommand, ResourceOperation, ResourceTarget,
};

use crate::prelude::*;

pub struct Panel {
    pub pool: PoolId,
    pub target: ResourceTarget,
    pub name: String,
    pub observed: Option<ObservedResource>,
    /// The catalog query while it runs. Dropping it cancels it.
    pub query: Option<HostDataSourceQuery>,
    pub sources: Option<Vec<HostDataSource>>,
    pub selected: Option<String>,
    pub search: String,
    pub review: Option<HostOperation>,
    pub acknowledged: bool,
    pub error: Option<String>,
}

/// The line that replaces every control's use once the panel's connection
/// is no longer the one it was opened on.
pub const CHANGED: &str =
    "The connection changed. Close and reopen monitoring before applying a change.";
pub const READ_ONLY: &str =
    "Read-only connection: recording changes and history deletion are unavailable.";
pub const PREVIEW: &str = "Synthetic preview · no host changes can be submitted";
/// The catalog answer of a connection that is not the panel's any more.
pub const CATALOG_CHANGED: &str =
    "Connection changed while reading the catalog. Reopen monitoring for the current host.";
/// The reason every area gives a confirm button in a synthetic pool.
pub const DEMO: &str = "Demo mode cannot change infrastructure.";
/// Why "Review start recording" is disabled (Rune gave no reason).
pub const ALREADY: &str = "This data source is already being recorded.";
/// Why the deletion cannot be confirmed yet (Rune gave no reason).
pub const UNACKNOWLEDGED: &str =
    "Confirm that this source's recorded history will be deleted first.";

/// The panel a server would get, or `None` when the object is not a server
/// its pool's inventory has.
pub fn capture(m: &Model, object: &ObjectRef) -> Option<Panel> {
    if object.kind != ObjectKind::Host {
        return None;
    }
    let pool = object.pool?;
    let target = resource_target(object)?;
    let inventory = m.inventory_for(pool)?;
    let host = inventory.hosts.get(&object.host_id()?)?;
    Some(Panel {
        pool,
        target,
        name: host.name.clone(),
        observed: m
            .snapshot_for(pool)
            .and_then(|v| v.observe_resource(target).ok()),
        query: None,
        sources: None,
        selected: None,
        search: String::new(),
        review: None,
        acknowledged: false,
        error: None,
    })
}

/// Opens the dialog for the one server of the target and reads its catalog.
pub fn open(m: &mut Model, target: &Target) {
    let Some(panel) = target.single().and_then(|object| capture(m, object)) else {
        return;
    };
    m.areas.performance.monitoring = Some(panel);
    refresh(m);
}

pub fn close(m: &mut Model) {
    m.areas.performance.monitoring = None;
}

pub fn synthetic(m: &Model, p: &Panel) -> bool {
    m.is_synthetic_pool(p.pool)
}

/// Whether the panel's connection is still the one it was opened on: its
/// pool is shown, and a fresh observation of the server equals the
/// captured one (a reconnect changes it).
pub fn current(m: &Model, p: &Panel) -> bool {
    m.inventory_for(p.pool).is_some()
        && p.observed.is_some()
        && m.snapshot_for(p.pool)
            .and_then(|v| v.observe_resource(p.target).ok())
            == p.observed
}

/// [`CHANGED`] when the panel is not current and not a preview.
pub fn stale(m: &Model, p: &Panel) -> Option<String> {
    (!synthetic(m, p) && !current(m, p)).then(|| CHANGED.to_owned())
}

/// Discards the catalog, the selection and the review, and reads the
/// catalog again. A synthetic pool has none to read.
pub fn refresh(m: &mut Model) {
    let Some(p) = &mut m.areas.performance.monitoring else {
        return;
    };
    p.query = None;
    p.sources = None;
    p.review = None;
    p.selected = None;
    p.error = None;
    p.acknowledged = false;
    let (pool, observed) = (p.pool, p.observed);
    if m.is_synthetic_pool(pool) {
        return;
    }
    let result = observed.ok_or(Error::Stale).and_then(|observed| {
        m.connection_for(pool)
            .ok_or(Error::Closed)?
            .host_data_sources(observed)
    });
    let waker = m.waker.clone();
    let Some(p) = &mut m.areas.performance.monitoring else {
        return;
    };
    match result {
        Ok(q) => {
            if let Some(w) = waker {
                q.notify_on_change(w);
            }
            p.query = Some(q);
        }
        Err(e) => p.error = Some(e.to_string()),
    }
}

/// Collects the catalog when the query has answered. An answer is accepted
/// only for the server and the connection the panel was opened on.
pub fn poll(m: &mut Model) -> bool {
    let Some(p) = &m.areas.performance.monitoring else {
        return false;
    };
    let Some(result) = p.query.as_ref().and_then(HostDataSourceQuery::latest) else {
        return false;
    };
    let valid = current(m, p);
    let Some(p) = &mut m.areas.performance.monitoring else {
        return false;
    };
    p.query = None;
    match result {
        Ok(result) if valid && ResourceTarget::Host(result.host) == p.target => {
            p.sources = Some(result.sources)
        }
        Ok(_) => p.error = Some(CATALOG_CHANGED.into()),
        Err(e) => p.error = Some(e),
    }
    true
}

/// The selected source, as the catalog listed it.
pub fn selected(p: &Panel) -> Option<&HostDataSource> {
    p.sources
        .as_ref()?
        .iter()
        .find(|v| Some(&v.name) == p.selected.as_ref())
}

/// The sources the search text leaves: it is looked for in the name and in
/// the description, whatever the case.
pub fn visible(p: &Panel) -> Vec<&HostDataSource> {
    let needle = p.search.to_lowercase();
    p.sources
        .iter()
        .flatten()
        .filter(|v| {
            v.name.to_lowercase().contains(&needle)
                || v.description.to_lowercase().contains(&needle)
        })
        .collect()
}

/// Reviews a change of the selected source. A source that is recording
/// cannot be started again.
pub fn review(m: &mut Model, action: HostRecordingAction) {
    let Some(p) = &mut m.areas.performance.monitoring else {
        return;
    };
    let Some(source) = selected(p).cloned() else {
        return;
    };
    if action == HostRecordingAction::StartRecording && source.enabled {
        return;
    }
    p.review = Some(HostOperation::DataSource {
        expected: source,
        action,
    });
    p.acknowledged = false;
    p.error = None;
}

/// Back from the review to the catalog.
pub fn back(m: &mut Model) {
    if let Some(p) = &mut m.areas.performance.monitoring {
        p.review = None;
        p.acknowledged = false;
    }
}

pub fn acknowledge(m: &mut Model) {
    if let Some(p) = &mut m.areas.performance.monitoring {
        p.acknowledged = !p.acknowledged;
    }
}

fn forgets(op: &HostOperation) -> bool {
    matches!(
        op,
        HostOperation::DataSource {
            action: HostRecordingAction::ForgetHistory,
            ..
        }
    )
}

/// Why the review cannot be confirmed now, in the order the dialog names
/// its obstacles; `None` when it can (the Rune `can_apply`). The first
/// reason is the one the confirm button shows.
pub fn blocked(m: &Model, p: &Panel) -> Option<String> {
    let op = p.review.as_ref()?;
    if synthetic(m, p) {
        return Some(DEMO.into());
    }
    if m.connection_for(p.pool).is_some_and(|c| c.is_read_only()) {
        return Some(READ_ONLY.into());
    }
    if !m.can_mutate_pool(p.pool) || !current(m, p) {
        return Some(CHANGED.into());
    }
    if m.resource_operations.len() >= 32 {
        return Some(Error::Busy.to_string());
    }
    let availability = m
        .inventory_for(p.pool)
        .map(|i| op.availability(&i, p.target));
    match availability {
        Some(availability) if availability.is_available() => {}
        Some(availability) => return Some(availability.explanation()),
        None => return Some(CHANGED.into()),
    }
    let permission = permissions::assess_for(m, p.pool, &op.permission_requirements());
    if let Some(denial) = permissions::denial(&permission) {
        return Some(denial);
    }
    (forgets(op) && !p.acknowledged).then(|| UNACKNOWLEDGED.to_owned())
}

/// The confirm button. On success the handle goes to the model, the dialog
/// closes and the pool's history is shown, as in Rune. On a refusal the
/// dialog stays with the review and the reason.
pub fn submit(m: &mut Model) {
    let Some(p) = &m.areas.performance.monitoring else {
        return;
    };
    let pool = p.pool;
    match submission(m, p) {
        Ok(operation) => {
            m.track_resource(operation);
            m.areas.performance.monitoring = None;
            crate::areas::events::show_history(m, pool);
        }
        Err(e) => {
            if let Some(p) = &mut m.areas.performance.monitoring {
                p.error = Some(e.to_string());
            }
        }
    }
}

/// Every check of the button again, since a handler can run after the
/// state moved on; then what was reviewed is submitted, once.
fn submission(m: &Model, p: &Panel) -> Result<ResourceOperation, Error> {
    if p.review.is_none() || blocked(m, p).is_some() {
        return Err(Error::Stale);
    }
    m.connection_for(p.pool)
        .ok_or(Error::Closed)?
        .submit_resource(
            OperationId::new(),
            ResourceCommand::Host {
                target: p.observed.ok_or(Error::Stale)?,
                operation: p.review.clone().ok_or(Error::Stale)?,
            },
        )
}

/// The History button: the dialog closes and the history of the pool the
/// panel was captured for is shown, whatever pool the tree shows by then.
pub fn history(m: &mut Model) {
    let Some(p) = m.areas.performance.monitoring.take() else {
        return;
    };
    crate::areas::events::show_history(m, p.pool);
}

/// The lines of a source, in the catalog's details and in the review.
pub fn source_lines(source: &HostDataSource) -> Vec<String> {
    vec![
        source.description.clone(),
        format!(
            "Units: {} · range: {} to {}",
            source.units, source.minimum, source.maximum
        ),
        format!(
            "Recording at catalog refresh: {} · standard source: {}",
            if source.enabled {
                "enabled"
            } else {
                "disabled"
            },
            if source.standard { "yes" } else { "no" }
        ),
    ]
}

/// The review of an operation: the source as it was listed, what the
/// change does and does not do, and the closing line. Word for word.
pub fn review_lines(op: &HostOperation) -> Vec<String> {
    let HostOperation::DataSource { expected, action } = op else {
        return Vec::new();
    };
    let mut lines = vec![expected.name.clone()];
    lines.extend(source_lines(expected));
    lines.push(
        if *action == HostRecordingAction::ForgetHistory {
            "This deletes this source's recorded history from the host's RRD archives. It is not a pause and cannot be undone here. Standard/default sources may start recording again immediately; the host can also enable other sources automatically."
        } else {
            "Begin collecting this data source on the host. Earlier missing samples are not recovered. Changing the displayed graph alone does not enable host recording."
        }
        .into(),
    );
    lines.push("The exact host, source metadata and observed recording state are checked again before the change. A lost reply remains unknown and is never automatically repeated.".into());
    lines
}
