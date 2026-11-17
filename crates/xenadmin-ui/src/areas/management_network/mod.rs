//! Area `management_network`. Not ported yet.
//!
//! Rune modules: `management_network_ui.rs`, `management_bond_ui.rs`, `management_ipv6_bond_ui.rs`.
//! Tabs: none.
//! See `notes/briefs/AREAS.md` for the full row and `notes/briefs/MODEL-API.md`
//! for how to fill this in. Everything of the area lives in this directory.

use crate::areas::AreaSpec;

/// The state of the area: its editors and workspaces. `m.areas.management_network`.
#[derive(Default)]
pub struct State {}

pub fn spec() -> AreaSpec {
    AreaSpec::new("management_network")
}
