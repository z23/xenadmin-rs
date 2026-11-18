//! The tests of the area, in four parts.
//!
//! 1. **The Rune tests**: the three of `host_power_view.rs`, the eight of
//!    `host_support_view.rs`, and this area's share of two tests of other
//!    Rune files that ran over its scenes. Each asserts what the Rune test
//!    asserted; where the Rune test pressed a button of the mode row, the
//!    test here runs the command that replaced it.
//! 2. **What the port changed**: the panel's modes became commands and
//!    dialogs, both reviews gained Back, the editors close with the scope.
//! 3. **The hazards** the map lists for the two modules (10 to 24).
//! 4. **Against the fixture server**: what needs a current snapshot.
//!
//! The two window sizes of the Rune tests (1440x900 and 513x652, "the
//! footer stays visible") are layout: they are GPUI tests, listed in
//! `notes/progress/gpui-tests-wanted.md`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use serde_json::json;
use xenadmin_core::{
    Error, HostArchiveOperation, HostCrashdump, HostId, HostPowerConfiguration, HostPowerOperation,
    HostPowerSecretEvidence, HostPowerSettings, HostSupportOverview, Inventory, OperationId,
    OperationState, PoolId, ResourceAction, ResourceOperationRecord, TransferAction,
};

use super::commands::NOT_WRITABLE;
use super::power::{self, Editor};
use super::support::{self, Mode, Panel};
use super::{scenes, view};
use crate::Harness;
use crate::intent::{FileMode, FileRequest, Intent};
use crate::prelude::*;
use crate::surface::EXPERIMENTAL_NOTICE_KEY;
use crate::testing::{Live, Server, TempDir, poll_until};

// -- Fixtures -------------------------------------------------------------------

const BACK_UP: &str = "server.backup";
const RESTORE: &str = "server.restore-from-backup";
const REPORT: &str = "tools.server-status-report";
const SETTINGS: &str = "server.power-on-settings";

/// The scene's server selected in the demo pool, and the dialog of one
/// command open on it.
fn dialog(command: &str) -> Harness {
    let mut h = Harness::scene("host-support");
    h.dismiss();
    assert!(h.surfaces().is_empty());
    h.invoke(command);
    h
}

#[track_caller]
fn panel(h: &Harness) -> &Panel {
    h.model.areas.host_services.support.as_ref().expect("panel")
}

#[track_caller]
fn panel_mut(h: &mut Harness) -> &mut Panel {
    h.model.areas.host_services.support.as_mut().expect("panel")
}

#[track_caller]
fn editor(h: &Harness) -> &Editor {
    h.model.areas.host_services.power.as_ref().expect("editor")
}

#[track_caller]
fn editor_mut(h: &mut Harness) -> &mut Editor {
    h.model.areas.host_services.power.as_mut().expect("editor")
}

fn pool(h: &Harness) -> PoolId {
    h.model.inventory.as_ref().expect("inventory").pool_id
}

/// A server of the pool that is not `host`.
fn other_host(h: &Harness, host: HostId) -> ObjectRef {
    let inventory = h.model.inventory.as_ref().expect("inventory");
    let other = inventory
        .hosts
        .keys()
        .copied()
        .find(|id| *id != host)
        .expect("a second server");
    ObjectRef::host(inventory.pool_id, other)
}

/// The inventory moves on: what a poll does when the host reports a change.
fn amend(h: &mut Harness, change: impl FnOnce(&mut Inventory)) {
    let mut inventory = h
        .model
        .inventory
        .take()
        .expect("inventory")
        .as_ref()
        .clone();
    change(&mut inventory);
    h.model.inventory = Some(Arc::new(inventory));
}

/// The file prompt that is waiting for its answer.
#[track_caller]
fn prompt(h: &Harness) -> &FileRequest {
    h.model
        .intents
        .iter()
        .find_map(|intent| match intent {
            Intent::ChooseFile(request) => Some(request),
            _ => None,
        })
        .expect("a file prompt")
}

/// Presses the file button, answers the prompt with `path` and waits for the
/// worker thread that inspects it.
fn choose(h: &mut Harness, path: PathBuf) {
    h.click("host-support-file");
    let restore = panel(h).mode == Mode::Restore;
    assert_eq!(matches!(prompt(h).mode, FileMode::Open), restore);
    assert_eq!(matches!(prompt(h).mode, FileMode::Save), !restore);
    h.answer_file(Some(path));
    poll_until(h, "the archive file is inspected", |h| !panel(h).pending);
}

fn dumps(count: u128) -> Vec<HostCrashdump> {
    (1..=count)
        .map(|id| HostCrashdump {
            uuid: uuid::Uuid::from_u128(id).into(),
            timestamp: "20261108T00:00:00Z".into(),
            size: 4096,
        })
        .collect()
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 0 {
                crc >> 1
            } else {
                (crc >> 1) ^ 0xEDB8_8320
            };
        }
    }
    !crc
}

/// A host filesystem backup with nothing but the inventory file that says
/// whose it is: a GNU tar of one entry, in a gzip member of one stored
/// block. (The Rune test built the same with the crates `tar` and `flate2`,
/// which this crate does not have.)
fn backup(path: &Path, host: HostId, version: &str) {
    let data = format!("INSTALLATION_UUID='{host}'\nPRODUCT_VERSION='{version}'\n");
    let mut header = [0u8; 512];
    let name = b"etc/xensource-inventory";
    header[..name.len()].copy_from_slice(name);
    header[100..108].copy_from_slice(b"0000600\0");
    header[108..116].copy_from_slice(b"0000000\0");
    header[116..124].copy_from_slice(b"0000000\0");
    header[124..136].copy_from_slice(format!("{:011o}\0", data.len()).as_bytes());
    header[136..148].copy_from_slice(b"00000000000\0");
    header[148..156].fill(b' ');
    header[156] = b'0';
    header[257..265].copy_from_slice(b"ustar  \0");
    let sum: u32 = header.iter().map(|byte| u32::from(*byte)).sum();
    header[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
    let mut tar = header.to_vec();
    tar.extend_from_slice(data.as_bytes());
    tar.resize(1024, 0);
    tar.resize(2048, 0);
    let length = u16::try_from(tar.len()).expect("one stored block");
    let mut gzip = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0xff, 1];
    gzip.extend(length.to_le_bytes());
    gzip.extend((!length).to_le_bytes());
    gzip.extend(&tar);
    gzip.extend(crc32(&tar).to_le_bytes());
    gzip.extend(u32::from(length).to_le_bytes());
    std::fs::write(path, gzip).expect("the backup file");
}

