//! Area `settings`: the Options, Help and About dialogs.
//!
//! Rune module: the dialogs of `settings.rs`. The data (`Options`,
//! `Appearance`) lives in `prefs.rs`. Options is a draft until Apply; Close
//! discards it; Apply does not touch any other preference. The Help topics
//! are the Rune topics rewritten for the new shell (menus, tabs, navigation
//! views, the Notifications view): the facts are kept, references to the
//! Rune toolbar row, the details pane and the compact layout are gone.
//!
//! None of the three surfaces is experimental: the Rune code shows no notice
//! for them (`AREAS.md`, decision 1).

use crate::prefs::{Appearance, Options};
use crate::prelude::*;

mod help;
#[cfg(test)]
mod tests;

pub use help::Topic;

/// The state of the area. `m.areas.settings`.
#[derive(Default)]
pub struct State {
    /// The open dialog: the Options draft, a Help topic, or About.
    pub panel: Option<Panel>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    /// The draft. It reaches the preferences only through Apply.
    Options(Options),
    Help(Topic),
    About,
}

pub fn spec() -> AreaSpec {
    AreaSpec {
        commands,
        scenes,
        surfaces,
        ..AreaSpec::new("settings")
    }
}

// -- Commands -----------------------------------------------------------------

pub const OPTIONS: CommandId = CommandId("tools.options");
pub const HELP: CommandId = CommandId("help.help");
pub const WEBSITE: CommandId = CommandId("help.website");
pub const ABOUT: CommandId = CommandId("help.about");

/// The command of the `vm_tools` area that opens External Tools. Options
/// offers it while that area provides it.
const EXTERNAL_TOOLS: CommandId = CommandId("tools.external-tools");

pub const WEBSITE_URL: &str = "https://github.com/z23/xenadmin-rs";

fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new(OPTIONS.0, "Options", "settings")
            .menu(place::TOOLS_OPTIONS)
            .invoke(|m, _| open(m)),
        CommandSpec::new(HELP.0, "Help", "settings")
            .menu(place::HELP_HELP)
            .invoke(|m, _| help(m)),
        CommandSpec::new(WEBSITE.0, "Project website", "settings")
            .menu(place::HELP_WEBSITE)
            .invoke(|m, _| m.open_url(WEBSITE_URL)),
        CommandSpec::new(ABOUT.0, "About", "settings")
            .menu(place::HELP_ABOUT)
            .invoke(|m, _| m.areas.settings.panel = Some(Panel::About)),
    ]
}

// -- Logic --------------------------------------------------------------------

/// Opens Options on a draft copy of the stored options (the Rune `open`).
pub fn open(m: &mut Model) {
    m.areas.settings.panel = Some(Panel::Options(m.preferences.options));
}

/// Opens Help on its first topic (the Rune `help`).
pub fn help(m: &mut Model) {
    m.areas.settings.panel = Some(Panel::Help(Topic::Connect));
}

fn close(m: &mut Model) {
    m.areas.settings.panel = None;
}

fn draft(m: &mut Model) -> Option<&mut Options> {
    match &mut m.areas.settings.panel {
        Some(Panel::Options(options)) => Some(options),
        _ => None,
    }
}

/// Stores the draft. The interface size goes through
/// `Model::set_interface_scale`, as the Ctrl shortcuts do; nothing but the
/// options changes.
fn apply(m: &mut Model) {
    let Some(Panel::Options(options)) = m.areas.settings.panel else {
        return;
    };
    let scale = options.interface_scale;
    m.preferences.options = Options {
        interface_scale: m.preferences.options.interface_scale,
        ..options
    };
    m.set_interface_scale(scale);
    m.save_preferences();
    m.areas.settings.panel = None;
}

fn set_scale(m: &mut Model, target: u16) {
    if let Some(draft) = draft(m) {
        draft.interface_scale = target;
    }
}

// -- Surfaces -----------------------------------------------------------------

fn surfaces(m: &Model) -> Vec<Surface> {
    let Some(panel) = &m.areas.settings.panel else {
        return Vec::new();
    };
    let close_button = button("settings-close", "Close", close);
    let surface = match panel {
        Panel::Options(options) => Surface::dialog("settings-dialog", "Options")
            .body(options_body(m, options))
            .footer([
                close_button,
                button("settings-apply", "Apply", apply).primary(),
            ]),
        Panel::Help(topic) => Surface::dialog("settings-dialog", "Help")
            .body(help_body(*topic))
            .footer([close_button]),
        Panel::About => Surface::dialog("settings-dialog", "About")
            .small()
            .body(blocks![version(), website_button()])
            .footer([close_button]),
    };
    vec![surface.on_close(close)]
}

fn version() -> Text {
    caption(format!(
        "XenAdmin RS {} • Unofficial XCP-ng client",
        env!("CARGO_PKG_VERSION")
    ))
    .key("settings-version")
}

