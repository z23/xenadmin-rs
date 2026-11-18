//! Tests of the area: the tests of `vm_devices.rs`,
//! `vm_device_actions.rs`, `vm_device_lifecycle_tests.rs`,
//! `vm_device_properties_tests.rs` and `disk_workspace_tests.rs` under their
//! Rune names (the "at both widths" of a name is gone with the layout
//! half), then what the port added: commands, tabs, pages, hazards.
//!
//! A Rune test that pressed a button of the Devices page presses the
//! command of the tab's local row now; "the form keeps its VM when the
//! table selection changes" is asserted for a grid selection, since a tree
//! selection closes the form (`AREA-PORT.md`, recipe 5).

use xenadmin_core::{
    ClassAvailability, DeviceOperation, DeviceProperty as Change, Inventory, PoolId, VbdId, VdiId,
    VifId,
};

use super::disks::Mode;
use super::lifecycle::{Kind, build_operation, parse_mac, review_for};
use super::tabs::{class_message, connected, unavailable};
use super::{actions, commands, disks, lifecycle, properties, scenes};
use crate::testing::Live;
use crate::{Harness, prelude::*};

// -- helpers ------------------------------------------------------------------

fn inventory(h: &Harness) -> std::sync::Arc<Inventory> {
    h.model.inventory.clone().expect("inventory")
}

fn pool(h: &Harness) -> PoolId {
    inventory(h).pool_id
}

/// The Rune `device_state`: the Devices of the demo guest, its user disk
/// deletable, and one more disk no VM uses.
fn devices() -> Harness {
    let mut h = Harness::scene("vm-devices");
    scenes::amend(&mut h.model, |i| {
        let disk = i
            .details
            .vdis
            .values_mut()
            .find(|disk| disk.kind == "user")
            .unwrap();
        disk.allowed_operations
            .as_mut()
            .unwrap()
            .push("destroy".into());
        let mut unattached = disk.clone();
        unattached.id = spare_disk();
        unattached.name = "Existing data".into();
        i.details.vdis.insert(unattached.id, unattached);
    });
    h
}

fn spare_disk() -> VdiId {
    VdiId::parse("71000000-0000-0000-0000-000000000099").unwrap()
}

#[track_caller]
fn form(h: &Harness) -> &lifecycle::Form {
    h.model.areas.vm_devices.lifecycle.as_ref().expect("a form")
}

#[track_caller]
fn reviewed(h: &Harness) -> &DeviceOperation {
    &form(h).review.as_ref().expect("a review").operation
}

#[track_caller]
fn property_form(h: &Harness) -> &properties::Form {
    h.model
        .areas
        .vm_devices
        .properties
        .as_ref()
        .expect("a properties form")
}

fn property_form_mut(h: &mut Harness) -> &mut properties::Form {
    h.model.areas.vm_devices.properties.as_mut().unwrap()
}

#[track_caller]
fn panel(h: &Harness) -> &disks::Panel {
    h.model.areas.vm_devices.disks.as_ref().expect("a panel")
}

fn panel_mut(h: &mut Harness) -> &mut disks::Panel {
    h.model.areas.vm_devices.disks.as_mut().unwrap()
}

/// The interface of the demo guest, its row selected on the Networking tab.
fn nic(h: &mut Harness) -> xenadmin_core::VirtualInterface {
    let nic = inventory(h)
        .details
        .vifs
        .values()
        .find(|v| v.vm == Some(scenes::vm()))
        .unwrap()
        .clone();
    h.open_tab(TabId::Networking);
    h.select_row("networking-vifs-table", &nic.id.to_string());
    nic
}

// -- vm_devices.rs, vm_device_actions.rs ---------------------------------------

#[test]
fn absent_permission_and_empty_are_distinct() {
    assert!(class_message(Some(&ClassAvailability::Available), "Disks").is_none());
    assert!(
        class_message(Some(&ClassAvailability::PermissionDenied), "Disks")
            .unwrap()
            .contains("permission")
    );
    assert!(
        class_message(Some(&ClassAvailability::Unsupported), "Disks")
            .unwrap()
            .contains("not supported")
    );
    assert!(
        class_message(None, "Disks")
            .unwrap()
            .contains("not been reported")
    );
    assert_eq!(connected(None), "Not reported");
    assert_eq!(connected(Some(false)), "Disconnected");
}

#[test]
fn growth_input_preserves_exact_capacity_and_rejects_overflow() {
    use actions::parse_size;
    assert_eq!(parse_size("16 GiB").unwrap(), 16 << 30);
    assert_eq!(parse_size("123456512").unwrap(), 123456512);
    assert!(parse_size("0").is_err());
    assert!(parse_size("123456789").is_err());
    assert!(parse_size("2.5 GiB").is_err());
    assert!(parse_size("18446744073709551615 GiB").is_err());
}

// -- vm_device_lifecycle_tests.rs ----------------------------------------------

