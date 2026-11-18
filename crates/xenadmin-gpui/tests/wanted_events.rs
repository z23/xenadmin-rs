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
    BOTH, DEMO, SMALL, Step, close, described, front, model, node, on, open_scene, run_step,
    run_steps, says, submitted, surfaces,
};
use xenadmin_gpui::kit::testing::{TestAppContext, TestQueryExt as _, TestWindowExt as _};
use xenadmin_gpui::kit::*;
use xenadmin_gpui::scenes::ShellWindow;

/// One step that must hold; the panic names the scene's size.
#[track_caller]
fn must(cx: &mut TestAppContext, scene: &ShellWindow, step: Step) {
    if let Err(error) = run_step(cx, scene, step) {
        let size = on(cx, scene, |window, _| window.viewport_size());
        panic!("at {size:?}: {step:?}: {error}");
    }
}

/// Polls the model, as the window's timer does, until the surface in front
/// has the key: for what a worker thread of the model brings.
fn poll_until_has(cx: &mut TestAppContext, scene: &ShellWindow, key: &str) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !has(cx, scene, key) && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(20));
        cx.update(|cx| scene.store.update(cx, |store, cx| store.poll(cx)));
        cx.run_until_parked();
        common::settle(cx, scene);
    }
}

/// Whether the surface in front, or the tab, has an element with the key.
fn has(cx: &mut TestAppContext, scene: &ShellWindow, key: &str) -> bool {
    let scope = front(cx, scene);
    on(cx, scene, |window, _| {
        common::locate(window, scope.as_deref(), key).is_ok()
    })
}

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

// -- typed text ---------------------------------------------------------------

/// vm_properties, wish 23: what is typed into the name is the draft, and it
/// is still there after another page was looked at.
#[gpui_kit::test]
fn typing_in_the_properties_changes_the_draft_and_a_page_change_keeps_it(cx: &mut TestAppContext) {
    at_sizes(
        cx,
        "properties",
        BOTH,
        &[
            Type("property-name", "-typed"),
            Described("property-name", "-typed"),
            Press("vm-properties-pages-advanced"),
            Absent(&["property-name"]),
            Press("vm-properties-pages-general"),
            Described("property-name", "-typed"),
            Says("-typed"),
            Inside(&["properties-cancel", "review-properties"]),
        ],
    );
}

/// snapshots, wish 24.
#[gpui_kit::test]
fn snapshot_metadata_creation_is_editable_at_compact_and_wide_sizes(cx: &mut TestAppContext) {
    for size in BOTH {
        let scene = open_scene(cx, "snapshot-create", *size);
        let before = model(cx, &scene, |m| {
            (
                m.areas.snapshots.name.clone(),
                m.areas.snapshots.description.clone(),
            )
        });
        must(&mut *cx, &scene, Type("snapshot-name", " typed-name"));
        must(
            &mut *cx,
            &scene,
            Type("snapshot-description", " typed-note"),
        );
        let after = model(cx, &scene, |m| {
            (
                m.areas.snapshots.name.clone(),
                m.areas.snapshots.description.clone(),
            )
        });
        assert_ne!(before, after);
        assert!(after.0.contains("typed-name"), "{after:?}");
        assert!(after.1.contains("typed-note"), "{after:?}");
        must(&mut *cx, &scene, Disabled("confirm-snapshot", ""));
        assert_eq!(submitted(cx, &scene), 0);
        close(cx, scene);
    }
}

/// snapshots, wish 25: a snapshot pressed in the table, then each of the
/// two clone commands of the local row.
#[gpui_kit::test]
fn cloning_reviews_the_captured_snapshot_in_both_modes_and_stays_safe_in_demo(
    cx: &mut TestAppContext,
) {
    for size in BOTH {
        for command in ["local:snapshot.new-vm", "local:snapshot.new-template"] {
            let scene = open_scene(cx, "snapshots", *size);
            must(
                &mut *cx,
                &scene,
                Press("snapshots-table:60000000-0000-0000-0000-000000000000"),
            );
            must(&mut *cx, &scene, Press(command));
            must(&mut *cx, &scene, Front("snapshot-clone"));
            must(
                &mut *cx,
                &scene,
                Type("snapshot-clone-name", " typed-clone"),
            );
            must(
                &mut *cx,
                &scene,
                Described("snapshot-clone-name", "typed-clone"),
            );
            must(&mut *cx, &scene, Says("typed-clone"));
            assert_eq!(submitted(cx, &scene), 0);
            close(cx, scene);
        }
    }
}

