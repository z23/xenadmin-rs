//! Area `transfers`: import, export and local disk conversion (plan W5).
//!
//! Rune modules: `transfers.rs` ([`panel`], [`view`]), `ovf_transfer_ui.rs`
//! ([`view`], [`scenes`]), `local_conversion_view.rs` ([`conversion`]) and
//! their tests ([`tests`], [`conversion_tests`]). Tabs: none.
//!
//! ## For other areas (public functions)
//!
//! | Function | For |
//! | --- | --- |
//! | [`vm_source`] | the export source of a VM or template object |
//! | [`export_eligibility`], [`export`] | `bulk_export`: the export of ONE VM, template, snapshot or checkpoint (the Rune `export_button`, key `export-xva`); a selection of several is that area's |
//! | [`export_disk_eligibility`], [`export_disk`] | `vm_devices`: the Rune `disk_button` ("Export disk image"); also the command `disk.export-image` |
//! | [`import`] | the import wizard for a pool (Rune `transfers::open`) |
//! | [`import_converted`] | the import of a converted disk image (Rune `transfers::import_converted`) |
//! | [`history`] | `host_services`: the transfers of the scope pool after a host backup or report (Rune `transfers::history`): the Events tab |
//! | [`convert`] | the conversion wizard (Rune `local_conversion_view::open`) |
//!
//! ## Destination
//!
//! | Rune | Here |
//! | --- | --- |
//! | "Archives" (`archives-open`), import form | command `file.import` (File > Import); wizard `transfer-dialog`, steps Type, Source, Storage, Networking (OVF / OVA only), Review |
//! | "Export…" (`export-xva`) of a VM or template | commands `vm.export` (VM > Export) and `templates.export` (Templates > Export to File); wizard `transfer-dialog`, steps Format, Destination, Review |
//! | "Export disk image" (`export-disk-{vdi}`) | command `disk.export-image` (disk context menu, local row of Storage) and, as in Rune, the disk list on the import's first step; wizard `transfer-dialog`, steps Format, Destination, Review |
//! | "Convert local disk…" (`conversion-open`) | command `tools.convert-disk-image` (Tools > Convert Disk Image); wizard `local-conversion`, steps Source and format, Review, Convert |
//! | the review flag of the form | the last step of each wizard; the Rune button `transfer-review` enters it |
//! | history after "Begin transfer" | the wizard stays on its last step and shows the outcome of the transfer it started, with Recheck, Stop transfer, Acknowledge after inspection and Release reservation |
//! | "History" of the dialog, `transfers::history` | button `transfer-history` and [`history`]: the Events tab of the pool |
//!
//! Every surface of the area is experimental (AREAS.md decision 1; Rune
//! `experimental_dialog_open` had both flags set).
//!
//! ## What has no successor
//!
//! - The list of EVERY transfer record in the transfer dialog: the Events
//!   tab lists them. The wizard shows only the transfer it started. The
//!   transfer-specific lines of a record (bytes and phase, the retained
//!   UUIDs) and the two-step "Release reservation" exist only for that one;
//!   Events has its own acknowledgement of a resource operation.
//! - "Import image" of the history view: File > Import.
//! - The disabled copies of the choices in the review: the review lists
//!   what was chosen as lines.
//! - `conversion-heading`: the title of the surface.
//!
//! ## Scenes
//!
//! `archive-export`, `xva-compression`, `checkpoint-export`, `ovf-import`,
//! `ovf-export`, `disk-export`, `local-conversion`,
//! `local-conversion-review`.

use std::path::PathBuf;

use xenadmin_core::{DiskImageFormat, ExportSource, PoolId, TransferAction, VdiId};

use crate::prelude::*;

pub mod commands;
pub mod conversion;
pub mod panel;
pub mod scenes;
pub mod view;

#[cfg(test)]
mod conversion_tests;
#[cfg(test)]
mod tests;

/// The state of the area: the transfer wizard and the conversion
/// workspace, both always present (`show`). `m.areas.transfers`.
#[derive(Default)]
pub struct State {
    pub panel: panel::Panel,
    pub conversion: conversion::Workspace,
}

pub fn spec() -> AreaSpec {
    AreaSpec {
        commands: commands::commands,
        scenes: scenes::scenes,
        surfaces: view::surfaces,
        poll,
        on_scope_change: scope_changed,
        on_session_change: session_changed,
        ..AreaSpec::new("transfers")
    }
}

fn poll(m: &mut Model, _clock: f64) -> bool {
    let inspected = panel::poll(m);
    let converted = conversion::poll(m);
    inspected || converted
}

