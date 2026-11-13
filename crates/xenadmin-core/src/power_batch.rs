use crate::{
    Error, PowerAction, PowerState, ResourceAction, ResourceOperationRecord, ResourceTarget, VmId,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use xenadmin_xapi::records::{PoolRecord, VmRecord};

/// One reviewed ordinary guest. UUIDs are scoped by the command's observed pool.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchVmPower {
    pub vm: VmId,
    pub expected_state: PowerState,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VmPowerBatch {
    pub action: PowerAction,
    pub vms: Vec<BatchVmPower>,
}
impl VmPowerBatch {
    pub(crate) fn validate(&self, target: ResourceTarget) -> Result<(), Error> {
        let mut seen = BTreeSet::new();
        if !matches!(target, ResourceTarget::Pool(_))
            || !(1..=32).contains(&self.vms.len())
            || self
                .vms
                .iter()
                .any(|item| !seen.insert(item.vm) || !compatible(self.action, &item.expected_state))
        {
            return Err(Error::InvalidResourceCommand);
        }
        Ok(())
    }
}
fn compatible(action: PowerAction, state: &PowerState) -> bool {
    use PowerAction::*;
    match action {
        Start => *state == PowerState::Halted,
        Resume => *state == PowerState::Suspended,
        Unpause => *state == PowerState::Paused,
        Pause | Suspend | CleanShutdown | Reboot => *state == PowerState::Running,
        HardShutdown => matches!(
            state,
            PowerState::Running | PowerState::Paused | PowerState::Suspended
        ),
        HardReboot => matches!(state, PowerState::Running | PowerState::Paused),
    }
}
fn expected_result(action: PowerAction) -> PowerState {
    match action {
        PowerAction::Pause => PowerState::Paused,
        PowerAction::Suspend => PowerState::Suspended,
        PowerAction::CleanShutdown | PowerAction::HardShutdown => PowerState::Halted,
        _ => PowerState::Running,
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Item {
    pub selected: BatchVmPower,
    pub dispatched: bool,
    pub task: Option<String>,
    pub succeeded: bool,
    pub verified: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Plan {
    pub selection: VmPowerBatch,
    pub items: Vec<Item>,
}
impl Plan {
    pub(crate) fn new(selection: VmPowerBatch) -> Self {
        let items = selection
            .vms
            .iter()
            .cloned()
            .map(|selected| Item {
                selected,
                dispatched: false,
                task: None,
                succeeded: false,
                verified: false,
            })
            .collect();
        Self { selection, items }
    }
    pub(crate) fn valid(&self, record: &ResourceOperationRecord) -> bool {
        if record.action != ResourceAction::PowerBatch(self.selection.clone())
            || self.selection.validate(record.target).is_err()
            || self.items.len() != self.selection.vms.len()
            || record.steps.len() != self.items.len()
            || record.task.is_some()
            || record.result_reference.is_some()
        {
            return false;
        }
        let uncertain = self.items.iter().any(|i| i.dispatched && !i.verified);
        if (record.state == crate::OperationState::Preparing && uncertain)
            || (record.state == crate::OperationState::Succeeded
                && self.items.iter().any(|i| !i.verified))
            || (record.state.terminal()
                && record.state != crate::OperationState::AcknowledgedUnknown
                && uncertain)
        {
            return false;
        }
        let mut previous_verified = true;
        for (item, selected) in self.items.iter().zip(&self.selection.vms) {
            if item.selected != *selected
                || (item.dispatched && !previous_verified)
                || (item.task.is_some() && !item.dispatched)
                || (item.succeeded && item.task.is_none())
                || (item.verified && !item.succeeded)
                || item.task.as_ref().is_some_and(|v| {
                    v.len() > 512
                        || !v.starts_with("OpaqueRef:")
                        || v == "OpaqueRef:NULL"
                        || v.chars().any(char::is_control)
                })
            {
                return false;
            }
            previous_verified = item.verified;
        }
        true
    }
    pub(crate) fn extends(&self, old: &Self) -> bool {
        self.selection == old.selection
            && self.items.len() == old.items.len()
            && self.items.iter().zip(&old.items).all(|(new, old)| {
                new.selected == old.selected
                    && (!old.dispatched || new.dispatched)
                    && (!old.succeeded || new.succeeded)
                    && (!old.verified || new.verified)
                    && (old.task.is_none() || new.task == old.task)
            })
    }
}
pub(crate) fn validate_record(record: &ResourceOperationRecord) -> Result<(), Error> {
    match (&record.action, &record.power_batch_plan) {
        (ResourceAction::PowerBatch(_), Some(plan)) if plan.valid(record) => Ok(()),
        (ResourceAction::PowerBatch(_), None)
            if record.state == crate::OperationState::Preparing && record.steps.is_empty() =>
        {
            Ok(())
        }
        (ResourceAction::PowerBatch(_), _) | (_, Some(_)) => Err(Error::Journal),
        _ => Ok(()),
    }
}
pub(crate) async fn pool(
    session: &crate::xapi::Session,
    record: &ResourceOperationRecord,
) -> Result<(), Error> {
    let reference = session
        .call("pool.get_by_uuid", vec![json!(record.pool.to_string())])
        .await?;
    let reference = crate::xapi::opaque_reference(&reference)?;
    let value = session
        .call("pool.get_record", vec![json!(reference)])
        .await?;
    if PoolRecord::new(&value).uuid() != record.pool.to_string() {
        return Err(Error::Stale);
    }
    Ok(())
}
async fn read(session: &crate::xapi::Session, vm: VmId) -> Result<(String, Value), Error> {
    let reference = session
        .call("VM.get_by_uuid", vec![json!(vm.to_string())])
        .await?;
    let reference = crate::xapi::opaque_reference(&reference)?;
    let value = session
        .call("VM.get_record", vec![json!(&reference)])
        .await?;
    if VmRecord::new(&value).uuid() != vm.to_string()
        || VmRecord::new(&value).is_a_template() != false
        || VmRecord::new(&value).is_a_snapshot() != false
        || VmRecord::new(&value).is_control_domain() != false
    {
        return Err(Error::Stale);
    }
    Ok((reference, value))
}
pub(crate) async fn prepare_item(
    session: &crate::xapi::Session,
    selected: &BatchVmPower,
    action: PowerAction,
) -> Result<Vec<Value>, Error> {
    let (reference, value) = read(session, selected.vm).await?;
    if VmRecord::new(&value).power_state() != selected.expected_state.to_string()
        || !crate::xapi::advertises(&value, action.wire())
        || !VmRecord::new(&value)
            .blocked_operations()
            .entries()
            .is_some()
        || !VmRecord::new(&value)
            .current_operations()
            .entries()
            .is_some_and(|v| v.is_empty())
    {
        return Err(Error::UnavailableAction);
    }
    session
        .call(
            "VM.assert_operation_valid",
            vec![json!(&reference), json!(action.wire())],
        )
        .await?;
    let mut args = vec![json!(reference)];
    if matches!(action, PowerAction::Start | PowerAction::Resume) {
        args.extend([json!(false), json!(false)]);
    }
    Ok(args)
}
pub(crate) async fn verify_item(
    session: &crate::xapi::Session,
    item: &Item,
    action: PowerAction,
) -> Result<(), Error> {
    if !item.succeeded || item.task.is_none() {
        return Err(Error::Protocol("Power task completion is not proven"));
    }
    let (_, value) = read(session, item.selected.vm).await?;
    if VmRecord::new(&value).power_state() != expected_result(action).to_string()
        || !VmRecord::new(&value)
            .current_operations()
            .entries()
            .is_some_and(|v| v.is_empty())
    {
        return Err(Error::Protocol(
            "VM state does not match the completed power task",
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "power_batch_tests.rs"]
mod tests;
