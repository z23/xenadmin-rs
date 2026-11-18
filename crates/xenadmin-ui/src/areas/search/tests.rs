//! Tests of the search area.
//!
//! The Rune tests of the modules and what became of each:
//!
//! - `search_tests.rs`
//!   - `search_entry_point_is_reachable_at_both_window_sizes`: retired. The
//!     entry point is the Search tab, which every selection has;
//!     `every_selection_has_a_search_tab_with_unique_keys` covers that.
//!   - `aliases_deduplicate_by_pool_and_prefer_current_selected_session_without_comparing_revisions`:
//!     moved, same name.
//!   - `navigation_refuses_removed_and_orphan_snapshots_instead_of_opening_another_vm`:
//!     moved, same name; the place is an object now, not a page and a row.
//!   - `cross_pool_results_keep_duplicate_vm_ids_distinct_and_synthetic_navigation_disabled`:
//!     ported to the harness, same name.
//!   - `saving_and_loading_does_not_erase_imported_complex_filters`: ported
//!     to the harness, same name; loading is selecting the saved search in
//!     the Saved Searches view.
//!   - `imported_queries_require_review_and_conflicts_leave_existing_definitions_intact`:
//!     ported to the harness, same name; the error is the review dialog's.
//!   - `native_search_picker_cancellation_and_closed_workspace_callbacks_are_ignored`:
//!     ported to the harness, same name; "closed and reopened" is "Clear
//!     filters".
//! - `search_files.rs`
//!   `local_search_reader_is_bounded_and_refuses_symlinks_and_special_files`:
//!   moved, in `files.rs`.
//! - `inventory_report.rs`
//!   - `reports_keep_pool_scope_and_explicit_stale_incomplete_coverage` and
//!     `report_limits_refuse_silent_truncation_or_invalid_filters`: moved,
//!     same names.
//!   - `native_report_export_captures_results_and_ignores_cancelled_or_stale_dialogs`:
//!     ported to the harness, same name.
//!
//! The window sizes of the Rune runtime tests are gone with the layout.

use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use std::sync::Arc;

use xenadmin_core::{
    InventorySearch, OrganizationObjectId as Id, PowerState, ProfileId, SavedSearch, SearchCatalog,
    SearchKind, SearchResult,
};

use crate::testing::TempDir;
use super::query::{self, add_source, navigation};
use super::*;
use crate::{Harness, MenuOrigin};

fn sheet(h: &Harness) -> &Sheet {
    &h.model.areas.search.sheet
}

/// The texts of the column headers of a table.
fn headers(h: &Harness, table: &str) -> Vec<String> {
    let after = format!("{table}/columns/");
    h.nodes()
        .into_iter()
        .filter(|node| node.role == "columnheader" && node.path.contains(&after))
        .filter_map(|node| node.text)
        .collect()
}

/// The ids of the rows of a table, in the order shown.
fn row_ids(h: &Harness, table: &str) -> Vec<String> {
    let prefix = format!("{table}:");
    h.nodes()
        .into_iter()
        .filter(|node| node.role == "row")
        .filter_map(|node| node.key)
        .filter_map(|key| key.strip_prefix(&prefix).map(str::to_owned))
        .collect()
}

fn wait_for_files(h: &mut Harness) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while h.model.areas.search.files.busy {
        assert!(
            std::time::Instant::now() < deadline,
            "the file job did not end"
        );
        h.tick(0.01);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

fn file_prompt(h: &Harness) -> FileMode {
    h.model
        .intents
        .iter()
        .find_map(|intent| match intent {
            Intent::ChooseFile(request) => Some(request.mode),
            _ => None,
        })
        .expect("a file prompt")
}

// -- Moved from search_tests.rs -----------------------------------------------

#[test]
fn aliases_deduplicate_by_pool_and_prefer_current_selected_session_without_comparing_revisions() {
    let mut older = crate::demonstration(3, 48);
    older.revision = 9999;
    let mut newer = older.clone();
    newer.revision = 1;
    let a = ProfileId::new();
    let b = ProfileId::new();
    let pool = older.pool_id;
    let mut sources = BTreeMap::new();
    add_source(&mut sources, a, true, Arc::new(older.clone()), Some(b));
    add_source(&mut sources, b, true, Arc::new(newer.clone()), Some(b));
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[&pool].0, Some(b));
    assert_eq!(sources[&pool].2.revision, 1);
    sources.clear();
    add_source(&mut sources, b, false, Arc::new(newer), Some(b));
    add_source(&mut sources, a, true, Arc::new(older), Some(b));
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[&pool].0, Some(a));
    assert!(sources[&pool].1);
}

