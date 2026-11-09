use super::*;
use crate::{OperationStep, StepState, snapshots};

#[derive(Clone, Copy)]
enum DeleteStep {
    DiscardMemory,
    Snapshot,
    Disk(crate::VdiId),
}
impl DeleteStep {
    fn label(self) -> String {
        match self {
            Self::DiscardMemory => "Discard checkpoint memory".into(),
            Self::Snapshot => "Delete snapshot record".into(),
            Self::Disk(id) => format!("Delete reviewed snapshot disk {id}"),
        }
    }
    async fn verified(
        self,
        session: &Session,
        parent: VmId,
        plan: &snapshots::DeletionPlan,
    ) -> Result<bool, Error> {
        match self {
            Self::DiscardMemory => {
                if let Some(memory) = plan.memory
                    && snapshots::optional_record(session, "VDI", &memory.to_string())
                        .await?
                        .is_some()
                {
                    return Ok(false);
                }
                let record =
                    snapshots::optional_record(session, "VM", &plan.snapshot.to_string()).await?;
                if record.is_none() {
                    return Ok(true);
                }
                let (_, record) =
                    snapshots::snapshot_record(session, parent, plan.snapshot).await?;
                Ok(record["power_state"] == "Halted")
            }
            Self::Snapshot => {
                Ok(
                    snapshots::optional_record(session, "VM", &plan.snapshot.to_string())
                        .await?
                        .is_none(),
                )
            }
            Self::Disk(id) => Ok(snapshots::optional_record(session, "VDI", &id.to_string())
                .await?
                .is_none()),
        }
    }
    async fn send(
        self,
        session: &Session,
        parent: VmId,
        plan: &snapshots::DeletionPlan,
    ) -> Result<(), Error> {
        match self {
            Self::DiscardMemory => {
                let fresh =
                    snapshots::deletion_plan(session, parent, plan.snapshot, &plan.disks).await?;
                if fresh != *plan {
                    return Err(Error::SnapshotConflict);
                }
                snapshots::stop_checkpoint(session, parent, plan.snapshot, plan.memory).await
            }
            Self::Snapshot => {
                // Recheck the exact reviewed disks after any checkpoint state transition.
                snapshots::deletion_plan(session, parent, plan.snapshot, &plan.disks).await?;
                snapshots::destroy_snapshot(session, parent, plan.snapshot).await
            }
            Self::Disk(id) => snapshots::destroy_disk(session, id).await,
        }
    }
}
fn steps(plan: &snapshots::DeletionPlan) -> Vec<DeleteStep> {
    let mut result = Vec::new();
    if plan.checkpoint {
        result.push(DeleteStep::DiscardMemory);
    }
    result.push(DeleteStep::Snapshot);
    result.extend(plan.disks.iter().copied().map(DeleteStep::Disk));
    result
}

