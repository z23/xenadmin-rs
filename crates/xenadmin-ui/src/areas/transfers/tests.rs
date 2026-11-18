//! The tests of the transfer wizard: the Rune tests of `transfers.rs`,
//! `ovf_transfer_tests.rs`, `checkpoint_transfer_tests.rs` and
//! `compression_transfer_tests.rs` under their names, then tests of what
//! the port added (commands, steps, the outcome of a started transfer).
//!
//! The Rune tests ran at two window sizes and checked that each control
//! was inside the window: that half is the GPUI crate's
//! (`notes/progress/gpui-tests-wanted.md`). Where Rune clicked a choice
//! that was disabled in the review, the choice is not on the review step
//! here: the test asserts that, and that its handler changes nothing.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use uuid::Uuid;
use xenadmin_core::{
    DiskImageFormat, ExportSource, OperationId, OperationState, OvfPackageFormat, PbdId,
    PowerState, ResourceOperationRecord, SrId, StorageAttachment, TransferAction, TransferFile,
    VdiId, XvaCompression,
};

use super::commands::{CONVERT, EXPORT, EXPORT_DISK, EXPORT_TEMPLATE, IMPORT};
use super::panel::{self, Panel, Step};
use super::scenes::amend;
use super::view::SURFACE;
use crate::testing::TempDir;
use crate::{Harness, prelude::*};

fn export() -> Harness {
    Harness::scene("archive-export")
}

fn p(h: &Harness) -> &Panel {
    &h.model.areas.transfers.panel
}

fn shows(h: &Harness, needle: &str) -> bool {
    h.dump().contains(needle)
}

/// The file prompt that was asked for last.
fn prompt(h: &Harness) -> FileRequest {
    h.model
        .intents
        .iter()
        .rev()
        .find_map(|intent| match intent {
            Intent::ChooseFile(request) => Some(request.clone()),
            _ => None,
        })
        .expect("a file prompt is queued")
}

fn prompts(h: &Harness) -> usize {
    h.model
        .intents
        .iter()
        .filter(|intent| matches!(intent, Intent::ChooseFile(_)))
        .count()
}

