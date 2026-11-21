//! The tests of the area, in three parts.
//!
//! 1. **The Rune tests**: the ten of `vm_snapshots.rs`, the five of
//!    `snapshot_inspection.rs`, the three of `snapshot_capture.rs`, and this
//!    area's share of two tests of other Rune files that ran over its
//!    scenes. Each is named `rune_<the Rune name>` and asserts what the Rune
//!    test asserted; where the Rune test pressed a page button, the test
//!    here presses the command that replaced it. The Rune tests ran at two
//!    window sizes; a description has no size, so each runs once, and the
//!    typing and reachability at 513x652 are in `gpui-tests-wanted.md`.
//!    Retired: the "close is reachable" half of
//!    `snapshot_inspection_captures_identity_and_close_is_reachable_at_both_sizes`
//!    (the information is a section of the tab and has no Close) and the
//!    `show-details` and `open-snapshots` steps of
//!    `snapshot_widgets_review_captured_targets_and_demo_cannot_confirm_changes`
//!    (the details pane is gone; the tab strip is the shell's).
//! 2. **What the port changed**: the page buttons became commands, the
//!    information dialog became a section, the selection is the area's and
//!    takes a search result handed over through the grid.
//! 3. **The hazards** the map lists for these modules (section 11, 35 to
//!    51), where a fixture can reach them.
//!
//! No fixture has a current connection snapshot, so nothing here submits
//! successfully: the live submission, the capture workflow past `begin` and
//! the preview query are not exercised.

use std::sync::{Arc, atomic::Ordering, mpsc};

use image::ImageEncoder;
use xenadmin_core::{
    ClassAvailability, ConsoleFrame, Error, OperationAction, OperationId, OperationRecord,
    OperationState, PoolId, PowerAction, PowerState, SnapshotAction, SnapshotId, SnapshotOperation,
    SnapshotPreview, SnapshotWorkflow, VbdId, VdiId, VmId,
};

use super::commands::{self, DELETE, NEW_TEMPLATE, NEW_VM, PROPERTIES, REVERT, TAKE};
use super::edit::{self, Dialog, SnapshotReview};
use super::inspection::{self, Decoded, Panel};
use super::scenes::{self, FIRST, SECOND, amend, snapshot, vm};
use super::view::TABLE;
use super::{capture, shown_vm};
use crate::Harness;
use crate::prelude::*;

const CONFIRM: &str = "confirm-snapshot";

/// The Rune `snapshot_state`: the demo guest's Snapshots tab with "Before
/// upgrade" selected.
fn selected() -> Harness {
    let mut h = Harness::scene("snapshots");
    h.select_rows(TABLE, &[&snapshot(FIRST).to_string()]);
    h
}

fn dialog(h: &Harness) -> &Dialog {
    h.model.areas.snapshots.dialog.as_ref().expect("a dialog")
}

fn pool(h: &Harness) -> PoolId {
    h.model.inventory.as_ref().expect("inventory").pool_id
}

/// The confirm button is disabled, and pressing the handler anyway submits
/// nothing and leaves the dialog open.
fn assert_inert(h: &mut Harness) {
    assert!(h.try_click(CONFIRM).is_err(), "the confirm button is live");
    edit::submit(&mut h.model);
    assert!(h.model.operations.is_empty());
    assert!(h.model.areas.snapshots.dialog.is_some());
}

fn record(h: &Harness, state: OperationState) -> OperationRecord {
    serde_json::from_value(serde_json::json!({
        "id": OperationId::new(), "pool": pool(h), "vm": vm(),
        "action": OperationAction::Snapshot(SnapshotOperation::Workflow {
            operation: SnapshotWorkflow::Create {
                name: "Before upgrade".into(), description: String::new(), memory: false,
            },
        }),
        "state": state, "progress": null, "detail": "Verified",
    }))
    .expect("operation record")
}

// -- 1. The Rune tests ----------------------------------------------------------------

#[test]
fn rune_snapshot_metadata_creation_is_editable_at_compact_and_wide_sizes() {
    let mut h = selected();
    h.click(TAKE.0);
    assert_eq!(h.surface().id, "snapshot-create");
    h.type_text("snapshot-name", "Before upgrade");
    h.type_text("snapshot-description", "Reviewed rollback point");
    assert_eq!(h.model.areas.snapshots.name, "Before upgrade");
    assert_eq!(
        h.model.areas.snapshots.description,
        "Reviewed rollback point"
    );
    assert!(h.has_text("Application quiescing is not available on XCP-ng 8.1 and later"));
    assert_eq!(h.try_click(CONFIRM), Err(edit::DEMO.into()));
    assert_inert(&mut h);
}

#[test]
fn rune_restore_requires_explicit_shutdown_and_start_choices_and_captures_identity() {
    let mut h = Harness::scene("snapshots");
    amend(&mut h.model, |inventory| {
        let guest = inventory.vms.get_mut(&vm()).unwrap();
        guest.power_state = PowerState::Running;
        guest.allowed_actions.push(PowerAction::CleanShutdown);
    });
    h.select_rows(TABLE, &[&snapshot(FIRST).to_string()]);
    h.click(REVERT.0);
    assert!(matches!(
        dialog(&h),
        Dialog::Restore {
            expected_state: PowerState::Running,
            shutdown: false,
            start_after: false,
            ..
        }
    ));
    // A running VM cannot be restored without the explicit shutdown.
    assert!(h.has_text(edit::RESTORE_INVALID));
    h.click("restore-shutdown");
    h.click("restore-start");
    assert!(
        matches!(dialog(&h), Dialog::Restore { shutdown: true, start_after: true, snapshot, .. } if snapshot == "Before upgrade")
    );
    assert!(!h.has_text(edit::RESTORE_INVALID));
    scenes::select(&mut h.model, snapshot(SECOND));
    assert!(matches!(dialog(&h), Dialog::Restore { snapshot, .. } if snapshot == "Before upgrade"));
    assert_inert(&mut h);
}

