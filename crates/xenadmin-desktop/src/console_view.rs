use super::{State, caption, heading};
use rune::{
    El, Key, KeyEvent, Modifier, Phase, PhysicalInput, PhysicalKeyCode, PointerButton, SurfaceFit,
    Tx, col, el, row,
};
use rune_kit::{Button, ButtonVariant, Checkbox, Kit};
use std::sync::atomic::{AtomicU64, Ordering};
use xenadmin_core::{ConsoleInput, ConsoleState, PoolId, VmId};

#[path = "console_export.rs"]
mod export;
pub(super) use export::poll as poll_export;
#[path = "console_recovery.rs"]
mod recovery;
pub(super) use recovery::poll as poll_recovery;

#[derive(Clone)]
pub(super) struct ConsoleTarget {
    pub pool: PoolId,
    pub vm: VmId,
    pub name: String,
    pub host: Option<xenadmin_core::HostId>,
}
#[derive(Default)]
pub(super) struct ConsoleUi {
    target: Option<ConsoleTarget>,
    recovery: recovery::Recovery,
    serial: u64,
    actual_size: bool,
    pointer: [f32; 2],
    wheel: [f32; 2],
    message: Option<String>,
    export: export::Export,
}
impl ConsoleUi {
    pub fn target(&self) -> Option<&ConsoleTarget> {
        self.target.as_ref()
    }
}
pub(super) fn opened(s: &mut State, pool: PoolId, vm: VmId, name: String) {
    release(s);
    s.show_details = false;
    s.console_ui = ConsoleUi {
        target: Some(ConsoleTarget {
            pool,
            vm,
            name,
            host: None,
        }),
        serial: next_serial(),
        actual_size: s.preferences.options.console_actual_size,
        recovery: recovery::Recovery::with_enabled(s.preferences.options.console_reconnect),
        ..Default::default()
    };
    s.console_pixels = rune::Pixels::new();
    recovery::opened(s);
}

pub(super) fn opened_host(
    s: &mut State,
    pool: PoolId,
    host: xenadmin_core::HostId,
    vm: VmId,
    name: String,
) {
    opened(s, pool, vm, name);
    s.console_ui.target.as_mut().unwrap().host = Some(host);
    recovery::opened(s);
}
pub(super) fn host(s: &State) -> Option<xenadmin_core::HostId> {
    if s.console.is_some() || s.demo {
        s.console_ui.target().and_then(|t| t.host)
    } else {
        None
    }
}

pub(super) fn recovery_scene(s: &mut State) {
    crate::demo(s);
    let inventory = s.inventory.as_ref().unwrap();
    let vm = inventory
        .vms
        .values()
        .find(|vm| vm.power_state == xenadmin_core::PowerState::Running)
        .unwrap();
    let (pool, id, name) = (inventory.pool_id, vm.id, vm.name.clone());
    opened(s, pool, id, name);
    s.console_snapshot = Some(std::sync::Arc::new(xenadmin_core::ConsoleSnapshot {
        state: ConsoleState::Closed,
        frame: None,
        cursor: None,
        guest_clipboard: None,
        clipboard_utf8: false,
        extended_keys: false,
    }));
}

/// A connected console showing a synthetic 800 by 600 frame. The recovery scene has no
/// frame, so without this the headless renders never draw the console canvas itself.
pub(super) fn frame_scene(s: &mut State) {
    recovery_scene(s);
    let (width, height) = (800_u16, 600_u16);
    let mut rgba = Vec::with_capacity(usize::from(width) * usize::from(height) * 4);
    for y in 0..height {
        for x in 0..width {
            // A light border marks the frame's edges; the gradient shows its orientation.
            let edge = x < 4 || y < 4 || x >= width - 4 || y >= height - 4;
            let (r, g, b) = if edge {
                (230, 230, 230)
            } else {
                (
                    (u32::from(x) * 255 / u32::from(width)) as u8,
                    (u32::from(y) * 255 / u32::from(height)) as u8,
                    96,
                )
            };
            rgba.extend_from_slice(&[r, g, b, 255]);
        }
    }
    s.console_pixels
        .set(u32::from(width), u32::from(height), rgba.clone());
    s.console_snapshot = Some(std::sync::Arc::new(xenadmin_core::ConsoleSnapshot {
        state: ConsoleState::Connected,
        frame: Some(std::sync::Arc::new(xenadmin_core::ConsoleFrame {
            width,
            height,
            rgba: rgba.into(),
        })),
        cursor: None,
        guest_clipboard: None,
        clipboard_utf8: false,
        extended_keys: true,
    }));
}
/// The same frame at its own size inside the panning view.
pub(super) fn actual_size_scene(s: &mut State) {
    frame_scene(s);
    s.console_ui.actual_size = true;
}

