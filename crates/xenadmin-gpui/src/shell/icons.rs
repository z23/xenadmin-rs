//! Which icon stands for which object kind, navigation view and toolbar
//! command.
//!
//! All are Lucide icons. Those outside the Kit's default set are embedded by
//! [`crate::assets::AppAssets`]; a test below loads every icon named here.

use xenadmin_ui::command::{CommandId, ToolbarSlot};
use xenadmin_ui::nav::NavView;
use xenadmin_ui::object::ObjectKind;

use crate::kit::Lucide;

/// The icon of an object of this kind: a tree row, the object header.
pub fn kind_icon(kind: ObjectKind) -> Lucide {
    match kind {
        ObjectKind::Pool => Lucide::Boxes,
        ObjectKind::Connection => Lucide::Unplug,
        ObjectKind::Host => Lucide::Server,
        ObjectKind::Vm => Lucide::Monitor,
        ObjectKind::Template => Lucide::FileStack,
        ObjectKind::StorageRepository => Lucide::Database,
        ObjectKind::VirtualDisk => Lucide::HardDrive,
        ObjectKind::Network => Lucide::Network,
        ObjectKind::Snapshot => Lucide::Camera,
        ObjectKind::Appliance => Lucide::Layers,
        ObjectKind::Folder => Lucide::Folder,
        ObjectKind::Tag => Lucide::Tag,
        ObjectKind::SavedSearch => Lucide::Search,
        ObjectKind::Multi => Lucide::Shapes,
        ObjectKind::None => Lucide::Inbox,
    }
}

/// The icon of a navigation view in the view switcher.
pub fn view_icon(view: NavView) -> Lucide {
    match view {
        NavView::Infrastructure => Lucide::ListTree,
        NavView::Objects => Lucide::Shapes,
        NavView::Organization => Lucide::FolderTree,
        NavView::SavedSearches => Lucide::Search,
        NavView::Notifications => Lucide::Bell,
    }
}

/// The icon of a toolbar slot of the plan. A command in a slot the plan
/// does not name has none and shows its label only.
pub fn slot_icon(slot: ToolbarSlot) -> Option<Lucide> {
    const ICONS: [(ToolbarSlot, Lucide); 12] = [
        (ToolbarSlot::BACK, Lucide::ArrowLeft),
        (ToolbarSlot::FORWARD, Lucide::ArrowRight),
        (ToolbarSlot::ADD_SERVER, Lucide::ServerPlus),
        (ToolbarSlot::NEW_STORAGE, Lucide::DatabasePlus),
        (ToolbarSlot::NEW_VM, Lucide::SquarePlus),
        (ToolbarSlot::START, Lucide::Play),
        (ToolbarSlot::SHUT_DOWN, Lucide::Power),
        (ToolbarSlot::REBOOT, Lucide::RotateCw),
        (ToolbarSlot::RESUME, Lucide::StepForward),
        (ToolbarSlot::SUSPEND, Lucide::Pause),
        (ToolbarSlot::FORCE_SHUT_DOWN, Lucide::PowerOff),
        (ToolbarSlot::FORCE_REBOOT, Lucide::Zap),
    ];
    ICONS
        .into_iter()
        .find_map(|(known, icon)| (known == slot).then_some(icon))
}

/// The icon of a toolbar command, by the slot the registry gives it.
pub fn command_icon(id: CommandId) -> Option<Lucide> {
    xenadmin_ui::command::find(id)
        .and_then(|spec| spec.toolbar)
        .and_then(slot_icon)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::AppAssets;
    use crate::kit::AssetSource as _;

    const KINDS: [ObjectKind; 15] = [
        ObjectKind::None,
        ObjectKind::Pool,
        ObjectKind::Host,
        ObjectKind::Vm,
        ObjectKind::Template,
        ObjectKind::StorageRepository,
        ObjectKind::Network,
        ObjectKind::VirtualDisk,
        ObjectKind::Snapshot,
        ObjectKind::Appliance,
        ObjectKind::Folder,
        ObjectKind::Tag,
        ObjectKind::SavedSearch,
        ObjectKind::Connection,
        ObjectKind::Multi,
    ];

    const SLOTS: [ToolbarSlot; 12] = [
        ToolbarSlot::BACK,
        ToolbarSlot::FORWARD,
        ToolbarSlot::ADD_SERVER,
        ToolbarSlot::NEW_STORAGE,
        ToolbarSlot::NEW_VM,
        ToolbarSlot::START,
        ToolbarSlot::SHUT_DOWN,
        ToolbarSlot::REBOOT,
        ToolbarSlot::RESUME,
        ToolbarSlot::SUSPEND,
        ToolbarSlot::FORCE_SHUT_DOWN,
        ToolbarSlot::FORCE_REBOOT,
    ];

    #[test]
    fn every_icon_the_shell_names_is_served_by_the_application_assets() {
        let icons = KINDS
            .into_iter()
            .map(kind_icon)
            .chain(NavView::ALL.into_iter().map(view_icon))
            .chain(SLOTS.into_iter().map(|slot| {
                slot_icon(slot).unwrap_or_else(|| panic!("no icon for the slot {slot:?}"))
            }));
        for icon in icons {
            let path = icon.path();
            let loaded = AppAssets.load(&path);
            assert!(
                matches!(loaded, Ok(Some(_))),
                "{path} is not in the application assets"
            );
        }
    }
}
