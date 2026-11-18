//! The tests of the area, in three parts.
//!
//! 1. **The Rune tests**: the three of `snapshot_policy_view.rs` and the
//!    three of `alarms_view.rs`, under their names (without "at both
//!    widths": a description has no width, and the compact-size half is in
//!    `gpui-tests-wanted.md`), and this area's share of two tests of other
//!    Rune files that ran over its scenes. Where a Rune test pressed a key
//!    the test here presses it, through the steps the wizard now has.
//! 2. **What the port changed**: the commands, the dialog with its list, the
//!    wizard, the two Properties pages, the refresh dialog.
//! 3. **The hazards** the maps list for the two modules (52 to 56, 58 to
//!    63), where a test of the first two parts does not already hold them.
//!
//! Not tested, because no test can have it: a submission that succeeds (the
//! fixture server serves neither `VMSS` nor the alarm calls), and the limit
//! of 64 unfinished resource operations of hazard 61 (a handle cannot be
//! made without a submission).

use std::collections::BTreeMap;
use std::sync::Arc;

use xenadmin_core::{
    AlarmEmailSettings, AlarmMetric, AlarmOperation, AlarmPolicy, AlarmRule, Error, Inventory,
    PoolId, ResourceTarget, ScheduledSnapshotKind, SnapshotPolicy, SnapshotPolicyClock,
    SnapshotPolicyConfig, SnapshotPolicyId, SnapshotPolicyOperation as Op, SnapshotPolicySupport,
    SnapshotSchedule, VmId,
};

use super::alarms::{self, ALERTS, EMAIL_OPTIONS, Page};
use super::alarms_view::{self, REFRESH_DIALOG};
use super::commands::NO_POOL;
use super::scenes;
use super::schedules::{self, LAST_DRAFT_STEP, Mode, STEPS};
use super::schedules_view::{DIALOG, WIZARD};
use crate::Harness;
use crate::prelude::*;
use crate::testing::{Live, Server};

// -- Fixtures -------------------------------------------------------------------

const DEMO: &str = "Demo mode cannot change infrastructure.";
const CLOCK: &str =
    "Refresh host support and local clock before configuring or enabling a schedule";
const HOST_PAGES: &str = "host-properties-pages";

fn workspace(h: &Harness) -> &schedules::Workspace {
    let open = h.model.areas.schedules_alarms.schedules.as_ref();
    open.expect("the snapshot schedules are open")
}

fn editor(h: &Harness) -> &alarms::Workspace {
    let open = h.model.areas.schedules_alarms.alarms.as_ref();
    open.expect("an alarm editor is open")
}

fn draft(h: &mut Harness) -> &mut schedules::Workspace {
    let open = h.model.areas.schedules_alarms.schedules.as_mut();
    open.expect("the snapshot schedules are open")
}

fn alarm_draft(h: &mut Harness) -> &mut alarms::Workspace {
    let open = h.model.areas.schedules_alarms.alarms.as_mut();
    open.expect("an alarm editor is open")
}

/// Replaces the inventory of the scope session with an edited copy.
fn amend(h: &mut Harness, change: impl FnOnce(&mut Inventory)) {
    let mut inventory = h.model.inventory.take().expect("inventory").as_ref().clone();
    change(&mut inventory);
    h.model.inventory = Some(Arc::new(inventory));
}

fn pool(h: &Harness) -> PoolId {
    h.model.inventory.as_ref().expect("inventory").pool_id
}

fn pool_target(h: &Harness) -> Target {
    Target::one(ObjectRef::pool(pool(h)))
}

fn error(h: &Harness) -> Option<String> {
    h.find("policy-error").and_then(|node| node.text)
}

/// The step of the wizard that shows.
fn step(h: &Harness) -> usize {
    match h.surface().kind {
        SurfaceKind::Wizard { steps, current } => {
            assert!(steps.iter().map(String::as_str).eq(STEPS));
            current
        }
        _ => panic!("the surface is no wizard"),
    }
}

/// The answer of the coordinator a synthetic pool cannot ask for.
fn answer(h: &mut Harness) -> SnapshotPolicyClock {
    let w = h.model.areas.schedules_alarms.schedules.as_mut().unwrap();
    let clock = SnapshotPolicyClock {
        coordinator: *w.initial.hosts.keys().next().expect("a server"),
        utc_offset_minutes: 600,
    };
    w.query = None;
    w.support = Some(Ok(SnapshotPolicySupport {
        clock,
        coordinator_name: "Example coordinator".into(),
        local_time: "20261108T10:00:00".into(),
        utc_time: "20261108T00:00:00Z".into(),
    }));
    clock
}

fn policy(id: u128, name: &str, enabled: Option<bool>, vms: Vec<VmId>) -> SnapshotPolicy {
    SnapshotPolicy {
        vms_known: true,
        last_run_time: None,
        id: SnapshotPolicyId::from(uuid::Uuid::from_u128(id)),
        name: name.into(),
        description: String::new(),
        enabled,
        kind: "snapshot".into(),
        frequency: "daily".into(),
        retained_snapshots: Some(3),
        schedule: BTreeMap::from([("hour".into(), "2".into()), ("min".into(), "0".into())]),
        vms,
    }
}

/// The demo pool with three schedules and the three guests `a`, `b`, `c`:
/// "Nightly" is disabled and empty, "Weekend" is enabled with `a`, "Staged"
/// is disabled with `c`. `b` is in no schedule.
struct Schedules {
    h: Harness,
    nightly: SnapshotPolicyId,
    weekend: SnapshotPolicyId,
    staged: SnapshotPolicyId,
    a: VmId,
    b: VmId,
    c: VmId,
}

impl Schedules {
    fn new() -> Self {
        let mut h = Harness::new();
        h.model.load_demo(3, 48);
        let guests: Vec<VmId> = (h.model.inventory.as_ref().unwrap().vms.values())
            .filter(|vm| !vm.is_template && !vm.is_control_domain)
            .map(|vm| vm.id)
            .collect();
        let (a, b, c) = (guests[0], guests[1], guests[2]);
        let nightly = policy(0x5001, "Nightly", Some(false), vec![]);
        let weekend = policy(0x5002, "Weekend", Some(true), vec![a]);
        let staged = policy(0x5003, "Staged", Some(false), vec![c]);
        let ids = (nightly.id, weekend.id, staged.id);
        amend(&mut h, |i| {
            i.details.vms.entry(a).or_default().snapshot_policy = Some(weekend.id);
            i.details.vms.entry(c).or_default().snapshot_policy = Some(staged.id);
            for policy in [nightly, weekend, staged] {
                i.details.snapshot_policies.insert(policy.id, policy);
            }
        });
        let target = pool_target(&h);
        h.select(target);
        Self {
            h,
            nightly: ids.0,
            weekend: ids.1,
            staged: ids.2,
            a,
            b,
            c,
        }
    }

    /// The same, with the dialog open.
    fn open() -> Self {
        let mut s = Self::new();
        s.h.invoke("pool.snapshot-schedules");
        assert_eq!(s.h.surface().id, DIALOG);
        s
    }

    fn choose(&mut self, policy: SnapshotPolicyId) {
        self.h.click(&format!("policy-select-{policy}"));
        assert_eq!(workspace(&self.h).selected, Some(policy));
    }

