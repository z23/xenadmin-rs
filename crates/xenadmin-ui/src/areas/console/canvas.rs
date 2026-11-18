//! The seam between the console model and the bespoke `console-canvas`
//! element of the front end. Plain Rust, no toolkit types.
//!
//! The canvas READS: [`frame_generation`], [`frame`], [`desktop_size`],
//! [`actual_size`], [`content_size`] and [`geometry`].
//!
//! The canvas CALLS, with its own [`CanvasInput`]: [`pointer_move`],
//! [`pointer_button`], [`wheel`], [`key`], [`keysym_key`], [`focus_lost`]
//! and [`text`].
//!
//! The canvas OWNS one [`CanvasInput`] per window: the canvas bounds, the
//! keys and buttons this window holds down in the guest, the wheel
//! remainders. Nothing of that is in the model, so two windows never share
//! it. Every function here first brings the input state in line with the
//! session ([`CanvasInput::sync`]): when the model released the guest's
//! input (a control was pressed, the console was reconnected or moved to
//! another window), what the window believes it holds is forgotten.
//!
//! Decision 0006 is binding for the keyboard: a key reaches the guest by its
//! evdev code, as the front end's second keyboard listener receives it.
//! [`key`] takes that code (`KEY_A` is 30; NOT the XKB keycode, which is
//! eight more), turns it into the XT make code the Rune console sent, and
//! sends `ConsoleInput::PhysicalKey`. The guest cursor is not drawn and the
//! local cursor stays visible, so nothing here knows a cursor; the "Hold
//! right button" control is gone because [`pointer_button`] takes the right
//! button's press and release as they happen.
//!
//! `/home/brendan/src/xenadmin-rs-wt/notes/progress/console-seam.md` says
//! who calls what and when.

use std::sync::Arc;

use xenadmin_core::ConsoleInput;

use super::session::{Seat, Session};
use crate::model::Model;

// ---------------------------------------------------------------------------
// Reading the picture
// ---------------------------------------------------------------------------

/// The picture to show: RGBA, four bytes a pixel, `width * height * 4`
/// bytes, shared with the console worker (no copy is made here).
#[derive(Clone)]
pub struct Frame {
    /// See [`frame_generation`].
    pub generation: u64,
    pub width: u16,
    pub height: u16,
    pub rgba: Arc<[u8]>,
}

/// The identity of the picture the canvas must show, cheap enough to ask
/// after every poll and on every paint.
///
/// - `0`: there is no picture. Drop the uploaded image and paint black.
/// - anything else: the canvas shows the right picture exactly when the
///   generation of its uploaded image equals this. Otherwise it drops the
///   old image and uploads [`frame`].
///
/// A generation is never reused in the process, not by another frame and
/// not by another session, so a console that was reopened, replaced or
/// moved to another window can never be mistaken for the one before.
pub fn frame_generation(m: &Model, seat: Seat) -> u64 {
    super::session(m, seat).map_or(0, |s| s.frame_generation)
}

/// The picture with its generation. `None` exactly when
/// [`frame_generation`] is `0`.
pub fn frame(m: &Model, seat: Seat) -> Option<Frame> {
    let s = super::session(m, seat)?;
    let frame = s.snapshot.as_ref()?.frame.as_ref()?;
    Some(Frame {
        generation: s.frame_generation,
        width: frame.width,
        height: frame.height,
        rgba: frame.rgba.clone(),
    })
}

/// The guest's desktop size in its own pixels: that of the latest frame.
pub fn desktop_size(m: &Model, seat: Seat) -> Option<[u16; 2]> {
    super::session(m, seat)?.desktop_size()
}

/// Fit (false) or Actual size (true).
pub fn actual_size(m: &Model, seat: Seat) -> bool {
    super::session(m, seat).is_some_and(Session::actual_size)
}

