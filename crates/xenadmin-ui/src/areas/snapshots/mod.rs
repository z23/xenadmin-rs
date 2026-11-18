//! Area `snapshots`: the Snapshots tab of a VM and the snapshot dialogs.
//!
//! Rune modules: `vm_snapshots.rs`, `snapshot_inspection.rs`,
//! `snapshot_capture.rs`. Tabs: Snapshots (of a VM). Plan package P3.4.
//!
//! The Rune Snapshots page replaced the content area; here it is the body of
//! the VM's Snapshots tab. Its buttons are commands in the tab's local
//! action row, the "Snapshot information" dialog is a section of the tab
//! under the list, and the five variants of the one Rune dialog are five
//! dialogs.
//!
//! | Rune control (key) | Command | Label | Where |
//! | --- | --- | --- | --- |
//! | `create-snapshot` | `vm.snapshot` | Take a Snapshot | VM > Take a Snapshot; a VM's context menu; local row 10 |
//! | `restore-snapshot` | `snapshot.revert` | Revert | local row 20; row menu |
//! | `edit-snapshot` | `snapshot.properties` | Edit properties | local row 30; row menu |
//! | `clone-snapshot-vm` | `snapshot.new-vm` | Create VM | local row 40; row menu |
//! | `clone-snapshot-template` | `snapshot.new-template` | Create template | local row 50; row menu |
//! | `delete-snapshot` | `snapshot.delete` | Delete | local row 90; row menu |
//!
//! The row actions read this area's own selected row ([`selected_snapshot`]):
//! a snapshot row is not a tree object, so the scope and the VM menu keep
//! meaning the VM while a row is selected. With none or several rows
//! selected they are disabled with the reason; in a synthetic pool they
//! open their dialog as a preview, as the Rune buttons did.
//!
//! | Surface | Id | Kind | Experimental |
//! | --- | --- | --- | --- |
//! | Create snapshot | `snapshot-create` | dialog (form) | no |
//! | Restore this snapshot? | `snapshot-restore` | dialog (review, key `snapshot-review`) | no |
//! | Edit snapshot properties | `snapshot-edit` | dialog (form) | no |
//! | Create VM / template from snapshot | `snapshot-clone` | dialog (form) | no |
//! | Delete this snapshot? | `snapshot-delete` | dialog (review, key `snapshot-review`) | no |
//!
//! None carries the experimental notice: Rune listed `snapshot_dialog` and
//! `snapshot_inspection` as supported (decision 1 of `AREAS.md`).
//!
//! Scenes, under their Rune names: `snapshots`, `snapshot-create`,
//! `snapshot-restore`, `snapshot-edit`, `snapshot-capture`,
//! `snapshot-ancestry`, `snapshot-information`.
//!
//! What other areas use of this one:
//!
//! - [`shown_vm`]: the VM whose snapshots the tab lists.
//! - [`selected_snapshots`], [`selected_snapshot`]: the selected rows, in the
//!   table's order (the batch export of `bulk_export` reviews them).
//! - [`commands::TAKE`] and the other command ids.
//!
//! What this area uses of others: `power::track_vm` (a VM command through
//! the connection of the VM's pool), `power::request_cancellation` and
//! `power::recheck` (controls of an operation record), and the command
//! `snapshot.export` of `bulk_export`: the button
//! `snapshot-batch-export-open` follows that command (disabled with its
//! reason, absent where the command is hidden).
//! A search result that is a snapshot arrives as a grid selection of the
//! snapshot object (`search::show`); the tab shows it as its selected row.
//!
//! What has no successor, and where it went:
//!
//! - "Back to inventory" (`back-from-snapshots`) and the VM name heading:
//!   the tab strip and the object header.
//! - "Inspect snapshot" (`inspect-snapshot`), the dialog
//!   `snapshot-inspection-dialog` and its Close
//!   (`snapshot-inspection-close`): the information is the section
//!   `snapshot-information` of the tab, shown for the one selected snapshot.
//!   It reads the inventory as it is now, where the Rune dialog kept the
//!   inventory of the moment it was opened; so its caption no longer says
//!   "Captured pool … at capture" (decision 10).
//! - "Activity" (`snapshot-activity`): the Events tab of the Notifications
//!   view (decision 6).
//! - "Export…" (`export-xva`) and "Export selection…": one button into
//!   `bulk_export`'s command (see above).
//! - The captions under the buttons that said why cloning or deleting was
//!   not offered: the disabled reasons of those commands.
//! - The page's error line: the shell's status bar shows `m.error`; the
//!   dialogs still show it (`snapshot-error`).
//! - The compact layout (short caption, no Created column, fixed table
//!   height below 700 px): the renderer lays the description out.
//!
//! The files, in the order to read them:
//!
//! - [`edit`]: `vm_snapshots.rs` without its view. The rows, the selection,
//!   the dialogs with what each captured, validation, the permission
//!   requirements and the submission.
//! - [`capture`]: `snapshot_capture.rs`. The opt-in preview workflow.
//! - [`inspection`]: `snapshot_inspection.rs`. The saved preview (query,
//!   decoder) and the description of the information section.
//! - [`commands`]: the command table.
//! - [`view`]: the tab body and the dialogs, described.
//! - [`scenes`], `tests`.

