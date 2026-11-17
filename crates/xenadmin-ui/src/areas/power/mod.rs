//! Area `power`. Not ported yet.
//!
//! Rune modules: power buttons, `force_action`, `unknown_review`, `ActionDiagnostics` of `main.rs`; `bulk_power.rs`, `vm_placement.rs`, `vm_recovery_view.rs`.
//! Tabs: none.
//! See `notes/briefs/AREAS.md` for the full row and `notes/briefs/MODEL-API.md`
//! for how to fill this in. Everything of the area lives in this directory.

use crate::areas::AreaSpec;

/// The state of the area: its editors and workspaces. `m.areas.power`.
#[derive(Default)]
pub struct State {}

pub fn spec() -> AreaSpec {
    AreaSpec::new("power")
}
