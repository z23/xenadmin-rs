use super::*;
#[test]
fn search_entry_point_is_reachable_at_both_window_sizes() {
    for size in [(1440.0, 900.0), (513.0, 652.0)] {
        let mut s = init();
        s.window_width = size.0;
        s.window_height = size.1;
        let mut rt = rune::Runtime::new(
            s,
            super::super::view,
            size,
            rune::Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        click(&mut rt, "search-pools-open");
        assert!(
            rt.state.searches.is_some(),
            "search entry unreachable at {size:?}"
        );
    }
}
#[test]
fn aliases_deduplicate_by_pool_and_prefer_current_selected_session_without_comparing_revisions() {
    let mut s = init();
    scene(&mut s);
    let mut older = s.inventory.as_ref().unwrap().as_ref().clone();
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
    let mut s = init();
    scene(&mut s);
    let mut inventory = s.inventory.as_ref().unwrap().as_ref().clone();
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
    assert_eq!(target.target, Some(vm.to_string()));
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
    inventory.vms.remove(&vm);
    assert!(navigation(&inventory, &vm_result).is_err());
    inventory.pool_id = uuid::Uuid::from_u128(903).into();
    assert!(navigation(&inventory, &vm_result).is_err());
}
fn click(rt: &mut rune::Runtime<State>, key: &str) {
    rt.update(|_, tx| tx.scroll_into_view(key));
    for _ in 0..30 {
        rt.frame(1.0 / 60.0);
    }
    let (x, y) = rt.find(key).unwrap_or_else(|| panic!("missing {key}"));
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.0);
    for _ in 0..30 {
        rt.frame(1.0 / 60.0);
    }
}
fn runtime(size: (f32, f32)) -> rune::Runtime<State> {
    let mut s = init();
    scene(&mut s);
    s.window_width = size.0;
    s.window_height = size.1;
    rune::Runtime::new(
        s,
        super::super::view,
        size,
        rune::Env {
            system_dark: true,
            scale: 1.0,
        },
    )
}
#[test]
fn cross_pool_results_keep_duplicate_vm_ids_distinct_and_synthetic_navigation_disabled() {
    for size in [(1440.0, 900.0), (513.0, 652.0)] {
        let mut rt = runtime(size);
        let w = rt.state.searches.as_ref().unwrap();
        assert_eq!(w.sources.len(), 2);
        let keys = w
            .rows
            .iter()
            .map(SearchResult::key)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(keys.len(), w.rows.len());
        let key = w.rows[0].key();
        rt.update(|s, _| edit(s).table.select(key.clone()));
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
        assert!(rt.accessibility_json().to_string().contains("2 stale"));
        click(&mut rt, "search-open-result");
        assert!(rt.state.searches.is_some());
        assert!(rt.state.connection.is_none());
        click(&mut rt, "search-filters");
        click(&mut rt, "search-kind-Host");
        assert!(
            rt.state
                .searches
                .as_ref()
                .unwrap()
                .query
                .kinds
                .contains(&SearchKind::Host)
        );
    }
}
#[test]
fn saving_and_loading_does_not_erase_imported_complex_filters() {
    let mut rt = runtime((513.0, 652.0));
    rt.update(|s, _| {
        let w = edit(s);
        w.query.tags.insert("tier,one".into());
        w.query.tags.insert("production".into());
        w.query
            .custom_fields
            .insert("Owner".into(), "Operations".into());
        w.query
            .custom_fields
            .insert("Service".into(), "Database".into());
        w.name = TextEdit::new("Saved scope");
    });
    click(&mut rt, "search-saved");
    click(&mut rt, "search-save");
    assert_eq!(rt.state.preferences.saved_searches.searches.len(), 1);
    let saved = rt.state.preferences.saved_searches.searches[0].clone();
    rt.update(|s, _| {
        edit(s).query = Default::default();
        load(s, saved.id);
    });
    click(&mut rt, "search-apply");
    assert_eq!(rt.state.searches.as_ref().unwrap().query, saved.query);
}
#[test]
fn imported_queries_require_review_and_conflicts_leave_existing_definitions_intact() {
    let mut rt = runtime((513.0, 652.0));
    let imported = SearchCatalog {
        schema_version: 1,
        searches: vec![SavedSearch {
            id: uuid::Uuid::from_u128(61),
            name: "Imported scope".into(),
            query: Default::default(),
        }],
    };
    rt.update(|s, _| edit(s).importing = Some(imported.clone()));
    assert!(rt.state.preferences.saved_searches.searches.is_empty());
    click(&mut rt, "search-import-apply");
    assert_eq!(rt.state.preferences.saved_searches, imported);
    rt.update(|s, _| {
        let mut conflict = imported.clone();
        conflict.searches[0].query.text = "changed".into();
        edit(s).importing = Some(conflict);
    });
    click(&mut rt, "search-import-apply");
    assert_eq!(rt.state.preferences.saved_searches, imported);
    assert!(
        rt.state
            .searches
            .as_ref()
            .unwrap()
            .error
            .as_deref()
            .unwrap()
            .contains("Conflicting")
    );
}

#[test]
fn native_search_picker_cancellation_and_closed_workspace_callbacks_are_ignored() {
    let mut rt = runtime((513.0, 652.0));
    click(&mut rt, "search-saved");
    click(&mut rt, "search-import");
    assert_eq!(rt.file_dialog().unwrap().0, rune::FileDialogKind::Open);
    rt.resolve_file_dialog(None).unwrap();
    rt.frame(0.0);
    assert!(!rt.state.searches.as_ref().unwrap().files.busy);
    click(&mut rt, "search-import");
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("scope.json");
    std::fs::write(&path, SearchCatalog::default().encode().unwrap()).unwrap();
    rt.update(|s, _| {
        s.searches = None;
        open(s);
    });
    rt.resolve_file_dialog(Some(path)).unwrap();
    rt.frame(0.0);
    assert!(!rt.state.searches.as_ref().unwrap().files.busy);
    assert!(rt.state.searches.as_ref().unwrap().importing.is_none());
}