    fn name(&self, vm: VmId) -> String {
        workspace(&self.h).initial.vms[&vm].name.clone()
    }

    fn vm(&self, vm: VmId) -> Target {
        Target::one(ObjectRef::vm(pool(&self.h), vm))
    }
}

/// The scene `alarms` without its dialog: the pool and the server that has
/// the alarm policy.
fn alarm_pool(email: bool) -> (Harness, Target, Target) {
    let mut h = Harness::new();
    let (pool, host) = scenes::prepare(&mut h.model, email).expect("the demo pool");
    h.select(host.clone());
    (h, pool, host)
}

fn pages(h: &Harness, target: &Target) -> Vec<&'static str> {
    (h.model.property_pages(target).into_iter())
        .filter(|(area, _)| *area == "schedules_alarms")
        .map(|(_, page)| page.id)
        .collect()
}

// -- 1. The Rune tests -------------------------------------------------------------

/// Rune `schedule_review_captures_coordinator_and_pool_at_both_widths`.
#[test]
fn schedule_review_captures_coordinator_and_pool() {
    let mut h = Harness::scene("snapshot-schedules");
    h.click("policy-next");
    h.click("policy-next");
    h.click("policy-review");
    let op = workspace(&h).review.clone().expect("a review");
    let coordinator = *workspace(&h).initial.hosts.keys().next().unwrap();
    assert!(matches!(&op, Op::Create { clock, .. } if clock.coordinator == coordinator));
    let captured = workspace(&h).pool();
    amend(&mut h, |i| i.pool_id = PoolId::from(uuid::Uuid::new_v4()));
    assert_eq!(
        h.try_click("policy-confirm"),
        Err(schedules::STALE.to_owned())
    );
    schedules::submit(&mut h.model);
    assert!(h.model.resource_operations.is_empty());
    assert_eq!(workspace(&h).error, Some(Error::Stale.to_string()));
    assert_eq!(workspace(&h).pool(), captured);
    assert_eq!(workspace(&h).review, Some(op));
}

/// Rune `invalid_minutes_retention_and_unknown_clock_block_review`.
#[test]
fn invalid_minutes_retention_and_unknown_clock_block_review() {
    let mut h = Harness::scene("snapshot-schedules");
    for value in ["1", "16", "60", "-1"] {
        draft(&mut h).minute = value.into();
        schedules::review(&mut h.model);
        assert!(workspace(&h).review.is_none(), "minute {value}");
    }
    draft(&mut h).minute = "15".into();
    draft(&mut h).retention = "11".into();
    schedules::review(&mut h.model);
    assert!(workspace(&h).review.is_none());
    draft(&mut h).retention = "3".into();
    draft(&mut h).support = None;
    schedules::review(&mut h.model);
    assert!(workspace(&h).review.is_none());
    assert_eq!(workspace(&h).error.as_deref(), Some(CLOCK));
}

/// Rune `weekly_day_selection_and_checkpoint_are_reviewed`.
#[test]
fn weekly_day_selection_and_checkpoint_are_reviewed() {
    let mut h = Harness::scene("snapshot-schedules");
    h.click("policy-next");
    h.click("policy-frequency-2");
    h.click("policy-day-1");
    h.click("policy-next");
    h.click("policy-checkpoint");
    h.click("policy-review");
    assert!(matches!(
        &workspace(&h).review,
        Some(Op::Create {
            config: SnapshotPolicyConfig {
                kind: ScheduledSnapshotKind::Checkpoint,
                schedule: SnapshotSchedule::Weekly { days, .. },
                ..
            },
            ..
        }) if days == &vec![1]
    ));
}

/// Rune `alarm_review_converts_percent_and_retains_original_pool_at_both_widths`.
#[test]
fn alarm_review_converts_percent_and_retains_original_pool() {
    let mut h = Harness::scene("alarms");
    h.click("alarm-review");
    let pool = editor(&h).initial.pool_id;
    let review = editor(&h).review.clone().expect("a review");
    assert!(matches!(
        &review,
        AlarmOperation::SetPolicy(change) if change.rule.as_ref().unwrap().threshold == "0.8"
    ));
    amend(&mut h, |i| i.pool_id = PoolId::from(uuid::Uuid::new_v4()));
    // Still open: nothing has ticked since the pool changed under it.
    assert_eq!(h.try_click("alarm-confirm"), Err(alarms::STALE.to_owned()));
    alarms::submit(&mut h.model);
    assert!(h.model.resource_operations.is_empty());
    assert_eq!(editor(&h).error, Some(Error::Stale.to_string()));
    assert_eq!(editor(&h).initial.pool_id, pool);
    assert_eq!(editor(&h).review, Some(review));
}

/// Rune `invalid_threshold_and_custom_source_refuse_review_but_explicit_disable_is_captured`.
#[test]
fn invalid_threshold_and_custom_source_refuse_review_but_explicit_disable_is_captured() {
    let mut h = Harness::scene("alarms");
    alarm_draft(&mut h).threshold = "101".into();
    alarms::review(&mut h.model);
    assert!(editor(&h).review.is_none());
    alarm_draft(&mut h).threshold = "70".into();
    let policy = alarm_draft(&mut h).policy.as_mut().unwrap();
    policy.customized.push(AlarmMetric::Cpu);
    alarms::review(&mut h.model);
    assert!(editor(&h).review.is_none());
    assert!(h.find("alarm-custom").is_some());
    h.click("alarm-enabled");
    assert!(!editor(&h).enabled);
    h.click("alarm-review");
    assert!(matches!(
        &editor(&h).review,
        Some(AlarmOperation::SetPolicy(change)) if change.rule.is_none()
    ));
    assert!(h.has_text("Disable CPU usage"));
}

/// Rune `alarm_email_and_refresh_reviews_explain_configuration_and_delivery_separately`.
#[test]
fn alarm_email_and_refresh_reviews_explain_configuration_and_delivery_separately() {
    let mut h = Harness::scene("alarms");
    let w = h.model.areas.schedules_alarms.alarms.as_mut().unwrap();
    w.page = Page::Refresh;
    let op = alarms::intent(w).unwrap();
    let lines = alarms::review_lines(w, &op).join(" ");
    assert!(lines.contains("does not confirm alarm evaluation or email delivery"));
    let op = AlarmOperation::SetEmail {
        expected_revision: "0".repeat(64),
        settings: AlarmEmailSettings {
            destination: String::new(),
            relay: String::new(),
            language: "en-US".into(),
        },
    };
    let lines = alarms::review_lines(w, &op).join(" ");
    assert!(lines.contains("Disable pool alarm email"));
    assert!(lines.contains("does not send a test message"));
}

/// This area's share of Rune `main.rs`
/// `dialogs_outside_the_supported_core_carry_the_experimental_notice`: both
/// Rune dialogs were flagged, so every surface of the area is.
#[test]
fn every_surface_of_the_area_carries_the_experimental_notice() {
    let scenes = [
        "snapshot-schedules",
        "snapshot-schedule-review",
        "alarms",
        "alarms-review",
        "alarms-email",
    ];
    for scene in scenes {
        let h = Harness::scene(scene);
        assert!(h.surface().experimental, "{scene}");
        assert!(h.model.experimental_notice(), "{scene}");
        assert!(h.find(EXPERIMENTAL_NOTICE_KEY).is_some(), "{scene}");
        h.assert_unique_keys();
    }
    let mut s = Schedules::open();
    assert!(s.h.surface().experimental, "the list");
    s.h.dismiss();
    s.h.invoke("tools.alarm-service-refresh");
    assert!(s.h.surface().experimental, "the refresh");
}