#[test]
fn disk_creation_reviews_selected_storage_size_and_captured_vm() {
    let mut h = devices();
    let sr = *inventory(&h).details.storage.keys().next().unwrap();
    h.click("vm.disk.new");
    assert_eq!(h.surface().id, "device-lifecycle");
    assert!(h.surface().experimental);
    h.type_text("lifecycle-name", "Application data");
    h.type_text("lifecycle-size", "16 GiB");
    h.pick("lifecycle-sr", &sr.to_string());
    h.click("review-device-lifecycle");
    assert_eq!(
        reviewed(&h),
        &DeviceOperation::CreateDisk {
            sr,
            name: "Application data".into(),
            size: 16 << 30,
            device: "1".into()
        }
    );
    // A selection in a grid does not retarget the open form.
    let other = *inventory(&h)
        .vms
        .keys()
        .find(|id| **id != scenes::vm())
        .unwrap();
    let pool = pool(&h);
    h.model
        .select_in_grid(Target::one(ObjectRef::vm(pool, other)));
    assert_eq!(form(&h).vm, scenes::vm());
    assert_eq!(form(&h).pool, pool);
    assert!(h.has_text("Application data"));
    assert_eq!(
        h.try_click("apply-device-lifecycle"),
        Err(lifecycle::PREVIEW.into())
    );
    lifecycle::apply(&mut h.model);
    assert!(h.model.operations.is_empty());
    assert!(form(&h).review.is_some(), "synthetic reviews never submit");
}

#[test]
fn attach_and_add_cd_review_exact_targets_without_deleting_existing_data() {
    let mut h = devices();
    let disk = spare_disk();
    h.click("vm.disk.attach");
    h.toggle("lifecycle-read-only");
    h.pick("lifecycle-disk", &disk.to_string());
    h.click("review-device-lifecycle");
    assert_eq!(
        reviewed(&h),
        &DeviceOperation::AttachDisk {
            vdi: disk,
            device: "1".into(),
            read_only: true
        }
    );
    assert!(h.has_text("Existing disk contents are retained"));
    h.click("cancel-device-lifecycle");
    h.click("vm.cd.add");
    h.click("review-device-lifecycle");
    assert_eq!(
        reviewed(&h),
        &DeviceOperation::AddCdDrive { device: "1".into() }
    );
}

#[test]
fn detach_preserves_data_delete_reviews_consumers_and_ownership_changes_disable_it() {
    let mut h = devices();
    let vbd = scenes::disk_attachment();
    let disk = inventory(&h).details.vbds[&vbd].vdi.unwrap();
    h.select_row("storage-vbds", &vbd.to_string());
    h.click("vm.disk.detach");
    assert_eq!(reviewed(&h), &DeviceOperation::DetachDisk { vbd, vdi: disk });
    assert!(!form(&h).review.as_ref().unwrap().destructive);
    assert!(h.has_text("disk and its data remain"));
    assert!(
        h.find("back-device-lifecycle").is_none(),
        "a fixed change has no draft to go back to"
    );
    h.click("cancel-device-lifecycle");
    h.click("vm.disk.delete");
    assert_eq!(reviewed(&h), &DeviceOperation::DeleteDisk { vbd, vdi: disk });
    assert!(h.has_text("All data on this disk will be lost"));
    assert!(h.has_text("Consumer:"));
    assert!(h.has_text(&form(&h).vm.to_string()));
    assert_eq!(
        h.node("apply-device-lifecycle").label.as_deref(),
        Some("Confirm change")
    );
    scenes::amend(&mut h.model, |i| {
        i.details.vbds.get_mut(&vbd).unwrap().owned = false;
    });
    assert!(h.has_text("no longer available"));
    assert!(h.try_click("apply-device-lifecycle").is_err());
    lifecycle::apply(&mut h.model);
    assert!(h.model.operations.is_empty());
    assert!(form(&h).review.is_some());
}

#[test]
fn nic_editor_reviews_generation_and_replacement_and_captures_original_identity() {
    let mut h = devices();
    let network = *inventory(&h).details.networks.keys().next().unwrap();
    let nic = nic(&mut h);
    h.click("vm.nic.add");
    h.pick("lifecycle-network", &network.to_string());
    h.click("review-device-lifecycle");
    assert_eq!(
        reviewed(&h),
        &DeviceOperation::AddNic {
            network,
            device: "1".into(),
            mac: None
        }
    );
    assert!(h.has_text("generated by the host"));
    h.click("cancel-device-lifecycle");
    h.click("vm.nic.properties");
    assert!(build_operation(form(&h)).unwrap_err().contains("different"));
    h.type_text("lifecycle-mac", "02:00:00:00:00:42");
    h.click("review-device-lifecycle");
    assert_eq!(
        reviewed(&h),
        &DeviceOperation::EditNic {
            vif: nic.id,
            expected_network: nic.network.unwrap(),
            expected_mac: nic.mac.clone(),
            network: nic.network.unwrap(),
            mac: "02:00:00:00:00:42".into()
        }
    );
    assert!(h.has_text("VM may be left without this NIC"));
    h.click("cancel-device-lifecycle");
    h.click("vm.nic.remove");
    assert_eq!(reviewed(&h), &DeviceOperation::DeleteNic { vif: nic.id });
}

