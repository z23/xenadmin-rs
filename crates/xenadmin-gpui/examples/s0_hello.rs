//! Spike S0: the Kit hello window.
//!
//! This is not the released binary. `cargo build --workspace` does not build
//! examples; `cargo build -p xenadmin-gpui --example s0_hello` does.
//! Opening it needs a display server. This environment has no Wayland
//! session, so the spike does not run this example as a product check.

use gpui_kit::prelude::*;
use gpui_kit::{Window, WindowOptions, div};

struct Hello;

impl Render for Hello {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().child("Hello, World!")
    }
}

fn main() {
    gpui_kit::application().run(|cx| {
        gpui_kit::init(cx);
        gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| Hello))
            .expect("failed to open window");
    });
}
