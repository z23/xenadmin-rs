//! The Networking tab of a pool, a server and a network, and the NICs tab
//! of a server: what the Rune hub showed as cards and buttons, as grids.
//!
//! - A grid of **networks** has tree objects as rows: selecting one makes
//!   it the target of the commands of the local row (Add VLAN Interface,
//!   Remove Network) and of the kernel's context menu (Properties).
//! - A grid of **interfaces** has rows that are no tree objects: the
//!   selected row is this area's (`State::pif`), its commands are the row
//!   menu ([`INTERFACE_MENU`]) and, below the grid, the Rune card of the
//!   selected interface says what the hub said about it.
//! - Under the grids, what the fallback tabs of the `general` area showed
//!   (`info.rs`), and the Rune "Latest operation" line with the way to the
//!   operation history.
//!
//! Keys. Kept from the `general` fallbacks: `networking-networks`,
//! `networking-networks-table`, `networking-uplinks`, `networking-vifs`,
//! `networking-vifs-table`, `nics-interfaces`, `pif-<uuid>`, `pifs-empty`,
//! `class-notice-<class>`. New: `networking-interfaces`,
//! `networking-interfaces-table`, `network-uplinks-table`, `nics-grid`,
//! `nics-table`, `network-interface-card`, `network-latest-operation`,
//! `network-operation-history`.

use xenadmin_core::{HostId, Inventory, NetworkId, PhysicalInterface, PoolId};

use super::commands::{INTERFACE_MENU, NO_IPV6};
use super::info::{self, known};
use super::workspace;
use crate::prelude::*;

pub fn tab(m: &Model, tab: TabId, scope: &Target) -> Option<TabBody> {
    let object = scope.single()?;
    let pool = object.pool?;
    // Without the inventory the `general` fallback says so.
    let inventory = m.inventory_for(pool)?;
    let i = &*inventory;
    let blocks = match (tab, object.kind) {
        (TabId::Networking, ObjectKind::Pool) => pool_networking(m, i, pool),
        (TabId::Networking, ObjectKind::Host) => {
            let host = object.host_id().filter(|id| i.hosts.contains_key(id))?;
            host_networking(m, i, pool, host)
        }
        (TabId::Networking, ObjectKind::Network) => {
            let uuid = object
                .uuid()
                .filter(|uuid| i.networks.iter().any(|n| n.uuid == *uuid))?;
            network_networking(m, i, pool, NetworkId::from(uuid))
        }
        (TabId::Nics, ObjectKind::Host) => {
            let host = object.host_id().filter(|id| i.hosts.contains_key(id))?;
            host_nics(m, i, pool, host)
        }
        // A VM's Networking tab is `vm_devices`'.
        _ => return None,
    };
    Some(TabBody::Blocks(blocks))
}

fn sorted<'a>(
    i: &Inventory,
    interfaces: impl Iterator<Item = &'a PhysicalInterface>,
) -> Vec<&'a PhysicalInterface> {
    let mut interfaces: Vec<_> = interfaces.collect();
    interfaces.sort_by(|a, b| {
        natural_cmp(&info::host_name(i, a.host), &info::host_name(i, b.host))
            .then_with(|| natural_cmp(&a.device, &b.device))
            .then_with(|| a.id.cmp(&b.id))
    });
    interfaces
}

fn vlan(interface: &PhysicalInterface) -> String {
    interface
        .vlan
        .map(|n| {
            if n < 0 {
                "Untagged".into()
            } else {
                n.to_string()
            }
        })
        .unwrap_or_else(|| "Unknown".into())
}

/// The grid of interfaces: one selectable row per interface, the commands
/// on an interface as its row menu.
fn interfaces_table(
    m: &Model,
    i: &Inventory,
    key: &str,
    interfaces: &[&PhysicalInterface],
    empty: &str,
) -> Table {
    let rows: Vec<Row> = interfaces
        .iter()
        .map(|p| {
            row(
                p.id,
                [
                    info::host_name(i, p.host),
                    info::network_name(i, p.network),
                    p.device.clone(),
                    info::supplied(&p.mac).to_owned(),
                    p.mtu
                        .map(|n| n.to_string())
                        .unwrap_or_else(|| "Unknown".into()),
                    vlan(p),
                    known(p.management).to_owned(),
                    info::supplied(&p.ip_configuration_mode).to_owned(),
                    info::supplied(&p.ip).to_owned(),
                    known(p.currently_attached).to_owned(),
                ],
            )
        })
        .collect();
    let selected = m
        .areas
        .network
        .pif
        .as_deref()
        .filter(|id| rows.iter().any(|row| row.id == *id));
    table(
        key,
        [
            column("server", "Server"),
            column("network", "Network"),
            column("device", "Device"),
            column("mac", "MAC"),
            column("mtu", "MTU").right(),
            column("vlan", "VLAN"),
            column("management", "Management"),
            column("mode", "IPv4 mode"),
            column("address", "IPv4 address"),
            column("attached", "Attached"),
        ],
        rows,
    )
    .single(selected, |m, id| {
        m.areas.network.pif = Some(id.to_owned());
    })
    .on_clear(|m| m.areas.network.pif = None)
    .row_menu(INTERFACE_MENU)
    .empty(empty)
}

