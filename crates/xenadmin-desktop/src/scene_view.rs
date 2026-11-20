//! The object header, the tab strip, the local action row and the tab body,
//! and a root view of them.
//!
//! [`SceneView`] is what the scene tool and the renderer tests show: the
//! content area of the window without the shell around it. The shell wraps
//! the same pieces: each is a function of the [`Ui`] that returns an
//! element, so the shell's own root view calls them where it wants them and
//! owns a [`DialogHost`] as this view does.

use xenadmin_ui::command::MenuItem;
use xenadmin_ui::surface::Block;
use xenadmin_ui::tabs::{TabBody, TabId};

use crate::kit::*;
use crate::render::{DialogHost, Pass, Ui, blocks, blocks_filling, pressable};
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
/// The element a tab body that scrolls is seen through. Of a body that ends
/// in a table it is the part above the table.
pub const TAB_SCROLL_KEY: &str = "tab-scroll";
/// The id of the column the two parts of a tab body that ends in a table
/// stand in. It scrolls when a short table is followed by sections.
pub const TAB_COLUMN_KEY: &str = "tab-column";
/// The key of the part of a tab body its last table is seen through, when
/// the body ends in one: the table, and the sections about its selection.
pub const TAB_RESULTS_KEY: &str = "tab-results-body";
/// The id of the head of such a body: the part above the table and the
/// table, which together are never higher than the body.
pub const TAB_HEAD_KEY: &str = "tab-head";
/// The id of what scrolls inside the part above a short table.
pub const TAB_ABOVE_KEY: &str = "tab-above";
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
/// The table a tab body ends in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Results {
    /// The index of the table among the blocks of the body.
    pub start: usize,
    /// The table has more rows than the least a grid shows: it gets at
    /// least half of the body.
    pub long: bool,
    /// Sections follow the table.
    pub followed: bool,
}

/// The rows a table needs before it is given half of a tab body: the least
/// a grid that fills shows.
const LONG_TABLE_ROWS: usize = 4;

/// The table a tab body ends in, if it does: its last table, when it is
/// the last block or only sections follow it (what the tab says about the
/// selected rows).
pub fn results_start(items: &[Block]) -> Option<Results> {
    let start = items
        .iter()
        .rposition(|block| matches!(block, Block::Table(_)))?;
    let Block::Table(table) = &items[start] else {
        return None;
    };
    let after = &items[start + 1..];
    after
        .iter()
        .all(|block| matches!(block, Block::Group(_)))
        .then_some(Results {
            start,
            long: table.rows.len() > LONG_TABLE_ROWS,
            followed: !after.is_empty(),
        })
}

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
        Some(TabBody::Blocks(items)) => match results_start(&items) {
            // A body that ends in a table is two parts, so that the table
            // is in view however long what stands above it is (the query
            // of the Search tab and its filters). Each part's element with
            // the id is what it is seen through.
            //
            // A table of more rows than the least a grid shows gets at
            // least half of the body: the part above takes its own height
            // but never more than the other half, and scrolls by itself
            // beyond that. The sections after the table (what the tab says
            // about the selected rows) are under it in its part.
            //
            // A shorter table is as high as its rows and needs no half:
            // the part above gives way only as far as the table needs.
            // The two stand in a head that is never higher than the body.
            // The sections that follow a short table stand after the
            // head, and the body scrolls as one column, so that they have
            // the height of the body to be read in.
            //
            // A row that is selected adds a section after the table. The
            // table (and a menu that the same press opened on it) must
            // stay the element it was, and it must stay where it was: the
            // head is laid out without regard to what follows it, so the
            // part above gives the same way with and without a section.
            // (Before 2026-11-20 the part above took its whole height
            // back when a section followed, and the table moved by what
            // it had given: more than a row where the row of local
            // actions is two lines high.)
            Some(results) => {
                let Results {
                    start,
                    long,
                    followed: _,
                } = results;
                // Only a long table takes the height that is left; a short
                // one is as high as its rows.
                let fills = long.then_some(start);
                let mut above = blocks_filling(&pass, TAB_BASE, &items, fills, window, cx);
                let mut results = above.split_off(start);
                // Under a long table the sections are in its part; after a
                // short one they follow the head.
                let after = match long {
                    true => Vec::new(),
                    false => results.split_off(1),
                };
                let above = v_flex()
                    .id(TAB_SCROLL_KEY)
                    .test_support()
                    .w_full()
                    .flex_shrink(1.)
                    .min_h_0()
                    .when(long, |part| part.max_h(relative(0.5)))
                    .overflow_hidden()
                    .child(match long {
                        true => v_flex()
                            .w_full()
                            .flex_shrink(1.)
                            .min_h_0()
                            .overflow_y_scrollbar()
                            .px_4()
                            .pt_4()
                            .pb_2()
                            .gap_3()
                            .children(above)
                            .into_any_element(),
                        // Above a short table the part scrolls without the
                        // Kit's scroll area, which keeps the wheel to
                        // itself while it has anything to scroll: the
                        // wheel over this part must also move the body,
                        // or the sections after the table are not reached
                        // from here. It has no scrollbar then.
                        false => v_flex()
                            .id(TAB_ABOVE_KEY)
                            .w_full()
                            .flex_shrink(1.)
                            .min_h_0()
                            .overflow_y_scroll()
                            .px_4()
                            .pt_4()
                            .pb_2()
                            .gap_3()
                            .children(above)
                            .into_any_element(),
                    });
                let results = div()
                    .id(TAB_RESULTS_KEY)
                    .test_support()
                    .w_full()
                    .map(|part| match long {
                        true => part.flex_1().min_h_0(),
                        false => part.flex_none(),
                    })
                    .overflow_hidden()
                    .child(
                        v_flex()
                            .w_full()
                            .when(long, |column| column.h_full())
                            .overflow_y_scrollbar()
                            .px_4()
                            // The same with and without sections after
                            // it: the head must not change its height.
                            .pb_4()
                            .when(start == 0, |column| column.pt_4())
                            .when(start > 0, |column| column.pt_1())
                            .gap_3()
                            .children(results),
                    );
                let head = v_flex()
                    .id(TAB_HEAD_KEY)
                    .w_full()
                    .map(|head| match long {
                        true => head.flex_1().min_h_0(),
                        false => head.flex_none().max_h(relative(1.)),
                    })
                    .when(start > 0, |head| head.child(above))
                    .child(results);
                v_flex()
                    .id(TAB_COLUMN_KEY)
                    .size_full()
                    .overflow_y_scroll()
                    .test_support()
                    .child(head)
                    .when(!after.is_empty(), |column| {
                        column.child(
                            v_flex()
                                .w_full()
                                .flex_none()
                                .px_4()
                                .pb_4()
                                .gap_3()
                                .children(after),
                        )
                    })
                    .into_any_element()
            }
            // Anything else is one column that scrolls. The element with
            // the id is what it is seen through.
            None => div()
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
                        .children(blocks(&pass, TAB_BASE, &items, window, cx)),
                )
                .into_any_element(),
        },
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
