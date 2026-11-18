//! The performance charts: the bespoke element `metrics-chart`.
//!
//! The Performance tab describes one `Custom` block per series, which only
//! names the chart (its key, its index, its source). The element reads the
//! chart of that key from the model at every render
//! (`xenadmin_ui::areas::performance::chart`) and keeps no state of its
//! own: the number of charts and their indexes change between two renders.
//!
//! One card per chart: the title, the source's description, the summary,
//! then the label of the top edge, the plot, the label of the bottom edge
//! and the times of the first and the last sample. A series without a
//! finite sample has the line [`NO_SAMPLES`] in the place of all that.
//!
//! The gap rule is why the element is bespoke (the Kit's line chart joins
//! the neighbours of a missing sample): every unbroken run of samples is a
//! stroke of its own, no stroke joins two runs, none is closed to the
//! baseline, and a run of one sample is a dot. [`geometry`] is all the plot
//! computes; the paint draws exactly that.
//!
//! As in the Rune client the plot has three horizontal grid lines and
//! nothing else: no ticks, no legend, no hover.

use xenadmin_core::MetricsState;
use xenadmin_ui::Model;
use xenadmin_ui::areas::performance::{
    self as area, CHART_HEIGHT, CHART_WIDTH, Chart, GRID_LINES, NO_SAMPLES, Plot,
};
use xenadmin_ui::surface::Custom;

use crate::kit::*;
use crate::render::Ui;

/// The height of the plot in pixels, as in the Rune client. The plot never
/// shrinks: the tab body scrolls instead.
pub const PLOT_HEIGHT: f32 = CHART_HEIGHT;
/// The width of the line of a series.
pub const STROKE: f32 = 1.8;
/// The diameter of the dot of a sample with a gap on both sides.
pub const MARK: f32 = 4.0;
/// The edge kept free inside the plot, so that a line along the top or the
/// bottom edge (a constant series) and a dot at a corner are drawn whole.
pub const INSET: f32 = MARK / 2.0;

/// The suffixes of the ids of the parts of a card, after the chart's key.
pub const TITLE: &str = "-title";
pub const DESCRIPTION: &str = "-description";
pub const SUMMARY: &str = "-summary";
pub const HIGH: &str = "-high";
pub const PLOT: &str = "-plot";
pub const LOW: &str = "-low";
pub const FIRST: &str = "-first";
pub const LAST: &str = "-last";
/// The line of a series without a finite sample.
pub const EMPTY: &str = "-empty";
/// The line of a chart whose data has not arrived.
pub const LOADING: &str = "-loading";

/// What the plot of a chart paints, in pixels from the top left corner of
/// the plot.
#[derive(Clone, Debug, PartialEq)]
pub struct Geometry {
    pub width: f32,
    pub height: f32,
    /// One stroke per unbroken run of two or more samples.
    pub lines: Vec<Vec<[f32; 2]>>,
    /// The centre of the dot of each run of one sample.
    pub marks: Vec<[f32; 2]>,
    /// The top of each horizontal grid line, which is one pixel high.
    pub grid: Vec<f32>,
}

/// Scales the box of a plot's points to a plot of this size. A run stays a
/// run: nothing is joined and nothing is added.
pub fn geometry(plot: &Plot, width: f32, height: f32) -> Geometry {
    let span_x = (width - 2.0 * INSET).max(0.0);
    let span_y = (height - 2.0 * INSET).max(0.0);
    let x = |x: f32| INSET + (x / CHART_WIDTH).clamp(0.0, 1.0) * span_x;
    let y = |y: f32| INSET + (y / CHART_HEIGHT).clamp(0.0, 1.0) * span_y;
    let mut lines = Vec::new();
    let mut marks = Vec::new();
    for segment in &plot.segments {
        let points: Vec<[f32; 2]> = segment
            .iter()
            .filter(|point| point[0].is_finite() && point[1].is_finite())
            .map(|point| [x(point[0]), y(point[1])])
            .collect();
        match points.as_slice() {
            [] => {}
            [point] => marks.push(*point),
            _ => lines.push(points),
        }
    }
    Geometry {
        width,
        height,
        lines,
        marks,
        // A whole pixel row, so the line is crisp, and inside the plot.
        grid: GRID_LINES
            .iter()
            .map(|line| (y(*line) - 0.5).round().clamp(0.0, (height - 1.0).max(0.0)))
            .collect(),
    }
}

