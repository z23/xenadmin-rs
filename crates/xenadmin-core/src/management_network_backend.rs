use super::{ResourceAction, ResourceOperationRecord, ResourceTarget, network, xapi};
use crate::{
    Error, HostId, ManagementInterface, ManagementNetworkOperation, OperationState, StepState,
    management_network::{Plan, canonical_ipv6},
    xapi::Session,
};
use serde_json::{Value, json};
use std::collections::BTreeSet;
fn require(condition: bool, message: &'static str) -> Result<(), Error> {
    if condition {
        Ok(())
    } else {
        Err(Error::Protocol(message))
    }
}
pub(super) fn operation(r: &ResourceOperationRecord) -> Result<&ManagementNetworkOperation, Error> {
    match &r.action {
        ResourceAction::ManagementNetwork(op) => Ok(op),
        _ => Err(Error::InvalidResourceCommand),
    }
}
pub(crate) fn validate_record(r: &ResourceOperationRecord) -> Result<(), Error> {
    let Some(p) = &r.management_network_plan else {
        require(
            r.management_network.is_none(),
            "Management evidence has no private plan",
        )?;
        if matches!(r.action, ResourceAction::ManagementNetwork(_)) {
            require(
                r.steps.is_empty()
                    && matches!(
                        r.state,
                        OperationState::Preparing
                            | OperationState::Cancelled
                            | OperationState::Rejected
                            | OperationState::OutcomeUnknown
                            | OperationState::AcknowledgedUnknown
                    ),
                "Management preparation evidence unavailable",
            )?;
        }
        return Ok(());
    };
    p.operation.validate(r.target)?;
    require(
        operation(r)? == &p.operation
            && r.pool == p.pool
            && r.target == ResourceTarget::Host(p.host)
            && p.host == p.operation.source().identity.host,
        "Management intent and plan differ",
    )?;
    require(
        p.recovery == p.operation.endpoint(&p.original_endpoint)?
            && p.recovery.as_str().starts_with("https://")
            && p.tasks_before.len() <= 10_000
            && p.tasks_before.iter().all(|id| !id.uuid().is_nil()),
        "Management recovery endpoint or baseline invalid",
    )?;
    require(
        r.management_network.as_ref() == Some(&p.evidence())
            && r.steps.len() == 1
            && r.steps[0].label == p.operation.label(),
        "Management public evidence differs from its plan",
    )?;
    require(
        p.expires_at_ms >= r.created_at_ms
            && p.expires_at_ms <= r.created_at_ms.saturating_add(30 * 60 * 1000),
        "Management plan expiry invalid",
    )?;
    require(
        r.task.is_none() || p.registered,
        "Management dispatch lacks registered recovery endpoint",
    )?;
    if let Some(task) = &r.task {
        xapi::reply_reference(&json!(task))?;
    }
    require(
        p.task_uuid.is_none_or(|id| {
            !id.uuid().is_nil() && r.task.is_some() && !p.tasks_before.contains(&id)
        }),
        "Management task identity invalid",
    )?;
    require(
        (!p.completed || p.ended)
            && (!p.ended || p.task_uuid.is_some())
            && (!p.configuration_observed || p.completed)
            && (!p.route_verified || p.configuration_observed),
        "Management completion proof invalid",
    )?;
    require(
        (r.steps[0].state == StepState::Verified) == p.route_verified
            && (matches!(r.steps[0].state, StepState::Pending | StepState::Failed) || p.registered),
        "Management step evidence differs from its private plan",
    )?;
    require(
        r.state != OperationState::Succeeded
            || p.route_verified && r.steps[0].state == StepState::Verified,
        "Management success lacks recovery verification",
    )?;
    Ok(())
}
fn addresses(value: &Value) -> Result<Vec<String>, Error> {
    let values = value
        .as_array()
        .filter(|v| v.len() <= 64)
        .ok_or(Error::Protocol("IPv6 addresses unavailable"))?
        .iter()
        .map(|v| v.as_str().map(String::from))
        .collect::<Option<Vec<_>>>()
        .ok_or(Error::Protocol("IPv6 addresses invalid"))?;
    canonical_ipv6(&values).ok_or(Error::Protocol("IPv6 addresses invalid"))
}
async fn pif(s: &Session, before: &ManagementInterface) -> Result<String, Error> {
    let (reference, v) = xapi::read(s, ResourceTarget::Pif(before.pif)).await?;
    network::pif_identity(s, &v, &before.identity).await?;
    require(
        v["primary_address_type"] == before.primary.as_str()
            && v["currently_attached"].as_bool() == Some(before.attached)
            && v["disallow_unplug"].as_bool() == Some(before.protected),
        "Management interface state changed since review",
    )?;
    require(
        v["ip_configuration_mode"] == before.ipv4.mode.as_str()
            && v["IP"] == before.ipv4.address
            && v["netmask"] == before.ipv4.netmask
            && v["gateway"] == before.ipv4.gateway
            && v["DNS"] == before.ipv4.dns
            && v["ipv6_configuration_mode"] == before.ipv6.mode.as_str()
            && addresses(&v["IPv6"])?
                == canonical_ipv6(&before.ipv6.addresses).ok_or(Error::InvalidResourceCommand)?
            && v["ipv6_gateway"] == before.ipv6.gateway,
        "Management IP settings changed since review",
    )?;
    require(
        v["managed"] == true
            && v["physical"] == true
            && network::number(&v["MTU"]) == Some(1500)
            && network::number(&v["VLAN"]) == Some(-1)
            && v["bond_slave_of"] == "OpaqueRef:NULL"
            && v["VLAN_master_of"] == "OpaqueRef:NULL",
        "This handover requires ordinary physical interfaces at MTU 1500",
    )?;
    for field in [
        "bond_master_of",
        "VLAN_slave_of",
        "tunnel_access_PIF_of",
        "tunnel_transport_PIF_of",
    ] {
        require(
            network::refs(&v[field])?.is_empty(),
            "Management topology requires a separate reviewed workflow",
        )?;
    }
    let config = v["other_config"]
        .as_object()
        .filter(|v| v.len() <= 1024)
        .ok_or(Error::Protocol("Management interface metadata unavailable"))?;
    require(
        !config.contains_key("management_purpose"),
        "Interface already has a storage or service purpose",
    )?;
    Ok(reference)
}
async fn graph(
    s: &Session,
    r: &ResourceOperationRecord,
    after: bool,
) -> Result<Vec<String>, Error> {
    let op = operation(r)?;
    let (_, pool) = xapi::read(s, ResourceTarget::Pool(r.pool)).await?;
    require(
        pool["ha_enabled"] == false,
        "Disable HA before changing management networking",
    )?;
    let (host_ref, host) = xapi::read(s, r.target).await?;
    let mut source = op.source().clone();
    let mut destination = None;
    if after {
        match op {
            ManagementNetworkOperation::Move {
                destination: dest, ..
            } => {
                source.identity.management = false;
                let mut dest = dest.clone();
                dest.identity.management = true;
                destination = Some(dest);
            }
            ManagementNetworkOperation::ConfigureIpv4 { configuration, .. } => {
                source.ipv4 = configuration.clone();
                source.ipv6.dns = configuration.dns.clone();
            }
            ManagementNetworkOperation::ConfigureIpv6 { configuration, .. } => {
                source.ipv6 = configuration.clone();
                source.ipv4.dns = configuration.dns.clone();
            }
        }
    } else if let ManagementNetworkOperation::Move {
        destination: dest, ..
    } = op
    {
        destination = Some(dest.clone());
    }
    let mut refs = vec![pif(s, &source).await?];
    if let Some(dest) = &destination {
        refs.push(pif(s, dest).await?);
    }
    let expected = if after {
        op.recovery_address()
    } else {
        op.source().address()
    };
    require(
        host["address"].as_str().and_then(|v| v.parse().ok()) == expected,
        "Host management address differs from the reviewed configuration",
    )?;
    let pifs = network::all(s, "PIF").await?;
    let recovery = op.recovery_address().ok_or(Error::InvalidResourceCommand)?;
    let receiver = match op {
        ManagementNetworkOperation::Move { destination, .. } => destination.pif,
        _ => source.pif,
    };
    for value in pifs.values() {
        if network::uuid(value)? == receiver.uuid() {
            continue;
        }
        let ipv4 = value["IP"]
            .as_str()
            .ok_or(Error::Protocol("Pool interface IP inventory unavailable"))?;
        let collides = ipv4.parse::<std::net::IpAddr>().ok() == Some(recovery)
            || addresses(&value["IPv6"])?.iter().any(|v| {
                v.split_once('/')
                    .and_then(|(ip, _)| ip.parse::<std::net::IpAddr>().ok())
                    == Some(recovery)
            });
        require(
            !collides,
            "Recovery address is already configured on another pool interface",
        )?;
    }

    let management = pifs
        .iter()
        .filter(|(_, v)| v["host"] == host_ref && v["management"] == true)
        .map(|(r, _)| r.as_str())
        .collect::<Vec<_>>();
    require(
        management
            == vec![if after && destination.is_some() {
                refs[1].as_str()
            } else {
                refs[0].as_str()
            }],
        "Host management interface identity is ambiguous",
    )?;
    Ok(refs)
}
pub(super) async fn unique_certificate(
    s: &Session,
    host: HostId,
) -> Result<crate::CertificateFingerprint, Error> {
    let hosts = network::all(s, "host").await?;
    require(
        !hosts.is_empty() && hosts.len() <= 128,
        "Pool host set unavailable or exceeds handover limit",
    )?;
    let ids = hosts
        .values()
        .map(|v| network::uuid(v).map(HostId::from))
        .collect::<Result<BTreeSet<_>, _>>()?;
    require(
        ids.len() == hosts.len() && ids.contains(&host),
        "Reviewed host is not an exact current pool member",
    )?;
    let certificate = crate::security_discovery::certificate(s, host).await?;
    for value in hosts.values() {
        let id = HostId::from(network::uuid(value)?);
        if id != host {
            require(
                crate::security_discovery::certificate(s, id).await? != certificate,
                "Host certificates are shared; the recovery address cannot be attributed to the reviewed host",
            )?;
        }
    }
    Ok(certificate)
}
pub(super) async fn prepare(s: &Session, r: &ResourceOperationRecord) -> Result<Plan, Error> {
    let op = operation(r)?;
    op.validate(r.target)?;
    graph(s, r, false).await?;
    let recovery = op.endpoint(&s.options().endpoint)?;
    require(
        recovery.as_str().starts_with("https://"),
        "Management handover requires HTTPS for host identity verification",
    )?;
    let host = op.source().identity.host;
    let certificate = unique_certificate(s, host).await?;
    let tasks_before = network::all(s, "task")
        .await?
        .values()
        .map(|v| network::uuid(v).map(Into::into))
        .collect::<Result<BTreeSet<_>, _>>()?;
    Ok(Plan {
        pool: r.pool,
        host,
        operation: op.clone(),
        recovery,
        original_endpoint: s.options().endpoint.clone(),
        certificate,
        configuration_observed: false,
        route_verified: false,
        tasks_before,
        task_uuid: None,
        completed: false,
        ended: false,
        registered: false,
        expires_at_ms: super::topology::now() + 10 * 60 * 1000,
    })
}
pub(super) async fn arguments(
    s: &Session,
    r: &ResourceOperationRecord,
) -> Result<(&'static str, Vec<Value>), Error> {
    let refs = graph(s, r, false).await?;
    Ok(match operation(r)? {
        ManagementNetworkOperation::Move { .. } => {
            ("Async.host.management_reconfigure", vec![json!(refs[1])])
        }
        ManagementNetworkOperation::ConfigureIpv4 {
            configuration: c, ..
        } => (
            "Async.PIF.reconfigure_ip",
            vec![
                json!(refs[0]),
                json!(c.mode.as_str()),
                json!(c.address),
                json!(c.netmask),
                json!(c.gateway),
                json!(c.dns),
            ],
        ),
        ManagementNetworkOperation::ConfigureIpv6 {
            configuration: c, ..
        } => (
            "Async.PIF.reconfigure_ipv6",
            vec![
                json!(refs[0]),
                json!(c.mode.as_str()),
                json!(c.addresses[0]),
                json!(c.gateway),
                json!(c.dns),
            ],
        ),
    })
}
pub(super) async fn verify(s: &Session, r: &ResourceOperationRecord) -> Result<(), Error> {
    graph(s, r, true).await.map(|_| ())
}
