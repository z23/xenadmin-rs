//! The scenes of the area, under their Rune names. Each ends on its review
//! (the `network-change` dialog of the `network` area) over the Networking
//! or NICs tab of the server whose management interface changes.
//!
//! | Scene | Rune | Shows |
//! | --- | --- | --- |
//! | `management-network` | `management::scene` | static IPv4 management, new address 192.0.2.20: the review |
//! | `management-dhcp` | `management::dynamic_scene` | the same interface to DHCP, expected address 192.0.2.30: the review |
//! | `management-vlan` | `management::topology_scene` | management on VLAN 17 over a bond of two NICs: the review with the topology lines |
//! | `management-bond` | `management_bond::scene` | a second NIC and an empty destination network: the review of the new management bond |
//! | `management-vlan-bond` | `management_bond::vlan_scene` | two VLANs over the management parent: the review with "Recreate VLAN" lines |
//! | `management-bond-guests` | `management_bond::consumer_scene` | three guest NICs as consumers (move, retain, no eligible host): the review with the guest impact |
//! | `management-ipv6-bond` | `management_ipv6_bond::scene` | IPv6 primary members and a temporary static IPv6 interface: the review of stage 1 |
//!
//! The Rune scenes built on each other by reading the editor the previous
//! one left open. A form is closed by its review here, so the shared parts
//! are fixtures that return what the next one needs.

use std::sync::Arc;

use xenadmin_core::{
    BondMode, Inventory, ManagementBondOperation as Bond, ManagementDynamicMode,
    ManagementInterface, ManagementNetworkOperation as Operation, NetworkId, PifId, PoolId,
    PowerState,
};

use super::{bond, ipv6, management};
use crate::areas::network;
use crate::prelude::*;

const AREA: &str = "management_network";

pub fn scenes() -> Vec<SceneSpec> {
    vec![
        SceneSpec::new("management-network", AREA, scene),
        SceneSpec::new("management-dhcp", AREA, dynamic_scene),
        SceneSpec::new("management-vlan", AREA, topology_scene),
        SceneSpec::new("management-bond", AREA, bond_scene),
        SceneSpec::new("management-vlan-bond", AREA, vlan_bond_scene),
        SceneSpec::new("management-bond-guests", AREA, consumer_scene),
        SceneSpec::new("management-ipv6-bond", AREA, ipv6_scene),
    ]
}

fn amend(m: &mut Model) -> Option<&mut Inventory> {
    m.inventory.as_mut().map(Arc::make_mut)
}

/// Selects the server of an interface and shows one of its tabs: what is
/// behind the dialog.
fn show(m: &mut Model, pif: PifId, tab: TabId) {
    let Some(i) = m.inventory.clone() else {
        return;
    };
    let Some(host) = i.details.pifs.get(&pif).and_then(|p| p.host) else {
        return;
    };
    m.select(Target::one(ObjectRef::host(i.pool_id, host)));
    m.set_active_tab(tab);
}

/// The inventory of the Rune `management::scene`: the first interface is a
/// static IPv4 management interface, HA is off. Returns the pool and that
/// interface.
pub(super) fn management_fixture(m: &mut Model) -> Option<(PoolId, PifId)> {
    network::scenes::prepare(m);
    let i = amend(m)?;
    let source_id = *i.details.pifs.keys().next()?;
    let p = i.details.pifs.get_mut(&source_id)?;
    p.management = Some(true);
    p.physical = Some(true);
    p.vlan = Some(-1);
    p.managed = Some(true);
    p.mtu = Some(1500);
    p.currently_attached = Some(true);
    p.disallow_unplug = Some(false);
    p.management_purpose = None;
    p.primary_address_type = Some("IPv4".into());
    p.ip_configuration_mode = "Static".into();
    p.ip = "192.0.2.10".into();
    p.netmask = "255.255.255.0".into();
    p.gateway = "192.0.2.1".into();
    p.dns = "192.0.2.53".into();
    p.ipv6_configuration_mode = "None".into();
    p.ipv6.clear();
    p.ipv6_gateway.clear();
    if let Some(pool) = &mut i.details.pool {
        pool.ha_enabled = Some(false);
    }
    let pool = i.pool_id;
    show(m, source_id, TabId::Networking);
    Some((pool, source_id))
}

fn scene(m: &mut Model) {
    let Some((pool, source)) = management_fixture(m) else {
        return;
    };
    management::open(m, pool, source, false);
    if let Some(v) = management::editor(m) {
        v.address = "192.0.2.20".into();
    }
    management::prepare(m);
}

fn dynamic_scene(m: &mut Model) {
    let Some((pool, source)) = management_fixture(m) else {
        return;
    };
    open_dynamic(m, pool, source);
    management::prepare(m);
}

