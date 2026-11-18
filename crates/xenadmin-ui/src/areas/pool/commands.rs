//! The commands of the area: what the buttons of the Rune "Pool
//! administration" dialog became, and the two entries of the Rune
//! connection manager (join, root password).
//!
//! Eligibility follows one rule. `Hidden`: the target is not a pool (or a
//! server, for the server commands). `Disabled`, with a reason: Rune
//! disabled or hid the button on that state of the pool (HA still on, a
//! value the host did not report, the coordinator itself, no writable
//! connection where Rune asked for one). `Enabled` otherwise: the surface
//! opens and its review makes every check again.
//!
//! The local action row of the HA tab acts on the tab's grid selection when
//! there is one, so with a VM chosen the target of a pool command is that
//! VM: [`pool_of`] resolves the pool for both cases.

use xenadmin_core::{HostId, Inventory, PoolId, PoolOperation, ResourceTarget, VmId};

use super::policy::{self, Kind};
use super::{DEMO, HA_FIRST, NOT_WRITABLE, dissolve, join, root_password};
use crate::prelude::*;

const AREA: &str = "pool";

pub const JOIN: CommandId = CommandId("server.join-pool");
pub const REMOVE_SERVER: CommandId = CommandId("pool.remove-server");
pub const HA_CONFIGURE: CommandId = CommandId("pool.ha.configure");
pub const HA_DISABLE: CommandId = CommandId("pool.ha.disable");
pub const HA_FAILURES: CommandId = CommandId("pool.ha.failures-to-tolerate");
pub const HA_OVERCOMMIT: CommandId = CommandId("pool.ha.overcommit");
pub const HA_TIMEOUT: CommandId = CommandId("pool.ha.timeout");
pub const HA_RESTART_PRIORITY: CommandId = CommandId("vm.ha.restart-priority");
pub const CHANGE_SERVER_PASSWORD: CommandId = CommandId("pool.change-server-password");
pub const MAKE_STANDALONE: CommandId = CommandId("pool.make-standalone");
pub const SERVER_CHANGE_PASSWORD: CommandId = CommandId("server.change-password");
pub const DESIGNATE_COORDINATOR: CommandId = CommandId("pool.designate-coordinator");
pub const REMOVE_FROM_POOL: CommandId = CommandId("server.remove-from-pool");

/// Rune hid these buttons on the state they name; a command says why.
pub const HA_ON: &str = "High availability is already enabled. Disable it before configuring it again.";
pub const HA_OFF: &str = "High availability is not enabled.";
pub const HA_UNKNOWN: &str = "HA: Unknown / unavailable";
pub const FAILURES_UNKNOWN: &str = "HA failure tolerance is unavailable";
pub const OVERCOMMIT_UNKNOWN: &str = "Allow HA overcommit: Unknown / unavailable";
pub const NO_COORDINATOR: &str = "Pool coordinator is unavailable";
pub const NO_MEMBER: &str = "This pool has no member besides its coordinator.";
pub const SELECT_VM: &str = "Select a VM";
pub const NOT_STANDALONE: &str = "Only a standalone source host can join.";

/// The planned handover has no row in the plan's Menus table: it sits with
/// the membership items of the Server menu, before Remove from Pool.
const SERVER_DESIGNATE_COORDINATOR: MenuPath = MenuPath::new(Menu::Server, 6, 5);

