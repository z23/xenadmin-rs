//! Area `host_admin`: the changes a server offers, and pool VM autostart.
//!
//! Rune module: `host_admin.rs`. Tabs: none.
//!
//! The Rune module was one dialog, "Host administration", that listed
//! thirteen changes as buttons and then turned into the editor of the one
//! chosen. That list is gone: each change is a command, and what it opens is
//! a dialog with its review or a page of the server's Properties dialog.
//!
//! | Change (Rune title) | Command | Surface | Notice |
//! | --- | --- | --- | --- |
//! | Enter maintenance, Exit maintenance | `server.maintenance` | dialog | |
//! | Evacuate guests | `server.evacuate` | dialog | |
//! | Reboot host | `server.reboot` | dialog | |
//! | Shut down host | `server.shutdown` | dialog | |
//! | Restart toolstack | `server.restart-toolstack` | dialog | |
//! | Power on host | `server.power-on` | dialog | experimental |
//! | Change dom0 memory | `server.control-domain-memory` | dialog | experimental |
//! | Forget dead host | `server.destroy` | dialog | experimental |
//! | Change multipathing | `server.properties` (kernel) | page Multipathing | experimental |
//! | Remote logging | `server.properties` (kernel) | page Log Destination | experimental |
//! | Configure Wake-on-LAN, Disable remote power-on | `server.properties` (kernel) | page Power On | experimental |
//! | Pool VM autostart | `pool.vm-autostart` | dialog | experimental |
//!
//! Every one of them is the same editor underneath, as in Rune: a captured
//! inventory and observation, a draft, a reviewed
//! [`xenadmin_core::HostOperation`], and a submission that is refused when
//! anything it was reviewed against has changed.
//!
//! What of the Rune dialog has no successor here, and where it went:
//!
//! - The chooser itself, its opening button `host-admin-open` and its
//!   closing caption about passwords, backups and support bundles: the
//!   Server menu and a server's context menu list the changes now.
//! - The chooser's three buttons into other modules. "Multipathing with
//!   storage reconnection…" and "BMC / power-on settings…" are on the
//!   Multipathing and Power On pages and run the commands
//!   `storage.multipath-workflow` and `server.power-on-settings` of the
//!   areas that own those workflows. "Monitoring recording…" is the
//!   `performance` area's command `server.monitoring-recording`; no page
//!   here is about monitoring, so nothing here leads to it.
//! - "Pool VM autostart…" in the pool administration history: the command
//!   `pool.vm-autostart` in the Pool menu and the pool's context menu.
//!
//! The files, in the order to read them:
//!
//! - [`change`]: the logic carried over from the Rune module. What a change
//!   is, how its operation is built from the captured inventory and the
//!   draft, the review, the staleness check and the submission.
//! - [`commands`]: the command table, with the eligibility of each command.
//! - [`view`]: the description of the open surface.
//! - [`scenes`]: the two scenes, under their Rune names.
//! - `tests`: the Rune tests, what the port changed, and one test for each
//!   hazard the map lists for the module.
//!
//! The server's Properties dialog is the kernel's ([`crate::properties`]):
//! it collects the pages of every area that has some for a server. This
//! area hands in its three through the hooks `property_pages`,
//! `property_page` and `on_property_page`; a page is the same editor as a
//! dialog, created when the page is entered and dropped when it is left.

use crate::prelude::*;

pub mod change;
pub mod commands;
pub mod scenes;
pub mod view;

#[cfg(test)]
mod tests;

/// The state of the area. `m.areas.host_admin`.
#[derive(Default)]
pub struct State {
    /// The open change. `Some` means its surface is open: opening a surface
    /// is a state change a command makes, not a message to the view.
    pub editor: Option<change::Editor>,
}

pub fn spec() -> AreaSpec {
    AreaSpec {
        commands: commands::commands,
        scenes: scenes::scenes,
        surfaces: view::surfaces,
        on_scope_change: close,
        on_session_change: close,
        property_pages: view::pages,
        property_page: view::page,
        on_property_page: change::page_event,
        ..AreaSpec::new("host_admin")
    }
}

/// Close, Escape, another object selected in the tree, another pool shown:
/// the editor belongs to the one server or pool it was opened for, so
/// nothing of it is kept. An editor that is a page takes the Properties
/// dialog with it: the dialog is closed, never left on a page without its
/// editor.
pub(crate) fn close(m: &mut Model) {
    let page = m
        .areas
        .host_admin
        .editor
        .take()
        .is_some_and(|w| w.kind.page().is_some());
    if page {
        m.close_properties();
    }
}
