//! Tests of the local action rows over the whole registry: which commands
//! the row of each tab holds for each kind of tree scope.

use super::*;
use crate::Harness;
use crate::command::{CommandSpec, Eligibility, registry};

/// The demo with a template, and one scope of each kind that has tabs.
fn scopes() -> (Harness, Vec<(ObjectKind, Target)>) {
    let mut h = Harness::with(Model::demo(3, 48));
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
    let vm = inventory
        .vms
        .values()
        .find(|vm| !vm.is_template && !vm.is_control_domain)
        .map(|vm| ObjectRef::vm(pool, vm.id))
        .expect("a guest");
    let template = ObjectRef::template(pool, last);
    let host = ObjectRef::host(pool, *inventory.hosts.keys().next().expect("a host"));
    let storage = ObjectRef::storage(pool, inventory.storage.first().expect("a repository").uuid);
    let network = ObjectRef::network(pool, inventory.networks.first().expect("a network").uuid);
    let scopes = vec![
        (ObjectKind::None, Target::none()),
        (ObjectKind::Pool, Target::one(ObjectRef::pool(pool))),
        (ObjectKind::Host, Target::one(host)),
        (ObjectKind::Vm, Target::one(vm)),
        (ObjectKind::Template, Target::one(template)),
        (ObjectKind::StorageRepository, Target::one(storage)),
        (ObjectKind::Network, Target::one(network)),
    ];
    (h, scopes)
}

fn on_row(spec: &CommandSpec, tab: TabId) -> Option<u8> {
    spec.local_rows()
        .find(|row| row.tab == tab)
        .map(|row| row.order)
}

#[test]
#[ignore = "prints the rows: cargo test -p xenadmin-ui print_the_local_rows -- --ignored --nocapture"]
fn print_the_local_rows() {
    let (mut h, scopes) = scopes();
    for (kind, scope) in scopes {
        h.select(scope);
        for tab in h.model.tabs().to_vec() {
            h.open_tab(tab);
            let target = h.model.target();
            let mut row: Vec<(u8, &CommandSpec)> = registry()
                .iter()
                .filter_map(|spec| on_row(spec, tab).map(|order| (order, spec)))
                .collect();
            row.sort_by_key(|(order, _)| *order);
            if row.is_empty() {
                continue;
            }
            println!("== {kind:?} / {tab:?}");
            let show = |eligibility: Eligibility| match eligibility {
                Eligibility::Enabled => "enabled".to_owned(),
                Eligibility::Hidden => "HIDDEN".to_owned(),
                Eligibility::Disabled(reason) => format!("disabled: {reason:.40}"),
            };
            let rows: Vec<Target> = grid_rows(&h.model, tab, kind);
            for (order, spec) in row {
                let scoped = match spec.local_scopes {
                    None => "any".to_owned(),
                    Some(kinds) if kinds.contains(&kind) => "yes".to_owned(),
                    Some(_) => "NO ".to_owned(),
                };
                let state = show((spec.eligibility)(&h.model, &target));
                let mut with_rows = String::new();
                for row in &rows {
                    h.model.select_in_grid(row.clone());
                    let on_row = show((spec.eligibility)(&h.model, &h.model.target()));
                    with_rows.push_str(&format!(" | {:?}: {on_row}", row.kind()));
                    h.model.select_in_grid(Target::none());
                }
                println!(
                    "  {order:>3} {scoped} {:<40} {:<8} {state}{with_rows}",
                    spec.id.0, spec.area
                );
            }
        }
    }
}

/// One target for each kind of object row the grid of a tab can hold.
fn grid_rows(m: &Model, tab: TabId, scope: ObjectKind) -> Vec<Target> {
    use ObjectKind as K;
    let inventory = m.inventory.clone().expect("the demo inventory");
    let pool = inventory.pool_id;
    let kinds: &[ObjectKind] = match (tab, scope) {
        (TabId::Storage, K::Pool | K::Host) => &[K::StorageRepository],
        (TabId::Storage, K::StorageRepository) => &[K::VirtualDisk],
        (TabId::Networking, K::Pool | K::Host | K::Network) => &[K::Network],
        (TabId::Ha, K::Pool) => &[K::Vm],
        _ => &[],
    };
    kinds
        .iter()
        .map(|kind| match kind {
            K::StorageRepository => ObjectRef::storage(pool, inventory.storage[0].uuid),
            K::Network => ObjectRef::network(pool, inventory.networks[0].uuid),
            K::Vm => inventory
                .vms
                .values()
                .find(|vm| !vm.is_template && !vm.is_control_domain)
                .map(|vm| ObjectRef::vm(pool, vm.id))
                .expect("a guest"),
            _ => ObjectRef::new(
                Some(pool),
                K::VirtualDisk,
                inventory.details.vdis.values().next().expect("a disk").id,
            ),
        })
        .map(Target::one)
        .collect()
}

