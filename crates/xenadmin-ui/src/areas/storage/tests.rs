//! The tests of the area, in three parts.
//!
//! 1. **The Rune tests**: the six of `storage_view.rs`, the five of
//!    `storage_local_view.rs`, the four of
//!    `storage_provisioning_view_tests.rs`, under their Rune names, and this
//!    area's share of two tests of other Rune files that ran over its
//!    scenes. Each asserts what the Rune test asserted. Where the Rune test
//!    found a control by its key and pressed it, the test here does the
//!    same through the harness; "inside the window at both sizes" is the
//!    front end's to show (`notes/progress/gpui-tests-wanted.md`).
//! 2. **What the port changed**: the dialog became a wizard, the hub became
//!    commands, the Storage tab of a pool and a server is a grid of objects.
//! 3. **Hazards** of the map (section 11) that no Rune test pinned.
//!
//! No fixture has a current snapshot, so no test submits successfully: the
//! refusals are tested.

use std::sync::Arc;

use uuid::Uuid;
use xenadmin_core::{
    DiscoveredIscsiLun, DiscoveredLocalRepository, DiscoveredRepository, Error, HostId, Inventory,
    LocalStorageKind, NfsVersion, PbdId, PoolId, ResourceTarget, SrId, StorageAllocation,
    StorageAttachment, StorageBackend, StorageDiscoveryResult, StorageLocation, StorageOperation,
};

use super::commands::{
    ATTACHED, NO_ATTACHMENT, NOT_ATTACHED, ONLY_DETACHED, SELECT, SEVERAL, UNKNOWN_STATE,
};
use super::editor::{self, Editor, Mode};
use super::view::{self, DEMO, NOT_WRITABLE};
use crate::Harness;
use crate::prelude::*;

// -- Fixtures -----------------------------------------------------------------------

#[track_caller]
fn editor(h: &Harness) -> &Editor {
    h.model.areas.storage.editor.as_ref().expect("an editor")
}

#[track_caller]
fn editor_mut(h: &mut Harness) -> &mut Editor {
    h.model.areas.storage.editor.as_mut().expect("an editor")
}

#[track_caller]
fn reviewed(h: &Harness) -> &StorageOperation {
    &editor(h).review.as_ref().expect("a review").2
}

fn inventory(h: &Harness) -> Arc<Inventory> {
    h.model.inventory.clone().expect("inventory")
}

fn pool(h: &Harness) -> PoolId {
    inventory(h).pool_id
}

fn amend(h: &mut Harness, change: impl FnOnce(&mut Inventory)) {
    let mut inventory = inventory(h).as_ref().clone();
    change(&mut inventory);
    h.model.inventory = Some(Arc::new(inventory));
}

fn step(h: &Harness) -> usize {
    match h.surface().kind {
        SurfaceKind::Wizard { current, .. } => current,
        _ => panic!("not a wizard"),
    }
}

/// The demo pool with nothing open, where the first repository has one
/// attachment on each of the three servers: attached, detached, unknown.
struct Attached {
    h: Harness,
    pool: PoolId,
    repository: Uuid,
    hosts: Vec<HostId>,
    pbds: Vec<PbdId>,
}

impl Attached {
    fn repository(&self) -> Target {
        Target::one(ObjectRef::storage(self.pool, self.repository))
    }

    fn host(&self, n: usize) -> Target {
        Target::one(ObjectRef::host(self.pool, self.hosts[n]))
    }
}

fn attached() -> Attached {
    let mut h = Harness::scene("storage-create");
    h.dismiss();
    assert!(h.surfaces().is_empty());
    let i = inventory(&h);
    let repository = i.storage.first().expect("a repository").uuid;
    let hosts: Vec<HostId> = i.hosts.keys().copied().take(3).collect();
    assert_eq!(hosts.len(), 3);
    let pbds: Vec<PbdId> = (0..3)
        .map(|n| PbdId::from(Uuid::from_u128(0x5000 + n)))
        .collect();
    amend(&mut h, |i| {
        i.details.pbds.clear();
        for n in 0..3 {
            i.details.pbds.insert(
                pbds[n],
                StorageAttachment {
                    id: pbds[n],
                    location: Some(StorageLocation::Nfs {
                        server: "nfs.example.test".into(),
                        path: "/exports/demo".into(),
                        version: NfsVersion::V4,
                    }),
                    requires_credentials: false,
                    multipathed: None,
                    paths: Vec::new(),
                    host: Some(hosts[n]),
                    sr: Some(SrId::from(repository)),
                    currently_attached: [Some(true), Some(false), None][n],
                },
            );
        }
    });
    Attached {
        pool: pool(&h),
        h,
        repository,
        hosts,
        pbds,
    }
}

// -- 1. The Rune tests ---------------------------------------------------------------