fn website_button() -> Button {
    button("help-website", "z23/xenadmin-rs on GitHub", |m| {
        m.open_url(WEBSITE_URL)
    })
}

fn options_body(m: &Model, options: &Options) -> Vec<Block> {
    let appearance = choice(
        "appearance",
        [Appearance::System, Appearance::Light, Appearance::Dark].map(|choice| {
            option(format!("{choice:?}"), choice.label()).key(format!("appearance-{choice:?}"))
        }),
        Some(&format!("{:?}", options.appearance)),
        |m, id| {
            let picked = match id {
                "Light" => Appearance::Light,
                "Dark" => Appearance::Dark,
                _ => Appearance::System,
            };
            if let Some(draft) = draft(m) {
                draft.appearance = picked;
            }
        },
    )
    .segmented();
    let (smaller, larger) = (options.scale_step(false), options.scale_step(true));
    let size = actions([
        button("settings-scale-smaller", "Smaller", move |m| {
            set_scale(m, smaller)
        })
        .disabled_if(
            smaller == options.interface_scale,
            "The interface is at its smallest size.",
        ),
        button("settings-scale-larger", "Larger", move |m| {
            set_scale(m, larger)
        })
        .disabled_if(
            larger == options.interface_scale,
            "The interface is at its largest size.",
        ),
        button("settings-scale-reset", "Reset", |m| set_scale(m, 100)).disabled_if(
            options.interface_scale == 100,
            "The interface is at its normal size.",
        ),
    ]);
    // External Tools belongs to another area. Its command opens that
    // area's dialog, so Options closes first, as the Rune "Configure
    // external tools…" did. The button is there while the command is.
    let target = m.target();
    let external = match m.eligibility(EXTERNAL_TOOLS, &target) {
        Eligibility::Hidden => None,
        eligibility => Some(
            button(
                "configure-external-tools",
                "Configure external tools…",
                |m| {
                    close(m);
                    let target = m.target();
                    m.invoke(EXTERNAL_TOOLS, &target);
                },
            )
            .disabled_opt(eligibility.reason()),
        ),
    };
    blocks![
        heading("Appearance"),
        appearance,
        heading("Interface size"),
        text(format!("{} %", options.interface_scale)).key("settings-scale-value"),
        size,
        caption(
            "Text and controls scale together. Ctrl with plus, minus or 0 changes the size at \
             once from anywhere outside a console."
        ),
        checkbox(
            "settings-reduced-motion",
            "Reduce animation",
            options.reduced_motion,
            |m| {
                if let Some(draft) = draft(m) {
                    draft.reduced_motion = !draft.reduced_motion;
                }
            }
        ),
        heading("Confirmations"),
        checkbox(
            "settings-review-power",
            "Review routine VM power actions",
            options.review_power,
            |m| {
                if let Some(draft) = draft(m) {
                    draft.review_power = !draft.review_power;
                }
            }
        ),
        caption("Force actions and configuration changes always keep their required reviews."),
        external,
        heading("New console defaults"),
        checkbox(
            "settings-console-size",
            "Use actual size instead of fit",
            options.console_actual_size,
            |m| {
                if let Some(draft) = draft(m) {
                    draft.console_actual_size = !draft.console_actual_size;
                }
            }
        ),
        checkbox(
            "settings-console-reconnect",
            "Reconnect automatically",
            options.console_reconnect,
            |m| {
                if let Some(draft) = draft(m) {
                    draft.console_reconnect = !draft.console_reconnect;
                }
            }
        ),
        caption(
            "These defaults apply when opening a console. Existing consoles keep their current \
             choices."
        ),
        caption(
            "Saved credentials, certificate policy, timeouts and automatic startup connections \
             are configured per saved connection, on the Home tab."
        ),
    ]
}

fn help_body(topic: Topic) -> Vec<Block> {
    let topics = choice(
        "help-topics",
        Topic::ALL
            .map(|item| option(item.id(), item.title()).key(format!("help-{}", item.title()))),
        Some(topic.id()),
        |m, id| {
            if let Some(topic) = Topic::ALL.into_iter().find(|topic| topic.id() == id) {
                m.areas.settings.panel = Some(Panel::Help(topic));
            }
        },
    )
    .segmented();
    blocks![
        topics,
        heading(topic.title()),
        topic
            .paragraphs()
            .iter()
            .map(|paragraph| caption(*paragraph))
            .collect::<Vec<_>>(),
        version(),
        website_button(),
    ]
}

// -- Scenes -------------------------------------------------------------------

fn scenes() -> Vec<SceneSpec> {
    vec![
        // The Rune scenes ran on the bare `init()` state: no inventory.
        SceneSpec::new("options", "settings", open),
        SceneSpec::new("help", "settings", help),
    ]
}
