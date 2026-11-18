//! The view of the Performance tab: the Rune `metrics_view.rs` without its
//! drawing.
//!
//! One [`MetricsUi`] is the tab of one object: its target, the options of
//! the subscription (range and data sources), the handle of the stream and
//! its last snapshot, and the drafts of the source picker and of the preset
//! name. In a synthetic pool there is no handle: the snapshot is the
//! generated preview ([`populate_preview`]).
//!
//! The second half of the file is the chart data ([`Chart`]): what the
//! bespoke element `metrics-chart` draws, computed from the snapshot with
//! the Rune arithmetic (`values`, `segments`, `number`, `timestamp`), word
//! for word.

use std::collections::BTreeSet;
use std::sync::Arc;

use xenadmin_core::{
    Error, MetricCategory, MetricSample, MetricSeries, MetricSource, MetricTarget, Metrics,
    MetricsOptions, MetricsSnapshot, MetricsState, MetricsWindow, PoolId,
};

use crate::prelude::*;

/// At most this many data sources in one subscription (the core's limit,
/// `MetricsOptions::validate`).
pub const MAX_SOURCES: usize = 32;

pub struct MetricsUi {
    pub pool: PoolId,
    pub target: MetricTarget,
    pub name: String,
    pub options: MetricsOptions,
    /// The subscription. Dropping it cancels it. `None` in a synthetic pool
    /// and after a subscription that was refused.
    pub handle: Option<Metrics>,
    pub snapshot: Option<Arc<MetricsSnapshot>>,
    pub choosing_sources: bool,
    pub selected_sources: BTreeSet<String>,
    pub source_filter: String,
    pub preset_name: String,
    pub show_presets: bool,
    pub notice: Option<String>,
    pub error: Option<String>,
}

/// The four ranges, in the order of the Rune buttons.
pub const WINDOWS: [MetricsWindow; 4] = [
    MetricsWindow::TenMinutes,
    MetricsWindow::TwoHours,
    MetricsWindow::OneWeek,
    MetricsWindow::OneYear,
];

/// What a tree object is measured as: a VM or a server of a pool. Nothing
/// else has a Performance tab.
pub fn target_of(object: &ObjectRef) -> Option<(PoolId, MetricTarget)> {
    let pool = object.pool?;
    let target = match object.kind {
        ObjectKind::Vm => MetricTarget::Vm(object.vm_id()?),
        ObjectKind::Host => MetricTarget::Host(object.host_id()?),
        _ => return None,
    };
    Some((pool, target))
}

/// The view, when it is the one of this pool and target.
pub fn view_of(m: &Model, pool: PoolId, target: MetricTarget) -> Option<&MetricsUi> {
    let view = m.areas.performance.metrics.as_ref();
    view.filter(|view| view.pool == pool && view.target == target)
}

/// What enabled the Rune "Performance" button: the demo, or a current
/// snapshot of the target's pool.
pub fn can_open(m: &Model, pool: PoolId) -> bool {
    m.is_synthetic_pool(pool) || m.snapshot_for(pool).is_some_and(|s| s.is_current())
}

/// Makes the view be the one of the scope object while its Performance tab
/// is shown: drops a view of another object, and opens one when there is
/// none and the Rune button would have been enabled. A view that is open
/// for the scope is left alone, also when its subscription failed: its
/// error line and `performance.reconnect` are the way on. Returns whether
/// anything changed.
pub fn ensure(m: &mut Model) -> bool {
    let wanted = (m.active_tab() == TabId::Performance)
        .then(|| m.scope().single().and_then(target_of))
        .flatten();
    let held = m.areas.performance.metrics.as_ref();
    let held = held.map(|view| (view.pool, view.target));
    if held.is_some() && held == wanted {
        return false;
    }
    m.areas.performance.metrics = None;
    let opened = wanted.is_some() && open(m);
    held.is_some() || opened
}

