//! Operations and permissions per pool, against the fixture server
//! (`crate::testing`): two real sessions, each with a VM.
//!
//! What these pin: a VM operation belongs to the session it was submitted
//! through. Its handle stays in `m.operations` and its record in
//! `m.operation_rows`, and both keep changing, whichever pool the tree
//! shows; the per-pool forms ask the session of the pool for admission,
//! control and permission.

use xenadmin_core::{
    Error, OperationControl, OperationId, OperationState, PermissionDecision, PoolId, PowerAction,
    PowerState, VmCommand,
};

use super::*;
use crate::permissions;
use crate::testing::{Live, Server, connect, poll_until};

/// Two writable sessions, each over its own server with a VM. `live` is the
/// first; the second is the scope session when this returns.
struct Two {
    live: Live,
    other: Server,
    other_pool: PoolId,
    other_profile: ProfileId,
}

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

fn start(m: &Model, pool: PoolId, vm: xenadmin_core::VmId) -> VmCommand {
    let observed = m
        .snapshot_for(pool)
        .and_then(|snapshot| snapshot.observe_vm(vm).ok())
        .expect("an observed VM");
    VmCommand::Power {
        vm: observed,
        action: PowerAction::Start,
    }
}

fn state(m: &Model, id: OperationId) -> Option<OperationState> {
    m.operation_rows
        .iter()
        .find(|record| record.id == id)
        .map(|record| record.state.clone())
}

// Deliverable 1, the case the power area reported: submitted in the scope
// session, then the scope moves to another pool.
#[test]
fn a_record_of_the_previous_scope_session_stays_and_ends_while_another_pool_is_shown() {
    let Two {
        mut live,
        other,
        other_pool,
        other_profile,
    } = two_pools();
    let (first, first_profile) = (live.pool, live.profile.id);
    let h = &mut live.h;
    // The second session is the scope session: the scope form submits there.
    other.hold_tasks(true);
    let command = start(&h.model, other_pool, other.vm());
    let id = h
        .model
        .submit_tracked_command(command)
        .expect("submitted in the scope session");
    assert_eq!(h.model.operation_sessions.get(&id), Some(&other_profile));
    poll_until(h, "the start has a host task", |h| {
        other.count("task.get_record") > 0 && state(&h.model, id).is_some()
    });
    assert!(!state(&h.model, id).unwrap().terminal());

    // The scope moves to the first pool: the session changes.
    h.select(Target::one(ObjectRef::pool(first)));
    assert_eq!(h.model.workspace.selected, Some(first_profile));
    assert_eq!(h.model.inventory.as_ref().unwrap().pool_id, first);
    assert_eq!(h.model.operations.len(), 1, "the handle is kept");
    h.tick(0.02);
    let record = h
        .model
        .operation_rows
        .iter()
        .find(|record| record.id == id)
        .expect("the record is still listed");
    assert_eq!((record.pool, record.vm), (other_pool, other.vm()));
    assert!(!record.state.terminal());

    // Its outcome arrives while the other pool is shown.
    other.hold_tasks(false);
    poll_until(h, "the start of the other pool succeeded", |h| {
        state(&h.model, id) == Some(OperationState::Succeeded)
    });
    assert_eq!(h.model.workspace.selected, Some(first_profile));
    assert_eq!(other.count("Async.VM.start"), 1);
    assert_eq!(live.server.count("Async.VM.start"), 0);
    poll_until(h, "the other pool's VM is shown running", |h| {
        h.model
            .inventory_for(other_pool)
            .is_some_and(|i| i.vms[&other.vm()].power_state == PowerState::Running)
    });

    // The finished handle goes at the next admission; the record stays.
    assert_eq!(h.model.operations.len(), 1);
    h.model.admit_command().expect("the scope session admits");
    assert!(h.model.operations.is_empty() && h.model.operation_sessions.is_empty());
    h.tick(0.02);
    assert_eq!(state(&h.model, id), Some(OperationState::Succeeded));
}

