//! The tests of the area, in three parts.
//!
//! 1. **The Rune tests** that built a `rune::Runtime`, under names that say
//!    which they were, driven through the harness; and the ones that called
//!    the logic directly, moved. (The two pure tests of `metrics_view.rs`
//!    are in `metrics.rs`, the catalog test of `metric_presets.rs` in
//!    `presets.rs`, both unchanged.)
//! 2. **What the port changed**: the view lives and dies with its tab, the
//!    chart is data behind a key, the control row is commands, the catalog
//!    is a table.
//! 3. **Against a live session** (`crate::testing::Live`): a subscription
//!    is started for the tab and ended with it.

use xenadmin_core::{
    Error, HostOperation, HostRecordingAction, MetricTarget, MetricsOptions, MetricsWindow, PoolId,
};

use super::commands::{
    DATA_SOURCES, MONITORING, NO_SUBSCRIPTION, NOT_CURRENT, PRESETS, RECONNECT,
};
use super::metrics::MetricsUi;
use super::monitoring::Panel;
use super::presets::{Catalog, Kind, Preset};
use super::{metrics, monitoring, presets, scenes, view};
use crate::Harness;
use crate::prelude::*;
use crate::surface::EXPERIMENTAL_NOTICE_KEY;
use crate::testing::Live;

// -- Fixtures -------------------------------------------------------------------

fn shown(h: &Harness) -> &MetricsUi {
    h.model.areas.performance.metrics.as_ref().expect("a view")
}

fn shown_mut(h: &mut Harness) -> &mut MetricsUi {
    h.model.areas.performance.metrics.as_mut().expect("a view")
}

fn panel(h: &Harness) -> &Panel {
    let panel = h.model.areas.performance.monitoring.as_ref();
    panel.expect("the monitoring dialog")
}

fn pool(h: &Harness) -> PoolId {
    h.model.inventory.as_ref().expect("inventory").pool_id
}

/// The nth server of the demo pool, as a tree object.
fn host(h: &Harness, n: usize) -> ObjectRef {
    let inventory = h.model.inventory.as_ref().expect("inventory");
    let id = *inventory.hosts.keys().nth(n).expect("a server");
    ObjectRef::host(inventory.pool_id, id)
}

/// The ids of this area's commands on the local row, in its order.
fn row(h: &Harness) -> Vec<&'static str> {
    h.local_row()
        .into_iter()
        .map(|item| item.id.0)
        .filter(|id| [DATA_SOURCES, PRESETS, RECONNECT, MONITORING].contains(&CommandId(id)))
        .collect()
}

// -- 1. The Rune tests -----------------------------------------------------------

// Rune: metrics_view.rs, the same name. The sizes are the GPUI half.
#[test]
fn performance_controls_show_actual_coverage_and_change_ranges_without_host_writes() {
    let mut h = Harness::scene("performance");
    assert_eq!(h.model.active_tab(), TabId::Performance);
    h.click("metrics-window-604800");
    assert_eq!(shown(&h).options.window, MetricsWindow::OneWeek);
    assert!(h.has_text("3600 second samples"));
    assert!(h.has_text("Synthetic performance preview"));
    assert!(shown(&h).handle.is_none());
    // "Data sources…" is the command of the tab's row now.
    h.click(DATA_SOURCES.0);
    assert!(shown(&h).choosing_sources);
    // Rune wrote the selection; here the user unchecks the other three.
    for name in ["memory", "vbd_xvda_read", "vif_0_rx"] {
        h.click(&format!("metrics-source-{name}"));
    }
    assert_eq!(
        shown(&h).selected_sources.iter().collect::<Vec<_>>(),
        ["cpu0"]
    );
    h.click("apply-metric-sources");
    assert_eq!(shown(&h).snapshot.as_ref().unwrap().series.len(), 1);
    assert!(h.model.resource_operations.is_empty());
    // "Back to inventory" is leaving the tab.
    h.open_tab(TabId::General);
    assert!(h.model.areas.performance.metrics.is_none());
}