/// This area's share of Rune `session_permissions_view.rs`
/// `review_permission_notice_is_reachable_without_bypassing_synthetic_target_guard`
/// (`snapshot_policy_view::review_scene` was one of its scenes).
#[test]
fn review_permission_notice_is_reachable_without_bypassing_synthetic_target_guard() {
    let mut h = Harness::scene("snapshot-schedule-review");
    assert!(
        h.find(permissions::REVIEW_KEY).is_some(),
        "captured review must explain permission discovery"
    );
    assert_eq!(h.try_click("policy-confirm"), Err(DEMO.into()));
    assert!(h.model.resource_operations.is_empty());
    assert!(h.model.operations.is_empty());
    assert!(h.model.error.is_none());
    // Before the review there is no operation to assess, so no line.
    let h = Harness::scene("snapshot-schedules");
    assert!(h.find(permissions::REVIEW_KEY).is_none());
    // The alarm review of a synthetic pool observed nothing to assess a
    // permission for (as in Rune); its confirm is refused all the same.
    let mut h = Harness::scene("alarms-review");
    assert!(h.find(permissions::REVIEW_KEY).is_none());
    assert_eq!(h.try_click("alarm-confirm"), Err(DEMO.into()));
    alarms::submit(&mut h.model);
    assert_eq!(editor(&h).error, Some(Error::Stale.to_string()));
    assert!(h.model.resource_operations.is_empty());
}

// -- 2. What the port changed: the commands ------------------------------------------

#[test]
fn the_commands_are_in_the_menus_of_the_plan() {
    let s = Schedules::new();
    let h = &s.h;
    let label = |id: &str| h.menu_item(id).map(|item| item.label);
    assert_eq!(
        label("pool.snapshot-schedules").as_deref(),
        Some("VM Snapshot Schedule Policies")
    );
    assert_eq!(
        label("tools.alarm-service-refresh").as_deref(),
        Some("Refresh Alarm Service")
    );
    // The VM menu item is for a VM: hidden while the pool is selected.
    assert_eq!(label("vm.assign-snapshot-schedule"), None);
    let ids = |target: &Target| -> Vec<&'static str> {
        (h.context_menu(target).into_iter())
            .map(|item| item.id.0)
            .filter(|id| {
                [
                    "pool.snapshot-schedules",
                    "vm.assign-snapshot-schedule",
                    "tools.alarm-service-refresh",
                ]
                .contains(id)
            })
            .collect()
    };
    assert_eq!(
        ids(&pool_target(h)),
        ["pool.snapshot-schedules", "tools.alarm-service-refresh"]
    );
    assert_eq!(ids(&s.vm(s.b)), ["vm.assign-snapshot-schedule"]);
}

#[test]
fn the_schedules_command_follows_the_pool_of_its_target() {
    let mut s = Schedules::new();
    assert_eq!(
        s.h.model
            .eligibility(super::commands::SNAPSHOT_SCHEDULES, &Target::none()),
        Eligibility::disabled(NO_POOL)
    );
    assert_eq!(
        s.h.model
            .eligibility(super::commands::ALARM_SERVICE_REFRESH, &Target::none()),
        Eligibility::disabled(NO_POOL)
    );
    // Any object of the pool will do: Rune's button asked only for an inventory.
    let vm = s.vm(s.b);
    s.h.invoke_on("pool.snapshot-schedules", &vm);
    let w = workspace(&s.h);
    assert_eq!(w.pool(), pool(&s.h));
    assert_eq!((w.vm, w.selected, w.mode), (None, None, Mode::Browse));
    assert_eq!(s.h.surface().id, DIALOG);
    assert_eq!(s.h.surface().title, "Snapshot schedules");
    // In a synthetic pool nothing was observed and nothing asked.
    assert!(w.observed.is_none() && w.query.is_none() && w.support.is_none());
    assert_eq!(
        s.h.node("policy-clock").text.as_deref(),
        Some("Host support and local clock have not been read")
    );
    assert!(s.h.find("policy-preview").is_some());
    s.h.tick(0.1);
    assert!(!s.h.tick(0.1), "a quiet dialog does not ask for a redraw");
    assert_eq!(s.h.surface().id, DIALOG);
}

#[test]
fn assign_to_snapshot_schedule_opens_the_dialog_for_that_vm() {
    let mut s = Schedules::new();
    // A member: its schedule is selected and it is first among the VMs.
    let a = s.vm(s.a);
    s.h.invoke_on("vm.assign-snapshot-schedule", &a);
    let w = workspace(&s.h);
    assert_eq!((w.vm, w.selected), (Some(s.a), Some(s.weekend)));
    assert!(
        s.h.node("policy-vm")
            .text
            .unwrap()
            .contains(&format!("Assign to Snapshot Schedule: {}", s.name(s.a)))
    );
    let first = (s.h.keys().into_iter())
        .find(|key| key.starts_with("policy-member-"))
        .expect("a member button");
    assert_eq!(first, format!("policy-member-{}", s.a));
    // Refresh keeps the dialog on that VM.
    s.h.click("policy-refresh");
    assert_eq!(workspace(&s.h).vm, Some(s.a));
    s.h.dismiss();
    // A VM in no schedule: nothing is selected, the user chooses; it is
    // first in the list of the schedule chosen, and the change is one
    // reviewed membership.
    let b = s.vm(s.b);
    s.h.invoke_on("vm.assign-snapshot-schedule", &b);
    assert_eq!(workspace(&s.h).selected, None);
    s.choose(s.nightly);
    let first = (s.h.keys().into_iter())
        .find(|key| key.starts_with("policy-member-"))
        .expect("a member button");
    assert_eq!(first, format!("policy-member-{}", s.b));
    s.h.click(&first);
    assert!(matches!(
        &workspace(&s.h).review,
        Some(Op::Membership { vm, join: true, expected }) if *vm == s.b && expected.id == s.nightly
    ));
    s.h.dismiss();
    // Not for a server, several VMs, a template or a control domain.
    let hidden = |h: &Harness, target: &Target| {
        h.model
            .eligibility(super::commands::ASSIGN_SNAPSHOT_SCHEDULE, target)
            == Eligibility::Hidden
    };
    let pool = pool(&s.h);
    let host = *s.h.model.inventory.as_ref().unwrap().hosts.keys().next().unwrap();
    assert!(hidden(&s.h, &Target::one(ObjectRef::host(pool, host))));
    assert!(hidden(&s.h, &pool_target(&s.h)));
    assert!(hidden(
        &s.h,
        &Target::many([ObjectRef::vm(pool, s.a), ObjectRef::vm(pool, s.b)])
    ));
    let b = s.b;
    amend(&mut s.h, |i| i.vms.get_mut(&b).unwrap().is_template = true);
    assert!(hidden(&s.h, &s.vm(s.b)));
    amend(&mut s.h, |i| {
        let vm = i.vms.get_mut(&b).unwrap();
        vm.is_template = false;
        vm.is_control_domain = true;
    });
    assert!(hidden(&s.h, &s.vm(s.b)));
}

