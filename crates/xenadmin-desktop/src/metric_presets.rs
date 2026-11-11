use super::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
enum Kind {
    Vm,
    Host,
}
impl Kind {
    fn of(target: MetricTarget) -> Self {
        match target {
            MetricTarget::Vm(_) => Self::Vm,
            MetricTarget::Host(_) => Self::Host,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Preset {
    name: String,
    kind: Kind,
    options: MetricsOptions,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct Catalog {
    presets: Vec<Preset>,
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
    fn save(&mut self, preset: Preset) -> Result<(), String> {
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
pub(super) fn open(s: &mut State) {
    if let Some(view) = &mut s.metrics {
        view.show_presets = !view.show_presets;
        view.choosing_sources = false;
    }
}
fn save(s: &mut State) {
    let Some(view) = &s.metrics else { return };
    let preset = Preset {
        name: view.preset_name.text.trim().into(),
        kind: Kind::of(view.target),
        options: view.options.clone(),
    };
    match s.preferences.metric_presets.save(preset) {
        Ok(()) => {
            s.metrics.as_mut().unwrap().error = None;
            s.metrics.as_mut().unwrap().notice = Some("Graph preset saved on this computer".into());
            s.save_preferences();
        }
        Err(e) => s.metrics.as_mut().unwrap().error = Some(e),
    }
}
fn apply(s: &mut State, name: &str) {
    let Some(view) = &mut s.metrics else { return };
    let Some(preset) = s
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
            view.error =
                Some("Wait for the target's data-source list before applying this preset".into());
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
    view.preset_name.text = preset.name;
    view.show_presets = false;
    view.error = None;
    view.notice = Some("Graph preset applied to the current target".into());
    if s.demo {
        populate_preview(s)
    } else {
        reload(s)
    }
}
fn remove(s: &mut State, name: &str) {
    let Some(view) = &s.metrics else { return };
    let kind = Kind::of(view.target);
    s.preferences
        .metric_presets
        .presets
        .retain(|p| p.kind != kind || p.name != name);
    s.save_preferences();
}
pub(super) fn view(s: &State, k: &Kit) -> El {
    let Some(view) = &s.metrics else { return el() };
    let mut body=col().grow(1.0).min_h(0.0).gap(8.0).scroll(rune::ScrollSpec{stick_bottom:false})
        .child(heading("Saved graph presets",k))
        .child(caption("Presets keep a time range and data-source names for this target type. They contain no samples or host credentials. Saving an existing name updates it.",k).wrap())
        .child(TextInput::new(&view.preset_name).on_edit(|s:&mut State|&mut s.metrics.as_mut().unwrap().preset_name).placeholder("Preset name").build(k).key("metric-preset-name"))
        .child(Button::new("Save current graph").on_press(save).build(k).key("metric-preset-save"));
    for preset in s
        .preferences
        .metric_presets
        .presets
        .iter()
        .filter(|p| p.kind == Kind::of(view.target))
    {
        let name = preset.name.clone();
        let remove_name = name.clone();
        body = body.child(
            row()
                .gap(6.0)
                .flex_wrap()
                .child(
                    Button::new(&preset.name)
                        .on_press(move |s: &mut State| apply(s, &name))
                        .build(k)
                        .key(format!("metric-preset-{}", preset.name)),
                )
                .child(caption(
                    format!(
                        "{} · {}",
                        preset.options.window.label(),
                        if preset.options.sources.is_empty() {
                            "default sources".into()
                        } else {
                            format!("{} sources", preset.options.sources.len())
                        }
                    ),
                    k,
                ))
                .child(
                    Button::new("Remove")
                        .variant(ButtonVariant::Danger)
                        .on_press(move |s: &mut State| remove(s, &remove_name))
                        .build(k)
                        .key(format!("metric-preset-remove-{}", preset.name)),
                ),
        );
    }
    body.child(
        Button::new("Back to graphs")
            .variant(ButtonVariant::Outline)
            .on_press(open)
            .build(k)
            .key("metric-presets-close"),
    )
}

pub(crate) fn scene(s: &mut State) {
    super::scene(s);
    let view = s.metrics.as_mut().unwrap();
    view.preset_name = TextEdit::new("CPU and memory");
    view.options.sources = vec!["cpu0".into(), "memory".into()];
    save(s);
    open(s);
}
#[cfg(test)]
mod tests {
    use super::*;
    fn click(rt: &mut rune::Runtime<State>, key: &str) {
        let (x, y) = rt.find(key).unwrap_or_else(|| panic!("missing {key}"));
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        for _ in 0..30 {
            rt.frame(0.016);
        }
    }
    #[test]
    fn save_apply_and_remove_presets_preserve_the_target_at_both_sizes() {
        for size in [(513.0, 652.0), (1440.0, 900.0)] {
            let mut state = init();
            super::super::scene(&mut state);
            let view = state.metrics.as_mut().unwrap();
            let target = view.target;
            view.options = MetricsOptions {
                window: MetricsWindow::OneWeek,
                sources: vec!["cpu0".into()],
            };
            view.preset_name = TextEdit::new("CPU history");
            let mut rt = rune::Runtime::new(
                state,
                crate::view,
                size,
                rune::Env {
                    system_dark: true,
                    scale: 1.0,
                },
            );
            for _ in 0..30 {
                rt.frame(0.016);
            }
            click(&mut rt, "metric-presets-open");
            click(&mut rt, "metric-preset-save");
            assert_eq!(
                rt.state.preferences.metric_presets.presets.len(),
                1,
                "error {:?}; name {}",
                rt.state.metrics.as_ref().unwrap().error,
                rt.state.metrics.as_ref().unwrap().preset_name.text
            );
            rt.update(|s, _| s.metrics.as_mut().unwrap().options = MetricsOptions::default());
            rt.frame(0.016);
            click(&mut rt, "metric-preset-CPU history");
            let view = rt.state.metrics.as_ref().unwrap();
            assert_eq!(view.target, target);
            assert_eq!(view.options.window, MetricsWindow::OneWeek);
            assert_eq!(view.options.sources, vec!["cpu0"]);
            assert!(view.handle.is_none());
            click(&mut rt, "metric-presets-open");
            click(&mut rt, "metric-preset-remove-CPU history");
            assert!(rt.state.preferences.metric_presets.presets.is_empty());
        }
    }
    #[test]
    fn missing_sources_and_other_target_types_never_silently_change_the_graph() {
        let mut s = init();
        super::super::scene(&mut s);
        let before = s.metrics.as_ref().unwrap().options.clone();
        s.preferences
            .metric_presets
            .save(Preset {
                name: "Missing".into(),
                kind: Kind::Vm,
                options: MetricsOptions {
                    window: MetricsWindow::OneYear,
                    sources: vec!["not_advertised".into()],
                },
            })
            .unwrap();
        apply(&mut s, "Missing");
        assert_eq!(s.metrics.as_ref().unwrap().options, before);
        assert!(
            s.metrics
                .as_ref()
                .unwrap()
                .error
                .as_ref()
                .unwrap()
                .contains("not_advertised")
        );
        s.preferences
            .metric_presets
            .save(Preset {
                name: "Host".into(),
                kind: Kind::Host,
                options: MetricsOptions {
                    window: MetricsWindow::OneYear,
                    sources: Vec::new(),
                },
            })
            .unwrap();
        apply(&mut s, "Host");
        assert_eq!(s.metrics.as_ref().unwrap().options, before);
        let encoded = serde_json::to_vec(&s.preferences.metric_presets).unwrap();
        let decoded: Catalog = serde_json::from_slice(&encoded).unwrap();
        assert!(decoded.validate().is_ok());
    }
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
}
