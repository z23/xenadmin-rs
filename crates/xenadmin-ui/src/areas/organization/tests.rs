//! Tests of the area.
//!
//! Section 1 accounts for the tests of the two Rune modules:
//!
//! | Rune test | Here |
//! | --- | --- |
//! | `organization_view::folder_rename_updates_descendants_and_registry_without_touching_other_paths` | moved, same name |
//! | `organization_view::custom_field_deletion_reviews_values_before_removing_definition` | moved, same name |
//! | `organization_view::exact_tag_and_folder_filters_are_distinct_from_name_search` | ported in part, same name: the filter input has no successor (the Search tab filters); the two rules it pinned, an exact tag and a folder matched at a path separator, are those of [`edit::has_tag`] and [`edit::in_folder`], which decide where a tag or folder command applies |
//! | `organization_view::compact_review_is_reachable_and_cannot_submit_demo_metadata` | ported to the harness, same name; the 513x652 geometry is a GPUI test (`gpui-tests-wanted.md`) |
//! | `appliance_view::power_review_captures_members_order_delays_and_requires_force_confirmation` | moved, same name |
//! | `appliance_view::membership_review_preserves_exact_group_and_refuses_stealing_other_group_vm` | moved, same name, through the table and the button |
//! | `appliance_view::compact_review_keeps_controls_visible_and_refuses_preview_submission` | ported to the harness, same name; geometry as above |
//! | `main.rs::dialogs_outside_the_supported_core_carry_the_experimental_notice` (share) | `both_dialogs_and_the_page_carry_the_experimental_notice` |
//! | `session_permissions_view.rs::review_permission_notice_is_reachable_without_bypassing_synthetic_target_guard` (share) | same name |
//!
//! Section 2 tests what the port changed: the navigation view, the
//! commands, the Custom Fields page, the per-pool rules.

use std::sync::Arc;

use xenadmin_core::{
    ApplianceId, ApplianceOperation as Op, AppliancePower, Error, Inventory,
    OrganizationChange as Change, PoolId,
};

use super::appliance;
use super::commands::*;
use super::edit::{self, Action, Workspace};
use super::scenes;
use super::view::{DEMO, NAME_NEEDED, NOT_EMPTY, pages};
use crate::surface::EXPERIMENTAL_NOTICE_KEY;
use crate::{Harness, prelude::*};

fn editor(h: &Harness) -> &Workspace {
    h.model
        .areas
        .organization
        .editor
        .as_ref()
        .expect("an organization change is open")
}

fn editor_mut(h: &mut Harness) -> &mut Workspace {
    h.model
        .areas
        .organization
        .editor
        .as_mut()
        .expect("an organization change is open")
}

fn vapps(h: &Harness) -> &appliance::Workspace {
    h.model
        .areas
        .organization
        .appliances
        .as_ref()
        .expect("the vApps dialog is open")
}

fn vapps_mut(h: &mut Harness) -> &mut appliance::Workspace {
    h.model
        .areas
        .organization
        .appliances
        .as_mut()
        .expect("the vApps dialog is open")
}

fn inventory(h: &Harness) -> Arc<Inventory> {
    h.model.inventory.clone().expect("the demo inventory")
}

fn pool(h: &Harness) -> PoolId {
    inventory(h).pool_id
}

/// The n-th guest of the demo pool.
fn guest(h: &Harness, n: usize) -> ObjectRef {
    let inventory = inventory(h);
    let vm = inventory
        .vms
        .values()
        .filter(|vm| !vm.is_template && !vm.is_control_domain)
        .nth(n)
        .expect("a guest");
    ObjectRef::vm(inventory.pool_id, vm.id)
}

/// The demo pool with the metadata of the Rune scene, nothing open.
fn with_metadata() -> Harness {
    let mut h = Harness::new();
    scenes::metadata(&mut h.model).expect("the demo pool");
    h
}

fn menu(h: &Harness, object: ObjectRef) -> Vec<&'static str> {
    h.context_menu(&Target::one(object))
        .into_iter()
        .map(|item| item.id.0)
        .collect()
}

fn disabled(h: &Harness, command: CommandId, target: &Target) -> Option<String> {
    match h.model.eligibility(command, target) {
        Eligibility::Disabled(reason) => Some(reason),
        _ => None,
    }
}

fn hidden(h: &Harness, command: CommandId, target: &Target) -> bool {
    matches!(h.model.eligibility(command, target), Eligibility::Hidden)
}

// -- 1. The Rune tests -----------------------------------------------------------

