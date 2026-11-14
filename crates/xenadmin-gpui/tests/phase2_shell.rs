//! Headless checks for the shell. They do not establish Wayland input.
#![cfg(feature = "scenes")]

use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::prelude::*;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{Bounds, TestAppContext, WindowBounds, WindowOptions, point, px, size};
use xenadmin_gpui::{ShellState, ShellView, load_scene};
use xenadmin_ui::ObjectKind;

fn open(cx: &mut TestAppContext) -> (gpui_kit::AnyWindowHandle, gpui_kit::Entity<ShellView>) {
    let state = ShellState::demo(3, 48);
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
    })
}

#[gpui_kit::test]
fn the_shell_shows_the_pool_and_reaches_a_command(cx: &mut TestAppContext) {
    let (window, shell) = open(cx);
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("shell").is_some());
        assert!(window.try_find("toolbar").is_some());
        assert!(window.try_find("status-bar").is_some());
        assert!(window.try_find("tab-general").is_some() || window.try_find("tab-home").is_some());
        assert_eq!(shell.read(cx).state.kind, ObjectKind::Pool);
        window.click("cmd-add-server", cx);
    })
    .expect("frame");
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("add-server").is_some());
    })
    .expect("dialog");
    cx.update(|cx| {
        Theme::change(ThemeMode::Light, None, cx);
        assert!(!Theme::global(cx).is_dark());
        Theme::change(ThemeMode::Dark, None, cx);
        assert!(Theme::global(cx).is_dark());
    });
}

fn open_named(
    cx: &mut TestAppContext,
    name: &str,
) -> (gpui_kit::AnyWindowHandle, gpui_kit::Entity<ShellView>) {
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
    })
}

#[gpui_kit::test]
fn keyboard_reaches_the_tree_the_grid_and_a_command(cx: &mut TestAppContext) {
    let (window, shell) = open_named(cx, "search-pools");
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        shell.update(cx, |shell, cx| shell.focus_navigation(window, cx));
        let before = shell.read(cx).navigation_index(cx);
        window.press("down", cx);
        let after = shell.read(cx).navigation_index(cx);
        assert_ne!(before, after, "down moves the tree");
        shell.update(cx, |shell, cx| shell.focus_grid(window, cx));
        let before = shell.read(cx).grid_row(cx);
        window.press("down", cx);
        let after = shell.read(cx).grid_row(cx);
        assert_ne!(before, after, "down moves the grid");
        assert!(window.try_find("tab-search").is_some());
        window.click("cmd-add-server", cx);
    })
    .expect("keys");
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("add-server").is_some());
    })
    .expect("command");
}

#[gpui_kit::test]
fn the_gate_scenes_render(cx: &mut TestAppContext) {
    let scenes = [
        ("demo", "tab-general"),
        ("connect", "add-server"),
        ("connect-insecure", "connection-notice"),
        ("large-inventory", "shell"),
        ("connections", "add-server"),
        ("connection-settings", "add-server"),
        ("connection-trust", "add-server"),
        ("certificate-review", "add-server"),
        ("host-version-notice", "host-version-notice"),
        ("inventory-context", "tab-general"),
        ("general-information", "tab-general"),
        ("search-pools", "tab-search"),
        ("unavailable-actions", "tab-general"),
        ("interface-large", "tab-general"),
    ];
    for (name, marker) in scenes {
        let (window, shell) = open_named(cx, name);
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("shell").is_some(), "{name} draws the shell");
            assert!(window.try_find(marker).is_some(), "{name} draws {marker}");
            if name == "large-inventory" {
                assert_eq!(shell.read(cx).state.inventory.vms.len(), 2_000);
            }
        })
        .unwrap_or_else(|_| panic!("{name} frame"));
    }
}
