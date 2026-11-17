//! Area `search`. Not ported yet.
//!
//! Rune modules: the inventory table of `main.rs`, `search_view.rs`, `search_files.rs`, `inventory_report.rs`.
//! Tabs: Search.
//! See `notes/briefs/AREAS.md` for the full row and `notes/briefs/MODEL-API.md`
//! for how to fill this in. Everything of the area lives in this directory.

use crate::areas::AreaSpec;

/// The state of the area: its editors and workspaces. `m.areas.search`.
#[derive(Default)]
pub struct State {}

pub fn spec() -> AreaSpec {
    AreaSpec::new("search")
}
