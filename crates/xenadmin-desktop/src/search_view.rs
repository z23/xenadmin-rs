use super::*;
use xenadmin_core::{
    InventorySearch, OrganizationObjectId as Id, ProfileId, SavedSearch, SearchCatalog,
    SearchGroup, SearchKind, SearchResult,
};
#[path = "search_files.rs"]
mod files;

pub(super) struct Workspace {
    token: uuid::Uuid,
    query: InventorySearch,
    text: TextEdit,
    host: TextEdit,
    tags: TextEdit,
    folder: TextEdit,
    field: TextEdit,
    value: TextEdit,
    name: TextEdit,
    advanced: bool,
    saved: bool,
    rows: Vec<SearchResult>,
    table: TableState,
    sources: BTreeMap<PoolId, Source>,
    signature: Vec<(ProfileId, u64, u64, bool)>,
    unavailable: usize,
    notice: Option<String>,
    error: Option<String>,
    files: files::Pending,
    importing: Option<SearchCatalog>,
    synthetic: Vec<Inventory>,
}
struct Source {
    name: String,
    profile: Option<ProfileId>,
    current: bool,
    complete: bool,
}
fn edit(s: &mut State) -> &mut Workspace {
    s.searches.as_mut().expect("search workspace")
}
pub(super) fn open(s: &mut State) {
    s.searches = Some(Workspace {
        token: uuid::Uuid::new_v4(),
        query: Default::default(),
        text: TextEdit::new(""),
        host: TextEdit::new(""),
        tags: TextEdit::new(""),
        folder: TextEdit::new(""),
        field: TextEdit::new(""),
        value: TextEdit::new(""),
        name: TextEdit::new(""),
        advanced: false,
        saved: false,
        rows: Vec::new(),
        table: TableState::new().key("search-results"),
        sources: BTreeMap::new(),
        signature: Vec::new(),
        unavailable: 0,
        notice: None,
        error: None,
        files: Default::default(),
        importing: None,
        synthetic: if s.demo {
            s.inventory.iter().map(|i| i.as_ref().clone()).collect()
        } else {
            Vec::new()
        },
    });
    rebuild(s);
}
fn apply(s: &mut State) {
    let w = edit(s);
    w.query.text = w.text.text.trim().into();
    w.query.host = w.host.text.trim().into();
    if !w.tags.text.trim().is_empty() {
        w.query.tags.insert(w.tags.text.trim().into());
        w.tags = TextEdit::new("");
    }
    w.query.folder = w.folder.text.trim().into();
    if !w.field.text.trim().is_empty() {
        w.query
            .custom_fields
            .insert(w.field.text.trim().into(), w.value.text.clone());
        w.field = TextEdit::new("");
        w.value = TextEdit::new("");
    }
    rebuild(s);
}
fn rebuild(s: &mut State) {
    let connections = profiles_view::inspection_connections(s).collect::<Vec<_>>();
    let selected_profile = s.preferences.last_profile;
    let w = edit(s);
    w.rows.clear();
    w.sources.clear();
    w.signature.clear();
    w.unavailable = 0;
    if let Err(error) = w.query.validate() {
        w.error = Some(error);
        return;
    }
    w.error = None;
    let mut pools = BTreeMap::<PoolId, (Option<ProfileId>, bool, Arc<Inventory>)>::new();
    for (id, connection) in connections {
        let snapshot = connection.latest();
        w.signature.push((
            id,
            snapshot.generation(),
            snapshot.inventory().map_or(0, |i| i.revision),
            snapshot.is_current(),
        ));
        if let Some(inventory) = snapshot.inventory() {
            add_source(
                &mut pools,
                id,
                snapshot.is_current(),
                inventory.clone(),
                selected_profile,
            );
        } else {
            w.unavailable += 1;
        }
    }
    for inventory in &w.synthetic {
        pools.insert(
            inventory.pool_id,
            (None, false, Arc::new(inventory.clone())),
        );
    }
    for (pool, (profile, current, inventory)) in pools {
        w.sources.insert(
            pool,
            Source {
                name: inventory.pool_name.clone(),
                profile,
                current,
                complete: inventory.details.organization.complete,
            },
        );
        match w.query.run(&inventory) {
            Ok(rows) => w.rows.extend(rows),
            Err(error) => w.error = Some(error),
        }
    }
    w.query.sort(&mut w.rows);
    if w.table
        .cursor()
        .is_some_and(|key| !w.rows.iter().any(|r| r.key() == key))
    {
        w.table = TableState::new().key("search-results");
    }
}
type PoolSources = BTreeMap<PoolId, (Option<ProfileId>, bool, Arc<Inventory>)>;
fn add_source(
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
pub(super) fn poll(s: &mut State) -> bool {
    if s.searches.is_none() {
        return false;
    }
    let changed = files::poll(s);
    let signature = profiles_view::inspection_connections(s)
        .map(|(id, c)| {
            let snapshot = c.latest();
            (
                id,
                snapshot.generation(),
                snapshot.inventory().map_or(0, |i| i.revision),
                snapshot.is_current(),
            )
        })
        .collect::<Vec<_>>();
    if signature != edit(s).signature {
        rebuild(s);
        return true;
    }
    changed
}
fn selected(w: &Workspace) -> Option<&SearchResult> {
    w.table
        .cursor()
        .and_then(|key| w.rows.iter().find(|r| r.key() == key))
}
fn save(s: &mut State) {
    apply(s);
    let w = edit(s);
    if w.error.is_some() {
        return;
    }
    let saved = SavedSearch {
        id: uuid::Uuid::new_v4(),
        name: w.name.text.trim().into(),
        query: w.query.clone(),
    };
    let mut next = s.preferences.saved_searches.clone();
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
    match persist(s, next) {
        Ok(()) => {
            edit(s).notice = Some("Search saved on this computer".into());
        }
        Err(e) => edit(s).error = Some(e),
    }
}
fn persist(s: &mut State, next: SearchCatalog) -> Result<(), String> {
    next.encode()?;
    let previous = std::mem::replace(&mut s.preferences.saved_searches, next);
    if s._instance.is_some()
        && let Err(error) = s.preferences.save()
    {
        s.preferences.saved_searches = previous;
        return Err(error);
    }
    Ok(())
}
fn load(s: &mut State, id: uuid::Uuid) {
    let Some(saved) = s
        .preferences
        .saved_searches
        .searches
        .iter()
        .find(|v| v.id == id)
        .cloned()
    else {
        return;
    };
    let w = edit(s);
    w.text = TextEdit::new(&saved.query.text);
    w.host = TextEdit::new(&saved.query.host);
    w.tags = TextEdit::new("");
    w.folder = TextEdit::new(&saved.query.folder);
    w.field = TextEdit::new("");
    w.value = TextEdit::new("");
    w.name = TextEdit::new(&saved.name);
    w.query = saved.query;
    w.saved = false;
    rebuild(s);
}
struct Navigation {
    page: Page,
    target: Option<String>,
    snapshot: Option<xenadmin_core::SnapshotId>,
}
fn navigation(inventory: &Inventory, result: &SearchResult) -> Result<Navigation, String> {
    let unavailable =
        || "This result or its parent is no longer available. Refresh the search.".to_string();
    if inventory.pool_id != result.pool {
        return Err(unavailable());
    }
    let (page, target, snapshot) = match result.object {
        Id::Pool(id) if id == inventory.pool_id => (Page::Hosts, None, None),
        Id::Host(id) if inventory.hosts.contains_key(&id) => {
            (Page::Hosts, Some(id.to_string()), None)
        }
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
                    Page::Templates
                } else {
                    Page::Guests
                },
                Some(id.to_string()),
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
            (Page::Guests, Some(vm.id.to_string()), Some(id))
        }
        Id::Disk(id) => {
            let sr = inventory
                .details
                .vdis
                .get(&id)
                .and_then(|d| d.sr)
                .filter(|id| inventory.storage.iter().any(|sr| sr.uuid == id.uuid()))
                .ok_or_else(unavailable)?;
            (Page::Storage, Some(sr.to_string()), None)
        }
        Id::Storage(id) if inventory.storage.iter().any(|sr| sr.uuid == id.uuid()) => {
            (Page::Storage, Some(id.to_string()), None)
        }
        Id::Network(id) if inventory.networks.iter().any(|n| n.uuid == id.uuid()) => {
            (Page::Networks, Some(id.to_string()), None)
        }
        _ => return Err(unavailable()),
    };
    Ok(Navigation {
        page,
        target,
        snapshot,
    })
}
fn navigate(s: &mut State) {
    let w = edit(s);
    let Some(result) = selected(w).cloned() else {
        return;
    };
    let Some(profile) = w
        .sources
        .get(&result.pool)
        .and_then(|source| source.profile)
    else {
        return;
    };
    let current = profiles_view::inspection_connections(s)
        .find(|(id, _)| *id == profile)
        .map(|(_, c)| c.latest());
    if !current.as_ref().is_some_and(|snapshot| {
        snapshot.is_current()
            && snapshot
                .inventory()
                .is_some_and(|i| i.pool_id == result.pool)
    }) {
        edit(s).error = Some("Reconnect this pool before opening the result".into());
        return;
    }
    if let Err(error) = current
        .as_ref()
        .and_then(|snapshot| snapshot.inventory())
        .ok_or_else(|| "Pool inventory is unavailable".into())
        .and_then(|i| navigation(i, &result))
    {
        edit(s).error = Some(error);
        return;
    }
    profiles_view::select(s, profile);
    let navigation = s
        .inventory
        .as_ref()
        .ok_or_else(|| "Pool inventory is unavailable".into())
        .and_then(|i| navigation(i, &result));
    let navigation = match navigation {
        Ok(n) => n,
        Err(error) => {
            edit(s).error = Some(error);
            return;
        }
    };
    s.searches = None;
    s.page = navigation.page;
    s.filter_rows();
    if let Some(target) = navigation.target {
        s.table.select(target);
        s.show_details = true;
    }
    if let Some(id) = navigation.snapshot {
        vm_snapshots::open(s);
        s.snapshot_table.select(id.to_string());
    }
}
fn text_field(
    k: &Kit,
    value: &TextEdit,
    label: &'static str,
    key: &'static str,
    editor: fn(&mut State) -> &mut TextEdit,
) -> El {
    col().gap(4.0).child(caption(label, k)).child(
        TextInput::new(value)
            .label(label)
            .on_edit(editor)
            .build(k)
            .key(key),
    )
}
pub(super) fn dialog(s: &State, k: &Kit) -> El {
    let w = s.searches.as_ref().expect("search workspace");
    let mut body = col()
        .gap(10.0)
        .w_full()
        .child(
            row()
                .gap(8.0)
                .center_y()
                .child(heading("Search all pools", k).grow(1.0))
                .child(
                    Button::new("Close")
                        .on_press(|s: &mut State| s.searches = None)
                        .build(k)
                        .key("search-close"),
                ),
        )
        .child(text_field(
            k,
            &w.text,
            "Name, UUID, description, host or pool",
            "search-text",
            |s| &mut edit(s).text,
        ))
        .child(
            row()
                .gap(6.0)
                .child(
                    Button::new("Search")
                        .on_press(apply)
                        .build(k)
                        .key("search-apply"),
                )
                .child(
                    Button::new("Filters")
                        .on_press(|s: &mut State| edit(s).advanced = !edit(s).advanced)
                        .build(k)
                        .key("search-filters"),
                )
                .child(
                    Button::new("Saved searches")
                        .on_press(|s: &mut State| edit(s).saved = !edit(s).saved)
                        .build(k)
                        .key("search-saved"),
                ),
        );
    if w.advanced {
        let mut kinds = col()
            .gap(4.0)
            .child(caption("Object types (none selected means all)", k));
        for pair in SearchKind::ALL.chunks(2) {
            let mut line = row().gap(8.0);
            for kind in pair {
                let kind = *kind;
                line = line.child(
                    Checkbox::new(w.query.kinds.contains(&kind))
                        .label(kind.label())
                        .on_toggle(move |s: &mut State| {
                            let q = &mut edit(s).query;
                            if !q.kinds.remove(&kind) {
                                q.kinds.insert(kind);
                            }
                            rebuild(s);
                        })
                        .build(k)
                        .key(format!("search-kind-{}", kind.label()))
                        .grow(1.0),
                );
            }
            kinds = kinds.child(line);
        }
        body = body.child(kinds).child(caption("VM power state", k));
        let states = [
            None,
            Some(PowerState::Running),
            Some(PowerState::Halted),
            Some(PowerState::Paused),
            Some(PowerState::Suspended),
        ];
        for pair in states.chunks(2) {
            let mut line = row().gap(6.0);
            for power in pair {
                let power = power.clone();
                let label = power
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or("Any".into());
                line = line.child(
                    Checkbox::new(w.query.power == power)
                        .label(label)
                        .on_toggle(move |s: &mut State| {
                            edit(s).query.power = power.clone();
                            rebuild(s);
                        })
                        .build(k)
                        .grow(1.0),
                );
            }
            body = body.child(line);
        }
        body = body
            .child(text_field(
                k,
                &w.host,
                "Host name, address or UUID contains",
                "search-host",
                |s| &mut edit(s).host,
            ))
            .child(text_field(
                k,
                &w.tags,
                "Add required tag (exact; all tags must match)",
                "search-tags",
                |s| &mut edit(s).tags,
            ))
            .child(text_field(
                k,
                &w.folder,
                "Folder and descendants",
                "search-folder",
                |s| &mut edit(s).folder,
            ))
            .child(text_field(
                k,
                &w.field,
                "Custom field name (exact)",
                "search-field",
                |s| &mut edit(s).field,
            ))
            .child(text_field(
                k,
                &w.value,
                "Custom field value (exact)",
                "search-field-value",
                |s| &mut edit(s).value,
            ));
        for tag in &w.query.tags {
            let tag = tag.clone();
            body = body.child(
                Button::new(format!("Remove tag: {tag}"))
                    .on_press(move |s: &mut State| {
                        edit(s).query.tags.remove(&tag);
                        rebuild(s);
                    })
                    .build(k),
            );
        }
        for (key, value) in &w.query.custom_fields {
            let key = key.clone();
            body = body.child(
                Button::new(format!("Remove field: {key} = {value}"))
                    .on_press(move |s: &mut State| {
                        edit(s).query.custom_fields.remove(&key);
                        rebuild(s);
                    })
                    .build(k),
            );
        }
        body = body.child(
            Button::new("Apply filters")
                .on_press(apply)
                .build(k)
                .key("search-apply-filters"),
        );
        body = body.child(caption(
            "Pool scope (none selected means all connected pools)",
            k,
        ));
        for (pool, source) in &w.sources {
            let pool = *pool;
            let label = format!("{} · {pool}", source.name);
            body = body.child(
                Checkbox::new(w.query.pools.contains(&pool))
                    .label(label)
                    .on_toggle(move |s: &mut State| {
                        if !edit(s).query.pools.remove(&pool) {
                            edit(s).query.pools.insert(pool);
                        }
                        rebuild(s);
                    })
                    .build(k),
            );
        }
    }
    if w.saved {
        body = body
            .child(text_field(
                k,
                &w.name,
                "Search name (saving again updates its filters)",
                "search-name",
                |s| &mut edit(s).name,
            ))
            .child(
                row()
                    .gap(6.0)
                    .child(
                        Button::new("Save")
                            .on_press(save)
                            .build(k)
                            .key("search-save"),
                    )
                    .child(
                        Button::new("Import…")
                            .disabled(w.files.busy)
                            .on_press_tx(files::import)
                            .build(k)
                            .key("search-import"),
                    )
                    .child(
                        Button::new("Export…")
                            .disabled(
                                w.files.busy || s.preferences.saved_searches.searches.is_empty(),
                            )
                            .on_press_tx(files::export)
                            .build(k)
                            .key("search-export"),
                    ),
            );
        for saved in &s.preferences.saved_searches.searches {
            let id = saved.id;
            body = body.child(
                row()
                    .gap(6.0)
                    .child(
                        Button::new(&saved.name)
                            .on_press(move |s: &mut State| load(s, id))
                            .build(k)
                            .grow(1.0)
                            .key(format!("search-load-{id}")),
                    )
                    .child(
                        Button::new("Remove")
                            .on_press(move |s: &mut State| {
                                let mut next = s.preferences.saved_searches.clone();
                                next.searches.retain(|v| v.id != id);
                                if let Err(error) = persist(s, next) {
                                    edit(s).error = Some(error);
                                }
                            })
                            .build(k),
                    ),
            );
        }
    }
    if let Some(imported) = &w.importing {
        body = body.child(heading("Review imported searches", k)).child(caption("Only filter definitions will be added. Existing searches will be preserved. Nothing connects or runs automatically.", k).wrap());
        for definition in &imported.searches {
            body = body.child(
                caption(
                    format!(
                        "{} · {} · {} object type filters",
                        definition.name,
                        definition.query.text,
                        definition.query.kinds.len()
                    ),
                    k,
                )
                .wrap_anywhere(),
            );
            let q = &definition.query;
            body = body.child(caption(format!("Types: {}\nPools: {}\nPower: {} · Host: {}\nTags: {} · Folder: {}\nCustom fields: {}",
                if q.kinds.is_empty() { "All".into() } else { q.kinds.iter().map(|k| k.label()).collect::<Vec<_>>().join(", ") },
                if q.pools.is_empty() { "All connected".into() } else { q.pools.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ") },
                q.power.as_ref().map(ToString::to_string).unwrap_or("Any".into()), q.host,
                q.tags.iter().cloned().collect::<Vec<_>>().join(", "), q.folder,
                q.custom_fields.iter().map(|(k,v)| format!("{k} = {v}")).collect::<Vec<_>>().join("; ")), k).wrap_anywhere());
        }
        body = body.child(
            row()
                .gap(6.0)
                .child(
                    Button::new("Add definitions")
                        .on_press(|s: &mut State| {
                            let imported = edit(s).importing.clone().expect("import review");
                            match s.preferences.saved_searches.merged(&imported) {
                                Ok(next) => match persist(s, next) {
                                    Ok(()) => edit(s).importing = None,
                                    Err(error) => edit(s).error = Some(error),
                                },
                                Err(error) => edit(s).error = Some(error),
                            }
                        })
                        .build(k)
                        .key("search-import-apply"),
                )
                .child(
                    Button::new("Discard import")
                        .on_press(|s: &mut State| edit(s).importing = None)
                        .build(k),
                ),
        );
    }
    if let Some(error) = &w.error {
        body = body.child(caption(error, k).color(k.c.danger.icon).wrap());
    }
    if let Some(notice) = &w.notice {
        body = body.child(caption(notice, k).wrap());
    }
    let stale = w.sources.values().filter(|source| !source.current).count();
    let partial = w.sources.values().filter(|source| !source.complete).count();
    body = body.child(caption(format!("{} results · {} pools · {stale} stale · {partial} with incomplete metadata · {} connections without inventory", w.rows.len(), w.sources.len(), w.unavailable), k).wrap())
        .child(caption("Results cover readable inventory only. Missing tag, folder or custom-field data never satisfies those filters.", k).wrap());
    let mut groups = row().gap(4.0);
    for (label, group) in [
        ("Name", SearchGroup::Name),
        ("Pool", SearchGroup::Pool),
        ("Type", SearchGroup::Kind),
        ("Folder", SearchGroup::Folder),
    ] {
        groups = groups.child(
            Button::new(label)
                .variant(if w.query.group == group {
                    ButtonVariant::Outline
                } else {
                    ButtonVariant::Ghost
                })
                .on_press(move |s: &mut State| {
                    edit(s).query.group = group;
                    rebuild(s);
                })
                .build(k),
        );
    }
    body = body.child(groups).child(
        Table::new(
            |s: &State| &s.searches.as_ref().unwrap().table,
            |s: &mut State| &mut edit(s).table,
            |s: &State| s.searches.as_ref().unwrap().rows.len(),
            |s: &State, i| Cow::Owned(s.searches.as_ref().unwrap().rows[i].key()),
        )
        .name("Search results")
        .column(TableColumn::new("name", "Name / type").flex(2.0).min(130.0))
        .column(
            TableColumn::new("pool", "Pool / state")
                .flex(1.0)
                .min(110.0),
        )
        .cell(|s: &State, i, column| {
            let w = s.searches.as_ref().unwrap();
            let r = &w.rows[i];
            if column == "name" {
                Cell::text(format!("{} · {}", r.name, r.kind.label()))
            } else {
                Cell::text(format!(
                    "{} · {}",
                    r.pool_name,
                    if w.sources.get(&r.pool).is_some_and(|source| source.current) {
                        "Current"
                    } else {
                        "Stale"
                    }
                ))
            }
        })
        .zebra(true)
        .build(s, k)
        .h(230.0)
        .min_h(170.0),
    );
    if let Some(result) = selected(w) {
        body = body
            .child(heading(&result.name, k))
            .child(
                caption(
                    format!(
                        "{} · {}\nPool: {}\n{}\nFolder: {}",
                        result.kind.label(),
                        result.object.uuid(),
                        result.pool,
                        result.description,
                        result.folder.as_deref().unwrap_or("Not reported")
                    ),
                    k,
                )
                .wrap_anywhere(),
            )
            .child(
                Button::new("Open in pool")
                    .disabled(
                        w.sources
                            .get(&result.pool)
                            .is_none_or(|source| !source.current || source.profile.is_none()),
                    )
                    .on_press(navigate)
                    .build(k)
                    .key("search-open-result"),
            );
    }
    el().w_full()
        .h_full()
        .absolute(0.0, 0.0)
        .center()
        .fill(rune::Color::BLACK.alpha(0.55))
        .role("dialog")
        .label("Search all pools")
        .modal(false, |s: &mut State| s.searches = None)
        .child(
            body.padding(20.0)
                .w((s.window_width - 24.0).clamp(280.0, 900.0))
                .max_h((s.window_height - 24.0).max(280.0))
                .fill(k.c.bg_base)
                .radius(12.0)
                .scroll(rune::ScrollSpec {
                    stick_bottom: false,
                }),
        )
}

pub(super) fn scene(s: &mut State) {
    organization_view::scene(s);
    s.organization = None;
    open(s);
    let mut second = edit(s).synthetic[0].clone();
    second.pool_id = uuid::Uuid::from_u128(99).into();
    second.pool_name = "Recovery lab".into();
    edit(s).synthetic.push(second);
    edit(s).query.kinds.insert(SearchKind::Vm);
    edit(s).query.group = SearchGroup::Pool;
    rebuild(s);
}

#[cfg(test)]
#[path = "search_tests.rs"]
mod tests;
