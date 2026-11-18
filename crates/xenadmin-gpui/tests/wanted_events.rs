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
    BOTH, DEMO, SMALL, Step, WIDE, close, described, front, model, node, on, open_scene, run_step,
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
    poll_until(cx, scene, |cx, scene| has(cx, scene, key));
}

/// Polls the model until `done`, or for twenty seconds.
fn poll_until(
    cx: &mut TestAppContext,
    scene: &ShellWindow,
    done: impl Fn(&mut TestAppContext, &ShellWindow) -> bool,
) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !done(cx, scene) && std::time::Instant::now() < deadline {
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
        // The preset's terminal launcher is not on every machine (it was
        // not on the CI runner): review a program every Linux has, as the
        // model's own tests of this dialog do.
        common::run(cx, &scene, |m| {
            m.preferences.external_tools.tools[0].executable = "/usr/bin/true".into();
        });
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

/// Whether the renderer still holds the widget of a text field of a
/// surface, with what was typed into it.
fn holds_text(cx: &mut TestAppContext, scene: &ShellWindow, surface: &str, key: &str) -> bool {
    let scope = xenadmin_gpui::render::surface_scope(surface);
    cx.update(|cx| scene.view.read(cx).ui().holds_text(&scope, key))
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
        let field = ("vm-lifecycle-dialog", "creation-cloud-user-data");
        assert!(holds_text(cx, &scene, field.0, field.1));
        for _ in 0..8 {
            if has(cx, &scene, "apply-vm-lifecycle") {
                break;
            }
            must(&mut *cx, &scene, Press("review-vm-lifecycle"));
        }
        must(&mut *cx, &scene, Inside(&["apply-vm-lifecycle"]));
        must(&mut *cx, &scene, Absent(&["creation-cloud-user-data"]));
        assert!(
            !holds_text(cx, &scene, field.0, field.1),
            "the text area's state outlived its step"
        );
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
    assert!(
        !holds_text(cx, &scene, "cloud-init-dialog", "cloud-init-user-data"),
        "the text area's state outlived the form"
    );
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

// -- a checkbox ---------------------------------------------------------------

/// power, wish 2: a forced shutdown over a running migration. No registered
/// scene has one: the task of the scene is made a `migrate_send` and the
/// review is asked for again.
#[gpui_kit::test]
fn a_forced_action_over_a_migration_warns_and_waits_for_the_operator_to_accept_it(
    cx: &mut TestAppContext,
) {
    let scene = common::open_scene_with(cx, "vm-recovery-force", SMALL, |m| {
        let target = m.target();
        m.areas.power.recovery = None;
        if let Some(inventory) = m.inventory.take() {
            let mut inventory = inventory.as_ref().clone();
            for details in inventory.details.vms.values_mut() {
                for running in &mut details.current_operations {
                    running.operation = "migrate_send".into();
                }
            }
            m.inventory = Some(std::sync::Arc::new(inventory));
        }
        m.invoke(xenadmin_ui::CommandId("vm.force-shutdown"), &target);
    });
    for step in [
        Front("vm-recovery"),
        Reach(&["recovery-migration-caution"]),
        Checked("recovery-accept-migration", false),
        Press("recovery-accept-migration"),
        Checked("recovery-accept-migration", true),
        // Accepted, and still nothing can be submitted in the demo.
        Inside(&["recovery-confirm", "recovery-close"]),
        Disabled("recovery-confirm", DEMO),
        Inert("recovery-confirm"),
        Press("recovery-accept-migration"),
        Checked("recovery-accept-migration", false),
    ] {
        must(&mut *cx, &scene, step);
    }
    assert_eq!(submitted(cx, &scene), 0);
    close(cx, scene);
}

// -- file prompts -------------------------------------------------------------

/// The file a control asks the desktop for: its title, whether it is a
/// save, and the name it suggests. The scene records its intents.
fn asked_for(
    cx: &mut TestAppContext,
    scene: &ShellWindow,
) -> Vec<(String, xenadmin_ui::FileMode, Option<String>)> {
    cx.update(|cx| {
        scene
            .store
            .read(cx)
            .recorded_intents()
            .iter()
            .filter_map(|intent| match intent {
                xenadmin_ui::Intent::ChooseFile(request) => Some((
                    request.title.clone(),
                    request.mode,
                    request.suggested_name.clone(),
                )),
                _ => None,
            })
            .collect()
    })
}

/// A scene whose file prompts are the platform's, as in the client.
fn open_with_prompts(cx: &mut TestAppContext, name: &str, size: (f32, f32)) -> ShellWindow {
    let scene = open_scene(cx, name, size);
    cx.update(|cx| {
        xenadmin_gpui::intents::install(&scene.store, xenadmin_gpui::intents::Hooks::default(), cx)
    });
    scene
}

/// A directory of this test's own, emptied.
fn scratch(test: &str) -> std::path::PathBuf {
    let directory =
        std::env::temp_dir().join(format!("xenadmin-gpui-{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("scratch directory");
    directory
}

/// events, wish 10.
#[gpui_kit::test]
fn the_activity_export_asks_for_a_path_and_writes_only_a_chosen_one(cx: &mut TestAppContext) {
    // What is asked for.
    let scene = open_scene(cx, "activity", WIDE);
    must(&mut *cx, &scene, Press("local:events.export"));
    assert_eq!(
        asked_for(cx, &scene),
        [(
            "Export entries as JSON".to_owned(),
            xenadmin_ui::FileMode::Save,
            Some("xenadmin-activity.json".to_owned())
        )]
    );
    close(cx, scene);

    // Cancelled: no file, and the button can be used again.
    let directory = scratch("activity-export");
    let path = directory.join("activity.json");
    let scene = open_with_prompts(cx, "activity", WIDE);
    must(&mut *cx, &scene, Press("local:events.export"));
    assert!(cx.did_prompt_for_new_path());
    cx.simulate_new_path_selection(|_| None);
    cx.run_until_parked();
    common::settle(cx, &scene);
    assert!(!path.exists());
    must(&mut *cx, &scene, Enabled("local:events.export"));

    // Chosen: the file is written.
    must(&mut *cx, &scene, Press("local:events.export"));
    assert!(cx.did_prompt_for_new_path());
    let chosen = path.clone();
    cx.simulate_new_path_selection(move |_| Some(chosen));
    cx.run_until_parked();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !path.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(20));
        cx.update(|cx| scene.store.update(cx, |store, cx| store.poll(cx)));
        cx.run_until_parked();
    }
    let written = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        written.contains("Synthetic task"),
        "no export at {}: {written:.80}",
        path.display()
    );
    close(cx, scene);
    let _ = std::fs::remove_dir_all(&directory);
}

/// transfers, wish 53: an export asks for a save named after the guest, an
/// import for an existing file.
#[gpui_kit::test]
fn native_save_captures_source_and_an_import_uses_the_open_dialog(cx: &mut TestAppContext) {
    let scene = open_scene(cx, "archive-export", SMALL);
    must(&mut *cx, &scene, Press("transfer-next"));
    must(&mut *cx, &scene, Press("transfer-choose"));
    let asked = asked_for(cx, &scene);
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert_eq!(asked[0].1, xenadmin_ui::FileMode::Save, "{asked:?}");
    assert_eq!(asked[0].2.as_deref(), Some("guest.xva"), "{asked:?}");
    close(cx, scene);

    let scene = open_scene(cx, "ovf-import", SMALL);
    for _ in 0..6 {
        if has(cx, &scene, "transfer-choose") {
            break;
        }
        must(&mut *cx, &scene, Press("transfer-back"));
    }
    must(&mut *cx, &scene, Press("transfer-choose"));
    let asked = asked_for(cx, &scene);
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert_eq!(asked[0].1, xenadmin_ui::FileMode::Open, "{asked:?}");
    close(cx, scene);
}

/// bulk_export, wish 73.
#[gpui_kit::test]
fn native_output_selection_is_bound_to_editor_identity_and_format(cx: &mut TestAppContext) {
    let directory = scratch("batch-export");
    let path = directory.join("first.xva");
    let scene = open_with_prompts(cx, "batch-export", WIDE);
    must(&mut *cx, &scene, Press("batch-export-back"));
    must(&mut *cx, &scene, Press("batch-export-choose-0"));
    assert!(cx.did_prompt_for_new_path());
    let chosen = path.clone();
    cx.simulate_new_path_selection(move |_| Some(chosen));
    cx.run_until_parked();
    // The path is inspected off the interface thread.
    let line = format!("Output: {}", path.display());
    poll_until(cx, &scene, |cx, scene| says(cx, scene, &line));
    assert!(says(cx, &scene, &line), "nothing says \"{line}\"");
    assert!(!path.exists(), "choosing an output wrote a file");
    close(cx, scene);
    let _ = std::fs::remove_dir_all(&directory);
}

/// host_services, wish 77.
#[gpui_kit::test]
fn host_archive_review_captures_target_and_keeps_compact_footer_visible(cx: &mut TestAppContext) {
    for size in BOTH {
        let directory = scratch("host-support");
        let path = directory.join("host-support.tar");
        let scene = open_with_prompts(cx, "host-support", *size);
        must(&mut *cx, &scene, Press("host-support-file"));
        assert!(cx.did_prompt_for_new_path());
        let chosen = path.clone();
        cx.simulate_new_path_selection(move |_| Some(chosen));
        cx.run_until_parked();
        // The path is inspected off the interface thread.
        poll_until(cx, &scene, |cx, scene| {
            run_step(cx, scene, Enabled("host-support-review")).is_ok()
        });
        must(&mut *cx, &scene, Press("host-support-review"));
        must(
            &mut *cx,
            &scene,
            Inside(&["host-support-submit", "host-support-close"]),
        );
        must(&mut *cx, &scene, Inert("host-support-submit"));
        assert_eq!(submitted(cx, &scene), 0);
        assert!(!path.exists(), "the demo created {}", path.display());
        close(cx, scene);
        let _ = std::fs::remove_dir_all(&directory);
    }
}

// -- a filter -----------------------------------------------------------------

/// The rows a table of the tab shows.
fn rows_of(cx: &mut TestAppContext, scene: &ShellWindow, table: &str) -> usize {
    on(cx, scene, |window, _| {
        window
            .elements()
            .into_iter()
            .filter(|element| element.role() == Some(Role::Row) && common::under(element, table))
            .count()
    })
}

/// Presses the Kit's own clear button of a field of the tab.
#[track_caller]
fn clear_field(cx: &mut TestAppContext, scene: &ShellWindow, key: &str) {
    on(cx, scene, |window, cx| {
        let button = window
            .elements()
            .into_iter()
            .find(|element| common::key_of(element) == Some("clean") && common::under(element, key))
            .unwrap_or_else(|| panic!("`{key}` has no clear button"));
        common::inside(window, &button).expect("the clear button is on screen");
        common::click_at(window, cx, button.bounds().center(), MouseButton::Left);
    });
}

/// events, wish 9.
#[gpui_kit::test]
fn clear_buttons_empty_the_activity_filters_and_restore_the_rows(cx: &mut TestAppContext) {
    let scene = open_scene(cx, "activity", WIDE);
    assert_eq!(rows_of(cx, &scene, "activity-table"), 2);
    must(
        &mut *cx,
        &scene,
        Type("activity-search", "Synthetic task 1"),
    );
    assert_eq!(rows_of(cx, &scene, "activity-table"), 1);
    must(&mut *cx, &scene, Says("1 matching"));
    must(&mut *cx, &scene, Type("activity-state-filter", "success"));
    assert_eq!(rows_of(cx, &scene, "activity-table"), 0);
    // Each button empties its own field only.
    clear_field(cx, &scene, "activity-search");
    let filters = model(cx, &scene, |m| {
        (
            m.areas.events.search.clone(),
            m.areas.events.state_filter.clone(),
        )
    });
    assert_eq!(filters, (String::new(), "success".to_owned()));
    assert_eq!(rows_of(cx, &scene, "activity-table"), 1);
    clear_field(cx, &scene, "activity-state-filter");
    assert!(model(cx, &scene, |m| m
        .areas
        .events
        .state_filter
        .is_empty()));
    assert_eq!(rows_of(cx, &scene, "activity-table"), 2);
    must(&mut *cx, &scene, Says("2 matching entries"));
    close(cx, scene);
}

// -- the keyboard and the secondary button in a grid --------------------------

/// The table of the Search tab.
const SEARCH: &str = "search-results";
/// The nine power commands of a VM.
const POWER: [&str; 9] = [
    "vm.start",
    "vm.shutdown",
    "vm.reboot",
    "vm.suspend",
    "vm.resume",
    "vm.pause",
    "vm.unpause",
    "vm.force-shutdown",
    "vm.force-reboot",
];

/// The rows of a table that are in the tree, from the top: the key of the
/// element and its accessible name.
fn rows_in(cx: &mut TestAppContext, scene: &ShellWindow, table: &str) -> Vec<(String, String)> {
    on(cx, scene, |window, _| {
        let mut rows: Vec<_> = window
            .elements()
            .into_iter()
            .filter(|element| element.role() == Some(Role::Row) && common::under(element, table))
            .collect();
        rows.sort_by(|left, right| {
            left.bounds()
                .top()
                .partial_cmp(&right.bounds().top())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        rows.iter()
            .map(|row| {
                (
                    common::key_of(row).unwrap_or_default().to_owned(),
                    row.label().unwrap_or_default().to_owned(),
                )
            })
            .collect()
    })
}

/// The id of the object a row of the Search table stands for.
fn object_of(row: &str) -> String {
    row.rsplit(':').next().unwrap_or_default().to_owned()
}

/// A press with the primary button on an element with a key that is not
/// known before the test runs.
#[track_caller]
fn press_on(cx: &mut TestAppContext, scene: &ShellWindow, key: &str) {
    let scope = front(cx, scene);
    on(cx, scene, |window, cx| {
        common::press(window, cx, scope.as_deref(), key)
            .map(|_| ())
            .unwrap_or_else(|error| panic!("{error}"));
    });
}

/// A press with the secondary button on a row, brought into view first.
#[track_caller]
fn secondary_press_on(cx: &mut TestAppContext, scene: &ShellWindow, key: &str) {
    on(cx, scene, |window, cx| {
        let row = common::reach(window, cx, None, key).unwrap_or_else(|error| panic!("{error}"));
        common::click_at(window, cx, row.bounds().center(), MouseButton::Right);
    });
}

/// The VMs the commands act on, as the model has them.
fn target_vms(cx: &mut TestAppContext, scene: &ShellWindow) -> Vec<String> {
    model(cx, scene, |m| {
        m.target().vms().iter().map(ToString::to_string).collect()
    })
}

/// The items of the context menu the model has open: command and the
/// reason it is disabled.
fn open_menu_items(
    cx: &mut TestAppContext,
    scene: &ShellWindow,
) -> Vec<(&'static str, Option<String>)> {
    model(cx, scene, |m| {
        m.context_items()
            .into_iter()
            .map(|item| (item.id.0, item.disabled))
            .collect()
    })
}

/// The place of a command in the popup of the open context menu.
#[track_caller]
fn popup_index(cx: &mut TestAppContext, scene: &ShellWindow, command: &str) -> usize {
    // An entry's element id is its place at the top level of the menu,
    // separators counted; a submenu is one entry.
    model(cx, scene, |m| {
        let mut index = 0;
        for (group, entries) in m.context_menu().into_iter().enumerate() {
            if group > 0 {
                index += 1;
            }
            for entry in entries {
                if let xenadmin_ui::command::MenuEntry::Item(item) = &entry
                    && item.id.0 == command
                {
                    return Some(index);
                }
                index += 1;
            }
        }
        None
    })
    .unwrap_or_else(|| panic!("the top level of the context menu has no {command}"))
}

/// Chooses an item of the popup the shell shows for the open context menu.
#[track_caller]
fn choose_from_popup(cx: &mut TestAppContext, scene: &ShellWindow, command: &str) {
    let index = popup_index(cx, scene, command);
    on(cx, scene, |window, cx| {
        window.within("popup-menu").click(index, cx);
    });
    for _ in 0..100 {
        if on(cx, scene, |window, _| {
            window.try_find("popup-menu").is_none()
        }) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("the popup stayed open after {command}");
}

/// The Search tab of the demo pool, and its first two VM rows.
fn search_tab(cx: &mut TestAppContext, size: (f32, f32)) -> (ShellWindow, [String; 2]) {
    let scene = open_scene(cx, "demo", size);
    must(&mut *cx, &scene, Press("tab-search"));
    let rows: Vec<String> = rows_in(cx, &scene, SEARCH)
        .into_iter()
        .map(|(key, _)| key)
        .filter(|key| key.contains(":VM:"))
        .collect();
    assert!(rows.len() >= 2, "the Search table shows {rows:?}");
    (scene, [rows[0].clone(), rows[1].clone()])
}

/// power, wish 4.
#[gpui_kit::test]
fn multi_selection_survives_inventory_refresh_and_reviews_captured_vms_at_both_sizes(
    cx: &mut TestAppContext,
) {
    for size in BOTH {
        let (scene, [first, second]) = search_tab(cx, *size);
        let both = vec![object_of(&first), object_of(&second)];
        press_on(cx, &scene, &first);
        assert_eq!(target_vms(cx, &scene), [object_of(&first)]);
        must(&mut *cx, &scene, Key("shift-down"));
        assert_eq!(target_vms(cx, &scene), both, "Shift+Down at {size:?}");

        // The inventory is replaced by one that says the same.
        common::run(cx, &scene, |m| {
            if let Some(inventory) = m.inventory.take() {
                m.inventory = Some(std::sync::Arc::new(inventory.as_ref().clone()));
            }
        });
        assert_eq!(target_vms(cx, &scene), both, "after a refresh");

        // The menu of a selected row is the selection's.
        secondary_press_on(cx, &scene, &second);
        assert_eq!(target_vms(cx, &scene), both, "after the secondary press");
        let items = open_menu_items(cx, &scene);
        assert!(
            items.contains(&("vm.power-selection", None)),
            "the menu of two VMs: {items:?}"
        );
        choose_from_popup(cx, &scene, "vm.power-selection");
        must(&mut *cx, &scene, Front("power-batch"));
        let captured: Vec<String> = model(cx, &scene, |m| {
            let batch = m.areas.power.batch.as_ref().expect("a power batch");
            batch.vms.iter().map(|vm| vm.id.to_string()).collect()
        });
        assert_eq!(captured, both, "the batch has the two rows, in row order");
        assert_eq!(submitted(cx, &scene), 0);
        close(cx, scene);
    }
}

/// power, wish 5.
#[gpui_kit::test]
fn context_menu_targets_clicked_row_and_keeps_non_vm_menus_relevant(cx: &mut TestAppContext) {
    let (scene, [first, second]) = search_tab(cx, WIDE);
    press_on(cx, &scene, &first);
    // A secondary press on a row that is not selected selects it, and the
    // menu is that row's.
    secondary_press_on(cx, &scene, &second);
    assert_eq!(target_vms(cx, &scene), [object_of(&second)]);
    let menu = model(cx, &scene, |m| m.selection.menu.clone()).expect("a context menu");
    assert_eq!(menu.object.id, object_of(&second));
    let items = open_menu_items(cx, &scene);
    for command in POWER {
        assert!(
            items.iter().any(|(id, _)| *id == command),
            "the menu of a VM lacks {command}: {items:?}"
        );
    }
    assert!(on(cx, &scene, |window, _| window
        .try_find("popup-menu")
        .is_some()));
    close(cx, scene);

    // A row of another kind has none of them.
    let scene = open_scene(cx, "demo", WIDE);
    must(&mut *cx, &scene, Press("tab-search"));
    must(&mut *cx, &scene, Type("search-text", "Demo network"));
    must(&mut *cx, &scene, Press("search-apply"));
    let rows = rows_in(cx, &scene, SEARCH);
    let network = rows
        .iter()
        .find(|(key, _)| !key.contains(":VM:"))
        .map(|(key, _)| key.clone())
        .unwrap_or_else(|| panic!("no row but VMs: {rows:?}"));
    secondary_press_on(cx, &scene, &network);
    let menu = model(cx, &scene, |m| m.selection.menu.clone()).expect("a context menu");
    assert_eq!(menu.object.id, object_of(&network));
    let items = open_menu_items(cx, &scene);
    assert!(!items.is_empty(), "the row {network} has an empty menu");
    assert!(
        !items
            .iter()
            .any(|(id, _)| POWER.contains(id) || *id == "vm.power-selection"),
        "{network}: {items:?}"
    );
    close(cx, scene);
}

/// Every guest of the demo pool selected with Ctrl+A in the Search table,
/// then the search narrowed to one of them. The rows that are left.
fn all_guests_selected_then_one_left(
    cx: &mut TestAppContext,
) -> (ShellWindow, Vec<(String, String)>) {
    let limit = "Select between 1 and 32 visible VMs for a power batch";
    // Only the guests: their UUIDs start so.
    let scene = open_scene(cx, "demo", WIDE);
    must(&mut *cx, &scene, Press("tab-search"));
    must(&mut *cx, &scene, Type("search-text", "30000000-0000"));
    must(&mut *cx, &scene, Press("search-apply"));
    let first = rows_in(cx, &scene, SEARCH)
        .into_iter()
        .map(|(key, _)| key)
        .next()
        .expect("a row");
    press_on(cx, &scene, &first);
    must(&mut *cx, &scene, Key("ctrl-a"));
    let selected = target_vms(cx, &scene);
    assert!(
        selected.len() > 32,
        "Ctrl+A selected {} VMs",
        selected.len()
    );
    let items: Vec<(&'static str, Option<String>)> = model(cx, &scene, |m| {
        xenadmin_ui::command::context_menu(m, &m.target())
            .into_iter()
            .flatten()
            .flat_map(xenadmin_ui::command::MenuEntry::into_items)
            .map(|item| (item.id.0, item.disabled))
            .collect()
    });
    for command in POWER.into_iter().chain(["vm.power-selection"]) {
        let reason = items
            .iter()
            .find(|(id, _)| *id == command)
            .unwrap_or_else(|| panic!("no {command} in {items:?}"));
        assert_eq!(reason.1.as_deref(), Some(limit), "{command}");
    }

    // A filter that leaves one row leaves that one VM as the target.
    must(&mut *cx, &scene, Press("search-text"));
    on(cx, &scene, |window, cx| {
        window.press("ctrl-a", cx);
        window.input("Application 02", cx);
    });
    must(&mut *cx, &scene, Press("search-apply"));
    let rows = rows_in(cx, &scene, SEARCH);
    assert_eq!(rows.len(), 1, "{rows:?}");
    (scene, rows)
}

/// power, wish 6: more than 32 VMs selected with Ctrl+A leave every power
/// command disabled with the limit as the reason, and no row a filter has
/// taken away stays a target.
#[gpui_kit::test]
fn batch_uses_only_visible_selection_and_rejects_over_limit_without_silent_subset(
    cx: &mut TestAppContext,
) {
    let (scene, rows) = all_guests_selected_then_one_left(cx);
    let visible = object_of(&rows[0].0);
    let targets = target_vms(cx, &scene);
    assert!(
        targets.iter().all(|vm| *vm == visible),
        "a VM that is not shown is still a target: {targets:?}"
    );
    close(cx, scene);
}

/// power, wish 6, its last sentence: the one row a filter leaves, which was
/// selected, is the target.
#[gpui_kit::test]
fn the_selected_row_a_filter_leaves_is_still_the_target(cx: &mut TestAppContext) {
    let (scene, rows) = all_guests_selected_then_one_left(cx);
    assert_eq!(target_vms(cx, &scene), [object_of(&rows[0].0)]);
    close(cx, scene);
}

/// The names of the items of the popup menu on screen.
fn popup_labels(cx: &mut TestAppContext, scene: &ShellWindow) -> Vec<String> {
    on(cx, scene, |window, _| {
        window
            .elements()
            .into_iter()
            .filter(|element| element.role() == Some(Role::MenuItem))
            .filter_map(|element| element.label().map(str::to_owned))
            .collect()
    })
}

fn snapshot_row(n: u8) -> String {
    format!("snapshots-table:60000000-0000-0000-0000-00000000000{n}")
}

/// snapshots, wish 27.
#[gpui_kit::test]
fn snapshot_rows_take_a_multi_selection_a_row_menu_and_escape(cx: &mut TestAppContext) {
    let scene = open_scene(cx, "snapshots", WIDE);
    press_on(cx, &scene, &snapshot_row(0));
    assert!(has(cx, &scene, "snapshot-information"));
    assert!(!has(cx, &scene, "snapshot-selection-count"));
    // The secondary modifier adds a row.
    let second = snapshot_row(1);
    on(cx, &scene, |window, cx| {
        let modifiers = Modifiers::secondary_key(cx);
        window.click_with_modifiers(ElementId::Name(second.into()), modifiers, cx);
    });
    assert!(has(cx, &scene, "snapshot-selection-count"));
    assert!(!has(cx, &scene, "snapshot-information"));
    must(&mut *cx, &scene, Enabled("snapshot-batch-export-open"));
    // Escape in the table clears the selection.
    must(&mut *cx, &scene, Key("escape"));
    assert!(!has(cx, &scene, "snapshot-selection-count"));
    assert!(!has(cx, &scene, "snapshot-information"));
    must(&mut *cx, &scene, Disabled("local:snapshot.revert", ""));
    // A secondary press selects the row and opens its menu.
    secondary_press_on(cx, &scene, &snapshot_row(2));
    assert!(has(cx, &scene, "snapshot-information"));
    assert_eq!(
        popup_labels(cx, &scene),
        [
            "Revert",
            "Edit properties",
            "Create VM",
            "Create template",
            "Delete"
        ]
    );
    close(cx, scene);
}

/// bulk_export, wish 74.
#[gpui_kit::test]
fn several_snapshots_selected_in_the_view_open_one_review_and_single_actions_take_one(
    cx: &mut TestAppContext,
) {
    for size in BOTH {
        let scene = open_scene(cx, "snapshots", *size);
        let first = rows_in(cx, &scene, "snapshots-table")
            .into_iter()
            .map(|(key, _)| key)
            .next()
            .expect("a snapshot row");
        press_on(cx, &scene, &first);
        must(&mut *cx, &scene, Key("shift-down"));
        assert!(has(cx, &scene, "snapshot-selection-count"), "{size:?}");
        must(&mut *cx, &scene, Press("snapshot-batch-export-open"));
        must(&mut *cx, &scene, Front("batch-export"));
        // Two of the three snapshots, in one wizard.
        let count = node(cx, &scene, "batch-export-count").expect("the count of targets");
        let said = format!("{:?} {:?} {:?}", count.label, count.value, count.text);
        assert!(said.contains('2') && !said.contains('3'), "{said}");
        assert_eq!(submitted(cx, &scene), 0);
        close(cx, scene);
    }
}

/// snapshots, wish 26.
#[gpui_kit::test]
fn snapshot_widgets_review_captured_targets_and_demo_cannot_confirm_changes(
    cx: &mut TestAppContext,
) {
    let guest = "30000000-0000-0000-0000-000000000000";
    let scene = open_scene(cx, "demo", WIDE);
    let name = model(cx, &scene, |m| {
        let inventory = m.inventory.clone().expect("the demo inventory");
        inventory
            .guests()
            .find(|vm| vm.id.to_string() == guest)
            .map(|vm| vm.name.clone())
            .expect("the guest")
    });
    // The tree draws the rows in view only: its search box brings the
    // guest's row there.
    on(cx, &scene, |window, cx| {
        window.click(xenadmin_gpui::shell::nav::SEARCH_KEY, cx);
        window.input(&name, cx);
    });
    on(cx, &scene, |window, cx| {
        let row = window
            .within(xenadmin_gpui::shell::nav::TREE_KEY)
            .find_by_label(&name);
        common::click_at(window, cx, row.bounds().center(), MouseButton::Left);
    });
    assert_eq!(target_vms(cx, &scene), [guest]);
    must(&mut *cx, &scene, Press("tab-snapshots"));
    assert_eq!(rows_of(cx, &scene, "snapshots-table"), 3);
    must(&mut *cx, &scene, Press("local:vm.snapshot"));
    must(&mut *cx, &scene, Front("snapshot-create"));
    must(&mut *cx, &scene, Press("cancel-snapshot"));
    must(&mut *cx, &scene, NoSurface);
    press_on(cx, &scene, &snapshot_row(0));
    must(&mut *cx, &scene, Press("local:snapshot.revert"));
    must(&mut *cx, &scene, Front("snapshot-restore"));
    must(
        &mut *cx,
        &scene,
        Says("Changes made since the snapshot will be lost"),
    );
    must(&mut *cx, &scene, Disabled("confirm-snapshot", ""));
    must(&mut *cx, &scene, Inert("confirm-snapshot"));
    close(cx, scene);
}

/// vm_devices, wish 59.
#[gpui_kit::test]
fn a_secondary_press_on_a_device_row_opens_its_menu(cx: &mut TestAppContext) {
    let scene = open_scene(cx, "vm-devices", WIDE);
    let rows = rows_in(cx, &scene, "storage-vbds");
    assert!(!rows.is_empty());
    secondary_press_on(cx, &scene, &rows[0].0);
    let labels = popup_labels(cx, &scene);
    for item in ["Detach", "Properties", "Resize", "Copy"] {
        assert!(labels.iter().any(|label| label == item), "{labels:?}");
    }
    close(cx, scene);

    // The disks of a repository, behind the dialog of the scene.
    let scene = open_scene(cx, "storage-cache", WIDE);
    must(&mut *cx, &scene, Press("storage-close"));
    must(&mut *cx, &scene, NoSurface);
    let rows = rows_in(cx, &scene, "repository-disks");
    assert!(!rows.is_empty(), "the repository shows no disk");
    secondary_press_on(cx, &scene, &rows[0].0);
    let labels = popup_labels(cx, &scene);
    assert!(!labels.is_empty(), "no menu opened at {}", rows[0].0);
    close(cx, scene);
}

/// vm_devices, wish 58.
#[gpui_kit::test]
fn the_device_property_chooser_shows_why_an_option_is_unavailable(cx: &mut TestAppContext) {
    for size in BOTH {
        let scene = open_scene(cx, "device-properties", *size);
        must(&mut *cx, &scene, Reach(&["property-value"]));
        must(
            &mut *cx,
            &scene,
            Inside(&["property-review", "property-cancel"]),
        );
        must(&mut *cx, &scene, Press("property-back"));
        must(&mut *cx, &scene, Reach(&["property-choice"]));
        let reason = node(cx, &scene, "property-choice-0").and_then(|node| node.disabled);
        assert!(
            reason.as_deref().is_some_and(|reason| !reason.is_empty()),
            "the first option has no reason"
        );
        must(&mut *cx, &scene, Disabled("property-choice-0", ""));
        must(&mut *cx, &scene, Inside(&["property-cancel"]));
        close(cx, scene);
    }
}

/// vm_devices, wish 57.
#[gpui_kit::test]
fn the_history_button_of_a_disk_review_closes_it_and_shows_the_events(cx: &mut TestAppContext) {
    let scene = open_scene(cx, "disk-cleanup", WIDE);
    must(&mut *cx, &scene, Press("disk-history"));
    must(&mut *cx, &scene, NoSurface);
    assert!(
        has(cx, &scene, "activity-table"),
        "the Events tab is not shown"
    );
    close(cx, scene);
}

/// cross_pool, wish 85, its last sentence: the editor a batch starts in.
#[gpui_kit::test]
fn the_first_stage_of_a_batch_cross_pool_move_keeps_its_footer_in_the_window(
    cx: &mut TestAppContext,
) {
    for size in BOTH {
        let scene = open_scene(cx, "batch-cross-pool", *size);
        for _ in 0..4 {
            if has(cx, &scene, "batch-cross-pool-next") && !has(cx, &scene, "batch-cross-pool-back")
            {
                break;
            }
            must(&mut *cx, &scene, Press("batch-cross-pool-back"));
        }
        must(
            &mut *cx,
            &scene,
            Inside(&["batch-cross-pool-close", "batch-cross-pool-next"]),
        );
        must(&mut *cx, &scene, Reach(&["experimental-notice"]));
        close(cx, scene);
    }
}

/// power, wish 7.
#[gpui_kit::test]
fn known_denial_disables_power_without_hiding_its_reason_at_both_widths(cx: &mut TestAppContext) {
    let scene = open_scene(cx, "unavailable-actions", (1440., 1200.));
    let (menus, toolbar) = cx.update(|cx| {
        let store = scene.store.read(cx);
        (store.menu_bar(), store.toolbar())
    });
    let mut reasons = Vec::new();
    for command in ["vm.start", "vm.shutdown"] {
        let item = menus
            .iter()
            .flat_map(|menu| menu.items())
            .find(|item| item.id.0 == command)
            .unwrap_or_else(|| panic!("no menu has {command}"));
        let reason = item.disabled.clone().unwrap_or_default();
        assert!(!reason.is_empty(), "{command} is enabled");
        reasons.push(reason);
    }
    // The toolbar shows a power command only while it can be used.
    let shown: Vec<&str> = toolbar.iter().flatten().map(|item| item.id.0).collect();
    assert!(!shown.iter().any(|id| POWER.contains(id)), "{shown:?}");
    assert!(!has(cx, &scene, "vm.start") && !has(cx, &scene, "toolbar:vm.start"));
    // The tree dump carries the reason.
    let tree = on(cx, &scene, |window, cx| {
        xenadmin_gpui::scenes::tree_json(window, cx, &scene.store).to_string()
    });
    for reason in &reasons {
        let quoted = serde_json::to_string(reason).expect("a string");
        assert!(tree.contains(quoted.trim_matches('"')), "{reason}");
    }
    close(cx, scene);
}

/// bulk_edit, wish 70 (and the keyboard half of 71): two rows selected with
/// Shift+Down, the menu opened with Shift+F10, Properties chosen from it.
#[gpui_kit::test]
fn property_batch_multiselect_review_is_immutable_and_footer_reachable_at_both_sizes(
    cx: &mut TestAppContext,
) {
    for size in BOTH {
        let (scene, [first, second]) = search_tab(cx, *size);
        press_on(cx, &scene, &first);
        must(&mut *cx, &scene, Key("shift-down"));
        assert_eq!(
            target_vms(cx, &scene),
            [object_of(&first), object_of(&second)]
        );
        must(&mut *cx, &scene, Key("shift-f10"));
        let items = open_menu_items(cx, &scene);
        assert!(
            items.contains(&("vm.properties", None)),
            "the keyboard's menu of two VMs: {items:?}"
        );
        choose_from_popup(cx, &scene, "vm.properties");
        must(&mut *cx, &scene, Front("vm-properties"));
        must(&mut *cx, &scene, Says("Bulk properties"));
        close(cx, scene);
    }
}

/// bulk_edit, wish 71: the Menu key on a grid selection of two custom
/// templates opens the context menu with Properties, which opens the
/// Properties dialog of templates on "Bulk properties"; an item pressed
/// after the selection changed does nothing. The demo has no custom
/// template: `xenadmin_ui::testing::custom_templates` gives it two.
#[gpui_kit::test]
fn property_batch_picker_and_template_entry_work_with_keyboard_menu_capture(
    cx: &mut TestAppContext,
) {
    const PROPERTIES: &str = "template.properties";
    let mut templates = Vec::new();
    let scene = common::open_scene_with(cx, "demo", WIDE, |m| {
        templates = xenadmin_ui::testing::custom_templates(m, 2);
    });
    assert_eq!(templates.len(), 2);
    let ids: Vec<String> = templates.iter().map(|object| object.id.clone()).collect();
    // Only the two templates are listed.
    must(&mut *cx, &scene, Press("tab-search"));
    must(&mut *cx, &scene, Type("search-text", "Custom template"));
    must(&mut *cx, &scene, Press("search-apply"));
    let rows: Vec<String> = rows_in(cx, &scene, SEARCH)
        .into_iter()
        .map(|(key, _)| key)
        .collect();
    assert_eq!(rows.len(), 2, "{rows:?}");
    let listed: Vec<String> = rows.iter().map(|row| object_of(row)).collect();
    assert!(ids.iter().all(|id| listed.contains(id)), "{rows:?}");
    let selected = |cx: &mut TestAppContext| -> Vec<String> {
        model(cx, &scene, |m| {
            let target = m.target();
            assert!(
                target.all_of(xenadmin_ui::ObjectKind::Template),
                "{target:?}"
            );
            target
                .objects
                .iter()
                .map(|object| object.id.clone())
                .collect()
        })
    };

    // Both rows selected with the keyboard, the menu opened with the Menu
    // key, Properties chosen from it.
    press_on(cx, &scene, &rows[0]);
    must(&mut *cx, &scene, Key("shift-down"));
    assert_eq!(selected(cx), listed);
    must(&mut *cx, &scene, Key("menu"));
    let items = open_menu_items(cx, &scene);
    assert!(
        items.contains(&(PROPERTIES, None)),
        "the Menu key's menu of two custom templates: {items:?}"
    );
    assert!(popup_labels(cx, &scene).contains(&"Properties".to_owned()));
    choose_from_popup(cx, &scene, PROPERTIES);
    must(&mut *cx, &scene, Front("template-properties"));
    must(&mut *cx, &scene, Says("Bulk properties"));
    must(&mut *cx, &scene, Press("batch-properties-close"));
    assert_eq!(front(cx, &scene), None);

    // The menu is open on both templates; the selection becomes one of
    // them. The shell takes the stale menu off the screen, so no item of
    // it can be pressed, and the model refuses the item as well.
    press_on(cx, &scene, &rows[0]);
    must(&mut *cx, &scene, Key("shift-down"));
    assert_eq!(selected(cx), listed);
    must(&mut *cx, &scene, Key("menu"));
    assert!(open_menu_items(cx, &scene).contains(&(PROPERTIES, None)));
    assert!(on(cx, &scene, |window, _| window
        .try_find("popup-menu")
        .is_some()));
    let one = templates[1].clone();
    common::run(cx, &scene, move |m| {
        m.select_in_grid(xenadmin_ui::Target::one(one));
    });
    cx.run_until_parked();
    assert!(
        on(cx, &scene, |window, _| window
            .try_find("popup-menu")
            .is_none()),
        "the menu of a selection that changed is still on screen"
    );
    let mut refused = false;
    common::run(cx, &scene, |m| {
        refused = m
            .invoke_context(xenadmin_ui::CommandId(PROPERTIES))
            .is_err();
    });
    assert!(refused, "the stale menu's item ran");
    assert_eq!(front(cx, &scene), None, "a stale menu item opened a dialog");
    assert!(model(cx, &scene, |m| m.properties.is_none()));
    assert_eq!(selected(cx), [listed[1].clone()]);
    close(cx, scene);
}

/// migration, wish 37: from the first step to the review by key.
#[gpui_kit::test]
fn destination_and_mapping_controls_produce_reviewed_typed_plan(cx: &mut TestAppContext) {
    at_sizes(
        cx,
        "vm-migration",
        &[SMALL],
        &[
            Press("migration-back"),
            Press("migration-back"),
            // Another host than the review of the scene had.
            Press("migration-host-20000000-0000-0000-0000-000000000003"),
            Checked("migration-host-20000000-0000-0000-0000-000000000003", true),
            Inside(&["migration-next"]),
            Press("migration-next"),
            Reach(&["migration-disk-0", "migration-nic-0", "migration-transfer"]),
            Inside(&["migration-next", "migration-back"]),
            Press("migration-next"),
            Label("migration-dialog-steps", "Step 3 of 3"),
            Inside(&["migration-apply"]),
        ],
    );
}

/// host_services, wish 79: 65 crashdumps are three pages, and the last
/// one can be reached and pressed in the smallest window.
#[gpui_kit::test]
fn crashdump_catalogue_pages_make_every_captured_candidate_reachable(cx: &mut TestAppContext) {
    let scene = common::open_scene_with(cx, "host-support", SMALL, |m| {
        let overview = m
            .areas
            .host_services
            .support
            .as_mut()
            .and_then(|panel| panel.overview.as_mut())
            .expect("the overview of the scene");
        overview.crashdumps = (1..=65u128)
            .map(|id| {
                serde_json::from_value(serde_json::json!({
                    "uuid": uuid::Uuid::from_u128(id),
                    "timestamp": "20261108T00:00:00Z",
                    "size": 4096,
                }))
                .expect("a crashdump record")
            })
            .collect();
    });
    let last = format!("host-crashdump-delete-{}", uuid::Uuid::from_u128(65));
    must(&mut *cx, &scene, Says("Page 1 of 3"));
    assert!(!has(cx, &scene, &last));
    must(&mut *cx, &scene, Press("host-crashdump-next"));
    must(&mut *cx, &scene, Says("Page 2 of 3"));
    must(&mut *cx, &scene, Press("host-crashdump-next"));
    must(&mut *cx, &scene, Says("Page 3 of 3"));
    press_on(cx, &scene, &last);
    // The review of that one dump, with its footer in the window.
    must(
        &mut *cx,
        &scene,
        Inside(&["host-crashdump-submit", "host-support-close"]),
    );
    must(
        &mut *cx,
        &scene,
        Says(&*uuid::Uuid::from_u128(65).to_string().leak()),
    );
    assert_eq!(submitted(cx, &scene), 0);
    close(cx, scene);
}

/// The activity scene with an operation of unknown outcome in its History:
/// an interrupted join of another pool to the demo pool. The History list
/// is shown and the row of the join is pressed.
fn activity_with_a_join(cx: &mut TestAppContext, size: (f32, f32)) -> ShellWindow {
    let source = uuid::Uuid::from_u128(0xa000_0000_0000_4000_8000_0000_0000_0001);
    let scene = common::open_scene_with(cx, "activity", size, |m| {
        let destination = m.inventory.as_ref().expect("the demo inventory").pool_id;
        let record = serde_json::from_value(serde_json::json!({
            "id": uuid::Uuid::from_u128(77),
            "pool": source,
            "target": {"Pool": source},
            "action": {"Pool": {"Join": {
                "source_host": uuid::Uuid::from_u128(78),
                "destination_pool": destination,
                "destination_coordinator": uuid::Uuid::from_u128(79),
                "destination_address": "192.0.2.20",
            }}},
            "state": "OutcomeUnknown",
            "progress": null,
            "detail": "Fixture interrupted join",
            "created_at_ms": 0,
            "can_cancel": false,
            "cancel_requested": false,
            "steps": [],
            "created": null,
        }))
        .expect("a join record");
        m.resource_rows.push(record);
    });
    must(&mut *cx, &scene, Press("activity-History"));
    let rows = rows_in(cx, &scene, "activity-table");
    let mut found = false;
    for (key, _) in &rows {
        press_on(cx, &scene, key);
        if says(cx, &scene, "Fixture interrupted join") {
            found = true;
            break;
        }
    }
    assert!(found, "no History row is the join: {rows:?}");
    scene
}

/// events, wish 13: an interrupted join of another pool to this one is a
/// row of the History list here, the destination.
#[gpui_kit::test]
fn joined_pool_history_is_visible_at_destination_and_acknowledges_both_reservations(
    cx: &mut TestAppContext,
) {
    let scene = activity_with_a_join(cx, WIDE);
    must(&mut *cx, &scene, Says("Fixture interrupted join"));
    close(cx, scene);
}

/// events, wish 11: with an operation of unknown outcome selected, the
/// explanation opens from the local row and its Close closes it.
#[gpui_kit::test]
fn the_outcome_explanation_opens_for_a_selected_unknown_operation(cx: &mut TestAppContext) {
    for size in BOTH {
        let scene = activity_with_a_join(cx, *size);
        must(&mut *cx, &scene, Press("local:events.explain"));
        must(&mut *cx, &scene, Front("failure-help-dialog"));
        must(&mut *cx, &scene, Says("Outcome explanation"));
        must(&mut *cx, &scene, Reach(&["failure-help-outcome"]));
        must(&mut *cx, &scene, Inside(&["failure-help-close"]));
        // What it says is its own: the selection of the table behind it
        // changes and the dialog keeps its content.
        let before = described(cx, &scene);
        common::run(cx, &scene, |m| {
            m.areas.events.search = "no such entry".into()
        });
        assert_eq!(described(cx, &scene), before);
        must(&mut *cx, &scene, Press("failure-help-close"));
        must(&mut *cx, &scene, NoSurface);
        assert_eq!(submitted(cx, &scene), 0);
        close(cx, scene);
    }
}

/// snapshots, wish 28: the preview is drawn with the information, in the
/// proportions of the picture, and can be brought wholly into view.
#[gpui_kit::test]
fn the_snapshot_preview_is_drawn_undistorted_inside_the_information(cx: &mut TestAppContext) {
    for size in BOTH {
        let scene = open_scene(cx, "snapshot-information", *size);
        must(&mut *cx, &scene, Reach(&["snapshot-preview-image"]));
        let (image, information) = on(cx, &scene, |window, _| {
            let bounds = |key| {
                common::locate(window, None, key)
                    .unwrap_or_else(|error| panic!("{error}"))
                    .bounds()
            };
            (
                bounds("snapshot-preview-image"),
                bounds("snapshot-information"),
            )
        });
        let (width, height) = (f32::from(image.size.width), f32::from(image.size.height));
        assert!(width > 100. && width <= 320.5, "{image:?} at {size:?}");
        assert!(
            (width / height - 320. / 180.).abs() < 0.02,
            "the picture is distorted at {size:?}: {image:?}"
        );
        // The element with the key is the line that starts the
        // information; the picture belongs to what follows it.
        assert!(
            image.top() >= information.top() && image.left() >= information.left(),
            "{image:?} is not part of {information:?}"
        );
        close(cx, scene);
    }
}