/// The form of the Rune `dynamic_scene`, before its review.
pub(super) fn open_dynamic(m: &mut Model, pool: PoolId, source: PifId) {
    management::open(m, pool, source, false);
    if let Some(v) = management::editor(m) {
        v.address = "192.0.2.20".into();
        v.automatic = Some(ManagementDynamicMode::Ipv4Dhcp);
        v.recovery = "192.0.2.30".into();
    }
}

/// The inventory of the Rune `topology_scene`: management on VLAN 17 over
/// a bond (`bond0`, 0xf001) of two NICs (0xf002, 0xf003).
pub(super) fn topology_fixture(m: &mut Model) -> Option<(PoolId, PifId)> {
    let (pool, source) = management_fixture(m)?;
    let i = amend(m)?;
    let original = i.details.pifs[&source].clone();
    for n in [1_u128, 2, 3] {
        let mut p = original.clone();
        p.id = uuid::Uuid::from_u128(0xf000 + n).into();
        p.network = Some(uuid::Uuid::from_u128(0xf100 + n).into());
        p.device = if n == 1 {
            "bond0".into()
        } else {
            format!("eth{}", n - 1)
        };
        p.mac = format!("02:00:00:00:00:{n:02x}");
        p.physical = Some(n != 1);
        p.management = Some(false);
        p.ip_configuration_mode = "None".into();
        p.ip.clear();
        p.netmask.clear();
        p.gateway.clear();
        p.dns.clear();
        i.networks.push(xenadmin_core::Network {
            uuid: uuid::Uuid::from_u128(0xf100 + n),
            name: format!("Uplink {n}"),
            bridge: format!("xenbr{n}"),
        });
        i.details.pifs.insert(p.id, p);
    }
    let parent: PifId = uuid::Uuid::from_u128(0xf001).into();
    let member: PifId = uuid::Uuid::from_u128(0xf002).into();
    let second: PifId = uuid::Uuid::from_u128(0xf003).into();
    let bond = uuid::Uuid::from_u128(0xf200).into();
    let vlan = uuid::Uuid::from_u128(0xf201).into();
    i.details.bonds.insert(
        bond,
        xenadmin_core::NetworkBond {
            id: bond,
            master: Some(parent),
            slaves: vec![member, second],
            primary_slave: Some(member),
            mode: "active-backup".into(),
            hashing_algorithm: None,
            hashing_algorithm_known: true,
            links_up: Some(2),
        },
    );
    i.details.vlans.insert(
        vlan,
        xenadmin_core::Vlan {
            id: vlan,
            tagged_pif: Some(parent),
            untagged_pif: Some(source),
            tag: Some(17),
        },
    );
    let p = i.details.pifs.get_mut(&source)?;
    p.physical = Some(false);
    p.vlan = Some(17);
    for class in ["pif", "bond", "vlan"] {
        i.details
            .classes
            .insert(class.into(), xenadmin_core::ClassAvailability::Available);
    }
    Some((pool, source))
}

fn topology_scene(m: &mut Model) {
    let Some((pool, source)) = topology_fixture(m) else {
        return;
    };
    management::open(m, pool, source, false);
    if let Some(v) = management::editor(m) {
        v.address = "192.0.2.30".into();
    }
    management::prepare(m);
}

/// What the Rune `management_bond::scene` left for the scenes after it.
pub(super) struct BondFixture {
    pub pool: PoolId,
    /// The management interface.
    pub source: PifId,
    /// The second NIC (0xb001).
    pub second: PifId,
    /// The empty destination network (0xb002).
    pub target: NetworkId,
}

/// The inventory of the Rune `management_bond::scene`: a second NIC on its
/// own network and an empty destination network; no guest interfaces.
pub(super) fn bond_fixture(m: &mut Model) -> Option<BondFixture> {
    let (pool, source) = management_fixture(m)?;
    let i = amend(m)?;
    i.details.vifs.clear();
    let second: PifId = uuid::Uuid::from_u128(0xb001).into();
    let target = NetworkId::from(uuid::Uuid::from_u128(0xb002));
    let mut member = i.details.pifs[&source].clone();
    member.id = second;
    member.network = Some(uuid::Uuid::from_u128(0xb003).into());
    i.networks.push(xenadmin_core::Network {
        uuid: uuid::Uuid::from_u128(0xb003),
        name: "Second physical uplink".into(),
        bridge: "xenbr-secondary".into(),
    });
    member.device = "eth1".into();
    member.mac = "02:00:00:00:00:02".into();
    member.management = Some(false);
    member.ip_configuration_mode = "None".into();
    member.ip.clear();
    member.netmask.clear();
    member.gateway.clear();
    member.dns.clear();
    i.details.pifs.insert(second, member);
    i.networks.push(xenadmin_core::Network {
        uuid: target.uuid(),
        name: "Management bond network".into(),
        bridge: "xenbr-mgmt".into(),
    });
    for class in ["pif", "bond", "vlan", "vif"] {
        i.details
            .classes
            .insert(class.into(), xenadmin_core::ClassAvailability::Available);
    }
    show(m, source, TabId::Nics);
    Some(BondFixture {
        pool,
        source,
        second,
        target,
    })
}