#[test]
fn invalid_resource_choices_and_user_values_cannot_reach_review() {
    let mut h = devices();
    let pool = pool(&h);
    assert!(lifecycle::open(
        &mut h.model,
        pool,
        scenes::vm(),
        Kind::CreateDisk
    ));
    assert!(build_operation(form(&h)).is_err());
    let sr = *inventory(&h).details.storage.keys().next().unwrap();
    let f = h.model.areas.vm_devices.lifecycle.as_mut().unwrap();
    f.name = "Data".into();
    f.size = "16 GiB".into();
    assert!(build_operation(f).unwrap_err().contains("storage"));
    f.sr = Some(sr);
    for invalid in ["0", "513", "1.5 GiB", "18446744073709551615 GiB", "-1"] {
        f.size = invalid.into();
        assert!(build_operation(f).is_err(), "accepted size {invalid}");
    }
    f.size = "512".into();
    for invalid in ["", "256", "-1", "01", "one"] {
        f.device = invalid.into();
        assert!(build_operation(f).is_err(), "accepted slot {invalid}");
    }
    f.device = "1".into();
    for invalid in ["", "  ", "Data\n", "\tData"] {
        f.name = invalid.into();
        assert!(build_operation(f).is_err(), "accepted name {invalid:?}");
    }
    assert!(lifecycle::open(
        &mut h.model,
        pool,
        scenes::vm(),
        Kind::AddNic
    ));
    assert!(build_operation(form(&h)).unwrap_err().contains("network"));
    for invalid in [
        "00:00:00:00:00:00",
        "01:00:00:00:00:01",
        "02:00:00:00:00",
        "02:00:00:00:00:gg",
    ] {
        assert!(parse_mac(invalid).is_err(), "accepted MAC {invalid}");
    }
    assert_eq!(parse_mac("02:AB:CD:EF:01:23").unwrap(), "02:ab:cd:ef:01:23");
}

#[test]
fn device_classes_are_read_using_canonical_inventory_keys() {
    let h = devices();
    let mut i = inventory(&h).as_ref().clone();
    assert!(unavailable(&i, "VBD", "Disks").is_none());
    i.details
        .classes
        .insert("vbd".into(), ClassAvailability::PermissionDenied);
    assert!(
        unavailable(&i, "VBD", "Disks")
            .unwrap()
            .contains("permission")
    );
}

#[test]
fn reviewed_changes_cannot_follow_a_pool_switch_or_missing_live_observation() {
    let mut h = devices();
    h.click("vm.cd.add");
    h.click("review-device-lifecycle");
    let source_pool = form(&h).pool;
    let operation = reviewed(&h).clone();
    scenes::amend(&mut h.model, |i| {
        i.pool_id = PoolId::parse("10000000-0000-0000-0000-000000000099").unwrap();
    });
    assert_ne!(pool(&h), source_pool);
    lifecycle::apply(&mut h.model);
    assert!(h.model.operations.is_empty());
    assert_eq!(reviewed(&h), &operation);
    scenes::amend(&mut h.model, |i| i.pool_id = source_pool);
    h.model.demo = false;
    assert!(h.has_text("Cancel and reopen"));
    assert_eq!(
        h.try_click("apply-device-lifecycle"),
        Err(lifecycle::STALE.into())
    );
    lifecycle::apply(&mut h.model);
    assert!(h.model.operations.is_empty());
    assert_eq!(reviewed(&h), &operation);
}

#[test]
fn shared_disk_consumers_are_described_and_deletion_stays_unavailable() {
    let h = devices();
    let vm = scenes::vm();
    let mut i = inventory(&h).as_ref().clone();
    let vbd = scenes::disk_attachment();
    let mut second = i.details.vbds[&vbd].clone();
    let disk = second.vdi.unwrap();
    second.id = VbdId::parse("72000000-0000-0000-0000-000000000099").unwrap();
    second.vm = Some(*i.vms.keys().find(|id| **id != vm).unwrap());
    i.details
        .vdi_attachments
        .get_mut(&disk)
        .unwrap()
        .push(second.id);
    i.details.vbds.insert(second.id, second);
    let operation = DeviceOperation::DeleteDisk { vbd, vdi: disk };
    assert!(!operation.availability(&i, vm).is_available());
    let review = review_for(operation, &i);
    assert_eq!(
        review
            .lines
            .iter()
            .filter(|line| line.starts_with("Consumer:"))
            .count(),
        2
    );
}

// -- vm_device_properties_tests.rs ---------------------------------------------