use xenadmin_core::{PoolId, Snapshot, SnapshotId, VmId};

use crate::prelude::*;

pub mod capture;
pub mod commands;
pub mod edit;
pub mod inspection;
pub mod scenes;
pub mod view;

#[cfg(test)]
mod tests;

/// The state of the area. `m.areas.snapshots`.
pub struct State {
    /// The rows selected in the Snapshots tab of the scope VM (the Rune
    /// `snapshot_table` selection). Dropped when the scope changes.
    pub selected: Vec<SnapshotId>,
    /// Ancestry order instead of newest first (the Rune `snapshot_tree`).
    pub tree: bool,
    /// The open dialog (the Rune `snapshot_dialog`).
    pub dialog: Option<edit::Dialog>,
    /// The drafts the dialogs share, as in Rune: the name of Create, Edit
    /// and Clone, the description of Create and Edit, the memory choice of
    /// Create.
    pub name: String,
    pub description: String,
    pub memory: bool,
    /// The saved preview of one snapshot, once it was asked for.
    pub inspection: Option<inspection::Panel>,
    /// One decode at a time; the slot outlives the panel that started it.
    pub decoder: inspection::Decoder,
    /// The preview capture workflow. Never absent; `busy()` says whether
    /// one is pending.
    pub capture: capture::Workflow,
}

impl Default for State {
    fn default() -> Self {
        Self {
            selected: Vec::new(),
            tree: true,
            dialog: None,
            name: String::new(),
            description: String::new(),
            memory: false,
            inspection: None,
            decoder: inspection::Decoder::default(),
            capture: capture::Workflow::default(),
        }
    }
}

pub fn spec() -> AreaSpec {
    AreaSpec {
        commands: commands::commands,
        scenes: scenes::scenes,
        surfaces: view::surfaces,
        tab: view::tab,
        poll,
        on_scope_change: leave,
        on_session_change: leave,
        ..AreaSpec::new("snapshots")
    }
}

/// The saved preview's query and decoder, then the capture workflow. Both
/// return at once; `true` only when a message or a picture changed.
fn poll(m: &mut Model, _clock: f64) -> bool {
    let inspected = inspection::poll(m);
    let captured = capture::poll(m);
    inspected || captured
}

/// Another object selected in the tree, another pool shown: the dialog, the
/// selection and the saved preview belong to the VM that was shown. A
/// capture that has not submitted its snapshot yet is cancelled with the
/// dialog; one whose snapshot was submitted goes on (it follows the
/// connection of its pool, not the scope).
pub(crate) fn leave(m: &mut Model) {
    capture::cancel_dialog(m);
    let a = &mut m.areas.snapshots;
    a.selected.clear();
    a.inspection = None;
}

/// The VM whose snapshots the Snapshots tab lists: the tree scope, when it
/// is one VM (the Rune `snapshot_view`).
pub fn shown_vm(m: &Model) -> Option<(PoolId, VmId)> {
    let object = m.scope().single_of(ObjectKind::Vm)?;
    Some((object.pool?, object.vm_id()?))
}

/// The selected snapshots of the shown VM, in the table's order.
pub fn selected_snapshots(m: &Model) -> Vec<Snapshot> {
    edit::selected_snapshots(m)
}

/// The one selected snapshot. Restore, edit, copy, delete and the
/// information section act on one snapshot, so with several selected none
/// of them has a target.
pub fn selected_snapshot(m: &Model) -> Option<Snapshot> {
    edit::selected_snapshot(m)
}
