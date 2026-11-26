//! The commands of the area: what the buttons of the Rune "Host
//! networking" hub, of the inspector (`resource_admin::buttons`) and of the
//! pool hub became.
//!
//! Three kinds of target:
//!
//! - **an object** (a network, a server, a storage repository): the
//!   command's `Target`. In the local row of a grid of such objects the
//!   command is `Disabled("Select a network.")` (or repository) while the
//!   target is still the pool or the server the grid belongs to; `Hidden`
//!   for every other kind.
//! - **an interface**: a physical interface is no tree object, so the
//!   commands on one read the row selected in the interface grid of the
//!   Networking or NICs tab ([`selected_pif`]). They are in the row menu of
//!   that grid; the plan's ones are also in the local row.
//! - **the pool** of the scope, for New Network.
//!
//! Eligibility: `Hidden` where Rune did not show the button at all (no
//! bond on the interface, no VLAN); `Disabled` with the reason where Rune
//! disabled it. Rune gave no text for most of them: the texts here say the
//! Rune condition. The connection rule is the Rune `enabled`: the demo, or
//! a writable current connection.

use std::sync::Arc;

use xenadmin_core::{
    BondHash, BondMode, HostId, Inventory, NetworkId, PhysicalInterface, PifId, PoolId,
    ResourceTarget, SrId,
};

use super::editor::{self, Kind};
use super::workspace::{self, Form};
use crate::prelude::*;

const AREA: &str = "network";

pub const NEW: CommandId = CommandId("network.new");
pub const VLAN_ADD: CommandId = CommandId("network.vlan.add");
pub const REMOVE: CommandId = CommandId("network.remove");
pub const NIC_IPV4: CommandId = CommandId("nic.ipv4");
pub const NIC_IPV6: CommandId = CommandId("nic.ipv6");
pub const NIC_PURPOSE: CommandId = CommandId("nic.purpose");
pub const NIC_PLUG: CommandId = CommandId("nic.plug");
pub const NIC_PROTECTION: CommandId = CommandId("nic.unplug-protection");
pub const NIC_VLAN_DELETE: CommandId = CommandId("nic.vlan.delete");
pub const BOND_CREATE: CommandId = CommandId("server.bond.create");
pub const BOND_DELETE: CommandId = CommandId("server.bond.delete");
pub const BOND_MODE: CommandId = CommandId("server.bond.mode");
pub const BOND_HASH: CommandId = CommandId("server.bond.hash");
pub const NICS_RESCAN: CommandId = CommandId("server.nics.rescan");
pub const SCHEDULING: CommandId = CommandId("server.scheduling");
pub const SET_DEFAULT: CommandId = CommandId("storage.set-default");
pub const STORAGE_RESCAN: CommandId = CommandId("storage.rescan");

/// The commands on an interface, in the order of the Rune card: the row
/// menu of every interface grid of this area.
pub const INTERFACE_MENU: [CommandId; 9] = [
    NIC_PLUG,
    NIC_IPV4,
    NIC_IPV6,
    NIC_PURPOSE,
    NIC_PROTECTION,
    NIC_VLAN_DELETE,
    BOND_MODE,
    BOND_HASH,
    BOND_DELETE,
];

/// Rune disabled its buttons without saying why when the pool is not the
/// demo and has no writable current connection. The words are those the
/// other areas use for the same condition.
pub const NOT_WRITABLE: &str = "This connection is read-only, disconnected or stale.";
pub const SELECT_NETWORK: &str = "Select a network.";
pub const SELECT_REPOSITORY: &str = "Select a storage repository.";
pub const SELECT_INTERFACE: &str = "Select an interface.";
pub const SELECT_BOND: &str = "Select the interface of a bond.";
/// The Rune caption of the inspector, word for word.
pub const NETWORK_USED: &str = "MTU changes and deletion require an unused internal network. This network has host or VM interfaces.";
/// The Rune caption of the interface card, word for word.
pub const NO_IPV6: &str = "IPv6 configuration requires IPv6 to be enabled on the host management interface at installation.";
/// The two errors of the Rune default-storage editor, word for word.
pub const DEFAULT_UNKNOWN: &str =
    "The current default storage repository is unknown. Refresh before changing it.";
