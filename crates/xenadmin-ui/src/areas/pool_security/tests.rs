//! Tests of the area.
//!
//! Rune `pool_security_view.rs` had four tests:
//!
//! - `certificate_files_are_regular_bounded_and_errors_do_not_include_contents`:
//!   moved with `read_file`, unchanged.
//! - `compact_review_keeps_footer_visible_and_captured_identity_immutable`:
//!   the model half is
//!   `review_of_a_synthetic_pool_submits_nothing_and_keeps_the_captured_identity`;
//!   that the footer is inside a 513x652 window is the GPUI half
//!   (`notes/progress/gpui-tests-wanted.md`).
//! - `native_certificate_picker_discards_closed_or_retargeted_requests_and_cancel_preserves_selection`:
//!   `certificate_picker_discards_closed_or_retargeted_requests_and_cancel_preserves_selection`,
//!   through the file prompt of the harness.
//! - `late_certificate_validation_is_discarded_when_host_changes`: moved.
//!
//! `main.rs` `dialogs_outside_the_supported_core_carry_the_experimental_notice`:
//! this area's share is `every_dialog_carries_the_experimental_notice`.
//! `session_permissions_view.rs`: its three tests are the kernel's and the
//! power area's (no review of this area has a permission line); the display
//! is `users_tab_shows_the_session_and_the_security_state` and
//! `a_live_session_shows_what_it_is`.

use std::sync::mpsc;

use xenadmin_core::{
    DirectoryHostState, DirectorySubject, HostId, PoolSecurityOperation as SecurityOp, RoleId,
    SecurityOverview, SecurityRole, SubjectId,
};

use super::commands::{
    CERTIFICATE_INSTALL, CERTIFICATE_RESET, DIRECTORY_JOIN, DIRECTORY_LEAVE, REFRESH, SUBJECT_ADD,
    SUBJECT_REMOVE, SUBJECT_ROLE,
};
use super::security::{self, Editor, Kind};
use super::{DEMO, scenes};
use crate::testing::{Live, TempDir, poll_until};
use crate::{Harness, prelude::*};

const ALL_METHODS: [&str; 8] = [
    "pool.enable_external_auth",
    "pool.disable_external_auth",
    "subject.create",
    "subject.destroy",
    "subject.add_to_roles",
    "subject.remove_from_roles",
    "host.install_server_certificate",
    "host.reset_server_certificate",
];

fn editor(h: &Harness) -> &Editor {
    h.model
        .areas
        .pool_security
        .editor
        .as_ref()
        .expect("security state")
}

fn editor_mut(h: &mut Harness) -> &mut Editor {
    h.model
        .areas
        .pool_security
        .editor
        .as_mut()
        .expect("security state")
}

fn overview(h: &mut Harness) -> &mut SecurityOverview {
    editor_mut(h).overview.as_mut().expect("overview")
}

fn pool(h: &Harness) -> Target {
    Target::one(ObjectRef::pool(editor(h).pool))
}

fn first_host(h: &Harness) -> HostId {
    *editor(h).inventory.hosts.keys().next().expect("a host")
}

fn disabled(h: &Harness, command: CommandId) -> Option<String> {
    h.local_row()
        .into_iter()
        .find(|item| item.id == command)
        .unwrap_or_else(|| panic!("{} is not on the local row", command.0))
        .disabled
}

/// The scene's pool with every method offered, RBAC available, two roles
/// and two subjects: Alice (Pool Admin) and Bob (no role).
struct Directory {
    alice: SubjectId,
    bob: SubjectId,
    admin: RoleId,
    operator: RoleId,
}

fn directory(h: &mut Harness) -> Directory {
    let ids = Directory {
        alice: SubjectId::from(uuid::Uuid::new_v4()),
        bob: SubjectId::from(uuid::Uuid::new_v4()),
        admin: RoleId::from(uuid::Uuid::new_v4()),
        operator: RoleId::from(uuid::Uuid::new_v4()),
    };
    let v = overview(h);
    v.supported_methods = ALL_METHODS.map(String::from).to_vec();
    v.rbac_available = Some(true);
    v.roles = vec![
        SecurityRole {
            id: ids.admin,
            name: "Pool Admin".into(),
            description: String::new(),
        },
        SecurityRole {
            id: ids.operator,
            name: "Pool Operator".into(),
            description: String::new(),
        },
    ];
    v.subjects = vec![
        DirectorySubject {
            id: ids.alice,
            identifier: "S-1-5-21-1".into(),
            name: "alice@example.test".into(),
            roles: vec![ids.admin],
        },
        DirectorySubject {
            id: ids.bob,
            identifier: "S-1-5-21-2".into(),
            name: "bob@example.test".into(),
            roles: Vec::new(),
        },
    ];
    ids
}

fn fingerprint(byte: &str) -> xenadmin_core::CertificateFingerprint {
    byte.repeat(32).parse().expect("a fingerprint")
}

// -- Moved from Rune ----------------------------------------------------------

