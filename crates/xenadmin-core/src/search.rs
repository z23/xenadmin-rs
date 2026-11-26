use crate::{HostId, Inventory, OrganizationObjectId as Id, PoolId, PowerState};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};

fn contains_ci(haystack: &str, needle_lower: &str) -> bool {
    if needle_lower.is_empty() {
        return true;
    }
    if haystack.is_ascii() && needle_lower.is_ascii() {
        let needle = needle_lower.as_bytes();
        haystack
            .as_bytes()
            .windows(needle.len())
            .any(|window| window.eq_ignore_ascii_case(needle))
    } else {
        haystack.to_lowercase().contains(needle_lower)
    }
}

fn folder_matches(folder: &str, filter: &str) -> bool {
    folder == filter
        || folder
            .strip_prefix(filter)
            .is_some_and(|rest| rest.starts_with('/'))
}

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
        if !(self.kinds.is_empty() || self.kinds.contains(&row.kind))
            || !(self.pools.is_empty() || self.pools.contains(&row.pool))
            || !self
                .power
                .as_ref()
                .is_none_or(|state| row.power.as_ref() == Some(state))
        {
            return false;
        }
        if !self.host.is_empty() {
            let host_needle = self.host.to_lowercase();
            if !contains_ci(&row.host, &host_needle) {
                return false;
            }
        }
        let text = self.text.trim();
        if !text.is_empty() {
            let text_needle = text.to_lowercase();
            let mut uuid_buf = uuid::Uuid::encode_buffer();
            let uuid_text = row.object.uuid().hyphenated().encode_lower(&mut uuid_buf);
            let hit = contains_ci(&row.name, &text_needle)
                || contains_ci(&row.description, &text_needle)
                || contains_ci(&row.host, &text_needle)
                || contains_ci(&row.pool_name, &text_needle)
                || contains_ci(uuid_text, &text_needle);
            if !hit {
                return false;
            }
        }
        if !self
            .tags
            .iter()
            .all(|tag| row.tags.as_ref().is_some_and(|tags| tags.contains(tag)))
        {
            return false;
        }
        if !self.folder.is_empty()
            && !(row.metadata_known
                && row
                    .folder
                    .as_deref()
                    .is_some_and(|f| folder_matches(f, &self.folder)))
        {
            return false;
        }
        self.custom_fields.is_empty()
            || row.metadata_known
                && self
                    .custom_fields
                    .iter()
                    .all(|(key, value)| row.custom_fields.get(key) == Some(value))
    }
    pub fn run(&self, inventory: &Inventory) -> Result<Vec<SearchResult>, String> {
        self.validate()?;
        if !self.pools.is_empty() && !self.pools.contains(&inventory.pool_id) {
            return Ok(Vec::new());
        }
        let want_kind = |kind: SearchKind| self.kinds.is_empty() || self.kinds.contains(&kind);
        let host_needle = if self.host.is_empty() {
            String::new()
        } else {
            self.host.to_lowercase()
        };
        let text_needle = {
            let trimmed = self.text.trim();
            if trimmed.is_empty() {
                String::new()
            } else {
                trimmed.to_lowercase()
            }
        };
        let pool_name_matches =
            !text_needle.is_empty() && contains_ci(&inventory.pool_name, &text_needle);
        let mut metadata = HashMap::with_capacity(inventory.details.organization.objects.len());
        for object in &inventory.details.organization.objects {
            metadata.insert(object.id, object);
        }
        let mut host_labels: HashMap<HostId, String> = HashMap::new();
        if want_kind(SearchKind::Host)
            || want_kind(SearchKind::Vm)
            || want_kind(SearchKind::Template)
        {
            host_labels.reserve(inventory.hosts.len());
            for h in inventory.hosts.values() {
                host_labels.insert(h.id, format!("{} {} {}", h.name, h.address, h.id));
            }
        }
        let mut rows = Vec::new();
        let mut maybe_add = |object: Id,
                             kind: SearchKind,
                             name: &str,
                             description: &str,
                             host: &str,
                             power: Option<&PowerState>| {
            if !self.power.as_ref().is_none_or(|state| power == Some(state)) {
                return;
            }
            if !host_needle.is_empty() && !contains_ci(host, &host_needle) {
                return;
            }
            if !text_needle.is_empty() && !pool_name_matches {
                let mut uuid_buf = uuid::Uuid::encode_buffer();
                let uuid_text = object.uuid().hyphenated().encode_lower(&mut uuid_buf);
                let hit = contains_ci(name, &text_needle)
                    || contains_ci(description, &text_needle)
                    || contains_ci(host, &text_needle)
                    || contains_ci(uuid_text, &text_needle);
                if !hit {
                    return;
                }
            }
            let meta = metadata.get(&object).copied();
            let metadata_known = meta.is_some_and(|m| m.other_config_known);
            if !self.tags.is_empty()
                && !self.tags.iter().all(|tag| {
                    meta.and_then(|m| m.tags.as_ref())
                        .is_some_and(|tags| tags.contains(tag))
                })
            {
                return;
            }
            if !self.folder.is_empty()
                && !(metadata_known
                    && meta
                        .and_then(|m| m.folder.as_deref())
                        .is_some_and(|f| folder_matches(f, &self.folder)))
            {
                return;
            }
            if !self.custom_fields.is_empty()
                && !(metadata_known
                    && meta.is_some_and(|m| {
                        self.custom_fields
                            .iter()
                            .all(|(key, value)| m.custom_fields.get(key) == Some(value))
                    }))
            {
                return;
            }
            rows.push(SearchResult {
                pool: inventory.pool_id,
                pool_name: inventory.pool_name.clone(),
                object,
                kind,
                name: name.into(),
                description: description.into(),
                host: host.into(),
                power: power.cloned(),
                tags: meta.and_then(|m| m.tags.clone()),
                folder: meta.and_then(|m| m.folder.clone()),
                metadata_known,
                custom_fields: meta.map(|m| m.custom_fields.clone()).unwrap_or_default(),
            });
        };
        if self.power.is_none() && want_kind(SearchKind::Pool) {
            maybe_add(
                Id::Pool(inventory.pool_id),
                SearchKind::Pool,
                &inventory.pool_name,
                "",
                "",
                None,
            );
        }
        if self.power.is_none() && want_kind(SearchKind::Host) {
            for h in inventory.hosts.values() {
                let host = host_labels.get(&h.id).map(String::as_str).unwrap_or("");
                maybe_add(
                    Id::Host(h.id),
                    SearchKind::Host,
                    &h.name,
                    &h.version,
                    host,
                    None,
                );
            }
        }
        if want_kind(SearchKind::Vm) || want_kind(SearchKind::Template) {
            for v in inventory.vms.values().filter(|v| !v.is_control_domain) {
                let kind = if v.is_template {
                    SearchKind::Template
                } else {
                    SearchKind::Vm
                };
                if !want_kind(kind) {
                    continue;
                }
                let fallback_host;
                let host = match v.host {
                    Some(id) => match host_labels.get(&id) {
                        Some(label) => label.as_str(),
                        None => {
                            fallback_host = id.to_string();
                            fallback_host.as_str()
                        }
                    },
                    None => "",
                };
                maybe_add(
                    Id::Vm(v.id),
                    kind,
                    &v.name,
                    &v.description,
                    host,
                    Some(&v.power_state),
                );
            }
        }
        if want_kind(SearchKind::Snapshot) {
            for v in inventory.snapshots.iter() {
                maybe_add(
                    Id::Snapshot(v.id),
                    SearchKind::Snapshot,
                    &v.name,
                    &v.description,
                    "",
                    Some(&v.power_state),
                );
            }
        }
        if self.power.is_none() && want_kind(SearchKind::Storage) {
            for sr in &inventory.storage {
                maybe_add(
                    Id::Storage(sr.uuid.into()),
                    SearchKind::Storage,
                    &sr.name,
                    &sr.kind,
                    "",
                    None,
                );
            }
        }
        if self.power.is_none() && want_kind(SearchKind::Network) {
            for n in &inventory.networks {
                maybe_add(
                    Id::Network(n.uuid.into()),
                    SearchKind::Network,
                    &n.name,
                    &n.bridge,
                    "",
                    None,
                );
            }
        }
        if self.power.is_none() && want_kind(SearchKind::Disk) {
            for d in inventory.details.vdis.values() {
                maybe_add(
                    Id::Disk(d.id),
                    SearchKind::Disk,
                    &d.name,
                    &d.description,
                    "",
                    None,
                );
            }
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
    #[test]
    fn run_filters_match_borrowed_predicates_including_unicode_uuid_and_metadata() {
        let pool_id: PoolId = uuid::Uuid::from_u128(1).into();
        let host_id: HostId = uuid::Uuid::from_u128(2).into();
        let vm_id: crate::VmId = uuid::Uuid::from_u128(3).into();
        let unicode_vm: crate::VmId = uuid::Uuid::from_u128(4).into();
        let mut inventory = Inventory {
            pool_id,
            pool_name: "Cluster Alpha".into(),
            coordinator: Some(host_id),
            hosts: BTreeMap::from([(
                host_id,
                crate::Host {
                    id: host_id,
                    name: "node-01".into(),
                    address: "10.0.0.1".into(),
                    enabled: true,
                    version: "8.3.0".into(),
                },
            )]),
            vms: BTreeMap::from([
                (
                    vm_id,
                    crate::Vm {
                        id: vm_id,
                        name: "App-Server".into(),
                        description: "Primary".into(),
                        power_state: PowerState::Running,
                        host: Some(host_id),
                        vcpus: 4,
                        memory_bytes: 4 << 30,
                        is_template: false,
                        is_control_domain: false,
                        allowed_actions: Vec::new(),
                        can_snapshot: false,
                    },
                ),
                (
                    unicode_vm,
                    crate::Vm {
                        id: unicode_vm,
                        name: "Ümlaut-Guest".into(),
                        description: "Café".into(),
                        power_state: PowerState::Halted,
                        host: None,
                        vcpus: 2,
                        memory_bytes: 2 << 30,
                        is_template: false,
                        is_control_domain: false,
                        allowed_actions: Vec::new(),
                        can_snapshot: false,
                    },
                ),
            ]),
            snapshots: Default::default(),
            storage: Vec::new(),
            networks: Vec::new(),
            details: Default::default(),
            revision: 1,
        };
        inventory
            .details
            .organization
            .objects
            .push(crate::OrganizationObject {
                id: Id::Vm(vm_id),
                name: "App-Server".into(),
                tags: Some(vec!["prod".into()]),
                folder: Some("/Prod/Apps".into()),
                other_config_known: true,
                custom_fields: BTreeMap::from([("Team".into(), "Core".into())]),
            });
        let all = InventorySearch::default().run(&inventory).unwrap();
        assert_eq!(all.len(), 4);
        let by_unicode = InventorySearch {
            text: "ümlaut".into(),
            ..Default::default()
        }
        .run(&inventory)
        .unwrap();
        assert_eq!(by_unicode.len(), 1);
        assert_eq!(by_unicode[0].object, Id::Vm(unicode_vm));
        let by_uuid = InventorySearch {
            text: vm_id.to_string().to_uppercase(),
            ..Default::default()
        }
        .run(&inventory)
        .unwrap();
        assert_eq!(by_uuid.len(), 1);
        assert_eq!(by_uuid[0].object, Id::Vm(vm_id));
        let by_meta = InventorySearch {
            host: "10.0.0.1".into(),
            power: Some(PowerState::Running),
            tags: BTreeSet::from(["prod".into()]),
            folder: "/Prod".into(),
            custom_fields: BTreeMap::from([("Team".into(), "Core".into())]),
            ..Default::default()
        }
        .run(&inventory)
        .unwrap();
        assert_eq!(by_meta.len(), 1);
        assert_eq!(by_meta[0].object, Id::Vm(vm_id));
        for row in &all {
            assert_eq!(
                by_meta.iter().any(|m| m.object == row.object),
                InventorySearch {
                    host: "10.0.0.1".into(),
                    power: Some(PowerState::Running),
                    tags: BTreeSet::from(["prod".into()]),
                    folder: "/Prod".into(),
                    custom_fields: BTreeMap::from([("Team".into(), "Core".into())]),
                    ..Default::default()
                }
                .matches(row)
            );
        }
    }
}
