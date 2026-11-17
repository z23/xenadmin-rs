//! The navigation pane: its views and their tree rows.
//!
//! A view is a flat list of [`NavRow`]s, parents before children. Rows are
//! plain data with stable ids, for a tree widget that re-applies expansion
//! and selection by id after every rebuild. The widget owns the expansion
//! state; a row only says whether it starts expanded.
//!
//! The views (plan, "Shell regions"):
//!
//! - **Infrastructure**: every saved connection. One with an inventory is
//!   its pool, with its hosts and their resident VMs, then the VMs that are
//!   on no host, the templates and the storage repositories. One that is not
//!   connected, is connecting or has failed is a node with its status word.
//! - **Objects**: every object of every connected pool by type.
//! - **Organization**: rows from the `organization` area.
//! - **Saved Searches**: rows from the `search` area.
//! - **Notifications**: Alerts and Events. Its tabs are those two, not the
//!   tab matrix of the selection.
//!
//! [`rows`] is rebuilt on demand and costs time proportional to the
//! inventories. The shell calls it after an update that changed something,
//! not on every frame.

use serde::Serialize;
use xenadmin_core::{Inventory, PoolId, ProfileId};

use crate::format::natural_cmp;
use crate::model::Model;
use crate::object::{ObjectKind, ObjectRef};
use crate::session::{self, SessionView};
use crate::tabs::TabId;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum NavView {
    #[default]
    Infrastructure,
    Objects,
    Organization,
    SavedSearches,
    Notifications,
}

impl NavView {
    pub const ALL: [NavView; 5] = [
        NavView::Infrastructure,
        NavView::Objects,
        NavView::Organization,
        NavView::SavedSearches,
        NavView::Notifications,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Infrastructure => "Infrastructure",
            Self::Objects => "Objects",
            Self::Organization => "Organization",
            Self::SavedSearches => "Saved Searches",
            Self::Notifications => "Notifications",
        }
    }

    /// The key of the view's entry in the view switcher.
    pub fn key(self) -> &'static str {
        match self {
            Self::Infrastructure => "nav-infrastructure",
            Self::Objects => "nav-objects",
            Self::Organization => "nav-organization",
            Self::SavedSearches => "nav-saved-searches",
            Self::Notifications => "nav-notifications",
        }
    }

    /// The area whose `nav_rows` hook gives the rows of the view. The
    /// kernel builds the other views itself.
    pub fn provider(self) -> Option<&'static str> {
        match self {
            Self::Organization => Some("organization"),
            Self::SavedSearches => Some("search"),
            Self::Infrastructure | Self::Objects | Self::Notifications => None,
        }
    }
}

/// One row of a navigation view.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct NavRow {
    /// Stable across rebuilds and unique within the view.
    pub id: String,
    /// The id of the parent row; `None` for a root.
    pub parent: Option<String>,
    pub label: String,
    /// The kind of icon the row shows.
    pub icon: ObjectKind,
    /// What selecting the row selects. `None` for a heading that groups
    /// rows and for the two rows of the Notifications view.
    pub object: Option<ObjectRef>,
    /// The state word of the row: the status word of a connection
    /// ("Connecting…", "Connection failed", "Disconnected", "Demo"; nothing
    /// for a pool whose connection is current), the power state of a VM,
    /// "Disabled" for a disabled host.
    pub status: Option<String>,
    /// Whether the row starts expanded when the tree first shows it.
    pub expanded: bool,
}

impl NavRow {
    pub fn new(id: impl Into<String>, label: impl Into<String>, icon: ObjectKind) -> Self {
        Self {
            id: id.into(),
            parent: None,
            label: label.into(),
            icon,
            object: None,
            status: None,
            expanded: false,
        }
    }
    pub fn parent(mut self, parent: impl Into<String>) -> Self {
        self.parent = Some(parent.into());
        self
    }
    pub fn object(mut self, object: ObjectRef) -> Self {
        self.object = Some(object);
        self
    }
    pub fn status(mut self, status: impl Into<String>) -> Self {
        self.status = Some(status.into());
        self
    }
    pub fn expanded(mut self) -> Self {
        self.expanded = true;
        self
    }
}

