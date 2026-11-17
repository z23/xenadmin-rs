//! What the area shows: the body of the Search tab and its three dialogs.

use std::rc::Rc;

use xenadmin_core::{PowerState, SavedSearch, SearchGroup, SearchKind};

use super::query::{self, Sheet};
use super::{Dialog, Import, ReportPanel, files};
use crate::prelude::*;

const READABLE_ONLY: &str = "Results cover readable inventory only. Missing tag, folder or \
     custom-field data never satisfies those filters.";
const REPORT_PRIVACY: &str = "The report captures these results and their freshness, including \
     private names, identifiers and metadata.";

pub fn tab(m: &Model, tab: TabId, _scope: &Target) -> Option<TabBody> {
    if tab != TabId::Search {
        return None;
    }
    let sheet = super::current(m);
    Some(TabBody::Blocks(body(m, &sheet)))
}

/// Stores a draft. The sheet is brought up to date first, so that a preset
/// that is still due does not overwrite what was typed.
fn draft(m: &mut Model, field: fn(&mut Sheet) -> &mut String, text: &str) {
    super::sync(m);
    *field(&mut m.areas.search.sheet) = text.to_owned();
}

fn body(m: &Model, sheet: &Sheet) -> Vec<Block> {
    blocks![
        caption(format!("Scope: {}", query::scope_line(m, sheet))).key("search-scope"),
        input(
            "search-text",
            "Name, UUID, description, host or pool",
            &sheet.text,
            |m, text| draft(m, |sheet| &mut sheet.text, text)
        )
        .on_submit(super::apply),
        actions([
            button("search-apply", "Search", super::apply).primary(),
            button("search-filters", "Filters", |m| {
                m.areas.search.advanced = !m.areas.search.advanced
            }),
            button("search-reset", "Clear filters", super::reset),
        ]),
        m.areas.search.advanced.then(|| filters(sheet)),
        sheet
            .error
            .clone()
            .map(|error| alert(error).key("search-error")),
        sheet
            .notice
            .clone()
            .map(|notice| caption(notice).key("search-notice")),
        caption(sheet.summary()).key("search-summary"),
        caption(READABLE_ONLY),
        grouping(sheet),
        results(m, sheet),
        selected(m, sheet),
    ]
}

/// The Filters section: object types, power state, host, tags, folder,
/// custom fields and the pool scope.
fn filters(sheet: &Sheet) -> Group {
    let kinds: Vec<Field> = SearchKind::ALL
        .into_iter()
        .map(|kind| {
            checkbox(
                format!("search-kind-{}", kind.label()),
                kind.label(),
                sheet.query.kinds.contains(&kind),
                move |m| {
                    super::change(m, |sheet| {
                        if !sheet.query.kinds.remove(&kind) {
                            sheet.query.kinds.insert(kind);
                        }
                    })
                },
            )
        })
        .collect();
    let states = [
        PowerState::Running,
        PowerState::Halted,
        PowerState::Paused,
        PowerState::Suspended,
    ];
    let labels: Vec<String> = std::iter::once("Any".to_owned())
        .chain(states.iter().map(ToString::to_string))
        .collect();
    let power = choice(
        "search-power",
        labels.into_iter().map(|label| (label.clone(), label)),
        Some(
            &sheet
                .query
                .power
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or("Any".into()),
        ),
        move |m, id| {
            let power = states.iter().find(|state| state.to_string() == id).cloned();
            super::change(m, |sheet| sheet.query.power = power)
        },
    )
    .label("VM power state");
    let tags: Vec<Button> = sheet
        .query
        .tags
        .iter()
        .enumerate()
        .map(|(index, tag)| {
            let tag = tag.clone();
            button(
                format!("search-remove-tag-{index}"),
                format!("Remove tag: {tag}"),
                move |m| {
                    let tag = tag.clone();
                    super::change(m, move |sheet| {
                        sheet.query.tags.remove(&tag);
                    })
                },
            )
        })
        .collect();
    let fields: Vec<Button> = sheet
        .query
        .custom_fields
        .iter()
        .enumerate()
        .map(|(index, (key, value))| {
            let key = key.clone();
            button(
                format!("search-remove-field-{index}"),
                format!("Remove field: {key} = {value}"),
                move |m| {
                    let key = key.clone();
                    super::change(m, move |sheet| {
                        sheet.query.custom_fields.remove(&key);
                    })
                },
            )
        })
        .collect();
    let pools: Vec<Field> = sheet
        .sources
        .iter()
        .map(|(pool, source)| {
            let pool = *pool;
            checkbox(
                format!("search-pool-{pool}"),
                format!("{} · {pool}", source.name),
                sheet.query.pools.contains(&pool),
                move |m| {
                    super::change(m, |sheet| {
                        if !sheet.query.pools.remove(&pool) {
                            sheet.query.pools.insert(pool);
                        }
                    })
                },
            )
        })
        .collect();
    group(
        "Filters",
        blocks![
            caption("Object types (none selected means all)"),
            kinds,
            power,
            input(
                "search-host",
                "Host name, address or UUID contains",
                &sheet.host,
                |m, text| draft(m, |sheet| &mut sheet.host, text)
            ),
            input(
                "search-tags",
                "Add required tag (exact; all tags must match)",
                &sheet.tags,
                |m, text| draft(m, |sheet| &mut sheet.tags, text)
            ),
            input(
                "search-folder",
                "Folder and descendants",
                &sheet.folder,
                |m, text| draft(m, |sheet| &mut sheet.folder, text)
            ),
            input(
                "search-field",
                "Custom field name (exact)",
                &sheet.field,
                |m, text| draft(m, |sheet| &mut sheet.field, text)
            ),
            input(
                "search-field-value",
                "Custom field value (exact)",
                &sheet.value,
                |m, text| draft(m, |sheet| &mut sheet.value, text)
            ),
            (!tags.is_empty()).then(|| actions(tags)),
            (!fields.is_empty()).then(|| actions(fields)),
            button("search-apply-filters", "Apply filters", super::apply),
            caption("Pool scope (none selected means all connected pools)"),
            pools,
        ],
    )
    .key("search-filter-section")
}

