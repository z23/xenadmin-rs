//! Frames, pointer, cursor, fullscreen, and clipboard for spike S1.
//!
//! This is not the released binary and it does not open a console to a host.

use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::AsFd;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use gpui_kit::prelude::*;
use gpui_kit::{
    Bounds, ClipboardItem, Context, Corners, Entity, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Render, RenderImage, ScrollEvent, TitlebarOptions, Window, WindowBounds,
    WindowOptions, canvas, div, point, px, size,
};
use image::{Frame, Rgba, RgbaImage};
use wayland_client::protocol::{
    wl_buffer, wl_output, wl_pointer, wl_registry, wl_shm, wl_shm_pool,
};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols_wlr::screencopy::v1::client::{
    zwlr_screencopy_frame_v1::{self, ZwlrScreencopyFrameV1},
    zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
    zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};
use xenadmin_core::{ConsoleInput, TypedText};

const TITLE: &str = "xenadmin-s1-surface";

const FIT: u8 = 1;
const REPLACED: u8 = 2;
const ACTUAL: u8 = 3;
const POINTER: u8 = 4;
const CURSOR: u8 = 5;
const CLIPBOARD: u8 = 6;
const FULLSCREEN: u8 = 7;
const WINDOWS: u8 = 8;
const MEMORY: u8 = 9;
const QUIT: u8 = 10;

struct Shared {
    lines: Mutex<Vec<String>>,
    phase: AtomicU8,
    pan: Mutex<(f32, f32)>,
    pointer: Mutex<Vec<String>>,
    clipboard_done: AtomicBool,
    clipboard_logged: AtomicBool,
    canvas_logged: AtomicBool,
    close_extras: AtomicBool,
    opened_once: AtomicBool,
    mouse_logs: AtomicU8,
    fullscreen_seen: AtomicU8,
    fullscreen_want: AtomicU8,
    fullscreen_toggles: AtomicU8,
    extra_windows: Mutex<Vec<gpui_kit::AnyWindowHandle>>,
}

struct Probe {
    shared: Arc<Shared>,
    image: Option<Arc<RenderImage>>,
    retired: Option<Arc<RenderImage>>,
    memory_started: Option<Instant>,
    memory_samples: Vec<u64>,
    showed_replaced: bool,
    showed_actual: bool,
}

impl Probe {
    fn paint_image(&mut self, image: RenderImage) {
        if let Some(current) = self.image.take() {
            self.retired = Some(current);
        }
        self.image = Some(Arc::new(image));
    }
}

impl Render for Probe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let shared = Arc::clone(&self.shared);
        let image = self.image.clone();
        let retired = self.retired.take();
        let phase = shared.phase.load(Ordering::SeqCst);
        let moves = Arc::clone(&shared);
        let downs = Arc::clone(&shared);
        let ups = Arc::clone(&shared);
        let wheels = Arc::clone(&shared);
        div()
            .size_full()
            .on_mouse_move(move |event, _, _| {
                moves.pointer(format!(
                    "div-move x={:.1} y={:.1}",
                    f32::from(event.position.x),
                    f32::from(event.position.y)
                ));
            })
            .capture_any_mouse_down(move |event, _, _| {
                downs.pointer(format!("button down {}", button_name(event.button)));
            })
            .capture_any_mouse_up(move |event, _, _| {
                ups.pointer(format!("button up {}", button_name(event.button)));
            })
            .on_scroll(move |event, _, _| {
                if event.modifiers.shift {
                    let dy = match event.delta {
                        gpui_kit::ScrollDelta::Pixels(delta) => f32::from(delta.y),
                        gpui_kit::ScrollDelta::Lines(delta) => delta.y * 16.0,
                    };
                    wheels.pan.lock().expect("pan").1 += dy;
                }
                wheels.pointer(format!(
                    "wheel shift={} delta={:?}",
                    event.modifiers.shift, event.delta
                ));
            })
            .child(
                canvas(
                    |bounds, _, _| bounds,
                    move |bounds, canvas_bounds, window, _cx| {
                        if let Some(retired) = retired.clone() {
                            let _ = window.drop_image(retired);
                        }
                        let actual = matches!(phase, ACTUAL | POINTER | CURSOR | CLIPBOARD);
                        if phase == POINTER && !shared.canvas_logged.swap(true, Ordering::SeqCst) {
                            shared.push(format!(
                                "canvas-origin {:.1},{:.1} size {:.1}x{:.1}",
                                f32::from(canvas_bounds.origin.x),
                                f32::from(canvas_bounds.origin.y),
                                f32::from(canvas_bounds.size.width),
                                f32::from(canvas_bounds.size.height)
                            ));
                        }
                        if let Some(image) = image.clone() {
                            let pixels = image.size(0);
                            let (pan_x, pan_y) = *shared.pan.lock().expect("pan");
                            let image_bounds = if actual {
                                Bounds {
                                    origin: point(
                                        canvas_bounds.origin.x + px(pan_x),
                                        canvas_bounds.origin.y + px(pan_y),
                                    ),
                                    size: size(
                                        px(pixels.width.0 as f32),
                                        px(pixels.height.0 as f32),
                                    ),
                                }
                            } else {
                                canvas_bounds
                            };
                            let _ = window.paint_image(
                                bounds,
                                image_bounds,
                                Corners::default(),
                                image,
                                0,
                                false,
                            );
                        }
                        let moves = Arc::clone(&shared);
                        window.on_mouse_event(move |event: &MouseMoveEvent, _, _, _| {
                            moves.pointer(format!(
                                "move x={:.1} y={:.1}",
                                f32::from(event.position.x),
                                f32::from(event.position.y)
                            ));
                        });
                        record_button(&shared, window, "down");
                        record_up(&shared, window);
                        let wheels = Arc::clone(&shared);
                        window.on_mouse_event(move |event: &ScrollEvent, _, _, _| {
                            if event.modifiers.shift {
                                let dy = match event.delta {
                                    gpui_kit::ScrollDelta::Pixels(delta) => f32::from(delta.y),
                                    gpui_kit::ScrollDelta::Lines(delta) => delta.y * 16.0,
                                };
                                wheels.pan.lock().expect("pan").1 += dy;
                            }
                            wheels.pointer(format!(
                                "wheel shift={} delta={:?}",
                                event.modifiers.shift, event.delta
                            ));
                        });
                    },
                )
                .size_full(),
            )
    }
}

fn record_button(shared: &Arc<Shared>, window: &mut Window, kind: &'static str) {
    let shared = Arc::clone(shared);
    window.on_mouse_event(move |event: &MouseDownEvent, _, _, _| {
        shared.pointer(format!("button {kind} {}", button_name(event.button)));
    });
}

fn record_up(shared: &Arc<Shared>, window: &mut Window) {
    let shared = Arc::clone(shared);
    window.on_mouse_event(move |event: &MouseUpEvent, _, _, _| {
        shared.pointer(format!("button up {}", button_name(event.button)));
    });
}

fn button_name(button: MouseButton) -> &'static str {
    match button {
        MouseButton::Left => "left",
        MouseButton::Right => "right",
        MouseButton::Middle => "middle",
        MouseButton::Navigate(_) => "navigate",
    }
}

