//! The commands of the area: the buttons the Rune "Host networking" hub
//! had on the card of a management interface, and the two buttons of its
//! IPv6 continuation panel.
//!
//! **The subject** of the first five is a management interface:
//!
//! - the target is a server (its context menu; the scope of its Networking
//!   or NICs tab): the management interface of that server;
//! - the target is a pool or a network (the scope of a Networking tab, or a
//!   network selected in its grid): the interface selected in the interface
//!   grid (`network::selected_pif`); without one, a network selected on
//!   the tab of a server stands for that server, and anything else asks
//!   for the selection.
//!
//! **Eligibility.** `Hidden` where Rune showed no button: another kind of
//! target, a selected interface that is not a management interface, the
//! IPv4 bond commands under IPv6 management and the reverse, Create where
//! the management parent is no physical interface, Remove where it is one.
//! `Disabled` with the Rune error of the form's `open` where the form could
//! not open, and with the words of the `network` area where Rune disabled
//! the button without a text (no demo and no writable current connection).
//! The two continuation commands are `Hidden` while the pool has no
//! unfinished handover and, as in Rune, need a writable connection also in
//! the demo.

use std::sync::Arc;

use xenadmin_core::{
    Inventory, ManagementBondOperation as Bond, ManagementInterface, PhysicalInterface, PifId,
    PoolId,
};

use super::{bond, ipv6, management};
use crate::areas::network::{self, commands::NOT_WRITABLE};
use crate::prelude::*;

const AREA: &str = "management_network";

pub const CONFIGURE: CommandId = CommandId("server.management.configure");
pub const MOVE: CommandId = CommandId("server.management.move");
pub const BOND_CREATE: CommandId = CommandId("server.management.bond.create");
pub const BOND_REMOVE: CommandId = CommandId("server.management.bond.remove");
pub const BOND_IPV6: CommandId = CommandId("server.management.bond.ipv6");
pub const IPV6_CONTINUE: CommandId = CommandId("server.management.bond.ipv6.continue");
pub const IPV6_LEAVE: CommandId = CommandId("server.management.bond.ipv6.leave");

/// No interface is selected and the scope is no server.
pub const SELECT_MANAGEMENT: &str = "Select the management interface of a server.";
/// The inventory has no management interface for the server.
pub const NO_MANAGEMENT: &str =
    "The management interface of this server is not in the inventory. Refresh first.";
/// Rune returned silently from "Remove management bond…".
pub const NO_BOND: &str =
    "The management bond or its primary member is not in the inventory. Refresh first.";
/// Rune returned silently from "Staged IPv6 management bond…".
pub const IPV6_INCOMPLETE: &str =
    "The management interface, its parent or its bond is incomplete in the inventory. Refresh first.";

pub fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new(CONFIGURE.0, "Configure Management IP", AREA)
            .local(TabId::Networking, 210)
            .context(&[ObjectKind::Host])
            .eligibility(|m, target| on_management(m, target, configurable))
            .invoke(|m, target| {
                if let Ok((pool, _, id)) = subject(m, target) {
                    management::open(m, pool, id, false);
                }
            }),
        CommandSpec::new(MOVE.0, "Move Management Interface", AREA)
            .local(TabId::Networking, 220)
            .context(&[ObjectKind::Host])
            .eligibility(|m, target| on_management(m, target, configurable))
            .invoke(|m, target| {
                if let Ok((pool, _, id)) = subject(m, target) {
                    management::open(m, pool, id, true);
                }
            }),
        CommandSpec::new(BOND_CREATE.0, "Create Management Bond", AREA)
            .local(TabId::Nics, 210)
            .context(&[ObjectKind::Host])
            .eligibility(|m, target| {
                on_management(m, target, |i, p| match parent(i, p) {
                    Parent::Ipv6 | Parent::Bonded => Eligibility::Hidden,
                    Parent::Unknown => Eligibility::disabled(bond::INCOMPLETE),
                    Parent::Physical => {
                        Eligibility::enabled_unless(bond::capture(i, p.id).err().map(String::from))
                    }
                })
            })
            .invoke(|m, target| {
                if let Ok((pool, _, id)) = subject(m, target) {
                    bond::open(m, pool, id);
                }
            }),
        CommandSpec::new(BOND_REMOVE.0, "Remove Management Bond", AREA)
            .local(TabId::Nics, 220)
            .context(&[ObjectKind::Host])
            .eligibility(|m, target| {
                on_management(m, target, |i, p| match parent(i, p) {
                    Parent::Ipv6 | Parent::Physical | Parent::Unknown => Eligibility::Hidden,
                    Parent::Bonded => {
                        Eligibility::enabled_if(bond::removal(i, p.id).is_some(), NO_BOND)
                    }
                })
            })
            .invoke(|m, target| {
                if let Ok((pool, _, id)) = subject(m, target) {
                    bond::remove(m, pool, id);
                }
            }),
        CommandSpec::new(BOND_IPV6.0, "Staged IPv6 Management Bond", AREA)
            .local(TabId::Nics, 230)
            .context(&[ObjectKind::Host])
            .eligibility(|m, target| {
                on_management(m, target, |i, p| match parent(i, p) {
                    Parent::Ipv6 => {
                        Eligibility::enabled_if(ipv6::capture(i, p.id).is_some(), IPV6_INCOMPLETE)
                    }
                    _ => Eligibility::Hidden,
                })
            })
            .invoke(|m, target| {
                if let Ok((pool, _, id)) = subject(m, target) {
                    ipv6::open(m, pool, id);
                }
            }),
        CommandSpec::new(IPV6_CONTINUE.0, "Review Next IPv6 Handover Stage", AREA)
            .local(TabId::Nics, 240)
            .local(TabId::Networking, 240)
            .context(&[ObjectKind::Host, ObjectKind::Pool])
            .eligibility(on_handover)
            .dynamic_label(|m, target| {
                handover(m, target)
                    .map(|(_, h)| format!("Review IPv6 Handover Stage {}", h.stage + 1))
            })
            .invoke(|m, target| {
                if let Some((pool, h)) = handover(m, target) {
                    ipv6::resume(m, pool, h.record, false);
                }
            }),
        CommandSpec::new(IPV6_LEAVE.0, "Leave Partial IPv6 Handover", AREA)
            .local(TabId::Nics, 250)
            .local(TabId::Networking, 250)
            .context(&[ObjectKind::Host, ObjectKind::Pool])
            .eligibility(on_handover)
            .invoke(|m, target| {
                if let Some((pool, h)) = handover(m, target) {
                    ipv6::resume(m, pool, h.record, true);
                }
            }),
    ]
}