/// The colour of the line of a series: the accent of the theme in use.
pub fn stroke_color(cx: &App) -> Hsla {
    cx.theme().primary
}

/// The colour of the grid lines: the border colour of the theme in use.
pub fn grid_color(cx: &App) -> Hsla {
    cx.theme().border
}

/// Registers the element with a renderer.
pub fn register(ui: &Ui) {
    ui.register_custom(area::CHART, card);
}

/// A chart whose block is described and whose data is not there is waiting
/// for it when the view has no snapshot yet or a loading one. Otherwise its
/// series is gone and nothing is drawn.
fn waiting(model: &Model) -> bool {
    model
        .areas
        .performance
        .metrics
        .as_ref()
        .is_some_and(|view| {
            view.snapshot
                .as_ref()
                .is_none_or(|snapshot| snapshot.state == MetricsState::Loading)
        })
}

fn id(key: &str, part: &str) -> SharedString {
    SharedString::from(format!("{key}{part}"))
}

/// A small line of text in the muted colour, which wraps.
fn caption(id: SharedString, value: &str, cx: &App) -> Keyed {
    let value = SharedString::from(value.to_owned());
    div()
        .id(id)
        .test_support()
        .role(Role::Label)
        .aria_label(value.clone())
        .min_w_0()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(value)
}

fn frame(key: &str, label: SharedString, cx: &App) -> Keyed {
    let theme = cx.theme();
    v_flex()
        .id(SharedString::from(key.to_owned()))
        .test_support()
        .role(Role::Figure)
        .aria_label(label)
        .w_full()
        .flex_none()
        .gap(px(6.))
        .p_3()
        .rounded(theme.radius)
        .border_1()
        .border_color(theme.border)
}

fn title(key: &str, value: &str) -> Keyed {
    let value = SharedString::from(value.to_owned());
    div()
        .id(id(key, TITLE))
        .test_support()
        .role(Role::Heading)
        .aria_label(value.clone())
        .w_full()
        .text_sm()
        .font_semibold()
        .child(value)
}

/// The card of the chart the block names, from the model as it is now.
fn card(custom: &Custom, ui: &Ui, window: &mut Window, cx: &mut App) -> AnyElement {
    let key = custom.key.clone().unwrap_or_else(|| custom.name.clone());
    let (chart, waiting) = {
        let model = ui.store.read(cx).model();
        let chart = area::chart(model, &key);
        let waiting = chart.is_none() && waiting(model);
        (chart, waiting)
    };
    match chart {
        Some(chart) => drawn(&chart, window, cx),
        None if waiting => loading(&key, custom, cx),
        // The series is gone: nothing is drawn. The key stays an element,
        // so the frame still has every key of its description.
        None => div()
            .id(SharedString::from(key))
            .test_support()
            .into_any_element(),
    }
}

/// The card of a chart whose data has not arrived.
fn loading(key: &str, custom: &Custom, cx: &App) -> AnyElement {
    let text = area::metrics::state_label(MetricsState::Loading);
    let source = custom
        .params
        .get("source")
        .and_then(|source| source.as_str())
        .unwrap_or(area::CHART);
    frame(key, format!("{source} chart. {text}").into(), cx)
        .child(title(key, source))
        .child(caption(id(key, LOADING), text, cx).w_full())
        .into_any_element()
}

