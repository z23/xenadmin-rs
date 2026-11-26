//! The element `console-canvas`: the picture of one console and the
//! pointer and keys that go to its guest.
//!
//! One [`ConsoleCanvas`] entity exists per window and seat. It keeps what
//! the seam leaves to the view and nothing else: the uploaded image with
//! its generation, and the window's input state (`CanvasInput`: bounds,
//! held keys, button mask, wheel remainders). Everything it does to the
//! console goes through the functions of
//! `xenadmin_ui::areas::console::canvas`, in an update of the store that
//! draws nothing again ([`AppStore::run_quiet`]).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use xenadmin_ui::Model;
use xenadmin_ui::areas::console::canvas::{self as model, CanvasInput, PointerButton};
use xenadmin_ui::areas::console::{Seat, view};

use super::keys;
use crate::kit::*;
use crate::render::Ui;
use crate::shell::actions::CONSOLE_CONTEXT;
use crate::store::{AppStore, Polled};
use crate::widgets;

/// The accessible label of the canvas (the Rune label).
pub const LABEL: &str = "VM console. Click to send keyboard and pointer input";
/// The id of the frame around the canvas: the element that takes the
/// remaining height, and in Actual size the view that scrolls on both axes.
pub const VIEW_KEY: &str = "console-view";
/// The id of the view that pans at Actual size, inside the frame.
pub const PAN_KEY: &str = "console-pan";
/// The id of the line shown while there is no picture.
pub const EMPTY_KEY: &str = "console-empty";
/// What that line says.
pub const EMPTY_TEXT: &str = "No picture from the console yet";
/// The canvas never gets less height than this; in a window too small for
/// the controls and the picture, the body scrolls instead. A detached
/// window at its smallest (480 by 640) still shows every control and the
/// help line with the picture; in the Console tab the picture comes before
/// the help line.
pub fn minimum_height(seat: Seat) -> f32 {
    match seat {
        Seat::Tab => 240.0,
        Seat::Window(_) => 112.0,
    }
}

/// A key the canvas forwarded to the model. Kept only in a build with the
/// `scenes` feature, for the tests: a scene's console has no stream, so the
/// model records nothing of a key.
#[cfg(feature = "scenes")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SentKey {
    /// `canvas::key`: an evdev code.
    Code { code: u32, down: bool, repeat: bool },
    /// `canvas::keysym_key`: a character no code is known for.
    Keysym { keysym: u32, down: bool },
}

pub struct ConsoleCanvas {
    store: Entity<AppStore>,
    seat: Seat,
    input: CanvasInput,
    focus: FocusHandle,
    /// The uploaded picture and the generation it is the picture of.
    image: Option<Arc<RenderImage>>,
    uploaded: u64,
    uploads: u64,
    drops: u64,
    /// Where the canvas was last laid out, in the window.
    origin: Point<Pixels>,
    /// The canvas has the focus of an active window.
    has_keyboard: bool,
    /// GPUI's modifier state as the canvas last saw it (the keystroke
    /// path makes the modifier keys from its changes).
    modifiers: Modifiers,
    caps_lock: bool,
    #[cfg(feature = "scenes")]
    sent: Vec<SentKey>,
    _subscriptions: Vec<Subscription>,
}