// -- 2. What the port changed: the list ----------------------------------------------

#[test]
fn the_list_selects_and_every_action_has_its_own_review() {
    let mut s = Schedules::open();
    let labels: Vec<_> = [s.nightly, s.weekend, s.staged]
        .map(|id| s.h.node(&format!("policy-select-{id}")).label.unwrap())
        .to_vec();
    assert_eq!(
        labels,
        [
            "Nightly · disabled · 0 VMs",
            "Weekend · enabled · 1 VMs",
            "Staged · disabled · 1 VMs"
        ]
    );
    assert!(s.h.find("policy-edit").is_none(), "nothing is chosen");
    assert!(s.h.find("policy-back").is_none());

    s.choose(s.nightly);
    assert_eq!(
        s.h.node("policy-detail").text.unwrap(),
        format!("UUID {} · daily · last run unknown", s.nightly)
    );
    assert_eq!(
        s.h.try_click("policy-run"),
        Err(schedules::RUN_NEEDS.into())
    );
    s.h.click("policy-toggle");
    assert!(workspace(&s.h).review.is_none());
    assert_eq!(
        error(&s.h).as_deref(),
        Some("Assign a VM before enabling this schedule")
    );
    // Hazard 55: a VM of another schedule is not offered.
    assert!(s.h.find(&format!("policy-member-{}", s.a)).is_none());
    assert!(s.h.find(&format!("policy-member-{}", s.c)).is_none());
    let member = format!("policy-member-{}", s.b);
    assert_eq!(
        s.h.node(&member).label.unwrap(),
        format!("Assign {}", s.name(s.b))
    );
    s.h.click(&member);
    assert!(matches!(
        &workspace(&s.h).review,
        Some(Op::Membership { vm, join: true, .. }) if *vm == s.b
    ));
    assert!(error(&s.h).is_none());
    // The review is in place: the same dialog, with Back and the confirm.
    assert_eq!(s.h.surface().id, DIALOG);
    assert!(s.h.has_text(&format!("Assign VM {} ({})", s.name(s.b), s.b)));
    assert!(s.h.has_text(&format!("Schedule Nightly ({})", s.nightly)));
    assert!(s.h.find("policy-create").is_none());
    assert_eq!(s.h.try_click("policy-confirm"), Err(DEMO.into()));
    s.h.click("policy-back");
    assert!(workspace(&s.h).review.is_none());
    assert_eq!(workspace(&s.h).selected, Some(s.nightly));
    assert_eq!(workspace(&s.h).mode, Mode::Browse);
    s.h.click("policy-delete");
    assert!(matches!(
        &workspace(&s.h).review,
        Some(Op::DeleteEmpty { expected }) if expected.id == s.nightly
    ));
    s.h.click("policy-back");

    // An enabled schedule: disabled first, everything else waits for that.
    s.choose(s.weekend);
    assert_eq!(
        s.h.try_click("policy-edit"),
        Err(schedules::EDIT_NEEDS_DISABLED.into())
    );
    assert_eq!(
        s.h.try_click("policy-delete"),
        Err(schedules::DELETE_NEEDS.into())
    );
    let member = format!("policy-member-{}", s.a);
    assert_eq!(
        s.h.node(&member).label.unwrap(),
        format!("Remove {}", s.name(s.a))
    );
    assert_eq!(
        s.h.try_click(&member),
        Err(schedules::MEMBER_NEEDS_DISABLED.into())
    );
    assert_eq!(s.h.node("policy-toggle").label.as_deref(), Some("Disable"));
    // Hazard 54: disabling needs no clock.
    s.h.click("policy-toggle");
    assert!(matches!(
        &workspace(&s.h).review,
        Some(Op::SetEnabled { enabled: false, clock: None, expected }) if expected.members == [s.a]
    ));
}

/// Hazard 54: enabling carries the clock, running now checks it.
#[test]
fn enabling_and_running_now_need_the_coordinator_clock() {
    let mut s = Schedules::open();
    s.choose(s.staged);
    assert_eq!(s.h.node("policy-toggle").label.as_deref(), Some("Enable"));
    s.h.click("policy-toggle");
    assert_eq!(error(&s.h).as_deref(), Some(CLOCK));
    s.choose(s.weekend);
    assert!(error(&s.h).is_none(), "choosing clears the error");
    s.h.click("policy-run");
    assert_eq!(error(&s.h).as_deref(), Some(CLOCK));
    assert!(workspace(&s.h).review.is_none());

    let clock = answer(&mut s.h);
    s.h.click("policy-run");
    assert!(matches!(
        &workspace(&s.h).review,
        Some(Op::RunNow { expected }) if expected.id == s.weekend
    ));
    assert!(s.h.has_text("A lost reply is never replayed."));
    assert!(s.h.has_text("Retention can delete the oldest VMSS-created snapshot"));
    assert!(s.h.has_text(&format!("Assigned VM: {} ({})", s.name(s.a), s.a)));
    s.h.click("policy-back");
    s.choose(s.staged);
    s.h.click("policy-toggle");
    assert!(matches!(
        &workspace(&s.h).review,
        Some(Op::SetEnabled { enabled: true, clock: Some(c), .. }) if *c == clock
    ));
    assert!(s.h.has_text("Retention can delete the oldest VMSS-created snapshot"));
}

/// Hazard 52: the dialog shows what it captured; Refresh captures again and
/// drops the selection and the draft.
#[test]
fn refresh_recaptures_and_drops_the_selection() {
    let mut s = Schedules::open();
    s.choose(s.nightly);
    let weekend = s.weekend;
    amend(&mut s.h, |i| {
        i.details.snapshot_policies.remove(&weekend);
    });
    assert!(s.h.find(&format!("policy-select-{weekend}")).is_some());
    s.h.click("policy-refresh");
    assert!(s.h.find(&format!("policy-select-{weekend}")).is_none());
    assert_eq!(workspace(&s.h).selected, None);
    amend(&mut s.h, |i| i.details.snapshot_policies.clear());
    s.h.click("policy-refresh");
    assert_eq!(
        s.h.node("policy-empty").text.as_deref(),
        Some("No snapshot schedules were returned by the host.")
    );
}

#[test]
fn a_schedule_the_client_cannot_read_has_no_actions() {
    let mut s = Schedules::new();
    let odd = policy(0x5009, "Odd", None, vec![]);
    let id = odd.id;
    amend(&mut s.h, |i| {
        i.details.snapshot_policies.insert(id, odd);
    });
    s.h.invoke("pool.snapshot-schedules");
    assert_eq!(
        s.h.node(&format!("policy-select-{id}")).label.as_deref(),
        Some("Odd · unknown · 0 VMs")
    );
    s.choose(id);
    assert!(s.h.find("policy-unsupported").is_some());
    assert!(s.h.find("policy-edit").is_none() && s.h.find("policy-toggle").is_none());
    schedules::edit(&mut s.h.model, false);
    assert_eq!(workspace(&s.h).mode, Mode::Browse);
    assert_eq!(
        error(&s.h).as_deref(),
        Some("This schedule cannot be edited")
    );
    schedules::action(&mut s.h.model, schedules::Action::Delete);
    assert_eq!(
        error(&s.h).as_deref(),
        Some("This schedule has unsupported settings or incomplete VM membership")
    );
}