#[test]
fn navigation_refuses_removed_and_orphan_snapshots_instead_of_opening_another_vm() {
    let mut inventory = crate::demonstration(3, 48);
    let pool = inventory.pool_id;
    let vm = inventory.guests().next().unwrap().id;
    let id = uuid::Uuid::from_u128(901).into();
    let mut snapshot = xenadmin_core::Snapshot {
        id,
        vm: Some(vm),
        name: "Navigation snapshot".into(),
        description: String::new(),
        created_at: None,
        power_state: PowerState::Halted,
        allowed_actions: vec![],
    };
    inventory.snapshots = [(id, snapshot.clone())]
        .into_iter()
        .collect::<BTreeMap<_, _>>()
        .into();
    let result = InventorySearch::default()
        .run(&inventory)
        .unwrap()
        .into_iter()
        .find(|r| r.object == Id::Snapshot(id))
        .unwrap();
    let target = navigation(&inventory, &result).unwrap();
    assert_eq!(target.object, ObjectRef::vm(pool, vm));
    assert_eq!(target.snapshot, Some(id));
    snapshot.vm = None;
    inventory.snapshots = [(id, snapshot.clone())]
        .into_iter()
        .collect::<BTreeMap<_, _>>()
        .into();
    assert!(navigation(&inventory, &result).is_err());
    snapshot.vm = Some(uuid::Uuid::from_u128(902).into());
    inventory.snapshots = [(id, snapshot)]
        .into_iter()
        .collect::<BTreeMap<_, _>>()
        .into();
    assert!(navigation(&inventory, &result).is_err());
    inventory.snapshots = Default::default();
    assert!(navigation(&inventory, &result).is_err());
    let vm_result = InventorySearch::default()
        .run(&inventory)
        .unwrap()
        .into_iter()
        .find(|r| r.object == Id::Vm(vm))
        .unwrap();
    assert_eq!(
        navigation(&inventory, &vm_result).unwrap().object,
        ObjectRef::vm(pool, vm)
    );
    // A VM that became a template is not the result any more.
    inventory.vms.get_mut(&vm).unwrap().is_template = true;
    assert!(navigation(&inventory, &vm_result).is_err());
    inventory.vms.remove(&vm);
    assert!(navigation(&inventory, &vm_result).is_err());
    inventory.pool_id = uuid::Uuid::from_u128(903).into();
    assert!(navigation(&inventory, &vm_result).is_err());
}

// -- Ported from search_tests.rs ----------------------------------------------

#[test]
fn cross_pool_results_keep_duplicate_vm_ids_distinct_and_synthetic_navigation_disabled() {
    let mut h = Harness::scene("search-pools");
    assert_eq!(sheet(&h).sources.len(), 2);
    let keys = sheet(&h)
        .rows
        .iter()
        .map(SearchResult::key)
        .collect::<BTreeSet<_>>();
    assert_eq!(keys.len(), sheet(&h).rows.len());
    // The same VM ids in both pools: twice the guests of one.
    let guests = h.model.inventory.as_ref().unwrap().guests().count();
    assert_eq!(sheet(&h).rows.len(), 2 * guests);
    assert_eq!(row_ids(&h, "search-results").len(), 2 * guests);
    let key = sheet(&h).rows[0].key();
    h.select_row("search-results", &key);
    assert!(h.has_text("2 stale"));
    // The selected result is the target of the commands and shows its
    // details.
    let object = object_of(&sheet(&h).rows[0]);
    assert_eq!(h.model.target(), Target::one(object));
    assert!(h.find("search-result").is_some());
    // A synthetic source cannot be opened: the command says why, and
    // activating the row changes nothing but the message.
    assert_eq!(
        h.try_click("search.open-result"),
        Err(query::SYNTHETIC.into())
    );
    h.activate_row("search-results", &key);
    assert!(h.model.scope().is_empty());
    assert_eq!(h.model.active_tab(), TabId::Search);
    assert!(h.model.connection.is_none());
    assert_eq!(sheet(&h).error.as_deref(), Some(query::SYNTHETIC));
    assert_eq!(
        h.node("search-error").text.as_deref(),
        Some(query::SYNTHETIC)
    );
    h.click("search-filters");
    h.click("search-kind-Host");
    assert!(sheet(&h).query.kinds.contains(&SearchKind::Host));
    // Changing the filters drops the selection: no command can apply to a
    // row that may no longer be shown.
    assert!(h.model.selection.grid.is_empty());
    h.assert_unique_keys();
}

#[test]
fn saving_and_loading_does_not_erase_imported_complex_filters() {
    let mut h = Harness::scene("search-pools");
    {
        let w = &mut h.model.areas.search.sheet;
        w.query.tags.insert("tier,one".into());
        w.query.tags.insert("production".into());
        w.query
            .custom_fields
            .insert("Owner".into(), "Operations".into());
        w.query
            .custom_fields
            .insert("Service".into(), "Database".into());
    }
    h.click("search.save");
    assert_eq!(h.surface_id().as_deref(), Some("search-save-dialog"));
    assert!(!h.surface().experimental);
    h.assert_unique_keys();
    h.type_text("search-name", "Saved scope");
    h.click("search-save");
    assert!(h.surface_id().is_none(), "saving closes the dialog");
    assert_eq!(h.model.preferences.saved_searches.searches.len(), 1);
    assert_eq!(
        h.node("search-notice").text.as_deref(),
        Some("Search saved on this computer")
    );
    let saved = h.model.preferences.saved_searches.searches[0].clone();
    assert_eq!(saved.name, "Saved scope");
    assert_eq!(saved.query.tags.len(), 2);
    h.model.areas.search.sheet.query = Default::default();
    // Loading is selecting the saved search in its navigation view.
    h.open_view(NavView::SavedSearches);
    assert_eq!(
        h.nav_row("Saved scope").id,
        format!("search-load-{}", saved.id)
    );
    h.press_nav("Saved scope");
    assert_eq!(h.model.scope(), &Target::one(saved_object("Saved scope")));
    assert_eq!(h.model.tabs(), [TabId::Search]);
    h.click("search-apply");
    assert_eq!(sheet(&h).query, saved.query);
    // Saving under the same name, in another case, updates the definition
    // and keeps its identity.
    h.click("search-filters");
    h.click("search-kind-Host");
    h.click("search.save");
    h.type_text("search-name", "saved SCOPE");
    h.click("search-save");
    let searches = &h.model.preferences.saved_searches.searches;
    assert_eq!(searches.len(), 1);
    assert_eq!(searches[0].id, saved.id);
    assert_eq!(searches[0].name, "Saved scope");
    assert!(searches[0].query.kinds.contains(&SearchKind::Host));
}

