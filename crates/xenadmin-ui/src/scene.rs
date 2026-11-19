//! Scenes: named model states the scene tool renders and tests start from.
//!
//! A scene sets the model up exactly as the Rune scene function did. The
//! scene tool then renders whatever the model describes. All Rune scene
//! names stay; [`unported`] lists the ones no area has registered yet, so
//! progress is countable and nothing is forgotten.
//!
//! Each area exports `fn scenes() -> Vec<SceneSpec>` through its
//! [`crate::areas::AreaSpec`]; [`registry`] concatenates them after the
//! kernel's own.

use std::sync::OnceLock;

use serde_json::json;

use crate::command;
use crate::model::Model;
use crate::selection::MenuOrigin;
use crate::surface;

/// One scene.
#[derive(Clone, Copy, Debug)]
pub struct SceneSpec {
    pub name: &'static str,
    /// Sets up a model that starts as [`Model::fixture`].
    pub apply: fn(&mut Model),
    pub area: &'static str,
}

impl SceneSpec {
    pub const fn new(name: &'static str, area: &'static str, apply: fn(&mut Model)) -> Self {
        Self { name, apply, area }
    }
}

/// The 128 scenes of the Rune application (the `AppSpec.scenes` of its
/// `main.rs`, removed with it), in its order. The record of what had to be
/// carried over: every name is registered here or listed as not ported.
pub const RUNE_APP_SCENES: [&str; 128] = [
    "demo",
    "large-inventory",
    "host-version-notice",
    "interface-large",
    "inventory-context",
    "general-information",
    "options",
    "help",
    "console-details",
    "batch-power",
    "batch-properties",
    "batch-delete",
    "lifecycle-permissions",
    "batch-move",
    "batch-cross-pool",
    "batch-export",
    "snapshot-batch-export",
    "console-recovery",
    "console-frame",
    "console-actual-size",
    "console-typing-review",
    "multiple-consoles",
    "guest-connections",
    "external-tools",
    "host-console",
    "search-pools",
    "performance",
    "metric-presets",
    "unavailable-actions",
    "vm-devices",
    "properties",
    "properties-running",
    "vm-create",
    "device-properties",
    "home-host",
    "firmware-create",
    "creation-cloud",
    "creation-cloud-pending",
    "secure-boot",
    "vm-migration",
    "vm-recovery",
    "vm-recovery-force",
    "cross-pool",
    "archive-export",
    "ha-policy",
    "failure-help",
    "checkpoint-export",
    "xva-compression",
    "cloud-init",
    "cloud-init-review",
    "ovf-import",
    "ovf-export",
    "disk-export",
    "snapshots",
    "snapshot-create",
    "snapshot-ancestry",
    "snapshot-information",
    "snapshot-restore",
    "snapshot-edit",
    "snapshot-capture",
    "connections",
    "connection-trust",
    "certificate-review",
    "network-interfaces",
    "network-services",
    "network-fabric",
    "network-fabric-pool",
    "network-fabric-pool-select",
    "network-sriov",
    "network-ipv6",
    "network-topology",
    "management-network",
    "management-dhcp",
    "management-vlan",
    "management-bond",
    "management-ipv6-bond",
    "management-bond-guests",
    "management-vlan-bond",
    "host-administration",
    "host-memory",
    "host-monitoring",
    "local-conversion",
    "local-conversion-review",
    "host-monitoring-catalog",
    "host-support",
    "host-restore",
    "host-crashdump-delete",
    "host-power-settings",
    "snapshot-schedules",
    "snapshot-schedule-review",
    "alarms",
    "alarms-review",
    "alarms-email",
    "network-create",
    "activity",
    "organization",
    "organization-review",
    "appliances",
    "appliance-review",
    "disk-cleanup",
    "disk-grow",
    "disk-copy",
    "storage-create",
    "storage-review",
    "storage-local",
    "storage-local-review",
    "storage-hba",
    "storage-hba-review",
    "storage-repair",
    "storage-graph",
    "storage-multipath",
    "storage-maintenance",
    "storage-cache",
    "storage-discovery",
    "storage-auth-review",
    "pool-security",
    "pool-join",
    "root-password",
    "pool-dissolve",
    "guest-tools",
    "guest-tools-review",
    "pool-security-review",
    "pool-policies",
    "pool-policy-review",
    "pool-administration",
    "connection-settings",
    "connect",
    "connect-insecure",
];

/// The scene of the Rune console window (`console_windows::scene`), which is
/// registered on the child window and so is not in the application's list.
pub const CONSOLE_WINDOW_SCENE: &str = "console-window";

/// Scenes the Rune application registers in debug builds only.
pub const DEBUG_ONLY_SCENES: [&str; 1] = ["pool-join"];

/// Every Rune scene name: the application's 128 and `console-window`.
pub fn rune_scenes() -> impl Iterator<Item = &'static str> {
    RUNE_APP_SCENES
        .into_iter()
        .chain(std::iter::once(CONSOLE_WINDOW_SCENE))
}

/// Every registered scene: the kernel's, then each area's in the order of
/// [`crate::areas::specs`]. Built once.
pub fn registry() -> &'static [SceneSpec] {
    static REGISTRY: OnceLock<Vec<SceneSpec>> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        let mut scenes = crate::demo::scenes();
        for area in crate::areas::specs() {
            scenes.extend((area.scenes)());
        }
        scenes
    })
}

/// The registered scene with this name.
pub fn find(name: &str) -> Option<&'static SceneSpec> {
    registry().iter().find(|scene| scene.name == name)
}

/// Applies the scene to the model. False when no such scene is registered.
pub fn apply(name: &str, m: &mut Model) -> bool {
    match find(name) {
        Some(scene) => {
            (scene.apply)(m);
            true
        }
        None => false,
    }
}

