//! Tests of the area. The Rune tests of `bulk_delete_tests.rs` and
//! `bulk_properties_tests.rs` are here under their names; the others are
//! new for what the port changed (commands, the page, the hooks).
//!
//! Retired with the Rune layout: the two window sizes of the Rune tests
//! (513x652 and 1440x900) and "the footer is inside the window"; the footer
//! is the frame's. Their GPUI halves are in `gpui-tests-wanted.md`.

use xenadmin_core::{ClassAvailability, Inventory, PowerState, VmId, VmSettingEdit, VmWorkflow};

use super::commands::{DELETE, TEMPLATES_DELETE};
use super::property::Property;
use super::{delete, edit, scenes};
use crate::areas::vm_lifecycle;
use crate::selection::MenuOrigin;
use crate::{Harness, prelude::*, properties};

fn amend(h: &mut Harness, change: impl FnOnce(&mut Inventory)) {
    scenes::amend(&mut h.model, change);
}

fn inventory(h: &Harness) -> &Inventory {
    h.model.inventory.as_ref().expect("inventory")
}

/// The demo pool, nothing open.
fn demo(vms: usize) -> Harness {
    let mut m = Model::fixture();
    m.load_demo(3, vms);
    Harness::with(m)
}

/// The first `count` guests of the demo pool, as a target. Not selected.
fn guests(h: &Harness, count: usize) -> Target {
    let i = inventory(h);
    Target::many(
        i.guests()
            .take(count)
            .map(|vm| ObjectRef::vm(i.pool_id, vm.id)),
    )
}

/// The first two guests made halted custom templates, as a target.
fn templates(h: &mut Harness) -> Target {
    let ids = guests(h, 2).vms();
    amend(h, |i| {
        for id in &ids {
            let vm = i.vms.get_mut(id).unwrap();
            vm.is_template = true;
            vm.power_state = PowerState::Halted;
            i.details.vms.get_mut(id).unwrap().is_default_template = Some(false);
        }
    });
    let pool = inventory(h).pool_id;
    Target::many(ids.into_iter().map(|id| ObjectRef::template(pool, id)))
}

fn del(h: &Harness) -> &delete::Editor {
    h.model.areas.bulk_edit.delete.as_ref().expect("deletion")
}

fn del_mut(h: &mut Harness) -> &mut delete::Editor {
    h.model.areas.bulk_edit.delete.as_mut().expect("deletion")
}

fn props(h: &Harness) -> &edit::Editor {
    h.model
        .areas
        .bulk_edit
        .properties
        .as_ref()
        .expect("bulk properties")
}

fn props_mut(h: &mut Harness) -> &mut edit::Editor {
    h.model
        .areas
        .bulk_edit
        .properties
        .as_mut()
        .expect("bulk properties")
}

fn proposed(h: &Harness) -> Result<edit::Review, String> {
    edit::proposed(&h.model, props(h))
}

/// The bulk page of the first two guests, open on its draft (the Rune
/// `select_two` and a press on "Edit selection…").
fn two_open() -> (Harness, Vec<VmId>) {
    let mut h = demo(48);
    let target = guests(&h, 2);
    h.invoke_on(properties::VM.0, &target);
    assert_eq!(h.surface().id, "vm-properties");
    let ids = props(&h).targets.iter().map(|vm| vm.id).collect();
    (h, ids)
}

