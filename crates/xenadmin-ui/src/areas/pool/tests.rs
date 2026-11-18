//! The tests of the area, in four parts.
//!
//! 1. **The Rune tests.** The thirteen of the five modules, under names
//!    that say which they were, and this area's share of two tests of other
//!    Rune files that ran over its scenes. Each asserts what the Rune test
//!    asserted; where the Rune test pressed a button of a `rune::Runtime`,
//!    the test here presses the same key through the harness.
//! 2. **What the port changed**: the hub became a tab, commands, a wizard
//!    and two pages.
//! 3. **The hazards** the map lists for these modules (25 to 37).
//! 4. **A live session** (the fixture server): a review that is current, a
//!    submission that is tracked, the join and the password change against
//!    real connections.

use std::sync::Arc;

use xenadmin_core::{
    CredentialKey, CredentialStoreError, Credentials, HaRestartPriority, HostId, Inventory,
    OperationId, PoolDissolve, PoolId, PoolOperation, PoolRootPasswordChange,
    ResourceOperationRecord, SrId, VmId,
};

use super::commands::{self, pool_of};
use super::policy::{self, Editor, Kind};
use super::{DEMO, HA_FIRST, dissolve, join, root_password, view};
use crate::Harness;
use crate::command;
use crate::prelude::*;
use crate::session::CredentialJob;
use crate::surface::EXPERIMENTAL_NOTICE_KEY;
use crate::testing::{Live, Server, connect};

// -- Fixtures -----------------------------------------------------------------------

const PAGES: &str = "pool-properties-pages";

/// The demo pool, selected, on its HA tab, with nothing open.
fn tab() -> Harness {
    let h = Harness::scene("pool-policies");
    assert!(h.surfaces().is_empty());
    h
}

fn inventory(h: &Harness) -> Arc<Inventory> {
    h.model.inventory.clone().expect("inventory")
}

fn pool(h: &Harness) -> PoolId {
    inventory(h).pool_id
}

fn pool_target(h: &Harness) -> Target {
    Target::one(ObjectRef::pool(pool(h)))
}

fn hosts(h: &Harness) -> Vec<HostId> {
    inventory(h).hosts.keys().copied().collect()
}

fn host_target(h: &Harness, host: HostId) -> Target {
    Target::one(ObjectRef::host(pool(h), host))
}

/// The first row of the HA tab's grid.
fn first_vm(h: &Harness) -> VmId {
    let inventory = inventory(h);
    inventory
        .guests()
        .min_by(|a, b| natural_cmp(&a.name, &b.name).then_with(|| a.id.cmp(&b.id)))
        .expect("a guest")
        .id
}

#[track_caller]
fn editor(h: &Harness) -> &Editor {
    h.model.areas.pool.editor.as_ref().expect("a pool operation")
}

#[track_caller]
fn editor_mut(h: &mut Harness) -> &mut Editor {
    h.model.areas.pool.editor.as_mut().expect("a pool operation")
}

#[track_caller]
fn dissolving(h: &Harness) -> &dissolve::Editor {
    h.model.areas.pool.dissolve.as_ref().expect("a dissolution")
}

#[track_caller]
fn joining(h: &Harness) -> &join::Editor {
    h.model.areas.pool.join.as_ref().expect("a join")
}

#[track_caller]
fn password(h: &Harness) -> &root_password::Editor {
    let editor = h.model.areas.pool.root_password.as_ref();
    editor.expect("a password change")
}

/// The inventory moves on: what a poll does when the host reports a change.
fn amend(h: &mut Harness, change: impl FnOnce(&mut Inventory)) {
    let mut inventory = inventory(h).as_ref().clone();
    change(&mut inventory);
    h.model.inventory = Some(Arc::new(inventory));
}

fn administration(i: &mut Inventory) -> &mut xenadmin_core::PoolAdministration {
    &mut i.details.pool.as_mut().expect("pool details").administration
}

fn disabled(reason: &str) -> Eligibility {
    Eligibility::Disabled(reason.to_owned())
}

// -- 1. The Rune tests --------------------------------------------------------------

/// Rune `pool_view.rs`
/// `review_captures_target_and_compact_footer_never_submits_demo`. The
/// footer staying on screen at 513x652 is the GPUI half.
#[test]
fn review_captures_target_and_never_submits_demo() {
    let mut h = Harness::scene("pool-policy-review");
    let pool = editor(&h).pool;
    assert_eq!(h.surface().id, "pool-change-ha-disable");
    assert!(h.find("pool-confirm").is_some());
    assert_eq!(h.try_click("pool-confirm"), Err(DEMO.into()));
    h.type_text("pool-confirmation", &pool.to_string());
    assert_eq!(h.try_click("pool-confirm"), Err(DEMO.into()));
    policy::apply(&mut h.model);
    assert_eq!(editor(&h).error.as_deref(), Some(policy::STALE));
    assert!(h.model.resource_operations.is_empty());
    assert_eq!(editor(&h).pool, pool);
    assert!(
        policy::describe(editor(&h), &PoolOperation::DisableHa)
            .iter()
            .any(|s| s.contains("no longer"))
    );
}

/// Rune `pool_view.rs`
/// `ejection_requires_host_uuid_and_priority_unknown_is_not_disabled`. The
/// Rune test rebuilt the table rows under the staged review; here a poll
/// runs under it.
#[test]
fn ejection_requires_host_uuid_and_priority_unknown_is_not_disabled() {
    let mut h = tab();
    let host = hosts(&h)[1];
    let op = PoolOperation::EjectHost {
        host,
        expected_name: "Fixture".into(),
    };
    assert_eq!(policy::confirmation(&op, pool(&h)), Some(host.to_string()));
    assert_eq!(HaRestartPriority::parse("future-policy"), None);
    assert_eq!(
        HaRestartPriority::parse(""),
        Some(HaRestartPriority::Disabled)
    );
    let target = host_target(&h, host);
    h.invoke_on("server.remove-from-pool", &target);
    h.tick(0.1);
    assert!(
        matches!(editor(&h).review, Some(PoolOperation::EjectHost { host: id, .. }) if id == host)
    );
    assert!(h.has_text(&format!("Type this UUID to confirm: {host}")));
    // The pool's UUID is not the one asked for.
    let wrong = pool(&h).to_string();
    h.type_text("pool-confirmation", &wrong);
    assert!(policy::unconfirmed(editor(&h)));
    h.type_text("pool-confirmation", &host.to_string());
    assert!(!policy::unconfirmed(editor(&h)));
}