#[test]
fn folder_rename_updates_descendants_and_registry_without_touching_other_paths() {
    let mut h = Harness::scene("organization");
    let w = editor_mut(&mut h);
    w.action = Action::RenameFolder;
    w.name = "/Services".into();
    w.value = "/Production".into();
    w.inventory.objects[0].folder = Some("/Services/Databases".into());
    w.inventory.objects[1].folder = Some("/Services-old".into());
    w.inventory.empty_folders = Some(vec!["/Services/Empty".into(), "/Archive".into()]);
    let patch = edit::plan(w).unwrap();
    assert!(patch.edits().iter().any(
        |e| matches!(&e.change, Change::Folder { value: Some(v), .. } if v == "/Production/Databases")
    ));
    assert!(
        !patch
            .edits()
            .iter()
            .any(|e| e.object == w.inventory.objects[1].id)
    );
    assert!(patch.edits().iter().any(|e| matches!(&e.change, Change::EmptyFolders { value, .. } if value == &vec!["/Archive".to_string(), "/Production/Empty".to_string()])));
}

#[test]
fn custom_field_deletion_reviews_values_before_removing_definition() {
    let mut h = Harness::scene("organization");
    let w = editor_mut(&mut h);
    w.inventory.objects[0]
        .custom_fields
        .insert("Owner".into(), "Fixture operator".into());
    w.action = Action::DeleteField;
    w.name = "Owner".into();
    let patch = edit::plan(w).unwrap();
    assert!(matches!(
        &patch.edits()[0].change,
        Change::CustomField { value: None, .. }
    ));
    assert!(
        matches!(&patch.edits().last().unwrap().change, Change::Definitions { value, .. } if value.is_empty())
    );
    w.inventory.complete = false;
    assert_eq!(edit::plan(w).unwrap_err(), edit::INCOMPLETE);
    assert_eq!(edit::blocked(w).as_deref(), Some(edit::INCOMPLETE));
}

#[test]
fn exact_tag_and_folder_filters_are_distinct_from_name_search() {
    let mut h = Harness::scene("organization");
    let w = editor_mut(&mut h);
    w.inventory.objects.truncate(1);
    w.inventory.empty_folders = Some(Vec::new());
    w.inventory.objects[0].tags = Some(vec!["production".into()]);
    w.inventory.objects[0].folder = Some("/Service/Db".into());
    assert!(!edit::has_tag(&w.inventory, "prod"));
    assert!(edit::has_tag(&w.inventory, "production"));
    assert!(edit::has_folder(&w.inventory, "/Service"));
    assert!(!edit::has_folder(&w.inventory, "/Services"));
    assert!(edit::in_folder("/Service/Db", "/Service"));
    assert!(!edit::in_folder("/Service/Db", "/Services"));
    assert!(!edit::in_folder("/Services-old", "/Services"));
}

#[test]
fn compact_review_is_reachable_and_cannot_submit_demo_metadata() {
    let mut h = Harness::scene("organization");
    h.assert_unique_keys();
    h.type_text("organization-name", "reviewed-tag");
    h.click("organization-review");
    assert!(editor(&h).review.is_some());
    h.assert_unique_keys();
    assert_eq!(h.try_click("organization-apply"), Err(DEMO.into()));
    assert!(h.model.resource_operations.is_empty());
    assert!(h.model.areas.organization.editor.is_some());
    // The handler refuses by itself too.
    edit::submit(&mut h.model);
    assert_eq!(editor(&h).error, Some(Error::Stale.to_string()));
    assert!(h.model.resource_operations.is_empty());
}

#[test]
fn power_review_captures_members_order_delays_and_requires_force_confirmation() {
    let mut h = Harness::scene("appliances");
    let w = vapps_mut(&mut h);
    let group = appliance::selected(w).unwrap().id;
    let op = appliance::power_plan(w, AppliancePower::HardShutdown).unwrap();
    let Op::Power { members, .. } = &op else {
        panic!()
    };
    assert!(
        members
            .windows(2)
            .all(|pair| pair[0].order.order >= pair[1].order.order)
    );
    assert!(members.iter().all(|m| m.order.start_delay == 15));
    assert_eq!(appliance::confirmation(&op), Some(group.to_string()));
    assert!(
        appliance::confirmation(&appliance::power_plan(w, AppliancePower::CleanShutdown).unwrap())
            .is_none()
    );
    let mut changed = w.inventory.as_ref().clone();
    changed
        .details
        .appliances
        .get_mut(&group)
        .unwrap()
        .vms_known = false;
    w.inventory = Arc::new(changed);
    assert_eq!(
        appliance::power_plan(w, AppliancePower::Start).unwrap_err(),
        appliance::NO_MEMBERS
    );
    // The buttons say the same before they are pressed.
    assert_eq!(
        h.try_click("appliance-start"),
        Err(appliance::NO_MEMBERS.into())
    );
    assert!(h.find("appliance-incomplete").is_some());
}

