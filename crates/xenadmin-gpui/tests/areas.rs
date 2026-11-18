//! What the area engineers asked to have checked in the real renderer:
//! surfaces of the ported areas at the sizes the main window can have, down
//! to its minimum of 800 by 600, driven through real events on the Kit's
//! headless test platform.
//!
//! These tests do not establish Wayland input or a screen reader: the
//! platform is the Kit's test platform.
#![cfg(feature = "scenes")]

use xenadmin_core::PoolId;
use xenadmin_gpui::kit::testing::{ElementSnapshot, TestAppContext, TestWindowExt as _};
use xenadmin_gpui::kit::*;
use xenadmin_gpui::scenes::{self, Options, Setup, ShellWindow};
use xenadmin_ui::Model;
use xenadmin_ui::areas::connections::{connect, reconnect};
use xenadmin_ui::testing::{Fixture, Server, connected};

/// The sizes a surface must work at: the usual one and the smallest.
const SIZES: [(f32, f32); 2] = [(1440., 900.), (800., 600.)];

fn open(cx: &mut TestAppContext, setup: Setup, (width, height): (f32, f32)) -> ShellWindow {
    let options = Options {
        width,
        height,
        ..Options::default()
    };
    let scene = cx
        .update(|cx| scenes::open_shell(cx, setup, &options))
        .expect("shell window");
    cx.run_until_parked();
    settle(cx, &scene);
    scene
}

fn open_scene(cx: &mut TestAppContext, name: &str, size: (f32, f32)) -> ShellWindow {
    let setup = scenes::setup(name).unwrap_or_else(|| panic!("no scene is named {name}"));
    open(cx, setup, size)
}

/// Draws until what the last event caused is on screen.
fn settle(cx: &mut TestAppContext, scene: &ShellWindow) {
    for _ in 0..3 {
        cx.update_window(scene.window, |_, window, cx| window.render_frame(cx))
            .expect("window");
        cx.run_until_parked();
    }
}

fn on<R>(
    cx: &mut TestAppContext,
    scene: &ShellWindow,
    act: impl FnOnce(&mut Window, &mut App) -> R,
) -> R {
    let result = cx
        .update_window(scene.window, |_, window, cx| act(window, cx))
        .expect("window");
    cx.run_until_parked();
    settle(cx, scene);
    result
}

fn model<R>(cx: &mut TestAppContext, scene: &ShellWindow, read: impl FnOnce(&Model) -> R) -> R {
    cx.update(|cx| read(scene.store.read(cx).model()))
}

/// The element is drawn, and all of it is inside the window: a user can
/// see it and press it without scrolling anything.
#[track_caller]
fn assert_on_screen(window: &Window, element: &ElementSnapshot, what: &str) {
    let viewport = window.viewport_size();
    let bounds = element.bounds();
    assert!(element.visible(), "{what} is not visible");
    assert!(
        bounds.size.width > px(0.) && bounds.size.height > px(0.),
        "{what} has no size: {bounds:?}"
    );
    assert!(
        bounds.left() >= px(0.)
            && bounds.top() >= px(0.)
            && bounds.right() <= viewport.width
            && bounds.bottom() <= viewport.height,
        "{what} is outside the {viewport:?} window: {bounds:?}"
    );
}

/// The title of a surface and its footer buttons are on screen, inside
/// the surface, the title at its top and the footer under the title: the
/// header and the footer keep their place and only the body between them
/// scrolls.
#[track_caller]
fn assert_frame_reachable(window: &Window, surface: &'static str, footer: &[&'static str]) {
    let frame = window.find(surface);
    assert_on_screen(window, &frame, surface);
    let frame = frame.bounds();
    let title = window.find(ElementId::Name(SharedString::from(format!(
        "{surface}/title"
    ))));
    assert_on_screen(window, &title, &format!("the title of {surface}"));
    let title = title.bounds();
    assert!(
        title.top() >= frame.top() && title.bottom() <= frame.bottom(),
        "the title of {surface} is outside it: {title:?} {frame:?}"
    );
    for key in footer {
        let button = window.find(*key);
        assert_on_screen(window, &button, key);
        assert_eq!(button.role(), Some(Role::Button));
        let button = button.bounds();
        assert!(
            button.top() >= title.bottom() && button.bottom() <= frame.bottom() + px(1.),
            "{key} is not at the foot of {surface}: {button:?} {title:?} {frame:?}"
        );
    }
}

// Asked for by the connections area: its Rune test
// `compact_and_wide_editor_keep_header_and_connect_controls_visible` checked
// pixel positions at 513x652, a size the new window cannot have.
#[gpui_kit::test]
fn the_heading_and_the_footer_of_the_connect_dialog_stay_reachable(cx: &mut TestAppContext) {
    for size in SIZES {
        let scene = open_scene(cx, "connect", size);
        on(cx, &scene, |window, _| {
            assert_frame_reachable(window, connect::DIALOG, &["cancel-connect", "connect"]);
        });
        // Its longest form: the saved connection options and the
        // certificate options unfolded (in the model: at the smallest size
        // their buttons are further down the body, which scrolls).
        cx.update(|cx| {
            scene.store.update(cx, |store, cx| {
                store.run(
                    |m| {
                        m.workspace.advanced = true;
                        m.workspace.show_certificate_details = true;
                    },
                    cx,
                );
            });
        });
        cx.run_until_parked();
        settle(cx, &scene);
        on(cx, &scene, |window, cx| {
            assert_frame_reachable(window, connect::DIALOG, &["cancel-connect", "connect"]);
            // Reachable means it can be pressed: Cancel closes the form.
            window.click("cancel-connect", cx);
        });
        assert!(!model(cx, &scene, |m| m.workspace.show_connect), "{size:?}");
        on(cx, &scene, |window, _| {
            assert!(window.try_find("cancel-connect").is_none());
        });
    }
}

