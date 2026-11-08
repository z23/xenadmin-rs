use crate::{
    Error, PowerState, ResourceAction, ResourceOperationRecord, ResourceTarget, VmId,
    VmSettingsPatch,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchVmProperties {
    pub vm: VmId,
    pub expected_state: PowerState,
    pub patch: VmSettingsPatch,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VmPropertiesBatch {
    pub vms: Vec<BatchVmProperties>,
}
impl VmPropertiesBatch {
    pub(crate) fn validate(&self, target: ResourceTarget) -> Result<(), Error> {
        let mut seen = BTreeSet::new();
        if !matches!(target, ResourceTarget::Pool(_)) || !(1..=32).contains(&self.vms.len()) {
            return Err(Error::InvalidResourceCommand);
        }
        for item in &self.vms {
            if !seen.insert(item.vm)
                || matches!(item.expected_state, PowerState::Unknown(_))
                || (item.patch.is_template() && item.expected_state != PowerState::Halted)
            {
                return Err(Error::InvalidResourceCommand);
            }
            item.patch.validate()?;
        }
        Ok(())
    }
    pub fn permission_requirements(&self) -> crate::PermissionRequirements {
        crate::PermissionRequirements::writes(self.vms.iter().flat_map(|item| {
            item.patch
                .permission_requirements()
                .methods()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        }))
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Receipt {
    pub dispatched: bool,
    pub succeeded: bool,
    pub verified: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Plan {
    pub selection: VmPropertiesBatch,
    pub receipts: Vec<Receipt>,
}
impl Plan {
    pub(crate) fn new(selection: VmPropertiesBatch) -> Self {
        let count = selection.vms.iter().map(|v| v.patch.steps().len()).sum();
        Self {
            selection,
            receipts: vec![
                Receipt {
                    dispatched: false,
                    succeeded: false,
                    verified: false
                };
                count
            ],
        }
    }
    pub(crate) fn step(
        &self,
        index: usize,
    ) -> Result<(&BatchVmProperties, crate::settings::SettingsStep), Error> {
        let mut offset = 0;
        for item in &self.selection.vms {
            let steps = item.patch.steps();
            if index < offset + steps.len() {
                return Ok((item, steps[index - offset].clone()));
            }
            offset += steps.len();
        }
        Err(Error::Journal)
    }
    pub(crate) fn valid(&self, record: &ResourceOperationRecord) -> bool {
        if record.action != ResourceAction::PropertiesBatch(self.selection.clone())
            || self.selection.validate(record.target).is_err()
            || self.receipts.len()
                != self
                    .selection
                    .vms
                    .iter()
                    .map(|v| v.patch.steps().len())
                    .sum::<usize>()
            || record.steps.len() != self.receipts.len()
            || record.task.is_some()
            || record.result_reference.is_some()
        {
            return false;
        }
        let uncertain = self.receipts.iter().any(|r| r.dispatched && !r.verified);
        if (record.state == crate::OperationState::Preparing && uncertain)
            || (record.state == crate::OperationState::Succeeded
                && self.receipts.iter().any(|r| !r.verified))
            || (record.state.terminal()
                && record.state != crate::OperationState::AcknowledgedUnknown
                && uncertain)
        {
            return false;
        }
        let mut previous_verified = true;
        for (index, receipt) in self.receipts.iter().enumerate() {
            let Ok((item, step)) = self.step(index) else {
                return false;
            };
            if record.steps[index].label != format!("VM {} · {}", item.vm, step.label)
                || (record.steps[index].state == crate::StepState::Verified) != receipt.verified
            {
                return false;
            }
            if (receipt.dispatched && !previous_verified)
                || (receipt.succeeded && !receipt.dispatched)
                || (receipt.verified && !receipt.succeeded)
            {
                return false;
            }
            previous_verified = receipt.verified;
        }
        true
    }
    pub(crate) fn extends(&self, old: &Self) -> bool {
        self.selection == old.selection
            && self.receipts.len() == old.receipts.len()
            && self.receipts.iter().zip(&old.receipts).all(|(new, old)| {
                (!old.dispatched || new.dispatched)
                    && (!old.succeeded || new.succeeded)
                    && (!old.verified || new.verified)
            })
    }
}
pub(crate) fn validate_record(record: &ResourceOperationRecord) -> Result<(), Error> {
    match (&record.action, &record.properties_batch_plan) {
        (ResourceAction::PropertiesBatch(_), Some(plan)) if plan.valid(record) => Ok(()),
        (ResourceAction::PropertiesBatch(_), None)
            if record.steps.is_empty()
                && matches!(
                    record.state,
                    crate::OperationState::Preparing
                        | crate::OperationState::Rejected
                        | crate::OperationState::Cancelled
                ) =>
        {
            Ok(())
        }
        (ResourceAction::PropertiesBatch(_), _) | (_, Some(_)) => Err(Error::Journal),
        _ => Ok(()),
    }
}
async fn read(
    session: &crate::xapi::Session,
    item: &BatchVmProperties,
) -> Result<(String, Value), Error> {
    let reference = session
        .call("VM.get_by_uuid", vec![json!(item.vm.to_string())])
        .await?;
    let reference = crate::xapi::opaque_reference(&reference)?;
    let value = session
        .call("VM.get_record", vec![json!(&reference)])
        .await?;
    if value["uuid"] != item.vm.to_string()
        || value["power_state"] != item.expected_state.to_string()
    {
        return Err(Error::Stale);
    }
    item.patch.target_guard(&value)?;
    Ok((reference, item.patch.resolve_record(session, value).await?))
}
pub(crate) async fn preflight(
    session: &crate::xapi::Session,
    item: &BatchVmProperties,
) -> Result<(), Error> {
    item.patch
        .permission_requirements()
        .guard(session.permissions())?;
    let (reference, value) = read(session, item).await?;
    item.patch.preflight(&value)?;
    item.patch.preflight_remote(session, &reference).await?;
    // Resolve every desired host and remote policy before any selected VM is changed.
    for step in item.patch.steps() {
        step.prepare_call(session, &reference, &value).await?;
    }
    Ok(())
}
pub(crate) async fn prepare(
    session: &crate::xapi::Session,
    plan: &Plan,
    index: usize,
) -> Result<(String, Vec<Value>), Error> {
    let (item, step) = plan.step(index)?;
    item.patch
        .permission_requirements()
        .guard(session.permissions())?;
    let (reference, value) = read(session, item).await?;
    step.preflight(&value)?;
    step.prepare_call(session, &reference, &value).await
}
pub(crate) async fn verify(
    session: &crate::xapi::Session,
    plan: &Plan,
    index: usize,
) -> Result<(), Error> {
    if !plan.receipts.get(index).is_some_and(|r| r.succeeded) {
        return Err(Error::Protocol(
            "Property setter has no durable success receipt; matching values cannot prove completion",
        ));
    }
    let (item, step) = plan.step(index)?;
    let (_, value) = read(session, item).await?;
    if !step.verified(&value) {
        return Err(Error::SettingsConflict);
    }
    Ok(())
}

#[cfg(test)]
#[path = "properties_batch_tests.rs"]
mod tests;
