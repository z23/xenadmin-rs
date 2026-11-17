//! Core-to-UI wake bridge.
//!
//! The core calls the waker from its own threads, any number of times. A
//! burst collapses into one message on a channel the foreground awaits, so
//! nothing polls per frame and the receiver needs no Tokio context.
//!
//! Why this shape is sound: a message is sent only when the pending flag
//! flips from false to true, so at most one message is queued per cleared
//! flag. The receiver clears the flag *before* the caller reads state and
//! never drains the channel afterwards. A wake that lands after the clear
//! queues a new message and causes one more poll; a wake that lands before
//! it is covered by the poll that follows, because the core changes its
//! state before it calls the waker.
//!
//! The spike this replaces drained the channel after clearing the flag. A
//! wake between the two flipped the flag and sent; the drain ate that
//! message; the flag then stayed true with an empty channel and every later
//! wake was dropped.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::kit::{Receiver, Sender, unbounded};

/// The callback type the core takes in `notify_on_change` and the model
/// takes in `Model::setup`.
pub type Waker = Arc<dyn Fn() + Send + Sync>;

/// The foreground end of the bridge.
pub struct WakeReceiver {
    rx: Receiver<()>,
    pending: Arc<AtomicBool>,
}

/// A waker for the core and the receiver the foreground awaits.
pub fn wake_channel() -> (Waker, WakeReceiver) {
    let (tx, rx): (Sender<()>, Receiver<()>) = unbounded();
    let pending = Arc::new(AtomicBool::new(false));
    let waker: Waker = {
        let pending = Arc::clone(&pending);
        Arc::new(move || {
            // Only the false -> true flip sends: a burst is one message.
            if !pending.swap(true, Ordering::AcqRel) {
                let _ = tx.try_send(());
            }
        })
    };
    (waker, WakeReceiver { rx, pending })
}

impl WakeReceiver {
    /// Resolves once per burst of wakes. Returns false when every waker is
    /// gone, which ends the bridge task.
    ///
    /// The flag is already cleared when this returns: read the state now,
    /// and do not drain the channel.
    pub async fn woken(&self) -> bool {
        if self.rx.recv().await.is_err() {
            return false;
        }
        self.pending.store(false, Ordering::Release);
        true
    }

    /// Whether no wake is waiting to be received. For tests of the bridge.
    pub fn idle(&self) -> bool {
        self.rx.is_empty()
    }
}
