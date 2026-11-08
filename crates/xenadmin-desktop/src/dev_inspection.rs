use super::*;

pub(super) fn start(owns_instance: bool) -> (Option<xenadmin_core::DevInspection>, Option<String>) {
    if !owns_instance || std::env::var("XENADMIN_DEV_INSPECT").as_deref() != Ok("1") {
        return (None, None);
    }
    let result = preferences::directory()
        .ok_or_else(|| "No application state directory is available".to_owned())
        .and_then(|directory| {
            xenadmin_core::DevInspection::start(directory).map_err(|error| error.to_string())
        });
    match result {
        Ok(inspection) => {
            eprintln!(
                "Read-only developer inspection: {}",
                inspection.socket_path().display()
            );
            (Some(inspection), None)
        }
        Err(error) => (
            None,
            Some(format!("Cannot start developer inspection: {error}")),
        ),
    }
}

pub(super) fn refresh(s: &State) {
    if let Some(inspection) = &s.dev_inspection {
        inspection.set_connections(profiles_view::inspection_connections(s));
    }
}
