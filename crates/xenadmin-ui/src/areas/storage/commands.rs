//! The commands of the area: the Rune button "New storage" and what the
//! buttons of the Rune hub ("Storage attachments and lifecycle…") did.
//!
//! Eligibility, one rule for the commands on a repository:
//!
//! - `Hidden` when the target is not one object, or is of a kind that has
//!   nothing to do with repositories;
//! - `Disabled("Select a storage repository.")` when the target is the pool
//!   or the server whose Storage tab lists the repositories, and no row is
//!   selected: the command is in the local row of that grid;
//! - `Disabled` with the reason the review would give when nothing the user
//!   can enter would make it pass;
//! - `Enabled` otherwise. The open dialog makes every check again.
//!
//! Detach, Reattach and Remove Detached Attachment act on ONE host
//! attachment of the repository, as the Rune buttons did (one per
//! attachment). Which one: the attachment of the server whose Storage tab
//! the repository was selected on; otherwise the only attachment the
//! repository has. A repository with several attachments, reached from
//! anywhere else, has these commands disabled with the way to choose one.

use xenadmin_core::{PoolId, ResourceTarget, SrId, StorageAttachment};

use super::editor;
use super::view::NOT_WRITABLE;
use crate::prelude::*;

const AREA: &str = "storage";

pub const NEW: CommandId = CommandId("storage.new");
pub const TRIM: CommandId = CommandId("storage.trim");
pub const DETACH: CommandId = CommandId("storage.detach");
pub const REATTACH: CommandId = CommandId("storage.reattach");
pub const ATTACHMENT_CREATE: CommandId = CommandId("storage.attachment.create");
pub const ATTACHMENT_REMOVE: CommandId = CommandId("storage.attachment.remove");
pub const FORGET: CommandId = CommandId("storage.forget");
pub const DESTROY: CommandId = CommandId("storage.destroy");

pub const NO_POOL: &str = "Connect to a pool before creating storage.";
pub const SELECT: &str = "Select a storage repository.";
pub const SEVERAL: &str = "This repository has several host attachments. Select it on the Storage tab of a server to choose one.";
pub const NO_ATTACHMENT: &str = "This repository has no host attachment in the inventory.";
pub const NOT_ATTACHED: &str = "This host attachment is not attached.";
pub const ATTACHED: &str = "This host attachment is already attached.";
pub const UNKNOWN_STATE: &str =
    "The state of this host attachment is unknown. Refresh before changing it.";
pub const ONLY_DETACHED: &str = "Only a detached host attachment can be removed.";

