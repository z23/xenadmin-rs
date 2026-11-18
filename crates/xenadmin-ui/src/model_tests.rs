//! Tests of `model.rs`: the shared helpers, the selection, the scope
//! session, the context menu, the header and the status bar.

use super::*;
use crate::nav::NavView;
use crate::selection::MenuOrigin;
use xenadmin_core::{HostId, PoolId, VmId};

fn vm_ref(pool: PoolId, n: u32) -> ObjectRef {
    ObjectRef::vm(
        pool,
        VmId::parse(&format!("30000000-0000-0000-0000-{n:012}")).expect("vm id"),
    )
}

fn host_ref(pool: PoolId, n: u32) -> ObjectRef {
    ObjectRef::host(
        pool,
        HostId::parse(&format!("20000000-0000-0000-0000-{n:012}")).expect("host id"),
    )
}

fn demo_pool(m: &Model) -> PoolId {
    m.inventory.clone().expect("inventory").pool_id
}

/// Replaces the scope session's inventory, as a new snapshot would.
fn change_inventory(m: &mut Model, change: impl FnOnce(&mut Inventory)) {
    let mut inventory = m.inventory.clone().expect("inventory").as_ref().clone();
    change(&mut inventory);
    m.inventory = Some(Arc::new(inventory));
}

fn second_pool() -> Inventory {
    let mut inventory = crate::demonstration(1, 4);
    inventory.pool_id = PoolId::parse("10000000-0000-0000-0000-000000000063").unwrap();
    inventory.pool_name = "Recovery lab".into();
    inventory
}

fn vm_record(index: u128, pool: &str, state: OperationState) -> OperationRecord {
    serde_json::from_value(serde_json::json!({
        "id": uuid::Uuid::from_u128(0x8100_0000_0000_0000_0000_0000_0000_0000 + index),
        "pool": pool,
        "vm": "30000000-0000-0000-0000-000000000001",
        "action": "Start",
        "state": state,
        "progress": null,
        "detail": "Synthetic operation result",
        "created_at_ms": index as u64,
    }))
    .unwrap()
}

fn resource_record(index: u128, state: OperationState) -> ResourceOperationRecord {
    serde_json::from_value(serde_json::json!({
        "id": uuid::Uuid::from_u128(0x8200_0000_0000_0000_0000_0000_0000_0000 + index),
        "pool": "10000000-0000-0000-0000-000000000001",
        "target": {"Storage": uuid::Uuid::from_u128(40)},
        "action": "ScanStorage",
        "state": state,
        "progress": 1.0,
        "detail": "Synthetic scan completed",
        "created_at_ms": index as u64,
        "can_cancel": false,
        "cancel_requested": false,
        "steps": [],
        "created": null,
        "baseline_networks": [uuid::Uuid::from_u128(41)],
    }))
    .unwrap()
}

const POOL: &str = "10000000-0000-0000-0000-000000000001";

// -- Fixture, tabs, grid (the K1 tests, kept) ---------------------------------

#[test]
fn a_fixture_model_shows_home_and_has_nothing_open() {
    let m = Model::fixture();
    assert!(m.surfaces().is_empty());
    assert!(m.front_surface().is_none());
    assert!(!m.experimental_notice());
    assert_eq!(m.tabs(), [TabId::Home, TabId::Search]);
    assert_eq!(m.active_tab(), TabId::Home);
    assert!(m.target().is_empty());
    // The connections area provides Home and the search area Search.
    assert!(m.tab_body(TabId::Home).is_some());
    assert!(m.tab_body(TabId::Search).is_some());
    // Nothing of the real process: no ownership, no backend, no session.
    assert!(!m.owns_state());
    assert!(m.backend.is_err());
    assert!(m.connection.is_none() && m.snapshot.is_none() && m.inventory.is_none());
    assert!(!m.can_mutate());
    assert_eq!(m.header().name, "XenAdmin RS");
    assert_eq!(m.window_title(), "XenAdmin RS");
    assert_eq!(m.status().connection, "No host connected");
}

#[test]
fn the_tabs_follow_the_scope_and_the_chosen_tab_survives_where_it_exists() {
    let mut m = Model::fixture();
    m.load_demo(3, 48);
    let inventory = m.inventory.clone().expect("inventory");
    let pool = inventory.pool_id;
    let vm = *inventory.vms.keys().next().expect("vm");
    let host = *inventory.hosts.keys().next().expect("host");

    m.select(Target::one(ObjectRef::vm(pool, vm)));
    assert_eq!(m.active_tab(), TabId::General);
    m.set_active_tab(TabId::Snapshots);
    assert_eq!(m.active_tab(), TabId::Snapshots);

    // A live host has no Snapshots tab: the first tab shows instead.
    m.select(Target::one(ObjectRef::host(pool, host)));
    assert!(m.tabs().contains(&TabId::Console));
    assert_eq!(m.active_tab(), TabId::General);

    // A host the inventory does not know is not live.
    let unknown = HostId::parse("20000000-0000-0000-0000-00000000ffff").expect("host id");
    m.select(Target::one(ObjectRef::host(pool, unknown)));
    assert_eq!(m.tabs(), [TabId::General, TabId::Search]);

    m.select(Target::many([
        ObjectRef::vm(pool, vm),
        ObjectRef::host(pool, host),
    ]));
    assert_eq!(m.tabs(), [TabId::Search]);
}

#[test]
fn the_grid_selection_is_the_target_until_the_scope_or_the_tab_changes() {
    let mut m = Model::fixture();
    m.load_demo(3, 48);
    let pool = demo_pool(&m);
    m.select(Target::one(ObjectRef::pool(pool)));
    assert_eq!(m.target().kind(), ObjectKind::Pool);
    m.select_in_grid(Target::many([vm_ref(pool, 1), vm_ref(pool, 2)]));
    assert_eq!(m.target().len(), 2);
    // The grid selection is the command target and does not change the scope.
    assert_eq!(m.scope().kind(), ObjectKind::Pool);
    assert_eq!(m.tabs()[0], TabId::General);
    assert!(m.selected().is_none(), "two rows are not one object");
    m.set_active_tab(TabId::Storage);
    assert_eq!(m.target().kind(), ObjectKind::Pool);
    m.select_in_grid(Target::one(vm_ref(pool, 1)));
    assert_eq!(m.selected(), Some(&vm_ref(pool, 1)));
    m.select(Target::one(vm_ref(pool, 2)));
    assert_eq!(m.target(), Target::one(vm_ref(pool, 2)));
}