/// A record of a power-on configuration of `host` that kept a Secret.
fn configuration(
    pool: PoolId,
    host: HostId,
    state: OperationState,
    cleaned: bool,
) -> ResourceOperationRecord {
    let mut record: ResourceOperationRecord = serde_json::from_value(json!({
        "id": OperationId::new(), "pool": pool, "target": {"Host": host},
        "action": "ScanStorage", "state": state, "progress": null,
        "detail": "Synthetic power-on configuration", "created_at_ms": 123,
        "can_cancel": false, "cancel_requested": false, "steps": [], "created": null,
        "baseline_networks": [],
    }))
    .expect("a record");
    record.action = ResourceAction::HostPower(HostPowerOperation::Configure {
        expected: HostPowerConfiguration {
            settings: HostPowerSettings::Disabled,
            secret: None,
        },
        settings: HostPowerSettings::WakeOnLan,
    });
    record.host_power_secret = Some(HostPowerSecretEvidence {
        secret: uuid::Uuid::new_v4().into(),
        cleaned,
    });
    record
}

// -- 1. The Rune tests: host_power_view.rs ----------------------------------------

/// Rune `review_has_private_password_and_captured_target_at_both_widths`
/// (the two sizes are a GPUI test).
#[test]
fn review_has_private_password_and_captured_target() {
    let mut h = Harness::scene("host-power-settings");
    assert_eq!(h.surface().id, view::POWER);
    let target = editor(&h).target;
    assert!(h.find("host-power-confirm").is_some());
    assert!(
        h.has_text("Requested: IPMI at 192.0.2.60, user fixture-admin, with a new password"),
        "{}",
        h.dump()
    );
    assert!(!h.dump().contains("synthetic-password-not-shown"));
    assert_eq!(h.try_click("host-power-confirm"), Err(view::DEMO.into()));
    assert_eq!(editor(&h).target, target);
    assert!(h.model.resource_operations.is_empty());
    assert!(editor(&h).review.is_some());
    // The draft does not show the password either.
    h.click("host-power-back");
    assert!(h.find("host-power-password").is_some());
    assert!(!h.dump().contains("synthetic-password-not-shown"));
    h.assert_unique_keys();
}

/// Rune `configuration_review_refuses_invalid_inputs_and_unknown_configuration`.
#[test]
fn configuration_review_refuses_invalid_inputs_and_unknown_configuration() {
    let mut h = Harness::scene("host-power-settings");
    h.click("host-power-back");
    for (address, error) in [
        ("not an address", "Enter a valid BMC IP address"),
        ("0.0.0.0", "Enter a unicast BMC IP address"),
        ("239.0.0.1", "Enter a unicast BMC IP address"),
    ] {
        h.type_text("host-power-address", address);
        h.click("host-power-review");
        assert!(editor(&h).review.is_none(), "{address}");
        assert_eq!(editor(&h).error.as_deref(), Some(error));
        assert!(h.has_text(error));
    }
    h.type_text("host-power-address", "192.0.2.60");
    editor_mut(&mut h).expected = None;
    h.click("host-power-review");
    assert!(editor(&h).review.is_none());
    assert_eq!(
        editor(&h).error.as_deref(),
        Some("Current power-on settings are unavailable or use an unsupported custom plugin")
    );
    assert!(h.has_text("Current: Unavailable or unsupported custom configuration"));
}

/// Rune `switching_mode_discards_pending_password_and_review`.
#[test]
fn switching_mode_discards_pending_password_and_review() {
    let mut h = Harness::scene("host-power-settings");
    power::change_mode(&mut h.model, 1);
    let e = editor(&h);
    assert!(e.password.is_empty());
    assert!(e.review.is_none());
    assert_eq!(power::settings(e).unwrap(), HostPowerSettings::WakeOnLan);
    // The way a user goes: the choice. Rune disabled the button of the mode
    // that is set; choosing it again keeps what was typed.
    let mut h = Harness::scene("host-power-settings");
    h.click("host-power-back");
    h.pick("host-power-mode", "2");
    assert!(!editor(&h).password.is_empty());
    h.pick("host-power-mode", "1");
    assert!(editor(&h).password.is_empty());
    assert!(h.find("host-power-password").is_none());
}

// -- 1. The Rune tests: host_support_view.rs ----------------------------------------

/// Rune `host_archive_review_captures_target_and_keeps_compact_footer_visible`.
/// Rune moved the cursor of the host table; the grid selection is its
/// successor (the tree selection closes the dialog, which is tested below).
#[test]
fn host_archive_review_captures_target() {
    let mut h = Harness::scene("host-support");
    assert_eq!(h.surface().id, view::SUPPORT);
    let captured = panel(&h).host;
    let directory = TempDir::new("host-services-archive");
    choose(&mut h, directory.path().join("private.tar"));
    assert!(h.has_text("Destination: private.tar"));
    h.click("host-support-review");
    let expected = TransferAction::Host {
        host: captured,
        expected_version: panel(&h).version.clone(),
        operation: HostArchiveOperation::Support {
            entries: vec!["host-crashdump-logs".into()],
        },
    };
    assert_eq!(panel(&h).review, Some(expected));
    assert!(h.has_text("Save as: private.tar"));
    assert!(h.has_text("Requested categories: host-crashdump-logs"));
    // Hazard 24: what the core does with the file.
    assert!(h.has_text(
        "Creates a new private file (0600). An existing file is never overwritten. The selected host's task and archive format are verified before the file is published."
    ));
    let other = other_host(&h, captured);
    h.model.select_in_grid(Target::one(other));
    assert_eq!(panel(&h).host, captured);
    assert_eq!(h.try_click("host-support-submit"), Err(view::DEMO.into()));
    support::submit(&mut h.model);
    assert_eq!(panel(&h).error, Some(Error::Stale.to_string()));
    assert!(h.model.resource_operations.is_empty());
    assert!(!directory.path().join("private.tar").exists());
    h.click("host-support-close");
    assert!(h.model.areas.host_services.support.is_none());
}