/// In menu order. "New Storage", "Detach", "Reattach", "Forget" and
/// "Destroy" are the plan's items; the other three exist only in this
/// client and sit beside the items they belong with.
///
/// The orders of the local row of the Storage tab that this area leaves to
/// commands of other areas: 20 Repair (`storage_maintenance`), 30 Set as
/// Default and 35 Rescan (`network`, the generic editor of
/// `resource_admin.rs`).
pub fn commands() -> Vec<CommandSpec> {
    const REPOSITORY: &[ObjectKind] = &[ObjectKind::StorageRepository];
    vec![
        // On the local row of the Storage tab of a pool and of a server
        // (the plan's "Repository grid with New, ..."), not of a VM's or a
        // repository's: the menu and the toolbar show it for every target,
        // so the row is filtered by scope kind and not by `Hidden`.
        CommandSpec::new(NEW.0, "New Storage", AREA)
            .menu(place::STORAGE_NEW)
            .toolbar(ToolbarSlot::NEW_STORAGE)
            .context(&[ObjectKind::Pool, ObjectKind::Host])
            .local(TabId::Storage, 10)
            .local_scopes(&[ObjectKind::Pool, ObjectKind::Host])
            .eligibility(|m, target| Eligibility::enabled_if(new_pool(m, target).is_some(), NO_POOL))
            .invoke(|m, target| {
                if let Some(pool) = new_pool(m, target) {
                    editor::open_new(m, pool);
                }
            }),
        CommandSpec::new(TRIM.0, "Reclaim Freed Space", AREA)
            .menu(MenuPath::new(Menu::Storage, 2, 40))
            .context(REPOSITORY)
            .local(TabId::Storage, 40)
            .eligibility(|m, target| match repository(m, target) {
                Ok((pool, _)) => Eligibility::enabled_if(writable(m, pool), NOT_WRITABLE),
                Err(refusal) => refusal,
            })
            .invoke(|m, target| {
                if let Ok((pool, sr)) = repository(m, target) {
                    editor::open_trim(m, pool, sr);
                }
            }),
        CommandSpec::new(DETACH.0, "Detach", AREA)
            .menu(place::STORAGE_DETACH)
            .context(REPOSITORY)
            .local(TabId::Storage, 50)
            .eligibility(|m, target| attachment_change(m, target, Change::Detach))
            .invoke(|m, target| open_attachment_change(m, target, false)),
        CommandSpec::new(REATTACH.0, "Reattach", AREA)
            .menu(place::STORAGE_REATTACH)
            .context(REPOSITORY)
            .local(TabId::Storage, 60)
            .eligibility(|m, target| attachment_change(m, target, Change::Reattach))
            .invoke(|m, target| open_attachment_change(m, target, false)),
        CommandSpec::new(ATTACHMENT_CREATE.0, "Create Missing Host Attachment", AREA)
            .menu(MenuPath::new(Menu::Storage, 4, 22))
            .context(REPOSITORY)
            .local(TabId::Storage, 62)
            .eligibility(|m, target| match repository(m, target) {
                Ok(_) => Eligibility::Enabled,
                Err(refusal) => refusal,
            })
            .invoke(|m, target| {
                if let Ok((pool, sr)) = repository(m, target) {
                    editor::open_attachment(m, pool, sr);
                }
            }),
        CommandSpec::new(ATTACHMENT_REMOVE.0, "Remove Detached Attachment", AREA)
            .menu(MenuPath::new(Menu::Storage, 4, 24))
            .context(REPOSITORY)
            .local(TabId::Storage, 64)
            .eligibility(|m, target| attachment_change(m, target, Change::Remove))
            .invoke(|m, target| open_attachment_change(m, target, true)),
        CommandSpec::new(FORGET.0, "Forget", AREA)
            .menu(place::STORAGE_FORGET)
            .context(REPOSITORY)
            .local(TabId::Storage, 70)
            .eligibility(removal)
            .invoke(|m, target| {
                if let Ok((pool, sr)) = repository(m, target) {
                    editor::review_removal(m, pool, sr, false);
                }
            }),
        CommandSpec::new(DESTROY.0, "Destroy", AREA)
            .menu(place::STORAGE_DESTROY)
            .context(REPOSITORY)
            .local(TabId::Storage, 80)
            .eligibility(removal)
            .invoke(|m, target| {
                if let Ok((pool, sr)) = repository(m, target) {
                    editor::review_removal(m, pool, sr, true);
                }
            }),
    ]
}

/// The pool New Storage creates in: the pool of the target, else of the
/// tree scope, else the pool looked at last. `None` while that pool has no
/// inventory. Rune showed its button whenever an inventory was present and
/// never disabled it: the wizard opens read-only and in the demo.
fn new_pool(m: &Model, target: &Target) -> Option<PoolId> {
    target
        .pool()
        .or_else(|| m.scope().pool())
        .or_else(|| m.inventory.as_ref().map(|inventory| inventory.pool_id))
        .filter(|pool| m.inventory_for(*pool).is_some())
}

/// The repository a command acts on, or how the command answers a target
/// that is none.
fn repository(m: &Model, target: &Target) -> Result<(PoolId, SrId), Eligibility> {
    let Some(object) = target.single() else {
        return Err(Eligibility::Hidden);
    };
    match object.kind {
        ObjectKind::StorageRepository => object
            .pool
            .zip(object.uuid())
            .filter(|(pool, uuid)| {
                m.inventory_for(*pool)
                    .is_some_and(|i| i.storage.iter().any(|sr| sr.uuid == *uuid))
            })
            .map(|(pool, uuid)| (pool, SrId::from(uuid)))
            .ok_or(Eligibility::Hidden),
        ObjectKind::Pool | ObjectKind::Host => Err(Eligibility::disabled(SELECT)),
        _ => Err(Eligibility::Hidden),
    }
}

