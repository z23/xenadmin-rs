//! Shared polling deadlines for the protocol and trust fixtures.
//!
//! A local run keeps the stated deadline. A slower shared runner sets
//! `XENADMIN_TEST_TIME_SCALE` to a small integer so waits that only poll for an
//! asynchronous publication tolerate CPU and disk contention. The scale never
//! changes what a fixture asserts or how many requests a mock accepts.
use std::time::Duration;

pub fn deadline(seconds: u64) -> Duration {
    let scale = std::env::var("XENADMIN_TEST_TIME_SCALE")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|scale| (1..=20).contains(scale))
        .unwrap_or(1);
    Duration::from_secs(seconds.saturating_mul(scale))
}
