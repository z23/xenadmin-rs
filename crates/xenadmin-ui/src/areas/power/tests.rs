//! The tests of the area, in three parts.
//!
//! 1. **The Rune tests**: the five of `vm_recovery_view_tests.rs`, the
//!    three of `bulk_power_tests.rs`, and this area's share of two tests of
//!    other Rune files (the denied power button of
//!    `session_permissions_view.rs`, the experimental notice of `main.rs`),
//!    under names that say which they were. Each asserts what the Rune test
//!    asserted of the model; what it asserted of the window (a control
//!    inside the frame at two sizes, Shift+Down in the grid, a secondary
//!    click) is listed in `notes/progress/gpui-tests-wanted.md`.
//! 2. **What the port changed**: the buttons became commands, the recovery
//!    overview became commands and record controls, and everything reads
//!    the pool of its target.
//! 3. **The hazards** the maps list for these modules, one test each where
//!    a fixture can reach it.
//!
//! No fixture here has a current snapshot, so nothing in this file submits:
//! an `ObservedVm` cannot be made without one. These tests reach every
//! refusal in front of a submission; `live_tests.rs` opens the confirmation
//! and the placement dialog and submits, against the fixture server.

use xenadmin_core::{
    ActionAvailability, Error, ForceRecoveryAction, OperationAction, OperationId, OperationRecord,
    PermissionDecision, PermissionUnknown, PoolId, PowerAction, PowerState, TaskId, VmId,
    VmRecoveryOperation,
};

use super::commands::POWER;
use super::recovery::{self, Review};
use super::scenes::{amend, make_busy, prepare_recovery};
use super::single::{self, Facts};
use super::{DEMO, GONE, Guest, NOT_CURRENT, READ_ONLY, batch, placement};
use crate::Harness;
use crate::prelude::*;
use crate::surface::{EXPERIMENTAL_NOTICE_KEY, duplicate_keys, flatten_blocks};

// -- Fixtures -------------------------------------------------------------------

/// The scene's guest (halted, HVM, allows Start), selected, nothing open.
fn guest() -> (Harness, Target, VmId) {
    let mut m = Model::fixture();
    let target = prepare_recovery(&mut m).expect("a halted guest");
    let id = target.vms()[0];
    (Harness::with(m), target, id)
}

fn pool(h: &Harness) -> PoolId {
    h.model.inventory.as_ref().expect("inventory").pool_id
}

/// The first `count` guests of the demo pool in `state`, as a target.
fn guests(h: &Harness, state: PowerState, count: usize) -> Target {
    let inventory = h.model.inventory.as_ref().expect("inventory");
    Target::many(
        inventory
            .guests()
            .filter(|vm| vm.power_state == state)
            .take(count)
            .map(|vm| ObjectRef::vm(inventory.pool_id, vm.id)),
    )
}

fn demo() -> Harness {
    let mut m = Model::fixture();
    m.load_demo(3, 48);
    Harness::with(m)
}

#[track_caller]
fn panel(h: &Harness) -> &recovery::Panel {
    h.model.areas.power.recovery.as_deref().expect("a review")
}

#[track_caller]
fn editor(h: &Harness) -> &batch::Editor {
    h.model.areas.power.batch.as_ref().expect("a batch")
}

fn disabled(h: &Harness, command: &str, target: &Target) -> Option<String> {
    match h.model.eligibility(CommandId(leak(command)), target) {
        Eligibility::Disabled(reason) => Some(reason),
        _ => None,
    }
}

fn hidden(h: &Harness, command: &str, target: &Target) -> bool {
    h.model.eligibility(CommandId(leak(command)), target) == Eligibility::Hidden
}

fn enabled(h: &Harness, command: &str, target: &Target) -> bool {
    h.model.eligibility(CommandId(leak(command)), target) == Eligibility::Enabled
}

fn leak(text: &str) -> &'static str {
    Box::leak(text.to_owned().into_boxed_str())
}

/// One of this client's operations on a VM, as a record.
fn record(h: &mut Harness, vm: VmId, state: &str, extra: serde_json::Value) -> OperationId {
    let id = OperationId::new();
    let mut value = serde_json::json!({
        "id": id, "pool": pool(h), "vm": vm, "action": "HardShutdown", "state": state,
        "progress": null, "detail": "Waiting for host task", "created_at_ms": 1,
        "cancel_requested": false, "can_cancel": true, "steps": []
    });
    if let (Some(value), Some(extra)) = (value.as_object_mut(), extra.as_object()) {
        value.extend(extra.clone());
    }
    let record: OperationRecord = serde_json::from_value(value).expect("a record");
    h.model.operation_rows.push(record);
    id
}

fn facts() -> Facts {
    Facts {
        permission: Some(PermissionDecision::Granted),
        synthetic: false,
        read_only: false,
        can_mutate: true,
        allowed: true,
        current: true,
        diagnostic: None,
        availability: Some(ActionAvailability::Available),
    }
}

// -- 1. The Rune tests ----------------------------------------------------------

/// `vm_recovery_view_tests.rs`: compact_and_wide_boot_review_keeps_target_and_visible_controls.
#[test]
fn rune_boot_review_keeps_target_and_refuses_in_demo_and_on_a_changed_connection() {
    let mut h = Harness::scene("vm-recovery");
    let target = panel(&h).vm;
    assert!(matches!(
        panel(&h).review,
        Review::Action(VmRecoveryOperation::Boot { .. })
    ));
    assert!(h.has_text("Temporary order"));
    assert!(h.find("recovery-confirm").is_some());
    // Another row chosen in a grid does not retarget the review.
    let other = guests(&h, PowerState::Running, 1);
    h.model.select_in_grid(other);
    assert_eq!(panel(&h).vm, target);
    assert_eq!(h.try_click("recovery-confirm"), Err(DEMO.into()));
    assert!(h.model.operations.is_empty());
    h.model.demo = false;
    assert!(h.has_text("captured connection changed"));
    assert_eq!(h.try_click("recovery-confirm"), Err(recovery::STALE.into()));
    recovery::apply(&mut h.model);
    assert!(h.model.operations.is_empty());
    assert!(h.model.areas.power.recovery.is_some());
}