/// The Rune `state`: the scene, the guest halted, its devices disconnected.
fn property_state() -> Harness {
    let mut h = Harness::scene("device-properties");
    let vm = scenes::vm();
    scenes::amend(&mut h.model, |i| {
        i.vms.get_mut(&vm).unwrap().power_state = xenadmin_core::PowerState::Halted;
        for v in i.details.vbds.values_mut().filter(|v| v.vm == Some(vm)) {
            v.currently_attached = Some(false);
        }
        for v in i.details.vifs.values_mut().filter(|v| v.vm == Some(vm)) {
            v.currently_attached = Some(false);
        }
    });
    h
}

#[test]
fn priority_review_keeps_device_identity_and_cannot_submit_synthetic_changes() {
    let mut h = property_state();
    assert_eq!(h.surface().id, "device-properties");
    assert!(h.surface().experimental);
    let original = property_form(&h).choice.clone().unwrap();
    if !property_form(&h).enabled {
        h.toggle("property-enabled");
    }
    h.type_text("property-value", "7");
    h.click("property-review");
    let reviewed = property_form(&h).review.clone().unwrap();
    assert!(matches!(
        reviewed,
        Change::DiskPriority {
            priority: Some(7),
            ..
        }
    ));
    if let (
        Change::DiskPriority { vbd: a, vdi: b, .. },
        Change::DiskPriority { vbd: c, vdi: d, .. },
    ) = (original, &reviewed)
    {
        assert_eq!((a, b), (*c, *d));
    } else {
        panic!("wrong action")
    }
    let vm = property_form(&h).vm;
    h.model.areas.vm_devices.disk_row = None;
    assert_eq!(property_form(&h).vm, vm);
    assert!(h.has_text("0 highest, 7 lowest"));
    assert_eq!(
        h.try_click("property-apply"),
        Err(properties::PREVIEW.into())
    );
    properties::apply(&mut h.model);
    assert!(h.model.operations.is_empty());
    assert_eq!(property_form(&h).review.as_ref(), Some(&reviewed));
    h.model.demo = false;
    assert!(h.has_text("connection or VM changed"));
    assert_eq!(h.try_click("property-apply"), Err(properties::STALE.into()));
    properties::apply(&mut h.model);
    assert!(h.model.operations.is_empty());
}

#[test]
fn nic_rate_review_has_correct_units_and_preserves_captured_qos() {
    let mut h = property_state();
    properties::close(&mut h.model);
    let nic = nic(&mut h).id;
    h.click("vm.nic.rate-limit");
    assert!(
        matches!(property_form(&h).choice, Some(Change::NicRate { vif, .. }) if vif == nic),
        "the rate limit opens past the chooser"
    );
    h.toggle("property-enabled");
    h.type_text("property-value", "12500");
    h.click("property-review");
    assert!(matches!(
        property_form(&h).review,
        Some(Change::NicRate { vif, kilobytes_per_second: Some(12500), .. }) if vif == nic
    ));
    assert!(h.has_text("kilobytes per second"));
    h.click("property-back");
    h.toggle("property-enabled");
    h.click("property-review");
    assert!(property_form(&h).review.is_none());
    assert!(
        property_form(&h)
            .error
            .as_ref()
            .unwrap()
            .contains("Change the rate limit")
    );
    assert!(h.has_text("Change the rate limit"));
}

#[test]
fn metadata_cbt_confirmation_and_invalid_values_require_explicit_valid_review() {
    let mut h = property_state();
    let choices = property_form(&h).choices.clone();
    let metadata = choices
        .iter()
        .find(|c| matches!(c, Change::DiskMetadata { .. }))
        .unwrap()
        .clone();
    properties::choose(&mut h.model, metadata);
    assert!(properties::build(property_form(&h)).is_err());
    let f = property_form_mut(&mut h);
    f.name = "Database data".into();
    f.description = "Reviewed description".into();
    properties::review(&mut h.model);
    assert!(matches!(
        property_form(&h).review,
        Some(Change::DiskMetadata { .. })
    ));
    let (vbd, vdi) = match &choices[0] {
        Change::DiskMetadata { vbd, vdi, .. } => (*vbd, *vdi),
        _ => unreachable!(),
    };
    properties::choose(&mut h.model, Change::DisableCbt { vbd, vdi });
    assert!(
        properties::build(property_form(&h))
            .unwrap_err()
            .contains("exact disk UUID")
    );
    property_form_mut(&mut h).value = vdi.to_string();
    assert!(properties::build(property_form(&h)).is_ok());
    assert!(
        properties::summary(&properties::build(property_form(&h)).unwrap())
            .contains("incremental backup chain")
    );
    let priority = choices
        .iter()
        .find(|c| matches!(c, Change::DiskPriority { .. }))
        .unwrap()
        .clone();
    properties::choose(&mut h.model, priority);
    property_form_mut(&mut h).enabled = true;
    for invalid in ["8", "-1", "", "1.5"] {
        property_form_mut(&mut h).value = invalid.into();
        assert!(properties::build(property_form(&h)).is_err());
    }
    properties::choose(
        &mut h.model,
        Change::DiskPosition {
            vbd,
            vdi,
            previous: "0".into(),
            value: String::new(),
        },
    );
    for invalid in ["0", "256", "01", ""] {
        property_form_mut(&mut h).value = invalid.into();
        assert!(properties::build(property_form(&h)).is_err());
    }
}