#[test]
fn imported_queries_require_review_and_conflicts_leave_existing_definitions_intact() {
    let mut h = Harness::scene("search-pools");
    let imported = SearchCatalog {
        schema_version: 1,
        searches: vec![SavedSearch {
            id: uuid::Uuid::from_u128(61),
            name: "Imported scope".into(),
            query: Default::default(),
        }],
    };
    h.model.areas.search.dialog = Some(Dialog::Import(Import {
        catalog: Some(imported.clone()),
        error: None,
    }));
    assert!(h.model.preferences.saved_searches.searches.is_empty());
    assert_eq!(h.surface().title, "Review imported searches");
    assert!(h.surface().experimental, "search files are experimental");
    assert!(h.has_text("Nothing connects or runs automatically."));
    assert!(h.has_text("Imported scope"));
    h.assert_unique_keys();
    h.click("search-import-apply");
    assert_eq!(h.model.preferences.saved_searches, imported);
    assert!(h.surface_id().is_none());
    let mut conflict = imported.clone();
    conflict.searches[0].query.text = "changed".into();
    h.model.areas.search.dialog = Some(Dialog::Import(Import {
        catalog: Some(conflict),
        error: None,
    }));
    h.click("search-import-apply");
    assert_eq!(h.model.preferences.saved_searches, imported);
    assert!(
        h.node("search-import-error")
            .text
            .unwrap()
            .contains("Conflicting")
    );
    // Discarding the review adds nothing.
    h.click("search-import-discard");
    assert!(h.surface_id().is_none());
    assert_eq!(h.model.preferences.saved_searches, imported);
}

#[test]
fn native_search_picker_cancellation_and_closed_workspace_callbacks_are_ignored() {
    let mut h = Harness::scene("search-pools");
    h.invoke("file.import-search");
    assert_eq!(file_prompt(&h), FileMode::Open);
    assert_eq!(
        h.try_invoke("file.import-search"),
        Err(files::BUSY.into()),
        "one file job at a time"
    );
    h.answer_file(None);
    assert!(!h.model.areas.search.files.busy);
    h.invoke("file.import-search");
    let directory = TempDir::new("search");
    let path = directory.path().join("scope.json");
    let catalog = SearchCatalog {
        schema_version: 1,
        searches: vec![SavedSearch {
            id: uuid::Uuid::from_u128(62),
            name: "From a file".into(),
            query: Default::default(),
        }],
    };
    std::fs::write(&path, catalog.encode().unwrap()).unwrap();
    // The successor of closing and reopening the workspace.
    h.click("search-reset");
    h.answer_file(Some(path.clone()));
    wait_for_files(&mut h);
    assert!(!h.model.areas.search.files.busy);
    assert!(h.model.areas.search.dialog.is_none());
    // A prompt that is answered for the sheet that asked reads the file and
    // opens the review; nothing is added before it.
    h.invoke("file.import-search");
    h.answer_file(Some(path));
    wait_for_files(&mut h);
    assert_eq!(h.surface().title, "Review imported searches");
    assert!(h.has_text("From a file"));
    assert!(h.model.preferences.saved_searches.searches.is_empty());
    h.dismiss();
    // A file that is not a regular file is refused in the same dialog.
    h.invoke("file.import-search");
    h.answer_file(Some(directory.path().to_owned()));
    wait_for_files(&mut h);
    assert_eq!(h.surface().title, "Import Search");
    assert_eq!(
        h.node("search-import-error").text.as_deref(),
        Some("Choose a regular JSON file")
    );
    h.assert_unique_keys();
    h.click("search-import-discard");
    assert!(h.surface_id().is_none());
}

// -- Moved from inventory_report.rs -------------------------------------------

