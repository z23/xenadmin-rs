//! The logic of the Rune `transfers.rs`: one `Panel` for the import of an
//! image (XVA, checkpoint XVA, OVF/OVA, VHD, raw), the export of a VM,
//! template, snapshot or checkpoint (XVA with compression, OVA, OVF
//! directory) and the export of one disk image.
//!
//! Carried over word for word: `begin`, `current`, `action`, `set_format`,
//! `choose` with its serial guard, `poll`, `apply`, `control`. What changed:
//!
//! - the panel follows the pool it captured, not the scope session;
//! - `step` says which step of the wizard is shown before the review (the
//!   Rune dialog was one column); `review` is still the Rune flag;
//! - `history: bool` became `started`: the operation this wizard submitted,
//!   whose outcome it then shows. The list of every transfer is the Events
//!   tab ([`history`]).

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};

use xenadmin_core::{
    DiskImageFormat, Error, ExportSource, NetworkId, ObservedResource, OperationControl,
    OperationId, OvfPackageFormat, OvfSummary, PoolId, ResourceOperationRecord, ResourceTarget,
    SrId, TransferAction, TransferFile, TransferRequest, VdiId, XvaCompression,
};

use crate::prelude::*;

/// Rune: the caption of a panel whose pool changed under it.
pub const STALE: &str = "Connection changed. Reopen this review using current inventory.";
/// Rune: under an unmapped package network.
pub const UNMAPPED: &str = "Choose the destination network before review.";
/// Rune: the label of the file button while an inspection runs.
pub const INSPECTING: &str = "Inspecting file…";
/// New texts: Rune disabled these buttons without saying why.
pub const DEMO: &str = "Demo mode cannot change infrastructure.";
pub const NO_SOURCE: &str = "Choose an image file first.";
pub const NO_DESTINATION: &str = "Choose a destination first.";
pub const NO_STORAGE: &str = "Choose the destination storage first.";
pub const INCOMPLETE: &str = "Complete the earlier steps first.";
pub const NOT_WRITABLE: &str = "This connection is read-only, disconnected or stale.";
pub const NO_POOL: &str = "Connect to a pool to import an image.";

#[derive(Default)]
pub struct Panel {
    pub show: bool,
    pub(super) serial: u64,
    pub(super) source: Option<ExportSource>,
    pub(super) source_name: String,
    pub(super) disk_export: Option<(VdiId, SrId, u64)>,
    pub(super) disk_picker: bool,
    pub(super) ovf_import: bool,
    pub(super) package_export: Option<OvfPackageFormat>,
    pub(super) compression: XvaCompression,
    pub(super) ovf_preview: Option<OvfSummary>,
    pub(super) networks: BTreeMap<String, NetworkId>,
    pub(super) inspection_cancel: Option<Arc<AtomicBool>>,
    pub(super) format: Option<DiskImageFormat>,
    pub(super) disk_name: String,
    pub(super) pool: Option<ObservedResource>,
    pub(super) pool_id: Option<PoolId>,
    pub(super) sr: Option<SrId>,
    pub(super) file: Option<TransferFile>,
    pub(super) pending: bool,
    pub(super) result: Option<mpsc::Receiver<Result<TransferFile, String>>>,
    pub(super) error: Option<String>,
    pub(super) review: bool,
    /// The step shown before the review: an index into [`steps`].
    pub(super) step: usize,
    /// The operation this wizard submitted.
    pub(super) started: Option<OperationId>,
    pub(super) acknowledge: Option<OperationId>,
}

/// The steps of the wizard. The last is always the review.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Type,
    Source,
    Storage,
    Networking,
    Format,
    Destination,
    Review,
}

impl Step {
    pub fn label(self) -> &'static str {
        match self {
            Step::Type => "Type",
            Step::Source => "Source",
            Step::Storage => "Storage",
            Step::Networking => "Networking",
            Step::Format => "Format",
            Step::Destination => "Destination",
            Step::Review => "Review",
        }
    }
}

