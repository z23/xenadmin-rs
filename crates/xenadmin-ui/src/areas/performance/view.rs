//! The Performance tab and the dialog "Host monitoring recording",
//! described. Nothing here changes the model.
//!
//! # The tab
//!
//! In the Rune order: the target line, the range, the notice and the error
//! of the view, and then ONE of three bodies, as in Rune: the saved presets,
//! the source picker, or the charts under their state heading. Each chart is
//! one `Custom` block named `metrics-chart` with the key
//! `metrics-chart-<index>`; its parameters name it (`index`, `source`, and
//! `label`, the accessible name) and its data comes from
//! [`super::chart`]. Everything else is vocabulary.
//!
//! Rune keys kept: `metrics-window-{seconds}` (now the options of the
//! choice `metrics-window`), `apply-metric-sources`, `metric-preset-name`,
//! `metric-preset-save`, `metric-preset-{name}`,
//! `metric-preset-remove-{name}`, `metric-presets-close`.
//! Rune keys without an element here: `open-performance`,
//! `close-performance` (the tab), `metrics-sources`, `metric-presets-open`
//! (the commands `performance.data-sources` and `performance.presets`).
//! New keys: `metrics-target`, `metrics-window`, `metrics-notice`,
//! `metrics-error`, `metrics-waiting`, `metrics-not-current`,
//! `metrics-snapshot-error`, `metrics-range`, `metrics-empty`,
//! `metrics-chart-<index>`, `metrics-source-filter`,
//! `metrics-source-{name}`, `metrics-sources-cancel`,
//! `metrics-sources-common`.
//!
//! As in Rune, a preset's name is part of two keys: a preset called "save"
//! or "name" shares its key with the Save button or the name field. The
//! harness finds the first; nothing else depends on it.
//!
//! # The dialog
//!
//! Rune keys kept: `monitoring-search`, `monitoring-review-start`,
//! `monitoring-review-forget`, `monitoring-confirm-history-loss`,
//! `monitoring-close`, `monitoring-refresh`, `monitoring-history`,
//! `monitoring-back`, `monitoring-submit`. Without an element here:
//! `monitoring-heading` (the dialog's title) and `monitoring-source-{name}`
//! (rows of the table `monitoring-sources`, whose ids are the names).
//! New keys: `monitoring-sources`, `monitoring-reading`,
//! `monitoring-count`, `monitoring-review` (and `monitoring-review-stale`),
//! `monitoring-read-only`, `monitoring-stale`, `monitoring-preview`,
//! `monitoring-error`.

use xenadmin_core::{HostOperation, HostRecordingAction, MetricTarget, PoolId};

use super::commands::NOT_CURRENT;
use super::metrics::{self, MAX_SOURCES, MetricsUi};
use super::monitoring::{self, Panel};
use super::presets::{self, Kind};
use crate::prelude::*;

/// The id of the dialog.
pub const MONITORING: &str = "host-monitoring";

/// Why a source that is not chosen cannot be added to a full selection.
const FULL: &str = "At most 32 data sources can be shown at once.";
/// Why an empty selection cannot be applied.
const NONE_SELECTED: &str = "Select at least one data source, or use the common sources.";

// -- The tab ------------------------------------------------------------------

pub fn tab(m: &Model, tab: TabId, scope: &Target) -> Option<TabBody> {
    if tab != TabId::Performance {
        return None;
    }
    let (pool, target) = scope.single().and_then(metrics::target_of)?;
    Some(TabBody::Blocks(match metrics::view_of(m, pool, target) {
        Some(view) => body(m, view),
        None => waiting(m, pool, target),
    }))
}

fn target_line(pool: PoolId, target: MetricTarget) -> Text {
    caption(format!("{} · Pool {}", target.uuid(), pool)).key("metrics-target")
}

