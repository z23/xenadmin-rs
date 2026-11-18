//! Tests of the nested context menus over the whole registry: what the menu
//! of each kind of object looks like with every area's commands in it.

use super::*;
use crate::Harness;
use crate::command::{Menu, outline, place};

/// The demo with one object of each kind that has a long menu.
struct Kinds {
    h: Harness,
    vm: Target,
    vms: Target,
    template: Target,
    host: Target,
    pool: Target,
    storage: Target,
    network: Target,
}

fn kinds() -> Kinds {
    let mut h = Harness::with(Model::demo(3, 48));
    // The demo has no template: its last guest becomes one.
    let mut inventory = h
        .model
        .inventory
        .clone()
        .expect("inventory")
        .as_ref()
        .clone();
    let last = *inventory.vms.keys().next_back().expect("a guest");
    inventory.vms.get_mut(&last).expect("the guest").is_template = true;
    h.model.inventory = Some(Arc::new(inventory));
    let inventory = h.model.inventory.clone().expect("the demo inventory");
    let pool = inventory.pool_id;
    let mut guests = inventory
        .vms
        .values()
        .filter(|vm| !vm.is_template && !vm.is_control_domain)
        .map(|vm| ObjectRef::vm(pool, vm.id));
    let vm = guests.next().expect("a guest");
    let second = guests.next().expect("a second guest");
    let template = inventory
        .vms
        .values()
        .find(|vm| vm.is_template)
        .map(|vm| ObjectRef::template(pool, vm.id))
        .expect("a template");
    let host = ObjectRef::host(pool, *inventory.hosts.keys().next().expect("a host"));
    let storage = ObjectRef::storage(pool, inventory.storage.first().expect("a repository").uuid);
    let network = ObjectRef::network(pool, inventory.networks.first().expect("a network").uuid);
    Kinds {
        h,
        vms: Target::many([vm.clone(), second]),
        vm: Target::one(vm),
        template: Target::one(template),
        host: Target::one(host),
        pool: Target::one(ObjectRef::pool(pool)),
        storage: Target::one(storage),
        network: Target::one(network),
    }
}

impl Kinds {
    fn all(&self) -> [(&'static str, &Target); 7] {
        [
            ("a VM", &self.vm),
            ("several VMs", &self.vms),
            ("a template", &self.template),
            ("a server", &self.host),
            ("a pool", &self.pool),
            ("a repository", &self.storage),
            ("a network", &self.network),
        ]
    }
}

#[test]
#[ignore = "prints the menus: cargo test -p xenadmin-ui print_the_context_menus -- --ignored --nocapture"]
fn print_the_context_menus() {
    let kinds = kinds();
    for (name, target) in kinds.all() {
        let menu = kinds.h.context_entries(target);
        println!(
            "== {name}: {} at the top, {} in all\n{}",
            kinds.h.context_top(target).len(),
            kinds.h.context_menu(target).len(),
            outline(&menu)
        );
    }
}

#[test]
#[ignore = "prints the registry by kind"]
fn print_the_registry_by_kind() {
    use crate::object::ObjectKind;
    for kind in [
        ObjectKind::Vm,
        ObjectKind::Template,
        ObjectKind::Host,
        ObjectKind::Pool,
        ObjectKind::StorageRepository,
        ObjectKind::Network,
        ObjectKind::VirtualDisk,
        ObjectKind::Snapshot,
        ObjectKind::Appliance,
    ] {
        println!("== {kind:?}");
        for spec in command::registry()
            .iter()
            .filter(|spec| spec.context.contains(&kind))
        {
            println!(
                "{:<34} {:<38} {:<18} multi={} menu={:?} sub={:?}",
                spec.id.0,
                spec.label,
                spec.area,
                spec.multi,
                spec.menu.map(|path| (path.menu, path.group, path.order)),
                spec.context_submenu_name()
            );
        }
    }
}

fn ids(items: &[command::MenuItem]) -> Vec<&'static str> {
    items.iter().map(|item| item.id.0).collect()
}

