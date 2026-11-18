//! The command table.
//!
//! Every action is a [`CommandSpec`]. The menu bar, the toolbar, the context
//! menus and the local action row of a tab are generated from the table and
//! from nothing else: a view never builds its own menu, and the front end
//! renders the resolved views of this module without knowing any area.
//!
//! Each area exports `fn commands() -> Vec<CommandSpec>` through its
//! [`crate::areas::AreaSpec`]; [`registry`] concatenates them.
//!
//! A command is run by [`Model::invoke`], which evaluates the eligibility
//! again and refuses a command that is not enabled.

use std::fmt;
use std::sync::OnceLock;

use serde::Serialize;

use crate::intent::Intent;
use crate::model::Model;
use crate::object::{ObjectKind, Target};
use crate::tabs::TabId;

/// The stable id of a command, for example `server.maintenance`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub struct CommandId(pub &'static str);

impl fmt::Display for CommandId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// Whether a command can run on a target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Eligibility {
    Enabled,
    /// Shown but not available, with the reason the Rune code or the core
    /// gives. The reason is the tooltip and is in the tree dump.
    Disabled(String),
    /// Not shown at all for this target.
    Hidden,
}

impl Eligibility {
    pub fn disabled(reason: impl Into<String>) -> Self {
        Self::Disabled(reason.into())
    }

    /// `Enabled` when `condition` holds, otherwise disabled with the reason.
    pub fn enabled_if(condition: bool, reason: impl Into<String>) -> Self {
        if condition {
            Self::Enabled
        } else {
            Self::Disabled(reason.into())
        }
    }

    /// `Enabled` without a reason, otherwise disabled with it. For checks
    /// that return `Option<String>`.
    pub fn enabled_unless(reason: Option<impl Into<String>>) -> Self {
        match reason {
            None => Self::Enabled,
            Some(reason) => Self::Disabled(reason.into()),
        }
    }

    pub fn is_enabled(&self) -> bool {
        *self == Self::Enabled
    }

    pub fn is_hidden(&self) -> bool {
        *self == Self::Hidden
    }

    /// The reason of a disabled command.
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Disabled(reason) => Some(reason),
            _ => None,
        }
    }
}

/// The menus of the menu bar, in the plan's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Menu {
    File,
    View,
    Pool,
    Server,
    Vm,
    Storage,
    Templates,
    Tools,
    Window,
    Help,
}

impl Menu {
    pub const ALL: [Menu; 10] = [
        Menu::File,
        Menu::View,
        Menu::Pool,
        Menu::Server,
        Menu::Vm,
        Menu::Storage,
        Menu::Templates,
        Menu::Tools,
        Menu::Window,
        Menu::Help,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::File => "File",
            Self::View => "View",
            Self::Pool => "Pool",
            Self::Server => "Server",
            Self::Vm => "VM",
            Self::Storage => "Storage",
            Self::Templates => "Templates",
            Self::Tools => "Tools",
            Self::Window => "Window",
            Self::Help => "Help",
        }
    }
}

/// Where a command sits in the menu bar.
///
/// Items of a menu are ordered by `group`, then `order`, then registry
/// order. A separator is drawn between groups. Items that name the same
/// `submenu` within a group are collected into one submenu, placed where its
/// first item would be.
///
/// The positions of the plan's menu items are the constants of [`place`].
/// Use them; pick a free `order` between two of them for a command the plan
/// does not list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct MenuPath {
    pub menu: Menu,
    pub group: u8,
    pub order: u8,
    pub submenu: Option<&'static str>,
}

impl MenuPath {
    pub const fn new(menu: Menu, group: u8, order: u8) -> Self {
        Self {
            menu,
            group,
            order,
            submenu: None,
        }
    }

    pub const fn submenu(mut self, name: &'static str) -> Self {
        self.submenu = Some(name);
        self
    }
}

/// The menu positions of the plan ("Target information architecture",
/// Menus), in the plan's order. Items the client never implemented are left
/// out (AREAS.md, decision 5).
pub mod place {
    use super::{Menu, MenuPath};

    const fn at(menu: Menu, group: u8, order: u8) -> MenuPath {
        MenuPath::new(menu, group, order)
    }

    pub const FILE_IMPORT: MenuPath = at(Menu::File, 1, 10);
    pub const FILE_IMPORT_SEARCH: MenuPath = at(Menu::File, 1, 20);
    pub const FILE_EXIT: MenuPath = at(Menu::File, 9, 10);

    pub const VIEW_CUSTOM_TEMPLATES: MenuPath = at(Menu::View, 1, 10);
    pub const VIEW_BUILTIN_TEMPLATES: MenuPath = at(Menu::View, 1, 20);
    pub const VIEW_LOCAL_STORAGE: MenuPath = at(Menu::View, 1, 30);
    pub const VIEW_HIDDEN_OBJECTS: MenuPath = at(Menu::View, 1, 40);
    pub const VIEW_TOOLBAR: MenuPath = at(Menu::View, 2, 10);
    /// Not in the plan: the interface scale commands the client has.
    pub const VIEW_LARGER: MenuPath = at(Menu::View, 3, 10);
    pub const VIEW_SMALLER: MenuPath = at(Menu::View, 3, 20);
    pub const VIEW_RESET_SIZE: MenuPath = at(Menu::View, 3, 30);

    pub const POOL_ADD_SERVER: MenuPath = at(Menu::Pool, 1, 10);
    pub const POOL_REMOVE_SERVER: MenuPath = at(Menu::Pool, 1, 20);
    pub const POOL_RECONNECT_AS: MenuPath = at(Menu::Pool, 1, 30);
    pub const POOL_DISCONNECT: MenuPath = at(Menu::Pool, 1, 40);
    pub const POOL_VAPPS: MenuPath = at(Menu::Pool, 2, 10);
    pub const POOL_HIGH_AVAILABILITY: MenuPath = at(Menu::Pool, 2, 20);
    pub const POOL_SNAPSHOT_SCHEDULES: MenuPath = at(Menu::Pool, 2, 30);
    /// Client-only: the plan's Pool menu does not list it.
    pub const POOL_VM_AUTOSTART: MenuPath = at(Menu::Pool, 2, 40);
    pub const POOL_CHANGE_SERVER_PASSWORD: MenuPath = at(Menu::Pool, 3, 10);
    pub const POOL_MAKE_STANDALONE: MenuPath = at(Menu::Pool, 3, 20);
    pub const POOL_PROPERTIES: MenuPath = at(Menu::Pool, 9, 10);