/// Opens the bond form for `source` and fills it as the Rune scenes did:
/// the second NIC as a member, the empty network as the destination.
pub(super) fn fill_bond(m: &mut Model, f: &BondFixture, source: PifId) -> Option<()> {
    let member = ManagementInterface::capture(m.inventory.as_deref()?, f.second)?;
    bond::open(m, f.pool, source);
    let v = bond::editor(m)?;
    v.members.push(member);
    v.network = Some(f.target);
    Some(())
}

fn bond_scene(m: &mut Model) {
    let Some(f) = bond_fixture(m) else {
        return;
    };
    if fill_bond(m, &f, f.source).is_some() {
        bond::prepare(m);
    }
}

/// The inventory of the Rune `vlan_scene`: two VLANs (17 with management,
/// 18) over the former management interface, which is now only their
/// parent. Returns the fixture and the new management interface (0xb010).
pub(super) fn vlan_bond_fixture(m: &mut Model) -> Option<(BondFixture, PifId)> {
    let f = bond_fixture(m)?;
    let original = f.source;
    let i = amend(m)?;
    let base = i.details.pifs[&original].clone();
    let source: PifId = uuid::Uuid::from_u128(0xb010).into();
    for (n, tag) in [(0xb010_u128, 17_i64), (0xb011, 18)] {
        let id: PifId = uuid::Uuid::from_u128(n).into();
        let vlan = uuid::Uuid::from_u128(n + 100).into();
        let network = NetworkId::from(uuid::Uuid::from_u128(n + 200));
        let mut p = base.clone();
        p.id = id;
        p.network = Some(network);
        p.physical = Some(false);
        p.vlan = Some(tag);
        p.mac = "fe:ff:ff:ff:ff:ff".into();
        if id != source {
            p.management = Some(false);
            p.currently_attached = Some(false);
            p.ip_configuration_mode = "None".into();
            p.ip.clear();
            p.netmask.clear();
            p.gateway.clear();
            p.dns.clear();
        }
        i.details.pifs.insert(id, p);
        i.details.vlans.insert(
            vlan,
            xenadmin_core::Vlan {
                id: vlan,
                tagged_pif: Some(original),
                untagged_pif: Some(id),
                tag: Some(tag as u64),
            },
        );
        i.networks.push(xenadmin_core::Network {
            uuid: network.uuid(),
            name: format!("Reviewed VLAN {tag}"),
            bridge: format!("xapi-{tag}"),
        });
    }
    let p = i.details.pifs.get_mut(&original)?;
    p.management = Some(false);
    p.ip_configuration_mode = "None".into();
    p.ip.clear();
    p.netmask.clear();
    p.gateway.clear();
    p.dns.clear();
    Some((f, source))
}

fn vlan_bond_scene(m: &mut Model) {
    let Some((f, source)) = vlan_bond_fixture(m) else {
        return;
    };
    if fill_bond(m, &f, source).is_some() {
        bond::prepare(m);
    }
}

