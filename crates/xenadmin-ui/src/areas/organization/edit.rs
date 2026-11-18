//! Tags, folders and custom fields: the logic of the Rune module
//! `organization_view.rs`, carried over.
//!
//! The Rune workspace was one dialog with a filter, a check list of up to
//! 256 objects and twelve actions. The twelve actions, the plan of each
//! ([`plan`]), the review, the staleness check and the submission are the
//! same here. What changed is where the selection and the action come from:
//! a command gives both (the selection is its target), or the Custom Fields
//! page of a Properties dialog does (one object, the four field actions).
//! The filter and the check list have no successor: the Search tab and its
//! multi-selection are the way to choose objects now.

use std::collections::BTreeSet;

use xenadmin_core::{
    CustomFieldDefinition, CustomFieldType, Error, ObservedResource, OperationId,
    OrganizationChange as Change, OrganizationEdit as Edit, OrganizationInventory,
    OrganizationObject, OrganizationObjectId as Id, OrganizationPatch, PoolId, ResourceCommand,
    ResourceTarget,
};

use crate::prelude::*;

/// The id of the Custom Fields page in every Properties dialog.
pub const PAGE: &str = "custom-fields";

/// The Rune limit of one selection and of one review.
pub const LIMIT: usize = 256;

/// The Rune text of "Select filtered" over more than 256 matches; here the
/// reason a command is disabled for a larger selection.
pub const TOO_MANY: &str = "Narrow the filter to at most 256 objects";

/// The Rune refusal of a pool-wide action over incomplete metadata.
pub const INCOMPLETE: &str =
    "Refresh with permission to read all organization metadata before a pool-wide change";

pub const NO_REGISTRY: &str = "Empty-folder registry is unavailable";
pub const NO_DEFINITIONS: &str = "Custom field definitions are unavailable";

/// New text: a field command over a pool that defines no custom field.
/// Rune answered "Choose an existing custom field" to whatever was typed.
pub const NO_FIELDS: &str = "No custom field is defined in this pool. Define one on the Custom Fields page of a Properties dialog.";

/// The Rune caption under a review or draft that cannot be applied; here
/// also the reason of the disabled Apply button.
pub const NOT_CURRENT: &str =
    "A current writable connection is required to apply changes. Refresh after reconnecting.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
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
    /// The four actions of the Custom Fields page, in the Rune order.
    pub const FIELDS: [Action; 4] = [
        Action::SetField,
        Action::RemoveField,
        Action::DefineField,
        Action::DeleteField,
    ];

    pub fn title(self) -> &'static str {
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

    /// A stable word for the action: the option id on the page.
    pub fn id(self) -> &'static str {
        match self {
            Self::AddTag => "add-tag",
            Self::RemoveTag => "remove-tag",
            Self::RenameTag => "rename-tag",
            Self::DeleteTag => "delete-tag",
            Self::MoveFolder => "move-folder",
            Self::CreateFolder => "create-folder",
            Self::RenameFolder => "rename-folder",
            Self::DeleteFolder => "delete-folder",
            Self::SetField => "set-field",
            Self::RemoveField => "clear-field",
            Self::DefineField => "define-field",
            Self::DeleteField => "delete-field",
        }
    }

    /// The action changes the objects of the selection, not the pool.
    pub fn on_selection(self) -> bool {
        matches!(
            self,
            Self::AddTag | Self::RemoveTag | Self::MoveFolder | Self::SetField | Self::RemoveField
        )
    }

    /// The Rune `global`: the action reads every object of the pool.
    pub fn pool_wide(self) -> bool {
        matches!(
            self,
            Self::RenameTag
                | Self::DeleteTag
                | Self::RenameFolder
                | Self::DeleteFolder
                | Self::DeleteField
        )
    }
}

/// The open organization change: a dialog of one action, or the Custom
/// Fields page of a Properties dialog (`page`).
pub struct Workspace {
    pub pool: PoolId,
    pub observed: Option<ObservedResource>,
    pub inventory: OrganizationInventory,
    /// The pools a tag or a folder is found in, when its command was run on
    /// a row of the Organization view (which lists every connected pool as
    /// one). With more than one the dialog asks which pool to change.
    pub pools: Vec<(PoolId, String)>,
    /// What the command was invoked on, for Refresh.
    pub target: Target,
    /// The Custom Fields page of the Properties dialog of `target`.
    pub page: bool,
    pub selected: BTreeSet<Id>,
    /// Names of objects of the target that the captured metadata does not
    /// list. Any of them refuses the review: a selection is never narrowed
    /// silently.
    pub missing: Vec<String>,
    pub action: Action,
    pub name: String,
    pub value: String,
    pub date: bool,
    pub review: Option<OrganizationPatch>,
    pub error: Option<String>,
}

