//! Spike S3: wake bridge, synthetic large inventory, tree and table.
//!
//! `XENADMIN_SPIKE_SECONDS` sets the update interval (default 60). The
//! updater touches the inventory 100 times a second. The foreground renders
//! only when the bridge delivers a wake.
//!
//! `Backend` is started on a temporary directory and never connects. No host
//! is contacted. Headless frames do not establish Wayland scrolling.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use gpui_kit::assets::Assets;
use gpui_kit::component::table::{Column, DataTable, TableDelegate, TableState};
use gpui_kit::component::tree::{TreeItem, TreeState, tree as navigation_tree};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{App, Entity, Window, WindowBounds, WindowOptions, div, point, px, size};

struct Rows {
    ids: Vec<String>,
    names: Vec<String>,
    revision: u64,
}

impl TableDelegate for Rows {
    fn columns_count(&self, _: &App) -> usize {
        2
    }

    fn rows_count(&self, _: &App) -> usize {
        self.ids.len()
    }

    fn column(&self, index: usize, _: &App) -> Column {
        Column::new(
            if index == 0 { "name" } else { "revision" },
            if index == 0 { "Name" } else { "Revision" },
        )
        .width(px(160.))
    }

    fn render_td(
        &mut self,
        row: usize,
        column: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        if column == 0 {
            div().child(self.names[row].clone())
        } else {
            div().child(self.revision.to_string())
        }
    }
}

struct ScaleView {
    table: Entity<TableState<Rows>>,
    tree: Entity<TreeState>,
}

impl Render for ScaleView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .size_full()
            .child(
                div()
                    .w(px(240.))
                    .child(navigation_tree(&self.tree, |_, entry, _, _, _| {
                        gpui_kit::component::list::ListItem::new(entry.item().id.clone())
                            .child(entry.item().label.clone())
                    })),
            )
            .child(
                v_flex()
                    .flex_1()
                    .child(div().h(px(480.)).child(DataTable::new(&self.table))),
            )
    }
}

fn rss_kib() -> u64 {
    let Ok(text) = std::fs::read_to_string("/proc/self/status") else {
        return 0;
    };
    text.lines()
        .find_map(|line| {
            let rest = line.strip_prefix("VmRSS:")?;
            rest.split_whitespace().next()?.parse().ok()
        })
        .unwrap_or(0)
}

fn percentile(samples: &[u128], pct: u128) -> u128 {
    if samples.is_empty() {
        return 0;
    }
    let mut ordered = samples.to_vec();
    ordered.sort_unstable();
    let index = ((ordered.len() - 1) as u128 * pct / 100) as usize;
    ordered[index]
}

fn main() {
    let seconds: u64 = std::env::var("XENADMIN_SPIKE_SECONDS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(60);
    let dir = std::env::temp_dir().join(format!("xenadmin-gpui-s3-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("spike directory");
    let backend = xenadmin_core::Backend::with_state_directory(dir.clone())
        .expect("core backend starts without a connection");

    let model = Arc::new(Mutex::new(xenadmin_gpui::ScaleModel::demonstration(
        16, 2_000,
    )));
    let selected = model.lock().expect("model").selected;
    let bridge = xenadmin_gpui::WakeBridge::new();
    let waker = bridge.waker();
    let running = Arc::new(AtomicBool::new(true));
    let worker = {
        let model = Arc::clone(&model);
        let running = Arc::clone(&running);
        thread::spawn(move || {
            while running.load(Ordering::Relaxed) {
                model.lock().expect("model").touch();
                waker();
                thread::sleep(Duration::from_millis(10));
            }
        })
    };

    let mut cx = gpui_kit::HeadlessAppContext::with_platform(
        gpui_kit::platform::current_platform(true).text_system(),
        std::sync::Arc::new(Assets),
        || Ok(None),
    );
    cx.update(gpui_kit::init);
    let (handle, view) = cx
        .update(|cx| {
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(gpui_kit::Bounds {
                        origin: point(px(0.), px(0.)),
                        size: size(px(1440.), px(900.)),
                    })),
                    focus: false,
                    show: false,
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    let snapshot = model.lock().expect("model");
                    let ids: Vec<_> = snapshot
                        .inventory
                        .vms
                        .keys()
                        .map(|id| id.to_string())
                        .collect();
                    let names: Vec<_> = snapshot
                        .inventory
                        .vms
                        .values()
                        .map(|vm| vm.name.clone())
                        .collect();
                    let hosts = snapshot
                        .inventory
                        .hosts
                        .values()
                        .map(|host| TreeItem::new(host.id.to_string(), host.name.clone()))
                        .collect::<Vec<_>>();
                    let table = cx.new(|cx| {
                        TableState::new(
                            Rows {
                                ids,
                                names,
                                revision: snapshot.inventory.revision,
                            },
                            window,
                            cx,
                        )
                    });
                    let tree = cx.new(|cx| {
                        TreeState::new(cx).items(vec![
                            TreeItem::new("pool", "Demonstration pool")
                                .expanded(true)
                                .children(hosts),
                        ])
                    });
                    cx.new(|_| ScaleView { table, tree })
                },
            )
        })
        .expect("open scale window");

    let rss_before = rss_kib();
    let started = Instant::now();
    let mut frames = 0u32;
    let mut frame_us = Vec::new();
    let mut selection_held = true;
    while started.elapsed() < Duration::from_secs(seconds) {
        if !bridge.poll_wake() {
            thread::sleep(Duration::from_millis(1));
            continue;
        }
        let (revision, still_selected) = {
            let current = model.lock().expect("model");
            (current.inventory.revision, current.selected_present())
        };
        selection_held &= still_selected;
        let frame = Instant::now();
        cx.update_window(handle, |_view, window, cx| {
            view.update(cx, |scale, cx| {
                scale.table.update(cx, |table, cx| {
                    table.delegate_mut().revision = revision;
                    cx.notify();
                });
            });
            window.render_frame(cx);
        })
        .expect("frame");
        frame_us.push(frame.elapsed().as_micros());
        frames += 1;
    }
    running.store(false, Ordering::Relaxed);
    worker.join().expect("updater");
    while bridge.poll_wake() {}
    let idle_started = Instant::now();
    let mut idle_frames = 0u32;
    while idle_started.elapsed() < Duration::from_millis(200) {
        if bridge.poll_wake() {
            idle_frames += 1;
        } else {
            thread::sleep(Duration::from_millis(5));
        }
    }
    let rss_after = rss_kib();
    drop(bridge);
    drop(backend);
    let _ = std::fs::remove_dir_all(&dir);

    println!(
        "seconds={seconds} frames={frames} frame_us_p50={} frame_us_p95={} selection_held={selection_held} selected={selected:?} idle_frames={idle_frames} rss_kib_before={rss_before} rss_kib_after={rss_after} shutdown=updater_stopped,bridge_dropped,backend_dropped",
        percentile(&frame_us, 50),
        percentile(&frame_us, 95),
    );
    if !selection_held || idle_frames != 0 {
        std::process::exit(1);
    }
}