// Rune: pool_security_view.rs, the same name.
#[test]
fn certificate_files_are_regular_bounded_and_errors_do_not_include_contents() {
    let dir = TempDir::new("xenadmin-cert-files");
    let path = dir.path().join("key.pem");
    std::fs::write(&path, b"synthetic-private-key").unwrap();
    assert_eq!(
        security::read_file(&path, 64).unwrap(),
        "synthetic-private-key"
    );
    assert!(
        !security::read_file(&path, 4)
            .unwrap_err()
            .contains("synthetic")
    );
    let link = dir.path().join("link.pem");
    std::os::unix::fs::symlink(&path, &link).unwrap();
    assert!(security::read_file(&link, 64).is_err());
    assert!(security::read_file(dir.path(), 64).is_err());
}

// Rune: pool_security_view.rs, the same name.
#[test]
fn late_certificate_validation_is_discarded_when_host_changes() {
    let mut h = Harness::scene("pool-security");
    let (send, recv) = mpsc::channel();
    editor_mut(&mut h).file_result = Some(recv);
    let host = first_host(&h);
    security::select_host(&mut h.model, host);
    assert!(send.send(Err("stale validation".into())).is_err());
    assert!(editor(&h).certificate.is_none());
}

// Rune: compact_review_keeps_footer_visible_and_captured_identity_immutable,
// the model half.
#[test]
fn review_of_a_synthetic_pool_submits_nothing_and_keeps_the_captured_identity() {
    let mut h = Harness::scene("pool-security-review");
    assert_eq!(h.surface().id, "pool-security-join");
    let pool = editor(&h).pool;
    let captured = editor(&h).review.clone();
    assert!(captured.is_some());
    assert_eq!(h.try_click("security-confirm"), Err(DEMO.into()));
    // The typed UUID does not open a synthetic pool.
    h.type_text("security-confirmation", &pool.to_string());
    assert_eq!(h.try_click("security-confirm"), Err(DEMO.into()));
    // Nor does the handler behind the button.
    security::apply(&mut h.model);
    assert_eq!(editor(&h).error.as_deref(), Some(security::CHANGED));
    assert!(h.model.resource_operations.is_empty());
    assert_eq!(editor(&h).pool, pool);
    assert_eq!(editor(&h).review, captured);
    h.assert_unique_keys();
}

// Rune: native_certificate_picker_discards_closed_or_retargeted_requests_and_cancel_preserves_selection.
#[test]
fn certificate_picker_discards_closed_or_retargeted_requests_and_cancel_preserves_selection() {
    let mut h = Harness::scene("pool-security");
    let pool = editor(&h).pool;
    security::choose_file(&mut h.model, 0);
    h.answer_file(Some("/tmp/synthetic-cert.pem".into()));
    assert_eq!(
        editor(&h).files[0],
        Some("/tmp/synthetic-cert.pem".into())
    );
    // A cancel keeps the earlier choice.
    security::choose_file(&mut h.model, 0);
    h.answer_file(None);
    assert!(editor(&h).files[0].is_some());
    // The selection moved on while the prompt was open.
    security::choose_file(&mut h.model, 0);
    editor_mut(&mut h).file_revision += 1;
    h.answer_file(Some("/tmp/late-key.pem".into()));
    assert_eq!(
        editor(&h).files[0],
        Some("/tmp/synthetic-cert.pem".into())
    );
    // A review was staged meanwhile: a reply never changes what it shows.
    security::choose_file(&mut h.model, 0);
    security::stage(
        &mut h.model,
        SecurityOp::AddSubject {
            identifier: "S-1-5-21-9".into(),
        },
    );
    h.answer_file(Some("/tmp/under-review.pem".into()));
    assert_eq!(
        editor(&h).files[0],
        Some("/tmp/synthetic-cert.pem".into())
    );
    editor_mut(&mut h).review = None;
    // A prompt that could not be shown says so, to the editor that asked.
    security::choose_file(&mut h.model, 1);
    h.fail_file("no portal");
    assert_eq!(
        editor(&h).error.as_deref(),
        Some("The file chooser could not open the selected file")
    );
    assert!(editor(&h).files[1].is_none());
    // The editor was closed and opened again: the reply is for the old one.
    security::choose_file(&mut h.model, 0);
    assert!(security::open(&mut h.model, pool));
    h.answer_file(Some("/tmp/old-editor.pem".into()));
    assert!(editor(&h).files[0].is_none());
}

// -- The tab ------------------------------------------------------------------

