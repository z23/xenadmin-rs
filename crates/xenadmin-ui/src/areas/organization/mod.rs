//! Area `organization`: the Organization navigation view, tags, folders and
//! custom fields, and vApps.
//!
//! Rune modules: `organization_view.rs` (the "Organization" workspace behind
//! the "Organize" button) and `appliance_view.rs` (the "vApps" workspace,
//! reached from a button of the first).
//!
//! What became of them:
//!
//! | Rune | Here |
//! | --- | --- |
//! | "Organize" button (`organization-open`) | no successor: a hub. Its parts are the rows below |
//! | quick-filter buttons `tag:x`, `folder:/p` | the rows of the Organization navigation view ([`nav_rows`]): "Folders" (a tree by path, with the empty folders), "Tags", "vApps". Selecting one shows its objects in the Search tab (area `search`) |
//! | filter input, check list, "Select filtered", "Clear selection" | no successor: the Search tab and its multi-selection choose the objects; a command takes them as its target (at most 256, the Rune limit) |
//! | actions "Add tag to selection", "Remove tag from selection", "Move selection to folder", "Set custom field on selection", "Clear custom field on selection" | commands `org.tag.add`, `org.tag.remove`, `org.folder.move`, `org.field.set`, `org.field.clear` in the context menu of every object that carries metadata (several objects too); each opens the dialog `organization` with that one action |
//! | actions "Rename tag across pool", "Delete tag across pool" | commands `org.tag.rename`, `org.tag.delete` in the context menu of a tag |
//! | actions "Create folder", "Rename folder across pool", "Delete folder across pool" | commands `org.folder.create` (context menu of a folder and of a pool), `org.folder.rename`, `org.folder.delete` (of a folder) |
//! | the four custom field actions on one object | the page `custom-fields` of every Properties dialog (VM, template, server, pool, repository, network, disk): the object's values, the action, its draft, the review and Apply on the page |
//! | review of an `OrganizationPatch`, Back, "Apply changes" | the same review in the dialog and on the page |
//! | "vApps · ordered VM groups" button | command `pool.vapps` (Pool > vApps; context menu of a pool and of a vApp) |
//! | vApps workspace | the dialog `appliances`: a list of the vApps and the editor of the chosen one, with the Rune reviews |
//! | choosing a VM in the vApps workspace | the same, and command `vm.assign-to-vapp` (VM > Assign to vApp) opens the dialog with the VM chosen |
//! | "Start vApp", "Shut down vApp", "Force shutdown" | buttons of the dialog, and commands `vapp.start`, `vapp.shutdown`, `vapp.force-shutdown` in the context menu of a vApp, which open the dialog on that review |
//! | jump to the pool history after a vApp change | no successor: the operation is followed in the Events tab |
//!
//! Folder and tag editing are commands with a dialog of their own, not part
//! of the Custom Fields page: they act on a selection or on the whole pool,
//! as `organization_view.rs` grouped them, and the page is about one object.
//!
//! Both surfaces and the page carry the experimental notice (Rune
//! `experimental_dialog_open` lists both workspaces).
//!
//! Commands (all of this area):
//!
//! | Id | Label | Where |
//! | --- | --- | --- |
//! | `org.tag.add` | Add tag to selection | context: pool, server, VM, template, repository, network, disk, snapshot; multi |
//! | `org.tag.remove` | Remove tag from selection | same |
//! | `org.folder.move` | Move selection to folder | same |
//! | `org.field.set` | Set custom field on selection | same |
//! | `org.field.clear` | Clear custom field on selection | same |
//! | `org.tag.rename` | Rename tag across pool | context: tag |
//! | `org.tag.delete` | Delete tag across pool | context: tag |
//! | `org.folder.create` | Create folder | context: folder, pool |
//! | `org.folder.rename` | Rename folder across pool | context: folder |
//! | `org.folder.delete` | Delete folder across pool | context: folder |
//! | `pool.vapps` | vApps | Pool > vApps; context: pool, vApp |
//! | `vm.assign-to-vapp` | Assign to vApp | VM > Assign to vApp; context: VM |
//! | `vapp.start` | Start vApp | context: vApp |
//! | `vapp.shutdown` | Shut down vApp | context: vApp |
//! | `vapp.force-shutdown` | Force shutdown | context: vApp |
//!
//! Surfaces: `organization` (dialog, experimental), `appliances` (dialog,
//! large, experimental), page `custom-fields` (experimental).
//!
//! Scenes: `organization`, `organization-review`, `appliances`,
//! `appliance-review`.
//!
//! What other areas may use: [`pools`], [`folder_object`], [`tag_object`],
//! [`edit::object_id`], the command ids in [`commands`], and
//! [`scenes::metadata`] (the demo pool with the organization metadata of
//! the Rune scene).

use std::collections::BTreeSet;
use std::sync::Arc;

use xenadmin_core::{Inventory, PoolId};

use crate::prelude::*;

pub mod appliance;
pub mod commands;
pub mod edit;
pub mod scenes;
#[cfg(test)]
mod tests;
mod view;

/// The state of the area. `m.areas.organization`.
#[derive(Default)]
pub struct State {
    /// The open organization change: the dialog `organization`, or the
    /// Custom Fields page of the open Properties dialog.
    pub editor: Option<edit::Workspace>,
    /// The open vApps dialog.
    pub appliances: Option<appliance::Workspace>,
}

pub fn spec() -> AreaSpec {
    AreaSpec {
        commands: commands::commands,
        scenes: scenes::scenes,
        surfaces: view::surfaces,
        on_scope_change: close,
        on_session_change: close,
        nav_rows,
        property_pages: view::pages,
        property_page: view::page,
        on_property_page: edit::page_event,
        ..AreaSpec::new("organization")
    }
}

