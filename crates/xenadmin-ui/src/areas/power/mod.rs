//! Area `power`: VM power, singly and as a batch, placement on a chosen
//! server, and the recovery reviews.
//!
//! Rune modules: the power part of `main.rs` (`request_power`,
//! `submit_power`, the `force_action` and `unknown_review` dialogs, the
//! "Operations" block of the details pane), `bulk_power.rs`,
//! `vm_placement.rs`, `vm_recovery_view.rs`. Tabs: none.
//!
//! In Rune the nine power buttons stood in the details pane, always drawn,
//! each with the reason it was disabled; "Recovery controls…" opened a panel
//! that listed tasks and operations and offered four kinds of review; the
//! batch was started from a selection bar above the table. All of that is
//! commands now, and what a command opens is a dialog with its review.
//!
//! ## Commands
//!
//! | Command | Label | Menu | Also | Rune |
//! | --- | --- | --- | --- | --- |
//! | `vm.start` | Start | VM > Start/Shut Down | toolbar, context | `power-Start` |
//! | `vm.shutdown` | Shut Down | VM > Start/Shut Down | toolbar, context | `power-CleanShutdown` |
//! | `vm.reboot` | Reboot | VM > Start/Shut Down | toolbar, context | `power-Reboot` |
//! | `vm.suspend` | Suspend | VM > Start/Shut Down | toolbar, context | `power-Suspend` |
//! | `vm.resume` | Resume | VM > Start/Shut Down | toolbar, context | `power-Resume` |
//! | `vm.pause` | Pause | VM > Start/Shut Down | context | `power-Pause` |
//! | `vm.unpause` | Unpause | VM > Start/Shut Down | context | `power-Unpause` |
//! | `vm.force-shutdown` | Force Shut Down | VM > Start/Shut Down | toolbar, context | `power-HardShutdown`, `recovery-force-shutdown` |
//! | `vm.force-reboot` | Force Reboot | VM > Start/Shut Down | toolbar, context | `power-HardReboot`, `recovery-force-reboot` |
//! | `vm.recovery-start` | Start in Recovery Mode | VM > Start/Shut Down | | `recovery-boot` |
//! | `vm.recovery-restore-boot` | Restore Saved Boot Order | VM > Start/Shut Down | | `recovery-restore-<operation>` |
//! | `vm.start-on-server` | Start on Server | VM | | "Start on host…" |
//! | `vm.resume-on-server` | Resume on Server | VM | | "Resume on host…" |
//! | `vm.power-selection` | Power actions for selection… | | context of several VMs | `batch-power-open`, the row menu item |
//! | `vm.check-availability` | Check action availability | | General tab's action row, context | the button of that name |
//!
//! The nine power commands accept several VMs: on more than one they open
//! the batch on its review with that action chosen. On one VM a routine
//! action is submitted at once unless the option "Review routine VM power
//! actions" is on; a force action is always reviewed. The toolbar shows a
//! power command only while it is enabled (the kernel's rule for the power
//! group). The plan's toolbar line has seven of them, the slots the kernel
//! defines; Pause and Unpause are in the menu only (decision 2).
//!
//! ## Surfaces
//!
//! | Id | Kind | Title | Notice | Rune |
//! | --- | --- | --- | --- | --- |
//! | `power-confirm` | dialog, small | Confirm force operation / Confirm power operation | | `force_action` |
//! | `unknown-outcome` | dialog, small | Acknowledge unknown outcome | | `unknown_review` |
//! | `vm-placement` | dialog | Choose a destination host | experimental | `vm_placement::dialog` |
//! | `vm-recovery` | dialog | VM recovery controls | experimental | `vm_recovery_view::dialog` |
//! | `power-batch` | dialog, large | VM power batch | | `bulk_power::dialog` |
//!
//! Back to front they are in that order with placement behind recovery: the
//! precedence of the Rune dialog chain, the batch being a layer above it.
//!
//! ## Scenes
//!
//! `batch-power`, `vm-recovery`, `vm-recovery-force`. `unavailable-actions`
//! is the kernel's (`demo.rs`): what it showed of this area is the reason on
//! each disabled power command.
//!
//! ## For other areas
//!
//! The General tab of a VM (area `general`) and the Events tab (area
//! `events`) list operation records; the controls under a record are here:
//!
//! - [`operation_controls`]: the buttons Rune's details pane put under a
//!   record ("Request cancellation" or "Stop remaining steps", "Recheck
//!   outcome", "Review unknown outcome"), with the Rune conditions, and
//!   "Restore saved boot order" under the record of a finished recovery
//!   boot. Append them to the record's `outcome(..).actions(..)`.
//! - [`request_cancellation`], [`recheck`], [`review_unknown`],
//!   [`review_cancellation`], [`review_restore_boot`]: what those buttons
//!   do, for a caller that draws its own.
//! - [`cancellation_review_button`]: "Review cancellation request…", the
//!   reviewed cancellation of the Rune recovery panel, bound to the
//!   record's revision.
//! - [`availability_section`]: the group "Power" for a VM's General tab:
//!   the read-only, stale and no-actions captions, the session permission
//!   line and, for each of the nine actions, why it is or is not available,
//!   including the host's answers to "Check action availability".
//! - [`host_tasks`]: the lines of "Current host tasks" for a VM.
//! - [`control`]: `Model::control_operation` through the connection of the
//!   record's pool, which may be another than the scope session's.
//!
//! ## What of Rune has no successor here, and where it went
//!
//! - The nine always-drawn power buttons with a caption under each: the
//!   commands above; the caption is the reason of the disabled command
//!   (menu item, tree dump) and a line of [`availability_section`].
//! - The overview of "VM recovery controls" (host tasks, this client's
//!   unfinished operations, a button per review) and its "Refresh captured
//!   state" and "Back": every review is opened by its command or by a
//!   control under an operation record and captures the inventory then; the
//!   captured host tasks are listed in the review itself; the unfinished
//!   operations are the Operations section of the General tab.
//! - The selection bar above the table with its count caption and
//!   "Power selection…": the power commands on a multi-selection and
//!   `vm.power-selection`. The limit of 32 is the disabled reason.
//! - `power-actions-unavailable`, `power-actions-stale` and
//!   `session-permission-explanation`: lines of [`availability_section`].
//!
//! ## Decisions
//!
//! - A power command reads the pool of its target, not the scope session:
//!   [`track_vm`] submits through the target pool's connection.
//! - Force Shut Down and Force Reboot are one command each for two Rune
//!   paths. The ordinary confirmation is used when the action is allowed;
//!   the task-aware review of the recovery panel is used when it is not and
//!   the VM's current tasks are known, and also whenever one of those tasks
//!   is a migration, so that the review with the migration acceptance (lab
//!   finding 63) cannot be bypassed through the ordinary confirmation.
//! - The confirmation is disabled, with the reason, once its captured
//!   observation is stale or the permission is known to be denied. Rune let
//!   the press through and showed the core's refusal.
//! - Every surface closes when the tree scope or the scope session changes.
//!   Rune kept the recovery panel and relied on its "captured connection
//!   changed" state, which a grid selection change still shows.
//!
//! The files, in the order to read them: [`single`] (one VM: reasons,
//! request, confirmation, unknown outcome, operation controls), [`batch`],
//! [`placement`], [`recovery`] (each carried over from its Rune module),
//! [`commands`], [`view`], [`scenes`], `tests`.

