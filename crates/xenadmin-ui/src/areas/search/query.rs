//! The query sheet of the Search tab: the query, its drafts, the preset
//! from the tree selection, the sources and the results.
//!
//! This is the Rune `search_view::Workspace` without its views and without
//! its table state. The logic of `apply`, `rebuild`, `add_source`, `save`,
//! `persist`, `load`, `navigation` and `navigate` is carried over; what is
//! new is the preset (the Rune workspace always searched every pool) and
//! the display rows with per-kind columns and a column sort (the Rune
//! inventory table of `main.rs`).

use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use std::sync::Arc;

use xenadmin_core::{
    Inventory, InventorySearch, OrganizationObjectId as Id, PoolId, ProfileId, SavedSearch,
    SearchCatalog, SearchKind, SearchResult, SnapshotId,
};

use crate::prelude::*;
use crate::rows::{DisplayRow, InventoryPage};
use crate::session;

/// One pool the search covers.
#[derive(Clone, Debug)]
pub struct Source {
    pub name: String,
    /// The saved connection through which the pool is searched. `None` for
    /// a synthetic inventory, which can never be navigated to.
    pub profile: Option<ProfileId>,
    pub current: bool,
    /// `inventory.details.organization.complete`.
    pub complete: bool,
    /// The inventory that was searched. Holding it keeps its address, which
    /// the change detector compares, from being reused.
    pub inventory: Arc<Inventory>,
}

/// The change detector: one entry per saved connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signature {
    pub profile: ProfileId,
    pub generation: Option<u64>,
    pub revision: Option<u64>,
    pub current: bool,
    /// The address of the inventory, so that an inventory replaced without
    /// a new revision (a scene, a test) is seen too.
    inventory: usize,
}

/// Where the sheet was preset for: the tree scope, and whether the Objects
/// navigation view is showing (it presets the object type).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    pub scope: Target,
    pub objects: bool,
}

impl Place {
    pub fn of(m: &Model) -> Self {
        Self {
            scope: m.scope().clone(),
            objects: m.nav.view == NavView::Objects,
        }
    }
}

/// What the preset of the current place wrote into the query, so that the
/// next preset can take it back.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Preset {
    /// The Objects view: the type of the selected object.
    pub kind: Option<SearchKind>,
    pub folder: Option<String>,
    pub tag: Option<String>,
    /// A saved search was loaded: the whole query is its definition.
    pub saved: bool,
    /// A saved search that is selected but no longer stored.
    pub missing: Option<String>,
    /// A multi-selection: the result keys of the selected objects. The
    /// query has no such filter, so it is applied to the results.
    pub only: Option<BTreeSet<String>>,
}

/// The columns of the result grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// Results of several types: name, type, pool, description.
    Mixed,
    /// The columns of the Rune inventory pages, when the query asks for
    /// one type.
    Guests,
    Hosts,
    Storage,
    Networks,
}

impl Layout {
    pub fn of(query: &InventorySearch) -> Self {
        let mut kinds = query.kinds.iter();
        match (kinds.next(), kinds.next()) {
            (Some(SearchKind::Vm | SearchKind::Template), None) => Self::Guests,
            (Some(SearchKind::Host), None) => Self::Hosts,
            (Some(SearchKind::Storage), None) => Self::Storage,
            (Some(SearchKind::Network), None) => Self::Networks,
            _ => Self::Mixed,
        }
    }

    /// Column id, title, sortable, right aligned. The ids `name`, `state`,
    /// `location`, `cpu` and `memory` are those of the Rune inventory
    /// table, so stored column widths keep applying.
    pub fn columns(self) -> &'static [(&'static str, &'static str, bool, bool)] {
        const POOL: (&str, &str, bool, bool) = ("pool", "Pool", true, false);
        const FRESH: (&str, &str, bool, bool) = ("freshness", "Inventory", true, false);
        match self {
            Self::Mixed => &[
                ("name", "Name", true, false),
                ("kind", "Type", true, false),
                POOL,
                FRESH,
                ("description", "Description", false, false),
            ],
            Self::Guests => &[
                ("name", "Name", true, false),
                ("state", "State", true, false),
                ("location", "Host", true, false),
                ("cpu", "vCPUs", true, true),
                ("memory", "Memory limit", false, true),
                POOL,
                FRESH,
            ],
            Self::Hosts => &[
                ("name", "Name", true, false),
                ("state", "State", true, false),
                ("location", "Address", true, false),
                POOL,
                FRESH,
            ],
            Self::Storage => &[
                ("name", "Name", true, false),
                ("state", "Type", true, false),
                ("location", "Used", true, false),
                ("memory", "Capacity", false, true),
                POOL,
                FRESH,
            ],
            Self::Networks => &[
                ("name", "Name", true, false),
                ("location", "Bridge", true, false),
                POOL,
                FRESH,
            ],
        }
    }

    fn page(self) -> Option<InventoryPage> {
        match self {
            Self::Mixed | Self::Guests => None,
            Self::Hosts => Some(InventoryPage::Hosts),
            Self::Storage => Some(InventoryPage::Storage),
            Self::Networks => Some(InventoryPage::Networks),
        }
    }
}

