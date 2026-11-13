use crate::{HostId, NetworkSriovId, PciId, PifId, SdnControllerId, TunnelId};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, net::Ipv4Addr};
use xenadmin_xapi::enumerations::{
    ip_configuration_mode, ipv6_configuration_mode, sriov_configuration_mode, tunnel_protocol,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TunnelProtocol {
    Gre,
    Vxlan,
}
impl TunnelProtocol {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Gre => tunnel_protocol::GRE,
            Self::Vxlan => tunnel_protocol::VXLAN,
        }
    }
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            tunnel_protocol::GRE => Some(Self::Gre),
            tunnel_protocol::VXLAN => Some(Self::Vxlan),
            _ => None,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SdnControllerConfiguration {
    Ssl { address: Ipv4Addr, port: u16 },
    PassiveSsl,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SdnController {
    pub id: SdnControllerId,
    pub configuration: Option<SdnControllerConfiguration>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkTunnel {
    pub id: TunnelId,
    pub transport: Option<PifId>,
    pub access: Option<PifId>,
    pub protocol: Option<TunnelProtocol>,
    pub controller_active: Option<bool>,
    pub controller_error: Option<bool>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SriovConfigurationMode {
    Sysfs,
    Modprobe,
    Manual,
    Unknown,
}
impl SriovConfigurationMode {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            sriov_configuration_mode::SYSFS => Some(Self::Sysfs),
            sriov_configuration_mode::MODPROBE => Some(Self::Modprobe),
            sriov_configuration_mode::MANUAL => Some(Self::Manual),
            sriov_configuration_mode::UNKNOWN => Some(Self::Unknown),
            _ => None,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkSriov {
    pub id: NetworkSriovId,
    pub physical: Option<PifId>,
    pub logical: Option<PifId>,
    pub requires_reboot: Option<bool>,
    pub mode: Option<SriovConfigurationMode>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkPciDevice {
    pub id: PciId,
    pub host: Option<HostId>,
    pub address: Option<String>,
    pub vendor_id: Option<String>,
    pub device_id: Option<String>,
    pub driver: Option<String>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct NetworkFabricInventory {
    pub controllers: BTreeMap<SdnControllerId, SdnController>,
    pub tunnels: BTreeMap<TunnelId, NetworkTunnel>,
    pub sriovs: BTreeMap<NetworkSriovId, NetworkSriov>,
    pub pci_devices: BTreeMap<PciId, NetworkPciDevice>,
    pub pif_pci: BTreeMap<PifId, PciId>,
    pub pool_sriov_restricted: Option<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NetworkFabricKind {
    Tunnel(TunnelProtocol),
    Sriov,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum NetworkFabricObject {
    Tunnel(TunnelId),
    Sriov(NetworkSriovId),
}
impl NetworkFabricObject {
    pub fn target(self) -> crate::ResourceTarget {
        match self {
            Self::Tunnel(id) => crate::ResourceTarget::Tunnel(id),
            Self::Sriov(id) => crate::ResourceTarget::Sriov(id),
        }
    }
    pub fn uuid(self) -> uuid::Uuid {
        self.target().uuid()
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkFabricMember {
    pub object: NetworkFabricObject,
    pub interface: crate::PhysicalInterface,
    pub input: PifId,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkFabricReview {
    pub pool: crate::PoolId,
    pub network: crate::NetworkId,
    pub host: HostId,
    pub kind: NetworkFabricKind,
    pub input: crate::PhysicalInterface,
    pub members: BTreeMap<PifId, NetworkFabricMember>,
    pub hosts: std::collections::BTreeSet<HostId>,
    pub coordinator: HostId,
    pub controller: Option<SdnController>,
    pub hardware: Option<NetworkPciDevice>,
    pub driver_interfaces: BTreeMap<PifId, crate::PhysicalInterface>,
    pub driver_hardware: BTreeMap<PifId, NetworkPciDevice>,
    pub mtu: u64,
    pub locking_mode: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NetworkFabricChange {
    Create,
    Delete(NetworkFabricObject),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkFabricOperation {
    pub review: NetworkFabricReview,
    pub change: NetworkFabricChange,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkFabricEvidence {
    pub object: NetworkFabricObject,
    pub interface: PifId,
    pub host: HostId,
    pub network: crate::NetworkId,
}
impl NetworkFabricOperation {
    pub fn capture(
        i: &crate::Inventory,
        network: crate::NetworkId,
        input: PifId,
        kind: NetworkFabricKind,
        change: NetworkFabricChange,
    ) -> Result<Self, crate::Error> {
        let review = NetworkFabricReview::capture(
            i,
            network,
            input,
            kind,
            matches!(change, NetworkFabricChange::Create),
        )?;
        let operation = Self { review, change };
        operation.validate(crate::ResourceTarget::Network(network))?;
        operation.availability(i)?;
        Ok(operation)
    }
    pub fn availability(&self, i: &crate::Inventory) -> Result<(), crate::Error> {
        if self.review
            != NetworkFabricReview::capture(
                i,
                self.review.network,
                self.review.input.id,
                self.review.kind,
                self.creates(),
            )?
        {
            return Err(crate::Error::ResourceConflict);
        }
        match self.change {
            NetworkFabricChange::Create => {
                if self
                    .review
                    .members
                    .values()
                    .any(|m| m.interface.host == Some(self.review.host))
                {
                    return Err(crate::Error::Protocol(
                        "This host already has an interface on the selected network",
                    ));
                }
                if self.review.kind == NetworkFabricKind::Sriov
                    && self.review.members.is_empty()
                    && self.review.host != self.review.coordinator
                {
                    return Err(crate::Error::Protocol(
                        "Create the SR-IOV interface on the pool coordinator first",
                    ));
                }
            }
            NetworkFabricChange::Delete(object) => {
                if !self.review.members.values().any(|m| {
                    m.object == object
                        && m.input == self.review.input.id
                        && m.interface.host == Some(self.review.host)
                }) {
                    return Err(crate::Error::ResourceConflict);
                }
            }
        }
        Ok(())
    }
    pub(crate) fn creates(&self) -> bool {
        matches!(self.change, NetworkFabricChange::Create)
    }
    pub(crate) fn method(&self) -> &'static str {
        match (&self.review.kind, &self.change) {
            (NetworkFabricKind::Tunnel(_), NetworkFabricChange::Create) => "Async.Tunnel.create",
            (NetworkFabricKind::Tunnel(_), NetworkFabricChange::Delete(_)) => {
                "Async.Tunnel.destroy"
            }
            (NetworkFabricKind::Sriov, NetworkFabricChange::Create) => "Async.Network_sriov.create",
            (NetworkFabricKind::Sriov, NetworkFabricChange::Delete(_)) => {
                "Async.Network_sriov.destroy"
            }
        }
    }
    pub fn label(&self) -> &'static str {
        match (&self.review.kind, &self.change) {
            (NetworkFabricKind::Tunnel(_), NetworkFabricChange::Create) => {
                "Create host tunnel interface"
            }
            (NetworkFabricKind::Tunnel(_), NetworkFabricChange::Delete(_)) => {
                "Remove host tunnel interface"
            }
            (NetworkFabricKind::Sriov, NetworkFabricChange::Create) => "Enable host SR-IOV network",
            (NetworkFabricKind::Sriov, NetworkFabricChange::Delete(_)) => {
                "Remove host SR-IOV network"
            }
        }
    }
    pub(crate) fn validate(&self, target: crate::ResourceTarget) -> Result<(), crate::Error> {
        let r = &self.review;
        let valid = target == crate::ResourceTarget::Network(r.network)
            && r.input.host == Some(r.host)
            && r.input.network.is_some()
            && r.input.network != Some(r.network)
            && !r.hosts.is_empty()
            && r.hosts.len() <= 1024
            && r.hosts.contains(&r.host)
            && r.hosts.contains(&r.coordinator)
            && r.members.len() <= 1024
            && r.driver_interfaces.len() <= 1024
            && r.driver_interfaces.keys().eq(r.driver_hardware.keys())
            && (576..=65535).contains(&r.mtu)
            && r.locking_mode.len() <= 128
            && r.input.device.len() <= 256
            && r.input.mac.len() <= 64
            && r.members
                .iter()
                .all(|(id, m)| *id == m.interface.id && m.interface.network == Some(r.network))
            && !matches!(
                (&r.kind, &self.change),
                (
                    NetworkFabricKind::Tunnel(_),
                    NetworkFabricChange::Delete(NetworkFabricObject::Sriov(_))
                ) | (
                    NetworkFabricKind::Sriov,
                    NetworkFabricChange::Delete(NetworkFabricObject::Tunnel(_))
                )
            );
        if valid {
            Ok(())
        } else {
            Err(crate::Error::InvalidResourceCommand)
        }
    }
}
impl NetworkFabricReview {
    pub(crate) fn capture(
        i: &crate::Inventory,
        network: crate::NetworkId,
        input: PifId,
        kind: NetworkFabricKind,
        creating: bool,
    ) -> Result<Self, crate::Error> {
        use crate::{ClassAvailability as C, Error};
        let d = &i.details;
        let f = &d.network_fabric;
        let mut classes = vec![
            "network",
            "pif",
            "vif",
            "bond",
            "vlan",
            "tunnel",
            "network_sriov",
        ];
        if kind == NetworkFabricKind::Sriov {
            classes.extend(["pci", "host_metrics"]);
        } else if creating {
            classes.push("sdn_controller");
        }
        if !classes
            .iter()
            .all(|c| d.classes.get(*c) == Some(&C::Available))
        {
            return Err(Error::Protocol(
                "Required network fabric inventory is unavailable, unsupported or not permitted",
            ));
        }
        let p = d.pifs.get(&input).ok_or(Error::ResourceConflict)?.clone();
        let host = p.host.ok_or(Error::ResourceConflict)?;
        if p.network.is_none() {
            return Err(Error::ResourceConflict);
        }
        let n = d.networks.get(&network).ok_or(Error::ResourceConflict)?;
        if n.managed != Some(true)
            || n.purposes.as_ref().is_none_or(|v| !v.is_empty())
            || p.managed != Some(true)
        {
            return Err(Error::Protocol(
                "Use managed interfaces and a target network without host service purposes",
            ));
        }
        if d.vifs.values().any(|v| v.network == Some(network)) {
            return Err(Error::Protocol(
                "The target network has VM interfaces; remove or move those consumers in a separate review",
            ));
        }
        let mut members = BTreeMap::new();
        for child in d.pifs.values().filter(|p| p.network == Some(network)) {
            let matches = match kind {
                NetworkFabricKind::Tunnel(protocol) => f
                    .tunnels
                    .values()
                    .filter(|t| t.access == Some(child.id) && t.protocol == Some(protocol))
                    .filter_map(|t| Some((NetworkFabricObject::Tunnel(t.id), t.transport?)))
                    .collect::<Vec<_>>(),
                NetworkFabricKind::Sriov => f
                    .sriovs
                    .values()
                    .filter(|s| s.logical == Some(child.id))
                    .filter_map(|s| Some((NetworkFabricObject::Sriov(s.id), s.physical?)))
                    .collect::<Vec<_>>(),
            };
            if matches.len() != 1 {
                return Err(Error::Protocol(
                    "The target network contains an incompatible or incompletely reported interface",
                ));
            }
            let (object, input) = matches[0];
            members.insert(
                child.id,
                NetworkFabricMember {
                    object,
                    interface: child.clone(),
                    input,
                },
            );
        }
        let mut controller = None;
        let mut hardware = None;
        let mut driver_interfaces = BTreeMap::new();
        let mut driver_hardware = BTreeMap::new();
        match kind {
            NetworkFabricKind::Tunnel(_) => {
                if creating {
                    if f.controllers.len() != 1
                        || f.controllers
                            .values()
                            .next()
                            .is_none_or(|c| c.configuration.is_none())
                    {
                        return Err(Error::Protocol(
                            "A known configured SDN controller is required; creating a bridge alone does not provide cross-host connectivity",
                        ));
                    }
                    controller = f.controllers.values().next().cloned();
                    if i.hosts.keys().any(|h| {
                        d.hosts
                            .get(h)
                            .and_then(|h| h.software_version.get("network_backend"))
                            .map(String::as_str)
                            != Some("openvswitch")
                    }) {
                        return Err(Error::Protocol(
                            "Every pool host must report the Open vSwitch backend",
                        ));
                    }
                    if p.currently_attached != Some(true)
                        || !matches!(
                            p.ip_configuration_mode.as_str(),
                            ip_configuration_mode::DHCP | ip_configuration_mode::STATIC
                        )
                        || p.ip.parse::<Ipv4Addr>().ok().is_none_or(|ip| {
                            ip.is_unspecified() || ip.is_multicast() || ip.is_loopback()
                        })
                    {
                        return Err(Error::Protocol(
                            "The tunnel transport must already be attached with usable IPv4 addressing",
                        ));
                    }
                }
                if d.bonds.values().any(|b| b.slaves.contains(&input))
                    || f.tunnels.values().any(|t| t.access == Some(input))
                    || f.sriovs.values().any(|s| {
                        s.logical == Some(input)
                            || d.vlans
                                .values()
                                .any(|v| v.untagged_pif == Some(input) && v.tagged_pif == s.logical)
                    })
                {
                    return Err(Error::Protocol(
                        "This interface topology cannot be used as tunnel transport",
                    ));
                }
            }
            NetworkFabricKind::Sriov => {
                if creating && f.pool_sriov_restricted != Some(false) {
                    return Err(Error::Protocol(
                        "The pool does not report SR-IOV networking as enabled",
                    ));
                }
                if p.physical != Some(true)
                    || creating
                        && p.capabilities
                            .as_ref()
                            .is_none_or(|c| !c.iter().any(|v| v == "sriov"))
                {
                    return Err(Error::Protocol(
                        "Select a physical interface reporting SR-IOV capability",
                    ));
                }
                let pci = f
                    .pif_pci
                    .get(&input)
                    .and_then(|id| f.pci_devices.get(id))
                    .ok_or(Error::Protocol("The interface PCI identity is unavailable"))?;
                if pci.host != Some(host)
                    || pci.driver.is_none()
                    || pci.vendor_id.is_none()
                    || pci.device_id.is_none()
                {
                    return Err(Error::Protocol(
                        "The PCI driver and hardware identity must be known",
                    ));
                }
                hardware = Some(pci.clone());
                for p in d
                    .pifs
                    .values()
                    .filter(|p| p.host == Some(host) && p.physical == Some(true))
                {
                    let peer = f
                        .pif_pci
                        .get(&p.id)
                        .and_then(|id| f.pci_devices.get(id))
                        .ok_or(Error::Protocol("The host PCI driver impact is incomplete"))?;
                    if peer.driver.is_none() {
                        return Err(Error::Protocol("The host PCI driver impact is incomplete"));
                    }
                    if peer.driver == pci.driver {
                        driver_interfaces.insert(p.id, p.clone());
                        driver_hardware.insert(p.id, peer.clone());
                    }
                }
                if d.hosts.get(&host).is_none_or(|h| {
                    h.enabled != Some(false)
                        || h.current_operations.as_ref().is_none_or(|v| !v.is_empty())
                }) || i
                    .vms
                    .values()
                    .any(|v| v.host == Some(host) && !v.is_control_domain)
                {
                    return Err(Error::Protocol(
                        "Disable and evacuate the host before changing SR-IOV; this action may reload its NIC driver or require a reboot",
                    ));
                }
                for p in driver_interfaces.values() {
                    if p.network
                        .and_then(|id| d.networks.get(&id))
                        .and_then(|n| n.purposes.as_ref())
                        .is_none_or(|purposes| !purposes.is_empty())
                        || p.management != Some(false)
                        || p.disallow_unplug != Some(false)
                        || !p.management_purpose.as_ref().is_none_or(String::is_empty)
                        || p.ip_configuration_mode != ip_configuration_mode::NONE
                        || p.ipv6_configuration_mode != ipv6_configuration_mode::NONE
                        || d.vifs.values().any(|v| v.network == p.network)
                        || d.bonds
                            .values()
                            .any(|b| b.master == Some(p.id) || b.slaves.contains(&p.id))
                        || d.vlans
                            .values()
                            .any(|v| v.tagged_pif == Some(p.id) || v.untagged_pif == Some(p.id))
                        || f.tunnels
                            .values()
                            .any(|t| t.transport == Some(p.id) || t.access == Some(p.id))
                    {
                        return Err(Error::Protocol(
                            "A same-driver interface has management, IP, service, VM or topology consumers; move those dependencies separately",
                        ));
                    }
                }
                for member in members.values() {
                    let peer = f
                        .pif_pci
                        .get(&member.input)
                        .and_then(|id| f.pci_devices.get(id))
                        .ok_or(Error::ResourceConflict)?;
                    if peer.vendor_id != pci.vendor_id || peer.device_id != pci.device_id {
                        return Err(Error::Protocol(
                            "Existing SR-IOV interfaces use a different PCI device type",
                        ));
                    }
                }
            }
        }
        Ok(Self {
            pool: i.pool_id,
            network,
            host,
            kind,
            input: p,
            members,
            hosts: i.hosts.keys().copied().collect(),
            coordinator: i.coordinator.ok_or(Error::ResourceConflict)?,
            controller,
            hardware,
            driver_interfaces,
            driver_hardware,
            mtu: n.mtu.ok_or(Error::ResourceConflict)?,
            locking_mode: n
                .default_locking_mode
                .clone()
                .ok_or(Error::ResourceConflict)?,
        })
    }
}
