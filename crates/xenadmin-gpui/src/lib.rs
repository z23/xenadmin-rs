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
//!   root view of them, which the scene tool renders and the shell wraps.
//! - `scenes` (feature `scenes`): the scene tool.

pub mod bridge;
pub mod kit;
mod menus;
pub mod render;
mod scancode;
pub mod scene_view;
#[cfg(feature = "scenes")]
pub mod scenes;
pub mod store;
pub mod theme;
mod wayland_keys;
pub mod widgets;

pub use kit::gpui_kit;
pub use menus::{StubItem, stub_menus};
pub use render::{DialogHost, Ui};
pub use scancode::{console_physical_key, x_keysym, xt_make_code};
pub use scene_view::SceneView;
pub use store::{AppStore, Describe, dispatch, new_store};
pub use wayland_keys::{ForeignKeyboard, RecordedKey, SeatEvent};

/// Installs what the front end needs after `gpui_kit::init`: the key
/// bindings of the renderer.
pub fn init(cx: &mut kit::App) {
    widgets::object_grid::init(cx);
}