/// The organization identity of a tree object. `None` for a kind that
/// carries no tags, folder or custom fields (a vApp, a folder, a tag).
pub fn object_id(object: &ObjectRef) -> Option<Id> {
    let uuid = object.uuid()?;
    Some(match object.kind {
        ObjectKind::Pool => Id::Pool(uuid.into()),
        ObjectKind::Host => Id::Host(uuid.into()),
        ObjectKind::Vm | ObjectKind::Template => Id::Vm(uuid.into()),
        ObjectKind::Snapshot => Id::Snapshot(uuid.into()),
        ObjectKind::StorageRepository => Id::Storage(uuid.into()),
        ObjectKind::Network => Id::Network(uuid.into()),
        ObjectKind::VirtualDisk => Id::Disk(uuid.into()),
        _ => return None,
    })
}

pub fn in_folder(folder: &str, path: &str) -> bool {
    folder == path
        || folder
            .strip_prefix(path)
            .is_some_and(|s| s.starts_with('/'))
}

/// Whether the metadata of a pool has this folder: an object in it or
/// under it, or an empty folder registered at or under it.
pub fn has_folder(inventory: &OrganizationInventory, path: &str) -> bool {
    inventory
        .objects
        .iter()
        .any(|o| o.folder.as_deref().is_some_and(|f| in_folder(f, path)))
        || inventory
            .empty_folders
            .iter()
            .flatten()
            .any(|f| in_folder(f, path))
}

/// Whether an object of the pool carries exactly this tag.
pub fn has_tag(inventory: &OrganizationInventory, tag: &str) -> bool {
    inventory
        .objects
        .iter()
        .any(|o| o.tags.iter().flatten().any(|t| t == tag))
}

/// The pool an action on this target changes, the pools the user may
/// choose from instead, and what the name draft starts as.
fn place(
    m: &Model,
    target: &Target,
    action: Action,
    prefer: Option<PoolId>,
) -> Option<(PoolId, Vec<(PoolId, String)>, String)> {
    let shown = super::pools(m);
    let among = |holds: &dyn Fn(&OrganizationInventory) -> bool| {
        let pools: Vec<(PoolId, String)> = shown
            .iter()
            .filter(|(_, inventory)| holds(&inventory.details.organization))
            .map(|(pool, inventory)| (*pool, inventory.pool_name.clone()))
            .collect();
        let scope = m.inventory.as_ref().map(|inventory| inventory.pool_id);
        let pool = [prefer, scope]
            .into_iter()
            .flatten()
            .find(|pool| pools.iter().any(|(candidate, _)| candidate == pool))
            .or(pools.first().map(|(pool, _)| *pool))?;
        Some((pool, pools))
    };
    let object = target.single();
    match (action, object.map(|object| object.kind)) {
        (Action::RenameTag | Action::DeleteTag, Some(ObjectKind::Tag)) => {
            let tag = &object?.id;
            let (pool, pools) = among(&|inventory| has_tag(inventory, tag))?;
            Some((pool, pools, tag.clone()))
        }
        (Action::RenameFolder | Action::DeleteFolder, Some(ObjectKind::Folder)) => {
            let path = &object?.id;
            let (pool, pools) = among(&|inventory| has_folder(inventory, path))?;
            Some((pool, pools, path.clone()))
        }
        // A new folder beside or under the one that was pressed: any pool.
        (Action::CreateFolder, Some(ObjectKind::Folder)) => {
            let (pool, pools) = among(&|_| true)?;
            Some((pool, pools, format!("{}/", object?.id)))
        }
        (Action::CreateFolder, Some(ObjectKind::Pool)) => {
            Some((object?.pool?, Vec::new(), String::new()))
        }
        (action, _) if action.on_selection() || Action::FIELDS.contains(&action) => {
            if target.is_empty() || !target.objects.iter().all(|o| object_id(o).is_some()) {
                return None;
            }
            Some((target.pool()?, Vec::new(), String::new()))
        }
        _ => None,
    }
}

/// The change a command or the page would open for this target. `None`:
/// the action has nothing to do with the target, or its pool is not shown.
pub fn capture(m: &Model, target: &Target, action: Action, page: bool) -> Option<Workspace> {
    capture_in(m, target, action, page, None)
}

