use super::{
    ResourceAction, ResourceOperationRecord, ResourceTarget,
    xapi::{Check, Step},
};
use crate::{Error, HostCrashdump, HostId, OperationState, PoolId, StepState, xapi::Session};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub(super) const COMPLETION: &str = "Host accepted deletion and the crashdump record is absent; host filesystem contents were not independently inspected";
const LABEL: &str = "Delete retained host crashdump";
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Plan {
    pool: PoolId,
    host: HostId,
    dump: HostCrashdump,
    pub(super) dispatched: bool,
    pub(super) completed: bool,
}
impl Plan {
    pub(crate) fn extends(&self, old: &Self) -> bool {
        self.pool == old.pool
            && self.host == old.host
            && self.dump == old.dump
            && (!old.dispatched || self.dispatched)
            && (!old.completed || self.completed)
    }
    pub(super) fn merge(&mut self, other: &Self) -> Result<(), Error> {
        if self.pool != other.pool
            || self.host != other.host
            || self.dump != other.dump
            || other.dispatched && !self.dispatched
        {
            return Err(Error::JournalChanged);
        }
        self.completed |= other.completed;
        Ok(())
    }
}
pub(crate) fn validate_command(target: ResourceTarget, dump: &HostCrashdump) -> Result<(), Error> {
    if !matches!(target, ResourceTarget::Host(host) if !host.uuid().is_nil())
        || dump.uuid.uuid().is_nil()
        || dump.timestamp.is_empty()
        || dump.timestamp.len() > 64
        || dump.timestamp.chars().any(char::is_control)
    {
        return Err(Error::InvalidResourceCommand);
    }
    Ok(())
}
pub(crate) fn validate_record(record: &ResourceOperationRecord) -> Result<(), Error> {
    let ResourceAction::DeleteHostCrashdump(dump) = &record.action else {
        return if record.crashdump_deletion.is_none() {
            Ok(())
        } else {
            Err(Error::Journal)
        };
    };
    validate_command(record.target, dump).map_err(|_| Error::Journal)?;
    let Some(p) = &record.crashdump_deletion else {
        return if record.steps.is_empty()
            && matches!(
                record.state,
                OperationState::Preparing
                    | OperationState::Rejected
                    | OperationState::Cancelled
                    | OperationState::OutcomeUnknown
                    | OperationState::AcknowledgedUnknown
            ) {
            Ok(())
        } else {
            Err(Error::Journal)
        };
    };
    if p.pool != record.pool
        || record.target != ResourceTarget::Host(p.host)
        || &p.dump != dump
        || p.completed && !p.dispatched
        || (p.dispatched || !record.steps.is_empty())
            && (record.steps.len() != 1 || record.steps[0].label != LABEL)
        || record
            .steps
            .first()
            .is_some_and(|s| s.state == StepState::Verified)
            && !p.completed
        || record.state == OperationState::Succeeded
            && (!p.completed
                || record
                    .steps
                    .first()
                    .is_none_or(|s| s.state != StepState::Verified))
    {
        return Err(Error::Journal);
    }
    Ok(())
}
async fn host(
    session: &Session,
    record: &ResourceOperationRecord,
    require_live: bool,
) -> Result<String, Error> {
    let ResourceTarget::Host(host) = record.target else {
        return Err(Error::InvalidResourceCommand);
    };
    let (reference, fresh) = crate::host_support::host_record(session, record.pool, host).await?;
    if require_live {
        if fresh["current_operations"]
            .as_object()
            .is_none_or(|v| !v.is_empty())
        {
            return Err(Error::UnavailableAction);
        }
        let metrics = super::xapi::reply_reference(&fresh["metrics"])?;
        let metrics = session
            .call("host_metrics.get_record", vec![json!(metrics)])
            .await?;
        if metrics["live"] != true {
            return Err(Error::UnavailableAction);
        }
    }
    Ok(reference)
}
async fn selected(session: &Session, record: &ResourceOperationRecord) -> Result<String, Error> {
    let ResourceAction::DeleteHostCrashdump(expected) = &record.action else {
        return Err(Error::InvalidResourceCommand);
    };
    validate_command(record.target, expected)?;
    let owner = host(session, record, true).await?;
    let (reference, fresh) =
        crate::snapshots::optional_record(session, "host_crashdump", &expected.uuid.to_string())
            .await?
            .ok_or(Error::Stale)?;
    if fresh["host"] != owner || crate::host_support::decode_crashdump(&fresh)? != *expected {
        return Err(Error::ResourceConflict);
    }
    Ok(reference)
}
pub(super) async fn prepare(
    session: &Session,
    record: &mut ResourceOperationRecord,
) -> Result<Vec<Step>, Error> {
    selected(session, record).await?;
    let ResourceTarget::Host(host) = record.target else {
        return Err(Error::InvalidResourceCommand);
    };
    let ResourceAction::DeleteHostCrashdump(dump) = &record.action else {
        return Err(Error::InvalidResourceCommand);
    };
    record.crashdump_deletion = Some(Box::new(Plan {
        pool: record.pool,
        host,
        dump: dump.clone(),
        dispatched: false,
        completed: false,
    }));
    Ok(vec![Step {
        label: LABEL.into(),
        method: "host_crashdump.destroy".into(),
        argument: None,
        expected: Value::Null,
        check: Check::HostCrashdumpDeletion,
    }])
}
pub(super) async fn arguments(
    session: &Session,
    record: &ResourceOperationRecord,
) -> Result<Vec<Value>, Error> {
    validate_record(record)?;
    if record
        .crashdump_deletion
        .as_ref()
        .is_none_or(|p| p.dispatched)
    {
        return Err(Error::ResourceConflict);
    }
    Ok(vec![json!(selected(session, record).await?)])
}
pub(super) async fn verify(
    session: &Session,
    record: &ResourceOperationRecord,
) -> Result<Option<ResourceTarget>, Error> {
    validate_record(record)?;
    let p=record.crashdump_deletion.as_ref().filter(|p|p.dispatched&&p.completed).ok_or(Error::Protocol("Crashdump deletion completion receipt is unavailable; absence alone cannot establish filesystem deletion and no request will be replayed"))?;
    host(session, record, false).await?;
    if crate::snapshots::optional_record(session, "host_crashdump", &p.dump.uuid.to_string())
        .await?
        .is_some()
    {
        return Err(Error::ResourceConflict);
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{OperationControl, OperationId, OperationStep, operations::Journal, resources};
    fn record() -> ResourceOperationRecord {
        let pool = uuid::Uuid::from_u128(1).into();
        let host = uuid::Uuid::from_u128(2).into();
        let dump = HostCrashdump {
            uuid: uuid::Uuid::from_u128(3).into(),
            timestamp: "20261108T00:00:00Z".into(),
            size: 4096,
        };
        let (_, request) = resources::channel(
            OperationId::new(),
            crate::ObservedResource {
                pool,
                generation: 1,
                target: ResourceTarget::Host(host),
            },
            ResourceAction::DeleteHostCrashdump(dump.clone()),
            tokio::runtime::Handle::current(),
        );
        let mut row = request.record;
        row.crashdump_deletion = Some(Box::new(Plan {
            pool,
            host,
            dump,
            dispatched: false,
            completed: false,
        }));
        row.steps = vec![OperationStep {
            label: LABEL.into(),
            state: StepState::Pending,
            detail: String::new(),
        }];
        row
    }
    #[tokio::test]
    async fn crashdump_late_completion_preserves_recovery_and_acknowledgement() {
        for ack in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let journal = Journal::open(dir.path()).unwrap();
            let mut row = record();
            journal.resource_admit(&mut row).unwrap();
            row.state = OperationState::Dispatching;
            row.steps[0].state = StepState::Dispatching;
            row.crashdump_deletion.as_mut().unwrap().dispatched = true;
            journal.resource_save(&mut row).unwrap();
            let mut late = row.clone();
            journal.resource_recover(row.pool).unwrap();
            if ack {
                journal
                    .resource_control(row.pool, row.id, OperationControl::AcknowledgeUnknown)
                    .unwrap();
            }
            late.crashdump_deletion.as_mut().unwrap().completed = true;
            let merged = journal.resource_evidence(&late).unwrap();
            assert_eq!(
                merged.state,
                if ack {
                    OperationState::AcknowledgedUnknown
                } else {
                    OperationState::OutcomeUnknown
                }
            );
            assert!(merged.interrupted);
            assert!(merged.crashdump_deletion.as_ref().unwrap().completed);
            assert_ne!(merged.steps[0].state, StepState::Verified);
            let public = serde_json::to_string(&merged).unwrap();
            assert!(!public.contains("crashdump_deletion"));
            let mut cleared = merged.clone();
            cleared.crashdump_deletion.as_mut().unwrap().completed = false;
            assert!(journal.resource_save(&mut cleared).is_err());
        }
    }
    #[tokio::test]
    async fn crashdump_journal_rejects_changed_identity_or_fabricated_completion() {
        let dir = tempfile::tempdir().unwrap();
        let journal = Journal::open(dir.path()).unwrap();
        let mut row = record();
        journal.resource_admit(&mut row).unwrap();
        for change in 0..5 {
            let mut bad = row.clone();
            match change {
                0 => bad.crashdump_deletion.as_mut().unwrap().dump.size += 1,
                1 => bad.crashdump_deletion.as_mut().unwrap().completed = true,
                2 => bad.steps[0].state = StepState::Verified,
                3 => bad.state = OperationState::Succeeded,
                _ => {
                    bad.crashdump_deletion.as_mut().unwrap().host = uuid::Uuid::from_u128(99).into()
                }
            }
            assert!(journal.resource_save(&mut bad).is_err(), "case {change}");
        }
        journal.0.execute("UPDATE resource_operations SET payload=json_set(payload,'$.crashdump_deletion.completed',json('true')) WHERE id=?1", [row.id.to_string()]).unwrap();
        assert!(journal.resource_load(row.id).is_err());
        let mut other = record();
        assert!(
            journal.resource_admit(&mut other).is_err(),
            "corrupt unresolved evidence must retain the pool reservation"
        );
    }
}
