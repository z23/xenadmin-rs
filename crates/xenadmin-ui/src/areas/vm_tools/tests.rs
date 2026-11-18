//! The tests of the area, in three parts.
//!
//! 1. **The Rune tests**: the three of `guest_tools_view.rs` and the four
//!    of `external_tools_tests.rs`, under names that say which they were
//!    (`rune_` + the Rune name, without its "at both sizes"). Each asserts
//!    what the Rune test asserted; where the Rune test pressed a disabled
//!    button and saw nothing happen, the test here reads the reason of the
//!    disabled button and then calls the handler, which must refuse too.
//!    The tests of `external_tool_command.rs` moved with `command.rs`.
//! 2. **What the port changed**: commands and their eligibility, the two
//!    surfaces of the area in front of each other, closing on a scope
//!    change, the texts that name Rune places (decision 10).
//! 3. **The hazards** the maps list for the two modules.
//!
//! Not tested here: a mount that is submitted and a tool that is launched.
//! Both need a current connection with a running guest that has a CD
//! drive; the fixture server serves a halted VM without devices and no
//! `VBD.insert`, and a launch would start a local process.

use std::sync::{Arc, mpsc};

use xenadmin_core::{
    DeviceOperation, GuestMetrics, GuestMetricsId, Inventory, OperationAction, OperationId,
    OperationRecord, OperationState, PoolId, PowerState, VdiId, VmId,
};

use super::command::{Command, Kind};
use super::commands::{CONNECT_EXTERNAL, EXTERNAL_TOOLS, INSTALL_GUEST_TOOLS, NOT_RUNNING};
use super::external::{self, Job, Launch, Panel, Preparation, Reviewed};
use super::{media, scenes};
use crate::Harness;
use crate::prelude::*;
use crate::testing::poll_until;

// -- Fixtures -------------------------------------------------------------------

fn inventory(h: &Harness) -> Arc<Inventory> {
    h.model.inventory.clone().expect("inventory")
}

fn pool(h: &Harness) -> PoolId {
    inventory(h).pool_id
}

/// The inventory moves on: what a poll does when the host reports a change.
fn amend(h: &mut Harness, change: impl FnOnce(&mut Inventory)) {
    let mut inventory = inventory(h).as_ref().clone();
    change(&mut inventory);
    h.model.inventory = Some(Arc::new(inventory));
}

#[track_caller]
fn panel(h: &Harness) -> &media::Panel {
    h.model.areas.vm_tools.media.as_ref().expect("guest tools")
}

fn panel_mut(h: &mut Harness) -> &mut media::Panel {
    h.model.areas.vm_tools.media.as_mut().expect("guest tools")
}

#[track_caller]
fn launch(h: &Harness) -> &Launch {
    external::launch_ref(&h.model).expect("the launch dialog")
}

fn launch_mut(h: &mut Harness) -> &mut Launch {
    external::launch_mut(&mut h.model).expect("the launch dialog")
}

fn status(h: &Harness) -> Option<String> {
    h.find("guest-tool-status").and_then(|node| node.text)
}

/// The first tools image of the scene's inventory.
fn tools_image(h: &Harness) -> VdiId {
    *inventory(h)
        .details
        .tools_media
        .keys()
        .next()
        .expect("tools media")
}

/// The VM of the guest tools scenes, as a target.
fn media_vm(h: &Harness) -> Target {
    let vm = VmId::parse(scenes::MEDIA_VM).expect("id");
    Target::one(ObjectRef::vm(pool(h), vm))
}

/// Another VM than the one a dialog captured.
fn other_vm(h: &Harness, not: VmId) -> Target {
    let i = inventory(h);
    let vm = i.guests().find(|v| v.id != not).expect("another VM").id;
    Target::one(ObjectRef::vm(i.pool_id, vm))
}

/// The running guest of the `guest-connections` scene, with nothing open.
fn guest() -> (Harness, Target) {
    let mut h = Harness::scene("guest-connections");
    let target = Target::one(ObjectRef::vm(pool(&h), launch(&h).guest.vm));
    h.dismiss();
    assert!(h.surfaces().is_empty());
    (h, target)
}

/// The guest's tools report an address.
fn report(h: &mut Harness, vm: VmId, ip: &str) {
    let id = GuestMetricsId::from(uuid::Uuid::from_u128(0x7700));
    amend(h, |i| {
        i.details.vms.entry(vm).or_default().guest_metrics = Some(id);
        i.details.guest_metrics.insert(
            id,
            GuestMetrics {
                id,
                os_version: Default::default(),
                tools_version: Default::default(),
                networks: std::collections::BTreeMap::from([("0/ip".into(), ip.to_owned())]),
                live: Some(true),
                last_updated: None,
            },
        );
    });
}

fn wait_for_review(h: &mut Harness) {
    poll_until(h, "the command review arrived", |h| {
        launch(h).pending.is_none()
    });
}

fn mount_record(h: &Harness, id: OperationId, state: OperationState) -> OperationRecord {
    let p = panel(h);
    serde_json::from_value(serde_json::json!({
        "id": id, "pool": p.pool(), "vm": p.vm,
        "action": OperationAction::Device(p.review.clone().expect("review")),
        "state": state, "progress": null, "detail": "Verified",
    }))
    .expect("operation record")
}

// -- 1. The Rune tests ----------------------------------------------------------------