/// Rune `storage_view.rs`. The model half; the GPUI half is the confirm
/// button inside the window at 513x652 and 1440x900.
#[test]
fn authenticated_storage_review_is_captured_and_footer_remains_reachable() {
    let mut h = Harness::scene("storage-auth-review");
    let e = editor_mut(&mut h);
    assert!(e.review_credentials.is_some());
    e.server = "changed.example.test".into();
    e.username = "changed".into();
    assert!(
        e.review_auth_label
            .as_ref()
            .unwrap()
            .contains("LAB\\fixture")
    );
    assert!(
        matches!(reviewed(&h), StorageOperation::Create { location: StorageLocation::IsoSmb { server, .. }, .. } if server == "smb.example.test")
    );
    assert!(h.has_text("SMB 3.0 authentication as LAB\\fixture"));
    assert!(h.find("storage-confirm").is_some());
    assert_eq!(h.try_click("storage-confirm"), Err(DEMO.into()));
    assert!(h.model.resource_operations.is_empty());
}

/// Rune `storage_view.rs`.
#[test]
fn storage_discovery_result_fence_includes_credential_changes() {
    let mut h = Harness::scene("storage-discovery");
    let e = editor_mut(&mut h);
    e.authenticate = true;
    e.query_inputs = Some(editor::inputs(e));
    e.auth_epoch += 1;
    editor::select_result(&mut h.model, 0);
    assert_eq!(editor(&h).server, "storage.example.test");
    assert!(editor(&h).error.is_some());
    let mut h = Harness::scene("storage-create");
    let e = editor_mut(&mut h);
    e.kind = 3;
    e.authenticate = true;
    e.share = "images".into();
    editor::review(&mut h.model);
    assert!(editor(&h).review.is_none());
    assert_eq!(
        editor(&h).error.as_deref(),
        Some("Enter the storage username and password before review")
    );
}

/// Rune `storage_view.rs`. The GPUI half: `storage-result-0` and
/// `storage-review` inside the window at both sizes.
#[test]
fn discovery_selection_is_reachable_bound_to_inputs_and_keeps_review_separate() {
    let mut h = Harness::scene("storage-discovery");
    assert!(h.find("storage-review").is_none(), "results page: only Close");
    h.click("storage-result-0");
    let e = editor(&h);
    assert_eq!(e.server, "192.0.2.50");
    assert_eq!(e.port, "3261");
    assert!(e.review.is_none());
    assert!(e.discovered.is_none());
    assert!(h.model.resource_operations.is_empty());
    assert!(h.find("storage-review").is_some());

    let mut h = Harness::scene("storage-discovery");
    editor_mut(&mut h).server = "changed.example.test".into();
    assert!(h.find("storage-result-0").is_none(), "stale results");
    assert!(h.has_text("Inputs changed; discover again before selecting a result."));
    editor::select_result(&mut h.model, 0);
    assert_eq!(editor(&h).server, "changed.example.test");
    let mut h = Harness::scene("storage-discovery");
    editor_mut(&mut h).port = "0".into();
    editor::review(&mut h.model);
    assert!(editor(&h).review.is_none());
    assert!(editor(&h).error.as_ref().unwrap().contains("65535"));
}

/// Rune `storage_view.rs`.
#[test]
fn chosen_lun_requires_separate_destructive_review() {
    let mut h = Harness::scene("storage-discovery");
    let e = editor_mut(&mut h);
    e.iqn = "iqn.2026-11.example:test".into();
    e.query_inputs = Some(editor::inputs(e));
    e.discovered = Some(StorageDiscoveryResult::IscsiLuns(
        (0..25)
            .map(|i| DiscoveredIscsiLun {
                scsi_id: format!("3600fixture{i}"),
                lun: Some(i),
                size_bytes: Some(4096),
                vendor: "Fixture".into(),
                serial: String::new(),
            })
            .collect(),
    ));
    // Twenty rows to a page: the twenty-fifth is on the second.
    assert!(h.find("storage-result-24").is_none());
    h.click("storage-results-next");
    h.click("storage-result-24");
    let e = editor(&h);
    assert_eq!(e.scsi, "3600fixture24");
    assert!(e.review.is_none());
    assert!(h.model.resource_operations.is_empty());
    h.click("storage-review");
    assert!(
        matches!(reviewed(&h), StorageOperation::Create { location: StorageLocation::Iscsi { port: 3261, scsi_id, .. }, .. } if scsi_id == "3600fixture24")
    );
    assert!(
        editor::describe(reviewed(&h))
            .iter()
            .any(|l| l.contains("erase existing data"))
    );
}