/// The size the canvas element takes.
///
/// - `None`: Fit. The canvas fills the space left to it (full width, the
///   remaining height) and letterboxes the picture inside itself.
/// - `Some([w, h])`: Actual size. The canvas is exactly as large as the
///   guest's desktop, in the canvas's own points, at the origin of a view
///   that scrolls on both axes. (In Rune a canvas that also asked for the
///   full width resolved to zero width inside the panning view; each mode
///   sets its own sizing.)
pub fn content_size(m: &Model, seat: Seat) -> Option<[f32; 2]> {
    let s = super::session(m, seat)?;
    if !s.actual_size {
        return None;
    }
    s.desktop_size()
        .map(|[width, height]| [f32::from(width), f32::from(height)])
}

/// Where the picture lies inside a canvas of `bounds` (width and height in
/// the canvas's own points): paint it at `offset` with each guest pixel
/// `scale` points large. `None` when there is no picture or no room.
pub fn geometry(m: &Model, seat: Seat, bounds: [f32; 2]) -> Option<Geometry> {
    geometry_of(super::session(m, seat)?, bounds)
}

fn geometry_of(s: &Session, bounds: [f32; 2]) -> Option<Geometry> {
    let [width, height] = s.desktop_size()?;
    Geometry::new(bounds, width, height, s.actual_size)
}

/// How a frame of `width` by `height` guest pixels lies in the canvas: Fit
/// letterboxes it (one scale for both axes, centred); Actual size shows it
/// one to one at the origin. Carried over from Rune.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geometry {
    pub scale: f32,
    pub offset: [f32; 2],
    pub width: u16,
    pub height: u16,
}

impl Geometry {
    pub fn new(bounds: [f32; 2], width: u16, height: u16, actual: bool) -> Option<Self> {
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

    /// A point of the canvas (its own points, origin at its top left) as a
    /// guest pixel. A point in the letterbox is the nearest pixel.
    pub fn position(self, x: f32, y: f32) -> Option<[u16; 2]> {
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        Some([
            ((x - self.offset[0]) / self.scale).clamp(0.0, f32::from(self.width - 1)) as u16,
            ((y - self.offset[1]) / self.scale).clamp(0.0, f32::from(self.height - 1)) as u16,
        ])
    }

    /// The size of the painted picture in canvas points.
    pub fn painted(self) -> [f32; 2] {
        [
            f32::from(self.width) * self.scale,
            f32::from(self.height) * self.scale,
        ]
    }
}

// ---------------------------------------------------------------------------
// The input state of one window
// ---------------------------------------------------------------------------

/// A pointer button, with its bit in the RFB button mask.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerButton {
    Left,
    Middle,
    Right,
}

impl PointerButton {
    pub fn mask(self) -> u8 {
        match self {
            Self::Left => 1,
            Self::Middle => 2,
            Self::Right => 4,
        }
    }
}

/// The modifier state the keyboard listener reports with a key (the
/// `wl_keyboard` modifiers event): Shift held, and the two locks. It is
/// used only to choose a keysym for a guest that did not negotiate key
/// codes; Control, Alt and Super reach the guest as keys of their own.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub shift: bool,
    pub caps_lock: bool,
    pub num_lock: bool,
}

/// What one window holds for its console. The view owns it; the model
/// never stores it.
#[derive(Debug, Default)]
pub struct CanvasInput {
    /// The canvas's width and height in its own points. The view sets it
    /// whenever the canvas is laid out ([`CanvasInput::set_bounds`]).
    bounds: [f32; 2],
    /// The RFB button mask this window holds (1 left, 2 middle, 4 right).
    buttons: u8,
    /// Held keys: the evdev code and the keysym sent at the press.
    keys: Vec<(u32, u32)>,
    /// Precise-scroll remainders, x and y.
    wheel: [f32; 2],
    /// Where the pointer last was over the canvas, in its own points.
    pointer: [f32; 2],
    /// The session's input epoch this state belongs to.
    epoch: (u64, u64),
}