/// `guest_tools_view.rs`
/// `tools_review_captures_explicit_media_and_identity_and_fixed_footer_at_both_sizes`.
/// The fixed footer at both sizes is the renderer's (gpui-tests-wanted).
#[test]
fn rune_tools_review_captures_explicit_media_and_identity() {
    let mut h = Harness::scene("guest-tools");
    assert!(panel(&h).image.is_none());
    assert!(panel(&h).cd.is_some(), "the first CD drive is preselected");
    let image = tools_image(&h);
    assert_eq!(
        h.try_click("tools-review"),
        Err("Select guest tools media".into())
    );
    h.click(&format!("tools-image-{image}"));
    h.click("tools-review");
    let original = panel(&h).vm;
    let review = panel(&h).review.clone().expect("review");
    assert!(matches!(
        review,
        DeviceOperation::MountGuestTools { image: id, previous: None, .. } if id == image
    ));
    let other = other_vm(&h, original);
    h.model.select_in_grid(other);
    assert_eq!(panel(&h).vm, original);
    assert!(h.find("tools-confirm").is_some());
    assert_eq!(h.try_click("tools-confirm"), Err(media::DEMO.into()));
    media::apply(&mut h.model);
    assert!(h.model.operations.is_empty());
    assert_eq!(panel(&h).review.as_ref(), Some(&review));
    assert!(panel(&h).submitted.is_none());
    h.click("tools-close");
    assert!(h.model.areas.vm_tools.media.is_none());
}

/// `guest_tools_view.rs`
/// `tools_review_explains_existing_media_and_missing_devices_without_issuing_commands`.
#[test]
fn rune_tools_review_explains_existing_media_and_missing_devices_without_issuing_commands() {
    let mut h = Harness::scene("guest-tools");
    let image = tools_image(&h);
    let p = panel_mut(&mut h);
    let cd = p.cd.expect("drive");
    let old = *p
        .inventory
        .details
        .vdis
        .keys()
        .find(|id| !p.inventory.details.tools_media.contains_key(id))
        .expect("another image");
    let i = Arc::make_mut(&mut p.inventory);
    let drive = i.details.vbds.get_mut(&cd).expect("drive");
    drive.empty = Some(false);
    drive.vdi = Some(old);
    p.image = Some(image);
    media::review(&mut h.model);
    assert!(matches!(
        panel(&h).review,
        Some(DeviceOperation::MountGuestTools { previous: Some(id), .. }) if id == old
    ));
    assert!(h.has_text("Eject"));
    assert!(h.has_text("remain empty"));
    assert!(h.try_click("tools-confirm").is_err());
    media::apply(&mut h.model);
    assert!(h.model.operations.is_empty());

    let mut h = Harness::scene("guest-tools");
    let p = panel_mut(&mut h);
    let i = Arc::make_mut(&mut p.inventory);
    i.details.vm_disks.clear();
    i.details.tools_media.clear();
    p.cd = None;
    assert!(h.has_text("No CD drive"));
    assert!(h.has_text("No guest tools ISO"));
    assert_eq!(h.node("tools-no-cd").text.as_deref(), Some(media::NO_CD));
    assert_eq!(
        h.node("tools-no-media").text.as_deref(),
        Some(media::NO_MEDIA)
    );
    assert_eq!(
        h.try_click("tools-review"),
        Err("Select a CD drive".into())
    );
}

/// `guest_tools_view.rs` `tools_panel_refuses_device_view_from_another_pool`,
/// restated: there is no devices page; the command does not apply to a VM
/// whose pool's inventory is not the one that has it.
#[test]
fn rune_tools_panel_refuses_device_view_from_another_pool() {
    let mut h = Harness::scene("guest-tools");
    h.dismiss();
    let vm = VmId::parse(scenes::MEDIA_VM).expect("id");
    let foreign = ObjectRef::vm(PoolId::from(uuid::Uuid::from_u128(999)), vm);
    let target = Target::one(foreign.clone());
    assert_eq!(
        h.model.eligibility(INSTALL_GUEST_TOOLS, &target),
        Eligibility::Hidden
    );
    assert!(h.try_invoke_on(INSTALL_GUEST_TOOLS.0, &target).is_err());
    media::open(&mut h.model, &foreign);
    assert!(h.model.areas.vm_tools.media.is_none());
}

/// `external_tools_tests.rs`
/// `reviewed_guest_command_keeps_target_and_rejects_changed_input_at_both_sizes`.
#[test]
fn rune_reviewed_guest_command_keeps_target_and_rejects_changed_input() {
    let mut h = Harness::scene("guest-connections");
    h.model.preferences.external_tools.tools[0].executable = "/usr/bin/true".into();
    let target = launch(&h).guest.vm;
    h.click("review-guest-tool");
    wait_for_review(&mut h);
    assert!(launch(&h).review.is_some(), "{:?}", launch(&h).error);
    assert!(h.find("guest-tool-command").is_some());
    assert!(h.has_text("/usr/bin/true"));
    let other = other_vm(&h, target);
    h.model.select_in_grid(other);
    assert_eq!(launch(&h).guest.vm, target);
    assert_eq!(
        h.try_click("launch-guest-tool"),
        Err(external::NO_LAUNCH_IN_DEMO.into())
    );
    external::launch(&mut h.model);
    assert!(
        h.model.areas.vm_tools.external.jobs.is_empty(),
        "synthetic scenes never launch"
    );
    assert_eq!(status(&h).as_deref(), Some(external::NO_LAUNCH_IN_DEMO));
    // Rune: the edit, then a poll. Here the edit itself drops the review.
    h.type_text("guest-tool-address", "192.0.2.24");
    assert!(launch(&h).review.is_none());
    assert_eq!(status(&h).as_deref(), Some(external::REVIEW_DROPPED));
    h.click("configure-guest-tools");
    assert!(matches!(
        h.model.areas.vm_tools.external.panel,
        Some(Panel::Configure(_))
    ));
    assert_eq!(h.surface().id, external::CONFIGURE);
    h.click("close-guest-tools");
    assert_eq!(launch(&h).guest.vm, target);
    assert_eq!(h.surface().id, external::LAUNCH);
    h.click("close-guest-tools");
    assert!(!external::is_open(&h.model));
}

