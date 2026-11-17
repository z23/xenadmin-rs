//! Saved graph presets: the data `Preferences` stores.
//!
//! This is the data half of the Rune `metric_presets.rs` (`Kind`, `Preset`,
//! `Catalog` with its validation), moved here ahead of the `performance`
//! area because `Preferences` embeds the `Catalog` and the stored file
//! format must not change. Saving, applying and removing a preset from the
//! Performance tab are still to be ported and build on this module.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use xenadmin_core::{MetricTarget, MetricsOptions};

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