pub fn steps(p: &Panel) -> Vec<Step> {
    if exporting(p) {
        vec![Step::Format, Step::Destination, Step::Review]
    } else if p.ovf_import {
        vec![
            Step::Type,
            Step::Source,
            Step::Storage,
            Step::Networking,
            Step::Review,
        ]
    } else {
        vec![Step::Type, Step::Source, Step::Storage, Step::Review]
    }
}

/// The step that is shown.
pub fn step(p: &Panel) -> Step {
    let all = steps(p);
    if p.review || p.started.is_some() {
        Step::Review
    } else {
        all[p.step.min(all.len() - 2)]
    }
}

fn panel(m: &mut Model) -> &mut Panel {
    &mut m.areas.transfers.panel
}

pub fn close(m: &mut Model) {
    cancel_inspection(m);
    let p = panel(m);
    p.show = false;
    p.serial += 1;
    p.pending = false;
    p.result = None;
}

/// Opens the wizard for a pool: an export when `source` is given, else an
/// import. The pool is captured as the Rune `begin` captured the active one.
pub fn begin(m: &mut Model, pool: Option<PoolId>, source: Option<ExportSource>) {
    cancel_inspection(m);
    let serial = m.areas.transfers.panel.serial + 1;
    let inventory = pool.and_then(|id| m.inventory_for(id));
    let pool_id = inventory.as_ref().map(|i| i.pool_id);
    let pool = pool_id.and_then(|id| {
        m.snapshot_for(id)?
            .observe_resource(ResourceTarget::Pool(id))
            .ok()
    });
    let source_name = source
        .as_ref()
        .and_then(|source| {
            let i = inventory.as_ref()?;
            match source {
                ExportSource::Vm { vm, .. } => i.vms.get(vm).map(|v| v.name.clone()),
                ExportSource::Snapshot { snapshot, .. }
                | ExportSource::Checkpoint { snapshot, .. } => {
                    i.snapshots.get(snapshot).map(|v| v.name.clone())
                }
            }
        })
        .unwrap_or_default();
    m.areas.transfers.panel = Panel {
        show: true,
        serial,
        source,
        source_name,
        pool,
        pool_id,
        ..Default::default()
    };
}

/// The Rune `disk_button` press: the export of one disk, with the identity
/// (disk, repository, size) captured now. `false` when the disk has no
/// repository or size.
pub fn begin_disk(m: &mut Model, pool: PoolId, vdi: VdiId) -> bool {
    let Some(i) = m.inventory_for(pool) else {
        return false;
    };
    let Some(d) = i.details.vdis.get(&vdi) else {
        return false;
    };
    let (Some(sr), Some(size)) = (d.sr, d.virtual_size) else {
        return false;
    };
    let name = d.name.clone();
    begin(m, Some(pool), None);
    let p = panel(m);
    p.disk_export = Some((vdi, sr, size));
    p.format = Some(if size > 2040 * 1024 * 1024 * 1024 {
        DiskImageFormat::Raw
    } else {
        DiskImageFormat::Vhd
    });
    p.source_name = name;
    true
}

/// The Rune `import_converted`: the import of a disk image with the file
/// already under inspection and the name "Converted disk".
pub fn import_converted(m: &mut Model, pool: Option<PoolId>, path: PathBuf, format: DiskImageFormat) {
    begin(m, pool, None);
    let wake = m.waker.clone();
    let p = panel(m);
    p.format = Some(format);
    p.disk_name = "Converted disk".into();
    p.pending = true;
    p.step = 1;
    let (sender, receiver) = mpsc::channel();
    match std::thread::Builder::new()
        .name("converted-disk-review".into())
        .spawn(move || {
            let result = TransferFile::disk_source(path, format).map_err(|e| e.to_string());
            let _ = sender.send(result);
            if let Some(wake) = wake {
                wake();
            }
        }) {
        Ok(_) => p.result = Some(receiver),
        Err(_) => {
            p.pending = false;
            p.error = Some("Cannot inspect converted disk".into());
        }
    }
}

