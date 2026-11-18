//! The commands of the area: what opened the two Rune modules' dialogs.
//!
//! | Rune entry | Command | Where |
//! | --- | --- | --- |
//! | "Guest tools media" (details pane), "Mount guest tools media" (devices page), both `open-guest-tools` | `vm.install-guest-tools` | VM > Install Guest Tools; a VM's context menu; the local row of a VM's Storage tab |
//! | "Guest SSH, RDP and tools…" (`guest-tools`, VM details) | `vm.connect-external` | a VM's context menu; the local row of a VM's Console tab |
//! | "Configure external tools…" (`configure-external-tools`, Options) | `tools.external-tools` | Tools > External Tools; the button in Options (the `settings` area) |
//!
//! Eligibility:
//!
//! - `vm.install-guest-tools` is `Hidden` unless the target is one VM that
//!   the inventory of its pool has (the restatement of Rune's refusal to
//!   open from a devices page of another pool). It is `Disabled`, with the
//!   text of the dialog's empty state, when that VM has no CD drive or its
//!   pool no tools media: no choice in the dialog could lead to a review.
//!   On a VM's Storage tab the grid selection is a disk; the command then
//!   acts on the VM of the scope, as the Rune button of the devices page
//!   acted on the VM of that page.
//! - `vm.connect-external` is `Hidden` unless the target is one VM that is
//!   neither a template nor a control domain, and `Disabled` unless it is
//!   running (Rune disabled its button for all of these without a reason;
//!   [`NOT_RUNNING`] is new text).
//! - `tools.external-tools` acts on no object and is always enabled.

use super::{external, media};
use crate::prelude::*;

const AREA: &str = "vm_tools";

pub const INSTALL_GUEST_TOOLS: CommandId = CommandId("vm.install-guest-tools");
pub const CONNECT_EXTERNAL: CommandId = CommandId("vm.connect-external");
pub const EXTERNAL_TOOLS: CommandId = CommandId("tools.external-tools");

/// New text: Rune disabled "Guest SSH, RDP and tools…" without a reason.
pub const NOT_RUNNING: &str = "External tools connect to a running guest. Start this VM first.";

pub fn commands() -> Vec<CommandSpec> {
    const VM: &[ObjectKind] = &[ObjectKind::Vm];
    vec![
        CommandSpec::new(INSTALL_GUEST_TOOLS.0, "Install Guest Tools", AREA)
            .menu(place::VM_INSTALL_GUEST_TOOLS)
            .context(VM)
            .local(TabId::Storage, 90)
            .local_scopes(&[ObjectKind::Vm])
            .eligibility(media_state)
            .invoke(|m, target| {
                if let Some(object) = media_vm(m, target) {
                    media::open(m, &object);
                }
            }),
        // The plan's Menus table has no row for it; like "Open Console" it
        // is on the context menu and on the Console tab's row.
        CommandSpec::new(
            CONNECT_EXTERNAL.0,
            "Connect with SSH, RDP or Local Tool…",
            AREA,
        )
        .context(VM)
        .local(TabId::Console, 200)
        .local_scopes(&[ObjectKind::Vm])
        .eligibility(connect_state)
        .invoke(|m, target| {
            if let Some(object) = target.single_of(ObjectKind::Vm).cloned() {
                external::open(m, &object);
            }
        }),
        CommandSpec::new(EXTERNAL_TOOLS.0, "External Tools", AREA)
            .menu(place::TOOLS_EXTERNAL_TOOLS)
            .invoke(|m, _| external::configure(m)),
    ]
}

/// The VM "Install Guest Tools" acts on: the target when it is one VM;
/// the scope's VM when the target is a selection of that VM's disks (its
/// Storage tab) or nothing.
fn media_vm(m: &Model, target: &Target) -> Option<ObjectRef> {
    if let Some(object) = target.single_of(ObjectKind::Vm) {
        return Some(object.clone());
    }
    target
        .objects
        .iter()
        .all(|object| object.kind == ObjectKind::VirtualDisk)
        .then(|| m.scope().single_of(ObjectKind::Vm).cloned())
        .flatten()
}

fn media_state(m: &Model, target: &Target) -> Eligibility {
    match media_vm(m, target).and_then(|object| media::capture(m, &object)) {
        Some(panel) => Eligibility::enabled_unless(media::blocked(&panel)),
        None => Eligibility::Hidden,
    }
}

fn connect_state(m: &Model, target: &Target) -> Eligibility {
    let guest = target.single_of(ObjectKind::Vm).and_then(|object| {
        let vm = object.vm_id()?;
        let inventory = m.inventory_for(object.pool?)?;
        let record = inventory.vms.get(&vm)?;
        (!record.is_template && !record.is_control_domain)
            .then(|| external::ordinary(&inventory, vm))
    });
    match guest {
        Some(running) => Eligibility::enabled_if(running, NOT_RUNNING),
        None => Eligibility::Hidden,
    }
}
