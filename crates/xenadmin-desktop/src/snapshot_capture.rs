use super::*;
use image::ImageEncoder;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use xenadmin_core::{
    ConsoleFrame, ConsoleState, ObservedVm, Operation, OperationState, SnapshotPreviewImage,
    VmCommand,
};
use zeroize::Zeroize;

#[derive(Default)]
pub(super) struct Workflow {
    pub opt_in: bool,
    pending: Option<Pending>,
    encoder: Arc<AtomicBool>,
    pub message: Option<String>,
}
struct Pending {
    vm: ObservedVm,
    id: VmId,
    started: Instant,
    stage: Stage,
}
enum Stage {
    Capture {
        console: xenadmin_core::Console,
        command: VmCommand,
    },
    Encode {
        receiver: mpsc::Receiver<Result<SnapshotPreviewImage, String>>,
        command: VmCommand,
    },
    Create {
        operation: Operation,
        image: SnapshotPreviewImage,
        succeeded: Option<Instant>,
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
}
fn current(s: &State, vm: ObservedVm, id: VmId) -> bool {
    !s.demo
        && s.connection
            .as_ref()
            .is_some_and(|c| !c.is_read_only() && c.latest().observe_vm(id).ok() == Some(vm))
}
pub(super) fn available(s: &State, id: VmId) -> bool {
    s.inventory
        .as_ref()
        .and_then(|i| i.vms.get(&id))
        .is_some_and(|v| v.power_state == PowerState::Running)
}
pub(super) fn begin(s: &mut State, command: VmCommand, id: VmId) {
    let result = (|| {
        if s.snapshot_capture.busy() {
            return Err("A snapshot preview is already pending".into());
        }
        let VmCommand::SnapshotWorkflow {
            vm,
            operation: xenadmin_core::SnapshotWorkflow::Create { .. },
        } = &command
        else {
            return Err("Preview capture requires snapshot creation".into());
        };
        if !current(s, *vm, id) || !available(s, id) {
            return Err("A current running VM is required to capture a preview".into());
        }
        let console = s
            .connection
            .as_ref()
            .unwrap()
            .open_console(*vm)
            .map_err(|e| e.to_string())?;
        if let Some(wake) = &s.waker {
            console.notify_on_change(wake.clone());
            wake();
        }
        s.snapshot_capture.pending = Some(Pending {
            vm: *vm,
            id,
            started: Instant::now(),
            stage: Stage::Capture { console, command },
        });
        s.snapshot_capture.message =
            Some("Capturing the console before creating the snapshot…".into());
        Ok::<_, String>(())
    })();
    s.error = result.err();
}
fn encode(frame: Arc<ConsoleFrame>) -> Result<SnapshotPreviewImage, String> {
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
    s: &State,
    frame: Arc<ConsoleFrame>,
) -> Result<mpsc::Receiver<Result<SnapshotPreviewImage, String>>, String> {
    s.snapshot_capture
        .encoder
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .map_err(|_| "The previous preview encoder is still finishing".to_owned())?;
    let permit = Permit(s.snapshot_capture.encoder.clone());
    let (tx, rx) = mpsc::channel();
    let wake = s.waker.clone();
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
pub(super) fn cancel_dialog(s: &mut State) {
    if s.snapshot_capture
        .pending
        .as_ref()
        .is_some_and(|p| matches!(p.stage, Stage::Capture { .. } | Stage::Encode { .. }))
    {
        s.snapshot_capture.pending = None;
        s.snapshot_capture.message =
            Some("Preview capture cancelled; no snapshot was submitted.".into());
    }
    s.snapshot_dialog = None;
}
fn creation_result(
    record: &xenadmin_core::OperationRecord,
) -> Result<Option<xenadmin_core::SnapshotId>, String> {
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
fn advance(s: &mut State, mut p: Pending) -> Result<Option<Pending>, String> {
    if !current(s, p.vm, p.id) {
        return Err("Preview stopped because its original connection changed. Check Activity for any snapshot already submitted.".into());
    }
    if matches!(p.stage, Stage::Capture { .. } | Stage::Encode { .. })
        && (!matches!(s.snapshot_dialog, Some(vm_snapshots::Dialog::Create { .. }))
            || p.started.elapsed() > Duration::from_secs(30))
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
                    let receiver = spawn(s, latest.frame.as_ref().unwrap().clone())?;
                    drop(console);
                    s.snapshot_capture.message = Some("Preparing the captured preview…".into());
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
                if !available(s, p.id) {
                    return Err("VM state changed before snapshot creation; review again.".into());
                }
                let id = s
                    .submit_tracked_command(command)
                    .map_err(|e| e.to_string())?;
                let operation = s
                    .operations
                    .iter()
                    .find(|o| o.latest().id == id)
                    .cloned()
                    .ok_or("Snapshot operation tracking unavailable")?;
                s.snapshot_dialog = None;
                s.snapshot_name = TextEdit::new("");
                s.snapshot_memory = false;
                s.snapshot_capture.message=Some("Creating the snapshot. The captured image will be attached only after creation is verified.".into());
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
                let connection = s.connection.as_ref().unwrap();
                let latest = connection.latest();
                match latest.observe_snapshot(p.id, id) {
                    Ok(snapshot) => {
                        s.resource_operations
                            .retain(|o| !o.latest().state.terminal());
                        if s.resource_operations.len() >= 64 {
                            return Err("Snapshot created; preview skipped because too many operations are active.".into());
                        }
                        let upload = connection
                            .upload_snapshot_preview(
                                xenadmin_core::OperationId::new(),
                                snapshot,
                                result.id,
                                image,
                            )
                            .map_err(|e| format!("Snapshot created; preview not submitted: {e}"))?;
                        if let Some(wake) = &s.waker {
                            upload.notify_on_change(wake.clone());
                        }
                        s.resource_operations.push(upload);
                        s.snapshot_capture.message = Some(
                            "Snapshot created. Preview upload has a separate result in Activity."
                                .into(),
                        );
                        return Ok(None);
                    }
                    Err(_) => {
                        let since = succeeded.get_or_insert_with(Instant::now);
                        if since.elapsed() > Duration::from_secs(30) {
                            return Err("Snapshot created; preview skipped because its inventory identity was not available in time.".into());
                        }
                    }
                }
            }
            if p.started.elapsed() > Duration::from_secs(1800) {
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
pub(super) fn poll(s: &mut State, tx: &mut Tx) -> bool {
    let Some(p) = s.snapshot_capture.pending.take() else {
        return false;
    };
    let old = s.snapshot_capture.message.clone();
    match advance(s, p) {
        Ok(p) => s.snapshot_capture.pending = p,
        Err(error) => {
            s.snapshot_capture.message = Some(error.clone());
            s.error = Some(error);
        }
    }
    if s.snapshot_capture.pending.is_some() {
        tx.wake_after(Duration::from_millis(250));
    }
    old != s.snapshot_capture.message
}
pub(super) fn scene(s: &mut State) {
    vm_snapshots::edit_scene(s);
    s.snapshot_dialog = None;
    vm_snapshots::begin_create(s);
    s.snapshot_name = TextEdit::new("Before upgrade");
    s.snapshot_description = TextEdit::new("Reviewed recovery point");
    s.snapshot_capture.opt_in = true;
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancelled_or_uncertain_snapshot_never_authorizes_preview_upload() {
        use xenadmin_core::{OperationAction, OperationId, SnapshotOperation, SnapshotWorkflow};
        let snapshot = xenadmin_core::SnapshotId::from(uuid::Uuid::from_u128(3));
        let mut record: xenadmin_core::OperationRecord=serde_json::from_value(serde_json::json!({
            "id":OperationId::new(),"pool":uuid::Uuid::from_u128(1),"vm":uuid::Uuid::from_u128(2),
            "action":OperationAction::Snapshot(SnapshotOperation::Workflow{operation:SnapshotWorkflow::Create{name:"Before upgrade".into(),description:String::new(),memory:false}}),
            "state":OperationState::Succeeded,"progress":null,"detail":"Verified","created_snapshot":snapshot
        })).unwrap();
        assert_eq!(creation_result(&record).unwrap(), Some(snapshot));
        record.cancel_requested = true;
        assert!(
            creation_result(&record).is_err(),
            "even a successful snapshot cannot continue a cancelled workflow"
        );
        record.cancel_requested = false;
        for state in [
            OperationState::OutcomeUnknown,
            OperationState::AcknowledgedUnknown,
            OperationState::PartiallyCompleted,
            OperationState::Failed,
        ] {
            record.state = state;
            assert!(creation_result(&record).is_err());
        }
    }
    #[test]
    fn snapshot_capture_encoder_bounds_large_frames_and_rejects_invalid_data() {
        let frame = Arc::new(ConsoleFrame {
            width: 3200,
            height: 1800,
            rgba: vec![110; 3200 * 1800 * 4].into(),
        });
        let image = encode(frame).unwrap();
        assert_eq!(image.dimensions(), (1600, 900));
        assert!(image.byte_len() < 8 * 1024 * 1024);
        assert!(
            encode(Arc::new(ConsoleFrame {
                width: 20,
                height: 10,
                rgba: vec![1; 10].into()
            }))
            .is_err()
        );
    }
    #[test]
    fn snapshot_preview_opt_in_is_explicit_and_synthetic_confirm_cannot_dispatch() {
        for size in [(1440.0, 900.0), (513.0, 652.0)] {
            let mut state = init();
            scene(&mut state);
            state.snapshot_capture.opt_in = false;
            let mut rt = rune::Runtime::new(
                state,
                super::super::view,
                size,
                rune::Env {
                    system_dark: true,
                    scale: 1.0,
                },
            );
            for _ in 0..30 {
                rt.frame(1.0 / 60.0);
            }
            rt.update(|_, tx| tx.scroll_into_view("snapshot-capture-option"));
            rt.frame(0.0);
            let (x, y) = rt.find("snapshot-capture-option").unwrap();
            rt.pointer_down(x, y);
            rt.pointer_up(x, y);
            rt.frame(0.0);
            assert!(rt.state.snapshot_capture.opt_in);
            assert!(
                rt.accessibility_json()
                    .to_string()
                    .contains("may contain sensitive")
            );
            rt.update(|_, tx| tx.scroll_into_view("confirm-snapshot"));
            rt.frame(0.0);
            let (x, y) = rt.find("confirm-snapshot").unwrap();
            rt.pointer_down(x, y);
            rt.pointer_up(x, y);
            rt.frame(0.0);
            assert!(rt.state.operations.is_empty());
            assert!(rt.state.snapshot_capture.pending.is_none());
        }
    }
}
