//! Area `pool`: the pool's HA tab, its membership, its policies, and the
//! two pool-wide changes that belonged to the connection manager.
//!
//! Rune modules: `pool_view.rs` with its child `ha_policy_view.rs`,
//! `pool_dissolve_view.rs`, `profile_pool_join.rs`,
//! `profile_root_password.rs`. Tabs: HA (of a pool).
//!
//! The Rune "Pool administration" dialog was a hub: every control of the
//! pool in one scrolling column, and buttons into four other dialogs. It is
//! gone. What it held:
//!
//! | Rune (`pool_view::dialog`) | Now | Surface |
//! | --- | --- | --- |
//! | HA captions, heartbeat list, the VM list | the HA tab of the pool ([`view::tab`]) | tab |
//! | heartbeat picker, `pool-ha-enable` | `pool.ha.configure` (Pool > High Availability, HA row) | wizard `pool-ha-configure` |
//! | `pool-ha-disable` | `pool.ha.disable` (HA row) | dialog `pool-change-ha-disable` |
//! | "Review failure tolerance" | `pool.ha.failures-to-tolerate` (HA row) | dialog `pool-change-ha-failures` |
//! | `ha-overcommit` | `pool.ha.overcommit` (HA row) | dialog `pool-change-ha-overcommit` |
//! | `ha-timeout-*` | `pool.ha.timeout` (HA row) | dialog `pool-change-ha-timeout` |
//! | priority buttons, `ha-startup-*` of the chosen VM | `vm.ha.restart-priority` (HA row, the VM chosen in the grid) | dialog `pool-change-vm-protection` |
//! | `pool-handover` | `pool.designate-coordinator` (Server menu, server context) | dialog `pool-change-handover` |
//! | `pool-eject` | `server.remove-from-pool` (Server menu, server context), `pool.remove-server` (Pool menu) | dialog `pool-change-eject` |
//! | `pool-compression` | `pool.properties` (kernel) | page Network Options |
//! | `pool-https`, `pool-tls`, `pool-secret` | `pool.properties` (kernel) | page Security |
//! | `pool-dissolve-open` | `pool.make-standalone` (Pool menu) | dialog `pool-dissolve` |
//! | `pool-join-open` | `server.join-pool` (Pool menu, at Add Server) | wizard `pool-join` |
//! | `pool-root-password` | `pool.change-server-password` (Pool menu), `server.change-password` (Server > Password) | dialog `root-password-dialog` |
//!
//! Every surface carries the experimental notice: Rune showed it for the
//! pool editor and the dissolution; the join and the root password were
//! children of the connection manager (which showed none) and are not in
//! the supported list of `docs/FEATURES.md`.
//!
//! What has no successor here, and where it went:
//!
//! - The hub and its opening button in the pool administration history
//!   ("Pool policies, HA and membership…").
//! - `pool-security-open` ("Authentication, access and certificates…"):
//!   the `pool_security` area's tab and commands; nothing here leads there.
//! - The limit of thirty VM buttons and its caption "Showing 30 matches.
//!   Filter by name or UUID to find another VM.": the HA tab lists every VM
//!   in a grid; the filter stays.
//! - The jump to the pool history after a submission: the surface closes
//!   and the operation is followed in the Events tab.
//!
//! Scenes: `pool-policies` (the HA tab), `pool-policy-review`, `ha-policy`,
//! `pool-dissolve`, `pool-join`, `root-password`.
//!
//! For other areas:
//!
//! - [`commands::pool_of`]: the pool a command of a pool's tab acts on.
//! - [`policy::administration`], [`policy::known`]: the pool's
//!   administration record and the words for a value the host did not give.
//! - [`policy::describe`]: the review lines of every `PoolOperation`
//!   (`pool_security` staged `PoolOperation::Security` through the same
//!   text in Rune).
//! - The page ids `network-options` and `security` of the pool's
//!   Properties dialog are provided here ([`view::pages`]). An area that
//!   has more for one of them (IGMP snooping is planned on Network Options)
//!   cannot register the same id: of two pages with one id the kernel
//!   keeps the first.
//!
//! The files, in the order to read them: [`policy`] (pool operations,
//! Rune `pool_view.rs` and `ha_policy_view.rs`), [`dissolve`], [`join`],
//! [`root_password`] (the carried-over logic of the other three modules),
//! [`commands`], [`view`], [`scenes`], `tests`.

use crate::prelude::*;