fn bgra(width: u32, height: u32, paint: impl Fn(u32, u32) -> [u8; 4]) -> Frame {
    let mut bytes = vec![0u8; width as usize * height as usize * 4];
    let (pixels, _) = bytes.as_chunks_mut::<4>();
    for (index, pixel) in pixels.iter_mut().enumerate() {
        let x = index as u32 % width;
        let y = index as u32 / width;
        *pixel = paint(x, y);
    }
    Frame::new(RgbaImage::from_raw(width, height, bytes).expect("frame size"))
}

fn split_frame() -> Frame {
    bgra(160, 120, |x, y| {
        let mut pixel = if x < 80 {
            [0, 0, 255, 255]
        } else {
            [255, 0, 0, 255]
        };
        if (6..11).contains(&x) && (6..11).contains(&y) {
            pixel = if (x, y) == (8, 8) {
                [255, 255, 255, 255]
            } else {
                [0, 255, 255, 255]
            };
        }
        pixel
    })
}

fn solid(b: u8, g: u8, r: u8) -> Frame {
    bgra(160, 120, |_, _| [b, g, r, 255])
}

fn memory_frame(tick: u8) -> Frame {
    bgra(1920, 1080, |_, _| [tick, tick, tick, 255])
}

impl Shared {
    fn push(&self, line: String) {
        println!("{line}");
        let _ = std::io::Write::flush(&mut std::io::stdout());
        self.lines.lock().expect("log").push(line);
    }

    fn pointer(&self, line: String) {
        println!("pointer {line}");
        let _ = std::io::Write::flush(&mut std::io::stdout());
        self.pointer.lock().expect("pointer").push(line);
    }
}

fn main() {
    if std::env::var("WAYLAND_DISPLAY")
        .unwrap_or_default()
        .is_empty()
    {
        println!("s1_surface_result=fail");
        println!("WAYLAND_DISPLAY is unset");
        std::process::exit(2);
    }
    let saved_cursor = cursor_pos();
    if std::env::args().any(|arg| arg == "--grab-test") {
        match capture_region("DP-2", 48, 48, 64, 64, false) {
            Ok(image) => println!("grab-test {}", describe_grab(&image)),
            Err(error) => println!("grab-test {error}"),
        }
        return;
    }
    let saved_clipboard = saved_clipboard();
    let shared = Arc::new(Shared {
        lines: Mutex::new(Vec::new()),
        phase: AtomicU8::new(0),
        pan: Mutex::new((0.0, 0.0)),
        pointer: Mutex::new(Vec::new()),
        clipboard_done: AtomicBool::new(false),
        clipboard_logged: AtomicBool::new(false),
        canvas_logged: AtomicBool::new(false),
        close_extras: AtomicBool::new(false),
        opened_once: AtomicBool::new(false),
        mouse_logs: AtomicU8::new(0),
        fullscreen_seen: AtomicU8::new(9),
        fullscreen_want: AtomicU8::new(0),
        fullscreen_toggles: AtomicU8::new(0),
        extra_windows: Mutex::new(Vec::new()),
    });
    let driver = {
        let shared = Arc::clone(&shared);
        thread::spawn(move || drive(shared))
    };
    let shared_run = Arc::clone(&shared);
    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);
        cx.set_cursor_hide_mode(gpui_kit::CursorHideMode::OnTyping);
        let shared_window = Arc::clone(&shared_run);
        let shared_pump = Arc::clone(&shared_run);
        gpui_kit::open_window(options(TITLE), cx, |window, cx| {
            let probe = cx.new(|cx| {
                let mut probe = Probe {
                    shared: Arc::clone(&shared_window),
                    image: None,
                    retired: None,
                    memory_started: None,
                    memory_samples: Vec::new(),
                    showed_replaced: false,
                    showed_actual: false,
                };
                probe.paint_image(RenderImage::new([split_frame()]));
                cx.notify();
                probe
            });
            pump(window, cx, probe.clone(), shared_pump);
            probe
        })
        .expect("surface window opens");
    });
    let _ = driver.join();
    restore_clipboard(&saved_clipboard);
    if let Some((x, y)) = saved_cursor {
        let _ = Command::new("hyprctl")
            .args([
                "dispatch",
                &format!("hl.dsp.cursor.move({{ x = {x}, y = {y} }})"),
            ])
            .status();
    }
    let pass = shared
        .lines
        .lock()
        .expect("log")
        .iter()
        .any(|line| line == "s1_surface_result=pass");
    std::process::exit(if pass { 0 } else { 1 });
}

fn pump(window: &mut Window, cx: &mut gpui_kit::App, probe: Entity<Probe>, shared: Arc<Shared>) {
    window
        .spawn(cx, async move |cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(33))
                    .await;
                let action = cx
                    .update(|window, app| {
                        step_window(window, app, &probe, &shared);
                        shared.phase.load(Ordering::SeqCst) != QUIT
                    })
                    .unwrap_or(false);
                if !action {
                    cx.update(|_window, app| app.quit()).ok();
                    break;
                }
            }
        })
        .detach();
}

