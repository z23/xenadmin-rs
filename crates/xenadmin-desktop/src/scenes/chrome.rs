//! The shell's own fixtures: the window with commands the registry does
//! not hold yet, and the window with nothing connected.
//!
//! The registry of this crate has the commands of the areas that are
//! ported. Until the toolbar and the menus of the plan are all there,
//! `gallery-shell` shows the shell with a representative set of them, so
//! the toolbar with its icons and labels, a menu with a submenu and a
//! disabled item can be looked at. Its commands do nothing.
//!
//! Like the renderer's gallery these are not product scenes.

use std::rc::Rc;

use xenadmin_ui::Model;
use xenadmin_ui::command::{self, CommandId, Menu, MenuEntry, MenuItem, ToolbarSlot};
use xenadmin_ui::object::{ObjectKind, ObjectRef, Target};

use crate::store::Describe;

/// The names of the shell's fixtures.
pub const SCENES: [&str; 2] = ["gallery-shell", "gallery-empty"];

/// The toolbar of the plan, by slot: id, label, and whether the command
/// applies to a running guest.
const TOOLBAR: [(&str, &str, ToolbarSlot, bool); 10] = [
    (
        "gallery.add-server",
        "Add Server…",
        ToolbarSlot::ADD_SERVER,
        true,
    ),
    (
        "gallery.new-storage",
        "New Storage…",
        ToolbarSlot::NEW_STORAGE,
        true,
    ),
    ("gallery.new-vm", "New VM…", ToolbarSlot::NEW_VM, true),
    ("gallery.start", "Start", ToolbarSlot::START, false),
    (
        "gallery.shut-down",
        "Shut Down",
        ToolbarSlot::SHUT_DOWN,
        true,
    ),
    ("gallery.reboot", "Reboot", ToolbarSlot::REBOOT, true),
    ("gallery.resume", "Resume", ToolbarSlot::RESUME, false),
    ("gallery.suspend", "Suspend", ToolbarSlot::SUSPEND, true),
    (
        "gallery.force-shut-down",
        "Force Shut Down",
        ToolbarSlot::FORCE_SHUT_DOWN,
        true,
    ),
    (
        "gallery.force-reboot",
        "Force Reboot",
        ToolbarSlot::FORCE_REBOOT,
        true,
    ),
];

fn item(id: &'static str, label: &str, disabled: Option<&str>) -> MenuItem {
    MenuItem {
        id: CommandId(id),
        label: label.to_owned(),
        shortcut: None,
        disabled: disabled.map(str::to_owned),
        checked: None,
    }
}

fn push(menus: &mut [command::MenuView], menu: Menu, group: Vec<MenuEntry>) {
    if let Some(view) = menus.iter_mut().find(|view| view.menu == menu) {
        view.groups.insert(0, group);
    }
}

/// The descriptions of `gallery-shell`: the registry's menus and toolbar
/// with the fixture's commands added.
pub fn describe() -> Describe {
    let fallback = Describe::default();
    let (slot, invoke) = (fallback.toolbar_slot.clone(), fallback.invoke.clone());
    Describe {
        menu_bar: Rc::new(|m| {
            let mut menus = command::menu_bar(m);
            let guest = m.target().kind() == ObjectKind::Vm;
            push(
                &mut menus,
                Menu::Server,
                vec![MenuEntry::Item(item(
                    "gallery.add-server",
                    "Add Server…",
                    None,
                ))],
            );
            push(
                &mut menus,
                Menu::Storage,
                vec![MenuEntry::Item(item(
                    "gallery.new-storage",
                    "New Storage…",
                    None,
                ))],
            );
            let mut vm = vec![MenuEntry::Item(item("gallery.new-vm", "New VM…", None))];
            if guest {
                vm.push(MenuEntry::Submenu {
                    label: "Start/Shut Down",
                    items: vec![
                        item("gallery.start", "Start", Some("The VM is already running.")),
                        item("gallery.shut-down", "Shut Down", None),
                        item("gallery.reboot", "Reboot", None),
                        item("gallery.suspend", "Suspend", None),
                    ],
                });
            }
            push(&mut menus, Menu::Vm, vm);
            menus
        }),
        toolbar: Rc::new(|m| {
            let mut groups = command::toolbar(m);
            let guest = m.target().kind() == ObjectKind::Vm;
            let entry =
                |(id, label, _, _): &(&'static str, &str, ToolbarSlot, bool)| item(id, label, None);
            groups.push(TOOLBAR[..3].iter().map(entry).collect());
            if guest {
                // A power command is there only while it applies.
                groups.push(
                    TOOLBAR[3..]
                        .iter()
                        .filter(|(_, _, _, applies)| *applies)
                        .map(entry)
                        .collect(),
                );
            }
            groups
        }),
        toolbar_slot: Rc::new(move |id| {
            TOOLBAR
                .iter()
                .find(|(other, _, _, _)| *other == id.0)
                .map(|(_, _, slot, _)| *slot)
                .or_else(|| slot(id))
        }),
        invoke: Rc::new(move |m, id| {
            if !id.0.starts_with("gallery.") {
                invoke(m, id);
            }
        }),
        ..fallback
    }
}

/// The model of `gallery-shell`: the demo pool with a running guest
/// selected and a step of history behind it.
pub fn shell_model() -> Model {
    let mut model = Model::demo(3, 48);
    let guest = model.inventory.as_ref().and_then(|inventory| {
        let mut guests: Vec<_> = inventory
            .guests()
            .filter(|vm| vm.name == "Database")
            .collect();
        guests.sort_by_key(|vm| vm.id.to_string());
        guests
            .first()
            .map(|vm| ObjectRef::vm(inventory.pool_id, vm.id))
    });
    if let Some(guest) = guest {
        model.select(Target::one(guest));
    }
    model
}
