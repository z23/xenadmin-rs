//! The performance chart element `metrics-chart`, drawn by the real
//! renderer on the Kit's headless test platform: the cards of the scene
//! `performance`, the gap rule, the states without a plot, and what a
//! change of the range, the sources, the tab and the theme does.
//!
//! The tests read the element tree and what each plot painted
//! (`charts::painted`, kept in a build with the `scenes` feature). Two
//! tests look at the pixels of a captured frame; they need a graphics
//! adapter (a software one will do) and say so and pass without one.
//!
//! What these tests do NOT establish: Wayland, a real keyboard and what a
//! screen reader says of the chart. The roles and labels are those of the
//! Kit's test platform.
#![cfg(feature = "scenes")]

use std::rc::Rc;
use std::sync::Arc;

use serde_json::{Value, json};
use xenadmin_core::{MetricSample, MetricsSnapshot};
use xenadmin_desktop::Describe;
use xenadmin_desktop::charts::{self, Geometry, Painted};
use xenadmin_desktop::kit::testing::{ElementSnapshot, TestAppContext, TestWindowExt as _};
use xenadmin_desktop::kit::*;
use xenadmin_desktop::scenes::{self, Options, Setup, ShellWindow};
use xenadmin_ui::Model;
use xenadmin_ui::areas::performance::{self as area, Chart};
use xenadmin_ui::command::CommandId;
use xenadmin_ui::surface::{Block, Custom};
use xenadmin_ui::tabs::{TabBody, TabId};

const WIDE: (f32, f32) = (1440., 900.);
/// The usual size of the main window and its smallest.
const SIZES: [(f32, f32); 2] = [WIDE, (800., 600.)];
const KEYS: [&str; 4] = [
    "metrics-chart-0",
    "metrics-chart-1",
    "metrics-chart-2",
    "metrics-chart-3",
];

fn options((width, height): (f32, f32), mode: ThemeMode) -> Options {
    Options {
        width,
        height,
        mode,
        ..Options::default()
    }
}

fn performance() -> Setup {
    scenes::setup("performance").expect("the scene performance")
}

fn open(cx: &mut TestAppContext, setup: Setup, options: &Options) -> ShellWindow {
    let scene = cx
        .update(|cx| scenes::open_shell(cx, setup, options))
        .expect("shell window");
    cx.run_until_parked();
    settle(cx, &scene);
    scene
}

fn open_scene(cx: &mut TestAppContext, size: (f32, f32)) -> ShellWindow {
    open(cx, performance(), &options(size, ThemeMode::Dark))
}

/// Draws until what the last event caused is on screen.
fn settle(cx: &mut TestAppContext, scene: &ShellWindow) {
    for _ in 0..3 {
        cx.update_window(scene.window, |_, window, cx| window.render_frame(cx))
            .expect("window");
        cx.run_until_parked();
    }
}

fn on<R>(
    cx: &mut TestAppContext,
    scene: &ShellWindow,
    act: impl FnOnce(&mut Window, &mut App) -> R,
) -> R {
    let result = cx
        .update_window(scene.window, |_, window, cx| act(window, cx))
        .expect("window");
    cx.run_until_parked();
    settle(cx, scene);
    result
}

fn run(cx: &mut TestAppContext, scene: &ShellWindow, act: impl FnOnce(&mut Model)) {
    cx.update(|cx| scene.store.update(cx, |store, cx| store.run(act, cx)));
    cx.run_until_parked();
    settle(cx, scene);
}

fn invoke(cx: &mut TestAppContext, scene: &ShellWindow, id: &'static str) {
    cx.update(|cx| {
        scene
            .store
            .update(cx, |store, cx| store.invoke(CommandId(id), cx))
    });
    cx.run_until_parked();
    settle(cx, scene);
}

fn model<R>(cx: &mut TestAppContext, scene: &ShellWindow, read: impl FnOnce(&Model) -> R) -> R {
    cx.update(|cx| read(scene.store.read(cx).model()))
}