/// "Demo" for the pool of the panel.
pub fn synthetic(m: &Model, p: &Panel) -> bool {
    p.pool_id.is_some_and(|pool| m.is_synthetic_pool(pool))
}

pub fn current(m: &Model, p: &Panel) -> bool {
    let Some(pool) = p.pool_id else { return false };
    m.inventory_for(pool).is_some()
        && (m.is_synthetic_pool(pool)
            || p.pool.is_some_and(|p| {
                m.snapshot_for(pool)
                    .and_then(|v| v.observe_resource(p.target()).ok())
                    == Some(p)
            }))
}

pub fn exporting(p: &Panel) -> bool {
    p.source.is_some() || p.disk_export.is_some()
}

pub fn action(p: &Panel) -> Option<TransferAction> {
    if let Some((vdi, expected_sr, expected_size)) = p.disk_export {
        return Some(TransferAction::ExportDisk {
            vdi,
            expected_sr,
            expected_size,
            format: p.format.unwrap_or(DiskImageFormat::Vhd),
        });
    }
    if let Some(source) = p.source.clone() {
        return Some(match p.package_export {
            Some(format) => TransferAction::ExportOvf { source, format },
            None => TransferAction::ExportXva {
                source,
                compression: p.compression,
            },
        });
    }
    if p.ovf_import {
        let summary = p.file.as_ref()?.ovf()?;
        if summary.networks.iter().cloned().collect::<BTreeSet<_>>()
            != p.networks.keys().cloned().collect()
        {
            return None;
        }
        return p.sr.map(|sr| TransferAction::ImportOvf {
            sr,
            networks: p.networks.clone(),
        });
    }
    p.sr.map(|sr| {
        if let Some(format) = p.format {
            TransferAction::ImportDisk {
                sr,
                name: p.disk_name.clone(),
                format,
            }
        } else if p
            .file
            .as_ref()
            .and_then(|f| f.archive())
            .is_some_and(|a| a.memory_images() > 0)
        {
            TransferAction::ImportCheckpointXva { sr }
        } else {
            TransferAction::ImportXva { sr }
        }
    })
}

/// Whether the action the panel describes is available in the current
/// inventory of its pool, and what the core says when it is not. `None`
/// while the panel describes no action.
pub fn availability(m: &Model, p: &Panel) -> Option<(bool, String)> {
    let action = action(p)?;
    let inventory = m.inventory_for(p.pool_id?)?;
    let availability = action.availability(&inventory);
    Some((availability.is_available(), availability.explanation()))
}

/// The package networks that must be mapped: of the inspected package, or
/// of the synthetic one of the demo scene.
pub fn package_networks(p: &Panel) -> Vec<String> {
    p.file
        .as_ref()
        .and_then(|f| f.ovf())
        .or(p.ovf_preview.as_ref())
        .map(|summary| summary.networks.clone())
        .unwrap_or_default()
}

fn no_file(p: &Panel) -> &'static str {
    if p.pending {
        INSPECTING
    } else if exporting(p) {
        NO_DESTINATION
    } else {
        NO_SOURCE
    }
}

/// Why the review cannot be entered: the conditions under which Rune
/// disabled "Review transfer", each with its reason.
pub fn review_blocked(m: &Model, p: &Panel) -> Option<String> {
    if !current(m, p) {
        return Some(STALE.into());
    }
    if p.file.is_none() {
        return Some(no_file(p).into());
    }
    match availability(m, p) {
        Some((true, _)) => None,
        Some((false, explanation)) => Some(explanation),
        None if p.sr.is_none() && !exporting(p) => Some(NO_STORAGE.into()),
        None if p.ovf_import => Some(UNMAPPED.into()),
        None => Some(INCOMPLETE.into()),
    }
}

