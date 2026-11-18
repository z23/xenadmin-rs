//! The tests of the area, in three parts.
//!
//! 1. **The Rune tests**, the seven of `vm_properties.rs` under their Rune
//!    names. Three never built a window and moved as they were. Four drove
//!    the Rune dialog at two window sizes; here they drive the pages through
//!    the harness, which has no size (what a size can break is asked for in
//!    `notes/progress/gpui-tests-wanted.md`). Each asserts what the Rune
//!    test asserted; where the Rune dialog showed everything at once, the
//!    test turns to the page that has the control.
//! 2. **What the port changed**: pages, one draft across them, the review
//!    as a page, the entry on the Memory tab, the pool of the target.
//! 3. **The hazards** the map lists for this module (57 to 62).
//!
//! No fixture has a current snapshot, so no test submits successfully: the
//! refusals are tested, and that nothing reaches `m.operations`.

use std::sync::Arc;

use xenadmin_core::{Error, Inventory, PowerState, VmId, VmSettingEdit};

use super::change::{self, BUILT_IN, DEMO, NOT_CURRENT, NOT_WRITABLE, Page, STALE};
use super::commands::MEMORY_SETTINGS;
use super::draft::{Editor, parse_memory};
use super::scenes::{self, HALTED, RUNNING};
use super::view;
use crate::Harness;
use crate::prelude::*;

// -- Fixtures -------------------------------------------------------------------

const PAGES: &str = "vm-properties-pages";
const TEMPLATE_PAGES: &str = "template-properties-pages";
const UNCHANGED: &str = "No properties have changed";

/// The demo pool with a guest selected and nothing open.
fn guest(id: &str) -> Harness {
    let mut m = Model::fixture();
    scenes::guest(&mut m, id).expect("the demo guest");
    Harness::with(m)
}

/// The same with the guest's Properties dialog open on its first page.
fn opened(id: &str) -> Harness {
    let mut h = guest(id);
    h.invoke("vm.properties");
    assert_eq!(h.surface().id, "vm-properties");
    h
}

#[track_caller]
fn editor(h: &Harness) -> &Editor {
    h.model.areas.vm_properties.editor.as_ref().expect("a draft")
}

#[track_caller]
fn editor_mut(h: &mut Harness) -> &mut Editor {
    h.model.areas.vm_properties.editor.as_mut().expect("a draft")
}

/// The page the dialog is on.
#[track_caller]
fn page(h: &Harness) -> &'static str {
    h.model.properties.as_ref().expect("the dialog").page.id
}

fn vm(id: &str) -> VmId {
    VmId::parse(id).expect("a VM id")
}

fn object(h: &Harness, id: &str) -> ObjectRef {
    let pool = h.model.inventory.as_ref().expect("inventory").pool_id;
    ObjectRef::vm(pool, vm(id))
}

/// The inventory moves on: what a poll does when the host reports a change.
fn amend(m: &mut Model, change: impl FnOnce(&mut Inventory)) {
    let mut inventory = m.inventory.take().expect("inventory").as_ref().clone();
    change(&mut inventory);
    m.inventory = Some(Arc::new(inventory));
}

/// The setup of the two Rune `firmware_ui_tests`: the halted demo guest as a
/// UEFI guest, or as a custom BIOS template with or without its disks, its
/// Properties dialog open on Advanced.
fn firmware(template: bool, attached: bool) -> Harness {
    let mut m = Model::fixture();
    m.load_demo(3, 48);
    let id = vm(HALTED);
    amend(&mut m, |i| {
        let guest = i.vms.get_mut(&id).expect("guest");
        guest.power_state = PowerState::Halted;
        guest.is_template = template;
        let d = i.details.vms.get_mut(&id).expect("details");
        d.is_default_template = Some(false);
        d.firmware = Some(if template { "bios" } else { "uefi" }.into());
        if !attached {
            i.details.vbds.retain(|_, v| v.vm != Some(id));
        }
    });
    let pool = m.inventory.as_ref().expect("inventory").pool_id;
    let target = Target::one(if template {
        ObjectRef::template(pool, id)
    } else {
        ObjectRef::vm(pool, id)
    });
    m.select(target);
    let mut h = Harness::with(m);
    if template {
        h.invoke("template.properties");
        assert_eq!(h.surface().id, "template-properties");
        h.pick(TEMPLATE_PAGES, "advanced");
    } else {
        h.invoke("vm.properties");
        h.pick(PAGES, "advanced");
    }
    h
}