impl ConsoleCanvas {
    fn new(
        store: Entity<AppStore>,
        seat: Seat,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        let subscriptions = vec![
            // A new frame alone does not notify the store: look after every
            // poll, and after every update.
            cx.subscribe_in(&store, window, |canvas, _, _: &Polled, window, cx| {
                canvas.frame_changed(window, cx);
            }),
            cx.observe_in(&store, window, |canvas, _, window, cx| {
                canvas.frame_changed(window, cx);
            }),
            cx.on_focus(&focus, window, |canvas, window, cx| {
                canvas.focus_gained(window, cx);
            }),
            // Another element or a dialog took the focus, or the canvas is
            // no longer drawn (its tab was left).
            cx.on_blur(&focus, window, |canvas, _, cx| canvas.release(cx)),
            cx.observe_window_activation(window, |canvas, window, cx| {
                if !window.is_window_active() {
                    canvas.release(cx);
                } else if canvas.focus.is_focused(window) {
                    canvas.focus_gained(window, cx);
                }
            }),
            cx.on_release(|canvas, cx| {
                let (seat, input) = (canvas.seat, &mut canvas.input);
                canvas.store.update(cx, |store, cx| {
                    store.run_quiet(|m| model::focus_lost(m, seat, input), cx);
                });
            }),
        ];
        keys::attach(window, cx);
        Self {
            store,
            seat,
            input: CanvasInput::new(),
            focus,
            image: None,
            uploaded: 0,
            uploads: 0,
            drops: 0,
            origin: Point::default(),
            has_keyboard: false,
            modifiers: Modifiers::default(),
            caps_lock: false,
            #[cfg(feature = "scenes")]
            sent: Vec::new(),
            _subscriptions: subscriptions,
        }
    }

    pub fn seat(&self) -> Seat {
        self.seat
    }

    /// The input state of this window's canvas.
    pub fn input(&self) -> &CanvasInput {
        &self.input
    }

    pub fn focus(&self) -> &FocusHandle {
        &self.focus
    }

    /// The uploaded picture.
    pub fn image(&self) -> Option<&Arc<RenderImage>> {
        self.image.as_ref()
    }

    /// The generation of the uploaded picture; 0 when there is none.
    pub fn uploaded(&self) -> u64 {
        self.uploaded
    }

    /// How many pictures were uploaded and how many dropped.
    pub fn uploads(&self) -> (u64, u64) {
        (self.uploads, self.drops)
    }

    /// Whether keys of the seat's listener go to this canvas.
    pub fn has_keyboard(&self) -> bool {
        self.has_keyboard
    }

    /// The keys forwarded to the model, oldest first.
    #[cfg(feature = "scenes")]
    pub fn sent_keys(&self) -> &[SentKey] {
        &self.sent
    }

    /// One call of the seam: an update of the model that draws nothing.
    fn call<R>(
        &mut self,
        cx: &mut Context<Self>,
        act: impl FnOnce(&mut Model, Seat, &mut CanvasInput) -> R,
    ) -> R {
        let (seat, input) = (self.seat, &mut self.input);
        self.store
            .update(cx, |store, cx| store.run_quiet(|m| act(m, seat, input), cx))
    }

    // -- the picture --------------------------------------------------------

    fn generation(&self, cx: &App) -> u64 {
        model::frame_generation(self.store.read(cx).model(), self.seat)
    }

    fn drop_image(&mut self, window: &mut Window) {
        if let Some(image) = self.image.take() {
            let _ = window.drop_image(image);
            self.drops += 1;
        }
    }

