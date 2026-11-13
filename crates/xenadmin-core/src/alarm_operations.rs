use super::{
    ResourceAction, ResourceOperationRecord, ResourceTarget,
    xapi::{Check, Step},
};
use crate::{AlarmOperation, Error, HostId, alarm_policies, xapi::Session};
use serde_json::{Value, json};
use xenadmin_xapi::records::{HostMetricsRecord, HostRecord, VmRecord};

async fn read(
    session: &Session,
    record: &ResourceOperationRecord,
) -> Result<(String, Value), Error> {
    let ResourceAction::Alarm(operation) = &record.action else {
        return Err(Error::InvalidResourceCommand);
    };
    operation.validate(record.target)?;
    let (reference, fresh) = super::xapi::read(session, record.target).await?;
    if let AlarmOperation::SetPolicy(change) = operation {
        if let ResourceTarget::Vm(_) = record.target {
            if VmRecord::new(&fresh).is_a_template() != false
                || VmRecord::new(&fresh).is_a_snapshot() != false
            {
                return Err(Error::UnavailableAction);
            }
            if let Some(host) = change.control_domain_host {
                let (host_ref, host_record) =
                    super::xapi::read(session, ResourceTarget::Host(host)).await?;
                if VmRecord::new(&fresh).is_control_domain() != true
                    || VmRecord::new(&fresh).resident_on() != host_ref
                    || HostRecord::new(&host_record).control_domain() != reference
                {
                    return Err(Error::Stale);
                }
            } else if VmRecord::new(&fresh).is_control_domain() != false {
                return Err(Error::UnavailableAction);
            }
        } else if change.control_domain_host.is_some() {
            return Err(Error::InvalidResourceCommand);
        }
    }
    if let ResourceTarget::Pool(pool) = record.target
        && pool != record.pool
    {
        return Err(Error::Stale);
    }
    Ok((reference, fresh))
}
async fn refresh_host(session: &Session, host: HostId) -> Result<String, Error> {
    let (reference, fresh) = super::xapi::read(session, ResourceTarget::Host(host)).await?;
    let metrics = super::xapi::reply_reference(HostRecord::new(&fresh).metrics())?;
    let metrics = session
        .call("host_metrics.get_record", vec![json!(metrics)])
        .await?;
    if HostMetricsRecord::new(&metrics).live() != true {
        return Err(Error::UnavailableAction);
    }
    Ok(reference)
}
pub(super) fn checks(record: &ResourceOperationRecord) -> Vec<Check> {
    match &record.action {
        ResourceAction::Alarm(AlarmOperation::Refresh { hosts }) => {
            hosts.iter().copied().map(Check::AlarmRefresh).collect()
        }
        ResourceAction::Alarm(_) => vec![Check::Alarm],
        _ => vec![],
    }
}
pub(super) async fn prepare(
    session: &Session,
    record: &ResourceOperationRecord,
) -> Result<Vec<Step>, Error> {
    let (_, fresh) = read(session, record).await?;
    let ResourceAction::Alarm(operation) = &record.action else {
        return Err(Error::InvalidResourceCommand);
    };
    match operation {
        AlarmOperation::SetPolicy(change) => {
            alarm_policies::changed_xml(record.target, &fresh, change)?;
        }
        AlarmOperation::SetEmail {
            expected_revision,
            settings,
        } => {
            alarm_policies::changed_email(&fresh, expected_revision, settings)?;
        }
        AlarmOperation::Refresh { hosts } => {
            for host in hosts {
                refresh_host(session, *host).await?;
            }
        }
    }
    Ok(checks(record)
        .into_iter()
        .map(|check| Step {
            label: match check {
                Check::AlarmRefresh(host) => format!("Request refresh on {host}"),
                _ => operation.label().into(),
            },
            method: match check {
                Check::AlarmRefresh(_) => "host.call_plugin".into(),
                _ => format!("{}.set_other_config", record.target.class()),
            },
            argument: None,
            expected: Value::Null,
            check,
        })
        .collect())
}
pub(super) async fn arguments(
    session: &Session,
    record: &ResourceOperationRecord,
    check: &Check,
) -> Result<Vec<Value>, Error> {
    let (reference, fresh) = read(session, record).await?;
    let ResourceAction::Alarm(operation) = &record.action else {
        return Err(Error::InvalidResourceCommand);
    };
    let value = match (operation, check) {
        (AlarmOperation::SetPolicy(change), Check::Alarm) => {
            let xml = alarm_policies::changed_xml(record.target, &fresh, change)?;
            let mut map = fresh["other_config"]
                .as_object()
                .cloned()
                .ok_or(Error::ResourceConflict)?;
            map.insert(alarm_policies::KEY.into(), json!(xml));
            Value::Object(map)
        }
        (
            AlarmOperation::SetEmail {
                expected_revision,
                settings,
            },
            Check::Alarm,
        ) => alarm_policies::changed_email(&fresh, expected_revision, settings)?,
        (AlarmOperation::Refresh { hosts }, Check::AlarmRefresh(host)) if hosts.contains(host) => {
            let host = refresh_host(session, *host).await?;
            return Ok(vec![
                json!(host),
                json!("perfmon"),
                json!("refresh"),
                json!({}),
            ]);
        }
        _ => return Err(Error::InvalidResourceCommand),
    };
    Ok(vec![json!(reference), value])
}
pub(super) async fn verify(
    session: &Session,
    record: &ResourceOperationRecord,
    check: &Check,
) -> Result<Option<ResourceTarget>, Error> {
    let (_, fresh) = read(session, record).await?;
    let ResourceAction::Alarm(operation) = &record.action else {
        return Err(Error::InvalidResourceCommand);
    };
    let matched = match (operation, check) {
        (AlarmOperation::SetPolicy(change), Check::Alarm) => {
            let policy = alarm_policies::project(record.target, change.control_domain_host, &fresh);
            policy.error.is_none()
                && (change.rule.is_none() || !policy.customized.contains(&change.metric))
                && policy.rules.iter().find(|r| r.metric == change.metric) == change.rule.as_ref()
        }
        (AlarmOperation::SetEmail { settings, .. }, Check::Alarm) => {
            alarm_policies::project_email(&fresh).settings.as_ref() == Some(settings)
        }
        (AlarmOperation::Refresh { hosts }, Check::AlarmRefresh(host)) => {
            hosts.contains(host) && record.alarm_refresh_receipts.contains(host)
        }
        _ => false,
    };
    if matched {
        Ok(None)
    } else {
        Err(Error::ResourceConflict)
    }
}

pub(super) fn validate_receipts(record: &ResourceOperationRecord) -> Result<(), Error> {
    if record.alarm_refresh_receipts.is_empty() {
        return Ok(());
    }
    let ResourceAction::Alarm(AlarmOperation::Refresh { hosts }) = &record.action else {
        return Err(Error::Journal);
    };
    if record.alarm_refresh_receipts.len() > 64
        || record
            .alarm_refresh_receipts
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != record.alarm_refresh_receipts.len()
        || record.alarm_refresh_receipts.iter().any(|host| {
            hosts
                .iter()
                .position(|reviewed| reviewed == host)
                .is_none_or(|index| {
                    record
                        .steps
                        .get(index)
                        .is_none_or(|step| step.state == crate::StepState::Pending)
                })
        })
    {
        return Err(Error::Journal);
    }
    Ok(())
}
