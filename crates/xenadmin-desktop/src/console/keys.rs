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
//! - `wl_keyboard` reports a held key once, and the console of a guest does
//!   not repeat keys by itself (QEMU's keyboard has no typematic repeat: the
//!   viewer sends the repeats). So the pump repeats the key held last
//!   ([`Repeater`]), at the rate and after the delay the compositor's
//!   `repeat_info` gives. In the driven session of 2026-11-19 a key held for
//!   two seconds arrived in the guest once, before this was added
//!   (`docs/reports/2026-11-19-gpui-vm-session.md`).
//! - What a drain gives the console is decided apart from the toolkit
//!   ([`SeatState::drain`]): a repeat is sent only while the listener lives
//!   and a canvas has the keyboard. A listener that fails is given up, and
//!   with it the repeat of a key that was held: its release would arrive
//!   on the keystroke path, which the repeat does not hear.
//!
//! Without a listener the canvas forwards GPUI's keystroke (the path the
//! seam calls experimental, plan decision D7): the key's name gives an evdev
//! code by the US layout ([`evdev_code`]), the modifier keys come from the
//! modifier state ([`modifier_changes`], always as the left key), and a
//! character the table does not know goes as a keysym ([`keysym`]).
//!
//! VERIFIED ONCE, in a virtual machine with injected input (sway 1.12, a
//! virtual seat, 2026-11-19, `docs/reports/2026-11-19-gpui-vm-session.md`):
//! the pump, the routing to the focused canvas, the release when the
//! keyboard leaves and the repeats, by what a lab guest recorded. Not on a
//! person's seat, and on no other compositor; the listener itself was shown
//! on Hyprland 0.56.2 by the spike.
//!
//! NOT RUN SINCE: on 2026-11-20 the drain was rewritten around
//! [`SeatState`] so that the ends of a repeat can be tested, and a flush
//! that would block stopped ending the listener. The decisions are held by
//! this module's tests; the rewritten pump has run on no compositor.

use std::time::{Duration, Instant};

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

/// The repeats of a held key, for the listener's path.
///
/// One key repeats: the one that went down last. A modifier or a lock key
/// neither repeats nor stops the repeat of another key. Everything stops
/// when the key goes up, when another key goes down, and when the keyboard
/// leaves ([`Repeater::clear`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repeater {
    /// Repeats a second. 0: the compositor asks for none.
    rate: u32,
    /// How long a key is held before its first repeat.
    delay: Duration,
    /// The key that repeats and when its next repeat is due.
    held: Option<(u32, Instant)>,
}

impl Default for Repeater {
    /// What compositors commonly announce, until this one has spoken.
    fn default() -> Self {
        Self {
            rate: 25,
            delay: Duration::from_millis(600),
            held: None,
        }
    }
}

impl Repeater {
    /// `wl_keyboard.repeat_info`. A negative value is taken as none.
    pub fn set_info(&mut self, rate: i32, delay: i32) {
        self.rate = u32::try_from(rate).unwrap_or(0);
        self.delay = Duration::from_millis(u64::try_from(delay).unwrap_or(0));
        if self.rate == 0 {
            self.held = None;
        }
    }

    /// A key of the seat went down or up at `now`.
    pub fn key(&mut self, code: u32, down: bool, now: Instant) {
        if !repeats(code) {
            return;
        }
        if down {
            self.held = (self.rate > 0).then(|| (code, now + self.delay));
        } else if self.held.is_some_and(|(held, _)| held == code) {
            self.held = None;
        }
    }

    /// Nothing is held any more.
    pub fn clear(&mut self) {
        self.held = None;
    }

    /// The key to press again, if a repeat is due at `now`. At most one a
    /// call: a late call does not make up for the repeats it missed.
    pub fn due(&mut self, now: Instant) -> Option<u32> {
        let (code, next) = self.held?;
        if now < next || self.rate == 0 {
            return None;
        }
        let period = Duration::from_secs(1) / self.rate;
        // On time: keep the rhythm. Late: start it again from now.
        let following = next + period;
        let following = if following > now {
            following
        } else {
            now + period
        };
        self.held = Some((code, following));
        Some(code)
    }
}

/// Whether a held key repeats: every key but the modifiers and the locks.
fn repeats(code: u32) -> bool {
    !matches!(
        code,
        29 | 42 | 54 | 56 | 58 | 69 | 70 | 97 | 100 | 125 | 126
    )
}

/// What one drain gives the console that has the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    /// The compositor took the keyboard away: nothing stays down, and the
    /// console has the keyboard no longer.
    Release,
    /// A key by its evdev code, with the modifier state it was pressed in.
    Key {
        code: u32,
        down: bool,
        modifiers: model::Modifiers,
    },
}

/// What the listener's path remembers between two drains.
#[derive(Debug, Default)]
pub struct SeatState {
    /// What the last modifiers event said.
    modifiers: model::Modifiers,
    /// The repeats of the key held in a console.
    repeater: Repeater,
}

