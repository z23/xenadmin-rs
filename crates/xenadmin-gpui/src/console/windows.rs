//! The detached console windows: one OS window per slot of the model.
//!
//! The model owns the slots and what each window shows
//! (`areas::console::view::window`). [`ConsoleWindows`] performs its three
//! requests (`Intent::ConsoleWindow` Open, Close, Activate) and tells it
//! when a window is gone, with the lease the Open request carried
//! (`windows::closed`): that is what ends the console of a window the user
//! closed and what frees a slot the model closed.
//!
//! [`ConsoleWindowView`] is the root of such a window: the model's blocks
//! through the generic renderer, one of which is the canvas of the window's
//! seat. Appearance and interface size are the theme's, which is the
//! application's: there is one model and nothing to forward.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use xenadmin_ui::areas::console::{self, Seat, view, windows};
use xenadmin_ui::command::CommandId;
use xenadmin_ui::intent::{ConsoleWindowId, ConsoleWindowRequest};

use super::canvas;
use crate::intents::Hooks;
use crate::kit::*;
use crate::render::{Pass, Ui, blocks};
use crate::shell::actions::{RunCommand, SHELL_CONTEXT};
use crate::store::AppStore;

/// The id of the root element of a console window.
pub const ROOT_KEY: &str = "console-window";
/// The path prefix of the window's flattened description.
pub const BASE: &str = "window";
/// The size a console window opens at, and the least it can have.
pub const DEFAULT_SIZE: (f32, f32) = (1100., 800.);
pub const MINIMUM_SIZE: (f32, f32) = (480., 640.);

const PADDING: f32 = 16.0;

/// Called with the window when its close button is pressed.
type OnClose = Rc<dyn Fn(&mut Window, &mut App)>;

/// The root view of a detached console window.
pub struct ConsoleWindowView {
    ui: Ui,
    id: ConsoleWindowId,
    focus: FocusHandle,
    title: String,
    on_close: OnClose,
    _subscription: Subscription,
}

impl ConsoleWindowView {
    /// The view of window `id` over `store`. `on_close` is what the close
    /// button of a client-side decoration runs before the window goes.
    pub fn new(
        store: Entity<AppStore>,
        id: ConsoleWindowId,
        on_close: impl Fn(&mut Window, &mut App) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let ui = Ui::new(store.clone());
        canvas::register(&ui);
        let subscription = cx.observe_in(&store, window, |view, _, window, cx| {
            view.sync_title(window, cx);
            cx.notify();
        });
        let focus = cx.focus_handle();
        // Bindings with a context match once something in the window has
        // the keyboard.
        focus.focus(window, cx);
        let mut view = Self {
            ui,
            id,
            focus,
            title: String::new(),
            on_close: Rc::new(on_close),
            _subscription: subscription,
        };
        view.sync_title(window, cx);
        view
    }

    pub fn id(&self) -> ConsoleWindowId {
        self.id
    }

    pub fn ui(&self) -> &Ui {
        &self.ui
    }

    /// The title the window was last given.
    pub fn title(&self) -> &str {
        &self.title
    }

    fn sync_title(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let title = view::window(self.ui.store.read(cx).model(), self.id).title;
        if title != self.title {
            window.set_window_title(&title);
            self.title = title;
        }
    }

    fn run_command(&mut self, action: &RunCommand, _: &mut Window, cx: &mut Context<Self>) {
        let id = CommandId(action.id);
        self.ui.store.update(cx, |store, cx| store.invoke(id, cx));
    }
}

impl Focusable for ConsoleWindowView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for ConsoleWindowView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let items = view::window(self.ui.store.read(cx).model(), self.id).blocks;
        let scope = format!("{ROOT_KEY}:{}", self.id.0);
        let width = (window.viewport_size().width - px(PADDING * 2.)).max(px(120.));
        let pass = Pass {
            ui: &self.ui,
            scope: &scope,
            width,
            fill: false,
            on_cancel: None,
        };
        self.ui.cache.borrow_mut().begin(&scope);
        let children = blocks(&pass, BASE, &items, window, cx);
        self.ui.cache.borrow_mut().end(&scope, window, cx);
        // Where the compositor leaves the decoration to the client, the
        // Kit's bar draws the window controls and moves the window.
        let title_bar =
            matches!(window.window_decorations(), Decorations::Client { .. }).then(|| {
                let on_close = Rc::clone(&self.on_close);
                let muted = cx.theme().muted_foreground;
                TitleBar::new()
                    .h(rems(2.125))
                    .on_close_window(move |_, window, cx| {
                        on_close(window, cx);
                        window.remove_window();
                    })
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .px_3()
                            .text_sm()
                            .text_color(muted)
                            .child(SharedString::from(self.title.clone())),
                    )
            });
        let theme = cx.theme();
        v_flex()
            .id(ROOT_KEY)
            .test_support()
            .track_focus(&self.focus)
            // The command shortcuts work here as in the main window, on the
            // same model; none of them inside the canvas.
            .key_context(SHELL_CONTEXT)
            .on_action(cx.listener(Self::run_command))
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .children(title_bar)
            .child(
                div().w_full().flex_1().min_h_0().overflow_hidden().child(
                    // A column of definite height: the canvas takes what
                    // the controls leave, and a window too small for both
                    // scrolls.
                    v_flex()
                        .id("console-window-scroll")
                        .size_full()
                        .overflow_y_scrollbar()
                        .p(px(PADDING))
                        .gap_3()
                        .children(children),
                ),
            )
    }
}

