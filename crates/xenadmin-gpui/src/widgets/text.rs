//! Headings, paragraphs, banners and the placeholder of a missing bespoke
//! element.

use xenadmin_ui::surface::{Custom, Notice, NoticeKind, Text, Tone};

use crate::kit::*;
use crate::render::tone_color;

/// A section title.
pub fn heading(id: String, value: &str, _cx: &App) -> AnyElement {
    let value = SharedString::from(value.to_owned());
    div()
        .id(SharedString::from(id))
        .test_support()
        .role(Role::Heading)
        .aria_label(value.clone())
        .w_full()
        .pt_1()
        .text_base()
        .font_semibold()
        .child(value)
        .into_any_element()
}

/// A paragraph in its tone. An alert text takes the alert role, so
/// assistive technology announces it.
pub fn text(id: String, text: &Text, cx: &App) -> AnyElement {
    let value = SharedString::from(text.text.clone());
    div()
        .id(SharedString::from(id))
        .test_support()
        .role(if text.alert { Role::Alert } else { Role::Label })
        .aria_label(value.clone())
        .w_full()
        .text_sm()
        .text_color(tone_color(text.tone, cx))
        .child(value)
        .into_any_element()
}

/// A tinted, bordered box: the frame of a notice, a review and an outcome.
pub fn panel(tone: Tone, cx: &App) -> Div {
    let theme = cx.theme();
    let edge = match tone {
        Tone::Normal | Tone::Muted => theme.border,
        tone => tone_color(tone, cx).opacity(0.55),
    };
    let fill = match tone {
        Tone::Normal | Tone::Muted => theme.muted.opacity(0.5),
        tone => tone_color(tone, cx).opacity(0.10),
    };
    v_flex()
        .w_full()
        .gap_2()
        .p_3()
        .rounded(theme.radius)
        .border_1()
        .border_color(edge)
        .bg(fill)
}

/// A banner.
pub fn notice(id: String, notice: &Notice, cx: &App) -> AnyElement {
    let (tone, icon) = match notice.kind {
        NoticeKind::Info => (Tone::Normal, IconName::Info),
        NoticeKind::Warning => (Tone::Warning, IconName::TriangleAlert),
        NoticeKind::Danger => (Tone::Danger, IconName::CircleX),
        NoticeKind::Preview => (Tone::Muted, IconName::Eye),
    };
    let message = SharedString::from(notice.text.clone());
    let name = SharedString::from(notice.title.clone().unwrap_or_else(|| notice.text.clone()));
    let accent = match tone {
        Tone::Normal | Tone::Muted => cx.theme().muted_foreground,
        tone => tone_color(tone, cx),
    };
    panel(tone, cx)
        .id(SharedString::from(id))
        .test_support()
        .role(if notice.kind == NoticeKind::Danger {
            Role::Alert
        } else {
            Role::Status
        })
        .aria_label(name)
        .child(
            h_flex()
                .items_start()
                .gap_2()
                .child(
                    div()
                        .pt_0p5()
                        .child(Icon::new(icon).small().text_color(accent)),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_1()
                        .when_some(notice.title.clone(), |column, title| {
                            column.child(
                                div()
                                    .text_sm()
                                    .font_semibold()
                                    .child(SharedString::from(title)),
                            )
                        })
                        .child(div().text_sm().child(message)),
                ),
        )
        .into_any_element()
}

/// What stands in for a bespoke element nothing is registered for. It names
/// the missing element, so the gap is visible on screen and in the tree.
pub fn missing_custom(id: String, custom: &Custom, cx: &App) -> AnyElement {
    let message = SharedString::from(format!(
        "Missing element: {} (nothing draws it yet)",
        custom.name
    ));
    let theme = cx.theme();
    div()
        .id(SharedString::from(id))
        .test_support()
        .role(Role::Note)
        .aria_label(message.clone())
        .w_full()
        .min_h(px(72.))
        .flex()
        .items_center()
        .justify_center()
        .p_3()
        .rounded(theme.radius)
        .border_1()
        .border_dashed()
        .border_color(theme.warning)
        .text_sm()
        .text_color(theme.muted_foreground)
        .child(message)
        .into_any_element()
}
