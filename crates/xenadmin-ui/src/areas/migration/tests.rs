//! The tests of the area.
//!
//! 1. The four tests of `vm_migration_tests.rs` and the three of
//!    `bulk_move_tests.rs`, each under a name that says which it was. The
//!    two pure ones of each file moved with their logic; the runtime ones
//!    are `Harness` tests asserting the same things. What they asserted of
//!    the two window sizes (the permission line and the confirm button
//!    reachable at 513x652) is the front end's to show
//!    (`notes/progress/gpui-tests-wanted.md`).
//! 2. This area's share of
//!    `dialogs_outside_the_supported_core_carry_the_experimental_notice`
//!    (`main.rs`).
//! 3. The commands, which Rune did not have, and one test for each hazard
//!    the map lists for the two modules.
//!
//! No test here submits anything: the demo pool has no observation, and
//! the fixture server (`live_tests`) serves no migration.

use uuid::Uuid;
use xenadmin_core::{Error, HostId, Inventory, NetworkId, PoolId, PowerState, SrId, VmId};

use super::commands::{MIGRATE, MOVE, MOVE_TO_POOL};
use super::{batch, scenes, view, wizard};
use crate::{Harness, prelude::*};

fn editor(h: &Harness) -> &wizard::Editor {
    h.model.areas.migration.wizard.as_deref().expect("wizard")
}

fn editor_mut(h: &mut Harness) -> &mut wizard::Editor {
    h.model
        .areas
        .migration
        .wizard
        .as_deref_mut()
        .expect("wizard")
}

fn several(h: &Harness) -> &batch::Editor {
    h.model.areas.migration.batch.as_ref().expect("batch")
}

fn amend(h: &mut Harness, edit: impl FnOnce(&mut Inventory)) {
    scenes::amend(&mut h.model, edit);
}

fn inventory(h: &Harness) -> &Inventory {
    h.model.inventory.as_deref().expect("inventory")
}

/// The fixture of the single scene with the VM selected and nothing open.
fn prepared() -> (Harness, ObjectRef) {
    let mut m = Model::fixture();
    let vm = scenes::prepare(&mut m).expect("the demo guest");
    (Harness::with(m), vm)
}

/// The fixture of the batch scene with both VMs selected and nothing open.
fn prepared_batch() -> (Harness, Target) {
    let mut m = Model::fixture();
    let target = scenes::prepare_batch(&mut m).expect("two demo guests");
    (Harness::with(m), target)
}

fn step(h: &Harness) -> (usize, usize) {
    match h.surface().kind {
        SurfaceKind::Wizard { steps, current } => (current, steps.len()),
        _ => panic!("not a wizard"),
    }
}

fn first_network(h: &Harness) -> NetworkId {
    NetworkId::from(inventory(h).networks[0].uuid)
}

// -- vm_migration_tests.rs ----------------------------------------------------------

/// `compact_and_wide_review_capture_exact_targets_and_remain_preview_only`.
#[test]
fn rune_review_captures_exact_targets_and_remains_preview_only() {
    let mut h = Harness::scene("vm-migration");
    let f = editor(&h);
    let o = f.review.clone().expect("review");
    assert!(o.availability(inventory(&h), f.source.id).is_available());
    assert_eq!(o.disks.len(), 1);
    assert_eq!(o.networks.len(), 1);
    assert_ne!(o.expected_host, Some(o.destination));
    // "3 of 3": the last of the three steps of the wizard.
    assert_eq!(h.surface().id, view::WIZARD);
    assert_eq!(step(&h), (2, 3));
    assert!(h.has_text("partial placement"));
    assert!(h.find(permissions::REVIEW_KEY).is_some());
    // The demo cannot apply: the button says so, and the handler behind it
    // is inert as well.
    assert_eq!(h.try_click("migration-apply"), Err(wizard::DEMO.into()));
    wizard::apply(&mut h.model);
    assert!(h.model.operations.is_empty());
    assert_eq!(editor(&h).review, Some(o));
    h.assert_unique_keys();
}