// Rune: metric_presets.rs, `save_apply_and_remove_presets_preserve_the_target_at_both_sizes`.
#[test]
fn save_apply_and_remove_presets_preserve_the_target() {
    let mut h = Harness::scene("performance");
    let target = shown(&h).target;
    shown_mut(&mut h).options = MetricsOptions {
        window: MetricsWindow::OneWeek,
        sources: vec!["cpu0".into()],
    };
    h.click(PRESETS.0);
    h.type_text("metric-preset-name", "CPU history");
    h.click("metric-preset-save");
    assert_eq!(
        h.model.preferences.metric_presets.presets.len(),
        1,
        "error {:?}; name {}",
        shown(&h).error,
        shown(&h).preset_name
    );
    assert_eq!(
        h.node("metrics-notice").text.as_deref(),
        Some(presets::SAVED)
    );
    shown_mut(&mut h).options = MetricsOptions::default();
    h.click("metric-preset-CPU history");
    let view = shown(&h);
    assert_eq!(view.target, target);
    assert_eq!(view.options.window, MetricsWindow::OneWeek);
    assert_eq!(view.options.sources, vec!["cpu0"]);
    assert!(view.handle.is_none());
    // Applying leaves the presets for the charts, with its notice.
    assert!(!view.show_presets);
    assert_eq!(
        h.node("metrics-notice").text.as_deref(),
        Some(presets::APPLIED)
    );
    h.click(PRESETS.0);
    h.click("metric-preset-remove-CPU history");
    assert!(h.model.preferences.metric_presets.presets.is_empty());
}

// Rune: metric_presets.rs, the same name, unchanged.
#[test]
fn missing_sources_and_other_target_types_never_silently_change_the_graph() {
    let mut h = Harness::scene("performance");
    let m = &mut h.model;
    let before = m.areas.performance.metrics.as_ref().unwrap().options.clone();
    m.preferences
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
    presets::apply(m, "Missing");
    assert_eq!(m.areas.performance.metrics.as_ref().unwrap().options, before);
    let error = m.areas.performance.metrics.as_ref().unwrap().error.clone();
    assert!(error.unwrap().contains("not_advertised"));
    m.preferences
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
    presets::apply(m, "Host");
    assert_eq!(m.areas.performance.metrics.as_ref().unwrap().options, before);
    let encoded = serde_json::to_vec(&m.preferences.metric_presets).unwrap();
    let decoded: Catalog = serde_json::from_slice(&encoded).unwrap();
    assert!(decoded.validate().is_ok());
    // The refusal is on screen, and the server's preset is not offered to a VM.
    assert!(h.node("metrics-error").text.unwrap().contains("not_advertised"));
    h.click(PRESETS.0);
    assert!(h.find("metric-preset-Missing").is_some());
    assert!(h.find("metric-preset-Host").is_none());
}

// Rune: host_monitoring_view_tests.rs,
// `destructive_review_keeps_captured_source_and_reachable_controls_at_both_widths`.
// The widths are the GPUI half.
#[test]
fn destructive_review_keeps_captured_source_and_reachable_controls() {
    let mut h = Harness::scene("host-monitoring");
    let original = panel(&h).review.clone();
    assert!(matches!(
        original,
        Some(HostOperation::DataSource {
            action: HostRecordingAction::ForgetHistory,
            ..
        })
    ));
    let target = panel(&h).target;
    // `monitoring-heading` is the dialog's title.
    assert_eq!(h.surface().title, "Host monitoring recording");
    for key in ["monitoring-close", "monitoring-submit", "monitoring-history"] {
        assert!(h.find(key).is_some(), "{key}");
    }
    assert_eq!(
        h.node("monitoring-submit").label.as_deref(),
        Some("Forget recorded history")
    );
    h.click("monitoring-confirm-history-loss");
    assert!(panel(&h).acknowledged);
    // A selection elsewhere cannot retarget an already captured destructive review.
    let other = host(&h, 1);
    h.model.select_in_grid(Target::one(other));
    assert_eq!(
        h.try_click("monitoring-submit"),
        Err(monitoring::DEMO.into())
    );
    // The handler makes the checks again, whatever the button said.
    monitoring::submit(&mut h.model);
    assert_eq!(panel(&h).target, target);
    assert_eq!(panel(&h).review, original);
    assert_eq!(panel(&h).error, Some(Error::Stale.to_string()));
    assert!(
        h.model.resource_operations.is_empty(),
        "synthetic review must never write"
    );
    h.click("monitoring-back");
    assert!(panel(&h).review.is_none());
    assert!(!panel(&h).acknowledged);
}

