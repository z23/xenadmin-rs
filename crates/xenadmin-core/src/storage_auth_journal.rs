use super::*;
use crate::operations::Journal;
impl Journal {
    pub(crate) fn storage_auth_evidence(
        &self,
        evidence: &ResourceOperationRecord,
    ) -> Result<ResourceOperationRecord, Error> {
        for _ in 0..16 {
            let mut current = self.resource_load(evidence.id)?.ok_or(Error::Journal)?;
            if current.pool != evidence.pool
                || current.target != evidence.target
                || current.action != evidence.action
            {
                return Err(Error::OperationConflict);
            }
            if let (Some(n), Some(e)) =
                (&mut current.storage_auth_plan, &evidence.storage_auth_plan)
            {
                if n.operation != e.operation
                    || n.authentication != e.authentication
                    || n.baseline != e.baseline
                    || n.task_baseline != e.task_baseline
                    || n.task_uuid.is_some() && e.task_uuid.is_some() && n.task_uuid != e.task_uuid
                    || n.secrets.len() != e.secrets.len()
                {
                    return Err(Error::JournalChanged);
                }
                for (n, e) in n.secrets.iter_mut().zip(&e.secrets) {
                    if n.kind != e.kind
                        || n.username_digest != e.username_digest
                        || n.id.is_some() && e.id.is_some() && n.id != e.id
                    {
                        return Err(Error::JournalChanged);
                    }
                    n.completed |= e.completed;
                    n.cleaned |= e.cleaned;
                    if n.id.is_none() {
                        n.id = e.id;
                    }
                }
                if n.task_uuid.is_none() {
                    n.task_uuid = e.task_uuid;
                }
                n.provision_succeeded |= e.provision_succeeded;
                n.verified |= e.verified;
            }
            if let (Some(n), Some(e)) = (
                &mut current.storage_secret_cleanup,
                &evidence.storage_secret_cleanup,
            ) {
                if n.source != e.source || n.secrets.len() != e.secrets.len() {
                    return Err(Error::JournalChanged);
                }
                for (n, e) in n.secrets.iter_mut().zip(&e.secrets) {
                    if n.id != e.id {
                        return Err(Error::JournalChanged);
                    }
                    n.completed |= e.completed;
                }
            }
            for (n, e) in [
                (&mut current.task, &evidence.task),
                (&mut current.result_reference, &evidence.result_reference),
            ] {
                if n.is_some() && e.is_some() && n != e {
                    return Err(Error::JournalChanged);
                }
                if n.is_none() {
                    *n = e.clone();
                }
            }
            if current.created.is_some()
                && evidence.created.is_some()
                && current.created != evidence.created
            {
                return Err(Error::JournalChanged);
            }
            if current.created.is_none() {
                current.created = evidence.created;
            }
            crate::storage_auth::project(&mut current);
            match self.resource_save(&mut current) {
                Ok(()) => return Ok(current),
                Err(Error::JournalChanged) => continue,
                Err(e) => return Err(e),
            }
        }
        Err(Error::JournalChanged)
    }
}