// Asked for by the connections area (the Rune
// `compact_reconnect_review_cancel_never_changes_current_session` at
// 513x652). The dialog needs a live session: a real backend against the
// local fixture server.
#[gpui_kit::test]
fn the_heading_and_the_footer_of_the_reconnect_dialog_stay_reachable(cx: &mut TestAppContext) {
    for size in SIZES {
        let pool = PoolId::from(uuid::Uuid::new_v4());
        let server = Server::new(pool);
        let mut fixture = Fixture::new();
        let profile = connected(&mut fixture, &server, pool);
        let mut m = std::mem::take(&mut fixture.model);
        reconnect::open(&mut m, profile.id);
        assert!(m.areas.connections.reconnect.is_some(), "{:?}", m.error);
        let generation = m.workspace.active[&profile.id].latest().generation();

        let scene = open(cx, Setup::model(m), size);
        on(cx, &scene, |window, _| {
            let footer = ["reconnect-cancel", "reconnect-confirm"];
            assert_frame_reachable(window, reconnect::DIALOG, &footer);
            // Nothing is typed: the confirm button says what is missing.
            let confirm = window.find("reconnect-confirm");
            assert_eq!(confirm.disabled(), Some(true));
            assert_eq!(
                confirm.description(),
                Some("Enter the account for this session.")
            );
        });
        // With an account and a password entered (in the model: at the
        // smallest size the fields are further down the body, which
        // scrolls) the footer is where it was, and Cancel can be pressed.
        cx.update(|cx| {
            scene.store.update(cx, |store, cx| {
                store.run(
                    |m| {
                        let editor = m.areas.connections.reconnect.as_mut().expect("the editor");
                        editor.username = "auditor".into();
                        editor.password.set("synthetic-session");
                    },
                    cx,
                );
            });
        });
        cx.run_until_parked();
        settle(cx, &scene);
        on(cx, &scene, |window, cx| {
            assert_frame_reachable(
                window,
                reconnect::DIALOG,
                &["reconnect-cancel", "reconnect-confirm"],
            );
            assert_eq!(window.find("reconnect-confirm").disabled(), None);
            window.click("reconnect-cancel", cx);
        });
        // Cancel changed nothing: the dialog is gone, the session is the
        // one it was, and the server saw one login.
        assert!(model(cx, &scene, |m| m
            .areas
            .connections
            .reconnect
            .is_none()));
        assert_eq!(
            model(cx, &scene, |m| m.workspace.active[&profile.id]
                .latest()
                .generation()),
            generation
        );
        assert!(model(cx, &scene, |m| m
            .workspace
            .session_accounts
            .is_empty()));
        assert_eq!(server.logins().len(), 1, "{size:?}");
        drop(fixture);
    }
}

// Asked for by the host administration area: the confirm button of the
// maintenance review is on screen at both sizes, and in the demo pressing
// it submits nothing.
#[gpui_kit::test]
fn the_confirm_button_of_host_administration_is_on_screen_and_submits_nothing_in_the_demo(
    cx: &mut TestAppContext,
) {
    for size in SIZES {
        let scene = open_scene(cx, "host-administration", size);
        let review = model(cx, &scene, |m| {
            let editor = m
                .areas
                .host_admin
                .editor
                .as_ref()
                .expect("the editor is open");
            editor.review.clone().expect("a review")
        });
        on(cx, &scene, |window, cx| {
            let confirm = window.find("host-confirm");
            assert_on_screen(window, &confirm, "host-confirm");
            assert_eq!(confirm.role(), Some(Role::Button));
            assert_eq!(confirm.label(), Some("Confirm host change"));
            assert_eq!(confirm.disabled(), Some(true));
            assert_eq!(
                confirm.description(),
                Some("Demo mode cannot change infrastructure.")
            );
            window.click("host-confirm", cx);
        });
        // Enter, which presses the default action of a dialog, does no
        // more than the click did.
        on(cx, &scene, |window, cx| window.press("enter", cx));
        let (operations, still_reviewing, error) = model(cx, &scene, |m| {
            let editor = m.areas.host_admin.editor.as_ref();
            (
                m.resource_operations.len() + m.resource_rows.len(),
                editor.and_then(|editor| editor.review.clone()),
                m.error.clone(),
            )
        });
        assert_eq!(operations, 0, "{size:?}: nothing was submitted");
        assert_eq!(still_reviewing, Some(review), "{size:?}: the review stands");
        assert_eq!(error, None);
        on(cx, &scene, |window, _| {
            assert_on_screen(window, &window.find("host-confirm"), "host-confirm");
        });
    }
}