/// The query, its drafts and its results. `m.areas.search.sheet`.
#[derive(Clone, Default)]
pub struct Sheet {
    /// The applied query.
    pub query: InventorySearch,
    // Drafts. `tags` and `field`/`value` are "add one" inputs: applying
    // moves them into the query and empties them.
    pub text: String,
    pub host: String,
    pub tags: String,
    pub folder: String,
    pub field: String,
    pub value: String,
    pub name: String,
    /// The results of all pools, in the order of `query.sort`.
    pub rows: Rc<Vec<SearchResult>>,
    /// The grid rows: `rows` with the cells of the layout, in the order
    /// shown.
    pub shown: Rc<Vec<Row>>,
    pub sources: BTreeMap<PoolId, Source>,
    pub signature: Vec<Signature>,
    /// Connections that have no inventory.
    pub unavailable: usize,
    pub notice: Option<String>,
    pub error: Option<String>,
    /// The column the grid is sorted by and whether descending. `None`:
    /// the order of the grouping. It survives a pool switch.
    pub sort: Option<(String, bool)>,
    pub preset: Preset,
    /// What the sheet was preset for. `None` before the first use.
    pub place: Option<Place>,
}

/// The kind a search result has as an object of the model.
pub fn object_kind(kind: SearchKind) -> ObjectKind {
    match kind {
        SearchKind::Pool => ObjectKind::Pool,
        SearchKind::Host => ObjectKind::Host,
        SearchKind::Vm => ObjectKind::Vm,
        SearchKind::Template => ObjectKind::Template,
        SearchKind::Snapshot => ObjectKind::Snapshot,
        SearchKind::Storage => ObjectKind::StorageRepository,
        SearchKind::Network => ObjectKind::Network,
        SearchKind::Disk => ObjectKind::VirtualDisk,
    }
}

/// The search type of an object kind. `None` for kinds the search does not
/// cover.
pub fn search_kind(kind: ObjectKind) -> Option<SearchKind> {
    SearchKind::ALL
        .into_iter()
        .find(|search| object_kind(*search) == kind)
}

/// A result as an object: what the grid selection and the commands hold.
pub fn object_of(result: &SearchResult) -> ObjectRef {
    ObjectRef::new(
        Some(result.pool),
        object_kind(result.kind),
        result.object.uuid(),
    )
}

/// The result key of an object: `SearchResult::key` without a result. The
/// pool is part of it, so one UUID in two pools is two rows.
pub fn key_of(object: &ObjectRef) -> Option<String> {
    Some(format!(
        "{}:{}:{}",
        object.pool?,
        search_kind(object.kind)?.label(),
        object.id
    ))
}

pub fn signature(m: &Model) -> Vec<Signature> {
    signature_of(&session::sessions(m))
}

fn signature_of(sessions: &[session::SessionView]) -> Vec<Signature> {
    sessions
        .iter()
        .map(|view| Signature {
            profile: view.profile,
            generation: view.generation,
            revision: view.inventory.as_ref().map(|inventory| inventory.revision),
            current: view.current,
            inventory: view
                .inventory
                .as_ref()
                .map_or(0, |inventory| Arc::as_ptr(inventory) as usize),
        })
        .collect()
}

impl Sheet {
    /// Whether the sheet is preset for this place and built from these
    /// inventories.
    pub fn fresh(&self, place: &Place, signature: &[Signature]) -> bool {
        self.place.as_ref() == Some(place) && self.signature == signature
    }

    pub fn layout(&self) -> Layout {
        Layout::of(&self.query)
    }

