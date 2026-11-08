use super::*;
fn at(generation: u64, host: u128, started: Option<&str>) -> Binding {
    Binding {
        generation,
        host: HostId::from(uuid::Uuid::from_u128(host)),
        started: started.map(str::to_owned),
    }
}
fn opened() -> Recovery {
    Recovery {
        binding: Some(at(1, 1, Some("boot-1"))),
        ..Default::default()
    }
}
#[test]
fn external_move_reboot_and_generation_change_close_then_reopen_once() {
    for next in [
        at(1, 2, Some("boot-1")),
        at(1, 1, Some("boot-2")),
        at(2, 1, Some("boot-1")),
    ] {
        let mut recovery = opened();
        let d = recovery.advance(Ok(next.clone()), &ConsoleState::Connected, 0.0);
        assert!(d.close);
        assert!(!d.reopen);
        assert_eq!(d.wake, Some(1.0));
        let d = recovery.advance(Ok(next.clone()), &ConsoleState::Closed, 0.5);
        assert!(!d.close && !d.reopen);
        let d = recovery.advance(Ok(next.clone()), &ConsoleState::Closed, 1.0);
        assert!(d.reopen);
        assert!(
            !recovery
                .advance(Ok(next.clone()), &ConsoleState::Connecting, 2.0)
                .reopen
        );
        assert!(
            !recovery
                .advance(Ok(next), &ConsoleState::Connected, 3.0)
                .reopen
        );
    }
}
#[test]
fn retry_budget_is_bounded_and_brief_connections_do_not_reset_it() {
    let mut recovery = opened();
    let b = recovery.binding.clone().unwrap();
    let mut clock = 0.0;
    for attempt in 1..=5 {
        let d = recovery.advance(
            Ok(b.clone()),
            &ConsoleState::Failed("unavailable".into()),
            clock,
        );
        assert!(!d.reopen);
        assert_eq!(d.wake, Some(f64::from(1u32 << (attempt - 1))));
        clock += d.wake.unwrap();
        assert!(
            recovery
                .advance(Ok(b.clone()), &ConsoleState::Closed, clock)
                .reopen
        );
        assert_eq!(recovery.attempts, attempt);
        clock += 0.25;
        recovery.advance(Ok(b.clone()), &ConsoleState::Connected, clock);
        assert!(!recovery.exhausted);
        clock += 0.25;
    }
    let d = recovery.advance(Ok(b.clone()), &ConsoleState::Closed, clock);
    assert!(!d.reopen && d.wake.is_none());
    assert!(recovery.exhausted);
    recovery.advance(Ok(b.clone()), &ConsoleState::Connected, clock + 1.0);
    recovery.advance(Ok(b.clone()), &ConsoleState::Connected, clock + 16.0);
    assert_eq!(
        recovery.attempts, 5,
        "a late frame on a closed stream cannot reopen it"
    );
    recovery = opened();
    recovery.attempts = 3;
    recovery.advance(Ok(b.clone()), &ConsoleState::Connected, clock + 1.0);
    recovery.advance(Ok(b.clone()), &ConsoleState::Connected, clock + 16.0);
    assert_eq!(recovery.attempts, 0);
    assert!(
        recovery
            .advance(Ok(b), &ConsoleState::Closed, clock + 17.0)
            .wake
            .is_some()
    );
}
#[test]
fn stale_missing_halted_and_read_only_targets_cannot_reopen_or_reset_failures() {
    for reason in [
        Waiting::Connection,
        Waiting::Guest,
        Waiting::Power,
        Waiting::ReadOnly,
    ] {
        let mut recovery = opened();
        recovery.attempts = 3;
        let d = recovery.advance(Err(reason), &ConsoleState::Connected, 1.0);
        assert!(d.close && !d.reopen && d.wake.is_none());
        let d = recovery.advance(Err(reason), &ConsoleState::Closed, 100.0);
        assert!(!d.close && !d.reopen && d.wake.is_none());
        assert_eq!(recovery.attempts, 3);
        let d = recovery.advance(Ok(at(2, 1, Some("boot-2"))), &ConsoleState::Closed, 101.0);
        assert!(!d.reopen);
        assert!(
            recovery
                .advance(Ok(at(2, 1, Some("boot-2"))), &ConsoleState::Closed, 102.0)
                .reopen
        );
    }
}
#[test]
fn optional_boot_metadata_never_causes_a_spurious_reopen_and_opt_out_is_honored() {
    let mut recovery = opened();
    assert!(
        !recovery
            .advance(Ok(at(1, 1, None)), &ConsoleState::Connected, 1.0)
            .close
    );
    assert_eq!(
        recovery.binding.as_ref().unwrap().started.as_deref(),
        Some("boot-1")
    );
    assert!(
        !recovery
            .advance(Ok(at(1, 1, Some("boot-1"))), &ConsoleState::Connected, 2.0)
            .close
    );
    recovery.enabled = false;
    let d = recovery.advance(Ok(at(1, 2, Some("boot-1"))), &ConsoleState::Connected, 3.0);
    assert!(d.close && !d.reopen && d.wake.is_none());
    assert!(
        !recovery
            .advance(Ok(at(1, 2, Some("boot-1"))), &ConsoleState::Closed, 100.0)
            .reopen
    );
    recovery.enabled = true;
    let d = recovery.advance(Ok(at(1, 2, Some("boot-1"))), &ConsoleState::Closed, 101.0);
    assert_eq!(d.wake, Some(1.0));
}

