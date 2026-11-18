//! Tests of the nested context menus over the whole registry: what the menu
//! of each kind of object looks like with every area's commands in it.

use super::*;
use crate::Harness;
use crate::command::outline;

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
    let mut inventory = h.model.inventory.clone().expect("inventory").as_ref().clone();
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
    let storage = ObjectRef::storage(
        pool,
        inventory.storage.first().expect("a repository").uuid,
    );
    let network = ObjectRef::network(
        pool,
        inventory.networks.first().expect("a network").uuid,
    );
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
        for spec in command::registry().iter().filter(|spec| spec.context.contains(&kind)) {
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
