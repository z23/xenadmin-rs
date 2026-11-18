//! The console: the one view of the client that is not a description.
//!
//! The model (`xenadmin_ui::areas::console`) owns every console: its
//! stream, its state, its controls and what its tab or window says. This
//! module owns the pixels and the raw input, and the OS windows of the
//! detached consoles:
//!
//! - [`canvas`]: the element `console-canvas` of the renderer's registry.
//!   It draws the session's current frame (uploaded once per frame
//!   generation), in Fit or at Actual size, and sends the pointer and the
//!   keys to the guest through the seam's functions.
//! - [`keys`]: where the keys come from. By evdev code from a second
//!   keyboard listener on Wayland (decision 0006, route A); from GPUI's
//!   keystrokes where there is no listener.
//! - [`windows`]: the detached console windows and their three hooks.
//!
//! While a canvas has the keyboard every key is its guest's: the canvas
//! sets the key context [`crate::shell::actions::CONSOLE_CONTEXT`] and an
//! interceptor stops the keystroke before any binding is looked up. The
//! guest's cursor is not drawn and the local one stays visible (decision
//! 0006).

pub mod canvas;
pub mod keys;
pub mod windows;

pub use canvas::{ConsoleCanvas, canvas_of, register};
pub use windows::{ConsoleWindowView, ConsoleWindows};