fn drawn(chart: &Chart, window: &mut Window, cx: &mut App) -> AnyElement {
    let key = chart.key.as_str();
    let label = SharedString::from(chart.label.clone());
    let card = frame(key, label.clone(), cx)
        .child(title(key, &chart.title))
        .when(!chart.description.is_empty(), |card| {
            card.child(caption(id(key, DESCRIPTION), &chart.description, cx).w_full())
        })
        .child(caption(id(key, SUMMARY), &chart.summary, cx).w_full());
    let Some(plot) = &chart.plot else {
        return card
            .child(caption(id(key, EMPTY), NO_SAMPLES, cx).w_full())
            .into_any_element();
    };
    card.child(caption(id(key, HIGH), &plot.high, cx).w_full())
        .child(self::plot(key, label, plot, window, cx))
        .child(caption(id(key, LOW), &plot.low, cx).w_full())
        .child(
            // The two times part to the two ends; in a card too narrow for
            // both on a line the second goes under the first.
            h_flex()
                .w_full()
                .flex_wrap()
                .justify_between()
                .gap_x_3()
                .child(caption(id(key, FIRST), &plot.first, cx))
                .child(caption(id(key, LAST), &plot.last, cx)),
        )
        .into_any_element()
}

/// The plot: the grid and the line, as wide as the card.
fn plot(
    key: &str,
    label: SharedString,
    plot: &Plot,
    window: &mut Window,
    cx: &mut App,
) -> impl IntoElement + use<> {
    let plot = plot.clone();
    #[cfg(feature = "scenes")]
    let record = log::slot(key, window, cx);
    #[cfg(not(feature = "scenes"))]
    let _ = &window;
    div()
        .id(id(key, PLOT))
        .test_support()
        .role(Role::Image)
        .aria_label(label)
        .w_full()
        .h(px(PLOT_HEIGHT))
        .flex_none()
        .child(
            canvas(
                move |bounds, _, _| {
                    geometry(
                        &plot,
                        bounds.size.width.as_f32(),
                        bounds.size.height.as_f32(),
                    )
                },
                move |bounds, geometry, window, cx| {
                    let (stroke, grid) = (stroke_color(cx), grid_color(cx));
                    paint(bounds, &geometry, stroke, grid, window);
                    #[cfg(feature = "scenes")]
                    record.update(cx, |record, _| {
                        *record = Some(Painted {
                            bounds,
                            geometry,
                            stroke,
                            grid,
                        });
                    });
                },
            )
            .size_full(),
        )
}

fn paint(
    bounds: Bounds<Pixels>,
    geometry: &Geometry,
    stroke: Hsla,
    grid: Hsla,
    window: &mut Window,
) {
    let at = |[x, y]: [f32; 2]| bounds.origin + point(px(x), px(y));
    for top in &geometry.grid {
        window.paint_quad(fill(
            Bounds::new(at([0.0, *top]), size(bounds.size.width, px(1.))),
            grid,
        ));
    }
    // One path per run: two runs are never one stroke.
    for line in &geometry.lines {
        let mut path = PathBuilder::stroke(px(STROKE));
        let mut points = line.iter();
        if let Some(first) = points.next() {
            path.move_to(at(*first));
        }
        for point in points {
            path.line_to(at(*point));
        }
        if let Ok(path) = path.build() {
            window.paint_path(path, stroke);
        }
    }
    for mark in &geometry.marks {
        window.paint_quad(
            fill(
                Bounds::centered_at(at(*mark), size(px(MARK), px(MARK))),
                stroke,
            )
            .corner_radii(Corners::all(px(MARK / 2.0))),
        );
    }
}

/// What the plot of a chart painted last. Kept only in a build with the
/// `scenes` feature, for the tests.
#[cfg(feature = "scenes")]
#[derive(Clone, Debug, PartialEq)]
pub struct Painted {
    /// Where the plot is in the window.
    pub bounds: Bounds<Pixels>,
    pub geometry: Geometry,
    pub stroke: Hsla,
    pub grid: Hsla,
}

/// What the plot of the chart `key` painted in the last frame of `window`
/// that had it. `None` once a frame was drawn without that chart.
#[cfg(feature = "scenes")]
pub fn painted(window: WindowId, key: &str, cx: &App) -> Option<Painted> {
    log::painted(window, key, cx)
}

