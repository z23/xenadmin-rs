//! The navigation pane: the search box, the tree of the active navigation
//! view and the view switcher.
//!
//! The model gives the rows of the active view (`xenadmin_ui::nav::rows`):
//! flat, parents before children, with stable ids. The pane turns them into
//! the Kit's tree items on every change and puts expansion and the cursor
//! back by row id, so an inventory update moves nothing under the user.
//!
//! What the Kit's tree does not do and the pane does itself:
//! - a row draws its own indentation, chevron, icon and status word;
//! - a press on a row selects and does not fold (the chevron folds, and so
//!   does a double press);
//! - the keyboard: the pane's own Tab stop takes the keys and moves the
//!   cursor (the Kit tree's focus handle is no Tab stop and cannot be made
//!   one);
//! - the context menu of a row: the secondary button, the Menu key or
//!   Shift+F10 ask the model for it (`open_context_menu`), and the shell
//!   presents whatever menu the model has open.

use std::collections::HashMap;
use std::rc::Rc;

use xenadmin_ui::nav::{self, NavRow, NavView};
use xenadmin_ui::object::ObjectRef;
use xenadmin_ui::selection::MenuOrigin;

use super::icons;
use super::menus::MenuAnchor;
use crate::kit::*;
use crate::render::{Ui, pressable, tone_color};
use crate::store::{AppStore, dispatch};

/// The id of the pane.
pub const PANE_KEY: &str = "sidebar-pane";
/// The id of the tree: the Tab stop that takes its keys.
pub const TREE_KEY: &str = "inventory-tree";
/// The accessible name of the tree.
pub const TREE_LABEL: &str = "Pools, hosts and virtual machines";
/// The id of the search box.
pub const SEARCH_KEY: &str = "search";
/// The accessible name of the search box.
pub const SEARCH_LABEL: &str = "Search inventory";
/// The placeholder of the search box.
pub const SEARCH_PLACEHOLDER: &str = "Search name, UUID or host";
/// The id of the view switcher.
pub const SWITCHER_KEY: &str = "nav-views";
/// The name of the id of a row's chevron: `(FOLD_KEY, <place on screen>)`.
pub const FOLD_KEY: &str = "nav-fold";
/// The id of the text shown when the view has no rows.
pub const EMPTY_KEY: &str = "nav-empty";

/// The scope of the pane's focus handles in the widget cache.
const SCOPE: &str = "nav";
/// How many rows Page Up and Page Down move.
const PAGE: usize = 10;

/// What a row shows besides its label, by row id.
struct RowMeta {
    icon: Lucide,
    status: Option<SharedString>,
    object: Option<ObjectRef>,
    /// The row shows the model's scope.
    in_scope: bool,
}

pub struct NavPane {
    ui: Ui,
    store: Entity<AppStore>,
    search: Entity<InputState>,
    tree: Entity<TreeState>,
    /// The Tab stop of the tree.
    tree_stop: FocusHandle,
    /// The rows the tree was built from, and what they were built for.
    rows: Vec<NavRow>,
    view: NavView,
    filter: String,
    meta: Rc<HashMap<SharedString, RowMeta>>,
    /// The ids of the rows that are on screen when nothing is scrolled off:
    /// every row whose ancestors are expanded, in order.
    visible: Vec<SharedString>,
    /// Expansion the user chose, by view and row id. A row the user did not
    /// touch is as `NavRow::expanded` says.
    expansion: HashMap<(NavView, String), bool>,
    /// Expansion the user chose while the search box narrows the view. It
    /// is forgotten when the search text changes.
    filter_expansion: HashMap<String, bool>,
    /// The row the cursor is on.
    cursor: Option<SharedString>,
    /// The rows that showed the model's scope at the last sync.
    scope_rows: Vec<String>,
    /// The store revision the pane was last brought in line with.
    revision: Option<u64>,
    /// Where the shell opens the menu the pane asks the model for.
    anchor: MenuAnchor,
    _subscriptions: Vec<Subscription>,
}

