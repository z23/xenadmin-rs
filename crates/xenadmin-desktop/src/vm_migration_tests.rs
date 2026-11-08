use super::*;
fn state() -> State {
    let mut s = init();
    demo(&mut s);
    scene(&mut s);
    s
}
fn runtime(mut s: State, size: (f32, f32)) -> rune::Runtime<State> {
    s.window_width = size.0;
    s.window_height = size.1;
    let mut r = rune::Runtime::new(
        s,
        view,
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
    r.frame(0.0);
    settle(r);
}
#[test]
fn compact_and_wide_review_capture_exact_targets_and_remain_preview_only() {
    for size in [(513.0, 652.0), (1440.0, 900.0)] {
        let mut r = runtime(state(), size);
        let f = r.state.vm_migration.as_ref().unwrap();
        let o = f.review.clone().unwrap();
        assert!(
            o.availability(r.state.inventory.as_ref().unwrap(), f.source.id)
                .is_available()
        );
        assert_eq!(o.disks.len(), 1);
        assert_eq!(o.networks.len(), 1);
        assert_ne!(o.expected_host, Some(o.destination));
        let text = r.accessibility_json().to_string();
        assert!(text.contains("3 of 3"));
        assert!(text.contains("partial placement"));
        r.update(|_, tx| tx.scroll_into_view("review-permission-explanation"));
        settle(&mut r);
        let (_, notice_y) = r.find("review-permission-explanation").unwrap();
        assert!((0.0..size.1).contains(&notice_y));
        click(&mut r, "migration-apply");
        assert!(r.state.operations.is_empty());
        assert_eq!(r.state.vm_migration.as_ref().unwrap().review, Some(o));
    }
}
#[test]
fn destination_and_mapping_controls_produce_reviewed_typed_plan() {
    let mut s = state();
    let f = s.vm_migration.as_mut().unwrap();
    f.stage = 0;
    f.review = None;
    let dest = f.destination.unwrap();
    let disk = f.disks[0].destination_sr;
    let net = f.networks[0].destination_network;
    let mut r = runtime(s, (513.0, 652.0));
    click(&mut r, &format!("migration-host-{dest}"));
    click(&mut r, "migration-next");
    click(&mut r, &format!("migration-disk-0-{disk}"));
    click(&mut r, &format!("migration-nic-0-{net}"));
    click(&mut r, &format!("migration-transfer-{net}"));
    click(&mut r, "migration-next");
    let o = r
        .state
        .vm_migration
        .as_ref()
        .unwrap()
        .review
        .as_ref()
        .unwrap();
    assert_eq!(o.destination, dest);
    assert_eq!(o.transfer_network, Some(net));
}
#[test]
fn stale_or_missing_vm_and_changed_devices_prevent_apply() {
    let mut s = state();
    let f = s.vm_migration.as_ref().unwrap();
    let id = f.source.id;
    let o = f.review.clone().unwrap();
    let i = Arc::make_mut(s.inventory.as_mut().unwrap());
    i.details.vifs.get_mut(&o.networks[0].vif).unwrap().network = None;
    assert!(!o.availability(i, id).is_available());
    apply(&mut s);
    assert!(s.operations.is_empty());
    Arc::make_mut(s.inventory.as_mut().unwrap()).pool_id = PoolId::from(uuid::Uuid::from_u128(999));
    assert!(!current(&s, s.vm_migration.as_ref().unwrap()));
    Arc::make_mut(s.inventory.as_mut().unwrap()).vms.remove(&id);
    apply(&mut s);
    assert!(s.operations.is_empty());
}
#[test]
fn halted_storage_move_requires_transfer_network_and_fresh_home_host() {
    let mut s = state();
    let f = s.vm_migration.as_mut().unwrap();
    f.source.power_state = PowerState::Halted;
    f.source.host = None;
    f.transfer = None;
    assert!(operation(f).is_err());
    f.transfer = f.initial.networks.first().map(|n| NetworkId::from(n.uuid));
    let o = operation(f).unwrap();
    assert!(!o.live);
    let id = f.source.id;
    let i = Arc::make_mut(s.inventory.as_mut().unwrap());
    i.vms.get_mut(&id).unwrap().power_state = PowerState::Halted;
    i.vms.get_mut(&id).unwrap().host = None;
    assert!(o.availability(i, id).is_available());
    i.details.vms.get_mut(&id).unwrap().affinity = Some(o.destination);
    assert!(!o.availability(i, id).is_available());
}