#[cfg(feature = "scenes")]
mod log {
    use super::Painted;
    use crate::kit::*;

    /// The record of one plot. It is element state: GPUI drops it with the
    /// first frame that does not draw the plot.
    type Record = Option<Painted>;

    #[derive(Default)]
    struct Records(Vec<(WindowId, String, WeakEntity<Record>)>);

    impl Global for Records {}

    pub(super) fn slot(key: &str, window: &mut Window, cx: &mut App) -> Entity<Record> {
        let state = SharedString::from(format!("{key}-painted"));
        let record = window.use_keyed_state(state, cx, |_, _| Record::None);
        let window = window.window_handle().window_id();
        let known = cx.try_global::<Records>().is_some_and(|records| {
            records
                .0
                .iter()
                .any(|(_, _, known)| known.entity_id() == record.entity_id())
        });
        if !known {
            let records = cx.default_global::<Records>();
            records
                .0
                .retain(|(_, _, record)| record.upgrade().is_some());
            records.0.push((window, key.to_owned(), record.downgrade()));
        }
        record
    }

    pub(super) fn painted(window: WindowId, key: &str, cx: &App) -> Option<Painted> {
        cx.try_global::<Records>()?
            .0
            .iter()
            .filter(|(of, name, _)| *of == window && name == key)
            .find_map(|(_, _, record)| record.upgrade()?.read(cx).clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plot(segments: Vec<Vec<[f32; 2]>>) -> Plot {
        Plot {
            high: "1".into(),
            low: "0".into(),
            first: "first".into(),
            last: "last".into(),
            segments,
        }
    }

    #[test]
    fn every_run_is_a_line_of_its_own_and_none_crosses_the_gap() {
        let plot = plot(vec![
            vec![[0.0, 120.0], [100.0, 60.0], [200.0, 0.0]],
            vec![[600.0, 30.0], [1000.0, 90.0]],
        ]);
        let g = geometry(&plot, 504.0, 124.0);
        assert_eq!(g.lines.len(), 2);
        assert!(g.marks.is_empty());
        assert_eq!(g.lines[0].len(), 3);
        assert_eq!(g.lines[1].len(), 2);
        // The box is scaled to the plot, the inset in from each edge.
        assert_eq!(g.lines[0][0], [2.0, 122.0]);
        assert_eq!(g.lines[0][2], [102.0, 2.0]);
        assert_eq!(g.lines[1][0], [302.0, 32.0]);
        assert_eq!(g.lines[1][1], [502.0, 92.0]);
        // Nothing is drawn between the end of a run and the start of the
        // next.
        let (end, start) = (g.lines[0][2][0], g.lines[1][0][0]);
        for line in &g.lines {
            for point in line {
                assert!(point[0] <= end || point[0] >= start, "{point:?}");
            }
        }
    }

    #[test]
    fn a_run_of_one_sample_is_a_mark() {
        let plot = plot(vec![vec![[0.0, 0.0]], vec![[1000.0, 120.0]]]);
        let g = geometry(&plot, 204.0, 124.0);
        assert!(g.lines.is_empty());
        assert_eq!(g.marks, vec![[2.0, 2.0], [202.0, 122.0]]);
    }

    #[test]
    fn the_grid_is_three_whole_rows_inside_the_plot() {
        let g = geometry(&plot(Vec::new()), 300.0, 120.0);
        // At the height of the top edge's value, the middle and the bottom
        // edge's value.
        assert_eq!(g.grid, vec![2.0, 60.0, 118.0]);
        assert!(g.lines.is_empty() && g.marks.is_empty());
        // A plot of no size computes nothing that is not finite.
        let g = geometry(&plot(vec![vec![[500.0, 60.0]]]), 0.0, 0.0);
        assert!(g.marks[0].iter().all(|value| value.is_finite()));
        assert!(g.grid.iter().all(|top| *top == 0.0));
    }
}
