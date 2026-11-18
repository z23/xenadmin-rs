//! `Preferences`: what the client remembers on this computer.
//!
//! The stored file is `preferences.json` in the application state directory,
//! and its format is that of the Rune front end, unchanged: a file written
//! by either front end is read by the other. `sidebar_width` and `columns`
//! are used by the new shell. `inspector_width` belonged to the Rune details
//! pane, which the new shell does not have: it is read, kept as it is and
//! written back, so the Rune front end still finds its width while both
//! exist (the plan drops it at cutover, Phase 5 step 3).
//!
//! `Options` is the data of the Options dialog (`settings.rs` in Rune). The
//! dialog itself is the `settings` area's.
//!
//! The catalogs embedded here are defined in the directories of the areas
//! that will own them: `areas/vm_tools/command.rs` (external tools),
//! `areas/performance/presets.rs` (graph presets) and `areas/events/hidden.rs`
//! (hidden history entries).

use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}
impl Appearance {
    pub fn label(self) -> &'static str {
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
pub struct Options {
    pub appearance: Appearance,
    pub reduced_motion: bool,
    pub review_power: bool,
    pub console_actual_size: bool,
    pub console_reconnect: bool,
    /// The size the whole interface is drawn at, as a percentage.
    pub interface_scale: u16,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            appearance: Appearance::System,
            reduced_motion: false,
            review_power: false,
            console_actual_size: false,
            console_reconnect: true,
            interface_scale: 100,
        }
    }
}
impl Options {
    /// The interface sizes on offer, as percentages.
    pub const SCALES: [u16; 8] = [80, 90, 100, 110, 125, 150, 175, 200];