pub mod commands;
pub mod dissolve;
pub mod join;
pub mod policy;
pub mod root_password;
pub mod scenes;
pub mod view;

#[cfg(test)]
mod tests;

/// The reason a synthetic pool gives a disabled confirm button (the text
/// every area uses).
pub const DEMO: &str = "Demo mode cannot change infrastructure.";

/// The reason for what Rune disabled without saying so on a connection
/// that cannot change anything (the words of `host_admin`).
pub const NOT_WRITABLE: &str = "This connection is read-only, disconnected or stale.";

/// The reason for what Rune disabled while HA was not known to be off; the
/// words are those of the reviews.
pub const HA_FIRST: &str = "HA must already be disabled.";

/// The state of the area. `m.areas.pool`.
#[derive(Default)]
pub struct State {
    /// The open pool operation: a dialog, the Configure HA wizard, or the
    /// current page of the pool's Properties dialog.
    pub editor: Option<policy::Editor>,
    /// Make into Standalone Server.
    pub dissolve: Option<dissolve::Editor>,
    /// Add Server to Pool.
    pub join: Option<join::Editor>,
    /// Change Server Password.
    pub root_password: Option<root_password::Editor>,
    /// The HA tab: the filter of its VM grid and the chosen row.
    pub filter: String,
    pub selected_vm: Option<String>,
}

pub fn spec() -> AreaSpec {
    AreaSpec {
        commands: commands::commands,
        scenes: scenes::scenes,
        surfaces: view::surfaces,
        tab: view::tab,
        poll,
        on_scope_change: leave,
        on_session_change: session_change,
        property_pages: view::pages,
        property_page: view::page,
        on_property_page: page_event,
        ..AreaSpec::new("pool")
    }
}

/// Closes the open pool operation. One that is a page takes the Properties
/// dialog with it: the dialog is never left on a page without its editor.
pub(crate) fn close(m: &mut Model) {
    let page = m
        .areas
        .pool
        .editor
        .take()
        .is_some_and(|e| e.kind.page().is_some());
    if page {
        m.close_properties();
    }
}

/// One surface of the area at a time: opening one closes the others, as
/// each Rune `open` cleared the other editors.
pub(crate) fn close_all(m: &mut Model) {
    close(m);
    m.areas.pool.dissolve = None;
    m.areas.pool.join = None;
    m.areas.pool.root_password = None;
}

/// Another object selected in the tree: the pool operation, the
/// dissolution and the HA tab's filter and row belonged to the previous
/// pool. The join and the password change stay: they belong to saved
/// connections, and the password change shows its result after the
/// submission.
fn leave(m: &mut Model) {
    close(m);
    m.areas.pool.dissolve = None;
    m.areas.pool.filter.clear();
    m.areas.pool.selected_vm = None;
}

/// The scope session goes: also the join and the password change when the
/// connection they captured is that one.
fn session_change(m: &mut Model) {
    leave(m);
    let Some(going) = m.workspace.selected else {
        return;
    };
    let joins = m.areas.pool.join.as_ref().is_some_and(|e| {
        e.source.profile.id == going || join::chosen(e).is_some_and(|d| d.profile.id == going)
    });
    if joins {
        m.areas.pool.join = None;
    }
    let password = m.areas.pool.root_password.as_ref();
    if password.is_some_and(|e| e.profile.id == going) {
        m.areas.pool.root_password = None;
    }
}

/// The polls the Rune session refresh ran for the two child editors.
fn poll(m: &mut Model, _clock: f64) -> bool {
    let join = join::poll(m);
    let password = root_password::poll(m);
    join || password
}

/// A page of the pool's Properties dialog is the same editor as a dialog,
/// created when the page is entered and dropped when it is left.
fn page_event(m: &mut Model, target: &Target, event: PageEvent) {
    match event {
        PageEvent::Entered(id) => {
            let Some(kind) = policy::Kind::of_page(id) else {
                return;
            };
            let pool = target.single_of(ObjectKind::Pool).and_then(|o| o.pool);
            m.areas.pool.editor = pool.and_then(|pool| policy::Editor::capture(m, pool, kind));
        }
        PageEvent::Left(id) => {
            let editor = m.areas.pool.editor.as_ref();
            if editor.is_some_and(|e| e.kind.page() == Some(id)) {
                m.areas.pool.editor = None;
            }
        }
        PageEvent::Closed => {}
    }
}
