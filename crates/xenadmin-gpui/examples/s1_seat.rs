//! Spike S1 on a real Wayland seat.
//!
//! Opens two GPUI windows, binds a second `wl_keyboard` on the first window's
//! display, and asks Hyprland to inject physical keys with `send_key_state`.
//! Each evdev code is turned into `ConsoleInput::PhysicalKey` with the XT make
//! code QEMU's extended key event uses.
//!
//! This is not the released binary. It does not open a console to a host.

use std::collections::BTreeSet;
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use gpui_kit::prelude::*;
use gpui_kit::{
    Bounds, Context, Entity, Render, TitlebarOptions, Window, WindowBounds, WindowOptions, div,
    point, px, size,
};
use xenadmin_gpui::{ForeignKeyboard, SeatEvent, console_physical_key, xt_make_code};

const TITLE_A: &str = "xenadmin-s1-a";
const TITLE_B: &str = "xenadmin-s1-b";

struct Shared {
    lines: Mutex<Vec<String>>,
    gpui: Mutex<Vec<String>>,
    start: AtomicBool,
    done: AtomicBool,
    /// 0 none, 1 window A, 2 window B. The injector sets this before each key.
    target: AtomicU8,
}

struct Probe {
    keyboard: Option<ForeignKeyboard>,
    shared: Arc<Shared>,
    started: Instant,
    ticks: u32,
    keys: Vec<(u32, bool)>,
    enter_ids: BTreeSet<u32>,
    leave_releases: Vec<u32>,
    held: BTreeSet<u32>,
    quiet_since: Option<Instant>,
}

impl Probe {
    fn new(window: &mut Window, shared: Arc<Shared>) -> Self {
        let keyboard = match ForeignKeyboard::from_gpui_display(window) {
            Ok(keyboard) => {
                if let Err(error) = keyboard.flush() {
                    shared.push(format!("flush=error {error}"));
                }
                Some(keyboard)
            }
            Err(error) => {
                shared.push(format!("attach=error {error}"));
                None
            }
        };
        match ForeignKeyboard::gpui_surface_ptr(window) {
            Ok(surface) => shared.push(format!("gpui-surface-a={surface:#x}")),
            Err(error) => shared.push(format!("gpui-surface-a=error {error}")),
        }
        Self {
            keyboard,
            shared,
            started: Instant::now(),
            ticks: 0,
            keys: Vec::new(),
            enter_ids: BTreeSet::new(),
            leave_releases: Vec::new(),
            held: BTreeSet::new(),
            quiet_since: None,
        }
    }

    fn tick(&mut self) -> bool {
        self.ticks = self.ticks.wrapping_add(1);
        if self.ticks == 1 || self.ticks.is_multiple_of(30) {
            eprintln!(
                "tick={} bound={} done={} events={}",
                self.ticks,
                self.keyboard
                    .as_ref()
                    .is_some_and(|keyboard| keyboard.keyboard_bound()),
                self.shared.done.load(Ordering::SeqCst),
                self.keys.len(),
            );
        }
        if self.keyboard.is_none() {
            self.finish(false);
            return false;
        }
        if self.started.elapsed() > Duration::from_secs(25) {
            self.shared.push("timeout=25s".to_string());
            self.finish(false);
            return false;
        }
        let target = self.shared.target.load(Ordering::SeqCst);
        let step = self.keyboard.as_mut().map(|keyboard| {
            let flushed = keyboard.flush();
            let events = keyboard.dispatch_seat();
            let bound = keyboard.keyboard_bound();
            (flushed, events, bound)
        });
        let Some((flushed, events, bound)) = step else {
            return false;
        };
        if let Err(error) = flushed {
            self.shared.push(format!("flush=error {error}"));
        }
        let events = match events {
            Ok(events) => events,
            Err(error) => {
                self.shared.push(format!("dispatch=error {error}"));
                self.finish(false);
                return false;
            }
        };
        for event in events {
            self.record(target, event);
        }
        if bound && !self.shared.start.load(Ordering::SeqCst) {
            self.shared.push("keyboard_bound=true".to_string());
            self.shared.start.store(true, Ordering::SeqCst);
        }
        if self.shared.done.load(Ordering::SeqCst) {
            let quiet = self.quiet_since.get_or_insert_with(Instant::now);
            if quiet.elapsed() > Duration::from_millis(400) {
                let pass = self.assess();
                self.finish(pass);
                return false;
            }
        } else {
            self.quiet_since = None;
        }
        true
    }