/// Rune `crashdump_delete_review_captures_exact_dump_and_keeps_footer_visible`.
#[test]
fn crashdump_delete_review_captures_exact_dump() {
    let mut h = Harness::scene("host-crashdump-delete");
    assert_eq!(h.surface().id, view::CRASHDUMP);
    let p = panel(&h);
    let captured_host = p.host;
    let captured_dump = p.deletion.clone().expect("a dump under review");
    assert_eq!(captured_dump, scenes::scene_dump());
    assert!(p.file.is_none() && p.review.is_none());
    assert_eq!(
        h.try_click("host-crashdump-submit"),
        Err(view::TYPE_DUMP.into())
    );
    assert!(h.model.resource_operations.is_empty());
    let other = other_host(&h, captured_host);
    h.model.select_in_grid(Target::one(other));
    h.type_text(
        "host-crashdump-confirmation",
        &captured_dump.uuid.to_string(),
    );
    let p = panel(&h);
    assert_eq!(p.host, captured_host);
    assert_eq!(p.deletion.as_ref(), Some(&captured_dump));
    assert_eq!(h.try_click("host-crashdump-submit"), Err(view::DEMO.into()));
    support::submit_deletion(&mut h.model);
    assert_eq!(panel(&h).error, Some(Error::Stale.to_string()));
    assert!(
        h.model.resource_operations.is_empty(),
        "synthetic reviews cannot mutate"
    );
    support::mode(&mut h.model, Mode::Backup);
    assert!(panel(&h).deletion.is_none());
    assert!(panel(&h).confirmation.is_empty());
}

/// Rune `crashdump_catalogue_pages_make_every_captured_candidate_reachable`.
#[test]
fn crashdump_catalogue_pages_make_every_captured_candidate_reachable() {
    let mut h = Harness::scene("host-support");
    panel_mut(&mut h)
        .overview
        .as_mut()
        .expect("overview")
        .crashdumps = dumps(65);
    let key = |id: u128| format!("host-crashdump-delete-{}", uuid::Uuid::from_u128(id));
    assert!(h.has_text("65 crashdump records"));
    assert!(h.has_text("Page 1 of 3"));
    assert!(h.find(&key(32)).is_some() && h.find(&key(33)).is_none());
    assert_eq!(
        h.try_click("host-crashdump-previous"),
        Err("This is the first page.".into())
    );
    h.click("host-crashdump-next");
    assert_eq!(panel(&h).dump_page, 1);
    h.click("host-crashdump-next");
    assert_eq!(panel(&h).dump_page, 2);
    assert_eq!(
        h.try_click("host-crashdump-next"),
        Err("This is the last page.".into())
    );
    h.assert_unique_keys();
    let last = uuid::Uuid::from_u128(65);
    h.click(&key(65));
    assert_eq!(
        panel(&h).deletion.as_ref().expect("a dump").uuid.uuid(),
        last
    );
    assert_eq!(h.surface().id, view::CRASHDUMP);
}

/// Rune `crashdump_delete_refuses_unadvertised_review_or_changed_host`.
#[test]
fn crashdump_delete_refuses_unadvertised_review_or_changed_host() {
    let mut h = Harness::scene("host-support");
    let invented = HostCrashdump {
        uuid: uuid::Uuid::from_u128(88).into(),
        timestamp: "20261108T00:00:00Z".into(),
        size: 4096,
    };
    support::review_deletion(&mut h.model, invented);
    assert!(panel(&h).deletion.is_none());
    // The same uuid with another size is not the advertised dump either
    // (hazard 20: compared by full value).
    let mut h = Harness::scene("host-crashdump-delete");
    support::back(&mut h.model);
    let mut resized = scenes::scene_dump();
    resized.size += 1;
    support::review_deletion(&mut h.model, resized);
    assert!(panel(&h).deletion.is_none());

    let mut h = Harness::scene("host-crashdump-delete");
    let host = panel(&h).host;
    let uuid = scenes::scene_dump().uuid.to_string();
    h.type_text("host-crashdump-confirmation", &uuid);
    amend(&mut h, |inventory| {
        inventory.hosts.remove(&host);
    });
    assert!(!support::current(&h.model, panel(&h)));
    assert_eq!(
        h.try_click("host-crashdump-submit"),
        Err(support::STALE.into())
    );
    assert!(h.has_text(support::STALE));
    support::submit_deletion(&mut h.model);
    assert!(h.model.resource_operations.is_empty());
}

/// Rune `stale_native_file_callback_cannot_set_a_reopened_host_review`.
#[test]
fn stale_native_file_callback_cannot_set_a_reopened_host_review() {
    let mut h = Harness::scene("host-support");
    let old = panel(&h).nonce;
    h.click("host-support-file");
    support::close(&mut h.model);
    h.invoke(REPORT);
    assert_ne!(panel(&h).nonce, old);
    let directory = TempDir::new("host-services-stale");
    h.answer_file(Some(directory.path().join("old.tar")));
    h.tick(0.1);
    assert!(panel(&h).file.is_none());
    assert!(!panel(&h).pending);
    assert!(panel(&h).result.is_none());
}

/// Rune `unknown_capabilities_and_changed_host_version_cannot_be_reviewed`.
#[test]
fn unknown_capabilities_and_changed_host_version_cannot_be_reviewed() {
    let mut h = Harness::scene("host-support");
    panel_mut(&mut h)
        .overview
        .as_mut()
        .expect("overview")
        .capabilities_error = Some("Unavailable".into());
    assert!(support::action(panel(&h)).is_none());
    assert!(h.has_text("Unavailable"));
    assert_eq!(
        h.try_click("host-support-review"),
        Err(view::NO_CATEGORY.into())
    );
    let host = panel(&h).host;
    amend(&mut h, |inventory| {
        inventory.hosts.get_mut(&host).expect("host").version = "changed".into();
    });
    support::mode(&mut h.model, Mode::Backup);
    assert!(!support::current(&h.model, panel(&h)));
    assert!(h.find("host-support-stale").is_some());
    assert_eq!(
        h.try_click("host-support-review"),
        Err(support::STALE.into())
    );
}

