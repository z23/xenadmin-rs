use super::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}
impl Appearance {
    fn label(self) -> &'static str {
        match self {
            Self::System => "Follow desktop",
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }
    pub fn dark(self, system: bool) -> bool {
        match self {
            Self::System => system,
            Self::Light => false,
            Self::Dark => true,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct Options {
    pub appearance: Appearance,
    pub reduced_motion: bool,
    pub review_power: bool,
    pub console_actual_size: bool,
    pub console_reconnect: bool,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            appearance: Appearance::System,
            reduced_motion: false,
            review_power: false,
            console_actual_size: false,
            console_reconnect: true,
        }
    }
}
impl Options {
    pub fn reviews(self, action: xenadmin_core::PowerAction) -> bool {
        action.is_force() || self.review_power
    }
}
#[derive(Clone, Copy)]
pub(super) enum Panel {
    Options(Options),
    Help(Topic),
}
#[derive(Clone, Copy)]
pub(super) enum Topic {
    Connect,
    Recovery,
    Console,
    Storage,
    Diagnostics,
}
impl Topic {
    fn title(self) -> &'static str {
        match self {
            Self::Connect => "Connections",
            Self::Recovery => "Operations and recovery",
            Self::Console => "Consoles",
            Self::Storage => "Snapshots and storage",
            Self::Diagnostics => "Development diagnostics",
        }
    }
    fn paragraphs(self) -> &'static [&'static str] {
        match self {
            Self::Connect => &[
                "Connect directly to an XCP-ng host management address. Any reachable member can discover its pool coordinator. Xen Orchestra is not required. Connections lets you save profiles, switch pools and reconnect as another user.",
                "Use normal certificate verification, a trusted private CA or independently verified endpoint fingerprints. The validation opt-out applies to all destinations for that connection, including coordinator and console endpoints. Its status remains visible.",
                "Passwords are session-only unless you explicitly save them in the desktop keyring. An unavailable or locked keyring leaves manual entry available. Automatic startup connections are configured per saved profile.",
            ],
            Self::Recovery => &[
                "Actions capture the selected object, pool and current connection. Review screens show the exact targets and consequences. The host checks capabilities again before execution; another operator or host task can change availability.",
                "A lost reply can leave Outcome unknown even when a host accepted the operation. Recheck outcome reads existing evidence; it does not repeat the original write. Request cancellation is a request to the host, not proof of cancellation.",
                "Activity keeps task progress, completed steps and partial resources. Acknowledge an unknown outcome only after inspecting the host and stopping other clients handling that operation. Acknowledgement releases a local guard without proving the result.",
                "Force power actions always require review. Options can also require a review for routine single-VM power actions. Bulk and configuration workflows retain their own target reviews.",
            ],
            Self::Console => &[
                "Open console pins input to the selected VM. Changing the inventory selection does not redirect keystrokes. Host text consoles use the selected host's verified control domain and show host details.",
                "The toolbar offers fit or actual size, deliberate key chords, fullscreen, reconnect and Release input. Focus loss releases held keys and buttons. Clipboard transfer and screenshot saving happen only when requested.",
                "Automatic reconnect follows the same VM after a reboot or same-pool host change. It pauses for stale inventory or a stopped guest and stops after five failed attempts. Use Reconnect to restart attempts; cross-pool transfers offer an explicit destination reopen.",
                "Console defaults in Options apply to newly opened consoles. The current console keeps its own fit and reconnect choices. At compact sizes, Details opens the pinned target's properties; Back to console returns without retargeting it.",
            ],
            Self::Storage => &[
                "A disk snapshot captures storage and configuration. A memory checkpoint also retains guest memory when the host supports it. Neither guarantees application consistency; guest-quiesced snapshots are unavailable on current XCP-ng targets.",
                "Restore reviews the exact parent and snapshot. Shutdown and start/resume choices are explicit. Snapshot deletion distinguishes its record from selected owned disk data; shared or unproven disks are retained.",
                "VM deletion, disk deletion, SR forgetting and SR destruction are separate operations. Read the selected UUIDs and retained-resource list. Growing a virtual disk does not grow partitions or filesystems inside the guest.",
                "Imports and cloud-init preparation retain created resource identities when interrupted. Cloud-init content can include secrets, so its local artifact stays private. Upload or attachment success does not prove the guest consumed that content.",
            ],
            Self::Diagnostics => &[
                "Check action availability in VM Details reads host explanations for disabled power actions without executing them. Activity exports contain object names and identifiers; keep them private when investigating a real pool.",
                "Explain outcome in Activity and operation history describes common host error codes and inspection steps. Its captured explanation keeps partial and unknown outcomes explicit; opening it does not reconnect, retry, change permissions or release an operation guard.",
                "Development builds can expose a same-user local inspection socket. Start with XENADMIN_DEV_INSPECT=1 and use the repository's scripts/inspect.sh helper. The inspector reads bounded client status and operation evidence; it does not submit host mutations or return passwords, sessions or console keystrokes.",
                "The read-only host-probe utility can inspect a connection or diagnose one VM. Prefer terminal password entry or a secret manager feeding standard input. Never put host passwords in command-line arguments or public reports.",
                "This development application separates fixture-tested behavior from live-host qualification. Destructive tests require a designated disposable lab. The repository's parity ledger and reports record current implementation and verification limits.",
            ],
        }
    }
}
pub(super) fn open(s: &mut State) {
    console_view::release(s);
    s.application_panel = Some(Panel::Options(s.preferences.options));
}
pub(super) fn help(s: &mut State) {
    console_view::release(s);
    s.application_panel = Some(Panel::Help(Topic::Connect));
}
fn apply(s: &mut State) {
    let Some(Panel::Options(options)) = s.application_panel else {
        return;
    };
    s.preferences.options = options;
    s.save_preferences();
    s.application_panel = None;
}
fn check(
    s: &State,
    k: &Kit,
    label: &'static str,
    key: &'static str,
    get: fn(&Options) -> bool,
    toggle: fn(&mut Options),
) -> El {
    let Some(Panel::Options(options)) = s.application_panel.as_ref() else {
        return el();
    };
    Checkbox::new(get(options))
        .label(label)
        .on_toggle(move |s: &mut State| {
            if let Some(Panel::Options(draft)) = &mut s.application_panel {
                toggle(draft);
            }
        })
        .build(k)
        .key(key)
}
pub(super) fn dialog(s: &State, k: &Kit) -> El {
    let Some(panel) = &s.application_panel else {
        return el();
    };
    let mut body = col().w_full().gap(12.0);
    let title = match panel {
        Panel::Options(options) => {
            let mut appearance = row().gap(6.0).flex_wrap();
            for choice in [Appearance::System, Appearance::Light, Appearance::Dark] {
                appearance = appearance.child(
                    Button::new(choice.label())
                        .variant(if choice == options.appearance {
                            ButtonVariant::Neutral
                        } else {
                            ButtonVariant::Outline
                        })
                        .on_press(move |s: &mut State| {
                            if let Some(Panel::Options(draft)) = &mut s.application_panel {
                                draft.appearance = choice;
                            }
                        })
                        .build(k)
                        .key(format!("appearance-{choice:?}")),
                );
            }
            body = body.child(heading("Appearance", k)).child(appearance)
                .child(check(s,k,"Reduce animation","settings-reduced-motion",|o|o.reduced_motion,|o|o.reduced_motion= !o.reduced_motion))
                .child(heading("Confirmations",k))
                .child(check(s,k,"Review routine VM power actions","settings-review-power",|o|o.review_power,|o|o.review_power= !o.review_power))
                .child(caption("Force actions and configuration changes always keep their required reviews.",k).wrap())
                .child(Button::new("Configure external tools…").on_press(external_tools::configure).build(k).key("configure-external-tools"))
                .child(heading("New console defaults",k))
                .child(check(s,k,"Use actual size instead of fit","settings-console-size",|o|o.console_actual_size,|o|o.console_actual_size= !o.console_actual_size))
                .child(check(s,k,"Reconnect automatically","settings-console-reconnect",|o|o.console_reconnect,|o|o.console_reconnect= !o.console_reconnect))
                .child(caption("These defaults apply when opening a console. Existing consoles keep their current choices.",k).wrap())
                .child(caption("Saved credentials, certificate policy, timeouts and automatic startup connections are configured per profile in Connections.",k).wrap());
            "Options"
        }
        Panel::Help(topic) => {
            let mut topics = row().gap(6.0).flex_wrap();
            for item in [
                Topic::Connect,
                Topic::Recovery,
                Topic::Console,
                Topic::Storage,
                Topic::Diagnostics,
            ] {
                topics = topics.child(
                    Button::new(item.title())
                        .variant(ButtonVariant::Outline)
                        .on_press(move |s: &mut State| {
                            s.application_panel = Some(Panel::Help(item))
                        })
                        .build(k)
                        .key(format!("help-{}", item.title())),
                );
            }
            body = body.child(topics).child(heading(topic.title(), k));
            for paragraph in topic.paragraphs() {
                body = body.child(caption(*paragraph, k).wrap());
            }
            body = body
                .child(
                    caption(
                        format!(
                            "XenAdmin RS {} • Unofficial XCP-ng client",
                            env!("CARGO_PKG_VERSION")
                        ),
                        k,
                    )
                    .wrap(),
                )
                .child(
                    Button::new("z23/xenadmin-rs on GitHub")
                        .variant(ButtonVariant::Ghost)
                        .on_press_tx(|s: &mut State, tx| {
                            s.error = tx
                                .open_url("https://github.com/z23/xenadmin-rs")
                                .err()
                                .map(|e| e.to_string());
                        })
                        .build(k),
                );
            "Help"
        }
    };
    let mut buttons = row().gap(8.0).justify_end().child(
        Button::new("Close")
            .variant(ButtonVariant::Ghost)
            .on_press(|s: &mut State| s.application_panel = None)
            .build(k)
            .key("settings-close"),
    );
    if matches!(panel, Panel::Options(_)) {
        buttons = buttons.child(
            Button::new("Apply")
                .on_press(apply)
                .build(k)
                .key("settings-apply"),
        );
    }
    let card = col()
        .w((s.window_width - 32.0).clamp(280.0, 660.0))
        .h((s.window_height - 48.0).clamp(280.0, 720.0))
        .min_h(0.0)
        .padding(20.0)
        .gap(12.0)
        .radius(10.0)
        .fill(k.c.bg_base)
        .child(heading(title, k))
        .child(body.grow(1.0).min_h(0.0).scroll(rune::ScrollSpec {
            stick_bottom: false,
        }))
        .child(buttons);
    el().w_full()
        .h_full()
        .absolute(0.0, 0.0)
        .center()
        .fill(rune::Color::BLACK.alpha(0.5))
        .role("dialog")
        .label(title)
        .modal(false, |s: &mut State| s.application_panel = None)
        .child(card)
        .key("settings-dialog")
}

