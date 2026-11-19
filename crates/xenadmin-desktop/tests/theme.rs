//! The theme's contrast, in both modes: the Rune client's test
//! (`controls_are_distinguishable_from_the_surfaces_they_sit_on` of its
//! `theme.rs`, removed with it) on the colours this front end draws with.
//!
//! The ratios are WCAG contrast ratios of the colours as they reach the
//! screen: a colour with an alpha is first laid over what it is drawn on.

use gpui_kit::{Hsla, TestAppContext};
use xenadmin_desktop::kit::{ActiveTheme as _, ThemeMode};
use xenadmin_desktop::theme;
use xenadmin_ui::surface::Tone;

/// An opaque colour in sRGB.
type Flat = [f32; 3];

/// `color` laid over the opaque `under`.
fn over(color: Hsla, under: Flat) -> Flat {
    let top = color.to_rgb();
    let mix = |t: f32, u: f32| t * top.a + u * (1.0 - top.a);
    [
        mix(top.r, under[0]),
        mix(top.g, under[1]),
        mix(top.b, under[2]),
    ]
}

fn luminance(color: Flat) -> f32 {
    let linear = |c: f32| {
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(color[0]) + 0.7152 * linear(color[1]) + 0.0722 * linear(color[2])
}

fn contrast(a: Flat, b: Flat) -> f32 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

/// The pairs of one theme mode and the ones that fall short.
struct Pairs {
    mode: &'static str,
    short: Vec<String>,
}

impl Pairs {
    /// `mark` (a text, an edge, a fill) drawn on `ground` has at least
    /// `least` to one against it.
    fn at_least(&mut self, what: &str, mark: Hsla, ground: Flat, least: f32) {
        let ratio = contrast(over(mark, ground), ground);
        if ratio < least {
            self.short.push(format!(
                "{} {what}: {ratio:.2} to one, wanted {least}",
                self.mode
            ));
        }
    }
}

#[gpui_kit::test]
fn controls_are_distinguishable_from_the_surfaces_they_sit_on(cx: &mut TestAppContext) {
    let mut short = Vec::new();
    cx.update(gpui_kit::init);
    for mode in [ThemeMode::Dark, ThemeMode::Light] {
        cx.update(|cx| {
            theme::apply(mode, 1.0, cx);
            let t = cx.theme().clone();
            let mut pairs = Pairs {
                mode: if mode.is_dark() { "dark" } else { "light" },
                short: Vec::new(),
            };
            let page = over(t.background, [0.0; 3]);
            let tones = [Tone::Success, Tone::Warning, Tone::Danger];

            // Text against its background, on every surface text is drawn
            // on: the page (a dialog has the same fill), the navigation
            // pane, a popup, the fill under the pointer, a plain panel (a
            // review) and the heading row of a table.
            let grounds = [
                ("the page", page, t.foreground),
                ("the sidebar", over(t.sidebar, page), t.sidebar_foreground),
                ("a popup", over(t.popover, page), t.popover_foreground),
                ("a hovered row", over(t.accent, page), t.foreground),
                ("a panel", over(t.muted.opacity(0.5), page), t.foreground),
                ("a table's headings", over(t.table_head, page), t.foreground),
            ];
            for (name, ground, text) in grounds {
                pairs.at_least(&format!("text on {name}"), text, ground, 7.0);
                pairs.at_least(
                    &format!("secondary text on {name}"),
                    t.muted_foreground,
                    ground,
                    4.5,
                );
                pairs.at_least(&format!("a link on {name}"), t.link, ground, 4.5);
                // The tones: a status word, a validation message, a warning
                // of a review.
                for tone in tones {
                    let color = theme::tone_color(tone, cx);
                    pairs.at_least(&format!("{tone:?} text on {name}"), color, ground, 4.5);
                }
                // A disabled button's label is the secondary text at half
                // strength (the Kit's rule). It names the action its
                // tooltip explains, so it has to stay legible.
                pairs.at_least(
                    &format!("a disabled label on {name}"),
                    t.muted_foreground.opacity(0.5),
                    ground,
                    2.0,
                );
            }
            pairs.at_least(
                "a table's headings",
                t.table_head_foreground,
                over(t.table_head, page),
                4.5,
            );
            // A notice and an outcome in a tone: the tone at a tenth as the
            // fill, text and the tone's own colour on it, and its edge.
            for tone in tones {
                let color = theme::tone_color(tone, cx);
                let fill = over(color.opacity(0.10), page);
                pairs.at_least(
                    &format!("text on a {tone:?} panel"),
                    t.foreground,
                    fill,
                    7.0,
                );
                pairs.at_least(&format!("{tone:?} text on its panel"), color, fill, 4.5);
                pairs.at_least(
                    &format!("the edge of a {tone:?} panel"),
                    color.opacity(0.55),
                    page,
                    2.0,
                );
            }

            // Controls against their surfaces. The edge of a field, a
            // select, an unchecked box and an outlined button meets the
            // three to one of non-text contrast on the page and stays
            // clearly visible on the navigation pane; the focus ring and
            // the accent (a checked box, the chosen radio, the underline of
            // the current tab) meet it everywhere.
            pairs.at_least("a control's edge on the page", t.input, page, 3.0);
            for (name, ground, _) in grounds.iter().take(3) {
                pairs.at_least(
                    &format!("a control's edge on {name}"),
                    t.input,
                    *ground,
                    2.7,
                );
                pairs.at_least(&format!("the focus ring on {name}"), t.ring, *ground, 3.0);
                pairs.at_least(&format!("the accent on {name}"), t.primary, *ground, 3.0);
            }
            pairs.at_least(
                "a checked box's mark",
                t.primary_foreground,
                over(t.primary, page),
                4.5,
            );

            // The filled buttons: a label of body-text contrast in every
            // state, a fill that stands out from the page and from an
            // outlined button beside it, and that does not sink into the
            // page under the pointer.
            let outlined = over(t.input_background(), page);
            pairs.at_least(
                "an outlined button's label",
                t.button_foreground,
                outlined,
                7.0,
            );
            pairs.at_least("a ghost button's label", t.secondary_foreground, page, 7.0);
            let filled = [
                (
                    "primary",
                    [
                        t.button_primary,
                        t.button_primary_hover,
                        t.button_primary_active,
                    ],
                    t.button_primary_foreground,
                ),
                (
                    "danger",
                    [
                        t.button_danger,
                        t.button_danger_hover,
                        t.button_danger_active,
                    ],
                    t.button_danger_foreground,
                ),
                (
                    "warning",
                    [
                        t.button_warning,
                        t.button_warning_hover,
                        t.button_warning_active,
                    ],
                    t.button_warning_foreground,
                ),
            ];
            for (name, fills, label) in filled {
                pairs.at_least(
                    &format!("the {name} button beside an outlined one"),
                    fills[0],
                    outlined,
                    2.0,
                );
                for (state, fill) in ["", " under the pointer", " pressed"].iter().zip(fills) {
                    // At rest three to one; darker under the pointer, which
                    // on the dark page may cost a little of it.
                    let least = if state.is_empty() { 3.0 } else { 2.0 };
                    pairs.at_least(
                        &format!("the {name} button{state} on the page"),
                        fill,
                        page,
                        least,
                    );
                    pairs.at_least(
                        &format!("the {name} button's label{state}"),
                        label,
                        over(fill, page),
                        4.5,
                    );
                }
            }

            // The selected row of a grid and of the tree: visibly not the
            // surface, and text keeps its contrast on it.
            for (name, fill, surface, text) in [
                ("grid", t.table_active, page, t.foreground),
                (
                    "tree",
                    t.list_active,
                    over(t.sidebar, page),
                    t.sidebar_foreground,
                ),
            ] {
                pairs.at_least(&format!("the selected {name} row"), fill, surface, 1.2);
                let row = over(fill, surface);
                pairs.at_least(&format!("text on the selected {name} row"), text, row, 7.0);
                pairs.at_least(
                    &format!("secondary text on the selected {name} row"),
                    t.muted_foreground,
                    row,
                    4.5,
                );
                for tone in tones {
                    let color = theme::tone_color(tone, cx);
                    pairs.at_least(
                        &format!("{tone:?} text on the selected {name} row"),
                        color,
                        row,
                        4.5,
                    );
                }
            }
            short.extend(pairs.short);
        });
    }
    assert!(
        short.is_empty(),
        "too little contrast:\n{}",
        short.join("\n")
    );
}