#[test]
fn users_tab_shows_the_session_and_the_security_state() {
    let h = Harness::scene("pool-security");
    assert!(h.surfaces().is_empty());
    assert_eq!(h.model.active_tab(), TabId::Users);
    // The demo has no session: the Rune line says why nothing is known.
    let session = h.node(permissions::SESSION_KEY);
    assert_eq!(
        session.text.as_deref(),
        Some(xenadmin_core::SessionPermissions::default().summary().as_str())
    );
    assert_eq!(
        h.node("security-session").text.as_deref(),
        Some("Local superuser session · security changes available")
    );
    assert!(h.find("security-reconnect").is_none());
    assert_eq!(
        h.node("security-rbac").text.as_deref(),
        Some(
            "Role-based access availability is unknown. Adding subjects and changing roles are disabled."
        )
    );
    assert!(h.find("security-preview").is_some());
    assert!(h.find("security-error").is_none());
    let dump = h.dump();
    assert_eq!(dump.matches("Local authentication").count(), 3, "{dump}");
    assert!(dump.contains("No directory user or group has access to this pool."));
    h.assert_unique_keys();

    let row: Vec<_> = h.local_row().into_iter().map(|item| item.id).collect();
    assert_eq!(
        row,
        [
            REFRESH,
            DIRECTORY_JOIN,
            DIRECTORY_LEAVE,
            SUBJECT_ADD,
            SUBJECT_ROLE,
            SUBJECT_REMOVE
        ]
    );
    // What the scene's overview allows: joining, nothing else.
    assert_eq!(disabled(&h, DIRECTORY_JOIN), None);
    assert_eq!(
        disabled(&h, DIRECTORY_LEAVE).as_deref(),
        Some("This pool does not offer pool.disable_external_auth to this session.")
    );
    assert_eq!(
        disabled(&h, SUBJECT_ADD).as_deref(),
        Some("This pool does not offer subject.create to this session.")
    );
    assert_eq!(
        disabled(&h, SUBJECT_ROLE).as_deref(),
        Some(security::NO_SUBJECT)
    );
    assert_eq!(
        disabled(&h, SUBJECT_REMOVE).as_deref(),
        Some(security::NO_SUBJECT)
    );
    // A synthetic pool cannot be asked.
    assert_eq!(
        disabled(&h, REFRESH).as_deref(),
        Some("Reconnect and open the Users tab again to refresh its pool identity")
    );
}

#[test]
fn a_session_that_is_not_the_local_superuser_can_change_nothing() {
    let mut h = Harness::scene("pool-security");
    directory(&mut h);
    overview(&mut h).is_local_superuser = false;
    assert_eq!(
        h.node("security-session").text.as_deref(),
        Some(security::READ_ONLY)
    );
    for command in [DIRECTORY_JOIN, DIRECTORY_LEAVE, SUBJECT_ADD] {
        assert_eq!(
            disabled(&h, command).as_deref(),
            Some(security::READ_ONLY),
            "{}",
            command.0
        );
    }
    let host = Target::one(ObjectRef::host(editor(&h).pool, first_host(&h)));
    for command in [CERTIFICATE_INSTALL, CERTIFICATE_RESET] {
        assert_eq!(
            h.model.eligibility(command, &host),
            Eligibility::disabled(security::READ_ONLY)
        );
    }
    // The demo cannot reconnect: the button, where the connections area
    // offers its command for this pool, never runs here.
    if h.find("security-reconnect").is_some() {
        assert!(h.try_click("security-reconnect").is_err());
        assert!(h.model.areas.pool_security.editor.is_some());
    }
}

#[test]
fn section_errors_and_directory_state_gate_joining_and_leaving() {
    let mut h = Harness::scene("pool-security");
    directory(&mut h);
    // Every member on local authentication: join, not leave.
    assert_eq!(disabled(&h, DIRECTORY_JOIN), None);
    assert_eq!(
        disabled(&h, DIRECTORY_LEAVE).as_deref(),
        Some(security::NOT_JOINED)
    );
    for host in &mut overview(&mut h).directory_hosts {
        host.auth_type = "AD".into();
        host.service_name = "example.test".into();
    }
    assert_eq!(
        disabled(&h, DIRECTORY_JOIN).as_deref(),
        Some(security::ALREADY_JOINED)
    );
    assert_eq!(disabled(&h, DIRECTORY_LEAVE), None);
    assert!(h.dump().contains("example.test"));
    // The directory section could not be read: neither.
    overview(&mut h)
        .errors
        .insert("directory".into(), "host unreachable".into());
    assert_eq!(
        h.node("security-section-error-directory").text.as_deref(),
        Some("directory: host unreachable")
    );
    for command in [DIRECTORY_JOIN, DIRECTORY_LEAVE] {
        assert_eq!(
            disabled(&h, command).as_deref(),
            Some(security::HOSTS_UNKNOWN)
        );
    }
}

#[test]
fn commands_are_hidden_for_other_kinds_and_unread_state_says_so() {
    let mut h = Harness::scene("pool-security");
    let pool = pool(&h);
    let host = Target::one(ObjectRef::host(editor(&h).pool, first_host(&h)));
    for command in [
        REFRESH,
        DIRECTORY_JOIN,
        DIRECTORY_LEAVE,
        SUBJECT_ADD,
        SUBJECT_ROLE,
        SUBJECT_REMOVE,
    ] {
        assert_eq!(h.model.eligibility(command, &host), Eligibility::Hidden);
    }
    for command in [CERTIFICATE_INSTALL, CERTIFICATE_RESET] {
        assert_eq!(h.model.eligibility(command, &pool), Eligibility::Hidden);
    }
    // Nothing read (the tab was never shown): the reason a query would get.
    h.model.areas.pool_security.editor = None;
    assert_eq!(
        h.model.eligibility(DIRECTORY_JOIN, &pool),
        Eligibility::disabled(security::REOPEN)
    );
    assert_eq!(
        h.node("security-unread").text.as_deref(),
        Some(security::REOPEN)
    );
    assert_eq!(
        h.model.eligibility(CERTIFICATE_RESET, &host),
        Eligibility::disabled(security::REOPEN)
    );
}

