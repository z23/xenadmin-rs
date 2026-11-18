//! The scene `large-inventory` (16 hosts, 2,000 guests) in the whole window.
//!
//! - `the_work_of_a_frame_does_not_grow_with_the_inventory` is a check: a
//!   frame, a turn of the wheel and a selection change build the rows in
//!   view and never every row; a replaced inventory gives the tree its rows
//!   once. The counts are `xenadmin_gpui::stats`. No adapter is needed.
//! - `measure_large_inventory` is a measurement, ignored: it prints the time
//!   of the first frame, of a scroll and of a selection change in the tree
//!   and in the Search grid, and of a replaced inventory, with the frame
//!   drawn by the real offscreen renderer. It asserts nothing about speed.
//!   `cargo test --release -p xenadmin-gpui --features scenes --locked --test
//!   large_inventory -- --ignored --nocapture`.
//!
//! A frame here is `Window::render_frame`: every view rendered, laid out and
//! painted into a scene on the CPU, as the application does for a window
//! that changed. The adapter's share (drawing the scene and reading it back)
//! is timed apart, because the test window has no surface to present to.

mod common;

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

use common::{key_of, locate, under};
use xenadmin_gpui::kit::testing::{
    ElementSnapshot, HeadlessAppContext, TestEventExt as _, TestQueryExt as _, TestWindowExt as _,
};
use xenadmin_gpui::kit::*;
use xenadmin_gpui::scenes::{self, Options, ShellWindow};
use xenadmin_gpui::shell::nav::TREE_KEY;
use xenadmin_gpui::stats::{self, Counts};
use xenadmin_ui::{Model, ObjectKind, TabId, nav};

const SCENE: &str = "large-inventory";
const SEARCH: &str = "search-results";
const HOSTS: usize = 16;
const GUESTS: usize = 2000;
/// More rows than a grid or the tree shows at 1440x900 (12 and 22 when this
/// was written), and far fewer than the inventory has.
const ROWS_IN_VIEW: u64 = 100;
/// What an event may build: the rows in view a few times over (a frame goes
/// over the rows it shows more than once: 44 tree rows were counted for a
/// turn of the wheel and its frame), never the inventory.
const ROWS_FOR_AN_EVENT: u64 = GUESTS as u64 / 4;

struct Bench {
    cx: HeadlessAppContext,
    scene: ShellWindow,
}

impl Bench {
    /// Opens the scene, and says how long the window took to open (its
    /// views built and the first frame drawn, which opening does).
    fn open(capture: bool) -> (Self, f64) {
        let setup = scenes::setup(SCENE).expect("the scene is registered");
        let mut cx = scenes::headless(capture);
        let options = Options::default();
        let start = Instant::now();
        let scene = cx
            .update(|cx| scenes::open_shell(cx, setup, &options))
            .expect("shell window");
        cx.run_until_parked();
        let opened = ms(start);
        let mut bench = Self { cx, scene };
        bench.on(|window, _| window.set_scale_factor(1.));
        (bench, opened)
    }

    fn on<R>(&mut self, act: impl FnOnce(&mut Window, &mut App) -> R) -> R {
        self.cx
            .update_window(self.scene.window, |_, window, cx| act(window, cx))
            .expect("window")
    }

    /// One frame: what the last event caused is rendered, laid out and
    /// painted.
    fn frame(&mut self) {
        self.on(|window, cx| window.render_frame(cx));
        self.cx.run_until_parked();
    }

    fn settle(&mut self) {
        for _ in 0..3 {
            self.frame();
        }
    }

    /// An event and the frame that shows it, in milliseconds.
    fn step(&mut self, event: impl FnOnce(&mut Window, &mut App)) -> f64 {
        let start = Instant::now();
        self.on(event);
        self.cx.run_until_parked();
        self.frame();
        ms(start)
    }

    /// The adapter draws the last frame and the picture is read back.
    fn raster(&mut self) -> f64 {
        let start = Instant::now();
        self.cx
            .capture_screenshot(self.scene.window)
            .expect("the frame is drawn by the offscreen renderer");
        ms(start)
    }

