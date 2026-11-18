//! Saved graph presets: the Rune `metric_presets.rs`.
//!
//! The first half is the data `Preferences` stores (`Kind`, `Preset`,
//! `Catalog` with its validation); the stored file format is the Rune one.
//! The second half is what the Performance tab does with the catalog:
//! showing the presets in place of the charts, saving the current graph
//! under a name, applying a preset to the current target and removing one.
//! A preset holds a range and data-source names for a target type, never
//! samples; applying one never changes the target.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use xenadmin_core::{MetricTarget, MetricsOptions};

use super::metrics;
use crate::model::Model;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Kind {
    Vm,
    Host,
}
impl Kind {
    pub fn of(target: MetricTarget) -> Self {
        match target {
            MetricTarget::Vm(_) => Self::Vm,
            MetricTarget::Host(_) => Self::Host,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Preset {
    pub name: String,
    pub kind: Kind,
    pub options: MetricsOptions,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Catalog {
    pub presets: Vec<Preset>,
}
impl Catalog {
    pub fn validate(&self) -> Result<(), String> {
        let mut names = BTreeSet::new();
        if self.presets.len() > 32 {
            return Err("Keep at most 32 graph presets".into());
        }
        for p in &self.presets {
            if p.name.trim() != p.name
                || p.name.is_empty()
                || p.name.len() > 80
                || p.name.chars().any(char::is_control)
                || !names.insert((p.kind, p.name.as_str()))
            {
                return Err(
                    "Graph preset names must be unique, nonempty and at most 80 bytes".into(),
                );
            }
            p.options.validate().map_err(|e| e.to_string())?;
        }
        Ok(())
    }
    /// Adds the preset, or replaces the one of the same kind and name. The
    /// catalog is left as it was when the result would not be valid.
    pub fn save(&mut self, preset: Preset) -> Result<(), String> {
        let mut next = self.clone();
        if let Some(old) = next
            .presets
            .iter_mut()
            .find(|p| p.kind == preset.kind && p.name == preset.name)
        {
            *old = preset;
        } else {
            next.presets.push(preset);
        }
        next.validate()?;
        *self = next;
        Ok(())
    }
}

/// The notice after a save.
pub const SAVED: &str = "Graph preset saved on this computer";
/// The notice after a preset was applied.
pub const APPLIED: &str = "Graph preset applied to the current target";
/// The refusal of a preset with named sources before the target's source
/// list is known.
pub const WAIT: &str = "Wait for the target's data-source list before applying this preset";

/// Shows the presets in place of the charts, or the charts again (the Rune
/// `open`, which was a toggle).
pub fn toggle(m: &mut Model) {
    if let Some(view) = &mut m.areas.performance.metrics {
        view.show_presets = !view.show_presets;
        view.choosing_sources = false;
    }
}

/// Saves the current range and sources under the drafted name, for the
/// current target's type. An existing name is updated.
pub fn save(m: &mut Model) {
    let Some(view) = &m.areas.performance.metrics else {
        return;
    };
    let preset = Preset {
        name: view.preset_name.trim().into(),
        kind: Kind::of(view.target),
        options: view.options.clone(),
    };
    let result = m.preferences.metric_presets.save(preset);
    let saved = result.is_ok();
    if let Some(view) = &mut m.areas.performance.metrics {
        match result {
            Ok(()) => {
                view.error = None;
                view.notice = Some(SAVED.into());
            }
            Err(e) => view.error = Some(e),
        }
    }
    if saved {
        m.save_preferences();
    }
}

/// Applies the preset of this name and of the current target's type. A
/// preset that names sources the target does not advertise changes
/// nothing and says which.
pub fn apply(m: &mut Model, name: &str) {
    let Some(view) = &mut m.areas.performance.metrics else {
        return;
    };
    let Some(preset) = m
        .preferences
        .metric_presets
        .presets
        .iter()
        .find(|p| p.kind == Kind::of(view.target) && p.name == name)
        .cloned()
    else {
        return;
    };
    if let Err(e) = preset.options.validate() {
        view.error = Some(e.to_string());
        return;
    }
    if !preset.options.sources.is_empty() {
        let Some(snapshot) = &view.snapshot else {
            view.error = Some(WAIT.into());
            return;
        };
        let missing = preset
            .options
            .sources
            .iter()
            .filter(|name| !snapshot.sources.iter().any(|s| &s.name == *name))
            .cloned()
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            view.error = Some(format!(
                "This target does not advertise preset sources: {}. Choose its data sources and save a separate preset.",
                missing.join(", ")
            ));
            return;
        }
    }
    view.options = preset.options;
    view.preset_name = preset.name;
    view.show_presets = false;
    view.error = None;
    view.notice = Some(APPLIED.into());
    metrics::restart(m);
}

/// Removes the preset of this name and of the current target's type.
pub fn remove(m: &mut Model, name: &str) {
    let Some(view) = &m.areas.performance.metrics else {
        return;
    };
    let kind = Kind::of(view.target);
    m.preferences
        .metric_presets
        .presets
        .retain(|p| p.kind != kind || p.name != name);
    m.save_preferences();
}

#[cfg(test)]
mod tests {
    use super::*;

    // Rune: metric_presets.rs, the same name.
    #[test]
    fn invalid_preset_does_not_replace_previous_definition() {
        let mut catalog = Catalog::default();
        let good = Preset {
            name: "CPU".into(),
            kind: Kind::Vm,
            options: MetricsOptions::default(),
        };
        catalog.save(good.clone()).unwrap();
        let mut bad = good.clone();
        // A slash is a real separator in host source names; a query character never is.
        bad.options.sources = vec!["bad&source".into()];
        assert!(catalog.save(bad).is_err());
        assert_eq!(catalog.presets[0].options, good.options);
        for n in 1..32 {
            let mut next = good.clone();
            next.name = format!("Preset {n}");
            catalog.save(next).unwrap();
        }
        let mut extra = good;
        extra.name = "Overflow".into();
        assert!(catalog.save(extra).is_err());
        assert_eq!(catalog.presets.len(), 32);
    }

    #[test]
    fn a_catalog_keeps_its_stored_form() {
        let mut catalog = Catalog::default();
        catalog
            .save(Preset {
                name: "Host load".into(),
                kind: Kind::Host,
                options: MetricsOptions::default(),
            })
            .unwrap();
        let encoded = serde_json::to_value(&catalog).unwrap();
        assert_eq!(encoded["presets"][0]["name"], "Host load");
        assert_eq!(encoded["presets"][0]["kind"], "Host");
        let decoded: Catalog = serde_json::from_value(encoded).unwrap();
        assert!(decoded.validate().is_ok());
        assert_eq!(decoded.presets[0].kind, Kind::Host);
        assert!(Catalog::default().presets.is_empty());
    }
}