#[test]
fn the_earliest_tick_request_wins_and_is_taken_once() {
    let mut m = Model::fixture();
    assert_eq!(m.take_tick_request(), None);
    m.request_tick(2.0);
    m.request_tick(0.5);
    m.request_tick(1.0);
    assert_eq!(m.take_tick_request(), Some(0.5));
    assert_eq!(m.take_tick_request(), None);
    assert!(!m.poll(12.5));
    assert_eq!(m.clock(), 12.5);
}

// -- Demo ---------------------------------------------------------------------

#[test]
fn the_demo_is_a_synthetic_session_with_its_pool_selected() {
    let m = Model::demo(3, 48);
    assert!(m.demo);
    assert!(m.connection.is_none() && m.snapshot.is_none());
    assert_eq!(m.workspace.selected, Some(crate::demo::profile_id()));
    let pool = demo_pool(&m);
    assert_eq!(m.scope(), &Target::one(ObjectRef::pool(pool)));
    assert_eq!(m.active_tab(), TabId::General);
    assert!(
        !m.can_go_back(),
        "loading the demo is not a place to return to"
    );
    assert!(!m.can_mutate());
    let header = m.header();
    assert_eq!(
        (header.icon, header.name.as_str(), header.state.as_deref()),
        (ObjectKind::Pool, "Demonstration pool", Some("Demo"))
    );
    assert_eq!(m.window_title(), "Demonstration pool");
    let status = m.status();
    assert_eq!(status.connection, "Demo inventory • no host connection");
    assert_eq!(
        status.demo,
        Some("DEMO  •  Synthetic data. Host actions are unavailable.")
    );
    assert_eq!(status.selection, "Demonstration pool");
    assert_eq!(status.host_notice, None);
    assert_eq!((status.operations, status.unknown_outcomes), (0, 0));
}

#[test]
fn loading_the_demo_again_drops_the_lists_of_the_previous_view() {
    let mut m = Model::demo(3, 48);
    m.operation_rows = vec![vm_record(1, POOL, OperationState::Succeeded)];
    m.resource_rows = vec![resource_record(1, OperationState::Succeeded)];
    m.error = Some("left over".into());
    m.workspace.show_connect = true;
    let pool = demo_pool(&m);
    m.select(Target::one(vm_ref(pool, 3)));
    m.load_demo(16, 2000);
    assert!(m.operation_rows.is_empty() && m.resource_rows.is_empty());
    assert!(m.error.is_none() && !m.workspace.show_connect);
    assert_eq!(m.scope().kind(), ObjectKind::Pool);
    assert!(!m.can_go_back());
    let inventory = m.inventory.clone().unwrap();
    assert_eq!((inventory.hosts.len(), inventory.vms.len()), (16, 2000));
    assert_eq!(session::sessions(&m).len(), 1);
}

// -- Shared helpers -----------------------------------------------------------

#[test]
fn the_submission_choke_point_refuses_demo_then_a_full_list_then_a_missing_connection() {
    // The refusals in the order `submit_tracked_command` makes them. A
    // command itself cannot be built here: every `VmCommand` carries an
    // observation only a current connection gives.
    let stale = Error::Stale.to_string();
    let busy = Error::Busy.to_string();
    let closed = Error::Closed.to_string();
    let text = |result: Result<(), Error>| result.map_err(|error| error.to_string());
    assert_eq!(text(admit(true, 0, true)), Err(stale.clone()));
    assert_eq!(text(admit(true, MAX_OPERATIONS, false)), Err(stale.clone()));
    assert_eq!(text(admit(false, MAX_OPERATIONS, true)), Err(busy.clone()));
    assert_eq!(text(admit(false, MAX_OPERATIONS + 5, false)), Err(busy));
    assert_eq!(
        text(admit(false, MAX_OPERATIONS - 1, false)),
        Err(closed.clone())
    );
    assert_eq!(text(admit(false, MAX_OPERATIONS - 1, true)), Ok(()));
    assert_eq!(MAX_OPERATIONS, 64);

    // On a model: the demo is stale, a fixture without a connection is closed.
    let mut m = Model::demo(3, 48);
    assert_eq!(text(m.admit_command()), Err(stale.clone()));
    m.demo = false;
    assert_eq!(text(m.admit_command()), Err(closed.clone()));
    assert_eq!(text(Model::fixture().admit_command()), Err(closed));
    assert!(m.operations.is_empty());

    // The power area's path in demo: the VM cannot even be observed.
    let mut m = Model::demo(3, 48);
    assert!(m.snapshot.is_none());
    m.diagnose_actions(VmId::parse("30000000-0000-0000-0000-000000000001").unwrap());
    assert_eq!(m.error.as_deref(), Some(stale.as_str()));
    assert!(m.diagnostics.is_none());
}

#[test]
fn control_without_a_connection_reports_closed() {
    let mut m = Model::fixture();
    m.control_operation(OperationId::new(), OperationControl::Recheck);
    assert_eq!(m.error.as_deref(), Some(Error::Closed.to_string().as_str()));
}

#[test]
fn vm_operation_rows_merge_by_pool_and_id_and_keep_the_unresolved() {
    // Rune: vm_lifecycle_tests.rs
    // `shared_history_retains_unresolved_records_and_bounds_terminal_records`.
    let mut rows = vec![vm_record(1, POOL, OperationState::OutcomeUnknown)];
    rows.extend((2..112).map(|i| vm_record(i, POOL, OperationState::Succeeded)));
    let retained = merge_operation_rows(&rows, Vec::new());
    assert_eq!(retained.len(), 101);
    assert_eq!(retained[0].id, rows[0].id);
    assert_eq!(merge_operation_rows(&retained, Vec::new()), retained);
    let rejected = vm_record(111, POOL, OperationState::Rejected);
    let merged = merge_operation_rows(&retained, vec![rejected.clone()]);
    assert_eq!(
        merged
            .iter()
            .find(|row| row.id == rejected.id)
            .unwrap()
            .state,
        OperationState::Rejected
    );
    // The result is oldest first, and the newest finished records are the
    // ones kept.
    assert!(
        retained
            .windows(2)
            .all(|pair| pair[0].created_at_ms <= pair[1].created_at_ms)
    );
    assert_eq!(retained.last().unwrap().created_at_ms, 111);
    assert_eq!(retained[1].created_at_ms, 12);
    // The same operation id in another pool is another row.
    let other = vm_record(
        111,
        "10000000-0000-0000-0000-000000000063",
        OperationState::Tracking,
    );
    let merged = merge_operation_rows(&retained, vec![other.clone()]);
    assert_eq!(merged.len(), 102);
    assert_eq!(merged.iter().filter(|row| row.id == other.id).count(), 2);
    // An older record of the same operation does not replace the stored one.
    let stale = vm_record(111, POOL, OperationState::Tracking);
    let merged = merge_operation_rows(&merged, vec![stale]);
    assert_eq!(
        merged
            .iter()
            .find(|row| row.id == rejected.id && row.pool.to_string() == POOL)
            .unwrap()
            .state,
        OperationState::Succeeded
    );
}

