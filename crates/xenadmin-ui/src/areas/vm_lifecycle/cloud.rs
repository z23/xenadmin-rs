//! The cloud-init inputs of a creation and what follows its submission:
//! the logic of the Rune `vm_creation_cloud.rs`.
//!
//! The inputs stay in the wizard ([`Draft`], zeroized when dropped) until
//! the creation succeeded and the exact new VM is verified ([`ready`]);
//! then they are handed to the cloud-init drive dialog of that VM
//! ([`continue_to_drive`]). Closing the continuation discards the draft; it
//! does not cancel the creation.

use xenadmin_core::{
    CloudInitInputs, ObservedVm, OperationAction, OperationId, OperationRecord, OperationState,
    PoolId, PowerState, SrId, VmId, VmLifecycleOperation,
};
use zeroize::Zeroize;

use super::{Panel, drive, edit};
use crate::prelude::*;

pub const INTRODUCTION: &str = "The template must already support cloud-init with the NoCloud datasource. This configuration stays in this wizard until the new VM is verified. It is not saved with the connection profile or creation history.";
pub const CONSEQUENCES: &str = "The new VM stays halted. After successful creation, review the exact new VM, storage and unused disk slot before uploading its configuration drive. Closing the wizard discards these unsaved inputs without cancelling host work.";
pub const ENABLE_HELP: &str = "For templates with cloud-init and NoCloud support installed. After successful VM creation, review a private configuration drive for that exact new VM. The VM stays halted.";
pub const VERIFIED: &str =
    "The new VM is verified and halted. Review its configuration drive before uploading.";
pub const PRIVATE: &str = "Configuration is still private and has not been uploaded. Closing discards this draft; it does not cancel creation or delete the retained VM. VM changes keeps the creation outcome, and cloud-init can be opened from the VM later.";

/// The cloud-init inputs of a creation. User-data and network-config can
/// hold passwords and keys.
pub struct Draft {
    pub hostname: String,
    pub user_data: String,
    pub network: String,
}

impl Default for Draft {
    fn default() -> Self {
        Self {
            hostname: String::new(),
            user_data: "#cloud-config\n".into(),
            network: String::new(),
        }
    }
}

impl Drop for Draft {
    fn drop(&mut self) {
        self.user_data.zeroize();
        self.network.zeroize();
    }
}

impl Draft {
    pub fn validate(&self) -> Result<(), String> {
        CloudInitInputs::validate(
            self.hostname.trim(),
            &self.user_data,
            (!self.network.trim().is_empty()).then_some(self.network.as_str()),
        )
        .map_err(|e| e.to_string())
    }
}

/// A submitted creation whose cloud-init inputs wait for the new VM.
pub struct Pending {
    pub id: OperationId,
    pub pool: PoolId,
    pub source: VmId,
    /// `None` only in a synthetic pool (the scene).
    pub target: Option<ObservedVm>,
    pub operation: VmLifecycleOperation,
    pub sr: SrId,
    pub draft: Draft,
}

pub(crate) fn pending(m: &Model) -> Option<&Pending> {
    match m.areas.vm_lifecycle.panel.as_ref()? {
        Panel::CloudCreation(p) => Some(p),
        Panel::Editor(_) => None,
    }
}

/// The record of the creation: the live handle's latest, else the row kept
/// for it.
pub(crate) fn record(m: &Model, p: &Pending) -> Option<OperationRecord> {
    m.operations
        .iter()
        .find(|o| o.latest().id == p.id)
        .map(|o| (*o.latest()).clone())
        .or_else(|| m.operation_rows.iter().find(|r| r.id == p.id).cloned())
}

/// The VM a record says was created, when the record is exactly this
/// creation and it succeeded.
pub(crate) fn completed_vm(p: &Pending, r: &OperationRecord) -> Result<VmId, String> {
    if r.id != p.id
        || r.pool != p.pool
        || r.vm != p.source
        || r.action != OperationAction::Lifecycle(p.operation.clone())
    {
        return Err("Creation identity does not match this wizard. Inspect VM changes.".into());
    }
    if r.state != OperationState::Succeeded {
        return Err(format!(
            "Creation is {:?}. Configuration has not been uploaded. Resolve the creation operation in VM changes before configuring the retained VM separately.",
            r.state
        ));
    }
    r.created_vm
        .filter(|vm| *vm != p.source)
        .ok_or_else(|| "The successful creation has no verified new VM identity.".into())
}

/// The created VM, when the configuration drive can be reviewed for it
/// now; else what is waited for. Never reads the selection.
pub(crate) fn ready(m: &Model, p: &Pending) -> Result<VmId, String> {
    let r = record(m, p).ok_or("Waiting for the creation operation to appear.")?;
    let vm = completed_vm(p, &r)?;
    let inventory = m
        .inventory_for(p.pool)
        .filter(|i| i.pool_id == p.pool)
        .ok_or("Select the original pool to inspect the created VM.")?;
    if !m.is_synthetic_pool(p.pool)
        && (p.target.is_none()
            || m.snapshot_for(p.pool)
                .and_then(|snap| snap.observe_vm(p.source).ok())
                != p.target)
    {
        return Err("The connection generation changed. Open cloud-init from the verified created VM to review it again; this wizard will not continue across reconnects.".into());
    }
    let guest = inventory
        .vms
        .get(&vm)
        .filter(|v| !v.is_template && !v.is_control_domain && v.power_state == PowerState::Halted)
        .ok_or("Waiting for the exact new halted VM in current inventory. A running, removed or converted VM cannot continue here.")?;
    if inventory
        .details
        .vms
        .get(&guest.id)
        .is_none_or(|v| !v.current_operations.is_empty())
    {
        return Err("Waiting for current VM details with no active operations.".into());
    }
    if inventory
        .details
        .storage
        .get(&p.sr)
        .is_none_or(|sr| sr.content_type != "user")
    {
        return Err("The selected configuration-drive storage is no longer available.".into());
    }
    Ok(vm)
}

/// Why "Review configuration drive" is disabled.
pub(crate) fn blocked(m: &Model, p: &Pending) -> Option<String> {
    if m.is_synthetic_pool(p.pool) {
        return Some(edit::DEMO.into());
    }
    if !m.can_mutate_pool(p.pool) {
        return Some(drive::READ_ONLY.into());
    }
    ready(m, p).err()
}

/// Hands the draft to the cloud-init drive dialog of the created VM, which
/// prepares its private media at once. Refused in a synthetic pool,
/// without a writable connection, before the VM is ready and for an
/// invalid draft; then nothing changes.
pub(crate) fn continue_to_drive(m: &mut Model) {
    let Some(p) = pending(m) else { return };
    if m.is_synthetic_pool(p.pool) || !m.can_mutate_pool(p.pool) {
        return;
    }
    let Ok(vm) = ready(m, p) else { return };
    if p.draft.validate().is_err() {
        return;
    }
    let Some(Panel::CloudCreation(mut p)) = m.areas.vm_lifecycle.panel.take() else {
        return;
    };
    drive::open_created(
        m,
        p.pool,
        vm,
        p.sr,
        std::mem::take(&mut p.draft.hostname),
        std::mem::take(&mut p.draft.user_data),
        std::mem::take(&mut p.draft.network),
    );
}