    fn record(&mut self, target: u8, event: SeatEvent) {
        let target = match target {
            1 => "a",
            2 => "b",
            _ => "-",
        };
        match event {
            SeatEvent::Enter {
                surface,
                protocol_id,
                held,
            } => {
                self.enter_ids.insert(protocol_id);
                self.shared.push(format!(
                    "seat target={target} enter surface={surface:#x} id={protocol_id} held={held:?}"
                ));
            }
            SeatEvent::Leave {
                surface,
                protocol_id,
            } => {
                let released = self.release_held();
                self.shared.push(format!(
                    "seat target={target} leave surface={surface:#x} id={protocol_id} released={released:?}"
                ));
            }
            SeatEvent::Key { evdev, down } => {
                if down {
                    self.held.insert(evdev);
                } else {
                    self.held.remove(&evdev);
                }
                self.keys.push((evdev, down));
                self.log_physical(target, evdev, down, false);
            }
            SeatEvent::Modifiers {
                depressed,
                latched,
                locked,
                group,
            } => {
                self.shared.push(format!(
                    "seat target={target} modifiers depressed={depressed:#x} latched={latched:#x} locked={locked:#x} group={group}"
                ));
            }
        }
    }

    fn release_held(&mut self) -> Vec<u32> {
        let held: Vec<u32> = self.held.iter().copied().collect();
        for evdev in &held {
            self.keys.push((*evdev, false));
            self.leave_releases.push(*evdev);
            self.log_physical("-", *evdev, false, true);
        }
        self.held.clear();
        held
    }

    fn log_physical(&self, target: &str, evdev: u32, down: bool, synthesized: bool) {
        let input = console_physical_key(evdev, down);
        let xt = xt_make_code(evdev)
            .map(|code| format!("{code:#x}"))
            .unwrap_or_else(|| "none".to_string());
        let kind = if synthesized { "leave" } else { "key" };
        self.shared.push(format!(
            "seat target={target} {kind} evdev={evdev} down={down} xt={xt} input={input:?}"
        ));
    }

    fn assess(&mut self) -> bool {
        let mut missing = Vec::new();
        for (evdev, name) in REQUIRED {
            for down in [true, false] {
                if !self.keys.contains(&(*evdev, down)) {
                    missing.push(format!(
                        "{name} {} evdev={evdev}",
                        if down { "down" } else { "up" }
                    ));
                }
            }
        }
        let repeats = self.keys.iter().filter(|key| **key == (30, true)).count();
        if repeats < 2 {
            missing.push(format!("A repeat (downs={repeats})"));
        }
        if !self.leave_releases.contains(&48) {
            missing.push("focus-loss release of B".to_string());
        }
        if self.enter_ids.len() < 2 {
            missing.push(format!(
                "two console surfaces (saw {})",
                self.enter_ids.len()
            ));
        }
        let gpui = self.shared.gpui.lock().expect("gpui log").clone();
        let gpui_saw_a = gpui.iter().any(|line| line.contains(" key=a "));
        let gpui_modifier = gpui.iter().any(|line| {
            const MODIFIERS: &[&str] = &[
                " key=shift ",
                " key=ctrl ",
                " key=control ",
                " key=alt ",
                " key=super ",
                " key=meta ",
                " key=left_shift ",
                " key=right_shift ",
                " key=left_ctrl ",
                " key=right_ctrl ",
                " key=left_alt ",
                " key=right_alt ",
            ];
            MODIFIERS.iter().any(|name| line.contains(name))
        });
        self.shared.push(format!(
            "gpui_keystrokes={} gpui_saw_a={gpui_saw_a} gpui_modifier_key={gpui_modifier}",
            gpui.len()
        ));
        if !gpui_saw_a {
            self.shared
                .push("note GPUI delivered no keystroke named a".to_string());
        }
        if missing.is_empty() {
            self.shared.push("s1_result=pass".to_string());
            true
        } else {
            for item in &missing {
                self.shared.push(format!("missing {item}"));
            }
            self.shared.push("s1_result=fail".to_string());
            false
        }
    }

    fn finish(&mut self, pass: bool) {
        self.keyboard.take();
        if !self
            .shared
            .lines
            .lock()
            .expect("log")
            .iter()
            .any(|line| line.starts_with("s1_result="))
        {
            self.shared
                .push(format!("s1_result={}", if pass { "pass" } else { "fail" }));
        }
        self.shared.done.store(true, Ordering::SeqCst);
    }
}

impl Shared {
    fn push(&self, line: String) {
        println!("{line}");
        self.lines.lock().expect("log").push(line);
    }
}

impl Render for Probe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().child("xenadmin S1 seat probe")
    }
}

