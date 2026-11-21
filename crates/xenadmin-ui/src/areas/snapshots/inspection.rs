//! `snapshot_inspection.rs`: what the client knows about one snapshot
//! (parent, ancestry, disks, memory checkpoint) and its saved console
//! preview, loaded on demand and decoded on a worker thread.
//!
//! In Rune this was the dialog "Snapshot information", opened for the
//! selected snapshot with the inventory of that moment. Information is a
//! tab, not a dialog: here it is the section [`section`] of the Snapshots
//! tab, shown for the one selected snapshot from the inventory as it is
//! now. What is kept per snapshot is only the saved preview ([`Panel`]):
//! its query, its decode and its picture belong to the snapshot they were
//! asked for and are dropped when another row is selected. The decoder
//! slot is the area's and is released when the job ends, not when the
//! panel goes.
//!
//! Keys: `snapshot-information` (the section), `snapshot-preview-message`,
//! `snapshot-preview-image` and `snapshot-preview-load` (both Rune keys).

use std::{
    collections::BTreeSet,
    io::Cursor,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};

use xenadmin_core::{
    ClassAvailability, Error, Inventory, ObservedSnapshot, PoolId, Snapshot, SnapshotId,
    SnapshotParent, SnapshotPreview, SnapshotPreviewQuery, VdiId, VmId,
};

use super::{selected_snapshot, shown_vm};
use crate::prelude::*;

pub const INITIAL: &str =
    "Load an existing saved preview from the host. It may contain sensitive guest information.";
pub const LOADED: &str = "Saved snapshot preview · may differ from the current guest display";
pub const SYNTHETIC: &str = "A synthetic inventory has no saved previews on a host.";

pub(super) struct Decoded {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Default)]
pub struct Decoder {
    pub(super) busy: Arc<AtomicBool>,
}

struct DecodePermit(Arc<AtomicBool>);

impl Drop for DecodePermit {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl Decoder {
    pub(super) fn submit(
        &self,
        job: impl FnOnce() -> Result<Decoded, String> + Send + 'static,
        wake: Option<Arc<dyn Fn() + Send + Sync>>,
    ) -> Result<mpsc::Receiver<Result<Decoded, String>>, String> {
        self.busy
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .map_err(|_| "Another saved preview is still decoding; try again shortly".to_owned())?;
        let permit = DecodePermit(self.busy.clone());
        let (sender, receiver) = mpsc::channel();
        std::thread::Builder::new()
            .name("snapshot-preview".into())
            .spawn(move || {
                let result = job();
                drop(permit);
                let _ = sender.send(result);
                if let Some(wake) = wake {
                    wake();
                }
            })
            .map_err(|_| "Cannot start the preview decoder".to_owned())?;
        Ok(receiver)
    }
}

/// The saved preview of one snapshot.
pub struct Panel {
    pub pool: PoolId,
    pub vm: VmId,
    pub snapshot: SnapshotId,
    pub(super) observed: Option<ObservedSnapshot>,
    pub(super) query: Option<SnapshotPreviewQuery>,
    pub(super) decode: Option<mpsc::Receiver<Result<Decoded, String>>>,
    /// The decoded picture. A clone shares the bytes, so the description
    /// carries it without a copy.
    pub image: Option<ImageData>,
    pub dimensions: Option<(u32, u32)>,
    pub message: String,
}

impl Panel {
    pub fn new(pool: PoolId, vm: VmId, snapshot: SnapshotId) -> Self {
        Self {
            pool,
            vm,
            snapshot,
            observed: None,
            query: None,
            decode: None,
            image: None,
            dimensions: None,
            message: INITIAL.into(),
        }
    }

    pub fn pending(&self) -> bool {
        self.query.is_some() || self.decode.is_some()
    }

