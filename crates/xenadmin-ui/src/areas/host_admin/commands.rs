//! The commands of the area: what the buttons of the Rune "Host
//! administration" dialog became.
//!
//! The eligibility of every command is one rule:
//!
//! - `Hidden` when the target is not exactly one server (one pool, for
//!   autostart);
//! - `Disabled`, with the core's explanation, when the change could not be
//!   reviewed whatever the user entered ([`change::blocked`]): the dialog
//!   would open on something that cannot go ahead;
//! - `Enabled` otherwise.
//!
//! Rune offered all thirteen changes always and explained inside the dialog.
//! The check and its texts are the same ones; they moved in front of the
//! dialog, where the plan wants them (a disabled menu item with its reason).
//! The open dialog still makes every check again: the inventory moves on
//! while it is open.
//!
//! A command of a change without a draft opens its dialog on the review. One
//! with a draft (memory, forgetting a server) opens on the draft.
//!
//! The three settings of a server (multipathing, log destination, power-on
//! mode) have no command here: they are pages of the server's Properties
//! dialog, which the kernel's `server.properties` opens (`view::pages`).

use super::change::{self, Editor, Kind};
use crate::prelude::*;

const AREA: &str = "host_admin";

pub const REBOOT: CommandId = CommandId("server.reboot");
pub const POWER_ON: CommandId = CommandId("server.power-on");
pub const SHUT_DOWN: CommandId = CommandId("server.shutdown");
pub const RESTART_TOOLSTACK: CommandId = CommandId("server.restart-toolstack");
pub const MAINTENANCE: CommandId = CommandId("server.maintenance");
pub const EVACUATE: CommandId = CommandId("server.evacuate");
pub const CONTROL_DOMAIN_MEMORY: CommandId = CommandId("server.control-domain-memory");
pub const DESTROY: CommandId = CommandId("server.destroy");
pub const POOL_AUTOSTART: CommandId = CommandId("pool.vm-autostart");

/// Commands of other areas that pages of the Properties dialog lead to. In
/// Rune they were buttons of the chooser that opened another module's
/// dialog. A third such button, "Monitoring recording…", has no page here:
/// it is the `performance` area's command `server.monitoring-recording`.
pub const MULTIPATH_WORKFLOW: CommandId = CommandId("storage.multipath-workflow");
pub const POWER_ON_SETTINGS: CommandId = CommandId("server.power-on-settings");

/// The reason Rune's "Pool VM autostart…" button was disabled without
/// saying so: not the demo, and no writable current connection. The words
/// are those of the Rune transfer dialogs for the same condition.
pub const NOT_WRITABLE: &str = "This connection is read-only, disconnected or stale.";

