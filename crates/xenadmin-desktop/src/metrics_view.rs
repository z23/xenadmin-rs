#[path = "metric_presets.rs"]
pub(super) mod presets;
use super::*;
use std::collections::BTreeSet;
use xenadmin_core::{
    MetricCategory, MetricSample, MetricSeries, MetricSource, MetricTarget, Metrics,
    MetricsOptions, MetricsSnapshot, MetricsState, MetricsWindow,
};

pub(super) struct MetricsUi {
    pool: PoolId,
    target: MetricTarget,
    name: String,
    options: MetricsOptions,
    handle: Option<Metrics>,
    snapshot: Option<Arc<MetricsSnapshot>>,
    choosing_sources: bool,
    selected_sources: BTreeSet<String>,
    source_filter: TextEdit,
    preset_name: TextEdit,
    show_presets: bool,
    notice: Option<String>,
    error: Option<String>,
}

fn target(s: &State) -> Option<MetricTarget> {
    let selected = s.selected()?;
    if let Some(host) = console_view::host(s) {
        Some(xenadmin_core::MetricTarget::Host(host))
    } else if s.page == Page::Hosts && s.console.is_none() {
        HostId::parse(&selected.id).ok().map(MetricTarget::Host)
    } else if s.page == Page::Guests || s.console.is_some() {
        selected.vm.map(MetricTarget::Vm)
    } else {
        None
    }
}
pub(super) fn button(s: &State, k: &Kit) -> El {
    if target(s).is_none() {
        return el();
    }
    Button::new("Performance")
        .variant(ButtonVariant::Outline)
        .disabled(
            !s.demo
                && !s
                    .snapshot
                    .as_ref()
                    .is_some_and(|snapshot| snapshot.is_current()),
        )
        .on_press(open)
        .build(k)
        .key("open-performance")
}
pub(super) fn open(s: &mut State) {
    let Some(target) = target(s) else {
        return;
    };
    let Some(inventory) = &s.inventory else {
        return;
    };
    let name = s
        .selected()
        .map(|selected| selected.name.clone())
        .unwrap_or_default();
    let pool = inventory.pool_id;
    console_view::release(s);
    s.console = None;
    s.console_snapshot = None;
    s.devices = None;
    s.snapshot_view = None;
    s.page = match target {
        MetricTarget::Vm(_) => Page::Guests,
        MetricTarget::Host(_) => Page::Hosts,
    };
    s.host_filter = None;
    s.rebuild();
    s.table.select(target.uuid().to_string());
    s.metrics = Some(MetricsUi {
        pool,
        target,
        name,
        options: MetricsOptions::default(),
        handle: None,
        snapshot: None,
        choosing_sources: false,
        selected_sources: BTreeSet::new(),
        source_filter: TextEdit::new(""),
        preset_name: TextEdit::new(""),
        show_presets: false,
        notice: None,
        error: None,
    });
    s.show_details = false;
    if s.demo {
        populate_preview(s);
    } else {
        reload(s);
    }
}
fn reload(s: &mut State) {
    let Some(view) = &mut s.metrics else {
        return;
    };
    view.handle = None;
    view.snapshot = None;
    let result = (|| {
        let snapshot = s
            .snapshot
            .as_ref()
            .filter(|snapshot| {
                snapshot
                    .inventory()
                    .is_some_and(|inventory| inventory.pool_id == view.pool)
            })
            .ok_or(xenadmin_core::Error::Stale)?;
        let observed = snapshot.observe_metrics(view.target)?;
        s.connection
            .as_ref()
            .ok_or(xenadmin_core::Error::Closed)?
            .subscribe_metrics(observed, view.options.clone())
    })();
    match result {
        Ok(handle) => {
            if let Some(waker) = &s.waker {
                handle.notify_on_change(waker.clone());
            }
            view.snapshot = Some(handle.latest());
            view.handle = Some(handle);
            view.error = None;
        }
        Err(error) => view.error = Some(error.to_string()),
    }
}
pub(super) fn poll(s: &mut State) -> bool {
    let Some(view) = &mut s.metrics else {
        return false;
    };
    let Some(handle) = &view.handle else {
        return false;
    };
    let latest = handle.latest();
    if view
        .snapshot
        .as_ref()
        .is_some_and(|old| Arc::ptr_eq(old, &latest))
    {
        return false;
    }
    view.snapshot = Some(latest);
    true
}
fn window(s: &mut State, window: MetricsWindow) {
    if let Some(view) = &mut s.metrics {
        view.options.window = window;
    }
    if s.demo {
        populate_preview(s);
    } else {
        reload(s);
    }
}
fn choose_sources(s: &mut State) {
    let Some(view) = &mut s.metrics else {
        return;
    };
    view.choosing_sources = true;
    view.show_presets = false;
    view.selected_sources = if view.options.sources.is_empty() {
        view.snapshot
            .iter()
            .flat_map(|snapshot| snapshot.series.iter().map(|series| series.source.clone()))
            .collect()
    } else {
        view.options.sources.iter().cloned().collect()
    };
}
fn apply_sources(s: &mut State, defaults: bool) {
    let Some(view) = &mut s.metrics else {
        return;
    };
    view.options.sources = if defaults {
        Vec::new()
    } else {
        view.selected_sources.iter().cloned().collect()
    };
    view.choosing_sources = false;
    if s.demo {
        populate_preview(s);
    } else {
        reload(s);
    }
}

