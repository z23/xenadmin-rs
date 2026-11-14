//! GPUI preview. The released binary remains `xenadmin-rs` from the Rune crate.

use gpui_kit::AppContext as _;
use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::{Bounds, WindowBounds, WindowOptions, point, px, size};
use xenadmin_gpui::{ShellState, ShellView, load_scene};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--version") {
        println!("xenadmin-gpui {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let scene = args
        .iter()
        .position(|arg| arg == "--scene")
        .and_then(|index| args.get(index + 1))
        .map(String::as_str);
    let demo = args.iter().any(|arg| arg == "--demo") || scene.is_some();
    let state = match scene {
        Some(name) => load_scene(name),
        None if demo => ShellState::demo(3, 48),
        None => ShellState::demo(0, 0),
    };
    let light = args.iter().any(|arg| arg == "--theme")
        && args
            .windows(2)
            .any(|pair| pair[0] == "--theme" && pair[1] == "light");
    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);
        if light {
            Theme::change(ThemeMode::Light, None, cx);
        }
        let _ = gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: point(px(0.), px(0.)),
                    size: size(px(1440.), px(900.)),
                })),
                ..Default::default()
            },
            cx,
            move |window, cx| cx.new(|cx| ShellView::new(state, window, cx)),
        );
    });
}