impl CanvasInput {
    pub fn new() -> Self {
        Self::default()
    }

    /// The canvas was laid out: its width and height in its own points.
    pub fn set_bounds(&mut self, width: f32, height: f32) {
        self.bounds = [width, height];
    }

    pub fn bounds(&self) -> [f32; 2] {
        self.bounds
    }

    /// The button mask this window holds in the guest.
    pub fn buttons(&self) -> u8 {
        self.buttons
    }

    /// The evdev codes of the keys this window holds in the guest.
    pub fn held_keys(&self) -> Vec<u32> {
        self.keys.iter().map(|(code, _)| *code).collect()
    }

    /// Where the pointer last was over the canvas, in its own points.
    pub fn pointer(&self) -> [f32; 2] {
        self.pointer
    }

    /// Forgets every held key and button and the wheel remainders. It sends
    /// nothing: the model has released them in the guest already, or is
    /// about to ([`focus_lost`]).
    fn forget(&mut self) {
        self.buttons = 0;
        self.keys.clear();
        self.wheel = [0.0; 2];
    }

    /// Brings the state in line with the session: when the session released
    /// its input or became another stream since this state last saw it,
    /// nothing is held any more.
    fn sync(&mut self, s: &Session) {
        let epoch = s.input_epoch();
        if self.epoch != epoch {
            self.forget();
            self.epoch = epoch;
        }
    }

    /// The pointer is at `(x, y)` with `buttons` down: the event for the
    /// guest, when the picture's geometry is known.
    fn pointer_event(
        &mut self,
        geometry: Option<Geometry>,
        x: f32,
        y: f32,
        buttons: u8,
    ) -> Option<ConsoleInput> {
        self.pointer = [x, y];
        let [x, y] = geometry.and_then(|g| g.position(x, y))?;
        Some(ConsoleInput::Pointer { x, y, buttons })
    }

    /// A key of the keyboard went down or up. `extended` is whether the
    /// guest's server negotiated key codes (`ConsoleSnapshot::extended_keys`).
    fn key_event(
        &mut self,
        extended: bool,
        code: u32,
        down: bool,
        repeat: bool,
        modifiers: Modifiers,
    ) -> Key {
        let Some(keycode) = scancode(code) else {
            return Key::Unknown;
        };
        // The release of a key carries the keysym its press carried, whatever
        // the modifiers are by then.
        let held = self
            .keys
            .iter()
            .find(|(held, _)| *held == code)
            .map(|(_, keysym)| *keysym);
        let keysym = match held.or_else(|| special_keysym(code)) {
            Some(keysym) => keysym,
            // With key codes negotiated the guest reads the code; Rune sent
            // no keysym with it.
            None if extended => 0,
            // Without them the guest gets a keysym, and the only layout the
            // model can know from a code is the US one.
            None => match us_keysym(code, modifiers) {
                Some(keysym) => keysym,
                None => return Key::Unknown,
            },
        };
        if !track_physical(&mut self.keys, code, keysym, down, repeat) {
            return Key::Dropped;
        }
        Key::Send(ConsoleInput::PhysicalKey {
            keysym,
            keycode,
            down,
        })
    }
}

/// What a key event comes to.
#[derive(Debug)]
enum Key {
    /// A code the console has no make code or keysym for.
    Unknown,
    /// A repeat of a key this window does not hold: nothing is sent.
    Dropped,
    Send(ConsoleInput),
}

// ---------------------------------------------------------------------------
// The functions the canvas calls
// ---------------------------------------------------------------------------

/// The pointer moved over the canvas to `(x, y)`, in the canvas's own
/// points with the origin at its top left corner.
pub fn pointer_move(m: &mut Model, seat: Seat, input: &mut CanvasInput, x: f32, y: f32) {
    let Some(s) = super::session_mut(m, seat) else {
        return;
    };
    input.sync(s);
    let buttons = input.buttons;
    if let Some(event) = input.pointer_event(geometry_of(s, input.bounds), x, y, buttons) {
        s.send(event);
    }
}