// -- 2. What the port changed: the wizard --------------------------------------------

#[test]
fn the_wizard_shows_one_step_at_a_time_and_back_keeps_the_draft() {
    let mut h = Harness::scene("snapshot-schedules");
    let surface = h.surface();
    assert_eq!((surface.id.as_str(), surface.title.as_str()), (WIZARD, "New schedule"));
    assert_eq!(step(&h), 0);
    assert_eq!(h.node("policy-name").value.as_deref(), Some("Daily recovery"));
    assert!(h.find("policy-description").is_some());
    assert!(h.find("policy-minute").is_none() && h.find("policy-retention").is_none());
    // The clock is on every step: the review needs it.
    assert_eq!(
        h.node("policy-clock").text.as_deref(),
        Some(
            "Example coordinator local clock: 2026-11-08 10:00:00 (UTC+10:00) · UTC clock: \
             2026-11-08 00:00:00. Hours and weekdays follow the coordinator's timezone, \
             including future daylight-saving changes."
        )
    );
    h.type_text("policy-name", "Weekly recovery");
    h.click("policy-next");
    assert_eq!(step(&h), 1);
    assert!(h.find("policy-name").is_none());
    assert_eq!(h.node("policy-hour").value.as_deref(), Some("2"));
    assert_eq!(h.node("policy-minute").value.as_deref(), Some("0"));
    assert!(h.find("policy-day-0").is_none(), "daily");
    assert!(h.find("policy-clock").is_some());
    h.click("policy-frequency-0");
    assert!(h.find("policy-hour").is_none(), "hourly");
    h.click("policy-frequency-2");
    // Hazard 56: a new schedule starts with no weekday ticked.
    for day in 0..7 {
        assert_eq!(h.node(&format!("policy-day-{day}")).checked, Some(false));
    }
    h.click("policy-next");
    assert_eq!(step(&h), LAST_DRAFT_STEP);
    assert_eq!(h.node("policy-retention").value.as_deref(), Some("3"));
    assert_eq!(h.node("policy-checkpoint").checked, Some(false));
    assert!(h.find("policy-next").is_none());
    // A weekly schedule without a day is refused by the review, in place.
    h.click("policy-review");
    assert_eq!(step(&h), LAST_DRAFT_STEP);
    assert_eq!(
        error(&h).as_deref(),
        Some(
            "Use a name, retention 1–10, hour 0–23 and minute 0, 15, 30 or 45; weekly \
             schedules need at least one day"
        )
    );
    h.click("policy-back");
    assert_eq!(step(&h), 1);
    assert!(error(&h).is_none());
    h.click("policy-day-6");
    h.click("policy-next");
    h.click("policy-review");
    assert_eq!(step(&h), STEPS.len() - 1);
    assert!(h.has_text("Weekly recovery · disks only · retain 3 per VM · Sunday at 02:00"));
    assert!(h.find("policy-retention").is_none());
    // Back from the review: the last draft step, the draft intact.
    h.click("policy-back");
    assert_eq!(step(&h), LAST_DRAFT_STEP);
    assert!(workspace(&h).review.is_none());
    assert_eq!(workspace(&h).name, "Weekly recovery");
    h.click("policy-back");
    h.click("policy-back");
    assert_eq!(step(&h), 0);
    // Back from the first step: the list.
    h.click("policy-back");
    assert_eq!(h.surface().id, DIALOG);
    assert_eq!(workspace(&h).mode, Mode::Browse);
    h.click("policy-create");
    assert_eq!(step(&h), 0);
    assert_eq!(workspace(&h).name, "", "a new schedule starts empty");
    assert_eq!(workspace(&h).days, [false; 7]);
}

/// Hazard 53: the hour is parsed even for an hourly schedule, whose step
/// does not show it, and the refusals come in the Rune order.
#[test]
fn the_review_checks_the_whole_draft_in_the_rune_order() {
    let mut h = Harness::scene("snapshot-schedules");
    let refusal = |h: &mut Harness| {
        schedules::review(&mut h.model);
        assert!(workspace(h).review.is_none());
        workspace(h).error.clone().unwrap()
    };
    draft(&mut h).frequency = 0;
    draft(&mut h).hour = "x".into();
    draft(&mut h).minute = "x".into();
    draft(&mut h).retention = "x".into();
    assert_eq!(refusal(&mut h), "Choose minute 0, 15, 30 or 45");
    draft(&mut h).minute = "30".into();
    assert_eq!(refusal(&mut h), "Choose hour 0–23");
    draft(&mut h).hour = "7".into();
    assert_eq!(refusal(&mut h), "Retention must be 1–10 snapshots per VM");
    draft(&mut h).retention = "10".into();
    schedules::review(&mut h.model);
    assert!(matches!(
        &workspace(&h).review,
        Some(Op::Create { config, .. })
            if config.schedule == SnapshotSchedule::Hourly { minute: 30 } && config.retained == 10
    ));
}

/// Hazard 54: editing needs a disabled schedule and an actual change.
#[test]
fn edit_settings_is_the_same_wizard_and_needs_a_change() {
    let mut s = Schedules::open();
    answer(&mut s.h);
    s.choose(s.staged);
    s.h.click("policy-edit");
    let surface = s.h.surface();
    assert_eq!((surface.id.as_str(), surface.title.as_str()), (WIZARD, "Edit settings"));
    assert_eq!(
        surface.subtitle,
        Some(format!("Staged ({})", s.staged))
    );
    let w = workspace(&s.h);
    assert_eq!((w.mode, w.frequency, w.checkpoint), (Mode::Edit, 1, false));
    assert_eq!(
        (w.name.as_str(), w.hour.as_str(), w.minute.as_str(), w.retention.as_str()),
        ("Staged", "2", "0", "3")
    );
    s.h.click("policy-next");
    s.h.click("policy-next");
    s.h.click("policy-review");
    assert_eq!(error(&s.h).as_deref(), Some("No settings changed"));
    s.h.type_text("policy-retention", "5");
    s.h.click("policy-review");
    assert!(matches!(
        &workspace(&s.h).review,
        Some(Op::Configure { expected, config, .. })
            if expected.id == s.staged && expected.config.retained == 3 && config.retained == 5
    ));
    assert!(s.h.has_text(
        "The schedule stays disabled. A failure may leave some reviewed settings changed."
    ));
    // The review of an enabled schedule's settings is refused even when the
    // button was got around.
    s.h.click("policy-back");
    let w = s.h.model.areas.schedules_alarms.schedules.as_mut().unwrap();
    w.selected = Some(s.weekend);
    schedules::review(&mut s.h.model);
    assert_eq!(
        workspace(&s.h).error.as_deref(),
        Some("Disable the schedule before editing")
    );
}

