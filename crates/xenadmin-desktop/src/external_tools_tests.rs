use super::*;
fn runtime(configure_only: bool, size: (f32, f32)) -> rune::Runtime<State> {
    let mut s = init();
    scene(&mut s, configure_only);
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
    for _ in 0..25 {
        r.frame(1.0 / 60.0);
    }
}
fn click(r: &mut rune::Runtime<State>, key: &str) {
    r.update(|_, tx| tx.scroll_into_view(key));
    settle(r);
    let (x, y) = r.find(key).unwrap_or_else(|| panic!("missing {key}"));
    assert!(
        x >= 0.0 && x <= r.state.window_width && y >= 0.0 && y <= r.state.window_height,
        "{key} {x},{y}"
    );
    r.pointer_down(x, y);
    r.pointer_up(x, y);
    settle(r);
}
#[test]
fn reviewed_guest_command_keeps_target_and_rejects_changed_input_at_both_sizes() {
    for size in [(513.0, 652.0), (1440.0, 900.0)] {
        let mut r = runtime(false, size);
        r.update(|s, _| {
            s.preferences.external_tools.tools[0].executable = "/usr/bin/true".into();
        });
        let target = launch_mut(&mut r.state).target.vm;
        click(&mut r, "review-guest-tool");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while launch_mut(&mut r.state).pending.is_some() {
            r.update(|s, _| {
                poll(s);
            });
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(launch_mut(&mut r.state).review.is_some());
        r.update(|s, _| s.table.select("unrelated-vm"));
        assert_eq!(launch_mut(&mut r.state).target.vm, target);
        click(&mut r, "launch-guest-tool");
        assert!(
            r.state.external_tools.jobs.is_empty(),
            "synthetic scenes never launch"
        );
        r.update(|s, _| {
            launch_mut(s).address.text = "192.0.2.24".into();
            poll(s);
        });
        assert!(launch_mut(&mut r.state).review.is_none());
        click(&mut r, "configure-guest-tools");
        assert!(matches!(
            r.state.external_tools.panel,
            Some(Panel::Configure(_))
        ));
        click(&mut r, "close-guest-tools");
        assert_eq!(launch_mut(&mut r.state).target.vm, target);
        click(&mut r, "close-guest-tools");
        assert!(!is_open(&r.state));
    }
}
#[test]
fn local_tool_configuration_saves_separate_arguments_and_can_remove_every_tool() {
    for size in [(513.0, 652.0), (1440.0, 900.0)] {
        let mut r = runtime(true, size);
        r.update(|s, _| {
            let p = config_mut(s);
            p.executable.text = "/usr/bin/true".into();
            p.arguments.text = r#"["literal with spaces", "{vm_uuid}"]"#.into();
        });
        click(&mut r, "save-guest-tools");
        assert_eq!(
            r.state.preferences.external_tools.tools[0].arguments,
            vec!["literal with spaces", "{vm_uuid}"]
        );
        r.update(|s, _| config_mut(s).arguments.text = "[\"{session}\"]".into());
        click(&mut r, "save-guest-tools");
        assert!(
            config_mut(&mut r.state)
                .error
                .as_deref()
                .unwrap()
                .contains("placeholders")
        );
        click(&mut r, "remove-guest-tool");
        click(&mut r, "remove-guest-tool");
        click(&mut r, "save-guest-tools");
        assert!(r.state.preferences.external_tools.tools.is_empty());
        click(&mut r, "close-guest-tools");
        assert!(!is_open(&r.state));
    }
}
#[test]
fn captured_guest_checks_pool_generation_power_and_current_reported_ip() {
    let mut s = init();
    scene(&mut s, false);
    let target = launch_mut(&mut s).target.clone();
    let mut inventory = (**s.inventory.as_ref().unwrap()).clone();
    let ip = "192.0.2.23".parse().unwrap();
    assert!(
        target
            .observed(true, target.generation, Some(&inventory), ip, false)
            .is_ok()
    );
    assert!(
        target
            .observed(false, target.generation, Some(&inventory), ip, false)
            .is_err()
    );
    assert!(
        target
            .observed(true, target.generation + 1, Some(&inventory), ip, false)
            .is_err()
    );
    let original = inventory.pool_id;
    inventory.pool_id = PoolId::from(uuid::Uuid::new_v4());
    assert!(
        target
            .observed(true, target.generation, Some(&inventory), ip, false)
            .is_err()
    );
    inventory.pool_id = original;
    let id = xenadmin_core::GuestMetricsId::from(uuid::Uuid::new_v4());
    inventory
        .details
        .vms
        .entry(target.vm)
        .or_default()
        .guest_metrics = Some(id);
    inventory.details.guest_metrics.insert(
        id,
        xenadmin_core::GuestMetrics {
            id,
            os_version: Default::default(),
            tools_version: Default::default(),
            networks: std::collections::BTreeMap::from([("0/ip".into(), ip.to_string())]),
            live: Some(true),
            last_updated: None,
        },
    );
    assert!(
        target
            .observed(true, target.generation, Some(&inventory), ip, true)
            .is_ok()
    );
    inventory.details.guest_metrics.get_mut(&id).unwrap().live = Some(false);
    assert!(
        target
            .observed(true, target.generation, Some(&inventory), ip, true)
            .is_err()
    );
    inventory.details.guest_metrics.get_mut(&id).unwrap().live = Some(true);
    inventory
        .details
        .guest_metrics
        .get_mut(&id)
        .unwrap()
        .networks
        .clear();
    assert!(
        target
            .observed(true, target.generation, Some(&inventory), ip, true)
            .is_err()
    );
    inventory.vms.get_mut(&target.vm).unwrap().power_state = PowerState::Halted;
    assert!(
        target
            .observed(true, target.generation, Some(&inventory), ip, false)
            .is_err()
    );
}

#[test]
fn delayed_command_review_cannot_apply_after_edit_or_close() {
    let mut s = init();
    scene(&mut s, false);
    s.preferences.external_tools.tools[0].executable = "/usr/bin/true".into();
    let p = match &s.external_tools.panel {
        Some(Panel::Launch(p)) => p,
        _ => panic!(),
    };
    let captured = inputs(&s, p).unwrap();
    let command = Command::prepare(&captured.tool, &captured.context).unwrap();
    let (sender, receiver) = mpsc::channel();
    launch_mut(&mut s).pending = Some(Preparation {
        inputs: captured.clone(),
        receiver,
    });
    launch_mut(&mut s).address.text = "192.0.2.254".into();
    sender
        .send(Ok(Review {
            inputs: captured.clone(),
            command: command.clone(),
        }))
        .unwrap();
    poll(&mut s);
    assert!(launch_mut(&mut s).review.is_none());
    assert!(
        launch_mut(&mut s)
            .error
            .as_deref()
            .unwrap()
            .contains("Inputs changed")
    );
    let (sender, receiver) = mpsc::channel();
    launch_mut(&mut s).pending = Some(Preparation {
        inputs: captured.clone(),
        receiver,
    });
    back(&mut s);
    assert!(
        sender
            .send(Ok(Review {
                inputs: captured,
                command
            }))
            .is_err()
    );
    assert!(!is_open(&s));
    assert!(s.external_tools.jobs.is_empty());
}