/// The Rune tests clicked `apply-properties` in the demo and asserted that
/// nothing happened. Here the button says why it cannot be pressed, and the
/// handler, called anyway, refuses too.
#[track_caller]
fn assert_apply_is_inert(h: &mut Harness) {
    assert_eq!(h.try_click("apply-properties"), Err(DEMO.into()));
    change::submit(&mut h.model);
    assert!(h.model.operations.is_empty());
    assert!(h.model.areas.vm_properties.editor.is_some());
    assert!(h.model.properties.is_some());
    assert_eq!(editor(h).error, Some(Error::Stale.to_string()));
}

// -- 1. The Rune tests ------------------------------------------------------------

#[test]
fn home_host_picker_reviews_original_identity_and_keeps_synthetic_apply_disabled() {
    let mut h = opened(HALTED);
    let previous = editor(&h).details.affinity;
    assert!(previous.is_some());
    let original = editor(&h).id;
    h.pick(PAGES, "home-server");
    h.click("property-home-host-none");
    assert!(editor(&h).home_host.is_none());
    h.click("review-properties");
    assert_eq!(page(&h), "review");
    assert_eq!(
        editor(&h).review.as_ref().expect("review").edits(),
        &[VmSettingEdit::HomeHost {
            previous,
            value: None
        }]
    );
    assert!(h.has_text("does not migrate"));
    // Another row chosen in a grid does not retarget the draft.
    let other = object(&h, RUNNING);
    h.model.select_in_grid(Target::one(other));
    assert_eq!(editor(&h).id, original);
    assert_apply_is_inert(&mut h);
}

#[test]
fn properties_review_is_reachable_and_demo_apply_is_disabled() {
    let mut h = opened(RUNNING);
    h.type_text("property-name", "Reviewed fixture name");
    h.click("review-properties");
    assert!(editor(&h).review.is_some());
    assert!(h.find("apply-properties").is_some());
    assert_apply_is_inert(&mut h);
}

#[test]
fn memory_input_is_exact_and_bounded() {
    assert_eq!(parse_memory("128 GiB").unwrap(), 128 << 30);
    assert_eq!(parse_memory("123456789").unwrap(), 123456789);
    assert!(parse_memory("18446744073709551615 GiB").is_err());
    assert!(parse_memory("1.5 MiB").is_err());
}

#[test]
fn property_draft_keeps_the_original_target_and_values() {
    let mut h = opened(HALTED);
    let other = object(&h, RUNNING);
    h.model.select_in_grid(Target::one(other));
    let editor = editor_mut(&mut h);
    editor.name = "Changed demo".into();
    let patch = editor.patch().unwrap();
    assert_eq!(editor.id.to_string(), HALTED);
    assert!(
        matches!(&patch.edits()[0], VmSettingEdit::Name { previous, value }
            if previous == &editor.original.name && value == "Changed demo")
    );
}

#[test]
fn running_guest_draft_uses_live_cpu_and_dynamic_memory_commands() {
    let mut h = guest(RUNNING);
    let id = vm(RUNNING);
    amend(&mut h.model, |i| {
        i.details.vms.get_mut(&id).unwrap().vcpus_max = Some(8);
    });
    h.invoke("vm.properties");
    let editor = editor_mut(&mut h);
    editor.startup_cpus = "4".into();
    editor.dynamic_min = "2 GiB".into();
    editor.cpu_weight = "512".into();
    let patch = editor.patch().unwrap();
    assert!(patch.edits().iter().any(|e| matches!(
        e,
        VmSettingEdit::LiveCpus {
            previous: 2,
            max: 8,
            value: 4
        }
    )));
    assert!(
        patch
            .edits()
            .iter()
            .any(|e| matches!(e, VmSettingEdit::DynamicMemory { min, .. } if *min == 2 << 30))
    );
    assert!(
        !patch
            .edits()
            .iter()
            .any(|e| matches!(e, VmSettingEdit::Cpus { .. } | VmSettingEdit::Memory { .. }))
    );
    editor.startup_cpus = "1".into();
    assert!(editor.patch().is_err());
}