fn capture_in(
    m: &Model,
    target: &Target,
    action: Action,
    page: bool,
    prefer: Option<PoolId>,
) -> Option<Workspace> {
    let (pool, pools, name) = place(m, target, action, prefer)?;
    let inventory = m.inventory_for(pool)?;
    let observed = m
        .snapshot_for(pool)
        .and_then(|v| v.observe_resource(ResourceTarget::Pool(pool)).ok());
    let organization = inventory.details.organization.clone();
    let mut selected = BTreeSet::new();
    let mut missing = Vec::new();
    for object in &target.objects {
        let Some(id) = object_id(object) else {
            continue;
        };
        if organization.objects.iter().any(|o| o.id == id) {
            selected.insert(id);
        } else {
            missing.push(m.name_of(object).unwrap_or_else(|| object.id.clone()));
        }
    }
    Some(Workspace {
        pool,
        observed,
        inventory: organization,
        pools,
        target: target.clone(),
        page,
        selected,
        missing,
        action,
        name,
        value: String::new(),
        date: false,
        review: None,
        error: None,
    })
}

/// Opens the dialog of an action for a target.
pub fn open(m: &mut Model, target: &Target, action: Action) {
    if let Some(w) = capture(m, target, action, false) {
        super::close(m);
        m.areas.organization.editor = Some(w);
    }
}

/// The Rune Refresh: captures the pool again and drops the draft.
pub fn refresh(m: &mut Model) {
    let Some(w) = &m.areas.organization.editor else {
        return;
    };
    let (target, action, page, pool) = (w.target.clone(), w.action, w.page, w.pool);
    match capture_in(m, &target, action, page, Some(pool)) {
        Some(fresh) => m.areas.organization.editor = Some(fresh),
        // What it was opened on is gone.
        None => super::close(m),
    }
}

/// Another of the pools that have the tag or the folder. The draft stays.
pub fn set_pool(m: &mut Model, id: &str) {
    let Some(w) = &m.areas.organization.editor else {
        return;
    };
    let Some(pool) = w
        .pools
        .iter()
        .map(|(pool, _)| *pool)
        .find(|pool| pool.to_string() == id)
    else {
        return;
    };
    let (target, action, page) = (w.target.clone(), w.action, w.page);
    let Some(mut fresh) = capture_in(m, &target, action, page, Some(pool)) else {
        return;
    };
    if let Some(w) = &mut m.areas.organization.editor {
        fresh.name = std::mem::take(&mut w.name);
        fresh.value = std::mem::take(&mut w.value);
        fresh.date = w.date;
    }
    m.areas.organization.editor = Some(fresh);
}

/// Another action on the page. The name and the value stay, as in Rune.
pub fn select_action(m: &mut Model, action: Action) {
    if let Some(w) = &mut m.areas.organization.editor {
        w.action = action;
        w.error = None;
    }
}

/// The Properties dialog entered or left the Custom Fields page.
pub fn page_event(m: &mut Model, target: &Target, event: PageEvent) {
    match event {
        PageEvent::Entered(id) if id == PAGE => {
            m.areas.organization.appliances = None;
            m.areas.organization.editor = capture(m, target, Action::SetField, true);
        }
        PageEvent::Left(id) if id == PAGE => {
            let editor = &mut m.areas.organization.editor;
            if editor.as_ref().is_some_and(|w| w.page) {
                *editor = None;
            }
        }
        _ => {}
    }
}

pub fn synthetic(m: &Model, w: &Workspace) -> bool {
    m.is_synthetic_pool(w.pool)
}

pub fn current(m: &Model, w: &Workspace) -> bool {
    !synthetic(m, w)
        && m.can_mutate_pool(w.pool)
        && w.observed.is_some()
        && w.observed
            == m.snapshot_for(w.pool)
                .and_then(|v| v.observe_resource(ResourceTarget::Pool(w.pool)).ok())
}

/// Why a review cannot be applied as it stands. `None` in a synthetic
/// pool, which says so itself.
pub fn stale(m: &Model, w: &Workspace) -> Option<String> {
    (w.review.is_some() && !synthetic(m, w) && !current(m, w)).then(|| NOT_CURRENT.to_owned())
}