/// `vm_recovery_view_tests.rs`: force_review_captures_exact_tasks_and_refuses_changed_tasks_or_admin_blocks.
#[test]
fn rune_force_review_captures_exact_tasks_and_refuses_changed_tasks_or_admin_blocks() {
    let (mut h, target, id) = guest();
    let task = TaskId::from(uuid::Uuid::from_u128(900));
    make_busy(&mut h.model, id, "export", Some(task));
    h.invoke_on("vm.force-shutdown", &target);
    assert!(matches!(
        &panel(&h).review,
        Review::Action(VmRecoveryOperation::Force { action: ForceRecoveryAction::Shutdown, tasks, .. })
            if tasks[0].task == task
    ));
    assert!(h.has_text("No task listed here will be cancelled"));
    // An export is not a migration, so the review does not warn of one.
    assert!(!h.has_text("A migration of this VM is running"));
    assert!(h.find("recovery-accept-migration").is_none());
    amend(&mut h.model, |i| {
        i.details
            .vms
            .get_mut(&id)
            .unwrap()
            .current_operations
            .clear()
    });
    assert!(h.has_text("captured task identities changed"));
    recovery::apply(&mut h.model);
    assert!(h.model.operations.is_empty());
    make_busy(&mut h.model, id, "export", Some(task));
    amend(&mut h.model, |i| {
        i.details
            .vms
            .get_mut(&id)
            .unwrap()
            .blocked_operations
            .insert("hard_shutdown".into(), "Protected guest".into());
    });
    assert!(h.has_text("Administratively blocked: Protected guest"));
    // The captured tasks were never rebuilt from the changed inventory.
    assert!(matches!(
        &panel(&h).review,
        Review::Action(VmRecoveryOperation::Force { tasks, .. }) if tasks.len() == 1 && tasks[0].task == task
    ));
}

/// `vm_recovery_view_tests.rs`: a_forced_action_over_a_migration_warns_and_waits_for_the_operator_to_accept_it.
#[test]
fn rune_a_forced_action_over_a_migration_warns_and_waits_for_the_operator_to_accept_it() {
    for (operation, warns) in [
        ("migrate_send", true),
        ("pool_migrate", true),
        ("snapshot", false),
    ] {
        let (mut h, target, id) = guest();
        let task = TaskId::from(uuid::Uuid::from_u128(901));
        make_busy(&mut h.model, id, operation, Some(task));
        h.invoke_on("vm.force-shutdown", &target);
        // The review still names the task and still promises to cancel nothing.
        assert!(h.has_text(&format!("{operation} · {task}")), "{operation}");
        assert!(h.has_text("No task listed here will be cancelled"));
        assert_eq!(
            h.has_text("A migration of this VM is running"),
            warns,
            "{operation}"
        );
        let unaccepted = |h: &Harness| match &panel(h).review {
            Review::Action(operation) => operation.migration_unaccepted(),
            Review::Cancel { .. } => panic!("no review of an action"),
        };
        if !warns {
            // Nothing to accept: the review has no such choice and is not held back by one.
            assert!(h.find("recovery-accept-migration").is_none(), "{operation}");
            assert!(h.find("recovery-migration-caution").is_none());
            assert!(!unaccepted(&h), "{operation}");
            continue;
        }
        // The forced action is refused until its consequence is accepted: the review
        // says so, starts unaccepted and submits nothing.
        assert!(h.has_text("This action is refused unless you accept"));
        assert!(h.find("recovery-migration-caution").is_some());
        assert!(unaccepted(&h), "{operation}");
        recovery::apply(&mut h.model);
        assert!(h.model.operations.is_empty(), "{operation}");
        assert!(h.model.areas.power.recovery.is_some());
        // Ticking the choice records the acceptance in the reviewed operation, and
        // unticking it withdraws it.
        h.toggle("recovery-accept-migration");
        assert!(!unaccepted(&h), "{operation}");
        assert_eq!(h.node("recovery-accept-migration").checked, Some(true));
        assert!(matches!(
            &panel(&h).review,
            Review::Action(VmRecoveryOperation::Force { interrupts_migration: true, tasks, .. })
                if tasks[0].task == task
        ));
        h.toggle("recovery-accept-migration");
        assert!(unaccepted(&h), "{operation}");
    }
}

/// `vm_recovery_view_tests.rs`: unsupported_guest_and_missing_task_identity_do_not_offer_recovery_dispatch.
/// The two offers were buttons of the overview; they are commands now, and
/// the two texts are the reason of one and a line of the host tasks.
#[test]
fn rune_unsupported_guest_and_missing_task_identity_do_not_offer_recovery_dispatch() {
    let (mut h, target, id) = guest();
    amend(&mut h.model, |i| {
        i.details.vms.get_mut(&id).unwrap().domain_type = Some("pv".into());
    });
    make_busy(&mut h.model, id, "export", None);
    let inventory = h.model.inventory.clone().unwrap();
    assert_eq!(
        recovery::force_operation(&inventory, id, ForceRecoveryAction::Shutdown).err(),
        Some(recovery::FORCE_UNAVAILABLE)
    );
    assert!(disabled(&h, "vm.force-shutdown", &target).is_some());
    assert!(disabled(&h, "vm.force-reboot", &target).is_some());
    let boot = disabled(&h, "vm.recovery-start", &target).expect("no recovery boot");
    assert_eq!(boot, recovery::BOOT_UNAVAILABLE);
    assert!(boot.contains("PV guests"));
    assert!(h.try_invoke_on("vm.recovery-start", &target).is_err());
    assert!(h.try_invoke_on("vm.force-shutdown", &target).is_err());
    assert!(h.model.areas.power.recovery.is_none());
    assert!(
        recovery::host_tasks(&inventory, id)
            .iter()
            .any(|line| line.contains("Task identity unavailable"))
    );
}

