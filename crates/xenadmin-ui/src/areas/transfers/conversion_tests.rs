//! The tests of the local conversion: the four of the Rune
//! `local_conversion_view_tests.rs` under their names, then the port's.
//!
//! The Rune reachability assertions (heading and final controls inside the
//! window at 1440x900 and 513x652) are the GPUI crate's
//! (`notes/progress/gpui-tests-wanted.md`).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};

use xenadmin_core::{
    ConversionFormat as Format, ConversionState, ConversionSummary, DiskImageFormat,
};

use super::conversion::{self, Event, Job, SURFACE, Workspace};
use super::scenes;
use crate::{Harness, prelude::*};

fn w(h: &Harness) -> &Workspace {
    &h.model.areas.transfers.conversion
}

fn prompts(h: &Harness) -> Vec<FileRequest> {
    h.model
        .intents
        .iter()
        .filter_map(|intent| match intent {
            Intent::ChooseFile(request) => Some(request.clone()),
            _ => None,
        })
        .collect()
}

fn job(h: &mut Harness) -> (mpsc::Sender<Event>, Arc<AtomicBool>) {
    let (tx, rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    h.model.areas.transfers.conversion.job = Some(Job {
        receiver: rx,
        cancel: cancel.clone(),
    });
    (tx, cancel)
}

fn row(state: ConversionState, detail: &str) -> ConversionSummary {
    ConversionSummary {
        id: uuid::Uuid::new_v4(),
        source: Format::Qcow2,
        output: DiskImageFormat::Vhd,
        virtual_size: Some(4096),
        state,
        detail: detail.into(),
    }
}

#[test]
fn offline_form_and_review_keep_heading_and_final_controls_reachable_without_host_access() {
    for review in [false, true] {
        let mut h = Harness::scene(if review {
            "local-conversion-review"
        } else {
            "local-conversion"
        });
        let shown = h.surface();
        assert_eq!(shown.id, SURFACE);
        assert_eq!(shown.title, "Local disk conversion");
        let last = if review {
            "conversion-submit"
        } else {
            "conversion-prepare"
        };
        assert!(h.find("conversion-close").is_some());
        assert_eq!(h.try_click(last), Err(conversion::PREVIEW.into()));
        conversion::prepare(&mut h.model);
        conversion::convert(&mut h.model);
        assert!(w(&h).job.is_none());
        assert!(h.model.resource_operations.is_empty());
        assert!(h.model.connection.is_none());
        assert_eq!(
            h.node("conversion-preview").text.as_deref(),
            Some(conversion::PREVIEW)
        );
        h.click("conversion-close");
        assert!(!w(&h).show);
        assert!(w(&h).job.is_none());
    }
}

#[test]
fn native_file_dialog_cancellation_and_late_result_do_not_change_closed_form() {
    let mut h = Harness::scene("local-conversion");
    let original = w(&h).source.clone();
    h.click("conversion-source");
    let request = prompts(&h).pop().unwrap();
    assert_eq!(request.mode, FileMode::Open);
    assert_eq!(request.title, "Select an offline self-contained disk image");
    assert!(request.filters.is_empty());
    h.answer_file(None);
    assert_eq!(w(&h).source, original);
    // A prompt that could not be shown: the desktop's words.
    h.click("conversion-source");
    h.fail_file("no portal");
    assert_eq!(w(&h).error.as_deref(), Some("no portal"));
    assert_eq!(w(&h).source, original);
    h.click("conversion-source");
    conversion::close(&mut h.model);
    h.answer_file(Some("/tmp/late-image.qcow2".into()));
    assert_eq!(w(&h).source, original);
    assert!(w(&h).job.is_none());
}

#[test]
fn background_job_controls_do_not_retarget_and_disconnection_is_reported() {
    let mut h = Harness::scene("local-conversion");
    let (tx, cancel) = job(&mut h);
    let source = w(&h).source.clone();
    assert_eq!(
        h.try_click("conversion-format-Raw"),
        Err(conversion::BUSY.into())
    );
    assert_eq!(w(&h).format, Format::Qcow2);
    assert_eq!(
        h.try_click("conversion-source"),
        Err(conversion::BUSY.into())
    );
    conversion::choose(&mut h.model, false);
    conversion::choose(&mut h.model, true);
    assert!(prompts(&h).is_empty());
    assert_eq!(w(&h).source, source);
    assert_eq!(h.node("conversion-close").label.as_deref(), Some("Hide"));
    h.click("conversion-cancel");
    assert!(cancel.load(Ordering::Relaxed));
    drop(tx);
    assert!(h.tick(0.1));
    assert!(w(&h).job.is_none());
    assert!(w(&h).error.as_ref().unwrap().contains("stopped"));
    assert_eq!(
        h.node("conversion-error").text.as_deref(),
        Some(conversion::STOPPED)
    );
    assert_eq!(h.node("conversion-close").label.as_deref(), Some("Close"));
}

#[test]
fn successful_conversion_only_offers_import_and_never_creates_a_host_operation() {
    let mut m = Model::fixture();
    scenes::conversion_scene(&mut m);
    let mut h = Harness::with(m);
    let (tx, _cancel) = job(&mut h);
    tx.send(Event::Finished(Ok(row(
        ConversionState::Completed,
        "Verified synthetic result",
    ))))
    .unwrap();
    drop(tx);
    assert!(conversion::poll(&mut h.model));
    assert!(w(&h).completed.is_some());
    assert!(h.model.resource_operations.is_empty());
    assert!(!h.model.areas.transfers.panel.show);
    assert!(h.model.connection.is_none());
    // The offer is there, and without a pool it says why it cannot be taken.
    assert_eq!(
        h.try_click("conversion-import"),
        Err(conversion::NO_POOL.into())
    );
    assert!(!h.model.areas.transfers.panel.show);
    assert!(h.dump().contains("Verified synthetic result"));
}

// -- The port -------------------------------------------------------------------------

#[test]
fn the_scenes_show_the_form_and_the_synthetic_review() {
    let h = Harness::scene("local-conversion");
    assert_eq!(
        h.node("conversion-tool").value.as_deref(),
        Some("/usr/bin/qemu-img")
    );
    assert_eq!(
        h.node("conversion-source-path").text.as_deref(),
        Some("/home/operator/images/offline-guest.qcow2")
    );
    assert_eq!(
        h.node("conversion-destination-path").text.as_deref(),
        Some("/home/operator/images/converted-guest.vhd")
    );
    assert_eq!(h.node("conversion-output-vhd").checked, Some(true));
    assert_eq!(h.node("conversion-format-QCOW2").checked, Some(true));

    let h = Harness::scene("local-conversion-review");
    let text = h.dump();
    assert!(text.contains("Review local conversion"));
    assert!(text.contains("QCOW2 → Vhd · 536870912 bytes"));
    assert!(text.contains("qemu-img version 10.1.2 · synthetic"));
    assert!(text.contains("New output: /home/operator/images/converted-guest.vhd"));
    assert!(text.contains("guest boot and driver compatibility remain unverified"));
    assert!(h.find("conversion-source").is_none());
    assert!(h.find("conversion-discard").is_some());
}

#[test]
fn changing_the_output_format_clears_the_destination_and_the_save_name_follows_it() {
    let mut h = Harness::scene("local-conversion");
    h.click("conversion-output-raw");
    assert_eq!(w(&h).output, DiskImageFormat::Raw);
    assert!(w(&h).destination.is_none());
    assert_eq!(
        h.node("conversion-destination-path").text.as_deref(),
        Some("No output selected")
    );
    h.click("conversion-destination");
    let request = prompts(&h).pop().unwrap();
    assert_eq!(request.mode, FileMode::Save);
    assert_eq!(request.title, "Save a new converted disk image");
    assert_eq!(request.suggested_name.as_deref(), Some("converted.raw"));
    h.answer_file(Some("/tmp/out.raw".into()));
    assert_eq!(w(&h).destination, Some("/tmp/out.raw".into()));
    h.click("conversion-format-VMDK");
    assert_eq!(w(&h).format, Format::Vmdk);
    h.type_text("conversion-tool", "/opt/qemu/bin/qemu-img");
    assert_eq!(w(&h).tool, "/opt/qemu/bin/qemu-img");
}

#[test]
fn a_finished_job_stays_on_the_last_step_and_a_completed_one_opens_the_import() {
    let mut h = Harness::scene("archive-export");
    super::panel::close(&mut h.model);
    scenes::conversion_scene(&mut h.model);
    h.model.areas.transfers.conversion.converting = true;
    let (tx, _cancel) = job(&mut h);
    tx.send(Event::Progress(row(
        ConversionState::Converting,
        "Converting 10%",
    )))
    .unwrap();
    assert!(conversion::poll(&mut h.model));
    assert!(matches!(
        h.surface().kind,
        SurfaceKind::Wizard { current: 2, .. }
    ));
    assert!(h.dump().contains("Converting 10%"));
    assert!(h.find("conversion-restart").is_none());
    assert!(h.find("conversion-cancel").is_some());
    // Hidden while it runs, the job is still there when the wizard is
    // shown again.
    h.click("conversion-close");
    assert!(!w(&h).show);
    assert!(w(&h).job.is_some());
    h.model.areas.transfers.conversion.show = true;
    tx.send(Event::Finished(Ok(row(
        ConversionState::Completed,
        "Verified synthetic result",
    ))))
    .unwrap();
    assert!(conversion::poll(&mut h.model));
    assert!(w(&h).job.is_none());
    assert_eq!(w(&h).history.len(), 1);
    assert!(h.find("conversion-restart").is_some());
    // The destination of the scene does not exist: the import opens, the
    // conversion hides, and the inspection of the missing file fails there.
    h.click("conversion-import");
    assert!(!w(&h).show);
    let panel = &h.model.areas.transfers.panel;
    assert!(panel.show);
    assert_eq!(panel.disk_name, "Converted disk");
    assert_eq!(panel.format, Some(DiskImageFormat::Vhd));
    assert_eq!(h.surface().id, super::view::SURFACE);
    assert!(h.model.resource_operations.is_empty());
}

#[test]
fn history_pages_and_cleanup_is_reviewed_before_it_runs() {
    let mut h = Harness::scene("local-conversion");
    let failed = row(ConversionState::Failed, "qemu-img failed");
    let id = failed.id;
    let mut rows = (0..13)
        .map(|n| row(ConversionState::Completed, &format!("job {n}")))
        .collect::<Vec<_>>();
    rows.push(failed);
    h.model.areas.transfers.conversion.history = rows;
    let text = h.dump();
    assert!(text.contains("Local history"));
    assert!(text.contains("Page 1 of 2 · 14 recorded jobs"));
    assert!(text.contains("qemu-img failed"));
    assert!(!text.contains("job 0"));
    assert!(h.try_click("conversion-history-previous").is_err());
    h.click("conversion-history-next");
    assert_eq!(w(&h).history_page, 1);
    assert!(h.dump().contains("job 0"));
    h.click("conversion-history-previous");
    h.click(&format!("conversion-cleanup-{id}"));
    assert_eq!(w(&h).cleanup, Some(id));
    assert!(
        h.dump()
            .contains("Remove stopped conversion temporary files?")
    );
    // The preview removes nothing.
    assert_eq!(
        h.try_click("conversion-cleanup-confirm"),
        Err(conversion::PREVIEW.into())
    );
    conversion::cleanup(&mut h.model);
    assert!(w(&h).job.is_none());
    assert!(w(&h).cleanup.is_none());
}

#[test]
fn a_review_that_arrives_while_hidden_is_not_kept_and_errors_are_shown() {
    let mut h = Harness::scene("local-conversion");
    let (tx, _cancel) = job(&mut h);
    tx.send(Event::Prepared(Box::new(Err(
        "qemu-img is not executable".into()
    ))))
    .unwrap();
    assert!(conversion::poll(&mut h.model));
    assert!(w(&h).job.is_none());
    assert!(w(&h).review.is_none());
    assert_eq!(
        h.node("conversion-error").text.as_deref(),
        Some("qemu-img is not executable")
    );
    let (tx, _cancel) = job(&mut h);
    tx.send(Event::History(Err("history unreadable".into())))
        .unwrap();
    assert!(conversion::poll(&mut h.model));
    assert_eq!(w(&h).error.as_deref(), Some("history unreadable"));
    // A quiet workspace does not report a change.
    assert!(!conversion::poll(&mut h.model));
}