#[test]
fn resource_operation_rows_merge_by_id_and_bound_the_finished_ones() {
    let mut rows = vec![resource_record(1, OperationState::OutcomeUnknown)];
    rows.extend((2..112).map(|i| resource_record(i, OperationState::Succeeded)));
    let retained = merge_resource_rows(&rows, Vec::new());
    assert_eq!(retained.len(), 101);
    assert_eq!(retained[0].id, rows[0].id);
    assert_eq!(merge_resource_rows(&retained, Vec::new()), retained);
    assert_eq!(retained.last().unwrap().created_at_ms, 111);
    // Polling a model without a connection keeps what it has.
    let mut m = Model::fixture();
    m.resource_rows = retained.clone();
    assert!(!m.poll(1.0));
    assert_eq!(m.resource_rows, retained);
    // The status bar counts what is unresolved, and what is unknown apart.
    m.operation_rows = vec![
        vm_record(1, POOL, OperationState::Tracking),
        vm_record(2, POOL, OperationState::OutcomeUnknown),
        vm_record(3, POOL, OperationState::Succeeded),
    ];
    let status = m.status();
    assert_eq!((status.operations, status.unknown_outcomes), (1, 2));
}

#[test]
fn widths_are_kept_in_the_preferences_and_saved_once_they_rest() {
    // Rune: main.rs `column_and_sidebar_widths_reach_the_preferences_and_survive_a_table_rebuild`
    // and `widths_are_saved_shortly_after_they_stop_changing_without_waiting_for_close`.
    let mut m = Model::demo(3, 48);
    m.poll(0.0);
    assert!(!m.layout_pending(), "nothing set yet");
    m.set_column_width("name", Some(321.0));
    assert_eq!(m.column_width("name"), Some(321.0));
    assert!(m.layout_pending(), "a change starts the delay");
    assert_eq!(m.take_tick_request(), Some(LAYOUT_SAVE_DELAY));
    m.set_column_width("name", Some(321.0));
    assert_eq!(
        m.take_tick_request(),
        None,
        "an unchanged width is not a change"
    );
    // A drag keeps deferring.
    m.poll(0.3);
    m.set_column_width("name", Some(340.0));
    m.poll(0.5);
    assert!(m.layout_pending());
    m.poll(0.95);
    assert!(!m.layout_pending(), "saved once it rests");
    assert_eq!(m.preferences.columns.get("name"), Some(&340.0));
    // A scope session change keeps the widths.
    m.set_column_width("state", Some(150.0));
    session::clear_view(&mut m);
    assert_eq!(m.column_width("state"), Some(150.0));
    // A column put back to its declared width is forgotten.
    m.set_column_width("name", None);
    assert!(!m.preferences.columns.contains_key("name"));
    assert!(m.layout_pending());
    // Widths outside what the client accepts are ignored.
    m.set_column_width("cpu", Some(4.0));
    m.set_column_width("cpu", Some(f32::NAN));
    assert_eq!(m.column_width("cpu"), None);
    m.set_sidebar_width(9000.0);
    assert_eq!(m.preferences.sidebar_width, 420.0);
    m.set_sidebar_width(f32::INFINITY);
    assert_eq!(m.preferences.sidebar_width, 420.0);
    // A fixture never writes: the save is a no-op that clears the delay.
    m.save_preferences();
    assert!(!m.layout_pending() && m.error.is_none());
}

#[test]
fn the_interface_scale_is_kept_in_the_options() {
    let mut m = Model::fixture();
    m.set_interface_scale(125);
    assert_eq!(m.preferences.options.interface_scale, 125);
    assert_eq!(m.preferences.options.zoom(), 1.25);
    m.set_interface_scale(125);
    assert!(m.error.is_none());
}

#[test]
fn hosts_outside_the_qualified_release_are_named_in_a_notice() {
    // Rune: main.rs, the same name; the notice text.
    let mut m = Model::demo(3, 48);
    assert_eq!(m.unqualified_hosts_notice(), None);
    change_inventory(&mut m, |inventory| {
        let mut hosts = inventory.hosts.values_mut();
        hosts.next().unwrap().version = "8.2.1".into();
        hosts.next().unwrap().version = String::new();
    });
    let notice = m.status().host_notice.expect("notice");
    assert_eq!(
        notice,
        "Not qualified on this host version: xcp-lab-01 (8.2.1), xcp-lab-02 (no version reported). This client has only been run against XCP-ng 8.3. Looking is safe; treat any change here as untested."
    );
    change_inventory(&mut m, |inventory| {
        for host in inventory.hosts.values_mut() {
            host.version = "9.0".into();
        }
        for n in 4..=6 {
            let mut host = inventory.hosts.values().next().unwrap().clone();
            host.id = HostId::parse(&format!("20000000-0000-0000-0000-{n:012}")).unwrap();
            host.name = format!("xcp-lab-{n:02}");
            inventory.hosts.insert(host.id, host);
        }
    });
    let notice = m.unqualified_hosts_notice().expect("notice");
    assert!(notice.contains("xcp-lab-01 (9.0), xcp-lab-02 (9.0), xcp-lab-03 (9.0) and 3 more."));
}

// -- Selection: survival, parent fallback ---------------------------------------

#[test]
fn the_selection_survives_an_inventory_update() {
    let mut m = Model::demo(3, 48);
    let pool = demo_pool(&m);
    m.select(Target::one(vm_ref(pool, 1)));
    m.select_in_grid(Target::one(vm_ref(pool, 2)));
    change_inventory(&mut m, |inventory| {
        inventory.revision += 1;
        for vm in inventory.vms.values_mut() {
            vm.name.push_str(" (renamed)");
        }
    });
    assert!(!m.reconcile_selection());
    assert_eq!(m.scope(), &Target::one(vm_ref(pool, 1)));
    assert_eq!(m.target(), Target::one(vm_ref(pool, 2)));
    assert_eq!(m.header().name, "Database (renamed)");
    assert!(m.notice.is_none());
}