/// What the Rune hub printed on the card of an interface, for the selected
/// one: identity and addressing, the management note, IPv6, purpose and
/// unplug protection, and its bonds. The buttons of the card are the row
/// menu.
fn card(i: &Inventory, interfaces: &[&PhysicalInterface], selected: Option<&str>) -> Option<Group> {
    let selected = selected?;
    let p = interfaces.iter().find(|p| p.id.to_string() == selected)?;
    let unknown = |value: Option<String>| value.unwrap_or_else(|| "unknown".into());
    Some(
        group(
            workspace::pif_name(i, p.id),
            blocks![
                caption(p.id.to_string()),
                caption(format!(
                    "MAC {} · MTU {} · VLAN {}",
                    p.mac,
                    unknown(p.mtu.map(|v| v.to_string())),
                    unknown(p.vlan.map(|v| v.to_string())),
                )),
                caption(format!(
                    "{} {} / {} · gateway {} · DNS {}",
                    p.ip_configuration_mode, p.ip, p.netmask, p.gateway, p.dns
                )),
                (p.management == Some(true)).then(|| caption(
                    "Host management interface. XCP-ng requires the standard 1500-byte MTU. Review recovery addressing before changing management access."
                )),
                caption(format!(
                    "IPv6 {} · {} · gateway {}",
                    p.ipv6_configuration_mode,
                    p.ipv6.join(", "),
                    p.ipv6_gateway,
                )),
                caption(format!(
                    "Purpose: {} · unplug protection: {}",
                    p.management_purpose.as_deref().unwrap_or("Not set"),
                    p.disallow_unplug
                        .map(|v| if v { "enabled" } else { "disabled" })
                        .unwrap_or("unknown")
                )),
                (!workspace::host_ipv6(i, p)).then(|| caption(NO_IPV6)),
                i.details
                    .bonds
                    .values()
                    .filter(|b| b.master == Some(p.id))
                    .map(|bond| caption(format!(
                        "Bond {} · {} · hash {}",
                        bond.id,
                        bond.mode,
                        bond.hashing_algorithm.as_deref().unwrap_or("default")
                    )))
                    .collect::<Vec<_>>(),
            ],
        )
        .key("network-interface-card"),
    )
}

/// The Rune line "Latest operation" of the networking dialog, for the
/// newest resource operation of the pool, and its footer button
/// "Operation history" (the Events tab of the Notifications view now).
fn latest(m: &Model, pool: PoolId) -> Vec<Block> {
    blocks![
        m.resource_rows
            .iter()
            .rev()
            .find(|record| record.pool == pool)
            .map(|record| caption(format!(
                "Latest operation: {} · {:?} · {}",
                record.action.label(),
                record.state,
                record.detail
            ))
            .key("network-latest-operation")),
        actions([button(
            "network-operation-history",
            "Operation history",
            move |m| crate::areas::events::show_history(m, pool)
        )
        .ghost()]),
    ]
}

/// The grid of networks: tree objects.
fn networks_table(
    m: &Model,
    i: &Inventory,
    pool: PoolId,
    shown: impl Fn(NetworkId) -> bool,
    empty: &str,
) -> Table {
    let mut networks: Vec<_> = i
        .networks
        .iter()
        .filter(|network| shown(NetworkId::from(network.uuid)))
        .collect();
    networks.sort_by(|a, b| natural_cmp(&a.name, &b.name).then_with(|| a.uuid.cmp(&b.uuid)));
    let rows: Vec<Row> = networks
        .into_iter()
        .map(|network| {
            let id = NetworkId::from(network.uuid);
            let details = i.details.networks.get(&id);
            row(
                network.uuid,
                [
                    network.name.clone(),
                    network.bridge.clone(),
                    details
                        .and_then(|d| d.mtu)
                        .map(|n| n.to_string())
                        .unwrap_or_else(|| "Unknown".into()),
                    known(details.and_then(|d| d.managed)).to_owned(),
                    i.details
                        .pifs
                        .values()
                        .filter(|p| p.network == Some(id))
                        .count()
                        .to_string(),
                    i.details
                        .vifs
                        .values()
                        .filter(|v| v.network == Some(id))
                        .count()
                        .to_string(),
                ],
            )
            .object(ObjectRef::network(pool, network.uuid))
        })
        .collect();
    // The selection is the kernel's grid selection, so a hand-over from
    // another area (search) shows too.
    let selected = m
        .selection
        .grid
        .single_of(ObjectKind::Network)
        .map(|object| object.id.clone())
        .filter(|id| rows.iter().any(|row| row.id == *id));
    let object = move |id: &str| {
        uuid::Uuid::parse_str(id)
            .ok()
            .map(|uuid| ObjectRef::network(pool, uuid))
    };
    table(
        "networking-networks-table",
        [
            column("name", "Name"),
            column("bridge", "Bridge"),
            column("mtu", "MTU").right(),
            column("managed", "Managed"),
            column("uplinks", "Host uplinks").right(),
            column("vifs", "VM interfaces").right(),
        ],
        rows,
    )
    .single(selected.as_deref(), move |m, id| {
        let target = object(id).map(Target::one).unwrap_or_else(Target::none);
        m.select_in_grid(target);
    })
    .on_activate(move |m, id| {
        if let Some(object) = object(id) {
            m.select(Target::one(object));
        }
    })
    .on_clear(|m| m.select_in_grid(Target::none()))
    .empty(empty)
}

