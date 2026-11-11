//! The desktop's theme: the kit's neutral theme with control contrast raised.
//!
//! The neutral theme paints a button with the fill of the card behind it and a hairline at
//! five percent white, an outlined button with a border about as faint, and an unchecked
//! box with the same; on a real pool it was hard to tell a button from a label. This keeps
//! the neutral surfaces and text and changes only what marks a control as a control:
//!
//! - a standard button has its own fill and a visible edge;
//! - an outlined button's edge reaches three to one against the surface behind it;
//! - the emphasised button, used for the one action a view or dialog is for, takes the
//!   accent colour with white text at more than 4.5 to one;
//! - hover and press change the fill enough to see.
//!
//! Every view builds its `Kit` here, so the levels stay the same across the application.

use rune::{Color, El, Shadow, oklch, oklcha};
use rune_kit::{ButtonVariant, Kit};

/// The hue the neutral tokens lean toward, and the accent hue the kit uses for focus and
/// selection.
const NEUTRAL_HUE: f32 = 265.0;
const ACCENT_HUE: f32 = 256.0;

fn gray(lightness: f32) -> Color {
    oklch(lightness, 0.008, NEUTRAL_HUE)
}

fn ring(color: Color) -> Shadow {
    Shadow {
        spread: 1.0,
        color,
        ..Shadow::default()
    }
}

fn drop(y: f32, blur: f32, color: Color) -> Shadow {
    Shadow {
        y,
        blur,
        color,
        ..Shadow::default()
    }
}

/// A tab inside a [`tab_strip`]: the current tab is a raised button and the others are
/// labels, which the strip's own edge marks as one clickable control.
pub(crate) fn tab(current: bool) -> ButtonVariant {
    if current {
        ButtonVariant::Neutral
    } else {
        ButtonVariant::Ghost
    }
}

/// Groups a row of [`tab`] buttons into one bordered strip.
pub(crate) fn tab_strip(k: &Kit, tabs: El) -> El {
    tabs.self_start()
        .gap(2.0)
        .padding(3.0)
        .radius(k.radius.lg + 3.0)
        .fill(k.c.layer_02)
        .border(1.0, k.c.border_muted)
}

/// One of several mutually exclusive choices drawn as separate buttons, which may wrap:
/// the current choice is filled and the others are outlined.
pub(crate) fn choice(current: bool) -> ButtonVariant {
    if current {
        ButtonVariant::Neutral
    } else {
        ButtonVariant::Outline
    }
}

pub(crate) fn kit(dark: bool) -> Kit {
    let mut k = Kit::neutral(dark);
    // Lightness of: a standard button's fill, its edge, an outlined control's edge and the
    // emphasised button's fill. The fill is lighter than the dark surfaces and darker than
    // the light ones, so a filled button differs from an outlined one on either. The edges
    // are 2.6 and 3.0 to one against the card in both themes; the emphasised fill gives
    // white text 4.9 (dark) and 5.6 (light) to one.
    let (fill, edge, outline, emphasis) = if dark {
        (gray(0.31), gray(0.48), gray(0.52), 0.55)
    } else {
        (gray(0.915), gray(0.70), gray(0.66), 0.52)
    };
    let shadow = if dark {
        oklcha(0.0, 0.0, 0.0, 0.42)
    } else {
        oklcha(0.2, 0.02, NEUTRAL_HUE, 0.16)
    };
    let c = &mut k.c;
    c.bg_button_neutral = fill;
    c.border_muted = outline;
    // Unchecked and checked boxes, and the lines that separate menu groups.
    c.border_weak = outline;
    c.std_border = outline;
    c.border_hover = if dark { gray(0.66) } else { gray(0.5) };
    // Pointer feedback: the neutral theme's 1.6 percent white is not visible on a button.
    if dark {
        c.hover = oklcha(1.0, 0.0, 0.0, 0.07);
        c.pressed = oklcha(1.0, 0.0, 0.0, 0.12);
        c.surface_hover = c.hover;
        c.surface_raised_hover = c.hover;
    }
    // The emphasised button.
    c.bg_contrast = oklch(emphasis, 0.17, ACCENT_HUE);
    c.text_contrast = Color::WHITE;
    c.icon_contrast = Color::WHITE;
    // Buttons and text fields share this elevation: both get the visible edge, and a field
    // keeps the card's fill, so a field reads as inset and a button as raised.
    k.elevation.button_neutral = vec![drop(1.0, 1.5, shadow), ring(edge)];
    k
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WCAG contrast ratio of two opaque colours.
    fn contrast(a: Color, b: Color) -> f32 {
        let (a, b) = (a.luminance(), b.luminance());
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn controls_are_distinguishable_from_the_surfaces_they_sit_on() {
        for dark in [true, false] {
            let plain = Kit::neutral(dark);
            let k = kit(dark);
            let card = plain.c.bg_base;
            let page = plain.c.page;
            // An outlined control's edge meets the three to one that non-text contrast asks
            // for on the card, and stays clearly visible on the page behind the sidebar.
            assert!(
                contrast(k.c.border_muted, card) >= 3.0,
                "outline on card, dark={dark}"
            );
            assert!(
                contrast(k.c.border_muted, page) >= 2.7,
                "outline on page, dark={dark}"
            );
            let edge = k.elevation.button_neutral.last().unwrap().color;
            assert!(
                contrast(edge, card) >= 2.5,
                "button edge on card, dark={dark}"
            );
            // The emphasised button carries white text at body-text contrast and stands out
            // from a standard button's fill.
            assert!(
                contrast(Color::WHITE, k.c.bg_contrast) >= 4.5,
                "emphasised label, dark={dark}"
            );
            assert!(
                contrast(k.c.bg_contrast, k.c.bg_button_neutral) >= 2.0,
                "emphasised against standard, dark={dark}"
            );
            // Labels stay readable on the standard fill.
            assert!(contrast(k.c.text_base, k.c.bg_button_neutral) >= 7.0);
            // Surfaces and text are the neutral theme's own.
            assert_eq!(k.c.bg_base, plain.c.bg_base);
            assert_eq!(k.c.text_base, plain.c.text_base);
        }
    }
}
