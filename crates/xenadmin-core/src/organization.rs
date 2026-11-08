use crate::{Error, HostId, NetworkId, PoolId, SnapshotId, SrId, VdiId, VmId};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) const FIELD_KEY: &str = "XenCenter.CustomFields";
pub(crate) const FIELD_PREFIX: &str = "XenCenter.CustomFields.";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum OrganizationObjectId {
    Pool(PoolId),
    Host(HostId),
    Vm(VmId),
    Snapshot(SnapshotId),
    Storage(SrId),
    Network(NetworkId),
    Disk(VdiId),
}
impl OrganizationObjectId {
    pub(crate) fn class(self) -> &'static str {
        match self {
            Self::Pool(_) => "pool",
            Self::Host(_) => "host",
            Self::Vm(_) | Self::Snapshot(_) => "VM",
            Self::Storage(_) => "SR",
            Self::Network(_) => "network",
            Self::Disk(_) => "VDI",
        }
    }
    pub fn uuid(self) -> uuid::Uuid {
        match self {
            Self::Pool(id) => id.uuid(),
            Self::Host(id) => id.uuid(),
            Self::Vm(id) => id.uuid(),
            Self::Snapshot(id) => id.uuid(),
            Self::Storage(id) => id.uuid(),
            Self::Network(id) => id.uuid(),
            Self::Disk(id) => id.uuid(),
        }
    }
    pub fn kind(self) -> &'static str {
        match self {
            Self::Pool(_) => "Pool",
            Self::Host(_) => "Host",
            Self::Vm(_) => "VM / template",
            Self::Snapshot(_) => "Snapshot",
            Self::Storage(_) => "Storage",
            Self::Network(_) => "Network",
            Self::Disk(_) => "Disk",
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct OrganizationInventory {
    #[serde(default)]
    pub complete: bool,
    pub objects: Vec<OrganizationObject>,
    pub empty_folders: Option<Vec<String>>,
    pub definitions: Option<Vec<CustomFieldDefinition>>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OrganizationObject {
    pub id: OrganizationObjectId,
    pub name: String,
    pub tags: Option<Vec<String>>,
    pub folder: Option<String>,
    pub other_config_known: bool,
    pub custom_fields: BTreeMap<String, String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CustomFieldType {
    String,
    Date,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomFieldDefinition {
    pub name: String,
    pub kind: CustomFieldType,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrganizationChange {
    Tags {
        previous: Vec<String>,
        value: Vec<String>,
    },
    Folder {
        previous: Option<String>,
        value: Option<String>,
    },
    EmptyFolders {
        previous: Vec<String>,
        value: Vec<String>,
    },
    Definitions {
        previous: Vec<CustomFieldDefinition>,
        value: Vec<CustomFieldDefinition>,
    },
    CustomField {
        definition: CustomFieldDefinition,
        previous: Option<String>,
        value: Option<String>,
    },
}
impl OrganizationChange {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Tags { .. } => "Set tags",
            Self::Folder { .. } => "Set folder",
            Self::EmptyFolders { .. } => "Set empty folders",
            Self::Definitions { .. } => "Set custom field definitions",
            Self::CustomField { .. } => "Set custom field value",
        }
    }
    pub(crate) fn field_key(&self) -> Option<(&'static str, String)> {
        match self {
            Self::Tags { .. } => None,
            Self::Folder { .. } => Some(("other_config", "folder".into())),
            Self::EmptyFolders { .. } => Some(("other_config", "EMPTY_FOLDERS".into())),
            Self::Definitions { .. } => Some(("gui_config", FIELD_KEY.into())),
            Self::CustomField { definition, .. } => {
                Some(("other_config", format!("{FIELD_PREFIX}{}", definition.name)))
            }
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrganizationEdit {
    pub object: OrganizationObjectId,
    pub change: OrganizationChange,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrganizationPatch {
    edits: Vec<OrganizationEdit>,
}
impl OrganizationPatch {
    pub fn new(edits: Vec<OrganizationEdit>) -> Result<Self, Error> {
        let result = Self { edits };
        result.validate()?;
        Ok(result)
    }
    pub fn edits(&self) -> &[OrganizationEdit] {
        &self.edits
    }
    pub(crate) fn validate(&self) -> Result<(), Error> {
        if self.edits.is_empty() || self.edits.len() > 256 {
            return Err(Error::InvalidResourceCommand);
        }
        let mut seen = BTreeSet::new();
        for edit in &self.edits {
            let valid = match &edit.change {
                OrganizationChange::Tags { previous, value } => {
                    valid_tags(previous) && valid_tags(value) && tag_set(previous) != tag_set(value)
                }
                OrganizationChange::Folder { previous, value } => {
                    previous.as_deref().is_none_or(valid_folder)
                        && value.as_deref().is_none_or(valid_folder)
                        && previous != value
                }
                OrganizationChange::EmptyFolders { previous, value } => {
                    matches!(edit.object, OrganizationObjectId::Pool(_))
                        && valid_folders(previous)
                        && valid_folders(value)
                        && tag_set(previous) != tag_set(value)
                }
                OrganizationChange::Definitions { previous, value } => {
                    matches!(edit.object, OrganizationObjectId::Pool(_))
                        && valid_definitions(previous)
                        && valid_definitions(value)
                        && previous != value
                }
                OrganizationChange::CustomField {
                    definition,
                    previous,
                    value,
                } => {
                    valid_name(&definition.name)
                        && previous
                            .as_ref()
                            .is_none_or(|v| v.len() <= 4096 && !v.contains('\0'))
                        && value
                            .as_deref()
                            .is_none_or(|v| valid_field_value(definition.kind, v))
                        && previous != value
                }
            };
            let key = edit
                .change
                .field_key()
                .map(|(f, k)| format!("{f}:{k}"))
                .unwrap_or_else(|| "tags".into());
            if !valid || !seen.insert((edit.object, key)) {
                return Err(Error::InvalidResourceCommand);
            }
        }
        Ok(())
    }
}
pub fn valid_folder(value: &str) -> bool {
    value.starts_with('/')
        && value.len() <= 1024
        && value != "/"
        && !value.contains(';')
        && value
            .split('/')
            .skip(1)
            .all(|part| valid_name(part) && part != "." && part != "..")
}
pub(crate) fn valid_name(value: &str) -> bool {
    !value.is_empty()
        && value.trim() == value
        && value.chars().count() <= 255
        && !value.chars().any(char::is_control)
}
fn valid_tags(values: &[String]) -> bool {
    values.len() <= 256
        && values.iter().all(|v| valid_name(v))
        && tag_set(values).len() == values.len()
}
fn valid_folders(values: &[String]) -> bool {
    values.len() <= 256
        && values.iter().all(|v| valid_folder(v))
        && tag_set(values).len() == values.len()
}
fn valid_definitions(values: &[CustomFieldDefinition]) -> bool {
    values.len() <= 128
        && values.iter().all(|d| valid_name(&d.name))
        && values
            .iter()
            .map(|d| &d.name)
            .collect::<BTreeSet<_>>()
            .len()
            == values.len()
}
pub(crate) fn tag_set(values: &[String]) -> BTreeSet<&str> {
    values.iter().map(String::as_str).collect()
}
fn valid_field_value(kind: CustomFieldType, value: &str) -> bool {
    if value.len() > 4096 || value.contains('\0') {
        return false;
    }
    if kind == CustomFieldType::String {
        return true;
    }
    // Use an unambiguous date representation shared by the reference client's parser.
    let bytes = value.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes
            .iter()
            .enumerate()
            .any(|(i, c)| i != 4 && i != 7 && !c.is_ascii_digit())
    {
        return false;
    }
    let year = value[..4].parse::<u32>().unwrap_or(0);
    let month = value[5..7].parse::<usize>().unwrap_or(0);
    let day = value[8..].parse::<u32>().unwrap_or(0);
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let days = [
        0,
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    year > 0 && month > 0 && month < 13 && day > 0 && day <= days[month]
}
pub(crate) fn tags(record: &Value) -> Option<Vec<String>> {
    let array = record["tags"].as_array().filter(|a| a.len() <= 256)?;
    let result = array
        .iter()
        .map(|v| v.as_str().map(str::to_owned))
        .collect::<Option<Vec<_>>>()?;
    valid_tags(&result).then_some(result)
}
pub(crate) fn folders(value: Option<&str>) -> Option<Vec<String>> {
    let value = value.unwrap_or("");
    if value.len() > 262_144 {
        return None;
    }
    let result: Vec<_> = value
        .split(';')
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
        .collect();
    valid_folders(&result).then_some(result)
}
pub(crate) fn definitions(value: Option<&str>) -> Option<Vec<CustomFieldDefinition>> {
    let Some(xml) = value else {
        return Some(Vec::new());
    };
    if xml.len() > 131_072 {
        return None;
    }
    let document = roxmltree::Document::parse_with_options(
        xml,
        roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: 512,
        },
    )
    .ok()?;
    let root = document.root_element();
    if root.tag_name().name() != "CustomFieldDefinitions" {
        return None;
    }
    let mut result = Vec::new();
    for node in root.children().filter(|n| n.is_element()) {
        if node.tag_name().name() != "CustomFieldDefinition"
            || node
                .attributes()
                .any(|a| !["name", "type", "defaultValue"].contains(&a.name()))
            || node.children().any(|n| n.is_element())
            || node
                .attribute("defaultValue")
                .is_some_and(|v| !v.is_empty())
        {
            return None;
        }
        result.push(CustomFieldDefinition {
            name: node.attribute("name")?.to_owned(),
            kind: match node.attribute("type")? {
                "String" => CustomFieldType::String,
                "Date" => CustomFieldType::Date,
                _ => return None,
            },
        });
    }
    valid_definitions(&result).then_some(result)
}
pub(crate) fn definitions_xml(defs: &[CustomFieldDefinition]) -> String {
    let escape = |s: &str| {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&apos;")
    };
    let mut xml = String::from("<CustomFieldDefinitions>");
    for d in defs {
        xml.push_str(&format!(
            "<CustomFieldDefinition name=\"{}\" type=\"{}\" defaultValue=\"\"/>",
            escape(&d.name),
            match d.kind {
                CustomFieldType::String => "String",
                CustomFieldType::Date => "Date",
            }
        ));
    }
    xml.push_str("</CustomFieldDefinitions>");
    xml
}
pub(crate) fn object(id: OrganizationObjectId, record: &Value) -> OrganizationObject {
    let config = record["other_config"].as_object();
    let known = config.is_some_and(|m| {
        m.len() <= 4096
            && m.iter()
                .filter(|(k, _)| *k == "folder" || k.starts_with(FIELD_PREFIX))
                .all(|(_, v)| {
                    v.as_str()
                        .is_some_and(|v| v.len() <= 4096 && !v.contains('\0'))
                })
    });
    OrganizationObject {
        id,
        name: record["name_label"]
            .as_str()
            .unwrap_or("")
            .chars()
            .take(1024)
            .collect(),
        tags: tags(record),
        folder: record["other_config"]["folder"]
            .as_str()
            .filter(|v| !v.is_empty())
            .map(str::to_owned),
        other_config_known: known,
        custom_fields: if known {
            config
                .into_iter()
                .flat_map(|m| m.iter())
                .filter_map(|(k, v)| {
                    Some((
                        k.strip_prefix(FIELD_PREFIX)?.to_owned(),
                        v.as_str()?.to_owned(),
                    ))
                })
                .collect()
        } else {
            BTreeMap::new()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn custom_field_xml_is_bounded_and_roundtrips_escaped_names_without_entities() {
        let defs = vec![
            CustomFieldDefinition {
                name: "Owner <primary> & \"backup\"".into(),
                kind: CustomFieldType::String,
            },
            CustomFieldDefinition {
                name: "Expires".into(),
                kind: CustomFieldType::Date,
            },
        ];
        assert_eq!(definitions(Some(&definitions_xml(&defs))), Some(defs));
        assert!(definitions(Some("<!DOCTYPE x [<!ENTITY secret SYSTEM 'file:///tmp/private'>]><CustomFieldDefinitions>&secret;</CustomFieldDefinitions>")).is_none());
        assert!(definitions(Some("<CustomFieldDefinitions><CustomFieldDefinition name=\"X\" type=\"Unknown\"/></CustomFieldDefinitions>")).is_none());
        assert!(definitions(Some(&" ".repeat(131_073))).is_none());
    }
    #[test]
    fn invalid_dates_paths_duplicate_keys_and_oversized_batches_are_refused() {
        let target = OrganizationObjectId::Vm(VmId::from(uuid::Uuid::from_u128(1)));
        let make = |value: &str| OrganizationEdit {
            object: target,
            change: OrganizationChange::CustomField {
                definition: CustomFieldDefinition {
                    name: "Date".into(),
                    kind: CustomFieldType::Date,
                },
                previous: None,
                value: Some(value.into()),
            },
        };
        assert!(OrganizationPatch::new(vec![make("2024-02-29")]).is_ok());
        for bad in [
            "2025-02-29",
            "2024-04-31",
            "0000-01-01",
            "2024-00-01",
            "2024-13-01",
            "2024-2-1",
            "garbage",
        ] {
            assert!(OrganizationPatch::new(vec![make(bad)]).is_err());
        }
        assert!(OrganizationPatch::new(vec![make("2024-01-01"), make("2024-01-02")]).is_err());
        assert!(OrganizationPatch::new(vec![make("2024-01-01"); 257]).is_err());
        for bad in ["relative", "/", "/a//b", "/a/../b", "/a;z", "/a/"] {
            assert!(!valid_folder(bad));
        }
        assert!(valid_folder("/Services/Database"));
    }
}