    pub fn result(&self, key: &str) -> Option<&SearchResult> {
        self.rows.iter().find(|result| result.key() == key)
    }

    pub fn stale_sources(&self) -> usize {
        self.sources.values().filter(|s| !s.current).count()
    }

    pub fn incomplete_sources(&self) -> usize {
        self.sources.values().filter(|s| !s.complete).count()
    }

    /// The Rune summary line under the query.
    pub fn summary(&self) -> String {
        format!(
            "{} results · {} pools · {} stale · {} with incomplete metadata · {} connections \
             without inventory",
            self.rows.len(),
            self.sources.len(),
            self.stale_sources(),
            self.incomplete_sources(),
            self.unavailable
        )
    }

    /// Moves the drafts into the query (the Rune `apply`, without the
    /// rebuild).
    pub fn apply_drafts(&mut self) {
        self.query.text = self.text.trim().into();
        self.query.host = self.host.trim().into();
        if !self.tags.trim().is_empty() {
            self.query.tags.insert(self.tags.trim().into());
            self.tags.clear();
        }
        self.query.folder = self.folder.trim().into();
        if !self.field.trim().is_empty() {
            self.query
                .custom_fields
                .insert(self.field.trim().into(), self.value.clone());
            self.field.clear();
            self.value.clear();
        }
    }

    /// Fills the sheet from a saved definition (the Rune `load`): the text,
    /// host, folder and name drafts; tags and custom fields stay in the
    /// query and their "add one" drafts are emptied.
    pub fn load(&mut self, saved: &SavedSearch) {
        self.text = saved.query.text.clone();
        self.host = saved.query.host.clone();
        self.tags.clear();
        self.folder = saved.query.folder.clone();
        self.field.clear();
        self.value.clear();
        self.name = saved.name.clone();
        self.query = saved.query.clone();
    }

    /// Empties the query and every draft. The sort order stays.
    pub fn clear_filters(&mut self) {
        self.query = InventorySearch::default();
        for draft in [
            &mut self.text,
            &mut self.host,
            &mut self.tags,
            &mut self.folder,
            &mut self.field,
            &mut self.value,
            &mut self.name,
        ] {
            draft.clear();
        }
        self.preset = Preset::default();
        self.place = None;
        self.notice = None;
        self.error = None;
    }
}

/// Presets the query from the tree selection. A pool sets the pool scope;
/// a server its pool and the host filter; any other object of a pool its
/// pool; in the Objects view the type of the selected object instead, over
/// every pool; a folder the folder filter; a tag a required tag; a saved
/// search its whole definition; several objects exactly those objects.
/// Nothing selected searches every connected pool. What the previous
/// preset wrote is taken back first; the text and the filters the user set
/// stay, except after a saved search, whose definition goes with it.
pub fn preset(m: &Model, sheet: &mut Sheet, place: Place) {
    let old = std::mem::take(&mut sheet.preset);
    if old.saved || old.missing.is_some() {
        sheet.clear_filters();
    }
    if old.folder.as_deref() == Some(sheet.query.folder.as_str()) {
        sheet.query.folder.clear();
        sheet.folder.clear();
    }
    if let Some(tag) = &old.tag {
        sheet.query.tags.remove(tag);
    }
    if let Some(kind) = old.kind
        && sheet.query.kinds.len() == 1
        && sheet.query.kinds.contains(&kind)
    {
        sheet.query.kinds.clear();
    }
    sheet.query.pools.clear();
    sheet.query.host.clear();
    sheet.host.clear();
    let mut next = Preset::default();
    let scope = &place.scope;
    if let Some(object) = scope.single() {
        match (object.kind, object.pool) {
            (ObjectKind::Folder, _) => {
                sheet.query.folder = object.id.clone();
                sheet.folder = object.id.clone();
                next.folder = Some(object.id.clone());
            }
            (ObjectKind::Tag, _) => {
                sheet.query.tags.insert(object.id.clone());
                next.tag = Some(object.id.clone());
            }
            (ObjectKind::SavedSearch, _) => {
                let saved = m
                    .preferences
                    .saved_searches
                    .searches
                    .iter()
                    .find(|saved| saved.name == object.id);
                match saved {
                    Some(saved) => {
                        sheet.load(saved);
                        next.saved = true;
                    }
                    None => next.missing = Some(object.id.clone()),
                }
            }
            (kind, Some(pool)) => match search_kind(kind).filter(|_| place.objects) {
                Some(kind) => {
                    sheet.query.kinds = [kind].into();
                    next.kind = Some(kind);
                }
                None => {
                    sheet.query.pools.insert(pool);
                    if kind == ObjectKind::Host {
                        sheet.query.host = object.id.clone();
                        sheet.host = object.id.clone();
                    }
                }
            },
            _ => {}
        }
    } else if scope.is_multi() {
        sheet
            .query
            .pools
            .extend(scope.objects.iter().filter_map(|object| object.pool));
        next.only = Some(scope.objects.iter().filter_map(key_of).collect());
    }
    sheet.preset = next;
    sheet.place = Some(place);
}

