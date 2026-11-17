//! The object grid: the renderer of a table block.
//!
//! It is the Kit's `DataTable` bound to the rows of a description. The
//! model is the truth for everything a description carries, and the grid
//! corrects the Kit's table where the two differ:
//!
//! - **Rows** are replaced when the description's rows change; the viewport
//!   is kept.
//! - **Selection** is by row id. The Kit's table has one selected row by
//!   index: for a single selection it is kept on the model's row; for a
//!   multi selection it is only the keyboard cursor and the grid paints the
//!   model's rows itself (a plain click or an arrow key selects one row,
//!   Ctrl toggles a row, Shift extends from the last row pressed).
//! - **Sort** is the model's. The Kit cycles its own indicator; the grid
//!   ignores the state the Kit asks for, reports the press with the order
//!   the vocabulary defines, and re-reads the columns afterwards.
//! - **Column widths**: a column with a declared width has it; the others
//!   share the rest. A width the user drags is kept by column id and
//!   reported to [`ColumnWidths`] so the shell can persist it.
//! - **Activation** is a double click or Enter.
//! - **The row menu** is the table's commands resolved on the model's
//!   target after the row under the pointer was selected.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use xenadmin_ui::command;
use xenadmin_ui::surface::{self as s, Align, TableSelection as Selection};

use crate::kit::*;
use crate::render::{ColumnWidths, Pass, tone_color};
use crate::store::{AppStore, dispatch};

gpui_kit::actions!(object_grid, [ActivateRow]);

/// The Kit's key context of a table.
const TABLE_CONTEXT: &str = "DataTable";
/// The height of a row and of the header at the size the grid uses.
const ROW_HEIGHT: f32 = 30.0;
/// The rows a grid shows before it scrolls, where it does not fill.
const MAX_ROWS: usize = 10;
/// The rows of height an empty grid keeps for its empty text.
const EMPTY_ROWS: usize = 3;
/// The narrowest a column without a declared width gets.
const MIN_SHARED_WIDTH: f32 = 96.0;
/// Room the table keeps at its right edge (the scrollbar and the spacer).
const TRAILING: f32 = 14.0;

/// Binds Enter in a table to the activation of the selected row. Call once
/// after `gpui_kit::init`.
pub fn init(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("enter", ActivateRow, Some(TABLE_CONTEXT))]);
}

/// How tall a grid is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridHeight {
    /// As tall as its rows, up to ten; more rows scroll inside.
    Rows,
    /// The height its container has left.
    Fill,
}

/// What Escape does while a table has the keyboard.
pub type OnCancel = Rc<dyn Fn(&mut App)>;

struct Shared {
    /// The latest description.
    table: s::Table,
    /// The width of each column now.
    widths: Vec<Pixels>,
    /// Widths the user dragged, by column id.
    dragged: HashMap<String, f32>,
    /// Row id to index, for the rows of `table`.
    index: HashMap<String, usize>,
    /// The row a Shift click extends from.
    anchor: Option<String>,
    /// Selections the grid made itself: their events are not the user's.
    echo_select: Vec<usize>,
    echo_clear: usize,
    /// The Kit's header must re-read the columns.
    columns_dirty: bool,
    on_cancel: Option<OnCancel>,
    column_widths: Option<ColumnWidths>,
}

impl Shared {
    fn multi(&self) -> bool {
        matches!(self.table.selection, Selection::Multi(_))
    }

    fn row_id(&self, index: usize) -> Option<String> {
        self.table.rows.get(index).map(|row| row.id.clone())
    }

    fn reindex(&mut self) {
        self.index = self
            .table
            .rows
            .iter()
            .enumerate()
            .map(|(index, row)| (row.id.clone(), index))
            .collect();
    }

    /// The order a press on the header of `column` asks for: a press on
    /// the column the rows are ordered by reverses it, a press on another
    /// column starts ascending.
    fn next_descending(&self, column: &str) -> bool {
        self.table
            .sort
            .as_ref()
            .is_some_and(|sort| sort.column == column && !sort.descending)
    }
}

/// The delegate of the Kit's table: it reads the description the grid
/// holds.
pub struct GridDelegate {
    shared: Rc<RefCell<Shared>>,
    store: Entity<AppStore>,
}