impl MutationRequest {
    pub(super) async fn execute_snapshot_delete(
        &mut self,
        session: &Session,
        journal: &mut Journal,
        cancel: &CancellationToken,
    ) -> Result<(), Error> {
        let OperationAction::Snapshot(SnapshotOperation::Delete {
            snapshot,
            ref disks,
        }) = self.record.action
        else {
            return Err(Error::UnavailableAction);
        };
        let plan = tokio::select! {_=cancel.cancelled()=>return Err(Error::Closed), result=snapshots::deletion_plan(session,self.record.vm,snapshot,disks)=>result?};
        SnapshotOperation::deletion_permission_requirements(disks, Some(plan.checkpoint))
            .guard(session.permissions())?;
        let plan_steps = steps(&plan);
        self.record.snapshot_deletion = Some(plan.clone());
        self.record.steps = plan_steps
            .iter()
            .map(|step| OperationStep {
                label: step.label(),
                state: StepState::Pending,
                detail: String::new(),
            })
            .collect();
        self.record.state = OperationState::Preparing;
        self.record.can_cancel = true;
        self.persist_steps(journal).await?;
        for (index, step) in plan_steps.into_iter().enumerate() {
            if cancel.is_cancelled() {
                return Err(Error::Closed);
            }
            if !self.begin_step(journal, index).await? {
                return Ok(());
            }
            let result = tokio::select! {_=cancel.cancelled()=>Err(Error::Closed), result=step.send(session,self.record.vm,&plan)=>result};
            if let Err(error) = result {
                let refused = matches!(
                    error,
                    Error::Api(_)
                        | Error::Coordinator(_)
                        | Error::SnapshotConflict
                        | Error::Stale
                        | Error::UnavailableAction
                );
                self.record.steps[index].state = if refused {
                    StepState::Failed
                } else {
                    StepState::OutcomeUnknown
                };
                self.record.steps[index].detail = error.to_string();
                self.record.state = if !refused {
                    OperationState::OutcomeUnknown
                } else if index == 0 {
                    OperationState::Failed
                } else {
                    OperationState::PartiallyCompleted
                };
                self.record.detail =
                    format!("{}; remaining deletion steps were not submitted", error);
                self.record.can_cancel = false;
                return self.persist_steps(journal).await;
            }
            self.record.state = OperationState::Verifying;
            self.persist_steps(journal).await?;
            if !step.verified(session, self.record.vm, &plan).await? {
                let detail=match step {
                    DeleteStep::DiscardMemory=>format!("Checkpoint memory cleanup was not verified for {}; remaining deletion steps were not submitted",plan.memory.map(|id|id.to_string()).unwrap_or_else(||"the saved image".into())),
                    DeleteStep::Snapshot=>"The snapshot record remains after deletion; no disk deletions were submitted".into(),
                    DeleteStep::Disk(id)=>format!("Snapshot disk {id} remains after deletion; further deletions were not submitted"),
                };
                self.record.steps[index].state = StepState::OutcomeUnknown;
                self.record.steps[index].detail = detail.clone();
                self.record.state = OperationState::OutcomeUnknown;
                self.record.detail = detail;
                self.record.can_cancel = false;
                return self.persist_steps(journal).await;
            }
            self.record.steps[index].state = StepState::Verified;
            self.record.steps[index].detail = "Result verified on host".into();
            self.record.state = OperationState::Preparing;
            self.record.progress = Some((index + 1) as f64 / self.record.steps.len() as f64);
            self.persist_steps(journal).await?;
        }
        self.record.state = OperationState::Succeeded;
        self.record.detail =
            "Snapshot and reviewed disks deleted; unselected disks retained".into();
        self.record.can_cancel = false;
        self.persist_steps(journal).await
    }

    pub(super) async fn recheck_snapshot_delete(
        &mut self,
        session: &Session,
        journal: &mut Journal,
    ) -> Result<(), Error> {
        let plan = self
            .record
            .snapshot_deletion
            .clone()
            .ok_or(Error::UnavailableAction)?;
        let mut complete = true;
        for (index, step) in steps(&plan).into_iter().enumerate() {
            if step.verified(session, self.record.vm, &plan).await? {
                self.record.steps[index].state = StepState::Verified;
                self.record.steps[index].detail =
                    "Deletion state observed during read-only reconciliation".into();
            } else {
                complete = false;
            }
        }
        self.record.state = if complete {
            OperationState::Succeeded
        } else {
            OperationState::OutcomeUnknown
        };
        self.record.detail = if complete {
            "Snapshot deletion state verified; no writes were repeated"
        } else {
            "Some reviewed resources remain or are uncertain; no deletion steps were repeated"
        }
        .into();
        self.record.can_cancel = false;
        self.persist_steps(journal).await
    }

