use super::*;
use std::collections::BTreeSet;
use xenadmin_core::{
    CustomFieldDefinition, CustomFieldType, ObservedResource, OrganizationChange as Change,
    OrganizationEdit as Edit, OrganizationInventory, OrganizationObject,
    OrganizationObjectId as Id, OrganizationPatch, ResourceCommand, ResourceTarget,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    AddTag,
    RemoveTag,
    RenameTag,
    DeleteTag,
    MoveFolder,
    CreateFolder,
    RenameFolder,
    DeleteFolder,
    SetField,
    RemoveField,
    DefineField,
    DeleteField,
}
impl Action {
    fn title(self) -> &'static str {
        match self {
            Self::AddTag => "Add tag to selection",
            Self::RemoveTag => "Remove tag from selection",
            Self::RenameTag => "Rename tag across pool",
            Self::DeleteTag => "Delete tag across pool",
            Self::MoveFolder => "Move selection to folder",
            Self::CreateFolder => "Create folder",
            Self::RenameFolder => "Rename folder across pool",
            Self::DeleteFolder => "Delete folder across pool",
            Self::SetField => "Set custom field on selection",
            Self::RemoveField => "Clear custom field on selection",
            Self::DefineField => "Define custom field",
            Self::DeleteField => "Delete custom field across pool",
        }
    }
}
pub(super) struct Workspace {
    pool: PoolId,
    observed: Option<ObservedResource>,
    inventory: OrganizationInventory,
    query: TextEdit,
    selected: BTreeSet<Id>,
    action: Action,
    name: TextEdit,
    value: TextEdit,
    date: bool,
    review: Option<OrganizationPatch>,
    error: Option<String>,
}
pub(super) fn open(s: &mut State) {
    let Some(inventory) = &s.inventory else {
        return;
    };
    let observed = s.snapshot.as_ref().and_then(|v| {
        v.observe_resource(ResourceTarget::Pool(inventory.pool_id))
            .ok()
    });
    s.organization = Some(Workspace {
        pool: inventory.pool_id,
        observed,
        inventory: inventory.details.organization.clone(),
        query: TextEdit::new(""),
        selected: BTreeSet::new(),
        action: Action::AddTag,
        name: TextEdit::new(""),
        value: TextEdit::new(""),
        date: false,
        review: None,
        error: None,
    });
}
fn edit(s: &mut State) -> &mut Workspace {
    s.organization
        .as_mut()
        .expect("organization workspace open")
}
fn current(s: &State, w: &Workspace) -> bool {
    !s.demo
        && s.can_mutate()
        && w.observed.is_some()
        && w.observed
            == s.snapshot
                .as_ref()
                .and_then(|v| v.observe_resource(ResourceTarget::Pool(w.pool)).ok())
}
fn selected(w: &Workspace) -> impl Iterator<Item = &OrganizationObject> {
    w.inventory
        .objects
        .iter()
        .filter(|o| w.selected.contains(&o.id))
}
fn in_folder(folder: &str, path: &str) -> bool {
    folder == path
        || folder
            .strip_prefix(path)
            .is_some_and(|s| s.starts_with('/'))
}
fn folder(value: &str) -> Result<String, String> {
    if xenadmin_core::valid_folder(value) {
        Ok(value.into())
    } else {
        Err("Use a full folder path such as /Services/Databases; empty parts, semicolons and . or .. are not allowed".into())
    }
}
fn change(edits: &mut Vec<Edit>, object: Id, change: Change) {
    edits.push(Edit { object, change });
}
fn plan(w: &Workspace) -> Result<OrganizationPatch, String> {
    let global = matches!(
        w.action,
        Action::RenameTag
            | Action::DeleteTag
            | Action::RenameFolder
            | Action::DeleteFolder
            | Action::DeleteField
    );
    if global
        && (!w.inventory.complete
            || w.inventory
                .objects
                .iter()
                .any(|o| o.tags.is_none() || !o.other_config_known))
    {
        return Err(
            "Refresh with permission to read all organization metadata before a pool-wide change"
                .into(),
        );
    }
    let mut edits = Vec::new();
    let name = w.name.text.trim();
    let value = w.value.text.trim();
    match w.action {
        Action::AddTag | Action::RemoveTag | Action::RenameTag | Action::DeleteTag => {
            if name.is_empty() {
                return Err("Enter a tag".into());
            }
            if w.action == Action::RenameTag && value.is_empty() {
                return Err("Enter the new tag".into());
            }
            for o in &w.inventory.objects {
                let global = matches!(w.action, Action::RenameTag | Action::DeleteTag);
                if !global && !w.selected.contains(&o.id) {
                    continue;
                }
                let Some(previous) = &o.tags else {
                    if !global {
                        return Err(format!("Tags are unavailable for {}", o.name));
                    }
                    continue;
                };
                if global && !previous.iter().any(|t| t == name) {
                    continue;
                }
                let mut next = previous.clone();
                if w.action != Action::AddTag {
                    next.retain(|v| v != name);
                }
                if matches!(w.action, Action::AddTag | Action::RenameTag) {
                    let tag = if w.action == Action::AddTag {
                        name
                    } else {
                        value
                    };
                    if !next.iter().any(|v| v == tag) {
                        next.push(tag.into());
                    }
                }
                if &next != previous {
                    change(
                        &mut edits,
                        o.id,
                        Change::Tags {
                            previous: previous.clone(),
                            value: next,
                        },
                    );
                }
            }
        }
        Action::MoveFolder => {
            let path = if name.is_empty() {
                None
            } else {
                Some(folder(name)?)
            };
            for o in selected(w) {
                if !o.other_config_known {
                    return Err(format!("Folder metadata is unavailable for {}", o.name));
                }
                if o.folder != path {
                    change(
                        &mut edits,
                        o.id,
                        Change::Folder {
                            previous: o.folder.clone(),
                            value: path.clone(),
                        },
                    );
                }
            }
        }
        Action::CreateFolder | Action::RenameFolder | Action::DeleteFolder => {
            let path = folder(name)?;
            let previous = w
                .inventory
                .empty_folders
                .clone()
                .ok_or("Empty-folder registry is unavailable")?;
            let mut next = previous.clone();
            if w.action == Action::CreateFolder {
                if next.contains(&path)
                    || w.inventory
                        .objects
                        .iter()
                        .any(|o| o.folder.as_deref().is_some_and(|f| in_folder(f, &path)))
                {
                    return Err("That folder already exists".into());
                }
                next.push(path);
            } else {
                let destination = if w.action == Action::RenameFolder {
                    Some(folder(value)?)
                } else {
                    None
                };
                if destination.as_deref().is_some_and(|d| {
                    in_folder(d, &path)
                        || w.inventory
                            .objects
                            .iter()
                            .any(|o| o.folder.as_deref().is_some_and(|f| in_folder(f, d)))
                        || previous.iter().any(|f| in_folder(f, d))
                }) {
                    return Err("Choose a new folder outside the existing folder tree".into());
                }
                let transform = |f: &str| {
                    destination
                        .as_ref()
                        .map(|d| format!("{d}{}", &f[path.len()..]))
                };
                for o in &w.inventory.objects {
                    if let Some(f) = o.folder.as_deref().filter(|f| in_folder(f, &path)) {
                        if !o.other_config_known {
                            return Err(format!("Folder metadata is unavailable for {}", o.name));
                        }
                        change(
                            &mut edits,
                            o.id,
                            Change::Folder {
                                previous: o.folder.clone(),
                                value: transform(f),
                            },
                        );
                    }
                }
                next = previous
                    .iter()
                    .filter_map(|f| {
                        if in_folder(f, &path) {
                            transform(f)
                        } else {
                            Some(f.clone())
                        }
                    })
                    .collect();
            }
            next.sort();
            next.dedup();
            let mut comparison = previous.clone();
            comparison.sort();
            if comparison != next {
                change(
                    &mut edits,
                    Id::Pool(w.pool),
                    Change::EmptyFolders {
                        previous,
                        value: next,
                    },
                );
            }
        }
        Action::DefineField | Action::DeleteField => {
            let previous = w
                .inventory
                .definitions
                .clone()
                .ok_or("Custom field definitions are unavailable")?;
            let mut next = previous.clone();
            if w.action == Action::DefineField {
                if name.is_empty() || previous.iter().any(|d| d.name == name) {
                    return Err("Choose a new custom field name".into());
                }
                next.push(CustomFieldDefinition {
                    name: name.into(),
                    kind: if w.date {
                        CustomFieldType::Date
                    } else {
                        CustomFieldType::String
                    },
                });
            } else {
                let definition = previous
                    .iter()
                    .find(|d| d.name == name)
                    .ok_or("Choose an existing custom field")?;
                for o in &w.inventory.objects {
                    if let Some(value) = o.custom_fields.get(name) {
                        change(
                            &mut edits,
                            o.id,
                            Change::CustomField {
                                definition: definition.clone(),
                                previous: Some(value.clone()),
                                value: None,
                            },
                        );
                    }
                }
                next.retain(|d| d.name != name);
            }
            change(
                &mut edits,
                Id::Pool(w.pool),
                Change::Definitions {
                    previous,
                    value: next,
                },
            );
        }
        Action::SetField | Action::RemoveField => {
            let definition = w
                .inventory
                .definitions
                .as_ref()
                .and_then(|defs| defs.iter().find(|d| d.name == name))
                .ok_or("Choose an existing custom field")?;
            let value = if w.action == Action::RemoveField {
                None
            } else {
                Some(w.value.text.clone())
            };
            for o in selected(w) {
                if !o.other_config_known {
                    return Err(format!(
                        "Custom field metadata is unavailable for {}",
                        o.name
                    ));
                }
                let previous = o.custom_fields.get(name).cloned();
                if previous != value {
                    change(
                        &mut edits,
                        o.id,
                        Change::CustomField {
                            definition: definition.clone(),
                            previous,
                            value: value.clone(),
                        },
                    );
                }
            }
        }
    }
    if edits.is_empty() {
        return Err("No changes: choose objects and enter a different value".into());
    }
    if edits.len() > 256 {
        return Err("This review exceeds 256 changes. Apply smaller selections first".into());
    }
    OrganizationPatch::new(edits).map_err(|e|format!("{e}. Date fields use YYYY-MM-DD; names and tags must be nonempty and at most 255 characters"))
}
fn visible(w: &Workspace, o: &OrganizationObject) -> bool {
    let q = w.query.text.to_lowercase();
    if let Some(tag) = q.strip_prefix("tag:") {
        return o
            .tags
            .as_ref()
            .is_some_and(|v| v.iter().any(|t| t.to_lowercase() == tag));
    }
    if let Some(folder) = q.strip_prefix("folder:") {
        return o
            .folder
            .as_ref()
            .is_some_and(|v| in_folder(&v.to_lowercase(), folder));
    }
    if let Some(kind) = q.strip_prefix("kind:") {
        return o.id.kind().to_lowercase().contains(kind);
    }
    o.name.to_lowercase().contains(&q)
        || o.id.uuid().to_string().contains(&q)
        || o.folder
            .as_ref()
            .is_some_and(|v| v.to_lowercase().contains(&q))
        || o.tags
            .as_ref()
            .is_some_and(|v| v.iter().any(|t| t.to_lowercase().contains(&q)))
        || o.custom_fields
            .iter()
            .any(|(k, v)| k.to_lowercase().contains(&q) || v.to_lowercase().contains(&q))
}
fn submit(s: &mut State) {
    let result = (|| {
        let w = s.organization.as_ref().ok_or(xenadmin_core::Error::Stale)?;
        if !current(s, w) {
            return Err(xenadmin_core::Error::Stale);
        }
        let permission = session_permissions_view::assess(
            s,
            &w.review
                .as_ref()
                .ok_or(xenadmin_core::Error::Stale)?
                .permission_requirements(),
        );
        if permission.decision().is_denied() {
            return Err(xenadmin_core::Error::Api("RBAC_PERMISSION_DENIED".into()));
        }
        s.connection
            .as_ref()
            .ok_or(xenadmin_core::Error::Closed)?
            .submit_resource(
                xenadmin_core::OperationId::new(),
                ResourceCommand::Organization {
                    pool: w.observed.ok_or(xenadmin_core::Error::Stale)?,
                    patch: w.review.clone().ok_or(xenadmin_core::Error::Stale)?,
                },
            )
    })();
    match result {
        Ok(operation) => {
            if let Some(waker) = &s.waker {
                operation.notify_on_change(waker.clone());
            }
            s.resource_operations.push(operation);
            s.organization = None;
            s.error = None;
        }
        Err(e) => edit(s).error = Some(e.to_string()),
    }
}
fn input(w: &Workspace, value: bool, label: &str, k: &Kit) -> El {
    col().gap(4.0).child(caption(label, k)).child(
        TextInput::new(if value { &w.value } else { &w.name })
            .label(label)
            .on_edit(move |s: &mut State| {
                if value {
                    &mut edit(s).value
                } else {
                    &mut edit(s).name
                }
            })
            .build(k),
    )
}
fn summary(change: &Change) -> String {
    match change {
        Change::Tags { previous, value } => {
            format!("Tags: {} → {}", previous.join(", "), value.join(", "))
        }
        Change::Folder { previous, value } => format!(
            "Folder: {} → {}",
            previous.as_deref().unwrap_or("none"),
            value.as_deref().unwrap_or("none")
        ),
        Change::EmptyFolders { previous, value } => format!(
            "Empty folders: {} → {}",
            previous.join(", "),
            value.join(", ")
        ),
        Change::Definitions { previous, value } => format!(
            "Definitions: {} → {}",
            previous
                .iter()
                .map(|d| d.name.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            value
                .iter()
                .map(|d| d.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Change::CustomField {
            definition,
            previous,
            value,
        } => format!(
            "{}: {} → {}",
            definition.name,
            previous.as_deref().unwrap_or("unset"),
            value.as_deref().unwrap_or("unset")
        ),
    }
}
pub(super) fn dialog(s: &State, k: &Kit) -> El {
    let Some(w) = &s.organization else {
        return el();
    };
    let permission = w
        .review
        .as_ref()
        .map(|op| session_permissions_view::assess(s, &op.permission_requirements()));
    let denied = permission
        .as_ref()
        .is_some_and(|p| p.decision().is_denied());
    let mut body = col()
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        })
        .padding(18.0)
        .gap(10.0)
        .child(heading(
            if w.review.is_some() {
                "Review organization changes"
            } else {
                "Tags, folders and custom fields"
            },
            k,
        ));
    if let Some(patch) = &w.review {
        body=body.child(caption(format!("{} changes in pool {}",patch.edits().len(),w.pool),k).wrap_anywhere())
            .child(caption("Only the listed objects will change. Changes apply separately; a failure stops later steps and keeps completed changes. Deleting a folder removes its grouping, not its VMs or data.",k).wrap());
        for e in patch.edits() {
            let name = w
                .inventory
                .objects
                .iter()
                .find(|o| o.id == e.object)
                .map(|o| o.name.as_str())
                .unwrap_or("Pool metadata");
            body = body.child(
                col()
                    .gap(4.0)
                    .child(heading(name, k))
                    .child(
                        caption(format!("{} · {}", e.object.kind(), e.object.uuid()), k)
                            .wrap_anywhere(),
                    )
                    .child(caption(summary(&e.change), k).wrap_anywhere()),
            );
        }
    } else {
        body = body.child(
            Button::new("vApps · ordered VM groups")
                .on_press(appliance_view::open)
                .build(k),
        );
        body=body.child(caption("Filter by name, UUID or metadata. Use tag:production, folder:/Services or kind:host to group objects. Reviews use this captured inventory; refresh to include later changes.",k).wrap())
            .child(TextInput::new(&w.query).label("Filter organization objects").placeholder("Search or tag:… / folder:… / kind:…").on_edit(|s:&mut State|&mut edit(s).query).build(k));
        let mut groups = row().gap(4.0).flex_wrap();
        let mut tags = BTreeSet::new();
        let mut folders = BTreeSet::new();
        for o in &w.inventory.objects {
            if let Some(t) = &o.tags {
                tags.extend(t.iter().cloned());
            }
            if let Some(f) = &o.folder {
                folders.insert(f.clone());
            }
        }
        if let Some(f) = &w.inventory.empty_folders {
            folders.extend(f.iter().cloned());
        }
        for (prefix, values) in [("tag", tags), ("folder", folders)] {
            for value in values.into_iter().take(24) {
                let query = format!("{prefix}:{value}");
                groups = groups.child(
                    Button::new(&query)
                        .variant(ButtonVariant::Ghost)
                        .on_press(move |s: &mut State| edit(s).query = TextEdit::new(&query))
                        .build(k),
                );
            }
        }
        body = body.child(groups);
        body = body.child(
            row()
                .gap(8.0)
                .flex_wrap()
                .child(
                    Button::new("Select filtered")
                        .on_press(|s: &mut State| {
                            let w = edit(s);
                            let ids = w
                                .inventory
                                .objects
                                .iter()
                                .filter(|o| visible(w, o))
                                .map(|o| o.id)
                                .collect::<BTreeSet<_>>();
                            if ids.len() > 256 {
                                w.error = Some("Narrow the filter to at most 256 objects".into());
                            } else {
                                w.selected = ids;
                                w.error = None;
                            }
                        })
                        .build(k),
                )
                .child(
                    Button::new("Clear selection")
                        .variant(ButtonVariant::Ghost)
                        .on_press(|s: &mut State| edit(s).selected.clear())
                        .build(k),
                )
                .child(caption(format!("{} selected", w.selected.len()), k)),
        );
        let mut objects = col()
            .w_full()
            .h(180.0)
            .scroll(rune::ScrollSpec {
                stick_bottom: false,
            })
            .gap(5.0);
        let count = w.inventory.objects.iter().filter(|o| visible(w, o)).count();
        for o in w
            .inventory
            .objects
            .iter()
            .filter(|o| visible(w, o))
            .take(100)
        {
            let id = o.id;
            objects = objects.child(
                Checkbox::new(w.selected.contains(&id))
                    .label(format!("{} · {}", o.name, o.id.kind()))
                    .on_toggle(move |s: &mut State| {
                        let w = edit(s);
                        if !w.selected.remove(&id) && w.selected.len() < 256 {
                            w.selected.insert(id);
                        }
                    })
                    .build(k),
            );
        }
        body=body.child(objects).child(caption(format!("{count} matches; showing up to 100. Select filtered includes every match within the 256-object limit."),k).wrap());
        for actions in [
            vec![
                Action::AddTag,
                Action::RemoveTag,
                Action::RenameTag,
                Action::DeleteTag,
            ],
            vec![
                Action::MoveFolder,
                Action::CreateFolder,
                Action::RenameFolder,
                Action::DeleteFolder,
            ],
            vec![
                Action::SetField,
                Action::RemoveField,
                Action::DefineField,
                Action::DeleteField,
            ],
        ] {
            let mut choices = row().gap(4.0).flex_wrap();
            for action in actions {
                choices = choices.child(
                    Button::new(action.title())
                        .variant(if action == w.action {
                            ButtonVariant::Outline
                        } else {
                            ButtonVariant::Ghost
                        })
                        .on_press(move |s: &mut State| {
                            let w = edit(s);
                            w.action = action;
                            w.error = None;
                        })
                        .build(k),
                );
            }
            body = body.child(choices);
        }
        body = body.child(heading(w.action.title(), k));
        let label = match w.action {
            Action::MoveFolder => "Folder path (empty removes grouping)",
            Action::CreateFolder | Action::RenameFolder | Action::DeleteFolder => "Folder path",
            Action::DefineField | Action::DeleteField | Action::SetField | Action::RemoveField => {
                "Custom field name"
            }
            _ => "Tag",
        };
        body = body.child(input(w, false, label, k));
        if matches!(
            w.action,
            Action::RenameTag | Action::RenameFolder | Action::SetField
        ) {
            body = body.child(input(
                w,
                true,
                match w.action {
                    Action::RenameTag => "New tag",
                    Action::RenameFolder => "New full folder path",
                    _ => "Value (dates use YYYY-MM-DD)",
                },
                k,
            ));
        }
        if w.action == Action::DefineField {
            body = body.child(
                Checkbox::new(w.date)
                    .label("Date field (otherwise text)")
                    .on_toggle(|s: &mut State| {
                        let w = edit(s);
                        w.date = !w.date;
                    })
                    .build(k),
            );
        }
        if matches!(
            w.action,
            Action::SetField | Action::RemoveField | Action::DeleteField
        ) {
            let mut fields = row().gap(4.0).flex_wrap();
            for d in w.inventory.definitions.iter().flatten() {
                let name = d.name.clone();
                fields = fields.child(
                    Button::new(&name)
                        .variant(ButtonVariant::Ghost)
                        .on_press(move |s: &mut State| edit(s).name = TextEdit::new(&name))
                        .build(k),
                );
            }
            body = body.child(fields);
        }
    }
    if let Some(error) = &w.error {
        body = body.child(
            caption(error, k)
                .color(k.c.danger.icon)
                .wrap()
                .role("alert"),
        );
    }
    if s.demo {
        body = body.child(caption("Synthetic preview; submission is disabled.", k).wrap());
    } else if !current(s, w) {
        body=body.child(caption("A current writable connection is required to apply changes. Refresh after reconnecting.",k).wrap());
    }
    let mut controls = row().gap(6.0).flex_wrap().justify_end().child(
        Button::new("Close")
            .variant(ButtonVariant::Ghost)
            .on_press(|s: &mut State| s.organization = None)
            .build(k),
    );
    if w.review.is_some() {
        controls = controls
            .child(
                Button::new("Back")
                    .on_press(|s: &mut State| edit(s).review = None)
                    .build(k),
            )
            .child(
                Button::new("Apply changes")
                    .disabled(!current(s, w) || denied)
                    .on_press(submit)
                    .build(k)
                    .key("organization-apply"),
            );
    } else {
        controls = controls
            .child(Button::new("Refresh").on_press(open).build(k))
            .child(
                Button::new("Review changes")
                    .on_press(|s: &mut State| {
                        let w = edit(s);
                        match plan(w) {
                            Ok(p) => {
                                w.review = Some(p);
                                w.error = None;
                            }
                            Err(e) => w.error = Some(e),
                        }
                    })
                    .build(k)
                    .key("organization-review"),
            );
    }
    el().w_full()
        .h_full()
        .absolute(0.0, 0.0)
        .center()
        .fill(rune::Color::BLACK.alpha(0.5))
        .modal(false, |s: &mut State| s.organization = None)
        .role("dialog")
        .label("Organization")
        .child(
            col()
                .w((s.window_width - 32.0).clamp(280.0, 1000.0))
                .h((s.window_height - 40.0).max(280.0))
                .radius(10.0)
                .fill(k.c.bg_base)
                .child(body)
                .child(
                    session_permissions_view::footer(
                        controls,
                        permission.as_ref(),
                        (s.window_width - 32.0).clamp(280.0, 1000.0) - 28.0,
                        k,
                    )
                    .padding(14.0),
                ),
        )
}
pub(super) fn scene(s: &mut State) {
    demo(s);
    let mut inventory = s.inventory.as_ref().unwrap().as_ref().clone();
    inventory.details.organization = OrganizationInventory {
        complete: true,
        objects: inventory
            .vms
            .values()
            .enumerate()
            .map(|(i, vm)| OrganizationObject {
                id: Id::Vm(vm.id),
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
    s.inventory = Some(Arc::new(inventory));
    open(s);
}

pub(super) fn review_scene(s: &mut State) {
    scene(s);
    let w = edit(s);
    w.selected.insert(w.inventory.objects[0].id);
    w.name = TextEdit::new("reviewed-tag");
    w.review = Some(plan(w).unwrap());
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn folder_rename_updates_descendants_and_registry_without_touching_other_paths() {
        let mut s = init();
        scene(&mut s);
        let w = edit(&mut s);
        w.action = Action::RenameFolder;
        w.name = TextEdit::new("/Services");
        w.value = TextEdit::new("/Production");
        w.inventory.objects[0].folder = Some("/Services/Databases".into());
        w.inventory.objects[1].folder = Some("/Services-old".into());
        w.inventory.empty_folders = Some(vec!["/Services/Empty".into(), "/Archive".into()]);
        let patch = plan(w).unwrap();
        assert!(patch.edits().iter().any(
            |e| matches!(&e.change,Change::Folder{value:Some(v),..} if v=="/Production/Databases")
        ));
        assert!(
            !patch
                .edits()
                .iter()
                .any(|e| e.object == w.inventory.objects[1].id)
        );
        assert!(patch.edits().iter().any(|e|matches!(&e.change,Change::EmptyFolders{value,..} if value==&vec!["/Archive".to_string(),"/Production/Empty".to_string()])));
    }
    #[test]
    fn custom_field_deletion_reviews_values_before_removing_definition() {
        let mut s = init();
        scene(&mut s);
        let w = edit(&mut s);
        w.inventory.objects[0]
            .custom_fields
            .insert("Owner".into(), "Fixture operator".into());
        w.action = Action::DeleteField;
        w.name = TextEdit::new("Owner");
        let patch = plan(w).unwrap();
        assert!(matches!(
            &patch.edits()[0].change,
            Change::CustomField { value: None, .. }
        ));
        assert!(
            matches!(&patch.edits().last().unwrap().change,Change::Definitions{value,..} if value.is_empty())
        );
        w.inventory.complete = false;
        assert!(plan(w).is_err());
    }
    #[test]
    fn exact_tag_and_folder_filters_are_distinct_from_name_search() {
        let mut s = init();
        scene(&mut s);
        let w = edit(&mut s);
        let mut object = w.inventory.objects[0].clone();
        object.tags = Some(vec!["production".into()]);
        object.folder = Some("/Service/Db".into());
        w.query = TextEdit::new("tag:prod");
        assert!(!visible(w, &object));
        w.query = TextEdit::new("tag:production");
        assert!(visible(w, &object));
        w.query = TextEdit::new("folder:/Service");
        assert!(visible(w, &object));
        w.query = TextEdit::new("folder:/Services");
        assert!(!visible(w, &object));
    }
    #[test]
    fn compact_review_is_reachable_and_cannot_submit_demo_metadata() {
        let mut s = init();
        scene(&mut s);
        let w = edit(&mut s);
        w.selected.insert(w.inventory.objects[0].id);
        w.name = TextEdit::new("reviewed-tag");
        let mut runtime = rune::Runtime::new(
            s,
            super::super::view,
            (513.0, 652.0),
            rune::Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        for _ in 0..30 {
            runtime.frame(1.0 / 60.0);
        }
        let (x, y) = runtime.find("organization-review").unwrap();
        assert!(x > 0.0 && x < 513.0 && y > 0.0 && y < 652.0);
        runtime.pointer_down(x, y);
        runtime.pointer_up(x, y);
        runtime.frame(0.0);
        assert!(
            runtime
                .state
                .organization
                .as_ref()
                .unwrap()
                .review
                .is_some()
        );
        for _ in 0..30 {
            runtime.frame(1.0 / 60.0);
        }
        let (x, y) = runtime.find("organization-apply").unwrap();
        runtime.pointer_down(x, y);
        runtime.pointer_up(x, y);
        runtime.frame(0.0);
        assert!(runtime.state.resource_operations.is_empty());
        assert!(runtime.state.organization.is_some());
    }
}
