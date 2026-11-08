use super::*;
use crate::operations::Journal;
impl Journal {
    pub(crate) fn host_power_evidence(
        &self,
        evidence: &ResourceOperationRecord,
    ) -> Result<ResourceOperationRecord, Error> {
        for _ in 0..16 {
            let mut current = self.resource_load(evidence.id)?.ok_or(Error::Journal)?;
            if current.pool != evidence.pool
                || current.target != evidence.target
                || current.action != evidence.action
            {
                return Err(Error::JournalChanged);
            }
            let (Some(n), Some(e)) = (&mut current.host_power_plan, &evidence.host_power_plan)
            else {
                return Err(Error::Journal);
            };
            if n.operation != e.operation
                || n.baseline != e.baseline
                || n.secret.is_some() && e.secret.is_some() && n.secret != e.secret
            {
                return Err(Error::JournalChanged);
            }
            n.secret_completed |= e.secret_completed;
            n.configure_completed |= e.configure_completed;
            n.verified |= e.verified;
            n.cleaned |= e.cleaned;
            if n.secret.is_none() {
                n.secret = e.secret;
            }
            crate::host_power::project(&mut current);
            match self.resource_save(&mut current) {
                Ok(()) => return Ok(current),
                Err(Error::JournalChanged) => continue,
                Err(e) => return Err(e),
            }
        }
        Err(Error::JournalChanged)
    }
}