/// The Rune `open`: a fresh view of the scope object with the default
/// options, then the preview or the subscription.
pub fn open(m: &mut Model) -> bool {
    let Some(object) = m.scope().single().cloned() else {
        return false;
    };
    let Some((pool, target)) = target_of(&object) else {
        return false;
    };
    if m.inventory_for(pool).is_none() || !can_open(m, pool) {
        return false;
    }
    let name = m.name_of(&object).unwrap_or_default();
    m.areas.performance.metrics = Some(MetricsUi {
        pool,
        target,
        name,
        options: MetricsOptions::default(),
        handle: None,
        snapshot: None,
        choosing_sources: false,
        selected_sources: BTreeSet::new(),
        source_filter: String::new(),
        preset_name: String::new(),
        show_presets: false,
        notice: None,
        error: None,
    });
    restart(m);
    true
}

/// After the options changed: the preview again, or a new subscription.
pub fn restart(m: &mut Model) {
    let Some(view) = &m.areas.performance.metrics else {
        return;
    };
    if m.is_synthetic_pool(view.pool) {
        populate_preview(m);
    } else {
        reload(m);
    }
}

/// Ends the subscription and starts one with the view's options. A refusal
/// is the view's error line; nothing is retried.
pub fn reload(m: &mut Model) {
    let Some(view) = &mut m.areas.performance.metrics else {
        return;
    };
    view.handle = None;
    view.snapshot = None;
    let (pool, target, options) = (view.pool, view.target, view.options.clone());
    let result = (|| {
        let snapshot = m
            .snapshot_for(pool)
            .filter(|snapshot| {
                snapshot
                    .inventory()
                    .is_some_and(|inventory| inventory.pool_id == pool)
            })
            .ok_or(Error::Stale)?;
        let observed = snapshot.observe_metrics(target)?;
        m.connection_for(pool)
            .ok_or(Error::Closed)?
            .subscribe_metrics(observed, options)
    })();
    let waker = m.waker.clone();
    let Some(view) = &mut m.areas.performance.metrics else {
        return;
    };
    match result {
        Ok(handle) => {
            if let Some(waker) = waker {
                handle.notify_on_change(waker);
            }
            view.snapshot = Some(handle.latest());
            view.handle = Some(handle);
            view.error = None;
        }
        Err(error) => view.error = Some(error.to_string()),
    }
}