/// Rune `storage_view.rs`. The Rune test wrote the fields and pressed
/// Review; here the user walks the wizard. The GPUI half: `storage-review`
/// and `storage-confirm` inside the window at both sizes.
#[test]
fn storage_review_preserves_inputs_and_compact_footer_never_submits_preview() {
    let mut h = Harness::scene("demo");
    h.invoke("storage.new");
    assert_eq!(h.surface().id, view::WIZARD);
    assert_eq!(step(&h), 0);
    assert!(h.find("storage-back").is_none());
    h.click("storage-next");
    assert_eq!(step(&h), 1);
    h.type_text("storage-name", "Fixture NFS");
    h.click("storage-next");
    assert_eq!(step(&h), 2);
    h.type_text("storage-server", "nfs.example.test");
    h.type_text("storage-nfs-path", "/images");
    h.click("storage-review");
    assert_eq!(step(&h), 3);
    let e = editor(&h);
    let (_, target, op) = e.review.as_ref().unwrap();
    assert_eq!(*target, ResourceTarget::Pool(e.pool));
    assert!(matches!(op, StorageOperation::Create { name, .. } if name == "Fixture NFS"));
    assert_eq!(h.try_click("storage-confirm"), Err(DEMO.into()));
    assert!(h.model.resource_operations.is_empty());
    // Back keeps the draft and drops the review.
    h.click("storage-back");
    assert_eq!(step(&h), 2);
    assert!(editor(&h).review.is_none());
    assert_eq!(editor(&h).name, "Fixture NFS");
}

/// Rune `storage_view.rs`. The Rune test opened the hub on the first row
/// of the Storage page and pressed its buttons; here the commands open the
/// reviews. Its last assertion (the target survives a page change) is the
/// tab change here; selecting another object closes the dialog, which
/// replaces the Rune behaviour.
#[test]
fn storage_forget_and_destroy_have_distinct_review_and_exact_disk_set() {
    let mut h = Harness::scene("demo");
    let i = inventory(&h);
    let repository = i.storage.first().expect("a repository").uuid;
    let sr = SrId::from(repository);
    h.select(Target::one(ObjectRef::storage(i.pool_id, repository)));
    h.invoke("storage.forget");
    let target = editor(&h).target;
    assert_eq!(target, ResourceTarget::Storage(sr));
    assert!(matches!(reviewed(&h), StorageOperation::Forget { .. }));
    assert_eq!(h.surface().id, view::CHANGE);
    assert!(h.find("storage-back").is_none(), "no draft to go back to");
    editor::removal(&mut h.model, true);
    assert_eq!(editor(&h).target, target);
    let disks = i.details.sr_disks.get(&sr).cloned().unwrap_or_default();
    assert!(
        matches!(reviewed(&h), StorageOperation::Destroy { expected_disks, .. } if *expected_disks == disks)
    );
    assert!(
        editor::describe(reviewed(&h))
            .iter()
            .any(|s| s.contains("PERMANENTLY DESTROYS"))
    );
    assert!(h.find("storage-confirm-uuid").is_some());
    h.model.set_active_tab(TabId::General);
    assert_eq!(editor(&h).target, target);
    h.select(Target::one(ObjectRef::pool(i.pool_id)));
    assert!(h.model.areas.storage.editor.is_none());
}

/// Rune `storage_local_view.rs`. The GPUI half: `storage-review`,
/// `storage-confirm-device` and `storage-confirm` inside the window at
/// both sizes.
#[test]
fn local_review_captures_operator_declaration_host_and_device_and_keeps_confirmation_reachable() {
    let mut h = Harness::scene("storage-local");
    let original = editor(&h).host.unwrap();
    h.click("storage-review");
    assert!(editor(&h).review.is_some(), "{:?}", editor(&h).error);
    let captured = editor(&h).review.clone().unwrap();
    assert!(
        matches!(&captured.2, StorageOperation::Create { host, location: StorageLocation::Local { device, .. }, local_device: Some(d), .. } if *host == original && device.ends_with("001") && d.identity_statement.contains("Fixture"))
    );
    let e = editor_mut(&mut h);
    e.host = None;
    e.path = "/dev/disk/by-id/ata-CHANGED".into();
    e.declaration = "changed".into();
    assert_eq!(editor(&h).review.as_ref(), Some(&captured));
    assert!(h.find("storage-confirm-device").is_some());
    assert!(h.try_click("storage-confirm").is_err());
    assert!(h.model.resource_operations.is_empty());
    let lines = editor::describe(&captured.2).join("\n");
    for text in [
        "PERMANENTLY ERASES",
        "Operator identity declaration",
        "does not fully verify",
        "automatically forgotten",
    ] {
        assert!(lines.contains(text), "{text}: {lines}");
        assert!(h.has_text(text), "{text} on the review");
    }
}

/// Rune `storage_local_view.rs`.
#[test]
fn local_review_requires_current_dual_probe_and_explicit_operator_acknowledgement() {
    for case in 0..6 {
        let mut h = Harness::scene("storage-local");
        let e = editor_mut(&mut h);
        match case {
            0 => e.local_probe = None,
            1 => e.host = None,
            2 => e.path = "/dev/disk/by-id/ata-CHANGED".into(),
            3 => e.local_acknowledged = false,
            4 => e.declaration = String::new(),
            _ => {}
        };
        if case == 5 {
            editor::query(&mut h.model, 6);
        }
        editor::review(&mut h.model);
        assert!(editor(&h).review.is_none(), "case {case}");
        assert!(h.model.resource_operations.is_empty());
    }
}