impl GridDelegate {
    /// Reports a press on the header of a sortable column to the model.
    fn sort_pressed(shared: &Rc<RefCell<Shared>>, store: &Entity<AppStore>, col_ix: usize, cx: &mut App) {
        let request = {
            let mut shared = shared.borrow_mut();
            // The Kit moved its own indicator; the next pass puts the
            // model's back.
            shared.columns_dirty = true;
            let column = shared.table.columns.get(col_ix).cloned();
            match (column, shared.table.on_sort.clone()) {
                (Some(column), Some(on_sort)) if column.sortable => {
                    let descending = shared.next_descending(&column.id);
                    Some((column.id, descending, on_sort))
                }
                _ => None,
            }
        };
        match request {
            Some((column, descending, on_sort)) => {
                dispatch(store, cx, move |m| on_sort(m, &column, descending));
            }
            // Nothing to report: still re-render, to correct the header.
            None => dispatch(store, cx, |_| {}),
        }
    }

    /// Makes `id` the selection unless the selection has it already: what a
    /// right click does before the menu opens.
    fn select_for_menu(shared: &Rc<RefCell<Shared>>, store: &Entity<AppStore>, id: &str, cx: &mut App) {
        let request = {
            let shared = shared.borrow();
            if shared.table.selection.contains(id) {
                return;
            }
            (
                shared.table.selection.clone(),
                shared.table.on_select.clone(),
                shared.table.on_select_many.clone(),
            )
        };
        let id = id.to_owned();
        match request {
            (Selection::Single(_), Some(on_select), _) => {
                shared.borrow_mut().table.selection = Selection::Single(Some(id.clone()));
                dispatch(store, cx, move |m| on_select(m, &id));
            }
            (Selection::Multi(_), _, Some(on_select_many)) => {
                let rows = vec![id.clone()];
                {
                    let mut shared = shared.borrow_mut();
                    shared.table.selection = Selection::Multi(rows.clone());
                    shared.anchor = Some(id);
                }
                dispatch(store, cx, move |m| on_select_many(m, &rows));
            }
            _ => {}
        }
    }
}

