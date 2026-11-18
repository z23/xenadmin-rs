//! Add Server to Pool: the logic of the Rune `profile_pool_join.rs`,
//! carried over. A standalone server (the source) joins another pool that
//! this client is connected to independently (the destination): choose the
//! destination, then review with the destination's credentials and the
//! source server's UUID typed.
//!
//! The editor belongs to two saved connections, not to the tree selection:
//! it was a child of the Rune connection manager. Its `current` check reads
//! both connections and fails closed when either moved on.

use xenadmin_core::{
    ConnectionProfile, Credentials, HostId, ObservedResource, OperationId, PoolId,
    PoolJoinSelection, ProfileId, ResourceTarget,
};

use crate::prelude::*;

/// The refusal of a review whose connections moved on, in the function.
pub const STALE: &str = "Connection settings or pool identity changed; reopen the join review";

/// The same, as the poll writes it into the review.
pub const CHANGED: &str =
    "Connection settings or pool identity changed. Close this review and reconnect both pools.";

pub const NO_SOURCE: &str =
    "Joining requires a writable, current connection to a standalone source host";

pub const CONFIRM: &str = "Enter the source host UUID exactly";

#[derive(Clone)]
pub struct Captured {
    pub profile: ConnectionProfile,
    pub observed: Option<ObservedResource>,
    pub pool: PoolId,
    pub name: String,
    pub host: HostId,
    pub host_name: String,
    pub address: String,
}

impl Captured {
    pub fn capture(m: &Model, id: ProfileId, standalone: bool) -> Option<Self> {
        let connection = m.workspace.active.get(&id)?;
        if connection.is_read_only() {
            return None;
        }
        let snapshot = connection.latest();
        let inventory = snapshot.inventory()?;
        if standalone && inventory.hosts.len() != 1 {
            return None;
        }
        let host = inventory.hosts.get(&inventory.coordinator?)?;
        Some(Self {
            profile: m.workspace.profile(id)?.clone(),
            observed: Some(
                snapshot
                    .observe_resource(ResourceTarget::Pool(inventory.pool_id))
                    .ok()?,
            ),
            pool: inventory.pool_id,
            name: inventory.pool_name.clone(),
            host: host.id,
            host_name: host.name.clone(),
            address: host.address.clone(),
        })
    }

    pub fn current(&self, m: &Model) -> bool {
        m.workspace.profile(self.profile.id) == Some(&self.profile)
            && m.workspace
                .active
                .get(&self.profile.id)
                .is_some_and(|c| {
                    !c.is_read_only()
                        && self.observed.is_some()
                        && c.latest()
                            .observe_resource(ResourceTarget::Pool(self.pool))
                            .ok()
                            == self.observed
                })
    }
}

pub struct Editor {
    /// The standalone server that will join.
    pub source: Captured,
    /// The other connected pools, one per pool, at most 128.
    pub destinations: Vec<Captured>,
    pub destination: Option<usize>,
    pub review: bool,
    pub username: String,
    /// The destination's password, for this join only.
    pub password: SecretText,
    pub confirmation: String,
    pub error: Option<String>,
}

/// The editor for a source pool, or why there is none.
pub fn capture(m: &Model, pool: PoolId) -> Result<Editor, String> {
    let source = m
        .workspace
        .active
        .keys()
        .find_map(|id| Captured::capture(m, *id, true).filter(|v| v.pool == pool))
        .ok_or(NO_SOURCE)?;
    let mut pools = std::collections::BTreeSet::new();
    let destinations = m
        .workspace
        .active
        .keys()
        .filter_map(|id| Captured::capture(m, *id, false))
        .filter(|c| c.pool != pool && pools.insert(c.pool))
        .take(128)
        .collect();
    Ok(Editor {
        source,
        destinations,
        destination: None,
        review: false,
        username: "root".into(),
        password: SecretText::new(),
        confirmation: String::new(),
        error: None,
    })
}

