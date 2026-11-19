//! The review step.

use xenadmin_ui::surface::{Review, Tone};

use super::{panel, property_list};
use crate::kit::*;
use crate::render::{Ui, join, tone_color};

/// What will happen, to what, and what stands in the way: the review lines
/// word for word, the affected resources, the warnings, and the stale
/// notice when the reviewed observation is no longer current.
pub fn review_panel(ui: &Ui, path: &str, review: &Review, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let warning = tone_color(Tone::Warning, cx);
    let danger = tone_color(Tone::Danger, cx);
    let muted = theme.muted_foreground;
    panel(Tone::Normal, cx)
        .id(SharedString::from(review.key.clone()))
        .test_support()
        .role(Role::Group)
        .aria_label("Review")
        .gap_3()
        .when(!review.lines.is_empty(), |panel| {
            panel.child(
                v_flex()
                    .gap_1()
                    .children(review.lines.iter().enumerate().map(|(index, line)| {
                        let line = SharedString::from(line.clone());
                        div()
                            .id(SharedString::from(format!("{path}/lines/#{index}")))
                            .test_support()
                            .role(Role::Label)
                            .aria_label(line.clone())
                            .text_sm()
                            .child(line)
                    })),
            )
        })
        .when(!review.affected.is_empty(), |panel| {
            panel
                .child(
                    div()
                        .text_xs()
                        .font_semibold()
                        .text_color(muted)
                        .child("AFFECTED"),
                )
                .child(property_list(
                    ui,
                    &join(path, "affected"),
                    &review.affected,
                    cx,
                ))
        })
        .when(!review.warnings.is_empty(), |panel| {
            panel.child(
                v_flex()
                    .gap_1()
                    .children(review.warnings.iter().enumerate().map(|(index, line)| {
                        let line = SharedString::from(line.clone());
                        h_flex()
                            .id(SharedString::from(format!("{path}/warnings/#{index}")))
                            .test_support()
                            .role(Role::Label)
                            .aria_label(line.clone())
                            .items_start()
                            .gap_2()
                            .text_sm()
                            .text_color(warning)
                            .child(
                                div()
                                    .pt_0p5()
                                    .child(Icon::new(IconName::TriangleAlert).small()),
                            )
                            .child(div().flex_1().min_w_0().child(line))
                    })),
            )
        })
        .when_some(review.stale.clone(), |panel, stale| {
            let stale = SharedString::from(stale);
            panel.child(
                h_flex()
                    .id(SharedString::from(format!("{}-stale", review.key)))
                    .test_support()
                    .role(Role::Alert)
                    .aria_label(stale.clone())
                    .items_start()
                    .gap_2()
                    .text_sm()
                    .text_color(danger)
                    .child(div().pt_0p5().child(Icon::new(IconName::CircleX).small()))
                    .child(div().flex_1().min_w_0().child(stale)),
            )
        })
        .into_any_element()
}