/// Rune `restore_native_open_inspects_captured_host_and_requires_exact_confirmation`.
#[test]
fn restore_native_open_inspects_captured_host_and_requires_exact_confirmation() {
    let mut h = dialog(RESTORE);
    assert_eq!(h.surface().id, view::RESTORE);
    let host = panel(&h).host;
    let version = panel(&h).version.clone();
    let directory = TempDir::new("host-services-restore");
    let source = directory.path().join("trusted.tar.gz");
    backup(&source, host, &version);
    choose(&mut h, source.clone());
    let p = panel(&h);
    assert!(p.error.is_none(), "{:?}", p.error);
    let file = p.file.as_ref().expect("the inspected backup");
    assert_eq!(file.host_backup().expect("a summary").host, host);
    assert!(h.has_text("Source: trusted.tar.gz"));
    h.click("host-support-review");
    assert!(matches!(
        &panel(&h).review,
        Some(TransferAction::Host {
            operation: HostArchiveOperation::StageRestore { archive },
            ..
        }) if archive.host == host
    ));
    assert!(h.has_text("Source archive: trusted.tar.gz"));
    assert_eq!(
        h.try_click("host-support-submit"),
        Err(view::TYPE_HOST.into())
    );
    support::submit(&mut h.model);
    assert!(h.model.resource_operations.is_empty());
    h.type_text("host-restore-confirmation", &host.to_string());
    assert_eq!(h.try_click("host-support-submit"), Err(view::DEMO.into()));
    support::submit(&mut h.model);
    assert!(
        h.model.resource_operations.is_empty(),
        "synthetic review never submits"
    );
    assert!(source.exists());
    h.assert_unique_keys();
    h.click("host-support-close");
    assert!(h.surfaces().is_empty());
}

/// Rune `restore_rejects_wrong_host_and_cancels_obsolete_inspections`.
#[test]
fn restore_rejects_wrong_host_and_cancels_obsolete_inspections() {
    let mut h = dialog(RESTORE);
    let directory = TempDir::new("host-services-wrong");
    let path = directory.path().join("wrong.tar.gz");
    backup(&path, HostId::from(uuid::Uuid::from_u128(999)), "8.3.0");
    choose(&mut h, path.clone());
    let p = panel(&h);
    assert!(p.error.is_some());
    assert!(p.file.is_none());
    assert!(support::action(p).is_none());
    assert_eq!(h.try_click("host-support-review"), Err(view::NO_FILE.into()));
    h.click("host-support-file");
    let cancel = panel(&h).inspection_cancel.clone().expect("a cancel flag");
    support::mode(&mut h.model, Mode::Backup);
    assert!(cancel.load(Ordering::Relaxed));
    h.answer_file(Some(path));
    h.tick(0.1);
    let p = panel(&h);
    assert!(p.file.is_none());
    assert!(!p.pending);
    assert_eq!(p.mode, Mode::Backup);
}

// -- 1. The Rune tests of other files, this area's share ------------------------------

/// Rune `main.rs` `dialogs_outside_the_supported_core_carry_the_experimental_notice`:
/// `experimental_dialog_open` had both dialogs of these modules flagged.
#[test]
fn every_surface_of_the_area_carries_the_experimental_notice() {
    for scene in [
        "host-power-settings",
        "host-support",
        "host-restore",
        "host-crashdump-delete",
    ] {
        let h = Harness::scene(scene);
        assert!(h.model.experimental_notice(), "{scene}");
        assert!(h.find(EXPERIMENTAL_NOTICE_KEY).is_some(), "{scene}");
        h.assert_unique_keys();
    }
    let h = dialog(BACK_UP);
    assert!(h.model.experimental_notice());
    let mut h = Harness::scene("host-support");
    h.dismiss();
    assert!(!h.model.experimental_notice());
}

/// Rune `session_permissions_view.rs`
/// `review_permission_notice_is_reachable_without_bypassing_synthetic_target_guard`
/// over `host_power_view::scene`.
#[test]
fn review_permission_notice_is_reachable_without_bypassing_synthetic_target_guard() {
    let mut h = Harness::scene("host-power-settings");
    assert!(
        h.find(permissions::REVIEW_KEY).is_some(),
        "captured review must explain permission discovery"
    );
    assert_eq!(h.try_click("host-power-confirm"), Err(view::DEMO.into()));
    assert!(h.model.resource_operations.is_empty());
    assert!(h.model.operations.is_empty());
    assert!(h.model.error.is_none());
    // Before the review there is no operation to assess, so no line. The
    // panel never had one (Rune checked no permission there).
    h.click("host-power-back");
    assert!(h.find(permissions::REVIEW_KEY).is_none());
    let h = Harness::scene("host-restore");
    assert!(h.find(permissions::REVIEW_KEY).is_none());
}

// -- 2. What the port changed ---------------------------------------------------------

/// The scenes show what the Rune scenes showed, under the ids of the
/// dialogs the panel became.
#[test]
fn the_scenes_open_the_dialogs_of_their_names() {
    for (scene, id, title) in [
        (
            "host-power-settings",
            view::POWER,
            "Host power-on settings",
        ),
        ("host-support", view::SUPPORT, "Support bundle"),
        ("host-restore", view::RESTORE, "Stage recovery"),
        (
            "host-crashdump-delete",
            view::CRASHDUMP,
            "Delete retained host crashdump",
        ),
    ] {
        let h = Harness::scene(scene);
        assert_eq!(h.surfaces().len(), 1, "{scene}");
        assert_eq!(h.surface().id, id);
        assert_eq!(h.surface().title, title);
    }
    let h = Harness::scene("host-restore");
    assert!(h.has_text("12042 entries · 3.0 GiB logical contents") || h.has_text("12042 entries"));
    assert!(h.has_text(&format!("SHA-256 {}", "42".repeat(32))));
    assert!(h.find("host-restore-confirmation").is_some());
    let h = Harness::scene("host-support");
    assert_eq!(h.node("host-support-cap-host-crashdump-logs").checked, Some(true));
    assert!(h.has_text("0 crashdump records"));
}

