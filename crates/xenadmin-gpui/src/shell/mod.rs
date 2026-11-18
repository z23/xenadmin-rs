//! The shell: the root view of the main window.
//!
//! ```text
//! title bar: [pane toggle] File View Pool Server VM Storage ...        title
//! toolbar:   Back Forward | Add Server  New Storage  New VM | power commands
//! +------------------+------------------------------------------------------+
//! | search box       | object header                                        |
//! | tree             | tab strip                                            |
//! |                  | local action row, tab body                           |
//! | view switcher    |                                                      |
//! +------------------+------------------------------------------------------+
//! notices; status line: connection | operations | unknown outcomes | selection
//! ```
//!
//! Everything shown comes from the model through the store: the menus, the
//! toolbar and the context menus from the command table, the tree from the
//! navigation rows, the header, the tabs and the status bar from their
//! projections. The shell owns only widget state: which rows are folded,
//! where the cursor is, how wide the pane is being dragged.
//!
//! No region holds an action button of its own: an action is a command in
//! the menu bar, the toolbar, a context menu or a tab's local action row,
//! or a button of a dialog.

pub mod actions;
pub mod icons;
pub mod menus;
pub mod nav;
pub mod status;
pub mod toolbar;

use std::rc::Rc;

use xenadmin_ui::command::{CommandId, MenuView};
use xenadmin_ui::model::APPLICATION_NAME;
use xenadmin_ui::object::ObjectRef;
use xenadmin_ui::prefs::{Appearance, Preferences};
use xenadmin_ui::selection::MenuOrigin;

use self::actions::{RunCommand, SHELL_CONTEXT};
use self::menus::MenuAnchor;
use self::nav::NavPane;
use crate::intents::Hooks;
use crate::kit::*;
use crate::render::{ColumnWidths, DialogHost, Ui};
use crate::scene_view::{object_header, tab_body, tab_strip};
use crate::store::{AppStore, dispatch};
use crate::theme;

/// The id of the shell's root element.
pub const ROOT_KEY: &str = "application";
/// The id of the button that shows and hides the navigation pane in a
/// narrow window.
pub const TOGGLE_KEY: &str = "sidebar-toggle";
/// The id of the title in the title bar.
pub const TITLE_KEY: &str = "window-title";
/// The id of the split between the navigation pane and the content.
pub const SPLIT_KEY: &str = "split";
/// The id of the content column: header, tabs, tab body.
pub const CONTENT_KEY: &str = "main-column";

/// Below this width, at an interface scale of 100, the navigation pane is
/// hidden behind its toggle.
pub const NARROW_WIDTH: f32 = 1100.0;

/// The inventory grid of the Search tab: the table whose column widths the
/// preferences keep under the column ids of a Rune preferences file. Every
/// other table keeps its widths too (`Model::set_column_width`).
pub const INVENTORY_GRID: &str = xenadmin_ui::model::INVENTORY_TABLE;

/// The title of the window: the model's title, and the application's name
/// after it when the title is something else.
pub fn window_title(title: &str) -> String {
    if title == APPLICATION_NAME {
        title.to_owned()
    } else {
        format!("{title} - {APPLICATION_NAME}")
    }
}

/// The context menu the shell shows: the one the model has open.
struct OpenMenu {
    menu: Entity<PopupMenu>,
    position: Point<Pixels>,
    /// The model's menu it was built from.
    key: (MenuOrigin, ObjectRef),
    /// The count of positioned requests when it was presented.
    request: u64,
    /// What had the keyboard before the menu took it.
    restore: Option<FocusHandle>,
    _dismiss: Subscription,
}