#[test]
fn membership_review_preserves_exact_group_and_refuses_stealing_other_group_vm() {
    let mut h = Harness::scene("appliances");
    let id = appliance::selected(vapps(&h)).unwrap().vms[0];
    h.select_row("appliance-vms", &id.to_string());
    assert_eq!(
        h.node("appliance-membership-review").label.as_deref(),
        Some("Review removal from vApp")
    );
    h.click("appliance-membership-review");
    assert!(
        matches!(&vapps(&h).review, Some(Op::Membership { vm, join: false, expected_members, .. }) if *vm == id && expected_members.contains(&id))
    );
    h.click("appliance-back");
    let w = vapps_mut(&mut h);
    assert!(w.review.is_none());
    let mut inventory = w.inventory.as_ref().clone();
    inventory.details.vms.get_mut(&id).unwrap().appliance =
        Some(ApplianceId::from(uuid::Uuid::new_v4()));
    w.inventory = Arc::new(inventory);
    h.click("appliance-membership-review");
    assert!(vapps(&h).review.is_none());
    assert_eq!(
        vapps(&h).error.as_deref(),
        Some("Remove this VM from its current vApp first")
    );
    assert!(h.has_text("Remove this VM from its current vApp first"));
}

#[test]
fn compact_review_keeps_controls_visible_and_refuses_preview_submission() {
    let mut h = Harness::scene("appliance-review");
    h.assert_unique_keys();
    let required = appliance::confirmation(vapps(&h).review.as_ref().unwrap()).unwrap();
    assert!(h.has_text(&format!("Enter vApp UUID to confirm: {required}")));
    assert!(appliance::unconfirmed(vapps(&h)));
    h.type_text("appliance-confirmation", &required);
    assert!(!appliance::unconfirmed(vapps(&h)));
    assert_eq!(h.try_click("appliance-apply"), Err(DEMO.into()));
    assert!(h.model.resource_operations.is_empty());
    assert!(h.model.areas.organization.appliances.is_some());
    appliance::apply(&mut h.model);
    assert_eq!(
        vapps(&h).error.as_deref(),
        Some("Connection changed; reopen vApps")
    );
    assert!(h.model.resource_operations.is_empty());
}

/// This area's share of Rune `main.rs`
/// `dialogs_outside_the_supported_core_carry_the_experimental_notice`.
#[test]
fn both_dialogs_and_the_page_carry_the_experimental_notice() {
    for scene in [
        "organization",
        "organization-review",
        "appliances",
        "appliance-review",
    ] {
        let h = Harness::scene(scene);
        assert!(h.find(EXPERIMENTAL_NOTICE_KEY).is_some(), "{scene}");
    }
    let mut h = with_metadata();
    h.select(guest(&h, 0));
    h.invoke("vm.properties");
    h.pick("vm-properties-pages", edit::PAGE);
    assert_eq!(h.surface().id, "vm-properties");
    assert!(h.find(EXPERIMENTAL_NOTICE_KEY).is_some());
}

/// This area's share of Rune `session_permissions_view.rs`
/// `review_permission_notice_is_reachable_without_bypassing_synthetic_target_guard`.
#[test]
fn review_permission_notice_is_reachable_without_bypassing_synthetic_target_guard() {
    for (scene, apply) in [
        ("organization-review", "organization-apply"),
        ("appliance-review", "appliance-apply"),
    ] {
        let mut h = Harness::scene(scene);
        assert!(
            h.find(permissions::REVIEW_KEY).is_some(),
            "captured review must explain permission discovery: {scene}"
        );
        assert_eq!(h.try_click(apply), Err(DEMO.into()), "{scene}");
        assert!(h.model.resource_operations.is_empty());
        assert!(h.model.error.is_none());
    }
    // Before the review there is no operation to assess, so no line.
    for scene in ["organization", "appliances"] {
        assert!(
            Harness::scene(scene)
                .find(permissions::REVIEW_KEY)
                .is_none()
        );
    }
}

// -- 2. What the port changed ------------------------------------------------------