/// `vm_recovery_view_tests.rs`: cancellation_review_is_bound_to_owned_operation_and_does_not_force.
#[test]
fn rune_cancellation_review_is_bound_to_owned_operation_and_does_not_force() {
    let (mut h, _, vm) = guest();
    let id = record(&mut h, vm, "Tracking", serde_json::json!({}));
    let row = h.model.operation_rows[0].clone();
    let offered = recovery::cancellation_review_button(&h.model, &row).expect("offered");
    assert_eq!(offered.key, format!("recovery-cancel-{id}"));
    assert_eq!(offered.label, "Review cancellation request…");
    assert_eq!(offered.disabled, None);
    super::review_cancellation(&mut h.model, id);
    assert!(matches!(&panel(&h).review, Review::Cancel { id: captured, .. } if *captured == id));
    assert!(h.has_text("does not cancel another client's tasks"));
    assert_eq!(h.try_click("recovery-confirm"), Err(DEMO.into()));
    recovery::apply(&mut h.model);
    assert!(h.model.operations.is_empty());
    assert!(!h.model.operation_rows[0].cancel_requested);
}

/// `bulk_power_tests.rs`: multi_selection_survives_inventory_refresh_and_reviews_captured_vms_at_both_sizes.
#[test]
fn rune_batch_reviews_the_captured_vms_in_order_and_demo_cannot_run_it() {
    let mut h = demo();
    let target = guests(&h, PowerState::Running, 2);
    assert_eq!(target.len(), 2);
    h.select(Target::one(ObjectRef::pool(pool(&h))));
    h.model.select_in_grid(target.clone());
    h.invoke("vm.power-selection");
    assert!(editor(&h).review.is_none());
    h.click("batch-power-review");
    let captured = editor(&h).review.clone().expect("review");
    assert_eq!(
        captured.vms.iter().map(|v| v.vm).collect::<Vec<_>>(),
        target.vms()
    );
    // A later selection in the grid does not alter the review.
    let other = guests(&h, PowerState::Halted, 1);
    h.model.select_in_grid(other);
    assert_eq!(editor(&h).review.as_ref(), Some(&captured));
    assert_eq!(h.try_click("batch-power-submit"), Err(DEMO.into()));
    batch::submit(&mut h.model);
    assert!(h.model.resource_operations.is_empty());
    assert_eq!(editor(&h).error, Some(Error::Stale.to_string()));
    let id = captured.vms[0].vm;
    amend(&mut h.model, |i| {
        i.vms.get_mut(&id).unwrap().power_state = PowerState::Unknown("changed".into());
    });
    let e = editor(&h);
    let reasons = batch::unavailable(&h.model, e, e.action);
    assert_eq!(reasons.len(), 1);
    assert!(reasons[0].ends_with(batch::CHANGED), "{reasons:?}");
}

/// `bulk_power_tests.rs`: context_menu_targets_clicked_row_and_keeps_non_vm_menus_relevant.
/// The model's half: which menu has the batch item. (That the menu opens on
/// the clicked row is the grid's, a GPUI test.)
#[test]
fn rune_context_menu_of_several_vms_has_the_batch_and_other_kinds_do_not() {
    let h = demo();
    let ids = |target: &Target| -> Vec<&'static str> {
        h.context_menu(target)
            .iter()
            .map(|item| item.id.0)
            .collect()
    };
    let several = ids(&guests(&h, PowerState::Running, 2));
    assert!(several.contains(&"vm.power-selection"), "{several:?}");
    let one = ids(&guests(&h, PowerState::Running, 1));
    assert!(!one.contains(&"vm.power-selection"), "{one:?}");
    let inventory = h.model.inventory.clone().unwrap();
    let host = *inventory.hosts.keys().next().unwrap();
    let server = ids(&Target::one(ObjectRef::host(inventory.pool_id, host)));
    assert!(!server.iter().any(|id| id.starts_with("vm.")), "{server:?}");
}

/// `bulk_power_tests.rs`: batch_uses_only_visible_selection_and_rejects_over_limit_without_silent_subset.
/// Which rows are visible and selected is the grid's now; the target is
/// what it hands over.
#[test]
fn rune_batch_rejects_over_limit_without_silent_subset() {
    let mut h = demo();
    let all = guests(&h, PowerState::Running, usize::MAX);
    assert!(all.len() > 32);
    let reason = disabled(&h, "vm.power-selection", &all).expect("refused");
    assert_eq!(reason, batch::LIMIT);
    assert!(reason.contains("32"));
    assert_eq!(
        disabled(&h, "vm.shutdown", &all).as_deref(),
        Some(batch::LIMIT)
    );
    batch::open(&mut h.model, &all, None);
    assert!(h.model.areas.power.batch.is_none());
    assert!(h.model.error.as_deref().unwrap().contains("32"));
    let within = guests(&h, PowerState::Running, 32);
    assert!(enabled(&h, "vm.power-selection", &within));
    assert_eq!(
        batch::capture(&h.model, &within).ok().unwrap().vms.len(),
        32
    );
}

/// `session_permissions_view.rs`: known_denial_disables_power_without_hiding_its_reason_at_both_widths
/// and unknown_permissions_allow_review_but_grants_never_override_object_ineligibility.
#[test]
fn rune_known_denial_disables_power_with_its_reason_and_unknown_does_not() {
    let denied = Facts {
        permission: Some(PermissionDecision::Denied),
        ..facts()
    };
    assert!(!denied.enabled());
    let reason = PermissionDecision::Denied.explanation().to_owned();
    assert_eq!(denied.refusal(), Some(reason.clone()));
    assert_eq!(denied.caption(), Some(reason));
    let unknown = Facts {
        permission: Some(PermissionDecision::Unknown(PermissionUnknown::Unsupported)),
        ..facts()
    };
    assert!(unknown.enabled());
    assert_eq!(unknown.refusal(), None);
    // A grant never overrides what the object does not allow.
    let ineligible = Facts {
        allowed: false,
        availability: Some(ActionAvailability::Unavailable),
        ..facts()
    };
    assert!(!ineligible.enabled());
    assert_eq!(
        ineligible.refusal(),
        Some(ActionAvailability::Unavailable.explanation())
    );
    // A disabled command is not invoked: no error, no operation, no review.
    let (mut h, target, _) = guest();
    assert_eq!(h.try_invoke_on("vm.start", &target), Err(DEMO.into()));
    assert!(h.model.operations.is_empty());
    assert!(h.model.areas.power.confirm.is_none());
}

