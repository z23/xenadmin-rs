//! The four scenes of the Rune module, under their Rune names. Each goes the
//! way a user goes: the demo pool, a guest selected, its Properties dialog
//! opened by the kernel's command, a page chosen.
//!
//! The Rune scenes showed the one scrolling editor. A scene here shows the
//! page that holds what its Rune scene was rendered for:
//!
//! | Scene | Guest | Page |
//! | --- | --- | --- |
//! | `properties` | halted `...0000` | General (the dialog as it opens) |
//! | `properties-running` | running `...0001` | CPU and Memory (live vCPUs, dynamic memory) |
//! | `home-host` | halted `...0000` | Review Changes, of "No preference" |
//! | `secure-boot` | halted `...0000` | Advanced (firmware and Secure Boot) |

use xenadmin_core::VmId;

use super::change::{self, Page};
use crate::prelude::*;

pub const HALTED: &str = "30000000-0000-0000-0000-000000000000";
pub const RUNNING: &str = "30000000-0000-0000-0000-000000000001";

pub fn scenes() -> Vec<SceneSpec> {
    vec![
        SceneSpec::new("properties", "vm_properties", |m| {
            open(m, HALTED, Page::General);
        }),
        SceneSpec::new("properties-running", "vm_properties", |m| {
            open(m, RUNNING, Page::CpuMemory);
        }),
        SceneSpec::new("home-host", "vm_properties", home_host_scene),
        SceneSpec::new("secure-boot", "vm_properties", firmware_scene),
    ]
}

/// The demo pool with one of its guests selected in the tree.
pub fn guest(m: &mut Model, id: &str) -> Option<Target> {
    m.load_demo(3, 48);
    let pool = m.inventory.as_ref()?.pool_id;
    let target = Target::one(ObjectRef::vm(pool, VmId::parse(id).ok()?));
    m.select(target.clone());
    Some(target)
}

fn open(m: &mut Model, id: &str, page: Page) {
    if let Some(target) = guest(m, id) {
        m.invoke(crate::properties::VM, &target);
        if page != Page::General {
            m.show_properties_page(page.id());
        }
    }
}

/// Rune `home_host_scene`: the home host set to "No preference", reviewed.
fn home_host_scene(m: &mut Model) {
    open(m, HALTED, Page::HomeServer);
    if let Some(w) = &mut m.areas.vm_properties.editor {
        w.home_host = None;
    }
    change::review(m);
}

/// Rune `firmware_scene`: the halted guest (the Rune scene forced the power
/// state; the demo guest `...0000` is halted) on the page of its firmware.
fn firmware_scene(m: &mut Model) {
    open(m, HALTED, Page::Advanced);
}
