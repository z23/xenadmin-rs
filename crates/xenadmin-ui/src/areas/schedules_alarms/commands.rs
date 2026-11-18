//! The commands of the area: the two Rune buttons of the top control row
//! ("Snapshot schedules", "Alarms") and what the new menus add to them.
//!
//! - `pool.snapshot-schedules`, "VM Snapshot Schedule Policies" (Pool menu,
//!   a pool's context menu): the Rune button `snapshot-schedules-open`. It
//!   acts on the pool of its target, whatever object of that pool is
//!   selected, and is disabled while that pool has no inventory (the Rune
//!   condition; Rune gave no reason).
//! - `vm.assign-snapshot-schedule`, "Assign to Snapshot Schedule" (VM menu,
//!   a VM's context menu): the same dialog, opened for one guest VM. Hidden
//!   for anything that is not exactly one VM that can be a member (not a
//!   template, not a control domain).
//! - `tools.alarm-service-refresh`, "Refresh Alarm Service" (Tools menu, a
//!   pool's context menu): the third tab of the Rune "Alarms" dialog, which
//!   has no page in the plan's Properties tables. Disabled with the Rune
//!   refusal when the pool has no server or more than 64.
//!
//! The Rune button `alarms-open` has no command: its first two tabs are
//! pages of the kernel's Properties dialogs (`alarms_view::pages`).

use xenadmin_core::{PoolId, VmId};

use super::alarms;
use crate::prelude::*;

const AREA: &str = "schedules_alarms";

pub const SNAPSHOT_SCHEDULES: CommandId = CommandId("pool.snapshot-schedules");
pub const ASSIGN_SNAPSHOT_SCHEDULE: CommandId = CommandId("vm.assign-snapshot-schedule");
pub const ALARM_SERVICE_REFRESH: CommandId = CommandId("tools.alarm-service-refresh");

/// The reason of a command whose target is in no pool the client has an
/// inventory of (Rune disabled its buttons without one).
pub const NO_POOL: &str = "Select a connected pool, or an object of one.";

/// After the two report items of the Tools menu.
const TOOLS_ALARM_SERVICE_REFRESH: MenuPath = MenuPath::new(Menu::Tools, 3, 30);

pub fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new(
            SNAPSHOT_SCHEDULES.0,
            "VM Snapshot Schedule Policies",
            AREA,
        )
        .menu(place::POOL_SNAPSHOT_SCHEDULES)
        .context(&[ObjectKind::Pool])
        .eligibility(|m, target| Eligibility::enabled_if(pool(m, target).is_some(), NO_POOL))
        .invoke(|m, target| {
            if let Some(pool) = pool(m, target) {
                super::open_schedules(m, pool, None);
            }
        }),
        CommandSpec::new(
            ASSIGN_SNAPSHOT_SCHEDULE.0,
            "Assign to Snapshot Schedule",
            AREA,
        )
        .menu(place::VM_ASSIGN_SNAPSHOT_SCHEDULE)
        .context(&[ObjectKind::Vm])
        .eligibility(|m, target| match guest(m, target) {
            Some(_) => Eligibility::Enabled,
            None => Eligibility::Hidden,
        })
        .invoke(|m, target| {
            if let Some((pool, vm)) = guest(m, target) {
                super::open_schedules(m, pool, Some(vm));
            }
        }),
        CommandSpec::new(ALARM_SERVICE_REFRESH.0, "Refresh Alarm Service", AREA)
            .menu(TOOLS_ALARM_SERVICE_REFRESH)
            .context(&[ObjectKind::Pool])
            .eligibility(|m, target| {
                match pool(m, target).and_then(|pool| alarms::refresh(m, pool)) {
                    Some(Ok(_)) => Eligibility::Enabled,
                    Some(Err(reason)) => Eligibility::disabled(reason),
                    None => Eligibility::disabled(NO_POOL),
                }
            })
            .invoke(|m, target| {
                if let Some(Ok(workspace)) =
                    pool(m, target).and_then(|pool| alarms::refresh(m, pool))
                {
                    super::close(m);
                    m.areas.schedules_alarms.alarms = Some(workspace);
                }
            }),
    ]
}

/// The pool of the target, when the client has its inventory.
fn pool(m: &Model, target: &Target) -> Option<PoolId> {
    target.pool().filter(|pool| m.inventory_for(*pool).is_some())
}

/// The one VM of the target, when it can be a member of a schedule.
fn guest(m: &Model, target: &Target) -> Option<(PoolId, VmId)> {
    let object = target.single_of(ObjectKind::Vm)?;
    let (pool, vm) = (object.pool?, object.vm_id()?);
    let inventory = m.inventory_for(pool)?;
    let record = inventory.vms.get(&vm)?;
    (!record.is_template && !record.is_control_domain).then_some((pool, vm))
}