/// The kinds of tree scope that have tabs of their own.
const SCOPES: [ObjectKind; 8] = [
    ObjectKind::None,
    ObjectKind::Pool,
    ObjectKind::Host,
    ObjectKind::Vm,
    ObjectKind::Template,
    ObjectKind::StorageRepository,
    ObjectKind::Network,
    ObjectKind::VirtualDisk,
];

/// The tabs whose row is the same whatever the tree scope is: Search is a
/// tab of every kind, and Alerts and Events are the tabs of the
/// Notifications view, which keeps the scope it was entered with.
const EVERYWHERE: [TabId; 3] = [TabId::Search, TabId::Alerts, TabId::Events];

fn tabs_of(kind: ObjectKind) -> Vec<TabId> {
    let mut tabs = crate::tabs::tabs_for(kind, true).to_vec();
    for tab in crate::tabs::tabs_for(kind, false) {
        if !tabs.contains(tab) {
            tabs.push(*tab);
        }
    }
    tabs
}

/// The commands the registry puts on the row of `tab` while the tree scope
/// is of `kind`, in the row's order: what `command::local_row` filters by,
/// before any eligibility.
fn declared(tab: TabId, kind: ObjectKind) -> Vec<&'static str> {
    let mut row: Vec<(u8, &'static str)> = registry()
        .iter()
        .filter(|spec| spec.local_scopes.is_none_or(|kinds| kinds.contains(&kind)))
        .filter_map(|spec| on_row(spec, tab).map(|order| (order, spec.id.0)))
        .collect();
    row.sort_by_key(|(order, _)| *order);
    row.into_iter().map(|(_, id)| id).collect()
}

/// A command on a local row says which kinds of tree scope it is for
/// (`CommandSpec::local_scopes`), and each of those kinds has one of the
/// command's tabs. Without the first, a command that is merely disabled
/// for the scope ("Select a virtual disk in the list." on a pool's Storage
/// tab, whose list holds repositories) stands on a row where it can never
/// run.
#[test]
fn every_local_row_command_says_which_scopes_it_is_for() {
    let mut missing = Vec::new();
    let mut foreign = Vec::new();
    for spec in registry() {
        let tabs: Vec<TabId> = spec.local_rows().map(|row| row.tab).collect();
        if tabs.is_empty() {
            assert!(
                spec.local_scopes.is_none(),
                "{} names scopes for a local row it is not on",
                spec.id.0
            );
            continue;
        }
        let Some(kinds) = spec.local_scopes else {
            missing.push(spec.id.0);
            continue;
        };
        assert!(!kinds.is_empty(), "{} is on no row at all", spec.id.0);
        if tabs.iter().all(|tab| EVERYWHERE.contains(tab)) {
            assert_eq!(
                kinds,
                ObjectKind::ALL.as_slice(),
                "{}: its tabs are shown under every scope",
                spec.id.0
            );
            continue;
        }
        for kind in kinds {
            if !tabs_of(*kind).iter().any(|tab| tabs.contains(tab)) {
                foreign.push((spec.id.0, *kind));
            }
        }
    }
    assert!(missing.is_empty(), "no local_scopes: {missing:?}");
    assert!(
        foreign.is_empty(),
        "a scope kind without the command's tab: {foreign:?}"
    );
}

