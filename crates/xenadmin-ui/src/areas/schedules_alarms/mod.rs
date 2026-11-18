//! Area `schedules_alarms`: snapshot schedules, performance alarms and the
//! pool's alarm email (plan W6).
//!
//! Rune modules: `snapshot_policy_view.rs`, `alarms_view.rs`. Tabs: none.
//! Everything of the area carries the experimental notice, as both Rune
//! dialogs did.
//!
//! ## Snapshot schedules
//!
//! The Rune dialog "Snapshot schedules" was a list, a form and the review of
//! six operations in one card. The plan's destination is "Dialog with a list
//! and a New Policy wizard":
//!
//! | Rune | Here |
//! | --- | --- |
//! | button `snapshot-schedules-open` of the top row | command `pool.snapshot-schedules`, "VM Snapshot Schedule Policies": Pool menu, a pool's context menu |
//! | (none: the plan's VM menu item) | command `vm.assign-snapshot-schedule`, "Assign to Snapshot Schedule": VM menu, a VM's context menu. The same dialog, that VM first among the assignments and its schedule selected |
//! | list, detail, action buttons | dialog `snapshot-schedules` (Large) |
//! | review of enable, disable, run now, delete, assign, remove | the same dialog, in place, with Back and "Confirm change" |
//! | form of "New schedule" and "Edit settings" | wizard `snapshot-schedule-wizard`: Name, Schedule, Snapshot Type, Review |
//!
//! The actions of a schedule (rows 27 to 31 of the proposed command table)
//! are buttons of the dialog, not commands: a schedule is no object of the
//! tree, so no menu could offer them.
//!
//! ## Alarms
//!
//! The Rune dialog "Performance alarms and email" was a hub of three tabs,
//! the first with a picker of every resource of the pool. It is gone:
//!
//! | Rune | Here |
//! | --- | --- |
//! | tab "Thresholds" and its picker | page `alerts` of the kernel's Properties dialog of a VM, a server and a repository, offered when the object has an alarm policy in the inventory. A server shows its own alarms and the memory alarm of its control domain together |
//! | tab "Pool email" | page `email-options` of the pool's Properties dialog, offered when the inventory has the pool's alarm email configuration |
//! | tab "Refresh service" | command `tools.alarm-service-refresh`, "Refresh Alarm Service": Tools menu, a pool's context menu, and the button `alarm-refresh-service` of the Alerts page. Dialog `alarm-service-refresh`, which opens on its review |
//!
//! ## What has no successor
//!
//! - The buttons `snapshot-schedules-open` and `alarms-open` of the top row:
//!   the commands and the pages above.
//! - The resource picker of the Thresholds tab (`alarm-resource-<index>`,
//!   its filter, "Showing the first 100 matches; refine the search"): the
//!   object is the one whose Properties are open. With it the second "Back"
//!   that returned to the picker.
//! - The three tab buttons of the alarms dialog.
//! - "Operation history" (`policy-history`) no longer opens the Activity
//!   dialog: it closes the dialog and shows the Events tab of the
//!   Notifications view. The label stays: it says what the button shows.
//!
//! ## What other areas may use
//!
//! - [`open_schedules`]: the snapshot schedules of a pool, optionally for a
//!   VM. The commands are [`commands::SNAPSHOT_SCHEDULES`],
//!   [`commands::ASSIGN_SNAPSHOT_SCHEDULE`] and
//!   [`commands::ALARM_SERVICE_REFRESH`].
//! - [`alarms::ALERTS`], [`alarms::EMAIL_OPTIONS`]: the ids of the two
//!   pages, for `m.open_properties(&target, Some(id))`.
//! - [`close`]: closes whatever of the area is open.
//!
//! ## Scenes
//!
//! `snapshot-schedules` (the wizard of a new schedule, first step),
//! `snapshot-schedule-review` (its review), `alarms` (the Alerts page of a
//! server), `alarms-review` (its review), `alarms-email` (the Email Options
//! page of the pool).
//!
//! ## Files
//!
//! - [`schedules`], [`alarms`]: the logic carried over from the two Rune
//!   modules; each says at its top what the port changed.
//! - [`schedules_view`], [`alarms_view`]: the descriptions, with the kept
//!   and the new keys listed at their tops.
//! - [`commands`], [`scenes`], `tests`.

use xenadmin_core::{PoolId, VmId};

use crate::prelude::*;

pub mod alarms;
pub mod alarms_view;
pub mod commands;
pub mod scenes;
pub mod schedules;
pub mod schedules_view;

#[cfg(test)]
mod tests;

/// The state of the area. `m.areas.schedules_alarms`. At most one of the
/// two is open.
#[derive(Default)]
pub struct State {
    /// The open snapshot schedules: the dialog, or the wizard of one.
    pub schedules: Option<schedules::Workspace>,
    /// The open alarm editor: a page of a Properties dialog, or the dialog
    /// of the service refresh.
    pub alarms: Option<alarms::Workspace>,
}

pub fn spec() -> AreaSpec {
    AreaSpec {
        commands: commands::commands,
        scenes: scenes::scenes,
        surfaces,
        poll,
        on_scope_change: close,
        on_session_change: close,
        property_pages: alarms_view::pages,
        property_page: alarms_view::page,
        on_property_page: alarms::page_event,
        ..AreaSpec::new("schedules_alarms")
    }
}

fn surfaces(m: &Model) -> Vec<Surface> {
    schedules_view::surface(m)
        .into_iter()
        .chain(alarms_view::surface(m))
        .collect()
}

/// Only the schedules wait for something: the answer of the coordinator.
fn poll(m: &mut Model, _clock: f64) -> bool {
    schedules::poll(m)
}

/// Opens the snapshot schedules of a pool. With a VM: for "Assign to
/// Snapshot Schedule" of that VM. Whatever else of the area was open closes.
pub fn open_schedules(m: &mut Model, pool: PoolId, vm: Option<VmId>) {
    close_alarms(m);
    schedules::open(m, pool, vm);
}

/// Close and Escape of the schedules, and "Operation history".
pub fn close_schedules(m: &mut Model) {
    m.areas.schedules_alarms.schedules = None;
}

/// Close of an alarm editor. One that is a page takes the Properties dialog
/// with it: the dialog is never left on a page without its editor.
pub fn close_alarms(m: &mut Model) {
    let page = m
        .areas
        .schedules_alarms
        .alarms
        .take()
        .is_some_and(|w| w.page.id().is_some());
    if page {
        m.close_properties();
    }
}

/// Another object selected in the tree, another pool shown, a disconnect:
/// nothing of the area is kept. (Rune kept both workspaces over a profile
/// switch and relied on their reviews being refused as stale.)
pub fn close(m: &mut Model) {
    close_schedules(m);
    close_alarms(m);
}