impl NavPane {
    pub fn new(ui: Ui, anchor: MenuAnchor, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = ui.store.clone();
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(SEARCH_PLACEHOLDER));
        let tree = cx.new(|cx| TreeState::new(cx));
        let subscriptions = vec![
            cx.observe_in(&store, window, |pane, _, window, cx| pane.sync(window, cx)),
            cx.subscribe_in(&search, window, |pane, input, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    let text = input.read(cx).value().to_string();
                    dispatch(&pane.store, cx, move |m| m.set_nav_filter(&text));
                }
            }),
            // The Kit's tree folds by itself when it has the keyboard and
            // when a selection is revealed: the pane keeps its record.
            cx.subscribe(&tree, |pane, _, event: &TreeEvent, cx| {
                let (id, expanded) = match event {
                    TreeEvent::Expanded(id) => (id, true),
                    TreeEvent::Collapsed(id) => (id, false),
                };
                pane.remember_expansion(id, expanded);
                pane.recompute_visible();
                cx.notify();
            }),
            // The tree has no selection event: observe it and compare. Only
            // a change the pane did not make itself is the user's.
            cx.observe(&tree, |pane, tree, cx| {
                let now = tree.read(cx).selected_item().map(|item| item.id.clone());
                if now != pane.cursor {
                    pane.cursor = now.clone();
                    if let Some(id) = now {
                        pane.select_in_model(&id, cx);
                    }
                    cx.notify();
                }
            }),
        ];
        let mut pane = Self {
            ui,
            store,
            search,
            tree,
            tree_stop: cx.focus_handle().tab_stop(true),
            rows: Vec::new(),
            view: NavView::default(),
            filter: String::new(),
            meta: Rc::new(HashMap::new()),
            visible: Vec::new(),
            expansion: HashMap::new(),
            filter_expansion: HashMap::new(),
            cursor: None,
            scope_rows: Vec::new(),
            revision: None,
            anchor,
            _subscriptions: subscriptions,
        };
        pane.sync(window, cx);
        pane
    }

    /// The Tab stop of the tree, for the shell to move the keyboard there.
    pub fn tree_focus(&self) -> &FocusHandle {
        &self.tree_stop
    }

    /// The id of the row the cursor is on.
    pub fn cursor(&self) -> Option<&str> {
        self.cursor.as_deref()
    }

    /// The ids of the rows that are not inside a folded row, in order.
    pub fn visible_rows(&self) -> Vec<String> {
        self.visible.iter().map(ToString::to_string).collect()
    }

    /// Where the context menu of a row that shows `object` opens when no
    /// pointer said where: at the row.
    pub fn menu_position(
        &self,
        object: &ObjectRef,
        window: &Window,
        cx: &App,
    ) -> Option<Point<Pixels>> {
        let row = self
            .rows
            .iter()
            .find(|row| row.object.as_ref() == Some(object))?;
        let id = SharedString::from(row.id.clone());
        self.visible
            .contains(&id)
            .then(|| self.row_position(&id, window, cx))
    }

    fn row(&self, id: &str) -> Option<&NavRow> {
        self.rows.iter().find(|row| row.id == id)
    }

    fn has_children(&self, id: &str) -> bool {
        self.rows
            .iter()
            .any(|row| row.parent.as_deref() == Some(id))
    }

    fn is_expanded(&self, row: &NavRow) -> bool {
        let chosen = if self.filter.is_empty() {
            self.expansion.get(&(self.view, row.id.clone()))
        } else {
            self.filter_expansion.get(&row.id)
        };
        chosen.copied().unwrap_or(row.expanded)
    }

    fn remember_expansion(&mut self, id: &str, expanded: bool) {
        if self.filter.is_empty() {
            self.expansion.insert((self.view, id.to_owned()), expanded);
        } else {
            self.filter_expansion.insert(id.to_owned(), expanded);
        }
    }

    /// The children of every row, in row order. The key `None` holds the
    /// roots.
    fn children(&self) -> HashMap<Option<&str>, Vec<usize>> {
        let mut children: HashMap<Option<&str>, Vec<usize>> = HashMap::new();
        for (ix, row) in self.rows.iter().enumerate() {
            children.entry(row.parent.as_deref()).or_default().push(ix);
        }
        children
    }

    fn recompute_visible(&mut self) {
        fn walk(
            pane: &NavPane,
            children: &HashMap<Option<&str>, Vec<usize>>,
            parent: Option<&str>,
            out: &mut Vec<SharedString>,
        ) {
            for ix in children.get(&parent).into_iter().flatten() {
                let row = &pane.rows[*ix];
                out.push(SharedString::from(row.id.clone()));
                if pane.is_expanded(row) {
                    walk(pane, children, Some(&row.id), out);
                }
            }
        }
        let mut visible = Vec::with_capacity(self.rows.len());
        walk(self, &self.children(), None, &mut visible);
        self.visible = visible;
    }

    fn items(&self) -> Vec<TreeItem> {
        fn build(
            pane: &NavPane,
            children: &HashMap<Option<&str>, Vec<usize>>,
            parent: Option<&str>,
        ) -> Vec<TreeItem> {
            children
                .get(&parent)
                .into_iter()
                .flatten()
                .map(|ix| {
                    let row = &pane.rows[*ix];
                    TreeItem::new(row.id.clone(), row.label.clone())
                        .expanded(pane.is_expanded(row))
                        .children(build(pane, children, Some(&row.id)))
                })
                .collect()
        }
        build(self, &self.children(), None)
    }

    fn rebuild_meta(&mut self) {
        self.meta = Rc::new(
            self.rows
                .iter()
                .map(|row| {
                    (
                        SharedString::from(row.id.clone()),
                        RowMeta {
                            icon: icons::row_icon(self.view, row.icon),
                            status: row.status.clone().map(SharedString::from),
                            object: row.object.clone(),
                            in_scope: self.scope_rows.contains(&row.id),
                        },
                    )
                })
                .collect(),
        );
    }

    /// Gives the tree the rows again and puts the cursor back by id.
    fn rebuild_tree(&mut self, cx: &mut Context<Self>) {
        let items = self.items();
        self.recompute_visible();
        let cursor = self.cursor.clone();
        let kept = self.tree.update(cx, |tree, cx| {
            // The scroll offset belongs to the state and survives.
            tree.set_items(items, cx);
            let index = cursor.as_ref().and_then(|id| tree.index_of(id));
            tree.set_selected_index(index, cx);
            index.is_some()
        });
        if !kept {
            self.cursor = None;
        }
    }

    /// Brings the pane in line with the model: the search text, the rows,
    /// and the cursor when the model's scope moved.
    pub fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let revision = self.store.read(cx).revision();
        if self.revision == Some(revision) {
            return;
        }
        self.revision = Some(revision);
        let (rows, scope_rows, view, filter) = {
            let model = self.store.read(cx).model();
            let rows = nav::rows(model);
            let scope_rows = nav::selected_ids(model, &rows);
            (rows, scope_rows, model.nav.view, model.nav.filter.clone())
        };
        let view_changed = view != self.view;
        if filter != self.filter {
            self.filter_expansion.clear();
            // The box follows the model when the model changed the text
            // (a scene, a command), never when the two merely differ: a
            // keystroke whose event is still on its way must not be undone.
            if self.search.read(cx).value().as_ref() != filter {
                let text = SharedString::from(filter.clone());
                self.search
                    .update(cx, |input, cx| input.set_value(text, window, cx));
            }
        }
        let rows_changed = rows != self.rows || view_changed || filter != self.filter;
        let scope_changed = scope_rows != self.scope_rows || view_changed;
        self.view = view;
        self.filter = filter;
        if rows_changed {
            self.rows = rows;
            if view_changed {
                // A row id of another view means nothing here.
                self.cursor = None;
            }
            self.rebuild_tree(cx);
        }
        if scope_changed {
            self.scope_rows = scope_rows;
            // The cursor follows the scope when the scope moved, and stays
            // where the user put it otherwise.
            let target = self.scope_rows.first().cloned().map(SharedString::from);
            if let Some(target) = target
                && self.cursor.as_ref() != Some(&target)
            {
                self.cursor = Some(target.clone());
                self.tree.update(cx, |tree, cx| {
                    // Unfolds the rows above it, which the pane records
                    // through the tree's events.
                    tree.set_selected_item(Some(&TreeItem::new(target.clone(), "")), cx);
                    if let Some(index) = tree.index_of(&target) {
                        tree.scroll_to_item(index, gpui_kit::ScrollStrategy::Center);
                    }
                });
            }
        }
        if rows_changed || scope_changed {
            self.rebuild_meta();
        }
        cx.notify();
    }

    fn select_in_model(&self, id: &str, cx: &mut App) {
        let id = id.to_owned();
        dispatch(&self.store, cx, move |m| m.select_nav_row(&id));
    }

    /// Puts the cursor on a row and selects what the row stands for.
    fn move_to(&mut self, id: &SharedString, cx: &mut Context<Self>) {
        let moved = self.tree.update(cx, |tree, cx| {
            let Some(index) = tree.index_of(id) else {
                return false;
            };
            tree.set_selected_index(Some(index), cx);
            tree.scroll_to_item(index, gpui_kit::ScrollStrategy::Nearest);
            true
        });
        if !moved {
            return;
        }
        // Set before the model answers: the observer of the tree then sees
        // nothing new.
        self.cursor = Some(id.clone());
        self.select_in_model(id, cx);
        cx.notify();
    }

    /// Folds or unfolds a row that has children.
    fn toggle(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(row) = self.row(id) else {
            return;
        };
        if !self.has_children(id) {
            return;
        }
        let expanded = !self.is_expanded(row);
        self.remember_expansion(id, expanded);
        self.rebuild_tree(cx);
        cx.notify();
    }

    /// A press on a row: the cursor and the selection go there; a second
    /// press in quick succession folds or unfolds it.
    fn press(
        &mut self,
        id: &SharedString,
        count: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.tree_stop.focus(window, cx);
        self.move_to(id, cx);
        if count == 2 {
            self.toggle(id, cx);
        }
    }

    fn cursor_position(&self) -> Option<usize> {
        let cursor = self.cursor.as_ref()?;
        self.visible.iter().position(|id| id == cursor)
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers;
        let plain = !modifiers.control && !modifiers.alt && !modifiers.platform;
        let menu_key = keystroke.key == "menu" || (keystroke.key == "f10" && modifiers.shift);
        if menu_key {
            if let Some(id) = self.cursor.clone() {
                self.open_menu(&id, None, window, cx);
            }
            cx.stop_propagation();
            return;
        }
        if !plain || modifiers.shift || self.visible.is_empty() {
            return;
        }
        let last = self.visible.len() - 1;
        let position = self.cursor_position();
        let target = match keystroke.key.as_str() {
            "down" => Some(position.map_or(0, |at| (at + 1).min(last))),
            "up" => Some(position.map_or(last, |at| at.saturating_sub(1))),
            "pagedown" => Some(position.map_or(0, |at| (at + PAGE).min(last))),
            "pageup" => Some(position.map_or(0, |at| at.saturating_sub(PAGE))),
            "home" => Some(0),
            "end" => Some(last),
            "right" | "left" | "enter" | "space" => None,
            _ => return,
        };
        cx.stop_propagation();
        if let Some(target) = target {
            let id = self.visible[target].clone();
            self.move_to(&id, cx);
            return;
        }
        let Some(id) = self.cursor.clone() else {
            return;
        };
        let Some(row) = self.row(&id) else {
            return;
        };
        let folder = self.has_children(&id);
        let expanded = folder && self.is_expanded(row);
        let parent = row.parent.clone();
        match keystroke.key.as_str() {
            "right" if folder && !expanded => self.toggle(&id, cx),
            "right" if folder => {
                // Already open: on to its first row.
                if let Some(next) = position.and_then(|at| self.visible.get(at + 1)).cloned() {
                    self.move_to(&next, cx);
                }
            }
            "left" if expanded => self.toggle(&id, cx),
            "left" => {
                if let Some(parent) = parent {
                    self.move_to(&SharedString::from(parent), cx);
                }
            }
            "enter" | "space" => {
                if self.meta.get(&id).is_some_and(|meta| meta.object.is_some()) {
                    self.select_in_model(&id, cx);
                } else {
                    // A heading selects nothing: it folds.
                    self.toggle(&id, cx);
                }
            }
            _ => {}
        }
    }

    /// Asks the model for the context menu of a row. `position` is where
    /// the pointer pressed; without it the menu opens at the row. The shell
    /// presents the menu the model then has open.
    fn open_menu(
        &mut self,
        id: &SharedString,
        position: Option<Point<Pixels>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(object) = self.meta.get(id).and_then(|meta| meta.object.clone()) else {
            return;
        };
        // The keys come back here when the menu closes.
        self.tree_stop.focus(window, cx);
        let position = position.unwrap_or_else(|| self.row_position(id, window, cx));
        self.anchor.request(position);
        dispatch(&self.store, cx, move |m| {
            m.open_context_menu(object, MenuOrigin::Tree);
        });
    }

    /// Where a menu opened by the keyboard appears: under the start of the
    /// row when the row is on screen, else at the top of the tree.
    fn row_position(&self, id: &SharedString, window: &Window, cx: &App) -> Point<Pixels> {
        let handle = self.tree.read(cx).scroll_handle().clone();
        let state = handle.0.borrow();
        let list = state.base_handle.bounds();
        let row_height = state
            .last_item_size
            .map(|size| size.item.height)
            .filter(|height| *height > px(0.))
            .unwrap_or_else(|| window.rem_size() * 1.75);
        let index = self
            .visible
            .iter()
            .position(|other| other == id)
            .unwrap_or(0);
        let offset = state.base_handle.offset().y;
        let y = list.top() + row_height * (index as f32 + 1.) + offset;
        point(
            list.left() + px(24.),
            y.clamp(list.top(), list.bottom().max(list.top())),
        )
    }

    fn empty_text(&self) -> &'static str {
        if !self.filter.is_empty() {
            return "No matching objects. Clear the search to see everything.";
        }
        match self.view {
            NavView::Infrastructure => {
                "No servers yet. Add a server to see its pool, hosts and virtual machines here."
            }
            NavView::Objects => "No objects yet. Connect to a server to see its objects by type.",
            NavView::Organization => "No folders, tags or vApps to show.",
            NavView::SavedSearches => "No saved searches.",
            NavView::Notifications => "No notifications.",
        }
    }

    fn search_box(&self, cx: &App) -> AnyElement {
        let border = cx.theme().border;
        div()
            .w_full()
            .flex_none()
            .p_2()
            .border_b_1()
            .border_color(border)
            .child(
                Input::new(&self.search)
                    .id(SEARCH_KEY)
                    .aria_label(SEARCH_LABEL)
                    .small()
                    .cleanable(true)
                    .prefix(Icon::new(IconName::Search).small()),
            )
            .into_any_element()
    }

    fn tree_area(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let ring = theme.ring;
        let muted = theme.muted_foreground;
        let content = if self.rows.is_empty() {
            let text = SharedString::from(self.empty_text());
            div()
                .id(EMPTY_KEY)
                .test_support()
                .role(Role::Label)
                .aria_label(text.clone())
                .size_full()
                .p_4()
                .text_sm()
                .text_color(muted)
                .child(text)
                .into_any_element()
        } else {
            let pane = cx.weak_entity();
            let meta = Rc::clone(&self.meta);
            tree(&self.tree, move |index, entry, selected, _, cx| {
                tree_row(&pane, &meta, index, entry, selected, cx)
            })
            .into_any_element()
        };
        div()
            .id(TREE_KEY)
            .test_support()
            .track_focus(&self.tree_stop)
            .role(Role::Tree)
            .aria_label(TREE_LABEL)
            .w_full()
            .flex_1()
            .min_h_0()
            .py_1()
            .border_1()
            .border_color(gpui_kit::transparent_black())
            .focus_visible(move |style| style.border_color(ring))
            .on_key_down(cx.listener(Self::on_key))
            .child(content)
            .into_any_element()
    }

    fn view_switcher(&self, cx: &mut Context<Self>) -> AnyElement {
        let handles: Vec<FocusHandle> = NavView::ALL
            .iter()
            .map(|view| self.ui.focus_handle(SCOPE, view.key(), cx))
            .collect();
        let theme = cx.theme();
        let border = theme.border;
        let hover = theme.accent;
        let active = theme.list_active;
        let foreground = theme.foreground;
        let muted = theme.muted_foreground;
        let ring = theme.ring;
        let radius = theme.radius;
        v_flex()
            .id(SWITCHER_KEY)
            .test_support()
            .role(Role::TabList)
            .aria_label("Navigation views")
            .aria_value(self.view.title())
            .w_full()
            .flex_none()
            .p_1()
            .gap_0p5()
            .border_t_1()
            .border_color(border)
            .children(NavView::ALL.into_iter().zip(handles).map(|(view, handle)| {
                let selected = view == self.view;
                let store = self.store.clone();
                pressable(view.key(), &handle, move |_, cx| {
                    dispatch(&store, cx, move |m| m.set_nav_view(view));
                })
                .role(Role::Tab)
                .aria_label(view.title())
                .aria_selected(selected)
                .flex()
                .items_center()
                .gap_2()
                .px_2()
                .py_1()
                .rounded(radius)
                .border_1()
                .border_color(gpui_kit::transparent_black())
                .text_sm()
                .text_color(if selected { foreground } else { muted })
                .when(selected, |row| row.bg(active).font_medium())
                .hover(move |style| style.bg(if selected { active } else { hover }))
                .focus_visible(move |style| style.border_color(ring))
                .child(Icon::new(icons::view_icon(view)).small())
                .child(view.title())
            }))
            .into_any_element()
    }
}