fn chart(cx: &mut TestAppContext, scene: &ShellWindow, key: &str) -> Chart {
    model(cx, scene, |m| area::chart(m, key)).unwrap_or_else(|| panic!("no chart {key}"))
}

fn painted(cx: &mut TestAppContext, scene: &ShellWindow, key: &str) -> Option<Painted> {
    cx.update(|cx| charts::painted(scene.window.window_id(), key, cx))
}

fn id(key: &str, part: &str) -> ElementId {
    ElementId::Name(SharedString::from(format!("{key}{part}")))
}

#[track_caller]
fn part(window: &Window, key: &str, part: &str) -> ElementSnapshot {
    window
        .try_find(id(key, part))
        .unwrap_or_else(|| panic!("{key}{part} is not in the frame"))
}

fn has(window: &Window, key: &str, part: &str) -> bool {
    window.try_find(id(key, part)).is_some()
}

/// Replaces the samples of the first series of the open view and drops the
/// other series.
fn set_samples(m: &mut Model, values: &[Option<f64>]) {
    let view = m.areas.performance.metrics.as_mut().expect("a view");
    let mut snapshot: MetricsSnapshot = (**view.snapshot.as_ref().expect("a snapshot")).clone();
    let step = snapshot.sample_step_seconds.unwrap_or(5);
    let start = snapshot.series[0].samples[0].timestamp;
    snapshot.series.truncate(1);
    snapshot.series[0].samples = values
        .iter()
        .enumerate()
        .map(|(index, value)| MetricSample {
            timestamp: start + index as u64 * step,
            value: *value,
        })
        .collect();
    view.snapshot = Some(Arc::new(snapshot));
}

/// The last x of the first stroke and the first x of the second: the two
/// sides of the gap.
fn gap(geometry: &Geometry) -> (f32, f32) {
    let end = geometry.lines[0].last().expect("a point")[0];
    let start = geometry.lines[1].first().expect("a point")[0];
    (end, start)
}

// Seam test 1 (and deliverables 2 and 3): every chart of the scene is a
// card with its key, its role and its label, and its content as text.
#[gpui_kit::test]
fn every_chart_of_the_scene_is_a_card_with_its_key_its_label_and_its_texts(
    cx: &mut TestAppContext,
) {
    for size in SIZES {
        let scene = open_scene(cx, size);
        let expected: Vec<Chart> = model(cx, &scene, area::charts);
        assert_eq!(expected.len(), 4);
        let store = scene.store.clone();
        on(cx, &scene, |window, cx| {
            for (key, chart) in KEYS.iter().zip(&expected) {
                let plot = chart.plot.as_ref().expect("a plot");
                assert_eq!(chart.key, *key);
                let card = part(window, key, "");
                // Not the placeholder of a missing element (a Note).
                assert_eq!(card.role(), Some(Role::Figure), "{key} {size:?}");
                assert_eq!(card.label(), Some(chart.label.as_str()));
                assert!(
                    chart
                        .label
                        .starts_with(&format!("{} chart. Latest: ", chart.title))
                );
                assert!(chart.label.ends_with("113 valid, 7 missing samples"));
                let texts = [
                    (charts::TITLE, Role::Heading, &chart.title),
                    (charts::DESCRIPTION, Role::Label, &chart.description),
                    (charts::SUMMARY, Role::Label, &chart.summary),
                    (charts::HIGH, Role::Label, &plot.high),
                    (charts::LOW, Role::Label, &plot.low),
                    (charts::FIRST, Role::Label, &plot.first),
                    (charts::LAST, Role::Label, &plot.last),
                ];
                let frame = card.bounds();
                assert!(frame.size.width > px(200.), "{key} {size:?}: {frame:?}");
                for (suffix, role, text) in texts {
                    let line = part(window, key, suffix);
                    assert_eq!(line.role(), Some(role), "{key}{suffix}");
                    assert_eq!(line.label(), Some(text.as_str()), "{key}{suffix}");
                    // The whole line is inside the card: nothing is cut.
                    let bounds = line.bounds();
                    assert!(
                        bounds.size.height > px(0.)
                            && bounds.left() >= frame.left()
                            && bounds.right() <= frame.right()
                            && bounds.top() >= frame.top()
                            && bounds.bottom() <= frame.bottom(),
                        "{key}{suffix} {size:?}: {bounds:?} in {frame:?}"
                    );
                }
                // The two times are on one line at these sizes, the first
                // at the left and the last at the right.
                let first = part(window, key, charts::FIRST).bounds();
                let last = part(window, key, charts::LAST).bounds();
                assert!(first.right() < last.left(), "{key} {size:?}");
                assert_eq!(first.top(), last.top(), "{key} {size:?}");
                // The plot: an image with the chart's label, as wide as
                // the inside of the card and of the fixed height.
                let image = part(window, key, charts::PLOT);
                assert_eq!(image.role(), Some(Role::Image));
                assert_eq!(image.label(), Some(chart.label.as_str()));
                let bounds = image.bounds();
                assert_eq!(bounds.size.height, px(charts::PLOT_HEIGHT));
                assert!(bounds.left() > frame.left() && bounds.right() < frame.right());
                assert!(bounds.size.width > frame.size.width - px(40.));
                assert!(!has(window, key, charts::EMPTY) && !has(window, key, charts::LOADING));
            }
            assert!(window.try_find(id("metrics-chart-4", "")).is_none());
            // The tool's check: every key of the description is an element.
            let tree = scenes::tree_json(window, cx, &store);
            assert_eq!(
                scenes::missing_keys(&tree),
                Vec::<String>::new(),
                "{size:?}"
            );
            for key in KEYS {
                let node = tree["elements"]
                    .as_array()
                    .expect("elements")
                    .iter()
                    .find(|element| element["id"] == *key)
                    .unwrap_or_else(|| panic!("{key} is not in the tree"));
                assert_eq!(node["role"], "Figure");
                assert!(
                    node["label"]
                        .as_str()
                        .is_some_and(|l| l.contains("Latest: "))
                );
            }
        });
    }
}

