//! Area `bulk_export`: several VMs, templates or snapshots exported in one
//! reviewed batch, each to an archive of its own (plan W5).
//!
//! Rune modules: `bulk_export.rs`, `bulk_export_snapshots.rs` and their
//! tests. Tabs: none.
//!
//! ## For other areas (public functions)
//!
//! | Function | For |
//! | --- | --- |
//! | [`open`] | the batch export wizard on a target of several VMs or templates of one pool (the Rune `bulk_export::open`) |
//! | [`open_snapshots`] | the export of the snapshots selected on the Snapshots tab (the Rune `snapshots::open`); one snapshot is handed to `transfers::export` |
//! | [`commands::EXPORT_VMS`], [`commands::EXPORT_TEMPLATES`], [`commands::EXPORT_SNAPSHOTS`] | the command ids |
//!
//! ## Destination
//!
//! | Rune | Here |
//! | --- | --- |
//! | "Export selection…" (`batch-export-open`) of the Guests selection bar and row menu | command `vm.export-selection`, label "Export", VM > Export (the place of `vm.export`) and the context menu of a VM; `Hidden` for one object, where `vm.export` of `transfers` shows |
//! | "Export selection…" of the Templates page and its row menu | command `templates.export-selection`, label "Export to File", Templates > Export to File (the place of `templates.export`) and the context menu of a template; `Hidden` for one template |
//! | "Export selection…" (`snapshot-batch-export-open`) and "Export…" (`export-xva`) of the Snapshots page | command `snapshot.export`, no menu: the button `snapshot-batch-export-open` of the Snapshots tab (the `snapshots` area draws it). One selected snapshot opens the single export of `transfers` (`transfer-dialog`), several open the batch |
//! | the dialog "Selected VM exports" / "Selected snapshot exports" | wizard `batch-export`, steps Format, Destinations, Review exports. The Rune form is the first two steps, its review the last |
//! | the pool history opened after the submission | the wizard closes and the History list of the Events tab is shown (`Model::show_operations`) |
//!
//! | Surface | Id | Kind | Experimental |
//! | --- | --- | --- | --- |
//! | Export selected VMs / snapshots | `batch-export` | wizard, large | yes |
//!
//! Experimental by decision 1 of AREAS.md: Rune's
//! `experimental_dialog_open` had the flag of `bulk_export` set.
//!
//! ## Rules kept
//!
//! - 1 to 32 targets, VMs and templates or snapshots, never mixed; a
//!   control domain is refused. The commands of several VMs start at two
//!   (one is the single export).
//! - The editor follows the pool of its targets. It is current only through
//!   a connection whose pool observation is the captured one; a read-only
//!   connection may export; a synthetic pool only previews.
//! - The proposal is made again on every render and at the submission, and
//!   stops at the first target that fails. No output path may be a prefix
//!   of another. A known permission denial is a refusal of the proposal.
//! - A destination is asked for one target at a time, and its answer is
//!   taken only by the editor identity, format, target and pending index it
//!   was asked for. A format change renews the identity and drops every
//!   destination. A destination is inspected on a thread; `poll` collects.
//! - A checkpoint in the selection leaves the XVA formats only. Memory is
//!   left out until chosen for each checkpoint; the choice ends a review
//!   and keeps the destinations.
//! - The submission refuses with `Stale` unless the editor is current, the
//!   review is what would be reviewed now and fewer than 64 resource
//!   operations are held. VMs and snapshots are two calls of the core.
//!
//! ## What has no successor
//!
//! - `batch-export-open` as a button: the commands above (menu, context
//!   menu). The caption of the selection bar belongs to the grid.
//! - The batch of ONE VM or template: the single export wizard of
//!   `transfers` has the same formats. One selected snapshot likewise.
//! - The mixed selection of VMs and templates is `Hidden` for both commands
//!   (a Rune page held one kind).
//! - The pool history's hiding of a batch's children: the History list of
//!   `events` shows every record (that area's decision); a child's controls
//!   are still refused with "Controlled by batch …".
//!
//! ## Scenes
//!
//! `batch-export`, `snapshot-batch-export`.
//!
//! The files, in the order to read them: [`export`] (the two Rune modules
//! without their views), [`commands`], [`view`], [`scenes`], `tests`.

use crate::areas::{AreaSpec, transfers};
use crate::prelude::*;

pub mod commands;
pub mod export;
pub mod scenes;
pub mod view;

#[cfg(test)]
mod tests;

/// The state of the area: the one open editor. `m.areas.bulk_export`.
#[derive(Default)]
pub struct State {
    pub editor: Option<export::Editor>,
}

pub fn spec() -> AreaSpec {
    AreaSpec {
        commands: commands::commands,
        scenes: scenes::scenes,
        surfaces: view::surfaces,
        poll: export::poll,
        on_scope_change: export::close,
        on_session_change: export::close,
        ..AreaSpec::new("bulk_export")
    }
}

/// Opens the batch export wizard on the VMs or templates of a target (the
/// Rune `bulk_export::open`). `false`, with the refusal of the selection in
/// `m.error`, when the target is not 1 to 32 ordinary VMs or custom
/// templates of one shown pool.
pub fn open(m: &mut Model, target: &Target) -> bool {
    match export::selected(m, target) {
        Ok((pool, vms)) => {
            export::begin(m, pool, vms.into_iter().map(export::Item::Vm).collect())
        }
        Err(reason) => {
            m.error = Some(reason);
            false
        }
    }
}

/// Opens the export of the snapshots selected on the Snapshots tab of the
/// shown VM (the Rune `snapshots::open`): the single export of `transfers`
/// for one snapshot (the Rune "Export…" of that page), the batch wizard for
/// several. `false`, with the reason in `m.error`, when the selection is
/// refused.
pub fn open_snapshots(m: &mut Model) -> bool {
    match export::selected_snapshots(m) {
        Ok((pool, targets)) => {
            if let ([one], Some(inventory)) = (targets.as_slice(), m.inventory_for(pool)) {
                let source = export::single_source(&inventory, one);
                return transfers::export(m, pool, source);
            }
            let items = targets.into_iter().map(export::Item::Snapshot).collect();
            export::begin(m, pool, items)
        }
        Err(reason) => {
            m.error = Some(reason);
            false
        }
    }
}