/// `destination_and_mapping_controls_produce_reviewed_typed_plan`. The
/// Rune test left the destination and the transfer network of the scene
/// in place; here they are emptied first, so the controls set them.
#[test]
fn rune_destination_and_mapping_controls_produce_reviewed_typed_plan() {
    let mut h = Harness::scene("vm-migration");
    let f = editor_mut(&mut h);
    let dest = f.destination.take().expect("destination of the scene");
    f.transfer = None;
    f.stage = 0;
    f.review = None;
    let disk = f.disks[0].destination_sr;
    let net = f.networks[0].destination_network;
    assert_eq!(step(&h), (0, 3));
    h.click(&format!("migration-host-{dest}"));
    h.click("migration-next");
    assert_eq!(step(&h), (1, 3));
    h.assert_unique_keys();
    h.click(&format!("migration-disk-0-{disk}"));
    h.click(&format!("migration-nic-0-{net}"));
    h.click(&format!("migration-transfer-{net}"));
    h.click("migration-next");
    assert_eq!(step(&h), (2, 3));
    let o = editor(&h).review.as_ref().expect("review");
    assert_eq!(o.destination, dest);
    assert_eq!(o.transfer_network, Some(net));
}

/// `stale_or_missing_vm_and_changed_devices_prevent_apply`.
#[test]
fn rune_stale_or_missing_vm_and_changed_devices_prevent_apply() {
    let mut h = Harness::scene("vm-migration");
    let f = editor(&h);
    let id = f.source.id;
    let o = f.review.clone().expect("review");
    amend(&mut h, |i| {
        i.details.vifs.get_mut(&o.networks[0].vif).unwrap().network = None;
    });
    let status = o.availability(inventory(&h), id);
    assert!(!status.is_available());
    // The review says what the core says, and so does the button.
    assert_eq!(
        h.try_click("migration-apply"),
        Err(status.explanation().to_string())
    );
    wizard::apply(&mut h.model);
    assert!(h.model.operations.is_empty());
    amend(&mut h, |i| i.pool_id = PoolId::from(Uuid::from_u128(999)));
    assert!(!wizard::current(&h.model, editor(&h)));
    assert_eq!(h.try_click("migration-apply"), Err(wizard::STALE.into()));
    assert!(h.find("migration-review-stale").is_some());
    amend(&mut h, |i| {
        i.vms.remove(&id);
    });
    wizard::apply(&mut h.model);
    assert!(h.model.operations.is_empty());
    assert!(h.model.areas.migration.wizard.is_some());
}

/// `halted_storage_move_requires_transfer_network_and_fresh_home_host`.
#[test]
fn rune_halted_storage_move_requires_transfer_network_and_fresh_home_host() {
    let mut h = Harness::scene("vm-migration");
    let transfer = first_network(&h);
    let f = editor_mut(&mut h);
    f.source.power_state = PowerState::Halted;
    f.source.host = None;
    f.transfer = None;
    assert!(wizard::operation(f).is_err());
    f.transfer = Some(transfer);
    let o = wizard::operation(f).unwrap();
    assert!(!o.live);
    let id = f.source.id;
    amend(&mut h, |i| {
        i.vms.get_mut(&id).unwrap().power_state = PowerState::Halted;
        i.vms.get_mut(&id).unwrap().host = None;
    });
    assert!(o.availability(inventory(&h), id).is_available());
    amend(&mut h, |i| {
        i.details.vms.get_mut(&id).unwrap().affinity = Some(o.destination);
    });
    assert!(!o.availability(inventory(&h), id).is_available());
    // The title follows the captured power state.
    assert_eq!(h.surface().title, "Move halted VM");
}

// -- bulk_move_tests.rs -------------------------------------------------------------