// Rune: host_monitoring_view_tests.rs, the same name, unchanged.
#[test]
fn refresh_discards_catalog_and_review_and_enabled_source_cannot_start_again() {
    let mut h = Harness::scene("host-monitoring");
    let m = &mut h.model;
    m.areas.performance.monitoring.as_mut().unwrap().review = None;
    monitoring::review(m, HostRecordingAction::StartRecording);
    assert!(m.areas.performance.monitoring.as_ref().unwrap().review.is_none());
    let p = m.areas.performance.monitoring.as_mut().unwrap();
    p.sources.as_mut().unwrap()[0].enabled = false;
    monitoring::review(m, HostRecordingAction::StartRecording);
    assert!(matches!(
        m.areas.performance.monitoring.as_ref().unwrap().review,
        Some(HostOperation::DataSource {
            action: HostRecordingAction::StartRecording,
            ..
        })
    ));
    monitoring::refresh(m);
    let p = m.areas.performance.monitoring.as_ref().unwrap();
    assert!(p.sources.is_none() && p.selected.is_none() && p.review.is_none() && !p.acknowledged);
}

// Rune: host_monitoring_view_tests.rs, the same name. "Activity" is the
// Events tab of the Notifications view, its History list filtered to the
// pool the panel was captured for.
#[test]
fn history_uses_captured_pool_after_switching_the_visible_inventory() {
    let mut h = Harness::scene("host-monitoring");
    let captured = panel(&h).pool;
    let mut inventory = h.model.inventory.take().unwrap().as_ref().clone();
    inventory.pool_id = uuid::Uuid::new_v4().into();
    h.model.inventory = Some(std::sync::Arc::new(inventory));
    // Still open: nothing has reconciled the selection yet.
    assert!(h.model.areas.performance.monitoring.is_some());
    h.click("monitoring-history");
    assert!(h.model.areas.performance.monitoring.is_none());
    assert_eq!(h.model.active_tab(), TabId::Events);
    assert_eq!(h.model.areas.events.pool, Some(captured));
}

// Rune: main.rs, `experimental_dialog_open` lists the monitoring dialog;
// the Performance view is supported and shows no notice.
#[test]
fn the_monitoring_dialog_is_experimental_and_the_performance_tab_is_not() {
    for scene in ["host-monitoring", "host-monitoring-catalog"] {
        let h = Harness::scene(scene);
        assert_eq!(h.surface().id, view::MONITORING);
        assert!(h.surface().experimental, "{scene}");
        assert!(h.find(EXPERIMENTAL_NOTICE_KEY).is_some(), "{scene}");
        h.assert_unique_keys();
    }
    for scene in ["performance", "metric-presets"] {
        let h = Harness::scene(scene);
        assert!(h.surfaces().is_empty(), "{scene}");
        assert!(h.find(EXPERIMENTAL_NOTICE_KEY).is_none(), "{scene}");
        h.assert_unique_keys();
    }
}

// Rune: session_permissions_view.rs, the footer of a review. The line
// belongs to the review: the catalog has no operation to assess.
#[test]
fn the_permission_line_is_under_the_review_only() {
    assert!(Harness::scene("host-monitoring").surface().footer_note.is_some());
    let catalog = Harness::scene("host-monitoring-catalog").surface();
    assert!(catalog.footer_note.is_none());
}

// -- 2. What the port changed -----------------------------------------------------