/// `main.rs`: dialogs_outside_the_supported_core_carry_the_experimental_notice,
/// for the surfaces of this area.
#[test]
fn rune_experimental_notice_follows_the_rune_status_of_each_surface() {
    let batch = Harness::scene("batch-power");
    assert!(!batch.surface().experimental);
    assert!(!batch.model.experimental_notice());
    for scene in ["vm-recovery", "vm-recovery-force"] {
        let h = Harness::scene(scene);
        assert!(h.surface().experimental, "{scene}");
        assert!(h.model.experimental_notice(), "{scene}");
        assert!(h.find(EXPERIMENTAL_NOTICE_KEY).is_some(), "{scene}");
    }
    // The acknowledgement of an unknown outcome was never in Rune's list.
    let (mut h, _, vm) = guest();
    let id = record(&mut h, vm, "OutcomeUnknown", serde_json::json!({}));
    super::review_unknown(&mut h.model, id);
    assert_eq!(h.surface().id, "unknown-outcome");
    assert!(!h.surface().experimental);
}

// -- 2. What the port changed ---------------------------------------------------

#[test]
fn the_scenes_open_their_surfaces_with_unique_keys() {
    for (scene, surface) in [
        ("batch-power", "power-batch"),
        ("vm-recovery", "vm-recovery"),
        ("vm-recovery-force", "vm-recovery"),
    ] {
        let h = Harness::scene(scene);
        assert_eq!(h.surface_id().as_deref(), Some(surface), "{scene}");
        h.assert_unique_keys();
    }
    let h = Harness::scene("batch-power");
    let e = editor(&h);
    assert_eq!(e.vms.len(), 2);
    assert_eq!(e.vms[0].power_state, e.vms[1].power_state);
    assert!(e.review.is_some());
    assert!(h.has_text(&format!("{} 2 VMs", e.action.label())));
    assert!(h.has_text("2 VMs captured in this pool"));
    let h = Harness::scene("vm-recovery-force");
    assert!(h.has_text("export · 00000000-0000-0000-0000-000000000384"));
}

#[test]
fn every_power_action_is_a_command_of_the_vm_menu_and_seven_are_in_the_toolbar() {
    let registry = crate::command::registry();
    for (action, id) in POWER {
        let spec = registry
            .iter()
            .find(|spec| spec.id == id)
            .unwrap_or_else(|| panic!("{action:?}"));
        assert!(spec.menu.is_some(), "{action:?}");
        assert!(spec.multi, "{action:?}");
        assert_eq!(
            spec.toolbar.is_some(),
            !matches!(action, PowerAction::Pause | PowerAction::Unpause),
            "{action:?}"
        );
    }
    for id in [
        "vm.recovery-start",
        "vm.recovery-restore-boot",
        "vm.start-on-server",
        "vm.resume-on-server",
        "vm.power-selection",
        "vm.check-availability",
    ] {
        assert!(registry.iter().any(|spec| spec.id.0 == id), "{id}");
    }
}

#[test]
fn power_commands_are_hidden_for_what_is_not_an_ordinary_vm() {
    let (mut h, target, id) = guest();
    let inventory = h.model.inventory.clone().unwrap();
    let pool = inventory.pool_id;
    let host = *inventory.hosts.keys().next().unwrap();
    for other in [
        Target::one(ObjectRef::pool(pool)),
        Target::one(ObjectRef::host(pool, host)),
        Target::one(ObjectRef::template(pool, id)),
        Target::none(),
    ] {
        for (_, command) in POWER {
            assert!(hidden(&h, command.0, &other), "{}", command.0);
        }
        assert!(hidden(&h, "vm.recovery-start", &other));
        assert!(hidden(&h, "vm.start-on-server", &other));
        assert!(hidden(&h, "vm.check-availability", &other));
        assert!(hidden(&h, "vm.power-selection", &other));
    }
    // A VM that became a control domain has no power commands; one the
    // inventory lost says so.
    amend(&mut h.model, |i| {
        i.vms.get_mut(&id).unwrap().is_control_domain = true
    });
    assert!(hidden(&h, "vm.start", &target));
    amend(&mut h.model, |i| {
        i.vms.remove(&id);
    });
    assert_eq!(disabled(&h, "vm.start", &target).as_deref(), Some(GONE));
}

#[test]
fn in_the_demo_one_vm_has_no_power_command_enabled_and_the_toolbar_shows_none() {
    let (h, target, _) = guest();
    for (action, command) in POWER {
        assert_eq!(
            disabled(&h, command.0, &target).as_deref(),
            Some(DEMO),
            "{action:?}"
        );
    }
    let toolbar: Vec<_> = h.toolbar().iter().map(|item| item.id.0).collect();
    assert!(
        !toolbar.iter().any(|id| id.starts_with("vm.")),
        "{toolbar:?}"
    );
    // Rune hid "Start on host…" and "Check action availability" in the demo.
    assert!(hidden(&h, "vm.start-on-server", &target));
    assert!(hidden(&h, "vm.resume-on-server", &target));
    assert!(hidden(&h, "vm.check-availability", &target));
    assert!(hidden(&h, "vm.recovery-restore-boot", &target));
}

