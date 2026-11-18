//! Area `search`: the Search tab, the Saved Searches navigation view, saved
//! searches with their files, and the inventory report.
//!
//! Rune modules: `search_view.rs` ("Search all pools"), `search_files.rs`,
//! `inventory_report.rs` and the inventory table of `main.rs` with its row
//! context menu.
//!
//! What became of them (map `rune-kernel`, sections 6.1 and 6.3):
//!
//! | Rune | Here |
//! | --- | --- |
//! | "Search all pools" dialog | the Search tab: one query, results of every pool it covers |
//! | inventory table of one type in one pool | the same grid: a query for one type shows the columns of that Rune page; the tree selection presets the pool and the server, the Objects view presets the type |
//! | table cursor and multi-selection | the grid selection (`m.selection.grid`), the target of the commands |
//! | "Open in pool" | command `search.open-result`, and activating a row |
//! | "Saved searches" section: Save | command `search.save` and the dialog `search-save-dialog` |
//! | saved search buttons, "Remove" | the rows of the Saved Searches navigation view; command `search.remove` |
//! | "Import…" and its review | command `file.import-search`, dialog `search-import-dialog` (experimental) |
//! | "Export…" | command `search.export` |
//! | "Export resource results…" | command `tools.inventory-report`, dialog `inventory-report` (experimental) |
//! | closing and reopening the dialog | "Clear filters" (`search-reset`) |
//! | row context menu | the grid's context menu, from the command table |
//!
//! The rules of the Rune module hold: two saved connections that reach one
//! pool are one source; result keys include the pool; a result is opened
//! only in a current pool and its place is validated before and after the
//! pool becomes the scope session; an invalid query empties the results; an
//! import is never applied without its review and a conflict changes
//! nothing; a failed preferences write puts the catalog back; a file
//! prompt answered after the job was abandoned is ignored; the report holds
//! the results as they were when its button was pressed.

use std::borrow::Cow;
use std::sync::Arc;

use xenadmin_core::{SavedSearch, SearchCatalog, SearchGroup, SearchKind};

use crate::prelude::*;

mod files;
mod query;
mod report;
#[cfg(test)]
mod tests;
mod view;

pub use query::{Destination, Layout, Place, Sheet, Source, key_of, object_of};

/// The state of the area. `m.areas.search`.
#[derive(Default)]
pub struct State {
    /// The query, its drafts and its results.
    pub sheet: Sheet,
    /// The Filters section is shown.
    pub advanced: bool,
    /// The identity of the file jobs of this sheet. A reply to a file
    /// prompt that was opened under another token is ignored.
    pub token: uuid::Uuid,
    pub files: files::Pending,
    pub dialog: Option<Dialog>,
}

/// The open dialog of the area.
pub enum Dialog {
    /// Save Search: the name draft is `sheet.name`.
    Save,
    Import(Import),
    Report(ReportPanel),
}

/// An import: the definitions awaiting review, or why the file could not
/// be read.
pub struct Import {
    pub catalog: Option<SearchCatalog>,
    pub error: Option<String>,
}

#[derive(Default)]
pub struct ReportPanel {
    pub notice: Option<String>,
    pub error: Option<String>,
}

pub fn spec() -> AreaSpec {
    AreaSpec {
        commands,
        scenes,
        surfaces: view::surfaces,
        tab: view::tab,
        poll,
        on_scope_change,
        // The Objects view presets the query by the kind of the selection.
        on_nav_change: on_scope_change,
        nav_rows,
        ..AreaSpec::new("search")
    }
}

// -- Hooks --------------------------------------------------------------------

fn poll(m: &mut Model, _clock: f64) -> bool {
    let files = files::poll(m);
    // Building the sheet for the first time changes nothing that is shown:
    // a description is always up to date (`current`).
    let built = m.areas.search.sheet.place.is_some();
    let rebuilt = sync(m);
    if rebuilt {
        prune_grid(m);
    }
    files || (rebuilt && built)
}

/// The tree selection presets the query.
fn on_scope_change(m: &mut Model) {
    sync(m);
}