/// No view yet: the hooks open one as soon as the connection is current.
fn waiting(m: &Model, pool: PoolId, target: MetricTarget) -> Vec<Block> {
    blocks![
        target_line(pool, target),
        caption("Waiting for performance data.").key("metrics-waiting"),
        (!metrics::can_open(m, pool)).then(|| caption(NOT_CURRENT)
            .tone(Tone::Warning)
            .key("metrics-not-current")),
    ]
}

fn body(m: &Model, view: &MetricsUi) -> Vec<Block> {
    blocks![
        target_line(view.pool, view.target),
        range(view),
        view.notice
            .clone()
            .map(|notice| caption(notice).key("metrics-notice")),
        view.error
            .clone()
            .map(|error| alert(error).key("metrics-error")),
        if view.show_presets {
            saved_presets(m, view)
        } else if view.choosing_sources {
            source_picker(view)
        } else {
            graphs(m, view)
        },
    ]
}

fn window_id(window: xenadmin_core::MetricsWindow) -> String {
    window.duration_seconds().to_string()
}

fn range(view: &MetricsUi) -> Choice {
    let options = metrics::WINDOWS.map(|window| {
        option(window_id(window), window.label()).key(format!("metrics-window-{}", window_id(window)))
    });
    let current = window_id(view.options.window);
    choice("metrics-window", options, Some(current.as_str()), |m, id| {
        if let Some(window) = metrics::WINDOWS
            .into_iter()
            .find(|window| window_id(*window) == id)
        {
            metrics::window(m, window);
        }
    })
    .label("Range")
    .segmented()
}

fn graphs(m: &Model, view: &MetricsUi) -> Vec<Block> {
    let Some(snapshot) = &view.snapshot else {
        return blocks![
            caption("Waiting for performance data.").key("metrics-waiting"),
            gaps_note(),
        ];
    };
    let charts = metrics::charts(m)
        .into_iter()
        .map(|chart| {
            custom(
                metrics::CHART,
                serde_json::json!({
                    "index": chart.index,
                    "source": chart.title,
                    "label": chart.label,
                }),
            )
            .key(chart.key)
        })
        .collect::<Vec<_>>();
    blocks![
        heading(if m.is_synthetic_pool(view.pool) {
            "Synthetic performance preview"
        } else {
            metrics::state_label(snapshot.state)
        }),
        snapshot
            .error
            .clone()
            .map(|error| alert(error).key("metrics-snapshot-error")),
        caption(metrics::range_line(snapshot)).key("metrics-range"),
        snapshot.series.is_empty().then(|| caption(
            "No recorded data sources were returned for this target. Choose data sources or reconnect when the host has samples."
        )
        .key("metrics-empty")),
        charts,
        gaps_note(),
    ]
}

fn gaps_note() -> Text {
    caption(
        "Missing samples remain gaps. Historical coverage and resolution depend on the host's retained data.",
    )
}

fn source_picker(view: &MetricsUi) -> Vec<Block> {
    let filter = view.source_filter.to_lowercase();
    let full = view.selected_sources.len() >= MAX_SOURCES;
    let sources = view.snapshot.iter().flat_map(|snapshot| &snapshot.sources);
    let choices = sources
        .filter(|source| {
            format!("{} {}", source.name, source.description)
                .to_lowercase()
                .contains(&filter)
        })
        .map(|source| {
            let name = source.name.clone();
            let chosen = view.selected_sources.contains(&name);
            let label = format!(
                "{} · {}{}",
                name,
                source.units,
                if source.enabled {
                    ""
                } else {
                    " · not recorded"
                }
            );
            let field = checkbox(format!("metrics-source-{name}"), label, chosen, move |m| {
                metrics::toggle_source(m, &name)
            })
            .disabled_if(!chosen && full, FULL);
            if source.description.is_empty() {
                field
            } else {
                field.help(source.description.clone())
            }
        })
        .collect::<Vec<_>>();
    blocks![
        heading(format!(
            "Data sources · {} of 32 selected",
            view.selected_sources.len()
        )),
        input(
            "metrics-source-filter",
            "Filter data sources",
            &view.source_filter,
            |m, text| {
                if let Some(view) = &mut m.areas.performance.metrics {
                    view.source_filter = text.to_owned();
                }
            }
        ),
        choices,
        actions([
            button("metrics-sources-cancel", "Cancel", metrics::cancel_sources),
            button("metrics-sources-common", "Common sources", |m| {
                metrics::apply_sources(m, true)
            }),
            button("apply-metric-sources", "Apply sources", |m| {
                metrics::apply_sources(m, false)
            })
            .primary()
            .disabled_if(view.selected_sources.is_empty(), NONE_SELECTED),
        ]),
    ]
}

