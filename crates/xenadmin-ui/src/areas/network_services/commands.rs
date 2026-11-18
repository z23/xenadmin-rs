//! The commands of the area: what the buttons of the Rune children
//! `service`, `topology`, `fabric` and `fabric_pool` in the hub of the
//! "Host networking" dialog became. Every one is about a network: the
//! command's `Target`, on the local row of the Networking tab (a network's
//! own tab, or the network selected in the grid of a pool's or a server's)
//! and in the context menu of a network.
//!
//! Eligibility follows the pattern of the `network` area: `Hidden` for
//! every kind but a network, `Disabled("Select a network.")` while the
//! target is still the pool or the server the grid belongs to; `Hidden`
//! where Rune showed no button (no record to remove); `Disabled` with the
//! Rune text where Rune disabled the button or its `open` refused, and
//! with the words of the `network` area for the connection rule (Rune
//! gave no text for `!enabled`).

use std::sync::Arc;

use xenadmin_core::{Inventory, NetworkId, NetworkServicePurpose, PoolId};

use super::edit::{self, Draft};
use super::{fabric, service, topology};
use crate::areas::network::{
    self,
    commands::{NOT_WRITABLE, SELECT_NETWORK},
};
use crate::prelude::*;

const AREA: &str = "network_services";

pub const SERVICE_TLS: CommandId = CommandId("network.service.nbd-tls");
pub const SERVICE_PLAINTEXT: CommandId = CommandId("network.service.nbd-plaintext");
pub const VLAN_CREATE_POOL: CommandId = CommandId("network.vlan.create-pool");
pub const VLAN_RETAG_POOL: CommandId = CommandId("network.vlan.retag-pool");
pub const FABRIC_ADD: CommandId = CommandId("network.fabric.add");
pub const FABRIC_COMPLETE_POOL: CommandId = CommandId("network.fabric.complete-pool");
pub const FABRIC_REMOVE: CommandId = CommandId("network.fabric.remove");
pub const FABRIC_REMOVE_POOL: CommandId = CommandId("network.fabric.remove-pool");

/// In the order of the local row, after the commands of the `network`
/// area (10 to 60).
pub const ALL: [CommandId; 8] = [
    VLAN_CREATE_POOL,
    VLAN_RETAG_POOL,
    SERVICE_TLS,
    SERVICE_PLAINTEXT,
    FABRIC_ADD,
    FABRIC_COMPLETE_POOL,
    FABRIC_REMOVE,
    FABRIC_REMOVE_POOL,
];

const NETWORK: &[ObjectKind] = &[ObjectKind::Network];

pub fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new(VLAN_CREATE_POOL.0, "Create Pool-wide VLANs", AREA)
            .local(TabId::Networking, 110)
            .context(NETWORK)
            .eligibility(|m, target| match on_network(m, target) {
                // Nothing a draft can enter helps while no other network
                // can be the uplink.
                Ok((_, i, id)) if topology::sources(&i, id).is_empty() => {
                    Eligibility::disabled(topology::NO_UPLINK)
                }
                Ok((pool, _, _)) => connection(m, pool),
                Err(refusal) => refusal,
            })
            .invoke(|m, target| open_topology(m, target, false)),
        CommandSpec::new(VLAN_RETAG_POOL.0, "Retag Pool-wide VLANs", AREA)
            .local(TabId::Networking, 120)
            .context(NETWORK)
            .eligibility(|m, target| match on_network(m, target) {
                Ok((pool, i, id)) => match topology::capture(&i, id, true) {
                    Ok(_) => connection(m, pool),
                    // Rune: the error of `open`, shown in the hub.
                    Err(reason) => Eligibility::disabled(reason),
                },
                Err(refusal) => refusal,
            })
            .invoke(|m, target| open_topology(m, target, true)),
        CommandSpec::new(SERVICE_TLS.0, "Enable NBD with TLS", AREA)
            .local(TabId::Networking, 130)
            .context(NETWORK)
            .eligibility(|m, t| service_eligibility(m, t, NetworkServicePurpose::NbdTls))
            .dynamic_label(|m, t| service_label(m, t, NetworkServicePurpose::NbdTls))
            .invoke(|m, t| open_service(m, t, NetworkServicePurpose::NbdTls)),
        CommandSpec::new(SERVICE_PLAINTEXT.0, "Enable NBD without TLS", AREA)
            .local(TabId::Networking, 140)
            .context(NETWORK)
            .eligibility(|m, t| service_eligibility(m, t, NetworkServicePurpose::NbdPlaintext))
            .dynamic_label(|m, t| service_label(m, t, NetworkServicePurpose::NbdPlaintext))
            .invoke(|m, t| open_service(m, t, NetworkServicePurpose::NbdPlaintext)),
        CommandSpec::new(FABRIC_ADD.0, "Add Tunnel or SR-IOV Interface", AREA)
            .local(TabId::Networking, 150)
            .context(NETWORK)
            .eligibility(writable_network)
            .invoke(|m, target| {
                if let Ok((pool, _, id)) = on_network(m, target) {
                    edit::open(m, pool, id, Draft::Fabric(fabric::Editor::default()));
                }
            }),
        CommandSpec::new(FABRIC_COMPLETE_POOL.0, "Complete Pool Coverage", AREA)
            .local(TabId::Networking, 160)
            .context(NETWORK)
            .eligibility(writable_network)
            .invoke(|m, target| {
                if let Ok((pool, _, id)) = on_network(m, target) {
                    edit::open(
                        m,
                        pool,
                        id,
                        Draft::FabricPool(fabric::PoolEditor::default()),
                    );
                }
            }),
        CommandSpec::new(FABRIC_REMOVE.0, "Remove Tunnel or SR-IOV Interface", AREA)
            .local(TabId::Networking, 170)
            .context(NETWORK)
            .eligibility(|m, target| match on_network(m, target) {
                Ok((pool, i, id)) => {
                    let records = fabric::records(&i, id);
                    if records.is_empty() {
                        // Rune: a button per record, none without one.
                        Eligibility::Hidden
                    } else if records.iter().any(|r| matches!(r.removal, Some(Ok(_)))) {
                        connection(m, pool)
                    } else {
                        Eligibility::disabled(
                            records
                                .iter()
                                .find_map(|r| match &r.removal {
                                    Some(Err(error)) => Some(fabric::removal_unavailable(error)),
                                    _ => None,
                                })
                                .unwrap_or_else(|| fabric::NOTHING_TO_REMOVE.into()),
                        )
                    }
                }
                Err(refusal) => refusal,
            })
            .invoke(|m, target| {
                if let Ok((pool, i, id)) = on_network(m, target) {
                    edit::open(m, pool, id, Draft::Removal(fabric::Removal::capture(&i, id)));
                }
            }),
        CommandSpec::new(FABRIC_REMOVE_POOL.0, "Remove Pool Coverage", AREA)
            .local(TabId::Networking, 180)
            .context(NETWORK)
            .eligibility(|m, target| match on_network(m, target) {
                // Rune: the button exists only while the core can build
                // the removal.
                Ok((pool, i, id)) => match fabric::pool_removal(&i, id) {
                    Some(_) => connection(m, pool),
                    None => Eligibility::Hidden,
                },
                Err(Eligibility::Disabled(_)) => Eligibility::Hidden,
                Err(refusal) => refusal,
            })
            .dynamic_label(|m, target| {
                let (_, i, id) = on_network(m, target).ok()?;
                fabric::pool_removal(&i, id).map(|(operation, _)| operation.label().to_owned())
            })
            .invoke(|m, target| {
                if let Ok((pool, _, id)) = on_network(m, target) {
                    fabric::open_pool_removal(m, pool, id);
                }
            }),
    ]
}