fn next_serial() -> u64 {
    static SERIAL: AtomicU64 = AtomicU64::new(1);
    SERIAL.fetch_add(1, Ordering::Relaxed)
}
pub(super) fn moving_window(s: &mut State) {
    release(s);
    s.console_ui.serial = next_serial();
    export::moving_window(s);
}
pub(super) fn reconnecting(s: &mut State) {
    release(s);
    s.console_ui.serial = next_serial();
    s.console_ui.message = None;
    s.console_ui.export = Default::default();
    s.console_pixels = rune::Pixels::new();
    recovery::opened(s);
}
pub(super) fn release(s: &mut State) {
    if let Some(console) = &s.console {
        let _ = console.input(ConsoleInput::ReleaseAll);
    }
    s.console_buttons = 0;
    s.console_keys.clear();
    s.console_ui.wheel = [0.0; 2];
}
fn send(s: &mut State, input: ConsoleInput) {
    if !recovery::input_ready(s) {
        return;
    }
    if let Some(console) = &s.console
        && let Err(error) = console.input(input)
    {
        s.console_ui.message = Some(error.to_string());
    }
}

pub(super) fn view(s: &State, k: &Kit) -> El {
    let connected = recovery::input_ready(s);
    let host_console = host(s).is_some();
    let status = match s.console_snapshot.as_ref().map(|x| &x.state) {
        Some(ConsoleState::Connected) => "Connected directly to the host console".into(),
        Some(ConsoleState::Failed(error)) => {
            format!("Console disconnected: {error}. Reconnect when the guest or host is available.")
        }
        Some(ConsoleState::Closed) => {
            "Console closed. Reconnect when the guest or host is available.".into()
        }
        _ => "Opening the host console…".into(),
    };
    // Actual size shows the frame at its own dimensions inside a panning view. A canvas that
    // also asked for the full width resolved to zero width there, because the panning view's
    // content has no definite width to take a share of; so each mode sets its own sizing.
    let actual_frame = s
        .console_ui
        .actual_size
        .then(|| s.console_snapshot.as_ref()?.frame.as_ref())
        .flatten()
        .map(|frame| (f32::from(frame.width), f32::from(frame.height)));
    let canvas = el()
        .key("console-canvas")
        .role("application")
        .label("VM console. Click to send keyboard and pointer input")
        .focusable(true)
        .fill(rune::Color::BLACK);
    let canvas = match actual_frame {
        Some((width, height)) => canvas.w(width).h(height).pixels(&s.console_pixels),
        None => canvas
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .pixels_fit(&s.console_pixels, SurfaceFit::Fit),
    };
    let mut canvas = canvas
        .on_layout_tx(|s: &mut State, rect, tx| {
            s.console_bounds = [rect[2], rect[3]];
            tx.live_only();
        })
        .on_key(|s: &mut State, event, tx| key(s, event, tx, true))
        .on_key_up(|s: &mut State, event, tx| key(s, event, tx, false))
        .on_physical_input(|s: &mut State, event, tx| {
            match event {
                PhysicalInput::FocusLost => release(s),
                PhysicalInput::Key {
                    key,
                    pressed,
                    repeat,
                    ..
                } => {
                    if let Some(keycode) = scancode(*key) {
                        let special = match key {
                            PhysicalKeyCode::CapsLock => Some(0xffe5),
                            PhysicalKeyCode::NumLock => Some(0xff7f),
                            PhysicalKeyCode::ScrollLock => Some(0xff14),
                            PhysicalKeyCode::PrintScreen => Some(0xff61),
                            PhysicalKeyCode::Pause => Some(0xff13),
                            PhysicalKeyCode::ContextMenu => Some(0xff67),
                            _ => None,
                        };
                        if special.is_some()
                            || s.console_snapshot.as_ref().is_some_and(|x| x.extended_keys)
                        {
                            let keysym = special.unwrap_or(0);
                            if track_physical(&mut s.console_keys, *key, keysym, *pressed, *repeat)
                            {
                                send(
                                    s,
                                    ConsoleInput::PhysicalKey {
                                        keysym,
                                        keycode,
                                        down: *pressed,
                                    },
                                );
                            }
                            tx.live_only();
                            return true;
                        }
                    }
                }
                _ => {}
            }
            false
        })
        .on_pointer_move_local(|s: &mut State, event, tx| {
            pointer(s, event.x, event.y, s.console_buttons);
            if s.console_snapshot
                .as_ref()
                .is_none_or(|snapshot| snapshot.cursor.is_none())
            {
                tx.live_only();
            }
        })
        .pointer_buttons([PointerButton::Middle])
        .on_drag(|s: &mut State, event, tx| {
            let bit = if tx.button() == PointerButton::Middle {
                2
            } else {
                1
            };
            if matches!(event.phase, Phase::Up) {
                s.console_buttons &= !bit;
            } else {
                s.console_buttons |= bit;
            }
            if event.cancelled() {
                release(s);
            } else {
                pointer(s, event.x, event.y, s.console_buttons);
            }
            tx.live_only();
        })
        .on_secondary_press_local(|s: &mut State, event, _| {
            let held = s.console_buttons;
            pointer(s, event.x, event.y, held | 4);
            pointer(s, event.x, event.y, held);
        })
        .on_wheel(|s: &mut State, event, tx| {
            if s.console_ui.actual_size && event.mods.shift {
                return false;
            }
            for (axis, delta) in [event.dx, event.dy].into_iter().enumerate() {
                if !delta.is_finite() || delta == 0.0 {
                    continue;
                }
                let steps = wheel_steps(&mut s.console_ui.wheel[axis], delta, event.precise);
                let bit = match (axis, steps > 0) {
                    (0, true) => 32,
                    (0, false) => 64,
                    (_, true) => 8,
                    (_, false) => 16,
                };
                for _ in 0..steps.abs() {
                    pointer(s, event.x, event.y, s.console_buttons | bit);
                    pointer(s, event.x, event.y, s.console_buttons);
                }
            }
            tx.live_only();
            true
        });
    if let Some(cursor) = s.console_snapshot.as_ref().and_then(|x| x.cursor.as_ref()) {
        canvas = canvas.cursor(rune::Cursor::Hidden);
        if cursor.width > 0 && cursor.height > 0 {
            let pixels = rune::Pixels::new();
            pixels.set(
                u32::from(cursor.width),
                u32::from(cursor.height),
                cursor.rgba.to_vec(),
            );
            let scale = geometry(s).map_or(1.0, |g| g.scale);
            canvas = canvas.child(
                el().pixels(&pixels)
                    .w(f32::from(cursor.width) * scale)
                    .h(f32::from(cursor.height) * scale)
                    .absolute(
                        s.console_ui.pointer[0] - f32::from(cursor.hotspot[0]) * scale,
                        s.console_ui.pointer[1] - f32::from(cursor.hotspot[1]) * scale,
                    ),
            );
        }
    }
    let canvas = if actual_frame.is_some() {
        el().w_full().grow(1.0).min_h(0.0).scroll_xy().child(canvas)
    } else {
        canvas
    };
    let title = s.console_ui.target().map_or_else(
        || "VM console".into(),
        |target| {
            format!(
                "{} · {}",
                if target.host.is_some() {
                    "Host text console"
                } else {
                    "Console"
                },
                target.name
            )
        },
    );
    let mut result = col()
        .w_full()
        .h_full()
        .min_h(0.0)
        .padding(20.0)
        .gap(10.0)
        .child(
            row()
                .w_full()
                .justify_between()
                .center_y()
                .child(heading(title, k))
                .child(
                    Button::new(if s.console_seat.is_some() {
                        "Close console"
                    } else {
                        "Back to inventory"
                    })
                    .variant(ButtonVariant::Outline)
                    .on_press_tx(|s: &mut State, tx| {
                        if s.console_seat.is_some() {
                            tx.close_window();
                        } else {
                            release(s);
                            s.console = None;
                            s.console_snapshot = None;
                            s.console_ui = Default::default();
                        }
                    })
                    .build(k)
                    .key("close-console"),
                ),
        )
        .child(caption(status, k).wrap())
        .children(
            s.console_ui
                .recovery
                .status()
                .map(|status| caption(status, k).wrap()),
        )
        .child(
            Checkbox::new(s.console_ui.recovery.enabled)
                .label("Reconnect automatically")
                .on_toggle(|s: &mut State| {
                    s.console_ui.recovery.enabled = !s.console_ui.recovery.enabled;
                    release(s);
                })
                .build(k)
                .key("console-auto-reconnect"),
        )
        .child(
            row()
                .gap(8.0)
                .flex_wrap()
                .child(
                    Button::new("Reconnect")
                        .disabled(connected)
                        .on_press(recovery::manual)
                        .build(k)
                        .key("reconnect-console"),
                )
                .child(
                    Button::new(if s.console_ui.actual_size {
                        "Fit to panel"
                    } else {
                        "Actual size"
                    })
                    .on_press(|s: &mut State| {
                        release(s);
                        s.console_ui.actual_size = !s.console_ui.actual_size;
                    })
                    .build(k),
                )
                .child(
                    Button::new("Fullscreen")
                        .on_press_tx(|s: &mut State, tx| {
                            release(s);
                            tx.toggle_full_screen();
                        })
                        .build(k),
                )
                .children(if s.console_seat.is_none() {
                    vec![
                        Button::new("Open in separate window")
                            .disabled(!crate::console_windows::can_detach(s))
                            .on_press(crate::console_windows::detach)
                            .build(k)
                            .key("detach-console"),
                    ]
                } else {
                    vec![]
                })
                .child(Button::new("Release input").on_press(release).build(k))
                .child(
                    Button::new("Save screenshot…")
                        .disabled(
                            s.console_ui.export.pending
                                || s.console_snapshot
                                    .as_ref()
                                    .is_none_or(|snapshot| snapshot.frame.is_none()),
                        )
                        .on_press_tx(export::choose)
                        .build(k)
                        .key("save-console-screenshot"),
                ),
        )
        .child(
            row()
                .gap(8.0)
                .flex_wrap()
                .children(if host_console {
                    vec![
                        chord("Ctrl+C", vec![0xffe3, 0x63], connected, k),
                        chord("Ctrl+D", vec![0xffe3, 0x64], connected, k),
                    ]
                } else {
                    vec![
                        chord(
                            "Ctrl+Alt+Delete",
                            vec![0xffe3, 0xffe9, 0xffff],
                            connected,
                            k,
                        ),
                        chord("Ctrl+Alt+F1", vec![0xffe3, 0xffe9, 0xffbe], connected, k),
                        chord("Alt+Tab", vec![0xffe9, 0xff09], connected, k),
                    ]
                })
                .child(
                    Button::new("Hold right button")
                        .disabled(!connected || s.console_buttons & 4 != 0)
                        .on_press(|s: &mut State| {
                            s.console_buttons |= 4;
                            let [x, y] = s.console_ui.pointer;
                            pointer(s, x, y, s.console_buttons);
                        })
                        .build(k),
                ),
        )
        .child(
            row()
                .gap(8.0)
                .flex_wrap()
                .child(
                    Button::new(if host_console {
                        "Send clipboard to host"
                    } else {
                        "Send clipboard to VM"
                    })
                    .disabled(!connected)
                    .on_press_tx(|s: &mut State, tx| {
                        release(s);
                        let serial = s.console_ui.serial;
                        tx.read_clipboard(move |s: &mut State, clipboard, _| {
                            if s.console.is_none() || s.console_ui.serial != serial {
                                return;
                            }
                            let text = clipboard.text();
                            s.console_ui.message =
                                Some("Clipboard text sent to the console.".into());
                            send(s, ConsoleInput::Clipboard(text.to_owned()));
                        });
                    })
                    .build(k)
                    .key("send-console-clipboard"),
                )
                .child(
                    Button::new(if host_console {
                        "Copy host clipboard"
                    } else {
                        "Copy VM clipboard"
                    })
                    .disabled(
                        !connected
                            || s.console_snapshot
                                .as_ref()
                                .is_none_or(|x| x.guest_clipboard.is_none()),
                    )
                    .on_press_tx(|s: &mut State, tx| {
                        release(s);
                        if let Some(text) = s
                            .console_snapshot
                            .as_ref()
                            .and_then(|x| x.guest_clipboard.as_ref())
                        {
                            tx.copy(text.to_string());
                            s.console_ui.message = Some("Remote clipboard copied locally.".into());
                        }
                    })
                    .build(k)
                    .key("copy-console-clipboard"),
                ),
        );
    if let Some(message) = &s.console_ui.message {
        result = result.child(caption(message, k).wrap());
    }
    result.child(canvas).child(caption("Click the console to type. Focus loss releases input. In Actual size, Shift+wheel pans the view. Clipboard text is limited to 256 KiB; Unicode requires guest support.", k).wrap())
}
fn wheel_steps(remainder: &mut f32, delta: f32, precise: bool) -> i32 {
    if !delta.is_finite() || delta == 0.0 {
        return 0;
    }
    if !precise {
        return (delta.abs() / 40.0).round().clamp(1.0, 8.0) as i32 * delta.signum() as i32;
    }
    *remainder += delta;
    let steps = (*remainder / 40.0).trunc().clamp(-8.0, 8.0) as i32;
    *remainder -= steps as f32 * 40.0;
    steps
}
fn chord(label: &'static str, keys: Vec<u32>, connected: bool, k: &Kit) -> El {
    Button::new(label)
        .disabled(!connected)
        .on_press(move |s: &mut State| {
            release(s);
            send(s, ConsoleInput::KeyChord(keys.clone()));
        })
        .build(k)
}
#[derive(Clone, Copy)]
struct Geometry {
    scale: f32,
    offset: [f32; 2],
    width: u16,
    height: u16,
}
impl Geometry {
    fn new(bounds: [f32; 2], width: u16, height: u16, actual: bool) -> Option<Self> {
        if width == 0 || height == 0 {
            return None;
        }
        let scale = if actual {
            1.0
        } else {
            (bounds[0] / f32::from(width)).min(bounds[1] / f32::from(height))
        };
        if !scale.is_finite() || scale <= 0.0 {
            return None;
        }
        let offset = if actual {
            [0.0; 2]
        } else {
            [
                (bounds[0] - f32::from(width) * scale) * 0.5,
                (bounds[1] - f32::from(height) * scale) * 0.5,
            ]
        };
        Some(Self {
            scale,
            offset,
            width,
            height,
        })
    }
    fn position(self, x: f32, y: f32) -> Option<[u16; 2]> {
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        Some([
            ((x - self.offset[0]) / self.scale).clamp(0.0, f32::from(self.width - 1)) as u16,
            ((y - self.offset[1]) / self.scale).clamp(0.0, f32::from(self.height - 1)) as u16,
        ])
    }
}
fn geometry(s: &State) -> Option<Geometry> {
    let frame = s.console_snapshot.as_ref()?.frame.as_ref()?;
    Geometry::new(
        s.console_bounds,
        frame.width,
        frame.height,
        s.console_ui.actual_size,
    )
}
fn pointer(s: &mut State, x: f32, y: f32, buttons: u8) {
    s.console_ui.pointer = [x, y];
    let Some([x, y]) = geometry(s).and_then(|g| g.position(x, y)) else {
        return;
    };
    send(s, ConsoleInput::Pointer { x, y, buttons });
}
fn key(s: &mut State, event: &KeyEvent, tx: &mut Tx, down: bool) -> bool {
    if s.console.is_none() {
        return false;
    }
    let down = if let Key::Modifier { pressed, .. } = event.key {
        pressed
    } else {
        down
    };
    let mut symbol = keysym(&event.key);
    let physical = tx.physical_key();
    if let Some(physical) = physical {
        if down {
            if let Some((_, existing)) = s.console_keys.iter().find(|(key, _)| *key == physical) {
                symbol = Some(*existing);
            } else if let Some(symbol) = symbol {
                s.console_keys.push((physical, symbol));
            }
        } else if let Some(index) = s.console_keys.iter().position(|(key, _)| *key == physical) {
            symbol = Some(s.console_keys.remove(index).1);
        }
    }
    if let Some(keysym) = symbol {
        let input =
            physical
                .and_then(scancode)
                .map_or(ConsoleInput::Key { keysym, down }, |keycode| {
                    ConsoleInput::PhysicalKey {
                        keysym,
                        keycode,
                        down,
                    }
                });
        send(s, input);
        tx.live_only();
        true
    } else {
        false
    }
}
/// Records which physical keys this console holds down in the guest and says whether an
/// event should be forwarded. A repeat of a key that is not held here is dropped: once a
/// focus loss has released every key, a repeat already in flight would press it again.
fn track_physical(
    held: &mut Vec<(PhysicalKeyCode, u32)>,
    key: PhysicalKeyCode,
    keysym: u32,
    pressed: bool,
    repeat: bool,
) -> bool {
    let position = held.iter().position(|(existing, _)| *existing == key);
    match (pressed, position) {
        (true, None) if repeat => false,
        (true, None) => {
            held.push((key, keysym));
            true
        }
        (true, Some(_)) => true,
        (false, Some(index)) => {
            held.remove(index);
            true
        }
        // A release for a key this console does not hold is harmless to forward.
        (false, None) => true,
    }
}
fn scancode(key: PhysicalKeyCode) -> Option<u32> {
    use PhysicalKeyCode::*;
    Some(match key {
        Escape => 1,
        Digit1 => 2,
        Digit2 => 3,
        Digit3 => 4,
        Digit4 => 5,
        Digit5 => 6,
        Digit6 => 7,
        Digit7 => 8,
        Digit8 => 9,
        Digit9 => 10,
        Digit0 => 11,
        Minus => 12,
        Equal => 13,
        Backspace => 14,
        Tab => 15,
        KeyQ => 16,
        KeyW => 17,
        KeyE => 18,
        KeyR => 19,
        KeyT => 20,
        KeyY => 21,
        KeyU => 22,
        KeyI => 23,
        KeyO => 24,
        KeyP => 25,
        BracketLeft => 26,
        BracketRight => 27,
        Enter => 28,
        ControlLeft => 29,
        KeyA => 30,
        KeyS => 31,
        KeyD => 32,
        KeyF => 33,
        KeyG => 34,
        KeyH => 35,
        KeyJ => 36,
        KeyK => 37,
        KeyL => 38,
        Semicolon => 39,
        Quote => 40,
        Backquote => 41,
        ShiftLeft => 42,
        Backslash => 43,
        KeyZ => 44,
        KeyX => 45,
        KeyC => 46,
        KeyV => 47,
        KeyB => 48,
        KeyN => 49,
        KeyM => 50,
        Comma => 51,
        Period => 52,
        Slash => 53,
        ShiftRight => 54,
        NumpadMultiply => 55,
        AltLeft => 56,
        Space => 57,
        CapsLock => 58,
        F1 => 59,
        F2 => 60,
        F3 => 61,
        F4 => 62,
        F5 => 63,
        F6 => 64,
        F7 => 65,
        F8 => 66,
        F9 => 67,
        F10 => 68,
        NumLock => 69,
        ScrollLock => 70,
        Numpad7 => 71,
        Numpad8 => 72,
        Numpad9 => 73,
        NumpadSubtract => 74,
        Numpad4 => 75,
        Numpad5 => 76,
        Numpad6 => 77,
        NumpadAdd => 78,
        Numpad1 => 79,
        Numpad2 => 80,
        Numpad3 => 81,
        Numpad0 => 82,
        NumpadDecimal => 83,
        PrintScreen => 84,
        IntlBackslash => 86,
        F11 => 87,
        F12 => 88,
        NumpadEnter => 0x9c,
        ControlRight => 0x9d,
        NumpadDivide => 0xb5,
        AltRight => 0xb8,
        Home => 0xc7,
        ArrowUp => 0xc8,
        PageUp => 0xc9,
        ArrowLeft => 0xcb,
        ArrowRight => 0xcd,
        End => 0xcf,
        ArrowDown => 0xd0,
        PageDown => 0xd1,
        Insert => 0xd2,
        Delete => 0xd3,
        SuperLeft => 0xdb,
        SuperRight => 0xdc,
        ContextMenu => 0xdd,
        Pause => 0xc6,
        _ => return None,
    })
}
fn keysym(key: &Key) -> Option<u32> {
    Some(match key {
        Key::Text(value) => {
            let c = value.chars().next()? as u32;
            if c <= 0xff { c } else { 0x0100_0000 | c }
        }
        Key::Command(c) => *c as u32,
        Key::Enter => 0xff0d,
        Key::Backspace => 0xff08,
        Key::Tab => 0xff09,
        Key::Escape => 0xff1b,
        Key::Delete => 0xffff,
        Key::Insert => 0xff63,
        Key::Left => 0xff51,
        Key::Up => 0xff52,
        Key::Right => 0xff53,
        Key::Down => 0xff54,
        Key::Home => 0xff50,
        Key::End => 0xff57,
        Key::PageUp => 0xff55,
        Key::PageDown => 0xff56,
        Key::Function(n @ 1..=24) => 0xffbd + u32::from(*n),
        Key::Modifier { modifier, .. } => match modifier {
            Modifier::ShiftLeft => 0xffe1,
            Modifier::ShiftRight => 0xffe2,
            Modifier::ControlLeft => 0xffe3,
            Modifier::ControlRight => 0xffe4,
            Modifier::AltLeft => 0xffe9,
            Modifier::AltRight => 0xffea,
            Modifier::SuperLeft => 0xffeb,
            Modifier::SuperRight => 0xffec,
            _ => return None,
        },
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn console_defaults_apply_only_when_opening_a_new_target() {
        let mut s = crate::init();
        s.preferences.options.console_actual_size = true;
        s.preferences.options.console_reconnect = false;
        recovery_scene(&mut s);
        assert!(s.console_ui.actual_size);
        assert!(!s.console_ui.recovery.enabled);
        s.preferences.options.console_actual_size = false;
        s.preferences.options.console_reconnect = true;
        assert!(s.console_ui.actual_size);
        assert!(!s.console_ui.recovery.enabled);
        recovery_scene(&mut s);
        assert!(!s.console_ui.actual_size);
        assert!(s.console_ui.recovery.enabled);
    }
    #[test]
    fn a_repeat_after_release_does_not_press_the_key_again() {
        use PhysicalKeyCode::{KeyA, KeyB};
        let mut held = Vec::new();
        assert!(track_physical(&mut held, KeyA, 0, true, false), "press");
        assert!(
            track_physical(&mut held, KeyA, 0, true, true),
            "repeat while held"
        );
        // Focus loss releases everything; the live desktop then saw one more press arrive.
        held.clear();
        assert!(
            !track_physical(&mut held, KeyA, 0, true, true),
            "a repeat of a released key is dropped"
        );
        assert!(held.is_empty());
        assert!(
            track_physical(&mut held, KeyA, 0, false, false),
            "stray release"
        );
        assert!(track_physical(&mut held, KeyB, 0, true, false));
        assert!(track_physical(&mut held, KeyB, 0, false, false));
        assert!(held.is_empty());
    }
    #[test]
    fn the_console_canvas_is_laid_out_in_fit_and_actual_size_modes() {
        let rect = |scene: fn(&mut State)| {
            let mut state = crate::init();
            scene(&mut state);
            let mut runtime = rune::Runtime::new(
                state,
                crate::view,
                (1440.0, 900.0),
                rune::Env {
                    system_dark: true,
                    scale: 1.0,
                },
            );
            for _ in 0..30 {
                runtime.frame(1.0 / 60.0);
            }
            let canvas = runtime
                .elements()
                .into_iter()
                .find(|e| e.key.as_deref() == Some("console-canvas"))
                .expect("the console canvas is part of the view");
            (canvas.rect, runtime.state.console_bounds)
        };
        let (fit, fit_bounds) = rect(frame_scene);
        assert!(
            fit[2] > 400.0 && fit[3] > 300.0,
            "fit fills the panel: {fit:?}"
        );
        assert_eq!(fit_bounds, [fit[2], fit[3]]);
        // The live desktop showed an empty panel here: the canvas had resolved to zero width.
        let (actual, actual_bounds) = rect(actual_size_scene);
        assert_eq!(
            [actual[2], actual[3]],
            [800.0, 600.0],
            "actual size shows the frame at its own dimensions"
        );
        assert_eq!(actual_bounds, [800.0, 600.0]);
        assert_eq!(
            [actual[0], actual[1]],
            [fit[0], fit[1]],
            "the frame starts at the view's origin"
        );
    }
    #[test]
    fn pointer_matches_letterboxing_actual_size_and_resize() {
        let fit = Geometry::new([1000.0, 1000.0], 800, 600, false).unwrap();
        assert_eq!(fit.position(500.0, 500.0), Some([400, 300]));
        assert_eq!(fit.position(-10.0, 1100.0), Some([0, 599]));
        let actual = Geometry::new([1000.0, 1000.0], 800, 600, true).unwrap();
        assert_eq!(actual.position(400.0, 300.0), Some([400, 300]));
        assert!(fit.position(f32::NAN, 0.0).is_none());
        assert!(Geometry::new([0.0, 0.0], 800, 600, false).is_none());
        assert_eq!(
            Geometry::new([1000.0, 1000.0], 400, 300, false)
                .unwrap()
                .position(500.0, 500.0),
            Some([200, 150])
        );
    }
    #[test]
    fn wheel_notches_and_precise_motion_reach_guest_without_losing_small_notches() {
        let mut remainder = 0.0;
        assert_eq!(wheel_steps(&mut remainder, 10.0, false), 1);
        assert_eq!(wheel_steps(&mut remainder, -30.0, false), -1);
        assert_eq!(wheel_steps(&mut remainder, 15.0, true), 0);
        assert_eq!(wheel_steps(&mut remainder, 30.0, true), 1);
        assert_eq!(remainder, 5.0);
        assert_eq!(wheel_steps(&mut remainder, f32::NAN, false), 0);
    }
    #[test]
    fn extended_scancodes_follow_qemu_make_code_encoding() {
        assert_eq!(scancode(PhysicalKeyCode::KeyA), Some(0x1e));
        assert_eq!(scancode(PhysicalKeyCode::ArrowRight), Some(0xcd));
        assert_eq!(scancode(PhysicalKeyCode::ControlRight), Some(0x9d));
        assert_eq!(scancode(PhysicalKeyCode::PrintScreen), Some(0x54));
        assert_eq!(scancode(PhysicalKeyCode::Pause), Some(0xc6));
    }
}
