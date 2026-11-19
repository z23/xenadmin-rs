//! Toolkit-free UI model.
//!
//! Everything of the front end except drawing lives here: the [`Model`], the
//! areas with their editors, the vocabulary an area describes its surfaces
//! and tab bodies with ([`surface`]), the command table ([`command`]), the
//! tab matrix ([`tabs`]), the scenes ([`mod@scene`]) and the [`Harness`] that
//! drives all of it in tests. No type of a toolkit lives here.
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

// What is named from the crate root. Everything else is reached through its
// module.
pub use command::CommandId;
pub use demo::demonstration;
pub use demo_details::populate as populate_demo_details;
pub use format::{bytes, memory_label, natural_cmp};
pub use harness::Harness;
pub use intent::{FileMode, FileRequest, Intent};
pub use model::Model;
pub use object::{ObjectKind, Target};
// The row `Model::selected_row` and `Model::object_row` answer with.
pub use rows::DisplayRow;
pub use secret::SecretText;
pub use selection::MenuOrigin;
pub use tabs::TabId;

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
