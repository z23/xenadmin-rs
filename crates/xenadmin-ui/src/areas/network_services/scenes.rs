//! The scenes of the area, under their Rune names. Each starts from the
//! inventory of the Rune `network_admin::scene` (`network::scenes::prepare`),
//! edits it as the Rune scene of the child module did, selects the first
//! network with its Networking tab and opens the workflow.
//!
//! | Scene | Rune | Shows |
//! | --- | --- | --- |
//! | `network-services` | `service::scene` | the shared dialog `network-change` on the review "enable NBD with TLS" |
//! | `network-topology` | `topology::scene` | the dialog `network-topology` on the review of pool-wide VLAN creation, tag 100, one uplink per host |
//! | `network-fabric` | `fabric::scene` | the wizard `network-fabric` on its review: a GRE tunnel on the one host |
//! | `network-sriov` | `fabric::sriov_scene` | the same wizard on the review of SR-IOV on a disabled host |
//! | `network-fabric-pool` | `fabric_pool::scene` | the wizard `network-fabric-pool` on its review: VXLAN on both hosts |
//! | `network-fabric-pool-select` | `fabric_pool::selection_scene` | the same wizard on its Settings step, the inputs chosen |

use std::sync::Arc;

use uuid::Uuid;
use xenadmin_core::{
    ClassAvailability, HostId, Inventory, NetworkId, NetworkPciDevice, NetworkServicePurpose,
    NetworkSriov, NetworkSriovId, PciId, PifId, PoolId, SdnController, SdnControllerConfiguration,
    SdnControllerId, SriovConfigurationMode, VlanUplink,
};

use super::edit::{self, Draft, SETTINGS};
use super::fabric::{self, Kind, TunnelProtocol};
use super::{service, topology};
use crate::areas::network;
use crate::prelude::*;

const AREA: &str = "network_services";

pub fn scenes() -> Vec<SceneSpec> {
    vec![
        SceneSpec::new("network-services", AREA, service_scene),
        SceneSpec::new("network-topology", AREA, topology_scene),
        SceneSpec::new("network-fabric", AREA, fabric_scene),
        SceneSpec::new("network-sriov", AREA, sriov_scene),
        SceneSpec::new("network-fabric-pool", AREA, fabric_pool_scene),
        SceneSpec::new(
            "network-fabric-pool-select",
            AREA,
            fabric_pool_selection_scene,
        ),
        SceneSpec::new("network-sriov-capacity", AREA, sriov_capacity_scene),
        SceneSpec::new("network-sriov-vlan", AREA, sriov_vlan_scene),
    ]
}

/// The network "Physical uplinks" of `network::scenes::prepare`.
pub(super) fn underlay() -> NetworkId {
    NetworkId::from(Uuid::from_u128(0x51000000000000000000000000000001))
}

fn amend(m: &mut Model) -> Option<&mut Inventory> {
    m.inventory.as_mut().map(Arc::make_mut)
}

/// Selects the network and shows its Networking tab (the Rune
/// `Page::Networks` with the network chosen in the hub).
fn show(m: &mut Model, network: NetworkId) -> Option<PoolId> {
    let pool = m.inventory.as_ref()?.pool_id;
    m.select(Target::one(ObjectRef::network(pool, network.uuid())));
    m.set_active_tab(TabId::Networking);
    Some(pool)
}

// -- Services ----------------------------------------------------------------------

/// The inventory of the Rune `service::scene`: every network has details,
/// and the first interface is on the first network with an address.
pub(super) fn service_fixture(m: &mut Model) -> Option<NetworkId> {
    network::scenes::prepare(m);
    let i = amend(m)?;
    let network = NetworkId::from(i.networks.first()?.uuid);
    let prototype = i.details.networks.get(&network)?.clone();
    for n in &i.networks {
        let id = NetworkId::from(n.uuid);
        i.details.networks.entry(id).or_insert_with(|| {
            let mut details = prototype.clone();
            details.id = id;
            details
        });
    }
    let p = i.details.pifs.values_mut().next()?;
    p.network = Some(network);
    p.ip = "192.0.2.20".into();
    Some(network)
}

