//! Progress, a result or an unknown outcome, shown where the action was
//! started.

use xenadmin_ui::surface::{Outcome, OutcomeState, Tone};

use super::panel;
use crate::kit::*;
use crate::render::{Ui, button_row, tone_color};

/// The banner of an outcome: its state, title, lines and actions (Recheck,
/// Acknowledge, Cancel). A running outcome shows its progress when the
/// fraction is known and a spinner when it is not.
pub fn outcome_banner(ui: &Ui, path: &str, outcome: &Outcome, cx: &App) -> AnyElement {
    let tone = outcome.state.tone();
    let accent = match tone {
        Tone::Normal | Tone::Muted => cx.theme().muted_foreground,
        tone => tone_color(tone, cx),
    };
    let state = match outcome.state {
        OutcomeState::Running(Some(fraction)) => {
            format!("running {:.0}%", fraction.clamp(0.0, 1.0) * 100.0)
        }
        state => state.name().to_owned(),
    };
    let mark = match outcome.state {
        OutcomeState::Running(_) => Spinner::new().small().into_any_element(),
        OutcomeState::Succeeded => icon(IconName::CircleCheck, accent),
        OutcomeState::Failed => icon(IconName::CircleX, accent),
        OutcomeState::Partial => icon(IconName::TriangleAlert, accent),
        OutcomeState::Unknown => icon(IconName::CircleAlert, accent),
        OutcomeState::Cancelled => icon(IconName::Ban, accent),
    };
    let title = SharedString::from(outcome.title.clone());
    panel(tone, cx)
        .id(SharedString::from(outcome.key.clone()))
        .test_support()
        .role(Role::Status)
        .aria_label(title.clone())
        .aria_value(SharedString::from(state))
        .child(
            h_flex()
                .items_start()
                .gap_2()
                .child(div().pt_0p5().child(mark))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_sm()
                        .font_semibold()
                        .child(title),
                ),
        )
        .when_some(
            match outcome.state {
                OutcomeState::Running(Some(fraction)) => Some(fraction.clamp(0.0, 1.0)),
                _ => None,
            },
            |panel, fraction| {
                panel.child(
                    Progress::new(SharedString::from(format!("{}-progress", outcome.key)))
                        .value(fraction * 100.0),
                )
            },
        )
        .when(!outcome.lines.is_empty(), |panel| {
            panel.child(
                v_flex()
                    .gap_1()
                    .children(outcome.lines.iter().enumerate().map(|(index, line)| {
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
        .when(!outcome.actions.is_empty(), |panel| {
            panel.child(button_row(ui, &outcome.actions, cx))
        })
        .into_any_element()
}

fn icon(name: IconName, color: Hsla) -> AnyElement {
    Icon::new(name).small().text_color(color).into_any_element()
}
