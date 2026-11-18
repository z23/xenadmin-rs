//! The tests of the area.
//!
//! 1. The six tests of `cross_pool_vm_tests.rs` and the six of
//!    `bulk_cross_pool_tests.rs`, each under a name that says which it was.
//!    The two pure ones moved with their logic; the runtime ones are
//!    `Harness` tests asserting the same things. What they asserted of the
//!    two window sizes (the footer and both permission lines reachable at
//!    513x652) is the front end's to show
//!    (`notes/progress/gpui-tests-wanted.md`). What the two history tests
//!    asserted of the Rune transfer history is asserted of the rules that
//!    list served (`can_recheck`, `recheck`, the control connection) and of
//!    what the Events tab says of the same records.
//! 2. This area's share of
//!    `dialogs_outside_the_supported_core_carry_the_experimental_notice`
//!    (`main.rs`).
//! 3. The command, which Rune did not have, and one test for each hazard
//!    the map lists for the two modules (36 to 46).
//!
//! No test here submits anything: the synthetic pools have no observation.
//! `live_tests` has what needs a connection.

use std::sync::Arc;

use uuid::Uuid;
use xenadmin_core::{
    CrossPoolMode, CrossPoolVmOperation, Error, Inventory, OperationControl, OperationId,
    OperationState, PoolId, PowerState, ResourceAction, ResourceOperationRecord, ResourceTarget,
    VmWorkflow,
};

use super::commands::{MOVE_TO_POOL, SINGLE_LABEL};
use super::{batch, scenes, single, view};
use crate::areas::events::{self, Family, OperationKey};
use crate::areas::migration::{self, net_label, sr_label};
use crate::command::{Menu, MenuPath};
use crate::{Harness, prelude::*, session};

fn editor(h: &Harness) -> &single::Editor {
    h.model.areas.cross_pool.wizard.as_deref().expect("wizard")
}

fn editor_mut(h: &mut Harness) -> &mut single::Editor {
    h.model
        .areas
        .cross_pool
        .wizard
        .as_deref_mut()
        .expect("wizard")
}

fn several(h: &Harness) -> &batch::Editor {
    h.model.areas.cross_pool.batch.as_ref().expect("batch")
}

fn several_mut(h: &mut Harness) -> &mut batch::Editor {
    h.model.areas.cross_pool.batch.as_mut().expect("batch")
}

fn amend(h: &mut Harness, edit: impl FnOnce(&mut Inventory)) {
    migration::scenes::amend(&mut h.model, edit);
}

fn step(h: &Harness) -> (usize, usize) {
    match h.surface().kind {
        SurfaceKind::Wizard { steps, current } => (current, steps.len()),
        _ => panic!("not a wizard"),
    }
}

fn moves(review: &batch::Review) -> Vec<&CrossPoolVmOperation> {
    review
        .batch
        .items
        .iter()
        .map(|item| match &item.workflow {
            VmWorkflow::MoveToPool(operation) => operation,
            _ => panic!("not a cross-pool move"),
        })
        .collect()
}

fn available(h: &Harness) -> bool {
    single::availability(&h.model, editor(h)).is_available()
}

fn refused(h: &Harness) -> String {
    batch::build_review(&h.model, several(h)).expect_err("the selection is refused")
}

// -- cross_pool_vm_tests.rs ---------------------------------------------------------

/// `cross_pool_vm_tests.rs`:
/// reviewed_mapping_and_host_transport_warning_render_at_both_widths_without_demo_dispatch
#[test]
fn rune_reviewed_mapping_and_host_transport_warning_without_demo_dispatch() {
    let mut h = Harness::scene("cross-pool");
    let availability = single::availability(&h.model, editor(&h));
    assert!(availability.is_available(), "{}", availability.explanation());
    let operation = editor(&h).review.clone().unwrap();
    assert_ne!(
        operation.destination_pool,
        editor(&h).source.inventory.pool_id
    );
    assert_eq!(h.surface().id, view::WIZARD);
    assert_eq!(step(&h), (2, 3));
    assert_eq!(view::STEPS[2], "Review both pools");
    assert!(h.has_text("unencrypted HTTP"));
    assert!(h.has_text("Both pools are reserved"));
    h.assert_unique_keys();
    // The synthetic pool submits nothing: the button says so, and the
    // handler behind it refuses by itself.
    assert_eq!(h.try_click("cross-apply"), Err(single::DEMO.into()));
    single::apply(&mut h.model);
    assert!(h.model.resource_operations.is_empty());
    assert_eq!(editor(&h).error.as_deref(), Some(single::UNAVAILABLE));
    assert_eq!(editor(&h).review, Some(operation));
    // Withdrawing the acknowledgement makes the reviewed transfer unavailable.
    assert_eq!(h.node("cross-transport-reviewed").checked, Some(true));
    h.click("cross-transport-reviewed");
    assert!(!editor(&h).review.as_ref().unwrap().host_transport_reviewed);
    assert!(!available(&h));
    let reason = single::availability(&h.model, editor(&h)).explanation();
    assert_eq!(h.try_click("cross-apply"), Err(reason.clone()));
    assert!(h.has_text(&reason));
}