use std::sync::Arc;

use xenadmin_core::{Error, Inventory, OperationId, PoolId, Vm, VmCommand, VmId};

use crate::prelude::*;
use crate::session;

pub mod batch;
pub mod commands;
pub mod placement;
pub mod recovery;
pub mod scenes;
pub mod single;
pub mod view;

#[cfg(test)]
mod live_tests;
#[cfg(test)]
mod tests;

pub use recovery::{
    cancellation_review_button, host_tasks, review_cancellation, review_restore_boot,
};
pub use single::{
    availability_section, control, operation_controls, recheck, request_cancellation,
    review_unknown,
};

/// The reason a synthetic pool gives (the text every area uses).
pub const DEMO: &str = "Demo mode cannot change infrastructure.";
/// The read-only caption of the Rune details pane.
pub const READ_ONLY: &str = "Read-only connection: changes and interactive consoles are disabled.";
/// The stale caption of the Rune details pane (`power-actions-stale`).
pub const NOT_CURRENT: &str =
    "The connection is not current. Power actions will stay disabled until it reconnects.";
/// What the Rune details pane said of a selected VM the inventory lost.
pub const GONE: &str = "This VM is no longer in the inventory. Select it again after reconnecting.";

/// The state of the area. `m.areas.power`. Each `Some` is an open surface.
#[derive(Default)]
pub struct State {
    /// The pending confirmation of one power action (Rune `force_action`).
    pub confirm: Option<single::Confirm>,
    /// The pending acknowledgement of an unknown outcome (Rune
    /// `unknown_review`).
    pub unknown: Option<single::Unknown>,
    /// Start or Resume on a chosen server (Rune `placement`).
    pub placement: Option<placement::Placement>,
    /// A recovery review (Rune `vm_recovery`).
    pub recovery: Option<Box<recovery::Panel>>,
    /// The power batch (Rune `bulk_power`).
    pub batch: Option<batch::Editor>,
}

