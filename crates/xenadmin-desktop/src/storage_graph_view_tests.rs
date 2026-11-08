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
            scale: 1.,
        },
    );
    settle(&mut r);
    r
}
fn settle(r: &mut rune::Runtime<State>) {
    for _ in 0..30 {
        r.frame(1. / 60.);
    }
}
fn click(r: &mut rune::Runtime<State>, key: &str) {
    r.update(|_, t| t.scroll_into_view(key));
    settle(r);
    let (x, y) = r.find(key).unwrap_or_else(|| panic!("missing {key}"));
    r.pointer_down(x, y);
    r.pointer_up(x, y);
    settle(r);
}
#[test]
fn reviewed_storage_graph_is_captured_reachable_and_never_submits_demo() {
    for multipath in [false, true] {
        for size in [(1440., 900.), (513., 652.)] {
            let mut s = init();
            scene(&mut s, multipath);
            assert!(
                s.storage_graph.as_ref().unwrap().review.is_some(),
                "{:?}",
                s.storage_graph.as_ref().unwrap().error
            );
            let captured = s.storage_graph.as_ref().unwrap().review.clone();
            let mut r = runtime(s, size);
            for key in ["storage-graph-close", "storage-graph-submit"] {
                let (x, y) = r.find(key).unwrap();
                assert!(
                    x >= 0. && x < size.0 && y >= 0. && y < size.1,
                    "{key}: {x},{y}"
                );
            }
            r.update(|s, _| {
                s.page = Page::Guests;
                s.rebuild();
            });
            settle(&mut r);
            assert_eq!(r.state.storage_graph.as_ref().unwrap().review, captured);
            click(&mut r, "storage-graph-submit");
            assert!(r.state.resource_operations.is_empty());
        }
    }
}
#[test]
fn missing_repair_requires_selection_and_rejects_new_pair_before_review() {
    let mut s = init();
    scene(&mut s, false);
    let e = editor(&mut s);
    e.review = None;
    let selected = *e.hosts.first().unwrap();
    let source = e.source.unwrap();
    e.hosts.clear();
    stage(&mut s);
    assert!(s.storage_graph.as_ref().unwrap().review.is_none());
    editor(&mut s).hosts.insert(selected);
    let i = Arc::make_mut(s.inventory.as_mut().unwrap());
    let mut new = i.details.pbds[&source].clone();
    new.id = PbdId::from(uuid::Uuid::from_u128(9951));
    new.host = Some(selected);
    new.currently_attached = Some(false);
    i.details.pbds.insert(new.id, new);
    stage(&mut s);
    assert!(s.storage_graph.as_ref().unwrap().review.is_none());
    assert!(
        s.storage_graph
            .as_ref()
            .unwrap()
            .error
            .as_ref()
            .unwrap()
            .contains("changed")
    );
}
#[test]
fn changed_multipath_attachment_and_pool_disable_captured_review() {
    let mut s = init();
    scene(&mut s, true);
    let id = *s
        .inventory
        .as_ref()
        .unwrap()
        .details
        .pbds
        .keys()
        .next()
        .unwrap();
    let e = s.storage_graph.as_ref().unwrap();
    assert!(
        e.review
            .as_ref()
            .unwrap()
            .availability(s.inventory.as_ref().unwrap(), e.target)
            .is_available()
    );
    let i = Arc::make_mut(s.inventory.as_mut().unwrap());
    i.details.pbds.get_mut(&id).unwrap().currently_attached = Some(false);
    let e = s.storage_graph.as_ref().unwrap();
    assert!(
        !e.review
            .as_ref()
            .unwrap()
            .availability(s.inventory.as_ref().unwrap(), e.target)
            .is_available()
    );
    let i = Arc::make_mut(s.inventory.as_mut().unwrap());
    i.pool_id = PoolId::from(uuid::Uuid::from_u128(999));
    assert!(!current(&s, s.storage_graph.as_ref().unwrap()));
}
#[test]
fn multipath_refuses_unknown_handler_and_nonmaintenance_host() {
    for changed in ["handler", "maintenance", "ha"] {
        let mut s = init();
        scene(&mut s, true);
        let mut i = (*s.inventory.as_ref().unwrap().as_ref()).clone();
        let ResourceTarget::Host(host) = s.storage_graph.as_ref().unwrap().target else {
            panic!()
        };
        match changed {
            "handler" => {
                i.details
                    .hosts
                    .get_mut(&host)
                    .unwrap()
                    .standard_multipath_handler = None
            }
            "maintenance" => i.details.hosts.get_mut(&host).unwrap().maintenance_mode = Some(false),
            _ => i.details.pool.as_mut().unwrap().ha_enabled = Some(true),
        }
        editor(&mut s).initial = Arc::new(i);
        assert!(proposal(s.storage_graph.as_ref().unwrap()).is_err());
    }
}
#[test]
fn edit_host_selection_then_review_keeps_exact_uuid() {
    let mut s = init();
    scene(&mut s, false);
    let host = *s.storage_graph.as_ref().unwrap().hosts.first().unwrap();
    let mut r = runtime(s, (513., 652.));
    click(&mut r, "storage-graph-back");
    click(&mut r, &format!("storage-host-{host}"));
    assert!(r.state.storage_graph.as_ref().unwrap().hosts.is_empty());
    click(&mut r, &format!("storage-host-{host}"));
    click(&mut r, "storage-graph-review");
    let StorageGraph::Repair { hosts, .. } = r
        .state
        .storage_graph
        .as_ref()
        .unwrap()
        .review
        .as_ref()
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(hosts.len(), 1);
    assert_eq!(hosts[0].host, host);
    assert!(hosts[0].attachment.is_none());
}
#[test]
fn partial_outcome_restore_is_a_new_review_and_unknown_cannot_offer_it() {
    for state in [
        OperationState::PartiallyCompleted,
        OperationState::OutcomeUnknown,
    ] {
        let mut s = init();
        scene(&mut s, true);
        let e = s.storage_graph.as_ref().unwrap();
        let StorageGraph::Multipath { attachments, .. } = e.review.as_ref().unwrap() else {
            panic!()
        };
        let p = attachments[0].clone();
        let record: xenadmin_core::ResourceOperationRecord=serde_json::from_value(serde_json::json!({"id":OperationId::new(),"pool":e.pool,"target":e.target,"action":ResourceAction::StorageGraph(e.review.clone().unwrap()),"state":state,"progress":0.25,"detail":"Fixture stopped after disconnect","created_at_ms":1,"can_cancel":false,"cancel_requested":false,"steps":[],"created":null,"storage_graph":[{"host":p.host,"sr":p.sr,"pbd":p.pbd,"created":false,"attached":false,"verified":true}]})).unwrap();
        s.resource_rows.push(record);
        Arc::make_mut(s.inventory.as_mut().unwrap())
            .details
            .pbds
            .get_mut(&p.pbd)
            .unwrap()
            .currently_attached = Some(false);
        let mut r = runtime(s, (513., 652.));
        let key = format!("storage-restore-{}", p.pbd);
        if state == OperationState::PartiallyCompleted {
            click(&mut r, &key);
            assert!(r.state.storage_graph.is_none());
            assert!(r.state.storage_editor.is_some());
            assert!(r.state.resource_operations.is_empty());
            assert!(
                r.accessibility_json()
                    .to_string()
                    .contains("Review storage change")
            );
        } else {
            assert!(r.find(&key).is_none());
        }
    }
}
#[test]
fn restore_cannot_turn_into_detach_after_state_changes() {
    let mut s = init();
    scene(&mut s, true);
    let p = s
        .inventory
        .as_ref()
        .unwrap()
        .details
        .pbds
        .values()
        .next()
        .unwrap()
        .clone();
    storage_view::review_restore(&mut s, p.sr.unwrap(), p.id, p.host.unwrap());
    assert!(s.storage_editor.is_none());
    assert!(s.error.as_ref().unwrap().contains("changed"));
}
