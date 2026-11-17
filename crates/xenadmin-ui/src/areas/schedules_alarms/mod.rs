//! Area `schedules_alarms`. Not ported yet.
//!
//! Rune modules: `snapshot_policy_view.rs`, `alarms_view.rs`.
//! Tabs: none.
//! See `notes/briefs/AREAS.md` for the full row and `notes/briefs/MODEL-API.md`
//! for how to fill this in. Everything of the area lives in this directory.

use crate::areas::AreaSpec;

/// The state of the area: its editors and workspaces. `m.areas.schedules_alarms`.
#[derive(Default)]
pub struct State {}

pub fn spec() -> AreaSpec {
    AreaSpec::new("schedules_alarms")
}