#[test]
fn secure_boot_review_preserves_vm_identity_and_does_not_claim_enforcement() {
    let mut h = firmware(false, true);
    let id = editor(&h).id;
    h.click("property-secure-boot");
    h.click("review-properties");
    let p = editor(&h).review.as_ref().expect("review");
    assert!(!p.is_template());
    assert!(matches!(
        p.edits(),
        [VmSettingEdit::SecureBoot {
            previous: false,
            value: true
        }]
    ));
    let other = object(&h, RUNNING);
    h.model.select_in_grid(Target::one(other));
    assert_eq!(editor(&h).id, id);
    assert!(h.has_text("does not establish actual guest enforcement"));
    assert_apply_is_inert(&mut h);
}

#[test]
fn empty_custom_template_has_explicit_firmware_review_and_builtin_is_read_only() {
    let mut h = firmware(true, false);
    h.click("property-firmware-uefi");
    h.click("property-secure-boot");
    h.click("review-properties");
    let p = editor(&h).review.as_ref().expect("review");
    assert!(p.is_template());
    assert!(matches!(
        p.edits(),
        [
            VmSettingEdit::Firmware { .. },
            VmSettingEdit::SecureBoot { .. }
        ]
    ));
    // A template with disks attached has no firmware choice.
    let mut locked = firmware(true, true);
    assert!(locked.find("property-firmware").is_none());
    assert!(locked.find("property-firmware-uefi").is_none());
    // A built-in template cannot be reviewed.
    let e = editor_mut(&mut locked);
    e.details.is_default_template = Some(true);
    e.name = "Changed".into();
    assert_eq!(locked.try_click("review-properties"), Err(BUILT_IN.into()));
    change::review(&mut locked.model);
    assert!(editor(&locked).review.is_none());
    assert_eq!(editor(&locked).error.as_deref(), Some(BUILT_IN));
    assert_eq!(page(&locked), "advanced");
}

// -- 2. What the port changed -----------------------------------------------------

fn mine(h: &Harness, target: &Target) -> Vec<&'static str> {
    h.model
        .property_pages(target)
        .into_iter()
        .filter(|(area, _)| *area == "vm_properties")
        .map(|(_, page)| page.id)
        .collect()
}

#[test]
fn one_guest_or_template_has_the_seven_pages_and_several_have_none() {
    let h = guest(HALTED);
    let all = Page::ALL.map(Page::id).to_vec();
    assert_eq!(
        all,
        [
            "general",
            "cpu-memory",
            "boot-options",
            "start-options",
            "home-server",
            "advanced",
            "review"
        ]
    );
    let one = Target::one(object(&h, HALTED));
    assert_eq!(mine(&h, &one), all);
    let pages = view::pages(&h.model, &one);
    assert!(pages.iter().all(|page| page.experimental));
    assert_eq!(
        pages.iter().map(|page| page.label).collect::<Vec<_>>(),
        [
            "General",
            "CPU and Memory",
            "Boot Options",
            "Start Options",
            "Home Server",
            "Advanced",
            "Review Changes"
        ]
    );
    // Several guests are the bulk_edit area's.
    let several = Target::many([object(&h, HALTED), object(&h, RUNNING)]);
    assert!(mine(&h, &several).is_empty());
    // A guest the inventory does not have, and another kind of object.
    let unknown = Target::one(object(&h, "30000000-0000-0000-0000-00000000ffff"));
    assert!(mine(&h, &unknown).is_empty());
    let inventory = h.model.inventory.as_ref().unwrap();
    let host = *inventory.hosts.keys().next().unwrap();
    let server = Target::one(ObjectRef::host(inventory.pool_id, host));
    assert!(mine(&h, &server).is_empty());
    // A template has the same pages, in its own dialog.
    let t = firmware(true, true);
    let template = t.model.scope().clone();
    assert_eq!(mine(&t, &template), all);
}

