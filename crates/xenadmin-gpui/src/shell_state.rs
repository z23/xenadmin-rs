//! Shell selection for the GPUI window. Inventory and commands stay in `xenadmin-ui`.

use xenadmin_core::{HostId, Inventory, VmId};
use xenadmin_ui::{
    DialogId, InventoryPage, ObjectKind, Selection, TabId, demonstration, populate_demo_details,
    tabs_for,
};

#[derive(Clone, Debug)]
pub struct ShellState {
    pub inventory: Inventory,
    pub kind: ObjectKind,
    pub host_live: bool,
    pub host: Option<HostId>,
    pub vm: Option<VmId>,
    pub tab: TabId,
    pub query: String,
    pub dialog: Option<DialogId>,
    pub dialog_text: Option<String>,
    pub notice: Option<String>,
    pub scene: String,
    history: Vec<Bookmark>,
    future: Vec<Bookmark>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Bookmark {
    kind: ObjectKind,
    host_live: bool,
    host: Option<HostId>,
    vm: Option<VmId>,
    tab: TabId,
}

impl ShellState {
    pub fn demo(host_count: usize, vm_count: usize) -> Self {
        let mut inventory = demonstration(host_count, vm_count);
        populate_demo_details(&mut inventory);
        let mut state = Self {
            inventory,
            kind: ObjectKind::Pool,
            host_live: false,
            host: None,
            vm: None,
            tab: TabId::General,
            query: String::new(),
            dialog: None,
            dialog_text: None,
            notice: None,
            scene: "demo".into(),
            history: Vec::new(),
            future: Vec::new(),
        };
        state.tab = state.tabs()[0];
        state
    }

    pub fn selection(&self) -> Selection {
        Selection {
            kind: self.kind,
            host_live: self.host_live,
        }
    }

    pub fn tabs(&self) -> &'static [TabId] {
        tabs_for(self.selection()).tabs()
    }

    pub fn select_pool(&mut self) {
        self.push();
        self.kind = ObjectKind::Pool;
        self.host = None;
        self.vm = None;
        self.host_live = false;
        self.keep_tab();
    }

    pub fn select_host(&mut self, id: HostId) {
        let live = self
            .inventory
            .hosts
            .get(&id)
            .is_some_and(|host| host.enabled);
        self.push();
        self.kind = ObjectKind::Host;
        self.host = Some(id);
        self.vm = None;
        self.host_live = live;
        self.keep_tab();
    }

    pub fn select_vm(&mut self, id: VmId) {
        let host = self.inventory.vms.get(&id).and_then(|vm| vm.host);
        self.push();
        self.kind = ObjectKind::Vm;
        self.host = host;
        self.vm = Some(id);
        self.host_live = false;
        self.keep_tab();
    }

    pub fn select_first_vm(&mut self) {
        if let Some(id) = self.inventory.vms.keys().next().copied() {
            self.select_vm(id);
        }
    }

    pub fn set_tab(&mut self, tab: TabId) {
        if self.tabs().contains(&tab) {
            self.push();
            self.tab = tab;
        }
    }

    pub fn back(&mut self) -> bool {
        let Some(previous) = self.history.pop() else {
            return false;
        };
        self.future.push(self.bookmark());
        self.restore(previous);
        true
    }

    pub fn forward(&mut self) -> bool {
        let Some(next) = self.future.pop() else {
            return false;
        };
        self.history.push(self.bookmark());
        self.restore(next);
        true
    }

    pub fn selected_name(&self) -> String {
        match self.kind {
            ObjectKind::Vm => self
                .vm
                .and_then(|id| self.inventory.vms.get(&id))
                .map(|vm| vm.name.clone())
                .unwrap_or_else(|| "Virtual machine".into()),
            ObjectKind::Host => self
                .host
                .and_then(|id| self.inventory.hosts.get(&id))
                .map(|host| host.name.clone())
                .unwrap_or_else(|| "Server".into()),
            ObjectKind::Pool => self.inventory.pool_name.clone(),
            _ => "Home".into(),
        }
    }

    pub fn rows_page(&self) -> InventoryPage {
        InventoryPage::Guests
    }

    fn keep_tab(&mut self) {
        if !self.tabs().contains(&self.tab) {
            self.tab = self.tabs()[0];
        }
    }

    fn push(&mut self) {
        self.history.push(self.bookmark());
        self.future.clear();
    }

    fn bookmark(&self) -> Bookmark {
        Bookmark {
            kind: self.kind,
            host_live: self.host_live,
            host: self.host,
            vm: self.vm,
            tab: self.tab,
        }
    }

    fn restore(&mut self, bookmark: Bookmark) {
        self.kind = bookmark.kind;
        self.host_live = bookmark.host_live;
        self.host = bookmark.host;
        self.vm = bookmark.vm;
        self.tab = bookmark.tab;
    }
}

/// Scene names the shell can open without a host. Live pool scenes stay on the Rune binary.
pub fn load_scene(name: &str) -> ShellState {
    let mut state = if name == "large-inventory" {
        ShellState::demo(16, 2_000)
    } else {
        ShellState::demo(3, 48)
    };
    state.scene = name.to_string();
    match name {
        "connect" | "connections" | "connection-settings" => {
            state.dialog = Some(DialogId::AddServer);
            state.dialog_text =
                Some("Address, username, and password stay in this preview.".into());
        }
        "connection-trust" => {
            state.dialog = Some(DialogId::AddServer);
            state.dialog_text = Some("Trust is not stored from this preview.".into());
        }
        "certificate-review" => {
            state.dialog = Some(DialogId::AddServer);
            state.dialog_text =
                Some("Review the certificate. This preview does not contact a host.".into());
        }
        "connect-insecure" => {
            state.dialog = Some(DialogId::AddServer);
            state.dialog_text =
                Some("The certificate is not pinned. This preview does not contact a host.".into());
            state.notice = Some("Certificate checks are not pinned in this preview.".into());
        }
        "search-pools" => state.set_tab(TabId::Search),
        "general-information"
        | "demo"
        | "unavailable-actions"
        | "inventory-context"
        | "interface-large" => state.select_first_vm(),
        "host-version-notice" => {
            if let Some(host) = state.inventory.hosts.values_mut().next() {
                host.version = "8.2.1".into();
            }
        }
        _ => {}
    }
    state
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_stays_on_the_same_vm_when_the_inventory_is_touched() {
        let mut state = ShellState::demo(3, 48);
        state.select_first_vm();
        let id = state.vm.expect("vm");
        state.inventory.revision += 1;
        assert_eq!(state.vm, Some(id));
        assert!(state.inventory.vms.contains_key(&id));
    }

    #[test]
    fn back_returns_to_the_pool_tab() {
        let mut state = ShellState::demo(3, 48);
        let pool_tab = state.tab;
        state.select_first_vm();
        assert_eq!(state.kind, ObjectKind::Vm);
        assert!(state.back());
        assert_eq!(state.kind, ObjectKind::Pool);
        assert_eq!(state.tab, pool_tab);
        assert!(state.forward());
        assert_eq!(state.kind, ObjectKind::Vm);
    }

    #[test]
    fn a_live_host_keeps_the_console_tab_available() {
        let mut state = ShellState::demo(3, 48);
        let id = *state.inventory.hosts.keys().next().unwrap();
        state.select_host(id);
        assert!(state.tabs().contains(&TabId::Console));
        assert!(state.tabs().contains(&TabId::Nics));
    }
}