pub fn commands() -> Vec<CommandSpec> {
    const POOL: &[ObjectKind] = &[ObjectKind::Pool];
    const SERVER: &[ObjectKind] = &[ObjectKind::Host];
    vec![
        // The Rune direction: the standalone server that is selected joins
        // another connected pool.
        CommandSpec::new(JOIN.0, "Add to Pool", AREA)
            .menu(place::POOL_ADD_SERVER)
            .context(&[ObjectKind::Pool, ObjectKind::Host])
            .eligibility(can_join)
            .invoke(|m, target| {
                if let Some(pool) = pool_or_server(m, target) {
                    join::open(m, pool);
                }
            }),
        CommandSpec::new(REMOVE_SERVER.0, "Remove Server", AREA)
            .menu(place::POOL_REMOVE_SERVER)
            .context(POOL)
            .eligibility(can_remove_server)
            .invoke(remove_server),
        CommandSpec::new(HA_CONFIGURE.0, "High Availability", AREA)
            .menu(place::POOL_HIGH_AVAILABILITY)
            .context(POOL)
            .local(TabId::Ha, 10)
            .local_scopes(&[ObjectKind::Pool])
            .eligibility(|m, target| ha(m, target, can_configure))
            .invoke(|m, target| {
                if let Some(pool) = pool_of(m, target) {
                    policy::open(m, pool, Kind::Configure);
                }
            }),
        CommandSpec::new(HA_DISABLE.0, "Disable HA", AREA)
            .local(TabId::Ha, 20)
            .local_scopes(&[ObjectKind::Pool])
            .eligibility(|m, target| ha(m, target, can_disable))
            .invoke(|m, target| {
                if let Some(pool) = pool_of(m, target) {
                    policy::open_review(m, pool, Kind::Disable, PoolOperation::DisableHa);
                }
            }),
        CommandSpec::new(HA_FAILURES.0, "Failure Tolerance", AREA)
            .local(TabId::Ha, 30)
            .local_scopes(&[ObjectKind::Pool])
            .eligibility(|m, target| {
                ha(m, target, |i| {
                    let known = policy::administration(i).ha_failures_to_tolerate.is_some();
                    Eligibility::enabled_if(known, FAILURES_UNKNOWN)
                })
            })
            .invoke(|m, target| {
                if let Some(pool) = pool_of(m, target) {
                    policy::open(m, pool, Kind::Failures);
                }
            }),
        CommandSpec::new(HA_OVERCOMMIT.0, "HA Overcommit", AREA)
            .local(TabId::Ha, 40)
            .local_scopes(&[ObjectKind::Pool])
            .dynamic_label(overcommit_label)
            .eligibility(|m, target| {
                ha(m, target, |i| {
                    let known = policy::administration(i).ha_allow_overcommit.is_some();
                    Eligibility::enabled_if(known, OVERCOMMIT_UNKNOWN)
                })
            })
            .invoke(overcommit),
        CommandSpec::new(HA_TIMEOUT.0, "HA Timeout Default", AREA)
            .local(TabId::Ha, 50)
            .local_scopes(&[ObjectKind::Pool])
            .eligibility(|m, target| {
                ha(m, target, |i| {
                    let editable = policy::timeout_editable(&policy::administration(i));
                    Eligibility::enabled_if(editable, policy::TIMEOUT_FIXED)
                })
            })
            .invoke(|m, target| {
                if let Some(pool) = pool_of(m, target) {
                    policy::open(m, pool, Kind::Timeout);
                }
            }),
        CommandSpec::new(HA_RESTART_PRIORITY.0, "Restart Priority", AREA)
            .local(TabId::Ha, 60)
            .local_scopes(&[ObjectKind::Pool])
            .eligibility(can_protect)
            .invoke(protect),
        CommandSpec::new(CHANGE_SERVER_PASSWORD.0, "Change Server Password", AREA)
            .menu(place::POOL_CHANGE_SERVER_PASSWORD)
            .context(POOL)
            .eligibility(|m, target| can_change_password(m, pool_of(m, target)))
            .invoke(|m, target| {
                if let Some(pool) = pool_of(m, target) {
                    root_password::open(m, pool);
                }
            }),
        CommandSpec::new(MAKE_STANDALONE.0, "Make into Standalone Server", AREA)
            .menu(place::POOL_MAKE_STANDALONE)
            .context(POOL)
            .eligibility(can_make_standalone)
            .invoke(|m, target| {
                if let Some(pool) = pool_of(m, target) {
                    dissolve::open(m, pool);
                }
            }),
        // The same change from a server: it is the pool's root password.
        CommandSpec::new(SERVER_CHANGE_PASSWORD.0, "Change Password", AREA)
            .menu(place::SERVER_PASSWORD_CHANGE)
            .context(SERVER)
            .eligibility(|m, target| can_change_password(m, server(target).map(|(pool, _)| pool)))
            .invoke(|m, target| {
                if let Some((pool, _)) = server(target) {
                    root_password::open(m, pool);
                }
            }),
        CommandSpec::new(DESIGNATE_COORDINATOR.0, "Hand Over Coordinator", AREA)
            .menu(SERVER_DESIGNATE_COORDINATOR)
            .context(SERVER)
            .eligibility(can_designate)
            .invoke(designate),
        CommandSpec::new(REMOVE_FROM_POOL.0, "Remove from Pool", AREA)
            .menu(place::SERVER_REMOVE_FROM_POOL)
            .context(SERVER)
            .eligibility(|m, target| match member(m, target) {
                Some((inventory, _, host)) => {
                    Eligibility::enabled_unless(policy::member_refusal(&inventory, host, false))
                }
                None => Eligibility::Hidden,
            })
            .invoke(eject),
    ]
}