fn saved_presets(m: &Model, view: &MetricsUi) -> Vec<Block> {
    let kind = Kind::of(view.target);
    let saved = m.preferences.metric_presets.presets.iter();
    let rows = saved
        .filter(|preset| preset.kind == kind)
        .flat_map(|preset| {
            let name = preset.name.clone();
            let remove_name = name.clone();
            blocks![
                actions([
                    button(
                        format!("metric-preset-{}", preset.name),
                        &preset.name,
                        move |m| presets::apply(m, &name)
                    ),
                    button(
                        format!("metric-preset-remove-{}", preset.name),
                        "Remove",
                        move |m| presets::remove(m, &remove_name)
                    )
                    .danger(),
                ]),
                caption(format!(
                    "{} · {}",
                    preset.options.window.label(),
                    if preset.options.sources.is_empty() {
                        "default sources".into()
                    } else {
                        format!("{} sources", preset.options.sources.len())
                    }
                )),
            ]
        })
        .collect::<Vec<_>>();
    blocks![
        heading("Saved graph presets"),
        caption(
            "Presets keep a time range and data-source names for this target type. They contain no samples or host credentials. Saving an existing name updates it."
        ),
        input(
            "metric-preset-name",
            "Preset name",
            &view.preset_name,
            |m, text| {
                if let Some(view) = &mut m.areas.performance.metrics {
                    view.preset_name = text.to_owned();
                }
            }
        )
        .placeholder("Preset name")
        .on_submit(presets::save),
        actions([button(
            "metric-preset-save",
            "Save current graph",
            presets::save
        )]),
        rows,
        actions([button("metric-presets-close", "Back to graphs", presets::toggle).outline()]),
    ]
}

// -- The dialog ---------------------------------------------------------------

pub fn surfaces(m: &Model) -> Vec<Surface> {
    let Some(p) = &m.areas.performance.monitoring else {
        return Vec::new();
    };
    let permission = p
        .review
        .as_ref()
        .map(|op| permissions::assess_for(m, p.pool, &op.permission_requirements()));
    let surface = Surface::dialog(MONITORING, "Host monitoring recording")
        .subtitle(format!(
            "{} · {} · Pool {}",
            p.name,
            p.target.uuid(),
            p.pool
        ))
        .experimental(true)
        .large()
        .body(blocks![
            match &p.review {
                Some(op) => reviewing(m, p, op),
                None => catalog(m, p),
            },
            m.connection_for(p.pool)
                .is_some_and(|c| c.is_read_only())
                .then(|| caption(monitoring::READ_ONLY).key("monitoring-read-only")),
            monitoring::synthetic(m, p).then(|| {
                notice(NoticeKind::Preview, monitoring::PREVIEW).key("monitoring-preview")
            }),
            p.error
                .clone()
                .map(|error| alert(error).key("monitoring-error")),
        ])
        .footer(footer(m, p))
        .on_close(monitoring::close);
    vec![match &permission {
        Some(permission) => surface.footer_note(permissions::footer_note(permission)),
        None => surface,
    }]
}