// Seam test 2, on what the element computed: the series of the scene has
// seven missing samples in a row, so the line is two strokes with nothing
// between them.
#[gpui_kit::test]
fn a_series_with_a_gap_is_painted_as_two_separate_strokes(cx: &mut TestAppContext) {
    for size in SIZES {
        let scene = open_scene(cx, size);
        for key in KEYS {
            let chart = chart(cx, &scene, key);
            let plot = chart.plot.expect("a plot");
            assert_eq!(
                plot.segments.iter().map(Vec::len).collect::<Vec<_>>(),
                [45, 68]
            );
            let painted = painted(cx, &scene, key).unwrap_or_else(|| panic!("{key} not painted"));
            let bounds = on(cx, &scene, |window, _| {
                part(window, key, charts::PLOT).bounds()
            });
            // The plot painted where the image element is, all of it.
            assert_eq!(painted.bounds, bounds);
            let g = &painted.geometry;
            assert_eq!(
                *g,
                charts::geometry(&plot, bounds.size.width.as_f32(), charts::PLOT_HEIGHT)
            );
            assert_eq!(g.lines.iter().map(Vec::len).collect::<Vec<_>>(), [45, 68]);
            assert!(g.marks.is_empty());
            let (end, start) = gap(g);
            // The gap is seven samples of 120: about a sixteenth of the
            // width, at 38 to 44 percent of it.
            let width = g.width;
            assert!(start - end > width * 0.05, "{key}: {end} {start}");
            assert!(
                end > width * 0.35 && start < width * 0.47,
                "{key}: {end} {start} {width}"
            );
            for (index, line) in g.lines.iter().enumerate() {
                for pair in line.windows(2) {
                    // Time goes on inside a stroke, and no stroke has a
                    // point in the gap.
                    assert!(pair[0][0] < pair[1][0]);
                }
                for point in line {
                    assert!(
                        if index == 0 {
                            point[0] <= end
                        } else {
                            point[0] >= start
                        },
                        "{key}: {point:?}"
                    );
                    // The line stays inside the plot, with room for half
                    // its width.
                    assert!(point[0] >= charts::INSET && point[0] <= g.width - charts::INSET);
                    assert!(point[1] >= charts::INSET && point[1] <= g.height - charts::INSET);
                }
            }
            assert_eq!(g.grid.len(), 3);
        }
        // The constant series is a flat line at the height of the top
        // grid line: its value is the top edge's.
        let memory = painted(cx, &scene, KEYS[1]).expect("memory").geometry;
        assert!(
            memory
                .lines
                .iter()
                .flatten()
                .all(|point| point[1] == charts::INSET)
        );
    }
}

