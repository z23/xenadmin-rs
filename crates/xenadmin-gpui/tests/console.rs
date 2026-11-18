//! The console view, driven through real events on the Kit's headless test
//! platform: the canvas element, its pointer and keys, and the detached
//! console windows.
//!
//! What these tests do NOT establish:
//! - The test platform has no Wayland seat. Keys reach the canvas here as
//!   GPUI keystrokes (the path used where no key listener exists), and the
//!   key's name gives the evdev code. What the second keyboard listener
//!   (decision 0006, route A) does on a real seat is not tested.
//! - A scene's console has no stream: the model takes every call of the
//!   seam and sends nothing, and it records no held key. The tests read
//!   what the canvas forwarded (`sent_keys`), the button mask and pointer
//!   of its `CanvasInput`, and the session's input epoch.
#![cfg(feature = "scenes")]

use std::cell::Cell;
use std::rc::Rc;

use xenadmin_gpui::console::canvas::{EMPTY_KEY, SentKey, VIEW_KEY};
use xenadmin_gpui::console::windows::{MINIMUM_SIZE, ROOT_KEY};
use xenadmin_gpui::console::{self, ConsoleCanvas, ConsoleWindows};
use xenadmin_gpui::intents::{self, Hooks};
use xenadmin_gpui::kit::testing::{
    TestAppContext, TestEventExt as _, TestWindowExt as _, VisualTestContext,
};
use xenadmin_gpui::kit::*;
use xenadmin_gpui::scenes::{self, Options, ShellWindow};
use xenadmin_gpui::store::AppStore;
use xenadmin_ui::Model;
use xenadmin_ui::areas::console as area;
use xenadmin_ui::areas::console::canvas as seam;
use xenadmin_ui::areas::console::{Seat, view, windows};
use xenadmin_ui::command::CommandId;
use xenadmin_ui::intent::{ConsoleWindowId, Intent};
use xenadmin_ui::tabs::TabId;

const WIDE: (f32, f32) = (1440., 900.);

fn options((width, height): (f32, f32)) -> Options {
    Options {
        width,
        height,
        ..Options::default()
    }
}

/// Draws until what the last event caused is on screen.
fn settle(cx: &mut TestAppContext, window: AnyWindowHandle) {
    for _ in 0..3 {
        cx.update_window(window, |_, window, cx| window.render_frame(cx))
            .expect("window");
        cx.run_until_parked();
    }
}

/// Opens the whole window of a scene and makes it the active window: GPUI
/// reports no focus change of an inactive window.
fn open(cx: &mut TestAppContext, name: &str, size: (f32, f32)) -> ShellWindow {
    let setup = scenes::setup(name).unwrap_or_else(|| panic!("no scene is named {name}"));
    let scene = cx
        .update(|cx| scenes::open_shell(cx, setup, &options(size)))
        .expect("shell window");
    cx.run_until_parked();
    cx.update_window(scene.window, |_, window, _| window.activate_window())
        .expect("window");
    settle(cx, scene.window);
    scene
}

fn on<R>(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    act: impl FnOnce(&mut Window, &mut App) -> R,
) -> R {
    let result = cx
        .update_window(window, |_, window, cx| act(window, cx))
        .expect("window");
    cx.run_until_parked();
    settle(cx, window);
    result
}

fn run(cx: &mut TestAppContext, store: &Entity<AppStore>, act: impl FnOnce(&mut Model)) {
    cx.update(|cx| store.update(cx, |store, cx| store.run(act, cx)));
    cx.run_until_parked();
}

fn invoke(cx: &mut TestAppContext, store: &Entity<AppStore>, id: &'static str) {
    cx.update(|cx| store.update(cx, |store, cx| store.invoke(CommandId(id), cx)));
    cx.run_until_parked();
}

fn model<R>(
    cx: &mut TestAppContext,
    store: &Entity<AppStore>,
    read: impl FnOnce(&Model) -> R,
) -> R {
    cx.update(|cx| read(store.read(cx).model()))
}

fn canvas(cx: &mut TestAppContext, window: AnyWindowHandle, seat: Seat) -> Entity<ConsoleCanvas> {
    cx.update(|cx| console::canvas_of(window.window_id(), seat, cx))
        .unwrap_or_else(|| panic!("no canvas of {seat:?} was drawn in the window"))
}

fn canvas_bounds(cx: &mut TestAppContext, window: AnyWindowHandle) -> Bounds<Pixels> {
    on(cx, window, |window, _| window.find(view::CANVAS).bounds())
}