/// The Saved Searches navigation view: one row per saved search, by name.
fn nav_rows(m: &Model, view: NavView) -> Vec<NavRow> {
    if view != NavView::SavedSearches {
        return Vec::new();
    }
    let mut searches: Vec<&SavedSearch> = m.preferences.saved_searches.searches.iter().collect();
    searches.sort_by(|a, b| natural_cmp(&a.name, &b.name));
    searches
        .into_iter()
        .map(|saved| {
            // The key of the Rune button that loaded the search.
            NavRow::new(
                format!("search-load-{}", saved.id),
                saved.name.clone(),
                ObjectKind::SavedSearch,
            )
            .object(saved_object(&saved.name))
        })
        .collect()
}

/// A saved search as an object of the tree: its name is its id.
pub fn saved_object(name: &str) -> ObjectRef {
    ObjectRef::new(None, ObjectKind::SavedSearch, name)
}

// -- The sheet ----------------------------------------------------------------

/// Brings the sheet in line with the tree selection and the inventories:
/// presets it when the selection moved, runs the query again when a
/// connection or an inventory changed. Returns whether it did.
pub fn sync(m: &mut Model) -> bool {
    let place = Place::of(m);
    let signature = query::signature(m);
    if m.areas.search.sheet.fresh(&place, &signature) {
        return false;
    }
    let mut sheet = std::mem::take(&mut m.areas.search.sheet);
    if sheet.place.as_ref() != Some(&place) {
        query::preset(m, &mut sheet, place);
    }
    query::rebuild(m, &mut sheet);
    m.areas.search.sheet = sheet;
    true
}

/// The sheet as it is to be shown now. A description is built from `&Model`
/// and cannot store anything: when the stored sheet is behind (the
/// selection or an inventory changed and no poll ran yet), a copy is
/// brought up to date for this one description.
pub fn current(m: &Model) -> Cow<'_, Sheet> {
    let place = Place::of(m);
    let signature = query::signature(m);
    let stored = &m.areas.search.sheet;
    if stored.fresh(&place, &signature) {
        return Cow::Borrowed(stored);
    }
    let mut sheet = stored.clone();
    if sheet.place.as_ref() != Some(&place) {
        query::preset(m, &mut sheet, place);
    }
    query::rebuild(m, &mut sheet);
    Cow::Owned(sheet)
}

/// Changes the query and runs it again (the Rune handlers that ended in
/// `rebuild`). As in the Rune table under a filter, a selected row that is
/// still shown stays selected and stays the target; a row the new query
/// no longer shows leaves the selection, so no command can apply to it.
fn change(m: &mut Model, edit: impl FnOnce(&mut Sheet)) {
    sync(m);
    let mut sheet = std::mem::take(&mut m.areas.search.sheet);
    edit(&mut sheet);
    query::rebuild(m, &mut sheet);
    m.areas.search.sheet = sheet;
    prune_grid(m);
}

/// After a new query or an inventory update: rows that are no longer
/// among the results leave the grid selection; the others keep it.
fn prune_grid(m: &mut Model) {
    if m.active_tab() != TabId::Search || m.selection.grid.is_empty() {
        return;
    }
    let keys: std::collections::BTreeSet<&str> = m
        .areas
        .search
        .sheet
        .shown
        .iter()
        .map(|row| row.id.as_str())
        .collect();
    let kept: Vec<ObjectRef> = m
        .selection
        .grid
        .objects
        .iter()
        .filter(|object| key_of(object).is_some_and(|key| keys.contains(key.as_str())))
        .cloned()
        .collect();
    if kept.len() != m.selection.grid.len() {
        m.select_in_grid(Target::many(kept));
    }
}

/// "Search" and "Apply filters": the drafts become the query.
pub(crate) fn apply(m: &mut Model) {
    change(m, Sheet::apply_drafts);
}

/// "Clear filters": a fresh sheet, as closing and opening the Rune dialog
/// gave one. A file job of the old sheet is abandoned. The Filters section
/// and the sort order stay.
pub(crate) fn reset(m: &mut Model) {
    let state = &mut m.areas.search;
    let sort = state.sheet.sort.take();
    *state = State {
        advanced: state.advanced,
        token: uuid::Uuid::new_v4(),
        ..State::default()
    };
    state.sheet.sort = sort;
    sync(m);
    prune_grid(m);
}