/// Waits for the inspection thread, as the Rune tests polled.
fn inspected(h: &mut Harness) {
    for _ in 0..2000 {
        h.tick(0.01);
        if !p(h).pending {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    panic!("inspection did not complete");
}

fn picked(h: &mut Harness, path: PathBuf) {
    h.answer_file(Some(path));
    inspected(h);
}

/// Nothing was submitted, and pressing and calling "Begin transfer" in the
/// demo pool submits nothing.
fn apply_is_inert(h: &mut Harness) {
    assert_eq!(h.try_click("transfer-apply"), Err(panel::DEMO.into()));
    panel::apply(&mut h.model);
    assert!(h.model.resource_operations.is_empty());
    assert!(p(h).started.is_none());
}

fn first_sr(h: &Harness) -> SrId {
    SrId::from(h.model.inventory.as_ref().unwrap().storage[0].uuid)
}

fn switch_pool(h: &mut Harness) {
    amend(&mut h.model, |i| i.pool_id = Uuid::new_v4().into());
}

/// The Rune test fixture `disk_scene`: the import open; the first
/// repository attached to a host; every VM halted and every disk detached.
/// Returns the first disk.
fn import_fixture(h: &mut Harness) -> VdiId {
    h.invoke(IMPORT.0);
    let mut first = None;
    amend(&mut h.model, |i| {
        let sr = SrId::from(i.storage[0].uuid);
        let pbd = PbdId::from(Uuid::from_u128(0x9991));
        i.details.pbds.insert(
            pbd,
            StorageAttachment {
                id: pbd,
                location: None,
                requires_credentials: false,
                multipathed: None,
                paths: Vec::new(),
                host: i.hosts.keys().next().copied(),
                sr: Some(sr),
                currently_attached: Some(true),
            },
        );
        for vm in i.vms.values_mut() {
            vm.power_state = PowerState::Halted;
        }
        for vbd in i.details.vbds.values_mut() {
            vbd.currently_attached = Some(false);
        }
        first = i.details.vdis.keys().next().copied();
    });
    first.unwrap()
}

// -- transfers.rs ---------------------------------------------------------------------

#[test]
fn native_save_captures_source_and_reviews_without_demo_writes() {
    let mut h = export();
    let source = p(&h).source.clone();
    let d = TempDir::new("transfers");
    let path = d.path().join("export.xva");
    h.click("transfer-next");
    h.click("transfer-choose");
    assert_eq!(prompt(&h).mode, FileMode::Save);
    picked(&mut h, path.clone());
    assert!(p(&h).file.is_some());
    assert_eq!(p(&h).source, source);
    h.click("transfer-review");
    assert!(p(&h).review);
    apply_is_inert(&mut h);
    assert!(!path.exists());
}

#[test]
fn cancelled_or_stale_file_dialog_does_not_replace_review() {
    let mut h = export();
    h.click("transfer-next");
    h.click("transfer-choose");
    assert!(p(&h).pending);
    assert_eq!(
        h.try_click("transfer-choose"),
        Err(panel::INSPECTING.into())
    );
    h.answer_file(None);
    assert!(!p(&h).pending);
    // A prompt that could not be shown: the desktop's words, as in Rune.
    h.click("transfer-choose");
    h.fail_file("no portal");
    assert!(!p(&h).pending);
    assert_eq!(p(&h).error.as_deref(), Some("no portal"));
    assert_eq!(h.node("transfer-error").text.as_deref(), Some("no portal"));
    h.click("transfer-choose");
    assert!(p(&h).error.is_none());
    panel::close(&mut h.model);
    let d = TempDir::new("transfers");
    h.answer_file(Some(d.path().join("stale.xva")));
    assert!(p(&h).file.is_none());
    assert!(p(&h).result.is_none());
    assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 0);
}

fn unknown_import(pool: xenadmin_core::PoolId, id: OperationId) -> ResourceOperationRecord {
    serde_json::from_value(serde_json::json!({"id":id,"pool":pool,"target":{"Pool":pool},"action":{"Transfer":{"ImportXva":{"sr":Uuid::from_u128(4)}}},"state":"OutcomeUnknown","progress":null,"detail":"Host import failed; cleanup unverified","created_at_ms":1,"can_cancel":false,"cancel_requested":false,"steps":[],"created":null,"transfer":{"bytes":1024,"total":2048,"stream_complete":false,"phase":"Stopped"},"transfer_created":{"vms":[],"disks":[Uuid::from_u128(22)],"networks":[]}})).unwrap()
}

#[test]
fn offline_unknown_history_preserves_evidence_and_cannot_release_guard() {
    let mut h = export();
    let pool = h.model.inventory.as_ref().unwrap().pool_id;
    let id = OperationId::new();
    h.model.resource_rows = vec![unknown_import(pool, id)];
    h.model.inventory = None;
    h.model.snapshot = None;
    h.model.areas.transfers.panel.started = Some(id);
    assert_eq!(
        h.try_click(&format!("transfer-acknowledge-{id}")),
        Err(panel::NOT_WRITABLE.into())
    );
    panel::review_acknowledge(&mut h.model, id);
    assert!(p(&h).acknowledge.is_none());
    panel::control(
        &mut h.model,
        id,
        xenadmin_core::OperationControl::AcknowledgeUnknown,
    );
    assert_eq!(
        h.model.resource_rows[0].state,
        OperationState::OutcomeUnknown
    );
    assert_eq!(h.model.resource_rows[0].transfer_created.disks.len(), 1);
    // The evidence of the Rune history card.
    assert!(shows(&h, "Host import failed; cleanup unverified"));
    assert!(shows(&h, "1024 bytes of 2048 · Stopped"));
    assert!(shows(
        &h,
        &format!("Retained disk UUID: {}", Uuid::from_u128(22))
    ));
    assert!(shows(&h, "never sends archive bytes again"));
    assert!(h.find(&format!("transfer-release-{id}")).is_none());
}

#[test]
fn switched_pool_invalidates_review_and_import_uses_open_dialog() {
    let mut h = export();
    assert!(panel::current(&h.model, p(&h)));
    assert!(h.find("transfer-stale").is_none());
    switch_pool(&mut h);
    assert!(!panel::current(&h.model, p(&h)));
    assert_eq!(h.node("transfer-stale").text.as_deref(), Some(panel::STALE));
    h.invoke(IMPORT.0);
    assert!(panel::current(&h.model, p(&h)));
    h.click("transfer-next");
    h.click("transfer-choose");
    assert_eq!(prompt(&h).mode, FileMode::Open);
    h.answer_file(None);
}

#[test]
fn native_raw_import_reviews_name_capacity_and_exact_storage_without_overwrite() {
    let mut h = export();
    import_fixture(&mut h);
    h.click("transfer-format-raw");
    h.click("transfer-next");
    let d = TempDir::new("transfers");
    let path = d.path().join("data.raw");
    std::fs::write(&path, vec![7; 4096]).unwrap();
    h.click("transfer-choose");
    let request = prompt(&h);
    assert_eq!(request.mode, FileMode::Open);
    assert_eq!(request.title, "Select a disk or archive image");
    assert_eq!(request.filters[0].extensions, ["raw"]);
    picked(&mut h, path.clone());
    assert_eq!(
        p(&h)
            .file
            .as_ref()
            .unwrap()
            .disk_image()
            .unwrap()
            .virtual_size,
        4096
    );
    h.click("transfer-next");
    let sr = first_sr(&h);
    h.type_text("transfer-disk-name", "Imported data");
    h.click(&format!("transfer-sr-{sr}"));
    h.click("transfer-review");
    assert!(p(&h).review);
    assert!(h.find("transfer-disk-name").is_none());
    assert!(shows(&h, "New disk name: Imported data"));
    assert_eq!(
        panel::action(p(&h)),
        Some(TransferAction::ImportDisk {
            sr,
            name: "Imported data".into(),
            format: DiskImageFormat::Raw
        })
    );
    apply_is_inert(&mut h);
    assert_eq!(std::fs::read(path).unwrap(), vec![7; 4096]);
}

#[test]
fn disk_export_captures_identity_and_rejects_capacity_or_attachment_changes() {
    let mut h = export();
    let id = import_fixture(&mut h);
    h.click("transfer-disk-picker");
    h.click(&format!("export-disk-{id}"));
    assert_eq!(h.surface().title, "Export disk image");
    let a = panel::action(p(&h)).unwrap();
    assert!(a.is_export());
    assert!(
        a.availability(h.model.inventory.as_ref().unwrap())
            .is_available()
    );
    h.click("transfer-next");
    h.click("transfer-choose");
    assert_eq!(prompt(&h).mode, FileMode::Save);
    h.answer_file(None);
    amend(&mut h.model, |i| {
        i.details.vdis.get_mut(&id).unwrap().virtual_size = Some(512);
    });
    assert!(
        !a.availability(h.model.inventory.as_ref().unwrap())
            .is_available()
    );
    assert_eq!(panel::action(p(&h)), Some(a));
    // The same disk through its command: it is a disk the command is for.
    let pool = h.model.inventory.as_ref().unwrap().pool_id;
    let disk = Target::one(ObjectRef::new(Some(pool), ObjectKind::VirtualDisk, id));
    assert_ne!(h.model.eligibility(EXPORT_DISK, &disk), Eligibility::Hidden);
}

#[test]
fn format_change_invalidates_selected_file_and_blank_disk_name_cannot_apply() {
    let mut h = export();
    import_fixture(&mut h);
    let sr = first_sr(&h);
    h.model.areas.transfers.panel.sr = Some(sr);
    panel::set_format(&mut h.model, Some(DiskImageFormat::Raw));
    let d = TempDir::new("transfers");
    let path = d.path().join("data.raw");
    std::fs::write(&path, vec![0; 512]).unwrap();
    h.model.areas.transfers.panel.file =
        Some(TransferFile::disk_source(path, DiskImageFormat::Raw).unwrap());
    assert!(
        !panel::action(p(&h))
            .unwrap()
            .availability(h.model.inventory.as_ref().unwrap())
            .is_available()
    );
    // The review cannot be entered, and says what the core says.
    let refusal = panel::review_blocked(&h.model, p(&h)).expect("a blank name is refused");
    assert_eq!(
        h.node("transfer-unavailable").text.as_deref(),
        Some(refusal.as_str())
    );
    panel::set_format(&mut h.model, Some(DiskImageFormat::Vhd));
    assert!(p(&h).file.is_none());
    assert!(!p(&h).review);
}

// -- ovf_transfer_tests.rs ------------------------------------------------------------

fn package(dir: &Path) -> PathBuf {
    let xml = r#"<Envelope xmlns="http://schemas.dmtf.org/ovf/envelope/1" xmlns:ovf="http://schemas.dmtf.org/ovf/envelope/1" xmlns:rasd="http://schemas.dmtf.org/wbem/wscim/1/cim-schema/2/CIM_ResourceAllocationSettingData"><References><File ovf:id="data" ovf:href="data.raw"/></References><DiskSection><Info>Disks</Info><Disk ovf:diskId="data" ovf:fileRef="data" ovf:capacity="4096" ovf:format="http://raw"/></DiskSection><NetworkSection><Info>Networks</Info><Network ovf:name="Application network"/></NetworkSection><VirtualSystem ovf:id="guest"><Name>Portable application</Name><VirtualHardwareSection><Info>Hardware</Info><Item><rasd:InstanceID>1</rasd:InstanceID><rasd:ResourceType>3</rasd:ResourceType><rasd:VirtualQuantity>2</rasd:VirtualQuantity></Item><Item><rasd:InstanceID>2</rasd:InstanceID><rasd:ResourceType>4</rasd:ResourceType><rasd:AllocationUnits>byte * 2^20</rasd:AllocationUnits><rasd:VirtualQuantity>512</rasd:VirtualQuantity></Item><Item><rasd:InstanceID>3</rasd:InstanceID><rasd:ResourceType>17</rasd:ResourceType><rasd:HostResource>ovf:/disk/data</rasd:HostResource></Item><Item><rasd:InstanceID>4</rasd:InstanceID><rasd:ResourceType>10</rasd:ResourceType><rasd:Connection>Application network</rasd:Connection></Item></VirtualHardwareSection></VirtualSystem></Envelope>"#;
    std::fs::write(dir.join("data.raw"), [7u8; 4096]).unwrap();
    let path = dir.join("application.ovf");
    std::fs::write(&path, xml).unwrap();
    path
}

#[test]
fn ovf_import_requires_explicit_network_mapping_and_reviews_captured_storage() {
    let mut h = export();
    import_fixture(&mut h);
    h.click("transfer-format-ovf");
    assert_eq!(
        panel::steps(p(&h)),
        [
            Step::Type,
            Step::Source,
            Step::Storage,
            Step::Networking,
            Step::Review
        ]
    );
    h.click("transfer-next");
    let dir = TempDir::new("transfers");
    let file = package(dir.path());
    h.click("transfer-choose");
    let request = prompt(&h);
    assert_eq!(request.mode, FileMode::Open);
    assert_eq!(request.filters[0].extensions, ["ovf", "ova"]);
    picked(&mut h, file.clone());
    let summary = p(&h).file.as_ref().unwrap().ovf().unwrap();
    assert_eq!(summary.guests[0].name, "Portable application");
    assert_eq!(summary.guests[0].memory_bytes, 512 << 20);
    assert!(shows(&h, "No package manifest"));
    h.click("transfer-next");
    let sr = first_sr(&h);
    h.click(&format!("transfer-sr-{sr}"));
    h.click("transfer-next");
    assert!(panel::action(p(&h)).is_none());
    assert_eq!(h.try_click("transfer-review"), Err(panel::UNMAPPED.into()));
    panel::next(&mut h.model);
    assert!(!p(&h).review);
    let network = *h
        .model
        .inventory
        .as_ref()
        .unwrap()
        .details
        .networks
        .keys()
        .next()
        .unwrap();
    h.click(&format!("ovf-network-0-{network}"));
    h.click("transfer-review");
    assert!(p(&h).review);
    assert_eq!(
        panel::action(p(&h)),
        Some(TransferAction::ImportOvf {
            sr,
            networks: BTreeMap::from([("Application network".into(), network)])
        })
    );
    assert!(shows(&h, "MAC addresses are generated"));
    assert!(shows(&h, "No package manifest"));
    assert!(shows(&h, &network.to_string()));
    apply_is_inert(&mut h);
    assert!(file.exists());
    switch_pool(&mut h);
    assert!(!panel::current(&h.model, p(&h)));
    assert_eq!(h.try_click("transfer-apply"), Err(panel::STALE.into()));
    panel::apply(&mut h.model);
    assert!(h.model.resource_operations.is_empty());
}

#[test]
fn ova_and_ovf_export_use_new_destinations_and_keep_captured_source() {
    for (format, key, name) in [
        (OvfPackageFormat::Ova, "ova", "application.ova"),
        (OvfPackageFormat::Ovf, "ovf", "application-package"),
    ] {
        let mut h = export();
        let source = p(&h).source.clone().unwrap();
        h.click(&format!("transfer-package-{key}"));
        assert_eq!(
            panel::action(p(&h)),
            Some(TransferAction::ExportOvf {
                source: source.clone(),
                format
            })
        );
        let dir = TempDir::new("transfers");
        let dest = dir.path().join(name);
        h.click("transfer-next");
        h.click("transfer-choose");
        let request = prompt(&h);
        assert_eq!(request.mode, FileMode::Save);
        if format == OvfPackageFormat::Ovf {
            assert_eq!(request.title, "Choose a new OVF package directory name");
            assert_eq!(request.suggested_name.as_deref(), Some("guest-ovf"));
            assert!(request.filters.is_empty());
        } else {
            assert_eq!(request.title, "Save a new disk or archive image");
            assert_eq!(request.suggested_name.as_deref(), Some("guest.ova"));
            assert_eq!(request.filters[0].extensions, ["ova"]);
        }
        picked(&mut h, dest.clone());
        assert!(p(&h).file.is_some());
        h.click("transfer-review");
        assert!(p(&h).review);
        apply_is_inert(&mut h);
        assert!(!dest.exists());
        assert_eq!(p(&h).source.as_ref(), Some(&source));
    }
}

#[test]
fn changing_package_format_discards_inspection_and_stale_picker_results() {
    let mut h = export();
    let dir = TempDir::new("transfers");
    h.click("transfer-package-ova");
    h.click("transfer-next");
    h.click("transfer-choose");
    let serial = p(&h).serial;
    panel::set_format(&mut h.model, None);
    h.model.areas.transfers.panel.package_export = Some(OvfPackageFormat::Ovf);
    assert!(p(&h).serial > serial);
    h.answer_file(Some(dir.path().join("old.ova")));
    assert!(p(&h).file.is_none());
    assert!(p(&h).result.is_none());
    assert!(!p(&h).pending);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

// -- checkpoint_transfer_tests.rs -----------------------------------------------------

#[test]
fn checkpoint_export_explicitly_reviews_memory_and_captured_identity_at_both_sizes() {
    let mut h = Harness::scene("checkpoint-export");
    assert!(matches!(
        p(&h).source,
        Some(ExportSource::Checkpoint {
            preserve_memory: false,
            ..
        })
    ));
    assert!(h.find("transfer-package-xva").is_some());
    assert!(h.find("transfer-package-ova").is_none());
    panel::pick_package(&mut h.model, "ova");
    assert!(p(&h).package_export.is_none());
    h.click("checkpoint-preserve-memory");
    let source = p(&h).source.clone();
    assert!(matches!(
        source,
        Some(ExportSource::Checkpoint {
            preserve_memory: true,
            ..
        })
    ));
    let d = TempDir::new("transfers");
    let file = TransferFile::destination(d.path().join("checkpoint.xva")).unwrap();
    h.model.areas.transfers.panel.file = Some(file);
    h.click("transfer-next");
    h.click("transfer-review");
    assert!(p(&h).review);
    assert!(shows(&h, "guest RAM"));
    assert!(shows(&h, "separate resume"));
    assert!(shows(&h, "Guest memory: Include guest memory"));
    // Locked in the review: the choice is not there, and its handler
    // changes nothing.
    assert!(h.find("checkpoint-disks-only").is_none());
    panel::pick_memory(&mut h.model, false);
    assert_eq!(p(&h).source, source);
    assert!(p(&h).review);
    apply_is_inert(&mut h);
    assert!(!d.path().join("checkpoint.xva").exists());
    switch_pool(&mut h);
    assert!(!panel::current(&h.model, p(&h)));
}

#[test]
fn checkpoint_import_review_uses_separate_action_and_retains_suspended_state() {
    let mut h = export();
    import_fixture(&mut h);
    let sr = first_sr(&h);
    let d = TempDir::new("transfers");
    let mut value =
        serde_json::to_value(TransferFile::destination(d.path().join("checkpoint.xva")).unwrap())
            .unwrap();
    value["identity"] = value["parent"].clone();
    value["manifest"] = serde_json::json!({"guests":[{"uuid":Uuid::from_u128(70),"name":"Captured application","template":false,"memory":{"uuid":Uuid::from_u128(71),"virtual_size":536870912}}],"data_disks":1,"virtual_bytes":536875008u64,"networks":0,"metadata_digest":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]});
    let file = serde_json::from_value(value).unwrap();
    h.click("transfer-next");
    h.model.areas.transfers.panel.file = Some(file);
    assert!(shows(&h, "Captured application · Suspended checkpoint"));
    h.click("transfer-next");
    h.click(&format!("transfer-sr-{sr}"));
    assert_eq!(
        panel::action(p(&h)),
        Some(TransferAction::ImportCheckpointXva { sr })
    );
    h.click("transfer-review");
    assert!(p(&h).review);
    assert!(shows(&h, "suspended guests"));
    assert!(shows(&h, "memory image"));
    apply_is_inert(&mut h);
}

// -- compression_transfer_tests.rs ----------------------------------------------------

#[test]
fn compression_choice_invalidates_destination_and_review_keeps_exact_source_and_format() {
    let mut h = export();
    let source = p(&h).source.clone();
    let d = TempDir::new("transfers");
    let plain = d.path().join("old.xva");
    h.model.areas.transfers.panel.file = Some(TransferFile::destination(&plain).unwrap());
    h.click("transfer-compression-zstd");
    assert!(p(&h).file.is_none());
    assert_eq!(p(&h).compression, XvaCompression::Zstd);
    h.click("transfer-next");
    h.click("transfer-choose");
    assert_eq!(prompt(&h).suggested_name.as_deref(), Some("guest.xva.zst"));
    h.answer_file(None);
    let output = d.path().join("chosen.xva.zst");
    h.model.areas.transfers.panel.file = Some(TransferFile::destination(&output).unwrap());
    h.click("transfer-review");
    assert!(p(&h).review);
    assert!(shows(&h, "Output format: zstd"));
    assert!(h.find("transfer-compression-gzip").is_none());
    panel::pick_compression(&mut h.model, "gzip");
    assert!(matches!(
        panel::action(p(&h)),
        Some(TransferAction::ExportXva {
            compression: XvaCompression::Zstd,
            ..
        })
    ));
    assert_eq!(p(&h).source, source);
    apply_is_inert(&mut h);
    assert!(!output.exists());
}

#[test]
fn changing_compression_discards_pending_picker_result_and_package_changes_clear_it() {
    let mut h = export();
    h.click("transfer-next");
    h.click("transfer-choose");
    let d = TempDir::new("transfers");
    panel::set_format(&mut h.model, None);
    h.model.areas.transfers.panel.compression = XvaCompression::Gzip;
    h.answer_file(Some(d.path().join("stale.xva")));
    assert!(p(&h).file.is_none());
    h.click("transfer-back");
    h.click("transfer-package-ova");
    assert_eq!(p(&h).compression, XvaCompression::Uncompressed);
    assert!(matches!(
        panel::action(p(&h)),
        Some(TransferAction::ExportOvf { .. })
    ));
    assert!(h.find("transfer-compression-zstd").is_none());
}

// -- The port -------------------------------------------------------------------------

/// The share of this area in the Rune `main.rs` test of the experimental
/// notice: `experimental_dialog_open` was true for both dialogs.
#[test]
fn every_scene_of_the_area_shows_an_experimental_wizard() {
    for (scene, surface, steps, current) in [
        ("archive-export", SURFACE, 3, 0),
        ("xva-compression", SURFACE, 3, 0),
        ("checkpoint-export", SURFACE, 3, 0),
        ("ovf-import", SURFACE, 5, 4),
        ("ovf-export", SURFACE, 3, 2),
        ("disk-export", SURFACE, 3, 0),
        ("local-conversion", super::conversion::SURFACE, 3, 0),
        ("local-conversion-review", super::conversion::SURFACE, 3, 1),
    ] {
        let h = Harness::scene(scene);
        let shown = h.surface();
        assert_eq!(shown.id, surface, "{scene}");
        assert!(shown.experimental, "{scene}");
        match shown.kind {
            SurfaceKind::Wizard {
                steps: ref all,
                current: at,
            } => {
                assert_eq!((all.len(), at), (steps, current), "{scene}");
            }
            _ => panic!("{scene} is not a wizard"),
        }
        h.assert_unique_keys();
    }
}

#[test]
fn scenes_show_what_the_rune_scenes_showed() {
    let h = Harness::scene("xva-compression");
    assert_eq!(h.node("transfer-compression-zstd").checked, Some(true));
    assert!(shows(&h, "Output format: zstd · .xva.zst"));

    let h = Harness::scene("ovf-export");
    assert!(shows(&h, "Format: OVA file"));
    assert!(shows(&h, "Create one OVA file"));
    assert!(h.find("transfer-apply").is_some());

    let h = Harness::scene("ovf-import");
    assert!(shows(&h, "application.ova · synthetic preview"));
    assert!(shows(&h, "Package manifest verified"));
    assert!(shows(&h, "Portable application"));
    assert!(shows(&h, "Map package network: Application network"));
    assert_eq!(
        h.node("transfer-apply").disabled.as_deref(),
        Some(panel::DEMO)
    );

    let h = Harness::scene("disk-export");
    assert_eq!(h.surface().title, "Export disk image");
    assert_eq!(h.node("transfer-format-vhd").checked, Some(true));
    assert!(h.find("transfer-format-xva").is_none());
    assert!(shows(&h, "All consumers must remain halted."));

    let h = Harness::scene("checkpoint-export");
    assert!(shows(
        &h,
        "Memory checkpoint: Application memory checkpoint"
    ));
    assert!(shows(&h, "Checkpoint memory image:"));
}

#[test]
fn commands_follow_the_kind_and_count_of_the_target() {
    let mut h = export();
    let i = h.model.inventory.clone().unwrap();
    let pool = i.pool_id;
    panel::close(&mut h.model);
    assert_eq!(h.menu_item(IMPORT.0).unwrap().label, "Import");
    assert_eq!(h.menu_item(EXPORT.0).unwrap().label, "Export");
    assert_eq!(h.menu_item(CONVERT.0).unwrap().label, "Convert Disk Image");
    assert_eq!(h.eligibility(EXPORT.0), Eligibility::Enabled);
    // A VM is not exported by the template command, nor the reverse.
    assert_eq!(h.eligibility(EXPORT_TEMPLATE.0), Eligibility::Hidden);
    let template = i.vms.values().find(|v| v.is_template).map(|v| v.id);
    if let Some(template) = template {
        let target = Target::one(ObjectRef::template(pool, template));
        assert_eq!(
            h.model.eligibility(EXPORT_TEMPLATE, &target),
            Eligibility::Enabled
        );
        assert_eq!(h.model.eligibility(EXPORT, &target), Eligibility::Hidden);
        h.invoke_on(EXPORT_TEMPLATE.0, &target);
        assert!(matches!(
            p(&h).source,
            Some(ExportSource::Vm {
                expected_template: true,
                ..
            })
        ));
        assert!(shows(&h, "Template: "));
        panel::close(&mut h.model);
    }
    // Several VMs are the bulk_export area's.
    let vms = i
        .vms
        .values()
        .filter(|v| !v.is_control_domain && !v.is_template)
        .take(2)
        .map(|v| ObjectRef::vm(pool, v.id))
        .collect::<Vec<_>>();
    assert_eq!(vms.len(), 2);
    assert_eq!(
        h.model.eligibility(EXPORT, &Target::many(vms)),
        Eligibility::Hidden
    );
    // A control domain had no button.
    if let Some(control) = i.vms.values().find(|v| v.is_control_domain) {
        assert_eq!(
            h.model
                .eligibility(EXPORT, &Target::one(ObjectRef::vm(pool, control.id))),
            Eligibility::Hidden
        );
    }
    let host = *i.hosts.keys().next().unwrap();
    let host = Target::one(ObjectRef::host(pool, host));
    assert_eq!(h.model.eligibility(EXPORT, &host), Eligibility::Hidden);
    assert_eq!(h.model.eligibility(EXPORT_DISK, &host), Eligibility::Hidden);
    assert_eq!(h.model.eligibility(IMPORT, &host), Eligibility::Enabled);

    // Without a pool: no import, but the conversion.
    let mut h = Harness::with(Model::fixture());
    assert_eq!(
        h.eligibility(IMPORT.0),
        Eligibility::Disabled(panel::NO_POOL.into())
    );
    assert_eq!(h.eligibility(CONVERT.0), Eligibility::Enabled);
    h.model.demo = true;
    h.invoke(CONVERT.0);
    assert_eq!(h.surface().id, super::conversion::SURFACE);
    assert!(h.model.areas.transfers.conversion.job.is_none());
}

/// Outside the demo the Rune button was disabled when the core does not
/// allow the export: the command says what the core says.
#[test]
fn export_of_a_running_vm_is_refused_with_the_reason_of_the_core() {
    let mut h = export();
    let Some(ExportSource::Vm { vm, .. }) = p(&h).source.clone() else {
        panic!("the scene exports a VM");
    };
    panel::close(&mut h.model);
    amend(&mut h.model, |i| {
        i.vms.get_mut(&vm).unwrap().power_state = PowerState::Running;
        i.details.vms.get_mut(&vm).unwrap().allowed_operations = Some(Vec::new());
    });
    h.model.demo = false;
    let now = h.model.inventory.clone().unwrap();
    let expected = TransferAction::ExportXva {
        compression: Default::default(),
        source: ExportSource::Vm {
            vm,
            expected_template: false,
        },
    }
    .availability(&now);
    assert!(!expected.is_available());
    assert_eq!(
        h.eligibility(EXPORT.0),
        Eligibility::Disabled(expected.explanation())
    );
    assert_eq!(h.try_invoke(EXPORT.0), Err(expected.explanation()));
    assert!(!p(&h).show);
}

#[test]
fn steps_go_back_and_forth_and_a_step_says_why_it_cannot_be_left() {
    let mut h = export();
    assert_eq!(panel::step(p(&h)), Step::Format);
    assert!(h.find("transfer-back").is_none());
    h.click("transfer-next");
    assert_eq!(panel::step(p(&h)), Step::Destination);
    assert_eq!(
        h.try_click("transfer-review"),
        Err(panel::NO_DESTINATION.into())
    );
    h.click("transfer-back");
    assert_eq!(panel::step(p(&h)), Step::Format);

    import_fixture(&mut h);
    assert_eq!(h.surface().title, "Import image");
    assert_eq!(panel::step(p(&h)), Step::Type);
    assert_eq!(h.node("transfer-format-xva").checked, Some(true));
    h.click("transfer-next");
    assert_eq!(h.try_click("transfer-next"), Err(panel::NO_SOURCE.into()));
    h.click("transfer-choose");
    assert_eq!(
        prompt(&h).filters[0].extensions,
        ["xva", "gz", "zst", "zstd"]
    );
    h.answer_file(None);
    let d = TempDir::new("transfers");
    h.model.areas.transfers.panel.file =
        Some(TransferFile::destination(d.path().join("x.xva")).unwrap());
    h.click("transfer-next");
    assert_eq!(
        h.try_click("transfer-review"),
        Err(panel::NO_STORAGE.into())
    );
    let sr = first_sr(&h);
    h.click(&format!("transfer-sr-{sr}"));
    assert_eq!(panel::action(p(&h)), Some(TransferAction::ImportXva { sr }));
    // Back from the review returns to the last step of the form.
    h.click("transfer-review");
    assert_eq!(panel::step(p(&h)), Step::Review);
    h.click("transfer-back");
    assert_eq!(panel::step(p(&h)), Step::Storage);
}

#[test]
fn history_closes_the_wizard_and_shows_the_events_tab() {
    let mut h = export();
    h.click("transfer-history");
    assert!(!p(&h).show);
    assert_eq!(h.model.active_tab(), TabId::Events);
}

#[test]
fn a_session_change_closes_the_wizard_and_a_late_file_is_dropped() {
    let mut h = export();
    h.click("transfer-next");
    h.click("transfer-choose");
    h.model.load_demo(2, 8);
    assert!(!p(&h).show);
    let d = TempDir::new("transfers");
    h.answer_file(Some(d.path().join("late.xva")));
    assert!(p(&h).file.is_none());
    assert!(p(&h).result.is_none());
}

#[test]
fn a_converted_image_opens_the_import_on_its_source_step_under_inspection() {
    let mut h = export();
    let d = TempDir::new("transfers");
    let path = d.path().join("converted.raw");
    std::fs::write(&path, vec![1; 1024]).unwrap();
    super::import_converted(&mut h.model, path.clone(), DiskImageFormat::Raw);
    assert_eq!(h.surface().title, "Import image");
    assert_eq!(panel::step(p(&h)), Step::Source);
    assert_eq!(p(&h).disk_name, "Converted disk");
    inspected(&mut h);
    assert_eq!(
        p(&h)
            .file
            .as_ref()
            .unwrap()
            .disk_image()
            .unwrap()
            .virtual_size,
        1024
    );
    h.click("transfer-next");
    assert_eq!(
        h.node("transfer-disk-name").value.as_deref(),
        Some("Converted disk")
    );
    assert_eq!(std::fs::read(path).unwrap(), vec![1; 1024]);
}

/// A started transfer stays in the wizard: its record, with the controls
/// of the Rune history. In the demo no connection can control it.
#[test]
fn a_started_transfer_shows_its_record_and_its_controls() {
    let mut h = export();
    let pool = h.model.inventory.as_ref().unwrap().pool_id;
    let id = OperationId::new();
    h.model.areas.transfers.panel.started = Some(id);
    // Before the record arrives.
    assert!(shows(&h, "Transfer submitted"));
    assert!(h.find("transfer-apply").is_none());
    assert!(h.find("transfer-back").is_none());
    h.model.resource_rows = vec![unknown_import(pool, id)];
    assert!(h.find("transfer-outcome").is_some());
    for key in ["recheck", "stop", "acknowledge"] {
        assert_eq!(
            h.node(&format!("transfer-{key}-{id}")).disabled.as_deref(),
            Some(panel::NOT_WRITABLE),
            "{key}"
        );
    }
    // The second step of the acknowledgement, once reviewed.
    h.model.areas.transfers.panel.acknowledge = Some(id);
    assert!(h.find(&format!("transfer-acknowledge-{id}")).is_none());
    assert_eq!(
        h.node(&format!("transfer-release-{id}")).label.as_deref(),
        Some("Release reservation")
    );
    assert!(shows(&h, "This releases the local reservation"));
    // The form's handlers do nothing any more.
    panel::choose(&mut h.model);
    assert_eq!(prompts(&h), 0);
    h.click("transfer-close");
    assert!(!p(&h).show);
}