/// The evdev codes the canvas forwarded, with their direction.
fn codes(cx: &mut TestAppContext, canvas: &Entity<ConsoleCanvas>) -> Vec<(u32, bool)> {
    canvas.read_with(cx, |canvas, _| {
        canvas
            .sent_keys()
            .iter()
            .filter_map(|key| match key {
                SentKey::Code { code, down, .. } => Some((*code, *down)),
                SentKey::Keysym { .. } => None,
            })
            .collect()
    })
}

fn extent(bounds: Bounds<Pixels>) -> [f32; 2] {
    [f32::from(bounds.size.width), f32::from(bounds.size.height)]
}

fn at(bounds: Bounds<Pixels>, x: f32, y: f32) -> Point<Pixels> {
    bounds.origin + point(px(x), px(y))
}

fn press(cx: &mut TestAppContext, window: AnyWindowHandle, at: Point<Pixels>, button: MouseButton) {
    on(cx, window, |window, cx| {
        window.pointer_move(at, None, cx);
        window.pointer_down(at, button, cx);
    });
}

fn release(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    at: Point<Pixels>,
    button: MouseButton,
) {
    on(cx, window, |window, cx| window.pointer_up(at, button, cx));
}

fn scale(cx: &mut TestAppContext, store: &Entity<AppStore>) -> u16 {
    model(cx, store, |m| m.preferences.options.interface_scale)
}

// ---------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------

/// Rune `the_console_canvas_is_laid_out_in_fit_and_actual_size_modes`, and
/// its zero-width regression.
#[gpui_kit::test]
fn the_canvas_is_laid_out_in_fit_and_actual_size(cx: &mut TestAppContext) {
    let fit = open(cx, "console-frame", WIDE);
    let fit_bounds = canvas_bounds(cx, fit.window);
    assert!(fit_bounds.size.width > px(400.), "{fit_bounds:?}");
    assert!(fit_bounds.size.height > px(300.), "{fit_bounds:?}");
    let input = canvas(cx, fit.window, Seat::Tab).read_with(cx, |c, _| c.input().bounds());
    assert_eq!(input, extent(fit_bounds));
    let (role, label) = on(cx, fit.window, |window, _| {
        let element = window.find(view::CANVAS);
        (element.role(), element.label().map(str::to_owned))
    });
    assert_eq!(role, Some(Role::Application));
    assert_eq!(label.as_deref(), Some(console::canvas::LABEL));

    let actual = open(cx, "console-actual-size", WIDE);
    let (actual_bounds, frame) = on(cx, actual.window, |window, _| {
        (
            window.find(view::CANVAS).bounds(),
            window.find(VIEW_KEY).bounds(),
        )
    });
    assert_eq!(extent(actual_bounds), [800., 600.]);
    // Both modes start at the same place, and at Actual size the canvas is
    // at the origin of the view that pans: just inside its border.
    assert_eq!(actual_bounds.origin, fit_bounds.origin);
    assert_eq!(actual_bounds.origin, frame.origin + point(px(1.), px(1.)));
    // The view takes the height that is left, not the guest's.
    assert!(frame.size.height < px(600.), "{frame:?}");
    assert!(frame.size.height > px(300.), "{frame:?}");
    let input = canvas(cx, actual.window, Seat::Tab).read_with(cx, |c, _| c.input().bounds());
    assert_eq!(input, [800., 600.]);
}

#[gpui_kit::test]
fn a_console_without_a_picture_says_so(cx: &mut TestAppContext) {
    let scene = open(cx, "console-recovery", WIDE);
    let (empty, bounds) = on(cx, scene.window, |window, _| {
        (
            window.find(EMPTY_KEY).label().map(str::to_owned),
            window.find(view::CANVAS).bounds(),
        )
    });
    assert_eq!(empty.as_deref(), Some(console::canvas::EMPTY_TEXT));
    assert!(bounds.size.height > px(300.), "{bounds:?}");
    let canvas = canvas(cx, scene.window, Seat::Tab);
    canvas.read_with(cx, |canvas, _| {
        assert!(canvas.image().is_none());
        assert_eq!(canvas.uploaded(), 0);
        assert_eq!(canvas.uploads(), (0, 0));
    });
}

// ---------------------------------------------------------------------------
// The picture
// ---------------------------------------------------------------------------