/// The state of the navigation pane that is not selection.
#[derive(Clone, Debug)]
pub struct Navigation {
    pub view: NavView,
    /// The text of the search box. It narrows the rows of every view.
    pub filter: String,
    /// The tab of the Notifications view: Alerts or Events.
    pub notification_tab: TabId,
    /// View > Custom Templates.
    pub show_custom_templates: bool,
    /// View > Built-in Templates.
    pub show_builtin_templates: bool,
    /// View > Local Storage.
    pub show_local_storage: bool,
    /// View > Toolbar.
    pub show_toolbar: bool,
}

impl Default for Navigation {
    fn default() -> Self {
        Self {
            view: NavView::Infrastructure,
            filter: String::new(),
            notification_tab: TabId::Alerts,
            show_custom_templates: true,
            show_builtin_templates: false,
            show_local_storage: true,
            show_toolbar: true,
        }
    }
}

/// The id of the root row of a saved connection in the Infrastructure view.
/// It is the same whether the row is the pool or the connection.
pub fn session_row_id(profile: ProfileId) -> String {
    format!("session:{profile}")
}

/// The id of the row of an object under its session in the Infrastructure
/// view.
pub fn object_row_id(profile: ProfileId, object: &ObjectRef) -> String {
    format!("{profile}:{}:{}", kind_word(object.kind), object.id)
}

fn kind_word(kind: ObjectKind) -> &'static str {
    match kind {
        ObjectKind::None => "none",
        ObjectKind::Pool => "pool",
        ObjectKind::Host => "host",
        ObjectKind::Vm => "vm",
        ObjectKind::Template => "template",
        ObjectKind::StorageRepository => "sr",
        ObjectKind::Network => "network",
        ObjectKind::VirtualDisk => "disk",
        ObjectKind::Snapshot => "snapshot",
        ObjectKind::Appliance => "vapp",
        ObjectKind::Folder => "folder",
        ObjectKind::Tag => "tag",
        ObjectKind::SavedSearch => "search",
        ObjectKind::Connection => "connection",
        ObjectKind::Multi => "multi",
    }
}

/// The rows of the active view, narrowed by the filter.
pub fn rows(m: &Model) -> Vec<NavRow> {
    filter_rows(view_rows(m, m.nav.view), &m.nav.filter)
}

/// The rows of one view, not narrowed.
pub fn view_rows(m: &Model, view: NavView) -> Vec<NavRow> {
    match view {
        NavView::Infrastructure => infrastructure(m),
        NavView::Objects => objects(m),
        NavView::Notifications => notifications(),
        NavView::Organization | NavView::SavedSearches => view
            .provider()
            .and_then(crate::areas::area)
            .map(|area| (area.nav_rows)(m, view))
            .unwrap_or_default(),
    }
}

/// The ids of the rows that show the scope: what the tree marks as
/// selected.
pub fn selected_ids(m: &Model, rows: &[NavRow]) -> Vec<String> {
    if m.nav.view == NavView::Notifications {
        let id = match m.nav.notification_tab {
            TabId::Events => "notifications:events",
            _ => "notifications:alerts",
        };
        return vec![id.to_owned()];
    }
    rows.iter()
        .filter(|row| {
            row.object
                .as_ref()
                .is_some_and(|object| m.selection.scope.contains(object))
        })
        .map(|row| row.id.clone())
        .collect()
}

