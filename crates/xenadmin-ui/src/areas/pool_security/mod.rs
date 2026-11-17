//! Area `pool_security`. Not ported yet.
//!
//! Rune modules: `pool_security_view.rs`, the session permission display of `session_permissions_view.rs`.
//! Tabs: Users.
//! See `notes/briefs/AREAS.md` for the full row and `notes/briefs/MODEL-API.md`
//! for how to fill this in. Everything of the area lives in this directory.

use crate::areas::AreaSpec;

/// The state of the area: its editors and workspaces. `m.areas.pool_security`.
#[derive(Default)]
pub struct State {}

pub fn spec() -> AreaSpec {
    AreaSpec::new("pool_security")
}