/// Why the step that is shown cannot be left forwards.
pub fn blocked(m: &Model, p: &Panel) -> Option<String> {
    let last = steps(p).len() - 2;
    match step(p) {
        Step::Source | Step::Destination if p.file.is_none() => {
            return Some(no_file(p).into());
        }
        Step::Storage if p.sr.is_none() => return Some(NO_STORAGE.into()),
        Step::Networking
            if package_networks(p)
                .iter()
                .any(|name| !p.networks.contains_key(name)) =>
        {
            return Some(UNMAPPED.into());
        }
        _ => {}
    }
    if p.step.min(last) < last {
        return None;
    }
    review_blocked(m, p)
}

/// "Next", and from the last step "Review transfer".
pub fn next(m: &mut Model) {
    let p = &m.areas.transfers.panel;
    if p.review || p.started.is_some() || blocked(m, p).is_some() {
        return;
    }
    let last = steps(p).len() - 2;
    let p = panel(m);
    if p.step >= last {
        p.step = last;
        p.review = true;
    } else {
        p.step += 1;
    }
}

pub fn back(m: &mut Model) {
    let p = panel(m);
    if p.started.is_some() {
        return;
    }
    if p.review {
        p.review = false;
        p.step = steps(p).len() - 2;
    } else {
        p.step = p.step.saturating_sub(1);
    }
}

pub fn toggle_picker(m: &mut Model) {
    let p = panel(m);
    p.disk_picker = !p.disk_picker;
}

fn cancel_inspection(m: &mut Model) {
    if let Some(cancel) = panel(m).inspection_cancel.take() {
        cancel.store(true, Ordering::Relaxed);
    }
}

pub fn set_format(m: &mut Model, format: Option<DiskImageFormat>) {
    cancel_inspection(m);
    let p = panel(m);
    p.ovf_import = false;
    p.package_export = None;
    p.compression = Default::default();
    p.networks.clear();
    p.format = format;
    p.file = None;
    p.pending = false;
    p.result = None;
    p.review = false;
    p.serial += 1;
}

/// The choices of the form are locked in the review and after the submit,
/// as the Rune buttons were disabled.
fn locked(m: &Model) -> bool {
    let p = &m.areas.transfers.panel;
    p.review || p.started.is_some()
}

/// The kind of image: `xva`, `ovf`, `vhd` or `raw` (the Rune buttons
/// `transfer-format-*`). A disk export has only the two image formats.
pub fn pick_kind(m: &mut Model, id: &str) {
    if locked(m) {
        return;
    }
    let disk = m.areas.transfers.panel.disk_export.is_some();
    match id {
        "xva" if !disk => set_format(m, None),
        "ovf" if !disk => {
            set_format(m, None);
            panel(m).ovf_import = true;
        }
        "vhd" => set_format(m, Some(DiskImageFormat::Vhd)),
        "raw" => set_format(m, Some(DiskImageFormat::Raw)),
        _ => return,
    }
    let p = panel(m);
    p.step = p.step.min(steps(p).len() - 2);
}

/// The package format of an export: `xva`, `ova` or `ovf`. A checkpoint
/// has only the XVA.
pub fn pick_package(m: &mut Model, id: &str) {
    if locked(m) {
        return;
    }
    let checkpoint = matches!(
        m.areas.transfers.panel.source,
        Some(ExportSource::Checkpoint { .. })
    );
    let format = match id {
        "xva" => None,
        "ova" if !checkpoint => Some(OvfPackageFormat::Ova),
        "ovf" if !checkpoint => Some(OvfPackageFormat::Ovf),
        _ => return,
    };
    set_format(m, None);
    panel(m).package_export = format;
}

pub fn pick_compression(m: &mut Model, label: &str) {
    if locked(m) {
        return;
    }
    let Some(compression) = XvaCompression::ALL
        .into_iter()
        .find(|compression| compression.label() == label)
    else {
        return;
    };
    set_format(m, None);
    panel(m).compression = compression;
}