/// Rune `storage_local_view.rs`.
#[test]
fn local_known_repository_selection_chooses_nonformatting_introduction() {
    let mut h = Harness::scene("storage-local");
    let e = editor_mut(&mut h);
    e.query_inputs = Some(editor::inputs(e));
    e.discovered = Some(StorageDiscoveryResult::LocalRepositories(vec![
        DiscoveredLocalRepository {
            kind: LocalStorageKind::Lvm,
            repository: DiscoveredRepository {
                uuid: SrId::from(Uuid::from_u128(701)),
                name: "Existing local".into(),
                size_bytes: Some(4096),
            },
        },
    ]));
    h.click("storage-result-0");
    let e = editor(&h);
    assert!(e.mode == Mode::Introduce);
    assert_eq!(e.kind, 6);
    assert!(e.review.is_none());
    editor::review(&mut h.model);
    assert!(matches!(
        reviewed(&h),
        StorageOperation::Introduce {
            location: StorageLocation::Local {
                kind: LocalStorageKind::Lvm,
                ..
            },
            ..
        }
    ));
    assert!(h.model.resource_operations.is_empty());
}

/// Rune `storage_local_view.rs`. The GPUI half: the two controls inside
/// the window at both sizes.
#[test]
fn local_form_exposes_declaration_and_erase_acknowledgement_at_both_widths() {
    let mut h = Harness::scene("storage-local");
    assert!(h.find("storage-local-identity").is_some());
    h.click("storage-local-ack");
    assert!(!editor(&h).local_acknowledged);
    h.click("storage-review");
    assert!(editor(&h).review.is_none());
    h.click("storage-local-ack");
    h.click("storage-review");
    assert!(editor(&h).review.is_some());
}

/// Rune `storage_local_view.rs`. Focus is not an event of the model: the
/// probe is kept until the text changes.
#[test]
fn local_path_focus_preserves_probe_but_text_edit_invalidates_it() {
    let mut h = Harness::scene("storage-local");
    assert!(h.find("storage-local-device").is_some());
    assert!(editor(&h).local_probe.is_some());
    assert!(h.find("storage-local-probe").is_some());
    let changed = format!("{}-changed", editor(&h).path);
    h.type_text("storage-local-device", &changed);
    assert!(editor(&h).local_probe.is_none());
    assert!(!editor(&h).local_acknowledged);
    h.click("storage-review");
    assert!(editor(&h).review.is_none());
}

/// Rune `storage_provisioning_view_tests.rs`. The GPUI half: the result,
/// Review, the LUN confirmation and Confirm inside the window at both
/// sizes.
#[test]
fn hba_selection_requires_review_and_captures_host_device_and_allocation_at_both_widths() {
    let mut h = Harness::scene("storage-hba");
    let original_host = editor(&h).host.unwrap();
    h.click("storage-result-0");
    assert!(editor(&h).review.is_none());
    assert!(h.model.resource_operations.is_empty());
    h.click("storage-review");
    let captured = editor(&h).review.clone().unwrap();
    assert!(
        matches!(&captured.2, StorageOperation::Create { host, location: StorageLocation::Hba { allocation: StorageAllocation::Thin, .. }, hba_device: Some(device), .. } if *host == original_host && device.serial == "LAB-2026-001")
    );
    let e = editor_mut(&mut h);
    e.host = None;
    e.scsi = "changed".into();
    e.allocation = StorageAllocation::Thick;
    assert_eq!(editor(&h).review.as_ref(), Some(&captured));
    assert!(h.find("storage-confirm").is_some());
    let confirmation = h.node("storage-confirm-lun");
    assert!(
        confirmation
            .label
            .as_deref()
            .is_some_and(|label| label.contains("confirm formatting"))
    );
    assert!(h.try_click("storage-confirm").is_err());
    assert!(h.model.resource_operations.is_empty());
}

/// Rune `storage_provisioning_view_tests.rs`.
#[test]
fn hba_review_refuses_unselected_device_changed_host_and_unadvertised_allocation() {
    let mut h = Harness::scene("storage-hba");
    editor_mut(&mut h).discovered = None;
    editor::review(&mut h.model);
    assert!(editor(&h).review.is_none());
    let mut h = Harness::scene("storage-hba");
    editor::select_result(&mut h.model, 0);
    editor_mut(&mut h).host = None;
    editor::review(&mut h.model);
    assert!(editor(&h).review.is_none());
    let mut h = Harness::scene("storage-hba");
    editor::select_result(&mut h.model, 0);
    editor_mut(&mut h).backends.clear();
    editor::review(&mut h.model);
    assert!(editor(&h).review.is_none());
    editor_mut(&mut h).allocation = StorageAllocation::DriverDefault;
    editor::review(&mut h.model);
    assert!(editor(&h).review.is_some());
}