/// The two service commands are the Rune buttons of `service::view`:
/// "Enable / Remove NBD with TLS" and "Enable / Remove NBD without TLS".
fn service_eligibility(m: &Model, target: &Target, purpose: NetworkServicePurpose) -> Eligibility {
    match on_network(m, target) {
        Ok((pool, i, id)) => match service::offer(&i, id, purpose) {
            // Rune: the text in place of the buttons, and the caption
            // under a disabled one.
            Err(reason) => Eligibility::disabled(reason),
            Ok(offer) => match offer.available {
                Err(reason) => Eligibility::disabled(reason),
                Ok(()) => connection(m, pool),
            },
        },
        Err(refusal) => refusal,
    }
}

fn service_label(m: &Model, target: &Target, purpose: NetworkServicePurpose) -> Option<String> {
    let (_, i, id) = on_network(m, target).ok()?;
    let offer = service::offer(&i, id, purpose).ok()?;
    Some(service::label(purpose, offer.enable))
}

fn open_service(m: &mut Model, target: &Target, purpose: NetworkServicePurpose) {
    if let Ok((pool, _, id)) = on_network(m, target) {
        service::open(m, pool, id, purpose);
    }
}

fn open_topology(m: &mut Model, target: &Target, changing: bool) {
    let Ok((pool, i, id)) = on_network(m, target) else {
        return;
    };
    match topology::capture(&i, id, changing) {
        Ok(editor) => edit::open(m, pool, id, Draft::Topology(editor)),
        Err(error) => m.error = Some(error),
    }
}

/// The Rune `enabled` as an eligibility.
fn connection(m: &Model, pool: PoolId) -> Eligibility {
    Eligibility::enabled_if(network::writable(m, pool), NOT_WRITABLE)
}

fn writable_network(m: &Model, target: &Target) -> Eligibility {
    match on_network(m, target) {
        Ok((pool, _, _)) => connection(m, pool),
        Err(refusal) => refusal,
    }
}

type Found = Result<(PoolId, Arc<Inventory>, NetworkId), Eligibility>;

/// The network a command is about, or how the command refuses the target
/// (the rule of the `network` area's commands on a network).
pub fn on_network(m: &Model, target: &Target) -> Found {
    let Some((object, pool)) = target.single().and_then(|o| Some((o, o.pool?))) else {
        return Err(Eligibility::Hidden);
    };
    match object.kind {
        ObjectKind::Network => {
            let inventory = m.inventory_for(pool).ok_or(Eligibility::Hidden)?;
            let uuid = object
                .uuid()
                .filter(|uuid| inventory.networks.iter().any(|n| n.uuid == *uuid))
                .ok_or(Eligibility::Hidden)?;
            Ok((pool, inventory, NetworkId::from(uuid)))
        }
        ObjectKind::Pool | ObjectKind::Host => Err(Eligibility::disabled(SELECT_NETWORK)),
        _ => Err(Eligibility::Hidden),
    }
}