    fn is_for(&self, pool: PoolId, vm: VmId, snapshot: SnapshotId) -> bool {
        self.pool == pool && self.vm == vm && self.snapshot == snapshot
    }
}

/// The panel of the one selected snapshot, when there is one for it.
pub fn panel(m: &Model) -> Option<&Panel> {
    let (pool, vm) = shown_vm(m)?;
    let snapshot = selected_snapshot(m)?;
    m.areas
        .snapshots
        .inspection
        .as_ref()
        .filter(|p| p.is_for(pool, vm, snapshot.id))
}

/// After the selection changed: a preview that belongs to another snapshot
/// than the one selected is dropped, with its pending query or decode.
pub fn follow(m: &mut Model) {
    if m.areas.snapshots.inspection.is_some() && panel(m).is_none() {
        m.areas.snapshots.inspection = None;
    }
}

/// "Load saved preview": asks the host for the saved picture of the
/// selected snapshot. A refusal is the panel's message, not the shared
/// error. A second press while one is pending does nothing.
pub fn request(m: &mut Model) {
    let Some((pool, vm)) = shown_vm(m) else {
        return;
    };
    let Some(selected) = selected_snapshot(m) else {
        return;
    };
    if panel(m).is_none() {
        m.areas.snapshots.inspection = Some(Panel::new(pool, vm, selected.id));
    }
    if m.areas
        .snapshots
        .inspection
        .as_ref()
        .is_some_and(Panel::pending)
    {
        return;
    }
    let result = (|| {
        let snapshot = m.snapshot_for(pool).ok_or(Error::Stale)?;
        if snapshot.inventory().is_none_or(|i| i.pool_id != pool) {
            return Err(Error::Stale);
        }
        let observed = snapshot.observe_snapshot(vm, selected.id)?;
        let query = m
            .connection_for(pool)
            .ok_or(Error::Closed)?
            .snapshot_preview(observed)?;
        if let Some(wake) = &m.waker {
            query.notify_on_change(wake.clone());
        }
        Ok((observed, query))
    })();
    let Some(p) = m.areas.snapshots.inspection.as_mut() else {
        return;
    };
    match result {
        Ok((observed, query)) => {
            p.observed = Some(observed);
            p.query = Some(query);
            p.message = "Loading saved preview…".into();
            p.dimensions = None;
            p.image = None;
        }
        Err(e) => p.message = e.to_string(),
    }
}

pub(super) fn decode(image: SnapshotPreview) -> Result<Decoded, String> {
    let format = match image.mime {
        "image/png" => image::ImageFormat::Png,
        "image/jpeg" => image::ImageFormat::Jpeg,
        _ => return Err("Unsupported preview type".into()),
    };
    let mut header = image::ImageReader::with_format(Cursor::new(image.bytes.as_ref()), format);
    let mut header_limits = image::Limits::default();
    header_limits.max_alloc = Some(64 * 1024 * 1024);
    header.limits(header_limits);
    let dimensions = header
        .into_dimensions()
        .map_err(|_| "Invalid preview dimensions")?;
    if dimensions.0 == 0
        || dimensions.1 == 0
        || dimensions.0 > 4096
        || dimensions.1 > 4096
        || u64::from(dimensions.0) * u64::from(dimensions.1) > 4_194_304
    {
        return Err("Preview exceeds the 4096-pixel side or 4-megapixel limit".into());
    }
    let mut reader = image::ImageReader::with_format(Cursor::new(image.bytes.as_ref()), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let rgba = reader
        .decode()
        .map_err(|_| "Cannot decode saved preview")?
        .into_rgba8();
    if rgba.dimensions() != dimensions {
        return Err("Preview dimensions changed".into());
    }
    Ok(Decoded {
        width: dimensions.0,
        height: dimensions.1,
        rgba: rgba.into_raw(),
    })
}

/// Moves the saved preview on: the query's answer to the decoder, the
/// decoder's result to the picture. A pending load whose snapshot is no
/// longer the one observed is dropped. Returns whether anything changed.
pub fn poll(m: &mut Model) -> bool {
    let Some(mut p) = m.areas.snapshots.inspection.take() else {
        return false;
    };
    let changed = advance(m, &mut p);
    m.areas.snapshots.inspection = Some(p);
    changed
}

fn advance(m: &Model, p: &mut Panel) -> bool {
    if p.pending()
        && p.observed.is_some_and(|o| {
            m.snapshot_for(p.pool)
                .and_then(|v| v.observe_snapshot(p.vm, p.snapshot).ok())
                != Some(o)
        })
    {
        p.query = None;
        p.decode = None;
        p.observed = None;
        p.message = "The connection or snapshot changed; load again from the original pool.".into();
        return true;
    }
    let mut changed = false;
    if let Some(result) = p.query.as_ref().and_then(|q| q.latest()) {
        p.query = None;
        changed = true;
        match result {
            Ok(Some(image)) => {
                match m
                    .areas
                    .snapshots
                    .decoder
                    .submit(move || decode(image), m.waker.clone())
                {
                    Ok(receiver) => {
                        p.decode = Some(receiver);
                        p.message = "Decoding saved preview…".into();
                    }
                    Err(error) => p.message = error,
                }
            }
            Ok(None) => p.message = "This snapshot has no saved preview image.".into(),
            Err(e) => p.message = e,
        }
    }
    if let Some(receiver) = &p.decode {
        let result = match receiver.try_recv() {
            Ok(v) => Some(v),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err("The preview decoder stopped".into()))
            }
        };
        if let Some(result) = result {
            p.decode = None;
            changed = true;
            match result {
                Ok(decoded) => show(p, decoded),
                Err(e) => p.message = e,
            }
        }
    }
    changed
}

