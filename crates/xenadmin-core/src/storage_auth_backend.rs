use super::{ResourceAction, ResourceOperationRecord, ResourceTarget, storage, xapi};
use crate::{
    Error, SecretId, StorageCredentials, StorageOperation,
    storage_auth::{Plan, SecretKind, SecretStep, username_digest},
    xapi::Session,
};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use xenadmin_xapi::records::{PbdRecord, SrRecord, TaskRecord};

pub(super) fn require(ok: bool, message: &'static str) -> Result<(), Error> {
    if ok {
        Ok(())
    } else {
        Err(Error::Protocol(message))
    }
}
pub(super) async fn secret_reference(session: &Session, id: SecretId) -> Result<String, Error> {
    xapi::reply_reference(
        &session
            .call("secret.get_by_uuid", vec![json!(id.to_string())])
            .await?,
    )
}
pub(super) async fn secret_uuid(session: &Session, reference: &str) -> Result<SecretId, Error> {
    let value = session
        .call("secret.get_uuid", vec![json!(reference)])
        .await?;
    value
        .as_str()
        .and_then(|v| SecretId::parse(v).ok())
        .filter(|id| !id.uuid().is_nil())
        .ok_or(Error::Protocol("Invalid Secret identity"))
}
pub(super) async fn secret_ids(session: &Session) -> Result<Vec<SecretId>, Error> {
    let values = session.call("secret.get_all", vec![]).await?;
    let values = values
        .as_array()
        .filter(|v| v.len() <= 4096)
        .ok_or(Error::Protocol(
            "Secret inventory exceeds the inspection limit",
        ))?;
    let mut ids = BTreeSet::new();
    for value in values {
        let reference = xapi::reply_reference(value)?;
        require(
            ids.insert(secret_uuid(session, &reference).await?),
            "Duplicate Secret identity",
        )?;
    }
    Ok(ids.into_iter().collect())
}
fn marker(operation: crate::OperationId, kind: SecretKind) -> String {
    format!("{operation}/{kind:?}")
}
pub(super) fn secret_record(
    operation: crate::OperationId,
    kind: SecretKind,
    password: &str,
) -> Value {
    json!({"value":password,"other_config":{xapi::OPERATION_TAG:marker(operation,kind)}})
}
pub(super) async fn owned(
    session: &Session,
    operation: crate::OperationId,
    kind: SecretKind,
    id: SecretId,
) -> Result<String, Error> {
    let reference = secret_reference(session, id).await?;
    require(
        secret_uuid(session, &reference).await? == id,
        "Secret identity changed",
    )?;
    let metadata = session
        .call("secret.get_other_config", vec![json!(reference)])
        .await?;
    require(
        metadata[xapi::OPERATION_TAG] == marker(operation, kind),
        "Secret ownership no longer matches this operation",
    )?;
    Ok(reference)
}
pub(super) async fn recover_secret(
    session: &Session,
    record: &ResourceOperationRecord,
    index: usize,
) -> Result<Option<SecretId>, Error> {
    let plan = record.storage_auth_plan.as_ref().ok_or(Error::Journal)?;
    let step = plan.secrets.get(index).ok_or(Error::Journal)?;
    let mut found = None;
    for id in secret_ids(session).await? {
        if plan.baseline.contains(&id) {
            continue;
        }
        let reference = secret_reference(session, id).await?;
        let metadata = session
            .call("secret.get_other_config", vec![json!(reference)])
            .await?;
        if metadata[xapi::OPERATION_TAG] == marker(record.id, step.kind) {
            require(
                found.is_none(),
                "Multiple Secrets match this interrupted operation",
            )?;
            found = Some(id);
        }
    }
    Ok(found)
}
pub(super) async fn prepare(
    session: &Session,
    record: &mut ResourceOperationRecord,
    credentials: &StorageCredentials,
) -> Result<(), Error> {
    let ResourceAction::AuthenticatedStorage {
        operation,
        authentication,
    } = &record.action
    else {
        return Err(Error::InvalidResourceCommand);
    };
    credentials.validate(operation)?;
    require(
        *authentication == credentials.authentication,
        "Storage credentials do not match the review",
    )?;
    let operation = operation.clone();
    storage::prepare(session, record).await?;
    let baseline = secret_ids(session).await?;
    let tasks = session.call("task.get_all_records", vec![]).await?;
    let task_baseline = tasks
        .as_object()
        .filter(|v| v.len() <= 10000)
        .ok_or(Error::Protocol("Task identity baseline unavailable"))?
        .values()
        .map(task_uuid)
        .collect::<Result<Vec<_>, _>>()?;
    record.storage_auth_plan = Some(Box::new(Plan {
        operation,
        authentication: credentials.authentication,
        baseline,
        task_baseline,
        task_uuid: None,
        secrets: credentials
            .authentication
            .kinds()
            .iter()
            .zip(&credentials.entries)
            .map(|(kind, c)| SecretStep {
                kind: *kind,
                username_digest: username_digest(&c.username),
                sent: false,
                completed: false,
                id: None,
                cleanup_sent: false,
                cleaned: false,
            })
            .collect(),
        provision_sent: false,
        provision_succeeded: false,
        verified: false,
    }));
    Ok(())
}
pub(super) async fn arguments(
    session: &Session,
    record: &ResourceOperationRecord,
    credentials: &StorageCredentials,
) -> Result<(&'static str, Vec<Value>), Error> {
    let plan = record.storage_auth_plan.as_ref().ok_or(Error::Journal)?;
    credentials.validate(&plan.operation)?;
    require(
        credentials.authentication == plan.authentication,
        "Storage credentials changed",
    )?;
    let mut args = storage::arguments(session, record).await?;
    let (method, config) = match &plan.operation {
        StorageOperation::Create { .. } => ("Async.SR.create", args.get_mut(1)),
        StorageOperation::CreateAttachment { .. } => (
            "Async.PBD.create",
            args.get_mut(0).and_then(|v| v.get_mut("device_config")),
        ),
        _ => return Err(Error::InvalidResourceCommand),
    };
    let config = config
        .and_then(Value::as_object_mut)
        .ok_or(Error::Journal)?;
    for (step, entry) in plan.secrets.iter().zip(&credentials.entries) {
        let id = step.id.filter(|_| step.completed).ok_or(Error::Journal)?;
        require(
            username_digest(&entry.username) == step.username_digest,
            "Storage credentials changed",
        )?;
        owned(session, record.id, step.kind, id).await?;
        config.insert(step.kind.password_key().into(), json!(id.to_string()));
        config.insert(step.kind.username_key().into(), json!(&*entry.username));
    }
    Ok((method, args))
}
async fn clone_config(
    session: &Session,
    record: &ResourceOperationRecord,
    pbd: &Value,
) -> Result<(), Error> {
    let plan = record.storage_auth_plan.as_ref().ok_or(Error::Journal)?;
    let map = PbdRecord::new(pbd)
        .device_config()
        .value()
        .as_object()
        .filter(|m| m.len() <= 128)
        .ok_or(Error::Protocol("Attachment credentials unavailable"))?;
    let mut ids = BTreeSet::new();
    for step in &plan.secrets {
        let id = map
            .get(step.kind.password_key())
            .and_then(Value::as_str)
            .and_then(|v| SecretId::parse(v).ok())
            .ok_or(Error::Protocol("Attachment Secret identity unavailable"))?;
        require(
            !plan.baseline.contains(&id)
                && !plan.secrets.iter().any(|s| s.id == Some(id))
                && ids.insert(id),
            "Attachment did not receive its own new Secret",
        )?;
        let username = map
            .get(step.kind.username_key())
            .and_then(Value::as_str)
            .filter(|v| v.len() <= 1024)
            .ok_or(Error::Protocol("Attachment authentication unavailable"))?;
        require(
            username_digest(username) == step.username_digest,
            "Attachment authentication differs from the review",
        )?;
        let reference = secret_reference(session, id).await?;
        require(
            secret_uuid(session, &reference).await? == id,
            "Attachment Secret identity changed",
        )?;
    }
    for key in map.keys() {
        let sensitive = key.contains("password")
            || key.contains("secret")
            || matches!(key.as_str(), "username" | "chapuser" | "incoming_chapuser");
        require(
            !sensitive
                || plan
                    .secrets
                    .iter()
                    .any(|s| key == s.kind.password_key() || key == s.kind.username_key()),
            "Unexpected attachment authentication fields",
        )?;
    }
    Ok(())
}
pub(super) async fn verify(
    session: &Session,
    record: &ResourceOperationRecord,
) -> Result<ResourceTarget, Error> {
    let plan = record.storage_auth_plan.as_ref().ok_or(Error::Journal)?;
    require(
        plan.provision_succeeded,
        "Storage task success was not recorded; no write will be repeated",
    )?;
    let created = storage::verify(session, record)
        .await?
        .ok_or(Error::Journal)?;
    match created {
        ResourceTarget::Pbd(id) => {
            let (_, pbd) = xapi::read(session, ResourceTarget::Pbd(id)).await?;
            let StorageOperation::CreateAttachment { host, location } = &plan.operation else {
                return Err(Error::Journal);
            };
            let ResourceTarget::Storage(sr) = record.target else {
                return Err(Error::Journal);
            };
            storage::attachment_identity(session, &pbd, sr, *host).await?;
            require(
                PbdRecord::new(&pbd).currently_attached() == false
                    && storage::config_matches(
                        &PbdRecord::new(&pbd).device_config(),
                        &location.device_config(),
                    ),
                "Created attachment identity or location changed during verification",
            )?;
            clone_config(session, record, &pbd).await?;
        }
        ResourceTarget::Storage(sr) => {
            let (sr_ref, fresh) = xapi::read(session, ResourceTarget::Storage(sr)).await?;
            let StorageOperation::Create {
                host,
                ref name,
                ref description,
                ref location,
                ..
            } = plan.operation
            else {
                return Err(Error::Journal);
            };
            require(
                SrRecord::new(&fresh).name_label() == *name
                    && SrRecord::new(&fresh).name_description() == *description
                    && SrRecord::new(&fresh).type_() == location.kind()
                    && SrRecord::new(&fresh).content_type() == location.content_type()
                    && SrRecord::new(&fresh).shared() == true,
                "Created storage metadata changed during verification",
            )?;
            crate::storage_provisioning::verify_allocation(&fresh, location)?;
            let (host_ref, _) = xapi::read(session, ResourceTarget::Host(host)).await?;
            let refs = SrRecord::new(&fresh)
                .pbds()
                .value()
                .as_array()
                .filter(|v| !v.is_empty() && v.len() <= 1024)
                .ok_or(Error::Protocol("Created storage attachments unavailable"))?;
            let baseline = record.storage_baseline.as_ref().ok_or(Error::Journal)?;
            let mut found = false;
            for value in refs {
                let reference = xapi::reply_reference(value)?;
                let pbd = session
                    .call("PBD.get_record", vec![json!(reference)])
                    .await?;
                if PbdRecord::new(&pbd).host() != host_ref {
                    continue;
                }
                let id = PbdRecord::new(&pbd)
                    .uuid()
                    .as_str()
                    .and_then(|v| crate::PbdId::parse(v).ok())
                    .ok_or(Error::Protocol("Created attachment identity unavailable"))?;
                require(
                    !baseline.pbds.contains(&id) && !found,
                    "Created attachment provenance is ambiguous",
                )?;
                require(
                    PbdRecord::new(&pbd).sr() == sr_ref
                        && PbdRecord::new(&pbd).currently_attached() == true
                        && storage::config_matches(
                            &PbdRecord::new(&pbd).device_config(),
                            &location.device_config(),
                        ),
                    "Created attachment identity or location changed during verification",
                )?;
                clone_config(session, record, &pbd).await?;
                found = true;
            }
            require(found, "Created host attachment unavailable")?;
        }
        _ => return Err(Error::Journal),
    }
    Ok(created)
}
pub(super) async fn unused(session: &Session, id: SecretId) -> Result<(), Error> {
    for (class, field) in [("PBD", "device_config"), ("SR", "sm_config")] {
        let records = session
            .call(&format!("{class}.get_all_records"), vec![])
            .await?;
        let records = records
            .as_object()
            .filter(|v| v.len() <= 10_000)
            .ok_or(Error::Protocol(
                "Storage Secret consumer scan is incomplete",
            ))?;
        for record in records.values() {
            let config =
                record[field]
                    .as_object()
                    .filter(|v| v.len() <= 128)
                    .ok_or(Error::Protocol(
                        "Storage Secret consumer configuration unavailable",
                    ))?;
            for value in config.values() {
                let value = value
                    .as_str()
                    .filter(|v| v.len() <= 4096)
                    .ok_or(Error::Protocol(
                        "Storage Secret consumer configuration is invalid",
                    ))?;
                require(
                    value != id.to_string(),
                    "Secret is still referenced by storage; it was not deleted",
                )?;
            }
        }
    }
    Ok(())
}
pub(super) async fn absent(session: &Session, id: SecretId) -> Result<bool, Error> {
    match secret_reference(session, id).await {
        Err(Error::Api(code)) if code == "UUID_INVALID" => Ok(true),
        Ok(_) => Ok(false),
        Err(error) => Err(error),
    }
}

pub(super) fn task_uuid(value: &Value) -> Result<crate::TaskId, Error> {
    TaskRecord::new(value)
        .uuid()
        .as_str()
        .and_then(|v| crate::TaskId::parse(v).ok())
        .filter(|id| !id.uuid().is_nil())
        .ok_or(Error::Protocol("Storage task UUID unavailable"))
}