/// Rune `storage_provisioning_view_tests.rs`.
#[test]
fn hba_discovery_results_cannot_be_selected_after_host_or_connection_changes() {
    let mut h = Harness::scene("storage-hba");
    editor_mut(&mut h).host = None;
    editor::select_result(&mut h.model, 0);
    assert!(editor(&h).hba_device.is_none());
    let mut h = Harness::scene("storage-hba");
    h.model.demo = false;
    editor::select_result(&mut h.model, 0);
    assert!(editor(&h).hba_device.is_none());
}

/// Rune `storage_provisioning_view_tests.rs`.
#[test]
fn hba_review_explains_external_ownership_destructive_format_and_no_automatic_cleanup() {
    let h = Harness::scene("storage-hba-review");
    let lines = editor::describe(reviewed(&h)).join("\n");
    for text in [
        "permanently erase",
        "other pool or system",
        "Thin",
        "automatically forgotten",
        "2048.0 GiB",
    ] {
        assert!(lines.contains(text), "{text}: {lines}");
    }
}

/// This area's share of Rune `main.rs`
/// `dialogs_outside_the_supported_core_carry_the_experimental_notice`:
/// storage administration is outside the supported part, in every shape.
#[test]
fn every_storage_surface_carries_the_experimental_notice() {
    for scene in super::scenes::scenes() {
        let h = Harness::scene(scene.name);
        assert!(h.model.experimental_notice(), "{}", scene.name);
        assert!(h.find(EXPERIMENTAL_NOTICE_KEY).is_some(), "{}", scene.name);
        h.assert_unique_keys();
    }
    let mut a = attached();
    let target = a.repository();
    for command in ["storage.trim", "storage.attachment.create", "storage.forget"] {
        a.h.invoke_on(command, &target);
        assert!(a.h.model.experimental_notice(), "{command}");
        a.h.dismiss();
    }
}

/// This area's share of Rune `session_permissions_view.rs`
/// `review_permission_notice_is_reachable_without_bypassing_synthetic_target_guard`
/// (its `storage` scene is `storage-review`).
#[test]
fn review_permission_notice_is_reachable_without_bypassing_synthetic_target_guard() {
    let mut h = Harness::scene("storage-review");
    assert!(
        h.find(permissions::REVIEW_KEY).is_some(),
        "captured review must explain permission discovery"
    );
    assert_eq!(h.try_click("storage-confirm"), Err(DEMO.into()));
    assert!(h.model.resource_operations.is_empty());
    assert!(h.model.error.is_none());
    // Before the review there is no operation to assess, so no line.
    let h = Harness::scene("storage-create");
    assert!(h.find(permissions::REVIEW_KEY).is_none());
}

// -- 2. What the port changed -----------------------------------------------------------

#[test]
fn new_storage_is_in_the_menu_the_toolbar_and_the_context_menus_of_pool_and_server() {
    let a = attached();
    let h = &a.h;
    let item = h.menu_item("storage.new").expect("Storage > New Storage");
    assert_eq!(item.label, "New Storage");
    assert!(item.disabled.is_none());
    assert!(h.toolbar().iter().any(|item| item.id.0 == "storage.new"));
    for target in [Target::one(ObjectRef::pool(a.pool)), a.host(0)] {
        let menu = h.context_menu(&target);
        assert!(menu.iter().any(|item| item.id.0 == "storage.new"));
    }
    // Not in the local row: that row also shows on a VM's Storage tab.
    assert!(h.local_row().iter().all(|item| item.id.0 != "storage.new"));
    // No pool at all: the slot stays, disabled with its reason.
    let h = Harness::new();
    assert_eq!(
        h.eligibility("storage.new"),
        Eligibility::Disabled(super::commands::NO_POOL.into())
    );
}

#[test]
fn the_wizard_opens_read_only_and_names_its_pool() {
    let mut a = attached();
    a.h.model.demo = false;
    a.h.invoke("storage.new");
    let surface = a.h.surface();
    assert_eq!(surface.id, view::WIZARD);
    assert_eq!(surface.title, "New Storage Repository");
    assert!(surface.subtitle.unwrap().contains(&a.pool.to_string()));
    // Outside the demo a failed discovery says so.
    assert_eq!(
        editor(&a.h).error.as_deref(),
        Some("Connect to discover storage")
    );
    assert!(a.h.find("storage-preview").is_none());
    assert!(!editor::poll(&mut a.h.model, 0.0), "nothing arrived");
}

