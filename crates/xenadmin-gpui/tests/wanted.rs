//! The presence and reachability wishes of the area engineers
//! (`gpui-tests-wanted.md`), as one table: a scene, the sizes it is asked
//! at, what must be inside the window there and what must be reachable by
//! scrolling the body it is in.
//!
//! Each row is the window half of a Rune test whose model half is a test of
//! `xenadmin-ui`. The Rune client's compact size, 513 by 652, does not exist
//! any more: a wish "at 513x652" is checked at 800 by 600, the smallest size
//! the main window can have. The footer buttons of a surface must be inside
//! the window at both sizes; a body may scroll.
//!
//! The wishes about what a press, a key or a file prompt does are tests of
//! their own in `wanted_events.rs`. These tests do not establish Wayland
//! input or a screen reader: the platform is the Kit's test platform.
#![cfg(feature = "scenes")]

mod common;

use common::Step::*;
use common::{BOTH, DEMO, ONLY_SMALL, Wish, run_wish};
use xenadmin_gpui::kit::testing::TestAppContext;

const fn row(
    area: &'static str,
    about: &'static str,
    scene: &'static str,
    sizes: &'static [(f32, f32)],
    steps: &'static [common::Step],
) -> Wish {
    Wish {
        area,
        about,
        scene,
        sizes,
        steps,
    }
}

/// The footer of the six transfer scenes that stand on a form step.
const TRANSFER_FORM: &[common::Step] = &[
    Front("transfer-dialog"),
    Inside(&["transfer-dialog-steps", "transfer-close", "transfer-next"]),
];
/// And of those that stand on the review.
const TRANSFER_REVIEW: &[common::Step] = &[
    Front("transfer-dialog"),
    Inside(&[
        "transfer-dialog-steps",
        "transfer-close",
        "transfer-back",
        "transfer-apply",
    ]),
    Reach(&["transfer-summary"]),
    Disabled("transfer-apply", ""),
];
/// The review of a change of the management interface.
const MANAGEMENT_REVIEW: &[common::Step] = &[
    Front("network-change"),
    Inside(&["network-confirm", "network-close"]),
    Disabled("network-confirm", DEMO),
];
const BATCH_EXPORT_REVIEW: &[common::Step] = &[
    Front("batch-export"),
    Inside(&[
        "review-permission-explanation",
        "batch-export-close",
        "batch-export-back",
        "batch-export-submit",
    ]),
];
const FABRIC_REVIEW: &[common::Step] = &[
    Front("network-fabric"),
    Label("network-fabric-steps", "Review"),
    Inside(&["network-confirm"]),
    Reach(&["network-services-review"]),
];