// -- disk_workspace_tests.rs ----------------------------------------------------

#[test]
fn copy_review_captures_target_and_blocks_demo_dispatch() {
    let mut h = Harness::scene("disk-copy");
    assert!(panel(&h).review.is_some(), "{:?}", panel(&h).error);
    assert_eq!(h.surface().id, "virtual-disk");
    assert!(h.surface().experimental);
    let id = panel(&h).review.as_ref().unwrap().disk;
    for key in ["disk-confirm", "disks-close", "disk-review-back"] {
        assert!(h.find(key).is_some(), "{key}");
    }
    let pool = pool(&h);
    h.model
        .select_in_grid(Target::one(ObjectRef::vm(pool, scenes::vm())));
    assert_eq!(panel(&h).review.as_ref().unwrap().disk, id);
    assert!(h.try_click("disk-confirm").is_err());
    disks::apply(&mut h.model);
    assert!(h.model.resource_operations.is_empty());
    h.model.demo = false;
    assert!(h.has_text("captured connection changed"));
    assert_eq!(h.try_click("disk-confirm"), Err(disks::STALE.into()));
}

#[test]
fn growth_rejects_shrink_and_copy_requires_name_and_chosen_storage() {
    let mut h = Harness::scene("disk-grow");
    assert!(panel(&h).review.is_some(), "{:?}", panel(&h).error);
    let p = panel_mut(&mut h);
    p.review = None;
    p.size = "0".into();
    disks::review(&mut h.model);
    assert!(panel(&h).review.is_none());
    let p = panel_mut(&mut h);
    p.mode = Mode::Copy;
    p.name = " ".into();
    disks::review(&mut h.model);
    assert!(panel(&h).review.is_none());
    let p = panel_mut(&mut h);
    p.name = "New copy".into();
    p.destination = None;
    disks::review(&mut h.model);
    assert!(panel(&h).review.is_none());
    assert_eq!(panel(&h).error.as_deref(), Some("Choose destination storage"));
    assert!(h.has_text("Choose destination storage"));
}

#[test]
fn cleanup_requires_local_ownership_and_current_consumer_graph() {
    let mut h = Harness::scene("disk-grow");
    let p = panel_mut(&mut h);
    p.review = None;
    p.mode = Mode::Cleanup;
    disks::review(&mut h.model);
    assert!(panel(&h).review.is_none());
    assert!(panel(&h).error.as_ref().unwrap().contains("ownership"));
    // The command says the same before the dialog opens.
    let (pool, disk) = (panel(&h).pool, panel(&h).disk);
    let target = Target::one(ObjectRef::new(Some(pool), ObjectKind::VirtualDisk, disk));
    assert_eq!(
        h.model.eligibility(commands::CLEANUP, &target),
        Eligibility::Disabled(disks::NO_OWNERSHIP.into())
    );
}

#[test]
fn cleanup_review_keeps_exact_uuid_and_explicit_destructive_confirmation() {
    let mut h = Harness::scene("disk-cleanup");
    let captured = panel(&h).review.as_ref().unwrap().disk;
    assert!(h.has_text("Permanently delete"));
    h.type_text("disk-delete-confirmation", &captured.to_string());
    assert_eq!(panel(&h).confirmation, captured.to_string());
    assert!(h.try_click("disk-confirm").is_err());
    disks::apply(&mut h.model);
    assert!(h.model.resource_operations.is_empty());
    assert_eq!(panel(&h).review.as_ref().unwrap().disk, captured);
    // Rune opened the pool history of the resource editor; it is the Events
    // tab of the Notifications view now (decision 6).
    h.click("disk-history");
    assert!(h.model.areas.vm_devices.disks.is_none());
    assert_eq!(h.model.nav.view, NavView::Notifications);
}

// -- main.rs: devices_keep_their_target_and_leave_compact_details ---------------

/// The part of the Rune test that still has a subject: seeding the demo
/// again closes what was open. (The pinned selection and "Back to
/// inventory" went with the Devices page.)
#[test]
fn reseeding_the_demo_closes_the_device_surfaces() {
    let mut h = devices();
    h.click("vm.cd.add");
    assert!(h.model.areas.vm_devices.lifecycle.is_some());
    h.model.load_demo(3, 48);
    let state = &h.model.areas.vm_devices;
    assert!(state.lifecycle.is_none() && state.properties.is_none() && state.disks.is_none());
    assert!(state.disk_row.is_none());
}

// -- what the port added ----------------------------------------------------------

