use crate::{
    ActionAvailability, Endpoint, Error, HostId, Inventory, Ipv4Configuration, Ipv4Mode,
    Ipv6Configuration, Ipv6Mode, PhysicalInterface, PifId, PifIdentity, ResourceTarget, TaskId,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, net::IpAddr};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ManagementAddressFamily {
    Ipv4,
    Ipv6,
}
impl ManagementAddressFamily {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ipv4 => "IPv4",
            Self::Ipv6 => "IPv6",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagementInterface {
    pub pif: PifId,
    pub identity: PifIdentity,
    pub primary: ManagementAddressFamily,
    pub ipv4: Ipv4Configuration,
    pub ipv6: Ipv6Configuration,
    pub attached: bool,
    pub protected: bool,
}
impl ManagementInterface {
    pub fn from_inventory(p: &PhysicalInterface) -> Option<Self> {
        Some(Self {
            pif: p.id,
            identity: PifIdentity::from_inventory(p)?,
            primary: match p.primary_address_type.as_deref()? {
                "IPv4" => ManagementAddressFamily::Ipv4,
                "IPv6" => ManagementAddressFamily::Ipv6,
                _ => return None,
            },
            ipv4: Ipv4Configuration {
                mode: match p.ip_configuration_mode.as_str() {
                    "None" => Ipv4Mode::None,
                    "DHCP" => Ipv4Mode::Dhcp,
                    "Static" => Ipv4Mode::Static,
                    _ => return None,
                },
                address: p.ip.clone(),
                netmask: p.netmask.clone(),
                gateway: p.gateway.clone(),
                dns: p.dns.clone(),
            },
            ipv6: Ipv6Configuration {
                mode: match p.ipv6_configuration_mode.as_str() {
                    "None" => Ipv6Mode::None,
                    "DHCP" => Ipv6Mode::Dhcp,
                    "Autoconf" => Ipv6Mode::Autoconf,
                    "Static" => Ipv6Mode::Static,
                    _ => return None,
                },
                addresses: canonical_ipv6(&p.ipv6)?,
                gateway: p.ipv6_gateway.clone(),
                dns: p.dns.clone(),
            },
            attached: p.currently_attached?,
            protected: p.disallow_unplug?,
        })
    }
    pub fn address(&self) -> Option<IpAddr> {
        match self.primary {
            ManagementAddressFamily::Ipv4 if self.ipv4.mode == Ipv4Mode::Static => {
                self.ipv4.address.parse().ok()
            }
            ManagementAddressFamily::Ipv6
                if self.ipv6.mode == Ipv6Mode::Static && self.ipv6.addresses.len() == 1 =>
            {
                crate::network_types::ipv6_cidr(&self.ipv6.addresses[0])
                    .map(|(ip, _)| IpAddr::V6(ip))
            }
            _ => None,
        }
        .filter(usable_address)
    }
    fn valid(&self) -> bool {
        !self.pif.uuid().is_nil()
            && !self.identity.host.uuid().is_nil()
            && !self.identity.network.uuid().is_nil()
            && !self.identity.device.is_empty()
            && self.identity.device.len() <= 256
            && self.identity.mac.len() <= 64
            && self.ipv4.address.len() <= 128
            && self.ipv4.netmask.len() <= 128
            && self.ipv4.gateway.len() <= 128
            && self.ipv4.dns.len() <= 2048
            && self.ipv6.observed_valid()
            && self.ipv4.dns == self.ipv6.dns
            && self.address().is_some()
            && match self.primary {
                ManagementAddressFamily::Ipv4 => self.ipv4.valid(),
                ManagementAddressFamily::Ipv6 => self.ipv6.valid(),
            }
    }
}
pub(crate) fn canonical_ipv6(addresses: &[String]) -> Option<Vec<String>> {
    addresses
        .iter()
        .filter(|v| !v.is_empty())
        .map(|v| crate::network_types::ipv6_cidr(v).map(|(ip, prefix)| format!("{ip}/{prefix}")))
        .collect::<Option<BTreeSet<_>>>()
        .map(|v| v.into_iter().collect())
}
fn usable_address(ip: &IpAddr) -> bool {
    !ip.is_unspecified()
        && !ip.is_multicast()
        && match ip {
            IpAddr::V4(v) => !v.is_broadcast(),
            IpAddr::V6(v) => !v.is_unicast_link_local(),
        }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ManagementNetworkOperation {
    Move {
        source: ManagementInterface,
        destination: ManagementInterface,
    },
    ConfigureIpv4 {
        interface: ManagementInterface,
        configuration: Ipv4Configuration,
    },
    ConfigureIpv6 {
        interface: ManagementInterface,
        configuration: Ipv6Configuration,
    },
}
impl ManagementNetworkOperation {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Move { .. } => "Move host management interface",
            Self::ConfigureIpv4 { .. } => "Change management IPv4 settings",
            Self::ConfigureIpv6 { .. } => "Change management IPv6 settings",
        }
    }
    pub fn source(&self) -> &ManagementInterface {
        match self {
            Self::Move { source, .. } => source,
            Self::ConfigureIpv4 { interface, .. } | Self::ConfigureIpv6 { interface, .. } => {
                interface
            }
        }
    }
    pub fn recovery_address(&self) -> Option<IpAddr> {
        match self {
            Self::Move { destination, .. } => destination.address(),
            Self::ConfigureIpv4 { configuration, .. } => {
                configuration.address.parse().ok().filter(usable_address)
            }
            Self::ConfigureIpv6 { configuration, .. } => configuration
                .addresses
                .first()
                .and_then(|v| crate::network_types::ipv6_cidr(v))
                .map(|(ip, _)| IpAddr::V6(ip))
                .filter(usable_address),
        }
    }
    pub(crate) fn validate(&self, target: ResourceTarget) -> Result<(), Error> {
        let source = self.source();
        let valid = target == ResourceTarget::Host(source.identity.host)
            && source.valid()
            && source.identity.management
            && source.attached
            && self.recovery_address().is_some()
            && match self {
                Self::Move { destination, .. } => {
                    destination.valid()
                        && !destination.identity.management
                        && destination.attached
                        && !destination.protected
                        && destination.pif != source.pif
                        && destination.identity.host == source.identity.host
                        && destination.identity.network != source.identity.network
                        && destination.primary == source.primary
                        && destination.address() != source.address()
                }
                Self::ConfigureIpv4 { configuration, .. } => {
                    source.primary == ManagementAddressFamily::Ipv4
                        && configuration.mode == Ipv4Mode::Static
                        && configuration.valid()
                        && configuration != &source.ipv4
                }
                Self::ConfigureIpv6 { configuration, .. } => {
                    source.primary == ManagementAddressFamily::Ipv6
                        && configuration.mode == Ipv6Mode::Static
                        && configuration.valid()
                        && configuration != &source.ipv6
                }
            };
        if valid {
            Ok(())
        } else {
            Err(Error::InvalidResourceCommand)
        }
    }
    pub fn availability(&self, i: &Inventory) -> ActionAvailability {
        if self
            .validate(ResourceTarget::Host(self.source().identity.host))
            .is_err()
        {
            return ActionAvailability::Prerequisite{reason:"Review a changed static management configuration in the installed address family. Moving management requires a different, already configured and plugged interface with a distinct address.".into()};
        }
        let Some(pool) = &i.details.pool else {
            return ActionAvailability::Unknown;
        };
        if pool.ha_enabled.is_none() {
            return ActionAvailability::Unknown;
        }
        if pool.ha_enabled != Some(false) {
            return ActionAvailability::Prerequisite {
                reason: "Disable HA before changing host management networking".into(),
            };
        }
        let sources = match self {
            Self::Move {
                source,
                destination,
            } => vec![source, destination],
            _ => vec![self.source()],
        };
        for expected in sources {
            let Some(p) = i.details.pifs.get(&expected.pif) else {
                return ActionAvailability::Stale;
            };
            if ManagementInterface::from_inventory(p).as_ref() != Some(expected) {
                return ActionAvailability::Stale;
            }
            if p.managed != Some(true)
                || p.mtu != Some(1500)
                || p.physical != Some(true)
                || p.vlan != Some(-1)
                || p.management_purpose.is_some()
            {
                return ActionAvailability::Prerequisite{reason:"Management handover requires managed interfaces at MTU 1500 without a separate storage or service purpose".into()};
            }
        }
        let receiver = match self {
            Self::Move { destination, .. } => destination.pif,
            _ => self.source().pif,
        };
        if i.details.pifs.values().any(|p| {
            p.id != receiver
                && (p.ip.parse::<IpAddr>().ok() == self.recovery_address()
                    || p.ipv6.iter().any(|v| {
                        crate::network_types::ipv6_cidr(v).map(|(ip, _)| IpAddr::V6(ip))
                            == self.recovery_address()
                    }))
        }) {
            return ActionAvailability::Prerequisite {
                reason: "Recovery address is already configured on another pool interface".into(),
            };
        }
        if !i.hosts.contains_key(&self.source().identity.host) {
            return ActionAvailability::Stale;
        }
        ActionAvailability::Available
    }
    pub(crate) fn endpoint(&self, base: &Endpoint) -> Result<Endpoint, Error> {
        let mut url = url::Url::parse(base.as_str()).map_err(|_| Error::InvalidEndpoint)?;
        let ip = self
            .recovery_address()
            .ok_or(Error::InvalidResourceCommand)?;
        url.set_ip_host(ip).map_err(|_| Error::InvalidEndpoint)?;
        Endpoint::parse(url.as_str())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Plan {
    pub pool: crate::PoolId,
    pub host: HostId,
    pub operation: ManagementNetworkOperation,
    pub recovery: Endpoint,
    pub original_endpoint: Endpoint,
    pub certificate: crate::CertificateFingerprint,
    pub configuration_observed: bool,
    pub route_verified: bool,
    pub tasks_before: BTreeSet<TaskId>,
    pub task_uuid: Option<TaskId>,
    pub completed: bool,
    pub ended: bool,
    pub registered: bool,
    pub expires_at_ms: u64,
}
impl Plan {
    pub(crate) fn accepts(&self, next: &Self) -> bool {
        self.certificate == next.certificate
            && (!self.configuration_observed || next.configuration_observed)
            && (!self.route_verified || next.route_verified)
            && self.pool == next.pool
            && self.host == next.host
            && self.operation == next.operation
            && self.recovery == next.recovery
            && self.original_endpoint == next.original_endpoint
            && self.tasks_before == next.tasks_before
            && self.expires_at_ms == next.expires_at_ms
            && self.task_uuid.is_none_or(|id| next.task_uuid == Some(id))
            && (!self.completed || next.completed)
            && (!self.ended || next.ended)
            && (!self.registered || next.registered)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagementNetworkEvidence {
    pub configuration_observed: bool,
    pub recovery_endpoint_verified: bool,
    pub recovery_endpoint: Option<Endpoint>,
}
pub(crate) struct Inputs {
    pub options: crate::ConnectionOptions,
    pub credentials: crate::Credentials,
    pub route: crate::connection::ManagementRoute,
}
impl Plan {
    pub fn evidence(&self) -> ManagementNetworkEvidence {
        ManagementNetworkEvidence {
            configuration_observed: self.configuration_observed,
            recovery_endpoint_verified: self.route_verified,
            recovery_endpoint: Some(self.recovery.clone()),
        }
    }
}