/// Rune `ha_policy_view.rs`
/// `ha_policy_review_binds_vm_and_shared_vapp_settings_at_both_sizes`.
#[test]
fn ha_policy_review_binds_vm_and_shared_vapp_settings() {
    let mut h = Harness::scene("ha-policy");
    assert_eq!(h.surface().id, "pool-change-vm-protection");
    let op = editor(&h).review.clone().expect("a review");
    assert_eq!(h.try_click("pool-confirm"), Err(DEMO.into()));
    assert!(h.model.resource_operations.is_empty());
    editor_mut(&mut h).vm = None;
    assert_eq!(editor(&h).review.as_ref(), Some(&op));
    let description = policy::describe(editor(&h), &op).join(" ");
    assert!(description.contains("vApp"));
    assert!(description.contains("0 → 2"));
}

/// Rune `ha_policy_view.rs`
/// `ha_policy_edits_refuse_invalid_inputs_and_keep_timeout_precedence_explicit`.
/// The startup drafts and the timeout draft are two dialogs now.
#[test]
fn ha_policy_edits_refuse_invalid_inputs_and_keep_timeout_precedence_explicit() {
    let mut h = Harness::scene("ha-policy");
    h.click("pool-back");
    for value in ["-1", "not-a-number", "86401"] {
        h.type_text("ha-startup-1", value);
        h.click("ha-startup-review-1");
        assert!(editor(&h).review.is_none(), "{value}");
        assert!(editor(&h).error.is_some(), "{value}");
    }
    h.dismiss();
    // The VM is still chosen in the grid: the command finds its pool.
    h.invoke("pool.ha.timeout");
    assert_eq!(h.surface().id, "pool-change-ha-timeout");
    assert_eq!(editor(&h).timeout, "60");
    h.type_text("ha-timeout-input", "120");
    h.click("ha-timeout-review");
    assert!(matches!(
        &editor(&h).review,
        Some(PoolOperation::SetHaTimeout { expected: Some(v), seconds: Some(120) }) if v == "60"
    ));
    h.click("pool-back");
    administration(Arc::make_mut(&mut editor_mut(&mut h).inventory)).ha_configuration_timeout =
        Some("90".into());
    policy::timeout(&mut h.model, false);
    assert!(editor(&h).review.is_none());
    let error = editor(&h).error.as_ref().expect("a refusal");
    assert!(error.contains("separate HA configuration"));
    // The draft is gone with it, and the precedence is said.
    assert!(h.find("ha-timeout-input").is_none());
    assert!(h.has_text("HA configuration timeout takes precedence: 90."));
}

/// Rune `ha_policy_view.rs`
/// `ha_policy_missing_membership_refuses_review_and_selecting_vm_resets_draft`.
#[test]
fn ha_policy_missing_membership_refuses_review_and_selecting_vm_resets_draft() {
    let mut h = Harness::scene("ha-policy");
    let vm = editor(&h).vm.expect("a VM");
    policy::select_vm(&mut h.model, vm);
    assert_eq!(editor(&h).startup[0], "0");
    let e = editor_mut(&mut h);
    e.review = None;
    e.startup[0] = "4".into();
    let details = &mut Arc::make_mut(&mut e.inventory).details;
    details.vms.get_mut(&vm).expect("details").appliance_known = false;
    policy::startup(&mut h.model, 0);
    assert!(editor(&h).review.is_none());
    assert_eq!(
        editor(&h).error.as_deref(),
        Some("vApp membership is unknown")
    );
    assert!(h.find("ha-startup-0").is_none());
    assert!(
        h.has_text("Startup settings or vApp membership are unknown; editing is unavailable.")
    );
}

/// Rune `pool_dissolve_view.rs`
/// `dissolve_review_keeps_exact_members_and_confirmation_reachable_at_both_widths`.
#[test]
fn dissolve_review_keeps_exact_members_and_confirmation_reachable() {
    let mut h = Harness::scene("pool-dissolve");
    assert_eq!(h.surface().id, view::DISSOLVE);
    let expected = dissolving(&h).operation.clone();
    let pool = dissolving(&h).pool;
    assert_eq!(expected.members.len(), 2);
    h.type_text("pool-dissolve-identity", &pool.to_string());
    for key in [
        "pool-dissolve-identity",
        permissions::REVIEW_KEY,
        "pool-dissolve-confirm",
    ] {
        assert!(h.find(key).is_some(), "{key}");
    }
    for member in &expected.members {
        assert!(h.has_text(&member.host.to_string()), "{}", member.name);
    }
    assert_eq!(h.try_click("pool-dissolve-confirm"), Err(DEMO.into()));
    assert!(h.model.resource_operations.is_empty());
    assert_eq!(dissolving(&h).operation, expected);
    // Still open: asserted before any tick.
    h.model.inventory = None;
    dissolve::apply(&mut h.model);
    assert!(h.model.resource_operations.is_empty());
    assert!(dissolving(&h).error.is_some());
}

/// Rune `pool_dissolve_view.rs`
/// `dissolve_capture_refuses_unknown_coordinator_ha_and_enabled_members_cannot_submit`.
#[test]
fn dissolve_capture_refuses_unknown_coordinator_ha_and_enabled_members_cannot_submit() {
    let mut h = Harness::scene("pool-dissolve");
    let inventory = inventory(&h).as_ref().clone();
    let mut bad = inventory.clone();
    administration(&mut bad).coordinator = None;
    assert!(PoolDissolve::capture(&bad).is_err());
    bad = inventory.clone();
    administration(&mut bad).ha_enabled = Some(true);
    assert!(PoolDissolve::capture(&bad).is_err());
    let member = dissolving(&h).operation.members[0].host;
    amend(&mut h, |i| {
        i.hosts.get_mut(&member).expect("member").enabled = true;
    });
    // Counted once, at open: the open review still says none is enabled.
    assert_eq!(dissolving(&h).enabled_members, 0);
    let target = pool_target(&h);
    h.dismiss();
    h.invoke_on("pool.make-standalone", &target);
    assert_eq!(dissolving(&h).enabled_members, 1);
    assert!(h.has_text("1 reviewed members are still enabled."));
}

/// Rune `profile_pool_join.rs`
/// `compact_join_review_keeps_footer_reachable_and_never_dispatches_demo`.
#[test]
fn join_review_never_dispatches_demo() {
    let mut h = Harness::scene("pool-join");
    assert_eq!(h.surface().id, view::JOIN);
    assert!(h.find("join-submit").is_some());
    assert_eq!(h.try_click("join-submit"), Err(DEMO.into()));
    let host = joining(&h).source.host.to_string();
    h.type_text("join-password", "synthetic-password");
    h.type_text("join-confirmation", &host);
    assert_eq!(h.try_click("join-submit"), Err(DEMO.into()));
    join::apply(&mut h.model);
    assert_eq!(joining(&h).error.as_deref(), Some(join::STALE));
    assert!(joining(&h).password.is_empty());
    assert!(h.model.resource_operations.is_empty());
    assert!(h.find("join-password").is_some());
}

