//! The tests of the area.
//!
//! Every `#[test]` of the Rune modules is accounted for:
//!
//! `vm_lifecycle_tests.rs` (9)
//! - `creation_wizard_reviews_typed_resources_and_keeps_its_source_at_both_widths`:
//!   ported, [`creation_wizard_reviews_typed_resources_and_keeps_its_source`]
//!   (the two widths are the renderer's).
//! - `cloning_distinguishes_fast_and_full_copy_and_exact_destination`: ported.
//! - `conversion_and_template_metadata_have_reviewed_captured_sources`:
//!   the conversion half ported, [`conversion_opens_on_the_review_of_its_captured_source`];
//!   the metadata half retired with the Rune kind `Metadata` (the General
//!   page of a template's Properties dialog, area `vm_properties`).
//! - `deletion_defaults_record_only_and_captures_exact_selected_data_and_snapshots`: ported.
//! - `invalid_resources_and_stale_or_changed_sources_cannot_submit`: ported.
//! - `global_history_survives_deleted_source_and_is_reachable_offline_at_compact_width`:
//!   retired with the "VM changes" panel; the Events tab (area `events`)
//!   lists these records, also offline. The 480 px position was layout.
//! - `shared_history_retains_unresolved_records_and_bounds_terminal_records`:
//!   moved by the kernel with `merge_operation_rows` (`model_tests.rs`,
//!   `vm_operation_rows_merge_by_pool_and_id_and_keep_the_unresolved`).
//! - `installation_iso_is_selected_by_uuid_and_built_in_templates_cannot_be_edited_or_deleted`:
//!   ported, [`installation_iso_is_selected_by_uuid_and_built_in_templates_cannot_be_deleted`]
//!   (editing: see above).
//! - `copied_bios_choice_captures_host_and_affinity_in_the_creation_review`: ported.
//!
//! `vm_creation_cloud.rs` (4): all ported under their names
//! (`optional_cloud_step_is_reachable_and_disabling_it_discards_the_draft`
//! asserts the inputs of the Cloud-init step where Rune asserted a fourth
//! step).
//!
//! `cloud_init_view_tests.rs` (5): all ported under their names (without
//! "at_compact_and_wide_sizes").
//!
//! From other Rune files: the share of `main.rs`
//! `dialogs_outside_the_supported_core_carry_the_experimental_notice` and of
//! `session_permissions_view.rs`
//! `review_permission_notice_is_reachable_without_bypassing_synthetic_target_guard`.

use std::time::Duration;

use xenadmin_core::{
    GuestFirmware, Inventory, OperationAction, OperationId, OperationState, PoolId, PowerState,
    SnapshotAction, SnapshotDeletionSelection, SnapshotId, VbdId, VdiId, VmCopyMode, VmId,
    VmLifecycleOperation,
};

use super::commands::{
    CLOUD_INIT, CONVERT, COPY, CREATE_FROM_SELECTION, DELETE, NEW, TEMPLATES_COPY,
    TEMPLATES_DELETE, TEMPLATES_NEW,
};
use super::{cloud, creation, deletion, drive, edit, scenes, view};
use crate::testing::Live;
use crate::{Harness, prelude::*};

fn source_id() -> VmId {
    scenes::demo_source()
}

fn snapshot_id() -> SnapshotId {
    SnapshotId::parse("60000000-0000-0000-0000-000000000000").unwrap()
}

fn editor(h: &Harness) -> &edit::Editor {
    edit::editor(&h.model).expect("lifecycle editor")
}

fn editor_mut(h: &mut Harness) -> &mut edit::Editor {
    edit::editor_mut(&mut h.model).expect("lifecycle editor")
}

fn reviewed(h: &Harness) -> &VmLifecycleOperation {
    &editor(h).review.as_ref().expect("review").operation
}

fn pending(h: &Harness) -> &cloud::Pending {
    cloud::pending(&h.model).expect("pending creation")
}

fn panel(h: &Harness) -> &drive::Panel {
    h.model.areas.vm_lifecycle.drive.as_ref().expect("drive")
}

fn panel_mut(h: &mut Harness) -> &mut drive::Panel {
    h.model.areas.vm_lifecycle.drive.as_mut().expect("drive")
}

fn amend(h: &mut Harness, change: impl FnOnce(&mut Inventory)) {
    scenes::amend(&mut h.model, change);
}

fn inventory(h: &Harness) -> &Inventory {
    h.model.inventory.as_deref().expect("inventory")
}

/// The fixture of the Rune tests (`state(template)`): the demo pool, its
/// first guest a custom template or a VM that advertises every lifecycle
/// operation, its snapshot deletable and owning a data disk. The source
/// is selected; nothing is open.
fn fixture(template: bool) -> (Harness, ObjectRef) {
    let mut m = Model::fixture();
    m.load_demo(3, 48);
    scenes::amend(&mut m, |inventory| {
        inventory.vms.get_mut(&source_id()).unwrap().is_template = template;
        let details = inventory.details.vms.get_mut(&source_id()).unwrap();
        details.is_default_template = Some(false);
        details.allowed_operations = Some(vec![
            "clone".into(),
            "copy".into(),
            "destroy".into(),
            "make_into_template".into(),
        ]);
        let mut snapshot = inventory.snapshots.get(&snapshot_id()).unwrap().clone();
        snapshot.allowed_actions.push(SnapshotAction::Delete);
        inventory.snapshots.insert(snapshot);
        let mut disk = inventory
            .details
            .vdis
            .values()
            .find(|disk| disk.kind == "user")
            .unwrap()
            .clone();
        disk.id = VdiId::parse("71000000-0000-0000-0000-000000000099").unwrap();
        disk.name = "Snapshot data".into();
        disk.is_snapshot = Some(true);
        let mut vbd = inventory
            .details
            .vbds
            .values()
            .find(|vbd| vbd.kind == "Disk")
            .unwrap()
            .clone();
        vbd.id = VbdId::parse("72000000-0000-0000-0000-000000000099").unwrap();
        vbd.vm = None;
        vbd.snapshot = Some(snapshot_id());
        vbd.vdi = Some(disk.id);
        inventory
            .details
            .vdi_attachments
            .insert(disk.id, vec![vbd.id]);
        inventory.details.vbds.insert(vbd.id, vbd);
        inventory.details.vdis.insert(disk.id, disk);
    });
    let pool = m.inventory.as_ref().unwrap().pool_id;
    let object = if template {
        ObjectRef::template(pool, source_id())
    } else {
        ObjectRef::vm(pool, source_id())
    };
    m.select(Target::one(object.clone()));
    (Harness::with(m), object)
}

