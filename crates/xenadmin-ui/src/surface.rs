//! The vocabulary: what an area shows, as plain data.
//!
//! An area never draws. It returns a [`Surface`] (a dialog, a wizard or a
//! properties dialog) or the [`Block`]s of a tab body, built with the
//! functions of this module. The GPUI front end renders any description with
//! one generic renderer; [`crate::Harness`] drives the same description in
//! tests without a window.
//!
//! A description is rebuilt whenever the front end renders and borrows
//! nothing from the model. Handlers are closures over `&mut Model`.
//!
//! Rules:
//! - Every interactive element has a `key`, and so has every text a test must
//!   find. Keep the Rune keys and the Rune labels.
//! - A disabled control always carries the reason.
//! - [`flatten`] and [`flatten_blocks`] give the description as a list of
//!   [`Node`]s. Every keyed element and every visible string is in that list.

use std::fmt;
use std::rc::Rc;
use std::sync::Arc;

use serde::Serialize;

use crate::command::CommandId;
use crate::model::Model;
use crate::object::ObjectRef;
use crate::secret::SecretText;

/// A handler without an argument: a button press, a checkbox toggle, close.
pub type Act = Rc<dyn Fn(&mut Model)>;
/// A handler with a text argument: the edited text, the picked option id,
/// the row id.
pub type ActText = Rc<dyn Fn(&mut Model, &str)>;
/// The handler of a multi-selection table: the ids of all selected rows.
pub type ActRows = Rc<dyn Fn(&mut Model, &[String])>;
/// The handler of a sortable table: the id of the column and whether the
/// order is descending.
pub type ActSort = Rc<dyn Fn(&mut Model, &str, bool)>;

/// Wraps a closure as an [`Act`].
pub fn act(handler: impl Fn(&mut Model) + 'static) -> Act {
    Rc::new(handler)
}

/// Wraps a closure as an [`ActText`].
pub fn act_text(handler: impl Fn(&mut Model, &str) + 'static) -> ActText {
    Rc::new(handler)
}

/// A handler that does nothing.
pub fn no_op() -> Act {
    Rc::new(|_| {})
}

/// The text of the notice `docs/FEATURES.md` requires on every surface of an
/// experimental area. The front end shows it when [`Surface::experimental`]
/// is set.
pub const EXPERIMENTAL_NOTICE: &str =
    "Experimental: it has worked on a test lab; how it fails is not yet proven.";
/// The key of that notice.
pub const EXPERIMENTAL_NOTICE_KEY: &str = "experimental-notice";
/// Its accessibility label.
pub const EXPERIMENTAL_NOTICE_LABEL: &str = "Experimental area";

// ---------------------------------------------------------------------------
// Surface
// ---------------------------------------------------------------------------

/// A modal surface: a dialog, a wizard or a properties dialog.
#[derive(Clone)]
pub struct Surface {
    /// Stable id, for example `host-maintenance`. Also the key of the surface.
    pub id: String,
    pub kind: SurfaceKind,
    pub title: String,
    /// The object the surface is about: its name and UUID.
    pub subtitle: Option<String>,
    /// The front end shows the experimental notice.
    pub experimental: bool,
    pub size: SurfaceSize,
    pub body: Vec<Block>,
    /// In order. The last button is the default action.
    pub footer: Vec<Button>,
    /// A line above the footer buttons, for example the permission assessment.
    pub footer_note: Option<Text>,
    /// Escape, a press on the overlay, the close button.
    pub on_close: Act,
}

#[derive(Clone)]
pub enum SurfaceKind {
    /// A single-step action with its review.
    Dialog,
    /// A multi-step creation or transfer. `current` indexes `steps`.
    Wizard { steps: Vec<String>, current: usize },
    /// Editable settings of one object, in pages. `current` is a page id;
    /// `on_page` receives the id of the page the user picked.
    Properties {
        pages: Vec<OptionItem>,
        current: String,
        on_page: ActText,
    },
}

/// How wide a surface wants to be.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SurfaceSize {
    /// A confirmation (the Rune dialogs of fixed width 460).
    Small,
    /// A form (the Rune dialogs up to 680 wide).
    #[default]
    Medium,
    /// A workspace with a grid (the Rune dialogs 760 wide and more).
    Large,
}

impl Surface {
    fn new(id: impl Into<String>, title: impl Into<String>, kind: SurfaceKind) -> Self {
        Self {
            id: id.into(),
            kind,
            title: title.into(),
            subtitle: None,
            experimental: false,
            size: SurfaceSize::Medium,
            body: Vec::new(),
            footer: Vec::new(),
            footer_note: None,
            on_close: no_op(),
        }
    }

    /// A single-step dialog.
    pub fn dialog(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self::new(id, title, SurfaceKind::Dialog)
    }

    /// A wizard showing step `current` (zero-based) of `steps`.
    pub fn wizard(
        id: impl Into<String>,
        title: impl Into<String>,
        steps: impl IntoIterator<Item = impl Into<String>>,
        current: usize,
    ) -> Self {
        let steps = steps.into_iter().map(Into::into).collect();
        Self::new(id, title, SurfaceKind::Wizard { steps, current })
    }

    /// A properties dialog showing the page with id `current`.
    pub fn properties(
        id: impl Into<String>,
        title: impl Into<String>,
        pages: impl IntoIterator<Item = impl Into<OptionItem>>,
        current: impl Into<String>,
        on_page: impl Fn(&mut Model, &str) + 'static,
    ) -> Self {
        let kind = SurfaceKind::Properties {
            pages: pages.into_iter().map(Into::into).collect(),
            current: current.into(),
            on_page: Rc::new(on_page),
        };
        Self::new(id, title, kind).large()
    }

    pub fn subtitle(mut self, subtitle: impl Into<String>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    /// Marks the surface as belonging to an experimental area.
    pub fn experimental(mut self, experimental: bool) -> Self {
        self.experimental = experimental;
        self
    }

    pub fn size(mut self, size: SurfaceSize) -> Self {
        self.size = size;
        self
    }

    pub fn small(self) -> Self {
        self.size(SurfaceSize::Small)
    }

    pub fn large(self) -> Self {
        self.size(SurfaceSize::Large)
    }

    /// The body, usually written with [`crate::blocks!`].
    pub fn body(mut self, body: Vec<Block>) -> Self {
        self.body = body;
        self
    }

    /// The footer buttons in order. The last is the default action.
    pub fn footer(mut self, buttons: impl IntoIterator<Item = Button>) -> Self {
        self.footer = buttons.into_iter().collect();
        self
    }

    pub fn footer_note(mut self, note: impl Into<Text>) -> Self {
        self.footer_note = Some(note.into());
        self
    }

    /// What Escape, the overlay and the close button do. Without it the
    /// surface cannot be dismissed that way.
    pub fn on_close(mut self, handler: impl Fn(&mut Model) + 'static) -> Self {
        self.on_close = Rc::new(handler);
        self
    }

    /// The key of the page list of a properties dialog: `<id>-pages`.
    pub fn pages_key(&self) -> String {
        format!("{}-pages", self.id)
    }

    /// The surface as a flat list of nodes. Same as [`flatten`].
    pub fn nodes(&self) -> Vec<Node> {
        flatten(self)
    }
}

// ---------------------------------------------------------------------------
// Blocks
// ---------------------------------------------------------------------------

/// One piece of a surface body or a tab body.
#[derive(Clone)]
pub enum Block {
    Heading(String),
    /// A paragraph.
    Text(Text),
    /// A banner.
    Notice(Notice),
    /// A label, a control, help and an error.
    Field(Field),
    /// Pick one option.
    Choice(Choice),
    /// A label and value list. Values can be copyable.
    Properties(Vec<Property>),
    /// Columns and rows, with optional selection, activation and row menu.
    Table(Table),
    /// The review step.
    Review(Review),
    /// Progress, a partial result or an unknown outcome, with its actions.
    Outcome(Outcome),
    /// A row of buttons.
    Actions(Vec<Button>),
    /// A titled section.
    Group(Group),
    /// Side by side when the surface is wide, stacked when it is narrow.
    Columns(Vec<Vec<Block>>),
    /// A bespoke element the front end provides.
    Custom(Custom),
    Divider,
    /// A picture the model holds.
    Image(Image),
}

/// The colour role of a text, a property value or a table row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Tone {
    #[default]
    Normal,
    Muted,
    Success,
    Warning,
    Danger,
}

/// A paragraph. `alert` marks an error for assistive technology.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Text {
    pub key: Option<String>,
    pub text: String,
    pub tone: Tone,
    pub alert: bool,
    /// The text keeps its line breaks, is drawn in a monospace face and can
    /// be selected and copied as one text ([`preformatted`]).
    pub preformatted: bool,
}

impl Text {
    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Keeps the line breaks, in a monospace face, selectable as one text.
    pub fn preformatted(mut self) -> Self {
        self.preformatted = true;
        self
    }

    pub fn tone(mut self, tone: Tone) -> Self {
        self.tone = tone;
        self
    }
}

impl From<&str> for Text {
    fn from(value: &str) -> Self {
        text(value)
    }
}

impl From<String> for Text {
    fn from(value: String) -> Self {
        text(value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NoticeKind {
    Info,
    Warning,
    Danger,
    /// The data shown is synthetic (demo mode, a preview).
    Preview,
}

impl NoticeKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Danger => "danger",
            Self::Preview => "preview",
        }
    }
}

/// A banner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub key: Option<String>,
    pub kind: NoticeKind,
    pub title: Option<String>,
    pub text: String,
}

impl Notice {
    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }
}

/// A form row: a label, a control, help and an error.
#[derive(Clone)]
pub struct Field {
    pub key: String,
    pub label: String,
    pub control: Control,
    pub help: Option<String>,
    pub error: Option<String>,
    /// The reason the control cannot be changed.
    pub disabled: Option<String>,
}