fn service_scene(m: &mut Model) {
    let Some(network) = service_fixture(m) else {
        return;
    };
    let Some(pool) = show(m, network) else {
        return;
    };
    service::open(m, pool, network, NetworkServicePurpose::NbdTls);
}

// -- Pool-wide VLANs ---------------------------------------------------------------

/// The inventory of the Rune `topology::scene`: one managed untagged
/// interface per host on "Physical uplinks", nothing else, and the classes
/// the availability of a topology change reads.
pub(super) fn topology_fixture(m: &mut Model) -> Option<NetworkId> {
    network::scenes::prepare(m);
    let i = amend(m)?;
    let network = NetworkId::from(i.networks.first()?.uuid);
    let prototype = i
        .details
        .pifs
        .values()
        .find(|p| p.network == Some(underlay()))?
        .clone();
    i.details.pifs.clear();
    i.details.vlans.clear();
    i.details.vifs.clear();
    let hosts = i.hosts.keys().copied().collect::<Vec<_>>();
    for (index, host) in hosts.into_iter().enumerate() {
        let mut p = prototype.clone();
        p.id = Uuid::from_u128(0x53000000000000000000000000000001 + index as u128).into();
        p.host = Some(host);
        p.device = "eth1".into();
        p.mac = format!("02:00:00:00:00:{:02x}", index + 1);
        i.details.pifs.insert(p.id, p);
    }
    for class in ["pif", "vif", "vlan"] {
        i.details
            .classes
            .insert(class.into(), ClassAvailability::Available);
    }
    Some(network)
}

/// Opens the creation of the scene: tag 100, the uplinks of "Physical
/// uplinks" chosen. Not reviewed yet.
pub(super) fn open_creation(m: &mut Model, network: NetworkId) -> Option<()> {
    let pool = show(m, network)?;
    let i = m.inventory.clone()?;
    let selection = topology::Selection::Create(topology::uplinks(&i, underlay(), network).ok()?);
    edit::open(
        m,
        pool,
        network,
        Draft::Topology(topology::Editor {
            network,
            tag: "100".into(),
            selection,
        }),
    );
    Some(())
}

fn topology_scene(m: &mut Model) {
    let Some(network) = topology_fixture(m) else {
        return;
    };
    if open_creation(m, network).is_some() {
        edit::review(m);
    }
}

// -- Tunnels and SR-IOV ------------------------------------------------------------

/// The inventory of the Rune `fabric::scene`: one host on the openvswitch
/// backend, an SDN controller, and an interface with a static address as
/// the transport. Returns the first network and that interface.
pub(super) fn fabric_fixture(m: &mut Model) -> Option<(NetworkId, PifId)> {
    network::scenes::prepare(m);
    let i = amend(m)?;
    let host = *i.hosts.keys().next()?;
    i.hosts.retain(|id, _| *id == host);
    i.coordinator = Some(host);
    i.vms.clear();
    i.details.vifs.clear();
    i.details.bonds.clear();
    i.details.vlans.clear();
    for class in [
        "network",
        "pif",
        "vif",
        "bond",
        "vlan",
        "tunnel",
        "network_sriov",
        "sdn_controller",
        "pci",
        "host_metrics",
    ] {
        i.details
            .classes
            .insert(class.into(), ClassAvailability::Available);
    }
    i.details
        .hosts
        .entry(host)
        .or_default()
        .software_version
        .insert("network_backend".into(), "openvswitch".into());
    let n = NetworkId::from(i.networks.first()?.uuid);
    let nd = i.details.networks.get_mut(&n)?;
    nd.managed = Some(true);
    nd.purposes = Some(Default::default());
    nd.mtu = Some(1500);
    nd.default_locking_mode = Some("unlocked".into());
    let p = i
        .details
        .pifs
        .values_mut()
        .find(|p| p.host == Some(host) && p.network != Some(n))?;
    p.ip_configuration_mode = "Static".into();
    p.ip = "192.0.2.20".into();
    p.netmask = "255.255.255.0".into();
    let input = p.id;
    let id = SdnControllerId::from(Uuid::from_u128(0x55000000000000000000000000000001));
    i.details.network_fabric.controllers.insert(
        id,
        SdnController {
            id,
            configuration: Some(SdnControllerConfiguration::Ssl {
                address: "192.0.2.40".parse().ok()?,
                port: 6632,
            }),
        },
    );
    Some((n, input))
}