/// `cross_pool_vm_tests.rs`:
/// choosing_a_pool_requires_explicit_device_mappings_and_new_transport_review
/// (hazards 36 and 38).
#[test]
fn rune_choosing_a_pool_requires_explicit_device_mappings_and_new_transport_review() {
    let mut h = Harness::scene("cross-pool");
    let e = editor_mut(&mut h);
    e.stage = 0;
    e.review = None;
    let host = e.host.unwrap();
    let sr = e.disks[0].destination_sr;
    let net = e.networks[0].destination_network;
    let (transfer, address) = e.transfer.clone().unwrap();
    assert_eq!(step(&h), (0, 3));
    h.click("cross-pool-0");
    let e = editor(&h);
    assert!(e.disks.iter().all(|d| d.destination_sr.uuid().is_nil()));
    assert!(
        e.networks
            .iter()
            .all(|n| n.destination_network.uuid().is_nil())
    );
    assert!(e.host.is_none() && e.transfer.is_none() && !e.transport_reviewed);
    // Without a host the first step stays.
    h.click("cross-next");
    assert_eq!(editor(&h).stage, 0);
    assert_eq!(editor(&h).error.as_deref(), Some(single::NO_DESTINATION));
    h.click(&format!("cross-host-{host}"));
    h.click("cross-next");
    assert_eq!(editor(&h).stage, 1);
    // Nothing is mapped by default: no option of a list is chosen.
    assert_eq!(h.node("cross-disk-0").value, None);
    h.click("cross-next");
    assert_eq!(editor(&h).stage, 1);
    assert_eq!(
        editor(&h).error.as_deref(),
        Some("Choose an attached destination transfer address")
    );
    h.click(&format!("cross-address-{transfer}-{address}"));
    h.click("cross-next");
    assert_eq!(editor(&h).stage, 1);
    assert_eq!(
        editor(&h).error.as_deref(),
        Some("Map every disk and network interface explicitly")
    );
    h.click(&format!("cross-disk-0-{sr}"));
    h.click(&format!("cross-nic-0-{net}"));
    h.click("cross-next");
    let e = editor(&h);
    assert_eq!(e.stage, 2, "{:?}", e.error);
    assert!(!e.review.as_ref().unwrap().host_transport_reviewed);
    assert!(!available(&h));
    h.click("cross-transport-reviewed");
    assert!(available(&h));
    // Back withdraws the acknowledgement with the review it belonged to.
    h.click("cross-back");
    let e = editor(&h);
    assert_eq!(e.stage, 1);
    assert!(e.review.is_none() && !e.transport_reviewed);
    assert_eq!(e.disks[0].destination_sr, sr);
}

/// `cross_pool_vm_tests.rs`:
/// current_source_membership_and_destination_attachments_gate_review
#[test]
fn rune_current_source_membership_and_destination_attachments_gate_review() {
    let mut h = Harness::scene("cross-pool");
    let o = editor(&h).review.clone().unwrap();
    let e = editor_mut(&mut h);
    Arc::make_mut(&mut e.source.inventory)
        .details
        .vdis
        .get_mut(&o.disks[0].vdi)
        .unwrap()
        .sharable = Some(true);
    assert!(!available(&h));

    let mut h = Harness::scene("cross-pool");
    Arc::make_mut(&mut editor_mut(&mut h).peers[0].inventory)
        .details
        .pbds
        .clear();
    assert!(!available(&h));

    // No session shows the source pool any more.
    let mut h = Harness::scene("cross-pool");
    assert!(single::current(&h.model, editor(&h)));
    amend(&mut h, |i| i.pool_id = PoolId::from(Uuid::from_u128(999)));
    assert!(!single::current(&h.model, editor(&h)));
    assert_eq!(h.try_click("cross-apply"), Err(single::STALE.into()));
    assert!(h.find("cross-review-stale").is_some());
    single::apply(&mut h.model);
    assert!(h.model.resource_operations.is_empty());
}

/// `cross_pool_vm_tests.rs`:
/// halted_copy_review_preserves_source_and_uses_distinct_identity_semantics
#[test]
fn rune_halted_copy_review_preserves_source_and_uses_distinct_identity_semantics() {
    let mut h = Harness::scene("cross-pool");
    let e = editor_mut(&mut h);
    e.vm.power_state = PowerState::Halted;
    e.vm.host = None;
    let id = e.vm.id;
    let i = Arc::make_mut(&mut e.source.inventory);
    i.vms.get_mut(&id).unwrap().power_state = PowerState::Halted;
    i.vms.get_mut(&id).unwrap().host = None;
    e.mode = CrossPoolMode::CopyHalted;
    e.review = Some(single::operation(e).unwrap());
    assert!(available(&h));
    assert!(h.has_text("new MAC addresses"));
    assert!(h.has_text("source stays halted"));
    assert_eq!(h.surface().title, "Copy halted VM to another pool");
    // A halted guest can be moved or copied; a running one only moved.
    editor_mut(&mut h).stage = 0;
    editor_mut(&mut h).review = None;
    h.click("cross-mode-MoveHalted");
    assert_eq!(editor(&h).mode, CrossPoolMode::MoveHalted);
    h.click("cross-mode-CopyHalted");
    assert_eq!(editor(&h).mode, CrossPoolMode::CopyHalted);
    let mut running = Harness::scene("cross-pool");
    editor_mut(&mut running).stage = 0;
    assert!(running.find("cross-mode").is_none());
    assert!(running.has_text(single::LIVE_ONLY));
    single::choose_mode(&mut running.model, "CopyHalted");
    assert_eq!(editor(&running).mode, CrossPoolMode::MoveRunning);
}

fn record(h: &Harness, state: OperationState) -> ResourceOperationRecord {
    let e = editor(h);
    serde_json::from_value(serde_json::json!({
        "id": Uuid::from_u128(9001),
        "pool": e.source.inventory.pool_id,
        "target": ResourceTarget::Pool(e.source.inventory.pool_id),
        "action": ResourceAction::CrossPoolVm(e.review.clone().unwrap()),
        "state": state,
        "progress": 0.5,
        "detail": "Synthetic partial transfer",
        "created_at_ms": 1,
        "can_cancel": false,
        "cancel_requested": false,
        "steps": [],
        "created": null,
        "transfer_created": {
            "vms": [e.vm.id],
            "disks": [Uuid::from_u128(9002)],
            "vbds": [],
            "vifs": [],
            "networks": [],
        },
    }))
    .unwrap()
}

