use super::{
    ResourceAction, ResourceOperationRecord, ResourceTarget,
    xapi::{self, Check, Step},
};
use crate::{Error, HaRestartPriority, HostId, PoolOperation, SrId, VmId, xapi::Session};
use serde_json::{Value, json};
use std::collections::BTreeSet;

pub(super) fn operation(record: &ResourceOperationRecord) -> Result<&PoolOperation, Error> {
    match &record.action {
        ResourceAction::Pool(op) => Ok(op),
        _ => Err(Error::InvalidResourceCommand),
    }
}
fn number(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| value.as_str()?.parse().ok())
}
fn reference(value: &Value) -> Result<String, Error> {
    xapi::reply_reference(value)
}
fn empty(fresh: &Value, field: &str) -> Result<(), Error> {
    if fresh[field].as_object().is_some_and(|v| v.is_empty()) {
        Ok(())
    } else {
        Err(Error::ResourceInUse)
    }
}
fn expect(fresh: &Value, field: &str, value: Value) -> Result<(), Error> {
    if fresh[field] == value {
        Ok(())
    } else {
        Err(Error::ResourceConflict)
    }
}
fn permitted(fresh: &Value, operation: &str) -> Result<(), Error> {
    if fresh["allowed_operations"]
        .as_array()
        .is_some_and(|ops| ops.iter().any(|v| v == operation))
    {
        Ok(())
    } else {
        Err(Error::UnavailableAction)
    }
}
async fn all(
    session: &Session,
    class: &str,
    limit: usize,
) -> Result<serde_json::Map<String, Value>, Error> {
    session
        .call(&format!("{class}.get_all_records"), vec![])
        .await?
        .as_object()
        .filter(|v| v.len() <= limit)
        .cloned()
        .ok_or(Error::Protocol(
            "pool membership unavailable or exceeds inspection limit",
        ))
}
async fn related(session: &Session, class: &str, value: &Value) -> Result<Value, Error> {
    session
        .call(
            &format!("{class}.get_record"),
            vec![json!(reference(value)?)],
        )
        .await
}
async fn coordinator(session: &Session, fresh: &Value) -> Result<HostId, Error> {
    let host = related(session, "host", &fresh["master"]).await?;
    host["uuid"]
        .as_str()
        .and_then(|v| HostId::parse(v).ok())
        .ok_or(Error::Protocol("pool coordinator identity unavailable"))
}
async fn vm_record(session: &Session, vm: VmId) -> Result<(String, Value), Error> {
    let reference = reference(
        &session
            .call("VM.get_by_uuid", vec![json!(vm.to_string())])
            .await?,
    )?;
    let fresh = session
        .call("VM.get_record", vec![json!(&reference)])
        .await?;
    if fresh["uuid"] != vm.to_string()
        || fresh["is_control_domain"] != false
        || fresh["is_a_template"] != false
        || fresh["is_a_snapshot"] != false
    {
        return Err(Error::ResourceConflict);
    }
    Ok((reference, fresh))
}
async fn host_live(session: &Session, host: &Value) -> Result<(), Error> {
    if related(session, "host_metrics", &host["metrics"]).await?["live"] != true {
        return Err(Error::UnavailableAction);
    }
    Ok(())
}
async fn no_resident_guests(session: &Session, host_ref: &str) -> Result<(), Error> {
    for vm in all(session, "VM", 10_000).await?.values() {
        if vm["is_control_domain"] != true
            && (vm["resident_on"] == host_ref || vm["scheduled_to_be_resident_on"] == host_ref)
        {
            return Err(Error::ResourceInUse);
        }
    }
    Ok(())
}
async fn heartbeat_arguments(session: &Session, srs: &[SrId]) -> Result<Vec<Value>, Error> {
    let hosts = all(session, "host", 256).await?;
    if hosts.len() < 2 {
        return Err(Error::UnavailableAction);
    }
    let mut result = Vec::new();
    for id in srs {
        let (sr_ref, sr) = xapi::read(session, ResourceTarget::Storage(*id)).await?;
        if sr["shared"] != true || sr["content_type"] != "user" {
            return Err(Error::UnavailableAction);
        }
        empty(&sr, "current_operations")?;
        session
            .call("SR.assert_can_host_ha_statefile", vec![json!(&sr_ref)])
            .await?;
        let pbds = sr["PBDs"]
            .as_array()
            .filter(|v| v.len() <= 256)
            .ok_or(Error::Protocol("heartbeat storage attachments unavailable"))?;
        let mut attached = BTreeSet::new();
        for pbd in pbds {
            let pbd = related(session, "PBD", pbd).await?;
            if pbd["SR"] != sr_ref || pbd["currently_attached"] != true {
                return Err(Error::UnavailableAction);
            }
            attached.insert(reference(&pbd["host"])?);
        }
        if hosts.keys().any(|host| !attached.contains(host)) {
            return Err(Error::UnavailableAction);
        }
        result.push(json!(sr_ref));
    }
    Ok(result)
}
pub(super) async fn prepare(
    session: &Session,
    record: &mut ResourceOperationRecord,
) -> Result<Vec<Step>, Error> {
    if matches!(operation(record)?, PoolOperation::Security(_)) {
        super::security::prepare(session, record).await?;
    }
    arguments(session, record).await?;
    let op = operation(record)?;
    Ok(vec![Step {
        label: op.label().into(),
        method: op.method().into(),
        argument: None,
        expected: Value::Null,
        check: Check::Pool,
    }])
}
pub(super) async fn arguments(
    session: &Session,
    record: &ResourceOperationRecord,
) -> Result<Vec<Value>, Error> {
    let op = operation(record)?;
    let (pool_ref, fresh) = xapi::read(session, record.target).await?;
    empty(&fresh, "current_operations")?;
    match op {
        PoolOperation::Security(_) => super::security::arguments(session, record).await,
        PoolOperation::SetMigrationCompression { expected, enabled } => {
            expect(&fresh, "migration_compression", json!(expected))?;
            Ok(vec![json!(pool_ref), json!(enabled)])
        }
        PoolOperation::SetHttpsOnly { expected, enabled } => {
            expect(&fresh, "https_only", json!(expected))?;
            Ok(vec![json!(pool_ref), json!(enabled)])
        }
        PoolOperation::DesignateCoordinator { expected, host } => {
            expect(&fresh, "ha_enabled", json!(false))?;
            permitted(&fresh, "designate_new_master")?;
            if coordinator(session, &fresh).await? != *expected {
                return Err(Error::ResourceConflict);
            }
            let (host_ref, host) = xapi::read(session, ResourceTarget::Host(*host)).await?;
            expect(&host, "enabled", json!(true))?;
            empty(&host, "current_operations")?;
            host_live(session, &host).await?;
            Ok(vec![json!(host_ref)])
        }
        PoolOperation::SetHaFailuresToTolerate { expected, failures } => {
            if number(&fresh["ha_host_failures_to_tolerate"]) != Some(*expected) {
                return Err(Error::ResourceConflict);
            }
            if *failures >= all(session, "host", 256).await?.len() as u64 {
                return Err(Error::InvalidResourceCommand);
            }
            if session
                .call(
                    "pool.ha_failover_plan_exists",
                    vec![json!(failures.to_string())],
                )
                .await?
                != true
            {
                return Err(Error::UnavailableAction);
            }
            Ok(vec![json!(pool_ref), json!(failures.to_string())])
        }
        PoolOperation::EnableHa {
            heartbeat_srs,
            expected_failures,
        } => {
            expect(&fresh, "ha_enabled", json!(false))?;
            permitted(&fresh, "ha_enable")?;
            if number(&fresh["ha_host_failures_to_tolerate"]) != Some(*expected_failures) {
                return Err(Error::ResourceConflict);
            }
            let srs = heartbeat_arguments(session, heartbeat_srs).await?;
            if session
                .call(
                    "pool.ha_failover_plan_exists",
                    vec![json!(expected_failures.to_string())],
                )
                .await?
                != true
            {
                return Err(Error::UnavailableAction);
            }
            Ok(vec![json!(srs), json!({})])
        }
        PoolOperation::DisableHa => {
            expect(&fresh, "ha_enabled", json!(true))?;
            permitted(&fresh, "ha_disable")?;
            Ok(vec![])
        }
        PoolOperation::SetVmRestartPriority {
            vm,
            expected,
            priority,
        } => {
            let (vm_ref, vm) = vm_record(session, *vm).await?;
            empty(&vm, "current_operations")?;
            expect(&vm, "ha_restart_priority", json!(expected.as_str()))?;
            if *priority == HaRestartPriority::Restart {
                session
                    .call("VM.assert_agile", vec![json!(&vm_ref)])
                    .await?;
            }
            Ok(vec![json!(vm_ref), json!(priority.as_str())])
        }
        PoolOperation::EjectHost {
            host,
            expected_name,
        } => {
            expect(&fresh, "ha_enabled", json!(false))?;
            if coordinator(session, &fresh).await? == *host {
                return Err(Error::UnavailableAction);
            }
            let (host_ref, host) = xapi::read(session, ResourceTarget::Host(*host)).await?;
            expect(&host, "name_label", json!(expected_name))?;
            expect(&host, "enabled", json!(false))?;
            empty(&host, "current_operations")?;
            host_live(session, &host).await?;
            no_resident_guests(session, &host_ref).await?;
            // A local SR with guest data can become inaccessible when its host leaves.
            for pbd in all(session, "PBD", 10_000)
                .await?
                .values()
                .filter(|p| p["host"] == host_ref)
            {
                let sr = related(session, "SR", &pbd["SR"]).await?;
                if sr["shared"].as_bool().is_none() || sr["content_type"].as_str().is_none() {
                    return Err(Error::Protocol("member storage classification unavailable"));
                }
                if sr["shared"] == false
                    && sr["content_type"] == "user"
                    && !sr["VDIs"].as_array().is_some_and(Vec::is_empty)
                {
                    return Err(Error::ResourceInUse);
                }
            }
            Ok(vec![json!(host_ref)])
        }
        PoolOperation::EnableTlsVerification => {
            expect(&fresh, "tls_verification_enabled", json!(false))?;
            permitted(&fresh, "tls_verification_enable")?;
            Ok(vec![])
        }
        PoolOperation::RotateSecret => {
            expect(&fresh, "ha_enabled", json!(false))?;
            expect(&fresh, "is_psr_pending", json!(false))?;
            for host in all(session, "host", 256).await?.values() {
                host_live(session, host).await?;
            }
            Ok(vec![])
        }
    }
}
pub(super) async fn verify(
    session: &Session,
    record: &ResourceOperationRecord,
) -> Result<Option<ResourceTarget>, Error> {
    if !record.pool_completed {
        return Err(Error::Protocol(
            "pool operation completion evidence unavailable; no write will be replayed",
        ));
    }
    let (_, fresh) = xapi::read(session, record.target).await?;
    let matched = match operation(record)? {
        PoolOperation::Security(_) => return super::security::verify(session, record).await,
        PoolOperation::SetMigrationCompression { enabled, .. } => {
            fresh["migration_compression"] == *enabled
        }
        PoolOperation::SetHttpsOnly { enabled, .. } => fresh["https_only"] == *enabled,
        PoolOperation::DesignateCoordinator { host, .. } => {
            coordinator(session, &fresh).await? == *host
        }
        PoolOperation::SetHaFailuresToTolerate { failures, .. } => {
            number(&fresh["ha_host_failures_to_tolerate"]) == Some(*failures)
        }
        PoolOperation::EnableHa { heartbeat_srs, .. } => {
            if fresh["ha_enabled"] != true {
                return Err(Error::ResourceConflict);
            }
            let files = fresh["ha_statefiles"]
                .as_array()
                .filter(|v| !v.is_empty() && v.len() <= 64)
                .ok_or(Error::Protocol("HA heartbeat evidence unavailable"))?;
            for file in files {
                let file_ref = if file.as_str().is_some_and(|v| v.starts_with("OpaqueRef:")) {
                    file.clone()
                } else {
                    let id = file
                        .as_str()
                        .and_then(|v| uuid::Uuid::parse_str(v).ok())
                        .ok_or(Error::Protocol("invalid HA heartbeat identity"))?;
                    session
                        .call("VDI.get_by_uuid", vec![json!(id.to_string())])
                        .await?
                };
                let vdi = related(session, "VDI", &file_ref).await?;
                let sr = related(session, "SR", &vdi["SR"]).await?;
                let id = sr["uuid"]
                    .as_str()
                    .and_then(|v| SrId::parse(v).ok())
                    .ok_or(Error::Protocol("HA storage identity unavailable"))?;
                if !heartbeat_srs.contains(&id) {
                    return Err(Error::ResourceConflict);
                }
            }
            true
        }
        PoolOperation::DisableHa => fresh["ha_enabled"] == false,
        PoolOperation::SetVmRestartPriority { vm, priority, .. } => {
            vm_record(session, *vm).await?.1["ha_restart_priority"] == priority.as_str()
        }
        PoolOperation::EjectHost { host, .. } => {
            match xapi::read(session, ResourceTarget::Host(*host)).await {
                Err(Error::Api(code)) if code == "UUID_INVALID" => true,
                Ok(_) => false,
                Err(error) => return Err(error),
            }
        }
        PoolOperation::EnableTlsVerification => fresh["tls_verification_enabled"] == true,
        // Only the task completion receipt proves rotation; pending=false alone never does.
        PoolOperation::RotateSecret => fresh["is_psr_pending"] == false,
    };
    if matched {
        Ok(None)
    } else {
        Err(Error::ResourceConflict)
    }
}
