//! The one command of the area. The Properties dialog and its commands
//! (`vm.properties`, `template.properties`) are the kernel's; the Rune
//! buttons "Properties" and "Template properties" of the details pane and
//! "Properties…" of the row menu are those commands now.
//!
//! The plan gives the Memory tab an "entry to memory settings". The tab is
//! the `general` area's information; the entry is this command in the tab's
//! local action row, which opens the Properties dialog of the VM or template
//! on the page CPU and Memory.

use super::change::{self, Page};
use crate::prelude::*;

const AREA: &str = "vm_properties";

pub const MEMORY_SETTINGS: CommandId = CommandId("vm.memory-settings");

pub fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new(MEMORY_SETTINGS.0, "Memory Settings", AREA)
            .local(TabId::Memory, 10)
            .eligibility(|m, target| {
                // The Memory tab of a server or a pool has the row too: not
                // theirs. Several objects: not this command's.
                if change::known(m, target).is_some() {
                    Eligibility::Enabled
                } else {
                    Eligibility::Hidden
                }
            })
            .invoke(|m, target| {
                m.open_properties(target, Some(Page::CpuMemory.id()));
            }),
    ]
}