#[test]
fn the_organization_view_lists_folders_tags_and_vapps_of_the_shown_pools() {
    let mut h = Harness::scene("organization");
    let labels: Vec<String> = h.nav_rows().into_iter().map(|row| row.label).collect();
    assert_eq!(
        labels,
        ["Folders", "Archives", "Services", "Tags", "lab", "service"]
    );
    let services = h.nav_row("Services");
    assert_eq!(services.object, Some(super::folder_object("/Services")));
    assert_eq!(services.parent.as_deref(), Some(super::FOLDERS_ROW));
    // The scene selected the folder: its row shows the scope.
    let rows = h.nav_rows();
    assert_eq!(
        crate::nav::selected_ids(&h.model, &rows),
        ["organization:folder:/Services"]
    );
    assert_eq!(h.model.tabs(), [TabId::Search]);
    // Another row is another scope: the dialog of the scene closes.
    h.press_nav("lab");
    assert_eq!(h.model.scope(), &Target::one(super::tag_object("lab")));
    assert!(h.model.areas.organization.editor.is_none());

    // The plain demo has no metadata and no vApp: no heading without rows.
    let mut plain = Harness::new();
    plain.model.load_demo(3, 48);
    plain.open_view(NavView::Organization);
    assert!(plain.nav_rows().is_empty());

    let mut h = Harness::scene("appliances");
    h.open_view(NavView::Organization);
    let rows = h.nav_rows();
    assert_eq!(
        rows.iter().map(|r| r.label.as_str()).collect::<Vec<_>>(),
        ["vApps", "Example application"]
    );
    assert_eq!(
        rows[1].object.as_ref().map(|o| (o.kind, o.id.as_str())),
        Some((ObjectKind::Appliance, scenes::EXAMPLE_VAPP))
    );
}

#[test]
fn nested_folders_are_a_tree_with_their_ancestors() {
    let mut h = with_metadata();
    let mut changed = inventory(&h).as_ref().clone();
    changed.details.organization.objects[0].folder = Some("/Services/Databases/Primary".into());
    changed.details.organization.empty_folders = Some(vec!["/Archives/2024".into()]);
    h.model.inventory = Some(Arc::new(changed));
    h.open_view(NavView::Organization);
    let rows = h.nav_rows();
    let folders: Vec<(&str, Option<&str>)> = rows
        .iter()
        .filter(|row| row.id.starts_with("organization:folder:"))
        .map(|row| (row.id.as_str(), row.parent.as_deref()))
        .collect();
    assert_eq!(
        folders,
        [
            ("organization:folder:/Archives", Some(super::FOLDERS_ROW)),
            (
                "organization:folder:/Archives/2024",
                Some("organization:folder:/Archives")
            ),
            ("organization:folder:/Services", Some(super::FOLDERS_ROW)),
            (
                "organization:folder:/Services/Databases",
                Some("organization:folder:/Services")
            ),
            (
                "organization:folder:/Services/Databases/Primary",
                Some("organization:folder:/Services/Databases")
            ),
        ]
    );
    assert_eq!(h.nav_row("Primary").label, "Primary");
    let mut ids: Vec<&str> = rows.iter().map(|row| row.id.as_str()).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), rows.len(), "row ids are unique");
}

#[test]
fn context_menus_offer_the_actions_of_the_kind() {
    let h = with_metadata();
    let vm = menu(&h, guest(&h, 0));
    for id in [
        "org.tag.add",
        "org.tag.remove",
        "org.folder.move",
        "org.field.set",
        "org.field.clear",
        "vm.assign-to-vapp",
    ] {
        assert!(vm.contains(&id), "{id} in {vm:?}");
    }
    assert!(!vm.contains(&"org.tag.rename"));
    assert!(!vm.contains(&"org.folder.create"));

    let tag = menu(&h, super::tag_object("lab"));
    assert!(tag.contains(&"org.tag.rename") && tag.contains(&"org.tag.delete"));
    assert!(!tag.contains(&"org.tag.add"));
    let folder = menu(&h, super::folder_object("/Services"));
    for id in ["org.folder.create", "org.folder.rename", "org.folder.delete"] {
        assert!(folder.contains(&id), "{id} in {folder:?}");
    }
    let of_pool = menu(&h, ObjectRef::pool(pool(&h)));
    assert!(of_pool.contains(&"org.folder.create") && of_pool.contains(&"pool.vapps"));
    // A tag or a folder no pool has: its commands do not apply.
    assert!(hidden(
        &h,
        TAG_RENAME,
        &Target::one(super::tag_object("nowhere"))
    ));
    assert!(hidden(
        &h,
        FOLDER_DELETE,
        &Target::one(super::folder_object("/Nowhere"))
    ));

    let h = Harness::scene("appliances");
    let vapp = ObjectRef::new(Some(pool(&h)), ObjectKind::Appliance, scenes::EXAMPLE_VAPP);
    let items = menu(&h, vapp.clone());
    for id in [
        "pool.vapps",
        "vapp.start",
        "vapp.shutdown",
        "vapp.force-shutdown",
    ] {
        assert!(items.contains(&id), "{id} in {items:?}");
    }
    // A vApp carries no tags, folder or custom fields.
    assert!(hidden(&h, TAG_ADD, &Target::one(vapp)));
}