pub fn spec() -> AreaSpec {
    AreaSpec {
        commands: commands::commands,
        scenes: scenes::scenes,
        surfaces: view::surfaces,
        on_scope_change: close,
        on_session_change: close,
        ..AreaSpec::new("power")
    }
}

/// Another object selected in the tree, another pool shown: every surface
/// here belongs to what it captured, so nothing of it is kept.
pub(crate) fn close(m: &mut Model) {
    m.areas.power = State::default();
}

/// Whether the pool is shown from a synthetic inventory (the demo).
pub(crate) fn synthetic(m: &Model, pool: PoolId) -> bool {
    match session::session_for_pool(m, pool) {
        Some(id) if m.workspace.selected == Some(id) => m.demo,
        Some(id) => m.workspace.synthetic.contains_key(&id),
        None => false,
    }
}

/// Why nothing can be changed in a pool that is neither synthetic nor
/// writable and current: the read-only caption or the stale one.
pub(crate) fn not_writable(m: &Model, pool: PoolId) -> &'static str {
    if m.connection_for(pool).is_some_and(|c| c.is_read_only()) {
        READ_ONLY
    } else {
        NOT_CURRENT
    }
}

/// One VM of a target that its pool's inventory has as an ordinary guest.
pub(crate) struct Guest {
    pub pool: PoolId,
    pub inventory: Arc<Inventory>,
    pub id: VmId,
}

impl Guest {
    /// The guest a command acts on. `Err` is what the command is instead:
    /// hidden for anything but one VM object (and for a template or a
    /// control domain, which Rune gave no power block), disabled with
    /// [`GONE`] for a VM the inventory no longer has.
    pub fn of(m: &Model, target: &Target) -> Result<Self, Eligibility> {
        let object = target
            .single_of(ObjectKind::Vm)
            .ok_or(Eligibility::Hidden)?;
        Self::of_object(m, object)
    }

    pub fn of_object(m: &Model, object: &ObjectRef) -> Result<Self, Eligibility> {
        if object.kind != ObjectKind::Vm {
            return Err(Eligibility::Hidden);
        }
        let (Some(pool), Some(id)) = (object.pool, object.vm_id()) else {
            return Err(Eligibility::Hidden);
        };
        let gone = || Eligibility::disabled(GONE);
        let inventory = m.inventory_for(pool).ok_or_else(gone)?;
        let vm = inventory.vms.get(&id).ok_or_else(gone)?;
        if vm.is_template || vm.is_control_domain {
            return Err(Eligibility::Hidden);
        }
        Ok(Self {
            pool,
            inventory,
            id,
        })
    }

    pub fn vm(&self) -> &Vm {
        &self.inventory.vms[&self.id]
    }

    pub fn object(&self) -> ObjectRef {
        ObjectRef::vm(self.pool, self.id)
    }
}

/// Submits a VM command through the session of the VM's pool, which keeps
/// its handle: `Model::submit_tracked_command_for` (synthetic: `Stale`; 64
/// unresolved operations of that session: `Busy`; no connection: `Closed`).
pub(crate) fn track_vm(
    m: &mut Model,
    pool: PoolId,
    command: VmCommand,
) -> Result<OperationId, Error> {
    m.submit_tracked_command_for(pool, command)
}

/// [`track_vm`] with the result in `m.error`, which a success clears (the
/// Rune `submit_command`).
pub(crate) fn submit_vm(m: &mut Model, pool: PoolId, command: VmCommand) {
    m.error = track_vm(m, pool, command).err().map(|e| e.to_string());
}

/// The submit-time permission gate for a pool: a known denial puts its
/// explanation in `m.error` and returns false (the Rune
/// `session_permissions_view::check`).
pub(crate) fn permitted(
    m: &mut Model,
    pool: PoolId,
    requirements: &xenadmin_core::PermissionRequirements,
) -> bool {
    permissions::check_for(m, pool, requirements)
}
