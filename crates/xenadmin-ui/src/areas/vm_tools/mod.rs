//! Area `vm_tools`. Not ported yet.
//!
//! Rune modules: `guest_tools_view.rs`, `external_tools.rs`, `external_tool_command.rs`.
//! Tabs: none.
//! See `notes/briefs/AREAS.md` for the full row and `notes/briefs/MODEL-API.md`
//! for how to fill this in. Everything of the area lives in this directory.

use crate::areas::AreaSpec;

/// External tool declarations and their command expansion (the Rune
/// `external_tool_command.rs`). Already here because `Preferences` stores
/// the catalog.
pub mod command;

/// The state of the area: its editors and workspaces. `m.areas.vm_tools`.
#[derive(Default)]
pub struct State {}

pub fn spec() -> AreaSpec {
    AreaSpec::new("vm_tools")
}