fn step_window(
    window: &mut Window,
    app: &mut gpui_kit::App,
    probe: &Entity<Probe>,
    shared: &Shared,
) {
    let phase = shared.phase.load(Ordering::SeqCst);
    if phase == FULLSCREEN {
        let want = shared.fullscreen_want.load(Ordering::SeqCst);
        let applied = shared.fullscreen_toggles.load(Ordering::SeqCst);
        if want != 0 && applied != want {
            shared.push(format!("fullscreen-gpui-before {}", window.is_fullscreen()));
            window.toggle_fullscreen();
            shared.fullscreen_toggles.store(want, Ordering::SeqCst);
            shared.push(format!(
                "fullscreen-toggle {want} gpui-now {}",
                window.is_fullscreen()
            ));
        }
        let now = u8::from(window.is_fullscreen());
        if shared.fullscreen_seen.swap(now, Ordering::SeqCst) != now {
            shared.push(format!("fullscreen-gpui {now}"));
        }
    }
    if phase == WINDOWS
        && !shared.opened_once.swap(true, Ordering::SeqCst)
        && shared.extra_windows.lock().expect("windows").is_empty()
    {
        let mut handles = Vec::new();
        for index in 2..=8 {
            if let Ok((handle, _view)) = gpui_kit::open_window(
                options(&format!("xenadmin-s1-w{index}")),
                app,
                |window, cx| {
                    window
                        .spawn(cx, async move |cx| {
                            loop {
                                cx.background_executor()
                                    .timer(Duration::from_millis(100))
                                    .await;
                                if cx
                                    .update(|window, _app| {
                                        window.refresh();
                                    })
                                    .is_err()
                                {
                                    break;
                                }
                            }
                        })
                        .detach();
                    cx.new(|_cx| Extra)
                },
            ) {
                handles.push(handle);
            }
        }
        shared.push(format!("opened-extra={}", handles.len()));
        *shared.extra_windows.lock().expect("windows") = handles;
    }
    if shared.close_extras.swap(false, Ordering::SeqCst) {
        let extras = std::mem::take(&mut *shared.extra_windows.lock().expect("windows"));
        for handle in extras {
            let _ = app.update_window(handle, |_view, window, _cx| window.remove_window());
        }
    }
    if phase == MEMORY {
        probe.update(app, |probe, cx| {
            if probe.memory_started.is_none() {
                probe.memory_started = Some(Instant::now());
                probe.memory_samples.push(rss_kb());
            }
            let elapsed = probe.memory_started.unwrap().elapsed();
            let tick = (elapsed.as_millis() / 33) as u8;
            probe.paint_image(RenderImage::new([memory_frame(tick)]));
            if elapsed >= Duration::from_secs(30) && probe.memory_samples.len() == 1 {
                probe.memory_samples.push(rss_kb());
            }
            if elapsed >= Duration::from_secs(60) && probe.memory_samples.len() == 2 {
                probe.memory_samples.push(rss_kb());
                shared.push(format!("memory-rss-kb={:?}", probe.memory_samples));
                shared.phase.store(QUIT, Ordering::SeqCst);
            }
            cx.notify();
        });
    } else {
        probe.update(app, |probe, cx| {
            if phase == REPLACED && !probe.showed_replaced {
                probe.paint_image(RenderImage::new([solid(0, 255, 0)]));
                probe.showed_replaced = true;
            }
            if phase == ACTUAL && !probe.showed_actual {
                probe.paint_image(RenderImage::new([split_frame()]));
                probe.showed_actual = true;
            }
            if phase == CLIPBOARD && !shared.clipboard_done.swap(true, Ordering::SeqCst) {
                let text = format!("xenadmin-s1-{}", std::process::id());
                cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                let read = cx
                    .read_from_clipboard()
                    .and_then(|item| item.text())
                    .unwrap_or_default();
                let review = TypedText::review(&text);
                let input = ConsoleInput::TypeText(text);
                shared.push(format!(
                    "clipboard gpui_read={read:?} review={review:?} input={input:?}"
                ));
            } else if phase == CLIPBOARD
                && !shared.clipboard_logged.load(Ordering::SeqCst)
                && let Some(text) = cx.read_from_clipboard().and_then(|item| item.text())
                && text.starts_with("xenadmin-s1-")
            {
                shared.clipboard_logged.store(true, Ordering::SeqCst);
                shared.push(format!("clipboard gpui_later={text}"));
            }
            cx.notify();
        });
    }
    if phase == POINTER {
        let sample = shared.mouse_logs.fetch_add(1, Ordering::SeqCst);
        if sample < 80 && sample.is_multiple_of(3) {
            let pos = window.mouse_position();
            shared.push(format!(
                "gpui-mouse {:.1},{:.1}",
                f32::from(pos.x),
                f32::from(pos.y)
            ));
        }
    }
    window.refresh();
}

struct Extra;

impl Render for Extra {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().child("extra")
    }
}

fn options(title: &str) -> WindowOptions {
    WindowOptions {
        titlebar: Some(TitlebarOptions {
            title: Some(title.to_string().into()),
            ..Default::default()
        }),
        app_id: Some("xenadmin-s1".to_string()),
        focus: title == TITLE,
        show: true,
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: point(px(64.), px(64.)),
            size: size(px(480.), px(320.)),
        })),
        ..Default::default()
    }
}

fn drive(shared: Arc<Shared>) {
    let Some(window) = wait_window(TITLE) else {
        shared.push("window-missing".to_string());
        shared.phase.store(QUIT, Ordering::SeqCst);
        finish(&shared, false);
        return;
    };
    log_hypr(
        &shared,
        "fullscreen-clear",
        &format!(
            "hl.dsp.window.fullscreen({{ mode = 'fullscreen', action = 'unset', window = 'address:{}' }})",
            window.address
        ),
    );
    thread::sleep(Duration::from_millis(200));
    log_hypr(
        &shared,
        "float",
        &format!(
            "hl.dsp.window.float({{ action = \"set\", window = \"address:{}\" }})",
            window.address
        ),
    );
    let (origin_x, origin_y) = monitor_origin();
    log_hypr(
        &shared,
        "move",
        &format!(
            "hl.dsp.window.move({{ x = {}, y = {}, relative = false, window = \"address:{}\" }})",
            origin_x + 48,
            origin_y + 48,
            window.address
        ),
    );
    log_hypr(
        &shared,
        "resize",
        &format!(
            "hl.dsp.window.resize({{ x = 480, y = 320, relative = false, window = \"address:{}\" }})",
            window.address
        ),
    );
    log_hypr(
        &shared,
        "raise",
        &format!(
            "hl.dsp.window.bring_to_top({{ window = \"address:{}\" }})",
            window.address
        ),
    );
    thread::sleep(Duration::from_millis(250));
    let Some(window) = find_window(TITLE) else {
        finish(&shared, false);
        return;
    };
    shared.push(format!(
        "window at={} size={} scale={} output={} rel={},{}",
        window.at, window.size, window.scale, window.output, window.rel_x, window.rel_y
    ));
    shared.push(format!("window-facts {}", window_facts(&window.address)));

    run_checks(&shared, &window);
}

