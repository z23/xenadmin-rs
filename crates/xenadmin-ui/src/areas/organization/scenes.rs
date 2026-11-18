//! The scenes of the area, under their Rune names.
//!
//! The Rune scenes opened a workspace and set its fields. Here a scene
//! reaches the same state the way a user does: the demo pool with the
//! metadata of the Rune scene, the Organization navigation view, a command.

use std::sync::Arc;

use xenadmin_core::{
    ApplianceId, ApplianceOrder, CustomFieldDefinition, CustomFieldType, OrganizationInventory,
    OrganizationObject, OrganizationObjectId as Id, VmAppliance,
};

use super::commands::{TAG_ADD, VAPP_FORCE_SHUT_DOWN, VAPPS};
use super::edit;
use crate::prelude::*;

const AREA: &str = "organization";

/// The id of the vApp of the Rune scene.
pub const EXAMPLE_VAPP: &str = "a0000000-0000-4000-8000-000000000001";

pub fn scenes() -> Vec<SceneSpec> {
    vec![
        SceneSpec::new("organization", AREA, scene),
        SceneSpec::new("organization-review", AREA, review_scene),
        SceneSpec::new("appliances", AREA, appliances_scene),
        SceneSpec::new("appliance-review", AREA, appliance_review_scene),
    ]
}

/// The demo pool with the metadata of the Rune `organization_view::scene`:
/// every VM tagged `service` or `lab` and in the folder `/Services`, the
/// empty folder `/Archives`, the custom field `Owner`. Returns the first
/// object of that metadata as a target.
pub fn metadata(m: &mut Model) -> Option<Target> {
    m.load_demo(3, 48);
    let mut inventory = m.inventory.as_deref().cloned()?;
    inventory.details.organization = OrganizationInventory {
        complete: true,
        objects: inventory
            .vms
            .values()
            .enumerate()
            .map(|(i, vm)| OrganizationObject {
                id: Id::Vm(vm.id),
                name: vm.name.clone(),
                tags: Some(vec![if i % 2 == 0 { "service" } else { "lab" }.into()]),
                folder: Some("/Services".into()),
                other_config_known: true,
                custom_fields: std::collections::BTreeMap::new(),
            })
            .collect(),
        empty_folders: Some(vec!["/Archives".into()]),
        definitions: Some(vec![CustomFieldDefinition {
            name: "Owner".into(),
            kind: CustomFieldType::String,
        }]),
    };
    let pool = inventory.pool_id;
    let first = inventory.vms.values().next().map(|vm| {
        if vm.is_template {
            ObjectRef::template(pool, vm.id)
        } else {
            ObjectRef::vm(pool, vm.id)
        }
    });
    m.inventory = Some(Arc::new(inventory));
    first.map(Target::one)
}

/// The Organization view with the folder `/Services` selected, and the
/// dialog of the Rune default action ("Add tag to selection") on the first
/// object. Rune opened its workspace with nothing selected; a command has a
/// target.
fn scene(m: &mut Model) {
    let Some(first) = metadata(m) else {
        return;
    };
    m.set_nav_view(NavView::Organization);
    m.select(Target::one(super::folder_object("/Services")));
    m.invoke(TAG_ADD, &first);
}

fn review_scene(m: &mut Model) {
    scene(m);
    if let Some(w) = &mut m.areas.organization.editor {
        w.name = "reviewed-tag".into();
    }
    edit::review(m);
}

/// The demo pool with the vApp of the Rune `appliance_view::scene`:
/// "Example application" with the first three guests, order 0 to 2, delays
/// 15 and 5. Returns the vApp as a target.
pub fn example_vapp(m: &mut Model) -> Option<Target> {
    m.load_demo(3, 48);
    let mut inventory = m.inventory.as_deref().cloned()?;
    let id = ApplianceId::from(uuid::Uuid::parse_str(EXAMPLE_VAPP).ok()?);
    let vms: Vec<_> = inventory
        .vms
        .values()
        .filter(|v| !v.is_template)
        .take(3)
        .map(|v| v.id)
        .collect();
    for (i, vm) in vms.iter().enumerate() {
        let d = inventory.details.vms.entry(*vm).or_default();
        d.appliance = Some(id);
        d.appliance_known = true;
        d.appliance_order = Some(ApplianceOrder {
            order: i as i64,
            start_delay: 15,
            shutdown_delay: 5,
        });
    }
    inventory.details.appliances.insert(
        id,
        VmAppliance {
            id,
            name: "Example application".into(),
            description: "Synthetic database and web guests".into(),
            vms,
            vms_known: true,
            allowed_operations: Some(vec![
                "start".into(),
                "clean_shutdown".into(),
                "hard_shutdown".into(),
            ]),
        },
    );
    let pool = inventory.pool_id;
    m.inventory = Some(Arc::new(inventory));
    Some(Target::one(ObjectRef::new(
        Some(pool),
        ObjectKind::Appliance,
        id,
    )))
}

fn appliances_scene(m: &mut Model) {
    if let Some(vapp) = example_vapp(m) {
        m.invoke(VAPPS, &vapp);
    }
}

fn appliance_review_scene(m: &mut Model) {
    if let Some(vapp) = example_vapp(m) {
        m.invoke(VAPP_FORCE_SHUT_DOWN, &vapp);
    }
}