#[test]
fn the_storage_tab_of_a_pool_lists_its_repositories_as_objects() {
    let mut a = attached();
    let (pool, repository) = (a.pool, a.repository);
    let h = &mut a.h;
    h.select(Target::one(ObjectRef::pool(pool)));
    h.open_tab(TabId::Storage);
    h.assert_unique_keys();
    assert!(h.find("storage-repositories-table").is_some());
    assert!(h.has_text("Demo storage"));
    // Nothing selected: the commands of the grid say what they need.
    for command in ["storage.forget", "storage.destroy", "storage.detach"] {
        assert_eq!(h.eligibility(command), Eligibility::Disabled(SELECT.into()));
    }
    h.select_row("storage-repositories-table", &repository.to_string());
    let object = ObjectRef::storage(pool, repository);
    assert_eq!(h.model.selection.grid, Target::one(object.clone()));
    assert_eq!(h.eligibility("storage.forget"), Eligibility::Enabled);
    assert_eq!(h.eligibility("storage.destroy"), Eligibility::Enabled);
    // Three attachments and no server to say which.
    assert_eq!(
        h.eligibility("storage.detach"),
        Eligibility::Disabled(SEVERAL.into())
    );
    let local: Vec<_> = h.local_row().into_iter().map(|item| item.id.0).collect();
    for command in [
        "storage.trim",
        "storage.detach",
        "storage.reattach",
        "storage.attachment.create",
        "storage.attachment.remove",
        "storage.forget",
        "storage.destroy",
    ] {
        assert!(local.contains(&command), "{command} in {local:?}");
    }
    // The commands of `storage_maintenance` and `network` on a repository
    // are on the same row, before this area's own, and in the row's menu.
    let at = |command: &str| {
        local
            .iter()
            .position(|id| *id == command)
            .unwrap_or_else(|| panic!("{command} in {local:?}"))
    };
    assert!(at("storage.repair") < at("storage.set-default"));
    assert!(at("storage.set-default") < at("storage.rescan"));
    assert!(at("storage.rescan") < at("storage.trim"));
    // The row is a tree object: the kernel's context menu.
    let menu = h.open_row_menu("storage-repositories-table", &repository.to_string());
    assert!(menu.iter().any(|item| item.id.0 == "storage.destroy"));
    for command in ["storage.repair", "storage.set-default", "storage.rescan"] {
        assert!(menu.iter().any(|item| item.id.0 == command), "{command}");
    }
    h.choose("storage.destroy");
    assert!(matches!(reviewed(h), StorageOperation::Destroy { .. }));
    h.dismiss();
    h.escape_in_table("storage-repositories-table");
    assert!(h.model.areas.storage.selected.is_none());
    assert!(h.model.selection.grid.is_empty());
    // Activating a row goes to the repository.
    h.activate_row("storage-repositories-table", &repository.to_string());
    assert_eq!(h.model.scope(), &Target::one(object));
}

#[test]
fn the_storage_tab_of_a_server_lists_its_attachments_and_says_which_one_a_command_acts_on() {
    let mut a = attached();
    let repository = a.repository();
    for n in 0..3 {
        let host = a.host(n);
        let row = a.pbds[n].to_string();
        let h = &mut a.h;
        h.select(host);
        h.open_tab(TabId::Storage);
        h.assert_unique_keys();
        assert!(h.find(&format!("storage-pbds:{row}")).is_some(), "server {n}");
        h.select_row("storage-pbds", &row);
        assert_eq!(h.model.selection.grid, repository);
        let (detach, reattach, remove) = (
            h.eligibility("storage.detach"),
            h.eligibility("storage.reattach"),
            h.eligibility("storage.attachment.remove"),
        );
        let disabled = |reason: &str| Eligibility::Disabled(reason.into());
        match n {
            0 => {
                assert_eq!(detach, Eligibility::Enabled);
                assert_eq!(reattach, disabled(ATTACHED));
                assert_eq!(remove, disabled(ONLY_DETACHED));
            }
            1 => {
                assert_eq!(detach, disabled(NOT_ATTACHED));
                assert_eq!(reattach, Eligibility::Enabled);
                assert_eq!(remove, Eligibility::Enabled);
            }
            _ => {
                assert_eq!(detach, disabled(UNKNOWN_STATE));
                assert_eq!(reattach, disabled(UNKNOWN_STATE));
                assert_eq!(remove, disabled(UNKNOWN_STATE));
            }
        }
    }
    // Detach on the first server: the review of that attachment, no Back.
    let (host, pbd, sr) = (a.hosts[0], a.pbds[0], SrId::from(a.repository));
    let target = a.host(0);
    let h = &mut a.h;
    h.select(target);
    h.open_tab(TabId::Storage);
    h.select_row("storage-pbds", &pbd.to_string());
    h.click("storage.detach");
    assert_eq!(h.surface().id, view::CHANGE);
    let (_, target, operation) = editor(h).review.clone().expect("a review");
    assert_eq!(target, ResourceTarget::Pbd(pbd));
    assert_eq!(
        operation,
        StorageOperation::SetAttached {
            sr,
            host,
            expected_attached: true,
            attached: false
        }
    );
    assert_eq!(editor(h).target, ResourceTarget::Storage(sr));
    assert!(h.find("storage-back").is_none());
    assert_eq!(h.try_click("storage-confirm"), Err(DEMO.into()));
}