// Seam test 3: a sample with a gap on both sides is a visible mark.
#[gpui_kit::test]
fn a_sample_between_two_gaps_is_a_mark(cx: &mut TestAppContext) {
    let scene = open_scene(cx, WIDE);
    run(cx, &scene, |m| {
        set_samples(m, &[Some(1.0), None, Some(0.25)])
    });
    let chart = chart(cx, &scene, KEYS[0]);
    let plot = chart.plot.clone().expect("a plot");
    assert_eq!(
        plot.segments.iter().map(Vec::len).collect::<Vec<_>>(),
        [1, 1]
    );
    assert!(chart.summary.ends_with("2 valid, 1 missing samples"));
    let painted = painted(cx, &scene, KEYS[0]).expect("painted");
    let g = painted.geometry;
    assert!(g.lines.is_empty());
    assert_eq!(g.marks.len(), 2);
    // The first at the left edge and the top value, the second at the
    // right edge, three quarters of the way down: each whole in the plot.
    let half = charts::MARK / 2.0;
    assert_eq!(g.marks[0], [half, half]);
    assert_eq!(g.marks[1][0], g.width - half);
    assert!((g.marks[1][1] - (half + 0.75 * (g.height - 2.0 * half))).abs() < 0.01);
    on(cx, &scene, |window, _| {
        assert_eq!(
            part(window, KEYS[0], "").label(),
            Some(chart.label.as_str())
        );
        // The other series are gone with the snapshot: so are their cards.
        assert!(!has(window, KEYS[1], ""));
    });
}

// Seam test 4: a series without a finite sample says so and has no plot.
#[gpui_kit::test]
fn a_series_without_a_finite_sample_says_so_and_has_no_plot(cx: &mut TestAppContext) {
    for size in SIZES {
        let scene = open_scene(cx, size);
        assert!(painted(cx, &scene, KEYS[0]).is_some());
        run(cx, &scene, |m| {
            set_samples(m, &[None, Some(f64::NAN), None])
        });
        let chart = chart(cx, &scene, KEYS[0]);
        assert!(chart.plot.is_none());
        on(cx, &scene, |window, _| {
            let card = part(window, KEYS[0], "");
            assert_eq!(card.role(), Some(Role::Figure));
            assert_eq!(card.label(), Some(chart.label.as_str()));
            assert!(chart.label.contains("Latest: Unavailable"));
            assert_eq!(
                part(window, KEYS[0], charts::TITLE).label(),
                Some(chart.title.as_str())
            );
            assert_eq!(
                part(window, KEYS[0], charts::SUMMARY).label(),
                Some(chart.summary.as_str())
            );
            let empty = part(window, KEYS[0], charts::EMPTY);
            assert_eq!(empty.label(), Some(area::NO_SAMPLES));
            assert!(empty.bounds().size.height > px(0.));
            for suffix in [
                charts::PLOT,
                charts::HIGH,
                charts::LOW,
                charts::FIRST,
                charts::LAST,
            ] {
                assert!(!has(window, KEYS[0], suffix), "{suffix}");
            }
        });
        // Nothing was painted for it, and the record of the plot that was
        // there before is gone.
        assert_eq!(painted(cx, &scene, KEYS[0]), None);
    }
}