/// The Rune `target`: a fresh demo whose first guest, as a VM or as a
/// custom template, is halted, knows every value and advertises every live
/// capability; the editor captured for it alone. (No command opens the
/// bulk page for one object here; the editor still takes one.)
fn target(h: &mut Harness, template: bool) -> VmId {
    h.model.load_demo(3, 48);
    let id = inventory(h).guests().next().unwrap().id;
    amend(h, |i| {
        let vm = i.vms.get_mut(&id).unwrap();
        vm.is_template = template;
        vm.power_state = PowerState::Halted;
        let d = i.details.vms.get_mut(&id).unwrap();
        d.is_default_template = Some(false);
        d.current_operations.clear();
        d.allowed_operations = Some(
            [
                "changing_VCPUs_live",
                "changing_dynamic_range",
                "changing_shadow_memory",
                "changing_shadow_memory_live",
            ]
            .map(str::to_owned)
            .into(),
        );
        d.blocked_operations.clear();
        d.vcpus_max = Some(8);
        d.vcpus_at_startup = Some(2);
        d.memory_static_min = Some(512 << 20);
        d.memory_dynamic_min = Some(1 << 30);
        d.memory_dynamic_max = Some(2 << 30);
        d.memory_static_max = Some(4 << 30);
        d.affinity_known = true;
        d.affinity = None;
        d.domain_type = Some("hvm".into());
        d.boot_policy = Some("BIOS order".into());
        d.boot_order = Some("cd".into());
        d.topology_known = true;
        d.cores_per_socket = Some("1".into());
        d.cpu_weight_known = true;
        d.cpu_weight = None;
        d.shadow_multiplier = Some("1".into());
        d.auto_power_on = Some(false);
        d.firmware = Some("uefi".into());
        d.secure_boot = Some(false);
        i.details
            .classes
            .insert("vbd".into(), ClassAvailability::Available);
        i.details.vbds.retain(|_, v| v.vm != Some(id));
    });
    let pool = inventory(h).pool_id;
    let object = if template {
        ObjectRef::template(pool, id)
    } else {
        ObjectRef::vm(pool, id)
    };
    let editor = edit::capture(&h.model, &Target::one(object)).expect("captured");
    h.model.areas.bulk_edit.properties = Some(editor);
    id
}

/// The Rune `property`: the property chosen (ignored under a review), the
/// values set whatever the step.
fn property(h: &mut Harness, p: Property, values: &[&str]) {
    edit::choose(&mut h.model, p);
    for (field, value) in props_mut(h).values.iter_mut().zip(values) {
        *field = (*value).to_owned();
    }
}

// -- bulk_delete_tests.rs -----------------------------------------------------------

#[test]
fn compact_and_wide_reviews_keep_exact_targets_and_the_confirmation_footer() {
    let mut h = Harness::scene("batch-delete");
    assert_eq!(h.surface().id, "batch-delete");
    h.assert_unique_keys();
    let review = del(&h).review.clone().expect("review");
    assert_eq!(review.batch.items.len(), 2);
    assert!(review.batch.items.iter().all(|i| matches!(
        &i.workflow,
        VmWorkflow::Delete { disks, snapshots, .. } if disks.is_empty() && snapshots.is_empty()
    )));
    assert!(review.rows.iter().all(|lines| {
        lines
            .iter()
            .any(|l| l.contains("Retain disk") || l.contains("Record-only"))
    }));
    // The review is of what was captured, whatever is selected afterwards.
    let other = guests(&h, 6).objects[5].clone();
    h.model.select_in_grid(Target::one(other));
    assert!(del(&h).review.as_ref() == Some(&review));
    assert!(h.find("batch-delete-close").is_some());
    assert!(h.has_text("Type DELETE 2 to confirm irreversible deletion."));
    assert!(h.has_text(delete::SEQUENCE));
    h.type_text("batch-delete-confirm", "DELETE 2");
    assert_eq!(del(&h).confirm, "DELETE 2");
    // The demo cannot submit: the button says so, and the handler refuses.
    assert_eq!(
        h.try_click("batch-delete-submit"),
        Err(delete::PREVIEW.to_owned())
    );
    delete::submit(&mut h.model);
    assert!(h.model.resource_operations.is_empty());
    assert_eq!(
        del(&h).error,
        Some(xenadmin_core::Error::Stale.to_string())
    );
    h.click("batch-delete-close");
    assert!(h.model.areas.bulk_edit.delete.is_none());
    assert!(h.surfaces().is_empty());
}