#[test]
fn rune_snapshot_editor_keeps_reviewed_metadata_when_selection_changes() {
    let mut h = selected();
    h.click(PROPERTIES.0);
    assert_eq!(h.model.areas.snapshots.name, "Before upgrade");
    let description = format!("{}Extra context", h.model.areas.snapshots.description);
    h.type_text("snapshot-description", &description);
    scenes::select(&mut h.model, snapshot(SECOND));
    assert!(matches!(dialog(&h), Dialog::Edit { source } if source.snapshot == "Before upgrade"));
    assert!(
        h.model
            .areas
            .snapshots
            .description
            .contains("Extra context")
    );
    assert_inert(&mut h);
}

#[test]
fn rune_checkpoint_restore_offers_resume_and_retains_saved_memory() {
    let mut h = Harness::scene("snapshots");
    amend(&mut h.model, |inventory| {
        let mut snap = inventory.snapshots.get(&snapshot(FIRST)).unwrap().clone();
        snap.power_state = PowerState::Suspended;
        inventory.snapshots.insert(snap);
    });
    h.select_rows(TABLE, &[&snapshot(FIRST).to_string()]);
    h.click(REVERT.0);
    assert!(h.has_text("Resume restored memory after restore"));
    assert!(matches!(
        dialog(&h),
        Dialog::Restore {
            checkpoint: true,
            start_after: false,
            ..
        }
    ));
    h.click("restore-start");
    assert!(matches!(
        dialog(&h),
        Dialog::Restore {
            start_after: true,
            ..
        }
    ));
}

#[test]
fn rune_cloning_reviews_the_captured_snapshot_in_both_modes_and_stays_safe_in_demo() {
    for as_template in [false, true] {
        let mut h = selected();
        h.click(if as_template { NEW_TEMPLATE.0 } else { NEW_VM.0 });
        assert!(
            matches!(dialog(&h), Dialog::Clone { as_template: mode, .. } if *mode == as_template)
        );
        h.type_text("snapshot-clone-name", "Recovery copy");
        scenes::select(&mut h.model, snapshot(SECOND));
        assert!(
            matches!(dialog(&h), Dialog::Clone { source, .. } if source.snapshot == "Before upgrade")
        );
        assert_eq!(
            h.surface().title,
            if as_template {
                "Create template from snapshot"
            } else {
                "Create VM from snapshot"
            }
        );
        assert!(h.has_text("Recovery copy"));
        assert_inert(&mut h);
    }
}

#[test]
fn rune_deletion_reviews_only_captured_owned_disks_and_lists_retained_disks() {
    let mut h = Harness::scene("snapshots");
    let owned_id = VdiId::from(uuid::Uuid::from_u128(0x7000_0000_0000_0000_0000_0000_0000_0001));
    amend(&mut h.model, |inventory| {
        let prototype_disk = inventory.details.vdis.values().next().unwrap().clone();
        let prototype_attachment = inventory
            .details
            .vbds
            .values()
            .find(|vbd| vbd.kind == "Disk")
            .unwrap()
            .clone();
        for (i, shared) in [(1u128, false), (2, true)] {
            let id = VdiId::from(uuid::Uuid::from_u128(
                0x7000_0000_0000_0000_0000_0000_0000_0000 + i,
            ));
            let mut disk = prototype_disk.clone();
            disk.id = id;
            disk.name = if shared {
                "Retained shared disk"
            } else {
                "Owned recovery disk"
            }
            .into();
            disk.is_snapshot = Some(true);
            disk.sharable = Some(shared);
            disk.missing = Some(false);
            inventory.details.vdis.insert(id, disk);
            let mut attachment = prototype_attachment.clone();
            attachment.id = VbdId::from(uuid::Uuid::from_u128(
                0x8000_0000_0000_0000_0000_0000_0000_0000 + i,
            ));
            attachment.vm = None;
            attachment.snapshot = Some(snapshot(FIRST));
            attachment.vdi = Some(id);
            attachment.owned = true;
            inventory
                .details
                .vdi_attachments
                .insert(id, vec![attachment.id]);
            inventory.details.vbds.insert(attachment.id, attachment);
        }
    });
    h.select_rows(TABLE, &[&snapshot(FIRST).to_string()]);
    h.click(DELETE.0);
    assert!(
        matches!(dialog(&h), Dialog::Delete { disks, retained, delete_disks: false, .. }
            if disks.len() == 1 && disks[0].id == owned_id && retained.len() == 1)
    );
    assert!(h.has_text("Owned recovery disk"));
    assert!(h.has_text("Retained shared disk"));
    h.click("snapshot-delete-disks");
    assert!(matches!(
        dialog(&h),
        Dialog::Delete {
            delete_disks: true,
            ..
        }
    ));
    scenes::select(&mut h.model, snapshot(SECOND));
    assert!(
        matches!(dialog(&h), Dialog::Delete { source, disks, .. }
            if source.snapshot_id == snapshot(FIRST) && disks.len() == 1 && disks[0].id == owned_id)
    );
    amend(&mut h.model, |inventory| {
        inventory.details.vdis.get_mut(&owned_id).unwrap().sharable = Some(true);
    });
    assert!(h.has_text("Disk ownership changed after this review"));
    assert_eq!(h.try_click(CONFIRM), Err(edit::OWNERSHIP_CHANGED.into()));
    assert_inert(&mut h);
}