/// The Rune `fabric::sriov_scene` on top: the input is the only interface,
/// capable of SR-IOV, with its PCI device; the host is disabled.
pub(super) fn sriov_fixture(m: &mut Model) -> Option<(NetworkId, PifId)> {
    let (network, _) = fabric_fixture(m)?;
    let i = amend(m)?;
    let host = *i.hosts.keys().next()?;
    let input = i
        .details
        .pifs
        .values()
        .find(|p| p.host == Some(host) && p.network != Some(network))?
        .id;
    i.details.pifs.retain(|id, _| *id == input);
    let p = i.details.pifs.get_mut(&input)?;
    p.ip_configuration_mode = "None".into();
    p.ip.clear();
    p.netmask.clear();
    p.capabilities = Some(vec!["sriov".into()]);
    let underlay = p.network?;
    if !i.details.networks.contains_key(&underlay) {
        let mut details = i.details.networks.get(&network)?.clone();
        details.id = underlay;
        i.details.networks.insert(underlay, details);
    }
    let hd = i.details.hosts.get_mut(&host)?;
    hd.enabled = Some(false);
    hd.current_operations = Some(vec![]);
    let pci = PciId::from(Uuid::from_u128(0x56000000000000000000000000000001));
    i.details.network_fabric.pool_sriov_restricted = Some(false);
    i.details.network_fabric.pif_pci.insert(input, pci);
    i.details.network_fabric.pci_devices.insert(
        pci,
        NetworkPciDevice {
            id: pci,
            host: Some(host),
            address: Some("0000:01:00.1".into()),
            vendor_id: Some("8086".into()),
            device_id: Some("100e".into()),
            driver: Some("fixture_nic".into()),
        },
    );
    Some((network, input))
}

/// Opens the wizard of one host on its Settings step with the kind and
/// the input chosen. Not reviewed yet.
pub(super) fn open_fabric(m: &mut Model, network: NetworkId, kind: Kind, input: PifId) {
    let Some(pool) = show(m, network) else {
        return;
    };
    edit::open(
        m,
        pool,
        network,
        Draft::Fabric(fabric::Editor {
            kind: Some(kind),
            input: Some(input),
        }),
    );
    if let Some(e) = edit::edit(m) {
        e.step = SETTINGS;
    }
}

fn fabric_scene(m: &mut Model) {
    let Some((network, input)) = fabric_fixture(m) else {
        return;
    };
    open_fabric(m, network, Kind::Tunnel(TunnelProtocol::Gre), input);
    edit::review(m);
}

fn sriov_scene(m: &mut Model) {
    let Some((network, input)) = sriov_fixture(m) else {
        return;
    };
    open_fabric(m, network, Kind::Sriov, input);
    edit::review(m);
}

/// The Rune `fabric_pool::scene` on top of the fabric one: a second host
/// with a copy of the transport interface. Returns the network and the
/// inputs of the two hosts.
pub(super) fn pool_fixture(m: &mut Model) -> Option<(NetworkId, [(HostId, PifId); 2])> {
    let (network, _) = fabric_fixture(m)?;
    let i = amend(m)?;
    let original = i.hosts.values().next()?.clone();
    let mut second = original.clone();
    second.id = HostId::from(Uuid::from_u128(0x22000000000000000000000000000022));
    second.name = "Synthetic second host".into();
    second.address = "192.0.2.21".into();
    let details = i.details.hosts.get(&original.id)?.clone();
    i.details.hosts.insert(second.id, details);
    let mut input = i
        .details
        .pifs
        .values()
        .find(|p| p.host == Some(original.id) && p.network != Some(network))?
        .clone();
    let first = input.id;
    input.id = PifId::from(Uuid::from_u128(0x77000000000000000000000000000022));
    input.host = Some(second.id);
    input.ip = "192.0.2.21".into();
    let next = input.id;
    i.details.pifs.insert(next, input);
    let hosts = [(original.id, first), (second.id, next)];
    i.hosts.insert(second.id, second);
    Some((network, hosts))
}

