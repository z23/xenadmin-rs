//! The tests the area could not have without an observed VM: against the
//! fixture server with a VM (`crate::testing::Live`), a real `Backend` and
//! a writable, current session. The confirmation and the placement dialog
//! open, and a confirmed Start is submitted, tracked and succeeds.
//!
//! The server serves one start of its VM (`Async.VM.start` and its task).
//! It does not serve a start on a chosen server, a recovery, a batch or a
//! forced action: the placement dialog is tested up to its review, and
//! nothing here confirms it.

use xenadmin_core::{
    OperationAction, OperationState, PermissionDecision, PoolId, PowerAction, PowerState,
};

use super::commands::{START, START_ON_SERVER};
use super::{placement, single};
use crate::prelude::*;
use crate::surface::EXPERIMENTAL_NOTICE_KEY;
use crate::testing::{Live, Server, poll_until};

fn text(live: &Live, key: &str) -> String {
    live.h.node(key).text.clone().unwrap_or_default()
}

#[test]
fn a_confirmed_start_of_an_observed_vm_is_submitted_once_tracked_and_succeeds() {
    let mut live = Live::with_vm();
    let (pool, vm) = (live.pool, live.server.vm());
    live.h.model.preferences.options.review_power = true;
    live.h.select(Target::one(live.vm()));
    assert_eq!(live.h.eligibility(START.0), Eligibility::Enabled);
    live.h.invoke(START.0);

    // The confirmation, with its lines.
    let surface = live.h.surface();
    assert_eq!(
        (surface.id.as_str(), surface.title.as_str()),
        ("power-confirm", "Confirm power operation")
    );
    let observed = live
        .h
        .model
        .snapshot_for(pool)
        .and_then(|snapshot| snapshot.observe_vm(vm).ok())
        .expect("an observed VM");
    assert_eq!(
        text(&live, "power-confirm-target"),
        format!("Target: Fixture guest\nUUID: {observed}")
    );
    assert_eq!(text(&live, "power-confirm-consequence"), single::ROUTINE);
    assert!(live.h.has_text("Start this VM?"));
    let submit = live.h.node("power-confirm-submit");
    assert_eq!((submit.label.as_deref(), &submit.disabled), (Some("Start"), &None));
    let confirm = live.h.model.areas.power.confirm.as_ref().unwrap();
    assert_eq!(
        (confirm.pool, confirm.vm, confirm.target, confirm.action),
        (pool, vm, observed, PowerAction::Start)
    );
    // Nothing is submitted by opening it.
    assert!(live.h.model.operations.is_empty());
    assert_eq!(live.server.count("Async.VM.start"), 0);

    // Confirmed: submitted once, tracked, the dialog closes.
    live.h.click("power-confirm-submit");
    assert!(live.h.model.areas.power.confirm.is_none());
    assert!(live.h.surfaces().is_empty());
    assert_eq!(live.h.model.error, None);
    assert_eq!(live.h.model.operations.len(), 1);
    let id = live.h.model.operations[0].latest().id;
    assert_eq!(
        live.h.model.operation_sessions.get(&id),
        Some(&live.profile.id)
    );
    poll_until(&mut live.h, "the start succeeded", |h| {
        h.model
            .operation_rows
            .iter()
            .any(|r| r.id == id && r.state == OperationState::Succeeded)
    });
    let rows = &live.h.model.operation_rows;
    assert_eq!(rows.len(), 1, "one record of one submission");
    assert_eq!((rows[0].pool, rows[0].vm), (pool, vm));
    assert!(matches!(
        rows[0].action,
        OperationAction::Power(PowerAction::Start)
    ));
    assert_eq!(live.server.count("Async.VM.start"), 1);

    // The VM runs: Start no longer applies to it.
    poll_until(&mut live.h, "the VM is shown running", |h| {
        h.model
            .inventory_for(pool)
            .is_some_and(|i| i.vms[&vm].power_state == PowerState::Running)
    });
    assert!(matches!(
        live.h.eligibility(START.0),
        Eligibility::Disabled(_)
    ));
    assert_eq!(live.server.count("Async.VM.start"), 1);
}