#[test]
fn rune_deletion_does_not_offer_cleanup_without_complete_disk_inventory() {
    let mut h = selected();
    amend(&mut h.model, |inventory| {
        inventory
            .details
            .classes
            .insert("vdi".into(), ClassAvailability::PermissionDenied);
    });
    h.click(DELETE.0);
    assert!(h.find("snapshot-delete-disks").is_none());
    assert!(h.has_text("disk cleanup is disabled"));
    edit::submit(&mut h.model);
    assert!(h.model.operations.is_empty());
}

#[test]
fn rune_explicit_checkpoint_block_overrides_advertised_memory_mode() {
    let mut h = selected();
    amend(&mut h.model, |inventory| {
        inventory
            .details
            .vms
            .entry(vm())
            .or_default()
            .allowed_operations = Some(vec!["checkpoint".into()]);
    });
    assert!(edit::memory_available(
        h.model.inventory.as_ref().unwrap(),
        vm()
    ));
    amend(&mut h.model, |inventory| {
        inventory
            .details
            .vms
            .get_mut(&vm())
            .unwrap()
            .blocked_operations
            .insert("checkpoint".into(), "Protected".into());
    });
    assert!(!edit::memory_available(
        h.model.inventory.as_ref().unwrap(),
        vm()
    ));
}

#[test]
fn rune_snapshot_review_requires_the_same_pool_parent_state_and_exact_capability() {
    let mut h = selected();
    let source = SnapshotReview::capture(&h.model).unwrap();
    assert!(!source.permits(&h.model, SnapshotAction::Clone));
    let id = source.snapshot_id;
    let change = |h: &mut Harness, change: &dyn Fn(&mut xenadmin_core::Snapshot)| {
        amend(&mut h.model, |inventory| {
            let mut snap = inventory.snapshots.get(&id).unwrap().clone();
            change(&mut snap);
            inventory.snapshots.insert(snap);
        });
    };
    change(&mut h, &|snap| {
        snap.allowed_actions.push(SnapshotAction::Clone)
    });
    assert!(source.permits(&h.model, SnapshotAction::Clone));
    assert!(!source.permits(&h.model, SnapshotAction::Delete));
    let other = VmId::from(uuid::Uuid::from_u128(
        0x3000_0000_0000_0000_0000_0000_0000_0001,
    ));
    change(&mut h, &|snap| snap.vm = Some(other));
    assert!(!source.permits(&h.model, SnapshotAction::Clone));
    change(&mut h, &|snap| {
        snap.vm = Some(vm());
        snap.power_state = PowerState::Suspended;
    });
    assert!(!source.permits(&h.model, SnapshotAction::Clone));
    change(&mut h, &|snap| snap.power_state = PowerState::Halted);
    assert!(source.permits(&h.model, SnapshotAction::Clone));
    amend(&mut h.model, |inventory| {
        inventory.pool_id = PoolId::from(uuid::Uuid::from_u128(
            0x2000_0000_0000_0000_0000_0000_0000_0099,
        ));
    });
    assert!(!source.permits(&h.model, SnapshotAction::Clone));
    assert!(!source.live(&h.model));
}

/// The model half of the Rune end-to-end test: from the tree to the tab,
/// the Create dialog, Cancel, a row and the restore review.
#[test]
fn rune_snapshot_widgets_review_captured_targets_and_demo_cannot_confirm_changes() {
    let mut model = Model::fixture();
    model.load_demo(3, 48);
    let pool = model.inventory.as_ref().unwrap().pool_id;
    let mut h = Harness::with(model);
    h.select(Target::one(ObjectRef::vm(pool, vm())));
    h.open_tab(TabId::Snapshots);
    assert_eq!(shown_vm(&h.model), Some((pool, vm())));
    assert_eq!(edit::shown_rows(&h.model).len(), 3);
    h.click(TAKE.0);
    assert_eq!(h.surface().id, "snapshot-create");
    h.type_text("snapshot-name", "Before upgrade");
    assert!(h.try_click(CONFIRM).is_err());
    assert!(h.model.operations.is_empty());
    assert!(
        h.model.areas.snapshots.dialog.is_some(),
        "demo confirmation stays disabled"
    );
    h.click("cancel-snapshot");
    assert!(h.model.areas.snapshots.dialog.is_none());
    h.select_rows(TABLE, &[&snapshot(FIRST).to_string()]);
    h.click(REVERT.0);
    assert!(
        matches!(dialog(&h), Dialog::Restore { .. }),
        "restore review was not opened"
    );
    assert!(h.has_text("Before upgrade"));
    assert!(h.has_text("Changes made since the snapshot will be lost"));
    // Changing the selection after opening review must not retarget the command.
    let second = edit::shown_rows(&h.model)[1].id;
    scenes::select(&mut h.model, second);
    assert!(matches!(dialog(&h), Dialog::Restore { snapshot, .. } if snapshot == "Before upgrade"));
}