#[test]
fn synchronous_admission_failure_consumes_attempts_and_releases_old_input() {
    let mut recovery = opened();
    let binding = recovery.binding.clone().unwrap();
    let mut clock = 0.0;
    for attempt in 1..=5 {
        let d = recovery.advance(Ok(binding.clone()), &ConsoleState::Closed, clock);
        assert!(d.close && !d.reopen);
        clock += d.wake.unwrap();
        assert!(
            recovery
                .advance(Ok(binding.clone()), &ConsoleState::Closed, clock)
                .reopen
        );
        assert_eq!(recovery.attempts, attempt);
        clock += 1.0;
    }
    let d = recovery.advance(Ok(binding), &ConsoleState::Closed, clock);
    assert!(d.close && !d.reopen && d.wake.is_none());
    assert!(recovery.exhausted);
}

#[test]
fn inventory_observation_uses_pinned_pool_and_vm_and_rejects_missing_or_wrong_kind() {
    let mut s = crate::init();
    crate::demo(&mut s);
    let mut inventory = s.inventory.as_ref().unwrap().as_ref().clone();
    let vm = inventory
        .vms
        .values()
        .find(|v| v.power_state == PowerState::Running)
        .unwrap()
        .clone();
    let target = ConsoleTarget {
        pool: inventory.pool_id,
        vm: vm.id,
        name: vm.name.clone(),
    };
    s.table.select("different selection");
    assert_eq!(
        inventory_binding(&inventory, &target, 7).unwrap().host,
        vm.host.unwrap()
    );
    inventory.pool_id = PoolId::from(uuid::Uuid::new_v4());
    assert_eq!(
        inventory_binding(&inventory, &target, 7),
        Err(Waiting::Connection)
    );
    inventory.pool_id = target.pool;
    inventory.vms.get_mut(&vm.id).unwrap().is_template = true;
    assert_eq!(
        inventory_binding(&inventory, &target, 7),
        Err(Waiting::Guest)
    );
    inventory.vms.get_mut(&vm.id).unwrap().is_template = false;
    inventory.vms.get_mut(&vm.id).unwrap().is_control_domain = true;
    assert_eq!(
        inventory_binding(&inventory, &target, 7),
        Err(Waiting::Guest)
    );
    inventory.vms.get_mut(&vm.id).unwrap().is_control_domain = false;
    inventory.vms.get_mut(&vm.id).unwrap().power_state = PowerState::Halted;
    assert_eq!(
        inventory_binding(&inventory, &target, 7),
        Err(Waiting::Power)
    );
    inventory.vms.get_mut(&vm.id).unwrap().power_state = PowerState::Paused;
    assert!(inventory_binding(&inventory, &target, 7).is_ok());
    inventory.hosts.remove(&vm.host.unwrap());
    assert_eq!(
        inventory_binding(&inventory, &target, 7),
        Err(Waiting::Guest)
    );
    inventory.vms.remove(&vm.id);
    assert_eq!(
        inventory_binding(&inventory, &target, 7),
        Err(Waiting::Guest)
    );
}

#[test]
fn automatic_reconnect_toggle_and_pinned_identity_render_at_both_window_sizes() {
    for size in [(1440.0, 900.0), (513.0, 652.0)] {
        let mut s = crate::init();
        crate::demo(&mut s);
        let inventory = s.inventory.as_ref().unwrap();
        let vm = inventory
            .vms
            .values()
            .find(|v| v.power_state == PowerState::Running)
            .unwrap();
        let (pool, id, name) = (inventory.pool_id, vm.id, vm.name.clone());
        super::super::opened(&mut s, pool, id, name);
        let mut rt = rune::Runtime::new(
            s,
            |s, cx| super::super::view(s, &Kit::neutral(cx.system_dark())),
            size,
            rune::Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        rt.frame(0.0);
        let (x, y) = rt.find("console-auto-reconnect").unwrap();
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(0.0);
        assert!(!rt.state.console_ui.recovery.enabled);
        assert_eq!(rt.state.console_ui.target().unwrap().vm, id);
        assert!(!input_ready(&rt.state));
    }
}