/// `external_tools_tests.rs`
/// `local_tool_configuration_saves_separate_arguments_and_can_remove_every_tool`.
/// As in Rune the test stands in for the preferences file.
#[test]
fn rune_local_tool_configuration_saves_separate_arguments_and_can_remove_every_tool() {
    let mut h = Harness::scene("external-tools");
    h.model.areas.vm_tools.external.storage = Some(Ok(()));
    h.type_text("guest-tool-executable", "/usr/bin/true");
    h.type_text(
        "guest-tool-arguments",
        r#"["literal with spaces", "{vm_uuid}"]"#,
    );
    h.click("save-guest-tools");
    assert_eq!(
        h.model.preferences.external_tools.tools[0].arguments,
        vec!["literal with spaces", "{vm_uuid}"]
    );
    assert_eq!(status(&h).as_deref(), Some(external::SAVED));
    h.type_text("guest-tool-arguments", "[\"{session}\"]");
    h.click("save-guest-tools");
    assert!(status(&h).expect("an error").contains("placeholders"));
    h.click("remove-guest-tool");
    // "Remove tool" changes the draft only.
    assert_eq!(h.model.preferences.external_tools.tools.len(), 2);
    h.click("remove-guest-tool");
    assert!(h.find("remove-guest-tool").is_none());
    h.click("save-guest-tools");
    assert!(h.model.preferences.external_tools.tools.is_empty());
    h.click("close-guest-tools");
    assert!(!external::is_open(&h.model));
}

/// `external_tools_tests.rs`
/// `captured_guest_checks_pool_generation_power_and_current_reported_ip`.
#[test]
fn rune_captured_guest_checks_pool_generation_power_and_current_reported_ip() {
    let h = Harness::scene("guest-connections");
    let target = launch(&h).guest.clone();
    let mut inventory = inventory(&h).as_ref().clone();
    let ip = "192.0.2.23".parse().unwrap();
    let generation = target.generation;
    assert!(
        target
            .observed(true, generation, Some(&inventory), ip, false)
            .is_ok()
    );
    assert_eq!(
        target.observed(false, generation, Some(&inventory), ip, false),
        Err(external::STALE_INVENTORY.into())
    );
    assert!(
        target
            .observed(true, generation + 1, Some(&inventory), ip, false)
            .is_err()
    );
    let original = inventory.pool_id;
    inventory.pool_id = PoolId::from(uuid::Uuid::new_v4());
    assert!(
        target
            .observed(true, generation, Some(&inventory), ip, false)
            .is_err()
    );
    inventory.pool_id = original;
    let id = GuestMetricsId::from(uuid::Uuid::new_v4());
    inventory
        .details
        .vms
        .entry(target.vm)
        .or_default()
        .guest_metrics = Some(id);
    inventory.details.guest_metrics.insert(
        id,
        GuestMetrics {
            id,
            os_version: Default::default(),
            tools_version: Default::default(),
            networks: std::collections::BTreeMap::from([("0/ip".into(), ip.to_string())]),
            live: Some(true),
            last_updated: None,
        },
    );
    assert!(
        target
            .observed(true, generation, Some(&inventory), ip, true)
            .is_ok()
    );
    inventory.details.guest_metrics.get_mut(&id).unwrap().live = Some(false);
    assert!(
        target
            .observed(true, generation, Some(&inventory), ip, true)
            .is_err()
    );
    inventory.details.guest_metrics.get_mut(&id).unwrap().live = Some(true);
    inventory
        .details
        .guest_metrics
        .get_mut(&id)
        .unwrap()
        .networks
        .clear();
    assert!(
        target
            .observed(true, generation, Some(&inventory), ip, true)
            .is_err()
    );
    inventory.vms.get_mut(&target.vm).unwrap().power_state = PowerState::Halted;
    assert!(
        target
            .observed(true, generation, Some(&inventory), ip, false)
            .is_err()
    );
}

/// `external_tools_tests.rs`
/// `delayed_command_review_cannot_apply_after_edit_or_close`.
#[test]
fn rune_delayed_command_review_cannot_apply_after_edit_or_close() {
    let mut h = Harness::scene("guest-connections");
    h.model.preferences.external_tools.tools[0].executable = "/usr/bin/true".into();
    let captured = external::inputs(&h.model, launch(&h)).unwrap();
    let command = Command::prepare(&captured.tool, &captured.context).unwrap();
    let (sender, receiver) = mpsc::channel();
    launch_mut(&mut h).pending = Some(Preparation {
        inputs: captured.clone(),
        receiver,
    });
    launch_mut(&mut h).address = "192.0.2.254".into();
    sender
        .send(Ok(Reviewed {
            inputs: captured.clone(),
            command: command.clone(),
        }))
        .unwrap();
    assert!(external::poll(&mut h.model, 0.0));
    assert!(launch(&h).review.is_none());
    assert!(launch(&h).pending.is_none());
    assert!(
        launch(&h)
            .error
            .as_deref()
            .unwrap()
            .contains("Inputs changed")
    );
    let (sender, receiver) = mpsc::channel();
    launch_mut(&mut h).pending = Some(Preparation {
        inputs: captured.clone(),
        receiver,
    });
    external::back(&mut h.model);
    assert!(
        sender
            .send(Ok(Reviewed {
                inputs: captured,
                command
            }))
            .is_err()
    );
    assert!(!external::is_open(&h.model));
    assert!(h.model.areas.vm_tools.external.jobs.is_empty());
}