/// `compact_and_wide_review_capture_all_vms_and_keep_safe_footer`.
#[test]
fn rune_batch_review_captures_all_vms_and_keeps_safe_footer() {
    let mut h = Harness::scene("batch-move");
    let review = several(&h).review.clone().expect("valid synthetic review");
    assert_eq!(review.batch.items.len(), 2);
    assert_eq!(h.surface().id, view::BATCH);
    assert_eq!(step(&h), (1, 2));
    assert!(h.has_text("Live migration"));
    assert!(h.has_text("Recovery never starts"));
    assert!(h.find(permissions::REVIEW_KEY).is_some());
    assert!(h.find("batch-move-submit").is_some());
    // Captured at open: another selection in the grid does not retarget.
    let other = inventory(&h)
        .guests()
        .map(|vm| vm.id)
        .find(|id| ![scenes::demo_vm(), scenes::second_demo_vm()].contains(id))
        .expect("a third guest");
    let pool = several(&h).pool;
    h.model
        .select_in_grid(Target::one(ObjectRef::vm(pool, other)));
    assert_eq!(several(&h).review.as_ref(), Some(&review));
    assert_eq!(h.try_click("batch-move-submit"), Err(wizard::DEMO.into()));
    assert!(h.find("batch-move-preview").is_some());
    batch::submit(&mut h.model);
    assert!(h.model.resource_operations.is_empty());
    assert_eq!(several(&h).error, Some(Error::Stale.to_string()));
    h.assert_unique_keys();
}

/// `mapping_edits_are_per_target_and_review_exact_prior_and_new_values`.
#[test]
fn rune_batch_mapping_edits_are_per_target_and_review_exact_prior_and_new_values() {
    // The draft of the scene, entered through its controls, in a pool that
    // has a second repository (the demo pool has one).
    let (mut h, target) = prepared_batch();
    let other = SrId::from(Uuid::from_u128(0x5151));
    amend(&mut h, |i| {
        let mut spare = i.storage[0].clone();
        spare.uuid = other.uuid();
        spare.name = "Spare repository".into();
        i.storage.push(spare);
    });
    h.invoke_on(MOVE.0, &target);
    let resident = several(&h).choices[0].source.host;
    let destination = inventory(&h)
        .hosts
        .keys()
        .copied()
        .find(|host| Some(*host) != resident)
        .expect("another host");
    h.click(&format!("batch-move-host-{destination}"));
    h.click(&format!("batch-move-transfer-{}", first_network(&h)));
    let id = several(&h).choices[0].disks[0].destination_sr;
    assert_ne!(id, other);
    h.click("batch-move-mappings-0");
    h.assert_unique_keys();
    // An edit lands in its own target and nowhere else.
    let untouched = several(&h).choices[1].disks.clone();
    h.click(&format!("batch-move-disk-0-0-{other}"));
    assert_eq!(several(&h).choices[0].disks[0].destination_sr, other);
    assert_eq!(several(&h).choices[0].disks[0].expected_sr, id);
    assert_eq!(several(&h).choices[1].disks, untouched);
    h.click(&format!("batch-move-disk-0-0-{id}"));
    h.click("batch-move-review");
    let review = several(&h).review.as_ref().expect("review");
    assert!(
        review.rows[0]
            .iter()
            .any(|v| v.contains("Disk ") && v.contains('→'))
    );
    assert!(review.rows[1].iter().all(|v| !v.starts_with("Disk ")));
}

/// `changed_or_unsupported_targets_refuse_whole_selection_without_omission`.
#[test]
fn rune_batch_changed_or_unsupported_targets_refuse_whole_selection_without_omission() {
    for missing in [false, true] {
        let mut h = Harness::scene("batch-move");
        let id = several(&h).choices[1].source.id;
        amend(&mut h, |i| {
            if missing {
                i.vms.remove(&id);
            } else {
                i.vms.get_mut(&id).unwrap().power_state = PowerState::Suspended;
            }
        });
        let refusal = batch::build_review(&h.model, several(&h)).expect_err("refused");
        assert!(refusal.contains("VM identity, state or resident host changed"));
        // The live refusal is on screen, in the danger line.
        assert_eq!(h.node("batch-move-proposal").text, Some(refusal));
        batch::submit(&mut h.model);
        assert!(h.model.resource_operations.is_empty());
        assert_eq!(several(&h).choices.len(), 2);
    }
    let mut h = Harness::scene("batch-move");
    h.model.demo = false;
    assert!(!batch::current(&h.model, several(&h)));
    assert_eq!(
        h.try_click("batch-move-submit"),
        Err(batch::STALE.to_owned())
    );
    assert!(h.find("batch-move-plan-stale").is_some());
    batch::submit(&mut h.model);
    assert!(h.model.resource_operations.is_empty());
}

