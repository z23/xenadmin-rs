//! Area `organization`. Not ported yet.
//!
//! Rune modules: `organization_view.rs`, `appliance_view.rs`.
//! Tabs: none (the Organization navigation view).
//! See `notes/briefs/AREAS.md` for the full row and `notes/briefs/MODEL-API.md`
//! for how to fill this in. Everything of the area lives in this directory.

use crate::areas::AreaSpec;

/// The state of the area: its editors and workspaces. `m.areas.organization`.
#[derive(Default)]
pub struct State {}

pub fn spec() -> AreaSpec {
    AreaSpec::new("organization")
}
