use std::sync::Arc;
use xenadmin_core::DesktopInstance;

pub(super) enum Launch {
    Preview,
    Owner(DesktopInstance),
    Forwarded,
}
pub(super) fn preview(arguments: &[String]) -> bool {
    super::profiles_view::preview_arguments(arguments.iter().cloned())
        || arguments
            .iter()
            .any(|a| matches!(a.as_str(), "--help" | "-h"))
}
pub(super) fn prepare() -> Result<Launch, String> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if preview(&args) {
        return Ok(Launch::Preview);
    }
    let directory =
        super::preferences::directory().ok_or("No application state directory is available")?;
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
pub(super) fn attach(
    mut owner: DesktopInstance,
    handle: rune::AppHandle<super::State>,
) -> Result<DesktopInstance, String> {
    owner
        .start_activation(Arc::new(move |token| match token {
            Some(token) => handle.activate(token),
            None => handle.open(),
        }))
        .map_err(|e| format!("Cannot start desktop activation: {e}"))?;
    owner
        .retain_lock_until_exit()
        .map_err(|e| format!("Cannot retain desktop ownership: {e}"))?;
    Ok(owner)
}
#[cfg(test)]
mod tests {
    use super::*;
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
    #[test]
    fn quiet_state_has_no_backend_ownership_profile_restore_or_inspection() {
        let state = super::super::init();
        assert!(state._instance.is_none());
        assert!(state.backend.is_err());
        assert!(state.dev_inspection.is_none());
        assert!(
            super::super::profiles_view::inspection_connections(&state)
                .next()
                .is_none()
        );
    }
}