const WISHES: &[Wish] = &[
    // -- vm_properties --------------------------------------------------------
    row(
        "vm_properties",
        "the page list, the fields and the footer of the properties",
        "properties",
        BOTH,
        &[
            Front("vm-properties"),
            Inside(&[
                "vm-properties-pages",
                "properties-cancel",
                "review-properties",
            ]),
            Reach(&["property-name"]),
        ],
    ),
    row(
        "vm_properties",
        "the properties of a running guest",
        "properties-running",
        BOTH,
        &[
            Front("vm-properties"),
            Inside(&[
                "vm-properties-pages",
                "properties-cancel",
                "review-properties",
            ]),
            Reach(&["property-live-cpus"]),
        ],
    ),
    row(
        "vm_properties",
        "the review of a home server change",
        "home-host",
        BOTH,
        &[
            Front("vm-properties"),
            Inside(&[
                "vm-properties-pages",
                "properties-cancel",
                "properties-back",
                "apply-properties",
            ]),
            Reach(&["properties-review"]),
        ],
    ),
    row(
        "vm_properties",
        "the secure boot page",
        "secure-boot",
        BOTH,
        &[
            Front("vm-properties"),
            Inside(&[
                "vm-properties-pages",
                "properties-cancel",
                "review-properties",
            ]),
            Reach(&["property-secure-boot"]),
        ],
    ),
    // -- snapshots ------------------------------------------------------------
    row(
        "snapshots",
        "snapshot_metadata_creation_is_editable_at_compact_and_wide_sizes",
        "snapshot-create",
        BOTH,
        &[
            Front("snapshot-create"),
            Reach(&["snapshot-name", "snapshot-description", "confirm-snapshot"]),
            Inside(&["cancel-snapshot", "confirm-snapshot"]),
            Disabled("confirm-snapshot", ""),
        ],
    ),
    row(
        "snapshots",
        "the preview picture of the snapshot information",
        "snapshot-information",
        BOTH,
        &[
            NoSurface,
            Reach(&["snapshot-information", "snapshot-preview-image"]),
            Disabled("snapshot-preview-load", ""),
        ],
    ),
    row(
        "snapshots",
        "review_permission_notice_is_reachable_without_bypassing_synthetic_target_guard",
        "snapshot-restore",
        BOTH,
        &[
            Front("snapshot-restore"),
            Inside(&["review-permission-explanation", "confirm-snapshot"]),
            Disabled("confirm-snapshot", ""),
            Absent(&["experimental-notice"]),
        ],
    ),
    row(
        "snapshots",
        "dialogs_outside_the_supported_core_carry_the_experimental_notice",
        "snapshot-edit",
        BOTH,
        &[
            Front("snapshot-edit"),
            Inside(&["review-permission-explanation", "confirm-snapshot"]),
            Disabled("confirm-snapshot", ""),
            Absent(&["experimental-notice"]),
        ],
    ),
    // -- storage --------------------------------------------------------------
    row(
        "storage",
        "authenticated_storage_review_is_captured_and_footer_remains_reachable",
        "storage-auth-review",
        BOTH,
        &[Front("storage-new"), Inside(&["storage-confirm"])],
    ),
    row(
        "storage",
        "discovery_selection_is_reachable_bound_to_inputs_and_keeps_review_separate",
        "storage-discovery",
        BOTH,
        &[
            Reach(&["storage-result-0"]),
            Press("storage-result-0"),
            Inside(&["storage-review"]),
        ],
    ),
    row(
        "storage",
        "storage_review_preserves_inputs_and_compact_footer_never_submits_preview",
        "storage-review",
        BOTH,
        &[Front("storage-new"), Inside(&["storage-confirm"])],
    ),
    row(
        "storage",
        "local_form_exposes_declaration_and_erase_acknowledgement_at_both_widths",
        "storage-local",
        BOTH,
        &[Reach(&["storage-local-ack", "storage-review"])],
    ),
    row(
        "storage",
        "local_review_captures_...: the device confirmation and the footer",
        "storage-local-review",
        BOTH,
        &[
            Reach(&["storage-confirm-device"]),
            Inside(&["storage-confirm"]),
        ],
    ),
    row(
        "storage",
        "hba_selection_requires_review_and_captures_host_device_and_allocation_at_both_widths",
        "storage-hba",
        BOTH,
        &[
            Press("storage-result-0"),
            Press("storage-review"),
            Reach(&["storage-confirm-lun"]),
            Described("storage-confirm-lun", "confirm formatting"),
            Inside(&["storage-confirm"]),
        ],
    ),
    // -- migration ------------------------------------------------------------
    row(
        "migration",
        "compact_and_wide_review_capture_exact_targets_and_remain_preview_only",
        "vm-migration",
        BOTH,
        &[
            Front("migration-dialog"),
            Label("migration-dialog-steps", "Step 3 of 3"),
            Inside(&["review-permission-explanation", "migration-apply"]),
        ],
    ),
    row(
        "migration",
        "compact_and_wide_review_capture_all_vms_and_keep_safe_footer",
        "batch-move",
        BOTH,
        &[
            Front("batch-move"),
            Inside(&["review-permission-explanation", "batch-move-submit"]),
        ],
    ),
    // -- performance (the chart `metrics-chart` is another engineer's) --------
    row(
        "performance",
        "performance_controls_show_actual_coverage_and_change_ranges_without_host_writes",
        "performance",
        BOTH,
        &[
            NoSurface,
            Inside(&["metrics-window-604800"]),
            Press("metrics-window-604800"),
            Label("metrics-range", "3600 second samples"),
            Says("Synthetic performance preview"),
            Press("local:performance.data-sources"),
            Reach(&["apply-metric-sources"]),
        ],
    ),
    row(
        "performance",
        "save_apply_and_remove_presets_preserve_the_target_at_both_sizes",
        "performance",
        BOTH,
        &[
            Press("local:performance.presets"),
            Type("metric-preset-name", "night"),
            Press("metric-preset-save"),
            Reach(&["metric-preset-night", "metric-preset-remove-night"]),
            Press("metric-preset-night"),
            Press("local:performance.presets"),
            Press("metric-preset-remove-night"),
            Absent(&["metric-preset-night"]),
        ],
    ),
    row(
        "performance",
        "destructive_review_keeps_captured_source_and_reachable_controls_at_both_widths",
        "host-monitoring",
        BOTH,
        &[
            Front("host-monitoring"),
            Label("host-monitoring/title", "Host monitoring recording"),
            Inside(&[
                "host-monitoring/title",
                "monitoring-close",
                "monitoring-submit",
                "monitoring-history",
            ]),
            Reach(&["monitoring-confirm-history-loss"]),
            Checked("monitoring-confirm-history-loss", false),
            Press("monitoring-confirm-history-loss"),
            Checked("monitoring-confirm-history-loss", true),
            Press("monitoring-back"),
            Inside(&["monitoring-refresh", "monitoring-close"]),
        ],
    ),
    // -- vm_tools -------------------------------------------------------------
    row(
        "vm_tools",
        "the guest tools review after choosing an image",
        "guest-tools",
        BOTH,
        &[
            Front("tools-dialog"),
            Press("tools-image-*"),
            Press("tools-review"),
            Inside(&["tools-confirm", "tools-close"]),
            Disabled("tools-confirm", DEMO),
        ],
    ),
    row(
        "vm_tools",
        "the footer of the guest connections",
        "guest-connections",
        BOTH,
        &[
            Front("guest-connections"),
            Reach(&[
                "review-guest-tool",
                "launch-guest-tool",
                "configure-guest-tools",
                "close-guest-tools",
                "guest-tool-address",
            ]),
        ],
    ),
    row(
        "vm_tools",
        "the footer of the tool declarations",
        "external-tools",
        BOTH,
        &[
            Front("external-tools"),
            Inside(&["save-guest-tools", "remove-guest-tool", "close-guest-tools"]),
            Reach(&["guest-tool-arguments"]),
        ],
    ),
    // -- storage_maintenance --------------------------------------------------
    row(
        "storage_maintenance",
        "repair_review_captures_targets_and_remains_reachable_at_compact_and_wide_sizes",
        "storage-repair",
        BOTH,
        &[Inside(&["storage-maintenance-confirm", "storage-close"])],
    ),
    row(
        "storage_maintenance",
        "reviewed_storage_graph_is_captured_reachable_and_never_submits_demo",
        "storage-graph",
        BOTH,
        &[Inside(&["storage-graph-close", "storage-graph-submit"])],
    ),
    row(
        "storage_maintenance",
        "reviewed_storage_graph_is_captured_reachable_and_never_submits_demo (multipath)",
        "storage-multipath",
        BOTH,
        &[Inside(&["storage-graph-close", "storage-graph-submit"])],
    ),
    row(
        "storage_maintenance",
        "the cache review: back to the form and review again",
        "storage-cache",
        BOTH,
        &[
            Press("storage-maintenance-back"),
            Press("storage-cache-review"),
            Inside(&["storage-maintenance-confirm"]),
        ],
    ),
    row(
        "storage_maintenance",
        "a connection is chosen for repair and reviewed",
        "storage-maintenance",
        BOTH,
        &[
            Press("repair-pbd-*"),
            Press("storage-repair-review"),
            Inside(&["storage-maintenance-confirm"]),
        ],
    ),
    row(
        "storage_maintenance",
        "the storage graph: back, another host, review",
        "storage-graph",
        ONLY_SMALL,
        &[
            Press("storage-graph-back"),
            Press("storage-host-*"),
            Press("storage-graph-review"),
            Inside(&["storage-graph-submit"]),
        ],
    ),
    // -- transfers ------------------------------------------------------------
    row(
        "transfers",
        "archive export",
        "archive-export",
        BOTH,
        TRANSFER_FORM,
    ),
    row(
        "transfers",
        "archive export: the destination step",
        "archive-export",
        BOTH,
        &[
            Press("transfer-next"),
            Reach(&["transfer-choose"]),
            Inside(&["transfer-close", "transfer-back"]),
        ],
    ),
    row(
        "transfers",
        "XVA compression",
        "xva-compression",
        BOTH,
        TRANSFER_FORM,
    ),
    row(
        "transfers",
        "checkpoint export",
        "checkpoint-export",
        BOTH,
        TRANSFER_FORM,
    ),
    row(
        "transfers",
        "disk export",
        "disk-export",
        BOTH,
        TRANSFER_FORM,
    ),
    row(
        "transfers",
        "OVF import review",
        "ovf-import",
        BOTH,
        TRANSFER_REVIEW,
    ),
    row(
        "transfers",
        "OVF export review",
        "ovf-export",
        BOTH,
        TRANSFER_REVIEW,
    ),
    row(
        "transfers",
        "offline_form_and_review_keep_heading_and_final_controls_reachable_without_host_access",
        "local-conversion",
        BOTH,
        &[
            Label("local-conversion/title", "Local disk conversion"),
            Inside(&[
                "local-conversion/title",
                "conversion-close",
                "conversion-prepare",
            ]),
        ],
    ),
    row(
        "transfers",
        "offline_form_and_review_...: the review",
        "local-conversion-review",
        BOTH,
        &[
            Label("local-conversion/title", "Local disk conversion"),
            Inside(&[
                "local-conversion/title",
                "conversion-close",
                "conversion-submit",
            ]),
        ],
    ),
    // -- vm_devices -----------------------------------------------------------
    row(
        "vm_devices",
        "compact_and_wide_copy_review_capture_target_and_block_demo_dispatch",
        "disk-copy",
        BOTH,
        &[
            Front("virtual-disk"),
            Inside(&["virtual-disk/title", "disk-confirm", "disks-close"]),
            Disabled("disk-confirm", ""),
        ],
    ),
    row(
        "vm_devices",
        "cleanup_review_keeps_exact_uuid_and_explicit_destructive_confirmation_at_both_sizes",
        "disk-cleanup",
        BOTH,
        &[
            Front("virtual-disk"),
            Inside(&["virtual-disk/title", "disk-confirm", "disks-close"]),
            Reach(&["disk-delete-confirmation"]),
            Disabled("disk-confirm", ""),
        ],
    ),
    row(
        "vm_devices",
        "the controls of the device properties",
        "device-properties",
        BOTH,
        &[
            Front("device-properties"),
            Reach(&["property-value", "property-review", "property-cancel"]),
        ],
    ),
    // -- pool -----------------------------------------------------------------
    row(
        "pool",
        "review_captures_target_and_compact_footer_never_submits_demo",
        "pool-policy-review",
        BOTH,
        &[Inside(&["pool-confirm"])],
    ),
    row(
        "pool",
        "ha_policy_review_binds_vm_and_shared_vapp_settings_at_both_sizes",
        "ha-policy",
        BOTH,
        &[Inside(&["pool-confirm"])],
    ),
    row(
        "pool",
        "the dissolve review",
        "pool-dissolve",
        BOTH,
        &[
            Inside(&["review-permission-explanation", "pool-dissolve-confirm"]),
            Reach(&["pool-dissolve-identity"]),
        ],
    ),
    row(
        "pool",
        "the join review",
        "pool-join",
        BOTH,
        &[Inside(&["join-submit"]), Reach(&["join-password"])],
    ),
    row(
        "pool",
        "the root password review",
        "root-password",
        BOTH,
        &[
            Inside(&["root-password-submit"]),
            Absent(&["root-password-new"]),
        ],
    ),
    // -- network --------------------------------------------------------------
    row(
        "network",
        "ipv6_review_is_visible_at_both_widths_and_keeps_captured_identity",
        "network-ipv6",
        BOTH,
        &[
            Inside(&["network-review"]),
            Press("network-review"),
            Inside(&["network-confirm"]),
            Says("DNS is shared"),
        ],
    ),
    row(
        "network",
        "network_review_captures_pool_name_mtu_and_never_submits_from_preview",
        "network-create",
        BOTH,
        &[
            Inside(&["resource-confirm", "resource-back"]),
            Press("resource-back"),
            Press("resource-next"),
            Press("resource-next"),
            Label("network-new-steps", "Settings"),
            Inside(&["resource-review"]),
        ],
    ),
    row(
        "network",
        "compact_network_controls_remain_inside_the_window (its successor: the local row)",
        "pool-administration",
        ONLY_SMALL,
        &[
            NoSurface,
            Inside(&[
                "local:network.new",
                "local:network.vlan.add",
                "local:network.remove",
                "local:nic.ipv4",
                "local:nic.ipv6",
                "local:nic.purpose",
                "local:server.management.configure",
                "local:server.management.move",
            ]),
        ],
    ),
    row(
        "network",
        "compact_and_wide_network_reviews_remain_visible_and_preview_cannot_submit",
        "network-interfaces",
        BOTH,
        &[
            Front("network-change"),
            Set(|m| {
                xenadmin_ui::areas::network::workspace::set_form(
                    m,
                    xenadmin_ui::areas::network::workspace::Form::Mtu {
                        value: "9000".into(),
                    },
                );
            }),
            Inside(&["network-review"]),
            Press("network-review"),
            Inside(&["network-confirm"]),
            Disabled("network-confirm", DEMO),
        ],
    ),
    // -- bulk_edit ------------------------------------------------------------
    row(
        "bulk_edit",
        "compact_and_wide_reviews_keep_exact_targets_and_the_confirmation_footer",
        "batch-delete",
        BOTH,
        &[
            Front("batch-delete"),
            Inside(&["batch-delete-close", "batch-delete-submit"]),
            Reach(&["batch-delete-confirm"]),
        ],
    ),
    row(
        "bulk_edit",
        "property_batch_multiselect_review_is_immutable_and_footer_reachable_at_both_sizes",
        "batch-properties",
        BOTH,
        &[
            Front("vm-properties"),
            Inside(&["batch-properties-submit", "review-permission-explanation"]),
        ],
    ),
    // -- bulk_export ----------------------------------------------------------
    row(
        "bulk_export",
        "compact_and_wide_export_review_keeps_exact_targets_and_safe_footer",
        "batch-export",
        BOTH,
        BATCH_EXPORT_REVIEW,
    ),
    row(
        "bulk_export",
        "review_names_each_snapshot_its_memory_choice_output_and_format_with_a_reachable_footer",
        "snapshot-batch-export",
        BOTH,
        BATCH_EXPORT_REVIEW,
    ),
    // -- host_services --------------------------------------------------------
    row(
        "host_services",
        "review_has_private_password_and_captured_target_at_both_widths",
        "host-power-settings",
        BOTH,
        &[Inside(&["host-power-confirm"])],
    ),
    row(
        "host_services",
        "host_archive_review_captures_target_and_keeps_compact_footer_visible (the form)",
        "host-support",
        BOTH,
        &[
            Reach(&["host-support-file"]),
            Inside(&["host-support-review", "host-support-close"]),
        ],
    ),
    row(
        "host_services",
        "crashdump_delete_review_captures_exact_dump_and_keeps_footer_visible",
        "host-crashdump-delete",
        BOTH,
        &[
            Inside(&["host-crashdump-submit", "host-support-close"]),
            Reach(&["host-crashdump-confirmation"]),
        ],
    ),
    row(
        "host_services",
        "restore_native_open_inspects_captured_host_and_requires_exact_confirmation",
        "host-restore",
        BOTH,
        &[
            Inside(&["host-support-submit", "host-support-close"]),
            Reach(&["host-restore-confirmation"]),
        ],
    ),
    // -- schedules_alarms -----------------------------------------------------
    row(
        "schedules_alarms",
        "schedule_review_captures_coordinator_and_pool_at_both_widths",
        "snapshot-schedules",
        BOTH,
        &[
            Inside(&["policy-next"]),
            Press("policy-next"),
            Inside(&["policy-next"]),
            Press("policy-next"),
            Inside(&["policy-review"]),
            Press("policy-review"),
            Inside(&["policy-confirm", "policy-back"]),
            Says("Creates an empty, disabled schedule."),
        ],
    ),
    row(
        "schedules_alarms",
        "alarm_review_converts_percent_and_retains_original_pool_at_both_widths",
        "alarms",
        BOTH,
        &[
            Inside(&["alarm-review"]),
            Press("alarm-review"),
            Inside(&["alarm-confirm", "alarm-back"]),
        ],
    ),
    // -- organization ---------------------------------------------------------
    row(
        "organization",
        "compact_review_is_reachable_and_cannot_submit_demo_metadata",
        "organization",
        ONLY_SMALL,
        &[
            Inside(&["organization-review"]),
            Type("organization-name", "Night shift"),
            Press("organization-review"),
            Inside(&["organization-apply"]),
        ],
    ),
    row(
        "organization",
        "compact_review_keeps_controls_visible_and_refuses_preview_submission",
        "appliance-review",
        ONLY_SMALL,
        &[
            Inside(&["appliance-apply"]),
            Reach(&["appliance-confirmation"]),
            Above("appliance-confirmation", "appliance-apply"),
        ],
    ),
    // -- cross_pool -----------------------------------------------------------
    row(
        "cross_pool",
        "the cross-pool review",
        "cross-pool",
        BOTH,
        &[
            Front("cross-pool"),
            Inside(&["cross-close", "cross-back", "cross-apply"]),
            Disabled("cross-apply", DEMO),
            Reach(&["cross-transport-reviewed"]),
            Says("transport"),
        ],
    ),
    row(
        "cross_pool",
        "the batch cross-pool review",
        "batch-cross-pool",
        BOTH,
        &[
            Front("batch-cross-pool"),
            Inside(&[
                "experimental-notice",
                "batch-cross-pool-close",
                "batch-cross-pool-back",
                "batch-cross-pool-submit",
            ]),
            Reach(&[
                "review-permission-explanation",
                "review-permission-destination",
                "batch-cross-transport-reviewed",
            ]),
        ],
    ),
    // -- pool_security --------------------------------------------------------
    row(
        "pool_security",
        "compact_review_keeps_footer_visible_and_captured_identity_immutable",
        "pool-security-review",
        BOTH,
        &[Inside(&["security-confirm"])],
    ),
    // -- network_services -----------------------------------------------------
    row(
        "network_services",
        "review_captures_plaintext_policy_and_interfaces_at_both_widths",
        "network-services",
        BOTH,
        &[
            Front("network-change"),
            Inside(&["network-confirm"]),
            Label("network-confirm", "Confirm network change"),
            Disabled("network-confirm", DEMO),
        ],
    ),
    row(
        "network_services",
        "pool_vlan_review_captures_every_host_at_compact_and_wide_sizes_without_demo_dispatch",
        "network-topology",
        BOTH,
        &[
            Front("network-topology"),
            Inside(&["network-confirm"]),
            Above("network-services-review", "network-confirm"),
        ],
    ),
    row(
        "network_services",
        "host_scoped_tunnel_review_is_visible_at_both_sizes_and_demo_cannot_submit",
        "network-fabric",
        BOTH,
        FABRIC_REVIEW,
    ),
    row(
        "network_services",
        "sriov_review_names_driver_impact_and_reboot_boundary_at_both_sizes",
        "network-sriov",
        BOTH,
        FABRIC_REVIEW,
    ),
    row(
        "network_services",
        "exact_order_and_all_hosts_are_reviewed_at_both_widths_and_demo_cannot_apply",
        "network-fabric-pool",
        BOTH,
        &[
            Front("network-fabric-pool"),
            Label("network-fabric-pool-steps", "Review"),
            Inside(&["network-confirm"]),
        ],
    ),
    row(
        "network_services",
        "input_editor_requires_all_missing_hosts_and_preserves_exact_selection",
        "network-fabric-pool-select",
        BOTH,
        &[
            Front("network-fabric-pool"),
            Label("network-fabric-pool-steps", "Settings"),
            Inside(&["network-review"]),
            Label("network-review", "Review network change"),
            Press("network-review"),
            Label("network-fabric-pool-steps", "Review"),
        ],
    ),
    // -- management_network ---------------------------------------------------
    row(
        "management_network",
        "management_review_captures_recovery_settings_at_both_widths_without_demo_dispatch",
        "management-network",
        BOTH,
        MANAGEMENT_REVIEW,
    ),
    row(
        "management_network",
        "automatic_address_review_captures_mode_expected_ip_and_does_not_dispatch_demo",
        "management-dhcp",
        BOTH,
        MANAGEMENT_REVIEW,
    ),
    row(
        "management_network",
        "vlan_over_bond_review_captures_full_topology_at_both_widths",
        "management-vlan",
        BOTH,
        MANAGEMENT_REVIEW,
    ),
    row(
        "management_network",
        "management_bond_review_has_exact_members_and_recovery_at_both_widths",
        "management-bond",
        BOTH,
        MANAGEMENT_REVIEW,
    ),
    row(
        "management_network",
        "management_vlan_bond_review_lists_recreated_ids_and_captures_parent_at_both_widths",
        "management-vlan-bond",
        BOTH,
        MANAGEMENT_REVIEW,
    ),
    row(
        "management_network",
        "ipv6_stage_review_renders_at_two_widths_and_keeps_demo_apply_disabled",
        "management-ipv6-bond",
        BOTH,
        MANAGEMENT_REVIEW,
    ),
    row(
        "management_network",
        "management_guest_impact_review_lists_moved_retained_and_no_eligible_guests",
        "management-bond-guests",
        BOTH,
        &[
            Front("network-change"),
            Says("Move network:"),
            Inside(&["network-confirm"]),
        ],
    ),
];

