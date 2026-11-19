//! The GPUI Kit front end.
//!
//! `xenadmin-ui` owns everything except drawing: it describes what it shows
//! as plain data. This crate draws descriptions. One generic renderer turns a
//! `Surface` or a list of `Block`s into Kit components; bespoke code exists
//! only for the shell, the tree, the console and the charts.
//!
//! - [`store`]: `AppStore`, the entity that owns the model.
//! - [`bridge`]: the wake bridge from the core's threads.
//! - [`theme`]: the product accent on the Kit's theme.
//! - [`render`]: the generic renderer of surfaces and blocks, and the
//!   dialog host.
//! - [`widgets`]: the shared widgets the renderer draws with.
//! - [`scene_view`]: the object header, the tab strip, the tab body and a
//!   root view of them alone, which the renderer tests show.
//! - [`shell`]: the root view of the main window: title and menu bar,
//!   toolbar, navigation pane, the content of `scene_view`, status bar.
//! - [`intents`]: the services the model asks the application for.
//! - [`app`]: the application: command line, start, the main window, the
//!   close path.
//! - [`console`]: the console canvas, its keys and pointer, and the
//!   detached console windows.
//! - [`charts`]: the performance chart element, which leaves a gap where
//!   data is missing.
//! - [`assets`]: the icons.
//! - [`stats`]: counts of the rows built, for the large-inventory test.
//! - `scenes` (feature `scenes`): the scene tool.

pub mod app;
pub mod assets;
pub mod bridge;
pub mod charts;
pub mod console;
pub mod intents;
pub mod kit;
pub mod render;
mod scancode;
pub mod scene_view;
#[cfg(feature = "scenes")]
pub mod scenes;
pub mod shell;
pub mod stats;
pub mod store;
pub mod theme;
mod wayland_activate;
mod wayland_keys;
pub mod widgets;

pub use kit::gpui_kit;
pub use render::{DialogHost, Ui};
pub use scancode::{console_physical_key, x_keysym, xt_make_code};
pub use scene_view::SceneView;
pub use shell::ShellView;
pub use store::{AppStore, Describe, dispatch, new_store};
pub use wayland_keys::{ForeignKeyboard, RecordedKey, SeatEvent};

/// Installs what the front end needs after `gpui_kit::init`: the key
/// bindings of the renderer and the shortcuts of the command table.
pub fn init(cx: &mut kit::App) {
    widgets::object_grid::init(cx);
    shell::actions::bind_keys(cx);
}