/// The row "Filesystem backup / Support bundle / Stage recovery…" of the
/// Rune panel is three commands at the plan's menu places; the power-on
/// settings have no menu place.
#[test]
fn the_modes_of_the_panel_are_commands_of_the_server_and_tools_menus() {
    let mut h = Harness::scene("host-support");
    h.dismiss();
    for (id, label, surface, title) in [
        (BACK_UP, "Back Up", view::BACKUP, "Filesystem backup"),
        (
            RESTORE,
            "Restore From Backup",
            view::RESTORE,
            "Stage recovery",
        ),
        (
            REPORT,
            "Server Status Report",
            view::SUPPORT,
            "Support bundle",
        ),
    ] {
        let item = h.menu_item(id).expect(id);
        assert_eq!(item.label, label);
        assert_eq!(item.disabled, None, "{id}");
        h.invoke(id);
        assert_eq!(h.surface().id, surface);
        assert_eq!(h.surface().title, title);
        assert!(panel(&h).file.is_none() && panel(&h).review.is_none());
        for gone in [
            "host-support-backup",
            "host-support-bundle",
            "host-support-restore",
        ] {
            assert!(h.find(gone).is_none(), "{gone}");
        }
        h.dismiss();
        assert!(h.surfaces().is_empty());
    }
    assert!(h.menu_item(SETTINGS).is_none());
    assert_eq!(h.eligibility(SETTINGS), Eligibility::Enabled);
    // Not a server, or several: the commands do not apply.
    let pool_id = pool(&h);
    let host = h.model.target();
    let both = Target::many(
        host.objects
            .iter()
            .cloned()
            .chain([other_host(&h, host.objects[0].host_id().expect("host"))]),
    );
    for target in [Target::one(ObjectRef::pool(pool_id)), both, Target::none()] {
        for id in [BACK_UP, RESTORE, REPORT, SETTINGS] {
            assert_eq!(
                h.model.eligibility(CommandId(id), &target),
                Eligibility::Hidden,
                "{id}"
            );
        }
    }
}

/// Only the status report asks the server what it offers: the backup and
/// the staging dialog show nothing of the answer.
#[test]
fn a_demo_asks_no_server_and_says_so() {
    for command in [BACK_UP, RESTORE, REPORT] {
        let h = dialog(command);
        assert!(panel(&h).query.is_none());
        assert!(h.find("host-support-preview").is_some());
        assert!(h.has_text("Synthetic preview · no host requests are submitted"));
    }
    let h = Harness::scene("host-power-settings");
    assert!(h.has_text("Synthetic preview · no host changes can be submitted"));
}

/// The button "BMC / power-on settings…" of the Power On page of a server's
/// Properties dialog (`host_admin`) runs this area's command: the Properties
/// dialog gives way, and the editor opens on its draft.
#[test]
fn the_power_on_page_of_the_properties_dialog_opens_the_settings() {
    let mut h = Harness::scene("host-power-settings");
    h.dismiss();
    h.invoke("server.properties");
    h.pick("host-properties-pages", "power-on");
    h.click("host-bmc-settings");
    assert_eq!(h.surfaces().len(), 1);
    assert_eq!(h.surface().id, view::POWER);
    let e = editor(&h);
    assert!(e.review.is_none());
    assert_eq!(e.mode, 0);
    assert!(h.has_text("Current: Disabled"));
    assert_eq!(h.node("host-power-mode-0").checked, Some(true));
    assert!(h.find("host-power-address").is_none());
    h.click("host-power-close");
    assert!(h.surfaces().is_empty());
}

/// The editor is prefilled from the configuration of the server.
#[test]
fn the_draft_starts_from_the_current_configuration() {
    let mut h = Harness::scene("host-power-settings");
    h.dismiss();
    let host = h.model.target().objects[0].host_id().expect("host");
    let secret = uuid::Uuid::from_u128(77);
    amend(&mut h, |inventory| {
        inventory
            .details
            .hosts
            .entry(host)
            .or_default()
            .power_configuration = Some(HostPowerConfiguration {
            settings: HostPowerSettings::Ipmi {
                address: "192.0.2.61".parse().unwrap(),
                username: "operator".into(),
            },
            secret: Some(secret.into()),
        });
    });
    h.invoke(SETTINGS);
    let e = editor(&h);
    assert_eq!((e.mode, e.address.as_str()), (2, "192.0.2.61"));
    assert_eq!(e.username, "operator");
    assert!(e.password.is_empty());
    assert!(h.has_text(&format!(
        "Current: IPMI at 192.0.2.61, user operator, Secret {secret}"
    )));
    // Hazard 12: the same IPMI settings can be saved again, with a new
    // password, which is always asked for.
    h.click("host-power-review");
    assert_eq!(
        editor(&h).error.as_deref(),
        Some("Enter a new BMC password")
    );
    h.type_text("host-power-password", "another-synthetic-password");
    h.click("host-power-review");
    assert!(matches!(
        editor(&h).review,
        Some(HostPowerOperation::Configure { .. })
    ));
}

/// Both reviews of the panel gained Back. It resets what the Rune mode
/// switch reset (hazard 23).
#[test]
fn back_from_a_review_of_the_panel_resets_what_the_mode_switch_reset() {
    let mut h = Harness::scene("host-restore");
    h.type_text("host-restore-confirmation", "typed");
    h.click("host-support-back");
    let p = panel(&h);
    assert_eq!(p.mode, Mode::Restore);
    assert!(p.review.is_none() && p.file.is_none() && p.confirmation.is_empty());
    assert_eq!(h.surface().id, view::RESTORE);
    assert!(h.find("host-support-file").is_some());

    let mut h = Harness::scene("host-crashdump-delete");
    let epoch = panel(&h).epoch;
    h.type_text("host-crashdump-confirmation", "typed");
    h.click("host-support-back");
    let p = panel(&h);
    assert!(p.deletion.is_none() && p.confirmation.is_empty());
    assert!(p.epoch > epoch);
    assert_eq!(h.surface().id, view::SUPPORT);
    // The catalogue and the chosen categories are kept.
    assert!(h.has_text("1 crashdump records"));
    assert!(p.selected.contains("host-crashdump-logs"));
}