#[test]
fn the_storage_tab_lists_attachments_and_the_card_of_the_selected_row() {
    let mut h = Harness::scene("vm-devices");
    h.assert_unique_keys();
    let vbd = scenes::disk_attachment();
    assert!(h.find("storage-vbds").is_some());
    assert!(h.find("vm-disk-details").is_some());
    assert!(h.has_text("Demo system disk"));
    assert!(h.has_text(&vbd.to_string()), "the attachment UUID is shown");
    h.model.areas.vm_devices.disk_row = None;
    assert!(h.find("vm-disk-details").is_none());
    // The row commands say what is missing; the adding commands do not need
    // a row.
    assert_eq!(
        h.eligibility("vm.disk.detach"),
        Eligibility::Disabled(commands::SELECT_DISK.into())
    );
    assert_eq!(h.eligibility("vm.cd.insert"), Eligibility::Hidden);
    assert_eq!(h.eligibility("vm.disk.new"), Eligibility::Enabled);
    // A CD/DVD drive has no disk commands, a disk no media commands.
    let cd = inventory(&h)
        .details
        .vbds
        .values()
        .find(|v| v.vm == Some(scenes::vm()) && actions::is_cd(v))
        .unwrap()
        .id;
    h.select_row("storage-vbds", &cd.to_string());
    for hidden in ["vm.disk.detach", "vm.disk.delete", "vm.disk.properties", "disk.grow"] {
        assert_eq!(h.eligibility(hidden), Eligibility::Hidden, "{hidden}");
    }
    assert_ne!(h.eligibility("vm.cd.insert"), Eligibility::Hidden);
    h.select_row("storage-vbds", &vbd.to_string());
    for hidden in ["vm.cd.insert", "vm.cd.eject", "vm.disk.deactivate"] {
        assert_eq!(h.eligibility(hidden), Eligibility::Hidden, "{hidden}");
    }
    // Connecting needs a live connection: never in a synthetic pool.
    assert_eq!(
        h.eligibility("vm.disk.activate"),
        Eligibility::Disabled(actions::RECONNECT.into())
    );
    // The row menu is the menu of the row commands.
    let menu = h.open_row_menu("storage-vbds", &vbd.to_string());
    assert!(menu.iter().any(|item| item.id == commands::DETACH));
    assert!(menu.iter().all(|item| item.id != commands::INSERT));
}

#[test]
fn a_running_guest_can_only_be_looked_at_and_says_why() {
    let mut h = Harness::scene("vm-devices");
    let vm = scenes::vm();
    scenes::amend(&mut h.model, |i| {
        i.vms.get_mut(&vm).unwrap().power_state = xenadmin_core::PowerState::Running;
    });
    for command in ["vm.disk.new", "vm.disk.attach", "vm.cd.add"] {
        assert_eq!(
            h.eligibility(command),
            Eligibility::Disabled(lifecycle::NOT_EDITABLE.into()),
            "{command}"
        );
    }
    assert_eq!(
        h.try_click("vm.disk.new"),
        Err(lifecycle::NOT_EDITABLE.into())
    );
    assert!(h.model.areas.vm_devices.lifecycle.is_none());
}

#[test]
fn a_template_shows_its_devices_and_offers_no_change() {
    let mut h = Harness::scene("vm-devices");
    let vm = scenes::vm();
    scenes::amend(&mut h.model, |i| {
        i.vms.get_mut(&vm).unwrap().is_template = true;
    });
    let template = Target::one(ObjectRef::template(pool(&h), vm));
    let body = super::tabs::tab(&h.model, TabId::Storage, &template).expect("a Storage tab");
    let nodes = crate::surface::flatten_blocks("tab", &body.blocks());
    assert!(nodes.iter().any(|n| n.key.as_deref() == Some("storage-vbds")));
    assert!(super::tabs::tab(&h.model, TabId::Networking, &template).is_some());
    for spec in commands::commands() {
        assert_eq!(
            h.model.eligibility(spec.id, &template),
            Eligibility::Hidden,
            "{}",
            spec.id.0
        );
    }
}

#[test]
fn the_networking_tab_lists_interfaces_guest_tools_and_the_row_commands() {
    let mut h = Harness::scene("vm-devices");
    h.open_tab(TabId::Networking);
    h.assert_unique_keys();
    assert!(h.find("networking-vifs-table").is_some());
    assert!(h.find("guest-tools").is_some());
    assert_eq!(
        h.eligibility("vm.nic.remove"),
        Eligibility::Disabled(commands::SELECT_NIC.into())
    );
    let nic = nic(&mut h);
    assert!(h.find("vm-nic-details").is_some());
    assert!(h.has_text(&nic.id.to_string()));
    assert_eq!(h.eligibility("vm.nic.properties"), Eligibility::Enabled);
    // No interface commands on the Storage tab's row, no disk commands here.
    let local: Vec<_> = h.local_row().into_iter().map(|item| item.id).collect();
    assert!(local.contains(&commands::NIC_ADD));
    assert!(!local.contains(&commands::NEW));
    // A QoS the editor does not understand is refused before the dialog.
    scenes::amend(&mut h.model, |i| {
        i.details.vifs.get_mut(&nic.id).unwrap().qos_algorithm = "other".into();
    });
    assert_eq!(
        h.eligibility("vm.nic.rate-limit"),
        Eligibility::Disabled(properties::NO_RATE.into())
    );
}

