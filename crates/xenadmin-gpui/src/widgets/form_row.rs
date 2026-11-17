//! A form row: label, control, help text, validation message.

use crate::kit::*;

/// The parts of a form row.
pub struct FormRow {
    /// The path of the field in the flattened description.
    pub path: String,
    /// The key of the control. The error line is `<key>-error`.
    pub key: String,
    /// Shown above the control. A checkbox carries its own label.
    pub label: Option<String>,
    pub control: AnyElement,
    pub help: Option<String>,
    pub error: Option<String>,
}

/// Lays a form row out: the label above the control, then the help, then
/// the error.
pub fn form_row(row: FormRow, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let muted = theme.muted_foreground;
    v_flex()
        .w_full()
        .gap_1()
        .when_some(
            row.label.filter(|label| !label.is_empty()),
            |column, label| {
                column.child(
                    div()
                        .text_sm()
                        .font_medium()
                        .child(SharedString::from(label)),
                )
            },
        )
        .child(row.control)
        .when_some(row.help, |column, help| {
            let help = SharedString::from(help);
            column.child(
                div()
                    .id(SharedString::from(format!("{}/help", row.path)))
                    .test_support()
                    .role(Role::Label)
                    .aria_label(help.clone())
                    .text_xs()
                    .text_color(muted)
                    .child(help),
            )
        })
        .when_some(row.error, |column, error| {
            column.child(field_error(format!("{}-error", row.key), &error, cx))
        })
        .into_any_element()
}

/// The validation message of a field. The Kit's inputs have no error
/// display; this line is it. It takes the alert role.
pub fn field_error(key: String, message: &str, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let message = SharedString::from(message.to_owned());
    h_flex()
        .id(SharedString::from(key))
        .test_support()
        .role(Role::Alert)
        .aria_label(message.clone())
        .items_start()
        .gap_1()
        .text_xs()
        .text_color(theme.danger)
        .child(
            div()
                .pt_0p5()
                .child(Icon::new(IconName::CircleAlert).xsmall()),
        )
        .child(div().flex_1().min_w_0().child(message))
        .into_any_element()
}