fn run_checks(shared: &Arc<Shared>, window: &SeatWindow) {
    shared.phase.store(FIT, Ordering::SeqCst);
    thread::sleep(Duration::from_millis(400));
    shared.push(format!("fit-sample {}", grab(window)));

    shared.phase.store(REPLACED, Ordering::SeqCst);
    thread::sleep(Duration::from_millis(400));
    let replaced_window = find_window(TITLE).unwrap_or_else(|| window.clone());
    shared.push(format!("replaced-sample {}", grab(&replaced_window)));

    shared.phase.store(ACTUAL, Ordering::SeqCst);
    thread::sleep(Duration::from_millis(400));
    let actual_window = find_window(TITLE).unwrap_or_else(|| window.clone());
    shared.push(format!("actual-sample {}", grab(&actual_window)));

    shared.phase.store(POINTER, Ordering::SeqCst);
    thread::sleep(Duration::from_millis(200));
    let pointer_window = find_window(TITLE).unwrap_or_else(|| actual_window.clone());
    focus_window(shared, &pointer_window.address);
    exercise_pointer(shared, &pointer_window);

    focus_window(shared, &pointer_window.address);
    shared.phase.store(CLIPBOARD, Ordering::SeqCst);
    thread::sleep(Duration::from_millis(500));
    shared.push(format!("clipboard wl-paste={}", clipboard_text()));

    shared.phase.store(CURSOR, Ordering::SeqCst);
    thread::sleep(Duration::from_millis(200));
    let cursor_window = find_window(TITLE).unwrap_or(pointer_window);
    focus_window(shared, &cursor_window.address);
    exercise_cursor(shared, &cursor_window);

    shared.phase.store(FULLSCREEN, Ordering::SeqCst);
    let initial = fullscreen_flag(TITLE);
    shared.push(format!("fullscreen-initial {initial}"));
    let mut step = 0u8;
    if initial != "0" && initial != "missing" && initial != "-1" {
        step = 1;
        shared.fullscreen_want.store(step, Ordering::SeqCst);
        shared.push(format!("fullscreen-off {}", poll_fullscreen(false)));
    }
    step += 1;
    shared.fullscreen_want.store(step, Ordering::SeqCst);
    let entered = poll_fullscreen(true);
    shared.push(format!("fullscreen-on {entered}"));
    let saw_flag = wait_for_line(shared, "fullscreen-gpui 1", Duration::from_secs(1));
    shared.push(format!("fullscreen-flag-seen {saw_flag}"));
    step += 1;
    shared.fullscreen_want.store(step, Ordering::SeqCst);
    shared.push(format!("fullscreen-off {}", poll_fullscreen(false)));
    shared.phase.store(ACTUAL, Ordering::SeqCst);

    shared.phase.store(WINDOWS, Ordering::SeqCst);
    let opened = Instant::now();
    while opened.elapsed() < Duration::from_secs(3) {
        let ready = shared
            .lines
            .lock()
            .expect("log")
            .iter()
            .any(|line| line.starts_with("opened-extra="));
        if ready {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    thread::sleep(Duration::from_millis(500));
    let count = count_titles();
    shared.push(format!("window-count={count}"));
    shared.push(format!("windows {}", our_client_summary()));
    shared.close_extras.store(true, Ordering::SeqCst);
    thread::sleep(Duration::from_millis(400));
    shared.phase.store(ACTUAL, Ordering::SeqCst);

    if std::env::var_os("S1_SKIP_MEMORY").is_some() {
        shared.push("memory skipped".to_string());
        let pass = evaluate(shared);
        finish(shared, pass);
        return;
    }
    shared.phase.store(MEMORY, Ordering::SeqCst);
    let memory_wait = Instant::now();
    while memory_wait.elapsed() < Duration::from_secs(70)
        && !shared
            .lines
            .lock()
            .expect("log")
            .iter()
            .any(|line| line.starts_with("memory-rss-kb="))
    {
        thread::sleep(Duration::from_millis(200));
    }
    let pass = evaluate(shared);
    finish(shared, pass);
}

fn finish(shared: &Shared, pass: bool) {
    shared.push(format!(
        "s1_surface_result={}",
        if pass { "pass" } else { "fail" }
    ));
    shared.phase.store(QUIT, Ordering::SeqCst);
}

#[derive(Clone)]
struct SeatWindow {
    address: String,
    at: String,
    size: String,
    scale: f64,
    output: String,
    x: i32,
    y: i32,
    rel_x: i32,
    rel_y: i32,
    w: i32,
    h: i32,
}

fn wait_window(title: &str) -> Option<SeatWindow> {
    for _ in 0..50 {
        if let Some(window) = find_window(title) {
            return Some(window);
        }
        thread::sleep(Duration::from_millis(100));
    }
    None
}

fn find_window(title: &str) -> Option<SeatWindow> {
    let output = Command::new("hyprctl")
        .args(["clients", "-j"])
        .output()
        .ok()?;
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    let pid = u64::from(std::process::id());
    for client in value.as_array()? {
        if client.get("pid").and_then(|pid| pid.as_u64()) != Some(pid) {
            continue;
        }
        let name = client
            .get("title")
            .and_then(|title| title.as_str())
            .unwrap_or("");
        if name != title {
            continue;
        }
        let at = client.get("at").and_then(|at| at.as_array())?;
        let size = client.get("size").and_then(|size| size.as_array())?;
        let x = at.first().and_then(|value| value.as_i64())? as i32;
        let y = at.get(1).and_then(|value| value.as_i64())? as i32;
        let (output_name, output_x, output_y, scale) = monitor_at(x, y);
        return Some(SeatWindow {
            address: client.get("address")?.as_str()?.to_string(),
            at: at
                .iter()
                .filter_map(|value| value.as_i64())
                .map(|value| value.to_string())
                .collect::<Vec<_>>()
                .join(","),
            size: size
                .iter()
                .filter_map(|value| value.as_i64())
                .map(|value| value.to_string())
                .collect::<Vec<_>>()
                .join(","),
            scale,
            output: output_name,
            x,
            y,
            rel_x: x - output_x,
            rel_y: y - output_y,
            w: size.first().and_then(|value| value.as_i64())? as i32,
            h: size.get(1).and_then(|value| value.as_i64())? as i32,
        });
    }
    None
}

fn grab(_window: &SeatWindow) -> String {
    // `--grab-test` showed zwlr_screencopy delivering a buffer and then never
    // a ready event. grim's ext-image-copy capture hangs the same way. A
    // second capture client would sit on that protocol for the rest of the run.
    "screencopy-timeout: buffer arrived, ready never did; grim hangs the same way".to_string()
}

fn describe_grab(image: &RgbaImage) -> String {
    let pixel = |x: u32, y: u32| {
        let x = x.min(image.width().saturating_sub(1));
        let y = y.min(image.height().saturating_sub(1));
        let sample = image.get_pixel(x, y);
        format!("{},{},{}", sample[0], sample[1], sample[2])
    };
    format!(
        "png={}x{} left={} right={} hotspot={}",
        image.width(),
        image.height(),
        pixel(image.width() / 4, image.height() / 2),
        pixel(image.width() * 3 / 4, image.height() / 2),
        pixel(20, 20)
    )
}

fn exercise_pointer(shared: &Shared, window: &SeatWindow) {
    let pointer = match VirtualPointer::connect() {
        Ok(pointer) => pointer,
        Err(error) => {
            shared.push(format!("pointer=unavailable {error}"));
            return;
        }
    };
    let layout = logical_layout();
    let center_x = window.x + window.w / 2;
    let center_y = window.y + window.h / 2;
    let outside_x = window.x - 80;
    let outside_y = window.y + 8;
    pointer.warp_to(outside_x, outside_y, &layout);
    settle_pointer(&pointer, outside_x, outside_y);
    shared.push(format!("cursor-outside {:?}", cursor_pos()));
    pointer.warp_to(center_x, center_y, &layout);
    settle_pointer(&pointer, center_x, center_y);
    shared.push(format!("cursor-before-move {:?}", cursor_pos()));
    shared.push(format!(
        "cursor-after-move {:?} target={center_x},{center_y} box={},{},{}x{}",
        cursor_pos(),
        layout.min_x,
        layout.min_y,
        layout.width,
        layout.height
    ));
    shared.push(format!("cursor-clients {}", windows_at_cursor()));
    for (button, _name) in [(0x110u32, "left"), (0x111, "right"), (0x112, "middle")] {
        pointer.button(button, true);
        thread::sleep(Duration::from_millis(150));
        pointer.button(button, false);
        thread::sleep(Duration::from_millis(80));
    }
    pointer.wheel(-1);
    thread::sleep(Duration::from_millis(80));
    log_hypr(
        shared,
        "shift-down",
        "hl.dsp.send_key_state({ mods = \"\", key = \"code:50\", state = \"down\" })",
    );
    thread::sleep(Duration::from_millis(40));
    pointer.wheel(-1);
    thread::sleep(Duration::from_millis(80));
    log_hypr(
        shared,
        "shift-up",
        "hl.dsp.send_key_state({ mods = \"\", key = \"code:50\", state = \"up\" })",
    );
    pointer.warp_to(center_x + 40, center_y + 24, &layout);
    thread::sleep(Duration::from_millis(200));
    let events = shared.pointer.lock().expect("pointer").clone();
    shared.push(format!("pointer-count={}", events.len()));
    let mut moves = 0;
    for line in &events {
        if line.contains("move") {
            if moves >= 2 {
                continue;
            }
            moves += 1;
        }
        shared.push(format!("pointer-line {line}"));
    }
}

fn exercise_cursor(shared: &Shared, window: &SeatWindow) {
    let before = cursor_patch(window);
    let _ = hypr(&format!(
        "hl.dsp.send_key_state({{ mods = \"\", key = \"code:38\", state = \"down\", window = \"address:{}\" }})",
        window.address
    ));
    thread::sleep(Duration::from_millis(30));
    let _ = hypr(&format!(
        "hl.dsp.send_key_state({{ mods = \"\", key = \"code:38\", state = \"up\", window = \"address:{}\" }})",
        window.address
    ));
    thread::sleep(Duration::from_millis(200));
    let after = cursor_patch(window);
    shared.push(format!("cursor-before {before}"));
    shared.push(format!("cursor-after {after}"));
    shared.push(
        "cursor-style CursorStyle has no hidden variant; hide is wl_pointer.set_cursor(None) on typing"
            .to_string(),
    );
}

fn cursor_patch(_window: &SeatWindow) -> String {
    "screencopy-timeout: cursor pixels were not captured".to_string()
}

fn fullscreen_flag(title: &str) -> String {
    let Ok(output) = Command::new("hyprctl").args(["clients", "-j"]).output() else {
        return "unreadable".to_string();
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&output.stdout) else {
        return "json".to_string();
    };
    let pid = u64::from(std::process::id());
    for client in value.as_array().into_iter().flatten() {
        if client.get("pid").and_then(|pid| pid.as_u64()) != Some(pid) {
            continue;
        }
        if client.get("title").and_then(|title| title.as_str()) != Some(title) {
            continue;
        }
        let flag = client
            .get("fullscreen")
            .and_then(|value| value.as_i64())
            .or_else(|| {
                client
                    .get("fullscreen")
                    .and_then(|value| value.as_bool())
                    .map(i64::from)
            })
            .unwrap_or(-1);
        return flag.to_string();
    }
    "missing".to_string()
}

fn count_titles() -> usize {
    let Ok(output) = Command::new("hyprctl").args(["clients", "-j"]).output() else {
        return 0;
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&output.stdout) else {
        return 0;
    };
    let pid = u64::from(std::process::id());
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter(|client| client.get("pid").and_then(|pid| pid.as_u64()) == Some(pid))
        .count()
}

fn monitor_origin() -> (i32, i32) {
    let Ok(output) = Command::new("hyprctl").args(["monitors", "-j"]).output() else {
        return (0, 0);
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&output.stdout) else {
        return (0, 0);
    };
    let Some(monitor) = value.as_array().and_then(|rows| {
        rows.iter()
            .find(|row| row.get("focused").and_then(|value| value.as_bool()) == Some(true))
            .or_else(|| rows.first())
    }) else {
        return (0, 0);
    };
    let x = monitor
        .get("x")
        .and_then(|value| value.as_i64())
        .unwrap_or(0) as i32;
    let y = monitor
        .get("y")
        .and_then(|value| value.as_i64())
        .unwrap_or(0) as i32;
    (x, y)
}

struct Layout {
    min_x: i32,
    min_y: i32,
    width: i32,
    height: i32,
}

fn logical_layout() -> Layout {
    let monitors = monitor_rows();
    let mut min_x = 0;
    let mut min_y = 0;
    let mut max_x = 1;
    let mut max_y = 1;
    for (index, monitor) in monitors.iter().enumerate() {
        if index == 0 {
            min_x = monitor.x;
            min_y = monitor.y;
            max_x = monitor.x + monitor.logical_w;
            max_y = monitor.y + monitor.logical_h;
            continue;
        }
        min_x = min_x.min(monitor.x);
        min_y = min_y.min(monitor.y);
        max_x = max_x.max(monitor.x + monitor.logical_w);
        max_y = max_y.max(monitor.y + monitor.logical_h);
    }
    Layout {
        min_x,
        min_y,
        width: (max_x - min_x).max(1),
        height: (max_y - min_y).max(1),
    }
}

fn evaluate(shared: &Shared) -> bool {
    let lines = shared.lines.lock().expect("log").clone();
    let text = lines.join("\n");
    let fit = lines.iter().find(|line| line.starts_with("fit-sample"));
    let replaced = lines
        .iter()
        .find(|line| line.starts_with("replaced-sample"));
    let actual = lines.iter().find(|line| line.starts_with("actual-sample"));
    let mut missing = Vec::new();
    match (fit, replaced, actual) {
        (Some(fit), Some(replaced), Some(actual))
            if fit.contains("screencopy-timeout")
                || replaced.contains("screencopy-timeout")
                || actual.contains("screencopy-timeout") =>
        {
            missing.push("frame pixels: screencopy never completes on this seat".to_string());
        }
        (Some(fit), Some(replaced), Some(actual)) => {
            shared.push(format!("frames fit={fit}"));
            if !colors_differ(fit) {
                missing.push("fit left and right colors".to_string());
            }
            if !replaced_is_new(fit, replaced) {
                missing.push("replaced frame still shows the first colors".to_string());
            }
            if !actual_is_unscaled(actual) {
                missing.push("actual size still fills the window".to_string());
            }
        }
        _ => missing.push("frame screenshots".to_string()),
    }
    let pointer = text.as_str();
    for name in [
        "button down left",
        "button up left",
        "button down right",
        "button up right",
        "button down middle",
        "button up middle",
    ] {
        if !pointer.contains(name) {
            missing.push(name.to_string());
        }
    }
    if !pointer.contains("pointer-line move") {
        missing.push("pointer motion".to_string());
    }
    if !pointer.contains("pointer-line wheel") {
        missing.push("wheel".to_string());
    }
    if !text.contains("wheel shift=true") {
        missing.push("shift+wheel".to_string());
    }
    let marker = format!("xenadmin-s1-{}", std::process::id());
    let pasted = text.lines().any(|line| {
        line.starts_with("clipboard wl-paste=") && line.contains(&marker)
            || line.starts_with("clipboard gpui_later=") && line.contains(&marker)
    });
    if !pasted {
        missing.push("clipboard round trip".to_string());
    }
    if !text.contains("TypeText") {
        missing.push("type clipboard as keys".to_string());
    }
    if !text.contains("fullscreen-on 1") && !text.contains("fullscreen-on 2") {
        missing.push(format!(
            "fullscreen entered ({})",
            fullscreen_line(&text, "fullscreen-on")
        ));
    }
    if !text.contains("fullscreen-off 0") {
        missing.push(format!(
            "fullscreen exited ({})",
            fullscreen_line(&text, "fullscreen-off")
        ));
    }
    if !text.contains("window-count=") || count_from(&text) < 8 {
        missing.push(format!("eight windows ({})", count_from(&text)));
    }
    if !text.contains("memory-rss-kb=") {
        missing.push("frame memory samples".to_string());
    }
    if missing.is_empty() {
        true
    } else {
        for item in missing {
            shared.push(format!("missing {item}"));
        }
        false
    }
}

fn colors_differ(line: &str) -> bool {
    let Some((left, right)) = sample_pair(line) else {
        return false;
    };
    left != right && saturated(left) && saturated(right)
}

fn replaced_is_new(fit: &str, replaced: &str) -> bool {
    let Some((fit_left, _)) = sample_pair(fit) else {
        return false;
    };
    let Some((left, right)) = sample_pair(replaced) else {
        return false;
    };
    left == right && left != fit_left && saturated(left)
}

fn actual_is_unscaled(line: &str) -> bool {
    let Some((left, right)) = sample_pair(line) else {
        return false;
    };
    // The 160px bitmap sits at the origin. Three quarters across a 480px
    // window is outside that bitmap, so it is the black canvas.
    saturated(left) && !saturated(right)
}

fn sample_pair(line: &str) -> Option<([u8; 3], [u8; 3])> {
    let left = line.split("left=").nth(1)?.split(' ').next()?;
    let right = line.split("right=").nth(1)?.split(' ').next()?;
    Some((parse_rgb(left)?, parse_rgb(right)?))
}

fn parse_rgb(text: &str) -> Option<[u8; 3]> {
    let mut parts = text.split(',');
    Some([
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    ])
}

fn saturated(pixel: [u8; 3]) -> bool {
    pixel.iter().any(|channel| *channel > 180)
}

fn fullscreen_line(text: &str, prefix: &str) -> String {
    text.lines()
        .find(|line| line.starts_with(prefix))
        .unwrap_or("absent")
        .to_string()
}

fn count_from(text: &str) -> usize {
    text.lines()
        .find_map(|line| line.strip_prefix("window-count="))
        .and_then(|value| value.parse().ok())
        .unwrap_or(0)
}

fn log_hypr(shared: &Shared, label: &str, lua: &str) {
    match hypr(lua) {
        Ok(text) => shared.push(format!("{label} {}", text.trim())),
        Err(text) => shared.push(format!("{label}-error {}", one_line(&text))),
    }
}

fn focus_window(shared: &Shared, address: &str) {
    log_hypr(
        shared,
        "focus",
        &format!("hl.dsp.focus({{ window = 'address:{address}' }})"),
    );
    thread::sleep(Duration::from_millis(150));
    let active = active_line();
    shared.push(format!("active {active}"));
    if !active.contains(TITLE) {
        log_hypr(
            shared,
            "focus-pid",
            &format!("hl.dsp.focus({{ window = 'pid:{}' }})", std::process::id()),
        );
        thread::sleep(Duration::from_millis(150));
        shared.push(format!("active-after-pid {}", active_line()));
    }
}

fn windows_at_cursor() -> String {
    let Some((x, y)) = cursor_pos() else {
        return "no-cursor".to_string();
    };
    let Ok(output) = Command::new("hyprctl").args(["clients", "-j"]).output() else {
        return "unreadable".to_string();
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&output.stdout) else {
        return "json".to_string();
    };
    let mut titles = Vec::new();
    for client in value.as_array().into_iter().flatten() {
        let at = client.get("at").and_then(|at| at.as_array());
        let size = client.get("size").and_then(|size| size.as_array());
        let (Some(at), Some(size)) = (at, size) else {
            continue;
        };
        let origin_x = at.first().and_then(|value| value.as_i64()).unwrap_or(0);
        let origin_y = at.get(1).and_then(|value| value.as_i64()).unwrap_or(0);
        let width = size.first().and_then(|value| value.as_i64()).unwrap_or(0);
        let height = size.get(1).and_then(|value| value.as_i64()).unwrap_or(0);
        if i64::from(x) < origin_x
            || i64::from(y) < origin_y
            || i64::from(x) >= origin_x + width
            || i64::from(y) >= origin_y + height
        {
            continue;
        }
        let title = client
            .get("title")
            .and_then(|title| title.as_str())
            .unwrap_or("");
        titles.push(title.to_string());
    }
    if titles.is_empty() {
        "none".to_string()
    } else {
        titles.join(" | ")
    }
}

fn active_line() -> String {
    let Ok(output) = Command::new("hyprctl")
        .args(["activewindow", "-j"])
        .output()
    else {
        return "unreadable".to_string();
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&output.stdout) else {
        return "json".to_string();
    };
    format!(
        "{} {}",
        value
            .get("title")
            .and_then(|title| title.as_str())
            .unwrap_or(""),
        value
            .get("address")
            .and_then(|address| address.as_str())
            .unwrap_or("")
    )
}

fn settle_pointer(pointer: &VirtualPointer, target_x: i32, target_y: i32) {
    thread::sleep(Duration::from_millis(60));
    for _ in 0..6 {
        let Some((x, y)) = cursor_pos() else {
            break;
        };
        if (x - target_x).abs() <= 2 && (y - target_y).abs() <= 2 {
            break;
        }
        pointer.motion_relative(f64::from(target_x - x), f64::from(target_y - y));
        thread::sleep(Duration::from_millis(40));
    }
}

fn wait_for_line(shared: &Shared, needle: &str, limit: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < limit {
        let found = shared
            .lines
            .lock()
            .expect("log")
            .iter()
            .any(|line| line.contains(needle));
        if found {
            return true;
        }
        thread::sleep(Duration::from_millis(50));
    }
    false
}

fn window_facts(address: &str) -> String {
    let Ok(output) = Command::new("hyprctl").args(["clients", "-j"]).output() else {
        return "unreadable".to_string();
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&output.stdout) else {
        return "json".to_string();
    };
    let Some(client) = value.as_array().and_then(|rows| {
        rows.iter()
            .find(|client| client.get("address").and_then(|value| value.as_str()) == Some(address))
    }) else {
        return "missing".to_string();
    };
    let workspace = client
        .get("workspace")
        .and_then(|workspace| workspace.get("name"))
        .and_then(|name| name.as_str())
        .unwrap_or("");
    let monitor = client
        .get("monitor")
        .and_then(|monitor| monitor.as_i64())
        .unwrap_or(-1);
    let floating = client
        .get("floating")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let pinned = client
        .get("pinned")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    format!("workspace={workspace} monitor={monitor} floating={floating} pinned={pinned}")
}

fn poll_fullscreen(want_on: bool) -> String {
    let start = Instant::now();
    let mut last = String::from("missing");
    while start.elapsed() < Duration::from_secs(2) {
        last = fullscreen_flag(TITLE);
        let on = last != "0" && last != "missing" && last != "-1";
        if on == want_on {
            return last;
        }
        thread::sleep(Duration::from_millis(100));
    }
    last
}

fn clipboard_text() -> String {
    let output = Command::new("wl-paste")
        .args(["-n", "-t", "text/plain"])
        .output();
    let Ok(output) = output else {
        return "wl-paste-missing".to_string();
    };
    if !output.status.success() {
        return format!("wl-paste-status {}", output.status);
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.starts_with("xenadmin-s1-") {
        text
    } else {
        format!("not-our-text len={}", text.len())
    }
}

struct SavedClipboard {
    mime: String,
    bytes: Vec<u8>,
}

fn saved_clipboard() -> Option<SavedClipboard> {
    let types = Command::new("wl-paste").arg("-l").output().ok()?;
    if !types.status.success() {
        return None;
    }
    let listed = String::from_utf8_lossy(&types.stdout);
    let mime = if listed.lines().any(|line| line.trim() == "image/png") {
        "image/png"
    } else {
        "text/plain"
    };
    let output = Command::new("wl-paste")
        .args(["-n", "-t", mime])
        .output()
        .ok()?;
    if !output.status.success() || output.stdout.is_empty() {
        return None;
    }
    Some(SavedClipboard {
        mime: mime.to_string(),
        bytes: output.stdout,
    })
}

fn restore_clipboard(saved: &Option<SavedClipboard>) {
    let Some(saved) = saved else {
        return;
    };
    let Ok(mut child) = Command::new("wl-copy")
        .args(["-t", &saved.mime])
        .stdin(Stdio::piped())
        .spawn()
    else {
        return;
    };
    if let Some(stdin) = child.stdin.as_mut() {
        let _ = stdin.write_all(&saved.bytes);
    }
    drop(child.stdin.take());
    let _ = child.wait();
}

struct MonitorBox {
    name: String,
    x: i32,
    y: i32,
    logical_w: i32,
    logical_h: i32,
    scale: f64,
}

fn monitor_rows() -> Vec<MonitorBox> {
    let Ok(output) = Command::new("hyprctl").args(["monitors", "-j"]).output() else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&output.stdout) else {
        return Vec::new();
    };
    value
        .as_array()
        .into_iter()
        .flatten()
        .map(|monitor| {
            let scale = monitor
                .get("scale")
                .and_then(|value| value.as_f64())
                .unwrap_or(1.0)
                .max(0.1);
            let width = monitor
                .get("width")
                .and_then(|value| value.as_i64())
                .unwrap_or(1);
            let height = monitor
                .get("height")
                .and_then(|value| value.as_i64())
                .unwrap_or(1);
            MonitorBox {
                name: monitor
                    .get("name")
                    .and_then(|value| value.as_str())
                    .unwrap_or("")
                    .to_string(),
                x: monitor
                    .get("x")
                    .and_then(|value| value.as_i64())
                    .unwrap_or(0) as i32,
                y: monitor
                    .get("y")
                    .and_then(|value| value.as_i64())
                    .unwrap_or(0) as i32,
                logical_w: (f64::from(width as i32) / scale).round() as i32,
                logical_h: (f64::from(height as i32) / scale).round() as i32,
                scale,
            }
        })
        .collect()
}

fn monitor_at(x: i32, y: i32) -> (String, i32, i32, f64) {
    let monitors = monitor_rows();
    let monitor = monitors.iter().find(|monitor| {
        x >= monitor.x
            && y >= monitor.y
            && x < monitor.x + monitor.logical_w
            && y < monitor.y + monitor.logical_h
    });
    let Some(monitor) = monitor.or_else(|| monitors.first()) else {
        return (String::new(), 0, 0, 1.0);
    };
    (monitor.name.clone(), monitor.x, monitor.y, monitor.scale)
}

fn our_client_summary() -> String {
    let Ok(output) = Command::new("hyprctl").args(["clients", "-j"]).output() else {
        return "unreadable".to_string();
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&output.stdout) else {
        return "json".to_string();
    };
    let pid = u64::from(std::process::id());
    let mut titles = Vec::new();
    for client in value.as_array().into_iter().flatten() {
        if client.get("pid").and_then(|pid| pid.as_u64()) != Some(pid) {
            continue;
        }
        let title = client
            .get("title")
            .and_then(|title| title.as_str())
            .unwrap_or("");
        let mapped = client
            .get("mapped")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        titles.push(format!("{title}:{mapped}"));
    }
    titles.join(" ")
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn hypr(lua: &str) -> Result<String, String> {
    let output = Command::new("hyprctl")
        .args(["dispatch", lua])
        .output()
        .map_err(|error| error.to_string())?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if !output.status.success() || text.to_ascii_lowercase().contains("error:") {
        return Err(text.trim().to_string());
    }
    Ok(text)
}

fn cursor_pos() -> Option<(i32, i32)> {
    let output = Command::new("hyprctl").arg("cursorpos").output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let mut parts = text.split(',');
    let x = parts.next()?.trim().parse().ok()?;
    let y = parts.next()?.trim().parse().ok()?;
    Some((x, y))
}

fn rss_kb() -> u64 {
    let Ok(text) = std::fs::read_to_string("/proc/self/status") else {
        return 0;
    };
    text.lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|value| value.parse().ok())
        .unwrap_or(0)
}

struct VirtualPointer {
    connection: Connection,
    pointer: ZwlrVirtualPointerV1,
}

impl VirtualPointer {
    fn connect() -> Result<Self, String> {
        let connection = Connection::connect_to_env().map_err(|error| error.to_string())?;
        let mut queue = connection.new_event_queue();
        let handle = queue.handle();
        let _registry = connection.display().get_registry(&handle, ());
        let mut state = PointerState::default();
        queue
            .roundtrip(&mut state)
            .map_err(|error| error.to_string())?;
        queue
            .roundtrip(&mut state)
            .map_err(|error| error.to_string())?;
        let manager = state
            .manager
            .ok_or("the compositor has no virtual pointer")?;
        let pointer = manager.create_virtual_pointer(None, &handle, ());
        connection.flush().ok();
        Ok(Self {
            connection,
            pointer,
        })
    }

    fn warp_to(&self, x: i32, y: i32, layout: &Layout) {
        self.move_to(
            x - layout.min_x,
            y - layout.min_y,
            layout.width,
            layout.height,
        );
    }

    fn motion_relative(&self, dx: f64, dy: f64) {
        self.pointer.motion(monotonic_ms(), dx, dy);
        self.pointer.frame();
        self.connection.flush().ok();
    }

    fn move_to(&self, x: i32, y: i32, width: i32, height: i32) {
        let time = monotonic_ms();
        self.pointer.motion_absolute(
            time,
            x.max(0) as u32,
            y.max(0) as u32,
            width.max(1) as u32,
            height.max(1) as u32,
        );
        self.pointer.frame();
        self.connection.flush().ok();
    }

    fn button(&self, button: u32, pressed: bool) {
        let state = if pressed {
            wl_pointer::ButtonState::Pressed
        } else {
            wl_pointer::ButtonState::Released
        };
        self.pointer.button(monotonic_ms(), button, state);
        self.pointer.frame();
        self.connection.flush().ok();
    }

    fn wheel(&self, discrete: i32) {
        let time = monotonic_ms();
        self.pointer.axis_source(wl_pointer::AxisSource::Wheel);
        self.pointer.axis_discrete(
            time,
            wl_pointer::Axis::VerticalScroll,
            f64::from(discrete) * 15.0,
            discrete,
        );
        self.pointer.frame();
        self.connection.flush().ok();
    }
}

#[derive(Default)]
struct PointerState {
    manager: Option<ZwlrVirtualPointerManagerV1>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for PointerState {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        (): &(),
        _: &Connection,
        handle: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
            && interface == "zwlr_virtual_pointer_manager_v1"
        {
            state.manager = Some(registry.bind(name, version.min(2), handle, ()));
        }
    }
}

wayland_client::delegate_noop!(PointerState: ignore ZwlrVirtualPointerManagerV1);
wayland_client::delegate_noop!(PointerState: ignore ZwlrVirtualPointerV1);

fn monotonic_ms() -> u32 {
    let Ok(clock) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) else {
        return 0;
    };
    clock.as_millis() as u32
}

static CAPTURE_DEAD: AtomicBool = AtomicBool::new(false);

fn capture_region(
    output_name: &str,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    cursor: bool,
) -> Result<RgbaImage, String> {
    if CAPTURE_DEAD.load(Ordering::SeqCst) {
        return Err("screencopy-unavailable".to_string());
    }
    let (sender, receiver) = mpsc::channel();
    let output_name = output_name.to_string();
    thread::spawn(move || {
        let _ = sender.send(capture_blocking(&output_name, x, y, width, height, cursor));
    });
    match receiver.recv_timeout(Duration::from_secs(3)) {
        Ok(result) => result,
        Err(_) => {
            CAPTURE_DEAD.store(true, Ordering::SeqCst);
            Err("screencopy-timeout".to_string())
        }
    }
}

fn capture_blocking(
    output_name: &str,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    cursor: bool,
) -> Result<RgbaImage, String> {
    let connection = Connection::connect_to_env().map_err(|error| error.to_string())?;
    let mut queue = connection.new_event_queue();
    let handle = queue.handle();
    let _registry = connection.display().get_registry(&handle, ());
    let mut state = CapState::default();
    queue
        .roundtrip(&mut state)
        .map_err(|error| error.to_string())?;
    queue
        .roundtrip(&mut state)
        .map_err(|error| error.to_string())?;
    println!(
        "capture-stage outputs={} manager={}",
        state.outputs.len(),
        state.manager.is_some()
    );
    let _ = std::io::Write::flush(&mut std::io::stdout());
    let output = state
        .outputs
        .iter()
        .find(|output| output.name == output_name)
        .ok_or_else(|| format!("output {output_name} not found"))?
        .output
        .clone();
    let manager = state
        .manager
        .clone()
        .ok_or("the compositor has no zwlr_screencopy_manager_v1")?;
    let frame =
        manager.capture_output_region(i32::from(cursor), &output, x, y, width, height, &handle, ());
    queue
        .roundtrip(&mut state)
        .map_err(|error| error.to_string())?;
    println!(
        "capture-stage buffer={} {}x{} stride={}",
        state.format.is_some(),
        state.width,
        state.height,
        state.stride
    );
    let _ = std::io::Write::flush(&mut std::io::stdout());
    let format = state.format.ok_or("screencopy offered no shm buffer")?;
    let shm = state.shm.clone().ok_or("the compositor has no wl_shm")?;
    let byte_len =
        usize::try_from(state.stride).unwrap_or(0) * usize::try_from(state.height).unwrap_or(0);
    if byte_len == 0 {
        return Err("screencopy buffer was empty".to_string());
    }
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open("/tmp/s1-screencopy-shm")
        .map_err(|error| error.to_string())?;
    file.set_len(byte_len as u64)
        .map_err(|error| error.to_string())?;
    let pool = shm.create_pool(file.as_fd(), byte_len as i32, &handle, ());
    let buffer = pool.create_buffer(
        0,
        state.width,
        state.height,
        state.stride,
        format,
        &handle,
        (),
    );
    frame.copy(&buffer);
    println!("capture-stage copy");
    let _ = std::io::Write::flush(&mut std::io::stdout());
    while !state.ready && !state.failed {
        queue
            .blocking_dispatch(&mut state)
            .map_err(|error| error.to_string())?;
    }
    if state.failed {
        return Err("screencopy-failed".to_string());
    }
    file.seek(SeekFrom::Start(0))
        .map_err(|error| error.to_string())?;
    let mut raw = vec![0u8; byte_len];
    file.read_exact(&mut raw)
        .map_err(|error| error.to_string())?;
    Ok(shm_to_rgba(&raw, state.width, state.height, state.stride))
}

fn shm_to_rgba(raw: &[u8], width: i32, height: i32, stride: i32) -> RgbaImage {
    let width = u32::try_from(width).unwrap_or(0);
    let height = u32::try_from(height).unwrap_or(0);
    let mut image = RgbaImage::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let offset = y as usize * stride as usize + x as usize * 4;
            let pixel = raw.get(offset..offset + 4).unwrap_or(&[0, 0, 0, 0]);
            image.put_pixel(x, y, Rgba([pixel[2], pixel[1], pixel[0], 255]));
        }
    }
    image
}