// -- The directory ------------------------------------------------------------

#[test]
fn joining_asks_for_credentials_then_reviews_with_the_pool_uuid_to_type() {
    let mut h = Harness::scene("pool-security");
    let pool = editor(&h).pool;
    h.click(DIRECTORY_JOIN.0);
    assert_eq!(h.surface().id, "pool-security-join");
    assert_eq!(h.surface().title, "Join Active Directory");
    assert!(h.find("security-back").is_none());
    h.click("security-review-join");
    assert_eq!(
        h.node("security-error").text.as_deref(),
        Some("Enter directory credentials for this change")
    );
    assert!(editor(&h).review.is_none());
    h.type_text("security-domain", "  example.test ");
    h.type_text("security-account", "directory-admin");
    h.type_text("security-password", "synthetic-password");
    h.click("security-review-join");
    let hosts = editor(&h).overview.as_ref().unwrap().directory_hosts.clone();
    assert_eq!(
        editor(&h).review,
        Some(SecurityOp::JoinDirectory {
            domain: "example.test".into(),
            expected_hosts: hosts,
        })
    );
    assert!(h.find("security-error").is_none());
    assert!(
        h.has_text("Join 3 members to example.test using directory account directory-admin.")
    );
    assert!(h.has_text("Host authentication changes. A partial failure can leave members configured differently. No automatic leave or rollback is performed."));
    assert_eq!(
        h.node("security-confirmation-prompt").text,
        Some(format!("Type the pool UUID to confirm: {pool}"))
    );
    // The password is nowhere in the description.
    assert!(!h.dump().contains("synthetic-password"));
    assert_eq!(h.try_click("security-confirm"), Err(DEMO.into()));
    // Back keeps the draft; Close drops the secret and leaves the tab.
    h.click("security-back");
    assert!(editor(&h).review.is_none());
    assert_eq!(editor(&h).domain, "  example.test ");
    h.click("security-close");
    assert!(h.surfaces().is_empty());
    assert!(editor(&h).password.is_empty());
    assert!(editor(&h).overview.is_some());
}

#[test]
fn the_uuid_typed_into_a_review_is_compared_exactly() {
    let mut h = Harness::scene("pool-security-review");
    h.model.demo = false;
    // No connection stands behind the scene: the review is stale, and says
    // so before the confirmation is looked at.
    assert_eq!(
        h.try_click("security-confirm"),
        Err(security::CHANGED.into())
    );
    assert_eq!(
        h.node("security-review-stale").text.as_deref(),
        Some("The connection changed; reopen this dialog")
    );
    h.model.demo = true;
    let pool = editor(&h).pool.to_string();
    let confirm = |h: &Harness| super::view::surfaces(&h.model)[0].footer[2].disabled.clone();
    // Past the demo, the only thing between the review and its submission.
    assert_eq!(confirm(&h).as_deref(), Some(DEMO));
    h.model.demo = false;
    assert_eq!(confirm(&h).as_deref(), Some(security::CHANGED));
    // `apply` makes the same checks in the same order (Rune).
    h.type_text("security-confirmation", &format!(" {pool}"));
    security::apply(&mut h.model);
    assert_eq!(editor(&h).error.as_deref(), Some(security::CHANGED));
    assert!(h.model.resource_operations.is_empty());
}

#[test]
fn leaving_reviews_with_or_without_removing_machine_accounts() {
    let mut h = Harness::scene("pool-security");
    directory(&mut h);
    for host in &mut overview(&mut h).directory_hosts {
        host.auth_type = "AD".into();
    }
    h.click(DIRECTORY_LEAVE.0);
    assert_eq!(h.surface().id, "pool-security-leave");
    h.click("security-review-leave");
    assert!(matches!(
        editor(&h).review,
        Some(SecurityOp::LeaveDirectory {
            remove_machine_accounts: false,
            ..
        })
    ));
    assert!(h.has_text("Leave Active Directory on 3 members. Remove directory machine accounts: no; directory cleanup remains manual."));
    assert!(h.has_text("Directory users will lose access. Keep local root credentials available. No automatic rollback is performed."));
    h.click("security-back");
    // Removing the accounts needs the directory account.
    h.click("security-review-leave-accounts");
    assert_eq!(
        h.node("security-error").text.as_deref(),
        Some(security::CREDENTIALS)
    );
    h.type_text("security-account", "directory-admin");
    h.type_text("security-password", "synthetic-password");
    h.click("security-review-leave-accounts");
    assert!(matches!(
        editor(&h).review,
        Some(SecurityOp::LeaveDirectory {
            remove_machine_accounts: true,
            ..
        })
    ));
    assert!(h.has_text("Leave Active Directory on 3 members. Remove directory machine accounts: yes, using supplied credentials."));
}

// -- Subjects and roles -------------------------------------------------------

