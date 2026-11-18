//! Area `host_services`: a server's remote power-on configuration, and its
//! backup, status report, recovery staging and crashdump deletion (plan W4).
//!
//! Rune modules: `host_power_view.rs` ([`power`]) and `host_support_view.rs`
//! ([`support`]). Tabs: none.
//!
//! ## For other areas (public functions)
//!
//! | Function | For |
//! | --- | --- |
//! | [`power::open`] | the power-on settings dialog of a server (Rune `host_power_view::open`); the command `server.power-on-settings` does the same, and the Power On page of `host_admin` runs that command |
//! | [`support::open`] | the backup, status report or staging dialog of a server |
//!
//! ## Destination
//!
//! | Rune | Command | Menu | Surface |
//! | --- | --- | --- | --- |
//! | "BMC / power-on settings…" (`host-bmc-settings`, in `host_admin`) | `server.power-on-settings` "BMC / Power-On Settings" | none: the button of the Power On page of a server's Properties dialog | dialog `host-power-settings`, on its draft |
//! | "Host backup and support…" (`host-support-open`), mode "Filesystem backup" | `server.backup` "Back Up" | Server > Back Up | dialog `host-backup` |
//! | the same, mode "Stage recovery…" | `server.restore-from-backup` "Restore From Backup" | Server > Restore From Backup | dialog `host-restore` |
//! | the same, mode "Support bundle" | `tools.server-status-report` "Server Status Report" | Tools > Server Status Report | dialog `host-support` |
//! | "Review deletion…" of a crashdump in the support mode | a button of the status report dialog (`host-crashdump-delete-<uuid>`) | | dialog `host-crashdump-delete` |
//! | "Review unused Secret cleanup" of a retained record | a button of the power-on settings dialog (`host-power-cleanup-<id>`) | | the review of that dialog |
//! | "Transfer history", "Activity history", and the jump after a submission | button `host-support-history`; [`crate::areas::transfers::history`] and [`crate::areas::events::show`] | | the Events tab |
//!
//! Every surface is experimental (AREAS.md decision 1: Rune's
//! `experimental_dialog_open` had both dialogs flagged).
//!
//! ## Decisions
//!
//! - The panel's three modes are three dialogs; the mode is fixed by the
//!   command. Only the status report asks the server for its categories and
//!   crashdumps (Rune asked in every mode because the panel could switch).
//! - The dialog titles are the Rune names of the modes ("Filesystem
//!   backup", "Support bundle", "Stage recovery"): "Restore From Backup" is
//!   the plan's menu label, but the action stages a backup for a later
//!   installer-assisted recovery and restores nothing that runs.
//! - Both reviews of the panel gained Back (`host-support-back`). It runs
//!   the Rune mode switch for the mode the panel is in, so the file, the
//!   review, the typed confirmation and a running inspection go (hazard 23).
//! - The archive commands are disabled where Rune opened the panel and
//!   showed the error of its first refresh: no connection, no current
//!   observation. The power-on settings stay enabled for every known server.
//! - Neither panel submission is checked against the session's permissions:
//!   Rune's panel had no permission line and no check. The power-on review
//!   has both, as in Rune.
//! - Crashdump deletion keeps the cap of 64 resource operation handles as
//!   written (hazard 21).
//! - The crashdump list stays a paged list of lines with a button each, with
//!   the Rune keys; it is not a table.
//!
//! ## What has no successor
//!
//! - The mode row `host-support-backup`, `host-support-bundle`,
//!   `host-support-restore` and the opening button `host-support-open`: the
//!   three commands.
//! - The repeated caption above the staging confirmation input: the input's
//!   label says it.
//!
//! ## Scenes
//!
//! `host-support`, `host-restore`, `host-crashdump-delete`,
//! `host-power-settings`.
//!
//! ## Tests (Rune test, disposition)
//!
//! | Rune test | Here |
//! | --- | --- |
//! | power `review_has_private_password_and_captured_target_at_both_widths` | ported (Harness); the two window sizes are a GPUI test (`gpui-tests-wanted.md`) |
//! | power `configuration_review_refuses_invalid_inputs_and_unknown_configuration` | moved |
//! | power `switching_mode_discards_pending_password_and_review` | moved |
//! | support `host_archive_review_captures_target_and_keeps_compact_footer_visible` | ported (Harness); the footer at 513x652 is a GPUI test |
//! | support `crashdump_delete_review_captures_exact_dump_and_keeps_footer_visible` | ported (Harness); the footer at 513x652 is a GPUI test |
//! | support `crashdump_catalogue_pages_make_every_captured_candidate_reachable` | ported (Harness) |
//! | support `crashdump_delete_refuses_unadvertised_review_or_changed_host` | ported (Harness) |
//! | support `stale_native_file_callback_cannot_set_a_reopened_host_review` | ported (Harness) |
//! | support `unknown_capabilities_and_changed_host_version_cannot_be_reviewed` | ported (Harness) |
//! | support `restore_native_open_inspects_captured_host_and_requires_exact_confirmation` | ported (Harness; the archive is built by hand, the crate has no `tar` or `flate2`) |
//! | support `restore_rejects_wrong_host_and_cancels_obsolete_inspections` | ported (Harness) |
//! | `main.rs` `dialogs_outside_the_supported_core_carry_the_experimental_notice` | this area's share ported |
//! | `session_permissions_view.rs` `review_permission_notice_is_reachable_without_bypassing_synthetic_target_guard` | this area's share (`host-power-settings`) ported |

use crate::prelude::*;

pub mod commands;
pub mod power;
pub mod scenes;
pub mod support;
pub mod view;

#[cfg(test)]
mod tests;

/// The state of the area. `m.areas.host_services`.
#[derive(Default)]
pub struct State {
    /// The open power-on settings editor.
    pub power: Option<power::Editor>,
    /// The open backup, status report or staging panel.
    pub support: Option<support::Panel>,
}

pub fn spec() -> AreaSpec {
    AreaSpec {
        commands: commands::commands,
        scenes: scenes::scenes,
        surfaces: view::surfaces,
        poll,
        on_scope_change: close,
        on_session_change: close,
        ..AreaSpec::new("host_services")
    }
}

/// The answer of the status report query and the result of an archive
/// inspection arrive here. Nothing open: nothing changed.
fn poll(m: &mut Model, _clock: f64) -> bool {
    support::poll(m)
}

/// Another object selected in the tree, another pool shown: both editors
/// belong to the one server they were opened for. Dropping the panel
/// cancels a running archive inspection.
pub(crate) fn close(m: &mut Model) {
    m.areas.host_services.power = None;
    m.areas.host_services.support = None;
}