#[derive(Clone)]
pub enum Control {
    Text {
        /// The draft. Always empty for a secret field.
        value: String,
        /// The draft of a secret field. The front end masks it and keeps no
        /// copy of it after it has handed an edit to `on_edit`.
        secret: Option<SecretText>,
        multiline: bool,
        placeholder: String,
        /// Receives the whole new text on every edit.
        on_edit: ActText,
        /// Enter in a single-line field.
        on_submit: Option<Act>,
    },
    Checkbox {
        checked: bool,
        on_toggle: Act,
    },
    Select {
        options: Vec<OptionItem>,
        /// The id of the selected option.
        selected: Option<String>,
        /// Receives the id of the picked option.
        on_pick: ActText,
    },
}

impl Field {
    fn new(key: impl Into<String>, label: impl Into<String>, control: Control) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            control,
            help: None,
            error: None,
            disabled: None,
        }
    }

    /// Help text under the control.
    pub fn help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    /// A validation message for this field.
    pub fn error(mut self, error: impl Into<String>) -> Self {
        self.error = Some(error.into());
        self
    }

    /// A validation message when there is one.
    pub fn error_opt(mut self, error: Option<impl Into<String>>) -> Self {
        self.error = error.map(Into::into);
        self
    }

    /// Placeholder of a text control. Ignored by the other controls.
    pub fn placeholder(mut self, text: impl Into<String>) -> Self {
        if let Control::Text { placeholder, .. } = &mut self.control {
            *placeholder = text.into();
        }
        self
    }

    /// What Enter does in a single-line text control.
    pub fn on_submit(mut self, handler: impl Fn(&mut Model) + 'static) -> Self {
        if let Control::Text { on_submit, .. } = &mut self.control {
            *on_submit = Some(Rc::new(handler));
        }
        self
    }

    /// Disables the control when `condition` holds, with the reason.
    pub fn disabled_if(mut self, condition: bool, reason: impl Into<String>) -> Self {
        if condition {
            self.disabled = Some(reason.into());
        }
        self
    }

    /// Disables the control when a reason is given.
    pub fn disabled_opt(mut self, reason: Option<impl Into<String>>) -> Self {
        if let Some(reason) = reason {
            self.disabled = Some(reason.into());
        }
        self
    }
}

/// One option of a [`Choice`], of a select control or one page of a
/// properties dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OptionItem {
    pub id: String,
    pub label: String,
    /// A second line under the label.
    pub detail: Option<String>,
    /// The key of the option when it is not `<owner key>-<id>`. Use it to
    /// keep the key of the Rune button the option replaces.
    pub key: Option<String>,
    /// The reason the option cannot be picked.
    pub disabled: Option<String>,
}

impl OptionItem {
    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }

    pub fn disabled_if(mut self, condition: bool, reason: impl Into<String>) -> Self {
        if condition {
            self.disabled = Some(reason.into());
        }
        self
    }
}

impl<A: Into<String>, B: Into<String>> From<(A, B)> for OptionItem {
    fn from((id, label): (A, B)) -> Self {
        option(id, label)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ChoiceStyle {
    /// A vertical list of radio buttons, with room for a detail line.
    #[default]
    Radio,
    /// A row of segments, for two to four short options.
    Segmented,
    /// A drop-down, for many options.
    Select,
}

/// Pick one option. It replaces the Rune rows of buttons used as a choice.
#[derive(Clone)]
pub struct Choice {
    pub key: String,
    pub label: Option<String>,
    pub options: Vec<OptionItem>,
    /// The id of the selected option.
    pub selected: Option<String>,
    pub style: ChoiceStyle,
    /// Receives the id of the picked option.
    pub on_pick: ActText,
    /// The reason nothing can be picked.
    pub disabled: Option<String>,
}

impl Choice {
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn style(mut self, style: ChoiceStyle) -> Self {
        self.style = style;
        self
    }

    pub fn radio(self) -> Self {
        self.style(ChoiceStyle::Radio)
    }

    pub fn segmented(self) -> Self {
        self.style(ChoiceStyle::Segmented)
    }

    pub fn dropdown(self) -> Self {
        self.style(ChoiceStyle::Select)
    }

    pub fn disabled_if(mut self, condition: bool, reason: impl Into<String>) -> Self {
        if condition {
            self.disabled = Some(reason.into());
        }
        self
    }
}

/// One line of a label and value list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Property {
    pub key: Option<String>,
    pub label: String,
    pub value: String,
    /// The front end offers to copy the value (UUIDs, addresses).
    pub copy: bool,
    pub tone: Tone,
}

impl Property {
    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }

    pub fn copyable(mut self) -> Self {
        self.copy = true;
        self
    }

    pub fn tone(mut self, tone: Tone) -> Self {
        self.tone = tone;
        self
    }
}

impl<A: Into<String>, B: Into<String>> From<(A, B)> for Property {
    fn from((label, value): (A, B)) -> Self {
        property(label, value)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Align {
    #[default]
    Start,
    /// Numbers.
    End,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Column {
    pub id: String,
    pub title: String,
    pub align: Align,
    /// Preferred width in pixels. Without one the column shares the rest.
    pub width: Option<u32>,
    /// A press on the header sorts by this column, when the table has
    /// [`Table::sorted`].
    pub sortable: bool,
}

impl Column {
    pub fn right(mut self) -> Self {
        self.align = Align::End;
        self
    }

    pub fn width(mut self, pixels: u32) -> Self {
        self.width = Some(pixels);
        self
    }

    /// The header sorts the table by this column.
    pub fn sortable(mut self) -> Self {
        self.sortable = true;
        self
    }
}

impl<A: Into<String>, B: Into<String>> From<(A, B)> for Column {
    fn from((id, title): (A, B)) -> Self {
        column(id, title)
    }
}

/// One table row. `cells` are in column order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// Stable id: the object's UUID, not an index.
    pub id: String,
    pub cells: Vec<String>,
    pub tone: Tone,
    /// The object the row stands for, when it stands for one. A secondary
    /// press or the Menu key on such a row opens the kernel's context menu
    /// of the object (`Model::open_context_menu(object, MenuOrigin::Grid)`)
    /// instead of [`Table::row_menu`].
    pub object: Option<ObjectRef>,
}

impl Row {
    /// The row stands for this object: its context menu is the object's.
    /// Keep the model's grid selection (`Model::select_in_grid`) equal to
    /// the selected rows in the table's selection handler, because the
    /// menu's commands act on that selection.
    pub fn object(mut self, object: ObjectRef) -> Self {
        self.object = Some(object);
        self
    }

    pub fn tone(mut self, tone: Tone) -> Self {
        self.tone = tone;
        self
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum TableSelection {
    /// Rows cannot be selected.
    #[default]
    None,
    /// At most one row, by id.
    Single(Option<String>),
    /// Any number of rows, by id.
    Multi(Vec<String>),
}

impl TableSelection {
    pub fn contains(&self, row: &str) -> bool {
        match self {
            Self::None => false,
            Self::Single(selected) => selected.as_deref() == Some(row),
            Self::Multi(selected) => selected.iter().any(|id| id == row),
        }
    }
}

/// The order the rows of a table are in. The model sorts; the table only
/// shows which column the order follows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableSort {
    /// The id of the column.
    pub column: String,
    pub descending: bool,
}

#[derive(Clone)]
pub struct Table {
    pub key: String,
    pub columns: Vec<Column>,
    /// Already in the order to show.
    pub rows: Rc<Vec<Row>>,
    pub selection: TableSelection,
    /// Single selection: receives the id of the row the user selected.
    pub on_select: Option<ActText>,
    /// Multi selection: receives the ids of all selected rows.
    pub on_select_many: Option<ActRows>,
    /// Double click or Enter on a row: receives its id.
    pub on_activate: Option<ActText>,
    /// The commands of the row context menu. The front end resolves them with
    /// [`crate::command::resolve`] against the model's target after the row
    /// was selected.
    pub row_menu: Vec<CommandId>,
    /// The column the rows are ordered by, when the user can change it.
    pub sort: Option<TableSort>,
    /// A press on the header of a sortable column: receives the column id
    /// and whether the new order is descending.
    pub on_sort: Option<ActSort>,
    /// Shown instead of rows when there are none.
    pub empty: String,
    /// Escape while the table of a tab has the keyboard: clear the
    /// selection. In a surface Escape closes the surface and this is not
    /// called.
    pub on_clear: Option<Act>,
}

impl Table {
    /// Rows can be selected one at a time.
    pub fn single(
        mut self,
        selected: Option<&str>,
        on_select: impl Fn(&mut Model, &str) + 'static,
    ) -> Self {
        self.selection = TableSelection::Single(selected.map(str::to_owned));
        self.on_select = Some(Rc::new(on_select));
        self
    }

    /// Several rows can be selected.
    pub fn multi(
        mut self,
        selected: impl IntoIterator<Item = impl Into<String>>,
        on_select: impl Fn(&mut Model, &[String]) + 'static,
    ) -> Self {
        self.selection = TableSelection::Multi(selected.into_iter().map(Into::into).collect());
        self.on_select_many = Some(Rc::new(on_select));
        self
    }

    pub fn on_activate(mut self, handler: impl Fn(&mut Model, &str) + 'static) -> Self {
        self.on_activate = Some(Rc::new(handler));
        self
    }

    pub fn row_menu(mut self, commands: impl IntoIterator<Item = CommandId>) -> Self {
        self.row_menu = commands.into_iter().collect();
        self
    }

    pub fn empty(mut self, text: impl Into<String>) -> Self {
        self.empty = text.into();
        self
    }

    /// The user can order the rows by the columns marked
    /// [`Column::sortable`]. `column` and `descending` say how `rows` are
    /// ordered now; `on_sort` receives the column the user pressed and
    /// whether the new order is descending (a press on the current column
    /// reverses it, a press on another column starts ascending). The model
    /// sorts and keeps the order; the table never sorts by itself.
    pub fn sorted(
        mut self,
        column: impl Into<String>,
        descending: bool,
        on_sort: impl Fn(&mut Model, &str, bool) + 'static,
    ) -> Self {
        self.sort = Some(TableSort {
            column: column.into(),
            descending,
        });
        self.on_sort = Some(Rc::new(on_sort));
        self
    }

    /// The user can order the rows by the columns marked
    /// [`Column::sortable`], and no column is the order yet (the rows are in
    /// the order the model gives them, a grouping for instance). The first
    /// press on a header calls `on_sort` with that column, ascending; from
    /// then on describe the table with [`Table::sorted`].
    pub fn sortable_by(mut self, on_sort: impl Fn(&mut Model, &str, bool) + 'static) -> Self {
        self.sort = None;
        self.on_sort = Some(Rc::new(on_sort));
        self
    }

    /// Escape in the table clears the selection: `handler` empties the
    /// selection the table is described with (and the model's grid
    /// selection, where the table sets it). Only for a table of a tab: in a
    /// surface Escape closes the surface.
    pub fn on_clear(mut self, handler: impl Fn(&mut Model) + 'static) -> Self {
        self.on_clear = Some(Rc::new(handler));
        self
    }

    /// The key of a row in the flattened description: `<table key>:<row id>`.
    pub fn row_key(table: &str, row: &str) -> String {
        format!("{table}:{row}")
    }

    /// The key of the header of a sortable column in the flattened
    /// description: `<table key>#<column id>`.
    pub fn column_key(table: &str, column: &str) -> String {
        format!("{table}#{column}")
    }
}

/// The pixels of an [`Image`]. Cloning shares the bytes.
#[derive(Clone, PartialEq, Eq)]
pub enum ImageData {
    /// Decoded pixels, row by row from the top left, four bytes each in the
    /// order red, green, blue, alpha (straight alpha):
    /// `width * height * 4` bytes.
    Rgba {
        width: u32,
        height: u32,
        pixels: Arc<[u8]>,
    },
    /// An encoded PNG file. The front end decodes it.
    Png(Arc<[u8]>),
}

impl ImageData {
    /// Decoded pixels. `None` when `pixels` is not `width * height * 4`
    /// bytes or a side is zero.
    pub fn rgba(width: u32, height: u32, pixels: impl Into<Arc<[u8]>>) -> Option<Self> {
        let pixels = pixels.into();
        let wanted = (width as usize)
            .checked_mul(height as usize)?
            .checked_mul(4)?;
        (wanted > 0 && pixels.len() == wanted).then_some(Self::Rgba {
            width,
            height,
            pixels,
        })
    }

    /// An encoded PNG file.
    pub fn png(bytes: impl Into<Arc<[u8]>>) -> Self {
        Self::Png(bytes.into())
    }

    /// Width and height in pixels. For a PNG they are read from its header
    /// without decoding; `None` when the bytes do not start as a PNG does.
    pub fn size(&self) -> Option<(u32, u32)> {
        match self {
            Self::Rgba { width, height, .. } => Some((*width, *height)),
            Self::Png(bytes) => {
                const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
                let header = bytes.get(..24)?;
                if &header[..8] != SIGNATURE || &header[12..16] != b"IHDR" {
                    return None;
                }
                let side = |at: usize| {
                    u32::from_be_bytes([header[at], header[at + 1], header[at + 2], header[at + 3]])
                };
                Some((side(16), side(20))).filter(|(width, height)| *width > 0 && *height > 0)
            }
        }
    }

    /// Whether two values share the same bytes (no comparison of content).
    pub fn same(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Self::Rgba {
                    width,
                    height,
                    pixels,
                },
                Self::Rgba {
                    width: other_width,
                    height: other_height,
                    pixels: other_pixels,
                },
            ) => {
                width == other_width && height == other_height && Arc::ptr_eq(pixels, other_pixels)
            }
            (Self::Png(bytes), Self::Png(other_bytes)) => Arc::ptr_eq(bytes, other_bytes),
            _ => false,
        }
    }
}

impl fmt::Debug for ImageData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rgba { width, height, .. } => write!(f, "Rgba({width}x{height})"),
            Self::Png(bytes) => write!(f, "Png({} bytes)", bytes.len()),
        }
    }
}

