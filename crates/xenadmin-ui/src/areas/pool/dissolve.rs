//! Make into Standalone Server: the logic of the Rune
//! `pool_dissolve_view.rs`, carried over. One review of a captured
//! [`PoolDissolve`] (the ordered ejection of every member, then clearing the
//! pool's name and description) with the pool's UUID as confirmation.

use xenadmin_core::{
    Inventory, ObservedResource, OperationId, PoolDissolve, PoolId, ResourceCommand, ResourceTarget,
};

use crate::prelude::*;

/// The refusal of a review whose connection moved on, in the function.
pub const STALE: &str = "Connection changed; reopen pool dissolution";

/// The same, as the review says it.
pub const NOT_CURRENT: &str =
    "Connection changed or is read-only; reopen this review before applying.";

/// The refusal while members are enabled or the UUID is not typed.
pub const UNCONFIRMED: &str =
    "Disable and evacuate the reviewed members first, then enter the exact pool UUID";

pub struct Editor {
    pub pool: PoolId,
    pub observed: Option<ObservedResource>,
    pub operation: PoolDissolve,
    /// Counted once, when the review opened: after disabling members the
    /// review is closed and opened again.
    pub enabled_members: usize,
    pub confirmation: String,
    pub error: Option<String>,
}

/// What the review would be for an inventory, or why there is none: the
/// text Rune put into the shared error line.
pub fn capture(inventory: &Inventory) -> Result<PoolDissolve, String> {
    PoolDissolve::capture(inventory).map_err(|e| {
        format!(
            "Cannot review dissolution: {e}. The pool must be named, HA must be disabled, and membership must be available."
        )
    })
}

pub fn open(m: &mut Model, pool: PoolId) {
    let Some(inventory) = m.inventory_for(pool) else {
        return;
    };
    let operation = match capture(&inventory) {
        Ok(op) => op,
        Err(error) => {
            m.error = Some(error);
            return;
        }
    };
    let enabled_members = operation
        .members
        .iter()
        .filter(|m| inventory.hosts.get(&m.host).is_some_and(|h| h.enabled))
        .count();
    let observed = m
        .snapshot_for(pool)
        .and_then(|v| v.observe_resource(ResourceTarget::Pool(pool)).ok());
    super::close_all(m);
    m.areas.pool.dissolve = Some(Editor {
        pool,
        observed,
        operation,
        enabled_members,
        confirmation: String::new(),
        error: None,
    });
}

pub fn synthetic(m: &Model, e: &Editor) -> bool {
    m.is_synthetic_pool(e.pool)
}

pub fn current(m: &Model, e: &Editor) -> bool {
    !synthetic(m, e)
        && m.can_mutate_pool(e.pool)
        && e.observed.is_some()
        && m.snapshot_for(e.pool)
            .and_then(|v| v.observe_resource(ResourceTarget::Pool(e.pool)).ok())
            == e.observed
}

pub fn apply(m: &mut Model) {
    let result = (|| {
        let e = m.areas.pool.dissolve.as_ref().ok_or("Pool review closed")?;
        if !current(m, e) {
            return Err(STALE.to_owned());
        }
        if e.confirmation != e.pool.to_string() || e.enabled_members > 0 {
            return Err(UNCONFIRMED.into());
        }
        let permission =
            permissions::assess_for(m, e.pool, &e.operation.permission_requirements());
        if permission.decision().is_denied() {
            return Err(permission.explanation());
        }
        m.connection_for(e.pool)
            .ok_or("Connect first")?
            .submit_resource(
                OperationId::new(),
                ResourceCommand::DissolvePool {
                    pool: e.observed.ok_or(STALE)?,
                    operation: e.operation.clone(),
                },
            )
            .map_err(|e| e.to_string())
    })();
    match result {
        Ok(op) => {
            m.track_resource(op);
            m.areas.pool.dissolve = None;
        }
        Err(error) => {
            if let Some(e) = &mut m.areas.pool.dissolve {
                e.error = Some(error);
            }
        }
    }
}

/// The lines of the review, in the Rune order.
pub fn lines(e: &Editor) -> Vec<String> {
    let mut lines = vec![
        format!(
            "{}\nPool {}\nRemaining coordinator {}",
            e.operation.expected_name, e.pool, e.operation.coordinator
        ),
        if e.operation.members.is_empty() {
            "This pool already has one host. Clear its pool name and description; keep the host, VMs and disks."
        } else {
            "Eject exactly these members in order, then clear the pool name and description on the remaining coordinator. Each ejected host reboots and its pool configuration is reset."
        }
        .to_owned(),
        "Every member must already be disabled, evacuated and free of local guest-disk data. The client checks all members before the first ejection and checks remaining membership before every step. HA must stay disabled.".to_owned(),
    ];
    for (n, host) in e.operation.members.iter().enumerate() {
        lines.push(format!("{}. {}\n{}", n + 1, host.name, host.host));
    }
    lines.push("Stop lets an already submitted ejection finish and skips later steps. Interrupted work is never resumed automatically. Ejected-host reboot and independent connectivity require separate verification.".to_owned());
    lines
}

/// The line about members that were still enabled when the review opened.
pub fn enabled_warning(e: &Editor) -> Option<String> {
    (e.enabled_members > 0).then(|| {
        format!(
            "{} reviewed members are still enabled. Close this review, disable and evacuate them, then review again.",
            e.enabled_members
        )
    })
}
