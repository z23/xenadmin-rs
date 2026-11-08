use super::*;
use crate::operations::Journal;
impl Journal {
    pub(crate) fn storage_graph_evidence(
        &self,
        evidence: &ResourceOperationRecord,
        index: usize,
    ) -> Result<ResourceOperationRecord, Error> {
        let p = evidence.storage_graph_plan.as_ref().ok_or(Error::Journal)?;
        let incoming = p.evidence.get(index).ok_or(Error::Journal)?;
        for _ in 0..16 {
            let mut current = self.resource_load(evidence.id)?.ok_or(Error::Journal)?;
            if current.pool != evidence.pool
                || current.target != evidence.target
                || current.action != evidence.action
            {
                return Err(Error::JournalChanged);
            }
            let plan = current.storage_graph_plan.as_mut().ok_or(Error::Journal)?;
            let step = plan.evidence.get_mut(index).ok_or(Error::Journal)?;
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
            merge!(result);
            merge!(created);
            if !step.secrets.is_empty()
                && !incoming.secrets.is_empty()
                && step.secrets != incoming.secrets
            {
                return Err(Error::JournalChanged);
            }
            if step.secrets.is_empty() {
                step.secrets = incoming.secrets.clone();
            }
            step.completed |= incoming.completed;
            step.ended |= incoming.ended;
            step.verified |= incoming.verified;
            if !current.state.terminal()
                && !current.interrupted
                && current.state != OperationState::OutcomeUnknown
            {
                current.state = OperationState::Verifying;
            }
            current.storage_graph = plan.results();
            match self.resource_save(&mut current) {
                Ok(()) => return Ok(current),
                Err(Error::JournalChanged) => continue,
                Err(e) => return Err(e),
            }
        }
        Err(Error::JournalChanged)
    }
}