/// Networking of a pool: its networks, and the interfaces of its servers.
fn pool_networking(m: &Model, i: &Inventory, pool: PoolId) -> Vec<Block> {
    let interfaces = sorted(i, i.details.pifs.values());
    blocks![
        info::class_notice(i, "PIF"),
        info::class_notice(i, "VIF"),
        group(
            "Networks",
            blocks![networks_table(
                m,
                i,
                pool,
                |_| true,
                "No network of this pool is in the readable inventory."
            )]
        )
        .key("networking-networks"),
        group(
            "Interfaces",
            blocks![interfaces_table(
                m,
                i,
                "networking-interfaces-table",
                &interfaces,
                "No interface of this pool is in the readable inventory."
            )]
        )
        .key("networking-interfaces"),
        card(i, &interfaces, m.areas.network.pif.as_deref()),
        latest(m, pool),
    ]
}

/// Networking of a server: the networks its interfaces reach, and those
/// interfaces with their address configuration.
fn host_networking(m: &Model, i: &Inventory, pool: PoolId, host: HostId) -> Vec<Block> {
    let interfaces = sorted(i, i.details.pifs.values().filter(|p| p.host == Some(host)));
    let reached = |id: NetworkId| interfaces.iter().any(|p| p.network == Some(id));
    blocks![
        info::class_notice(i, "PIF"),
        group(
            "Networks of this server",
            blocks![networks_table(
                m,
                i,
                pool,
                reached,
                "No network of this server is in the readable inventory."
            )]
        )
        .key("networking-networks"),
        group(
            "Interfaces",
            blocks![interfaces_table(
                m,
                i,
                "networking-uplinks",
                &interfaces,
                "No interface of this server is in the readable inventory."
            )]
        )
        .key("networking-interfaces"),
        card(i, &interfaces, m.areas.network.pif.as_deref()),
        latest(m, pool),
    ]
}

/// Networking of a network: its host uplinks and its VM interfaces.
fn network_networking(m: &Model, i: &Inventory, pool: PoolId, id: NetworkId) -> Vec<Block> {
    let interfaces = sorted(i, i.details.pifs.values().filter(|p| p.network == Some(id)));
    let vifs = i
        .details
        .vifs
        .values()
        .filter(|nic| nic.network == Some(id))
        .map(|nic| {
            row(
                nic.id,
                [
                    info::vm_name(i, nic.vm).unwrap_or_else(|| "Unresolved VM".into()),
                    nic.id.to_string(),
                    nic.device.clone(),
                    nic.mac.clone(),
                    known(nic.currently_attached).to_owned(),
                ],
            )
        });
    blocks![
        info::class_notice(i, "PIF"),
        group(
            "Host uplinks",
            blocks![
                interfaces_table(
                    m,
                    i,
                    "network-uplinks-table",
                    &interfaces,
                    "No host uplink of this network is in the readable inventory."
                ),
                card(i, &interfaces, m.areas.network.pif.as_deref()),
                interfaces
                    .iter()
                    .map(|p| info::pif(i, p))
                    .collect::<Vec<_>>(),
            ]
        )
        .key("networking-uplinks"),
        group(
            "VM interfaces",
            blocks![
                info::class_notice(i, "VIF"),
                table("networking-vifs-table", info::vif_columns("VM"), vifs)
                    .empty("No VM interface of this network is in the readable inventory."),
            ]
        )
        .key("networking-vifs"),
        // The service state (NBD) and the tunnel and SR-IOV coverage of the
        // network: information of the `network_services` area, whose
        // commands are on this tab's row.
        crate::areas::network_services::networking_sections(i, id),
        latest(m, pool),
    ]
}

/// NICs of a server: the grid of its interfaces, then every interface with
/// its link metrics and bonds (the Rune "Host interfaces").
fn host_nics(m: &Model, i: &Inventory, pool: PoolId, host: HostId) -> Vec<Block> {
    let interfaces = sorted(i, i.details.pifs.values().filter(|p| p.host == Some(host)));
    blocks![
        info::class_notice(i, "PIF"),
        group(
            "Interfaces",
            blocks![interfaces_table(
                m,
                i,
                "nics-table",
                &interfaces,
                "No interface of this server is in the readable inventory."
            )]
        )
        .key("nics-grid"),
        card(i, &interfaces, m.areas.network.pif.as_deref()),
        group(
            "Host interfaces",
            blocks![
                interfaces
                    .iter()
                    .map(|p| info::pif(i, p))
                    .collect::<Vec<_>>(),
                interfaces.is_empty().then(|| caption(
                    "No interface of this server is in the readable inventory."
                )
                .key("pifs-empty")),
            ]
        )
        .key("nics-interfaces"),
        latest(m, pool),
    ]
}
