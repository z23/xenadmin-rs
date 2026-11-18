//! Where the keys of a console come from.
//!
//! Decision 0006 binds the console to physical key codes (route A): GPUI
//! turns a Wayland key into a `Keystroke` and drops the evdev code, and it
//! reports no modifier key as a key. So on Wayland a second `wl_keyboard`
//! ([`ForeignKeyboard`]) listens on the display GPUI's windows use, and
//! every key it reports goes to the canvas that has the keyboard, by its
//! evdev code.
//!
//! - [`attach`] makes the listener from the first window that has a Wayland
//!   display. It is tried once for the application. Any error means "no
//!   listener": the test platform, a headless render, X11.
//! - The listener does not read the display's socket (GPUI does). A timer
//!   drains its queue: every [`FAST`] while a canvas has the keyboard, every
//!   [`IDLE`] otherwise (a frame callback stops once the window is idle,
//!   report 2026-11-14).
//! - While the listener has its keyboard ([`seat_bound`]) the keystrokes
//!   GPUI makes of the same keys are stopped and not forwarded: one physical
//!   event never takes both paths.
//!
//! Without a listener the canvas forwards GPUI's keystroke (the path the
//! seam calls experimental, plan decision D7): the key's name gives an evdev
//! code by the US layout ([`evdev_code`]), the modifier keys come from the
//! modifier state ([`modifier_changes`], always as the left key), and a
//! character the table does not know goes as a keysym ([`keysym`]).
//!
//! NOT VERIFIED: nothing here ran on a Wayland seat. The listener itself was
//! shown on Hyprland 0.56.2 by the spike; its use by a console canvas (the
//! pump, the routing to the focused canvas, the stop of GPUI's keystrokes)
//! compiles and its pure parts are unit tested, no more.

use std::time::Duration;

use xenadmin_ui::areas::console::canvas as model;

use super::canvas;
use crate::kit::*;
use crate::wayland_keys::{ForeignKeyboard, SeatEvent};

/// How often the listener's queue is drained while a canvas has the
/// keyboard.
const FAST: Duration = Duration::from_millis(8);
/// How often it is drained otherwise: only to keep the queue short and the
/// modifier state current.
const IDLE: Duration = Duration::from_millis(250);

pub const LEFT_CTRL: u32 = 29;
pub const LEFT_SHIFT: u32 = 42;
pub const LEFT_ALT: u32 = 56;
pub const LEFT_META: u32 = 125;

/// The evdev code of the key GPUI names `key`, on the US layout. A shifted
/// character is the key that carries it (`+` is the `=` key): Shift goes to
/// the guest as a key of its own.
pub fn evdev_code(key: &str) -> Option<u32> {
    const ROWS: [(&str, &str, u32); 4] = [
        ("`1234567890-=", "~!@#$%^&*()_+", 0),
        ("qwertyuiop[]", "QWERTYUIOP{}", 16),
        ("asdfghjkl;'", "ASDFGHJKL:\"", 30),
        ("zxcvbnm,./", "ZXCVBNM<>?", 44),
    ];
    let mut characters = key.chars();
    if let (Some(character), None) = (characters.next(), characters.next()) {
        for (plain, shifted, first) in ROWS {
            for row in [plain, shifted] {
                if let Some(index) = row.chars().position(|c| c == character) {
                    let index = index as u32;
                    return Some(match first {
                        // The grave accent is KEY_GRAVE; the digits follow
                        // Escape.
                        0 if index == 0 => 41,
                        0 => index + 1,
                        first => first + index,
                    });
                }
            }
        }
        return match character {
            '\\' | '|' => Some(43),
            ' ' => Some(57),
            _ => None,
        };
    }
    let code = match key.to_ascii_lowercase().as_str() {
        "escape" => 1,
        "backspace" => 14,
        "tab" => 15,
        "enter" | "return" => 28,
        "space" => 57,
        "capslock" => 58,
        "f1" => 59,
        "f2" => 60,
        "f3" => 61,
        "f4" => 62,
        "f5" => 63,
        "f6" => 64,
        "f7" => 65,
        "f8" => 66,
        "f9" => 67,
        "f10" => 68,
        "numlock" => 69,
        "scrolllock" => 70,
        "f11" => 87,
        "f12" => 88,
        "print" | "printscreen" => 99,
        "home" => 102,
        "up" => 103,
        "pageup" => 104,
        "left" => 105,
        "right" => 106,
        "end" => 107,
        "down" => 108,
        "pagedown" => 109,
        "insert" => 110,
        "delete" => 111,
        "pause" => 119,
        "menu" => 127,
        "control" => LEFT_CTRL,
        "shift" => LEFT_SHIFT,
        "alt" => LEFT_ALT,
        "platform" | "super" | "cmd" => LEFT_META,
        _ => return None,
    };
    Some(code)
}