/// `cross_pool_vm_tests.rs`:
/// offline_history_keeps_both_pool_namespaces_and_partial_resources_after_source_disappears.
/// The Rune history list is the History list of the Events tab; what is
/// asserted here is what that list is given and the recheck rule.
#[test]
fn rune_offline_history_keeps_both_pool_namespaces_and_recheck_needs_both_pools() {
    let mut h = Harness::scene("cross-pool");
    let record = record(&h, OperationState::OutcomeUnknown);
    let (id, pool) = (record.id, record.pool);
    let peer = record.peer_pool().unwrap();
    let export = serde_json::to_value(events::export::Entry::resource(&record)).unwrap();
    assert_eq!(export["destination_pool"], peer.to_string());
    assert_eq!(export["target"], editor(&h).vm.id.to_string());
    assert!(!export.to_string().contains("cross_pool_plan"));
    h.model.resource_rows.push(record);
    super::close(&mut h.model);
    h.model.inventory = None;
    // Offline, neither pool has a writable connection: nothing is rechecked.
    assert!(single::connection_for_pool(&h.model, pool).is_none());
    assert!(!single::can_recheck(&h.model, id));
    assert!(matches!(
        single::recheck(&mut h.model, id),
        Err(Error::Closed)
    ));
    assert!(h.model.resource_operations.is_empty());
    let key = OperationKey {
        pool,
        id,
        family: Family::Resource,
    };
    assert_eq!(
        events::control::recheck_eligibility(&h.model, key),
        Eligibility::disabled(events::control::RECHECK_TRANSFER)
    );
    // A record nobody has is a conflict, not a transfer to read again.
    assert!(matches!(
        single::recheck(&mut h.model, OperationId::new()),
        Err(Error::OperationConflict)
    ));
}

/// `cross_pool_vm_tests.rs`:
/// destination_console_requires_proven_result_and_exact_current_pool_vm_host_and_power_state
/// (hazard 45).
#[test]
fn rune_destination_console_requires_proven_result_and_exact_current_pool_vm_host_and_power_state()
 {
    let h = Harness::scene("cross-pool");
    let mut record = record(&h, OperationState::Succeeded);
    let e = editor(&h);
    let operation = e.review.as_ref().unwrap();
    let mut dest = e.peers[0].inventory.as_ref().clone();
    let mut guest = e.vm.clone();
    guest.host = Some(operation.destination_host);
    guest.power_state = PowerState::Running;
    dest.vms.insert(guest.id, guest.clone());
    assert_eq!(
        single::destination_guest(&dest, &record),
        Some((guest.id, true))
    );
    assert_eq!(
        single::destination_label(&record),
        Some("Open destination console")
    );
    record.state = OperationState::OutcomeUnknown;
    assert!(single::destination_guest(&dest, &record).is_none());
    assert_eq!(single::destination_label(&record), None);
    record.state = OperationState::Succeeded;
    dest.vms.get_mut(&guest.id).unwrap().host = None;
    assert!(single::destination_guest(&dest, &record).is_none());
    dest.vms.get_mut(&guest.id).unwrap().host = Some(operation.destination_host);
    dest.vms.get_mut(&guest.id).unwrap().power_state = PowerState::Halted;
    assert!(single::destination_guest(&dest, &record).is_none());
    dest.vms.get_mut(&guest.id).unwrap().power_state = PowerState::Running;
    dest.pool_id = e.source.inventory.pool_id;
    assert!(single::destination_guest(&dest, &record).is_none());
    dest.pool_id = operation.destination_pool;
    dest.vms.clear();
    assert!(single::destination_guest(&dest, &record).is_none());
}

/// Without a connection that proves the guest nothing is selected and the
/// Rune notice says what to wait for.
#[test]
fn showing_the_destination_waits_for_a_connection_that_proves_the_guest() {
    let mut h = Harness::scene("cross-pool");
    let record = record(&h, OperationState::Succeeded);
    let id = record.id;
    assert!(single::destination_target(&h.model, &record).is_none());
    h.model.resource_rows.push(record);
    let scope = h.model.scope().clone();
    assert!(!single::show_destination(&mut h.model, id));
    assert_eq!(h.model.error.as_deref(), Some(single::WAIT));
    assert_eq!(h.model.scope(), &scope);
}

// -- bulk_cross_pool_tests.rs -------------------------------------------------------

/// `bulk_cross_pool_tests.rs`:
/// the_inventory_selection_offers_the_move_to_another_pool_beside_the_same_pool_move
#[test]
fn rune_the_selection_offers_the_move_to_another_pool_beside_the_same_pool_move() {
    let mut m = Model::fixture();
    let target = scenes::batch_fixture(&mut m).expect("two demo guests");
    let mut h = Harness::with(m);
    // The entry is in the VM menu and the context menu, after the move
    // within the pool. An object that is no VM is offered neither.
    let menu = h
        .context_menu(&target)
        .iter()
        .map(|item| item.id.0)
        .collect::<Vec<_>>();
    let same_pool = menu.iter().position(|id| *id == "vm.move").unwrap();
    let other_pool = menu.iter().position(|id| *id == MOVE_TO_POOL.0).unwrap();
    assert!(same_pool < other_pool, "{menu:?}");
    assert_eq!(
        h.menu_item(MOVE_TO_POOL.0).unwrap().label,
        "Move to Another Pool"
    );
    let inventory = h.model.inventory.clone().unwrap();
    let hosts = Target::many(
        inventory
            .hosts
            .keys()
            .take(2)
            .map(|id| ObjectRef::host(inventory.pool_id, *id)),
    );
    assert_eq!(
        h.model.eligibility(MOVE_TO_POOL, &hosts),
        Eligibility::Hidden
    );
    batch::open(&mut h.model, &hosts);
    assert!(h.model.areas.cross_pool.batch.is_none());
    assert_eq!(
        h.model.error.as_deref(),
        Some(migration::batch::ORDINARY),
        "the Rune refusal of the selection"
    );
    h.model.error = None;

    h.invoke(MOVE_TO_POOL.0);
    let e = several(&h);
    assert_eq!(
        e.choices.iter().map(|c| c.source.id).collect::<Vec<_>>(),
        target.vms()
    );
    assert_eq!(e.stage, 0);
    assert!(e.review.is_none() && !e.transport_reviewed);
    assert!(e.destination.is_none() && e.host.is_none());
    assert!(h.model.experimental_notice());
    let surface = h.surface();
    assert_eq!(surface.id, view::BATCH);
    assert_eq!(surface.title, "Move selected VMs to another pool");
    assert_eq!(step(&h), (0, 3));
    assert_eq!(view::BATCH_STEPS[0], "Destination pool and host");
    assert!(h.has_text("2 captured VMs"));
    // The synthetic session has one pool, and the editor says what is needed.
    assert!(h.has_text("Connect another pool with write access"));
    assert!(h.find("batch-cross-pool-close").is_some());
    h.click("batch-cross-pool-next");
    let e = several(&h);
    assert_eq!(e.stage, 0);
    assert_eq!(e.error.as_deref(), Some(batch::NO_DESTINATION));
    assert!(h.find("batch-cross-pool-error").is_some());
    // The editor keeps what it captured when the selection behind it changes.
    h.model.select_in_grid(Target::none());
    assert_eq!(several(&h).choices.len(), 2);
    h.click("batch-cross-pool-close");
    assert!(h.model.areas.cross_pool.batch.is_none());
}

