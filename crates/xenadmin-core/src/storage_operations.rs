use super::{
    ResourceAction, ResourceOperationRecord, ResourceTarget,
    xapi::{self, Check, Step},
};
use crate::{Error, PbdId, SrId, StorageOperation, VdiId, storage::StorageBaseline, xapi::Session};
use serde_json::{Value, json};
use xenadmin_xapi::calls;
use xenadmin_xapi::enumerations::{storage_operations, vdi_type};
use xenadmin_xapi::records::{PbdRecord, SmRecord, SrRecord, TaskRecord, VbdRecord, VdiRecord};
use xenadmin_xapi::{Reads, kind};

fn operation(record: &ResourceOperationRecord) -> Result<&StorageOperation, Error> {
    match &record.action {
        ResourceAction::Storage(op)
        | ResourceAction::AuthenticatedStorage { operation: op, .. } => Ok(op),
        _ => Err(Error::InvalidResourceCommand),
    }
}
fn array<'a>(value: &'a Value, field: &str) -> Result<&'a Vec<Value>, Error> {
    value[field]
        .as_array()
        .filter(|values| values.len() <= 10000)
        .ok_or(Error::Protocol("storage relationship data unavailable"))
}
fn empty_current(value: &Value) -> Result<(), Error> {
    match value["current_operations"].as_object() {
        Some(ops) if ops.is_empty() => Ok(()),
        _ => Err(Error::StoragePrerequisite(
            "The repository has running operations, or their availability is unknown",
        )),
    }
}
fn allowed(value: &Value, action: &str) -> Result<(), Error> {
    if array(value, "allowed_operations")?
        .iter()
        .any(|value| value == action)
    {
        Ok(())
    } else {
        Err(Error::StoragePrerequisite(
            "The host no longer advertises the reviewed storage operation",
        ))
    }
}
async fn by_ref(session: &Session, class: &str, reference: &Value) -> Result<Value, Error> {
    let reference = xapi::reply_reference(reference)?;
    session
        .call(&format!("{class}.get_record"), vec![json!(reference)])
        .await
}
pub(super) async fn capability(session: &Session, kind: &str, feature: &str) -> Result<(), Error> {
    let records = session.send(calls::sm::GetAllRecords).await?;
    let records = records
        .as_object()
        .filter(|r| r.len() <= 256)
        .ok_or(Error::Protocol("storage capability data unavailable"))?;
    let supported = records
        .values()
        .filter(|r| SmRecord::new(r).type_() == kind)
        .any(|r| {
            feature.is_empty()
                || SmRecord::new(r)
                    .features()
                    .entries()
                    .is_some_and(|f| f.contains_key(feature))
                || SmRecord::new(r)
                    .capabilities()
                    .members()
                    .is_some_and(|v| v.iter().any(|v| v == feature))
        });
    if supported {
        Ok(())
    } else {
        Err(Error::UnavailableAction)
    }
}
async fn baseline(session: &Session) -> Result<StorageBaseline, Error> {
    let srs = session.send(calls::sr::GetAllRecords).await?;
    let pbds = session.send(calls::pbd::GetAllRecords).await?;
    fn parse(records: &Value) -> Result<Vec<&Value>, Error> {
        Ok(records
            .as_object()
            .filter(|r| r.len() <= 10000)
            .ok_or(Error::Protocol("storage creation baseline unavailable"))?
            .values()
            .collect())
    }
    Ok(StorageBaseline {
        srs: parse(&srs)?
            .into_iter()
            .map(|r| {
                SrRecord::new(r)
                    .uuid()
                    .text()
                    .and_then(|s| SrId::parse(s).ok())
                    .ok_or(Error::Protocol("invalid storage baseline UUID"))
            })
            .collect::<Result<_, _>>()?,
        pbds: parse(&pbds)?
            .into_iter()
            .map(|r| {
                PbdRecord::new(r)
                    .uuid()
                    .text()
                    .and_then(|s| PbdId::parse(s).ok())
                    .ok_or(Error::Protocol("invalid attachment baseline UUID"))
            })
            .collect::<Result<_, _>>()?,
    })
}
pub(super) async fn attachment_identity(
    session: &Session,
    fresh: &Value,
    sr: SrId,
    host: crate::HostId,
) -> Result<(String, Value), Error> {
    let (sr_ref, sr_record) = xapi::read(session, ResourceTarget::Storage(sr)).await?;
    let (host_ref, _) = xapi::read(session, ResourceTarget::Host(host)).await?;
    if PbdRecord::new(fresh).sr() != sr_ref || PbdRecord::new(fresh).host() != host_ref {
        return Err(Error::ResourceConflict);
    }
    Ok((sr_ref, sr_record))
}
async fn no_consumers(
    session: &Session,
    sr: &Value,
    require_unused: bool,
) -> Result<Vec<VdiId>, Error> {
    let mut ids = Vec::new();
    let mut reads = 0usize;
    for reference in array(sr, "VDIs")? {
        reads += 1;
        if reads > 10000 {
            return Err(Error::Protocol(
                "storage consumer graph exceeds the read limit",
            ));
        }
        let vdi = by_ref(session, "VDI", reference).await?;
        empty_current(&vdi)?;
        if require_unused && VdiRecord::new(&vdi).type_() != vdi_type::USER {
            return Err(Error::StorageInUse);
        }
        ids.push(
            VdiRecord::new(&vdi)
                .uuid()
                .text()
                .and_then(|id| VdiId::parse(id).ok())
                .ok_or(Error::Protocol("storage disk identity unavailable"))?,
        );
        for reference in array(&vdi, "VBDs")? {
            if require_unused {
                return Err(Error::StorageInUse);
            }
            reads += 1;
            if reads > 10000 {
                return Err(Error::Protocol(
                    "storage consumer graph exceeds the read limit",
                ));
            }
            let vbd = by_ref(session, "VBD", reference).await?;
            if VbdRecord::new(&vbd).currently_attached() != false {
                return Err(Error::StorageInUse);
            }
        }
    }
    ids.sort();
    if ids.windows(2).any(|w| w[0] == w[1]) {
        return Err(Error::Protocol("duplicate storage disk identity"));
    }
    Ok(ids)
}
async fn removable(
    session: &Session,
    record: &ResourceOperationRecord,
    sr_ref: &str,
    sr: &Value,
    name: &str,
    disks: &[VdiId],
) -> Result<(), Error> {
    if SrRecord::new(sr).name_label() != name {
        return Err(Error::ResourceConflict);
    }
    empty_current(sr)?;
    // A repository that was some host's local cache is refused below while a host still
    // binds it; the flag alone must not keep a dead host's local storage from being
    // forgotten (lab finding 37).
    if SrRecord::new(sr).is_tools_sr() != false {
        return Err(Error::StorageInUse);
    }
    let (_, pool) = xapi::read(session, ResourceTarget::Pool(record.pool)).await?;
    for field in ["default_SR", "suspend_image_SR", "crash_dump_SR"] {
        if pool[field].as_str().is_none() {
            return Err(Error::Protocol("pool storage dependencies unavailable"));
        }
        if pool[field] == sr_ref {
            return Err(Error::StorageInUse);
        }
    }
    // HA statefiles and host-local cache/suspend/crash dependencies can survive unplugged guests.
    let actual_disks = no_consumers(session, sr, true).await?;
    for statefile in array(&pool, "ha_statefiles")? {
        let statefile_id = statefile.as_str().and_then(|s| VdiId::parse(s).ok());
        if statefile_id.is_some_and(|id| actual_disks.contains(&id))
            || array(sr, "VDIs")?.contains(statefile)
        {
            return Err(Error::StorageInUse);
        }
    }
    let hosts = session.send(calls::host::GetAllRecords).await?;
    for host in hosts
        .as_object()
        .filter(|h| h.len() <= 1024)
        .ok_or(Error::Protocol("host storage dependencies unavailable"))?
        .values()
    {
        for field in ["local_cache_sr", "suspend_image_sr", "crash_dump_sr"] {
            if host[field].as_str().is_none() {
                return Err(Error::Protocol("host storage dependencies unavailable"));
            }
            if host[field] == sr_ref {
                return Err(Error::StorageInUse);
            }
        }
    }
    for pbd in array(sr, "PBDs")? {
        if PbdRecord::new(&by_ref(session, "PBD", pbd).await?).currently_attached() != false {
            return Err(Error::StorageInUse);
        }
    }
    let mut expected = disks.to_vec();
    expected.sort();
    if actual_disks != expected {
        return Err(Error::ResourceConflict);
    }
    Ok(())
}
pub(super) async fn prepare(
    session: &Session,
    record: &mut ResourceOperationRecord,
) -> Result<Vec<Step>, Error> {
    let op = operation(record)?.clone();
    if op.creates_reference() {
        record.storage_baseline = Some(baseline(session).await?);
    }
    let method = op.method();
    // Run the same fresh admission checks again immediately before dispatch.
    arguments(session, record).await?;
    Ok(vec![Step {
        label: op.label().into(),
        method: method.into(),
        argument: None,
        expected: Value::Null,
        check: Check::Storage,
    }])
}
pub(super) async fn arguments(
    session: &Session,
    record: &ResourceOperationRecord,
) -> Result<Vec<Value>, Error> {
    let (reference, fresh) = xapi::read(session, record.target).await?;
    match operation(record)? {
        StorageOperation::Trim {
            host,
            expected_name,
        } => {
            if SrRecord::new(&fresh).name_label() != *expected_name {
                return Err(Error::ResourceConflict);
            }
            empty_current(&fresh)?;
            capability(
                session,
                SrRecord::new(&fresh)
                    .type_()
                    .text()
                    .ok_or(Error::Protocol("storage type unavailable"))?,
                "SR_TRIM",
            )
            .await?;
            let (host_ref, _) = xapi::read(session, ResourceTarget::Host(*host)).await?;
            let mut attached = false;
            for pbd in array(&fresh, "PBDs")? {
                let pbd = by_ref(session, "PBD", pbd).await?;
                if PbdRecord::new(&pbd).host() == host_ref
                    && PbdRecord::new(&pbd).sr() == reference
                    && PbdRecord::new(&pbd).currently_attached() == true
                {
                    attached = true;
                }
            }
            if !attached {
                return Err(Error::UnavailableAction);
            }
            Ok(vec![
                json!(host_ref),
                json!("trim"),
                json!("do_trim"),
                json!({"sr_uuid":record.target.uuid().to_string()}),
            ])
        }
        StorageOperation::Create {
            host,
            name,
            description,
            location,
            hba_device,
            ..
        } => {
            capability(session, location.kind(), "").await?;
            crate::storage_provisioning::allocation_capability(session, location).await?;
            let (host_ref, _) = xapi::read(session, ResourceTarget::Host(*host)).await?;
            if let Some(device) = hba_device {
                capability(session, location.kind(), "SR_PROBE").await?;
                crate::storage_provisioning::hba_creation(
                    session, &host_ref, *host, location, device,
                )
                .await?;
            }
            if matches!(location, crate::StorageLocation::Local { .. }) {
                crate::storage_local::preflight(session, *host, location).await?;
            }
            Ok(vec![
                json!(host_ref),
                location.device_config(),
                json!("0"),
                json!(name),
                json!(description),
                json!(location.kind()),
                json!(location.content_type()),
                json!(location.shared()),
                json!({}),
            ])
        }
        StorageOperation::Introduce {
            uuid,
            name,
            description,
            location,
        } => {
            capability(session, location.kind(), "").await?;
            match xapi::read(session, ResourceTarget::Storage(*uuid)).await {
                Err(Error::Api(code)) if code == "UUID_INVALID" => {}
                Ok(_) => return Err(Error::ResourceConflict),
                Err(error) => return Err(error),
            }
            Ok(vec![
                json!(uuid.to_string()),
                json!(name),
                json!(description),
                json!(location.kind()),
                json!(location.content_type()),
                json!(location.shared()),
                json!({}),
            ])
        }
        StorageOperation::CreateAttachment { host, location } => {
            if SrRecord::new(&fresh).type_() != location.kind()
                || SrRecord::new(&fresh).content_type() != location.content_type()
                || SrRecord::new(&fresh).shared() != location.shared()
            {
                return Err(Error::ResourceConflict);
            }
            crate::storage_provisioning::allocation_capability(session, location).await?;
            crate::storage_provisioning::verify_allocation(&fresh, location)?;
            allowed(&fresh, storage_operations::PBD_CREATE)?;
            empty_current(&fresh)?;
            let (host_ref, _) = xapi::read(session, ResourceTarget::Host(*host)).await?;
            if !location.shared() && !array(&fresh, "PBDs")?.is_empty() {
                return Err(Error::Protocol(
                    "Local storage already has a host attachment; repair that attachment instead",
                ));
            }
            if !location.shared() {
                crate::storage_local::no_attachments(session, &reference).await?;
            }
            for pbd in array(&fresh, "PBDs")? {
                let attachment = by_ref(session, "PBD", pbd).await?;
                if PbdRecord::new(&attachment).host() == host_ref {
                    return Err(Error::ResourceConflict);
                }
                if !matches!(record.action, ResourceAction::AuthenticatedStorage { .. })
                    && crate::storage::requires_credentials(
                        PbdRecord::new(&attachment).device_config(),
                    )
                {
                    return Err(Error::Protocol(
                        "This attachment requires fresh storage credentials",
                    ));
                }
                if crate::storage::safe_location_for_kind(
                    PbdRecord::new(&attachment).device_config(),
                    SrRecord::new(&fresh).type_().text(),
                )
                .as_ref()
                    != Some(location)
                {
                    return Err(Error::ResourceConflict);
                }
            }
            Ok(vec![
                json!({"host":host_ref,"SR":reference,"device_config":location.device_config(),"other_config":{xapi::OPERATION_TAG:record.id.to_string()}}),
            ])
        }
        StorageOperation::SetAttached {
            sr,
            host,
            expected_attached,
            attached,
        } => {
            let (_, sr_record) = attachment_identity(session, &fresh, *sr, *host).await?;
            if *attached {
                crate::storage_local::attachment_graph(&sr_record, &reference)?;
            }
            if PbdRecord::new(&fresh).currently_attached() != *expected_attached {
                return Err(Error::ResourceConflict);
            }
            allowed(
                &sr_record,
                if *attached {
                    storage_operations::PLUG
                } else {
                    storage_operations::UNPLUG
                },
            )?;
            empty_current(&sr_record)?;
            if !attached {
                no_consumers(session, &sr_record, false).await?;
            }
            Ok(vec![json!(reference)])
        }
        StorageOperation::RemoveAttachment { sr, host } => {
            let (_, sr_record) = attachment_identity(session, &fresh, *sr, *host).await?;
            if PbdRecord::new(&fresh).currently_attached() != false {
                return Err(Error::StorageInUse);
            }
            allowed(&sr_record, storage_operations::PBD_DESTROY)?;
            empty_current(&sr_record)?;
            Ok(vec![json!(reference)])
        }
        StorageOperation::Forget {
            expected_name,
            expected_disks,
        }
        | StorageOperation::Destroy {
            expected_name,
            expected_disks,
        } => {
            let action = if matches!(operation(record)?, StorageOperation::Destroy { .. }) {
                storage_operations::DESTROY
            } else {
                storage_operations::FORGET
            };
            allowed(&fresh, action)?;

            removable(
                session,
                record,
                &reference,
                &fresh,
                expected_name,
                expected_disks,
            )
            .await?;
            Ok(vec![json!(reference)])
        }
    }
}
pub(super) async fn verify(
    session: &Session,
    record: &ResourceOperationRecord,
) -> Result<Option<ResourceTarget>, Error> {
    let op = operation(record)?;
    match op {
        StorageOperation::Trim { expected_name, .. } => {
            if !record.storage_trim_succeeded {
                return Err(Error::Protocol(
                    "trim result is unavailable; no plugin call was repeated",
                ));
            }
            let (_, fresh) = xapi::read(session, record.target).await?;
            if SrRecord::new(&fresh).name_label() != *expected_name {
                return Err(Error::ResourceConflict);
            }
            Ok(None)
        }
        StorageOperation::Create {
            name,
            description,
            location,
            ..
        }
        | StorageOperation::Introduce {
            name,
            description,
            location,
            ..
        } => {
            let baseline = record
                .storage_baseline
                .as_ref()
                .ok_or(Error::Protocol("storage creation baseline unavailable"))?;
            let reference = record.result_reference.as_ref().ok_or(Error::Protocol(
                "storage creation result identity unavailable; no write was repeated",
            ))?;
            let fresh = by_ref(session, "SR", &json!(reference)).await?;
            let id = SrRecord::new(&fresh)
                .uuid()
                .text()
                .and_then(|id| SrId::parse(id).ok())
                .ok_or(Error::Protocol("created storage UUID unavailable"))?;
            if baseline.srs.contains(&id)
                || SrRecord::new(&fresh).name_label() != *name
                || SrRecord::new(&fresh).name_description() != *description
                || SrRecord::new(&fresh).type_() != location.kind()
                || SrRecord::new(&fresh).content_type() != location.content_type()
                || SrRecord::new(&fresh).shared() != location.shared()
            {
                return Err(Error::ResourceConflict);
            }
            if let StorageOperation::Introduce { uuid, .. } = op
                && uuid != &id
            {
                return Err(Error::ResourceConflict);
            }
            if let StorageOperation::Create { host, .. } = op {
                crate::storage_provisioning::verify_allocation(&fresh, location)?;
                let (host_ref, _) = xapi::read(session, ResourceTarget::Host(*host)).await?;
                let mut found = false;
                if !location.shared() && array(&fresh, "PBDs")?.len() != 1 {
                    return Err(Error::ResourceConflict);
                }
                for pbd in array(&fresh, "PBDs")? {
                    let pbd = by_ref(session, "PBD", pbd).await?;
                    let id = PbdRecord::new(&pbd)
                        .uuid()
                        .text()
                        .and_then(|v| PbdId::parse(v).ok())
                        .ok_or(Error::Protocol(
                            "Created storage attachment UUID unavailable",
                        ))?;
                    if baseline.pbds.contains(&id) {
                        return Err(Error::ResourceConflict);
                    }
                    if PbdRecord::new(&pbd).host() == host_ref
                        && PbdRecord::new(&pbd).sr() == *reference
                        && config_matches(
                            PbdRecord::new(&pbd).device_config(),
                            &location.device_config(),
                        )
                    {
                        found = true;
                    }
                }
                if !found {
                    return Err(Error::Protocol(
                        "created storage attachment could not be verified",
                    ));
                }
            }
            Ok(Some(ResourceTarget::Storage(id)))
        }
        StorageOperation::CreateAttachment { host, location } => {
            let baseline = record
                .storage_baseline
                .as_ref()
                .ok_or(Error::Protocol("attachment creation baseline unavailable"))?;
            let reference = record.result_reference.as_ref().ok_or(Error::Protocol(
                "attachment creation result identity unavailable; no write was repeated",
            ))?;
            let fresh = by_ref(session, "PBD", &json!(reference)).await?;
            let id = PbdRecord::new(&fresh)
                .uuid()
                .text()
                .and_then(|id| PbdId::parse(id).ok())
                .ok_or(Error::Protocol("created attachment UUID unavailable"))?;
            let ResourceTarget::Storage(sr) = record.target else {
                return Err(Error::InvalidResourceCommand);
            };
            let (_, sr_record) = attachment_identity(session, &fresh, sr, *host).await?;
            if !location.shared() {
                if SrRecord::new(&sr_record).shared() != false {
                    return Err(Error::ResourceConflict);
                }
                crate::storage_local::attachment_graph(&sr_record, reference)?;
            }
            if PbdRecord::new(&fresh).currently_attached() != false
                || baseline.pbds.contains(&id)
                || !config_matches(
                    PbdRecord::new(&fresh).device_config(),
                    &location.device_config(),
                )
            {
                return Err(Error::ResourceConflict);
            }
            Ok(Some(ResourceTarget::Pbd(id)))
        }
        StorageOperation::SetAttached {
            sr, host, attached, ..
        } => {
            let (_, fresh) = xapi::read(session, record.target).await?;
            attachment_identity(session, &fresh, *sr, *host).await?;
            if PbdRecord::new(&fresh).currently_attached() != *attached {
                return Err(Error::ResourceConflict);
            }
            Ok(None)
        }
        StorageOperation::RemoveAttachment { .. }
        | StorageOperation::Forget { .. }
        | StorageOperation::Destroy { .. } => match xapi::read(session, record.target).await {
            Err(Error::Api(code)) if code == "UUID_INVALID" => Ok(None),
            Ok(_) => Err(Error::ResourceConflict),
            Err(error) => Err(error),
        },
    }
}
pub(super) fn config_matches<'a>(actual: impl Reads<'a, kind::Map>, expected: &Value) -> bool {
    let actual = actual.raw();
    expected.as_object().is_some_and(|expected| {
        expected.iter().all(|(key, value)| {
            actual[key] == *value || (key == "port" && value == "3260" && actual.get(key).is_none())
        })
    })
}

