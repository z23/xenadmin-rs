//! Area `cross_pool`. Not ported yet.
//!
//! Rune modules: `cross_pool_vm.rs`, `cross_pool_vm_view.rs`, `bulk_cross_pool.rs`.
//! Tabs: none.
//! See `notes/briefs/AREAS.md` for the full row and `notes/briefs/MODEL-API.md`
//! for how to fill this in. Everything of the area lives in this directory.

use crate::areas::AreaSpec;

/// The state of the area: its editors and workspaces. `m.areas.cross_pool`.
#[derive(Default)]
pub struct State {}

pub fn spec() -> AreaSpec {
    AreaSpec::new("cross_pool")
}
