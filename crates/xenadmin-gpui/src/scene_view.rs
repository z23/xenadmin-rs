//! The object header, the tab strip, the local action row and the tab body,
//! and a root view of them.
//!
//! [`SceneView`] is what the scene tool and the renderer tests show: the
//! content area of the window without the shell around it. The shell wraps
//! the same pieces: each is a function of the [`Ui`] that returns an
//! element, so the shell's own root view calls them where it wants them and
//! owns a [`DialogHost`] as this view does.

use xenadmin_ui::command::MenuItem;
use xenadmin_ui::tabs::{TabBody, TabId};

use crate::kit::*;
use crate::render::{DialogHost, Pass, Ui, blocks, pressable};
use crate::shell::icons::row_icon;
use crate::store::{AppStore, dispatch};
use crate::widgets;

/// The id of the object header.
pub const HEADER_KEY: &str = "object-header";
/// The id of the tab strip.
pub const TABS_KEY: &str = "object-tabs";
/// The id of the local action row.
pub const LOCAL_ROW_KEY: &str = "local-actions";
/// The id of the tab body.
pub const TAB_BODY_KEY: &str = "tab-body";
/// The element a tab body that scrolls is seen through. A body that ends in
/// a table has none: the table scrolls its own rows.
pub const TAB_SCROLL_KEY: &str = "tab-scroll";
/// The id of the text shown when nothing provides the active tab.
pub const TAB_EMPTY_KEY: &str = "tab-empty";
/// The path prefix of the tab body in the flattened description.
pub const TAB_BASE: &str = "tab";

/// The scope of the widget state of the header and the tab strip.
const CHROME_SCOPE: &str = "chrome";
/// The padding around the tab body.
const BODY_PADDING: f32 = 16.0;

/// The header of the selected object: its icon, its name, its state and
/// where it is.
pub fn object_header(ui: &Ui, cx: &App) -> AnyElement {
    let (header, view) = {
        let model = ui.store.read(cx).model();
        (model.header(), model.nav.view)
    };
    let theme = cx.theme();
    let muted = theme.muted_foreground;
    let name = SharedString::from(header.name.clone());
    let place: Vec<String> = [
        header.pool.as_ref().map(|pool| format!("Pool {pool}")),
        header.host.as_ref().map(|host| format!("Host {host}")),
    ]
    .into_iter()
    .flatten()
    .collect();
    let description = [header.state.clone().unwrap_or_default(), place.join(", ")]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(", ");
    h_flex()
        .id(HEADER_KEY)
        .test_support()
        .role(Role::Heading)
        .aria_label(name.clone())
        .when(!description.is_empty(), |row| {
            row.aria_description(SharedString::from(description))
        })
        .w_full()
        .flex_none()
        .gap_3()
        .px_4()
        .pt_3()
        .pb_2()
        .child(
            Icon::new(row_icon(view, header.icon))
                .size_5()
                .text_color(muted),
        )
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_lg()
                .font_semibold()
                .child(name),
        )
        .when_some(header.state.clone(), |row, state| {
            row.child(
                div()
                    .flex_none()
                    .px_2()
                    .py_0p5()
                    .rounded(theme.radius)
                    .border_1()
                    .border_color(theme.border)
                    .text_xs()
                    .text_color(muted)
                    .child(SharedString::from(state)),
            )
        })
        .when(!place.is_empty(), |row| {
            row.child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .text_color(muted)
                    .child(SharedString::from(place.join("  •  "))),
            )
        })
        .into_any_element()
}

/// The tab strip: the tabs of the model, the active one marked. A press
/// chooses the tab in the model.
pub fn tab_strip(ui: &Ui, cx: &mut App) -> AnyElement {
    let (tabs, active) = {
        let model = ui.store.read(cx).model();
        (model.tabs(), model.active_tab())
    };
    let handles: std::collections::HashMap<&'static str, FocusHandle> = tabs
        .iter()
        .map(|tab| (tab.key(), ui.focus_handle(CHROME_SCOPE, tab.key(), cx)))
        .collect();
    let theme = cx.theme();
    let border = theme.border;
    let primary = theme.primary;
    let foreground = theme.foreground;
    let muted = theme.muted_foreground;
    let hover = theme.accent;
    let ring = theme.ring;
    h_flex()
        .id(TABS_KEY)
        .test_support()
        .role(Role::TabList)
        .aria_label("Tabs")
        .aria_value(active.title())
        .w_full()
        .flex_none()
        .px_3()
        .border_b_1()
        .border_color(border)
        .children(tabs.iter().map(|tab| {
            let tab = *tab;
            let selected = tab == active;
            let store = ui.store.clone();
            pressable(tab.key(), &handles[tab.key()], move |_, cx| {
                dispatch(&store, cx, move |m| m.set_active_tab(tab));
            })
            .role(Role::Tab)
            .aria_label(tab.title())
            .aria_selected(selected)
            .px_3()
            .py_2()
            .text_sm()
            .border_b_2()
            .border_color(if selected {
                primary
            } else {
                gpui_kit::transparent_black()
            })
            .text_color(if selected { foreground } else { muted })
            .when(selected, |tab| tab.font_medium())
            .hover(|style| style.bg(hover))
            .focus_visible(move |style| style.bg(hover).border_color(ring))
            .child(tab.title())
        }))
        .into_any_element()
}