    pub const SERVER_ADD: MenuPath = at(Menu::Server, 1, 10);
    pub const SERVER_REBOOT: MenuPath = at(Menu::Server, 2, 10);
    /// The plan lists Power On between Reboot and Shut Down.
    pub const SERVER_POWER_ON: MenuPath = at(Menu::Server, 2, 15);
    pub const SERVER_SHUT_DOWN: MenuPath = at(Menu::Server, 2, 20);
    pub const SERVER_RESTART_TOOLSTACK: MenuPath = at(Menu::Server, 2, 30);
    pub const SERVER_CONNECT: MenuPath = at(Menu::Server, 3, 10);
    pub const SERVER_DISCONNECT: MenuPath = at(Menu::Server, 3, 20);
    pub const SERVER_RECONNECT_AS: MenuPath = at(Menu::Server, 3, 30);
    pub const SERVER_CONNECT_ALL: MenuPath = at(Menu::Server, 3, 40);
    pub const SERVER_DISCONNECT_ALL: MenuPath = at(Menu::Server, 3, 50);
    pub const SERVER_BACK_UP: MenuPath = at(Menu::Server, 4, 10);
    pub const SERVER_RESTORE_FROM_BACKUP: MenuPath = at(Menu::Server, 4, 20);
    pub const SERVER_MAINTENANCE: MenuPath = at(Menu::Server, 5, 10);
    /// Client-only: beside the maintenance item it is a part of.
    pub const SERVER_EVACUATE: MenuPath = at(Menu::Server, 5, 15);
    pub const SERVER_CONTROL_DOMAIN_MEMORY: MenuPath = at(Menu::Server, 5, 20);
    pub const SERVER_PASSWORD_CHANGE: MenuPath = at(Menu::Server, 5, 30).submenu("Password");
    pub const SERVER_PASSWORD_FORGET: MenuPath = at(Menu::Server, 5, 40).submenu("Password");
    pub const SERVER_REMOVE_FROM_POOL: MenuPath = at(Menu::Server, 6, 10);
    pub const SERVER_DESTROY: MenuPath = at(Menu::Server, 6, 20);
    pub const SERVER_PROPERTIES: MenuPath = at(Menu::Server, 9, 10);

    /// The name of the power submenu of the VM menu.
    pub const VM_POWER_SUBMENU: &str = "Start/Shut Down";
    pub const VM_NEW: MenuPath = at(Menu::Vm, 1, 10);
    pub const VM_START: MenuPath = at(Menu::Vm, 2, 10).submenu(VM_POWER_SUBMENU);
    pub const VM_SHUT_DOWN: MenuPath = at(Menu::Vm, 2, 20).submenu(VM_POWER_SUBMENU);
    pub const VM_REBOOT: MenuPath = at(Menu::Vm, 2, 30).submenu(VM_POWER_SUBMENU);
    pub const VM_SUSPEND: MenuPath = at(Menu::Vm, 2, 40).submenu(VM_POWER_SUBMENU);
    pub const VM_RESUME: MenuPath = at(Menu::Vm, 2, 50).submenu(VM_POWER_SUBMENU);
    /// Pause and Unpause stay (AREAS.md, decision 2).
    pub const VM_PAUSE: MenuPath = at(Menu::Vm, 2, 60).submenu(VM_POWER_SUBMENU);
    pub const VM_UNPAUSE: MenuPath = at(Menu::Vm, 2, 70).submenu(VM_POWER_SUBMENU);
    pub const VM_FORCE_SHUT_DOWN: MenuPath = at(Menu::Vm, 2, 80).submenu(VM_POWER_SUBMENU);
    pub const VM_FORCE_REBOOT: MenuPath = at(Menu::Vm, 2, 90).submenu(VM_POWER_SUBMENU);
    pub const VM_RECOVERY_START: MenuPath = at(Menu::Vm, 2, 100).submenu(VM_POWER_SUBMENU);
    /// Present only for a VM whose finished recovery boot left a boot order
    /// to restore; directly under Start in Recovery Mode.
    pub const VM_RECOVERY_RESTORE_BOOT: MenuPath = at(Menu::Vm, 2, 105).submenu(VM_POWER_SUBMENU);
    pub const VM_START_ON_SERVER: MenuPath = at(Menu::Vm, 2, 120);
    pub const VM_RESUME_ON_SERVER: MenuPath = at(Menu::Vm, 2, 130);
    pub const VM_MIGRATE: MenuPath = at(Menu::Vm, 3, 10);
    pub const VM_MOVE: MenuPath = at(Menu::Vm, 3, 20);
    pub const VM_COPY: MenuPath = at(Menu::Vm, 3, 30);
    pub const VM_SNAPSHOT: MenuPath = at(Menu::Vm, 4, 10);
    pub const VM_ASSIGN_SNAPSHOT_SCHEDULE: MenuPath = at(Menu::Vm, 4, 20);
    pub const VM_ASSIGN_VAPP: MenuPath = at(Menu::Vm, 4, 30);
    pub const VM_CONVERT_TO_TEMPLATE: MenuPath = at(Menu::Vm, 5, 10);
    pub const VM_EXPORT: MenuPath = at(Menu::Vm, 5, 20);
    pub const VM_INSTALL_GUEST_TOOLS: MenuPath = at(Menu::Vm, 6, 10);
    pub const VM_SEND_CTRL_ALT_DEL: MenuPath = at(Menu::Vm, 6, 20);
    pub const VM_DELETE: MenuPath = at(Menu::Vm, 7, 10);
    pub const VM_PROPERTIES: MenuPath = at(Menu::Vm, 9, 10);