#[test]
fn the_subject_grid_filters_selects_and_feeds_its_row_menu() {
    let mut h = Harness::scene("pool-security");
    let ids = directory(&mut h);
    let dump = h.dump();
    assert!(dump.contains("alice@example.test") && dump.contains("bob@example.test"));
    assert!(dump.contains("Pool Admin"));
    // The filter is Rune's: name or identifier, without regard to case.
    h.type_text("security-subject-filter", "ALICE");
    let dump = h.dump();
    assert!(dump.contains("alice@example.test") && !dump.contains("bob@example.test"));
    h.type_text("security-subject-filter", "21-2");
    assert!(h.dump().contains("bob@example.test"));
    h.type_text("security-subject-filter", "nobody");
    assert!(h.dump().contains("No directory user or group matches."));
    h.type_text("security-subject-filter", "");

    h.select_row("security-subjects", &ids.alice.to_string());
    assert_eq!(editor(&h).subject, Some(ids.alice));
    let menu: Vec<_> = h
        .row_menu("security-subjects")
        .into_iter()
        .map(|item| (item.id, item.disabled))
        .collect();
    assert_eq!(menu, [(SUBJECT_ROLE, None), (SUBJECT_REMOVE, None)]);
    h.escape_in_table("security-subjects");
    assert_eq!(editor(&h).subject, None);
    assert_eq!(
        disabled(&h, SUBJECT_REMOVE).as_deref(),
        Some(security::NO_SUBJECT)
    );
}

#[test]
fn removing_a_subject_opens_on_its_review_and_has_no_back() {
    let mut h = Harness::scene("pool-security");
    let ids = directory(&mut h);
    h.select_row("security-subjects", &ids.alice.to_string());
    h.click(SUBJECT_REMOVE.0);
    assert_eq!(h.surface().id, "pool-security-subject-remove");
    assert_eq!(
        editor(&h).review,
        Some(SecurityOp::RemoveSubject {
            subject: ids.alice,
            expected_identifier: "S-1-5-21-1".into(),
            expected_roles: vec![ids.admin],
        })
    );
    assert!(h.has_text(&format!(
        "Remove S-1-5-21-1 ({}) from pool access and close the sessions opened under it. Sessions of a removed group's members stay open until the host next checks them.",
        ids.alice
    )));
    assert!(h.find("security-back").is_none());
    assert_eq!(h.try_click("security-confirm"), Err(DEMO.into()));
    h.dismiss();
    assert!(h.surfaces().is_empty());
    // The choice in the grid is the tab's and stays.
    assert_eq!(editor(&h).subject, Some(ids.alice));
}

#[test]
fn a_role_change_is_one_button_per_role_as_in_rune() {
    let mut h = Harness::scene("pool-security");
    let ids = directory(&mut h);
    h.select_row("security-subjects", &ids.alice.to_string());
    h.click(SUBJECT_ROLE.0);
    assert_eq!(h.surface().id, "pool-security-subject-role");
    let admin = format!("security-role-{}", ids.admin);
    let operator = format!("security-role-{}", ids.operator);
    assert_eq!(h.node(&admin).label.as_deref(), Some("Remove Pool Admin"));
    assert_eq!(
        h.node(&operator).label.as_deref(),
        Some("Grant Pool Operator")
    );
    // The pool does not let this session take a role away.
    overview(&mut h)
        .supported_methods
        .retain(|m| m != "subject.remove_from_roles");
    assert_eq!(
        h.try_click(&admin),
        Err("This pool does not offer subject.remove_from_roles to this session.".into())
    );
    h.click(&operator);
    assert_eq!(
        editor(&h).review,
        Some(SecurityOp::ChangeSubjectRole {
            subject: ids.alice,
            expected_identifier: "S-1-5-21-1".into(),
            expected_roles: vec![ids.admin],
            role: ids.operator,
            grant: true,
        })
    );
    // The heading says which of the two changes this is.
    assert!(h.dump().contains("Grant subject role"));
    assert!(h.has_text(&format!(
        "Grant role Pool Operator ({}) for S-1-5-21-1 ({}).",
        ids.operator, ids.alice
    )));
    h.click("security-back");
    assert!(h.find(&operator).is_some());
    // Without RBAC known to be available no role can be changed.
    h.click("security-close");
    overview(&mut h).rbac_available = None;
    assert_eq!(
        disabled(&h, SUBJECT_ROLE).as_deref(),
        Some(security::RBAC_UNKNOWN)
    );
}

