//! `bulk_export.rs` and `bulk_export_snapshots.rs` without their views: the
//! editor of a batch export of 2 to 32 VMs or templates, or of snapshots of
//! one VM, each written to an archive of its own.
//!
//! The selection rules, the per-target checks, the proposal, the binding of
//! a chosen destination to the editor it was chosen for, the memory choice
//! of each checkpoint and the submission are the Rune code. What changed:
//!
//! - the Rune `Target` is [`Item`] here (`Target` is the command target);
//! - the selection is the command's target (VMs and templates) or the
//!   selected rows of the Snapshots tab (`snapshots::selected_snapshots`);
//! - the editor follows the pool of its targets, not the scope session;
//! - the flat Rune form is two steps before the review ([`Step`]).

use std::{path::PathBuf, sync::mpsc};

use uuid::Uuid;
use xenadmin_core::{
    BatchVmWorkflow, CheckpointMemory, Error, ExportSource, Inventory, ObservedResource,
    OperationId, OvfPackageFormat, PoolId, PowerState, ResourceTarget, Snapshot, SnapshotId,
    TransferAction, TransferFile, Vm, VmExportFormat, VmId, VmWorkflow, VmWorkflowBatch,
    XvaCompression,
};

use crate::areas::snapshots;
use crate::prelude::*;

pub const KINDS: &str = "Select ordinary VMs or custom templates";
pub const NO_INVENTORY: &str = "Current inventory is unavailable";
pub const LIMIT: &str = "Select 1–32 visible VMs or custom templates";
pub const NOT_ORDINARY: &str = "A selected row is no longer an ordinary VM or custom template";
/// A selection across pools: the batch is one command to one pool. (New
/// text: a Rune selection was always of one pool.)
pub const ONE_POOL: &str = "Select VMs or templates of one pool to export them together";
pub const NO_VIEW: &str = "Open the snapshots of a VM in the current inventory";
pub const SNAPSHOT_LIMIT: &str = "Select 1–32 snapshots of this VM";
pub const NOT_A_SNAPSHOT: &str = "A selected row is no longer a snapshot of this VM";
pub const NOT_CURRENT_POOL: &str = "Captured pool is no longer current";
pub const OVERLAP: &str =
    "Output paths overlap; choose a distinct new file or directory for every target";
pub const WAITING: &str = "Waiting for destination selection or inspection…";
pub const PREVIEW: &str = "Synthetic preview; transfers are disabled.";
pub const STALE: &str = "The source connection changed. Reopen this review from current inventory.";
/// Why the submit button waits when the inventory moved on under the
/// review. (New text: Rune disabled the button without a word.)
pub const CHANGED: &str =
    "The exports that would be reviewed now differ from this review. Go back and review again.";
pub const PROMPT: &str = "Choose a new export file or package directory";
pub const NO_INSPECTION: &str = "Cannot inspect the selected destination";
pub const INSPECTION_STOPPED: &str = "Destination inspection stopped";
pub const XVA_ONLY: &str = "A memory checkpoint is exported as XVA only, so OVF and OVA are not offered for this selection.";
pub const VM_SEQUENCE: &str = "Every source and destination is checked before the first transfer. Exports run serially without stopping or changing guests. Stop finishes the current archive and skips the remainder. Recovery reads existing evidence; it never starts another transfer or publishes a new output. Existing files and directories are never overwritten.";
pub const VM_FORMATS: &str = "XVA preserves native guest configuration and disks; full XVA does not include snapshot ancestry. OVF/OVA uses the supported ordinary HVM subset. Snapshots and memory checkpoints are exported from a VM's Snapshots view. Read-only connections may export, subject to host permission.";
pub const SNAPSHOT_SEQUENCE: &str = "Every snapshot and destination is checked before the first transfer. Exports run one at a time and change neither the snapshots nor the VM. Stop finishes the current archive and skips the remainder. Recovery reads existing evidence; it never starts another transfer or publishes a new output. Existing files and directories are never overwritten.";
pub const SNAPSHOT_FORMATS: &str = "Each archive is one independent guest: snapshot ancestry is not included. A checkpoint's archive holds guest memory only where Include guest memory is chosen for that checkpoint. OVF/OVA uses the supported ordinary HVM subset and cannot hold a checkpoint. Read-only connections may export, subject to host permission.";
pub const MEMORY_INCLUDED: &str = "Memory: included. The archive holds guest RAM, which may contain credentials and other sensitive data. Import creates a suspended guest; CPU, hypervisor and guest compatibility must be checked before a separate resume.";
pub const MEMORY_LEFT_OUT: &str = "Memory: left out. Only configuration and disks are exported; import creates a halted guest.";