    pub const STORAGE_NEW: MenuPath = at(Menu::Storage, 1, 10);
    pub const STORAGE_REPAIR: MenuPath = at(Menu::Storage, 2, 10);
    pub const STORAGE_SET_DEFAULT: MenuPath = at(Menu::Storage, 2, 20);
    pub const STORAGE_NEW_VIRTUAL_DISK: MenuPath =
        at(Menu::Storage, 3, 10).submenu("Virtual Disks");
    pub const STORAGE_ATTACH_VIRTUAL_DISK: MenuPath =
        at(Menu::Storage, 3, 20).submenu("Virtual Disks");
    pub const STORAGE_DETACH: MenuPath = at(Menu::Storage, 4, 10);
    pub const STORAGE_REATTACH: MenuPath = at(Menu::Storage, 4, 20);
    pub const STORAGE_FORGET: MenuPath = at(Menu::Storage, 4, 30);
    pub const STORAGE_DESTROY: MenuPath = at(Menu::Storage, 4, 40);
    pub const STORAGE_PROPERTIES: MenuPath = at(Menu::Storage, 9, 10);

    pub const TEMPLATES_NEW_VM: MenuPath = at(Menu::Templates, 1, 10);
    pub const TEMPLATES_CREATE_FROM_SELECTION: MenuPath = at(Menu::Templates, 1, 20);
    pub const TEMPLATES_EXPORT: MenuPath = at(Menu::Templates, 2, 10);
    pub const TEMPLATES_COPY: MenuPath = at(Menu::Templates, 2, 20);
    pub const TEMPLATES_DELETE: MenuPath = at(Menu::Templates, 3, 10);
    pub const TEMPLATES_PROPERTIES: MenuPath = at(Menu::Templates, 9, 10);

    pub const TOOLS_OPTIONS: MenuPath = at(Menu::Tools, 1, 10);
    pub const TOOLS_EXTERNAL_TOOLS: MenuPath = at(Menu::Tools, 2, 10);
    pub const TOOLS_CONVERT_DISK_IMAGE: MenuPath = at(Menu::Tools, 2, 20);
    pub const TOOLS_SERVER_STATUS_REPORT: MenuPath = at(Menu::Tools, 3, 10);
    pub const TOOLS_INVENTORY_REPORT: MenuPath = at(Menu::Tools, 3, 20);

    /// The entry of console window `slot` (1 to 8) in the Window menu.
    pub const fn window_console(slot: u8) -> MenuPath {
        at(Menu::Window, 1, slot)
    }
    pub const WINDOW_CLOSE_ALL_CONSOLES: MenuPath = at(Menu::Window, 2, 10);

    pub const HELP_HELP: MenuPath = at(Menu::Help, 1, 10);
    pub const HELP_WEBSITE: MenuPath = at(Menu::Help, 2, 10);
    pub const HELP_ABOUT: MenuPath = at(Menu::Help, 9, 10);
}

/// The groups of the toolbar, left to right.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolbarGroup {
    /// Back, Forward.
    Navigation,
    /// Add Server, New Storage, New VM.
    New,
    /// The power commands. A power command is shown only when it is enabled.
    Power,
}

/// Where a command sits in the toolbar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct ToolbarSlot {
    pub group: ToolbarGroup,
    pub order: u8,
}

impl ToolbarSlot {
    pub const fn new(group: ToolbarGroup, order: u8) -> Self {
        Self { group, order }
    }

    pub const BACK: Self = Self::new(ToolbarGroup::Navigation, 10);
    pub const FORWARD: Self = Self::new(ToolbarGroup::Navigation, 20);
    pub const ADD_SERVER: Self = Self::new(ToolbarGroup::New, 10);
    pub const NEW_STORAGE: Self = Self::new(ToolbarGroup::New, 30);
    pub const NEW_VM: Self = Self::new(ToolbarGroup::New, 40);
    pub const START: Self = Self::new(ToolbarGroup::Power, 10);
    pub const SHUT_DOWN: Self = Self::new(ToolbarGroup::Power, 20);
    pub const REBOOT: Self = Self::new(ToolbarGroup::Power, 30);
    pub const RESUME: Self = Self::new(ToolbarGroup::Power, 40);
    pub const SUSPEND: Self = Self::new(ToolbarGroup::Power, 50);
    pub const FORCE_SHUT_DOWN: Self = Self::new(ToolbarGroup::Power, 60);
    pub const FORCE_REBOOT: Self = Self::new(ToolbarGroup::Power, 70);
}

/// The tab whose local action row shows a command, and its position there.
/// The command's target is the grid selection of the tab when there is one,
/// otherwise the tree scope; return `Hidden` from the eligibility for a
/// scope the command does not belong to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct LocalRow {
    pub tab: TabId,
    pub order: u8,
}

impl LocalRow {
    /// How many tabs a command can be on beyond its first.
    pub const MORE: usize = 3;

    pub const fn new(tab: TabId, order: u8) -> Self {
        Self { tab, order }
    }
}

/// One command.
///
/// Build it with [`CommandSpec::new`] and the chained setters, which are
/// `const`, or as a struct literal over `..CommandSpec::new(..)`.
#[derive(Clone, Copy)]
pub struct CommandSpec {
    pub id: CommandId,
    pub label: &'static str,
    /// Menu, group within it, optional submenu.
    pub menu: Option<MenuPath>,
    pub toolbar: Option<ToolbarSlot>,
    /// The object kinds whose context menu shows the command.
    pub context: &'static [ObjectKind],
    /// The tab whose local action row shows the command: the first
    /// [`CommandSpec::local`] call. Read [`CommandSpec::local_rows`], which
    /// has every tab.
    pub local: Option<LocalRow>,
    /// The further tabs whose local action row shows the command: the
    /// second and later [`CommandSpec::local`] calls.
    pub local_more: [Option<LocalRow>; LocalRow::MORE],
    pub shortcut: Option<&'static str>,
    /// The command accepts several objects. A command that does not is
    /// left out of the context menu of a multi-selection.
    pub multi: bool,
    pub eligibility: fn(&Model, &Target) -> Eligibility,
    /// Runs the command: usually it sets the area's editor, which opens the
    /// surface. Called only when the eligibility is `Enabled`.
    pub invoke: fn(&mut Model, &Target),
    pub area: &'static str,
    /// For a toggle (View > Toolbar): whether it is on. `None` for a command
    /// that is not a toggle.
    pub checked: Option<fn(&Model) -> bool>,
    /// A label that depends on the state ("Exit Maintenance Mode", the name
    /// of a console window). Returning `None` keeps `label`.
    pub dynamic_label: Option<fn(&Model, &Target) -> Option<String>>,
}

