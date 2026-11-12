use super::{
    ResourceAction, ResourceOperationRecord, ResourceTarget,
    xapi::{Check, Step},
};
use crate::{
    Error, OrganizationChange, OrganizationEdit, OrganizationObjectId,
    organization::{self, FIELD_KEY, FIELD_PREFIX},
    xapi::Session,
};
use serde_json::{Value, json};
use xenadmin_xapi::records::VmRecord;

async fn read(session: &Session, id: OrganizationObjectId) -> Result<(String, Value), Error> {
    let reference = super::xapi::reply_reference(
        &session
            .call(
                &format!("{}.get_by_uuid", id.class()),
                vec![json!(id.uuid().to_string())],
            )
            .await?,
    )?;
    let fresh = session
        .call(
            &format!("{}.get_record", id.class()),
            vec![json!(&reference)],
        )
        .await?;
    // A record of whichever class `id` names; every class has a `uuid`.
    if fresh["uuid"] != id.uuid().to_string() {
        return Err(Error::Stale);
    }
    if matches!(
        id,
        OrganizationObjectId::Vm(_) | OrganizationObjectId::Snapshot(_)
    ) && (VmRecord::new(&fresh).is_control_domain() != false
        || VmRecord::new(&fresh).is_a_snapshot().as_bool()
            != Some(matches!(id, OrganizationObjectId::Snapshot(_))))
    {
        return Err(Error::UnavailableAction);
    }
    Ok((reference, fresh))
}
fn option_string(record: &Value, field: &str, key: &str) -> Result<Option<String>, Error> {
    let map = record[field]
        .as_object()
        .filter(|m| m.len() <= 4096)
        .ok_or(Error::Protocol("organization metadata unavailable"))?;
    map.get(key)
        .map(|v| {
            v.as_str()
                .filter(|v| v.len() <= 262_144 && !v.contains('\0'))
                .map(str::to_owned)
                .ok_or(Error::Protocol("invalid organization metadata"))
        })
        .transpose()
}
fn matches_change(edit: &OrganizationEdit, record: &Value, desired: bool) -> Result<bool, Error> {
    Ok(match &edit.change {
        OrganizationChange::Tags { previous, value } => {
            organization::tags(record).is_some_and(|v| {
                organization::tag_set(&v)
                    == organization::tag_set(if desired { value } else { previous })
            })
        }
        OrganizationChange::Folder { previous, value } => {
            option_string(record, "other_config", "folder")?.filter(|v| !v.is_empty())
                == *if desired { value } else { previous }
        }
        OrganizationChange::EmptyFolders { previous, value } => organization::folders(
            option_string(record, "other_config", "EMPTY_FOLDERS")?.as_deref(),
        )
        .is_some_and(|v| {
            organization::tag_set(&v)
                == organization::tag_set(if desired { value } else { previous })
        }),
        OrganizationChange::Definitions { previous, value } => {
            organization::definitions(option_string(record, "gui_config", FIELD_KEY)?.as_deref())
                .as_ref()
                == Some(if desired { value } else { previous })
        }
        OrganizationChange::CustomField {
            definition,
            previous,
            value,
        } => {
            option_string(
                record,
                "other_config",
                &format!("{FIELD_PREFIX}{}", definition.name),
            )? == *if desired { value } else { previous }
        }
    })
}
fn desired_value(change: &OrganizationChange) -> Value {
    match change {
        OrganizationChange::Tags { value, .. } => json!(value),
        OrganizationChange::Folder { value, .. }
        | OrganizationChange::CustomField { value, .. } => json!(value),
        OrganizationChange::EmptyFolders { value, .. } => json!(value.join(";")),
        OrganizationChange::Definitions { value, .. } => {
            json!(organization::definitions_xml(value))
        }
    }
}
fn step(edit: &OrganizationEdit) -> Step {
    let field = edit
        .change
        .field_key()
        .map(|(field, _)| field)
        .unwrap_or("tags");
    Step {
        label: format!(
            "{} · {} {}",
            edit.change.label(),
            edit.object.kind(),
            edit.object.uuid()
        ),
        method: format!("{}.set_{field}", edit.object.class()),
        argument: None,
        expected: Value::Null,
        check: Check::Organization(edit.clone()),
    }
}
impl crate::OrganizationPatch {
    pub fn permission_requirements(&self) -> crate::PermissionRequirements {
        let patch = self;
        crate::PermissionRequirements::writes(patch.edits().iter().map(|edit| step(edit).method))
    }
}
pub(super) async fn prepare(
    session: &Session,
    record: &ResourceOperationRecord,
) -> Result<Vec<Step>, Error> {
    let ResourceAction::Organization(patch) = &record.action else {
        return Err(Error::InvalidResourceCommand);
    };
    patch.validate()?;
    for (index, edit) in patch.edits().iter().enumerate() {
        let (_, fresh) = read(session, edit.object).await?;
        if !matches_change(edit, &fresh, false)? {
            return Err(Error::ResourceConflict);
        }
        if let OrganizationObjectId::Pool(pool) = edit.object
            && pool != record.pool
        {
            return Err(Error::Stale);
        }
        check_definition(session, record, edit).await?;
        check_removed_definitions(session, edit, &patch.edits()[..index]).await?;
    }
    Ok(patch.edits().iter().map(step).collect())
}
async fn check_definition(
    session: &Session,
    record: &ResourceOperationRecord,
    edit: &OrganizationEdit,
) -> Result<(), Error> {
    if let OrganizationChange::CustomField { definition, .. } = &edit.change {
        let (_, pool) = super::xapi::read(session, ResourceTarget::Pool(record.pool)).await?;
        let definitions =
            organization::definitions(option_string(&pool, "gui_config", FIELD_KEY)?.as_deref())
                .ok_or(Error::Protocol("custom field definitions unavailable"))?;
        if !definitions.contains(definition) {
            return Err(Error::ResourceConflict);
        }
    }
    Ok(())
}
pub(super) async fn arguments(
    session: &Session,
    record: &ResourceOperationRecord,
    edit: &OrganizationEdit,
) -> Result<Vec<Value>, Error> {
    let (reference, fresh) = read(session, edit.object).await?;
    if !matches_change(edit, &fresh, false)? {
        return Err(Error::ResourceConflict);
    }
    check_definition(session, record, edit).await?;
    check_removed_definitions(session, edit, &[]).await?;
    let mut value = desired_value(&edit.change);
    if let Some((field, key)) = edit.change.field_key() {
        let mut map = fresh[field]
            .as_object()
            .filter(|m| m.len() <= 4096)
            .cloned()
            .ok_or(Error::Protocol("organization map unavailable"))?;
        if value.is_null() {
            map.remove(&key);
        } else {
            map.insert(key, value);
        }
        value = Value::Object(map);
    }
    Ok(vec![json!(reference), value])
}
pub(super) async fn verify(
    session: &Session,
    edit: &OrganizationEdit,
) -> Result<Option<ResourceTarget>, Error> {
    let (_, fresh) = read(session, edit.object).await?;
    if matches_change(edit, &fresh, true)? {
        Ok(None)
    } else {
        Err(Error::ResourceConflict)
    }
}
pub(super) fn checks(record: &ResourceOperationRecord) -> Vec<Check> {
    match &record.action {
        ResourceAction::Organization(patch) => patch
            .edits()
            .iter()
            .cloned()
            .map(Check::Organization)
            .collect(),
        _ => Vec::new(),
    }
}