#[test]
fn the_clock_line_says_what_is_known() {
    let mut s = Schedules::open();
    let line = |h: &Harness| h.node("policy-clock").text.unwrap();
    draft(&mut s.h).support = Some(Err("The host has no VMSS plugin".into()));
    assert_eq!(line(&s.h), "The host has no VMSS plugin");
    answer(&mut s.h);
    if let Some(Ok(support)) = &mut draft(&mut s.h).support {
        support.clock.utc_offset_minutes = -210;
    }
    assert!(line(&s.h).contains("(UTC−03:30)"), "{}", line(&s.h));
    assert_eq!(schedules::clock_label("20261108T10:00:00"), "2026-11-08 10:00:00");
    assert_eq!(schedules::clock_label("not a clock"), "not a clock");
}

/// The label says what the button shows; it leads to the Events tab.
#[test]
fn operation_history_closes_the_dialog_and_shows_the_events() {
    let mut s = Schedules::open();
    assert_eq!(
        s.h.node("policy-history").label.as_deref(),
        Some("Operation history")
    );
    s.h.click("policy-history");
    assert!(s.h.model.areas.schedules_alarms.schedules.is_none());
    assert!(s.h.surfaces().is_empty());
    assert_eq!(s.h.model.nav.view, NavView::Notifications);
    assert_eq!(s.h.model.active_tab(), TabId::Events);
}

#[test]
fn the_schedules_close_with_escape_the_scope_and_the_session() {
    let mut s = Schedules::open();
    s.h.dismiss();
    assert!(s.h.model.areas.schedules_alarms.schedules.is_none());
    s.h.invoke("pool.snapshot-schedules");
    s.h.click("policy-close");
    assert!(s.h.surfaces().is_empty());
    s.h.invoke("pool.snapshot-schedules");
    let vm = s.vm(s.a);
    s.h.select(vm);
    assert!(s.h.model.areas.schedules_alarms.schedules.is_none());
    s.h.invoke("pool.snapshot-schedules");
    s.h.model.load_demo(3, 48);
    assert!(s.h.model.areas.schedules_alarms.schedules.is_none());
    // One thing of the area at a time.
    s.h.invoke_on("tools.alarm-service-refresh", &pool_target(&s.h));
    assert_eq!(s.h.surface().id, REFRESH_DIALOG);
    let id = pool(&s.h);
    super::open_schedules(&mut s.h.model, id, None);
    assert_eq!(s.h.surfaces().len(), 1);
    assert_eq!(s.h.surface().id, DIALOG);
    let target = pool_target(&s.h);
    s.h.model
        .invoke(super::commands::ALARM_SERVICE_REFRESH, &target);
    assert_eq!(s.h.surfaces().len(), 1);
    assert_eq!(s.h.surface().id, REFRESH_DIALOG);
}

/// With a live session the pool is observed, the review is current and the
/// confirm button is offered; a read-only session's review is stale.
#[test]
fn a_review_is_current_only_on_a_writable_current_connection() {
    let review = |live: &mut Live| {
        let target = Target::one(ObjectRef::pool(live.pool));
        live.h.select(target);
        live.h.invoke("pool.snapshot-schedules");
        assert!(workspace(&live.h).observed.is_some());
        assert!(live.h.find("policy-preview").is_none());
        answer(&mut live.h);
        live.h.click("policy-create");
        live.h.type_text("policy-name", "Nightly");
        live.h.click("policy-next");
        live.h.click("policy-next");
        live.h.click("policy-review");
        assert!(matches!(&workspace(&live.h).review, Some(Op::Create { .. })));
        assert!(live.h.find(permissions::REVIEW_KEY).is_some());
    };
    let mut live = Live::with_vm();
    review(&mut live);
    assert!(schedules::current(&live.h.model, workspace(&live.h)));
    assert_eq!(schedules::stale(&live.h.model, workspace(&live.h)), None);
    let confirm = live.h.node("policy-confirm");
    assert_ne!(confirm.disabled.as_deref(), Some(schedules::STALE));
    assert_ne!(confirm.disabled.as_deref(), Some(DEMO));

    let pool = PoolId::from(uuid::Uuid::new_v4());
    let mut live = Live::over(Server::with_vm(pool), pool, true);
    review(&mut live);
    assert_eq!(
        live.h.try_click("policy-confirm"),
        Err(schedules::STALE.to_owned())
    );
    assert_eq!(
        live.h.node("policy-change-review-stale").text.as_deref(),
        Some(schedules::STALE)
    );
    schedules::submit(&mut live.h.model);
    assert_eq!(workspace(&live.h).error, Some(Error::Stale.to_string()));
    assert_eq!(live.server.count("VMSS.create"), 0);
}

// -- 2. What the port changed: the alarm pages ---------------------------------------

#[test]
fn the_pages_are_offered_only_where_the_inventory_has_the_settings() {
    let (mut h, pool, host) = alarm_pool(false);
    assert_eq!(pages(&h, &host), [ALERTS]);
    assert!(pages(&h, &pool).is_empty(), "no email configuration");
    let inventory = h.model.inventory.clone().unwrap();
    let id = inventory.pool_id;
    let other = Target::one(ObjectRef::host(id, *inventory.hosts.keys().nth(1).unwrap()));
    assert!(pages(&h, &other).is_empty(), "a server without a policy");
    let guest = (inventory.vms.values())
        .find(|vm| !vm.is_template && !vm.is_control_domain)
        .unwrap()
        .id;
    let vm = Target::one(ObjectRef::vm(id, guest));
    let storage = ObjectRef::storage(id, inventory.storage[0].uuid);
    assert!(pages(&h, &vm).is_empty());
    assert!(pages(&h, &Target::one(storage.clone())).is_empty());

    let blank = |target| AlarmPolicy {
        target,
        control_domain_host: None,
        revision: "1".repeat(64),
        rules: vec![],
        customized: vec![],
        other_variables: 0,
        error: None,
    };
    let repository = resource_target(&storage).expect("a repository is a resource");
    amend(&mut h, |i| {
        i.details.alarm_policies.push(blank(ResourceTarget::Vm(guest)));
        i.details.alarm_policies.push(blank(repository));
    });
    assert_eq!(pages(&h, &vm), [ALERTS]);
    assert_eq!(pages(&h, &Target::one(storage.clone())), [ALERTS]);
    assert!(pages(&h, &other).is_empty());
    assert!(
        pages(&h, &Target::many([ObjectRef::vm(id, guest), host.objects[0].clone()])).is_empty()
    );

    // A VM's page: its three metrics, none with a rule yet.
    h.select(vm.clone());
    assert!(h.model.open_properties(&vm, Some(ALERTS)));
    assert_eq!(h.surface().id, "vm-properties");
    let w = editor(&h);
    assert_eq!((w.page, w.index, w.metric, w.enabled), (Page::Policies, Some(1), AlarmMetric::Cpu, false));
    assert_eq!(w.threshold, "80");
    h.dismiss();
    // A repository's page: its one metric, in its own unit.
    let storage = Target::one(storage);
    h.select(storage.clone());
    assert!(h.model.open_properties(&storage, Some(ALERTS)));
    assert_eq!(h.surface().id, "storage-properties");
    let w = editor(&h);
    assert_eq!((w.index, w.metric), (Some(2), AlarmMetric::StorageThroughput));
    assert_eq!(w.threshold, "1024");
    h.dismiss();

    // The pool's page needs the email configuration.
    let (h, pool, _) = alarm_pool(true);
    assert_eq!(pages(&h, &pool), [EMAIL_OPTIONS]);
}

