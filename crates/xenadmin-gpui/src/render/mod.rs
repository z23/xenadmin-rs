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
//!   flattened description gives it as its id (`tab/#0`,
//!   `<surface>/<review>/lines/#1`), so the two lists can be merged.
//! - A disabled control shows its reason as a tooltip.

mod blocks;
mod cache;
mod dialogs;
mod fields;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use xenadmin_ui::Model;
use xenadmin_ui::surface::{Act, Custom};

pub use blocks::{Pass, blocks, button, button_row, filling_table};
pub use cache::{TAB_SCOPE, WidgetCache, surface_scope};
pub use dialogs::{DialogHost, surface_width};

use crate::kit::*;
use crate::store::{AppStore, dispatch};

/// How a bespoke element is drawn: the console canvas, a chart, the
/// snapshot tree. Registered by name with [`Ui::register_custom`].
pub type CustomRender = Rc<dyn Fn(&Custom, &Ui, &mut Window, &mut App) -> AnyElement>;

/// Reads the stored width of a column: the model, the table key, the
/// column id.
pub type GetColumnWidth = Rc<dyn Fn(&Model, &str, &str) -> Option<f32>>;
/// Stores the width of a column: the model, the table key, the column id,
/// the width in pixels.
pub type SetColumnWidth = Rc<dyn Fn(&mut Model, &str, &str, f32)>;

/// Where a table's column widths are kept when the user resizes a column.
/// Without one the widths live as long as the table is shown.
#[derive(Clone)]
pub struct ColumnWidths {
    /// The stored width of `column` of `table`, if any.
    pub get: GetColumnWidth,
    /// Stores the width the user gave `column` of `table`.
    pub set: SetColumnWidth,
}

/// Told where the context menu the renderer is about to ask the model for
/// was asked: the position of the pointer, or of the row for the keyboard.
pub type MenuRequest = Rc<dyn Fn(Point<Pixels>)>;

/// What every render function needs. Cheap to clone.
#[derive(Clone)]
pub struct Ui {
    pub store: Entity<AppStore>,
    pub(crate) cache: Rc<RefCell<WidgetCache>>,
    customs: Rc<RefCell<HashMap<String, CustomRender>>>,
    column_widths: Rc<RefCell<Option<ColumnWidths>>>,
    menu_request: Rc<RefCell<Option<MenuRequest>>>,
}

impl Ui {
    pub fn new(store: Entity<AppStore>) -> Self {
        Self {
            store,
            cache: Rc::new(RefCell::new(WidgetCache::default())),
            customs: Rc::new(RefCell::new(HashMap::new())),
            column_widths: Rc::new(RefCell::new(None)),
            menu_request: Rc::new(RefCell::new(None)),
        }
    }

    /// The widget state the renderer keeps, for tests.
    pub fn widgets(&self) -> std::cell::Ref<'_, WidgetCache> {
        self.cache.borrow()
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

    /// Whoever presents the context menu the model has open (the shell)
    /// asks to be told where a row of a table asked for it. A table whose
    /// rows are objects calls it just before `Model::open_context_menu`.
    pub fn on_menu_request(&self, request: impl Fn(Point<Pixels>) + 'static) {
        *self.menu_request.borrow_mut() = Some(Rc::new(request));
    }

    pub(crate) fn menu_request(&self) -> Option<MenuRequest> {
        self.menu_request.borrow().clone()
    }

    /// Whether the widget of a text field of a scope is still held: what
    /// was typed into a field that is no longer drawn must be gone.
    pub fn holds_text(&self, scope: &str, key: &str) -> bool {
        self.cache.borrow().holds_text(scope, key)
    }

    /// The focus handle of a pressable element the renderer draws itself,
    /// kept with the widget state of `scope`.
    pub fn focus_handle(&self, scope: &str, key: &str, cx: &mut App) -> FocusHandle {
        self.cache.borrow_mut().focus_handle(scope, key, cx)
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

pub use crate::theme::tone_color;

/// A tooltip with plain text, for `.tooltip(..)` of any element with an id.
pub fn tip(text: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let text: SharedString = text.into();
    move |window, cx| Tooltip::new(text.clone()).build(window, cx)
}

/// A keyed element the keyboard can reach and press: a Tab stop that a
/// click, Enter and Space activate. `focus` comes from [`Ui::focus_handle`].
///
/// Inside a dialog the Kit binds Enter to its confirm action, which is
/// dispatched before any key listener: the element handles that action
/// too, so Enter presses what has the keyboard there as well.
pub fn pressable(
    id: impl Into<ElementId>,
    focus: &FocusHandle,
    on_press: impl Fn(&mut Window, &mut App) + 'static,
) -> Keyed {
    let on_press = Rc::new(on_press);
    let on_key = Rc::clone(&on_press);
    let on_confirm = Rc::clone(&on_press);
    div()
        .id(id)
        .test_support()
        .track_focus(focus)
        .cursor_pointer()
        .on_click(move |_, window, cx| on_press(window, cx))
        .on_action(move |_: &Confirm, window, cx| on_confirm(window, cx))
        .on_key_down(move |event: &KeyDownEvent, window, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                on_key(window, cx);
                cx.stop_propagation();
            }
        })
}

/// Joins a path of the flattened description, as `surface::flatten` does.
pub fn join(base: &str, segment: &str) -> String {
    if base.is_empty() {
        segment.to_owned()
    } else {
        format!("{base}/{segment}")
    }
}

/// The id of an element of a description: its key when it has one, else
/// its path in the flattened description.
pub(crate) fn element_id(base: &str, key: Option<&String>, index: usize) -> String {
    match key {
        Some(key) => key.clone(),
        None => join(base, &format!("#{index}")),
    }
}

/// The path segment of an element: its key, or `#<index>`.
pub(crate) fn segment(key: Option<&String>, index: usize) -> String {
    key.cloned().unwrap_or_else(|| format!("#{index}"))
}