    /// After a poll or an update: another picture than the uploaded one is
    /// a reason to draw this canvas again, and only it. A picture that is
    /// gone is dropped at once, drawn or not.
    fn frame_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let generation = self.generation(cx);
        if generation == self.uploaded {
            return;
        }
        if generation == 0 {
            self.drop_image(window);
            self.uploaded = 0;
        }
        cx.notify();
    }

    /// Before every paint: the uploaded picture is the session's current
    /// one, or the old one is dropped and the current one uploaded.
    fn sync_frame(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let generation = self.generation(cx);
        if generation == self.uploaded {
            return;
        }
        self.drop_image(window);
        let frame = model::frame(self.store.read(cx).model(), self.seat);
        if let Some(image) = frame.as_ref().and_then(render_image) {
            self.image = Some(Arc::new(image));
            self.uploads += 1;
        }
        self.uploaded = frame.map_or(generation, |frame| frame.generation);
    }

    fn laid_out(&mut self, bounds: Bounds<Pixels>) {
        self.origin = bounds.origin;
        self.input
            .set_bounds(f32::from(bounds.size.width), f32::from(bounds.size.height));
    }

    // -- the pointer --------------------------------------------------------

    fn local(&self, position: Point<Pixels>) -> [f32; 2] {
        [
            f32::from(position.x - self.origin.x),
            f32::from(position.y - self.origin.y),
        ]
    }

    fn pointer_button(
        &mut self,
        button: PointerButton,
        down: bool,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let [x, y] = self.local(position);
        self.call(cx, |m, seat, input| {
            model::pointer_button(m, seat, input, button, down, x, y);
        });
    }

    fn pointer_move(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let [x, y] = self.local(position);
        self.call(cx, |m, seat, input| {
            model::pointer_move(m, seat, input, x, y);
        });
    }

    /// Returns whether the wheel was the console's; `false` leaves it to
    /// the view that pans (Shift with the wheel in Actual size).
    fn wheel(&mut self, event: &ScrollEvent, cx: &mut Context<Self>) -> bool {
        let position = self.local(event.position);
        let (delta, precise) = match event.delta {
            ScrollDelta::Pixels(delta) => ([f32::from(delta.x), f32::from(delta.y)], true),
            ScrollDelta::Lines(lines) => ([lines.x * 40.0, lines.y * 40.0], false),
        };
        let shift = event.modifiers.shift;
        self.call(cx, |m, seat, input| {
            model::wheel(m, seat, input, position, delta, precise, shift)
        })
    }

    // -- the keys -----------------------------------------------------------

    /// A key by its evdev code: from the seat's listener, or from the
    /// keystroke path's table.
    pub(crate) fn send_code(
        &mut self,
        code: u32,
        down: bool,
        modifiers: model::Modifiers,
        cx: &mut Context<Self>,
    ) {
        // The listener reports a key once; GPUI repeats a held key.
        let repeat = down && self.input.held_keys().contains(&code);
        self.call(cx, |m, seat, input| {
            model::key(m, seat, input, code, down, repeat, modifiers);
        });
        #[cfg(feature = "scenes")]
        self.sent.push(SentKey::Code { code, down, repeat });
    }

    fn keystroke_modifiers(&self) -> model::Modifiers {
        model::Modifiers {
            shift: self.modifiers.shift,
            caps_lock: self.caps_lock,
            // GPUI does not report Num Lock.
            num_lock: false,
        }
    }

    /// The keystroke path: a key GPUI names. Nothing while the listener
    /// delivers the same key by its code.
    fn named_key(&mut self, key: &str, down: bool, cx: &mut Context<Self>) {
        if keys::seat_bound(cx) {
            return;
        }
        if let Some(code) = keys::evdev_code(key) {
            let modifiers = self.keystroke_modifiers();
            self.send_code(code, down, modifiers, cx);
        } else if let Some(keysym) = keys::keysym(key) {
            self.call(cx, |m, seat, input| {
                model::keysym_key(m, seat, input, keysym, down);
            });
            #[cfg(feature = "scenes")]
            self.sent.push(SentKey::Keysym { keysym, down });
        }
    }

    /// A keystroke the interceptor stopped while this canvas had the focus.
    pub(crate) fn keystroke(&mut self, keystroke: &Keystroke, cx: &mut Context<Self>) {
        self.named_key(&keystroke.key, true, cx);
    }

    fn key_up(&mut self, event: &KeyUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.named_key(&event.keystroke.key, false, cx);
    }

    fn modifiers_changed(
        &mut self,
        event: &ModifiersChangedEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let changes = keys::modifier_changes(self.modifiers, event.modifiers);
        self.modifiers = event.modifiers;
        self.caps_lock = event.capslock.on;
        if keys::seat_bound(cx) {
            return;
        }
        let modifiers = self.keystroke_modifiers();
        for (code, down) in changes {
            self.send_code(code, down, modifiers, cx);
        }
    }

    fn focus_gained(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.has_keyboard = window.is_window_active();
        if self.has_keyboard {
            keys::focus_gained(cx);
        }
        cx.notify();
    }

    /// The canvas no longer has the keyboard, for whatever reason: every
    /// held key and button is released in the guest. Harmless when nothing
    /// is held.
    pub fn release(&mut self, cx: &mut Context<Self>) {
        self.has_keyboard = false;
        self.modifiers = Modifiers::default();
        self.call(cx, model::focus_lost);
        cx.notify();
    }

    /// The window of the canvas is going away: the input is released and
    /// the picture leaves the window's atlas.
    pub fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.release(cx);
        self.drop_image(window);
        self.uploaded = 0;
    }
}

