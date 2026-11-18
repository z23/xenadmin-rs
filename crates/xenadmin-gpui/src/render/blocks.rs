//! Draws a list of blocks.

use xenadmin_ui::surface::{self as s, Block};

use super::{Ui, element_id, fields, join, pressable, segment, tip};
use crate::kit::*;
use crate::store::dispatch;
use crate::widgets;
use crate::widgets::object_grid::{GridHeight, ObjectGrid, OnCancel};

/// Below this width per column, the columns of a `Columns` block stack.
const COLUMN_MIN_WIDTH: f32 = 300.0;

/// What a pass over a list of blocks carries.
#[derive(Clone)]
pub struct Pass<'a> {
    pub ui: &'a Ui,
    /// The cache scope: one surface, or the tab body.
    pub scope: &'a str,
    /// The width the blocks have, to decide when columns stack.
    pub width: Pixels,
    /// The last table at the top level takes the height that is left. Set
    /// for a tab body, whose container has a definite height.
    pub fill: bool,
    /// What Escape does while a table has the keyboard: the close handler
    /// of the surface the table is in. A table in a tab has none.
    pub on_cancel: Option<OnCancel>,
}

impl Pass<'_> {
    fn nested(&self, width: Pixels) -> Self {
        Self {
            width,
            fill: false,
            ..self.clone()
        }
    }
}

/// The index of the top-level table that takes the remaining height: the
/// last block, when it is a table.
pub fn filling_table(pass: &Pass<'_>, blocks: &[Block]) -> Option<usize> {
    let last = blocks.len().checked_sub(1)?;
    (pass.fill && matches!(blocks[last], Block::Table(_))).then_some(last)
}

/// Draws `blocks`. `base` is the path prefix of the flattened description:
/// the surface id, or `tab`.
pub fn blocks(
    pass: &Pass<'_>,
    base: &str,
    blocks: &[Block],
    window: &mut Window,
    cx: &mut App,
) -> Vec<AnyElement> {
    let filling = filling_table(pass, blocks);
    blocks
        .iter()
        .enumerate()
        .map(|(index, item)| block(pass, base, index, item, filling == Some(index), window, cx))
        .collect()
}

fn block(
    pass: &Pass<'_>,
    base: &str,
    index: usize,
    block: &Block,
    fill: bool,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let anonymous = join(base, &format!("#{index}"));
    match block {
        Block::Heading(value) => widgets::heading(anonymous, value, cx),
        Block::Text(text) if text.preformatted => {
            widgets::preformatted(element_id(base, text.key.as_ref(), index), text, cx)
        }
        Block::Text(text) => widgets::text(element_id(base, text.key.as_ref(), index), text, cx),
        Block::Notice(notice) => {
            widgets::notice(element_id(base, notice.key.as_ref(), index), notice, cx)
        }
        Block::Field(field) => fields::field(pass, base, field, window, cx),
        Block::Choice(choice) => fields::choice(pass, base, choice, window, cx),
        Block::Properties(items) => widgets::property_list(pass.ui, &anonymous, items, cx),
        Block::Table(table) => {
            let height = if fill {
                GridHeight::Fill
            } else {
                GridHeight::Rows
            };
            ObjectGrid::render(pass, base, table, height, window, cx)
        }
        Block::Review(review) => {
            widgets::review_panel(pass.ui, &join(base, &review.key), review, cx)
        }
        Block::Outcome(outcome) => {
            widgets::outcome_banner(pass.ui, &join(base, &outcome.key), outcome, cx)
        }
        Block::Actions(buttons) => button_row(pass.ui, buttons, cx).into_any_element(),
        Block::Group(group) => {
            let path = join(base, &segment(group.key.as_ref(), index));
            let id = element_id(base, group.key.as_ref(), index);
            self::group(pass, &id, &path, group, window, cx)
        }
        Block::Columns(columns) => self::columns(pass, &anonymous, columns, window, cx),
        Block::Custom(custom) => match pass.ui.custom(&custom.name) {
            Some(render) => render(custom, pass.ui, window, cx),
            None => {
                widgets::missing_custom(element_id(base, custom.key.as_ref(), index), custom, cx)
            }
        },
        Block::Image(image) => widgets::picture(pass, image, window, cx),
        Block::Divider => div()
            .id(SharedString::from(anonymous))
            .test_support()
            .role(Role::Splitter)
            .w_full()
            .child(Separator::horizontal())
            .into_any_element(),
    }
}