// The per-pool forms: submitted through a session that is not the scope
// session, controlled there, and kept through scope changes.
#[test]
fn an_operation_of_another_pool_is_submitted_tracked_and_controlled_through_its_session() {
    let Two {
        mut live,
        other,
        other_pool,
        other_profile,
    } = two_pools();
    let (first, first_profile) = (live.pool, live.profile.id);
    let (server, h) = (&live.server, &mut live.h);
    assert_eq!(h.model.workspace.selected, Some(other_profile));

    // The reply of the first pool's start is lost: an unknown outcome.
    server.lose_replies(true);
    let command = start(&h.model, first, server.vm());
    assert!(h.model.admit_command_for(first).is_ok());
    let id = h
        .model
        .submit_tracked_command_for(first, command)
        .expect("submitted through the first session");
    assert_eq!(h.model.operation_sessions.get(&id), Some(&first_profile));
    assert_eq!(h.model.operations.len(), 1);
    poll_until(h, "the outcome is unknown", |h| {
        state(&h.model, id) == Some(OperationState::OutcomeUnknown)
    });
    assert_eq!(h.model.workspace.selected, Some(other_profile));

    // The scope goes to the first pool and back: nothing is lost.
    h.select(Target::one(ObjectRef::vm(first, server.vm())));
    assert_eq!(h.model.workspace.selected, Some(first_profile));
    h.tick(0.02);
    h.select(Target::one(ObjectRef::pool(other_pool)));
    assert_eq!(h.model.workspace.selected, Some(other_profile));
    h.tick(0.02);
    assert_eq!(state(&h.model, id), Some(OperationState::OutcomeUnknown));
    assert_eq!(h.model.operations.len(), 1);
    assert_eq!(h.model.operation_sessions.get(&id), Some(&first_profile));

    // The per-pool form reaches the session that has the operation, while
    // another session is the scope session.
    h.model.error = Some("an earlier error".into());
    h.model
        .control_operation_for(first, id, OperationControl::AcknowledgeUnknown);
    assert_eq!(h.model.error, None, "a success clears the error line");
    poll_until(h, "the unknown outcome is acknowledged", |h| {
        state(&h.model, id) == Some(OperationState::AcknowledgedUnknown)
    });
    assert_eq!(h.model.workspace.selected, Some(other_profile));
    assert_eq!(
        server.count("Async.VM.start"),
        1,
        "nothing replays the write"
    );
    assert_eq!(other.count("Async.VM.start"), 0);

    // A pool no session shows: nothing to control it through.
    let nowhere = PoolId::from(uuid::Uuid::new_v4());
    assert_eq!(
        h.model
            .try_control_operation_for(nowhere, id, OperationControl::Recheck)
            .map_err(|e| e.to_string()),
        Err(Error::Closed.to_string())
    );
    h.model
        .control_operation_for(nowhere, id, OperationControl::Recheck);
    assert_eq!(h.model.error, Some(Error::Closed.to_string()));
}

// A handle belongs to its session's connection: replaced or removed, the
// handle is forgotten and the record stays.
#[test]
fn replacing_a_sessions_connection_forgets_its_handles_and_keeps_their_records() {
    let Two {
        mut live,
        other,
        other_pool,
        other_profile,
    } = two_pools();
    let (first, first_profile) = (live.pool, live.profile.id);
    let (server, h) = (&live.server, &mut live.h);
    server.hold_tasks(true);
    other.hold_tasks(true);
    let command = start(&h.model, first, server.vm());
    let mine = h.model.submit_tracked_command_for(first, command).unwrap();
    let command = start(&h.model, other_pool, other.vm());
    let theirs = h
        .model
        .submit_tracked_command_for(other_pool, command)
        .unwrap();
    poll_until(h, "both records are listed", |h| {
        state(&h.model, mine).is_some() && state(&h.model, theirs).is_some()
    });
    assert_eq!(h.model.operations.len(), 2);

    // What `session::start_connection` and `session::remove_record` call.
    h.model.forget_operations_of(first_profile);
    assert_eq!(h.model.operations.len(), 1);
    assert_eq!(h.model.operations[0].latest().id, theirs);
    assert_eq!(
        h.model
            .operation_sessions
            .iter()
            .map(|(id, profile)| (*id, *profile))
            .collect::<Vec<_>>(),
        [(theirs, other_profile)]
    );
    h.tick(0.02);
    assert!(state(&h.model, mine).is_some(), "the record stays");

    // A session that left the workspace: its handles go at the next poll.
    h.model.workspace.active.remove(&first_profile);
    h.model.operation_sessions.insert(mine, first_profile);
    h.tick(0.02);
    assert_eq!(h.model.operation_sessions.len(), 1);
    assert!(state(&h.model, mine).is_some() && state(&h.model, theirs).is_some());

    // The demo drops everything, as it always did.
    h.model.load_demo(1, 2);
    assert!(h.model.operations.is_empty() && h.model.operation_sessions.is_empty());
}

