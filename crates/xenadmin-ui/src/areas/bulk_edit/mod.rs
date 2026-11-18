//! Area `bulk_edit`: changes to several VMs or custom templates at once:
//! deleting them, and setting one property on all of them.
//!
//! Rune modules: `bulk_delete.rs`, `bulk_properties.rs`,
//! `bulk_properties_model.rs` (and their tests). Tabs: none.
//!
//! # Where everything went
//!
//! | Rune | Here |
//! | --- | --- |
//! | "Delete selection…" (`batch-delete-open`) of the selection bar and of the Templates page; the row menu item "Delete selection…" | commands `vm.delete-several` and `templates.delete-several`: label, menu place and context menu of `vm.delete` / `templates.delete` (area `vm_lifecycle`), which are hidden for several objects as these are for one |
//! | the dialog "Batch VM deletion" (`bulk_delete::dialog`) | the dialog `batch-delete`: a draft (what besides each record is deleted) and its review with the typed confirmation |
//! | "Edit selection…" (`batch-properties-open`); the row menu item "Edit properties for selection…" | the kernel's `vm.properties` / `template.properties` on several objects |
//! | the dialog "Bulk property editor" (`bulk_properties::dialog`) | the page "Bulk properties" ([`PropertyPage::BULK`]) of the dialog `vm-properties` / `template-properties` of several objects: a draft and its review, on the one page |
//! | the picker button `batch-property-picker` and its fourteen buttons | one choice `batch-property`; its options have the Rune keys `batch-property-<Property>` |
//! | the two toggle buttons, the host buttons | the choices `batch-property-toggle` (options `batch-property-toggle-<false\|true>`) and `batch-property-host` |
//!
//! # Commands
//!
//! | Id | Label | Menu | Also | Opens |
//! | --- | --- | --- | --- | --- |
//! | `vm.delete-several` | Delete VM | VM (the place of `vm.delete`) | VM context menu | the dialog `batch-delete` on its draft |
//! | `templates.delete-several` | Delete Template | Templates (the place of `templates.delete`) | template context menu | the same |
//!
//! Both are `Hidden` unless the target is several VMs (several templates);
//! `Disabled` with the refusal of the selection (more than 32, several
//! pools, a row that is gone) or with what no choice in the dialog could fix
//! (a pool that cannot be changed now, a target that cannot be deleted).
//! The packet proposed the multi form of `vm.delete` itself; that id is the
//! `vm_lifecycle` area's, so the batch has ids of its own.
//!
//! # Surfaces and pages
//!
//! | Id | Kind | Experimental |
//! | --- | --- | --- |
//! | `batch-delete` | dialog (large) | yes |
//! | page `bulk` of `vm-properties` / `template-properties` | properties page | yes |
//!
//! The bulk page is offered for every selection of several VMs or of
//! several templates. A selection it cannot edit (more than 32, several
//! pools, a row that changed type) is told so on the page
//! (`batch-properties-refused`), where Rune disabled its button.
//!
//! # Scenes
//!
//! `batch-delete`, `batch-properties`.
//!
//! # What has no successor
//!
//! - A batch of ONE: the Rune editors took 1 to 32 rows. One VM has
//!   `vm.delete` and the pages of its own Properties dialog. The editors
//!   here still accept a target of one (`delete::capture`, `edit::capture`);
//!   no command reaches them with one.
//! - The page rule of `selected` ("Select VMs or custom templates", "Select
//!   VM or custom-template rows first"): there is no Guests or Templates
//!   page; the target's kinds decide, and a foreign kind hides the command.
//! - "Visible" rows: the target is the selection, whatever a filter shows.
//! - The selection bar and its captions ("{n} selected · Shift/Ctrl-click to
//!   select VMs", "… Narrow the selection to at most 32 VMs"): a selection
//!   of more than 32 disables the command with the Rune refusal.
//! - The row menu's capture of scope and selection (`inventory_menu`): the
//!   kernel's context menu acts on the selection it was opened on.
//! - Opening the pool operation history after a batch deletion was
//!   submitted: the Events tab of the Notifications view is shown
//!   (`Model::show_operations`).
//!
//! # Tests (`tests.rs`)
//!
//! All ten Rune tests are ported to the `Harness` under their names: the
//! four of `bulk_delete_tests.rs` and the six of `bulk_properties_tests.rs`
//! (two of them were pure and moved with the logic). Retired with the Rune
//! layout: the two window sizes and "the footer is inside the window";
//! their GPUI halves are listed in `gpui-tests-wanted.md`. The shares of
//! the experimental-notice test of `main.rs` and of the permission-line
//! test of `session_permissions_view.rs` are there too, and seven new
//! tests for the commands, the page, the hooks and a live observation.
//! Not tested: a submission that succeeds (the fixture server serves no
//! `VmWorkflowBatch` or `PropertiesBatch`).
//!
//! # For other areas
//!
//! - [`delete_several`]: opens the batch deletion for a target of several
//!   VMs or templates (what the two commands do); a refusal is in `m.error`.

use crate::prelude::*;

pub mod commands;
pub mod delete;
pub mod edit;
pub mod property;
pub mod scenes;
pub mod view;

#[cfg(test)]
mod tests;

/// The state of the area. `m.areas.bulk_edit`.
#[derive(Default)]
pub struct State {
    /// The batch deletion (Rune `bulk_delete`).
    pub delete: Option<delete::Editor>,
    /// The draft of the page "Bulk properties" (Rune `bulk_properties`): it
    /// lives while that page is shown.
    pub properties: Option<edit::Editor>,
}

pub fn spec() -> AreaSpec {
    AreaSpec {
        commands: commands::commands,
        scenes: scenes::scenes,
        surfaces: view::surfaces,
        property_pages: view::pages,
        property_page: view::page,
        on_property_page: edit::page_event,
        on_scope_change: close,
        on_session_change: close,
        ..AreaSpec::new("bulk_edit")
    }
}

/// Another object selected in the tree, another session: the batch deletion
/// belongs to the selection it captured. The draft of the bulk page follows
/// its dialog, which the kernel closes then (`edit::page_event`).
pub(crate) fn close(m: &mut Model) {
    m.areas.bulk_edit.delete = None;
}

/// Opens the batch deletion for several VMs or several templates, on its
/// draft. A refusal of the selection is in `m.error` and nothing opens.
pub fn delete_several(m: &mut Model, target: &Target) {
    delete::open(m, target);
}
