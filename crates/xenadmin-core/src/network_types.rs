use crate::{BondId, Error, HostId, NetworkId, PifId, ResourceTarget};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BondMode {
    BalanceSlb,
    ActiveBackup,
    Lacp,
}
impl BondMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BalanceSlb => "balance-slb",
            Self::ActiveBackup => "active-backup",
            Self::Lacp => "lacp",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BondHash {
    SourceMac,
    TcpUdpPorts,
}
impl BondHash {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SourceMac => "src_mac",
            Self::TcpUdpPorts => "tcpudp_ports",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PifIdentity {
    pub host: HostId,
    pub network: NetworkId,
    pub device: String,
    pub mac: String,
    pub management: bool,
}
impl PifIdentity {
    pub fn from_inventory(pif: &crate::PhysicalInterface) -> Option<Self> {
        Some(Self {
            host: pif.host?,
            network: pif.network?,
            device: pif.device.clone(),
            mac: pif.mac.clone(),
            management: pif.management?,
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Ipv4Mode {
    None,
    Dhcp,
    Static,
}
impl Ipv4Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Dhcp => "DHCP",
            Self::Static => "Static",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ipv4Configuration {
    pub mode: Ipv4Mode,
    pub address: String,
    pub netmask: String,
    pub gateway: String,
    pub dns: String,
}
impl Ipv4Configuration {
    pub fn valid(&self) -> bool {
        if self.dns.len() > 2048
            || self
                .dns
                .split(',')
                .filter(|v| !v.is_empty())
                .any(|v| v.parse::<IpAddr>().is_err())
        {
            return false;
        }
        match self.mode {
            Ipv4Mode::None | Ipv4Mode::Dhcp => {
                self.address.is_empty() && self.netmask.is_empty() && self.gateway.is_empty()
            }
            Ipv4Mode::Static => {
                let Ok(mask) = self.netmask.parse::<Ipv4Addr>() else {
                    return false;
                };
                let inverse = !u32::from(mask);
                inverse & inverse.wrapping_add(1) == 0
                    && self.address.parse::<Ipv4Addr>().is_ok()
                    && (self.gateway.is_empty() || self.gateway.parse::<Ipv4Addr>().is_ok())
            }
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Ipv6Mode {
    None,
    Dhcp,
    Autoconf,
    Static,
}
impl Ipv6Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Dhcp => "DHCP",
            Self::Autoconf => "Autoconf",
            Self::Static => "Static",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ipv6Configuration {
    pub mode: Ipv6Mode,
    pub addresses: Vec<String>,
    pub gateway: String,
    pub dns: String,
}
impl Ipv6Configuration {
    pub fn valid(&self) -> bool {
        self.addresses.len() <= 1
            && self.addresses.iter().all(|v| ipv6_cidr(v).is_some())
            && (self.gateway.is_empty() || self.gateway.parse::<Ipv6Addr>().is_ok())
            && self.dns.len() <= 2048
            && self
                .dns
                .split(',')
                .filter(|v| !v.is_empty())
                .all(|v| v.parse::<IpAddr>().is_ok())
            && match self.mode {
                Ipv6Mode::Static => self.addresses.len() == 1,
                _ => self.addresses.is_empty() && self.gateway.is_empty(),
            }
    }
    pub(crate) fn observed_valid(&self) -> bool {
        self.addresses.len() <= 64
            && self.addresses.iter().all(|v| ipv6_cidr(v).is_some())
            && self.gateway.len() <= 128
            && self.dns.len() <= 2048
    }
}
pub(crate) fn ipv6_cidr(value: &str) -> Option<(Ipv6Addr, u8)> {
    let (ip, prefix) = value.split_once('/')?;
    let prefix = prefix.parse::<u8>().ok().filter(|p| *p <= 128)?;
    Some((ip.parse().ok()?, prefix))
}
fn purpose_valid(value: &Option<String>) -> bool {
    value
        .as_ref()
        .is_none_or(|v| !v.is_empty() && v.len() <= 256 && !v.chars().any(char::is_control))
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NetworkOperation {
    ConfigureIpv6 {
        expected: PifIdentity,
        before: Ipv6Configuration,
        configuration: Ipv6Configuration,
    },
    SetInterfacePurpose {
        expected: PifIdentity,
        before: Option<String>,
        purpose: Option<String>,
    },
    SetUnplugProtection {
        expected: PifIdentity,
        before: bool,
        enabled: bool,
    },
    ScanInterfaces,
    SetAttached {
        expected: PifIdentity,
        expected_attached: bool,
        attached: bool,
    },
    CreateVlan {
        parent: PifId,
        expected: PifIdentity,
        tag: u16,
    },
    DeleteVlan {
        parent: PifId,
        child: PifId,
        network: NetworkId,
        tag: u16,
    },
    CreateBond {
        members: Vec<(PifId, PifIdentity)>,
        mode: BondMode,
        hash: BondHash,
    },
    DeleteBond {
        master: PifId,
        members: Vec<PifId>,
        primary: PifId,
    },
    SetBondMode {
        expected_mode: String,
        mode: BondMode,
    },
    SetBondHash {
        expected: Option<String>,
        hash: BondHash,
    },
    SetAutomaticAttachment {
        expected: Option<String>,
        enabled: bool,
    },
    SetMtu {
        expected: u64,
        mtu: u64,
        pifs: Vec<PifId>,
    },
    SetIgmpSnooping {
        expected: bool,
        enabled: bool,
    },
    ConfigureIpv4 {
        expected: PifIdentity,
        before: Ipv4Configuration,
        configuration: Ipv4Configuration,
    },
}
impl NetworkOperation {
    pub fn label(&self) -> &'static str {
        match self {
            Self::ConfigureIpv6 { .. } => "Configure host IPv6 interface",
            Self::SetInterfacePurpose { .. } => "Set interface purpose",
            Self::SetUnplugProtection { .. } => "Set interface unplug protection",
            Self::ScanInterfaces => "Scan physical interfaces",
            Self::SetAttached { attached: true, .. } => "Plug host interface",
            Self::SetAttached {
                attached: false, ..
            } => "Unplug host interface",
            Self::CreateVlan { .. } => "Create VLAN interface",
            Self::DeleteVlan { .. } => "Delete VLAN interface",
            Self::CreateBond { .. } => "Create bond",
            Self::DeleteBond { .. } => "Destroy bond",
            Self::SetBondMode { .. } => "Change bond mode",
            Self::SetBondHash { .. } => "Change bond hash",
            Self::SetAutomaticAttachment { .. } => "Set automatic network attachment",
            Self::SetMtu { .. } => "Change network MTU",
            Self::SetIgmpSnooping { .. } => "Set pool IGMP snooping",
            Self::ConfigureIpv4 { .. } => "Configure host IPv4 interface",
        }
    }
    pub(crate) fn creates(&self) -> bool {
        matches!(self, Self::CreateVlan { .. } | Self::CreateBond { .. })
    }
    pub(crate) fn validate(&self, target: ResourceTarget) -> Result<(), Error> {
        let pif = matches!(target, ResourceTarget::Pif(_));
        let network = matches!(target, ResourceTarget::Network(_));
        let bond = matches!(target, ResourceTarget::Bond(_));
        let valid = match self {
            Self::ConfigureIpv6 {
                expected,
                before,
                configuration,
            } => {
                pif && !expected.management
                    && before.observed_valid()
                    && configuration.valid()
                    && before != configuration
            }
            Self::SetInterfacePurpose {
                expected,
                before,
                purpose,
            } => {
                pif && !expected.management
                    && before != purpose
                    && purpose_valid(before)
                    && purpose_valid(purpose)
            }
            Self::SetUnplugProtection {
                expected,
                before,
                enabled,
            } => pif && !expected.management && before != enabled,
            Self::ScanInterfaces => matches!(target, ResourceTarget::Host(_)),
            Self::SetAttached {
                expected,
                expected_attached,
                attached,
            } => pif && expected_attached != attached && (*attached || !expected.management),
            Self::CreateVlan { expected, tag, .. } => {
                network
                    && (1..=4094).contains(tag)
                    && target != ResourceTarget::Network(expected.network)
            }
            Self::DeleteVlan { tag, .. } => {
                matches!(target, ResourceTarget::Vlan(_)) && (1..=4094).contains(tag)
            }
            Self::CreateBond { members, .. } => {
                network
                    && (2..=4).contains(&members.len())
                    && members
                        .iter()
                        .map(|(id, _)| id)
                        .collect::<BTreeSet<_>>()
                        .len()
                        == members.len()
                    && members.iter().all(|(_, p)| {
                        p.host == members[0].1.host
                            && !p.management
                            && target != ResourceTarget::Network(p.network)
                    })
            }
            Self::DeleteBond {
                members, primary, ..
            } => bond && (2..=4).contains(&members.len()) && members.contains(primary),
            Self::SetBondMode {
                expected_mode,
                mode,
            } => {
                bond && ["balance-slb", "active-backup", "lacp"].contains(&expected_mode.as_str())
                    && expected_mode != mode.as_str()
            }
            Self::SetBondHash { expected, hash } => {
                bond && expected.as_deref() != Some(hash.as_str())
                    && expected.as_ref().is_none_or(|v| v.len() <= 64)
            }
            Self::SetAutomaticAttachment { expected, enabled } => {
                network
                    && expected.as_ref().is_none_or(|v| v.len() <= 64)
                    && (expected.as_deref() != Some("false")) != *enabled
            }
            Self::SetMtu {
                expected,
                mtu,
                pifs,
            } => {
                network
                    && expected != mtu
                    && (576..=65_535).contains(mtu)
                    && pifs.len() <= 1024
                    && pifs.iter().collect::<BTreeSet<_>>().len() == pifs.len()
            }
            Self::SetIgmpSnooping { expected, enabled } => {
                matches!(target, ResourceTarget::Pool(_)) && expected != enabled
            }
            Self::ConfigureIpv4 {
                expected,
                before,
                configuration,
            } => pif && !expected.management && before != configuration && configuration.valid(),
        };
        if valid {
            Ok(())
        } else {
            Err(Error::InvalidResourceCommand)
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct NetworkCreationBaseline {
    pub vlans: Vec<crate::VlanId>,
    pub bonds: Vec<BondId>,
    pub pifs: Vec<PifId>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ipv4_validation_rejects_noncontiguous_masks_and_mixed_modes() {
        let mut config = Ipv4Configuration {
            mode: Ipv4Mode::Static,
            address: "192.0.2.20".into(),
            netmask: "255.255.255.0".into(),
            gateway: "192.0.2.1".into(),
            dns: "192.0.2.53,2001:db8::53".into(),
        };
        assert!(config.valid());
        config.netmask = "255.0.255.0".into();
        assert!(!config.valid());
        config.netmask = "255.255.255.0".into();
        config.mode = Ipv4Mode::Dhcp;
        assert!(!config.valid());
        config.address.clear();
        config.netmask.clear();
        config.gateway.clear();
        assert!(config.valid());
        config.dns = "credential@example.invalid".into();
        assert!(!config.valid());
    }
    #[test]
    fn management_unplug_and_address_reconfiguration_are_refused_by_core() {
        let target = ResourceTarget::Pif(PifId::from(uuid::Uuid::new_v4()));
        let identity = PifIdentity {
            host: HostId::from(uuid::Uuid::new_v4()),
            network: NetworkId::from(uuid::Uuid::new_v4()),
            device: "eth0".into(),
            mac: "02:00:00:00:00:01".into(),
            management: true,
        };
        assert!(
            NetworkOperation::SetAttached {
                expected: identity.clone(),
                expected_attached: true,
                attached: false
            }
            .validate(target)
            .is_err()
        );
        let before = Ipv4Configuration {
            mode: Ipv4Mode::None,
            address: String::new(),
            netmask: String::new(),
            gateway: String::new(),
            dns: String::new(),
        };
        assert!(
            NetworkOperation::ConfigureIpv4 {
                expected: identity,
                before: before.clone(),
                configuration: Ipv4Configuration {
                    mode: Ipv4Mode::Dhcp,
                    ..before
                }
            }
            .validate(target)
            .is_err()
        );
    }
}
