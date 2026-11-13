use super::{ResourceOperationRecord, ResourceTarget, storage_auth_backend as secrets, xapi};
use crate::{
    Error, HostPowerConfiguration, HostPowerOperation, HostPowerSettings, OperationId, SecretId,
    xapi::Session,
};
pub(super) use secrets::{absent, require, secret_ids, secret_uuid};
use serde_json::{Value, json};
use xenadmin_xapi::records::HostRecord;
use xenadmin_xapi::{calls, refs};

pub(super) async fn current(
    session: &Session,
    record: &ResourceOperationRecord,
) -> Result<(String, HostPowerConfiguration), Error> {
    xapi::read(session, ResourceTarget::Pool(record.pool)).await?;
    let (reference, host) = xapi::read(session, record.target).await?;
    require(
        HostRecord::new(&host)
            .current_operations()
            .entries()
            .is_some_and(|m| m.is_empty()),
        "Host operations are active or unavailable",
    )?;
    let config = crate::host_power::configuration(&host).ok_or(Error::HostPrerequisite(
        "The current power-on configuration is unsupported or incomplete",
    ))?;
    Ok((reference, config))
}
pub(super) async fn expected(
    session: &Session,
    record: &ResourceOperationRecord,
) -> Result<String, Error> {
    let crate::ResourceAction::HostPower(HostPowerOperation::Configure { expected, .. }) =
        &record.action
    else {
        return Err(Error::Journal);
    };
    let (reference, current) = current(session, record).await?;
    require(
        &current == expected,
        "Power-on settings changed since review",
    )?;
    if let Some(secret) = expected.secret {
        secrets::unused(session, secret).await?;
        no_host_consumer(session, secret, Some((record.target.uuid(), expected))).await?;
    }
    Ok(reference)
}
fn marker(id: OperationId) -> String {
    format!("{id}/host-power")
}
pub(super) fn secret_record(id: OperationId, password: &str) -> Value {
    json!({"value":password,"other_config":{xapi::OPERATION_TAG:marker(id)}})
}
pub(super) async fn owned(
    session: &Session,
    operation: OperationId,
    id: SecretId,
) -> Result<String, Error> {
    let reference = secrets::secret_reference(session, id).await?;
    require(
        secret_uuid(session, &reference).await? == id,
        "Host power Secret identity changed",
    )?;
    let config = session
        .send(calls::secret::GetOtherConfig(refs::secret(&reference)))
        .await?;
    require(
        config[xapi::OPERATION_TAG] == marker(operation),
        "Host power Secret ownership changed",
    )?;
    Ok(reference)
}
pub(super) async fn recover_secret(
    session: &Session,
    record: &ResourceOperationRecord,
) -> Result<Option<SecretId>, Error> {
    let plan = record.host_power_plan.as_ref().ok_or(Error::Journal)?;
    let mut found = None;
    for id in secret_ids(session).await? {
        if plan.baseline.contains(&id) {
            continue;
        }
        let reference = secrets::secret_reference(session, id).await?;
        let config = session
            .send(calls::secret::GetOtherConfig(refs::secret(&reference)))
            .await?;
        if config[xapi::OPERATION_TAG] == marker(record.id) {
            require(
                found.is_none(),
                "Multiple Secrets match the host power operation",
            )?;
            found = Some(id);
        }
    }
    Ok(found)
}
pub(super) fn arguments(
    settings: &HostPowerSettings,
    secret: Option<SecretId>,
) -> Result<Value, Error> {
    Ok(match settings {
        HostPowerSettings::Disabled | HostPowerSettings::WakeOnLan => json!({}),
        HostPowerSettings::Ipmi { address, username } => {
            json!({"power_on_ip":address.to_string(),"power_on_user":username,"power_on_password_secret":secret.ok_or(Error::Journal)?.to_string()})
        }
    })
}
pub(super) async fn verify(
    session: &Session,
    record: &ResourceOperationRecord,
) -> Result<(), Error> {
    let plan = record.host_power_plan.as_ref().ok_or(Error::Journal)?;
    let HostPowerOperation::Configure { settings, .. } = &plan.operation else {
        return Err(Error::Journal);
    };
    require(
        plan.configure_completed,
        "The host setting write has no completion receipt; matching fields do not establish completion",
    )?;
    let (_, actual) = current(session, record).await?;
    require(
        actual.settings == *settings && actual.secret == plan.secret,
        "Host power-on settings or Secret no longer match the completed operation",
    )?;
    if let Some(secret) = plan.secret {
        owned(session, record.id, secret).await?;
    }
    Ok(())
}
pub(super) async fn unused(session: &Session, id: SecretId) -> Result<(), Error> {
    secrets::unused(session, id).await?;
    no_host_consumer(session, id, None).await
}
async fn no_host_consumer(
    session: &Session,
    id: SecretId,
    excluded: Option<(uuid::Uuid, &HostPowerConfiguration)>,
) -> Result<(), Error> {
    let hosts = session.send(calls::host::GetAllRecords).await?;
    let hosts = hosts
        .as_object()
        .filter(|v| v.len() <= 1024)
        .ok_or(Error::Protocol("Host Secret consumer scan is incomplete"))?;
    let mut excluded_count = 0;
    for host in hosts.values() {
        let host_id = HostRecord::new(host)
            .uuid()
            .text()
            .and_then(|v| uuid::Uuid::parse_str(v).ok())
            .ok_or(Error::Protocol("Host Secret consumer identity unavailable"))?;
        if let Some((id, expected)) = excluded
            && id == host_id
        {
            excluded_count += 1;
            require(
                crate::host_power::configuration(host).as_ref() == Some(expected)
                    && HostRecord::new(host)
                        .current_operations()
                        .entries()
                        .is_some_and(|v| v.is_empty()),
                "Reviewed host configuration or operations changed during Secret consumer inspection",
            )?;
            continue;
        }
        let config = HostRecord::new(host)
            .power_on_config()
            .entries()
            .filter(|v| v.len() <= 128)
            .ok_or(Error::Protocol(
                "Host Secret consumer configuration unavailable",
            ))?;
        for value in config.values() {
            let value = value
                .as_str()
                .filter(|v| v.len() <= 4096)
                .ok_or(Error::Protocol(
                    "Host Secret consumer configuration invalid",
                ))?;
            require(
                !secrets::names(value, id),
                "Secret is still referenced by a host; it was not deleted",
            )?;
        }
    }
    require(
        excluded.is_none() || excluded_count == 1,
        "Selected host identity was missing or duplicated in the Secret consumer scan",
    )
}