#[test]
fn changed_or_missing_target_refuses_the_entire_review() {
    let mut h = Harness::scene("batch-delete");
    let selected = del(&h).choices[0].vm.id;
    amend(&mut h, |i| {
        i.vms.get_mut(&selected).unwrap().power_state = PowerState::Running;
    });
    let error = delete::build_review(&h.model, del(&h)).err().unwrap();
    assert!(error.contains(&selected.to_string()));
    // The dialog shows the refusal and the reviewed batch cannot be sent.
    assert_eq!(h.node("batch-delete-proposal").text, Some(error));
    assert!(h.try_click("batch-delete-submit").is_err());
    amend(&mut h, |i| {
        i.vms.remove(&selected);
    });
    assert!(delete::build_review(&h.model, del(&h)).is_err());
}

#[test]
fn selected_disk_data_is_explicit_and_new_ownership_cannot_be_silently_omitted() {
    let mut h = Harness::scene("batch-delete");
    h.type_text("batch-delete-confirm", "DELETE 2");
    h.click("batch-delete-back");
    assert!(del(&h).review.is_none());
    assert_eq!(del(&h).confirm, "");
    assert!(h.has_text(delete::RECORD_ONLY));
    let e = del(&h);
    let (index, disk) = e
        .choices
        .iter()
        .enumerate()
        .find_map(|(n, c)| {
            e.initial
                .vm_deletion_disks(c.vm.id)
                .first()
                .copied()
                .map(|d| (n, d))
        })
        .expect("fixture owned disk");
    h.click(&format!("batch-delete-disk-{disk}"));
    assert!(del(&h).choices[index].disks.contains(&disk));
    h.click("batch-delete-review");
    assert!(
        del(&h)
            .review
            .as_ref()
            .unwrap()
            .rows
            .iter()
            .flatten()
            .any(|s| s.contains("DELETE DISK DATA") && s.contains(&disk.to_string()))
    );
    assert_eq!(del(&h).confirm, "");
    amend(&mut h, |i| {
        i.details.vdis.get_mut(&disk).unwrap().sharable = Some(true);
    });
    assert!(delete::build_review(&h.model, del(&h)).is_err());
}

#[test]
fn template_and_read_only_reviews_do_not_drop_ineligible_rows() {
    let mut h = Harness::scene("batch-delete");
    let id = del(&h).choices[0].vm.id;
    amend(&mut h, |i| {
        i.vms.get_mut(&id).unwrap().is_template = true;
        i.details.vms.get_mut(&id).unwrap().is_default_template = Some(true);
    });
    del_mut(&mut h).choices[0].vm.is_template = true;
    assert!(delete::build_review(&h.model, del(&h)).is_err());
    h.model.demo = false;
    del_mut(&mut h).observed = None;
    assert!(!delete::current(&h.model, del(&h)));
    assert_eq!(
        h.try_click("batch-delete-submit"),
        Err(delete::STALE.to_owned())
    );
    let before = h.model.resource_operations.len();
    delete::submit(&mut h.model);
    assert_eq!(h.model.resource_operations.len(), before);
    assert!(h.model.areas.bulk_edit.delete.is_some());
}

// -- bulk_properties_tests.rs -------------------------------------------------------

#[test]
fn property_batch_multiselect_review_is_immutable_and_footer_reachable_at_both_sizes() {
    let (mut h, _) = two_open();
    h.type_text("batch-property-value-0", "Common reviewed description");
    h.click("batch-properties-review");
    let reviewed = props(&h).review.clone().unwrap();
    assert_eq!(reviewed.batch.vms.len(), 2);
    assert!(h.find("batch-properties-submit").is_some());
    assert!(h.has_text("Session permissions are unavailable"));
    h.assert_unique_keys();
    // Another selection and another property do not touch the review.
    let other = guests(&h, 6).objects[5].clone();
    h.model.select_in_grid(Target::one(other));
    edit::choose(&mut h.model, Property::Name);
    edit::set_value(&mut h.model, 0, "Not this");
    assert_eq!(props(&h).property, Property::Description);
    assert!(props(&h).review.as_ref() == Some(&reviewed));
    assert_eq!(
        h.try_click("batch-properties-submit"),
        Err(edit::DEMO.to_owned())
    );
    edit::submit(&mut h.model);
    assert!(h.model.resource_operations.is_empty());
    assert!(h.model.properties.is_some());
    // Before any tick: the pool under the review is another one.
    amend(&mut h, |i| i.pool_id = uuid::Uuid::new_v4().into());
    assert!(proposed(&h).is_err());
    assert!(h.has_text("Connection changed"));
}