/// Keeps the rows a filter text matches, with their ancestors and their
/// descendants. A row matches when its label contains the text, ignoring
/// case, or its object's id (the UUID) contains it: the substring rule of
/// the Rune inventory search. Every kept row is expanded. An empty filter
/// keeps everything as it is.
pub fn filter_rows(rows: Vec<NavRow>, filter: &str) -> Vec<NavRow> {
    let query = filter.trim().to_lowercase();
    if query.is_empty() {
        return rows;
    }
    let index: std::collections::BTreeMap<&str, usize> = rows
        .iter()
        .enumerate()
        .map(|(position, row)| (row.id.as_str(), position))
        .collect();
    let matches: Vec<bool> = rows
        .iter()
        .map(|row| {
            row.label.to_lowercase().contains(&query)
                || row
                    .object
                    .as_ref()
                    .is_some_and(|object| object.id.to_lowercase().contains(&query))
        })
        .collect();
    let parents: Vec<Option<usize>> = rows
        .iter()
        .map(|row| row.parent.as_deref().and_then(|id| index.get(id)).copied())
        .collect();
    let mut keep = matches.clone();
    // Rows come parents first, so whether a parent is under a match is
    // already known when its child asks.
    let mut under_match = vec![false; rows.len()];
    for position in 0..rows.len() {
        if let Some(parent) = parents[position].filter(|parent| *parent < position) {
            under_match[position] = matches[parent] || under_match[parent];
        }
        if under_match[position] {
            keep[position] = true;
        }
        if matches[position] {
            let mut ancestor = parents[position];
            while let Some(at) = ancestor.filter(|at| *at < position) {
                keep[at] = true;
                ancestor = parents[at];
            }
        }
    }
    rows.into_iter()
        .zip(keep)
        .filter(|(_, keep)| *keep)
        .map(|(mut row, _)| {
            row.expanded = true;
            row
        })
        .collect()
}

fn by_name<T>(mut items: Vec<T>, name: impl Fn(&T) -> &str) -> Vec<T> {
    items.sort_by(|a, b| natural_cmp(name(a), name(b)));
    items
}

/// Whether a template is one the host ships. Unknown counts as custom.
fn builtin_template(inventory: &Inventory, vm: xenadmin_core::VmId) -> bool {
    inventory
        .details
        .vms
        .get(&vm)
        .and_then(|details| details.is_default_template)
        == Some(true)
}

fn shows_template(m: &Model, inventory: &Inventory, vm: xenadmin_core::VmId) -> bool {
    if builtin_template(inventory, vm) {
        m.nav.show_builtin_templates
    } else {
        m.nav.show_custom_templates
    }
}

/// Whether a storage repository is local to one host. Unknown counts as
/// shared.
fn local_storage(inventory: &Inventory, uuid: uuid::Uuid) -> bool {
    inventory
        .details
        .storage
        .get(&xenadmin_core::SrId::from(uuid))
        .and_then(|details| details.shared)
        == Some(false)
}

fn shows_storage(m: &Model, inventory: &Inventory, uuid: uuid::Uuid) -> bool {
    m.nav.show_local_storage || !local_storage(inventory, uuid)
}

/// The sessions whose inventory a view lists: one per pool, the session
/// through which that pool is shown.
fn pools(m: &Model) -> Vec<(SessionView, std::sync::Arc<Inventory>)> {
    session::sessions(m)
        .into_iter()
        .filter_map(|view| {
            let inventory = view.inventory.clone()?;
            (session::session_for_pool(m, inventory.pool_id) == Some(view.profile))
                .then_some((view, inventory))
        })
        .collect()
}

