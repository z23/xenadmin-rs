//! GPUI Kit spike crate.
//!
//! The released application is still the Rune binary in `xenadmin-desktop`.
//! This crate holds the Phase 0 probes: the wake bridge, the evdev-to-XT
//! scancode table, and a second Wayland keyboard listener. Views are not
//! ported here.

mod bridge;
mod inventory;
mod kit;
mod menus;
mod scancode;
mod shell_state;
mod shell_view;
mod wayland_keys;

gpui_kit::actions!(shell_cmd, [ShowGeneral]);

pub use bridge::{Changed, WakeBridge};
pub use inventory::ScaleModel;
pub use kit::gpui_kit;
pub use menus::{StubItem, stub_menus};
pub use scancode::{console_physical_key, x_keysym, xt_make_code};
pub use shell_state::{ShellState, load_scene};
pub use shell_view::ShellView;
pub use wayland_keys::{ForeignKeyboard, RecordedKey, SeatEvent};