/// Takes the stream's latest snapshot. True when it is another one.
pub fn poll(m: &mut Model) -> bool {
    let Some(view) = &mut m.areas.performance.metrics else {
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

pub fn window(m: &mut Model, window: MetricsWindow) {
    if let Some(view) = &mut m.areas.performance.metrics {
        view.options.window = window;
    }
    restart(m);
}

/// Opens the source picker on what is shown now: the chosen sources, or,
/// with the host's common set, the sources of the series on screen.
pub fn choose_sources(m: &mut Model) {
    let Some(view) = &mut m.areas.performance.metrics else {
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

/// The picker's Cancel.
pub fn cancel_sources(m: &mut Model) {
    if let Some(view) = &mut m.areas.performance.metrics {
        view.choosing_sources = false;
    }
}

/// A checkbox of the picker: out when it was in, in while there is room.
pub fn toggle_source(m: &mut Model, name: &str) {
    if let Some(view) = &mut m.areas.performance.metrics
        && !view.selected_sources.remove(name)
        && view.selected_sources.len() < MAX_SOURCES
    {
        view.selected_sources.insert(name.to_owned());
    }
}

pub fn apply_sources(m: &mut Model, defaults: bool) {
    let Some(view) = &mut m.areas.performance.metrics else {
        return;
    };
    view.options.sources = if defaults {
        Vec::new()
    } else {
        view.selected_sources.iter().cloned().collect()
    };
    view.choosing_sources = false;
    restart(m);
}

/// The heading over the charts.
pub fn state_label(state: MetricsState) -> &'static str {
    match state {
        MetricsState::Loading => "Loading performance data…",
        MetricsState::Ready => "Live performance data",
        MetricsState::Stale => "Performance data is stale",
        MetricsState::Failed => "Performance data unavailable",
        MetricsState::Closed => "Performance subscription closed",
    }
}

/// The line under that heading.
pub fn range_line(snapshot: &MetricsSnapshot) -> String {
    format!(
        "Available range: {} — {} · {} second samples · averages",
        timestamp(snapshot.range_start),
        timestamp(snapshot.range_end),
        snapshot
            .sample_step_seconds
            .map(|step| step.to_string())
            .unwrap_or_else(|| "unknown".into())
    )
}

// -- The chart data -----------------------------------------------------------

/// The name of the bespoke element that draws one chart.
pub const CHART: &str = "metrics-chart";
/// The box the points of [`Plot::segments`] are in: x from 0 to
/// `CHART_WIDTH`, y from 0 (the high label) down to `CHART_HEIGHT` (the low
/// label).
pub const CHART_WIDTH: f32 = 1000.0;
pub const CHART_HEIGHT: f32 = 120.0;
/// The y of the three horizontal grid lines, in the same box.
pub const GRID_LINES: [f32; 3] = [0.0, 60.0, 120.0];
/// What a chart says in place of its plot when no sample of its series is
/// a finite number.
pub const NO_SAMPLES: &str = "No finite samples in this range";

/// One chart: one series of the snapshot. Never several series in one box.
#[derive(Clone, Debug, PartialEq)]
pub struct Chart {
    /// The key of the chart's block: `metrics-chart-<index>`.
    pub key: String,
    /// The position of the series in the snapshot.
    pub index: usize,
    /// The data source's name; the card's heading.
    pub title: String,
    /// The data source's description; empty when the host gave none.
    pub description: String,
    /// "Latest: {value} {unit} · {n} valid, {n} missing samples".
    pub summary: String,
    /// The accessible name of the plot: "{title} chart. {summary}".
    pub label: String,
    /// `None`: no finite sample; the card shows [`NO_SAMPLES`] and no plot.
    pub plot: Option<Plot>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Plot {
    /// The label of the top edge: the largest value, never below zero.
    pub high: String,
    /// The label of the bottom edge: the smallest value, never above zero.
    pub low: String,
    /// The time of the first and of the last sample ("YYYY-MM-DD HH:MM:SS
    /// UTC"), for the left and the right end of the x axis.
    pub first: String,
    pub last: String,
    /// The line, cut wherever a sample is missing or two samples are more
    /// than one and a half steps apart. Each inner vector is one unbroken
    /// run of `[x, y]` points; nothing joins two runs. A run of one point
    /// is a sample with a gap on both sides.
    pub segments: Vec<Vec<[f32; 2]>>,
}

/// The key of the block of the chart at `index`.
pub fn chart_key(index: usize) -> String {
    format!("{CHART}-{index}")
}

/// The charts of the view that is open, in the order of the snapshot's
/// series. Empty without a view or a snapshot.
pub fn charts(m: &Model) -> Vec<Chart> {
    let view = m.areas.performance.metrics.as_ref();
    let Some(snapshot) = view.and_then(|view| view.snapshot.as_ref()) else {
        return Vec::new();
    };
    (0..snapshot.series.len())
        .filter_map(|index| describe(snapshot, index))
        .collect()
}

/// The chart of the block with this key, as the view is now. `None` when
/// the key is not a chart's or the series is gone: the element then draws
/// nothing.
pub fn chart(m: &Model, key: &str) -> Option<Chart> {
    let index = key.strip_prefix(CHART)?.strip_prefix('-')?.parse().ok()?;
    let view = m.areas.performance.metrics.as_ref()?;
    describe(view.snapshot.as_ref()?, index)
}

fn describe(snapshot: &MetricsSnapshot, index: usize) -> Option<Chart> {
    let series = snapshot.series.get(index)?;
    let source = snapshot
        .sources
        .iter()
        .find(|source| source.name == series.source);
    Some(build(index, series, source, snapshot.sample_step_seconds))
}

/// The Rune `chart` up to the point where it drew.
fn build(
    index: usize,
    series: &MetricSeries,
    source: Option<&MetricSource>,
    step: Option<u64>,
) -> Chart {
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
    let plot = (!finite.is_empty()).then(|| Plot {
        high: format!("{} {}", number(high), unit(units)),
        low: format!("{} {}", number(low), unit(units)),
        first: timestamp(series.samples.first().map(|sample| sample.timestamp)),
        last: timestamp(series.samples.last().map(|sample| sample.timestamp)),
        segments: segments(&series.samples, &values, step, low, high),
    });
    Chart {
        key: chart_key(index),
        index,
        title: series.source.clone(),
        description: source.map_or(String::new(), |source| source.description.clone()),
        label: format!("{} chart. {summary}", series.source),
        summary,
        plot,
    }
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
pub(super) fn timestamp(value: Option<u64>) -> String {
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

/// The synthetic snapshot of a demo pool: four sources, 120 samples at the
/// range's interval, seven of them missing so that every chart has a gap.
pub fn populate_preview(m: &mut Model) {
    let Some(view) = &mut m.areas.performance.metrics else {
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

#[cfg(test)]
mod tests {
    use super::*;

    // Rune: metrics_view.rs, the same name, unchanged.
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

    // Rune: metrics_view.rs, the same name, unchanged.
    #[test]
    fn timestamps_have_explicit_utc_calendar_dates_for_long_ranges() {
        assert_eq!(timestamp(Some(0)), "1970-01-01 00:00:00 UTC");
        assert_eq!(timestamp(Some(951_782_400)), "2000-02-29 00:00:00 UTC");
        assert_eq!(timestamp(Some(253_402_300_799)), "9999-12-31 23:59:59 UTC");
        assert_eq!(timestamp(Some(u64::MAX)), "unknown");
        assert_eq!(timestamp(None), "unknown");
    }

    /// The chart of a series is the Rune card's numbers: the axis always
    /// holds zero, a fraction is a percentage, and a series without a
    /// finite sample has a summary and no plot.
    #[test]
    fn a_chart_has_the_cards_labels_and_no_plot_without_finite_samples() {
        let sample = |timestamp, value| MetricSample { timestamp, value };
        let series = MetricSeries {
            source: "cpu0".into(),
            samples: vec![
                sample(0, Some(0.25)),
                sample(5, None),
                sample(10, Some(0.5)),
            ],
        };
        let source = MetricSource {
            name: "cpu0".into(),
            description: "CPU 0 utilization".into(),
            units: "(fraction)".into(),
            enabled: true,
            category: MetricCategory::Cpu,
        };
        let chart = build(3, &series, Some(&source), Some(5));
        assert_eq!(chart.key, "metrics-chart-3");
        assert_eq!(chart.title, "cpu0");
        assert_eq!(chart.description, "CPU 0 utilization");
        assert_eq!(
            chart.summary,
            "Latest: 50.00 % · 2 valid, 1 missing samples"
        );
        assert_eq!(chart.label, format!("cpu0 chart. {}", chart.summary));
        let plot = chart.plot.expect("a plot");
        assert_eq!((plot.high.as_str(), plot.low.as_str()), ("50.00 %", "0 %"));
        assert_eq!(plot.first, "1970-01-01 00:00:00 UTC");
        assert_eq!(plot.last, "1970-01-01 00:00:10 UTC");
        // The missing sample cuts the line: two runs of one point each.
        assert_eq!(plot.segments, vec![vec![[0.0, 60.0]], vec![[1000.0, 0.0]]]);

        let empty = MetricSeries {
            source: "gone".into(),
            samples: vec![sample(0, None), sample(5, Some(f64::NAN))],
        };
        let chart = build(0, &empty, None, Some(5));
        assert_eq!(
            chart.summary,
            "Latest: Unavailable unknown · 0 valid, 2 missing samples"
        );
        assert_eq!(chart.description, "");
        assert!(chart.plot.is_none());
    }
}