/// Another object selected in the tree: both dialogs close (Rune kept them
/// and relied on `current` failing), and a running inspection is cancelled
/// (hazard 22).
#[test]
fn another_selection_closes_both_dialogs_and_cancels_the_inspection() {
    let mut h = Harness::scene("host-support");
    h.click("host-support-file");
    let cancel = panel(&h).inspection_cancel.clone().expect("a cancel flag");
    let host = h.model.target().objects[0].clone();
    assert!(power::open(&mut h.model, &host));
    // Rune's ranking: the power-on settings in front of the panel.
    assert_eq!(h.surfaces().len(), 2);
    assert_eq!(h.surface().id, view::POWER);
    let other = other_host(&h, host.host_id().expect("host"));
    h.select(Target::one(other));
    assert!(h.model.areas.host_services.power.is_none());
    assert!(h.model.areas.host_services.support.is_none());
    assert!(cancel.load(Ordering::Relaxed));
    assert!(h.surfaces().is_empty());
}

/// The history button closes the panel and shows the Events tab. Decision
/// 10 of AREAS.md: its two Rune labels stay until the one sweep.
#[test]
fn the_history_button_leads_to_the_events_tab() {
    let mut h = Harness::scene("host-support");
    assert_eq!(
        h.node("host-support-history").label.as_deref(),
        Some("Transfer history")
    );
    h.click("host-support-history");
    assert!(h.model.areas.host_services.support.is_none());
    assert_eq!(h.model.active_tab(), TabId::Events);

    let mut h = Harness::scene("host-crashdump-delete");
    assert_eq!(
        h.node("host-support-history").label.as_deref(),
        Some("Activity history")
    );
    h.click("host-support-history");
    assert!(h.model.areas.host_services.support.is_none());
    assert_eq!(h.model.active_tab(), TabId::Events);
}

/// Decision 10 of AREAS.md and hazard 24: the texts that name Rune places
/// or describe what the core guarantees, pinned for the sweep.
#[test]
fn the_review_texts_are_the_rune_texts() {
    assert!(power::CONFIGURE_UNCERTAIN.contains(
        "Use Activity to recheck evidence and review retained Secret cleanup separately."
    ));
    let h = Harness::scene("host-power-settings");
    assert!(h.has_text(power::CONFIGURE_EFFECT));
    assert!(h.has_text(power::CONFIGURE_UNCERTAIN));
    assert!(h.has_text("Current: Disabled"));
    let h = Harness::scene("host-crashdump-delete");
    assert!(h.has_text(&format!("Crashdump {}", scenes::scene_dump().uuid)));
    assert!(h.has_text("Recheck never repeats deletion."));
    assert!(h.has_text("There is no undo."));
    assert!(h.has_text(view::TYPE_DUMP));
    let h = Harness::scene("host-restore");
    assert!(h.has_text("This reformats the host's configured BACKUP_PARTITION"));
    assert!(h.has_text("Use only a backup you trust."));
    assert_eq!(
        h.node("host-support-submit").label.as_deref(),
        Some("Replace backup partition")
    );
    assert_eq!(
        h.node("host-restore-confirmation").label.as_deref(),
        Some(view::TYPE_HOST)
    );
}

// -- 3. The hazards -------------------------------------------------------------------

/// Hazard 10: after every submission attempt, also a refused one, the
/// password is gone, the review cleared and the epoch bumped; the error
/// asks for the password again.
#[test]
fn a_refused_submission_drops_the_password_and_asks_for_it_again() {
    let mut h = Harness::scene("host-power-settings");
    let epoch = editor(&h).epoch;
    power::submit(&mut h.model);
    let e = editor(&h);
    assert!(e.password.is_empty());
    assert!(e.review.is_none());
    assert_eq!(e.epoch, epoch + 1);
    let error = format!("{}. Enter the password and review again.", Error::Stale);
    assert_eq!(e.error.as_deref(), Some(error.as_str()));
    assert!(h.has_text(&error));
    assert!(h.model.resource_operations.is_empty());
    h.click("host-power-review");
    assert_eq!(
        editor(&h).error.as_deref(),
        Some("Enter a new BMC password")
    );
}

/// Hazard 11: any edit after a review invalidates it. Outside the demo the
/// review then says it is stale and cannot be confirmed.
#[test]
fn every_edit_after_a_review_invalidates_it() {
    let mut h = Harness::scene("host-power-settings");
    assert_eq!(editor(&h).epoch, editor(&h).review_epoch);
    assert_eq!(power::stale(&h.model, editor(&h)), None, "the demo says so itself");
    power::edit(&mut h.model, |e| e.username = "someone-else".into());
    assert_ne!(editor(&h).epoch, editor(&h).review_epoch);
    assert!(!power::current(&h.model, editor(&h)));
    // Typing bumps the epoch, field by field.
    let mut h = Harness::scene("host-power-settings");
    h.click("host-power-back");
    let epoch = editor(&h).epoch;
    h.type_text("host-power-address", "192.0.2.62");
    h.type_text("host-power-username", "u");
    h.type_text("host-power-password", "p");
    assert_eq!(editor(&h).epoch, epoch + 3);
    // Not a demo and no current observation: stale.
    let mut h = Harness::scene("host-power-settings");
    h.model.demo = false;
    assert_eq!(
        h.try_click("host-power-confirm"),
        Err(power::STALE.into())
    );
    assert!(h.has_text(power::STALE));
    assert!(h.find("host-power-preview").is_none());
}

/// Hazard 12: an unchanged Disabled or Wake-on-LAN is refused.
#[test]
fn an_unchanged_mode_without_a_password_is_refused() {
    let mut h = Harness::scene("host-power-settings");
    h.click("host-power-back");
    h.pick("host-power-mode", "0");
    h.click("host-power-review");
    assert!(editor(&h).review.is_none());
    assert_eq!(
        editor(&h).error.as_deref(),
        Some("Choose a different power-on mode")
    );
    h.pick("host-power-mode", "1");
    assert!(editor(&h).error.is_none());
    h.click("host-power-review");
    assert!(h.has_text("Current: Disabled"));
    assert!(h.has_text("Requested: Wake-on-LAN"));
    // Back keeps the draft.
    h.click("host-power-back");
    assert_eq!(editor(&h).mode, 1);
    assert!(editor(&h).review.is_none());
}