#[test]
fn an_unreadable_class_replaces_the_grid_with_its_sentence() {
    let mut h = Harness::scene("vm-devices");
    scenes::amend(&mut h.model, |i| {
        i.details
            .classes
            .insert("vbd".into(), ClassAvailability::PermissionDenied);
    });
    assert!(h.find("storage-vbds").is_none());
    assert_eq!(
        h.node("devices-unavailable-vbd").text.as_deref(),
        Some("Disk attachments are unavailable: this account does not have permission to read them.")
    );
}

#[test]
fn another_object_in_the_tree_closes_the_forms_and_forgets_the_rows() {
    let mut h = devices();
    h.click("vm.cd.add");
    let pool = pool(&h);
    let other = *inventory(&h)
        .vms
        .keys()
        .find(|id| **id != scenes::vm())
        .unwrap();
    assert!(h.model.areas.vm_devices.lifecycle.is_some());
    h.model.select(Target::one(ObjectRef::vm(pool, other)));
    assert!(h.model.areas.vm_devices.lifecycle.is_none());
    assert!(h.model.areas.vm_devices.disk_row.is_none());
}

/// The detached, resizable disk of the disk scenes, and its repository.
fn detached_disk(h: &mut Harness) -> (VdiId, uuid::Uuid) {
    let mut found = None;
    scenes::amend(&mut h.model, |i| {
        let disk = i
            .details
            .vdis
            .values_mut()
            .find(|d| d.kind == "user")
            .unwrap();
        disk.cbt_enabled = Some(false);
        disk.allowed_operations = Some(vec!["resize".into(), "copy".into(), "destroy".into()]);
        let id = disk.id;
        found = Some((id, disk.sr.unwrap().uuid()));
        i.details.vbds.retain(|_, v| v.vdi != Some(id));
    });
    found.unwrap()
}

#[test]
fn a_repository_lists_its_disks_and_a_selected_disk_can_be_copied() {
    let mut h = Harness::scene("demo");
    let (disk, repository) = detached_disk(&mut h);
    let pool = pool(&h);
    h.select(Target::one(ObjectRef::storage(pool, repository)));
    h.open_tab(TabId::Storage);
    h.assert_unique_keys();
    assert!(h.find("storage-pbds").is_some());
    assert!(h.find("repository-disks").is_some());
    assert_eq!(
        h.eligibility("disk.copy"),
        Eligibility::Disabled(commands::SELECT_VIRTUAL_DISK.into())
    );
    assert_eq!(h.eligibility("vm.disk.new"), Eligibility::Hidden);
    // The search field narrows the list; it is the Rune `disk-search`.
    h.type_text("disk-search", "no such disk");
    assert!(h.has_text("0 disks on selected storage"));
    h.type_text("disk-search", &disk.to_string());
    assert!(h.has_text("1 disks on selected storage"));
    h.select_row("repository-disks", &disk.to_string());
    let selected = ObjectRef::new(Some(pool), ObjectKind::VirtualDisk, disk);
    assert_eq!(h.model.target(), Target::one(selected));
    assert_eq!(super::selected_disk(&h.model), Some((pool, disk)));
    h.click("disk.copy");
    assert_eq!(h.surface().id, "virtual-disk");
    assert_eq!(panel(&h).disk, disk);
    assert_eq!(panel(&h).mode, Mode::Copy);
    assert_eq!(
        panel(&h).destination.map(|sr| sr.uuid()),
        Some(repository),
        "the copy stays on the repository of the list unless another is picked"
    );
    h.type_text("disk-copy-name", "Second copy");
    h.click("disk-review");
    assert!(panel(&h).review.is_some(), "{:?}", panel(&h).error);
    assert!(h.has_text("Create a detached copy named Second copy"));
    h.click("disk-review-back");
    assert!(panel(&h).review.is_none());
    h.click("disks-close");
    assert!(h.surfaces().is_empty());
}

#[test]
fn a_disk_without_complete_records_cannot_be_resized_and_says_why() {
    let mut h = Harness::scene("vm-devices");
    let vbd = scenes::disk_attachment();
    let disk = inventory(&h).details.vbds[&vbd].vdi.unwrap();
    scenes::amend(&mut h.model, |i| {
        i.details.vdis.get_mut(&disk).unwrap().virtual_size = None;
    });
    assert_eq!(
        h.eligibility("disk.grow"),
        Eligibility::Disabled(disks::INCOMPLETE.into())
    );
}

