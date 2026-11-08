use super::*;
use std::sync::{
    Mutex,
    atomic::{AtomicU64, Ordering},
};

const LIMIT: usize = 8;
#[derive(Clone, Default)]
struct Metadata {
    lease: u64,
    target: Option<console_view::ConsoleTarget>,
    closing: bool,
}
#[derive(Clone)]
struct Slot {
    app: rune::AppHandle<State>,
    metadata: Arc<Mutex<Metadata>>,
}
#[derive(Default)]
pub(super) struct Registry {
    slots: Vec<Slot>,
    seen: Vec<u64>,
    pub show: bool,
    launch_demo: bool,
}
pub(super) struct Seat {
    metadata: Arc<Mutex<Metadata>>,
    lease: u64,
    parent: Option<rune::AppHandle<State>>,
}
struct Payload {
    lease: u64,
    connection: Option<Connection>,
    console: Option<xenadmin_core::Console>,
    snapshot: Option<Arc<xenadmin_core::ConsoleSnapshot>>,
    ui: console_view::ConsoleUi,
    options: settings::Options,
    demo: bool,
    demo_inventory: Option<Arc<Inventory>>,
}
fn empty() -> State {
    init_with_instance(None)
}
fn setup(s: &mut State, waker: Waker) {
    s.waker = Some(waker);
}
fn next_lease() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}
impl Slot {
    fn target(&self) -> Option<console_view::ConsoleTarget> {
        self.metadata.lock().unwrap().target.clone()
    }
    fn reserve(&self, target: console_view::ConsoleTarget) -> Option<u64> {
        let mut meta = self.metadata.lock().unwrap();
        if meta.target.is_some() || meta.closing {
            return None;
        }
        meta.lease = next_lease();
        meta.target = Some(target);
        Some(meta.lease)
    }
    fn show_if(&self, lease: u64) {
        let meta = self.metadata.lock().unwrap();
        if meta.lease == lease && meta.target.is_some() {
            self.app.open();
        }
    }
    fn close_if(&self, lease: u64) {
        let mut meta = self.metadata.lock().unwrap();
        if meta.lease != lease || meta.closing || meta.target.is_none() {
            return;
        }
        meta.target = None;
        meta.closing = true;
        self.app
            .update(move |s, tx| acknowledge_close(s, lease, tx));
    }
    fn close(&self) {
        let lease = self.metadata.lock().unwrap().lease;
        self.close_if(lease);
    }
    fn available(&self) -> bool {
        let meta = self.metadata.lock().unwrap();
        meta.target.is_none() && !meta.closing
    }
}
pub(super) fn register(app: &mut rune::App<State>) {
    let parent = app.handle();
    let mut slots = Vec::new();
    for index in 0..LIMIT {
        let metadata = Arc::new(Mutex::new(Metadata::default()));
        let state_metadata = metadata.clone();
        let close_parent = parent.clone();
        let spec = AppSpec {
            setup: Some(setup),
            poll: Some(poll),
            scenes: &[("console-window", scene)],
            min_size: Some((480.0, 640.0)),
            ..AppSpec::new("XenAdmin RS console", (1100.0, 800.0), empty, view)
        };
        let child = rune::App::new(spec)
            .init(move || {
                let mut s = empty();
                s.console_seat = Some(Seat {
                    metadata: state_metadata,
                    lease: 0,
                    parent: Some(close_parent.clone()),
                });
                s
            })
            .windowless()
            .release_gpu_when_closed()
            .tab_visits_controls(true)
            .modifier_keys()
            .on_focus(|s, focused, _| {
                if !focused {
                    console_view::release(s);
                }
            })
            .on_close(|s, _| {
                closed(s);
                if let Some(parent) = s
                    .console_seat
                    .as_ref()
                    .and_then(|seat| seat.parent.as_ref())
                {
                    parent.wake();
                }
            });
        let (handle, _) = app.window(rune::WindowSpec::from_app(
            format!("console-{}", index + 1),
            child,
        ));
        slots.push(Slot {
            app: handle,
            metadata,
        });
    }
    parent.update(move |s, _| {
        s.console_windows.slots = slots;
        if s.console_windows.launch_demo {
            launch_demo(s);
        }
    });
}
pub(super) fn can_detach(s: &State) -> bool {
    s.console_seat.is_none()
        && s.console_ui.target().is_some()
        && (s.console.is_some() || s.demo)
        && s.console_windows.slots.iter().any(Slot::available)
}
fn take(s: &mut State, lease: u64) -> Payload {
    console_view::moving_window(s);
    let packet = Payload {
        lease,
        connection: s.connection.clone(),
        console: s.console.take(),
        snapshot: s.console_snapshot.take(),
        ui: std::mem::take(&mut s.console_ui),
        options: s.preferences.options,
        demo: s.demo,
        demo_inventory: s.demo.then(|| s.inventory.clone()).flatten(),
    };
    s.console_pixels = rune::Pixels::new();
    packet
}
pub(super) fn detach(s: &mut State) {
    if !can_detach(s) {
        s.error =
            Some("Close an existing console window before opening another (maximum eight).".into());
        return;
    }
    let target = s.console_ui.target().unwrap().clone();
    let reserved = s.console_windows.slots.iter().find_map(|slot| {
        slot.reserve(target.clone())
            .map(|lease| (slot.clone(), lease))
    });
    let Some((slot, lease)) = reserved else {
        return;
    };
    let packet = take(s, lease);
    slot.app.update(move |s, tx| adopt(s, packet, tx));
    slot.app.open();
}
fn adopt(s: &mut State, packet: Payload, tx: &mut Tx) {
    let Some(seat) = s.console_seat.as_ref() else {
        return;
    };
    let expected = seat.metadata.lock().unwrap().clone();
    if expected.lease != packet.lease || expected.target.is_none() || expected.closing {
        return;
    }
    console_view::release(s);
    s.console_seat.as_mut().unwrap().lease = packet.lease;
    s.connection = packet.connection;
    s.console = packet.console;
    s.console_snapshot = packet.snapshot;
    s.console_ui = packet.ui;
    s.preferences.options = packet.options;
    s.demo = packet.demo;
    s.error = None;
    s.snapshot = s.connection.as_ref().map(Connection::latest);
    s.inventory = if s.demo {
        packet.demo_inventory
    } else {
        s.snapshot.as_ref().and_then(|x| x.inventory().cloned())
    };
    s.console_pixels = rune::Pixels::new();
    if let Some(frame) = s.console_snapshot.as_ref().and_then(|v| v.frame.as_ref()) {
        s.console_pixels.set(
            u32::from(frame.width),
            u32::from(frame.height),
            frame.rgba.to_vec(),
        );
    }
    if let (Some(console), Some(waker)) = (&s.console, &s.waker) {
        console.notify_on_change(waker.clone());
    }
    if let Some(target) = s.console_ui.target() {
        tx.set_title(format!("{} · XenAdmin RS console", target.name));
    }
}
fn closed(s: &mut State) {
    console_view::release(s);
    s.console = None;
    s.console_snapshot = None;
    s.console_ui = Default::default();
    s.console_pixels = rune::Pixels::new();
    s.connection = None;
    s.snapshot = None;
    s.inventory = None;
    s.error = None;
    if let Some(seat) = &mut s.console_seat {
        let mut meta = seat.metadata.lock().unwrap();
        if meta.lease == seat.lease {
            meta.target = None;
            meta.closing = false;
        }
        seat.lease = 0;
    }
}
fn acknowledge_close(s: &mut State, lease: u64, tx: &mut Tx) {
    let Some(seat) = &s.console_seat else {
        return;
    };
    let meta = seat.metadata.lock().unwrap().clone();
    if meta.lease != lease || !meta.closing {
        return;
    }
    let metadata = seat.metadata.clone();
    closed(s);
    // Rune drains this update and its shell close synchronously before another window can
    // reserve the slot. The native close hook sees an empty seat; stale queued acknowledgements
    // have their own lease check above. Hidden, never-opened slots need this explicit acknowledgement.
    metadata.lock().unwrap().closing = false;
    tx.close_window();
    if let Some(parent) = s.console_seat.as_ref().and_then(|s| s.parent.as_ref()) {
        parent.wake();
    }
}
pub(super) fn close_all(s: &mut State) {
    for slot in &s.console_windows.slots {
        slot.close();
    }
}
pub(super) fn poll_registry(s: &mut State) -> bool {
    let current = s
        .console_windows
        .slots
        .iter()
        .map(|slot| {
            let meta = slot.metadata.lock().unwrap();
            if meta.target.is_some() { meta.lease } else { 0 }
        })
        .collect::<Vec<_>>();
    if current == s.console_windows.seen {
        return false;
    }
    s.console_windows.seen = current;
    true
}
fn poll(s: &mut State, clock: f64, tx: &mut Tx) -> bool {
    if s.console_ui.target().is_none() {
        return false;
    }
    // The connection is shared with the main workspace. Bounded local polling avoids adding
    // a permanent connection subscription every time this reusable window opens.
    tx.wake_after(std::time::Duration::from_millis(250));
    let mut changed = console_view::poll_export(s);
    if let Some(connection) = &s.connection {
        let snapshot = connection.latest();
        if s.snapshot
            .as_ref()
            .is_none_or(|old| !Arc::ptr_eq(old, &snapshot))
        {
            s.inventory = snapshot.inventory().cloned();
            s.snapshot = Some(snapshot);
            changed = true;
        }
    }
    if let Some(console) = &s.console {
        let latest = console.latest();
        if s.console_snapshot
            .as_ref()
            .is_none_or(|old| !Arc::ptr_eq(old, &latest))
        {
            if let Some(frame) = &latest.frame
                && s.console_snapshot
                    .as_ref()
                    .and_then(|old| old.frame.as_ref())
                    .is_none_or(|old| !Arc::ptr_eq(old, frame))
            {
                s.console_pixels.set(
                    u32::from(frame.width),
                    u32::from(frame.height),
                    frame.rgba.to_vec(),
                );
            }
            s.console_snapshot = Some(latest);
            changed = true;
        }
    }
    console_view::poll_recovery(s, clock, tx) | changed
}
fn view(s: &State, cx: &mut Cx<State>) -> El {
    let k = Kit::neutral(s.preferences.options.appearance.dark(cx.system_dark()));
    cx.reduced_motion(s.preferences.options.reduced_motion || cx.system_reduced_motion());
    let body = if s.console_ui.target().is_some() {
        console_view::view(s, &k)
    } else {
        col()
            .padding(24.0)
            .child(heading("Console closed", &k))
            .child(caption("Open a console from the main window.", &k))
    };
    rune_kit::root(
        col()
            .w_full()
            .h_full()
            .fill(k.c.bg_base)
            .child(body.grow(1.0))
            .children(s.error.as_ref().map(|e| caption(e, &k).wrap())),
        [],
    )
    .on_layout_tx(|s: &mut State, rect, _| {
        s.window_width = rect[2];
        s.window_height = rect[3];
    })
}
pub(super) fn button(s: &State, k: &Kit) -> El {
    let count = s
        .console_windows
        .slots
        .iter()
        .filter(|slot| slot.target().is_some())
        .count();
    Button::new(format!("Console windows ({count})"))
        .on_press(|s: &mut State| s.console_windows.show = true)
        .build(k)
        .key("console-windows")
}
pub(super) fn dialog(s: &State, k: &Kit) -> El {
    let mut body=col().gap(12.0).child(heading("Independent console windows",k))
        .child(caption("Up to eight consoles can stay open while you browse other guests or pools. Closing a console releases its input and connection to the display. Closing the main application closes every console.",k).wrap());
    let mut count = 0;
    for (index, slot) in s.console_windows.slots.iter().enumerate() {
        let metadata = slot.metadata.lock().unwrap().clone();
        if let Some(target) = metadata.target {
            let lease = metadata.lease;
            count += 1;
            body = body
                .child(
                    caption(
                        format!(
                            "{}\nPool {} · {} {}",
                            target.name,
                            target.pool,
                            if target.host.is_some() {
                                "Host console VM"
                            } else {
                                "VM"
                            },
                            target.vm
                        ),
                        k,
                    )
                    .wrap_anywhere(),
                )
                .child(
                    row()
                        .gap(8.0)
                        .child(
                            Button::new("Show window")
                                .on_press({
                                    let slot = slot.clone();
                                    move |_: &mut State| slot.show_if(lease)
                                })
                                .build(k)
                                .key(format!("show-console-window-{index}")),
                        )
                        .child(
                            Button::new("Close console")
                                .on_press({
                                    let slot = slot.clone();
                                    move |_: &mut State| slot.close_if(lease)
                                })
                                .build(k)
                                .key(format!("close-console-window-{index}")),
                        ),
                );
        }
    }
    if count == 0 {
        body = body.child(
            caption(
                "Open a VM or host console, then choose Open in separate window.",
                k,
            )
            .wrap(),
        );
    }
    el().absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .modal(false, |s: &mut State| s.console_windows.show = false)
        .fill(rune::Color::BLACK.alpha(0.6))
        .center()
        .child(
            col()
                .w((s.window_width - 32.0).clamp(280.0, 760.0))
                .h((s.window_height - 40.0).clamp(300.0, 820.0))
                .padding(20.0)
                .gap(12.0)
                .fill(k.c.bg_base)
                .radius(10.0)
                .child(body.grow(1.0).min_h(0.0).scroll(rune::ScrollSpec {
                    stick_bottom: false,
                }))
                .child(
                    row()
                        .gap(8.0)
                        .child(
                            Button::new("Done")
                                .on_press(|s: &mut State| s.console_windows.show = false)
                                .build(k)
                                .key("console-windows-close"),
                        )
                        .child(
                            Button::new("Close all consoles")
                                .disabled(count == 0)
                                .on_press(close_all)
                                .build(k)
                                .key("console-windows-close-all"),
                        ),
                ),
        )
}
fn scene(s: &mut State) {
    console_view::recovery_scene(s);
    sample_frame(s, 60);
    if s.console_seat.is_none() {
        s.console_seat = Some(Seat {
            metadata: Arc::new(Mutex::new(Metadata::default())),
            lease: 0,
            parent: None,
        });
    }
}

