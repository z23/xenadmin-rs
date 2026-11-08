use super::*;
fn state() -> State {
    let mut s = init();
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
fn reviewed_mapping_and_host_transport_warning_render_at_both_widths_without_demo_dispatch() {
    for size in [(513.0, 652.0), (1440.0, 900.0)] {
        let mut r = runtime(state(), size);
        let e = r.state.cross_pool.editor.as_ref().unwrap();
        assert!(
            availability(&r.state, e).is_available(),
            "{}",
            availability(&r.state, e).explanation()
        );
        let operation = e.review.clone().unwrap();
        assert_ne!(operation.destination_pool, e.source.inventory.pool_id);
        let text = r.accessibility_json().to_string();
        assert!(text.contains("3 of 3"));
        assert!(text.contains("unencrypted HTTP"));
        assert!(text.contains("Both pools are reserved"));
        click(&mut r, "cross-apply");
        assert!(r.state.resource_operations.is_empty());
        click(&mut r, "cross-transport-reviewed");
        assert!(
            !r.state
                .cross_pool
                .editor
                .as_ref()
                .unwrap()
                .review
                .as_ref()
                .unwrap()
                .host_transport_reviewed
        );
        assert!(
            !availability(&r.state, r.state.cross_pool.editor.as_ref().unwrap()).is_available()
        );
    }
}
#[test]
fn choosing_a_pool_requires_explicit_device_mappings_and_new_transport_review() {
    let mut s = state();
    let e = s.cross_pool.editor.as_mut().unwrap();
    e.stage = 0;
    e.review = None;
    let host = e.host.unwrap();
    let sr = e.disks[0].destination_sr;
    let net = e.networks[0].destination_network;
    let (transfer, address) = e.transfer.clone().unwrap();
    let mut r = runtime(s, (513.0, 652.0));
    click(&mut r, "cross-pool-0");
    assert!(
        r.state.cross_pool.editor.as_ref().unwrap().disks[0]
            .destination_sr
            .uuid()
            .is_nil()
    );
    click(&mut r, &format!("cross-host-{host}"));
    click(&mut r, "cross-next");
    click(&mut r, "cross-next");
    assert_eq!(r.state.cross_pool.editor.as_ref().unwrap().stage, 1);
    click(&mut r, &format!("cross-disk-0-{sr}"));
    click(&mut r, &format!("cross-nic-0-{net}"));
    click(&mut r, &format!("cross-address-{transfer}-{address}"));
    click(&mut r, "cross-next");
    let e = r.state.cross_pool.editor.as_ref().unwrap();
    assert_eq!(e.stage, 2);
    assert!(!e.review.as_ref().unwrap().host_transport_reviewed);
    click(&mut r, "cross-transport-reviewed");
    assert!(availability(&r.state, r.state.cross_pool.editor.as_ref().unwrap()).is_available());
}
#[test]
fn current_source_membership_and_destination_attachments_gate_review() {
    let mut s = state();
    let e = s.cross_pool.editor.as_mut().unwrap();
    let o = e.review.clone().unwrap();
    let i = Arc::make_mut(&mut e.source.inventory);
    i.details.vdis.get_mut(&o.disks[0].vdi).unwrap().sharable = Some(true);
    assert!(!availability(&s, s.cross_pool.editor.as_ref().unwrap()).is_available());
    let mut s = state();
    let e = s.cross_pool.editor.as_mut().unwrap();
    Arc::make_mut(&mut e.peers[0].inventory)
        .details
        .pbds
        .clear();
    assert!(!availability(&s, s.cross_pool.editor.as_ref().unwrap()).is_available());
    let mut s = state();
    Arc::make_mut(s.inventory.as_mut().unwrap()).pool_id = PoolId::from(uuid::Uuid::from_u128(999));
    assert!(!current(&s, s.cross_pool.editor.as_ref().unwrap()));
    apply(&mut s);
    assert!(s.resource_operations.is_empty());
}
#[test]
fn halted_copy_review_preserves_source_and_uses_distinct_identity_semantics() {
    let mut s = state();
    let e = s.cross_pool.editor.as_mut().unwrap();
    e.vm.power_state = PowerState::Halted;
    e.vm.host = None;
    let i = Arc::make_mut(&mut e.source.inventory);
    i.vms.get_mut(&e.vm.id).unwrap().power_state = PowerState::Halted;
    i.vms.get_mut(&e.vm.id).unwrap().host = None;
    e.mode = CrossPoolMode::CopyHalted;
    e.review = Some(operation(e).unwrap());
    assert!(availability(&s, s.cross_pool.editor.as_ref().unwrap()).is_available());
    let r = runtime(s, (513.0, 652.0));
    let text = r.accessibility_json().to_string();
    assert!(text.contains("new MAC addresses"));
    assert!(text.contains("source stays halted"));
}
fn record(s: &State, state: OperationState) -> ResourceOperationRecord {
    let e = s.cross_pool.editor.as_ref().unwrap();
    serde_json::from_value(serde_json::json!({"id":uuid::Uuid::from_u128(9001),"pool":e.source.inventory.pool_id,"target":ResourceTarget::Pool(e.source.inventory.pool_id),"action":ResourceAction::CrossPoolVm(e.review.clone().unwrap()),"state":state,"progress":0.5,"detail":"Synthetic partial transfer","created_at_ms":1,"can_cancel":false,"cancel_requested":false,"steps":[],"created":null,"transfer_created":{"vms":[e.vm.id],"disks":[uuid::Uuid::from_u128(9002)],"vbds":[],"vifs":[],"networks":[]}})).unwrap()
}
#[test]
fn offline_history_keeps_both_pool_namespaces_and_partial_resources_after_source_disappears() {
    let mut s = state();
    let record = record(&s, OperationState::OutcomeUnknown);
    let id = record.id;
    let peer = record.peer_pool().unwrap();
    let export = serde_json::to_value(activity_export::Entry::resource(&record)).unwrap();
    assert_eq!(export["destination_pool"], peer.to_string());
    assert_eq!(
        export["target"],
        s.cross_pool.editor.as_ref().unwrap().vm.id.to_string()
    );
    assert!(!export.to_string().contains("cross_pool_plan"));
    s.resource_rows.push(record);
    s.inventory = None;
    history(&mut s);
    let mut r = runtime(s, (513.0, 652.0));
    let text = r.accessibility_json().to_string();
    assert!(text.contains(&peer.to_string()));
    assert!(text.contains("Destination Disk"));
    assert!(!can_recheck(&r.state, id));
    click(&mut r, &format!("cross-recheck-{id}"));
    assert!(r.state.resource_operations.is_empty());
}

#[test]
fn destination_console_requires_proven_result_and_exact_current_pool_vm_host_and_power_state() {
    let s = state();
    let mut record = record(&s, OperationState::Succeeded);
    let e = s.cross_pool.editor.as_ref().unwrap();
    let operation = e.review.as_ref().unwrap();
    let mut dest = e.peers[0].inventory.as_ref().clone();
    let mut guest = e.vm.clone();
    guest.host = Some(operation.destination_host);
    guest.power_state = PowerState::Running;
    dest.vms.insert(guest.id, guest.clone());
    assert_eq!(destination_guest(&dest, &record), Some((guest.id, true)));
    record.state = OperationState::OutcomeUnknown;
    assert!(destination_guest(&dest, &record).is_none());
    record.state = OperationState::Succeeded;
    dest.vms.get_mut(&guest.id).unwrap().host = None;
    assert!(destination_guest(&dest, &record).is_none());
    dest.vms.get_mut(&guest.id).unwrap().host = Some(operation.destination_host);
    dest.vms.get_mut(&guest.id).unwrap().power_state = PowerState::Halted;
    assert!(destination_guest(&dest, &record).is_none());
    dest.vms.get_mut(&guest.id).unwrap().power_state = PowerState::Running;
    dest.pool_id = e.source.inventory.pool_id;
    assert!(destination_guest(&dest, &record).is_none());
    dest.pool_id = operation.destination_pool;
    dest.vms.clear();
    assert!(destination_guest(&dest, &record).is_none());
}
