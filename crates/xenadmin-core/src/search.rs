use crate::{Inventory, OrganizationObjectId as Id, PoolId, PowerState};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SearchKind {
    Pool,
    Host,
    Vm,
    Template,
    Snapshot,
    Storage,
    Network,
    Disk,
}
impl SearchKind {
    pub const ALL: [Self; 8] = [
        Self::Pool,
        Self::Host,
        Self::Vm,
        Self::Template,
        Self::Snapshot,
        Self::Storage,
        Self::Network,
        Self::Disk,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Pool => "Pool",
            Self::Host => "Host",
            Self::Vm => "VM",
            Self::Template => "Template",
            Self::Snapshot => "Snapshot",
            Self::Storage => "Storage",
            Self::Network => "Network",
            Self::Disk => "Disk",
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SearchGroup {
    #[default]
    Name,
    Pool,
    Kind,
    Folder,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InventorySearch {
    pub text: String,
    pub kinds: BTreeSet<SearchKind>,
    pub pools: BTreeSet<PoolId>,
    pub power: Option<PowerState>,
    pub host: String,
    pub tags: BTreeSet<String>,
    pub folder: String,
    pub custom_fields: BTreeMap<String, String>,
    pub group: SearchGroup,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchResult {
    pub pool: PoolId,
    pub pool_name: String,
    pub object: Id,
    pub kind: SearchKind,
    pub name: String,
    pub description: String,
    pub host: String,
    pub power: Option<PowerState>,
    pub tags: Option<Vec<String>>,
    pub folder: Option<String>,
    pub metadata_known: bool,
    pub custom_fields: BTreeMap<String, String>,
}
impl SearchResult {
    pub fn key(&self) -> String {
        format!("{}:{}:{}", self.pool, self.kind.label(), self.object.uuid())
    }
}
impl InventorySearch {
    pub fn validate(&self) -> Result<(), String> {
        let texts = [&self.text, &self.host, &self.folder]
            .into_iter()
            .chain(self.tags.iter())
            .chain(self.custom_fields.keys())
            .chain(self.custom_fields.values());
        if self.pools.len() > 100
            || self.tags.len() > 32
            || self.custom_fields.len() > 32
            || texts
                .into_iter()
                .any(|s| s.len() > 512 || s.chars().any(char::is_control))
        {
            return Err(
                "Search filters exceed their size limits or contain control characters".into(),
            );
        }
        if !self.folder.is_empty() && !crate::valid_folder(&self.folder) {
            return Err("Folder must be a full path such as /Services".into());
        }
        if matches!(&self.power, Some(PowerState::Unknown(_))) {
            return Err("Choose a known VM power state".into());
        }
        if self.tags.iter().any(|s| s.trim().is_empty())
            || self.custom_fields.keys().any(|s| s.trim().is_empty())
        {
            return Err("Tag and custom field names cannot be empty".into());
        }
        Ok(())
    }
    pub fn matches(&self, row: &SearchResult) -> bool {
        let contains =
            |haystack: &str, needle: &str| haystack.to_lowercase().contains(&needle.to_lowercase());
        (self.kinds.is_empty() || self.kinds.contains(&row.kind))
            && (self.pools.is_empty() || self.pools.contains(&row.pool))
            && self
                .power
                .as_ref()
                .is_none_or(|state| row.power.as_ref() == Some(state))
            && (self.host.is_empty() || contains(&row.host, &self.host))
            && (self.text.trim().is_empty()
                || [
                    &row.name,
                    &row.description,
                    &row.host,
                    &row.pool_name,
                    &row.object.uuid().to_string(),
                ]
                .iter()
                .any(|s| contains(s, self.text.trim())))
            && self
                .tags
                .iter()
                .all(|tag| row.tags.as_ref().is_some_and(|tags| tags.contains(tag)))
            && (self.folder.is_empty()
                || row.metadata_known
                    && row.folder.as_ref().is_some_and(|f| {
                        f == &self.folder
                            || f.strip_prefix(&self.folder)
                                .is_some_and(|rest| rest.starts_with('/'))
                    }))
            && (self.custom_fields.is_empty()
                || row.metadata_known
                    && self
                        .custom_fields
                        .iter()
                        .all(|(key, value)| row.custom_fields.get(key) == Some(value)))
    }
    pub fn run(&self, inventory: &Inventory) -> Result<Vec<SearchResult>, String> {
        self.validate()?;
        let metadata = inventory
            .details
            .organization
            .objects
            .iter()
            .map(|o| (o.id, o))
            .collect::<BTreeMap<_, _>>();
        let mut rows = Vec::new();
        let mut add = |object, kind, name: &str, description: &str, host: String, power| {
            let meta = metadata.get(&object);
            let row = SearchResult {
                pool: inventory.pool_id,
                pool_name: inventory.pool_name.clone(),
                object,
                kind,
                name: name.into(),
                description: description.into(),
                host,
                power,
                tags: meta.and_then(|m| m.tags.clone()),
                folder: meta.and_then(|m| m.folder.clone()),
                metadata_known: meta.is_some_and(|m| m.other_config_known),
                custom_fields: meta.map(|m| m.custom_fields.clone()).unwrap_or_default(),
            };
            if self.matches(&row) {
                rows.push(row);
            }
        };
        add(
            Id::Pool(inventory.pool_id),
            SearchKind::Pool,
            &inventory.pool_name,
            "",
            String::new(),
            None,
        );
        for h in inventory.hosts.values() {
            add(
                Id::Host(h.id),
                SearchKind::Host,
                &h.name,
                &h.version,
                format!("{} {} {}", h.name, h.address, h.id),
                None,
            );
        }
        for v in inventory.vms.values().filter(|v| !v.is_control_domain) {
            let host = v
                .host
                .map(|id| {
                    inventory
                        .hosts
                        .get(&id)
                        .map(|h| format!("{} {} {}", h.name, h.address, h.id))
                        .unwrap_or_else(|| id.to_string())
                })
                .unwrap_or_default();
            add(
                Id::Vm(v.id),
                if v.is_template {
                    SearchKind::Template
                } else {
                    SearchKind::Vm
                },
                &v.name,
                &v.description,
                host,
                Some(v.power_state.clone()),
            );
        }
        for v in inventory.snapshots.iter() {
            add(
                Id::Snapshot(v.id),
                SearchKind::Snapshot,
                &v.name,
                &v.description,
                String::new(),
                Some(v.power_state.clone()),
            );
        }
        for sr in &inventory.storage {
            add(
                Id::Storage(sr.uuid.into()),
                SearchKind::Storage,
                &sr.name,
                &sr.kind,
                String::new(),
                None,
            );
        }
        for n in &inventory.networks {
            add(
                Id::Network(n.uuid.into()),
                SearchKind::Network,
                &n.name,
                &n.bridge,
                String::new(),
                None,
            );
        }
        for d in inventory.details.vdis.values() {
            add(
                Id::Disk(d.id),
                SearchKind::Disk,
                &d.name,
                &d.description,
                String::new(),
                None,
            );
        }
        self.sort(&mut rows);
        Ok(rows)
    }
    pub fn sort(&self, rows: &mut [SearchResult]) {
        rows.sort_by_cached_key(|r| {
            (
                match self.group {
                    SearchGroup::Name => String::new(),
                    SearchGroup::Pool => format!("{}:{}", r.pool_name.to_lowercase(), r.pool),
                    SearchGroup::Kind => r.kind.label().into(),
                    SearchGroup::Folder => r.folder.clone().unwrap_or_default(),
                },
                r.name.to_lowercase(),
                r.key(),
            )
        });
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedSearch {
    pub id: uuid::Uuid,
    pub name: String,
    pub query: InventorySearch,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchCatalog {
    pub schema_version: u8,
    pub searches: Vec<SavedSearch>,
}
impl Default for SearchCatalog {
    fn default() -> Self {
        Self {
            schema_version: 1,
            searches: Vec::new(),
        }
    }
}
impl SearchCatalog {
    pub const MAX_BYTES: usize = 1024 * 1024;
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 || self.searches.len() > 100 {
            return Err("Unsupported saved-search version or more than 100 definitions".into());
        }
        let mut ids = BTreeSet::new();
        let mut names = BTreeSet::new();
        for s in &self.searches {
            if s.id.is_nil()
                || !ids.insert(s.id)
                || s.name.trim() != s.name
                || s.name.is_empty()
                || s.name.len() > 100
                || s.name.chars().any(char::is_control)
                || !names.insert(s.name.to_lowercase())
            {
                return Err(
                    "Search names and identities must be nonempty, bounded and unique".into(),
                );
            }
            s.query.validate()?;
        }
        Ok(())
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > Self::MAX_BYTES {
            return Err("Saved searches exceed 1 MiB".into());
        }
        let catalog: Self = serde_json::from_slice(bytes).map_err(
            |_| "Invalid XenAdmin RS search JSON (legacy XenCenter XML is not supported)",
        )?;
        catalog.validate()?;
        Ok(catalog)
    }
    pub fn encode(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        let bytes = serde_json::to_vec_pretty(self).map_err(|_| "Cannot encode saved searches")?;
        if bytes.len() > Self::MAX_BYTES {
            return Err("Saved searches exceed 1 MiB".into());
        }
        Ok(bytes)
    }
    pub fn merged(&self, imported: &Self) -> Result<Self, String> {
        self.validate()?;
        imported.validate()?;
        let mut next = self.clone();
        for s in &imported.searches {
            if next.searches.contains(s) {
                continue;
            }
            if next
                .searches
                .iter()
                .any(|v| v.id == s.id || v.name.eq_ignore_ascii_case(&s.name))
            {
                return Err(format!(
                    "Conflicting saved search: {}. Rename or remove the local definition first.",
                    s.name
                ));
            }
            next.searches.push(s.clone());
        }
        next.validate()?;
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn row() -> SearchResult {
        SearchResult {
            pool: uuid::Uuid::from_u128(1).into(),
            pool_name: "Pool one".into(),
            object: Id::Vm(uuid::Uuid::from_u128(2).into()),
            kind: SearchKind::Vm,
            name: "Database".into(),
            description: "Primary database".into(),
            host: "host-one 192.0.2.1".into(),
            power: Some(PowerState::Running),
            tags: Some(vec!["production".into(), "tier,one".into()]),
            folder: Some("/Services/Database".into()),
            metadata_known: true,
            custom_fields: BTreeMap::from([("Owner".into(), "Operations".into())]),
        }
    }
    #[test]
    fn all_filters_intersect_without_treating_missing_metadata_as_a_match() {
        let mut r = row();
        let q = InventorySearch {
            text: "DATABASE".into(),
            host: "192.0.2".into(),
            kinds: BTreeSet::from([SearchKind::Vm]),
            pools: BTreeSet::from([r.pool]),
            power: Some(PowerState::Running),
            tags: BTreeSet::from(["production".into()]),
            folder: "/Services".into(),
            custom_fields: r.custom_fields.clone(),
            ..Default::default()
        };
        assert!(q.matches(&r));
        r.tags = None;
        assert!(!q.matches(&r));
        r.tags = row().tags;
        r.metadata_known = false;
        assert!(!q.matches(&r));
        r.metadata_known = true;
        r.folder = Some("/Services-old/Database".into());
        assert!(!q.matches(&r));
        r.folder = row().folder;
        r.power = Some(PowerState::Halted);
        assert!(!q.matches(&r));
        r.power = row().power;
        r.pool = uuid::Uuid::from_u128(3).into();
        assert!(!q.matches(&r));
    }
    #[test]
    fn equal_object_uuids_in_different_pools_or_classes_are_distinct() {
        let a = row();
        let mut b = a.clone();
        b.pool = uuid::Uuid::from_u128(3).into();
        assert_ne!(a.key(), b.key());
        b = a.clone();
        b.object = Id::Host(a.object.uuid().into());
        b.kind = SearchKind::Host;
        assert_ne!(a.key(), b.key());
    }
    fn catalog() -> SearchCatalog {
        SearchCatalog {
            schema_version: 1,
            searches: vec![SavedSearch {
                id: uuid::Uuid::from_u128(5),
                name: "Production".into(),
                query: InventorySearch {
                    tags: BTreeSet::from(["tier,one".into()]),
                    custom_fields: BTreeMap::from([
                        ("Owner".into(), "Operations".into()),
                        ("Service".into(), "Database".into()),
                    ]),
                    ..Default::default()
                },
            }],
        }
    }
    #[test]
    fn saved_search_roundtrip_preserves_filters_and_never_contains_inventory() {
        let c = catalog();
        let bytes = c.encode().unwrap();
        assert_eq!(SearchCatalog::decode(&bytes).unwrap(), c);
        let text = String::from_utf8(bytes).unwrap();
        assert!(!text.contains("host-one"));
        assert!(!text.contains("power_state"));
        assert_eq!(c.merged(&c).unwrap(), c);
        let mut conflict = c.clone();
        conflict.searches[0].query.text = "Different".into();
        assert!(c.merged(&conflict).is_err());
        let mut addition = c.clone();
        addition.searches[0].id = uuid::Uuid::from_u128(6);
        addition.searches[0].name = "Other".into();
        assert_eq!(c.merged(&addition).unwrap().searches.len(), 2);
    }
    #[test]
    fn malformed_or_unbounded_searches_are_rejected() {
        assert!(SearchCatalog::decode(&vec![b' '; SearchCatalog::MAX_BYTES + 1]).is_err());
        let mut c = catalog();
        c.schema_version = 2;
        assert!(c.encode().is_err());
        c = catalog();
        c.searches[0].query.text = "a".repeat(513);
        assert!(c.encode().is_err());
        c = catalog();
        c.searches[0].query.folder = "/a/../b".into();
        assert!(c.encode().is_err());
        c = catalog();
        c.searches.push(c.searches[0].clone());
        assert!(c.encode().is_err());
        assert!(
            SearchCatalog::decode(br#"{"schema_version":1,"searches":[],"code":"no"}"#).is_err()
        );
        assert!(SearchCatalog::decode(b"<Search/>").is_err());
    }
}