/// Hazard 13: only terminal, not succeeded, uncleaned records of the same
/// pool and server offer the cleanup, which is a review of its own and
/// changes no settings.
#[test]
fn only_finished_unsucceeded_uncleaned_records_offer_the_cleanup() {
    let mut h = Harness::scene("host-power-settings");
    h.click("host-power-back");
    let pool_id = pool(&h);
    let host = editor(&h).host().expect("host");
    let elsewhere = other_host(&h, host).host_id().expect("host");
    let failed = configuration(pool_id, host, OperationState::Failed, false);
    let running = configuration(pool_id, host, OperationState::Tracking, false);
    let succeeded = configuration(pool_id, host, OperationState::Succeeded, false);
    let cleaned = configuration(pool_id, host, OperationState::Failed, true);
    let foreign = configuration(pool_id, elsewhere, OperationState::Failed, false);
    let cleanup = |id: OperationId| format!("host-power-cleanup-{id}");
    let listed = |id: OperationId| format!("host-power-retained-{id}");
    h.model.resource_rows = vec![
        failed.clone(),
        running.clone(),
        succeeded.clone(),
        cleaned.clone(),
        foreign.clone(),
    ];
    assert!(h.find(&listed(failed.id)).is_some());
    assert!(h.find(&cleanup(failed.id)).is_some());
    assert!(h.find(&listed(running.id)).is_some());
    assert!(h.find(&cleanup(running.id)).is_none(), "not finished");
    for record in [&succeeded, &cleaned, &foreign] {
        assert!(h.find(&listed(record.id)).is_none());
        assert!(h.find(&cleanup(record.id)).is_none());
        power::cleanup(&mut h.model, record.id);
        assert!(
            editor(&h).review.is_none() || record.id == succeeded.id,
            "no cleanup of a cleaned or foreign record"
        );
        power::back(&mut h.model);
    }
    h.assert_unique_keys();
    h.type_text("host-power-password", "typed");
    h.click(&cleanup(failed.id));
    let secret = failed.host_power_secret.as_ref().expect("evidence").secret;
    assert_eq!(
        editor(&h).review,
        Some(HostPowerOperation::CleanupSecret {
            operation: failed.id,
            secret
        })
    );
    assert!(editor(&h).password.is_empty());
    assert!(h.has_text(&format!("Remove retained Secret {secret}")));
    assert!(h.has_text(&format!("Original operation {}", failed.id)));
    assert!(h.has_text(power::CLEANUP_EFFECT));
    assert!(h.find(permissions::REVIEW_KEY).is_some());
    assert_eq!(h.try_click("host-power-confirm"), Err(view::DEMO.into()));
    // A cleanup that succeeded takes the record off the list.
    h.click("host-power-back");
    let mut done = configuration(pool_id, host, OperationState::Succeeded, false);
    done.action = ResourceAction::HostPower(HostPowerOperation::CleanupSecret {
        operation: failed.id,
        secret,
    });
    done.host_power_secret = None;
    h.model.resource_rows.push(done);
    assert!(h.find(&listed(failed.id)).is_none());
}

/// Hazard 15: a prompt answer is taken only for the same panel in the same
/// state. The prompts are the Rune prompts; a prompt that could not be shown
/// says why, a cancelled one says nothing.
#[test]
fn the_file_prompts_are_the_rune_prompts() {
    let mut h = Harness::scene("host-support");
    h.click("host-support-file");
    assert_eq!(
        h.node("host-support-file").label.as_deref(),
        Some("Inspecting archive file…")
    );
    assert_eq!(
        h.try_click("host-support-file"),
        Err(view::INSPECTING.into())
    );
    let request = prompt(&h);
    assert_eq!(request.title, "Save a new private host archive");
    assert!(matches!(request.mode, FileMode::Save));
    assert_eq!(request.suggested_name.as_deref(), Some("host-support.tar"));
    h.fail_file("no portal");
    assert_eq!(panel(&h).error.as_deref(), Some("no portal"));
    assert!(!panel(&h).pending);
    assert!(h.has_text("no portal"));
    h.click("host-support-file");
    assert!(panel(&h).error.is_none());
    h.answer_file(None);
    assert!(!panel(&h).pending && panel(&h).file.is_none() && panel(&h).error.is_none());
    assert_eq!(
        h.node("host-support-file").label.as_deref(),
        Some("Choose new archive file…")
    );

    let mut h = dialog(BACK_UP);
    h.click("host-support-file");
    assert_eq!(
        prompt(&h).suggested_name.as_deref(),
        Some("host-backup.tar.gz")
    );

    let mut h = dialog(RESTORE);
    assert_eq!(
        h.node("host-support-file").label.as_deref(),
        Some("Choose trusted backup…")
    );
    h.click("host-support-file");
    let request = prompt(&h);
    assert_eq!(
        request.title,
        "Select a trusted backup from this host and version"
    );
    assert!(matches!(request.mode, FileMode::Open));
    assert_eq!(request.filters.len(), 1);
    // The state moves on (a deletion review bumps the epoch as the mode
    // switch does): the late answer is dropped.
    let mut h = Harness::scene("host-support");
    panel_mut(&mut h)
        .overview
        .as_mut()
        .expect("overview")
        .crashdumps = dumps(1);
    h.click("host-support-file");
    support::review_deletion(&mut h.model, dumps(1).remove(0));
    let directory = TempDir::new("host-services-late");
    h.answer_file(Some(directory.path().join("late.tar")));
    h.tick(0.1);
    assert!(panel(&h).file.is_none() && !panel(&h).pending);
    assert!(panel(&h).deletion.is_some());
}

/// Hazard 16: toggling a category clears the review, and without a category
/// there is nothing to review.
#[test]
fn toggling_a_category_clears_the_review() {
    let mut h = Harness::scene("host-support");
    let directory = TempDir::new("host-services-toggle");
    choose(&mut h, directory.path().join("bundle.tar"));
    h.click("host-support-review");
    assert!(panel(&h).review.is_some());
    support::toggle(&mut h.model, "host-crashdump-logs");
    assert!(panel(&h).review.is_none());
    assert!(panel(&h).selected.is_empty());
    assert_eq!(
        h.try_click("host-support-review"),
        Err(view::NO_CATEGORY.into())
    );
    h.click("host-support-cap-host-crashdump-logs");
    assert!(panel(&h).selected.contains("host-crashdump-logs"));
    assert!(panel(&h).file.is_some(), "the file is kept");
    h.click("host-support-review");
    // A category the server never advertised cannot be requested.
    support::back(&mut h.model);
    panel_mut(&mut h).selected.insert("invented".into());
    assert!(support::action(panel(&h)).is_none());
}