/// vm_tools, wish 44: an address typed after a review drops the review.
#[gpui_kit::test]
fn a_typed_address_reaches_the_model_and_drops_the_shown_review(cx: &mut TestAppContext) {
    for size in BOTH {
        let scene = open_scene(cx, "guest-connections", *size);
        must(&mut *cx, &scene, Press("review-guest-tool"));
        // The local executable is inspected off the interface thread.
        poll_until_has(cx, &scene, "guest-tool-command");
        must(&mut *cx, &scene, Reach(&["guest-tool-command"]));
        must(&mut *cx, &scene, Type("guest-tool-address", "7"));
        must(&mut *cx, &scene, Described("guest-tool-address", "7"));
        must(&mut *cx, &scene, Absent(&["guest-tool-command"]));
        must(
            &mut *cx,
            &scene,
            Inside(&["review-guest-tool", "close-guest-tools"]),
        );
        close(cx, scene);
    }
}

/// vm_tools, wish 45: the multiline field takes typed JSON, and Escape from
/// the declarations opened over the launch dialog returns to that dialog.
#[gpui_kit::test]
fn the_tool_arguments_take_typed_json_and_escape_returns_to_the_launch_dialog(
    cx: &mut TestAppContext,
) {
    at_sizes(
        cx,
        "external-tools",
        BOTH,
        &[
            Type("guest-tool-arguments", "[\"--typed\"]"),
            Described("guest-tool-arguments", "[\"--typed\"]"),
            Inside(&["save-guest-tools", "remove-guest-tool", "close-guest-tools"]),
        ],
    );
    at_sizes(
        cx,
        "guest-connections",
        BOTH,
        &[
            Press("configure-guest-tools"),
            Front("external-tools"),
            Press("guest-tool-name"),
            Key("escape"),
            Front("guest-connections"),
        ],
    );
}

/// vm_lifecycle, wish 49: from the first step to the review with the
/// pointer and the keyboard; the seven step labels do not overlap.
#[gpui_kit::test]
fn creation_wizard_reviews_typed_resources_and_keeps_its_source_at_both_widths(
    cx: &mut TestAppContext,
) {
    for size in BOTH {
        let scene = open_scene(cx, "vm-create", *size);
        let steps: Vec<Bounds<Pixels>> = on(cx, &scene, |window, _| {
            window
                .elements()
                .into_iter()
                .filter(|element| {
                    element.role() == Some(Role::ListItem)
                        && common::under(element, "vm-lifecycle-dialog-steps")
                })
                .map(|element| element.bounds())
                .collect()
        });
        assert_eq!(steps.len(), 7, "{steps:?}");
        let width = px(size.0);
        for (index, step) in steps.iter().enumerate() {
            assert!(step.left() >= px(0.) && step.right() <= width, "{step:?}");
            for other in &steps[index + 1..] {
                assert!(
                    !step.intersects(other),
                    "two step labels overlap at {size:?}: {step:?} {other:?}"
                );
            }
        }
        // Back to the first step, then forward to the review.
        for _ in 0..8 {
            if run_step(cx, &scene, Press("back-vm-lifecycle")).is_err() {
                break;
            }
        }
        let (mut named, mut sized) = (false, false);
        for _ in 0..12 {
            if has(cx, &scene, "apply-vm-lifecycle") {
                break;
            }
            if !named && has(cx, &scene, "vm-lifecycle-name") {
                must(&mut *cx, &scene, Type("vm-lifecycle-name", "-typed"));
                named = true;
            }
            if !sized && has(cx, &scene, "vm-lifecycle-disk-size") {
                // The whole field: a capacity with its unit.
                must(&mut *cx, &scene, Press("vm-lifecycle-disk-size"));
                on(cx, &scene, |window, cx| {
                    window.press("ctrl-a", cx);
                    window.input("3 GiB", cx);
                });
                let size = node(cx, &scene, "vm-lifecycle-disk-size").and_then(|node| node.value);
                assert_eq!(size.as_deref(), Some("3 GiB"));
                sized = true;
            }
            must(&mut *cx, &scene, Press("review-vm-lifecycle"));
        }
        assert!(named && sized, "name {named}, disk size {sized}");
        must(&mut *cx, &scene, Inside(&["apply-vm-lifecycle"]));
        must(&mut *cx, &scene, Disabled("apply-vm-lifecycle", ""));
        must(&mut *cx, &scene, Says("-typed"));
        assert_eq!(submitted(cx, &scene), 0);
        close(cx, scene);
    }
}

/// A marker no fixture has.
const MARKER: &str = "zq7-private-marker";

/// Nothing the window draws, and nothing the surfaces describe, has the
/// marker.
#[track_caller]
fn assert_marker_gone(cx: &mut TestAppContext, scene: &ShellWindow) {
    assert!(!says(cx, scene, MARKER), "an element still has the marker");
    let leaked: Vec<String> = cx.update(|cx| {
        let surfaces = scene.store.read(cx).surfaces();
        surfaces
            .iter()
            .flat_map(xenadmin_ui::surface::flatten)
            .filter(|node| format!("{node:?}").contains(MARKER))
            .map(|node| node.path)
            .collect()
    });
    assert!(
        leaked.is_empty(),
        "the description has the marker: {leaked:?}"
    );
}

