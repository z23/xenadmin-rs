use crate::{Error, HostId, PoolId, VmId};
use serde::{Deserialize, Serialize};
use std::{sync::Arc, time::Duration};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

#[path = "metrics_decode.rs"]
mod decode;
#[path = "metrics_transport.rs"]
mod transport;
#[path = "metrics_worker.rs"]
mod worker;
pub(crate) use worker::MetricsRequest;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MetricTarget {
    Vm(VmId),
    Host(HostId),
}
impl MetricTarget {
    pub(crate) fn class(self) -> &'static str {
        match self {
            Self::Vm(_) => "VM",
            Self::Host(_) => "host",
        }
    }
    pub fn uuid(self) -> uuid::Uuid {
        match self {
            Self::Vm(id) => id.uuid(),
            Self::Host(id) => id.uuid(),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObservedMetrics {
    pub(crate) pool: PoolId,
    pub(crate) generation: u64,
    pub(crate) target: MetricTarget,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MetricsWindow {
    #[default]
    TenMinutes,
    TwoHours,
    OneWeek,
    OneYear,
}
impl MetricsWindow {
    pub fn duration_seconds(self) -> u64 {
        match self {
            Self::TenMinutes => 600,
            Self::TwoHours => 7_200,
            Self::OneWeek => 604_800,
            Self::OneYear => 31_536_000,
        }
    }
    pub fn interval_seconds(self) -> u64 {
        match self {
            Self::TenMinutes => 5,
            Self::TwoHours => 60,
            Self::OneWeek => 3_600,
            Self::OneYear => 86_400,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::TenMinutes => "10 minutes",
            Self::TwoHours => "2 hours",
            Self::OneWeek => "1 week",
            Self::OneYear => "1 year",
        }
    }
    pub(crate) fn poll(self) -> Duration {
        Duration::from_secs(self.interval_seconds().clamp(5, 60))
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetricsOptions {
    pub window: MetricsWindow,
    pub sources: Vec<String>,
}
impl MetricsOptions {
    pub fn validate(&self) -> Result<(), Error> {
        let unique = self
            .sources
            .iter()
            .collect::<std::collections::BTreeSet<_>>();
        if self.sources.len() > 32
            || unique.len() != self.sources.len()
            || self.sources.iter().any(|source| !valid_source(source))
        {
            return Err(Error::InvalidMetricsOptions);
        }
        Ok(())
    }
}
pub(crate) fn valid_source(source: &str) -> bool {
    !source.is_empty()
        && source.len() <= 128
        && source
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.'))
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MetricCategory {
    Cpu,
    Memory,
    Disk,
    Network,
    Other,
}
impl MetricCategory {
    fn for_name(name: &str) -> Self {
        if name.starts_with("cpu") || name.starts_with("runstate_") {
            Self::Cpu
        } else if name.starts_with("memory") {
            Self::Memory
        } else if name.starts_with("vbd_") || name.starts_with("disk_") || name.starts_with("io_") {
            Self::Disk
        } else if name.starts_with("vif_")
            || name.starts_with("pif_")
            || name.starts_with("network")
        {
            Self::Network
        } else {
            Self::Other
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MetricSource {
    pub name: String,
    pub description: String,
    pub units: String,
    pub enabled: bool,
    pub category: MetricCategory,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MetricSample {
    pub timestamp: u64,
    pub value: Option<f64>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MetricSeries {
    pub source: String,
    pub samples: Vec<MetricSample>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MetricsState {
    Loading,
    Ready,
    Stale,
    Failed,
    Closed,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MetricsSnapshot {
    pub pool: PoolId,
    pub generation: u64,
    pub target: MetricTarget,
    pub state: MetricsState,
    pub sources: Vec<MetricSource>,
    pub series: Vec<MetricSeries>,
    pub updated_at_ms: Option<u64>,
    pub error: Option<String>,
    pub sample_step_seconds: Option<u64>,
    pub range_start: Option<u64>,
    pub range_end: Option<u64>,
}
pub struct Metrics {
    receiver: watch::Receiver<Arc<MetricsSnapshot>>,
    cancel: CancellationToken,
    runtime: tokio::runtime::Handle,
}
impl Metrics {
    pub fn latest(&self) -> Arc<MetricsSnapshot> {
        self.receiver.borrow().clone()
    }
    pub fn close(&self) {
        self.cancel.cancel();
    }
    pub fn notify_on_change(&self, callback: Arc<dyn Fn() + Send + Sync>) {
        let mut receiver = self.receiver.clone();
        self.runtime.spawn(async move {
            while receiver.changed().await.is_ok() {
                callback();
            }
        });
    }
}
impl Drop for Metrics {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
pub(crate) fn channel(
    observed: ObservedMetrics,
    options: MetricsOptions,
    runtime: tokio::runtime::Handle,
) -> (Metrics, MetricsRequest) {
    let snapshot = MetricsSnapshot {
        pool: observed.pool,
        generation: observed.generation,
        target: observed.target,
        state: MetricsState::Loading,
        sources: Vec::new(),
        series: Vec::new(),
        updated_at_ms: None,
        error: None,
        sample_step_seconds: None,
        range_start: None,
        range_end: None,
    };
    let cancel = CancellationToken::new();
    let (sender, receiver) = watch::channel(Arc::new(snapshot));
    (
        Metrics {
            receiver,
            cancel: cancel.clone(),
            runtime,
        },
        MetricsRequest::new(observed, options, sender, cancel),
    )
}

#[cfg(test)]
#[path = "metrics_tests.rs"]
mod tests;