#[gpui_kit::test]
fn a_frame_is_uploaded_once_per_generation_and_the_replaced_image_is_dropped(
    cx: &mut TestAppContext,
) {
    let scene = open(cx, "console-frame", WIDE);
    let canvas = canvas(cx, scene.window, Seat::Tab);
    let generation =
        |cx: &mut TestAppContext| model(cx, &scene.store, |m| seam::frame_generation(m, Seat::Tab));
    let in_atlas = |cx: &mut TestAppContext, image: &std::sync::Arc<RenderImage>| {
        on(cx, scene.window, |window, _| {
            window.has_image_atlas_entry(image)
        })
    };

    let first_generation = generation(cx);
    assert_ne!(first_generation, 0);
    let first = canvas.read_with(cx, |canvas, _| {
        assert_eq!(canvas.uploaded(), first_generation);
        assert_eq!(canvas.uploads(), (1, 0));
        canvas.image().cloned().expect("an uploaded picture")
    });
    assert!(in_atlas(cx, &first));

    // Drawing again, and an update that leaves the picture alone, upload
    // nothing.
    settle(cx, scene.window);
    run(cx, &scene.store, |m| m.set_active_tab(TabId::Console));
    settle(cx, scene.window);
    assert_eq!(canvas.read_with(cx, |canvas, _| canvas.uploads()), (1, 0));

    // The tab's console is replaced by another: the first frame drawn after
    // that shows the new picture, and the old one has left the atlas.
    cx.update(|cx| {
        scene
            .store
            .update(cx, |store, cx| store.run(area::scenes::frame_scene, cx));
    });
    cx.update_window(scene.window, |_, window, cx| window.render_frame(cx))
        .expect("window");
    let second_generation = generation(cx);
    assert_ne!(second_generation, first_generation);
    let second = canvas.read_with(cx, |canvas, _| {
        assert_eq!(canvas.uploaded(), second_generation);
        assert_eq!(canvas.uploads(), (2, 1));
        canvas.image().cloned().expect("the new picture")
    });
    assert!(!in_atlas(cx, &first), "the replaced image was not dropped");
    assert!(in_atlas(cx, &second));

    // The console is closed: generation 0, nothing is kept.
    run(cx, &scene.store, area::close_tab);
    settle(cx, scene.window);
    assert_eq!(generation(cx), 0);
    canvas.read_with(cx, |canvas, _| {
        assert!(canvas.image().is_none());
        assert_eq!(canvas.uploaded(), 0);
        assert_eq!(canvas.uploads(), (2, 2));
    });
    assert!(!in_atlas(cx, &second));
}

/// `Model::poll` returns false for a new frame alone. The canvas looks
/// after every poll and draws itself again; the store notifies no one, so
/// the shell is not drawn again.
#[gpui_kit::test]
fn a_poll_with_only_a_new_frame_repaints_the_canvas_and_not_the_shell(cx: &mut TestAppContext) {
    let scene = open(cx, "console-frame", WIDE);
    let canvas = canvas(cx, scene.window, Seat::Tab);
    let before = canvas.read_with(cx, |canvas, _| canvas.uploaded());
    let notified = Rc::new(Cell::new(0));
    cx.update(|cx| {
        let notified = notified.clone();
        cx.observe(&scene.store, move |_, _| notified.set(notified.get() + 1))
            .detach();
    });
    // A poll before the change, so that what a first poll of the scene
    // changes is not counted.
    cx.update(|cx| scene.store.update(cx, |store, cx| store.poll(cx)));
    cx.run_until_parked();
    let baseline = notified.get();

    cx.update(|cx| {
        scene.store.update(cx, |store, cx| {
            // Another picture arrives, as the console worker's would.
            store.take(area::scenes::frame_scene);
            store.poll(cx);
        });
    });
    cx.run_until_parked();
    assert_eq!(notified.get(), baseline, "the poll notified the shell");
    let generation = model(cx, &scene.store, |m| seam::frame_generation(m, Seat::Tab));
    assert_ne!(generation, before);
    canvas.read_with(cx, |canvas, _| {
        assert_eq!(canvas.uploaded(), generation);
        assert_eq!(canvas.uploads(), (2, 1));
    });
}

// ---------------------------------------------------------------------------
// The pointer
// ---------------------------------------------------------------------------

