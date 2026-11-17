//! Area `performance`. Not ported yet.
//!
//! Rune modules: `metrics_view.rs`, `metric_presets.rs`, `host_monitoring_view.rs`.
//! Tabs: Performance.
//! See `notes/briefs/AREAS.md` for the full row and `notes/briefs/MODEL-API.md`
//! for how to fill this in. Everything of the area lives in this directory.

use crate::areas::AreaSpec;

/// The graph preset catalog `Preferences` stores (the data of the Rune
/// `metric_presets.rs`).
pub mod presets;

/// The state of the area: its editors and workspaces. `m.areas.performance`.
#[derive(Default)]
pub struct State {}

pub fn spec() -> AreaSpec {
    AreaSpec::new("performance")
}
