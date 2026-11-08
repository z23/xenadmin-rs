use super::*;
use std::{
    collections::BTreeSet,
    io::Cursor,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};
use xenadmin_core::{ClassAvailability, ObservedSnapshot, SnapshotId, SnapshotParent, VdiId};

struct Decoded {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}
#[derive(Default)]
pub(super) struct Decoder {
    busy: Arc<AtomicBool>,
}
struct DecodePermit(Arc<AtomicBool>);
impl Drop for DecodePermit {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
impl Decoder {
    fn submit(
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
pub(super) struct Panel {
    inventory: Arc<Inventory>,
    vm: VmId,
    snapshot: SnapshotId,
    current: bool,
    synthetic: bool,
    observed: Option<ObservedSnapshot>,
    query: Option<xenadmin_core::SnapshotPreviewQuery>,
    decode: Option<mpsc::Receiver<Result<Decoded, String>>>,
    pixels: rune::Pixels,
    dimensions: Option<(u32, u32)>,
    message: String,
}
pub(super) fn open(s: &mut State) {
    let Some(snapshot) = vm_snapshots::selected_snapshot(s) else {
        return;
    };
    let Some(vm) = snapshot.vm else { return };
    let Some(inventory) = s.inventory.clone() else {
        return;
    };
    let id = snapshot.id;
    console_view::release(s);
    s.snapshot_inspection = Some(Panel {
        inventory, vm, snapshot: id,
        current: s.snapshot.as_ref().is_some_and(|v|v.is_current()),
        synthetic: s.demo,
        observed: None, query: None, decode: None,
        pixels: rune::Pixels::new(), dimensions: None,
        message: "Load an existing saved preview from the host. It may contain sensitive guest information.".into(),
    });
}
fn request(s: &mut State) {
    let Some(p) = &s.snapshot_inspection else {
        return;
    };
    if p.query.is_some() || p.decode.is_some() {
        return;
    }
    let result = (|| {
        let snapshot = s.snapshot.as_ref().ok_or(xenadmin_core::Error::Stale)?;
        if snapshot
            .inventory()
            .is_none_or(|i| i.pool_id != p.inventory.pool_id)
        {
            return Err(xenadmin_core::Error::Stale);
        }
        let observed = snapshot.observe_snapshot(p.vm, p.snapshot)?;
        let query = s
            .connection
            .as_ref()
            .ok_or(xenadmin_core::Error::Closed)?
            .snapshot_preview(observed)?;
        if let Some(wake) = &s.waker {
            query.notify_on_change(wake.clone());
        }
        Ok((observed, query))
    })();
    let p = s.snapshot_inspection.as_mut().unwrap();
    match result {
        Ok((observed, query)) => {
            p.observed = Some(observed);
            p.query = Some(query);
            p.message = "Loading saved preview…".into();
            p.dimensions = None;
            p.pixels = rune::Pixels::new();
        }
        Err(e) => p.message = e.to_string(),
    }
}
fn decode(image: xenadmin_core::SnapshotPreview) -> Result<Decoded, String> {
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
pub(super) fn poll(s: &mut State) -> bool {
    let Some(p) = &mut s.snapshot_inspection else {
        return false;
    };
    if (p.query.is_some() || p.decode.is_some())
        && p.observed.is_some_and(|o| {
            s.snapshot
                .as_ref()
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
                match s
                    .snapshot_decoder
                    .submit(move || decode(image), s.waker.clone())
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
                Ok(image) => {
                    p.pixels.set(image.width, image.height, image.rgba);
                    p.dimensions = Some((image.width, image.height));
                    p.message =
                        "Saved snapshot preview · may differ from the current guest display".into();
                }
                Err(e) => p.message = e,
            }
        }
    }
    changed
}
fn field(label: impl Into<String>, value: impl Into<String>, k: &Kit) -> El {
    col().gap(3.0).child(caption(label, k)).child(
        text(value.into())
            .font(k.ty.text_small)
            .color(k.c.text_base)
            .wrap_anywhere()
            .selectable(true),
    )
}
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
fn disk(i: &Inventory, id: VdiId, k: &Kit) -> El {
    let Some(d) = i.details.vdis.get(&id) else {
        return field("Disk unavailable", id.to_string(), k);
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
    col()
        .gap(6.0)
        .child(field(d.name.clone(), id.to_string(), k))
        .child(field("Storage repository", sr, k))
        .child(field(
            "Virtual capacity / reported allocation",
            format!(
                "{} / {}",
                size(d.virtual_size),
                size(d.physical_utilisation)
            ),
            k,
        ))
        .child(field(
            "Disk ancestry",
            d.snapshot_of
                .map(|id| format!("Snapshot of VDI {id}"))
                .unwrap_or_else(|| "None or not reported".into()),
            k,
        ))
        .child(field(
            "Read only / sharable / managed / missing",
            format!(
                "{} / {} / {} / {}",
                yes(d.read_only),
                yes(d.sharable),
                yes(d.managed),
                yes(d.missing)
            ),
            k,
        ))
}
pub(super) fn dialog(s: &State, k: &Kit) -> El {
    let Some(p) = &s.snapshot_inspection else {
        return el();
    };
    let i = &p.inventory;
    let snapshot = i.snapshots.get(&p.snapshot);
    let details = i.details.snapshots.get(&p.snapshot);
    let ancestry = i
        .snapshot_ancestry(p.vm)
        .into_iter()
        .find(|row| row.id == p.snapshot);
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
    let mut body = col()
        .gap(12.0)
        .child(field(
            snapshot
                .map(|v| v.name.as_str())
                .unwrap_or("Snapshot unavailable"),
            p.snapshot.to_string(),
            k,
        ))
        .child(
            caption(
                format!(
                    "Captured pool {} · revision {} · {}",
                    i.pool_id,
                    i.revision,
                    if p.synthetic {
                        "Synthetic inventory"
                    } else if p.current {
                        "Current at capture"
                    } else {
                        "Stale at capture"
                    }
                ),
                k,
            )
            .wrap_anywhere(),
        )
        .child(field(
            "VM",
            format!(
                "{} ({})",
                i.vms
                    .get(&p.vm)
                    .map(|vm| vm.name.as_str())
                    .unwrap_or("Unavailable"),
                p.vm
            ),
            k,
        ))
        .child(field("Parent snapshot", parent, k));
    if let Some(row) = ancestry {
        if row.current_parent {
            body = body.child(caption("The VM currently descends from this snapshot.", k).wrap());
        }
        if let Some(issue) = row.issue {
            body = body.child(caption(issue, k).wrap());
        }
    }
    body = body.child(heading("Virtual disks", k));
    let complete = ["vdi", "vbd"]
        .iter()
        .all(|c| i.details.classes.get(*c) == Some(&ClassAvailability::Available));
    if !complete {
        body = body.child(
            caption(
                "Disk inventory is incomplete; sizes and attachments may be missing.",
                k,
            )
            .wrap(),
        );
    }
    let mut ids = BTreeSet::new();
    let mut unresolved = 0usize;
    for id in i
        .details
        .snapshot_disks
        .get(&p.snapshot)
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
    for id in &ids {
        body = body.child(disk(i, *id, k));
    }
    if ids.is_empty() {
        body =
            body.child(caption("No resolved attached disks in this captured inventory.", k).wrap());
    }
    if unresolved > 0 {
        body = body.child(
            caption(
                format!("{unresolved} attachment(s) could not be resolved."),
                k,
            )
            .wrap(),
        );
    }
    let total = ids.iter().try_fold(0u64, |sum, id| {
        sum.checked_add(i.details.vdis.get(id)?.physical_utilisation?)
    });
    body=body.child(field("Sum of reported disk allocations",if complete && unresolved==0 {size(total)}else{"Incomplete inventory".into()},k))
        .child(caption("Allocations are reported by the storage backend. Shared blocks and snapshot chains mean this is not exclusive usage or the space deletion would reclaim.",k).wrap());
    if let Some(id) = details.and_then(|d| d.memory_vdi) {
        body = body
            .child(heading("Memory checkpoint disk", k))
            .child(disk(i, id, k));
    } else if details.is_none_or(|d| !d.memory_vdi_known) {
        body = body.child(caption("Memory checkpoint disk information is unavailable.", k).wrap());
    }
    body = body
        .child(heading("Saved preview", k))
        .child(caption(&p.message, k).wrap());
    if let Some((width, height)) = p.dimensions {
        let height = (height as f32 / width as f32 * (s.window_width - 100.0).clamp(200.0, 640.0))
            .clamp(120.0, 360.0);
        body = body.child(
            el().w_full()
                .h(height)
                .min_h(height)
                .pixels_fit(&p.pixels, rune::SurfaceFit::Fit)
                .key("snapshot-preview-image"),
        );
    }
    let card = col()
        .w((s.window_width - 32.0).clamp(280.0, 780.0))
        .h((s.window_height - 48.0).clamp(280.0, 850.0))
        .min_h(0.0)
        .padding(20.0)
        .gap(12.0)
        .radius(10.0)
        .fill(k.c.bg_base)
        .child(heading("Snapshot information", k))
        .child(body.grow(1.0).min_h(0.0).scroll(rune::ScrollSpec {
            stick_bottom: false,
        }))
        .child(
            row()
                .gap(8.0)
                .flex_wrap()
                .child(
                    Button::new("Load saved preview")
                        .disabled(p.synthetic || s.demo || p.query.is_some() || p.decode.is_some())
                        .on_press(request)
                        .build(k)
                        .key("snapshot-preview-load"),
                )
                .child(
                    Button::new("Close")
                        .on_press(|s: &mut State| s.snapshot_inspection = None)
                        .build(k)
                        .key("snapshot-inspection-close"),
                ),
        );
    el().w_full()
        .h_full()
        .absolute(0.0, 0.0)
        .center()
        .fill(rune::Color::BLACK.alpha(0.5))
        .role("dialog")
        .label("Snapshot information")
        .modal(false, |s: &mut State| s.snapshot_inspection = None)
        .child(card)
        .key("snapshot-inspection-dialog")
}

pub(super) fn ancestry_scene(s: &mut State) {
    demo(s);
    let vm = VmId::parse("30000000-0000-0000-0000-000000000000").unwrap();
    let i = Arc::make_mut(s.inventory.as_mut().unwrap());
    let first = SnapshotId::parse("60000000-0000-0000-0000-000000000000").unwrap();
    let second = SnapshotId::parse("60000000-0000-0000-0000-000000000001").unwrap();
    i.details.snapshots.insert(
        first,
        xenadmin_core::SnapshotDetails {
            can_export: None,
            parent: SnapshotParent::Root,
            memory_vdi_known: true,
            ..Default::default()
        },
    );
    i.details.snapshots.insert(
        second,
        xenadmin_core::SnapshotDetails {
            can_export: None,
            parent: SnapshotParent::Snapshot(first),
            memory_vdi_known: true,
            preview_available: true,
            ..Default::default()
        },
    );
    i.details.vms.entry(vm).or_default().snapshot_parent = SnapshotParent::Snapshot(second);
    if let Some(vbd) = i
        .details
        .vm_disks
        .get(&vm)
        .and_then(|ids| ids.first())
        .and_then(|id| i.details.vbds.get(id))
        .cloned()
    {
        let mut vbd = vbd;
        vbd.id = uuid::Uuid::from_u128(0x70000000000000000000000000000001).into();
        vbd.vm = None;
        vbd.snapshot = Some(second);
        i.details.snapshot_disks.insert(second, vec![vbd.id]);
        i.details.vbds.insert(vbd.id, vbd);
    }
    s.table.select(vm.to_string());
    vm_snapshots::open(s);
    s.snapshot_table.select(second.to_string());
}
pub(super) fn scene(s: &mut State) {
    ancestry_scene(s);
    open(s);
    let p = s.snapshot_inspection.as_mut().unwrap();
    let mut rgba = vec![0u8; 320 * 180 * 4];
    for y in 0..180 {
        for x in 0..320 {
            let index = (y * 320 + x) * 4;
            rgba[index..index + 4].copy_from_slice(if (x / 40 + y / 30) % 2 == 0 {
                &[25, 65, 94, 255]
            } else {
                &[239, 133, 48, 255]
            });
        }
    }
    p.pixels.set(320, 180, rgba);
    p.dimensions = Some((320, 180));
    p.message = "Synthetic preview fixture · no guest content".into();
}
#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageEncoder;
    fn runtime(mut s: State, size: (f32, f32)) -> rune::Runtime<State> {
        s.window_width = size.0;
        s.window_height = size.1;
        let mut r = rune::Runtime::new(
            s,
            crate::view,
            size,
            rune::Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        for _ in 0..30 {
            r.frame(0.016);
        }
        r
    }
    fn click(r: &mut rune::Runtime<State>, key: &str) {
        let (x, y) = r.find(key).unwrap();
        r.pointer_down(x, y);
        r.pointer_up(x, y);
        for _ in 0..30 {
            r.frame(0.016);
        }
    }
    #[test]
    fn snapshot_inspection_captures_identity_and_close_is_reachable_at_both_sizes() {
        for size in [(513.0, 652.0), (1440.0, 900.0)] {
            let mut s = init();
            ancestry_scene(&mut s);
            let selected = s.snapshot_table.selected().next().unwrap().to_owned();
            let mut r = runtime(s, size);
            click(&mut r, "inspect-snapshot");
            let captured = r.state.snapshot_inspection.as_ref().unwrap().snapshot;
            assert_eq!(captured.to_string(), selected);
            r.update(|s, _| {
                s.snapshot_table
                    .select("60000000-0000-0000-0000-000000000000");
                s.snapshot_view = None;
            });
            r.frame(0.0);
            assert_eq!(
                r.state.snapshot_inspection.as_ref().unwrap().snapshot,
                captured
            );
            let tree = r.accessibility_json().to_string();
            assert!(tree.contains("Snapshot information"));
            assert!(tree.contains("Parent snapshot"));
            assert!(tree.contains("reclaim"));
            click(&mut r, "snapshot-inspection-close");
            assert!(r.state.snapshot_inspection.is_none());
        }
    }
    #[test]
    fn tree_and_list_keep_selection_and_show_current_branch() {
        let mut s = init();
        ancestry_scene(&mut s);
        let selected = s.snapshot_table.selected().next().unwrap().to_owned();
        let mut r = runtime(s, (1440.0, 900.0));
        assert!(r.accessibility_json().to_string().contains("current base"));
        click(&mut r, "snapshot-tree-toggle");
        assert!(!r.state.snapshot_tree);
        assert_eq!(
            r.state.snapshot_table.selected().next(),
            Some(selected.as_str())
        );
        click(&mut r, "snapshot-tree-toggle");
        assert!(r.state.snapshot_tree);
        assert_eq!(
            r.state.snapshot_table.selected().next(),
            Some(selected.as_str())
        );
    }
    #[test]
    fn decoder_bounds_dimensions_and_supports_png_and_jpeg() {
        for format in [image::ImageFormat::Png, image::ImageFormat::Jpeg] {
            let mut bytes = Cursor::new(Vec::new());
            image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
                2,
                2,
                image::Rgb([20, 40, 60]),
            ))
            .write_to(&mut bytes, format)
            .unwrap();
            let image = xenadmin_core::SnapshotPreview {
                mime: if format == image::ImageFormat::Png {
                    "image/png"
                } else {
                    "image/jpeg"
                },
                bytes: bytes.into_inner().into(),
            };
            let result = decode(image).unwrap();
            assert_eq!((result.width, result.height), (2, 2));
            assert_eq!(result.rgba.len(), 16);
        }
        let mut encoded = Vec::new();
        image::codecs::png::PngEncoder::new(&mut encoded)
            .write_image(&vec![0; 4097 * 4], 4097, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        assert!(
            decode(xenadmin_core::SnapshotPreview {
                mime: "image/png",
                bytes: encoded.into()
            })
            .is_err()
        );
        assert!(
            decode(xenadmin_core::SnapshotPreview {
                mime: "image/png",
                bytes: vec![0; 20].into()
            })
            .is_err()
        );
    }
    #[test]
    fn closing_or_replacing_the_panel_does_not_release_a_running_decoder_slot() {
        let mut s = init();
        scene(&mut s);
        let (release, wait) = mpsc::channel();
        let receiver = s
            .snapshot_decoder
            .submit(
                move || {
                    wait.recv().unwrap();
                    Ok(Decoded {
                        width: 1,
                        height: 1,
                        rgba: vec![0; 4],
                    })
                },
                None,
            )
            .unwrap();
        s.snapshot_inspection.as_mut().unwrap().decode = Some(receiver);
        s.snapshot_inspection = None;
        scene(&mut s);
        assert!(
            s.snapshot_decoder
                .submit(|| panic!("a second decoder must not start"), None)
                .is_err()
        );
        release.send(()).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while s.snapshot_decoder.busy.load(Ordering::Acquire) {
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        let result = s
            .snapshot_decoder
            .submit(
                || {
                    Ok(Decoded {
                        width: 1,
                        height: 1,
                        rgba: vec![1; 4],
                    })
                },
                None,
            )
            .unwrap()
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap()
            .unwrap();
        assert_eq!(result.rgba, vec![1; 4]);
    }
    #[test]
    fn preview_request_refuses_a_changed_or_disconnected_workspace() {
        let mut s = init();
        scene(&mut s);
        s.demo = false;
        request(&mut s);
        let p = s.snapshot_inspection.as_ref().unwrap();
        assert!(p.query.is_none());
        assert!(p.observed.is_none());
        assert!(!p.message.is_empty());
    }
}