// -- Targets ------------------------------------------------------------------------

/// The pool a pool command acts on: the pool that is the target, or the
/// pool whose HA tab is showing when the target is a VM chosen in its grid.
pub fn pool_of(m: &Model, target: &Target) -> Option<PoolId> {
    if let Some(object) = target.single_of(ObjectKind::Pool) {
        return object.pool;
    }
    let scope = m.scope().single_of(ObjectKind::Pool)?;
    let vm = target.single_of(ObjectKind::Vm)?;
    (m.active_tab() == TabId::Ha && vm.pool == scope.pool)
        .then_some(scope.pool)
        .flatten()
}

/// The server a server command acts on, with its pool.
fn server(target: &Target) -> Option<(PoolId, HostId)> {
    let object = target.single_of(ObjectKind::Host)?;
    Some((object.pool?, object.host_id()?))
}

/// A server that its pool's inventory has.
fn member(m: &Model, target: &Target) -> Option<(std::sync::Arc<Inventory>, PoolId, HostId)> {
    let (pool, host) = server(target)?;
    let inventory = m.inventory_for(pool)?;
    inventory
        .hosts
        .contains_key(&host)
        .then_some((inventory, pool, host))
}

fn pool_or_server(m: &Model, target: &Target) -> Option<PoolId> {
    pool_of(m, target).or_else(|| server(target).map(|(pool, _)| pool))
}

/// The eligibility of an HA command from the pool's inventory.
fn ha(m: &Model, target: &Target, rule: impl Fn(&Inventory) -> Eligibility) -> Eligibility {
    match pool_of(m, target).and_then(|pool| m.inventory_for(pool)) {
        Some(inventory) => rule(&inventory),
        None => Eligibility::Hidden,
    }
}

/// The pool's observation can be taken now: what the Rune pool editor's
/// `current` came to when it had just opened.
fn observable(m: &Model, pool: PoolId) -> bool {
    m.snapshot_for(pool)
        .is_some_and(|s| s.observe_resource(ResourceTarget::Pool(pool)).is_ok())
}

// -- High availability --------------------------------------------------------------

/// Rune showed the heartbeat picker only with HA known to be off, and its
/// review button only with a known failure tolerance.
fn can_configure(i: &Inventory) -> Eligibility {
    let a = policy::administration(i);
    match a.ha_enabled {
        Some(true) => Eligibility::disabled(HA_ON),
        None => Eligibility::disabled(HA_UNKNOWN),
        Some(false) => {
            Eligibility::enabled_if(a.ha_failures_to_tolerate.is_some(), FAILURES_UNKNOWN)
        }
    }
}

fn can_disable(i: &Inventory) -> Eligibility {
    match policy::administration(i).ha_enabled {
        Some(true) => Eligibility::Enabled,
        Some(false) => Eligibility::disabled(HA_OFF),
        None => Eligibility::disabled(HA_UNKNOWN),
    }
}

fn overcommit_label(m: &Model, target: &Target) -> Option<String> {
    let inventory = m.inventory_for(pool_of(m, target)?)?;
    let allowed = policy::administration(&inventory).ha_allow_overcommit?;
    Some(if allowed {
        "Disallow HA overcommit…".to_owned()
    } else {
        "Allow HA overcommit…".to_owned()
    })
}

fn overcommit(m: &mut Model, target: &Target) {
    let Some(pool) = pool_of(m, target) else {
        return;
    };
    let expected = m
        .inventory_for(pool)
        .and_then(|i| policy::administration(&i).ha_allow_overcommit);
    if let Some(expected) = expected {
        let op = PoolOperation::SetHaAllowOvercommit {
            expected,
            enabled: !expected,
        };
        policy::open_review(m, pool, Kind::Overcommit, op);
    }
}

/// The VM of the HA tab's grid the command is for.
fn protected(m: &Model, target: &Target) -> Option<(PoolId, VmId)> {
    let object = target.single_of(ObjectKind::Vm)?;
    let pool = object.pool?;
    let vm = object.vm_id()?;
    m.inventory_for(pool)?
        .guests()
        .any(|guest| guest.id == vm)
        .then_some((pool, vm))
}

fn can_protect(m: &Model, target: &Target) -> Eligibility {
    if protected(m, target).is_some() {
        Eligibility::Enabled
    } else if target.single_of(ObjectKind::Pool).is_some() {
        Eligibility::disabled(SELECT_VM)
    } else {
        Eligibility::Hidden
    }
}

