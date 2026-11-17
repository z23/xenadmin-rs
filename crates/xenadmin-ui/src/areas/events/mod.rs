//! Area `events`. Not ported yet.
//!
//! Rune modules: `activity.rs`, `activity_export.rs`, `failure_help.rs`; the history list, Recheck, Acknowledge and Cancel of `resource_admin.rs`.
//! Tabs: Alerts and Events.
//! See `notes/briefs/AREAS.md` for the full row and `notes/briefs/MODEL-API.md`
//! for how to fill this in. Everything of the area lives in this directory.

use crate::areas::AreaSpec;

/// `Family` and `HiddenKey` of the Rune `activity.rs`: what `Preferences`
/// stores for a hidden history entry.
pub mod hidden;

/// The state of the area: its editors and workspaces. `m.areas.events`.
#[derive(Default)]
pub struct State {}

pub fn spec() -> AreaSpec {
    AreaSpec::new("events")
}
