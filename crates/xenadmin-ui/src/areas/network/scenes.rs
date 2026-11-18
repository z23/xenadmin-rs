//! The scenes of the area, under their Rune names.
//!
//! | Scene | Rune | Shows |
//! | --- | --- | --- |
//! | `network-interfaces` | `network_admin::scene` | the first network selected, its Networking tab, the dialog Add VLAN interface with tag 100 |
//! | `network-ipv6` | `network_admin::ipv6_scene` | the same inventory plus a management interface with static IPv6; the IPv6 form of the first interface, filled |
//! | `network-create` | `resource_admin::scene(s, false)` | the wizard New Network on its review, "Isolated lab network" |
//! | `pool-administration` | `resource_admin::scene(s, true)` | the pool's Networking tab. The Rune scene was the pool hub over the Networks page; the hub has no successor (its buttons are commands, its history is the Events tab), so the scene shows what was behind it |
//!
//! The other scenes of `network_admin.rs` are wrappers around the scenes
//! of its children: they belong to `network_services` and
//! `management_network`.

use std::sync::Arc;

use xenadmin_core::{NetworkId, PhysicalInterface, PifId};

use super::commands::{NEW, VLAN_ADD};
use super::workspace::{self, Form, InterfaceEditor};
use super::editor;
use crate::prelude::*;

pub fn scenes() -> Vec<SceneSpec> {
    vec![
        SceneSpec::new("network-interfaces", "network", scene),
        SceneSpec::new("network-ipv6", "network", ipv6_scene),
        SceneSpec::new("network-create", "network", create_scene),
        SceneSpec::new("pool-administration", "network", pool_scene),
    ]
}

/// The inventory of the Rune `network_admin::scene`: the demo, a network
/// "Physical uplinks" and two physical interfaces of the first server on
/// it. Public for the scenes of the areas whose Rune scenes started from
/// this one.
pub fn prepare(m: &mut Model) {
    m.load_demo(3, 48);
    let Some(inventory) = m.inventory.as_mut().map(Arc::make_mut) else {
        return;
    };
    let underlay = uuid::Uuid::from_u128(0x51000000000000000000000000000001);
    inventory.networks.push(xenadmin_core::Network {
        uuid: underlay,
        name: "Physical uplinks".into(),
        bridge: "xenbr1".into(),
    });
    let Some(host) = inventory.hosts.keys().next().copied() else {
        return;
    };
    for index in 0..2 {
        let id = PifId::from(uuid::Uuid::from_u128(
            0x52000000000000000000000000000001 + index,
        ));
        inventory.details.pifs.insert(
            id,
            PhysicalInterface {
                metrics: None,
                capabilities: None,
                managed: Some(true),
                disallow_unplug: Some(false),
                management_purpose: None,
                primary_address_type: Some("IPv4".into()),
                id,
                host: Some(host),
                network: Some(NetworkId::from(underlay)),
                device: format!("eth{index}"),
                mac: format!("02:00:00:00:00:{:02x}", index + 1),
                mtu: Some(1500),
                vlan: Some(-1),
                management: Some(false),
                physical: Some(true),
                currently_attached: Some(true),
                ip_configuration_mode: "None".into(),
                ip: String::new(),
                netmask: String::new(),
                gateway: String::new(),
                dns: String::new(),
                ipv6: vec![],
                ipv6_configuration_mode: "None".into(),
                ipv6_gateway: String::new(),
            },
        );
    }
}

/// Selects a network (the first one when `None`) and shows its Networking
/// tab: the Rune `Page::Networks` with the network chosen in the hub.
fn show_network(m: &mut Model, network: Option<NetworkId>) {
    let Some(inventory) = m.inventory.clone() else {
        return;
    };
    let uuid = network
        .map(|id| id.uuid())
        .or_else(|| inventory.networks.first().map(|n| n.uuid));
    let object = match uuid {
        Some(uuid) => ObjectRef::network(inventory.pool_id, uuid),
        None => ObjectRef::pool(inventory.pool_id),
    };
    m.select(Target::one(object));
    m.set_active_tab(TabId::Networking);
}

fn scene(m: &mut Model) {
    prepare(m);
    show_network(m, None);
    let target = m.target();
    m.invoke(VLAN_ADD, &target);
}

/// The Rune `interfaces::scene` and the setup of its tests: a management
/// interface with static IPv6 beside the first interface, whose IPv6
/// editor is then opened and filled.
pub(super) fn ipv6_fixture(m: &mut Model) -> Option<PifId> {
    let inventory = m.inventory.as_mut().map(Arc::make_mut)?;
    let id = *inventory.details.pifs.keys().next()?;
    let mut management = inventory.details.pifs[&id].clone();
    management.id = PifId::from(uuid::Uuid::from_u128(991));
    management.management = Some(true);
    management.ipv6_configuration_mode = "Static".into();
    management.ipv6 = vec!["2001:db8::10/64".into()];
    inventory.details.pifs.insert(management.id, management);
    Some(id)
}

/// Opens the IPv6 or the purpose editor of an interface as the Rune
/// `interfaces::open` did: on the network of the interface.
pub(super) fn open_interface(m: &mut Model, id: PifId, purpose: bool) -> bool {
    let Some(inventory) = m.inventory.clone() else {
        return false;
    };
    let Some(pif) = inventory.details.pifs.get(&id) else {
        return false;
    };
    let Some(form) = workspace::interface_form(&inventory, id, purpose) else {
        return false;
    };
    workspace::open(m, inventory.pool_id, pif.network, None, form);
    true
}

fn ipv6_scene(m: &mut Model) {
    prepare(m);
    let Some(id) = ipv6_fixture(m) else {
        return;
    };
    let network = m
        .inventory
        .as_ref()
        .and_then(|i| i.details.pifs.get(&id).and_then(|p| p.network));
    show_network(m, network);
    m.areas.network.pif = Some(id.to_string());
    if !open_interface(m, id, false) {
        return;
    }
    if let Some(w) = &mut m.areas.network.workspace
        && let Form::Interface(editor) = &mut w.form
        && let InterfaceEditor::Ipv6 {
            mode,
            address,
            gateway,
            dns,
            ..
        } = &mut **editor
    {
        *mode = xenadmin_core::Ipv6Mode::Static;
        *address = "2001:db8::20/64".into();
        *gateway = "2001:db8::1".into();
        *dns = "2001:db8::53".into();
    }
}

fn show_pool(m: &mut Model) {
    m.load_demo(3, 48);
    let Some(pool) = m.inventory.as_ref().map(|i| i.pool_id) else {
        return;
    };
    m.select(Target::one(ObjectRef::pool(pool)));
    m.set_active_tab(TabId::Networking);
}

fn create_scene(m: &mut Model) {
    show_pool(m);
    let target = m.target();
    m.invoke(NEW, &target);
    if let Some(editor) = &mut m.areas.network.editor {
        editor.name = "Isolated lab network".into();
        editor.description = "Synthetic preview".into();
    }
    editor::review(m);
}

fn pool_scene(m: &mut Model) {
    show_pool(m);
}