/// A picture the model holds: a snapshot preview, for instance. It is drawn
/// scaled to fit the width it has and `max_size`, never enlarged and never
/// distorted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub key: String,
    /// What the picture shows, for assistive technology and for tests.
    pub alt: String,
    pub data: ImageData,
    /// The largest width and height it is drawn at, in pixels of the
    /// interface. Without one: its own size, or the width there is.
    pub max_size: Option<(u32, u32)>,
}

impl Image {
    pub fn max_size(mut self, width: u32, height: u32) -> Self {
        self.max_size = Some((width, height));
        self
    }
}

/// The review step: what will happen, to what, and what stands in the way.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Review {
    pub key: String,
    /// The review text, one line per entry, word for word from the editor.
    pub lines: Vec<String>,
    /// The affected resources.
    pub affected: Vec<Property>,
    pub warnings: Vec<String>,
    /// Set when the reviewed observation is no longer current. A stale review
    /// cannot be confirmed: disable the confirm button with this reason.
    pub stale: Option<String>,
}

impl Review {
    pub fn affected(mut self, affected: impl IntoIterator<Item = impl Into<Property>>) -> Self {
        self.affected = affected.into_iter().map(Into::into).collect();
        self
    }

    pub fn warning(mut self, warning: impl Into<String>) -> Self {
        self.warnings.push(warning.into());
        self
    }

    pub fn warnings(mut self, warnings: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.warnings.extend(warnings.into_iter().map(Into::into));
        self
    }

