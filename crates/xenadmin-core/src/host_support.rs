use crate::{Error, HostId, ObservedResource, PoolId, ResourceTarget, xapi::Session};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeSet, sync::Arc};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use xenadmin_xapi::records::{HostCrashdumpRecord, HostRecord};

#[path = "host_archive.rs"]
pub(crate) mod archive;
#[path = "host_restore.rs"]
pub(crate) mod restore;
pub use restore::HostBackupSummary;
pub const MAX_HOST_ARCHIVE_BYTES: u64 = 64 * 1024 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HostArchiveOperation {
    Backup,
    StageRestore { archive: HostBackupSummary },
    Support { entries: Vec<String> },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostSupportCapability {
    pub key: String,
    pub privacy: String,
    pub minimum_bytes: Option<u64>,
    pub maximum_bytes: Option<u64>,
    pub maximum_seconds: Option<u64>,
    pub default_selected: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostCrashdump {
    pub uuid: crate::HostCrashdumpId,
    pub timestamp: String,
    pub size: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostSupportOverview {
    pub host: HostId,
    pub version: String,
    pub capabilities: Vec<HostSupportCapability>,
    pub capabilities_error: Option<String>,
    pub crashdumps: Vec<HostCrashdump>,
    pub crashdumps_error: Option<String>,
}
pub struct HostSupportQuery {
    receiver: watch::Receiver<Option<Result<HostSupportOverview, String>>>,
    runtime: tokio::runtime::Handle,
    cancel: CancellationToken,
}
impl HostSupportQuery {
    pub fn latest(&self) -> Option<Result<HostSupportOverview, String>> {
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
impl Drop for HostSupportQuery {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
pub(crate) struct Request {
    pub observed: ObservedResource,
    sender: watch::Sender<Option<Result<HostSupportOverview, String>>>,
    cancel: CancellationToken,
}
pub(crate) fn channel(
    observed: ObservedResource,
    runtime: tokio::runtime::Handle,
) -> Result<(HostSupportQuery, Request), Error> {
    if !matches!(observed.target(), ResourceTarget::Host(_)) {
        return Err(Error::InvalidResourceCommand);
    }
    let (sender, receiver) = watch::channel(None);
    let cancel = CancellationToken::new();
    Ok((
        HostSupportQuery {
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
        let result = tokio::select! { _=self.cancel.cancelled()=>Err(Error::Closed), _=generation.cancelled()=>Err(Error::Stale), result=tokio::time::timeout(std::time::Duration::from_secs(45), discover(&session, self.observed.pool, host))=> result.unwrap_or(Err(Error::Timeout)) };
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
pub(crate) fn key(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}
fn number(s: Option<&str>) -> Result<Option<u64>, Error> {
    match s {
        None | Some("-1") => Ok(None),
        Some(s) => s
            .parse()
            .map(Some)
            .map_err(|_| Error::Protocol("invalid host support capability size")),
    }
}
pub(crate) fn capabilities(xml: &str) -> Result<Vec<HostSupportCapability>, Error> {
    if xml.len() > 2 * 1024 * 1024 {
        return Err(Error::TransferLimit);
    }
    let doc = roxmltree::Document::parse(xml)
        .map_err(|_| Error::Protocol("invalid host support capabilities XML"))?;
    let root = doc.root_element();
    if root.tag_name().name() != "system-status-capabilities" {
        return Err(Error::Protocol("unexpected host support capabilities root"));
    }
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for node in root.children().filter(|n| n.is_element()) {
        let name = node
            .attribute("key")
            .filter(|s| key(s))
            .ok_or(Error::Protocol("invalid host support capability key"))?;
        if node.tag_name().name() != "capability" || out.len() >= 256 || !seen.insert(name) {
            return Err(Error::Protocol(
                "invalid or duplicate host support capability",
            ));
        }
        let privacy = node
            .attribute("pii")
            .filter(|v| matches!(*v, "yes" | "no" | "maybe" | "if_customized"))
            .unwrap_or("unknown");
        let minimum_bytes = number(node.attribute("min-size"))?;
        let maximum_bytes = number(node.attribute("max-size"))?;
        if minimum_bytes.zip(maximum_bytes).is_some_and(|(a, b)| a > b) {
            return Err(Error::Protocol(
                "invalid host support capability size range",
            ));
        }
        out.push(HostSupportCapability {
            key: name.into(),
            privacy: privacy.into(),
            minimum_bytes,
            maximum_bytes,
            maximum_seconds: number(node.attribute("max-time"))?,
            default_selected: node.attribute("default-checked") == Some("yes"),
        });
    }
    out.sort_by(|a, b| a.key.cmp(&b.key));
    Ok(out)
}
pub(crate) async fn host_record(
    s: &Session,
    pool: PoolId,
    host: HostId,
) -> Result<(String, Value), Error> {
    crate::snapshots::optional_record(s, "pool", &pool.to_string())
        .await?
        .ok_or(Error::Stale)?;
    crate::snapshots::optional_record(s, "host", &host.to_string())
        .await?
        .ok_or(Error::Stale)
}
pub(crate) fn version(r: &Value) -> Result<String, Error> {
    crate::security_discovery::text(
        &HostRecord::new(r).software_version()["product_version"],
        64,
    )
    .and_then(|v| {
        if v.is_empty() {
            Err(Error::TransferConflict)
        } else {
            Ok(v)
        }
    })
}
async fn advertised(s: &Session, reference: &str) -> Result<Vec<HostSupportCapability>, Error> {
    let value = s
        .call(
            "host.get_system_status_capabilities",
            vec![json!(reference)],
        )
        .await?;
    capabilities(
        value
            .as_str()
            .ok_or(Error::Protocol("host support capabilities are not XML"))?,
    )
}
async fn crashdumps(s: &Session, reference: &str) -> Result<Vec<HostCrashdump>, Error> {
    let values = crate::security_discovery::records(s, "host_crashdump", 4096).await?;
    let mut seen = BTreeSet::new();
    let mut out = vec![];
    for r in values.values().filter(|r| r["host"] == reference) {
        let dump = decode_crashdump(r)?;
        if !seen.insert(dump.uuid) {
            return Err(Error::Protocol("duplicate host crashdump UUID"));
        }
        out.push(dump);
    }
    out.sort_by_key(|d| d.uuid);
    Ok(out)
}
pub(crate) fn decode_crashdump(r: &Value) -> Result<HostCrashdump, Error> {
    Ok(HostCrashdump {
        uuid: HostCrashdumpRecord::new(r)
            .uuid()
            .text()
            .and_then(|s| crate::HostCrashdumpId::parse(s).ok())
            .filter(|id| !id.uuid().is_nil())
            .ok_or(Error::Protocol("invalid host crashdump UUID"))?,
        size: HostCrashdumpRecord::new(r)
            .size()
            .unsigned()
            .ok_or(Error::Protocol("invalid host crashdump size"))?,
        timestamp: crate::security_discovery::text(HostCrashdumpRecord::new(r).timestamp(), 64)?,
    })
}
pub(crate) async fn discover(
    s: &Session,
    pool: PoolId,
    host: HostId,
) -> Result<HostSupportOverview, Error> {
    let (reference, record) = host_record(s, pool, host).await?;
    let version = version(&record)?;
    let caps = advertised(s, &reference).await;
    let dumps = crashdumps(s, &reference).await;
    Ok(HostSupportOverview {
        host,
        version,
        capabilities_error: caps.as_ref().err().map(ToString::to_string),
        capabilities: caps.unwrap_or_default(),
        crashdumps_error: dumps.as_ref().err().map(ToString::to_string),
        crashdumps: dumps.unwrap_or_default(),
    })
}
pub(crate) fn validate_operation(operation: &HostArchiveOperation) -> Result<(), Error> {
    if let HostArchiveOperation::Support { entries } = operation
        && (entries.is_empty()
            || entries.len() > 256
            || entries.iter().any(|s| !key(s))
            || entries.iter().collect::<BTreeSet<_>>().len() != entries.len())
    {
        return Err(Error::InvalidResourceCommand);
    }
    Ok(())
}
pub(crate) async fn prepare(
    s: &Session,
    pool: PoolId,
    host: HostId,
    expected_version: &str,
    operation: &HostArchiveOperation,
) -> Result<crate::transfer::http::Prepared, Error> {
    validate_operation(operation)?;
    let (reference, record) = host_record(s, pool, host).await?;
    if version(&record)? != expected_version {
        return Err(Error::TransferConflict);
    }
    let address = HostRecord::new(&record)
        .address()
        .text()
        .filter(|s| !s.is_empty() && s.len() <= 253 && !s.contains(['/', '@', '?', '#']))
        .ok_or(Error::TransferEndpoint)?;
    let mut endpoint = s.options().endpoint.clone();
    let mut target = url::Url::parse(endpoint.as_str()).map_err(|_| Error::TransferEndpoint)?;
    if target.host_str() != Some(address) {
        target
            .set_host(Some(address))
            .map_err(|_| Error::TransferEndpoint)?;
        target
            .set_scheme("https")
            .map_err(|_| Error::TransferEndpoint)?;
        target
            .set_port(Some(443))
            .map_err(|_| Error::TransferEndpoint)?;
        endpoint = crate::Endpoint::parse(target.as_str()).map_err(|_| Error::TransferEndpoint)?;
    }
    let (kind, query) = match operation {
        HostArchiveOperation::Backup => (crate::transfer::http::Kind::HostBackup, vec![]),
        HostArchiveOperation::StageRestore { archive } => {
            if s.options().read_only {
                return Err(Error::ReadOnly);
            }
            if archive.host != host
                || archive.version != expected_version
                || HostRecord::new(&record)
                    .current_operations()
                    .entries()
                    .is_none_or(|m| !m.is_empty())
            {
                return Err(Error::TransferConflict);
            }
            (crate::transfer::http::Kind::HostRestore, vec![])
        }
        HostArchiveOperation::Support { entries } => {
            let current = advertised(s, &reference).await?;
            if entries.iter().any(|e| !current.iter().any(|c| &c.key == e)) {
                return Err(Error::UnavailableAction);
            }
            (
                crate::transfer::http::Kind::HostSupport,
                vec![
                    ("entries".into(), entries.join(",")),
                    ("output".into(), "tar".into()),
                ],
            )
        }
    };
    if !endpoint.as_str().starts_with("https://") {
        return Err(Error::Protocol(
            "Host archives require HTTPS with exact host identity",
        ));
    }
    let peer_identity = unique_certificate(s, host).await?;
    let (_, current) = host_record(s, pool, host).await?;
    if HostRecord::new(&current).address() != HostRecord::new(&record).address()
        || version(&current)? != expected_version
        || (matches!(operation, HostArchiveOperation::StageRestore { .. })
            && HostRecord::new(&current)
                .current_operations()
                .entries()
                .is_none_or(|m| !m.is_empty()))
    {
        return Err(Error::TransferConflict);
    }
    Ok(crate::transfer::http::Prepared {
        kind,
        peer_identity: Some(peer_identity),
        endpoint,
        known_hosts: BTreeSet::new(),
        query,
        upload: matches!(operation, HostArchiveOperation::StageRestore { .. }),
    })
}

async fn unique_certificate(
    s: &Session,
    host: HostId,
) -> Result<crate::CertificateFingerprint, Error> {
    let hosts = crate::security_discovery::records(s, "host", 128).await?;
    let ids = hosts
        .values()
        .map(|r| {
            HostRecord::new(r)
                .uuid()
                .text()
                .and_then(|v| HostId::parse(v).ok())
                .ok_or(Error::TransferConflict)
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    if ids.len() != hosts.len() || !ids.contains(&host) {
        return Err(Error::TransferConflict);
    }
    let certificate = crate::security_discovery::certificate(s, host).await?;
    for other in ids.into_iter().filter(|id| *id != host) {
        if crate::security_discovery::certificate(s, other).await? == certificate {
            return Err(Error::Protocol(
                "Host certificates are shared; archive endpoint cannot be attributed to the selected host",
            ));
        }
    }
    Ok(certificate)
}