#[test]
fn one_draft_spans_the_pages_and_is_reviewed_as_one_patch() {
    let mut h = opened(HALTED);
    let previous = editor(&h).details.affinity;
    let name = editor(&h).original.name.clone();
    assert_eq!(page(&h), "general");
    h.type_text("property-name", "Renamed");
    h.pick(PAGES, "start-options");
    h.click("property-autostart");
    h.pick(PAGES, "home-server");
    h.click("property-home-host-none");
    // What was typed two pages ago is still the draft.
    assert_eq!(editor(&h).name, "Renamed");
    assert!(editor(&h).autostart);
    // The review, chosen from the page list: one patch, in the Rune order.
    h.pick(PAGES, "review");
    assert_eq!(
        editor(&h).review.as_ref().expect("review").edits(),
        &[
            VmSettingEdit::HomeHost {
                previous,
                value: None
            },
            VmSettingEdit::Name {
                previous: name.clone(),
                value: "Renamed".into()
            },
            VmSettingEdit::Autostart {
                previous: false,
                value: true
            },
        ]
    );
    assert!(h.has_text(&format!("Name: {name} → Renamed")));
    assert!(h.has_text("Autostart: false → true"));
    assert!(h.has_text("Changes apply in steps."));
    assert!(h.has_text("Review VM property changes"));
    // No field is on the review: it cannot go stale against the draft.
    assert!(h.find("property-name").is_none());
    assert!(h.find("review-properties").is_none());
    // Back: to the page the review was asked from, the draft kept, the
    // review gone.
    h.click("properties-back");
    assert_eq!(page(&h), "home-server");
    assert!(editor(&h).review.is_none());
    assert_eq!(editor(&h).name, "Renamed");
    h.pick(PAGES, "general");
    assert_eq!(
        h.node("property-name").value.as_deref(),
        Some("Renamed"),
        "{}",
        h.dump()
    );
}

#[test]
fn the_draft_goes_with_the_dialog_and_a_new_dialog_starts_afresh() {
    let mut h = opened(HALTED);
    let name = editor(&h).original.name.clone();
    h.type_text("property-name", "Renamed");
    h.click("properties-cancel");
    assert!(h.model.properties.is_none());
    assert!(h.model.areas.vm_properties.editor.is_none());
    h.invoke("vm.properties");
    assert_eq!(editor(&h).name, name);
    h.type_text("property-name", "Renamed");
    h.dismiss();
    assert!(h.model.areas.vm_properties.editor.is_none());
    // Another guest selected in the tree closes the dialog and the draft.
    h.invoke("vm.properties");
    let other = object(&h, RUNNING);
    h.select(Target::one(other));
    assert!(h.model.properties.is_none());
    assert!(h.model.areas.vm_properties.editor.is_none());
    h.invoke("vm.properties");
    assert_eq!(editor(&h).id.to_string(), RUNNING);
}

#[test]
fn the_review_page_of_an_unchanged_draft_says_so_and_cannot_apply() {
    let mut h = opened(HALTED);
    h.pick(PAGES, "review");
    assert!(editor(&h).review.is_none());
    assert_eq!(h.node("properties-error").text.as_deref(), Some(UNCHANGED));
    assert!(h.find(view::REVIEW).is_none());
    assert_eq!(h.try_click("apply-properties"), Err(UNCHANGED.into()));
    change::submit(&mut h.model);
    assert!(h.model.operations.is_empty());
    // The button of a page says the same and stays on its page.
    h.click("properties-back");
    assert_eq!(page(&h), "general");
    h.click("review-properties");
    assert_eq!(page(&h), "general");
    assert_eq!(editor(&h).error.as_deref(), Some(UNCHANGED));
}