    fn model<R>(&mut self, read: impl FnOnce(&Model) -> R) -> R {
        let store = self.scene.store.clone();
        self.cx.update(|cx| read(store.read(cx).model()))
    }

    /// What is selected, as text to compare.
    fn selected(&mut self) -> String {
        self.model(|model| format!("{:?}", model.selection))
    }

    /// A change of the model, without the frame.
    fn run(&mut self, change: impl FnOnce(&mut Model)) {
        let store = self.scene.store.clone();
        self.cx
            .update(|cx| store.update(cx, |store, cx| store.run(change, cx)));
        self.cx.run_until_parked();
    }

    fn centre(&mut self, key: &'static str) -> Point<Pixels> {
        self.on(|window, _| {
            locate(window, None, key)
                .unwrap_or_else(|error| panic!("{error}"))
                .bounds()
                .center()
        })
    }

    /// The pointer goes to a point and the frame shows it there.
    fn point(&mut self, at: Point<Pixels>) {
        self.on(|window, cx| window.pointer_move(at, None, cx));
        self.frame();
    }

    /// The rows of the tree that stand for a guest and are wholly in view,
    /// from the top.
    fn tree_rows(&mut self) -> Vec<ElementSnapshot> {
        let guests: HashSet<String> = self.model(|model| {
            nav::rows(model)
                .into_iter()
                .filter(|row| row.icon == ObjectKind::Vm && row.object.is_some())
                .map(|row| row.id)
                .collect()
        });
        self.rows_in(TREE_KEY, |element| {
            key_of(element).is_some_and(|key| guests.contains(key))
        })
    }

    /// The rows of the Search grid that are wholly in view, from the top.
    fn grid_rows(&mut self) -> Vec<ElementSnapshot> {
        self.rows_in(SEARCH, |element| element.role() == Some(Role::Row))
    }

