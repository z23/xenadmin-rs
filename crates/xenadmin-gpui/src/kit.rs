//! Every Kit import used by a view goes through this module, so a later Kit
//! upgrade touches one file. Phase 0 only names the pinned facade. Shared
//! widgets are Phase 2.

/// The reviewed facade. Application code reaches GPUI as `gpui_kit::*`.
pub use gpui_kit;