/// One line that says what the preset restricts the search to.
pub fn scope_line(m: &Model, sheet: &Sheet) -> String {
    let Some(place) = &sheet.place else {
        return "Every connected pool".into();
    };
    let scope = &place.scope;
    if scope.is_multi() {
        return format!("The {} selected objects", scope.len());
    }
    let Some(object) = scope.single() else {
        return "Every connected pool".into();
    };
    let name = m.name_of(object).unwrap_or_else(|| object.id.clone());
    match object.kind {
        ObjectKind::Folder => format!("Folder {name} and its descendants"),
        ObjectKind::Tag => format!("Objects with the tag {name}"),
        ObjectKind::SavedSearch => format!("Saved search {name}"),
        ObjectKind::Connection => "Every connected pool".into(),
        _ => match sheet.preset.kind {
            Some(kind) => format!("Every {} of every connected pool", kind.label()),
            None => {
                let pool = object
                    .pool
                    .and_then(|pool| m.inventory_for(pool))
                    .map(|inventory| inventory.pool_name.clone())
                    .unwrap_or_else(|| "its pool".into());
                match object.kind {
                    ObjectKind::Pool => format!("Pool {pool}"),
                    ObjectKind::Host => format!("Server {name} in pool {pool}"),
                    _ => format!("Pool {pool}, the pool of {name}"),
                }
            }
        },
    }
}

pub type PoolSources = BTreeMap<PoolId, (Option<ProfileId>, bool, Arc<Inventory>)>;

/// Two saved connections that reach one pool are one source: a current one
/// beats a stale one, then the selected session wins. Inventory revisions
/// are deliberately not compared.
pub fn add_source(
    pools: &mut PoolSources,
    id: ProfileId,
    current: bool,
    inventory: Arc<Inventory>,
    selected: Option<ProfileId>,
) {
    let priority = |profile: Option<ProfileId>, current| (current, profile == selected);
    if pools
        .get(&inventory.pool_id)
        .is_none_or(|(old_id, old_current, _)| {
            priority(Some(id), current) > priority(*old_id, *old_current)
        })
    {
        pools.insert(inventory.pool_id, (Some(id), current, inventory));
    }
}

/// Runs the query over every pool that has an inventory (the Rune
/// `rebuild`). An invalid query empties the results and shows the error; it
/// never falls back to an unfiltered list.
pub fn rebuild(m: &Model, sheet: &mut Sheet) {
    let sessions = session::sessions(m);
    sheet.rows = Rc::default();
    sheet.shown = Rc::default();
    sheet.sources.clear();
    sheet.signature = signature_of(&sessions);
    sheet.unavailable = 0;
    if let Some(name) = &sheet.preset.missing {
        sheet.error = Some(format!(
            "The saved search \"{name}\" is no longer stored on this computer."
        ));
        return;
    }
    if let Err(error) = sheet.query.validate() {
        sheet.error = Some(error);
        return;
    }
    sheet.error = None;
    // The selected session is the scope session, as in the tree
    // (`session::session_for_pool`); Rune compared with the profile selected
    // last, which the scope session is.
    let selected = m.workspace.selected;
    let mut pools = PoolSources::new();
    let mut synthetic = Vec::new();
    for view in &sessions {
        match &view.inventory {
            Some(inventory) if view.synthetic => synthetic.push(inventory.clone()),
            Some(inventory) => add_source(
                &mut pools,
                view.profile,
                view.current,
                inventory.clone(),
                selected,
            ),
            // A saved connection that was never connected is not a
            // connection without an inventory.
            None if view.state.is_some() => sheet.unavailable += 1,
            None => {}
        }
    }
    // Synthetic sources are always stale and have no profile.
    for inventory in synthetic {
        pools.insert(inventory.pool_id, (None, false, inventory));
    }
    let mut rows = Vec::new();
    for (pool, (profile, current, inventory)) in pools {
        match sheet.query.run(&inventory) {
            Ok(found) => rows.extend(found),
            Err(error) => sheet.error = Some(error),
        }
        sheet.sources.insert(
            pool,
            Source {
                name: inventory.pool_name.clone(),
                profile,
                current,
                complete: inventory.details.organization.complete,
                inventory,
            },
        );
    }
    if let Some(only) = &sheet.preset.only {
        rows.retain(|result| only.contains(&result.key()));
    }
    sheet.query.sort(&mut rows);
    sheet.rows = Rc::new(rows);
    project(sheet);
}