#[gpui_kit::test]
fn a_press_on_the_letterboxed_picture_reaches_the_guest_pixel_under_it(cx: &mut TestAppContext) {
    let scene = open(cx, "console-frame", WIDE);
    let bounds = canvas_bounds(cx, scene.window);
    let canvas = canvas(cx, scene.window, Seat::Tab);
    let [width, height] = extent(bounds);
    let guest = |cx: &mut TestAppContext| {
        let (pointer, extent) =
            canvas.read_with(cx, |c, _| (c.input().pointer(), c.input().bounds()));
        model(cx, &scene.store, |m| {
            seam::geometry(m, Seat::Tab, extent)
                .expect("geometry")
                .position(pointer[0], pointer[1])
        })
    };

    // The centre of the canvas is the centre of the 800 by 600 guest.
    let centre = at(bounds, width / 2., height / 2.);
    press(cx, scene.window, centre, MouseButton::Left);
    canvas.read_with(cx, |canvas, _| {
        assert_eq!(canvas.input().buttons(), 1);
        assert_eq!(canvas.input().pointer(), [width / 2., height / 2.]);
    });
    assert_eq!(guest(cx), Some([400, 300]));
    let focused = on(cx, scene.window, |window, cx| {
        canvas.read(cx).focus().is_focused(window)
    });
    assert!(focused, "a press gives the canvas the keyboard");
    release(cx, scene.window, centre, MouseButton::Left);
    assert_eq!(canvas.read_with(cx, |c, _| c.input().buttons()), 0);

    // The corners of the picture, which is letterboxed left and right.
    let geometry = model(cx, &scene.store, |m| {
        seam::geometry(m, Seat::Tab, [width, height]).expect("geometry")
    });
    assert!(geometry.offset[0] > 100., "{geometry:?}");
    let [painted_width, painted_height] = geometry.painted();
    let corner = at(bounds, geometry.offset[0] + 0.25, geometry.offset[1] + 0.25);
    press(cx, scene.window, corner, MouseButton::Left);
    assert_eq!(guest(cx), Some([0, 0]));
    release(cx, scene.window, corner, MouseButton::Left);
    let far = at(
        bounds,
        geometry.offset[0] + painted_width - 0.25,
        geometry.offset[1] + painted_height - 0.25,
    );
    press(cx, scene.window, far, MouseButton::Left);
    assert_eq!(guest(cx), Some([799, 599]));
    release(cx, scene.window, far, MouseButton::Left);

    // A right button is a real press: down, a drag, up. The drag leaves the
    // canvas and its release still arrives.
    press(cx, scene.window, centre, MouseButton::Right);
    assert_eq!(canvas.read_with(cx, |c, _| c.input().buttons()), 4);
    let outside = point(px(4.), px(4.));
    on(cx, scene.window, |window, cx| {
        window.pointer_move(outside, Some(MouseButton::Right), cx);
    });
    canvas.read_with(cx, |canvas, _| {
        assert_eq!(canvas.input().buttons(), 4);
        let pointer = canvas.input().pointer();
        assert!(pointer[0] < 0. && pointer[1] < 0., "{pointer:?}");
    });
    release(cx, scene.window, outside, MouseButton::Right);
    assert_eq!(canvas.read_with(cx, |c, _| c.input().buttons()), 0);
}

#[gpui_kit::test]
fn at_actual_size_a_canvas_point_is_the_same_guest_pixel(cx: &mut TestAppContext) {
    let scene = open(cx, "console-actual-size", WIDE);
    let bounds = canvas_bounds(cx, scene.window);
    let canvas = canvas(cx, scene.window, Seat::Tab);
    press(cx, scene.window, at(bounds, 100.5, 50.5), MouseButton::Left);
    let (pointer, extent) = canvas.read_with(cx, |c, _| (c.input().pointer(), c.input().bounds()));
    let guest = model(cx, &scene.store, |m| {
        seam::geometry(m, Seat::Tab, extent)
            .expect("geometry")
            .position(pointer[0], pointer[1])
    });
    assert_eq!(guest, Some([100, 50]));
}

/// Rune `a_console_window_is_drawn_at_the_interface_size_and_follows_changes_to_it`:
/// the pointer stays canvas-local at another interface size.
#[gpui_kit::test]
fn the_pointer_stays_canvas_local_at_a_larger_interface_size(cx: &mut TestAppContext) {
    let scene = open(cx, "console-frame", WIDE);
    let before = canvas_bounds(cx, scene.window);
    run(cx, &scene.store, |m| {
        m.preferences.options.interface_scale = 150;
    });
    settle(cx, scene.window);
    let bounds = canvas_bounds(cx, scene.window);
    assert_ne!(bounds, before, "the larger interface moved nothing");
    let [width, height] = extent(bounds);
    let canvas = canvas(cx, scene.window, Seat::Tab);
    assert_eq!(
        canvas.read_with(cx, |c, _| c.input().bounds()),
        [width, height]
    );
    press(
        cx,
        scene.window,
        at(bounds, width * 0.25, height * 0.75),
        MouseButton::Left,
    );
    let pointer = canvas.read_with(cx, |c, _| c.input().pointer());
    assert!((pointer[0] - width * 0.25).abs() < 0.01, "{pointer:?}");
    assert!((pointer[1] - height * 0.75).abs() < 0.01, "{pointer:?}");
}

// ---------------------------------------------------------------------------
// The keys
// ---------------------------------------------------------------------------