// -- main.rs ------------------------------------------------------------------------

/// `main.rs`: dialogs_outside_the_supported_core_carry_the_experimental_notice,
/// for the two surfaces of this area: the move of one VM is supported, the
/// batch is not.
#[test]
fn rune_experimental_notice_is_on_the_batch_and_not_on_the_single_move() {
    let single = Harness::scene("vm-migration");
    assert!(!single.surface().experimental);
    assert!(!single.model.experimental_notice());
    let batch = Harness::scene("batch-move");
    assert!(batch.surface().experimental);
    assert!(batch.model.experimental_notice());
}

// -- The commands -------------------------------------------------------------------

#[test]
fn both_commands_open_the_wizard_of_one_vm_on_its_first_step() {
    for command in [MIGRATE, MOVE] {
        let (mut h, vm) = prepared();
        assert_eq!(h.eligibility(command.0), Eligibility::Enabled);
        assert!(h.menu_item(command.0).is_some(), "{command:?} in the menu");
        let target = Target::one(vm.clone());
        assert!(
            h.context_menu(&target)
                .iter()
                .any(|item| item.id == command),
            "{command:?} in the context menu"
        );
        h.invoke(command.0);
        let surface = h.surface();
        assert_eq!(
            (surface.id.as_str(), surface.title.as_str()),
            (view::WIZARD, "Migrate running VM")
        );
        assert_eq!(step(&h), (0, 3));
        let f = editor(&h);
        assert_eq!(Some(f.source.id), vm.vm_id());
        assert_eq!((f.destination, f.transfer, f.review.is_none()), (None, None, true));
        // Identity mappings to start with.
        assert!(f.disks.iter().all(|d| d.destination_sr == d.expected_sr));
        assert!(
            f.networks
                .iter()
                .all(|n| n.destination_network == n.expected_network)
        );
        // Every host is offered, the current one included (hazard 83).
        let hosts = inventory(&h).hosts.keys().copied().collect::<Vec<HostId>>();
        assert!(hosts.contains(&f.source.host.expect("a resident host")));
        for host in hosts {
            assert!(h.find(&format!("migration-host-{host}")).is_some());
        }
        assert!(h.find("migration-back").is_none());
        assert!(h.find(permissions::REVIEW_KEY).is_none());
        h.assert_unique_keys();
        h.click("migration-cancel");
        assert!(h.model.areas.migration.wizard.is_none());
    }
}

#[test]
fn the_command_is_hidden_or_disabled_where_rune_hid_or_disabled_its_button() {
    let (mut h, vm) = prepared();
    let pool = vm.pool.unwrap();
    let id = vm.vm_id().unwrap();
    // Neither running nor halted: the caption of the Rune button.
    amend(&mut h, |i| {
        i.vms.get_mut(&id).unwrap().power_state = PowerState::Suspended;
    });
    assert_eq!(
        h.eligibility(MIGRATE.0),
        Eligibility::disabled(wizard::UNSUPPORTED_STATE)
    );
    assert_eq!(
        h.try_invoke(MOVE.0),
        Err(wizard::UNSUPPORTED_STATE.to_owned())
    );
    assert!(h.model.areas.migration.wizard.is_none());
    // No button at all: another kind, a control domain, a VM that is gone.
    let host = *inventory(&h).hosts.keys().next().unwrap();
    for target in [
        Target::one(ObjectRef::host(pool, host)),
        Target::one(ObjectRef::pool(pool)),
        Target::one(ObjectRef::vm(pool, VmId::from(Uuid::from_u128(7)))),
        Target::none(),
    ] {
        assert_eq!(h.model.eligibility(MIGRATE, &target), Eligibility::Hidden);
        assert_eq!(h.model.eligibility(MOVE, &target), Eligibility::Hidden);
    }
    amend(&mut h, |i| {
        let vm = i.vms.get_mut(&id).unwrap();
        vm.power_state = PowerState::Halted;
        vm.is_control_domain = true;
    });
    assert_eq!(h.eligibility(MIGRATE.0), Eligibility::Hidden);
    // Outside the demo, without a writable current connection.
    amend(&mut h, |i| i.vms.get_mut(&id).unwrap().is_control_domain = false);
    assert_eq!(h.eligibility(MIGRATE.0), Eligibility::Enabled);
    h.model.demo = false;
    assert_eq!(
        h.eligibility(MIGRATE.0),
        Eligibility::disabled(wizard::NOT_WRITABLE)
    );
}

