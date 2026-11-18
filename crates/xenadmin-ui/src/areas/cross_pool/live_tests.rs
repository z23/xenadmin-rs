//! The tests the area could not have without connections: against the
//! fixture server with a VM (`crate::testing::Live`), a real `Backend` and
//! current sessions, one pool and then two.
//!
//! The server has one host and one halted VM without disks, interfaces or
//! networks, and serves no transfer: the wizards are tested for what they
//! capture from each pool's connection, for the command's eligibility on a
//! writable and on a read-only session, for reaching a session by its pool
//! when the scope session is another one, and up to the choice the pools
//! cannot offer (a transfer address). Nothing is submitted here.

use uuid::Uuid;
use xenadmin_core::{
    CrossPoolMode, CrossPoolVmOperation, OperationState, PoolId, ResourceAction,
    ResourceOperationRecord, ResourceTarget,
};

use super::commands::MOVE_TO_POOL;
use super::{batch, single, view};
use crate::prelude::*;
use crate::testing::{Live, Server, connect, poll_until};

#[test]
fn a_writable_current_session_opens_the_wizard_on_the_observed_vm_with_no_pool_to_go_to() {
    let mut live = Live::with_vm();
    let (pool, vm) = (live.pool, live.server.vm());
    live.h.select(Target::one(live.vm()));
    assert_eq!(live.h.eligibility(MOVE_TO_POOL.0), Eligibility::Enabled);
    live.h.invoke(MOVE_TO_POOL.0);
    let surface = live.h.surface();
    assert_eq!(surface.id, view::WIZARD);
    assert_eq!(surface.title, "Move halted VM to another pool");
    assert!(surface.experimental);
    let observed = live
        .h
        .model
        .snapshot_for(pool)
        .and_then(|snapshot| snapshot.observe_vm(vm).ok());
    assert!(observed.is_some());
    let e = live.h.model.areas.cross_pool.wizard.as_deref().unwrap();
    assert_eq!((e.pool(), e.vm.id, e.observed), (pool, vm, observed));
    assert_eq!(e.source.profile, Some(live.profile.id));
    assert!(e.source.observed.is_some());
    assert!(e.source.connection(&live.h.model).is_some());
    assert!(e.peers.is_empty());
    assert_eq!(e.mode, CrossPoolMode::MoveHalted);
    assert!(single::current(&live.h.model, e));
    assert!(live.h.find("cross-stale").is_none());
    assert!(live.h.find("cross-no-peers").is_some());
    assert!(live.h.find("cross-mode-CopyHalted").is_some());
    live.h.click("cross-next");
    let e = live.h.model.areas.cross_pool.wizard.as_deref().unwrap();
    assert_eq!((e.stage, e.error.as_deref()), (0, Some(single::NO_DESTINATION)));
    // Applying without a review is inert.
    single::apply(&mut live.h.model);
    assert!(live.h.model.resource_operations.is_empty());
    assert!(
        live.server
            .calls()
            .iter()
            .all(|call| !call.to_lowercase().contains("migrate"))
    );
}

#[test]
fn a_read_only_session_cannot_open_either_wizard() {
    let pool = PoolId::from(Uuid::new_v4());
    let mut live = Live::over(Server::with_vm(pool), pool, true);
    let target = Target::one(live.vm());
    live.h.select(target.clone());
    assert_eq!(
        live.h.eligibility(MOVE_TO_POOL.0),
        Eligibility::disabled(single::NOT_WRITABLE)
    );
    assert_eq!(
        live.h.try_invoke(MOVE_TO_POOL.0),
        Err(single::NOT_WRITABLE.to_owned())
    );
    assert!(live.h.model.areas.cross_pool.wizard.is_none());
    batch::open(&mut live.h.model, &target);
    assert!(live.h.model.areas.cross_pool.batch.is_none());
    assert_eq!(live.h.model.error.as_deref(), Some(batch::NOT_WRITABLE));
    assert!(single::connection_for_pool(&live.h.model, pool).is_none());
}

/// A record of a transfer from one pool to the other, unknown.
fn unknown_transfer(source: PoolId, operation: CrossPoolVmOperation) -> ResourceOperationRecord {
    serde_json::from_value(serde_json::json!({
        "id": Uuid::new_v4(),
        "pool": source,
        "target": ResourceTarget::Pool(source),
        "action": ResourceAction::CrossPoolVm(operation),
        "state": OperationState::OutcomeUnknown,
        "progress": null,
        "detail": "Synthetic unknown transfer",
        "created_at_ms": 1,
        "can_cancel": false,
        "cancel_requested": false,
        "steps": [],
        "created": null,
    }))
    .unwrap()
}