fn show(p: &mut Panel, decoded: Decoded) {
    match ImageData::rgba(decoded.width, decoded.height, decoded.rgba) {
        Some(image) => {
            p.image = Some(image);
            p.dimensions = Some((decoded.width, decoded.height));
            p.message = LOADED.into();
        }
        None => p.message = "Cannot decode saved preview".into(),
    }
}

/// A decoded picture and its message, as a load that finished leaves them
/// (the scene's fixture and the tests use it).
pub fn set_picture(p: &mut Panel, width: u32, height: u32, rgba: Vec<u8>, message: &str) {
    show(
        p,
        Decoded {
            width,
            height,
            rgba,
        },
    );
    if p.image.is_some() {
        p.message = message.into();
    }
}

// -- The section ------------------------------------------------------------------

fn size(value: Option<u64>) -> String {
    value.map(bytes).unwrap_or_else(|| "Not reported".into())
}

fn yes(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "Yes",
        Some(false) => "No",
        None => "Unknown",
    }
}

fn disk(i: &Inventory, id: VdiId) -> Block {
    let Some(d) = i.details.vdis.get(&id) else {
        return properties([property("Disk unavailable", id.to_string()).copyable()]);
    };
    let sr =
        d.sr.map(|id| {
            i.storage
                .iter()
                .find(|sr| sr.uuid == id.uuid())
                .map(|sr| format!("{} ({id})", sr.name))
                .unwrap_or_else(|| format!("Storage unavailable ({id})"))
        })
        .unwrap_or_else(|| "Storage not reported".into());
    properties([
        property(d.name.clone(), id.to_string()).copyable(),
        property("Storage repository", sr),
        property(
            "Virtual capacity / reported allocation",
            format!(
                "{} / {}",
                size(d.virtual_size),
                size(d.physical_utilisation)
            ),
        ),
        property(
            "Disk ancestry",
            d.snapshot_of
                .map(|id| format!("Snapshot of VDI {id}"))
                .unwrap_or_else(|| "None or not reported".into()),
        ),
        property(
            "Read only / sharable / managed / missing",
            format!(
                "{} / {} / {} / {}",
                yes(d.read_only),
                yes(d.sharable),
                yes(d.managed),
                yes(d.missing)
            ),
        ),
    ])
}