#[test]
fn when_the_object_disappears_its_parent_is_selected_and_a_notice_says_so() {
    let mut m = Model::demo(3, 48);
    let pool = demo_pool(&m);
    // Guest 1 runs on the second host.
    let vm = vm_ref(pool, 1);
    m.select(Target::one(vm.clone()));
    assert_eq!(m.parent_of(&vm), Some(host_ref(pool, 2)));
    m.set_active_tab(TabId::Console);
    change_inventory(&mut m, |inventory| {
        inventory.vms.remove(&vm.vm_id().unwrap());
    });
    assert!(m.poll(1.0));
    assert_eq!(m.scope(), &Target::one(host_ref(pool, 2)));
    assert_eq!(
        m.notice.as_deref(),
        Some("VM \"Database\" is no longer in the inventory. Showing xcp-lab-02 instead.")
    );
    assert_eq!(m.status().notice, m.notice);
    // The host has a Console tab too, so the tab stays.
    assert_eq!(m.active_tab(), TabId::Console);
    // The host goes as well: the pool.
    change_inventory(&mut m, |inventory| {
        inventory
            .hosts
            .remove(&host_ref(pool, 2).host_id().unwrap());
    });
    assert!(m.reconcile_selection());
    assert_eq!(m.scope(), &Target::one(ObjectRef::pool(pool)));
    assert_eq!(
        m.notice.as_deref(),
        Some(
            "Server \"xcp-lab-02\" is no longer in the inventory. Showing Demonstration pool instead."
        )
    );
    // Selecting something by hand clears the notice.
    m.select(Target::one(vm_ref(pool, 2)));
    assert!(m.notice.is_none());
}

#[test]
fn a_vm_and_its_host_gone_together_fall_back_to_the_pool() {
    let mut m = Model::demo(3, 48);
    let pool = demo_pool(&m);
    let vm = vm_ref(pool, 1);
    m.select(Target::one(vm.clone()));
    change_inventory(&mut m, |inventory| {
        inventory.vms.remove(&vm.vm_id().unwrap());
        inventory
            .hosts
            .remove(&host_ref(pool, 2).host_id().unwrap());
    });
    assert!(m.reconcile_selection());
    assert_eq!(m.scope(), &Target::one(ObjectRef::pool(pool)));
}

#[test]
fn a_vm_converted_to_a_template_is_followed_without_a_notice() {
    let mut m = Model::demo(3, 48);
    let pool = demo_pool(&m);
    let vm = vm_ref(pool, 5);
    m.select(Target::one(vm.clone()));
    change_inventory(&mut m, |inventory| {
        inventory
            .vms
            .get_mut(&vm.vm_id().unwrap())
            .unwrap()
            .is_template = true;
    });
    assert!(m.reconcile_selection());
    assert_eq!(
        m.scope(),
        &Target::one(ObjectRef::template(pool, vm.vm_id().unwrap()))
    );
    assert!(m.notice.is_none());
    assert_eq!(m.tabs(), tabs::tabs_for(ObjectKind::Template, false));
}

#[test]
fn rows_of_the_grid_selection_that_are_gone_are_dropped() {
    let mut m = Model::demo(3, 48);
    let pool = demo_pool(&m);
    m.select_in_grid(Target::many([vm_ref(pool, 1), vm_ref(pool, 2)]));
    change_inventory(&mut m, |inventory| {
        inventory.vms.remove(&vm_ref(pool, 1).vm_id().unwrap());
    });
    assert!(m.reconcile_selection());
    assert_eq!(m.target(), Target::one(vm_ref(pool, 2)));
    assert_eq!(m.scope().kind(), ObjectKind::Pool, "the scope stays");
    assert!(m.notice.is_none());
}

#[test]
fn several_selected_objects_lose_the_ones_that_are_gone() {
    let mut m = Model::demo(3, 48);
    let pool = demo_pool(&m);
    m.select(Target::many([
        vm_ref(pool, 1),
        vm_ref(pool, 2),
        vm_ref(pool, 3),
    ]));
    assert_eq!(m.header().name, "3 objects selected");
    assert_eq!(m.header().pool.as_deref(), Some("Demonstration pool"));
    change_inventory(&mut m, |inventory| {
        inventory.vms.remove(&vm_ref(pool, 2).vm_id().unwrap());
    });
    assert!(m.reconcile_selection());
    assert_eq!(m.scope().len(), 2);
    assert_eq!(
        m.notice.as_deref(),
        Some("1 selected object is no longer in the inventory.")
    );
}

// -- Back, Forward, last tab per kind -------------------------------------------

#[test]
fn back_and_forward_step_through_selection_and_tab_pairs() {
    let mut m = Model::demo(3, 48);
    let pool = demo_pool(&m);
    let home = Target::one(ObjectRef::pool(pool));
    m.select(Target::one(vm_ref(pool, 1))); // (pool, General) -> (vm1, General)
    m.set_active_tab(TabId::Snapshots); // -> (vm1, Snapshots)
    m.select(Target::one(host_ref(pool, 1))); // -> (host1, General)
    assert!(m.can_go_back() && !m.can_go_forward());
    assert!(m.back());
    assert_eq!(
        (m.scope().clone(), m.active_tab()),
        (Target::one(vm_ref(pool, 1)), TabId::Snapshots)
    );
    assert!(m.back());
    assert_eq!(
        (m.scope().clone(), m.active_tab()),
        (Target::one(vm_ref(pool, 1)), TabId::General)
    );
    assert!(m.back());
    assert_eq!(
        (m.scope().clone(), m.active_tab()),
        (home.clone(), TabId::General)
    );
    assert!(!m.back(), "nothing before the first place");
    assert!(m.forward());
    assert!(m.forward());
    assert_eq!(
        (m.scope().clone(), m.active_tab()),
        (Target::one(vm_ref(pool, 1)), TabId::Snapshots)
    );
    // Going somewhere new drops what was ahead.
    m.select(home.clone());
    assert!(!m.can_go_forward());
    assert!(!m.forward());
    assert!(m.back());
    assert_eq!(m.scope(), &Target::one(vm_ref(pool, 1)));
    // Selecting what is already selected is not a step.
    let before = m.can_go_forward();
    m.select(Target::one(vm_ref(pool, 1)));
    assert_eq!(m.can_go_forward(), before);
}

#[test]
fn back_skips_a_place_whose_object_is_gone() {
    let mut m = Model::demo(3, 48);
    let pool = demo_pool(&m);
    m.select(Target::one(vm_ref(pool, 1)));
    m.select(Target::one(vm_ref(pool, 2)));
    m.select(Target::one(vm_ref(pool, 3)));
    change_inventory(&mut m, |inventory| {
        inventory.vms.remove(&vm_ref(pool, 2).vm_id().unwrap());
    });
    assert!(m.back());
    assert_eq!(m.scope(), &Target::one(vm_ref(pool, 1)));
}