/// Rune `profile_pool_join.rs`
/// `destination_change_and_stale_connection_clear_transient_credentials`.
#[test]
fn destination_change_and_stale_connection_clear_transient_credentials() {
    let mut h = Harness::scene("pool-join");
    let host = joining(&h).source.host.to_string();
    h.type_text("join-password", "synthetic-password");
    h.type_text("join-confirmation", &host);
    join::choose(&mut h.model, 0);
    assert!(joining(&h).password.is_empty());
    assert!(joining(&h).confirmation.is_empty());
    h.type_text("join-password", "synthetic-password");
    h.model.demo = false;
    assert!(join::poll(&mut h.model));
    assert!(joining(&h).password.is_empty());
    assert!(!join::current(&h.model, joining(&h)));
    assert_eq!(joining(&h).error.as_deref(), Some(join::CHANGED));
    // Nothing new to say the second time.
    assert!(!join::poll(&mut h.model));
    join::apply(&mut h.model);
    assert!(h.model.resource_operations.is_empty());
}

/// Rune `profile_root_password.rs`
/// `late_positive_evidence_never_saves_unknown_acknowledged_or_interrupted_completion`.
#[test]
fn late_positive_evidence_never_saves_unknown_acknowledged_or_interrupted_completion() {
    let pool = PoolId::from(uuid::Uuid::from_u128(0x11));
    let coordinator = HostId::from(uuid::Uuid::from_u128(0x22));
    let change = PoolRootPasswordChange {
        coordinator,
        members: vec![coordinator],
    };
    for (state, interrupted, allowed) in [
        ("Succeeded", false, true),
        ("PartiallyCompleted", false, true),
        ("OutcomeUnknown", false, false),
        ("AcknowledgedUnknown", true, false),
        ("PartiallyCompleted", true, false),
        ("Succeeded", true, false),
    ] {
        let record: ResourceOperationRecord = serde_json::from_value(serde_json::json!({
            "id": OperationId::new(), "pool": pool, "target": {"Pool": pool},
            "action": {"RootPassword": change},
            "state": state, "interrupted": interrupted, "progress": null,
            "detail": "Fixture result", "created_at_ms": 1,
            "can_cancel": false, "cancel_requested": false, "steps": [], "created": null,
            "root_password": {
                "coordinator_changed": true, "coordinator_login_verified": true,
                "reconnect_credentials_updated": true, "unverified_members": []
            }
        }))
        .expect("a record");
        assert_eq!(
            root_password::verified_completion(&record),
            allowed,
            "{state}, interrupted={interrupted}"
        );
    }
}

/// Rune `profile_root_password.rs`
/// `review_is_pool_scoped_masked_and_footer_stays_reachable`.
#[test]
fn password_review_is_pool_scoped_and_masked() {
    let mut h = Harness::scene("root-password");
    assert_eq!(h.surface().id, view::ROOT_PASSWORD);
    assert!(h.find("root-password-submit").is_some());
    assert_eq!(h.try_click("root-password-submit"), Err(DEMO.into()));
    root_password::submit(&mut h.model);
    assert_eq!(
        password(&h).message.as_deref(),
        Some(root_password::STALE)
    );
    assert!(h.model.resource_operations.is_empty());
    assert!(!password(&h).save);
    assert!(
        h.find("root-password-new").is_none(),
        "Review must not render the secret again"
    );
    assert!(h.find("root-password-repeat").is_none());
    assert!(h.has_text(&format!(
        "Type the pool UUID to confirm: {}",
        password(&h).pool
    )));
}

/// Rune `profile_root_password.rs`
/// `mismatched_passwords_and_stale_profile_clear_without_submission`.
#[test]
fn mismatched_passwords_and_stale_profile_clear_without_submission() {
    let mut h = Harness::scene("root-password");
    h.click("root-password-back");
    h.type_text("root-password-new", "synthetic-first");
    h.type_text("root-password-repeat", "synthetic-other");
    h.click("root-password-review");
    assert!(!password(&h).review);
    assert_eq!(
        password(&h).message.as_deref(),
        Some("The new passwords do not match")
    );
    h.type_text("root-password-repeat", "synthetic-first");
    h.click("root-password-review");
    assert!(password(&h).review);
    h.model.demo = false;
    assert!(root_password::poll(&mut h.model));
    let e = password(&h);
    assert!(e.password.is_empty() && e.repeated.is_empty());
    assert!(!e.review);
    assert_eq!(e.message.as_deref(), Some(root_password::CHANGED));
    assert!(!root_password::can_save(&h.model, e));
    root_password::submit(&mut h.model);
    assert!(h.model.resource_operations.is_empty());
}

/// Rune `profile_root_password.rs`
/// `password_store_failure_preserves_host_result_and_never_retries_automatically`.
#[test]
fn password_store_failure_preserves_host_result_and_never_retries_automatically() {
    let mut h = Harness::scene("root-password");
    let job = CredentialJob::save(
        CredentialKey::for_profile(&password(&h).profile),
        Credentials::new("root".into(), "fixture".into()),
        None,
    );
    job.result
        .replace(Some(Err(CredentialStoreError::Locked)));
    h.model.areas.pool.root_password.as_mut().expect("editor").saving = Some(job);
    assert!(root_password::poll(&mut h.model));
    let e = password(&h);
    assert!(e.saving.is_none());
    let message = e.message.as_ref().expect("a message");
    assert!(message.contains("Coordinator password changed"));
    assert!(message.contains("locked"));
    assert!(h.find("root-password-message").is_some());
    assert!(!root_password::poll(&mut h.model));
    assert!(h.model.workspace.credential_tasks.is_empty());
}

/// This area's share of Rune `main.rs`
/// `dialogs_outside_the_supported_core_carry_the_experimental_notice`, by
/// decision 1 of the port: every surface of the area carries the notice.
#[test]
fn every_surface_of_the_area_carries_the_experimental_notice() {
    for scene in [
        "pool-policy-review",
        "ha-policy",
        "pool-dissolve",
        "pool-join",
        "root-password",
    ] {
        let h = Harness::scene(scene);
        assert!(h.model.experimental_notice(), "{scene}");
        assert!(h.find(EXPERIMENTAL_NOTICE_KEY).is_some(), "{scene}");
    }
    // The tab is information: no notice.
    let mut h = tab();
    assert!(!h.model.experimental_notice());
    for page in ["advanced", "security"] {
        h.invoke("pool.properties");
        h.pick(PAGES, page);
        assert!(h.model.experimental_notice(), "{page}");
        h.dismiss();
    }
}

