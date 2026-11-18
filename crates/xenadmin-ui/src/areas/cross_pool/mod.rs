//! Area `cross_pool`: moving and copying VMs to another connected pool.
//!
//! Rune modules: `cross_pool_vm.rs` with `cross_pool_vm_view.rs` (one VM,
//! moved or copied) and `bulk_cross_pool.rs` (1 to 32 VMs, moved). Tabs:
//! none. Plan package W5; destination "Migrate or Move VM, within a pool or
//! across pools" of the plan's Wizards table, of which this is the half
//! across pools (the half within a pool is the `migration` area's).
//! Experimental: both surfaces carry the notice, as in Rune.
//!
//! | Command | Label | Menu | Also | On one VM | On several VMs |
//! | --- | --- | --- | --- | --- | --- |
//! | `vm.move-to-pool` | Move to Another Pool ("Move or Copy to Another Pool" on one VM) | VM, group 3, order 25 (between Move VM and Copy) | VM context menu; the button `migration-cross-pool` of the migration wizard | wizard `cross-pool` | wizard `batch-cross-pool` |
//!
//! | Surface | Kind | Steps | Notice |
//! | --- | --- | --- | --- |
//! | `cross-pool` | wizard | Destination and mode; Storage, NICs and transfer address; Review both pools | experimental |
//! | `batch-cross-pool` | wizard (large) | Destination pool and host; Storage, networks and transfer address; Review both pools | experimental |
//!
//! The steps are the three stages each Rune dialog has, and the last step
//! of each is its Rune review with the acknowledgement of host-to-host
//! transport. A transfer uses the connections of both pools: the source is
//! the pool of the command's target (whichever session shows it), the
//! destination one of the other writable connected pools captured when the
//! wizard opened ([`single::Peer`]). Neither follows the scope session.
//!
//! Scenes: `cross-pool`, `batch-cross-pool` ([`scenes`]).
//!
//! # For other areas
//!
//! - [`recheck`], [`can_recheck`], [`connection_for_pool`]: reading a
//!   transfer between two pools again (the Rune `cross_pool_vm::recheck`,
//!   `can_recheck` and `connection_for_pool`, unchanged in behaviour). The
//!   `events` area's Recheck calls the first and
//!   `session::resource_control_connection` the second; the copies both
//!   carried until this area landed are gone.
//! - [`destination_guest`], [`destination_target`], [`destination_label`],
//!   [`show_destination`]: the VM a finished single transfer created in its
//!   destination pool (the Rune buttons "Show destination VM" and "Open
//!   destination console" of the transfer history). The `events` area
//!   offers them as the button `cross-destination-<id>` of the record's
//!   outcome (`events::destination_action`, part of
//!   `events::outcome_actions`).
//! - [`Peer`], [`peers`]: a writable connected pool as a review captures
//!   it.
//!
//! What this area uses of others: `migration::{selected, host_label,
//! sr_label, net_label}` and its scene fixture; `events::show_operation`
//! (after a submission); `console::{tab_target, close_tab}` and
//! `console::windows::{open_windows, close}` (every console of a guest that
//! moves live, on the tab and in a detached window, closes at submission:
//! `single::release_console`) and the command
//! `vm.console` (from [`show_destination`]);
//! `session::select_transfer_destination`.
//!
//! # What has no successor here, and where it went
//!
//! - The buttons `cross-pool-open` ("Move / copy to another pool…",
//!   details pane) and `batch-cross-pool-open` ("Move selection to another
//!   pool…", selection bar and row menu): the one command. On one VM it
//!   opens the single wizard, which also copies; the Rune batch of exactly
//!   one VM is not reachable any more.
//! - The transfer history of the Rune dialog (`cross-pool-history`,
//!   "Cross-pool transfers", with `cross-recheck-<id>`, `cross-stop-<id>`,
//!   the two-step acknowledgement and `cross-destination-<id>`): the
//!   History list of the Events tab, which already lists these records
//!   with both pools, each VM of a batch and what was created, and has
//!   Recheck, Request cancellation and Acknowledge for them (`events`).
//!   The destination button is in the outcome of the record there: see
//!   [`show_destination`] above.
//! - `Panel::notice` (the result line of a control or a refused apply): a
//!   refused apply is the wizard's error line; the controls are Events'.
//! - The headings "n of 3 · …": the step list of each wizard.
//! - The check mark in front of the chosen button of each list: each list
//!   is one choice ([`view`] says which key went where). The two
//!   acknowledgement buttons are checkboxes.
//! - The two-line captions of the single review ("Disk {vdi}" above
//!   "{from} → {to}"): one line each, in the form of the batch review.
//! - The scroll container `batch-cross-pool-body`.
//! - Opening the pool history after a batch was accepted, and switching
//!   the single dialog to its history: the surface closes and the Events
//!   tab shows the operation (`events::show_operation`).
//! - A dialog kept open over another pool after a profile switch, showing
//!   its "connection changed" line: both surfaces close on a session or
//!   scope change, as every area's do. The line still shows when a pool
//!   moves on under an open wizard.
//! - The silent return of the Rune `open` for a suspended or paused VM
//!   behind an enabled button: the command is disabled with the words the
//!   batch has for the same guest.
//!
//! New texts (a disabled control carries its reason, and Rune had none):
//! [`batch::NOT_ACKNOWLEDGED`]; the single command without a writable
//! current connection says what the batch says ([`single::NOT_WRITABLE`]).
//!
//! The files, in the order to read them:
//!
//! - [`single`]: the logic of one VM, carried over from `cross_pool_vm.rs`,
//!   with what is done to a transfer after it was submitted.
//! - [`batch`]: the logic of several, carried over from `bulk_cross_pool.rs`.
//! - [`commands`]: the command and its eligibility.
//! - [`view`]: the description of the open wizard, and the table of keys.
//! - [`scenes`]: the two scenes.
//! - `tests`: every Rune test of the two modules, and one test for each
//!   hazard the map lists for them. `live_tests`: what needs a connection.

use crate::prelude::*;

pub mod batch;
pub mod commands;
pub mod scenes;
pub mod single;
pub mod view;

#[cfg(test)]
mod live_tests;
#[cfg(test)]
mod tests;

pub use single::{
    Peer, can_recheck, connection_for_pool, destination_guest, destination_label,
    destination_target, peers, recheck, show_destination,
};

/// The state of the area. `m.areas.cross_pool`. At most one of the two is
/// open.
#[derive(Default)]
pub struct State {
    /// The wizard of one VM (the editor of the Rune `cross_pool`).
    pub wizard: Option<Box<single::Editor>>,
    /// The batch of several (Rune `bulk_cross_pool`).
    pub batch: Option<batch::Editor>,
}

pub fn spec() -> AreaSpec {
    AreaSpec {
        commands: commands::commands,
        scenes: scenes::scenes,
        surfaces: view::surfaces,
        on_scope_change: close,
        on_session_change: close,
        ..AreaSpec::new("cross_pool")
    }
}

/// Another object selected in the tree, another pool shown: both surfaces
/// belong to what they captured, so nothing of them is kept.
pub(crate) fn close(m: &mut Model) {
    m.areas.cross_pool = State::default();
}
