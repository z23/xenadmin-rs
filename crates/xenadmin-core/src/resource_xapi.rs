use super::*;
use crate::xapi::Session;
use serde_json::{Value, json};

pub(super) const OPERATION_TAG: &str = "xenadmin-rs:operation";
#[derive(Clone)]
pub(super) enum Check {
    Alarm,
    AlarmRefresh(HostId),
    SnapshotPolicy(Box<super::snapshot_policy::Verify>),
    Appliance(super::appliance::Verify),
    Pool,
    Organization(crate::OrganizationEdit),
    Network,
    Field(&'static str, Value),
    DefaultSr(SrId),
    CreatedNetwork,
    Deleted,
    Scan,
    TaskCancellation,
    MessageDeleted,
    Storage,
}
pub(super) struct Step {
    pub label: String,
    pub method: String,
    pub argument: Option<Value>,
    pub expected: Value,
    pub check: Check,
}

fn reference(value: &Value) -> Result<String, Error> {
    value
        .as_str()
        .filter(|value| {
            value.starts_with("OpaqueRef:")
                && *value != "OpaqueRef:NULL"
                && value.len() <= 512
                && value.bytes().all(|byte| byte.is_ascii_graphic())
        })
        .map(str::to_owned)
        .ok_or(Error::Protocol("invalid resource reference"))
}
pub(super) async fn read(
    session: &Session,
    target: ResourceTarget,
) -> Result<(String, Value), Error> {
    let class = target.class();
    let value = session
        .call(
            &format!("{class}.get_by_uuid"),
            vec![json!(target.uuid().to_string())],
        )
        .await?;
    let reference = reference(&value)?;
    let record = session
        .call(&format!("{class}.get_record"), vec![json!(&reference)])
        .await?;
    if record["uuid"].as_str() != Some(target.uuid().to_string().as_str()) {
        return Err(Error::Stale);
    }
    Ok((reference, record))
}
fn empty_network(record: &Value) -> Result<(), Error> {
    for field in ["PIFs", "VIFs"] {
        match record[field].as_array() {
            Some(values) if values.is_empty() => {}
            Some(_) => return Err(Error::ResourceInUse),
            None => return Err(Error::Protocol("network interface membership unavailable")),
        }
    }
    Ok(())
}
fn number(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| value.as_str()?.parse().ok())
}
fn same_field(actual: &Value, expected: &Value, field: &str) -> bool {
    if field == "MTU" {
        number(actual).is_some() && number(actual) == number(expected)
    } else {
        actual == expected
    }
}
async fn default_sr(session: &Session, record: &Value) -> Result<Option<SrId>, Error> {
    if record["default_SR"] == "OpaqueRef:NULL" {
        return Ok(None);
    }
    let reference = reference(&record["default_SR"])?;
    let sr = session
        .call("SR.get_record", vec![json!(reference)])
        .await?;
    let id = sr["uuid"]
        .as_str()
        .ok_or(Error::Protocol("default storage UUID unavailable"))?;
    SrId::parse(id)
        .map(Some)
        .map_err(|_| Error::Protocol("invalid default storage UUID"))
}
fn field_step(
    target: ResourceTarget,
    field: &'static str,
    expected: Value,
    value: Value,
    label: &str,
) -> Step {
    Step {
        label: label.into(),
        method: format!("{}.set_{field}", target.class()),
        argument: Some(value.clone()),
        expected,
        check: Check::Field(field, value),
    }
}
pub(super) async fn prepare(
    session: &Session,
    record: &mut ResourceOperationRecord,
) -> Result<Vec<Step>, Error> {
    if matches!(record.action, ResourceAction::SnapshotPolicy(_)) {
        return super::snapshot_policy::prepare(session, record).await;
    }
    if matches!(record.action, ResourceAction::Appliance(_)) {
        return super::appliance::prepare(session, record).await;
    }
    if matches!(record.action, ResourceAction::Pool(_)) {
        return super::pool::prepare(session, record).await;
    }
    if matches!(record.action, ResourceAction::Network(_)) {
        return super::network::prepare(session, record).await;
    }
    if matches!(record.action, ResourceAction::Storage(_)) {
        return super::storage::prepare(session, record).await;
    }
    let (_, fresh) = read(session, record.target).await?;
    let mut steps = Vec::new();
    match &record.action {
        ResourceAction::SnapshotPolicy(_) => unreachable!(),
        ResourceAction::Appliance(_) => unreachable!(),
        ResourceAction::Pool(_) => unreachable!(),
        ResourceAction::AuthenticatedStorage { .. }
        | ResourceAction::CleanupStorageCredentials { .. }
        | ResourceAction::RootPassword(_)
        | ResourceAction::HostPower(_)
        | ResourceAction::Host(_)
        | ResourceAction::NetworkTopology(_)
        | ResourceAction::ManagementNetwork(_)
        | ResourceAction::PowerBatch(_) => return Err(Error::InvalidResourceCommand),
        ResourceAction::Alarm(_) => return super::alarms::prepare(session, record).await,
        ResourceAction::Organization(_) => {
            return super::organization::prepare(session, record).await;
        }
        ResourceAction::Network(_) => return super::network::prepare(session, record).await,
        ResourceAction::Transfer(_)
        | ResourceAction::CrossPoolVm(_)
        | ResourceAction::CloudInit(_) => {
            return Err(Error::TransferConflict);
        }
        ResourceAction::CancelTask => {
            cancellable_task(&fresh)?;
            steps.push(Step {
                label: "Request task cancellation".into(),
                method: "task.cancel".into(),
                argument: None,
                expected: Value::Null,
                check: Check::TaskCancellation,
            });
        }
        ResourceAction::DismissMessage { expected_digest } => {
            if crate::xapi::decode_server_message(&fresh)?.review_digest() != *expected_digest {
                return Err(Error::ResourceConflict);
            }
            steps.push(Step {
                label: "Dismiss server message".into(),
                method: "message.destroy".into(),
                argument: None,
                expected: Value::Null,
                check: Check::MessageDeleted,
            });
        }
        ResourceAction::Storage(_) => unreachable!(),
        ResourceAction::RenameMetadata {
            expected_name,
            expected_description,
            name,
            description,
        } => {
            if fresh["name_label"] != *expected_name
                || fresh["name_description"] != *expected_description
            {
                return Err(Error::ResourceConflict);
            }
            if expected_name != name {
                steps.push(field_step(
                    record.target,
                    "name_label",
                    json!(expected_name),
                    json!(name),
                    "Set name",
                ));
            }
            if expected_description != description {
                steps.push(field_step(
                    record.target,
                    "name_description",
                    json!(expected_description),
                    json!(description),
                    "Set description",
                ));
            }
        }
        ResourceAction::CreateInternalNetwork { .. } => {
            let networks = session.call("network.get_all_records", vec![]).await?;
            let networks = networks
                .as_object()
                .filter(|records| records.len() <= 10_000)
                .ok_or(Error::Protocol("network baseline unavailable"))?;
            record.baseline_networks = Some(
                networks
                    .values()
                    .map(|record| {
                        record["uuid"]
                            .as_str()
                            .and_then(|value| NetworkId::parse(value).ok())
                            .ok_or(Error::Protocol("invalid network baseline UUID"))
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            );
            steps.push(Step {
                label: "Create internal network".into(),
                method: "network.create".into(),
                argument: None,
                expected: Value::Null,
                check: Check::CreatedNetwork,
            });
        }
        ResourceAction::ConfigureInternalNetwork { expected_mtu, mtu } => {
            empty_network(&fresh)?;
            if number(&fresh["MTU"]) != Some(*expected_mtu) {
                return Err(Error::ResourceConflict);
            }
            steps.push(field_step(
                record.target,
                "MTU",
                json!(expected_mtu),
                json!(mtu.to_string()),
                "Set MTU",
            ));
        }
        ResourceAction::DeleteInternalNetwork { expected_name } => {
            empty_network(&fresh)?;
            if fresh["name_label"] != *expected_name {
                return Err(Error::ResourceConflict);
            }
            steps.push(Step {
                label: "Delete internal network".into(),
                method: "network.destroy".into(),
                argument: None,
                expected: json!(expected_name),
                check: Check::Deleted,
            });
        }
        ResourceAction::ScanStorage => {
            if !fresh["allowed_operations"]
                .as_array()
                .is_some_and(|ops| ops.iter().any(|op| op == "scan"))
            {
                return Err(Error::UnavailableAction);
            }
            steps.push(Step {
                label: "Scan storage".into(),
                method: "Async.SR.scan".into(),
                argument: None,
                expected: Value::Null,
                check: Check::Scan,
            });
        }
        ResourceAction::SetHostEnabled {
            expected_enabled,
            enabled,
        } => {
            if fresh["enabled"] != *expected_enabled {
                return Err(Error::ResourceConflict);
            }
            steps.push(Step {
                label: if *enabled {
                    "Enable host"
                } else {
                    "Disable host"
                }
                .into(),
                method: if *enabled {
                    "host.enable"
                } else {
                    "host.disable"
                }
                .into(),
                argument: None,
                expected: json!(expected_enabled),
                check: Check::Field("enabled", json!(enabled)),
            });
        }
        ResourceAction::SetDefaultStorage {
            sr,
            expected_default,
        } => {
            if default_sr(session, &fresh).await? != *expected_default {
                return Err(Error::ResourceConflict);
            }
            let (_, sr_record) = read(session, ResourceTarget::Storage(*sr)).await?;
            if sr_record["content_type"] != "user" {
                return Err(Error::InvalidResourceCommand);
            }
            steps.push(Step {
                label: "Set default storage".into(),
                method: "pool.set_default_SR".into(),
                argument: None,
                expected: json!(expected_default),
                check: Check::DefaultSr(*sr),
            });
        }
    }
    Ok(steps)
}
pub(super) async fn arguments(
    session: &Session,
    record: &ResourceOperationRecord,
    step: &Step,
) -> Result<Vec<Value>, Error> {
    if let Check::SnapshotPolicy(check) = &step.check {
        return super::snapshot_policy::arguments(session, record, check).await;
    }
    if let Check::Appliance(check) = &step.check {
        return super::appliance::arguments(session, record, check).await;
    }
    if matches!(step.check, Check::Pool) {
        return super::pool::arguments(session, record).await;
    }
    if matches!(step.check, Check::Storage) {
        return super::storage::arguments(session, record).await;
    }
    if matches!(step.check, Check::Network) {
        return super::network::arguments(session, record).await;
    }
    let (reference, fresh) = read(session, record.target).await?;
    match &step.check {
        Check::Alarm | Check::AlarmRefresh(_) => {
            super::alarms::arguments(session, record, &step.check).await
        }
        Check::Appliance(_) => unreachable!(),
        Check::Pool => super::pool::arguments(session, record).await,
        Check::Organization(edit) => super::organization::arguments(session, record, edit).await,
        Check::Network => super::network::arguments(session, record).await,
        Check::TaskCancellation => {
            cancellable_task(&fresh)?;
            Ok(vec![json!(reference)])
        }
        Check::MessageDeleted => {
            let ResourceAction::DismissMessage { expected_digest } = &record.action else {
                return Err(Error::InvalidResourceCommand);
            };
            if crate::xapi::decode_server_message(&fresh)?.review_digest() != *expected_digest {
                return Err(Error::ResourceConflict);
            }
            Ok(vec![json!(reference)])
        }
        Check::Storage => super::storage::arguments(session, record).await,
        Check::CreatedNetwork => {
            let ResourceAction::CreateInternalNetwork {
                name,
                description,
                mtu,
            } = &record.action
            else {
                return Err(Error::InvalidResourceCommand);
            };
            Ok(vec![
                json!({"name_label":name,"name_description":description,"MTU":mtu.to_string(),"other_config":{OPERATION_TAG:record.id.to_string()},"bridge":""}),
            ])
        }
        Check::Field(field, _) => {
            if *field == "MTU" {
                empty_network(&fresh)?;
            }
            if !same_field(&fresh[*field], &step.expected, field) {
                return Err(Error::ResourceConflict);
            }
            let mut args = vec![json!(reference)];
            if let Some(value) = &step.argument {
                args.push(value.clone());
            }
            Ok(args)
        }
        Check::DefaultSr(sr) => {
            let ResourceAction::SetDefaultStorage {
                expected_default, ..
            } = &record.action
            else {
                return Err(Error::InvalidResourceCommand);
            };
            if default_sr(session, &fresh).await? != *expected_default {
                return Err(Error::ResourceConflict);
            }
            let (sr_reference, sr_record) = read(session, ResourceTarget::Storage(*sr)).await?;
            if sr_record["content_type"] != "user" {
                return Err(Error::InvalidResourceCommand);
            }
            Ok(vec![json!(reference), json!(sr_reference)])
        }
        Check::Deleted => {
            empty_network(&fresh)?;
            if fresh["name_label"] != step.expected {
                return Err(Error::ResourceConflict);
            }
            Ok(vec![json!(reference)])
        }
        Check::SnapshotPolicy(_) => unreachable!(),
        Check::Scan => {
            if !fresh["allowed_operations"]
                .as_array()
                .is_some_and(|ops| ops.iter().any(|op| op == "scan"))
            {
                return Err(Error::UnavailableAction);
            }
            Ok(vec![json!(reference)])
        }
    }
}
pub(super) fn reply_reference(value: &Value) -> Result<String, Error> {
    reference(value)
}
fn cancellable_task(record: &Value) -> Result<(), Error> {
    if record["status"] != "pending"
        || !record["allowed_operations"]
            .as_array()
            .is_some_and(|ops| ops.iter().any(|op| op == "cancel"))
    {
        return Err(Error::UnavailableAction);
    }
    Ok(())
}
pub(super) fn reconciliation_steps(record: &ResourceOperationRecord) -> Vec<Check> {
    match &record.action {
        ResourceAction::SnapshotPolicy(_) => super::snapshot_policy::reconciliation_checks(record),
        ResourceAction::Appliance(_) => super::appliance::reconciliation_checks(record),
        ResourceAction::Pool(_) => vec![Check::Pool],
        ResourceAction::AuthenticatedStorage { .. }
        | ResourceAction::CleanupStorageCredentials { .. }
        | ResourceAction::RootPassword(_)
        | ResourceAction::HostPower(_)
        | ResourceAction::Host(_)
        | ResourceAction::NetworkTopology(_)
        | ResourceAction::ManagementNetwork(_)
        | ResourceAction::PowerBatch(_) => Vec::new(),
        ResourceAction::Alarm(_) => super::alarms::checks(record),
        ResourceAction::Organization(_) => super::organization::checks(record),
        ResourceAction::Network(_) => vec![Check::Network],
        ResourceAction::Transfer(_)
        | ResourceAction::CrossPoolVm(_)
        | ResourceAction::CloudInit(_) => vec![],
        ResourceAction::CancelTask => vec![Check::TaskCancellation],
        ResourceAction::DismissMessage { .. } => vec![Check::MessageDeleted],
        ResourceAction::Storage(_) => vec![Check::Storage],
        ResourceAction::RenameMetadata {
            expected_name,
            expected_description,
            name,
            description,
        } => {
            let mut result = Vec::new();
            if name != expected_name {
                result.push(Check::Field("name_label", json!(name)));
            }
            if description != expected_description {
                result.push(Check::Field("name_description", json!(description)));
            }
            result
        }
        ResourceAction::CreateInternalNetwork { .. } => vec![Check::CreatedNetwork],
        ResourceAction::ConfigureInternalNetwork { mtu, .. } => {
            vec![Check::Field("MTU", json!(mtu))]
        }
        ResourceAction::DeleteInternalNetwork { .. } => vec![Check::Deleted],
        ResourceAction::ScanStorage => vec![Check::Scan],
        ResourceAction::SetHostEnabled { enabled, .. } => {
            vec![Check::Field("enabled", json!(enabled))]
        }
        ResourceAction::SetDefaultStorage { sr, .. } => vec![Check::DefaultSr(*sr)],
    }
}
pub(super) async fn verify(
    session: &Session,
    record: &ResourceOperationRecord,
    check: &Check,
) -> Result<Option<ResourceTarget>, Error> {
    if matches!(check, Check::Alarm | Check::AlarmRefresh(_)) {
        return super::alarms::verify(session, record, check).await;
    }
    if let Check::SnapshotPolicy(check) = check {
        return super::snapshot_policy::verify(session, record, check).await;
    }
    if let Check::Appliance(check) = check {
        return super::appliance::verify(session, record, check).await;
    }
    if matches!(check, Check::Pool) {
        return super::pool::verify(session, record).await;
    }
    if let Check::Organization(edit) = check {
        return super::organization::verify(session, edit).await;
    }
    if matches!(check, Check::Network) {
        return super::network::verify(session, record).await;
    }
    if matches!(check, Check::Storage) {
        return super::storage::verify(session, record).await;
    }
    if matches!(check, Check::CreatedNetwork) {
        return verify_created(session, record).await.map(Some);
    }
    if matches!(check, Check::Deleted | Check::MessageDeleted) {
        return match read(session, record.target).await {
            Err(Error::Api(code)) if code == "UUID_INVALID" => Ok(None),
            Ok(_) => Err(Error::ResourceConflict),
            Err(error) => Err(error),
        };
    }
    let (_, fresh) = read(session, record.target).await?;
    let matches = match check {
        Check::Field(field, value) => same_field(&fresh[*field], value, field),
        Check::DefaultSr(sr) => default_sr(session, &fresh).await? == Some(*sr),
        Check::Scan => true,
        _ => false,
    };
    if matches {
        Ok(None)
    } else {
        Err(Error::ResourceConflict)
    }
}
async fn verify_created(
    session: &Session,
    record: &ResourceOperationRecord,
) -> Result<ResourceTarget, Error> {
    let ResourceAction::CreateInternalNetwork {
        name,
        description,
        mtu,
    } = &record.action
    else {
        return Err(Error::InvalidResourceCommand);
    };
    let baseline = record
        .baseline_networks
        .as_ref()
        .ok_or(Error::Protocol("network creation baseline missing"))?;
    let fresh = if let Some(reference) = &record.result_reference {
        session
            .call("network.get_record", vec![json!(reference)])
            .await?
    } else {
        let records = session.call("network.get_all_records", vec![]).await?;
        let records = records
            .as_object()
            .filter(|records| records.len() <= 10_000)
            .ok_or(Error::Protocol("network reconciliation unavailable"))?;
        let candidates = records
            .values()
            .filter(|candidate| candidate["other_config"][OPERATION_TAG] == record.id.to_string())
            .collect::<Vec<_>>();
        if candidates.len() != 1 {
            return Err(Error::Protocol(
                "created network is not uniquely identified",
            ));
        }
        candidates[0].clone()
    };
    let id = fresh["uuid"]
        .as_str()
        .and_then(|value| NetworkId::parse(value).ok())
        .ok_or(Error::Protocol("created network UUID unavailable"))?;
    if baseline.contains(&id)
        || fresh["other_config"][OPERATION_TAG] != record.id.to_string()
        || fresh["name_label"] != *name
        || fresh["name_description"] != *description
        || number(&fresh["MTU"]) != Some(*mtu)
    {
        return Err(Error::ResourceConflict);
    }
    empty_network(&fresh)?;
    Ok(ResourceTarget::Network(id))
}