/// This area's share of Rune `session_permissions_view.rs`
/// `review_permission_notice_is_reachable_without_bypassing_synthetic_target_guard`
/// (its `pool` and `root_password_scene` entries).
#[test]
fn review_permission_notice_is_reachable_without_bypassing_synthetic_target_guard() {
    for (scene, confirm) in [
        ("pool-policy-review", "pool-confirm"),
        ("root-password", "root-password-submit"),
        ("pool-dissolve", "pool-dissolve-confirm"),
    ] {
        let mut h = Harness::scene(scene);
        assert!(
            h.find(permissions::REVIEW_KEY).is_some(),
            "{scene}: captured review must explain permission discovery"
        );
        assert_eq!(h.try_click(confirm), Err(DEMO.into()), "{scene}");
        assert!(h.model.resource_operations.is_empty());
        assert!(h.model.error.is_none());
    }
    // Before a review there is no operation to assess, so no line.
    let mut h = tab();
    h.invoke("pool.ha.failures-to-tolerate");
    assert!(h.find(permissions::REVIEW_KEY).is_none());
}

// -- 2. What the port changed -------------------------------------------------------

#[test]
fn the_commands_are_in_the_plans_menus() {
    let places: Vec<_> = command::registry()
        .iter()
        .filter(|spec| spec.area == "pool")
        .map(|spec| (spec.id.0, spec.label, spec.menu))
        .collect();
    for expected in [
        ("server.join-pool", "Add to Pool", Some(place::POOL_ADD_SERVER)),
        ("pool.remove-server", "Remove Server", Some(place::POOL_REMOVE_SERVER)),
        (
            "pool.ha.configure",
            "High Availability",
            Some(place::POOL_HIGH_AVAILABILITY),
        ),
        ("pool.ha.disable", "Disable HA", None),
        ("pool.ha.failures-to-tolerate", "Failure Tolerance", None),
        ("pool.ha.overcommit", "HA Overcommit", None),
        ("pool.ha.timeout", "HA Timeout Default", None),
        ("vm.ha.restart-priority", "Restart Priority", None),
        (
            "pool.change-server-password",
            "Change Server Password",
            Some(place::POOL_CHANGE_SERVER_PASSWORD),
        ),
        (
            "pool.make-standalone",
            "Make into Standalone Server",
            Some(place::POOL_MAKE_STANDALONE),
        ),
        (
            "server.change-password",
            "Change Password",
            Some(place::SERVER_PASSWORD_CHANGE),
        ),
        (
            "pool.designate-coordinator",
            "Hand Over Coordinator",
            Some(MenuPath::new(Menu::Server, 6, 5)),
        ),
        (
            "server.remove-from-pool",
            "Remove from Pool",
            Some(place::SERVER_REMOVE_FROM_POOL),
        ),
    ] {
        assert!(places.contains(&expected), "{expected:?} in {places:#?}");
    }
    assert_eq!(places.len(), 13);
}

#[test]
fn the_ha_tab_shows_the_state_and_its_local_row_follows_it() {
    let h = tab();
    for key in ["ha-state", "ha-priorities", "ha-vm-filter", "ha-vms"] {
        assert!(h.find(key).is_some(), "{key}");
    }
    // Heartbeat repositories are listed while HA is on.
    assert!(h.find("ha-heartbeat").is_none());
    let row: Vec<_> = h
        .local_row()
        .into_iter()
        .filter(|item| command::registry().iter().any(|s| s.id == item.id && s.area == "pool"))
        .map(|item| (item.id.0, item.disabled))
        .collect();
    assert_eq!(
        row,
        [
            ("pool.ha.configure", None),
            ("pool.ha.disable", Some(commands::HA_OFF.to_owned())),
            ("pool.ha.failures-to-tolerate", None),
            (
                "pool.ha.overcommit",
                Some(commands::OVERCOMMIT_UNKNOWN.to_owned())
            ),
            ("pool.ha.timeout", Some(policy::TIMEOUT_FIXED.to_owned())),
            (
                "vm.ha.restart-priority",
                Some(commands::SELECT_VM.to_owned())
            ),
        ]
    );
    // HA on: the scene of the Disable review.
    let mut h = Harness::scene("pool-policy-review");
    h.dismiss();
    assert!(h.find("ha-heartbeat").is_some());
    assert_eq!(h.eligibility("pool.ha.disable"), Eligibility::Enabled);
    assert_eq!(h.eligibility("pool.ha.configure"), disabled(commands::HA_ON));
    // HA unknown is neither.
    amend(&mut h, |i| administration(i).ha_enabled = None);
    assert_eq!(h.eligibility("pool.ha.disable"), disabled(commands::HA_UNKNOWN));
    assert_eq!(
        h.eligibility("pool.ha.configure"),
        disabled(commands::HA_UNKNOWN)
    );
}

#[test]
fn the_overcommit_command_names_the_change_and_opens_on_its_review() {
    let mut h = tab();
    amend(&mut h, |i| administration(i).ha_allow_overcommit = Some(true));
    let item = h.menu_item("pool.ha.overcommit");
    let item = item.or_else(|| {
        h.local_row()
            .into_iter()
            .find(|item| item.id == commands::HA_OVERCOMMIT)
    });
    assert_eq!(item.expect("the command").label, "Disallow HA overcommit…");
    h.invoke("pool.ha.overcommit");
    assert_eq!(h.surface().id, "pool-change-ha-overcommit");
    assert_eq!(
        editor(&h).review,
        Some(PoolOperation::SetHaAllowOvercommit {
            expected: true,
            enabled: false
        })
    );
    // Nothing to enter: no way back to a draft, no UUID asked for.
    assert!(h.find("pool-back").is_none());
    assert!(h.find("pool-confirmation").is_none());
}