#[gpui_kit::test]
fn every_key_is_the_guest_s_while_the_canvas_has_the_keyboard(cx: &mut TestAppContext) {
    let scene = open(cx, "console-frame", WIDE);
    let bounds = canvas_bounds(cx, scene.window);
    let canvas = canvas(cx, scene.window, Seat::Tab);
    let size = scale(cx, &scene.store);
    let title = scene
        .view
        .read_with(cx, |shell, _| shell.title().to_owned());

    let centre = at(bounds, 20., 20.);
    press(cx, scene.window, centre, MouseButton::Left);
    release(cx, scene.window, centre, MouseButton::Left);
    on(cx, scene.window, |window, cx| {
        for key in [
            "ctrl-=", "ctrl-0", "alt-left", "ctrl-q", "ctrl-n", "tab", "escape", "a",
        ] {
            window.press(key, cx);
        }
        window.change_modifiers(Modifiers::control(), cx);
        window.change_modifiers(Modifiers::default(), cx);
    });

    // Each key went to the seam by its evdev code, down and up.
    let sent = codes(cx, &canvas);
    for code in [13, 11, 105, 16, 49, 15, 1, 30, 29] {
        assert!(sent.contains(&(code, true)), "no press of {code}: {sent:?}");
        assert!(
            sent.contains(&(code, false)),
            "no release of {code}: {sent:?}"
        );
    }
    // And nothing of the application happened: no interface size, no
    // navigation, no exit, no new VM, no focus traversal.
    assert_eq!(scale(cx, &scene.store), size);
    assert_eq!(
        scene
            .view
            .read_with(cx, |shell, _| shell.title().to_owned()),
        title
    );
    assert_eq!(model(cx, &scene.store, Model::active_tab), TabId::Console);
    assert!(cx.update(|cx| scene.store.read(cx).surfaces().is_empty()));
    let quit = cx.update(|cx| {
        scene
            .store
            .read(cx)
            .recorded_intents()
            .iter()
            .any(|intent| matches!(intent, Intent::Quit))
    });
    assert!(!quit, "ctrl-q reached File > Exit");
    let focused = on(cx, scene.window, |window, cx| {
        canvas.read(cx).focus().is_focused(window)
    });
    assert!(focused, "Tab moved the focus out of the console");

    // A character the US table has no key for goes as a keysym.
    on(cx, scene.window, |window, cx| window.press("é", cx));
    let keysyms: Vec<SentKey> = canvas.read_with(cx, |canvas, _| {
        canvas
            .sent_keys()
            .iter()
            .copied()
            .filter(|key| matches!(key, SentKey::Keysym { .. }))
            .collect()
    });
    assert_eq!(
        keysyms,
        vec![
            SentKey::Keysym {
                keysym: 0xe9,
                down: true
            },
            SentKey::Keysym {
                keysym: 0xe9,
                down: false
            },
        ]
    );

    // The focus leaves the canvas: the shortcuts are the application's
    // again, and the canvas hears nothing.
    let count = canvas.read_with(cx, |canvas, _| canvas.sent_keys().len());
    on(cx, scene.window, |window, cx| {
        scene.view.read(cx).focus_handle(cx).focus(window, cx);
    });
    on(cx, scene.window, |window, cx| window.press("ctrl-=", cx));
    assert!(scale(cx, &scene.store) > size, "ctrl-= did nothing");
    assert_eq!(
        canvas.read_with(cx, |canvas, _| canvas.sent_keys().len()),
        count
    );
}

#[gpui_kit::test]
fn losing_the_keyboard_releases_what_the_console_holds(cx: &mut TestAppContext) {
    let scene = open(cx, "console-frame", WIDE);
    let bounds = canvas_bounds(cx, scene.window);
    let canvas = canvas(cx, scene.window, Seat::Tab);
    let centre = at(bounds, 30., 30.);
    let epoch = |cx: &mut TestAppContext| {
        model(cx, &scene.store, |m| {
            area::session(m, Seat::Tab).expect("session").input_epoch()
        })
    };
    let hold = |cx: &mut TestAppContext| {
        press(cx, scene.window, centre, MouseButton::Right);
        let (buttons, focused) = on(cx, scene.window, |window, cx| {
            let canvas = canvas.read(cx);
            (canvas.input().buttons(), canvas.focus().is_focused(window))
        });
        assert_eq!(buttons, 4);
        assert!(focused);
    };

    // Another element takes the focus.
    hold(cx);
    let before = epoch(cx);
    on(cx, scene.window, |window, cx| {
        scene.view.read(cx).focus_handle(cx).focus(window, cx);
    });
    assert_ne!(epoch(cx), before, "blur did not release the guest's input");
    assert_eq!(canvas.read_with(cx, |c, _| c.input().buttons()), 0);

    // The window is deactivated.
    hold(cx);
    let before = epoch(cx);
    VisualTestContext::from_window(scene.window, cx).deactivate_window();
    cx.run_until_parked();
    settle(cx, scene.window);
    assert_ne!(
        epoch(cx),
        before,
        "deactivation did not release the guest's input"
    );
    canvas.read_with(cx, |canvas, _| {
        assert_eq!(canvas.input().buttons(), 0);
        assert!(!canvas.has_keyboard());
    });
    on(cx, scene.window, |window, _| window.activate_window());

    // The Console tab is left: the canvas is no longer drawn.
    hold(cx);
    let before = epoch(cx);
    run(cx, &scene.store, |m| m.set_active_tab(TabId::General));
    settle(cx, scene.window);
    assert_ne!(
        epoch(cx),
        before,
        "leaving the tab did not release the guest's input"
    );
    assert_eq!(canvas.read_with(cx, |c, _| c.input().buttons()), 0);
    let drawn = on(cx, scene.window, |window, _| {
        window.try_find(view::CANVAS).is_some()
    });
    assert!(!drawn);
}