pub const ALREADY_DEFAULT: &str = "This repository is already the default.";
/// Rune: `management == Some(false)`.
pub const MANAGEMENT: &str =
    "Not available for the host management interface, or while the role of the interface is unknown.";
/// Rune: `managed == Some(true) && management == Some(false)`.
pub const NOT_SECONDARY: &str =
    "Only a managed interface that is not the host management interface can be changed here.";
/// Rune returned silently from the editor's `open`.
pub const INCOMPLETE: &str =
    "The identity or the configuration of this interface is incomplete. Refresh first.";
pub const PROTECTED: &str =
    "The host management interface and an interface protected from unplugging cannot be unplugged.";
pub const NO_PRIMARY: &str = "The primary member of this bond is unknown. Refresh first.";

/// In the order of the local rows. Labels: "Add Network", "Remove",
/// "Create Bond", "Delete Bond", "Rescan" and "Set as Default" are the
/// plan's; the others exist only in this client.
pub fn commands() -> Vec<CommandSpec> {
    vec![
        // -- Networking tab -------------------------------------------------
        CommandSpec::new(NEW.0, "Add Network", AREA)
            .local(TabId::Networking, 10)
            .local_scopes(&[ObjectKind::Pool, ObjectKind::Host, ObjectKind::Network])
            .eligibility(|m, target| match new_pool(m, target) {
                Some(pool) => Eligibility::enabled_if(writable(m, pool), NOT_WRITABLE),
                None => Eligibility::Hidden,
            })
            .invoke(|m, target| {
                if let Some(pool) = new_pool(m, target) {
                    editor::open(m, pool, ResourceTarget::Pool(pool), Kind::CreateNetwork);
                }
            }),
        CommandSpec::new(VLAN_ADD.0, "Add VLAN Interface", AREA)
            .local(TabId::Networking, 20)
            .local_scopes(&[ObjectKind::Pool, ObjectKind::Host, ObjectKind::Network])
            .context(&[ObjectKind::Network])
            .eligibility(|m, target| match network(m, target) {
                Ok((pool, _, _)) => Eligibility::enabled_if(writable(m, pool), NOT_WRITABLE),
                Err(refusal) => refusal,
            })
            .invoke(|m, target| {
                if let Ok((pool, _, id)) = network(m, target) {
                    let form = Form::Vlan {
                        parent: None,
                        tag: "100".into(),
                    };
                    workspace::open(m, pool, Some(id), scope_host(m, pool), form);
                }
            }),
        CommandSpec::new(REMOVE.0, "Remove Network", AREA)
            .local(TabId::Networking, 30)
            .local_scopes(&[ObjectKind::Pool, ObjectKind::Host, ObjectKind::Network])
            .context(&[ObjectKind::Network])
            .eligibility(|m, target| match network(m, target) {
                Ok((_, inventory, id)) if workspace::network_used(&inventory, id) => {
                    Eligibility::disabled(NETWORK_USED)
                }
                Ok((pool, _, _)) => Eligibility::enabled_if(writable(m, pool), NOT_WRITABLE),
                Err(refusal) => refusal,
            })
            .invoke(|m, target| {
                if let Ok((pool, _, id)) = network(m, target) {
                    open_reviewed(m, pool, ResourceTarget::Network(id), Kind::DeleteNetwork);
                }
            }),
        CommandSpec::new(NIC_IPV4.0, "Configure IPv4", AREA)
            .local(TabId::Networking, 40)
            .local_scopes(&[ObjectKind::Pool, ObjectKind::Host, ObjectKind::Network])
            .eligibility(|m, _| {
                on_interface(m, |i, p| {
                    if p.management != Some(false) {
                        Eligibility::disabled(MANAGEMENT)
                    } else {
                        Eligibility::enabled_if(
                            workspace::ipv4_form(i, p.id).is_some(),
                            INCOMPLETE,
                        )
                    }
                })
            })
            .invoke(|m, _| open_form(m, |i, p| workspace::ipv4_form(i, p.id))),
        CommandSpec::new(NIC_IPV6.0, "Configure IPv6", AREA)
            .local(TabId::Networking, 50)
            .local_scopes(&[ObjectKind::Pool, ObjectKind::Host, ObjectKind::Network])
            .eligibility(|m, _| {
                on_interface(m, |i, p| {
                    if !workspace::secondary(p) {
                        Eligibility::disabled(NOT_SECONDARY)
                    } else if !workspace::host_ipv6(i, p) {
                        Eligibility::disabled(NO_IPV6)
                    } else {
                        Eligibility::enabled_if(
                            workspace::interface_form(i, p.id, false).is_some(),
                            INCOMPLETE,
                        )
                    }
                })
            })
            .invoke(|m, _| open_form(m, |i, p| workspace::interface_form(i, p.id, false))),
        CommandSpec::new(NIC_PURPOSE.0, "Interface Purpose", AREA)
            .local(TabId::Networking, 60)
            .local_scopes(&[ObjectKind::Pool, ObjectKind::Host, ObjectKind::Network])
            .eligibility(|m, _| {
                on_interface(m, |i, p| {
                    if !workspace::secondary(p) {
                        Eligibility::disabled(NOT_SECONDARY)
                    } else {
                        Eligibility::enabled_if(
                            workspace::interface_form(i, p.id, true).is_some(),
                            INCOMPLETE,
                        )
                    }
                })
            })
            .invoke(|m, _| open_form(m, |i, p| workspace::interface_form(i, p.id, true))),
        // -- The row menu of an interface ------------------------------------
        CommandSpec::new(NIC_PLUG.0, "Plug Interface", AREA)
            .dynamic_label(|m, _| {
                let (_, inventory, id) = selected_pif(m)?;
                let attached = inventory.details.pifs.get(&id)?.currently_attached?;
                attached.then(|| "Unplug Interface".to_owned())
            })
            .eligibility(|m, _| {
                on_interface(m, |i, p| {
                    if crate::areas::network_services::capacity::needs_driver_review(i, p.id) {
                        Eligibility::disabled(
                            crate::areas::network_services::capacity::SRIOV_DRIVER_REVIEW,
                        )
                    } else if workspace::attach(i, p).is_none() {
                        Eligibility::disabled(INCOMPLETE)
                    } else {
                        Eligibility::enabled_if(workspace::may_change_attachment(p), PROTECTED)
                    }
                })
            })
            .invoke(|m, _| open_staged(m, workspace::attach)),
        CommandSpec::new(NIC_PROTECTION.0, "Protect from Unplugging", AREA)
            .dynamic_label(|m, _| {
                let (_, inventory, id) = selected_pif(m)?;
                let protected = inventory.details.pifs.get(&id)?.disallow_unplug?;
                protected.then(|| "Allow Unplugging".to_owned())
            })
            .eligibility(|m, _| {
                on_interface(m, |i, p| {
                    if workspace::protection(i, p).is_none() {
                        Eligibility::disabled(INCOMPLETE)
                    } else {
                        Eligibility::enabled_if(workspace::secondary(p), NOT_SECONDARY)
                    }
                })
            })
            .invoke(|m, _| open_staged(m, workspace::protection)),
        CommandSpec::new(NIC_VLAN_DELETE.0, "Delete VLAN Interface", AREA)
            .eligibility(|m, _| {
                on_interface(m, |i, p| {
                    if workspace::delete_vlan(i, p).is_none() {
                        Eligibility::Hidden
                    } else {
                        Eligibility::enabled_if(p.management == Some(false), MANAGEMENT)
                    }
                })
            })
            .invoke(|m, _| open_staged(m, workspace::delete_vlan)),
        CommandSpec::new(BOND_MODE.0, "Change Bond Mode", AREA)
            .eligibility(|m, _| {
                on_interface(m, |i, p| {
                    if workspace::bond_of(i, p.id).is_none() {
                        Eligibility::Hidden
                    } else {
                        Eligibility::enabled_if(p.management == Some(false), MANAGEMENT)
                    }
                })
            })
            .invoke(|m, _| {
                open_form(m, |i, p| {
                    let bond = workspace::bond_of(i, p.id)?;
                    Some(Form::BondMode {
                        bond: bond.id,
                        mode: None::<BondMode>,
                    })
                })
            }),
        CommandSpec::new(BOND_HASH.0, "Change Bond Hash", AREA)
            .eligibility(|m, _| {
                on_interface(m, |i, p| {
                    let hashes = workspace::bond_of(i, p.id).map(workspace::other_hashes);
                    if hashes.is_none_or(|hashes| hashes.is_empty()) {
                        Eligibility::Hidden
                    } else {
                        Eligibility::enabled_if(p.management == Some(false), MANAGEMENT)
                    }
                })
            })
            .invoke(|m, _| {
                open_form(m, |i, p| {
                    let bond = workspace::bond_of(i, p.id)?;
                    // One other hash: it is the change. Two (the current one
                    // is neither): the user chooses.
                    let hashes = workspace::other_hashes(bond);
                    let hash: Option<BondHash> = match hashes.as_slice() {
                        [only] => Some(*only),
                        _ => None,
                    };
                    Some(Form::BondHash {
                        bond: bond.id,
                        hash,
                    })
                })
            }),
        // -- NICs tab -------------------------------------------------------
        CommandSpec::new(BOND_CREATE.0, "Create Bond", AREA)
            .local(TabId::Nics, 10)
            .local_scopes(&[ObjectKind::Host])
            .eligibility(|m, target| match server(m, target) {
                Some((pool, _)) => Eligibility::enabled_if(writable(m, pool), NOT_WRITABLE),
                None => Eligibility::Hidden,
            })
            .invoke(|m, target| {
                if let Some((pool, host)) = server(m, target) {
                    let form = Form::Bond {
                        members: vec![],
                        mode: BondMode::ActiveBackup,
                        hash: BondHash::SourceMac,
                    };
                    workspace::open(m, pool, None, Some(host), form);
                }
            }),
        CommandSpec::new(BOND_DELETE.0, "Delete Bond", AREA)
            .local(TabId::Nics, 20)
            .local_scopes(&[ObjectKind::Host])
            .eligibility(|m, _| {
                // In the local row of the NICs tab the missing selection is
                // a missing bond.
                match on_interface(m, |i, p| {
                    if workspace::bond_of(i, p.id).is_none() {
                        Eligibility::disabled(SELECT_BOND)
                    } else if workspace::delete_bond(i, p).is_none() {
                        Eligibility::disabled(NO_PRIMARY)
                    } else {
                        Eligibility::enabled_if(p.management == Some(false), MANAGEMENT)
                    }
                }) {
                    Eligibility::Disabled(reason) if reason == SELECT_INTERFACE => {
                        Eligibility::disabled(SELECT_BOND)
                    }
                    other => other,
                }
            })
            .invoke(|m, _| open_staged(m, workspace::delete_bond)),
        CommandSpec::new(NICS_RESCAN.0, "Rescan", AREA)
            .local(TabId::Nics, 30)
            .local_scopes(&[ObjectKind::Host])
            .eligibility(|m, target| match server(m, target) {
                Some((pool, _)) => Eligibility::enabled_if(writable(m, pool), NOT_WRITABLE),
                None => Eligibility::Hidden,
            })
            .invoke(|m, target| {
                let Some((pool, host)) = server(m, target) else {
                    return;
                };
                let staged = m
                    .inventory_for(pool)
                    .and_then(|inventory| workspace::scan(&inventory, host));
                if let Some(staged) = staged {
                    workspace::open_staged(m, pool, None, Some(host), staged);
                }
            }),
        // -- The generic editor on a server and on a storage repository -------
        CommandSpec::new(SCHEDULING.0, "Disable New VM Placement", AREA)
            .menu(MenuPath::new(Menu::Server, 5, 12))
            .context(&[ObjectKind::Host])
            .dynamic_label(|m, target| {
                let (pool, host) = server(m, target)?;
                let enabled = m.inventory_for(pool)?.hosts.get(&host)?.enabled;
                (!enabled).then(|| "Enable New VM Placement".to_owned())
            })
            .eligibility(|m, target| match server(m, target) {
                Some((pool, _)) => Eligibility::enabled_if(writable(m, pool), NOT_WRITABLE),
                None => Eligibility::Hidden,
            })
            .invoke(|m, target| {
                let Some((pool, host)) = server(m, target) else {
                    return;
                };
                let enabled = m
                    .inventory_for(pool)
                    .and_then(|inventory| inventory.hosts.get(&host).map(|h| h.enabled));
                if let Some(enabled) = enabled {
                    open_reviewed(
                        m,
                        pool,
                        ResourceTarget::Host(host),
                        Kind::HostEnabled(!enabled),
                    );
                }
            }),
        CommandSpec::new(SET_DEFAULT.0, "Set as Default", AREA)
            .menu(place::STORAGE_SET_DEFAULT)
            .context(&[ObjectKind::StorageRepository])
            .local(TabId::Storage, 30)
            .local_scopes(&[ObjectKind::Pool, ObjectKind::Host, ObjectKind::StorageRepository])
            .eligibility(|m, target| match repository(m, target) {
                Ok((pool, inventory, id)) => {
                    // Rune offered only repositories for VM disks.
                    let user = inventory
                        .details
                        .storage
                        .get(&id)
                        .is_some_and(|details| details.content_type == "user");
                    let details = inventory.details.pool.as_ref();
                    if !user {
                        Eligibility::Hidden
                    } else if !details.is_some_and(|p| p.default_sr_known) {
                        Eligibility::disabled(DEFAULT_UNKNOWN)
                    } else if details.and_then(|p| p.default_sr) == Some(id) {
                        Eligibility::disabled(ALREADY_DEFAULT)
                    } else {
                        Eligibility::enabled_if(writable(m, pool), NOT_WRITABLE)
                    }
                }
                Err(refusal) => refusal,
            })
            .invoke(|m, target| {
                let Ok((pool, _, id)) = repository(m, target) else {
                    return;
                };
                // The pool is the target of the change; the repository the
                // command ran on is the choice the Rune dialog asked for.
                if editor::open(m, pool, ResourceTarget::Pool(pool), Kind::DefaultStorage) {
                    if let Some(editor) = &mut m.areas.network.editor {
                        editor.selected_sr = Some(id);
                    }
                    editor::review(m);
                }
            }),
        CommandSpec::new(STORAGE_RESCAN.0, "Rescan", AREA)
            .menu(MenuPath::new(Menu::Storage, 2, 30))
            .context(&[ObjectKind::StorageRepository])
            .local(TabId::Storage, 35)
            .local_scopes(&[ObjectKind::Pool, ObjectKind::Host, ObjectKind::StorageRepository])
            .eligibility(|m, target| match repository(m, target) {
                Ok((pool, _, _)) => Eligibility::enabled_if(writable(m, pool), NOT_WRITABLE),
                Err(refusal) => refusal,
            })
            .invoke(|m, target| {
                if let Ok((pool, _, id)) = repository(m, target) {
                    open_reviewed(m, pool, ResourceTarget::Storage(id), Kind::ScanStorage);
                }
            }),
    ]
}