#[test]
fn a_vm_chosen_in_the_grid_is_the_target_and_pool_commands_keep_their_pool() {
    let mut h = tab();
    let vm = first_vm(&h);
    h.select_row("ha-vms", &vm.to_string());
    assert_eq!(h.model.target(), Target::one(ObjectRef::vm(pool(&h), vm)));
    assert_eq!(pool_of(&h.model, &h.model.target()), Some(pool(&h)));
    assert_eq!(h.eligibility("vm.ha.restart-priority"), Eligibility::Enabled);
    assert_eq!(
        h.eligibility("pool.ha.failures-to-tolerate"),
        Eligibility::Enabled
    );
    h.invoke("pool.ha.failures-to-tolerate");
    assert_eq!(h.surface().id, "pool-change-ha-failures");
    assert_eq!(editor(&h).pool, pool(&h));
    h.dismiss();
    h.click("vm.ha.restart-priority");
    assert_eq!(h.surface().id, "pool-change-vm-protection");
    assert_eq!(editor(&h).kind, Kind::Protection);
    assert_eq!(editor(&h).vm, Some(vm));
    assert!(h.has_text(&format!("({vm})")));
    h.dismiss();
    // On another tab a VM is no way to a pool command.
    h.open_tab(TabId::General);
    let target = Target::one(ObjectRef::vm(pool(&h), vm));
    assert_eq!(pool_of(&h.model, &target), None);
    assert_eq!(
        h.model.eligibility(commands::HA_CONFIGURE, &target),
        Eligibility::Hidden
    );
}

#[test]
fn the_filter_narrows_the_grid_by_name_or_uuid() {
    let mut h = tab();
    let vm = first_vm(&h);
    let rows = |h: &Harness| match h.tab() {
        TabBody::Blocks(blocks) => crate::surface::flatten_blocks("tab", &blocks)
            .into_iter()
            .filter(|node| node.path.contains("ha-vms:"))
            .count(),
        TabBody::Custom(_) => 0,
    };
    let all = rows(&h);
    assert!(all > 1);
    h.type_text("ha-vm-filter", &vm.to_string());
    assert!(rows(&h) < all);
    assert!(rows(&h) >= 1);
    // A chosen row the filter hides is no target any more.
    h.select_row("ha-vms", &vm.to_string());
    assert_eq!(h.eligibility("vm.ha.restart-priority"), Eligibility::Enabled);
    h.type_text("ha-vm-filter", "no such machine");
    assert_eq!(rows(&h), 0);
    assert!(h.model.areas.pool.selected_vm.is_none());
    assert_eq!(
        h.eligibility("vm.ha.restart-priority"),
        disabled(commands::SELECT_VM)
    );
    assert!(h.has_text("No virtual machine matches the filter."));
}

#[test]
fn the_failure_tolerance_draft_is_checked_before_its_review() {
    let mut h = tab();
    h.invoke("pool.ha.failures-to-tolerate");
    assert_eq!(editor(&h).failures, "0");
    for (value, refusal) in [
        ("x", "Enter a whole number of host failures"),
        ("0", "Choose a different value below the number of pool hosts"),
        ("3", "Choose a different value below the number of pool hosts"),
    ] {
        h.type_text("ha-failures-input", value);
        h.click("ha-failures-review");
        assert_eq!(editor(&h).error.as_deref(), Some(refusal), "{value}");
        assert!(editor(&h).review.is_none());
    }
    h.type_text("ha-failures-input", "1");
    h.click("ha-failures-review");
    assert_eq!(
        editor(&h).review,
        Some(PoolOperation::SetHaFailuresToTolerate {
            expected: 0,
            failures: 1
        })
    );
    assert!(editor(&h).error.is_none());
    h.click("pool-back");
    assert_eq!(editor(&h).failures, "1");
    assert!(h.find("ha-failures-input").is_some());
}

#[test]
fn the_two_pages_stage_their_operations_inside_the_properties_dialog() {
    let mut h = tab();
    h.invoke("pool.properties");
    assert_eq!(h.surface().id, view::PROPERTIES);
    h.pick(PAGES, "security");
    assert_eq!(editor(&h).kind, Kind::Security);
    for key in ["pool-https", "pool-tls", "pool-secret"] {
        assert!(h.find(key).is_some(), "{key}");
    }
    assert!(h.has_text("Pool certificate verification: Disabled"));
    h.click("pool-https");
    assert_eq!(
        editor(&h).review,
        Some(PoolOperation::SetHttpsOnly {
            expected: false,
            enabled: true
        })
    );
    assert_eq!(h.surface().id, view::PROPERTIES);
    assert!(h.find("pool-confirmation").is_none());
    assert!(h.find(permissions::REVIEW_KEY).is_some());
    h.click("pool-back");
    h.click("pool-tls");
    assert_eq!(editor(&h).review, Some(PoolOperation::EnableTlsVerification));
    assert!(h.find("pool-confirmation").is_some());
    assert_eq!(h.try_click("pool-confirm"), Err(DEMO.into()));
    // Another page: the draft of this one is dropped.
    h.pick(PAGES, "advanced");
    assert_eq!(editor(&h).kind, Kind::Network);
    assert!(editor(&h).review.is_none());
    h.click("pool-compression");
    assert_eq!(
        editor(&h).review,
        Some(PoolOperation::SetMigrationCompression {
            expected: false,
            enabled: true
        })
    );
    // Close closes the dialog with the page's editor.
    h.click("pool-close");
    assert!(h.surfaces().is_empty());
    assert!(h.model.areas.pool.editor.is_none());
    assert!(h.model.properties.is_none());
}

#[test]
fn the_configure_wizard_chooses_heartbeat_storage_and_then_reviews() {
    let mut h = tab();
    let first = SrId::from(inventory(&h).storage[0].uuid);
    h.invoke("pool.ha.configure");
    let surface = h.surface();
    assert_eq!(surface.id, view::CONFIGURE);
    assert!(matches!(surface.kind, SurfaceKind::Wizard { current: 0, .. }));
    assert_eq!(
        h.try_click("pool-ha-enable"),
        Err("Select heartbeat storage first.".into())
    );
    h.click(&format!("ha-heartbeat-{first}"));
    h.click("pool-ha-enable");
    assert_eq!(
        editor(&h).review,
        Some(PoolOperation::EnableHa {
            heartbeat_srs: vec![first],
            expected_failures: 0
        })
    );
    assert!(matches!(
        h.surface().kind,
        SurfaceKind::Wizard { current: 1, .. }
    ));
    assert!(h.has_text(&format!("({first})")));
    assert!(h.find("pool-confirmation").is_none());
    h.click("pool-back");
    assert!(editor(&h).heartbeat.contains(&first));
    assert_eq!(h.node(&format!("ha-heartbeat-{first}")).checked, Some(true));
}