/// The scenes are the Rune ones: the guest and its four charts; the saved
/// preset with the presets shown; the review; the catalog of two.
#[test]
fn the_scenes_show_what_the_rune_scenes_showed() {
    let h = Harness::scene("performance");
    let view = shown(&h);
    assert_eq!(view.target.uuid().to_string(), scenes::GUEST);
    assert!(!view.name.is_empty());
    assert_eq!(view.snapshot.as_ref().unwrap().series.len(), 4);
    assert_eq!(metrics::charts(&h.model).len(), 4);
    assert!(h.has_text("5 second samples"));
    assert!(h.has_text("Missing samples remain gaps."));

    let h = Harness::scene("metric-presets");
    assert!(shown(&h).show_presets);
    assert_eq!(shown(&h).options.sources, ["cpu0", "memory"]);
    assert!(h.has_text("Saved graph presets"));
    assert!(h.has_text("10 minutes · 2 sources"));
    assert_eq!(
        h.node("metric-preset-CPU and memory").label.as_deref(),
        Some("CPU and memory")
    );
    assert!(h.find("metrics-chart-0").is_none());

    let h = Harness::scene("host-monitoring");
    assert!(h.has_text("Processor 7 utilization"));
    assert!(h.has_text("It is not a pause and cannot be undone here."));
    assert!(h.has_text("A lost reply remains unknown and is never automatically repeated."));
    assert!(h.has_text(monitoring::PREVIEW));

    let h = Harness::scene("host-monitoring-catalog");
    assert_eq!(panel(&h).selected.as_deref(), Some("memory_free_kib"));
    assert_eq!(
        h.node("monitoring-count").text.as_deref(),
        Some("2 sources")
    );
    assert!(h.has_text("Units: KiB · range: 0 to Infinity"));
    assert!(h.has_text("Recording at catalog refresh: disabled · standard source: no"));
    assert!(h.has_text("Stock XCP-ng 8.3 does not ship Nagios NRPE integration."));
}

/// The view exists exactly while the Performance tab of its object is
/// shown, and it follows the scope: Rune opened it with a button and
/// closed it on every navigation.
#[test]
fn the_view_lives_with_its_tab_and_follows_the_scope() {
    let mut h = Harness::scene("performance");
    let first = shown(&h).target;
    assert!(matches!(first, MetricTarget::Vm(_)));
    // The scene's poll changes nothing: the view is there already.
    assert!(!super::poll(&mut h.model, 0.0));
    shown_mut(&mut h).options.window = MetricsWindow::OneYear;

    // Another tab: the view is dropped; back: a fresh one with the defaults.
    h.open_tab(TabId::General);
    assert!(h.model.areas.performance.metrics.is_none());
    h.open_tab(TabId::Performance);
    assert_eq!(shown(&h).target, first);
    assert_eq!(shown(&h).options, MetricsOptions::default());

    // A server under the same tab: the view is the server's.
    let server = host(&h, 0);
    h.select(Target::one(server.clone()));
    assert_eq!(h.model.active_tab(), TabId::Performance);
    assert_eq!(
        shown(&h).target,
        MetricTarget::Host(server.host_id().unwrap())
    );
    assert_eq!(shown(&h).name, h.model.name_of(&server).unwrap());

    // The pool has no Performance tab: nothing is held.
    let pool = pool(&h);
    h.select(Target::one(ObjectRef::pool(pool)));
    assert!(h.model.areas.performance.metrics.is_none());
    assert!(!super::poll(&mut h.model, 0.0));
}

/// Without a current connection there is no view: the tab says so, the
/// commands of the view are disabled with the reason, and nothing opens.
#[test]
fn a_tab_whose_connection_is_not_current_waits_and_says_why() {
    let mut h = Harness::scene("performance");
    h.open_tab(TabId::General);
    h.model.demo = false;
    h.open_tab(TabId::Performance);
    assert!(h.model.areas.performance.metrics.is_none());
    assert!(!super::poll(&mut h.model, 0.0));
    assert_eq!(
        h.node("metrics-waiting").text.as_deref(),
        Some("Waiting for performance data.")
    );
    assert_eq!(
        h.node("metrics-not-current").text.as_deref(),
        Some(NOT_CURRENT)
    );
    assert!(h.find("metrics-window").is_none());
    for id in [DATA_SOURCES, PRESETS, RECONNECT] {
        assert_eq!(h.eligibility(id.0), Eligibility::disabled(NOT_CURRENT));
    }
    // The connection is back (here: the demo again): the next poll opens it.
    h.model.demo = true;
    assert!(super::poll(&mut h.model, 0.0));
    assert!(h.find("metrics-window").is_some());
}