/// The step of the wizard before the review. The review itself is
/// `Editor::review`: while it is there the wizard shows its last step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Format,
    Destinations,
}

pub struct Editor {
    /// The identity of the editor, renewed on a format change: a
    /// destination chosen for an earlier identity is dropped.
    pub(crate) id: Uuid,
    pub(crate) pool: PoolId,
    pub(crate) pool_name: String,
    pub(crate) observed: Option<ObservedResource>,
    pub(crate) targets: Vec<Item>,
    pub(crate) format: VmExportFormat,
    /// One per target.
    pub(crate) files: Vec<Option<(PathBuf, TransferFile)>>,
    /// The target whose destination is being chosen or inspected.
    pub(crate) pending: Option<usize>,
    pub(crate) result: Option<mpsc::Receiver<Result<(PathBuf, TransferFile), String>>>,
    pub(crate) step: Step,
    pub(crate) review: Option<Review>,
    pub(crate) error: Option<String>,
}

impl Editor {
    pub(crate) fn of_snapshots(&self) -> bool {
        matches!(self.targets.first(), Some(Item::Snapshot(_)))
    }
}

/// What one item of the editor exports (the Rune `Target`). An editor holds
/// VMs or snapshots, never both.
#[derive(Clone)]
pub enum Item {
    Vm(Vm),
    Snapshot(SnapshotTarget),
}

impl Item {
    /// The UUID of the exported object. Its output is bound to it.
    pub(crate) fn id(&self) -> Uuid {
        match self {
            Self::Vm(vm) => vm.id.uuid(),
            Self::Snapshot(target) => target.snapshot.id.uuid(),
        }
    }

    pub(crate) fn name(&self) -> &str {
        match self {
            Self::Vm(vm) => &vm.name,
            Self::Snapshot(target) => &target.snapshot.name,
        }
    }

    /// The batch item of this target against the inventory as it is now.
    fn workflow(&self, i: &Inventory, format: &VmExportFormat) -> Result<BatchVmWorkflow, String> {
        match self {
            Self::Vm(captured) => vm_item(i, captured, format),
            Self::Snapshot(captured) => captured.item(i, format),
        }
    }
}

#[derive(Clone, PartialEq)]
pub struct Review {
    pub(crate) batch: VmWorkflowBatch,
    /// One output for each item of the batch, in its order.
    pub(crate) outputs: Vec<TransferFile>,
    pub(crate) paths: Vec<String>,
}

/// A snapshot as it was when the editor opened, with the choice made for it there.
#[derive(Clone)]
pub struct SnapshotTarget {
    pub(crate) snapshot: Snapshot,
    pub(crate) parent: VmId,
    pub(crate) parent_name: String,
    /// The memory image of a checkpoint, as the inventory had it when the editor opened.
    pub(crate) memory: Option<CheckpointMemory>,
    /// Whether the archive is to hold that memory. Off until it is chosen for this
    /// checkpoint; no choice made for another item changes it.
    pub(crate) preserve_memory: bool,
}

impl SnapshotTarget {
    pub(crate) fn capture(i: &Inventory, parent: &Vm, snapshot: &Snapshot) -> Self {
        let memory = match ExportSource::checkpoint(i, snapshot.id, parent.id, false) {
            Some(ExportSource::Checkpoint { memory, .. }) => Some(memory),
            _ => None,
        };
        Self {
            snapshot: snapshot.clone(),
            parent: parent.id,
            parent_name: parent.name.clone(),
            memory,
            preserve_memory: false,
        }
    }

    pub(crate) fn is_checkpoint(&self) -> bool {
        self.memory.is_some() || self.snapshot.power_state == PowerState::Suspended
    }