/// Hazards 17 and 21, without a demo: the reasons of the disabled buttons.
#[test]
fn a_connection_that_cannot_write_refuses_staging_and_deletion() {
    let mut h = Harness::scene("host-crashdump-delete");
    h.model.demo = false;
    let uuid = scenes::scene_dump().uuid.to_string();
    h.type_text("host-crashdump-confirmation", &uuid);
    // No snapshot: neither observation is current.
    assert_eq!(
        h.try_click("host-crashdump-submit"),
        Err(support::STALE.into())
    );
    assert!(h.find("host-support-preview").is_none());
    // The commands say what Rune said inside the panel after opening.
    h.dismiss();
    for id in [BACK_UP, RESTORE, REPORT] {
        assert_eq!(
            h.eligibility(id),
            Eligibility::Disabled(Error::Closed.to_string()),
            "{id}"
        );
    }
    assert_eq!(h.eligibility(SETTINGS), Eligibility::Enabled);
}

// -- 4. Against the fixture server -------------------------------------------------------

/// Hazard 17: a download needs no write access; staging does. The status
/// report asks the server as soon as it opens.
#[test]
fn a_read_only_session_can_download_and_cannot_stage() {
    let pool = PoolId::from(uuid::Uuid::new_v4());
    let mut live = Live::over(Server::with_vm(pool), pool, true);
    let host = live.host();
    live.h.select(Target::one(host));
    assert_eq!(live.h.eligibility(BACK_UP), Eligibility::Enabled);
    assert_eq!(live.h.eligibility(REPORT), Eligibility::Enabled);
    assert_eq!(
        live.h.eligibility(RESTORE),
        Eligibility::Disabled(NOT_WRITABLE.into())
    );
    assert_eq!(live.h.eligibility(SETTINGS), Eligibility::Enabled);
    live.h.invoke(REPORT);
    let p = panel(&live.h);
    assert!(p.observed.is_some() && p.pool.is_some());
    assert!(support::current(&live.h.model, p));
    assert!(p.query.is_some(), "{:?}", p.error);
    assert!(live.h.find("host-support-reading").is_some());
    assert!(live.h.find("host-support-preview").is_none());
    live.h.dismiss();
    live.h.invoke(BACK_UP);
    assert!(panel(&live.h).query.is_none());
}

/// A reviewed download on a writable, current session is submitted once,
/// the panel closes and the Events tab follows it.
#[test]
fn a_reviewed_download_is_submitted_once_and_followed_in_events() {
    let mut live = Live::with_vm();
    let host = live.host();
    live.h.select(Target::one(host));
    live.h.invoke(BACK_UP);
    let directory = TempDir::new("host-services-live");
    choose(&mut live.h, directory.path().join("backup.tar.gz"));
    assert!(panel(&live.h).error.is_none(), "{:?}", panel(&live.h).error);
    live.h.click("host-support-review");
    assert!(panel(&live.h).review.is_some());
    live.h.click("host-support-submit");
    let open = live.h.model.areas.host_services.support.as_ref();
    assert!(open.is_none(), "{:?}", open.and_then(|p| p.error.clone()));
    assert_eq!(live.h.model.resource_operations.len(), 1);
    assert_eq!(live.h.model.active_tab(), TabId::Events);
}

/// A crashdump deletion on a writable, current session needs the typed
/// UUID, is submitted once, and the Events tab follows it.
#[test]
fn a_confirmed_crashdump_deletion_is_submitted_once() {
    let mut live = Live::with_vm();
    let host = live.host();
    live.h.select(Target::one(host));
    live.h.invoke(REPORT);
    let dump = scenes::scene_dump();
    let p = panel_mut(&mut live.h);
    p.query = None;
    p.error = None;
    p.overview = Some(HostSupportOverview {
        host: p.host,
        version: p.version.clone(),
        capabilities: Vec::new(),
        capabilities_error: None,
        crashdumps: vec![dump.clone()],
        crashdumps_error: None,
    });
    live.h.click(&format!("host-crashdump-delete-{}", dump.uuid));
    assert_eq!(live.h.surface().id, view::CRASHDUMP);
    assert_eq!(
        live.h.try_click("host-crashdump-submit"),
        Err(view::TYPE_DUMP.into())
    );
    live.h
        .type_text("host-crashdump-confirmation", &dump.uuid.to_string());
    live.h.click("host-crashdump-submit");
    let open = live.h.model.areas.host_services.support.as_ref();
    assert!(open.is_none(), "{:?}", open.and_then(|p| p.error.clone()));
    assert_eq!(live.h.model.resource_operations.len(), 1);
    assert_eq!(live.h.model.active_tab(), TabId::Events);
}

/// A reviewed configuration on a writable, current session is submitted
/// once and the editor closes.
#[test]
fn a_confirmed_configuration_is_submitted_and_the_editor_closes() {
    let mut live = Live::with_vm();
    let host = live.host();
    let id = live.server.host();
    live.h.select(Target::one(host));
    amend(&mut live.h, |inventory| {
        inventory
            .details
            .hosts
            .entry(id)
            .or_default()
            .power_configuration = Some(HostPowerConfiguration {
            settings: HostPowerSettings::Disabled,
            secret: None,
        });
    });
    live.h.invoke(SETTINGS);
    live.h.pick("host-power-mode", "1");
    live.h.click("host-power-review");
    assert!(editor(&live.h).review.is_some(), "{:?}", editor(&live.h).error);
    assert_eq!(power::stale(&live.h.model, editor(&live.h)), None);
    live.h.click("host-power-confirm");
    let open = live.h.model.areas.host_services.power.as_ref();
    assert!(open.is_none(), "{:?}", open.and_then(|e| e.error.clone()));
    assert_eq!(live.h.model.resource_operations.len(), 1);
}