pub fn selected(w: &Workspace) -> impl Iterator<Item = &OrganizationObject> {
    w.inventory
        .objects
        .iter()
        .filter(|o| w.selected.contains(&o.id))
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

/// The Rune text for an object whose metadata the action needs and the
/// captured inventory does not have.
fn unavailable(action: Action, name: &str) -> String {
    match action {
        Action::AddTag | Action::RemoveTag => format!("Tags are unavailable for {name}"),
        Action::MoveFolder => format!("Folder metadata is unavailable for {name}"),
        _ => format!("Custom field metadata is unavailable for {name}"),
    }
}

/// Why no draft could make the review of this action pass: the reason its
/// command is disabled, and what the page says above its draft. Each text
/// is the one [`plan`] gives for the same condition.
pub fn blocked(w: &Workspace) -> Option<String> {
    if w.selected.len() + w.missing.len() > LIMIT {
        return Some(TOO_MANY.into());
    }
    let inventory = &w.inventory;
    if w.action.pool_wide()
        && (!inventory.complete
            || inventory
                .objects
                .iter()
                .any(|o| o.tags.is_none() || !o.other_config_known))
    {
        return Some(INCOMPLETE.into());
    }
    if w.action.on_selection() {
        if let Some(name) = w.missing.first() {
            return Some(unavailable(w.action, name));
        }
        let lacking = selected(w).find(|o| match w.action {
            Action::AddTag | Action::RemoveTag => o.tags.is_none(),
            _ => !o.other_config_known,
        });
        if let Some(o) = lacking {
            return Some(unavailable(w.action, &o.name));
        }
    }
    match w.action {
        Action::CreateFolder | Action::RenameFolder | Action::DeleteFolder => {
            inventory.empty_folders.is_none().then(|| NO_REGISTRY.into())
        }
        Action::DefineField | Action::DeleteField => {
            inventory.definitions.is_none().then(|| NO_DEFINITIONS.into())
        }
        Action::SetField | Action::RemoveField => match &inventory.definitions {
            None => Some(NO_DEFINITIONS.into()),
            // On the page the user can define one first.
            Some(definitions) if definitions.is_empty() && !w.page => Some(NO_FIELDS.into()),
            Some(_) => None,
        },
        _ => None,
    }
}

pub fn plan(w: &Workspace) -> Result<OrganizationPatch, String> {
    // New with the port: the selection is given, not picked from the
    // captured list, so it can name an object the metadata does not have.
    if w.action.on_selection()
        && let Some(name) = w.missing.first()
    {
        return Err(unavailable(w.action, name));
    }
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
        return Err(INCOMPLETE.into());
    }
    let mut edits = Vec::new();
    let name = w.name.trim();
    let value = w.value.trim();
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
            let previous = w.inventory.empty_folders.clone().ok_or(NO_REGISTRY)?;
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
            let previous = w.inventory.definitions.clone().ok_or(NO_DEFINITIONS)?;
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
                Some(w.value.clone())
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
    if edits.len() > LIMIT {
        return Err("This review exceeds 256 changes. Apply smaller selections first".into());
    }
    OrganizationPatch::new(edits).map_err(|e|format!("{e}. Date fields use YYYY-MM-DD; names and tags must be nonempty and at most 255 characters"))
}

/// The "Review changes" button.
pub fn review(m: &mut Model) {
    let Some(w) = &mut m.areas.organization.editor else {
        return;
    };
    match plan(w) {
        Ok(p) => {
            w.review = Some(p);
            w.error = None;
        }
        Err(e) => w.error = Some(e),
    }
}

/// Back from the review to the draft. The error line stays, as in Rune.
pub fn back(m: &mut Model) {
    if let Some(w) = &mut m.areas.organization.editor {
        w.review = None;
    }
}

pub fn submit(m: &mut Model) {
    let result = (|| {
        let w = m.areas.organization.editor.as_ref().ok_or(Error::Stale)?;
        if !current(m, w) {
            return Err(Error::Stale);
        }
        let patch = w.review.as_ref().ok_or(Error::Stale)?;
        let permission = permissions::assess_for(m, w.pool, &patch.permission_requirements());
        if permission.decision().is_denied() {
            return Err(Error::Api("RBAC_PERMISSION_DENIED".into()));
        }
        m.connection_for(w.pool)
            .ok_or(Error::Closed)?
            .submit_resource(
                OperationId::new(),
                ResourceCommand::Organization {
                    pool: w.observed.ok_or(Error::Stale)?,
                    patch: patch.clone(),
                },
            )
    })();
    match result {
        Ok(operation) => {
            m.track_resource(operation);
            super::close(m);
            m.error = None;
        }
        Err(e) => {
            if let Some(w) = &mut m.areas.organization.editor {
                w.error = Some(e.to_string());
            }
        }
    }
}

pub fn summary(change: &Change) -> String {
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

/// The two lines above the changes of a review.
pub fn review_lines(w: &Workspace, patch: &OrganizationPatch) -> Vec<String> {
    vec![
        format!("{} changes in pool {}", patch.edits().len(), w.pool),
        "Only the listed objects will change. Changes apply separately; a failure stops later steps and keeps completed changes. Deleting a folder removes its grouping, not its VMs or data.".into(),
    ]
}

/// One reviewed change as Rune listed it: the object's name (or "Pool
/// metadata"), "{kind} · {uuid}", and what changes.
pub fn change_lines(w: &Workspace, edit: &Edit) -> [String; 3] {
    let name = w
        .inventory
        .objects
        .iter()
        .find(|o| o.id == edit.object)
        .map(|o| o.name.as_str())
        .unwrap_or("Pool metadata");
    [
        name.to_owned(),
        format!("{} · {}", edit.object.kind(), edit.object.uuid()),
        summary(&edit.change),
    ]
}
