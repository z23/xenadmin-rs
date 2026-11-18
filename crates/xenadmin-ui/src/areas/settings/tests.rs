//! Tests of the Options, Help and About dialogs.
//!
//! The Rune tests of `settings.rs`:
//! - `options_cancel_and_apply_preserve_other_preferences_at_both_sizes`:
//!   ported below (the two window sizes and the "button is inside the
//!   window" assertions are layout and are the renderer's).
//! - `legacy_preferences_default_safely_and_force_review_cannot_be_disabled`:
//!   moved with `Options` to `prefs.rs` by the kernel.
//! - `help_topics_and_close_remain_reachable_on_compact_windows`: ported
//!   below without the 652 px assertion.
//! - `compact_console_details_returns_to_same_pinned_guest`: retired. It
//!   tested the compact swap between the console and the details pane, which
//!   no longer exists; that a console keeps its target belongs to the
//!   `console` area.

use super::*;
use crate::Harness;
use crate::prefs::Appearance;

#[test]
fn options_cancel_and_apply_preserve_other_preferences() {
    // Rune: options_cancel_and_apply_preserve_other_preferences_at_both_sizes
    let mut h = Harness::scene("options");
    h.model.preferences.endpoint = "https://synthetic.example.test".into();
    assert_eq!(h.surface().id, "settings-dialog");
    assert_eq!(h.surface().title, "Options");
    assert!(!h.surface().experimental);
    h.assert_unique_keys();
    for key in ["settings-close", "settings-apply"] {
        assert!(h.find(key).is_some(), "{key} is missing");
    }
    h.click("appearance-Light");
    assert_eq!(h.node("appearance-Light").checked, Some(true));
    h.click("settings-close");
    assert_eq!(h.model.preferences.options.appearance, Appearance::System);
    assert!(h.surfaces().is_empty());

    h.invoke("tools.options");
    assert_eq!(
        h.node("appearance-System").checked,
        Some(true),
        "a closed draft is discarded"
    );
    h.click("appearance-Dark");
    h.toggle("settings-review-power");
    h.click("settings-apply");
    assert_eq!(h.model.preferences.options.appearance, Appearance::Dark);
    assert!(h.model.preferences.options.review_power);
    assert_eq!(
        h.model.preferences.endpoint,
        "https://synthetic.example.test"
    );
    assert!(h.model.areas.settings.panel.is_none());
}

#[test]
fn the_interface_size_is_a_draft_until_apply() {
    let mut h = Harness::scene("options");
    assert_eq!(
        h.node("settings-scale-value").text.as_deref(),
        Some("100 %")
    );
    assert_eq!(
        h.try_click("settings-scale-reset"),
        Err("The interface is at its normal size.".into())
    );
    h.click("settings-scale-larger");
    h.click("settings-scale-larger");
    assert_eq!(
        h.node("settings-scale-value").text.as_deref(),
        Some("125 %")
    );
    assert_eq!(
        h.model.preferences.options.interface_scale, 100,
        "the window does not change size until Apply"
    );
    h.dismiss();
    assert_eq!(h.model.preferences.options.interface_scale, 100);

    h.invoke("tools.options");
    for _ in 0..2 {
        h.click("settings-scale-smaller");
    }
    assert_eq!(h.node("settings-scale-value").text.as_deref(), Some("80 %"));
    assert_eq!(
        h.try_click("settings-scale-smaller"),
        Err("The interface is at its smallest size.".into())
    );
    h.toggle("settings-console-size");
    h.toggle("settings-console-reconnect");
    h.toggle("settings-reduced-motion");
    h.click("settings-apply");
    let options = h.model.preferences.options;
    assert_eq!(options.interface_scale, 80);
    assert!(options.console_actual_size && !options.console_reconnect && options.reduced_motion);
    // Largest end.
    h.model.preferences.options.interface_scale = 200;
    h.invoke("tools.options");
    assert_eq!(
        h.try_click("settings-scale-larger"),
        Err("The interface is at its largest size.".into())
    );
}

