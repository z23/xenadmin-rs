//! The application's start on the Kit's test platform: `app::start` builds
//! what `app::main` runs after the launch was decided, over a preview
//! model. Nothing here owns a state directory or reaches a host.
//!
//! What this does not establish: the real platform's window, the desktop's
//! activation of a second start, or the quit itself (the test platform's
//! quit does nothing).
#![cfg(feature = "scenes")]

use xenadmin_desktop::app;
use xenadmin_desktop::kit::testing::{TestAppContext, TestWindowExt as _, VisualTestContext};
use xenadmin_desktop::kit::*;
use xenadmin_desktop::shell;
use xenadmin_ui::Model;

#[gpui_kit::test]
fn the_application_starts_performs_intents_and_answers_a_second_start(cx: &mut TestAppContext) {
    let (requests, activations) = unbounded::<Option<String>>();
    let (window, view) = cx
        .update(|cx| app::start(Model::demo(3, 48), app::DEFAULT_SIZE, Some(activations), cx))
        .expect("the application starts");
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        // The window opens at the default size with the shell in it.
        assert_eq!(
            window.viewport_size(),
            size(px(app::DEFAULT_SIZE.0), px(app::DEFAULT_SIZE.1))
        );
        assert!(window.find(shell::ROOT_KEY).visible());
        assert!(window.find(shell::nav::TREE_KEY).visible());
    })
    .expect("window");
    assert_eq!(
        cx.update(|cx| view.read(cx).title().to_owned()),
        "Demonstration pool"
    );
    assert_eq!(
        VisualTestContext::from_window(window, cx)
            .window_title()
            .as_deref(),
        Some("Demonstration pool - XenAdmin RS")
    );

    // The model's intents are performed by the platform, not recorded.
    let store = cx.update(|cx| view.read(cx).ui().store.clone());
    cx.update(|cx| {
        store.update(cx, |store, cx| {
            store.run(|m| m.open_url("https://docs.xcp-ng.org/"), cx);
        });
    });
    cx.run_until_parked();
    assert_eq!(cx.opened_url().as_deref(), Some("https://docs.xcp-ng.org/"));
    assert!(cx.update(|cx| store.read(cx).recorded_intents().is_empty()));
    // A preview is not live: nothing was restored or connected.
    cx.update(|cx| {
        let model = store.read(cx).model();
        assert!(model.demo && !model.owns_state() && model.backend.is_err());
        assert!(model.waker.is_some(), "the wake bridge is connected");
    });

    // A later start asks this one to show itself.
    let active = |cx: &mut TestAppContext| {
        cx.update_window(window, |_, window, _| window.is_window_active())
            .expect("window")
    };
    assert!(!active(cx), "no window is active on the test platform");
    requests.try_send(None).expect("the request is queued");
    cx.run_until_parked();
    assert!(active(cx));

    // The window manager asks to close: the close path runs and allows it;
    // asked again, it does nothing more.
    let mut visual = VisualTestContext::from_window(window, cx);
    assert!(visual.simulate_close());
    assert!(visual.simulate_close());
}

#[gpui_kit::test]
fn the_main_window_has_the_desktop_entrys_id_and_a_minimum_size(cx: &mut TestAppContext) {
    let options = cx.update(|cx| app::window_options("Lab pool", (1440., 900.), cx));
    // The application id is the name of the installed desktop entry.
    let entry = format!("{}.desktop", app::APP_ID);
    let installed = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../packaging")
        .join(&entry);
    assert!(
        installed.is_file(),
        "{} is not packaged",
        installed.display()
    );
    assert_eq!(options.app_id.as_deref(), Some(app::APP_ID));
    assert_eq!(options.window_min_size, Some(size(px(800.), px(600.))));
    assert_eq!(
        options
            .titlebar
            .as_ref()
            .and_then(|titlebar| titlebar.title.clone())
            .as_deref(),
        Some("Lab pool - XenAdmin RS")
    );
    assert!(options.is_resizable && options.show && options.focus);
    assert_eq!(options.window_decorations, Some(WindowDecorations::Server));
}
