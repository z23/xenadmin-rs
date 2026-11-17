//! Area `network`. Not ported yet.
//!
//! Rune modules: `network_admin.rs`, `network_interfaces.rs`, the generic editor of `resource_admin.rs`.
//! Tabs: Networking of a host, pool or network; NICs.
//! See `notes/briefs/AREAS.md` for the full row and `notes/briefs/MODEL-API.md`
//! for how to fill this in. Everything of the area lives in this directory.

use crate::areas::AreaSpec;

/// The state of the area: its editors and workspaces. `m.areas.network`.
#[derive(Default)]
pub struct State {}

pub fn spec() -> AreaSpec {
    AreaSpec::new("network")
}