/// Hazards 59 and 60, on the page of the scene.
#[test]
fn the_alerts_page_reviews_one_alarm_and_back_keeps_the_draft() {
    let mut h = Harness::scene("alarms");
    assert_eq!(h.surface().id, "host-properties");
    assert_eq!(h.model.properties.as_ref().unwrap().area, "schedules_alarms");
    let w = editor(&h);
    assert_eq!((w.page, w.index, w.metric, w.enabled), (Page::Policies, Some(0), AlarmMetric::Cpu, true));
    assert_eq!(
        (w.threshold.as_str(), w.duration.as_str(), w.repeat.as_str()),
        ("80", "60", "3600")
    );
    assert!(w.target.is_none(), "nothing is observed in a synthetic pool");
    assert_eq!(
        h.node("alarm-trigger").text.as_deref(),
        Some("Trigger when CPU usage is above the threshold (percent (0–100))")
    );
    assert_eq!(
        h.node("alarm-retained").text.as_deref(),
        Some(
            "1 other/custom variables are retained. Host monitoring runs independently of this \
             desktop."
        )
    );
    assert!(h.find("alarm-preview").is_some());
    assert!(h.find("alarm-back").is_none() && h.find("alarm-confirm").is_none());

    // The review prints the threshold as typed; the operation has it as a
    // fraction, with the revision of the captured policy.
    h.type_text("alarm-threshold", "75");
    h.type_text("alarm-duration", "120");
    h.click("alarm-review");
    assert!(matches!(
        &editor(&h).review,
        Some(AlarmOperation::SetPolicy(change))
            if change.expected_revision == "0".repeat(64)
                && change.control_domain_host.is_none()
                && change.rule == Some(AlarmRule {
                    metric: AlarmMetric::Cpu,
                    threshold: "0.75".into(),
                    duration_seconds: 120,
                    repeat_seconds: 3600,
                })
    ));
    assert!(h.has_text(
        "CPU usage: above 75 percent (0–100), sustained for 120 seconds; repeat no sooner than \
         3600 seconds"
    ));
    assert!(h.has_text("Request a service refresh separately"));
    assert!(h.find("alarm-threshold").is_none() && h.find("alarm-review").is_none());
    h.click("alarm-back");
    assert!(editor(&h).review.is_none());
    assert_eq!(editor(&h).threshold, "75");

    // A metric without a rule starts disabled, and cannot be disabled again.
    let network = alarms_view::metric_id(0, AlarmMetric::Network);
    h.pick("alarm-metric", &network);
    let w = editor(&h);
    assert_eq!((w.metric, w.enabled), (AlarmMetric::Network, false));
    assert_eq!(w.threshold, "1024");
    h.click("alarm-review");
    assert_eq!(
        h.node("alarm-error").text.as_deref(),
        Some("This alarm is already disabled")
    );
    h.click("alarm-enabled");
    h.type_text("alarm-duration", "soon");
    h.click("alarm-review");
    assert_eq!(
        h.node("alarm-error").text.as_deref(),
        Some("Duration must be a whole number of seconds")
    );
    h.type_text("alarm-duration", "0");
    h.click("alarm-review");
    assert_eq!(
        h.node("alarm-error").text.as_deref(),
        Some(
            "Check the threshold and use duration 1–604800 seconds and repeat interval \
             1–2592000 seconds"
        )
    );

    // Reload settings: the page is entered again and captures afresh.
    h.click("alarm-reload");
    assert_eq!(h.surface().id, "host-properties");
    let w = editor(&h);
    assert_eq!((w.metric, w.threshold.as_str()), (AlarmMetric::Cpu, "80"));
    assert!(w.error.is_none());
}

/// A server has two policies, its own and the memory of its control domain:
/// one page shows the metrics of both, and the metric picks the policy.
#[test]
fn a_server_page_has_the_alarm_of_its_control_domain() {
    let (mut h, _, host) = alarm_pool(false);
    let server = host.objects[0].host_id().unwrap();
    let control = VmId::from(uuid::Uuid::from_u128(0x9902));
    amend(&mut h, |i| {
        i.details.alarm_policies.push(AlarmPolicy {
            target: ResourceTarget::Vm(control),
            control_domain_host: Some(server),
            revision: "2".repeat(64),
            rules: vec![],
            customized: vec![],
            other_variables: 0,
            error: None,
        });
    });
    assert_eq!(alarms::policies_of(h.model.inventory.as_ref().unwrap(), &host.objects[0]), [0, 1]);
    assert!(h.model.open_properties(&host, Some(ALERTS)));
    assert_eq!(editor(&h).index, Some(0));
    let memory = alarms_view::metric_id(1, AlarmMetric::ControlDomainMemory);
    h.pick("alarm-metric", &memory);
    let w = editor(&h);
    assert_eq!((w.index, w.metric, w.enabled), (Some(1), AlarmMetric::ControlDomainMemory, false));
    assert_eq!(w.threshold, "80", "a percent metric");
    h.click("alarm-enabled");
    h.click("alarm-review");
    assert!(matches!(
        &editor(&h).review,
        Some(AlarmOperation::SetPolicy(change))
            if change.control_domain_host == Some(server)
                && change.expected_revision == "2".repeat(64)
                && change.rule.as_ref().is_some_and(|rule| rule.threshold == "0.8")
    ));
    let name = h.model.inventory.as_ref().unwrap().hosts[&server].name.clone();
    assert!(h.has_text(&format!("{name} · control-domain memory ({control})")));
    // Back to a metric of the server's own policy.
    h.click("alarm-back");
    h.pick("alarm-metric", &alarms_view::metric_id(0, AlarmMetric::Cpu));
    let w = editor(&h);
    assert_eq!((w.index, w.metric, w.enabled), (Some(0), AlarmMetric::Cpu, true));
}

/// A policy the host could not read refuses its review with the host's
/// words, and shows them.
#[test]
fn a_policy_error_refuses_the_review() {
    let (mut h, _, host) = alarm_pool(false);
    amend(&mut h, |i| {
        i.details.alarm_policies[0].error = Some("perfmon configuration is not valid XML".into());
    });
    assert!(h.model.open_properties(&host, Some(ALERTS)));
    assert_eq!(
        h.node("alarm-policy-error").text.as_deref(),
        Some("perfmon configuration is not valid XML")
    );
    h.click("alarm-review");
    assert!(editor(&h).review.is_none());
    assert_eq!(
        h.node("alarm-error").text.as_deref(),
        Some("perfmon configuration is not valid XML")
    );
}

#[test]
fn the_editor_lives_and_dies_with_its_page() {
    let mut h = Harness::scene("alarms");
    // To a page of another area: the editor is dropped.
    h.pick(HOST_PAGES, "multipathing");
    assert!(h.model.areas.schedules_alarms.alarms.is_none());
    assert!(h.find("alarm-threshold").is_none());
    h.pick(HOST_PAGES, ALERTS);
    assert!(h.find("alarm-threshold").is_some());
    // Escape closes the dialog and the editor with it.
    h.dismiss();
    assert!(h.model.properties.is_none());
    assert!(h.model.areas.schedules_alarms.alarms.is_none());
    // The page's own Close closes the dialog.
    let mut h = Harness::scene("alarms-review");
    h.click("alarm-close");
    assert!(h.model.properties.is_none() && h.surfaces().is_empty());
    assert!(h.model.areas.schedules_alarms.alarms.is_none());
    // Another object selected, another session: the same.
    let mut h = Harness::scene("alarms");
    let pool = pool_target(&h);
    h.select(pool);
    assert!(h.model.properties.is_none());
    assert!(h.model.areas.schedules_alarms.alarms.is_none());
    let mut h = Harness::scene("alarms-email");
    h.model.load_demo(3, 48);
    assert!(h.model.properties.is_none());
    assert!(h.model.areas.schedules_alarms.alarms.is_none());
}

