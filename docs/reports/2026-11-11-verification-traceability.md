# Verification traceability

This report maps the requirements in [VERIFICATION.md](../VERIFICATION.md) to the evidence
that exists on 2026-11-11, at workspace version 0.18.0 after the
[0.17 checkpoint](2026-11-11-checkpoint-0.17.md). It adds no new results. Fixture tests are
the offline suite that CI runs; live evidence is the named scenario in the
[live qualification report](2026-11-09-lab-qualification.md), all on XCP-ng 8.3.0 in the
disposable lab. A fixture test is listed only where its body asserts the required
observation or a stated part of it; every listed test name was checked to exist.

Paths are relative to `crates/`: `core/tests/…` is `xenadmin-core/tests`, `core/src/…` is
`xenadmin-core/src`, `desktop/…` is `xenadmin-desktop/src`. The files under
`core/tests/fixtures` compile into the `direct_connection` test binary.

## Core scenarios

| ID | Fixture tests | Live evidence | Not yet shown |
| --- | --- | --- | --- |
| V01 | `direct_login_bootstrap_and_logout_need_no_xo` (core/tests/direct_connection.rs); `remembered_member_is_tried_when_primary_is_unreachable` (core/tests/connection_profiles.rs); `reconnect_is_ephemeral_cancels_same_pool_aliases_and_retains_other_pool` (desktop/profile_reconnect.rs) | Recovery gate through the packaged desktop client: cold start with XO shut down, a hand-typed endpoint and a session-only login; restarted instances then used the saved endpoint and pins | The workstation's internet access was not cut; no fixture does a cold start from a saved profile with an injected XO failure |
| V02 | `delayed_initial_inventory_reports_synchronizing_and_admits_no_command_until_ready` (core/tests/direct_connection.rs): with the first inventory held back the state is Synchronizing, submissions, controls and a console are refused as stale, no mutation reaches the server and no journal is created | None | A live run; the desktop's wording of the state |
| V03 | `changes_racing_the_initial_inventory_are_applied_in_order_from_consecutive_tokens`; `task_progress_does_not_cancel_the_inventory_long_poll` (core/tests/direct_connection.rs) | None | A live run of the race |
| V04 | `optional_classes_are_negotiated_without_losing_base_inventory` (core/src/xapi_inventory_tests.rs); `permission_changes_and_full_resync_remove_old_optional_data` (core/src/xapi_inventory.rs); `unsupported_denied_or_malformed_discovery_defers_to_server` (core/tests/fixtures/permissions.rs) | Inventory tables and certificate opt-out: 23 optional classes negotiated as available; finding 1: an unimplemented method catalogue treated as "cannot pre-verify" | A required class that is missing or denied withholding readiness with a reason |
| V05 | `session_expiry_resynchronizes_in_a_new_generation`; `an_established_session_follows_a_new_coordinator_in_a_new_generation`; `a_control_queued_across_a_reconnect_does_not_publish_ready_before_the_new_inventory` (core/tests/direct_connection.rs) | Coordinator handover round trip and the coordinator loss with HA rerun: the session followed the pool to its new coordinator with every member pinned; session closed on the host with no outage | A session that times out by itself: the live run closed a directory user's sessions by an administrator's call (the client signed in again in 1.5 s and followed events), and after the user's access was removed the client stopped with the refusal. The fixtures use plain HTTP, so they do not assert the destination's identity |
| V06 | `reconnecting_the_same_pool_rejects_the_previous_connection_observation` (core/tests/direct_connection.rs); `generation_change_stales_old_metrics_and_rejects_old_observations` (core/src/metrics_tests.rs); `external_move_reboot_and_generation_change_close_then_reopen_once` (desktop/console_recovery_tests.rs) | Coordinator power loss under a live session: a write refused while stale, Ready again under a new generation, no added or replayed records | A late event reply is now injected and shown to be discarded (`a_late_event_reply_from_the_previous_coordinator_is_not_applied_to_the_new_generation`, core/tests/direct_connection.rs); a late read and a late console frame are not |
| V07 | `late_task_receipts_survive_recovery_and_acknowledgement_without_old_worker_continuation` (core/tests/fixtures/cross_pool.rs); `late_vlan_task_receipts_survive_recovery_without_reviving_original_worker` (core/tests/fixtures/network_topology.rs); `late_task_evidence_survives_recovery_and_keeps_the_vm_reserved` (core/src/operations.rs) | Coordinator handover round trip: the record stayed unknown when the session was lost and the recheck on the new coordinator confirmed it without a replay | A protocol-level fixture that ends in reconciliation; the two protocol tests end in an unknown outcome |
| V08 | `malformed_batch_does_not_partially_modify_inventory`; `malformed_numbers_identify_the_field_and_leave_inventory_unchanged` (core/src/xapi.rs); four tests ending `_is_not_applied_and_its_token_is_never_sent_back` for a malformed batch, an undecodable batch, a batch over the object limit and a reply over the size limit (core/tests/direct_connection.rs) | None | A live host that sends such a batch, which the lab cannot make it do |
| V09 | `dropping_a_handle_does_not_disconnect_other_handles` (core/tests/connection_profiles.rs); `a_slow_or_closed_observer_does_not_hold_back_snapshots_or_operation_results` (core/tests/direct_connection.rs) | None | The desktop's own observers under load |
| V10 | `a_full_journal_refuses_the_next_admission_and_adds_nothing` (core/src/operations.rs); `a_full_journal_refuses_both_operation_families_and_adds_nothing` (core/src/resource_journal_tests.rs); `a_full_journal_refuses_the_submission_with_its_reason_and_sends_no_mutation` and `a_full_command_queue_refuses_the_submission_at_once_and_dispatches_nothing_for_it` (core/tests/direct_connection.rs) | None | The refusal appears on the returned operation only, not in the history; a desktop test that it is shown |
| V11 | `lost_mutation_reply_remains_unknown_and_blocks_a_second_mutation`; `lost_settings_response_reconciles_values_without_repeating_the_write` (core/tests/direct_connection.rs); `lost_create_reply_recovers_the_new_identity_using_the_saved_marker` (core/tests/fixtures/resource_administration.rs) | Coordinator handover round trip and the reinstalled host's handover: replies lost in the toolstack restart left the record unknown and the recheck confirmed it | A lost reply to a single setter, which completes too quickly on the lab to interrupt |
| V12 | `lost_mutation_reply_remains_unknown_and_blocks_a_second_mutation` (restart half); `late_task_evidence_survives_recovery_and_keeps_the_vm_reserved` (core/src/operations.rs); `recovery_never_replays_unsubmitted_steps_and_revokes_old_worker_revision` (core/src/resource_journal_tests.rs) | Interrupted snapshot create across a restart; description edit interrupted before dispatch; snapshot restore interrupted mid-workflow; clean shutdown and start interrupted by a client restart: none replayed | Nothing outstanding: a live start was cut between its dispatch and the saved task reference and stayed unknown, unrepeated, until acknowledged. The one early restore attempt that left no record is explained: its wait accepted a request not yet admitted |
| V13 | `snapshot_creation_verifies_result_identity_and_deduplicates_the_full_intent` (core/tests/direct_connection.rs); `journal_deduplicates_ids_and_reserves_unresolved_targets` (core/src/operations.rs); `id_deduplication_and_cross_family_identity_conflicts_do_not_create_second_operations` (core/src/resource_journal_tests.rs) | None | A live duplicate submission |
| V14 | `refusal_is_definite_but_missing_task_is_unknown` (core/src/migration_tests.rs); `removed_task_and_existing_guest_do_not_claim_success` (core/src/transfer_tests.rs) | Other-client task inspection and cancellation: the task vanished and the record stayed unknown, never success; halted guest move: polled fractions carried progress from 0 to 1 | A fixture asserting that polled progress reaches the record |
| V15 | `cancellation_reports_actual_terminal_status_and_never_destroys_the_original_task`; `accepted_request_is_not_completion_and_duplicate_task_cancellation_is_rejected` (core/tests/fixtures/activity.rs); `cancellation_after_reconnect_reaches_the_recovered_task` (core/tests/direct_connection.rs) | Other-client task inspection and cancellation: the migration ended Cancelled with its placement verified while the canceller's own record stayed unknown | A UI test that the three states read differently; a live cancel that loses to completion |
| V16 | `wrong_pin_and_untrusted_self_signed_certificate_withhold_credentials` (core/tests/tls_trust.rs); `inspected_pin_does_not_accept_a_certificate_rotated_before_login` (core/src/connection_trust_tests.rs); `reviewed_changed_pin_requires_exact_endpoint_and_explicit_fingerprint_check` (desktop/profile_trust_tests.rs) | Member certificate reset and trust replacement; second desktop session: the member's console refused with the stale pin, then reviewed and replaced | A fixture for a certificate change on the console endpoint |
| V17 | `member_recovery_route_binds_reviewed_host_before_following_coordinator_redirect` (core/tests/fixtures/management_network.rs); `unsafe_redirects_authentication_errors_oversize_and_malformed_data_are_sanitized`; `redirect_authority_validation_rejects_ports_paths_userinfo_and_scheme_changes` (core/src/metrics_tests.rs) | Login through a member: the redirect to an unpinned coordinator was refused by TLS before any credential; live migration: the destination's console tunnel was refused without that member's pin | A unit test of the console location's host check |
| V18 | `locked_and_malformed_stores_fail_without_falling_back_to_files`; `a_store_that_needs_no_password_is_unlocked_on_demand` (core/src/credentials.rs); `startup_attempts_only_opted_in_profiles_once_and_explains_locked_keyring` (desktop/profile_restore_tests.rs); `saved_password_results_are_bound_to_the_open_profile_and_failures_allow_manual_input` (desktop/profiles_view.rs) | Saved credentials with a real Secret Service provider: a locked store left the connection disconnected with the reason shown, and a typed session password then connected | No provider at all, in a fixture or live |
| V19 | `external_move_reboot_and_generation_change_close_then_reopen_once` (desktop/console_recovery_tests.rs), decision logic only | Console across a hard reboot; second desktop session: bounded reconnect attempts across a member reboot; console follows a guest migrated by another client; console lost under a held modifier by a migration and by a toolstack restart, with no key left held in the guest | A fixture that stale frames and input stop |
| V20 | `unsafe_redirects_authentication_errors_oversize_and_malformed_data_are_sanitized` (core/src/metrics_tests.rs): failed or stale series on a live connection | None | Management staying usable is now asserted for five metrics failures (`failed_metrics_leave_the_connection_ready_and_a_power_command_still_succeeds`, core/src/metrics_tests.rs); the chart's unavailable state in the desktop is not |
| V21 | `authenticated_aliases_share_one_inventory_and_keep_owner_alive` (core/tests/connection_profiles.rs); `same_pool_alias_routes_resource_commands_and_history_to_one_owner` (core/tests/fixtures/resource_administration.rs); `a_vm_observation_cannot_be_used_in_another_pool_with_the_same_vm_uuid` (core/tests/direct_connection.rs) | Login through a member and pool sharing: member and coordinator connections shared one session and revision | Nothing outstanding |
| V22 | `failed_attachment_retains_new_disk_as_explicit_partial_result` (core/src/device_lifecycle_tests.rs); `creation_failure_retains_vm_and_copied_disks_without_automatic_cleanup` (core/src/vm_lifecycle_tests.rs); `cleanup_requires_terminal_owned_creation_and_refuses_all_consumers` (core/src/disk_administration_tests.rs) | Guest provisioning, first attempt: the creation stopped at the memory step and left a visible device-less guest that the reviewed deletion later removed | Shared or externally changed resources surviving inside one failed composite creation |