/// The grouping of the results: the order the query gives them. Choosing
/// one returns the grid to that order from a column sort.
fn grouping(sheet: &Sheet) -> Choice {
    let groups = [
        (SearchGroup::Name, "Name"),
        (SearchGroup::Pool, "Pool"),
        (SearchGroup::Kind, "Type"),
        (SearchGroup::Folder, "Folder"),
    ];
    let current = groups
        .iter()
        .find(|(group, _)| *group == sheet.query.group)
        .map(|(_, label)| *label);
    choice(
        "search-group",
        groups.map(|(_, label)| (label, label)),
        current,
        move |m, id| {
            if let Some((group, _)) = groups.iter().find(|(_, label)| *label == id) {
                let group = *group;
                super::change(m, move |sheet| {
                    sheet.query.group = group;
                    sheet.sort = None;
                })
            }
        },
    )
    .label("Group by")
    .segmented()
}

/// The result grid: multi-selection, sortable columns, activation opens
/// the result, and the context menu of the objects.
fn results(m: &Model, sheet: &Sheet) -> Table {
    let columns = sheet
        .layout()
        .columns()
        .iter()
        .map(|(id, title, sortable, right)| {
            let mut column = column(*id, *title);
            if *sortable {
                column = column.sortable();
            }
            if *right {
                column = column.right();
            }
            column
        });
    let selected: Vec<String> = m
        .selection
        .grid
        .objects
        .iter()
        .filter_map(query::key_of)
        .collect();
    let mut results = Table {
        rows: sheet.shown.clone(),
        ..table("search-results", columns, [])
    }
    .multi(selected, select_rows)
    .on_activate(super::navigate)
    .row_menu(row_menu(sheet))
    .empty(if sheet.sources.is_empty() {
        "No connected pool has an inventory to search."
    } else {
        "No matching objects"
    });
    match &sheet.sort {
        Some((column, descending)) => results.sorted(column.clone(), *descending, sort),
        None => {
            // No column is the sort order yet; the headers still sort.
            results.on_sort = Some(Rc::new(sort));
            results
        }
    }
}

/// The selected rows become the target of the commands.
fn select_rows(m: &mut Model, ids: &[String]) {
    super::sync(m);
    let wanted: std::collections::BTreeSet<&str> = ids.iter().map(String::as_str).collect();
    let objects: Vec<ObjectRef> = m
        .areas
        .search
        .sheet
        .rows
        .iter()
        .filter(|result| wanted.contains(result.key().as_str()))
        .map(query::object_of)
        .collect();
    m.select_in_grid(Target::many(objects));
}

fn sort(m: &mut Model, column: &str, descending: bool) {
    super::sync(m);
    let sheet = &mut m.areas.search.sheet;
    sheet.sort = Some((column.to_owned(), descending));
    query::project(sheet);
}

/// The commands of the row context menu: opening the result, then every
/// command that has one of the result types in its context menu. The
/// commands hide themselves for a row they do not belong to.
fn row_menu(sheet: &Sheet) -> Vec<CommandId> {
    let kinds: Vec<ObjectKind> = if sheet.query.kinds.is_empty() {
        SearchKind::ALL.to_vec()
    } else {
        sheet.query.kinds.iter().copied().collect()
    }
    .into_iter()
    .map(query::object_kind)
    .collect();
    std::iter::once(super::OPEN_RESULT)
        .chain(
            crate::command::registry()
                .iter()
                .filter(|spec| spec.id != super::OPEN_RESULT)
                .filter(|spec| spec.context.iter().any(|kind| kinds.contains(kind)))
                .map(|spec| spec.id),
        )
        .collect()
}