/// The Rune scene names that are registered, in Rune order.
pub fn ported() -> Vec<&'static str> {
    rune_scenes().filter(|name| find(name).is_some()).collect()
}

/// The Rune scene names no area has registered yet, in Rune order.
pub fn unported() -> Vec<&'static str> {
    rune_scenes().filter(|name| find(name).is_none()).collect()
}

/// The element tree of a model as JSON: what the scene tool writes next to
/// the PNG. It holds the window title, the navigation view with its rows,
/// the selection, the object header, the tabs, the resolved menus, toolbar
/// and local action row, the flattened tab body, the open context menu
/// (`context_menu`, `null` when none is open), the status bar and every open
/// surface, back to front.
pub fn tree(m: &Model) -> serde_json::Value {
    let tab = m.active_tab();
    let body = m
        .tab_body(tab)
        .map(|body| surface::flatten_blocks("tab", &body.blocks()));
    let surfaces: Vec<_> = m
        .surfaces()
        .iter()
        .map(|surface| {
            json!({
                "id": surface.id,
                "title": surface.title,
                "experimental": surface.experimental,
                "size": surface.size,
                "nodes": surface::flatten(surface),
            })
        })
        .collect();
    let rows = crate::nav::rows(m);
    let selected = crate::nav::selected_ids(m, &rows);
    // The open context menu: where it was opened, on which row, what its
    // commands act on, whether it still stands, and its resolved items in
    // groups, each with the reason when it is disabled. `null` when none
    // is open.
    let context_menu = m.selection.menu.as_ref().map(|menu| {
        json!({
            "origin": match menu.origin {
                MenuOrigin::Tree => "tree",
                MenuOrigin::Grid => "grid",
            },
            "object": menu.object,
            "name": m.name_of(&menu.object),
            "target": menu.target.objects,
            "valid": m.context_menu_valid(),
            "items": m.context_menu(),
        })
    });
    json!({
        "title": m.window_title(),
        "demo": m.demo,
        "error": m.error,
        "interface_scale": m.preferences.options.interface_scale,
        "nav": {
            "view": m.nav.view,
            "filter": m.nav.filter,
            "rows": rows,
            "selected": selected,
        },
        "header": m.header(),
        "status": m.status(),
        "experimental_notice": m.experimental_notice(),
        "scope": m.selection.scope.objects,
        "target": m.target().objects,
        "tabs": m.tabs(),
        "active_tab": tab,
        "menus": command::menu_bar(m),
        "toolbar": command::toolbar(m),
        "local": command::local_row(m, tab),
        "tab": body,
        "context_menu": context_menu,
        "surfaces": surfaces,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn every_rune_scene_is_registered_or_listed_as_unported() {
        assert_eq!(rune_scenes().count(), 129);
        let rune: BTreeSet<_> = rune_scenes().collect();
        assert_eq!(rune.len(), 129, "a Rune scene name is listed twice");
        let ported = ported();
        let unported = unported();
        assert_eq!(ported.len() + unported.len(), 129);
        for name in rune_scenes() {
            let registered = find(name).is_some();
            assert_ne!(registered, unported.contains(&name), "{name}");
            assert_eq!(registered, ported.contains(&name), "{name}");
        }
        // The count the migration ledger tracks.
        println!("scenes ported: {} of 129", ported.len());
        println!("scenes not ported: {unported:?}");
    }

    #[test]
    fn registered_scene_names_are_unique_and_name_a_known_area() {
        let areas: Vec<_> = crate::areas::specs().iter().map(|area| area.id).collect();
        let mut seen = BTreeSet::new();
        for scene in registry() {
            assert!(
                seen.insert(scene.name),
                "{} is registered twice",
                scene.name
            );
            assert!(
                scene.area == command::KERNEL || areas.contains(&scene.area),
                "{} names the unknown area {}",
                scene.name,
                scene.area
            );
        }
    }

    #[test]
    fn the_rune_application_list_keeps_its_names() {
        assert_eq!(RUNE_APP_SCENES.len(), 128);
        assert!(RUNE_APP_SCENES.contains(&"demo"));
        assert!(RUNE_APP_SCENES.contains(&"pool-join"));
        assert!(!RUNE_APP_SCENES.contains(&CONSOLE_WINDOW_SCENE));
        assert!(
            DEBUG_ONLY_SCENES
                .iter()
                .all(|name| RUNE_APP_SCENES.contains(name))
        );
    }

    #[test]
    fn the_tree_of_a_scene_is_json_with_menus_and_selection() {
        let mut m = Model::fixture();
        assert!(apply("unavailable-actions", &mut m));
        let tree = tree(&m);
        assert_eq!(tree["demo"], json!(false));
        assert_eq!(tree["scope"][0]["kind"], json!("vm"));
        assert_eq!(tree["active_tab"], json!("general"));
        assert_eq!(tree["menus"].as_array().map(Vec::len), Some(10));
        assert_eq!(tree["menus"][0]["title"], json!("File"));
        assert!(tree["surfaces"].is_array());
        assert_eq!(tree["title"], json!("Database"));
        assert_eq!(tree["header"]["name"], json!("Database"));
        assert_eq!(tree["header"]["state"], json!("Running"));
        assert_eq!(tree["status"]["connection"], json!("No host connected"));
        assert_eq!(tree["nav"]["view"], json!("infrastructure"));
        assert_eq!(tree["nav"]["rows"][0]["label"], json!("Demonstration pool"));
        assert_eq!(tree["nav"]["selected"].as_array().map(Vec::len), Some(1));
        assert_eq!(tree["experimental_notice"], json!(false));
    }
}
