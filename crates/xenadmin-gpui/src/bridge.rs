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
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicU64;
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;

    fn block_on<T>(future: impl Future<Output = T>) -> T {
        gpui_kit::block_on(future)
    }

    #[test]
    fn a_burst_collapses_to_one_wake() {
        let (waker, receiver) = wake_channel();
        for _ in 0..1000 {
            waker();
        }
        assert!(block_on(receiver.woken()));
        // Nothing else is queued: the next wake is a new burst.
        assert!(receiver.rx.is_empty());
        waker();
        assert!(block_on(receiver.woken()));
        assert!(receiver.rx.is_empty());
    }

    #[test]
    fn the_bridge_ends_when_every_waker_is_gone() {
        let (waker, receiver) = wake_channel();
        waker();
        drop(waker);
        assert!(block_on(receiver.woken()));
        assert!(!block_on(receiver.woken()));
    }

    /// Producers race the receiver. The state changes before each wake, as
    /// in the core. Whatever the interleaving, the receiver must see the
    /// last value: a lost wake leaves it waiting for ever, which the timeout
    /// turns into a failure. The spike's bridge stuck in about 90 of 200
    /// such rounds.
    #[test]
    fn no_wake_is_lost_when_producers_race_the_receiver() {
        const ROUNDS: usize = 300;
        const PRODUCERS: u64 = 4;
        const WAKES: u64 = 500;
        for round in 0..ROUNDS {
            let (waker, receiver) = wake_channel();
            let state = Arc::new(AtomicU64::new(0));
            let producers: Vec<_> = (0..PRODUCERS)
                .map(|_| {
                    let waker = Arc::clone(&waker);
                    let state = Arc::clone(&state);
                    std::thread::spawn(move || {
                        for _ in 0..WAKES {
                            state.fetch_add(1, Ordering::AcqRel);
                            waker();
                        }
                    })
                })
                .collect();
            drop(waker);
            let (done, finished) = mpsc::channel();
            let consumer = {
                let state = Arc::clone(&state);
                std::thread::spawn(move || {
                    let mut polls = 0u64;
                    let mut seen = 0;
                    // Ends when the producers have dropped their wakers.
                    while block_on(receiver.woken()) {
                        seen = state.load(Ordering::Acquire);
                        polls += 1;
                    }
                    let _ = done.send((seen, polls));
                })
            };
            for producer in producers {
                producer.join().expect("producer");
            }
            let (seen, polls) = finished
                .recv_timeout(Duration::from_secs(20))
                .unwrap_or_else(|_| panic!("round {round}: the receiver is stuck"));
            consumer.join().expect("consumer");
            assert_eq!(seen, PRODUCERS * WAKES, "round {round}: a wake was lost");
            assert!(polls <= PRODUCERS * WAKES, "round {round}");
        }
    }
}