    pub(crate) fn kind(&self) -> &'static str {
        if self.memory.is_some() {
            "Memory checkpoint"
        } else if self.is_checkpoint() {
            "Memory checkpoint without a memory image to review"
        } else {
            "Disk snapshot"
        }
    }

    /// The source the single export of this snapshot would take from the inventory as it
    /// is now. None when a checkpoint's memory image is not the one that was captured.
    fn source(&self, i: &Inventory) -> Option<ExportSource> {
        match &self.memory {
            None => Some(ExportSource::Snapshot {
                snapshot: self.snapshot.id,
                parent: self.parent,
            }),
            Some(captured) => {
                ExportSource::checkpoint(i, self.snapshot.id, self.parent, self.preserve_memory)
                    .filter(|source| {
                        matches!(source, ExportSource::Checkpoint { memory, .. } if memory == captured)
                    })
            }
        }
    }

    pub(crate) fn item(
        &self,
        i: &Inventory,
        format: &VmExportFormat,
    ) -> Result<BatchVmWorkflow, String> {
        let name = format!("{} ({})", self.snapshot.name, self.snapshot.id);
        if i.snapshots.get(&self.snapshot.id).is_none_or(|now| {
            now.vm != Some(self.parent) || now.power_state != self.snapshot.power_state
        }) {
            return Err(format!("{name}: identity or state changed"));
        }
        let source = self.source(i).ok_or_else(|| {
            format!("{name}: the checkpoint's memory image changed or is no longer available")
        })?;
        let status = action(source.clone(), format).availability(i);
        if !status.is_available() {
            return Err(format!("{name}: {}", status.explanation()));
        }
        BatchVmWorkflow::snapshot_export(&source, format.clone())
            .ok_or_else(|| format!("{name}: not a snapshot"))
    }

    /// The lines under a snapshot's heading: what it is and, for a
    /// checkpoint, its memory image.
    pub(crate) fn lines(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "{} · {}{}",
            self.snapshot.id,
            self.kind(),
            self.snapshot
                .created_at
                .as_deref()
                .map(|created| format!(" · {created}"))
                .unwrap_or_default()
        )];
        if let Some(memory) = &self.memory {
            lines.push(format!(
                "Checkpoint memory image: {} · {} bytes · storage {}",
                memory.vdi, memory.virtual_size, memory.sr
            ));
        }
        lines
    }
}

/// The source of the single export of one snapshot, as the Rune "Export…"
/// button of the Snapshots page built it: a checkpoint with its memory
/// image left out until the wizard asks, else a disk snapshot.
pub(crate) fn single_source(i: &Inventory, target: &SnapshotTarget) -> ExportSource {
    ExportSource::checkpoint(i, target.snapshot.id, target.parent, false).unwrap_or(
        ExportSource::Snapshot {
            snapshot: target.snapshot.id,
            parent: target.parent,
        },
    )
}