/// Rune disabled the buttons that change an attachment, without a reason,
/// unless the connection could write or the pool was the demo.
fn writable(m: &Model, pool: PoolId) -> bool {
    m.is_synthetic_pool(pool) || m.can_mutate_pool(pool)
}

/// What `editor::stage` would refuse a review of `target` with: outside a
/// synthetic pool the target must be observable on the current connection.
fn unreviewable(m: &Model, pool: PoolId, target: ResourceTarget) -> Option<String> {
    let observable = m
        .snapshot_for(pool)
        .is_some_and(|snapshot| snapshot.observe_resource(target).is_ok());
    (!m.is_synthetic_pool(pool) && !observable).then(|| editor::STALE.to_owned())
}

/// Forget and Destroy: Rune offered both always and refused in the review.
fn removal(m: &Model, target: &Target) -> Eligibility {
    match repository(m, target) {
        Ok((pool, sr)) => {
            Eligibility::enabled_unless(unreviewable(m, pool, ResourceTarget::Storage(sr)))
        }
        Err(refusal) => refusal,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Change {
    Detach,
    Reattach,
    Remove,
}

/// The one host attachment of the target repository that Detach, Reattach
/// and Remove act on (see the head of this file).
pub(super) fn attachment(
    m: &Model,
    target: &Target,
) -> Result<(PoolId, StorageAttachment), Eligibility> {
    let (pool, sr) = repository(m, target)?;
    let Some(inventory) = m.inventory_for(pool) else {
        return Err(Eligibility::Hidden);
    };
    let of_repository = || inventory.details.pbds.values().filter(|p| p.sr == Some(sr));
    // The server whose grid the repository was selected in says which.
    let from_grid = target
        .single()
        .is_some_and(|object| m.selection.grid.contains(object));
    let host = m
        .scope()
        .single_of(ObjectKind::Host)
        .filter(|object| from_grid && object.pool == Some(pool))
        .and_then(ObjectRef::host_id);
    let chosen = match host {
        Some(host) => of_repository().find(|p| p.host == Some(host)),
        None if of_repository().count() == 1 => of_repository().next(),
        None if of_repository().count() > 1 => return Err(Eligibility::disabled(SEVERAL)),
        None => None,
    };
    chosen
        .cloned()
        .map(|attachment| (pool, attachment))
        .ok_or_else(|| Eligibility::disabled(NO_ATTACHMENT))
}

fn attachment_change(m: &Model, target: &Target, change: Change) -> Eligibility {
    let (pool, attachment) = match attachment(m, target) {
        Ok(found) => found,
        Err(refusal) => return refusal,
    };
    if !writable(m, pool) {
        return Eligibility::disabled(NOT_WRITABLE);
    }
    let refusal = match (change, attachment.currently_attached) {
        (Change::Detach, Some(true)) | (Change::Reattach | Change::Remove, Some(false)) => None,
        (Change::Detach, Some(false)) => Some(NOT_ATTACHED),
        (Change::Reattach, Some(true)) => Some(ATTACHED),
        (Change::Remove, Some(true)) => Some(ONLY_DETACHED),
        (_, None) => Some(UNKNOWN_STATE),
    };
    if let Some(refusal) = refusal {
        return Eligibility::disabled(refusal);
    }
    Eligibility::enabled_unless(unreviewable(m, pool, ResourceTarget::Pbd(attachment.id)))
}

fn open_attachment_change(m: &mut Model, target: &Target, remove: bool) {
    if let Ok((pool, attachment)) = attachment(m, target) {
        editor::review_attachment(m, pool, attachment.id, remove);
    }
}