    pub fn reviews(self, action: xenadmin_core::PowerAction) -> bool {
        action.is_force() || self.review_power
    }
    /// The factor the interface is drawn at. A saved value outside the offered range, as a
    /// hand-edited file could hold, is brought back into it.
    pub fn zoom(self) -> f32 {
        let (smallest, largest) = (Self::SCALES[0], Self::SCALES[Self::SCALES.len() - 1]);
        f32::from(self.interface_scale.clamp(smallest, largest)) / 100.0
    }
    /// The next offered size above or below the current one, or the current one at an end.
    pub fn scale_step(self, larger: bool) -> u16 {
        let current = self.interface_scale;
        if larger {
            Self::SCALES.into_iter().find(|scale| *scale > current)
        } else {
            Self::SCALES
                .into_iter()
                .rev()
                .find(|scale| *scale < current)
        }
        .unwrap_or_else(|| current.clamp(Self::SCALES[0], Self::SCALES[Self::SCALES.len() - 1]))
    }
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub external_tools: crate::areas::vm_tools::command::Catalog,
    pub metric_presets: crate::areas::performance::presets::Catalog,
    pub options: Options,
    pub saved_searches: xenadmin_core::SearchCatalog,
    pub hidden_history: Vec<crate::areas::events::hidden::HiddenKey>,
    pub last_profile: Option<xenadmin_core::ProfileId>,
    pub endpoint: String,
    pub username: String,
    pub fingerprint: String,
    pub host_certificates: String,
    pub accept_invalid_certificates: bool,
    /// The width of the navigation pane.
    pub sidebar_width: f32,
    /// The width of the Rune details pane. Not used by the new shell; kept
    /// so the file round-trips.
    pub inspector_width: f32,
    /// Widths the user gave to columns of the inventory grid, by column id.
    pub columns: BTreeMap<String, f32>,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            external_tools: Default::default(),
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
            inspector_width: 300.0,
            columns: BTreeMap::new(),
        }
    }
}
/// The application state directory: `$XENADMIN_STATE_DIR`, else
/// `$XDG_STATE_HOME/xenadmin-rs`, else `$HOME/.local/state/xenadmin-rs`.
pub fn directory() -> Option<PathBuf> {
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
    /// The narrowest and the widest the navigation pane may be.
    pub const SIDEBAR_RANGE: (f32, f32) = (180.0, 420.0);

    pub fn load() -> Self {
        directory()
            .map(|dir| Self::load_from(&dir))
            .unwrap_or_default()
    }
    /// Reads `preferences.json` of a state directory. A missing, unreadable
    /// or unparsable file gives the defaults.
    pub fn load_from(directory: &std::path::Path) -> Self {
        std::fs::read(directory.join("preferences.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Self>(&bytes).ok())
            .unwrap_or_default()
            .normalized()
    }
    /// Brings what a file held inside what the client accepts.
    pub fn normalized(self) -> Self {
        let mut prefs = self;
        if !prefs.sidebar_width.is_finite() {
            prefs.sidebar_width = 260.0;
        }
        prefs.sidebar_width = prefs
            .sidebar_width
            .clamp(Self::SIDEBAR_RANGE.0, Self::SIDEBAR_RANGE.1);
        if !prefs.inspector_width.is_finite() {
            prefs.inspector_width = 300.0;
        }
        prefs.inspector_width = prefs.inspector_width.clamp(230.0, 480.0);
        prefs.hidden_history.truncate(5000);
        if prefs.metric_presets.validate().is_err() {
            prefs.metric_presets = Default::default();
        }
        if prefs.saved_searches.validate().is_err() {
            prefs.saved_searches = Default::default();
        }
        if !prefs.external_tools.valid() {
            prefs.external_tools = Default::default();
        }
        prefs
            .columns
            .retain(|_, width| width.is_finite() && (32.0..=2000.0).contains(width));
        prefs
    }
    pub fn save(&self) -> Result<(), String> {
        let dir = directory().ok_or("No application state directory is available")?;
        self.save_to(&dir)
    }
    /// Writes `preferences.json` into a state directory: a private
    /// temporary file renamed over the old one.
    pub fn save_to(&self, dir: &std::path::Path) -> Result<(), String> {
        use std::{
            io::Write,
            os::unix::fs::{DirBuilderExt, OpenOptionsExt},
        };
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::areas::events::hidden::Family;

    /// A `preferences.json` as the Rune front end writes it, with every
    /// field present: two external tools, a graph preset, options, a saved
    /// search, a hidden history entry, the legacy connection fields, the
    /// pane widths and two column widths.
    pub(crate) const RUNE_FILE: &str = r#"{
  "external_tools": {
    "version": 1,
    "tools": [
      {
        "id": "7f2c1f0e-6a59-4a0e-9d55-0d6b3c1a9e01",
        "name": "SSH in default terminal",
        "kind": "Ssh",
        "executable": "/usr/bin/xdg-terminal-exec",
        "arguments": ["--", "/usr/bin/ssh", "-p", "{port}", "-l", "{user}", "--", "{ip}"]
      },
      {
        "id": "7f2c1f0e-6a59-4a0e-9d55-0d6b3c1a9e02",
        "name": "RDP with Remmina",
        "kind": "Rdp",
        "executable": "/usr/bin/remmina",
        "arguments": ["--connect", "{rdp_uri}"]
      }
    ]
  },
  "metric_presets": {
    "presets": [
      {
        "name": "CPU",
        "kind": "Vm",
        "options": METRICS_OPTIONS
      }
    ]
  },
  "options": {
    "appearance": "Dark",
    "reduced_motion": true,
    "review_power": true,
    "console_actual_size": false,
    "console_reconnect": false,
    "interface_scale": 125
  },
  "saved_searches": SAVED_SEARCHES,
  "hidden_history": [
    {
      "pool": "10000000-0000-0000-0000-000000000001",
      "id": "81000000-0000-0000-0000-000000000001",
      "family": "Resource"
    }
  ],
  "last_profile": "c0000000-0000-0000-0000-000000000007",
  "endpoint": "https://xcp-lab.example/",
  "username": "operator",
  "fingerprint": "",
  "host_certificates": "",
  "accept_invalid_certificates": false,
  "sidebar_width": 312.5,
  "inspector_width": 412.0,
  "columns": {
    "name": 321.0,
    "state": 150.0
  }
}"#;

    /// The sample with the two core-defined parts filled in by the core's
    /// own encoders, so the sample follows their format.
    pub(crate) fn rune_file() -> String {
        let options = serde_json::to_string(&xenadmin_core::MetricsOptions::default())
            .expect("metrics options");
        let searches = xenadmin_core::SearchCatalog {
            schema_version: 1,
            searches: vec![xenadmin_core::SavedSearch {
                id: uuid::Uuid::from_u128(0x5a00_0000_0000_0000_0000_0000_0000_0001),
                name: "Halted guests".into(),
                query: xenadmin_core::InventorySearch {
                    text: "lab".into(),
                    power: Some(xenadmin_core::PowerState::Halted),
                    ..Default::default()
                },
            }],
        };
        assert!(searches.validate().is_ok());
        RUNE_FILE.replace("METRICS_OPTIONS", &options).replace(
            "SAVED_SEARCHES",
            &serde_json::to_string(&searches).expect("search catalog"),
        )
    }

    #[test]
    fn a_rune_preferences_file_is_read_whole_and_written_back_unchanged() {
        let file = rune_file();
        let read: Preferences = serde_json::from_str(&file).expect("the Rune file is read");
        let read = read.normalized();
        assert_eq!(read.external_tools.tools.len(), 2);
        assert_eq!(read.external_tools.tools[1].name, "RDP with Remmina");
        assert_eq!(read.metric_presets.presets.len(), 1);
        assert_eq!(read.metric_presets.presets[0].name, "CPU");
        assert_eq!(read.options.appearance, Appearance::Dark);
        assert!(read.options.reduced_motion && read.options.review_power);
        assert!(!read.options.console_reconnect);
        assert_eq!(read.options.interface_scale, 125);
        assert_eq!(read.hidden_history.len(), 1);
        assert!(read.hidden_history[0].family == Family::Resource);
        assert_eq!(
            read.last_profile.map(|id| id.to_string()).as_deref(),
            Some("c0000000-0000-0000-0000-000000000007")
        );
        assert_eq!(read.endpoint, "https://xcp-lab.example/");
        assert_eq!(read.username, "operator");
        assert_eq!(read.sidebar_width, 312.5);
        assert_eq!(read.columns.get("name"), Some(&321.0));
        assert_eq!(read.columns.get("state"), Some(&150.0));
        // The Rune details pane is gone, and its width is kept for the Rune
        // front end, which reads the same file.
        assert_eq!(read.inspector_width, 412.0);

        // Written back, the file says the same thing, field for field.
        let written = serde_json::to_value(&read).expect("encoded");
        let original: serde_json::Value = serde_json::from_str(&file).expect("sample");
        assert_eq!(written, original);
        // The fourteen fields of the Rune struct, no more and no fewer. The
        // order of an object's keys depends on a serde_json feature that
        // another crate of the workspace may turn on, so it is not compared.
        let mut names: Vec<_> = written
            .as_object()
            .expect("an object")
            .keys()
            .map(String::as_str)
            .collect();
        names.sort_unstable();
        assert_eq!(
            names,
            [
                "accept_invalid_certificates",
                "columns",
                "endpoint",
                "external_tools",
                "fingerprint",
                "hidden_history",
                "host_certificates",
                "inspector_width",
                "last_profile",
                "metric_presets",
                "options",
                "saved_searches",
                "sidebar_width",
                "username",
            ]
        );
    }

    #[test]
    fn preferences_are_written_privately_and_read_back_from_a_state_directory() {
        use std::os::unix::fs::PermissionsExt;
        let directory =
            std::env::temp_dir().join(format!("xenadmin-ui-prefs-{}", uuid::Uuid::new_v4()));
        // Nothing there yet: the defaults.
        let fresh = Preferences::load_from(&directory);
        assert_eq!(fresh.username, "root");
        assert_eq!(fresh.sidebar_width, 260.0);
        let read: Preferences = serde_json::from_str(&rune_file()).expect("sample");
        read.save_to(&directory).expect("saved");
        let path = directory.join("preferences.json");
        let mode = std::fs::metadata(&path).expect("file").permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(
            std::fs::read_dir(&directory).expect("directory").count(),
            1,
            "the temporary file is gone"
        );
        let again = Preferences::load_from(&directory);
        assert_eq!(
            serde_json::to_value(&again).expect("encoded"),
            serde_json::to_value(read.normalized()).expect("encoded")
        );
        // An unparsable file gives the defaults instead of an error.
        std::fs::write(&path, b"{ not json").expect("written");
        assert_eq!(Preferences::load_from(&directory).sidebar_width, 260.0);
        let _ = std::fs::remove_dir_all(&directory);
    }

    // Rune: settings.rs, the same name.
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

    // Rune: main.rs `interface_size_scales_the_layout_and_follows_ctrl_plus_minus_and_zero`,
    // the part about `Options`. The key handling is the shell's.
    #[test]
    fn the_interface_scale_steps_through_the_offered_sizes_and_holds_at_the_ends() {
        let at = |interface_scale| Options {
            interface_scale,
            ..Default::default()
        };
        assert_eq!(at(100).scale_step(true), 110);
        assert_eq!(at(110).scale_step(true), 125);
        assert_eq!(at(125).scale_step(false), 110);
        assert_eq!(at(100).scale_step(false), 90);
        assert_eq!(at(100).zoom(), 1.0);
        assert_eq!(at(125).zoom(), 1.25);
        // The ends of the range hold, and a hand-edited value is brought back into it.
        assert_eq!(at(200).scale_step(true), 200);
        assert_eq!(at(80).scale_step(false), 80);
        assert_eq!(at(5000).zoom(), 2.0);
        assert_eq!(at(5000).scale_step(false), 200);
    }

    // Rune: main.rs `the_details_pane_width_reaches_the_preferences_and_the_next_start_uses_it`,
    // the part about what a file may hold.
    #[test]
    fn widths_a_file_holds_are_brought_inside_what_the_client_accepts() {
        // A file written before the width was kept starts at the declared width, and a
        // width outside what the pane allows is brought inside it.
        let earlier = serde_json::from_str::<Preferences>(r#"{"sidebar_width":300.0}"#)
            .unwrap()
            .normalized();
        assert_eq!(earlier.inspector_width, 300.0);
        assert_eq!(earlier.sidebar_width, 300.0);
        let wide = serde_json::from_str::<Preferences>(
            r#"{"inspector_width":9000.0,"sidebar_width":9000.0}"#,
        )
        .unwrap()
        .normalized();
        assert_eq!(wide.inspector_width, 480.0);
        assert_eq!(wide.sidebar_width, 420.0);
        let columns = serde_json::from_str::<Preferences>(
            r#"{"columns":{"name":321.0,"state":4.0,"cpu":90000.0}}"#,
        )
        .unwrap()
        .normalized();
        assert_eq!(columns.columns.len(), 1);
        assert_eq!(columns.columns.get("name"), Some(&321.0));
    }

    #[test]
    fn an_invalid_catalog_is_replaced_and_the_rest_of_the_file_is_kept() {
        let file = rune_file().replace("\"version\": 1", "\"version\": 9");
        let read = serde_json::from_str::<Preferences>(&file)
            .expect("read")
            .normalized();
        // The default catalog: the two presets with fresh ids.
        assert!(read.external_tools.valid());
        assert_eq!(read.external_tools.tools.len(), 2);
        assert_eq!(read.metric_presets.presets.len(), 1);
        assert_eq!(read.username, "operator");
    }
}
