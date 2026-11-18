//! Area `pool_security`: the Users tab of a pool (session permissions,
//! Active Directory, directory users and groups with their roles) and the
//! changes that start from it, and the two changes of a server's TLS
//! certificate.
//!
//! Rune modules: `pool_security_view.rs`, and the display of the session's
//! permissions of `session_permissions_view.rs` (its `assess`, `check` and
//! footer are the kernel's `permissions.rs`). Tabs: Users (of a pool).
//!
//! The Rune "Pool security" dialog was one scrolling column: status, three
//! forms and their review buttons. It is gone. What it held:
//!
//! | Rune (`pool_security_view::dialog`) | Now | Surface |
//! | --- | --- | --- |
//! | superuser caption, section errors, directory state of the members, RBAC caption | the Users tab ([`view::tab`]) | tab |
//! | the subject buttons (✓ / ○) and their filter | the grid `security-subjects` of the tab, filter `security-subject-filter` | tab |
//! | "Refresh security status" | `pool.security.refresh` (Users row) | none |
//! | "Reconnect as local administrator…" | button `security-reconnect` of the tab, which hands over to `pool.reconnect-as` | (connections) |
//! | domain, account, password, "Review joining directory" | `pool.directory.join` (Users row) | dialog `pool-security-join` |
//! | the two "Review leaving …" buttons | `pool.directory.leave` (Users row) | dialog `pool-security-leave` |
//! | "Resolve directory identity", "Review adding access" | `pool.subject.add` (Users row) | dialog `pool-security-subject-add` |
//! | "Review removing subject access" | `pool.subject.remove` (Users row, row menu) | dialog `pool-security-subject-remove`, opens on its review |
//! | "Grant {role}" / "Remove {role}" | `pool.subject.role` (Users row, row menu) | dialog `pool-security-subject-role` |
//! | host buttons, file buttons, "Validate selected PEM files", "Review installing certificate" | `server.certificate.install` (Server menu) | dialog `pool-security-certificate-install` |
//! | "Review resetting to self-signed" | `server.certificate.reset` (Server menu) | dialog `pool-security-certificate-reset` |
//! | the review with the typed pool UUID, `security-confirm` | the review step of each dialog | the same dialog |
//!
//! Every dialog carries the experimental notice (Rune:
//! `experimental_dialog_open` names `pool_security`). The tab is not a
//! surface and has none, as the HA tab.
//!
//! Session permissions (`session_permissions_view.rs`): the group "Session
//! permissions" of the tab says what the session of this pool is (the
//! core's summary and the standard roles it holds), or, while that is
//! unknown, why, under the Rune key `session-permission-explanation`. The
//! permission line of a review is the kernel's; the reviews of this area
//! never had one (their gate is the security overview: local superuser, the
//! methods the pool offers, RBAC).
//!
//! What has no successor, and where it went:
//!
//! - `pool-security-open` of the pool editor: the tab is the way in.
//! - The limit of thirty subject buttons and "Showing up to 30 matches.
//!   Search to narrow the list.": the grid lists every subject.
//! - The list of up to 256 host buttons: the certificate commands act on the
//!   server that is their target.
//! - The jump to the pool history after a submission: the dialog closes, the
//!   operation is followed in the Events tab, and the tab reads the overview
//!   again when it has ended.
//! - "close and reopen": Refresh captures the pool again. The Rune texts
//!   that said "reopen security" name this dialog or the Users tab now
//!   (AREAS decision 10).
//!
//! Scenes: `pool-security` (the Users tab), `pool-security-review` (the
//! review of joining a directory).
//!
//! For other areas:
//!
//! - [`security::open`]: reads the security state of a pool into this
//!   area's editor (what Rune `pool_security_view::open` did).
//! - [`security::review_lines`]: the review text of a
//!   `PoolSecurityOperation`.
//! - [`close`]: closes the open dialog. The whole editor goes on a session
//!   change and when the tab hands over to `pool.reconnect-as` (AREAS
//!   decision 11).
//!
//! The files, in the order to read them: [`security`] (the carried-over
//! logic), [`commands`], [`view`], [`scenes`], `tests`.

use xenadmin_core::PoolId;

use crate::prelude::*;

pub mod commands;
pub mod scenes;
pub mod security;
pub mod view;

#[cfg(test)]
mod tests;

/// The reason a synthetic pool gives a disabled confirm button (the text
/// every area uses).
pub const DEMO: &str = "Demo mode cannot change infrastructure.";

/// The state of the area. `m.areas.pool_security`.
#[derive(Default)]
pub struct State {
    /// The security state of one pool: what its Users tab shows, and the
    /// change that is open as a dialog (`Editor::dialog`).
    pub editor: Option<security::Editor>,
}

pub fn spec() -> AreaSpec {
    AreaSpec {
        commands: commands::commands,
        scenes: scenes::scenes,
        surfaces: view::surfaces,
        tab: view::tab,
        poll,
        on_scope_change: scope_changed,
        on_tab_change: tab_changed,
        on_session_change: session_changed,
        ..AreaSpec::new("pool_security")
    }
}

/// The pool whose Users tab is showing.
pub(crate) fn shown(m: &Model) -> Option<PoolId> {
    if m.active_tab() != TabId::Users {
        return None;
    }
    m.scope().single_of(ObjectKind::Pool)?.pool
}

/// Keeps the editor with what is shown: it is opened for the pool whose
/// Users tab shows (when that pool can be asked, or is synthetic) and
/// dropped when neither that tab nor a dialog needs it. Returns whether an
/// editor was opened.
pub(crate) fn sync(m: &mut Model) -> bool {
    let wanted = shown(m);
    let state = &mut m.areas.pool_security;
    if let Some(e) = &state.editor {
        if e.dialog.is_some() || Some(e.pool) == wanted {
            return false;
        }
        state.editor = None;
    }
    match wanted {
        Some(pool) if m.is_synthetic_pool(pool) || security::refusal(m, pool).is_none() => {
            security::open(m, pool)
        }
        _ => false,
    }
}

fn poll(m: &mut Model, _clock: f64) -> bool {
    let opened = sync(m);
    security::poll(m) || opened
}

/// Close, Escape, a submission: the dialog goes, the tab keeps its state.
pub(crate) fn close(m: &mut Model) {
    if let Some(e) = &mut m.areas.pool_security.editor {
        security::end(e);
    }
    sync(m);
}

/// Another object in the tree: nothing of the previous one is kept.
fn scope_changed(m: &mut Model) {
    m.areas.pool_security.editor = None;
    sync(m);
}

fn tab_changed(m: &mut Model, _from: TabId, _to: TabId) {
    sync(m);
}

/// The session of the scope changes (reconnect, Reconnect As, disconnect):
/// the editor belongs to the old one (AREAS decision 11). `poll` opens it
/// again for the new session while the tab still shows.
fn session_changed(m: &mut Model) {
    m.areas.pool_security.editor = None;
}
