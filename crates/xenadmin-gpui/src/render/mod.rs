//! The generic renderer: draws any `Surface` and any list of `Block`s.
//!
//! - [`Ui`] is what every render function needs: the store, the cache of
//!   widget state, the registry of bespoke elements.
//! - [`blocks`] draws a list of blocks (a tab body or the body of a surface).
//! - [`DialogHost`] keeps the window's dialogs equal to `model.surfaces()`.
//!
//! Rules the renderer keeps:
//! - The model is the truth. A text field's `String`, a checkbox's `bool`, a
//!   table's selection are read from the description on every render; the
//!   widget is corrected only where it differs.
//! - Every keyed element has `.id(key)`, the test-support marking, and an
//!   accessible role and label. An element without a key takes the path the
//!   flattened description gives it, so the two lists can be merged.
//! - A disabled control shows its reason as a tooltip.

mod blocks;
mod cache;
mod dialogs;
mod fields;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use xenadmin_ui::Model;
use xenadmin_ui::surface::{Act, Custom, Tone};

pub use blocks::{Pass, blocks, button, button_row, filling_table};
pub use cache::{TAB_SCOPE, WidgetCache, surface_scope};
pub use dialogs::{DialogHost, surface_width};

use crate::kit::*;
use crate::store::{AppStore, dispatch};

/// How a bespoke element is drawn: the console canvas, a chart, the
/// snapshot tree. Registered by name with [`Ui::register_custom`].
pub type CustomRender = Rc<dyn Fn(&Custom, &Ui, &mut Window, &mut App) -> AnyElement>;

/// Where a table's column widths are kept when the user resizes a column.
/// Without one the widths live as long as the table is shown.
#[derive(Clone)]
pub struct ColumnWidths {
    /// The stored width of `column` of `table`, if any.
    pub get: Rc<dyn Fn(&Model, &str, &str) -> Option<f32>>,
    /// Stores the width the user gave `column` of `table`.
    pub set: Rc<dyn Fn(&mut Model, &str, &str, f32)>,
}

/// What every render function needs. Cheap to clone.
#[derive(Clone)]
pub struct Ui {
    pub store: Entity<AppStore>,
    pub(crate) cache: Rc<RefCell<WidgetCache>>,
    customs: Rc<RefCell<HashMap<String, CustomRender>>>,
    column_widths: Rc<RefCell<Option<ColumnWidths>>>,
}

impl Ui {
    pub fn new(store: Entity<AppStore>) -> Self {
        Self {
            store,
            cache: Rc::new(RefCell::new(WidgetCache::default())),
            customs: Rc::new(RefCell::new(HashMap::new())),
            column_widths: Rc::new(RefCell::new(None)),
        }
    }

    /// Registers the element drawn for `Custom { name, .. }`.
    pub fn register_custom(
        &self,
        name: impl Into<String>,
        render: impl Fn(&Custom, &Ui, &mut Window, &mut App) -> AnyElement + 'static,
    ) {
        self.customs
            .borrow_mut()
            .insert(name.into(), Rc::new(render));
    }

    pub(crate) fn custom(&self, name: &str) -> Option<CustomRender> {
        self.customs.borrow().get(name).cloned()
    }

    /// Says where resized column widths are kept.
    pub fn persist_column_widths(&self, widths: ColumnWidths) {
        *self.column_widths.borrow_mut() = Some(widths);
    }

    pub(crate) fn column_widths(&self) -> Option<ColumnWidths> {
        self.column_widths.borrow().clone()
    }

    /// Runs a handler of a description on the model.
    pub fn run(&self, cx: &mut App, act: impl FnOnce(&mut Model)) {
        dispatch(&self.store, cx, act);
    }

    /// A click handler that runs `act`.
    pub fn on_click(&self, act: &Act) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
        let store = self.store.clone();
        let act = act.clone();
        move |_, _, cx| {
            let act = act.clone();
            dispatch(&store, cx, move |m| act(m));
        }
    }
}

/// The text colour of a tone.
pub fn tone_color(tone: Tone, cx: &App) -> Hsla {
    let theme = cx.theme();
    match tone {
        Tone::Normal => theme.foreground,
        Tone::Muted => theme.muted_foreground,
        Tone::Success => theme.success,
        Tone::Warning => theme.warning,
        Tone::Danger => theme.danger,
    }
}

/// A tooltip with plain text, for `.tooltip(..)` of any element with an id.
pub fn tip(text: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let text: SharedString = text.into();
    move |window, cx| Tooltip::new(text.clone()).build(window, cx)
}

/// Joins a path of the flattened description, as `surface::flatten` does.
pub(crate) fn join(base: &str, segment: &str) -> String {
    if base.is_empty() {
        segment.to_owned()
    } else {
        format!("{base}/{segment}")
    }
}

/// The path segment of an element: its key, or `#<index>`.
pub(crate) fn segment(key: Option<&String>, index: usize) -> String {
    key.cloned().unwrap_or_else(|| format!("#{index}"))
}
