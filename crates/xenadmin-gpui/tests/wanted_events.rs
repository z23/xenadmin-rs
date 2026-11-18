//! The behaviour wishes of the area engineers (`gpui-tests-wanted.md`): what
//! a press, typed text, a key, a secondary press or a file prompt does, with
//! real events on the Kit's headless test platform. One function per wish,
//! named after the Rune test whose window half it is (where the wish names
//! one). The presence and reachability wishes are the table of `wanted.rs`.
//!
//! The Rune client's compact size, 513 by 652, is 800 by 600 here: the
//! smallest size the main window can have. These tests do not establish
//! Wayland input, the clipboard or a screen reader.
#![cfg(feature = "scenes")]

mod common;

use common::Step::*;
use common::{
    BOTH, SMALL, Step, WIDE, close, described, front, model, on, open_scene, run_steps, submitted,
    surfaces,
};
use xenadmin_gpui::kit::testing::TestAppContext;

/// Runs the steps on a scene at each size; a failure names the size.
#[track_caller]
fn at_sizes(cx: &mut TestAppContext, scene: &str, sizes: &[(f32, f32)], steps: &[Step]) {
    for size in sizes {
        let window = open_scene(cx, scene, *size);
        if let Err(error) = run_steps(cx, &window, steps) {
            panic!("{scene} at {}x{}: {error}", size.0, size.1);
        }
        close(cx, window);
    }
}

/// The button is inside the window at both sizes, and a press on it with
/// the pointer submits nothing, opens and closes nothing and leaves the
/// review as it was: the demo cannot change a host.
#[track_caller]
fn a_press_submits_nothing(cx: &mut TestAppContext, scene: &str, key: &'static str) {
    for size in BOTH {
        let window = open_scene(cx, scene, *size);
        let before = surfaces(cx, &window);
        let result = on(cx, &window, |window, _| {
            let scope = before.last().map(String::as_str);
            let element = common::locate(window, scope, key)?;
            common::inside(window, &element)
        })
        .and_then(|()| run_steps(cx, &window, &[Inert(key)]));
        if let Err(error) = result {
            panic!("{scene} at {}x{}: {error}", size.0, size.1);
        }
        assert_eq!(submitted(cx, &window), 0, "{scene}: an operation exists");
        close(cx, window);
    }
}

/// One test per review whose wish is "pressing the button submits nothing".
macro_rules! submits_nothing {
    ($($(#[$meta:meta])* $name:ident: $scene:literal, $key:literal;)*) => {
        $(
            $(#[$meta])*
            #[gpui_kit::test]
            fn $name(cx: &mut TestAppContext) {
                a_press_submits_nothing(cx, $scene, $key);
            }
        )*
    };
}

submits_nothing! {
    // storage
    authenticated_storage_review_is_captured_and_footer_remains_reachable:
        "storage-auth-review", "storage-confirm";
    storage_review_preserves_inputs_and_compact_footer_never_submits_preview:
        "storage-review", "storage-confirm";
    // migration
    compact_and_wide_review_capture_exact_targets_and_remain_preview_only:
        "vm-migration", "migration-apply";
    compact_and_wide_review_capture_all_vms_and_keep_safe_footer:
        "batch-move", "batch-move-submit";
    // storage_maintenance
    reviewed_storage_graph_is_captured_reachable_and_never_submits_demo:
        "storage-graph", "storage-graph-submit";
    reviewed_storage_multipath_is_captured_reachable_and_never_submits_demo:
        "storage-multipath", "storage-graph-submit";
    // pool
    review_captures_target_and_compact_footer_never_submits_demo:
        "pool-policy-review", "pool-confirm";
    ha_policy_review_binds_vm_and_shared_vapp_settings_at_both_sizes:
        "ha-policy", "pool-confirm";
    the_pool_dissolve_review_submits_nothing_in_the_demo:
        "pool-dissolve", "pool-dissolve-confirm";
    the_pool_join_review_submits_nothing_in_the_demo:
        "pool-join", "join-submit";
    the_root_password_review_submits_nothing_in_the_demo:
        "root-password", "root-password-submit";
    // bulk_edit
    compact_and_wide_reviews_keep_exact_targets_and_the_confirmation_footer:
        "batch-delete", "batch-delete-submit";
    // host_services
    review_has_private_password_and_captured_target_at_both_widths:
        "host-power-settings", "host-power-confirm";
    // organization
    compact_review_keeps_controls_visible_and_refuses_preview_submission:
        "appliance-review", "appliance-apply";
    // pool_security
    compact_review_keeps_footer_visible_and_captured_identity_immutable:
        "pool-security-review", "security-confirm";
    // network_services
    review_captures_plaintext_policy_and_interfaces_at_both_widths:
        "network-services", "network-confirm";
    pool_vlan_review_captures_every_host_at_compact_and_wide_sizes_without_demo_dispatch:
        "network-topology", "network-confirm";
    host_scoped_tunnel_review_is_visible_at_both_sizes_and_demo_cannot_submit:
        "network-fabric", "network-confirm";
    sriov_review_names_driver_impact_and_reboot_boundary_at_both_sizes:
        "network-sriov", "network-confirm";
    exact_order_and_all_hosts_are_reviewed_at_both_widths_and_demo_cannot_apply:
        "network-fabric-pool", "network-confirm";
    // management_network
    management_review_captures_recovery_settings_at_both_widths_without_demo_dispatch:
        "management-network", "network-confirm";
    automatic_address_review_captures_mode_expected_ip_and_does_not_dispatch_demo:
        "management-dhcp", "network-confirm";
    vlan_over_bond_review_captures_full_topology_at_both_widths:
        "management-vlan", "network-confirm";
    management_bond_review_has_exact_members_and_recovery_at_both_widths:
        "management-bond", "network-confirm";
    management_vlan_bond_review_lists_recreated_ids_and_captures_parent_at_both_widths:
        "management-vlan-bond", "network-confirm";
    ipv6_stage_review_renders_at_two_widths_and_keeps_demo_apply_disabled:
        "management-ipv6-bond", "network-confirm";
}

/// organization, wish 82: after a name is typed the review is reached with
/// the pointer, and its Apply button submits nothing.
#[gpui_kit::test]
fn compact_review_is_reachable_and_cannot_submit_demo_metadata(cx: &mut TestAppContext) {
    at_sizes(
        cx,
        "organization",
        &[SMALL],
        &[
            Type("organization-name", "Edge"),
            Inside(&["organization-review"]),
            Press("organization-review"),
            Inside(&["organization-apply"]),
            Inert("organization-apply"),
        ],
    );
}