#[test]
fn adding_a_subject_needs_rbac_and_an_identity_resolved_for_the_typed_name() {
    let mut h = Harness::scene("pool-security");
    directory(&mut h);
    overview(&mut h).rbac_available = Some(false);
    assert_eq!(
        disabled(&h, SUBJECT_ADD).as_deref(),
        Some("Role-based access is restricted. Adding subjects is disabled because the server may assign full administration automatically.")
    );
    overview(&mut h).rbac_available = None;
    assert_eq!(
        disabled(&h, SUBJECT_ADD).as_deref(),
        Some(security::RBAC_UNKNOWN)
    );
    overview(&mut h).rbac_available = Some(true);
    h.click(SUBJECT_ADD.0);
    assert_eq!(h.surface().id, "pool-security-subject-add");
    assert_eq!(
        h.try_click("security-resolve"),
        Err("Enter a directory user or group.".into())
    );
    assert_eq!(
        h.try_click("security-review-add"),
        Err(security::UNRESOLVED.into())
    );
    h.type_text("security-subject-name", "carol");
    // A synthetic pool cannot be asked: the Rune refusal of `query`.
    h.click("security-resolve");
    assert_eq!(
        h.node("security-error").text.as_deref(),
        Some(security::REOPEN)
    );
    // The directory answered for this text.
    editor_mut(&mut h).resolved = Some(("carol".into(), "S-1-5-21-3".into(), "Carol".into()));
    assert_eq!(
        h.node("security-resolved").text.as_deref(),
        Some("Resolved: Carol · S-1-5-21-3")
    );
    // An answer for another text is not an answer.
    h.type_text("security-subject-name", "caro");
    assert!(h.find("security-resolved").is_none());
    assert_eq!(
        h.try_click("security-review-add"),
        Err(security::UNRESOLVED.into())
    );
    h.type_text("security-subject-name", "carol");
    h.click("security-review-add");
    assert_eq!(
        editor(&h).review,
        Some(SecurityOp::AddSubject {
            identifier: "S-1-5-21-3".into()
        })
    );
    assert!(h.has_text("Add directory identity S-1-5-21-3 with no roles. Grant a role separately after successful creation."));
    assert!(h.has_text("Servers with unavailable or restricted RBAC are refused because they may assign full administration automatically."));
}

// -- Certificates -------------------------------------------------------------

#[test]
fn certificate_changes_are_of_one_server_and_wait_for_its_fingerprint() {
    let mut h = Harness::scene("pool-security");
    let host = first_host(&h);
    let name = editor(&h).inventory.hosts[&host].name.clone();
    let target = Target::one(ObjectRef::host(editor(&h).pool, host));
    // The scene's pool offers the reset only.
    assert_eq!(
        h.model.eligibility(CERTIFICATE_INSTALL, &target),
        Eligibility::disabled(
            "This pool does not offer host.install_server_certificate to this session."
        )
    );
    h.invoke_on(CERTIFICATE_RESET.0, &target);
    assert_eq!(h.surface().id, "pool-security-certificate-reset");
    assert_eq!(h.surface().subtitle, Some(format!("{name} ({host})")));
    assert_eq!(editor(&h).host, Some(host));
    assert_eq!(
        h.try_click("security-review-reset"),
        Err(security::NO_FINGERPRINT.into())
    );
    editor_mut(&mut h).fingerprint = Some((host, fingerprint("ab")));
    let current = fingerprint("ab");
    assert_eq!(
        h.node("security-current-fingerprint").text,
        Some(format!("Current SHA-256: {current}"))
    );
    assert!(h.find("security-choose-certificate").is_none());
    h.click("security-review-reset");
    assert_eq!(
        editor(&h).review,
        Some(SecurityOp::ResetCertificate {
            host,
            expected: current.clone(),
        })
    );
    assert!(h.has_text(&format!(
        "Replace host {host} certificate ({current}) with a new self-signed certificate."
    )));
    assert!(h.has_text("Connections may close. Reconnect and verify the new certificate independently. Saved trust is not automatically changed."));
    h.click("security-close");
    assert!(editor(&h).host.is_none() && editor(&h).fingerprint.is_none());

    // Installing: the files first, then their validation.
    overview(&mut h)
        .supported_methods
        .push("host.install_server_certificate".into());
    h.invoke_on(CERTIFICATE_INSTALL.0, &target);
    assert_eq!(h.surface().id, "pool-security-certificate-install");
    editor_mut(&mut h).fingerprint = Some((host, fingerprint("ab")));
    for (key, label) in [
        ("security-choose-certificate", "Choose certificate…"),
        ("security-choose-key", "Choose private key…"),
        ("security-choose-chain", "Choose optional chain…"),
    ] {
        assert_eq!(h.node(key).label.as_deref(), Some(label));
    }
    assert_eq!(
        h.try_click("security-validate"),
        Err("Choose a certificate file and a private key file.".into())
    );
    assert_eq!(
        h.try_click("security-review-install"),
        Err(security::NO_CERTIFICATE.into())
    );
    h.click("security-choose-key");
    h.answer_file(Some("/tmp/synthetic/host-key.pem".into()));
    assert_eq!(
        h.node("security-file-1").text.as_deref(),
        Some("host-key.pem")
    );
    // A fingerprint of another server is not this server's.
    let other = *editor(&h).inventory.hosts.keys().nth(1).expect("a second host");
    editor_mut(&mut h).fingerprint = Some((other, fingerprint("cd")));
    assert!(h.find("security-choose-certificate").is_none());
    h.assert_unique_keys();
}

