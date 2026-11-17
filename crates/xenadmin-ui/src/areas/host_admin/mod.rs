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
//! | Change multipathing | `server.properties` | page Multipathing | experimental |
//! | Remote logging | `server.properties` | page Log Destination | experimental |
//! | Configure Wake-on-LAN, Disable remote power-on | `server.properties` | page Power On | experimental |
//! | Pool VM autostart | `pool.vm-autostart` | dialog | experimental |
//!
//! Every one of them is the same editor underneath, as in Rune: a captured
//! inventory and observation, a draft, a reviewed
//! [`xenadmin_core::HostOperation`], and a submission that is refused when
//! anything it was reviewed against has changed.
//!
//! The files, in the order to read them:
//!
//! - [`change`]: the logic carried over from the Rune module. What a change
//!   is, how its operation is built from the captured inventory and the
//!   draft, the review, the staleness check and the submission.
//! - [`commands`]: the command table, with the eligibility of each command.
//! - [`view`]: the description of the open surface.
//! - [`scenes`]: the two scenes, under their Rune names.
//! - `tests`: the Rune tests, and tests of what the port added.

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
        ..AreaSpec::new("host_admin")
    }
}

/// Close, Escape, another object selected in the tree, another pool shown:
/// the editor belongs to the one server or pool it was opened for, so
/// nothing of it is kept.
pub(crate) fn close(m: &mut Model) {
    m.areas.host_admin.editor = None;
}