The boundary injection the same section asks for (before admission, after durable
admission, before dispatch, after dispatch, after host acceptance, after response receipt,
each with the server's mutation count asserted) is covered for the lost-reply and restart
boundaries by the V11 and V12 tests. No test walks all six boundaries for one request.

## Console qualification

| Requirement | Evidence | Open |
| --- | --- | --- |
| Authenticated tunnel accepted by the RFB library on the actual host | Guest frames at 720 by 400 and 800 by 600 and the control domain's console at 640 by 384, through the core facade and the desktop | Which encodings and pseudo-encodings the host negotiated is not recorded |
| Resize the guest display and the client window; pointer coordinates at each scale | Pointer targets read back exactly from inside a guest with the frame fitted at 1.1975, at 0.75 after a window resize, at actual size after finding 43's fix, and under Xwayland | A guest that changes its own display size; the wheel's direction |
| Text in US and non-US layouts, modifier combinations, QEMU extended keys, Ctrl+Alt+Delete | Shifted characters through physical keys ran guest commands under Wayland and Xwayland; a Ctrl+Alt+Delete chord through the core facade rebooted a guest | A non-US layout; the desktop's Ctrl+Alt+Delete button; shortcut capture |
| Hold a key or button, change focus, close the tab, break the connection | A held key was released in the guest the moment the window lost focus, with no further press after finding 44's fix | A held pointer button; closing the console while input is held; the desktop window's handling when the connection breaks under a held key (through the core facade a held Shift left no key held in the guest after a migration and after a toolstack restart) |
| Framebuffer, cursor and resize encodings, with a baseline fallback | Fixture coverage in the vendored library and the adapter ([decision](../decisions/0003-console-protocol-adapter.md)) | Live negotiation results and cursor shapes |
| Clipboard opt-in and refusal without logging | Fixtures only; the operator found no exchange working (finding 54). Typing the clipboard as keys: the exact key sequence, the refusal and the stop in `authenticated_console_preserves_the_handshake_and_transmits_input`, the review in `clipboard_text_is_typed_only_after_a_review_that_never_shows_it`, and a lab guest that reported every printable character back | Any live exchange: the lab guests have no clipboard agent, and no guest with tools that implement it has been tried. Recorded as experimental |
| Several consoles while read-only API calls run | Fixtures for the independent console windows | A live run with more than one console open |

## Outage acceptance run

The eight steps were run on 2026-11-11 with the packaged 0.17.0 AppImage (the "Recovery gate
through the packaged desktop client" row).

| Step | Result | Caveat |
| --- | --- | --- |
| 1. Record pool, account, VM UUID and allowed operations | Recorded in the run record | The account is the lab's root account |
| 2. Make XO unavailable while keeping host access | The XO VM was shut down and XO refused connections | A shutdown. A later harness scenario detached the running appliance's NIC instead and repaired it through the direct connection; the packaged desktop was not used for that |
| 3. Cold-start the installed application | The AppImage, on an empty state directory, with a member endpoint typed by hand | On the development workstation in a nested compositor, not a clean system |
| 4. Authenticate, wait for inventory, locate the XO VM by UUID | Session-only login; the first eight characters of the UUID left that one VM | |
| 5. Start it and follow the result | One Start operation, Succeeded; VM running; XO answering again within 40 s | |
| 6. Open the console; keyboard and pointer input | Typed text appeared at the appliance's login prompt | The appliance's text console shows no pointer effect; pointer input was verified on another guest in the same session |
| 7. Restore XO; confirm the client never used it | XO was down throughout login, inventory and start; a socket watch saw the two pool hosts only | The watch samples four times a second and misses very short connections |
| 8. Save the sanitized outcome and restore the lab | Evidence kept outside the repository; lab back at its baseline | |

## Release checks

| Requirement | State |
| --- | --- |
| Clean Arch and Ubuntu 24.04 installation | Installed, rendered and started offline on untouched cloud images of both ([clean-install report](2026-11-11-clean-install.md)); the released 0.18.0 packages were installed, rendered and used for a read-only lab login on Ubuntu before publication ([release report](2026-11-11-release-0.18.md)), and the 0.19.0 packages likewise ([release report](2026-11-12-release-0.19.md)); nobody has used the window on a clean system |
| Wayland compositor and X11 window manager actually used | Nested Hyprland 0.56.2 as a Wayland client, and its Xwayland server as an X11 client; the operator's own Hyprland session as a Wayland client; the X.Org server with Openbox and no Wayland, in a virtual machine with injected input and synthetic data ([clean install](2026-11-11-clean-install.md)). No person has used an X session, and no host connection or console was tried under X.Org |
| Hardware rendering and one software Vulkan setup | Hardware Vulkan on the workstation; software Vulkan on the two clean images, headless and in a window under Weston |
| Theme contrast, keyboard-only operation, HiDPI | Open. Dialogs were driven by Tab and Space, with the clipped-control gap of finding 22 |
| File chooser | The portal's save dialog wrote a console screenshot in the second desktop session |
| Orca | Open, with the accessibility-tree gap of finding 6 |
| Performance measurements | A first synthetic measurement at 16 hosts and 2,000 guests ([baseline](2026-11-11-performance-baseline.md)); nothing against a live host |
| Report links the actual CI run and lab evidence | The [0.17 checkpoint](2026-11-11-checkpoint-0.17.md) and the release reports for [0.18.0](2026-11-11-release-0.18.md) and [0.19.0](2026-11-12-release-0.19.md) do |

## What this leaves

Updated later on 2026-11-11: V02 and V10, which had no test of any kind, now have fixture
tests, and V06 and V20 gained the assertions they lacked. V03, V08 and V09 then gained the
fixture tests for their stated observations, so every core scenario now has one. Writing
the V05 companion test found a defect: a control request left in the queue when a session
ended was answered by the next session before its inventory arrived, and that answer
reported the connection Ready with the previous generation's inventory. Such refusals now
carry the session's real state, and the test that failed on every run passes.

The live gaps that need the lab are narrower: a session that times out by itself (V05; a
session closed by an administrator is recorded), a late reply from an old connection
(V06), a fault in the network path to a running XO (a detached NIC is recorded) and
the failure paths listed in the acceptance record. The gaps that need a person at a real
seat or a clean machine are the console's layout, clipboard and cursor rows and every open
row of the release checks.
