//! The commands of the area: what the action buttons of the Rune
//! "Organization" workspace and the entry to the "vApps" workspace became.
//!
//! One rule for the organization commands: `Hidden` when the action has
//! nothing to do with the target (another kind, an object of a pool that is
//! not shown, objects of two pools); `Disabled` with the text the review
//! would give when no draft could make the review pass
//! ([`edit::blocked`]); `Enabled` otherwise. Each opens the dialog
//! `organization` on the draft of that one action, with the target as the
//! selection. The dialog makes every check again.
//!
//! None of the organization commands has a menu path: the plan's Menus
//! table has no item for them, and Rune reached them from one button. In a
//! context menu they are the submenu "Organize"
//! ([`place::CONTEXT_ORGANIZE`]); where one of them is alone (Create folder
//! on a folder) it is a plain item, and a tag's or a folder's own Rename and
//! Delete are the menu of that tag or folder, not a submenu of it.

use xenadmin_core::AppliancePower;

use super::appliance;
use super::edit::{self, Action};
use crate::prelude::*;

const AREA: &str = "organization";

pub const TAG_ADD: CommandId = CommandId("org.tag.add");
pub const TAG_REMOVE: CommandId = CommandId("org.tag.remove");
pub const TAG_RENAME: CommandId = CommandId("org.tag.rename");
pub const TAG_DELETE: CommandId = CommandId("org.tag.delete");
pub const FOLDER_MOVE: CommandId = CommandId("org.folder.move");
pub const FOLDER_CREATE: CommandId = CommandId("org.folder.create");
pub const FOLDER_RENAME: CommandId = CommandId("org.folder.rename");
pub const FOLDER_DELETE: CommandId = CommandId("org.folder.delete");
pub const FIELD_SET: CommandId = CommandId("org.field.set");
pub const FIELD_CLEAR: CommandId = CommandId("org.field.clear");
pub const VAPPS: CommandId = CommandId("pool.vapps");
pub const ASSIGN_TO_VAPP: CommandId = CommandId("vm.assign-to-vapp");
pub const VAPP_START: CommandId = CommandId("vapp.start");
pub const VAPP_SHUT_DOWN: CommandId = CommandId("vapp.shutdown");
pub const VAPP_FORCE_SHUT_DOWN: CommandId = CommandId("vapp.force-shutdown");

/// The Rune refusal of a membership review of a VM whose vApp is unknown;
/// here the reason "Assign to vApp" is disabled for it.
pub const NO_MEMBERSHIP: &str = "VM membership is unavailable";

/// The kinds that carry tags, a folder and custom fields.
const OBJECTS: &[ObjectKind] = &[
    ObjectKind::Pool,
    ObjectKind::Host,
    ObjectKind::Vm,
    ObjectKind::Template,
    ObjectKind::StorageRepository,
    ObjectKind::Network,
    ObjectKind::VirtualDisk,
    ObjectKind::Snapshot,
];

macro_rules! organization {
    ($id:expr, $action:expr, $kinds:expr) => {
        CommandSpec::new($id.0, $action.title(), AREA)
            .context($kinds)
            .eligibility(|m, target| eligibility(m, target, $action))
            .invoke(|m, target| edit::open(m, target, $action))
    };
}