/// The X keysym of a key GPUI names by one character that [`evdev_code`]
/// does not know: a letter of another layout. Latin-1 is its own keysym;
/// anything else is the Unicode keysym.
pub fn keysym(key: &str) -> Option<u32> {
    let mut characters = key.chars();
    let (Some(character), None) = (characters.next(), characters.next()) else {
        return None;
    };
    let point = u32::from(character);
    match point {
        0x20..=0x7e | 0xa0..=0xff => Some(point),
        0x100..=0x10_ffff => Some(0x0100_0000 + point),
        _ => None,
    }
}

/// The modifier keys that went down or up between two modifier states of
/// GPUI, as evdev codes of the left keys. GPUI does not say which side.
pub fn modifier_changes(before: Modifiers, after: Modifiers) -> Vec<(u32, bool)> {
    [
        (before.control, after.control, LEFT_CTRL),
        (before.shift, after.shift, LEFT_SHIFT),
        (before.alt, after.alt, LEFT_ALT),
        (before.platform, after.platform, LEFT_META),
    ]
    .into_iter()
    .filter(|(before, after, _)| before != after)
    .map(|(_, down, code)| (code, down))
    .collect()
}

/// What the model wants to know of a `wl_keyboard` modifiers event: Shift
/// held or latched, and the two locks. The bits are those of the core
/// modifiers every XKB keymap of a desktop starts with (Shift, Lock,
/// Control, Mod1, Mod2 = Num Lock).
pub fn seat_modifiers(depressed: u32, latched: u32, locked: u32) -> model::Modifiers {
    model::Modifiers {
        shift: (depressed | latched) & 0x01 != 0,
        caps_lock: locked & 0x02 != 0,
        num_lock: locked & 0x10 != 0,
    }
}

/// The listener of the application.
struct SeatKeys {
    /// `None`: there is no listener (see the module).
    keyboard: Option<ForeignKeyboard>,
    /// What the last modifiers event said.
    modifiers: model::Modifiers,
    _pump: Option<Task<()>>,
}

impl Global for SeatKeys {}

/// Makes the application's listener from `window`, once. Called when a
/// canvas is made.
pub(crate) fn attach(window: &Window, cx: &mut App) {
    if cx.try_global::<SeatKeys>().is_some() {
        return;
    }
    let keyboard = ForeignKeyboard::from_gpui_display(window)
        .and_then(|keyboard| keyboard.flush().map(|()| keyboard))
        .ok();
    let listening = keyboard.is_some();
    cx.set_global(SeatKeys {
        keyboard,
        modifiers: model::Modifiers::default(),
        _pump: None,
    });
    if listening {
        pump(cx);
    }
}

/// Whether the listener has its keyboard: keys arrive by evdev code, and
/// GPUI's keystrokes are not forwarded.
pub(crate) fn seat_bound(cx: &App) -> bool {
    cx.try_global::<SeatKeys>().is_some_and(|seat| {
        seat.keyboard
            .as_ref()
            .is_some_and(ForeignKeyboard::keyboard_bound)
    })
}

/// A canvas took the keyboard: what the listener queued before that was
/// typed somewhere else, and the queue is drained quickly from now on.
pub(crate) fn focus_gained(cx: &mut App) {
    if cx
        .try_global::<SeatKeys>()
        .is_none_or(|seat| seat.keyboard.is_none())
    {
        return;
    }
    for event in take_events(cx) {
        if let SeatEvent::Modifiers {
            depressed,
            latched,
            locked,
            ..
        } = event
        {
            cx.global_mut::<SeatKeys>().modifiers = seat_modifiers(depressed, latched, locked);
        }
    }
    pump(cx);
}

/// (Re)starts the task that drains the queue.
fn pump(cx: &mut App) {
    let task = cx.spawn(async move |cx| {
        loop {
            let fast = cx.update(drain);
            let wait = if fast { FAST } else { IDLE };
            cx.background_executor().timer(wait).await;
        }
    });
    cx.global_mut::<SeatKeys>()._pump = Some(task);
}