/// What the Rune dialog showed under the table for the selected result.
fn selected(m: &Model, sheet: &Sheet) -> Option<Group> {
    let key = m.selection.grid.single().and_then(query::key_of)?;
    let result = sheet.result(&key)?;
    Some(
        group(
            result.name.clone(),
            blocks![properties([
                property("Type", result.kind.label()),
                property("UUID", result.object.uuid().to_string()).copyable(),
                property("Pool", result.pool.to_string()),
                property("Description", result.description.clone()),
                property(
                    "Folder",
                    result.folder.as_deref().unwrap_or("Not reported")
                ),
            ])],
        )
        .key("search-result"),
    )
}

// -- Dialogs ------------------------------------------------------------------

pub fn surfaces(m: &Model) -> Vec<Surface> {
    let surface = match &m.areas.search.dialog {
        None => return Vec::new(),
        Some(Dialog::Save) => save_dialog(m),
        Some(Dialog::Import(import)) => import_dialog(import),
        Some(Dialog::Report(panel)) => report_dialog(m, panel),
    };
    vec![surface.on_close(super::close_dialog)]
}

/// Save Search. Saving is local and not experimental.
fn save_dialog(m: &Model) -> Surface {
    let sheet = super::current(m);
    let definition = SavedSearch {
        id: uuid::Uuid::nil(),
        name: sheet.name.trim().to_owned(),
        query: sheet.query.clone(),
    };
    let [_, filters] = query::definition_lines(&definition);
    Surface::dialog("search-save-dialog", "Save Search")
        .body(blocks![
            input(
                "search-name",
                "Search name (saving again updates its filters)",
                &sheet.name,
                |m, text| draft(m, |sheet| &mut sheet.name, text)
            )
            .on_submit(super::save),
            caption(
                "The search is saved on this computer with the filters that are applied now, \
                 including the pool and the server the selection set."
            ),
            caption(filters).key("search-save-definition"),
            sheet
                .error
                .clone()
                .map(|error| alert(error).key("search-save-error")),
        ])
        .footer([
            button("search-save-cancel", "Cancel", super::close_dialog),
            button("search-save", "Save", super::save).primary(),
        ])
}

/// The review of an import, or why the file could not be read. Files of
/// saved searches are experimental.
fn import_dialog(import: &Import) -> Surface {
    let error = import
        .error
        .clone()
        .map(|error| alert(error).key("search-import-error"));
    let Some(catalog) = &import.catalog else {
        return Surface::dialog("search-import-dialog", "Import Search")
            .experimental(true)
            .small()
            .body(blocks![error])
            .footer([button(
                "search-import-discard",
                "Close",
                super::close_dialog,
            )]);
    };
    let definitions: Vec<Text> = catalog
        .searches
        .iter()
        .flat_map(query::definition_lines)
        .map(caption)
        .collect();
    Surface::dialog("search-import-dialog", "Review imported searches")
        .experimental(true)
        .body(blocks![
            caption(
                "Only filter definitions will be added. Existing searches will be preserved. \
                 Nothing connects or runs automatically."
            ),
            definitions,
            error,
        ])
        .footer([
            button(
                "search-import-discard",
                "Discard import",
                super::close_dialog,
            ),
            button("search-import-apply", "Add definitions", super::import_apply).primary(),
        ])
}

/// The inventory report: the coverage of the current results, what the
/// file will hold, and the export. Experimental.
fn report_dialog(m: &Model, panel: &ReportPanel) -> Surface {
    let sheet = super::current(m);
    Surface::dialog("inventory-report", "Inventory Report")
        .experimental(true)
        .body(blocks![
            caption(
                "The report holds the results the Search tab shows for the current selection, \
                 with the pools they came from."
            ),
            caption(format!("Scope: {}", query::scope_line(m, &sheet))).key("report-scope"),
            caption(sheet.summary()).key("report-summary"),
            caption(READABLE_ONLY),
            caption(REPORT_PRIVACY),
            sheet
                .error
                .clone()
                .map(|error| alert(error).key("report-search-error")),
            panel
                .error
                .clone()
                .map(|error| alert(error).key("report-error")),
            panel
                .notice
                .clone()
                .map(|notice| caption(notice).key("report-notice")),
        ])
        .footer([
            button("report-close", "Close", super::close_dialog),
            button(
                "search-export-results",
                "Export resource results…",
                files::export_results,
            )
            .primary()
            .disabled_if(m.areas.search.files.busy, files::BUSY)
            .disabled_if(
                sheet.sources.is_empty(),
                "No connected pool has an inventory to report.",
            )
            .disabled_if(
                sheet.error.is_some(),
                "Resolve the search error before exporting results",
            ),
        ])
}
