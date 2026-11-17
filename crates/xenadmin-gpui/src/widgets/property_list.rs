//! A label and value list.

use xenadmin_ui::surface::Property;

use crate::kit::*;
use crate::render::{Ui, element_id, tone_color};

/// The width of the label column.
const LABEL_WIDTH: f32 = 168.0;

/// Draws properties as rows of a muted label and a value. A copyable value
/// has a copy button, which asks the model to copy the text: the clipboard
/// is written by the application's intent handler, on this explicit press.
pub fn property_list(ui: &Ui, base: &str, items: &[Property], cx: &App) -> AnyElement {
    let muted = cx.theme().muted_foreground;
    v_flex()
        .w_full()
        .gap_1p5()
        .children(items.iter().enumerate().map(|(index, item)| {
            let id = element_id(base, item.key.as_ref(), index);
            let label = SharedString::from(item.label.clone());
            let value = SharedString::from(item.value.clone());
            h_flex()
                .id(SharedString::from(id.clone()))
                .test_support()
                .role(Role::ListItem)
                .aria_label(label.clone())
                .aria_value(value.clone())
                .w_full()
                .items_start()
                .gap_3()
                .text_sm()
                .child(
                    div()
                        .w(px(LABEL_WIDTH))
                        .flex_none()
                        .text_color(muted)
                        .child(label.clone()),
                )
                .child(
                    h_flex()
                        .flex_1()
                        .min_w_0()
                        .items_start()
                        .gap_2()
                        .child(
                            div()
                                .min_w_0()
                                .text_color(tone_color(item.tone, cx))
                                .child(value.clone()),
                        )
                        .when(item.copy, |row| {
                            let store = ui.store.clone();
                            let text = item.value.clone();
                            row.child(
                                Button::new(SharedString::from(format!("{id}-copy")))
                                    .icon(IconName::Copy)
                                    .ghost()
                                    .xsmall()
                                    .accessibility_label(SharedString::from(format!(
                                        "Copy {}",
                                        item.label
                                    )))
                                    .tooltip(SharedString::from(format!("Copy {}", item.label)))
                                    .on_click(move |_, _, cx| {
                                        let text = text.clone();
                                        crate::store::dispatch(&store, cx, move |m| {
                                            m.copy_text(text)
                                        });
                                    }),
                            )
                        }),
                )
        }))
        .into_any_element()
}