// -- 2. What the port changed -------------------------------------------------------

/// Every scene shows its dialog, with unique keys, a quiet first poll and
/// the experimental notice: both Rune modules were flagged in
/// `experimental_dialog_open` (this area's share of `main.rs`
/// `dialogs_outside_the_supported_core_carry_the_experimental_notice`,
/// which named none of these scenes).
#[test]
fn every_scene_describes_its_experimental_dialog_with_unique_keys_and_a_quiet_first_poll() {
    let surfaces = [
        ("guest-connections", external::LAUNCH),
        ("external-tools", external::CONFIGURE),
        ("guest-tools", media::SURFACE),
        ("guest-tools-review", media::SURFACE),
    ];
    assert_eq!(scenes::scenes().len(), surfaces.len());
    for (scene, surface) in surfaces {
        let mut h = Harness::scene(scene);
        h.assert_unique_keys();
        assert_eq!(h.surface_id().as_deref(), Some(surface), "{scene}");
        assert!(h.surface().experimental, "{scene}");
        assert!(h.find(EXPERIMENTAL_NOTICE_KEY).is_some(), "{scene}");
        assert!(!h.tick(0.1), "{scene}: nothing changed");
        assert_eq!(h.surface_id().as_deref(), Some(surface), "{scene}");
    }
}

/// The permission line belongs to the review of the mount (the scene was
/// not in the list of `session_permissions_view.rs`
/// `review_permission_notice_is_reachable_without_bypassing_synthetic_target_guard`;
/// the same is asserted of it here).
#[test]
fn the_mount_review_explains_permission_and_the_demo_cannot_confirm() {
    let h = Harness::scene("guest-tools");
    assert!(h.find(permissions::REVIEW_KEY).is_none());
    let mut h = Harness::scene("guest-tools-review");
    assert!(h.find(permissions::REVIEW_KEY).is_some());
    assert!(h.find(media::REVIEW).is_some());
    assert!(h.has_text("Mount the selected tools ISO in the reviewed empty CD drive."));
    assert!(h.has_text("guest-tools.iso"));
    assert_eq!(h.try_click("tools-confirm"), Err(media::DEMO.into()));
    media::apply(&mut h.model);
    assert!(h.model.operations.is_empty());
    assert_eq!(panel(&h).error.as_deref(), Some(media::DEMO));
    h.click("tools-back");
    assert!(panel(&h).review.is_none());
    assert!(h.find("tools-review").is_some());
}

#[test]
fn install_guest_tools_is_in_the_vm_menu_and_says_what_is_missing() {
    let mut h = Harness::scene("guest-tools");
    h.dismiss();
    let item = h.menu_item(INSTALL_GUEST_TOOLS.0).expect("the menu item");
    assert_eq!(item.label, "Install Guest Tools");
    assert_eq!(item.disabled, None);
    let vm = media_vm(&h);
    assert!(
        h.context_menu(&vm)
            .iter()
            .any(|item| item.id == INSTALL_GUEST_TOOLS)
    );
    // No tools media in the pool, then no CD drive either: the texts of the
    // dialog's empty states.
    amend(&mut h, |i| i.details.tools_media.clear());
    assert_eq!(
        h.model.eligibility(INSTALL_GUEST_TOOLS, &vm),
        Eligibility::disabled(media::NO_MEDIA)
    );
    amend(&mut h, |i| i.details.vm_disks.clear());
    assert_eq!(
        h.model.eligibility(INSTALL_GUEST_TOOLS, &vm),
        Eligibility::disabled(media::NO_CD)
    );
    assert!(media::NO_CD.contains("Add CD/DVD Drive on the Storage tab of the VM"));
    assert!(h.try_invoke_on(INSTALL_GUEST_TOOLS.0, &vm).is_err());
    assert!(h.model.areas.vm_tools.media.is_none());
    // Not a VM: the command does not apply.
    let i = inventory(&h);
    let host = Target::one(ObjectRef::host(i.pool_id, *i.hosts.keys().next().unwrap()));
    let template = Target::one(ObjectRef::template(
        i.pool_id,
        VmId::parse(scenes::MEDIA_VM).unwrap(),
    ));
    for target in [host, template, Target::one(ObjectRef::pool(i.pool_id))] {
        assert_eq!(
            h.model.eligibility(INSTALL_GUEST_TOOLS, &target),
            Eligibility::Hidden
        );
    }
}

