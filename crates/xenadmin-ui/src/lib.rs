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
pub mod command_list;
pub mod demo;
mod demo_details;
pub mod diagnostics;
pub mod files;
pub mod format;
pub mod harness;
pub mod intent;
pub mod launch;
pub mod model;
pub mod nav;
pub mod object;
pub mod permissions;
pub mod prefs;
pub mod properties;
mod rows;
pub mod scene;
pub mod secret;
pub mod selection;
pub mod session;
pub mod surface;
pub mod tabs;
#[cfg(any(test, feature = "test-support"))]
pub mod testing;
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
    ConsoleWindowId, ConsoleWindowRequest, FileAnswer, FileFilter, FileMode, FileRequest, Intent,
    WindowRef,
};
pub use model::{Header, Model, Status, Waker};
pub use nav::{NavRow, NavView};
pub use object::{ObjectKind, ObjectRef, Target};
pub use prefs::{Appearance, Options, Preferences};
pub use properties::{PageEvent, PageView, PropertiesKind, PropertyPage};
pub use rows::{DisplayRow, InventoryPage, compare_rows, project_rows};
pub use scene::SceneSpec;
pub use secret::SecretText;
pub use selection::{MenuOrigin, Selection};
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
        ConsoleWindowId, ConsoleWindowRequest, FileAnswer, FileFilter, FileMode, FileRequest,
        Intent, WindowRef,
    };
    pub use crate::model::{Model, resource_target};
    pub use crate::nav::{NavRow, NavView};
    pub use crate::object::{ObjectKind, ObjectRef, Target};
    pub use crate::permissions;
    pub use crate::properties::{PageEvent, PageView, PropertiesKind, PropertyPage};
    pub use crate::scene::SceneSpec;
    pub use crate::secret::SecretText;
    pub use crate::surface::{
        Act, ActRows, ActSort, ActText, Align, Block, Button, ButtonStyle, Choice, ChoiceStyle,
        Column, Control, Custom, EXPERIMENTAL_NOTICE, EXPERIMENTAL_NOTICE_KEY, Field, Group, Image,
        ImageData, Notice, NoticeKind, OptionItem, Outcome, OutcomeState, Property, Review, Row,
        Surface, SurfaceKind, SurfaceSize, Table, TableSelection, TableSort, Text, Tone, act,
        act_text, actions, alert, button, caption, checkbox, choice, column, columns, custom,
        divider, group, heading, image, input, no_op, notice, option, outcome, preformatted,
        private_textarea, properties, property, review, row, secret_input, select, table, text,
        textarea,
    };
    pub use crate::tabs::{TabBody, TabId};
}
