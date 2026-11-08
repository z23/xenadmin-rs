use crate::{
    DirectoryHostState, Error, HostId, ObservedResource, RoleId, SubjectId, xapi::Session,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DirectorySubject {
    pub id: SubjectId,
    pub identifier: String,
    pub name: String,
    pub roles: Vec<RoleId>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SecurityRole {
    pub id: RoleId,
    pub name: String,
    pub description: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SecurityOverview {
    pub supported_methods: Vec<String>,
    pub directory_hosts: Vec<DirectoryHostState>,
    pub subjects: Vec<DirectorySubject>,
    pub roles: Vec<SecurityRole>,
    pub is_local_superuser: bool,
    pub rbac_available: Option<bool>,
    pub errors: BTreeMap<String, String>,
}
#[derive(Clone, Debug)]
pub enum PoolSecurityQuery {
    Overview,
    ResolveSubject { name: String },
    Certificate { host: HostId },
}
#[derive(Clone, Debug)]
pub enum PoolSecurityResult {
    Overview(SecurityOverview),
    Subject {
        query: String,
        identifier: String,
        name: String,
    },
    Certificate {
        host: HostId,
        fingerprint: crate::CertificateFingerprint,
    },
}
pub struct PoolSecurityDiscovery {
    receiver: watch::Receiver<Option<Result<PoolSecurityResult, String>>>,
    runtime: tokio::runtime::Handle,
    cancel: CancellationToken,
}
impl PoolSecurityDiscovery {
    pub fn latest(&self) -> Option<Result<PoolSecurityResult, String>> {
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
impl Drop for PoolSecurityDiscovery {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
pub(crate) struct Request {
    pub observed: ObservedResource,
    query: PoolSecurityQuery,
    sender: watch::Sender<Option<Result<PoolSecurityResult, String>>>,
    cancel: CancellationToken,
}
pub(crate) fn channel(
    observed: ObservedResource,
    query: PoolSecurityQuery,
    runtime: tokio::runtime::Handle,
) -> Result<(PoolSecurityDiscovery, Request), Error> {
    if !matches!(observed.target(), crate::ResourceTarget::Pool(_))
        || matches!(&query,PoolSecurityQuery::ResolveSubject{name} if name.is_empty()||name.len()>1024||name.chars().any(char::is_control))
    {
        return Err(Error::InvalidResourceCommand);
    }
    let (sender, receiver) = watch::channel(None);
    let cancel = CancellationToken::new();
    Ok((
        PoolSecurityDiscovery {
            receiver,
            runtime,
            cancel: cancel.clone(),
        },
        Request {
            observed,
            query,
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
        let result = tokio::select! {_=self.cancel.cancelled()=>Err(Error::Closed),_=generation.cancelled()=>Err(Error::Stale),result=tokio::time::timeout(std::time::Duration::from_secs(45),query(&session,&self.query))=>result.unwrap_or(Err(Error::Timeout))};
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
pub(crate) async fn records(
    session: &Session,
    class: &str,
    max: usize,
) -> Result<serde_json::Map<String, Value>, Error> {
    session
        .call(&format!("{class}.get_all_records"), vec![])
        .await?
        .as_object()
        .filter(|v| v.len() <= max)
        .cloned()
        .ok_or(Error::Protocol(
            "security inventory exceeds inspection bounds",
        ))
}
pub(crate) fn text(value: &Value, max: usize) -> Result<String, Error> {
    value
        .as_str()
        .filter(|s| s.len() <= max && !s.chars().any(char::is_control))
        .map(str::to_owned)
        .ok_or(Error::Protocol("invalid security record text"))
}
pub(crate) async fn directory_hosts(session: &Session) -> Result<Vec<DirectoryHostState>, Error> {
    let mut hosts = records(session, "host", 256)
        .await?
        .values()
        .map(|r| {
            Ok(DirectoryHostState {
                host: HostId::parse(&text(&r["uuid"], 36)?)
                    .map_err(|_| Error::Protocol("invalid host UUID"))?,
                auth_type: text(&r["external_auth_type"], 64)?,
                service_name: text(&r["external_auth_service_name"], 1024)?,
            })
        })
        .collect::<Result<Vec<_>, Error>>()?;
    if hosts.is_empty() {
        return Err(Error::Protocol("pool members unavailable"));
    }
    hosts.sort_by_key(|h| h.host);
    Ok(hosts)
}
pub(crate) async fn methods(session: &Session) -> Result<Vec<String>, Error> {
    let value = session.call("host.list_methods", vec![]).await?;
    let values = value
        .as_array()
        .filter(|v| v.len() <= 16_384)
        .ok_or(Error::Protocol("method catalogue unavailable"))?;
    values
        .iter()
        .map(|v| text(v, 256))
        .filter(|v| {
            v.as_ref().map_or(true, |v| {
                v.starts_with("pool.")
                    || v.starts_with("subject.")
                    || v == "host.install_server_certificate"
                    || v == "host.reset_server_certificate"
            })
        })
        .collect()
}
pub(crate) async fn local_superuser(session: &Session) -> Result<bool, Error> {
    session
        .call("session.get_record", vec![json!(session.session_cookie())])
        .await?["is_local_superuser"]
        .as_bool()
        .ok_or(Error::Protocol("session privilege information unavailable"))
}
pub(crate) async fn certificate(
    session: &Session,
    host: HostId,
) -> Result<crate::CertificateFingerprint, Error> {
    use sha2::{Digest, Sha256};
    let reference = session
        .call("host.get_by_uuid", vec![json!(host.to_string())])
        .await?;
    let reference = crate::xapi::opaque_reference(&reference)?;
    let record = session
        .call("host.get_record", vec![json!(&reference)])
        .await?;
    if record["uuid"] != host.to_string() {
        return Err(Error::Stale);
    }
    let pem = session
        .call("host.get_server_certificate", vec![json!(reference)])
        .await?;
    let pem = pem
        .as_str()
        .filter(|p| p.len() <= 256 * 1024)
        .ok_or(Error::Protocol("host certificate unavailable"))?;
    let certificates = rustls_pemfile::certs(&mut pem.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| Error::Protocol("invalid host certificate"))?;
    let leaf = certificates
        .first()
        .ok_or(Error::Protocol("host certificate unavailable"))?;
    format!("{:x}", Sha256::digest(leaf.as_ref())).parse()
}
pub(crate) fn roles(
    value: &serde_json::Map<String, Value>,
) -> Result<(BTreeMap<String, RoleId>, Vec<SecurityRole>), Error> {
    let mut ids = BTreeMap::new();
    let mut result = Vec::new();
    for (reference, r) in value {
        let id = RoleId::parse(&text(&r["uuid"], 36)?)
            .map_err(|_| Error::Protocol("invalid role UUID"))?;
        ids.insert(reference.clone(), id);
        let name = text(&r["name_label"], 256)?;
        // Leaf permission roles are not assignable user roles.
        let subroles = r["subroles"]
            .as_array()
            .filter(|v| v.len() <= 16_384)
            .ok_or(Error::Protocol("role permissions are unavailable"))?;
        if !subroles.is_empty() {
            result.push(SecurityRole {
                id,
                name,
                description: text(&r["name_description"], 2048)?,
            });
        }
    }
    if result.len() > 128 {
        return Err(Error::Protocol(
            "assignable role catalogue exceeds inspection limit",
        ));
    }
    Ok((ids, result))
}
pub(crate) fn subject_roles(
    value: &Value,
    roles: &BTreeMap<String, RoleId>,
) -> Result<Vec<RoleId>, Error> {
    let mut ids = value
        .as_array()
        .filter(|v| v.len() <= 64)
        .ok_or(Error::Protocol("subject roles unavailable"))?
        .iter()
        .map(|v| {
            roles
                .get(v.as_str().ok_or(Error::Protocol("invalid subject role"))?)
                .copied()
                .ok_or(Error::Protocol("unknown subject role"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    ids.sort();
    ids.dedup();
    Ok(ids)
}
pub(crate) async fn rbac_available(session: &Session) -> Result<Option<bool>, Error> {
    let records = records(session, "host", 256).await?;
    if records.is_empty() {
        return Ok(None);
    }
    let values: Option<Vec<_>> = records
        .values()
        .map(
            |host| match host["license_params"]["restrict_rbac"].as_str() {
                Some("false") => Some(true),
                Some("true") => Some(false),
                _ => None,
            },
        )
        .collect();
    Ok(values.map(|values| values.iter().all(|v| *v)))
}
async fn overview(session: &Session) -> Result<SecurityOverview, Error> {
    let mut result = SecurityOverview::default();
    match methods(session).await {
        Ok(v) => result.supported_methods = v,
        Err(e) => {
            result.errors.insert("methods".into(), e.to_string());
        }
    }
    match directory_hosts(session).await {
        Ok(v) => result.directory_hosts = v,
        Err(e) => {
            result.errors.insert("directory".into(), e.to_string());
        }
    }
    match local_superuser(session).await {
        Ok(v) => result.is_local_superuser = v,
        Err(e) => {
            result.errors.insert("privileges".into(), e.to_string());
        }
    }
    match rbac_available(session).await {
        Ok(value) => result.rbac_available = value,
        Err(error) => {
            result.errors.insert("RBAC".into(), error.to_string());
        }
    }
    let role_records = records(session, "role", 16_384).await;
    match role_records.and_then(|r| roles(&r)) {
        Ok((ids, roles)) => {
            result.roles = roles;
            match records(session, "subject", 4096).await.and_then(|records| {
                records
                    .values()
                    .map(|r| {
                        Ok(DirectorySubject {
                            id: SubjectId::parse(&text(&r["uuid"], 36)?)
                                .map_err(|_| Error::Protocol("invalid subject UUID"))?,
                            identifier: text(&r["subject_identifier"], 1024)?,
                            name: r["other_config"]["subject-name"]
                                .as_str()
                                .filter(|s| s.len() <= 1024 && !s.chars().any(char::is_control))
                                .unwrap_or("")
                                .into(),
                            roles: subject_roles(&r["roles"], &ids)?,
                        })
                    })
                    .collect::<Result<Vec<_>, Error>>()
            }) {
                Ok(v) => result.subjects = v,
                Err(e) => {
                    result.errors.insert("subjects".into(), e.to_string());
                }
            }
        }
        Err(e) => {
            result.errors.insert("roles".into(), e.to_string());
        }
    }
    Ok(result)
}
async fn query(session: &Session, query: &PoolSecurityQuery) -> Result<PoolSecurityResult, Error> {
    match query {
        PoolSecurityQuery::Overview => overview(session).await.map(PoolSecurityResult::Overview),
        PoolSecurityQuery::Certificate { host } => Ok(PoolSecurityResult::Certificate {
            host: *host,
            fingerprint: certificate(session, *host).await?,
        }),
        PoolSecurityQuery::ResolveSubject { name } => {
            let identifier = text(
                &session
                    .call("auth.get_subject_identifier", vec![json!(name)])
                    .await?,
                1024,
            )?;
            let info = session
                .call(
                    "auth.get_subject_information_from_identifier",
                    vec![json!(&identifier)],
                )
                .await?;
            Ok(PoolSecurityResult::Subject {
                query: name.clone(),
                identifier,
                name: text(&info["subject-name"], 1024)?,
            })
        }
    }
}