async fn check_removed_definitions(
    session: &Session,
    edit: &OrganizationEdit,
    earlier: &[OrganizationEdit],
) -> Result<(), Error> {
    let OrganizationChange::Definitions { previous, value } = &edit.change else {
        return Ok(());
    };
    let removed = previous
        .iter()
        .filter(|d| !value.contains(d))
        .collect::<Vec<_>>();
    if removed.is_empty() {
        return Ok(());
    }
    for class in ["pool", "host", "VM", "SR", "network", "VDI"] {
        let values = session
            .call(&format!("{class}.get_all_records"), vec![])
            .await?;
        let records = values
            .as_object()
            .filter(|r| r.len() <= 50_000)
            .ok_or(Error::Protocol(
                "custom field consumer inventory unavailable",
            ))?;
        for record in records.values() {
            let uuid = record["uuid"]
                .as_str()
                .and_then(|s| uuid::Uuid::parse_str(s).ok())
                .ok_or(Error::Protocol(
                    "custom field consumer identity unavailable",
                ))?;
            let config = record["other_config"].as_object().ok_or(Error::Protocol(
                "custom field consumer metadata unavailable",
            ))?;
            for definition in &removed {
                if let Some(current) = config.get(&format!("{FIELD_PREFIX}{}", definition.name)) {
                    let planned=earlier.iter().any(|e| e.object.class()==class && e.object.uuid()==uuid && matches!(&e.change,OrganizationChange::CustomField{definition:d,previous:Some(p),value:None} if d==*definition && current==p));
                    if !planned {
                        return Err(Error::Protocol(
                            "custom field still has values outside this reviewed cleanup; refresh before deleting its definition",
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}
