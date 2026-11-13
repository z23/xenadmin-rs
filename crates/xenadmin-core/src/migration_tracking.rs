use super::*;
use xenadmin_xapi::enumerations::{task_allowed_operations, task_status_type};
use xenadmin_xapi::records::TaskRecord;
use xenadmin_xapi::{calls, refs};

impl MutationRequest {
    pub(super) async fn track_migration(
        &mut self,
        session: &Session,
        journal: &mut Journal,
        cancel: &CancellationToken,
        rechecking: bool,
    ) -> Result<(), Error> {
        let reference = self.record.task.clone().ok_or(Error::Journal)?;
        let mut delay = 1;
        loop {
            self.record = journal.load(self.record.id)?.ok_or(Error::Journal)?;
            if self.migration_stopped(journal, rechecking)? {
                self.emit().await;
                return Ok(());
            }
            if self.record.task.as_ref() != Some(&reference) {
                return Err(Error::OperationConflict);
            }
            let task = tokio::select! {_=cancel.cancelled()=>return Err(Error::Closed),value=session.send(calls::task::GetRecord(refs::task(&reference)))=>value};
            let task = match task {
                Ok(task) => {
                    delay = 1;
                    task
                }
                Err(error)
                    if !rechecking
                        && error.retryable()
                        && !matches!(error, Error::Coordinator(_)) =>
                {
                    self.record.state = OperationState::Tracking;
                    self.record.detail =
                        format!("Cannot query the host migration task; retrying: {error}");
                    if self.save_migration_evidence(journal, false).await? {
                        return Ok(());
                    }
                    tokio::select! {_=cancel.cancelled()=>return Err(Error::Closed),_=tokio::time::sleep(Duration::from_secs(delay))=>{}}
                    delay = (delay * 2).min(10);
                    continue;
                }
                Err(error) => return Err(error),
            };
            self.record.progress = TaskRecord::new(&task)
                .progress()
                .number()
                .filter(|p| p.is_finite())
                .map(|p| p.clamp(0.0, 1.0));
            self.record.can_cancel = !rechecking
                && self.record.batch_parent.is_none()
                && TaskRecord::new(&task)
                    .allowed_operations()
                    .members()
                    .is_some_and(|v| v.iter().any(|v| v == task_allowed_operations::CANCEL));
            match TaskRecord::new(&task).status().name() {
                Some(task_status_type::SUCCESS) => {
                    return self
                        .complete_migration(session, journal, &task, !rechecking)
                        .await;
                }
                Some(task_status_type::FAILURE) => {
                    return self
                        .failed_migration(session, journal, &task, false, rechecking)
                        .await;
                }
                Some(task_status_type::CANCELLED) => {
                    return self
                        .failed_migration(session, journal, &task, true, rechecking)
                        .await;
                }
                Some(task_status_type::PENDING | task_status_type::CANCELLING) => {}
                _ => return Err(Error::Protocol("unknown migration task status")),
            }
            if rechecking {
                self.record.state = OperationState::OutcomeUnknown;
                self.record.can_cancel = false;
                self.record.detail="The existing migration task is still active. Recheck later; no task cancellation or remaining change was submitted.".into();
                self.save_migration_evidence(journal, true).await?;
                return Ok(());
            }
            self.record.state = OperationState::Tracking;
            self.record.detail = "Tracking the existing migration task".into();
            if self.save_migration_evidence(journal, false).await? {
                return Ok(());
            }
            if self.record.cancel_requested
                && !self.record.cancel_dispatched
                && self.record.can_cancel
            {
                if !journal.migration_cancel_dispatch(&mut self.record)? {
                    self.record = journal.load(self.record.id)?.ok_or(Error::Journal)?;
                    self.emit().await;
                    return Ok(());
                }
                self.emit_dispatch();
                if let Err(error) = session
                    .send(calls::task::Cancel(refs::task(&reference)))
                    .await
                {
                    if matches!(error, Error::Coordinator(_)) {
                        return Err(error);
                    }
                    self.record.detail =
                        format!("Migration cancellation was not confirmed: {error}");
                    if self.save_migration_evidence(journal, false).await? {
                        return Ok(());
                    }
                }
            }
            tokio::select! {_=cancel.cancelled()=>return Err(Error::Closed),_=tokio::time::sleep(Duration::from_secs(1))=>{}}
        }
    }
}

impl Journal {
    fn migration_cancel_dispatch(&self, record: &mut OperationRecord) -> Result<bool, Error> {
        self.0
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(|_| Error::Journal)?;
        let result = (|| {
            let current = self.load(record.id)?.ok_or(Error::Journal)?;
            if current.revision != record.revision
                || current.state.terminal()
                || current.state == OperationState::OutcomeUnknown
                || current.batch_parent.is_some()
                || current
                    .migration_plan
                    .as_ref()
                    .is_none_or(|p| p.interrupted)
                || !current.cancel_requested
                || current.cancel_dispatched
                || !current.can_cancel
            {
                return Ok(false);
            }
            record.cancel_dispatched = true;
            record.detail = "Waiting for the host to confirm migration cancellation".into();
            self.save(record)?;
            Ok(true)
        })();
        if result.is_ok() {
            self.0.execute_batch("COMMIT").map_err(|_| Error::Journal)?;
        } else {
            let _ = self.0.execute_batch("ROLLBACK");
        }
        result
    }
}