/// The kernel's scene `unavailable-actions`: an inventory, not the demo, no
/// connection. Rune drew nine disabled buttons, each with what the
/// inventory says of the action on a connection that is not current.
#[test]
fn unavailable_actions_scene_gives_each_power_command_the_inventory_reason() {
    let h = Harness::scene("unavailable-actions");
    let target = h.model.target();
    let Ok(g) = Guest::of(&h.model, &target) else {
        panic!("the running guest");
    };
    let details = &g.inventory.details.vms[&g.id];
    for (action, command) in POWER {
        assert_eq!(
            disabled(&h, command.0, &target),
            Some(details.power_availability(action, false).explanation()),
            "{action:?}"
        );
    }
    // A running VM: no placement. Not the demo: the check is offered, and
    // waits for a current connection.
    assert!(hidden(&h, "vm.start-on-server", &target));
    assert!(hidden(&h, "vm.resume-on-server", &target));
    assert_eq!(
        disabled(&h, "vm.check-availability", &target).as_deref(),
        Some(NOT_CURRENT)
    );
    assert!(
        h.local_row()
            .iter()
            .any(|item| item.id.0 == "vm.check-availability" && item.disabled.is_some())
    );
    // What the details pane said above the buttons is the section's.
    let section = super::availability_section(&h.model, &g.object()).expect("a section");
    let nodes = flatten_blocks("tab", &blocks![section]);
    assert!(duplicate_keys(&nodes).is_empty());
    let keys: Vec<_> = nodes.iter().filter_map(|n| n.key.as_deref()).collect();
    assert!(keys.contains(&"power-actions-stale"), "{keys:?}");
    assert!(!keys.contains(&"power-read-only"));
    for (action, _) in POWER {
        assert!(keys.contains(&format!("power-reason-{action:?}").as_str()));
    }
    // A halted VM there: Start on Server is shown and waits as well.
    let halted = guests(&h, PowerState::Halted, 1);
    assert_eq!(
        disabled(&h, "vm.start-on-server", &halted).as_deref(),
        Some(NOT_CURRENT)
    );
    assert!(hidden(&h, "vm.resume-on-server", &halted));
}

#[test]
fn the_section_is_for_vms_only_and_says_when_one_is_gone() {
    let (mut h, target, id) = guest();
    let object = target.single().unwrap().clone();
    let pool = pool(&h);
    assert!(super::availability_section(&h.model, &ObjectRef::pool(pool)).is_none());
    let section = super::availability_section(&h.model, &object).expect("a section");
    let nodes = flatten_blocks("tab", &blocks![section]);
    // In the demo Rune said nothing above the buttons.
    assert!(!nodes.iter().any(|n| {
        matches!(
            n.key.as_deref(),
            Some("power-actions-stale" | "power-actions-unavailable")
        )
    }));
    amend(&mut h.model, |i| {
        i.vms.remove(&id);
    });
    let section = super::availability_section(&h.model, &object).expect("a section");
    let nodes = flatten_blocks("tab", &blocks![section]);
    assert!(
        nodes
            .iter()
            .any(|n| n.key.as_deref() == Some("power-vm-gone"))
    );
}

#[test]
fn a_force_command_takes_the_task_aware_review_only_when_tasks_are_known() {
    let (mut h, target, id) = guest();
    // Halted, no tasks: the ordinary way, refused in the demo.
    assert_eq!(
        disabled(&h, "vm.force-shutdown", &target).as_deref(),
        Some(DEMO)
    );
    make_busy(
        &mut h.model,
        id,
        "export",
        Some(TaskId::from(uuid::Uuid::from_u128(7))),
    );
    assert!(enabled(&h, "vm.force-shutdown", &target));
    assert!(enabled(&h, "vm.force-reboot", &target));
    // The toolbar shows a power command only while it is enabled.
    let toolbar: Vec<_> = h.toolbar().iter().map(|item| item.id.0).collect();
    assert!(toolbar.contains(&"vm.force-shutdown"), "{toolbar:?}");
    assert!(!toolbar.contains(&"vm.start"), "{toolbar:?}");
    h.invoke_on("vm.force-reboot", &target);
    assert!(matches!(
        &panel(&h).review,
        Review::Action(VmRecoveryOperation::Force {
            action: ForceRecoveryAction::Reboot,
            interrupts_migration: false,
            ..
        })
    ));
    // Opened on its review: nothing to go back to, and no overview.
    let keys = h.keys();
    assert!(keys.contains(&"recovery-close".to_owned()));
    assert!(
        !keys
            .iter()
            .any(|key| key.contains("back") || key == "recovery-refresh")
    );
    assert!(h.model.areas.power.confirm.is_none());
    h.click("recovery-close");
    assert!(h.surfaces().is_empty());
    // An administrative block is the reason in front of the dialog too.
    amend(&mut h.model, |i| {
        i.details
            .vms
            .get_mut(&id)
            .unwrap()
            .blocked_operations
            .insert("hard_shutdown".into(), "Protected guest".into());
    });
    assert_eq!(
        disabled(&h, "vm.force-shutdown", &target).as_deref(),
        Some("Administratively blocked: Protected guest")
    );
    assert!(enabled(&h, "vm.force-reboot", &target));
}

#[test]
fn recovery_start_opens_on_its_review_with_the_captured_host_tasks() {
    let (mut h, target, id) = guest();
    assert!(enabled(&h, "vm.recovery-start", &target));
    h.invoke_on("vm.recovery-start", &target);
    assert_eq!(h.surface().id, "vm-recovery");
    assert_eq!(h.surface().title, "VM recovery controls");
    assert!(h.has_text("Boot recovery media"));
    assert!(h.has_text("No current host tasks were reported."));
    assert!(h.find("recovery-host-tasks").is_some());
    assert_eq!(panel(&h).vm, id);
    h.dismiss();
    assert!(h.model.areas.power.recovery.is_none());
    // A running VM has no recovery boot.
    amend(&mut h.model, |i| {
        i.vms.get_mut(&id).unwrap().power_state = PowerState::Running
    });
    assert_eq!(
        disabled(&h, "vm.recovery-start", &target).as_deref(),
        Some(recovery::BOOT_UNAVAILABLE)
    );
}

