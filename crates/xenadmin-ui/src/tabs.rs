//! Tabs: which the selection has, what a tab body is, and which area
//! provides it.

use serde::Serialize;

use crate::object::ObjectKind;
use crate::surface::{Block, Custom};

/// A tab of the selected object, or of the Notifications view.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TabId {
    Home,
    General,
    Memory,
    Storage,
    Networking,
    Nics,
    Ha,
    Users,
    Console,
    Performance,
    Snapshots,
    Search,
    /// Notifications view: pool messages.
    Alerts,
    /// Notifications view: operations, tasks and unknown outcomes.
    Events,
}

impl TabId {
    pub const ALL: [TabId; 14] = [
        TabId::Home,
        TabId::General,
        TabId::Memory,
        TabId::Storage,
        TabId::Networking,
        TabId::Nics,
        TabId::Ha,
        TabId::Users,
        TabId::Console,
        TabId::Performance,
        TabId::Snapshots,
        TabId::Search,
        TabId::Alerts,
        TabId::Events,
    ];

    /// The label on the tab strip.
    pub fn title(self) -> &'static str {
        match self {
            Self::Home => "Home",
            Self::General => "General",
            Self::Memory => "Memory",
            Self::Storage => "Storage",
            Self::Networking => "Networking",
            Self::Nics => "NICs",
            Self::Ha => "HA",
            Self::Users => "Users",
            Self::Console => "Console",
            Self::Performance => "Performance",
            Self::Snapshots => "Snapshots",
            Self::Search => "Search",
            Self::Alerts => "Alerts",
            Self::Events => "Events",
        }
    }

    /// The key of the tab on the tab strip, for example `tab-general`.
    pub fn key(self) -> &'static str {
        match self {
            Self::Home => "tab-home",
            Self::General => "tab-general",
            Self::Memory => "tab-memory",
            Self::Storage => "tab-storage",
            Self::Networking => "tab-networking",
            Self::Nics => "tab-nics",
            Self::Ha => "tab-ha",
            Self::Users => "tab-users",
            Self::Console => "tab-console",
            Self::Performance => "tab-performance",
            Self::Snapshots => "tab-snapshots",
            Self::Search => "tab-search",
            Self::Alerts => "tab-alerts",
            Self::Events => "tab-events",
        }
    }
}

/// What a tab shows: blocks the generic renderer draws, or one bespoke view
/// of the front end (the console, the charts).
#[derive(Clone, Debug)]
pub enum TabBody {
    Blocks(Vec<Block>),
    Custom(Custom),
}

impl TabBody {
    /// The blocks of the body. A custom body is one custom block.
    pub fn blocks(&self) -> Vec<Block> {
        match self {
            Self::Blocks(blocks) => blocks.clone(),
            Self::Custom(custom) => vec![Block::Custom(custom.clone())],
        }
    }
}

impl From<Vec<Block>> for TabBody {
    fn from(blocks: Vec<Block>) -> Self {
        Self::Blocks(blocks)
    }
}

impl From<Custom> for TabBody {
    fn from(custom: Custom) -> Self {
        Self::Custom(custom)
    }
}

const HOME: &[TabId] = &[TabId::Home, TabId::Search];
const POOL: &[TabId] = &[
    TabId::General,
    TabId::Memory,
    TabId::Storage,
    TabId::Networking,
    TabId::Ha,
    TabId::Users,
    TabId::Search,
];
const HOST_LIVE: &[TabId] = &[
    TabId::General,
    TabId::Memory,
    TabId::Storage,
    TabId::Networking,
    TabId::Nics,
    TabId::Console,
    TabId::Performance,
    TabId::Search,
];
const HOST_OFFLINE: &[TabId] = &[TabId::General, TabId::Search];
const VM: &[TabId] = &[
    TabId::General,
    TabId::Memory,
    TabId::Storage,
    TabId::Networking,
    TabId::Console,
    TabId::Performance,
    TabId::Snapshots,
    TabId::Search,
];
const TEMPLATE: &[TabId] = &[
    TabId::General,
    TabId::Memory,
    TabId::Storage,
    TabId::Networking,
    TabId::Search,
];
const STORAGE: &[TabId] = &[TabId::General, TabId::Storage, TabId::Search];
const NETWORK: &[TabId] = &[TabId::General, TabId::Networking, TabId::Search];
const DISK: &[TabId] = &[TabId::General, TabId::Search];
const SEARCH_ONLY: &[TabId] = &[TabId::Search];

/// The tabs of the Notifications navigation view. It replaces the Activity
/// dialog (AREAS.md, decision 6).
pub const NOTIFICATION_TABS: &[TabId] = &[TabId::Alerts, TabId::Events];