fn sample_frame(s: &mut State, tint: u8) {
    let width = 640u16;
    let height = 360u16;
    let rgba = (0..usize::from(width) * usize::from(height))
        .flat_map(|n| {
            [
                tint,
                (n % usize::from(width) / 5) as u8,
                (n / usize::from(width) / 3) as u8,
                255,
            ]
        })
        .collect::<Vec<_>>();
    let frame = Arc::new(xenadmin_core::ConsoleFrame {
        width,
        height,
        rgba: rgba.into(),
    });
    s.console_pixels
        .set(u32::from(width), u32::from(height), frame.rgba.to_vec());
    s.console_snapshot = Some(Arc::new(xenadmin_core::ConsoleSnapshot {
        state: xenadmin_core::ConsoleState::Closed,
        frame: Some(frame),
        cursor: None,
        guest_clipboard: None,
        clipboard_utf8: false,
        extended_keys: false,
    }));
}
pub(super) fn multiple_scene(s: &mut State) {
    demo(s);
    s.console_windows.launch_demo = true;
}
fn launch_demo(s: &mut State) {
    let Some(inventory) = s.inventory.clone() else {
        return;
    };
    let guests = inventory
        .vms
        .values()
        .filter(|v| !v.is_control_domain && !v.is_template)
        .take(2)
        .map(|v| (v.id, v.name.clone()))
        .collect::<Vec<_>>();
    for (index, (vm, name)) in guests.into_iter().enumerate() {
        console_view::opened(s, inventory.pool_id, vm, name);
        sample_frame(s, 40 + index as u8 * 50);
        detach(s);
        if let Some(slot) = s.console_windows.slots.get(index) {
            slot.app.set_position((20 + index as i32 * 1120, 40));
        }
    }
    s.console_windows.launch_demo = false;
    s.console_windows.show = true;
}