/// The picture as GPUI wants it: BGRA, opaque.
fn render_image(frame: &model::Frame) -> Option<RenderImage> {
    let (width, height) = (u32::from(frame.width), u32::from(frame.height));
    let length = width as usize * height as usize * 4;
    let rgba = frame.rgba.get(..length)?;
    let mut bgra = vec![0u8; length];
    for (dst, src) in bgra
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .zip(rgba.as_chunks::<4>().0)
    {
        *dst = [src[2], src[1], src[0], 255];
    }
    let buffer = image::RgbaImage::from_raw(width, height, bgra)?;
    Some(RenderImage::new([image::Frame::new(buffer)]))
}

fn pointer_button(button: MouseButton) -> Option<PointerButton> {
    match button {
        MouseButton::Left => Some(PointerButton::Left),
        MouseButton::Middle => Some(PointerButton::Middle),
        MouseButton::Right => Some(PointerButton::Right),
        MouseButton::Navigate(_) => None,
    }
}

/// Paints the picture and listens to the pointer. The listeners are the
/// window's, so a drag that leaves the canvas keeps arriving.
fn paint(
    bounds: Bounds<Pixels>,
    hitbox: &Hitbox,
    image: Option<Arc<RenderImage>>,
    canvas: &Entity<ConsoleCanvas>,
    window: &mut Window,
    cx: &mut App,
) {
    let (store, seat) = {
        let canvas = canvas.read(cx);
        (canvas.store.clone(), canvas.seat)
    };
    let extent = [f32::from(bounds.size.width), f32::from(bounds.size.height)];
    let geometry = model::geometry(store.read(cx).model(), seat, extent);
    if let (Some(image), Some(geometry)) = (image, geometry) {
        let [width, height] = geometry.painted();
        let picture = Bounds {
            origin: bounds.origin + point(px(geometry.offset[0]), px(geometry.offset[1])),
            size: size(px(width), px(height)),
        };
        let _ = window.paint_image(bounds, picture, Corners::default(), image, 0, false);
    }

    window.on_mouse_event({
        let (canvas, hitbox) = (canvas.clone(), hitbox.clone());
        move |event: &MouseDownEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble || !hitbox.is_hovered(window) {
                return;
            }
            let Some(button) = pointer_button(event.button) else {
                return;
            };
            canvas.update(cx, |canvas, cx| {
                canvas.focus.focus(window, cx);
                canvas.pointer_button(button, true, event.position, cx);
            });
            cx.stop_propagation();
        }
    });
    window.on_mouse_event({
        let (canvas, hitbox) = (canvas.clone(), hitbox.clone());
        move |event: &MouseUpEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            let Some(button) = pointer_button(event.button) else {
                return;
            };
            // A button pressed on the canvas is released wherever the
            // pointer is by then.
            let hovered = hitbox.is_hovered(window);
            let held = canvas.read(cx).input.buttons() & button.mask() != 0;
            if !hovered && !held {
                return;
            }
            canvas.update(cx, |canvas, cx| {
                canvas.pointer_button(button, false, event.position, cx);
            });
            if hovered {
                cx.stop_propagation();
            }
        }
    });
    window.on_mouse_event({
        let (canvas, hitbox) = (canvas.clone(), hitbox.clone());
        move |event: &MouseMoveEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            let dragging = canvas.read(cx).input.buttons() != 0;
            if dragging || hitbox.is_hovered(window) {
                canvas.update(cx, |canvas, cx| canvas.pointer_move(event.position, cx));
            }
        }
    });
    window.on_mouse_event({
        let (canvas, hitbox) = (canvas.clone(), hitbox.clone());
        move |event: &ScrollEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble || !hitbox.is_hovered(window) {
                return;
            }
            if canvas.update(cx, |canvas, cx| canvas.wheel(event, cx)) {
                cx.stop_propagation();
            }
        }
    });
}

