//! The one action of the command table and the key bindings of its
//! shortcuts.
//!
//! Every command of `xenadmin_ui::command::registry()` is run through
//! [`RunCommand`]: a menu item carries it, a shortcut is bound to it, and
//! the shell handles it once, on its root element.

use xenadmin_ui::command::{self, CommandId};

use crate::kit::{App, KeyBinding};

/// Runs the command with this id on the model's target.
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = xenadmin, no_json)]
pub struct RunCommand {
    pub id: &'static str,
}

/// The key context of the shell's root element.
pub const SHELL_CONTEXT: &str = "Shell";

/// The key context a console canvas gives itself while it has the keyboard.
/// A focused console forwards every key to its guest, so no command
/// shortcut is delivered inside it (the Rune `app_key` rule).
pub const CONSOLE_CONTEXT: &str = "Console";

/// Where the command shortcuts apply: anywhere in the shell except inside a
/// console. A dialog is drawn in the Kit's own layer outside the shell, so
/// no shortcut reaches a command while a dialog has the keyboard.
const SHORTCUT_CONTEXT: &str = "Shell && !Console";

/// The interface size commands. They also apply while a dialog is open.
pub const ZOOM_IN: CommandId = CommandId("view.zoom-in");
pub const ZOOM_OUT: CommandId = CommandId("view.zoom-out");
pub const ZOOM_RESET: CommandId = CommandId("view.zoom-reset");

/// Where the interface size shortcuts apply: everywhere except inside a
/// console.
const ZOOM_CONTEXT: &str = "!Console";

/// Whether `id` is one of the interface size commands.
pub fn is_zoom(id: CommandId) -> bool {
    [ZOOM_IN, ZOOM_OUT, ZOOM_RESET].contains(&id)
}

/// Whether every keystroke of a shortcut parses. `KeyBinding::new` panics on
/// one that does not.
fn valid(keys: &'static str) -> bool {
    !keys.trim().is_empty()
        && keys
            .split_whitespace()
            .all(|stroke| gpui_kit::Keystroke::parse(stroke).is_ok())
}

/// The bindings of the registry's shortcuts, and the shortcuts a command
/// has beside the one the registry names.
pub fn bindings() -> Vec<KeyBinding> {
    let mut bindings = Vec::new();
    for spec in command::registry() {
        let Some(keys) = spec.shortcut else {
            continue;
        };
        if !valid(keys) {
            eprintln!(
                "The shortcut `{keys}` of the command {} is not a keystroke and is not bound.",
                spec.id
            );
            continue;
        }
        let context = if is_zoom(spec.id) {
            ZOOM_CONTEXT
        } else {
            SHORTCUT_CONTEXT
        };
        bindings.push(KeyBinding::new(
            keys,
            RunCommand { id: spec.id.0 },
            Some(context),
        ));
    }
    // A command has one shortcut in the registry. Larger Interface is Ctrl
    // with equals there; Ctrl with plus is the same key with Shift on most
    // layouts and a key of its own on others.
    for keys in ["ctrl-+", "ctrl-shift-+", "ctrl-shift-="] {
        bindings.push(KeyBinding::new(
            keys,
            RunCommand { id: ZOOM_IN.0 },
            Some(ZOOM_CONTEXT),
        ));
    }
    bindings
}

/// Binds the shortcuts. Call it once, after `gpui_kit::init`.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys(bindings());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_registry_shortcut_is_a_keystroke() {
        for spec in command::registry() {
            if let Some(keys) = spec.shortcut {
                assert!(valid(keys), "{}: `{keys}`", spec.id);
            }
        }
    }

    #[test]
    fn the_interface_size_commands_are_those_of_the_registry() {
        for id in [ZOOM_IN, ZOOM_OUT, ZOOM_RESET] {
            assert!(command::find(id).is_some(), "{id}");
            assert!(is_zoom(id));
        }
        assert!(!is_zoom(CommandId("nav.back")));
        // Building the bindings parses every keystroke and every context.
        assert!(bindings().len() >= 3);
    }
}