// Admission asks the session of the pool: a synthetic pool is stale, a
// read-only session is refused by its own connection, and the scope form
// is the per-pool form of the scope session's pool.
#[test]
fn admission_and_refusals_are_those_of_the_pools_session() {
    // Synthetic pools: the scope session and another one.
    let mut m = Model::demo(2, 4);
    let demo = m.inventory.clone().unwrap().pool_id;
    let mut second = crate::demo::demonstration(1, 2);
    second.pool_id = PoolId::from(uuid::Uuid::new_v4());
    let second_pool = second.pool_id;
    m.add_synthetic_pool("Second", second);
    let stale = Err(Error::Stale.to_string());
    let text = |result: Result<(), Error>| result.map_err(|error| error.to_string());
    assert_eq!(text(m.admit_command_for(demo)), stale);
    assert_eq!(text(m.admit_command_for(second_pool)), stale);
    assert_eq!(
        permissions::power_for(&m, second_pool, PowerAction::Start),
        None
    );
    // A pool no session shows falls to the scope session.
    let nowhere = PoolId::from(uuid::Uuid::new_v4());
    assert_eq!(text(m.admit_command_for(nowhere)), stale);
    m.demo = false;
    assert_eq!(
        text(m.admit_command_for(nowhere)),
        Err(Error::Closed.to_string())
    );

    // A read-only session admits (the model does not know better) and its
    // connection refuses the command and the control.
    let pool = PoolId::from(uuid::Uuid::new_v4());
    let mut live = Live::over(Server::with_vm(pool), pool, true);
    let h = &mut live.h;
    assert!(h.model.admit_command_for(pool).is_ok());
    let command = start(&h.model, pool, live.server.vm());
    assert_eq!(
        h.model
            .submit_tracked_command_for(pool, command)
            .map_err(|e| e.to_string()),
        Err(Error::ReadOnly.to_string())
    );
    assert!(h.model.operations.is_empty() && h.model.operation_sessions.is_empty());
    let command = start(&h.model, pool, live.server.vm());
    h.model.submit_command_for(pool, command);
    assert_eq!(h.model.error, Some(Error::ReadOnly.to_string()));
    assert_eq!(
        h.model
            .try_control_operation_for(pool, OperationId::new(), OperationControl::Recheck)
            .map_err(|e| e.to_string()),
        Err(Error::ReadOnly.to_string())
    );
    assert_eq!(live.server.count("Async.VM.start"), 0);
}

// Deliverable 2: the permission of the pool's own session, which may differ
// from the scope session's.
#[test]
fn the_permission_asked_is_that_of_the_pools_session() {
    let mut live = Live::with_vm();
    let first = live.pool;
    let denied_pool = PoolId::from(uuid::Uuid::new_v4());
    let denied = Server::with_vm(denied_pool);
    denied.session(serde_json::json!({
        "is_local_superuser": false,
        "rbac_permissions": ["session.logout", "task.get_record", "task.destroy"],
    }));
    connect(&mut live.h.model, &denied, denied_pool, "Operator", false);
    let h = &mut live.h;
    poll_until(h, "both pools are current", |h| {
        h.model.can_mutate_pool(first) && h.model.can_mutate_pool(denied_pool)
    });
    let requirements =
        xenadmin_core::OperationAction::Power(PowerAction::Start).permission_requirements();

    // The denied session is the scope session.
    let m = &mut h.model;
    assert_eq!(
        permissions::power(m, PowerAction::Start),
        Some(PermissionDecision::Denied)
    );
    assert_eq!(
        permissions::power_for(m, denied_pool, PowerAction::Start),
        Some(PermissionDecision::Denied)
    );
    assert_eq!(
        permissions::power_for(m, first, PowerAction::Start),
        Some(PermissionDecision::Granted)
    );
    assert!(permissions::check_for(m, first, &requirements));
    assert_eq!(m.error, None);
    assert!(!permissions::check_for(m, denied_pool, &requirements));
    let explanation = permissions::assess_for(m, denied_pool, &requirements).explanation();
    assert_eq!(m.error, Some(explanation));
    m.error = None;
    assert!(!permissions::check(m, &requirements));

    // The scope moves to the granted pool: the per-pool answers stay.
    h.select(Target::one(ObjectRef::pool(first)));
    h.tick(0.02);
    let m = &mut h.model;
    m.error = None;
    assert!(permissions::check(m, &requirements));
    assert_eq!(
        permissions::power(m, PowerAction::Start),
        Some(PermissionDecision::Granted)
    );
    assert_eq!(
        permissions::power_for(m, denied_pool, PowerAction::Start),
        Some(PermissionDecision::Denied)
    );
    assert!(!permissions::check_for(m, denied_pool, &requirements));
    assert!(m.error.is_some());
    // A pool no session shows: unknown, which never blocks.
    let nowhere = PoolId::from(uuid::Uuid::new_v4());
    m.error = None;
    assert_eq!(permissions::power_for(m, nowhere, PowerAction::Start), None);
    assert!(permissions::check_for(m, nowhere, &requirements));
    assert_eq!(m.error, None);
}
