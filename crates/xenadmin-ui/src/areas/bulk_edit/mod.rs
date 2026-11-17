//! Area `bulk_edit`. Not ported yet.
//!
//! Rune modules: `bulk_delete.rs`, `bulk_properties.rs`, `bulk_properties_model.rs`.
//! Tabs: none.
//! See `notes/briefs/AREAS.md` for the full row and `notes/briefs/MODEL-API.md`
//! for how to fill this in. Everything of the area lives in this directory.

use crate::areas::AreaSpec;

/// The state of the area: its editors and workspaces. `m.areas.bulk_edit`.
#[derive(Default)]
pub struct State {}

pub fn spec() -> AreaSpec {
    AreaSpec::new("bulk_edit")
}