fn infrastructure(m: &Model) -> Vec<NavRow> {
    let mut rows = Vec::new();
    for view in session::sessions(m) {
        let root = session_row_id(view.profile);
        let shown = view.inventory.clone().filter(|inventory| {
            session::session_for_pool(m, inventory.pool_id) == Some(view.profile)
        });
        let Some(inventory) = shown else {
            // Not connected, connecting, failed, or a second saved
            // connection to a pool that is already shown.
            rows.push(
                NavRow::new(root, view.name.clone(), ObjectKind::Connection)
                    .object(ObjectRef::new(None, ObjectKind::Connection, view.profile))
                    .status(view.status),
            );
            continue;
        };
        let pool = inventory.pool_id;
        let current = view.synthetic
            || view.state == Some(xenadmin_core::ConnectionState::Ready)
            || (view.selected && view.state.is_none());
        let mut row = NavRow::new(root.clone(), inventory.pool_name.clone(), ObjectKind::Pool)
            .object(ObjectRef::pool(pool))
            .expanded();
        if view.synthetic || !current {
            row = row.status(view.status);
        }
        rows.push(row);
        let child = |object: ObjectRef, label: &str, parent: &str| {
            NavRow::new(object_row_id(view.profile, &object), label, object.kind)
                .parent(parent)
                .object(object)
        };
        for host in by_name(inventory.hosts.values().collect(), |host| &host.name) {
            let object = ObjectRef::host(pool, host.id);
            let host_row = object_row_id(view.profile, &object);
            let mut row = child(object, &host.name, &root).expanded();
            if !host.enabled {
                row = row.status("Disabled");
            }
            rows.push(row);
            let guests = by_name(
                inventory
                    .guests()
                    .filter(|vm| vm.host == Some(host.id))
                    .collect(),
                |vm| &vm.name,
            );
            for vm in guests {
                rows.push(
                    child(ObjectRef::vm(pool, vm.id), &vm.name, &host_row)
                        .status(vm.power_state.to_string()),
                );
            }
        }
        let unplaced = by_name(
            inventory.guests().filter(|vm| vm.host.is_none()).collect(),
            |vm| &vm.name,
        );
        for vm in unplaced {
            rows.push(
                child(ObjectRef::vm(pool, vm.id), &vm.name, &root)
                    .status(vm.power_state.to_string()),
            );
        }
        let templates = by_name(
            inventory
                .vms
                .values()
                .filter(|vm| vm.is_template && !vm.is_control_domain)
                .filter(|vm| shows_template(m, &inventory, vm.id))
                .collect(),
            |vm| &vm.name,
        );
        for vm in templates {
            rows.push(child(ObjectRef::template(pool, vm.id), &vm.name, &root));
        }
        let storage = by_name(
            inventory
                .storage
                .iter()
                .filter(|storage| shows_storage(m, &inventory, storage.uuid))
                .collect(),
            |storage| &storage.name,
        );
        for repository in storage {
            rows.push(child(
                ObjectRef::storage(pool, repository.uuid),
                &repository.name,
                &root,
            ));
        }
    }
    rows
}

/// The headings of the Objects view, in the plan's order.
pub const OBJECT_GROUPS: [(ObjectKind, &str); 9] = [
    (ObjectKind::Pool, "Pools"),
    (ObjectKind::Host, "Servers"),
    (ObjectKind::Vm, "VMs"),
    (ObjectKind::Template, "Templates"),
    (ObjectKind::StorageRepository, "Storage repositories"),
    (ObjectKind::Network, "Networks"),
    (ObjectKind::VirtualDisk, "Virtual disks"),
    (ObjectKind::Snapshot, "Snapshots"),
    (ObjectKind::Appliance, "vApps"),
];

/// The id of a heading of the Objects view.
pub fn group_row_id(kind: ObjectKind) -> String {
    format!("objects:{}", kind_word(kind))
}

fn objects(m: &Model) -> Vec<NavRow> {
    let pools = pools(m);
    let mut rows = Vec::new();
    for (kind, title) in OBJECT_GROUPS {
        let group = group_row_id(kind);
        rows.push(NavRow::new(group.clone(), title, kind).expanded());
        // (name, object, status), over every pool, by name.
        let mut items: Vec<(String, ObjectRef, Option<String>)> = Vec::new();
        for (_, inventory) in &pools {
            let pool = inventory.pool_id;
            objects_of(m, inventory, pool, kind, &mut items);
        }
        items.sort_by(|a, b| natural_cmp(&a.0, &b.0).then_with(|| a.1.cmp(&b.1)));
        for (name, object, status) in items {
            let pool = object.pool.map(|pool| pool.to_string()).unwrap_or_default();
            let mut row = NavRow::new(format!("{group}:{pool}:{}", object.id), name, kind)
                .parent(group.clone())
                .object(object);
            row.status = status;
            rows.push(row);
        }
    }
    rows
}