pub fn commands() -> Vec<CommandSpec> {
    const TAG: &[ObjectKind] = &[ObjectKind::Tag];
    const FOLDER: &[ObjectKind] = &[ObjectKind::Folder];
    const VAPP: &[ObjectKind] = &[ObjectKind::Appliance];
    const ORGANIZE: &str = place::CONTEXT_ORGANIZE;
    vec![
        organization!(TAG_ADD, Action::AddTag, OBJECTS)
            .multi()
            .context_submenu(ORGANIZE),
        organization!(TAG_REMOVE, Action::RemoveTag, OBJECTS)
            .multi()
            .context_submenu(ORGANIZE),
        organization!(FOLDER_MOVE, Action::MoveFolder, OBJECTS)
            .multi()
            .context_submenu(ORGANIZE),
        organization!(FIELD_SET, Action::SetField, OBJECTS)
            .multi()
            .context_submenu(ORGANIZE),
        organization!(FIELD_CLEAR, Action::RemoveField, OBJECTS)
            .multi()
            .context_submenu(ORGANIZE),
        organization!(TAG_RENAME, Action::RenameTag, TAG),
        organization!(TAG_DELETE, Action::DeleteTag, TAG),
        // With the five above in the menu of a pool; alone, and so a plain
        // item, in the menu of a folder.
        organization!(
            FOLDER_CREATE,
            Action::CreateFolder,
            &[ObjectKind::Folder, ObjectKind::Pool]
        )
        .context_submenu(ORGANIZE),
        organization!(FOLDER_RENAME, Action::RenameFolder, FOLDER),
        organization!(FOLDER_DELETE, Action::DeleteFolder, FOLDER),
        CommandSpec::new(VAPPS.0, "vApps", AREA)
            .menu(place::POOL_VAPPS)
            .context(&[ObjectKind::Pool, ObjectKind::Appliance])
            .eligibility(|m, target| match appliance::capture_for(m, target, false) {
                Some(_) => Eligibility::Enabled,
                None => Eligibility::Hidden,
            })
            .invoke(|m, target| {
                if let Some(w) = appliance::capture_for(m, target, false) {
                    appliance::open(m, w);
                }
            }),
        CommandSpec::new(ASSIGN_TO_VAPP.0, "Assign to vApp", AREA)
            .menu(place::VM_ASSIGN_VAPP)
            .context(&[ObjectKind::Vm])
            .eligibility(can_assign)
            .invoke(|m, target| {
                if let Some(w) = appliance::capture_for(m, target, true) {
                    appliance::open(m, w);
                }
            }),
        CommandSpec::new(VAPP_START.0, "Start vApp", AREA)
            .context(VAPP)
            .eligibility(|m, target| can_power(m, target, AppliancePower::Start))
            .invoke(|m, target| power(m, target, AppliancePower::Start)),
        CommandSpec::new(VAPP_SHUT_DOWN.0, "Shut down vApp", AREA)
            .context(VAPP)
            .eligibility(|m, target| can_power(m, target, AppliancePower::CleanShutdown))
            .invoke(|m, target| power(m, target, AppliancePower::CleanShutdown)),
        CommandSpec::new(VAPP_FORCE_SHUT_DOWN.0, "Force shutdown", AREA)
            .context(VAPP)
            .eligibility(|m, target| can_power(m, target, AppliancePower::HardShutdown))
            .invoke(|m, target| power(m, target, AppliancePower::HardShutdown)),
    ]
}

fn eligibility(m: &Model, target: &Target, action: Action) -> Eligibility {
    match edit::capture(m, target, action, false) {
        Some(w) => Eligibility::enabled_unless(edit::blocked(&w)),
        None => Eligibility::Hidden,
    }
}

/// A VM whose vApp is unknown can be neither added nor removed: no review
/// of the dialog would pass for it.
fn can_assign(m: &Model, target: &Target) -> Eligibility {
    let Some(w) = appliance::capture_for(m, target, true) else {
        return Eligibility::Hidden;
    };
    let known = w
        .assign
        .and_then(|vm| w.inventory.details.vms.get(&vm))
        .is_some_and(|details| details.appliance_known);
    Eligibility::enabled_if(known, NO_MEMBERSHIP)
}

fn vapp(m: &Model, target: &Target) -> Option<appliance::Workspace> {
    target.single_of(ObjectKind::Appliance)?;
    appliance::capture_for(m, target, false)
}

fn can_power(m: &Model, target: &Target, action: AppliancePower) -> Eligibility {
    match vapp(m, target) {
        Some(w) => Eligibility::enabled_unless(appliance::power_plan(&w, action).err()),
        None => Eligibility::Hidden,
    }
}

/// Opens the vApps dialog on the review of the power operation.
fn power(m: &mut Model, target: &Target, action: AppliancePower) {
    if let Some(w) = vapp(m, target) {
        appliance::open(m, w);
        appliance::stage_power(m, action);
    }
}