fn always_enabled(_: &Model, _: &Target) -> Eligibility {
    Eligibility::Enabled
}

fn do_nothing(_: &mut Model, _: &Target) {}

impl CommandSpec {
    /// A command that is always enabled, does nothing and appears nowhere.
    pub const fn new(id: &'static str, label: &'static str, area: &'static str) -> Self {
        Self {
            id: CommandId(id),
            label,
            menu: None,
            toolbar: None,
            context: &[],
            local: None,
            local_more: [None; LocalRow::MORE],
            shortcut: None,
            multi: false,
            eligibility: always_enabled,
            invoke: do_nothing,
            area,
            checked: None,
            dynamic_label: None,
        }
    }

    pub const fn menu(mut self, path: MenuPath) -> Self {
        self.menu = Some(path);
        self
    }

    pub const fn toolbar(mut self, slot: ToolbarSlot) -> Self {
        self.toolbar = Some(slot);
        self
    }

    pub const fn context(mut self, kinds: &'static [ObjectKind]) -> Self {
        self.context = kinds;
        self
    }

    /// Puts the command on the local action row of a tab. Call it once per
    /// tab for a command that belongs to several (the export of Alerts and
    /// Events): at most `1 + LocalRow::MORE` tabs; a call beyond that
    /// fails the build of a `const` spec and panics otherwise.
    pub const fn local(mut self, tab: TabId, order: u8) -> Self {
        let row = Some(LocalRow::new(tab, order));
        if self.local.is_none() {
            self.local = row;
            return self;
        }
        let mut index = 0;
        while index < LocalRow::MORE {
            if self.local_more[index].is_none() {
                self.local_more[index] = row;
                return self;
            }
            index += 1;
        }
        panic!("a command is on the local action row of too many tabs");
    }

    /// Every tab whose local action row shows the command, in the order of
    /// the [`CommandSpec::local`] calls.
    pub fn local_rows(&self) -> impl Iterator<Item = LocalRow> + '_ {
        self.local
            .into_iter()
            .chain(self.local_more.iter().copied().flatten())
    }

    pub const fn shortcut(mut self, keys: &'static str) -> Self {
        self.shortcut = Some(keys);
        self
    }

    pub const fn multi(mut self) -> Self {
        self.multi = true;
        self
    }

    pub const fn eligibility(mut self, check: fn(&Model, &Target) -> Eligibility) -> Self {
        self.eligibility = check;
        self
    }

    pub const fn invoke(mut self, run: fn(&mut Model, &Target)) -> Self {
        self.invoke = run;
        self
    }

    pub const fn checked(mut self, state: fn(&Model) -> bool) -> Self {
        self.checked = Some(state);
        self
    }

    pub const fn dynamic_label(mut self, label: fn(&Model, &Target) -> Option<String>) -> Self {
        self.dynamic_label = Some(label);
        self
    }
}

impl fmt::Debug for CommandSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CommandSpec")
            .field("id", &self.id.0)
            .field("label", &self.label)
            .field("menu", &self.menu)
            .field("area", &self.area)
            .finish_non_exhaustive()
    }
}

/// The area name of the commands and scenes the kernel owns.
pub const KERNEL: &str = "kernel";

fn exit(m: &mut Model, _: &Target) {
    m.send(Intent::Quit);
}

fn can_go_back(m: &Model, _: &Target) -> Eligibility {
    Eligibility::enabled_if(
        m.can_go_back(),
        "There is no earlier selection to go back to.",
    )
}

fn can_go_forward(m: &Model, _: &Target) -> Eligibility {
    Eligibility::enabled_if(
        m.can_go_forward(),
        "There is no later selection to go forward to.",
    )
}

fn can_grow(m: &Model, _: &Target) -> Eligibility {
    let options = m.preferences.options;
    Eligibility::enabled_if(
        options.scale_step(true) != options.interface_scale,
        "The interface is at its largest size.",
    )
}

fn can_shrink(m: &Model, _: &Target) -> Eligibility {
    let options = m.preferences.options;
    Eligibility::enabled_if(
        options.scale_step(false) != options.interface_scale,
        "The interface is at its smallest size.",
    )
}

fn can_reset_size(m: &Model, _: &Target) -> Eligibility {
    Eligibility::enabled_if(
        m.preferences.options.interface_scale != 100,
        "The interface is at its normal size.",
    )
}

/// The kinds whose objects have a UUID to copy.
const WITH_UUID: &[ObjectKind] = &[
    ObjectKind::Pool,
    ObjectKind::Host,
    ObjectKind::Vm,
    ObjectKind::Template,
    ObjectKind::StorageRepository,
    ObjectKind::Network,
    ObjectKind::VirtualDisk,
    ObjectKind::Snapshot,
    ObjectKind::Appliance,
];

fn has_uuid(_: &Model, target: &Target) -> Eligibility {
    match target.single() {
        Some(object) if WITH_UUID.contains(&object.kind) && object.uuid().is_some() => {
            Eligibility::Enabled
        }
        _ => Eligibility::Hidden,
    }
}

fn copy_uuid(m: &mut Model, target: &Target) {
    if let Some(object) = target.single() {
        m.copy_text(object.id.clone());
    }
}