#[test]
fn external_tools_is_offered_by_the_command_of_vm_tools() {
    // Was `external_tools_is_offered_only_while_its_area_provides_the_command`:
    // `vm_tools` registers `tools.external-tools` now.
    let mut h = Harness::scene("options");
    assert!(crate::command::find(CommandId("tools.external-tools")).is_some());
    assert_eq!(h.node("configure-external-tools").disabled, None);
    h.click("configure-external-tools");
    assert_ne!(h.surface().id, "options");
}

#[test]
fn help_topics_switch_and_close_stays_reachable() {
    // Rune: help_topics_and_close_remain_reachable_on_compact_windows
    let mut h = Harness::scene("help");
    assert_eq!(h.surface().title, "Help");
    assert!(!h.surface().experimental);
    h.assert_unique_keys();
    assert_eq!(h.node("help-Connections").checked, Some(true));
    for topic in [
        Topic::Recovery,
        Topic::Console,
        Topic::Storage,
        Topic::Diagnostics,
        Topic::Connect,
    ] {
        h.click(&format!("help-{}", topic.title()));
        assert_eq!(h.model.areas.settings.panel, Some(Panel::Help(topic)));
        assert!(h.has_text(topic.paragraphs()[0]));
        assert!(h.find("settings-close").is_some());
        assert!(h.find("settings-apply").is_none());
    }
    assert!(h.has_text("Unofficial XCP-ng client"));
    h.click("settings-close");
    assert!(h.model.areas.settings.panel.is_none());
}

#[test]
fn help_describes_the_new_shell_and_names_no_retired_control() {
    let all: Vec<&str> = Topic::ALL
        .into_iter()
        .flat_map(|topic| topic.paragraphs().iter().copied())
        .collect();
    let text = all.join("\n");
    // Controls of the Rune window that no longer exist.
    for retired in [
        "Details",
        "Back to console",
        "compact",
        "Activity",
        "Connections lets you",
        "Open console",
        "operation history",
    ] {
        assert!(!text.contains(retired), "Help still mentions {retired:?}");
    }
    // The shell the client has now.
    for present in [
        "Home tab",
        "Infrastructure view",
        "navigation pane",
        "Notifications view",
        "Events tab",
        "Alerts tab",
        "Console tab",
        "Tools menu",
        "Window menu",
        "status bar",
        "toolbar",
    ] {
        assert!(text.contains(present), "Help does not mention {present:?}");
    }
    // Facts that must survive the rewrite.
    for fact in [
        "Xen Orchestra is not required",
        "it does not repeat the original write",
        "Acknowledgement releases a local guard without proving the result",
        "Force power actions always require review",
        "stops after five failed attempts",
        "guest-quiesced snapshots are unavailable",
        "XENADMIN_DEV_INSPECT=1",
        "Never put host passwords in command-line arguments",
    ] {
        assert!(text.contains(fact), "Help lost the fact {fact:?}");
    }
}

#[test]
fn the_commands_are_in_the_tools_and_help_menus() {
    let mut h = Harness::new();
    for (command, label) in [
        ("tools.options", "Options"),
        ("help.help", "Help"),
        ("help.website", "Project website"),
        ("help.about", "About"),
    ] {
        let item = h.menu_item(command).expect(command);
        assert_eq!((item.label.as_str(), item.disabled), (label, None));
    }
    h.invoke("help.website");
    assert!(matches!(
        h.take_intents().as_slice(),
        [Intent::OpenUrl(url)] if url == WEBSITE_URL
    ));
    h.invoke("help.about");
    assert_eq!(h.surface().title, "About");
    assert!(h.has_text("Unofficial XCP-ng client"));
    h.click("help-website");
    assert!(matches!(
        h.take_intents().as_slice(),
        [Intent::OpenUrl(url)] if url == WEBSITE_URL
    ));
    h.dismiss();
    assert!(h.surfaces().is_empty());
}