    pub fn stale(mut self, reason: Option<impl Into<String>>) -> Self {
        self.stale = reason.map(Into::into);
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum OutcomeState {
    /// In progress, with the fraction done when it is known.
    Running(Option<f32>),
    Succeeded,
    Failed,
    /// Some steps succeeded and some did not.
    Partial,
    /// The response was lost: the server may or may not have done it.
    Unknown,
    Cancelled,
}

impl OutcomeState {
    pub fn name(self) -> &'static str {
        match self {
            Self::Running(_) => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Partial => "partial",
            Self::Unknown => "unknown",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn tone(self) -> Tone {
        match self {
            Self::Running(_) | Self::Cancelled => Tone::Normal,
            Self::Succeeded => Tone::Success,
            Self::Failed => Tone::Danger,
            Self::Partial | Self::Unknown => Tone::Warning,
        }
    }
}

/// Progress, a partial result or an unknown outcome, shown where the action
/// was started, with its actions (Recheck, Acknowledge, Cancel).
#[derive(Clone)]
pub struct Outcome {
    pub key: String,
    pub state: OutcomeState,
    pub title: String,
    pub lines: Vec<String>,
    pub actions: Vec<Button>,
}

impl Outcome {
    pub fn line(mut self, line: impl Into<String>) -> Self {
        self.lines.push(line.into());
        self
    }

    pub fn lines(mut self, lines: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.lines.extend(lines.into_iter().map(Into::into));
        self
    }

    pub fn action(mut self, button: Button) -> Self {
        self.actions.push(button);
        self
    }

    pub fn actions(mut self, buttons: impl IntoIterator<Item = Button>) -> Self {
        self.actions.extend(buttons);
        self
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ButtonStyle {
    /// The one emphasised action (Rune `Contrast`).
    Primary,
    /// Rune `Neutral` and `Outline`.
    #[default]
    Secondary,
    /// Force and destructive actions (Rune `Danger`).
    Danger,
    /// An action that gives something up without destroying anything: it
    /// asks for a second look, not for alarm (Rune `Warning`: "Review
    /// unknown outcome", "Acknowledge outcome").
    Warning,
    Ghost,
}

#[derive(Clone)]
pub struct Button {
    pub key: String,
    pub label: String,
    pub style: ButtonStyle,
    /// The reason the button cannot be pressed.
    pub disabled: Option<String>,
    pub on_press: Act,
}

impl Button {
    pub fn style(mut self, style: ButtonStyle) -> Self {
        self.style = style;
        self
    }

    pub fn primary(self) -> Self {
        self.style(ButtonStyle::Primary)
    }

    pub fn danger(self) -> Self {
        self.style(ButtonStyle::Danger)
    }

    /// The Rune `Warning` variant: see [`ButtonStyle::Warning`].
    pub fn warning(self) -> Self {
        self.style(ButtonStyle::Warning)
    }

    /// The secondary style, which is the default. It exists so that a port
    /// of a Rune `Outline` button reads like the original.
    pub fn outline(self) -> Self {
        self.style(ButtonStyle::Secondary)
    }

    pub fn ghost(self) -> Self {
        self.style(ButtonStyle::Ghost)
    }

    /// Disables the button when `condition` holds, with the reason. A reason
    /// set earlier is kept, so chain the checks in the order the reasons
    /// should win.
    pub fn disabled_if(mut self, condition: bool, reason: impl Into<String>) -> Self {
        if condition && self.disabled.is_none() {
            self.disabled = Some(reason.into());
        }
        self
    }

    /// Disables the button when a reason is given. A reason set earlier is
    /// kept.
    pub fn disabled_opt(mut self, reason: Option<impl Into<String>>) -> Self {
        if let (None, Some(reason)) = (&self.disabled, reason) {
            self.disabled = Some(reason.into());
        }
        self
    }
}

/// A titled section of blocks.
#[derive(Clone)]
pub struct Group {
    pub key: Option<String>,
    pub title: String,
    pub blocks: Vec<Block>,
    /// The user can fold the section. The front end keeps the folded state.
    pub collapsible: bool,
    /// The section starts folded.
    pub collapsed: bool,
}

impl Group {
    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The section can be folded and starts open.
    pub fn collapsible(mut self) -> Self {
        self.collapsible = true;
        self
    }

    /// The section can be folded and starts folded.
    pub fn collapsed(mut self) -> Self {
        self.collapsible = true;
        self.collapsed = true;
        self
    }
}

/// A bespoke element the front end provides (the console canvas, a chart,
/// the snapshot tree), named and parameterised. The front end looks `name`
/// up in its own table; an unknown name renders as a placeholder.
#[derive(Clone, Debug, PartialEq)]
pub struct Custom {
    pub name: String,
    pub key: Option<String>,
    pub params: serde_json::Value,
}

impl Custom {
    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }
}

// ---------------------------------------------------------------------------
// Builder functions. They carry the Rune helper names so a port reads like
// the original.
// ---------------------------------------------------------------------------

/// A section title.
pub fn heading(value: impl Into<String>) -> Block {
    Block::Heading(value.into())
}

/// A paragraph in the normal text colour.
pub fn text(value: impl Into<String>) -> Text {
    Text {
        key: None,
        text: value.into(),
        tone: Tone::Normal,
        alert: false,
        preformatted: false,
    }
}

/// A text that keeps its line breaks and is selectable as one text, in a
/// monospace face: the details of an operation record, a list of UUIDs, a
/// host's error text. Lines too long for the width wrap; nothing is cut.
/// Its node has the role `preformatted` and the whole text.
pub fn preformatted(value: impl Into<String>) -> Text {
    text(value).preformatted()
}

/// Muted text: help, status, review lines.
pub fn caption(value: impl Into<String>) -> Text {
    text(value).tone(Tone::Muted)
}

/// An error: danger text announced to assistive technology.
pub fn alert(value: impl Into<String>) -> Text {
    Text {
        alert: true,
        ..text(value).tone(Tone::Danger)
    }
}

/// A banner.
pub fn notice(kind: NoticeKind, value: impl Into<String>) -> Notice {
    Notice {
        key: None,
        kind,
        title: None,
        text: value.into(),
    }
}

fn text_control(
    key: impl Into<String>,
    label: impl Into<String>,
    value: &str,
    multiline: bool,
    on_edit: impl Fn(&mut Model, &str) + 'static,
) -> Field {
    Field::new(
        key,
        label,
        Control::Text {
            value: value.to_owned(),
            secret: None,
            multiline,
            placeholder: String::new(),
            on_edit: Rc::new(on_edit),
            on_submit: None,
        },
    )
}

/// A single-line text field. `on_edit` receives the whole new text.
pub fn input(
    key: impl Into<String>,
    label: impl Into<String>,
    value: &str,
    on_edit: impl Fn(&mut Model, &str) + 'static,
) -> Field {
    text_control(key, label, value, false, on_edit)
}

/// A masked single-line field for a password draft.
pub fn secret_input(
    key: impl Into<String>,
    label: impl Into<String>,
    value: &SecretText,
    on_edit: impl Fn(&mut Model, &str) + 'static,
) -> Field {
    Field::new(
        key,
        label,
        Control::Text {
            value: String::new(),
            secret: Some(value.clone()),
            multiline: false,
            placeholder: String::new(),
            on_edit: Rc::new(on_edit),
            on_submit: None,
        },
    )
}

/// A multi-line text field.
pub fn textarea(
    key: impl Into<String>,
    label: impl Into<String>,
    value: &str,
    on_edit: impl Fn(&mut Model, &str) + 'static,
) -> Field {
    text_control(key, label, value, true, on_edit)
}

/// A checkbox. `on_toggle` flips the draft.
pub fn checkbox(
    key: impl Into<String>,
    label: impl Into<String>,
    checked: bool,
    on_toggle: impl Fn(&mut Model) + 'static,
) -> Field {
    Field::new(
        key,
        label,
        Control::Checkbox {
            checked,
            on_toggle: Rc::new(on_toggle),
        },
    )
}

/// A labelled drop-down. `on_pick` receives the id of the picked option.
pub fn select(
    key: impl Into<String>,
    label: impl Into<String>,
    options: impl IntoIterator<Item = impl Into<OptionItem>>,
    selected: Option<&str>,
    on_pick: impl Fn(&mut Model, &str) + 'static,
) -> Field {
    Field::new(
        key,
        label,
        Control::Select {
            options: options.into_iter().map(Into::into).collect(),
            selected: selected.map(str::to_owned),
            on_pick: Rc::new(on_pick),
        },
    )
}

/// One option. A pair `(id, label)` converts to one as well.
pub fn option(id: impl Into<String>, label: impl Into<String>) -> OptionItem {
    OptionItem {
        id: id.into(),
        label: label.into(),
        detail: None,
        key: None,
        disabled: None,
    }
}

/// Pick one option: a radio list unless `.segmented()` or `.dropdown()` is
/// chosen. `on_pick` receives the id of the picked option.
pub fn choice(
    key: impl Into<String>,
    options: impl IntoIterator<Item = impl Into<OptionItem>>,
    selected: Option<&str>,
    on_pick: impl Fn(&mut Model, &str) + 'static,
) -> Choice {
    Choice {
        key: key.into(),
        label: None,
        options: options.into_iter().map(Into::into).collect(),
        selected: selected.map(str::to_owned),
        style: ChoiceStyle::Radio,
        on_pick: Rc::new(on_pick),
        disabled: None,
    }
}

/// A button in the secondary style.
pub fn button(
    key: impl Into<String>,
    label: impl Into<String>,
    on_press: impl Fn(&mut Model) + 'static,
) -> Button {
    Button {
        key: key.into(),
        label: label.into(),
        style: ButtonStyle::Secondary,
        disabled: None,
        on_press: Rc::new(on_press),
    }
}

/// A row of buttons.
pub fn actions(buttons: impl IntoIterator<Item = Button>) -> Block {
    Block::Actions(buttons.into_iter().collect())
}

/// One line of a property list. A pair `(label, value)` converts to one.
pub fn property(label: impl Into<String>, value: impl Into<String>) -> Property {
    Property {
        key: None,
        label: label.into(),
        value: value.into(),
        copy: false,
        tone: Tone::Normal,
    }
}

/// A label and value list.
pub fn properties(items: impl IntoIterator<Item = impl Into<Property>>) -> Block {
    Block::Properties(items.into_iter().map(Into::into).collect())
}

/// One table column. A pair `(id, title)` converts to one.
pub fn column(id: impl Into<String>, title: impl Into<String>) -> Column {
    Column {
        id: id.into(),
        title: title.into(),
        align: Align::Start,
        width: None,
        sortable: false,
    }
}

/// One table row with its cells in column order.
pub fn row(id: impl ToString, cells: impl IntoIterator<Item = impl Into<String>>) -> Row {
    Row {
        id: id.to_string(),
        cells: cells.into_iter().map(Into::into).collect(),
        tone: Tone::Normal,
        object: None,
    }
}

/// A table without selection. Add `.single(..)` or `.multi(..)`,
/// `.on_activate(..)`, `.row_menu(..)` and `.empty(..)` as needed.
pub fn table(
    key: impl Into<String>,
    columns: impl IntoIterator<Item = impl Into<Column>>,
    rows: impl IntoIterator<Item = Row>,
) -> Table {
    Table {
        key: key.into(),
        columns: columns.into_iter().map(Into::into).collect(),
        rows: Rc::new(rows.into_iter().collect()),
        selection: TableSelection::None,
        on_select: None,
        on_select_many: None,
        on_activate: None,
        row_menu: Vec::new(),
        sort: None,
        on_sort: None,
        empty: String::new(),
        on_clear: None,
    }
}

/// The review step with its lines.
pub fn review(
    key: impl Into<String>,
    lines: impl IntoIterator<Item = impl Into<String>>,
) -> Review {
    Review {
        key: key.into(),
        lines: lines.into_iter().map(Into::into).collect(),
        affected: Vec::new(),
        warnings: Vec::new(),
        stale: None,
    }
}

/// An outcome banner.
pub fn outcome(key: impl Into<String>, state: OutcomeState, title: impl Into<String>) -> Outcome {
    Outcome {
        key: key.into(),
        state,
        title: title.into(),
        lines: Vec::new(),
        actions: Vec::new(),
    }
}

/// A titled section.
pub fn group(title: impl Into<String>, blocks: Vec<Block>) -> Group {
    Group {
        key: None,
        title: title.into(),
        blocks,
        collapsible: false,
        collapsed: false,
    }
}

/// Blocks side by side when wide, stacked when narrow.
pub fn columns(columns: impl IntoIterator<Item = Vec<Block>>) -> Block {
    Block::Columns(columns.into_iter().collect())
}

/// A bespoke element of the front end, by name, with JSON parameters.
pub fn custom(name: impl Into<String>, params: serde_json::Value) -> Custom {
    Custom {
        name: name.into(),
        key: None,
        params,
    }
}

/// A horizontal rule.
pub fn divider() -> Block {
    Block::Divider
}

/// A picture: `data` is `ImageData::rgba(width, height, pixels)` or
/// `ImageData::png(bytes)`, `alt` says what it shows. Keep the data in your
/// state as shared bytes and clone it here: the front end keeps the texture
/// for as long as the description hands it the same bytes.
pub fn image(key: impl Into<String>, alt: impl Into<String>, data: ImageData) -> Image {
    Image {
        key: key.into(),
        alt: alt.into(),
        data,
        max_size: None,
    }
}

// ---------------------------------------------------------------------------
// Block lists
// ---------------------------------------------------------------------------

/// What [`crate::blocks!`] accepts as an item: a block, anything a builder
/// returns, an `Option` of one (nothing when `None`) or a `Vec` of them.
pub trait IntoBlocks {
    fn add_to(self, out: &mut Vec<Block>);
}

impl IntoBlocks for Block {
    fn add_to(self, out: &mut Vec<Block>) {
        out.push(self);
    }
}

impl<T: IntoBlocks> IntoBlocks for Option<T> {
    fn add_to(self, out: &mut Vec<Block>) {
        if let Some(item) = self {
            item.add_to(out);
        }
    }
}

impl<T: IntoBlocks> IntoBlocks for Vec<T> {
    fn add_to(self, out: &mut Vec<Block>) {
        for item in self {
            item.add_to(out);
        }
    }
}

macro_rules! block_from {
    ($($type:ident => $variant:ident),* $(,)?) => {$(
        impl From<$type> for Block {
            fn from(value: $type) -> Self {
                Block::$variant(value)
            }
        }
        impl IntoBlocks for $type {
            fn add_to(self, out: &mut Vec<Block>) {
                out.push(Block::$variant(self));
            }
        }
    )*};
}

block_from!(
    Text => Text,
    Notice => Notice,
    Field => Field,
    Choice => Choice,
    Table => Table,
    Review => Review,
    Outcome => Outcome,
    Group => Group,
    Custom => Custom,
    Image => Image,
);

/// A lone button is a row of one.
impl From<Button> for Block {
    fn from(value: Button) -> Self {
        Block::Actions(vec![value])
    }
}

impl IntoBlocks for Button {
    fn add_to(self, out: &mut Vec<Block>) {
        out.push(self.into());
    }
}

/// Builds a `Vec<Block>` from blocks, builder results, `Option`s of them
/// (skipped when `None`) and `Vec`s of them (spliced in).
///
/// ```
/// use xenadmin_ui::blocks;
/// use xenadmin_ui::surface::*;
/// let error: Option<String> = None;
/// let body = blocks![
///     heading("Name"),
///     caption("Shown in the tree."),
///     error.map(alert),
///     button("ok", "OK", |_| {}).primary(),
/// ];
/// assert_eq!(body.len(), 3);
/// ```
#[macro_export]
macro_rules! blocks {
    ($($item:expr),* $(,)?) => {{
        #[allow(unused_mut)]
        let mut out: ::std::vec::Vec<$crate::surface::Block> = ::std::vec::Vec::new();
        $($crate::surface::IntoBlocks::add_to($item, &mut out);)*
        out
    }};
}

// ---------------------------------------------------------------------------
// Flattening
// ---------------------------------------------------------------------------

/// One element of a flattened description.
///
/// - `path` is the slash-joined ancestry: keys where an element has one,
///   `#<index>` where it has not. It is unique within one description.
/// - `key` is the element's key.
/// - `role` is one of: `dialog`, `step`, `tablist`, `tab`, `heading`, `text`,
///   `alert`, `status`, `textbox`, `placeholder`, `checkbox`, `combobox`,
///   `option`, `radiogroup`, `radio`, `property`, `table`, `columnheader`,
///   `row`, `cell`, `group`, `columns`, `button`, `custom`, `separator`,
///   `image`.
/// - `label` is the accessible name: a field label, a button label, a title.
/// - `value` is the current value: the text of a field (bullets for a
///   secret), the label of the selected option, a property value, the state
///   of an outcome, the detail line of an option, `ascending` or
///   `descending` on the header of the column a table is sorted by.
/// - `disabled` is the reason the element cannot be used.
/// - `checked` is the state of a checkbox, and for an option, a page, a step
///   or a selectable row whether it is the selected one.
/// - `tone` is set when it is not `Normal`.
/// - `text` is the string a text element shows.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Node {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    pub role: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checked: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tone: Option<Tone>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

impl Node {
    fn new(path: String, role: &'static str) -> Self {
        Self {
            path,
            role,
            ..Self::default()
        }
    }

    fn key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }

    fn key_opt(mut self, key: Option<&String>) -> Self {
        self.key = key.cloned();
        self
    }

    fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    fn value(mut self, value: impl Into<String>) -> Self {
        self.value = Some(value.into());
        self
    }

    fn text(mut self, text: impl Into<String>) -> Self {
        self.text = Some(text.into());
        self
    }

    fn tone(mut self, tone: Tone) -> Self {
        self.tone = (tone != Tone::Normal).then_some(tone);
        self
    }

    fn checked(mut self, checked: bool) -> Self {
        self.checked = Some(checked);
        self
    }

    fn disabled(mut self, reason: Option<&String>) -> Self {
        self.disabled = reason.cloned();
        self
    }

    /// The strings of this node a user can read: its text, its label, and
    /// its value where the value is shown as text (fields, selections,
    /// properties, option details).
    pub fn strings(&self) -> impl Iterator<Item = &str> {
        let shows_value = matches!(
            self.role,
            "textbox" | "combobox" | "radiogroup" | "tablist" | "property" | "option" | "radio"
        );
        self.text
            .as_deref()
            .into_iter()
            .chain(self.label.as_deref())
            .chain(self.value.as_deref().filter(|_| shows_value))
    }

    pub fn is_disabled(&self) -> bool {
        self.disabled.is_some()
    }
}

/// What interacting with an element does. The harness runs these; the front
/// end wires the same handlers to its widgets from the description itself.
#[derive(Clone)]
pub enum Handle {
    /// A button.
    Press(Act),
    /// A text control.
    Edit {
        on_edit: ActText,
        on_submit: Option<Act>,
    },
    /// A checkbox.
    Toggle(Act),
    /// One option of the choice, select control or page list with key
    /// `owner`. Clicking it picks it.
    Pick {
        owner: String,
        id: String,
        on_pick: ActText,
    },
    /// A table. `row_menu` is the table's row context menu, to be resolved
    /// with [`crate::command::resolve`] against the model's target.
    Table {
        multi: bool,
        on_select_many: Option<ActRows>,
        row_menu: Vec<CommandId>,
        /// Escape in the table of a tab.
        on_clear: Option<Act>,
    },
    /// One row of the table with key `table`.
    Row {
        table: String,
        id: String,
        multi: bool,
        on_select: Option<ActText>,
        on_select_many: Option<ActRows>,
        on_activate: Option<ActText>,
        /// The object the row stands for.
        object: Option<ObjectRef>,
    },
    /// The header of a sortable column of the table with key `table`. A
    /// press calls `on_sort` with `column` and `descending`, the order the
    /// press asks for.
    Sort {
        table: String,
        column: String,
        descending: bool,
        on_sort: ActSort,
    },
    /// A toolbar button or a button of a local action row: it invokes the
    /// command on the model's target. A description never contains one; the
    /// harness adds them for the command rows around a tab.
    Command(CommandId),
}

/// A node with what interacting with it does.
#[derive(Clone)]
pub struct Element {
    pub node: Node,
    pub handle: Option<Handle>,
}

/// The surface as a flat list of nodes, in reading order.
pub fn flatten(surface: &Surface) -> Vec<Node> {
    elements(surface).into_iter().map(|e| e.node).collect()
}

/// A list of blocks (a tab body) as a flat list of nodes. `base` is the path
/// prefix, for example `tab`; it may be empty.
pub fn flatten_blocks(base: &str, blocks: &[Block]) -> Vec<Node> {
    block_elements(base, blocks)
        .into_iter()
        .map(|e| e.node)
        .collect()
}

/// [`flatten`] with the handler of each interactive node.
pub fn elements(surface: &Surface) -> Vec<Element> {
    let mut walk = Walk::default();
    walk.surface(surface);
    walk.out
}

/// [`flatten_blocks`] with the handler of each interactive node.
pub fn block_elements(base: &str, blocks: &[Block]) -> Vec<Element> {
    let mut walk = Walk::default();
    walk.blocks(base, blocks);
    walk.out
}

/// The keys that occur more than once in a flattened description. A
/// description with a duplicate key is a defect: the front end uses keys as
/// element ids.
pub fn duplicate_keys(nodes: &[Node]) -> Vec<String> {
    let mut seen = std::collections::BTreeSet::new();
    let mut duplicates = std::collections::BTreeSet::new();
    for key in nodes.iter().filter_map(|node| node.key.as_deref()) {
        if !seen.insert(key) {
            duplicates.insert(key.to_owned());
        }
    }
    duplicates.into_iter().collect()
}

fn join(base: &str, segment: &str) -> String {
    if base.is_empty() {
        segment.to_owned()
    } else {
        format!("{base}/{segment}")
    }
}

fn segment(key: Option<&String>, index: usize) -> String {
    key.cloned().unwrap_or_else(|| format!("#{index}"))
}

#[derive(Default)]
struct Walk {
    out: Vec<Element>,
}

impl Walk {
    fn push(&mut self, node: Node) {
        self.out.push(Element { node, handle: None });
    }

    fn push_with(&mut self, node: Node, handle: Handle) {
        self.out.push(Element {
            node,
            handle: Some(handle),
        });
    }

    fn surface(&mut self, surface: &Surface) {
        let root = surface.id.clone();
        let kind = match &surface.kind {
            SurfaceKind::Dialog => "dialog",
            SurfaceKind::Wizard { .. } => "wizard",
            SurfaceKind::Properties { .. } => "properties",
        };
        self.push(
            Node::new(root.clone(), "dialog")
                .key(&surface.id)
                .label(&surface.title)
                .value(kind),
        );
        if let Some(subtitle) = &surface.subtitle {
            self.push(
                Node::new(join(&root, "subtitle"), "text")
                    .tone(Tone::Muted)
                    .text(subtitle),
            );
        }
        if surface.experimental {
            self.push(
                Node::new(join(&root, EXPERIMENTAL_NOTICE_KEY), "status")
                    .key(EXPERIMENTAL_NOTICE_KEY)
                    .label(EXPERIMENTAL_NOTICE_LABEL)
                    .tone(Tone::Warning)
                    .text(EXPERIMENTAL_NOTICE),
            );
        }
        match &surface.kind {
            SurfaceKind::Dialog => {}
            SurfaceKind::Wizard { steps, current } => {
                let base = join(&root, "steps");
                for (index, step) in steps.iter().enumerate() {
                    self.push(
                        Node::new(join(&base, &format!("#{index}")), "step")
                            .label(step)
                            .checked(index == *current),
                    );
                }
            }
            SurfaceKind::Properties {
                pages,
                current,
                on_page,
            } => {
                let owner = surface.pages_key();
                let base = join(&root, &owner);
                let mut list = Node::new(base.clone(), "tablist").key(&owner);
                if let Some(page) = pages.iter().find(|page| &page.id == current) {
                    list = list.value(&page.label);
                }
                self.push(list);
                self.options(&base, &owner, "tab", pages, Some(current), None, on_page);
            }
        }
        self.blocks(&root, &surface.body);
        let footer = join(&root, "footer");
        if let Some(note) = &surface.footer_note {
            self.text(join(&footer, &segment(note.key.as_ref(), 0)), note);
        }
        for button in &surface.footer {
            self.button(&footer, button);
        }
    }

    fn blocks(&mut self, base: &str, blocks: &[Block]) {
        for (index, block) in blocks.iter().enumerate() {
            self.block(base, index, block);
        }
    }

    fn block(&mut self, base: &str, index: usize, block: &Block) {
        let anonymous = join(base, &format!("#{index}"));
        match block {
            Block::Heading(value) => self.push(Node::new(anonymous, "heading").text(value)),
            Block::Text(text) => self.text(join(base, &segment(text.key.as_ref(), index)), text),
            Block::Notice(notice) => {
                let role = if notice.kind == NoticeKind::Danger {
                    "alert"
                } else {
                    "status"
                };
                let tone = match notice.kind {
                    NoticeKind::Info => Tone::Normal,
                    NoticeKind::Warning => Tone::Warning,
                    NoticeKind::Danger => Tone::Danger,
                    NoticeKind::Preview => Tone::Muted,
                };
                let mut node = Node::new(join(base, &segment(notice.key.as_ref(), index)), role)
                    .key_opt(notice.key.as_ref())
                    .value(notice.kind.name())
                    .tone(tone)
                    .text(&notice.text);
                if let Some(title) = &notice.title {
                    node = node.label(title);
                }
                self.push(node);
            }
            Block::Field(field) => self.field(base, field),
            Block::Choice(choice) => {
                let path = join(base, &choice.key);
                let (role, option_role) = match choice.style {
                    ChoiceStyle::Radio | ChoiceStyle::Segmented => ("radiogroup", "radio"),
                    ChoiceStyle::Select => ("combobox", "option"),
                };
                let mut node = Node::new(path.clone(), role)
                    .key(&choice.key)
                    .disabled(choice.disabled.as_ref());
                if let Some(label) = &choice.label {
                    node = node.label(label);
                }
                if let Some(option) = selected_option(&choice.options, choice.selected.as_ref()) {
                    node = node.value(&option.label);
                }
                self.push(node);
                self.options(
                    &path,
                    &choice.key,
                    option_role,
                    &choice.options,
                    choice.selected.as_ref(),
                    choice.disabled.as_ref(),
                    &choice.on_pick,
                );
            }
            Block::Properties(items) => {
                for (item_index, item) in items.iter().enumerate() {
                    self.property(&anonymous, item_index, item);
                }
            }
            Block::Table(table) => self.table(base, table),
            Block::Review(review) => {
                let path = join(base, &review.key);
                self.push(Node::new(path.clone(), "group").key(&review.key));
                let lines = join(&path, "lines");
                for (line_index, line) in review.lines.iter().enumerate() {
                    self.push(
                        Node::new(join(&lines, &format!("#{line_index}")), "text").text(line),
                    );
                }
                let affected = join(&path, "affected");
                for (item_index, item) in review.affected.iter().enumerate() {
                    self.property(&affected, item_index, item);
                }
                let warnings = join(&path, "warnings");
                for (warning_index, warning) in review.warnings.iter().enumerate() {
                    self.push(
                        Node::new(join(&warnings, &format!("#{warning_index}")), "text")
                            .tone(Tone::Warning)
                            .text(warning),
                    );
                }
                if let Some(stale) = &review.stale {
                    let key = format!("{}-stale", review.key);
                    self.push(
                        Node::new(join(&path, &key), "alert")
                            .key(key)
                            .tone(Tone::Danger)
                            .text(stale),
                    );
                }
            }
            Block::Outcome(outcome) => {
                let path = join(base, &outcome.key);
                let value = match outcome.state {
                    OutcomeState::Running(Some(fraction)) => {
                        format!("running {:.0}%", fraction.clamp(0.0, 1.0) * 100.0)
                    }
                    state => state.name().to_owned(),
                };
                self.push(
                    Node::new(path.clone(), "status")
                        .key(&outcome.key)
                        .label(&outcome.title)
                        .value(value)
                        .tone(outcome.state.tone()),
                );
                let lines = join(&path, "lines");
                for (line_index, line) in outcome.lines.iter().enumerate() {
                    self.push(
                        Node::new(join(&lines, &format!("#{line_index}")), "text").text(line),
                    );
                }
                for button in &outcome.actions {
                    self.button(&path, button);
                }
            }
            Block::Actions(buttons) => {
                for button in buttons {
                    self.button(&anonymous, button);
                }
            }
            Block::Group(group) => {
                let path = join(base, &segment(group.key.as_ref(), index));
                let mut node = Node::new(path.clone(), "group")
                    .key_opt(group.key.as_ref())
                    .label(&group.title);
                if group.collapsed {
                    node = node.value("collapsed");
                }
                self.push(node);
                self.blocks(&path, &group.blocks);
            }
            Block::Columns(columns) => {
                self.push(Node::new(anonymous.clone(), "columns"));
                for (column_index, column) in columns.iter().enumerate() {
                    self.blocks(&join(&anonymous, &format!("#{column_index}")), column);
                }
            }
            Block::Custom(custom) => {
                let mut node =
                    Node::new(join(base, &segment(custom.key.as_ref(), index)), "custom")
                        .key_opt(custom.key.as_ref())
                        .label(&custom.name);
                if !custom.params.is_null() {
                    node = node.value(custom.params.to_string());
                }
                self.push(node);
            }
            Block::Divider => self.push(Node::new(anonymous, "separator")),
            Block::Image(image) => {
                let mut node = Node::new(join(base, &image.key), "image")
                    .key(&image.key)
                    .label(&image.alt);
                if let Some((width, height)) = image.data.size() {
                    node = node.value(format!("{width}x{height}"));
                }
                self.push(node);
            }
        }
    }

    fn text(&mut self, path: String, text: &Text) {
        let role = if text.alert {
            "alert"
        } else if text.preformatted {
            "preformatted"
        } else {
            "text"
        };
        self.push(
            Node::new(path, role)
                .key_opt(text.key.as_ref())
                .tone(text.tone)
                .text(&text.text),
        );
    }

    fn property(&mut self, base: &str, index: usize, item: &Property) {
        self.push(
            Node::new(join(base, &segment(item.key.as_ref(), index)), "property")
                .key_opt(item.key.as_ref())
                .label(&item.label)
                .value(&item.value)
                .tone(item.tone),
        );
    }

    fn button(&mut self, base: &str, button: &Button) {
        let tone = match button.style {
            ButtonStyle::Danger => Tone::Danger,
            ButtonStyle::Warning => Tone::Warning,
            _ => Tone::Normal,
        };
        let node = Node::new(join(base, &button.key), "button")
            .key(&button.key)
            .label(&button.label)
            .tone(tone)
            .disabled(button.disabled.as_ref());
        self.push_with(node, Handle::Press(button.on_press.clone()));
    }

    #[allow(clippy::too_many_arguments)]
    fn options(
        &mut self,
        base: &str,
        owner: &str,
        role: &'static str,
        options: &[OptionItem],
        selected: Option<&String>,
        disabled: Option<&String>,
        on_pick: &ActText,
    ) {
        for option in options {
            let key = option
                .key
                .clone()
                .unwrap_or_else(|| format!("{owner}-{}", option.id));
            let mut node = Node::new(join(base, &key), role)
                .key(key)
                .label(&option.label)
                .checked(selected == Some(&option.id))
                .disabled(disabled.or(option.disabled.as_ref()));
            if let Some(detail) = &option.detail {
                node = node.value(detail);
            }
            self.push_with(
                node,
                Handle::Pick {
                    owner: owner.to_owned(),
                    id: option.id.clone(),
                    on_pick: on_pick.clone(),
                },
            );
        }
    }

    fn field(&mut self, base: &str, field: &Field) {
        let path = join(base, &field.key);
        let node = |role| {
            Node::new(path.clone(), role)
                .key(&field.key)
                .label(&field.label)
                .disabled(field.disabled.as_ref())
        };
        match &field.control {
            Control::Text {
                value,
                secret,
                placeholder,
                on_edit,
                on_submit,
                ..
            } => {
                let shown = match secret {
                    Some(secret) => "•".repeat(secret.len()),
                    None => value.clone(),
                };
                let empty = shown.is_empty();
                self.push_with(
                    node("textbox").value(shown),
                    Handle::Edit {
                        on_edit: on_edit.clone(),
                        on_submit: on_submit.clone(),
                    },
                );
                if empty && !placeholder.is_empty() {
                    self.push(
                        Node::new(join(&path, "placeholder"), "placeholder").text(placeholder),
                    );
                }
            }
            Control::Checkbox { checked, on_toggle } => {
                self.push_with(
                    node("checkbox").checked(*checked),
                    Handle::Toggle(on_toggle.clone()),
                );
            }
            Control::Select {
                options,
                selected,
                on_pick,
            } => {
                let mut combobox = node("combobox");
                if let Some(option) = selected_option(options, selected.as_ref()) {
                    combobox = combobox.value(&option.label);
                }
                self.push(combobox);
                self.options(
                    &path,
                    &field.key,
                    "option",
                    options,
                    selected.as_ref(),
                    field.disabled.as_ref(),
                    on_pick,
                );
            }
        }
        if let Some(help) = &field.help {
            self.push(
                Node::new(join(&path, "help"), "text")
                    .tone(Tone::Muted)
                    .text(help),
            );
        }
        if let Some(error) = &field.error {
            let key = format!("{}-error", field.key);
            self.push(
                Node::new(join(&path, &key), "alert")
                    .key(key)
                    .tone(Tone::Danger)
                    .text(error),
            );
        }
    }

    fn table(&mut self, base: &str, table: &Table) {
        let path = join(base, &table.key);
        let multi = matches!(table.selection, TableSelection::Multi(_));
        self.push_with(
            Node::new(path.clone(), "table").key(&table.key),
            Handle::Table {
                multi,
                on_select_many: table.on_select_many.clone(),
                row_menu: table.row_menu.clone(),
                on_clear: table.on_clear.clone(),
            },
        );
        let header = join(&path, "columns");
        for column in &table.columns {
            let mut node = Node::new(join(&header, &column.id), "columnheader").text(&column.title);
            let current = table
                .sort
                .as_ref()
                .filter(|sort| sort.column == column.id)
                .map(|sort| sort.descending);
            if let Some(descending) = current {
                node = node.value(if descending {
                    "descending"
                } else {
                    "ascending"
                });
            }
            match (&table.on_sort, column.sortable) {
                (Some(on_sort), true) => self.push_with(
                    node.key(Table::column_key(&table.key, &column.id)),
                    Handle::Sort {
                        table: table.key.clone(),
                        column: column.id.clone(),
                        // A press on the current column reverses the order.
                        descending: current == Some(false),
                        on_sort: on_sort.clone(),
                    },
                ),
                _ => self.push(node),
            }
        }
        if table.rows.is_empty() {
            if !table.empty.is_empty() {
                let key = format!("{}-empty", table.key);
                self.push(
                    Node::new(join(&path, &key), "text")
                        .key(key)
                        .tone(Tone::Muted)
                        .text(&table.empty),
                );
            }
            return;
        }
        let selectable = table.selection != TableSelection::None;
        for row in table.rows.iter() {
            let key = Table::row_key(&table.key, &row.id);
            let row_path = join(&path, &key);
            let mut node = Node::new(row_path.clone(), "row").key(key).tone(row.tone);
            if selectable {
                node = node.checked(table.selection.contains(&row.id));
            }
            self.push_with(
                node,
                Handle::Row {
                    table: table.key.clone(),
                    id: row.id.clone(),
                    multi,
                    on_select: table.on_select.clone(),
                    on_select_many: table.on_select_many.clone(),
                    on_activate: table.on_activate.clone(),
                    object: row.object.clone(),
                },
            );
            for (index, cell) in row.cells.iter().enumerate() {
                let column = table.columns.get(index);
                let name = column.map_or_else(|| format!("#{index}"), |column| column.id.clone());
                let mut node = Node::new(join(&row_path, &name), "cell").text(cell);
                if let Some(column) = column {
                    node = node.label(&column.title);
                }
                self.push(node);
            }
        }
    }
}

fn selected_option<'a>(
    options: &'a [OptionItem],
    selected: Option<&String>,
) -> Option<&'a OptionItem> {
    let selected = selected?;
    options.iter().find(|option| &option.id == selected)
}

// ---------------------------------------------------------------------------
// Debug: a description prints as its nodes.
// ---------------------------------------------------------------------------

impl fmt::Debug for Surface {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Surface")
            .field("id", &self.id)
            .field("title", &self.title)
            .field("experimental", &self.experimental)
            .field("size", &self.size)
            .field("nodes", &flatten(self))
            .finish()
    }
}

