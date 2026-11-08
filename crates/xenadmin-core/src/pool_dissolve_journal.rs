use super::*;
use crate::operations::Journal;
impl Journal {
    pub(crate) fn pool_dissolve_evidence(
        &self,
        evidence: &ResourceOperationRecord,
        index: usize,
    ) -> Result<ResourceOperationRecord, Error> {
        crate::pool_dissolve::validate_record(evidence)?;
        let p = evidence.pool_dissolve_plan.as_ref().ok_or(Error::Journal)?;
        let incoming = p.evidence.get(index).ok_or(Error::Journal)?;
        for _ in 0..16 {
            let mut current = self.resource_load(evidence.id)?.ok_or(Error::Journal)?;
            if current.pool != evidence.pool
                || current.target != evidence.target
                || current.action != evidence.action
            {
                return Err(Error::JournalChanged);
            }
            let plan = current.pool_dissolve_plan.as_mut().ok_or(Error::Journal)?;
            if plan.tasks_before != p.tasks_before || plan.evidence.len() != p.evidence.len() {
                return Err(Error::JournalChanged);
            }
            let step = plan.evidence.get_mut(index).ok_or(Error::Journal)?;
            if step.sent != incoming.sent {
                return Err(Error::JournalChanged);
            }
            macro_rules! merge {
                ($field:ident) => {
                    if step.$field.is_some()
                        && incoming.$field.is_some()
                        && step.$field != incoming.$field
                    {
                        return Err(Error::JournalChanged);
                    }
                    if step.$field.is_none() {
                        step.$field = incoming.$field.clone();
                    }
                };
            }
            merge!(task);
            merge!(task_uuid);

            step.completed |= incoming.completed;
            step.ended |= incoming.ended;
            step.verified |= incoming.verified;
            if !current.state.terminal()
                && !current.interrupted
                && current.state != OperationState::OutcomeUnknown
            {
                current.state = OperationState::Verifying;
            }
            current.pool_dissolve = Some(plan.result());
            match self.resource_save(&mut current) {
                Ok(()) => return Ok(current),
                Err(Error::JournalChanged) => continue,
                Err(e) => return Err(e),
            }
        }
        Err(Error::JournalChanged)
    }
}
