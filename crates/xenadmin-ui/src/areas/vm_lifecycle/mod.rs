//! Area `vm_lifecycle`: creating, copying, converting and deleting one VM or
//! template, and the cloud-init drive of a VM.
//!
//! Rune modules: `vm_lifecycle.rs`, `vm_creation.rs`, `vm_creation_cloud.rs`,
//! `vm_deletion.rs`, `cloud_init_view.rs`. Tabs: none.
//!
//! # Where everything went
//!
//! | Rune | Here |
//! | --- | --- |
//! | "Create VM…" (`create-from-template`) of a template's details pane | commands `vm.new`, `templates.new`, `templates.create-vm`; the wizard `vm-lifecycle-dialog` |
//! | "Clone or full copy…" (`clone-vm`) | commands `vm.copy`, `templates.copy`; a dialog with a draft and its review |
//! | "Convert to template…" (`convert-vm-template`) | command `vm.convert-to-template`; a dialog that opens on its review, without Back |
//! | "Delete VM…" / "Delete template…" (`delete-vm`) | commands `vm.delete`, `templates.delete`; a dialog with a draft and its review |
//! | "Cloud-init drive…" (`open-cloud-init`) | command `vm.cloud-init-drive`; the dialog `cloud-init-dialog` |
//! | the panel after a creation with cloud-init inputs | the dialog `vm-lifecycle-dialog`, titled "Create VM · configuration drive" |
//!
//! # Commands
//!
//! | Id | Label | Menu | Also | Opens |
//! | --- | --- | --- | --- | --- |
//! | `vm.new` | New VM | VM | the toolbar's New VM slot | the wizard for the selected template |
//! | `templates.new` | New VM wizard | Templates | | the same |
//! | `templates.create-vm` | Create VM From Selection | Templates | template context menu | the same |
//! | `vm.copy` | Copy VM | VM | VM context menu | the copy dialog |
//! | `templates.copy` | Copy | Templates | template context menu | the copy dialog |
//! | `vm.convert-to-template` | Convert to Template | VM | VM context menu | the conversion review |
//! | `vm.cloud-init-drive` | Cloud-init Drive | VM | VM context menu; local row of a VM's Storage tab | the cloud-init drive dialog |
//! | `vm.delete` | Delete VM | VM | VM context menu | the deletion dialog |
//! | `templates.delete` | Delete Template | Templates | template context menu | the deletion dialog |
//!
//! Eligibility is in `commands.rs`. Every command is for ONE object: all
//! are hidden for several (deleting several at once is the `bulk_edit`
//! area's, not ported yet).
//!
//! # Surfaces
//!
//! | Id | Kind | Experimental |
//! | --- | --- | --- |
//! | `vm-lifecycle-dialog` (creation) | wizard: Name, Installation Media, CPU and Memory, Storage, Networking, Cloud-init, Review | yes |
//! | `vm-lifecycle-dialog` (copy, conversion, deletion, the continuation of a cloud creation) | dialog | yes |
//! | `cloud-init-dialog` | dialog | yes |
//!
//! The wizard has the steps the Rune workflow has, under the plan's names
//! and in the plan's order. The plan's "Template" step is not there: the
//! source is the template the command was invoked on (the subtitle names
//! it), and the client has no template picker. Neither is "Home Server":
//! the client sets a home host only by copying a host's BIOS identity,
//! which depends on the firmware chosen on the same page, so that choice
//! stays a group of "CPU and Memory" as it was a section of the Rune step.
//!
//! # Scenes
//!
//! `vm-create`, `lifecycle-permissions`, `firmware-create`, `creation-cloud`,
//! `creation-cloud-pending`, `cloud-init`, `cloud-init-review`.
//!
//! # What has no successor
//!
//! - "Edit template name and description…" (`edit-template`, the Rune kind
//!   `Metadata`, `VmLifecycleOperation::EditTemplate`): name and description
//!   of a custom template are the General page of its Properties dialog
//!   (area `vm_properties`), which Rune offered beside this button.
//! - The "VM changes" history panel and its button (`vm-lifecycle-history`,
//!   `close-vm-lifecycle-history`): the Events tab of the Notifications view
//!   lists every operation with its created objects, steps and controls
//!   (area `events`). The review and status texts that name "VM changes"
//!   and "Activity" are kept word for word (decision 10 of `AREAS.md`).
//! - The headings "n of 3 · Identity" and so on: the wizard frame shows the
//!   steps.
//! - The details-pane section "VM lifecycle" / "Template actions" with its
//!   caption: its buttons are the commands; the caption is the reason a
//!   disabled command gives.
//! - "✓" in front of a chosen list item: the choice shows its selection.
//!
//! # For other areas
//!
//! - [`disk_label`] and [`deletion_review`]: the label of a disk and the
//!   review lines of one deletion, for the batch deletion of `bulk_edit`
//!   (Rune `bulk_delete.rs` called both).
//! - [`open_created`]: the cloud-init drive dialog for a VM that was just
//!   created, with inputs handed over.

use std::collections::BTreeSet;

use xenadmin_core::{Inventory, SnapshotId, VdiId, Vm};

use crate::prelude::*;

pub mod cloud;
pub mod commands;
pub mod creation;
pub mod deletion;
pub mod drive;
pub mod edit;
pub mod scenes;
pub mod view;

pub use drive::open_created;
pub use edit::disk_label;

/// What the surface `vm-lifecycle-dialog` shows.
pub enum Panel {
    /// A change being drafted or reviewed.
    Editor(Box<edit::Editor>),
    /// A submitted creation whose cloud-init inputs wait for the new VM.
    CloudCreation(Box<cloud::Pending>),
}

/// The state of the area. `m.areas.vm_lifecycle`.
#[derive(Default)]
pub struct State {
    /// The lifecycle editor or the continuation of a cloud creation (Rune
    /// `vm_lifecycle`).
    pub panel: Option<Panel>,
    /// The cloud-init drive dialog (Rune `cloud_init`). In front of the
    /// other when both are open.
    pub drive: Option<drive::Panel>,
}

pub fn spec() -> AreaSpec {
    AreaSpec {
        commands: commands::commands,
        scenes: scenes::scenes,
        surfaces: view::surfaces,
        poll: drive::poll,
        on_scope_change: close,
        on_session_change: close,
        ..AreaSpec::new("vm_lifecycle")
    }
}

/// Another object selected in the tree, another pool shown: both surfaces
/// belong to what they captured, so nothing of them is kept. A cloud-init
/// draft is zeroized as it goes, and a late result of the media
/// preparation is discarded.
pub(crate) fn close(m: &mut Model) {
    m.areas.vm_lifecycle = State::default();
}

/// The review lines of the deletion of one VM or template with the chosen
/// disks, snapshots and snapshot disks.
pub fn deletion_review(
    inventory: &Inventory,
    source: &Vm,
    disks: &BTreeSet<VdiId>,
    snapshots: &BTreeSet<SnapshotId>,
    snapshot_disks: &BTreeSet<VdiId>,
) -> Vec<String> {
    deletion::describe(inventory, source, disks, snapshots, snapshot_disks)
}