/// For each tab and each kind of scope, the commands of the local row. The
/// table is the reviewed state: every command in it can be enabled under
/// that scope, with the scope itself or with a row of that tab's list as
/// its target. A command added to a row, or given another scope, changes
/// the table here, where the question "can it ever run there?" is asked.
/// On a mismatch the test prints the whole table as the registry has it.
#[test]
fn the_local_row_of_each_tab_holds_only_commands_for_its_scope() {
    let mut actual: Vec<(TabId, ObjectKind, Vec<&'static str>)> = Vec::new();
    for tab in EVERYWHERE {
        let row = declared(tab, ObjectKind::None);
        for kind in ObjectKind::ALL {
            assert_eq!(declared(tab, kind), row, "{tab:?} under {kind:?}");
        }
        actual.push((tab, ObjectKind::None, row));
    }
    for kind in SCOPES {
        for tab in tabs_of(kind) {
            if EVERYWHERE.contains(&tab) {
                continue;
            }
            let row = declared(tab, kind);
            if !row.is_empty() {
                actual.push((tab, kind, row));
            }
        }
    }
    let expected: Vec<(TabId, ObjectKind, Vec<&'static str>)> = ROWS
        .iter()
        .map(|(tab, kind, row)| (*tab, *kind, row.to_vec()))
        .collect();
    if actual != expected {
        let mut table = String::new();
        for (tab, kind, row) in &actual {
            table.push_str(&format!("    (T::{tab:?}, K::{kind:?}, &{row:?}),\n"));
        }
        panic!("the local rows changed; review each change, then ROWS is:\n{table}");
    }
    // The defect this table was made for: the three disk commands stood
    // between the repository commands of a pool's and a server's Storage
    // tab, where the list holds repositories and no disk can be selected.
    for kind in [ObjectKind::Pool, ObjectKind::Host] {
        let row = declared(TabId::Storage, kind);
        assert!(row.contains(&"storage.rescan"), "{row:?}");
        for disk in ["disk.grow", "disk.copy", "disk.cleanup"] {
            assert!(
                !row.contains(&disk),
                "{disk} on the Storage tab of a {kind:?}"
            );
        }
    }
    for kind in [ObjectKind::Vm, ObjectKind::StorageRepository] {
        let row = declared(TabId::Storage, kind);
        for disk in ["disk.grow", "disk.copy", "disk.cleanup"] {
            assert!(
                row.contains(&disk),
                "{disk} on the Storage tab of a {kind:?}"
            );
        }
    }
    // A repository's own Storage tab lists its disks and has both sets:
    // the repository's commands first, then the disk's, neither between
    // the other's.
    let row = declared(TabId::Storage, ObjectKind::StorageRepository);
    let first_disk = row
        .iter()
        .position(|id| id.starts_with("disk."))
        .expect("a disk command");
    assert!(
        row[..first_disk]
            .iter()
            .all(|id| id.starts_with("storage."))
            && row[first_disk..].iter().all(|id| id.starts_with("disk.")),
        "{row:?}"
    );
}

use ObjectKind as K;
use TabId as T;

/// See [`the_local_row_of_each_tab_holds_only_commands_for_its_scope`].
/// `K::None` with Search, Alerts and Events stands for every scope.
#[rustfmt::skip]
const ROWS: &[(TabId, ObjectKind, &[&str])] = &[
    (T::Search, K::None, &[
        "search.open-result", "search.save", "search.export", "search.remove",
    ]),
    (T::Alerts, K::None, &[
        "alerts.dismiss", "events.export",
    ]),
    (T::Events, K::None, &[
        "events.cancel-task", "events.explain", "events.hide", "events.recheck",
        "events.acknowledge", "events.request-cancel", "events.export",
    ]),
    (T::Home, K::None, &[
        "server.add", "probe.open", "server.connect", "probe.both", "server.cancel-connect",
        "server.disconnect", "server.reconnect-as", "server.review-certificate",
        "server.remove-connection", "server.connect-all", "server.disconnect-all",
    ]),
    (T::General, K::Pool, &[
        "pool.properties",
    ]),
    (T::Storage, K::Pool, &[
        "storage.new", "storage.repair", "storage.set-default", "storage.rescan",
        "storage.trim", "storage.detach", "storage.reattach", "storage.attachment.create",
        "storage.attachment.remove", "storage.forget", "storage.destroy",
    ]),
    (T::Networking, K::Pool, &[
        "network.new", "network.vlan.add", "network.remove", "nic.ipv4", "nic.ipv6",
        "nic.purpose", "network.vlan.create-pool", "network.vlan.retag-pool",
        "network.service.nbd-tls", "network.service.nbd-plaintext", "network.fabric.add",
        "network.fabric.complete-pool", "network.fabric.remove", "network.fabric.remove-pool",
        "server.management.configure", "server.management.move",
        "server.management.bond.ipv6.continue", "server.management.bond.ipv6.leave",
    ]),
    (T::Ha, K::Pool, &[
        "pool.ha.configure", "pool.ha.disable", "pool.ha.failures-to-tolerate",
        "pool.ha.overcommit", "pool.ha.timeout", "vm.ha.restart-priority",
    ]),
    (T::Users, K::Pool, &[
        "pool.security.refresh", "pool.directory.join", "pool.directory.leave",
        "pool.subject.add", "pool.subject.role", "pool.subject.remove",
    ]),
    (T::General, K::Host, &[
        "server.properties",
    ]),
    (T::Memory, K::Host, &[
        "server.control-domain-memory",
    ]),
    (T::Storage, K::Host, &[
        "storage.new", "storage.repair", "storage.set-default", "storage.rescan",
        "storage.trim", "storage.detach", "storage.reattach", "storage.attachment.create",
        "storage.attachment.remove", "storage.forget", "storage.destroy",
    ]),
    (T::Networking, K::Host, &[
        "network.new", "network.vlan.add", "network.remove", "nic.ipv4", "nic.ipv6",
        "nic.purpose", "network.vlan.create-pool", "network.vlan.retag-pool",
        "network.service.nbd-tls", "network.service.nbd-plaintext", "network.fabric.add",
        "network.fabric.complete-pool", "network.fabric.remove", "network.fabric.remove-pool",
        "server.management.configure", "server.management.move",
        "server.management.bond.ipv6.continue", "server.management.bond.ipv6.leave",
    ]),
    (T::Nics, K::Host, &[
        "server.bond.create", "server.bond.delete", "server.nics.rescan",
        "server.management.bond.create", "server.management.bond.remove",
        "server.management.bond.ipv6", "server.management.bond.ipv6.continue",
        "server.management.bond.ipv6.leave",
    ]),
    (T::Console, K::Host, &[
        "server.console", "console.reconnect", "console.actual-size", "console.fullscreen",
        "console.undock", "console.release-input", "console.save-screenshot",
        "console.send-ctrl-alt-f1", "console.send-alt-tab", "console.send-ctrl-c",
        "console.send-ctrl-d", "console.send-clipboard", "console.type-clipboard",
        "console.copy-remote-clipboard", "console.close",
    ]),
    (T::Performance, K::Host, &[
        "performance.data-sources", "performance.presets", "performance.reconnect",
        "server.monitoring-recording",
    ]),
    (T::General, K::Vm, &[
        "vm.check-availability", "vm.properties",
    ]),
    (T::Memory, K::Vm, &[
        "vm.memory-settings",
    ]),
    (T::Storage, K::Vm, &[
        "vm.disk.new", "vm.disk.attach", "vm.cd.add", "vm.disk.detach", "vm.disk.activate",
        "vm.disk.deactivate", "vm.cd.insert", "vm.cd.eject", "vm.disk.properties",
        "vm.disk.delete", "vm.cloud-init-drive", "disk.grow", "disk.copy", "disk.cleanup",
        "vm.install-guest-tools",
    ]),
    (T::Networking, K::Vm, &[
        "vm.nic.add", "vm.nic.properties", "vm.nic.rate-limit", "vm.nic.activate",
        "vm.nic.deactivate", "vm.nic.remove",
    ]),
    (T::Console, K::Vm, &[
        "vm.console", "console.reconnect", "console.actual-size", "console.fullscreen",
        "console.undock", "console.release-input", "console.save-screenshot",
        "vm.send-ctrl-alt-del", "console.send-ctrl-alt-f1", "console.send-alt-tab",
        "console.send-ctrl-c", "console.send-ctrl-d", "console.send-clipboard",
        "console.type-clipboard", "console.copy-remote-clipboard", "console.close",
        "vm.connect-external",
    ]),
    (T::Performance, K::Vm, &[
        "performance.data-sources", "performance.presets", "performance.reconnect",
    ]),
    (T::Snapshots, K::Vm, &[
        "vm.snapshot", "snapshot.revert", "snapshot.properties", "snapshot.new-vm",
        "snapshot.new-template", "snapshot.export", "snapshot.delete",
    ]),
    (T::General, K::Template, &[
        "template.properties",
    ]),
    (T::Memory, K::Template, &[
        "vm.memory-settings",
    ]),
    (T::General, K::StorageRepository, &[
        "storage.properties",
    ]),
    (T::Storage, K::StorageRepository, &[
        "storage.repair", "storage.set-default", "storage.rescan", "storage.trim",
        "storage.detach", "storage.reattach", "storage.attachment.create",
        "storage.attachment.remove", "storage.forget", "storage.destroy", "disk.grow",
        "disk.copy", "disk.cleanup", "disk.export-image",
    ]),
    (T::General, K::Network, &[
        "network.properties",
    ]),
    (T::Networking, K::Network, &[
        "network.new", "network.vlan.add", "network.remove", "nic.ipv4", "nic.ipv6",
        "nic.purpose", "network.vlan.create-pool", "network.vlan.retag-pool",
        "network.service.nbd-tls", "network.service.nbd-plaintext", "network.fabric.add",
        "network.fabric.complete-pool", "network.fabric.remove", "network.fabric.remove-pool",
        "server.management.configure", "server.management.move",
        "server.management.bond.ipv6.continue", "server.management.bond.ipv6.leave",
    ]),
    (T::General, K::VirtualDisk, &[
        "disk.properties",
    ]),
];