impl Render for ConsoleCanvas {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_frame(window, cx);
        let content = model::content_size(self.store.read(cx).model(), self.seat);
        let theme = cx.theme();
        let ring = if self.focus.is_focused(window) {
            theme.ring
        } else {
            theme.border
        };
        let entity = cx.entity();
        let image = self.image.clone();
        let painter = canvas(
            {
                let entity = entity.clone();
                move |bounds, window: &mut Window, cx: &mut App| {
                    entity.update(cx, |canvas, _| canvas.laid_out(bounds));
                    window.insert_hitbox(bounds, HitboxBehavior::Normal)
                }
            },
            move |bounds, hitbox, window, cx| paint(bounds, &hitbox, image, &entity, window, cx),
        )
        .absolute()
        .inset_0();
        let empty = self.image.is_none().then(|| {
            div()
                .id(EMPTY_KEY)
                .test_support()
                .role(Role::Label)
                .aria_label(EMPTY_TEXT)
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(white().opacity(0.7))
                .child(EMPTY_TEXT)
        });
        let surface = div()
            .id(view::CANVAS)
            .test_support()
            .role(Role::Application)
            .aria_label(LABEL)
            .track_focus(&self.focus)
            .key_context(CONSOLE_CONTEXT)
            .on_key_up(cx.listener(Self::key_up))
            .on_modifiers_changed(cx.listener(Self::modifiers_changed))
            .relative()
            .overflow_hidden()
            .bg(black())
            .child(painter)
            .children(empty);
        // The frame takes the height the other blocks leave. Fit: the
        // canvas fills it and letterboxes the picture inside itself.
        // Actual size: the frame scrolls on both axes and the canvas is
        // exactly as large as the guest's desktop, at its origin (a canvas
        // that also asked for the full width had no width in Rune).
        let frame = v_flex()
            .id(VIEW_KEY)
            .test_support()
            .w_full()
            // Not `flex_1`: its basis is a percentage, which is the height
            // of the content where the column's own height is not known
            // yet, and a canvas at Actual size would then never be less
            // than the guest's desktop.
            .flex_grow(1.)
            .flex_shrink(1.)
            .flex_basis(px(0.))
            .min_h(px(minimum_height(self.seat)))
            .border_1()
            .border_color(ring)
            .bg(black());
        match content {
            None => frame
                .child(surface.w_full().flex_1().min_h_0())
                .into_any_element(),
            // The view that pans, with scrollbars: Shift with the wheel
            // pans (the canvas leaves that event alone), and a wheel has
            // one axis. The Kit's scroll area takes the id of what it
            // wraps, so the keyed frame stays around it.
            Some([width, height]) => frame
                .child(
                    div()
                        .id(PAN_KEY)
                        .w_full()
                        .flex_grow(1.)
                        .flex_shrink(1.)
                        .flex_basis(px(0.))
                        .min_h_0()
                        .child(surface.flex_none().w(px(width)).h(px(height)))
                        .overflow_scrollbar(),
                )
                .into_any_element(),
        }
    }
}

// ---------------------------------------------------------------------------
// The canvases of the application
// ---------------------------------------------------------------------------

struct Entry {
    window: WindowId,
    seat: Seat,
    canvas: WeakEntity<ConsoleCanvas>,
}

/// Every live canvas, by window: the interceptor and the windows find
/// theirs here.
#[derive(Default)]
struct Canvases {
    entries: Vec<Entry>,
    _interceptor: Option<Subscription>,
}

impl Global for Canvases {}

/// Every canvas of the application with its window.
pub(crate) fn live(cx: &App) -> Vec<(WindowId, Entity<ConsoleCanvas>)> {
    cx.try_global::<Canvases>()
        .map(|canvases| {
            canvases
                .entries
                .iter()
                .filter_map(|entry| Some((entry.window, entry.canvas.upgrade()?)))
                .collect()
        })
        .unwrap_or_default()
}