/// The capture half of the Rune test. The information is a section of the
/// tab now: it shows the selected snapshot and follows the selection, where
/// the Rune dialog kept the snapshot it was opened for; a saved preview
/// still belongs to the snapshot it was loaded for.
#[test]
fn rune_snapshot_inspection_captures_identity_and_close_is_reachable_at_both_sizes() {
    let mut h = Harness::scene("snapshot-information");
    assert_eq!(edit::selected_ids(&h.model), [snapshot(SECOND)]);
    let section = h.node("snapshot-information");
    assert_eq!(section.label.as_deref(), Some("Snapshot information"));
    assert!(h.has_text("Parent snapshot"));
    assert!(h.has_text("reclaim"));
    assert!(h.has_text(&snapshot(SECOND).to_string()));
    assert!(h.find("snapshot-preview-image").is_some());
    assert!(h.find("snapshot-inspection-close").is_none());
    h.select_rows(TABLE, &[&snapshot(FIRST).to_string()]);
    assert!(h.has_text("Root snapshot"));
    assert!(
        h.model.areas.snapshots.inspection.is_none(),
        "the preview of another snapshot is dropped"
    );
    assert!(h.find("snapshot-preview-image").is_none());
    assert!(h.has_text(inspection::INITIAL));
}

#[test]
fn rune_tree_and_list_keep_selection_and_show_current_branch() {
    let mut h = Harness::scene("snapshot-ancestry");
    let chosen = edit::selected_ids(&h.model);
    assert_eq!(chosen, [snapshot(SECOND)]);
    assert!(h.has_text("current base"));
    assert!(h.has_text("└ "));
    h.click("snapshot-tree-toggle");
    assert!(!h.model.areas.snapshots.tree);
    assert!(!h.has_text("current base"));
    assert_eq!(edit::selected_ids(&h.model), chosen);
    h.click("snapshot-tree-toggle");
    assert!(h.model.areas.snapshots.tree);
    assert_eq!(edit::selected_ids(&h.model), chosen);
}

#[test]
fn rune_decoder_bounds_dimensions_and_supports_png_and_jpeg() {
    for format in [image::ImageFormat::Png, image::ImageFormat::Jpeg] {
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(2, 2, image::Rgb([20, 40, 60])))
            .write_to(&mut bytes, format)
            .unwrap();
        let image = SnapshotPreview {
            mime: if format == image::ImageFormat::Png {
                "image/png"
            } else {
                "image/jpeg"
            },
            bytes: bytes.into_inner().into(),
        };
        let result = inspection::decode(image).unwrap();
        assert_eq!((result.width, result.height), (2, 2));
        assert_eq!(result.rgba.len(), 16);
    }
    let mut encoded = Vec::new();
    image::codecs::png::PngEncoder::new(&mut encoded)
        .write_image(&vec![0; 4097 * 4], 4097, 1, image::ExtendedColorType::Rgba8)
        .unwrap();
    assert!(
        inspection::decode(SnapshotPreview {
            mime: "image/png",
            bytes: encoded.into()
        })
        .is_err()
    );
    assert!(
        inspection::decode(SnapshotPreview {
            mime: "image/png",
            bytes: vec![0; 20].into()
        })
        .is_err()
    );
}