/// Runs the rows of one area and fails with every row that did not hold.
fn run_area(cx: &mut TestAppContext, area: &str) {
    let rows: Vec<&Wish> = WISHES.iter().filter(|wish| wish.area == area).collect();
    assert!(!rows.is_empty(), "no row is of the area {area}");
    let mut failed = Vec::new();
    let mut passed = 0;
    for wish in &rows {
        let failures = run_wish(cx, wish);
        if failures.is_empty() {
            passed += 1;
        }
        failed.extend(failures);
    }
    eprintln!("{area}: {passed} of {} rows hold", rows.len());
    assert!(
        failed.is_empty(),
        "{area}: {} of {} rows hold\n{}",
        passed,
        rows.len(),
        failed.join("\n")
    );
}

/// One test per area, so that the areas run side by side and a failure
/// names its area.
macro_rules! areas {
    ($($area:ident),* $(,)?) => {
        const AREAS: &[&str] = &[$(stringify!($area)),*];
        $(
            #[gpui_kit::test]
            fn $area(cx: &mut TestAppContext) {
                run_area(cx, stringify!($area));
            }
        )*
    };
}

areas!(
    vm_properties,
    snapshots,
    storage,
    migration,
    performance,
    vm_tools,
    storage_maintenance,
    transfers,
    vm_devices,
    pool,
    network,
    bulk_edit,
    bulk_export,
    host_services,
    schedules_alarms,
    organization,
    cross_pool,
    pool_security,
    network_services,
    management_network,
);

/// No row is left out: each belongs to an area that has a test, names a
/// registered scene and asks for something.
#[test]
fn every_row_of_the_table_runs() {
    let names = xenadmin_gpui::scenes::names();
    for wish in WISHES {
        assert!(
            AREAS.contains(&wish.area),
            "{}: no test runs this area",
            wish.area
        );
        assert!(names.contains(&wish.scene), "{}: not a scene", wish.scene);
        assert!(!wish.sizes.is_empty() && !wish.steps.is_empty());
    }
    eprintln!("the table has {} rows", WISHES.len());
}