/// `bulk_cross_pool_tests.rs`:
/// the_review_lists_every_vm_with_its_mapping_both_pools_and_a_reachable_footer
/// (hazards 39 and 40).
#[test]
fn rune_the_review_lists_every_vm_with_its_mapping_and_both_pools() {
    let mut h = Harness::scene("batch-cross-pool");
    let e = several(&h);
    let review = e.review.clone().expect("a valid synthetic review");
    assert_eq!(e.stage, 2);
    let operations = moves(&review);
    assert_eq!(operations.len(), 2);
    // A running guest moves live and a halted one halted, in one batch.
    assert_eq!(operations[0].mode, CrossPoolMode::MoveRunning);
    assert_eq!(review.batch.items[0].expected_state, PowerState::Running);
    assert_eq!(operations[1].mode, CrossPoolMode::MoveHalted);
    assert_eq!(review.batch.items[1].expected_state, PowerState::Halted);
    assert!(operations[1].expected_host.is_none());
    // One destination pool, host and address, and the one repository for every disk.
    let target = &e.peers[0].inventory;
    assert_ne!(target.pool_id, e.source.inventory.pool_id);
    for operation in &operations {
        assert_eq!(operation.destination_pool, target.pool_id);
        assert_eq!(Some(operation.destination_host), e.host);
        assert_eq!(operation.transfer_address, operations[0].transfer_address);
        assert!(operation.host_transport_reviewed);
        assert_eq!(operation.disks.len(), 1);
        assert_eq!(operation.networks.len(), 1);
        assert_eq!(Some(operation.disks[0].destination_sr), e.repository);
    }
    assert_ne!(operations[0].disks[0].vdi, operations[1].disks[0].vdi);
    assert_eq!(h.surface().title, "Review moves to another pool");
    assert_eq!(step(&h), (2, 3));
    assert_eq!(view::BATCH_STEPS[2], "Review both pools");
    assert!(h.has_text("Move 2 VMs to another pool"));
    assert!(h.has_text(&target.pool_id.to_string()));
    assert!(h.has_text("Live move: the guest keeps running"));
    assert!(h.has_text("Halted move: the guest stays halted"));
    assert!(h.has_text("Target 1") && h.has_text("Target 2"));
    for (index, operation) in operations.iter().enumerate() {
        let vm = &e.source.inventory.vms[&operation.vm];
        assert!(h.has_text(&format!("{} ({})", vm.name, vm.id)));
        // Each VM's own disk and NIC, with where each goes.
        let lines = &review.rows[index];
        let disk = lines
            .iter()
            .find(|line| line.starts_with(&format!("Disk {}: ", operation.disks[0].vdi)))
            .unwrap_or_else(|| panic!("no disk line in {lines:?}"));
        assert!(disk.contains(&format!(
            "→ {}",
            sr_label(target, operation.disks[0].destination_sr)
        )));
        let nic = lines
            .iter()
            .find(|line| line.starts_with(&format!("NIC {}: ", operation.networks[0].vif)))
            .unwrap_or_else(|| panic!("no NIC line in {lines:?}"));
        assert!(nic.contains(&format!(
            "→ {}",
            net_label(target, operation.networks[0].destination_network)
        )));
        assert!(h.has_text(disk) && h.has_text(nic));
    }
    assert!(
        review
            .capacity
            .contains("the disks of the whole selection need 48.0 GiB")
    );
    assert!(h.has_text("the disks of the whole selection need"));
    // The single form's transport warning, with one acknowledgement for the batch.
    assert!(h.has_text("unencrypted HTTP"));
    assert!(h.has_text("for every VM in this batch"));
    assert!(h.has_text("Both pools are reserved"));
    assert!(h.has_text("never starts a remaining VM"));
    assert!(h.has_text("it does not ask the hosts to cancel a transfer"));
    assert!(h.has_text("The console of a running guest"));
    assert!(h.has_text("Synthetic preview; submission is disabled."));
    assert!(h.model.experimental_notice());
    // The assessment of each pool's session, the acknowledgement and the footer.
    assert!(h.has_text("Source pool session") && h.has_text("Destination pool session"));
    for key in [
        "review-permission-explanation",
        "review-permission-destination",
        "batch-cross-transport-reviewed",
        "batch-cross-pool-close",
        "batch-cross-pool-back",
        "batch-cross-pool-submit",
    ] {
        assert!(h.find(key).is_some(), "{key}");
    }
    h.assert_unique_keys();
    // A synthetic session submits nothing, and the review stands as it was.
    assert_eq!(
        h.try_click("batch-cross-pool-submit"),
        Err(batch::PREVIEW.into())
    );
    batch::submit(&mut h.model);
    assert!(h.model.resource_operations.is_empty());
    assert_eq!(several(&h).review.as_ref(), Some(&review));
    assert_eq!(several(&h).error, Some(Error::Stale.to_string()));
}