#[test]
fn membership_commands_open_on_their_review_and_the_pool_item_chooses_first() {
    let mut h = tab();
    let hosts = hosts(&h);
    let (coordinator, member) = (hosts[0], hosts[1]);
    let on = |h: &Harness, id: CommandId, host: HostId| {
        h.model.eligibility(id, &host_target(h, host))
    };
    assert_eq!(
        on(&h, commands::REMOVE_FROM_POOL, coordinator),
        disabled("This server is the pool coordinator.")
    );
    assert_eq!(
        on(&h, commands::DESIGNATE_COORDINATOR, coordinator),
        disabled("This server is already the pool coordinator.")
    );
    assert_eq!(
        on(&h, commands::REMOVE_FROM_POOL, member),
        Eligibility::Enabled
    );
    let context: Vec<_> = h
        .context_menu(&host_target(&h, member))
        .into_iter()
        .map(|item| item.id)
        .collect();
    for id in [
        commands::REMOVE_FROM_POOL,
        commands::DESIGNATE_COORDINATOR,
        commands::SERVER_CHANGE_PASSWORD,
    ] {
        assert!(context.contains(&id), "{id:?}");
    }
    let target = host_target(&h, member);
    h.invoke_on("pool.designate-coordinator", &target);
    assert_eq!(h.surface().id, "pool-change-handover");
    assert_eq!(
        editor(&h).review,
        Some(PoolOperation::DesignateCoordinator {
            expected: coordinator,
            host: member
        })
    );
    assert!(h.has_text(&format!("Type this UUID to confirm: {}", pool(&h))));
    assert!(h.find("pool-back").is_none());
    h.dismiss();
    // Pool > Remove Server: the member is chosen in the dialog.
    let target = pool_target(&h);
    h.invoke_on("pool.remove-server", &target);
    assert_eq!(h.surface().id, "pool-change-eject");
    assert!(editor(&h).review.is_none());
    assert_eq!(h.try_click("pool-eject"), Err("Select a member".into()));
    h.pick("pool-member", &coordinator.to_string());
    assert_eq!(
        h.try_click("pool-eject"),
        Err("This server is the pool coordinator.".into())
    );
    h.pick("pool-member", &member.to_string());
    h.click("pool-eject");
    assert!(
        matches!(editor(&h).review, Some(PoolOperation::EjectHost { host, .. }) if host == member)
    );
    assert!(h.find("pool-back").is_some());
}

#[test]
fn join_and_password_are_refused_in_the_demo_with_the_reason() {
    let mut h = tab();
    let host = hosts(&h)[0];
    assert_eq!(h.eligibility("server.join-pool"), disabled(DEMO));
    assert_eq!(h.eligibility("pool.change-server-password"), disabled(DEMO));
    assert_eq!(
        h.model
            .eligibility(commands::SERVER_CHANGE_PASSWORD, &host_target(&h, host)),
        disabled(DEMO)
    );
    // Neither is a command of a VM.
    h.open_tab(TabId::General);
    let vm = Target::one(ObjectRef::vm(pool(&h), first_vm(&h)));
    for id in [
        commands::JOIN,
        commands::CHANGE_SERVER_PASSWORD,
        commands::MAKE_STANDALONE,
        commands::REMOVE_SERVER,
    ] {
        assert_eq!(h.model.eligibility(id, &vm), Eligibility::Hidden, "{id:?}");
    }
}

#[test]
fn a_scope_change_closes_what_belonged_to_the_pool_and_keeps_the_password_change() {
    for scene in ["pool-policy-review", "pool-dissolve"] {
        let mut h = Harness::scene(scene);
        assert_eq!(h.surfaces().len(), 1, "{scene}");
        let target = host_target(&h, hosts(&h)[0]);
        h.select(target);
        assert!(h.surfaces().is_empty(), "{scene}");
    }
    let mut h = tab();
    h.type_text("ha-vm-filter", "app");
    h.select_row("ha-vms", &first_vm(&h).to_string());
    let target = host_target(&h, hosts(&h)[0]);
    h.select(target);
    assert!(h.model.areas.pool.filter.is_empty());
    assert!(h.model.areas.pool.selected_vm.is_none());
    // The password change belongs to a saved connection.
    let mut h = Harness::scene("root-password");
    let target = host_target(&h, hosts(&h)[0]);
    h.select(target);
    assert_eq!(h.surface().id, view::ROOT_PASSWORD);
    // Until that connection is the scope session that goes.
    h.model.workspace.selected = Some(password(&h).profile.id);
    super::session_change(&mut h.model);
    assert!(h.model.areas.pool.root_password.is_none());
    let mut h = Harness::scene("pool-join");
    h.model.workspace.selected = Some(joining(&h).destinations[0].profile.id);
    super::session_change(&mut h.model);
    assert!(h.model.areas.pool.join.is_none());
}

/// Decision 10 of the port: the texts that named Rune places ("pool
/// administration", "operation history") name this client's.
#[test]
fn texts_name_the_places_of_this_client() {
    assert_eq!(
        policy::STALE,
        "The connection changed; reopen this dialog"
    );
    assert!(policy::CLOSING.contains("Interrupted operations stay on the Events tab of the Notifications view for recheck"));
    assert!(root_password::SUBMITTED.contains("the result remains on the Events tab of the Notifications view"));
    assert_eq!(dissolve::STALE, "Connection changed; reopen pool dissolution");
    assert_eq!(
        join::STALE,
        "Connection settings or pool identity changed; reopen the join review"
    );
    let mut h = tab();
    h.invoke("pool.ha.failures-to-tolerate");
    editor_mut(&mut h).review = None;
    h.model.areas.pool.editor = None;
    policy::review_failures(&mut h.model);
    policy::startup(&mut h.model, 0);
    // "The dialog of this pool change is closed" goes nowhere when nothing is open.
    assert!(h.model.areas.pool.editor.is_none());
}

// -- 3. The hazards -----------------------------------------------------------------

/// Hazard 25: every new review empties the confirmation; Back keeps the
/// error and what was typed.
#[test]
fn hazard_a_new_review_empties_the_confirmation_and_back_keeps_the_error() {
    let mut h = Harness::scene("pool-policy-review");
    h.type_text("pool-confirmation", "typed");
    policy::stage(&mut h.model, PoolOperation::RotateSecret);
    assert!(editor(&h).confirmation.is_empty());
    h.type_text("pool-confirmation", "typed again");
    policy::apply(&mut h.model);
    assert!(editor(&h).error.is_some());
    policy::back(&mut h.model);
    assert!(editor(&h).review.is_none());
    assert!(editor(&h).error.is_some());
    assert_eq!(editor(&h).confirmation, "typed again");
}