/// The colour of a row's icon: what is running is green, what failed is
/// red, the rest is quiet.
fn state_color(status: Option<&str>, cx: &App) -> Hsla {
    use xenadmin_ui::surface::Tone;
    let tone = match status {
        Some("Running") => Tone::Success,
        Some("Paused" | "Suspended") => Tone::Warning,
        Some("Connection failed") => Tone::Danger,
        _ => Tone::Muted,
    };
    tone_color(tone, cx)
}

/// One row of the tree: indentation, chevron, icon, label, status word.
fn tree_row(
    pane: &WeakEntity<NavPane>,
    meta: &HashMap<SharedString, RowMeta>,
    index: usize,
    entry: &TreeEntry,
    selected: bool,
    cx: &App,
) -> ListItem {
    let item = entry.item();
    let id = item.id.clone();
    let row = meta.get(&id);
    let theme = cx.theme();
    let muted = theme.muted_foreground;
    let hover = theme.accent;
    let scope = theme.list_active;
    let icon = row.map_or(Lucide::Inbox, |row| row.icon);
    let status = row.and_then(|row| row.status.clone());
    let in_scope = row.is_some_and(|row| row.in_scope);
    let heading = row.is_some_and(|row| row.object.is_none());
    let chevron = if entry.is_folder() {
        let pane = pane.clone();
        let id = id.clone();
        div()
            .id((FOLD_KEY, index))
            .test_support()
            .flex_none()
            .rounded_sm()
            .text_color(muted)
            .hover(move |style| style.bg(hover))
            .on_click(move |_, _, cx| {
                cx.stop_propagation();
                let _ = pane.update(cx, |pane, cx| pane.toggle(&id, cx));
            })
            .child(
                Icon::new(if entry.is_expanded() {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                })
                .small(),
            )
            .into_any_element()
    } else {
        div().flex_none().size_3p5().into_any_element()
    };
    let on_press = {
        let pane = pane.clone();
        let id = id.clone();
        move |event: &ClickEvent, window: &mut Window, cx: &mut App| {
            // The tree's own press selects and folds: this one only selects.
            cx.stop_propagation();
            let count = event.click_count();
            let _ = pane.update(cx, |pane, cx| pane.press(&id, count, window, cx));
        }
    };
    let on_menu = {
        let pane = pane.clone();
        let id = id.clone();
        move |event: &MouseDownEvent, window: &mut Window, cx: &mut App| {
            cx.stop_propagation();
            let position = event.position;
            let _ = pane.update(cx, |pane, cx| {
                pane.open_menu(&id, Some(position), window, cx);
            });
        }
    };
    ListItem::new(id)
        .text_sm()
        .py_0p5()
        .pr_2()
        .pl(rems(0.375 + 0.875 * entry.depth() as f32))
        // The scope keeps its mark while the cursor is elsewhere.
        .when(in_scope && !selected, |row| row.bg(scope))
        .on_click(on_press)
        .on_mouse_down(MouseButton::Right, on_menu)
        .child(
            h_flex()
                .w_full()
                .gap_1p5()
                .child(chevron)
                .child(
                    Icon::new(icon)
                        .small()
                        .flex_none()
                        .text_color(state_color(status.as_deref(), cx)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .when(heading, |label| label.text_color(muted))
                        .child(item.label.clone()),
                )
                .when_some(status, |row, status| {
                    row.child(div().flex_none().text_xs().text_color(muted).child(status))
                }),
        )
}

impl Render for NavPane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let background = theme.sidebar;
        let foreground = theme.sidebar_foreground;
        let border = theme.border;
        v_flex()
            .id(PANE_KEY)
            .test_support()
            .role(Role::Navigation)
            .aria_label("Sidebar")
            .size_full()
            .min_w_0()
            .border_r_1()
            .border_color(border)
            .bg(background)
            .text_color(foreground)
            .child(self.search_box(cx))
            .child(self.tree_area(cx))
            .child(self.view_switcher(cx))
    }
}