pub struct ShellView {
    ui: Ui,
    hooks: Hooks,
    dialogs: Entity<DialogHost>,
    nav: Entity<NavPane>,
    menu_bar: Entity<AppMenuBar>,
    split: Entity<ResizableState>,
    focus: FocusHandle,
    /// The menus last given to the platform and to the Kit's bar.
    installed: Vec<MenuView>,
    /// The appearance and the interface scale the theme was last set from.
    look: Option<(Appearance, u16)>,
    title: String,
    /// In a narrow window: whether the user opened the navigation pane.
    nav_open: bool,
    /// Whether the appearance `System` means the desktop's (the
    /// application) or whatever the theme is (a scene, a test).
    follows_system: bool,
    menu: Option<OpenMenu>,
    menu_anchor: MenuAnchor,
    _subscriptions: Vec<Subscription>,
}

impl ShellView {
    /// The shell over `store`. `hooks.quit` is the close path it runs when
    /// the window is asked to close.
    pub fn new(
        store: Entity<AppStore>,
        hooks: Hooks,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // What `Intent::ToggleFullscreen(WindowRef::Main)` toggles.
        hooks.main_window.set(Some(window.window_handle()));
        let ui = Ui::new(store.clone());
        // The bespoke elements of a tab body: the console canvas and the
        // performance charts.
        crate::console::register(&ui);
        crate::charts::register(&ui);
        // Every table keeps the widths the user gave its columns, by the
        // table's key and the column's id.
        ui.persist_column_widths(ColumnWidths {
            get: Rc::new(|model, table, column| model.column_width(table, column)),
            set: Rc::new(|model, table, column, width| {
                model.set_column_width(table, column, Some(width));
            }),
        });
        let dialogs = cx.new(|cx| DialogHost::new(ui.clone(), window, cx));
        let menu_anchor = MenuAnchor::default();
        // A row of a table that stands for an object says where its
        // context menu was asked for, as a row of the tree does.
        ui.on_menu_request({
            let anchor = menu_anchor.clone();
            move |position| anchor.request(position)
        });
        let nav = cx.new(|cx| NavPane::new(ui.clone(), menu_anchor.clone(), window, cx));
        let installed = store.read(cx).menu_bar();
        menus::install(&installed, cx);
        let menu_bar = AppMenuBar::new(cx);
        let split = cx.new(|_| ResizableState::default());
        let focus = cx.focus_handle();
        let subscriptions = vec![
            cx.observe_in(&store, window, |shell, _, window, cx| {
                shell.sync(window, cx);
            }),
            cx.subscribe(&split, |shell, _, _: &ResizablePanelEvent, cx| {
                shell.pane_resized(cx);
            }),
            // Leaving the window lets go of what a console holds down in
            // its guest (the Rune `App::on_focus`).
            cx.observe_window_activation(window, |shell, window, cx| {
                if !window.is_window_active() {
                    (shell.hooks.release_console_input)(cx);
                }
            }),
        ];
        // The window manager asks to close (the close button of a server
        // decoration, Alt+F4): the close path runs first.
        let quit = Rc::clone(&hooks.quit);
        window.on_window_should_close(cx, move |_, cx| {
            quit(cx);
            true
        });
        // While a dialog has the keyboard the shell's root is not on the
        // focus path. The interface size commands still apply there.
        let weak = store.downgrade();
        let app: &mut App = cx;
        app.on_action(move |action: &RunCommand, cx: &mut App| {
            let id = CommandId(action.id);
            if actions::is_zoom(id) {
                let _ = weak.update(cx, |store, cx| store.invoke(id, cx));
            }
        });
        // Bindings with a context match only once something inside the
        // shell has the keyboard.
        focus.focus(window, cx);
        let mut shell = Self {
            ui,
            hooks,
            dialogs,
            nav,
            menu_bar,
            split,
            focus,
            installed,
            look: None,
            title: String::new(),
            nav_open: false,
            follows_system: false,
            menu: None,
            menu_anchor,
            _subscriptions: subscriptions,
        };
        shell.sync(window, cx);
        shell
    }

    /// Makes the theme follow the desktop's light or dark appearance while
    /// the preference says `System`: now, and whenever the desktop changes
    /// it. The application calls this; a scene or a test does not, so the
    /// theme it asked for stays.
    pub fn follow_system_appearance(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.follows_system = true;
        let follow = |shell: &mut Self, window: &mut Window, cx: &mut Context<Self>| {
            // `sync` reads the desktop's appearance when the preference is
            // `System`, and applies the accent again over the mode's theme.
            shell.look = None;
            shell.sync(window, cx);
        };
        follow(self, window, cx);
        self._subscriptions
            .push(cx.observe_window_appearance(window, follow));
    }