fn consumer_scene(m: &mut Model) {
    let Some(f) = bond_fixture(m) else {
        return;
    };
    if fill_bond(m, &f, f.source).is_none() {
        return;
    }
    let Some(Ok(mut op)) = bond::editor(m).map(|v| bond::operation(v)) else {
        return;
    };
    let host = op.source().identity.host;
    let source = op.source().identity.network;
    let target = f.target;
    let Some(i) = amend(m) else {
        return;
    };
    let guests = i.vms.keys().copied().take(3).collect::<Vec<_>>();
    let Some(other) = i.hosts.keys().find(|id| **id != host).copied() else {
        return;
    };
    let mut consumers = xenadmin_core::ManagementBondConsumers::default();
    for (index, vm) in guests.into_iter().enumerate() {
        let id = xenadmin_core::VifId::from(uuid::Uuid::from_u128(0xc000 + index as u128));
        let Some(guest) = i.vms.get_mut(&vm) else {
            continue;
        };
        guest.power_state = if index == 2 {
            PowerState::Halted
        } else {
            PowerState::Running
        };
        guest.host = if index == 2 {
            None
        } else if index == 1 {
            Some(other)
        } else {
            Some(host)
        };
        let locality = guest
            .host
            .map(xenadmin_core::ManagementVmLocality::Resident)
            .unwrap_or(xenadmin_core::ManagementVmLocality::PossibleHosts(vec![]));
        let destination = if index == 1 { source } else { target };
        let vm_name = guest.name.clone();
        let power_state = guest.power_state.to_string();
        i.details.vifs.insert(
            id,
            xenadmin_core::VirtualInterface {
                qos_known: true,
                qos_supported_algorithms: Some(vec!["ratelimit".into()]),
                id,
                vm: Some(vm),
                network: Some(source),
                device: "0".into(),
                mac: "02:00:00:00:00:01".into(),
                mtu: Some(1500),
                currently_attached: Some(index != 2),
                qos_algorithm: "ratelimit".into(),
                qos_parameters: std::collections::BTreeMap::from([("kbps".into(), "4096".into())]),
                allowed_operations: Some(vec![]),
            },
        );
        consumers.interfaces.insert(
            id,
            xenadmin_core::ManagementBondConsumer {
                interface: id,
                vm,
                vm_name,
                power_state,
                template: false,
                snapshot: false,
                locality,
                network: source,
                destination,
                change: if index == 1 {
                    xenadmin_core::ManagementVifChange::Retain
                } else {
                    xenadmin_core::ManagementVifChange::Move
                },
                device: "0".into(),
                mac: "02:00:00:00:00:01".into(),
                mtu: 1500,
                attached: index != 2,
                settings_fingerprint: [1; 32],
            },
        );
    }
    if let Operation::Bond {
        operation: Bond::Create {
            members, network, ..
        },
        ..
    } = &op
    {
        for id in members
            .iter()
            .map(|m| m.identity.network)
            .chain(std::iter::once(*network))
        {
            consumers.networks.insert(
                id,
                xenadmin_core::ManagementBondNetwork {
                    name: network::name(i, id),
                    mtu: 1500,
                    default_locking_mode: Some("unlocked".into()),
                    settings_fingerprint: [2; 32],
                },
            );
        }
    }
    if let Operation::Bond {
        consumers: impact, ..
    } = &mut op
    {
        *impact = Some(Box::new(consumers));
    }
    bond::finish_review(m, f.pool, op);
}

/// The form of the Rune `management_ipv6_bond::scene`, filled, before its
/// review: both NICs IPv6 primary, a temporary static IPv6 interface
/// (0x6b001) on its own network.
pub(super) fn open_ipv6(m: &mut Model) -> Option<PoolId> {
    let f = bond_fixture(m)?;
    let members = [f.source, f.second];
    let i = amend(m)?;
    for id in &members {
        let p = i.details.pifs.get_mut(id)?;
        p.primary_address_type = Some("IPv6".into());
        p.ip_configuration_mode = "None".into();
        p.ip.clear();
        p.netmask.clear();
        p.gateway.clear();
        p.dns.clear();
        p.ipv6_configuration_mode = if *id == f.source { "Static" } else { "None" }.into();
        p.ipv6 = if *id == f.source {
            vec!["2001:db8:1::10/64".into()]
        } else {
            vec![]
        };
        p.ipv6_gateway.clear();
    }
    let temporary = PifId::from(uuid::Uuid::from_u128(0x6b001));
    let net = NetworkId::from(uuid::Uuid::from_u128(0x6b002));
    let mut p = i.details.pifs[&f.source].clone();
    p.id = temporary;
    p.network = Some(net);
    p.management = Some(false);
    p.device = "eth-recovery".into();
    p.mac = "02:00:00:00:06:01".into();
    p.ipv6 = vec!["2001:db8:2::10/64".into()];
    i.details.pifs.insert(temporary, p);
    i.networks.push(xenadmin_core::Network {
        uuid: net.uuid(),
        name: "Independent IPv6 recovery".into(),
        bridge: "xenbr-recovery".into(),
    });
    let captured = members
        .iter()
        .map(|id| ManagementInterface::capture(i, *id))
        .collect::<Option<Vec<_>>>()?;
    let recovery = ManagementInterface::capture(i, temporary)?;
    ipv6::open(m, f.pool, f.source);
    let v = ipv6::editor(m)?;
    v.operation = Bond::Create {
        members: captured,
        network: f.target,
        mode: BondMode::ActiveBackup,
        hash: None,
    };
    v.temporary = Some(recovery);
    Some(f.pool)
}

fn ipv6_scene(m: &mut Model) {
    if open_ipv6(m).is_some() {
        ipv6::prepare(m);
    }
}