/// Builds the grid rows from the results: the cells of the layout, in the
/// order of the grouping or of the column the user sorted by.
pub fn project(sheet: &mut Sheet) {
    let layout = sheet.layout();
    let columns = layout.columns();
    // The rows of the Rune inventory page of each pool, by object id.
    let mut pages: BTreeMap<PoolId, BTreeMap<String, DisplayRow>> = BTreeMap::new();
    if let Some(page) = layout.page() {
        for (pool, source) in &sheet.sources {
            pages.insert(
                *pool,
                crate::rows::project_rows(&source.inventory, page, None, "")
                    .into_iter()
                    .map(|row| (row.id.clone(), row))
                    .collect(),
            );
        }
    }
    let mut shown: Vec<Row> = sheet
        .rows
        .iter()
        .map(|result| {
            let source = sheet.sources.get(&result.pool);
            let detail = match layout {
                Layout::Mixed => None,
                Layout::Guests => source.and_then(|source| {
                    let Id::Vm(id) = result.object else {
                        return None;
                    };
                    let vm = source.inventory.vms.get(&id)?;
                    Some(crate::rows::vm_row(&source.inventory, vm))
                }),
                _ => pages
                    .get(&result.pool)
                    .and_then(|page| page.get(&result.object.uuid().to_string()))
                    .cloned(),
            };
            let cells = columns.iter().map(|(id, ..)| {
                let cell = match *id {
                    "name" => Some(result.name.clone()),
                    "kind" => Some(result.kind.label().to_owned()),
                    "pool" => Some(result.pool_name.clone()),
                    "freshness" => Some(
                        if source.is_some_and(|source| source.current) {
                            "Current"
                        } else {
                            "Stale"
                        }
                        .to_owned(),
                    ),
                    "description" => Some(result.description.clone()),
                    "state" => detail.as_ref().map(|row| row.state.clone()),
                    "location" => detail.as_ref().map(|row| row.location.clone()),
                    "cpu" => detail.as_ref().map(|row| row.cpu.clone()),
                    "memory" => detail.as_ref().map(|row| row.memory.clone()),
                    _ => None,
                };
                // A cell the inventory cannot fill says so.
                cell.unwrap_or_else(|| "Unknown".to_owned())
            });
            row(result.key(), cells)
        })
        .collect();
    let sorted = sheet.sort.as_ref().and_then(|(column, descending)| {
        columns
            .iter()
            .position(|(id, _, sortable, _)| id == column && *sortable)
            .map(|index| (index, *descending))
    });
    if let Some((index, descending)) = sorted {
        // Stable: rows that compare equal keep the order of the grouping.
        shown.sort_by(|left, right| {
            let order = natural_cmp(&left.cells[index], &right.cells[index]);
            if descending { order.reverse() } else { order }
        });
    }
    sheet.shown = Rc::new(shown);
}

/// Where opening a result goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Destination {
    /// The object that becomes the tree selection.
    pub object: ObjectRef,
    /// For a snapshot result: the snapshot, shown on the Snapshots tab of
    /// its VM.
    pub snapshot: Option<SnapshotId>,
}

pub const UNAVAILABLE: &str =
    "This result or its parent is no longer available. Refresh the search.";
pub const RECONNECT: &str = "Reconnect this pool before opening the result";
pub const SYNTHETIC: &str = "A synthetic inventory has no host connection to open the result in.";