impl TableDelegate for GridDelegate {
    fn columns_count(&self, _: &App) -> usize {
        self.shared.borrow().table.columns.len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.shared.borrow().table.rows.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> KitColumn {
        let shared = self.shared.borrow();
        let Some(column) = shared.table.columns.get(col_ix) else {
            return KitColumn::new("", "");
        };
        let width = shared
            .widths
            .get(col_ix)
            .copied()
            .unwrap_or(px(MIN_SHARED_WIDTH));
        let mut kit = KitColumn::new(
            SharedString::from(column.id.clone()),
            SharedString::from(column.title.clone()),
        )
        .width(width)
        .min_width(px(40.))
        .movable(false);
        if column.align == Align::End {
            kit = kit.text_right();
        }
        if column.sortable && shared.table.on_sort.is_some() {
            let indicator = match &shared.table.sort {
                Some(sort) if sort.column == column.id && sort.descending => ColumnSort::Descending,
                Some(sort) if sort.column == column.id => ColumnSort::Ascending,
                _ => ColumnSort::Default,
            };
            kit = kit.sort(indicator);
        }
        kit
    }

    fn render_th(
        &mut self,
        col_ix: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let shared = self.shared.borrow();
        let Some(column) = shared.table.columns.get(col_ix) else {
            return div().into_any_element();
        };
        let title = SharedString::from(column.title.clone());
        let sortable = column.sortable && shared.table.on_sort.is_some();
        // The key the flattened description gives a sortable header; the
        // others are found by their path.
        let id = if sortable {
            s::Table::column_key(&shared.table.key, &column.id)
        } else {
            format!("{}/columns/{}", shared.table.key, column.id)
        };
        let order = shared
            .table
            .sort
            .as_ref()
            .filter(|sort| sort.column == column.id)
            .map(|sort| {
                if sort.descending {
                    "descending"
                } else {
                    "ascending"
                }
            });
        div()
            .id(SharedString::from(id))
            .test_support()
            .role(Role::ColumnHeader)
            .aria_label(title.clone())
            .when_some(order, |header, order| header.aria_value(order))
            .w_full()
            .truncate()
            .when(sortable, |header| {
                let shared = Rc::clone(&self.shared);
                let store = self.store.clone();
                header.cursor_pointer().on_click(move |_, _, cx| {
                    Self::sort_pressed(&shared, &store, col_ix, cx);
                })
            })
            .child(title)
            .into_any_element()
    }

    fn render_tr(
        &mut self,
        row_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> Stateful<Div> {
        let shared = self.shared.borrow();
        // The Kit also asks for rows past the last one, to fill a striped
        // table.
        let Some(row) = shared.table.rows.get(row_ix) else {
            return div().id(("filler", row_ix));
        };
        let key = s::Table::row_key(&shared.table.key, &row.id);
        let multi = shared.multi();
        let marked = multi && shared.table.selection.contains(&row.id);
        let color = tone_color(row.tone, cx);
        let fill = cx.theme().table_active;
        let id = row.id.clone();
        let handle = Rc::clone(&self.shared);
        let store = self.store.clone();
        div()
            .id(SharedString::from(key))
            .text_color(color)
            .when(marked, |row| row.bg(fill))
            .when(multi, |row| {
                // Runs beside the table's own click handler, which moves
                // the cursor row on a plain click only.
                row.on_click(move |event: &ClickEvent, _, cx| {
                    let modifiers = event.modifiers();
                    if !(modifiers.secondary(cx) || modifiers.shift) {
                        return;
                    }
                    let request = {
                        let mut shared = handle.borrow_mut();
                        let Selection::Multi(current) = shared.table.selection.clone() else {
                            return;
                        };
                        let next = if modifiers.shift {
                            let anchor = shared
                                .anchor
                                .as_ref()
                                .and_then(|anchor| shared.index.get(anchor).copied());
                            match (anchor, shared.index.get(&id).copied()) {
                                (Some(from), Some(to)) => {
                                    let (low, high) = (from.min(to), from.max(to));
                                    shared.table.rows[low..=high]
                                        .iter()
                                        .map(|row| row.id.clone())
                                        .collect()
                                }
                                _ => vec![id.clone()],
                            }
                        } else {
                            let mut next = current.clone();
                            match next.iter().position(|row| row == &id) {
                                Some(position) => {
                                    next.remove(position);
                                }
                                None => next.push(id.clone()),
                            }
                            shared.anchor = Some(id.clone());
                            next
                        };
                        if shared.anchor.is_none() {
                            shared.anchor = Some(id.clone());
                        }
                        shared.table.selection = Selection::Multi(next.clone());
                        shared.table.on_select_many.clone().map(|handler| (handler, next))
                    };
                    if let Some((on_select_many, rows)) = request {
                        dispatch(&store, cx, move |m| on_select_many(m, &rows));
                    }
                })
            })
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let shared = self.shared.borrow();
        let text = shared
            .table
            .rows
            .get(row_ix)
            .and_then(|row| row.cells.get(col_ix))
            .cloned()
            .unwrap_or_default();
        div().w_full().truncate().child(SharedString::from(text))
    }

    fn cell_label(&self, row_ix: usize, col_ix: usize, _: &App) -> Option<SharedString> {
        let shared = self.shared.borrow();
        shared
            .table
            .rows
            .get(row_ix)
            .and_then(|row| row.cells.get(col_ix))
            .map(|cell| SharedString::from(cell.clone()))
    }

    fn cell_text(&self, row_ix: usize, col_ix: usize, _: &App) -> String {
        let shared = self.shared.borrow();
        shared
            .table
            .rows
            .get(row_ix)
            .and_then(|row| row.cells.get(col_ix))
            .cloned()
            .unwrap_or_default()
    }

    fn perform_sort(
        &mut self,
        col_ix: usize,
        _: ColumnSort,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) {
        Self::sort_pressed(&self.shared, &self.store, col_ix, cx);
    }

    fn context_menu(
        &mut self,
        row_ix: usize,
        menu: PopupMenu,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> PopupMenu {
        let (commands, id) = {
            let shared = self.shared.borrow();
            (shared.table.row_menu.clone(), shared.row_id(row_ix))
        };
        let Some(id) = id else {
            return menu;
        };
        if commands.is_empty() {
            return menu;
        }
        // The menu is for the row under the pointer: it is the selection
        // before the commands are resolved.
        Self::select_for_menu(&self.shared, &self.store, &id, cx);
        let items = {
            let store = self.store.read(cx);
            let model = store.model();
            command::resolve(model, &commands, &model.target())
        };
        items.into_iter().fold(menu, |menu, item| {
            let store = self.store.clone();
            let command = item.id;
            let mut entry = PopupMenuItem::new(SharedString::from(item.label.clone()))
                .disabled(item.disabled.is_some());
            if let Some(checked) = item.checked {
                entry = entry.checked(checked);
            }
            menu.item(entry.on_click(move |_, _, cx| {
                dispatch(&store, cx, move |m| {
                    let target = m.target();
                    m.invoke(command, &target);
                });
            }))
        })
    }

    fn render_empty(
        &mut self,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let shared = self.shared.borrow();
        let text = SharedString::from(shared.table.empty.clone());
        let muted = cx.theme().muted_foreground;
        h_flex()
            .id(SharedString::from(format!("{}-empty", shared.table.key)))
            .test_support()
            .role(Role::Label)
            .aria_label(text.clone())
            .size_full()
            .justify_center()
            .p_3()
            .text_sm()
            .text_color(muted)
            .child(text)
    }
}

struct Inner {
    table: Entity<TableState<GridDelegate>>,
    shared: Rc<RefCell<Shared>>,
    _subscription: Subscription,
}

/// The grid of one table block. Created once per scope and table key and
/// kept in the widget cache; [`ObjectGrid::render`] brings it in step with
/// the description on every pass.
#[derive(Clone)]
pub struct ObjectGrid {
    inner: Rc<Inner>,
}

impl ObjectGrid {
    fn new(table: &s::Table, store: &Entity<AppStore>, window: &mut Window, cx: &mut App) -> Self {
        let multi = matches!(table.selection, Selection::Multi(_));
        let selectable = table.selection != Selection::None;
        let shared = Rc::new(RefCell::new(Shared {
            table: table.clone(),
            widths: Vec::new(),
            dragged: HashMap::new(),
            index: HashMap::new(),
            anchor: None,
            echo_select: Vec::new(),
            echo_clear: 0,
            columns_dirty: true,
            on_cancel: None,
            column_widths: None,
        }));
        shared.borrow_mut().reindex();
        let state = cx.new(|cx| {
            TableState::new(
                GridDelegate {
                    shared: Rc::clone(&shared),
                    store: store.clone(),
                },
                window,
                cx,
            )
            .col_selectable(false)
            .col_movable(false)
            .loop_selection(false)
            .row_selectable(selectable)
            .select_on_modifier_click(!multi)
        });
        let subscription = cx.subscribe(&state, {
            let shared = Rc::clone(&shared);
            let store = store.clone();
            move |table, event: &TableEvent, cx| on_event(&shared, &store, &table, event, cx)
        });
        Self {
            inner: Rc::new(Inner {
                table: state,
                shared,
                _subscription: subscription,
            }),
        }
    }

    /// The Kit's table state, for tests.
    pub fn state(&self) -> &Entity<TableState<GridDelegate>> {
        &self.inner.table
    }

    /// Draws `table`, after bringing the grid in step with it.
    pub fn render(
        pass: &Pass<'_>,
        table: &s::Table,
        height: GridHeight,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let ui = pass.ui;
        let grid = ui
            .cache
            .borrow_mut()
            .grid(pass.scope, &table.key, || Self::new(table, &ui.store, window, cx));
        grid.sync(pass, table, cx);

        let rows = table.rows.len();
        let shown = if rows == 0 {
            EMPTY_ROWS
        } else {
            rows.min(MAX_ROWS)
        };
        // The header, the rows and the border.
        let fixed = px(ROW_HEIGHT * (shown as f32 + 1.) + 2.);
        let state = grid.inner.table.clone();
        let activate = {
            let shared = Rc::clone(&grid.inner.shared);
            let state = state.clone();
            let store = ui.store.clone();
            move |_: &ActivateRow, _: &mut Window, cx: &mut App| {
                let request = {
                    let shared = shared.borrow();
                    let row = state.read(cx).selected_row().and_then(|ix| shared.row_id(ix));
                    row.zip(shared.table.on_activate.clone())
                };
                match request {
                    Some((row, on_activate)) => {
                        dispatch(&store, cx, move |m| on_activate(m, &row));
                    }
                    // Nothing to activate: Enter goes on to the dialog.
                    None => cx.propagate(),
                }
            }
        };
        div()
            .id(SharedString::from(table.key.clone()))
            .test_support()
            .role(Role::Group)
            .aria_label(SharedString::from(table.key.clone()))
            .w_full()
            .map(|frame| match height {
                GridHeight::Fill => frame.flex_1().min_h(fixed.min(px(ROW_HEIGHT * 4.))),
                GridHeight::Rows => frame.h(fixed).flex_none(),
            })
            .on_action(activate)
            // A right click on the space under the last row must not open
            // the menu of the row that was right-clicked before.
            .capture_any_mouse_down({
                let state = state.clone();
                move |event, _, cx| {
                    if event.button == MouseButton::Right {
                        state.update(cx, |table, cx| table.set_right_clicked_row(None, cx));
                    }
                }
            })
            .child(
                DataTable::new(&state)
                    .small()
                    .stripe(false)
                    .accessibility_label(SharedString::from(table.key.clone())),
            )
            .into_any_element()
    }

    /// Brings the Kit's table in step with the description.
    fn sync(&self, pass: &Pass<'_>, table: &s::Table, cx: &mut App) {
        let shared = &self.inner.shared;
        let state = &self.inner.table;
        let (rows_changed, columns_changed) = {
            let mut shared = shared.borrow_mut();
            shared.on_cancel = pass.on_cancel.clone();
            shared.column_widths = pass.ui.column_widths();
            let rows_changed =
                !Rc::ptr_eq(&shared.table.rows, &table.rows) && shared.table.rows != table.rows;
            let previous = &shared.table;
            let columns_changed = shared.columns_dirty
                || previous.columns != table.columns
                || previous.sort != table.sort
                || previous.on_sort.is_some() != table.on_sort.is_some();
            shared.table = table.clone();
            if rows_changed || shared.index.len() != table.rows.len() {
                shared.reindex();
            }
            // Widths: the declared or dragged width, and the rest shared.
            let stored = shared.column_widths.clone();
            let model = pass.ui.store.read(cx);
            let fixed: Vec<Option<f32>> = table
                .columns
                .iter()
                .map(|column| {
                    stored
                        .as_ref()
                        .and_then(|widths| (widths.get)(model.model(), &table.key, &column.id))
                        .or_else(|| shared.dragged.get(&column.id).copied())
                        .or(column.width.map(|width| width as f32))
                })
                .collect();
            let taken: f32 = fixed.iter().flatten().sum();
            let sharing = fixed.iter().filter(|width| width.is_none()).count();
            let rest = f32::from(pass.width) - TRAILING - taken;
            let each = if sharing == 0 {
                0.
            } else {
                (rest / sharing as f32).floor().max(MIN_SHARED_WIDTH)
            };
            let widths: Vec<Pixels> = fixed
                .iter()
                .map(|width| px(width.unwrap_or(each)))
                .collect();
            let widths_changed = shared.widths != widths;
            shared.widths = widths;
            shared.columns_dirty = false;
            (rows_changed, columns_changed || widths_changed)
        };

        // What the Kit's selected row should be.
        let (wanted, multi) = {
            let shared = shared.borrow();
            let wanted = match &table.selection {
                Selection::None => None,
                Selection::Single(selected) => selected
                    .as_ref()
                    .and_then(|id| shared.index.get(id).copied()),
                // The cursor stays where it is while its row is selected.
                Selection::Multi(_) => state
                    .read(cx)
                    .selected_row()
                    .filter(|ix| {
                        shared
                            .row_id(*ix)
                            .is_some_and(|id| table.selection.contains(&id))
                    }),
            };
            (wanted, shared.multi())
        };
        let current = state.read(cx).selected_row();
        let selection_changed = current != wanted && !(multi && wanted.is_some());
        if !(rows_changed || columns_changed || selection_changed) {
            return;
        }
        state.update(cx, |state, cx| {
            let offset = state.vertical_scroll_handle.0.borrow().base_handle.offset();
            let mark = state.right_clicked_row();
            if columns_changed {
                state.refresh(cx);
            }
            if selection_changed {
                match wanted {
                    Some(ix) => {
                        shared.borrow_mut().echo_select.push(ix);
                        state.set_selected_row(ix, cx);
                        // `set_selected_row` drops the mark the menu of a
                        // right-clicked row needs.
                        if mark.is_some() {
                            state.set_right_clicked_row(mark, cx);
                        }
                    }
                    None => {
                        shared.borrow_mut().echo_clear += 1;
                        state.clear_selection(cx);
                    }
                }
                if rows_changed {
                    // The selection moved with its row, not by the user:
                    // keep the viewport.
                    let handle = &state.vertical_scroll_handle.0;
                    handle.borrow().base_handle.set_offset(offset);
                    handle.borrow_mut().deferred_scroll_to_item = None;
                }
            }
            cx.notify();
        });
    }
}

/// What the user did in the Kit's table, handed to the model.
fn on_event(
    shared: &Rc<RefCell<Shared>>,
    store: &Entity<AppStore>,
    table: &Entity<TableState<GridDelegate>>,
    event: &TableEvent,
    cx: &mut App,
) {
    match event {
        TableEvent::SelectRow(ix) => {
            let request = {
                let mut shared = shared.borrow_mut();
                if let Some(position) = shared.echo_select.iter().position(|echo| echo == ix) {
                    shared.echo_select.remove(position);
                    return;
                }
                let Some(id) = shared.row_id(*ix) else {
                    return;
                };
                match shared.table.selection.clone() {
                    Selection::None => None,
                    Selection::Single(selected) => {
                        if selected.as_deref() == Some(id.as_str()) {
                            None
                        } else {
                            shared.table.selection = Selection::Single(Some(id.clone()));
                            shared
                                .table
                                .on_select
                                .clone()
                                .map(|on_select| Request::One(on_select, id))
                        }
                    }
                    Selection::Multi(selected) => {
                        shared.anchor = Some(id.clone());
                        if selected.len() == 1 && selected[0] == id {
                            None
                        } else {
                            let rows = vec![id];
                            shared.table.selection = Selection::Multi(rows.clone());
                            shared
                                .table
                                .on_select_many
                                .clone()
                                .map(|on_select_many| Request::Many(on_select_many, rows))
                        }
                    }
                }
            };
            match request {
                Some(Request::One(on_select, id)) => {
                    dispatch(store, cx, move |m| on_select(m, &id));
                }
                Some(Request::Many(on_select_many, rows)) => {
                    dispatch(store, cx, move |m| on_select_many(m, &rows));
                }
                None => {}
            }
        }
        TableEvent::ClearSelection => {
            let on_cancel = {
                let mut shared = shared.borrow_mut();
                if shared.echo_clear > 0 {
                    shared.echo_clear -= 1;
                    return;
                }
                shared.on_cancel.clone()
            };
            // Escape in a table clears the Kit's selection. The model has
            // no such thing: in a surface Escape closes the surface, and
            // the next pass puts the selection back either way.
            match on_cancel {
                Some(on_cancel) => on_cancel(cx),
                None => dispatch(store, cx, |_| {}),
            }
        }
        TableEvent::DoubleClickedRow(ix) => {
            let request = {
                let shared = shared.borrow();
                shared.row_id(*ix).zip(shared.table.on_activate.clone())
            };
            if let Some((row, on_activate)) = request {
                dispatch(store, cx, move |m| on_activate(m, &row));
            }
        }
        TableEvent::RightClickedRow(Some(ix)) => {
            let id = shared.borrow().row_id(*ix);
            if let Some(id) = id {
                GridDelegate::select_for_menu(shared, store, &id, cx);
            }
        }
        TableEvent::ColumnWidthsChanged(widths) => {
            let changed: Vec<(String, String, f32)> = {
                let mut shared = shared.borrow_mut();
                let key = shared.table.key.clone();
                let columns: Vec<String> = shared
                    .table
                    .columns
                    .iter()
                    .map(|column| column.id.clone())
                    .collect();
                let mut changed = Vec::new();
                for (index, (column, width)) in columns.iter().zip(widths).enumerate() {
                    let before = shared.widths.get(index).copied().unwrap_or_default();
                    if (f32::from(before) - f32::from(*width)).abs() > 0.5 {
                        shared.dragged.insert(column.clone(), f32::from(*width));
                        changed.push((key.clone(), column.clone(), f32::from(*width)));
                    }
                }
                shared.widths = widths.clone();
                changed
            };
            let stored = shared.borrow().column_widths.clone();
            if let (Some(stored), false) = (stored, changed.is_empty()) {
                dispatch(store, cx, move |m| {
                    for (table, column, width) in &changed {
                        (stored.set)(m, table, column, *width);
                    }
                });
            }
        }
        _ => {}
    }
    let _ = table;
}

enum Request {
    One(s::ActText, String),
    Many(s::ActRows, Vec<String>),
}