/// Hazard 26: ejection asks for the host's UUID, four operations for the
/// pool's, the rest for nothing.
#[test]
fn hazard_the_confirmation_follows_the_operation() {
    let h = tab();
    let pool = pool(&h);
    let host = hosts(&h)[1];
    let asks_pool = [
        PoolOperation::DesignateCoordinator {
            expected: hosts(&h)[0],
            host,
        },
        PoolOperation::DisableHa,
        PoolOperation::RotateSecret,
        PoolOperation::EnableTlsVerification,
    ];
    for op in asks_pool {
        assert_eq!(policy::confirmation(&op, pool), Some(pool.to_string()));
    }
    let asks_nothing = [
        PoolOperation::SetHttpsOnly {
            expected: false,
            enabled: true,
        },
        PoolOperation::SetMigrationCompression {
            expected: false,
            enabled: true,
        },
        PoolOperation::SetHaFailuresToTolerate {
            expected: 0,
            failures: 1,
        },
        PoolOperation::EnableHa {
            heartbeat_srs: Vec::new(),
            expected_failures: 0,
        },
        PoolOperation::SetHaTimeout {
            expected: None,
            seconds: Some(60),
        },
    ];
    for op in asks_nothing {
        assert_eq!(policy::confirmation(&op, pool), None, "{}", op.label());
    }
}

/// Hazards 28 and 29: a control is absent while its value is unknown; TLS
/// and the secret rotation are offered only for a known "off"; the
/// rotation, the handover, the ejection and the dissolution need HA known
/// to be off, and unknown counts as on.
#[test]
fn hazard_unknown_values_hide_controls_and_unknown_ha_blocks_membership_changes() {
    let mut h = tab();
    amend(&mut h, |i| {
        let a = administration(i);
        a.https_only = None;
        a.tls_verification_enabled = None;
        a.migration_compression = None;
        a.ha_enabled = None;
    });
    h.invoke("pool.properties");
    h.pick(PAGES, "security");
    assert!(h.find("pool-https").is_none());
    assert!(h.find("pool-tls").is_none());
    assert!(h.has_text("Pool certificate verification: Unknown / unavailable"));
    assert_eq!(h.try_click("pool-secret"), Err(HA_FIRST.into()));
    h.pick(PAGES, "advanced");
    assert!(h.find("pool-compression").is_none());
    assert!(h.has_text("Migration compression: Unknown / unavailable"));
    h.dismiss();
    let member = host_target(&h, hosts(&h)[1]);
    for id in [commands::REMOVE_FROM_POOL, commands::DESIGNATE_COORDINATOR] {
        assert_eq!(h.model.eligibility(id, &member), disabled(HA_FIRST));
    }
    for id in ["pool.make-standalone", "pool.remove-server"] {
        assert_eq!(h.eligibility(id), disabled(HA_FIRST), "{id}");
    }
    // TLS already on, a rotation pending: neither is offered again.
    amend(&mut h, |i| {
        let a = administration(i);
        a.tls_verification_enabled = Some(true);
        a.secret_rotation_pending = Some(true);
    });
    h.invoke("pool.properties");
    h.pick(PAGES, "security");
    assert!(h.find("pool-tls").is_none());
    assert!(h.find("pool-secret").is_none());
    assert!(h.has_text("Pool certificate verification: Enabled"));
}

/// Hazard 30: at most eight heartbeat repositories, only shared ones with
/// user content; the operation carries the failure tolerance it was
/// reviewed against and does not set it.
#[test]
fn hazard_heartbeat_storage_is_shared_user_storage_and_at_most_eight() {
    let mut h = tab();
    amend(&mut h, |i| {
        let extra: Vec<_> = (0..12u128)
            .map(|n| {
                let mut sr = i.storage[0].clone();
                sr.uuid = uuid::Uuid::from_u128(0x5000 + n);
                sr.name = format!("Extra {n:02}");
                sr
            })
            .collect();
        for sr in extra {
            let mut details = i.details.storage[&SrId::from(i.storage[0].uuid)].clone();
            details.id = SrId::from(sr.uuid);
            i.details.storage.insert(details.id, details);
            i.storage.push(sr);
        }
        let local = SrId::from(i.storage[0].uuid);
        i.details.storage.get_mut(&local).expect("details").shared = Some(false);
        let iso = SrId::from(i.storage[1].uuid);
        i.details.storage.get_mut(&iso).expect("details").content_type = "iso".into();
        administration(i).ha_failures_to_tolerate = Some(1);
    });
    let (local, iso) = {
        let i = inventory(&h);
        (SrId::from(i.storage[0].uuid), SrId::from(i.storage[1].uuid))
    };
    h.invoke("pool.ha.configure");
    assert!(h.find(&format!("ha-heartbeat-{local}")).is_none());
    assert!(h.find(&format!("ha-heartbeat-{iso}")).is_none());
    let candidates = policy::heartbeat_candidates(&inventory(&h));
    // More than the eight that can be chosen.
    assert!(candidates.len() > 8, "{}", candidates.len());
    for (id, _) in &candidates {
        h.click(&format!("ha-heartbeat-{id}"));
    }
    assert_eq!(editor(&h).heartbeat.len(), 8);
    h.click("pool-ha-enable");
    assert!(matches!(
        &editor(&h).review,
        Some(PoolOperation::EnableHa { heartbeat_srs, expected_failures: 1 })
            if heartbeat_srs.len() == 8
    ));
}

/// Hazard 31: a priority that does not parse disables all three choices;
/// a known one stages the change to another.
#[test]
fn hazard_an_unknown_priority_disables_every_choice() {
    let mut h = tab();
    let vm = first_vm(&h);
    h.select_row("ha-vms", &vm.to_string());
    h.click("vm.ha.restart-priority");
    for key in [
        "ha-priority-restart",
        "ha-priority-best-effort",
        "ha-priority-disabled",
    ] {
        assert_eq!(
            h.try_click(key),
            Err("The current priority is unknown or unsupported.".into())
        );
    }
    assert!(h.has_text("Current priority: Unknown / unsupported"));
    h.dismiss();
    amend(&mut h, |i| {
        i.details.vms.get_mut(&vm).expect("details").ha_restart_priority = Some(String::new());
    });
    h.click("vm.ha.restart-priority");
    assert_eq!(
        h.try_click("ha-priority-disabled"),
        Err("This is the current priority.".into())
    );
    h.click("ha-priority-restart");
    assert_eq!(
        editor(&h).review,
        Some(PoolOperation::SetVmRestartPriority {
            vm,
            expected: HaRestartPriority::Disabled,
            priority: HaRestartPriority::Restart
        })
    );
    assert!(h.find("pool-confirmation").is_none());
}