#[test]
fn a_draft_that_cannot_be_read_keeps_its_page_and_says_why() {
    let mut h = opened(HALTED);
    h.pick(PAGES, "cpu-memory");
    h.type_text("property-max-cpus", "many");
    h.click("review-properties");
    assert_eq!(page(&h), "cpu-memory");
    assert_eq!(
        h.node("properties-error").text.as_deref(),
        Some("Enter whole positive CPU counts")
    );
    h.type_text("property-max-cpus", "8");
    h.type_text("property-static-max", "1.5 GiB");
    h.click("review-properties");
    assert_eq!(
        editor(&h).error.as_deref(),
        Some("Memory must be a whole byte count, or a whole number with KiB, MiB or GiB")
    );
    h.type_text("property-static-max", "8 GiB");
    h.click("review-properties");
    assert_eq!(page(&h), "review");
    assert!(editor(&h).error.is_none());
    assert!(h.has_text("vCPUs: maximum"));
    assert!(h.has_text("Memory (bytes):"));
}

#[test]
fn the_memory_tab_leads_to_the_cpu_and_memory_page() {
    let mut h = guest(RUNNING);
    h.open_tab(TabId::Memory);
    let row = h.local_row();
    let item = row
        .iter()
        .find(|item| item.id == MEMORY_SETTINGS)
        .expect("the entry to memory settings");
    assert_eq!(item.label, "Memory Settings");
    assert!(item.disabled.is_none());
    h.click(MEMORY_SETTINGS.0);
    assert_eq!(h.surface().id, "vm-properties");
    assert_eq!(page(&h), "cpu-memory");
    assert!(h.find("property-live-cpus").is_some());
    assert!(h.find("property-dynamic-min").is_some());
    assert!(h.find("property-static-max").is_none());
    h.dismiss();
    // A server has a Memory tab too: the entry is not there.
    let inventory = h.model.inventory.clone().unwrap();
    let host = *inventory.hosts.keys().next().unwrap();
    h.select(Target::one(ObjectRef::host(inventory.pool_id, host)));
    h.open_tab(TabId::Memory);
    assert_eq!(h.eligibility(MEMORY_SETTINGS.0), Eligibility::Hidden);
    assert!(h.local_row().iter().all(|i| i.id != MEMORY_SETTINGS));
    // Several guests: not this command's.
    let several = Target::many([object(&h, HALTED), object(&h, RUNNING)]);
    assert_eq!(
        h.model.eligibility(MEMORY_SETTINGS, &several),
        Eligibility::Hidden
    );
}

#[test]
fn every_page_carries_the_experimental_notice_and_has_unique_keys() {
    for id in [HALTED, RUNNING] {
        let mut h = opened(id);
        for p in Page::ALL {
            h.pick(PAGES, p.id());
            assert_eq!(page(&h), p.id());
            assert!(h.surface().experimental, "{}", p.id());
            assert!(h.model.experimental_notice(), "{}", p.id());
            h.assert_unique_keys();
            assert!(h.find("properties-cancel").is_some(), "{}", p.id());
            assert!(h.find("properties-preview").is_some(), "{}", p.id());
        }
    }
}

