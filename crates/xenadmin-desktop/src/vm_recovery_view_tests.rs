use super::*;
fn runtime(mut s: State, size: (f32, f32)) -> rune::Runtime<State> {
    s.window_width = size.0;
    s.window_height = size.1;
    let mut r = rune::Runtime::new(
        s,
        crate::view,
        size,
        rune::Env {
            system_dark: true,
            scale: 1.0,
        },
    );
    settle(&mut r);
    r
}
fn settle(r: &mut rune::Runtime<State>) {
    for _ in 0..30 {
        r.frame(1.0 / 60.0);
    }
}
fn click(r: &mut rune::Runtime<State>, key: &str) {
    r.update(|_, tx| tx.scroll_into_view(key));
    settle(r);
    let (x, y) = r.find(key).unwrap_or_else(|| panic!("missing {key}"));
    r.pointer_down(x, y);
    r.pointer_up(x, y);
    settle(r);
}
#[test]
fn compact_and_wide_boot_review_keeps_target_and_visible_controls() {
    for size in [(1440.0, 900.0), (513.0, 652.0)] {
        let mut s = init();
        scene(&mut s);
        let target = s.vm_recovery.as_ref().unwrap().vm;
        let mut r = runtime(s, size);
        assert!(
            r.accessibility_json()
                .to_string()
                .contains("Temporary order")
        );
        let (x, y) = r.find("recovery-confirm").unwrap();
        assert!(x >= 0.0 && x < size.0 && y >= 0.0 && y < size.1, "{x},{y}");
        r.update(|s, _| s.table.select("30000000-0000-0000-0000-000000000000"));
        settle(&mut r);
        assert_eq!(r.state.vm_recovery.as_ref().unwrap().vm, target);
        click(&mut r, "recovery-confirm");
        assert!(r.state.operations.is_empty());
        r.update(|s, _| s.demo = false);
        settle(&mut r);
        assert!(
            r.accessibility_json()
                .to_string()
                .contains("captured connection changed")
        );
        apply(&mut r.state);
        assert!(r.state.operations.is_empty());
    }
}
#[test]
fn force_review_captures_exact_tasks_and_refuses_changed_tasks_or_admin_blocks() {
    let mut s = init();
    scene(&mut s);
    let id = s.vm_recovery.as_ref().unwrap().vm;
    let task = xenadmin_core::TaskId::from(uuid::Uuid::from_u128(900));
    {
        let i = Arc::make_mut(s.inventory.as_mut().unwrap());
        i.vms.get_mut(&id).unwrap().power_state = PowerState::Running;
        i.vms.get_mut(&id).unwrap().allowed_actions.clear();
        i.details.vms.get_mut(&id).unwrap().current_operations =
            vec![xenadmin_core::RunningOperation {
                task: Some(task),
                operation: "export".into(),
            }];
    }
    open(&mut s, id);
    let mut r = runtime(s, (513.0, 652.0));
    click(&mut r, "recovery-force-shutdown");
    assert!(
        matches!(&r.state.vm_recovery.as_ref().unwrap().review,Some(Review::Action(VmRecoveryOperation::Force{tasks,..})) if tasks[0].task==task)
    );
    assert!(
        r.accessibility_json()
            .to_string()
            .contains("No task listed here will be cancelled")
    );
    // An export is not a migration, so the review does not warn of one.
    assert!(
        !r.accessibility_json()
            .to_string()
            .contains("A migration of this VM is running")
    );
    r.update(|s, _| {
        Arc::make_mut(s.inventory.as_mut().unwrap())
            .details
            .vms
            .get_mut(&id)
            .unwrap()
            .current_operations
            .clear()
    });
    settle(&mut r);
    assert!(
        r.accessibility_json()
            .to_string()
            .contains("captured task identities changed")
    );
    apply(&mut r.state);
    assert!(r.state.operations.is_empty());
    r.update(|s, _| {
        let d = Arc::make_mut(s.inventory.as_mut().unwrap())
            .details
            .vms
            .get_mut(&id)
            .unwrap();
        d.current_operations = vec![xenadmin_core::RunningOperation {
            task: Some(task),
            operation: "export".into(),
        }];
        d.blocked_operations
            .insert("hard_shutdown".into(), "Protected guest".into());
    });
    settle(&mut r);
    assert!(
        r.accessibility_json()
            .to_string()
            .contains("Administratively blocked: Protected guest")
    );
}
#[test]
fn force_review_warns_when_the_task_it_would_interrupt_is_a_migration() {
    for (operation, warns) in [
        ("migrate_send", true),
        ("pool_migrate", true),
        ("snapshot", false),
    ] {
        let mut s = init();
        scene(&mut s);
        let id = s.vm_recovery.as_ref().unwrap().vm;
        let task = xenadmin_core::TaskId::from(uuid::Uuid::from_u128(901));
        {
            let i = Arc::make_mut(s.inventory.as_mut().unwrap());
            i.vms.get_mut(&id).unwrap().power_state = PowerState::Running;
            i.details.vms.get_mut(&id).unwrap().current_operations =
                vec![xenadmin_core::RunningOperation {
                    task: Some(task),
                    operation: operation.into(),
                }];
        }
        open(&mut s, id);
        let mut r = runtime(s, (513.0, 652.0));
        click(&mut r, "recovery-force-shutdown");
        let tree = r.accessibility_json().to_string();
        // The review still names the task and still promises to cancel nothing.
        assert!(
            tree.contains(&format!("{operation} · {task}")),
            "{operation}"
        );
        assert!(tree.contains("No task listed here will be cancelled"));
        assert_eq!(
            tree.contains("A migration of this VM is running"),
            warns,
            "{operation}"
        );
    }
}
#[test]
fn unsupported_guest_and_missing_task_identity_do_not_offer_recovery_dispatch() {
    let mut s = init();
    scene(&mut s);
    let id = s.vm_recovery.as_ref().unwrap().vm;
    {
        let i = Arc::make_mut(s.inventory.as_mut().unwrap());
        let d = i.details.vms.get_mut(&id).unwrap();
        d.domain_type = Some("pv".into());
        d.current_operations = vec![xenadmin_core::RunningOperation {
            task: None,
            operation: "export".into(),
        }];
        i.vms.get_mut(&id).unwrap().power_state = PowerState::Running;
    }
    open(&mut s, id);
    let r = runtime(s, (513.0, 652.0));
    assert!(r.find("recovery-force-shutdown").is_none());
    assert!(r.find("recovery-boot").is_none());
    assert!(
        r.accessibility_json()
            .to_string()
            .contains("Task identity unavailable")
    );
    assert!(r.accessibility_json().to_string().contains("PV guests"));
}
#[test]
fn cancellation_review_is_bound_to_owned_operation_and_does_not_force() {
    let mut s = init();
    scene(&mut s);
    let p = s.vm_recovery.as_ref().unwrap();
    let id = OperationId::new();
    let record: xenadmin_core::OperationRecord=serde_json::from_value(serde_json::json!({"id":id,"pool":p.pool,"vm":p.vm,"action":"HardShutdown","state":"Tracking","progress":null,"detail":"Waiting for host task","created_at_ms":1,"cancel_requested":false,"can_cancel":true,"steps":[]})).unwrap();
    s.operation_rows.push(record);
    s.vm_recovery.as_mut().unwrap().review = None;
    let mut r = runtime(s, (513.0, 652.0));
    click(&mut r, &format!("recovery-cancel-{id}"));
    assert!(
        matches!(&r.state.vm_recovery.as_ref().unwrap().review,Some(Review::Cancel{id:captured,..})if *captured==id)
    );
    assert!(
        r.accessibility_json()
            .to_string()
            .contains("does not cancel another client's tasks")
    );
    click(&mut r, "recovery-confirm");
    assert!(r.state.operations.is_empty());
    assert!(!r.state.operation_rows[0].cancel_requested);
}