/// vm_lifecycle, wish 50 (plan R11): the review of a creation does not show
/// the private user-data, and the text area that held it is gone.
#[gpui_kit::test]
fn creation_cloud_inputs_are_validated_and_review_omits_the_private_payload(
    cx: &mut TestAppContext,
) {
    for size in BOTH {
        let scene = open_scene(cx, "creation-cloud", *size);
        must(
            &mut *cx,
            &scene,
            Type("creation-cloud-user-data", "\n# zq7-private-marker\n"),
        );
        assert!(says(cx, &scene, MARKER), "the typed text is not on screen");
        for _ in 0..8 {
            if has(cx, &scene, "apply-vm-lifecycle") {
                break;
            }
            must(&mut *cx, &scene, Press("review-vm-lifecycle"));
        }
        must(&mut *cx, &scene, Inside(&["apply-vm-lifecycle"]));
        must(&mut *cx, &scene, Absent(&["creation-cloud-user-data"]));
        assert_marker_gone(cx, &scene);
        close(cx, scene);
    }
}

/// vm_lifecycle, wish 51 (plan R11).
#[gpui_kit::test]
fn creation_handoff_prepares_private_media_for_the_exact_created_target_and_chosen_sr(
    cx: &mut TestAppContext,
) {
    let scene = open_scene(cx, "cloud-init", SMALL);
    must(
        &mut *cx,
        &scene,
        Type("cloud-init-user-data", "\n# zq7-private-marker\n"),
    );
    assert!(says(cx, &scene, MARKER), "the typed text is not on screen");
    must(&mut *cx, &scene, Press("cloud-init-prepare"));
    // The media is prepared off the interface thread.
    poll_until_has(cx, &scene, "cloud-init-review");
    must(&mut *cx, &scene, Reach(&["cloud-init-review"]));
    must(&mut *cx, &scene, Absent(&["cloud-init-user-data"]));
    assert_marker_gone(cx, &scene);
    close(cx, scene);
}

/// vm_devices, wish 55.
#[gpui_kit::test]
fn disk_creation_reviews_selected_storage_size_and_captured_vm_at_both_widths(
    cx: &mut TestAppContext,
) {
    at_sizes(
        cx,
        "vm-devices",
        BOTH,
        &[
            Press("local:vm.disk.new"),
            Type("lifecycle-name", "typed-disk"),
            Type("lifecycle-size", "3"),
            Press("lifecycle-sr-*"),
            Press("review-device-lifecycle"),
            Says("typed-disk"),
        ],
    );
}

/// vm_devices, wish 56: the exact UUID typed into the confirmation of a
/// deletion leaves Confirm disabled in the demo.
#[gpui_kit::test]
fn cleanup_review_keeps_exact_uuid_and_explicit_destructive_confirmation_at_both_sizes(
    cx: &mut TestAppContext,
) {
    for size in BOTH {
        let scene = open_scene(cx, "disk-cleanup", *size);
        let uuid = described(cx, &scene)
            .into_iter()
            .find(|node| node.label.as_deref() == Some("UUID"))
            .and_then(|node| node.value)
            .expect("the review names the UUID of the disk");
        must(&mut *cx, &scene, Press("disk-delete-confirmation"));
        on(cx, &scene, |window, cx| window.input(&uuid, cx));
        let typed = node(cx, &scene, "disk-delete-confirmation").and_then(|node| node.value);
        assert_eq!(typed.as_deref(), Some(uuid.as_str()));
        must(&mut *cx, &scene, Inside(&["disk-confirm", "disks-close"]));
        must(&mut *cx, &scene, Disabled("disk-confirm", DEMO));
        must(&mut *cx, &scene, Inert("disk-confirm"));
        close(cx, scene);
    }
}

/// bulk_edit, wish 69: the confirmation text reaches the model, and the
/// footer stays where it was.
#[gpui_kit::test]
fn the_typed_confirmation_of_a_batch_deletion_reaches_the_model(cx: &mut TestAppContext) {
    at_sizes(
        cx,
        "batch-delete",
        BOTH,
        &[
            Type("batch-delete-confirm", "DELETE 2"),
            Described("batch-delete-confirm", "DELETE 2"),
            Inside(&["batch-delete-close", "batch-delete-submit"]),
            Inert("batch-delete-submit"),
        ],
    );
}

/// storage, wish 34.
#[gpui_kit::test]
fn local_path_focus_preserves_probe_but_text_edit_invalidates_it(cx: &mut TestAppContext) {
    at_sizes(
        cx,
        "storage-local",
        BOTH,
        &[
            Reach(&["storage-local-probe"]),
            Press("storage-local-device"),
            Reach(&["storage-local-probe"]),
            Type("storage-local-device", "x"),
            Absent(&["storage-local-probe"]),
        ],
    );
}