#[test]
fn the_install_review_keeps_the_key_out_of_the_operation() {
    let h = Harness::scene("pool-security");
    let host = first_host(&h);
    let name = editor(&h).inventory.hosts[&host].name.clone();
    let op = SecurityOp::InstallCertificate {
        host,
        expected: fingerprint("ab"),
        fingerprint: fingerprint("cd"),
    };
    let lines = security::review_lines(editor(&h), &op);
    assert_eq!(lines[0], format!("Host: {name} ({host})"));
    assert_eq!(
        lines[1],
        format!(
            "Current SHA-256: {}\nReplacement SHA-256: {}",
            fingerprint("ab"),
            fingerprint("cd")
        )
    );
    // AREAS decision 10: the Rune text said "in operation history".
    assert_eq!(
        lines[2],
        "The key and certificate are sent directly to this pool, never saved in the operation's entry on the Events tab of the Notifications view. Connections may close. Reconnect and verify trust independently; saved trust is not automatically changed."
    );
}

// -- Notice, hooks, staleness -------------------------------------------------

// Rune: main.rs dialogs_outside_the_supported_core_carry_the_experimental_notice,
// this area's share (`experimental_dialog_open` names `pool_security`).
#[test]
fn every_dialog_carries_the_experimental_notice() {
    let mut h = Harness::scene("pool-security");
    let ids = directory(&mut h);
    let pool = editor(&h).pool;
    let host = first_host(&h);
    editor_mut(&mut h).subject = Some(ids.bob);
    let mut seen = Vec::new();
    for kind in Kind::ALL {
        security::begin(&mut h.model, pool, kind, kind.certificate().then_some(host));
        let surface = h.surface();
        assert_eq!(surface.id, kind.surface_id());
        assert_eq!(surface.title, kind.title());
        assert!(surface.experimental, "{kind:?}");
        assert!(h.find("security-close").is_some());
        h.assert_unique_keys();
        seen.push(surface.id.clone());
        h.dismiss();
        assert!(h.surfaces().is_empty(), "{kind:?}");
    }
    seen.dedup();
    assert_eq!(seen.len(), Kind::ALL.len());
}

#[test]
fn the_editor_follows_the_users_tab_and_a_dialog_keeps_it() {
    let mut h = Harness::scene("pool-security");
    let pool = editor(&h).pool;
    h.open_tab(TabId::General);
    assert!(h.model.areas.pool_security.editor.is_none());
    // Shown again: opened for the pool, and a synthetic pool is not asked.
    h.open_tab(TabId::Users);
    assert_eq!(editor(&h).pool, pool);
    assert!(editor(&h).overview.is_none() && editor(&h).error.is_none());
    assert!(h.find("security-preview").is_some());
    h.tick(0.1);
    assert_eq!(editor(&h).pool, pool);
    // A dialog keeps the editor while the tab is another one...
    security::begin(&mut h.model, pool, Kind::Join, None);
    h.model.set_active_tab(TabId::General);
    h.tick(0.1);
    assert_eq!(h.surface().id, "pool-security-join");
    // ...and takes it along when it closes there.
    h.dismiss();
    assert!(h.model.areas.pool_security.editor.is_none());
}

#[test]
fn a_scope_change_and_a_session_change_close_everything() {
    let mut h = Harness::scene("pool-security-review");
    let host = Target::one(ObjectRef::host(editor(&h).pool, first_host(&h)));
    h.model.select(host);
    assert!(h.model.areas.pool_security.editor.is_none());
    assert!(h.surfaces().is_empty());
    // AREAS decision 11: the session changes, the editor goes.
    let mut h = Harness::scene("pool-security-review");
    (super::spec().on_session_change)(&mut h.model);
    assert!(h.model.areas.pool_security.editor.is_none());
    // The demo loaded again is a session change too.
    let mut h = Harness::scene("pool-security-review");
    h.model.load_demo(3, 48);
    assert!(h.surfaces().is_empty());
}

#[test]
fn a_connection_change_invalidates_once_and_drops_the_secrets() {
    let mut h = Harness::scene("pool-security");
    let pool = editor(&h).pool;
    security::begin(&mut h.model, pool, Kind::Join, None);
    h.type_text("security-password", "synthetic-password");
    let (_send, recv) = mpsc::channel();
    editor_mut(&mut h).file_result = Some(recv);
    // Not the demo any more, and nothing observes the pool.
    h.model.demo = false;
    assert!(security::poll(&mut h.model));
    let e = editor(&h);
    assert!(e.password.is_empty() && e.file_result.is_none() && e.query.is_none());
    // AREAS decision 10: the Rune text said "reopen security".
    assert_eq!(
        e.error.as_deref(),
        Some("Connection changed; close and reopen this dialog before continuing")
    );
    editor_mut(&mut h).error = None;
    assert!(!security::poll(&mut h.model));
    assert!(editor(&h).error.is_none());
    // Refresh cannot ask either, and says why.
    h.click("security-close");
    assert_eq!(
        disabled(&h, REFRESH).as_deref(),
        Some("Reconnect and open the Users tab again to refresh its pool identity")
    );
}