/// Polls until the preparation thread of the drive dialog delivered.
fn prepared(h: &mut Harness) {
    for _ in 0..2000 {
        h.tick(0.0);
        if !panel(h).pending {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(panel(h).media.is_some(), "{:?}", panel(h).error);
}

// -- vm_lifecycle_tests.rs ----------------------------------------------------------

#[test]
fn creation_wizard_reviews_typed_resources_and_keeps_its_source() {
    let (mut h, _) = fixture(true);
    let sr = *inventory(&h).details.storage.keys().next().unwrap();
    let network = *inventory(&h).details.networks.keys().next().unwrap();
    h.invoke(CREATE_FROM_SELECTION.0);
    let surface = h.surface();
    assert_eq!(
        (surface.id.as_str(), surface.title.as_str()),
        (view::DIALOG, "Create VM from template")
    );
    assert!(matches!(
        &surface.kind,
        SurfaceKind::Wizard { steps, current: 0 } if steps.len() == 7
    ));
    assert_eq!(editor(&h).name, format!("{} VM", editor(&h).source.name));
    h.type_text("vm-lifecycle-name", "New application VM");
    assert!(h.find("back-vm-lifecycle").is_none());
    h.click("review-vm-lifecycle");
    assert_eq!(editor(&h).step, creation::Step::Media);
    h.click("review-vm-lifecycle");
    assert_eq!(editor(&h).step, creation::Step::Resources);
    h.click("vm-lifecycle-firmware-uefi");
    h.click("review-vm-lifecycle");
    assert_eq!(editor(&h).step, creation::Step::Storage);
    h.click(&format!("vm-lifecycle-sr-{sr}"));
    h.type_text("vm-lifecycle-disk-size", "8 GiB");
    h.click("review-vm-lifecycle");
    assert_eq!(editor(&h).step, creation::Step::Networking);
    h.click(&format!("vm-lifecycle-network-{network}"));
    h.click("review-vm-lifecycle");
    assert_eq!(editor(&h).step, creation::Step::Cloud);
    assert_eq!(h.node("review-vm-lifecycle").label.as_deref(), Some("Review change"));
    h.click("review-vm-lifecycle");
    assert!(
        matches!(reviewed(&h), VmLifecycleOperation::Create {
            name, sr: selected_sr, network: selected_network, vcpus: 4, memory,
            disk_size: Some(size), firmware: GuestFirmware::Uefi, iso: None, ..
        } if name == "New application VM" && *selected_sr == sr
            && *selected_network == network && *memory == 2 << 30 && *size == 8 << 30),
        "{:?}",
        reviewed(&h)
    );
    let surface = h.surface();
    assert_eq!(surface.title, "Review VM change");
    assert!(matches!(&surface.kind, SurfaceKind::Wizard { current: 6, .. }));
    // Another row selected in a grid does not retarget the editor.
    let pool = editor(&h).pool;
    let other = VmId::parse("30000000-0000-0000-0000-000000000001").unwrap();
    h.model
        .select_in_grid(Target::one(ObjectRef::vm(pool, other)));
    assert_eq!(editor(&h).source.id, source_id());
    assert!(h.has_text("Retain inherited installation media"));
    // The synthetic pool submits nothing, by the button and by the handler.
    assert_eq!(
        h.try_click("apply-vm-lifecycle"),
        Err(edit::DEMO.to_owned())
    );
    edit::apply(&mut h.model);
    assert!(h.model.operations.is_empty());
    assert!(editor(&h).review.is_some());
    // Back keeps the fields.
    h.click("back-vm-lifecycle");
    assert_eq!(editor(&h).step, creation::Step::Cloud);
    assert_eq!(editor(&h).disk_size, "8 GiB");
    h.click("back-vm-lifecycle");
    assert_eq!(editor(&h).step, creation::Step::Networking);
}

#[test]
fn cloning_distinguishes_fast_and_full_copy_and_exact_destination() {
    for source_template in [false, true] {
        let (mut h, _) = fixture(source_template);
        let sr = *inventory(&h).details.storage.keys().next().unwrap();
        h.invoke(if source_template {
            TEMPLATES_COPY.0
        } else {
            COPY.0
        });
        assert_eq!(h.surface().title, "Clone or full copy");
        assert!(matches!(h.surface().kind, SurfaceKind::Dialog));
        assert!(h.find("vm-lifecycle-sr-choices").is_none());
        h.click("review-vm-lifecycle");
        assert!(matches!(
            reviewed(&h),
            VmLifecycleOperation::Clone { expected_template, mode: VmCopyMode::Fast, as_template, .. }
                if *expected_template == source_template && *as_template == source_template
        ));
        h.click("back-vm-lifecycle");
        h.toggle("vm-lifecycle-full-copy");
        h.toggle("vm-lifecycle-as-template");
        // A full copy without a repository is refused by the review.
        h.click("review-vm-lifecycle");
        assert_eq!(editor(&h).error.as_deref(), Some(edit::NO_SR));
        assert!(editor(&h).review.is_none());
        h.click(&format!("vm-lifecycle-sr-{sr}"));
        assert_eq!(editor(&h).error, None);
        h.click("review-vm-lifecycle");
        assert!(matches!(
            reviewed(&h),
            VmLifecycleOperation::Clone { mode: VmCopyMode::Full { sr: target }, as_template, .. }
                if *target == sr && *as_template != source_template
        ));
    }
}

#[test]
fn conversion_opens_on_the_review_of_its_captured_source() {
    let (mut h, _) = fixture(false);
    h.invoke(CONVERT.0);
    assert_eq!(reviewed(&h), &VmLifecycleOperation::ConvertToTemplate);
    assert!(h.has_text("no longer appear as a runnable VM"));
    assert_eq!(h.surface().title, "Review VM change");
    // Nothing was entered: there is nothing to go back to.
    assert!(h.find("back-vm-lifecycle").is_none());
    assert_eq!(editor(&h).source.id, source_id());
    // A template is not converted: the command is not there for it.
    let (h, _) = fixture(true);
    assert_eq!(h.eligibility(CONVERT.0), Eligibility::Hidden);
}

#[test]
fn deletion_defaults_record_only_and_captures_exact_selected_data_and_snapshots() {
    let (mut h, _) = fixture(false);
    h.invoke(DELETE.0);
    assert_eq!(h.surface().title, "Delete VM");
    h.click("review-vm-lifecycle");
    assert!(matches!(
        reviewed(&h),
        VmLifecycleOperation::Delete { disks, snapshots, .. }
            if disks.is_empty() && snapshots.is_empty()
    ));
    assert!(h.has_text("Record-only deletion"));
    assert_eq!(h.node("apply-vm-lifecycle").label.as_deref(), Some("Delete reviewed resources"));
    h.click("back-vm-lifecycle");
    let disk = inventory(&h).vm_deletion_disks(source_id())[0];
    let snapshot_disk = inventory(&h).snapshot_deletion_disks(snapshot_id())[0];
    // The disks of a snapshot are offered once the snapshot is chosen.
    assert!(
        h.find(&format!("vm-delete-snapshot-disk-{snapshot_disk}"))
            .is_none()
    );
    h.toggle(&format!("vm-delete-disk-{disk}"));
    h.toggle(&format!("vm-delete-snapshot-{}", snapshot_id()));
    h.toggle(&format!("vm-delete-snapshot-disk-{snapshot_disk}"));
    h.click("review-vm-lifecycle");
    assert_eq!(
        reviewed(&h),
        &VmLifecycleOperation::Delete {
            expected_template: false,
            disks: vec![disk],
            snapshots: vec![SnapshotDeletionSelection {
                snapshot: snapshot_id(),
                disks: vec![snapshot_disk]
            }]
        }
    );
    assert!(h.has_text("Consumer:"));
    assert!(h.has_text("DELETE DISK DATA:"));
    amend(&mut h, |inventory| {
        inventory
            .details
            .vbds
            .values_mut()
            .find(|vbd| vbd.vdi == Some(disk))
            .unwrap()
            .owned = false;
    });
    assert!(h.has_text("no longer available"));
    assert!(h.try_click("apply-vm-lifecycle").is_err());
    edit::apply(&mut h.model);
    assert!(h.model.operations.is_empty());
}

#[test]
fn invalid_resources_and_stale_or_changed_sources_cannot_submit() {
    let (mut h, template) = fixture(true);
    edit::open(&mut h.model, &template, edit::Kind::Create);
    for invalid in ["0", "257", "-1", "1.5"] {
        editor_mut(&mut h).vcpus = invalid.into();
        assert!(edit::operation(editor(&h)).is_err());
    }
    editor_mut(&mut h).vcpus = "2".into();
    for invalid in ["0", "1 KiB", "1025", "-1", "18446744073709551615 GiB"] {
        editor_mut(&mut h).memory = invalid.into();
        assert!(edit::operation(editor(&h)).is_err());
    }
    // The step that owns the fields refuses "Next" with the same words.
    editor_mut(&mut h).step = creation::Step::Resources;
    h.click("review-vm-lifecycle");
    assert_eq!(editor(&h).step, creation::Step::Resources);
    assert!(h.find("vm-lifecycle-error").is_some());

    let (mut h, _) = fixture(false);
    h.invoke(CONVERT.0);
    amend(&mut h, |inventory| {
        inventory.pool_id = PoolId::parse("10000000-0000-0000-0000-000000000099").unwrap()
    });
    assert!(h.has_text("Cancel and reopen"));
    assert_eq!(
        h.try_click("apply-vm-lifecycle"),
        Err(edit::STALE.to_owned())
    );
    edit::apply(&mut h.model);
    assert!(h.model.operations.is_empty());
}

#[test]
fn installation_iso_is_selected_by_uuid_and_built_in_templates_cannot_be_deleted() {
    let (mut h, template) = fixture(true);
    let iso_id = VdiId::parse("71000000-0000-0000-0000-000000000077").unwrap();
    amend(&mut h, |inventory| {
        let mut iso = inventory.details.vdis.values().next().unwrap().clone();
        iso.id = iso_id;
        iso.name = "Installer ISO".into();
        iso.kind = "iso".into();
        iso.read_only = Some(true);
        iso.is_snapshot = Some(false);
        iso.missing = Some(false);
        inventory.details.vdis.insert(iso.id, iso);
        inventory
            .details
            .vms
            .get_mut(&source_id())
            .unwrap()
            .is_default_template = Some(true);
    });
    let sr = *inventory(&h).details.storage.keys().next().unwrap();
    let network = *inventory(&h).details.networks.keys().next().unwrap();
    assert_eq!(
        h.eligibility(TEMPLATES_DELETE.0),
        Eligibility::disabled(
            "Built-in templates cannot be edited or deleted here; copy them first"
        )
    );
    assert!(h.try_invoke(TEMPLATES_DELETE.0).is_err());
    edit::open(&mut h.model, &template, edit::Kind::Delete);
    assert!(h.model.areas.vm_lifecycle.panel.is_none());
    h.invoke(CREATE_FROM_SELECTION.0);
    h.click("review-vm-lifecycle");
    assert_eq!(editor(&h).step, creation::Step::Media);
    assert_eq!(h.node("vm-lifecycle-iso-choices").value.as_deref(), Some("Keep inherited installation media"));
    h.click(&format!("vm-lifecycle-iso-{iso_id}"));
    assert_eq!(editor(&h).iso, Some(iso_id));
    h.click("review-vm-lifecycle");
    h.click("review-vm-lifecycle");
    h.click(&format!("vm-lifecycle-sr-{sr}"));
    h.click("review-vm-lifecycle");
    h.click(&format!("vm-lifecycle-network-{network}"));
    h.click("review-vm-lifecycle");
    h.click("review-vm-lifecycle");
    assert!(
        matches!(reviewed(&h), VmLifecycleOperation::Create { iso: Some(id), .. } if *id == iso_id),
        "{:?}",
        editor(&h).error
    );
    assert!(h.has_text("Replace the first CD drive"));
    // The inherited media again.
    h.click("back-vm-lifecycle");
    editor_mut(&mut h).step = creation::Step::Media;
    h.click("vm-lifecycle-no-iso");
    assert_eq!(editor(&h).iso, None);
}

#[test]
fn copied_bios_choice_captures_host_and_affinity_in_the_creation_review() {
    let mut h = Harness::scene("firmware-create");
    let host = editor(&h).initial.hosts.keys().next().copied().unwrap();
    assert_eq!(h.node("vm-lifecycle-bios-host-choices").value.as_deref(), Some("Use generic BIOS identity"));
    h.click(&format!("vm-lifecycle-bios-host-{host}"));
    assert_eq!(editor(&h).bios_host, Some(host));
    let lines = creation::review(editor(&h));
    assert!(
        lines
            .iter()
            .any(|s| s.contains("home-host preference") && s.contains(&host.to_string()))
    );
    assert!(matches!(
        creation::operation(editor(&h)).unwrap(),
        VmLifecycleOperation::Create { bios_host: Some(h), .. } if h == host
    ));
    // The installed BIOS disk blocks the flip: the option is disabled, and
    // the handler changes nothing either.
    assert!(h.try_click("vm-lifecycle-firmware-uefi").is_err());
    creation::choose_firmware(&mut h.model, GuestFirmware::Uefi);
    assert_eq!(editor(&h).firmware, GuestFirmware::Inherited);
    assert_eq!(editor(&h).bios_host, Some(host));
    // A firmware that can be chosen takes the copied identity with it.
    h.click("vm-lifecycle-firmware-bios");
    assert_eq!(editor(&h).firmware, GuestFirmware::Bios);
    assert_eq!(editor(&h).bios_host, None);
    h.click(&format!("vm-lifecycle-bios-host-{host}"));
    h.click("vm-lifecycle-bios-generic");
    assert_eq!(editor(&h).bios_host, None);
}

// -- vm_creation_cloud.rs -----------------------------------------------------------

#[test]
fn optional_cloud_step_is_reachable_and_disabling_it_discards_the_draft() {
    let mut h = Harness::scene("vm-create");
    assert_eq!(editor(&h).step, creation::Step::Storage);
    h.click("review-vm-lifecycle");
    h.click("review-vm-lifecycle");
    assert_eq!(editor(&h).step, creation::Step::Cloud);
    assert!(h.find("creation-cloud-user-data").is_none());
    h.toggle("creation-cloud-enable");
    assert!(editor(&h).cloud);
    assert_eq!(h.node("creation-cloud-user-data").value.as_deref(), Some("#cloud-config\n"));
    h.type_text(
        "creation-cloud-user-data",
        "#cloud-config\nprivate-fixture-token: true\n",
    );
    assert!(
        editor(&h)
            .cloud_input
            .user_data
            .contains("private-fixture-token")
    );
    h.click("back-vm-lifecycle");
    h.click("review-vm-lifecycle");
    h.toggle("creation-cloud-enable");
    let form = editor(&h);
    assert!(!form.cloud);
    assert!(!form.cloud_input.user_data.contains("private-fixture-token"));
    assert!(h.find("creation-cloud-user-data").is_none());
    h.click("review-vm-lifecycle");
    assert!(editor(&h).review.is_some());
}

#[test]
fn creation_cloud_inputs_are_validated_and_review_omits_the_private_payload() {
    let mut h = Harness::scene("creation-cloud");
    h.type_text("creation-cloud-hostname", "bad/hostname");
    h.click("review-vm-lifecycle");
    assert!(editor(&h).review.is_none());
    assert!(editor(&h).error.is_some());
    h.type_text("creation-cloud-hostname", "fixture");
    h.type_text(
        "creation-cloud-user-data",
        "#cloud-config\nprivate-fixture-token: true\n",
    );
    h.click("review-vm-lifecycle");
    let f = editor(&h);
    assert!(f.review.is_some(), "{:?}", f.error);
    let shown = h.dump();
    assert!(!shown.contains("private-fixture-token"));
    assert!(shown.contains("separately reviewed"));
    let payload = serde_json::to_string(&f.review.as_ref().unwrap().operation).unwrap();
    assert!(!payload.contains("private-fixture-token"));
    assert!(h.try_click("apply-vm-lifecycle").is_err());
    edit::apply(&mut h.model);
    assert!(h.model.operations.is_empty());
}

#[test]
fn creation_cloud_pending_requires_exact_success_and_never_uses_the_selection() {
    let mut h = Harness::scene("creation-cloud-pending");
    let p = pending(&h);
    let r = cloud::record(&h.model, p).unwrap();
    let vm = cloud::completed_vm(p, &r).unwrap();
    assert_eq!(vm, scenes::demo_created());
    for state in [
        OperationState::OutcomeUnknown,
        OperationState::AcknowledgedUnknown,
        OperationState::PartiallyCompleted,
        OperationState::Failed,
        OperationState::Cancelled,
    ] {
        let mut bad = r.clone();
        bad.state = state;
        assert!(cloud::completed_vm(p, &bad).is_err());
    }
    for mutation in [0, 1, 2, 3, 4, 5] {
        let mut bad = r.clone();
        match mutation {
            0 => bad.id = OperationId::new(),
            1 => bad.pool = uuid::Uuid::from_u128(3).into(),
            2 => bad.vm = vm,
            3 => bad.action = OperationAction::Lifecycle(VmLifecycleOperation::ConvertToTemplate),
            4 => bad.created_vm = None,
            _ => bad.created_vm = Some(p.source),
        };
        assert!(cloud::completed_vm(p, &bad).is_err());
    }
    assert_eq!(cloud::ready(&h.model, pending(&h)).unwrap(), vm);
    let (pool, source) = (pending(&h).pool, pending(&h).source);
    h.model
        .select_in_grid(Target::one(ObjectRef::template(pool, source)));
    assert_eq!(cloud::ready(&h.model, pending(&h)).unwrap(), vm);
    amend(&mut h, |i| {
        i.vms.get_mut(&vm).unwrap().power_state = PowerState::Running
    });
    assert!(cloud::ready(&h.model, pending(&h)).is_err());
    amend(&mut h, |i| {
        i.vms.get_mut(&vm).unwrap().power_state = PowerState::Halted
    });
    assert_eq!(cloud::ready(&h.model, pending(&h)).unwrap(), vm);
    h.model.demo = false;
    assert!(cloud::ready(&h.model, pending(&h)).is_err());
}

#[test]
fn creation_cloud_pending_controls_are_visible_and_synthetic_continue_never_prepares_media() {
    let mut h = Harness::scene("creation-cloud-pending");
    let surface = h.surface();
    assert_eq!(
        (surface.id.as_str(), surface.title.as_str()),
        (view::DIALOG, "Create VM · configuration drive")
    );
    assert_eq!(
        h.try_click("creation-cloud-continue"),
        Err(edit::DEMO.to_owned())
    );
    cloud::continue_to_drive(&mut h.model);
    assert!(h.model.areas.vm_lifecycle.drive.is_none());
    assert!(h.model.operations.is_empty());
    let shown = h.dump();
    assert!(shown.contains("Created VM:"), "{shown}");
    assert!(shown.contains("Closing discards"));
    assert!(h.has_text(cloud::VERIFIED));
    // The draft is not shown here.
    assert!(!shown.contains("packages: [curl]"));
    h.click("creation-cloud-close");
    assert!(h.model.areas.vm_lifecycle.panel.is_none());
    assert!(!h.model.operation_rows.is_empty());
}

// -- cloud_init_view_tests.rs -------------------------------------------------------

#[test]
fn cloud_init_review_keeps_target_and_resources() {
    let mut h = Harness::scene("cloud-init");
    assert_eq!(h.surface().id, view::DRIVE);
    let (target, pool, sr) = (panel(&h).vm, panel(&h).pool, panel(&h).sr);
    assert!(sr.is_some());
    h.type_text("cloud-init-user-data", "#cloud-config\nwrite_files: []\n");
    h.click("cloud-init-prepare");
    prepared(&mut h);
    assert!(panel(&h).user_data.is_empty());
    let config_id = panel(&h).media.as_ref().unwrap().configuration_id();
    h.model
        .select_in_grid(Target::one(ObjectRef::vm(pool, source_id())));
    assert_eq!(panel(&h).vm, target);
    assert_eq!(panel(&h).sr, sr);
    assert_eq!(
        panel(&h).media.as_ref().unwrap().configuration_id(),
        config_id
    );
    assert!(h.has_text(&format!("Configuration ID: {config_id}")));
    assert!(h.has_text("Media: 8 MiB FAT16 · CIDATA"));
    assert_eq!(h.try_click("cloud-init-apply"), Err(edit::DEMO.to_owned()));
    drive::apply(&mut h.model);
    assert!(h.model.resource_operations.is_empty());
    assert!(panel(&h).media.is_some());
    // "Discard and edit" returns to the inputs with fresh user-data.
    h.click("cloud-init-edit");
    assert!(panel(&h).media.is_none());
    assert_eq!(h.node("cloud-init-user-data").value.as_deref(), Some("#cloud-config\n"));
    h.click("cloud-init-close");
    assert!(h.model.areas.vm_lifecycle.drive.is_none());
}

#[test]
fn cloud_init_bad_inputs_preserve_editor_and_pool_changes_disable_review() {
    let mut h = Harness::scene("cloud-init");
    h.type_text("cloud-init-hostname", "bad/host");
    h.type_text(
        "cloud-init-user-data",
        "#cloud-config\nfixture: preserve-me\n",
    );
    h.click("cloud-init-prepare");
    assert!(panel(&h).error.is_some());
    assert!(h.find("cloud-init-error").is_some());
    assert!(panel(&h).user_data.contains("preserve-me"));
    assert!(!panel(&h).pending);
    h.type_text("cloud-init-hostname", "valid");
    h.click("cloud-init-prepare");
    prepared(&mut h);
    amend(&mut h, |i| {
        i.pool_id = PoolId::from(uuid::Uuid::from_u128(999))
    });
    assert!(!drive::current(&h.model, panel(&h)));
    assert_eq!(
        h.try_click("cloud-init-apply"),
        Err(drive::STALE.to_owned())
    );
    drive::apply(&mut h.model);
    assert!(h.model.resource_operations.is_empty());
    h.click("cloud-init-close");
}

#[test]
fn cloud_init_cancelling_preparation_drops_late_media_without_reopening() {
    let mut h = Harness::scene("cloud-init");
    h.click("cloud-init-prepare");
    assert!(panel(&h).pending);
    assert!(h.find("cloud-init-preparing").is_some());
    h.click("cloud-init-close");
    for _ in 0..20 {
        h.tick(0.0);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(h.model.areas.vm_lifecycle.drive.is_none());
    assert!(h.surfaces().is_empty());
    assert!(h.model.resource_operations.is_empty());
}

#[test]
fn cloud_init_slot_validation_refuses_occupied_and_noncanonical_slots() {
    let mut h = Harness::scene("cloud-init");
    assert!(drive::can_prepare(&h.model, panel(&h)));
    assert_eq!(h.node("cloud-init-prepare").disabled, None);
    for slot in ["-1", "256", "01"] {
        h.type_text("cloud-init-device", slot);
        assert!(!drive::can_prepare(&h.model, panel(&h)), "{slot}");
        assert!(h.node("cloud-init-prepare").disabled.is_some());
    }
    let vm = panel(&h).vm;
    let slot = inventory(&h)
        .details
        .vbds
        .values()
        .find(|v| v.vm == Some(vm))
        .unwrap()
        .userdevice
        .clone();
    panel_mut(&mut h).device = slot;
    assert!(!drive::can_prepare(&h.model, panel(&h)));
    // Without a repository the button says what is missing.
    panel_mut(&mut h).sr = None;
    assert_eq!(
        h.try_click("cloud-init-prepare"),
        Err(drive::NO_STORAGE.to_owned())
    );
}

#[test]
fn creation_handoff_prepares_private_media_for_the_exact_created_target_and_chosen_sr() {
    let mut h = Harness::scene("cloud-init");
    let (vm, pool, sr) = (panel(&h).vm, panel(&h).pool, panel(&h).sr.unwrap());
    drive::close(&mut h.model);
    drive::open_created(
        &mut h.model,
        pool,
        vm,
        sr,
        "created-fixture".into(),
        "#cloud-config\nprivate-fixture-token: true\n".into(),
        String::new(),
    );
    prepared(&mut h);
    assert_eq!(panel(&h).vm, vm);
    assert_eq!(panel(&h).sr, Some(sr));
    assert!(panel(&h).user_data.is_empty());
    assert!(!h.dump().contains("private-fixture-token"));
    assert!(h.has_text("Hostname: created-fixture"));
    assert!(h.model.resource_operations.is_empty());
    drive::close(&mut h.model);
}

// -- main.rs, session_permissions_view.rs -------------------------------------------

/// `main.rs`: dialogs_outside_the_supported_core_carry_the_experimental_notice,
/// for every surface of this area.
#[test]
fn rune_experimental_notice_is_on_every_surface_of_the_area() {
    for scene in scenes::scenes() {
        let h = Harness::scene(scene.name);
        assert!(h.surface().experimental, "{}", scene.name);
        assert!(h.model.experimental_notice(), "{}", scene.name);
        h.assert_unique_keys();
    }
    let (mut h, _) = fixture(false);
    for command in [COPY, CONVERT, DELETE] {
        h.invoke(command.0);
        assert!(h.surface().experimental, "{command:?}");
        h.dismiss();
        assert!(h.surfaces().is_empty());
    }
}

/// This area's share of Rune `session_permissions_view.rs`
/// `review_permission_notice_is_reachable_without_bypassing_synthetic_target_guard`.
#[test]
fn review_permission_notice_is_reachable_without_bypassing_synthetic_target_guard() {
    for (scene, confirm) in [
        ("lifecycle-permissions", "apply-vm-lifecycle"),
        ("cloud-init-review", "cloud-init-apply"),
    ] {
        let mut h = Harness::scene(scene);
        assert!(
            h.find(permissions::REVIEW_KEY).is_some(),
            "{scene}: captured review must explain permission discovery"
        );
        assert_eq!(h.try_click(confirm), Err(edit::DEMO.to_owned()), "{scene}");
        assert!(h.model.resource_operations.is_empty());
        assert!(h.model.operations.is_empty());
    }
    // Before a review there is no operation to assess, so no line.
    for scene in ["vm-create", "cloud-init"] {
        assert!(Harness::scene(scene).find(permissions::REVIEW_KEY).is_none());
    }
}

// -- The commands -------------------------------------------------------------------

#[test]
fn every_command_is_in_its_menu_and_the_context_menu_of_its_kind() {
    let (h, vm) = fixture(false);
    let menu = h.context_menu(&Target::one(vm));
    for command in [COPY, CONVERT, CLOUD_INIT, DELETE] {
        assert_eq!(h.eligibility(command.0), Eligibility::Enabled, "{command:?}");
        assert!(h.menu_item(command.0).is_some(), "{command:?}");
        assert!(menu.iter().any(|item| item.id == command), "{command:?}");
    }
    // The template's commands are not there for a VM; New VM is, disabled.
    for command in [CREATE_FROM_SELECTION, TEMPLATES_COPY, TEMPLATES_DELETE] {
        assert_eq!(h.eligibility(command.0), Eligibility::Hidden, "{command:?}");
        assert!(menu.iter().all(|item| item.id != command), "{command:?}");
    }
    for command in [NEW, TEMPLATES_NEW] {
        assert_eq!(
            h.eligibility(command.0),
            Eligibility::disabled(edit::NO_TEMPLATE),
            "{command:?}"
        );
    }
    assert!(h.toolbar().iter().any(|item| item.id == NEW));

    let (h, template) = fixture(true);
    let menu = h.context_menu(&Target::one(template));
    for command in [
        NEW,
        TEMPLATES_NEW,
        CREATE_FROM_SELECTION,
        TEMPLATES_COPY,
        TEMPLATES_DELETE,
    ] {
        assert_eq!(h.eligibility(command.0), Eligibility::Enabled, "{command:?}");
        assert!(h.menu_item(command.0).is_some(), "{command:?}");
    }
    for command in [CREATE_FROM_SELECTION, TEMPLATES_COPY, TEMPLATES_DELETE] {
        assert!(menu.iter().any(|item| item.id == command), "{command:?}");
    }
    for command in [COPY, CONVERT, CLOUD_INIT, DELETE] {
        assert_eq!(h.eligibility(command.0), Eligibility::Hidden, "{command:?}");
    }
}

#[test]
fn the_three_ways_to_a_new_vm_open_the_same_wizard() {
    for command in [NEW, TEMPLATES_NEW, CREATE_FROM_SELECTION] {
        let (mut h, _) = fixture(true);
        h.invoke(command.0);
        let form = editor(&h);
        assert_eq!(
            (form.kind, form.step, form.source.id),
            (edit::Kind::Create, creation::Step::Name, source_id()),
            "{command:?}"
        );
        assert_eq!(form.name, format!("{} VM", form.source.name));
        assert!(
            h.surface()
                .subtitle
                .unwrap()
                .contains(&source_id().to_string())
        );
    }
}

#[test]
fn commands_are_for_one_halted_object_of_a_writable_pool() {
    let (mut h, vm) = fixture(false);
    let pool = vm.pool.unwrap();
    let other = ObjectRef::vm(
        pool,
        VmId::parse("30000000-0000-0000-0000-000000000001").unwrap(),
    );
    // Several: hidden (the batch deletion is another area's).
    let several = Target::many([vm.clone(), other]);
    for command in [COPY, CONVERT, CLOUD_INIT, DELETE] {
        assert_eq!(
            h.model.eligibility(command, &several),
            Eligibility::Hidden,
            "{command:?}"
        );
    }
    // Not halted: the caption Rune showed; the drive says what it is for.
    amend(&mut h, |i| {
        i.vms.get_mut(&source_id()).unwrap().power_state = PowerState::Running
    });
    for command in [COPY, CONVERT, DELETE] {
        assert_eq!(
            h.eligibility(command.0),
            Eligibility::disabled(edit::REQUIRES),
            "{command:?}"
        );
    }
    assert_eq!(
        h.eligibility(CLOUD_INIT.0),
        Eligibility::disabled(drive::INTRODUCTION)
    );
    assert_eq!(
        h.menu_item(CLOUD_INIT.0).unwrap().label,
        "Cloud-init Drive (shut down first)"
    );
    amend(&mut h, |i| {
        i.vms.get_mut(&source_id()).unwrap().power_state = PowerState::Halted
    });
    assert_eq!(h.menu_item(CLOUD_INIT.0).unwrap().label, "Cloud-init Drive");
    // Not advertised by the host: what the core says against the operation.
    amend(&mut h, |i| {
        i.details
            .vms
            .get_mut(&source_id())
            .unwrap()
            .allowed_operations = Some(vec!["clone".into()]);
    });
    assert_eq!(h.eligibility(COPY.0), Eligibility::Enabled);
    for command in [CONVERT, DELETE] {
        assert_eq!(
            h.eligibility(command.0),
            Eligibility::disabled("The host does not currently allow this action"),
            "{command:?}"
        );
        assert!(h.try_invoke(command.0).is_err());
        assert!(h.surfaces().is_empty());
    }
    // A control domain has none of them.
    amend(&mut h, |i| {
        i.vms.get_mut(&source_id()).unwrap().is_control_domain = true
    });
    for command in [COPY, CONVERT, CLOUD_INIT, DELETE] {
        assert_eq!(h.eligibility(command.0), Eligibility::Hidden, "{command:?}");
    }
}

#[test]
fn the_cloud_init_drive_is_on_the_storage_tab_of_a_vm_only() {
    let (mut h, vm) = fixture(false);
    h.open_tab(TabId::Storage);
    assert!(h.local_row().iter().any(|item| item.id == CLOUD_INIT));
    h.click(CLOUD_INIT.0);
    assert_eq!(h.surface().id, view::DRIVE);
    assert_eq!(Some(panel(&h).vm), vm.vm_id());
    assert_eq!(panel(&h).user_data, "#cloud-config\n");
    h.dismiss();
    assert!(h.surfaces().is_empty());
    let host = inventory(&h).hosts.keys().next().copied().unwrap();
    h.select(Target::one(ObjectRef::host(vm.pool.unwrap(), host)));
    h.open_tab(TabId::Storage);
    assert!(h.local_row().iter().all(|item| item.id != CLOUD_INIT));
}

#[test]
fn another_scope_closes_both_surfaces() {
    let (mut h, vm) = fixture(false);
    h.invoke(COPY.0);
    assert!(h.model.areas.vm_lifecycle.panel.is_some());
    h.dismiss();
    assert!(h.model.areas.vm_lifecycle.panel.is_none());
    h.invoke(DELETE.0);
    let other = ObjectRef::vm(
        vm.pool.unwrap(),
        VmId::parse("30000000-0000-0000-0000-000000000001").unwrap(),
    );
    h.select(Target::one(other));
    assert!(h.model.areas.vm_lifecycle.panel.is_none());
    let mut h = Harness::scene("cloud-init");
    let pool = panel(&h).pool;
    h.select(Target::one(ObjectRef::pool(pool)));
    assert!(h.model.areas.vm_lifecycle.drive.is_none());
}

/// Decision 10 of `AREAS.md`: the texts that named "VM changes" and
/// "Activity" of Rune name the Events tab.
#[test]
fn texts_name_the_places_of_this_client() {
    let h = Harness::scene("lifecycle-permissions");
    assert!(h.has_text("remains on the Events tab of the Notifications view if a later step fails"));
    let h = Harness::scene("creation-cloud-pending");
    assert!(h.has_text("The Events tab of the Notifications view keeps the creation outcome"));
    assert!(drive::POOL_BLOCK.contains("appear on the Events tab of the Notifications view"));
    assert!(deletion::describe(
        inventory(&h),
        &inventory(&h).vms[&source_id()],
        &Default::default(),
        &Default::default(),
        &Default::default()
    )
    .last()
    .unwrap()
    .contains("remain on the Events tab of the Notifications view"));
    let (mut h, _) = fixture(false);
    h.invoke(COPY.0);
    h.click("review-vm-lifecycle");
    assert!(h.has_text("its UUID on the Events tab of the Notifications view"));
}

// -- Against the fixture server -----------------------------------------------------

/// The fixture server's VM allows `start` only and the server serves no
/// lifecycle call: what a live, observed VM gives is tested for capture,
/// eligibility and refusal. No lifecycle operation and no cloud-init drive
/// is submitted in any test.
#[test]
fn a_live_session_observes_the_vm_and_the_host_decides_what_is_offered() {
    let mut live = Live::with_vm();
    let (pool, vm) = (live.pool, live.server.vm());
    live.h.select(Target::one(live.vm()));
    for command in [COPY, CONVERT, DELETE] {
        assert!(
            matches!(live.h.eligibility(command.0), Eligibility::Disabled(_)),
            "{command:?}"
        );
    }
    // The drive dialog opens for the halted VM, on the observed VM.
    assert_eq!(live.h.eligibility(CLOUD_INIT.0), Eligibility::Enabled);
    live.h.invoke(CLOUD_INIT.0);
    let observed = live
        .h
        .model
        .snapshot_for(pool)
        .and_then(|snapshot| snapshot.observe_vm(vm).ok());
    assert!(observed.is_some());
    let p = panel(&live.h);
    assert_eq!((p.pool, p.vm, p.observed), (pool, vm, observed));
    assert!(drive::current(&live.h.model, p));
    assert!(live.h.find("cloud-init-stale").is_none());
    assert!(live.h.find("cloud-init-read-only").is_none());
    // The pool has no repository: nothing to prepare for.
    assert_eq!(
        live.h.try_click("cloud-init-prepare"),
        Err(drive::NO_STORAGE.to_owned())
    );
    drive::prepare(&mut live.h.model);
    assert!(!panel(&live.h).pending);
    drive::apply(&mut live.h.model);
    assert!(live.h.model.resource_operations.is_empty());
    assert!(live.h.model.areas.vm_lifecycle.panel.is_none());
}

// -- The private drafts ----------------------------------------------------------------

/// The text control of a field of the front surface, by key.
fn text_control(h: &Harness, key: &str) -> Control {
    fn find(blocks: &[Block], key: &str) -> Option<Control> {
        blocks.iter().find_map(|block| match block {
            Block::Field(field) if field.key == key => Some(field.control.clone()),
            Block::Group(group) => find(&group.blocks, key),
            Block::Columns(columns) => columns.iter().find_map(|column| find(column, key)),
            _ => None,
        })
    }
    find(&h.surface().body, key).unwrap_or_else(|| panic!("no field {key}"))
}

/// Cloud-init user-data and network-config can hold passwords and keys.
/// Rune zeroized both drafts when they were dropped and emptied them when
/// the media preparation took them. The descriptions mark all four fields
/// as private text areas, so the front end keeps no copy and empties its
/// widget buffer when the field goes away; ordinary text areas are not.
#[test]
fn the_cloud_init_drafts_are_private_text_areas() {
    let mut h = Harness::scene("creation-cloud");
    h.type_text("creation-cloud-user-data", "#cloud-config\nprivate-fixture-token: true\n");
    for key in ["creation-cloud-user-data", "creation-cloud-network"] {
        let Control::Text { value, secret, multiline, .. } = text_control(&h, key) else {
            panic!("{key} is a text field")
        };
        assert!(multiline, "{key}");
        assert!(value.is_empty(), "{key}: the plain copy is empty");
        assert!(secret.is_some(), "{key} is not marked private");
    }
    // It is shown in clear, as on screen, and holds what was typed.
    let Control::Text { secret, .. } = text_control(&h, "creation-cloud-user-data") else {
        unreachable!()
    };
    assert_eq!(
        secret.as_ref().map(SecretText::expose),
        Some("#cloud-config\nprivate-fixture-token: true\n")
    );
    assert_eq!(
        h.node("creation-cloud-user-data").value.as_deref(),
        Some("#cloud-config\nprivate-fixture-token: true\n")
    );
    // The hostname is an ordinary field.
    assert!(matches!(
        text_control(&h, "creation-cloud-hostname"),
        Control::Text { secret: None, .. }
    ));
    // The review has no such field and no such text.
    h.type_text("creation-cloud-hostname", "fixture");
    h.click("review-vm-lifecycle");
    assert!(editor(&h).review.is_some());
    assert!(h.find("creation-cloud-user-data").is_none());
    assert!(!h.dump().contains("private-fixture-token"));

    let mut h = Harness::scene("cloud-init");
    h.type_text("cloud-init-user-data", "#cloud-config\nprivate-fixture-token: true\n");
    for key in ["cloud-init-user-data", "cloud-init-network"] {
        assert!(
            matches!(
                text_control(&h, key),
                Control::Text { secret: Some(_), multiline: true, .. }
            ),
            "{key} is not a private text area"
        );
    }
    // Preparing the media takes the drafts: nothing of them is left in the
    // model or on the surface.
    h.click("cloud-init-prepare");
    assert!(panel(&h).user_data.is_empty());
    assert!(panel(&h).network.is_empty());
    prepared(&mut h);
    assert!(h.find("cloud-init-user-data").is_none());
    assert!(!h.dump().contains("private-fixture-token"));
}
