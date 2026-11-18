//! Area `performance`: the Performance tab of a server or a VM, and the
//! recording settings of a server.
//!
//! Rune modules: `metrics_view.rs`, `metric_presets.rs`,
//! `host_monitoring_view.rs`, `host_monitoring_view_tests.rs`. Tab:
//! Performance (plan: "Charts, range, data sources, presets").
//!
//! # What other areas and the front end may call
//!
//! - [`chart`]`(m, key) -> Option<Chart>` and [`charts`]`(m)`: the data of
//!   the bespoke element `metrics-chart`, by the key of its block. The GPUI
//!   chart element reads nothing else of this area. [`Chart`], [`Plot`],
//!   [`CHART`], [`CHART_WIDTH`], [`CHART_HEIGHT`], [`GRID_LINES`] and
//!   [`NO_SAMPLES`] are its vocabulary. The seam is written out in
//!   `notes/progress/performance-seam.md`.
//! - [`commands::MONITORING`] (`server.monitoring-recording`): the way from
//!   a server to its recording settings, for a button of another area.
//! - [`close`]: drops everything the area holds.
//!
//! # Destinations
//!
//! | Rune | Here |
//! | --- | --- |
//! | "Performance" in the details pane (`open-performance`) | choosing the Performance tab of a server or VM starts the stream (hooks `on_tab_change`, `on_scope_change`); no command |
//! | "Back to inventory" (`close-performance`) | none: leaving the tab, or selecting another object, drops the view and with it the subscription |
//! | the heading "Performance · {name}" | none: the object header and the tab strip say it |
//! | the four range buttons `metrics-window-{seconds}` | one segmented choice `metrics-window`; its options keep those keys |
//! | "Data sources…" (`metrics-sources`) | command `performance.data-sources` on the tab's local row (a toggle: the picker replaces the charts, as in Rune) |
//! | "Reconnect metrics" (no key) | command `performance.reconnect` on the local row |
//! | "Graph presets…" (`metric-presets-open`) | command `performance.presets` on the local row (a toggle, as in Rune) |
//! | the source picker, the presets sub-view | blocks of the tab body, in place of the charts |
//! | a chart card (`rune::canvas`) | ONE `Custom` block `metrics-chart`, key `metrics-chart-<index>`; its data through [`chart`] |
//! | "Monitoring recording…" in the host administration chooser (`host-monitoring-open`) | command `server.monitoring-recording`: Server menu, local row of a server's Performance tab |
//! | the dialog "Host monitoring recording" | dialog `host-monitoring` (experimental notice), catalog then review |
//! | the catalog's source buttons `monitoring-source-{name}`, 12 a page, with "Previous sources" / "Next sources" | one table `monitoring-sources` with single selection; the row id is the source name. The page count left the count line |
//! | the dialog's heading `monitoring-heading` | the dialog's title |
//!
//! # Commands
//!
//! | Id | Label | Where |
//! | --- | --- | --- |
//! | `performance.data-sources` | Data Sources… (checked while picking) | Performance tab row |
//! | `performance.presets` | Graph Presets… (checked while shown) | Performance tab row |
//! | `performance.reconnect` | Reconnect Metrics | Performance tab row |
//! | `server.monitoring-recording` | Monitoring Recording… | Server menu (group 5); Performance tab row of a server |
//!
//! # Surfaces
//!
//! | Id | Kind | Experimental |
//! | --- | --- | --- |
//! | `host-monitoring` | dialog, large | yes |
//!
//! The Performance tab is supported and has no notice, as in Rune.
//!
//! # Scenes
//!
//! `performance`, `metric-presets`, `host-monitoring`,
//! `host-monitoring-catalog`.
//!
//! # The stream
//!
//! Metrics are polled by the core; the view holds the handle and the last
//! snapshot ([`metrics::MetricsUi`]). The view exists exactly while the
//! Performance tab of its object is shown: [`metrics::ensure`] opens it
//! (from the two hooks, and from `poll` for the case that a connection
//! became current while the tab was already shown), and leaving the tab or
//! the object drops it, which cancels the subscription. Nothing of it is an
//! operation: no record, no journal, no write to a host.
//!
//! # Files
//!
//! - [`metrics`]: the Rune `metrics_view.rs` without its drawing: the view
//!   state, the subscription, the range and the source picker, the
//!   synthetic preview, and the chart data.
//! - [`presets`]: the Rune `metric_presets.rs`: the stored catalog and
//!   saving, applying and removing a preset.
//! - [`monitoring`]: the Rune `host_monitoring_view.rs`: the catalog query,
//!   the review, the checks and the submission.
//! - [`commands`], [`view`], [`scenes`], `tests`.

use crate::prelude::*;

pub mod commands;
pub mod metrics;
pub mod monitoring;
/// The graph preset catalog `Preferences` stores, and what the Performance
/// tab does with it (the Rune `metric_presets.rs`).
pub mod presets;
pub mod scenes;
pub mod view;

#[cfg(test)]
mod tests;

pub use metrics::{
    CHART, CHART_HEIGHT, CHART_WIDTH, Chart, GRID_LINES, NO_SAMPLES, Plot, chart, charts,
};

/// The state of the area. `m.areas.performance`.
#[derive(Default)]
pub struct State {
    /// The view of the Performance tab that is shown. `None` while no
    /// Performance tab is shown, or while its connection is not current.
    pub metrics: Option<metrics::MetricsUi>,
    /// The open dialog "Host monitoring recording".
    pub monitoring: Option<monitoring::Panel>,
}

pub fn spec() -> AreaSpec {
    AreaSpec {
        commands: commands::commands,
        scenes: scenes::scenes,
        surfaces: view::surfaces,
        tab: view::tab,
        poll,
        on_scope_change: scope_changed,
        on_tab_change: tab_changed,
        on_session_change: close,
        ..AreaSpec::new("performance")
    }
}

/// Drops the view and the dialog. The session is about to change: both
/// belong to the connection they were opened on.
pub fn close(m: &mut Model) {
    m.areas.performance.metrics = None;
    m.areas.performance.monitoring = None;
}

/// Never waits: opens the view when its tab is shown and nothing is open
/// for it yet, takes the stream's latest snapshot, and collects the answer
/// of the catalog query.
fn poll(m: &mut Model, _clock: f64) -> bool {
    let mut changed = metrics::ensure(m);
    changed |= metrics::poll(m);
    changed |= monitoring::poll(m);
    changed
}

/// Another object is the scope. The dialog belonged to the server it was
/// opened for; the view follows the scope: the previous object's
/// subscription ends and, under a Performance tab, the new object's starts.
fn scope_changed(m: &mut Model) {
    m.areas.performance.monitoring = None;
    metrics::ensure(m);
}

/// The Rune view opened with its button and closed with "Back to
/// inventory": here the tab is both.
fn tab_changed(m: &mut Model, from: TabId, to: TabId) {
    if from == TabId::Performance {
        m.areas.performance.metrics = None;
    }
    if to == TabId::Performance {
        metrics::ensure(m);
    }
}