#[test]
fn property_batch_names_every_unchanged_target_and_refuses_all_noop() {
    let (mut h, ids) = two_open();
    amend(&mut h, |i| {
        i.vms.get_mut(&ids[0]).unwrap().description = "desired".into();
    });
    h.type_text("batch-property-value-0", "desired");
    h.click("batch-properties-review");
    let r = props(&h).review.as_ref().unwrap();
    assert_eq!(r.batch.vms.len(), 1);
    assert_eq!(r.batch.vms[0].vm, ids[1]);
    assert_eq!(r.rows.len(), 2);
    assert!(h.has_text("Already desired"));
    assert!(h.has_text(&ids[0].to_string()));
    assert!(h.has_text("1 changes · 1 already desired"));
    h.click("batch-properties-back");
    amend(&mut h, |i| {
        i.vms.get_mut(&ids[1]).unwrap().description = "desired".into();
    });
    // Nothing to change: the button says so, and so does the handler.
    assert_eq!(
        h.try_click("batch-properties-review"),
        Err(edit::NOTHING_TO_CHANGE.to_owned())
    );
    edit::review(&mut h.model);
    assert!(props(&h).review.is_none());
    assert!(props(&h).error.as_ref().unwrap().contains("All selected"));
}

#[test]
fn property_batch_missing_unsupported_and_changed_targets_block_whole_review() {
    let (mut h, ids) = two_open();
    property(&mut h, Property::Weight, &["512"]);
    amend(&mut h, |i| {
        i.details.vms.get_mut(&ids[1]).unwrap().cpu_weight_known = false;
    });
    edit::review(&mut h.model);
    let e = props(&h);
    assert!(e.review.is_none());
    assert!(e.error.as_ref().unwrap().contains(&ids[1].to_string()));
    assert!(h.try_click("batch-properties-review").is_err());
    property(&mut h, Property::Description, &["desired"]);
    amend(&mut h, |i| {
        i.details.vms.remove(&ids[0]);
    });
    edit::review(&mut h.model);
    assert!(props(&h).error.as_ref().unwrap().contains("unknown"));
    amend(&mut h, |i| {
        i.vms.get_mut(&ids[1]).unwrap().power_state = PowerState::Unknown("fixture".into());
    });
    edit::review(&mut h.model);
    let e = props(&h);
    assert!(e.error.as_ref().unwrap().contains("power state changed"));
    assert!(e.review.is_none());
}