/// The commands the kernel owns: Back and Forward, the View toggles, the
/// interface size, Copy UUID, Exit, and the Properties commands
/// ([`crate::properties`]).
///
/// The interface size commands are the Rune `app_key`: Ctrl with plus (or
/// equals), minus and 0. A command has one shortcut, so the shell binds
/// `ctrl-+` to `view.zoom-in` as well, and delivers none of the three while
/// a console has the keyboard.
fn kernel_commands() -> Vec<CommandSpec> {
    let mut commands = shell_commands();
    commands.extend(crate::properties::commands());
    commands
}

fn shell_commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("nav.back", "Back", KERNEL)
            .toolbar(ToolbarSlot::BACK)
            .shortcut("alt-left")
            .eligibility(can_go_back)
            .invoke(|m, _| {
                m.back();
            }),
        CommandSpec::new("nav.forward", "Forward", KERNEL)
            .toolbar(ToolbarSlot::FORWARD)
            .shortcut("alt-right")
            .eligibility(can_go_forward)
            .invoke(|m, _| {
                m.forward();
            }),
        CommandSpec::new("view.custom-templates", "Custom Templates", KERNEL)
            .menu(place::VIEW_CUSTOM_TEMPLATES)
            .checked(|m| m.nav.show_custom_templates)
            .invoke(|m, _| m.nav.show_custom_templates = !m.nav.show_custom_templates),
        CommandSpec::new("view.builtin-templates", "Built-in Templates", KERNEL)
            .menu(place::VIEW_BUILTIN_TEMPLATES)
            .checked(|m| m.nav.show_builtin_templates)
            .invoke(|m, _| m.nav.show_builtin_templates = !m.nav.show_builtin_templates),
        CommandSpec::new("view.local-storage", "Local Storage", KERNEL)
            .menu(place::VIEW_LOCAL_STORAGE)
            .checked(|m| m.nav.show_local_storage)
            .invoke(|m, _| m.nav.show_local_storage = !m.nav.show_local_storage),
        CommandSpec::new("view.toolbar", "Toolbar", KERNEL)
            .menu(place::VIEW_TOOLBAR)
            .checked(|m| m.nav.show_toolbar)
            .invoke(|m, _| m.nav.show_toolbar = !m.nav.show_toolbar),
        CommandSpec::new("view.zoom-in", "Larger Interface", KERNEL)
            .menu(place::VIEW_LARGER)
            .shortcut("ctrl-=")
            .eligibility(can_grow)
            .invoke(|m, _| {
                let scale = m.preferences.options.scale_step(true);
                m.set_interface_scale(scale);
            }),
        CommandSpec::new("view.zoom-out", "Smaller Interface", KERNEL)
            .menu(place::VIEW_SMALLER)
            .shortcut("ctrl--")
            .eligibility(can_shrink)
            .invoke(|m, _| {
                let scale = m.preferences.options.scale_step(false);
                m.set_interface_scale(scale);
            }),
        CommandSpec::new("view.zoom-reset", "Reset Interface Size", KERNEL)
            .menu(place::VIEW_RESET_SIZE)
            .shortcut("ctrl-0")
            .eligibility(can_reset_size)
            .invoke(|m, _| m.set_interface_scale(100)),
        CommandSpec::new("object.copy-uuid", "Copy UUID", KERNEL)
            .context(WITH_UUID)
            .eligibility(has_uuid)
            .invoke(copy_uuid),
        CommandSpec::new("file.exit", "Exit", KERNEL)
            .menu(place::FILE_EXIT)
            .shortcut("ctrl-q")
            .invoke(exit),
    ]
}

/// Every command: the kernel's, then each area's in the order of
/// [`crate::areas::specs`]. Built once.
pub fn registry() -> &'static [CommandSpec] {
    static REGISTRY: OnceLock<Vec<CommandSpec>> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        let mut commands = kernel_commands();
        for area in crate::areas::specs() {
            commands.extend((area.commands)());
        }
        commands
    })
}

/// The command with this id.
pub fn find(id: CommandId) -> Option<&'static CommandSpec> {
    registry().iter().find(|spec| spec.id == id)
}

/// The eligibility of a command on a target. `Hidden` for an unknown id.
pub fn eligibility(m: &Model, id: CommandId, target: &Target) -> Eligibility {
    find(id).map_or(Eligibility::Hidden, |spec| (spec.eligibility)(m, target))
}

/// Why [`Model::try_invoke`] did not run a command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refused {
    /// No command has this id.
    Unknown(CommandId),
    /// The command is hidden for the target.
    Hidden(CommandId),
    /// The command is disabled, with its reason.
    Disabled(String),
}

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown(id) => write!(f, "There is no command {id}."),
            Self::Hidden(id) => write!(f, "The command {id} is not available for this selection."),
            Self::Disabled(reason) => f.write_str(reason),
        }
    }
}

// ---------------------------------------------------------------------------
// Resolved views: what the front end renders.
// ---------------------------------------------------------------------------

/// One command as a menu item, a toolbar button, a context menu item or a
/// button of a local action row. Its key is the command id.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MenuItem {
    pub id: CommandId,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shortcut: Option<&'static str>,
    /// The reason the item is disabled. The tooltip.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled: Option<String>,
    /// The state of a toggle.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checked: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MenuEntry {
    Item(MenuItem),
    Submenu {
        label: &'static str,
        items: Vec<MenuItem>,
    },
}

impl MenuEntry {
    /// The items of the entry: itself, or those of the submenu.
    pub fn items(&self) -> &[MenuItem] {
        match self {
            Self::Item(item) => std::slice::from_ref(item),
            Self::Submenu { items, .. } => items,
        }
    }
}

/// One menu of the menu bar. `groups` are separated by separators. A menu
/// with no visible item has no group.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MenuView {
    pub menu: Menu,
    pub title: &'static str,
    pub groups: Vec<Vec<MenuEntry>>,
}

impl MenuView {
    /// Every item of the menu, submenus included, in order.
    pub fn items(&self) -> impl Iterator<Item = &MenuItem> {
        self.groups
            .iter()
            .flatten()
            .flat_map(|entry| entry.items().iter())
    }
}

