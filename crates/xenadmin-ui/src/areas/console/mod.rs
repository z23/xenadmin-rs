//! Area `console`. Not ported yet.
//!
//! Rune modules: `console_view.rs`, `console_recovery.rs`, `console_export.rs`, `host_console.rs`, `console_windows.rs`.
//! Tabs: Console.
//! See `notes/briefs/AREAS.md` for the full row and `notes/briefs/MODEL-API.md`
//! for how to fill this in. Everything of the area lives in this directory.

use crate::areas::AreaSpec;

/// The state of the area: its editors and workspaces. `m.areas.console`.
#[derive(Default)]
pub struct State {}

pub fn spec() -> AreaSpec {
    AreaSpec::new("console")
}
