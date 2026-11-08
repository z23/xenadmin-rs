//! Host RRD recording controls; forgetting a source deletes its recorded history.
use crate::{Error, HostId, ObservedResource, ResourceTarget, xapi::Session};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeSet, sync::Arc, time::Duration};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostDataSource {
    pub name: String,
    pub description: String,
    pub units: String,
    pub standard: bool,
    pub enabled: bool,
    /// Canonical XAPI floating-point bounds, including Infinity and NaN.
    pub minimum: String,
    pub maximum: String,
}
impl HostDataSource {
    pub(crate) fn valid(&self) -> bool {
        crate::metrics::valid_source(&self.name)
            && self.description.len() <= 2048
            && !self.description.contains('\0')
            && self.units.len() <= 128
            && !self.units.chars().any(char::is_control)
            && bound(&json!(self.minimum)).is_ok_and(|v| v == self.minimum)
            && bound(&json!(self.maximum)).is_ok_and(|v| v == self.maximum)
    }
    fn same_metadata(&self, other: &Self) -> bool {
        let mut other = other.clone();
        other.enabled = self.enabled;
        *self == other
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HostRecordingAction {
    StartRecording,
    ForgetHistory,
}
impl HostRecordingAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::StartRecording => "Start recording",
            Self::ForgetHistory => "Forget recorded history",
        }
    }
    pub(crate) fn method(self) -> &'static str {
        match self {
            Self::StartRecording => "host.record_data_source",
            Self::ForgetHistory => "host.forget_data_source_archives",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostDataSources {
    pub host: HostId,
    pub sources: Vec<HostDataSource>,
}
pub struct HostDataSourceQuery {
    receiver: watch::Receiver<Option<Result<HostDataSources, String>>>,
    runtime: tokio::runtime::Handle,
    cancel: CancellationToken,
}
impl HostDataSourceQuery {
    pub fn latest(&self) -> Option<Result<HostDataSources, String>> {
        self.receiver.borrow().clone().or_else(|| {
            self.receiver
                .has_changed()
                .is_err()
                .then(|| Err(Error::Closed.to_string()))
        })
    }
    pub fn notify_on_change(&self, waker: Arc<dyn Fn() + Send + Sync>) {
        let mut receiver = self.receiver.clone();
        self.runtime.spawn(async move {
            if receiver.changed().await.is_ok() {
                waker();
            }
        });
    }
}
impl Drop for HostDataSourceQuery {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
pub(crate) struct Request {
    pub observed: ObservedResource,
    sender: watch::Sender<Option<Result<HostDataSources, String>>>,
    cancel: CancellationToken,
}
pub(crate) fn channel(
    observed: ObservedResource,
    runtime: tokio::runtime::Handle,
) -> Result<(HostDataSourceQuery, Request), Error> {
    if !matches!(observed.target(), ResourceTarget::Host(_)) {
        return Err(Error::InvalidResourceCommand);
    }
    let (sender, receiver) = watch::channel(None);
    let cancel = CancellationToken::new();
    Ok((
        HostDataSourceQuery {
            receiver,
            runtime,
            cancel: cancel.clone(),
        },
        Request {
            observed,
            sender,
            cancel,
        },
    ))
}
impl Request {
    pub(crate) fn fail(self, error: Error) {
        self.sender.send_replace(Some(Err(error.to_string())));
    }
    pub(crate) async fn run(
        self,
        session: Arc<Session>,
        receiver: watch::Receiver<Arc<crate::ConnectionSnapshot>>,
        generation: CancellationToken,
    ) {
        let ResourceTarget::Host(host) = self.observed.target() else {
            self.fail(Error::InvalidResourceCommand);
            return;
        };
        let result = tokio::select! {
            _=self.cancel.cancelled()=>Err(Error::Closed),
            _=generation.cancelled()=>Err(Error::Stale),
            result=tokio::time::timeout(Duration::from_secs(30), async {
                let (reference, _) = crate::host_support::host_record(&session, self.observed.pool, host).await?;
                let sources = catalog(&session, &reference).await?;
                let (current, _) = crate::host_support::host_record(&session, self.observed.pool, host).await?;
                if current != reference { return Err(Error::Stale); }
                Ok(HostDataSources { host, sources })
            })=>result.unwrap_or(Err(Error::Timeout))
        };
        let current = receiver
            .borrow()
            .observe_resource(self.observed.target())
            .is_ok_and(|v| v == self.observed);
        self.sender.send_replace(Some(if current {
            result.map_err(|e| e.to_string())
        } else {
            Err(Error::Stale.to_string())
        }));
    }
}
fn bound(value: &Value) -> Result<String, Error> {
    let n = value
        .as_f64()
        .or_else(|| {
            value
                .as_str()
                .filter(|v| v.len() <= 512)?
                .parse::<f64>()
                .ok()
        })
        .ok_or(Error::Protocol("Invalid host data-source bound"))?;
    Ok(if n.is_nan() {
        "NaN".into()
    } else if n == f64::INFINITY {
        "Infinity".into()
    } else if n == f64::NEG_INFINITY {
        "-Infinity".into()
    } else if n == 0.0 {
        "0".into()
    } else {
        n.to_string()
    })
}
fn decode(value: &Value) -> Result<Vec<HostDataSource>, Error> {
    let values = value
        .as_array()
        .filter(|v| v.len() <= 512)
        .ok_or(Error::Protocol(
            "Host data-source catalog is unavailable or too large",
        ))?;
    let mut seen = BTreeSet::new();
    let mut sources = Vec::new();
    for value in values {
        let text = |key: &str| {
            value[key]
                .as_str()
                .map(str::to_string)
                .ok_or(Error::Protocol("Invalid host data-source metadata"))
        };
        let boolean = |key: &str| {
            value[key]
                .as_bool()
                .ok_or(Error::Protocol("Invalid host data-source state"))
        };
        let source = HostDataSource {
            name: text("name_label")?,
            description: text("name_description")?,
            units: text("units")?,
            standard: boolean("standard")?,
            enabled: boolean("enabled")?,
            minimum: bound(&value["min"])?,
            maximum: bound(&value["max"])?,
        };
        if !source.valid() || !seen.insert(source.name.clone()) {
            return Err(Error::Protocol("Invalid or duplicated host data source"));
        }
        sources.push(source);
    }
    sources.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(sources)
}
pub(crate) async fn catalog(
    session: &Session,
    reference: &str,
) -> Result<Vec<HostDataSource>, Error> {
    decode(
        &session
            .call("host.get_data_sources", vec![json!(reference)])
            .await?,
    )
}
pub(crate) async fn check(
    session: &Session,
    reference: &str,
    expected: &HostDataSource,
    action: HostRecordingAction,
    after_receipt: bool,
) -> Result<(), Error> {
    let current = catalog(session, reference)
        .await?
        .into_iter()
        .find(|v| v.name == expected.name);
    let valid = if after_receipt {
        match current {
            None => action == HostRecordingAction::ForgetHistory,
            Some(current) => {
                current.same_metadata(expected)
                    && (action == HostRecordingAction::ForgetHistory || current.enabled)
            }
        }
    } else {
        current.as_ref() == Some(expected)
    };
    if valid {
        Ok(())
    } else {
        Err(Error::HostPrerequisite(
            "The host data source changed or its requested recording state could not be verified; refresh its catalog",
        ))
    }
}