    pub fn ui(&self) -> &Ui {
        &self.ui
    }

    pub fn dialogs(&self) -> &Entity<DialogHost> {
        &self.dialogs
    }

    pub fn nav(&self) -> &Entity<NavPane> {
        &self.nav
    }

    /// The split between the navigation pane and the content.
    pub fn split(&self) -> &Entity<ResizableState> {
        &self.split
    }

    /// Whether a context menu is shown.
    pub fn menu_open(&self) -> bool {
        self.menu.is_some()
    }

    /// The title the window was last given.
    pub fn title(&self) -> &str {
        &self.title
    }

    fn zoom(&self, cx: &App) -> f32 {
        self.ui.store.read(cx).model().preferences.options.zoom()
    }

    /// Brings what the shell keeps outside its render in line with the
    /// model: the theme, the window title and the menus.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (options, title, menu_bar) = {
            let store = self.ui.store.read(cx);
            let model = store.model();
            (
                model.preferences.options,
                model.window_title(),
                store.menu_bar(),
            )
        };
        let look = (options.appearance, options.interface_scale);
        if self.look != Some(look) {
            let scaled = self
                .look
                .is_some_and(|(_, scale)| scale != options.interface_scale);
            self.look = Some(look);
            if self.follows_system && options.appearance == Appearance::System {
                Theme::sync_system_appearance(Some(window), cx);
            }
            theme::apply_options(options, cx);
            if scaled {
                self.rescale_pane(window, cx);
            }
        }
        if title != self.title {
            window.set_window_title(&window_title(&title));
            self.title = title;
        }
        if menu_bar != self.installed {
            self.installed = menu_bar;
            menus::install(&self.installed, cx);
            // Reloading inside the handler of an action that came from the
            // bar leaves the keyboard nowhere: after the update.
            let bar = self.menu_bar.clone();
            cx.defer_in(window, move |_, _, cx| {
                bar.update(cx, |bar, cx| bar.reload(cx));
            });
        }
        self.sync_menu(window, cx);
        cx.notify();
    }

    /// Keeps the context menu on screen equal to the one the model has
    /// open: a row asked for it (`Model::open_context_menu`), and choosing
    /// an item, another selection, another tab or another session ends it.
    fn sync_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let wanted = self
            .ui
            .store
            .read(cx)
            .model()
            .selection
            .menu
            .as_ref()
            .map(|menu| (menu.origin, menu.object.clone()));
        let requests = self.menu_anchor.requests();
        match (&self.menu, wanted) {
            (Some(open), Some(key)) if open.key == key && open.request == requests => {}
            (_, Some(key)) => self.present_menu(key, window, cx),
            (Some(_), None) => {
                self.end_menu(window, cx);
            }
            (None, None) => {}
        }
    }

    fn present_menu(
        &mut self,
        key: (MenuOrigin, ObjectRef),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A menu that is replaced gives the keyboard back first, so the
        // new one remembers the right place.
        self.end_menu(window, cx);
        let groups = self.ui.store.read(cx).model().context_menu();
        if groups.iter().all(Vec::is_empty) {
            // Nothing applies to the row: no empty box.
            dispatch(&self.ui.store, cx, |m| m.close_context_menu());
            return;
        }
        let position = self
            .menu_anchor
            .take_position()
            .unwrap_or_else(|| self.default_menu_position(&key, window, cx));
        let restore = window.focused(cx);
        let store = self.ui.store.clone();
        let menu = PopupMenu::build(window, cx, move |mut menu, window, _| {
            // A guest's menu holds every area's commands and can be taller
            // than a small window: it scrolls inside the window instead.
            let room = window.viewport_size().height - px(16.);
            menu = menu.scrollable(true).max_h(room);
            for (index, group) in groups.iter().enumerate() {
                if index > 0 {
                    menu = menu.separator();
                }
                for item in group {
                    menu = menu.item(menus::context_item(&store, item));
                }
            }
            menu
        });
        let dismiss = cx.subscribe_in(&menu, window, |shell, _, _: &DismissEvent, window, cx| {
            shell.menu_dismissed(window, cx);
        });
        menu.focus_handle(cx).focus(window, cx);
        self.menu = Some(OpenMenu {
            menu,
            position,
            key,
            request: self.menu_anchor.requests(),
            restore,
            _dismiss: dismiss,
        });
    }

    /// Takes the menu off the screen and gives the keyboard back to what
    /// had it, unless a press elsewhere has already moved it on.
    fn end_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<u64> {
        let open = self.menu.take()?;
        let held = open.menu.focus_handle(cx).contains_focused(window, cx);
        if held || window.focused(cx).is_none() {
            match &open.restore {
                Some(handle) => handle.focus(window, cx),
                None => self.focus.focus(window, cx),
            }
        }
        cx.notify();
        Some(open.request)
    }

    /// The menu closed itself: Escape, a press outside it, or an item was
    /// chosen. The model's menu is over too, unless a newer one was asked
    /// for in the same press.
    fn menu_dismissed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(request) = self.end_menu(window, cx) else {
            return;
        };
        if request == self.menu_anchor.requests() {
            dispatch(&self.ui.store, cx, |m| m.close_context_menu());
        } else {
            self.sync_menu(window, cx);
        }
    }

    /// Where a menu opens that no pointer placed: at the row of the tree
    /// for a menu of the tree, else beside the pane at mid height. Only a
    /// menu the model opened by itself (a scene) comes here for a grid.
    fn default_menu_position(
        &self,
        (origin, object): &(MenuOrigin, ObjectRef),
        window: &Window,
        cx: &App,
    ) -> Point<Pixels> {
        let at_row = (*origin == MenuOrigin::Tree)
            .then(|| self.nav.read(cx).menu_position(object, window, cx))
            .flatten();
        at_row.unwrap_or_else(|| {
            let viewport = window.viewport_size();
            let pane = self
                .split
                .read(cx)
                .sizes()
                .first()
                .copied()
                .unwrap_or(px(0.));
            point(
                (pane + window.rem_size() * 3.).min(viewport.width * 0.5),
                viewport.height * 0.5,
            )
        })
    }

    /// The pane keeps its width in the units of the interface scale: when
    /// the scale changes, its width on screen follows.
    fn rescale_pane(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.split.read(cx).sizes().len() < 2 {
            return;
        }
        let model = self.ui.store.read(cx).model();
        let width = px(model.preferences.sidebar_width * model.preferences.options.zoom());
        self.split
            .update(cx, |split, cx| split.resize_panel(0, width, window, cx));
    }

    /// The user dragged the divider (or the pane was resized from code):
    /// the model keeps the width and writes it once it rests.
    fn pane_resized(&mut self, cx: &mut Context<Self>) {
        let Some(width) = self.split.read(cx).sizes().first().copied() else {
            return;
        };
        let width = f32::from(width) / self.zoom(cx);
        dispatch(&self.ui.store, cx, move |m| m.set_sidebar_width(width));
    }

    fn run_command(&mut self, action: &RunCommand, _: &mut Window, cx: &mut Context<Self>) {
        let id = CommandId(action.id);
        self.ui.store.update(cx, |store, cx| store.invoke(id, cx));
    }

    fn toggle_pane(&mut self, cx: &mut Context<Self>) {
        self.nav_open = !self.nav_open;
        cx.notify();
    }

    fn title_bar(&self, narrow: bool, cx: &mut Context<Self>) -> TitleBar {
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let title = SharedString::from(self.title.clone());
        let toggle = narrow.then(|| {
            let label = if self.nav_open {
                "Hide sidebar"
            } else {
                "Show sidebar"
            };
            div()
                // A press in the title bar would start to move the window.
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|shell, _, window, cx| {
                        window.prevent_default();
                        cx.stop_propagation();
                        shell.toggle_pane(cx);
                    }),
                )
                .child(
                    Button::new(TOGGLE_KEY)
                        .ghost()
                        .small()
                        .icon(IconName::PanelLeft)
                        .selected(self.nav_open)
                        .accessibility_label(label)
                        .tooltip(label)
                        .on_click(cx.listener(|shell, event: &ClickEvent, _, cx| {
                            // The pointer already toggled on its way down.
                            if !matches!(event, ClickEvent::Mouse(_)) {
                                shell.toggle_pane(cx);
                            }
                        })),
                )
        });
        let quit = Rc::clone(&self.hooks.quit);
        TitleBar::new()
            // The Kit's bar is 34 px high whatever the font: at a large
            // interface scale the menus would not fit in it.
            .h(rems(2.125))
            // The close button of a client-side decoration.
            .on_close_window(move |_, _, cx| quit(cx))
            .child(
                h_flex()
                    .h_full()
                    .flex_none()
                    .gap_1()
                    .children(toggle)
                    .child(self.menu_bar.clone()),
            )
            .child(
                div()
                    .id(TITLE_KEY)
                    .test_support()
                    .role(Role::Label)
                    .aria_label(title.clone())
                    .min_w_0()
                    .truncate()
                    .px_3()
                    .text_sm()
                    .text_color(muted)
                    .child(title),
            )
    }
}