/// The Rune `enabled`: the demo, or a writable current connection.
pub fn writable(m: &Model, pool: PoolId) -> bool {
    m.is_synthetic_pool(pool) || m.can_mutate_pool(pool)
}

/// The pool a new network would be created in: that of a pool, a server or
/// a network whose inventory is there.
fn new_pool(m: &Model, target: &Target) -> Option<PoolId> {
    let object = target.single()?;
    matches!(
        object.kind,
        ObjectKind::Pool | ObjectKind::Host | ObjectKind::Network
    )
    .then_some(object.pool?)
    .filter(|pool| m.inventory_for(*pool).is_some())
}

type Found<T> = Result<(PoolId, Arc<Inventory>, T), Eligibility>;

/// The network a command is about, or how the command refuses the target.
fn network(m: &Model, target: &Target) -> Found<NetworkId> {
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

/// The storage repository a command is about, likewise.
fn repository(m: &Model, target: &Target) -> Found<SrId> {
    let Some((object, pool)) = target.single().and_then(|o| Some((o, o.pool?))) else {
        return Err(Eligibility::Hidden);
    };
    match object.kind {
        ObjectKind::StorageRepository => {
            let inventory = m.inventory_for(pool).ok_or(Eligibility::Hidden)?;
            let uuid = object
                .uuid()
                .filter(|uuid| inventory.storage.iter().any(|sr| sr.uuid == *uuid))
                .ok_or(Eligibility::Hidden)?;
            Ok((pool, inventory, SrId::from(uuid)))
        }
        ObjectKind::Pool | ObjectKind::Host => Err(Eligibility::disabled(SELECT_REPOSITORY)),
        _ => Err(Eligibility::Hidden),
    }
}

/// The one server of a target, in the inventory of its pool.
fn server(m: &Model, target: &Target) -> Option<(PoolId, HostId)> {
    let object = target.single_of(ObjectKind::Host)?;
    let (pool, host) = (object.pool?, object.host_id()?);
    m.inventory_for(pool)?
        .hosts
        .contains_key(&host)
        .then_some((pool, host))
}

/// The server of the scope, when the scope is a server of this pool: the
/// Rune workspace offered only its interfaces.
fn scope_host(m: &Model, pool: PoolId) -> Option<HostId> {
    m.scope()
        .single_of(ObjectKind::Host)
        .filter(|object| object.pool == Some(pool))
        .and_then(|object| object.host_id())
}

/// The interface selected in the interface grid of the scope's Networking
/// or NICs tab, when the scope still shows it. Public: the commands of the
/// management network area act on the same selection.
pub fn selected_pif(m: &Model) -> Option<(PoolId, Arc<Inventory>, PifId)> {
    let scope = m.scope().single()?;
    let pool = scope.pool?;
    let inventory = m.inventory_for(pool)?;
    let selected = m.areas.network.pif.as_deref()?;
    let pif = inventory
        .details
        .pifs
        .values()
        .find(|p| p.id.to_string() == selected)?;
    let shown = match scope.kind {
        ObjectKind::Pool => true,
        ObjectKind::Host => pif.host.is_some() && pif.host == scope.host_id(),
        ObjectKind::Network => scope
            .uuid()
            .is_some_and(|uuid| pif.network == Some(NetworkId::from(uuid))),
        _ => false,
    };
    let id = pif.id;
    shown.then_some((pool, inventory, id))
}

/// The eligibility of a command on the selected interface: `Hidden` on a
/// scope without interface grid, the selection, the connection, then the
/// command's own rule.
fn on_interface(
    m: &Model,
    rule: impl FnOnce(&Inventory, &PhysicalInterface) -> Eligibility,
) -> Eligibility {
    let kind = m.scope().single().map(|object| object.kind);
    if !matches!(
        kind,
        Some(ObjectKind::Pool | ObjectKind::Host | ObjectKind::Network)
    ) {
        return Eligibility::Hidden;
    }
    let Some((pool, inventory, id)) = selected_pif(m) else {
        return Eligibility::disabled(SELECT_INTERFACE);
    };
    let Some(pif) = inventory.details.pifs.get(&id) else {
        return Eligibility::disabled(SELECT_INTERFACE);
    };
    match rule(&inventory, pif) {
        Eligibility::Enabled if !writable(m, pool) => Eligibility::disabled(NOT_WRITABLE),
        other => other,
    }
}

/// Opens the dialog of the selected interface on a form.
fn open_form(
    m: &mut Model,
    form: impl FnOnce(&Inventory, &PhysicalInterface) -> Option<Form>,
) {
    let Some((pool, inventory, id)) = selected_pif(m) else {
        return;
    };
    let Some(pif) = inventory.details.pifs.get(&id) else {
        return;
    };
    if let Some(form) = form(&inventory, pif) {
        workspace::open(m, pool, pif.network, pif.host, form);
    }
}

/// Opens the dialog of the selected interface on a complete review.
fn open_staged(
    m: &mut Model,
    staged: impl FnOnce(&Inventory, &PhysicalInterface) -> Option<workspace::Staged>,
) {
    let Some((pool, inventory, id)) = selected_pif(m) else {
        return;
    };
    let Some(pif) = inventory.details.pifs.get(&id) else {
        return;
    };
    if let Some(staged) = staged(&inventory, pif) {
        workspace::open_staged(m, pool, pif.network, pif.host, staged);
    }
}

/// A change of the generic editor with nothing to enter: its dialog opens
/// on the review.
fn open_reviewed(m: &mut Model, pool: PoolId, target: ResourceTarget, kind: Kind) {
    if editor::open(m, pool, target, kind) {
        editor::review(m);
    }
}