#[test]
fn the_pages_hold_the_fields_of_the_captured_power_state() {
    let fields = |h: &mut Harness, page: &str| -> Vec<String> {
        h.pick(PAGES, page);
        h.keys()
            .into_iter()
            .filter(|key| key.starts_with("property-") && !key.contains("home-host-"))
            .collect()
    };
    let mut h = opened(HALTED);
    assert_eq!(
        fields(&mut h, "general"),
        ["property-name", "property-description"]
    );
    assert_eq!(
        fields(&mut h, "cpu-memory"),
        [
            "property-max-cpus",
            "property-startup-cpus",
            "property-cores-per-socket",
            "property-static-min",
            "property-dynamic-min",
            "property-dynamic-max",
            "property-static-max",
            "property-cpu-weight",
            "property-shadow-multiplier",
        ]
    );
    assert_eq!(fields(&mut h, "boot-options"), ["property-boot-order"]);
    assert_eq!(fields(&mut h, "start-options"), ["property-autostart"]);
    assert_eq!(fields(&mut h, "home-server"), ["property-home-host"]);
    assert_eq!(fields(&mut h, "advanced"), ["property-secure-boot"]);
    assert!(h.has_text("Firmware: uefi · Secure Boot policy: off"));

    let mut h = opened(RUNNING);
    assert_eq!(
        fields(&mut h, "cpu-memory"),
        [
            "property-live-cpus",
            "property-dynamic-min",
            "property-dynamic-max",
            "property-cpu-weight",
            "property-shadow-multiplier",
        ]
    );
    assert!(h.has_text("Live changes apply to a running guest."));
    // Boot order needs a halted guest: the page says so and has no field.
    assert!(fields(&mut h, "boot-options").is_empty());
    assert!(h.has_text("boot order require a halted VM"));
    // Secure Boot can be requested of a halted guest only.
    assert!(fields(&mut h, "advanced").is_empty());

    // A suspended guest: Rune showed no CPU, memory or boot field.
    let mut h = guest(RUNNING);
    amend(&mut h.model, |i| {
        i.vms.get_mut(&vm(RUNNING)).unwrap().power_state = PowerState::Suspended;
    });
    h.invoke("vm.properties");
    assert!(fields(&mut h, "cpu-memory").is_empty());
    assert_eq!(h.node("properties-nothing").text.as_deref(), Some(view::NOTHING));
    assert!(fields(&mut h, "boot-options").is_empty());
    assert!(h.find("properties-nothing").is_some());
}

#[test]
fn the_scenes_show_their_pages() {
    let h = Harness::scene("properties");
    assert_eq!(h.surface().id, "vm-properties");
    assert_eq!(page(&h), "general");
    assert_eq!(editor(&h).id.to_string(), HALTED);
    assert!(h.find("property-name").is_some());

    let h = Harness::scene("properties-running");
    assert_eq!(page(&h), "cpu-memory");
    assert_eq!(editor(&h).id.to_string(), RUNNING);
    assert!(h.find("property-live-cpus").is_some());

    let h = Harness::scene("home-host");
    assert_eq!(page(&h), "review");
    assert!(matches!(
        editor(&h).review.as_ref().expect("review").edits(),
        [VmSettingEdit::HomeHost { value: None, .. }]
    ));
    assert!(h.has_text("→ No preference."));
    assert!(h.find("apply-properties").is_some());

    let h = Harness::scene("secure-boot");
    assert_eq!(page(&h), "advanced");
    assert_eq!(
        editor(&h).original.power_state,
        PowerState::Halted,
        "the Rune scene made the guest halted"
    );
    assert!(h.find("property-secure-boot").is_some());
    assert!(h.node("refresh-boot-readiness").disabled.is_some());
    for scene in ["properties", "properties-running", "home-host", "secure-boot"] {
        assert!(Harness::scene(scene).model.experimental_notice(), "{scene}");
    }
}

#[test]
fn a_quiet_fixture_does_not_change_on_the_first_poll() {
    let mut h = opened(HALTED);
    assert!(!change::poll(&mut h.model, 0.0));
    assert!(!change::poll(&mut h.model, 1.0));
    // Without a draft there is nothing to poll.
    h.dismiss();
    assert!(!change::poll(&mut h.model, 2.0));
}

// -- 3. The hazards ---------------------------------------------------------------

/// Hazards 1 and 61: Apply needs the captured target to be what the pool
/// observes now, and the handler checks it as the button does. A live pool
/// in a fixture has no snapshot, so the captured target is `None`: stale.
#[test]
fn a_review_whose_target_is_not_observed_cannot_be_applied() {
    let mut h = opened(HALTED);
    h.type_text("property-name", "Renamed");
    h.click("review-properties");
    let patch = editor(&h).review.clone().expect("review");
    // The pool stops being the demo under the open review.
    h.model.demo = false;
    assert!(editor(&h).target.is_none());
    assert_eq!(h.try_click("apply-properties"), Err(STALE.into()));
    assert_eq!(
        h.node("properties-review-stale").text.as_deref(),
        Some(STALE),
        "{}",
        h.dump()
    );
    assert!(h.find("properties-preview").is_none());
    change::submit(&mut h.model);
    assert_eq!(editor(&h).error.as_deref(), Some(STALE));
    assert_eq!(editor(&h).review, Some(patch));
    assert!(h.model.operations.is_empty());
    assert!(h.model.properties.is_some());
}