/// The VMs and templates of a target, as their pool's inventory has them
/// now, in the target's order: the Rune `selected`. The refusal is the Rune
/// text.
pub fn selected(m: &Model, target: &Target) -> Result<(PoolId, Vec<Vm>), String> {
    if !target
        .objects
        .iter()
        .all(|object| matches!(object.kind, ObjectKind::Vm | ObjectKind::Template))
    {
        return Err(KINDS.into());
    }
    if !(1..=32).contains(&target.len()) {
        return Err(LIMIT.into());
    }
    let pool = target.pool().ok_or(ONE_POOL)?;
    let i = m.inventory_for(pool).ok_or(NO_INVENTORY)?;
    let vms = target
        .vms()
        .into_iter()
        .map(|id| {
            i.vms
                .get(&id)
                .filter(|v| !v.is_control_domain)
                .cloned()
                .ok_or_else(|| NOT_ORDINARY.to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((pool, vms))
}

/// The snapshots selected on the Snapshots tab, in the table's order, as
/// the inventory has them now: the Rune `snapshots::selected`.
pub fn selected_snapshots(m: &Model) -> Result<(PoolId, Vec<SnapshotTarget>), String> {
    let (pool, vm) = snapshots::shown_vm(m).ok_or(NO_VIEW)?;
    let i = m.inventory_for(pool).ok_or(NO_INVENTORY)?;
    let parent = i.vms.get(&vm).ok_or(NO_VIEW)?;
    let rows = snapshots::selected_snapshots(m);
    if !(1..=32).contains(&rows.len()) {
        return Err(SNAPSHOT_LIMIT.into());
    }
    let targets = rows
        .iter()
        .map(|row| capture_snapshot(&i, parent, row.id))
        .collect::<Result<Vec<_>, _>>()?;
    Ok((pool, targets))
}

fn capture_snapshot(i: &Inventory, parent: &Vm, id: SnapshotId) -> Result<SnapshotTarget, String> {
    i.snapshots
        .get(&id)
        .filter(|now| now.vm == Some(parent.id))
        .map(|now| SnapshotTarget::capture(i, parent, now))
        .ok_or_else(|| NOT_A_SNAPSHOT.to_owned())
}

/// The given snapshots of a VM, in the given order: what a scene or a test
/// opens the editor on.
pub(crate) fn snapshots_of(
    m: &Model,
    pool: PoolId,
    parent: VmId,
    ids: &[SnapshotId],
) -> Result<Vec<SnapshotTarget>, String> {
    let i = m.inventory_for(pool).ok_or(NO_INVENTORY)?;
    let parent = i.vms.get(&parent).ok_or(NO_VIEW)?;
    ids.iter()
        .map(|id| capture_snapshot(&i, parent, *id))
        .collect()
}

/// Why no choice in the wizard could make the export of a captured
/// selection go ahead: in every format it offers a target is refused (the
/// line of the first format, XVA), or the session may not export. `None`:
/// the command can open it. A synthetic pool always opens its preview.
pub(crate) fn hopeless(m: &Model, pool: PoolId, items: &[Item]) -> Option<String> {
    if m.is_synthetic_pool(pool) {
        return None;
    }
    let i = m.inventory_for(pool)?;
    let mut first = None;
    for (_, _, format) in offered(items) {
        match items
            .iter()
            .map(|item| item.workflow(&i, &format))
            .collect::<Result<Vec<_>, _>>()
        {
            Ok(items) => {
                let batch = VmWorkflowBatch { items };
                return permissions::denial(&permissions::assess_for(
                    m,
                    pool,
                    &batch.permission_requirements(),
                ));
            }
            Err(reason) => {
                first.get_or_insert(reason);
            }
        }
    }
    first
}

/// Opens the editor on captured targets of a pool (the Rune `begin`).
/// `false` when the pool is not shown.
pub(crate) fn begin(m: &mut Model, pool: PoolId, targets: Vec<Item>) -> bool {
    let Some(i) = m.inventory_for(pool) else {
        return false;
    };
    m.areas.bulk_export.editor = Some(Editor {
        id: Uuid::new_v4(),
        pool,
        pool_name: i.pool_name.clone(),
        observed: m
            .snapshot_for(pool)
            .and_then(|v| v.observe_resource(ResourceTarget::Pool(pool)).ok()),
        files: vec![None; targets.len()],
        targets,
        format: VmExportFormat::Xva {
            compression: XvaCompression::Uncompressed,
        },
        pending: None,
        result: None,
        step: Step::Format,
        review: None,
        error: None,
    });
    true
}

fn edit(m: &mut Model) -> Option<&mut Editor> {
    m.areas.bulk_export.editor.as_mut()
}

pub(crate) fn close(m: &mut Model) {
    m.areas.bulk_export.editor = None;
}

/// The pool is still the one observed, through a connection. A read-only
/// connection may export. Never in a synthetic pool.
pub(crate) fn current(m: &Model, e: &Editor) -> bool {
    !m.is_synthetic_pool(e.pool)
        && m.connection_for(e.pool).is_some()
        && e.observed.is_some()
        && m.snapshot_for(e.pool)
            .and_then(|v| v.observe_resource(ResourceTarget::Pool(e.pool)).ok())
            == e.observed
}

/// The single export of `source` in `format`: its availability is the batch item's.
fn action(source: ExportSource, format: &VmExportFormat) -> TransferAction {
    match format {
        VmExportFormat::Xva { compression } => TransferAction::ExportXva {
            source,
            compression: *compression,
        },
        VmExportFormat::Ovf { format } => TransferAction::ExportOvf {
            source,
            format: *format,
        },
    }
}

fn vm_item(
    i: &Inventory,
    captured: &Vm,
    format: &VmExportFormat,
) -> Result<BatchVmWorkflow, String> {
    let v = i
        .vms
        .get(&captured.id)
        .filter(|v| {
            !v.is_control_domain
                && v.is_template == captured.is_template
                && v.power_state == captured.power_state
        })
        .ok_or_else(|| {
            format!(
                "{} ({}): identity or power state changed",
                captured.name, captured.id
            )
        })?;
    let source = ExportSource::Vm {
        vm: v.id,
        expected_template: v.is_template,
    };
    let status = action(source, format).availability(i);
    if !status.is_available() {
        return Err(format!("{} ({}): {}", v.name, v.id, status.explanation()));
    }
    Ok(BatchVmWorkflow {
        vm: v.id,
        expected_state: v.power_state.clone(),
        workflow: VmWorkflow::Export {
            expected_template: v.is_template,
            format: format.clone(),
        },
    })
}

/// The first target that cannot be exported in the chosen format, whatever
/// its destination: what the Format step shows.
pub(crate) fn source_refusal(m: &Model, e: &Editor) -> Option<String> {
    let Some(i) = m.inventory_for(e.pool) else {
        return Some(NOT_CURRENT_POOL.into());
    };
    e.targets
        .iter()
        .find_map(|target| target.workflow(&i, &e.format).err())
}

/// The batch the editor describes against the inventory as it is now, or
/// the first reason it cannot be made (the reasons are not joined). Cheap:
/// the view calls it on every render to show the live refusal, and the
/// submission compares it with the stored review.
pub(crate) fn proposal(m: &Model, e: &Editor) -> Result<Review, String> {
    let i = m.inventory_for(e.pool).ok_or(NOT_CURRENT_POOL)?;
    let mut items = Vec::new();
    let mut outputs = Vec::new();
    let mut paths = Vec::new();
    for (index, target) in e.targets.iter().enumerate() {
        let item = target.workflow(&i, &e.format)?;
        let (path, file) = e
            .files
            .get(index)
            .and_then(Option::as_ref)
            .ok_or_else(|| format!("Choose the output for {} ({})", target.name(), target.id()))?;
        if paths
            .iter()
            .any(|p: &String| PathBuf::from(p).starts_with(path) || path.starts_with(p))
        {
            return Err(OVERLAP.into());
        }
        paths.push(path.to_string_lossy().into_owned());
        outputs.push(file.clone());
        items.push(item);
    }
    let batch = VmWorkflowBatch { items };
    let permission = permissions::assess_for(m, e.pool, &batch.permission_requirements());
    if permission.decision().is_denied() {
        return Err(permission.explanation());
    }
    Ok(Review {
        batch,
        outputs,
        paths,
    })
}

pub(crate) fn has_checkpoint(items: &[Item]) -> bool {
    items
        .iter()
        .any(|target| matches!(target, Item::Snapshot(t) if t.is_checkpoint()))
}

/// The formats the editor offers. A selection with a memory checkpoint gets the XVA
/// formats only, as the single export of a checkpoint does.
pub(crate) fn formats(e: &Editor) -> Vec<(usize, &'static str, VmExportFormat)> {
    offered(&e.targets)
}

fn offered(items: &[Item]) -> Vec<(usize, &'static str, VmExportFormat)> {
    let mut formats = all_formats();
    if has_checkpoint(items) {
        formats.retain(|(_, _, format)| matches!(format, VmExportFormat::Xva { .. }));
    }
    formats
}

pub(crate) fn all_formats() -> Vec<(usize, &'static str, VmExportFormat)> {
    vec![
        (
            0,
            "XVA",
            VmExportFormat::Xva {
                compression: XvaCompression::Uncompressed,
            },
        ),
        (
            1,
            "XVA gzip",
            VmExportFormat::Xva {
                compression: XvaCompression::Gzip,
            },
        ),
        (
            2,
            "XVA zstd",
            VmExportFormat::Xva {
                compression: XvaCompression::Zstd,
            },
        ),
        (
            3,
            "OVF directory",
            VmExportFormat::Ovf {
                format: OvfPackageFormat::Ovf,
            },
        ),
        (
            4,
            "OVA",
            VmExportFormat::Ovf {
                format: OvfPackageFormat::Ova,
            },
        ),
    ]
}

/// The index and the name of a format in [`all_formats`].
pub(crate) fn format_entry(format: &VmExportFormat) -> (usize, &'static str) {
    all_formats()
        .into_iter()
        .find(|(_, _, f)| f == format)
        .map_or((0, "XVA"), |(index, name, _)| (index, name))
}

pub(crate) fn format_name(format: &VmExportFormat) -> &'static str {
    format_entry(format).1
}

/// A format change is another export: the editor gets a new identity and
/// loses every destination, a pending one too. A format that is not
/// offered is ignored.
pub(crate) fn change_format(m: &mut Model, format: VmExportFormat) {
    let Some(e) = edit(m) else { return };
    if !formats(e).iter().any(|(_, _, offered)| *offered == format) {
        return;
    }
    e.id = Uuid::new_v4();
    e.format = format;
    e.files.fill(None);
    e.pending = None;
    e.result = None;
    e.review = None;
    e.error = None;
}

/// The handler of the format choice: `id` is the index of the format.
pub(crate) fn pick_format(m: &mut Model, id: &str) {
    let picked = all_formats()
        .into_iter()
        .find(|(index, _, _)| index.to_string() == id);
    if let Some((_, _, format)) = picked
        && edit(m).is_some_and(|e| e.pending.is_none() && e.format != format)
    {
        change_format(m, format);
    }
}

/// Whether the editor is still the one a destination was asked for.
fn bound(e: &Editor, id: Uuid, format: &VmExportFormat, index: usize, target: Uuid) -> bool {
    e.id == id
        && e.format == *format
        && e.targets.get(index).is_some_and(|v| v.id() == target)
        && e.pending == Some(index)
}

/// Asks for the destination of one target. One at a time; the answer is
/// taken only by the editor, format and target it was asked for, and is
/// inspected on a thread of its own ([`poll`] collects it).
pub(crate) fn choose(m: &mut Model, index: usize) {
    let Some(e) = edit(m) else { return };
    if e.pending.is_some() || index >= e.targets.len() {
        return;
    }
    let id = e.id;
    let format = e.format.clone();
    let target = e.targets[index].id();
    e.pending = Some(index);
    e.error = None;
    e.review = None;
    e.files[index] = None;
    let extension = match &format {
        VmExportFormat::Xva { compression } => compression.extension(),
        VmExportFormat::Ovf {
            format: OvfPackageFormat::Ova,
        } => "ova",
        VmExportFormat::Ovf { .. } => "ovf-package",
    };
    let asked = format.clone();
    let reply = move |m: &mut Model, path: Option<PathBuf>| {
        let wake = m.waker.clone();
        let Some(e) = edit(m).filter(|e| bound(e, id, &asked, index, target)) else {
            return;
        };
        let Some(path) = path else {
            e.pending = None;
            return;
        };
        let (sender, receiver) = mpsc::channel();
        let job = std::thread::Builder::new()
            .name("batch-export-destination".into())
            .spawn(move || {
                let value = TransferFile::destination(&path)
                    .map(|file| (path, file))
                    .map_err(|e| e.to_string());
                let _ = sender.send(value);
                if let Some(w) = wake {
                    w()
                }
            });
        if job.is_ok() {
            e.result = Some(receiver);
        } else {
            e.pending = None;
            e.error = Some(NO_INSPECTION.into());
        }
    };
    let failed = move |m: &mut Model, error: &str| {
        if let Some(e) = edit(m).filter(|e| bound(e, id, &format, index, target)) {
            e.pending = None;
            e.error = Some(error.to_owned());
        }
    };
    m.choose_file(
        FileRequest::save(PROMPT, format!("{target}.{extension}"), reply).on_failure(failed),
    );
}

/// Collects the inspection of a chosen destination. Never waits.
pub fn poll(m: &mut Model, _clock: f64) -> bool {
    let Some(e) = edit(m) else {
        return false;
    };
    let Some(rx) = &e.result else { return false };
    let result = match rx.try_recv() {
        Ok(v) => v,
        Err(mpsc::TryRecvError::Empty) => return false,
        Err(_) => Err(INSPECTION_STOPPED.into()),
    };
    e.result = None;
    let Some(index) = e.pending.take() else {
        return true;
    };
    match result {
        Ok(file) => {
            if let Some(slot) = e.files.get_mut(index) {
                *slot = Some(file);
            }
        }
        Err(error) => e.error = Some(error),
    }
    true
}

/// Sets one checkpoint's memory choice. A review made before the change no longer stands.
pub(crate) fn set_memory(m: &mut Model, index: usize, preserve: bool) {
    let Some(e) = edit(m) else { return };
    if let Some(Item::Snapshot(target)) = e.targets.get_mut(index)
        && target.memory.is_some()
    {
        target.preserve_memory = preserve;
        e.review = None;
        e.error = None;
    }
}

/// From the format to the destinations.
pub(crate) fn next(m: &mut Model) {
    if let Some(e) = edit(m) {
        e.step = Step::Destinations;
    }
}

/// One step back: out of the review (the Rune "Back"), else from the
/// destinations to the format.
pub(crate) fn back(m: &mut Model) {
    if let Some(e) = edit(m)
        && e.review.take().is_none()
    {
        e.step = Step::Format;
    }
}

pub(crate) fn review(m: &mut Model) {
    let Some(e) = &m.areas.bulk_export.editor else {
        return;
    };
    let result = proposal(m, e);
    let Some(e) = edit(m) else { return };
    match result {
        Ok(r) => {
            e.step = Step::Destinations;
            e.review = Some(r);
            e.error = None;
        }
        Err(error) => e.error = Some(error),
    }
}

/// Why the reviewed batch cannot be submitted, the first that applies;
/// `None` when it can. The demo first: there the pool has no observation
/// at all.
pub(crate) fn blocked(m: &Model, e: &Editor, proposal: &Result<Review, String>) -> Option<String> {
    if m.is_synthetic_pool(e.pool) {
        return Some(PREVIEW.into());
    }
    if !current(m, e) {
        return Some(STALE.into());
    }
    match proposal {
        Err(reason) => return Some(reason.clone()),
        Ok(now) if Some(now) != e.review.as_ref() => return Some(CHANGED.into()),
        Ok(_) => {}
    }
    (m.resource_operations.len() >= 64).then(|| Error::Busy.to_string())
}

/// Submits the reviewed batch. Every refusal before the connection is the
/// same `Stale` in the editor's error line, as in Rune: the pool is not
/// the one observed, the review is not what would be reviewed now, or 64
/// resource operations are already held. Each output goes with the object
/// its destination was chosen for; VMs and snapshots are two calls of the
/// core. Accepted, the batch is tracked, the editor closes and the
/// operations are shown (Rune opened the pool history).
pub(crate) fn submit(m: &mut Model) {
    let result = (|| {
        let e = m.areas.bulk_export.editor.as_ref().ok_or(Error::Stale)?;
        let r = e.review.as_ref().ok_or(Error::Stale)?;
        if !current(m, e)
            || proposal(m, e).ok().as_ref() != Some(r)
            || m.resource_operations.len() >= 64
        {
            return Err(Error::Stale);
        }
        let connection = m.connection_for(e.pool).ok_or(Error::Closed)?;
        let id = OperationId::new();
        let observed = e.observed.ok_or(Error::Stale)?;
        // Each output goes with the object its destination was chosen for.
        let targets = e.targets.iter().zip(&r.outputs);
        if r.batch.is_snapshot_export() {
            let outputs = targets
                .filter_map(|(target, file)| match target {
                    Item::Snapshot(t) => Some((t.snapshot.id, file.clone())),
                    Item::Vm(_) => None,
                })
                .collect();
            connection.submit_snapshot_export_batch(id, observed, r.batch.clone(), outputs)
        } else {
            let outputs = targets
                .filter_map(|(target, file)| match target {
                    Item::Vm(vm) => Some((vm.id, file.clone())),
                    Item::Snapshot(_) => None,
                })
                .collect();
            connection.submit_vm_workflow_batch(id, observed, r.batch.clone(), outputs)
        }
    })();
    match result {
        Ok(operation) => {
            m.track_resource(operation);
            close(m);
            m.show_operations();
        }
        Err(error) => {
            if let Some(e) = edit(m) {
                e.error = Some(error.to_string());
            }
        }
    }
}