/// The Rune button of the devices page: on a VM's Storage tab, where the
/// grid selection is a disk, the command acts on the VM of the scope.
#[test]
fn install_guest_tools_on_a_vms_storage_tab_acts_on_the_scope_vm() {
    let mut h = Harness::scene("guest-tools");
    h.dismiss();
    h.open_tab(TabId::Storage);
    assert!(
        h.local_row()
            .iter()
            .any(|item| item.id == INSTALL_GUEST_TOOLS && item.disabled.is_none())
    );
    let disk = ObjectRef::disk(pool(&h), uuid::Uuid::from_u128(0x7600_0000 << 96 | 2));
    assert_eq!(disk.kind, ObjectKind::VirtualDisk);
    h.model.select_in_grid(Target::one(disk));
    h.invoke(INSTALL_GUEST_TOOLS.0);
    assert_eq!(panel(&h).vm, VmId::parse(scenes::MEDIA_VM).unwrap());
    // A server's Storage tab does not have it.
    h.dismiss();
    let i = inventory(&h);
    h.select(Target::one(ObjectRef::host(
        i.pool_id,
        *i.hosts.keys().next().unwrap(),
    )));
    h.open_tab(TabId::Storage);
    assert!(
        h.local_row()
            .iter()
            .all(|item| item.id != INSTALL_GUEST_TOOLS)
    );
}

#[test]
fn connect_external_needs_a_running_guest_and_is_on_the_console_tab() {
    let (mut h, running) = guest();
    assert_eq!(
        h.model.eligibility(CONNECT_EXTERNAL, &running),
        Eligibility::Enabled
    );
    let item = h
        .context_menu(&running)
        .into_iter()
        .find(|item| item.id == CONNECT_EXTERNAL)
        .expect("the context item");
    assert_eq!(item.label, "Connect with SSH, RDP or Local Tool…");
    assert!(
        h.menu_item(CONNECT_EXTERNAL.0).is_none(),
        "no menu bar row"
    );
    h.open_tab(TabId::Console);
    assert!(h.local_row().iter().any(|item| item.id == CONNECT_EXTERNAL));
    let i = inventory(&h);
    let halted = i
        .guests()
        .find(|v| v.power_state == PowerState::Halted)
        .expect("a halted VM")
        .id;
    let halted = Target::one(ObjectRef::vm(i.pool_id, halted));
    assert_eq!(
        h.model.eligibility(CONNECT_EXTERNAL, &halted),
        Eligibility::disabled(NOT_RUNNING)
    );
    external::open(&mut h.model, halted.single().unwrap());
    assert!(!external::is_open(&h.model));
    let host = Target::one(ObjectRef::host(i.pool_id, *i.hosts.keys().next().unwrap()));
    assert_eq!(
        h.model.eligibility(CONNECT_EXTERNAL, &host),
        Eligibility::Hidden
    );
    // A guest whose VM became a control domain is no guest.
    let vm = running.single().unwrap().vm_id().unwrap();
    amend(&mut h, |i| {
        i.vms.get_mut(&vm).unwrap().is_control_domain = true
    });
    assert_eq!(
        h.model.eligibility(CONNECT_EXTERNAL, &running),
        Eligibility::Hidden
    );
}

/// Rune: with no tool declared, opening the launcher opens the
/// declarations, and Close then closes (there is no launch to return to).
#[test]
fn connecting_without_a_declared_tool_opens_the_declarations() {
    let (mut h, running) = guest();
    h.model.preferences.external_tools.tools.clear();
    h.invoke_on(CONNECT_EXTERNAL.0, &running);
    assert_eq!(h.surface().id, external::CONFIGURE);
    assert!(h.find("guest-tool-edit").is_none());
    assert!(h.find("guest-tool-name").is_none());
    h.click("add-guest-tool-custom");
    assert!(h.find("guest-tool-name").is_some());
    h.click("close-guest-tools");
    assert!(h.surfaces().is_empty());
}

/// Tools > External Tools needs no object; Options hands over to it and
/// closes, as the Rune "Configure external tools…" did.
#[test]
fn external_tools_opens_from_the_tools_menu_and_from_options() {
    let mut h = Harness::new();
    let item = h.menu_item(EXTERNAL_TOOLS.0).expect("the menu item");
    assert_eq!(item.label, "External Tools");
    assert_eq!(item.disabled, None);
    h.invoke(EXTERNAL_TOOLS.0);
    assert_eq!(h.surface().id, external::CONFIGURE);
    assert_eq!(h.surface().title, "Configure external tools");
    assert_eq!(
        h.node("guest-tool-name").value.as_deref(),
        Some("SSH in default terminal")
    );
    h.dismiss();
    assert!(h.surfaces().is_empty());

    let mut h = Harness::scene("demo");
    h.invoke("tools.options");
    h.click("configure-external-tools");
    assert_eq!(h.surfaces().len(), 1, "Options closed");
    assert_eq!(h.surface().id, external::CONFIGURE);
}

/// The external tools dialog is in front of the guest tools dialog.
#[test]
fn the_declarations_open_in_front_of_the_guest_tools_dialog() {
    let mut h = Harness::scene("guest-tools");
    external::configure(&mut h.model);
    assert_eq!(h.surfaces().len(), 2);
    assert_eq!(h.surface().id, external::CONFIGURE);
    h.assert_unique_keys();
    h.dismiss();
    assert_eq!(h.surface().id, media::SURFACE);
}