#[test]
fn a_cancelled_confirmation_submits_nothing_and_a_start_without_review_submits_at_once() {
    let mut live = Live::with_vm();
    live.h.model.preferences.options.review_power = true;
    live.h.select(Target::one(live.vm()));
    live.h.invoke(START.0);
    assert_eq!(live.h.surface().id, "power-confirm");
    live.h.click("power-confirm-cancel");
    assert!(live.h.surfaces().is_empty());
    live.h.tick(0.05);
    assert!(live.h.model.operations.is_empty() && live.h.model.operation_rows.is_empty());
    assert_eq!(live.server.count("Async.VM.start"), 0);

    // Without the review preference an ordinary action is submitted when
    // it is asked for (the Rune `request_power`).
    live.h.model.preferences.options.review_power = false;
    live.h.invoke(START.0);
    assert!(live.h.surfaces().is_empty());
    assert_eq!(live.h.model.error, None);
    assert_eq!(live.h.model.operations.len(), 1);
    poll_until(&mut live.h, "the start succeeded", |h| {
        h.model
            .operation_rows
            .iter()
            .any(|r| r.state == OperationState::Succeeded)
    });
    assert_eq!(live.server.count("Async.VM.start"), 1);
}

#[test]
fn a_session_that_may_not_start_a_vm_has_the_command_disabled_with_the_denial() {
    let pool = PoolId::from(uuid::Uuid::new_v4());
    let server = Server::with_vm(pool);
    server.session(serde_json::json!({
        "is_local_superuser": false,
        "rbac_permissions": ["session.logout", "task.get_record", "task.destroy"],
    }));
    let mut live = Live::over(server, pool, false);
    live.h.select(Target::one(live.vm()));
    assert_eq!(
        live.h.eligibility(START.0),
        Eligibility::disabled(PermissionDecision::Denied.explanation())
    );
    // The request refuses with the same words and opens nothing.
    let vm = live.vm();
    single::request(&mut live.h.model, &vm, PowerAction::Start);
    assert_eq!(
        live.h.model.error.as_deref(),
        Some(PermissionDecision::Denied.explanation())
    );
    assert!(live.h.model.areas.power.confirm.is_none());
    assert!(live.h.model.operations.is_empty());
    assert_eq!(live.server.count("Async.VM.start"), 0);
}

#[test]
fn the_placement_dialog_opens_for_an_observed_vm_and_lists_the_hosts() {
    let mut live = Live::with_vm();
    let (pool, vm, host) = (live.pool, live.server.vm(), live.server.host());
    live.h.select(Target::one(live.vm()));
    assert_eq!(live.h.eligibility(START_ON_SERVER.0), Eligibility::Enabled);
    live.h.invoke(START_ON_SERVER.0);
    assert_eq!(live.h.model.error, None);

    let surface = live.h.surface();
    assert_eq!(
        (surface.id.as_str(), surface.title.as_str()),
        ("vm-placement", "Choose a destination host")
    );
    assert!(surface.experimental);
    assert!(live.h.find(EXPERIMENTAL_NOTICE_KEY).is_some());
    assert!(live.h.has_text("Start on a selected host"));
    assert_eq!(
        text(&live, "placement-target"),
        format!("Fixture guest\nVM UUID: {vm}")
    );
    let captured = live.h.model.areas.power.placement.as_ref().unwrap();
    assert_eq!(
        (captured.pool, captured.vm, captured.action),
        (pool, vm, PowerAction::Start)
    );
    assert_eq!(placement::stale(&live.h.model, captured), None);

    // The servers of the pool, each an option; no review, no confirm yet.
    let option = live.h.node(&format!("placement-host-{host}"));
    assert_eq!((option.label.as_deref(), &option.disabled), (Some("Fixture host"), &None));
    assert!(live.h.find("placement-review").is_none());
    assert!(live.h.find("placement-confirm").is_none());

    // Choosing one turns the dialog to its review.
    live.h.pick("placement-hosts", &host.to_string());
    assert!(live.h.find("placement-hosts").is_none());
    assert!(live.h.has_text("Destination: Fixture host"));
    assert!(live.h.has_text(&format!("Host UUID: {host}")));
    assert!(live.h.has_text(placement::PREREQUISITES));
    let confirm = live.h.node("placement-confirm");
    assert_eq!((confirm.label.as_deref(), &confirm.disabled), (Some("Start"), &None));
    assert!(live.h.find(permissions::REVIEW_KEY).is_some());

    // "Choose another host" is the way back; Cancel closes.
    live.h.click("placement-back");
    assert!(live.h.find("placement-hosts").is_some());
    live.h.click("placement-cancel");
    assert!(live.h.surfaces().is_empty());
    assert!(live.h.model.operations.is_empty());
    assert_eq!(live.server.count("Async.VM.start"), 0);
    assert_eq!(live.server.count("Async.VM.start_on"), 0);
}