pub fn pick_memory(m: &mut Model, preserve: bool) {
    if locked(m) {
        return;
    }
    let p = panel(m);
    if let Some(ExportSource::Checkpoint {
        preserve_memory, ..
    }) = &mut p.source
    {
        *preserve_memory = preserve;
    }
    p.review = false;
}

pub fn pick_sr(m: &mut Model, id: &str) {
    if locked(m) {
        return;
    }
    if let Ok(uuid) = id.parse::<uuid::Uuid>() {
        panel(m).sr = Some(SrId::from(uuid));
    }
}

pub fn pick_network(m: &mut Model, name: &str, id: &str) {
    if locked(m) {
        return;
    }
    if let Ok(uuid) = id.parse::<uuid::Uuid>() {
        panel(m)
            .networks
            .insert(name.to_owned(), NetworkId::from(uuid));
    }
}

pub fn edit_name(m: &mut Model, text: &str) {
    if locked(m) {
        return;
    }
    panel(m).disk_name = text.to_owned();
}

/// The file prompt of the form: a save prompt for an export, an open
/// prompt for an import. A result that arrives after the form changed
/// (`serial`) or closed is dropped: it must not create or reserve a file.
pub fn choose(m: &mut Model) {
    let p = &m.areas.transfers.panel;
    if p.pending || p.started.is_some() {
        return;
    }
    let serial = p.serial;
    let export = exporting(p);
    let format = p.format;
    let ovf = p.ovf_import;
    let package = p.package_export;
    let compression = p.compression;
    cancel_inspection(m);
    let cancel = Arc::new(AtomicBool::new(false));
    let p = panel(m);
    p.inspection_cancel = Some(cancel.clone());
    p.networks.clear();
    p.pending = true;
    p.error = None;
    p.file = None;
    p.review = false;
    let reply = move |m: &mut Model, path: Option<PathBuf>| {
        let p = panel(m);
        if p.serial != serial || !p.show {
            return;
        }
        let Some(path) = path else {
            p.pending = false;
            return;
        };
        let (sender, receiver) = mpsc::channel();
        let wake = m.waker.clone();
        let cancel = cancel.clone();
        let spawned = std::thread::Builder::new()
            .name("archive-file-review".into())
            .spawn(move || {
                let result = if export {
                    TransferFile::destination(path)
                } else if ovf {
                    TransferFile::ovf_source_with_cancel(path, &cancel)
                } else if let Some(format) = format {
                    TransferFile::disk_source(path, format)
                } else {
                    TransferFile::source_with_cancel(path, &cancel)
                }
                .map_err(|e| e.to_string());
                let _ = sender.send(result);
                if let Some(w) = wake {
                    w();
                }
            });
        let p = panel(m);
        if spawned.is_ok() {
            p.result = Some(receiver);
        } else {
            p.pending = false;
            p.error = Some("Cannot start file inspection".into());
        }
    };
    let failed = move |m: &mut Model, error: &str| {
        let p = panel(m);
        if p.serial != serial || !p.show {
            return;
        }
        p.pending = false;
        p.error = Some(error.to_owned());
    };
    let folder = package == Some(OvfPackageFormat::Ovf);
    let extension = if package == Some(OvfPackageFormat::Ova) {
        "ova"
    } else {
        format.map_or(compression.extension(), DiskImageFormat::extension)
    };
    let title = if folder {
        "Choose a new OVF package directory name"
    } else if export {
        "Save a new disk or archive image"
    } else {
        "Select a disk or archive image"
    };
    let mut request = if export {
        let name = if folder {
            "guest-ovf".to_owned()
        } else {
            format!("guest.{extension}")
        };
        FileRequest::save(title, name, reply)
    } else {
        FileRequest::open(title, reply)
    };
    if !folder {
        request = request.filter(
            "Disk or archive image",
            if ovf {
                vec!["ovf", "ova"]
            } else if !export && format.is_none() {
                vec!["xva", "gz", "zst", "zstd"]
            } else {
                vec![extension]
            },
        );
    }
    m.choose_file(request.on_failure(failed));
}