#[cfg(test)]
mod tests {
    use super::*;
    use rune::{Env, Runtime};

    fn runtime(size: (f32, f32)) -> Runtime<State> {
        let mut state = init();
        open(&mut state);
        let mut rt = Runtime::new(
            state,
            view,
            size,
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        for _ in 0..30 {
            rt.frame(0.016);
        }
        rt
    }
    fn click(rt: &mut Runtime<State>, key: &str) {
        let (x, y) = rt.find(key).unwrap_or_else(|| panic!("Missing {key}"));
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(0.016);
    }
    #[test]
    fn options_cancel_and_apply_preserve_other_preferences_at_both_sizes() {
        for size in [(513.0, 652.0), (1440.0, 900.0)] {
            let mut rt = runtime(size);
            rt.state.preferences.endpoint = "https://synthetic.example.test".into();
            for key in ["settings-close", "settings-apply"] {
                let (x, y) = rt.find(key).unwrap();
                assert!(x > 0.0 && x < size.0 && y > 0.0 && y < size.1);
            }
            click(&mut rt, "appearance-Light");
            click(&mut rt, "settings-close");
            assert_eq!(rt.state.preferences.options.appearance, Appearance::System);
            rt.update(|s, _| open(s));
            rt.frame(0.016);
            click(&mut rt, "appearance-Dark");
            click(&mut rt, "settings-review-power");
            click(&mut rt, "settings-apply");
            assert_eq!(rt.state.preferences.options.appearance, Appearance::Dark);
            assert!(rt.state.preferences.options.review_power);
            assert_eq!(
                rt.state.preferences.endpoint,
                "https://synthetic.example.test"
            );
            assert!(rt.state.application_panel.is_none());
        }
    }
    #[test]
    fn legacy_preferences_default_safely_and_force_review_cannot_be_disabled() {
        let legacy: Preferences = serde_json::from_str(r#"{"sidebar_width":300.0}"#).unwrap();
        assert_eq!(legacy.options, Options::default());
        assert!(!legacy.options.reviews(xenadmin_core::PowerAction::Start));
        for action in xenadmin_core::PowerAction::ALL
            .into_iter()
            .filter(|a| a.is_force())
        {
            assert!(legacy.options.reviews(action));
        }
        let mut next = legacy;
        next.options.appearance = Appearance::Light;
        next.options.console_reconnect = false;
        let loaded: Preferences =
            serde_json::from_slice(&serde_json::to_vec(&next).unwrap()).unwrap();
        assert_eq!(next.options, loaded.options);
        assert!(!Appearance::Light.dark(true));
        assert!(Appearance::Dark.dark(false));
    }
    #[test]
    fn help_topics_and_close_remain_reachable_on_compact_windows() {
        let mut rt = runtime((513.0, 652.0));
        rt.update(|s, _| help(s));
        rt.frame(0.016);
        for topic in [
            Topic::Recovery,
            Topic::Console,
            Topic::Storage,
            Topic::Diagnostics,
            Topic::Connect,
        ] {
            click(&mut rt, &format!("help-{}", topic.title()));
            assert!(matches!(rt.state.application_panel, Some(Panel::Help(_))));
            let (_, y) = rt.find("settings-close").unwrap();
            assert!(y < 652.0);
        }
        click(&mut rt, "settings-close");
        assert!(rt.state.application_panel.is_none());
    }
    #[test]
    fn compact_console_details_returns_to_same_pinned_guest() {
        let mut state = init();
        console_view::recovery_scene(&mut state);
        let target = state.console_ui.target().unwrap().vm;
        let mut rt = Runtime::new(
            state,
            view,
            (513.0, 652.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        for _ in 0..30 {
            rt.frame(0.016);
        }
        click(&mut rt, "show-details");
        assert!(rt.state.show_details);
        assert!(rt.find("console-auto-reconnect").is_none());
        assert_eq!(rt.state.console_ui.target().unwrap().vm, target);
        click(&mut rt, "back-to-inventory");
        assert!(!rt.state.show_details);
        assert!(rt.find("console-auto-reconnect").is_some());
        assert_eq!(rt.state.console_ui.target().unwrap().vm, target);
    }
}