#[test]
fn a_repository_with_one_attachment_needs_no_server_to_say_which() {
    let mut a = attached();
    let (first, second) = (a.pbds[1], a.pbds[2]);
    amend(&mut a.h, |i| {
        i.details.pbds.remove(&second);
    });
    let repository = a.repository();
    a.h.select(repository.clone());
    assert_eq!(
        a.h.eligibility("storage.detach"),
        Eligibility::Disabled(SEVERAL.into())
    );
    amend(&mut a.h, |i| {
        i.details.pbds.remove(&first);
    });
    assert_eq!(a.h.eligibility("storage.detach"), Eligibility::Enabled);
    amend(&mut a.h, |i| i.details.pbds.clear());
    assert_eq!(
        a.h.eligibility("storage.detach"),
        Eligibility::Disabled(NO_ATTACHMENT.into())
    );
    // Rune disabled these buttons on a connection that cannot write.
    let mut a = attached();
    a.h.model.demo = false;
    a.h.select(a.host(0));
    a.h.open_tab(TabId::Storage);
    a.h.select_row("storage-pbds", &a.pbds[0].to_string());
    assert_eq!(
        a.h.eligibility("storage.detach"),
        Eligibility::Disabled(NOT_WRITABLE.into())
    );
    // Forget was always offered; outside the demo its review needs the
    // repository observed on a current connection.
    assert_eq!(
        a.h.eligibility("storage.forget"),
        Eligibility::Disabled(editor::STALE.into())
    );
}

#[test]
fn the_commands_on_a_repository_are_hidden_for_other_kinds_and_tabs_of_other_kinds_pass_on() {
    let mut a = attached();
    let i = inventory(&a.h);
    let vm = *i.vms.keys().next().expect("a VM");
    let vm = Target::one(ObjectRef::vm(a.pool, vm));
    a.h.select(vm.clone());
    for command in ["storage.forget", "storage.detach", "storage.trim"] {
        assert_eq!(a.h.eligibility(command), Eligibility::Hidden, "{command}");
    }
    assert_eq!(a.h.eligibility("storage.new"), Eligibility::Enabled);
    assert!(view::tab(&a.h.model, TabId::Storage, &vm).is_none());
    assert!(view::tab(&a.h.model, TabId::Storage, &a.repository()).is_none());
    assert!(view::tab(&a.h.model, TabId::General, &a.host(0)).is_none());
    assert!(view::tab(&a.h.model, TabId::Storage, &a.host(0)).is_some());
    // The context menu of a repository has every command on it.
    let menu = a.h.context_menu(&a.repository());
    for command in ["storage.forget", "storage.destroy", "storage.reattach"] {
        assert!(menu.iter().any(|item| item.id.0 == command), "{command}");
    }
}

#[test]
fn a_new_host_attachment_starts_from_one_the_repository_has() {
    let mut a = attached();
    let target = a.repository();
    a.h.invoke_on("storage.attachment.create", &target);
    assert_eq!(a.h.surface().id, view::ATTACHMENT);
    let e = editor(&a.h);
    assert!(e.mode == Mode::Attachment);
    assert_eq!(e.kind, 0);
    assert_eq!(e.server, "nfs.example.test");
    assert_eq!(e.path, "/exports/demo");
    assert!(e.version == NfsVersion::V4);
    assert!(a.h.find("storage-name").is_none());
    assert!(a.h.has_text("· attachment exists"));
    a.h.click("storage-review");
    assert!(matches!(
        reviewed(&a.h),
        StorageOperation::CreateAttachment {
            location: StorageLocation::Nfs { .. },
            ..
        }
    ));
    a.h.click("storage-back");
    assert!(editor(&a.h).review.is_none());
    assert!(editor(&a.h).mode == Mode::Attachment);
}

#[test]
fn reclaiming_blocks_is_offered_through_attached_hosts_once_the_driver_advertises_it() {
    let mut a = attached();
    let target = a.repository();
    a.h.invoke_on("storage.trim", &target);
    assert_eq!(a.h.surface().id, view::CHANGE);
    assert!(a.h.find("storage-trim-unsupported").is_some());
    let kind = inventory(&a.h).storage[0].kind.clone();
    editor_mut(&mut a.h).backends.push(StorageBackend {
        kind,
        name: "Fixture driver".into(),
        features: vec!["SR_TRIM".into()],
        configuration: Default::default(),
    });
    assert!(a.h.find("storage-trim-unsupported").is_none());
    let attached = format!("storage-trim-{}", a.hosts[0]);
    assert!(a.h.find(&format!("storage-trim-{}", a.hosts[1])).is_none());
    a.h.click(&attached);
    assert!(
        matches!(reviewed(&a.h), StorageOperation::Trim { host, .. } if *host == a.hosts[0])
    );
}

