//! The commands of the area.
//!
//! Rune reached this area by two buttons: "Host backup and support…" in a
//! server's details pane, which opened one panel with three modes, and "BMC
//! / power-on settings…" in the host administration chooser. The panel's
//! modes are three commands now, at the places the plan's Menus table gives
//! them; the power-on settings keep one command without a menu place, which
//! the Power On page of a server's Properties dialog (`host_admin`) runs.
//!
//! Eligibility, one rule for the three archive commands:
//!
//! - `Hidden` when the target is not exactly one server of a known pool;
//! - `Disabled` when the panel could never reach a submission
//!   ([`support::blocked`]): no connection, no current observation (the
//!   errors Rune showed inside the panel right after opening), and for
//!   staging a connection that cannot write (Rune disabled "Stage
//!   recovery…" there, without a reason; the text is [`NOT_WRITABLE`]);
//! - `Enabled` otherwise. A download needs no write access.
//!
//! The power-on settings are `Enabled` for every known server, as the Rune
//! button was: the dialog also shows the current configuration and the
//! retained Secrets, and says itself what cannot be reviewed.

use super::power;
use super::support::{self, Mode, Panel};
use crate::prelude::*;

const AREA: &str = "host_services";

pub const BACK_UP: CommandId = CommandId("server.backup");
pub const RESTORE_FROM_BACKUP: CommandId = CommandId("server.restore-from-backup");
pub const STATUS_REPORT: CommandId = CommandId("tools.server-status-report");
pub const POWER_ON_SETTINGS: CommandId = CommandId("server.power-on-settings");

/// The words of the Rune transfer dialogs for a connection that cannot
/// write (`host_admin` uses the same for pool VM autostart).
pub const NOT_WRITABLE: &str = "This connection is read-only, disconnected or stale.";

/// Labels are the plan's (Menus table). The Rune names of the three modes
/// ("Filesystem backup", "Stage recovery…", "Support bundle") are the
/// titles of the dialogs.
pub fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new(BACK_UP.0, "Back Up", AREA)
            .menu(place::SERVER_BACK_UP)
            .eligibility(|m, target| eligibility(m, target, Mode::Backup))
            .invoke(|m, target| open(m, target, Mode::Backup)),
        CommandSpec::new(RESTORE_FROM_BACKUP.0, "Restore From Backup", AREA)
            .menu(place::SERVER_RESTORE_FROM_BACKUP)
            .eligibility(|m, target| eligibility(m, target, Mode::Restore))
            .invoke(|m, target| open(m, target, Mode::Restore)),
        CommandSpec::new(STATUS_REPORT.0, "Server Status Report", AREA)
            .menu(place::TOOLS_SERVER_STATUS_REPORT)
            .eligibility(|m, target| eligibility(m, target, Mode::Support))
            .invoke(|m, target| open(m, target, Mode::Support)),
        CommandSpec::new(POWER_ON_SETTINGS.0, "BMC / Power-On Settings", AREA)
            .eligibility(|m, target| {
                let known = target
                    .single()
                    .is_some_and(|object| power::Editor::capture(m, object).is_some());
                if known {
                    Eligibility::Enabled
                } else {
                    Eligibility::Hidden
                }
            })
            .invoke(|m, target| {
                if let Some(object) = target.single() {
                    power::open(m, object);
                }
            }),
    ]
}

fn eligibility(m: &Model, target: &Target, mode: Mode) -> Eligibility {
    match target
        .single()
        .and_then(|object| Panel::capture(m, object, mode))
    {
        Some(panel) => Eligibility::enabled_unless(support::blocked(m, &panel)),
        None => Eligibility::Hidden,
    }
}

fn open(m: &mut Model, target: &Target, mode: Mode) {
    if let Some(object) = target.single() {
        support::open(m, object, mode);
    }
}