#[test]
fn several_vms_open_the_batch_on_the_review_of_the_command_s_action() {
    let mut h = demo();
    let halted = guests(&h, PowerState::Halted, 3);
    assert!(enabled(&h, "vm.start", &halted));
    let refused = disabled(&h, "vm.shutdown", &halted).expect("halted VMs cannot shut down");
    assert!(
        refused.starts_with("Unavailable for 3 VM(s): "),
        "{refused}"
    );
    h.invoke_on("vm.start", &halted);
    let e = editor(&h);
    assert_eq!(e.action, PowerAction::Start);
    let review = e.review.as_ref().expect("opened on the review");
    assert_eq!(review.action, PowerAction::Start);
    assert!(
        review
            .vms
            .iter()
            .all(|vm| vm.expected_state == PowerState::Halted)
    );
    // Back leads to the choice, where an unavailable action cannot be picked.
    h.click("batch-back");
    assert!(editor(&h).review.is_none());
    let shut_down = h.node("batch-action-Shut down");
    assert!(
        shut_down
            .disabled
            .as_deref()
            .is_some_and(|r| r.starts_with("Unavailable for 3 VM(s): "))
    );
    assert_eq!(h.node("batch-action-Start").disabled, None);
    h.click("batch-close");
    assert!(h.surfaces().is_empty());
    // The toolbar offers a multi-selection what it can run.
    h.select(Target::one(ObjectRef::pool(pool(&h))));
    h.model.select_in_grid(halted);
    let toolbar: Vec<_> = h.toolbar().iter().map(|item| item.id.0).collect();
    assert!(toolbar.contains(&"vm.start"), "{toolbar:?}");
    assert!(!toolbar.contains(&"vm.shutdown"), "{toolbar:?}");
}

#[test]
fn a_batch_is_one_pool_and_only_vms() {
    let mut h = demo();
    let mut second = crate::demo::demonstration(1, 6);
    crate::demo_details::populate(&mut second);
    second.pool_id = PoolId::from(uuid::Uuid::from_u128(0xB0));
    let other_pool = second.pool_id;
    let other_vm = second.guests().next().unwrap().id;
    h.model.add_synthetic_pool("Second pool", second);
    let here = guests(&h, PowerState::Running, 1);
    let across = Target::many(
        here.objects
            .iter()
            .cloned()
            .chain([ObjectRef::vm(other_pool, other_vm)]),
    );
    assert_eq!(
        disabled(&h, "vm.power-selection", &across).as_deref(),
        Some(batch::ONE_POOL)
    );
    let host = *h
        .model
        .inventory
        .as_ref()
        .unwrap()
        .hosts
        .keys()
        .next()
        .unwrap();
    let mixed = Target::many(
        here.objects
            .iter()
            .cloned()
            .chain([ObjectRef::host(pool(&h), host)]),
    );
    assert!(hidden(&h, "vm.power-selection", &mixed));
    assert!(hidden(&h, "vm.start", &mixed));
}

#[test]
fn a_command_reads_the_pool_of_its_target_not_the_scope_session() {
    let mut h = demo();
    let mut second = crate::demo::demonstration(1, 6);
    crate::demo_details::populate(&mut second);
    second.pool_id = PoolId::from(uuid::Uuid::from_u128(0xB1));
    let other_pool = second.pool_id;
    let halted = second
        .guests()
        .find(|vm| vm.power_state == PowerState::Halted)
        .expect("a halted guest")
        .id;
    second.details.vms.get_mut(&halted).unwrap().domain_type = Some("hvm".into());
    second.vms.get_mut(&halted).unwrap().allowed_actions = vec![PowerAction::Start];
    h.model.add_synthetic_pool("Second pool", second);
    let target = Target::one(ObjectRef::vm(other_pool, halted));
    // The scope session is still the demo pool.
    assert_ne!(pool(&h), other_pool);
    assert_eq!(disabled(&h, "vm.start", &target).as_deref(), Some(DEMO));
    assert!(enabled(&h, "vm.recovery-start", &target));
    h.invoke_on("vm.recovery-start", &target);
    assert_eq!(panel(&h).pool, other_pool);
    assert_eq!(panel(&h).initial.pool_id, other_pool);
    assert_eq!(h.try_click("recovery-confirm"), Err(DEMO.into()));
    // The checks of the host are the scope session's: not offered there.
    h.model.demo = false;
    assert!(hidden(&h, "vm.check-availability", &target));
}

#[test]
fn surfaces_close_when_the_scope_or_the_session_changes() {
    let mut h = Harness::scene("vm-recovery");
    let other = guests(&h, PowerState::Running, 1);
    h.select(other);
    assert!(h.surfaces().is_empty());
    let mut h = Harness::scene("batch-power");
    crate::session::clear_view(&mut h.model);
    assert!(h.model.areas.power.batch.is_none());
}

#[test]
fn placement_needs_a_current_snapshot_and_never_opens_without_one() {
    let (mut h, target, _) = guest();
    let object = target.single().unwrap().clone();
    assert!(matches!(
        placement::capture(&h.model, &object),
        Err(Error::Stale)
    ));
    placement::open(&mut h.model, &object);
    assert!(h.model.areas.power.placement.is_none());
    assert_eq!(h.model.error, Some(Error::Stale.to_string()));
    h.model.demo = false;
    assert_eq!(
        disabled(&h, "vm.start-on-server", &target).as_deref(),
        Some(NOT_CURRENT)
    );
    assert!(h.try_invoke_on("vm.start-on-server", &target).is_err());
    assert!(h.model.areas.power.placement.is_none());
}

#[test]
fn the_controls_of_a_record_follow_the_rune_conditions() {
    let (mut h, _, vm) = guest();
    let keys = |h: &Harness, index: usize| -> Vec<(String, String)> {
        super::operation_controls(&h.model, &h.model.operation_rows[index])
            .into_iter()
            .map(|b| (b.key, b.label))
            .collect()
    };
    let running = record(&mut h, vm, "Tracking", serde_json::json!({}));
    assert_eq!(
        keys(&h, 0),
        [(
            format!("operation-cancel-{running}"),
            "Request cancellation".to_owned()
        )]
    );
    // A recovery workflow stops its remaining steps instead.
    let inventory = h.model.inventory.clone().unwrap();
    let boot = recovery::boot_operation(&inventory, vm).expect("a recovery boot");
    let workflow = record(
        &mut h,
        vm,
        "Tracking",
        serde_json::json!({ "action": OperationAction::Recovery(boot) }),
    );
    assert_eq!(
        keys(&h, 1),
        [(
            format!("operation-cancel-{workflow}"),
            "Stop remaining steps".to_owned()
        )]
    );
    // Asked already, finished, or a child of a batch: nothing to press.
    record(
        &mut h,
        vm,
        "Tracking",
        serde_json::json!({ "cancel_requested": true }),
    );
    record(&mut h, vm, "Succeeded", serde_json::json!({}));
    record(
        &mut h,
        vm,
        "OutcomeUnknown",
        serde_json::json!({ "batch_parent": OperationId::new() }),
    );
    for index in 2..5 {
        assert!(keys(&h, index).is_empty(), "{index}");
    }
    // An unknown outcome is reviewed, never acknowledged in one press.
    let unknown = record(
        &mut h,
        vm,
        "OutcomeUnknown",
        serde_json::json!({ "can_cancel": false }),
    );
    let controls = keys(&h, 5);
    assert_eq!(
        controls.last(),
        Some(&(
            format!("operation-unknown-{unknown}"),
            "Review unknown outcome".to_owned()
        ))
    );
    assert!(
        controls
            .iter()
            .all(|(key, _)| !key.starts_with("operation-cancel"))
    );
}