#[test]
fn a_command_is_disabled_with_the_text_its_review_would_give() {
    // The plain demo has no organization metadata at all.
    let mut h = Harness::new();
    h.model.load_demo(3, 48);
    let vm = guest(&h, 0);
    let name = h.model.name_of(&vm).unwrap();
    assert_eq!(
        disabled(&h, TAG_ADD, &Target::one(vm.clone())),
        Some(format!("Tags are unavailable for {name}"))
    );
    assert_eq!(
        disabled(&h, FOLDER_MOVE, &Target::one(vm.clone())),
        Some(format!("Folder metadata is unavailable for {name}"))
    );
    assert_eq!(
        disabled(&h, FIELD_SET, &Target::one(vm.clone())),
        Some(format!("Custom field metadata is unavailable for {name}"))
    );
    assert_eq!(
        disabled(&h, FOLDER_CREATE, &Target::one(ObjectRef::pool(pool(&h)))),
        Some(edit::NO_REGISTRY.into())
    );
    assert_eq!(
        disabled(&h, ASSIGN_TO_VAPP, &Target::one(vm)),
        Some(NO_MEMBERSHIP.into())
    );

    let h = with_metadata();
    let p = pool(&h);
    // More than the Rune limit of one selection.
    let many = Target::many(
        (0..=edit::LIMIT as u128).map(|n| ObjectRef::vm(p, uuid::Uuid::from_u128(n + 1).into())),
    );
    assert_eq!(disabled(&h, TAG_ADD, &many), Some(edit::TOO_MANY.into()));
    // Objects of two pools are no selection of one review.
    let two = Target::many([
        guest(&h, 0),
        ObjectRef::vm(uuid::Uuid::from_u128(7).into(), uuid::Uuid::from_u128(8).into()),
    ]);
    assert!(hidden(&h, TAG_ADD, &two));
    // Several objects of one pool are.
    let both = Target::many([guest(&h, 0), guest(&h, 1)]);
    assert!(matches!(
        h.model.eligibility(TAG_ADD, &both),
        Eligibility::Enabled
    ));
    // A pool-wide action needs complete metadata.
    let mut h = with_metadata();
    let mut changed = inventory(&h).as_ref().clone();
    changed.details.organization.complete = false;
    h.model.inventory = Some(Arc::new(changed));
    assert_eq!(
        disabled(&h, TAG_DELETE, &Target::one(super::tag_object("lab"))),
        Some(edit::INCOMPLETE.into())
    );
}

#[test]
fn a_selection_becomes_the_selection_of_the_review() {
    let mut h = with_metadata();
    let both = Target::many([guest(&h, 0), guest(&h, 1)]);
    h.invoke_on("org.folder.move", &both);
    assert_eq!(h.surface().id, "organization");
    assert_eq!(h.surface().title, "Move selection to folder");
    assert_eq!(editor(&h).selected.len(), 2);
    assert!(h.node("organization-selected").text.is_some_and(|t| t.starts_with("2 selected: ")));
    assert!(h.find("organization-action").is_none());
    h.type_text("organization-name", "Databases");
    h.click("organization-review");
    assert!(editor(&h).review.is_none());
    assert!(h.node("organization-error").text.is_some_and(|t| t.starts_with("Use a full folder path")));
    h.type_text("organization-name", "/Databases");
    h.submit("organization-name");
    let patch = editor(&h).review.clone().expect("the review");
    assert_eq!(patch.edits().len(), 2);
    assert!(h.has_text(&format!("2 changes in pool {}", pool(&h))));
    assert!(h.has_text("Folder: /Services → /Databases"));
    // Back keeps the draft; Refresh drops it and keeps the selection.
    h.click("organization-back");
    assert_eq!(editor(&h).name, "/Databases");
    h.click("organization-refresh");
    assert_eq!(editor(&h).name, "");
    assert_eq!(editor(&h).selected.len(), 2);
    h.click("organization-close");
    assert!(h.model.areas.organization.editor.is_none());
}

