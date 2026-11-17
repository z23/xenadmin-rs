//! Area `vm_devices`. Not ported yet.
//!
//! Rune modules: `vm_devices.rs`, `vm_device_actions.rs`, `vm_device_lifecycle.rs`, `vm_device_properties.rs`, `disk_workspace.rs`.
//! Tabs: Storage and Networking of a VM or template; Storage of a repository.
//! See `notes/briefs/AREAS.md` for the full row and `notes/briefs/MODEL-API.md`
//! for how to fill this in. Everything of the area lives in this directory.

use crate::areas::AreaSpec;

/// The state of the area: its editors and workspaces. `m.areas.vm_devices`.
#[derive(Default)]
pub struct State {}

pub fn spec() -> AreaSpec {
    AreaSpec::new("vm_devices")
}