#[test]
fn property_batch_all_existing_property_kinds_bind_per_target_previous_values() {
    let mut h = demo(48);
    let id = target(&mut h, false);
    let cases = [
        (Property::Name, vec!["New name"]),
        (Property::Description, vec!["New description"]),
        (Property::Cpus, vec!["6", "3"]),
        (Property::Memory, vec!["512 MiB", "2 GiB", "3 GiB", "4 GiB"]),
        (Property::Topology, vec!["2"]),
        (Property::Weight, vec!["512"]),
        (Property::Shadow, vec!["2"]),
        (Property::BootOrder, vec!["dc"]),
        (Property::Autostart, vec![]),
        (Property::SecureBoot, vec![]),
        (Property::HomeHost, vec![]),
    ];
    for (p, values) in cases {
        property(&mut h, p, &values);
        props_mut(&mut h).toggle = true;
        if p == Property::HomeHost {
            let host = inventory(&h).hosts.keys().next().copied();
            props_mut(&mut h).host = host;
        }
        let r = proposed(&h).unwrap_or_else(|error| panic!("{p:?}: {error}"));
        assert_eq!(r.batch.vms.len(), 1, "{p:?}");
        assert_eq!(r.batch.vms[0].vm, id);
        assert_eq!(r.batch.vms[0].patch.edits().len(), 1);
        assert!(r.rows[0].summary.contains('→'));
    }
    amend(&mut h, |i| {
        i.vms.get_mut(&id).unwrap().power_state = PowerState::Running;
    });
    props_mut(&mut h).targets[0].power_state = PowerState::Running;
    property(&mut h, Property::LiveCpus, &["4"]);
    let r = proposed(&h).unwrap();
    assert!(matches!(
        r.batch.vms[0].patch.edits(),
        [VmSettingEdit::LiveCpus {
            previous: 2,
            max: 8,
            value: 4
        }]
    ));
    property(&mut h, Property::DynamicMemory, &["2 GiB", "3 GiB"]);
    assert_eq!(proposed(&h).unwrap().batch.vms.len(), 1);
    let id = target(&mut h, true);
    property(&mut h, Property::Firmware, &[]);
    props_mut(&mut h).toggle = false;
    let r = proposed(&h).unwrap();
    assert!(r.batch.vms[0].patch.is_template());
    assert!(matches!(
        r.batch.vms[0].patch.edits(),
        [VmSettingEdit::Firmware { previous, value }] if previous == "uefi" && value == "bios"
    ));
    amend(&mut h, |i| {
        i.details.vms.get_mut(&id).unwrap().is_default_template = Some(true);
    });
    assert!(proposed(&h).err().unwrap().contains("built-in"));
}

#[test]
fn property_batch_live_limits_capabilities_and_stale_review_refuse_submission() {
    let mut h = demo(48);
    let id = target(&mut h, false);
    property(&mut h, Property::LiveCpus, &["4"]);
    assert!(proposed(&h).is_err());
    amend(&mut h, |i| {
        i.vms.get_mut(&id).unwrap().power_state = PowerState::Running;
    });
    props_mut(&mut h).targets[0].power_state = PowerState::Running;
    property(&mut h, Property::LiveCpus, &["1"]);
    assert!(proposed(&h).is_err());
    property(&mut h, Property::LiveCpus, &["9"]);
    assert!(proposed(&h).is_err());
    property(&mut h, Property::DynamicMemory, &["1 GiB", "8 GiB"]);
    assert!(proposed(&h).is_err());
    property(&mut h, Property::LiveCpus, &["4"]);
    edit::review(&mut h.model);
    assert!(props(&h).review.is_some());
    amend(&mut h, |i| {
        i.details.vms.get_mut(&id).unwrap().vcpus_at_startup = Some(3);
    });
    assert!(proposed(&h).ok() != props(&h).review);
    edit::submit(&mut h.model);
    assert!(h.model.resource_operations.is_empty());
    property(&mut h, Property::LiveCpus, &["4"]);
    amend(&mut h, |i| {
        i.details
            .vms
            .get_mut(&id)
            .unwrap()
            .blocked_operations
            .insert(
                "changing_VCPUs_live".into(),
                "administrative restriction".into(),
            );
    });
    assert!(proposed(&h).is_err());
}