fn resolve_spec(m: &Model, spec: &CommandSpec, target: &Target) -> Option<MenuItem> {
    let disabled = match (spec.eligibility)(m, target) {
        Eligibility::Hidden => return None,
        Eligibility::Enabled => None,
        Eligibility::Disabled(reason) => Some(reason),
    };
    Some(MenuItem {
        id: spec.id,
        label: spec
            .dynamic_label
            .and_then(|label| label(m, target))
            .unwrap_or_else(|| spec.label.to_owned()),
        shortcut: spec.shortcut,
        disabled,
        checked: spec.checked.map(|checked| checked(m)),
    })
}

fn menu_bar_of(specs: &[CommandSpec], m: &Model, target: &Target) -> Vec<MenuView> {
    Menu::ALL
        .into_iter()
        .map(|menu| {
            let mut placed: Vec<(MenuPath, &CommandSpec)> = specs
                .iter()
                .filter_map(|spec| spec.menu.map(|path| (path, spec)))
                .filter(|(path, _)| path.menu == menu)
                .collect();
            placed.sort_by_key(|(path, _)| (path.group, path.order));
            let mut groups: Vec<(u8, Vec<MenuEntry>)> = Vec::new();
            for (path, spec) in placed {
                let Some(item) = resolve_spec(m, spec, target) else {
                    continue;
                };
                if groups.last().is_none_or(|(group, _)| *group != path.group) {
                    groups.push((path.group, Vec::new()));
                }
                let entries = &mut groups.last_mut().expect("a group was pushed").1;
                match path.submenu {
                    None => entries.push(MenuEntry::Item(item)),
                    Some(name) => {
                        let existing = entries.iter_mut().find_map(|entry| match entry {
                            MenuEntry::Submenu { label, items } if *label == name => Some(items),
                            _ => None,
                        });
                        match existing {
                            Some(items) => items.push(item),
                            None => entries.push(MenuEntry::Submenu {
                                label: name,
                                items: vec![item],
                            }),
                        }
                    }
                }
            }
            MenuView {
                menu,
                title: menu.title(),
                groups: groups.into_iter().map(|(_, entries)| entries).collect(),
            }
        })
        .collect()
}

fn toolbar_of(specs: &[CommandSpec], m: &Model, target: &Target) -> Vec<Vec<MenuItem>> {
    let mut placed: Vec<(ToolbarSlot, &CommandSpec)> = specs
        .iter()
        .filter_map(|spec| spec.toolbar.map(|slot| (slot, spec)))
        .collect();
    placed.sort_by_key(|(slot, _)| (slot.group, slot.order));
    let mut groups: Vec<(ToolbarGroup, Vec<MenuItem>)> = Vec::new();
    for (slot, spec) in placed {
        let Some(item) = resolve_spec(m, spec, target) else {
            continue;
        };
        if slot.group == ToolbarGroup::Power && item.disabled.is_some() {
            continue;
        }
        if groups.last().is_none_or(|(group, _)| *group != slot.group) {
            groups.push((slot.group, Vec::new()));
        }
        groups.last_mut().expect("a group was pushed").1.push(item);
    }
    groups.into_iter().map(|(_, items)| items).collect()
}

fn in_context(spec: &CommandSpec, target: &Target) -> bool {
    match target.objects.as_slice() {
        [] => spec.context.contains(&ObjectKind::None),
        [object] => spec.context.contains(&object.kind),
        objects => {
            spec.multi
                && objects
                    .iter()
                    .all(|object| spec.context.contains(&object.kind))
        }
    }
}

fn context_menu_of(specs: &[CommandSpec], m: &Model, target: &Target) -> Vec<Vec<MenuItem>> {
    // Commands with a menu position keep the order of the menu bar; the
    // others follow in registry order as a last group.
    type Group = Option<(Menu, u8)>;
    type Position = (u8, Group, u8);
    let mut placed: Vec<(Position, &CommandSpec)> = specs
        .iter()
        .filter(|spec| in_context(spec, target))
        .map(|spec| match spec.menu {
            Some(path) => ((0, Some((path.menu, path.group)), path.order), spec),
            None => ((1, None, 0), spec),
        })
        .collect();
    placed.sort_by_key(|(position, _)| *position);
    let mut groups: Vec<(Group, Vec<MenuItem>)> = Vec::new();
    for ((_, group, _), spec) in placed {
        let Some(item) = resolve_spec(m, spec, target) else {
            continue;
        };
        if groups.last().is_none_or(|(last, _)| *last != group) {
            groups.push((group, Vec::new()));
        }
        groups.last_mut().expect("a group was pushed").1.push(item);
    }
    groups.into_iter().map(|(_, items)| items).collect()
}

fn local_row_of(specs: &[CommandSpec], m: &Model, tab: TabId, target: &Target) -> Vec<MenuItem> {
    let mut placed: Vec<(u8, &CommandSpec)> = specs
        .iter()
        .filter_map(|spec| {
            spec.local_rows()
                .find(|row| row.tab == tab)
                .map(|row| (row.order, spec))
        })
        .collect();
    placed.sort_by_key(|(order, _)| *order);
    placed
        .into_iter()
        .filter_map(|(_, spec)| resolve_spec(m, spec, target))
        .collect()
}

/// The menu bar for the model's current target: always the ten menus, in the
/// plan's order. Hidden commands are omitted; disabled ones carry their
/// reason.
pub fn menu_bar(m: &Model) -> Vec<MenuView> {
    menu_bar_of(registry(), m, &m.target())
}

/// The toolbar for the model's current target, as groups left to right. A
/// power command is present only when it is enabled.
pub fn toolbar(m: &Model) -> Vec<Vec<MenuItem>> {
    toolbar_of(registry(), m, &m.target())
}

/// The context menu of `target`, as groups. For several objects only the
/// commands marked `multi` whose kinds cover every object are offered.
/// Submenus are flattened.
pub fn context_menu(m: &Model, target: &Target) -> Vec<Vec<MenuItem>> {
    context_menu_of(registry(), m, target)
}

/// The local action row of a tab for the model's current target.
pub fn local_row(m: &Model, tab: TabId) -> Vec<MenuItem> {
    local_row_of(registry(), m, tab, &m.target())
}

