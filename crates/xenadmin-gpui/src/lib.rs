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

pub mod bridge;
pub mod kit;
mod menus;
mod scancode;
pub mod store;
pub mod theme;
mod wayland_keys;

pub use kit::gpui_kit;
pub use menus::{StubItem, stub_menus};
pub use scancode::{console_physical_key, x_keysym, xt_make_code};
pub use store::{AppStore, Describe, dispatch, new_store};
pub use wayland_keys::{ForeignKeyboard, RecordedKey, SeatEvent};