#[test]
fn the_last_active_tab_is_remembered_per_object_kind() {
    let mut m = Model::demo(3, 48);
    let pool = demo_pool(&m);
    m.select(Target::one(vm_ref(pool, 1)));
    m.set_active_tab(TabId::Console);
    m.select(Target::one(host_ref(pool, 1)));
    // No tab was chosen for a host yet: the one shown stays, a host has it.
    assert_eq!(m.active_tab(), TabId::Console);
    m.set_active_tab(TabId::Nics);
    m.select(Target::one(ObjectRef::pool(pool)));
    // The pool has neither; its first tab.
    assert_eq!(m.active_tab(), TabId::General);
    m.set_active_tab(TabId::Ha);
    // Each kind gets its own tab back.
    m.select(Target::one(vm_ref(pool, 2)));
    assert_eq!(m.active_tab(), TabId::Console);
    m.select(Target::one(host_ref(pool, 2)));
    assert_eq!(m.active_tab(), TabId::Nics);
    m.select(Target::one(ObjectRef::pool(pool)));
    assert_eq!(m.active_tab(), TabId::Ha);
    assert_eq!(m.selection.last_tab(ObjectKind::Vm), Some(TabId::Console));
    assert_eq!(m.selection.last_tab(ObjectKind::Template), None);
}

#[test]
fn a_multi_selection_shows_only_the_search_tab() {
    let mut m = Model::demo(3, 48);
    let pool = demo_pool(&m);
    m.select(Target::one(vm_ref(pool, 1)));
    m.set_active_tab(TabId::Console);
    m.select(Target::many([vm_ref(pool, 1), vm_ref(pool, 2)]));
    assert_eq!(m.tabs(), [TabId::Search]);
    assert_eq!(m.active_tab(), TabId::Search);
    assert_eq!(m.status().selection, "2 objects selected");
}

// -- The scope session ----------------------------------------------------------

/// Two areas record what they are told, to see the order of the hooks.
fn hook_log(m: &Model) -> Vec<&'static str> {
    m.areas.probe.hooks.clone()
}

#[test]
fn moving_to_another_pool_switches_the_scope_session_and_closes_its_surfaces() {
    let mut m = Model::demo(3, 48);
    let first = demo_pool(&m);
    let other = m.add_synthetic_pool("Recovery lab", second_pool());
    let second = PoolId::parse("10000000-0000-0000-0000-000000000063").unwrap();
    assert_eq!(session::session_for_pool(&m, second), Some(other));
    assert_eq!(m.inventory_for(second).unwrap().pool_name, "Recovery lab");
    // A surface of the first pool is open, and a VM operation handle list
    // and diagnostics results belong to it.
    m.select(Target::one(vm_ref(first, 1)));
    let target = m.target();
    assert!(m.invoke(crate::areas::example_tests::RENAME, &target));
    assert_eq!(m.surfaces().len(), 1);
    m.diagnostic_results = vec!["left over".into()];
    m.select_in_grid(Target::one(vm_ref(first, 2)));
    m.areas.probe.hooks.clear();

    // Within the pool: the scope changes, the session does not.
    m.select(Target::one(vm_ref(first, 3)));
    assert_eq!(hook_log(&m), ["scope"]);
    assert_eq!(m.workspace.selected, Some(crate::demo::profile_id()));
    assert_eq!(m.diagnostic_results, ["left over"]);
    m.areas.probe.hooks.clear();
    let target = m.target();
    assert!(m.invoke(crate::areas::example_tests::RENAME, &target));

    // To the other pool: the session is re-pointed first, then the scope.
    m.select(Target::one(vm_ref(second, 1)));
    assert_eq!(hook_log(&m), ["session", "scope"]);
    assert_eq!(m.workspace.selected, Some(other));
    assert_eq!(m.inventory.as_ref().unwrap().pool_id, second);
    assert_eq!(m.inventory.as_ref().unwrap().vms.len(), 4);
    assert!(m.demo, "the other pool is synthetic too");
    assert!(
        m.surfaces().is_empty(),
        "the surface of the previous pool is closed"
    );
    assert!(m.diagnostic_results.is_empty() && m.diagnostics.is_none());
    assert!(m.operations.is_empty());
    assert!(m.selection.grid.is_empty());
    assert_eq!(m.header().pool.as_deref(), Some("Recovery lab"));
    assert_eq!(m.preferences.last_profile, Some(other));

    // Back returns to the first pool and its session.
    assert!(m.back());
    assert_eq!(m.scope(), &Target::one(vm_ref(first, 3)));
    assert_eq!(m.workspace.selected, Some(crate::demo::profile_id()));
    assert_eq!(m.inventory.as_ref().unwrap().vms.len(), 48);

    // Nothing selected has no owner: the scope session stays.
    m.select(Target::none());
    assert_eq!(m.workspace.selected, Some(crate::demo::profile_id()));
    assert!(m.inventory.is_some());
    // Objects of two pools have no single owner either.
    m.select(Target::many([vm_ref(first, 1), vm_ref(second, 1)]));
    assert_eq!(m.workspace.selected, Some(crate::demo::profile_id()));
    assert_eq!(m.tabs(), [TabId::Search]);
}

#[test]
fn a_change_made_to_a_synthetic_inventory_is_kept_across_a_switch() {
    let mut m = Model::demo(3, 48);
    let first = demo_pool(&m);
    m.add_synthetic_pool("Recovery lab", second_pool());
    let second = PoolId::parse("10000000-0000-0000-0000-000000000063").unwrap();
    change_inventory(&mut m, |inventory| inventory.pool_name = "Renamed".into());
    m.select(Target::one(ObjectRef::pool(second)));
    m.select(Target::one(ObjectRef::pool(first)));
    assert_eq!(m.inventory.as_ref().unwrap().pool_name, "Renamed");
    assert_eq!(m.header().name, "Renamed");
}

#[test]
fn a_saved_connection_that_is_not_connected_can_be_selected_and_owns_no_inventory() {
    let mut m = Model::demo(3, 48);
    let profile = xenadmin_core::ConnectionProfile::new(
        "Recovery pool".into(),
        xenadmin_core::Endpoint::parse("https://recovery.example").unwrap(),
        "operator".into(),
    );
    let id = profile.id;
    m.workspace.preview_profiles.push(profile);
    let node = ObjectRef::new(None, ObjectKind::Connection, id);
    m.select(Target::one(node.clone()));
    assert_eq!(m.workspace.selected, Some(id));
    assert!(m.inventory.is_none() && m.connection.is_none() && !m.demo);
    assert_eq!(m.tabs(), [TabId::General, TabId::Search]);
    let header = m.header();
    assert_eq!(
        (header.icon, header.name.as_str(), header.state.as_deref()),
        (
            ObjectKind::Connection,
            "Recovery pool",
            Some("Disconnected")
        )
    );
    assert_eq!(m.status().connection, "No host connected");
    assert_eq!(m.status().demo, None);
    assert_eq!(m.window_title(), "Recovery pool");
    // The saved connection is removed from the list: nothing is selected.
    m.workspace.preview_profiles.retain(|p| p.id != id);
    assert!(m.reconcile_selection());
    assert!(m.scope().is_empty());
}

