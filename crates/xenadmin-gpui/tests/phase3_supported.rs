//! Headless previews of the supported areas. They do not call a host.
#![cfg(feature = "scenes")]

use gpui_kit::prelude::*;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{Bounds, TestAppContext, WindowBounds, WindowOptions, point, px, size};
use xenadmin_gpui::{ShellView, load_scene};

fn open(cx: &mut TestAppContext, name: &str) -> gpui_kit::AnyWindowHandle {
    let state = load_scene(name);
    cx.update(|cx| {
        gpui_kit::init(cx);
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: point(px(0.), px(0.)),
                    size: size(px(1440.), px(900.)),
                })),
                focus: false,
                show: false,
                ..Default::default()
            },
            cx,
            |window, cx| cx.new(|cx| ShellView::new(state, window, cx)),
        )
        .expect("window")
        .0
    })
}

fn shows(cx: &mut TestAppContext, name: &str, marker: &str) {
    let marker = marker.to_string();
    let name = name.to_string();
    let window = open(cx, name.as_str());
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        let found = window.try_find(marker.clone()).is_some();
        assert!(found, "{name} draws {marker}");
    })
    .unwrap_or_else(|_| panic!("{name}"));
}

#[gpui_kit::test]
fn supported_previews_render_without_a_host(cx: &mut TestAppContext) {
    shows(cx, "snapshots", "tab-snapshots");
    shows(cx, "performance", "tab-performance");
    shows(cx, "console-frame", "console-canvas");
    shows(cx, "host-console", "console-canvas");
    shows(cx, "activity", "activity-counters");
    shows(cx, "host-administration", "review-dialog");
    shows(cx, "vm-migration", "migrate-wizard");
}

#[gpui_kit::test]
fn the_move_wizard_advances_without_migrating(cx: &mut TestAppContext) {
    let window = open(cx, "vm-migration");
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.click("migrate-next", cx);
    })
    .expect("next");
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("migrate-review").is_some());
    })
    .expect("review");
}