/// Closes whatever the area has open. A draft of the Custom Fields page
/// does not outlive its Properties dialog, nor the reverse.
pub(crate) fn close(m: &mut Model) {
    let page = m
        .areas
        .organization
        .editor
        .take()
        .is_some_and(|w| w.page);
    m.areas.organization.appliances = None;
    if page {
        m.close_properties();
    }
}

/// The pools the tree shows, each once, with the inventory of the session
/// through which it is shown (the rule of the kernel's navigation views).
pub fn pools(m: &Model) -> Vec<(PoolId, Arc<Inventory>)> {
    crate::session::sessions(m)
        .into_iter()
        .filter_map(|view| {
            let inventory = view.inventory.clone()?;
            (crate::session::session_for_pool(m, inventory.pool_id) == Some(view.profile))
                .then(|| (inventory.pool_id, inventory))
        })
        .collect()
}

/// A folder as an object of the tree: its full path is its id. A folder of
/// the same path in two pools is one row.
pub fn folder_object(path: &str) -> ObjectRef {
    ObjectRef::new(None, ObjectKind::Folder, path)
}

/// A tag as an object of the tree: the tag is its id.
pub fn tag_object(tag: &str) -> ObjectRef {
    ObjectRef::new(None, ObjectKind::Tag, tag)
}

pub const FOLDERS_ROW: &str = "organization:folders";
pub const TAGS_ROW: &str = "organization:tags";
pub const VAPPS_ROW: &str = "organization:vapps";

fn folder_row_id(path: &str) -> String {
    format!("organization:folder:{path}")
}

/// The Organization navigation view: what the Rune workspace grouped by.
/// "Folders" holds every folder of every shown pool as a tree by path (the
/// folders objects are in, their ancestors, and the registered empty
/// folders); "Tags" every tag; "vApps" every vApp by name. A heading is
/// there only while it has rows under it, so a pool without any of the
/// three shows an empty view.
fn nav_rows(m: &Model, view: NavView) -> Vec<NavRow> {
    if view != NavView::Organization {
        return Vec::new();
    }
    let pools = pools(m);
    let mut folders: BTreeSet<String> = BTreeSet::new();
    let mut tags: BTreeSet<String> = BTreeSet::new();
    let mut vapps: Vec<(PoolId, String, String)> = Vec::new();
    for (pool, inventory) in &pools {
        let organization = &inventory.details.organization;
        let paths = organization
            .objects
            .iter()
            .filter_map(|o| o.folder.as_deref())
            .chain(organization.empty_folders.iter().flatten().map(String::as_str));
        for path in paths {
            // The folder and every ancestor: "/a/b" gives "/a" and "/a/b".
            for (at, _) in path.match_indices('/').skip(1) {
                folders.insert(path[..at].to_owned());
            }
            if path.starts_with('/') && path.len() > 1 {
                folders.insert(path.to_owned());
            }
        }
        for object in &organization.objects {
            tags.extend(object.tags.iter().flatten().cloned());
        }
        for group in inventory.details.appliances.values() {
            vapps.push((*pool, group.id.to_string(), group.name.clone()));
        }
    }
    let mut rows = Vec::new();
    if !folders.is_empty() {
        rows.push(NavRow::new(FOLDERS_ROW, "Folders", ObjectKind::Folder).expanded());
        // Parents come before their children: a path sorts after its prefix.
        let mut folders: Vec<String> = folders.into_iter().collect();
        folders.sort_by(|a, b| {
            let (mut a, mut b) = (a.split('/'), b.split('/'));
            loop {
                match (a.next(), b.next()) {
                    (Some(x), Some(y)) => match natural_cmp(x, y) {
                        std::cmp::Ordering::Equal => {}
                        other => break other,
                    },
                    (None, Some(_)) => break std::cmp::Ordering::Less,
                    (Some(_), None) => break std::cmp::Ordering::Greater,
                    (None, None) => break std::cmp::Ordering::Equal,
                }
            }
        });
        for path in folders {
            let (parent, name) = path.rsplit_once('/').unwrap_or(("", &path));
            let parent = if parent.is_empty() {
                FOLDERS_ROW.to_owned()
            } else {
                folder_row_id(parent)
            };
            rows.push(
                NavRow::new(folder_row_id(&path), name, ObjectKind::Folder)
                    .parent(parent)
                    .object(folder_object(&path))
                    .expanded(),
            );
        }
    }
    if !tags.is_empty() {
        rows.push(NavRow::new(TAGS_ROW, "Tags", ObjectKind::Tag).expanded());
        let mut tags: Vec<String> = tags.into_iter().collect();
        tags.sort_by(|a, b| natural_cmp(a, b));
        for tag in tags {
            rows.push(
                NavRow::new(format!("organization:tag:{tag}"), tag.clone(), ObjectKind::Tag)
                    .parent(TAGS_ROW)
                    .object(tag_object(&tag)),
            );
        }
    }
    if !vapps.is_empty() {
        rows.push(NavRow::new(VAPPS_ROW, "vApps", ObjectKind::Appliance).expanded());
        vapps.sort_by(|a, b| natural_cmp(&a.2, &b.2).then_with(|| a.1.cmp(&b.1)));
        for (pool, id, name) in vapps {
            rows.push(
                NavRow::new(
                    format!("organization:vapp:{pool}:{id}"),
                    name,
                    ObjectKind::Appliance,
                )
                .parent(VAPPS_ROW)
                .object(ObjectRef::new(Some(pool), ObjectKind::Appliance, id)),
            );
        }
    }
    rows
}