/// Abandons a file job: its prompt, when it is answered, is ignored.
fn abandon_files(m: &mut Model) {
    m.areas.search.token = uuid::Uuid::new_v4();
    m.areas.search.files = files::Pending::default();
}

fn close_dialog(m: &mut Model) {
    if matches!(m.areas.search.dialog, Some(Dialog::Report(_))) {
        abandon_files(m);
    }
    m.areas.search.dialog = None;
}

/// The Save button of the Save Search dialog (the Rune `save`): applies
/// the drafts, then stores the query under the name. A name that exists
/// (ignoring ASCII case) keeps its identity and gets the new filters.
pub(crate) fn save(m: &mut Model) {
    apply(m);
    let sheet = &m.areas.search.sheet;
    if sheet.error.is_some() {
        return;
    }
    let saved = SavedSearch {
        id: uuid::Uuid::new_v4(),
        name: sheet.name.trim().into(),
        query: sheet.query.clone(),
    };
    let mut next = m.preferences.saved_searches.clone();
    // A name identifies a local definition; saving it again deliberately updates its filters.
    if let Some(old) = next
        .searches
        .iter_mut()
        .find(|v| v.name.eq_ignore_ascii_case(&saved.name))
    {
        old.query = saved.query;
    } else {
        next.searches.push(saved);
    }
    match query::persist(m, next) {
        Ok(()) => {
            m.areas.search.sheet.notice = Some("Search saved on this computer".into());
            m.areas.search.dialog = None;
        }
        Err(error) => m.areas.search.sheet.error = Some(error),
    }
}

/// Removes a saved search from this computer (the Rune "Remove"). When it
/// is the tree selection, the selection moves to nothing.
fn remove(m: &mut Model, target: &Target) {
    let Some(object) = target.single_of(ObjectKind::SavedSearch).cloned() else {
        return;
    };
    let mut next = m.preferences.saved_searches.clone();
    next.searches.retain(|v| v.name != object.id);
    match query::persist(m, next) {
        Ok(()) => {
            if m.scope().contains(&object) {
                m.select(Target::none());
            }
        }
        Err(error) => m.areas.search.sheet.error = Some(error),
    }
}

/// "Add definitions": merges the reviewed import into the catalog. A
/// conflict changes nothing and stays in the review with its reason.
pub(crate) fn import_apply(m: &mut Model) {
    let Some(Dialog::Import(Import {
        catalog: Some(imported),
        ..
    })) = &m.areas.search.dialog
    else {
        return;
    };
    let merged = m.preferences.saved_searches.merged(imported);
    match merged.and_then(|next| query::persist(m, next)) {
        Ok(()) => m.areas.search.dialog = None,
        Err(error) => {
            if let Some(Dialog::Import(import)) = &mut m.areas.search.dialog {
                import.error = Some(error);
            }
        }
    }
}

/// Opens a result: the object becomes the tree selection (the Rune
/// `navigate`, "Open in pool"). Only in a pool whose connection is current;
/// the place of the result is validated in the inventory the search saw and
/// again in the inventory of the scope session after the selection moved
/// there ([`show`]). A refusal is shown and nothing else changes.
pub(crate) fn navigate(m: &mut Model, key: &str) {
    sync(m);
    let sheet = &m.areas.search.sheet;
    let Some(result) = sheet.result(key).cloned() else {
        return;
    };
    let refusal = query::open_refusal(sheet, &result);
    let profile = sheet.sources.get(&result.pool).and_then(|s| s.profile);
    let (None, Some(profile)) = (refusal, profile) else {
        refuse(m, refusal.unwrap_or(query::SYNTHETIC).into());
        return;
    };
    // The source may be older than the session: ask the session itself.
    let session = crate::session::sessions(m)
        .into_iter()
        .find(|view| view.profile == profile);
    let inventory = session
        .filter(|view| view.current)
        .and_then(|view| view.inventory)
        .filter(|inventory| inventory.pool_id == result.pool);
    let Some(inventory) = inventory else {
        refuse(m, query::RECONNECT.into());
        return;
    };
    match query::navigation(&inventory, &result) {
        Ok(destination) => show(m, &result, destination),
        Err(error) => refuse(m, error),
    }
}