#[test]
fn the_same_commands_open_the_batch_on_several_vms() {
    for command in [MIGRATE, MOVE] {
        let (mut h, target) = prepared_batch();
        assert_eq!(h.model.eligibility(command, &target), Eligibility::Enabled);
        h.invoke_on(command.0, &target);
        assert!(h.model.areas.migration.wizard.is_none());
        let surface = h.surface();
        assert_eq!(
            (surface.id.as_str(), surface.title.as_str()),
            (view::BATCH, "Move selected VMs")
        );
        assert_eq!(step(&h), (0, 2));
        let e = several(&h);
        assert_eq!(
            e.choices.iter().map(|c| c.source.id).collect::<Vec<_>>(),
            target.vms()
        );
        assert!(e.choices.iter().all(|c| c.refusal.is_none()));
        assert_eq!((e.destination, e.transfer, e.expanded), (None, None, None));
        // Nothing to review before a destination is chosen.
        assert_eq!(
            h.try_click("batch-move-review"),
            Err("Choose a destination host for the selection".into())
        );
        assert!(h.find(permissions::REVIEW_KEY).is_none());
        h.assert_unique_keys();
        h.click("batch-move-close");
        assert!(h.model.areas.migration.batch.is_none());
    }
}

#[test]
fn a_selection_that_cannot_be_moved_disables_the_command_with_its_refusal() {
    let (mut h, target) = prepared_batch();
    let pool = target.pool().unwrap();
    let object = |id: VmId| ObjectRef::vm(pool, id);
    let refusal = |h: &Harness, target: &Target| h.model.eligibility(MOVE, target);
    // More than 32.
    let many = Target::many(inventory(&h).guests().take(33).map(|vm| object(vm.id)));
    assert_eq!(many.len(), 33);
    assert_eq!(refusal(&h, &many), Eligibility::disabled(batch::LIMIT));
    // Not all VMs: the command does not apply.
    let host = *inventory(&h).hosts.keys().next().unwrap();
    let mixed = Target::many([object(scenes::demo_vm()), ObjectRef::host(pool, host)]);
    assert_eq!(refusal(&h, &mixed), Eligibility::Hidden);
    // A VM the inventory does not have.
    let gone = Target::many([
        object(scenes::demo_vm()),
        object(VmId::from(Uuid::from_u128(7))),
    ]);
    assert_eq!(refusal(&h, &gone), Eligibility::disabled(batch::MISSING));
    // Two pools.
    let elsewhere = PoolId::from(Uuid::from_u128(8));
    let two = Target::many([
        object(scenes::demo_vm()),
        ObjectRef::vm(elsewhere, scenes::second_demo_vm()),
    ]);
    assert_eq!(refusal(&h, &two), Eligibility::disabled(batch::ONE_POOL));
    // One VM that cannot move refuses the selection; nothing is left out.
    let second = scenes::second_demo_vm();
    let name = inventory(&h).vms[&second].name.clone();
    amend(&mut h, |i| {
        i.vms.get_mut(&second).unwrap().power_state = PowerState::Suspended;
    });
    let reason = format!("{name} ({second}): Only ordinary running or halted VMs can move");
    assert_eq!(refusal(&h, &target), Eligibility::disabled(reason.clone()));
    assert_eq!(h.try_invoke_on(MIGRATE.0, &target), Err(reason));
    assert!(h.model.areas.migration.batch.is_none());
    // Outside the demo, without a writable current connection.
    amend(&mut h, |i| {
        i.vms.get_mut(&second).unwrap().power_state = PowerState::Running;
    });
    assert_eq!(refusal(&h, &target), Eligibility::Enabled);
    h.model.demo = false;
    assert_eq!(
        refusal(&h, &target),
        Eligibility::disabled(wizard::NOT_WRITABLE)
    );
}