#[test]
fn a_tag_row_presets_its_tag_and_asks_for_the_pool_when_two_have_it() {
    let mut h = with_metadata();
    let tag = Target::one(super::tag_object("lab"));
    h.invoke_on("org.tag.rename", &tag);
    assert_eq!(editor(&h).action, Action::RenameTag);
    assert_eq!(editor(&h).name, "lab");
    assert!(h.find("organization-pool").is_none());
    h.type_text("organization-value", "staging");
    h.click("organization-review");
    let tagged = inventory(&h)
        .details
        .organization
        .objects
        .iter()
        .filter(|o| o.tags.iter().flatten().any(|t| t == "lab"))
        .count();
    assert!(tagged > 0);
    assert_eq!(editor(&h).review.as_ref().unwrap().edits().len(), tagged);
    assert!(h.has_text("Tags: lab → staging"));
    h.dismiss();
    assert!(h.model.areas.organization.editor.is_none());

    // A second pool with the same tag.
    let first = pool(&h);
    let mut second = inventory(&h).as_ref().clone();
    second.pool_id = uuid::Uuid::from_u128(99).into();
    second.pool_name = "Recovery lab".into();
    let second_pool = second.pool_id;
    h.model.add_synthetic_pool("Recovery lab", second);
    h.invoke_on("org.tag.rename", &tag);
    assert_eq!(editor(&h).pools.len(), 2);
    assert_eq!(editor(&h).pool, first, "the scope session's pool first");
    h.type_text("organization-value", "staging");
    h.pick("organization-pool", &second_pool.to_string());
    assert_eq!(editor(&h).pool, second_pool);
    assert_eq!(editor(&h).value, "staging", "the draft stays");
    assert_eq!(editor(&h).name, "lab");

    // A folder row presets the path; a new folder starts under it.
    let folder = Target::one(super::folder_object("/Services"));
    h.invoke_on("org.folder.delete", &folder);
    assert_eq!(editor(&h).name, "/Services");
    h.invoke_on("org.folder.create", &folder);
    assert_eq!(editor(&h).name, "/Services/");
    h.type_text("organization-name", "/Services/New");
    h.click("organization-review");
    assert!(h.has_text("Empty folders: /Archives → /Archives, /Services/New"));
}

#[test]
fn the_custom_fields_page_edits_the_fields_of_one_object() {
    let mut h = with_metadata();
    let vm = guest(&h, 0);
    h.select(vm.clone());
    h.invoke("vm.properties");
    h.pick("vm-properties-pages", edit::PAGE);
    assert!(h.surfaces().iter().all(|s| s.id != "organization"));
    assert!(editor(&h).page);
    assert_eq!(editor(&h).action, Action::SetField);
    assert_eq!(h.node("organization-value-of-Owner").value.as_deref(), Some("unset"));
    // The buttons of the defined fields copy the name.
    h.click("organization-field-Owner");
    h.type_text("organization-value", "ops");
    h.click("organization-review");
    assert_eq!(editor(&h).review.as_ref().unwrap().edits().len(), 1);
    assert!(h.has_text("Owner: unset → ops"));
    assert!(h.find(permissions::REVIEW_KEY).is_some());
    assert_eq!(h.try_click("organization-apply"), Err(DEMO.into()));
    h.click("organization-back");
    // Another action keeps the name, as in Rune: defining "Owner" again
    // is refused.
    h.pick("organization-action", "define-field");
    assert_eq!(editor(&h).action, Action::DefineField);
    assert!(h.find("organization-date").is_some());
    h.click("organization-review");
    assert_eq!(
        editor(&h).error.as_deref(),
        Some("Choose a new custom field name")
    );
    h.type_text("organization-name", "Renewal");
    h.toggle("organization-date");
    h.click("organization-review");
    assert!(h.has_text("Definitions: Owner → Owner, Renewal"));
    // The draft does not outlive the page, and Close closes the dialog.
    h.pick("vm-properties-pages", "general");
    assert!(h.model.areas.organization.editor.is_none());
    h.pick("vm-properties-pages", edit::PAGE);
    assert!(editor(&h).review.is_none());
    h.click("organization-close");
    assert!(h.model.properties.is_none());
    assert!(h.model.areas.organization.editor.is_none());
}

#[test]
fn every_properties_dialog_of_one_object_has_the_page() {
    let h = with_metadata();
    let inventory = inventory(&h);
    let p = inventory.pool_id;
    let mut objects = vec![ObjectRef::pool(p), guest(&h, 0)];
    objects.extend(inventory.hosts.keys().next().map(|id| ObjectRef::host(p, *id)));
    objects.extend(
        inventory
            .storage
            .first()
            .map(|sr| ObjectRef::storage(p, sr.uuid)),
    );
    objects.extend(
        inventory
            .networks
            .first()
            .map(|network| ObjectRef::network(p, network.uuid)),
    );
    assert!(objects.len() >= 4);
    for object in objects {
        let found = pages(&h.model, &Target::one(object.clone()));
        assert_eq!(found.len(), 1, "{object:?}");
        assert_eq!(found[0].id, edit::PAGE);
        assert!(found[0].experimental);
    }
    // Several VMs have the bulk page of another area; a vApp, a folder and
    // an object that is gone have no dialog of ours.
    assert!(pages(&h.model, &Target::many([guest(&h, 0), guest(&h, 1)])).is_empty());
    assert!(pages(&h.model, &Target::one(super::folder_object("/Services"))).is_empty());
    let gone = ObjectRef::vm(p, uuid::Uuid::from_u128(424242).into());
    assert!(pages(&h.model, &Target::one(gone)).is_empty());
}