/// The Rune `open`, for the pool of the command's target. A refusal goes
/// to the shared error line.
pub fn open(m: &mut Model, pool: PoolId) {
    match capture(m, pool) {
        Ok(editor) => {
            super::close_all(m);
            m.areas.pool.join = Some(editor);
        }
        Err(error) => m.error = Some(error),
    }
}

/// Choosing a destination starts its review afresh: no password, no typed
/// UUID, the administrator "root".
pub fn choose(m: &mut Model, index: usize) {
    if let Some(e) = &mut m.areas.pool.join
        && index < e.destinations.len()
    {
        e.destination = Some(index);
        e.review = true;
        e.password.clear();
        e.username = "root".into();
        e.confirmation = String::new();
        e.error = None;
    }
}

/// Back to the list of destinations; the credentials are dropped.
pub fn back(m: &mut Model) {
    if let Some(e) = &mut m.areas.pool.join {
        e.review = false;
        e.destination = None;
        e.password.clear();
        e.confirmation = String::new();
    }
}

pub fn close(m: &mut Model) {
    m.areas.pool.join = None;
}

pub fn chosen(e: &Editor) -> Option<&Captured> {
    e.destination.and_then(|i| e.destinations.get(i))
}

pub fn current(m: &Model, e: &Editor) -> bool {
    e.source.current(m) && chosen(e).is_some_and(|d| d.current(m))
}

/// The Rune `s.demo`: the scope session is synthetic, or the source pool
/// is shown by a synthetic session.
pub fn synthetic(m: &Model, e: &Editor) -> bool {
    m.demo || m.is_synthetic_pool(e.source.pool)
}

/// A review whose connections moved on loses its password and says so.
/// True when that changed what is shown.
pub fn poll(m: &mut Model) -> bool {
    let Some(e) = m.areas.pool.join.as_ref() else {
        return false;
    };
    if synthetic(m, e) {
        return false;
    }
    let stale = e.review && !current(m, e);
    if stale && let Some(e) = &mut m.areas.pool.join {
        let changed = !e.password.is_empty() || e.error.is_none();
        e.password.clear();
        e.error = Some(CHANGED.into());
        return changed;
    }
    false
}

pub fn apply(m: &mut Model) {
    let prepared = (|| {
        let e = m.areas.pool.join.as_ref().ok_or("Join review is closed")?;
        if synthetic(m, e) || !m.workspace.writable || !current(m, e) {
            return Err(STALE.to_owned());
        }
        let destination = e
            .destinations
            .get(e.destination.ok_or("Choose a destination pool")?)
            .ok_or("Choose a destination pool")?;
        if e.confirmation != e.source.host.to_string() {
            return Err(CONFIRM.into());
        }
        let selected = PoolJoinSelection {
            source: e.source.observed.ok_or("Source connection is stale")?,
            source_host: e.source.host,
            destination: destination
                .observed
                .ok_or("Destination connection is stale")?,
            destination_coordinator: destination.host,
            destination_address: destination.address.clone(),
        };
        Ok((
            selected,
            e.source.profile.id,
            destination.profile.id,
            e.username.trim().to_owned(),
        ))
    })();
    let result = prepared.and_then(|(selected, source_id, destination_id, username)| {
        // The password leaves the draft with the submission, whatever its
        // result.
        let password = m
            .areas
            .pool
            .join
            .as_mut()
            .map(|e| e.password.take())
            .unwrap_or_default();
        let source = m
            .workspace
            .active
            .get(&source_id)
            .ok_or("Source connection was closed")?;
        let destination = m
            .workspace
            .active
            .get(&destination_id)
            .ok_or("Destination connection was closed")?;
        source
            .join_pool(
                OperationId::new(),
                selected,
                destination,
                Credentials::new(username, password.expose().to_owned()),
            )
            .map_err(|e| e.to_string())
    });
    match result {
        Ok(handle) => {
            m.track_resource(handle);
            m.areas.pool.join = None;
        }
        Err(error) => {
            if let Some(e) = &mut m.areas.pool.join {
                e.password.clear();
                e.error = Some(error);
            }
        }
    }
}