#[test]
fn can_mutate_needs_a_writable_current_connection_in_every_pool() {
    let mut m = Model::demo(3, 48);
    let first = demo_pool(&m);
    m.add_synthetic_pool("Recovery lab", second_pool());
    let second = PoolId::parse("10000000-0000-0000-0000-000000000063").unwrap();
    assert!(!m.can_mutate());
    assert!(!m.can_mutate_pool(first));
    assert!(!m.can_mutate_pool(second));
    assert!(m.connection_for(first).is_none() && m.snapshot_for(second).is_none());
    assert!(!m.can_mutate_pool(PoolId::parse("10000000-0000-0000-0000-0000000000ff").unwrap()));
    // The unavailable-actions fixture: an inventory, not demo, no connection.
    m.demo = false;
    assert!(!m.can_mutate());
}

#[test]
fn show_home_selects_nothing_and_shows_the_connection_list_from_any_view() {
    let mut m = Model::demo(3, 48);
    let pool = demo_pool(&m);
    m.select(Target::one(vm_ref(pool, 1)));
    m.set_nav_view(NavView::Notifications);
    m.show_home();
    assert_eq!(m.nav.view, NavView::Infrastructure);
    assert!(m.scope().is_empty());
    assert_eq!(m.active_tab(), TabId::Home);
    // The scope session stays, and Back returns to the VM.
    assert_eq!(m.workspace.selected, Some(crate::demo::profile_id()));
    assert!(m.back());
    assert_eq!(m.scope(), &Target::one(vm_ref(pool, 1)));
}

// -- Notifications view ---------------------------------------------------------

#[test]
fn the_notifications_view_keeps_the_selection_and_its_tab() {
    let mut m = Model::demo(3, 48);
    let pool = demo_pool(&m);
    m.select(Target::one(vm_ref(pool, 1)));
    m.set_active_tab(TabId::Console);
    m.set_nav_view(NavView::Notifications);
    assert_eq!(m.tabs(), tabs::NOTIFICATION_TABS);
    assert_eq!(m.header().name, "Notifications");
    m.select_nav_row("notifications:events");
    assert_eq!(m.active_tab(), TabId::Events);
    // A tab of the object matrix is not a tab of this view.
    m.set_active_tab(TabId::Console);
    assert_eq!(m.active_tab(), TabId::Events);
    m.set_nav_view(NavView::Infrastructure);
    assert_eq!(m.scope(), &Target::one(vm_ref(pool, 1)));
    assert_eq!(m.active_tab(), TabId::Console);
    // Selecting an object from the Notifications view shows it in the tree.
    m.set_nav_view(NavView::Notifications);
    m.select(Target::one(vm_ref(pool, 2)));
    assert_eq!(m.nav.view, NavView::Infrastructure);
}

#[test]
fn a_press_on_a_tree_row_selects_its_object_and_a_heading_selects_nothing() {
    let mut m = Model::demo(3, 48);
    let pool = demo_pool(&m);
    let profile = crate::demo::profile_id();
    let row = crate::nav::object_row_id(profile, &vm_ref(pool, 7));
    m.select_nav_row(&row);
    assert_eq!(m.scope(), &Target::one(vm_ref(pool, 7)));
    m.set_nav_view(NavView::Objects);
    m.select_nav_row("objects:vm");
    assert_eq!(m.scope(), &Target::one(vm_ref(pool, 7)));
    m.select_nav_row("no such row");
    assert_eq!(m.scope(), &Target::one(vm_ref(pool, 7)));
}

// -- The context menu (Rune inventory_menu_tests.rs) -----------------------------

#[test]
fn a_menu_on_a_row_outside_the_grid_selection_replaces_it_and_inside_keeps_it() {
    // Rune rule 31 (`inventory_menu::open`).
    let mut m = Model::demo(3, 48);
    let pool = demo_pool(&m);
    m.select_in_grid(Target::many([vm_ref(pool, 1), vm_ref(pool, 2)]));
    assert!(m.open_context_menu(vm_ref(pool, 2), MenuOrigin::Grid));
    assert_eq!(m.selection.menu.as_ref().unwrap().target.len(), 2);
    assert!(m.context_menu_valid());
    assert!(m.open_context_menu(vm_ref(pool, 3), MenuOrigin::Grid));
    assert_eq!(m.target(), Target::one(vm_ref(pool, 3)));
    assert_eq!(
        m.selection.menu.as_ref().unwrap().target,
        Target::one(vm_ref(pool, 3))
    );
    assert_eq!(m.scope().kind(), ObjectKind::Pool, "the scope is not moved");
    // A row that does not exist opens nothing.
    m.close_context_menu();
    assert!(!m.open_context_menu(vm_ref(pool, 9999), MenuOrigin::Grid));
    assert!(m.selection.menu.is_none());
    // In the tree the row becomes the scope.
    assert!(m.open_context_menu(vm_ref(pool, 4), MenuOrigin::Tree));
    assert_eq!(m.scope(), &Target::one(vm_ref(pool, 4)));
    assert!(m.context_menu_valid());
}

#[test]
fn stale_menu_cannot_apply_to_a_removed_row_or_changed_pool() {
    // Rune: inventory_menu_tests.rs, the same name.
    let mut m = Model::demo(3, 48);
    let pool = demo_pool(&m);
    let (first, second) = (vm_ref(pool, 1), vm_ref(pool, 2));
    assert!(m.open_context_menu(first.clone(), MenuOrigin::Grid));
    assert!(
        m.context_menu()
            .iter()
            .flatten()
            .any(|item| item.id == crate::areas::example_tests::RENAME && item.disabled.is_none())
    );
    change_inventory(&mut m, |inventory| {
        inventory.vms.remove(&first.vm_id().unwrap());
    });
    assert!(!m.context_menu_valid());
    // A stale menu shows its items disabled, with the reason.
    assert!(
        m.context_menu()
            .iter()
            .flatten()
            .all(|item| item.disabled.as_deref() == Some(STALE_MENU))
    );
    assert_eq!(
        m.invoke_context(crate::areas::example_tests::RENAME),
        Err(Refused::Disabled(STALE_MENU.into()))
    );
    assert!(
        m.selection.menu.is_none(),
        "a refused item still closes the menu"
    );
    assert!(m.surfaces().is_empty());

    assert!(m.open_context_menu(second, MenuOrigin::Grid));
    change_inventory(&mut m, |inventory| {
        inventory.pool_id = uuid::Uuid::from_u128(999).into();
    });
    assert!(!m.context_menu_valid());
    assert!(
        m.invoke_context(crate::areas::example_tests::RENAME)
            .is_err()
    );
    assert!(m.surfaces().is_empty());
}