/// The seam of the chart element: one `Custom` block per series, named
/// `metrics-chart`, keyed by index, and its data by that key.
#[test]
fn each_chart_is_one_custom_block_and_its_data_is_reached_by_its_key() {
    let h = Harness::scene("performance");
    let titles = ["cpu0", "memory", "vbd_xvda_read", "vif_0_rx"];
    for (index, title) in titles.into_iter().enumerate() {
        let key = format!("metrics-chart-{index}");
        let node = h.node(&key);
        assert_eq!(node.label.as_deref(), Some(metrics::CHART));
        let params: serde_json::Value =
            serde_json::from_str(node.value.as_deref().unwrap()).unwrap();
        assert_eq!(params["index"], index);
        assert_eq!(params["source"], title);
        let chart = super::chart(&h.model, &key).expect("the chart of the key");
        assert_eq!((chart.key.as_str(), chart.index), (key.as_str(), index));
        assert_eq!(chart.title, title);
        assert_eq!(params["label"], chart.label.as_str());
        assert!(chart.summary.ends_with("113 valid, 7 missing samples"));
        // The seven missing samples are a gap: two runs, never one line
        // through zero.
        let plot = chart.plot.expect("a plot");
        let runs: Vec<_> = plot.segments.iter().map(Vec::len).collect();
        assert_eq!(runs, [45, 68], "{title}");
        let gap = plot.segments[1][0][0] - plot.segments[0][44][0];
        assert!(gap > 1000.0 * 7.5 / 119.0, "{title}: {gap}");
        for [x, y] in plot.segments.iter().flatten() {
            assert!((0.0..=super::CHART_WIDTH).contains(x), "{title}: x {x}");
            assert!((0.0..=super::CHART_HEIGHT).contains(y), "{title}: y {y}");
        }
        assert_eq!(plot.first, "2026-11-07 23:50:05 UTC");
        assert_eq!(plot.last, "2026-11-08 00:00:00 UTC");
    }
    let cpu = super::chart(&h.model, "metrics-chart-0").unwrap();
    assert_eq!(cpu.description, "CPU 0 utilization");
    assert_eq!(cpu.plot.unwrap().low, "0 %");
    let memory = super::chart(&h.model, "metrics-chart-1").unwrap();
    assert_eq!(memory.plot.unwrap().high, "4.29G bytes");
    assert_eq!(super::charts(&h.model).len(), 4);
    for key in ["metrics-chart-4", "metrics-chart", "metrics-chart-x", "other-0"] {
        assert!(super::chart(&h.model, key).is_none(), "{key}");
    }
    // Without a view there is no chart.
    let mut h = h;
    h.open_tab(TabId::General);
    assert!(super::chart(&h.model, "metrics-chart-0").is_none());
    assert!(super::charts(&h.model).is_empty());
}

/// The control row is commands: three for a VM, a fourth for a server.
#[test]
fn the_local_row_has_the_views_commands_and_monitoring_for_a_server() {
    let mut h = Harness::scene("performance");
    assert_eq!(row(&h), [DATA_SOURCES.0, PRESETS.0, RECONNECT.0]);
    assert_eq!(h.eligibility(MONITORING.0), Eligibility::Hidden);
    assert_eq!(
        h.eligibility(RECONNECT.0),
        Eligibility::disabled(NO_SUBSCRIPTION)
    );
    // Both openers are toggles and show it; one replaces the other.
    let checked = |h: &Harness, id: CommandId| {
        let item = h.local_row().into_iter().find(|item| item.id == id);
        item.and_then(|item| item.checked)
    };
    assert_eq!(checked(&h, DATA_SOURCES), Some(false));
    h.click(DATA_SOURCES.0);
    assert_eq!(checked(&h, DATA_SOURCES), Some(true));
    assert!(h.has_text("Data sources · 4 of 32 selected"));
    h.click(PRESETS.0);
    assert_eq!(checked(&h, DATA_SOURCES), Some(false));
    assert_eq!(checked(&h, PRESETS), Some(true));
    h.click("metric-presets-close");
    assert_eq!(checked(&h, PRESETS), Some(false));
    assert!(h.find("metrics-chart-0").is_some());
    h.click(DATA_SOURCES.0);
    h.click(DATA_SOURCES.0);
    assert!(!shown(&h).choosing_sources);

    let server = host(&h, 0);
    h.select(Target::one(server));
    assert_eq!(
        row(&h),
        [DATA_SOURCES.0, PRESETS.0, RECONNECT.0, MONITORING.0]
    );
    let item = h.menu_item(MONITORING.0).expect("in the Server menu");
    assert_eq!(item.label, "Monitoring Recording…");
    assert_eq!(item.disabled, None);
    h.click(MONITORING.0);
    assert_eq!(h.surface().id, view::MONITORING);
    assert!(h.surface().subtitle.unwrap().contains(&panel(&h).name));
}