// -- Hazards ------------------------------------------------------------------------

/// The first step is not left without a destination, the refusal is in
/// words, and only a step that succeeds clears it.
#[test]
fn next_without_a_destination_is_refused_and_back_keeps_the_error() {
    let (mut h, _) = prepared();
    h.invoke(MIGRATE.0);
    h.click("migration-next");
    assert_eq!(step(&h), (0, 3));
    assert_eq!(
        h.node("migration-error").text.as_deref(),
        Some(wizard::NO_DESTINATION)
    );
    let host = *inventory(&h).hosts.keys().nth(1).unwrap();
    h.click(&format!("migration-host-{host}"));
    assert_eq!(editor(&h).destination, Some(host));
    h.click("migration-next");
    assert_eq!(step(&h), (1, 3));
    assert!(h.find("migration-error").is_none());
    // Back drops the review and keeps an error (as in Rune).
    let mut h = Harness::scene("vm-migration");
    editor_mut(&mut h).error = Some("kept".into());
    h.click("migration-back");
    assert_eq!(step(&h), (1, 3));
    let f = editor(&h);
    assert_eq!((f.review.is_none(), f.error.as_deref()), (true, Some("kept")));
    assert!(h.find(permissions::REVIEW_KEY).is_none());
    h.click("migration-back");
    assert_eq!(step(&h), (0, 3));
    assert!(h.find("migration-back").is_none());
}

/// The transfer network can be changed but there is no option that unsets
/// it (hazard 84), and the review names it only for a storage transport.
#[test]
fn the_transfer_network_cannot_be_unset_and_shows_only_for_storage_transport() {
    let mut h = Harness::scene("vm-migration");
    let o = editor(&h).review.clone().unwrap();
    assert!(!o.uses_storage_transport());
    assert!(o.transfer_network.is_some());
    assert!(!h.has_text("Transfer network:"));
    h.click("migration-back");
    let networks = inventory(&h)
        .networks
        .iter()
        .map(|n| NetworkId::from(n.uuid))
        .collect::<Vec<_>>();
    let options = h
        .keys()
        .into_iter()
        .filter(|key| key.starts_with("migration-transfer-"))
        .count();
    assert_eq!(options, networks.len());
    let last = *networks.last().unwrap();
    h.click(&format!("migration-transfer-{last}"));
    assert_eq!(editor(&h).transfer, Some(last));
}

/// The texts that name Rune places or another area's workflow, pinned for
/// the one sweep that renames them (decision 10 of the area list).
#[test]
fn texts_that_name_other_places_are_carried_over_word_for_word() {
    let mut h = Harness::scene("vm-migration");
    assert!(h.has_text("verified new disks are recorded in operation history"));
    h.click("migration-back");
    assert!(h.has_text("Use Move / copy to another pool for cross-pool transfers."));
    // The button under that caption is the `cross_pool` area's command: it
    // is absent until that area registers it.
    let target = h.model.target();
    assert_eq!(
        h.model.eligibility(MOVE_TO_POOL, &target),
        Eligibility::Hidden
    );
    assert!(h.find("migration-cross-pool").is_none());
    let h = Harness::scene("batch-move");
    assert!(h.has_text("This review covers this pool only."));
}