struct Blank;

impl Render for Blank {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().child("xenadmin S1 seat probe b")
    }
}

fn main() {
    let display = std::env::var("WAYLAND_DISPLAY").unwrap_or_default();
    println!("wayland_display={display}");
    if display.is_empty() {
        println!("s1_result=fail");
        println!("WAYLAND_DISPLAY is unset");
        std::process::exit(2);
    }
    let shared = Arc::new(Shared {
        lines: Mutex::new(Vec::new()),
        gpui: Mutex::new(Vec::new()),
        start: AtomicBool::new(false),
        done: AtomicBool::new(false),
        target: AtomicU8::new(0),
    });
    let injector = {
        let shared = Arc::clone(&shared);
        thread::spawn(move || inject(shared))
    };
    let shared_for_run = Arc::clone(&shared);
    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);
        let shared_keys = Arc::clone(&shared_for_run);
        std::mem::forget(cx.intercept_keystrokes(move |event, window, cx| {
            let surface = ForeignKeyboard::gpui_surface_ptr(window).unwrap_or(0);
            let key = &event.keystroke;
            let line = format!(
                "gpui surface={surface:#x} key={} char={} shift={} ctrl={} alt={} platform={}",
                key.key,
                key.key_char.as_deref().unwrap_or(""),
                key.modifiers.shift,
                key.modifiers.control,
                key.modifiers.alt,
                key.modifiers.platform,
            );
            println!("{line}");
            shared_keys.gpui.lock().expect("gpui log").push(line);
            cx.stop_propagation();
        }));
        let shared_window = Arc::clone(&shared_for_run);
        let (_window_a, probe) = gpui_kit::open_window(options(TITLE_A, true), cx, |window, cx| {
            let probe = cx.new(|_cx| Probe::new(window, Arc::clone(&shared_window)));
            pump(window, cx, probe.clone());
            probe
        })
        .expect("window A opens");
        let _ = probe;
        let shared_b = Arc::clone(&shared_for_run);
        gpui_kit::open_window(options(TITLE_B, false), cx, |window, cx| {
            match ForeignKeyboard::gpui_surface_ptr(window) {
                Ok(surface) => shared_b.push(format!("gpui-surface-b={surface:#x}")),
                Err(error) => shared_b.push(format!("gpui-surface-b=error {error}")),
            }
            cx.new(|_cx| Blank)
        })
        .expect("window B opens");
    });
    let _ = injector.join();
    if let Ok(path) = std::env::var("XENADMIN_S1_LOG") {
        let lines = shared.lines.lock().expect("log");
        let body = lines.join("\n");
        if let Err(error) = std::fs::write(&path, body + "\n") {
            eprintln!("log write failed: {error}");
        }
    }
    let pass = shared
        .lines
        .lock()
        .expect("log")
        .iter()
        .any(|line| line == "s1_result=pass");
    std::process::exit(if pass { 0 } else { 1 });
}

fn pump(window: &mut Window, cx: &gpui_kit::App, probe: Entity<Probe>) {
    // A compositor frame callback stops being scheduled once the window is
    // idle, which drops the drain after the first tick. The foreground
    // timer keeps running with the Wayland read loop.
    let task = window.spawn(cx, async move |cx| {
        loop {
            cx.background_executor()
                .timer(Duration::from_millis(16))
                .await;
            let keep = cx
                .update(|_window, app| probe.update(app, |probe, _cx| probe.tick()))
                .unwrap_or(false);
            if !keep {
                cx.update(|_window, app| app.quit()).ok();
                break;
            }
        }
    });
    task.detach();
}

fn options(title: &str, focus: bool) -> WindowOptions {
    WindowOptions {
        titlebar: Some(TitlebarOptions {
            title: Some(title.to_string().into()),
            ..Default::default()
        }),
        app_id: Some("xenadmin-s1".to_string()),
        focus,
        show: true,
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: point(px(64.), px(64.)),
            size: size(px(320.), px(180.)),
        })),
        ..Default::default()
    }
}

/// Evdev code, and the name used when it is missing from the log.
const REQUIRED: &[(u32, &str)] = &[
    (30, "A"),
    (42, "ShiftLeft"),
    (54, "ShiftRight"),
    (29, "ControlLeft"),
    (97, "ControlRight"),
    (56, "AltLeft"),
    (100, "AltRight"),
    (125, "SuperLeft"),
    (126, "SuperRight"),
    (58, "CapsLock"),
    (69, "NumLock"),
    (70, "ScrollLock"),
    (99, "PrintScreen"),
    (119, "Pause"),
    (139, "Menu"),
    (59, "F1"),
    (106, "Right"),
    (15, "Tab"),
    (1, "Escape"),
    (46, "C"),
    (111, "Delete"),
];

