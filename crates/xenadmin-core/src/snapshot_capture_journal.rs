use super::*;
use crate::operations::Journal;
impl Journal {
    pub(crate) fn snapshot_preview_evidence(
        &self,
        evidence: &ResourceOperationRecord,
    ) -> Result<ResourceOperationRecord, Error> {
        crate::snapshot_capture::validate_record(evidence)?;
        for _ in 0..16 {
            let mut current = self.resource_load(evidence.id)?.ok_or(Error::Journal)?;
            if current.pool != evidence.pool
                || current.target != evidence.target
                || current.action != evidence.action
            {
                return Err(Error::JournalChanged);
            }
            let (Some(p), Some(e)) = (
                &mut current.snapshot_preview_plan,
                &evidence.snapshot_preview_plan,
            ) else {
                return Err(Error::Journal);
            };
            if p.expected != e.expected
                || p.sha256 != e.sha256
                || p.baseline != e.baseline
                || p.create_sent != e.create_sent
                || p.upload_sent != e.upload_sent
                || p.create_reference.is_some()
                    && e.create_reference.is_some()
                    && p.create_reference != e.create_reference
                || p.blob.is_some() && e.blob.is_some() && p.blob != e.blob
            {
                return Err(Error::JournalChanged);
            }
            if p.create_reference.is_none() {
                p.create_reference = e.create_reference.clone();
            }
            if p.blob.is_none() {
                p.blob = e.blob;
            }
            p.content_verified |= e.content_verified;
            if !current.interrupted && !current.state.terminal() {
                current.state = evidence.state.clone();
                current.detail = evidence.detail.clone();
                current.steps = evidence.steps.clone();
                current.can_cancel = evidence.can_cancel;
            }
            match self.resource_save(&mut current) {
                Ok(()) => return Ok(current),
                Err(Error::JournalChanged) => continue,
                Err(e) => return Err(e),
            }
        }
        Err(Error::JournalChanged)
    }
}
