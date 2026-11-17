//! The wake bridge from the core's threads to the foreground.
//!
//! These tests wait for the receiver on plain threads with the Kit's
//! `block_on`. They are here and not beside the bridge because nothing
//! under `src` may name `block_on`: the front end never blocks on a future
//! (`scripts/check.sh` checks it).

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use xenadmin_gpui::bridge::wake_channel;
use xenadmin_gpui::gpui_kit;

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
    assert!(receiver.idle());
    waker();
    assert!(block_on(receiver.woken()));
    assert!(receiver.idle());
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