type Found = Result<(PoolId, Arc<Inventory>, PifId), Eligibility>;

/// The management interface a command is about, or how the command
/// refuses the target (see the header).
pub fn subject(m: &Model, target: &Target) -> Found {
    let Some((object, pool)) = target.single().and_then(|o| Some((o, o.pool?))) else {
        return Err(Eligibility::Hidden);
    };
    let host = match object.kind {
        ObjectKind::Host => object.host_id(),
        ObjectKind::Pool | ObjectKind::Network => {
            if let Some((selected_pool, inventory, id)) = network::selected_pif(m)
                && selected_pool == pool
            {
                let management = inventory
                    .details
                    .pifs
                    .get(&id)
                    .is_some_and(|p| p.management == Some(true));
                return if management {
                    Ok((pool, inventory, id))
                } else {
                    Err(Eligibility::Hidden)
                };
            }
            // A network selected in the grid of a server's Networking
            // tab: the tab is still the server's.
            m.scope()
                .single_of(ObjectKind::Host)
                .filter(|scope| object.kind == ObjectKind::Network && scope.pool == Some(pool))
                .and_then(|scope| scope.host_id())
        }
        _ => return Err(Eligibility::Hidden),
    };
    let Some(inventory) = m.inventory_for(pool) else {
        return Err(Eligibility::Hidden);
    };
    let Some(host) = host else {
        return Err(Eligibility::disabled(SELECT_MANAGEMENT));
    };
    if !inventory.hosts.contains_key(&host) {
        return Err(Eligibility::Hidden);
    }
    let id = inventory
        .details
        .pifs
        .values()
        .find(|p| p.host == Some(host) && p.management == Some(true))
        .map(|p| p.id);
    match id {
        Some(id) => Ok((pool, inventory, id)),
        None => Err(Eligibility::disabled(NO_MANAGEMENT)),
    }
}

/// The eligibility of a command on the management interface: the subject,
/// the command's own rule, then the connection (the Rune `enabled`).
fn on_management(
    m: &Model,
    target: &Target,
    rule: impl FnOnce(&Inventory, &PhysicalInterface) -> Eligibility,
) -> Eligibility {
    let (pool, inventory, id) = match subject(m, target) {
        Ok(found) => found,
        Err(refusal) => return refusal,
    };
    let Some(pif) = inventory.details.pifs.get(&id) else {
        return Eligibility::Hidden;
    };
    match rule(&inventory, pif) {
        Eligibility::Enabled if !network::writable(m, pool) => Eligibility::disabled(NOT_WRITABLE),
        other => other,
    }
}

/// Change management IP and Move management: Rune showed both on every
/// management interface and reported a failed capture when pressed.
fn configurable(i: &Inventory, p: &PhysicalInterface) -> Eligibility {
    Eligibility::enabled_if(
        ManagementInterface::capture(i, p.id).is_some(),
        management::INCOMPLETE,
    )
}

/// Which of the three bond buttons the Rune card showed.
enum Parent {
    /// IPv6 is the primary family: the staged handover.
    Ipv6,
    /// The management parent is a physical interface: a bond can be made.
    Physical,
    /// The management parent is no physical interface: a bond can be
    /// removed.
    Bonded,
    /// The management interface cannot be captured. Rune showed "Remove
    /// management bond…", which then did nothing; Create says why.
    Unknown,
}

fn parent(i: &Inventory, p: &PhysicalInterface) -> Parent {
    if p.primary_address_type.as_deref() == Some("IPv6") {
        return Parent::Ipv6;
    }
    let Some(source) = ManagementInterface::capture(i, p.id) else {
        return Parent::Unknown;
    };
    let physical = Bond::management_parent(&source)
        .and_then(|id| i.details.pifs.get(&id))
        .is_some_and(|p| p.physical == Some(true));
    if physical {
        Parent::Physical
    } else {
        Parent::Bonded
    }
}

/// The unfinished IPv6 handover of the target's pool (the first one; the
/// pool is reserved while one is unfinished).
fn handover(m: &Model, target: &Target) -> Option<(PoolId, ipv6::Handover)> {
    let object = target.single()?;
    if !matches!(
        object.kind,
        ObjectKind::Pool | ObjectKind::Host | ObjectKind::Network
    ) {
        return None;
    }
    let pool = object.pool?;
    let first = ipv6::handovers(m, pool).into_iter().next()?;
    Some((pool, first))
}

fn on_handover(m: &Model, target: &Target) -> Eligibility {
    match handover(m, target) {
        Some((pool, _)) => Eligibility::enabled_if(m.can_mutate_pool(pool), NOT_WRITABLE),
        None => Eligibility::Hidden,
    }
}
