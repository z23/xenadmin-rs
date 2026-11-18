//! The notice on every surface of an experimental area.

use xenadmin_ui::surface::{
    EXPERIMENTAL_NOTICE, EXPERIMENTAL_NOTICE_KEY, EXPERIMENTAL_NOTICE_LABEL, Tone,
};

use crate::kit::*;
use crate::render::tone_color;

/// "Experimental: it has worked on a test lab; how it fails is not yet
/// proven." The text, the key and the label are the model's.
pub fn experimental_notice(cx: &App) -> AnyElement {
    let theme = cx.theme();
    let warning = tone_color(Tone::Warning, cx);
    h_flex()
        .id(EXPERIMENTAL_NOTICE_KEY)
        .test_support()
        .role(Role::Status)
        .aria_label(EXPERIMENTAL_NOTICE_LABEL)
        .aria_description(EXPERIMENTAL_NOTICE)
        .w_full()
        .items_start()
        .gap_2()
        .px_3()
        .py_2()
        .rounded(theme.radius)
        .border_1()
        .border_color(warning.opacity(0.55))
        .bg(warning.opacity(0.10))
        .child(
            div().pt_0p5().child(
                Icon::new(IconName::TriangleAlert)
                    .small()
                    .text_color(warning),
            ),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_sm()
                .child(EXPERIMENTAL_NOTICE),
        )
        .into_any_element()
}