/// A pointer button went down or up at `(x, y)`. Left, middle and right are
/// all taken as they happen; a drag is a press, moves and a release.
pub fn pointer_button(
    m: &mut Model,
    seat: Seat,
    input: &mut CanvasInput,
    button: PointerButton,
    down: bool,
    x: f32,
    y: f32,
) {
    let Some(s) = super::session_mut(m, seat) else {
        return;
    };
    input.sync(s);
    if down {
        input.buttons |= button.mask();
    } else {
        input.buttons &= !button.mask();
    }
    let buttons = input.buttons;
    if let Some(event) = input.pointer_event(geometry_of(s, input.bounds), x, y, buttons) {
        s.send(event);
    }
}

/// The wheel turned by `delta` (x, y) at `position` (x, y). `precise` is a pixel delta
/// (a touchpad); otherwise the delta is in notches of 40. Returns `false`
/// when the canvas must let its panning view have the event instead: in
/// Actual size, Shift with the wheel pans.
pub fn wheel(
    m: &mut Model,
    seat: Seat,
    input: &mut CanvasInput,
    position: [f32; 2],
    delta: [f32; 2],
    precise: bool,
    shift: bool,
) -> bool {
    let [x, y] = position;
    let Some(s) = super::session_mut(m, seat) else {
        return false;
    };
    input.sync(s);
    if s.actual_size && shift {
        return false;
    }
    let geometry = geometry_of(s, input.bounds);
    for (axis, delta) in delta.into_iter().enumerate() {
        if !delta.is_finite() || delta == 0.0 {
            continue;
        }
        let steps = wheel_steps(&mut input.wheel[axis], delta, precise);
        let bit = match (axis, steps > 0) {
            (0, true) => 32,
            (0, false) => 64,
            (_, true) => 8,
            (_, false) => 16,
        };
        for _ in 0..steps.abs() {
            for buttons in [input.buttons | bit, input.buttons] {
                if let Some(event) = input.pointer_event(geometry, x, y, buttons) {
                    s.send(event);
                }
            }
        }
    }
    true
}

/// A key went down (`down`), repeated (`down` and `repeat`) or up, by its
/// evdev code, with the modifier state the listener last reported.
///
/// Returns whether the event was the console's (sent, or a repeat dropped
/// on purpose). The canvas keeps every key away from the application while
/// it has the keyboard, whatever this returns.
pub fn key(
    m: &mut Model,
    seat: Seat,
    input: &mut CanvasInput,
    code: u32,
    down: bool,
    repeat: bool,
    modifiers: Modifiers,
) -> bool {
    let Some(s) = super::session_mut(m, seat) else {
        return false;
    };
    input.sync(s);
    if s.console.is_none() {
        return false;
    }
    let extended = s.snapshot.as_ref().is_some_and(|x| x.extended_keys);
    match input.key_event(extended, code, down, repeat, modifiers) {
        Key::Unknown => false,
        Key::Dropped => true,
        Key::Send(event) => {
            s.send(event);
            true
        }
    }
}

/// A key by keysym alone, for a platform without the evdev listener (X11,
/// which stays experimental: plan decision D7). Not held-key tracked: the
/// console worker pairs a keysym's release with its press.
pub fn keysym_key(
    m: &mut Model,
    seat: Seat,
    input: &mut CanvasInput,
    keysym: u32,
    down: bool,
) -> bool {
    let Some(s) = super::session_mut(m, seat) else {
        return false;
    };
    input.sync(s);
    if s.console.is_none() {
        return false;
    }
    s.send(ConsoleInput::Key { keysym, down });
    true
}