#[test]
fn the_properties_dialog_of_a_disk_edits_its_name_one_review_at_a_time() {
    let mut h = Harness::scene("vm-devices");
    let vbd = scenes::disk_attachment();
    let disk = inventory(&h).details.vbds[&vbd].vdi.unwrap();
    let target = Target::one(ObjectRef::new(
        Some(pool(&h)),
        ObjectKind::VirtualDisk,
        disk,
    ));
    // This area's two pages, among those other areas provide for a disk.
    let pages: Vec<_> = h
        .model
        .property_pages(&target)
        .into_iter()
        .map(|(area, page)| (area, page.id))
        .filter(|(area, _)| *area == "vm_devices")
        .collect();
    assert_eq!(
        pages,
        [("vm_devices", "general"), ("vm_devices", "size-location")]
    );
    h.invoke_on("disk.properties", &target);
    assert_eq!(h.surface().id, "disk-properties");
    assert!(h.surface().experimental);
    assert!(property_form(&h).page);
    assert!(
        h.find("property-back").is_none(),
        "the page has no chooser to go back to"
    );
    h.click("property-review");
    assert!(h.has_text("Change the name or description before reviewing."));
    h.type_text("property-name", "Renamed disk");
    h.click("property-review");
    assert!(matches!(
        &property_form(&h).review,
        Some(Change::DiskMetadata { name, vdi, .. }) if name == "Renamed disk" && *vdi == disk
    ));
    assert_eq!(
        h.try_click("property-apply"),
        Err(properties::PREVIEW.into())
    );
    h.click("property-back");
    assert!(property_form(&h).review.is_none());
    // Size and Location has no draft; leaving General drops its form.
    h.pick("disk-properties-pages", "size-location");
    assert!(h.model.areas.vm_devices.properties.is_none());
    assert!(h.has_text("Virtual size"));
    h.click("disk-properties-close");
    assert!(h.surfaces().is_empty());
    // A disk no VM uses has nothing these modules can rename.
    scenes::amend(&mut h.model, |i| {
        i.details.vbds.retain(|_, v| v.vdi != Some(disk));
    });
    let pages: Vec<_> = h
        .model
        .property_pages(&target)
        .into_iter()
        .filter(|(area, _)| *area == "vm_devices")
        .map(|(_, page)| page.id)
        .collect();
    assert_eq!(pages, ["size-location"]);
}

#[test]
fn the_device_properties_chooser_offers_only_what_the_host_allows() {
    let mut h = property_state();
    h.click("property-back");
    assert!(property_form(&h).choice.is_none());
    let choices = property_form(&h).choices.clone();
    assert!(matches!(choices[0], Change::DiskMetadata { .. }));
    let index = choices
        .iter()
        .position(|c| matches!(c, Change::Bootable { .. }))
        .expect("the bootable flag is offered");
    h.pick("property-choice", &index.to_string());
    assert!(matches!(
        property_form(&h).choice,
        Some(Change::Bootable { .. })
    ));
    h.click("property-review");
    assert!(h.has_text("does not reorder the VM firmware boot sequence"));
    h.click("property-cancel");
    assert!(h.surfaces().is_empty());
}

/// Hazard 1 of the packet: Rune's `apply` of a device change relied on the
/// disabled button. Here it checks availability and the observation again.
#[test]
fn a_device_change_is_checked_again_when_it_is_applied() {
    let mut live = Live::with_vm();
    let vm = live.vm();
    let (pool, id) = (vm.pool.unwrap(), vm.vm_id().unwrap());
    live.h.select(Target::one(vm));
    let vif = VifId::parse("73000000-0000-0000-0000-0000000000aa").unwrap();
    let operation = DeviceOperation::PlugNic { vif };
    assert!(actions::open(
        &mut live.h.model,
        pool,
        id,
        Some(operation),
        None
    ));
    assert_eq!(live.h.surface().id, "device-change");
    assert!(live.h.surface().experimental);
    assert!(live.h.has_text("no longer available"));
    assert!(live.h.try_click("apply-device-change").is_err());
    actions::apply(&mut live.h.model);
    assert!(live.h.model.areas.vm_devices.form.is_some());
    assert!(live.h.model.operations.is_empty());
    live.h.click("cancel-device-change");
    assert!(live.h.surfaces().is_empty());
}

/// Decision 10 of `AREAS.md`: "Operation history" is the button of this
/// dialog, which leads to the Events tab; a review names that tab.
#[test]
fn texts_name_the_places_of_this_client() {
    let h = Harness::scene("disk-grow");
    assert!(h.has_text(
        "Unknown outcomes reserve the pool. Operation history provides rechecks, stop controls and explicit acknowledgement. Stop remaining steps does not cancel in-flight disk I/O."
    ));
    assert_eq!(
        h.node("disk-history").label.as_deref(),
        Some("Operation history")
    );
    let i = inventory(&h);
    let review = review_for(
        DeviceOperation::CreateDisk {
            sr: *i.details.storage.keys().next().unwrap(),
            name: "Data".into(),
            size: 512,
            device: "1".into(),
        },
        &i,
    );
    assert!(review.lines[1].contains("identified on the Events tab of the Notifications view"));
}
