use crate::{Error, Inventory, NetworkId, PhysicalInterface, PifId, PifIdentity};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, net::Ipv4Addr};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NetworkServicePurpose {
    NbdTls,
    NbdPlaintext,
}
impl NetworkServicePurpose {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NbdTls => "nbd",
            Self::NbdPlaintext => "insecure_nbd",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::NbdTls => "NBD with TLS",
            Self::NbdPlaintext => "NBD without TLS",
        }
    }
    pub(crate) fn opposite(self) -> &'static str {
        match self {
            Self::NbdTls => "insecure_nbd",
            Self::NbdPlaintext => "nbd",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkServiceInterface {
    pub pif: PifId,
    pub identity: PifIdentity,
    pub attached: bool,
    pub ipv4: Option<Ipv4Addr>,
    pub ipv6: BTreeSet<String>,
}
impl NetworkServiceInterface {
    fn capture(p: &PhysicalInterface) -> Option<Self> {
        if p.managed != Some(true) {
            return None;
        }
        Some(Self {
            pif: p.id,
            identity: PifIdentity::from_inventory(p)?,
            attached: p.currently_attached?,
            ipv4: if p.ip.is_empty() {
                None
            } else {
                Some(p.ip.parse().ok()?)
            },
            ipv6: canonical_ipv6(p.ipv6.iter().map(String::as_str))?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkServiceReview {
    pub purposes: BTreeSet<String>,
    pub interfaces: Vec<NetworkServiceInterface>,
}
impl NetworkServiceReview {
    pub fn capture(i: &Inventory, network: NetworkId) -> Result<Self, Error> {
        if !["network", "pif"].iter().all(|class| {
            i.details.classes.get(*class) == Some(&crate::ClassAvailability::Available)
        }) {
            return Err(Error::Protocol(
                "Network service interface inventory is unavailable",
            ));
        }
        let n = i
            .details
            .networks
            .get(&network)
            .ok_or(Error::ResourceConflict)?;
        if n.managed != Some(true) {
            return Err(Error::UnavailableAction);
        }
        let review = Self {
            purposes: n
                .purposes
                .clone()
                .ok_or(Error::Protocol("Network service purposes are unavailable"))?,
            interfaces: i
                .details
                .pifs
                .values()
                .filter(|p| p.network == Some(network))
                .map(NetworkServiceInterface::capture)
                .collect::<Option<Vec<_>>>()
                .ok_or(Error::Protocol(
                    "Network service interface details are incomplete",
                ))?,
        };
        if !review.valid(network) {
            return Err(Error::Protocol(
                "The network has unknown or invalid service purposes",
            ));
        }
        Ok(review)
    }
    pub(crate) fn valid(&self, network: NetworkId) -> bool {
        self.purposes
            .iter()
            .all(|p| matches!(p.as_str(), "nbd" | "insecure_nbd"))
            && self.purposes.len() <= 1
            && self.interfaces.len() <= 1024
            && self
                .interfaces
                .iter()
                .map(|p| p.pif)
                .collect::<BTreeSet<_>>()
                .len()
                == self.interfaces.len()
            && self.interfaces.iter().all(|p| {
                p.identity.network == network
                    && p.identity.device.len() <= 256
                    && p.identity.mac.len() <= 64
                    && canonical_ipv6(p.ipv6.iter().map(String::as_str)).as_ref() == Some(&p.ipv6)
            })
    }
    pub fn availability(
        &self,
        i: &Inventory,
        network: NetworkId,
        purpose: NetworkServicePurpose,
        enabled: bool,
    ) -> Result<(), Error> {
        if Self::capture(i, network)? != *self {
            return Err(Error::ResourceConflict);
        }
        if enabled {
            for network in &i.networks {
                let n = i
                    .details
                    .networks
                    .get(&NetworkId::from(network.uuid))
                    .ok_or(Error::Protocol(
                        "Pool network service purposes are unavailable",
                    ))?;
                let purposes = n.purposes.as_ref().ok_or(Error::Protocol(
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
    pub(crate) fn after(&self, purpose: NetworkServicePurpose, enabled: bool) -> BTreeSet<String> {
        let mut after = self.purposes.clone();
        if enabled {
            after.insert(purpose.as_str().into());
        } else {
            after.remove(purpose.as_str());
        }
        after
    }
}

pub(crate) fn canonical_ipv6<'a>(
    values: impl Iterator<Item = &'a str>,
) -> Option<BTreeSet<String>> {
    let values = values
        .filter(|v| !v.is_empty())
        .take(65)
        .map(|v| crate::network_types::ipv6_cidr(v).map(|(ip, prefix)| format!("{ip}/{prefix}")))
        .collect::<Option<Vec<_>>>()?;
    (values.len() <= 64).then(|| values.into_iter().collect())
}

pub(crate) fn purposes(value: &serde_json::Value) -> Option<BTreeSet<String>> {
    let values = value.as_array().filter(|v| v.len() <= 32)?;
    let result = values
        .iter()
        .map(|v| {
            v.as_str()
                .filter(|s| !s.is_empty() && s.len() <= 128 && !s.chars().any(char::is_control))
                .map(str::to_owned)
        })
        .collect::<Option<BTreeSet<_>>>()?;
    (result.len() == values.len()).then_some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    #[test]
    fn service_policy_preserves_unknown_names_but_distinguishes_missing_and_invalid_data() {
        assert_eq!(purposes(&json!([])), Some(BTreeSet::new()));
        assert_eq!(
            purposes(&json!(["future-service"])),
            Some(["future-service".into()].into())
        );
        for value in [
            Value::Null,
            json!(["nbd", "nbd"]),
            json!(["bad\nvalue"]),
            json!([false]),
            json!(["x".repeat(129)]),
        ] {
            assert!(purposes(&value).is_none());
        }
    }
    #[test]
    fn captured_ipv6_addresses_are_canonical_and_bounded_before_deduplication() {
        assert_eq!(
            canonical_ipv6(["2001:0DB8::1/64", "2001:db8::1/64"].into_iter()),
            Some(["2001:db8::1/64".into()].into())
        );
        assert!(canonical_ipv6(std::iter::repeat_n("2001:db8::1/64", 65)).is_none());
        assert!(canonical_ipv6(["not-an-address"].into_iter()).is_none());
    }
}
