//! Resource operations per pool, against the fixture server
//! (`crate::testing`): two real sessions.
//!
//! What these pin: the handle of a resource operation belongs to the
//! session that showed its pool when it was tracked; it stays in
//! `m.resource_operations`, polled, whichever pool the tree shows;
//! `m.resource_rows` lists what every connected session's history holds;
//! a session's handles go with its connection and their records stay.
//! (The fixture server does not serve the operation itself: the record
//! ends as the core ends an operation the host refuses.)

use xenadmin_core::{
    OperationControl, OperationId, PoolId, PoolOperation, ResourceCommand, ResourceOperationRecord,
    ResourceTarget,
};

use super::*;
use crate::testing::{Live, Server, connect, poll_until};

struct Two {
    live: Live,
    other: Server,
    other_pool: PoolId,
    other_profile: ProfileId,
}

/// Two writable sessions; the second is the scope session.
fn two_pools() -> Two {
    let mut live = Live::with_vm();
    let other_pool = PoolId::from(uuid::Uuid::new_v4());
    let other = Server::with_vm(other_pool);
    let profile = connect(&mut live.h.model, &other, other_pool, "Second", false);
    let first = live.pool;
    poll_until(&mut live.h, "both pools are current", |h| {
        h.model.can_mutate_pool(first) && h.model.can_mutate_pool(other_pool)
    });
    assert_eq!(live.h.model.workspace.selected, Some(profile.id));
    Two {
        live,
        other,
        other_pool,
        other_profile: profile.id,
    }
}

/// Submits a pool operation on the connection of `pool` and tracks it, as
/// an area does.
fn submit(m: &mut Model, pool: PoolId) -> OperationId {
    let target = m
        .snapshot_for(pool)
        .and_then(|snapshot| snapshot.observe_resource(ResourceTarget::Pool(pool)).ok())
        .expect("an observed pool");
    let handle = m
        .connection_for(pool)
        .expect("a connection of the pool")
        .submit_resource(
            OperationId::new(),
            ResourceCommand::Pool {
                target,
                operation: PoolOperation::DisableHa,
            },
        )
        .expect("submitted");
    m.track_resource(handle)
}

fn row(m: &Model, id: OperationId) -> Option<&ResourceOperationRecord> {
    m.resource_rows.iter().find(|record| record.id == id)
}

fn handles(m: &Model) -> Vec<OperationId> {
    m.resource_operations
        .iter()
        .map(|operation| operation.latest().id)
        .collect()
}

#[test]
fn a_resource_handle_belongs_to_the_session_of_its_pool_and_stays_through_a_scope_change() {
    let Two {
        mut live,
        other: _other,
        other_pool,
        other_profile,
    } = two_pools();
    let (first, first_profile) = (live.pool, live.profile.id);
    let h = &mut live.h;
    // The second session is the scope session; the operation is the first's.
    let mine = submit(&mut h.model, first);
    let theirs = submit(&mut h.model, other_pool);
    assert_eq!(h.model.resource_session(mine), Some(first_profile));
    assert_eq!(h.model.resource_session(theirs), Some(other_profile));
    h.tick(0.02);
    assert_eq!(row(&h.model, mine).map(|record| record.pool), Some(first));
    assert_eq!(
        row(&h.model, theirs).map(|record| record.pool),
        Some(other_pool)
    );

    // The scope moves to the first pool: both handles and both records stay.
    h.select(Target::one(ObjectRef::pool(first)));
    assert_eq!(h.model.workspace.selected, Some(first_profile));
    assert_eq!(handles(&h.model), [mine, theirs]);
    h.tick(0.02);
    assert!(row(&h.model, mine).is_some() && row(&h.model, theirs).is_some());
    // The record of the pool that is no longer shown still reaches its end.
    poll_until(h, "the other pool's operation has ended", |h| {
        row(&h.model, theirs).is_some_and(|record| record.state.terminal())
    });
    assert_eq!(h.model.workspace.selected, Some(first_profile));
    // The control connection of a record is found whichever pool is shown.
    let control = session::resource_control_connection(
        &h.model,
        theirs,
        OperationControl::AcknowledgeUnknown,
    )
    .expect("the connection of the other pool");
    assert_eq!(
        control
            .latest()
            .inventory()
            .map(|inventory| inventory.pool_id),
        Some(other_pool)
    );
}

