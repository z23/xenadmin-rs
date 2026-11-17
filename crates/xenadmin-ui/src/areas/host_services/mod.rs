//! Area `host_services`. Not ported yet.
//!
//! Rune modules: `host_power_view.rs`, `host_support_view.rs`.
//! Tabs: none.
//! See `notes/briefs/AREAS.md` for the full row and `notes/briefs/MODEL-API.md`
//! for how to fill this in. Everything of the area lives in this directory.

use crate::areas::AreaSpec;

/// The state of the area: its editors and workspaces. `m.areas.host_services`.
#[derive(Default)]
pub struct State {}

pub fn spec() -> AreaSpec {
    AreaSpec::new("host_services")
}