/// A titled section. A collapsible one folds on a press of its title; the
/// folded state is the front end's.
fn group(
    pass: &Pass<'_>,
    id: &str,
    path: &str,
    group: &s::Group,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let folded = group.collapsible
        && pass
            .ui
            .cache
            .borrow_mut()
            .folded(pass.scope, path, group.collapsed);
    let muted = cx.theme().muted_foreground;
    let border = cx.theme().border;
    let title = h_flex()
        .gap_1()
        .when(group.collapsible, |title| {
            title.child(
                Icon::new(if folded {
                    IconName::ChevronRight
                } else {
                    IconName::ChevronDown
                })
                .small()
                .text_color(muted),
            )
        })
        .child(
            div()
                .text_sm()
                .font_semibold()
                .child(SharedString::from(group.title.clone())),
        );
    let label = SharedString::from(group.title.clone());
    let header = if group.collapsible {
        let focus = pass.ui.focus_handle(pass.scope, path, cx);
        let ui = pass.ui.clone();
        let scope = pass.scope.to_owned();
        let toggled = path.to_owned();
        let ring = cx.theme().ring;
        pressable(
            SharedString::from(id.to_owned()),
            &focus,
            move |window, _| {
                ui.cache.borrow_mut().toggle_folded(&scope, &toggled);
                window.refresh();
            },
        )
        .role(Role::Group)
        .aria_label(label)
        .aria_expanded(!folded)
        .rounded(px(4.))
        .border_1()
        .border_color(gpui_kit::transparent_black())
        .focus_visible(move |style| style.border_color(ring))
        .child(title)
        .into_any_element()
    } else {
        div()
            .id(SharedString::from(id.to_owned()))
            .test_support()
            .role(Role::Group)
            .aria_label(label)
            .child(title)
            .into_any_element()
    };
    v_flex()
        .w_full()
        .gap_2()
        .child(header)
        .when(!folded, |section| {
            section.child(
                v_flex()
                    .w_full()
                    .gap_3()
                    .pl_3()
                    .border_l_2()
                    .border_color(border)
                    .children(blocks(
                        &pass.nested(pass.width - px(14.)),
                        path,
                        &group.blocks,
                        window,
                        cx,
                    )),
            )
        })
        .into_any_element()
}

/// Side by side when there is room, stacked when there is not.
fn columns(
    pass: &Pass<'_>,
    path: &str,
    columns: &[Vec<Block>],
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let count = columns.len().max(1) as f32;
    let gap = px(16.);
    let each = (pass.width - gap * (count - 1.)) / count;
    let stacked = each < px(COLUMN_MIN_WIDTH);
    let inner = pass.nested(if stacked { pass.width } else { each });
    let children: Vec<AnyElement> = columns
        .iter()
        .enumerate()
        .map(|(index, column)| {
            v_flex()
                .gap_3()
                .min_w_0()
                .when(!stacked, |column| column.flex_1())
                .when(stacked, |column| column.w_full())
                .children(blocks(
                    &inner,
                    &join(path, &format!("#{index}")),
                    column,
                    window,
                    cx,
                ))
                .into_any_element()
        })
        .collect();
    div()
        .id(SharedString::from(path.to_owned()))
        .test_support()
        .w_full()
        .flex()
        .gap_4()
        .when(stacked, |row| row.flex_col())
        .when(!stacked, |row| row.flex_row().items_start())
        .children(children)
        .into_any_element()
}

/// One button of a description: its style, its handler, and when it is
/// disabled the reason as its tooltip.
pub fn button(ui: &Ui, button: &s::Button) -> Button {
    let base = Button::new(SharedString::from(button.key.clone()))
        .label(SharedString::from(button.label.clone()));
    let styled = match button.style {
        s::ButtonStyle::Primary => base.primary(),
        s::ButtonStyle::Secondary => base.outline(),
        s::ButtonStyle::Danger => base.danger(),
        s::ButtonStyle::Warning => base.warning(),
        s::ButtonStyle::Ghost => base.ghost(),
    };
    match &button.disabled {
        Some(reason) => styled
            .disabled(true)
            .tooltip(SharedString::from(reason.clone())),
        None => {
            // In a dialog the Kit sends Enter to the dialog as its confirm
            // action. The button that has the keyboard takes it first: Enter
            // presses the focused button, in a dialog as outside one.
            let store = ui.store.clone();
            let on_press = button.on_press.clone();
            styled
                .on_click(ui.on_click(&button.on_press))
                .on_action(move |_: &Confirm, _, cx| {
                    let on_press = on_press.clone();
                    dispatch(&store, cx, move |m| on_press(m));
                })
        }
    }
}

/// A row of buttons that wraps.
pub fn button_row(ui: &Ui, buttons: &[s::Button], _cx: &App) -> Div {
    h_flex()
        .gap_2()
        .flex_wrap()
        .children(buttons.iter().map(|item| button(ui, item)))
}

/// A wrapper that shows `reason` as a tooltip over a disabled control that
/// has no tooltip of its own.
pub(crate) fn with_reason(
    id: String,
    label: &str,
    reason: Option<&String>,
    control: impl IntoElement,
) -> AnyElement {
    match reason {
        // The reason is the tooltip, and the description of a group named
        // like the control, so assistive technology reads it too.
        Some(reason) => div()
            .id(SharedString::from(id))
            .test_support()
            .role(Role::Group)
            .aria_label(SharedString::from(label.to_owned()))
            .aria_description(SharedString::from(reason.clone()))
            .w_full()
            .tooltip(tip(reason.clone()))
            .child(control)
            .into_any_element(),
        None => control.into_any_element(),
    }
}