// ---------------------------------------------------------------------------
// The detached windows
// ---------------------------------------------------------------------------

/// A scene's main window with console windows that are really opened: the
/// application's console hooks perform the model's requests, those the
/// scene queued included.
fn open_with_windows(cx: &mut TestAppContext, name: &str) -> (ShellWindow, ConsoleWindows) {
    let scene = open(cx, name, WIDE);
    let consoles = cx.update(|cx| {
        let consoles = ConsoleWindows::new(&scene.store);
        // What was recorded before the hooks existed.
        let recorded = scene
            .store
            .update(cx, |store, _| store.take_recorded_intents());
        intents::install(&scene.store, consoles.hooks(Hooks::default()), cx);
        consoles.install(Some(scene.window), cx);
        for intent in recorded {
            if let Intent::ConsoleWindow(request) = intent {
                consoles.perform(request, cx);
            }
        }
        consoles
    });
    // What is still in the model's outbox leaves with the next update.
    run(cx, &scene.store, |_| {});
    settle(cx, scene.window);
    (scene, consoles)
}

fn console_window(
    cx: &mut TestAppContext,
    consoles: &ConsoleWindows,
    id: u32,
) -> (AnyWindowHandle, Entity<console::ConsoleWindowView>) {
    let (handle, view) = consoles
        .window(ConsoleWindowId(id))
        .unwrap_or_else(|| panic!("console window {id} is not open"));
    settle(cx, handle);
    (handle, view)
}

#[gpui_kit::test]
fn a_detached_window_opens_from_the_intent_and_its_close_reaches_the_model(
    cx: &mut TestAppContext,
) {
    let (scene, consoles) = open_with_windows(cx, "console-window");
    assert_eq!(consoles.open_ids(), vec![ConsoleWindowId(1)]);
    assert_eq!(cx.windows().len(), 2);
    let (handle, view) = console_window(cx, &consoles, 1);
    let lease = model(cx, &scene.store, |m| windows::lease(m, ConsoleWindowId(1)));
    assert!(lease.is_some());

    // The title is the model's, and the window shows the model's controls
    // and the canvas of its own seat.
    let title = model(cx, &scene.store, |m| {
        view::window(m, ConsoleWindowId(1)).title
    });
    assert!(title.ends_with(view::WINDOW_TITLE), "{title}");
    assert_eq!(view.read_with(cx, |view, _| view.title().to_owned()), title);
    assert_eq!(
        VisualTestContext::from_window(handle, cx).window_title(),
        Some(title)
    );
    on(cx, handle, |window, _| {
        for key in [
            "close-console",
            "console-status",
            "console-auto-reconnect",
            "reconnect-console",
            "console-actual-size",
            "console-fullscreen",
            "console-release-input",
            "save-console-screenshot",
            view::CANVAS,
            "console-help",
        ] {
            assert!(window.try_find(key).is_some(), "{key} is not in the window");
        }
        assert!(window.try_find("detach-console").is_none());
    });
    let canvas = canvas(cx, handle, Seat::Window(ConsoleWindowId(1)));
    canvas.read_with(cx, |canvas, _| {
        assert!(canvas.image().is_some());
        assert_eq!(canvas.uploads(), (1, 0));
    });
    // The main window shows the VM's Console tab without a console.
    let closed = on(cx, scene.window, |window, _| {
        window.try_find(view::CLOSED).is_some() && window.try_find(view::CANVAS).is_none()
    });
    assert!(closed);

    // The user closes the OS window: the model hears of it with the lease,
    // which ends the console and frees the slot.
    assert!(VisualTestContext::from_window(handle, cx).simulate_close());
    cx.run_until_parked();
    assert!(consoles.open_ids().is_empty());
    model(cx, &scene.store, |m| {
        assert_eq!(windows::lease(m, ConsoleWindowId(1)), None);
        assert!(windows::open_windows(m).is_empty());
        assert!(area::session(m, Seat::Window(ConsoleWindowId(1))).is_none());
    });
}

