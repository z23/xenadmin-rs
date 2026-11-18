//! The open workflow of the area: a draft, its review and its submission.
//!
//! In Rune the drafts of this area were forms of the networking workspace
//! (`Form::Topology`, `Form::Fabric`, `Form::FabricPool`) and shared its
//! review, its "Back", its confirm rule and its `submit`. A wizard ends on
//! its own review, so the drafts of this area keep theirs here; the rules
//! are the ones of `network_admin.rs`, word for word, as the `network`
//! area carried them over (`network::workspace`):
//!
//! - [`review`] ("Review network change") validates the draft with the
//!   Rune function of its module and observes the target ([`prepare`]);
//! - [`back`] clears the review only; an edit of the draft does not exist
//!   while a review is shown;
//! - the confirm rule: a writable current connection, the target observed
//!   as it was ([`current`], [`stale`]), the staleness rule of the module
//!   ([`unavailable`]), no known permission denial ([`permission`]), not a
//!   synthetic pool;
//! - [`submit`] ("Confirm network change") makes every one of these checks
//!   again and builds the command by action, as the Rune `submit` did for
//!   a network operation and a network topology operation.
//!
//! The changes with nothing to enter (a service purpose, the removal of a
//! pool's coverage) have no draft: their review is the shared dialog
//! `network-change` of the `network` area (`network::open_review`).

use xenadmin_core::{
    Error, Inventory, NetworkId, NetworkOperation, NetworkTopologyOperation, ObservedResource,
    OperationId, PermissionAssessment, PoolId, ResourceAction, ResourceCommand, ResourceTarget,
};

use super::{fabric, topology};
use crate::areas::network;
use crate::prelude::*;

/// The reason of a review whose connection or target moved on (the text of
/// the `network` area for the same condition).
pub const STALE: &str = network::workspace::STALE;

/// The steps of a wizard before its review: Type, Name, Settings.
pub const TYPE: usize = 0;
pub const NAME: usize = 1;
pub const SETTINGS: usize = 2;

pub enum Draft {
    /// The dialog `network-topology`: pool-wide VLAN creation or retagging.
    Topology(topology::Editor),
    /// The wizard `network-fabric`: a tunnel or SR-IOV interface on one host.
    Fabric(fabric::Editor),
    /// The wizard `network-fabric-pool`: coverage of every host of the pool.
    FabricPool(fabric::PoolEditor),
    /// The dialog `network-fabric-remove`: one tunnel or SR-IOV record.
    Removal(fabric::Removal),
}

impl Draft {
    /// A wizard (Type, Name, Settings, Review), not a dialog.
    pub const fn wizard(&self) -> bool {
        matches!(self, Self::Fabric(_) | Self::FabricPool(_))
    }
}

pub struct Review {
    pub target: ResourceTarget,
    pub observed: Option<ObservedResource>,
    pub operation: ResourceAction,
    pub lines: Vec<String>,
}

pub struct Editor {
    pub pool: PoolId,
    /// The network the change is about: the target of the command.
    pub network: NetworkId,
    pub draft: Draft,
    /// The step of a wizard while it has no review ([`TYPE`], [`NAME`],
    /// [`SETTINGS`]); a dialog stays on 0.
    pub step: usize,
    pub review: Option<Review>,
    pub error: Option<String>,
}

/// Opens a workflow on its draft. The dialog of the `network` area gives
/// way (Rune: one workspace, whose form was replaced).
pub fn open(m: &mut Model, pool: PoolId, network: NetworkId, draft: Draft) {
    network::close(m);
    m.areas.network_services.editor = Some(Editor {
        pool,
        network,
        draft,
        step: TYPE,
        review: None,
        error: None,
    });
}

pub fn edit(m: &mut Model) -> Option<&mut Editor> {
    m.areas.network_services.editor.as_mut()
}

/// Next and Back of a wizard, among the steps of its draft.
pub fn step(m: &mut Model, by: isize) {
    if let Some(e) = edit(m) {
        e.step = e.step.saturating_add_signed(by).min(SETTINGS);
        e.error = None;
    }
}

/// Back from the review to what was entered (Rune: clears the review only).
pub fn back(m: &mut Model) {
    if let Some(e) = edit(m) {
        e.review = None;
        e.error = None;
    }
}

/// The complete change of a draft, or the Rune reason it has none yet.
pub fn staged(
    e: &Editor,
    i: &Inventory,
) -> Result<(ResourceTarget, ResourceAction, Vec<String>), String> {
    let network = ResourceTarget::Network(e.network);
    Ok(match &e.draft {
        Draft::Topology(editor) => {
            let (operation, lines) = topology::review(editor, i)?;
            (
                ResourceTarget::Network(editor.network),
                ResourceAction::NetworkTopology(operation),
                lines,
            )
        }
        Draft::Fabric(editor) => {
            let (target, operation, lines) = fabric::review(e.network, editor, i)?;
            (target, ResourceAction::Network(operation), lines)
        }
        Draft::FabricPool(editor) => {
            let (operation, lines) = fabric::reviewed(e.network, editor, i)?;
            (
                network,
                ResourceAction::NetworkTopology(NetworkTopologyOperation::Fabric(Box::new(
                    operation,
                ))),
                lines,
            )
        }
        Draft::Removal(removal) => {
            let (target, operation, lines) = fabric::review_removal(e.network, removal, i)?;
            (target, ResourceAction::Network(operation), lines)
        }
    })
}