// Only this successful plugin result is evidence of completion; arbitrary backend error XML
// and secrets are never copied into operation history.
pub(super) fn trim_succeeded(task: &Value) -> Result<bool, Error> {
    let xml = TaskRecord::new(task)
        .result()
        .text()
        .filter(|s| s.len() <= 65536)
        .ok_or(Error::Protocol("trim task result unavailable"))?;
    let doc = roxmltree::Document::parse_with_options(
        xml,
        roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: 32,
        },
    )
    .map_err(|_| Error::Protocol("invalid trim task result"))?;
    let root = doc.root_element();
    if root.tag_name().name() != "value" {
        return Err(Error::Protocol("invalid trim task result"));
    }
    let children = root
        .children()
        .filter(|n| n.is_element())
        .collect::<Vec<_>>();
    let text = match children.as_slice() {
        [] => root.text(),
        [child]
            if child.tag_name().name() == "string" && !child.children().any(|n| n.is_element()) =>
        {
            child.text()
        }
        _ => None,
    };
    Ok(text == Some("true"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn trim_proof_requires_positive_boolean_and_never_exposes_plugin_errors() {
        assert!(trim_succeeded(&json!({"result":"<value><string>true</string></value>"})).unwrap());
        assert!(!trim_succeeded(&json!({"result":"<value><string>&lt;private-error&gt;secret&lt;/private-error&gt;</string></value>"})).unwrap());
        for xml in [
            "<!DOCTYPE a><value>true</value>",
            "<value><int>1</int></value>",
            "<value><string>true</string><string>true</string></value>",
        ] {
            assert!(!trim_succeeded(&json!({"result":xml})).unwrap_or(false));
        }
    }
}

#[cfg(test)]
mod port_compatibility_tests {
    use super::*;
    #[test]
    fn missing_legacy_port_is_only_equivalent_to_the_default() {
        let actual = json!({"target":"192.0.2.1"});
        assert!(config_matches(
            &actual,
            &json!({"target":"192.0.2.1","port":"3260"})
        ));
        assert!(!config_matches(
            &actual,
            &json!({"target":"192.0.2.1","port":"3261"})
        ));
        assert!(!config_matches(
            &json!({"target":"192.0.2.1","port":"3261"}),
            &json!({"target":"192.0.2.1","port":"3260"})
        ));
    }
}