/// The canvas no longer has the keyboard, for whatever reason: the window
/// was deactivated, another element or a dialog took the focus, the tab was
/// left, the canvas is being removed. Every held key and button is released
/// in the guest and forgotten here.
pub fn focus_lost(m: &mut Model, seat: Seat, input: &mut CanvasInput) {
    input.forget();
    if let Some(s) = super::session_mut(m, seat) {
        s.release();
        input.epoch = s.input_epoch();
    }
}

/// Text that arrived as text and not as keys: a paste into the canvas, or
/// what an input method committed. It is never typed straight away: it goes
/// to the same review as "Type clipboard as keys", which shows how much
/// there is and never the text, and types it only when confirmed.
pub fn text(m: &mut Model, seat: Seat, input: &mut CanvasInput, text: &str) {
    let Some(s) = super::session_mut(m, seat) else {
        return;
    };
    input.forget();
    s.release();
    input.epoch = s.input_epoch();
    s.arm_typing(text);
}

// ---------------------------------------------------------------------------
// Carried over from Rune
// ---------------------------------------------------------------------------

pub(super) fn wheel_steps(remainder: &mut f32, delta: f32, precise: bool) -> i32 {
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

/// Records which physical keys this console holds down in the guest and says whether an
/// event should be forwarded. A repeat of a key that is not held here is dropped: once a
/// focus loss has released every key, a repeat already in flight would press it again.
///
/// Keyed by evdev code (in Rune by the toolkit's physical key).
pub(super) fn track_physical(
    held: &mut Vec<(u32, u32)>,
    key: u32,
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

/// The XT make code of an evdev key code, in the encoding QEMU's extended
/// key event takes (an `0xe0` prefix is bit 7). The Rune `scancode`, keyed
/// by evdev code: up to `KEY_F12` (88) the two numberings are the same but
/// for the keys that have none.
pub fn scancode(code: u32) -> Option<u32> {
    Some(match code {
        // Escape to Numpad-dot, then the key right of left Shift, F11, F12.
        1..=83 | 86..=88 => code,
        96 => 0x9c,  // KEY_KPENTER
        97 => 0x9d,  // KEY_RIGHTCTRL
        98 => 0xb5,  // KEY_KPSLASH
        99 => 0x54,  // KEY_SYSRQ (Print Screen)
        100 => 0xb8, // KEY_RIGHTALT
        102 => 0xc7, // KEY_HOME
        103 => 0xc8, // KEY_UP
        104 => 0xc9, // KEY_PAGEUP
        105 => 0xcb, // KEY_LEFT
        106 => 0xcd, // KEY_RIGHT
        107 => 0xcf, // KEY_END
        108 => 0xd0, // KEY_DOWN
        109 => 0xd1, // KEY_PAGEDOWN
        110 => 0xd2, // KEY_INSERT
        111 => 0xd3, // KEY_DELETE
        119 => 0xc6, // KEY_PAUSE
        125 => 0xdb, // KEY_LEFTMETA
        126 => 0xdc, // KEY_RIGHTMETA
        127 => 0xdd, // KEY_COMPOSE (the menu key)
        _ => return None,
    })
}

/// The keys Rune always sent with their keysym, key codes negotiated or
/// not: the locks, Print Screen, Pause and the menu key.
fn special_keysym(code: u32) -> Option<u32> {
    Some(match code {
        58 => 0xffe5,  // Caps_Lock
        69 => 0xff7f,  // Num_Lock
        70 => 0xff14,  // Scroll_Lock
        99 => 0xff61,  // Print
        119 => 0xff13, // Pause
        127 => 0xff67, // Menu
        _ => return None,
    })
}

/// The keysym a US keyboard gives for an evdev code.
///
/// Rune took the keysym from the toolkit, which had applied the host's
/// layout. On GPUI the key arrives as a code only (decision 0006), so for a
/// guest whose server did not negotiate key codes the keysym is that of the
/// US layout. With key codes negotiated, which every supported host does,
/// this is not used.
pub fn us_keysym(code: u32, modifiers: Modifiers) -> Option<u32> {
    const ROWS: [(u32, &[u8], &[u8]); 4] = [
        (2, b"1234567890-=", b"!@#$%^&*()_+"),
        (16, b"qwertyuiop[]", b"QWERTYUIOP{}"),
        (30, b"asdfghjkl;'`", b"ASDFGHJKL:\"~"),
        (44, b"zxcvbnm,./", b"ZXCVBNM<>?"),
    ];
    for (first, plain, shifted) in ROWS {
        let Some(index) = code.checked_sub(first).map(|index| index as usize) else {
            continue;
        };
        if index >= plain.len() {
            continue;
        }
        let letter = plain[index].is_ascii_lowercase();
        // Caps Lock changes letters only; Shift undoes it.
        let upper = modifiers.shift != (letter && modifiers.caps_lock);
        return Some(u32::from(if upper {
            shifted[index]
        } else {
            plain[index]
        }));
    }
    let keypad = |locked: u32, unlocked: u32| {
        if modifiers.num_lock != modifiers.shift {
            locked
        } else {
            unlocked
        }
    };
    Some(match code {
        1 => 0xff1b,  // Escape
        14 => 0xff08, // BackSpace
        15 => 0xff09, // Tab
        28 => 0xff0d, // Return
        29 => 0xffe3, // Control_L
        42 => 0xffe1, // Shift_L
        43 => {
            if modifiers.shift {
                u32::from(b'|')
            } else {
                u32::from(b'\\')
            }
        }
        54 => 0xffe2, // Shift_R
        55 => 0xffaa, // KP_Multiply
        56 => 0xffe9, // Alt_L
        57 => 0x20,   // space
        59..=68 => 0xffbe + (code - 59), // F1 to F10
        71 => keypad(0xffb7, 0xff95), // KP_7, KP_Home
        72 => keypad(0xffb8, 0xff97), // KP_8, KP_Up
        73 => keypad(0xffb9, 0xff9a), // KP_9, KP_Prior
        74 => 0xffad, // KP_Subtract
        75 => keypad(0xffb4, 0xff96), // KP_4, KP_Left
        76 => keypad(0xffb5, 0xff9d), // KP_5, KP_Begin
        77 => keypad(0xffb6, 0xff98), // KP_6, KP_Right
        78 => 0xffab, // KP_Add
        79 => keypad(0xffb1, 0xff9c), // KP_1, KP_End
        80 => keypad(0xffb2, 0xff99), // KP_2, KP_Down
        81 => keypad(0xffb3, 0xff9b), // KP_3, KP_Next
        82 => keypad(0xffb0, 0xff9e), // KP_0, KP_Insert
        83 => keypad(0xffae, 0xff9f), // KP_Decimal, KP_Delete
        86 => {
            if modifiers.shift {
                u32::from(b'>')
            } else {
                u32::from(b'<')
            }
        }
        87 => 0xffc8,  // F11
        88 => 0xffc9,  // F12
        96 => 0xff8d,  // KP_Enter
        97 => 0xffe4,  // Control_R
        98 => 0xffaf,  // KP_Divide
        100 => 0xffea, // Alt_R
        102 => 0xff50, // Home
        103 => 0xff52, // Up
        104 => 0xff55, // Prior
        105 => 0xff51, // Left
        106 => 0xff53, // Right
        107 => 0xff57, // End
        108 => 0xff54, // Down
        109 => 0xff56, // Next
        110 => 0xff63, // Insert
        111 => 0xffff, // Delete
        125 => 0xffeb, // Super_L
        126 => 0xffec, // Super_R
        _ => return special_keysym(code),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rune: `console_view.rs`, moved; the keys are evdev codes now.
    #[test]
    fn a_repeat_after_release_does_not_press_the_key_again() {
        const KEY_A: u32 = 30;
        const KEY_B: u32 = 48;
        let mut held = Vec::new();
        assert!(track_physical(&mut held, KEY_A, 0, true, false), "press");
        assert!(
            track_physical(&mut held, KEY_A, 0, true, true),
            "repeat while held"
        );
        // Focus loss releases everything; the live desktop then saw one more press arrive.
        held.clear();
        assert!(
            !track_physical(&mut held, KEY_A, 0, true, true),
            "a repeat of a released key is dropped"
        );
        assert!(held.is_empty());
        assert!(
            track_physical(&mut held, KEY_A, 0, false, false),
            "stray release"
        );
        assert!(track_physical(&mut held, KEY_B, 0, true, false));
        assert!(track_physical(&mut held, KEY_B, 0, false, false));
        assert!(held.is_empty());
    }

    /// Rune: `console_view.rs`, moved.
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

    /// Rune: `console_view.rs`, moved.
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

    /// Rune: `console_view.rs`, moved; the keys are evdev codes now
    /// (`KEY_A` 30, `KEY_RIGHT` 106, `KEY_RIGHTCTRL` 97, `KEY_SYSRQ` 99,
    /// `KEY_PAUSE` 119).
    #[test]
    fn extended_scancodes_follow_qemu_make_code_encoding() {
        assert_eq!(scancode(30), Some(0x1e));
        assert_eq!(scancode(106), Some(0xcd));
        assert_eq!(scancode(97), Some(0x9d));
        assert_eq!(scancode(99), Some(0x54));
        assert_eq!(scancode(119), Some(0xc6));
        // Every code of the Rune table, and nothing else.
        for (code, make) in [
            (1, 1),
            (28, 28),
            (57, 57),
            (58, 58),
            (69, 69),
            (83, 83),
            (86, 86),
            (87, 87),
            (88, 88),
            (96, 0x9c),
            (98, 0xb5),
            (100, 0xb8),
            (102, 0xc7),
            (103, 0xc8),
            (104, 0xc9),
            (105, 0xcb),
            (107, 0xcf),
            (108, 0xd0),
            (109, 0xd1),
            (110, 0xd2),
            (111, 0xd3),
            (125, 0xdb),
            (126, 0xdc),
            (127, 0xdd),
        ] {
            assert_eq!(scancode(code), Some(make), "evdev {code}");
        }
        for code in [0, 84, 85, 89, 95, 101, 112, 118, 120, 124, 128, 255] {
            assert_eq!(scancode(code), None, "evdev {code}");
        }
    }

    #[test]
    fn a_key_goes_out_by_code_and_its_release_carries_the_keysym_of_its_press() {
        let plain = Modifiers::default();
        let shift = Modifiers {
            shift: true,
            ..plain
        };
        // Key codes negotiated: the code, and no keysym but for the keys
        // Rune always named.
        let mut input = CanvasInput::new();
        assert!(matches!(
            input.key_event(true, 30, true, false, plain),
            Key::Send(ConsoleInput::PhysicalKey {
                keysym: 0,
                keycode: 0x1e,
                down: true
            })
        ));
        assert_eq!(input.held_keys(), [30]);
        assert!(matches!(
            input.key_event(true, 58, true, false, plain),
            Key::Send(ConsoleInput::PhysicalKey {
                keysym: 0xffe5,
                keycode: 58,
                down: true
            })
        ));
        assert!(matches!(
            input.key_event(true, 30, false, false, plain),
            Key::Send(ConsoleInput::PhysicalKey {
                keysym: 0,
                keycode: 0x1e,
                down: false
            })
        ));
        assert_eq!(input.held_keys(), [58]);

        // Not negotiated: the US keysym at the press, and the same keysym at
        // the release although Shift went up in between.
        let mut input = CanvasInput::new();
        assert!(matches!(
            input.key_event(false, 30, true, false, shift),
            Key::Send(ConsoleInput::PhysicalKey {
                keysym: 0x41,
                keycode: 0x1e,
                down: true
            })
        ));
        assert!(matches!(
            input.key_event(false, 30, false, false, plain),
            Key::Send(ConsoleInput::PhysicalKey {
                keysym: 0x41,
                keycode: 0x1e,
                down: false
            })
        ));
        // A repeat of a key that is not held is dropped; an unknown code is
        // not the console's.
        assert!(matches!(
            input.key_event(false, 30, true, true, plain),
            Key::Dropped
        ));
        assert!(matches!(
            input.key_event(true, 240, true, false, plain),
            Key::Unknown
        ));
        assert!(input.held_keys().is_empty());
    }

    #[test]
    fn the_us_layout_gives_the_keysyms_for_a_guest_without_key_codes() {
        let plain = Modifiers::default();
        let shift = Modifiers {
            shift: true,
            ..plain
        };
        let caps = Modifiers {
            caps_lock: true,
            ..plain
        };
        let num = Modifiers {
            num_lock: true,
            ..plain
        };
        let sym = |c: char| Some(c as u32);
        assert_eq!(us_keysym(30, plain), sym('a'));
        assert_eq!(us_keysym(30, shift), sym('A'));
        assert_eq!(us_keysym(30, caps), sym('A'));
        assert_eq!(
            us_keysym(
                30,
                Modifiers {
                    shift: true,
                    ..caps
                }
            ),
            sym('a')
        );
        assert_eq!(us_keysym(2, plain), sym('1'));
        assert_eq!(us_keysym(2, shift), sym('!'));
        assert_eq!(us_keysym(2, caps), sym('1'), "Caps Lock is for letters");
        assert_eq!(us_keysym(13, shift), sym('+'));
        assert_eq!(us_keysym(27, shift), sym('}'));
        assert_eq!(us_keysym(39, plain), sym(';'));
        assert_eq!(us_keysym(40, shift), sym('"'));
        assert_eq!(us_keysym(41, plain), sym('`'));
        assert_eq!(us_keysym(43, plain), sym('\\'));
        assert_eq!(us_keysym(43, shift), sym('|'));
        assert_eq!(us_keysym(53, shift), sym('?'));
        assert_eq!(us_keysym(57, plain), sym(' '));
        assert_eq!(us_keysym(28, plain), Some(0xff0d));
        assert_eq!(us_keysym(59, plain), Some(0xffbe));
        assert_eq!(us_keysym(68, plain), Some(0xffc7));
        assert_eq!(us_keysym(88, plain), Some(0xffc9));
        assert_eq!(us_keysym(71, plain), Some(0xff95));
        assert_eq!(us_keysym(71, num), Some(0xffb7));
        assert_eq!(us_keysym(111, plain), Some(0xffff));
        assert_eq!(us_keysym(97, plain), Some(0xffe4));
        assert_eq!(us_keysym(119, plain), Some(0xff13));
        assert_eq!(us_keysym(240, plain), None);
        // Whatever has a make code has a keysym, so no key is lost on a
        // guest without key codes.
        for code in 0..256 {
            assert_eq!(
                scancode(code).is_some(),
                us_keysym(code, plain).is_some(),
                "evdev {code}"
            );
        }
    }

    #[test]
    fn the_pointer_is_mapped_through_the_geometry_and_remembered() {
        let mut input = CanvasInput::new();
        input.set_bounds(1000.0, 1000.0);
        let fit = Geometry::new(input.bounds(), 800, 600, false);
        assert!(matches!(
            input.pointer_event(fit, 500.0, 500.0, 5),
            Some(ConsoleInput::Pointer {
                x: 400,
                y: 300,
                buttons: 5
            })
        ));
        assert_eq!(input.pointer(), [500.0, 500.0]);
        // No picture yet: nothing to send, the position is still remembered.
        assert!(input.pointer_event(None, 10.0, 20.0, 0).is_none());
        assert_eq!(input.pointer(), [10.0, 20.0]);
        assert_eq!(fit.unwrap().painted(), [1000.0, 750.0]);
    }
}