#[test]
fn changed_selection_cannot_start_a_batch() {
    // Rune: inventory_menu_tests.rs
    // `keyboard_menu_follows_virtual_scrolling_and_changed_selection_cannot_start_a_batch`,
    // the rule about the selection.
    let mut m = Model::demo(3, 48);
    let pool = demo_pool(&m);
    m.select_in_grid(Target::many([vm_ref(pool, 1), vm_ref(pool, 2)]));
    assert!(m.open_context_menu(vm_ref(pool, 1), MenuOrigin::Grid));
    m.selection.grid = Target::one(vm_ref(pool, 5));
    assert!(!m.context_menu_valid());
    assert_eq!(
        m.invoke_context(crate::areas::example_tests::RENAME),
        Err(Refused::Disabled(STALE_MENU.into()))
    );
    assert!(m.selection.menu.is_none());
    assert_eq!(
        m.target(),
        Target::one(vm_ref(pool, 5)),
        "the selection is left alone"
    );
    // A menu that still stands runs its command on the captured selection.
    assert!(m.open_context_menu(vm_ref(pool, 5), MenuOrigin::Grid));
    assert_eq!(
        m.invoke_context(crate::areas::example_tests::RENAME),
        Ok(())
    );
    assert_eq!(m.surfaces().len(), 1);
    assert!(m.selection.menu.is_none());
    // A scope change, a tab change and a new grid selection close the menu.
    for change in [0, 1, 2] {
        assert!(m.open_context_menu(vm_ref(pool, 5), MenuOrigin::Grid));
        match change {
            0 => m.select(Target::one(vm_ref(pool, 6))),
            1 => m.set_active_tab(TabId::Search),
            _ => m.select_in_grid(Target::one(vm_ref(pool, 7))),
        }
        assert!(m.selection.menu.is_none(), "change {change}");
        m.select(Target::one(ObjectRef::pool(pool)));
    }
    assert_eq!(
        m.invoke_context(crate::areas::example_tests::RENAME),
        Err(Refused::Hidden(crate::areas::example_tests::RENAME))
    );
}

// -- Header and target mapping --------------------------------------------------

#[test]
fn the_header_names_the_selection_with_its_state_pool_and_host() {
    let mut m = Model::demo(3, 48);
    let pool = demo_pool(&m);
    m.select(Target::one(vm_ref(pool, 1)));
    assert_eq!(
        m.header(),
        Header {
            icon: ObjectKind::Vm,
            name: "Database".into(),
            state: Some("Running".into()),
            pool: Some("Demonstration pool".into()),
            host: Some("xcp-lab-02".into()),
        }
    );
    assert_eq!(m.window_title(), "Database");
    m.select(Target::one(vm_ref(pool, 0)));
    let header = m.header();
    assert_eq!(
        (header.state.as_deref(), header.host),
        (Some("Halted"), None)
    );
    m.select(Target::one(host_ref(pool, 1)));
    assert_eq!(
        m.header(),
        Header {
            icon: ObjectKind::Host,
            name: "xcp-lab-01".into(),
            state: Some("Enabled".into()),
            pool: Some("Demonstration pool".into()),
            host: None,
        }
    );
    let storage = m.inventory.as_ref().unwrap().storage[0].uuid;
    m.select(Target::one(ObjectRef::storage(pool, storage)));
    let header = m.header();
    assert_eq!(
        (header.icon, header.name.as_str(), header.pool.as_deref()),
        (
            ObjectKind::StorageRepository,
            "Demo storage",
            Some("Demonstration pool")
        )
    );
    m.select(Target::none());
    assert_eq!(m.header().name, "XenAdmin RS");
    assert_eq!(m.window_title(), "Demonstration pool");
}

#[test]
fn the_selected_target_is_a_host_a_repository_or_a_network() {
    // Rune: `resource_admin::selected_target`.
    let mut m = Model::demo(3, 48);
    let pool = demo_pool(&m);
    assert_eq!(
        m.selected_target(),
        None,
        "a pool is not a row of those pages"
    );
    m.select(Target::one(host_ref(pool, 2)));
    assert_eq!(
        m.selected_target(),
        Some(ResourceTarget::Host(host_ref(pool, 2).host_id().unwrap()))
    );
    let storage = m.inventory.as_ref().unwrap().storage[0].uuid;
    m.select(Target::one(ObjectRef::storage(pool, storage)));
    assert_eq!(
        m.selected_target(),
        Some(ResourceTarget::Storage(storage.into()))
    );
    let network = m.inventory.as_ref().unwrap().networks[0].uuid;
    m.select(Target::one(ObjectRef::network(pool, network)));
    assert_eq!(
        m.selected_target(),
        Some(ResourceTarget::Network(network.into()))
    );
    m.select(Target::one(vm_ref(pool, 1)));
    assert_eq!(m.selected_target(), None);
    assert_eq!(m.selected_vm(), vm_ref(pool, 1).vm_id());
    let row = m.selected_row().expect("the row of the VM");
    assert_eq!(
        (row.name.as_str(), row.state.as_str(), row.location.as_str()),
        ("Database", "Running", "xcp-lab-02")
    );
    // A grid selection of one row is what the commands act on.
    m.select_in_grid(Target::one(host_ref(pool, 3)));
    assert_eq!(
        m.selected_target(),
        Some(ResourceTarget::Host(host_ref(pool, 3).host_id().unwrap()))
    );
    assert_eq!(
        resource_target(&ObjectRef::pool(pool)),
        Some(ResourceTarget::Pool(pool))
    );
    assert_eq!(
        resource_target(&ObjectRef::new(None, ObjectKind::Folder, "/Production")),
        None
    );
}

// -- Kernel commands ------------------------------------------------------------

