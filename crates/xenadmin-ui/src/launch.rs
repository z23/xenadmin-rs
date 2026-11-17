//! How the process starts: a preview, the one owner of the state directory,
//! or a second start that hands over to the running owner.
//!
//! This is the toolkit-free part of the Rune `instance_launch.rs`, and
//! `preview_arguments` of `profiles_view.rs`. Wiring the activation to the
//! window is the front end's ([`attach`] takes the callback).

use std::sync::Arc;
use xenadmin_core::DesktopInstance;

pub enum Launch {
    /// Scenes, `--demo`, `--help`: no lock, no backend, nothing written.
    /// The model is [`crate::Model::fixture`].
    Preview,
    /// This process owns the state directory. The model is
    /// [`crate::Model::live`].
    Owner(DesktopInstance),
    /// Another instance owns the state directory and was asked to show
    /// itself. This process exits.
    Forwarded,
}

/// Whether the command line asks for a preview or a headless run, which
/// never restores saved sessions. The flags are those of the Rune front end;
/// `--demo` and `--scene` are the ones the new binary keeps.
pub fn preview_arguments(arguments: impl Iterator<Item = String>) -> bool {
    fn matches(argument: &str, flag: &str) -> bool {
        argument == flag
            || argument
                .strip_prefix(flag)
                .is_some_and(|tail| tail.starts_with('='))
    }
    let mut native = false;
    let mut preview = false;
    let mut unless_native = false;
    for argument in arguments {
        if argument == "--native" {
            native = true;
            continue;
        }
        if [
            "--demo",
            "--scene",
            "--snap",
            "--tree",
            "--html",
            "--video",
            "--strip",
            "--list-scenes",
            "--first-pixels",
            "--list-windows",
            "--desktop",
        ]
        .into_iter()
        .any(|flag| matches(&argument, flag))
        {
            preview = true;
        // Rune's headless() is false for these when --native is set.
        } else if ["--script", "--trace", "--frames", "--duration", "--out"]
            .into_iter()
            .any(|flag| matches(&argument, flag))
        {
            unless_native = true;
        }
    }
    preview || (unless_native && !native)
}

pub fn preview(arguments: &[String]) -> bool {
    preview_arguments(arguments.iter().cloned())
        || arguments
            .iter()
            .any(|a| matches!(a.as_str(), "--help" | "-h"))
}
/// Whether the command line asks which version this is. That is answered and nothing
/// else is done: no window, no state directory, no connection.
pub fn version_requested(arguments: &[String]) -> bool {
    arguments
        .iter()
        .any(|a| matches!(a.as_str(), "--version" | "-V"))
}
pub fn prepare() -> Result<Launch, String> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if preview(&args) {
        return Ok(Launch::Preview);
    }
    let directory =
        crate::prefs::directory().ok_or("No application state directory is available")?;
    match DesktopInstance::acquire(directory.clone())
        .map_err(|e| format!("Cannot acquire desktop ownership: {e}"))?
    {
        Some(owner) => Ok(Launch::Owner(owner)),
        None => {
            let token = std::env::var("XDG_ACTIVATION_TOKEN")
                .ok()
                .filter(|t| !t.is_empty())
                .or_else(|| std::env::var("DESKTOP_STARTUP_ID").ok());
            xenadmin_core::activate_desktop(&directory,token).map_err(|e|format!("Another XenAdmin RS instance owns this state directory, but activation failed: {e}. The existing client remains unchanged."))?;
            Ok(Launch::Forwarded)
        }
    }
}
/// Starts answering activation requests of later starts, and keeps the lock
/// until the process exits. `activate` runs on a core thread with the
/// activation token of the request, if it has one: it must only queue the
/// activation for the window.
pub fn attach(
    mut owner: DesktopInstance,
    activate: Arc<dyn Fn(Option<String>) + Send + Sync>,
) -> Result<DesktopInstance, String> {
    owner
        .start_activation(activate)
        .map_err(|e| format!("Cannot start desktop activation: {e}"))?;
    owner
        .retain_lock_until_exit()
        .map_err(|e| format!("Cannot retain desktop ownership: {e}"))?;
    Ok(owner)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Rune: instance_launch.rs, the same name.
    #[test]
    fn previews_and_help_do_not_claim_or_activate_live_desktop() {
        for flag in [
            "--demo",
            "--scene=connect",
            "--scene",
            "--snap",
            "--tree=preview.json",
            "--help",
            "-h",
        ] {
            assert!(preview(&[flag.into()]));
        }
        assert!(!preview(&[]));
        assert!(!preview(&["--theme".into(), "dark".into()]));
        assert!(!preview(&["--scene-lookalike".into()]));
    }

    // Rune: instance_launch.rs, the same name.
    #[test]
    fn a_version_question_is_recognised_wherever_it_stands() {
        assert!(version_requested(&["--version".into()]));
        assert!(version_requested(&["-V".into()]));
        assert!(version_requested(&["--".into(), "--version".into()]));
        assert!(!version_requested(&[]));
        assert!(!version_requested(&["--demo".into()]));
        assert!(!version_requested(&["--versions".into(), "-v".into()]));
    }

    // Rune: instance_launch.rs, the same name.
    #[test]
    fn quiet_state_has_no_backend_ownership_profile_restore_or_inspection() {
        let model = crate::Model::fixture();
        assert!(!model.owns_state());
        assert!(model.backend.is_err());
        assert!(model.dev_inspection.is_none());
        assert!(
            crate::session::inspection_connections(&model)
                .next()
                .is_none()
        );
    }

    // Rune: profile_restore_tests.rs, the flag lists of
    // `previews_do_not_restore_profiles_and_keyring_mutations_cannot_race_profile_saves`
    // and of `headless_flags_do_not_restore_a_saved_session` (the part that
    // needs no second process).
    #[test]
    fn headless_flags_are_previews_unless_native_is_asked_for() {
        for flag in ["--demo", "--scene=connect", "--snap", "--tree"] {
            assert!(preview_arguments([flag.to_owned()].into_iter()));
        }
        assert!(!preview_arguments(
            ["--theme".into(), "dark".into()].into_iter()
        ));
        for flag in [
            "--html",
            "--html=out.html",
            "--video",
            "--video=clip.mp4",
            "--strip",
            "--strip=out.png",
            "--list-scenes",
            "--first-pixels",
            "--list-windows",
            "--desktop",
            "--desktop=backdrop.png",
            "--script",
            "--script=replay.json",
            "--trace",
            "--trace=motion.jsonl",
            "--frames",
            "--frames=1",
            "--duration",
            "--duration=auto",
            "--out",
            "--out=frames",
        ] {
            let args = vec![flag.to_string()];
            assert!(preview_arguments(args.iter().cloned()), "{flag}");
            assert!(preview(&args), "{flag}");
        }
        for flag in [
            "--script",
            "--script=replay.json",
            "--trace",
            "--frames=1",
            "--duration=auto",
            "--out",
        ] {
            let args = vec![flag.to_string(), "--native".to_string()];
            assert!(!preview_arguments(args.iter().cloned()), "{flag} --native");
            assert!(!preview(&args), "{flag} --native");
        }
        for args in [
            vec!["--html-template".to_string()],
            vec!["--html-template=page.html".to_string()],
            vec!["--html-widths=360,640".to_string()],
        ] {
            assert!(!preview_arguments(args.iter().cloned()), "{args:?}");
        }
        let html_and_native = ["--native".to_string(), "--html=out.html".to_string()];
        assert!(preview_arguments(html_and_native.iter().cloned()));
    }
}