#[test]
fn resource_rows_list_the_history_of_every_connected_session() {
    let Two {
        mut live,
        other: _other,
        other_pool,
        ..
    } = two_pools();
    let first = live.pool;
    let h = &mut live.h;
    let mine = submit(&mut h.model, first);
    let theirs = submit(&mut h.model, other_pool);
    poll_until(
        h,
        "both sessions list their operation in their history",
        |h| {
            [first, other_pool]
                .iter()
                .zip([mine, theirs])
                .all(|(pool, id)| {
                    h.model.connection_for(*pool).is_some_and(|connection| {
                        connection
                            .resource_history()
                            .iter()
                            .any(|record| record.id == id)
                    })
                })
        },
    );
    // Without the handles and the rows (a record known only to a journal):
    // the scope session's history and the other session's both come back.
    h.model.resource_operations.clear();
    h.model.resource_rows.clear();
    h.tick(0.02);
    assert!(
        h.model.resource_sessions.is_empty(),
        "no entry without a handle"
    );
    assert_eq!(
        row(&h.model, theirs).map(|record| record.pool),
        Some(other_pool)
    );
    assert_eq!(
        row(&h.model, mine).map(|record| record.pool),
        Some(first),
        "the record of the session that is not the scope session"
    );
}

#[test]
fn a_sessions_resource_handles_go_with_its_connection_and_their_records_stay() {
    let Two {
        mut live,
        other: _other,
        other_pool,
        other_profile,
    } = two_pools();
    let (first, first_profile) = (live.pool, live.profile.id);
    let h = &mut live.h;
    let mine = submit(&mut h.model, first);
    let theirs = submit(&mut h.model, other_pool);
    assert_eq!(handles(&h.model), [mine, theirs]);

    // What `session::start_connection`, `session::remove_record` and
    // Reconnect As call: before any poll, so the record is merged here.
    h.model.forget_operations_of(first_profile);
    assert_eq!(handles(&h.model), [theirs]);
    assert_eq!(h.model.resource_session(mine), None);
    assert_eq!(h.model.resource_session(theirs), Some(other_profile));
    assert!(row(&h.model, mine).is_some(), "the record stays");

    // A session that left the workspace: its handles go at the next poll.
    let again = submit(&mut h.model, first);
    assert_eq!(h.model.resource_session(again), Some(first_profile));
    h.model.workspace.active.remove(&first_profile);
    h.tick(0.02);
    assert_eq!(handles(&h.model), [theirs]);
    assert!(row(&h.model, again).is_some() && row(&h.model, mine).is_some());

    // An area that drops finished handles itself leaves no entry behind.
    h.model.resource_operations.clear();
    h.model.forget_operations_of(first_profile);
    assert!(h.model.resource_sessions.is_empty());

    // The demo drops everything, as it always did.
    h.model.load_demo(1, 2);
    assert!(h.model.resource_operations.is_empty() && h.model.resource_rows.is_empty());
}

#[test]
fn reconnect_as_forgets_the_handles_of_every_session_of_the_pool_it_closes() {
    let mut live = Live::with_vm();
    let (pool, profile) = (live.pool, live.profile.clone());
    let h = &mut live.h;
    // A second saved connection to the same pool, over the same connection.
    let mut alias = profile.clone();
    alias.id = ProfileId::new();
    alias.name = "Alias".into();
    h.model
        .workspace
        .store
        .as_mut()
        .expect("a store")
        .save(alias.clone())
        .expect("saved");
    let shared = h.model.workspace.active[&profile.id].clone();
    h.model.workspace.active.insert(alias.id, shared);
    // One handle of each family belongs to the alias.
    let resource = submit(&mut h.model, pool);
    h.model.resource_sessions.insert(resource, alias.id);
    live.server.hold_tasks(true);
    let observed = h
        .model
        .snapshot_for(pool)
        .and_then(|snapshot| snapshot.observe_vm(live.server.vm()).ok())
        .expect("an observed VM");
    let vm = h
        .model
        .submit_tracked_command(xenadmin_core::VmCommand::Power {
            vm: observed,
            action: xenadmin_core::PowerAction::Start,
        })
        .expect("submitted");
    h.model.operation_sessions.insert(vm, alias.id);
    poll_until(h, "both records are listed", |h| {
        row(&h.model, resource).is_some() && h.model.operation_rows.iter().any(|r| r.id == vm)
    });

    h.invoke_on("pool.reconnect-as", &Target::one(ObjectRef::pool(pool)));
    h.type_text("reconnect-username", "root");
    h.type_text("reconnect-password", "synthetic-elevated");
    h.click("reconnect-confirm");
    assert!(h.model.areas.connections.reconnect.is_none());
    assert!(!h.model.workspace.active.contains_key(&alias.id));
    assert!(h.model.resource_operations.is_empty(), "before any poll");
    assert!(h.model.operations.is_empty());
    assert!(h.model.resource_sessions.is_empty() && h.model.operation_sessions.is_empty());
    assert!(row(&h.model, resource).is_some());
    assert!(h.model.operation_rows.iter().any(|r| r.id == vm));
    session::disconnect_all(&mut h.model);
}
