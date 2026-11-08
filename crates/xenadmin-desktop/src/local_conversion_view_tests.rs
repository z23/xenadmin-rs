use super::*;
fn runtime(size: (f32, f32), review: bool) -> rune::Runtime<State> {
    let mut s = init();
    if review {
        review_scene(&mut s)
    } else {
        scene(&mut s)
    }
    s.window_width = size.0;
    s.window_height = size.1;
    let mut r = rune::Runtime::new(
        s,
        view,
        size,
        rune::Env {
            system_dark: true,
            scale: 1.0,
        },
    );
    settle(&mut r);
    r
}
fn settle(r: &mut rune::Runtime<State>) {
    for _ in 0..25 {
        r.frame(1.0 / 60.0);
    }
}
fn click(r: &mut rune::Runtime<State>, key: &str) {
    let (x, y) = r.find(key).unwrap();
    r.pointer_down(x, y);
    r.pointer_up(x, y);
    settle(r);
}
#[test]
fn offline_form_and_review_keep_heading_and_final_controls_reachable_without_host_access() {
    for size in [(1440.0, 900.0), (513.0, 652.0)] {
        for review in [false, true] {
            let mut r = runtime(size, review);
            for key in [
                "conversion-heading",
                "conversion-close",
                if review {
                    "conversion-submit"
                } else {
                    "conversion-prepare"
                },
            ] {
                let (x, y) = r.find(key).unwrap();
                assert!(
                    x >= 0.0 && x < size.0 && y >= 0.0 && y < size.1,
                    "{key}: {x},{y}"
                );
            }
            click(
                &mut r,
                if review {
                    "conversion-submit"
                } else {
                    "conversion-prepare"
                },
            );
            assert!(r.state.local_conversion.job.is_none());
            assert!(r.state.resource_operations.is_empty());
            assert!(r.state.connection.is_none());
            click(&mut r, "conversion-close");
            assert!(!r.state.local_conversion.show);
        }
    }
}
#[test]
fn native_file_dialog_cancellation_and_late_result_do_not_change_closed_form() {
    let mut r = runtime((1440.0, 900.0), false);
    let original = r.state.local_conversion.source.clone();
    click(&mut r, "conversion-source");
    assert_eq!(r.file_dialog().unwrap().0, rune::FileDialogKind::Open);
    r.resolve_file_dialog(None).unwrap();
    settle(&mut r);
    assert_eq!(r.state.local_conversion.source, original);
    click(&mut r, "conversion-source");
    r.update(|s, _| close(s));
    r.resolve_file_dialog(Some("/tmp/late-image.qcow2".into()))
        .unwrap();
    settle(&mut r);
    assert_eq!(r.state.local_conversion.source, original);
    assert!(r.state.local_conversion.job.is_none());
}
#[test]
fn background_job_controls_do_not_retarget_and_disconnection_is_reported() {
    let mut r = runtime((1440.0, 900.0), false);
    let (tx, rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let stop = cancel.clone();
    r.update(|s, _| {
        s.local_conversion.job = Some(Job {
            receiver: rx,
            cancel: stop,
        })
    });
    settle(&mut r);
    let source = r.state.local_conversion.source.clone();
    click(&mut r, "conversion-format-Raw");
    assert_eq!(r.state.local_conversion.format, Format::Qcow2);
    click(&mut r, "conversion-source");
    assert!(r.file_dialog().is_none());
    assert_eq!(r.state.local_conversion.source, source);
    click(&mut r, "conversion-cancel");
    assert!(cancel.load(Ordering::Relaxed));
    drop(tx);
    r.update(|s, _| {
        poll(s);
    });
    settle(&mut r);
    assert!(r.state.local_conversion.job.is_none());
    assert!(
        r.state
            .local_conversion
            .error
            .as_ref()
            .unwrap()
            .contains("stopped")
    );
}
#[test]
fn successful_conversion_only_offers_import_and_never_creates_a_host_operation() {
    let mut s = init();
    scene(&mut s);
    let (tx, rx) = mpsc::channel();
    s.local_conversion.job = Some(Job {
        receiver: rx,
        cancel: Arc::new(AtomicBool::new(false)),
    });
    let row = ConversionSummary {
        id: uuid::Uuid::new_v4(),
        source: Format::Qcow2,
        output: DiskImageFormat::Vhd,
        virtual_size: Some(4096),
        state: ConversionState::Completed,
        detail: "Verified synthetic result".into(),
    };
    tx.send(Event::Finished(Ok(row))).unwrap();
    drop(tx);
    assert!(poll(&mut s));
    assert!(s.local_conversion.completed.is_some());
    assert!(s.resource_operations.is_empty());
    assert!(!s.transfers.show);
    assert!(s.connection.is_none());
}