fn refuse(m: &mut Model, error: String) {
    m.areas.search.sheet.error = Some(error);
}

/// Moves the tree selection to a destination that was validated, and
/// validates it again in the inventory of the scope session, which the
/// selection has just made the result's pool. A snapshot shows on the
/// Snapshots tab of its VM; any other object on its first tab. When the
/// second validation fails the selection returns to where it was, with the
/// reason.
pub(crate) fn show(m: &mut Model, result: &xenadmin_core::SearchResult, destination: Destination) {
    let (scope, view) = (m.scope().clone(), m.nav.view);
    // A saved search has no row for the object: show the tree that has.
    if view == NavView::SavedSearches || view == NavView::Organization {
        m.set_nav_view(NavView::Infrastructure);
    }
    m.select(Target::one(destination.object));
    let after = m
        .inventory
        .as_ref()
        .ok_or_else(|| "Pool inventory is unavailable".to_owned())
        .and_then(|inventory| query::navigation(inventory, result));
    match after {
        Ok(destination) => {
            if let Some(snapshot) = destination.snapshot
                && m.tabs().contains(&TabId::Snapshots)
            {
                m.set_active_tab(TabId::Snapshots);
                m.select_in_grid(Target::one(ObjectRef::snapshot(result.pool, snapshot)));
            } else if m.active_tab() == TabId::Search {
                // Opening shows the object, not its own Search tab.
                let first = m.tabs()[0];
                m.set_active_tab(first);
            }
        }
        Err(error) => {
            m.set_nav_view(view);
            m.select(scope);
            m.set_active_tab(TabId::Search);
            sync(m);
            refuse(m, error);
        }
    }
}

// -- Commands -----------------------------------------------------------------

pub const OPEN_RESULT: CommandId = CommandId("search.open-result");
pub const SAVE: CommandId = CommandId("search.save");
pub const EXPORT: CommandId = CommandId("search.export");
pub const REMOVE: CommandId = CommandId("search.remove");
pub const IMPORT: CommandId = CommandId("file.import-search");
pub const REPORT: CommandId = CommandId("tools.inventory-report");

/// The kinds a result can have.
const RESULT_KINDS: &[ObjectKind] = &[
    ObjectKind::Pool,
    ObjectKind::Host,
    ObjectKind::Vm,
    ObjectKind::Template,
    ObjectKind::Snapshot,
    ObjectKind::StorageRepository,
    ObjectKind::Network,
    ObjectKind::VirtualDisk,
];

fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new(OPEN_RESULT.0, "Open in Pool", "search")
            .context(RESULT_KINDS)
            .local(TabId::Search, 10)
            .eligibility(can_open)
            .invoke(|m, target| {
                if let Some(key) = target.single().and_then(key_of) {
                    navigate(m, &key);
                }
            }),
        CommandSpec::new(SAVE.0, "Save Search", "search")
            .local(TabId::Search, 20)
            .invoke(|m, _| {
                sync(m);
                m.areas.search.dialog = Some(Dialog::Save);
            }),
        CommandSpec::new(EXPORT.0, "Export Saved Searches", "search")
            .local(TabId::Search, 30)
            .eligibility(|m, _| {
                if m.areas.search.files.busy {
                    Eligibility::disabled(files::BUSY)
                } else {
                    Eligibility::enabled_if(
                        !m.preferences.saved_searches.searches.is_empty(),
                        "There are no saved searches to export.",
                    )
                }
            })
            .invoke(|m, _| files::export(m)),
        CommandSpec::new(REMOVE.0, "Delete Saved Search", "search")
            .context(&[ObjectKind::SavedSearch])
            .local(TabId::Search, 40)
            .eligibility(
                |m, target| match target.single_of(ObjectKind::SavedSearch) {
                    None => Eligibility::Hidden,
                    Some(object) => Eligibility::enabled_if(
                        m.preferences
                            .saved_searches
                            .searches
                            .iter()
                            .any(|saved| saved.name == object.id),
                        "This saved search is no longer stored on this computer.",
                    ),
                },
            )
            .invoke(remove),
        CommandSpec::new(IMPORT.0, "Import Search", "search")
            .menu(place::FILE_IMPORT_SEARCH)
            .eligibility(|m, _| Eligibility::enabled_if(!m.areas.search.files.busy, files::BUSY))
            .invoke(|m, _| files::import(m)),
        CommandSpec::new(REPORT.0, "Inventory Report", "search")
            .menu(place::TOOLS_INVENTORY_REPORT)
            .invoke(|m, _| {
                sync(m);
                m.areas.search.dialog = Some(Dialog::Report(ReportPanel::default()));
            }),
    ]
}

