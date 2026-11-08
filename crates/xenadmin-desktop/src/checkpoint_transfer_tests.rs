use super::{
    tests::{click, runtime},
    *,
};

#[test]
fn checkpoint_export_explicitly_reviews_memory_and_captured_identity_at_both_sizes() {
    for size in [(513.0, 652.0), (1440.0, 900.0)] {
        let mut rt = runtime(size);
        rt.update(|s, _| checkpoint_scene(s));
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
        assert!(matches!(
            rt.state.transfers.source,
            Some(ExportSource::Checkpoint {
                preserve_memory: false,
                ..
            })
        ));
        assert!(rt.find("transfer-package-ova").is_none());
        click(&mut rt, "checkpoint-preserve-memory");
        let source = rt.state.transfers.source.clone();
        let d = tempfile::tempdir().unwrap();
        let file = TransferFile::destination(d.path().join("checkpoint.xva")).unwrap();
        rt.update(|s, _| s.transfers.file = Some(file));
        click(&mut rt, "transfer-review");
        assert!(rt.state.transfers.review);
        let text = rt.accessibility_json().to_string();
        assert!(text.contains("guest RAM"));
        assert!(text.contains("separate resume"));
        click(&mut rt, "checkpoint-disks-only");
        assert_eq!(rt.state.transfers.source, source);
        click(&mut rt, "transfer-apply");
        assert!(rt.state.resource_operations.is_empty());
        assert!(!d.path().join("checkpoint.xva").exists());
        rt.update(|s, _| {
            Arc::make_mut(s.inventory.as_mut().unwrap()).pool_id = uuid::Uuid::new_v4().into();
        });
        assert!(!current(&rt.state, &rt.state.transfers));
    }
}

#[test]
fn checkpoint_import_review_uses_separate_action_and_retains_suspended_state() {
    for size in [(513.0, 652.0), (1440.0, 900.0)] {
        let mut rt = runtime(size);
        tests::disk_scene(&mut rt);
        let sr = SrId::from(rt.state.inventory.as_ref().unwrap().storage[0].uuid);
        click(&mut rt, &format!("transfer-sr-{sr}"));
        let d = tempfile::tempdir().unwrap();
        let mut value = serde_json::to_value(
            TransferFile::destination(d.path().join("checkpoint.xva")).unwrap(),
        )
        .unwrap();
        value["identity"] = value["parent"].clone();
        value["manifest"] = serde_json::json!({"guests":[{"uuid":uuid::Uuid::from_u128(70),"name":"Captured application","template":false,"memory":{"uuid":uuid::Uuid::from_u128(71),"virtual_size":536870912}}],"data_disks":1,"virtual_bytes":536875008u64,"networks":0,"metadata_digest":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]});
        let file = serde_json::from_value(value).unwrap();
        rt.update(|s, _| s.transfers.file = Some(file));
        assert_eq!(
            action(&rt.state.transfers),
            Some(TransferAction::ImportCheckpointXva { sr })
        );
        click(&mut rt, "transfer-review");
        assert!(rt.state.transfers.review);
        let text = rt.accessibility_json().to_string();
        assert!(text.contains("suspended guests"));
        assert!(text.contains("memory image"));
        click(&mut rt, "transfer-apply");
        assert!(rt.state.resource_operations.is_empty());
    }
}