// The state the model's own descriptions never reach: a chart block whose
// data is not there. While the view waits for its first snapshot the card
// says so; a block of a series that is gone draws nothing.
#[gpui_kit::test]
fn a_chart_whose_data_has_not_arrived_says_it_is_loading(cx: &mut TestAppContext) {
    let mut setup = performance();
    let block = |index: usize, source: &str| {
        Block::Custom(Custom {
            name: area::CHART.into(),
            key: Some(format!("metrics-chart-{index}")),
            params: json!({ "index": index, "source": source }),
        })
    };
    setup.describe = Some(Describe {
        tab_body: Rc::new(move |_, _| {
            Some(TabBody::Blocks(vec![block(0, "cpu0"), block(9, "gone")]))
        }),
        ..Describe::default()
    });
    let view = setup
        .model
        .areas
        .performance
        .metrics
        .as_mut()
        .expect("a view");
    let snapshot = view.snapshot.take().expect("a snapshot");
    let scene = open(cx, setup, &options(WIDE, ThemeMode::Dark));
    let text = "Loading performance data…";
    on(cx, &scene, |window, _| {
        for (key, source) in [("metrics-chart-0", "cpu0"), ("metrics-chart-9", "gone")] {
            let card = part(window, key, "");
            assert_eq!(card.role(), Some(Role::Figure));
            assert_eq!(
                card.label(),
                Some(format!("{source} chart. {text}").as_str())
            );
            assert_eq!(part(window, key, charts::TITLE).label(), Some(source));
            assert_eq!(part(window, key, charts::LOADING).label(), Some(text));
            assert!(!has(window, key, charts::PLOT) && !has(window, key, charts::EMPTY));
        }
    });
    assert_eq!(painted(cx, &scene, KEYS[0]), None);
    // The snapshot arrives: the next render draws the chart, and the block
    // of a series the snapshot does not have draws nothing.
    run(cx, &scene, move |m| {
        m.areas
            .performance
            .metrics
            .as_mut()
            .expect("a view")
            .snapshot = Some(snapshot);
    });
    on(cx, &scene, |window, _| {
        assert!(has(window, KEYS[0], charts::PLOT));
        assert!(!has(window, KEYS[0], charts::LOADING));
        let gone = part(window, "metrics-chart-9", "");
        assert_eq!(gone.bounds().size.height, px(0.));
        assert!(!has(window, "metrics-chart-9", charts::TITLE));
        assert!(!has(window, "metrics-chart-9", charts::LOADING));
    });
    assert!(painted(cx, &scene, KEYS[0]).is_some());
}

// Seam test 5: the charts follow the model. Another range draws the new
// snapshot; the source picker replaces the charts, and applying one source
// leaves one card.
#[gpui_kit::test]
fn a_change_of_the_range_or_of_the_sources_draws_the_charts_again(cx: &mut TestAppContext) {
    let scene = open_scene(cx, WIDE);
    let before = chart(cx, &scene, KEYS[0]).plot.expect("a plot");
    on(cx, &scene, |window, cx| {
        window.click("metrics-window-604800", cx)
    });
    on(cx, &scene, |window, _| {
        let range = window.find("metrics-range");
        assert!(
            range
                .label()
                .is_some_and(|label| label.contains("3600 second samples")),
            "{:?}",
            range.label()
        );
    });
    let week = chart(cx, &scene, KEYS[0]);
    let after = painted(cx, &scene, KEYS[0]).expect("painted");
    // The synthetic week has the shape of the ten minutes: what tells
    // them apart is the time the line starts at.
    assert_ne!(week.plot.as_ref().expect("a plot").first, before.first);
    assert_eq!(
        after.geometry,
        charts::geometry(
            week.plot.as_ref().expect("a plot"),
            after.bounds.size.width.as_f32(),
            charts::PLOT_HEIGHT
        )
    );
    on(cx, &scene, |window, _| {
        let plot = week.plot.as_ref().expect("a plot");
        assert_eq!(
            part(window, KEYS[0], charts::FIRST).label(),
            Some(plot.first.as_str())
        );
        assert_eq!(part(window, KEYS[0], "").label(), Some(week.label.as_str()));
    });

    // The picker stands in the place of the charts.
    invoke(cx, &scene, "performance.data-sources");
    on(cx, &scene, |window, _| {
        assert!(window.try_find("apply-metric-sources").is_some());
        for key in KEYS {
            assert!(!has(window, key, ""), "{key}");
        }
    });
    for key in KEYS {
        assert_eq!(painted(cx, &scene, key), None, "{key}");
    }
    for source in ["memory", "vbd_xvda_read", "vif_0_rx"] {
        on(cx, &scene, |window, cx| {
            window.click(id("metrics-source-", source), cx);
        });
    }
    on(cx, &scene, |window, cx| {
        window.click("apply-metric-sources", cx)
    });
    let charts_now = model(cx, &scene, area::charts);
    assert_eq!(charts_now.len(), 1);
    assert_eq!(charts_now[0].title, "cpu0");
    on(cx, &scene, |window, _| {
        assert_eq!(
            part(window, KEYS[0], "").label(),
            Some(charts_now[0].label.as_str())
        );
        for key in &KEYS[1..] {
            assert!(!has(window, key, ""), "{key}");
        }
    });
    assert!(painted(cx, &scene, KEYS[0]).is_some());
    for key in &KEYS[1..] {
        assert_eq!(painted(cx, &scene, key), None, "{key}");
    }
}