    pub(super) async fn complete_snapshot_clone(
        &mut self,
        session: &Session,
        journal: &mut Journal,
        cancel: &CancellationToken,
        allow_writes: bool,
    ) -> Result<(), Error> {
        let OperationAction::Snapshot(SnapshotOperation::Clone {
            snapshot,
            as_template,
            ..
        }) = self.record.action
        else {
            return Err(Error::UnavailableAction);
        };
        let reference = self
            .record
            .result_reference
            .clone()
            .ok_or(Error::Protocol("clone result missing"))?;
        let (id, current) =
            snapshots::verify_clone(session, self.record.vm, snapshot, &reference).await?;
        let baseline = self
            .record
            .snapshot_clone_baseline
            .as_ref()
            .ok_or(Error::Protocol(
                "clone identity baseline is missing; no conversion was submitted",
            ))?;
        if baseline.contains(&id) {
            return Err(Error::Protocol("clone task returned a pre-existing VM"));
        }
        if self
            .record
            .created_vm
            .is_some_and(|previous| previous != id)
        {
            return Err(Error::SnapshotConflict);
        }
        if self.record.created_vm.is_none() && current["is_a_template"] != true {
            return Err(Error::Protocol("snapshot clone did not return a template"));
        }
        self.record.created_vm = Some(id);
        if as_template {
            if current["is_a_template"] != true {
                return Err(Error::SnapshotConflict);
            }
            return Ok(());
        }
        if self.record.steps.is_empty() {
            self.record.steps = vec![OperationStep {
                label: "Convert cloned template into a VM".into(),
                state: StepState::Pending,
                detail: String::new(),
            }];
            self.persist_steps(journal).await?;
        }
        if current["is_a_template"] == false {
            // The conversion reached the host even if its reply was lost; the host still
            // needs to recompute the new VM's allowed operations (see below).
            let refreshed = tokio::select! {_=cancel.cancelled()=>Err(Error::Closed),result=session.call("VM.update_allowed_operations",vec![json!(&reference)])=>result};
            self.record.steps[0].state = StepState::Verified;
            self.record.steps[0].detail = verified_detail(refreshed);
            return self.persist_steps(journal).await;
        }
        if self.record.steps[0].state != StepState::Pending {
            self.record.state = OperationState::OutcomeUnknown;
            self.record.detail =
                "The created template still exists; its conversion was not repeated".into();
            return self.persist_steps(journal).await;
        }
        if !allow_writes {
            self.record.state = OperationState::PartiallyCompleted;
            self.record.detail="Clone created as a template; conversion was not submitted during read-only reconciliation".into();
            self.record.can_cancel = false;
            return self.persist_steps(journal).await;
        }
        if self.record.cancel_requested {
            self.record.state = OperationState::PartiallyCompleted;
            self.record.detail =
                "Clone created as a template; conversion cancelled before dispatch".into();
            return self.persist_steps(journal).await;
        }
        self.record.state = OperationState::Dispatching;
        self.record.steps[0].state = StepState::Dispatching;
        self.persist_steps(journal).await?;
        let result = tokio::select! {_=cancel.cancelled()=>Err(Error::Closed),result=session.call("VM.set_is_a_template",vec![json!(&reference),json!(false)])=>result};
        if let Err(error) = result {
            let refused = matches!(error, Error::Api(_) | Error::Coordinator(_));
            self.record.state = if refused {
                OperationState::PartiallyCompleted
            } else {
                OperationState::OutcomeUnknown
            };
            self.record.steps[0].state = if refused {
                StepState::Failed
            } else {
                StepState::OutcomeUnknown
            };
            self.record.detail =
                format!("Clone created; template conversion could not be confirmed: {error}");
            self.record.can_cancel = false;
            return self.persist_steps(journal).await;
        }
        // XCP-ng 8.3 keeps the template's allowed operations on the new VM until asked to
        // recompute them, so Start would stay withheld; the recompute changes nothing else.
        let refreshed = tokio::select! {_=cancel.cancelled()=>Err(Error::Closed),result=session.call("VM.update_allowed_operations",vec![json!(&reference)])=>result};
        self.record.state = OperationState::Verifying;
        self.persist_steps(journal).await?;
        let (verified, after) =
            snapshots::verify_clone(session, self.record.vm, snapshot, &reference).await?;
        if verified != id || after["is_a_template"] != false {
            return Err(Error::SnapshotConflict);
        }
        self.record.steps[0].state = StepState::Verified;
        self.record.steps[0].detail = verified_detail(refreshed);
        self.persist_steps(journal).await
    }
}

/// The conversion step's detail: the identity check passed either way, and a host that
/// would not recompute the new VM's allowed operations is named rather than hidden.
fn verified_detail(refreshed: Result<serde_json::Value, Error>) -> String {
    match refreshed {
        Ok(_) => "New VM identity and halted state verified".into(),
        Err(error) => format!(
            "New VM identity and halted state verified; the host did not refresh its allowed operations ({error}), so Start may stay withheld until it does"
        ),
    }
}

#[cfg(test)]
#[path = "snapshot_operations_tests.rs"]
mod tests;