fn objects_of(
    m: &Model,
    inventory: &Inventory,
    pool: PoolId,
    kind: ObjectKind,
    items: &mut Vec<(String, ObjectRef, Option<String>)>,
) {
    match kind {
        ObjectKind::Pool => items.push((inventory.pool_name.clone(), ObjectRef::pool(pool), None)),
        ObjectKind::Host => items.extend(inventory.hosts.values().map(|host| {
            (
                host.name.clone(),
                ObjectRef::host(pool, host.id),
                (!host.enabled).then(|| "Disabled".to_owned()),
            )
        })),
        ObjectKind::Vm => items.extend(inventory.guests().map(|vm| {
            (
                vm.name.clone(),
                ObjectRef::vm(pool, vm.id),
                Some(vm.power_state.to_string()),
            )
        })),
        ObjectKind::Template => items.extend(
            inventory
                .vms
                .values()
                .filter(|vm| vm.is_template && !vm.is_control_domain)
                .filter(|vm| shows_template(m, inventory, vm.id))
                .map(|vm| (vm.name.clone(), ObjectRef::template(pool, vm.id), None)),
        ),
        ObjectKind::StorageRepository => items.extend(
            inventory
                .storage
                .iter()
                .filter(|storage| shows_storage(m, inventory, storage.uuid))
                .map(|storage| {
                    (
                        storage.name.clone(),
                        ObjectRef::storage(pool, storage.uuid),
                        None,
                    )
                }),
        ),
        ObjectKind::Network => items.extend(inventory.networks.iter().map(|network| {
            (
                network.name.clone(),
                ObjectRef::network(pool, network.uuid),
                None,
            )
        })),
        ObjectKind::VirtualDisk => items.extend(
            inventory
                .details
                .vdis
                .values()
                .filter(|disk| disk.is_snapshot != Some(true))
                .map(|disk| {
                    (
                        disk.name.clone(),
                        ObjectRef::new(Some(pool), ObjectKind::VirtualDisk, disk.id),
                        None,
                    )
                }),
        ),
        ObjectKind::Snapshot => items.extend(inventory.snapshots.iter().map(|snapshot| {
            (
                snapshot.name.clone(),
                ObjectRef::snapshot(pool, snapshot.id),
                None,
            )
        })),
        ObjectKind::Appliance => {
            items.extend(inventory.details.appliances.values().map(|appliance| {
                (
                    appliance.name.clone(),
                    ObjectRef::new(Some(pool), ObjectKind::Appliance, appliance.id),
                    None,
                )
            }))
        }
        _ => {}
    }
}