#[test]
fn a_review_cannot_be_applied_without_a_current_writable_connection() {
    let mut h = Harness::scene("organization-review");
    // Not the demo any more, and no connection: stale, not a preview.
    h.model.demo = false;
    assert!(h.find("organization-preview").is_none());
    assert_eq!(
        h.node("organization-change-review-stale").text.as_deref(),
        Some(edit::NOT_CURRENT)
    );
    assert_eq!(
        h.try_click("organization-apply"),
        Err(edit::NOT_CURRENT.into())
    );
    edit::submit(&mut h.model);
    assert_eq!(editor(&h).error, Some(Error::Stale.to_string()));
    assert!(h.model.resource_operations.is_empty());
    h.click("organization-back");
    assert_eq!(h.node("organization-not-current").text.as_deref(), Some(edit::NOT_CURRENT));

    let mut h = Harness::scene("appliance-review");
    h.model.demo = false;
    assert_eq!(
        h.try_click("appliance-apply"),
        Err(appliance::NOT_CURRENT.into())
    );
    appliance::apply(&mut h.model);
    assert_eq!(
        vapps(&h).error.as_deref(),
        Some("Connection changed; reopen vApps")
    );
    assert!(h.model.resource_operations.is_empty());
}

#[test]
fn the_vapps_dialog_lists_edits_and_reviews() {
    let mut h = Harness::scene("appliances");
    assert_eq!(h.surface().id, "appliances");
    assert_eq!(vapps(&h).name, "Example application");
    assert!(h.has_text("1 matching vApps; showing up to 256"));
    assert_eq!(
        h.try_click("appliance-delete-review"),
        Err(NOT_EMPTY.into())
    );
    // Order and delays of a member.
    let member = appliance::selected(vapps(&h)).unwrap().vms[1];
    h.select_row("appliance-vms", &member.to_string());
    assert_eq!(vapps(&h).order, "1");
    assert_eq!(vapps(&h).start_delay, "15");
    assert_eq!(vapps(&h).shutdown_delay, "5");
    h.click("appliance-order-review");
    assert_eq!(
        vapps(&h).error.as_deref(),
        Some("Order and delays have not changed")
    );
    h.type_text("appliance-order", "x");
    h.click("appliance-order-review");
    assert_eq!(vapps(&h).error.as_deref(), Some("Enter an integer order"));
    h.type_text("appliance-order", "7");
    h.click("appliance-order-review");
    assert!(matches!(
        &vapps(&h).review,
        Some(Op::SetOrder { vm, value, .. }) if *vm == member && value.order == 7
    ));
    assert!(h.has_text("Order: 1 → 7"));
    assert!(h.find("appliance-confirmation").is_none());
    h.click("appliance-back");
    // Rename: the drafts are the operation, untrimmed.
    h.type_text("appliance-name", "Renamed ");
    h.click("appliance-rename-review");
    assert!(matches!(
        &vapps(&h).review,
        Some(Op::Rename { previous_name, name, .. })
            if previous_name == "Example application" && name == "Renamed "
    ));
    h.click("appliance-back");
    // A new vApp needs a name.
    h.click("appliance-new");
    assert!(vapps(&h).selected.is_none());
    assert_eq!(
        h.try_click("appliance-create-review"),
        Err(NAME_NEEDED.into())
    );
    h.type_text("appliance-name", "Batch");
    h.click("appliance-create-review");
    assert!(matches!(&vapps(&h).review, Some(Op::Create { name, .. }) if name == "Batch"));
    assert!(h.has_text("Create empty vApp: Batch"));
    h.click("appliance-back");
    // Refresh drops the draft; the list filter narrows the list.
    h.click("appliance-refresh");
    assert_eq!(vapps(&h).name, "");
    h.type_text("appliance-find", "no such");
    assert!(h.has_text("0 matching vApps; showing up to 256"));
    h.click("appliance-close");
    assert!(h.model.areas.organization.appliances.is_none());
}