#[test]
fn the_power_commands_and_the_organization_commands_are_submenus_of_a_context_menu() {
    let kinds = kinds();
    let power = kinds.h.context_submenu(&kinds.vm, place::VM_POWER_SUBMENU);
    assert_eq!(
        ids(&power)[..9],
        [
            "vm.start",
            "vm.shutdown",
            "vm.reboot",
            "vm.suspend",
            "vm.resume",
            "vm.pause",
            "vm.unpause",
            "vm.force-shutdown",
            "vm.force-reboot"
        ]
    );
    // The same submenu, with the same commands in the same order, as the
    // VM menu of the menu bar has for this guest.
    let mut h = kinds.h;
    h.select(kinds.vm.clone());
    let bar = command::menu_bar(&h.model);
    let vm_menu = bar.iter().find(|menu| menu.menu == Menu::Vm).expect("VM");
    let in_bar = vm_menu
        .groups
        .iter()
        .flatten()
        .find(|entry| entry.submenu() == Some(place::VM_POWER_SUBMENU))
        .expect("the submenu of the menu bar");
    assert_eq!(ids(in_bar.items()), ids(&power));

    for (name, target) in [
        ("a VM", &kinds.vm),
        ("several VMs", &kinds.vms),
        ("a template", &kinds.template),
        ("a server", &kinds.host),
        ("a repository", &kinds.storage),
        ("a network", &kinds.network),
    ] {
        let organize = h.context_submenu(target, place::CONTEXT_ORGANIZE);
        assert_eq!(
            ids(&organize),
            [
                "org.tag.add",
                "org.tag.remove",
                "org.folder.move",
                "org.field.set",
                "org.field.clear"
            ],
            "{name}"
        );
    }
    // A pool's has Create folder too.
    let organize = h.context_submenu(&kinds.pool, place::CONTEXT_ORGANIZE);
    assert_eq!(organize.len(), 6);
    assert_eq!(organize[5].id.0, "org.folder.create");
    // A server's management interface commands are one entry as well.
    let management = h.context_submenu(&kinds.host, place::CONTEXT_MANAGEMENT_INTERFACE);
    assert!(management.len() >= 5, "{:?}", ids(&management));
    assert!(
        management
            .iter()
            .all(|item| item.id.0.starts_with("server.management."))
    );
}

#[test]
fn no_context_menu_has_a_long_top_level_and_nesting_loses_no_command() {
    let kinds = kinds();
    for (name, target) in kinds.all() {
        let top = kinds.h.context_top(target);
        assert!(
            top.len() <= 20,
            "the menu of {name} has {} entries at its top: {top:?}",
            top.len()
        );
        // Every command the registry offers for the target and does not
        // hide is in the menu once, at the top or in a submenu.
        let mut listed = ids(&kinds.h.context_menu(target));
        let mut offered: Vec<&str> = command::registry()
            .iter()
            .filter(|spec| match target.objects.as_slice() {
                [object] => spec.context.contains(&object.kind),
                objects => {
                    spec.multi
                        && objects
                            .iter()
                            .all(|object| spec.context.contains(&object.kind))
                }
            })
            .filter(|spec| !(spec.eligibility)(&kinds.h.model, target).is_hidden())
            .map(|spec| spec.id.0)
            .collect();
        listed.sort_unstable();
        offered.sort_unstable();
        assert_eq!(listed, offered, "{name}");
        // A submenu is never a single item, and never empty.
        for entry in kinds.h.context_entries(target).iter().flatten() {
            if let Some(label) = entry.submenu() {
                assert!(entry.items().len() > 1, "{name}: {label}");
            }
        }
    }
}

/// The bound of the test below: "about twenty" entries for an object in
/// any one state, counted here with every command of the kind shown at
/// once, which no state does (a VM's count has Export and Delete twice, for
/// one guest and for several, the batch power dialog of several, and both
/// Start on Server and Resume on Server).
const LONGEST_TOP_LEVEL: usize = 26;