#[cfg(test)]
mod tests {
    use super::*;
    fn slot() -> Slot {
        Slot {
            app: rune::App::new(AppSpec::new("fixture", (640.0, 700.0), empty, view)).handle(),
            metadata: Arc::new(Mutex::new(Metadata::default())),
        }
    }
    fn child(slot: &Slot, size: (f32, f32)) -> rune::Runtime<State> {
        let mut s = empty();
        s.console_seat = Some(Seat {
            metadata: slot.metadata.clone(),
            lease: 0,
            parent: None,
        });
        let mut rt = rune::Runtime::new(
            s,
            view,
            size,
            rune::Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        settle(&mut rt);
        rt
    }
    fn settle(rt: &mut rune::Runtime<State>) {
        for _ in 0..20 {
            rt.frame(1.0 / 60.0);
        }
    }
    fn click(rt: &mut rune::Runtime<State>, key: &str) {
        rt.update(|_, tx| tx.scroll_into_view(key));
        settle(rt);
        let (x, y) = rt.find(key).unwrap_or_else(|| panic!("missing {key}"));
        assert!(
            x >= 0.0 && x <= rt.state.window_width && y >= 0.0 && y <= rt.state.window_height,
            "{key} ({x},{y}) is in bounds {}×{}",
            rt.state.window_width,
            rt.state.window_height
        );
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        settle(rt);
    }
    fn prepared(tint: u8) -> State {
        let mut s = empty();
        console_view::recovery_scene(&mut s);
        sample_frame(&mut s, tint);
        s
    }
    #[test]
    fn independent_console_windows_keep_targets_frames_and_input_state_separate() {
        for size in [(513.0, 652.0), (1100.0, 800.0)] {
            let mut main = prepared(40);
            let first = slot();
            let second = slot();
            let target = main.console_ui.target().unwrap().clone();
            let lease = first.reserve(target.clone()).unwrap();
            main.console_buttons = 7;
            main.console_keys.push((rune::PhysicalKeyCode::KeyA, 0x61));
            let packet = take(&mut main, lease);
            assert!(main.console_ui.target().is_none());
            assert_eq!(main.console_buttons, 0);
            assert!(main.console_keys.is_empty());
            let mut a = child(&first, size);
            a.update(|s, tx| adopt(s, packet, tx));
            settle(&mut a);
            console_view::recovery_scene(&mut main);
            sample_frame(&mut main, 90);
            let inventory = main.inventory.clone().unwrap();
            let other = inventory
                .vms
                .values()
                .find(|v| v.id != target.vm && !v.is_control_domain && !v.is_template)
                .unwrap();
            console_view::opened(&mut main, inventory.pool_id, other.id, other.name.clone());
            sample_frame(&mut main, 90);
            let other_target = main.console_ui.target().unwrap().clone();
            let packet = take(&mut main, second.reserve(other_target.clone()).unwrap());
            let mut b = child(&second, size);
            b.update(|s, tx| adopt(s, packet, tx));
            settle(&mut b);
            demo(&mut main);
            assert_eq!(a.state.console_ui.target().unwrap().vm, target.vm);
            assert_eq!(b.state.console_ui.target().unwrap().vm, other_target.vm);
            assert_eq!(
                a.state
                    .console_snapshot
                    .as_ref()
                    .unwrap()
                    .frame
                    .as_ref()
                    .unwrap()
                    .rgba[0],
                40
            );
            assert_eq!(
                b.state
                    .console_snapshot
                    .as_ref()
                    .unwrap()
                    .frame
                    .as_ref()
                    .unwrap()
                    .rgba[0],
                90
            );
            a.state.console_buttons = 4;
            b.state.console_buttons = 2;
            a.update(|s, _| console_view::release(s));
            assert_eq!(a.state.console_buttons, 0);
            assert_eq!(b.state.console_buttons, 2);
            click(&mut a, "close-console");
            a.update(|s, _| closed(s));
            assert!(a.state.console_snapshot.is_none() && a.state.connection.is_none());
            assert!(first.target().is_none());
            assert!(second.target().is_some());
            assert_eq!(b.state.console_ui.target().unwrap().vm, other_target.vm);
        }
    }
    #[test]
    fn reusable_slot_rejects_old_adoption_close_and_focus_requests() {
        let mut main = prepared(20);
        let slot = slot();
        let target = main.console_ui.target().unwrap().clone();
        let old = slot.reserve(target.clone()).unwrap();
        let packet = take(&mut main, old);
        slot.close_if(old);
        assert!(
            slot.reserve(target.clone()).is_none(),
            "close must be acknowledged before reuse"
        );
        let mut closing = child(&slot, (513.0, 652.0));
        closing.update(|s, tx| acknowledge_close(s, old, tx));
        let new = slot.reserve(target).unwrap();
        assert_ne!(old, new);
        slot.close_if(old);
        slot.show_if(old);
        assert!(slot.target().is_some());
        let mut rt = child(&slot, (513.0, 652.0));
        rt.update(|s, tx| adopt(s, packet, tx));
        assert!(rt.state.console_ui.target().is_none());
        rt.state.console_seat.as_mut().unwrap().lease = old;
        rt.update(|s, _| closed(s));
        assert_eq!(slot.metadata.lock().unwrap().lease, new);
        assert!(slot.target().is_some());
    }
    #[test]
    fn window_capacity_is_bounded_and_closing_returns_the_slot() {
        let mut s = prepared(25);
        s.console_windows.slots = (0..LIMIT).map(|_| slot()).collect();
        assert!(can_detach(&s));
        for slot in &s.console_windows.slots {
            slot.reserve(s.console_ui.target().unwrap().clone())
                .unwrap();
        }
        assert!(!can_detach(&s));
        s.console_windows.slots[2].close();
        assert!(!can_detach(&s));
        let slot = s.console_windows.slots[2].clone();
        let lease = slot.metadata.lock().unwrap().lease;
        let mut child = child(&slot, (513.0, 652.0));
        child.update(|s, tx| acknowledge_close(s, lease, tx));
        assert!(can_detach(&s));
        close_all(&mut s);
        assert!(
            s.console_windows
                .slots
                .iter()
                .all(|slot| slot.target().is_none())
        );
        assert!(s._instance.is_none() && s.backend.is_err());
    }
    #[test]
    fn manager_is_reachable_with_eight_targets_at_compact_and_wide_sizes() {
        for size in [(513.0, 652.0), (1440.0, 900.0)] {
            let mut s = prepared(30);
            s.console_windows.slots = (0..LIMIT).map(|_| slot()).collect();
            let target = s.console_ui.target().unwrap().clone();
            for slot in &s.console_windows.slots {
                slot.reserve(target.clone()).unwrap();
            }
            s.console_ui = Default::default();
            s.console_snapshot = None;
            let mut rt = rune::Runtime::new(
                s,
                crate::view,
                size,
                rune::Env {
                    system_dark: true,
                    scale: 1.0,
                },
            );
            settle(&mut rt);
            click(&mut rt, "console-windows");
            assert!(rt.state.console_windows.show);
            click(&mut rt, "close-console-window-7");
            assert!(rt.state.console_windows.slots[7].target().is_none());
            click(&mut rt, "console-windows-close-all");
            assert!(
                rt.state
                    .console_windows
                    .slots
                    .iter()
                    .all(|slot| slot.target().is_none())
            );
            click(&mut rt, "console-windows-close");
            assert!(!rt.state.console_windows.show);
        }
    }
    #[test]
    fn moving_console_invalidates_old_file_dialog_without_disabling_new_window_export() {
        let mut main = rune::Runtime::new(
            prepared(30),
            view,
            (1100.0, 800.0),
            rune::Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        settle(&mut main);
        click(&mut main, "save-console-screenshot");
        assert!(main.file_dialog().is_some());
        let slot = slot();
        let lease = slot
            .reserve(main.state.console_ui.target().unwrap().clone())
            .unwrap();
        let mut packet = None;
        main.update(|s, _| packet = Some(take(s, lease)));
        let mut child = child(&slot, (1100.0, 800.0));
        child.update(|s, tx| adopt(s, packet.take().unwrap(), tx));
        settle(&mut child);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("obsolete.png");
        main.resolve_file_dialog(Some(path.clone())).unwrap();
        settle(&mut main);
        assert!(!path.exists());
        click(&mut child, "save-console-screenshot");
        assert!(child.file_dialog().is_some());
        child.resolve_file_dialog(None).unwrap();
    }
}
