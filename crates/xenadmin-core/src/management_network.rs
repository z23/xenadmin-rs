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
    #[serde(default)]
    pub topology: Option<Box<crate::ManagementTopology>>,
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
            topology: None,
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
    pub fn capture(inventory: &Inventory, id: PifId) -> Option<Self> {
        let p = inventory.details.pifs.get(&id)?;
        let mut value = Self::from_inventory(p)?;
        if let Some(topology) = crate::ManagementTopology::capture(&inventory.details, id) {
            if topology.pifs.values().any(|p| {
                p.network
                    .is_none_or(|id| !inventory.networks.iter().any(|n| n.uuid == id.uuid()))
            }) {
                return None;
            }
            if !topology.bonds.is_empty() || !topology.vlans.is_empty() {
                value.topology = Some(Box::new(topology));
            }
        } else if p.physical != Some(true)
            || p.vlan != Some(-1)
            || inventory
                .details
                .bonds
                .values()
                .any(|b| b.master == Some(id) || b.slaves.contains(&id))
            || inventory
                .details
                .vlans
                .values()
                .any(|v| v.tagged_pif == Some(id) || v.untagged_pif == Some(id))
        {
            return None;
        }
        Some(value)
    }
    pub fn address(&self) -> Option<IpAddr> {
        match self.primary {
            ManagementAddressFamily::Ipv4 if self.ipv4.mode != Ipv4Mode::None => {
                self.ipv4.address.parse().ok()
            }
            ManagementAddressFamily::Ipv6 if self.ipv6.mode != Ipv6Mode::None => {
                let mut addresses = self
                    .ipv6
                    .addresses
                    .iter()
                    .filter_map(|v| {
                        crate::network_types::ipv6_cidr(v).map(|(ip, _)| IpAddr::V6(ip))
                    })
                    .filter(usable_address);
                let first = addresses.next()?;
                addresses.next().is_none().then_some(first)
            }
            _ => None,
        }
        .filter(usable_address)
    }
    pub(crate) fn bounded(&self) -> bool {
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
            && self
                .ipv4
                .dns
                .split(',')
                .filter(|v| !v.is_empty())
                .all(|v| v.parse::<IpAddr>().is_ok())
            && self
                .topology
                .as_ref()
                .is_none_or(|t| t.valid(self.pif) && t.matches_root(self))
    }
    pub(crate) fn valid(&self) -> bool {
        self.bounded()
            && self.address().is_some()
            && match self.primary {
                ManagementAddressFamily::Ipv4 => Ipv4Configuration {
                    mode: Ipv4Mode::Static,
                    ..self.ipv4.clone()
                }
                .valid(),
                ManagementAddressFamily::Ipv6 => {
                    self.ipv6.observed_valid()
                        && (self.ipv6.gateway.is_empty()
                            || self.ipv6.gateway.parse::<std::net::Ipv6Addr>().is_ok())
                }
            }
    }
}
fn dns_family(value: &str, ipv4: bool) -> BTreeSet<IpAddr> {
    value
        .split(',')
        .filter_map(|v| v.parse::<IpAddr>().ok())
        .filter(|v| v.is_ipv4() == ipv4)
        .collect()
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ManagementDynamicMode {
    Ipv4Dhcp,
    Ipv6Dhcp,
    Ipv6Autoconf,
}
impl ManagementDynamicMode {
    pub fn family(self) -> ManagementAddressFamily {
        match self {
            Self::Ipv4Dhcp => ManagementAddressFamily::Ipv4,
            _ => ManagementAddressFamily::Ipv6,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ipv6Autoconf => "Autoconf",
            _ => "DHCP",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Ipv4Dhcp => "IPv4 DHCP",
            Self::Ipv6Dhcp => "IPv6 DHCP",
            Self::Ipv6Autoconf => "IPv6 autoconfiguration",
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
    Bond {
        source: ManagementInterface,
        operation: crate::ManagementBondOperation,
    },
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
    ConfigureDynamic {
        interface: ManagementInterface,
        mode: ManagementDynamicMode,
        recovery: IpAddr,
    },
}
impl ManagementNetworkOperation {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Bond { operation, .. } => {
                if operation.creates() {
                    "Create management bond"
                } else {
                    "Remove management bond"
                }
            }
            Self::Move { .. } => "Move host management interface",
            Self::ConfigureIpv4 { .. } => "Change management IPv4 settings",
            Self::ConfigureIpv6 { .. } => "Change management IPv6 settings",
            Self::ConfigureDynamic { .. } => "Change automatic management addressing",
        }
    }
    pub fn source(&self) -> &ManagementInterface {
        match self {
            Self::Move { source, .. } | Self::Bond { source, .. } => source,
            Self::ConfigureIpv4 { interface, .. }
            | Self::ConfigureIpv6 { interface, .. }
            | Self::ConfigureDynamic { interface, .. } => interface,
        }
    }
    pub fn recovery_address(&self) -> Option<IpAddr> {
        match self {
            Self::Move { destination, .. } => destination.address(),
            Self::Bond { source, .. } => source.address(),
            Self::ConfigureDynamic { recovery, .. } => Some(*recovery).filter(usable_address),
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
                Self::Bond { operation, .. } => operation.valid(source),
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
                        && match destination.primary {
                            ManagementAddressFamily::Ipv4 => {
                                destination.ipv4.mode == Ipv4Mode::Static
                            }
                            ManagementAddressFamily::Ipv6 => {
                                destination.ipv6.mode == Ipv6Mode::Static
                            }
                        }
                }
                Self::ConfigureIpv4 { configuration, .. } => {
                    source.primary == ManagementAddressFamily::Ipv4
                        && configuration.mode == Ipv4Mode::Static
                        && configuration.valid()
                        && (configuration.mode != source.ipv4.mode
                            || configuration.address != source.ipv4.address
                            || configuration.netmask != source.ipv4.netmask
                            || configuration.gateway != source.ipv4.gateway
                            || dns_family(&configuration.dns, true)
                                != dns_family(&source.ipv4.dns, true))
                }
                Self::ConfigureIpv6 { configuration, .. } => {
                    source.primary == ManagementAddressFamily::Ipv6
                        && configuration.mode == Ipv6Mode::Static
                        && configuration.valid()
                        && (configuration.mode != source.ipv6.mode
                            || canonical_ipv6(&configuration.addresses)
                                != canonical_ipv6(&source.ipv6.addresses)
                            || configuration.gateway != source.ipv6.gateway
                            || dns_family(&configuration.dns, false)
                                != dns_family(&source.ipv6.dns, false))
                }
                Self::ConfigureDynamic { mode, recovery, .. } => {
                    mode.family() == source.primary
                        && matches!(
                            (mode.family(), recovery),
                            (ManagementAddressFamily::Ipv4, IpAddr::V4(_))
                                | (ManagementAddressFamily::Ipv6, IpAddr::V6(_))
                        )
                        && match mode {
                            ManagementDynamicMode::Ipv4Dhcp => source.ipv4.mode != Ipv4Mode::Dhcp,
                            ManagementDynamicMode::Ipv6Dhcp => source.ipv6.mode != Ipv6Mode::Dhcp,
                            ManagementDynamicMode::Ipv6Autoconf => {
                                source.ipv6.mode != Ipv6Mode::Autoconf
                            }
                        }
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
            if matches!(self, Self::Bond { .. }) {
                return ActionAvailability::Prerequisite { reason: "Management bond handover requires static IPv4 on an unprotected management interface, no IPv6 settings, and 2–4 physical members for creation. Management VLANs need unique tags and complete reviewed topology; other VLANs may use static IPv4 or have no IP. Only the management endpoint may have the management role. Removal requires the exact bond and primary member.".into() };
            }
            return ActionAvailability::Prerequisite{reason:"Review a changed management configuration in the installed address family and a usable recovery IP. The current primary address must be unambiguous. Moving management requires a different, already statically configured and plugged interface with a distinct address.".into()};
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
        if let Self::Bond { source, operation } = self {
            let availability = operation.availability(source, i);
            if !availability.is_available() {
                return availability;
            }
        }
        let sources = match self {
            Self::Bond {
                operation: crate::ManagementBondOperation::Create { members, .. },
                ..
            } => members.iter().collect(),
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
            if (if expected.topology.is_some() || matches!(self, Self::Bond { .. }) {
                ManagementInterface::capture(i, expected.pif)
            } else {
                ManagementInterface::from_inventory(p)
            })
            .as_ref()
                != Some(expected)
            {
                return ActionAvailability::Stale;
            }
            if p.managed != Some(true)
                || p.mtu != Some(1500)
                || (expected.topology.is_none() && (p.physical != Some(true) || p.vlan != Some(-1)))
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
    #[serde(default)]
    pub bond: Option<crate::management_bond::Baseline>,
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
        self.bond
            .as_ref()
            .map(|b| next.bond.as_ref().is_some_and(|n| b.accepts(n)))
            .unwrap_or(next.bond.is_none())
            && self.certificate == next.certificate
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
    #[serde(default)]
    pub created_bond: Option<crate::ManagementBondCreated>,
    #[serde(default)]
    pub moved_vlans: Vec<crate::ManagementVlanReplacement>,
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
            created_bond: self.bond.as_ref().and_then(|b| b.created),
            moved_vlans: self
                .bond
                .as_ref()
                .map(|b| b.replacements.values().copied().collect())
                .unwrap_or_default(),
            configuration_observed: self.configuration_observed,
            recovery_endpoint_verified: self.route_verified,
            recovery_endpoint: Some(self.recovery.clone()),
        }
    }
}