fn protect(m: &mut Model, target: &Target) {
    if let Some((pool, vm)) = protected(m, target)
        && policy::open(m, pool, Kind::Protection)
    {
        policy::select_vm(m, vm);
    }
}

// -- Membership ---------------------------------------------------------------------

fn can_designate(m: &Model, target: &Target) -> Eligibility {
    let Some((inventory, _, host)) = member(m, target) else {
        return Eligibility::Hidden;
    };
    if policy::administration(&inventory).coordinator.is_none() {
        return Eligibility::disabled(NO_COORDINATOR);
    }
    Eligibility::enabled_unless(policy::member_refusal(&inventory, host, true))
}

fn designate(m: &mut Model, target: &Target) {
    let Some((inventory, pool, host)) = member(m, target) else {
        return;
    };
    if let Some(expected) = policy::administration(&inventory).coordinator {
        let op = PoolOperation::DesignateCoordinator { expected, host };
        policy::open_review(m, pool, Kind::Handover, op);
        if let Some(editor) = &mut m.areas.pool.editor {
            editor.host = Some(host);
        }
    }
}

fn eject(m: &mut Model, target: &Target) {
    let Some((inventory, pool, host)) = member(m, target) else {
        return;
    };
    let Some(row) = inventory.hosts.get(&host) else {
        return;
    };
    let op = PoolOperation::EjectHost {
        host,
        expected_name: row.name.clone(),
    };
    policy::open_review(m, pool, Kind::Eject, op);
    if let Some(editor) = &mut m.areas.pool.editor {
        editor.host = Some(host);
    }
}

/// Pool > Remove Server: for a server it is Remove from Pool; for the pool
/// the member is chosen in the dialog.
fn can_remove_server(m: &Model, target: &Target) -> Eligibility {
    if let Some((inventory, _, host)) = member(m, target) {
        return Eligibility::enabled_unless(policy::member_refusal(&inventory, host, false));
    }
    ha(m, target, |i| {
        if i.hosts.len() < 2 {
            Eligibility::disabled(NO_MEMBER)
        } else {
            Eligibility::enabled_if(
                policy::administration(i).ha_enabled == Some(false),
                HA_FIRST,
            )
        }
    })
}

fn remove_server(m: &mut Model, target: &Target) {
    if member(m, target).is_some() {
        eject(m, target);
    } else if let Some(pool) = pool_of(m, target) {
        policy::open(m, pool, Kind::Eject);
    }
}

// -- Dissolution, join, root password -----------------------------------------------

/// Rune disabled "Dissolve pool…" while HA was not known to be off and
/// while the pool could not be observed; the capture's own refusal went to
/// the error line after the press.
fn can_make_standalone(m: &Model, target: &Target) -> Eligibility {
    let Some(pool) = pool_of(m, target) else {
        return Eligibility::Hidden;
    };
    let Some(inventory) = m.inventory_for(pool) else {
        return Eligibility::Hidden;
    };
    if policy::administration(&inventory).ha_enabled != Some(false) {
        return Eligibility::disabled(HA_FIRST);
    }
    if let Err(reason) = dissolve::capture(&inventory) {
        return Eligibility::disabled(reason);
    }
    Eligibility::enabled_if(
        m.is_synthetic_pool(pool) || observable(m, pool),
        NOT_WRITABLE,
    )
}

/// Rune disabled "Join another pool…" in the demo, without a writable
/// connection and for a pool of several servers.
fn can_join(m: &Model, target: &Target) -> Eligibility {
    let Some(pool) = pool_or_server(m, target) else {
        return Eligibility::Hidden;
    };
    let Some(inventory) = m.inventory_for(pool) else {
        return Eligibility::Hidden;
    };
    if m.is_synthetic_pool(pool) {
        return Eligibility::disabled(DEMO);
    }
    if inventory.hosts.len() != 1 {
        return Eligibility::disabled(NOT_STANDALONE);
    }
    if !m.can_mutate_pool(pool) {
        return Eligibility::disabled(join::NO_SOURCE);
    }
    Eligibility::enabled_unless(join::capture(m, pool).err())
}

/// Rune disabled "Change pool root password…" without a writable
/// connection; what its `open` refused after the press is the reason here.
fn can_change_password(m: &Model, pool: Option<PoolId>) -> Eligibility {
    let Some(pool) = pool else {
        return Eligibility::Hidden;
    };
    if m.inventory_for(pool).is_none() {
        return Eligibility::Hidden;
    }
    if m.is_synthetic_pool(pool) {
        return Eligibility::disabled(DEMO);
    }
    Eligibility::enabled_unless(root_password::capture(m, pool).err())
}
