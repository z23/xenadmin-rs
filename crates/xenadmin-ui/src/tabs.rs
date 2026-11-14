//! Tabs for the selected object. The order is the order of [`TabId`].

use crate::selection::{ObjectKind, Selection};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TabSet {
    tabs: &'static [TabId],
}

impl TabSet {
    pub fn tabs(self) -> &'static [TabId] {
        self.tabs
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

pub fn tabs_for(selection: Selection) -> TabSet {
    let tabs = match selection.kind {
        ObjectKind::None => HOME,
        ObjectKind::Pool => POOL,
        ObjectKind::Host if selection.host_live => HOST_LIVE,
        ObjectKind::Host => HOST_OFFLINE,
        ObjectKind::Vm => VM,
        ObjectKind::Template => TEMPLATE,
        ObjectKind::StorageRepository => STORAGE,
        ObjectKind::Network => NETWORK,
        ObjectKind::VirtualDisk => DISK,
        ObjectKind::Multi => SEARCH_ONLY,
    };
    TabSet { tabs }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selection::Selection;

    fn tabs(kind: ObjectKind, host_live: bool) -> &'static [TabId] {
        tabs_for(Selection { kind, host_live }).tabs()
    }

    #[test]
    fn every_matrix_row_has_its_tabs_in_order() {
        assert_eq!(tabs(ObjectKind::None, false), HOME);
        assert_eq!(tabs(ObjectKind::Pool, false), POOL);
        assert_eq!(tabs(ObjectKind::Host, true), HOST_LIVE);
        assert_eq!(tabs(ObjectKind::Host, false), HOST_OFFLINE);
        assert_eq!(tabs(ObjectKind::Vm, false), VM);
        assert_eq!(tabs(ObjectKind::Template, false), TEMPLATE);
        assert_eq!(tabs(ObjectKind::StorageRepository, false), STORAGE);
        assert_eq!(tabs(ObjectKind::Network, false), NETWORK);
        assert_eq!(tabs(ObjectKind::VirtualDisk, false), DISK);
        assert_eq!(tabs(ObjectKind::Multi, false), SEARCH_ONLY);
        assert!(HOME.contains(&TabId::Home) && HOME.contains(&TabId::Search));
        assert!(!HOST_OFFLINE.contains(&TabId::Console));
        assert!(HOST_LIVE.contains(&TabId::Nics));
        assert!(VM.contains(&TabId::Snapshots));
        assert!(POOL.contains(&TabId::Ha) && POOL.contains(&TabId::Users));
    }
}
