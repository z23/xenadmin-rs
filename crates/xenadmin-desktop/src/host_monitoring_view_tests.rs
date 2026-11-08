use super::*;
fn runtime(size: (f32, f32)) -> rune::Runtime<State> {
    let mut s = init();
    scene(&mut s);
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
    for _ in 0..30 {
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
fn destructive_review_keeps_captured_source_and_reachable_controls_at_both_widths() {
    for size in [(1440.0, 900.0), (513.0, 652.0)] {
        let mut r = runtime(size);
        let original = r.state.host_monitoring.as_ref().unwrap().review.clone();
        let target = r.state.host_monitoring.as_ref().unwrap().target;
        for key in [
            "monitoring-heading",
            "monitoring-close",
            "monitoring-submit",
            "monitoring-history",
        ] {
            let (x, y) = r.find(key).unwrap();
            assert!(
                x >= 0.0 && x < size.0 && y >= 0.0 && y < size.1,
                "{key}: {x},{y}"
            );
        }
        let (_, checkbox_y) = r.find("monitoring-confirm-history-loss").unwrap();
        if checkbox_y > size.1 - 130.0 {
            r.wheel(size.0 / 2.0, size.1 / 2.0, -200.0);
            settle(&mut r);
        }
        click(&mut r, "monitoring-confirm-history-loss");
        assert!(r.state.host_monitoring.as_ref().unwrap().acknowledged);
        // A selection elsewhere cannot retarget an already captured destructive review.
        r.update(|s, _| {
            s.table.select("a-different-selection");
        });
        settle(&mut r);
        click(&mut r, "monitoring-submit");
        let panel = r.state.host_monitoring.as_ref().unwrap();
        assert_eq!(panel.target, target);
        assert_eq!(panel.review, original);
        assert!(
            r.state.resource_operations.is_empty(),
            "synthetic review must never write"
        );
        click(&mut r, "monitoring-back");
        let panel = r.state.host_monitoring.as_ref().unwrap();
        assert!(panel.review.is_none());
        assert!(!panel.acknowledged);
    }
}
#[test]
fn refresh_discards_catalog_and_review_and_enabled_source_cannot_start_again() {
    let mut s = init();
    scene(&mut s);
    s.host_monitoring.as_mut().unwrap().review = None;
    review(&mut s, HostRecordingAction::StartRecording);
    assert!(s.host_monitoring.as_ref().unwrap().review.is_none());
    s.host_monitoring
        .as_mut()
        .unwrap()
        .sources
        .as_mut()
        .unwrap()[0]
        .enabled = false;
    review(&mut s, HostRecordingAction::StartRecording);
    assert!(matches!(
        s.host_monitoring.as_ref().unwrap().review,
        Some(HostOperation::DataSource {
            action: HostRecordingAction::StartRecording,
            ..
        })
    ));
    refresh(&mut s);
    let p = s.host_monitoring.as_ref().unwrap();
    assert!(p.sources.is_none() && p.selected.is_none() && p.review.is_none() && !p.acknowledged);
}
#[test]
fn history_uses_captured_pool_after_switching_the_visible_inventory() {
    let mut r = runtime((513.0, 652.0));
    let pool = r.state.host_monitoring.as_ref().unwrap().pool;
    r.update(|s, _| {
        Arc::make_mut(s.inventory.as_mut().unwrap()).pool_id = uuid::Uuid::new_v4().into();
    });
    settle(&mut r);
    click(&mut r, "monitoring-history");
    assert!(r.state.host_monitoring.is_none());
    assert!(r.state.activity.show);
    assert!(r.state.activity.scope_label().contains(&pool.to_string()));
    assert!(r.state.activity.scope_label().starts_with("History"));
}