/// Hazard 35: a dissolution that cannot be captured is refused with the
/// text Rune put into the error line, now the reason of the command.
#[test]
fn hazard_a_dissolution_that_cannot_be_captured_says_why() {
    let mut h = tab();
    assert_eq!(h.eligibility("pool.make-standalone"), Eligibility::Enabled);
    amend(&mut h, |i| administration(i).coordinator = None);
    let Eligibility::Disabled(reason) = h.eligibility("pool.make-standalone") else {
        panic!("the command must be disabled");
    };
    assert!(reason.starts_with("Cannot review dissolution: "), "{reason}");
    assert!(reason.ends_with(
        "The pool must be named, HA must be disabled, and membership must be available."
    ));
    let pool = pool(&h);
    dissolve::open(&mut h.model, pool);
    assert!(h.model.areas.pool.dissolve.is_none());
    assert_eq!(h.model.error, Some(reason));
}

// -- 4. A live session --------------------------------------------------------------

#[test]
fn a_live_review_is_current_asks_for_the_uuid_and_its_submission_is_tracked() {
    let mut live = Live::with_vm();
    let pool = live.pool;
    live.h.select(Target::one(ObjectRef::pool(pool)));
    policy::open_review(
        &mut live.h.model,
        pool,
        Kind::Disable,
        PoolOperation::DisableHa,
    );
    let h = &mut live.h;
    assert!(editor(h).observed.is_some());
    assert!(policy::current(&h.model, editor(h)));
    assert_eq!(policy::stale(&h.model, editor(h)), None);
    assert!(h.find("pool-preview").is_none());
    assert_eq!(h.try_click("pool-confirm"), Err(policy::CONFIRM.into()));
    h.type_text("pool-confirmation", &pool.to_string());
    h.click("pool-confirm");
    assert!(h.model.areas.pool.editor.is_none());
    assert_eq!(h.model.resource_operations.len(), 1);
}

#[test]
fn a_live_review_goes_stale_when_its_observation_is_not_the_current_one() {
    let mut live = Live::with_vm();
    let pool = live.pool;
    policy::open_review(
        &mut live.h.model,
        pool,
        Kind::Overcommit,
        PoolOperation::SetHaAllowOvercommit {
            expected: false,
            enabled: true,
        },
    );
    let h = &mut live.h;
    editor_mut(h).observed = None;
    assert_eq!(h.try_click("pool-confirm"), Err(policy::STALE.into()));
    assert!(h.find("pool-change-review-stale").is_some());
    policy::apply(&mut h.model);
    assert_eq!(editor(h).error.as_deref(), Some(policy::STALE));
    assert!(h.model.resource_operations.is_empty());
}

#[test]
fn a_live_password_change_needs_local_root_and_clears_the_draft_on_submission() {
    let mut live = Live::with_vm();
    let pool = live.pool;
    live.h.select(Target::one(ObjectRef::pool(pool)));
    let h = &mut live.h;
    // The fixture's account is not root.
    assert_eq!(
        h.eligibility("pool.change-server-password"),
        disabled(root_password::NOT_ROOT)
    );
    h.model
        .workspace
        .session_accounts
        .insert(live.profile.id, ("root".into(), true));
    assert_eq!(
        h.eligibility("pool.change-server-password"),
        Eligibility::Enabled
    );
    h.invoke("pool.change-server-password");
    assert_eq!(h.surface().id, view::ROOT_PASSWORD);
    // Not a keyring profile of root: nothing to save.
    assert!(h.find("root-password-save").is_none());
    h.click("root-password-review");
    assert!(!password(h).review);
    h.type_text("root-password-new", "synthetic-new");
    h.type_text("root-password-repeat", "synthetic-new");
    h.click("root-password-review");
    assert!(password(h).review);
    assert!(root_password::current(&h.model, password(h)));
    assert_eq!(
        h.try_click("root-password-submit"),
        Err(root_password::CONFIRM.into())
    );
    h.type_text("root-password-confirmation", &pool.to_string());
    h.click("root-password-submit");
    let e = password(h);
    assert_eq!(e.message.as_deref(), Some(root_password::SUBMITTED));
    assert!(e.operation.is_some());
    assert!(e.password.is_empty() && e.repeated.is_empty());
    assert!(e.candidate.is_none());
    assert_eq!(h.model.resource_operations.len(), 1);
    // The dialog stays, with nothing left to press but Close.
    assert!(h.find("root-password-submit").is_none());
    assert!(h.find("root-password-confirmation").is_none());
    // A second submission is refused.
    root_password::submit(&mut h.model);
    assert_eq!(h.model.resource_operations.len(), 1);
    h.click("root-password-close");
    assert!(h.surfaces().is_empty());
}

#[test]
fn a_live_standalone_server_reviews_its_join_to_another_connected_pool() {
    let mut live = Live::with_vm();
    let source = live.pool;
    let source_target = Target::one(ObjectRef::pool(source));
    live.h.select(source_target.clone());
    assert_eq!(live.h.eligibility("server.join-pool"), Eligibility::Enabled);
    live.h.invoke("server.join-pool");
    assert_eq!(live.h.surface().id, view::JOIN);
    assert!(joining(&live.h).destinations.is_empty());
    assert!(
        live.h
            .has_text("Connect the destination pool first, then reopen this view.")
    );
    live.h.dismiss();
    // A second pool is connected; it becomes the scope session.
    let other = PoolId::from(uuid::Uuid::new_v4());
    let server = Server::with_vm(other);
    connect(&mut live.h.model, &server, other, "B", false);
    assert!(live.h.model.areas.pool.join.is_none());
    let h = &mut live.h;
    h.invoke_on("server.join-pool", &source_target);
    let e = joining(h);
    assert_eq!(e.source.pool, source);
    assert_eq!(e.destinations.len(), 1);
    assert_eq!(e.destinations[0].pool, other);
    assert!(!e.review);
    h.click("join-destination-0");
    assert!(joining(h).review);
    assert!(join::current(&h.model, joining(h)));
    assert_eq!(
        h.try_click("join-submit"),
        Err("Enter the destination password.".into())
    );
    h.type_text("join-password", "synthetic-password");
    assert_eq!(h.try_click("join-submit"), Err(join::CONFIRM.into()));
    // Back drops the credentials.
    h.click("join-back");
    assert!(joining(h).password.is_empty());
    assert!(joining(h).destination.is_none());
    h.click("join-cancel");
    assert!(h.model.areas.pool.join.is_none());
    assert!(h.model.resource_operations.is_empty());
}