#[test]
fn the_email_options_page_reviews_the_pool_email() {
    let mut h = Harness::scene("alarms-email");
    assert_eq!(h.surface().id, "pool-properties");
    assert_eq!(editor(&h).page, Page::Email);
    assert_eq!(
        h.node("alarm-email").value.as_deref(),
        Some("operator@example.invalid")
    );
    assert_eq!(
        h.node("alarm-relay").value.as_deref(),
        Some("smtp.example.invalid:25")
    );
    assert!(h.find("alarm-threshold").is_none() && h.find("alarm-refresh-service").is_none());
    h.pick("alarm-language", "ja-JP");
    h.click("alarm-review");
    assert!(matches!(
        &editor(&h).review,
        Some(AlarmOperation::SetEmail { expected_revision, settings })
            if *expected_revision == "0".repeat(64) && settings.language == "ja-JP"
    ));
    assert!(h.has_text(
        "Destination: operator@example.invalid · relay: smtp.example.invalid:25 · language: ja-JP"
    ));
    assert_eq!(h.try_click("alarm-confirm"), Err(DEMO.into()));
    h.click("alarm-back");
    h.type_text("alarm-email", "two@example.invalid, addresses@example.invalid");
    h.click("alarm-review");
    assert_eq!(
        h.node("alarm-error").text.as_deref(),
        Some(
            "Use one email address (or leave it empty to disable), a relay host[:port], and a \
             supported language"
        )
    );
    h.type_text("alarm-email", "");
    h.click("alarm-review");
    assert!(h.has_text("Disable pool alarm email delivery"));

    // A configuration the host could not read refuses with the host's words.
    let (mut h, pool, _) = alarm_pool(true);
    amend(&mut h, |i| {
        i.details.alarm_email.as_mut().unwrap().error = Some("mail.conf is unreadable".into());
    });
    h.select(pool.clone());
    assert!(h.model.open_properties(&pool, Some(EMAIL_OPTIONS)));
    assert_eq!(
        h.node("alarm-email-error").text.as_deref(),
        Some("mail.conf is unreadable")
    );
    h.click("alarm-review");
    assert_eq!(
        h.node("alarm-error").text.as_deref(),
        Some("mail.conf is unreadable")
    );
}

// -- 2. What the port changed: the refresh of the service -----------------------------

#[test]
fn the_refresh_opens_on_its_review_and_has_nothing_to_go_back_to() {
    let (mut h, pool, _) = alarm_pool(false);
    h.invoke_on("tools.alarm-service-refresh", &pool);
    let surface = h.surface();
    assert_eq!((surface.id.as_str(), surface.title.as_str()), (REFRESH_DIALOG, "Refresh service"));
    let hosts: Vec<_> = editor(&h).initial.hosts.keys().copied().collect();
    assert_eq!(hosts.len(), 3);
    assert_eq!(
        editor(&h).review,
        Some(AlarmOperation::Refresh { hosts: hosts.clone() })
    );
    for host in &hosts {
        let name = &editor(&h).initial.hosts[host].name;
        assert!(h.has_text(&format!("{name} ({host})")));
    }
    assert!(h.has_text("Failed or uncertain requests are not replayed."));
    assert!(h.find("alarm-refresh-about").is_some());
    assert!(h.find("alarm-back").is_none() && h.find("alarm-review").is_none());
    assert_eq!(h.try_click("alarm-confirm"), Err(DEMO.into()));
    // Reload settings captures and reviews again.
    h.click("alarm-reload");
    assert_eq!(h.surface().id, REFRESH_DIALOG);
    assert!(editor(&h).review.is_some());
    h.click("alarm-close");
    assert!(h.surfaces().is_empty());

    // More servers than one request takes: the command says so.
    let mut h = Harness::new();
    h.model.load_demo(65, 1);
    assert_eq!(
        h.model
            .eligibility(super::commands::ALARM_SERVICE_REFRESH, &pool_target(&h)),
        Eligibility::disabled("Refresh supports 1–64 captured hosts")
    );
}

/// The Rune tab beside "Thresholds" is a button of the Alerts page: the
/// Properties dialog gives way to the command's dialog.
#[test]
fn the_alerts_page_leads_to_the_refresh() {
    let mut h = Harness::scene("alarms");
    assert_eq!(
        h.node("alarm-refresh-service").label.as_deref(),
        Some("Refresh service…")
    );
    h.click("alarm-refresh-service");
    assert!(h.model.properties.is_none());
    assert_eq!(h.surfaces().len(), 1);
    assert_eq!(h.surface().id, REFRESH_DIALOG);
    assert_eq!(editor(&h).page, Page::Refresh);
}

// -- 3. Hazards ----------------------------------------------------------------------

/// Hazards 58 and 62 with a live session: the target of the policy is
/// observed when the page is entered, the review is current, and the
/// permission is assessed for the action on that target.
#[test]
fn a_live_alarm_review_observes_its_target_and_explains_the_permission() {
    let mut live = Live::with_vm();
    let host = Target::one(live.host());
    let server = live.server.host();
    // A live inventory has a policy for every server, guest and repository;
    // the fixture server has no alarm metadata, so each carries the error
    // of its reading. This test gives the server's a readable one.
    amend(&mut live.h, |i| {
        let policy = (i.details.alarm_policies.iter_mut())
            .find(|policy| policy.target == ResourceTarget::Host(server))
            .expect("the policy of the server");
        assert_eq!(
            policy.error.as_deref(),
            Some("Invalid XAPI response: alarm metadata unavailable")
        );
        policy.error = None;
        policy.rules.clear();
    });
    live.h.select(host.clone());
    assert!(live.h.model.open_properties(&host, Some(ALERTS)));
    let w = editor(&live.h);
    assert_eq!(w.target.map(|o| o.target()), Some(ResourceTarget::Host(server)));
    assert_eq!(w.pool.map(|o| o.target()), Some(ResourceTarget::Pool(live.pool)));
    assert!(live.h.find("alarm-preview").is_none());
    assert!(live.h.find(permissions::REVIEW_KEY).is_none(), "nothing to assess yet");
    live.h.click("alarm-enabled");
    live.h.click("alarm-review");
    let w = editor(&live.h);
    assert!(w.review.is_some());
    assert!(alarms::current(&live.h.model, w));
    assert_eq!(alarms::stale(&live.h.model, w), None);
    assert!(alarms::permission(&live.h.model, w).is_some());
    assert!(live.h.find(permissions::REVIEW_KEY).is_some());
    let confirm = live.h.node("alarm-confirm");
    assert_ne!(confirm.disabled.as_deref(), Some(alarms::STALE));
    assert_ne!(confirm.disabled.as_deref(), Some(DEMO));
}
