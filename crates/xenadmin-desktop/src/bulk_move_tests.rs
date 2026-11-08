use super::*;
fn state() -> State {
    let mut s = init();
    scene(&mut s);
    s
}
fn runtime(mut s: State, size: (f32, f32)) -> rune::Runtime<State> {
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
    r.update(|_, tx| tx.scroll_into_view(key));
    settle(r);
    let (x, y) = r.find(key).unwrap_or_else(|| panic!("missing {key}"));
    r.pointer_down(x, y);
    r.pointer_up(x, y);
    settle(r);
}
#[test]
fn compact_and_wide_review_capture_all_vms_and_keep_safe_footer() {
    for size in [(513.0, 652.0), (1440.0, 900.0)] {
        let mut r = runtime(state(), size);
        let review = r
            .state
            .bulk_move
            .as_ref()
            .unwrap()
            .review
            .clone()
            .expect("valid synthetic review");
        assert_eq!(review.batch.items.len(), 2);
        let text = r.accessibility_json().to_string();
        assert!(text.contains("Live migration"));
        assert!(text.contains("Recovery never starts"));
        r.update(|_, tx| tx.scroll_into_view("review-permission-explanation"));
        settle(&mut r);
        let (_, notice_y) = r.find("review-permission-explanation").unwrap();
        assert!((0.0..size.1).contains(&notice_y));
        let (_, y) = r.find("batch-move-submit").unwrap();
        assert!(y < size.1);
        r.state.table.select("changed selection");
        assert_eq!(
            r.state.bulk_move.as_ref().unwrap().review.as_ref(),
            Some(&review)
        );
        click(&mut r, "batch-move-submit");
        assert!(r.state.resource_operations.is_empty());
    }
}
#[test]
fn mapping_edits_are_per_target_and_review_exact_prior_and_new_values() {
    let mut s = state();
    s.bulk_move.as_mut().unwrap().review = None;
    let id = s.bulk_move.as_ref().unwrap().choices[0].disks[0].destination_sr;
    let mut r = runtime(s, (513.0, 652.0));
    click(&mut r, "batch-move-mappings-0");
    click(&mut r, &format!("batch-move-disk-0-0-{id}"));
    click(&mut r, "batch-move-review");
    let review = r.state.bulk_move.as_ref().unwrap().review.as_ref().unwrap();
    assert!(
        review.rows[0]
            .iter()
            .any(|v| v.contains("Disk ") && v.contains('→'))
    );
    assert!(review.rows[1].iter().all(|v| !v.starts_with("Disk ")));
}
#[test]
fn changed_or_unsupported_targets_refuse_whole_selection_without_omission() {
    for missing in [false, true] {
        let mut s = state();
        let id = s.bulk_move.as_ref().unwrap().choices[1].source.id;
        let i = Arc::make_mut(s.inventory.as_mut().unwrap());
        if missing {
            i.vms.remove(&id);
        } else {
            i.vms.get_mut(&id).unwrap().power_state = PowerState::Suspended;
        }
        assert!(build_review(&s, s.bulk_move.as_ref().unwrap()).is_err());
        submit(&mut s);
        assert!(s.resource_operations.is_empty());
        assert_eq!(s.bulk_move.as_ref().unwrap().choices.len(), 2);
    }
    let mut s = state();
    s.demo = false;
    assert!(!current(&s, s.bulk_move.as_ref().unwrap()));
    submit(&mut s);
    assert!(s.resource_operations.is_empty());
}