/// Where a result is in an inventory (the Rune `navigation`). It refuses
/// rather than opening a neighbouring object: another pool, a removed
/// object, a VM that became a template or the reverse, a snapshot whose VM
/// is unset, unknown or not a guest, a disk whose repository is gone.
pub fn navigation(inventory: &Inventory, result: &SearchResult) -> Result<Destination, String> {
    let unavailable = || UNAVAILABLE.to_owned();
    if inventory.pool_id != result.pool {
        return Err(unavailable());
    }
    let pool = inventory.pool_id;
    let (object, snapshot) = match result.object {
        Id::Pool(id) if id == pool => (ObjectRef::pool(pool), None),
        Id::Host(id) if inventory.hosts.contains_key(&id) => (ObjectRef::host(pool, id), None),
        Id::Vm(id) => {
            let vm = inventory
                .vms
                .get(&id)
                .filter(|v| {
                    !v.is_control_domain && v.is_template == (result.kind == SearchKind::Template)
                })
                .ok_or_else(unavailable)?;
            (
                if vm.is_template {
                    ObjectRef::template(pool, id)
                } else {
                    ObjectRef::vm(pool, id)
                },
                None,
            )
        }
        Id::Snapshot(id) => {
            let vm = inventory
                .snapshots
                .get(&id)
                .and_then(|v| v.vm)
                .and_then(|id| inventory.vms.get(&id))
                .filter(|v| !v.is_control_domain && !v.is_template)
                .ok_or_else(unavailable)?;
            (ObjectRef::vm(pool, vm.id), Some(id))
        }
        Id::Disk(id) => {
            // Rune opened the repository of the disk, because a disk could
            // not be selected. A disk is an object of the tree now; its
            // repository must still be there.
            inventory
                .details
                .vdis
                .get(&id)
                .and_then(|d| d.sr)
                .filter(|id| inventory.storage.iter().any(|sr| sr.uuid == id.uuid()))
                .ok_or_else(unavailable)?;
            (ObjectRef::disk(pool, id.uuid()), None)
        }
        Id::Storage(id) if inventory.storage.iter().any(|sr| sr.uuid == id.uuid()) => {
            (ObjectRef::storage(pool, id.uuid()), None)
        }
        Id::Network(id) if inventory.networks.iter().any(|n| n.uuid == id.uuid()) => {
            (ObjectRef::network(pool, id.uuid()), None)
        }
        _ => return Err(unavailable()),
    };
    Ok(Destination { object, snapshot })
}

/// Why a result cannot be opened now, from its source alone: a synthetic
/// source has no saved connection, and a stale one must reconnect first.
pub fn open_refusal(sheet: &Sheet, result: &SearchResult) -> Option<&'static str> {
    match sheet.sources.get(&result.pool) {
        None => Some(UNAVAILABLE),
        Some(source) if source.profile.is_none() => Some(SYNTHETIC),
        Some(source) if !source.current => Some(RECONNECT),
        Some(_) => None,
    }
}

/// Stores a catalog (the Rune `persist`): it must encode, it replaces the
/// one in the preferences, and when this process owns the state directory
/// it is written; a failed write puts the previous catalog back.
pub fn persist(m: &mut Model, next: SearchCatalog) -> Result<(), String> {
    next.encode()?;
    let previous = std::mem::replace(&mut m.preferences.saved_searches, next);
    if m.owns_state()
        && let Err(error) = m.preferences.save()
    {
        m.preferences.saved_searches = previous;
        return Err(error);
    }
    Ok(())
}

/// The two captions that describe one definition in the import review and
/// in the save dialog.
pub fn definition_lines(definition: &SavedSearch) -> [String; 2] {
    let q = &definition.query;
    [
        format!(
            "{} · {} · {} object type filters",
            definition.name,
            q.text,
            q.kinds.len()
        ),
        format!(
            "Types: {}\nPools: {}\nPower: {} · Host: {}\nTags: {} · Folder: {}\nCustom fields: {}",
            if q.kinds.is_empty() {
                "All".into()
            } else {
                q.kinds
                    .iter()
                    .map(|k| k.label())
                    .collect::<Vec<_>>()
                    .join(", ")
            },
            if q.pools.is_empty() {
                "All connected".into()
            } else {
                q.pools
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            },
            q.power
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or("Any".into()),
            q.host,
            q.tags.iter().cloned().collect::<Vec<_>>().join(", "),
            q.folder,
            q.custom_fields
                .iter()
                .map(|(k, v)| format!("{k} = {v}"))
                .collect::<Vec<_>>()
                .join("; ")
        ),
    ]
}