/// The tab matrix of the plan: the tabs of a selection of this kind, in
/// column order. `host_live` matters only for `Host`.
///
/// Kinds the plan's matrix does not name:
/// - `Connection` (a saved profile that is not connected) has the row
///   "Host, not live or disconnected".
/// - `Snapshot` and `Appliance` have the Search tab only, like the plan's
///   last row: no Rune module shows a General view of either.
pub fn tabs_for(kind: ObjectKind, host_live: bool) -> &'static [TabId] {
    match kind {
        ObjectKind::None => HOME,
        ObjectKind::Pool => POOL,
        ObjectKind::Host if host_live => HOST_LIVE,
        ObjectKind::Host | ObjectKind::Connection => HOST_OFFLINE,
        ObjectKind::Vm => VM,
        ObjectKind::Template => TEMPLATE,
        ObjectKind::StorageRepository => STORAGE,
        ObjectKind::Network => NETWORK,
        ObjectKind::VirtualDisk => DISK,
        ObjectKind::Snapshot
        | ObjectKind::Appliance
        | ObjectKind::Folder
        | ObjectKind::Tag
        | ObjectKind::SavedSearch
        | ObjectKind::Multi => SEARCH_ONLY,
    }
}

/// The areas that can provide a tab for an object kind, by area id, in the
/// order they are asked. The first whose `tab` function returns a body wins.
/// The `general` area is the information-only fallback of Storage,
/// Networking and NICs until their areas land (AREAS.md).
pub fn providers(tab: TabId, kind: ObjectKind) -> &'static [&'static str] {
    use ObjectKind as K;
    match tab {
        TabId::Home => &["connections"],
        TabId::General | TabId::Memory => &["general"],
        TabId::Storage => match kind {
            K::Vm | K::Template | K::StorageRepository => &["vm_devices", "general"],
            K::Host | K::Pool => &["storage", "general"],
            _ => &["general"],
        },
        TabId::Networking => match kind {
            K::Vm | K::Template => &["vm_devices", "general"],
            K::Host | K::Pool | K::Network => &["network", "general"],
            _ => &["general"],
        },
        TabId::Nics => &["network", "general"],
        TabId::Ha => &["pool"],
        TabId::Users => &["pool_security"],
        TabId::Console => &["console"],
        TabId::Performance => &["performance"],
        TabId::Snapshots => &["snapshots"],
        TabId::Search => &["search"],
        TabId::Alerts | TabId::Events => &["events"],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use TabId::*;

    #[test]
    fn nothing_or_the_root_node_has_home_and_search() {
        assert_eq!(tabs_for(ObjectKind::None, false), [Home, Search]);
    }

    #[test]
    fn a_pool_has_ha_and_users_but_no_console() {
        assert_eq!(
            tabs_for(ObjectKind::Pool, false),
            [General, Memory, Storage, Networking, Ha, Users, Search]
        );
    }

    #[test]
    fn a_live_host_has_nics_console_and_performance() {
        assert_eq!(
            tabs_for(ObjectKind::Host, true),
            [
                General,
                Memory,
                Storage,
                Networking,
                Nics,
                Console,
                Performance,
                Search
            ]
        );
    }

    #[test]
    fn a_host_that_is_not_live_has_general_and_search_only() {
        assert_eq!(tabs_for(ObjectKind::Host, false), [General, Search]);
        assert_eq!(tabs_for(ObjectKind::Connection, false), [General, Search]);
        assert_eq!(tabs_for(ObjectKind::Connection, true), [General, Search]);
    }

    #[test]
    fn a_vm_has_console_performance_and_snapshots() {
        assert_eq!(
            tabs_for(ObjectKind::Vm, false),
            [
                General,
                Memory,
                Storage,
                Networking,
                Console,
                Performance,
                Snapshots,
                Search
            ]
        );
    }

    #[test]
    fn a_template_has_no_console_and_no_snapshots() {
        assert_eq!(
            tabs_for(ObjectKind::Template, false),
            [General, Memory, Storage, Networking, Search]
        );
    }

    #[test]
    fn a_storage_repository_has_general_storage_and_search() {
        assert_eq!(
            tabs_for(ObjectKind::StorageRepository, false),
            [General, Storage, Search]
        );
    }

    #[test]
    fn a_network_has_general_networking_and_search() {
        assert_eq!(
            tabs_for(ObjectKind::Network, false),
            [General, Networking, Search]
        );
    }

    #[test]
    fn a_virtual_disk_has_general_and_search() {
        assert_eq!(tabs_for(ObjectKind::VirtualDisk, false), [General, Search]);
    }

    #[test]
    fn several_objects_a_folder_a_tag_and_a_saved_search_have_search_only() {
        for kind in [
            ObjectKind::Multi,
            ObjectKind::Folder,
            ObjectKind::Tag,
            ObjectKind::SavedSearch,
            ObjectKind::Snapshot,
            ObjectKind::Appliance,
        ] {
            assert_eq!(tabs_for(kind, false), [Search], "{kind:?}");
        }
    }

    #[test]
    fn only_the_live_host_row_depends_on_liveness_and_every_row_ends_with_search() {
        let kinds = [
            ObjectKind::None,
            ObjectKind::Pool,
            ObjectKind::Host,
            ObjectKind::Vm,
            ObjectKind::Template,
            ObjectKind::StorageRepository,
            ObjectKind::Network,
            ObjectKind::VirtualDisk,
            ObjectKind::Snapshot,
            ObjectKind::Appliance,
            ObjectKind::Folder,
            ObjectKind::Tag,
            ObjectKind::SavedSearch,
            ObjectKind::Connection,
            ObjectKind::Multi,
        ];
        for kind in kinds {
            let tabs = tabs_for(kind, true);
            assert_eq!(tabs.last(), Some(&Search), "{kind:?}");
            if kind != ObjectKind::Host {
                assert_eq!(tabs, tabs_for(kind, false), "{kind:?}");
            }
            assert!(
                !tabs.contains(&Alerts) && !tabs.contains(&Events),
                "{kind:?}"
            );
            assert_eq!(tabs.contains(&Home), kind == ObjectKind::None, "{kind:?}");
        }
        assert_eq!(NOTIFICATION_TABS, [Alerts, Events]);
    }

    #[test]
    fn every_tab_of_the_matrix_has_a_provider_that_is_an_area() {
        let areas: Vec<_> = crate::areas::specs().iter().map(|area| area.id).collect();
        let kinds = [
            ObjectKind::None,
            ObjectKind::Pool,
            ObjectKind::Host,
            ObjectKind::Vm,
            ObjectKind::Template,
            ObjectKind::StorageRepository,
            ObjectKind::Network,
            ObjectKind::VirtualDisk,
            ObjectKind::Multi,
        ];
        for kind in kinds {
            for tab in tabs_for(kind, true).iter().chain(NOTIFICATION_TABS) {
                let ids = providers(*tab, kind);
                assert!(!ids.is_empty(), "{tab:?} of {kind:?}");
                for id in ids {
                    assert!(areas.contains(id), "{id} is not an area");
                }
            }
        }
    }

    #[test]
    fn the_provider_table_follows_the_area_assignment() {
        assert_eq!(providers(Home, ObjectKind::None), ["connections"]);
        assert_eq!(providers(General, ObjectKind::Vm), ["general"]);
        assert_eq!(providers(Memory, ObjectKind::Host), ["general"]);
        for kind in [
            ObjectKind::Vm,
            ObjectKind::Template,
            ObjectKind::StorageRepository,
        ] {
            assert_eq!(providers(Storage, kind), ["vm_devices", "general"]);
        }
        for kind in [ObjectKind::Host, ObjectKind::Pool] {
            assert_eq!(providers(Storage, kind), ["storage", "general"]);
            assert_eq!(providers(Networking, kind), ["network", "general"]);
        }
        assert_eq!(
            providers(Networking, ObjectKind::Network),
            ["network", "general"]
        );
        assert_eq!(
            providers(Networking, ObjectKind::Template),
            ["vm_devices", "general"]
        );
        assert_eq!(providers(Nics, ObjectKind::Host), ["network", "general"]);
        assert_eq!(providers(Ha, ObjectKind::Pool), ["pool"]);
        assert_eq!(providers(Users, ObjectKind::Pool), ["pool_security"]);
        assert_eq!(providers(Console, ObjectKind::Vm), ["console"]);
        assert_eq!(providers(Performance, ObjectKind::Host), ["performance"]);
        assert_eq!(providers(Snapshots, ObjectKind::Vm), ["snapshots"]);
        assert_eq!(providers(Search, ObjectKind::Multi), ["search"]);
        assert_eq!(providers(Alerts, ObjectKind::None), ["events"]);
        assert_eq!(providers(Events, ObjectKind::None), ["events"]);
        assert_eq!(TabId::Nics.title(), "NICs");
        assert_eq!(TabId::General.key(), "tab-general");
        assert_eq!(TabId::ALL.len(), 14);
    }
}