#[test]
fn back_and_forward_are_toolbar_commands_that_say_why_they_are_disabled() {
    let mut m = Model::demo(3, 48);
    let pool = demo_pool(&m);
    let none = Target::none();
    assert_eq!(
        m.eligibility(CommandId("nav.back"), &none),
        Eligibility::disabled("There is no earlier selection to go back to.")
    );
    let toolbar = command::toolbar(&m);
    assert_eq!(
        toolbar[0]
            .iter()
            .map(|item| (item.id.0, item.disabled.is_some()))
            .collect::<Vec<_>>(),
        [("nav.back", true), ("nav.forward", true)]
    );
    m.select(Target::one(vm_ref(pool, 1)));
    assert!(m.invoke(CommandId("nav.back"), &none));
    assert_eq!(m.scope().kind(), ObjectKind::Pool);
    assert_eq!(
        m.eligibility(CommandId("nav.forward"), &none),
        Eligibility::Enabled
    );
    assert!(m.invoke(CommandId("nav.forward"), &none));
    assert_eq!(m.scope(), &Target::one(vm_ref(pool, 1)));
    assert_eq!(
        m.try_invoke(CommandId("nav.forward"), &none),
        Err(Refused::Disabled(
            "There is no later selection to go forward to.".into()
        ))
    );
}

#[test]
fn the_view_menu_toggles_the_tree_and_steps_the_interface_size() {
    let mut m = Model::demo(3, 48);
    let none = Target::none();
    let view = |m: &Model| {
        command::menu_bar(m)
            .into_iter()
            .find(|menu| menu.menu == command::Menu::View)
            .expect("View menu")
    };
    let item = |m: &Model, id: &str| {
        view(m)
            .items()
            .find(|item| item.id.0 == id)
            .cloned()
            .unwrap_or_else(|| panic!("{id} is not in the View menu"))
    };
    assert_eq!(
        view(&m)
            .items()
            .map(|item| item.label.as_str())
            .collect::<Vec<_>>(),
        [
            "Custom Templates",
            "Built-in Templates",
            "Local Storage",
            "Toolbar",
            "Larger Interface",
            "Smaller Interface",
            "Reset Interface Size",
        ]
    );
    assert_eq!(item(&m, "view.custom-templates").checked, Some(true));
    assert_eq!(item(&m, "view.builtin-templates").checked, Some(false));
    assert_eq!(item(&m, "view.local-storage").checked, Some(true));
    assert_eq!(item(&m, "view.toolbar").checked, Some(true));
    assert!(m.invoke(CommandId("view.builtin-templates"), &none));
    assert!(m.invoke(CommandId("view.toolbar"), &none));
    assert!(m.nav.show_builtin_templates && !m.nav.show_toolbar);
    assert_eq!(item(&m, "view.toolbar").checked, Some(false));

    // Ctrl with plus, minus and 0 (the Rune `app_key`).
    assert_eq!(
        item(&m, "view.zoom-reset").disabled.as_deref(),
        Some("The interface is at its normal size.")
    );
    assert_eq!(item(&m, "view.zoom-in").shortcut, Some("ctrl-="));
    assert!(m.invoke(CommandId("view.zoom-in"), &none));
    assert!(m.invoke(CommandId("view.zoom-in"), &none));
    assert_eq!(m.preferences.options.interface_scale, 125);
    for _ in 0..3 {
        assert!(m.invoke(CommandId("view.zoom-out"), &none));
    }
    assert_eq!(m.preferences.options.interface_scale, 90);
    assert!(m.invoke(CommandId("view.zoom-reset"), &none));
    assert_eq!(m.preferences.options.interface_scale, 100);
    m.set_interface_scale(200);
    assert_eq!(
        m.try_invoke(CommandId("view.zoom-in"), &none),
        Err(Refused::Disabled(
            "The interface is at its largest size.".into()
        ))
    );
    m.set_interface_scale(80);
    assert_eq!(
        item(&m, "view.zoom-out").disabled.as_deref(),
        Some("The interface is at its smallest size.")
    );
}

#[test]
fn copy_uuid_is_in_the_context_menu_of_an_object_with_a_uuid() {
    let mut m = Model::demo(3, 48);
    let pool = demo_pool(&m);
    let vm = vm_ref(pool, 1);
    assert!(m.open_context_menu(vm.clone(), MenuOrigin::Grid));
    assert!(
        m.context_menu()
            .iter()
            .flatten()
            .any(|item| item.id.0 == "object.copy-uuid" && item.label == "Copy UUID")
    );
    assert_eq!(m.invoke_context(CommandId("object.copy-uuid")), Ok(()));
    assert!(matches!(m.take_intents().as_slice(), [Intent::CopyText(text)] if *text == vm.id));
    // Not for several objects, and not for a folder.
    assert_eq!(
        m.eligibility(
            CommandId("object.copy-uuid"),
            &Target::many([vm.clone(), vm_ref(pool, 2)])
        ),
        Eligibility::Hidden
    );
    assert_eq!(
        m.eligibility(
            CommandId("object.copy-uuid"),
            &Target::one(ObjectRef::new(None, ObjectKind::Folder, "/Production"))
        ),
        Eligibility::Hidden
    );
}

// -- Two saved connections to one pool -----------------------------------------

#[test]
fn two_saved_connections_to_one_pool_show_it_once_through_the_scope_session() {
    // Rune: search_tests.rs
    // `aliases_deduplicate_by_pool_and_prefer_current_selected_session_without_comparing_revisions`,
    // the part a synthetic session can show: one source per pool, and the
    // scope session wins. (A current connection beating a stale one needs a
    // real snapshot.)
    let mut m = Model::demo(3, 48);
    let pool = demo_pool(&m);
    let mut copy = m.inventory.clone().unwrap().as_ref().clone();
    copy.revision += 100;
    let alias = m.add_synthetic_pool("Alias of the demonstration pool", copy);
    assert_eq!(
        session::session_for_pool(&m, pool),
        Some(crate::demo::profile_id()),
        "the scope session wins, even with a lower revision"
    );
    let rows = crate::nav::view_rows(&m, NavView::Infrastructure);
    let roots: Vec<_> = rows.iter().filter(|row| row.parent.is_none()).collect();
    assert_eq!(roots.len(), 2);
    assert_eq!(
        roots.iter().map(|row| row.icon).collect::<Vec<_>>(),
        [ObjectKind::Connection, ObjectKind::Pool]
    );
    assert_eq!(
        rows.iter().filter(|row| row.icon == ObjectKind::Vm).count(),
        48,
        "the pool's objects are listed once"
    );
    // Selecting the other saved connection makes it the scope session, and
    // the pool is then shown through it.
    m.select(Target::one(ObjectRef::new(
        None,
        ObjectKind::Connection,
        alias,
    )));
    assert_eq!(m.workspace.selected, Some(alias));
    assert_eq!(session::session_for_pool(&m, pool), Some(alias));
    assert!(
        m.reconcile_selection(),
        "the connection node is its pool now"
    );
    assert_eq!(m.scope(), &Target::one(ObjectRef::pool(pool)));
    assert!(m.notice.is_none());
}
