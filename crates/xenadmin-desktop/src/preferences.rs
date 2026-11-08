use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub(super) metric_presets: crate::metrics_view::presets::Catalog,
    pub(super) options: crate::settings::Options,
    pub saved_searches: xenadmin_core::SearchCatalog,
    pub hidden_history: Vec<crate::activity::HiddenKey>,
    pub last_profile: Option<xenadmin_core::ProfileId>,
    pub endpoint: String,
    pub username: String,
    pub fingerprint: String,
    pub host_certificates: String,
    pub accept_invalid_certificates: bool,
    pub sidebar_width: f32,
    pub columns: BTreeMap<String, f32>,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            metric_presets: Default::default(),
            options: Default::default(),
            saved_searches: Default::default(),
            hidden_history: Vec::new(),
            last_profile: None,
            endpoint: String::new(),
            username: "root".into(),
            fingerprint: String::new(),
            host_certificates: String::new(),
            accept_invalid_certificates: false,
            sidebar_width: 260.0,
            columns: BTreeMap::new(),
        }
    }
}
pub(super) fn directory() -> Option<PathBuf> {
    std::env::var_os("XENADMIN_STATE_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("XDG_STATE_HOME").map(|p| PathBuf::from(p).join("xenadmin-rs"))
        })
        .or_else(|| {
            std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".local/state/xenadmin-rs"))
        })
}
impl Preferences {
    pub fn load() -> Self {
        let mut prefs: Self = directory()
            .and_then(|dir| std::fs::read(dir.join("preferences.json")).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        if !prefs.sidebar_width.is_finite() {
            prefs.sidebar_width = 260.0;
        }
        prefs.sidebar_width = prefs.sidebar_width.clamp(180.0, 420.0);
        prefs.hidden_history.truncate(5000);
        if prefs.metric_presets.validate().is_err() {
            prefs.metric_presets = Default::default();
        }
        if prefs.saved_searches.validate().is_err() {
            prefs.saved_searches = Default::default();
        }
        prefs
            .columns
            .retain(|_, width| width.is_finite() && (32.0..=2000.0).contains(width));
        prefs
    }
    pub fn save(&self) -> Result<(), String> {
        use std::{
            io::Write,
            os::unix::fs::{DirBuilderExt, OpenOptionsExt},
        };
        let dir = directory().ok_or("No application state directory is available")?;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&dir)
            .map_err(|_| "Cannot create the application state directory")?;
        let temporary = dir.join(format!("preferences.{}.tmp", uuid::Uuid::new_v4()));
        let result = (|| {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temporary)
                .map_err(|_| "Cannot write application preferences")?;
            file.write_all(
                &serde_json::to_vec_pretty(self)
                    .map_err(|_| "Cannot encode application preferences")?,
            )
            .map_err(|_| "Cannot write application preferences")?;
            file.sync_all()
                .map_err(|_| "Cannot persist application preferences")?;
            std::fs::rename(&temporary, dir.join("preferences.json"))
                .map_err(|_| "Cannot replace application preferences")
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        result.map_err(str::to_owned)
    }
}

/// The open file owns the advisory lock for the desktop session.
pub(super) struct InstanceGuard {
    _file: std::fs::File,
}
impl InstanceGuard {
    pub(super) fn acquire() -> Result<Self, String> {
        Self::at(directory().ok_or("No application state directory is available")?)
    }
    fn at(directory: PathBuf) -> Result<Self, String> {
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&directory)
            .map_err(|_| "Cannot create the application state directory")?;
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| "Cannot protect the application state directory")?;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(directory.join("desktop.lock"))
            .map_err(|_| "Cannot open the application session lock")?;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|_| "Cannot protect the application session lock")?;
        file.try_lock().map_err(|_| "Another XenAdmin RS window is using this state directory. Close it before connecting here.")?;
        Ok(Self { _file: file })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn desktop_lock_is_exclusive_and_released_on_drop() {
        let directory =
            std::env::temp_dir().join(format!("xenadmin-lock-test-{}", uuid::Uuid::new_v4()));
        let first = InstanceGuard::at(directory.clone()).unwrap();
        assert!(InstanceGuard::at(directory.clone()).is_err());
        drop(first);
        drop(InstanceGuard::at(directory.clone()).unwrap());
        std::fs::remove_dir_all(directory).unwrap();
    }
}