// Seam test 6: leaving the tab takes the charts and what they kept away;
// coming back draws them from the view the model made again.
#[gpui_kit::test]
fn leaving_the_tab_and_coming_back_draws_the_charts_again(cx: &mut TestAppContext) {
    let scene = open_scene(cx, WIDE);
    let before: Vec<Painted> = KEYS
        .iter()
        .map(|key| painted(cx, &scene, key).expect("painted"))
        .collect();
    run(cx, &scene, |m| m.set_active_tab(TabId::General));
    assert!(model(cx, &scene, |m| m.areas.performance.metrics.is_none()));
    on(cx, &scene, |window, _| {
        for key in KEYS {
            assert!(!has(window, key, "") && !has(window, key, charts::PLOT));
        }
    });
    for key in KEYS {
        assert_eq!(painted(cx, &scene, key), None, "{key}");
    }
    run(cx, &scene, |m| m.set_active_tab(TabId::Performance));
    assert_eq!(model(cx, &scene, area::charts).len(), 4);
    for (key, before) in KEYS.iter().zip(&before) {
        let chart = chart(cx, &scene, key);
        let again = painted(cx, &scene, key).unwrap_or_else(|| panic!("{key} not painted"));
        assert_eq!(again.bounds, before.bounds, "{key}");
        assert_eq!(
            again.geometry,
            charts::geometry(
                chart.plot.as_ref().expect("a plot"),
                again.bounds.size.width.as_f32(),
                charts::PLOT_HEIGHT
            ),
            "{key}"
        );
        assert_eq!(again.geometry.lines.len(), 2, "{key}");
    }
}

// Seam test 7: the line is the accent and the grid the border colour of
// the theme in use.
#[gpui_kit::test]
fn the_line_is_the_accent_and_the_grid_the_border_of_the_theme_in_use(cx: &mut TestAppContext) {
    let mut strokes = Vec::new();
    for mode in [ThemeMode::Dark, ThemeMode::Light] {
        let scene = open(cx, performance(), &options(WIDE, mode));
        let (accent, border, background) = cx.update(|cx| {
            let theme = cx.theme();
            assert_eq!(theme.mode, mode);
            (theme.primary, theme.border, theme.background)
        });
        for key in KEYS {
            let painted = painted(cx, &scene, key).expect("painted");
            assert_eq!(painted.stroke, accent, "{key} {mode:?}");
            assert_eq!(painted.grid, border, "{key} {mode:?}");
        }
        assert_ne!(accent, background);
        assert_ne!(border, background);
        strokes.push((accent, border));
    }
    assert_ne!(
        strokes[0], strokes[1],
        "the two themes have colours of their own"
    );
}

// ------------------------------------------------------------- the pixels

