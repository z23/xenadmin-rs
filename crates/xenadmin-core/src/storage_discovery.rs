use crate::{Error, HostId, ObservedResource, SrId, StorageLocation, xapi::Session};
use serde::{Deserialize, Serialize};
use xenadmin_xapi::records::{HostRecord, SmRecord};
use xenadmin_xapi::{calls, refs};
#[path = "storage_discovery_xml.rs"]
mod xml;
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
pub use xml::{
    DiscoveredHbaDevice, DiscoveredIscsiLun, DiscoveredIscsiTarget, DiscoveredNfsExport,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageBackend {
    pub kind: String,
    pub name: String,
    pub features: Vec<String>,
    pub configuration: BTreeMap<String, String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredRepository {
    pub uuid: SrId,
    pub name: String,
    pub size_bytes: Option<u64>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StorageDiscoveryResult {
    LocalRepositories(Vec<crate::DiscoveredLocalRepository>),
    Backends(Vec<StorageBackend>),
    Repositories(Vec<DiscoveredRepository>),
    NfsExports(Vec<DiscoveredNfsExport>),
    IscsiTargets(Vec<DiscoveredIscsiTarget>),
    IscsiLuns(Vec<DiscoveredIscsiLun>),
    HbaDevices(Vec<DiscoveredHbaDevice>),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StorageQuery {
    LocalDevice {
        host: HostId,
        device: String,
    },
    HbaDevices {
        host: HostId,
    },
    Backends,
    NfsExports {
        host: HostId,
        server: String,
        version: crate::NfsVersion,
    },
    IscsiTargets {
        host: HostId,
        portal: String,
        port: u16,
    },
    IscsiLuns {
        host: HostId,
        portal: String,
        port: u16,
        target_iqn: String,
    },
    Probe {
        host: HostId,
        location: StorageLocation,
    },
}
impl StorageQuery {
    pub(crate) fn validate(&self) -> Result<(), Error> {
        let valid = match self {
            Self::LocalDevice { device, .. } => crate::storage_local::valid_path(device),
            Self::Backends | Self::HbaDevices { .. } => true,
            Self::Probe { location, .. } => location.validate(),
            Self::NfsExports { server, .. } => crate::storage::storage_server(server),
            Self::IscsiTargets { portal, port, .. } => {
                crate::storage::storage_server(portal) && *port > 0
            }
            Self::IscsiLuns {
                portal,
                port,
                target_iqn,
                ..
            } => {
                crate::storage::storage_server(portal)
                    && *port > 0
                    && crate::storage::storage_token(target_iqn)
                    && target_iqn.starts_with("iqn.")
            }
        };
        if valid {
            Ok(())
        } else {
            Err(Error::InvalidResourceCommand)
        }
    }
}
pub struct StorageDiscovery {
    receiver: watch::Receiver<Option<Result<StorageDiscoveryResult, String>>>,
    runtime: tokio::runtime::Handle,
    cancel: CancellationToken,
}
impl StorageDiscovery {
    pub fn latest(&self) -> Option<Result<StorageDiscoveryResult, String>> {
        self.receiver.borrow().clone().or_else(|| {
            self.receiver
                .has_changed()
                .is_err()
                .then(|| Err(Error::Closed.to_string()))
        })
    }
    pub fn notify_on_change(&self, callback: Arc<dyn Fn() + Send + Sync>) {
        let mut receiver = self.receiver.clone();
        self.runtime.spawn(async move {
            if receiver.changed().await.is_ok() {
                callback();
            }
        });
    }
}
impl Drop for StorageDiscovery {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
pub(crate) struct DiscoveryRequest {
    pub observed: ObservedResource,
    query: StorageQuery,
    pub(crate) credentials: Option<crate::StorageCredentials>,
    sender: watch::Sender<Option<Result<StorageDiscoveryResult, String>>>,
    cancel: CancellationToken,
}
pub(crate) fn channel(
    observed: ObservedResource,
    query: StorageQuery,
    runtime: tokio::runtime::Handle,
) -> (StorageDiscovery, DiscoveryRequest) {
    let (sender, receiver) = watch::channel(None);
    let cancel = CancellationToken::new();
    (
        StorageDiscovery {
            receiver,
            runtime,
            cancel: cancel.clone(),
        },
        DiscoveryRequest {
            observed,
            query,
            credentials: None,
            sender,
            cancel,
        },
    )
}
impl DiscoveryRequest {
    pub(crate) fn fail(self, error: Error) {
        self.sender.send_replace(Some(Err(error.to_string())));
    }
    pub(crate) async fn run(
        self,
        session: Arc<Session>,
        receiver: watch::Receiver<Arc<crate::ConnectionSnapshot>>,
        generation: CancellationToken,
    ) {
        let result = tokio::select! {
            _=self.cancel.cancelled()=>Err(Error::Closed),
            _=generation.cancelled()=>Err(Error::Stale),
            result=tokio::time::timeout(std::time::Duration::from_secs(45), self.query(&session))=>result.unwrap_or(Err(Error::Timeout)),
        };
        let current = receiver
            .borrow()
            .observe_resource(self.observed.target)
            .is_ok_and(|observed| observed == self.observed);
        self.sender.send_replace(Some(if current {
            result.map_err(|e| e.to_string())
        } else {
            Err(Error::Stale.to_string())
        }));
    }
    async fn query(&self, session: &Session) -> Result<StorageDiscoveryResult, Error> {
        match &self.query {
            StorageQuery::LocalDevice { host, device } => {
                crate::storage_local::probe(session, *host, device)
                    .await
                    .map(StorageDiscoveryResult::LocalRepositories)
            }
            StorageQuery::Backends => {
                decode_backends(&session.send(calls::sm::GetAllRecords).await?)
                    .map(StorageDiscoveryResult::Backends)
            }
            query => {
                let (host, kind, mut config, expected) = match query {
                    StorageQuery::HbaDevices { host } => (
                        *host,
                        "lvmohba",
                        json!({}),
                        &["SR_BACKEND_FAILURE_90", "SR_BACKEND_FAILURE_107"][..],
                    ),
                    StorageQuery::NfsExports {
                        host,
                        server,
                        version,
                    } => {
                        let mut config = json!({"server":server});
                        if let Some(version) = version.value() {
                            config["nfsversion"] = json!(version);
                        }
                        (*host, "nfs", config, &["SR_BACKEND_FAILURE_101"][..])
                    }
                    StorageQuery::IscsiTargets { host, portal, port } => (
                        *host,
                        "lvmoiscsi",
                        json!({"target":portal,"port":port.to_string()}),
                        &["SR_BACKEND_FAILURE_96"][..],
                    ),
                    StorageQuery::IscsiLuns {
                        host,
                        portal,
                        port,
                        target_iqn,
                    } => (
                        *host,
                        "lvmoiscsi",
                        json!({"target":portal,"port":port.to_string(),"targetIQN":target_iqn}),
                        &["SR_BACKEND_FAILURE_107"][..],
                    ),
                    StorageQuery::Probe { host, location } => {
                        (*host, location.kind(), location.device_config(), &[][..])
                    }
                    StorageQuery::Backends | StorageQuery::LocalDevice { .. } => unreachable!(),
                };
                if let Some(credentials) = &self.credentials {
                    validate_credentials(query, credentials)?;
                    for (kind, entry) in credentials
                        .authentication
                        .kinds()
                        .iter()
                        .zip(&credentials.entries)
                    {
                        config[kind.username_key()] = json!(&*entry.username);
                        let password = kind
                            .password_key()
                            .strip_suffix("_secret")
                            .ok_or(Error::InvalidResourceCommand)?;
                        config[password] = json!(&*entry.password);
                    }
                }
                let backends = decode_backends(&session.send(calls::sm::GetAllRecords).await?)?;
                if !backends
                    .iter()
                    .any(|b| b.kind == kind && b.features.iter().any(|f| f == "SR_PROBE"))
                {
                    return Err(Error::UnavailableAction);
                }
                let host_ref = session
                    .send(calls::host::GetByUuid(&host.to_string()))
                    .await?;
                let host_ref = crate::xapi::opaque_reference(&host_ref)?;
                let fresh = session
                    .send(calls::host::GetRecord(refs::host(&host_ref)))
                    .await?;
                if HostRecord::new(&fresh).uuid() != host.to_string() {
                    return Err(Error::Stale);
                }
                let result = session
                    .storage_probe_xml(
                        vec![json!(host_ref), config, json!(kind), json!({})],
                        expected,
                    )
                    .await?;
                match query {
                    StorageQuery::HbaDevices { .. } => {
                        xml::hba(&result).map(StorageDiscoveryResult::HbaDevices)
                    }
                    StorageQuery::NfsExports { .. } => {
                        xml::exports(&result).map(StorageDiscoveryResult::NfsExports)
                    }
                    StorageQuery::IscsiTargets { .. } => {
                        xml::targets(&result).map(StorageDiscoveryResult::IscsiTargets)
                    }
                    StorageQuery::IscsiLuns { .. } => {
                        xml::luns(&result).map(StorageDiscoveryResult::IscsiLuns)
                    }
                    _ => decode_repositories(&result).map(StorageDiscoveryResult::Repositories),
                }
            }
        }
    }
}
fn clean(value: &str, limit: usize) -> String {
    value
        .chars()
        .filter(|c| !c.is_control())
        .take(limit)
        .collect()
}
fn decode_backends(value: &Value) -> Result<Vec<StorageBackend>, Error> {
    let records = value
        .as_object()
        .filter(|r| r.len() <= 256)
        .ok_or(Error::Protocol("invalid storage backend list"))?;
    records
        .values()
        .map(|r| {
            let kind = SmRecord::new(r)
                .type_()
                .text()
                .filter(|s| {
                    s.len() <= 128
                        && s.bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
                })
                .ok_or(Error::Protocol("invalid storage backend type"))?;
            let mut features = SmRecord::new(r)
                .capabilities()
                .members()
                .into_iter()
                .flatten()
                .take(256)
                .filter_map(Value::as_str)
                .map(|s| clean(s, 128))
                .collect::<Vec<_>>();
            features.extend(
                SmRecord::new(r)
                    .features()
                    .entries()
                    .into_iter()
                    .flat_map(|r| r.keys())
                    .take(256)
                    .map(|s| clean(s, 128)),
            );
            features.sort();
            features.dedup();
            let configuration = SmRecord::new(r)
                .configuration()
                .entries()
                .into_iter()
                .flat_map(|r| r.iter())
                .take(128)
                .filter_map(|(k, v)| Some((clean(k, 128), clean(v.as_str()?, 512))))
                .collect();
            Ok(StorageBackend {
                kind: kind.into(),
                name: clean(SmRecord::new(r).name_label().text().unwrap_or(kind), 256),
                features,
                configuration,
            })
        })
        .collect()
}
pub(crate) fn decode_repositories(xml: &str) -> Result<Vec<DiscoveredRepository>, Error> {
    if xml.len() > 512 * 1024 {
        return Err(Error::Protocol("storage probe exceeds size limit"));
    }
    let doc = roxmltree::Document::parse_with_options(
        xml,
        roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: 16000,
        },
    )
    .map_err(|_| Error::Protocol("invalid storage probe XML"))?;
    if !matches!(doc.root_element().tag_name().name(), "SRlist" | "SRList") {
        return Err(Error::Protocol("unsupported storage probe schema"));
    }
    if doc
        .root_element()
        .children()
        .filter(|n| n.is_element())
        .any(|n| n.tag_name().name() != "SR")
    {
        return Err(Error::Protocol("unsupported storage probe schema"));
    }
    let mut results = Vec::new();
    for node in doc
        .root_element()
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "SR")
    {
        if results.len() >= 1000 {
            return Err(Error::Protocol("too many storage probe results"));
        }
        for field in ["uuid", "name_label", "size"] {
            let nodes = node
                .children()
                .filter(|n| n.is_element() && n.tag_name().name().eq_ignore_ascii_case(field))
                .collect::<Vec<_>>();
            if nodes.len() > 1 || nodes.iter().any(|n| n.children().any(|c| c.is_element())) {
                return Err(Error::Protocol("ambiguous storage probe field"));
            }
        }
        let text = |field: &str| {
            node.children()
                .find(|n| n.is_element() && n.tag_name().name().eq_ignore_ascii_case(field))
                .and_then(|n| n.text())
                .map(str::trim)
        };
        let uuid = text("uuid")
            .and_then(|s| SrId::parse(s).ok())
            .ok_or(Error::Protocol("storage probe UUID unavailable"))?;
        if results
            .iter()
            .any(|r: &DiscoveredRepository| r.uuid == uuid)
        {
            return Err(Error::Protocol("duplicate storage probe UUID"));
        }
        let size_bytes = text("size")
            .map(|s| {
                s.parse()
                    .map_err(|_| Error::Protocol("invalid storage probe size"))
            })
            .transpose()?;
        results.push(DiscoveredRepository {
            uuid,
            name: clean(text("name_label").unwrap_or(""), 255),
            size_bytes,
        });
    }
    Ok(results)
}
pub(crate) fn validate_credentials(
    query: &StorageQuery,
    credentials: &crate::StorageCredentials,
) -> Result<(), Error> {
    credentials.validate_values()?;
    let valid = match query {
        StorageQuery::IscsiTargets { .. }
        | StorageQuery::IscsiLuns { .. }
        | StorageQuery::Probe {
            location: StorageLocation::Iscsi { .. },
            ..
        } => matches!(
            credentials.authentication,
            crate::StorageAuthentication::Chap | crate::StorageAuthentication::MutualChap
        ),
        StorageQuery::Probe {
            location: StorageLocation::IsoSmb { .. },
            ..
        } => credentials.authentication == crate::StorageAuthentication::Smb,
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(Error::InvalidResourceCommand)
    }
}

pub(crate) async fn hba_devices(
    session: &Session,
    host: &str,
) -> Result<Vec<DiscoveredHbaDevice>, Error> {
    let xml = session
        .storage_probe_xml(
            vec![json!(host), json!({}), json!("lvmohba"), json!({})],
            &["SR_BACKEND_FAILURE_90", "SR_BACKEND_FAILURE_107"],
        )
        .await?;
    xml::hba(&xml)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn probe_decoder_is_bounded_and_projects_only_public_fields() {
        let result=decode_repositories("<SRlist><SR><UUID>aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa</UUID><name_label>disk</name_label><password>private</password><size>1024</size></SR></SRlist>").unwrap();
        assert_eq!(result[0].size_bytes, Some(1024));
        assert!(!serde_json::to_string(&result).unwrap().contains("private"));
        for xml in [
            "<!DOCTYPE a [<!ENTITY x 'private'>]><SRlist/>",
            "<SRlist><SR><UUID>bad</UUID></SR></SRlist>",
            "<unknown/>",
        ] {
            assert!(decode_repositories(xml).is_err());
        }
    }
    #[test]
    fn feature_union_is_bounded_and_does_not_project_device_secrets() {
        let result=decode_backends(&json!({"OpaqueRef:sm":{"type":"nfs","features":{"SR_PROBE":"1"},"capabilities":["SR_PROBE","THIN_PROVISIONING"],"configuration":{"server":"Server name"},"device_config":{"password":"private"}}})).unwrap();
        assert_eq!(result[0].features, vec!["SR_PROBE", "THIN_PROVISIONING"]);
        assert!(!serde_json::to_string(&result).unwrap().contains("private"));
    }
}