/// "Review network change": the Rune `prepare` of the draft's module.
pub fn review(m: &mut Model) {
    let result = (|| {
        let e = m
            .areas
            .network_services
            .editor
            .as_ref()
            .ok_or("Close and reopen networking")?;
        let i = m.inventory_for(e.pool).ok_or("The pool changed")?;
        staged(e, &i)
    })();
    match result {
        Ok((target, operation, lines)) => prepare(m, target, operation, lines),
        Err(error) => {
            if let Some(e) = edit(m) {
                e.error = Some(error);
            }
        }
    }
}

/// Stores the review of the open editor (the Rune `prepare_action`). In a
/// synthetic pool there is no observation; otherwise the target is observed
/// now, and a failure is the editor's error.
pub fn prepare(
    m: &mut Model,
    target: ResourceTarget,
    operation: ResourceAction,
    lines: Vec<String>,
) {
    let Some(pool) = m.areas.network_services.editor.as_ref().map(|e| e.pool) else {
        return;
    };
    let observed = if m.is_synthetic_pool(pool) {
        None
    } else {
        match m
            .snapshot_for(pool)
            .ok_or(Error::Stale)
            .and_then(|snap| snap.observe_resource(target))
        {
            Ok(observed) => Some(observed),
            Err(error) => {
                if let Some(e) = edit(m) {
                    e.error = Some(error.to_string());
                }
                return;
            }
        }
    };
    if let Some(e) = edit(m) {
        e.review = Some(Review {
            target,
            observed,
            operation,
            lines,
        });
        e.error = None;
    }
}

/// Whether the editor's pool is a synthetic one: reviews are previews.
pub fn synthetic(m: &Model, e: &Editor) -> bool {
    m.is_synthetic_pool(e.pool)
}

/// The Rune rule of the confirm button for the connection: a writable
/// current connection, and the reviewed target observed as it was.
pub fn current(m: &Model, e: &Editor) -> bool {
    m.can_mutate_pool(e.pool)
        && e.review
            .as_ref()
            .and_then(|r| r.observed)
            .is_some_and(|observed| {
                m.snapshot_for(e.pool)
                    .and_then(|snap| snap.observe_resource(observed.target()).ok())
                    == Some(observed)
            })
}

/// Why the open review cannot be confirmed any more. `None` in a synthetic
/// pool, which says so itself.
pub fn stale(m: &Model, e: &Editor) -> Option<&'static str> {
    (e.review.is_some() && !synthetic(m, e) && !current(m, e)).then_some(STALE)
}

/// The staleness rule of the reviewed operation's module: the Rune
/// `fabric::unavailable` for a host-scoped fabric operation,
/// `topology::review_unavailable` for a topology operation (pool-wide
/// VLANs and pool-wide fabric coverage).
pub fn unavailable(m: &Model, e: &Editor) -> Option<String> {
    let review = e.review.as_ref()?;
    fabric::unavailable_for(m, e.pool, &review.operation)
        .or_else(|| topology::unavailable_for(m, e.pool, review.target, &review.operation))
}

pub fn permission(m: &Model, e: &Editor) -> Option<PermissionAssessment> {
    e.review.as_ref().map(|review| {
        permissions::assess_for(
            m,
            e.pool,
            &review.operation.permission_requirements(review.target),
        )
    })
}

/// "Confirm network change". Every rule of the confirm button is checked
/// again (Rune relied on the button for the observation and the
/// permission). On success the surface closes: the operation is followed
/// on the Networking tab ("Latest operation") and in the Events tab.
pub fn submit(m: &mut Model) {
    let result = (|| {
        let e = m
            .areas
            .network_services
            .editor
            .as_ref()
            .ok_or(Error::Stale)?;
        if !m.can_mutate_pool(e.pool) {
            return Err(Error::Stale);
        }
        let review = e.review.as_ref().ok_or(Error::Stale)?;
        if fabric::unavailable_for(m, e.pool, &review.operation).is_some() {
            return Err(Error::ResourceConflict);
        }
        let target = review.observed.ok_or(Error::Stale)?;
        if !current(m, e) {
            return Err(Error::Stale);
        }
        if permission(m, e)
            .as_ref()
            .and_then(permissions::denial)
            .is_some()
        {
            return Err(Error::Api("RBAC_PERMISSION_DENIED".into()));
        }
        let command = match &review.operation {
            ResourceAction::Network(operation @ NetworkOperation::Fabric(_)) => {
                ResourceCommand::Network {
                    target,
                    operation: operation.clone(),
                }
            }
            ResourceAction::NetworkTopology(operation) => {
                let ResourceTarget::Network(network) = review.target else {
                    return Err(Error::Stale);
                };
                if !m
                    .inventory_for(e.pool)
                    .is_some_and(|i| operation.availability(&i, network).is_available())
                {
                    return Err(Error::Stale);
                }
                ResourceCommand::NetworkTopology {
                    target,
                    operation: operation.clone(),
                }
            }
            _ => return Err(Error::InvalidResourceCommand),
        };
        m.connection_for(e.pool)
            .ok_or(Error::Closed)?
            .submit_resource(OperationId::new(), command)
    })();
    match result {
        Ok(operation) => {
            m.track_resource(operation);
            super::close(m);
        }
        Err(error) => {
            if let Some(e) = edit(m) {
                e.error = Some(error.to_string());
            }
        }
    }
}