/// What the listener has queued. A listener that fails is given up: the
/// keystroke path takes over.
fn take_events(cx: &mut App) -> Vec<SeatEvent> {
    let seat = cx.global_mut::<SeatKeys>();
    let Some(keyboard) = &mut seat.keyboard else {
        return Vec::new();
    };
    // Binding the seat and its keyboard are requests of this queue: they
    // leave with the flush.
    let events = keyboard
        .dispatch_seat()
        .and_then(|events| keyboard.flush().map(|()| events));
    match events {
        Ok(events) => events,
        Err(error) => {
            eprintln!("The console key listener stopped: {error}");
            seat.keyboard = None;
            Vec::new()
        }
    }
}

/// Delivers what the listener queued. Returns whether a canvas has the
/// keyboard.
fn drain(cx: &mut App) -> bool {
    for event in take_events(cx) {
        match event {
            SeatEvent::Enter { .. } => {}
            // The compositor took the keyboard away: nothing stays down.
            SeatEvent::Leave { .. } => {
                if let Some(target) = canvas::with_keyboard(cx) {
                    target.update(cx, |canvas, cx| canvas.release(cx));
                }
            }
            SeatEvent::Key { evdev, down } => {
                let modifiers = cx.global::<SeatKeys>().modifiers;
                if let Some(target) = canvas::with_keyboard(cx) {
                    target.update(cx, |canvas, cx| canvas.send_code(evdev, down, modifiers, cx));
                }
            }
            SeatEvent::Modifiers {
                depressed,
                latched,
                locked,
                ..
            } => {
                cx.global_mut::<SeatKeys>().modifiers = seat_modifiers(depressed, latched, locked);
            }
        }
    }
    canvas::with_keyboard(cx).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_names_give_the_evdev_codes_of_the_us_layout() {
        for (key, code) in [
            ("a", 30),
            ("A", 30),
            ("q", 16),
            ("p", 25),
            ("[", 26),
            ("}", 27),
            ("l", 38),
            (";", 39),
            ("'", 40),
            ("z", 44),
            ("m", 50),
            ("/", 53),
            ("?", 53),
            ("`", 41),
            ("~", 41),
            ("1", 2),
            ("!", 2),
            ("0", 11),
            ("-", 12),
            ("=", 13),
            ("+", 13),
            ("\\", 43),
            (" ", 57),
            ("space", 57),
            ("escape", 1),
            ("tab", 15),
            ("enter", 28),
            ("backspace", 14),
            ("left", 105),
            ("delete", 111),
            ("f1", 59),
            ("f10", 68),
            ("f12", 88),
        ] {
            assert_eq!(evdev_code(key), Some(code), "{key}");
            // Every code the table gives is one the model can send.
            assert!(model::scancode(code).is_some(), "{key}: {code}");
        }
        assert_eq!(evdev_code("é"), None);
        assert_eq!(evdev_code("f13"), None);
    }

    #[test]
    fn a_character_of_another_layout_goes_as_its_keysym() {
        assert_eq!(keysym("é"), Some(0xe9));
        assert_eq!(keysym("€"), Some(0x0100_20ac));
        assert_eq!(keysym("escape"), None);
        assert_eq!(keysym(""), None);
    }

    #[test]
    fn modifier_keys_follow_the_modifier_state() {
        let none = Modifiers::default();
        let ctrl_shift = Modifiers {
            control: true,
            shift: true,
            ..Modifiers::default()
        };
        assert_eq!(
            modifier_changes(none, ctrl_shift),
            vec![(LEFT_CTRL, true), (LEFT_SHIFT, true)]
        );
        assert_eq!(
            modifier_changes(ctrl_shift, Modifiers::control()),
            vec![(LEFT_SHIFT, false)]
        );
        assert!(modifier_changes(ctrl_shift, ctrl_shift).is_empty());
    }

    #[test]
    fn the_listener_s_modifier_masks_are_shift_and_the_two_locks() {
        assert_eq!(seat_modifiers(0, 0, 0), model::Modifiers::default());
        assert!(seat_modifiers(1, 0, 0).shift);
        assert!(seat_modifiers(0, 1, 0).shift);
        // Control and Alt are keys of their own, not state the model reads.
        assert_eq!(seat_modifiers(0x0c, 0, 0), model::Modifiers::default());
        let locks = seat_modifiers(0, 0, 0x12);
        assert!(locks.caps_lock && locks.num_lock && !locks.shift);
    }
}