/// `bulk_cross_pool_tests.rs`:
/// each_stage_asks_for_its_choices_and_the_acknowledgement_is_given_once_per_review
/// (hazards 37, 38 and 39).
#[test]
fn rune_each_stage_asks_for_its_choices_and_the_acknowledgement_is_given_once_per_review() {
    let mut h = Harness::scene("batch-cross-pool");
    let e = several(&h);
    let (host, repository, transfer) = (
        e.host.unwrap(),
        e.repository.unwrap(),
        e.transfer.clone().unwrap(),
    );
    let network = *e.networks.values().next().unwrap();
    let e = several_mut(&mut h);
    e.stage = 0;
    e.destination = None;
    e.host = None;
    batch::reset_mappings(e);
    h.click("batch-cross-pool-0");
    h.click("batch-cross-pool-next");
    assert_eq!(several(&h).stage, 0, "a host is needed");
    h.click(&format!("batch-cross-host-{host}"));
    h.click("batch-cross-pool-next");
    assert_eq!(several(&h).stage, 1);
    assert_eq!(step(&h), (1, 3));
    assert_eq!(
        view::BATCH_STEPS[1],
        "Storage, networks and transfer address"
    );
    assert!(h.has_text("One destination repository takes every disk of the selection"));
    assert_eq!(
        h.node("batch-cross-pool-next").label.as_deref(),
        Some("Review moves")
    );
    // Nothing is mapped by default in another pool: each choice is asked for.
    for (choose, missing) in [
        (
            format!("batch-cross-sr-{repository}"),
            "Choose the destination repository",
        ),
        (
            format!("batch-cross-net-0-{network}"),
            "Choose a destination network for",
        ),
        (
            format!("batch-cross-address-{}-{}", transfer.0, transfer.1),
            "Choose an attached destination transfer address",
        ),
    ] {
        h.click("batch-cross-pool-next");
        let e = several(&h);
        assert_eq!(e.stage, 1);
        assert!(e.review.is_none());
        assert!(
            e.error.as_deref().is_some_and(|e| e.contains(missing)),
            "{missing}: {:?}",
            e.error
        );
        h.click(&choose);
    }
    h.click("batch-cross-pool-next");
    let e = several(&h);
    assert_eq!(e.stage, 2, "{:?}", e.error);
    // The review is made without the acknowledgement, which belongs to it.
    assert!(!e.transport_reviewed);
    let review = e.review.clone().unwrap();
    for operation in moves(&review) {
        assert!(!operation.host_transport_reviewed);
        assert_eq!(operation.disks[0].destination_sr, repository);
        assert_eq!(operation.networks[0].destination_network, network);
        assert_eq!(operation.transfer_network, transfer.0);
        assert_eq!(operation.transfer_address, transfer.1);
    }
    h.click("batch-cross-transport-reviewed");
    let e = several(&h);
    assert!(e.transport_reviewed);
    assert!(
        moves(e.review.as_ref().unwrap())
            .iter()
            .all(|o| o.host_transport_reviewed)
    );
    // What the choices make of both pools is the review that was acknowledged.
    assert_eq!(
        batch::build_review(&h.model, e).as_ref(),
        Ok(e.review.as_ref().unwrap())
    );
    assert!(h.find("batch-cross-pool-proposal").is_none());
    h.click("batch-cross-transport-reviewed");
    let e = several(&h);
    assert!(!e.transport_reviewed);
    assert!(
        moves(e.review.as_ref().unwrap())
            .iter()
            .all(|o| !o.host_transport_reviewed)
    );
    assert!(h.find("batch-cross-pool-proposal").is_none());
    // Going back ends the review and its acknowledgement; so does another host or pool.
    h.click("batch-cross-transport-reviewed");
    h.click("batch-cross-pool-back");
    let e = several(&h);
    assert_eq!(e.stage, 1);
    assert!(e.review.is_none() && !e.transport_reviewed);
    assert_eq!(e.repository, Some(repository));
    h.click("batch-cross-pool-back");
    h.click(&format!("batch-cross-host-{host}"));
    let e = several(&h);
    assert!(e.repository.is_none() && e.networks.is_empty() && e.transfer.is_none());
    h.click("batch-cross-pool-0");
    assert!(several(&h).host.is_none());
}

