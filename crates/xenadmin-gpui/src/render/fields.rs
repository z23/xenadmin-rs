//! Fields and choices: the controls of a form.

use xenadmin_ui::surface::{self as s, ChoiceStyle, Control};

use super::blocks::{Pass, with_reason};
use super::cache::{TextSpec, TextWidget};
use super::join;
use crate::kit::*;
use crate::store::dispatch;
use crate::widgets::{self, FormRow};

/// A label, a control, help and an error.
pub(crate) fn field(
    pass: &Pass<'_>,
    base: &str,
    field: &s::Field,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let ui = pass.ui;
    let path = join(base, &field.key);
    let disabled = field.disabled.is_some();
    let key = SharedString::from(field.key.clone());
    let label = SharedString::from(field.label.clone());
    let wrap = format!("{}-reason", field.key);
    let (control, labelled) = match &field.control {
        Control::Text {
            value,
            secret,
            multiline,
            placeholder,
            on_edit,
            on_submit,
        } => {
            let spec = TextSpec {
                key: &field.key,
                value: match secret {
                    Some(secret) => secret.expose(),
                    None => value,
                },
                secret: secret.is_some(),
                multiline: *multiline,
                placeholder,
                on_edit,
                on_submit: on_submit.as_ref(),
            };
            let widget = ui
                .cache
                .borrow_mut()
                .text(pass.scope, spec, &ui.store, window, cx);
            let control = match widget {
                TextWidget::Line(state) => Input::new(&state)
                    .id(key.clone())
                    .aria_label(label.clone())
                    .disabled(disabled)
                    .into_any_element(),
                // A text area has no id of its own: the keyed wrapper is
                // what tests and the tree find. The Kit binds Tab in a text
                // area to indentation, which would hold the keyboard in a
                // form: here Tab and Shift+Tab move on, as in every field.
                TextWidget::Area(state) => div()
                    .id(key.clone())
                    .test_support()
                    .w_full()
                    .capture_action(|_: &IndentInline, window, cx| {
                        window.focus_next(cx);
                        cx.stop_propagation();
                    })
                    .capture_action(|_: &OutdentInline, window, cx| {
                        window.focus_prev(cx);
                        cx.stop_propagation();
                    })
                    .child(
                        Textarea::new(&state)
                            .aria_label(label.clone())
                            .disabled(disabled),
                    )
                    .into_any_element(),
            };
            (
                with_reason(wrap, &field.label, field.disabled.as_ref(), control),
                true,
            )
        }
        Control::Checkbox { checked, on_toggle } => {
            let store = ui.store.clone();
            let on_toggle = on_toggle.clone();
            let control = Checkbox::new(key.clone())
                .label(label.clone())
                .checked(*checked)
                .disabled(disabled)
                .on_change(move |_: &bool, _, cx| {
                    let on_toggle = on_toggle.clone();
                    dispatch(&store, cx, move |m| on_toggle(m));
                });
            let control = with_reason(wrap, &field.label, field.disabled.as_ref(), control);
            (control, false)
        }
        Control::Select {
            options,
            selected,
            on_pick,
        } => {
            let control = select(
                pass,
                &field.key,
                &field.label,
                options,
                selected.as_ref(),
                on_pick,
                disabled,
                window,
                cx,
            );
            (
                with_reason(wrap, &field.label, field.disabled.as_ref(), control),
                true,
            )
        }
    };
    widgets::form_row(
        FormRow {
            path,
            key: field.key.clone(),
            // A checkbox carries its own label.
            label: labelled.then(|| field.label.clone()),
            control,
            help: field.help.clone(),
            error: field.error.clone(),
        },
        cx,
    )
}

