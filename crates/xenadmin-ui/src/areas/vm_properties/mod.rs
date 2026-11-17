//! Area `vm_properties`. Not ported yet.
//!
//! Rune modules: `vm_properties.rs`.
//! Tabs: none (the Properties dialog of a VM).
//! See `notes/briefs/AREAS.md` for the full row and `notes/briefs/MODEL-API.md`
//! for how to fill this in. Everything of the area lives in this directory.

use crate::areas::AreaSpec;

/// The state of the area: its editors and workspaces. `m.areas.vm_properties`.
#[derive(Default)]
pub struct State {}

pub fn spec() -> AreaSpec {
    AreaSpec::new("vm_properties")
}
