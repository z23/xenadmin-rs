//! Toolkit-free UI model.
//!
//! Selection, rows, the tab matrix, the command table, and scene names live
//! here. Rune and GPUI types do not.

mod commands;
mod demo;
mod demo_details;
mod intent;
mod rows;
mod scenes;
mod selection;
mod tabs;
mod tree;

pub use commands::{COMMANDS, Command, CommandId, Eligibility, eligibility};
pub use demo::demonstration;
pub use demo_details::populate as populate_demo_details;
pub use intent::{DialogId, Intent, PageId, Surface, WizardId};
pub use rows::{
    DisplayRow, InventoryPage, compare_rows, memory_label, project_rows, storage_bytes,
};
pub use scenes::{REGISTRY as SCENES, Scene, by_name as scene};
pub use selection::{ObjectKind, Selection};
pub use tabs::{TabId, TabSet, tabs_for};
pub use tree::{Node, TreeInsert, infrastructure_nodes};