/// The Kit select of a select control or a drop-down choice.
#[allow(clippy::too_many_arguments)]
fn select(
    pass: &Pass<'_>,
    key: &str,
    label: &str,
    options: &[s::OptionItem],
    selected: Option<&String>,
    on_pick: &s::ActText,
    disabled: bool,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let ui = pass.ui;
    let widget = ui.cache.borrow_mut().select(
        pass.scope, key, options, selected, on_pick, &ui.store, window, cx,
    );
    Select::new(&widget)
        .id(SharedString::from(key.to_owned()))
        .accessibility_label(SharedString::from(label.to_owned()))
        .placeholder("Choose")
        .disabled(disabled)
        .into_any_element()
}

/// Pick one option: a radio list, a segmented row or a drop-down.
pub(crate) fn choice(
    pass: &Pass<'_>,
    base: &str,
    choice: &s::Choice,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let ui = pass.ui;
    let path = join(base, &choice.key);
    let key = SharedString::from(choice.key.clone());
    let group_label = SharedString::from(choice.label.clone().unwrap_or_default());
    let muted = cx.theme().muted_foreground;
    let option_key = |option: &s::OptionItem| {
        option
            .key
            .clone()
            .unwrap_or_else(|| format!("{}-{}", choice.key, option.id))
    };
    let reason_of = |option: &s::OptionItem| choice.disabled.clone().or(option.disabled.clone());
    let pick = |option: &s::OptionItem| {
        let store = ui.store.clone();
        let on_pick = choice.on_pick.clone();
        let id = option.id.clone();
        move |cx: &mut App| {
            let (on_pick, id) = (on_pick.clone(), id.clone());
            dispatch(&store, cx, move |m| on_pick(m, &id));
        }
    };
    let control = match choice.style {
        ChoiceStyle::Radio => v_flex()
            .id(key)
            .test_support()
            .role(Role::RadioGroup)
            .aria_label(group_label)
            .gap_2()
            .children(choice.options.iter().map(|option| {
                let checked = choice.selected.as_ref() == Some(&option.id);
                let reason = reason_of(option);
                let pick = pick(option);
                let radio = Radio::new(SharedString::from(option_key(option)))
                    .label(SharedString::from(option.label.clone()))
                    .checked(checked)
                    .disabled(reason.is_some())
                    .when_some(reason, |radio, reason| {
                        radio.tooltip(SharedString::from(reason))
                    })
                    .on_click(move |_: &bool, _, cx| pick(cx));
                v_flex()
                    .gap_0p5()
                    .child(radio)
                    .when_some(option.detail.clone(), |row, detail| {
                        row.child(
                            div()
                                .pl_6()
                                .text_xs()
                                .text_color(muted)
                                .child(SharedString::from(detail)),
                        )
                    })
            }))
            .into_any_element(),
        ChoiceStyle::Segmented => h_flex()
            .id(key)
            .test_support()
            .role(Role::RadioGroup)
            .aria_label(group_label)
            .flex_wrap()
            .gap_1()
            .children(choice.options.iter().map(|option| {
                let checked = choice.selected.as_ref() == Some(&option.id);
                let reason = reason_of(option);
                let pick = pick(option);
                let segment = Button::new(SharedString::from(option_key(option)))
                    .label(SharedString::from(option.label.clone()))
                    .outline()
                    .selected(checked)
                    .toggled(checked);
                match reason {
                    Some(reason) => segment.disabled(true).tooltip(SharedString::from(reason)),
                    None => segment.on_click(move |_, _, cx| pick(cx)),
                }
            }))
            .into_any_element(),
        ChoiceStyle::Select => {
            let control = select(
                pass,
                &choice.key,
                choice.label.as_deref().unwrap_or_default(),
                &choice.options,
                choice.selected.as_ref(),
                &choice.on_pick,
                choice.disabled.is_some(),
                window,
                cx,
            );
            with_reason(
                format!("{}-reason", choice.key),
                choice.label.as_deref().unwrap_or_default(),
                choice.disabled.as_ref(),
                control,
            )
        }
    };
    widgets::form_row(
        FormRow {
            path,
            key: choice.key.clone(),
            label: choice.label.clone(),
            control,
            help: None,
            error: None,
        },
        cx,
    )
}
