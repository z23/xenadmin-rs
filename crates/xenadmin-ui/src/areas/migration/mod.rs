//! Area `migration`: moving VMs within their pool.
//!
//! Rune modules: `vm_migration.rs` (one VM) and `bulk_move.rs` (1 to 32
//! VMs). Tabs: none. Plan packages P3.7 and W5; destination "Migrate or
//! Move VM, within a pool or across pools" of the plan's Wizards table,
//! of which this is the half within a pool. The half across pools
//! (`cross_pool_vm.rs`, `bulk_cross_pool.rs`) is the `cross_pool` area's.
//!
//! | Command | Label | Menu | Also | On one VM | On several VMs |
//! | --- | --- | --- | --- | --- | --- |
//! | `vm.migrate` | Migrate to Server | VM (`place::VM_MIGRATE`) | VM context menu | wizard `migration-dialog` | wizard `batch-move` |
//! | `vm.move` | Move VM | VM (`place::VM_MOVE`) | VM context menu | the same | the same |
//!
//! The plan lists two menu items; the client has one workflow for a host
//! move, a move of the disks, or both, so both commands open the same
//! wizard with the same eligibility ([`commands`]). The id `vm.move.bulk`
//! that the map proposed for the batch is not used: the batch is what the
//! two commands open on a multi-selection, as the power commands do.
//!
//! | Surface | Kind | Steps | Notice |
//! | --- | --- | --- | --- |
//! | `migration-dialog` | wizard | Destination host; Storage and networks; Review migration | no (supported) |
//! | `batch-move` | wizard | Destination and device mappings; Review moves | experimental |
//!
//! The steps are the stages the Rune editors have: the single move has
//! three (the plan's "Storage" and "Networking" are one stage here, with
//! the transfer network), the batch has its draft and its review. The last
//! step of each is the Rune review.
//!
//! Scenes: `vm-migration`, `batch-move` ([`scenes`]). The scene
//! `cross-pool` of the `cross_pool` area starts from the same fixture:
//! [`scenes::prepare`].
//!
//! What other areas use of this one:
//!
//! - [`host_label`], [`sr_label`], [`net_label`]: the labels of the review
//!   lines (Rune `vm_migration::{host_label, sr_label, net_label}`, used by
//!   `bulk_cross_pool.rs`);
//! - [`selected`]: the VMs of a multi-selection with the Rune refusals
//!   (Rune `bulk_move::selected`, used by `bulk_cross_pool.rs`);
//! - [`scenes::prepare`], [`scenes::demo_vm`]: the scene fixture.
//!
//! What this area uses of another: the command `vm.move-to-pool` of
//! `cross_pool` (not ported yet; the id its map proposes), as the button
//! `migration-cross-pool` on the second step of the wizard, under the Rune
//! caption that sends the user there. It is absent until that command is
//! registered.
//!
//! What of the Rune code has no successor here, and where it went:
//!
//! - The buttons `open-vm-migration` ("Migrate / move VM", details pane)
//!   and `batch-move-open` ("Move selection…", selection bar and row
//!   menu): the two commands. The caption under the disabled button is the
//!   reason of the disabled command.
//! - The headings "1 of 3 · Destination host", "2 of 3 · Storage and
//!   networks", "3 of 3 · Review migration": the step list of the wizard.
//! - The check mark in front of the chosen button of each list: each list
//!   is one choice ([`view`] says which key went where).
//! - The two-line captions of the single review ("Disk {vdi}" above
//!   "{from} → {to}"): one line each, in the form of the batch review.
//! - `show_details = false` on opening: there is no details pane.
//! - Opening the pool history after a batch was accepted
//!   (`resource_admin::open_pool(.., Kind::History)`): the Events tab of
//!   the Notifications view (`Model::show_operations`).
//! - A wizard kept open over another pool after a profile switch, showing
//!   its "connection changed" line: both surfaces close on a session or
//!   scope change, as every area's do. The line still shows when the pool
//!   moves on under an open wizard.
//!
//! The files, in the order to read them:
//!
//! - [`wizard`]: the logic of one VM, carried over from `vm_migration.rs`.
//! - [`batch`]: the logic of several, carried over from `bulk_move.rs`.
//! - [`commands`]: the two commands and their eligibility.
//! - [`view`]: the description of the open wizard, and the table of keys.
//! - [`scenes`]: the two scenes.
//! - `tests`: every Rune test of the two modules, and one test for each
//!   hazard the map lists for them.

use xenadmin_core::{HostId, Inventory, NetworkId, SrId};

use crate::prelude::*;

pub mod batch;
pub mod commands;
pub mod scenes;
pub mod view;
pub mod wizard;

#[cfg(test)]
mod live_tests;
#[cfg(test)]
mod tests;

pub use batch::selected;

/// The state of the area. `m.areas.migration`. At most one is `Some`: each
/// is an open surface, and opening one closes the other.
#[derive(Default)]
pub struct State {
    /// The wizard of one VM (Rune `vm_migration`).
    pub wizard: Option<Box<wizard::Editor>>,
    /// The batch of several (Rune `bulk_move`).
    pub batch: Option<batch::Editor>,
}

pub fn spec() -> AreaSpec {
    AreaSpec {
        commands: commands::commands,
        scenes: scenes::scenes,
        surfaces: view::surfaces,
        on_scope_change: close,
        on_session_change: close,
        ..AreaSpec::new("migration")
    }
}

/// Another object selected in the tree, another pool shown: both surfaces
/// belong to what they captured, so nothing of them is kept.
pub(crate) fn close(m: &mut Model) {
    m.areas.migration = State::default();
}

/// "{name} ({id})", or the bare id of a host the inventory does not have.
pub fn host_label(i: &Inventory, id: HostId) -> String {
    i.hosts
        .get(&id)
        .map_or_else(|| id.to_string(), |h| format!("{} ({id})", h.name))
}

/// "{name} ({id})", or the bare id of a repository the inventory does not
/// have.
pub fn sr_label(i: &Inventory, id: SrId) -> String {
    i.storage
        .iter()
        .find(|d| d.uuid == id.uuid())
        .map_or_else(|| id.to_string(), |d| format!("{} ({id})", d.name))
}

/// "{name} ({id})", or the bare id of a network the inventory does not
/// have.
pub fn net_label(i: &Inventory, id: NetworkId) -> String {
    i.networks
        .iter()
        .find(|d| d.uuid == id.uuid())
        .map_or_else(|| id.to_string(), |d| format!("{} ({id})", d.name))
}
