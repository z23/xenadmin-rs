//! `snapshot_capture.rs`: the opt-in workflow of the Create dialog that
//! captures one console frame, encodes it on a worker thread, creates the
//! snapshot and uploads the picture as a separate resource operation.
//!
//! Carried over from the Rune module. What changed: the workflow keeps the
//! pool of its VM and follows that pool's connection (not the scope
//! session's); its timeouts are measured on the model's clock
//! (`Model::clock`, the time of the last poll) and it asks for a tick every
//! 250 ms while it is pending, where Rune read `Instant` and called
//! `tx.wake_after`.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};

use image::ImageEncoder;
use xenadmin_core::{
    Console, ConsoleFrame, ConsoleState, ObservedVm, Operation, OperationId, OperationRecord,
    OperationState, PoolId, PowerState, SnapshotId, SnapshotPreviewImage, SnapshotWorkflow,
    VmCommand, VmId,
};
use zeroize::Zeroize;

use super::edit::{CAPTURE_PENDING, Dialog};
use crate::prelude::*;

/// How often the pending workflow is looked at when no handle wakes the
/// model, in seconds.
const TICK: f64 = 0.25;
/// Capture and encoding together, and the wait for the created snapshot to
/// appear in the inventory, in seconds.
const STEP_LIMIT: f64 = 30.0;
/// The whole wait for the snapshot operation, in seconds.
const TOTAL_LIMIT: f64 = 1800.0;

#[derive(Default)]
pub struct Workflow {
    pub opt_in: bool,
    pending: Option<Pending>,
    encoder: Arc<AtomicBool>,
    pub message: Option<String>,
}

struct Pending {
    pool: PoolId,
    vm: ObservedVm,
    id: VmId,
    started: f64,
    stage: Stage,
}

enum Stage {
    Capture {
        console: Console,
        command: VmCommand,
    },
    Encode {
        receiver: mpsc::Receiver<Result<SnapshotPreviewImage, String>>,
        command: VmCommand,
    },
    Create {
        operation: Operation,
        image: SnapshotPreviewImage,
        succeeded: Option<f64>,
    },
}

struct Permit(Arc<AtomicBool>);

impl Drop for Permit {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl Workflow {
    pub fn busy(&self) -> bool {
        self.pending.is_some() || self.encoder.load(Ordering::Acquire)
    }

