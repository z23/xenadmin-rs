use super::{network, xapi};
use crate::{Error, NetworkServiceReview, ResourceOperationRecord, ResourceTarget, xapi::Session};
use serde_json::Value;
use std::collections::BTreeSet;
use xenadmin_xapi::records::{NetworkRecord, PifRecord};

async fn interfaces(
    s: &Session,
    network_ref: &str,
    fresh: &Value,
    review: &NetworkServiceReview,
) -> Result<(), Error> {
    if NetworkRecord::new(fresh).managed().as_bool() != Some(true) {
        return Err(Error::UnavailableAction);
    }
    let pifs = network::all(s, "PIF").await?;
    let actual = pifs
        .iter()
        .filter(|(_, p)| p["network"].as_str() == Some(network_ref))
        .collect::<Vec<_>>();
    let refs = actual
        .iter()
        .map(|(r, _)| (*r).clone())
        .collect::<BTreeSet<_>>();
    let ids = actual
        .iter()
        .map(|(_, p)| network::uuid(p).map(crate::PifId::from))
        .collect::<Result<BTreeSet<_>, _>>()?;
    if refs != network::refs(&NetworkRecord::new(fresh).pifs())?
        || actual.len() != review.interfaces.len()
        || ids != review.interfaces.iter().map(|p| p.pif).collect()
    {
        return Err(Error::ResourceConflict);
    }
    for (_, pif) in actual {
        let id = crate::PifId::from(network::uuid(pif)?);
        let expected = review
            .interfaces
            .iter()
            .find(|p| p.pif == id)
            .ok_or(Error::ResourceConflict)?;
        network::pif_identity(s, pif, &expected.identity).await?;
        let ipv4 = PifRecord::new(pif)
            .ip()
            .value()
            .as_str()
            .ok_or(Error::ResourceConflict)?;
        let ipv4 = if ipv4.is_empty() {
            None
        } else {
            Some(ipv4.parse().map_err(|_| Error::ResourceConflict)?)
        };
        let ipv6 = PifRecord::new(pif)
            .ipv6()
            .value()
            .as_array()
            .filter(|v| v.len() <= 64)
            .ok_or(Error::ResourceConflict)?;
        let ipv6 = ipv6
            .iter()
            .map(Value::as_str)
            .collect::<Option<Vec<_>>>()
            .ok_or(Error::ResourceConflict)?;
        if PifRecord::new(pif).managed().as_bool() != Some(true)
            || PifRecord::new(pif).currently_attached().as_bool() != Some(expected.attached)
            || ipv4 != expected.ipv4
            || crate::network_service::canonical_ipv6(ipv6.into_iter()).as_ref()
                != Some(&expected.ipv6)
        {
            return Err(Error::ResourceConflict);
        }
    }
    Ok(())
}

pub(super) async fn check(
    s: &Session,
    record: &ResourceOperationRecord,
    network_ref: &str,
    verifying: bool,
) -> Result<(), Error> {
    let crate::NetworkOperation::SetServicePurpose {
        review,
        purpose,
        enabled,
    } = network::operation(record)?
    else {
        return Err(Error::InvalidResourceCommand);
    };
    xapi::read(s, ResourceTarget::Pool(record.pool)).await?;
    let networks = network::all(s, "network").await?;
    let ids = networks
        .values()
        .map(network::uuid)
        .collect::<Result<BTreeSet<_>, _>>()?;
    let fresh = networks.get(network_ref).ok_or(Error::ResourceConflict)?;
    if ids.len() != networks.len() || network::uuid(fresh)? != record.target.uuid() {
        return Err(Error::ResourceConflict);
    }
    let expected = if verifying {
        review.after(*purpose, *enabled)
    } else {
        review.purposes.clone()
    };
    if crate::network_service::purposes(&NetworkRecord::new(fresh).purpose()).as_ref()
        != Some(&expected)
    {
        return Err(Error::ResourceConflict);
    }
    interfaces(s, network_ref, fresh, review).await?;
    if *enabled {
        for n in networks.values() {
            let purposes = crate::network_service::purposes(&NetworkRecord::new(n).purpose())
                .ok_or(Error::Protocol(
                    "Pool network service purposes are unavailable",
                ))?;
            if purposes.contains(purpose.opposite()) {
                return Err(Error::Protocol(
                    "Another network service policy conflicts with this NBD mode; review and remove that policy separately",
                ));
            }
        }
    }
    Ok(())
}