#[test]
fn reports_keep_pool_scope_and_explicit_stale_incomplete_coverage() {
    let mut m = Model::fixture();
    scene(&mut m);
    let mut w = m.areas.search.sheet.clone();
    w.unavailable = 1;
    let first = *w.sources.keys().next().unwrap();
    w.sources.get_mut(&first).unwrap().complete = false;
    let bytes = report::encode(&w).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["schema"], "xenadmin-rs-inventory-report/v1");
    assert_eq!(value["connections_without_inventory"], 1);
    assert_eq!(value["resources"].as_array().unwrap().len(), w.rows.len());
    assert!(
        value["coverage"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["current"] == false
                && p["metadata_complete"] == false
                && p["synthetic"] == true)
    );
    for row in value["resources"].as_array().unwrap() {
        assert!(row.get("pool").is_some());
        assert!(row.get("uuid").is_some());
        assert!(row.get("session").is_none());
    }
    let captured = bytes;
    w.rows = Rc::default();
    assert!(
        !serde_json::from_slice::<serde_json::Value>(&captured).unwrap()["resources"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn report_limits_refuse_silent_truncation_or_invalid_filters() {
    let mut m = Model::fixture();
    scene(&mut m);
    let mut w = m.areas.search.sheet.clone();
    let row = w.rows[0].clone();
    w.rows = Rc::new(vec![row.clone(); 10_001]);
    assert!(report::encode(&w).is_err());
    let mut large = row;
    large.description = "x".repeat(report::MAX_BYTES);
    w.rows = Rc::new(vec![large]);
    assert!(report::encode(&w).is_err());
    w.rows = Rc::default();
    w.query.text = "x".repeat(513);
    assert!(report::encode(&w).is_err());
    // An existing search error refuses too.
    w.query.text.clear();
    assert!(report::encode(&w).is_ok());
    w.error = Some("Folder must be a full path such as /Services".into());
    assert_eq!(
        report::encode(&w).unwrap_err(),
        "Resolve the search error before exporting results"
    );
}

// -- Ported from inventory_report.rs ------------------------------------------

#[test]
fn native_report_export_captures_results_and_ignores_cancelled_or_stale_dialogs() {
    use std::os::unix::fs::PermissionsExt;
    let mut h = Harness::scene("search-pools");
    h.invoke("tools.inventory-report");
    assert_eq!(h.surface_id().as_deref(), Some("inventory-report"));
    assert!(h.surface().experimental);
    assert!(h.has_text("2 stale"));
    assert!(h.has_text("including private names, identifiers and metadata"));
    h.assert_unique_keys();
    h.click("search-export-results");
    assert_eq!(file_prompt(&h), FileMode::Save);
    assert_eq!(
        h.try_click("search-export-results"),
        Err(files::BUSY.into())
    );
    h.answer_file(None);
    assert!(!h.model.areas.search.files.busy);
    // A prompt answered after its dialog was closed writes nothing.
    h.click("search-export-results");
    let d = TempDir::new("search");
    let ignored = d.path().join("ignored.json");
    h.click("report-close");
    h.invoke("tools.inventory-report");
    h.answer_file(Some(ignored.clone()));
    wait_for_files(&mut h);
    assert!(!ignored.exists());
    // The file holds the results as they were when the button was pressed.
    h.click("search-export-results");
    let expected = sheet(&h).rows.len();
    assert!(expected > 0);
    h.model.areas.search.sheet.rows = Rc::default();
    let path = d.path().join("report.json");
    h.answer_file(Some(path.clone()));
    wait_for_files(&mut h);
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(value["resources"].as_array().unwrap().len(), expected);
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        h.node("report-notice").text.as_deref(),
        Some("Captured resource results exported; later inventory changes are not included")
    );
}

// -- New: what the port added -------------------------------------------------

#[test]
fn every_selection_has_a_search_tab_with_unique_keys() {
    let mut h = Harness::with(Model::demo(3, 48));
    let inventory = h.model.inventory.clone().unwrap();
    let pool = inventory.pool_id;
    let host = *inventory.hosts.keys().next().unwrap();
    let vm = inventory.guests().next().unwrap().id;
    let other = inventory.guests().nth(1).unwrap().id;
    let scopes = [
        Target::none(),
        Target::one(ObjectRef::pool(pool)),
        Target::one(ObjectRef::host(pool, host)),
        Target::one(ObjectRef::vm(pool, vm)),
        Target::one(ObjectRef::storage(pool, inventory.storage[0].uuid)),
        Target::one(ObjectRef::network(pool, inventory.networks[0].uuid)),
        Target::many([ObjectRef::vm(pool, vm), ObjectRef::vm(pool, other)]),
        Target::one(ObjectRef::new(None, ObjectKind::Folder, "/Services")),
        Target::one(ObjectRef::new(None, ObjectKind::Tag, "lab")),
        Target::one(saved_object("Gone")),
    ];
    for scope in scopes {
        h.select(scope.clone());
        h.open_tab(TabId::Search);
        assert!(h.find("search-results").is_some(), "{scope:?}");
        assert!(h.find("search-text").is_some(), "{scope:?}");
        h.click("search-filters");
        h.assert_unique_keys();
        h.click("search-filters");
    }
}

#[test]
fn the_tree_selection_presets_the_scope_of_the_query() {
    let mut h = Harness::scene("search-pools");
    let inventory = h.model.inventory.clone().unwrap();
    let pool = inventory.pool_id;
    let guests = inventory.guests().count();
    // Nothing selected: every pool.
    assert!(sheet(&h).query.pools.is_empty());
    assert_eq!(sheet(&h).rows.len(), 2 * guests);
    assert_eq!(
        h.node("search-scope").text.as_deref(),
        Some("Scope: Every connected pool")
    );
    // A pool: that pool. The filters the user chose stay.
    h.select(ObjectRef::pool(pool));
    h.open_tab(TabId::Search);
    assert_eq!(sheet(&h).query.pools, [pool].into());
    assert_eq!(sheet(&h).query.kinds, [SearchKind::Vm].into());
    assert_eq!(sheet(&h).rows.len(), guests);
    assert!(sheet(&h).rows.iter().all(|r| r.pool == pool));
    // A server: its pool and the guests on it.
    let host = *inventory.hosts.keys().next().unwrap();
    let resident = inventory.guests().filter(|v| v.host == Some(host)).count();
    h.select(ObjectRef::host(pool, host));
    assert_eq!(sheet(&h).query.host, host.to_string());
    assert_eq!(sheet(&h).host, host.to_string());
    assert_eq!(sheet(&h).rows.len(), resident);
    assert!(resident > 0 && resident < guests);
    // Several objects: exactly those.
    let two: Vec<ObjectRef> = inventory
        .guests()
        .take(2)
        .map(|vm| ObjectRef::vm(pool, vm.id))
        .collect();
    h.select(Target::many(two.clone()));
    assert_eq!(h.model.tabs(), [TabId::Search]);
    assert!(sheet(&h).query.host.is_empty());
    let found: BTreeSet<ObjectRef> = sheet(&h).rows.iter().map(object_of).collect();
    assert_eq!(found, two.into_iter().collect());
    // Back to nothing: every pool again, and the host filter is gone.
    h.select(Target::none());
    assert!(sheet(&h).query.pools.is_empty());
    assert_eq!(sheet(&h).rows.len(), 2 * guests);
    // A folder and a tag set their filter and take it back.
    h.select(ObjectRef::new(None, ObjectKind::Folder, "/Services"));
    assert_eq!(sheet(&h).query.folder, "/Services");
    assert_eq!(sheet(&h).rows.len(), 2 * guests);
    h.select(ObjectRef::new(None, ObjectKind::Tag, "lab"));
    assert!(sheet(&h).query.folder.is_empty());
    assert_eq!(sheet(&h).query.tags, ["lab".to_owned()].into());
    assert!(sheet(&h).rows.len() < 2 * guests && !sheet(&h).rows.is_empty());
    h.select(Target::none());
    assert!(sheet(&h).query.tags.is_empty());
    // A saved search that is not stored shows nothing and says so.
    h.select(saved_object("Gone"));
    assert!(sheet(&h).rows.is_empty());
    assert!(
        h.node("search-error")
            .text
            .unwrap()
            .contains("no longer stored")
    );
    h.select(Target::none());
    assert!(sheet(&h).error.is_none());
}

#[test]
fn the_objects_view_presets_the_type_and_a_description_is_never_stale() {
    let mut h = Harness::with(Model::demo(3, 48));
    let inventory = h.model.inventory.clone().unwrap();
    let pool = inventory.pool_id;
    let host = *inventory.hosts.keys().next().unwrap();
    h.select(ObjectRef::host(pool, host));
    h.open_tab(TabId::Search);
    assert_eq!(sheet(&h).query.host, host.to_string());
    assert_eq!(headers(&h, "search-results")[1], "Type");
    // Switching the navigation view tells the area (`on_nav_change`): the
    // stored sheet is brought up to date at once.
    h.open_view(NavView::Objects);
    assert_eq!(sheet(&h).query.kinds, [SearchKind::Host].into());
    assert!(sheet(&h).query.host.is_empty());
    assert_eq!(
        headers(&h, "search-results"),
        ["Name", "State", "Address", "Pool", "Inventory"]
    );
    assert_eq!(row_ids(&h, "search-results").len(), inventory.hosts.len());
    assert_eq!(
        h.node("search-scope").text.as_deref(),
        Some("Scope: Every Host of every connected pool")
    );
    assert!(!h.tick(0.1), "nothing changed since");
    // When no hook ran (the view is set behind the area's back), the
    // stored sheet is behind and the description is right all the same.
    h.model.nav.view = NavView::Infrastructure;
    assert_eq!(sheet(&h).query.kinds, [SearchKind::Host].into());
    assert_eq!(headers(&h, "search-results")[1], "Type");
    // A poll brings the stored sheet up to date: leaving the Objects view
    // takes the type back.
    assert!(h.tick(0.1));
    assert!(sheet(&h).query.kinds.is_empty());
    assert_eq!(sheet(&h).query.host, host.to_string());
    assert!(!h.tick(0.1), "nothing changed since");
}

#[test]
fn a_query_for_one_type_shows_the_columns_of_that_type_and_sorts_by_them() {
    let mut h = Harness::with(Model::demo(3, 48));
    h.open_tab(TabId::Search);
    assert_eq!(
        headers(&h, "search-results"),
        ["Name", "Type", "Pool", "Inventory", "Description"]
    );
    h.click("search-filters");
    h.click("search-kind-VM");
    assert_eq!(
        headers(&h, "search-results"),
        [
            "Name",
            "State",
            "Host",
            "vCPUs",
            "Memory limit",
            "Pool",
            "Inventory"
        ]
    );
    let names = |h: &Harness| -> Vec<String> {
        h.model
            .areas
            .search
            .sheet
            .shown
            .iter()
            .map(|row| row.cells[0].clone())
            .collect()
    };
    let by_name = names(&h);
    assert_eq!(by_name[0], "Application");
    // A synthetic inventory is never current.
    assert!(sheet(&h).shown.iter().all(|row| row.cells[6] == "Stale"));
    h.sort("search-results", "name");
    assert_eq!(sheet(&h).sort, Some(("name".into(), false)));
    assert_eq!(names(&h), by_name);
    h.sort("search-results", "name");
    assert_eq!(sheet(&h).sort, Some(("name".into(), true)));
    let mut reversed = by_name.clone();
    reversed.reverse();
    assert_eq!(names(&h), reversed);
    assert_eq!(
        h.node("search-results#name").value.as_deref(),
        Some("descending")
    );
    // The order shown is the order of the rows of the table.
    let first = sheet(&h).shown[0].id.clone();
    assert_eq!(row_ids(&h, "search-results")[0], first);
    // By state: the halted guests first.
    h.sort("search-results", "state");
    let states: Vec<String> = sheet(&h)
        .shown
        .iter()
        .map(|row| row.cells[1].clone())
        .collect();
    assert!(states.is_sorted());
    assert_eq!(states[0], "Halted");
    // Choosing a grouping returns to the order of the query.
    h.click("search-group-Name");
    assert_eq!(sheet(&h).sort, None);
    assert_eq!(names(&h), by_name);
    // The sort order survives "Clear filters"; the filters do not.
    h.sort("search-results", "name");
    h.click("search-reset");
    assert_eq!(sheet(&h).sort, Some(("name".into(), false)));
    assert!(sheet(&h).query.kinds.is_empty());
    // The other pages.
    for (kind, expected) in [
        ("Storage", vec!["Name", "Type", "Used", "Capacity"]),
        ("Network", vec!["Name", "Bridge"]),
    ] {
        h.click(&format!("search-kind-{kind}"));
        assert_eq!(headers(&h, "search-results")[..expected.len()], expected);
        assert!(!sheet(&h).shown.is_empty());
        assert!(
            sheet(&h)
                .shown
                .iter()
                .all(|row| row.cells.iter().all(|cell| !cell.is_empty()))
        );
        h.click(&format!("search-kind-{kind}"));
    }
}

#[test]
fn the_selected_rows_are_the_target_of_the_commands() {
    let mut h = Harness::with(Model::demo(3, 48));
    let inventory = h.model.inventory.clone().unwrap();
    let pool = inventory.pool_id;
    h.open_tab(TabId::Search);
    h.click("search-filters");
    h.click("search-kind-VM");
    let keys: Vec<String> = sheet(&h)
        .shown
        .iter()
        .take(3)
        .map(|r| r.id.clone())
        .collect();
    let rows: Vec<&str> = keys.iter().map(String::as_str).collect();
    h.select_rows("search-results", &rows);
    let target = h.model.target();
    assert_eq!(target.len(), 3);
    assert!(target.all_of(ObjectKind::Vm));
    assert_eq!(h.model.scope(), &Target::one(ObjectRef::pool(pool)));
    for key in &keys {
        assert_eq!(h.node(&format!("search-results:{key}")).checked, Some(true));
    }
    // Opening belongs to one row.
    assert!(h.find("search.open-result").is_none());
    assert!(h.find("search-result").is_none());
    // A row stands for its object: a secondary press selects it and opens
    // the kernel's context menu of the object, in the grid.
    let first = object_of(sheet(&h).result(&keys[0]).unwrap());
    h.select_row("search-results", &keys[1]);
    let menu = h.open_row_menu("search-results", &keys[0]);
    assert_eq!(h.model.target(), Target::one(first.clone()));
    let open = h.model.selection.menu.clone().expect("the menu is open");
    assert_eq!((open.origin, open.object), (MenuOrigin::Grid, first));
    assert!(h.row_menu("search-results").is_empty(), "no menu of the table's own");
    let item = menu.iter().find(|item| item.id == OPEN_RESULT).unwrap();
    assert_eq!(item.disabled.as_deref(), Some(query::SYNTHETIC));
    assert!(menu.iter().any(|item| item.id.0 == "object.copy-uuid"));
    // A secondary press on a row of a larger selection keeps the selection.
    h.select_rows("search-results", &rows);
    h.open_row_menu("search-results", rows[1]);
    assert_eq!(h.model.target().len(), 3);
    assert_eq!(h.model.selection.menu.as_ref().unwrap().target.len(), 3);
    h.model.close_context_menu();
    // Escape in the grid clears the selection.
    h.escape_in_table("search-results");
    assert!(h.model.selection.grid.is_empty());
    assert_eq!(&h.model.target(), h.model.scope());
    assert!(
        h.nodes()
            .iter()
            .filter(|node| node.role == "row")
            .all(|node| node.checked == Some(false))
    );
    // A row that leaves the results leaves the selection; one that stays
    // keeps it.
    h.select_rows("search-results", &rows);
    let gone = object_of(sheet(&h).result(&keys[0]).unwrap());
    let mut next = inventory.as_ref().clone();
    next.vms.remove(&gone.vm_id().unwrap());
    next.revision += 1;
    h.model.inventory = Some(Arc::new(next));
    h.tick(0.1);
    assert_eq!(h.model.target().len(), 2);
    assert!(!h.model.target().contains(&gone));
    // An invalid query empties the results and says why; it never falls
    // back to an unfiltered list.
    h.type_text("search-folder", "Services");
    h.click("search-apply-filters");
    assert!(sheet(&h).rows.is_empty());
    assert_eq!(
        h.node("search-error").text.as_deref(),
        Some("Folder must be a full path such as /Services")
    );
    assert!(h.model.selection.grid.is_empty());
    assert_eq!(
        h.node("search-results-empty").text.as_deref(),
        Some("No matching objects")
    );
}

#[test]
fn tag_and_field_drafts_are_added_once_and_removed_by_their_buttons() {
    let mut h = Harness::scene("search-pools");
    h.click("search-filters");
    h.type_text("search-tags", " lab ");
    h.type_text("search-field", "Owner");
    h.type_text("search-field-value", "Operations");
    h.type_text("search-text", "  database ");
    h.submit("search-text");
    let w = sheet(&h);
    assert_eq!(w.query.text, "database");
    assert_eq!(w.query.tags, ["lab".to_owned()].into());
    assert_eq!(w.query.custom_fields["Owner"], "Operations");
    assert!(w.tags.is_empty() && w.field.is_empty() && w.value.is_empty());
    assert_eq!(
        h.node("search-remove-tag-0").label.as_deref(),
        Some("Remove tag: lab")
    );
    h.click("search-remove-tag-0");
    assert!(sheet(&h).query.tags.is_empty());
    assert_eq!(
        h.node("search-remove-field-0").label.as_deref(),
        Some("Remove field: Owner = Operations")
    );
    h.click("search-remove-field-0");
    assert!(sheet(&h).query.custom_fields.is_empty());
    // The power state is one choice.
    h.pick("search-power", "Halted");
    assert_eq!(sheet(&h).query.power, Some(PowerState::Halted));
    assert!(
        sheet(&h)
            .rows
            .iter()
            .all(|r| r.power == Some(PowerState::Halted))
    );
    h.pick("search-power", "Any");
    assert_eq!(sheet(&h).query.power, None);
    // The pool scope lists the sources.
    let pool = *sheet(&h).sources.keys().next().unwrap();
    h.click(&format!("search-pool-{pool}"));
    assert_eq!(sheet(&h).query.pools, [pool].into());
    assert!(sheet(&h).rows.iter().all(|r| r.pool == pool));
}

#[test]
fn saved_searches_are_rows_of_their_view_and_can_be_exported_and_deleted() {
    let mut h = Harness::with(Model::demo(3, 48));
    h.open_tab(TabId::Search);
    h.open_view(NavView::SavedSearches);
    assert!(h.nav_rows().is_empty());
    assert_eq!(
        h.try_click("search.export"),
        Err("There are no saved searches to export.".into())
    );
    for name in ["Halted guests 10", "Halted guests 2"] {
        h.click("search.save");
        h.type_text("search-name", name);
        h.click("search-save");
    }
    let labels: Vec<String> = h.nav_rows().into_iter().map(|row| row.label).collect();
    assert_eq!(labels, ["Halted guests 2", "Halted guests 10"]);
    assert!(
        h.nav_rows()
            .iter()
            .all(|row| row.icon == ObjectKind::SavedSearch)
    );
    // A name that cannot be stored is refused in the dialog, which stays.
    h.click("search.save");
    h.type_text("search-name", "");
    h.click("search-save");
    assert_eq!(h.surface_id().as_deref(), Some("search-save-dialog"));
    assert!(h.find("search-save-error").is_some());
    assert_eq!(h.model.preferences.saved_searches.searches.len(), 2);
    h.click("search-save-cancel");
    // Export writes the definitions, privately.
    let d = TempDir::new("search");
    let path = d.path().join("searches.json");
    h.click("search.export");
    assert_eq!(file_prompt(&h), FileMode::Save);
    h.answer_file(Some(path.clone()));
    wait_for_files(&mut h);
    assert_eq!(
        SearchCatalog::decode(&std::fs::read(&path).unwrap()).unwrap(),
        h.model.preferences.saved_searches
    );
    assert_eq!(
        h.node("search-notice").text.as_deref(),
        Some("Saved search definitions exported; no result inventory was included")
    );
    // Delete: from the context menu of the row, or the local row while the
    // saved search is selected. The selection then moves to nothing.
    h.press_nav("Halted guests 2");
    let object = saved_object("Halted guests 2");
    let menu = h.open_menu(object.clone(), MenuOrigin::Tree);
    assert!(menu.iter().any(|item| item.id == REMOVE));
    h.choose("search.remove");
    assert_eq!(h.model.preferences.saved_searches.searches.len(), 1);
    assert!(h.model.scope().is_empty());
    assert!(h.find("search.remove").is_none(), "hidden without one");
    assert_eq!(
        h.model.eligibility(REMOVE, &Target::one(object)).reason(),
        Some("This saved search is no longer stored on this computer.")
    );
}

#[test]
fn the_context_scene_opens_the_menu_of_the_first_guest() {
    let h = Harness::scene("inventory-context");
    assert_eq!(h.model.active_tab(), TabId::Search);
    let menu = h.model.selection.menu.as_ref().expect("a context menu");
    assert_eq!(menu.origin, MenuOrigin::Grid);
    assert_eq!(
        h.model.name_of(&menu.object).as_deref(),
        Some("Application")
    );
    assert_eq!(h.model.target(), Target::one(menu.object.clone()));
    assert!(h.model.context_menu_valid());
    let items: Vec<_> = h.model.context_menu().into_iter().flatten().collect();
    assert!(items.iter().any(|item| item.id.0 == "object.copy-uuid"));
    assert!(items.iter().any(|item| item.id == OPEN_RESULT));
    // The scene tree says which menu is open and what it offers.
    let tree = h.tree();
    let shown = &tree["context_menu"];
    assert_eq!(shown["origin"], "grid");
    assert_eq!(shown["name"], "Application");
    assert_eq!(shown["object"], serde_json::json!(menu.object));
    assert_eq!(shown["target"], serde_json::json!([menu.object]));
    assert_eq!(shown["valid"], true);
    let listed: Vec<&serde_json::Value> = shown["items"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|group| group.as_array().unwrap())
        .collect();
    assert_eq!(listed.len(), items.len());
    for (listed, item) in listed.iter().zip(&items) {
        assert_eq!(listed["id"], item.id.0);
        assert_eq!(listed["label"], item.label.as_str());
        assert_eq!(
            listed.get("disabled").and_then(|reason| reason.as_str()),
            item.disabled.as_deref()
        );
    }
    assert!(items.iter().any(|item| item.disabled.is_some()));
    // No menu: the entry is null.
    assert!(Harness::scene("search-pools").tree()["context_menu"].is_null());
}

#[test]
fn only_the_file_dialogs_carry_the_experimental_notice() {
    let mut h = Harness::scene("search-pools");
    assert!(!h.model.experimental_notice());
    h.click("search.save");
    assert!(!h.model.experimental_notice());
    h.dismiss();
    h.invoke("tools.inventory-report");
    assert!(h.model.experimental_notice());
    h.dismiss();
    assert!(h.surface_id().is_none());
    // The commands are where the plan puts them.
    assert_eq!(
        h.menu_item("file.import-search").unwrap().label,
        "Import Search"
    );
    assert_eq!(
        h.menu_item("tools.inventory-report").unwrap().label,
        "Inventory Report"
    );
    let local: Vec<&str> = h.local_row().iter().map(|item| item.id.0).collect();
    assert_eq!(local, ["search.save", "search.export"]);
}

#[test]
fn a_result_opens_only_in_a_current_pool_and_its_place_is_checked_again_after_the_move() {
    let mut h = Harness::with(Model::demo(3, 48));
    let inventory = h.model.inventory.clone().unwrap();
    let pool = inventory.pool_id;
    h.select(Target::none());
    h.open_tab(TabId::Search);
    let results = InventorySearch::default().run(&inventory).unwrap();
    let vm = inventory.guests().next().unwrap().id;
    let result = results.iter().find(|r| r.object == Id::Vm(vm)).unwrap();
    // The source decides first: none, synthetic, stale, current.
    let mut w = sheet(&h).clone();
    assert_eq!(query::open_refusal(&w, result), Some(query::SYNTHETIC));
    let source = w.sources.get_mut(&pool).unwrap();
    source.profile = Some(crate::demo::profile_id());
    assert_eq!(query::open_refusal(&w, result), Some(query::RECONNECT));
    w.sources.get_mut(&pool).unwrap().current = true;
    assert_eq!(query::open_refusal(&w, result), None);
    w.sources.clear();
    assert_eq!(query::open_refusal(&w, result), Some(query::UNAVAILABLE));
    // A source that claims to be current does not open a result while the
    // session itself is not: the session is asked again.
    {
        let source = h.model.areas.search.sheet.sources.get_mut(&pool).unwrap();
        source.profile = Some(crate::demo::profile_id());
        source.current = true;
    }
    navigate(&mut h.model, &result.key());
    assert!(h.model.scope().is_empty());
    assert_eq!(sheet(&h).error.as_deref(), Some(query::RECONNECT));
    // A validated place becomes the tree selection and shows the object,
    // in the tree that has it.
    h.open_view(NavView::SavedSearches);
    let destination = navigation(&inventory, result).unwrap();
    show(&mut h.model, result, destination);
    assert_eq!(h.model.scope(), &Target::one(ObjectRef::vm(pool, vm)));
    assert_eq!(h.model.active_tab(), TabId::General);
    assert_eq!(h.model.nav.view, NavView::Infrastructure);
    // A snapshot opens its VM on the Snapshots tab with the snapshot
    // selected there.
    let snapshot = results
        .iter()
        .find(|r| r.kind == SearchKind::Snapshot)
        .expect("the demo has snapshots");
    let destination = navigation(&inventory, snapshot).unwrap();
    let parent = destination.object.clone();
    h.select(Target::none());
    h.open_tab(TabId::Search);
    show(&mut h.model, snapshot, destination);
    assert_eq!(h.model.scope(), &Target::one(parent));
    assert_eq!(h.model.active_tab(), TabId::Snapshots);
    assert_eq!(h.model.target(), Target::one(object_of(snapshot)));
    // The inventory of the scope session no longer has the object: the
    // selection returns to where it was and the reason is shown.
    h.select(ObjectRef::pool(pool));
    h.open_tab(TabId::Search);
    let destination = navigation(&inventory, result).unwrap();
    let mut next = inventory.as_ref().clone();
    next.vms.remove(&vm);
    next.revision += 1;
    h.model.inventory = Some(Arc::new(next));
    show(&mut h.model, result, destination);
    assert_eq!(h.model.scope(), &Target::one(ObjectRef::pool(pool)));
    assert_eq!(h.model.active_tab(), TabId::Search);
    assert_eq!(sheet(&h).error.as_deref(), Some(query::UNAVAILABLE));
    assert_eq!(
        h.node("search-error").text.as_deref(),
        Some(query::UNAVAILABLE)
    );
}