/// The given commands as menu items for `target`, in the given order: the
/// row menu of a table. Hidden and unknown commands are omitted.
pub fn resolve(m: &Model, ids: &[CommandId], target: &Target) -> Vec<MenuItem> {
    ids.iter()
        .filter_map(|id| find(*id))
        .filter_map(|spec| resolve_spec(m, spec, target))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::ObjectRef;
    use xenadmin_core::{PoolId, VmId};

    fn hidden(_: &Model, _: &Target) -> Eligibility {
        Eligibility::Hidden
    }

    fn not_in_demo(m: &Model, _: &Target) -> Eligibility {
        Eligibility::enabled_if(!m.demo, "Demo mode cannot change infrastructure.")
    }

    fn vm_only(_: &Model, target: &Target) -> Eligibility {
        if target.all_of(ObjectKind::Vm) {
            Eligibility::Enabled
        } else {
            Eligibility::Hidden
        }
    }

    fn toolbar_shown(m: &Model) -> bool {
        !m.demo
    }

    fn maintenance_label(m: &Model, _: &Target) -> Option<String> {
        m.demo.then(|| "Exit Maintenance Mode".to_owned())
    }

    const VM: &[ObjectKind] = &[ObjectKind::Vm];
    const VM_OR_TEMPLATE: &[ObjectKind] = &[ObjectKind::Vm, ObjectKind::Template];

    fn specs() -> Vec<CommandSpec> {
        vec![
            CommandSpec::new("vm.properties", "Properties", "t")
                .menu(place::VM_PROPERTIES)
                .context(VM_OR_TEMPLATE),
            CommandSpec::new("vm.delete", "Delete VM", "t")
                .menu(place::VM_DELETE)
                .context(VM_OR_TEMPLATE)
                .multi()
                .eligibility(not_in_demo),
            CommandSpec::new("vm.force-reboot", "Force Reboot", "t")
                .menu(place::VM_FORCE_REBOOT)
                .toolbar(ToolbarSlot::FORCE_REBOOT)
                .context(VM)
                .multi()
                .eligibility(not_in_demo),
            CommandSpec::new("vm.start", "Start", "t")
                .menu(place::VM_START)
                .toolbar(ToolbarSlot::START)
                .context(VM)
                .multi(),
            CommandSpec::new("vm.start-on", "Start on Server", "t")
                .menu(place::VM_START_ON_SERVER)
                .context(VM)
                .eligibility(hidden),
            CommandSpec::new("vm.new", "New VM", "t")
                .menu(place::VM_NEW)
                .toolbar(ToolbarSlot::NEW_VM)
                .shortcut("ctrl-n")
                .eligibility(not_in_demo),
            CommandSpec::new("nav.back", "Back", "t").toolbar(ToolbarSlot::BACK),
            CommandSpec::new("object.copy-uuid", "Copy UUID", "t").context(VM_OR_TEMPLATE),
            CommandSpec::new("view.toolbar", "Toolbar", "t")
                .menu(place::VIEW_TOOLBAR)
                .checked(toolbar_shown),
            CommandSpec::new("server.maintenance", "Enter Maintenance Mode", "t")
                .menu(place::SERVER_MAINTENANCE)
                .dynamic_label(maintenance_label),
            CommandSpec::new("disk.detach", "Detach", "t")
                .local(TabId::Storage, 20)
                .eligibility(vm_only),
            CommandSpec::new("disk.add", "Add", "t").local(TabId::Storage, 10),
            CommandSpec::new("nic.add", "Add Interface", "t").local(TabId::Networking, 10),
        ]
    }

    fn pool() -> PoolId {
        PoolId::parse("10000000-0000-0000-0000-000000000001").expect("pool id")
    }

    fn vm(n: u8) -> ObjectRef {
        ObjectRef::vm(
            pool(),
            VmId::parse(&format!("30000000-0000-0000-0000-0000000000{n:02}")).expect("vm id"),
        )
    }

    fn labels(items: &[MenuItem]) -> Vec<&str> {
        items.iter().map(|item| item.label.as_str()).collect()
    }

    fn flat(groups: Vec<Vec<MenuItem>>) -> Vec<MenuItem> {
        groups.into_iter().flatten().collect()
    }

    #[test]
    fn the_menu_bar_has_the_ten_menus_in_the_plans_order() {
        let m = Model::fixture();
        let menus = menu_bar_of(&specs(), &m, &Target::none());
        let titles: Vec<_> = menus.iter().map(|menu| menu.title).collect();
        assert_eq!(
            titles,
            [
                "File",
                "View",
                "Pool",
                "Server",
                "VM",
                "Storage",
                "Templates",
                "Tools",
                "Window",
                "Help"
            ]
        );
        assert!(menus[0].groups.is_empty(), "a menu without items is empty");
    }

    #[test]
    fn a_menu_orders_groups_and_items_and_collects_a_submenu() {
        let m = Model::fixture();
        let menus = menu_bar_of(&specs(), &m, &Target::none());
        let vm = &menus[4];
        assert_eq!(vm.menu, Menu::Vm);
        // New VM | Start/Shut Down > (Start, Force Reboot) | Delete VM | Properties.
        // Start on Server is hidden and leaves no trace.
        assert_eq!(vm.groups.len(), 4);
        assert!(matches!(&vm.groups[0][..], [MenuEntry::Item(item)] if item.label == "New VM"));
        match &vm.groups[1][..] {
            [MenuEntry::Submenu { label, items }] => {
                assert_eq!(*label, "Start/Shut Down");
                assert_eq!(labels(items), ["Start", "Force Reboot"]);
            }
            other => panic!("unexpected power group: {other:?}"),
        }
        assert_eq!(
            labels(&vm.items().cloned().collect::<Vec<_>>()),
            ["New VM", "Start", "Force Reboot", "Delete VM", "Properties"]
        );
        let new_vm = vm.items().next().expect("New VM");
        assert_eq!(new_vm.shortcut, Some("ctrl-n"));
        assert_eq!(new_vm.disabled, None);
    }

    #[test]
    fn a_disabled_item_carries_its_reason_and_toggles_and_labels_follow_the_model() {
        let mut m = Model::fixture();
        let item = |m: &Model, menu: usize, id: &str| {
            menu_bar_of(&specs(), m, &Target::none())[menu]
                .items()
                .find(|item| item.id.0 == id)
                .cloned()
                .expect("item")
        };
        assert_eq!(item(&m, 1, "view.toolbar").checked, Some(true));
        assert_eq!(item(&m, 4, "vm.new").checked, None);
        assert_eq!(
            item(&m, 3, "server.maintenance").label,
            "Enter Maintenance Mode"
        );
        m.demo = true;
        assert_eq!(
            item(&m, 4, "vm.new").disabled.as_deref(),
            Some("Demo mode cannot change infrastructure.")
        );
        assert_eq!(item(&m, 1, "view.toolbar").checked, Some(false));
        assert_eq!(
            item(&m, 3, "server.maintenance").label,
            "Exit Maintenance Mode"
        );
    }

    #[test]
    fn the_toolbar_groups_commands_and_shows_a_power_command_only_when_enabled() {
        let mut m = Model::fixture();
        let bar = toolbar_of(&specs(), &m, &Target::none());
        assert_eq!(bar.len(), 3);
        assert_eq!(labels(&bar[0]), ["Back"]);
        assert_eq!(labels(&bar[1]), ["New VM"]);
        assert_eq!(labels(&bar[2]), ["Start", "Force Reboot"]);
        m.demo = true;
        let bar = toolbar_of(&specs(), &m, &Target::none());
        // New VM stays, disabled with its reason; Force Reboot leaves the bar.
        assert!(bar[1][0].disabled.is_some());
        assert_eq!(labels(&bar[2]), ["Start"]);
    }

    #[test]
    fn a_context_menu_follows_the_kind_and_a_multi_selection_keeps_only_bulk_commands() {
        let m = Model::fixture();
        let one = context_menu_of(&specs(), &m, &Target::one(vm(1)));
        assert_eq!(
            one.len(),
            4,
            "power, delete, properties, commands without a menu"
        );
        assert_eq!(
            labels(&flat(one)),
            [
                "Start",
                "Force Reboot",
                "Delete VM",
                "Properties",
                "Copy UUID"
            ]
        );
        let many = context_menu_of(&specs(), &m, &Target::many([vm(1), vm(2)]));
        assert_eq!(labels(&flat(many)), ["Start", "Force Reboot", "Delete VM"]);
        let template = ObjectRef::template(pool(), vm(3).vm_id().expect("id"));
        let mixed = context_menu_of(&specs(), &m, &Target::many([vm(1), template.clone()]));
        assert_eq!(labels(&flat(mixed)), ["Delete VM"]);
        let on_template = context_menu_of(&specs(), &m, &Target::one(template));
        assert_eq!(
            labels(&flat(on_template)),
            ["Delete VM", "Properties", "Copy UUID"]
        );
        assert!(context_menu_of(&specs(), &m, &Target::none()).is_empty());
    }

    #[test]
    fn a_local_row_lists_the_commands_of_its_tab_in_order() {
        let m = Model::fixture();
        let on_vm = local_row_of(&specs(), &m, TabId::Storage, &Target::one(vm(1)));
        assert_eq!(labels(&on_vm), ["Add", "Detach"]);
        let on_nothing = local_row_of(&specs(), &m, TabId::Storage, &Target::none());
        assert_eq!(labels(&on_nothing), ["Add"]);
        let networking = local_row_of(&specs(), &m, TabId::Networking, &Target::none());
        assert_eq!(labels(&networking), ["Add Interface"]);
        assert!(local_row_of(&specs(), &m, TabId::General, &Target::none()).is_empty());
    }

    #[test]
    fn registry_ids_are_unique_and_every_command_names_its_area() {
        let mut ids: Vec<_> = registry().iter().map(|spec| spec.id.0).collect();
        let count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), count, "duplicate command id");
        let areas: Vec<_> = crate::areas::specs().iter().map(|area| area.id).collect();
        for spec in registry() {
            assert!(
                spec.area == KERNEL || areas.contains(&spec.area),
                "{} names the unknown area {}",
                spec.id,
                spec.area
            );
            assert!(!spec.label.is_empty(), "{} has no label", spec.id);
        }
    }

    #[test]
    fn exit_is_the_last_item_of_the_file_menu_and_asks_the_front_end_to_quit() {
        let mut m = Model::fixture();
        let file = &menu_bar(&m)[0];
        assert_eq!(file.items().last().map(|item| item.id.0), Some("file.exit"));
        assert_eq!(
            eligibility(&m, CommandId("file.exit"), &Target::none()),
            Eligibility::Enabled
        );
        assert_eq!(
            eligibility(&m, CommandId("missing"), &Target::none()),
            Eligibility::Hidden
        );
        assert!(m.invoke(CommandId("file.exit"), &Target::none()));
        assert!(matches!(m.take_intents().as_slice(), [Intent::Quit]));
        assert_eq!(
            m.try_invoke(CommandId("missing"), &Target::none()),
            Err(Refused::Unknown(CommandId("missing")))
        );
        let items = resolve(
            &m,
            &[CommandId("file.exit"), CommandId("missing")],
            &Target::none(),
        );
        assert_eq!(labels(&items), ["Exit"]);
    }

    #[test]
    fn eligibility_helpers_carry_the_reason() {
        assert!(Eligibility::enabled_if(true, "no").is_enabled());
        assert_eq!(Eligibility::enabled_if(false, "no").reason(), Some("no"));
        assert_eq!(
            Eligibility::enabled_unless(None::<String>),
            Eligibility::Enabled
        );
        assert_eq!(
            Eligibility::enabled_unless(Some("stale")),
            Eligibility::disabled("stale")
        );
        assert!(Eligibility::Hidden.is_hidden());
        assert_eq!(
            Refused::Disabled("The VM is running.".into()).to_string(),
            "The VM is running."
        );
    }
}