impl SeatState {
    /// One drain of the queue at `now`. `events` is what the listener
    /// queued, `listening` whether the listener is still alive after it
    /// was asked, `console` whether a canvas has the keyboard. Returns what
    /// that canvas is given, in order.
    ///
    /// A key repeats only while the listener lives and a console has the
    /// keyboard. Every other state ends the repeat: who would report the
    /// key's release is gone, or who would receive the repeat is.
    pub fn drain(
        &mut self,
        events: Vec<SeatEvent>,
        listening: bool,
        mut console: bool,
        now: Instant,
    ) -> Vec<Delivery> {
        let mut out = Vec::new();
        for event in events {
            match event {
                SeatEvent::Enter { .. } => {}
                SeatEvent::Leave { .. } => {
                    self.repeater.clear();
                    if console {
                        out.push(Delivery::Release);
                        console = false;
                    }
                }
                SeatEvent::Key { evdev, down } => {
                    if console {
                        self.repeater.key(evdev, down, now);
                        out.push(Delivery::Key {
                            code: evdev,
                            down,
                            modifiers: self.modifiers,
                        });
                    }
                }
                other => self.settings(other),
            }
        }
        if !(listening && console) {
            self.repeater.clear();
        } else if let Some(code) = self.repeater.due(now) {
            out.push(Delivery::Key {
                code,
                down: true,
                modifiers: self.modifiers,
            });
        }
        out
    }

    /// A canvas took the keyboard: the keys in `events` were typed
    /// somewhere else and are dropped; what they say about the modifiers
    /// and the repeat is kept. Nothing is held.
    pub fn focus_gained(&mut self, events: Vec<SeatEvent>) {
        for event in events {
            self.settings(event);
        }
        self.repeater.clear();
    }

    /// The events that are state and no key.
    fn settings(&mut self, event: SeatEvent) {
        match event {
            SeatEvent::Modifiers {
                depressed,
                latched,
                locked,
                ..
            } => self.modifiers = seat_modifiers(depressed, latched, locked),
            SeatEvent::RepeatInfo { rate, delay } => self.repeater.set_info(rate, delay),
            _ => {}
        }
    }
}

/// The listener of the application.
struct SeatKeys {
    /// `None`: there is no listener (see the module).
    keyboard: Option<ForeignKeyboard>,
    state: SeatState,
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
        state: SeatState::default(),
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
    let events = take_events(cx);
    cx.global_mut::<SeatKeys>().state.focus_gained(events);
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
            // Nobody reports the release of a held key from here on.
            seat.state.repeater.clear();
            Vec::new()
        }
    }
}

