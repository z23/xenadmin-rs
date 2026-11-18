//! The two commands of the area.
//!
//! `storage.repair` is the plan's Storage > Repair. Its eligibility is the
//! rule of the storage area's commands on a repository:
//!
//! - `Hidden` when the target is not one object, or is of a kind that has
//!   nothing to do with repositories;
//! - `Disabled("Select a storage repository.")` when the target is the pool
//!   or the server whose Storage tab lists the repositories and no row is
//!   selected: the command is in the local row of that grid;
//! - `Enabled` for a repository its pool's inventory has. Rune showed the
//!   repair controls for every repository and never disabled its way in:
//!   the dialog says what there is to repair, and its review button what is
//!   missing.
//!
//! `storage.multipath-workflow` has no place in a menu: it is the way from
//! the Multipathing page of a server's Properties dialog (area `host_admin`,
//! Rune key `host-multipath-workflow`) into the storage workflow. It is
//! `Hidden` for everything but one known server and `Enabled` for that: the
//! Rune button was never disabled, and the dialog it opens is also where the
//! results of earlier runs and their Restore controls are, which must stay
//! reachable when the server is not in the state a new run needs. The dialog
//! says why a run cannot be reviewed.

use xenadmin_core::{HostId, PoolId, ResourceTarget, SrId};

use super::{graph, maintenance};
use crate::prelude::*;

const AREA: &str = "storage_maintenance";

pub const REPAIR: CommandId = CommandId("storage.repair");
pub const MULTIPATH_WORKFLOW: CommandId = CommandId("storage.multipath-workflow");

/// The reason of the storage area's commands for a grid without a selected
/// row; a test holds the two texts together.
pub const SELECT: &str = "Select a storage repository.";

pub fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new(REPAIR.0, "Repair", AREA)
            .menu(place::STORAGE_REPAIR)
            .context(&[ObjectKind::StorageRepository])
            // The order the storage area left free for it.
            .local(TabId::Storage, 20)
            .eligibility(|m, target| match repository(m, target) {
                Ok(_) => Eligibility::Enabled,
                Err(refusal) => refusal,
            })
            .invoke(|m, target| {
                if let Ok((pool, sr)) = repository(m, target) {
                    maintenance::open_repair(m, pool, sr);
                }
            }),
        CommandSpec::new(
            MULTIPATH_WORKFLOW.0,
            "Multipathing with Storage Reconnection",
            AREA,
        )
        .eligibility(|m, target| match server(m, target) {
            Some(_) => Eligibility::Enabled,
            None => Eligibility::Hidden,
        })
        .invoke(|m, target| {
            if let Some((pool, host)) = server(m, target) {
                graph::open(m, pool, ResourceTarget::Host(host));
            }
        }),
    ]
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

/// The one server of the target, when the inventory of its pool has it.
fn server(m: &Model, target: &Target) -> Option<(PoolId, HostId)> {
    let object = target.single_of(ObjectKind::Host)?;
    let pool = object.pool?;
    let host = object.host_id()?;
    m.inventory_for(pool)?
        .hosts
        .contains_key(&host)
        .then_some((pool, host))
}