/// Opens the wizard of the pool on its Settings step: VXLAN, the inputs
/// chosen. Not reviewed yet.
pub(super) fn open_pool(m: &mut Model, network: NetworkId, inputs: [(HostId, PifId); 2]) {
    let Some(pool) = show(m, network) else {
        return;
    };
    edit::open(
        m,
        pool,
        network,
        Draft::FabricPool(fabric::PoolEditor {
            kind: Some(Kind::Tunnel(TunnelProtocol::Vxlan)),
            inputs: inputs.into_iter().collect(),
        }),
    );
    if let Some(e) = edit::edit(m) {
        e.step = SETTINGS;
    }
}

fn fabric_pool_scene(m: &mut Model) {
    let Some((network, inputs)) = pool_fixture(m) else {
        return;
    };
    open_pool(m, network, inputs);
    edit::review(m);
}

fn fabric_pool_selection_scene(m: &mut Model) {
    let Some((network, inputs)) = pool_fixture(m) else {
        return;
    };
    open_pool(m, network, inputs);
}

/// An SR-IOV network with an active logical interface on the Networking tab,
/// showing the read-only VF capacity inspection controls.
pub(super) fn sriov_capacity_fixture(m: &mut Model) -> Option<(NetworkId, NetworkSriovId)> {
    let (network, _) = sriov_fixture(m)?;
    let i = amend(m)?;
    let physical = i.details.pifs.values().next()?.clone();
    let id = NetworkSriovId::from(Uuid::from_u128(0x57000000000000000000000000000001));
    let mut logical = physical.clone();
    logical.id = Uuid::from_u128(0x57000000000000000000000000000002).into();
    logical.physical = Some(false);
    logical.network = Some(network);
    logical.mac = "fe:ff:ff:ff:ff:ff".into();
    logical.capabilities = Some(vec![]);
    i.details.network_fabric.sriovs.insert(
        id,
        NetworkSriov {
            id,
            physical: Some(physical.id),
            logical: Some(logical.id),
            requires_reboot: Some(false),
            mode: Some(SriovConfigurationMode::Sysfs),
        },
    );
    i.details.pifs.insert(logical.id, logical);
    show(m, network)?;
    Some((network, id))
}

fn sriov_capacity_scene(m: &mut Model) {
    let _ = sriov_capacity_fixture(m);
}

fn sriov_vlan_scene(m: &mut Model) {
    let Some(_) = sriov_capacity_fixture(m) else {
        return;
    };
    let Some(i) = amend(m) else {
        return;
    };
    let Some(parent) = i
        .details
        .network_fabric
        .sriovs
        .values()
        .next()
        .and_then(|s| s.logical)
    else {
        return;
    };
    let target = NetworkId::from(Uuid::from_u128(0x58000000000000000000000000000001));
    let mut row = i.networks[0].clone();
    row.uuid = target.uuid();
    row.name = "Synthetic SR-IOV VLAN".into();
    i.networks.push(row);
    let mut details = i.details.networks.values().next().unwrap().clone();
    details.id = target;
    details.purposes = Some(Default::default());
    i.details.networks.insert(target, details);
    let Ok(uplink) = VlanUplink::capture(i, parent, target) else {
        return;
    };
    let Some(pool) = show(m, target) else {
        return;
    };
    edit::open(
        m,
        pool,
        target,
        Draft::Topology(topology::Editor {
            network: target,
            tag: "100".into(),
            selection: topology::Selection::Create(vec![uplink]),
        }),
    );
    edit::review(m);
}
