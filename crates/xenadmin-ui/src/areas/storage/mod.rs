//! Area `storage`: storage repositories of a pool. The Storage tab of a pool
//! or a server, the New Storage Repository wizard, and the reviewed changes
//! of one repository.
//!
//! Rune modules: `storage_view.rs`, `storage_local_view.rs` (and the tests
//! of `storage_provisioning_view_tests.rs`).
//!
//! The Rune module was one dialog, "Storage administration": the form of a
//! new repository, and for an existing one a hub of buttons ("Storage
//! attachments and lifecycle…"). The hub is gone: its information is the
//! Storage tab of the repository (areas `vm_devices`, `general`), its
//! buttons are commands.
//!
//! | Rune | Command | Surface |
//! | --- | --- | --- |
//! | "New storage" (`create-storage`) | `storage.new` "New Storage": Storage menu, toolbar, context menu of a pool or server | wizard `storage-new` (Type, Name, Location, Review) |
//! | "Reclaim unused blocks via host …" | `storage.trim` "Reclaim Freed Space" | dialog `storage-change`: the hosts, then the review |
//! | "Detach from host…" | `storage.detach` "Detach" | dialog `storage-change`, on the review |
//! | "Attach / repair…" | `storage.reattach` "Reattach" | dialog `storage-change`, on the review |
//! | "Create missing host attachment…" | `storage.attachment.create` "Create Missing Host Attachment" | dialog `storage-attachment`: form, then review |
//! | "Remove detached attachment…" | `storage.attachment.remove` "Remove Detached Attachment" | dialog `storage-change`, on the review |
//! | "Forget repository…" | `storage.forget` "Forget" | dialog `storage-change`, on the review |
//! | "Destroy repository and data…" | `storage.destroy` "Destroy" | dialog `storage-change`, on the review |
//! | "Review unused Secret cleanup…" | none: a button inside every surface of the area, as in Rune | dialog `storage-secret-cleanup` |
//!
//! Every command on a repository is in the Storage menu, the context menu
//! of a repository and the local row of the Storage tab. Every surface
//! carries the experimental notice: storage administration is outside the
//! supported part (`docs/FEATURES.md`).
//!
//! The seven commands on a repository are registered HERE although the
//! plan's package gives Detach, Reattach, Forget and Destroy to
//! `storage_maintenance`: their code is `storage_view.rs` (`removal`,
//! `attachment_action`), they share this editor's `stage`, `apply`,
//! `describe` and typed confirmation, and cannot be separated from it.
//! `storage_maintenance` must not register these ids again.
//!
//! Tab: Storage of a pool (table `storage-repositories-table`, one row per
//! repository) and of a server (table `storage-pbds`, one row per attachment
//! of the server; the row is the repository, its id is the attachment). The
//! rows are tree objects: a secondary press opens the kernel's context
//! menu, the local row acts on the selected row. A VM's and a repository's
//! Storage tab are not this area's: `tab` returns `None`.
//!
//! Scenes: `storage-create`, `storage-review`, `storage-discovery`,
//! `storage-auth-review`, `storage-hba`, `storage-hba-review`,
//! `storage-local`, `storage-local-review`.
//!
//! For other areas (public functions; an area never reaches into
//! [`State`]):
//!
//! - [`location_lines`]: the review lines of a storage location (the
//!   storage workflow and the repair review of `storage_maintenance`).
//! - [`review_restore`]: "Restore this connection…" of the storage
//!   workflow's history (`storage.restore-connection`).
//! - [`review_removal`], [`review_attachment`], [`open_attachment`],
//!   [`open_trim`], [`open_new`]: what this area's commands call.
//! - [`close`].
//!
//! Commands of other areas on this area's grid (the local row and the
//! context menu are generated from the registry; this area's sources do
//! not open them): `storage.repair` (`storage_maintenance`; the hub's
//! "Repair missing or detached host connections…", key
//! `storage-graph-open`), `storage.set-default` and `storage.rescan`
//! (`network`: the generic editor of `resource_admin.rs`). They are
//! registered with `.local(TabId::Storage, order)` and
//! `.context(&[ObjectKind::StorageRepository])`, at the orders
//! `commands.rs` left free for them, before this area's own.
//!
//! What has no successor here, and where it went:
//!
//! - The hub and its button `storage-manage`; the heading `storage-heading`
//!   (the title of each surface).
//! - The hub's list of host attachments with multipathing and paths: the
//!   Storage tab of the repository (`vm_devices`, today `general`).
//! - `maintenance::controls` (repair across hosts, provisioning and read
//!   caching), the editor fields `maintenance_review` and
//!   `repair_selection`, and the scenes `storage-maintenance`,
//!   `storage-repair`, `storage-cache`: `storage_maintenance`.
//! - A General page in the repository's Properties dialog: none from this
//!   area. These modules never changed the name or description of an
//!   existing repository (Rune: `resource_admin`'s Rename, area `network`).
//!
//! The files: [`editor`] (the logic carried over), [`commands`], [`view`],
//! [`scenes`], `tests`.

use crate::prelude::*;

pub mod commands;
pub mod editor;
pub mod scenes;
pub mod view;

#[cfg(test)]
mod tests;

pub use editor::{
    location_lines, open_attachment, open_new, open_trim, review_attachment, review_removal,
    review_restore,
};

/// The state of the area. `m.areas.storage`.
#[derive(Default)]
pub struct State {
    /// The open storage change. `Some` means its surface is open.
    pub editor: Option<editor::Editor>,
    /// The selected row of the Storage tab's grid: the UUID of a repository
    /// on a pool, the id of an attachment on a server.
    pub selected: Option<String>,
}

pub fn spec() -> AreaSpec {
    AreaSpec {
        commands: commands::commands,
        scenes: scenes::scenes,
        surfaces: view::surfaces,
        tab: view::tab,
        poll: editor::poll,
        on_scope_change: leave,
        on_session_change: leave,
        ..AreaSpec::new("storage")
    }
}

/// Close, Escape, a successful submission: nothing of the editor is kept.
pub fn close(m: &mut Model) {
    m.areas.storage.editor = None;
}

/// Another object selected in the tree, another pool shown: the editor and
/// the grid's selection belonged to the previous one.
fn leave(m: &mut Model) {
    close(m);
    m.areas.storage.selected = None;
}