/// "Open in Pool" belongs to one selected row of the result grid. It is
/// disabled, with the reason, while the pool of the row is synthetic or its
/// connection is not current.
fn can_open(m: &Model, target: &Target) -> Eligibility {
    let Some(object) = target.single() else {
        return Eligibility::Hidden;
    };
    if m.active_tab() != TabId::Search || !m.selection.grid.contains(object) {
        return Eligibility::Hidden;
    }
    let Some(key) = key_of(object) else {
        return Eligibility::Hidden;
    };
    let sheet = current(m);
    match sheet.result(&key) {
        Some(result) => Eligibility::enabled_unless(query::open_refusal(&sheet, result)),
        None => Eligibility::disabled(query::UNAVAILABLE),
    }
}

// -- Scenes -------------------------------------------------------------------

fn scenes() -> Vec<SceneSpec> {
    vec![
        SceneSpec::new("search-pools", "search", scene),
        SceneSpec::new("inventory-context", "search", context_scene),
    ]
}

/// The Rune `search_view::scene`: the demo pool with organization metadata
/// (as the Rune organization scene gave it), a second synthetic pool that
/// is a copy of it, nothing selected, and the VMs of both grouped by pool.
/// The same VM ids in two pools stay distinct rows.
pub(crate) fn scene(m: &mut Model) {
    use xenadmin_core::{
        CustomFieldDefinition, CustomFieldType, OrganizationInventory, OrganizationObject,
        OrganizationObjectId,
    };
    m.load_demo(3, 48);
    let Some(mut inventory) = m.inventory.as_deref().cloned() else {
        return;
    };
    inventory.details.organization = OrganizationInventory {
        complete: true,
        objects: inventory
            .vms
            .values()
            .enumerate()
            .map(|(i, vm)| OrganizationObject {
                id: OrganizationObjectId::Vm(vm.id),
                name: vm.name.clone(),
                tags: Some(vec![if i % 2 == 0 { "service" } else { "lab" }.into()]),
                folder: Some("/Services".into()),
                other_config_known: true,
                custom_fields: std::collections::BTreeMap::new(),
            })
            .collect(),
        empty_folders: Some(vec!["/Archives".into()]),
        definitions: Some(vec![CustomFieldDefinition {
            name: "Owner".into(),
            kind: CustomFieldType::String,
        }]),
    };
    let mut second = inventory.clone();
    second.pool_id = uuid::Uuid::from_u128(99).into();
    second.pool_name = "Recovery lab".into();
    m.inventory = Some(Arc::new(inventory));
    m.add_synthetic_pool("Recovery lab", second);
    m.select(Target::none());
    m.set_active_tab(TabId::Search);
    change(m, |sheet| {
        sheet.query.kinds.insert(SearchKind::Vm);
        sheet.query.group = SearchGroup::Pool;
    });
}

/// The Rune `inventory_menu::scene`: the demo, the grid of its virtual
/// machines, and the context menu of the first row.
fn context_scene(m: &mut Model) {
    m.load_demo(3, 48);
    m.set_active_tab(TabId::Search);
    change(m, |sheet| {
        sheet.query.kinds.insert(SearchKind::Vm);
    });
    let sheet = &m.areas.search.sheet;
    let first = sheet
        .shown
        .first()
        .and_then(|row| sheet.result(&row.id))
        .map(object_of);
    if let Some(object) = first {
        m.open_context_menu(object, crate::selection::MenuOrigin::Grid);
    }
}