    /// Whether a workflow is pending (as opposed to an encoder thread that
    /// is still finishing).
    pub fn pending(&self) -> bool {
        self.pending.is_some()
    }
}

fn current(m: &Model, pool: PoolId, vm: ObservedVm, id: VmId) -> bool {
    !m.is_synthetic_pool(pool)
        && m.connection_for(pool)
            .is_some_and(|c| !c.is_read_only() && c.latest().observe_vm(id).ok() == Some(vm))
}

pub fn available(m: &Model, pool: PoolId, id: VmId) -> bool {
    m.inventory_for(pool)
        .as_ref()
        .and_then(|i| i.vms.get(&id))
        .is_some_and(|v| v.power_state == PowerState::Running)
}

pub fn begin(m: &mut Model, pool: PoolId, command: VmCommand, id: VmId) {
    let result = (|| {
        if m.areas.snapshots.capture.busy() {
            return Err(CAPTURE_PENDING.into());
        }
        let VmCommand::SnapshotWorkflow {
            vm,
            operation: SnapshotWorkflow::Create { .. },
        } = &command
        else {
            return Err("Preview capture requires snapshot creation".into());
        };
        let vm = *vm;
        if !current(m, pool, vm, id) || !available(m, pool, id) {
            return Err("A current running VM is required to capture a preview".into());
        }
        let console = m
            .connection_for(pool)
            .ok_or_else(|| xenadmin_core::Error::Closed.to_string())?
            .open_console(vm)
            .map_err(|e| e.to_string())?;
        if let Some(wake) = &m.waker {
            console.notify_on_change(wake.clone());
            wake();
        }
        let started = m.clock();
        m.request_tick(TICK);
        let capture = &mut m.areas.snapshots.capture;
        capture.pending = Some(Pending {
            pool,
            vm,
            id,
            started,
            stage: Stage::Capture { console, command },
        });
        capture.message = Some("Capturing the console before creating the snapshot…".into());
        Ok::<_, String>(())
    })();
    m.error = result.err();
}

pub(super) fn encode(frame: Arc<ConsoleFrame>) -> Result<SnapshotPreviewImage, String> {
    let w = u32::from(frame.width);
    let h = u32::from(frame.height);
    let pixels = u64::from(w) * u64::from(h);
    if pixels == 0 || pixels > 8_294_400 || frame.rgba.len() as u64 != pixels * 4 {
        return Err("Invalid console frame".into());
    }
    let source = image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(w, h, frame.rgba.clone())
        .ok_or("Invalid console frame")?;
    let ratio = (1600.0 / f64::from(w)).min(900.0 / f64::from(h)).min(1.0);
    let w = (f64::from(w) * ratio).floor().max(1.0) as u32;
    let h = (f64::from(h) * ratio).floor().max(1.0) as u32;
    let mut scaled = image::imageops::resize(&source, w, h, image::imageops::FilterType::Triangle);
    let mut bytes = Vec::new();
    let result = image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(&scaled, w, h, image::ExtendedColorType::Rgba8)
        .map_err(|_| "Cannot encode console preview".to_owned());
    scaled.as_mut().zeroize();
    result?;
    SnapshotPreviewImage::from_png(bytes).map_err(|e| e.to_string())
}

fn spawn(
    m: &Model,
    frame: Arc<ConsoleFrame>,
) -> Result<mpsc::Receiver<Result<SnapshotPreviewImage, String>>, String> {
    let encoder = &m.areas.snapshots.capture.encoder;
    encoder
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .map_err(|_| "The previous preview encoder is still finishing".to_owned())?;
    let permit = Permit(encoder.clone());
    let (tx, rx) = mpsc::channel();
    let wake = m.waker.clone();
    std::thread::Builder::new()
        .name("snapshot-capture".into())
        .spawn(move || {
            let result = encode(frame);
            drop(permit);
            let _ = tx.send(result);
            if let Some(wake) = wake {
                wake();
            }
        })
        .map_err(|_| "Cannot start the preview encoder".to_owned())?;
    Ok(rx)
}

/// Cancel and Escape of every snapshot dialog. A capture that has not
/// submitted its snapshot yet is dropped with the dialog; one that waits
/// for the created snapshot is not cancelled by closing.
pub fn cancel_dialog(m: &mut Model) {
    let a = &mut m.areas.snapshots;
    if a.capture
        .pending
        .as_ref()
        .is_some_and(|p| matches!(p.stage, Stage::Capture { .. } | Stage::Encode { .. }))
    {
        a.capture.pending = None;
        a.capture.message = Some("Preview capture cancelled; no snapshot was submitted.".into());
    }
    a.dialog = None;
}

pub(super) fn creation_result(record: &OperationRecord) -> Result<Option<SnapshotId>, String> {
    if record.cancel_requested {
        return Err("Snapshot operation was stopped; the optional preview will not be uploaded. Inspect Activity for the snapshot outcome.".into());
    }
    if record.state == OperationState::Succeeded {
        return record.created_snapshot.map(Some).ok_or_else(|| {
            "Snapshot creation did not return a verified identity; preview skipped".into()
        });
    }
    if record.state.terminal() || record.state == OperationState::OutcomeUnknown {
        return Err("Snapshot creation did not finish with a verified success; preview skipped. Inspect its result in Activity.".into());
    }
    Ok(None)
}

fn advance(m: &mut Model, mut p: Pending) -> Result<Option<Pending>, String> {
    if !current(m, p.pool, p.vm, p.id) {
        return Err("Preview stopped because its original connection changed. Check Activity for any snapshot already submitted.".into());
    }
    let now = m.clock();
    if matches!(p.stage, Stage::Capture { .. } | Stage::Encode { .. })
        && (!matches!(m.areas.snapshots.dialog, Some(Dialog::Create { .. }))
            || now - p.started > STEP_LIMIT)
    {
        return Err("Preview capture stopped before snapshot creation. Retry explicitly or create without a preview.".into());
    }
    p.stage = match p.stage {
        Stage::Capture { console, command } => {
            let latest = console.latest();
            match &latest.state {
                ConsoleState::Failed(_) | ConsoleState::Closed => return Err(
                    "Cannot capture this console. Retry explicitly or create without a preview."
                        .into(),
                ),
                ConsoleState::Connected if latest.frame.is_some() => {
                    let frame = latest.frame.clone().ok_or("Invalid console frame")?;
                    let receiver = spawn(m, frame)?;
                    drop(console);
                    m.areas.snapshots.capture.message =
                        Some("Preparing the captured preview…".into());
                    Stage::Encode { receiver, command }
                }
                _ => Stage::Capture { console, command },
            }
        }
        Stage::Encode { receiver, command } => match receiver.try_recv() {
            Err(mpsc::TryRecvError::Empty) => Stage::Encode { receiver, command },
            Err(mpsc::TryRecvError::Disconnected) => {
                return Err("The preview encoder stopped. No snapshot was submitted.".into());
            }
            Ok(result) => {
                let image = result?;
                if !available(m, p.pool, p.id) {
                    return Err("VM state changed before snapshot creation; review again.".into());
                }
                let id = crate::areas::power::track_vm(m, p.pool, command)
                    .map_err(|e| e.to_string())?;
                let operation = m
                    .operations
                    .iter()
                    .find(|o| o.latest().id == id)
                    .cloned()
                    .ok_or("Snapshot operation tracking unavailable")?;
                let a = &mut m.areas.snapshots;
                a.dialog = None;
                a.name.clear();
                a.memory = false;
                a.capture.message = Some("Creating the snapshot. The captured image will be attached only after creation is verified.".into());
                Stage::Create {
                    operation,
                    image,
                    succeeded: None,
                }
            }
        },
        Stage::Create {
            operation,
            image,
            mut succeeded,
        } => {
            let result = operation.latest();
            if let Some(id) = creation_result(&result)? {
                let connection = m
                    .connection_for(p.pool)
                    .ok_or_else(|| xenadmin_core::Error::Closed.to_string())?;
                let latest = connection.latest();
                match latest.observe_snapshot(p.id, id) {
                    Ok(snapshot) => {
                        m.resource_operations
                            .retain(|o| !o.latest().state.terminal());
                        if m.resource_operations.len() >= 64 {
                            return Err("Snapshot created; preview skipped because too many operations are active.".into());
                        }
                        let upload = connection
                            .upload_snapshot_preview(OperationId::new(), snapshot, result.id, image)
                            .map_err(|e| format!("Snapshot created; preview not submitted: {e}"))?;
                        m.track_resource(upload);
                        m.areas.snapshots.capture.message = Some(
                            "Snapshot created. Preview upload has a separate result in Activity."
                                .into(),
                        );
                        return Ok(None);
                    }
                    Err(_) => {
                        let since = *succeeded.get_or_insert(now);
                        if now - since > STEP_LIMIT {
                            return Err("Snapshot created; preview skipped because its inventory identity was not available in time.".into());
                        }
                    }
                }
            }
            if now - p.started > TOTAL_LIMIT {
                return Err("Preview discarded after waiting 30 minutes. The snapshot operation remains in Activity.".into());
            }
            Stage::Create {
                operation,
                image,
                succeeded,
            }
        }
    };
    Ok(Some(p))
}

/// One step of the pending workflow. A failure ends it and is both its
/// message and the shared error. Returns whether the message changed.
pub fn poll(m: &mut Model) -> bool {
    let Some(p) = m.areas.snapshots.capture.pending.take() else {
        return false;
    };
    let old = m.areas.snapshots.capture.message.clone();
    match advance(m, p) {
        Ok(p) => m.areas.snapshots.capture.pending = p,
        Err(error) => {
            m.areas.snapshots.capture.message = Some(error.clone());
            m.error = Some(error);
        }
    }
    if m.areas.snapshots.capture.pending.is_some() {
        m.request_tick(TICK);
    }
    old != m.areas.snapshots.capture.message
}