#[test]
fn the_registry_keeps_the_top_level_of_every_kind_short() {
    use crate::object::ObjectKind;
    use std::collections::BTreeSet;
    for kind in [
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
    ] {
        let specs: Vec<_> = command::registry()
            .iter()
            .filter(|spec| spec.context.contains(&kind))
            .collect();
        let submenus: BTreeSet<_> = specs
            .iter()
            .filter_map(|spec| spec.context_submenu_name())
            .collect();
        let plain = specs
            .iter()
            .filter(|spec| spec.context_submenu_name().is_none())
            .count();
        let top = plain + submenus.len();
        assert!(
            top <= LONGEST_TOP_LEVEL,
            "the context menu of {kind:?} can have {top} entries at its top level: give the \
             commands of a long run one `.context_submenu(..)` (command.rs)"
        );
    }
}

#[test]
fn an_item_of_a_submenu_is_chosen_from_the_open_menu() {
    use crate::selection::MenuOrigin;
    use crate::testing::Live;
    let mut live = Live::with_vm();
    live.h.model.preferences.options.review_power = true;
    let guest = live.vm();
    // The open menu lists every item, flattened; its submenu holds Start.
    let flat = live.h.open_menu(guest.clone(), MenuOrigin::Tree);
    assert!(flat.iter().any(|item| item.id.0 == "vm.start"));
    assert!(
        !live.h.menu_entries().iter().flatten().any(
            |entry| matches!(entry, command::MenuEntry::Item(item) if item.id.0 == "vm.start")
        ),
        "Start is not at the top level"
    );
    let power = live.h.open_submenu(place::VM_POWER_SUBMENU);
    let start = power
        .iter()
        .find(|item| item.id.0 == "vm.start")
        .expect("Start");
    assert_eq!(start.disabled, None);
    live.h.choose("vm.start");
    assert_eq!(live.h.surface().id, "power-confirm");
    assert!(live.h.model.selection.menu.is_none());
    live.h.dismiss();
}

#[test]
fn a_menu_that_no_longer_stands_disables_the_items_of_its_submenus_too() {
    use crate::selection::MenuOrigin;
    let Kinds { mut h, vm, .. } = kinds();
    let guest = vm.single().cloned().expect("the guest");
    assert!(!h.open_menu(guest.clone(), MenuOrigin::Tree).is_empty());
    let before = h.open_submenu(place::VM_POWER_SUBMENU);
    assert!(
        before
            .iter()
            .all(|item| item.disabled.as_deref() != Some(STALE_MENU))
    );
    // The guest goes while the menu is open.
    let mut inventory = h
        .model
        .inventory
        .clone()
        .expect("inventory")
        .as_ref()
        .clone();
    inventory.vms.remove(&guest.vm_id().expect("a VM"));
    h.model.inventory = Some(Arc::new(inventory));
    assert!(!h.model.context_menu_valid());
    let after = h.open_submenu(place::VM_POWER_SUBMENU);
    assert!(after.len() >= before.len());
    assert!(
        after
            .iter()
            .all(|item| item.disabled.as_deref() == Some(STALE_MENU))
    );
    assert_eq!(h.try_choose("vm.start"), Err(STALE_MENU.to_owned()));
    assert!(h.model.selection.menu.is_none());
}

#[test]
fn the_scene_tree_lists_a_context_menu_with_its_submenus() {
    let h = Harness::scene("inventory-context");
    let tree = h.tree();
    let groups = tree["context_menu"]["items"].as_array().expect("groups");
    let entries: Vec<&serde_json::Value> = groups
        .iter()
        .flat_map(|group| group.as_array().expect("a group"))
        .collect();
    let power = entries
        .iter()
        .find(|entry| entry["submenu"]["label"] == place::VM_POWER_SUBMENU)
        .expect("the power submenu");
    let items = power["submenu"]["items"].as_array().expect("its items");
    assert_eq!(items[0]["id"], "vm.start");
    assert!(items[0]["disabled"].is_string(), "the demo starts nothing");
    assert!(
        entries
            .iter()
            .any(|entry| entry["item"]["id"] == "object.copy-uuid")
    );
    assert!(
        entries
            .iter()
            .any(|entry| entry["submenu"]["label"] == place::CONTEXT_ORGANIZE)
    );
}
