//! The layout of a tab body that ends in a table: the Search tab. What
//! stands above the table (the query, its filters, the grouping) never
//! takes more than half of the body, so the grid keeps at least half of the
//! tab at the smallest window, with the filters open as well, and does not
//! move when a row is selected.

mod common;

use common::Step::*;
use common::{SMALL, WIDE, close, on, open_scene, run_step};
use xenadmin_gpui::kit::testing::{TestAppContext, TestQueryExt as _};
use xenadmin_gpui::kit::*;
use xenadmin_gpui::scene_view::{TAB_RESULTS_KEY, TAB_SCROLL_KEY};
use xenadmin_gpui::scenes::ShellWindow;

const SEARCH: &str = "search-results";

#[track_caller]
fn must(cx: &mut TestAppContext, scene: &ShellWindow, step: common::Step) {
    if let Err(error) = run_step(cx, scene, step) {
        panic!("{error}");
    }
}

fn bounds(cx: &mut TestAppContext, scene: &ShellWindow, key: &str) -> Bounds<Pixels> {
    on(cx, scene, |window, _| {
        common::locate(window, None, key)
            .unwrap_or_else(|error| panic!("{error}"))
            .bounds()
    })
}

/// The rows of the Search grid that are wholly inside the part of the tab
/// the grid is seen through.
fn rows_in_view(cx: &mut TestAppContext, scene: &ShellWindow) -> usize {
    let part = bounds(cx, scene, TAB_RESULTS_KEY);
    on(cx, scene, |window, _| {
        window
            .elements()
            .into_iter()
            .filter(|element| {
                element.role() == Some(Role::Row)
                    && common::under(element, SEARCH)
                    && element.bounds().top() >= part.top()
                    && element.bounds().bottom() <= part.bottom()
            })
            .count()
    })
}

/// The grid has at least half of the tab's body and all of it is in the
/// window. Returns the grid's bounds.
#[track_caller]
fn grid_has_half(cx: &mut TestAppContext, scene: &ShellWindow, when: &str) -> Bounds<Pixels> {
    let (above, results) = (
        bounds(cx, scene, TAB_SCROLL_KEY),
        bounds(cx, scene, TAB_RESULTS_KEY),
    );
    let body = above.size.height + results.size.height;
    assert!(
        results.size.height >= body / 2. - px(1.),
        "{when}: the grid's part is {:?} of {body:?}",
        results.size.height
    );
    assert_eq!(above.bottom(), results.top(), "{when}");
    let grid = bounds(cx, scene, SEARCH);
    assert!(
        grid.top() >= results.top() && grid.bottom() <= results.bottom(),
        "{when}: the grid {grid:?} is not inside {results:?}"
    );
    grid
}

#[gpui_kit::test]
fn the_search_grid_keeps_half_of_the_tab_at_the_smallest_window(cx: &mut TestAppContext) {
    let scene = open_scene(cx, "search-pools", SMALL);
    let grid = grid_has_half(cx, &scene, "as opened");
    assert!(rows_in_view(cx, &scene) >= 4, "{grid:?}");
    // The query's field and its buttons are in view without scrolling.
    must(&mut *cx, &scene, Inside(&["search-text"]));
    must(&mut *cx, &scene, Inside(&["search-apply"]));

    // The filters open above the grid: the grid stays where it is, and the
    // filters are reached by scrolling their own part.
    must(&mut *cx, &scene, Press("search-filters"));
    assert_eq!(grid_has_half(cx, &scene, "with the filters open"), grid);
    must(&mut *cx, &scene, Reach(&["search-apply-filters"]));
    assert_eq!(
        grid_has_half(cx, &scene, "after scrolling the filters"),
        grid
    );
    must(&mut *cx, &scene, Reach(&["search-filters"]));
    must(&mut *cx, &scene, Press("search-filters"));

    // A selected row adds a section under the grid. The grid does not jump.
    let row = on(cx, &scene, |window, _| {
        window
            .elements()
            .into_iter()
            .find(|element| element.role() == Some(Role::Row) && common::under(element, SEARCH))
            .map(|row| common::key_of(&row).unwrap_or_default().to_owned())
            .expect("a row")
    });
    on(cx, &scene, |window, cx| {
        common::press(window, cx, None, &row)
            .map(|_| ())
            .unwrap_or_else(|error| panic!("{error}"));
    });
    let selected = grid_has_half(cx, &scene, "with a row selected");
    assert_eq!(selected.top(), grid.top());
    assert!(rows_in_view(cx, &scene) >= 3, "{selected:?}");
    must(&mut *cx, &scene, Reach(&["search-result"]));
    close(cx, scene);
}

#[gpui_kit::test]
fn the_search_grid_fills_what_the_query_leaves_in_a_large_window(cx: &mut TestAppContext) {
    let scene = open_scene(cx, "search-pools", WIDE);
    let grid = grid_has_half(cx, &scene, "as opened");
    assert!(rows_in_view(cx, &scene) >= 8, "{grid:?}");
    // Nothing above the grid is cut: the grouping is in view.
    must(&mut *cx, &scene, Inside(&["search-group"]));
    close(cx, scene);
}

/// The path of the element with a key: where it is among the elements.
fn path(cx: &mut TestAppContext, scene: &ShellWindow, key: &str) -> Vec<ElementId> {
    on(cx, scene, |window, _| {
        common::locate(window, None, key)
            .unwrap_or_else(|error| panic!("{error}"))
            .path()
            .to_vec()
    })
}

#[gpui_kit::test]
fn a_short_table_is_as_high_as_its_rows_and_stays_the_same_element_when_a_row_is_selected(
    cx: &mut TestAppContext,
) {
    const TABLE: &str = "snapshots-table";
    let scene = open_scene(cx, "snapshots", SMALL);
    // Three snapshots: the table needs no half of the body, and all of it
    // is in the window under what stands above it.
    let (above, results) = (
        bounds(cx, &scene, TAB_SCROLL_KEY),
        bounds(cx, &scene, TAB_RESULTS_KEY),
    );
    assert!(results.size.height < above.size.height, "{results:?}");
    must(&mut *cx, &scene, Inside(&[TABLE]));
    let (before, at) = (path(cx, &scene, TABLE), bounds(cx, &scene, TABLE));

    // A selected row adds the snapshot's information after the table. The
    // table is the element it was (a menu opened by the same press would be
    // lost otherwise), it hardly moves, and the information is reached by
    // scrolling the body as one column.
    let row = on(cx, &scene, |window, _| {
        window
            .elements()
            .into_iter()
            .find(|element| element.role() == Some(Role::Row) && common::under(element, TABLE))
            .map(|row| common::key_of(&row).unwrap_or_default().to_owned())
            .expect("a row")
    });
    on(cx, &scene, |window, cx| {
        common::press(window, cx, None, &row)
            .map(|_| ())
            .unwrap_or_else(|error| panic!("{error}"));
    });
    assert_eq!(path(cx, &scene, TABLE), before);
    let moved = bounds(cx, &scene, TABLE).top() - at.top();
    assert!(moved.abs() <= px(12.), "the table moved by {moved:?}");
    must(&mut *cx, &scene, Reach(&["snapshot-information"]));
    must(&mut *cx, &scene, Reach(&[TABLE]));
    close(cx, scene);
}