/// The Rune button "Review changes" was disabled when the editor was not
/// current or the connection could not change anything; here it says why,
/// and the review page chosen from the list refuses the same way.
#[test]
fn a_draft_that_is_not_current_cannot_be_reviewed() {
    let mut h = guest(HALTED);
    h.model.demo = false;
    h.invoke("vm.properties");
    assert!(editor(&h).target.is_none());
    assert!(!change::current(&h.model, editor(&h)));
    assert_eq!(
        h.node("properties-not-current").text.as_deref(),
        Some(NOT_CURRENT)
    );
    h.type_text("property-name", "Renamed");
    assert_eq!(h.try_click("review-properties"), Err(NOT_CURRENT.into()));
    change::review(&mut h.model);
    assert_eq!(page(&h), "general");
    assert_eq!(editor(&h).error.as_deref(), Some(NOT_CURRENT));
    h.pick(PAGES, "review");
    assert!(editor(&h).review.is_none());
    assert_eq!(h.try_click("apply-properties"), Err(NOT_CURRENT.into()));
    // The readiness of the certificates cannot be asked for either.
    h.pick(PAGES, "advanced");
    assert_eq!(
        h.try_click("refresh-boot-readiness"),
        Err(NOT_CURRENT.into())
    );
    assert!(!h.has_text("Certificate readiness has not been queried."));
    change::refresh_boot(&mut h.model);
    assert!(editor(&h).boot_query.is_none());
    assert!(editor(&h).boot_error.is_none());
    // The words of the other refusal, which needs a current target and so a
    // live connection, are pinned here.
    assert_eq!(
        NOT_WRITABLE,
        "This connection is read-only, disconnected or stale."
    );
}

/// Hazard 57: which fields are edits follows the power state captured at
/// opening, and the order of the edits is fixed.
#[test]
fn edits_follow_the_captured_power_state_in_a_fixed_order() {
    let mut h = opened(HALTED);
    let e = editor_mut(&mut h);
    e.home_host = None;
    e.name = "N".into();
    e.description = "D".into();
    e.max_cpus = "8".into();
    e.startup_cpus = "4".into();
    e.static_max = "16 GiB".into();
    e.dynamic_max = "16 GiB".into();
    e.boot_order = "dc".into();
    e.topology = "2".into();
    e.cpu_weight = "512".into();
    e.shadow = "2".into();
    e.autostart = true;
    e.secure_boot = true;
    let kinds: Vec<&str> = e
        .patch()
        .unwrap()
        .edits()
        .iter()
        .map(|edit| match edit {
            VmSettingEdit::HomeHost { .. } => "home",
            VmSettingEdit::Name { .. } => "name",
            VmSettingEdit::Description { .. } => "description",
            VmSettingEdit::Cpus { .. } => "cpus",
            VmSettingEdit::Memory { .. } => "memory",
            VmSettingEdit::BootOrder { .. } => "boot",
            VmSettingEdit::LiveCpus { .. } => "live-cpus",
            VmSettingEdit::DynamicMemory { .. } => "dynamic-memory",
            VmSettingEdit::CpuTopology { .. } => "topology",
            VmSettingEdit::CpuWeight { .. } => "weight",
            VmSettingEdit::ShadowMultiplier { live, .. } => {
                assert!(!live);
                "shadow"
            }
            VmSettingEdit::Autostart { .. } => "autostart",
            VmSettingEdit::Firmware { .. } => "firmware",
            VmSettingEdit::SecureBoot { .. } => "secure-boot",
        })
        .collect();
    let expected = [
        "home",
        "name",
        "description",
        "cpus",
        "memory",
        "boot",
        "topology",
        "weight",
        "shadow",
        "autostart",
        "secure-boot",
    ];
    // The demo guest reports its topology or not: the order is what counts.
    let expected: Vec<&str> = expected
        .into_iter()
        .filter(|kind| *kind != "topology" || e.details.topology_known)
        .collect();
    assert_eq!(kinds, expected);
    // The inventory moving on does not change what the draft was captured
    // as: the guest starts, the draft still edits a halted guest.
    amend(&mut h.model, |i| {
        i.vms.get_mut(&vm(HALTED)).unwrap().power_state = PowerState::Running;
    });
    assert_eq!(editor(&h).original.power_state, PowerState::Halted);
    assert!(
        editor(&h)
            .patch()
            .unwrap()
            .edits()
            .iter()
            .any(|edit| matches!(edit, VmSettingEdit::Cpus { .. }))
    );
}