/// `bulk_cross_pool_tests.rs`:
/// a_vm_or_destination_that_does_not_fit_refuses_the_whole_selection_and_omits_none
/// (hazard 40).
#[test]
fn rune_a_vm_or_destination_that_does_not_fit_refuses_the_whole_selection_and_omits_none() {
    let scene = || Harness::scene("batch-cross-pool");
    // The second VM was suspended after the editor opened.
    let mut h = scene();
    let id = several(&h).choices[1].source.id;
    let name = several(&h).choices[1].source.name.clone();
    Arc::make_mut(&mut several_mut(&mut h).source.inventory)
        .vms
        .get_mut(&id)
        .unwrap()
        .power_state = PowerState::Suspended;
    let reason = refused(&h);
    assert!(reason.contains(&format!("{name} ({id})")), "{reason}");
    assert!(
        reason.contains("state or resident host changed"),
        "{reason}"
    );
    assert_eq!(several(&h).choices.len(), 2);
    // The review says so and cannot be submitted.
    assert!(h.has_text("state or resident host changed"));
    assert!(h.find("batch-cross-pool-proposal").is_some());
    // It is gone from the source pool.
    let mut h = scene();
    Arc::make_mut(&mut several_mut(&mut h).source.inventory)
        .vms
        .remove(&id);
    assert!(refused(&h).contains(&id.to_string()));
    // Its disk became shared: the single move's own refusal, for that VM.
    let mut h = scene();
    let disk = several(&h).choices[1].disks[0].0;
    Arc::make_mut(&mut several_mut(&mut h).source.inventory)
        .details
        .vdis
        .get_mut(&disk)
        .unwrap()
        .sharable = Some(true);
    let reason = refused(&h);
    assert!(reason.contains(&id.to_string()), "{reason}");
    assert!(
        reason.contains("Shared, unowned, missing, snapshot or CBT disks"),
        "{reason}"
    );
    assert!(!reason.contains(&several(&h).choices[0].source.id.to_string()));
    // The destination repository is no longer attached to the chosen host: both VMs.
    let mut h = scene();
    Arc::make_mut(&mut several_mut(&mut h).peers[0].inventory)
        .details
        .pbds
        .clear();
    let reason = refused(&h);
    for c in &several(&h).choices {
        assert!(reason.contains(&c.source.id.to_string()), "{reason}");
    }
    // The repository reports less free space than the two disks need together.
    let mut h = scene();
    let repository = several(&h).repository.unwrap();
    let sr = Arc::make_mut(&mut several_mut(&mut h).peers[0].inventory)
        .storage
        .iter_mut()
        .find(|sr| sr.uuid == repository.uuid())
        .unwrap();
    sr.capacity_bytes = Some(45 << 30);
    sr.used_bytes = Some(0);
    let reason = refused(&h);
    assert!(
        reason.contains("reports 45.0 GiB free and the disks of the whole selection need 48.0 GiB"),
        "{reason}"
    );
    // No session shows the source pool any more.
    let mut h = scene();
    amend(&mut h, |i| i.pool_id = PoolId::from(Uuid::from_u128(999)));
    assert_eq!(refused(&h), "The captured source pool is no longer current");
    // A VM that was suspended when the editor opened is named on the first stage, and
    // the selection cannot be reviewed.
    let mut h = scene();
    let i = h.model.inventory.clone().unwrap();
    let mut suspended = i.vms[&id].clone();
    suspended.power_state = PowerState::Suspended;
    let e = several_mut(&mut h);
    e.choices[1] = batch::choice(&i, suspended);
    e.stage = 0;
    e.review = None;
    assert!(
        several(&h).choices[1].refusal.as_deref().is_some_and(
            |r| r.contains("A running guest moves live and a halted guest moves halted")
        )
    );
    assert!(refused(&h).contains("resume or shut this one down first"));
    assert!(h.has_text("resume or shut this one down first"));
    assert!(h.find("batch-cross-refusal-1").is_some());
    // Such a guest in the selection disables the command with the same line.
    assert_eq!(
        batch::hopeless(several(&h)),
        Some(format!(
            "{name} ({id}): {}",
            single::unsupported_state(&PowerState::Suspended)
        ))
    );
}

/// `bulk_cross_pool_tests.rs`:
/// a_review_that_no_longer_matches_the_pools_or_lacks_a_current_session_is_not_submitted
#[test]
fn rune_a_review_that_no_longer_matches_the_pools_or_lacks_a_current_session_is_not_submitted() {
    // A change after the review: the wizard says so and nothing is submitted.
    let mut h = Harness::scene("batch-cross-pool");
    let reviewed = several(&h).review.clone().unwrap();
    let id = several(&h).choices[0].source.id;
    Arc::make_mut(&mut several_mut(&mut h).source.inventory)
        .details
        .vms
        .get_mut(&id)
        .unwrap()
        .affinity = None;
    assert!(
        several(&h).choices[0].affinity.is_some(),
        "the synthetic VM has a home host"
    );
    assert!(batch::build_review(&h.model, several(&h)).is_err());
    assert!(h.has_text("Home-host or appliance membership is incomplete or changed"));
    assert!(h.try_click("batch-cross-pool-submit").is_err());
    batch::submit(&mut h.model);
    assert!(h.model.resource_operations.is_empty());
    assert_eq!(several(&h).review.as_ref(), Some(&reviewed));
    // Without the two captured connections nothing is submitted, acknowledged or not.
    let mut h = Harness::scene("batch-cross-pool");
    h.model.demo = false;
    assert!(!batch::current(&h.model, several(&h)));
    assert_eq!(
        h.try_click("batch-cross-pool-submit"),
        Err(batch::STALE.into())
    );
    assert!(h.find("batch-cross-pool-plan-stale").is_some());
    batch::submit(&mut h.model);
    assert!(h.model.resource_operations.is_empty());
    assert_eq!(several(&h).error, Some(Error::Stale.to_string()));
    assert_eq!(several(&h).choices.len(), 2);
    // A session that cannot change this pool is not given a review it could never
    // submit: the entry says what is needed instead.
    let mut m = Model::fixture();
    let target = scenes::batch_fixture(&mut m).expect("two demo guests");
    m.demo = false;
    let mut h = Harness::with(m);
    assert_eq!(
        h.model.eligibility(MOVE_TO_POOL, &target),
        Eligibility::disabled(batch::NOT_WRITABLE)
    );
    batch::open(&mut h.model, &target);
    assert!(h.model.areas.cross_pool.batch.is_none());
    assert!(
        h.model
            .error
            .as_deref()
            .is_some_and(|e| e.contains("needs a current connection to this pool"))
    );
}