#[test]
fn the_scene_function_is_the_rune_scene() {
    let mut m = Model::fixture();
    scenes::scene(&mut m, false);
    let e = m.areas.pool_security.editor.as_ref().expect("the editor");
    assert!(e.dialog.is_none() && e.review.is_none() && e.error.is_none());
    let v = e.overview.as_ref().expect("the synthetic overview");
    assert!(v.is_local_superuser);
    assert_eq!(
        v.supported_methods,
        ["pool.enable_external_auth", "host.reset_server_certificate"]
    );
    assert_eq!(v.directory_hosts.len(), e.inventory.hosts.len());
    assert!(
        v.directory_hosts
            .iter()
            .all(|h: &DirectoryHostState| h.auth_type.is_empty())
    );
    scenes::scene(&mut m, true);
    let e = m.areas.pool_security.editor.as_ref().expect("the editor");
    assert_eq!(e.dialog, Some(Kind::Join));
    assert_eq!(e.username, "directory-admin");
    assert!(e.password.is_empty());
    assert!(matches!(
        &e.review,
        Some(SecurityOp::JoinDirectory { domain, .. }) if domain == "example.test"
    ));
}

// -- Against the fixture server -----------------------------------------------

/// A live pool on its Users tab, with the overview a test wants put in by
/// hand (the fixture server serves no security query).
fn live(superuser: bool) -> Live {
    let mut live = Live::with_vm();
    live.h.select(Target::one(ObjectRef::pool(live.pool)));
    live.h.open_tab(TabId::Users);
    let host = live.host().host_id().expect("the host");
    let e = editor_mut(&mut live.h);
    assert!(e.observed.is_some(), "a live pool is observed");
    e.query = None;
    e.error = None;
    e.overview = Some(SecurityOverview {
        is_local_superuser: superuser,
        supported_methods: ALL_METHODS.map(String::from).to_vec(),
        directory_hosts: vec![DirectoryHostState {
            host,
            auth_type: String::new(),
            service_name: String::new(),
        }],
        rbac_available: Some(true),
        ..Default::default()
    });
    live
}

#[test]
fn a_live_session_shows_what_it_is() {
    let mut live = Live::with_vm();
    live.h.select(Target::one(ObjectRef::pool(live.pool)));
    live.h.open_tab(TabId::Users);
    // Showing the tab asked the pool.
    let e = editor(&live.h);
    assert_eq!(e.pool, live.pool);
    assert!(e.observed.is_some());
    assert!(e.query.is_some() || e.error.is_some());
    assert!(live.h.find("security-preview").is_none());
    let permissions = live
        .h
        .model
        .snapshot_for(live.pool)
        .expect("a snapshot")
        .session_permissions();
    let known = permissions.is_local_superuser() || permissions.permission_count().is_some();
    if known {
        assert_eq!(
            live.h.node("session-permission-summary").value.as_deref(),
            Some(permissions.summary().as_str())
        );
    } else {
        assert_eq!(
            live.h.node(permissions::SESSION_KEY).text.as_deref(),
            Some(permissions.summary().as_str())
        );
    }
}

#[test]
fn a_confirmed_change_is_handed_over_once_and_the_dialog_closes() {
    let mut live = live(true);
    let pool = live.pool;
    let h = &mut live.h;
    h.click(DIRECTORY_JOIN.0);
    h.type_text("security-domain", "example.test");
    h.type_text("security-account", "directory-admin");
    h.type_text("security-password", "synthetic-password");
    h.click("security-review-join");
    assert!(h.find("security-review-stale").is_none());
    // Only the pool's UUID, exactly, opens the confirmation.
    assert_eq!(
        h.try_click("security-confirm"),
        Err("Enter the pool UUID shown in the review".into())
    );
    h.type_text("security-confirmation", &format!("{pool} "));
    assert_eq!(
        h.try_click("security-confirm"),
        Err(security::CONFIRMATION.into())
    );
    h.type_text("security-confirmation", &pool.to_string());
    h.click("security-confirm");
    assert!(h.surfaces().is_empty(), "{:?}", editor(h).error);
    assert_eq!(h.model.resource_operations.len(), 1);
    let e = editor(h);
    assert!(e.pending.is_some() && e.review.is_none() && e.dialog.is_none());
    assert!(e.password.is_empty() && e.confirmation.is_empty());
    // The tab is still there; nothing is submitted twice.
    security::apply(&mut h.model);
    assert_eq!(h.model.resource_operations.len(), 1);
    // When the operation has ended the overview is asked for again.
    poll_until(h, "the change ended and the tab asked again", |h| {
        h.model
            .areas
            .pool_security
            .editor
            .as_ref()
            .is_some_and(|e| e.pending.is_none())
    });
}

#[test]
fn reconnect_as_takes_over_and_this_area_lets_go() {
    let mut live = live(false);
    let h = &mut live.h;
    assert_eq!(
        h.node("security-session").text.as_deref(),
        Some(security::READ_ONLY)
    );
    let button = h.node("security-reconnect");
    assert_eq!(
        button.label.as_deref(),
        Some("Reconnect as local administrator…")
    );
    assert_eq!(button.disabled, None);
    h.click("security-reconnect");
    // AREAS decision 11, before any tick: the editor is closed and the
    // connections area's surface is in front.
    assert!(h.model.areas.pool_security.editor.is_none());
    assert!(!h.surface().id.starts_with("pool-security"));
}
