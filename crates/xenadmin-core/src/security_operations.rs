use super::{ResourceOperationRecord, ResourceTarget};
use crate::{
    Error, PoolSecurityOperation, RoleId, SubjectId, security_discovery as discovery, xapi::Session,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn operation(record: &ResourceOperationRecord) -> Result<&PoolSecurityOperation, Error> {
    match &record.action {
        crate::ResourceAction::Pool(crate::PoolOperation::Security(op)) => Ok(op),
        _ => Err(Error::InvalidResourceCommand),
    }
}
fn require(value: bool, reason: &'static str) -> Result<(), Error> {
    if value {
        Ok(())
    } else {
        Err(Error::SecurityPrerequisite(reason))
    }
}
async fn read(session: &Session, class: &str, id: uuid::Uuid) -> Result<(String, Value), Error> {
    let reference = crate::xapi::opaque_reference(
        &session
            .call(&format!("{class}.get_by_uuid"), vec![json!(id.to_string())])
            .await?,
    )?;
    let record = session
        .call(&format!("{class}.get_record"), vec![json!(&reference)])
        .await?;
    require(
        record["uuid"] == id.to_string(),
        "Security object identity changed",
    )?;
    Ok((reference, record))
}
async fn reviewed_hosts(
    session: &Session,
    expected: &[crate::DirectoryHostState],
) -> Result<(), Error> {
    let current = discovery::directory_hosts(session).await?;
    let mut expected = expected.to_vec();
    expected.sort_by_key(|h| h.host);
    require(
        current == expected,
        "Pool membership or directory configuration changed; refresh and review again",
    )
}
async fn roles(
    session: &Session,
) -> Result<(BTreeMap<String, RoleId>, Vec<crate::SecurityRole>), Error> {
    discovery::roles(&discovery::records(session, "role", 16_384).await?)
}
async fn subject(
    session: &Session,
    id: SubjectId,
    identifier: &str,
    expected_roles: &[RoleId],
) -> Result<(String, Value), Error> {
    let (reference, value) = read(session, "subject", id.uuid()).await?;
    let (ids, _) = roles(session).await?;
    let mut expected = expected_roles.to_vec();
    expected.sort();
    require(
        value["subject_identifier"] == identifier
            && discovery::subject_roles(&value["roles"], &ids)? == expected,
        "The subject or its assigned roles changed since review",
    )?;
    Ok((reference, value))
}
pub(super) async fn prepare(
    session: &Session,
    record: &mut ResourceOperationRecord,
) -> Result<(), Error> {
    if matches!(operation(record)?, PoolSecurityOperation::AddSubject { .. }) {
        let subjects = discovery::records(session, "subject", 4096).await?;
        record.subject_baseline = Some(
            subjects
                .values()
                .map(|v| {
                    SubjectId::parse(&discovery::text(&v["uuid"], 36)?)
                        .map_err(|_| Error::Protocol("invalid subject UUID"))
                })
                .collect::<Result<Vec<_>, Error>>()?,
        );
    }
    Ok(())
}
pub(super) async fn arguments(
    session: &Session,
    record: &ResourceOperationRecord,
) -> Result<Vec<Value>, Error> {
    let op = operation(record)?;
    require(
        discovery::advertised(session, op.method()).await? != Some(false),
        "The host does not advertise this security operation",
    )?;
    require(
        discovery::local_superuser(session).await?,
        "Reconnect with a local superuser before changing pool authentication or certificates",
    )?;
    let (pool_ref, pool) = super::xapi::read(session, record.target).await?;
    require(
        pool["current_operations"]
            .as_object()
            .is_some_and(|v| v.is_empty()),
        "Wait for other pool operations to finish",
    )?;
    match op {
        PoolSecurityOperation::JoinDirectory {
            domain,
            expected_hosts,
        } => {
            reviewed_hosts(session, expected_hosts).await?;
            Ok(vec![json!(pool_ref), json!({}), json!(domain), json!("AD")])
        }
        PoolSecurityOperation::LeaveDirectory { expected_hosts, .. } => {
            reviewed_hosts(session, expected_hosts).await?;
            Ok(vec![json!(pool_ref), json!({})])
        }
        PoolSecurityOperation::AddSubject { identifier } => {
            let hosts = discovery::records(session, "host", 256).await?;
            require(
                !hosts.is_empty()
                    && hosts
                        .values()
                        .all(|h| h["license_params"]["restrict_rbac"] == "false"),
                "Subject creation requires explicitly available RBAC; restricted servers may grant new subjects full administration",
            )?;
            require(
                hosts.values().all(|h| h["external_auth_type"] == "AD"),
                "All pool members must already use Active Directory",
            )?;
            let subjects = discovery::records(session, "subject", 4096).await?;
            require(
                subjects
                    .values()
                    .all(|s| s["subject_identifier"] != *identifier),
                "This directory subject already has access",
            )?;
            session
                .call(
                    "auth.get_subject_information_from_identifier",
                    vec![json!(identifier)],
                )
                .await?;
            Ok(vec![
                json!({"subject_identifier":identifier,"other_config":{}}),
            ])
        }
        PoolSecurityOperation::RemoveSubject {
            subject: id,
            expected_identifier,
            expected_roles,
        } => Ok(vec![json!(
            subject(session, *id, expected_identifier, expected_roles)
                .await?
                .0
        )]),
        PoolSecurityOperation::ChangeSubjectRole {
            subject: id,
            expected_identifier,
            expected_roles,
            role,
            ..
        } => {
            require(
                discovery::rbac_available(session).await? == Some(true),
                "Role assignment requires explicitly available RBAC",
            )?;
            let (reference, _) = subject(session, *id, expected_identifier, expected_roles).await?;
            let (_, assignable) = roles(session).await?;
            require(
                assignable.iter().any(|r| r.id == *role),
                "The selected role is not an assignable role",
            )?;
            let (role, _) = read(session, "role", role.uuid()).await?;
            Ok(vec![json!(reference), json!(role)])
        }
        PoolSecurityOperation::InstallCertificate { host, expected, .. }
        | PoolSecurityOperation::ResetCertificate { host, expected } => {
            require(
                discovery::certificate(session, *host).await? == *expected,
                "The host certificate changed since review",
            )?;
            let (reference, host) = super::xapi::read(session, ResourceTarget::Host(*host)).await?;
            require(
                host["current_operations"]
                    .as_object()
                    .is_some_and(|v| v.is_empty()),
                "Finish host operations before replacing its certificate",
            )?;
            Ok(
                if matches!(op, PoolSecurityOperation::InstallCertificate { .. }) {
                    vec![json!(reference), json!(""), json!(""), json!("")]
                } else {
                    vec![json!(reference)]
                },
            )
        }
    }
}
pub(super) fn add_sensitive(
    record: &ResourceOperationRecord,
    inputs: Option<&crate::pool_security::SensitiveInputs>,
    args: &mut [Value],
) -> Result<(), Error> {
    use crate::pool_security::SensitiveInputs;
    let op = operation(record)?;
    if !op.secret_required() {
        return require(inputs.is_none(), "Unexpected security credentials");
    }
    let inputs = inputs
        .filter(|v| v.valid_for(op))
        .ok_or(Error::SecurityPrerequisite(
            "Re-enter credentials; sensitive inputs are never restored from the journal",
        ))?;
    match inputs {
        SensitiveInputs::Credentials(c) => {
            args[1] = json!({"user":c.username,"pass":&*c.password});
        }
        SensitiveInputs::Certificate(c) => {
            args[1] = json!(c.certificate);
            args[2] = json!(&*c.private_key);
            args[3] = json!(c.chain);
        }
    }
    Ok(())
}
pub(super) async fn verify(
    session: &Session,
    record: &ResourceOperationRecord,
) -> Result<Option<ResourceTarget>, Error> {
    require(
        record.pool_completed,
        "Security operation completion receipt is unavailable; no change will be replayed",
    )?;
    match operation(record)? {
        PoolSecurityOperation::JoinDirectory {
            domain,
            expected_hosts,
        } => {
            let current = discovery::directory_hosts(session).await?;
            require(
                current.len() == expected_hosts.len()
                    && current.iter().all(|h| {
                        expected_hosts.iter().any(|old| old.host == h.host)
                            && h.auth_type == "AD"
                            && h.service_name.eq_ignore_ascii_case(domain)
                    }),
                "Directory join is incomplete or pool membership changed",
            )?;
        }
        PoolSecurityOperation::LeaveDirectory { expected_hosts, .. } => {
            let current = discovery::directory_hosts(session).await?;
            require(
                current.len() == expected_hosts.len()
                    && current.iter().all(|h| {
                        expected_hosts.iter().any(|old| old.host == h.host)
                            && h.auth_type.is_empty()
                            && h.service_name.is_empty()
                    }),
                "Directory leave is incomplete or pool membership changed",
            )?;
        }
        PoolSecurityOperation::AddSubject { identifier } => {
            let reference = record
                .result_reference
                .as_ref()
                .ok_or(Error::SecurityPrerequisite(
                    "New subject identity receipt is missing",
                ))?;
            let value = session
                .call("subject.get_record", vec![json!(reference)])
                .await?;
            let id = SubjectId::parse(&discovery::text(&value["uuid"], 36)?)
                .map_err(|_| Error::Protocol("invalid subject UUID"))?;
            require(
                record
                    .subject_baseline
                    .as_ref()
                    .is_some_and(|ids| !ids.contains(&id))
                    && value["subject_identifier"] == *identifier,
                "The returned subject does not match this creation",
            )?;
            require(
                value["roles"].as_array().is_some_and(Vec::is_empty),
                "The server assigned unexpected roles; inspect subject access before acknowledging",
            )?;
        }
        PoolSecurityOperation::RemoveSubject {
            subject,
            expected_identifier,
            ..
        } => {
            match read(session, "subject", subject.uuid()).await {
                Err(Error::Api(code)) if code == "UUID_INVALID" => {}
                Ok(_) => return Err(Error::SecurityPrerequisite("The subject still has access")),
                Err(e) => return Err(e),
            }
            // A removed subject's open sessions keep their role until the host next checks
            // them. The reference client closes them before the removal, which a client that
            // signs in again at once survives; closing them after it leaves nothing to sign
            // in with. The call closes sessions only, so a recheck may repeat it.
            match session
                .call(
                    "session.logout_subject_identifier",
                    vec![json!(expected_identifier)],
                )
                .await
            {
                Ok(_) => {}
                Err(Error::Api(_)) => {
                    return Err(Error::SecurityPrerequisite(
                        "The subject's access was removed, but the host refused to close its open sessions. They last until the host next checks its sessions",
                    ));
                }
                Err(e) => return Err(e),
            }
        }
        PoolSecurityOperation::ChangeSubjectRole {
            subject: id,
            expected_identifier,
            expected_roles,
            role,
            grant,
        } => {
            let mut expected = expected_roles.clone();
            if *grant {
                expected.push(*role);
            } else {
                expected.retain(|v| v != role);
            }
            subject(session, *id, expected_identifier, &expected).await?;
        }
        PoolSecurityOperation::InstallCertificate {
            host, fingerprint, ..
        } => require(
            discovery::certificate(session, *host).await? == *fingerprint,
            "The installed certificate fingerprint is not yet verified",
        )?,
        PoolSecurityOperation::ResetCertificate { host, expected } => require(
            discovery::certificate(session, *host).await? != *expected,
            "The replacement certificate has not been verified",
        )?,
    }
    Ok(None)
}