/// The Rune test pressed "Edit selection…" on the Templates page, chose
/// Firmware in the picker, then opened the row menu with the keyboard and
/// pressed "Edit properties for selection…"; an item pressed after the
/// selection changed did nothing. Here the entry is the kernel's
/// `template.properties` / `vm.properties` on several objects, the picker
/// is one choice, and the menu is the kernel's context menu.
#[test]
fn property_batch_picker_and_template_entry_work_with_keyboard_menu_capture() {
    let mut h = demo(48);
    let several = templates(&mut h);
    h.invoke_on(properties::TEMPLATE.0, &several);
    assert_eq!(h.surface().id, "template-properties");
    assert!(h.find("batch-property-Firmware").is_some());
    h.pick("batch-property", "Firmware");
    assert_eq!(props(&h).property, Property::Firmware);
    assert_eq!(
        h.node("batch-property-toggle-true").label.as_deref(),
        Some("UEFI")
    );
    h.click("batch-properties-close");
    assert!(h.model.areas.bulk_edit.properties.is_none());
    assert!(h.surfaces().is_empty());
    // The context menu of the selection has the entry, enabled.
    assert!(
        h.context_menu(&several)
            .iter()
            .any(|item| item.id == properties::TEMPLATE && item.disabled.is_none())
    );
    // A menu opened on two VMs of the grid; the selection changes; the
    // item does nothing.
    let mut h = demo(48);
    let two = guests(&h, 2);
    let pool = inventory(&h).pool_id;
    h.select(Target::one(ObjectRef::pool(pool)));
    h.model.select_in_grid(two.clone());
    assert!(
        h.model
            .open_context_menu(two.objects[0].clone(), MenuOrigin::Grid)
    );
    assert!(
        h.model
            .context_menu()
            .iter()
            .flatten()
            .any(|item| item.id == properties::VM && item.disabled.is_none())
    );
    let other = guests(&h, 5).objects[4].clone();
    h.model.select_in_grid(Target::one(other));
    assert!(h.model.invoke_context(properties::VM).is_err());
    assert!(h.model.areas.bulk_edit.properties.is_none());
    assert!(h.model.properties.is_none());
    // Unchanged, the same item opens the page for both.
    h.model.select_in_grid(two.clone());
    assert!(
        h.model
            .open_context_menu(two.objects[0].clone(), MenuOrigin::Grid)
    );
    assert!(h.model.invoke_context(properties::VM).is_ok());
    assert_eq!(props(&h).targets.len(), 2);
}

// -- Shares of Rune tests that live in other files ----------------------------------

/// This area's share of Rune `main.rs`
/// `experimental_notice_is_on_every_surface_outside_the_supported_core`.
#[test]
fn rune_experimental_notice_is_on_both_surfaces_of_the_area() {
    for scene in scenes::scenes() {
        let h = Harness::scene(scene.name);
        assert!(h.surface().experimental, "{}", scene.name);
        assert!(h.model.experimental_notice(), "{}", scene.name);
        h.assert_unique_keys();
    }
    // The drafts too.
    let mut h = Harness::scene("batch-delete");
    h.click("batch-delete-back");
    assert!(h.surface().experimental);
    h.assert_unique_keys();
    let (h, _) = two_open();
    assert!(h.surface().experimental);
    h.assert_unique_keys();
}

/// This area's share of Rune `session_permissions_view.rs`
/// `review_permission_notice_is_reachable_without_bypassing_synthetic_target_guard`.
/// The batch deletion never had the line: a denial refuses its review.
#[test]
fn review_permission_notice_is_reachable_without_bypassing_synthetic_target_guard() {
    let mut h = Harness::scene("batch-properties");
    assert_eq!(h.surface().id, "vm-properties");
    assert_eq!(h.surface().subtitle.as_deref(), Some("3 VMs"));
    assert_eq!(props(&h).review.as_ref().unwrap().batch.vms.len(), 3);
    assert!(h.find(permissions::REVIEW_KEY).is_some());
    assert_eq!(
        h.try_click("batch-properties-submit"),
        Err(edit::DEMO.to_owned())
    );
    assert!(h.model.resource_operations.is_empty());
    // Before the review there is no batch to assess, so no line.
    h.click("batch-properties-back");
    assert!(h.find(permissions::REVIEW_KEY).is_none());
    assert!(Harness::scene("batch-delete").find(permissions::REVIEW_KEY).is_none());
}

// -- The commands, the page and the hooks -------------------------------------------

