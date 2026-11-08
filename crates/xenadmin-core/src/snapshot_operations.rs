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
            let current = journal.load(self.record.id)?.ok_or(Error::Journal)?;
            self.record.revision = current.revision;
            self.record.cancel_requested |= current.cancel_requested;
            if self.record.cancel_requested {
                self.record.state = if index == 0 {
                    OperationState::Cancelled
                } else {
                    OperationState::PartiallyCompleted
                };
                self.record.detail="Stopped before the next deletion step; already deleted resources remain deleted".into();
                self.record.can_cancel = false;
                return self.persist_steps(journal).await;
            }
            if cancel.is_cancelled() {
                return Err(Error::Closed);
            }
            self.record.state = OperationState::Dispatching;
            self.record.steps[index].state = StepState::Dispatching;
            self.record.detail = format!(
                "{} ({}/{})",
                step.label(),
                index + 1,
                self.record.steps.len()
            );
            self.persist_steps(journal).await?;
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
            self.record.steps[0].state = StepState::Verified;
            self.record.steps[0].detail = "New VM identity and halted state verified".into();
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
        self.record.state = OperationState::Verifying;
        self.persist_steps(journal).await?;
        let (verified, after) =
            snapshots::verify_clone(session, self.record.vm, snapshot, &reference).await?;
        if verified != id || after["is_a_template"] != false {
            return Err(Error::SnapshotConflict);
        }
        self.record.steps[0].state = StepState::Verified;
        self.record.steps[0].detail = "New VM identity and halted state verified".into();
        self.persist_steps(journal).await
    }
}

#[cfg(test)]
#[path = "snapshot_operations_tests.rs"]
mod tests;