/// Another object in the tree: the dialogs that captured an object close;
/// the declarations stay and lose the launch dialog they would return to.
#[test]
fn a_scope_change_closes_what_belongs_to_the_object_and_keeps_the_declarations() {
    let mut h = Harness::scene("guest-tools-review");
    let pool = Target::one(ObjectRef::pool(pool(&h)));
    h.select(pool.clone());
    assert!(h.model.areas.vm_tools.media.is_none());

    let mut h = Harness::scene("guest-connections");
    h.select(pool.clone());
    assert!(!external::is_open(&h.model));

    let mut h = Harness::scene("guest-connections");
    h.click("configure-guest-tools");
    h.type_text("guest-tool-name", "My SSH");
    h.select(pool);
    assert_eq!(h.surface().id, external::CONFIGURE);
    assert_eq!(h.node("guest-tool-name").value.as_deref(), Some("My SSH"));
    h.click("close-guest-tools");
    assert!(h.surfaces().is_empty());
}

/// Decision 10: the texts that named Rune places name this client's.
#[test]
fn texts_name_the_places_of_this_client() {
    assert_eq!(
        media::ADMITTED,
        "Operation admitted; open the Events tab of the Notifications view to inspect its progress."
    );
    assert_eq!(
        media::NO_CD,
        "No CD drive is available. Shut down the VM and add an empty CD drive with Add CD/DVD \
         Drive on the Storage tab of the VM."
    );
    assert_eq!(
        external::CONNECTION_CHANGED,
        "Connection changed. Close and reopen this dialog."
    );
    assert_eq!(
        external::STALE_INVENTORY,
        "Connection changed or inventory is stale. Close and reopen this dialog."
    );
}

// -- 3. The hazards --------------------------------------------------------------------

/// Guest tools 80: the operation is built from the CAPTURED inventory, its
/// availability is judged against the LIVE one.
#[test]
fn the_mount_is_built_from_the_captured_inventory_and_judged_against_the_live_one() {
    let mut h = Harness::scene("guest-tools");
    let image = tools_image(&h);
    let cd = panel(&h).cd.expect("drive");
    amend(&mut h, |i| {
        i.details.tools_media.clear();
        i.details.vbds.remove(&cd);
    });
    h.click(&format!("tools-image-{image}"));
    h.click("tools-review");
    let op = panel(&h).review.clone().expect("built from what was captured");
    let live = inventory(&h);
    let availability = op.availability(&live, panel(&h).vm);
    assert!(!availability.is_available());
    assert_eq!(
        media::unavailable(&h.model, panel(&h)),
        Some(availability.explanation())
    );
    assert!(h.has_text(&availability.explanation()));
    assert_eq!(
        h.try_click("tools-confirm"),
        Err(availability.explanation())
    );
    media::apply(&mut h.model);
    assert!(h.model.operations.is_empty());
    assert_eq!(
        panel(&h).error.as_deref(),
        Some(availability.explanation().as_str())
    );
}

/// Guest tools 80, the refusals of `capture`: an unknown media state
/// cannot be reviewed.
#[test]
fn a_drive_whose_media_state_is_unknown_cannot_be_reviewed() {
    let mut h = Harness::scene("guest-tools");
    let image = tools_image(&h);
    let p = panel_mut(&mut h);
    let cd = p.cd.expect("drive");
    let i = Arc::make_mut(&mut p.inventory);
    i.details.vbds.get_mut(&cd).unwrap().empty = None;
    assert!(h.has_text("Media state unavailable"));
    h.click(&format!("tools-image-{image}"));
    h.click("tools-review");
    assert!(panel(&h).review.is_none());
    assert_eq!(
        h.node("tools-error").text.as_deref(),
        Some("Mounted image state is unknown")
    );
    let p = panel_mut(&mut h);
    let i = Arc::make_mut(&mut p.inventory);
    let drive = i.details.vbds.get_mut(&cd).unwrap();
    drive.empty = Some(false);
    drive.vdi = None;
    h.click("tools-review");
    assert_eq!(
        panel(&h).error.as_deref(),
        Some("Mounted image identity is unavailable")
    );
}

/// Outside the demo a review without a current, writable connection and
/// the observation made at capture is stale: it says so and cannot be
/// confirmed. (Rune disabled the button without a reason.)
#[test]
fn a_review_without_a_current_observation_is_stale() {
    let mut h = Harness::scene("guest-tools-review");
    h.model.demo = false;
    assert!(panel(&h).observed.is_none());
    assert_eq!(
        h.node(&format!("{}-stale", media::REVIEW)).text.as_deref(),
        Some(media::STALE)
    );
    assert_eq!(h.try_click("tools-confirm"), Err(media::STALE.into()));
    media::apply(&mut h.model);
    assert!(h.model.operations.is_empty());
    assert_eq!(panel(&h).error.as_deref(), Some(media::STALE));
    assert!(panel(&h).submitted.is_none());
}

/// Guest tools 81: once submitted the dialog stays open and follows the
/// operation; a second submit is not possible.
#[test]
fn a_submitted_mount_is_followed_in_place_and_cannot_be_submitted_twice() {
    let mut h = Harness::scene("guest-tools-review");
    let id = OperationId::new();
    panel_mut(&mut h).submitted = Some(id);
    assert_eq!(
        h.node("tools-admitted").text.as_deref(),
        Some(media::ADMITTED)
    );
    for key in ["tools-confirm", "tools-back", "tools-review"] {
        assert!(h.find(key).is_none(), "{key}");
    }
    media::apply(&mut h.model);
    assert_eq!(panel(&h).submitted, Some(id));
    assert!(panel(&h).error.is_none());
    assert!(h.model.operations.is_empty());

    let record = mount_record(&h, id, OperationState::Tracking);
    h.model.operation_rows.push(record);
    assert!(h.find("tools-admitted").is_none());
    assert!(h.find("tools-outcome").is_some());
    assert!(h.has_text("Tracking: Verified"));
    assert!(!h.has_text(media::MOUNTED));
    h.model.operation_rows[0] = mount_record(&h, id, OperationState::Succeeded);
    assert!(h.has_text("Succeeded: Verified"));
    assert!(h.has_text(media::MOUNTED));
    h.model.operation_rows[0] = mount_record(&h, id, OperationState::OutcomeUnknown);
    assert!(h.has_text("OutcomeUnknown: Verified"));
    assert!(!h.has_text(media::MOUNTED));
    h.click("tools-close");
    assert!(h.model.areas.vm_tools.media.is_none());
}