/// The options of a console window: the model's title, 1100 by 800, at
/// least 480 by 640, the application's id.
pub fn window_options(title: &str, cx: &App) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(DEFAULT_SIZE.0), px(DEFAULT_SIZE.1)),
            cx,
        ))),
        titlebar: Some(TitlebarOptions {
            title: Some(SharedString::from(title.to_owned())),
            ..TitleBar::title_bar_options()
        }),
        app_id: Some(crate::app::APP_ID.to_owned()),
        window_min_size: Some(size(px(MINIMUM_SIZE.0), px(MINIMUM_SIZE.1))),
        window_decorations: Some(WindowDecorations::Server),
        focus: true,
        show: true,
        is_resizable: true,
        ..TitleBar::window_options()
    }
}

/// One open console window.
struct Open {
    handle: AnyWindowHandle,
    /// The lease of the console the window was opened for.
    lease: u64,
    view: Entity<ConsoleWindowView>,
}

struct Inner {
    store: Entity<AppStore>,
    /// The main window: where Fullscreen of the Console tab applies.
    main: Option<AnyWindowHandle>,
    open: BTreeMap<u32, Open>,
    _subscriptions: Vec<Subscription>,
}

/// The console windows of the application. Cheap to clone.
#[derive(Clone)]
pub struct ConsoleWindows {
    inner: Rc<RefCell<Inner>>,
}

impl ConsoleWindows {
    pub fn new(store: &Entity<AppStore>) -> Self {
        Self {
            inner: Rc::new(RefCell::new(Inner {
                store: store.clone(),
                main: None,
                open: BTreeMap::new(),
                _subscriptions: Vec::new(),
            })),
        }
    }

    /// `base` with the three console hooks: the windows' requests, the
    /// release of every console's input, and the close of every window.
    pub fn hooks(&self, base: Hooks) -> Hooks {
        let (perform, release, close) = (self.clone(), self.clone(), self.clone());
        Hooks {
            console_window: Rc::new(move |request, cx| perform.perform(request, cx)),
            release_console_input: Rc::new(move |cx| release.release_input(cx)),
            close_console_windows: Rc::new(move |cx| close.close_all(cx)),
            ..base
        }
    }

    /// Starts what follows the model without a request: Fullscreen, and a
    /// window that went away without being asked. `main` is the main
    /// window, which Fullscreen of the Console tab toggles.
    pub fn install(&self, main: Option<AnyWindowHandle>, cx: &mut App) {
        let store = self.inner.borrow().store.clone();
        let (fullscreen, gone) = (self.clone(), self.clone());
        let subscriptions = vec![
            // TO BE REPLACED: `intent.rs` has no fullscreen request yet, so
            // the model keeps a list that is drained after every update.
            cx.observe(&store, move |_, cx| fullscreen.drain_fullscreen(cx)),
            cx.on_window_closed(move |cx, id| gone.window_gone(id, cx)),
        ];
        let mut inner = self.inner.borrow_mut();
        inner.main = main;
        inner._subscriptions = subscriptions;
    }

    /// The window of a slot and its view, while it is open.
    pub fn window(
        &self,
        id: ConsoleWindowId,
    ) -> Option<(AnyWindowHandle, Entity<ConsoleWindowView>)> {
        let inner = self.inner.borrow();
        let open = inner.open.get(&id.0)?;
        Some((open.handle, open.view.clone()))
    }

    /// The slots that have a window, in order.
    pub fn open_ids(&self) -> Vec<ConsoleWindowId> {
        self.inner
            .borrow()
            .open
            .keys()
            .map(|id| ConsoleWindowId(*id))
            .collect()
    }

    fn store(&self) -> Entity<AppStore> {
        self.inner.borrow().store.clone()
    }

    /// Tells the model the window of this lease is gone.
    fn closed(&self, id: ConsoleWindowId, lease: u64, cx: &mut App) {
        self.store().update(cx, |store, cx| {
            store.run(
                |m| {
                    windows::closed(m, id, lease);
                },
                cx,
            );
        });
    }

    /// Performs one request of the model.
    pub fn perform(&self, request: ConsoleWindowRequest, cx: &mut App) {
        match request {
            ConsoleWindowRequest::Open { id, payload } => {
                let payload =
                    payload.and_then(|payload| payload.downcast::<windows::WindowPayload>().ok());
                match payload {
                    Some(payload) => self.open(id, payload.lease, &payload.title, cx),
                    None => eprintln!("A console window request carried no payload: {id:?}"),
                }
            }
            ConsoleWindowRequest::Activate(id) => {
                if let Some((handle, _)) = self.window(id) {
                    let _ = handle.update(cx, |_, window, _| window.activate_window());
                }
            }
            ConsoleWindowRequest::Close(id) => self.close(id, cx),
        }
    }