#[test]
fn rune_closing_or_replacing_the_panel_does_not_release_a_running_decoder_slot() {
    let mut h = Harness::scene("snapshot-information");
    let (release, wait) = mpsc::channel();
    let receiver = h
        .model
        .areas
        .snapshots
        .decoder
        .submit(
            move || {
                wait.recv().unwrap();
                Ok(Decoded {
                    width: 1,
                    height: 1,
                    rgba: vec![0; 4],
                })
            },
            None,
        )
        .unwrap();
    h.model.areas.snapshots.inspection.as_mut().unwrap().decode = Some(receiver);
    // Closed: another row is selected. Replaced: a new panel for this one.
    h.select_rows(TABLE, &[&snapshot(FIRST).to_string()]);
    assert!(h.model.areas.snapshots.inspection.is_none());
    let (pool, vm) = shown_vm(&h.model).unwrap();
    h.model.areas.snapshots.inspection = Some(Panel::new(pool, vm, snapshot(FIRST)));
    let decoder = &h.model.areas.snapshots.decoder;
    assert!(
        decoder
            .submit(|| panic!("a second decoder must not start"), None)
            .is_err()
    );
    release.send(()).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while decoder.busy.load(Ordering::Acquire) {
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    let result = decoder
        .submit(
            || {
                Ok(Decoded {
                    width: 1,
                    height: 1,
                    rgba: vec![1; 4],
                })
            },
            None,
        )
        .unwrap()
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap()
        .unwrap();
    assert_eq!(result.rgba, vec![1; 4]);
}

#[test]
fn rune_preview_request_refuses_a_changed_or_disconnected_workspace() {
    let mut h = Harness::scene("snapshot-information");
    h.model.demo = false;
    inspection::request(&mut h.model);
    let p = h.model.areas.snapshots.inspection.as_ref().unwrap();
    assert!(p.query.is_none());
    assert!(p.observed.is_none());
    assert_eq!(p.message, Error::Stale.to_string());
}

#[test]
fn rune_cancelled_or_uncertain_snapshot_never_authorizes_preview_upload() {
    let h = Harness::scene("snapshots");
    let created = SnapshotId::from(uuid::Uuid::from_u128(3));
    let mut record = record(&h, OperationState::Succeeded);
    record.created_snapshot = Some(created);
    assert_eq!(capture::creation_result(&record).unwrap(), Some(created));
    record.cancel_requested = true;
    let stopped = capture::creation_result(&record);
    assert!(
        stopped.is_err(),
        "even a successful snapshot cannot continue a cancelled workflow"
    );
    assert!(
        stopped
            .unwrap_err()
            .contains("Inspect the Events tab of the Notifications view")
    );
    record.cancel_requested = false;
    for state in [
        OperationState::OutcomeUnknown,
        OperationState::AcknowledgedUnknown,
        OperationState::PartiallyCompleted,
        OperationState::Failed,
    ] {
        record.state = state;
        let refused = capture::creation_result(&record);
        assert!(refused
            .unwrap_err()
            .contains("on the Events tab of the Notifications view"));
    }
}

#[test]
fn rune_snapshot_capture_encoder_bounds_large_frames_and_rejects_invalid_data() {
    let frame = Arc::new(ConsoleFrame {
        width: 3200,
        height: 1800,
        rgba: vec![110; 3200 * 1800 * 4].into(),
    });
    let image = capture::encode(frame).unwrap();
    assert_eq!(image.dimensions(), (1600, 900));
    assert!(image.byte_len() < 8 * 1024 * 1024);
    assert!(
        capture::encode(Arc::new(ConsoleFrame {
            width: 20,
            height: 10,
            rgba: vec![1; 10].into()
        }))
        .is_err()
    );
}

#[test]
fn rune_snapshot_preview_opt_in_is_explicit_and_synthetic_confirm_cannot_dispatch() {
    let mut h = Harness::scene("snapshot-capture");
    assert!(h.model.areas.snapshots.capture.opt_in);
    h.click("snapshot-capture-option");
    assert!(!h.model.areas.snapshots.capture.opt_in);
    h.click("snapshot-capture-option");
    assert!(h.model.areas.snapshots.capture.opt_in);
    assert!(h.has_text("may contain sensitive"));
    assert_eq!(h.try_click(CONFIRM), Err(edit::DEMO.into()));
    assert_inert(&mut h);
    assert!(!h.model.areas.snapshots.capture.pending());
    assert!(!h.model.areas.snapshots.capture.busy());
}

/// This area's share of `main.rs`
/// `dialogs_outside_the_supported_core_carry_the_experimental_notice`: no
/// snapshot dialog and no snapshot scene carries the notice.
#[test]
fn rune_main_dialogs_outside_the_supported_core_carry_the_experimental_notice() {
    for spec in scenes::scenes() {
        let h = Harness::scene(spec.name);
        assert!(h.find(EXPERIMENTAL_NOTICE_KEY).is_none(), "{}", spec.name);
        assert!(
            h.surfaces().iter().all(|surface| !surface.experimental),
            "{}",
            spec.name
        );
    }
    for command in [NEW_VM, NEW_TEMPLATE, DELETE] {
        let mut h = selected();
        h.click(command.0);
        assert!(!h.surface().experimental, "{}", command.0);
    }
}

/// This area's share of `session_permissions_view.rs`
/// `review_permission_notice_is_reachable_without_bypassing_synthetic_target_guard`.
#[test]
fn rune_review_permission_notice_is_reachable_without_bypassing_synthetic_target_guard() {
    for scene in ["snapshot-restore", "snapshot-edit"] {
        let mut h = Harness::scene(scene);
        assert!(h.find(permissions::REVIEW_KEY).is_some(), "{scene}");
        assert_inert(&mut h);
    }
}

// -- 2. What the port changed -------------------------------------------------------

#[test]
fn every_scene_describes_its_surface_with_unique_keys_and_a_quiet_first_poll() {
    let surfaces = [
        ("snapshots", None),
        ("snapshot-create", Some("snapshot-create")),
        ("snapshot-restore", Some("snapshot-restore")),
        ("snapshot-edit", Some("snapshot-edit")),
        ("snapshot-capture", Some("snapshot-create")),
        ("snapshot-ancestry", None),
        ("snapshot-information", None),
    ];
    assert_eq!(scenes::scenes().len(), surfaces.len());
    for (scene, surface) in surfaces {
        let mut h = Harness::scene(scene);
        h.assert_unique_keys();
        assert_eq!(h.surface_id().as_deref(), surface, "{scene}");
        assert_eq!(h.model.active_tab(), TabId::Snapshots, "{scene}");
        assert!(!h.tick(0.1), "{scene}: nothing changed");
        assert_eq!(h.surface_id().as_deref(), surface, "{scene}");
    }
    // The tab itself, without a dialog in front.
    let h = selected();
    h.assert_unique_keys();
}

#[test]
fn the_page_buttons_are_commands_in_the_menu_the_local_row_and_the_row_menu() {
    let mut h = Harness::scene("snapshots");
    let local: Vec<&str> = h.local_row().iter().map(|item| item.id.0).collect();
    let ours: Vec<&str> = commands::commands().iter().map(|spec| spec.id.0).collect();
    assert_eq!(
        ours,
        [
            "vm.snapshot",
            "snapshot.revert",
            "snapshot.properties",
            "snapshot.new-vm",
            "snapshot.new-template",
            "snapshot.delete",
            "snapshot.stop-workflow",
            "snapshot.recheck-outcome",
            "snapshot.load-preview",
        ]
    );
    assert!(
        ours.iter().all(|id| {
            local.contains(id) || h.eligibility(id) == Eligibility::Hidden
        }),
        "{local:?}"
    );
    let item = h.menu_item(TAKE.0).expect("VM > Take a Snapshot");
    assert_eq!(item.label, "Take a Snapshot");
    assert_eq!(item.disabled, None);
    let guest = Target::one(ObjectRef::vm(pool(&h), vm()));
    assert!(h.context_menu(&guest).iter().any(|item| item.id == TAKE));
    assert!(h.context_menu(&guest).iter().all(|item| item.id != REVERT));
    let menu: Vec<&str> = h
        .open_row_menu(TABLE, &snapshot(SECOND).to_string())
        .iter()
        .map(|item| item.id.0)
        .collect();
    assert_eq!(menu, commands::ROW_MENU.map(|id| id.0));
    // The secondary press selected the row the menu is about.
    assert_eq!(edit::selected_ids(&h.model), [snapshot(SECOND)]);
    // A row is not a tree object: the item runs on the model's target.
    h.invoke(PROPERTIES.0);
    assert!(matches!(dialog(&h), Dialog::Edit { source } if source.snapshot_id == snapshot(SECOND)));
}

#[test]
fn row_actions_take_exactly_one_selected_row_and_say_so() {
    let mut h = Harness::scene("snapshots");
    for command in commands::ROW_MENU {
        assert_eq!(
            h.eligibility(command.0),
            Eligibility::Disabled(commands::NONE_SELECTED.into())
        );
    }
    assert_eq!(h.eligibility(TAKE.0), Eligibility::Enabled);
    h.select_rows(
        TABLE,
        &[
            &snapshot(FIRST).to_string(),
            &snapshot(SECOND).to_string(),
        ],
    );
    assert_eq!(h.node("snapshot-selection-count").text, Some(commands::several(2)));
    assert!(h.find("snapshot-information").is_none());
    for command in commands::ROW_MENU {
        assert_eq!(
            h.eligibility(command.0),
            Eligibility::Disabled(commands::several(2))
        );
        assert!(h.try_invoke(command.0).is_err());
    }
    assert!(h.model.areas.snapshots.dialog.is_none());
    h.escape_in_table(TABLE);
    assert!(edit::selected_ids(&h.model).is_empty());
    // Not the commands of any other object: a server has no snapshot rows.
    let inventory = h.model.inventory.clone().unwrap();
    let host = *inventory.hosts.keys().next().unwrap();
    let server = Target::one(ObjectRef::host(inventory.pool_id, host));
    for spec in commands::commands() {
        assert_eq!(
            h.model.eligibility(spec.id, &server),
            Eligibility::Hidden,
            "{}",
            spec.id.0
        );
    }
}

#[test]
fn take_a_snapshot_acts_on_the_vm_of_its_target_not_only_on_the_shown_one() {
    let mut h = selected();
    let other = VmId::from(uuid::Uuid::from_u128(
        0x3000_0000_0000_0000_0000_0000_0000_0001,
    ));
    let target = Target::one(ObjectRef::vm(pool(&h), other));
    h.invoke_on(TAKE.0, &target);
    assert!(matches!(dialog(&h), Dialog::Create { id, .. } if *id == other));
    // The row actions belong to the shown VM's rows only.
    h.dismiss();
    assert!(h.model.areas.snapshots.dialog.is_none());
    assert_eq!(
        h.model.eligibility(REVERT, &target),
        Eligibility::Hidden
    );
}

#[test]
fn a_snapshot_handed_over_through_the_grid_is_the_selected_row_until_one_is_chosen() {
    let mut h = Harness::scene("snapshots");
    let handed = ObjectRef::snapshot(pool(&h), snapshot(SECOND));
    // What `search::show` does after selecting the VM and its tab.
    h.model.select_in_grid(Target::one(handed));
    assert_eq!(edit::selected_ids(&h.model), [snapshot(SECOND)]);
    assert!(h.find("snapshot-information").is_some());
    // The commands' target is the snapshot now; they still find their VM.
    assert_eq!(h.eligibility(REVERT.0), Eligibility::Enabled);
    assert_eq!(h.eligibility(TAKE.0), Eligibility::Enabled);
    h.click(REVERT.0);
    assert!(
        matches!(dialog(&h), Dialog::Restore { snapshot_id, .. } if *snapshot_id == snapshot(SECOND))
    );
    h.click("cancel-snapshot");
    h.select_rows(TABLE, &[&snapshot(FIRST).to_string()]);
    assert_eq!(edit::selected_ids(&h.model), [snapshot(FIRST)]);
    assert_eq!(h.model.target(), h.model.scope().clone());
}

/// Was `the_export_button_waits_for_the_command_of_bulk_export`: the command
/// `snapshot.export` of `bulk_export` is registered now.
#[test]
fn the_export_of_the_selected_snapshots_is_a_command_of_the_local_row() {
    // Nothing selected: the command is there and says what to select.
    let mut h = Harness::scene("snapshots");
    h.select_rows(TABLE, &[]);
    assert!(h.find("snapshot-tree-toggle").is_some());
    assert_eq!(
        h.try_click(commands::EXPORT.0),
        Err("Select 1–32 snapshots of this VM".into())
    );
    assert!(h.surfaces().is_empty());
    // One snapshot: its single export, the wizard of `transfers`.
    let mut h = selected();
    assert_eq!(
        h.model.eligibility(commands::EXPORT, &h.model.target()),
        Eligibility::Enabled
    );
    let item = h
        .local_row()
        .into_iter()
        .find(|item| item.id == commands::EXPORT)
        .expect("the export is in the local row");
    assert_eq!(item.label, "Export selection…");
    // The body of the tab has no button of its own for it.
    assert!(h.find("snapshot-batch-export-open").is_none());
    h.click(commands::EXPORT.0);
    assert_eq!(h.surface().id, "transfer-dialog");
    // Several: the batch wizard of `bulk_export`, on exactly those rows.
    let mut h = selected();
    h.select_rows(
        TABLE,
        &[&snapshot(FIRST).to_string(), &snapshot(SECOND).to_string()],
    );
    h.click(commands::EXPORT.0);
    let surface = h.surface();
    assert_eq!(surface.id, "batch-export");
    assert!(surface.experimental);
    assert!(h.has_text("2 captured snapshots"));
}

#[test]
fn the_tab_lists_the_newest_two_operations_of_the_vm_with_their_controls() {
    let mut h = selected();
    let created = SnapshotId::from(uuid::Uuid::from_u128(3));
    let other = VmId::from(uuid::Uuid::from_u128(
        0x3000_0000_0000_0000_0000_0000_0000_0001,
    ));
    let mut oldest = record(&h, OperationState::Failed);
    oldest.detail = "Oldest".into();
    let mut done = record(&h, OperationState::Succeeded);
    done.created_snapshot = Some(created);
    done.created_vm = Some(other);
    let mut running = record(&h, OperationState::Tracking);
    running.can_cancel = true;
    let mut elsewhere = record(&h, OperationState::Succeeded);
    elsewhere.vm = other;
    elsewhere.detail = "Another VM".into();
    let (done_id, running_id) = (done.id, running.id);
    h.model.operation_rows = vec![oldest, done, running, elsewhere];
    assert!(!h.has_text("Oldest"));
    assert!(!h.has_text("Another VM"));
    assert!(h.has_text(&format!("Created snapshot UUID: {created}")));
    assert!(h.has_text(&format!("Created object UUID: {other}")));
    assert!(h.find(&format!("snapshot-operation-{done_id}")).is_some());
    // A synthetic pool cannot control an operation; the command says so.
    // The newest shown workflow that can be stopped is the running one.
    let _ = running_id;
    assert_eq!(
        h.node(commands::STOP_WORKFLOW.0).label.as_deref(),
        Some("Stop remaining steps")
    );
    assert_eq!(
        h.eligibility(commands::STOP_WORKFLOW.0),
        Eligibility::disabled(edit::DEMO)
    );
    // "Show created VM" is a plain selection of the created object.
    h.click(&format!("snapshot-show-created-{done_id}"));
    assert_eq!(
        h.model.scope(),
        &Target::one(ObjectRef::vm(pool(&h), other))
    );
    assert!(h.model.areas.snapshots.selected.is_empty());
}

// -- 3. The hazards -------------------------------------------------------------------

/// Hazards 2 and 42: a pool that is not synthetic and has no current
/// connection opens nothing, and says why on every command and above the
/// list.
#[test]
fn without_a_current_connection_every_command_is_disabled_with_the_page_caption() {
    let mut h = selected();
    h.model.demo = false;
    for spec in commands::commands() {
        assert_eq!(
            h.eligibility(spec.id.0),
            Eligibility::Disabled(edit::NOT_CURRENT.into()),
            "{}",
            spec.id.0
        );
        assert!(h.try_invoke(spec.id.0).is_err());
    }
    assert!(h.model.areas.snapshots.dialog.is_none());
    assert_eq!(
        h.node("snapshots-status").text.as_deref(),
        Some(edit::NOT_CURRENT)
    );
    // Forced past the command, the capture itself refuses.
    h.model.error = None;
    edit::begin_restore(&mut h.model);
    edit::begin_delete(&mut h.model);
    let (pool, vm) = shown_vm(&h.model).unwrap();
    edit::begin_create(&mut h.model, pool, vm);
    assert!(h.model.areas.snapshots.dialog.is_none());
    assert_eq!(h.model.error, Some(Error::Stale.to_string()));
}

/// Hazard 3 and the rule that a stale review cannot be confirmed: a dialog
/// captured in a synthetic pool holds no observation, so once the pool is a
/// live one nothing it shows can be confirmed.
#[test]
fn a_dialog_without_a_live_capture_is_stale_and_cannot_be_confirmed() {
    let mut h = Harness::scene("snapshot-restore");
    h.click("restore-shutdown");
    assert_eq!(h.try_click(CONFIRM), Err(edit::DEMO.into()));
    h.model.demo = false;
    assert_eq!(
        h.node("snapshot-review-stale").text,
        Some(Error::Stale.to_string())
    );
    assert_eq!(h.try_click(CONFIRM), Err(Error::Stale.to_string()));
    assert_inert(&mut h);
    let mut h = Harness::scene("snapshot-edit");
    // Nothing is changed yet, which the core's validation refuses first.
    assert_eq!(
        h.try_click(CONFIRM),
        Err(Error::InvalidSnapshotMetadata.to_string())
    );
    h.type_text("snapshot-name", "Before the upgrade");
    assert_eq!(h.try_click(CONFIRM), Err(edit::DEMO.into()));
    h.model.demo = false;
    assert!(h.find("snapshot-stale").is_some());
    assert_eq!(h.try_click(CONFIRM), Err(Error::Stale.to_string()));
    assert_inert(&mut h);
}

/// Hazards 37 to 39: what opening Create resets, and what the shared
/// drafts keep.
#[test]
fn create_resets_the_drafts_and_preselects_memory_only_when_disks_cannot_be_snapshotted() {
    let mut h = Harness::scene("snapshot-edit");
    assert_eq!(h.model.areas.snapshots.name, "Before upgrade");
    assert_eq!(h.model.areas.snapshots.description, "Synthetic disk snapshot");
    h.click("cancel-snapshot");
    h.model.areas.snapshots.capture.opt_in = true;
    h.model.areas.snapshots.capture.message = Some("left over".into());
    h.click(TAKE.0);
    let a = &h.model.areas.snapshots;
    assert!(a.name.is_empty() && a.description.is_empty() && !a.memory);
    assert!(!a.capture.opt_in && a.capture.message.is_none());
    // An empty name disables the button with the core's words, without a line.
    assert_eq!(
        h.try_click(CONFIRM),
        Err(Error::InvalidSnapshotMetadata.to_string())
    );
    assert!(!h.has_text(&Error::InvalidSnapshotMetadata.to_string()));
    h.click("cancel-snapshot");
    amend(&mut h.model, |inventory| {
        inventory.vms.get_mut(&vm()).unwrap().can_snapshot = false;
        inventory
            .details
            .vms
            .entry(vm())
            .or_default()
            .allowed_operations = Some(vec!["checkpoint".into()]);
    });
    h.click(TAKE.0);
    assert!(h.model.areas.snapshots.memory);
    assert_eq!(h.node("snapshot-memory").checked, Some(true));
}

/// Hazard 39: Clone validates the name as typed; an invalid one is said.
#[test]
fn a_clone_name_is_validated_untrimmed_and_the_refusal_is_shown() {
    let mut h = selected();
    h.click(NEW_VM.0);
    assert_eq!(h.try_click(CONFIRM), Err(edit::CLONE_NAME.into()));
    assert!(!h.has_text(edit::CLONE_NAME));
    h.type_text("snapshot-clone-name", "bad\u{7}name");
    assert!(h.has_text(edit::CLONE_NAME));
    assert_eq!(h.try_click(CONFIRM), Err(edit::CLONE_NAME.into()));
    h.type_text("snapshot-clone-name", "Recovery copy");
    assert_eq!(h.try_click(CONFIRM), Err(edit::DEMO.into()));
}

/// Hazard 43, and closing on a scope change: Cancel, Escape and another
/// object in the tree all close the dialog; the scope change also drops
/// the selection and the saved preview.
#[test]
fn cancel_escape_and_a_scope_change_close_the_dialog() {
    let mut h = Harness::scene("snapshot-create");
    h.dismiss();
    assert!(h.model.areas.snapshots.dialog.is_none());
    let mut h = Harness::scene("snapshot-information");
    h.click(PROPERTIES.0);
    assert!(h.model.areas.snapshots.dialog.is_some());
    assert!(h.model.areas.snapshots.inspection.is_some());
    let other = VmId::from(uuid::Uuid::from_u128(
        0x3000_0000_0000_0000_0000_0000_0000_0001,
    ));
    let target = Target::one(ObjectRef::vm(pool(&h), other));
    h.model.select(target);
    let a = &h.model.areas.snapshots;
    assert!(a.dialog.is_none() && a.selected.is_empty() && a.inspection.is_none());
    assert!(!a.capture.busy());
}

/// Hazard 51: the saved preview cannot be asked for in a synthetic pool.
#[test]
fn load_saved_preview_is_disabled_in_a_synthetic_pool() {
    let mut h = selected();
    assert_eq!(
        h.try_click(commands::LOAD_PREVIEW.0),
        Err(inspection::SYNTHETIC.into())
    );
    assert_eq!(
        h.node("snapshot-preview-message").text.as_deref(),
        Some(inspection::INITIAL)
    );
    assert!(h.model.areas.snapshots.inspection.is_none());
}

/// Hazard 47 and 48, as far as a fixture reaches: a capture cannot begin
/// without a current running VM, and its refusal is the shared error.
#[test]
fn a_capture_does_not_begin_in_a_synthetic_pool() {
    let mut h = Harness::scene("snapshot-capture");
    let (pool, id) = shown_vm(&h.model).unwrap();
    // No fixture has an `ObservedVm`, so no create command can be built
    // here: `begin` is reached only through `submit`, which a synthetic
    // pool leaves before it.
    edit::submit(&mut h.model);
    assert!(!h.model.areas.snapshots.capture.pending());
    assert!(!capture::available(&h.model, pool, id), "the demo guest is halted");
    assert!(h.model.take_tick_request().is_none());
    capture::cancel_dialog(&mut h.model);
    assert!(h.model.areas.snapshots.dialog.is_none());
    assert!(h.model.areas.snapshots.capture.message.is_none());
}

/// `snapshot-information` is the key of the SECTION (a group), as the Rune
/// key was the key of the container of the information and its preview:
/// everything about the selected snapshot, the saved preview and its Load
/// button included, is inside the node with that key, and nothing of the
/// tab's list is.
#[test]
fn the_snapshot_information_key_is_the_section_that_holds_details_and_preview() {
    let h = Harness::scene("snapshot-information");
    let section = h.node("snapshot-information");
    assert_eq!(section.role, "group");
    assert_eq!(section.label.as_deref(), Some("Snapshot information"));
    let inside = format!("{}/", section.path);
    for key in [
        "snapshot-information-source",
        "snapshot-preview-message",
        "snapshot-preview-image",
    ] {
        let node = h.node(key);
        assert!(
            node.path.starts_with(&inside),
            "{key} at {} is outside {}",
            node.path,
            section.path
        );
    }
    let table = h.node(TABLE);
    assert!(!table.path.starts_with(&inside), "{}", table.path);
    // The description says so too: one Group block with that key.
    let TabBody::Blocks(blocks) = h.tab() else {
        panic!("the Snapshots tab is made of blocks")
    };
    fn groups<'a>(blocks: &'a [Block], found: &mut Vec<&'a crate::surface::Group>) {
        for block in blocks {
            match block {
                Block::Group(group) => {
                    found.push(group);
                    groups(&group.blocks, found);
                }
                Block::Columns(columns) => columns.iter().for_each(|c| groups(c, found)),
                _ => {}
            }
        }
    }
    let mut found = Vec::new();
    groups(&blocks, &mut found);
    let keyed: Vec<_> = found
        .iter()
        .filter(|group| group.key.as_deref() == Some("snapshot-information"))
        .collect();
    assert_eq!(keyed.len(), 1);
    assert!(!keyed[0].collapsible);
}
