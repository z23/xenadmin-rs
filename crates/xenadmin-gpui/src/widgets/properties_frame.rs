//! The frame of a properties dialog: a vertical page list beside the page.

use xenadmin_ui::surface::OptionItem;

use crate::kit::*;
use crate::render::{Ui, pressable, tip};

/// The width of the page list.
pub const PAGE_LIST_WIDTH: f32 = 208.0;

/// Draws the page list on the left and `page` on the right, each with its
/// own scroll, in a frame of height `height`. `on_page` receives the id of
/// the page the user picked.
#[allow(clippy::too_many_arguments)]
pub fn properties_frame(
    ui: &Ui,
    scope: &str,
    list_key: &str,
    pages: &[OptionItem],
    current: &str,
    on_page: impl Fn(&str, &mut Window, &mut App) + Clone + 'static,
    page: AnyElement,
    height: Pixels,
    cx: &mut App,
) -> AnyElement {
    let theme = cx.theme();
    let border = theme.border;
    let selected_fill = theme.list_active;
    let hover = theme.list_hover;
    let muted = theme.muted_foreground;
    let foreground = theme.foreground;
    let radius = theme.radius;
    let primary = theme.primary;
    let ring = theme.ring;
    let current_label = pages
        .iter()
        .find(|item| item.id == current)
        .map(|item| item.label.clone())
        .unwrap_or_default();
    let list = v_flex()
        .id(SharedString::from(list_key.to_owned()))
        .test_support()
        .role(Role::TabList)
        .aria_label("Pages")
        .aria_value(SharedString::from(current_label))
        .w(px(PAGE_LIST_WIDTH))
        .flex_none()
        .h_full()
        .gap_0p5()
        .pr_3()
        .border_r_1()
        .border_color(border)
        .overflow_y_scroll()
        .children(pages.iter().map(|item| {
            let key = item
                .key
                .clone()
                .unwrap_or_else(|| format!("{list_key}-{}", item.id));
            let selected = item.id == current;
            let disabled = item.disabled.clone();
            let label = SharedString::from(item.label.clone());
            let id = item.id.clone();
            let on_page = on_page.clone();
            let row = match &disabled {
                Some(_) => div().id(SharedString::from(key)).test_support(),
                None => {
                    let focus = ui.focus_handle(scope, &key, cx);
                    pressable(SharedString::from(key), &focus, move |window, cx| {
                        on_page(&id, window, cx);
                    })
                    .hover(move |style| style.bg(hover))
                    .focus_visible(move |style| style.border_color(ring))
                }
            };
            row.role(Role::Tab)
                .aria_label(label.clone())
                .aria_selected(selected)
                .flex()
                .flex_col()
                .w_full()
                .px_2()
                .py_1p5()
                .rounded(radius)
                .text_sm()
                .border_l_2()
                .border_color(if selected {
                    primary
                } else {
                    gpui_kit::transparent_black()
                })
                .when(selected, |row| row.bg(selected_fill).font_medium())
                .text_color(if disabled.is_some() {
                    muted
                } else {
                    foreground
                })
                .when_some(disabled, |row, reason| {
                    row.aria_description(SharedString::from(reason.clone()))
                        .tooltip(tip(reason))
                })
                .child(label)
                .when_some(item.detail.clone(), |row, detail| {
                    row.child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(SharedString::from(detail)),
                    )
                })
        }));
    h_flex()
        .w_full()
        .h(height)
        .items_start()
        .gap_4()
        .child(list)
        .child(
            div()
                .id(SharedString::from(format!("{list_key}-page")))
                .flex_1()
                .min_w_0()
                .h_full()
                .overflow_y_scrollbar()
                .child(page),
        )
        .into_any_element()
}