/// Hazard 58: a halted guest's CPU counts are trimmed, the live count of a
/// running guest is not. Hazard 59: the name is not trimmed by the editor.
#[test]
fn halted_counts_are_trimmed_and_the_live_count_is_not() {
    let mut h = opened(HALTED);
    let e = editor_mut(&mut h);
    e.max_cpus = " 8 ".into();
    e.startup_cpus = " 4".into();
    assert!(matches!(
        e.patch().unwrap().edits(),
        [VmSettingEdit::Cpus {
            max: 8,
            startup: 4,
            ..
        }]
    ));
    let mut h = opened(RUNNING);
    let e = editor_mut(&mut h);
    e.startup_cpus = " 2".into();
    assert_eq!(e.patch(), Err("Enter a whole live vCPU count".into()));
    e.startup_cpus = "2".into();
    e.name = format!("{} ", e.original.name);
    assert!(matches!(
        e.patch().unwrap().edits(),
        [VmSettingEdit::Name { value, .. }] if value.ends_with(' ')
    ));
}

/// Hazard 60: choosing BIOS clears a Secure Boot request, and the firmware
/// choice is what was computed at opening.
#[test]
fn choosing_bios_firmware_clears_the_secure_boot_request() {
    let mut h = firmware(true, false);
    assert_eq!(editor(&h).firmware.as_deref(), Some("bios"));
    assert!(h.find("property-secure-boot").is_none());
    h.click("property-firmware-uefi");
    h.click("property-secure-boot");
    assert!(editor(&h).secure_boot);
    h.click("property-firmware-bios");
    assert!(!editor(&h).secure_boot);
    assert!(h.find("property-secure-boot").is_none());
    assert_eq!(h.try_click("review-properties"), Ok(()));
    assert_eq!(page(&h), "advanced");
    assert_eq!(editor(&h).error.as_deref(), Some(UNCHANGED));
    assert!(h.has_text("Only empty custom templates can change BIOS/UEFI firmware."));
    assert!(h.has_text("Review changes") || h.find("review-properties").is_some());
}

/// Hazard 62: nothing asks for the readiness in a synthetic pool, and the
/// page says what the Rune dialog said.
#[test]
fn the_demo_has_no_boot_readiness_query() {
    let mut h = firmware(false, true);
    assert!(editor(&h).boot_query.is_none());
    assert!(h.has_text("Certificate readiness has not been queried."));
    assert!(h.has_text("Readiness is sampled at the last check"));
    assert!(h.node("refresh-boot-readiness").disabled.is_some());
    change::refresh_boot(&mut h.model);
    assert!(editor(&h).boot_query.is_none());
    assert!(editor(&h).boot_error.is_none());
}

/// The template caption of the Rune dialog, and the review heading of a
/// template.
#[test]
fn a_template_says_what_its_properties_affect() {
    let mut h = firmware(true, false);
    h.pick(TEMPLATE_PAGES, "general");
    assert!(h.has_text("Custom template properties affect future VMs"));
    h.type_text("property-name", "Renamed template");
    h.click("review-properties");
    assert!(h.has_text("Review template property changes"));
    assert!(editor(&h).review.as_ref().unwrap().is_template());
}
