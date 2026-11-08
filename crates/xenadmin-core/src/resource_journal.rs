use super::*;
use crate::{OperationControl, StepState, operations::Journal};
use rusqlite::{OptionalExtension, params};

impl Journal {
    pub(crate) fn resource_load(
        &self,
        id: OperationId,
    ) -> Result<Option<ResourceOperationRecord>, Error> {
        let data:Option<(String,Option<String>,Option<String>,u64)>=self.0.query_row("SELECT payload,task,result_reference,revision FROM resource_operations WHERE id=?1",[id.to_string()],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).optional().map_err(|_|Error::Journal)?;
        data.map(|(payload, task, result_reference, revision)| {
            let mut record: ResourceOperationRecord =
                serde_json::from_str(&payload).map_err(|_| Error::Journal)?;
            record.task = task;
            record.result_reference = result_reference;
            record.revision = revision;
            Ok(record)
        })
        .transpose()
    }
    pub(crate) fn resource_save(&self, record: &mut ResourceOperationRecord) -> Result<(), Error> {
        if let Some(old) = self.resource_load(record.id)? {
            if old.pool != record.pool || old.target != record.target || old.action != record.action
            {
                return Err(Error::OperationConflict);
            }
            if (old.state.terminal() && old.state != record.state)
                || (old.task.is_some() && old.task != record.task)
                || (old.result_reference.is_some()
                    && old.result_reference != record.result_reference)
                || (old.created.is_some() && old.created != record.created)
                || (old.cancel_requested && !record.cancel_requested)
                || (old.cancel_dispatched && !record.cancel_dispatched)
                || (old.network_baseline.is_some()
                    && old.network_baseline != record.network_baseline)
                || (old.baseline_networks.is_some()
                    && old.baseline_networks != record.baseline_networks)
            {
                return Err(Error::JournalChanged);
            }
        } else if record.revision != 0 {
            return Err(Error::JournalChanged);
        }
        let changed=self.0.execute("INSERT INTO resource_operations(id,pool,payload,task,result_reference,terminal,revision) VALUES(?1,?2,?3,?4,?5,?6,?7+1) ON CONFLICT(id) DO UPDATE SET payload=excluded.payload,task=excluded.task,result_reference=excluded.result_reference,terminal=excluded.terminal,revision=excluded.revision WHERE resource_operations.revision=?7 AND (resource_operations.terminal=0 OR excluded.terminal=1)",params![record.id.to_string(),record.pool.to_string(),serde_json::to_string(record).map_err(|_|Error::Journal)?,record.task,record.result_reference,record.state.terminal(),record.revision]).map_err(|_|Error::Journal)?;
        if changed != 1 {
            return Err(Error::JournalChanged);
        }
        record.revision += 1;
        Ok(())
    }
    pub(crate) fn resource_admit(
        &self,
        record: &mut ResourceOperationRecord,
    ) -> Result<bool, Error> {
        self.0
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(|_| Error::Journal)?;
        let result = (|| {
            if self.load(record.id)?.is_some() {
                return Err(Error::OperationConflict);
            }
            if let Some(old) = self.resource_load(record.id)? {
                if old.pool != record.pool
                    || old.target != record.target
                    || old.action != record.action
                {
                    return Err(Error::OperationConflict);
                }
                *record = old;
                return Ok(false);
            }
            let busy:bool=self.0.query_row("SELECT EXISTS(SELECT 1 FROM operations WHERE pool=?1 AND terminal=0 UNION ALL SELECT 1 FROM resource_operations WHERE pool=?1 AND terminal=0)",[record.pool.to_string()],|row|row.get(0)).map_err(|_|Error::Journal)?;
            if busy {
                return Err(Error::ConflictingPoolOperation);
            }
            let count:u64=self.0.query_row("SELECT (SELECT COUNT(*) FROM operations WHERE terminal=0)+(SELECT COUNT(*) FROM resource_operations WHERE terminal=0)",[],|row|row.get(0)).map_err(|_|Error::Journal)?;
            if count >= 1000 {
                return Err(Error::JournalFull);
            }
            self.resource_save(record)?;
            self.0.execute("DELETE FROM resource_operations WHERE terminal=1 AND rowid NOT IN (SELECT rowid FROM resource_operations WHERE terminal=1 ORDER BY rowid DESC LIMIT 500)",[]).map_err(|_|Error::Journal)?;
            Ok(true)
        })();
        if result.is_ok() {
            self.0.execute_batch("COMMIT").map_err(|_| Error::Journal)?;
        } else {
            let _ = self.0.execute_batch("ROLLBACK");
        }
        result
    }
    pub(crate) fn resource_evidence(
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
            for (old, new) in [
                (&mut current.task, &evidence.task),
                (&mut current.result_reference, &evidence.result_reference),
            ] {
                if old.is_some() && new.is_some() && old != new {
                    return Err(Error::JournalChanged);
                }
                if old.is_none() {
                    *old = new.clone();
                }
            }
            if !current.state.terminal() {
                current.state = if current.task.is_some() {
                    OperationState::Tracking
                } else {
                    OperationState::Verifying
                };
            }
            match self.resource_save(&mut current) {
                Ok(()) => return Ok(current),
                Err(Error::JournalChanged) => continue,
                Err(error) => return Err(error),
            }
        }
        Err(Error::JournalChanged)
    }
    pub(crate) fn resource_history(
        &self,
        pool: PoolId,
    ) -> Result<Vec<ResourceOperationRecord>, Error> {
        let mut query = self
            .0
            .prepare(
                "SELECT id FROM resource_operations WHERE pool=?1 ORDER BY rowid DESC LIMIT 1500",
            )
            .map_err(|_| Error::Journal)?;
        let ids = query
            .query_map([pool.to_string()], |row| row.get::<_, String>(0))
            .map_err(|_| Error::Journal)?;
        let mut records = Vec::new();
        for id in ids {
            let id: OperationId =
                serde_json::from_value(serde_json::Value::String(id.map_err(|_| Error::Journal)?))
                    .map_err(|_| Error::Journal)?;
            if let Some(record) = self.resource_load(id)? {
                records.push(record);
            }
        }
        Ok(records)
    }
    pub(crate) fn resource_control(
        &self,
        pool: PoolId,
        id: OperationId,
        control: OperationControl,
    ) -> Result<ResourceOperationRecord, Error> {
        for _ in 0..16 {
            let mut record = self
                .resource_load(id)?
                .filter(|record| record.pool == pool)
                .ok_or(Error::OperationConflict)?;
            if record.state.terminal() {
                return Err(Error::UnavailableAction);
            }
            match control {
                OperationControl::Recheck => return Ok(record),
                OperationControl::AcknowledgeUnknown => {
                    if record.state != OperationState::OutcomeUnknown {
                        return Err(Error::UnavailableAction);
                    }
                    record.state = OperationState::AcknowledgedUnknown;
                    record.can_cancel = false;
                    record.detail="Outcome remains unverified. Operator acknowledged it and released the local pool reservation.".into();
                }
                OperationControl::RequestCancellation => {
                    record.cancel_requested = true;
                    record.detail = "Cancellation requested; server work may still continue".into();
                }
            }
            match self.resource_save(&mut record) {
                Ok(()) => return Ok(record),
                Err(Error::JournalChanged) => continue,
                Err(error) => return Err(error),
            }
        }
        Err(Error::JournalChanged)
    }
    pub(crate) fn resource_recover(
        &self,
        pool: PoolId,
    ) -> Result<Vec<ResourceOperationRecord>, Error> {
        let mut rows = self.resource_history(pool)?;
        for record in &mut rows {
            if record.state.terminal() {
                continue;
            }
            if record.state == OperationState::Preparing {
                record.state = if !record.steps.is_empty()
                    && record
                        .steps
                        .iter()
                        .all(|step| step.state == StepState::Verified)
                {
                    OperationState::Succeeded
                } else if record
                    .steps
                    .iter()
                    .any(|step| step.state == StepState::Verified)
                {
                    OperationState::PartiallyCompleted
                } else {
                    OperationState::Cancelled
                };
                record.detail =
                    "Interrupted before the next dispatch; remaining changes were not submitted."
                        .into();
            } else {
                record.state = OperationState::OutcomeUnknown;
                record.detail="Interrupted operation. Reconciliation reads existing evidence and never repeats a write.".into();
            }
            record.can_cancel = false;
            self.resource_save(record)?;
        }
        Ok(rows)
    }
}
