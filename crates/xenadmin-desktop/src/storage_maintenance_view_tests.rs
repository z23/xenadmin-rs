use super::*;
fn runtime(mut s: State, size: (f32, f32)) -> rune::Runtime<State> {
    s.window_width = size.0;
    s.window_height = size.1;
    let mut r = rune::Runtime::new(
        s,
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
fn repair_review_captures_targets_and_remains_reachable_at_compact_and_wide_sizes() {
    for size in [(1440., 900.), (513., 652.)] {
        let mut s = init();
        scene(&mut s, false);
        assert!(
            s.storage_editor
                .as_ref()
                .unwrap()
                .maintenance_review
                .is_some(),
            "{:?}",
            s.storage_editor.as_ref().unwrap().error
        );
        let captured = s
            .storage_editor
            .as_ref()
            .unwrap()
            .maintenance_review
            .clone();
        let mut r = runtime(s, size);
        for key in ["storage-maintenance-confirm", "storage-close"] {
            let (x, y) = r.find(key).unwrap();
            assert!(
                x >= 0. && x < size.0 && y >= 0. && y < size.1,
                "{key} {x},{y}"
            );
        }
        r.update(|s, _| {
            s.page = Page::Guests;
            s.rebuild();
        });
        settle(&mut r);
        assert_eq!(
            r.state.storage_editor.as_ref().unwrap().maintenance_review,
            captured
        );
        click(&mut r, "storage-maintenance-confirm");
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
fn repair_requires_selection_and_rejects_changed_attachment_graph() {
    let mut s = init();
    scene(&mut s, false);
    let e = s.storage_editor.as_mut().unwrap();
    e.maintenance_review = None;
    e.repair_selection.clear();
    stage(&mut s, false);
    assert!(
        s.storage_editor
            .as_ref()
            .unwrap()
            .maintenance_review
            .is_none()
    );
    let i = Arc::make_mut(s.inventory.as_mut().unwrap());
    let id = *i.details.pbds.keys().next().unwrap();
    i.details.pbds.get_mut(&id).unwrap().currently_attached = Some(true);
    s.storage_editor
        .as_mut()
        .unwrap()
        .repair_selection
        .insert(id);
    stage(&mut s, false);
    assert!(
        s.storage_editor
            .as_ref()
            .unwrap()
            .maintenance_review
            .is_none()
    );
}
#[test]
fn read_cache_review_explains_policy_and_preserves_target_at_both_widths() {
    for size in [(1440., 900.), (513., 652.)] {
        let mut s = init();
        scene(&mut s, true);
        assert!(matches!(
            s.storage_editor.as_ref().unwrap().maintenance_review,
            Some(StorageMaintenance::ReadCaching { enabled: false, .. })
        ));
        let mut r = runtime(s, size);
        assert!(
            r.accessibility_json()
                .to_string()
                .contains("does not establish effective caching")
        );
        click(&mut r, "storage-maintenance-back");
        assert!(
            r.state
                .storage_editor
                .as_ref()
                .unwrap()
                .maintenance_review
                .is_none()
        );
        click(&mut r, "storage-cache-review");
        assert!(
            r.state
                .storage_editor
                .as_ref()
                .unwrap()
                .maintenance_review
                .is_some()
        );
        click(&mut r, "storage-maintenance-confirm");
        assert!(r.state.resource_operations.is_empty());
    }
}
#[test]
fn cache_unknown_or_changed_current_policy_cannot_be_reviewed_or_submitted() {
    let mut s = init();
    scene(&mut s, true);
    let ResourceTarget::Storage(sr) = s.storage_editor.as_ref().unwrap().target else {
        panic!()
    };
    let op = s
        .storage_editor
        .as_ref()
        .unwrap()
        .maintenance_review
        .clone()
        .unwrap();
    Arc::make_mut(s.inventory.as_mut().unwrap())
        .details
        .storage
        .get_mut(&sr)
        .unwrap()
        .read_cache_override = Some("true".into());
    assert!(
        !op.availability(s.inventory.as_ref().unwrap(), sr)
            .is_available()
    );
    s.storage_editor.as_mut().unwrap().maintenance_review = None;
    Arc::make_mut(s.inventory.as_mut().unwrap())
        .details
        .storage
        .get_mut(&sr)
        .unwrap()
        .read_cache_policy_known = false;
    stage(&mut s, true);
    assert!(
        s.storage_editor
            .as_ref()
            .unwrap()
            .maintenance_review
            .is_none()
    );
}
#[test]
fn selecting_a_host_attachment_reviews_only_that_exact_target_at_both_widths() {
    for size in [(1440., 900.), (513., 652.)] {
        let mut s = init();
        scene(&mut s, false);
        let e = s.storage_editor.as_mut().unwrap();
        e.maintenance_review = None;
        e.repair_selection.clear();
        let id = *s
            .inventory
            .as_ref()
            .unwrap()
            .details
            .pbds
            .keys()
            .next()
            .unwrap();
        let mut r = runtime(s, size);
        click(&mut r, &format!("repair-pbd-{id}"));
        assert_eq!(
            r.state
                .storage_editor
                .as_ref()
                .unwrap()
                .repair_selection
                .len(),
            1
        );
        click(&mut r, "storage-repair-review");
        let Some(StorageMaintenance::Repair { attachments, .. }) =
            &r.state.storage_editor.as_ref().unwrap().maintenance_review
        else {
            panic!("review missing")
        };
        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].attachment, id);
    }
}