#[test]
fn acknowledging_an_unknown_outcome_is_two_steps_and_marks_nothing_locally() {
    let (mut h, _, vm) = guest();
    let id = record(&mut h, vm, "OutcomeUnknown", serde_json::json!({}));
    let name = h.model.inventory.as_ref().unwrap().vms[&vm].name.clone();
    super::review_unknown(&mut h.model, id);
    // The Rune dialog, by its key and its texts; it is the one dialog the
    // Events tab opens too (the `events` area draws it).
    assert_eq!(h.surface().id, "unknown-outcome");
    assert_eq!(h.surface().title, "Acknowledge unknown outcome");
    assert!(h.has_text("Acknowledge unknown outcome?"));
    assert_eq!(
        h.node("unknown-target").text,
        Some(format!("VM: {name}"))
    );
    assert!(h.has_text("it does not cancel the host task or establish its result"));
    assert_eq!(h.node("unknown-acknowledge").tone, Some(Tone::Warning));
    assert_eq!(
        h.node("unknown-acknowledge").label.as_deref(),
        Some("Acknowledge outcome")
    );
    h.click("unknown-cancel");
    assert!(h.surfaces().is_empty());
    assert_eq!(h.model.error, None);
    assert!(h.model.areas.events.acknowledge.is_none());
    super::review_unknown(&mut h.model, id);
    // The demo cannot acknowledge: the button says so.
    assert_eq!(h.try_click("unknown-acknowledge"), Err(super::DEMO.into()));
    // Outside the demo and without a connection the request is refused.
    // The review stays open with the refusal (Rune closed it and left the
    // refusal on the error line), and the record is as it was.
    h.model.demo = false;
    h.click("unknown-acknowledge");
    assert_eq!(h.surface().id, "unknown-outcome");
    assert_eq!(
        h.node("unknown-error").text,
        Some(Error::Closed.to_string())
    );
    assert_eq!(
        format!("{:?}", h.model.operation_rows[0].state),
        "OutcomeUnknown"
    );
    // Once the outcome is known the open review cannot be confirmed.
    let known: OperationRecord = {
        let mut value = serde_json::to_value(&h.model.operation_rows[0]).expect("a record");
        value["state"] = "Succeeded".into();
        serde_json::from_value(value).expect("a record")
    };
    let unknown = std::mem::replace(&mut h.model.operation_rows[0], known);
    assert!(h.try_click("unknown-acknowledge").is_err());
    assert!(h.find("unknown-stale").is_some());
    h.dismiss();
    assert!(h.surfaces().is_empty());
    // A record whose outcome is known opens no review at all.
    super::review_unknown(&mut h.model, id);
    assert!(h.surfaces().is_empty());
    h.model.operation_rows[0] = unknown;
    // A VM the inventory lost is named by its UUID.
    amend(&mut h.model, |i| {
        i.vms.remove(&vm);
    });
    super::review_unknown(&mut h.model, id);
    assert_eq!(h.node("unknown-target").text, Some(format!("VM: {vm}")));
}

// -- 3. Hazards -----------------------------------------------------------------

/// Reason precedence under a power button: a known denial, then the host
/// check, then the inventory; the demo and a read-only connection have
/// their own caption; a host check never enables.
#[test]
fn the_reason_of_a_disabled_power_command_has_the_rune_precedence() {
    let checked = |answer| Facts {
        diagnostic: Some(answer),
        allowed: false,
        availability: Some(ActionAvailability::Unavailable),
        ..facts()
    };
    assert_eq!(
        checked(Some(Ok(ActionAvailability::Available)))
            .refusal()
            .as_deref(),
        Some(single::ADVISORY)
    );
    assert!(!checked(Some(Ok(ActionAvailability::Available))).enabled());
    assert_eq!(
        checked(Some(Ok(ActionAvailability::Unsupported))).refusal(),
        Some(ActionAvailability::Unsupported.explanation())
    );
    assert_eq!(
        checked(Some(Err("HOST_OFFLINE".into())))
            .refusal()
            .as_deref(),
        Some("HOST_OFFLINE")
    );
    assert_eq!(checked(None).refusal().as_deref(), Some(single::CHECKING));
    let denied = Facts {
        permission: Some(PermissionDecision::Denied),
        ..checked(None)
    };
    assert_eq!(
        denied.refusal().as_deref(),
        Some(PermissionDecision::Denied.explanation())
    );
    let demo = Facts {
        synthetic: true,
        can_mutate: false,
        ..facts()
    };
    assert_eq!(demo.refusal().as_deref(), Some(DEMO));
    assert_eq!(demo.caption(), None);
    let read_only = Facts {
        read_only: true,
        can_mutate: false,
        ..facts()
    };
    assert_eq!(read_only.refusal().as_deref(), Some(READ_ONLY));
    let stale = Facts {
        current: false,
        can_mutate: false,
        availability: Some(ActionAvailability::Stale),
        ..facts()
    };
    assert_eq!(
        stale.refusal(),
        Some(ActionAvailability::Stale.explanation())
    );
    let no_details = Facts {
        allowed: false,
        availability: None,
        ..facts()
    };
    assert_eq!(no_details.refusal().as_deref(), Some(single::UNKNOWN));
    let no_details_stale = Facts {
        current: false,
        can_mutate: false,
        availability: None,
        ..facts()
    };
    assert_eq!(no_details_stale.refusal().as_deref(), Some(NOT_CURRENT));
    // An enabled action with a host answer still shows the answer.
    let answered = Facts {
        diagnostic: Some(Some(Ok(ActionAvailability::Unsupported))),
        ..facts()
    };
    assert!(answered.enabled());
    assert_eq!(answered.refusal(), None);
    assert_eq!(
        answered.caption(),
        Some(ActionAvailability::Unsupported.explanation())
    );
}