    fn open(&self, id: ConsoleWindowId, lease: u64, title: &str, cx: &mut App) {
        // A window that is still there is used again; the console it was
        // opened for is over.
        let existing = self.inner.borrow_mut().open.get_mut(&id.0).map(|open| {
            let before = std::mem::replace(&mut open.lease, lease);
            (open.handle, before)
        });
        if let Some((handle, before)) = existing {
            if before != lease {
                self.closed(id, before, cx);
            }
            let _ = handle.update(cx, |_, window, _| window.activate_window());
            return;
        }
        let store = self.store();
        let options = window_options(title, cx);
        let manager = self.clone();
        let opened = gpui_kit::open_window(options, cx, move |window, cx| {
            // The window manager asks to close: the model is told first.
            let on_user_close = manager.clone();
            window.on_window_should_close(cx, move |window, cx| {
                on_user_close.user_closed(id, window, cx);
                true
            });
            cx.new(|cx| {
                ConsoleWindowView::new(
                    store,
                    id,
                    move |window, cx| manager.user_closed(id, window, cx),
                    window,
                    cx,
                )
            })
        });
        match opened {
            Ok((handle, view)) => {
                self.inner.borrow_mut().open.insert(
                    id.0,
                    Open {
                        handle,
                        lease,
                        view,
                    },
                );
            }
            Err(error) => {
                eprintln!("The console window did not open: {error}");
                // The slot stays unusable until the model hears of it.
                self.closed(id, lease, cx);
            }
        }
    }

    /// The model asked for the window to close.
    fn close(&self, id: ConsoleWindowId, cx: &mut App) {
        let Some(open) = self.inner.borrow_mut().open.remove(&id.0) else {
            // Gone already; the model heard of it then.
            return;
        };
        let _ = open.handle.update(cx, |_, window, cx| {
            release_window(window, cx);
            window.remove_window();
        });
        self.closed(id, open.lease, cx);
    }

    /// The user closed the window. It runs inside the window's update.
    fn user_closed(&self, id: ConsoleWindowId, window: &mut Window, cx: &mut App) {
        let Some(open) = self.inner.borrow_mut().open.remove(&id.0) else {
            return;
        };
        release_window(window, cx);
        self.closed(id, open.lease, cx);
    }

    /// A window is gone that no one closed through this type.
    fn window_gone(&self, window: WindowId, cx: &mut App) {
        let gone = {
            let mut inner = self.inner.borrow_mut();
            let id = inner
                .open
                .iter()
                .find(|(_, open)| open.handle.window_id() == window)
                .map(|(id, _)| *id);
            id.and_then(|id| Some((id, inner.open.remove(&id)?)))
        };
        if let Some((id, open)) = gone {
            self.closed(ConsoleWindowId(id), open.lease, cx);
        }
    }

    /// The main window was left: every console of a window that is not
    /// the active one lets go of what it holds down in its guest. (Each
    /// canvas also lets go when its own window is left; the window the
    /// user went to, a console window perhaps, keeps its keyboard.)
    pub fn release_input(&self, cx: &mut App) {
        let active = cx.active_window().map(|window| window.window_id());
        for (window, canvas) in canvas::live(cx) {
            if Some(window) != active {
                canvas.update(cx, |canvas, cx| canvas.release(cx));
            }
        }
    }

    /// Closes every console window: the close path of the application,
    /// before the sessions are disconnected. Every console lets go of its
    /// guest's input first, the Console tab's too.
    pub fn close_all(&self, cx: &mut App) {
        for canvas in canvas::all(cx) {
            canvas.update(cx, |canvas, cx| canvas.release(cx));
        }
        self.store()
            .update(cx, |store, cx| store.run(windows::close_all, cx));
        // The requests this queued are performed later; the windows go now.
        for id in self.open_ids() {
            self.close(id, cx);
        }
    }

    /// TO BE REPLACED by an intent: toggles fullscreen of the window of
    /// each seat the model lists.
    fn drain_fullscreen(&self, cx: &mut App) {
        let seats = self
            .store()
            .update(cx, |store, _| store.take(console::take_fullscreen_requests));
        for seat in seats {
            let handle = match seat {
                Seat::Tab => self.inner.borrow().main,
                Seat::Window(id) => self.window(id).map(|(handle, _)| handle),
            };
            if let Some(handle) = handle {
                // After the update that asked: the window may be the one
                // being updated now.
                cx.defer(move |cx| {
                    let _ = handle.update(cx, |_, window, _| window.toggle_fullscreen());
                });
            }
        }
    }
}

/// The canvases of a window that is going away let go of the guest's
/// input and of their picture.
fn release_window(window: &mut Window, cx: &mut App) {
    for canvas in canvas::in_window(window.window_handle().window_id(), cx) {
        canvas.update(cx, |canvas, cx| canvas.close(window, cx));
    }
}