#[test]
fn assign_to_vapp_opens_the_dialog_on_the_vm() {
    let mut h = Harness::new();
    let vapp = scenes::example_vapp(&mut h.model).expect("the example vApp");
    let p = pool(&h);
    let group = ApplianceId::from(vapp.single().unwrap().uuid().unwrap());
    let members = inventory(&h).details.appliances[&group].vms.clone();
    // A member: the dialog opens on its vApp with the VM chosen.
    h.invoke_on("vm.assign-to-vapp", &Target::one(ObjectRef::vm(p, members[1])));
    assert_eq!(vapps(&h).selected, Some(group));
    assert_eq!(vapps(&h).vm, Some(members[1]));
    assert_eq!(vapps(&h).order, "1");
    assert_eq!(
        h.node("appliance-membership-review").label.as_deref(),
        Some("Review removal from vApp")
    );
    h.dismiss();
    assert!(h.model.areas.organization.appliances.is_none());

    // A guest in no vApp: the user picks the vApp, the VM stays chosen.
    let mut changed = inventory(&h).as_ref().clone();
    let other = changed
        .vms
        .values()
        .find(|vm| !vm.is_template && !vm.is_control_domain && !members.contains(&vm.id))
        .map(|vm| vm.id)
        .expect("a guest outside the vApp");
    let details = changed.details.vms.entry(other).or_default();
    details.appliance = None;
    details.appliance_known = true;
    h.model.inventory = Some(Arc::new(changed));
    h.invoke_on("vm.assign-to-vapp", &Target::one(ObjectRef::vm(p, other)));
    assert_eq!(vapps(&h).selected, None);
    assert_eq!(vapps(&h).assign, Some(other));
    h.select_row("appliance-list", &group.to_string());
    assert_eq!(vapps(&h).vm, Some(other));
    assert_eq!(
        h.node("appliance-membership-review").label.as_deref(),
        Some("Review addition to vApp")
    );
    h.click("appliance-membership-review");
    assert!(matches!(
        &vapps(&h).review,
        Some(Op::Membership { vm, join: true, expected_members, .. })
            if *vm == other && *expected_members == members
    ));
    // A template or several VMs: not this command's.
    assert!(hidden(
        &h,
        ASSIGN_TO_VAPP,
        &Target::many([ObjectRef::vm(p, members[0]), ObjectRef::vm(p, members[1])])
    ));
}

#[test]
fn one_surface_of_the_area_at_a_time() {
    let mut h = Harness::scene("organization");
    let whole = Target::one(ObjectRef::pool(pool(&h)));
    h.invoke_on("pool.vapps", &whole);
    assert!(h.model.areas.organization.editor.is_none());
    assert_eq!(h.surfaces().len(), 1);
    assert_eq!(h.surface().id, "appliances");
    assert!(vapps(&h).selected.is_none());
    assert!(h.has_text("0 matching vApps; showing up to 256"));
    let first = guest(&h, 0);
    h.invoke_on("org.tag.add", &Target::one(first));
    assert!(h.model.areas.organization.appliances.is_none());
    assert_eq!(h.surface().id, "organization");
    // A vApp power command opens the dialog on its review.
    let mut h = Harness::new();
    let vapp = scenes::example_vapp(&mut h.model).unwrap();
    h.invoke_on("vapp.start", &vapp);
    assert!(matches!(
        &vapps(&h).review,
        Some(Op::Power {
            action: AppliancePower::Start,
            ..
        })
    ));
    assert!(h.has_text("Start in ascending order; VMs at the same order start together. Running VMs are skipped. Suspended/paused VMs need a separate recovery action first."));
    assert!(h.find("appliance-confirmation").is_none());
}

/// On a current writable connection the reviewed vApp operation is
/// submitted once on the connection of its pool and the dialog closes.
/// (The fixture server does not serve the operation itself: only the
/// submission is tested.)
#[test]
fn a_reviewed_vapp_change_is_submitted_on_the_connection_of_its_pool() {
    let mut live = crate::testing::Live::with_vm();
    let whole = Target::one(ObjectRef::pool(live.pool));
    live.h.invoke_on("pool.vapps", &whole);
    assert!(live.h.find("appliance-preview").is_none());
    assert!(live.h.find("appliance-not-current").is_none());
    live.h.type_text("appliance-name", "Batch");
    live.h.click("appliance-create-review");
    assert!(live.h.find("appliance-change-review-stale").is_none());
    live.h.click("appliance-apply");
    assert!(live.h.model.areas.organization.appliances.is_none());
    assert_eq!(live.h.model.resource_operations.len(), 1);
}
