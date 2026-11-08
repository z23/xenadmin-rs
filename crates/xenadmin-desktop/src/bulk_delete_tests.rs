use super::*;
fn runtime(size: (f32, f32)) -> rune::Runtime<State> {
    let mut s = init();
    scene(&mut s);
    s.window_width = size.0;
    s.window_height = size.1;
    let mut r = rune::Runtime::new(
        s,
        super::super::view,
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
    r.frame(0.0);
    settle(r);
}
#[test]
fn compact_and_wide_reviews_keep_exact_targets_and_the_confirmation_footer() {
    for size in [(513.0, 652.0), (1440.0, 900.0)] {
        let mut r = runtime(size);
        let e = r.state.bulk_delete.as_ref().unwrap();
        let review = e.review.as_ref().unwrap();
        assert_eq!(review.batch.items.len(), 2);
        assert!(review.batch.items.iter().all(|i|matches!(&i.workflow,VmWorkflow::Delete{disks,snapshots,..} if disks.is_empty()&&snapshots.is_empty())));
        assert!(review.rows.iter().all(|lines| {
            lines
                .iter()
                .any(|l| l.contains("Retain disk") || l.contains("Record-only"))
        }));
        let captured = review.clone();
        r.update(|s, _| s.table.select("different-row"));
        assert!(r.state.bulk_delete.as_ref().unwrap().review.as_ref() == Some(&captured));
        let (_, y) = r.find("batch-delete-close").unwrap();
        assert!(y > 0.0 && y < size.1);
        click(&mut r, "batch-delete-confirm");
        r.key(rune::Key::Text("DELETE 2".into()).into());
        r.frame(0.0);
        assert_eq!(
            r.state.bulk_delete.as_ref().unwrap().confirm.text,
            "DELETE 2"
        );
        click(&mut r, "batch-delete-submit");
        assert!(r.state.resource_operations.is_empty());
        assert!(r.state.bulk_delete.is_some());
        click(&mut r, "batch-delete-close");
        assert!(r.state.bulk_delete.is_none());
    }
}
#[test]
fn changed_or_missing_target_refuses_the_entire_review() {
    let mut r = runtime((900.0, 700.0));
    let selected = r.state.bulk_delete.as_ref().unwrap().choices[0].vm.id;
    r.update(|s, _| {
        Arc::make_mut(s.inventory.as_mut().unwrap())
            .vms
            .get_mut(&selected)
            .unwrap()
            .power_state = PowerState::Running;
    });
    let error = build_review(&r.state, r.state.bulk_delete.as_ref().unwrap())
        .err()
        .unwrap();
    assert!(error.contains(&selected.to_string()));
    r.update(|s, _| {
        Arc::make_mut(s.inventory.as_mut().unwrap())
            .vms
            .remove(&selected);
    });
    assert!(build_review(&r.state, r.state.bulk_delete.as_ref().unwrap()).is_err());
}
#[test]
fn selected_disk_data_is_explicit_and_new_ownership_cannot_be_silently_omitted() {
    let mut r = runtime((900.0, 700.0));
    r.update(|s, _| edit(s).review = None);
    let e = r.state.bulk_delete.as_ref().unwrap();
    let (index, disk) = e
        .choices
        .iter()
        .enumerate()
        .find_map(|(n, c)| {
            e.initial
                .vm_deletion_disks(c.vm.id)
                .first()
                .copied()
                .map(|d| (n, d))
        })
        .expect("fixture owned disk");
    click(&mut r, &format!("batch-delete-disk-{disk}"));
    assert!(
        r.state.bulk_delete.as_ref().unwrap().choices[index]
            .disks
            .contains(&disk)
    );
    click(&mut r, "batch-delete-review");
    assert!(
        r.state
            .bulk_delete
            .as_ref()
            .unwrap()
            .review
            .as_ref()
            .unwrap()
            .rows
            .iter()
            .flatten()
            .any(|s| s.contains("DELETE DISK DATA") && s.contains(&disk.to_string()))
    );
    r.update(|s, _| {
        Arc::make_mut(s.inventory.as_mut().unwrap())
            .details
            .vdis
            .get_mut(&disk)
            .unwrap()
            .sharable = Some(true);
    });
    assert!(build_review(&r.state, r.state.bulk_delete.as_ref().unwrap()).is_err());
}
#[test]
fn template_and_read_only_reviews_do_not_drop_ineligible_rows() {
    let mut r = runtime((900.0, 700.0));
    let id = r.state.bulk_delete.as_ref().unwrap().choices[0].vm.id;
    r.update(|s, _| {
        let i = Arc::make_mut(s.inventory.as_mut().unwrap());
        i.vms.get_mut(&id).unwrap().is_template = true;
        i.details.vms.get_mut(&id).unwrap().is_default_template = Some(true);
        edit(s).choices[0].vm.is_template = true;
    });
    assert!(build_review(&r.state, r.state.bulk_delete.as_ref().unwrap()).is_err());
    r.update(|s, _| {
        s.demo = false;
        edit(s).observed = None;
    });
    assert!(!current(&r.state, r.state.bulk_delete.as_ref().unwrap()));
    let before = r.state.resource_operations.len();
    r.update(|s, _| submit(s));
    assert_eq!(r.state.resource_operations.len(), before);
}