/// Two pools connected for writing, the scope session being the second:
/// the wizard of a VM of the first reaches each pool through its own
/// connection.
#[test]
fn with_two_pools_the_wizard_captures_each_through_its_own_connection() {
    let mut live = Live::with_vm();
    let (pool, vm) = (live.pool, live.server.vm());
    let other_pool = PoolId::from(Uuid::new_v4());
    let other = Server::with_vm(other_pool);
    let second = connect(&mut live.h.model, &other, other_pool, "Second", false);
    poll_until(&mut live.h, "both pools are current and writable", |h| {
        h.model.can_mutate_pool(pool) && h.model.can_mutate_pool(other_pool)
    });
    // The scope session is the second pool's; the target is in the first.
    assert_eq!(
        live.h.model.inventory.as_ref().map(|i| i.pool_id),
        Some(other_pool)
    );
    for id in [pool, other_pool] {
        assert!(single::connection_for_pool(&live.h.model, id).is_some());
    }
    assert_eq!(single::peers(&live.h.model).len(), 2);
    let target = Target::one(live.vm());
    assert_eq!(
        live.h.model.eligibility(MOVE_TO_POOL, &target),
        Eligibility::Enabled
    );
    live.h.invoke_on(MOVE_TO_POOL.0, &target);
    let e = live.h.model.areas.cross_pool.wizard.as_deref().unwrap();
    assert_eq!((e.pool(), e.vm.id), (pool, vm));
    assert_eq!(e.source.profile, Some(live.profile.id));
    assert_eq!(
        e.peers
            .iter()
            .map(|p| (p.pool(), p.profile))
            .collect::<Vec<_>>(),
        [(other_pool, Some(second.id))]
    );
    assert!(single::current(&live.h.model, e));
    live.h.click("cross-pool-0");
    live.h.click(&format!("cross-host-{}", other.host()));
    let e = live.h.model.areas.cross_pool.wizard.as_deref().unwrap();
    assert!(single::current(&live.h.model, e));
    assert!(e.peers[0].connection(&live.h.model).is_some());
    live.h.click("cross-next");
    live.h.click("cross-next");
    // The destination host has no attached interface with an address.
    let e = live.h.model.areas.cross_pool.wizard.as_deref().unwrap();
    assert_eq!(e.stage, 1);
    assert_eq!(
        e.error.as_deref(),
        Some("Choose an attached destination transfer address")
    );
    // A transfer between the two can be rechecked while both are connected
    // for writing, whichever is the scope session.
    let record = unknown_transfer(
        pool,
        CrossPoolVmOperation {
            vm,
            destination_pool: other_pool,
            destination_host: other.host(),
            expected_host: None,
            expected_affinity: None,
            transfer_network: xenadmin_core::NetworkId::from(Uuid::new_v4()),
            transfer_address: "192.0.2.10".into(),
            mode: CrossPoolMode::MoveHalted,
            disks: Vec::new(),
            networks: Vec::new(),
            host_transport_reviewed: true,
        },
    );
    let id = record.id;
    live.h.model.resource_rows.push(record);
    assert!(single::can_recheck(&live.h.model, id));

    // The batch of the same VM captures the same two connections.
    batch::open(&mut live.h.model, &target);
    assert!(live.h.model.areas.cross_pool.wizard.is_none());
    let e = live.h.model.areas.cross_pool.batch.as_ref().unwrap();
    assert_eq!(e.source.profile, Some(live.profile.id));
    assert_eq!(e.peers.len(), 1);
    assert!(batch::captured(&live.h.model, e));
    assert!(!batch::current(&live.h.model, e), "no destination yet");
    assert!(live.h.find("batch-cross-pool-stale").is_none());
    assert!(live.h.find("batch-cross-pool-preview").is_none());
    live.h.click("batch-cross-pool-0");
    let e = live.h.model.areas.cross_pool.batch.as_ref().unwrap();
    assert!(batch::current(&live.h.model, e));
    batch::submit(&mut live.h.model);
    let e = live.h.model.areas.cross_pool.batch.as_ref().unwrap();
    assert_eq!(e.error, Some(xenadmin_core::Error::Stale.to_string()));
    assert!(live.h.model.resource_operations.is_empty());
    for server in [&live.server, &other] {
        assert!(
            server
                .calls()
                .iter()
                .all(|call| !call.to_lowercase().contains("migrate"))
        );
    }
}