/// The demo never reaches the core: a request made past the command (which
/// is disabled there) is refused as stale, with no review and no operation.
#[test]
fn a_request_in_the_demo_is_stale_and_opens_nothing() {
    let (mut h, target, _) = guest();
    let object = target.single().unwrap().clone();
    for action in [PowerAction::Start, PowerAction::HardShutdown] {
        single::request(&mut h.model, &object, action);
        assert_eq!(h.model.error, Some(Error::Stale.to_string()));
        assert!(h.model.areas.power.confirm.is_none());
        assert!(h.model.operations.is_empty());
    }
    // The single choke point of a submission says the same.
    assert!(matches!(h.model.admit_command(), Err(Error::Stale)));
}

/// Force actions are always reviewed, whatever the option says; routine
/// ones only when it is on.
#[test]
fn force_actions_are_always_reviewed() {
    let mut h = demo();
    for review_power in [false, true] {
        h.model.preferences.options.review_power = review_power;
        let options = h.model.preferences.options;
        assert!(options.reviews(PowerAction::HardShutdown));
        assert!(options.reviews(PowerAction::HardReboot));
        assert_eq!(options.reviews(PowerAction::Start), review_power);
    }
}

/// The batch: one unavailable VM blocks the action, a review resets the
/// force acceptance, and choosing another action clears the error.
#[test]
fn batch_review_blocks_on_one_vm_and_resets_the_force_acceptance() {
    let mut h = demo();
    let running = guests(&h, PowerState::Running, 2);
    // The demo guests do not list the force actions: allow one.
    let ids = running.vms();
    amend(&mut h.model, |i| {
        for id in &ids {
            let details = i.details.vms.get_mut(id).unwrap();
            details
                .allowed_operations
                .get_or_insert_with(Vec::new)
                .push("hard_shutdown".into());
        }
    });
    h.invoke_on("vm.power-selection", &running);
    assert!(h.node("batch-action-Force reboot").disabled.is_some());
    h.pick("batch-action", "Force shut down");
    h.click("batch-power-review");
    assert!(h.has_text(batch::FORCE_RISK));
    assert_eq!(h.node("batch-force-ack").checked, Some(false));
    h.toggle("batch-force-ack");
    assert!(editor(&h).force_ack);
    h.click("batch-back");
    h.click("batch-power-review");
    assert!(!editor(&h).force_ack);
    // One VM changes: the action is unavailable for the batch, none is dropped.
    let id = editor(&h).vms[1].id;
    amend(&mut h.model, |i| {
        i.vms.get_mut(&id).unwrap().power_state = PowerState::Halted
    });
    h.click("batch-back");
    let refused = h
        .try_click("batch-power-review")
        .expect_err("one VM changed");
    assert!(
        refused.starts_with("Unavailable for 1 VM(s): "),
        "{refused}"
    );
    assert!(refused.ends_with(batch::CHANGED), "{refused}");
    batch::review(&mut h.model);
    assert!(editor(&h).review.is_none());
    assert!(
        editor(&h)
            .error
            .as_deref()
            .unwrap()
            .ends_with(batch::CHANGED)
    );
    assert_eq!(editor(&h).vms.len(), 2);
    batch::choose(&mut h.model, PowerAction::Reboot);
    assert_eq!(editor(&h).error, None);
    // Outside the demo, without the observed pool, the dialog says it is stale.
    h.model.demo = false;
    assert!(h.has_text(batch::STALE));
    assert!(!batch::current(&h.model, editor(&h)));
}

/// A cancellation review is bound to the record's revision and state: a
/// record that was asked to cancel in the meantime is no longer the one.
#[test]
fn a_cancellation_review_refuses_a_record_that_moved_on() {
    let (mut h, _, vm) = guest();
    let id = record(&mut h, vm, "Tracking", serde_json::json!({}));
    super::review_cancellation(&mut h.model, id);
    let Review::Cancel { revision, .. } = panel(&h).review.clone() else {
        panic!("a cancellation review");
    };
    assert!(recovery::cancellable(&h.model, panel(&h), id, revision));
    assert!(!recovery::cancellable(
        &h.model,
        panel(&h),
        id,
        revision + 1
    ));
    h.model.operation_rows[0].cancel_requested = true;
    assert!(!recovery::cancellable(&h.model, panel(&h), id, revision));
    // Outside the demo the review says so.
    h.model.demo = false;
    assert!(h.has_text("captured connection changed"));
    // Nothing is offered for a record that cannot be cancelled.
    let done = record(&mut h, vm, "Succeeded", serde_json::json!({}));
    let row = h.model.operation_rows[1].clone();
    assert!(recovery::cancellation_review_button(&h.model, &row).is_none());
    h.model.areas.power.recovery = None;
    super::review_cancellation(&mut h.model, done);
    assert!(h.model.areas.power.recovery.is_none());
}

/// Texts that name Rune places are carried over word for word and renamed
/// in one sweep at the end (decision 10): pinned here so the sweep finds
/// them. "Go Back" and "Refresh" name controls the review no longer has.
#[test]
fn texts_awaiting_the_final_rename_sweep_are_pinned() {
    assert!(recovery::RESERVATIONS.contains("Inspect unknown outcomes in Activity"));
    assert!(recovery::CANCEL_CHANGED.contains("Go Back and review its current state"));
    assert!(recovery::STALE.contains("Close this panel and reopen it from current inventory"));
    assert!(recovery::TASKS_NOTE.contains("Refresh before reviewing a changed VM"));
    assert!(recovery::CANCEL.contains("then refresh and review any force action separately"));
    assert!(single::NO_ACTIONS.contains("Check action availability below"));
    assert!(batch::LIMIT.contains("visible VMs"));
}