/// Delivers what the listener queued ([`SeatState::drain`]). Returns
/// whether a canvas has the keyboard and the listener lives: whether the
/// queue is to be drained quickly.
fn drain(cx: &mut App) -> bool {
    let events = take_events(cx);
    let target = canvas::with_keyboard(cx);
    let seat = cx.global_mut::<SeatKeys>();
    let listening = seat.keyboard.is_some();
    let deliveries = seat
        .state
        .drain(events, listening, target.is_some(), Instant::now());
    let Some(target) = target else {
        return false;
    };
    for delivery in deliveries {
        target.update(cx, |canvas, cx| match delivery {
            Delivery::Release => canvas.release(cx),
            Delivery::Key {
                code,
                down,
                modifiers,
            } => canvas.send_code(code, down, modifiers, cx),
        });
    }
    listening && target.read(cx).has_keyboard()
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
    fn a_held_key_repeats_after_the_delay_at_the_rate() {
        let start = Instant::now();
        let ms = |n: u64| start + Duration::from_millis(n);
        let mut repeater = Repeater::default();
        repeater.set_info(20, 500);
        repeater.key(30, true, ms(0));
        assert_eq!(repeater.due(ms(499)), None, "not before the delay");
        assert_eq!(repeater.due(ms(500)), Some(30));
        assert_eq!(repeater.due(ms(520)), None, "one period is 50 ms");
        assert_eq!(repeater.due(ms(550)), Some(30));
        // A late call gives one repeat and does not make up for the rest.
        assert_eq!(repeater.due(ms(1000)), Some(30));
        assert_eq!(repeater.due(ms(1001)), None);
        assert_eq!(repeater.due(ms(1050)), Some(30));
        repeater.key(30, false, ms(1010));
        assert_eq!(repeater.due(ms(2000)), None, "released");
    }

    #[test]
    fn only_the_key_pressed_last_repeats_and_modifiers_never() {
        let start = Instant::now();
        let ms = |n: u64| start + Duration::from_millis(n);
        let mut repeater = Repeater::default();
        repeater.set_info(25, 600);
        repeater.key(LEFT_SHIFT, true, ms(0));
        assert_eq!(repeater.due(ms(5000)), None, "a modifier does not repeat");
        repeater.key(30, true, ms(5000));
        repeater.key(48, true, ms(5100));
        assert_eq!(repeater.due(ms(5650)), None, "the delay starts again");
        assert_eq!(repeater.due(ms(5700)), Some(48));
        // The first key going up, or a modifier, leaves the repeat alone.
        repeater.key(30, false, ms(5710));
        repeater.key(LEFT_SHIFT, false, ms(5720));
        assert_eq!(repeater.due(ms(5740)), Some(48));
        repeater.clear();
        assert_eq!(repeater.due(ms(9000)), None, "the keyboard left");
    }

    #[test]
    fn a_compositor_can_ask_for_no_repeat() {
        let start = Instant::now();
        let mut repeater = Repeater::default();
        repeater.key(30, true, start);
        repeater.set_info(0, 600);
        assert_eq!(repeater.due(start + Duration::from_secs(5)), None);
        repeater.key(31, true, start);
        assert_eq!(repeater.due(start + Duration::from_secs(5)), None);
        repeater.set_info(-1, -1);
        assert_eq!(repeater.due(start + Duration::from_secs(9)), None);
    }

    fn key(code: u32, down: bool) -> Delivery {
        Delivery::Key {
            code,
            down,
            modifiers: model::Modifiers::default(),
        }
    }

    /// A state in which the key `a` has been down for a second in a
    /// console, at the default rate, and `start`.
    fn held() -> (SeatState, Instant) {
        let start = Instant::now();
        let mut seat = SeatState::default();
        let down = vec![SeatEvent::Key {
            evdev: 30,
            down: true,
        }];
        assert_eq!(seat.drain(down, true, true, start), [key(30, true)]);
        (seat, start + Duration::from_secs(1))
    }

    #[test]
    fn a_held_key_repeats_into_the_console_that_has_the_keyboard() {
        let (mut seat, later) = held();
        assert_eq!(seat.drain(Vec::new(), true, true, later), [key(30, true)]);
        let up = vec![SeatEvent::Key {
            evdev: 30,
            down: false,
        }];
        assert_eq!(seat.drain(up, true, true, later), [key(30, false)]);
        let much_later = later + Duration::from_secs(5);
        assert_eq!(seat.drain(Vec::new(), true, true, much_later), []);
    }

    #[test]
    fn a_listener_that_failed_ends_the_repeat_of_the_held_key() {
        // The release of the key will not arrive through the listener.
        let (mut seat, later) = held();
        assert_eq!(seat.drain(Vec::new(), false, true, later), []);
        // And the key is not remembered for a listener that is not there.
        let much_later = later + Duration::from_secs(5);
        assert_eq!(seat.drain(Vec::new(), false, true, much_later), []);
        assert_eq!(seat.drain(Vec::new(), true, true, much_later), []);
    }

    #[test]
    fn no_console_with_the_keyboard_ends_the_repeat() {
        // Blur, another window, Release Input, a closed console.
        let (mut seat, later) = held();
        assert_eq!(seat.drain(Vec::new(), true, false, later), []);
        let back = later + Duration::from_secs(5);
        assert_eq!(seat.drain(Vec::new(), true, true, back), []);
        // A key typed while no console has the keyboard goes nowhere and
        // does not repeat later.
        let elsewhere = vec![SeatEvent::Key {
            evdev: 31,
            down: true,
        }];
        assert_eq!(seat.drain(elsewhere, true, false, back), []);
        let after = back + Duration::from_secs(5);
        assert_eq!(seat.drain(Vec::new(), true, true, after), []);
    }

    #[test]
    fn the_keyboard_leaving_releases_the_console_and_ends_the_repeat() {
        let (mut seat, later) = held();
        let leave = vec![
            SeatEvent::Leave {
                surface: 1,
                protocol_id: 1,
            },
            // What follows a leave in the same batch is not the console's.
            SeatEvent::Key {
                evdev: 31,
                down: true,
            },
        ];
        assert_eq!(seat.drain(leave, true, true, later), [Delivery::Release]);
        let much_later = later + Duration::from_secs(5);
        assert_eq!(seat.drain(Vec::new(), true, true, much_later), []);
    }

    #[test]
    fn gaining_the_focus_keeps_the_state_and_drops_the_keys() {
        let (mut seat, later) = held();
        seat.focus_gained(vec![
            SeatEvent::Key {
                evdev: 31,
                down: true,
            },
            SeatEvent::Modifiers {
                depressed: 1,
                latched: 0,
                locked: 0,
                group: 0,
            },
        ]);
        assert_eq!(seat.drain(Vec::new(), true, true, later), []);
        let down = vec![SeatEvent::Key {
            evdev: 32,
            down: true,
        }];
        let shifted = seat.drain(down, true, true, later);
        assert!(
            matches!(shifted[..], [Delivery::Key { code: 32, down: true, modifiers }] if modifiers.shift),
            "{shifted:?}"
        );
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