/// The picker: the filter narrows the list, Cancel keeps the graph, and
/// "Common sources" goes back to the host's own set.
#[test]
fn the_source_picker_filters_cancels_and_returns_to_the_common_sources() {
    let mut h = Harness::scene("performance");
    h.click(DATA_SOURCES.0);
    assert_eq!(
        h.node("metrics-source-cpu0").label.as_deref(),
        Some("cpu0 · (fraction)")
    );
    h.type_text("metrics-source-filter", "THROUGHPUT");
    assert!(h.find("metrics-source-cpu0").is_none());
    assert!(h.find("metrics-source-vif_0_rx").is_some());
    h.type_text("metrics-source-filter", "");
    for name in ["cpu0", "memory", "vbd_xvda_read", "vif_0_rx"] {
        h.click(&format!("metrics-source-{name}"));
    }
    assert!(h.has_text("Data sources · 0 of 32 selected"));
    assert!(h.try_click("apply-metric-sources").is_err());
    h.click("metrics-sources-cancel");
    assert!(shown(&h).options.sources.is_empty());
    assert_eq!(shown(&h).snapshot.as_ref().unwrap().series.len(), 4);

    h.click(DATA_SOURCES.0);
    h.click("metrics-source-memory");
    h.click("apply-metric-sources");
    assert_eq!(shown(&h).options.sources.len(), 3);
    assert_eq!(metrics::charts(&h.model).len(), 3);
    h.click(DATA_SOURCES.0);
    h.click("metrics-sources-common");
    assert!(shown(&h).options.sources.is_empty());
    assert_eq!(metrics::charts(&h.model).len(), 4);
}

/// The catalog is a table: a row is a source, the search narrows it, and
/// the two reviews start from the selected row.
#[test]
fn the_catalog_is_a_table_and_reviews_start_from_its_selected_row() {
    let mut h = Harness::scene("host-monitoring-catalog");
    assert!(h.find("monitoring-sources").is_some());
    // A source that records cannot be started again, and says so.
    h.select_row("monitoring-sources", "cpu7");
    assert_eq!(panel(&h).selected.as_deref(), Some("cpu7"));
    assert_eq!(
        h.try_click("monitoring-review-start"),
        Err(monitoring::ALREADY.into())
    );
    h.select_row("monitoring-sources", "memory_free_kib");
    h.click("monitoring-review-start");
    assert!(matches!(
        &panel(&h).review,
        Some(HostOperation::DataSource { expected, action: HostRecordingAction::StartRecording })
            if expected.name == "memory_free_kib"
    ));
    assert_eq!(
        h.node("monitoring-submit").label.as_deref(),
        Some("Start recording")
    );
    assert!(h.has_text("Earlier missing samples are not recovered."));
    assert!(h.find("monitoring-confirm-history-loss").is_none());
    h.click("monitoring-back");

    // Deleting history needs its acknowledgement besides everything else.
    h.click("monitoring-review-forget");
    h.model.demo = false;
    let reason = monitoring::blocked(&h.model, panel(&h));
    assert_eq!(reason.as_deref(), Some(monitoring::CHANGED));
    assert!(h.find("monitoring-review-stale").is_some());
    h.model.demo = true;
    h.click("monitoring-back");

    h.type_text("monitoring-search", "FREE");
    assert_eq!(h.node("monitoring-count").text.as_deref(), Some("1 sources"));
    // The selection is kept while its row is filtered out and back.
    h.type_text("monitoring-search", "processor");
    assert_eq!(panel(&h).selected.as_deref(), Some("memory_free_kib"));
    assert!(h.has_text("Host free memory"));

    // In the demo there is no catalog to read again.
    h.click("monitoring-refresh");
    assert!(panel(&h).sources.is_none() && panel(&h).query.is_none());
    assert!(h.find("monitoring-sources").is_none());
}

