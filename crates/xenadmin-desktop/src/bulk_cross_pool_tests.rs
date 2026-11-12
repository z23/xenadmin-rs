use super::*;
use xenadmin_core::{
    OperationControl, OperationState, ResourceAction, ResourceOperationRecord, VmWorkflow,
};

const SIZES: [(f32, f32); 2] = [(513.0, 652.0), (1440.0, 900.0)];

fn settle(r: &mut rune::Runtime<State>) {
    for _ in 0..30 {
        r.frame(1.0 / 60.0);
    }
}
fn runtime_of(mut s: State, size: (f32, f32)) -> rune::Runtime<State> {
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
/// The synthetic review: a running and a halted VM, mapped and acknowledged.
fn state() -> State {
    let mut s = init();
    scene(&mut s);
    s
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
fn editor(s: &State) -> &Editor {
    s.bulk_cross_pool.as_ref().unwrap()
}
fn moves(review: &Review) -> Vec<&CrossPoolVmOperation> {
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
/// Whether an element is inside the window, so that a person can reach it.
fn inside(r: &mut rune::Runtime<State>, key: &str, size: (f32, f32)) -> bool {
    r.update(|_, tx| tx.scroll_into_view(key));
    settle(r);
    r.find(key)
        .is_some_and(|(x, y)| (0.0..size.0).contains(&x) && (0.0..size.1).contains(&y))
}

#[test]
fn the_inventory_selection_offers_the_move_to_another_pool_beside_the_same_pool_move() {
    for size in SIZES {
        let mut s = init();
        demo(&mut s);
        let mut r = runtime_of(s, size);
        // The entry is in the selection bar with the same-pool move. A page that has no
        // VM rows to select offers neither.
        assert!(r.find("batch-move-open").is_some());
        assert!(r.find("batch-cross-pool-open").is_some());
        r.update(|s, _| s.page = Page::Hosts);
        assert!(bulk_move::selected(&r.state).is_err());
        r.update(|s, _| open(s));
        assert!(r.state.bulk_cross_pool.is_none());
        r.update(|s, _| {
            s.page = Page::Guests;
            s.error = None;
            s.rows.retain(|row| row.state == "Running");
        });
        settle(&mut r);
        let first = r.state.rows[0].id.clone();
        click(&mut r, &format!("inventory-name-{first}"));
        r.key(rune::KeyEvent {
            key: rune::Key::Down,
            mods: rune::Mods {
                shift: true,
                ..Default::default()
            },
        });
        settle(&mut r);
        let selected = bulk_move::selected(&r.state)
            .unwrap()
            .iter()
            .map(|vm| vm.id)
            .collect::<Vec<_>>();
        assert_eq!(selected.len(), 2, "{size:?}");
        assert!(
            r.accessibility_json()
                .to_string()
                .contains("Move selection to another pool…")
        );
        click(&mut r, "batch-cross-pool-open");
        let e = editor(&r.state);
        assert_eq!(
            e.choices.iter().map(|c| c.source.id).collect::<Vec<_>>(),
            selected
        );
        assert_eq!(e.stage, 0);
        assert!(e.review.is_none() && !e.transport_reviewed);
        assert!(e.destination.is_none() && e.host.is_none());
        assert!(r.find("experimental-notice").is_some());
        let text = r.accessibility_json().to_string();
        assert!(text.contains("Move selected VMs to another pool"));
        assert!(text.contains("1 of 3 · Destination pool and host"));
        assert!(text.contains("2 captured VMs"));
        // The synthetic session has one pool, and the editor says what is needed.
        assert!(text.contains("Connect another pool with write access"));
        for key in ["batch-cross-pool-close", "batch-cross-pool-next"] {
            assert!(inside(&mut r, key, size), "{key} at {size:?}");
        }
        click(&mut r, "batch-cross-pool-next");
        let e = editor(&r.state);
        assert_eq!(e.stage, 0);
        assert_eq!(
            e.error.as_deref(),
            Some("Choose a destination pool and host")
        );
        // The editor keeps what it captured when the selection behind it changes.
        r.update(|s, _| s.table.select("changed selection"));
        assert_eq!(editor(&r.state).choices.len(), 2);
        click(&mut r, "batch-cross-pool-close");
        assert!(r.state.bulk_cross_pool.is_none());
    }
    // The row menu has the same entry, after the same-pool move.
    let mut s = init();
    demo(&mut s);
    let mut r = runtime_of(s, (1440.0, 900.0));
    let first = r.state.rows[0].id.clone();
    click(&mut r, &format!("inventory-name-{first}"));
    r.key(rune::Key::ContextMenu.into());
    settle(&mut r);
    let text = r.accessibility_json().to_string();
    let same_pool = text.find("Move selection…").unwrap();
    let other_pool = text.find("Move selection to another pool…").unwrap();
    assert!(same_pool < other_pool);
    click(&mut r, "inventory-context-item8");
    assert_eq!(editor(&r.state).choices.len(), 1);
}

#[test]
fn the_review_lists_every_vm_with_its_mapping_both_pools_and_a_reachable_footer() {
    for size in SIZES {
        let mut r = runtime_of(state(), size);
        let e = editor(&r.state);
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
        let text = r.accessibility_json().to_string();
        assert!(text.contains("3 of 3 · Review both pools"));
        assert!(text.contains("Move 2 VMs to another pool"));
        assert!(text.contains(&target.pool_id.to_string()));
        assert!(text.contains("Live move: the guest keeps running"));
        assert!(text.contains("Halted move: the guest stays halted"));
        for (index, operation) in operations.iter().enumerate() {
            let vm = &e.source.inventory.vms[&operation.vm];
            assert!(text.contains(&format!("{} ({})", vm.name, vm.id)));
            // Each VM's own disk and NIC, with where each goes.
            let lines = &review.rows[index];
            let disk = lines
                .iter()
                .find(|line| line.starts_with(&format!("Disk {}: ", operation.disks[0].vdi)))
                .unwrap_or_else(|| panic!("no disk line in {lines:?}"));
            assert!(disk.contains(&format!(
                "→ {}",
                vm_migration::sr_label(target, operation.disks[0].destination_sr)
            )));
            let nic = lines
                .iter()
                .find(|line| line.starts_with(&format!("NIC {}: ", operation.networks[0].vif)))
                .unwrap_or_else(|| panic!("no NIC line in {lines:?}"));
            assert!(nic.contains(&format!(
                "→ {}",
                vm_migration::net_label(target, operation.networks[0].destination_network)
            )));
            assert!(text.contains(&operation.disks[0].vdi.to_string()));
            assert!(text.contains(&operation.networks[0].vif.to_string()));
        }
        assert!(
            review
                .capacity
                .contains("the disks of the whole selection need 48.0 GiB")
        );
        assert!(text.contains("the disks of the whole selection need"));
        // The single form's transport warning, with one acknowledgement for the batch.
        assert!(text.contains("unencrypted HTTP"));
        assert!(text.contains("for every VM in this batch"));
        assert!(text.contains("Both pools are reserved"));
        assert!(text.contains("never starts a remaining VM"));
        assert!(text.contains("it does not ask the hosts to cancel a transfer"));
        assert!(text.contains("The console of a running guest"));
        assert!(text.contains("Synthetic preview; submission is disabled."));
        assert!(r.find("experimental-notice").is_some());
        // The assessment of each pool's session, and the footer, are inside the window.
        assert!(text.contains("Source pool session") && text.contains("Destination pool session"));
        for key in [
            "review-permission-explanation",
            "review-permission-destination",
            "batch-cross-transport-reviewed",
            "batch-cross-pool-close",
            "batch-cross-pool-back",
            "batch-cross-pool-submit",
        ] {
            assert!(inside(&mut r, key, size), "{key} at {size:?}");
        }
        // A synthetic session submits nothing, and the review stands as it was.
        click(&mut r, "batch-cross-pool-submit");
        assert!(r.state.resource_operations.is_empty());
        assert_eq!(editor(&r.state).review.as_ref(), Some(&review));
    }
}

#[test]
fn each_stage_asks_for_its_choices_and_the_acknowledgement_is_given_once_per_review() {
    let mut s = state();
    let (host, repository, transfer) = {
        let e = editor(&s);
        (
            e.host.unwrap(),
            e.repository.unwrap(),
            e.transfer.clone().unwrap(),
        )
    };
    let network = *editor(&s).networks.values().next().unwrap();
    let e = s.bulk_cross_pool.as_mut().unwrap();
    e.stage = 0;
    e.destination = None;
    e.host = None;
    reset_mappings(e);
    let mut r = runtime_of(s, (513.0, 652.0));
    click(&mut r, "batch-cross-pool-0");
    click(&mut r, "batch-cross-pool-next");
    assert_eq!(editor(&r.state).stage, 0, "a host is needed");
    click(&mut r, &format!("batch-cross-host-{host}"));
    click(&mut r, "batch-cross-pool-next");
    assert_eq!(editor(&r.state).stage, 1);
    let text = r.accessibility_json().to_string();
    assert!(text.contains("2 of 3 · Storage, networks and transfer address"));
    assert!(text.contains("One destination repository takes every disk of the selection"));
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
        click(&mut r, "batch-cross-pool-next");
        let e = editor(&r.state);
        assert_eq!(e.stage, 1);
        assert!(e.review.is_none());
        assert!(
            e.error.as_deref().is_some_and(|e| e.contains(missing)),
            "{missing}: {:?}",
            e.error
        );
        click(&mut r, &choose);
    }
    click(&mut r, "batch-cross-pool-next");
    let e = editor(&r.state);
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
    click(&mut r, "batch-cross-transport-reviewed");
    let e = editor(&r.state);
    assert!(e.transport_reviewed);
    assert!(
        moves(e.review.as_ref().unwrap())
            .iter()
            .all(|o| o.host_transport_reviewed)
    );
    // What the choices make of both pools is the review that was acknowledged.
    assert_eq!(
        build_review(&r.state, e).as_ref(),
        Ok(e.review.as_ref().unwrap())
    );
    click(&mut r, "batch-cross-transport-reviewed");
    let e = editor(&r.state);
    assert!(!e.transport_reviewed);
    assert!(
        moves(e.review.as_ref().unwrap())
            .iter()
            .all(|o| !o.host_transport_reviewed)
    );
    // Going back ends the review and its acknowledgement; so does another host or pool.
    click(&mut r, "batch-cross-transport-reviewed");
    click(&mut r, "batch-cross-pool-back");
    let e = editor(&r.state);
    assert_eq!(e.stage, 1);
    assert!(e.review.is_none() && !e.transport_reviewed);
    assert_eq!(e.repository, Some(repository));
    click(&mut r, "batch-cross-pool-back");
    click(&mut r, &format!("batch-cross-host-{host}"));
    let e = editor(&r.state);
    assert!(e.repository.is_none() && e.networks.is_empty() && e.transfer.is_none());
    click(&mut r, "batch-cross-pool-0");
    assert!(editor(&r.state).host.is_none());
}

#[test]
fn a_vm_or_destination_that_does_not_fit_refuses_the_whole_selection_and_omits_none() {
    let second = |s: &State| editor(s).choices[1].source.id;
    let refused = |s: &State| build_review(s, editor(s)).expect_err("the selection is refused");
    // The second VM was suspended after the editor opened.
    let mut s = state();
    let id = second(&s);
    let name = editor(&s).choices[1].source.name.clone();
    let e = s.bulk_cross_pool.as_mut().unwrap();
    Arc::make_mut(&mut e.source.inventory)
        .vms
        .get_mut(&id)
        .unwrap()
        .power_state = PowerState::Suspended;
    let reason = refused(&s);
    assert!(reason.contains(&format!("{name} ({id})")), "{reason}");
    assert!(
        reason.contains("state or resident host changed"),
        "{reason}"
    );
    assert_eq!(editor(&s).choices.len(), 2);
    // It is gone from the source pool.
    let mut s = state();
    let e = s.bulk_cross_pool.as_mut().unwrap();
    Arc::make_mut(&mut e.source.inventory).vms.remove(&id);
    assert!(refused(&s).contains(&id.to_string()));
    // Its disk became shared: the single move's own refusal, for that VM.
    let mut s = state();
    let disk = editor(&s).choices[1].disks[0].0;
    let e = s.bulk_cross_pool.as_mut().unwrap();
    Arc::make_mut(&mut e.source.inventory)
        .details
        .vdis
        .get_mut(&disk)
        .unwrap()
        .sharable = Some(true);
    let reason = refused(&s);
    assert!(reason.contains(&id.to_string()), "{reason}");
    assert!(
        reason.contains("Shared, unowned, missing, snapshot or CBT disks"),
        "{reason}"
    );
    assert!(!reason.contains(&editor(&s).choices[0].source.id.to_string()));
    // The destination repository is no longer attached to the chosen host: both VMs.
    let mut s = state();
    let e = s.bulk_cross_pool.as_mut().unwrap();
    Arc::make_mut(&mut e.peers[0].inventory)
        .details
        .pbds
        .clear();
    let reason = refused(&s);
    for c in &editor(&s).choices {
        assert!(reason.contains(&c.source.id.to_string()), "{reason}");
    }
    // The repository reports less free space than the two disks need together.
    let mut s = state();
    let repository = editor(&s).repository.unwrap();
    let e = s.bulk_cross_pool.as_mut().unwrap();
    let sr = Arc::make_mut(&mut e.peers[0].inventory)
        .storage
        .iter_mut()
        .find(|sr| sr.uuid == repository.uuid())
        .unwrap();
    sr.capacity_bytes = Some(45 << 30);
    sr.used_bytes = Some(0);
    let reason = refused(&s);
    assert!(
        reason.contains("reports 45.0 GiB free and the disks of the whole selection need 48.0 GiB"),
        "{reason}"
    );
    // Another pool is in front.
    let mut s = state();
    Arc::make_mut(s.inventory.as_mut().unwrap()).pool_id = PoolId::from(uuid::Uuid::from_u128(999));
    assert_eq!(refused(&s), "The captured source pool is no longer current");
    // A VM that was suspended when the editor opened is named on the first stage, and
    // the selection cannot be reviewed.
    let mut s = state();
    let i = s.inventory.clone().unwrap();
    let mut suspended = i.vms[&id].clone();
    suspended.power_state = PowerState::Suspended;
    let e = s.bulk_cross_pool.as_mut().unwrap();
    e.choices[1] = choice(&i, suspended);
    e.stage = 0;
    e.review = None;
    assert!(
        editor(&s).choices[1].refusal.as_deref().is_some_and(
            |r| r.contains("A running guest moves live and a halted guest moves halted")
        )
    );
    assert!(refused(&s).contains("resume or shut this one down first"));
    let r = runtime_of(s, (513.0, 652.0));
    assert!(
        r.accessibility_json()
            .to_string()
            .contains("resume or shut this one down first")
    );
}

#[test]
fn a_review_that_no_longer_matches_the_pools_or_lacks_a_current_session_is_not_submitted() {
    // A change after the review: the dialog says so and nothing is submitted.
    let mut s = state();
    let reviewed = editor(&s).review.clone().unwrap();
    let id = editor(&s).choices[0].source.id;
    let e = s.bulk_cross_pool.as_mut().unwrap();
    Arc::make_mut(&mut e.source.inventory)
        .details
        .vms
        .get_mut(&id)
        .unwrap()
        .affinity = None;
    assert!(
        editor(&s).choices[0].affinity.is_some(),
        "the synthetic VM has a home host"
    );
    assert!(build_review(&s, editor(&s)).is_err());
    let mut r = runtime_of(s, (513.0, 652.0));
    assert!(
        r.accessibility_json()
            .to_string()
            .contains("Home-host or appliance membership is incomplete or changed")
    );
    click(&mut r, "batch-cross-pool-submit");
    assert!(r.state.resource_operations.is_empty());
    assert_eq!(editor(&r.state).review.as_ref(), Some(&reviewed));
    // Without the two captured connections nothing is submitted, acknowledged or not.
    let mut s = state();
    s.demo = false;
    assert!(!current(&s, editor(&s)));
    submit(&mut s);
    assert!(s.resource_operations.is_empty());
    assert!(editor(&s).error.is_some());
    assert_eq!(editor(&s).choices.len(), 2);
    // A session that cannot change this pool is not given a review it could never
    // submit: the entry says what is needed instead.
    let mut s = init();
    demo(&mut s);
    let first = s.rows[0].id.clone();
    s.table.select(first);
    assert!(bulk_move::selected(&s).is_ok());
    s.demo = false;
    open(&mut s);
    assert!(s.bulk_cross_pool.is_none());
    assert!(
        s.error
            .as_deref()
            .is_some_and(|e| e.contains("needs a current connection to this pool"))
    );
}

/// The batch as its history shows it: unknown after an interruption, with its first move
/// unknown and its second never started.
fn records(s: &State) -> (ResourceOperationRecord, ResourceOperationRecord) {
    let e = editor(s);
    let batch = e.review.as_ref().unwrap().batch.clone();
    let pool = e.source.inventory.pool_id;
    let (parent, first, second) = (
        uuid::Uuid::from_u128(9101),
        uuid::Uuid::from_u128(9102),
        uuid::Uuid::from_u128(9103),
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

#[test]
fn history_shows_a_batch_with_each_vm_and_offers_recheck_stop_and_acknowledgement_on_the_batch_only()
 {
    let mut s = state();
    let (batch, child) = records(&s);
    let (parent, first) = (batch.id, child.id);
    let peer = batch.peer_pool().expect("a destination pool");
    assert_eq!(
        Some(peer),
        editor(&s).peers.first().map(|p| p.inventory.pool_id)
    );
    assert!(batch.action.spans_pools() && child.action.spans_pools());
    let export = serde_json::to_value(activity_export::Entry::resource(&batch)).unwrap();
    assert_eq!(export["destination_pool"], peer.to_string());
    assert!(!export.to_string().contains("workflow_batch_plan"));
    let second = batch.workflow_batch[1].vm;
    s.resource_rows.push(batch);
    s.resource_rows.push(child);
    s.bulk_cross_pool = None;
    // With neither pool connected nothing can be rechecked, and a move that belongs to a
    // batch is never controlled by itself.
    assert!(!cross_pool_vm::can_recheck(&s, parent));
    assert!(!cross_pool_vm::can_recheck(&s, first));
    for control in [
        OperationControl::Recheck,
        OperationControl::RequestCancellation,
        OperationControl::AcknowledgeUnknown,
    ] {
        assert!(profiles_view::resource_control_connection(&s, first, control).is_none());
    }
    cross_pool_vm::history(&mut s);
    for size in SIZES {
        let mut s = init();
        scene(&mut s);
        let (batch, child) = records(&s);
        s.resource_rows.push(batch);
        s.resource_rows.push(child);
        s.bulk_cross_pool = None;
        cross_pool_vm::history(&mut s);
        let mut r = runtime_of(s, size);
        let text = r.accessibility_json().to_string();
        assert!(text.contains("Move selected VMs to another pool · OutcomeUnknown"));
        assert!(text.contains(&format!("Destination pool: {peer}")));
        assert!(text.contains(&format!("VM {second} · Not started")));
        assert!(text.contains(&format!("Part of batch {parent}")));
        assert!(text.contains("Acknowledge unresolved batch…"));
        assert!(text.contains("never starts a VM that a batch had not started"));
        assert!(r.find(&format!("cross-recheck-{parent}")).is_some());
        assert!(r.find(&format!("cross-recheck-{first}")).is_none());
        click(&mut r, &format!("cross-recheck-{parent}"));
        assert!(r.state.resource_operations.is_empty());
    }
    // While a batch runs, the pool's history words Stop as the other batches do.
    let mut s = state();
    let (mut batch, _) = records(&s);
    batch.state = OperationState::Tracking;
    batch.can_cancel = true;
    batch.workflow_batch[0].state = Some(OperationState::Tracking);
    s.resource_rows.push(batch);
    s.bulk_cross_pool = None;
    resource_admin::open_pool(&mut s, resource_admin::Kind::History);
    let r = runtime_of(s, (1440.0, 900.0));
    let text = r.accessibility_json().to_string();
    assert!(text.contains("Move selected VMs to another pool · Tracking"));
    assert!(text.contains("Finish current VM and stop"));
}
