use super::*;
use crate::{OperationControl, StepState, operations::Journal};
use rusqlite::{OptionalExtension, params};

impl Journal {
    pub(crate) fn transfer_publication_guard<T>(
        &self,
        expected: &ResourceOperationRecord,
        reconcile: bool,
        cancel: &tokio_util::sync::CancellationToken,
        publish: impl FnOnce() -> Result<T, Error>,
    ) -> Result<T, Error> {
        self.0
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(|_| Error::Journal)?;
        let result = (|| {
            let current = self.resource_load(expected.id)?.ok_or(Error::Journal)?;
            if current.revision() != expected.revision()
                || current.pool != expected.pool
                || current.action != expected.action
                || current.target != expected.target
                || current.transfer_plan != expected.transfer_plan
                || current.state != OperationState::Verifying
                || current.cancel_requested
                || current.interrupted && !reconcile
                || !matches!(&current.action,crate::ResourceAction::Transfer(action) if action.is_export())
                || current.transfer_plan.as_ref().is_none_or(|p| {
                    !p.task_success || p.stream_invalid || p.stream_receipt.is_none()
                })
            {
                return Err(Error::JournalChanged);
            }
            if cancel.is_cancelled() {
                return Err(Error::Closed);
            }
            publish()
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
    pub(crate) fn resource_pool_busy(&self, pool: PoolId) -> Result<bool, Error> {
        Ok(self
            .active_resources(pool)?
            .iter()
            .any(|record| record.action != ResourceAction::CancelTask))
    }
    fn active_resources(&self, pool: PoolId) -> Result<Vec<ResourceOperationRecord>, Error> {
        let mut query = self
            .0
            .prepare("SELECT id,pool FROM resource_operations WHERE terminal=0 LIMIT 1001")
            .map_err(|_| Error::Journal)?;
        let records = query
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|_| Error::Journal)?
            .map(|row| {
                let (id, stored_pool) = row.map_err(|_| Error::Journal)?;
                let id = serde_json::from_value(serde_json::Value::String(id))
                    .map_err(|_| Error::Journal)?;
                let record = self.resource_load(id)?.ok_or(Error::Journal)?;
                if record.pool.to_string() != stored_pool || record.state.terminal() {
                    return Err(Error::Journal);
                }
                Ok(record)
            })
            .collect::<Result<Vec<_>, Error>>()?;
        if records.len() > 1000 {
            return Err(Error::JournalFull);
        }
        Ok(records
            .into_iter()
            .filter(|record| record.applies_to_pool(pool))
            .collect())
    }
    pub(crate) fn resource_load(
        &self,
        id: OperationId,
    ) -> Result<Option<ResourceOperationRecord>, Error> {
        let data:Option<(String,Option<String>,Option<String>,u64)>=self.0.query_row("SELECT payload,task,result_reference,revision FROM resource_operations WHERE id=?1",[id.to_string()],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).optional().map_err(|_|Error::Journal)?;
        data.map(|(payload, task, result_reference, revision)| {
            let payload: serde_json::Value =
                serde_json::from_str(&payload).map_err(|_| Error::Journal)?;
            let management_network_plan = payload
                .get("management_network_plan")
                .filter(|v| !v.is_null())
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .map_err(|_| Error::Journal)?;
            let network_topology_plan = payload
                .get("network_topology_plan")
                .filter(|v| !v.is_null())
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .map_err(|_| Error::Journal)?;
            let power_batch_plan = payload
                .get("power_batch_plan")
                .filter(|v| !v.is_null())
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .map_err(|_| Error::Journal)?;
            let root_password_plan = payload
                .get("root_password_plan")
                .filter(|v| !v.is_null())
                .map(|v| serde_json::from_value(v.clone()).map_err(|_| Error::Journal))
                .transpose()?;
            let host_power_plan = payload
                .get("host_power_plan")
                .filter(|v| !v.is_null())
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .map_err(|_| Error::Journal)?;
            let host_plan = payload
                .get("host_plan")
                .filter(|v| !v.is_null())
                .map(|v| serde_json::from_value(v.clone()).map_err(|_| Error::Journal))
                .transpose()?;
            let transfer_plan = payload
                .get("transfer_plan")
                .filter(|value| !value.is_null())
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .map_err(|_| Error::Journal)?;
            let snapshot_policy_evidence = payload
                .get("snapshot_policy_evidence")
                .filter(|v| !v.is_null())
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .map_err(|_| Error::Journal)?;
            let cross_pool_plan = payload
                .get("cross_pool_plan")
                .filter(|v| !v.is_null())
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .map_err(|_| Error::Journal)?;
            let storage_auth_plan = payload
                .get("storage_auth_plan")
                .filter(|v| !v.is_null())
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .map_err(|_| Error::Journal)?;
            let cloud_init_plan = payload
                .get("cloud_init_plan")
                .filter(|v| !v.is_null())
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .map_err(|_| Error::Journal)?;
            let storage_secret_cleanup = payload
                .get("storage_secret_cleanup")
                .filter(|v| !v.is_null())
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .map_err(|_| Error::Journal)?;
            let mut record: ResourceOperationRecord =
                serde_json::from_value(payload).map_err(|_| Error::Journal)?;
            if !record.state.terminal() {
                record
                    .action
                    .validate(record.target)
                    .map_err(|_| Error::Journal)?;
                if matches!(record.target,ResourceTarget::Pool(pool) if pool!=record.pool) {
                    return Err(Error::Journal);
                }
            }
            record.management_network_plan = management_network_plan;
            record.network_topology_plan = network_topology_plan;
            record.power_batch_plan = power_batch_plan;
            record.storage_auth_plan = storage_auth_plan;
            record.storage_secret_cleanup = storage_secret_cleanup;
            record.host_plan = host_plan;
            record.host_power_plan = host_power_plan;
            record.root_password_plan = root_password_plan;
            record.snapshot_policy_evidence = snapshot_policy_evidence;
            record.transfer_plan = transfer_plan;
            record.cross_pool_plan = cross_pool_plan;
            record.cloud_init_plan = cloud_init_plan;
            record.task = task;
            record.result_reference = result_reference;
            record.revision = revision;
            record.interrupted |= record.state == OperationState::OutcomeUnknown;
            // Terminal rejection before preparation may legitimately have no plan.
            if !record.state.terminal() || record.power_batch_plan.is_some() {
                crate::power_batch::validate_record(&record)?;
            }

            if record.id != id {
                return Err(Error::Journal);
            }
            crate::cross_pool_backend::validate_record(&record)?;
            crate::host_power::validate_record(&record)?;
            super::topology::validate_record(&record).map_err(|_| Error::Journal)?;
            crate::root_password::validate_record(&record)?;
            super::management::validate_record(&record).map_err(|_| Error::Journal)?;
            crate::storage_auth::validate_record(&record)?;
            crate::cloud_init::validate_record(&record)?;
            crate::host_support::restore::validate_record(&record)?;
            Ok(record)
        })
        .transpose()
    }
    pub(crate) fn resource_save(&self, record: &mut ResourceOperationRecord) -> Result<(), Error> {
        record.interrupted |= record.state == OperationState::OutcomeUnknown;
        if !record.state.terminal() || record.power_batch_plan.is_some() {
            crate::power_batch::validate_record(record)?;
        }
        crate::cross_pool_backend::validate_record(record)?;
        super::topology::validate_record(record).map_err(|_| Error::Journal)?;
        super::management::validate_record(record).map_err(|_| Error::Journal)?;
        if record.state == OperationState::OutcomeUnknown
            && let Some(plan) = &mut record.cloud_init_plan
        {
            plan.interrupted = true;
        }
        crate::host_power::validate_record(record)?;
        crate::root_password::validate_record(record)?;
        crate::storage_auth::validate_record(record)?;
        crate::cloud_init::validate_record(record)?;
        crate::host_support::restore::validate_record(record)?;
        if let Some(old) = self.resource_load(record.id)? {
            if old.root_password_plan.as_ref().is_some_and(|p| {
                record
                    .root_password_plan
                    .as_ref()
                    .is_none_or(|n| !n.extends(p))
            }) || old.host_power_plan.as_ref().is_some_and(|p| {
                record
                    .host_power_plan
                    .as_ref()
                    .is_none_or(|n| !n.extends(p))
            }) || old.storage_auth_plan.as_ref().is_some_and(|p| {
                record
                    .storage_auth_plan
                    .as_ref()
                    .is_none_or(|n| !n.extends(p))
            }) || old.storage_secret_cleanup.as_ref().is_some_and(|p| {
                record
                    .storage_secret_cleanup
                    .as_ref()
                    .is_none_or(|n| !n.extends(p))
            }) {
                return Err(Error::JournalChanged);
            }
            if old.pool != record.pool || old.target != record.target || old.action != record.action
            {
                return Err(Error::OperationConflict);
            }
            if old.management_network_plan.as_ref().is_some_and(|p| {
                record
                    .management_network_plan
                    .as_ref()
                    .is_none_or(|next| !p.accepts(next))
            }) {
                return Err(Error::JournalChanged);
            }
            if old.network_topology_plan.as_ref().is_some_and(|p| {
                record
                    .network_topology_plan
                    .as_ref()
                    .is_none_or(|n| !p.accepts(n))
            }) {
                return Err(Error::JournalChanged);
            }
            if old.power_batch_plan.as_ref().is_some_and(|p| {
                record
                    .power_batch_plan
                    .as_ref()
                    .is_none_or(|n| !n.extends(p))
            }) {
                return Err(Error::JournalChanged);
            }
            if old.cloud_init_plan.as_ref().is_some_and(|p| {
                record
                    .cloud_init_plan
                    .as_ref()
                    .is_none_or(|n| !p.accepts(n))
            }) {
                return Err(Error::JournalChanged);
            }
            if !crate::transfer::valid_update(&old, record) {
                return Err(Error::JournalChanged);
            }
            if old.cross_pool_plan.as_ref().is_some_and(|p| {
                record
                    .cross_pool_plan
                    .as_ref()
                    .is_none_or(|n| !p.accepts(n))
            }) || (matches!(record.action, ResourceAction::CrossPoolVm(_))
                && !cross_created_contains(&record.transfer_created, &old.transfer_created))
            {
                return Err(Error::JournalChanged);
            }
            if old.snapshot_policy_evidence.as_ref().is_some_and(|e| {
                record
                    .snapshot_policy_evidence
                    .as_ref()
                    .is_none_or(|n| !e.accepts(n))
            }) {
                return Err(Error::JournalChanged);
            }
            if (old.state.terminal() && old.state != record.state)
                || (old.task.is_some() && old.task != record.task)
                || (old.result_reference.is_some()
                    && old.result_reference != record.result_reference)
                || (old.created.is_some() && old.created != record.created)
                || (old.interrupted && !record.interrupted)
                || (old.cancel_requested && !record.cancel_requested)
                || (old.cancel_dispatched && !record.cancel_dispatched)
                || old.host_plan.as_ref().is_some_and(|plan| {
                    record
                        .host_plan
                        .as_ref()
                        .is_none_or(|new| !plan.accepts(new))
                })
                || (old.network_baseline.is_some()
                    && old.network_baseline != record.network_baseline)
                || (old.pool_completed && !record.pool_completed)
                || (old.appliance_completed && !record.appliance_completed)
                || (old.appliance_baseline.is_some()
                    && old.appliance_baseline != record.appliance_baseline)
                || (old.join_baseline.is_some() && old.join_baseline != record.join_baseline)
                || (old.subject_baseline.is_some()
                    && old.subject_baseline != record.subject_baseline)
                || (old.storage_trim_succeeded && !record.storage_trim_succeeded)
                || (old.storage_baseline.is_some()
                    && old.storage_baseline != record.storage_baseline)
                || (old.baseline_networks.is_some()
                    && old.baseline_networks != record.baseline_networks)
            {
                return Err(Error::JournalChanged);
            }
        } else if record.revision != 0 {
            return Err(Error::JournalChanged);
        }
        let mut payload = serde_json::to_value(&record).map_err(|_| Error::Journal)?;
        if let Some(plan) = &record.power_batch_plan {
            payload.as_object_mut().ok_or(Error::Journal)?.insert(
                "power_batch_plan".into(),
                serde_json::to_value(plan).map_err(|_| Error::Journal)?,
            );
        }
        if let Some(plan) = &record.cross_pool_plan {
            payload.as_object_mut().ok_or(Error::Journal)?.insert(
                "cross_pool_plan".into(),
                serde_json::to_value(plan).map_err(|_| Error::Journal)?,
            );
        }
        for (key, value) in [
            (
                "root_password_plan",
                serde_json::to_value(&record.root_password_plan),
            ),
            (
                "host_power_plan",
                serde_json::to_value(&record.host_power_plan),
            ),
            (
                "storage_auth_plan",
                serde_json::to_value(&record.storage_auth_plan),
            ),
            (
                "storage_secret_cleanup",
                serde_json::to_value(&record.storage_secret_cleanup),
            ),
        ] {
            payload
                .as_object_mut()
                .ok_or(Error::Journal)?
                .insert(key.into(), value.map_err(|_| Error::Journal)?);
        }
        if let Some(plan) = &record.management_network_plan {
            payload.as_object_mut().ok_or(Error::Journal)?.insert(
                "management_network_plan".into(),
                serde_json::to_value(plan).map_err(|_| Error::Journal)?,
            );
        }
        if let Some(plan) = &record.network_topology_plan {
            payload.as_object_mut().ok_or(Error::Journal)?.insert(
                "network_topology_plan".into(),
                serde_json::to_value(plan).map_err(|_| Error::Journal)?,
            );
        }
        if let Some(plan) = &record.host_plan {
            payload.as_object_mut().ok_or(Error::Journal)?.insert(
                "host_plan".into(),
                serde_json::to_value(plan).map_err(|_| Error::Journal)?,
            );
        }
        if let Some(plan) = &record.transfer_plan {
            payload.as_object_mut().ok_or(Error::Journal)?.insert(
                "transfer_plan".into(),
                serde_json::to_value(plan).map_err(|_| Error::Journal)?,
            );
        }
        if let Some(e) = &record.snapshot_policy_evidence {
            payload.as_object_mut().ok_or(Error::Journal)?.insert(
                "snapshot_policy_evidence".into(),
                serde_json::to_value(e).map_err(|_| Error::Journal)?,
            );
        }
        if let Some(plan) = &record.cloud_init_plan {
            payload.as_object_mut().ok_or(Error::Journal)?.insert(
                "cloud_init_plan".into(),
                serde_json::to_value(plan).map_err(|_| Error::Journal)?,
            );
        }
        let payload = serde_json::to_string(&payload).map_err(|_| Error::Journal)?;
        let changed=self.0.execute("INSERT INTO resource_operations(id,pool,payload,task,result_reference,terminal,revision) VALUES(?1,?2,?3,?4,?5,?6,?7+1) ON CONFLICT(id) DO UPDATE SET payload=excluded.payload,task=excluded.task,result_reference=excluded.result_reference,terminal=excluded.terminal,revision=excluded.revision WHERE resource_operations.revision=?7 AND (resource_operations.terminal=0 OR excluded.terminal=1)",params![record.id.to_string(),record.pool.to_string(),payload,record.task,record.result_reference,record.state.terminal(),record.revision]).map_err(|_|Error::Journal)?;
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
        crate::cloud_init::validate_record(record)?;
        crate::host_support::restore::validate_record(record)?;
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
                    || old.cloud_init_plan.as_ref().map(|p| p.sha256)
                        != record.cloud_init_plan.as_ref().map(|p| p.sha256)
                    || old.transfer_plan.as_ref().map(|p| &p.file)
                        != record.transfer_plan.as_ref().map(|p| &p.file)
                {
                    return Err(Error::OperationConflict);
                }
                *record = old;
                return Ok(false);
            }
            // Cancelling a server task is a recovery action. Reserve only that task;
            // never release or mutate the original operation's reservation.
            let busy = if record.action == ResourceAction::CancelTask {
                self.active_resources(record.pool)?.iter().any(|old| {
                    old.target == record.target && old.action == ResourceAction::CancelTask
                })
            } else {
                let mut busy = false;
                for pool in record.pool_scopes() {
                    let vm_busy: bool = self
                        .0
                        .query_row(
                            "SELECT EXISTS(SELECT 1 FROM operations WHERE pool=?1 AND terminal=0)",
                            [pool.to_string()],
                            |row| row.get(0),
                        )
                        .map_err(|_| Error::Journal)?;
                    busy |= vm_busy || self.resource_pool_busy(pool)?;
                }
                busy
            };
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
            if let (Some(current), Some(evidence)) = (
                &mut current.snapshot_policy_evidence,
                &evidence.snapshot_policy_evidence,
            ) {
                current.completed |= evidence.completed;
                current.plugin_success |= evidence.plugin_success;
            }
            if let (Some(current_plan), Some(received)) = (
                &mut current.management_network_plan,
                &evidence.management_network_plan,
            ) {
                if current_plan.task_uuid.is_some()
                    && received.task_uuid.is_some()
                    && current_plan.task_uuid != received.task_uuid
                {
                    return Err(Error::JournalChanged);
                }
                if current_plan.task_uuid.is_none() {
                    current_plan.task_uuid = received.task_uuid;
                }
                current_plan.completed |= received.completed;
                current_plan.ended |= received.ended;
                current_plan.configuration_observed |= received.configuration_observed;
                current_plan.route_verified |= received.route_verified;
                current.management_network = Some(current_plan.evidence());
            }
            current.pool_completed |= evidence.pool_completed;
            if let (Some(current_plan), Some(evidence_plan)) =
                (&mut current.cross_pool_plan, &evidence.cross_pool_plan)
            {
                current_plan.merge(evidence_plan)?;
                for id in &evidence.transfer_created.vms {
                    if !current.transfer_created.vms.contains(id) {
                        current.transfer_created.vms.push(*id);
                    }
                }
                for id in &evidence.transfer_created.disks {
                    if !current.transfer_created.disks.contains(id) {
                        current.transfer_created.disks.push(*id);
                    }
                }
                for id in &evidence.transfer_created.vbds {
                    if !current.transfer_created.vbds.contains(id) {
                        current.transfer_created.vbds.push(*id);
                    }
                }
                for id in &evidence.transfer_created.vifs {
                    if !current.transfer_created.vifs.contains(id) {
                        current.transfer_created.vifs.push(*id);
                    }
                }
            }
            current.appliance_completed |= evidence.appliance_completed;
            if !current.state.terminal()
                && !current.interrupted
                && current.state != OperationState::OutcomeUnknown
            {
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
    pub(crate) fn host_evidence(
        &self,
        evidence: &ResourceOperationRecord,
        index: usize,
    ) -> Result<ResourceOperationRecord, Error> {
        for _ in 0..16 {
            let mut current = self.resource_load(evidence.id)?.ok_or(Error::Journal)?;
            if current.pool != evidence.pool
                || current.target != evidence.target
                || current.action != evidence.action
            {
                return Err(Error::OperationConflict);
            }
            let received = evidence
                .host_plan
                .as_ref()
                .and_then(|p| p.steps.get(index))
                .ok_or(Error::Journal)?;
            let step = current
                .host_plan
                .as_mut()
                .and_then(|p| p.steps.get_mut(index))
                .ok_or(Error::Journal)?;
            if step.kind != received.kind
                || (step.task.is_some() && received.task.is_some() && step.task != received.task)
            {
                return Err(Error::JournalChanged);
            }
            if step.task.is_none() {
                step.task = received.task.clone();
            }
            step.accepted |= received.accepted;
            step.task_succeeded |= received.task_succeeded;
            if !current.state.terminal()
                && !current.interrupted
                && current.state != OperationState::OutcomeUnknown
            {
                current.state = if step.task.is_some() && !step.task_succeeded {
                    OperationState::Tracking
                } else {
                    OperationState::Verifying
                };
            }
            match self.resource_save(&mut current) {
                Ok(()) => return Ok(current),
                Err(Error::JournalChanged) => continue,
                Err(e) => return Err(e),
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
            .prepare("SELECT id FROM resource_operations ORDER BY rowid DESC LIMIT 1500")
            .map_err(|_| Error::Journal)?;
        let ids = query
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|_| Error::Journal)?;
        let mut records = Vec::new();
        for id in ids {
            let id: OperationId =
                serde_json::from_value(serde_json::Value::String(id.map_err(|_| Error::Journal)?))
                    .map_err(|_| Error::Journal)?;
            if let Some(record) = self.resource_load(id)?
                && record.applies_to_pool(pool)
            {
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
                .filter(|record| record.applies_to_pool(pool))
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
                    record.detail = if matches!(record.action, ResourceAction::PowerBatch(_)) { "Stop requested; the current VM task will finish before remaining VMs are skipped" } else { "Cancellation requested; server work may still continue" }.into();
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
        self.resource_recover_filtered(pool, false)
    }
    pub(crate) fn resource_recover_exports(
        &self,
        pool: PoolId,
    ) -> Result<Vec<ResourceOperationRecord>, Error> {
        self.resource_recover_filtered(pool, true)
    }
    fn resource_recover_filtered(
        &self,
        pool: PoolId,
        exports_only: bool,
    ) -> Result<Vec<ResourceOperationRecord>, Error> {
        let mut rows = self.resource_history(pool)?;
        if exports_only {
            rows.retain(|r| r.action.read_only_export())
        }
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

fn cross_created_contains(new: &crate::TransferCreated, old: &crate::TransferCreated) -> bool {
    old.vms.iter().all(|id| new.vms.contains(id))
        && old.disks.iter().all(|id| new.disks.contains(id))
        && old.vbds.iter().all(|id| new.vbds.contains(id))
        && old.vifs.iter().all(|id| new.vifs.contains(id))
}

impl Journal {
    pub(crate) fn topology_evidence(
        &self,
        evidence: &ResourceOperationRecord,
        index: usize,
    ) -> Result<ResourceOperationRecord, Error> {
        let incoming = evidence
            .network_topology_plan
            .as_ref()
            .and_then(|p| p.steps.get(index))
            .ok_or(Error::Journal)?;
        for _ in 0..16 {
            let mut current = self.resource_load(evidence.id)?.ok_or(Error::Journal)?;
            if current.pool != evidence.pool
                || current.target != evidence.target
                || current.action != evidence.action
            {
                return Err(Error::OperationConflict);
            }
            let plan = current
                .network_topology_plan
                .as_mut()
                .ok_or(Error::Journal)?;
            let step = plan.steps.get_mut(index).ok_or(Error::Journal)?;
            if step.kind != incoming.kind {
                return Err(Error::JournalChanged);
            }
            macro_rules! receipt {
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
            receipt!(task);
            receipt!(task_uuid);
            receipt!(result);
            receipt!(created);
            step.completed |= incoming.completed;
            step.ended |= incoming.ended;
            step.cancel_sent |= incoming.cancel_sent;
            let completed = step.completed;
            current.topology_changes = plan.changes();
            if !current.state.terminal() && !current.interrupted {
                current.state = if completed {
                    OperationState::Verifying
                } else {
                    OperationState::Tracking
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
}
