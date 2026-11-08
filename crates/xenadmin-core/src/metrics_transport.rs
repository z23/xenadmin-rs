use super::*;
use crate::{Endpoint, Inventory, xapi::Session};
use serde_json::{Value, json};
use url::Url;
const MAX_BYTES: usize = 8 * 1024 * 1024;
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Route {
    pub host: HostId,
    pub endpoint: Endpoint,
    pub reference: String,
}
fn reference(value: &Value) -> Result<&str, Error> {
    value
        .as_str()
        .filter(|s| {
            s.starts_with("OpaqueRef:")
                && *s != "OpaqueRef:NULL"
                && s.len() <= 512
                && s.bytes().all(|b| b.is_ascii_graphic())
        })
        .ok_or(Error::Protocol("metrics target reference unavailable"))
}
fn origin(session: &Session, address: &str) -> Result<Endpoint, Error> {
    let mut url =
        Url::parse(session.options().endpoint.as_str()).map_err(|_| Error::MetricsEndpoint)?;
    if let Ok(ip) = address.parse::<std::net::IpAddr>() {
        url.set_ip_host(ip).map_err(|_| Error::MetricsEndpoint)?;
    } else {
        if address.is_empty()
            || address.len() > 253
            || !address
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'-'))
        {
            return Err(Error::MetricsEndpoint);
        }
        url.set_host(Some(address))
            .map_err(|_| Error::MetricsEndpoint)?;
    }
    Endpoint::parse(url.as_str()).map_err(|_| Error::MetricsEndpoint)
}
pub(super) async fn resolve(
    session: &Session,
    target: ObservedMetrics,
    inventory: &Inventory,
) -> Result<Route, Error> {
    if inventory.pool_id != target.pool {
        return Err(Error::Stale);
    }
    let class = target.target.class();
    let object_ref = session
        .call(
            &format!("{class}.get_by_uuid"),
            vec![json!(target.target.uuid().to_string())],
        )
        .await?;
    let object_ref = reference(&object_ref)?.to_owned();
    let object = session
        .call(&format!("{class}.get_record"), vec![json!(&object_ref)])
        .await?;
    if object["uuid"].as_str() != Some(target.target.uuid().to_string().as_str()) {
        return Err(Error::Stale);
    }
    let host = match target.target {
        MetricTarget::Host(_) => object,
        MetricTarget::Vm(_) => {
            let host_ref = if object["resident_on"] == "OpaqueRef:NULL" {
                let pool = session
                    .call("pool.get_by_uuid", vec![json!(target.pool.to_string())])
                    .await?;
                let pool = session
                    .call("pool.get_record", vec![json!(reference(&pool)?)])
                    .await?;
                if pool["uuid"] != target.pool.to_string() {
                    return Err(Error::Stale);
                }
                reference(&pool["master"])?.to_owned()
            } else {
                reference(&object["resident_on"])?.to_owned()
            };
            session
                .call("host.get_record", vec![json!(host_ref)])
                .await?
        }
    };
    let id = host["uuid"]
        .as_str()
        .and_then(|id| HostId::parse(id).ok())
        .ok_or(Error::Protocol("metrics host identity unavailable"))?;
    let known = inventory.hosts.get(&id).ok_or(Error::MetricsEndpoint)?;
    let address = host["address"]
        .as_str()
        .filter(|address| *address == known.address)
        .ok_or(Error::MetricsEndpoint)?;
    let endpoint = if inventory.coordinator == Some(id) {
        session.options().endpoint.clone()
    } else {
        origin(session, address)?
    };
    Ok(Route {
        host: id,
        endpoint,
        reference: object_ref,
    })
}
pub(super) async fn sources(
    session: &Session,
    target: MetricTarget,
    route: &Route,
) -> Result<Vec<MetricSource>, Error> {
    let value = session
        .call(
            &format!("{}.get_data_sources", target.class()),
            vec![json!(&route.reference)],
        )
        .await?;
    decode::sources(&value)
}
pub(super) fn redirect(
    session: &Session,
    current: &Url,
    location: &str,
    inventory: &Inventory,
) -> Result<Endpoint, Error> {
    let redirected = current.join(location).map_err(|_| Error::MetricsEndpoint)?;
    let base =
        Url::parse(session.options().endpoint.as_str()).map_err(|_| Error::MetricsEndpoint)?;
    if redirected.scheme() != base.scheme()
        || redirected.port_or_known_default() != base.port_or_known_default()
        || redirected.path() != "/rrd_updates"
        || !redirected.username().is_empty()
        || redirected.password().is_some()
        || redirected.fragment().is_some()
    {
        return Err(Error::MetricsEndpoint);
    }
    let endpoint = Endpoint::parse(&redirected[..url::Position::BeforePath])
        .map_err(|_| Error::MetricsEndpoint)?;
    if endpoint != session.options().endpoint
        && !inventory
            .hosts
            .values()
            .any(|host| origin(session, &host.address).ok().as_ref() == Some(&endpoint))
    {
        return Err(Error::MetricsEndpoint);
    }
    Ok(endpoint)
}
pub(super) async fn fetch(
    session: &Session,
    route: &Route,
    target: MetricTarget,
    inventory: &Inventory,
    start: i64,
    interval: u64,
) -> Result<Vec<u8>, Error> {
    let mut endpoint = route.endpoint.clone();
    let mut visited = std::collections::BTreeSet::new();
    for _ in 0..4 {
        if !visited.insert(endpoint.to_string()) {
            return Err(Error::MetricsEndpoint);
        }
        let mut url = Url::parse(endpoint.as_str()).map_err(|_| Error::MetricsEndpoint)?;
        url.set_path("/rrd_updates");
        url.query_pairs_mut()
            .append_pair("session_id", session.session_cookie())
            .append_pair("start", &start.to_string())
            .append_pair("interval", &interval.to_string())
            .append_pair("cf", "AVERAGE")
            .append_pair(
                "vm_uuid",
                &match target {
                    MetricTarget::Vm(id) => id.to_string(),
                    MetricTarget::Host(_) => "none".into(),
                },
            )
            .append_pair("sr_uuid", "none")
            .append_pair(
                "host",
                if matches!(target, MetricTarget::Host(_)) {
                    "true"
                } else {
                    "false"
                },
            )
            .append_pair("json", "true");
        let client = session.options().for_endpoint(endpoint.clone()).client()?;
        let mut response = client
            .get(url.clone())
            .header(reqwest::header::ACCEPT, "application/json")
            .timeout(Duration::from_secs(15))
            .send()
            .await
            .map_err(crate::xapi::transport_error)?;
        if response.status().is_redirection() {
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or(Error::MetricsEndpoint)?;
            endpoint = redirect(session, &url, location, inventory)?;
            continue;
        }
        match response.status().as_u16() {
            401 => return Err(Error::Api("SESSION_INVALID".into())),
            403 => return Err(Error::Api("RBAC_PERMISSION_DENIED".into())),
            _ => {}
        }
        if !response.status().is_success() {
            return Err(Error::Http(response.status().as_u16()));
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_BYTES as u64)
        {
            return Err(Error::Protocol("metrics response exceeds the size limit"));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(crate::xapi::transport_error)?
        {
            if bytes.len() + chunk.len() > MAX_BYTES {
                return Err(Error::Protocol("metrics response exceeds the size limit"));
            }
            bytes.extend_from_slice(&chunk);
        }
        return Ok(bytes);
    }
    Err(Error::MetricsEndpoint)
}
