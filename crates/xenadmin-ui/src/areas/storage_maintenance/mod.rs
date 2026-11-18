//! Area `storage_maintenance`: repair of a repository's host attachments,
//! its read-cache policy, and the storage workflow (repair of shared
//! storage connections, multipathing with storage reconnection).
//!
//! Rune modules: `storage_maintenance_view.rs` (a child of `storage_view`,
//! drawn inside the "Storage administration" dialog) and
//! `storage_graph_view.rs` (the dialog "Storage workflow").
//!
//! | Rune | Command or page | Surface |
//! | --- | --- | --- |
//! | hub section "Repair across hosts" (`repair-pbd-…`, `storage-repair-review`) | `storage.repair` "Repair": Storage menu, context menu of a repository, local row of the Storage tab (order 20) | dialog `storage-repair`: the detached attachments, then the review |
//! | hub button "Repair missing or detached host connections…" (`storage-graph-open`) | the same button, in the dialog `storage-repair` | dialog `storage-graph` on the repository: source and hosts, then the review |
//! | hub section "Provisioning and read caching" (`storage-cache-review`) | page Read Caching (`read-caching`) of the repository's Properties dialog (`storage.properties`, the kernel's) | the page, then the review on the page |
//! | chooser button "Multipathing with storage reconnection…" (`host-multipath-workflow`, in `host_admin`) | `storage.multipath-workflow`, no menu place: the button of the Multipathing page of a server's Properties dialog | dialog `storage-graph` on the server, on its review |
//! | "Restore this connection…" in the workflow's history | a button of the history, as in Rune | the storage area's `storage-change` review (`storage::review_restore`) |
//! | "Open Activity history" | a button of the history | the Events tab of the Notifications view |
//!
//! Every surface and the page carry the experimental notice: storage
//! administration is outside the supported part (`docs/FEATURES.md`; Rune
//! showed the notice for `storage_editor` and `storage_graph`).
//!
//! `storage.detach`, `storage.reattach`, `storage.forget`,
//! `storage.destroy`, `storage.trim` and the attachment commands are the
//! `storage` area's (decision 12 of the port); this area registers none of
//! them.
//!
//! Scenes: `storage-maintenance` (the Repair dialog with both detached
//! attachments chosen), `storage-repair` (its review), `storage-cache` (the
//! Read Caching page on the review of disabling), `storage-graph` (the
//! reviewed repair of shared storage), `storage-multipath` (the reviewed
//! multipath change).
//!
//! For other areas (public functions; an area never reaches into
//! [`State`]):
//!
//! - [`open_repair`]: the Repair dialog on a repository.
//! - [`open_workflow`]: the storage workflow on a repository or a server
//!   (`ResourceTarget::Storage` or `Host`).
//! - [`close`].
//! - The command ids [`commands::REPAIR`] and
//!   [`commands::MULTIPATH_WORKFLOW`].
//!
//! Of other areas this area calls `storage::location_lines`,
//! `storage::review_restore` and `storage::close`, and invokes no command.
//!
//! What has no successor here, and where it went:
//!
//! - The hub ("Storage administration", manage mode) that held the two
//!   sections: the `storage` area took it apart. The repair section is a
//!   dialog of its own, the read-cache section a Properties page.
//! - The stage error "Select a storage repository" of a hub opened on the
//!   pool: this editor exists only for a repository.
//! - The host that Rune's backend query asked for and never used ("Choose a
//!   host"): the capabilities are the pool's.
//! - The preview of a server's proposal before a press on "Review
//!   changes…": the workflow on a server opens on its review. The preview
//!   is still what the dialog shows after a submission and when the review
//!   could not be staged.
//! - The "✓" in the label of the chosen source button: the source is a
//!   choice, and its options keep the buttons' keys.
//!
//! The files: [`maintenance`] and [`graph`] (the logic carried over),
//! [`commands`], [`view`], [`scenes`], `tests`.

use crate::prelude::*;

pub mod commands;
pub mod graph;
pub mod maintenance;
pub mod scenes;
pub mod view;

#[cfg(test)]
mod tests;

pub use graph::open as open_workflow;
pub use maintenance::open_repair;

/// The state of the area. `m.areas.storage_maintenance`.
#[derive(Default)]
pub struct State {
    /// The open repair or read-cache change. A repair is the dialog
    /// `storage-repair`; a read-cache change is the Read Caching page of
    /// the Properties dialog.
    pub editor: Option<maintenance::Editor>,
    /// The open storage workflow: the dialog `storage-graph`.
    pub graph: Option<graph::Editor>,
}

pub fn spec() -> AreaSpec {
    AreaSpec {
        commands: commands::commands,
        scenes: scenes::scenes,
        surfaces: view::surfaces,
        poll: maintenance::poll,
        on_scope_change: close,
        on_session_change: close,
        property_pages: view::pages,
        property_page: view::page,
        on_property_page: maintenance::page_event,
        ..AreaSpec::new("storage_maintenance")
    }
}

/// Close, Escape, a successful repair or policy change, another object
/// selected in the tree, another pool shown: nothing of the editors is
/// kept. An editor that is the Read Caching page takes the Properties
/// dialog with it: the dialog is never left on a page without its editor.
pub fn close(m: &mut Model) {
    let state = &mut m.areas.storage_maintenance;
    state.graph = None;
    let page = state
        .editor
        .take()
        .is_some_and(|e| e.kind == maintenance::Kind::Cache);
    if page {
        m.close_properties();
    }
}
