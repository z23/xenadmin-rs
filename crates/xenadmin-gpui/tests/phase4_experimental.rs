//! Experimental scenes show the notice. The Rune modules are not ported here.
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
                    size: size(px(1100.), px(700.)),
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

#[gpui_kit::test]
fn experimental_scenes_show_the_notice(cx: &mut TestAppContext) {
    for name in [
        "vm-create",
        "guest-tools",
        "storage-create",
        "network-create",
        "ha-policy",
        "ovf-import",
        "alarms",
        "organization",
    ] {
        let window = open(cx, name);
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("experimental-notice").is_some(), "{name}");
        })
        .unwrap_or_else(|_| panic!("{name}"));
    }
}