/// The dialog belongs to its server: another scope closes it, and Escape
/// closes it.
#[test]
fn the_monitoring_dialog_closes_with_its_scope_and_on_escape() {
    let mut h = Harness::scene("host-monitoring");
    let other = host(&h, 1);
    h.model.select(Target::one(other));
    assert!(h.model.areas.performance.monitoring.is_none());

    let mut h = Harness::scene("host-monitoring");
    h.dismiss();
    assert!(h.surfaces().is_empty());

    // The command is for one server the inventory has.
    let vm = ObjectRef::vm(pool(&h), scenes::GUEST.parse::<uuid::Uuid>().unwrap().into());
    let hidden = h.model.eligibility(MONITORING, &Target::one(vm));
    assert_eq!(hidden, Eligibility::Hidden);
    let unknown = ObjectRef::host(pool(&h), uuid::Uuid::new_v4().into());
    let hidden = h.model.eligibility(MONITORING, &Target::one(unknown));
    assert_eq!(hidden, Eligibility::Hidden);
}

// -- 3. Against a live session ----------------------------------------------------

/// The tab starts a subscription on a current connection and leaving it
/// ends it. The fixture server serves no RRD data: what the stream then
/// reports is the core's business, and nothing of it is an operation.
#[test]
fn the_tab_of_a_live_vm_subscribes_and_leaving_it_drops_the_subscription() {
    let mut live = Live::with_vm();
    live.h.select(Target::one(live.vm()));
    assert!(live.h.model.areas.performance.metrics.is_none());
    live.h.open_tab(TabId::Performance);
    let view = shown(&live.h);
    assert_eq!(view.pool, live.pool);
    assert_eq!(view.target, MetricTarget::Vm(live.server.vm()));
    assert_eq!(view.error, None);
    assert!(view.handle.is_some() && view.snapshot.is_some());
    assert!(!live.h.has_text("Synthetic performance preview"));
    assert_eq!(live.h.eligibility(RECONNECT.0), Eligibility::Enabled);
    assert_eq!(live.h.eligibility(DATA_SOURCES.0), Eligibility::Enabled);

    // A new range is a new subscription for the same target.
    live.h.click("metrics-window-7200");
    let view = shown(&live.h);
    assert_eq!(view.options.window, MetricsWindow::TwoHours);
    assert!(view.handle.is_some());

    // The server of the pool, under the same tab.
    live.h.select(Target::one(live.host()));
    let view = shown(&live.h);
    assert_eq!(view.target, MetricTarget::Host(live.server.host()));
    assert!(view.handle.is_some());

    live.h.open_tab(TabId::General);
    assert!(live.h.model.areas.performance.metrics.is_none());
    assert!(live.h.model.resource_operations.is_empty());
    assert!(live.h.model.operation_rows.is_empty());
}

/// On a live server the dialog observes it and asks for the catalog; the
/// review of a listed source is current and blocked by nothing but its
/// own acknowledgement.
#[test]
fn the_dialog_of_a_live_server_reads_the_catalog_and_its_review_is_current() {
    let mut live = Live::with_vm();
    live.h.select(Target::one(live.host()));
    live.h.invoke(MONITORING.0);
    let p = panel(&live.h);
    assert_eq!(p.pool, live.pool);
    assert!(p.observed.is_some());
    assert!(p.query.is_some() || p.error.is_some());
    assert!(monitoring::current(&live.h.model, p));
    assert!(live.h.find("monitoring-preview").is_none());
    assert!(live.h.find("monitoring-stale").is_none());
    assert_eq!(live.h.node("monitoring-refresh").disabled, None);
}
