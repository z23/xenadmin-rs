use super::*;

impl MutationRequest {
    pub(super) async fn save_migration_evidence(
        &mut self,
        journal: &mut Journal,
        rechecking: bool,
    ) -> Result<bool, Error> {
        let received = self.record.clone();
        for _ in 0..16 {
            let mut current = journal.load(received.id)?.ok_or(Error::Journal)?;
            if current.action != received.action
                || current.vm != received.vm
                || current.pool != received.pool
            {
                return Err(Error::OperationConflict);
            }
            let mut plan = received.migration_plan.clone().ok_or(Error::Journal)?;
            if let Some(old) = &current.migration_plan {
                plan.interrupted |= old.interrupted;
                plan.completed |= old.completed;
                plan.affinity_received |= old.affinity_received;
                if plan.outcome.is_none() {
                    plan.outcome = old.outcome.clone();
                }
                if !plan.extends(old) {
                    return Err(Error::JournalChanged);
                }
            }
            let stop = current.state.terminal()
                || !rechecking
                    && (plan.interrupted
                        || current.state == OperationState::OutcomeUnknown
                        || journal.workflow_child_stopped(current.id)?);
            if !stop {
                current.state = received.state.clone();
                current.steps = received.steps.clone();
                current.detail = received.detail.clone();
                current.progress = received.progress;
                current.can_cancel = received.can_cancel;
            }
            if current.steps.is_empty() {
                current.steps = received.steps.clone();
            }
            for (old, new) in [
                (&mut current.task, &received.task),
                (&mut current.result_reference, &received.result_reference),
            ] {
                if old.is_some() && new.is_some() && old.as_ref() != new.as_ref() {
                    return Err(Error::OperationConflict);
                }
                if old.is_none() {
                    *old = new.clone();
                }
            }
            current.migration_plan = Some(plan);
            for id in &received.created_devices {
                if !current.created_devices.contains(id) {
                    current.created_devices.push(id.clone());
                }
            }
            match journal.save(&mut current) {
                Ok(()) => {
                    self.record = current;
                    self.emit().await;
                    return Ok(stop);
                }
                Err(Error::JournalChanged) => continue,
                Err(error) => return Err(error),
            }
        }
        Err(Error::JournalChanged)
    }
    pub(super) fn migration_stopped(
        &self,
        journal: &Journal,
        rechecking: bool,
    ) -> Result<bool, Error> {
        Ok(self.record.state.terminal()
            || !rechecking
                && (self
                    .record
                    .migration_plan
                    .as_ref()
                    .is_some_and(|p| p.interrupted)
                    || self.record.state == OperationState::OutcomeUnknown
                    || journal.workflow_child_stopped(self.record.id)?))
    }
}