fn catalog(m: &Model, p: &Panel) -> Vec<Block> {
    blocks![
        caption(
            "Host recording controls affect the history collected by XAPI. Your graph selections are separate."
        ),
        input(
            "monitoring-search",
            "Find a data source",
            &p.search,
            |m, text| {
                if let Some(p) = &mut m.areas.performance.monitoring {
                    p.search = text.to_owned();
                }
            }
        )
        .placeholder("Find a data source"),
        p.query
            .is_some()
            .then(|| caption("Reading host data sources…").key("monitoring-reading")),
        p.sources.is_some().then(|| sources(p)),
        monitoring::selected(p).map(|source| {
            blocks![
                heading(&source.name),
                monitoring::source_lines(source)
                    .into_iter()
                    .map(caption)
                    .collect::<Vec<_>>(),
                actions([
                    button("monitoring-review-start", "Review start recording", |m| {
                        monitoring::review(m, HostRecordingAction::StartRecording)
                    })
                    .disabled_if(source.enabled, monitoring::ALREADY),
                    button(
                        "monitoring-review-forget",
                        "Review history deletion",
                        |m| { monitoring::review(m, HostRecordingAction::ForgetHistory) }
                    ),
                ]),
            ]
        }),
        caption(
            "Stock XCP-ng 8.3 does not ship Nagios NRPE integration. This page manages XAPI data-source recording; it does not install monitoring agents or change NRPE settings."
        ),
        monitoring::stale(m, p)
            .map(|reason| notice(NoticeKind::Warning, reason).key("monitoring-stale")),
    ]
}

/// The catalog as a grid: one row per source the search leaves, selected
/// by its name.
fn sources(p: &Panel) -> Vec<Block> {
    let visible = monitoring::visible(p);
    let count = caption(format!("{} sources", visible.len())).key("monitoring-count");
    let rows = visible.into_iter().map(|source| {
        row(
            &source.name,
            [
                source.name.as_str(),
                if source.enabled {
                    "Recording"
                } else {
                    "Not recording"
                },
                source.units.as_str(),
                source.description.as_str(),
            ],
        )
    });
    let columns = [
        column("name", "Data source"),
        column("recording", "Recording"),
        column("units", "Units"),
        column("description", "Description"),
    ];
    blocks![
        count,
        table("monitoring-sources", columns, rows)
            .single(p.selected.as_deref(), |m, id| {
                if let Some(p) = &mut m.areas.performance.monitoring {
                    p.selected = Some(id.to_owned());
                }
            })
            .empty("No data source matches."),
    ]
}

fn reviewing(m: &Model, p: &Panel, op: &HostOperation) -> Vec<Block> {
    let HostOperation::DataSource { action, .. } = op else {
        return Vec::new();
    };
    let availability = m
        .inventory_for(p.pool)
        .map(|i| op.availability(&i, p.target))
        .filter(|availability| !availability.is_available());
    blocks![
        heading(action.label()),
        review("monitoring-review", monitoring::review_lines(op))
            .warnings(availability.map(|availability| availability.explanation()))
            .stale(monitoring::stale(m, p)),
        (*action == HostRecordingAction::ForgetHistory).then(|| checkbox(
            "monitoring-confirm-history-loss",
            "I understand that this source's recorded history will be deleted",
            p.acknowledged,
            monitoring::acknowledge
        )),
    ]
}

/// Close, History, and then the step's own buttons; the last is its
/// default action.
fn footer(m: &Model, p: &Panel) -> Vec<Button> {
    let mut footer = vec![
        button("monitoring-close", "Close", monitoring::close),
        button("monitoring-history", "History", monitoring::history),
    ];
    match &p.review {
        Some(HostOperation::DataSource { action, .. }) => {
            footer.push(button("monitoring-back", "Back", monitoring::back));
            let submit = button("monitoring-submit", action.label(), monitoring::submit)
                .disabled_opt(monitoring::blocked(m, p));
            footer.push(if *action == HostRecordingAction::ForgetHistory {
                submit.danger()
            } else {
                submit.primary()
            });
        }
        _ => footer.push(
            button("monitoring-refresh", "Refresh catalog", monitoring::refresh)
                .disabled_opt(monitoring::stale(m, p)),
        ),
    }
    footer
}