struct CapOutput {
    output: wl_output::WlOutput,
    name: String,
}

#[derive(Default)]
struct CapState {
    shm: Option<wl_shm::WlShm>,
    manager: Option<ZwlrScreencopyManagerV1>,
    outputs: Vec<CapOutput>,
    format: Option<wl_shm::Format>,
    width: i32,
    height: i32,
    stride: i32,
    ready: bool,
    failed: bool,
}

impl Dispatch<wl_registry::WlRegistry, ()> for CapState {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        (): &(),
        _: &Connection,
        handle: &QueueHandle<Self>,
    ) {
        let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        else {
            return;
        };
        match interface.as_str() {
            "wl_shm" => state.shm = Some(registry.bind(name, version.min(1), handle, ())),
            "zwlr_screencopy_manager_v1" => {
                state.manager = Some(registry.bind(name, version.min(3), handle, ()));
            }
            "wl_output" => state.outputs.push(CapOutput {
                output: registry.bind(name, version.min(4), handle, ()),
                name: String::new(),
            }),
            _ => {}
        }
    }
}

impl Dispatch<wl_output::WlOutput, ()> for CapState {
    fn event(
        state: &mut Self,
        proxy: &wl_output::WlOutput,
        event: wl_output::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Name { name } = event {
            let id = Proxy::id(proxy);
            if let Some(slot) = state
                .outputs
                .iter_mut()
                .find(|slot| Proxy::id(&slot.output) == id)
            {
                slot.name = name;
            }
        }
    }
}

impl Dispatch<ZwlrScreencopyFrameV1, ()> for CapState {
    fn event(
        state: &mut Self,
        _: &ZwlrScreencopyFrameV1,
        event: zwlr_screencopy_frame_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_screencopy_frame_v1::Event::Buffer {
                format,
                width,
                height,
                stride,
            } => {
                state.format = format.into_result().ok();
                state.width = i32::try_from(width).unwrap_or(0);
                state.height = i32::try_from(height).unwrap_or(0);
                state.stride = i32::try_from(stride).unwrap_or(0);
            }
            zwlr_screencopy_frame_v1::Event::Ready { .. } => state.ready = true,
            zwlr_screencopy_frame_v1::Event::Failed => state.failed = true,
            _ => {}
        }
    }
}

wayland_client::delegate_noop!(CapState: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(CapState: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(CapState: ignore wl_buffer::WlBuffer);
wayland_client::delegate_noop!(CapState: ignore ZwlrScreencopyManagerV1);