/// The batch as its history shows it: unknown after an interruption, with its first move
/// unknown and its second never started.
fn records(h: &Harness) -> (ResourceOperationRecord, ResourceOperationRecord) {
    let e = several(h);
    let batch = e.review.as_ref().unwrap().batch.clone();
    let pool = e.source.inventory.pool_id;
    let (parent, first, second) = (
        Uuid::from_u128(9101),
        Uuid::from_u128(9102),
        Uuid::from_u128(9103),
    );
    let VmWorkflow::MoveToPool(operation) = batch.items[0].workflow.clone() else {
        panic!("not a cross-pool move")
    };
    let batch_record = serde_json::from_value(serde_json::json!({
        "id": parent, "pool": pool, "target": ResourceTarget::Pool(pool),
        "action": ResourceAction::VmWorkflowBatch(batch.clone()),
        "state": OperationState::OutcomeUnknown, "progress": null,
        "detail": "Batch interrupted; both pools stay reserved. Reconnect both pools and recheck",
        "created_at_ms": 1, "can_cancel": false, "cancel_requested": false, "steps": [],
        "created": null,
        "workflow_batch": [
            {"vm": batch.items[0].vm, "child": first, "state": OperationState::OutcomeUnknown, "detail": "Synthetic unknown move"},
            {"vm": batch.items[1].vm, "child": second, "state": null, "detail": "Not started"},
        ],
    }))
    .unwrap();
    let child = serde_json::from_value(serde_json::json!({
        "id": first, "pool": pool, "target": ResourceTarget::Pool(pool),
        "action": ResourceAction::CrossPoolVm(operation), "batch_parent": parent,
        "state": OperationState::OutcomeUnknown, "progress": null,
        "detail": "Synthetic unknown move", "created_at_ms": 2, "can_cancel": false,
        "cancel_requested": false, "steps": [], "created": null,
    }))
    .unwrap();
    (batch_record, child)
}

/// `bulk_cross_pool_tests.rs`:
/// history_shows_a_batch_with_each_vm_and_offers_recheck_stop_and_acknowledgement_on_the_batch_only
/// (hazard 43). The list itself is the Events tab's; here: the rules it
/// asks this area and the session for, and what Events makes of them.
#[test]
fn rune_a_batch_is_rechecked_stopped_and_acknowledged_through_the_batch_only() {
    let mut h = Harness::scene("batch-cross-pool");
    let (batch, child) = records(&h);
    let (parent, first, pool) = (batch.id, child.id, batch.pool);
    let peer = batch.peer_pool().expect("a destination pool");
    assert_eq!(
        Some(peer),
        several(&h).peers.first().map(|p| p.inventory.pool_id)
    );
    assert!(batch.action.spans_pools() && child.action.spans_pools());
    let export = serde_json::to_value(events::export::Entry::resource(&batch)).unwrap();
    assert_eq!(export["destination_pool"], peer.to_string());
    assert!(!export.to_string().contains("workflow_batch_plan"));
    let second = batch.workflow_batch[1].vm;
    assert!(
        events::evidence::resource(&batch)
            .iter()
            .any(|line| line.contains(&second.to_string()) && line.contains("Not started"))
    );
    let mut running = batch.clone();
    h.model.resource_rows.push(batch);
    h.model.resource_rows.push(child);
    super::close(&mut h.model);
    // With neither pool connected nothing can be rechecked, and a move that belongs to a
    // batch is never controlled by itself.
    assert!(!single::can_recheck(&h.model, parent));
    assert!(!single::can_recheck(&h.model, first));
    for control in [
        OperationControl::Recheck,
        OperationControl::RequestCancellation,
        OperationControl::AcknowledgeUnknown,
    ] {
        assert!(session::resource_control_connection(&h.model, first, control).is_none());
    }
    assert!(matches!(
        single::recheck(&mut h.model, parent),
        Err(Error::Closed)
    ));
    assert!(h.model.resource_operations.is_empty());
    let key = |id| OperationKey {
        pool,
        id,
        family: Family::Resource,
    };
    assert_eq!(
        events::control::recheck_eligibility(&h.model, key(parent)),
        Eligibility::disabled(events::control::RECHECK_TRANSFER)
    );
    assert_eq!(
        events::control::recheck_eligibility(&h.model, key(first)),
        Eligibility::disabled(events::control::batch_reason(parent))
    );
    // While a batch runs, Stop is worded as for the other batches.
    running.id = OperationId::new();
    running.state = OperationState::Tracking;
    running.can_cancel = true;
    let id = running.id;
    h.model.resource_rows.push(running);
    assert_eq!(
        events::control::cancel_label(&h.model, key(id)),
        "Finish current VM and stop"
    );
}

// -- The command --------------------------------------------------------------------

#[test]
fn the_command_is_in_the_vm_menu_between_move_and_copy_and_in_the_vm_context_menu() {
    let specs = crate::command::registry();
    let found = specs
        .iter()
        .filter(|spec| spec.id == MOVE_TO_POOL)
        .collect::<Vec<_>>();
    assert_eq!(found.len(), 1);
    let spec = found[0];
    assert_eq!(spec.area, "cross_pool");
    assert_eq!(spec.label, "Move to Another Pool");
    assert_eq!(spec.menu, Some(MenuPath::new(Menu::Vm, 3, 25)));
    assert_eq!(spec.context, &[ObjectKind::Vm]);
    assert!(spec.multi);
    assert!(spec.toolbar.is_none());
}