/// "Installation guide" asks the front end to open the documentation.
#[test]
fn the_installation_guide_is_a_url_intent() {
    let mut h = Harness::scene("guest-tools");
    h.take_intents();
    h.click("tools-guide");
    let intents = h.take_intents();
    assert!(
        intents
            .iter()
            .any(|intent| matches!(intent, Intent::OpenUrl(url) if url == media::GUIDE_URL)),
        "{intents:?}"
    );
    assert!(h.model.areas.vm_tools.media.is_some());
}

/// External tools: typing an address does not make it a manual one. A
/// typed address the guest does not report is refused until "Use a
/// manually supplied address" is pressed.
#[test]
fn a_typed_address_stays_reported_until_it_is_declared_manual() {
    let (mut h, running) = guest();
    let vm = running.single().unwrap().vm_id().unwrap();
    report(&mut h, vm, "192.0.2.50");
    h.invoke_on(CONNECT_EXTERNAL.0, &running);
    assert_eq!(launch(&h).address, "192.0.2.50");
    assert!(launch(&h).reported);
    assert_eq!(
        h.node("guest-tool-source").text.as_deref(),
        Some("Source: current guest tools report")
    );
    h.type_text("guest-tool-address", "192.0.2.99");
    assert!(launch(&h).reported);
    let guest = launch(&h).guest.clone();
    let live = inventory(&h);
    let typed = "192.0.2.99".parse().unwrap();
    let refused = guest.observed(true, guest.generation, Some(&live), typed, true);
    assert!(refused.unwrap_err().contains("no longer reported"));
    h.click("guest-tool-manual-address");
    assert!(!launch(&h).reported);
    assert!(h.find("guest-tool-manual-address").is_none());
    assert!(
        guest
            .observed(true, guest.generation, Some(&live), typed, false)
            .is_ok()
    );
    h.click("guest-tool-use-192.0.2.50");
    assert_eq!(launch(&h).address, "192.0.2.50");
    assert!(launch(&h).reported);
}

/// Choosing a tool sets its usual port and drops the review.
#[test]
fn choosing_a_tool_sets_its_port_and_drops_the_review() {
    let mut h = Harness::scene("guest-connections");
    h.model.preferences.external_tools.tools[0].executable = "/usr/bin/true".into();
    assert_eq!(launch(&h).port, "22");
    h.click("review-guest-tool");
    wait_for_review(&mut h);
    assert!(launch(&h).review.is_some());
    let rdp = h
        .model
        .preferences
        .external_tools
        .tools
        .iter()
        .find(|tool| tool.kind == Kind::Rdp)
        .expect("the RDP preset")
        .id;
    h.click(&format!("guest-tool-{rdp}"));
    assert_eq!(launch(&h).tool, rdp);
    assert_eq!(launch(&h).port, "3389");
    assert!(launch(&h).review.is_none());
    assert!(h.find("guest-tool-command").is_none());
}

/// A review is only the review of what is declared now: a changed tool
/// declaration drops it at the next poll.
#[test]
fn a_changed_declaration_drops_the_review() {
    let mut h = Harness::scene("guest-connections");
    h.model.preferences.external_tools.tools[0].executable = "/usr/bin/true".into();
    h.click("review-guest-tool");
    assert_eq!(
        h.try_click("review-guest-tool"),
        Err(external::INSPECTING.into())
    );
    wait_for_review(&mut h);
    assert!(launch(&h).review.is_some());
    h.model.preferences.external_tools.tools[0]
        .arguments
        .push("-v".into());
    assert!(h.tick(0.1));
    assert!(launch(&h).review.is_none());
    assert_eq!(status(&h).as_deref(), Some(external::REVIEW_DROPPED));
    h.model.preferences.external_tools.tools.remove(0);
    h.click("review-guest-tool");
    assert_eq!(
        status(&h).as_deref(),
        Some("The tool declaration changed. Select it again.")
    );
    assert!(launch(&h).pending.is_none());
}

/// Inputs are validated before anything is inspected.
#[test]
fn a_review_needs_a_literal_address_and_a_port() {
    let mut h = Harness::scene("guest-connections");
    h.type_text("guest-tool-address", "guest.example");
    h.click("review-guest-tool");
    assert_eq!(
        status(&h).as_deref(),
        Some("Enter a literal IPv4 or IPv6 guest address")
    );
    h.type_text("guest-tool-address", "192.0.2.23");
    h.type_text("guest-tool-port", "0");
    h.click("review-guest-tool");
    assert_eq!(status(&h).as_deref(), Some("Port must be from 1 to 65535"));
    assert!(launch(&h).pending.is_none());
    assert!(launch(&h).review.is_none());
}