/// One button of the local action row: its key is the command id, and it
/// invokes the command on the model's target.
fn local_button(store: &Entity<AppStore>, item: &MenuItem) -> Button {
    let command = item.id;
    let base = Button::new(SharedString::from(command.0))
        .label(SharedString::from(item.label.clone()))
        .outline()
        .small();
    // A toggle says its state three ways: the pressed state for assistive
    // technology, the selected fill, and a check mark, because the fill of
    // an outline button alone is easy to miss.
    let base = match item.checked {
        Some(true) => base.selected(true).toggled(true).icon(IconName::Check),
        Some(false) => base.toggled(false),
        None => base,
    };
    match &item.disabled {
        Some(reason) => base
            .disabled(true)
            .tooltip(SharedString::from(reason.clone())),
        None => {
            let store = store.clone();
            base.on_click(move |_, _, cx| {
                store.update(cx, |store, cx| store.invoke(command, cx));
            })
        }
    }
}

/// The local action row of a tab: the commands registered for it, resolved
/// on the model's target. `None` when the tab has none.
pub fn local_actions(ui: &Ui, tab: TabId, cx: &App) -> Option<AnyElement> {
    let items = ui.store.read(cx).local_row(tab);
    if items.is_empty() {
        return None;
    }
    let border = cx.theme().border;
    Some(
        h_flex()
            .id(LOCAL_ROW_KEY)
            .test_support()
            .role(Role::Toolbar)
            .aria_label("Actions")
            .w_full()
            .flex_none()
            .flex_wrap()
            .gap_2()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(border)
            .children(items.iter().map(|item| local_button(&ui.store, item)))
            .into_any_element(),
    )
}

/// The local action row and the body of the active tab, in the height the
/// parent has left. `width` is the width the body is given.
///
/// A body that ends in a table gives the table the height that is left;
/// any other body scrolls.
pub fn tab_body(ui: &Ui, width: Pixels, window: &mut Window, cx: &mut App) -> AnyElement {
    let (tab, body) = {
        let store = ui.store.read(cx);
        let tab = store.model().active_tab();
        (tab, store.tab_body(tab))
    };
    let scope = format!("{}:{}", crate::render::TAB_SCOPE, tab.key());
    ui.cache.borrow_mut().keep_tab_scope(&scope, window, cx);
    let inner = (width - px(BODY_PADDING * 2.)).max(px(120.));
    let pass = Pass {
        ui,
        scope: &scope,
        width: inner,
        fill: true,
        on_cancel: None,
    };
    let muted = cx.theme().muted_foreground;
    let local = local_actions(ui, tab, cx);
    ui.cache.borrow_mut().begin(&scope);
    let content = match body {
        None => {
            let text = SharedString::from(format!(
                "The {} tab is not available for this selection yet.",
                tab.title()
            ));
            div()
                .id(TAB_EMPTY_KEY)
                .test_support()
                .role(Role::Label)
                .aria_label(text.clone())
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .p_4()
                .text_sm()
                .text_color(muted)
                .child(text)
                .into_any_element()
        }
        Some(TabBody::Custom(custom)) => {
            let path = crate::render::element_id(TAB_BASE, custom.key.as_ref(), 0);
            let element = match ui.custom(&custom.name) {
                Some(render) => render(&custom, ui, window, cx),
                None => widgets::missing_custom(path, &custom, cx),
            };
            div().size_full().p_4().child(element).into_any_element()
        }
        Some(TabBody::Blocks(items)) => {
            let children = blocks(&pass, TAB_BASE, &items, window, cx);
            // The element with the id is what a scrolling body is seen
            // through. A body whose last table fills the height scrolls as
            // well: where the window is too short for what stands above the
            // table and the table's least height, the table would otherwise
            // be drawn below the window, out of reach.
            div()
                .id(TAB_SCROLL_KEY)
                .test_support()
                .size_full()
                .overflow_hidden()
                .child(
                    v_flex()
                        .size_full()
                        .overflow_y_scrollbar()
                        .p_4()
                        .gap_3()
                        .children(children),
                )
                .into_any_element()
        }
    };
    ui.cache.borrow_mut().end(&scope, window, cx);
    v_flex()
        .id(TAB_BODY_KEY)
        .test_support()
        .role(Role::TabPanel)
        .aria_label(tab.title())
        .w_full()
        .flex_1()
        .min_h_0()
        .children(local)
        .child(div().w_full().flex_1().min_h_0().child(content))
        .into_any_element()
}

/// The content area of the window: the object header, the tab strip, the
/// active tab's body, and the surfaces as dialogs above them.
pub struct SceneView {
    ui: Ui,
    dialogs: Entity<DialogHost>,
    focus: FocusHandle,
    _subscription: Subscription,
}

impl SceneView {
    /// A view of `store` with a renderer of its own.
    pub fn new(store: Entity<AppStore>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let ui = Ui::new(store);
        crate::console::register(&ui);
        crate::charts::register(&ui);
        Self::with_ui(ui, window, cx)
    }

    /// A view that draws with `ui` (and so with the bespoke elements
    /// registered on it).
    pub fn with_ui(ui: Ui, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let dialogs = cx.new(|cx| DialogHost::new(ui.clone(), window, cx));
        let subscription = cx.observe(&ui.store, |_, _, cx| cx.notify());
        Self {
            ui,
            dialogs,
            focus: cx.focus_handle(),
            _subscription: subscription,
        }
    }

    pub fn ui(&self) -> &Ui {
        &self.ui
    }

    pub fn dialogs(&self) -> &Entity<DialogHost> {
        &self.dialogs
    }
}

impl Focusable for SceneView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for SceneView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let background = theme.background;
        let foreground = theme.foreground;
        let width = window.viewport_size().width;
        v_flex()
            .id("scene")
            .test_support()
            .track_focus(&self.focus)
            .size_full()
            .bg(background)
            .text_color(foreground)
            .child(object_header(&self.ui, cx))
            .child(tab_strip(&self.ui, cx))
            .child(tab_body(&self.ui, width, window, cx))
    }
}
