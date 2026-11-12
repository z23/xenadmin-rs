use super::*;
use xenadmin_core::OperationId;
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
    r.update(|_, tx| tx.scroll_into_view(key));
    settle(r);
    let (x, y) = r.find(key).unwrap_or_else(|| panic!("missing {key}"));
    r.pointer_down(x, y);
    r.pointer_up(x, y);
    settle(r);
}
#[test]
fn compact_and_wide_export_review_keeps_exact_targets_and_safe_footer() {
    for size in [(513.0, 652.0), (1440.0, 900.0)] {
        let mut r = runtime(size);
        let review = r
            .state
            .bulk_export
            .as_ref()
            .unwrap()
            .review
            .clone()
            .unwrap();
        assert_eq!(review.batch.items.len(), 2);
        let text = r.accessibility_json().to_string();
        assert!(text.contains("snapshot ancestry"));
        assert!(text.contains("Read-only connections"));
        r.update(|_, tx| tx.scroll_into_view("review-permission-explanation"));
        settle(&mut r);
        let (_, y) = r.find("review-permission-explanation").unwrap();
        assert!((0.0..size.1).contains(&y));
        let (_, y) = r.find("batch-export-submit").unwrap();
        assert!(y < size.1);
        r.state.table.select("different selection");
        click(&mut r, "batch-export-submit");
        assert!(r.state.resource_operations.is_empty());
        assert!(r.state.bulk_export.as_ref().unwrap().review.as_ref() == Some(&review));
        assert!(review.paths.iter().all(|p| !PathBuf::from(p).exists()));
    }
}
#[test]
fn native_output_selection_is_bound_to_editor_identity_and_format() {
    let dir = tempfile::tempdir().unwrap();
    let mut r = runtime((513.0, 652.0));
    click(&mut r, "batch-export-back");
    click(&mut r, "batch-export-choose-0");
    assert_eq!(r.file_dialog().unwrap().0, rune::FileDialogKind::Save);
    let path = dir.path().join("selected.xva");
    r.resolve_file_dialog(Some(path.clone())).unwrap();
    r.frame(0.0);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while r.state.bulk_export.as_ref().unwrap().pending.is_some() {
        poll(&mut r.state);
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    assert_eq!(
        r.state.bulk_export.as_ref().unwrap().files[0]
            .as_ref()
            .unwrap()
            .0,
        path
    );
    assert!(!path.exists());
    click(&mut r, "batch-export-choose-1");
    change_format(
        &mut r.state,
        VmExportFormat::Ovf {
            format: OvfPackageFormat::Ova,
        },
    );
    r.resolve_file_dialog(Some(dir.path().join("old.xva")))
        .unwrap();
    r.frame(0.0);
    assert!(
        r.state
            .bulk_export
            .as_ref()
            .unwrap()
            .files
            .iter()
            .all(Option::is_none)
    );
    assert!(!dir.path().join("old.xva").exists());
}
#[test]
fn overlapping_destinations_and_changed_sources_block_entire_review() {
    let mut r = runtime((513.0, 652.0));
    let first = r.state.bulk_export.as_ref().unwrap().files[0].clone();
    edit(&mut r.state).files[1] = first;
    assert!(
        proposal(&r.state, r.state.bulk_export.as_ref().unwrap())
            .err()
            .unwrap()
            .contains("overlap")
    );
    let id = VmId::from(r.state.bulk_export.as_ref().unwrap().targets[0].id());
    Arc::make_mut(r.state.inventory.as_mut().unwrap())
        .vms
        .get_mut(&id)
        .unwrap()
        .power_state = PowerState::Running;
    assert!(proposal(&r.state, r.state.bulk_export.as_ref().unwrap()).is_err());
    submit(&mut r.state);
    assert!(r.state.resource_operations.is_empty());
    assert_eq!(r.state.bulk_export.as_ref().unwrap().targets.len(), 2);
}

#[test]
fn batch_history_keeps_parent_reachable_and_children_have_no_independent_controls() {
    let mut rt = runtime((1440.0, 900.0));
    let selection = rt
        .state
        .bulk_export
        .as_ref()
        .unwrap()
        .review
        .as_ref()
        .unwrap()
        .batch
        .clone();
    let pool = rt.state.inventory.as_ref().unwrap().pool_id;
    let parent = OperationId::new();
    let row = |id, action, batch_parent, detail| {
        serde_json::from_value(serde_json::json!({
            "id":id,"pool":pool,"target":{"Pool":pool},"action":action,
            "batch_parent":batch_parent,"state":"OutcomeUnknown","progress":null,"detail":detail,
            "created_at_ms":0,"can_cancel":false,"cancel_requested":false,"steps":[],"created":null
        }))
        .unwrap()
    };
    rt.state.resource_rows.push(row(
        parent,
        xenadmin_core::ResourceAction::VmWorkflowBatch(selection.clone()),
        None,
        "Captured parent export history",
    ));
    for _ in 0..32 {
        let item = &selection.items[0];
        rt.state.resource_rows.push(row(
            OperationId::new(),
            xenadmin_core::ResourceAction::Transfer(TransferAction::ExportXva {
                source: xenadmin_core::ExportSource::Vm {
                    vm: item.vm,
                    expected_template: false,
                },
                compression: XvaCompression::Uncompressed,
            }),
            Some(parent),
            "Owned transfer detail",
        ));
    }
    rt.update(|s, _| {
        s.bulk_export = None;
        resource_admin::open_pool(s, resource_admin::Kind::History);
    });
    settle(&mut rt);
    let text = rt.accessibility_json().to_string();
    assert!(text.contains("Captured parent export history"));
    assert!(!text.contains("Owned transfer detail"));
    rt.update(|s, _| {
        s.resource_editor = None;
        transfers::history(s);
    });
    settle(&mut rt);
    let text = rt.accessibility_json().to_string();
    assert!(text.contains("Controlled by batch"));
    assert!(!text.contains("Acknowledge after inspection"));
    assert!(!text.contains("Stop transfer"));
}