#[test]
fn deleting_several_takes_the_place_of_deleting_one() {
    let mut h = demo(48);
    let two = guests(&h, 2);
    let one = Target::one(two.objects[0].clone());
    // One object: these are hidden, and `vm.delete` is the item.
    for command in [DELETE, TEMPLATES_DELETE] {
        assert_eq!(h.model.eligibility(command, &one), Eligibility::Hidden);
    }
    // Several: one "Delete VM" in the context menu, this area's.
    let items: Vec<_> = h
        .context_menu(&two)
        .into_iter()
        .filter(|item| item.label == "Delete VM")
        .collect();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].id, DELETE);
    let items: Vec<_> = h
        .context_menu(&one)
        .into_iter()
        .filter(|item| item.label == "Delete VM")
        .collect();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].id, vm_lifecycle::commands::DELETE);
    // Same label and menu place as the command for one.
    let registry = crate::command::registry();
    let spec = |id: CommandId| registry.iter().find(|spec| spec.id == id).expect("spec");
    for (several, single) in [
        (DELETE, vm_lifecycle::commands::DELETE),
        (TEMPLATES_DELETE, vm_lifecycle::commands::TEMPLATES_DELETE),
    ] {
        assert_eq!(spec(several).label, spec(single).label);
        assert_eq!(spec(several).menu, spec(single).menu);
        assert_eq!(spec(several).context, spec(single).context);
        assert!(spec(several).multi);
    }
    // VMs and templates do not mix, and a VM command is not for templates.
    assert_eq!(h.model.eligibility(TEMPLATES_DELETE, &two), Eligibility::Hidden);
    let several = templates(&mut h);
    assert_eq!(h.model.eligibility(DELETE, &several), Eligibility::Hidden);
    assert_ne!(
        h.model.eligibility(TEMPLATES_DELETE, &several),
        Eligibility::Hidden
    );
}

#[test]
fn deleting_several_is_refused_with_the_reason_of_the_selection_or_of_the_review() {
    let mut h = demo(64);
    // More than 32: never cut down silently.
    let many = guests(&h, 33);
    assert_eq!(many.len(), 33);
    assert_eq!(
        h.model.eligibility(DELETE, &many),
        Eligibility::disabled(delete::LIMIT)
    );
    // Several pools.
    let two = guests(&h, 2);
    let foreign = ObjectRef::vm(uuid::Uuid::new_v4().into(), two.vms()[1]);
    let mixed = Target::many([two.objects[0].clone(), foreign]);
    assert_eq!(
        h.model.eligibility(DELETE, &mixed),
        Eligibility::disabled(delete::ONE_POOL)
    );
    // A row that is gone.
    let gone = Target::many([
        two.objects[0].clone(),
        ObjectRef::vm(inventory(&h).pool_id, VmId::from(uuid::Uuid::new_v4())),
    ]);
    assert_eq!(
        h.model.eligibility(DELETE, &gone),
        Eligibility::disabled(delete::MISSING)
    );
    // Targets that cannot be deleted: the first line of the review's
    // refusal and how many more there are.
    let ids = two.vms();
    amend(&mut h, |i| {
        for id in &ids {
            i.vms.get_mut(id).unwrap().power_state = PowerState::Running;
        }
    });
    let editor = delete::capture(&h.model, &two).expect("captured");
    let refusal = delete::build_review(&h.model, &editor).err().unwrap();
    assert_eq!(refusal.lines().count(), 2);
    assert_eq!(
        h.model.eligibility(DELETE, &two),
        Eligibility::disabled(format!(
            "{} (and 1 more of the selection)",
            refusal.lines().next().unwrap()
        ))
    );
    assert!(h.try_invoke_on(DELETE.0, &two).is_err());
    assert!(h.model.areas.bulk_edit.delete.is_none());
    // Deletable: the batch opens on its draft, nothing chosen.
    amend(&mut h, |i| {
        for id in &ids {
            i.vms.get_mut(id).unwrap().power_state = PowerState::Halted;
            let d = i.details.vms.get_mut(id).unwrap();
            d.is_default_template = Some(false);
            d.allowed_operations = Some(vec!["destroy".into()]);
            d.current_operations.clear();
            d.blocked_operations.clear();
        }
    });
    assert_eq!(h.model.eligibility(DELETE, &two), Eligibility::Enabled);
    h.invoke_on(DELETE.0, &two);
    assert_eq!(h.surface().id, "batch-delete");
    assert!(del(&h).review.is_none());
    assert!(del(&h).choices.iter().all(|c| c.disks.is_empty()
        && c.snapshots.is_empty()
        && c.snapshot_disks.is_empty()));
    assert!(h.find("batch-delete-review").is_some());
    h.dismiss();
    assert!(h.model.areas.bulk_edit.delete.is_none());
}

