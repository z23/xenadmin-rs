//! Toolkit-free UI model.
//!
//! Everything of the front end except drawing lives here: the [`Model`], the
//! areas with their editors, the vocabulary an area describes its surfaces
//! and tab bodies with ([`surface`]), the command table ([`command`]), the
//! tab matrix ([`tabs`]), the scenes ([`mod@scene`]) and the [`Harness`] that
//! drives all of it in tests. Rune and GPUI types do not live here.
//!
//! `notes/briefs/MODEL-API.md` is the reference for area ports.

pub mod areas;
pub mod command;
mod demo;
mod demo_details;
pub mod format;
pub mod harness;
pub mod intent;
pub mod model;
pub mod object;
mod rows;
pub mod scene;
pub mod secret;
pub mod selection;
pub mod surface;
pub mod tabs;
mod tree;

pub use areas::{AreaSpec, Areas};
pub use command::{
    CommandId, CommandSpec, Eligibility, LocalRow, Menu, MenuEntry, MenuItem, MenuPath, MenuView,
    Refused, ToolbarGroup, ToolbarSlot,
};
pub use demo::demonstration;
pub use demo_details::populate as populate_demo_details;
pub use format::{bytes, memory_label, natural_cmp, storage_bytes};
pub use harness::Harness;
pub use intent::{
    ConsoleWindowId, ConsoleWindowRequest, FileFilter, FileMode, FileRequest, Intent,
};
pub use model::{Model, Waker};
pub use object::{ObjectKind, ObjectRef, Target};
pub use rows::{DisplayRow, InventoryPage, compare_rows, project_rows};
pub use scene::SceneSpec;
pub use secret::SecretText;
pub use selection::Selection;
pub use surface::{Act, ActText, Block, Surface, SurfaceKind, SurfaceSize};
pub use tabs::{TabBody, TabId, tabs_for};
// The infrastructure tree rows of the Rune front end. `Node` here is the
// tree node; the node of a flattened description is `surface::Node`.
pub use tree::{Node, TreeInsert, infrastructure_nodes};

// What the frozen Rune front end checks its scene list against: the 128
// application scene names and a lookup by name. New code uses `scene::find`.
pub use scene::{RUNE_APP_SCENES as SCENES, rune_app_scene as scene};

/// Everything an area module needs: `use crate::prelude::*;`.
pub mod prelude {
    pub use crate::areas::AreaSpec;
    pub use crate::blocks;
    pub use crate::command::{
        CommandId, CommandSpec, Eligibility, LocalRow, Menu, MenuPath, ToolbarGroup, ToolbarSlot,
        place,
    };
    pub use crate::format::{bytes, memory_label, natural_cmp, storage_bytes};
    pub use crate::intent::{
        ConsoleWindowId, ConsoleWindowRequest, FileFilter, FileMode, FileRequest, Intent,
    };
    pub use crate::model::Model;
    pub use crate::object::{ObjectKind, ObjectRef, Target};
    pub use crate::scene::SceneSpec;
    pub use crate::secret::SecretText;
    pub use crate::surface::{
        Act, ActRows, ActSort, ActText, Align, Block, Button, ButtonStyle, Choice, ChoiceStyle,
        Column, Control, Custom, Field, Group, Notice, NoticeKind, OptionItem, Outcome,
        OutcomeState, Property, Review, Row, Surface, SurfaceKind, SurfaceSize, Table,
        TableSelection, TableSort, Text, Tone, act, act_text, actions, alert, button, caption,
        checkbox, choice, column, columns, custom, divider, group, heading, input, no_op, notice,
        option, outcome, properties, property, review, row, secret_input, select, table, text,
        textarea,
    };
    pub use crate::tabs::{TabBody, TabId};
}
