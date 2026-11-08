use super::*;
use crate::operations::Journal;
impl Journal {
    /// Serialize a local credential replacement with recovery and acknowledgement.
    pub(crate) fn root_password_credential_guard<T>(
        &self,
        id: OperationId,
        pool: PoolId,
        replace: impl FnOnce() -> Result<T, Error>,
    ) -> Result<T, Error> {
        self.0
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(|_| Error::Journal)?;
        let result = (|| {
            let row = self.resource_load(id)?.ok_or(Error::Journal)?;
            if row.pool != pool
                || row.state != OperationState::Verifying
                || row.interrupted
                || row
                    .root_password_plan
                    .as_ref()
                    .is_none_or(|p| !p.completed || !p.login_verified || p.credentials_updated)
            {
                return Err(Error::Stale);
            }
            replace()
        })();
        match result {
            Ok(value) => {
                if self.0.execute_batch("COMMIT").is_err() {
                    let _ = self.0.execute_batch("ROLLBACK");
                    return Err(Error::Journal);
                }
                Ok(value)
            }
            Err(error) => {
                let _ = self.0.execute_batch("ROLLBACK");
                Err(error)
            }
        }
    }
    pub(crate) fn root_password_evidence(
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
            let (Some(p), Some(e)) = (
                &mut current.root_password_plan,
                &evidence.root_password_plan,
            ) else {
                return Err(Error::Journal);
            };
            if p.expected != e.expected || p.sent != e.sent {
                return Err(Error::JournalChanged);
            }
            p.completed |= e.completed;
            p.login_verified |= e.login_verified;
            p.credentials_updated |= e.credentials_updated;
            if !current.interrupted && !current.state.terminal() {
                current.state = evidence.state.clone();
                current.detail = evidence.detail.clone();
                current.steps = evidence.steps.clone();
                current.can_cancel = evidence.can_cancel;
            }
            crate::root_password::project(&mut current);
            match self.resource_save(&mut current) {
                Ok(()) => return Ok(current),
                Err(Error::JournalChanged) => continue,
                Err(error) => return Err(error),
            }
        }
        Err(Error::JournalChanged)
    }
}
