//! Spike S2: headless scene dump.
//!
//! Build with `--features scenes`. Two element-tree dumps of one scene are
//! compared. Screenshot capture is attempted and the error is printed when
//! the published platform crate does not provide the renderer the plan names.
//!
//! Headless output does not establish Wayland, clipboard, keyboard, or Orca
//! behavior.

use std::sync::Arc;

use gpui_kit::assets::Assets;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::prelude::*;
use gpui_kit::test::{ElementSnapshot, TestQueryExt, TestWindowExt};
use gpui_kit::{
    Bounds, TestSupportExt as _, Window, WindowBounds, WindowOptions, div, point, px, size,
};

struct Scene;

impl Render for Scene {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("spike-scene")
            .test_support()
            .size_full()
            .child(Button::new("spike-button").label("Add Server").primary())
    }
}

fn dump(window: &Window) -> String {
    let mut lines = Vec::new();
    for element in window.elements() {
        lines.push(describe(&element));
    }
    lines.sort();
    lines.join("\n")
}

fn describe(element: &ElementSnapshot) -> String {
    let path: Vec<String> = element.path().iter().map(|id| format!("{id:?}")).collect();
    format!(
        "path={path:?} role={:?} label={:?} value={:?} focused={:?} checked={:?} disabled={:?} bounds={:?}",
        element.role(),
        element.label(),
        element.value(),
        element.focused(),
        element.checked(),
        element.disabled(),
        element.bounds(),
    )
}

fn main() {
    let mut cx = gpui_kit::HeadlessAppContext::with_platform(
        gpui_kit::platform::current_platform(true).text_system(),
        Arc::new(Assets),
        // gpui-pre-linux 0.3.12 exports `current_platform` and `linux_platform`
        // only. `current_headless_renderer` is not in that public API, so this
        // factory cannot install one.
        || Ok(None),
    );
    cx.update(gpui_kit::init);
    let (handle, _) = cx
        .update(|cx| {
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
                |_, cx| cx.new(|_| Scene),
            )
        })
        .expect("open scene window");
    let first = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            dump(window)
        })
        .expect("first frame");
    let second = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            dump(window)
        })
        .expect("second frame");
    println!("element_trees_match={}", first == second);
    if first != second {
        println!("--- first ---\n{first}\n--- second ---\n{second}");
        std::process::exit(1);
    }
    println!("elements={}", first.lines().count());
    match cx.capture_screenshot(handle) {
        Ok(image) => {
            let repeated = cx.capture_screenshot(handle).expect("second capture");
            println!(
                "png_bytes={} png_match={}",
                image.as_raw().len(),
                image.as_raw() == repeated.as_raw()
            );
        }
        Err(error) => {
            println!("png=unavailable");
            println!("{error}");
        }
    }
}