fn notifications() -> Vec<NavRow> {
    vec![
        NavRow::new(
            "notifications:alerts",
            TabId::Alerts.title(),
            ObjectKind::None,
        ),
        NavRow::new(
            "notifications:events",
            TabId::Events.title(),
            ObjectKind::None,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::Target;

    fn demo() -> Model {
        Model::demo(3, 48)
    }

    #[test]
    fn the_infrastructure_view_lists_the_demo_pool_with_hosts_guests_and_storage() {
        let m = demo();
        let rows = view_rows(&m, NavView::Infrastructure);
        let inventory = m.inventory.clone().unwrap();
        let profile = m.workspace.selected.unwrap();
        // The pool is the root, expanded, and says it is synthetic.
        assert_eq!(rows[0].id, session_row_id(profile));
        assert_eq!(rows[0].label, "Demonstration pool");
        assert_eq!(rows[0].icon, ObjectKind::Pool);
        assert_eq!(rows[0].object, Some(ObjectRef::pool(inventory.pool_id)));
        assert_eq!(rows[0].status.as_deref(), Some("Demo"));
        assert!(rows[0].expanded && rows[0].parent.is_none());
        // Three hosts under it, by name, expanded.
        let hosts: Vec<_> = rows
            .iter()
            .filter(|row| row.icon == ObjectKind::Host)
            .collect();
        assert_eq!(
            hosts
                .iter()
                .map(|row| row.label.as_str())
                .collect::<Vec<_>>(),
            ["xcp-lab-01", "xcp-lab-02", "xcp-lab-03"]
        );
        assert!(
            hosts
                .iter()
                .all(|row| row.parent.as_deref() == Some(rows[0].id.as_str()) && row.expanded)
        );
        // Every guest is there once: the running ones under their host, by
        // name, the halted ones under the pool.
        let guests: Vec<_> = rows
            .iter()
            .filter(|row| row.icon == ObjectKind::Vm)
            .collect();
        assert_eq!(guests.len(), 48);
        let under_pool = guests
            .iter()
            .filter(|row| row.parent.as_deref() == Some(rows[0].id.as_str()))
            .count();
        assert_eq!(under_pool, 10, "every fifth guest is halted and unplaced");
        assert!(
            guests
                .iter()
                .filter(|row| row.parent.as_deref() == Some(rows[0].id.as_str()))
                .all(|row| row.status.as_deref() == Some("Halted"))
        );
        let first_host = hosts[0].id.as_str();
        let on_first: Vec<_> = guests
            .iter()
            .filter(|row| row.parent.as_deref() == Some(first_host))
            .map(|row| row.label.as_str())
            .collect();
        let mut sorted = on_first.clone();
        sorted.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(on_first, sorted);
        assert!(!on_first.is_empty());
        // The storage repositories of the demo details are under the pool.
        assert!(
            rows.iter()
                .any(|row| row.icon == ObjectKind::StorageRepository
                    && row.label == "Demo storage"
                    && row.parent.as_deref() == Some(rows[0].id.as_str()))
        );
        // Ids are unique, and every parent is an earlier row.
        let mut seen = std::collections::BTreeSet::new();
        for row in &rows {
            if let Some(parent) = &row.parent {
                assert!(
                    seen.contains(parent.as_str()),
                    "{} before its parent",
                    row.id
                );
            }
            assert!(seen.insert(row.id.as_str()), "{} twice", row.id);
        }
    }

    #[test]
    fn the_objects_view_groups_every_object_by_type() {
        let m = demo();
        let rows = view_rows(&m, NavView::Objects);
        let headings: Vec<_> = rows
            .iter()
            .filter(|row| row.parent.is_none())
            .map(|row| row.label.as_str())
            .collect();
        assert_eq!(
            headings,
            [
                "Pools",
                "Servers",
                "VMs",
                "Templates",
                "Storage repositories",
                "Networks",
                "Virtual disks",
                "Snapshots",
                "vApps"
            ]
        );
        let count = |kind: ObjectKind| {
            rows.iter()
                .filter(|row| row.parent.as_deref() == Some(group_row_id(kind).as_str()))
                .count()
        };
        let inventory = m.inventory.clone().unwrap();
        assert_eq!(count(ObjectKind::Pool), 1);
        assert_eq!(count(ObjectKind::Host), 3);
        assert_eq!(count(ObjectKind::Vm), 48);
        assert_eq!(count(ObjectKind::Network), inventory.networks.len());
        assert_eq!(count(ObjectKind::Snapshot), 3);
        // A heading selects nothing; an object row selects its object.
        assert!(rows[0].object.is_none());
        let vm = rows
            .iter()
            .find(|row| row.label == "Xen Orchestra")
            .expect("the first guest");
        assert_eq!(
            vm.object.as_ref().map(|object| object.kind),
            Some(ObjectKind::Vm)
        );
        assert_eq!(vm.status.as_deref(), Some("Halted"));
    }

    #[test]
    fn organization_and_saved_searches_are_empty_until_their_areas_provide_rows() {
        let m = demo();
        assert!(view_rows(&m, NavView::Organization).is_empty());
        assert!(view_rows(&m, NavView::SavedSearches).is_empty());
        assert_eq!(NavView::Organization.provider(), Some("organization"));
        assert_eq!(NavView::SavedSearches.provider(), Some("search"));
    }

    #[test]
    fn the_notifications_view_has_alerts_and_events_and_its_own_tabs() {
        let mut m = demo();
        m.set_nav_view(NavView::Notifications);
        let rows = rows(&m);
        assert_eq!(
            rows.iter()
                .map(|row| row.label.as_str())
                .collect::<Vec<_>>(),
            ["Alerts", "Events"]
        );
        assert_eq!(m.tabs(), [TabId::Alerts, TabId::Events]);
        assert_eq!(m.active_tab(), TabId::Alerts);
        assert_eq!(selected_ids(&m, &rows), ["notifications:alerts"]);
        m.set_active_tab(TabId::Events);
        assert_eq!(m.active_tab(), TabId::Events);
        assert_eq!(selected_ids(&m, &rows), ["notifications:events"]);
        // Back in another view the tabs are the selection's again.
        m.set_nav_view(NavView::Infrastructure);
        assert!(m.tabs().contains(&TabId::General));
    }

    #[test]
    fn a_filter_keeps_matches_with_their_ancestors_and_descendants() {
        let mut m = demo();
        m.set_nav_filter("xen orchestra");
        let rows = rows(&m);
        // The pool, the hosts that hold a match, and the six matching guests.
        let guests: Vec<_> = rows
            .iter()
            .filter(|row| row.icon == ObjectKind::Vm)
            .collect();
        assert_eq!(guests.len(), 6);
        assert!(guests.iter().all(|row| row.label.contains("Xen Orchestra")));
        assert_eq!(rows[0].icon, ObjectKind::Pool);
        assert!(rows.iter().all(|row| row.expanded));
        assert!(
            !rows
                .iter()
                .any(|row| row.icon == ObjectKind::StorageRepository)
        );
        // A host name keeps the host and what is on it.
        m.set_nav_filter("XCP-LAB-02");
        let rows = super::rows(&m);
        let hosts: Vec<_> = rows
            .iter()
            .filter(|row| row.icon == ObjectKind::Host)
            .collect();
        assert_eq!(hosts.len(), 1);
        assert!(
            rows.iter()
                .filter(|row| row.icon == ObjectKind::Vm)
                .all(|row| row.parent.as_deref() == Some(hosts[0].id.as_str()))
        );
        assert!(rows.iter().any(|row| row.icon == ObjectKind::Vm));
        // A UUID fragment finds the object.
        m.set_nav_filter("30000000-0000-0000-0000-000000000007");
        let rows = super::rows(&m);
        assert_eq!(
            rows.iter().filter(|row| row.icon == ObjectKind::Vm).count(),
            1
        );
        // Nothing matches: no rows.
        m.set_nav_filter("no such object");
        assert!(super::rows(&m).is_empty());
        m.set_nav_filter("  ");
        assert_eq!(
            super::rows(&m).len(),
            view_rows(&m, NavView::Infrastructure).len()
        );
    }

    #[test]
    fn a_saved_connection_without_an_inventory_is_a_node_with_its_status_word() {
        let mut m = demo();
        let profile = xenadmin_core::ConnectionProfile::new(
            "Recovery pool".into(),
            xenadmin_core::Endpoint::parse("https://recovery.example").unwrap(),
            "operator".into(),
        );
        let id = profile.id;
        m.workspace.preview_profiles.push(profile);
        let rows = view_rows(&m, NavView::Infrastructure);
        let node = rows
            .iter()
            .find(|row| row.id == session_row_id(id))
            .expect("the saved connection");
        assert_eq!(node.label, "Recovery pool");
        assert_eq!(node.icon, ObjectKind::Connection);
        assert_eq!(node.status.as_deref(), Some("Disconnected"));
        assert_eq!(
            node.object,
            Some(ObjectRef::new(None, ObjectKind::Connection, id))
        );
        assert!(
            !rows
                .iter()
                .any(|row| row.parent.as_deref() == Some(node.id.as_str()))
        );
        // Connections are listed by name: the demo pool first.
        assert_eq!(rows[0].label, "Demonstration pool");
        assert_eq!(
            rows.last().map(|row| row.id.as_str()),
            Some(node.id.as_str())
        );
    }

    #[test]
    fn two_pools_are_listed_side_by_side_and_the_selected_rows_follow_the_scope() {
        let mut m = demo();
        let mut second = crate::demonstration(1, 4);
        second.pool_id = PoolId::parse("10000000-0000-0000-0000-000000000063").unwrap();
        second.pool_name = "Recovery lab".into();
        let other = m.add_synthetic_pool("Recovery lab", second);
        let rows = view_rows(&m, NavView::Infrastructure);
        let roots: Vec<_> = rows
            .iter()
            .filter(|row| row.parent.is_none())
            .map(|row| row.label.as_str())
            .collect();
        assert_eq!(roots, ["Demonstration pool", "Recovery lab"]);
        // The same VM UUID exists in both pools; the rows are distinct.
        let same: Vec<_> = rows
            .iter()
            .filter(|row| {
                row.object
                    .as_ref()
                    .is_some_and(|o| o.id == "30000000-0000-0000-0000-000000000001")
            })
            .collect();
        assert_eq!(same.len(), 2);
        assert_ne!(same[0].id, same[1].id);
        assert_ne!(same[0].object, same[1].object);
        // The scope marks exactly its own row.
        let pool = PoolId::parse("10000000-0000-0000-0000-000000000063").unwrap();
        let vm = xenadmin_core::VmId::parse("30000000-0000-0000-0000-000000000001").unwrap();
        m.select(Target::one(ObjectRef::vm(pool, vm)));
        let rows = view_rows(&m, NavView::Infrastructure);
        let selected = selected_ids(&m, &rows);
        assert_eq!(selected.len(), 1);
        assert!(selected[0].starts_with(&other.to_string()));
        // The Objects view lists both pools' objects together.
        let objects = view_rows(&m, NavView::Objects);
        assert_eq!(
            objects
                .iter()
                .filter(|row| row.parent.as_deref() == Some("objects:pool"))
                .count(),
            2
        );
        assert_eq!(
            objects
                .iter()
                .filter(|row| row.parent.as_deref() == Some("objects:vm"))
                .count(),
            52
        );
    }

    #[test]
    fn the_view_toggles_hide_built_in_templates_and_local_storage() {
        let mut m = demo();
        let mut inventory = m.inventory.clone().unwrap().as_ref().clone();
        let pool = inventory.pool_id;
        // Two templates, one of them shipped by the host.
        let mut custom = inventory.vms.values().next().unwrap().clone();
        custom.id = xenadmin_core::VmId::parse("30000000-0000-0000-0000-00000000aaaa").unwrap();
        custom.name = "Golden image".into();
        custom.is_template = true;
        custom.host = None;
        let mut builtin = custom.clone();
        builtin.id = xenadmin_core::VmId::parse("30000000-0000-0000-0000-00000000bbbb").unwrap();
        builtin.name = "Debian 12".into();
        inventory
            .details
            .vms
            .entry(builtin.id)
            .or_default()
            .is_default_template = Some(true);
        inventory.vms.insert(custom.id, custom);
        inventory.vms.insert(builtin.id, builtin);
        m.inventory = Some(std::sync::Arc::new(inventory));
        let labels = |m: &Model| {
            view_rows(m, NavView::Infrastructure)
                .into_iter()
                .filter(|row| row.icon == ObjectKind::Template)
                .map(|row| row.label)
                .collect::<Vec<_>>()
        };
        assert_eq!(labels(&m), ["Golden image"]);
        m.nav.show_builtin_templates = true;
        assert_eq!(labels(&m), ["Debian 12", "Golden image"]);
        m.nav.show_custom_templates = false;
        assert_eq!(labels(&m), ["Debian 12"]);
        // A template row selects a template, not a VM.
        let rows = view_rows(&m, NavView::Infrastructure);
        let row = rows.iter().find(|row| row.label == "Debian 12").unwrap();
        assert_eq!(
            row.object,
            Some(ObjectRef::template(
                pool,
                xenadmin_core::VmId::parse("30000000-0000-0000-0000-00000000bbbb").unwrap()
            ))
        );
    }
}