#[test]
fn restoring_a_connection_reviews_only_the_detached_attachment_it_was_offered_for() {
    let mut a = attached();
    let sr = SrId::from(a.repository);
    // The attachment is attached by now: refused on the shared error line.
    editor::review_restore(&mut a.h.model, a.pool, sr, a.pbds[0], a.hosts[0]);
    assert!(a.h.model.areas.storage.editor.is_none());
    assert_eq!(
        a.h.model.error.as_deref(),
        Some("The detached attachment changed; close and reopen this dialog")
    );
    editor::review_restore(&mut a.h.model, a.pool, sr, a.pbds[1], a.hosts[1]);
    assert_eq!(
        *reviewed(&a.h),
        StorageOperation::SetAttached {
            sr,
            host: a.hosts[1],
            expected_attached: false,
            attached: true
        }
    );
}

// -- 3. Hazards ---------------------------------------------------------------------------

/// Hazard 5: a change of backend or host clears what depended on the old one.
#[test]
fn changing_backend_or_host_clears_what_depended_on_it() {
    let mut h = Harness::scene("storage-hba");
    h.click("storage-result-0");
    assert!(editor(&h).hba_device.is_some());
    let other = *inventory(&h).hosts.keys().nth(1).expect("a second host");
    h.pick("storage-host", &other.to_string());
    let e = editor(&h);
    assert_eq!(e.host, Some(other));
    assert!(e.hba_device.is_none());
    assert!(e.scsi.is_empty());
    // Into a local backend: the path of the other backend does not become a device.
    let mut h = Harness::scene("storage-create");
    h.click("storage-back");
    h.click("storage-back");
    assert_eq!(step(&h), 0);
    h.click("storage-kind-5");
    let e = editor(&h);
    assert_eq!(e.kind, 5);
    assert!(e.path.is_empty());
    assert!(e.allocation == StorageAllocation::DriverDefault);
    // And the mode is chosen on the same step.
    h.pick("storage-mode", "introduce");
    assert!(editor(&h).mode == Mode::Introduce);
    h.click("storage-next");
    h.click("storage-next");
    assert!(h.find("storage-uuid").is_some());
    assert!(h.find("storage-local-identity").is_none(), "only when creating");
}

/// Hazard 8: without an advertised allocation option only the driver's
/// default can be chosen.
#[test]
fn allocation_choices_need_the_driver_to_advertise_them() {
    let mut h = Harness::scene("storage-hba");
    h.click("storage-result-0");
    assert_eq!(h.try_click("storage-allocation-Thick"), Ok(()));
    assert!(editor(&h).allocation == StorageAllocation::Thick);
    editor_mut(&mut h).backends.clear();
    assert_eq!(
        h.try_click("storage-allocation-Thin"),
        Err("Allocation choices require the host driver to advertise the allocation option.".into())
    );
}

/// Hazard 13: every credential edit moves the discovery fence.
#[test]
fn credential_edits_move_the_discovery_fence() {
    let mut h = Harness::scene("storage-discovery");
    h.click("storage-results-back");
    let before = editor(&h).auth_epoch;
    h.click("storage-auth");
    h.type_text("storage-username", "fixture");
    h.type_text("storage-password", "synthetic-only");
    assert_eq!(editor(&h).auth_epoch, before + 3);
    assert_eq!(editor(&h).password.expose(), "synthetic-only");
    assert!(
        h.texts().iter().all(|text| !text.contains("synthetic-only")),
        "a password is never described as text"
    );
}

/// Hazards 9 and 14: the submission makes the checks of the button again.
/// A synthetic review has no observation, so it is stale; the captured
/// credentials are not consumed by a refusal before the submission.
#[test]
fn the_submission_refuses_what_the_button_refuses() {
    let mut h = Harness::scene("storage-auth-review");
    editor::apply(&mut h.model);
    assert_eq!(
        editor(&h).error.as_deref(),
        Some(Error::Stale.to_string().as_str())
    );
    assert!(editor(&h).review.is_some());
    assert!(editor(&h).review_credentials.is_some());
    assert!(h.model.resource_operations.is_empty());
    // Outside the demo the button says the connection cannot write.
    h.model.demo = false;
    assert_eq!(h.try_click("storage-confirm"), Err(NOT_WRITABLE.into()));
    // Back drops the captured credentials with the review.
    h.click("storage-back");
    assert!(editor(&h).review_credentials.is_none());
    assert!(editor(&h).review_auth_label.is_none());
}

/// The Rune text said "in Activity" (decision 10).
#[test]
fn the_cleanup_caption_names_the_events_tab() {
    let source = include_str!("view.rs");
    assert!(source.contains(
        "Resolve or acknowledge the uncertain source operation on the Events tab of the Notifications view before separate cleanup."
    ));
}