/// The section "Snapshot information" for the one selected snapshot of the
/// shown VM: the selection summary of the Rune page (name, date, UUID,
/// description) and the body of the Rune dialog, in its order.
pub fn section(m: &Model, i: &Inventory, pool: PoolId, vm: VmId, snapshot: &Snapshot) -> Group {
    let details = i.details.snapshots.get(&snapshot.id);
    let ancestry = i
        .snapshot_ancestry(vm)
        .into_iter()
        .find(|row| row.id == snapshot.id);
    let parent = match details.map(|d| d.parent).unwrap_or_default() {
        SnapshotParent::Root => "Root snapshot".into(),
        SnapshotParent::Snapshot(id) => format!(
            "{} ({id})",
            i.snapshots
                .get(&id)
                .map(|s| s.name.as_str())
                .unwrap_or("Unavailable snapshot")
        ),
        SnapshotParent::Vm(id) => format!("VM {id}"),
        SnapshotParent::Unresolved => "Not reported or unavailable".into(),
    };
    let synthetic = m.is_synthetic_pool(pool);
    let current = m.snapshot_for(pool).is_some_and(|v| v.is_current());
    let complete = ["vdi", "vbd"]
        .iter()
        .all(|c| i.details.classes.get(*c) == Some(&ClassAvailability::Available));
    let mut ids = BTreeSet::new();
    let mut unresolved = 0usize;
    for id in i
        .details
        .snapshot_disks
        .get(&snapshot.id)
        .into_iter()
        .flatten()
    {
        if let Some(vbd) = i.details.vbds.get(id) {
            if vbd.empty == Some(true) {
                continue;
            }
            if let Some(id) = vbd.vdi {
                ids.insert(id);
            } else {
                unresolved += 1;
            }
        } else {
            unresolved += 1;
        }
    }
    let total = ids.iter().try_fold(0u64, |sum, id| {
        sum.checked_add(i.details.vdis.get(id)?.physical_utilisation?)
    });
    let memory: Vec<Block> = if let Some(id) = details.and_then(|d| d.memory_vdi) {
        blocks![heading("Memory checkpoint disk"), disk(i, id)]
    } else if details.is_none_or(|d| !d.memory_vdi_known) {
        blocks![caption(
            "Memory checkpoint disk information is unavailable."
        )]
    } else {
        Vec::new()
    };
    let panel = panel(m);
    let message = panel.map_or(INITIAL, |p| p.message.as_str());
    let picture = panel.and_then(|p| p.image.clone()).map(|data| {
        image("snapshot-preview-image", "Saved snapshot preview", data).max_size(640, 360)
    });
    // What is known of the snapshot, and beside it (under it when narrow)
    // its saved preview.
    let known: Vec<Block> = blocks![
        properties([
            property("Name", snapshot.name.clone()),
            property("UUID", snapshot.id.to_string()).copyable(),
            property(
                "Created (host time)",
                snapshot.created_at.as_deref().unwrap_or("Unknown date"),
            ),
            property("Description", snapshot.description.clone()),
        ]),
        caption(format!(
            "Pool {} · revision {} · {}",
            i.pool_id,
            i.revision,
            if synthetic {
                "Synthetic inventory"
            } else if current {
                "Current"
            } else {
                "Stale"
            }
        ))
        .key("snapshot-information-source"),
        properties([
            property(
                "VM",
                format!(
                    "{} ({vm})",
                    i.vms
                        .get(&vm)
                        .map(|vm| vm.name.as_str())
                        .unwrap_or("Unavailable"),
                ),
            ),
            property("Parent snapshot", parent),
        ]),
        ancestry
            .as_ref()
            .filter(|row| row.current_parent)
            .map(|_| caption("The VM currently descends from this snapshot.")),
        ancestry.as_ref().and_then(|row| row.issue).map(caption),
        heading("Virtual disks"),
        (!complete).then(|| caption(
            "Disk inventory is incomplete; sizes and attachments may be missing."
        )),
        ids.iter().map(|id| disk(i, *id)).collect::<Vec<_>>(),
        ids.is_empty()
            .then(|| caption("No resolved attached disks in this captured inventory.")),
        (unresolved > 0).then(|| caption(format!(
            "{unresolved} attachment(s) could not be resolved."
        ))),
        properties([property(
            "Sum of reported disk allocations",
            if complete && unresolved == 0 {
                size(total)
            } else {
                "Incomplete inventory".into()
            },
        )]),
        caption("Allocations are reported by the storage backend. Shared blocks and snapshot chains mean this is not exclusive usage or the space deletion would reclaim."),
        memory,
    ];
    let preview: Vec<Block> = blocks![
        heading("Saved preview"),
        caption(message).key("snapshot-preview-message"),
        picture,
    ];
    group("Snapshot information", vec![columns([known, preview])])
        .key("snapshot-information")
}