/// Rune `independent_console_windows_keep_targets_frames_and_input_state_separate`,
/// the GPUI half.
#[gpui_kit::test]
fn two_console_windows_keep_pictures_and_input_apart(cx: &mut TestAppContext) {
    let (scene, consoles) = open_with_windows(cx, "multiple-consoles");
    assert_eq!(
        consoles.open_ids(),
        vec![ConsoleWindowId(1), ConsoleWindowId(2)]
    );
    assert_eq!(cx.windows().len(), 3);
    let (first, first_view) = console_window(cx, &consoles, 1);
    let (second, second_view) = console_window(cx, &consoles, 2);
    let first_title = first_view.read_with(cx, |view, _| view.title().to_owned());
    let second_title = second_view.read_with(cx, |view, _| view.title().to_owned());
    assert!(first_title.ends_with(view::WINDOW_TITLE), "{first_title}");
    assert!(second_title.ends_with(view::WINDOW_TITLE), "{second_title}");
    assert_ne!(first_title, second_title);

    let first_canvas = canvas(cx, first, Seat::Window(ConsoleWindowId(1)));
    let second_canvas = canvas(cx, second, Seat::Window(ConsoleWindowId(2)));
    let first_picture = first_canvas.read_with(cx, |canvas, _| canvas.uploaded());
    let second_picture = second_canvas.read_with(cx, |canvas, _| canvas.uploaded());
    assert_ne!(first_picture, 0);
    assert_ne!(second_picture, 0);
    assert_ne!(first_picture, second_picture);

    // Keys typed into one window reach only its seat.
    on(cx, first, |window, _| window.activate_window());
    let bounds = canvas_bounds(cx, first);
    press(cx, first, at(bounds, 10., 10.), MouseButton::Left);
    release(cx, first, at(bounds, 10., 10.), MouseButton::Left);
    on(cx, first, |window, cx| window.press("a", cx));
    assert_eq!(codes(cx, &first_canvas), vec![(30, true), (30, false)]);
    assert!(codes(cx, &second_canvas).is_empty());
    // The button a window holds is that window's.
    press(cx, first, at(bounds, 10., 10.), MouseButton::Left);
    assert_eq!(first_canvas.read_with(cx, |c, _| c.input().buttons()), 1);
    assert_eq!(second_canvas.read_with(cx, |c, _| c.input().buttons()), 0);
    release(cx, first, at(bounds, 10., 10.), MouseButton::Left);

    // With the canvas not focused the interface size shortcut works in a
    // console window too, on the one model.
    let size = scale(cx, &scene.store);
    on(cx, first, |window, cx| {
        first_view.read(cx).focus_handle(cx).focus(window, cx);
    });
    on(cx, first, |window, cx| window.press("ctrl-=", cx));
    assert!(scale(cx, &scene.store) > size);

    // The OS close button of one window ends its console and leaves the
    // other.
    assert!(VisualTestContext::from_window(first, cx).simulate_close());
    cx.run_until_parked();
    assert_eq!(consoles.open_ids(), vec![ConsoleWindowId(2)]);
    model(cx, &scene.store, |m| {
        assert_eq!(windows::lease(m, ConsoleWindowId(1)), None);
        assert!(windows::lease(m, ConsoleWindowId(2)).is_some());
    });

    // Window > Close All Consoles closes the rest, and the model is told:
    // the slots are free, so the next console takes the first.
    invoke(cx, &scene.store, "window.console.close-all");
    cx.run_until_parked();
    assert!(consoles.open_ids().is_empty());
    assert!(model(cx, &scene.store, |m| windows::open_windows(m).is_empty()));
    run(cx, &scene.store, area::scenes::window_scene);
    cx.run_until_parked();
    assert_eq!(consoles.open_ids(), vec![ConsoleWindowId(1)]);
}

