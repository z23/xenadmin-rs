//! Area `pool`. Not ported yet.
//!
//! Rune modules: `pool_view.rs`, `ha_policy_view.rs`, `pool_dissolve_view.rs`, `profile_pool_join.rs`, `profile_root_password.rs`.
//! Tabs: HA.
//! See `notes/briefs/AREAS.md` for the full row and `notes/briefs/MODEL-API.md`
//! for how to fill this in. Everything of the area lives in this directory.

use crate::areas::AreaSpec;

/// The state of the area: its editors and workspaces. `m.areas.pool`.
#[derive(Default)]
pub struct State {}

pub fn spec() -> AreaSpec {
    AreaSpec::new("pool")
}
