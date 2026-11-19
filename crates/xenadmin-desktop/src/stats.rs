//! Counts of the work that grows with an inventory: the rows the tree and
//! the grids make into elements, and the times they take all their rows
//! again.
//!
//! A frame draws the rows in view, whatever the inventory holds; only a
//! change of the rows themselves is work per row. The large-inventory test
//! (`tests/large_inventory.rs`) reads these counts to hold that: a selection
//! change or a turn of the wheel must not build two thousand rows. The
//! counts are per thread (the interface has one) and cost an addition.
//!
//! One count is not about size: the edits of a private field that reached
//! the model in a zeroizing copy, which is nothing a test can see otherwise.

use std::cell::Cell;

/// What was counted on this thread since the last [`take`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    /// Rows of the navigation tree made into elements.
    pub tree_rows: u64,
    /// Times the tree was given every row again.
    pub tree_rebuilds: u64,
    /// Rows of grids made into elements.
    pub grid_rows: u64,
    /// Times a grid indexed every row again, because its rows changed.
    pub grid_reindexes: u64,
    /// Edits of a private field handed to the model in a copy that is
    /// zeroized when it is dropped.
    pub secret_edits: u64,
}

thread_local! {
    static COUNTS: Cell<Counts> = const {
        Cell::new(Counts {
            tree_rows: 0,
            tree_rebuilds: 0,
            grid_rows: 0,
            grid_reindexes: 0,
            secret_edits: 0,
        })
    };
}

/// The counts since the last call, which start again from nothing.
pub fn take() -> Counts {
    COUNTS.with(Cell::take)
}

pub(crate) fn count(change: impl FnOnce(&mut Counts)) {
    COUNTS.with(|counts| {
        let mut now = counts.get();
        change(&mut now);
        counts.set(now);
    });
}
