use super::*;
fn runtime(mut s: State, size: (f32, f32)) -> rune::Runtime<State> {
    s.window_width = size.0;
    s.window_height = size.1;
    let mut rt = rune::Runtime::new(
        s,
        crate::view,
        size,
        rune::Env {
            system_dark: true,
            scale: 1.,
        },
    );
    settle(&mut rt);
    rt
}
fn settle(rt: &mut rune::Runtime<State>) {
    for _ in 0..30 {
        rt.frame(1. / 60.);
    }
}
fn click(rt: &mut rune::Runtime<State>, key: &str) {
    rt.update(|_, t| t.scroll_into_view(key));
    settle(rt);
    let (x, y) = rt.find(key).unwrap_or_else(|| panic!("missing {key}"));
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    settle(rt);
}
#[test]
fn hba_selection_requires_review_and_captures_host_device_and_allocation_at_both_widths() {
    for size in [(513., 652.), (1440., 900.)] {
        let mut s = init();
        hba_scene(&mut s);
        let original_host = s.storage_editor.as_ref().unwrap().host.unwrap();
        let mut rt = runtime(s, size);
        click(&mut rt, "storage-result-0");
        assert!(rt.state.storage_editor.as_ref().unwrap().review.is_none());
        assert!(rt.state.resource_operations.is_empty());
        click(&mut rt, "storage-review");
        let captured = rt
            .state
            .storage_editor
            .as_ref()
            .unwrap()
            .review
            .clone()
            .unwrap();
        assert!(
            matches!(&captured.2,StorageOperation::Create {host,location:StorageLocation::Hba {allocation:StorageAllocation::Thin,..},hba_device:Some(device),..} if *host==original_host && device.serial=="LAB-2026-001")
        );
        rt.update(|s, _| {
            let e = s.storage_editor.as_mut().unwrap();
            e.host = None;
            e.scsi = TextEdit::new("changed");
            e.allocation = StorageAllocation::Thick;
        });
        settle(&mut rt);
        assert_eq!(
            rt.state.storage_editor.as_ref().unwrap().review.as_ref(),
            Some(&captured)
        );
        let (x, y) = rt.find("storage-confirm").unwrap();
        assert!(x > 0. && x < size.0 && y > 0. && y < size.1);
        click(&mut rt, "storage-confirm-lun");
        assert!(
            rt.accessibility_json()
                .to_string()
                .contains("confirm formatting")
        );
        click(&mut rt, "storage-confirm");
        assert!(rt.state.resource_operations.is_empty());
    }
}
#[test]
fn hba_review_refuses_unselected_device_changed_host_and_unadvertised_allocation() {
    let mut s = init();
    hba_scene(&mut s);
    s.storage_editor.as_mut().unwrap().discovered = None;
    review(&mut s);
    assert!(s.storage_editor.as_ref().unwrap().review.is_none());
    hba_scene(&mut s);
    select_result(&mut s, 0);
    s.storage_editor.as_mut().unwrap().host = None;
    review(&mut s);
    assert!(s.storage_editor.as_ref().unwrap().review.is_none());
    hba_scene(&mut s);
    select_result(&mut s, 0);
    s.storage_editor.as_mut().unwrap().backends.clear();
    review(&mut s);
    assert!(s.storage_editor.as_ref().unwrap().review.is_none());
    s.storage_editor.as_mut().unwrap().allocation = StorageAllocation::DriverDefault;
    review(&mut s);
    assert!(s.storage_editor.as_ref().unwrap().review.is_some());
}
#[test]
fn hba_discovery_results_cannot_be_selected_after_host_or_connection_changes() {
    let mut s = init();
    hba_scene(&mut s);
    s.storage_editor.as_mut().unwrap().host = None;
    select_result(&mut s, 0);
    assert!(s.storage_editor.as_ref().unwrap().hba_device.is_none());
    hba_scene(&mut s);
    s.demo = false;
    select_result(&mut s, 0);
    assert!(s.storage_editor.as_ref().unwrap().hba_device.is_none());
}
#[test]
fn hba_review_explains_external_ownership_destructive_format_and_no_automatic_cleanup() {
    let mut s = init();
    hba_review_scene(&mut s);
    let lines = describe(
        &s.storage_editor
            .as_ref()
            .unwrap()
            .review
            .as_ref()
            .unwrap()
            .2,
    )
    .join("\n");
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