/// Hazard 46 and the Rune button `cross-pool-open`: shown for an ordinary
/// VM only; disabled where the session cannot change the pool. The wizard
/// never opens on a synthetic pool.
#[test]
fn the_command_on_one_object_is_hidden_for_what_is_no_ordinary_vm_and_disabled_without_a_writable_pool()
 {
    let mut m = Model::fixture();
    let vm = scenes::fixture(&mut m).expect("the demo guest");
    let mut h = Harness::with(m);
    let inventory = h.model.inventory.clone().unwrap();
    let pool = inventory.pool_id;
    let host = *inventory.hosts.keys().next().unwrap();
    for other in [
        Target::one(ObjectRef::host(pool, host)),
        Target::one(ObjectRef::pool(pool)),
        Target::none(),
        Target::many([vm.clone(), ObjectRef::host(pool, host)]),
    ] {
        assert_eq!(
            h.model.eligibility(MOVE_TO_POOL, &other),
            Eligibility::Hidden,
            "{other:?}"
        );
    }
    let gone = Target::one(ObjectRef::vm(pool, xenadmin_core::VmId::from(Uuid::nil())));
    assert_eq!(
        h.model.eligibility(MOVE_TO_POOL, &gone),
        Eligibility::Hidden
    );
    // On one VM the command says that it also copies.
    assert_eq!(h.menu_item(MOVE_TO_POOL.0).unwrap().label, SINGLE_LABEL);
    assert_eq!(
        h.eligibility(MOVE_TO_POOL.0),
        Eligibility::disabled(single::DEMO)
    );
    assert_eq!(
        h.try_invoke(MOVE_TO_POOL.0),
        Err(single::DEMO.to_owned())
    );
    assert!(h.model.areas.cross_pool.wizard.is_none());
    single::open(&mut h.model, &vm);
    assert!(h.model.areas.cross_pool.wizard.is_none());
    // Not synthetic, and no connection either.
    h.model.demo = false;
    assert_eq!(
        h.eligibility(MOVE_TO_POOL.0),
        Eligibility::disabled(single::NOT_WRITABLE)
    );
}

/// The button of the migration wizard that leads here follows the command.
#[test]
fn the_migration_wizard_has_its_button_to_this_command() {
    let mut h = Harness::scene("vm-migration");
    migration::wizard::back(&mut h.model);
    let node = h.node("migration-cross-pool");
    assert_eq!(node.label.as_deref(), Some("Move or Copy to Another Pool…"));
    assert_eq!(node.disabled.as_deref(), Some(single::DEMO));
}

/// `main.rs`: dialogs_outside_the_supported_core_carry_the_experimental_notice,
/// this area's share (`cross_pool.show` and `bulk_cross_pool`).
#[test]
fn rune_experimental_notice_is_on_both_wizards() {
    for scene in ["cross-pool", "batch-cross-pool"] {
        let h = Harness::scene(scene);
        assert!(h.surface().experimental, "{scene}");
        assert!(h.model.experimental_notice(), "{scene}");
    }
}

#[test]
fn both_wizards_close_on_escape_and_when_the_scope_or_the_session_changes() {
    for scene in ["cross-pool", "batch-cross-pool"] {
        let open = |h: &Harness| {
            h.model.areas.cross_pool.wizard.is_some() || h.model.areas.cross_pool.batch.is_some()
        };
        let mut h = Harness::scene(scene);
        assert!(open(&h));
        h.dismiss();
        assert!(!open(&h), "{scene}: Escape");
        let mut h = Harness::scene(scene);
        let pool = h.model.inventory.as_ref().unwrap().pool_id;
        h.select(Target::one(ObjectRef::pool(pool)));
        assert!(!open(&h), "{scene}: scope");
        let mut h = Harness::scene(scene);
        h.model.load_demo(2, 8);
        assert!(!open(&h), "{scene}: session");
    }
}

/// Every step of both wizards describes itself with keys that occur once.
#[test]
fn every_step_has_unique_keys() {
    let mut h = Harness::scene("cross-pool");
    for stage in [2, 1, 0] {
        editor_mut(&mut h).stage = stage;
        h.assert_unique_keys();
    }
    let mut h = Harness::scene("batch-cross-pool");
    for stage in [2, 1, 0] {
        several_mut(&mut h).stage = stage;
        h.assert_unique_keys();
    }
}

/// Hazard 37: in the single wizard another host ends the transfer address
/// and keeps the device mappings (the review catches a repository the new
/// host has not attached).
#[test]
fn another_host_keeps_the_mappings_of_the_single_wizard_and_ends_its_address() {
    let mut h = Harness::scene("cross-pool");
    let e = editor_mut(&mut h);
    e.stage = 0;
    e.review = None;
    let (disks, networks) = (e.disks.clone(), e.networks.clone());
    let other = e.peers[0]
        .inventory
        .hosts
        .values()
        .find(|host| host.enabled && Some(host.id) != e.host)
        .map(|host| host.id)
        .expect("a second host");
    h.click(&format!("cross-host-{other}"));
    let e = editor(&h);
    assert_eq!(e.host, Some(other));
    assert!(e.transfer.is_none());
    assert_eq!((&e.disks, &e.networks), (&disks, &networks));
}

/// Hazard 41: the destination is an index into the pools captured at open,
/// and an index outside them chooses nothing.
#[test]
fn a_destination_outside_the_captured_pools_is_not_chosen() {
    let mut h = Harness::scene("cross-pool");
    let before = editor(&h).disks.clone();
    single::choose_pool(&mut h.model, "7");
    single::choose_pool(&mut h.model, "x");
    assert_eq!(editor(&h).destination, Some(0));
    assert_eq!(editor(&h).disks, before);
    let mut h = Harness::scene("batch-cross-pool");
    batch::choose_pool(&mut h.model, "7");
    assert_eq!(several(&h).destination, Some(0));
    assert!(several(&h).review.is_some());
}

/// AREAS.md decision 10: the texts that named Rune places name the
/// commands and the tab this client has.
#[test]
fn texts_name_the_places_of_this_client() {
    assert!(single::NO_PEERS.contains("using Add Server… or Connect… of the Server menu, then reopen this transfer"));
    assert!(batch::NO_PEERS.contains("using Add Server… or Connect… of the Server menu, then reopen this review"));
    assert!(single::CONSOLE.contains("use Open destination console from transfer history"));
    assert!(single::RESERVED.contains("Verified destination UUIDs remain on the Events tab of the Notifications view"));
    assert!(batch::SELECTION_HELP.contains("with Move or Copy to Another Pool on that VM"));
    assert!(batch::INTERRUPTION.contains("Reconnect both pools and use Recheck"));
    assert!(batch::CONSOLE.contains("open it from the destination pool"));
}