    fn rows_in(
        &mut self,
        scope: &'static str,
        is_row: impl Fn(&ElementSnapshot) -> bool,
    ) -> Vec<ElementSnapshot> {
        self.on(|window, _| {
            let seen = locate(window, None, scope)
                .unwrap_or_else(|error| panic!("{error}"))
                .bounds();
            let mut rows: Vec<_> = window
                .elements()
                .into_iter()
                .filter(|element| {
                    let bounds = element.bounds();
                    under(element, scope)
                        && is_row(element)
                        && bounds.top() >= seen.top()
                        && bounds.bottom() <= seen.bottom()
                })
                .collect();
            rows.sort_by(|left, right| {
                left.bounds()
                    .top()
                    .partial_cmp(&right.bounds().top())
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            rows
        })
    }

    /// The Search tab of the pool, with the grid's rows drawn.
    fn show_search(&mut self) {
        let tab = self.centre(TabId::Search.key());
        self.on(|window, cx| common::click_at(window, cx, tab, MouseButton::Left));
        self.cx.run_until_parked();
        self.settle();
        assert_eq!(self.model(Model::active_tab), TabId::Search);
        assert!(
            self.grid_rows().len() > 4,
            "the Search grid shows its rows at 1440x900"
        );
    }
}

fn ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.
}

fn keys(rows: &[ElementSnapshot]) -> Vec<String> {
    rows.iter()
        .map(|row| key_of(row).unwrap_or_default().to_owned())
        .collect()
}

/// The centre of the row with a key, which must be in view.
#[track_caller]
fn centre_of(rows: &[ElementSnapshot], key: &str) -> Point<Pixels> {
    rows.iter()
        .find(|row| key_of(row) == Some(key))
        .unwrap_or_else(|| panic!("the row `{key}` is not in view"))
        .bounds()
        .center()
}

/// A turn of the wheel at a point: `down` pixels down. No frame.
fn wheel(window: &mut Window, cx: &mut App, at: Point<Pixels>, down: f32) {
    let event = ScrollEvent {
        position: at,
        delta: ScrollDelta::Pixels(point(px(0.), px(-down))),
        ..Default::default()
    };
    window.dispatch_event(gpui_kit::InputEvent::to_platform_input(event), cx);
}

/// A press and a release of the primary button. No frame: the Kit's
/// `pointer_down` and `pointer_up` draw two each, which is not what a
/// press costs.
fn click(window: &mut Window, cx: &mut App, at: Point<Pixels>) {
    let down = MouseDownEvent {
        button: MouseButton::Left,
        position: at,
        modifiers: window.modifiers(),
        click_count: 1,
        first_mouse: false,
    };
    window.dispatch_event(gpui_kit::InputEvent::to_platform_input(down), cx);
    let up = MouseUpEvent {
        button: MouseButton::Left,
        position: at,
        modifiers: window.modifiers(),
        click_count: 1,
    };
    window.dispatch_event(gpui_kit::InputEvent::to_platform_input(up), cx);
}

/// The inventory again, with one guest named otherwise: what a refresh
/// that found a change gives the model.
fn replace_inventory(model: &mut Model, turn: usize) {
    let mut inventory = model
        .inventory
        .take()
        .expect("an inventory")
        .as_ref()
        .clone();
    let guest = inventory
        .vms
        .values_mut()
        .find(|vm| !vm.is_control_domain && !vm.is_template)
        .expect("a guest");
    guest.name = format!("renamed {turn}");
    inventory.revision += 1;
    model.inventory = Some(Arc::new(inventory));
}

#[test]
fn the_work_of_a_frame_does_not_grow_with_the_inventory() {
    let (mut bench, _) = Bench::open(false);
    assert_eq!(
        bench.model(|model| {
            let inventory = model.inventory.as_ref().expect("an inventory");
            (inventory.hosts.len(), inventory.guests().count())
        }),
        (HOSTS, GUESTS)
    );
    let tree_rows = bench.model(|model| nav::rows(model).len());
    assert!(tree_rows > GUESTS, "the tree has a row for every guest");
    bench.show_search();

    // A frame in which nothing changed.
    stats::take();
    bench.frame();
    let still = stats::take();
    println!("a frame in which nothing changed: {still:?}");
    assert!(still.tree_rows > 0 && still.grid_rows > 0, "{still:?}");
    assert!(
        still.tree_rows <= ROWS_IN_VIEW && still.grid_rows <= ROWS_IN_VIEW,
        "a frame builds the rows in view, not the inventory: {still:?}"
    );
    assert_eq!((still.tree_rebuilds, still.grid_reindexes), (0, 0));

    // A turn of the wheel over the tree, and over the grid.
    for key in [TREE_KEY, SEARCH] {
        let at = bench.centre(key);
        bench.point(at);
        let before = (keys(&bench.tree_rows()), keys(&bench.grid_rows()));
        stats::take();
        bench.step(|window, cx| wheel(window, cx, at, 600.));
        let scrolled = stats::take();
        let after = (keys(&bench.tree_rows()), keys(&bench.grid_rows()));
        assert_ne!(before, after, "the wheel over `{key}` scrolled its rows");
        within_view(scrolled, &format!("a scroll of `{key}`"));
    }

    // Another row of the grid is selected.
    let rows = bench.grid_rows();
    let at = rows[2].bounds().center();
    bench.point(at);
    let before = bench.selected();
    stats::take();
    bench.step(|window, cx| click(window, cx, at));
    bench.frame();
    let selected = stats::take();
    assert_ne!(
        before,
        bench.selected(),
        "the press selected a row of the grid"
    );
    within_view(selected, "a selection change in the grid");

    // Another row of the tree is selected.
    let rows = bench.tree_rows();
    assert!(rows.len() > 4, "the tree shows guests");
    let at = rows[3].bounds().center();
    bench.point(at);
    let before = bench.selected();
    stats::take();
    bench.step(|window, cx| click(window, cx, at));
    bench.frame();
    let selected = stats::take();
    assert_ne!(
        before,
        bench.selected(),
        "the press selected a row of the tree"
    );
    within_view(selected, "a selection change in the tree");

    // An inventory that says the same gives the tree nothing to do.
    stats::take();
    bench.run(|model| {
        let inventory = model.inventory.take().expect("an inventory");
        model.inventory = Some(Arc::new(inventory.as_ref().clone()));
    });
    bench.frame();
    bench.frame();
    let same = stats::take();
    within_view(same, "an inventory that says the same");

    // One that differs gives the tree its rows once, not once a frame.
    bench.run(|model| replace_inventory(model, 1));
    bench.frame();
    bench.frame();
    bench.frame();
    let replaced = stats::take();
    println!("an inventory that differs: {replaced:?}");
    assert_eq!(replaced.tree_rebuilds, 1, "{replaced:?}");
    assert!(replaced.grid_reindexes <= 1, "{replaced:?}");
    assert!(
        replaced.tree_rows <= ROWS_FOR_AN_EVENT && replaced.grid_rows <= ROWS_FOR_AN_EVENT,
        "{replaced:?}"
    );
}

/// The rows in view were built, a few times over, and no list was given all
/// its rows again.
#[track_caller]
fn within_view(counts: Counts, what: &str) {
    println!("{what}: {counts:?}");
    assert_eq!(
        (counts.tree_rebuilds, counts.grid_reindexes),
        (0, 0),
        "{what} takes no list's rows again: {counts:?}"
    );
    assert!(
        counts.tree_rows <= ROWS_FOR_AN_EVENT && counts.grid_rows <= ROWS_FOR_AN_EVENT,
        "{what} builds the rows in view, not the inventory: {counts:?}"
    );
}

/// The median, the 95th percentile and the worst of the samples.
fn spread(mut samples: Vec<f64>) -> (f64, f64, f64, usize) {
    samples.sort_by(f64::total_cmp);
    let count = samples.len();
    let at = |part: f64| samples[((count - 1) as f64 * part).round() as usize];
    (at(0.5), at(0.95), samples[count - 1], count)
}

fn report(label: &str, samples: Vec<f64>) {
    let (p50, p95, worst, count) = spread(samples);
    println!("{label}: p50 {p50:.2} ms, p95 {p95:.2} ms, worst {worst:.2} ms over {count}");
}

#[test]
#[ignore = "a measurement, not a check"]
fn measure_large_inventory() {
    const FRAMES: usize = 200;
    println!(
        "scene {SCENE}: {HOSTS} hosts, {GUESTS} guests, 1440x900, dark, scale 1, {} build",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
    );
    let start = Instant::now();
    drop(scenes::setup(SCENE).expect("the scene is registered"));
    println!(
        "build the model of the scene (not part of a frame): {:.2} ms",
        ms(start)
    );

    let (mut bench, opened) = Bench::open(true);
    println!("open the window, first frame included (views built, layout, paint): {opened:.2} ms");
    let start = Instant::now();
    bench.frame();
    println!("second frame, the first at scale 1: {:.2} ms", ms(start));
    println!(
        "the adapter draws the first frame and it is read back: {:.2} ms",
        bench.raster()
    );
    bench.settle();

    let frames = (0..FRAMES)
        .map(|_| {
            let start = Instant::now();
            bench.frame();
            ms(start)
        })
        .collect();
    report("a frame in which nothing changed, pool selected", frames);
    let raster = (0..50).map(|_| bench.raster()).collect();
    report("the adapter's share of a frame, with the read-back", raster);

    // The tree: down by three rows a frame, then back up.
    let at = bench.centre(TREE_KEY);
    bench.point(at);
    let top = keys(&bench.tree_rows());
    let mut samples = Vec::new();
    for turn in 0..FRAMES {
        let down = if turn < FRAMES / 2 { 72. } else { -72. };
        samples.push(bench.step(|window, cx| wheel(window, cx, at, down)));
        if turn == FRAMES / 2 - 1 {
            assert_ne!(top, keys(&bench.tree_rows()), "the tree scrolled");
        }
    }
    report("scroll the tree, wheel event to frame", samples);

    // The tree: two guests in view, pressed in turn.
    let rows = keys(&bench.tree_rows());
    assert!(rows.len() > 4, "the tree shows guests");
    let pair = [rows[1].clone(), rows[3].clone()];
    let mut samples = Vec::new();
    for turn in 0..FRAMES {
        let key = pair[turn % 2].clone();
        let at = centre_of(&bench.tree_rows(), &key);
        bench.point(at);
        let before = bench.selected();
        samples.push(bench.step(|window, cx| click(window, cx, at)));
        assert_ne!(before, bench.selected());
        bench.frame();
    }
    println!(
        "(the tab of a selected guest is {:?})",
        bench.model(Model::active_tab)
    );
    report("select another guest in the tree, press to frame", samples);

    // The Search grid of the pool.
    bench.run(|model| model.load_demo(HOSTS, GUESTS));
    bench.settle();
    bench.show_search();
    let frames = (0..FRAMES)
        .map(|_| {
            let start = Instant::now();
            bench.frame();
            ms(start)
        })
        .collect();
    report("a frame in which nothing changed, Search tab", frames);
    let raster = (0..50).map(|_| bench.raster()).collect();
    report(
        "the adapter's share of that frame, with the read-back",
        raster,
    );

    let at = bench.centre(SEARCH);
    bench.point(at);
    let top = keys(&bench.grid_rows());
    let mut samples = Vec::new();
    for turn in 0..FRAMES {
        let down = if turn < FRAMES / 2 { 96. } else { -96. };
        samples.push(bench.step(|window, cx| wheel(window, cx, at, down)));
        if turn == FRAMES / 2 - 1 {
            assert_ne!(top, keys(&bench.grid_rows()), "the grid scrolled");
        }
    }
    report("scroll the Search grid, wheel event to frame", samples);

    // The first two rows: a selected row puts its details under the grid,
    // which then shows fewer rows.
    let rows = keys(&bench.grid_rows());
    let pair = [rows[0].clone(), rows[1].clone()];
    let mut samples = Vec::new();
    for turn in 0..FRAMES {
        let key = pair[turn % 2].clone();
        let at = centre_of(&bench.grid_rows(), &key);
        bench.point(at);
        let before = bench.selected();
        samples.push(bench.step(|window, cx| click(window, cx, at)));
        assert_ne!(before, bench.selected());
        bench.frame();
    }
    report(
        "select another row of the Search grid, press to frame",
        samples,
    );

    // What the model's share of an update is: the descriptions the front
    // end asks for again after every change (the Search tab is shown, a
    // row of it is selected).
    let store = bench.scene.store.clone();
    let mut parts: [Vec<f64>; 4] = Default::default();
    for _ in 0..50 {
        bench.cx.update(|cx| {
            let store = store.read(cx);
            let start = Instant::now();
            let rows = nav::rows(store.model());
            parts[0].push(ms(start));
            let start = Instant::now();
            drop(nav::selected_ids(store.model(), &rows));
            parts[1].push(ms(start));
            let start = Instant::now();
            drop((store.menu_bar(), store.toolbar()));
            parts[2].push(ms(start));
            let start = Instant::now();
            drop(store.tab_body(TabId::Search));
            parts[3].push(ms(start));
        });
    }
    let [rows, scope, menus, tab] = parts;
    report("  the model lists the rows of the tree (nav::rows)", rows);
    report("  the model finds the rows of the scope", scope);
    report("  the model resolves the menu bar and the toolbar", menus);
    report("  the model describes the Search tab", tab);

    // The inventory is replaced by one in which a guest has another name.
    let mut samples = Vec::new();
    for turn in 0..50 {
        let start = Instant::now();
        bench.run(|model| replace_inventory(model, turn));
        bench.frame();
        samples.push(ms(start));
        bench.frame();
    }
    report(
        "replace the inventory (one guest renamed), update to frame",
        samples,
    );
}