/// Both surfaces belong to what they captured: another object in the tree
/// or another session closes them.
#[test]
fn a_scope_or_session_change_closes_both_surfaces() {
    for scene in ["vm-migration", "batch-move"] {
        let mut h = Harness::scene(scene);
        assert_eq!(h.surfaces().len(), 1);
        let pool = inventory(&h).pool_id;
        h.select(Target::one(ObjectRef::pool(pool)));
        assert!(h.surfaces().is_empty(), "{scene}: scope");
        let mut h = Harness::scene(scene);
        h.model.load_demo(3, 48);
        assert!(h.surfaces().is_empty(), "{scene}: session");
    }
}

/// The batch draft: one target's mappings are open at a time, a disabled
/// host cannot be chosen, Back keeps the draft.
#[test]
fn the_batch_draft_opens_one_target_at_a_time_and_back_keeps_it() {
    let (mut h, target) = prepared_batch();
    let disabled = *inventory(&h).hosts.keys().last().unwrap();
    amend(&mut h, |i| i.hosts.get_mut(&disabled).unwrap().enabled = false);
    h.invoke_on(MOVE.0, &target);
    assert_eq!(
        h.try_click(&format!("batch-move-host-{disabled}")),
        Err(batch::HOST_DISABLED.into())
    );
    assert_eq!(several(&h).destination, None);
    h.click("batch-move-mappings-0");
    assert_eq!(several(&h).expanded, Some(0));
    assert_eq!(
        h.node("batch-move-mappings-0").label.as_deref(),
        Some("Close device mappings")
    );
    h.click("batch-move-mappings-1");
    assert_eq!(several(&h).expanded, Some(1));
    assert_eq!(
        h.node("batch-move-mappings-0").label.as_deref(),
        Some("Review / edit device mappings")
    );
    h.click("batch-move-mappings-1");
    assert_eq!(several(&h).expanded, None);
    // From the review of the scene, Back returns to the same draft.
    let mut h = Harness::scene("batch-move");
    let (destination, transfer) = (several(&h).destination, several(&h).transfer);
    assert!(destination.is_some() && transfer.is_some());
    h.click("batch-move-back");
    assert_eq!(step(&h), (0, 2));
    let e = several(&h);
    assert_eq!(
        (e.review.is_none(), e.destination, e.transfer, e.choices.len()),
        (true, destination, transfer, 2)
    );
    assert_eq!(h.surface().title, "Move selected VMs");
    // And the transfer network is a control of the draft.
    h.model.areas.migration.batch.as_mut().unwrap().transfer = None;
    let network = first_network(&h);
    h.click(&format!("batch-move-transfer-{network}"));
    assert_eq!(several(&h).transfer, Some(network));
}

/// A VM that was refused when captured is named in the draft and refuses
/// the review of the whole selection (hazard 34). Such a selection does
/// not open through the command; the editor is opened directly.
#[test]
fn a_refused_target_is_named_in_the_draft_and_refuses_the_whole_review() {
    let (mut h, target) = prepared_batch();
    let second = scenes::second_demo_vm();
    amend(&mut h, |i| {
        i.details.vms.get_mut(&second).unwrap().affinity_known = false;
    });
    batch::open(&mut h.model, &target);
    let reason = "VM details or home-host identity are unavailable";
    assert_eq!(several(&h).choices[1].refusal.as_deref(), Some(reason));
    assert_eq!(
        h.node("batch-move-refusal-1").text.as_deref(),
        Some(reason)
    );
    assert!(h.find("batch-move-refusal-0").is_none());
    let host = *inventory(&h).hosts.keys().last().unwrap();
    h.click(&format!("batch-move-host-{host}"));
    let name = several(&h).choices[1].source.name.clone();
    let refusal = format!("{name} ({second}): {reason}");
    assert_eq!(h.try_click("batch-move-review"), Err(refusal.clone()));
    batch::review(&mut h.model);
    assert_eq!(several(&h).error, Some(refusal));
    assert!(several(&h).review.is_none());
}