/// In menu order. Labels are the plan's (Menus table) where it lists the
/// item; "Evacuate Guests" and "VM Autostart" exist only in this client
/// (`place::SERVER_EVACUATE`, `place::POOL_VM_AUTOSTART`).
pub fn commands() -> Vec<CommandSpec> {
    const SERVER: &[ObjectKind] = &[ObjectKind::Host];
    vec![
        CommandSpec::new(REBOOT.0, "Reboot", AREA)
            .menu(place::SERVER_REBOOT)
            .context(SERVER)
            .eligibility(|m, target| eligibility(m, target, Kind::Reboot))
            .invoke(|m, target| open(m, target, Kind::Reboot)),
        CommandSpec::new(POWER_ON.0, "Power On", AREA)
            .menu(place::SERVER_POWER_ON)
            .context(SERVER)
            .eligibility(|m, target| eligibility(m, target, Kind::PowerOn))
            .invoke(|m, target| open(m, target, Kind::PowerOn)),
        CommandSpec::new(SHUT_DOWN.0, "Shut Down", AREA)
            .menu(place::SERVER_SHUT_DOWN)
            .context(SERVER)
            .eligibility(|m, target| eligibility(m, target, Kind::Shutdown))
            .invoke(|m, target| open(m, target, Kind::Shutdown)),
        CommandSpec::new(RESTART_TOOLSTACK.0, "Restart Toolstack", AREA)
            .menu(place::SERVER_RESTART_TOOLSTACK)
            .context(SERVER)
            .eligibility(|m, target| eligibility(m, target, Kind::Restart))
            .invoke(|m, target| open(m, target, Kind::Restart)),
        // One item, as in the plan ("Enter or Exit Maintenance Mode"): it is
        // whichever of the two the server's state leaves possible.
        CommandSpec::new(MAINTENANCE.0, "Enter Maintenance Mode", AREA)
            .menu(place::SERVER_MAINTENANCE)
            .context(SERVER)
            .dynamic_label(maintenance_label)
            .eligibility(|m, target| eligibility(m, target, maintenance(m, target)))
            .invoke(|m, target| open(m, target, maintenance(m, target))),
        CommandSpec::new(EVACUATE.0, "Evacuate Guests", AREA)
            .menu(place::SERVER_EVACUATE)
            .context(SERVER)
            .eligibility(|m, target| eligibility(m, target, Kind::Evacuate))
            .invoke(|m, target| open(m, target, Kind::Evacuate)),
        // Also the entry to memory settings on a server's Memory tab.
        CommandSpec::new(CONTROL_DOMAIN_MEMORY.0, "Control Domain Memory", AREA)
            .menu(place::SERVER_CONTROL_DOMAIN_MEMORY)
            .context(SERVER)
            .local(TabId::Memory, 10)
            .local_scopes(&[ObjectKind::Host])
            .eligibility(|m, target| eligibility(m, target, Kind::Memory))
            .invoke(|m, target| open(m, target, Kind::Memory)),
        CommandSpec::new(DESTROY.0, "Destroy", AREA)
            .menu(place::SERVER_DESTROY)
            .context(SERVER)
            .eligibility(|m, target| eligibility(m, target, Kind::Forget))
            .invoke(|m, target| open(m, target, Kind::Forget)),
        CommandSpec::new(POOL_AUTOSTART.0, "VM Autostart", AREA)
            .menu(place::POOL_VM_AUTOSTART)
            .context(&[ObjectKind::Pool])
            .eligibility(can_change_autostart)
            .invoke(|m, target| open(m, target, Kind::Autostart)),
    ]
}

/// The editor the command would open for its target. `None`: the command
/// has nothing to do with this target.
fn capture(m: &Model, target: &Target, kind: Kind) -> Option<Editor> {
    Editor::capture(m, target.single()?, kind)
}

fn eligibility(m: &Model, target: &Target, kind: Kind) -> Eligibility {
    match capture(m, target, kind) {
        Some(editor) => Eligibility::enabled_unless(change::blocked(&editor)),
        None => Eligibility::Hidden,
    }
}

/// Runs only when the eligibility was `Enabled` for the same target, so a
/// change without a draft passes its review here.
fn open(m: &mut Model, target: &Target, kind: Kind) {
    let Some(editor) = capture(m, target, kind) else {
        return;
    };
    // One editor: a page of the Properties dialog that is open gives way.
    super::close(m);
    m.areas.host_admin.editor = Some(editor);
    if kind.opens_on_review() {
        change::review(m);
    }
}

/// Which of the two changes the maintenance item is. Leaving needs the
/// server marked in maintenance and entering needs it not marked, so at
/// most one of them can be available: the item is that one.
fn maintenance(m: &Model, target: &Target) -> Kind {
    let marked = target
        .single_of(ObjectKind::Host)
        .and_then(|object| {
            let inventory = m.inventory_for(object.pool?)?;
            let details = inventory.details.hosts.get(&object.host_id()?)?;
            details.maintenance_mode
        })
        .unwrap_or(false);
    if marked {
        Kind::Exit
    } else {
        Kind::Maintenance
    }
}

fn maintenance_label(m: &Model, target: &Target) -> Option<String> {
    (maintenance(m, target) == Kind::Exit).then(|| "Exit Maintenance Mode".to_owned())
}

/// Rune disabled "Pool VM autostart…" without a writable current connection
/// (except in the demo); that check stays in front of the common rule.
fn can_change_autostart(m: &Model, target: &Target) -> Eligibility {
    let Some(editor) = capture(m, target, Kind::Autostart) else {
        return Eligibility::Hidden;
    };
    if !change::synthetic(m, &editor) && !m.can_mutate_pool(editor.pool()) {
        return Eligibility::disabled(NOT_WRITABLE);
    }
    Eligibility::enabled_unless(change::blocked(&editor))
}