pub fn poll(m: &mut Model) -> bool {
    let p = panel(m);
    let Some(rx) = &p.result else {
        return false;
    };
    let value = match rx.try_recv() {
        Ok(v) => v,
        Err(mpsc::TryRecvError::Empty) => return false,
        Err(_) => Err("File inspection stopped".into()),
    };
    p.pending = false;
    p.result = None;
    match value {
        Ok(file) => p.file = Some(file),
        Err(e) => p.error = Some(e),
    }
    true
}

/// "Begin transfer". Every check of the button is made again; an export is
/// allowed on a read-only connection, an import is not.
pub fn apply(m: &mut Model) {
    let p = &m.areas.transfers.panel;
    let Some(action) = action(p) else { return };
    let (Some(pool), Some(pool_id)) = (p.pool, p.pool_id) else {
        return;
    };
    let Some(file) = p.file.clone() else { return };
    if m.resource_operations.len() >= 64
        || !p.review
        || p.started.is_some()
        || !current(m, p)
        || m.is_synthetic_pool(pool_id)
        || (!action.is_export() && !m.can_mutate_pool(pool_id))
        || !m
            .inventory_for(pool_id)
            .is_some_and(|i| action.availability(&i).is_available())
    {
        return;
    }
    let permission = permissions::assess_for(m, pool_id, &action.permission_requirements());
    if let Some(denial) = permissions::denial(&permission) {
        panel(m).error = Some(denial);
        return;
    }
    let result = m
        .connection_for(pool_id)
        .ok_or(Error::Closed)
        .and_then(|c| c.transfer(OperationId::new(), pool, TransferRequest { action, file }));
    match result {
        Ok(op) => {
            let id = m.track_resource(op);
            let p = panel(m);
            p.started = Some(id);
            p.review = false;
            p.file = None;
            p.error = None;
        }
        Err(e) => panel(m).error = Some(e.to_string()),
    }
}

/// Whether a transfer record can be controlled now: the Rune `allowed` of
/// the history, for the pool of the record. A read-only connection may
/// control an export.
pub fn allowed(m: &Model, r: &ResourceOperationRecord) -> bool {
    m.connection_for(r.pool).is_some_and(|c| {
        !c.is_read_only()
            || matches!(&r.action, xenadmin_core::ResourceAction::Transfer(a) if a.is_export())
    }) && m.snapshot_for(r.pool).is_some_and(|v| v.is_current())
        && m.inventory_for(r.pool).is_some()
}

/// Recheck, stop or release: on the connection of the record's pool. A
/// recheck reads existing evidence and never sends archive bytes again.
pub fn control(m: &mut Model, id: OperationId, action: OperationControl) {
    let Some(record) = m.resource_rows.iter().find(|r| r.id == id) else {
        return;
    };
    if !allowed(m, record) {
        return;
    }
    if let Some(c) = m.connection_for(record.pool)
        && let Err(e) = c.control_resource(id, action)
    {
        panel(m).error = Some(e.to_string());
    }
}

/// The first step of the acknowledgement: the second, "Release
/// reservation", is then offered.
pub fn review_acknowledge(m: &mut Model, id: OperationId) {
    let Some(record) = m.resource_rows.iter().find(|r| r.id == id) else {
        return;
    };
    if allowed(m, record) {
        panel(m).acknowledge = Some(id);
    }
}

/// "History": the transfers of the pool in the Events tab. The wizard
/// closes; Rune replaced its form by the list.
pub fn history(m: &mut Model) {
    let pool = m
        .areas
        .transfers
        .panel
        .pool_id
        .or_else(|| m.inventory.as_ref().map(|i| i.pool_id));
    close(m);
    match pool {
        Some(pool) => crate::areas::events::show_history(m, pool),
        None => crate::areas::events::show(m),
    }
}