/// A captured frame of the scene's model and the chart of its first
/// series. `None` without a graphics adapter.
fn capture(setup: Setup, options: &Options) -> Option<(image::RgbaImage, Value)> {
    capture_scrolled(setup, options, &[])
}

/// [`capture`] after steps of scrolling: a chart below the fold.
fn capture_scrolled(
    setup: Setup,
    options: &Options,
    scrolls: &[scenes::Scroll],
) -> Option<(image::RgbaImage, Value)> {
    match scenes::render_scrolled("performance", setup, options, scrolls, true) {
        Ok(output) => Some((output.image.expect("a frame"), output.tree)),
        Err(error) => {
            eprintln!("no frame was captured, the pixels were not checked: {error}");
            None
        }
    }
}

/// The bounds of an element of the tree: x, y, width, height.
fn bounds_of(tree: &Value, id: &str) -> [f32; 4] {
    let element = tree["elements"]
        .as_array()
        .expect("elements")
        .iter()
        .find(|element| element["id"] == id)
        .unwrap_or_else(|| panic!("{id} is not in the tree"));
    let bounds = element["bounds"].as_array().expect("bounds");
    std::array::from_fn(|index| bounds[index].as_f64().expect("a number") as f32)
}

/// The columns of the plot in which a row that is not a grid line has
/// ink: a pixel that is not the background. With the bluest ink found.
fn inked_columns(
    frame: &image::RgbaImage,
    [left, top, width, height]: [f32; 4],
    grid: &[f32],
) -> (Vec<bool>, [u8; 4]) {
    let (left, top) = (left as u32, top as u32);
    let (width, height) = (width as u32, height as u32);
    // The middle of the gap, between two grid lines.
    let background = frame.get_pixel(left + width * 41 / 100, top + 30).0;
    let differs =
        |pixel: [u8; 4]| (0..3).any(|channel| pixel[channel].abs_diff(background[channel]) > 24);
    let blue = |pixel: [u8; 4]| i32::from(pixel[2]) - i32::from(pixel[0]);
    let mut bluest = background;
    let columns = (0..width)
        .map(|x| {
            (0..height)
                .filter(|y| grid.iter().all(|line| (*y as f32 - line).abs() > 1.0))
                .any(|y| {
                    let pixel = frame.get_pixel(left + x, top + y).0;
                    if differs(pixel) && blue(pixel) > blue(bluest) {
                        bluest = pixel;
                    }
                    differs(pixel)
                })
        })
        .collect();
    (columns, bluest)
}

// Seam test 2, on the frame: no ink in the columns of the gap but the grid
// lines, and ink on both sides of it, in both themes.
#[test]
fn the_frame_has_no_ink_in_the_gap_but_the_grid() {
    for mode in [ThemeMode::Dark, ThemeMode::Light] {
        let setup = performance();
        let chart = area::chart(&setup.model, KEYS[0]).expect("the chart");
        let Some((frame, tree)) = capture(setup, &options(WIDE, mode)) else {
            return;
        };
        let bounds = bounds_of(&tree, "metrics-chart-0-plot");
        let g = charts::geometry(chart.plot.as_ref().expect("a plot"), bounds[2], bounds[3]);
        let (end, start) = gap(&g);
        let (columns, ink) = inked_columns(&frame, bounds, &g.grid);
        let inked = |from: f32, to: f32| {
            columns[from.ceil() as usize..=to.floor() as usize]
                .iter()
                .filter(|inked| **inked)
                .count()
        };
        // No column of the gap has ink. The columns of the two strokes
        // have, but where a stroke runs along a grid line or crosses one.
        assert_eq!(inked(end + 3.0, start - 3.0), 0, "{mode:?}");
        let first = g.lines[0][0][0];
        let last = g.lines[1].last().expect("a point")[0];
        for (from, to) in [(first + 2.0, end - 2.0), (start + 2.0, last - 2.0)] {
            let columns = to.floor() as usize - from.ceil() as usize + 1;
            let inked = inked(from, to);
            assert!(inked * 5 > columns * 4, "{mode:?}: {inked} of {columns}");
        }
        // Each stroke goes on to its last sample: the gap starts there.
        assert!(inked(end - 4.0, end - 1.0) > 0 && inked(start + 1.0, start + 4.0) > 0);
        // The ink is the accent: a blue.
        assert!(ink[2] > ink[0].saturating_add(60), "{mode:?}: {ink:?}");
    }
}

