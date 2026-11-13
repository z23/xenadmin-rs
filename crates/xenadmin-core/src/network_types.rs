use crate::{BondId, Error, HostId, NetworkId, PifId, ResourceTarget};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
};
use xenadmin_xapi::enumerations::{bond_mode, ip_configuration_mode, ipv6_configuration_mode};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BondMode {
    BalanceSlb,
    ActiveBackup,
    Lacp,
}
impl BondMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BalanceSlb => bond_mode::BALANCE_SLB,
            Self::ActiveBackup => bond_mode::ACTIVE_BACKUP,
            Self::Lacp => bond_mode::LACP,
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
            Self::None => ip_configuration_mode::NONE,
            Self::Dhcp => ip_configuration_mode::DHCP,
            Self::Static => ip_configuration_mode::STATIC,
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
            Self::None => ipv6_configuration_mode::NONE,
            Self::Dhcp => ipv6_configuration_mode::DHCP,
            Self::Autoconf => ipv6_configuration_mode::AUTOCONF,
            Self::Static => ipv6_configuration_mode::STATIC,
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
    Fabric(Box<crate::NetworkFabricOperation>),
    SetServicePurpose {
        review: crate::NetworkServiceReview,
        purpose: crate::NetworkServicePurpose,
        enabled: bool,
    },
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
    /// Forget a physical interface record whose device the host no longer presents under
    /// that name (a hot-plugged NIC keeps its temporary `side-N-ethX` name in the record
    /// while the kernel renames the device at boot); a rescan then re-introduces it.
    ForgetInterface {
        expected: PifIdentity,
    },
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
    pub fn permission_requirements(&self) -> crate::PermissionRequirements {
        crate::PermissionRequirements::writes([self.method()])
    }
    pub(crate) fn method(&self) -> &'static str {
        match self {
            Self::Fabric(op) => op.method(),
            Self::SetServicePurpose { enabled: true, .. } => "Async.network.add_purpose",
            Self::SetServicePurpose { enabled: false, .. } => "Async.network.remove_purpose",
            NetworkOperation::ConfigureIpv6 { .. } => "Async.PIF.reconfigure_ipv6",
            NetworkOperation::SetInterfacePurpose { .. } => "Async.PIF.set_other_config",
            NetworkOperation::SetUnplugProtection { .. } => "Async.PIF.set_disallow_unplug",
            NetworkOperation::ScanInterfaces => "Async.PIF.scan",
            NetworkOperation::ForgetInterface { .. } => "Async.PIF.forget",
            NetworkOperation::SetAttached { attached: true, .. } => "Async.PIF.plug",
            NetworkOperation::SetAttached {
                attached: false, ..
            } => "Async.PIF.unplug",
            NetworkOperation::CreateVlan { .. } => "Async.VLAN.create",
            NetworkOperation::DeleteVlan { .. } => "Async.VLAN.destroy",
            NetworkOperation::CreateBond { .. } => "Async.Bond.create",
            NetworkOperation::DeleteBond { .. } => "Async.Bond.destroy",
            NetworkOperation::SetBondMode { .. } => "Async.Bond.set_mode",
            NetworkOperation::SetBondHash { .. } => "Async.Bond.set_property",
            NetworkOperation::SetAutomaticAttachment { .. } => "Async.network.set_other_config",
            NetworkOperation::SetMtu { .. } => "Async.network.set_MTU",
            NetworkOperation::SetIgmpSnooping { .. } => "Async.pool.set_igmp_snooping_enabled",
            NetworkOperation::ConfigureIpv4 { .. } => "Async.PIF.reconfigure_ip",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Fabric(op) => op.label(),
            Self::SetServicePurpose { enabled: true, .. } => "Enable network service purpose",
            Self::SetServicePurpose { enabled: false, .. } => "Remove network service purpose",
            Self::ConfigureIpv6 { .. } => "Configure host IPv6 interface",
            Self::SetInterfacePurpose { .. } => "Set interface purpose",
            Self::SetUnplugProtection { .. } => "Set interface unplug protection",
            Self::ScanInterfaces => "Scan physical interfaces",
            Self::ForgetInterface { .. } => "Forget physical interface record",
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
            || matches!(self,Self::Fabric(op) if op.creates())
    }
    pub(crate) fn validate(&self, target: ResourceTarget) -> Result<(), Error> {
        let pif = matches!(target, ResourceTarget::Pif(_));
        let network = matches!(target, ResourceTarget::Network(_));
        let bond = matches!(target, ResourceTarget::Bond(_));
        let valid = match self {
            Self::Fabric(op) => return op.validate(target),
            Self::SetServicePurpose {
                review,
                purpose,
                enabled,
            } => {
                matches!(target, ResourceTarget::Network(id) if review.valid(id))
                    && review.purposes.contains(purpose.as_str()) != *enabled
                    && (!enabled || !review.purposes.contains(purpose.opposite()))
            }
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
            Self::ForgetInterface { expected } => pif && !expected.management,
            Self::SetAttached {
                expected,
                expected_attached,
                attached,
            } => pif && expected_attached != attached && (*attached || !expected.management),
            Self::CreateVlan { expected, tag, .. } => {
                network
                    && (0..=4094).contains(tag)
                    && target != ResourceTarget::Network(expected.network)
            }
            Self::DeleteVlan { tag, .. } => {
                matches!(target, ResourceTarget::Vlan(_)) && (0..=4094).contains(tag)
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
                bond && [
                    bond_mode::BALANCE_SLB,
                    bond_mode::ACTIVE_BACKUP,
                    bond_mode::LACP,
                ]
                .contains(&expected_mode.as_str())
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
    fn forgetting_an_interface_record_requires_a_non_management_pif_target() {
        let pif = ResourceTarget::Pif(PifId::from(uuid::Uuid::new_v4()));
        let mut identity = PifIdentity {
            host: HostId::from(uuid::Uuid::new_v4()),
            network: NetworkId::from(uuid::Uuid::new_v4()),
            device: "side-7679-eth1".into(),
            mac: "02:00:00:00:00:02".into(),
            management: false,
        };
        let forget = NetworkOperation::ForgetInterface {
            expected: identity.clone(),
        };
        assert!(forget.validate(pif).is_ok());
        assert_eq!(forget.method(), "Async.PIF.forget");
        assert!(
            forget
                .validate(ResourceTarget::Host(HostId::from(uuid::Uuid::new_v4())))
                .is_err()
        );
        identity.management = true;
        assert!(
            NetworkOperation::ForgetInterface { expected: identity }
                .validate(pif)
                .is_err()
        );
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