/// The canvas of `seat` in the window `window`, once it was drawn there.
pub fn canvas_of(window: WindowId, seat: Seat, cx: &App) -> Option<Entity<ConsoleCanvas>> {
    cx.try_global::<Canvases>()?
        .entries
        .iter()
        .filter(|entry| entry.window == window && entry.seat == seat)
        .find_map(|entry| entry.canvas.upgrade())
}

/// The canvases of one window.
pub(crate) fn in_window(window: WindowId, cx: &App) -> Vec<Entity<ConsoleCanvas>> {
    live(cx)
        .into_iter()
        .filter(|(id, _)| *id == window)
        .map(|(_, canvas)| canvas)
        .collect()
}

/// Every canvas of the application.
pub(crate) fn all(cx: &App) -> Vec<Entity<ConsoleCanvas>> {
    live(cx).into_iter().map(|(_, canvas)| canvas).collect()
}

/// The canvas the keys of the seat go to: the focused one of the active
/// window.
pub(crate) fn with_keyboard(cx: &App) -> Option<Entity<ConsoleCanvas>> {
    all(cx)
        .into_iter()
        .find(|canvas| canvas.read(cx).has_keyboard)
}

/// While a console canvas has the focus of the window a keystroke is
/// for, the keystroke is the guest's: no binding is looked up, so no
/// command shortcut, no focus traversal and no Escape of the application
/// happens. A keystroke that is a modifier key alone passes: stopping it
/// would also stop the report of the modifier's release (cookbook 10.3).
fn intercept(event: &KeystrokeEvent, window: &mut Window, cx: &mut App) {
    let target = in_window(window.window_handle().window_id(), cx)
        .into_iter()
        .find(|canvas| canvas.read(cx).focus.is_focused(window));
    let Some(target) = target else {
        return;
    };
    if event.keystroke.is_modifier_key() {
        return;
    }
    cx.stop_propagation();
    target.update(cx, |canvas, cx| canvas.keystroke(&event.keystroke, cx));
}

fn create(
    store: &Entity<AppStore>,
    seat: Seat,
    window: &mut Window,
    cx: &mut App,
) -> Entity<ConsoleCanvas> {
    let canvas = cx.new(|cx| ConsoleCanvas::new(store.clone(), seat, window, cx));
    if cx.try_global::<Canvases>().is_none() {
        cx.set_global(Canvases::default());
    }
    if cx.global::<Canvases>()._interceptor.is_none() {
        let interceptor = cx.intercept_keystrokes(intercept);
        cx.global_mut::<Canvases>()._interceptor = Some(interceptor);
    }
    let canvases = cx.global_mut::<Canvases>();
    canvases
        .entries
        .retain(|entry| entry.canvas.upgrade().is_some());
    canvases.entries.push(Entry {
        window: window.window_handle().window_id(),
        seat,
        canvas: canvas.downgrade(),
    });
    canvas
}

/// Registers the element with a renderer. Each renderer (each window) has
/// canvases of its own: input state is never shared between windows.
pub fn register(ui: &Ui) {
    let canvases: Rc<RefCell<HashMap<Seat, Entity<ConsoleCanvas>>>> = Rc::default();
    ui.register_custom(view::CANVAS, move |custom, ui, window, cx| {
        let seat = custom
            .params
            .get("session")
            .and_then(|session| session.as_str())
            .and_then(Seat::parse);
        let Some(seat) = seat else {
            let id = custom.key.clone().unwrap_or_else(|| custom.name.clone());
            return widgets::missing_custom(id, custom, cx);
        };
        let existing = canvases.borrow().get(&seat).cloned();
        let canvas = existing.unwrap_or_else(|| {
            let canvas = create(&ui.store, seat, window, cx);
            canvases.borrow_mut().insert(seat, canvas.clone());
            canvas
        });
        canvas.into_any_element()
    });
}
