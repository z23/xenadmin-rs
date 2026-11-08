use super::*;
fn runtime(mut state: State, size: (f32, f32)) -> rune::Runtime<State> {
    state.window_width = size.0;
    state.window_height = size.1;
    let mut r = rune::Runtime::new(
        state,
        crate::view,
        size,
        rune::Env {
            system_dark: true,
            scale: 1.,
        },
    );
    settle(&mut r);
    r
}
fn settle(r: &mut rune::Runtime<State>) {
    for _ in 0..30 {
        r.frame(1. / 60.);
    }
}
fn click(r: &mut rune::Runtime<State>, key: &str) {
    r.update(|_, t| t.scroll_into_view(key));
    settle(r);
    let (x, y) = r.find(key).unwrap_or_else(|| panic!("missing {key}"));
    r.pointer_down(x, y);
    r.pointer_up(x, y);
    settle(r);
}
#[test]
fn compact_and_wide_copy_review_capture_target_and_block_demo_dispatch() {
    for size in [(1440., 900.), (513., 652.)] {
        let mut s = init();
        scene(&mut s, true);
        assert!(
            s.disk_workspace.as_ref().unwrap().review.is_some(),
            "{:?}",
            s.disk_workspace.as_ref().unwrap().error
        );
        let id = s
            .disk_workspace
            .as_ref()
            .unwrap()
            .review
            .as_ref()
            .unwrap()
            .disk;
        let mut r = runtime(s, size);
        for key in ["disk-heading", "disk-confirm", "disks-close"] {
            let (x, y) = r.find(key).unwrap();
            assert!(
                x >= 0. && x < size.0 && y >= 0. && y < size.1,
                "{key} {x},{y}"
            );
        }
        r.update(|s, _| s.table.select("30000000-0000-0000-0000-000000000000"));
        settle(&mut r);
        assert_eq!(
            r.state
                .disk_workspace
                .as_ref()
                .unwrap()
                .review
                .as_ref()
                .unwrap()
                .disk,
            id
        );
        click(&mut r, "disk-confirm");
        assert!(r.state.resource_operations.is_empty());
        r.update(|s, _| s.demo = false);
        settle(&mut r);
        assert!(
            r.accessibility_json()
                .to_string()
                .contains("captured connection changed")
        );
    }
}
#[test]
fn growth_rejects_shrink_and_copy_requires_name_and_chosen_storage() {
    let mut s = init();
    scene(&mut s, false);
    let p = s.disk_workspace.as_mut().unwrap();
    p.review = None;
    p.size = TextEdit::new("0");
    review(&mut s);
    assert!(s.disk_workspace.as_ref().unwrap().review.is_none());
    let p = s.disk_workspace.as_mut().unwrap();
    p.mode = Mode::Copy;
    p.name = TextEdit::new(" ");
    review(&mut s);
    assert!(s.disk_workspace.as_ref().unwrap().review.is_none());
    let p = s.disk_workspace.as_mut().unwrap();
    p.name = TextEdit::new("New copy");
    p.destination = None;
    review(&mut s);
    assert!(s.disk_workspace.as_ref().unwrap().review.is_none());
}
#[test]
fn cleanup_requires_local_ownership_and_current_consumer_graph() {
    let mut s = init();
    scene(&mut s, false);
    let p = s.disk_workspace.as_mut().unwrap();
    p.review = None;
    p.mode = Mode::Cleanup;
    review(&mut s);
    assert!(s.disk_workspace.as_ref().unwrap().review.is_none());
    assert!(
        s.disk_workspace
            .as_ref()
            .unwrap()
            .error
            .as_ref()
            .unwrap()
            .contains("ownership")
    );
}

#[test]
fn cleanup_review_keeps_exact_uuid_and_explicit_destructive_confirmation_at_both_sizes() {
    for size in [(1440., 900.), (513., 652.)] {
        let mut s = init();
        cleanup_scene(&mut s);
        let captured = s
            .disk_workspace
            .as_ref()
            .unwrap()
            .review
            .as_ref()
            .unwrap()
            .disk;
        let mut r = runtime(s, size);
        assert!(
            r.accessibility_json()
                .to_string()
                .contains("Permanently delete")
        );
        click(&mut r, "disk-delete-confirmation");
        r.update(|s, _| {
            s.disk_workspace.as_mut().unwrap().confirmation = TextEdit::new(captured.to_string())
        });
        settle(&mut r);
        click(&mut r, "disk-confirm");
        assert!(r.state.resource_operations.is_empty());
        assert_eq!(
            r.state
                .disk_workspace
                .as_ref()
                .unwrap()
                .review
                .as_ref()
                .unwrap()
                .disk,
            captured
        );
        click(&mut r, "disk-history");
        assert!(r.state.disk_workspace.is_none());
        assert!(r.state.resource_editor.is_some());
    }
}
