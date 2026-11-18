//! The client's theme: the Kit's neutral theme with the product accent and
//! the control contrast of the Rune client.
//!
//! The Rune theme (`crates/xenadmin-desktop/src/theme.rs`) kept the neutral
//! surfaces and text and changed what marks a control as a control: the
//! emphasised button takes the accent with white text at more than 4.5 to
//! one, and an outlined control's edge reaches about three to one against
//! the surface behind it. The same values are carried here.
//!
//! In the Kit the brand colour is `primary` (a checked box, the selected
//! radio) and `button_primary` (the primary button, which has colours of
//! its own). `accent` is the neutral hover surface and stays as it is.

use xenadmin_ui::prefs::{Appearance, Options};
use xenadmin_ui::surface::Tone;

use crate::kit::{ActiveTheme, App, Hsla, Theme, ThemeMode, gpui_kit, oklch, px};

/// The accent hue of the Rune client.
const ACCENT_HUE: f32 = 256.0;
/// The hue the neutral greys lean toward.
const NEUTRAL_HUE: f32 = 265.0;
/// How much darker a filled button is under the pointer, and again when
/// pressed.
const HOVER_STEP: f32 = 0.04;
/// The Kit's base font size at an interface scale of 100.
const BASE_FONT_SIZE: f32 = 16.0;

fn accent(lightness: f32) -> Hsla {
    oklch(lightness, 0.17, ACCENT_HUE)
}

fn gray(lightness: f32) -> Hsla {
    oklch(lightness, 0.008, NEUTRAL_HUE)
}

/// The mode an appearance preference asks for. `System` follows the theme
/// the Kit has now (the application start syncs it with the desktop).
pub fn mode_for(appearance: Appearance, cx: &App) -> ThemeMode {
    match appearance {
        Appearance::Light => ThemeMode::Light,
        Appearance::Dark => ThemeMode::Dark,
        Appearance::System => Theme::global(cx).mode,
    }
}

/// Switches the theme mode and applies the accent and the interface scale.
///
/// `Theme::change` reloads the mode's registered theme and discards colour
/// edits, so the accent is applied again after every switch, in a separate
/// update.
pub fn apply(mode: ThemeMode, zoom: f32, cx: &mut App) {
    Theme::change(mode, None, cx);
    let dark = mode.is_dark();
    Theme::update(cx, |theme| {
        // The emphasised fill gives white text 4.9 (dark) and 5.6 (light)
        // to one. Under the pointer and pressed a filled button darkens, so
        // its white label only gains contrast.
        let emphasis = if dark { 0.55 } else { 0.52 };
        let white = oklch(1.0, 0.0, 0.0);
        theme.primary = accent(emphasis);
        theme.primary_hover = accent(emphasis - HOVER_STEP);
        theme.primary_active = accent(emphasis - HOVER_STEP * 2.0);
        theme.primary_foreground = white;
        // The primary button has colours of its own in the Kit.
        theme.button_primary = accent(emphasis);
        theme.button_primary_hover = accent(emphasis - HOVER_STEP);
        theme.button_primary_active = accent(emphasis - HOVER_STEP * 2.0);
        theme.button_primary_foreground = white;
        theme.progress_bar = accent(emphasis);
        theme.link = accent(if dark { 0.72 } else { 0.50 });
        // The selected row of a table and of a list: the accent, faint.
        theme.table_active = accent(emphasis).opacity(if dark { 0.30 } else { 0.16 });
        theme.table_active_border = accent(emphasis).opacity(0.0);
        theme.list_active = accent(emphasis).opacity(if dark { 0.26 } else { 0.14 });
        // The destructive and the warning button ("Delete", "Review unknown
        // outcome", "Acknowledge outcome"). The Kit's are a pale fill with a
        // label in the status colour: salmon on pink and yellow on cream in
        // the light theme, and a fill that is hard to tell from the surface
        // in both. Here they are filled as the primary button is: a fill of
        // three to one or more against the surface, white text of more than
        // 4.5 to one on it.
        let red = |lightness: f32| oklch(lightness, 0.19, 27.0);
        let danger = if dark { 0.53 } else { 0.50 };
        theme.button_danger = red(danger);
        theme.button_danger_hover = red(danger - HOVER_STEP);
        theme.button_danger_active = red(danger - HOVER_STEP * 2.0);
        theme.button_danger_foreground = white;
        let amber = |lightness: f32| oklch(lightness, 0.12, 70.0);
        let warning = 0.50;
        theme.button_warning = amber(warning);
        theme.button_warning_hover = amber(warning - HOVER_STEP);
        theme.button_warning_active = amber(warning - HOVER_STEP * 2.0);
        theme.button_warning_foreground = white;
        // Secondary text. The Kit's light grey has 4.7 to one on white and
        // less than 4.5 on a selected row and on the muted surface; halved
        // for a disabled label it falls under two to one.
        if !dark {
            theme.muted_foreground = gray(0.50);
        }
        // A table's column headings are secondary text, not fainter.
        theme.table_head_foreground = theme.muted_foreground;
        // What is behind a dialog is dimmed enough to read as out of reach.
        theme.overlay = gpui_kit::hsla(0.0, 0.0, 0.0, if dark { 0.55 } else { 0.32 });
        theme.ring = accent(if dark { 0.68 } else { 0.55 });
        // The edge of a text field, a select and an unchecked box.
        theme.input = gray(if dark { 0.52 } else { 0.66 });
        theme.font_size = px(BASE_FONT_SIZE * zoom);
    });
}

/// The colour of a text in a tone. The Kit's status colours are fills: as
/// text on the light surface they have too little contrast, so the light
/// theme takes darker ones (each more than 4.5 to one on white).
pub fn tone_color(tone: Tone, cx: &App) -> Hsla {
    let theme = cx.theme();
    let dark = theme.mode.is_dark();
    match tone {
        Tone::Normal => theme.foreground,
        Tone::Muted => theme.muted_foreground,
        Tone::Success if dark => oklch(0.80, 0.17, 150.0),
        Tone::Success => oklch(0.47, 0.13, 150.0),
        Tone::Warning if dark => oklch(0.83, 0.15, 85.0),
        Tone::Warning => oklch(0.50, 0.12, 70.0),
        Tone::Danger if dark => oklch(0.74, 0.17, 25.0),
        Tone::Danger => oklch(0.50, 0.19, 27.0),
    }
}

/// [`apply`] from the stored options.
pub fn apply_options(options: Options, cx: &mut App) {
    let mode = mode_for(options.appearance, cx);
    apply(mode, options.zoom(), cx);
}
