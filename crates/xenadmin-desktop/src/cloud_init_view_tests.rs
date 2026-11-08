use super::*;
fn runtime(size: (f32, f32)) -> rune::Runtime<State> {
    let mut s = init();
    scene(&mut s);
    s.window_width = size.0;
    s.window_height = size.1;
    let mut rt = rune::Runtime::new(
        s,
        view,
        size,
        rune::Env {
            system_dark: true,
            scale: 1.0,
        },
    );
    settle(&mut rt);
    rt
}
fn settle(rt: &mut rune::Runtime<State>) {
    for _ in 0..30 {
        rt.frame(1.0 / 60.0);
    }
}
fn click(rt: &mut rune::Runtime<State>, key: &str) {
    rt.update(|_, tx| tx.scroll_into_view(key));
    settle(rt);
    let (x, y) = rt.find(key).unwrap_or_else(|| panic!("missing {key}"));
    assert!(
        x >= 0.0 && x <= rt.state.window_width && y >= 0.0 && y <= rt.state.window_height,
        "{key} outside viewport ({x},{y})"
    );
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    settle(rt);
}
fn prepared(rt: &mut rune::Runtime<State>) {
    for _ in 0..1000 {
        rt.update(|s, _| {
            poll(s);
        });
        if !rt.state.cloud_init.pending {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(
        rt.state.cloud_init.media.is_some(),
        "{:?}",
        rt.state.cloud_init.error
    );
    settle(rt);
}
#[test]
fn cloud_init_review_keeps_target_and_resources_at_compact_and_wide_sizes() {
    for size in [(1440.0, 900.0), (513.0, 652.0)] {
        let mut rt = runtime(size);
        let target = rt.state.cloud_init.vm;
        let sr = rt.state.cloud_init.sr;
        rt.update(|s, _| {
            s.cloud_init.user_data = TextEdit::new("#cloud-config\nwrite_files: []\n");
        });
        click(&mut rt, "cloud-init-prepare");
        prepared(&mut rt);
        assert!(rt.state.cloud_init.user_data.text.is_empty());
        let config_id = rt
            .state
            .cloud_init
            .media
            .as_ref()
            .unwrap()
            .configuration_id();
        rt.update(|s, _| {
            s.table.select("30000000-0000-0000-0000-000000000000");
        });
        assert_eq!(rt.state.cloud_init.vm, target);
        assert_eq!(rt.state.cloud_init.sr, sr);
        assert_eq!(
            rt.state
                .cloud_init
                .media
                .as_ref()
                .unwrap()
                .configuration_id(),
            config_id
        );
        click(&mut rt, "cloud-init-apply");
        assert!(rt.state.resource_operations.is_empty());
        assert!(rt.state.cloud_init.media.is_some());
        click(&mut rt, "cloud-init-close");
        assert!(!rt.state.cloud_init.show);
        assert!(rt.state.cloud_init.media.is_none());
    }
}
#[test]
fn cloud_init_bad_inputs_preserve_editor_and_pool_changes_disable_review() {
    let mut rt = runtime((513.0, 652.0));
    rt.update(|s, _| {
        s.cloud_init.hostname = TextEdit::new("bad/host");
        s.cloud_init.user_data = TextEdit::new("#cloud-config\nfixture: preserve-me\n");
    });
    click(&mut rt, "cloud-init-prepare");
    assert!(rt.state.cloud_init.error.is_some());
    assert!(rt.state.cloud_init.user_data.text.contains("preserve-me"));
    assert!(!rt.state.cloud_init.pending);
    rt.update(|s, _| s.cloud_init.hostname = TextEdit::new("valid"));
    click(&mut rt, "cloud-init-prepare");
    prepared(&mut rt);
    rt.update(|s, _| {
        Arc::make_mut(s.inventory.as_mut().unwrap()).pool_id =
            PoolId::from(uuid::Uuid::from_u128(999))
    });
    assert!(!current(&rt.state));
    click(&mut rt, "cloud-init-apply");
    assert!(rt.state.resource_operations.is_empty());
    click(&mut rt, "cloud-init-close");
}
#[test]
fn cloud_init_cancelling_preparation_drops_late_media_without_reopening() {
    let mut rt = runtime((1440.0, 900.0));
    click(&mut rt, "cloud-init-prepare");
    click(&mut rt, "cloud-init-close");
    for _ in 0..20 {
        rt.update(|s, _| {
            poll(s);
        });
    }
    assert!(!rt.state.cloud_init.show);
    assert!(rt.state.cloud_init.media.is_none());
    assert!(rt.state.resource_operations.is_empty());
}
#[test]
fn cloud_init_slot_validation_refuses_occupied_and_noncanonical_slots() {
    let mut rt = runtime((1440.0, 900.0));
    for slot in ["-1", "256", "01"] {
        rt.update(|s, _| s.cloud_init.device = TextEdit::new(slot));
        assert!(!can_prepare(&rt.state));
    }
    let vm = rt.state.cloud_init.vm.unwrap();
    let slot = rt
        .state
        .inventory
        .as_ref()
        .unwrap()
        .details
        .vbds
        .values()
        .find(|v| v.vm == Some(vm))
        .unwrap()
        .userdevice
        .clone();
    rt.update(|s, _| s.cloud_init.device = TextEdit::new(slot));
    assert!(!can_prepare(&rt.state));
}

#[test]
fn creation_handoff_prepares_private_media_for_the_exact_created_target_and_chosen_sr() {
    let mut rt = runtime((513.0, 652.0));
    let vm = rt.state.cloud_init.vm.unwrap();
    let sr = rt.state.cloud_init.sr.unwrap();
    close(&mut rt.state);
    open_created(
        &mut rt.state,
        vm,
        sr,
        "created-fixture".into(),
        "#cloud-config\nprivate-fixture-token: true\n".into(),
        String::new(),
    );
    prepared(&mut rt);
    assert_eq!(rt.state.cloud_init.vm, Some(vm));
    assert_eq!(rt.state.cloud_init.sr, Some(sr));
    assert!(rt.state.cloud_init.user_data.text.is_empty());
    assert!(
        !rt.accessibility_json()
            .to_string()
            .contains("private-fixture-token")
    );
    assert!(rt.state.resource_operations.is_empty());
    close(&mut rt.state);
}