impl fmt::Debug for Block {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list()
            .entries(flatten_blocks("", std::slice::from_ref(self)))
            .finish()
    }
}

impl fmt::Debug for Button {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Button")
            .field("key", &self.key)
            .field("label", &self.label)
            .field("style", &self.style)
            .field("disabled", &self.disabled)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_image_is_a_node_with_its_alt_text_and_its_size() {
        let pixels: Vec<u8> = vec![255; 3 * 2 * 4];
        let data = ImageData::rgba(3, 2, pixels).expect("3 x 2 pixels");
        assert_eq!(data.size(), Some((3, 2)));
        assert!(data.same(&data.clone()), "a clone shares the bytes");
        assert!(!data.same(&ImageData::rgba(3, 2, vec![255; 24]).unwrap()));
        assert_eq!(format!("{data:?}"), "Rgba(3x2)");
        // The wrong number of bytes, or no pixels, is no image.
        assert!(ImageData::rgba(3, 2, vec![0; 23]).is_none());
        assert!(ImageData::rgba(0, 2, Vec::new()).is_none());
        // The size of a PNG comes from its header.
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend([0, 0, 1, 64, 0, 0, 0, 200, 8, 6, 0, 0, 0]);
        let png = ImageData::png(png);
        assert_eq!(png.size(), Some((320, 200)));
        assert_eq!(ImageData::png(*b"not a png").size(), None);

        let body = blocks![
            image("preview", "Console of web-01", data.clone()).max_size(320, 200),
            image("broken", "Preview", ImageData::png(*b"not a png")),
        ];
        let Block::Image(first) = &body[0] else {
            panic!("an image block");
        };
        assert_eq!(first.max_size, Some((320, 200)));
        let nodes = flatten_blocks("tab", &body);
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[0].role, "image");
        assert_eq!(nodes[0].path, "tab/preview");
        assert_eq!(nodes[0].key.as_deref(), Some("preview"));
        assert_eq!(nodes[0].label.as_deref(), Some("Console of web-01"));
        assert_eq!(nodes[0].value.as_deref(), Some("3x2"));
        assert_eq!(nodes[1].value, None);
        // The alt text is a string a test can find.
        assert!(nodes[0].strings().any(|text| text == "Console of web-01"));
    }

    #[test]
    fn a_preformatted_text_and_a_warning_button_say_what_they_are() {
        let uuids = "6f1c0a52-8d3e-4b71-9c5a-2e7d0f4b9a13\n0b7d7f0e-1a2b-4c3d-8e9f-a1b2c3d4e5f6";
        let blocks = crate::blocks![
            preformatted(uuids).key("uuids"),
            caption("plain").key("plain"),
            alert("host said:\n  SR_FULL").preformatted().key("error"),
            button("ack", "Acknowledge outcome", |_| {}).warning(),
            button("force", "Force Shut Down", |_| {}).danger(),
            button("close", "Close", |_| {}),
        ];
        let nodes = flatten_blocks("tab", &blocks);
        let node = |key: &str| {
            nodes
                .iter()
                .find(|node| node.key.as_deref() == Some(key))
                .unwrap_or_else(|| panic!("no node {key}"))
        };
        // The whole text, line breaks included, in one node of its own role.
        assert_eq!(node("uuids").role, "preformatted");
        assert_eq!(node("uuids").text.as_deref(), Some(uuids));
        assert_eq!(node("uuids").tone, None, "the normal tone is not written");
        assert_eq!(node("plain").role, "text");
        // An alert stays an alert, whatever its face.
        assert_eq!(node("error").role, "alert");
        assert_eq!(node("error").tone, Some(Tone::Danger));
        let Block::Text(error) = &blocks[2] else {
            panic!("a text block");
        };
        assert!(error.preformatted && error.alert);
        // The three weights of a button are three tones in the tree.
        assert_eq!(node("ack").tone, Some(Tone::Warning));
        assert_eq!(node("force").tone, Some(Tone::Danger));
        assert_eq!(node("close").tone, None);
        let Block::Actions(buttons) = &blocks[3..][0] else {
            // `blocks!` keeps each button as its own action row.
            panic!("an action row");
        };
        assert_eq!(buttons[0].style, ButtonStyle::Warning);
        assert_eq!(
            serde_json::to_value(ButtonStyle::Warning).unwrap(),
            serde_json::json!("warning")
        );
    }

    #[test]
    fn a_table_can_be_sortable_before_any_column_is_the_order() {
        let pool = xenadmin_core::PoolId::from(uuid::Uuid::from_u128(1));
        let vm = ObjectRef::vm(pool, xenadmin_core::VmId::from(uuid::Uuid::from_u128(2)));
        let table = table(
            "guests",
            [column("name", "Name").sortable(), column("state", "State")],
            [
                row("a", ["web", "Running"]).object(vm.clone()),
                row("b", ["db", "Halted"]),
            ],
        )
        .sortable_by(|_, _, _| {})
        .on_clear(|_| {});
        assert!(table.sort.is_none() && table.on_sort.is_some());
        assert_eq!(table.rows[0].object, Some(vm.clone()));
        assert_eq!(table.rows[1].object, None);
        let elements = block_elements("tab", &[table.into()]);
        let header = elements
            .iter()
            .find(|e| e.node.key.as_deref() == Some("guests#name"))
            .expect("a sortable header has a key");
        // Not sorted yet: no order shown, and the first press is ascending.
        assert_eq!(header.node.value, None);
        assert!(matches!(
            &header.handle,
            Some(Handle::Sort {
                descending: false,
                ..
            })
        ));
        assert!(matches!(
            &elements[0].handle,
            Some(Handle::Table {
                on_clear: Some(_),
                ..
            })
        ));
        let objects: Vec<Option<ObjectRef>> = elements
            .iter()
            .filter_map(|e| match &e.handle {
                Some(Handle::Row { object, .. }) => Some(object.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(objects, [Some(vm), None]);
    }

    fn sample() -> Surface {
        let error: Option<String> = Some("Enter a name".into());
        let nothing: Option<Text> = None;
        Surface::dialog("sample", "Sample dialog")
            .subtitle("vm-1 (uuid)")
            .experimental(true)
            .body(blocks![
                heading("Details"),
                caption("Muted").key("sample-caption"),
                nothing,
                notice(NoticeKind::Warning, "Careful")
                    .title("Heads up")
                    .key("sample-notice"),
                input("sample-name", "Name", "", |_, _| {})
                    .placeholder("New name")
                    .help("Shown in the tree")
                    .error_opt(error),
                secret_input(
                    "sample-password",
                    "Password",
                    &SecretText::from("abc"),
                    |_, _| {}
                ),
                checkbox("sample-force", "Force", true, |_| {})
                    .disabled_if(true, "Not allowed here"),
                select(
                    "sample-host",
                    "Host",
                    [("a", "Host A"), ("b", "Host B")],
                    Some("b"),
                    |_, _| {}
                ),
                choice(
                    "sample-mode",
                    vec![
                        option("live", "Live").detail("Keeps the VM running"),
                        option("halted", "Halted")
                            .key("legacy-halted")
                            .disabled_if(true, "The VM is running"),
                    ],
                    Some("live"),
                    |_, _| {}
                )
                .label("Mode"),
                properties([("Name", "vm-1"), ("State", "Running")]),
                table(
                    "sample-disks",
                    [
                        column("name", "Name").sortable(),
                        column("size", "Size").right()
                    ],
                    [
                        row("d1", ["Disk 1", "8.0 GiB"]),
                        row("d2", ["Disk 2", "1.0 GiB"])
                    ],
                )
                .single(Some("d2"), |_, _| {})
                .sorted("name", false, |_, _, _| {}),
                table("sample-empty", [("name", "Name")], []).empty("No snapshots"),
                review("sample-review", ["The VM will move."])
                    .affected([property("VM", "vm-1").key("sample-review-vm")])
                    .warning("The VM pauses briefly.")
                    .stale(Some("The VM changed. Review again.")),
                outcome("sample-outcome", OutcomeState::Running(Some(0.4)), "Moving")
                    .line("Step 1 of 3")
                    .action(button("sample-cancel-op", "Request cancellation", |_| {})),
                group(
                    "Advanced",
                    blocks![button("sample-advanced", "Do it", |_| {}).danger()]
                )
                .key("sample-group")
                .collapsed(),
                columns([blocks![text("Left")], blocks![text("Right")]]),
                custom("chart", serde_json::json!({ "series": 2 })).key("sample-chart"),
                divider(),
                vec![alert("First error"), alert("Second error")],
            ])
            .footer_note(caption("The session grants the required permissions."))
            .footer([
                button("sample-cancel", "Cancel", |_| {}),
                button("sample-confirm", "Confirm", |_| {})
                    .primary()
                    .disabled_if(true, "Review again")
                    .disabled_if(true, "A later reason does not replace the first"),
            ])
    }

    fn node<'a>(nodes: &'a [Node], key: &str) -> &'a Node {
        nodes
            .iter()
            .find(|node| node.key.as_deref() == Some(key))
            .unwrap_or_else(|| panic!("no node with key {key}"))
    }

    #[test]
    fn every_keyed_element_is_in_the_flat_list_with_its_state() {
        let nodes = flatten(&sample());
        assert!(duplicate_keys(&nodes).is_empty());
        let root = node(&nodes, "sample");
        assert_eq!(
            (root.role, root.label.as_deref(), root.value.as_deref()),
            ("dialog", Some("Sample dialog"), Some("dialog"))
        );
        assert_eq!(
            node(&nodes, EXPERIMENTAL_NOTICE_KEY).text.as_deref(),
            Some(EXPERIMENTAL_NOTICE)
        );
        assert_eq!(node(&nodes, "sample-caption").tone, Some(Tone::Muted));
        let notice = node(&nodes, "sample-notice");
        assert_eq!(
            (
                notice.role,
                notice.label.as_deref(),
                notice.value.as_deref()
            ),
            ("status", Some("Heads up"), Some("warning"))
        );
        let name = node(&nodes, "sample-name");
        assert_eq!((name.role, name.value.as_deref()), ("textbox", Some("")));
        assert_eq!(name.path, "sample/sample-name");
        assert_eq!(
            node(&nodes, "sample-name-error").text.as_deref(),
            Some("Enter a name")
        );
        assert_eq!(
            node(&nodes, "sample-password").value.as_deref(),
            Some("•••")
        );
        let force = node(&nodes, "sample-force");
        assert_eq!(force.checked, Some(true));
        assert_eq!(force.disabled.as_deref(), Some("Not allowed here"));
        assert_eq!(node(&nodes, "sample-host").value.as_deref(), Some("Host B"));
        assert_eq!(node(&nodes, "sample-host-b").checked, Some(true));
        assert_eq!(node(&nodes, "sample-host-a").checked, Some(false));
        let mode = node(&nodes, "sample-mode");
        assert_eq!(
            (mode.role, mode.label.as_deref(), mode.value.as_deref()),
            ("radiogroup", Some("Mode"), Some("Live"))
        );
        assert_eq!(
            node(&nodes, "sample-mode-live").value.as_deref(),
            Some("Keeps the VM running")
        );
        assert_eq!(
            node(&nodes, "legacy-halted").disabled.as_deref(),
            Some("The VM is running")
        );
        assert_eq!(node(&nodes, "sample-disks:d2").checked, Some(true));
        assert_eq!(node(&nodes, "sample-disks:d1").checked, Some(false));
        let header = node(&nodes, "sample-disks#name");
        assert_eq!(
            (header.role, header.text.as_deref(), header.value.as_deref()),
            ("columnheader", Some("Name"), Some("ascending"))
        );
        // A column that is not sortable has a header without a key.
        assert!(
            nodes
                .iter()
                .all(|node| node.key.as_deref() != Some("sample-disks#size"))
        );
        assert_eq!(
            node(&nodes, "sample-empty-empty").text.as_deref(),
            Some("No snapshots")
        );
        assert_eq!(
            node(&nodes, "sample-review-stale").text.as_deref(),
            Some("The VM changed. Review again.")
        );
        assert_eq!(
            node(&nodes, "sample-review-vm").value.as_deref(),
            Some("vm-1")
        );
        let outcome = node(&nodes, "sample-outcome");
        assert_eq!(
            (outcome.label.as_deref(), outcome.value.as_deref()),
            (Some("Moving"), Some("running 40%"))
        );
        assert_eq!(
            node(&nodes, "sample-cancel-op").path,
            "sample/sample-outcome/sample-cancel-op"
        );
        assert_eq!(
            node(&nodes, "sample-group").value.as_deref(),
            Some("collapsed")
        );
        assert_eq!(node(&nodes, "sample-advanced").tone, Some(Tone::Danger));
        assert_eq!(
            node(&nodes, "sample-chart").value.as_deref(),
            Some(r#"{"series":2}"#)
        );
        let confirm = node(&nodes, "sample-confirm");
        assert_eq!(confirm.disabled.as_deref(), Some("Review again"));
        assert_eq!(confirm.path, "sample/footer/sample-confirm");
        assert_eq!(node(&nodes, "sample-cancel").disabled, None);
    }

    #[test]
    fn every_visible_string_is_in_the_flat_list() {
        let nodes = flatten(&sample());
        let strings: Vec<&str> = nodes.iter().flat_map(Node::strings).collect();
        for expected in [
            "Sample dialog",
            "vm-1 (uuid)",
            EXPERIMENTAL_NOTICE,
            "Details",
            "Muted",
            "Heads up",
            "Careful",
            "Name",
            "New name",
            "Shown in the tree",
            "Enter a name",
            "Password",
            "Force",
            "Host",
            "Host A",
            "Host B",
            "Mode",
            "Live",
            "Keeps the VM running",
            "Halted",
            "State",
            "Running",
            "Size",
            "Disk 1",
            "8.0 GiB",
            "No snapshots",
            "The VM will move.",
            "VM",
            "The VM pauses briefly.",
            "The VM changed. Review again.",
            "Moving",
            "Step 1 of 3",
            "Request cancellation",
            "Advanced",
            "Do it",
            "Left",
            "Right",
            "First error",
            "Second error",
            "The session grants the required permissions.",
            "Cancel",
            "Confirm",
        ] {
            assert!(strings.contains(&expected), "missing {expected:?}");
        }
        // A secret never appears, and neither do machine values.
        assert!(!strings.contains(&"abc"));
        assert!(!strings.contains(&"running 40%"));
        let mut paths: Vec<&str> = nodes.iter().map(|node| node.path.as_str()).collect();
        let count = paths.len();
        paths.sort_unstable();
        paths.dedup();
        assert_eq!(paths.len(), count, "paths are unique");
    }

    #[test]
    fn a_wizard_and_a_properties_dialog_list_their_steps_and_pages() {
        let wizard = Surface::wizard("new-vm", "New VM", ["Template", "Name", "Review"], 1);
        let nodes = flatten(&wizard);
        let steps: Vec<_> = nodes.iter().filter(|node| node.role == "step").collect();
        assert_eq!(steps.len(), 3);
        assert_eq!(steps[1].label.as_deref(), Some("Name"));
        assert_eq!(
            steps.iter().map(|step| step.checked).collect::<Vec<_>>(),
            [Some(false), Some(true), Some(false)]
        );
        let properties = Surface::properties(
            "vm-properties",
            "Properties",
            [("general", "General"), ("cpu", "CPU and Memory")],
            "cpu",
            |_, _| {},
        );
        assert_eq!(properties.size, SurfaceSize::Large);
        let nodes = flatten(&properties);
        assert_eq!(
            node(&nodes, "vm-properties-pages").value.as_deref(),
            Some("CPU and Memory")
        );
        assert_eq!(node(&nodes, "vm-properties-pages-cpu").checked, Some(true));
        assert_eq!(node(&nodes, "vm-properties-pages-general").role, "tab");
    }

    #[test]
    fn nodes_serialize_without_empty_fields() {
        let nodes = flatten_blocks("tab", &blocks![caption("Hello").key("greeting")]);
        let json = serde_json::to_value(&nodes).expect("json");
        assert_eq!(
            json,
            serde_json::json!([{
                "path": "tab/greeting",
                "key": "greeting",
                "role": "text",
                "tone": "muted",
                "text": "Hello",
            }])
        );
        let duplicate = flatten_blocks(
            "",
            &blocks![button("same", "One", |_| {}), button("same", "Two", |_| {})],
        );
        assert_eq!(duplicate_keys(&duplicate), ["same"]);
        assert!(format!("{:?}", button("k", "Label", |_| {})).contains("Label"));
    }
}