pub(super) fn view(s: &State, k: &Kit) -> El {
    let Some(view) = &s.metrics else {
        return el();
    };
    let mut content = col()
        .w_full()
        .h_full()
        .min_h(0.0)
        .padding(20.0)
        .gap(12.0)
        .child(
            row()
                .flex_wrap()
                .gap(8.0)
                .child(heading(format!("Performance · {}", view.name), k).grow(1.0))
                .child(
                    Button::new("Back to inventory")
                        .on_press(|s: &mut State| s.metrics = None)
                        .build(k)
                        .key("close-performance"),
                ),
        )
        .child(
            caption(
                if s.window_width < 800.0 {
                    view.target.uuid().to_string()
                } else {
                    format!("{} · Pool {}", view.target.uuid(), view.pool)
                },
                k,
            )
            .wrap_anywhere(),
        );
    let mut controls = row().flex_wrap().gap(6.0);
    for period in [
        MetricsWindow::TenMinutes,
        MetricsWindow::TwoHours,
        MetricsWindow::OneWeek,
        MetricsWindow::OneYear,
    ] {
        controls = controls.child(
            Button::new(period.label())
                .variant(crate::theme::choice(view.options.window == period))
                .on_press(move |s: &mut State| window(s, period))
                .build(k)
                .key(format!("metrics-window-{}", period.duration_seconds())),
        );
    }
    controls = controls
        .child(
            Button::new("Data sources…")
                .on_press(choose_sources)
                .build(k)
                .key("metrics-sources"),
        )
        .child(
            Button::new("Reconnect metrics")
                .disabled(
                    s.demo
                        || !s
                            .snapshot
                            .as_ref()
                            .is_some_and(|snapshot| snapshot.is_current()),
                )
                .on_press(reload)
                .build(k),
        );
    controls = controls.child(
        Button::new("Graph presets…")
            .on_press(presets::open)
            .build(k)
            .key("metric-presets-open"),
    );
    content = content.child(controls);
    if let Some(notice) = &view.notice {
        content = content.child(caption(notice, k).wrap());
    }
    if let Some(error) = &view.error {
        content = content.child(caption(error, k).wrap().color(k.c.danger.icon));
    }
    let mut charts = col()
        .gap(16.0)
        .grow(1.0)
        .min_h(0.0)
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        });
    if view.show_presets {
        return content.child(presets::view(s, k));
    }
    if view.choosing_sources {
        return content.child(source_picker(s, k));
    }
    if let Some(snapshot) = &view.snapshot {
        let state = match snapshot.state {
            MetricsState::Loading => "Loading performance data…",
            MetricsState::Ready => "Live performance data",
            MetricsState::Stale => "Performance data is stale",
            MetricsState::Failed => "Performance data unavailable",
            MetricsState::Closed => "Performance subscription closed",
        };
        content = content.child(heading(
            if s.demo {
                "Synthetic performance preview"
            } else {
                state
            },
            k,
        ));
        if let Some(error) = &snapshot.error {
            content = content.child(caption(error, k).wrap().color(k.c.danger.icon));
        }
        content = content.child(
            caption(
                format!(
                    "Available range: {} — {} · {} second samples · averages",
                    timestamp(snapshot.range_start),
                    timestamp(snapshot.range_end),
                    snapshot
                        .sample_step_seconds
                        .map(|step| step.to_string())
                        .unwrap_or_else(|| "unknown".into())
                ),
                k,
            )
            .wrap(),
        );
        if snapshot.series.is_empty() {
            charts = charts.child(caption("No recorded data sources were returned for this target. Choose data sources or reconnect when the host has samples.", k).wrap());
        }
        for series in &snapshot.series {
            let source = snapshot
                .sources
                .iter()
                .find(|source| source.name == series.source);
            charts = charts.child(chart(series, source, snapshot.sample_step_seconds, k));
        }
    } else {
        charts = charts.child(caption("Waiting for performance data.", k));
    }
    content.child(charts).child(caption("Missing samples remain gaps. Historical coverage and resolution depend on the host's retained data.", k).wrap())
}
fn source_picker(s: &State, k: &Kit) -> El {
    let view = s.metrics.as_ref().unwrap();
    let mut choices = col()
        .gap(8.0)
        .grow(1.0)
        .min_h(0.0)
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        });
    let filter = view.source_filter.text.to_lowercase();
    if let Some(snapshot) = &view.snapshot {
        for source in snapshot.sources.iter().filter(|source| {
            format!("{} {}", source.name, source.description)
                .to_lowercase()
                .contains(&filter)
        }) {
            let name = source.name.clone();
            let chosen = view.selected_sources.contains(&name);
            choices = choices
                .child(
                    Checkbox::new(chosen)
                        .label(format!(
                            "{} · {}{}",
                            name,
                            source.units,
                            if source.enabled {
                                ""
                            } else {
                                " · not recorded"
                            }
                        ))
                        .disabled(!chosen && view.selected_sources.len() >= 32)
                        .on_toggle(move |s: &mut State| {
                            if let Some(view) = &mut s.metrics
                                && !view.selected_sources.remove(&name)
                                && view.selected_sources.len() < 32
                            {
                                view.selected_sources.insert(name.clone());
                            }
                        })
                        .build(k),
                )
                .child(caption(&source.description, k).wrap());
        }
    }
    col()
        .grow(1.0)
        .min_h(0.0)
        .gap(10.0)
        .child(heading(
            format!(
                "Data sources · {} of 32 selected",
                view.selected_sources.len()
            ),
            k,
        ))
        .child(
            TextInput::new(&view.source_filter)
                .label("Filter data sources")
                .on_edit(|s: &mut State| &mut s.metrics.as_mut().unwrap().source_filter)
                .build(k),
        )
        .child(choices)
        .child(
            row()
                .flex_wrap()
                .gap(8.0)
                .child(
                    Button::new("Cancel")
                        .on_press(|s: &mut State| {
                            if let Some(view) = &mut s.metrics {
                                view.choosing_sources = false;
                            }
                        })
                        .build(k),
                )
                .child(
                    Button::new("Common sources")
                        .on_press(|s: &mut State| apply_sources(s, true))
                        .build(k),
                )
                .child(
                    Button::new("Apply sources")
                        .disabled(view.selected_sources.is_empty())
                        .on_press(|s: &mut State| apply_sources(s, false))
                        .build(k)
                        .key("apply-metric-sources"),
                ),
        )
}
fn factor(units: &str) -> f64 {
    if units == "(fraction)" { 100.0 } else { 1.0 }
}
fn unit(units: &str) -> &str {
    match units {
        "(fraction)" | "percent" => "%",
        "" => "unit not reported",
        value => value,
    }
}
fn number(value: f64) -> String {
    let magnitude = value.abs();
    for (limit, suffix) in [(1e12, "T"), (1e9, "G"), (1e6, "M"), (1e3, "k")] {
        if magnitude >= limit {
            return format!("{:.2}{suffix}", value / limit);
        }
    }
    if value == 0.0 {
        "0".into()
    } else if magnitude < 0.01 {
        format!("{value:.2e}")
    } else {
        format!("{value:.2}")
    }
}
fn values(series: &MetricSeries, multiplier: f64) -> Vec<Option<f64>> {
    series
        .samples
        .iter()
        .map(|sample| {
            sample
                .value
                .map(|v| v * multiplier)
                .filter(|v| v.is_finite())
        })
        .collect()
}
fn segments(
    samples: &[MetricSample],
    values: &[Option<f64>],
    step: Option<u64>,
    low: f64,
    high: f64,
) -> Vec<Vec<[f32; 2]>> {
    let first = samples.first().map_or(0, |sample| sample.timestamp);
    let span = samples
        .last()
        .map_or(1, |sample| sample.timestamp.saturating_sub(first))
        .max(1) as f64;
    let mut paths = Vec::new();
    let mut path = Vec::new();
    let mut previous = None;
    let scale = low.abs().max(high.abs()).max(1.0);
    for (sample, value) in samples.iter().zip(values) {
        if (value.is_none()
            || previous.is_some_and(|last| {
                step.is_some_and(|step| {
                    sample.timestamp.saturating_sub(last) > step.saturating_add(step / 2)
                })
            }))
            && !path.is_empty()
        {
            paths.push(std::mem::take(&mut path));
        }
        if let Some(value) = value {
            path.push([
                (sample.timestamp.saturating_sub(first) as f64 / span * 1000.0) as f32,
                (120.0 - (value / scale - low / scale) / (high / scale - low / scale) * 120.0)
                    as f32,
            ]);
        }
        previous = Some(sample.timestamp);
    }
    if !path.is_empty() {
        paths.push(path);
    }
    paths
}
fn chart(series: &MetricSeries, source: Option<&MetricSource>, step: Option<u64>, k: &Kit) -> El {
    let units = source.map_or("unknown", |source| source.units.as_str());
    let values = values(series, factor(units));
    let finite: Vec<_> = values.iter().flatten().copied().collect();
    let (low, mut high) = finite
        .iter()
        .fold((0.0_f64, 0.0_f64), |(low, high), value| {
            (low.min(*value), high.max(*value))
        });
    if high == low {
        high = low + 1.0;
    }
    let mut shape = rune::Shape::new().view_box(1000.0, 120.0);
    for y in [0.0, 60.0, 120.0] {
        shape = shape.stroke(vec![[0.0, y], [1000.0, y]], 1.0, k.c.border_weak);
    }
    for mut points in segments(&series.samples, &values, step, low, high) {
        if points.len() == 1 {
            let point = points[0];
            points.push([point[0] + 0.2, point[1]]);
        }
        shape = shape.stroke(points, 1.8, accent());
    }
    let last = values
        .last()
        .copied()
        .flatten()
        .map(number)
        .unwrap_or_else(|| "Unavailable".into());
    let summary = format!(
        "Latest: {last} {} · {} valid, {} missing samples",
        unit(units),
        finite.len(),
        values.len() - finite.len()
    );
    let mut card = col()
        .gap(6.0)
        .padding(12.0)
        .radius(8.0)
        .border(1.0, k.c.border_weak)
        .child(heading(&series.source, k))
        .child(caption(source.map_or("", |source| source.description.as_str()), k).wrap())
        .child(caption(&summary, k).wrap());
    if finite.is_empty() {
        return card.child(caption("No finite samples in this range", k));
    }
    card = card
        .child(caption(format!("{} {}", number(high), unit(units)), k))
        .child(
            rune::canvas(shape)
                .w_full()
                .h(120.0)
                .label(format!("{} chart. {summary}", series.source)),
        )
        .child(caption(format!("{} {}", number(low), unit(units)), k));
    card.child(
        row()
            .flex_wrap()
            .justify_between()
            .child(caption(
                timestamp(series.samples.first().map(|sample| sample.timestamp)),
                k,
            ))
            .child(caption(
                timestamp(series.samples.last().map(|sample| sample.timestamp)),
                k,
            )),
    )
}
fn timestamp(value: Option<u64>) -> String {
    let Some(value) = value.filter(|value| *value <= 253_402_300_799) else {
        return "unknown".into();
    };
    let days = (value / 86400) as i64 + 719468;
    let era = days / 146097;
    let doe = days - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02} UTC",
        value / 3600 % 24,
        value / 60 % 60,
        value % 60
    )
}
fn populate_preview(s: &mut State) {
    let Some(view) = &mut s.metrics else {
        return;
    };
    let end = 1_794_096_000_u64;
    let step = view.options.window.interval_seconds();
    let sources = vec![
        MetricSource {
            name: "cpu0".into(),
            description: "CPU 0 utilization".into(),
            units: "(fraction)".into(),
            enabled: true,
            category: MetricCategory::Cpu,
        },
        MetricSource {
            name: "memory".into(),
            description: "Allocated guest memory".into(),
            units: "bytes".into(),
            enabled: true,
            category: MetricCategory::Memory,
        },
        MetricSource {
            name: "vbd_xvda_read".into(),
            description: "Disk read throughput".into(),
            units: "bytes/s".into(),
            enabled: true,
            category: MetricCategory::Disk,
        },
        MetricSource {
            name: "vif_0_rx".into(),
            description: "Network receive throughput".into(),
            units: "bytes/s".into(),
            enabled: true,
            category: MetricCategory::Network,
        },
    ];
    let series = sources
        .iter()
        .enumerate()
        .filter(|(_, source)| {
            view.options.sources.is_empty() || view.options.sources.contains(&source.name)
        })
        .map(|(index, source)| MetricSeries {
            source: source.name.clone(),
            samples: (0..120)
                .map(|sample| MetricSample {
                    timestamp: end - (119 - sample) * step,
                    value: if (45..52).contains(&sample) {
                        None
                    } else {
                        Some(match index {
                            0 => 0.4 + (sample as f64 / 8.0).sin() * 0.25,
                            1 => 4.0 * 1024.0_f64.powi(3),
                            _ => (1.2 + (sample as f64 / 7.0).sin()) * 2e6,
                        })
                    },
                })
                .collect(),
        })
        .collect();
    view.snapshot = Some(Arc::new(MetricsSnapshot {
        pool: view.pool,
        generation: 0,
        target: view.target,
        state: MetricsState::Ready,
        sources,
        series,
        updated_at_ms: Some(end * 1000),
        error: None,
        sample_step_seconds: Some(step),
        range_start: Some(end - 119 * step),
        range_end: Some(end),
    }));
}
pub(super) fn scene(s: &mut State) {
    demo(s);
    s.table.select("30000000-0000-0000-0000-000000000000");
    open(s);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn graph_preserves_missing_samples_time_gaps_zero_and_finite_extremes() {
        let samples = vec![
            MetricSample {
                timestamp: 0,
                value: Some(0.0),
            },
            MetricSample {
                timestamp: 5,
                value: Some(0.5),
            },
            MetricSample {
                timestamp: 10,
                value: None,
            },
            MetricSample {
                timestamp: 15,
                value: Some(0.7),
            },
            MetricSample {
                timestamp: 60,
                value: Some(1.0),
            },
        ];
        let series = MetricSeries {
            source: "cpu0".into(),
            samples,
        };
        let data = values(&series, factor("(fraction)"));
        assert_eq!(
            data,
            vec![Some(0.0), Some(50.0), None, Some(70.0), Some(100.0)]
        );
        let paths = segments(&series.samples, &data, Some(5), 0.0, 100.0);
        assert_eq!(
            paths.iter().map(Vec::len).collect::<Vec<_>>(),
            vec![2, 1, 1]
        );
        assert_eq!(paths[0][0], [0.0, 120.0]);
        let extreme = segments(
            &series.samples[..2],
            &[Some(-1e308), Some(1e308)],
            Some(5),
            -1e308,
            1e308,
        );
        assert!(
            extreme
                .iter()
                .flatten()
                .flatten()
                .all(|coordinate| coordinate.is_finite())
        );
        let nonfinite = MetricSeries {
            source: "unknown".into(),
            samples: vec![MetricSample {
                timestamp: 0,
                value: Some(f64::INFINITY),
            }],
        };
        assert_eq!(values(&nonfinite, 1.0), vec![None]);
    }
    #[test]
    fn timestamps_have_explicit_utc_calendar_dates_for_long_ranges() {
        assert_eq!(timestamp(Some(0)), "1970-01-01 00:00:00 UTC");
        assert_eq!(timestamp(Some(951_782_400)), "2000-02-29 00:00:00 UTC");
        assert_eq!(timestamp(Some(253_402_300_799)), "9999-12-31 23:59:59 UTC");
        assert_eq!(timestamp(Some(u64::MAX)), "unknown");
        assert_eq!(timestamp(None), "unknown");
    }
    #[test]
    fn performance_controls_show_actual_coverage_and_change_ranges_without_host_writes() {
        for size in [(513.0, 652.0), (1440.0, 900.0)] {
            let mut state = init();
            scene(&mut state);
            let mut runtime = rune::Runtime::new(
                state,
                crate::view,
                size,
                rune::Env {
                    system_dark: true,
                    scale: 1.0,
                },
            );
            for _ in 0..30 {
                runtime.frame(1.0 / 60.0);
            }
            let (x, y) = runtime.find("metrics-window-604800").unwrap();
            assert!(x > 0.0 && x < size.0 && y > 0.0 && y < size.1);
            runtime.pointer_down(x, y);
            runtime.pointer_up(x, y);
            runtime.frame(0.0);
            assert_eq!(
                runtime.state.metrics.as_ref().unwrap().options.window,
                MetricsWindow::OneWeek
            );
            let accessibility = runtime.accessibility_json().to_string();
            assert!(accessibility.contains("3600 second samples"));
            assert!(accessibility.contains("Synthetic performance preview"));
            assert!(runtime.state.metrics.as_ref().unwrap().handle.is_none());
            let (x, y) = runtime.find("metrics-sources").unwrap();
            runtime.pointer_down(x, y);
            runtime.pointer_up(x, y);
            runtime.frame(0.0);
            assert!(runtime.state.metrics.as_ref().unwrap().choosing_sources);
            runtime.update(|s, _| {
                s.metrics.as_mut().unwrap().selected_sources =
                    ["cpu0".to_owned()].into_iter().collect()
            });
            for _ in 0..30 {
                runtime.frame(1.0 / 60.0);
            }
            let (x, y) = runtime.find("apply-metric-sources").unwrap();
            assert!(x > 0.0 && x < size.0 && y > 0.0 && y < size.1);
            runtime.pointer_down(x, y);
            runtime.pointer_up(x, y);
            runtime.frame(0.0);
            assert_eq!(
                runtime
                    .state
                    .metrics
                    .as_ref()
                    .unwrap()
                    .snapshot
                    .as_ref()
                    .unwrap()
                    .series
                    .len(),
                1
            );
            let (x, y) = runtime.find("close-performance").unwrap();
            runtime.pointer_down(x, y);
            runtime.pointer_up(x, y);
            runtime.frame(0.0);
            assert!(runtime.state.metrics.is_none());
        }
    }
}