// A constant series at the highest value lies on the top grid line. In its
// gap that grid line is left out: no row of the plot has ink there, so the
// gap reads as a gap and not as a dim continuation of the series.
#[test]
fn the_gap_of_a_constant_series_at_the_top_edge_has_no_ink_at_all() {
    for mode in [ThemeMode::Dark, ThemeMode::Light] {
        let setup = performance();
        // The scene's second chart is the constant `memory` series.
        let chart = area::chart(&setup.model, KEYS[1]).expect("the chart");
        let reveal = [scenes::Scroll::Reveal("metrics-chart-1-plot".into())];
        let Some((frame, tree)) = capture_scrolled(setup, &options(WIDE, mode), &reveal) else {
            return;
        };
        let bounds = bounds_of(&tree, "metrics-chart-1-plot");
        let g = charts::geometry(chart.plot.as_ref().expect("a plot"), bounds[2], bounds[3]);
        let (end, start) = gap(&g);
        assert_eq!(g.lines[0][0][1], g.lines[1][0][1], "a constant series");
        assert!(
            g.breaks
                .iter()
                .any(|b| b.top == g.grid[0] && b.from <= end + 3.0 && b.to >= start - 3.0),
            "{mode:?}: {:?}",
            g.breaks
        );
        // Every row counts here, the grid lines too: the two other lines
        // are the only ink in the columns of the gap.
        let (left, top) = (bounds[0] as u32, bounds[1] as u32);
        let background = frame
            .get_pixel(left + bounds[2] as u32 * 41 / 100, top + 30)
            .0;
        let differs =
            |pixel: [u8; 4]| (0..3).any(|channel| pixel[channel].abs_diff(background[channel]) > 8);
        let rows_with_ink = |x: u32| -> Vec<u32> {
            (0..bounds[3] as u32)
                .filter(|y| differs(frame.get_pixel(left + x, top + y).0))
                .collect()
        };
        for x in (end + 4.0).ceil() as u32..=(start - 4.0).floor() as u32 {
            assert_eq!(
                rows_with_ink(x),
                [g.grid[1] as u32, g.grid[2] as u32],
                "{mode:?}: column {x} of the gap"
            );
        }
        // Outside the gap the series is drawn on the top line.
        let beside = rows_with_ink((end - 8.0) as u32);
        assert!(beside.contains(&(g.grid[0] as u32)), "{mode:?}: {beside:?}");
    }
}

// Seam test 3, on the frame: the two samples around a missing one are two
// dots, with nothing between them.
#[test]
fn the_frame_has_a_dot_for_a_sample_between_two_gaps() {
    let mut setup = performance();
    set_samples(&mut setup.model, &[Some(1.0), None, Some(0.25)]);
    let chart = area::chart(&setup.model, KEYS[0]).expect("the chart");
    let Some((frame, tree)) = capture(setup, &options(WIDE, ThemeMode::Dark)) else {
        return;
    };
    let bounds = bounds_of(&tree, "metrics-chart-0-plot");
    let g = charts::geometry(chart.plot.as_ref().expect("a plot"), bounds[2], bounds[3]);
    assert_eq!(g.marks.len(), 2);
    let (columns, ink) = inked_columns(&frame, bounds, &g.grid);
    let width = columns.len();
    let mark = charts::MARK as usize;
    assert!(columns[..mark].iter().any(|inked| *inked), "the first dot");
    assert!(
        columns[width - mark..].iter().any(|inked| *inked),
        "the last dot"
    );
    assert!(
        columns[mark + 2..width - mark - 2]
            .iter()
            .all(|inked| !*inked),
        "nothing joins the two samples"
    );
    assert!(ink[2] > ink[0].saturating_add(60), "{ink:?}");
}