#[test]
fn another_scope_closes_the_batch_deletion() {
    let mut h = Harness::scene("batch-delete");
    let pool = del(&h).pool;
    h.select(Target::one(ObjectRef::pool(pool)));
    assert!(h.model.areas.bulk_edit.delete.is_none());
}

#[test]
fn the_bulk_page_lives_with_its_dialog_and_refuses_a_selection_it_cannot_edit() {
    let mut h = Harness::scene("batch-properties");
    assert!(h.model.areas.bulk_edit.properties.is_some());
    h.dismiss();
    assert!(h.model.areas.bulk_edit.properties.is_none());
    assert!(h.model.properties.is_none());
    // More than 32: the page says why it has nothing to edit.
    let mut h = demo(64);
    let many = guests(&h, 33);
    h.invoke_on(properties::VM.0, &many);
    assert!(h.model.areas.bulk_edit.properties.is_none());
    assert_eq!(
        h.node("batch-properties-refused").text.as_deref(),
        Some(edit::LIMIT)
    );
    assert!(h.find("batch-properties-review").is_none());
    h.click("batch-properties-close");
    assert!(h.model.properties.is_none());
    // One VM never has the page.
    let one = Target::one(many.objects[0].clone());
    assert!(
        !h.model
            .property_pages(&one)
            .iter()
            .any(|(_, page)| page.id == PropertyPage::BULK.id)
    );
}

#[test]
fn the_draft_of_the_bulk_page_follows_the_property() {
    let (mut h, _) = two_open();
    assert_eq!(props(&h).property, Property::Description);
    assert_eq!(
        h.node("batch-property-value-0").label.as_deref(),
        Some("Common description")
    );
    assert!(h.has_text(Property::Description.note()));
    h.pick("batch-property", "Memory");
    assert_eq!(
        props(&h).values,
        ["512 MiB", "1 GiB", "2 GiB", "2 GiB"].map(str::to_owned)
    );
    assert!(h.find("batch-property-value-3").is_some());
    h.pick("batch-property", "Autostart");
    assert!(h.find("batch-property-value-0").is_none());
    h.click("batch-property-toggle-true");
    assert!(props(&h).toggle);
    assert_eq!(
        h.node("batch-property-toggle-false").label.as_deref(),
        Some("Disabled")
    );
    // The toggle and the host survive another property (Rune `choose`).
    h.pick("batch-property", "HomeHost");
    assert!(props(&h).toggle);
    let (host, _) = props(&h).hosts[0].clone();
    h.pick("batch-property-host", &host.to_string());
    assert_eq!(props(&h).host, Some(host));
    h.pick("batch-property-host", "none");
    assert_eq!(props(&h).host, None);
    h.assert_unique_keys();
}

/// Decision 10 of `AREAS.md`: the texts that name Rune places are kept
/// word for word until the one sweep at the end renames them.
#[test]
fn texts_that_name_rune_places_are_kept_for_the_sweep() {
    assert!(edit::SEQUENCE.contains("Operation history retains per-target steps"));
    assert!(Harness::scene("batch-properties").has_text(edit::SEQUENCE));
}

#[test]
fn a_live_session_observes_the_pool_the_batches_are_submitted_with() {
    let live = crate::testing::Live::with_vm();
    let one = Target::one(live.vm());
    let editor = delete::capture(&live.h.model, &one).expect("captured");
    assert!(editor.observed.is_some());
    assert!(delete::current(&live.h.model, &editor));
    let editor = edit::capture(&live.h.model, &one).expect("captured");
    assert!(editor.observed.is_some());
    assert!(edit::current(&live.h.model, &editor));
}
