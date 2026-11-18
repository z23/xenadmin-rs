//! Recheck and Acknowledge through a real connection: against the fixture
//! server with a VM (`crate::testing::Live`), where a start can be held at
//! its host task or lose its reply. The tests of `tests.rs` reach these
//! commands only up to their refusals, on synthetic records.

use xenadmin_core::{
    OperationId, OperationRecord, OperationState, PowerAction, PowerState, VmCommand,
};

use super::control::{self, RECHECKING};
use super::hidden::{Family, HiddenKey};
use crate::prelude::*;
use crate::testing::{Live, poll_until};

/// Submits a start of the server's VM through the session of its pool.
fn start(live: &mut Live) -> HiddenKey {
    let (pool, vm) = (live.pool, live.server.vm());
    let m = &mut live.h.model;
    let observed = m
        .snapshot_for(pool)
        .and_then(|snapshot| snapshot.observe_vm(vm).ok())
        .expect("an observed VM");
    let id = m
        .submit_tracked_command_for(
            pool,
            VmCommand::Power {
                vm: observed,
                action: PowerAction::Start,
            },
        )
        .expect("the start is submitted");
    HiddenKey {
        pool,
        id,
        family: Family::Vm,
    }
}

fn record(m: &Model, id: OperationId) -> Option<&OperationRecord> {
    m.operation_rows.iter().find(|record| record.id == id)
}

fn state(m: &Model, id: OperationId) -> Option<OperationState> {
    record(m, id).map(|record| record.state.clone())
}

#[test]
fn a_record_with_a_host_task_is_rechecked_over_its_connection_and_nothing_is_replayed() {
    let mut live = Live::with_vm();
    live.server.hold_tasks(true);
    let key = start(&mut live);
    poll_until(&mut live.h, "the start waits on its host task", |h| {
        live.server.count("task.get_record") > 0
            && record(&h.model, key.id).is_some_and(|r| !r.state.terminal() && r.can_recheck())
    });

    // The record on the Events tab, selected.
    super::show_operation(&mut live.h.model, key);
    assert_eq!(live.h.model.active_tab(), TabId::Events);
    assert_eq!(live.h.eligibility("events.recheck"), Eligibility::Enabled);
    assert_eq!(live.h.eligibility("events.acknowledge"), Eligibility::Hidden);
    let reads = live.server.count("task.get_record");
    live.h.click("events.recheck");
    assert_eq!(live.h.model.areas.events.notice.as_deref(), Some(RECHECKING));
    assert_eq!(
        live.h.node("activity-notice").text.as_deref(),
        Some(RECHECKING)
    );
    // The host task is read again (by the recheck or by the core's own
    // tracking: the server cannot tell which); the write is not sent again
    // and the connection reports no refusal.
    poll_until(&mut live.h, "the host task is read again", |_| {
        live.server.count("task.get_record") > reads
    });
    assert_eq!(live.server.count("Async.VM.start"), 1);
    assert!(!state(&live.h.model, key.id).unwrap().terminal());
    assert_eq!(live.h.model.snapshot.as_ref().unwrap().error(), None);

    // The task ends: so does the record, and there is nothing to recheck.
    live.server.hold_tasks(false);
    poll_until(&mut live.h, "the start succeeded", |h| {
        state(&h.model, key.id) == Some(OperationState::Succeeded)
    });
    assert_eq!(live.h.eligibility("events.recheck"), Eligibility::Hidden);
    assert_eq!(live.server.count("Async.VM.start"), 1);
}

