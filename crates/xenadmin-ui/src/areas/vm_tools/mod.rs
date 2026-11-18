//! Area `vm_tools`: guest tools media and external tools (plan W1).
//!
//! Rune modules: `guest_tools_view.rs`, `external_tools.rs`,
//! `external_tools_tests.rs`, `external_tool_command.rs`. Tabs: none.
//!
//! | Rune surface | Command | Surface | Notice |
//! | --- | --- | --- | --- |
//! | "Guest tools media" (`tools-dialog`) | `vm.install-guest-tools` "Install Guest Tools": VM menu, VM context menu, local row of a VM's Storage tab | dialog `tools-dialog`: choose, review, the operation in place | experimental |
//! | "Guest SSH, RDP and local tools" (launch panel) | `vm.connect-external` "Connect with SSH, RDP or Local Tool…": VM context menu, local row of a VM's Console tab | dialog `guest-connections` | experimental |
//! | "Configure external tools" (declarations panel) | `tools.external-tools` "External Tools": Tools menu; "Configure external tools…" in Options; "Configure local tools…" in the launch dialog | dialog `external-tools` | experimental |
//!
//! Scenes (Rune names): `guest-tools`, `guest-tools-review`,
//! `guest-connections`, `external-tools`.
//!
//! What other areas may use:
//!
//! - the command ids [`commands::INSTALL_GUEST_TOOLS`],
//!   [`commands::CONNECT_EXTERNAL`], [`commands::EXTERNAL_TOOLS`] (through
//!   `m.invoke`; `settings` already has a button for the last);
//! - [`external::addresses`] and [`external::ordinary`]: the usable
//!   addresses a guest reports, and whether a VM is a running guest;
//! - [`command`]: the tool declarations the preferences store;
//! - [`scenes::prepare_media`], [`scenes::prepare_guest`]: the fixtures of
//!   the scenes.
//!
//! What of the Rune modules has no successor here, and where it went:
//!
//! - The opening buttons `open-guest-tools` (details pane and devices page)
//!   and `guest-tools` (VM details): the three commands above.
//! - `console_view::release` before opening either dialog: the console
//!   canvas reports that it lost the keyboard (see the `console` area).
//! - The failure of "Installation guide" to open, which Rune put on the
//!   dialog's error line: opening a URL is an intent the front end performs
//!   and its failure does not come back to the model.
//! - `external_tools::clear` (from `demo_sized`) emptied the job log: jobs
//!   and their log are local processes and are kept; a session or scope
//!   change closes the dialogs that belong to an object (see [`leave`]).
//! - `application_panel = None` in `configure`: the Options dialog of the
//!   `settings` area closes itself before it invokes the command.
//! - Rune's own copy of the tracked submit in `guest_tools_view::apply`:
//!   the model's `submit_tracked_command_for`.
//! - A guest tools dialog that survived a profile switch showing a dead
//!   confirm button: the dialog closes on a session or scope change.
//! - Fixed heights of the two chooser lists (95 and 140 px) and the fixed
//!   footer: layout of the generic renderer.
//!
//! The files, in the order to read them:
//!
//! - [`command`]: the Rune `external_tool_command.rs`, unchanged.
//! - [`media`]: guest tools media, logic and description.
//! - [`external`]: external tools, logic and description.
//! - [`commands`]: the three commands and their eligibility.
//! - [`scenes`]: the four scenes.
//! - `tests`: the Rune tests, what the port changed, and the hazards.

use crate::prelude::*;

/// External tool declarations and their command expansion (the Rune
/// `external_tool_command.rs`). `Preferences` stores the catalog.
pub mod command;
pub mod commands;
pub mod external;
pub mod media;
pub mod scenes;

#[cfg(test)]
mod tests;

/// The state of the area. `m.areas.vm_tools`.
#[derive(Default)]
pub struct State {
    /// The open guest tools media dialog.
    pub media: Option<media::Panel>,
    /// The external tools workspace: its open dialog, the launched jobs and
    /// their log.
    pub external: external::Workspace,
}

pub fn spec() -> AreaSpec {
    AreaSpec {
        commands: commands::commands,
        scenes: scenes::scenes,
        surfaces,
        poll: external::poll,
        on_scope_change: leave,
        on_session_change: leave,
        ..AreaSpec::new("vm_tools")
    }
}

/// Back to front: the guest tools dialog, then the external tools dialog
/// (which Options and a context menu can open while the other is not the
/// front surface).
fn surfaces(m: &Model) -> Vec<Surface> {
    media::surface(m)
        .into_iter()
        .chain(external::surface(m))
        .collect()
}

/// Another object is the scope, or another session is shown: the dialogs
/// that captured an object close. A submitted mount goes on and is followed
/// in the Events tab; the declarations dialog, which is about no object,
/// stays.
pub(crate) fn leave(m: &mut Model) {
    media::close(m);
    external::leave(m);
}