/// Outside the demo a dialog whose connection is not current, or is of
/// another generation, can neither review nor launch.
#[test]
fn without_the_captured_connection_generation_nothing_is_reviewed_or_launched() {
    let mut h = Harness::scene("guest-connections");
    h.model.demo = false;
    assert_eq!(
        h.try_click("launch-guest-tool"),
        Err(external::CONNECTION_CHANGED.into())
    );
    h.click("review-guest-tool");
    assert_eq!(status(&h).as_deref(), Some(external::CONNECTION_CHANGED));
    assert!(launch(&h).pending.is_none());
    external::launch(&mut h.model);
    assert_eq!(status(&h).as_deref(), Some(external::CONNECTION_CHANGED));
    assert!(h.model.areas.vm_tools.external.jobs.is_empty());
}

/// The log keeps 32 lines and shows the last 8, newest first; a finished
/// job is forgotten.
#[test]
fn job_reports_reach_the_log_which_keeps_32_lines_and_shows_8() {
    let mut h = Harness::scene("guest-connections");
    assert!(h.find("guest-tool-history").is_none());
    let (sender, receiver) = mpsc::channel();
    h.model.areas.vm_tools.external.jobs.push(Job {
        label: "SSH · Database · 192.0.2.23".into(),
        receiver,
    });
    assert!(!external::poll(&mut h.model, 0.0), "nothing reported yet");
    for n in 0..40 {
        sender.send(format!("line {n}")).unwrap();
    }
    assert!(external::poll(&mut h.model, 0.0));
    assert_eq!(h.model.areas.vm_tools.external.jobs.len(), 1);
    let history = &h.model.areas.vm_tools.external.history;
    assert_eq!(history.len(), 32);
    assert_eq!(history[0], "SSH · Database · 192.0.2.23: line 8");
    let shown = h.node("guest-tool-history").text.expect("the log");
    let lines: Vec<&str> = shown.lines().collect();
    assert_eq!(lines.len(), 8);
    assert_eq!(lines[0], "SSH · Database · 192.0.2.23: line 39");
    assert_eq!(lines[7], "SSH · Database · 192.0.2.23: line 32");
    drop(sender);
    external::poll(&mut h.model, 0.0);
    assert!(h.model.areas.vm_tools.external.jobs.is_empty());
    // The log is also under the declarations, and survives a scope change.
    h.click("configure-guest-tools");
    assert!(h.find("guest-tool-history").is_some());
    let pool = Target::one(ObjectRef::pool(pool(&h)));
    h.select(pool);
    assert_eq!(h.model.areas.vm_tools.external.history.len(), 32);
}

/// Demo and preview never save declarations; a failed write rolls back.
#[test]
fn declarations_are_not_saved_in_a_preview_and_a_failed_write_rolls_back() {
    let mut h = Harness::scene("external-tools");
    let before = h.model.preferences.external_tools.clone();
    h.type_text("guest-tool-name", "Renamed");
    h.click("save-guest-tools");
    assert_eq!(status(&h).as_deref(), Some(external::PREVIEW));
    assert!(h.model.preferences.external_tools.tools == before.tools);

    h.model.areas.vm_tools.external.storage = Some(Err("disk full".into()));
    h.click("save-guest-tools");
    assert_eq!(status(&h).as_deref(), Some("disk full"));
    assert!(h.model.preferences.external_tools.tools == before.tools);

    h.model.areas.vm_tools.external.storage = Some(Ok(()));
    h.click("save-guest-tools");
    assert_eq!(status(&h).as_deref(), Some(external::SAVED));
    assert_eq!(h.model.preferences.external_tools.tools[0].name, "Renamed");
}

/// What a declaration must be: a JSON array of strings, of bounded size,
/// an absolute executable. Nothing invalid reaches the preferences.
#[test]
fn invalid_declarations_are_refused_before_anything_is_stored() {
    let mut h = Harness::scene("external-tools");
    h.model.areas.vm_tools.external.storage = Some(Ok(()));
    let before = h.model.preferences.external_tools.clone();
    h.type_text("guest-tool-arguments", "{\"a\": 1}");
    h.click("save-guest-tools");
    assert_eq!(
        status(&h).as_deref(),
        Some("Arguments must be a JSON array of separate strings")
    );
    h.type_text("guest-tool-arguments", &" ".repeat(20_001));
    h.click("save-guest-tools");
    assert_eq!(
        status(&h).as_deref(),
        Some("Argument declaration is too large")
    );
    h.type_text("guest-tool-arguments", "[]");
    h.type_text("guest-tool-executable", "ssh");
    h.click("save-guest-tools");
    assert!(status(&h).is_some_and(|text| text != external::SAVED));
    assert!(h.model.preferences.external_tools.tools == before.tools);
}

/// At most sixteen declarations: the three "Add" buttons say so.
#[test]
fn the_catalog_holds_sixteen_tools() {
    let mut h = Harness::scene("external-tools");
    for _ in 0..14 {
        h.click("add-guest-tool-custom");
    }
    for key in [
        "add-guest-tool-ssh",
        "add-guest-tool-rdp",
        "add-guest-tool-custom",
    ] {
        assert_eq!(h.try_click(key), Err(external::FULL.into()), "{key}");
    }
    // The newest declaration is the one being edited.
    h.pick("guest-tool-edit", "1");
    assert_eq!(
        h.node("guest-tool-name").value.as_deref(),
        Some("RDP with Remmina")
    );
}