#[test]
fn an_unknown_outcome_is_acknowledged_over_its_connection_and_releases_the_vm() {
    let mut live = Live::with_vm();
    let (pool, vm) = (live.pool, live.server.vm());
    live.server.lose_replies(true);
    let key = start(&mut live);
    poll_until(&mut live.h, "the outcome is unknown", |h| {
        state(&h.model, key.id) == Some(OperationState::OutcomeUnknown)
    });
    assert_eq!(live.server.count("Async.VM.start"), 1);

    // While it is unknown the client keeps the VM: a second start is
    // refused by the core without reaching the host.
    live.server.lose_replies(false);
    let second = start(&mut live);
    poll_until(&mut live.h, "the second start is rejected", |h| {
        state(&h.model, second.id) == Some(OperationState::Rejected)
    });
    assert_eq!(live.server.count("Async.VM.start"), 1);

    // The review of the acknowledgement, opened from the selected record.
    super::show_operation(&mut live.h.model, key);
    assert_eq!(
        live.h.eligibility("events.acknowledge"),
        Eligibility::Enabled
    );
    live.h.click("events.acknowledge");
    let surface = live.h.surface();
    assert_eq!(
        (surface.id.as_str(), surface.title.as_str()),
        ("unknown-outcome", "Acknowledge unknown outcome")
    );
    assert!(live.h.has_text("VM: Fixture guest"));
    assert!(live.h.has_text(control::ACK_VM));
    assert_eq!(live.h.node("unknown-acknowledge").disabled, None);
    let open = live.h.model.areas.events.acknowledge.as_ref().unwrap();
    assert_eq!(control::acknowledge_stale(&live.h.model, open), None);

    // Confirmed: the connection takes it, the dialog closes, the record
    // says the outcome was acknowledged, never that it is known.
    live.h.click("unknown-acknowledge");
    assert!(live.h.model.areas.events.acknowledge.is_none());
    assert!(live.h.surfaces().is_empty());
    poll_until(&mut live.h, "the unknown outcome is acknowledged", |h| {
        state(&h.model, key.id) == Some(OperationState::AcknowledgedUnknown)
    });
    assert_eq!(
        live.h.eligibility("events.acknowledge"),
        Eligibility::Hidden
    );
    assert_eq!(live.h.eligibility("events.recheck"), Eligibility::Hidden);
    assert_eq!(live.server.count("Async.VM.start"), 1);

    // The VM is released: a start is sent to the host again and succeeds.
    let third = start(&mut live);
    poll_until(&mut live.h, "the third start succeeded", |h| {
        state(&h.model, third.id) == Some(OperationState::Succeeded)
    });
    assert_eq!(live.server.count("Async.VM.start"), 2);
    poll_until(&mut live.h, "the VM is shown running", |h| {
        h.model
            .inventory_for(pool)
            .is_some_and(|i| i.vms[&vm].power_state == PowerState::Running)
    });
}

// A start whose reply was lost has no host task to read: the core cannot
// recheck it (`OperationRecord::can_recheck`). The command is offered as in
// Rune (any unfinished record of a writable pool); the connection takes the
// request and the core's refusal arrives in the connection's error line.
// Nothing is replayed and the record stays unknown.
#[test]
fn a_recheck_of_an_unknown_outcome_without_a_host_task_changes_nothing() {
    let mut live = Live::with_vm();
    live.server.lose_replies(true);
    let key = start(&mut live);
    poll_until(&mut live.h, "the outcome is unknown", |h| {
        state(&h.model, key.id) == Some(OperationState::OutcomeUnknown)
    });
    assert!(!record(&live.h.model, key.id).unwrap().can_recheck());
    super::show_operation(&mut live.h.model, key);
    assert_eq!(live.h.eligibility("events.recheck"), Eligibility::Enabled);
    live.h.click("events.recheck");
    assert_eq!(live.h.model.areas.events.notice.as_deref(), Some(RECHECKING));
    poll_until(&mut live.h, "the core answers the request", |h| {
        h.model
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.error().is_some())
    });
    let snapshot = live.h.model.snapshot.clone().unwrap();
    assert_eq!(
        snapshot.error(),
        Some(xenadmin_core::Error::UnavailableAction.to_string().as_str())
    );
    assert!(snapshot.is_current(), "the session stays usable");
    assert_eq!(
        state(&live.h.model, key.id),
        Some(OperationState::OutcomeUnknown)
    );
    assert_eq!(live.server.count("Async.VM.start"), 1);
    assert_eq!(live.server.count("task.get_record"), 0);
}