/// The scene tool's artifact of `console-window`: window 1 alone.
#[gpui_kit::test]
fn the_console_window_scene_shows_every_control_inside_the_window(cx: &mut TestAppContext) {
    for size in [(1100., 800.), (513., 652.), MINIMUM_SIZE] {
        let setup = scenes::setup(scenes::CONSOLE_WINDOW_SCENE).expect("scene");
        let scene = cx
            .update(|cx| scenes::open_console_window(cx, setup, &options(size), ConsoleWindowId(1)))
            .expect("console window");
        cx.run_until_parked();
        settle(cx, scene.window);
        // The scene's own Open request was not performed: this is the only
        // window.
        assert_eq!(cx.windows().len(), 1);
        let tree = on(cx, scene.window, |window, cx| {
            scenes::console_window_tree_json(window, cx, &scene.store, scene.id)
        });
        assert!(scenes::missing_keys(&tree).is_empty(), "{size:?}: {tree:#}");
        on(cx, scene.window, |window, _| {
            let viewport = window.viewport_size();
            assert!(window.try_find(ROOT_KEY).is_some());
            assert!(window.try_find("detach-console").is_none());
            // Every control is inside the window. The help text under the
            // canvas is in the tree; with the test platform's wider font it
            // is reached by scrolling in the smallest windows.
            for key in [
                "close-console",
                "console-status",
                "console-auto-reconnect",
                "reconnect-console",
                "console-actual-size",
                "console-fullscreen",
                "console-release-input",
                "save-console-screenshot",
                "console-ctrl-alt-delete",
                "console-ctrl-alt-f1",
                "console-alt-tab",
                "send-console-clipboard",
                "type-console-clipboard",
                "copy-console-clipboard",
            ] {
                let bounds = window.find(key).bounds();
                assert!(
                    bounds.left() >= px(0.)
                        && bounds.top() >= px(0.)
                        && bounds.right() <= viewport.width
                        && bounds.bottom() <= viewport.height,
                    "{key} is outside the {size:?} window: {bounds:?}"
                );
            }
            assert!(window.try_find("console-help").is_some());
            let canvas = window.find(view::CANVAS).bounds();
            assert!(canvas.size.height >= px(100.), "{size:?}: {canvas:?}");
            assert!(
                canvas.top() < viewport.height && canvas.right() <= viewport.width,
                "{size:?}: {canvas:?}"
            );
            if size == (1100., 800.) {
                let help = window.find("console-help").bounds();
                assert!(help.bottom() <= viewport.height, "{help:?}");
                assert!(canvas.size.height > px(300.), "{canvas:?}");
            }
        });
        cx.update_window(scene.window, |_, window, _| window.remove_window())
            .expect("window");
        cx.run_until_parked();
    }
}

#[gpui_kit::test]
fn fullscreen_toggles_the_window_of_the_console(cx: &mut TestAppContext) {
    let (scene, consoles) = open_with_windows(cx, "console-window");
    let (handle, _) = console_window(cx, &consoles, 1);
    let fullscreen = |cx: &mut TestAppContext, window: AnyWindowHandle| {
        cx.update_window(window, |_, window, _| window.is_fullscreen())
            .expect("window")
    };
    assert!(!fullscreen(cx, handle));

    // The window's own button.
    on(cx, handle, |window, cx| {
        window.click("console-fullscreen", cx)
    });
    assert!(fullscreen(cx, handle));
    assert!(!fullscreen(cx, scene.window));
    on(cx, handle, |window, cx| {
        window.click("console-fullscreen", cx)
    });
    assert!(!fullscreen(cx, handle));

    // The Console tab's command toggles the main window.
    run(cx, &scene.store, area::scenes::frame_scene);
    settle(cx, scene.window);
    invoke(cx, &scene.store, "console.fullscreen");
    assert!(fullscreen(cx, scene.window));
    assert!(!fullscreen(cx, handle));
    invoke(cx, &scene.store, "console.fullscreen");
    assert!(!fullscreen(cx, scene.window));
}

/// Rune `chooser_captures_one_frame_and_cancel_or_console_switch_does_not_write`,
/// the GPUI half.
#[gpui_kit::test]
fn save_screenshot_asks_for_a_path_and_writes_a_png(cx: &mut TestAppContext) {
    let scene = open(cx, "console-frame", WIDE);
    cx.update(|cx| intents::install(&scene.store, Hooks::default(), cx));

    // Cancelled: nothing is written and the command can be used again.
    invoke(cx, &scene.store, "console.save-screenshot");
    assert!(cx.did_prompt_for_new_path());
    cx.simulate_new_path_selection(|_| None);
    cx.run_until_parked();

    let directory = std::env::temp_dir().join(format!(
        "xenadmin-gpui-console-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&directory).expect("scratch directory");
    let path = directory.join("vm-console.png");
    let _ = std::fs::remove_file(&path);
    invoke(cx, &scene.store, "console.save-screenshot");
    assert!(
        cx.did_prompt_for_new_path(),
        "the command stayed disabled after a cancel"
    );
    let chosen = path.clone();
    cx.simulate_new_path_selection(move |_| Some(chosen));
    cx.run_until_parked();
    // The model writes on a thread of its own: wait for the file.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let written = loop {
        let bytes = std::fs::read(&path).unwrap_or_default();
        if bytes.starts_with(b"\x89PNG") || std::time::Instant::now() > deadline {
            break bytes;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    assert!(
        written.starts_with(b"\x89PNG"),
        "no PNG was written to {}",
        path.display()
    );
    let _ = std::fs::remove_dir_all(&directory);
}