/// An export belongs to the object it was opened for and closes with it;
/// an import belongs to its pool and a submitted transfer to its record.
fn scope_changed(m: &mut Model) {
    let p = &m.areas.transfers.panel;
    if p.show && panel::exporting(p) && p.started.is_none() {
        panel::close(m);
    }
}

/// The pool of the wizard is going: Rune kept the dialog open and relied
/// on its `current` check. The conversion needs no connection and stays.
fn session_changed(m: &mut Model) {
    if m.areas.transfers.panel.show {
        panel::close(m);
    }
}

/// The export source of a VM or template object: `None` for another kind,
/// an object its pool does not list, and a control domain.
pub fn vm_source(m: &Model, object: &ObjectRef) -> Option<(PoolId, ExportSource)> {
    if !matches!(object.kind, ObjectKind::Vm | ObjectKind::Template) {
        return None;
    }
    let pool = object.pool?;
    let id = object.vm_id()?;
    let inventory = m.inventory_for(pool)?;
    let vm = inventory.vms.get(&id).filter(|v| !v.is_control_domain)?;
    Some((
        pool,
        ExportSource::Vm {
            vm: id,
            expected_template: vm.is_template,
        },
    ))
}

/// Whether one source can be exported: the rule of the Rune "Export…"
/// button, with what the core says where Rune only disabled it. `Hidden`
/// when the pool is not shown.
pub fn export_eligibility(m: &Model, pool: PoolId, source: &ExportSource) -> Eligibility {
    let Some(inventory) = m.inventory_for(pool) else {
        return Eligibility::Hidden;
    };
    let availability = TransferAction::ExportXva {
        compression: Default::default(),
        source: source.clone(),
    }
    .availability(&inventory);
    Eligibility::enabled_if(
        m.is_synthetic_pool(pool) || availability.is_available(),
        availability.explanation(),
    )
}

/// Opens the export wizard for one VM, template, snapshot or checkpoint.
/// `false` when [`export_eligibility`] refuses; the reason is in `m.error`.
pub fn export(m: &mut Model, pool: PoolId, source: ExportSource) -> bool {
    match export_eligibility(m, pool, &source) {
        Eligibility::Enabled => {
            panel::begin(m, Some(pool), Some(source));
            true
        }
        Eligibility::Disabled(reason) => {
            m.error = Some(reason);
            false
        }
        Eligibility::Hidden => false,
    }
}

/// Whether a disk can be exported as an image: the rule of the Rune
/// "Export disk image" button. `Hidden` for a disk without a repository
/// or a size.
pub fn export_disk_eligibility(m: &Model, pool: PoolId, vdi: VdiId) -> Eligibility {
    let Some(inventory) = m.inventory_for(pool) else {
        return Eligibility::Hidden;
    };
    let Some((sr, size)) = inventory
        .details
        .vdis
        .get(&vdi)
        .and_then(|d| d.sr.zip(d.virtual_size))
    else {
        return Eligibility::Hidden;
    };
    let availability = TransferAction::ExportDisk {
        vdi,
        expected_sr: sr,
        expected_size: size,
        format: DiskImageFormat::Raw,
    }
    .availability(&inventory);
    Eligibility::enabled_if(
        m.is_synthetic_pool(pool) || availability.is_available(),
        availability.explanation(),
    )
}

/// Opens the export of one disk image. `false` when
/// [`export_disk_eligibility`] refuses; the reason is in `m.error`.
pub fn export_disk(m: &mut Model, pool: PoolId, vdi: VdiId) -> bool {
    match export_disk_eligibility(m, pool, vdi) {
        Eligibility::Enabled => panel::begin_disk(m, pool, vdi),
        Eligibility::Disabled(reason) => {
            m.error = Some(reason);
            false
        }
        Eligibility::Hidden => false,
    }
}

/// Opens the import wizard for a pool.
pub fn import(m: &mut Model, pool: PoolId) {
    panel::begin(m, Some(pool), None);
}

/// Opens the import of a converted disk image into the pool of the scope
/// session, with the file already under inspection.
pub fn import_converted(m: &mut Model, path: PathBuf, format: DiskImageFormat) {
    let pool = m.inventory.as_ref().map(|i| i.pool_id);
    panel::import_converted(m, pool, path, format);
}

/// Shows the transfers of the scope pool: the Events tab.
pub fn history(m: &mut Model) {
    panel::history(m);
}

/// Opens the local conversion wizard.
pub fn convert(m: &mut Model) {
    conversion::open(m);
}
