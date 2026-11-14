//! Core-to-UI wake bridge.
//!
//! The core calls the waker from its own threads. Bursts collapse into one
//! wake. The receiver does not need a Tokio context.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};

/// What [`crate::inventory::ScaleModel::poll`] found. The spike only tracks
/// inventory freshness. Later phases add the other core handles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Changed {
    pub inventory: bool,
}

impl Changed {
    pub fn any(self) -> bool {
        self.inventory
    }
}

/// Coalesces core notifications into one signal for the foreground.
pub struct WakeBridge {
    pending: Arc<AtomicBool>,
    tx: Sender<()>,
    rx: Receiver<()>,
}

impl WakeBridge {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            pending: Arc::new(AtomicBool::new(false)),
            tx,
            rx,
        }
    }

    /// Callback suitable for `notify_on_change`.
    pub fn waker(&self) -> Arc<dyn Fn() + Send + Sync> {
        let pending = Arc::clone(&self.pending);
        let tx = self.tx.clone();
        Arc::new(move || {
            if !pending.swap(true, Ordering::AcqRel) {
                let _ = tx.send(());
            }
        })
    }

    /// Returns whether the core signaled since the previous poll.
    ///
    /// Clears the pending flag before the caller reads state, matching the
    /// bridge in the migration plan, and drops any extra signals that were
    /// queued for the same burst.
    pub fn poll_wake(&self) -> bool {
        if self.rx.try_recv().is_err() {
            return false;
        }
        self.pending.store(false, Ordering::Release);
        while self.rx.try_recv().is_ok() {}
        true
    }
}

impl Default for WakeBridge {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_burst_collapses_to_one_wake() {
        let bridge = WakeBridge::new();
        let waker = bridge.waker();
        for _ in 0..100 {
            waker();
        }
        assert!(bridge.poll_wake());
        assert!(!bridge.poll_wake());
    }

    #[test]
    fn a_wake_after_poll_is_delivered() {
        let bridge = WakeBridge::new();
        let waker = bridge.waker();
        waker();
        assert!(bridge.poll_wake());
        waker();
        assert!(bridge.poll_wake());
    }
}