impl Focusable for ShellView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for ShellView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (show_toolbar, sidebar_width, zoom) = {
            let model = self.ui.store.read(cx).model();
            (
                model.nav.show_toolbar,
                model.preferences.sidebar_width,
                model.preferences.options.zoom(),
            )
        };
        let viewport = window.viewport_size();
        // The interface scale makes everything larger, so a window is
        // narrow sooner (the Rune rule: the layout size decides).
        let narrow = viewport.width < px(NARROW_WIDTH * zoom);
        let show_nav = !narrow || self.nav_open;
        let pane_width = self
            .split
            .read(cx)
            .sizes()
            .first()
            .copied()
            .unwrap_or(px(sidebar_width * zoom));
        let content_width = if show_nav {
            (viewport.width - pane_width).max(px(120.))
        } else {
            viewport.width
        };
        let theme = cx.theme();
        let background = theme.background;
        let foreground = theme.foreground;

        let content = v_flex()
            .id(CONTENT_KEY)
            .test_support()
            .size_full()
            .min_w_0()
            .child(object_header(&self.ui, cx))
            .child(tab_strip(&self.ui, cx))
            .child(tab_body(&self.ui, content_width, window, cx));
        let body = if show_nav {
            let (low, high) = Preferences::SIDEBAR_RANGE;
            h_resizable(SPLIT_KEY)
                .with_state(&self.split)
                .child(
                    resizable_panel()
                        .size(px(sidebar_width * zoom))
                        .size_range(px(low * zoom)..px(high * zoom))
                        .fixed()
                        .child(self.nav.clone()),
                )
                .child(resizable_panel().child(content))
                .into_any_element()
        } else {
            content.into_any_element()
        };
        let title_bar = self.title_bar(narrow, cx);
        let toolbar = show_toolbar.then(|| toolbar::toolbar(&self.ui, cx));
        let status = status::status_bar(&self.ui, cx);
        let menu = self.menu.as_ref().map(|open| {
            deferred(
                anchored()
                    .position(open.position)
                    .snap_to_window_with_margin(px(8.))
                    .child(open.menu.clone()),
            )
            .with_priority(1)
        });
        v_flex()
            .id(ROOT_KEY)
            .test_support()
            .track_focus(&self.focus)
            .key_context(SHELL_CONTEXT)
            .on_action(cx.listener(Self::run_command))
            .size_full()
            .bg(background)
            .text_color(foreground)
            .child(title_bar)
            .children(toolbar)
            .child(div().w_full().flex_1().min_h_0().child(body))
            .child(status)
            .children(menu)
    }
}