fn inject(shared: Arc<Shared>) {
    let waiting = Instant::now();
    while !shared.start.load(Ordering::SeqCst) {
        if waiting.elapsed() > Duration::from_secs(8) {
            shared.push("inject=keyboard was not bound".to_string());
            shared.done.store(true, Ordering::SeqCst);
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
    thread::sleep(Duration::from_millis(200));
    let windows = match find_windows() {
        Ok(windows) => windows,
        Err(error) => {
            shared.push(format!("inject=windows {error}"));
            shared.done.store(true, Ordering::SeqCst);
            return;
        }
    };
    let Some(window_a) = windows.iter().find(|window| window.title == TITLE_A) else {
        shared.push("inject=window A was not in the compositor client list".to_string());
        shared.done.store(true, Ordering::SeqCst);
        return;
    };
    let Some(window_b) = windows.iter().find(|window| window.title == TITLE_B) else {
        shared.push("inject=window B was not in the compositor client list".to_string());
        shared.done.store(true, Ordering::SeqCst);
        return;
    };
    shared.push(format!(
        "window-a address={} floating={}",
        window_a.address, window_a.floating
    ));
    shared.push(format!(
        "window-b address={} floating={}",
        window_b.address, window_b.floating
    ));
    for window in [window_a, window_b] {
        match hypr(&format!(
            "hl.dsp.window.float({{ action = \"set\", window = \"address:{}\" }})",
            window.address
        )) {
            Ok(text) => shared.push(format!("float {} {}", window.title, text.trim())),
            Err(error) => shared.push(format!("float {} {error}", window.title)),
        }
    }
    let locks = read_locks();
    shared.push(format!("locks-before {locks:?}"));

    // One physical key at a time. Modifier chords are separate presses, which
    // is what an extended key event needs. Ctrl+Alt+F keys are not sent.
    let script = [
        (1u8, 30u32, "down"),
        (1, 30, "up"),
        (1, 30, "repeat"),
        (1, 42, "down"),
        (1, 30, "down"),
        (1, 30, "up"),
        (1, 42, "up"),
        (1, 54, "down"),
        (1, 54, "up"),
        (1, 29, "down"),
        (1, 46, "down"),
        (1, 46, "up"),
        (1, 29, "up"),
        (1, 97, "down"),
        (1, 97, "up"),
        (1, 56, "down"),
        (1, 56, "up"),
        (1, 100, "down"),
        (1, 100, "up"),
        (1, 125, "down"),
        (1, 125, "up"),
        (1, 126, "down"),
        (1, 126, "up"),
        (1, 58, "down"),
        (1, 58, "up"),
        (1, 69, "down"),
        (1, 69, "up"),
        (1, 70, "down"),
        (1, 70, "up"),
        (1, 99, "down"),
        (1, 99, "up"),
        (1, 119, "down"),
        (1, 119, "up"),
        (1, 139, "down"),
        (1, 139, "up"),
        (1, 59, "down"),
        (1, 59, "up"),
        (1, 106, "down"),
        (1, 106, "up"),
        (1, 15, "down"),
        (1, 15, "up"),
        (1, 1, "down"),
        (1, 1, "up"),
        (1, 29, "down"),
        (1, 56, "down"),
        (1, 111, "down"),
        (1, 111, "up"),
        (1, 56, "up"),
        (1, 29, "up"),
        (1, 48, "down"),
        (2, 44, "down"),
        (2, 44, "up"),
    ];
    for (target, evdev, state) in script {
        shared.target.store(target, Ordering::SeqCst);
        let address = if target == 1 {
            &window_a.address
        } else {
            &window_b.address
        };
        let code = evdev + 8;
        let lua = format!(
            "hl.dsp.send_key_state({{ mods = \"\", key = \"code:{code}\", state = \"{state}\", window = \"address:{address}\" }})"
        );
        match hypr(&lua) {
            Ok(_) => shared.push(format!(
                "inject target={target} evdev={evdev} state={state}"
            )),
            Err(error) => shared.push(format!(
                "inject-error target={target} evdev={evdev} state={state} {error}"
            )),
        }
        thread::sleep(Duration::from_millis(40));
    }
    // The held B is released by focus loss. Send the matching up afterwards
    // so the client is not left with B down if the compositor did not leave.
    thread::sleep(Duration::from_millis(150));
    shared.target.store(1, Ordering::SeqCst);
    let lua = format!(
        "hl.dsp.send_key_state({{ mods = \"\", key = \"code:56\", state = \"up\", window = \"address:{}\" }})",
        window_a.address
    );
    let _ = hypr(&lua);
    shared.push("inject evdev=48 state=up".to_string());

    let after = read_locks();
    shared.push(format!("locks-after {after:?}"));
    restore_locks(&locks, &after, &window_a.address, &shared);
    let restored = read_locks();
    shared.push(format!("locks-restored {restored:?}"));
    shared.target.store(0, Ordering::SeqCst);
    shared.done.store(true, Ordering::SeqCst);
}

struct SeatWindow {
    address: String,
    title: String,
    floating: bool,
}

fn find_windows() -> Result<Vec<SeatWindow>, String> {
    let pid = u64::from(std::process::id());
    let mut found = Vec::new();
    for _ in 0..50 {
        let output = Command::new("hyprctl")
            .args(["clients", "-j"])
            .output()
            .map_err(|error| format!("hyprctl clients: {error}"))?;
        let text = String::from_utf8_lossy(&output.stdout);
        let value: serde_json::Value =
            serde_json::from_str(&text).map_err(|error| format!("clients json: {error}"))?;
        found.clear();
        for client in value.as_array().into_iter().flatten() {
            if client.get("pid").and_then(|pid| pid.as_u64()) != Some(pid) {
                continue;
            }
            let title = client
                .get("title")
                .and_then(|title| title.as_str())
                .unwrap_or("")
                .to_string();
            let initial = client
                .get("initialTitle")
                .and_then(|title| title.as_str())
                .unwrap_or("");
            if title != TITLE_A && title != TITLE_B && initial != TITLE_A && initial != TITLE_B {
                continue;
            }
            let address = client
                .get("address")
                .and_then(|address| address.as_str())
                .unwrap_or("")
                .to_string();
            if address.is_empty() {
                continue;
            }
            let name = if title == TITLE_A || title == TITLE_B {
                title
            } else {
                initial.to_string()
            };
            found.push(SeatWindow {
                address,
                title: name,
                floating: client
                    .get("floating")
                    .and_then(|floating| floating.as_bool())
                    .unwrap_or(false),
            });
        }
        if found.iter().any(|window| window.title == TITLE_A)
            && found.iter().any(|window| window.title == TITLE_B)
        {
            return Ok(found);
        }
        thread::sleep(Duration::from_millis(100));
    }
    Err("the probe windows did not appear within 5s".to_string())
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
    let lowered = text.to_ascii_lowercase();
    if !output.status.success() || lowered.contains("error:") || lowered.contains("not found") {
        return Err(text.trim().to_string());
    }
    Ok(text)
}

fn read_locks() -> Vec<(String, bool, bool)> {
    let Ok(output) = Command::new("hyprctl").args(["devices", "-j"]).output() else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&output.stdout);
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    value
        .get("keyboards")
        .and_then(|keyboards| keyboards.as_array())
        .into_iter()
        .flatten()
        .filter_map(|keyboard| {
            Some((
                keyboard.get("name")?.as_str()?.to_string(),
                keyboard.get("capsLock")?.as_bool()?,
                keyboard.get("numLock")?.as_bool()?,
            ))
        })
        .collect()
}

fn restore_locks(
    before: &[(String, bool, bool)],
    after: &[(String, bool, bool)],
    address: &str,
    shared: &Shared,
) {
    let caps_changed = lock_bit(before, true) != lock_bit(after, true);
    let num_changed = lock_bit(before, false) != lock_bit(after, false);
    if caps_changed {
        let _ = tap(58, address);
        shared.push("restored caps lock".to_string());
    }
    if num_changed {
        let _ = tap(69, address);
        shared.push("restored num lock".to_string());
    }
}

fn lock_bit(locks: &[(String, bool, bool)], caps: bool) -> bool {
    locks.iter().any(
        |(_, caps_lock, num_lock)| {
            if caps { *caps_lock } else { *num_lock }
        },
    )
}

fn tap(evdev: u32, address: &str) -> Result<(), String> {
    let code = evdev + 8;
    hypr(&format!(
        "hl.dsp.send_key_state({{ mods = \"\", key = \"code:{code}\", state = \"down\", window = \"address:{address}\" }})"
    ))?;
    thread::sleep(Duration::from_millis(30));
    hypr(&format!(
        "hl.dsp.send_key_state({{ mods = \"\", key = \"code:{code}\", state = \"up\", window = \"address:{address}\" }})"
    ))?;
    Ok(())
}
